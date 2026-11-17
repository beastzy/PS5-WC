//! Steam is a per-user singleton. Bind its long-lived client to our display
//! before sending an app launch request.
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{state::Backend, AnvilState};
use smithay::reexports::{
    calloop::channel,
    rustix::{
        fs::OFlags,
        process::{kill_process, Pid, Signal},
    },
};

const BOOT_TIMEOUT: Duration = Duration::from_secs(40);
const STOP_TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Debug)]
pub(crate) struct Steam {
    send: Sender<Request>,
    requested_game: Arc<AtomicU64>,
}

#[derive(Debug)]
enum Request {
    Warm,
    Play { serial: u64, app_id: String },
}

impl Steam {
    pub(crate) fn play(&self, serial: u64, app_id: String) -> bool {
        self.requested_game.store(serial, Ordering::Relaxed);
        self.send.send(Request::Play { serial, app_id }).is_ok()
    }

    pub(crate) fn cancel_play(&self) {
        self.requested_game.store(0, Ordering::Relaxed);
    }
}

#[derive(Debug)]
struct Client {
    pid: u32,
    state: char,
    session: Option<String>,
    display: Option<String>,
    wayland: Option<String>,
    token: Option<String>,
}

#[derive(Debug)]
struct Session {
    display: String,
    wayland: String,
    id: Option<String>,
    token: String,
    native: bool,
    root: PathBuf,
    fifo: PathBuf,
    uid: u32,
}

fn field(env: &[u8], name: &str) -> Option<String> {
    env.split(|byte| *byte == 0).find_map(|entry| {
        let (key, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
        (key == name).then(|| value.to_owned())
    })
}

fn process_state(process: &Path) -> Option<char> {
    fs::read_to_string(process.join("status"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("State:"))?
        .trim()
        .chars()
        .next()
}

impl Session {
    fn clients(&self) -> Vec<Client> {
        fs::read_dir("/proc")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|entry| {
                let pid = entry.file_name().to_str()?.parse::<u32>().ok()?;
                let path = entry.path();
                if entry.metadata().ok()?.uid() != self.uid
                    || fs::read_to_string(path.join("comm")).ok()?.trim() != "steam"
                {
                    return None;
                }
                let state = process_state(&path)?;
                if matches!(state, 'Z' | 'X') {
                    return None;
                }
                let cmd = fs::read(path.join("cmdline")).ok()?;
                if cmd.split(|byte| *byte == 0).any(|arg| arg == b"-child-update-ui") {
                    return None;
                }
                let env = fs::read(path.join("environ")).ok()?;
                Some(Client {
                    pid,
                    state,
                    session: field(&env, "XDG_SESSION_ID"),
                    display: field(&env, "DISPLAY"),
                    wayland: field(&env, "WAYLAND_DISPLAY"),
                    token: field(&env, "CONSOLE_DE_STEAM_INSTANCE"),
                })
            })
            .collect()
    }

    fn owns(&self, client: &Client) -> bool {
        client.token.as_deref() == Some(self.token.as_str())
            || (client.token.is_none()
                && client.session == self.id
                && client.display.as_deref() == Some(self.display.as_str())
                && client.wayland.as_deref() == Some(self.wayland.as_str()))
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new("steam");
        command
            .args(args)
            .env("DISPLAY", &self.display)
            .env("WAYLAND_DISPLAY", &self.wayland)
            .env("CONSOLE_DE_STEAM_INSTANCE", &self.token)
            .env_remove("WAYLAND_SOCKET")
            .stdin(Stdio::null());

        match fs::read_to_string(self.root.join("gpu"))
            .as_deref()
            .map(str::trim)
        {
            Ok("nvidia") => {
                command
                    .env("__NV_PRIME_RENDER_OFFLOAD", "1")
                    .env("__GLX_VENDOR_LIBRARY_NAME", "nvidia");
            }
            Ok("amd") => {
                command.env("DRI_PRIME", "1");
            }
            _ => (),
        }
        command
    }

    fn spawn(&self, args: &[&str]) -> Result<Child, String> {
        let mut command = self.command(args);
        if fs::create_dir_all(self.root.join("state")).is_ok() {
            if let Ok(log) = OpenOptions::new()
                .create(true)
                .append(true)
                .mode(0o600)
                .open(self.root.join("state/steam.log"))
            {
                if let Ok(stdout) = log.try_clone() {
                    command.stdout(stdout).stderr(log);
                }
            }
        }
        command
            .spawn()
            .map_err(|error| format!("Could not start Steam: {error}"))
    }

    fn command_pipe_ready(&self) -> bool {
        fs::metadata(&self.fifo).is_ok_and(|meta| meta.file_type().is_fifo() && meta.uid() == self.uid)
            && OpenOptions::new()
                .write(true)
                .custom_flags(OFlags::NONBLOCK.bits() as i32)
                .open(&self.fifo)
                .is_ok()
    }

    fn wait_for_client(&self) -> Result<(), String> {
        let end = Instant::now() + BOOT_TIMEOUT;
        while Instant::now() < end {
            if self.clients().iter().any(|client| self.owns(client)) && self.command_pipe_ready() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(250));
        }
        Err("Steam did not become ready. Check Steam or its session log, then retry.".into())
    }

    fn ensure(&self) -> Result<(), String> {
        let clients = self.clients();
        if let Some(client) = clients.iter().find(|client| self.owns(client)) {
            resume(client);
            return self.wait_for_client();
        }
        if !clients.is_empty() {
            if !self.native
                || !clients.iter().all(|client| {
                    client
                        .session
                        .as_deref()
                        .is_some_and(|id| Some(id) != self.id.as_deref() && session_closed(id))
                        || (client.session == self.id && client.token.as_deref().is_some_and(instance_closed))
                })
            {
                return Err("Steam is open on another desktop. Exit it there and retry.".into());
            }
            for client in &clients {
                resume(client);
            }
            tracing::info!("Closing the Steam client left by a previous Console DE session");
            if self.command_pipe_ready() {
                wait_child(self.spawn(&["-shutdown"])?, STOP_TIMEOUT)?;
            } else {
                // The IPC endpoint may have died with the old display while its
                // Steam process remained. Only signal the verified stale clients.
                for client in &clients {
                    if let Some(pid) = Pid::from_raw(client.pid as i32) {
                        let _ = kill_process(pid, Signal::TERM);
                    }
                }
            }
            let end = Instant::now() + STOP_TIMEOUT;
            while Instant::now() < end && !self.clients().is_empty() {
                thread::sleep(Duration::from_millis(200));
            }
            if !self.clients().is_empty() {
                return Err(
                    "Steam from the previous session is still running. Exit Steam, then retry.".into(),
                );
            }
        }

        let mut process = self.spawn(&["-silent"])?;
        tracing::info!(pid = process.id(), display = %self.display, "Started Steam minimized");
        thread::spawn(move || {
            if let Ok(status) = process.wait() {
                if !status.success() {
                    tracing::warn!(%status, "Steam client exited with an error");
                }
            }
        });
        self.wait_for_client()
    }

    fn run(
        self,
        receive: mpsc::Receiver<Request>,
        events: channel::Sender<(Option<u64>, Result<(), String>)>,
        requested_game: Arc<AtomicU64>,
    ) {
        for request in receive {
            let serial = match &request {
                Request::Play { serial, .. } => Some(*serial),
                _ => None,
            };
            let result = self.ensure().and_then(|()| match &request {
                Request::Warm => {
                    tracing::info!(display = %self.display, "Steam is ready in the background");
                    Ok(())
                }
                Request::Play { app_id, serial } => {
                    if requested_game.load(Ordering::Relaxed) != *serial {
                        return Ok(());
                    }
                    tracing::info!(%app_id, display = %self.display, "Sending Steam launch request");
                    wait_child(
                        self.spawn(&["-silent", "-applaunch", app_id])?,
                        Duration::from_secs(15),
                    )
                }
            });
            if let Err(error) = &result {
                tracing::warn!(%error, ?request, "Steam request failed");
            }
            if events.send((serial, result)).is_err() {
                return;
            }
        }
    }
}

fn resume(client: &Client) {
    if matches!(client.state, 'T' | 't') {
        if let Some(pid) = Pid::from_raw(client.pid as i32) {
            let _ = kill_process(pid, Signal::CONT);
        }
    }
}

fn session_closed(id: &str) -> bool {
    let Ok(output) = Command::new("timeout")
        .args([
            "2s",
            "loginctl",
            "show-session",
            id,
            "--property=State",
            "--value",
        ])
        .output()
    else {
        return false;
    };
    if output.status.success() {
        return String::from_utf8_lossy(&output.stdout).trim() == "closing";
    }
    Command::new("timeout")
        .args(["2s", "loginctl", "list-sessions", "--no-legend"])
        .output()
        .is_ok_and(|sessions| {
            sessions.status.success()
                && !String::from_utf8_lossy(&sessions.stdout)
                    .lines()
                    .any(|line| line.split_whitespace().next() == Some(id))
        })
}

fn instance_closed(token: &str) -> bool {
    token
        .split_once('-')
        .and_then(|(pid, _)| pid.parse::<u32>().ok())
        .is_some_and(|pid| !Path::new("/proc").join(pid.to_string()).exists())
}

fn wait_child(mut child: Child, timeout: Duration) -> Result<(), String> {
    let end = Instant::now() + timeout;
    while Instant::now() < end {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                return Err(format!(
                    "Steam command failed ({status}). See the Steam session log."
                ))
            }
            Ok(None) => thread::sleep(Duration::from_millis(150)),
            Err(error) => return Err(format!("Could not check Steam startup: {error}")),
        }
    }
    // The command may be waiting for Steam's IPC reader. Leave the actual
    // Steam client alone and reap only this invocation.
    let _ = child.kill();
    let _ = child.wait();
    Err("Steam did not acknowledge the request. Try again once Steam is ready.".into())
}

pub(crate) fn install<B: Backend + 'static>(state: &mut AnvilState<B>, display: u32) {
    let Some(wayland) = state.socket_name.clone() else {
        return;
    };
    let Ok(meta) = fs::metadata("/proc/self") else {
        return;
    };
    let native = std::env::var("CONSOLE_DE_WAYLAND_MODE").as_deref() == Ok("native");
    let session = Session {
        display: format!(":{display}"),
        wayland,
        id: std::env::var("XDG_SESSION_ID").ok(),
        token: format!("{}-{}", std::process::id(), chrono::Utc::now().timestamp_micros()),
        native,
        root: state.home.root.clone(),
        fifo: gio::glib::home_dir().join(".steam/steam.pipe"),
        uid: meta.uid(),
    };
    let (events, receiver) = channel::channel::<(Option<u64>, Result<(), String>)>();
    if let Err(error) = state.handle.insert_source(receiver, |event, _, data| {
        if let channel::Event::Msg((serial, Err(error))) = event {
            if let Some(serial) = serial {
                data.home.launch_error(serial, error);
            }
        }
    }) {
        tracing::warn!(%error, "Cannot start Steam supervisor");
        return;
    }
    let (send, receive) = mpsc::channel();
    if native {
        let _ = send.send(Request::Warm);
    }
    let requested_game = Arc::new(AtomicU64::new(0));
    state.steam = Some(Steam {
        send,
        requested_game: requested_game.clone(),
    });
    thread::spawn(move || session.run(receive, events, requested_game));
}
