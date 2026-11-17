use std::{
    f64::consts::{PI, TAU},
    path::PathBuf,
    time::{Duration, Instant},
};

use cairo::{Context, Extend, FontSlant, FontWeight, LinearGradient, RadialGradient};
use rand::{rngs::StdRng, Rng, SeedableRng};

use super::{
    image,
    native::{label, load_warm_gradient, Align},
    rounded, Entry, HeaderItem, Home, Page, HOME_LIMIT, SCALES, SETTINGS,
};

pub(super) const DESIGN_WIDTH: f64 = 2880.0;
pub(super) const DESIGN_HEIGHT: f64 = 1800.0;
pub(super) const WELCOME_DURATION: f64 = 6.0;
pub(super) const ENTRANCE_DURATION: f64 = 1.7;
const WHITE: (f64, f64, f64) = (1.0, 1.0, 1.0);
const GREY: (f64, f64, f64) = (0.60, 0.60, 0.60);
const TAB_GREY: (f64, f64, f64) = (0.55, 0.58, 0.61);
const SUBTITLE: (f64, f64, f64) = (0.75, 0.78, 0.84);
pub(super) const TILE: f64 = 229.0;
pub(super) const SELECTED: f64 = 292.0;
const GAP: f64 = 43.0;
const FIRST: f64 = 390.0;

#[derive(Clone, Copy, Debug)]
pub(super) struct CarouselPose {
    offset: f64,
    grow: [f64; HOME_LIMIT + 1],
}

impl CarouselPose {
    fn resting(selected: usize) -> Self {
        let offset = match selected {
            0 => 0.0,
            1 => SELECTED + GAP,
            _ => FIRST + TILE * 0.70 + (selected - 2) as f64 * (TILE + GAP),
        };
        let mut grow = [0.0; HOME_LIMIT + 1];
        grow[selected] = 1.0;
        Self { offset, grow }
    }
}

#[derive(Debug)]
pub(super) struct CarouselMotion {
    from: CarouselPose,
    to: CarouselPose,
    start: Instant,
    duration: Duration,
}

impl CarouselMotion {
    pub(super) fn new(from: CarouselPose, selected: usize, page: Page, start: Instant) -> Self {
        let duration = Duration::from_millis(if page == Page::Games { 340 } else { 260 });
        Self {
            from,
            to: CarouselPose::resting(selected),
            start,
            duration,
        }
    }

    pub(super) fn active(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) < self.duration
    }

    pub(super) fn detail_progress(&self, now: Instant) -> f64 {
        let fraction = now.saturating_duration_since(self.start).as_secs_f64() / self.duration.as_secs_f64();
        let progress = ((fraction - 0.10) / 0.74).clamp(0.0, 1.0);
        progress * progress * (3.0 - 2.0 * progress)
    }

    fn pose(&self, now: Instant) -> CarouselPose {
        let fraction = now.saturating_duration_since(self.start).as_secs_f64() / self.duration.as_secs_f64();
        if fraction >= 1.0 {
            return self.to;
        }
        let t = carousel_easing(fraction);
        let lerp = |from, to| from + (to - from) * t;
        CarouselPose {
            offset: lerp(self.from.offset, self.to.offset),
            grow: std::array::from_fn(|index| lerp(self.from.grow[index], self.to.grow[index])),
        }
    }
}

// CSS cubic-bezier(0.16, 1, 0.3, 1), matching the original carousel clock.
pub(super) fn carousel_easing(progress: f64) -> f64 {
    if progress <= 0.0 || progress >= 1.0 {
        return progress.clamp(0.0, 1.0);
    }
    let (mut low, mut high) = (0.0, 1.0);
    for _ in 0..16 {
        let t = (low + high) / 2.0;
        let remaining = 1.0 - t;
        let x = 3.0 * remaining * remaining * t * 0.16 + 3.0 * remaining * t * t * 0.3 + t * t * t;
        if x < progress {
            low = t;
        } else {
            high = t;
        }
    }
    let remaining: f64 = 1.0 - (low + high) / 2.0;
    1.0 - remaining.powi(3)
}

#[derive(Clone, Copy)]
pub(super) struct Rect(pub f64, pub f64, pub f64, pub f64);

impl Rect {
    pub(super) fn contains(self, x: f64, y: f64) -> bool {
        let Self(left, top, width, height) = self;
        (left..left + width).contains(&x) && (top..top + height).contains(&y)
    }

    fn fill(self, cr: &Context, radius: f64, (r, g, b, a): (f64, f64, f64, f64)) -> Result<(), cairo::Error> {
        let Self(x, y, width, height) = self;
        rounded(cr, x, y, width, height, radius);
        cr.set_source_rgba(r, g, b, a);
        cr.fill()
    }
}

#[derive(Clone, Copy)]
pub(super) struct Layout {
    pub height: f64,
}

impl Layout {
    pub(super) fn y(self, y: f64) -> f64 {
        self.height * y / DESIGN_HEIGHT
    }

    fn font(self, size: f64) -> f64 {
        size * (self.height / DESIGN_HEIGHT).min(1.0)
    }

    pub(super) fn header_hit(self, item: HeaderItem, active: Page) -> Rect {
        match item {
            HeaderItem::Games => Rect(85.0, self.y(49.0), 275.0, self.y(119.0)),
            HeaderItem::Apps => Rect(
                if active == Page::Games { 360.0 } else { 336.0 },
                self.y(49.0),
                238.0,
                self.y(119.0),
            ),
            HeaderItem::Search => Rect(1993.0, self.y(46.0), 124.0, self.y(124.0)),
            HeaderItem::Settings => Rect(2206.0, self.y(46.0), 116.0, self.y(124.0)),
            HeaderItem::Profile => Rect(2396.0, self.y(46.0), 124.0, self.y(124.0)),
        }
    }

    pub(super) fn tiles(
        self,
        pose: CarouselPose,
        len: usize,
        entrance: Option<f64>,
    ) -> impl Iterator<Item = (usize, Rect, f64)> {
        (0..=len)
            .scan(FIRST - pose.offset, move |x, index| {
                let grow = pose.grow[index];
                let width = TILE + (SELECTED - TILE) * grow;
                let start = *x;
                *x += width + GAP;
                let top = self.y(269.0) + 10.0 * grow;
                let (left, size, ring) = if let Some(elapsed) = entrance {
                    let lag = (index as f64 * 0.10).min(ENTRANCE_DURATION * 0.4);
                    let progress = ((elapsed - lag) / (ENTRANCE_DURATION - lag)).clamp(0.0, 1.0);
                    let eased = 1.0 - (1.0 - progress).powi(3);
                    let scale = 0.18 + 0.82 * eased;
                    (
                        start + (DESIGN_WIDTH + width - start) * (1.0 - eased),
                        width * scale,
                        grow * scale,
                    )
                } else {
                    (start, width, grow)
                };
                Some((index, Rect(left, top, size, size), ring))
            })
            .filter(|(_, rect, _)| rect.0 < DESIGN_WIDTH && rect.0 + rect.2 > 0.0)
    }

    pub(super) fn buttons(self, center: f64) -> (Rect, Rect) {
        let height = self.y(125.0);
        let x = center - (575.0 + 30.0 + height) / 2.0;
        let y = self.y(1560.0);
        (Rect(x, y, 575.0, height), Rect(x + 605.0, y, height, height))
    }

    pub(super) fn category(self, index: usize) -> Rect {
        Rect(143.0, self.y(201.0 + 117.0 * index as f64), 633.0, self.y(107.0))
    }

    pub(super) fn setting(self, index: usize) -> Rect {
        Rect(949.0, self.y(208.0 + 142.0 * index as f64), 1665.0, self.y(107.0))
    }

    pub(super) fn scale_choice(self, index: usize) -> Rect {
        Rect(2137.0, self.y(318.0 + 94.0 * index as f64), 393.0, self.y(94.0))
    }
}

impl Home {
    pub(super) fn carousel_pose(&self, now: Instant) -> CarouselPose {
        match &self.carousel_motion {
            Some(motion) => motion.pose(now),
            None => CarouselPose::resting(self.selected()),
        }
    }

    pub(super) fn info_center(&self) -> f64 {
        if self.page == Page::Games
            && self
                .entries()
                .get(self.selected())
                .is_some_and(|entry| entry.logo.is_some())
        {
            717.0
        } else {
            652.0
        }
    }

    pub(super) fn paint_main(
        &mut self,
        cr: &Context,
        layout: Layout,
        time: &str,
        now: Instant,
        entrance: Option<f64>,
    ) -> Result<(), cairo::Error> {
        self.paint_background(cr, layout)?;
        self.paint_header(cr, layout, time)?;
        if self.page == Page::Settings {
            return self.paint_display_settings(cr, layout);
        }

        self.paint_carousel(cr, layout, now, entrance)?;
        self.paint_info(cr, layout)
    }

    pub(super) fn paint_carousel(
        &mut self,
        cr: &Context,
        layout: Layout,
        now: Instant,
        entrance: Option<f64>,
    ) -> Result<(), cairo::Error> {
        let count = self.entries().len();
        let pose = self.carousel_pose(now);
        let can_draw_ring = self.action_focus == 0 && self.header_focus.is_none();
        for (index, rect, grow) in layout.tiles(pose, count, entrance) {
            let entry = self.entries().get(index).cloned();
            self.paint_carousel_tile(cr, rect, entry.as_ref(), if can_draw_ring { grow } else { 0.0 })?;
        }

        Ok(())
    }

    pub(super) fn info_bounds(&mut self, layout: Layout) -> Option<(f64, f64, f64, bool)> {
        let entry = self.entries().get(self.selected())?;
        let letters = entry.label.chars().take(28).count() as f64;
        let logo = entry.logo.clone();
        let center = self.info_center();
        let art = logo.as_deref().and_then(|path| self.art(path));
        let (title_half, title_top) = art.as_ref().map_or(
            (layout.font(56.0) * letters, layout.y(1140.0)),
            |logo| (435.0, layout.y(1348.0) - 864.0 * f64::from(logo.height()) / f64::from(logo.width()) - 8.0),
        );
        let half = title_half.max(440.0);
        Some(((center - half).max(0.0), title_top, center + half, art.is_some()))
    }

    pub(super) fn header_revision(&self) -> u64 {
        self.menu_size.to_bits()
            ^ (self.menu_weight as u64) << 32
            ^ (self.page as u64) << 11
            ^ (self.header_focus.map_or(0, |item| item as u64 + 1)) << 5
            ^ (u64::from(self.high_contrast) << 4)
            ^ self.art_revision.rotate_left(23)
    }

    pub(super) fn paint_info(&mut self, cr: &Context, layout: Layout) -> Result<(), cairo::Error> {
        let selected = self.selected();
        let Some(item) = self.entries().get(selected).cloned() else {
            return Ok(());
        };
        let center = self.info_center();
        let (action, more) = layout.buttons(center);
        let game = self.page == Page::Games;
        let running = game && self.running_games.contains(&item.id);
        if let Some(logo) = item.logo.as_deref().and_then(|path| self.art(path)) {
            let width = 864.0;
            let height = width * f64::from(logo.height()) / f64::from(logo.width());
            let bottom = layout.y(1348.0);
            image(
                cr,
                &logo,
                (center + 2.0 - width / 2.0, bottom - height, width, height),
                false,
            )?;
        } else {
            let title = item.label.chars().take(28).collect::<String>();
            label(
                cr,
                &title,
                (center, layout.y(1322.0)),
                layout.font(102.0),
                800,
                WHITE,
                Align::Center,
            );
        }
        label(
            cr,
            if running {
                "Running  ·  Steam"
            } else if game {
                "Ready To Play  ·  Steam"
            } else {
                "Ready To Open  ·  App"
            },
            (center, layout.y(1463.0)),
            layout.font(60.0),
            400,
            if self.high_contrast { WHITE } else { SUBTITLE },
            Align::Center,
        );
        let action_focused = self.action_focus == 1;
        let more_focused = self.action_focus == 2;
        let muted = (0.24, 0.24, 0.26, 0.4);
        action.fill(
            cr,
            action.3 / 2.0,
            if action_focused {
                (1.0, 1.0, 1.0, 1.0)
            } else {
                muted
            },
        )?;
        more.fill(
            cr,
            more.3 / 2.0,
            if more_focused { (1.0, 1.0, 1.0, 1.0) } else { muted },
        )?;
        let baseline = layout.y(1640.0);
        label(
            cr,
            if running { "Continue" } else if game { "Play" } else { "Open" },
            (action.0 + action.2 / 2.0, baseline),
            layout.font(48.0),
            700,
            if action_focused { (0.0, 0.0, 0.0) } else { WHITE },
            Align::Center,
        );
        label(
            cr,
            "...",
            (more.0 + more.2 / 2.0, baseline),
            layout.font(48.0),
            400,
            if more_focused { (0.0, 0.0, 0.0) } else { WHITE },
            Align::Center,
        );
        Ok(())
    }

    pub(super) fn paint_background(&mut self, cr: &Context, layout: Layout) -> Result<(), cairo::Error> {
        if self.page == Page::Settings {
            return self.paint_warm_gradient(cr, layout);
        }
        let (art, shade) = self.background_art();
        self.paint_background_art(cr, layout, art.as_ref(), shade)
    }

    pub(super) fn background_art(&mut self) -> (Option<cairo::ImageSurface>, bool) {
        let hero = self
            .games
            .get(self.game)
            .filter(|_| self.page == Page::Games && self.game < super::HOME_LIMIT)
            .and_then(|game| game.hero.clone());
        if let Some(art) = hero.as_deref().and_then(|path| self.art(path)) {
            (Some(art), true)
        } else {
            (self.art(&self.root.join("icons/ps-background.png")), false)
        }
    }

    pub(super) fn paint_background_art(
        &self,
        cr: &Context,
        layout: Layout,
        art: Option<&cairo::ImageSurface>,
        shade: bool,
    ) -> Result<(), cairo::Error> {
        cr.set_source_rgb(0.051, 0.067, 0.090);
        cr.paint()?;
        if let Some(art) = art {
            image(cr, art, (0.0, 0.0, DESIGN_WIDTH, layout.height), true)?;
        }
        let gradient = LinearGradient::new(0.0, 0.0, DESIGN_WIDTH, 0.0);
        if shade {
            gradient.add_color_stop_rgba(0.0, 0.02, 0.03, 0.05, 0.88);
            gradient.add_color_stop_rgba(0.55, 0.02, 0.03, 0.05, 0.55);
            gradient.add_color_stop_rgba(1.0, 0.02, 0.03, 0.05, 0.35);
        } else {
            gradient.add_color_stop_rgba(0.0, 0.02, 0.03, 0.05, 0.30);
            gradient.add_color_stop_rgba(1.0, 0.02, 0.03, 0.05, 0.10);
        }
        cr.set_source(&gradient)?;
        cr.paint()
    }

    pub(super) fn paint_warm_gradient(&mut self, cr: &Context, layout: Layout) -> Result<(), cairo::Error> {
        cr.set_source_rgb(0.055, 0.055, 0.060);
        cr.paint()?;
        let path = self.root.join("settings_startup_gradient.png");
        let art = self
            .art
            .entry(path.clone())
            .or_insert_with(|| load_warm_gradient(&path));
        if let Some(art) = art {
            cr.save()?;
            cr.scale(
                DESIGN_WIDTH / f64::from(art.width()),
                layout.height / f64::from(art.height()),
            );
            cr.set_source_surface(&art, 0.0, 0.0)?;
            cr.source().set_extend(Extend::Pad);
            cr.paint()?;
            cr.restore()?;
        }
        Ok(())
    }

    pub(super) fn paint_header(
        &mut self,
        cr: &Context,
        layout: Layout,
        time: &str,
    ) -> Result<(), cairo::Error> {
        let shift = if self.page == Page::Settings { -6.0 } else { 0.0 };
        let menu_scale = self.menu_size / 1.3;
        for (page, header, title, x) in [
            (Page::Games, HeaderItem::Games, "Games", 127.0),
            (
                Page::Apps,
                HeaderItem::Apps,
                "Apps",
                if self.page == Page::Games && !self.library.open { 410.0 } else { 374.0 },
            ),
        ] {
            let active = self.page == page && !self.library.open;
            let size = if active { 58.0 } else { 50.0 };
            let baseline = if active { 131.0 } else { 128.0 };
            if self.header_focus == Some(header) {
                let Rect(left, y, width, height) = layout.header_hit(header, self.header_page());
                self.focus_ring(cr, Rect(left + 13.0, y + 9.0, width - 26.0, height - 18.0), 24.0, 3.0, 1.0)?;
            }
            label(
                cr,
                title,
                (x, layout.y(baseline + shift)),
                layout.font(size) * menu_scale,
                if active {
                    700.max(self.menu_weight)
                } else {
                    self.menu_weight
                },
                if active || self.high_contrast { WHITE } else { TAB_GREY },
                Align::Left,
            );
        }
        for (item, path, icon) in [
            (
                HeaderItem::Search,
                "icons/search.png",
                Rect(2016.0, layout.y(67.0 + shift), 80.0, 80.0),
            ),
            (
                HeaderItem::Settings,
                "icons/settings.png",
                Rect(2229.0, layout.y(72.0 + shift), 72.0, 72.0),
            ),
        ] {
            if self.header_focus == Some(item) {
                let diameter = layout.y(97.0);
                let focus = Rect(
                    icon.0 + (icon.2 - diameter) / 2.0,
                    icon.1 + (icon.3 - diameter) / 2.0,
                    diameter,
                    diameter,
                );
                focus.fill(cr, diameter / 2.0, (0.91, 0.93, 0.97, 0.98))?;
                orbit(cr, focus, diameter / 2.0, 3.0)?;
                cr.push_group();
                let result = self.icon(cr, path, (icon.0, icon.1, icon.2, icon.3));
                let mask = cr.pop_group()?;
                result?;
                cr.set_source_rgb(0.02, 0.03, 0.05);
                cr.mask(mask)?;
            } else {
                self.icon(cr, path, (icon.0, icon.1, icon.2, icon.3))?;
            }
        }
        if self.header_focus == Some(HeaderItem::Profile) {
            orbit(cr, Rect(2413.0, layout.y(62.0 + shift), 90.0, 90.0), 20.0, 3.0)?;
        }
        let face = self.profile.clone();
        if let Some(art) = self.art(&face) {
            image(cr, &art, (2425.0, layout.y(74.0 + shift), 66.0, 66.0), true)?;
        }
        label(
            cr,
            time,
            (2794.0, layout.y(129.0 + shift)),
            layout.font(60.0),
            600,
            WHITE,
            Align::Right,
        );
        Ok(())
    }

    pub(super) fn paint_carousel_tile(
        &mut self,
        cr: &Context,
        rect: Rect,
        entry: Option<&Entry>,
        ring: f64,
    ) -> Result<(), cairo::Error> {
        self.paint_tile_body(cr, rect, entry, ring)?;
        self.paint_tile_focus(cr, rect, entry, ring)
    }

    pub(super) fn paint_tile_body(
        &mut self,
        cr: &Context,
        rect: Rect,
        entry: Option<&Entry>,
        ring: f64,
    ) -> Result<(), cairo::Error> {
        let Rect(x, y, size, _) = rect;
        cr.save()?;
        rounded(cr, x, y, size, size, 34.0);
        cr.clip();
        if self.page == Page::Apps || entry.is_some() {
            cr.set_source_rgba(0.11, 0.13, 0.16, 0.85);
            cr.paint()?;
        }
        if let Some(entry) = entry {
            if let Some(art) = entry.image.as_deref().and_then(|path| self.art(path)) {
                let inset = if self.page == Page::Apps { size * 0.21 } else { 0.0 };
                image(
                    cr,
                    &art,
                    (x + inset, y + inset, size - 2.0 * inset, size - 2.0 * inset),
                    self.page == Page::Games,
                )?;
            }
        } else {
            let icon_size = size * (1.29 + 0.47 * ring);
            let inset = (size - icon_size) / 2.0;
            self.icon(
                cr,
                "icons/game-library.png",
                (x + inset, y + inset, icon_size, icon_size),
            )?;
        }
        cr.restore()
    }

    pub(super) fn focus_ring(
        &self, cr: &Context, rect: Rect, radius: f64, stroke: f64, opacity: f64,
    ) -> Result<(), cairo::Error> {
        if self.high_contrast {
            rounded(cr, rect.0, rect.1, rect.2, rect.3, radius);
            cr.set_line_width(stroke.max(3.5) * 1.30);
            cr.set_source_rgba(1.0, 1.0, 1.0, opacity);
            cr.stroke()
        } else {
            orbit_with_opacity(cr, rect, radius, stroke, opacity)
        }
    }

    pub(super) fn paint_tile_focus(
        &self,
        cr: &Context,
        rect: Rect,
        entry: Option<&Entry>,
        ring: f64,
    ) -> Result<(), cairo::Error> {
        let Rect(x, y, size, _) = rect;
        if ring > 0.01 {
            self.focus_ring(cr, Rect(x - 8.0, y - 8.0, size + 16.0, size + 16.0), 34.0, 3.2, ring)?;
        }
        if ring > 0.5 {
            let name = entry.map_or(
                if self.page == Page::Games {
                    "Game Library"
                } else {
                    "App Library"
                },
                |entry| &entry.label,
            );
            let name = name.chars().take(24).collect::<String>();
            cr.select_font_face("SF Compact Display", FontSlant::Normal, FontWeight::Normal);
            cr.set_font_size(28.8);
            let bounds = cr.text_extents(&name)?;
            cr.set_source_rgba(1.0, 1.0, 1.0, ring);
            cr.move_to(x + (size - bounds.width()) / 2.0, y + size + 51.0);
            cr.show_text(&name.replace('\0', ""))?;
        }
        Ok(())
    }

    fn paint_display_settings(&mut self, cr: &Context, layout: Layout) -> Result<(), cairo::Error> {
        cr.set_source_rgba(1.0, 1.0, 1.0, 0.14);
        cr.rectangle(862.0, layout.y(196.0), 2.0, layout.height - layout.y(196.0));
        cr.fill()?;
        for (index, (group, _)) in SETTINGS.iter().enumerate() {
            let rect = layout.category(index);
            if self.header_focus.is_none() && index == self.category && !self.settings_options {
                settings_highlight(cr, rect, self.high_contrast)?;
            }
            label(
                cr,
                group,
                (192.0, layout.y(272.0 + index as f64 * 117.0)),
                layout.font(48.0),
                400,
                WHITE,
                Align::Left,
            );
        }
        for (index, name) in SETTINGS[self.category].1.iter().enumerate() {
            let rect = layout.setting(index);
            if self.header_focus.is_none() && self.settings_options && self.setting == index {
                settings_highlight(cr, rect, self.high_contrast)?;
            }
            let baseline = layout.y(278.0 + index as f64 * 142.0);
            label(
                cr,
                name,
                (1006.0, baseline),
                layout.font(48.0),
                400,
                WHITE,
                Align::Left,
            );
            let value = self.setting_value(index);
            label(
                cr,
                &value,
                (2443.0, baseline),
                layout.font(48.0),
                400,
                GREY,
                Align::Right,
            );
            if self.category == 0 && index == 0 {
                label(
                    cr,
                    "▾",
                    (2480.0, baseline),
                    layout.font(48.0),
                    400,
                    GREY,
                    Align::Center,
                );
            }
        }
        if let Some(selected) = self.scale_menu {
            let first = layout.scale_choice(0);
            Rect(first.0, first.1, first.2, first.3 * SCALES.len() as f64).fill(
                cr,
                22.0,
                (0.10, 0.12, 0.17, 0.99),
            )?;
            for (index, scale) in SCALES.iter().enumerate() {
                let rect = layout.scale_choice(index);
                if index == selected {
                    settings_highlight(cr, rect, self.high_contrast)?;
                }
                label(
                    cr,
                    &format!("{:.0} %", scale * 100.0),
                    (rect.0 + rect.2 / 2.0, rect.1 + rect.3 * 0.67),
                    layout.font(43.0),
                    400,
                    WHITE,
                    Align::Center,
                );
            }
        }
        Ok(())
    }
}

fn settings_highlight(cr: &Context, rect: Rect, contrast: bool) -> Result<(), cairo::Error> {
    cr.set_source_rgb(0.20, 0.25, 0.35);
    cr.rectangle(rect.0, rect.1, rect.2, rect.3);
    cr.fill()?;
    if contrast {
        rounded(cr, rect.0 + 2.0, rect.1 + 2.0, rect.2 - 4.0, rect.3 - 4.0, 28.0);
        cr.set_line_width(4.0);
        cr.set_source_rgb(1.0, 1.0, 1.0);
        return cr.stroke();
    }
    orbit(
        cr,
        Rect(rect.0 + 1.0, rect.1 + 1.0, rect.2 - 2.0, rect.3 - 2.0),
        28.0,
        2.0,
    )
}

pub(super) fn orbit(cr: &Context, rect: Rect, radius: f64, stroke: f64) -> Result<(), cairo::Error> {
    orbit_with_opacity(cr, rect, radius, stroke, 1.0)
}

fn orbit_with_opacity(
    cr: &Context,
    Rect(x, y, width, height): Rect,
    radius: f64,
    stroke: f64,
    opacity: f64,
) -> Result<(), cairo::Error> {
    rounded(cr, x, y, width, height, radius);
    cr.set_line_width(stroke * 2.1);
    cr.set_source_rgba(0.30, 0.38, 0.96, 0.10 * opacity);
    cr.stroke_preserve()?;
    cr.set_line_width(stroke);
    let tint = LinearGradient::new(x, y + height, x + width, y);
    tint.add_color_stop_rgba(0.0, 0.63, 0.77, 1.0, 0.78 * opacity);
    tint.add_color_stop_rgba(0.45, 0.39, 0.48, 0.88, 0.69 * opacity);
    tint.add_color_stop_rgba(1.0, 0.25, 0.33, 0.69, 0.70 * opacity);
    cr.set_source(&tint)?;
    cr.stroke()?;
    Ok(())
}

pub(super) fn profile(root: &std::path::Path) -> PathBuf {
    let user = gio::glib::user_name();
    [
        PathBuf::from("/var/lib/AccountsService/icons").join(user),
        gio::glib::home_dir().join(".face"),
        root.join("profile.png"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .unwrap_or_else(|| root.join("icons/control_center/cc-profile.png"))
}

#[derive(Debug)]
pub(super) struct Particle {
    target: (f64, f64),
    wave: (f64, f64),
    drift: (f64, f64),
    radius: f64,
    opacity: f64,
    gold: bool,
    phase: f64,
}

pub(super) fn particles() -> Vec<Particle> {
    let mut rng = StdRng::seed_from_u64(87);
    (0..140)
        .map(|_| {
            let mut normal = |center: f64, spread: f64| {
                let normal = (0..6).map(|_| rng.gen::<f64>()).sum::<f64>() - 3.0;
                (center + normal * spread * 2.0_f64.sqrt()).clamp(0.0, 1.0)
            };
            let clustered = normal(0.39, 0.22);
            let vertical = normal(0.49, 0.25);
            Particle {
                target: if rng.gen_bool(0.66) {
                    (clustered, vertical)
                } else {
                    (rng.gen(), rng.gen())
                },
                wave: (rng.gen(), rng.gen_range(-0.03..0.03)),
                drift: (rng.gen_range(-0.025..0.025), rng.gen_range(-0.025..0.025)),
                radius: if rng.gen_bool(0.08) {
                    rng.gen_range(2.4..4.4)
                } else {
                    rng.gen_range(0.3..2.2)
                },
                opacity: if rng.gen_bool(0.12) {
                    rng.gen_range(0.66..0.96)
                } else {
                    rng.gen_range(0.06..0.64)
                },
                gold: rng.gen_bool(0.18),
                phase: rng.gen_range(0.0..TAU),
            }
        })
        .collect()
}

pub(super) fn paint_launch_cover(
    cr: &Context,
    layout: Layout,
    art: Option<&cairo::ImageSurface>,
) -> Result<(), cairo::Error> {
    cr.set_source_rgb(0.0, 0.0, 0.0);
    cr.paint()?;
    if let Some(art) = art {
        image(cr, art, (0.0, 0.0, DESIGN_WIDTH, layout.height), true)?;
    }
    Ok(())
}

pub(super) fn paint_welcome(
    cr: &Context,
    layout: Layout,
    elapsed: f64,
    particles: &[Particle],
) -> Result<(), cairo::Error> {
    paint_welcome_word(cr, layout, elapsed)?;
    paint_welcome_particles(cr, layout, elapsed, particles)
}

pub(super) fn welcome_reveal(elapsed: f64) -> f64 {
    let progress = ((elapsed - 1.0) / 4.0).clamp(0.0, 1.0);
    (1.0 - (progress * PI).cos()) / 2.0
}

pub(super) fn paint_welcome_word(
    cr: &Context,
    layout: Layout,
    elapsed: f64,
) -> Result<(f64, f64), cairo::Error> {
    let reveal = welcome_reveal(elapsed);
    if reveal > 0.0 {
        cr.select_font_face("SF Compact Display", FontSlant::Normal, FontWeight::Bold);
        cr.set_font_size(layout.height * 0.055);
        let font = cr.font_extents()?;
        let advance = cr.text_extents("Welcome")?.x_advance();
        let left = (DESIGN_WIDTH - advance) / 2.0;
        let baseline = (layout.height + font.ascent() - font.descent()) / 2.0;
        let pad = layout.height * 0.012;
        cr.save()?;
        cr.rectangle(left - pad, 0.0, (advance + pad * 2.0) * reveal, layout.height);
        cr.clip();
        cr.move_to(left, baseline);
        cr.text_path("Welcome");
        cr.set_source_rgba(1.0, 0.84, 0.70, 0.12);
        cr.set_line_width(7.2);
        cr.stroke_preserve()?;
        cr.set_source_rgba(0.93, 0.92, 0.88, 0.18);
        cr.set_line_width(3.2);
        cr.stroke_preserve()?;
        let tint = LinearGradient::new(0.0, 0.0, DESIGN_WIDTH, 0.0);
        tint.add_color_stop_rgb(0.0, 0.99, 0.87, 0.77);
        tint.add_color_stop_rgb(0.48, 0.91, 0.91, 0.91);
        tint.add_color_stop_rgb(1.0, 0.73, 0.82, 0.96);
        cr.set_source(&tint)?;
        cr.fill()?;
        cr.restore()?;
        return Ok((left - pad, advance + pad * 2.0));
    }
    Ok((0.0, 0.0))
}

pub(super) fn paint_welcome_particles(
    cr: &Context,
    layout: Layout,
    elapsed: f64,
    particles: &[Particle],
) -> Result<(), cairo::Error> {
    for (_, particle, x, y, alpha) in visible_particles(layout, elapsed, particles) {
        particle.paint(cr, x, y, alpha)?;
    }
    Ok(())
}

pub(super) fn visible_particles(
    layout: Layout,
    elapsed: f64,
    particles: &[Particle],
) -> impl Iterator<Item = (usize, &Particle, f64, f64, f64)> {
    let spread = ((elapsed - 1.0) / 3.8).clamp(0.0, 1.0);
    let spread = spread * spread * (3.0 - 2.0 * spread);
    let drift = 1.0 - (-elapsed * 0.35).exp();
    particles.iter().enumerate().filter_map(move |(index, particle)| {
        let (wx, wy) = particle.wave;
        let fade = ((elapsed - wx * 0.82) / 0.16).clamp(0.0, 1.0);
        let opacity = particle.opacity * fade * (0.58 + 0.42 * (elapsed * 3.0 + particle.phase).sin());
        if opacity < 0.004 {
            return None;
        }
        let wave_y = 0.70 - 0.24 * wx + 0.075 * (wx * TAU - elapsed * 0.65).sin() + wy;
        let x = DESIGN_WIDTH
            * (wx * (1.0 - spread)
                + particle.target.0 * spread
                + particle.drift.0 * drift
                + 0.006 * (elapsed * 0.72 + particle.phase).sin());
        let y = layout.height
            * (wave_y * (1.0 - spread)
                + particle.target.1 * spread
                + particle.drift.1 * drift
                + 0.0045 * (elapsed * 0.55 + particle.phase * 1.37).sin());
        Some((index, particle, x, y, opacity))
    })
}

impl Particle {
    pub(super) fn extent(&self) -> f64 {
        if self.radius >= 1.8 {
            self.radius * 3.2
        } else {
            self.radius
        }
    }

    pub(super) fn paint(&self, cr: &Context, x: f64, y: f64, opacity: f64) -> Result<(), cairo::Error> {
        let (r, g, b) = if self.gold {
            (1.0, 0.73, 0.54)
        } else {
            (0.93, 0.95, 1.0)
        };
        if self.radius >= 1.8 {
            let radius = self.extent();
            let glow = RadialGradient::new(x, y, 0.0, x, y, radius);
            glow.add_color_stop_rgba(0.0, r, g, b, opacity * 0.22);
            glow.add_color_stop_rgba(1.0, r, g, b, 0.0);
            cr.rectangle(x - radius, y - radius, radius * 2.0, radius * 2.0);
            cr.set_source(&glow)?;
            cr.fill()?;
        }
        cr.arc(x, y, self.radius, 0.0, TAU);
        cr.set_source_rgba(r, g, b, opacity);
        cr.fill()
    }
}
