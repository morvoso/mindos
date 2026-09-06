#![warn(rust_2018_idioms)]
// If no backend is enabled, a large portion of the codebase is unused.
// So silence this useless warning for the CI.
#![cfg_attr(
    not(any(feature = "winit", feature = "udev")),
    allow(dead_code, unused_imports)
)]

#[cfg(any(feature = "udev", feature = "xwayland"))]
pub mod cursor;
pub mod config;
pub mod drawing;
pub mod launcher;
pub mod mind;
pub mod mindbar;
pub mod text;
pub mod focus;
pub mod input_handler;
pub mod ipc;
pub mod layout;
pub mod prefs;
pub mod procinfo;
pub mod render;
pub mod shell;
pub mod state;
#[cfg(feature = "udev")]
pub mod edid;
#[cfg(feature = "udev")]
pub mod udev;
#[cfg(feature = "winit")]
pub mod winit;

pub use state::{AnvilState, ClientState};
