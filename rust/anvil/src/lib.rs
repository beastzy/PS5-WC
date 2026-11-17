#![warn(rust_2018_idioms)]
// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.
#![cfg_attr(
    not(any(feature = "winit", feature = "x11", feature = "udev")),
    allow(dead_code, unused_imports)
)]

mod apps;
#[cfg(any(feature = "udev", feature = "xwayland"))]
pub mod cursor;
pub mod drawing;
pub mod focus;
mod capture;
mod game_session;
mod gamepad;
pub mod home;
pub mod input_handler;
pub mod render;
pub mod shell;
pub mod state;
mod steam;
#[cfg(feature = "udev")]
pub mod udev;
mod window;
#[cfg(feature = "winit")]
pub mod winit;
#[cfg(feature = "x11")]
pub mod x11;

pub use state::{AnvilState, ClientState};
