//! Use native seat devices in the TTY session and SDL in windowed previews.
#[cfg(feature = "udev")]
mod linux;
use std::{
    collections::HashMap,
    ffi::{c_char, c_void, CStr},
    ptr::NonNull,
    time::{Duration, Instant},
};

use crate::{apps::RedWindow, focus::KeyboardFocusTarget, home::HomeAction, state::Backend, AnvilState};
use smithay::{
    backend::input::KeyState,
    input::keyboard::{FilterResult, Keysym},
    reexports::calloop::timer::{TimeoutAction, Timer},
    utils::SERIAL_COUNTER,
};

const GAMEPAD: u32 = 0x0000_2000;
const GAMEPAD_BUTTON_DOWN: u32 = 0x651;
const GAMEPAD_BUTTON_UP: u32 = 0x652;
const GAMEPAD_ADDED: u32 = 0x653;
const GAMEPAD_REMOVED: u32 = 0x654;
const GUIDE: u8 = 5;

#[derive(Clone, Default, PartialEq, Eq)]
struct Devices {
    count: usize,
    battery: Option<u8>,
    ids: Vec<(u16, u16)>,
}
const HOLD: Duration = Duration::from_millis(550);

#[derive(Clone, Copy)]
enum PadAction {
    Key(Keysym),
    RedKey(u32),
    Control,
    Home,
    Create,
}

#[derive(Default)]
struct Guide {
    pressed: Option<Instant>,
    held: bool,
}

impl Guide {
    fn button(&mut self, down: bool, now: Instant) -> Option<PadAction> {
        if down {
            self.pressed.get_or_insert(now);
            return None;
        }
        let deadline = self.pressed.take()? + HOLD;
        if std::mem::take(&mut self.held) {
            return None;
        }
        Some(if now < deadline {
            PadAction::Control
        } else {
            PadAction::Home
        })
    }

    fn tick(&mut self, now: Instant) -> Option<PadAction> {
        if !self.held && self.pressed.is_some_and(|pressed| now >= pressed + HOLD) {
            self.held = true;
            return Some(PadAction::Home);
        }
        None
    }
}

fn dispatch<B: Backend>(state: &mut AnvilState<B>, action: PadAction) {
    let action = match action {
        PadAction::Control => HomeAction::ToggleControl,
        PadAction::Home => HomeAction::ShowHome,
        PadAction::Create => HomeAction::OpenCreate,
        PadAction::Key(key) if owns_gamepad(state) => state.home.press_gamepad(key),
        PadAction::RedKey(code) if owns_red_gamepad(state) => {
            let Some(keyboard) = state.seat.get_keyboard() else { return };
            let time = state.clock.now().as_millis();
            // Smithay takes XKB keycodes, which are eight above evdev codes.
            for direction in [KeyState::Pressed, KeyState::Released] {
                keyboard.input::<(), _>(
                    state,
                    (code + 8).into(),
                    direction,
                    SERIAL_COUNTER.next_serial(),
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            return;
        }
        PadAction::Key(_) | PadAction::RedKey(_) => return,
    };
    state.apply_home_action(action);
}

#[repr(C)]
#[derive(Clone, Copy)]
struct GamepadEvent {
    kind: u32,
    _reserved: u32,
    _timestamp: u64,
    instance: u32,
    button: u8,
}

#[repr(C, align(8))]
union Event {
    gamepad: GamepadEvent,
    bytes: [u8; 128],
}

#[link(name = "SDL3")]
extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> bool;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_GetError() -> *const c_char;
    fn SDL_SetHint(name: *const c_char, value: *const c_char) -> bool;
    fn SDL_GetGamepadVendorForID(instance: u32) -> u16;
    fn SDL_GetGamepadProductForID(instance: u32) -> u16;
    fn SDL_OpenGamepad(instance: u32) -> *mut c_void;
    fn SDL_CloseGamepad(gamepad: *mut c_void);
    fn SDL_GetGamepadButton(gamepad: *mut c_void, button: i32) -> bool;
    fn SDL_GetGamepadPowerInfo(gamepad: *mut c_void, percent: *mut i32) -> i32;
    fn SDL_PollEvent(event: *mut Event) -> bool;
}

struct Pad {
    handle: NonNull<c_void>,
    held_on_open: u32,
    guide: Guide,
    identity: (u16, u16),
}

struct Gamepads(HashMap<u32, Pad>);

impl Gamepads {
    fn new() -> Option<Self> {
        unsafe {
            // Leave session termination signals with the compositor.
            SDL_SetHint(c"SDL_NO_SIGNAL_HANDLERS".as_ptr(), c"1".as_ptr());
            // Focus is managed by Smithay. SDL has no window on this event loop.
            SDL_SetHint(
                c"SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS".as_ptr(),
                c"1".as_ptr(),
            );
            if !SDL_InitSubSystem(GAMEPAD) {
                let error = CStr::from_ptr(SDL_GetError()).to_string_lossy();
                tracing::warn!(%error, "Controller input unavailable");
                return None;
            }
        }
        Some(Self(HashMap::new()))
    }

    fn poll(
        &mut self,
        native: &Devices,
        menu: bool,
        red: bool,
        mut on_action: impl FnMut(PadAction),
    ) -> bool {
        let mut devices_changed = false;
        let mut event = Event { bytes: [0; 128] };
        while unsafe { SDL_PollEvent(&mut event) } {
            // These SDL gamepad events all begin with the type and instance ID.
            let event = unsafe { event.gamepad };
            match event.kind {
                GAMEPAD_ADDED => {
                    let identity = unsafe {
                        (
                            SDL_GetGamepadVendorForID(event.instance),
                            SDL_GetGamepadProductForID(event.instance),
                        )
                    };
                    if native.ids.contains(&identity) {
                        continue;
                    }
                    if let std::collections::hash_map::Entry::Vacant(entry) =
                        self.0.entry(event.instance)
                    {
                        if let Some(pad) = NonNull::new(unsafe { SDL_OpenGamepad(event.instance) })
                        {
                            // A button already held by a game must be released before navigating Home.
                            let held_on_open = (0..=14)
                                .filter(|&button| unsafe {
                                    SDL_GetGamepadButton(pad.as_ptr(), button)
                                })
                                .fold(0, |held, button| held | (1 << button));
                            entry.insert(Pad {
                                handle: pad,
                                held_on_open,
                                guide: Guide::default(),
                                identity,
                            });
                            devices_changed = true;
                        }
                    }
                }
                GAMEPAD_REMOVED => {
                    if let Some(pad) = self.0.remove(&event.instance) {
                        unsafe { SDL_CloseGamepad(pad.handle.as_ptr()) };
                        devices_changed = true;
                    }
                }
                GAMEPAD_BUTTON_DOWN | GAMEPAD_BUTTON_UP => {
                    if event.button >= 32 {
                        continue;
                    }
                    let Some(pad) = self.0.get_mut(&event.instance) else {
                        continue;
                    };
                    let button = 1 << event.button;
                    let down = event.kind == GAMEPAD_BUTTON_DOWN;
                    if !native.ids.contains(&pad.identity) && pad.held_on_open & button == 0 {
                        if event.button == GUIDE {
                            if let Some(action) = pad.guide.button(down, Instant::now()) {
                                on_action(action);
                            }
                        } else if !down && event.button == 4 && pad.identity.0 == 0x054c {
                            on_action(PadAction::Create);
                        } else if down && red && !(native.count > 0 && pad.identity.0 == 0x28de) {
                            if let Some(code) = sdl_red_key(event.button) {
                                on_action(PadAction::RedKey(code));
                            }
                        } else if down && menu {
                            if let Some(key) = button_key(event.button) {
                                on_action(PadAction::Key(key));
                            }
                        }
                    }
                    if !down {
                        pad.held_on_open &= !button;
                    }
                }
                _ => (),
            }
        }
        let now = Instant::now();
        for pad in self.0.values_mut() {
            if native.ids.contains(&pad.identity) {
                pad.guide = Guide::default();
            } else if let Some(action) = pad.guide.tick(now) {
                on_action(action);
            }
        }
        devices_changed
    }

    fn battery(&self, native: &Devices) -> Option<u8> {
        self.0
            .iter()
            .filter(|(_, pad)| !native.ids.contains(&pad.identity))
            .min_by_key(|(id, _)| *id)
            .and_then(|(_, pad)| {
                let mut percent = -1;
                unsafe { SDL_GetGamepadPowerInfo(pad.handle.as_ptr(), &mut percent) };
                (0..=100).contains(&percent).then_some(percent as u8)
            })
    }
}

impl Drop for Gamepads {
    fn drop(&mut self) {
        for (_, pad) in self.0.drain() {
            unsafe { SDL_CloseGamepad(pad.handle.as_ptr()) };
        }
        unsafe { SDL_QuitSubSystem(GAMEPAD) };
    }
}

fn button_key(button: u8) -> Option<Keysym> {
    Some(match button {
        0 => Keysym::Return,       // south (A / Cross)
        1 => Keysym::Escape,       // east (B / Circle)
        6 => Keysym::Menu,         // Start / Options
        9 => Keysym::ISO_Left_Tab, // left shoulder
        10 => Keysym::Tab,         // right shoulder
        11 => Keysym::Up,
        12 => Keysym::Down,
        13 => Keysym::Left,
        14 => Keysym::Right,
        _ => return None,
    })
}

fn sdl_red_key(button: u8) -> Option<u32> {
    red_key(match button {
        0 => 0x130, // south
        1 => 0x131, // east
        4 => 0x13a, // back
        6 => 0x13b, // start
        7 => 0x13d, // left stick click
        9 => 0x136, // left shoulder
        10 => 0x137, // right shoulder
        11..=14 => 0x220 + u32::from(button - 11),
        _ => return None,
    })
}

fn red_key(button: u32) -> Option<u32> {
    // Linux input codes for the Red TV shortcuts used by the original shell.
    Some(match button {
        0x130 => 28, // Enter
        0x131 => 1, // Esc
        0x136 => 62, // F4
        0x137 => 63, // F5
        0x138 => 60, // F2
        0x139 => 61, // F3
        0x13a => 12, // Minus
        0x13b => 13, // Equals
        0x13d => 50, // M
        0x220 => 103, // Up
        0x221 => 108, // Down
        0x222 => 105, // Left
        0x223 => 106, // Right
        _ => return None,
    })
}

pub(crate) fn install<B: Backend + 'static>(state: &AnvilState<B>, native: bool) {
    #[cfg(feature = "udev")]
    let controller = native.then(|| linux::install(state)).flatten();
    let mut previous_native = Devices::default();
    let mut gamepads = None;
    let mut had_focus = false;
    let period = Duration::from_millis(12);
    let mut next_battery = Instant::now();
    if let Err(error) =
        state
            .handle
            .insert_source(Timer::from_duration(period), move |_, _, state| {
                #[cfg(feature = "udev")]
                let native_devices = if let Some(controller) = &controller {
                    controller.update(state);
                    controller.devices()
                } else {
                    Devices::default()
                };
                #[cfg(not(feature = "udev"))]
                let native_devices = Devices::default();

                if !state.controller_active {
                    // Windowed previews must release controllers when the host takes focus.
                    gamepads = None;
                    had_focus = false;
                    return TimeoutAction::ToDuration(period);
                }
                if !had_focus {
                    gamepads = Gamepads::new();
                    had_focus = true;
                    next_battery = Instant::now();
                }
                let now = Instant::now();
                let menu = owns_gamepad(state);
                let red = owns_red_gamepad(state);
                let devices_changed = gamepads.as_mut().is_some_and(|pads| {
                    pads.poll(&native_devices, menu, red, |action| dispatch(state, action))
                });
                if devices_changed || previous_native != native_devices || now >= next_battery {
                    let fallback = gamepads.as_ref();
                    let count = native_devices.count
                        + fallback.map_or(0, |pads| {
                            pads.0
                                .values()
                                .filter(|pad| !native_devices.ids.contains(&pad.identity))
                                .count()
                        });
                    let battery = native_devices
                        .battery
                        .or_else(|| fallback.and_then(|pads| pads.battery(&native_devices)));
                    state.home.update_controller(count, battery);
                    previous_native = native_devices;
                    next_battery = now + Duration::from_secs(5);
                }
                TimeoutAction::ToDuration(period)
            })
    {
        tracing::warn!(%error, "Cannot watch controllers");
    }
}

fn owns_gamepad<B: Backend>(state: &AnvilState<B>) -> bool {
    state.controller_active && state.home.uses_keyboard() && state.home.launch.is_none()
}

fn owns_red_gamepad<B: Backend>(state: &AnvilState<B>) -> bool {
    state.controller_active && !state.home.uses_keyboard() && state.home.launch.is_none()
        && matches!(
            state.seat.get_keyboard().and_then(|keyboard| keyboard.current_focus()),
            Some(KeyboardFocusTarget::Window(window)) if window.user_data().get::<RedWindow>().is_some()
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guide_tap_opens_control_on_release() {
        let start = Instant::now();
        let release = start + Duration::from_millis(100);
        let mut guide = Guide::default();
        assert!(guide.button(true, start).is_none());
        assert!(guide.tick(release).is_none());
        assert!(matches!(
            guide.button(false, release),
            Some(PadAction::Control)
        ));
        assert!(guide.button(false, release).is_none());
    }

    #[test]
    fn guide_hold_opens_home_once_and_consumes_release() {
        let start = Instant::now();
        let mut guide = Guide::default();
        guide.button(true, start);
        assert!(matches!(guide.tick(start + HOLD), Some(PadAction::Home)));
        assert!(guide.tick(start + HOLD * 2).is_none());
        assert!(guide.button(false, start + HOLD * 2).is_none());
    }
}
