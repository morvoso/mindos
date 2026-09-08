// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

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
pub mod capture;
pub mod drawing;
pub mod launcher;
pub mod markdown;
pub mod mind;
pub mod mindbar;
pub mod text;
pub mod timing;
mod pointer;
pub mod focus;
pub mod idle;
pub mod input_handler;
pub mod input_config;
mod window_cycle;
mod media;
mod osd;
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
#[cfg(feature = "xwayland")]
pub mod xtray;

pub use state::{AnvilState, ClientState};
