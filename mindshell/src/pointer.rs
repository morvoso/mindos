//! The pointer: which XCursor theme the desktop uses and how big it is.
//!
//! GSettings (`org.gnome.desktop.interface cursor-theme` / `cursor-size`) is
//! the desktop-wide source of truth. GTK applications follow it live through
//! the settings portal; `mindos-session` reads it at login and exports
//! `XCURSOR_THEME` / `XCURSOR_SIZE` for everything that only looks at the
//! environment (SDL, Qt, XWayland); the compositor is told over IPC so its own
//! cursor changes without a restart.

use std::collections::BTreeSet;

use gtk4 as gtk;
use gtk::gio;
use gtk::prelude::*;
use serde_json::{json, Value};

const SCHEMA: &str = "org.gnome.desktop.interface";
/// The sizes the MindOS theme is drawn at; in between them XCursor scales and
/// the edge goes soft.
pub const SIZES: [u32; 4] = [24, 32, 48, 64];

/// The interface settings, or None when the schema is not installed (a bare
/// build box, or a system without gsettings-desktop-schemas): looking one up
/// that is missing aborts the process, so this never uses `Settings::new`.
fn settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    let schema = source.lookup(SCHEMA, true)?;
    (schema.has_key("cursor-theme") && schema.has_key("cursor-size"))
        .then(|| gio::Settings::new(SCHEMA))
}

/// Installed cursor themes: a theme directory with a `cursors` folder in it.
pub fn themes() -> Vec<String> {
    let mut found = BTreeSet::new();
    for dir in crate::icons::theme_dirs() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if !entry.path().join("cursors").is_dir() {
                continue;
            }
            if let Some(name) = entry.file_name().to_str() {
                found.insert(name.to_string());
            }
        }
    }
    found.into_iter().collect()
}

/// What the pointer is set to, and what it could be set to.
pub fn state() -> Value {
    let settings = settings();
    let theme = settings
        .as_ref()
        .map(|s| s.string("cursor-theme").to_string())
        .unwrap_or_else(|| std::env::var("XCURSOR_THEME").unwrap_or_else(|_| "MindOS".into()));
    let size = settings
        .as_ref()
        .map(|s| s.int("cursor-size") as u32)
        .unwrap_or_else(|| std::env::var("XCURSOR_SIZE").ok().and_then(|s| s.parse().ok()).unwrap_or(24));
    json!({
        "theme": theme,
        "size": size,
        "themes": themes(),
        "sizes": SIZES,
        // Without the schema nothing here can be saved; the page says so.
        "writable": settings.is_some(),
    })
}

/// Write the choice. GTK applications pick it up immediately; the compositor
/// is told separately (`app.rs` forwards to `set_prefs`).
pub fn set(theme: Option<&str>, size: Option<u32>) -> Result<(), String> {
    let settings = settings().ok_or("the desktop interface settings are not installed")?;
    if let Some(theme) = theme {
        if theme.trim().is_empty() {
            return Err("the cursor theme cannot be empty".into());
        }
        settings.set_string("cursor-theme", theme).map_err(|e| e.to_string())?;
    }
    if let Some(size) = size {
        if !(8..=256).contains(&size) {
            return Err("the cursor size must be between 8 and 256".into());
        }
        settings.set_int("cursor-size", size as i32).map_err(|e| e.to_string())?;
    }
    gio::Settings::sync();
    Ok(())
}
