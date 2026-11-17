use std::{
    process::Command,
    thread,
    time::{Duration, Instant},
};

use cairo::Context;
use smithay::reexports::calloop::channel::{channel, Event};

use crate::{state::Backend, AnvilState};

use super::{
    visual::{carousel_easing, Layout},
    Home,
};

const FADE: f64 = 0.45;
const HOLD: f64 = 4.0;
const END: f64 = FADE * 2.0 + HOLD;

#[derive(Debug, Default)]
pub(super) struct Status {
    pub(super) mic: Option<bool>,
    pub(super) controller: bool,
    pub(super) battery: Option<u8>,
    pub(super) revision: u64,
    popup: Option<Popup>,
}

#[derive(Debug)]
struct Popup {
    started: Instant,
    controller: bool,
    microphone: bool,
    id: u64,
}

impl Status {
    pub(super) fn show(&mut self, controller: bool, microphone: bool, now: Instant) {
        // Preserve the other status card if the two updates arrived together.
        let recent = self
            .popup
            .as_ref()
            .filter(|popup| now.saturating_duration_since(popup.started).as_secs_f64() < FADE);
        self.revision = self.revision.wrapping_add(1);
        self.popup = Some(Popup {
            started: now,
            controller: controller || recent.is_some_and(|popup| popup.controller),
            microphone: microphone || recent.is_some_and(|popup| popup.microphone),
            id: self.revision,
        });
    }

    pub(super) fn opacity(&self, now: Instant) -> Option<(f64, u64)> {
        let popup = self.popup.as_ref()?;
        let elapsed = now.saturating_duration_since(popup.started).as_secs_f64();
        if elapsed >= END {
            return None;
        }
        let alpha = if elapsed < FADE {
            carousel_easing(elapsed / FADE)
        } else if elapsed > FADE + HOLD {
            1.0 - carousel_easing((elapsed - FADE - HOLD) / FADE)
        } else {
            1.0
        };
        Some((alpha, popup.id))
    }

    pub(super) fn moving(&self, now: Instant) -> bool {
        self.popup.as_ref().is_some_and(|popup| {
            let elapsed = now.saturating_duration_since(popup.started).as_secs_f64();
            elapsed < FADE || (FADE + HOLD..END).contains(&elapsed)
        })
    }
}

impl Home {
    pub(super) fn paint_peripherals(&mut self, cr: &Context, layout: Layout) -> Result<(), cairo::Error> {
        let Some(popup) = &self.status.popup else {
            return Ok(());
        };
        const WIDTH: f64 = 230.4;
        const SCALE: f64 = WIDTH / 256.0;
        let x = 2880.0 - 46.08 - WIDTH;
        let mut y = layout.y(207.0);
        let controller = popup.controller && self.status.controller;
        let microphone = popup.microphone;
        let battery = self.status.battery;
        if controller {
            self.icon(
                cr,
                "ui/status_notice_controller_bg.png",
                (x, y, WIDTH, 144.0 * SCALE),
            )?;
            let (gx, gy) = if battery.is_some() {
                (59.0, 47.0)
            } else {
                (93.0, 49.0)
            };
            self.icon(
                cr,
                "ui/status_notice_controller_icon.png",
                (x + gx * SCALE, y + gy * SCALE, 70.0 * SCALE, 46.0 * SCALE),
            )?;
            if let Some(battery) = battery {
                self.icon(
                    cr,
                    "ui/status_notice_battery_outline.png",
                    (x + 139.0 * SCALE, y + 65.0 * SCALE, 53.0 * SCALE, 34.0 * SCALE),
                )?;
                cr.set_source_rgb(1.0, 1.0, 1.0);
                for bar in [153.0, 164.0, 175.0]
                    .into_iter()
                    .take((usize::from(battery) * 3).div_ceil(100))
                {
                    cr.rectangle(x + bar * SCALE, y + 74.0 * SCALE, 7.0 * SCALE, 16.0 * SCALE);
                }
                cr.fill()?;
            }
            y += 144.0 * SCALE + layout.y(12.6);
        }
        if microphone {
            self.icon(
                cr,
                "ui/status_notice_microphone_bg.png",
                (x, y, WIDTH, 144.0 * SCALE),
            )?;
            self.icon(
                cr,
                "ui/status_notice_microphone_icon.png",
                (x + 132.0 * SCALE, y + 40.0 * SCALE, 44.0 * SCALE, 64.0 * SCALE),
            )?;
            if self.status.mic != Some(true) {
                let radius = 10.8 * SCALE;
                let nx = x + (154.0 + 44.0 * 0.78) * SCALE;
                let ny = y + (72.0 + 64.0 * 0.60) * SCALE;
                cr.set_source_rgba(0.95, 0.08, 0.12, 0.98);
                cr.set_line_width(4.0 * SCALE);
                cr.arc(nx, ny, radius, 0.0, std::f64::consts::TAU);
                cr.stroke()?;
                cr.move_to(nx - radius * 0.68, ny - radius * 0.68);
                cr.line_to(nx + radius * 0.68, ny + radius * 0.68);
                cr.stroke()?;
            }
        }
        Ok(())
    }
}

fn microphone_enabled() -> bool {
    let read = |tool: &str, args: &[&str]| {
        let output = Command::new("timeout")
            .args(["1.2s", tool])
            .args(args)
            .output()
            .ok()?;
        output
            .status
            .success()
            .then_some(String::from_utf8_lossy(&output.stdout).to_lowercase())
    };
    if let Some(volume) = read("wpctl", &["get-volume", "@DEFAULT_AUDIO_SOURCE@"]) {
        return !volume.contains("[muted]");
    }
    read("pactl", &["get-source-mute", "@DEFAULT_SOURCE@"]).is_some_and(|answer| answer.contains("mute: no"))
}

pub(super) fn monitor<B: Backend + 'static>(state: &AnvilState<B>) {
    let (send, receive) = channel();
    if let Err(error) = state.handle.insert_source(receive, |event, _, state| {
        if let Event::Msg(enabled) = event {
            state.home.update_microphone(enabled);
        }
    }) {
        tracing::warn!(%error, "Cannot monitor microphone state");
        return;
    }
    thread::spawn(move || loop {
        if send.send(microphone_enabled()).is_err() {
            return;
        }
        thread::sleep(Duration::from_secs(2));
    });
}
