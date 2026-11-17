//! The full-screen All Games and All Apps pages.
use cairo::{Context, LinearGradient};
use smithay::input::keyboard::Keysym;

use super::{
    bounded, image,
    native::{label, wrapped_label, Align},
    ordered, rounded, steam_games,
    visual::{Layout, Rect, DESIGN_WIDTH},
    Entry, Home, HomeAction, Overlay, Page, HOME_LIMIT,
};
use std::{fs, time::Instant};

const COLUMNS: usize = 5;
const STAGGER: f64 = 0.018;
const APP_FADE: f64 = 0.24;
const GAME_FADE: f64 = 0.20;

#[derive(Debug, Default)]
pub(super) struct Library {
    pub(super) open: bool,
    pub(super) selected: usize,
    pub(super) offset: f64,
    pub(super) entered: Option<Instant>,
    pub(super) revision: u64,
    pub(super) games: Vec<Entry>,
    pub(super) apps: Vec<Entry>,
}

impl Library {
    pub(super) fn opacity(&self, now: Instant, index: usize, games: bool) -> f64 {
        let Some(start) = self.entered else {
            return 1.0;
        };
        let elapsed = now.saturating_duration_since(start).as_secs_f64();
        let (delay, duration) = if games {
            (0.0, GAME_FADE)
        } else {
            (index as f64 * STAGGER, APP_FADE)
        };
        let progress = ((elapsed - delay) / duration).clamp(0.0, 1.0);
        1.0 - (1.0 - progress).powi(3)
    }
}

#[derive(Clone, Copy)]
pub(super) struct Grid {
    pub(super) top: f64,
    bottom: f64,
    first: f64,
    size: f64,
    gap: f64,
    font_scale: f64,
    games: bool,
}

impl Grid {
    pub(super) fn new(layout: Layout, page: Page) -> Self {
        let games = page == Page::Games;
        let font_scale = (layout.height / super::visual::DESIGN_HEIGHT).min(1.0);
        let top = layout.y(144.0);
        let gap = layout.y(43.0);
        let heading = if games {
            layout.y(36.0) + 59.0 * font_scale + gap
        } else {
            0.0
        };
        Self {
            top,
            bottom: layout.height,
            first: top + layout.y(72.0) + heading,
            size: DESIGN_WIDTH * (0.88 - 0.02 * (COLUMNS - 1) as f64) / COLUMNS as f64,
            gap,
            font_scale,
            games,
        }
    }

    pub(super) fn size(self) -> f64 {
        self.size
    }

    pub(super) fn cell(self, index: usize, offset: f64) -> Rect {
        let x = DESIGN_WIDTH * 0.06 + (index % COLUMNS) as f64 * (self.size + DESIGN_WIDTH * 0.02);
        let y = self.first + (index / COLUMNS) as f64 * (self.size + self.gap) - offset;
        Rect(x, y, self.size, self.size)
    }

    pub(super) fn visible(self, count: usize, offset: f64) -> std::ops::Range<usize> {
        let stride = self.size + self.gap;
        let row = |y: f64| ((y + offset - self.first) / stride).floor().max(0.0) as usize;
        let start = row(self.top).saturating_mul(COLUMNS).min(count);
        let end = (row(self.bottom) + 1).saturating_mul(COLUMNS).min(count);
        start..end
    }

    fn max_offset(self, count: usize) -> f64 {
        if count == 0 {
            return 0.0;
        }
        let Rect(_, y, _, side) = self.cell(count - 1, 0.0);
        (y + side + self.bottom * 0.05 - self.bottom).max(0.0)
    }
}

impl Home {
    pub(super) fn open_library(&mut self) {
        self.library.open = true;
        self.library.selected = 0;
        self.library.offset = 0.0;
        self.library.entered = (!self.reduced_motion).then(Instant::now);
        self.library.revision = self.library.revision.wrapping_add(1);
        self.header_focus = None;
        self.action_focus = 0;
        self.carousel_motion = None;
        self.notice = None;
        if self.page == Page::Games {
            if let Some(request) = &self.game_catalog_requests {
                let _ = request.send(self.library.revision);
            } else {
                self.update_game_library(steam_games(&self.root));
            }
        }
        self.changed();
    }

    pub(super) fn update_game_library(&mut self, mut games: Vec<Entry>) {
        self.favorites.prioritize(Page::Games, &mut games);
        if self.games == games {
            return;
        }
        let previous = self
            .library
            .games
            .get(self.library.selected)
            .map(|entry| &entry.id);
        let mut catalog = games.clone();
        super::sort_catalog(&mut catalog);
        if self.library.open && self.page == Page::Games {
            self.library.selected = previous
                .and_then(|id| catalog.iter().position(|entry| &entry.id == id))
                .unwrap_or(0);
        }
        self.library.games = catalog;
        let was_library_tile = self.game == self.games.len().min(HOME_LIMIT);
        self.games = games;
        let end = self.games.len().min(HOME_LIMIT);
        self.game = if was_library_tile {
            end
        } else {
            self.game.min(end)
        };
        if self.library.open && self.page == Page::Games {
            self.select_library(self.library.selected);
        }
        self.library.revision = self.library.revision.wrapping_add(1);
        self.scene.invalidate_games();
        self.changed();
    }

    pub(super) fn library_entries(&self) -> &[Entry] {
        match self.page {
            Page::Games => &self.library.games,
            Page::Apps => &self.library.apps,
            Page::Settings => &[],
        }
    }

    fn select_library(&mut self, index: usize) {
        let grid = Grid::new(
            Layout {
                height: self.view_height,
            },
            self.page,
        );
        self.library.selected = index;
        if index < COLUMNS {
            self.library.offset = 0.0;
        } else {
            let Rect(_, top, _, size) = grid.cell(index, 0.0);
            let pad = self.view_height * 0.03;
            let min = (top + size + pad - grid.bottom).max(0.0);
            let max = (top - pad - grid.top).max(min);
            self.library.offset = self
                .library
                .offset
                .clamp(min, max)
                .min(grid.max_offset(self.library_entries().len()));
        }
        self.notice = None;
        self.changed();
    }

    pub(crate) fn scroll_library(&mut self, wheel_steps: f64) -> bool {
        if !self.library.open
            || !self.visible
            || self.overlay != Overlay::None
            || self.launch.is_some()
        {
            return false;
        }
        let grid = Grid::new(
            Layout {
                height: self.view_height,
            },
            self.page,
        );
        let step = (grid.bottom - grid.top).powf(2.0 / 3.0);
        let offset = (self.library.offset - wheel_steps * step)
            .clamp(0.0, grid.max_offset(self.library_entries().len()));
        if self.library.offset != offset {
            self.library.offset = offset;
            self.library.entered = None;
            self.changed();
        }
        true
    }

    pub(super) fn library_animating(&self, now: Instant) -> bool {
        if !self.library.open || !matches!(self.overlay, Overlay::None | Overlay::Control) {
            return false;
        }
        let grid = Grid::new(
            Layout {
                height: self.view_height,
            },
            self.page,
        );
        grid.visible(self.library_entries().len(), self.library.offset)
            .next_back()
            .is_some_and(|index| self.library.opacity(now, index, self.page == Page::Games) < 1.0)
    }

    pub(super) fn press_library(&mut self, sym: Keysym) -> HomeAction {
        let selected = self.library.selected;
        if let Some(item) = self.header_focus {
            let action = self.press_header(item, sym);
            if self.library.open
                && self.header_focus.is_none()
                && matches!(sym, Keysym::Down | Keysym::Escape)
            {
                self.select_library(selected);
            }
            return action;
        }
        let step = match sym {
            Keysym::Menu => {
                self.action_menu();
                return HomeAction::None;
            }
            Keysym::Escape => {
                self.open_page(self.page);
                return HomeAction::None;
            }
            Keysym::Up if selected < COLUMNS => {
                self.enter_header();
                return HomeAction::None;
            }
            Keysym::Left => -1,
            Keysym::Right => 1,
            Keysym::Up => -(COLUMNS as isize),
            Keysym::Down => COLUMNS as isize,
            Keysym::Return | Keysym::KP_Enter | Keysym::space => return self.activate_library(),
            _ => return HomeAction::None,
        };
        let index = bounded(selected, step, self.library_entries().len());
        if index != selected {
            self.library.entered = None;
            self.select_library(index);
        }
        HomeAction::None
    }

    pub(super) fn click_library(&mut self, layout: Layout, x: f64, y: f64) -> HomeAction {
        let grid = Grid::new(layout, self.page);
        if y < grid.top {
            return HomeAction::None;
        }
        let offset = self.library.offset;
        if let Some(index) = grid
            .visible(self.library_entries().len(), offset)
            .find(|index| grid.cell(*index, offset).contains(x, y))
        {
            self.select_library(index);
            return self.activate_library();
        }
        HomeAction::None
    }

    pub(super) fn activate_library(&mut self) -> HomeAction {
        let Some(entry) = self.library_entries().get(self.library.selected).cloned() else {
            return HomeAction::None;
        };
        if self.page == Page::Games {
            if let Some(index) = self.games.iter().position(|game| game.id == entry.id) {
                super::move_to_front(&mut self.games, index);
            } else {
                self.games.insert(0, entry.clone());
            }
            self.save_recent_order("game_order", &self.games);
            self.favorites.prioritize(Page::Games, &mut self.games);
            self.game = self
                .games
                .iter()
                .take(HOME_LIMIT)
                .position(|game| game.id == entry.id)
                .unwrap_or(0);
            self.library.revision = self.library.revision.wrapping_add(1);
            self.scene.invalidate_games();
            HomeAction::LaunchGame {
                id: entry.id,
                title: entry.label,
                art: entry.hero,
            }
        } else {
            HomeAction::LaunchApp(entry.id)
        }
    }

    pub(crate) fn promote_app(&mut self, id: &str) {
        if let Some(index) = self.apps.iter().position(|entry| entry.id == id) {
            if super::move_to_front(&mut self.apps, index) {
                self.save_recent_order("app_order", &self.apps);
                self.favorites.prioritize(Page::Apps, &mut self.apps);
                self.app = self
                    .apps
                    .iter()
                    .take(HOME_LIMIT)
                    .position(|entry| entry.id == id)
                    .unwrap_or(0);
                self.scene.invalidate_apps();
                self.changed();
            }
        }
        if let Some(index) = self.library.apps.iter().position(|entry| entry.id == id) {
            if super::move_to_front(&mut self.library.apps, index) {
                if self.library.open && self.page == Page::Apps {
                    self.select_library(0);
                }
                self.library.revision = self.library.revision.wrapping_add(1);
                self.changed();
            }
        }
    }

    fn save_recent_order(&self, file: &str, entries: &[Entry]) {
        let mut order = String::new();
        for entry in entries.iter().take(HOME_LIMIT) {
            order.push_str(&entry.id);
            order.push('\n');
        }
        if let Err(error) = fs::write(self.root.join(file), order) {
            tracing::warn!(%error, file, "Could not save launcher order");
        }
    }

    pub(super) fn refresh_libraries(&mut self) {
        self.update_game_library(steam_games(&self.root));
        let mut apps = super::apps();
        let previous = self.library.apps.iter().map(|entry| entry.id.as_str());
        super::order_entries(&mut apps, previous);
        self.library.apps = apps;
        self.apps = ordered(&self.root, "app_order", self.library.apps.clone());
        self.favorites.prioritize(Page::Apps, &mut self.apps);
        if self.library.open {
            self.select_library(
                self.library
                    .selected
                    .min(self.library_entries().len().saturating_sub(1)),
            );
        }
    }

    pub(super) fn paint_library_heading(&self, cr: &Context, grid: Grid) {
        let count = self.library_entries().len();
        let (heading, baseline) = match (self.page, count) {
            (Page::Games, 0) => ("No games found".into(), grid.top + grid.bottom * 0.17),
            (Page::Apps, 0) => (
                "No applications found".into(),
                grid.top + grid.bottom * 0.17,
            ),
            (Page::Games, _) => (
                format!("All Games ({count})"),
                grid.first - grid.gap - 13.0 * grid.font_scale,
            ),
            _ => return,
        };
        label(
            cr,
            &heading,
            (DESIGN_WIDTH / 2.0, baseline - self.library.offset),
            48.0 * grid.font_scale,
            400,
            (1.0, 1.0, 1.0),
            Align::Center,
        );
    }

    pub(super) fn paint_library_cell(
        &self,
        cr: &Context,
        grid: Grid,
        Rect(left, top, side, _): Rect,
        entry: &Entry,
        art: Option<&cairo::ImageSurface>,
        focused: bool,
    ) -> Result<(), cairo::Error> {
        let scale = grid.font_scale;
        let center = left + side / 2.0;
        if grid.games {
            let ring_offset = DESIGN_WIDTH * 0.003;
            let pad = ring_offset + 9.0 * scale;
            let tile = side - 2.0 * pad;
            let x = left + pad;
            let y = top + pad;
            cr.save()?;
            rounded(cr, x, y, tile, tile, 42.0 * scale);
            cr.clip();
            cr.set_source_rgb(0.11, 0.13, 0.16);
            cr.paint()?;
            if let Some(art) = art {
                image(cr, art, (x, y, tile, tile), true)?;
            } else {
                let name: String = entry.label.chars().take(12).collect();
                label(
                    cr,
                    &name,
                    (center, y + tile / 2.0 + 12.0 * scale),
                    42.0 * scale,
                    700,
                    (1.0, 1.0, 1.0),
                    Align::Center,
                );
            }
            cr.restore()?;
            if focused {
                self.focus_ring(
                    cr,
                    Rect(
                        x - ring_offset,
                        y - ring_offset,
                        tile + 2.0 * ring_offset,
                        tile + 2.0 * ring_offset,
                    ),
                    42.0 * scale,
                    DESIGN_WIDTH * 0.002 * 1.12,
                    1.0,
                )?;
            }
        } else {
            let radius = 66.0 * scale;
            let gradient = LinearGradient::new(left, top, left + side * 0.65, top + side);
            let color = if focused {
                ((57, 76, 108), (22, 31, 49), 0.98)
            } else {
                ((40, 50, 68), (13, 19, 30), 0.96)
            };
            let add_stop = |offset, (r, g, b)| {
                gradient.add_color_stop_rgba(
                    offset,
                    f64::from(r) / 255.0,
                    f64::from(g) / 255.0,
                    f64::from(b) / 255.0,
                    color.2,
                );
            };
            add_stop(0.0, color.0);
            add_stop(1.0, color.1);
            rounded(cr, left + 0.5, top + 0.5, side - 1.0, side - 1.0, radius);
            cr.set_source(&gradient)?;
            cr.fill_preserve()?;
            cr.set_line_width(1.0);
            if focused {
                cr.set_source_rgba(0.725, 0.812, 1.0, 0.72);
            } else {
                cr.set_source_rgba(0.725, 0.78, 0.878, 0.18);
            }
            cr.stroke()?;
            let icon = (276.0 * scale).min(DESIGN_WIDTH * 0.052);
            let spacing = 30.0 * scale;
            let name_top = wrapped_label(
                cr,
                &entry.label,
                (center, top + side / 2.0 + (icon + spacing) / 2.0),
                side - 60.0 * scale,
                42.0 * scale,
            );
            if let Some(art) = art {
                image(
                    cr,
                    art,
                    (center - icon / 2.0, name_top - spacing - icon, icon, icon),
                    false,
                )?;
            }
            if focused {
                let inset = (1.5 * scale).max(1.0);
                self.focus_ring(
                    cr,
                    Rect(
                        left + inset,
                        top + inset,
                        side - 2.0 * inset,
                        side - 2.0 * inset,
                    ),
                    27.0 * scale,
                    (3.0 * scale).max(2.0),
                    1.0,
                )?;
            }
        }
        Ok(())
    }

    pub(super) fn paint_library_page(
        &mut self,
        cr: &Context,
        layout: Layout,
        time: &str,
    ) -> Result<(), cairo::Error> {
        let (art, shade) = self.library_background();
        self.paint_background_art(cr, layout, art.as_ref(), shade)?;
        self.paint_header(cr, layout, time)?;
        let grid = Grid::new(layout, self.page);
        cr.save()?;
        cr.rectangle(0.0, grid.top, DESIGN_WIDTH, layout.height - grid.top);
        cr.clip();
        self.paint_library_heading(cr, grid);
        let offset = self.library.offset;
        for index in grid.visible(self.library_entries().len(), offset) {
            let entry = self.library_entries()[index].clone();
            let art = entry.image.as_deref().and_then(|path| self.art(path));
            let focused = index == self.library.selected && self.header_focus.is_none();
            self.paint_library_cell(
                cr,
                grid,
                grid.cell(index, offset),
                &entry,
                art.as_ref(),
                focused,
            )?;
        }
        cr.restore()
    }

    pub(super) fn library_background(&mut self) -> (Option<cairo::ImageSurface>, bool) {
        (self.art(&self.root.join("icons/ps-background.png")), false)
    }
}
