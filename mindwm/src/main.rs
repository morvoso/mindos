//! mindwm — the MindOS compositor.
//!
//! Backend selection:
//!   mindwm                 auto: nested (winit) if WAYLAND_DISPLAY/DISPLAY is set, else DRM/KMS
//!   mindwm --tty-udev      run on a TTY via DRM/KMS + libinput + libseat
//!   mindwm --winit         run nested inside another Wayland/X11 session (development)

fn main() {
    if let Ok(env_filter) = tracing_subscriber::EnvFilter::try_from_default_env() {
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter(env_filter)
            .init();
    } else {
        tracing_subscriber::fmt()
            .compact()
            .with_env_filter("info,smithay=warn")
            .init();
    }

    profiling::register_thread!("Main Thread");

    let arg = std::env::args().nth(1);
    let nested = std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some();
    let backend = match arg.as_deref() {
        Some("--winit") => "winit",
        Some("--tty-udev") => "udev",
        Some("--help") | Some("-h") => {
            println!("USAGE: mindwm [--winit | --tty-udev]");
            println!("  --winit     nested inside a Wayland/X11 session (development)");
            println!("  --tty-udev  real session on a TTY via DRM/KMS (default when no display is set)");
            return;
        }
        Some(other) => {
            eprintln!("mindwm: unknown option {other}");
            std::process::exit(2);
        }
        None if nested => "winit",
        None => "udev",
    };

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
            mindwm::winit::run_winit();
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
