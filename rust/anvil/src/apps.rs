//! Launch an installed desktop entry in this compositor's session.
use std::time::Duration;

use gio::{
    glib::{KeyFile, KeyFileFlags},
    prelude::*,
    AppLaunchContext, DesktopAppInfo,
};

use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

use crate::{state::Backend, window::Identity, AnvilState};

const DESKTOP: &str = "Desktop Entry";
pub(crate) const RED_DESKTOP: &str = "com.ktechpit.red.desktop";

pub(crate) struct RedWindow;

impl<B: Backend> AnvilState<B> {
    pub(crate) fn open_console_app(&mut self, id: &str) -> Result<(), String> {
        let app = DesktopAppInfo::new(id).ok_or_else(|| format!("{id} is no longer installed."))?;
        let launch = launch_entry(&app)?;
        let context = AppLaunchContext::new();
        context.unsetenv("WAYLAND_SOCKET");
        context.unsetenv("DESKTOP_STARTUP_ID");
        context.unsetenv("XDG_ACTIVATION_TOKEN");
        context.unsetenv("DISPLAY");
        if let Some(socket) = &self.socket_name {
            context.setenv("WAYLAND_DISPLAY", socket);
        }
        #[cfg(feature = "xwayland")]
        if let Some(display) = self.xdisplay {
            context.setenv("DISPLAY", format!(":{display}"));
        }

        // Apply theme preferences to these child processes only.
        context.setenv("GTK_THEME", "Adwaita:dark");
        context.setenv("ADW_DEBUG_COLOR_SCHEME", "prefer-dark");
        context.setenv("QT_QPA_PLATFORMTHEME", "xdgdesktopportal");

        if id == RED_DESKTOP {
            // Frame generation is for games. Red's Qt startup log showed this
            // implicit layer failing to load during GPU initialization.
            context.setenv("DISABLE_LSFGVK", "1");
        }

        let steam = id == "steam.desktop";
        let wm_class = app.startup_wm_class();
        if steam {
            launch
                .launch(&[], Some(&context))
                .map_err(|error| error.to_string())?;
            self.steam_ui_requested = true;
        }
        let existing = self
            .space
            .elements()
            .rev()
            .find(|window| {
                window.wl_surface().is_some_and(|surface| {
                    if !crate::input_handler::has_buffer(&surface) {
                        return false;
                    }
                    let identity = Identity::read(window, &surface, &self.display_handle);
                    identity.matches_desktop(id, wm_class.as_deref()) || (steam && identity.is_steam_ui())
                })
            })
            .cloned();

        if let Some(window) = existing {
            self.home.hide();
            if window.is_normal_toplevel() {
                self.fullscreen_window(&window, None);
            }
            self.focus_game_or_app(window);
            return Ok(());
        }

        if !steam {
            launch
                .launch(&[], Some(&context))
                .map_err(|error| error.to_string())?;
            self.steam_ui_requested = false;
        }

        self.home.app_launch_generation = self.home.app_launch_generation.wrapping_add(1);
        let generation = self.home.app_launch_generation;
        let id = id.to_owned();
        if let Err(error) = self.handle.insert_source(Timer::from_duration(Duration::from_secs(5)), move |_, _, state| {
            if state.home.app_launch_generation == generation {
                state.home.promote_app(&id);
            }
            TimeoutAction::Drop
        }) {
            tracing::warn!(%error, "Could not schedule app order update");
        }
        Ok(())
    }
}

fn launch_entry(app: &DesktopAppInfo) -> Result<DesktopAppInfo, String> {
    let action = app.list_actions().into_iter().find(|action| {
        let name: String = action
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|ch| ch.to_ascii_lowercase())
            .collect();
        matches!(name.as_str(), "tvmode" | "fullscreen" | "bigpicture" | "kiosk")
    });
    if action.is_none() && !app.boolean("DBusActivatable") {
        return Ok(app.clone());
    }

    let path = app.filename().ok_or("Desktop entry has no file.")?;
    let file = KeyFile::new();
    file.load_from_file(path, KeyFileFlags::NONE)
        .map_err(|error| error.to_string())?;
    // The Exec path inherits our display and theme even when the entry
    // also offers D-Bus activation through the login session.
    file.set_boolean(DESKTOP, "DBusActivatable", false);
    if let Some(action) = action {
        let exec = file
            .value(&format!("Desktop Action {action}"), "Exec")
            .map_err(|error| error.to_string())?;
        file.set_value(DESKTOP, "Exec", &exec);
    }
    DesktopAppInfo::from_keyfile(&file).ok_or_else(|| "Desktop entry has no usable command.".into())
}
