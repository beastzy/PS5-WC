use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

use cairo::ImageSurface;
use tracing::info;

use super::Home;

const REVEAL: f64 = 0.30;
const READY_HOLD: Duration = Duration::from_millis(500);
const WINDOW_TIMEOUT: Duration = Duration::from_secs(150);

#[derive(Debug)]
pub(crate) struct Launch {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) serial: u64,
    pub(crate) ready_at: Option<Instant>,
    reveal_game: bool,
    art: Option<PathBuf>,
    requested: Instant,
    revealed_at: Option<Instant>,
}

impl Home {
    pub(crate) fn begin_launch(&mut self, id: String, title: String, art: Option<PathBuf>) -> u64 {
        self.changed();
        let serial = self.version;
        self.launch = Some(Launch {
            id,
            title,
            art,
            serial,
            requested: Instant::now(),
            revealed_at: None,
            ready_at: None,
            reveal_game: false,
        });
        self.notice = None;
        self.sync_music();
        serial
    }

    pub(crate) fn launch_error(&mut self, serial: u64, error: String) {
        if self
            .launch
            .as_ref()
            .is_some_and(|launch| launch.serial == serial && launch.ready_at.is_none())
        {
            self.cancel_launch();
            self.show_error(error);
        }
    }

    pub(super) fn check_launch(&mut self, now: Instant) {
        let Some(launch) = &mut self.launch else {
            return;
        };
        if let Some(ready_at) = launch.ready_at {
            launch.reveal_game = now.saturating_duration_since(ready_at) >= READY_HOLD;
        } else if now.saturating_duration_since(launch.requested) >= WINDOW_TIMEOUT {
            let serial = launch.serial;
            let error = format!(
                "{} did not open a window. Check Steam and retry.",
                launch.title
            );
            self.launch_error(serial, error);
        }
    }

    pub(super) fn cancel_launch(&mut self) {
        if self.launch.take().is_some() {
            self.sync_music();
            self.changed();
        }
    }

    pub(crate) fn game_ready(&mut self) {
        if let Some(launch) = &mut self.launch {
            let ready_at = Instant::now();
            info!(
                game = %launch.title,
                app_id = %launch.id,
                play_to_surface_ms = ready_at.duration_since(launch.requested).as_millis(),
                "Game surface ready; holding launch cover"
            );
            launch.ready_at = Some(ready_at);
            self.visible = false;
            self.library.open = false;
            self.overlay = super::Overlay::None;
            self.carousel_motion = None;
            self.changed();
        }
    }

    pub(crate) fn cover_underlay(&self) -> bool {
        self.launch
            .as_ref()
            .is_some_and(|launch| launch.reveal_game)
    }

    pub(crate) fn presented_game(&mut self) {
        if self.cover_underlay() {
            if let Some(launch) = self.launch.take() {
                if let Some(ready_at) = launch.ready_at {
                    let presented_at = Instant::now();
                    info!(
                        app_id = %launch.id,
                        hold_ms = presented_at.duration_since(ready_at).as_millis(),
                        play_to_visible_ms = presented_at.duration_since(launch.requested).as_millis(),
                        "Game frame presented; removed launch cover"
                    );
                }
            }
            self.changed();
        }
    }

    pub(super) fn launch_is_animating(&self, now: Instant) -> bool {
        self.launch.as_ref().is_some_and(|launch| {
            launch.ready_at.is_some()
                || (!self.reduced_motion
                    && launch.revealed_at.is_none_or(|start| {
                        now.saturating_duration_since(start).as_secs_f64() < REVEAL
                    }))
        })
    }

    pub(super) fn launch_cover_clip(&mut self, now: Instant, width: i32) -> Option<(u64, i32)> {
        let (serial, art, ready) = self
            .launch
            .as_ref()
            .map(|launch| (launch.serial, launch.art.clone(), launch.ready_at.is_some()))?;
        let art_ready = art.as_deref().and_then(|path| self.art(path)).is_some();
        if ready || self.reduced_motion {
            return Some((serial, width));
        }
        if art
            .as_ref()
            .is_some_and(|path| self.queued_art.contains(path))
        {
            return None;
        }
        if !art_ready {
            return Some((serial, width));
        }
        let started = self.launch.as_mut()?.revealed_at.get_or_insert(now);
        let elapsed = now.saturating_duration_since(*started).as_secs_f64();
        let t = (elapsed / REVEAL).clamp(0.0, 1.0);
        let fraction = 1.0 - (1.0 - t).powi(3);
        Some((serial, (f64::from(width) * fraction).round() as i32))
    }

    pub(super) fn launch_art(&mut self) -> Option<ImageSurface> {
        let art = self.launch.as_ref().and_then(|launch| launch.art.clone());
        art.as_deref().and_then(|path| self.art(path))
    }
}
