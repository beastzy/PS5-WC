use std::time::{Duration, Instant};

use super::visual::{carousel_easing, ENTRANCE_DURATION, WELCOME_DURATION};

const REVEAL_PAUSE: f64 = 0.1;
const HEADER_DURATION: f64 = 0.65;
const INFO_DELAY: f64 = 0.5;
const INFO_DURATION: f64 = 1.5;
pub(super) const ENTRANCE_START: f64 = WELCOME_DURATION + REVEAL_PAUSE;
pub(super) const HEADER_START: f64 = ENTRANCE_START + ENTRANCE_DURATION;
pub(super) const NOTICES_START: f64 = HEADER_START + HEADER_DURATION;
pub(super) const END: f64 = NOTICES_START + INFO_DELAY + INFO_DURATION;

#[derive(Debug, Default)]
pub(super) struct Intro {
    pub(super) started: Option<Instant>,
    pub(super) home_cue: bool,
    pub(super) notices: bool,
    pub(super) done: bool,
    particle_detail: u8,
    sustained_lateness: u8,
    detail_changed_at: f64,
}

#[derive(Clone, Copy)]
pub(super) struct Phase {
    pub(super) welcome: Option<f64>,
    pub(super) entrance: Option<f64>,
    pub(super) header: f64,
    pub(super) info: f64,
}

impl Intro {
    pub(super) fn elapsed(&self, now: Instant) -> Option<f64> {
        self.started
            .map(|start| now.saturating_duration_since(start).as_secs_f64())
    }

    pub(super) fn observe_welcome_render(&mut self, now: Instant, work: Duration, period: Duration) {
        let Some(elapsed) = self.elapsed(now) else {
            return;
        };
        // Ignore initialization and isolated stalls. Keep the chosen quality for the
        // rest of this short animation so the effect does not switch back and forth.
        if self.done || !(0.35..WELCOME_DURATION).contains(&elapsed) || self.particle_detail == 4 {
            return;
        }
        self.sustained_lateness = if work > period.mul_f64(0.88) {
            self.sustained_lateness.saturating_add(1)
        } else {
            self.sustained_lateness.saturating_sub(1)
        };
        if self.sustained_lateness >= 5 && elapsed - self.detail_changed_at >= 0.24 {
            self.particle_detail += 1;
            self.detail_changed_at = elapsed;
            self.sustained_lateness = 0;
            tracing::debug!(
                detail = self.particle_detail,
                ?work,
                ?period,
                "Reduced Welcome particle work"
            );
        }
    }

    pub(super) fn particle_time(&self, elapsed: f64) -> f64 {
        if self.particle_detail == 4 {
            elapsed.min(self.detail_changed_at)
        } else {
            elapsed
        }
    }

    pub(super) fn particle_opacity(&self, index: usize, elapsed: f64) -> f64 {
        let stride = 1 << self.particle_detail.min(3);
        if index % stride == 0 {
            1.0
        } else if index % (stride / 2) == 0 && self.particle_detail < 4 {
            (1.0 - (elapsed - self.detail_changed_at) / 0.12).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    pub(super) fn phase(&self, now: Instant) -> Option<Phase> {
        if self.done {
            return None;
        }
        let elapsed = self.elapsed(now)?;
        let fade =
            |start: f64, duration: f64| carousel_easing(((elapsed - start) / duration).clamp(0.0_f64, 1.0));
        Some(Phase {
            welcome: (elapsed < WELCOME_DURATION).then_some(elapsed),
            entrance: (elapsed < HEADER_START).then_some((elapsed - ENTRANCE_START).max(0.0)),
            header: fade(HEADER_START, HEADER_DURATION),
            info: fade(NOTICES_START + INFO_DELAY, INFO_DURATION),
        })
    }
}
