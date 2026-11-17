//! On-screen search entry for a controller or pointer.
use cairo::Context;
use smithay::input::keyboard::Keysym;

use super::{
    bounded,
    native::{label, Align},
    visual::Rect,
    Home, HomeAction,
};

const ROWS: [&[&str]; 5] = [
    &["1", "2", "3", "4", "5", "6", "7", "8", "9", "0"],
    &["q", "w", "e", "r", "t", "y", "u", "i", "o", "p"],
    &["a", "s", "d", "f", "g", "h", "j", "k", "l"],
    &["z", "x", "c", "v", "b", "n", "m", "-", "."],
    &["Space", "Delete", "Results"],
];

#[derive(Clone, Copy, Debug)]
pub(super) struct SearchKeyboard {
    row: usize,
    column: usize,
}

impl Default for SearchKeyboard {
    fn default() -> Self {
        Self { row: 1, column: 0 }
    }
}

fn key_rect(row: usize, column: usize) -> Rect {
    let width = if row == ROWS.len() - 1 { 139.0 } else { 40.0 };
    let used = ROWS[row].len() as f64 * (width + 6.0) - 6.0;
    let left = 956.0 - used / 2.0;
    Rect(
        left + column as f64 * (width + 6.0),
        169.0 + row as f64 * 57.0,
        width,
        50.0,
    )
}

impl Home {
    pub(super) fn press_search_keyboard(&mut self, sym: Keysym) -> Option<HomeAction> {
        let mut cursor = self.search_keyboard?;
        match sym {
            Keysym::Return | Keysym::KP_Enter => {
                self.choose_search_key(cursor);
                return Some(HomeAction::None);
            }
            Keysym::Escape | Keysym::Menu => {
                self.search_keyboard = None;
                self.changed();
                return Some(HomeAction::None);
            }
            Keysym::Up | Keysym::Down => {
                let delta = if sym == Keysym::Up { -1 } else { 1 };
                let Rect(x, _, w, _) = key_rect(cursor.row, cursor.column);
                cursor.row = bounded(cursor.row, delta, ROWS.len());
                cursor.column = (0..ROWS[cursor.row].len())
                    .min_by(|&a, &b| {
                        let distance = |col| {
                            let Rect(left, _, width, _) = key_rect(cursor.row, col);
                            (x + w / 2.0 - left - width / 2.0).abs()
                        };
                        distance(a).total_cmp(&distance(b))
                    })
                    .unwrap_or(0);
            }
            Keysym::Left | Keysym::Right => {
                let delta = if sym == Keysym::Left { -1 } else { 1 };
                cursor.column = bounded(cursor.column, delta, ROWS[cursor.row].len());
            }
            _ => return None,
        }
        self.search_keyboard = Some(cursor);
        self.changed();
        Some(HomeAction::None)
    }

    fn choose_search_key(&mut self, cursor: SearchKeyboard) {
        match ROWS[cursor.row][cursor.column] {
            "Results" => self.search_keyboard = None,
            "Delete" => {
                self.search.pop();
            }
            "Space" => {
                if self.search.chars().count() < 48 {
                    self.search.push(' ');
                }
            }
            key => {
                if self.search.chars().count() < 48 {
                    self.search.push_str(key);
                }
            }
        }
        self.search_selection = 0;
        self.changed();
    }

    pub(super) fn click_search_keyboard(&mut self, x: f64, y: f64) -> bool {
        if self.search_keyboard.is_none() {
            if Rect(1087.0, 95.0, 80.0, 33.0).contains(x, y) {
                self.search_keyboard = Some(SearchKeyboard::default());
                self.changed();
                return true;
            }
            return false;
        }
        for (row, keys) in ROWS.iter().enumerate() {
            if let Some(column) = (0..keys.len()).find(|&col| key_rect(row, col).contains(x, y)) {
                let cursor = SearchKeyboard { row, column };
                self.search_keyboard = Some(cursor);
                self.choose_search_key(cursor);
                return true;
            }
        }
        if x < 723.0 && y > 130.0 {
            self.search_keyboard = None;
            self.changed();
            return false;
        }
        true
    }

    pub(super) fn paint_search_keyboard(&self, cr: &Context) -> Result<(), cairo::Error> {
        let Some(cursor) = self.search_keyboard else {
            super::pill(cr, 1087.0, 95.0, 80.0, 33.0, (0.15, 0.20, 0.28, 1.0))?;
            label(
                cr,
                "Keyboard",
                (1127.0, 116.0),
                12.0,
                600,
                (1.0, 1.0, 1.0),
                Align::Center,
            );
            return Ok(());
        };
        for (row, keys) in ROWS.iter().enumerate() {
            for (column, key) in keys.iter().enumerate() {
                let Rect(x, y, w, h) = key_rect(row, column);
                let focused = row == cursor.row && column == cursor.column;
                let background = if focused {
                    (0.93, 0.95, 1.0, 1.0)
                } else {
                    (0.15, 0.20, 0.28, 1.0)
                };
                let text = if focused {
                    (0.07, 0.09, 0.13)
                } else {
                    (0.95, 0.96, 1.0)
                };
                super::rounded(cr, x, y, w, h, 8.0);
                cr.set_source_rgba(background.0, background.1, background.2, background.3);
                cr.fill()?;
                label(
                    cr,
                    key,
                    (x + w / 2.0, y + 31.0),
                    17.0,
                    500,
                    text,
                    Align::Center,
                );
            }
        }
        Ok(())
    }
}
