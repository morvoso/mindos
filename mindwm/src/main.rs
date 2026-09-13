// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! mindwm — the MindOS compositor.
//!
//! Backend selection:
//!   mindwm                 auto: nested (winit) if WAYLAND_DISPLAY/DISPLAY is set, else DRM/KMS
//!   mindwm --tty-udev      run on a TTY via DRM/KMS + libinput + libseat
//!   mindwm --winit         run nested inside another Wayland/X11 session (development)

fn main() {
    // Held to the end of `main`, so the last lines reach the journal.
    let _log = mindwm::logging::init("info,smithay=warn");

    profiling::register_thread!("Main Thread");

    // Before anything can panic, so a crash leaves a journal entry that says
    // where it was rather than a session that simply ended.
    mindwm::recover::say_where_panics_happen();

    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let mut backend = if nested { "winit" } else { "udev" };
    // A nested instance shares the user's session bus and systemd user
    // manager with the desktop it runs inside, so it starts no session of
    // its own unless asked: see `AnvilState::skip_startup`.
    let mut session_startup = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--winit" => backend = "winit",
            "--tty-udev" => backend = "udev",
            "--with-startup" => session_startup = true,
            "--help" | "-h" => {
                println!("USAGE: mindwm [--winit | --tty-udev] [--with-startup]");
                println!("  --winit          nested inside a Wayland/X11 session (development)");
                println!("  --tty-udev       real session on a TTY via DRM/KMS (default when no display is set)");
                println!("  --with-startup   run [startup].exec when nested; it takes over the");
                println!("                   session environment of the desktop you are running in");
                return;
            }
            other => {
                eprintln!("mindwm: unknown option {other}");
                std::process::exit(2);
            }
        }
    }

    // The pointer, resolved before any backend loads a cursor or the
    // compositor starts a child: what the session exported (mindos-session
    // takes it from GSettings, the desktop-wide source of truth) wins, then
    // what the Pointer settings last asked for, then the system default.
    {
        let config = mindwm::config::Config::load();
        let prefs = mindwm::prefs::Prefs::load();
        let theme = std::env::var("XCURSOR_THEME")
            .ok()
            .filter(|t| !t.trim().is_empty())
            .or(prefs.cursor_theme)
            .unwrap_or(config.theme.cursor_theme);
        let size = std::env::var("XCURSOR_SIZE")
            .ok()
            .and_then(|s| s.trim().parse().ok())
            .or(prefs.cursor_size)
            .unwrap_or(config.theme.cursor_size);
        mindwm::cursor::configure(&theme, size);
    }

    match backend {
        #[cfg(feature = "winit")]
        "winit" => {
            tracing::info!("mindwm {}: starting nested (winit) backend", env!("CARGO_PKG_VERSION"));
            mindwm::winit::run_winit(session_startup);
        }
        #[cfg(feature = "udev")]
        "udev" => {
            tracing::info!("mindwm {}: starting DRM/KMS backend", env!("CARGO_PKG_VERSION"));
            mindwm::udev::run_udev();
        }
        other => {
            eprintln!("mindwm: backend {other} not compiled in");
            std::process::exit(2);
        }
    }
}
