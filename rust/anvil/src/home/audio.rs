use std::{
    collections::HashMap,
    ffi::{c_char, c_void, CStr, CString},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::{self, NonNull},
    sync::mpsc::{self, RecvTimeoutError, Sender},
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum Cue {
    Home,
    Peripherals,
    Move,
    Select,
    Cancel,
    Control,
    OptionsOpen,
    OptionsClose,
    Dialog,
    Typing,
}

impl Cue {
    const ALL: [Self; 10] = [
        Self::Home,
        Self::Peripherals,
        Self::Move,
        Self::Select,
        Self::Cancel,
        Self::Control,
        Self::OptionsOpen,
        Self::OptionsClose,
        Self::Dialog,
        Self::Typing,
    ];

    fn asset(self) -> (&'static str, f32) {
        match self {
            Self::Home => ("PS5_Home.mp3", 4.0),
            Self::Peripherals => ("PS5_Peripherals.mp3", 2.1),
            Self::Move => ("PS5_Change.mp3", 4.0),
            Self::Select => ("PS5_Select.mp3", 2.1),
            Self::Cancel => ("Cancel.mp3", 2.1),
            Self::Control => ("PS5_Control_Center.mp3", 2.1),
            Self::OptionsOpen => ("Open Option Menu.mp3", 2.1),
            Self::OptionsClose => ("Close Option Menu.mp3", 2.1),
            Self::Dialog => ("Open Dialog.mp3", 2.1),
            Self::Typing => ("Text Input.mp3", 2.1),
        }
    }
}

#[derive(Debug)]
pub(super) struct Audio(Sender<Message>);

enum Message {
    Cue(Cue),
    Music(bool),
}

impl Audio {
    pub(super) fn start(root: &Path) -> Self {
        let (send, receive) = mpsc::channel();
        let sfx = root.join("assets/sfx");
        thread::spawn(move || run(sfx, receive));
        Self(send)
    }

    pub(super) fn cue(&self, cue: Cue) {
        let _ = self.0.send(Message::Cue(cue));
    }

    pub(super) fn music(&self, enabled: bool) {
        let _ = self.0.send(Message::Music(enabled));
    }
}

#[repr(C)]
#[derive(Default)]
struct SfInfo {
    frames: i64,
    sample_rate: i32,
    channels: i32,
    format: i32,
    sections: i32,
    seekable: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct AudioSpec {
    format: u32,
    channels: i32,
    frequency: i32,
}

const AUDIO: u32 = 0x10;
const DEFAULT_PLAYBACK: u32 = u32::MAX;
const FLOAT: u32 = if cfg!(target_endian = "little") {
    0x8120
} else {
    0x9120
};

#[link(name = "sndfile")]
extern "C" {
    fn sf_open(path: *const c_char, mode: i32, info: *mut SfInfo) -> *mut c_void;
    fn sf_close(file: *mut c_void) -> i32;
    fn sf_readf_float(file: *mut c_void, buffer: *mut f32, frames: i64) -> i64;
    fn sf_seek(file: *mut c_void, frames: i64, whence: i32) -> i64;
}

#[link(name = "SDL3")]
extern "C" {
    fn SDL_InitSubSystem(flags: u32) -> bool;
    fn SDL_QuitSubSystem(flags: u32);
    fn SDL_GetError() -> *const c_char;
    fn SDL_OpenAudioDevice(device: u32, spec: *const AudioSpec) -> u32;
    fn SDL_CloseAudioDevice(device: u32);
    fn SDL_CreateAudioStream(src: *const AudioSpec, dst: *const AudioSpec) -> *mut c_void;
    fn SDL_DestroyAudioStream(stream: *mut c_void);
    fn SDL_SetAudioStreamGain(stream: *mut c_void, gain: f32) -> bool;
    fn SDL_BindAudioStream(device: u32, stream: *mut c_void) -> bool;
    fn SDL_UnbindAudioStream(stream: *mut c_void);
    fn SDL_PutAudioStreamData(stream: *mut c_void, data: *const c_void, bytes: i32) -> bool;
    fn SDL_GetAudioStreamQueued(stream: *mut c_void) -> i32;
    fn SDL_ClearAudioStream(stream: *mut c_void) -> bool;
    fn SDL_FlushAudioStream(stream: *mut c_void) -> bool;
}

struct Track {
    file: NonNull<c_void>,
    info: SfInfo,
}

impl Track {
    fn open(path: &Path) -> Option<Self> {
        let path_arg = CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut info = SfInfo::default();
        let file = NonNull::new(unsafe { sf_open(path_arg.as_ptr(), 0x10, &mut info) });
        if file.is_none() {
            tracing::warn!(?path, "Could not decode Console DE audio");
        }
        Some(Self { file: file?, info })
    }

    fn spec(&self) -> AudioSpec {
        AudioSpec {
            format: FLOAT,
            channels: self.info.channels,
            frequency: self.info.sample_rate,
        }
    }

    fn read(&mut self, samples: &mut [f32]) -> usize {
        let channels = self.info.channels as usize;
        let frames = (samples.len() / channels) as i64;
        unsafe { sf_readf_float(self.file.as_ptr(), samples.as_mut_ptr(), frames).max(0) as usize * channels }
    }

    fn rewind(&mut self) -> bool {
        (unsafe { sf_seek(self.file.as_ptr(), 0, 0) }) == 0
    }
}

impl Drop for Track {
    fn drop(&mut self) {
        unsafe { sf_close(self.file.as_ptr()) };
    }
}

struct Device(u32);

impl Device {
    fn open() -> Option<Self> {
        unsafe {
            if SDL_InitSubSystem(AUDIO) {
                let id = SDL_OpenAudioDevice(DEFAULT_PLAYBACK, ptr::null());
                if id != 0 {
                    return Some(Self(id));
                }
                SDL_QuitSubSystem(AUDIO);
            }
            tracing::debug!(error = %CStr::from_ptr(SDL_GetError()).to_string_lossy(), "Console DE audio device unavailable");
        }
        None
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        unsafe {
            SDL_CloseAudioDevice(self.0);
            SDL_QuitSubSystem(AUDIO);
        }
    }
}

struct Stream(NonNull<c_void>);

impl Stream {
    fn new(spec: &AudioSpec, gain: f32) -> Option<Self> {
        let stream = NonNull::new(unsafe { SDL_CreateAudioStream(spec, ptr::null()) })?;
        unsafe { SDL_SetAudioStreamGain(stream.as_ptr(), gain) };
        Some(Self(stream))
    }

    fn bind(&self, device: &Device) -> bool {
        unsafe { SDL_BindAudioStream(device.0, self.0.as_ptr()) }
    }

    fn append(&self, samples: &[f32]) {
        if samples.is_empty() {
            return;
        }
        unsafe {
            SDL_PutAudioStreamData(
                self.0.as_ptr(),
                samples.as_ptr().cast(),
                std::mem::size_of_val(samples) as i32,
            )
        };
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        unsafe { SDL_DestroyAudioStream(self.0.as_ptr()) };
    }
}

struct Effect {
    spec: AudioSpec,
    data: Vec<f32>,
    gain: f32,
}

fn run(root: PathBuf, receiver: mpsc::Receiver<Message>) {
    let mut theme = Track::open(&root.join("PS5_Home_Menu.mp3"));
    let effects: HashMap<_, _> = Cue::ALL
        .into_iter()
        .filter_map(|cue| {
            let (name, gain) = cue.asset();
            let mut track = Track::open(&root.join(name))?;
            let mut data = vec![0.0; (track.info.frames * i64::from(track.info.channels)) as usize];
            let got = track.read(&mut data);
            data.truncate(got);
            Some((
                cue,
                Effect {
                    spec: track.spec(),
                    data,
                    gain,
                },
            ))
        })
        .collect();

    // Streams drop before the device. Decode/prebuffer on this worker, away from the renderer.
    let mut device = None;
    let mut music = None;
    let mut streams = HashMap::<Cue, Stream>::new();
    let mut playing = false;
    let mut requested = false;
    let mut pending = Vec::new();
    let mut samples = vec![0.0; 2048 * 2];
    let mut next_device_attempt = std::time::Instant::now();
    loop {
        match receiver.recv_timeout(Duration::from_millis(12)) {
            Ok(Message::Music(enabled)) => requested = enabled,
            Ok(Message::Cue(cue)) => pending.push(cue),
            Err(RecvTimeoutError::Timeout) => (),
            Err(RecvTimeoutError::Disconnected) => break,
        }
        if device.is_none() && std::time::Instant::now() >= next_device_attempt {
            device = Device::open();
            next_device_attempt = std::time::Instant::now() + Duration::from_millis(750);
            if device.is_some() {
                music = theme.as_ref().and_then(|track| Stream::new(&track.spec(), 0.25));
                tracing::debug!("Console DE audio mixer ready");
            }
        }
        let Some(device) = &device else {
            // Keep only the latest cue while the login audio service starts.
            if pending.len() > 1 {
                pending.drain(..pending.len() - 1);
            }
            continue;
        };
        for cue in pending.drain(..) {
            let Some(effect) = effects.get(&cue) else {
                continue;
            };
            if let std::collections::hash_map::Entry::Vacant(entry) = streams.entry(cue) {
                if let Some(stream) =
                    Stream::new(&effect.spec, effect.gain).filter(|stream| stream.bind(device))
                {
                    entry.insert(stream);
                }
            }
            if let Some(stream) = streams.get(&cue) {
                unsafe { SDL_ClearAudioStream(stream.0.as_ptr()) };
                stream.append(&effect.data);
                unsafe { SDL_FlushAudioStream(stream.0.as_ptr()) };
                tracing::debug!(?cue, "Queued Console DE sound");
            }
        }
        if let (Some(music), Some(track)) = (&music, &mut theme) {
            if requested != playing {
                if requested {
                    playing = music.bind(device);
                } else {
                    unsafe { SDL_UnbindAudioStream(music.0.as_ptr()) };
                    playing = false;
                }
            }
            if playing
                && unsafe { SDL_GetAudioStreamQueued(music.0.as_ptr()) } < 2048 * track.info.channels * 4
            {
                let mut read = track.read(&mut samples);
                if read == 0 && track.rewind() {
                    read = track.read(&mut samples);
                }
                music.append(&samples[..read]);
            }
        }
    }
}
