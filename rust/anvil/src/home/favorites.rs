//! Per-page pinned entries. The libraries retain their own sorting.
use std::{collections::HashSet, fs, path::Path};

use super::{ordered, Entry, Home, Overlay, Page, HOME_LIMIT};

#[derive(Debug, Clone, Default)]
pub(super) struct Favorites {
    games: Vec<String>,
    apps: Vec<String>,
}

fn filename(page: Page) -> &'static str {
    match page {
        Page::Games => "pinned_games",
        Page::Apps => "pinned_apps",
        Page::Settings => unreachable!("Settings has no pinned entries"),
    }
}

impl Favorites {
    pub(super) fn read(root: &Path) -> Self {
        let read = |page| {
            let mut known = HashSet::new();
            fs::read_to_string(root.join(filename(page)))
                .unwrap_or_default()
                .lines()
                .filter_map(|line| {
                    (!line.is_empty() && known.insert(line.to_owned())).then(|| line.to_owned())
                })
                .take(HOME_LIMIT)
                .collect()
        };
        Self {
            games: read(Page::Games),
            apps: read(Page::Apps),
        }
    }

    fn ids(&self, page: Page) -> &[String] {
        match page {
            Page::Games => &self.games,
            Page::Apps => &self.apps,
            Page::Settings => &[],
        }
    }

    pub(super) fn prioritize(&self, page: Page, entries: &mut [Entry]) {
        super::order_entries(entries, self.ids(page).iter().map(String::as_str));
    }

    pub(super) fn contains(&self, page: Page, id: &str) -> bool {
        self.ids(page).iter().any(|pinned| pinned == id)
    }

    fn toggle(&mut self, page: Page, id: &str) -> bool {
        let ids = match page {
            Page::Games => &mut self.games,
            Page::Apps => &mut self.apps,
            Page::Settings => return false,
        };
        if let Some(index) = ids.iter().position(|pinned| pinned == id) {
            ids.remove(index);
        } else {
            if ids.len() >= HOME_LIMIT {
                return false;
            }
            ids.push(id.to_owned());
        }
        true
    }
}

impl Home {
    pub(super) fn favorite_entry(&self) -> Option<&Entry> {
        if self.library.open {
            self.library_entries().get(self.library.selected)
        } else {
            self.entries().get(self.selected())
        }
    }

    pub(super) fn toggle_favorite(&mut self) {
        let Some(id) = self.favorite_entry().map(|entry| entry.id.clone()) else {
            return;
        };
        let page = self.page;
        let previous = self.favorites.clone();
        if !self.favorites.toggle(page, &id) {
            self.notice = Some("Unpin a favorite to make room.".into());
            self.changed();
            return;
        }
        let path = self.root.join(filename(page));
        let temporary = path.with_extension("new");
        let result = fs::write(&temporary, self.favorites.ids(page).join("\n"))
            .and_then(|()| fs::rename(&temporary, &path));
        if let Err(error) = result {
            let _ = fs::remove_file(&temporary);
            self.favorites = previous;
            self.notice = Some(format!("Could not save favorites: {error}"));
            self.changed();
            return;
        }
        self.overlay = Overlay::None;
        if page == Page::Games {
            self.games = ordered(&self.root, "game_order", std::mem::take(&mut self.games));
            self.favorites.prioritize(page, &mut self.games);
            self.scene.invalidate_games();
        } else {
            self.apps = ordered(&self.root, "app_order", std::mem::take(&mut self.apps));
            self.favorites.prioritize(page, &mut self.apps);
            self.scene.invalidate_apps();
        }
        if !self.library.open {
            let position = self.entries().iter().position(|entry| entry.id == id);
            self.set_selected(
                position
                    .unwrap_or_else(|| self.selected().min(self.entries().len().saturating_sub(1))),
            );
        }
        self.carousel_motion = None;
        self.changed();
    }
}
