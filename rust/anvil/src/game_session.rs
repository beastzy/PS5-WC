//! Lifecycle commands for the process and audio guard of a mapped game.
use std::{
    fmt,
    io::Write,
    path::Path,
    process::{ChildStdin, Command, Stdio},
    thread,
};

use smithay::reexports::rustix::fs::{fcntl_getfl, fcntl_setfl, OFlags};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    Playing,
    Parked,
    Closing,
}

pub(crate) struct GameSession {
    pub(crate) mode: Mode,
    commands: Option<ChildStdin>,
}

impl fmt::Debug for GameSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GameSession")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl GameSession {
    pub(crate) fn start(
        root: &Path,
        app_id: &str,
        title: &str,
        pid: Option<u32>,
        wayland: Option<&str>,
        x11: Option<u32>,
    ) -> Self {
        let mut child = Command::new(root.join("bin/console-de-restore-game-audio"));
        child.args([
            app_id,
            title,
            &pid.map_or_else(String::new, |pid| pid.to_string()),
        ]);
        // Only discover game-tagged peers attached to this compositor.
        child.env_remove("WAYLAND_DISPLAY").env_remove("DISPLAY");
        if let Some(socket) = wayland {
            child.env("WAYLAND_DISPLAY", socket);
        }
        if let Some(display) = x11 {
            child.env("DISPLAY", format!(":{display}"));
        }
        let commands = match child.stdin(Stdio::piped()).spawn() {
            Ok(mut child) => {
                let commands = child.stdin.take().and_then(|pipe| {
                    let flags = fcntl_getfl(&pipe).ok()?;
                    fcntl_setfl(&pipe, flags | OFlags::NONBLOCK).ok()?;
                    Some(pipe)
                });
                thread::spawn(move || {
                    let _ = child.wait();
                });
                commands
            }
            Err(error) => {
                tracing::warn!(%error, app_id, "Could not start game session guard");
                None
            }
        };
        Self {
            mode: Mode::Playing,
            commands,
        }
    }

    pub(crate) fn set_mode(&mut self, mode: Mode) {
        if self.mode == mode {
            return;
        }
        if let Some(pipe) = &mut self.commands {
            let command = match mode {
                Mode::Playing => b"R",
                Mode::Parked => b"P",
                Mode::Closing => b"C",
            };
            if let Err(error) = pipe.write_all(command) {
                // EOF also thaws the game if the command channel fails.
                self.commands = None;
                tracing::warn!(%error, "Game session guard stopped accepting commands");
            }
        }
        self.mode = mode;
    }
}
