//! Audio and media commands run on a worker, with snapshots delivered to Home.
use std::{
    process::Command,
    sync::mpsc::{self, Receiver, RecvTimeoutError},
    thread,
    time::Duration,
};

use smithay::reexports::calloop::channel::{channel, Event};

use crate::{state::Backend, AnvilState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Audio {
    Output,
    Input,
}

impl Audio {
    fn devices(self) -> &'static str {
        match self {
            Self::Output => "sinks",
            Self::Input => "sources",
        }
    }

    fn device(self) -> &'static str {
        match self {
            Self::Output => "sink",
            Self::Input => "source",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Device {
    pub id: String,
    pub name: String,
    pub volume: u16,
    pub muted: bool,
    pub default: bool,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct Media {
    pub player: String,
    pub artist: String,
    pub title: String,
    pub playing: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub output: Vec<Device>,
    pub input: Vec<Device>,
    pub media: Option<Media>,
}

impl Snapshot {
    pub(super) fn devices(&self, audio: Audio) -> &[Device] {
        match audio {
            Audio::Output => &self.output,
            Audio::Input => &self.input,
        }
    }
}

#[derive(Debug)]
pub(super) enum Request {
    Watch(bool),
    Select(Audio, String),
    Volume(Audio, String, i8),
    Mute(Audio, String),
    Transport(String, &'static str),
    PauseRed,
}

fn command(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new("timeout")
        .args(["-k", "0.2s", "0.9s", program])
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn audio_devices(audio: Audio) -> Vec<Device> {
    let default = command("pactl", &[&format!("get-default-{}", audio.device())]);
    let Some(text) = command("pactl", &["list", audio.devices()]) else {
        return Vec::new();
    };
    parse_devices(&text, default.as_deref(), audio)
}

fn parse_devices(text: &str, default: Option<&str>, audio: Audio) -> Vec<Device> {
    let mut devices = Vec::new();
    for block in text.split("\n\n") {
        let mut id = "";
        let mut name = "";
        let mut monitor = false;
        let mut muted = false;
        let mut volume = 0;
        for line in block.lines().map(str::trim) {
            if let Some(value) = line.strip_prefix("Name: ") {
                id = value;
            } else if let Some(value) = line.strip_prefix("Description: ") {
                name = value;
            } else if let Some(value) = line.strip_prefix("Mute: ") {
                muted = value == "yes";
            } else if let Some(value) = line.strip_prefix("Volume: ") {
                let levels: Vec<u16> = value
                    .split_whitespace()
                    .filter_map(|part| part.strip_suffix('%')?.parse().ok())
                    .collect();
                if !levels.is_empty() {
                    volume = ((levels.iter().map(|level| u32::from(*level)).sum::<u32>())
                        / levels.len() as u32) as u16;
                }
            } else if line
                .strip_prefix("Monitor of Sink: ")
                .is_some_and(|value| value != "n/a")
            {
                monitor = true;
            }
        }
        if id.is_empty() || (audio == Audio::Input && (monitor || id.ends_with(".monitor"))) {
            continue;
        }
        devices.push(Device {
            id: id.into(),
            name: if name.is_empty() { id } else { name }.into(),
            volume,
            muted,
            default: default == Some(id),
        });
    }
    // Keep positions stable when the default changes under the cursor.
    devices.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    devices
}

fn media() -> Option<Media> {
    let data = command(
        "playerctl",
        &[
            "-a",
            "metadata",
            "--format",
            "{{playerInstance}}\t{{status}}\t{{artist}}\t{{title}}",
        ],
    )?;
    data.lines()
        .filter_map(|line| {
            let columns: Vec<_> = line.splitn(4, '\t').collect();
            let [player, status, artist, title] = columns[..] else {
                return None;
            };
            if player.is_empty() || title.is_empty() {
                return None;
            }
            Some(Media {
                player: player.into(),
                artist: artist.into(),
                title: title.into(),
                playing: status == "Playing",
            })
        })
        .max_by_key(|track| track.playing)
}

fn perform(request: Request, watch: &mut bool) -> Result<(), &'static str> {
    let result = match request {
        Request::Watch(active) => {
            *watch = active;
            return Ok(());
        }
        Request::Select(kind, id) => {
            command("pactl", &[&format!("set-default-{}", kind.device()), &id])
        }
        Request::Volume(kind, id, step) => command(
            "pactl",
            &[
                &format!("set-{}-volume", kind.device()),
                &id,
                &format!("{step:+}%"),
            ],
        ),
        Request::Mute(kind, id) => command(
            "pactl",
            &[&format!("set-{}-mute", kind.device()), &id, "toggle"],
        ),
        Request::Transport(player, action) => command("playerctl", &["--player", &player, action]),
        Request::PauseRed => {
            // Red may already have closed by the time Home requests a pause.
            let _ = command("playerctl", &["--player", "red", "pause"]);
            return Ok(());
        }
    };
    result
        .map(|_| ())
        .ok_or("The device or media player is unavailable.")
}

enum Update {
    Snapshot(Snapshot),
    Error(&'static str),
}

fn watch(
    requests: Receiver<Request>,
    updates: smithay::reexports::calloop::channel::Sender<Update>,
) {
    let mut active = false;
    let mut cached = Snapshot::default();
    loop {
        let request = if active {
            match requests.recv_timeout(Duration::from_millis(1250)) {
                Ok(request) => Some(request),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            let Ok(request) = requests.recv() else { break };
            Some(request)
        };
        for request in request.into_iter().chain(requests.try_iter().take(16)) {
            if let Err(error) = perform(request, &mut active) {
                if updates.send(Update::Error(error)).is_err() {
                    return;
                }
            }
        }
        if active {
            let snapshot = Snapshot {
                output: audio_devices(Audio::Output),
                input: audio_devices(Audio::Input),
                media: media(),
            };
            if snapshot != cached {
                cached = snapshot;
                let update = Snapshot {
                    output: cached.output.clone(),
                    input: cached.input.clone(),
                    media: cached.media.clone(),
                };
                if updates.send(Update::Snapshot(update)).is_err() {
                    break;
                }
            }
        }
    }
}

pub(super) fn install<B: Backend + 'static>(state: &mut AnvilState<B>) {
    let (updates, receive) = channel();
    if let Err(error) = state.handle.insert_source(receive, |event, _, state| {
        if let Event::Msg(update) = event {
            match update {
                Update::Snapshot(snapshot) => {
                    state.home.services = snapshot;
                    state.home.changed();
                }
                Update::Error(error) => state.home.control_error(error),
            }
        }
    }) {
        tracing::warn!(%error, "Cannot update audio and media panels");
        return;
    }
    let (send, requests) = mpsc::channel();
    state.home.service_requests = Some(send);
    thread::spawn(move || watch(requests, updates));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulse_devices_exclude_monitors_and_keep_focus_order() {
        let input = "Source #1\n Name: z\n Description: USB mic\n Monitor of Sink: n/a\n Mute: yes\n Volume: front-left: 65536 / 100% / 0 dB, front-right: 32768 / 50% / -6 dB\n\nSource #2\n Name: a.monitor\n Description: Monitor\n Monitor of Sink: 1\n\nSource #3\n Name: b\n Description: Headset\n Monitor of Sink: n/a\n Mute: no";
        let devices = parse_devices(input, Some("z"), Audio::Input);
        assert_eq!(devices.len(), 2);
        assert_eq!(devices[0].id, "b");
        assert_eq!(devices[1].volume, 75);
        assert!(devices[1].muted && devices[1].default);
    }
}
