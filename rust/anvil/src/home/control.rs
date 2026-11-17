use std::time::Instant;

use cairo::{Context, Extend, LinearGradient, RadialGradient};
use smithay::input::keyboard::Keysym;

use super::{
    bounded,
    native::{label, Align},
    services::{Audio, Request},
    visual::{orbit, Layout, Rect, DESIGN_WIDTH},
    wrap, Home, HomeAction,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Control {
    Home,
    Switcher,
    Notifications,
    Friends,
    Music,
    GameAlerts,
    Sound,
    Microphone,
    Controller,
    Profile,
    Power,
}

pub(super) const CONTROL: [Control; 11] = [
    Control::Home,
    Control::Switcher,
    Control::Notifications,
    Control::Friends,
    Control::Music,
    Control::GameAlerts,
    Control::Sound,
    Control::Microphone,
    Control::Controller,
    Control::Profile,
    Control::Power,
];

impl Control {
    pub(super) fn icon(self) -> &'static str {
        match self {
            Self::Home => "icons/control_center/cc-home.svg",
            Self::Switcher => "icons/control_center/cc-storage.svg",
            Self::Notifications => "icons/control_center/cc-notifications.svg",
            Self::Friends => "icons/control_center/cc-friends.svg",
            Self::Music => "icons/control_center/cc-music.svg",
            Self::GameAlerts => "icons/control_center/cc-alert.svg",
            Self::Sound => "icons/control_center/cc-volume.svg",
            Self::Microphone => "icons/control_center/cc-microphone.svg",
            Self::Controller => "icons/control_center/cc-controller.svg",
            Self::Profile => "icons/control_center/cc-profile.png",
            Self::Power => "icons/control_center/cc-power.svg",
        }
    }

    fn unavailable(self) -> &'static str {
        match self {
            Self::Friends => "Friends",
            Self::GameAlerts => "Game Alerts",
            Self::Profile => "Profile",
            _ => unreachable!("an implemented Control Center action"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Panel {
    Audio(Audio),
    Switcher,
    Music,
    Controller,
    Notifications,
    Create,
    Gallery,
}

const VISIBLE_ROWS: usize = 5;
const ON_WHITE: (f64, f64, f64) = (0.08, 0.11, 0.16);
const ON_PANEL: (f64, f64, f64) = (0.95, 0.96, 1.0);

#[derive(Clone, Copy)]
struct Card {
    x: f64,
    y: f64,
    height: f64,
    width: f64,
    row: f64,
}

impl Card {
    fn new(layout: Layout, rows: usize) -> Self {
        let width = 1060.0;
        let row = layout.y(84.0);
        let height = layout.y(177.0) + rows.clamp(1, VISIBLE_ROWS) as f64 * row;
        Self {
            x: (DESIGN_WIDTH - width) / 2.0,
            y: layout.y(1574.0) - height,
            height,
            width,
            row,
        }
    }

    fn item(self, index: usize, layout: Layout) -> Rect {
        Rect(
            self.x + 34.0,
            self.y + layout.y(105.0) + index as f64 * self.row,
            self.width - 68.0,
            self.row,
        )
    }
}

const POWER_ACTIONS: [&str; 4] = ["Log Out", "Shut Down", "Suspend", "Cancel"];

impl Layout {
    fn control_center(self, index: usize) -> (f64, f64) {
        const PITCH: f64 = 171.6;
        let offset = index as f64 - (CONTROL.len() - 1) as f64 / 2.0;
        (DESIGN_WIDTH / 2.0 - 1.0 + offset * PITCH, self.y(1675.5))
    }

    fn control_cell(self, index: usize) -> Rect {
        let (x, y) = self.control_center(index);
        Rect(x - 85.8, y - self.y(90.0), 171.6, self.y(180.0))
    }

    fn power_item(self, index: usize) -> Rect {
        let s = self.y(86.4);
        let (x, _) = self.control_center(CONTROL.len() - 1);
        let bottom = self.y(1605.6);
        Rect(
            x - 187.2,
            bottom - (POWER_ACTIONS.len() - index) as f64 * s,
            374.4,
            s,
        )
    }

    pub(super) fn control_top(self, power_open: bool) -> f64 {
        if power_open {
            self.power_item(0).1 - 2.0
        } else {
            self.y(1615.0)
        }
    }
}

impl Home {
    pub(super) fn click_control(&mut self, layout: Layout, x: f64, y: f64) -> HomeAction {
        if self.control_panel.is_some() {
            return self.click_panel(layout, x, y);
        }
        if self.control_power.is_some() {
            if let Some(index) =
                (0..POWER_ACTIONS.len()).find(|&i| layout.power_item(i).contains(x, y))
            {
                return self.activate_power(index);
            }
            self.control_power = None;
            self.changed();
            return HomeAction::None;
        }
        if let Some(index) = (0..CONTROL.len()).find(|&i| layout.control_cell(i).contains(x, y)) {
            self.control = index;
            self.control_card_focus = None;
            self.changed();
            return self.activate_control();
        }
        if let Some(index) =
            (0..2).find(|&i| self.has_card(i) && Self::activity_card(layout, i).contains(x, y))
        {
            self.control_card_focus = Some(index);
            self.open_panel(Self::card_panel(index));
            return HomeAction::None;
        }
        HomeAction::ToggleControl
    }

    pub(super) fn press_control(&mut self, sym: Keysym) -> HomeAction {
        if sym == Keysym::F1 {
            return HomeAction::ToggleControl;
        }
        if self.control_panel.is_some() {
            return self.press_panel(sym);
        }
        if let Some(index) = self.control_power {
            match sym {
                Keysym::Escape => self.control_power = None,
                Keysym::Up | Keysym::Left => {
                    self.control_power = Some(bounded(index, -1, POWER_ACTIONS.len()))
                }
                Keysym::Down | Keysym::Right => {
                    self.control_power = Some(bounded(index, 1, POWER_ACTIONS.len()))
                }
                Keysym::Return | Keysym::KP_Enter | Keysym::space => {
                    return self.activate_power(index)
                }
                _ => return HomeAction::None,
            }
        } else {
            match sym {
                Keysym::Escape => {
                    if self.control_card_focus.take().is_some() {
                        self.changed();
                    } else {
                        return HomeAction::ToggleControl;
                    }
                }
                Keysym::Up => self.control_card_focus = (0..2).find(|&index| self.has_card(index)),
                Keysym::Down => self.control_card_focus = None,
                Keysym::Left | Keysym::Right if self.control_card_focus.is_none() => {
                    let delta = if sym == Keysym::Left { -1 } else { 1 };
                    self.control = wrap(self.control, delta, CONTROL.len());
                    self.notice = None;
                }
                Keysym::Left | Keysym::Right => {
                    let target = usize::from(sym == Keysym::Right);
                    if self.has_card(target) {
                        self.control_card_focus = Some(target);
                    }
                }
                Keysym::Return | Keysym::KP_Enter | Keysym::space => {
                    if let Some(index) = self
                        .control_card_focus
                        .filter(|&index| self.has_card(index))
                    {
                        self.open_panel(Self::card_panel(index));
                    } else {
                        return self.activate_control();
                    }
                }
                _ => return HomeAction::None,
            }
        }
        self.changed();
        HomeAction::None
    }

    fn activate_control(&mut self) -> HomeAction {
        let item = CONTROL[self.control];
        match item {
            Control::Home => return HomeAction::ShowHome,
            Control::Switcher => {
                self.open_panel(Panel::Switcher);
                return HomeAction::RefreshSwitcher;
            }
            Control::Notifications => self.open_panel(Panel::Notifications),
            Control::Music => self.open_panel(Panel::Music),
            Control::Sound => self.open_panel(Panel::Audio(Audio::Output)),
            Control::Microphone => self.open_panel(Panel::Audio(Audio::Input)),
            Control::Controller => self.open_panel(Panel::Controller),
            Control::Power => self.control_power = Some(0),
            item => self.notice = Some(format!("{} is unavailable", item.unavailable())),
        }
        self.changed();
        HomeAction::None
    }

    fn activate_power(&mut self, index: usize) -> HomeAction {
        self.control_power = None;
        self.changed();
        match index {
            0 => HomeAction::Quit,
            1 => HomeAction::run("systemctl", &["poweroff"]),
            2 => HomeAction::run("systemctl", &["suspend"]),
            _ => HomeAction::None,
        }
    }

    pub(super) fn paint_control_scrim(
        &self,
        cr: &Context,
        layout: Layout,
    ) -> Result<(), cairo::Error> {
        let dim = LinearGradient::new(0.0, 0.0, 0.0, layout.height);
        dim.add_color_stop_rgba(0.0, 0.006, 0.008, 0.012, 0.30);
        dim.add_color_stop_rgba(1.0, 0.006, 0.008, 0.012, 0.78);
        cr.set_source(&dim)?;
        cr.paint()?;

        let (x, y) = (DESIGN_WIDTH / 2.0, layout.height / 2.0);
        let vignette = RadialGradient::new(
            x,
            y,
            DESIGN_WIDTH.min(layout.height) * 0.12,
            x,
            y,
            x.hypot(y),
        );
        vignette.add_color_stop_rgba(0.0, 0.0, 0.0, 0.0, 0.0);
        vignette.add_color_stop_rgba(0.55, 0.0, 0.0, 0.0, 0.08);
        vignette.add_color_stop_rgba(1.0, 0.0, 0.0, 0.0, 0.55);
        cr.set_source(&vignette)?;
        cr.paint()
    }

    pub(super) fn paint_control(
        &mut self,
        cr: &Context,
        layout: Layout,
    ) -> Result<(), cairo::Error> {
        let side = layout.y(82.5);
        let radius = layout.y(53.4);
        for (index, item) in CONTROL.into_iter().enumerate() {
            let (x, y) = layout.control_center(index);
            let selected = index == self.control && self.control_card_focus.is_none();
            if selected {
                cr.arc(x, y, radius, 0.0, std::f64::consts::TAU);
                cr.set_source_rgba(0.91, 0.93, 0.97, 0.98);
                cr.fill()?;
                orbit(
                    cr,
                    Rect(x - radius, y - radius, 2.0 * radius, 2.0 * radius),
                    radius,
                    layout.y(2.1),
                )?;
            }
            let fallback = self.root.join(item.icon());
            let art = if item == Control::Profile {
                self.art(&self.profile.clone())
                    .or_else(|| self.art(&fallback))
            } else {
                self.art(&fallback)
            };
            if let Some(art) = art {
                let width = side * f64::from(art.width()) / f64::from(art.height());
                cr.save()?;
                cr.translate(x - width / 2.0, y - side / 2.0);
                cr.scale(
                    width / f64::from(art.width()),
                    side / f64::from(art.height()),
                );
                // The photo and the power icon already contain their own colors.
                if selected && !matches!(item, Control::Profile | Control::Power) {
                    cr.set_source_rgb(0.0, 0.0, 0.0);
                    cr.mask_surface(&art, 0.0, 0.0)?;
                } else {
                    cr.set_source_surface(&art, 0.0, 0.0)?;
                    cr.source().set_extend(Extend::None);
                    cr.paint_with_alpha(if selected { 1.0 } else { 0.72 })?;
                }
                cr.restore()?;
            }
        }
        if let Some(selected) = self.control_power {
            for (index, title) in POWER_ACTIONS.into_iter().enumerate() {
                let Rect(x, y, w, h) = layout.power_item(index);
                if index == selected {
                    cr.set_source_rgb(0.27, 0.39, 0.62);
                } else {
                    cr.set_source_rgba(0.10, 0.12, 0.16, 0.98);
                }
                cr.rectangle(x, y, w, h);
                cr.fill()?;
                label(
                    cr,
                    title,
                    (x + w / 2.0, y + h * 0.65),
                    layout.y(37.0),
                    400,
                    (1.0, 1.0, 1.0),
                    Align::Center,
                );
            }
        }
        Ok(())
    }
}

impl Home {
    pub(crate) fn control_open(&self) -> bool {
        self.overlay == super::Overlay::Control
    }

    pub(super) fn control_progress(&self, now: Instant) -> f64 {
        if self.reduced_motion {
            return 1.0;
        }
        self.control_opened.map_or(1.0, |start| {
            let progress =
                (now.saturating_duration_since(start).as_secs_f64() / 0.17).clamp(0.0, 1.0);
            1.0 - (1.0 - progress).powi(3)
        })
    }

    pub(super) fn watch_services(&self, active: bool) {
        if let Some(worker) = &self.service_requests {
            let _ = worker.send(Request::Watch(active));
        }
    }

    fn open_panel(&mut self, panel: Panel) {
        self.control_panel = Some(panel);
        self.control_parent = None;
        self.control_row = 0;
        self.control_close_window = false;
        self.control_message = None;
        self.changed();
    }

    pub(crate) fn open_create(&mut self) {
        self.open_panel(Panel::Create);
    }

    pub(crate) fn set_switcher(&mut self, windows: Vec<String>) {
        self.control_windows = windows;
        self.control_row = self
            .control_row
            .min(self.control_windows.len().saturating_sub(1));
        self.changed();
    }

    pub(crate) fn remove_switcher_row(&mut self, index: usize) {
        if index < self.control_windows.len() {
            self.control_windows.remove(index);
            if index < self.control_row {
                self.control_row -= 1;
            }
            self.control_row = self
                .control_row
                .min(self.control_windows.len().saturating_sub(1));
            self.control_close_window = false;
            self.changed();
        }
    }

    pub(super) fn record_notification(&mut self, message: &str) {
        if self
            .notifications
            .front()
            .is_some_and(|last| last == message)
        {
            return;
        }
        self.notifications.push_front(message.to_owned());
        self.notifications.truncate(20);
        self.changed();
    }

    pub(super) fn control_error(&mut self, error: &str) {
        self.record_notification(error);
        if self.control_open() {
            self.control_message = Some(error.to_owned());
            self.changed();
        }
    }

    fn panel_rows(&self, panel: Panel) -> usize {
        match panel {
            Panel::Audio(kind) => self.services.devices(kind).len(),
            Panel::Switcher => self.control_windows.len(),
            Panel::Music => {
                if self.services.media.is_some() {
                    3
                } else {
                    0
                }
            }
            Panel::Controller => 0,
            Panel::Notifications => self.notifications.len() + 1,
            Panel::Create => 1 + usize::from(self.last_capture.is_some()),
            Panel::Gallery => 0,
        }
    }

    fn panel_start(&self, count: usize) -> usize {
        self.control_row
            .min(count.saturating_sub(1))
            .saturating_sub(VISIBLE_ROWS - 1)
    }

    fn click_panel(&mut self, layout: Layout, x: f64, y: f64) -> HomeAction {
        let panel = self.control_panel.expect("open panel");
        if panel == Panel::Gallery {
            if !Self::gallery_rect(layout).contains(x, y) {
                return self.press_panel(Keysym::Escape);
            }
            return HomeAction::None;
        }
        let count = self.panel_rows(panel);
        let card = Card::new(layout, count);
        let start = self.panel_start(count);
        if let Some(index) = (0..count.saturating_sub(start).min(VISIBLE_ROWS))
            .find(|&i| card.item(i, layout).contains(x, y))
        {
            self.control_row = start + index;
            self.control_close_window = panel == Panel::Switcher && x > card.x + card.width - 230.0;
            self.changed();
            if matches!(panel, Panel::Audio(_)) && x > card.x + card.width - 180.0 {
                return self.press_panel(if x < card.x + card.width - 100.0 {
                    Keysym::Left
                } else {
                    Keysym::Right
                });
            }
            return self.press_panel(Keysym::Return);
        }
        if !Rect(card.x, card.y, card.width, card.height).contains(x, y) {
            return self.press_panel(Keysym::Escape);
        }
        HomeAction::None
    }

    fn press_panel(&mut self, sym: Keysym) -> HomeAction {
        let panel = self.control_panel.expect("open panel");
        let count = self.panel_rows(panel);
        self.control_row = self.control_row.min(count.saturating_sub(1));
        let index = self.control_row;
        let confirm = matches!(sym, Keysym::Return | Keysym::KP_Enter | Keysym::space);
        match sym {
            Keysym::Escape => {
                self.control_panel = self.control_parent.take();
                self.control_row = usize::from(self.control_panel == Some(Panel::Create));
            }
            Keysym::Up | Keysym::Down => {
                let delta = if sym == Keysym::Up { -1 } else { 1 };
                self.control_row = bounded(index, delta, count);
                self.control_close_window = false;
            }
            Keysym::Left | Keysym::Right if panel == Panel::Switcher => {
                self.control_close_window = sym == Keysym::Right;
            }
            _ => match panel {
                Panel::Switcher if confirm && index < self.control_windows.len() => {
                    return if self.control_close_window {
                        HomeAction::CloseWindow(index)
                    } else {
                        HomeAction::ResumeWindow(index)
                    };
                }
                Panel::Audio(kind) => {
                    if let Some(device) = self.services.devices(kind).get(index) {
                        let id = device.id.clone();
                        let request = match sym {
                            Keysym::Left if device.volume != 0 => {
                                Some(Request::Volume(kind, id, -5))
                            }
                            Keysym::Right if device.volume < 100 => Some(Request::Volume(
                                kind,
                                id,
                                (100 - device.volume).min(5) as i8,
                            )),
                            Keysym::Menu => Some(Request::Mute(kind, id)),
                            _ if confirm => Some(Request::Select(kind, id)),
                            _ => None,
                        };
                        if let Some(request) = request {
                            self.send_service(request);
                        }
                    }
                }
                Panel::Create if confirm => {
                    if index == 0 {
                        return HomeAction::Capture;
                    }
                    self.open_panel(Panel::Gallery);
                    self.control_parent = Some(Panel::Create);
                }
                Panel::Notifications if confirm && index == 0 => {
                    self.do_not_disturb = !self.do_not_disturb;
                    self.persist_toggle("do_not_disturb", self.do_not_disturb);
                }
                Panel::Music if confirm => {
                    if let Some(media) = &self.services.media {
                        let action = ["previous", "play-pause", "next"][index];
                        self.send_service(Request::Transport(media.player.clone(), action));
                    }
                }
                _ => return HomeAction::None,
            },
        }
        self.control_message = None;
        self.changed();
        HomeAction::None
    }

    pub(crate) fn pause_red(&self) {
        self.send_service(Request::PauseRed);
    }

    fn send_service(&self, request: Request) {
        if let Some(worker) = &self.service_requests {
            let _ = worker.send(request);
        }
    }

    fn has_card(&self, index: usize) -> bool {
        if index == 0 {
            self.services.media.is_some()
        } else {
            self.last_capture.is_some()
        }
    }

    fn card_panel(index: usize) -> Panel {
        if index == 0 {
            Panel::Music
        } else {
            Panel::Gallery
        }
    }

    fn activity_card(layout: Layout, index: usize) -> Rect {
        Rect(
            525.0 + index as f64 * 650.0,
            layout.y(1435.0),
            615.0,
            layout.y(135.0),
        )
    }

    fn gallery_rect(layout: Layout) -> Rect {
        Rect(570.0, layout.y(475.0), 1740.0, layout.y(1090.0))
    }

    pub(super) fn has_control_panel(&self) -> bool {
        self.control_power.is_none()
            && (self.control_panel.is_some() || self.has_card(0) || self.has_card(1))
    }

    pub(super) fn panel_top(&self, layout: Layout) -> f64 {
        if self.control_panel == Some(Panel::Gallery) {
            Self::gallery_rect(layout).1 - 2.0
        } else if let Some(panel) = self.control_panel {
            Card::new(layout, self.panel_rows(panel)).y - 2.0
        } else {
            Self::activity_card(layout, 0).1 - 2.0
        }
    }

    pub(super) fn paint_control_panel(
        &mut self,
        cr: &Context,
        layout: Layout,
    ) -> Result<(), cairo::Error> {
        if self.control_power.is_some() {
            return Ok(());
        }
        let Some(panel) = self.control_panel else {
            for index in (0..2).filter(|&index| self.has_card(index)) {
                let Rect(x, y, w, h) = Self::activity_card(layout, index);
                self.paint_card(cr, x, y, w, h)?;
                if self.control_card_focus == Some(index) {
                    self.focus_ring(cr, Rect(x + 4.0, y + 4.0, w - 8.0, h - 8.0), 29.0, 4.0, 1.0)?;
                }
                let (header, title) =
                    if let Some(media) = self.services.media.as_ref().filter(|_| index == 0) {
                        ("Now Playing", media.title.as_str())
                    } else {
                        ("Recent Capture", "View screenshot")
                    };
                label(
                    cr,
                    header,
                    (x + 32.0, y + layout.y(46.0)),
                    layout.y(30.0),
                    500,
                    ON_PANEL,
                    Align::Left,
                );
                card_line(
                    cr,
                    title,
                    Rect(x + 32.0, y + layout.y(94.0), w - 64.0, h),
                    layout.y(38.0),
                    ON_PANEL,
                    Align::Left,
                )?;
            }
            return Ok(());
        };
        if panel == Panel::Gallery {
            let Rect(x, y, w, h) = Self::gallery_rect(layout);
            self.paint_card(cr, x, y, w, h)?;
            label(
                cr,
                "Recent Capture",
                (x + 40.0, y + layout.y(70.0)),
                layout.y(48.0),
                700,
                ON_PANEL,
                Align::Left,
            );
            if let Some(path) = self.last_capture.clone() {
                if let Some(art) = self.art(&path) {
                    super::image(
                        cr,
                        &art,
                        (x + 32.0, y + layout.y(102.0), w - 64.0, h - layout.y(143.0)),
                        false,
                    )?;
                }
            }
            return Ok(());
        }
        let count = self.panel_rows(panel);
        let card = Card::new(layout, count);
        self.paint_card(cr, card.x, card.y, card.width, card.height)?;
        let title = match panel {
            Panel::Audio(Audio::Output) => "Sound",
            Panel::Audio(Audio::Input) => "Microphone",
            Panel::Switcher => "Switcher",
            Panel::Music => "Now Playing",
            Panel::Controller => "Controller",
            Panel::Notifications => "Notifications",
            Panel::Create => "Create",
            Panel::Gallery => unreachable!(),
        };
        label(
            cr,
            title,
            (card.x + 33.0, card.y + layout.y(65.0)),
            layout.y(50.0),
            700,
            ON_PANEL,
            Align::Left,
        );
        let footer = if let Some(message) = &self.control_message {
            message.as_str()
        } else {
            match panel {
                Panel::Audio(_) if count > 0 => "← / → Level     Enter Default     Options Mute",
                Panel::Switcher if count > 0 => "Select Resume or Close",
                Panel::Music => self
                    .services
                    .media
                    .as_ref()
                    .map_or("", |media| media.artist.as_str()),
                _ => "",
            }
        };
        if !footer.is_empty() {
            label(
                cr,
                footer,
                (card.x + 35.0, card.y + card.height - layout.y(24.0)),
                layout.y(27.0),
                400,
                ON_PANEL,
                Align::Left,
            );
        }
        if count == 0 {
            let value = match panel {
                Panel::Audio(Audio::Output) => "No audio outputs detected",
                Panel::Audio(Audio::Input) => "No microphones detected",
                Panel::Switcher => "No open games or apps",
                Panel::Music => "No media playing",
                Panel::Controller if self.status.controller => "Controller connected",
                Panel::Controller => "No controller connected",
                Panel::Notifications => "No notifications",
                Panel::Create | Panel::Gallery => unreachable!(),
            };
            let Rect(x, y, _, h) = card.item(0, layout);
            label(
                cr,
                value,
                (x + 18.0, y + h * 0.67),
                layout.y(37.0),
                400,
                ON_PANEL,
                Align::Left,
            );
            if panel == Panel::Controller {
                if let Some(battery) = self.status.battery {
                    label(
                        cr,
                        &format!("{battery}%"),
                        (card.x + card.width - 40.0, y + h * 0.67),
                        layout.y(37.0),
                        400,
                        ON_PANEL,
                        Align::Right,
                    );
                }
            }
            return Ok(());
        }
        let start = self.panel_start(count);
        for (row, index) in (start..count).take(VISIBLE_ROWS).enumerate() {
            let Rect(x, y, w, h) = card.item(row, layout);
            let focused = index == self.control_row.min(count - 1);
            let (title, value) = match panel {
                Panel::Audio(kind) => {
                    let device = &self.services.devices(kind)[index];
                    let mark = if device.default { "✓  " } else { "" };
                    let level = if device.muted {
                        "Muted".into()
                    } else {
                        format!("{}%", device.volume)
                    };
                    (format!("{mark}{}", device.name), format!("{level}   −  +"))
                }
                Panel::Switcher => (self.control_windows[index].clone(), String::new()),
                Panel::Music => {
                    let track = self.services.media.as_ref().unwrap();
                    (
                        if index == 1 {
                            format!(
                                "{}  {}",
                                if track.playing { "Pause" } else { "Play" },
                                track.title
                            )
                        } else if index == 0 {
                            "Previous Track".into()
                        } else {
                            "Next Track".into()
                        },
                        String::new(),
                    )
                }
                Panel::Create => (
                    if index == 0 {
                        "Take Screenshot"
                    } else {
                        "Recent Capture"
                    }
                    .into(),
                    String::new(),
                ),
                Panel::Gallery => unreachable!(),
                Panel::Notifications => {
                    if index == 0 {
                        (
                            "Do Not Disturb".into(),
                            if self.do_not_disturb { "On" } else { "Off" }.into(),
                        )
                    } else {
                        (self.notifications[index - 1].clone(), String::new())
                    }
                }
                Panel::Controller => unreachable!("controller panel has no selections"),
            };
            if focused && panel != Panel::Switcher {
                super::pill(cr, x, y + 3.0, w, h - 6.0, (0.91, 0.93, 0.98, 1.0))?;
            }
            let color = if focused && panel != Panel::Switcher {
                ON_WHITE
            } else {
                ON_PANEL
            };
            let label_width = w - match panel {
                Panel::Audio(_) => 260.0,
                Panel::Switcher => 380.0,
                _ => 36.0,
            };
            card_line(
                cr,
                &title,
                Rect(x + 20.0, y + h * 0.66, label_width, h),
                layout.y(36.0),
                color,
                Align::Left,
            )?;
            if panel == Panel::Switcher {
                for (left, name, close) in [
                    (x + w - 342.0, "Resume", false),
                    (x + w - 163.0, "Close", true),
                ] {
                    let active = focused && close == self.control_close_window;
                    if active {
                        super::pill(cr, left, y + 6.0, 160.0, h - 12.0, (0.91, 0.93, 0.98, 1.0))?;
                    }
                    label(
                        cr,
                        name,
                        (left + 80.0, y + h * 0.66),
                        layout.y(32.0),
                        600,
                        if active { ON_WHITE } else { ON_PANEL },
                        Align::Center,
                    );
                }
            } else if !value.is_empty() {
                label(
                    cr,
                    &value,
                    (x + w - 20.0, y + h * 0.66),
                    layout.y(33.0),
                    500,
                    color,
                    Align::Right,
                );
            }
        }
        Ok(())
    }

    fn paint_card(&self, cr: &Context, x: f64, y: f64, w: f64, h: f64) -> Result<(), cairo::Error> {
        super::rounded(cr, x, y, w, h, 30.0);
        if self.high_contrast {
            cr.set_source_rgb(0.017, 0.020, 0.026);
        } else {
            cr.set_source_rgba(0.081, 0.104, 0.148, 0.98);
        }
        cr.fill_preserve()?;
        cr.set_source_rgba(0.65, 0.72, 0.84, 0.26);
        cr.set_line_width(2.0);
        cr.stroke()
    }
}

fn card_line(
    cr: &Context,
    message: &str,
    Rect(x, baseline, width, height): Rect,
    size: f64,
    color: (f64, f64, f64),
    alignment: Align,
) -> Result<(), cairo::Error> {
    cr.save()?;
    cr.rectangle(x, baseline - height, width, height + size * 0.30);
    cr.clip();
    label(cr, message, (x, baseline), size, 500, color, alignment);
    cr.restore()
}
