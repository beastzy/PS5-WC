use std::{fs, os::unix::fs::MetadataExt};

use crate::shell::WindowElement;
use smithay::{
    reexports::wayland_server::{protocol::wl_surface::WlSurface, DisplayHandle, Resource},
    wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData},
};

pub(crate) struct Identity {
    names: Vec<String>,
    title: String,
    pid: Option<u32>,
}

impl Identity {
    pub(crate) fn read(window: &WindowElement, surface: &WlSurface, display: &DisplayHandle) -> Self {
        #[cfg(feature = "xwayland")]
        if let Some(x11) = window.0.x11_surface() {
            return Self::from_x11(x11);
        }
        let (name, title) = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|role| {
                    let role = role.lock().ok()?;
                    Some((
                        role.app_id.clone().unwrap_or_default(),
                        role.title.clone().unwrap_or_default(),
                    ))
                })
                .unwrap_or_default()
        });
        let pid = surface
            .client()
            .and_then(|client| client.get_credentials(display).ok())
            .and_then(|creds| u32::try_from(creds.pid).ok());
        Self {
            names: vec![name],
            title,
            pid,
        }
    }

    #[cfg(feature = "xwayland")]
    pub(crate) fn from_x11(window: &smithay::xwayland::X11Surface) -> Self {
        Self {
            names: vec![window.class(), window.instance()],
            title: window.title(),
            pid: window.pid(),
        }
    }

    pub(crate) fn matches_desktop(&self, id: &str, wm_class: Option<&str>) -> bool {
        let id = id.trim_end_matches(".desktop");
        self.names.iter().any(|name| {
            name.eq_ignore_ascii_case(id)
                || wm_class.is_some_and(|class| !class.is_empty() && name.eq_ignore_ascii_case(class))
        })
    }

    pub(crate) fn label(&self) -> &str {
        if !self.title.trim().is_empty() {
            &self.title
        } else {
            self.names.iter().find(|name| !name.is_empty()).map_or("Application", |name| name)
        }
    }

    pub(crate) fn pid(&self) -> Option<u32> {
        self.pid
    }

    pub(crate) fn is_steam_ui(&self) -> bool {
        if self.names.iter().any(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "steam" | "steamwebhelper" | "steam-runtime" | "com.valvesoftware.steam"
            )
        }) {
            return true;
        }
        self.pid
            .and_then(|pid| fs::read_to_string(format!("/proc/{pid}/comm")).ok())
            .is_some_and(|comm| matches!(comm.trim(), "steam" | "steamwebhelper"))
    }

    pub(crate) fn matches_game(&self, app_id: &str, title: &str) -> bool {
        let by_name = |name: &str| {
            let name = name.to_ascii_lowercase();
            name.strip_prefix("steam_app_").is_some_and(|id| id == app_id)
                || name.strip_prefix("steam_appid_").is_some_and(|id| id == app_id)
        };
        if self.names.iter().any(|name| by_name(name)) {
            return true;
        }
        if self.is_steam_ui() {
            return false;
        }
        if let Some(pid) = self.pid {
            let path = format!("/proc/{pid}");
            let owner = fs::metadata("/proc/self").ok().map(|meta| meta.uid());
            if owner.is_some() && fs::metadata(&path).ok().map(|meta| meta.uid()) == owner {
                if let Ok(env) = fs::read(format!("{path}/environ")) {
                    let ids = env
                        .split(|byte| *byte == 0)
                        .filter_map(|entry| {
                            entry
                                .strip_prefix(b"SteamAppId=")
                                .or_else(|| entry.strip_prefix(b"SteamGameId="))
                        })
                        .filter(|id| !id.is_empty() && *id != b"0")
                        .collect::<Vec<_>>();
                    if !ids.is_empty() {
                        return ids.contains(&app_id.as_bytes());
                    }
                }
            }
        }
        let title = normalize(title);
        title.len() > 3
            && (normalize(&self.title) == title || self.names.iter().any(|name| normalize(name) == title))
    }
}

fn normalize(value: &str) -> String {
    value
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
