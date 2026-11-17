//! Console DE's compositor-owned launcher and desktop rendering.
mod audio;
mod control;
mod favorites;
mod intro;
mod library;
mod launch;
mod native;
mod scene;
mod search_keyboard;
mod services;
mod status;
mod visual;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    time::{Duration, Instant},
};

use cairo::{Context, FontSlant, FontWeight, Format, ImageSurface};
use chrono::Local;
use gio::prelude::*;
use smithay::{
    backend::renderer::{ImportAll, ImportMem, Renderer},
    input::keyboard::Keysym,
    output::Output,
    reexports::calloop::channel::{channel, Event},
};

use crate::render::CustomRenderElements;
use crate::{state::Backend, AnvilState};

use audio::{Audio, Cue};
use control::CONTROL;
use intro::{Intro, Phase};
use native::{load_art, IconTheme};
use scene::Scene;
use status::Status;
use visual::{CarouselMotion, Layout, Particle, DESIGN_WIDTH, WELCOME_DURATION};

const WIDTH: f64 = 1280.0;
const HEIGHT: f64 = 720.0;
const HOME_LIMIT: usize = 7;
const GRID_CAPACITY: usize = 24;
const SCALES: [f64; 6] = [1.0, 1.25, 1.33, 1.5, 1.66, 2.0];
const MENU_SIZES: [f64; 5] = [0.7, 1.0, 1.3, 1.6, 2.0];
const MENU_WEIGHTS: [i32; 3] = [400, 600, 800];
const GPUS: [&str; 3] = ["nvidia", "amd", "off"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Games,
    Apps,
    Settings,
}
const PAGES: [Page; 3] = [Page::Games, Page::Apps, Page::Settings];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HeaderItem {
    Games,
    Apps,
    Search,
    Settings,
    Profile,
}

const HEADER: [HeaderItem; 5] = [
    HeaderItem::Games,
    HeaderItem::Apps,
    HeaderItem::Search,
    HeaderItem::Settings,
    HeaderItem::Profile,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Overlay {
    None,
    Control,
    Search,
    More,
}
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    id: String,
    label: String,
    image: Option<PathBuf>,
    hero: Option<PathBuf>,
    logo: Option<PathBuf>,
}

#[derive(Debug)]
pub enum HomeAction {
    None,
    Command {
        program: String,
        arguments: Vec<String>,
        hide_home: bool,
    },
    LaunchApp(String),
    LaunchGame {
        id: String,
        title: String,
        art: Option<PathBuf>,
    },
    CancelGame,
    ToggleControl,
    OpenCreate,
    Capture,
    ResumeControl,
    ShowHome,
    RefreshSwitcher,
    ResumeWindow(usize),
    CloseWindow(usize),
    Resume,
    Switch,
    Scale(f64),
    Quit,
}

impl HomeAction {
    fn run(program: &str, args: &[&str]) -> Self {
        Self::Command {
            program: program.into(),
            arguments: args.iter().map(|arg| (*arg).into()).collect(),
            hide_home: false,
        }
    }
}

#[derive(Debug)]
pub struct Home {
    pub root: PathBuf,
    pub visible: bool,
    pub scale: f64,
    page: Page,
    overlay: Overlay,
    game: usize,
    app: usize,
    setting: usize,
    category: usize,
    settings_options: bool,
    scale_menu: Option<usize>,
    return_page: Page,
    menu_size: f64,
    menu_weight: i32,
    reduced_motion: bool,
    high_contrast: bool,
    swap_confirm: bool,
    do_not_disturb: bool,
    notifications: VecDeque<String>,
    last_capture: Option<PathBuf>,
    gpu: usize,
    action_focus: usize,
    header_focus: Option<HeaderItem>,
    carousel_motion: Option<CarouselMotion>,
    last_frame_time: Option<Instant>,
    search: String,
    search_selection: usize,
    search_keyboard: Option<search_keyboard::SearchKeyboard>,
    library: library::Library,
    game_catalog_requests: Option<Sender<u64>>,
    pub(crate) app_launch_generation: u64,
    more: usize,
    profile: PathBuf,
    intro: Intro,
    pub(crate) launch: Option<launch::Launch>,
    audio: Option<Audio>,
    audio_active: bool,
    music_playing: bool,
    status: Status,
    particles: Vec<Particle>,
    view_height: f64,
    resolution: (i32, i32),
    control: usize,
    control_power: Option<usize>,
    home_before_control: bool,
    overlay_before_control: Overlay,
    control_opened: Option<Instant>,
    control_panel: Option<control::Panel>,
    control_row: usize,
    control_windows: Vec<String>,
    control_close_window: bool,
    control_card_focus: Option<usize>,
    control_parent: Option<control::Panel>,
    control_message: Option<String>,
    services: services::Snapshot,
    service_requests: Option<Sender<services::Request>>,
    pub(crate) running_games: HashSet<String>,
    favorites: favorites::Favorites,
    games: Vec<Entry>,
    apps: Vec<Entry>,
    version: u64,
    art: HashMap<PathBuf, Option<ImageSurface>>,
    queued_art: HashSet<PathBuf>,
    art_requests: Option<Sender<PathBuf>>,
    art_revision: u64,
    scene: Scene,
    notice: Option<String>,
}

impl Home {
    pub fn new() -> Self {
        let root = std::env::var_os("CONSOLE_DE_WAYLAND_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let scale = fs::read_to_string(root.join("scale"))
            .ok()
            .and_then(|value| value.trim().parse::<f64>().ok())
            .filter(|value| (1.0..=2.0).contains(value))
            .unwrap_or(1.0);
        let menu_size = read_choice(&root, "menu_font_size", &MENU_SIZES, 1.3);
        let menu_weight = read_choice(&root, "menu_font_weight", &MENU_WEIGHTS, 400);
        let gpu = fs::read_to_string(root.join("gpu"))
            .ok()
            .and_then(|text| GPUS.iter().position(|value| *value == text.trim()))
            .unwrap_or(0);
        let read_toggle = |name| fs::read_to_string(root.join(name)).is_ok_and(|value| value.trim() == "on");
        let reduced_motion = read_toggle("reduced_motion");
        let high_contrast = read_toggle("high_contrast");
        let swap_confirm = read_toggle("swap_confirm");
        let do_not_disturb = read_toggle("do_not_disturb");
        let mut intro = Intro::default();
        if reduced_motion {
            intro.done = true;
            intro.notices = true;
        }
        let profile = visual::profile(&root);
        let favorites = favorites::Favorites::read(&root);
        let mut games = steam_games(&root);
        let mut all_games = games.clone();
        sort_catalog(&mut all_games);
        let all_apps = apps();
        let mut apps = ordered(&root, "app_order", all_apps.clone());
        favorites.prioritize(Page::Games, &mut games);
        favorites.prioritize(Page::Apps, &mut apps);
        Self {
            root,
            visible: true,
            scale,
            page: Page::Games,
            overlay: Overlay::None,
            game: 0,
            app: 0,
            setting: 0,
            category: 0,
            settings_options: false,
            scale_menu: None,
            return_page: Page::Games,
            menu_size,
            menu_weight,
            reduced_motion,
            high_contrast,
            swap_confirm,
            do_not_disturb,
            notifications: VecDeque::new(),
            last_capture: None,
            gpu,
            action_focus: 0,
            header_focus: None,
            carousel_motion: None,
            last_frame_time: None,
            search: String::new(),
            search_selection: 0,
            search_keyboard: None,
            library: library::Library { games: all_games, apps: all_apps, ..Default::default() },
            game_catalog_requests: None,
            app_launch_generation: 0,
            more: 0,
            profile,
            intro,
            launch: None,
            audio: None,
            audio_active: false,
            music_playing: false,
            status: Status::default(),
            particles: visual::particles(),
            view_height: visual::DESIGN_HEIGHT,
            resolution: (2880, 1800),
            control: 0,
            control_power: None,
            home_before_control: false,
            overlay_before_control: Overlay::None,
            control_opened: None,
            control_panel: None,
            control_row: 0,
            control_windows: Vec::new(),
            control_close_window: false,
            control_card_focus: None,
            control_parent: None,
            control_message: None,
            services: services::Snapshot::default(),
            service_requests: None,
            running_games: HashSet::new(),
            favorites,
            games,
            apps,
            version: 0,
            art: HashMap::new(),
            queued_art: HashSet::new(),
            art_requests: None,
            art_revision: 0,
            scene: Scene::default(),
            notice: None,
        }
    }

    pub fn changed(&mut self) {
        self.version = self.version.wrapping_add(1);
    }

    pub fn uses_keyboard(&self) -> bool {
        self.visible || self.overlay == Overlay::Control
    }

    pub fn show(&mut self) {
        self.reset_control();
        self.launch = None;
        self.visible = true;
        self.sync_music();
        self.overlay = Overlay::None;
        self.action_focus = 0;
        self.header_focus = None;
        self.scale_menu = None;
        self.notice = None;
        self.changed();
    }

    pub fn hide(&mut self) {
        self.reset_control();
        self.launch = None;
        self.visible = false;
        self.sync_music();
        self.overlay = Overlay::None;
        self.carousel_motion = None;
        self.changed();
    }

    fn reset_control(&mut self) {
        if self.control_open() {
            self.watch_services(false);
        }
        self.control_opened = None;
        self.control_panel = None;
        self.control_power = None;
        self.control_card_focus = None;
        self.control_parent = None;
        self.control_message = None;
    }

    pub fn toggle_control(&mut self) -> HomeAction {
        if self.control_open() {
            self.reset_control();
            self.visible = self.home_before_control;
            self.sync_music();
            self.overlay = self.overlay_before_control;
            self.cue(Cue::Control);
            self.changed();
            return if self.visible { HomeAction::None } else { HomeAction::ResumeControl };
        }
        self.home_before_control = self.visible;
        self.overlay_before_control = self.overlay;
        self.overlay = Overlay::Control;
        self.scale_menu = None;
        self.control = 0;
        self.control_opened = Some(Instant::now());
        self.watch_services(true);
        self.cue(Cue::Control);
        self.changed();
        HomeAction::None
    }

    pub fn show_error(&mut self, error: String) {
        self.record_notification(&error);
        self.notice = Some(error);
        self.visible = true;
        self.sync_music();
        self.changed();
    }

    pub fn refresh(&mut self) {
        self.refresh_libraries();
        self.game = self.game.min(self.games.len().min(HOME_LIMIT));
        self.app = self.app.min(self.apps.len().min(HOME_LIMIT));
        self.search_selection = self
            .search_selection
            .min(self.search_indices().len().saturating_sub(1));
        self.art.clear();
        self.scene = Scene::default();
        self.art_revision = self.art_revision.wrapping_add(1);
        self.carousel_motion = None;
        self.changed();
    }

    fn entries(&self) -> &[Entry] {
        let entries = match self.page {
            Page::Games => &self.games[..],
            Page::Apps => &self.apps[..],
            Page::Settings => &[],
        };
        if self.overlay == Overlay::Search {
            entries
        } else {
            &entries[..entries.len().min(HOME_LIMIT)]
        }
    }

    fn search_indices(&self) -> Vec<usize> {
        let filter = self.search.trim().to_lowercase();
        self.entries()
            .iter()
            .enumerate()
            .filter(|(_, entry)| {
                self.overlay != Overlay::Search || entry.label.to_lowercase().contains(&filter)
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn open_page(&mut self, page: Page) {
        let from_library = std::mem::take(&mut self.library.open);
        self.overlay = Overlay::None;
        self.scale_menu = None;
        self.action_focus = 0;
        self.header_focus = None;
        self.carousel_motion = None;
        if self.page != page {
            if page == Page::Settings {
                self.return_page = self.page;
                self.category = 0;
                self.settings_options = false;
            }
            self.page = page;
            self.set_selected(0);
        } else if from_library {
            self.set_selected(0);
        }
        self.changed();
    }

    fn cycle_page(&mut self, delta: isize) {
        if self.library.open {
            self.open_page(Page::Games);
            return;
        }
        let index = PAGES.iter().position(|page| *page == self.page).unwrap_or(0);
        self.open_page(PAGES[wrap(index, delta, PAGES.len())]);
    }

    fn enter_header(&mut self) {
        self.header_focus = Some(match self.page {
            Page::Games => HeaderItem::Games,
            Page::Apps => HeaderItem::Apps,
            Page::Settings => HeaderItem::Settings,
        });
        self.changed();
    }

    fn activate_header(&mut self, item: HeaderItem) -> HomeAction {
        self.header_focus = None;
        self.changed();
        match item {
            HeaderItem::Games => self.open_page(Page::Games),
            HeaderItem::Apps => self.open_page(Page::Apps),
            HeaderItem::Settings => self.open_page(Page::Settings),
            HeaderItem::Search => self.open_search(),
            HeaderItem::Profile => return HomeAction::ToggleControl,
        }
        HomeAction::None
    }

    fn header_page(&self) -> Page {
        if self.library.open { Page::Apps } else { self.page }
    }

    fn press_header(&mut self, item: HeaderItem, sym: Keysym) -> HomeAction {
        match sym {
            Keysym::Left | Keysym::Right => {
                let index = HEADER
                    .iter()
                    .position(|candidate| *candidate == item)
                    .unwrap_or(0);
                let delta = if sym == Keysym::Left { -1 } else { 1 };
                self.header_focus = Some(HEADER[bounded(index, delta, HEADER.len())]);
            }
            Keysym::Down | Keysym::Escape => self.header_focus = None,
            Keysym::Return | Keysym::KP_Enter | Keysym::space => return self.activate_header(item),
            _ => return HomeAction::None,
        }
        self.changed();
        HomeAction::None
    }

    fn open_search(&mut self) {
        if self.page == Page::Settings {
            self.page = self.return_page;
        }
        self.overlay = Overlay::Search;
        self.search.clear();
        self.search_keyboard = None;
        self.header_focus = None;
        self.carousel_motion = None;
        self.set_selected(0);
        self.changed();
    }

    fn close_overlay(&mut self) {
        self.overlay = Overlay::None;
        self.action_focus = 0;
        self.set_selected(self.selected().min(self.entries().len()));
        self.changed();
    }

    fn action_menu(&mut self) {
        if self.favorite_entry().is_some() {
            self.more = 0;
            self.overlay = Overlay::More;
            self.changed();
        }
    }

    pub(crate) fn capture_complete(&mut self, result: Result<Option<PathBuf>, String>) {
        match result {
            Ok(Some(path)) => {
                self.last_capture = Some(path);
                self.changed();
                self.record_notification("Screenshot saved");
            }
            Ok(None) => (),
            Err(error) => self.record_notification(&format!("Could not save screenshot: {error}")),
        }
    }

    fn persist_setting(&mut self, file: &str, value: &str) {
        if let Err(error) = fs::write(self.root.join(file), value) {
            self.show_error(format!("Could not save {file}: {error}"));
        }
        self.changed();
    }

    fn persist_toggle(&mut self, file: &str, enable: bool) {
        self.persist_setting(file, if enable { "on" } else { "off" });
    }

    fn activate_setting(&mut self, direction: isize) -> HomeAction {
        match (self.category, self.setting) {
            (0, 0) => {
                let nearest = SCALES
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| (*a - self.scale).abs().total_cmp(&(*b - self.scale).abs()))
                    .map_or(0, |(index, _)| index);
                self.scale_menu = Some(nearest);
            }
            (0, 1) => {
                self.gpu = wrap(self.gpu, direction, GPUS.len());
                self.persist_setting("gpu", GPUS[self.gpu]);
            }
            (0, 2) => {
                self.menu_size = next_choice(MENU_SIZES, self.menu_size, direction);
                self.persist_setting("menu_font_size", &self.menu_size.to_string());
            }
            (0, 3) => {
                self.menu_weight = next_choice(MENU_WEIGHTS, self.menu_weight, direction);
                self.persist_setting("menu_font_weight", &self.menu_weight.to_string());
            }
            (1, 0) => return HomeAction::Quit,
            (1, 1) => return HomeAction::run("systemctl", &["poweroff"]),
            (1, 2) => {
                return HomeAction::Command {
                    program: "foot".into(),
                    arguments: vec![],
                    hide_home: true,
                }
            }
            (1, 3) => return HomeAction::run("systemctl", &["suspend"]),
            (4, 0) => {
                self.reduced_motion = !self.reduced_motion;
                self.carousel_motion = None;
                self.library.entered = None;
                self.persist_toggle("reduced_motion", self.reduced_motion);
            }
            (4, 1) => {
                self.high_contrast = !self.high_contrast;
                self.library.revision = self.library.revision.wrapping_add(1);
                self.persist_toggle("high_contrast", self.high_contrast);
            }
            (4, 2) => {
                self.swap_confirm = !self.swap_confirm;
                self.persist_toggle("swap_confirm", self.swap_confirm);
            }
            _ => (),
        }
        self.changed();
        HomeAction::None
    }

    fn choose_scale(&mut self, index: usize) -> HomeAction {
        self.scale_menu = None;
        let nearest = SCALES[index];
        if (self.scale - nearest).abs() > 0.011 {
            self.scale = nearest;
        }
        self.changed();
        HomeAction::Scale(self.scale)
    }

    fn setting_value(&self, index: usize) -> String {
        match (self.category, index) {
            (0, 0) => format!("{:.0} %", self.scale * 100.0),
            (0, 1) => GPUS[self.gpu].to_uppercase(),
            (0, 2) => format!("{:.1}x", self.menu_size),
            (0, 3) => self.menu_weight.to_string(),
            (2, 0) => "Wayland · Smithay 0.7".into(),
            (2, 1) => option_env!("CONSOLE_DE_BUILD").unwrap_or("Local build").into(),
            (2, 2) => format!("{}×{}", self.resolution.0, self.resolution.1),
            (3, 0) => fs::read_to_string("/proc/loadavg")
                .ok()
                .and_then(|data| data.split_whitespace().next().map(str::to_owned))
                .unwrap_or_default(),
            (3, 1) => system_memory().unwrap_or_default(),
            (3, 2) => fs::read_to_string("/proc/self/status")
                .ok()
                .and_then(|data| {
                    data.lines()
                        .find_map(|row| row.strip_prefix("VmRSS:").map(|s| s.trim().to_owned()))
                })
                .unwrap_or_default(),
            (3, 3) => std::process::id().to_string(),
            (4, 0) => if self.reduced_motion { "On" } else { "Off" }.into(),
            (4, 1) => if self.high_contrast { "On" } else { "Off" }.into(),
            (4, 2) => if self.swap_confirm { "East / Circle" } else { "South / Cross" }.into(),
            _ => String::new(),
        }
    }

    fn selected(&self) -> usize {
        if self.overlay == Overlay::Search {
            return self.search_selection;
        }
        match self.page {
            Page::Games => self.game,
            Page::Apps => self.app,
            Page::Settings => self.setting,
        }
    }

    fn set_selected(&mut self, index: usize) {
        if self.overlay == Overlay::Search {
            self.search_selection = index;
            return;
        }
        match self.page {
            Page::Games => self.game = index,
            Page::Apps => self.app = index,
            Page::Settings => self.setting = index,
        }
    }

    fn select_tile(&mut self, index: usize, now: Instant) {
        let selected = self.selected();
        if index == selected {
            return;
        }
        let sample = self.last_frame_time.map_or(now, |last| last.max(now));
        let previous = self.carousel_pose(sample);
        self.carousel_motion = (!self.reduced_motion).then(|| CarouselMotion::new(previous, index, self.page, sample));
        self.set_selected(index);
        self.notice = None;
        self.changed();
    }

    fn step(&mut self, amount: isize) {
        if self.overlay == Overlay::Search {
            let count = self.search_indices().len();
            self.set_selected(bounded(self.selected(), amount, count));
        } else {
            let next = bounded(self.selected(), amount, self.entries().len() + 1);
            self.select_tile(next, Instant::now());
            return;
        }
        self.notice = None;
        self.changed();
    }

    fn press_settings(&mut self, sym: Keysym) -> HomeAction {
        match sym {
            Keysym::Up
                if (self.settings_options && self.setting == 0)
                    || (!self.settings_options && self.category == 0) =>
            {
                self.enter_header()
            }
            Keysym::Up | Keysym::Down => {
                let direction = if sym == Keysym::Up { -1 } else { 1 };
                if self.settings_options {
                    self.setting = bounded(self.setting, direction, SETTINGS[self.category].1.len());
                } else {
                    self.category = bounded(self.category, direction, SETTINGS.len());
                    self.setting = 0;
                }
            }
            Keysym::Left if self.settings_options && (self.category == 4 || (self.category == 0 && self.setting > 0)) => {
                return self.activate_setting(-1)
            }
            Keysym::Left => self.settings_options = false,
            Keysym::Right if self.settings_options && matches!(self.category, 0 | 4) => return self.activate_setting(1),
            Keysym::Right => self.settings_options = true,
            Keysym::Return | Keysym::KP_Enter | Keysym::space if self.settings_options => {
                return self.activate_setting(1)
            }
            Keysym::Return | Keysym::KP_Enter | Keysym::space => self.settings_options = true,
            Keysym::Escape if self.settings_options => self.settings_options = false,
            Keysym::Escape => self.open_page(self.return_page),
            _ => (),
        }
        self.changed();
        HomeAction::None
    }

    fn cue(&self, cue: Cue) {
        if self.audio_active {
            if let Some(audio) = &self.audio {
                audio.cue(cue);
            }
        }
    }

    fn sync_music(&mut self) {
        let playing = self.visible && self.audio_active && self.launch.is_none();
        if self.music_playing != playing {
            self.music_playing = playing;
            if let Some(audio) = &self.audio {
                audio.music(playing);
            }
        }
    }

    pub(crate) fn set_audio_active(&mut self, active: bool) {
        self.audio_active = active;
        self.sync_music();
    }

    fn show_peripherals(&mut self, controller: bool, microphone: bool, now: Instant) {
        if self.do_not_disturb {
            return;
        }
        self.status.show(controller, microphone, now);
        self.cue(Cue::Peripherals);
    }

    pub(crate) fn update_controller(&mut self, count: usize, battery: Option<u8>) {
        let connected = count > 0;
        if self.status.controller == connected && self.status.battery == battery {
            return;
        }
        let just_connected = connected && !self.status.controller;
        self.status.controller = connected;
        self.status.battery = battery;
        self.status.revision = self.status.revision.wrapping_add(1);
        if just_connected && self.intro.notices {
            self.record_notification("Controller connected");
            self.show_peripherals(true, false, Instant::now());
        }
    }

    fn update_microphone(&mut self, enabled: bool) {
        let previous = self.status.mic.replace(enabled);
        if previous != Some(enabled) {
            self.status.revision = self.status.revision.wrapping_add(1);
            if previous.is_some() && self.intro.notices {
                self.record_notification(if enabled { "Microphone on" } else { "Microphone muted" });
                self.show_peripherals(false, true, Instant::now());
            }
        }
    }

    fn intro_at(&self, now: Instant) -> Option<Phase> {
        self.intro.phase(now)
    }

    fn advance_intro(&mut self, now: Instant) {
        if self.intro.done {
            return;
        }
        self.intro.started.get_or_insert(now);
        let elapsed = self.intro.elapsed(now).unwrap_or_default();
        if !self.intro.home_cue && elapsed >= intro::ENTRANCE_START {
            self.intro.home_cue = true;
            self.cue(Cue::Home);
        }
        if !self.intro.notices && elapsed >= intro::NOTICES_START {
            self.intro.notices = true;
            self.show_peripherals(self.status.controller, true, now);
        }
        self.intro.done = elapsed >= intro::END;
    }

    fn finish_intro(&mut self, now: Instant) {
        if self.intro.done {
            return;
        }
        if !self.intro.home_cue {
            self.intro.home_cue = true;
            self.cue(Cue::Home);
        }
        if !self.intro.notices {
            self.intro.notices = true;
            self.show_peripherals(self.status.controller, true, now);
        }
        self.intro.done = true;
        self.changed();
    }

    pub(crate) fn observe_render_time(&mut self, work: Duration, period: Duration) {
        if self.visible && self.overlay == Overlay::None {
            self.intro.observe_welcome_render(Instant::now(), work, period);
        }
    }

    pub(crate) fn animating(&self) -> bool {
        let now = Instant::now();
        self.launch_is_animating(now)
            || (self.control_open() && self.control_progress(now) < 1.0)
            || (self.visible
                && (!self.intro.done
                    || self.status.moving(now)
                    || self.library_animating(now)
                    || self
                        .carousel_motion
                        .as_ref()
                        .is_some_and(|motion| motion.active(now))))
    }

    fn input_ready(&mut self, skip: bool, now: Instant) -> bool {
        if self.intro.done {
            return true;
        }
        if self
            .intro
            .elapsed(now)
            .is_none_or(|elapsed| elapsed < WELCOME_DURATION)
        {
            if skip {
                self.finish_intro(now);
            }
            return false;
        }
        true
    }

    fn feedback(&self, previous: (u64, Overlay, Option<usize>), action: &HomeAction, default: Option<Cue>) {
        let (version, overlay, scale) = previous;
        if matches!(action, HomeAction::ToggleControl) {
            return;
        }
        let cue = match (overlay, self.overlay) {
            (Overlay::Control, Overlay::Control) => default,
            (Overlay::Control, _) | (_, Overlay::Control) => None,
            (Overlay::More, other) if other != Overlay::More => Some(Cue::OptionsClose),
            (other, Overlay::More) if other != Overlay::More => Some(Cue::OptionsOpen),
            _ if scale.is_none() && self.scale_menu.is_some() => Some(Cue::Dialog),
            _ => default,
        };
        if self.version != version || !matches!(action, HomeAction::None) {
            if let Some(cue) = cue {
                self.cue(cue);
            }
        }
    }

    pub(crate) fn press_gamepad(&mut self, sym: Keysym) -> HomeAction {
        let sym = if self.swap_confirm {
            match sym {
                Keysym::Return => Keysym::Escape,
                Keysym::Escape => Keysym::Return,
                _ => sym,
            }
        } else {
            sym
        };
        let was_search = self.overlay == Overlay::Search;
        let action = self.press(sym);
        if !was_search && self.overlay == Overlay::Search {
            self.search_keyboard = Some(search_keyboard::SearchKeyboard::default());
            self.changed();
        }
        action
    }

    pub fn press(&mut self, sym: Keysym) -> HomeAction {
        if self.launch.is_some() {
            if sym == Keysym::Escape {
                self.cancel_launch();
                self.cue(Cue::Cancel);
                return HomeAction::CancelGame;
            }
            return HomeAction::None;
        }
        let now = Instant::now();
        if !self.input_ready(sym == Keysym::Escape, now) {
            return HomeAction::None;
        }
        let previous = (self.version, self.overlay, self.scale_menu);
        let search = self.search.clone();
        let action = self.press_inner(sym);
        if previous.0 != self.version {
            self.finish_intro(now);
        }
        let cue = match sym {
            Keysym::Return | Keysym::KP_Enter | Keysym::space => Some(Cue::Select),
            Keysym::Escape => Some(Cue::Cancel),
            Keysym::Up
            | Keysym::Down
            | Keysym::Left
            | Keysym::Right
            | Keysym::Tab
            | Keysym::ISO_Left_Tab
            | Keysym::F2
            | Keysym::F3 => Some(Cue::Move),
            _ => None,
        };
        self.feedback(
            previous,
            &action,
            if search != self.search {
                Some(Cue::Typing)
            } else {
                cue
            },
        );
        action
    }

    fn press_inner(&mut self, sym: Keysym) -> HomeAction {
        if self.overlay == Overlay::Control {
            return self.press_control(sym);
        }
        if let Some(selected) = self.scale_menu {
            self.scale_menu = match sym {
                Keysym::Escape => None,
                Keysym::Up | Keysym::Left => Some(wrap(selected, -1, SCALES.len())),
                Keysym::Down | Keysym::Right => Some(wrap(selected, 1, SCALES.len())),
                Keysym::Return | Keysym::KP_Enter | Keysym::space => return self.choose_scale(selected),
                _ => Some(selected),
            };
            self.changed();
            return HomeAction::None;
        }
        if self.overlay == Overlay::More {
            match sym {
                Keysym::Escape => self.close_overlay(),
                Keysym::Up | Keysym::Down => {
                    let delta = if sym == Keysym::Up { -1 } else { 1 };
                    self.more = bounded(self.more, delta, self.more_labels().len());
                    self.changed();
                }
                Keysym::Return | Keysym::KP_Enter | Keysym::space => return self.activate_more(),
                _ => (),
            }
            return HomeAction::None;
        }
        if self.overlay == Overlay::Search {
            if let Some(action) = self.press_search_keyboard(sym) {
                return action;
            }
            if sym == Keysym::Menu {
                self.search_keyboard = Some(search_keyboard::SearchKeyboard::default());
                self.changed();
                return HomeAction::None;
            }
            let edit = match sym {
                Keysym::BackSpace => self.search.pop().is_some(),
                _ => sym.key_char().filter(|ch| !ch.is_control()).is_some_and(|ch| {
                    if self.search.chars().count() < 48 {
                        self.search.push(ch);
                    }
                    true
                }),
            };
            if edit {
                self.set_selected(0);
                self.changed();
                return HomeAction::None;
            }
        }
        if matches!(self.overlay, Overlay::None | Overlay::Search) {
            if let Some(delta) = match sym {
                Keysym::Tab => Some(1),
                Keysym::ISO_Left_Tab => Some(-1),
                _ => None,
            } {
                self.cycle_page(delta);
                return HomeAction::None;
            }
        }
        if self.overlay == Overlay::None {
            match sym {
                Keysym::F5 => {
                    self.refresh();
                    return HomeAction::None;
                }
                Keysym::F2 => {
                    self.open_page(Page::Settings);
                    return HomeAction::None;
                }
                Keysym::F1 => return HomeAction::ToggleControl,
                Keysym::F3 => {
                    self.open_search();
                    return HomeAction::None;
                }
                _ => (),
            }
            if self.library.open {
                return self.press_library(sym);
            }
            if let Some(item) = self.header_focus {
                return self.press_header(item, sym);
            }
            if self.page == Page::Settings {
                return self.press_settings(sym);
            }
        }
        match sym {
            Keysym::Escape => {
                if self.overlay != Overlay::None {
                    self.close_overlay();
                } else {
                    self.hide();
                    return HomeAction::Resume;
                }
            }
            Keysym::F5 => self.refresh(),
            Keysym::F1 => return HomeAction::ToggleControl,
            Keysym::l if self.overlay == Overlay::None => self.open_library(),
            Keysym::Menu if self.overlay == Overlay::None => self.action_menu(),
            Keysym::Down if self.overlay == Overlay::None => {
                if self.action_focus == 0 && self.selected() < self.entries().len() {
                    self.action_focus = 1;
                    self.changed();
                }
            }
            Keysym::Up if self.overlay == Overlay::None => {
                if self.action_focus > 0 {
                    self.action_focus -= 1;
                    self.changed();
                } else {
                    self.enter_header();
                }
            }
            Keysym::Left if self.overlay == Overlay::None && self.action_focus > 0 => {
                self.action_focus -= 1;
                self.changed();
            }
            Keysym::Right if self.overlay == Overlay::None && self.action_focus > 0 => {
                self.action_focus = if self.action_focus == 1 { 2 } else { 0 };
                self.changed();
            }
            Keysym::Left => self.step(-1),
            Keysym::Right => self.step(1),
            Keysym::Up => self.step(if self.overlay == Overlay::Search {
                -6
            } else {
                -1
            }),
            Keysym::Down => self.step(if self.overlay == Overlay::Search {
                6
            } else {
                1
            }),
            Keysym::Return | Keysym::KP_Enter | Keysym::space
                if self.action_focus == 2 && self.overlay == Overlay::None =>
            {
                self.action_menu()
            }
            Keysym::Return | Keysym::KP_Enter | Keysym::space => return self.activate(),
            _ => {}
        }
        HomeAction::None
    }

    pub fn click(&mut self, x: f64, y: f64) -> HomeAction {
        if self.launch.is_some() {
            return HomeAction::None;
        }
        let now = Instant::now();
        if !self.input_ready(false, now) {
            return HomeAction::None;
        }
        let previous = (self.version, self.overlay, self.scale_menu);
        let action = self.click_inner(x, y, now);
        if previous.0 != self.version {
            self.finish_intro(now);
        }
        self.feedback(previous, &action, Some(Cue::Select));
        action
    }

    fn click_inner(&mut self, x: f64, y: f64, now: Instant) -> HomeAction {
        let entrance = self.intro_at(now).and_then(|phase| phase.entrance);
        if self.overlay == Overlay::Control {
            let layout = Layout {
                height: self.view_height,
            };
            return self.click_control(layout, x * DESIGN_WIDTH, y * layout.height);
        }
        if self.overlay == Overlay::Search {
            let (x, y) = (x * WIDTH, y * HEIGHT);
            if self.click_search_keyboard(x, y) {
                return HomeAction::None;
            }
            if (122.0..698.0).contains(&x) && (134.0..610.0).contains(&y) {
                let page = self.selected() / GRID_CAPACITY * GRID_CAPACITY;
                let index = page + (((y - 134.0) / 119.0) as usize * 6) + ((x - 122.0) / 96.0) as usize;
                if index < self.search_indices().len() {
                    self.set_selected(index);
                    return self.activate();
                }
            }
            return HomeAction::None;
        }
        if self.overlay == Overlay::More {
            let (x, y) = (x * WIDTH, y * HEIGHT);
            let bottom = 346.0 + self.more_labels().len() as f64 * 58.0;
            if (414.0..866.0).contains(&x) && (346.0..bottom).contains(&y) {
                self.more = ((y - 346.0) / 58.0) as usize;
                return self.activate_more();
            }
            self.close_overlay();
            return HomeAction::None;
        }
        let layout = Layout {
            height: self.view_height,
        };
        let (x, y) = (x * DESIGN_WIDTH, y * layout.height);
        if self.scale_menu.is_some() {
            if let Some(index) = (0..SCALES.len()).find(|index| layout.scale_choice(*index).contains(x, y)) {
                return self.choose_scale(index);
            }
            self.scale_menu = None;
            self.changed();
            return HomeAction::None;
        }
        if let Some(item) = HEADER
            .into_iter()
            .find(|item| layout.header_hit(*item, self.header_page()).contains(x, y))
        {
            return self.activate_header(item);
        }
        if self.header_focus.take().is_some() {
            self.changed();
        }
        if self.library.open {
            return self.click_library(layout, x, y);
        }
        if self.page == Page::Settings {
            if let Some(index) = (0..SETTINGS.len()).find(|index| layout.category(*index).contains(x, y)) {
                self.category = index;
                self.setting = 0;
                self.settings_options = false;
                self.changed();
            } else if let Some(index) =
                (0..SETTINGS[self.category].1.len()).find(|index| layout.setting(*index).contains(x, y))
            {
                self.setting = index;
                self.settings_options = true;
                return self.activate_setting(1);
            }
        } else {
            let selected = self.selected();
            if let Some((index, _, _)) = layout
                .tiles(self.carousel_pose(now), self.entries().len(), entrance)
                .find(|(_, rect, _)| rect.contains(x, y))
            {
                self.select_tile(index, now);
                self.action_focus = 0;
                self.changed();
                if selected == index || index == self.entries().len() {
                    return self.activate();
                }
            }
            if self.entries().get(selected).is_some() {
                let (action, more) = layout.buttons(self.info_center());
                if action.contains(x, y) {
                    return self.activate();
                }
                if more.contains(x, y) {
                    self.action_menu();
                }
            }
        }
        HomeAction::None
    }

    fn activate_more(&mut self) -> HomeAction {
        let option = self.more;
        self.close_overlay();
        match option {
            0 if self.library.open => self.activate_library(),
            0 => self.activate(),
            1 => {
                self.toggle_favorite();
                HomeAction::None
            }
            _ => {
                self.open_library();
                HomeAction::None
            }
        }
    }

    fn activate(&mut self) -> HomeAction {
        let selected = self.selected();
        let index = if self.overlay == Overlay::Search {
            match self.search_indices().get(selected) {
                Some(index) => *index,
                None => return HomeAction::None,
            }
        } else {
            selected
        };
        let Some(item) = self.entries().get(index) else {
            self.open_library();
            return HomeAction::None;
        };
        if self.page == Page::Games {
            return HomeAction::LaunchGame {
                id: item.id.clone(),
                title: item.label.clone(),
                art: item.hero.clone(),
            };
        }
        HomeAction::LaunchApp(item.id.clone())
    }

    pub fn render<R>(
        &mut self,
        renderer: &mut R,
        output: &Output,
        enabled: bool,
        presentation_time: Instant,
    ) -> Vec<CustomRenderElements<R>>
    where
        R: Renderer + ImportAll + ImportMem,
        R::TextureId: Clone + Send + 'static,
    {
        if !enabled || (!self.uses_keyboard() && self.launch.is_none()) {
            return Vec::new();
        }
        let Some(mode) = output.current_mode() else {
            return Vec::new();
        };
        let physical = output.current_transform().transform_size(mode.size);
        if physical.w <= 0 || physical.h <= 0 {
            return Vec::new();
        }
        let size = (physical.w, physical.h);
        self.view_height = DESIGN_WIDTH * f64::from(physical.h) / f64::from(physical.w);
        if size != self.resolution {
            self.scene = Scene::default();
            self.resolution = size;
        }
        let now = self
            .last_frame_time
            .map_or(presentation_time, |last| last.max(presentation_time));
        self.last_frame_time = Some(now);
        self.advance_intro(now);
        self.check_launch(Instant::now());
        let local = Local::now();
        let minute = (local.timestamp() / 60) as u64;
        let time = local.format("%H:%M").to_string();
        let mut scene = std::mem::take(&mut self.scene);
        let elements = scene.render(
            self,
            renderer,
            size,
            output.current_scale().fractional_scale(),
            now,
            &time,
            minute,
        );
        self.scene = scene;
        elements
    }

    pub fn save_preview(&mut self, file: &Path, view: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.page = match view {
            "welcome" | "games" | "control" => Page::Games,
            "apps" => Page::Apps,
            "settings" => Page::Settings,
            _ => {
                return Err(format!(
                    "Unknown view {view:?}. Choose welcome, games, apps, settings, or control."
                )
                .into())
            }
        };
        let time = Local::now().format("%H:%M").to_string();
        let surface = ImageSurface::create(Format::ARgb32, 2880, 1800)?;
        let cr = Context::new(&surface)?;
        let layout = Layout {
            height: visual::DESIGN_HEIGHT,
        };
        if view == "welcome" {
            self.paint_warm_gradient(&cr, layout)?;
            visual::paint_welcome(&cr, layout, 5.15, &self.particles)?;
        } else {
            self.paint_full(&cr, layout, &time, Instant::now())?;
            if view == "control" {
                self.paint_control_scrim(&cr, layout)?;
                self.paint_control(&cr, layout)?;
            }
        }
        surface.write_to_png(&mut fs::File::create(file)?)?;
        Ok(())
    }

    fn art(&mut self, path: &Path) -> Option<ImageSurface> {
        if let Some(image) = self.art.get(path) {
            return image.clone();
        }
        if self.queued_art.contains(path) {
            return None;
        }
        if let Some(requests) = &self.art_requests {
            if requests.send(path.to_owned()).is_ok() {
                self.queued_art.insert(path.to_owned());
                return None;
            }
        }
        let image = load_art(path);
        self.art.insert(path.to_owned(), image.clone());
        image
    }

    fn paint_full(
        &mut self,
        cr: &Context,
        layout: Layout,
        time: &str,
        now: Instant,
    ) -> Result<(), cairo::Error> {
        if self.visible {
            if self.overlay == Overlay::Search {
                self.paint_warm_gradient(cr, layout)?;
            } else if self.library.open {
                self.paint_library_page(cr, layout, time)?;
            } else {
                self.paint_main(cr, layout, time, now, None)?;
            }
        }
        cr.save()?;
        cr.scale(DESIGN_WIDTH / WIDTH, layout.height / HEIGHT);
        match self.overlay {
            Overlay::Search => self.paint_search(cr)?,
            Overlay::More => self.paint_more(cr)?,
            Overlay::None | Overlay::Control => (),
        }
        if let Some(notice) = &self.notice {
            pill(cr, 765.0, 84.0, 475.0, 49.0, (0.03, 0.05, 0.12, 0.93))?;
            text(cr, notice, 780.0, 115.0, 17.0, 1.0, false)?;
        }
        cr.restore()
    }

    fn paint_tile(
        &mut self,
        cr: &Context,
        item: Option<&Entry>,
        x: f64,
        y: f64,
        width: f64,
        selected: bool,
    ) -> Result<(), cairo::Error> {
        rounded(cr, x, y, width, width, 17.0);
        cr.set_source_rgba(0.12, 0.13, 0.18, 0.94);
        cr.fill()?;
        if selected {
            rounded(cr, x - 3.0, y - 3.0, width + 6.0, width + 6.0, 19.0);
            cr.set_line_width(1.5);
            cr.set_source_rgba(0.78, 0.83, 1.0, 0.8);
            cr.stroke()?;
        }
        if let Some(art) = item
            .and_then(|entry| entry.image.as_deref())
            .and_then(|path| self.art(path))
        {
            cr.save()?;
            rounded(cr, x, y, width, width, 17.0);
            cr.clip();
            let inset = if self.page == Page::Apps {
                width * 0.15
            } else {
                0.0
            };
            image(
                cr,
                &art,
                (x + inset, y + inset, width - 2.0 * inset, width - 2.0 * inset),
                self.page == Page::Games,
            )?;
            cr.restore()?;
        } else {
            let fallback = if self.page == Page::Games {
                "icons/game-library.png"
            } else {
                "icons/search.png"
            };
            self.icon(
                cr,
                fallback,
                (x + width * 0.28, y + width * 0.28, width * 0.44, width * 0.44),
            )?;
        }
        if selected || item.is_some_and(|entry| entry.image.is_none()) {
            cr.save()?;
            cr.rectangle(x - 2.0, y + width, width + 8.0, 30.0);
            cr.clip();
            text(
                cr,
                item.map_or(
                    if self.page == Page::Games {
                        "Game Library"
                    } else {
                        "App Library"
                    },
                    |entry| &entry.label,
                ),
                x + 2.0,
                y + width + 18.0,
                14.0,
                1.0,
                false,
            )?;
            cr.restore()?;
        }
        Ok(())
    }

    fn icon(&mut self, cr: &Context, relative: &str, rect: (f64, f64, f64, f64)) -> Result<(), cairo::Error> {
        if let Some(art) = self.art(&self.root.join(relative)) {
            image(cr, &art, rect, false)?;
        }
        Ok(())
    }

    fn more_labels(&self) -> Vec<&'static str> {
        let action = if self.page == Page::Games {
            if self.favorite_entry().is_some_and(|entry| self.running_games.contains(&entry.id)) {
                "Continue"
            } else {
                "Play"
            }
        } else { "Open" };
        let pin = if self.favorite_entry().is_some_and(|entry| self.favorites.contains(self.page, &entry.id)) {
            "Unpin from Home"
        } else { "Pin to Home" };
        let mut labels = vec![action, pin];
        if !self.library.open { labels.push("Open Library"); }
        labels
    }

    fn paint_more(&mut self, cr: &Context) -> Result<(), cairo::Error> {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.32);
        cr.paint()?;
        let items = self.more_labels();
        pill(cr, 400.0, 304.0, 480.0, 51.0 + 58.0 * items.len() as f64, (0.08, 0.10, 0.15, 0.97))?;
        let title = self.favorite_entry().map_or("", |entry| entry.label.as_str());
        cr.save()?;
        cr.rectangle(427.0, 309.0, 430.0, 33.0);
        cr.clip();
        text(cr, title, 428.0, 333.0, 21.0, 1.0, true)?;
        cr.restore()?;
        for (index, label) in items.into_iter().enumerate() {
            let y = 346.0 + index as f64 * 58.0;
            if index == self.more {
                pill(cr, 414.0, y, 452.0, 55.0, (0.20, 0.25, 0.35, 1.0))?;
            }
            text(cr, label, 430.0, y + 35.0, 20.0, 1.0, false)?;
        }
        Ok(())
    }

    fn paint_search(&mut self, cr: &Context) -> Result<(), cairo::Error> {
        pill(cr, 90.0, 82.0, 1100.0, 580.0, (0.025, 0.04, 0.09, 0.97))?;
        text(
            cr,
            if self.page == Page::Games { "Search Games" } else { "Search Apps" },
            121.0,
            120.0,
            31.0,
            1.0,
            true,
        )?;
        let selected = self.selected();
        let start = selected / GRID_CAPACITY * GRID_CAPACITY;
        let search = if self.search.is_empty() { "Type to search" } else { &self.search };
        cr.save()?;
        cr.rectangle(460.0, 84.0, 608.0, 46.0);
        cr.clip();
        text(cr, search, 460.0, 118.0, 24.0, 0.85, false)?;
        cr.restore()?;
        self.paint_search_keyboard(cr)?;
        let entries: Vec<_> = self
            .search_indices()
            .into_iter()
            .skip(start)
            .take(GRID_CAPACITY)
            .filter_map(|index| self.entries().get(index).cloned())
            .collect();
        for (i, entry) in entries.iter().enumerate() {
            self.paint_tile(
                cr,
                Some(entry),
                122.0 + (i % 6) as f64 * 96.0,
                134.0 + (i / 6) as f64 * 119.0,
                78.0,
                start + i == selected && self.search_keyboard.is_none(),
            )?;
        }
        Ok(())
    }
}

// Load artwork and sound before the dashboard entrance without blocking scanout.
pub(crate) fn install_runtime<B: Backend + 'static>(state: &mut AnvilState<B>, active: bool) {
    state.home.audio = Some(Audio::start(&state.home.root));
    state.home.set_audio_active(active);
    status::monitor(state);
    services::install(state);
    crate::capture::install(state);
    let (games, updates) = channel::<(u64, Vec<Entry>)>();
    if state.handle.insert_source(updates, |event, _, state| {
        if let Event::Msg((revision, games)) = event {
            if revision == state.home.library.revision {
                state.home.update_game_library(games);
            }
        }
    }).is_ok() {
        let root = state.home.root.clone();
        let (requests, jobs) = mpsc::channel();
        state.home.game_catalog_requests = Some(requests);
        std::thread::spawn(move || {
            for revision in jobs {
                if games.send((revision, steam_games(&root))).is_err() {
                    break;
                }
            }
        });
    }

    let (send, receive) = channel::<(PathBuf, Option<(i32, i32, i32, Vec<u8>)>)>();
    if let Err(error) = state.handle.insert_source(receive, |event, _, state| {
        if let Event::Msg((path, image)) = event {
            state.home.queued_art.remove(&path);
            let surface = image.and_then(|(w, h, stride, data)| {
                ImageSurface::create_for_data(data, Format::ARgb32, w, h, stride).ok()
            });
            state.home.art.insert(path, surface);
            state.home.art_revision = state.home.art_revision.wrapping_add(1);
            state.home.changed();
        }
    }) {
        tracing::warn!(%error, "Cannot preload dashboard artwork");
        return;
    }

    let home = &mut state.home;
    let mut files = [
        "icons/ps-background.png",
        "icons/search.png",
        "icons/settings.png",
        "icons/game-library.png",
        "status_notice_controller_bg.png",
        "status_notice_controller_icon.png",
        "status_notice_microphone_bg.png",
        "status_notice_microphone_icon.png",
        "status_notice_battery_outline.png",
    ]
    .into_iter()
    .map(|path| home.root.join(path))
    .collect::<Vec<_>>();
    files.push(home.profile.clone());
    files.extend(CONTROL.into_iter().map(|item| home.root.join(item.icon())));
    for entry in home
        .games
        .iter()
        .take(HOME_LIMIT)
        .chain(home.apps.iter().take(HOME_LIMIT))
    {
        files.extend(
            [&entry.hero, &entry.image, &entry.logo]
                .into_iter()
                .flatten()
                .cloned(),
        );
    }
    let (requests, jobs) = mpsc::channel::<PathBuf>();
    home.art_requests = Some(requests);
    for path in files {
        home.art(&path);
    }
    std::thread::spawn(move || {
        for path in jobs {
            let image = load_art(&path).and_then(|image| {
                let mut pixels = Vec::new();
                image.with_data(|data| pixels.extend_from_slice(data)).ok()?;
                Some((image.width(), image.height(), image.stride(), pixels))
            });
            if send.send((path, image)).is_err() {
                break;
            }
        }
    });
}

const SETTINGS: [(&str, &[&str]); 5] = [
    (
        "Display",
        &["Display Scale", "GPU", "Menu Font Size", "Menu Font Weight"],
    ),
    ("System", &["Log Out", "Shut Down", "Terminal", "Suspend"]),
    ("About", &["PS5-WC", "Installed Build", "Resolution"]),
    (
        "Performance",
        &["System Load", "System Memory", "DE Process Memory", "Process ID"],
    ),
    ("Accessibility", &["Reduced Motion", "High Contrast", "Controller Confirm Button"]),
];

fn read_choice<T: Copy + PartialEq + std::str::FromStr>(
    root: &Path,
    file: &str,
    choices: &[T],
    default: T,
) -> T {
    fs::read_to_string(root.join(file))
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .filter(|value| choices.contains(value))
        .unwrap_or(default)
}

fn next_choice<T: Copy + PartialEq, const N: usize>(choices: [T; N], current: T, delta: isize) -> T {
    let index = choices.iter().position(|choice| *choice == current).unwrap_or(0);
    choices[wrap(index, delta, N)]
}

fn system_memory() -> Option<String> {
    let info = fs::read_to_string("/proc/meminfo").ok()?;
    let read = |name: &str| {
        info.lines()
            .find_map(|line| line.strip_prefix(name))?
            .split_whitespace()
            .next()?
            .parse::<f64>()
            .ok()
    };
    let total = read("MemTotal:")?;
    let used = total - read("MemAvailable:")?;
    let gib = 1024.0 * 1024.0;
    Some(format!(
        "{:.1}/{:.1} GB ({:.0}%)",
        used / gib,
        total / gib,
        used * 100.0 / total
    ))
}

fn bounded(index: usize, delta: isize, size: usize) -> usize {
    index.saturating_add_signed(delta).min(size.saturating_sub(1))
}

fn wrap(index: usize, delta: isize, size: usize) -> usize {
    if size == 0 {
        return 0;
    }
    (index as isize + delta).rem_euclid(size as isize) as usize
}

fn rounded(cr: &Context, x: f64, y: f64, width: f64, height: f64, radius: f64) {
    let pi = std::f64::consts::PI;
    cr.new_sub_path();
    cr.arc(x + width - radius, y + radius, radius, -pi / 2.0, 0.0);
    cr.arc(x + width - radius, y + height - radius, radius, 0.0, pi / 2.0);
    cr.arc(x + radius, y + height - radius, radius, pi / 2.0, pi);
    cr.arc(x + radius, y + radius, radius, pi, 3.0 * pi / 2.0);
    cr.close_path();
}

fn pill(
    cr: &Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    color: (f64, f64, f64, f64),
) -> Result<(), cairo::Error> {
    rounded(cr, x, y, width, height, (height / 2.0).min(24.0));
    cr.set_source_rgba(color.0, color.1, color.2, color.3);
    cr.fill()
}

fn text(
    cr: &Context,
    label: &str,
    x: f64,
    y: f64,
    size: f64,
    alpha: f64,
    bold: bool,
) -> Result<(), cairo::Error> {
    cr.select_font_face(
        "Sans",
        FontSlant::Normal,
        if bold {
            FontWeight::Bold
        } else {
            FontWeight::Normal
        },
    );
    cr.set_font_size(size);
    cr.set_source_rgba(0.98, 0.98, 1.0, alpha);
    cr.move_to(x, y);
    cr.show_text(&label.replace('\0', ""))
}

fn image(
    cr: &Context,
    surface: &ImageSurface,
    (x, y, width, height): (f64, f64, f64, f64),
    cover: bool,
) -> Result<(), cairo::Error> {
    let (iw, ih) = (f64::from(surface.width()), f64::from(surface.height()));
    let scale = if cover {
        (width / iw).max(height / ih)
    } else {
        (width / iw).min(height / ih)
    };
    cr.save()?;
    cr.rectangle(x, y, width, height);
    cr.clip();
    cr.translate(x + (width - iw * scale) / 2.0, y + (height - ih * scale) / 2.0);
    cr.scale(scale, scale);
    cr.set_source_surface(surface, 0.0, 0.0)?;
    cr.source().set_extend(cairo::Extend::Pad);
    cr.paint()?;
    cr.restore()
}

fn field(line: &str, name: &str) -> Option<String> {
    let mut values = line.split('"');
    values.next()?;
    if values.next()? != name {
        return None;
    }
    values.next()?;
    Some(values.next()?.to_owned())
}

fn sort_catalog(entries: &mut [Entry]) {
    entries.sort_by_cached_key(|entry| (
        gio::glib::casefold(&entry.label),
        gio::glib::casefold(&entry.id),
    ));
}

fn order_entries<'a>(entries: &mut [Entry], ids: impl Iterator<Item = &'a str>) {
    let order: HashMap<_, _> = ids.enumerate().map(|(i, id)| (id, i)).collect();
    entries.sort_by_key(|entry| order.get(entry.id.as_str()).copied().unwrap_or(usize::MAX));
}

fn ordered(root: &Path, file: &str, mut entries: Vec<Entry>) -> Vec<Entry> {
    let preferences = fs::read_to_string(root.join(file)).unwrap_or_default();
    order_entries(&mut entries, preferences.lines().map(str::trim));
    entries
}

fn move_to_front(entries: &mut [Entry], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    entries[..=index].rotate_right(1);
    true
}

fn steam_games(root: &Path) -> Vec<Entry> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/share")));
    let Some(steam) = data.map(|dir| dir.join("Steam")) else {
        return Vec::new();
    };
    let mut libraries = vec![steam.clone()];
    if let Ok(vdf) = fs::read_to_string(steam.join("steamapps/libraryfolders.vdf")) {
        libraries.extend(
            vdf.lines()
                .filter_map(|line| field(line, "path"))
                .map(PathBuf::from),
        );
    }
    let cache = steam.join("appcache/librarycache");
    let mut games = HashMap::new();
    for library in libraries {
        for file in fs::read_dir(library.join("steamapps"))
            .into_iter()
            .flatten()
            .flatten()
        {
            let filename = file.file_name();
            let Some(id) = filename
                .to_str()
                .and_then(|name| name.strip_prefix("appmanifest_"))
                .and_then(|name| name.strip_suffix(".acf"))
            else {
                continue;
            };
            if id.is_empty() || !id.bytes().all(|ch| ch.is_ascii_digit()) || games.contains_key(id) {
                continue;
            }
            let label = fs::read_to_string(file.path())
                .ok()
                .and_then(|text| text.lines().find_map(|line| field(line, "name")))
                .unwrap_or_else(|| id.into());
            if [
                "Proton",
                "Steamworks Common Redist",
                "Steam Linux Runtime",
                "SteamVR",
            ]
            .iter()
            .any(|prefix| label.starts_with(prefix))
            {
                continue;
            }
            let game_cache = cache.join(id);
            let art = |name: &str| {
                let direct = game_cache.join(name);
                if direct.is_file() {
                    return Some(direct);
                }
                fs::read_dir(&game_cache)
                    .into_iter()
                    .flatten()
                    .flatten()
                    .map(|entry| entry.path().join(name))
                    .find(|path| path.is_file())
                    .or_else(|| {
                        let flat = cache.join(format!("{id}_{name}"));
                        flat.is_file().then_some(flat)
                    })
            };
            let capsule = art("library_capsule.jpg");
            let hero = art("library_hero.jpg");
            let legacy_capsule = art("library_600x900.jpg");
            let header = art("header.jpg").or_else(|| art("library_header.jpg"));
            games.insert(
                id.to_owned(),
                Entry {
                    id: id.into(),
                    label,
                    image: capsule
                        .clone()
                        .or_else(|| hero.clone())
                        .or_else(|| legacy_capsule.clone())
                        .or_else(|| header.clone()),
                    hero: hero.or(legacy_capsule).or(header).or(capsule),
                    logo: art("logo.png"),
                },
            );
        }
    }
    let mut games: Vec<_> = games.into_values().collect();
    sort_catalog(&mut games);
    ordered(root, "game_order", games)
}

fn apps() -> Vec<Entry> {
    let mut entries = Vec::new();
    let theme = IconTheme::new();
    let fallback = theme.path("application-x-executable");
    for app in gio::AppInfo::all().into_iter().filter(|app| app.should_show()) {
        let Some(id) = app.id().filter(|id| id.ends_with(".desktop")) else {
            continue;
        };
        let art = app
            .icon()
            .and_then(|icon| {
                if let Some(file) = icon.downcast_ref::<gio::FileIcon>() {
                    return file.file().path();
                }
                if let Some(themed) = icon.downcast_ref::<gio::ThemedIcon>() {
                    return themed.names().iter().find_map(|name| theme.path(name));
                }
                None
            })
            .or_else(|| fallback.clone());
        entries.push(Entry {
            id: id.to_string(),
            label: app.display_name().to_string(),
            image: art,
            hero: None,
            logo: None,
        });
    }
    sort_catalog(&mut entries);
    entries
}
