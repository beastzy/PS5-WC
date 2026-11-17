//! Read physical gamepads on the active Linux seat. Grab while the menu or
//! Red's keyboard bridge owns input, and observe Guide during gameplay.
use std::{
    collections::HashMap,
    ffi::{c_int, c_long, c_uint, c_void},
    fs::{File, OpenOptions},
    os::unix::{fs::OpenOptionsExt, io::AsRawFd},
    path::{Path, PathBuf},
    ptr::NonNull,
    sync::{
        atomic::{AtomicU8, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use smithay::{
    input::keyboard::Keysym,
    reexports::{
        calloop::channel::{channel, Event},
        udev,
    },
};

use super::{dispatch, owns_gamepad, owns_red_gamepad, red_key, Guide, PadAction};
use crate::{state::Backend, AnvilState};

const INACTIVE: u8 = 0;
const GAME: u8 = 1;
const MENU: u8 = 2;
const RED: u8 = 3;
const EV_KEY: c_uint = 1;
const EV_ABS: c_uint = 3;
const ABS_HAT0X: c_uint = 0x10;
const ABS_HAT0Y: c_uint = 0x11;
const BTN_SOUTH: c_uint = 0x130;
const BTN_EAST: c_uint = 0x131;
const BTN_TL: c_uint = 0x136;
const BTN_TR: c_uint = 0x137;
const BTN_START: c_uint = 0x13b;
const BTN_MODE: c_uint = 0x13c;
const DPAD: std::ops::RangeInclusive<c_uint> = 0x220..=0x223;

#[repr(C)]
#[derive(Default)]
struct InputEvent {
    seconds: c_long,
    microseconds: c_long,
    kind: u16,
    code: u16,
    value: i32,
}

#[link(name = "evdev")]
extern "C" {
    fn libevdev_new_from_fd(fd: c_int, dev: *mut *mut c_void) -> c_int;
    fn libevdev_free(dev: *mut c_void);
    fn libevdev_get_id_vendor(dev: *const c_void) -> c_int;
    fn libevdev_get_id_product(dev: *const c_void) -> c_int;
    fn libevdev_has_event_code(dev: *const c_void, kind: c_uint, code: c_uint) -> c_int;
    fn libevdev_get_event_value(dev: *const c_void, kind: c_uint, code: c_uint) -> c_int;
    fn libevdev_grab(dev: *mut c_void, mode: c_int) -> c_int;
    fn libevdev_next_event(dev: *mut c_void, flags: c_uint, event: *mut InputEvent) -> c_int;
}

struct Pad {
    dev: NonNull<c_void>,
    _file: File,
    battery: Option<PathBuf>,
    guide: Guide,
    grabbed: bool,
}

impl Pad {
    fn open(path: &Path, battery: Option<PathBuf>) -> Option<Self> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(smithay::reexports::rustix::fs::OFlags::NONBLOCK.bits() as i32)
            .open(path)
            .ok()?;
        let mut dev = std::ptr::null_mut();
        if unsafe { libevdev_new_from_fd(file.as_raw_fd(), &mut dev) } != 0 {
            return None;
        }
        Some(Self {
            dev: NonNull::new(dev)?,
            _file: file,
            battery,
            guide: Guide::default(),
            grabbed: false,
        })
    }

    fn event_value(&self, kind: c_uint, code: c_uint) -> i32 {
        unsafe { libevdev_get_event_value(self.dev.as_ptr(), kind, code) }
    }

    fn handoff(&mut self, grab: bool) {
        if self.grabbed == grab {
            return;
        }
        // Let both sides see the release of buttons they owned. A hold can
        // show Home immediately, then transfer input when Guide is released.
        let held = (BTN_SOUTH..=0x13e)
            .chain(DPAD)
            .any(|code| self.event_value(EV_KEY, code) != 0);
        if held
            || [ABS_HAT0X, ABS_HAT0Y]
                .iter()
                .any(|&code| self.event_value(EV_ABS, code) != 0)
        {
            return;
        }
        // libevdev_grab_mode values LIBEVDEV_GRAB and LIBEVDEV_UNGRAB.
        if unsafe { libevdev_grab(self.dev.as_ptr(), if grab { 3 } else { 4 }) } == 0 {
            self.grabbed = grab;
        }
    }

    fn poll(&mut self, mode: u8, mut send: impl FnMut(PadAction)) -> bool {
        let mut event = InputEvent::default();
        // Bound work when reconnecting or when an analog device produces a burst.
        for _ in 0..256 {
            let result = unsafe { libevdev_next_event(self.dev.as_ptr(), 2, &mut event) };
            if result < 0 {
                if !matches!(result, -11 | -4) {
                    return false;
                }
                break;
            }
            if result == 1 {
                // Recover the internal state after SYN_DROPPED. The missed
                // events aren't suitable for menu navigation or gestures.
                self.guide = Guide::default();
                while unsafe { libevdev_next_event(self.dev.as_ptr(), 1, &mut event) } == 1 {}
                continue;
            }
            let now = Instant::now();
            match (
                c_uint::from(event.kind),
                c_uint::from(event.code),
                event.value,
            ) {
                (EV_KEY, BTN_MODE, 0 | 1) => {
                    if let Some(action) = self.guide.button(event.value == 1, now) {
                        send(action);
                    }
                }
                (EV_KEY, 0x13a, 0) if self.identity().0 == 0x054c => {
                    send(PadAction::Create);
                }
                (EV_KEY, code, value @ (1 | 2))
                    if self.grabbed && (mode == RED || (mode == MENU && value == 1)) =>
                {
                    if let Some(action) = navigation(mode, code) {
                        send(action);
                    }
                }
                (EV_ABS, ABS_HAT0X | ABS_HAT0Y, -1 | 1)
                    if self.grabbed && matches!(mode, MENU | RED) =>
                {
                    let button = match (c_uint::from(event.code), event.value) {
                        (ABS_HAT0X, -1) => 0x222,
                        (ABS_HAT0X, _) => 0x223,
                        (_, -1) => 0x220,
                        _ => 0x221,
                    };
                    if let Some(action) = navigation(mode, button) {
                        send(action);
                    }
                }
                _ => (),
            }
        }
        if let Some(action) = self.guide.tick(Instant::now()) {
            send(action);
        }
        self.handoff(matches!(mode, MENU | RED));
        true
    }

    fn identity(&self) -> (u16, u16) {
        unsafe {
            (
                libevdev_get_id_vendor(self.dev.as_ptr()) as u16,
                libevdev_get_id_product(self.dev.as_ptr()) as u16,
            )
        }
    }

    fn battery(&self) -> Option<u8> {
        let path = self.battery.as_ref()?;
        std::fs::read_to_string(path)
            .ok()?
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|x| *x <= 100)
    }
}

impl Drop for Pad {
    fn drop(&mut self) {
        // Closing the file releases any grab even if the device went away.
        unsafe { libevdev_free(self.dev.as_ptr()) };
    }
}

fn navigation(mode: u8, button: c_uint) -> Option<PadAction> {
    if mode == RED {
        red_key(button).map(PadAction::RedKey)
    } else {
        key(button).map(PadAction::Key)
    }
}

fn key(code: c_uint) -> Option<Keysym> {
    Some(match code {
        BTN_SOUTH => Keysym::Return,
        BTN_EAST => Keysym::Escape,
        BTN_TL => Keysym::ISO_Left_Tab,
        BTN_TR => Keysym::Tab,
        BTN_START => Keysym::Menu,
        0x220 => Keysym::Up,
        0x221 => Keysym::Down,
        0x222 => Keysym::Left,
        0x223 => Keysym::Right,
        _ => return None,
    })
}

fn battery_path(device: &udev::Device) -> Option<PathBuf> {
    let parent = device.parent_with_subsystem("hid").ok()??;
    std::fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .find_map(|entry| {
            let path = entry.path().canonicalize().ok()?;
            let capacity = path.join("capacity");
            (path.starts_with(parent.syspath()) && capacity.is_file()).then_some(capacity)
        })
}

fn scan(pads: &mut HashMap<PathBuf, Pad>, seat: &str) {
    let Ok(mut devices) = udev::Enumerator::new() else {
        return;
    };
    if devices.match_subsystem("input").is_err() {
        return;
    }
    let Ok(devices) = devices.scan_devices() else {
        return;
    };
    for device in devices {
        let Some(path) = device.devnode() else {
            continue;
        };
        if !path
            .file_name()
            .is_some_and(|name| name.as_encoded_bytes().starts_with(b"event"))
            || device
                .property_value("ID_INPUT_JOYSTICK")
                .and_then(|s| s.to_str())
                != Some("1")
            || device
                .property_value("ID_SEAT")
                .and_then(|s| s.to_str())
                .unwrap_or("seat0")
                != seat
            || device.syspath().starts_with("/sys/devices/virtual/input")
            || pads.contains_key(path)
        {
            continue;
        }
        if let Some(pad) = Pad::open(path, battery_path(&device)) {
            // Exclude companion keys and pedals that can't navigate the menu.
            if unsafe { libevdev_has_event_code(pad.dev.as_ptr(), EV_KEY, BTN_SOUTH) } != 0 {
                pads.insert(path.to_path_buf(), pad);
            }
        }
    }
}

pub(super) struct Ownership {
    mode: Arc<AtomicU8>,
    devices: Arc<Mutex<super::Devices>>,
}

impl Ownership {
    pub(super) fn update<B: Backend>(&self, state: &AnvilState<B>) {
        let mode = if !state.controller_active {
            INACTIVE
        } else if owns_gamepad(state) {
            MENU
        } else if owns_red_gamepad(state) {
            RED
        } else {
            GAME
        };
        self.mode.store(mode, Ordering::Release);
    }

    pub(super) fn devices(&self) -> super::Devices {
        self.devices.lock().unwrap().clone()
    }
}

impl Drop for Ownership {
    fn drop(&mut self) {
        self.mode.store(u8::MAX, Ordering::Release);
    }
}

pub(super) fn install<B: Backend + 'static>(state: &AnvilState<B>) -> Option<Ownership> {
    let mode = Arc::new(AtomicU8::new(INACTIVE));
    let worker = mode.clone();
    let devices = Arc::new(Mutex::new(super::Devices::default()));
    let inventory = devices.clone();
    let seat = state.seat_name.clone();
    let (send, receive) = channel();
    if let Err(error) = state.handle.insert_source(receive, |event, _, state| {
        if let Event::Msg(action) = event {
            if state.controller_active {
                dispatch(state, action);
            }
        }
    }) {
        tracing::warn!(%error, "Cannot watch native controllers");
        return None;
    }
    thread::spawn(move || {
        let mut pads = HashMap::new();
        let mut next_scan = Instant::now();
        let mut next_report = next_scan;
        let mut status = None;
        loop {
            let now = Instant::now();
            let mode = worker.load(Ordering::Acquire);
            if mode == u8::MAX {
                break;
            }
            if mode == INACTIVE {
                pads.clear();
                if status.take().is_some() {
                    *inventory.lock().unwrap() = super::Devices::default();
                }
                next_scan = now;
                next_report = now;
                thread::sleep(Duration::from_millis(80));
                continue;
            }
            if now >= next_scan {
                scan(&mut pads, &seat);
                next_scan = now + Duration::from_secs(2);
            }
            pads.retain(|_, pad| {
                pad.poll(mode, |action| {
                    let _ = send.send(action);
                })
            });
            if now >= next_report || status.is_none_or(|(count, _)| count != pads.len()) {
                let update = (
                    pads.len(),
                    pads.iter()
                        .min_by_key(|(path, _)| *path)
                        .and_then(|(_, pad)| pad.battery()),
                );
                *inventory.lock().unwrap() = super::Devices {
                    count: update.0,
                    battery: update.1,
                    ids: {
                        let mut ids: Vec<_> = pads.values().map(Pad::identity).collect();
                        ids.sort_unstable();
                        ids
                    },
                };
                status = Some(update);
                next_report = now + Duration::from_secs(5);
            }
            thread::sleep(Duration::from_millis(if pads.is_empty() { 150 } else { 5 }));
        }
    });
    Some(Ownership { mode, devices })
}
