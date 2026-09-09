// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! mindshell: the MindOS desktop shell host. GTK4 layer-shell windows with
//! WebKitGTK views, a `window.mindos` bridge for the TypeScript UI, the
//! compositor IPC client, StatusNotifier tray, and system helpers.
//!
//! With `--app NAME` the same binary opens one ordinary window running the
//! UI's `NAME` app (Settings, Files) instead of the panels. `--app greeter`
//! is the login screen: a full-screen overlay per output, started by greetd
//! (see `mindos-greeter` in mindos-session).

mod workspace;
mod app;
mod apps;
mod auth;
mod bridge;
mod config;
mod fs;
mod greeter;
mod icons;
mod ipc;
mod layout;
mod mind;
mod media;
mod gaming;
mod mindwatch;
mod notify;
mod pointer;
mod polkit;
mod portal;
mod scheme;
mod sleepwatch;
mod system;
mod tray;
mod windows;

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};

use gtk4 as gtk;
use gtk::glib;
use serde_json::Value;

/// Everything that reaches the main loop from another thread.
pub enum HostEvent {
    Ipc(ipc::IpcEvent),
    Tray(Value),
    Apps(Vec<apps::AppEntry>),
    Audio(Option<system::Audio>),
    /// NetworkManager reported a change (`nmcli monitor`): connections, tunnels, devices.
    Network,
    Gpu(Option<Value>),
    /// The user layout file changed on disk (another shell process saved it).
    LayoutFile,
    /// Something in the Desktop folder changed (the desktop icons re-list).
    DesktopDir,
    DesktopOpen(Value),
    /// A game started or ended (the GameMode counter in /run/mindos/perf).
    Game,
    /// logind completed a sleep cycle; rebuild shell render content.
    Resumed,
    /// An application's notification (`org.freedesktop.Notifications.Notify`).
    Notify(Value),
    /// `CloseNotification(id)`; the second field is the close reason.
    NotifyClosed(u32, u32),
    /// A line from the Mind daemon subscription (`notice`, `updates`, `sleep`, ...).
    Mind(Value),
    /// The polkit agent wants the password dialog shown, updated or closed.
    Polkit(Value),
    Quit,
}

static QUIT: AtomicBool = AtomicBool::new(false);

/// The write end of the pipe a signal is reported through.
static SIGNAL_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_signal(_: libc::c_int) {
    // Only async-signal-safe calls here: a flag and one byte down the pipe.
    QUIT.store(true, Ordering::SeqCst);
    let fd = SIGNAL_PIPE.load(Ordering::SeqCst);
    if fd >= 0 {
        let byte = [0u8];
        unsafe { libc::write(fd, byte.as_ptr() as *const libc::c_void, 1) };
    }
}

/// End the main loop on systemd's stop signal (and Ctrl-C at a terminal)
/// through the event channel. The handler writes to a pipe; a thread parked
/// on its other end sends the event, so nothing polls for it.
fn watch_signals(tx: async_channel::Sender<HostEvent>) {
    let mut fds = [0 as libc::c_int; 2];
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        tracing::warn!("cannot create the signal pipe; stop signals are not handled");
        return;
    }
    let [read_end, write_end] = fds;
    SIGNAL_PIPE.store(write_end, Ordering::SeqCst);
    std::thread::Builder::new()
        .name("mindshell-signals".into())
        .spawn(move || {
            let mut byte = [0u8];
            loop {
                let n = unsafe { libc::read(read_end, byte.as_mut_ptr() as *mut libc::c_void, 1) };
                if n == 1 {
                    let _ = tx.send_blocking(HostEvent::Quit);
                } else if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                    continue;
                } else {
                    return;
                }
            }
        })
        .expect("spawn signal thread");
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGHUP, on_signal as *const () as libc::sighandler_t);
    }
}

/// True once SIGTERM/SIGINT/SIGHUP arrived (the main loop quits shortly after).
pub fn quit_requested() -> bool {
    QUIT.load(Ordering::SeqCst)
}


/// The apps `--app` accepts (each is a page set in the UI bundle).
pub const APPS: &[&str] = &["settings", "library", "gaming", "greeter"];

fn usage() {
    println!(
        "mindshell {}\n\nUsage: mindshell [--devtools] [--ui-dir DIR]\n       mindshell --app NAME [--page PAGE] [PATH]\n\n  --app NAME     open an app window instead of the shell: {}\n  --page PAGE    the page the app opens on (settings: mind, updates, performance, games, software, wallpaper, displays, screen, shell, about)\n  --devtools     enable the WebKit inspector (F12, context menu); also MINDSHELL_DEVTOOLS=1\n  --ui-dir DIR   serve the UI bundle from DIR instead of {} (also MINDSHELL_UI_DIR)\n  --version      print the version\n  --help         this text\n\nConfig: /etc/mindos/shell.toml, ~/.config/mindos/shell.toml\nLayout: /usr/share/mindos/shell/layout.json, ~/.config/mindos/shell/layout.json\nLogs:   journalctl --user -u mindos-shell (RUST_LOG=debug for more)",
        env!("CARGO_PKG_VERSION"),
        APPS.join(", "),
        app::DEFAULT_UI_DIR
    );
}

/// When the process started: the desktop's first view logs how long it took to
/// come up, which is the number to watch when the boot feels slow.
pub static STARTED: std::sync::LazyLock<std::time::Instant> = std::sync::LazyLock::new(std::time::Instant::now);

fn main() {
    std::sync::LazyLock::force(&STARTED);
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
                    // A positional argument for the app (what a desktop entry's %f passes).
                    opts.arg = Some(other.to_string());
                } else {
                    eprintln!("unknown argument '{other}'");
                    usage();
                    std::process::exit(2);
                }
            }
        }
    }
    if matches!(opts.app.as_deref(), Some("settings" | "gaming" | "library")) {
        let request = serde_json::json!({"name": opts.app, "page": opts.page, "arg": opts.arg});
        if workspace::forward(&request) { return; }
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

    // The Wallpaper portal backend, claimed before GTK says a word on the bus.
    // xdg-desktop-portal will not finish starting until this name is on the
    // session bus, and GTK's first act is to ask that same portal for the
    // colour scheme: claiming the name after GTK is up means both sides wait
    // for each other, and the desktop appears only when D-Bus gives up 25
    // seconds later.
    if opts.app.is_none() {
        portal::start();
    }

    // GTK must talk to a Wayland compositor with the layer-shell protocol.
    std::env::set_var("GDK_BACKEND", "wayland");
    if let Err(e) = gtk::init() {
        tracing::error!(%e, "cannot initialise GTK (is WAYLAND_DISPLAY set?)");
        std::process::exit(1);
    }
    let needs_layer_shell = opts.app.as_deref().map(|a| a == "greeter").unwrap_or(true);
    if needs_layer_shell && !gtk4_layer_shell::is_supported() {
        tracing::error!("the compositor does not support wlr-layer-shell; mindshell cannot run here");
        std::process::exit(1);
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
    watch_signals(tx.clone());

    main_loop.run();
    tracing::info!("bye");
}
