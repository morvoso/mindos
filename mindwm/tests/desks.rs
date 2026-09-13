// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! Desks with real windows: a Wayland client opens toplevels on a headless
//! mindwm, and the desks are switched under them.

use std::io::Write;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use mindwm::{
    config::Config,
    headless::{Headless, HeadlessData},
    layout::LayoutMode,
    AnvilState,
};
use smithay::output::Mode;
use wayland_client::{
    delegate_noop,
    globals::{registry_queue_init, GlobalListContents},
    protocol::{wl_buffer, wl_compositor, wl_registry, wl_shm, wl_shm_pool, wl_surface},
    Connection, Dispatch, QueueHandle,
};
use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

struct Win {
    surface: wl_surface::WlSurface,
    buffer: wl_buffer::WlBuffer,
    _toplevel: xdg_toplevel::XdgToplevel,
}

#[derive(Default)]
struct Client {
    windows: Vec<Win>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Client {
    fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Client {
    fn event(_: &mut Self, wm: &xdg_wm_base::XdgWmBase, event: xdg_wm_base::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_wm_base::Event::Ping { serial } = event {
            wm.pong(serial);
        }
    }
}

impl Dispatch<xdg_surface::XdgSurface, usize> for Client {
    fn event(state: &mut Self, surface: &xdg_surface::XdgSurface, event: xdg_surface::Event, index: &usize, _: &Connection, _: &QueueHandle<Self>) {
        if let xdg_surface::Event::Configure { serial } = event {
            surface.ack_configure(serial);
            let win = &state.windows[*index];
            win.surface.attach(Some(&win.buffer), 0, 0);
            win.surface.commit();
        }
    }
}

delegate_noop!(Client: ignore wl_compositor::WlCompositor);
delegate_noop!(Client: ignore wl_surface::WlSurface);
delegate_noop!(Client: ignore wl_shm::WlShm);
delegate_noop!(Client: ignore wl_shm_pool::WlShmPool);
delegate_noop!(Client: ignore wl_buffer::WlBuffer);
delegate_noop!(Client: ignore xdg_toplevel::XdgToplevel);

/// A client on its own thread that opens a window for every app id sent to it.
fn spawn_client(socket: &str) -> (mpsc::Sender<String>, Arc<AtomicBool>) {
    std::env::set_var("WAYLAND_DISPLAY", socket);
    let (tx, rx) = mpsc::channel::<String>();
    let stop = Arc::new(AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        let conn = Connection::connect_to_env().expect("connect to the headless compositor");
        let (globals, mut queue) = registry_queue_init::<Client>(&conn).unwrap();
        let qh = queue.handle();
        let compositor: wl_compositor::WlCompositor = globals.bind(&qh, 4..=6, ()).unwrap();
        let shm: wl_shm::WlShm = globals.bind(&qh, 1..=1, ()).unwrap();
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=6, ()).unwrap();
        let mut client = Client::default();
        while !stopped.load(Ordering::SeqCst) {
            while let Ok(app_id) = rx.try_recv() {
                let (w, h) = (320, 200);
                let mut file = tempfile::tempfile().unwrap();
                file.write_all(&vec![0x80u8; (w * h * 4) as usize]).unwrap();
                let pool = shm.create_pool(file.as_fd(), w * h * 4, &qh, ());
                let buffer = pool.create_buffer(0, w, h, w * 4, wl_shm::Format::Argb8888, &qh, ());
                let surface = compositor.create_surface(&qh, ());
                let index = client.windows.len();
                let xdg = wm.get_xdg_surface(&surface, &qh, index);
                let toplevel = xdg.get_toplevel(&qh, ());
                toplevel.set_app_id(app_id);
                surface.commit();
                client.windows.push(Win { surface, buffer, _toplevel: toplevel });
            }
            if queue.roundtrip(&mut client).is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    });
    (tx, stop)
}

fn settle(h: &mut Headless, what: &str, until: impl Fn(&AnvilState<HeadlessData>) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !until(&h.state) {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        h.turn_waiting(Duration::from_millis(5));
    }
    // A few more, for what follows on from it.
    for _ in 0..5 {
        h.turn_waiting(Duration::from_millis(2));
    }
}

fn window(state: &AnvilState<HeadlessData>, app_id: &str) -> mindwm::shell::WindowElement {
    let id = state
        .windows_snapshot()
        .windows
        .iter()
        .find(|w| w.app_id == app_id)
        .unwrap_or_else(|| panic!("no {app_id} window"))
        .id;
    state.window_by_id(id).unwrap()
}

fn shown(state: &AnvilState<HeadlessData>) -> Vec<String> {
    let mut ids: Vec<String> = state.space.elements().map(|w| w.app_id()).collect();
    ids.sort();
    ids
}

fn listed(state: &AnvilState<HeadlessData>, app_id: &str) -> mindwm::ipc::WindowInfo {
    state.windows_snapshot().windows.into_iter().find(|w| w.app_id == app_id).unwrap()
}

#[test]
fn windows_live_on_their_desks() {
    let home = tempfile::tempdir().unwrap();
    for var in ["XDG_RUNTIME_DIR", "XDG_STATE_HOME", "XDG_CONFIG_HOME", "HOME"] {
        std::env::set_var(var, home.path());
    }
    let mut h = Headless::new(Config::default()).unwrap();
    h.state.add_output("HEADLESS-1", &[Mode { size: (1920, 1080).into(), refresh: 60_000 }]);
    let socket = h.state.socket_name.clone().unwrap();
    let (open, stop) = spawn_client(&socket);

    let drawn = |app: &'static str| move |s: &AnvilState<HeadlessData>| s.space.elements().any(|w| w.app_id() == app && w.0.geometry().size.w > 0);
    open.send("game".into()).unwrap();
    open.send("chat".into()).unwrap();
    settle(&mut h, "game", drawn("game"));
    settle(&mut h, "chat", drawn("chat"));
    // Before the shell chooses, windows have no desk and everything shows.
    assert_eq!(listed(&h.state, "game").desk, "");

    h.state.set_desk("gaming", None, None);
    assert_eq!(h.state.window_desk(&window(&h.state, "game")), "gaming");
    assert_eq!(h.state.window_desk(&window(&h.state, "chat")), "gaming");

    // Chat is on every desk; the game stays behind.
    h.state.set_desk("work", Some(vec!["Chat".into()]), Some(LayoutMode::Columns));
    settle(&mut h, "the switch", |_| true);
    assert_eq!(shown(&h.state), ["chat"]);
    assert_eq!(h.state.layout.mode, LayoutMode::Columns);
    let game = listed(&h.state, "game");
    assert!(game.away && !game.minimized && !game.sticky && game.desk == "gaming");
    let chat = listed(&h.state, "chat");
    // Put on the list during the switch, it still belongs where it was.
    assert!(!chat.away && chat.sticky && chat.desk == "gaming");

    // A new window opens on the desk it was opened on.
    open.send("editor".into()).unwrap();
    settle(&mut h, "editor", drawn("editor"));
    assert_eq!(listed(&h.state, "editor").desk, "work");

    // Minimised on work, it stays minimised when work comes back.
    let editor = window(&h.state, "editor");
    h.state.minimize_window(&editor);
    h.state.set_desk("gaming", None, Some(LayoutMode::Floating));
    settle(&mut h, "back to gaming", |_| true);
    assert_eq!(shown(&h.state), ["chat", "game"]);
    let listed_editor = listed(&h.state, "editor");
    assert!(listed_editor.minimized && listed_editor.away);
    h.state.set_desk("work", None, None);
    settle(&mut h, "work again", |_| true);
    assert_eq!(shown(&h.state), ["chat"]);
    assert!(listed(&h.state, "editor").minimized);

    // Reaching for the game from work goes to the gaming desk.
    let game = window(&h.state, "game");
    h.state.show_desk_of(&game);
    h.state.activate_window(&game);
    settle(&mut h, "the game", |_| true);
    assert_eq!(h.state.desks.current, "gaming");
    assert_eq!(shown(&h.state), ["chat", "game"]);
    assert_eq!(h.state.focused_window().map(|w| w.app_id()), Some("game".into()));

    // Unminimising the editor from gaming takes the user to work.
    let editor = window(&h.state, "editor");
    h.state.unminimize_window(&editor);
    settle(&mut h, "the editor", |_| true);
    assert_eq!(h.state.desks.current, "work");
    assert_eq!(shown(&h.state), ["chat", "editor"]);
    assert_eq!(h.state.focused_window().map(|w| w.app_id()), Some("editor".into()));

    // Given to gaming, the editor leaves work; taken off the sticky list,
    // chat stays where the user is.
    h.state.move_window_to_desk(&editor, "gaming");
    h.state.set_desk("work", Some(vec![]), None);
    settle(&mut h, "moves", |_| true);
    assert_eq!(shown(&h.state), ["chat"]);
    h.state.set_desk("gaming", None, None);
    settle(&mut h, "gaming", |_| true);
    assert_eq!(shown(&h.state), ["editor", "game"]);
    assert!(listed(&h.state, "chat").away);

    stop.store(true, Ordering::SeqCst);
}
