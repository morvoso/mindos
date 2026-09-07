//! mindshell: the MindOS desktop shell host. GTK4 layer-shell windows with
//! WebKitGTK views, a `window.mindos` bridge for the TypeScript UI, the
//! compositor IPC client, StatusNotifier tray, and system helpers.
//!
//! With `--app NAME` the same binary opens one ordinary window running the
//! UI's `NAME` app (Settings, Files) instead of the panels.

mod app;
mod apps;
mod bridge;
mod config;
mod fs;
mod icons;
mod ipc;
mod layout;
mod mind;
mod scheme;
mod system;
mod tray;
mod windows;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gtk4 as gtk;
use gtk::glib;
use serde_json::Value;

/// Everything that reaches the main loop from another thread.
pub enum HostEvent {
    Ipc(ipc::IpcEvent),
    Tray(Value),
    Apps(Vec<apps::AppEntry>),
    Audio(Option<system::Audio>),
    Gpu(Option<Value>),
    /// The user layout file changed on disk (another shell process saved it).
    LayoutFile,
    /// Something in the Desktop folder changed (the desktop icons re-list).
    DesktopDir,
    Quit,
}

static QUIT: AtomicBool = AtomicBool::new(false);

/// True once SIGTERM/SIGINT/SIGHUP arrived (the main loop quits shortly after).
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::SeqCst)
}

extern "C" fn on_signal(_: libc::c_int) {
    QUIT.store(true, Ordering::SeqCst);
}

/// The apps `--app` accepts (each is a page set in the UI bundle).
pub const APPS: &[&str] = &["settings", "files"];

fn usage() {
    println!(
        "mindshell {}\n\nUsage: mindshell [--devtools] [--ui-dir DIR]\n       mindshell --app NAME [--page PAGE] [PATH]\n\n  --app NAME     open an app window instead of the shell: {}\n  --page PAGE    the page the app opens on (settings: mind, wallpaper, displays, shell, about)\n  PATH           for the files app: the folder to open\n  --devtools     enable the WebKit inspector (F12, context menu); also MINDSHELL_DEVTOOLS=1\n  --ui-dir DIR   serve the UI bundle from DIR instead of {} (also MINDSHELL_UI_DIR)\n  --version      print the version\n  --help         this text\n\nConfig: /etc/mindos/shell.toml, ~/.config/mindos/shell.toml\nLayout: /usr/share/mindos/shell/layout.json, ~/.config/mindos/shell/layout.json\nLogs:   journalctl --user -u mindos-shell (RUST_LOG=debug for more)",
        env!("CARGO_PKG_VERSION"),
        APPS.join(", "),
        app::DEFAULT_UI_DIR
    );
}

fn main() {
    let mut opts = app::Options::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--devtools" => opts.devtools = true,
            "--ui-dir" => match args.next() {
                Some(dir) => opts.ui_dir = Some(dir.into()),
                None => {
                    eprintln!("--ui-dir needs a directory");
                    std::process::exit(2);
                }
            },
            "--app" => match args.next() {
                Some(name) => opts.app = Some(name),
                None => {
                    eprintln!("--app needs a name ({})", APPS.join(", "));
                    std::process::exit(2);
                }
            },
            "--page" => match args.next() {
                Some(page) => opts.page = Some(page),
                None => {
                    eprintln!("--page needs a page name");
                    std::process::exit(2);
                }
            },
            "--version" | "-V" => {
                println!("mindshell {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            "--help" | "-h" => {
                usage();
                return;
            }
            other => {
                if let Some(dir) = other.strip_prefix("--ui-dir=") {
                    opts.ui_dir = Some(dir.into());
                } else if let Some(name) = other.strip_prefix("--app=") {
                    opts.app = Some(name.into());
                } else if let Some(page) = other.strip_prefix("--page=") {
                    opts.page = Some(page.into());
                } else if !other.starts_with('-') && opts.app.is_some() {
                    // `mindshell --app files ~/Downloads` (also what the desktop entry's %f passes).
                    opts.arg = Some(other.to_string());
                } else {
                    eprintln!("unknown argument '{other}'");
                    usage();
                    std::process::exit(2);
                }
            }
        }
    }
    if let Some(name) = &opts.app {
        if !APPS.contains(&name.as_str()) {
            eprintln!("unknown app '{name}' (expected one of: {})", APPS.join(", "));
            std::process::exit(2);
        }
        // The Wayland app id (and so the taskbar icon and desktop entry match).
        glib::set_prgname(Some(&format!("mindos-{name}")));
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .without_time()
        .with_target(false)
        // Colour codes only when a person is watching, not in the journal.
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .init();

    // GTK must talk to a Wayland compositor with the layer-shell protocol.
    std::env::set_var("GDK_BACKEND", "wayland");
    if let Err(e) = gtk::init() {
        tracing::error!(%e, "cannot initialise GTK (is WAYLAND_DISPLAY set?)");
        std::process::exit(1);
    }
    if opts.app.is_none() && !gtk4_layer_shell::is_supported() {
        tracing::error!("the compositor does not support wlr-layer-shell; mindshell cannot run here");
        std::process::exit(1);
    }

    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
    }

    let main_loop = glib::MainLoop::new(None, false);
    let (tx, rx) = async_channel::unbounded::<HostEvent>();
    let app = app::App::new(opts, tx.clone(), main_loop.clone());
    app.sync_windows();
    app.start_background();

    {
        let app = app.clone();
        glib::spawn_future_local(async move {
            while let Ok(event) = rx.recv().await {
                app.handle_event(event);
            }
        });
    }
    glib::timeout_add_local(Duration::from_millis(200), move || {
        if QUIT.load(Ordering::SeqCst) {
            let _ = tx.send_blocking(HostEvent::Quit);
            return glib::ControlFlow::Break;
        }
        glib::ControlFlow::Continue
    });

    main_loop.run();
    tracing::info!("bye");
}
