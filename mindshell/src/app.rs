//! The shell host: state, windows, bridge dispatch and the event loop.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gtk4 as gtk;
use gtk::gdk;
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use serde_json::{json, Value};
use webkit6 as webkit;
use webkit::prelude::*;

use crate::apps::{self, AppEntry};
use crate::bridge;
use crate::config::Config;
use crate::fs;
use crate::greeter;
use crate::icons;
use crate::ipc::{IpcClient, IpcEvent};
use crate::layout::{Layout, Panel};
use crate::mind;
use crate::notify::NotifyHandle;
use crate::polkit::PolkitHandle;
use crate::system::{self, Audio};
use crate::pointer;
use crate::tray::TrayHandle;
use crate::windows::{self, Kind, PanelSpec, ShellWindow};
use crate::HostEvent;

pub const DEFAULT_UI_DIR: &str = "/usr/share/mindos/shell/ui";
/// Besides `greeter.*`, all the login screen may ask the host for.
const GREETER_ALLOWED: &[&str] = &["shell.state", "shell.ready", "shell.reload", "app.setTitle"];
/// How long a closed window keeps its view alive for late bridge requests.
const RETIRE_GRACE: Duration = Duration::from_millis(1500);

#[derive(Debug, Default, Clone)]
pub struct Options {
    pub devtools: bool,
    pub ui_dir: Option<PathBuf>,
    /// `--app NAME`: one ordinary window running that UI app instead of the shell.
    pub app: Option<String>,
    pub page: Option<String>,
    /// Positional argument for the app (the folder for `files`).
    pub arg: Option<String>,
}

/// What `--app` asked for.
#[derive(Debug, Clone)]
pub struct AppMode {
    pub name: String,
    pub page: String,
    pub arg: String,
}

fn app_title(name: &str) -> String {
    match name {
        "settings" => "Settings".into(),
        "files" => "Files".into(),
        other => {
            let mut c = other.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        }
    }
}

#[derive(Default)]
struct State {
    layout: Layout,
    edit_mode: bool,
    windows: Value,
    focused: Value,
    ipc_outputs: Vec<Value>,
    apps: Vec<AppEntry>,
    /// StatusNotifier items from the D-Bus tray host.
    tray: Value,
    /// XEmbed icons the compositor hosts, already in the shell's item shape
    /// (`Null` until the first `tray` event).
    xtray: Value,
    audio: Option<Audio>,
    mind_open: bool,
    mind_extra: Value,
    gpu: Option<Value>,
    last_outputs: Option<String>,
    /// Applications' notifications, newest last (the notification server).
    notifications: Vec<Value>,
    /// Do not disturb: notifications still collect, only critical ones toast.
    dnd: bool,
    /// The Mind daemon subscription: connected, sleeping, its notices,
    /// update status and last health report.
    mind_daemon: bool,
    mind_sleeping: bool,
    mind_notices: Vec<Value>,
    mind_updates: Value,
    mind_health: Value,
    /// The authorisation the polkit agent is waiting for (`Null` when none).
    polkit: Value,
}

/// How many application notifications the centre keeps.
const NOTIFICATION_LIMIT: usize = 60;

pub struct App {
    pub config: Config,
    pub icon_theme: String,
    pub ui_dir: PathBuf,
    pub devtools: bool,
    pub web_context: webkit::WebContext,
    pub network_session: webkit::NetworkSession,
    pub settings: webkit::Settings,
    pub ipc: IpcClient,
    /// The StatusNotifier host; app windows do not run one.
    pub tray: Option<TrayHandle>,
    /// The notification server (the shell only).
    pub notify: Option<NotifyHandle>,
    /// The polkit authentication agent (the shell only).
    pub polkit: Option<PolkitHandle>,
    pub app_mode: Option<AppMode>,
    pub events: async_channel::Sender<HostEvent>,
    /// `--ui-dir` as given, handed to app windows this shell spawns.
    ui_dir_override: Option<PathBuf>,
    main_loop: glib::MainLoop,
    started: Instant,
    state: RefCell<State>,
    windows: RefCell<Vec<Rc<ShellWindow>>>,
    /// Closed windows kept hidden for a moment: a popup that closes itself and
    /// then runs its menu action sends that request after `popup.close`.
    retired: RefCell<Vec<Rc<ShellWindow>>>,
    first_view: RefCell<Option<webkit::WebView>>,
    stats: RefCell<system::Stats>,
    sync_pending: Cell<bool>,
    layout_monitor: RefCell<Option<gio::FileMonitor>>,
    desktop_monitor: RefCell<Option<gio::FileMonitor>>,
    desktop_notify_pending: Cell<bool>,
    layout_reload_pending: Cell<bool>,
    /// The login conversation in progress (`--app greeter` only).
    greeter: RefCell<Option<greeter::Login>>,
}

impl App {
    pub fn new(opts: Options, events: async_channel::Sender<HostEvent>, main_loop: glib::MainLoop) -> Rc<App> {
        let config = Config::load();
        let icon_theme = icons::pick_theme(&config.shell.icon_theme);
        let ui_dir_override = opts.ui_dir.clone();
        let ui_dir = opts
            .ui_dir
            .or_else(|| std::env::var_os("MINDSHELL_UI_DIR").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from(DEFAULT_UI_DIR));
        let devtools = opts.devtools || std::env::var("MINDSHELL_DEVTOOLS").map(|v| v == "1").unwrap_or(false);
        if !ui_dir.join("index.html").is_file() {
            tracing::warn!(dir = %ui_dir.display(), "UI bundle not found; windows will be empty");
        }
        let app_mode = opts.app.map(|name| AppMode {
    /// The pending re-read of the network state (NetworkManager's changes come in bursts).
    network_refresh: RefCell<Option<glib::SourceId>>,
            name,
            page: opts.page.unwrap_or_default(),
            arg: opts.arg.unwrap_or_default(),
        });
        tracing::info!(ui = %ui_dir.display(), icon_theme, devtools, hw = config.hardware_acceleration(), app = ?app_mode.as_ref().map(|m| &m.name), "mindshell starting");

        let web_context = webkit::WebContext::new();
        web_context.set_cache_model(webkit::CacheModel::DocumentViewer);
        let network_session = if app_mode.as_ref().map(|m| m.name == "greeter").unwrap_or(false) {
            // The login screen keeps nothing on disk.
            webkit::NetworkSession::new_ephemeral()
        } else {
            let data_dir = dirs_data().join("mindos/shell");
            let cache_dir = dirs_cache().join("mindos/shell");
            let _ = std::fs::create_dir_all(&data_dir);
            let _ = std::fs::create_dir_all(&cache_dir);
            webkit::NetworkSession::new(Some(&data_dir.to_string_lossy()), Some(&cache_dir.to_string_lossy()))
        };

        let settings = webkit::Settings::new();
        settings.set_enable_developer_extras(devtools);
        settings.set_hardware_acceleration_policy(if config.hardware_acceleration() {
            webkit::HardwareAccelerationPolicy::Always
        } else {
            webkit::HardwareAccelerationPolicy::Never
        });
        settings.set_enable_webgl(true);
        settings.set_enable_smooth_scrolling(true);
        settings.set_enable_back_forward_navigation_gestures(false);
        settings.set_enable_page_cache(false);
        settings.set_javascript_can_access_clipboard(true);
        settings.set_enable_write_console_messages_to_stdout(true);
        settings.set_user_agent(Some(&format!("mindshell/{}", env!("CARGO_PKG_VERSION"))));

        let ipc = IpcClient::start(events.clone());
        let tray = if app_mode.is_some() { None } else { Some(TrayHandle::start(events.clone(), icon_theme.clone())) };
        let notify = if app_mode.is_some() { None } else { Some(NotifyHandle::start(events.clone())) };
        let polkit = if app_mode.is_some() { None } else { Some(PolkitHandle::start(events.clone())) };
        if app_mode.is_none() {
            crate::portal::start();
        }
        let layout = Layout::load();

        let app = Rc::new(App {
            config,
            icon_theme,
            ui_dir,
            devtools,
            web_context,
            network_session,
            settings,
            ipc,
            tray,
            notify,
            polkit,
            app_mode,
            events,
            ui_dir_override,
            main_loop,
            started: Instant::now(),
            state: RefCell::new(State { layout, ..Default::default() }),
            windows: RefCell::new(Vec::new()),
            retired: RefCell::new(Vec::new()),
            first_view: RefCell::new(None),
            stats: RefCell::new(system::Stats::default()),
            sync_pending: Cell::new(false),
            layout_monitor: RefCell::new(None),
            desktop_monitor: RefCell::new(None),
            desktop_notify_pending: Cell::new(false),
            layout_reload_pending: Cell::new(false),
            greeter: RefCell::new(None),
        });
        crate::scheme::register(&app.web_context, Rc::downgrade(&app));
        windows::install_css();
        app.watch_layout();
        app.watch_desktop();
        if let Some(display) = gdk::Display::default() {
            let weak = Rc::downgrade(&app);
            display.monitors().connect_items_changed(move |_, _, _, _| {
                if let Some(app) = weak.upgrade() {
                    app.schedule_sync();
                }
            });
        }
        app
    }

    // ---------------------------------------------------------------- windows

    /// Re-create windows after a monitor change, debounced (a new monitor's
    /// connector name can arrive a moment after the monitor itself).
    pub fn schedule_sync(self: &Rc<Self>) {
        if self.sync_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
            network_refresh: RefCell::new(None),
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            if let Some(app) = weak.upgrade() {
                app.sync_pending.set(false);
                app.sync_windows();
            }
        });
    }

    fn panels_for<'a>(layout: &'a Layout, name: &'a str, primary: bool) -> impl Iterator<Item = &'a Panel> + 'a {
        layout.panels.iter().filter(move |p| {
            p.output == "*" || p.output == name || (primary && (p.output == "primary" || p.output.is_empty()))
        })
    }

    /// Reload the layout when another process saves it (the Settings app
    /// changes the wallpaper, or the shell saves while an app is open).
    fn watch_layout(self: &Rc<Self>) {
        let path = Layout::user_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = gio::File::for_path(&path);
        match file.monitor_file(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let events = self.events.clone();
                monitor.connect_changed(move |_, _, _, event| {
                    use gio::FileMonitorEvent as E;
                    if matches!(event, E::ChangesDoneHint | E::Changed | E::Created | E::Deleted | E::Renamed) {
                        let _ = events.send_blocking(HostEvent::LayoutFile);
                    }
                });
                *self.layout_monitor.borrow_mut() = Some(monitor);
            }
            Err(e) => tracing::warn!(path = %path.display(), %e, "cannot watch the layout file"),
        }
    }

    /// Tell the desktop views when the Desktop folder changes so the icons
    /// follow (a download landing there, a file renamed in the file manager).
    fn watch_desktop(self: &Rc<Self>) {
        let path = fs::desktop_dir();
        let dir = gio::File::for_path(&path);
        match dir.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE) {
            Ok(monitor) => {
                let events = self.events.clone();
                monitor.connect_changed(move |_, _, _, event| {
                    use gio::FileMonitorEvent as E;
                    if matches!(event, E::ChangesDoneHint | E::Created | E::Deleted | E::Renamed | E::MovedIn | E::MovedOut | E::AttributeChanged) {
                        let _ = events.send_blocking(HostEvent::DesktopDir);
                    }
                });
                *self.desktop_monitor.borrow_mut() = Some(monitor);
            }
            Err(e) => tracing::warn!(path = %path.display(), %e, "cannot watch the Desktop folder"),
        }
    }

    fn desktop_dir_changed(self: &Rc<Self>) {
        if self.desktop_notify_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(250), move || {
            let Some(app) = weak.upgrade() else { return };
            app.desktop_notify_pending.set(false);
            app.broadcast("desktop.changed", &json!({ "path": fs::desktop_dir().to_string_lossy() }));
        });
    }

    fn layout_file_changed(self: &Rc<Self>) {
        if self.layout_reload_pending.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::timeout_add_local_once(Duration::from_millis(300), move || {
            let Some(app) = weak.upgrade() else { return };
            app.layout_reload_pending.set(false);
            let layout = Layout::load();
            if app.state.borrow().layout != layout {
                tracing::info!("layout changed on disk, reloading");
                app.set_layout(layout);
            }
        });
    }

    /// `--app greeter`?
    pub fn is_greeter(&self) -> bool {
        self.app_mode.as_ref().map(|m| m.name == "greeter").unwrap_or(false)
    }

    /// The login screen: a full-screen overlay on every output. The first
    /// one carries the login card and the keyboard, the rest the wallpaper.
    fn sync_greeter_windows(self: &Rc<Self>) {
        let monitors = windows::monitors();
        let names: Vec<(String, gdk::Monitor)> = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| (windows::monitor_name(m, i), m.clone()))
            .collect();
        let gone: Vec<Rc<ShellWindow>> = self
            .windows
            .borrow()
            .iter()
            .filter(|w| !names.iter().any(|(n, m)| *n == w.output && *m == w.monitor))
            .cloned()
            .collect();
        for w in gone {
            self.remove_window(&w);
        }
        let has_primary = self.windows.borrow().iter().any(|w| w.keyboard.get());
        for (i, (name, monitor)) in names.iter().enumerate() {
            if self.find_window(Kind::Greeter, "greeter", name).is_some() {
                continue;
            }
            let primary = !has_primary && i == 0;
            let arg = json!({ "primary": primary });
            let url = bridge::window_url("greeter", "greeter", name, None, Some(&arg), None);
            self.create_window(Kind::Greeter, "greeter", name, monitor, None, primary, &url);
        }
        if self.windows.borrow().is_empty() {
            tracing::error!("no monitor to show the login screen on");
            self.main_loop.quit();
        }
    }

    /// `--app`: the one window this process shows.
    fn sync_app_window(self: &Rc<Self>, mode: &AppMode) {
        if mode.name == "greeter" {
            self.sync_greeter_windows();
            return;
        }
        if self.windows.borrow().iter().any(|w| w.kind == Kind::App) {
            return;
        }
        let monitors = windows::monitors();
        let Some(monitor) = monitors.first().cloned() else {
            tracing::error!("no monitor to open the window on");
            self.main_loop.quit();
            return;
        };
        let output = windows::monitor_name(&monitor, 0);
        let arg = json!({ "page": mode.page, "arg": mode.arg });
        let url = bridge::window_url("app", &mode.name, &output, None, Some(&arg), None);
        let win = self.create_window(Kind::App, &mode.name, &output, &monitor, None, true, &url);
        win.window.set_title(Some(&app_title(&mode.name)));
    }

    /// Make the set of windows match the monitors and the layout.
    pub fn sync_windows(self: &Rc<Self>) {
        if let Some(mode) = self.app_mode.clone() {
            self.sync_app_window(&mode);
            return;
        }
        let monitors = windows::monitors();
        let names: Vec<(String, gdk::Monitor)> = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| (windows::monitor_name(m, i), m.clone()))
            .collect();
        let edit_mode = self.state.borrow().edit_mode;
        let layout = self.state.borrow().layout.clone();

        // Drop windows whose monitor went away.
        let gone: Vec<Rc<ShellWindow>> = self
            .windows
            .borrow()
            .iter()
            .filter(|w| !names.iter().any(|(n, m)| *n == w.output && *m == w.monitor))
            .cloned()
            .collect();
        for w in gone {
            tracing::info!(kind = w.kind.as_str(), id = w.id, output = w.output, "output gone, closing window");
            self.remove_window(&w);
        }

        for (i, (name, monitor)) in names.iter().enumerate() {
            if self.find_window(Kind::Desktop, "desktop", name).is_none() {
                let url = bridge::window_url("desktop", "desktop", name, None, None, None);
                self.create_window(Kind::Desktop, "desktop", name, monitor, None, false, &url);
            }
            if i == 0 && self.find_window(Kind::Toast, "toast", name).is_none() {
                let url = bridge::window_url("toast", "toast", name, None, None, None);
                let w = self.create_window(Kind::Toast, "toast", name, monitor, None, false, &url);
                w.window.set_visible(false);
            }
            // Horizontal panels first: their exclusive zones inset the vertical ones.
            let mut wanted: Vec<&Panel> = Self::panels_for(&layout, name, i == 0).collect();
            wanted.sort_by_key(|p| p.edge == "left" || p.edge == "right");
            for panel in &wanted {
                let mut spec = PanelSpec::from(*panel);
                if spec.edge == "left" || spec.edge == "right" {
                    for other in wanted.iter().filter(|o| o.id != panel.id && o.edge != "left" && o.edge != "right") {
                        let zone = other.size + other.margin.max(0);
                        if other.edge == "top" {
                            spec.inset_start = spec.inset_start.max(zone);
                        } else {
                            spec.inset_end = spec.inset_end.max(zone);
                        }
                    }
                }
                match self.find_window(Kind::Panel, &panel.id, name) {
                    Some(w) => {
                        // Keep the length the UI measured for fit-to-content panels.
                        spec.fit = w.panel.borrow().as_ref().map(|s| s.fit).unwrap_or(0);
                        if w.panel.borrow().as_ref() != Some(&spec) {
                            *w.panel.borrow_mut() = Some(spec);
                            w.apply_geometry(edit_mode);
                        }
                    }
                    None => {
                        let url = bridge::window_url("panel", &panel.id, name, None, None, None);
                        self.create_window(Kind::Panel, &panel.id, name, monitor, Some(spec), false, &url);
                    }
                }
            }
            let stale: Vec<Rc<ShellWindow>> = self
                .windows
                .borrow()
                .iter()
                .filter(|w| {
                    (w.kind == Kind::Panel && w.output == *name && !wanted.iter().any(|p| p.id == w.id))
                        || (w.kind == Kind::Toast && w.output == *name && i != 0)
                })
                .cloned()
                .collect();
            for w in stale {
                self.remove_window(&w);
            }
        }
        let outputs = self.outputs_json();
        let text = outputs.to_string();
        let changed = self.state.borrow().last_outputs.as_deref() != Some(text.as_str());
        if changed {
            self.state.borrow_mut().last_outputs = Some(text);
            self.broadcast("outputs", &json!({ "outputs": outputs }));
        }
    }

    fn find_window(&self, kind: Kind, id: &str, output: &str) -> Option<Rc<ShellWindow>> {
        self.windows
            .borrow()
            .iter()
            .find(|w| w.kind == kind && w.id == id && w.output == output)
            .cloned()
    }

    fn find_popup(&self, name: &str) -> Option<Rc<ShellWindow>> {
        self.windows.borrow().iter().find(|w| w.kind == Kind::Popup && w.id == name).cloned()
    }

    /// Take a window off the screen. The view is destroyed after a grace
    /// period so requests it sends while closing (a context menu runs its
    /// action after `popup.close`) still reach the host and get their reply.
    fn remove_window(self: &Rc<Self>, w: &Rc<ShellWindow>) {
        self.windows.borrow_mut().retain(|x| !Rc::ptr_eq(x, w));
        // Keep the shared web process alive through another view when the first one goes.
        let is_first = self
            .first_view
            .borrow()
            .as_ref()
            .map(|v| *v == w.view)
            .unwrap_or(false);
        if is_first {
            let next = self.windows.borrow().first().map(|x| x.view.clone());
            *self.first_view.borrow_mut() = next;
        }
        w.hide();
        self.retired.borrow_mut().push(w.clone());
        tracing::debug!(kind = w.kind.as_str(), id = w.id, output = w.output, "window retired");
        let weak = Rc::downgrade(self);
        let w = w.clone();
        glib::timeout_add_local_once(RETIRE_GRACE, move || {
            if let Some(app) = weak.upgrade() {
                app.retired.borrow_mut().retain(|x| !Rc::ptr_eq(x, &w));
            }
            w.destroy();
        });
    }

    fn window_for_manager(&self, ucm: &webkit::UserContentManager) -> Option<Rc<ShellWindow>> {
        let live = self
            .windows
            .borrow()
            .iter()
            .find(|w| w.view.user_content_manager().as_ref() == Some(ucm))
            .cloned();
        live.or_else(|| {
            self.retired
                .borrow()
                .iter()
                .find(|w| w.view.user_content_manager().as_ref() == Some(ucm))
                .cloned()
        })
    }

    fn create_window(
        self: &Rc<Self>,
        kind: Kind,
        id: &str,
        output: &str,
        monitor: &gdk::Monitor,
        panel: Option<PanelSpec>,
        keyboard: bool,
        url: &str,
    ) -> Rc<ShellWindow> {
        let ucm = webkit::UserContentManager::new();
        let script = webkit::UserScript::new(
            bridge::BOOTSTRAP,
            webkit::UserContentInjectedFrames::AllFrames,
            webkit::UserScriptInjectionTime::Start,
            &[],
            &[],
        );
        ucm.add_script(&script);
        ucm.register_script_message_handler("mindos", None);
        {
            let weak = Rc::downgrade(self);
            ucm.connect_script_message_received(Some("mindos"), move |ucm, value| {
                let Some(app) = weak.upgrade() else { return };
                let text = value.to_str().to_string();
                if let Some(win) = app.window_for_manager(ucm) {
                    app.on_message(win, text);
                } else {
                    tracing::warn!("bridge message from an unknown view");
                }
            });
        }
        let builder = webkit::WebView::builder()
            .settings(&self.settings)
            .user_content_manager(&ucm);
        let first = self.first_view.borrow().clone();
        let view = match first {
            Some(related) => builder.related_view(&related).build(),
            None => builder
                .web_context(&self.web_context)
                .network_session(&self.network_session)
                .build(),
        };
        if self.first_view.borrow().is_none() {
            *self.first_view.borrow_mut() = Some(view.clone());
        }
        view.set_background_color(&gdk::RGBA::new(0.0, 0.0, 0.0, 0.0));
        view.set_vexpand(true);
        view.set_hexpand(true);
        let devtools = self.devtools;
        view.connect_context_menu(move |_, _, _| !devtools);
        view.connect_web_process_terminated(|view, reason| {
            if crate::quit_requested() {
                return; // systemd stopped the whole cgroup; we are on our way out too
            }
            tracing::error!(?reason, "web process terminated; reloading the view");
            let view = view.clone();
            glib::timeout_add_local_once(Duration::from_millis(500), move || {
                if !crate::quit_requested() {
                    view.reload();
                }
            });
        });
        let greeter_view = kind == Kind::Greeter;
        view.connect_decide_policy(move |_, decision, kind| {
            if kind != webkit::PolicyDecisionType::NavigationAction {
                return false;
            }
            let Some(nav) = decision.downcast_ref::<webkit::NavigationPolicyDecision>() else { return false };
            let uri = nav
                .navigation_action()
                .and_then(|a| a.request())
                .and_then(|r| r.uri())
                .map(|u| u.to_string())
                .unwrap_or_default();
            if uri.starts_with("mindos://") || uri.starts_with("about:") || uri.is_empty() {
                return false;
            }
            decision.ignore();
            if greeter_view {
                // The login screen opens nothing.
                return true;
            }
            tracing::info!(uri, "opening external link");
            let _ = apps::spawn_detached(&format!("xdg-open {}", shell_quote(&uri)), &[]);
            true
        });

        let window = gtk::Window::new();
        let sw = ShellWindow::new(kind, id, output, monitor, window, view);
        *sw.panel.borrow_mut() = panel;
        sw.keyboard.set(keyboard);
        sw.apply_geometry(self.state.borrow().edit_mode);
        if kind == Kind::Greeter {
            // The void, so the hand-over from the splash never flashes.
            sw.view.set_background_color(&gdk::RGBA::new(0.0196, 0.0275, 0.0392, 1.0));
        }
        if kind == Kind::App {
            // Opaque page background (the theme's --bg-0) so nothing shows through while loading.
            sw.view.set_background_color(&gdk::RGBA::new(0.039, 0.051, 0.071, 1.0));
            let events = self.events.clone();
            sw.window.connect_close_request(move |_| {
                let _ = events.send_blocking(HostEvent::Quit);
                glib::Propagation::Proceed
            });
        }

        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        {
            let weak = Rc::downgrade(self);
            let win = Rc::downgrade(&sw);
            keys.connect_key_pressed(move |_, key, _, _| {
                let (Some(app), Some(win)) = (weak.upgrade(), win.upgrade()) else {
                    return glib::Propagation::Proceed;
                };
                if key == gdk::Key::Escape && win.kind == Kind::Popup {
                    app.close_popup(&win.id);
                    return glib::Propagation::Stop;
                }
                if key == gdk::Key::F12 && app.devtools {
                    if let Some(inspector) = win.view.inspector() {
                        inspector.show();
                    }
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            });
        }
        sw.window.add_controller(keys);

        sw.load(url);
        sw.window.present();
        tracing::debug!(kind = kind.as_str(), id, output, "window created");
        self.windows.borrow_mut().push(sw.clone());
        sw
    }

    // ----------------------------------------------------------------- bridge

    fn on_message(self: &Rc<Self>, win: Rc<ShellWindow>, text: String) {
        let req = match bridge::parse_request(&text) {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(%e, text = text.chars().take(200).collect::<String>(), "dropping bridge message");
                return;
            }
        };
        let app = self.clone();
        glib::spawn_future_local(async move {
            let result = app.dispatch(&win, &req.method, req.params).await;
            if let Some(id) = req.id {
                let (ok, payload) = match result {
                    Ok(v) => (true, v),
                    Err(e) => {
                        tracing::debug!(method = req.method, error = %e, "bridge call failed");
                        (false, Value::String(e))
                    }
                };
                win.eval(&bridge::reply_js(id, ok, &payload));
            } else if let Err(e) = result {
                tracing::debug!(method = req.method, error = %e, "bridge notification failed");
            }
        });
    }

    pub fn broadcast(&self, event: &str, payload: &Value) {
        let js = bridge::dispatch_js(event, payload);
        for w in self.windows.borrow().iter() {
            w.eval(&js);
        }
    }

    async fn dispatch(self: &Rc<Self>, win: &Rc<ShellWindow>, method: &str, params: Value) -> Result<Value, String> {
        tracing::debug!(kind = win.kind.as_str(), id = win.id, method, "bridge");
        let str_param = |key: &str| -> Result<String, String> {
            params
                .get(key)
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .ok_or_else(|| format!("{method}: missing '{key}'"))
        };
        if self.is_greeter() && !method.starts_with("greeter.") && !GREETER_ALLOWED.contains(&method) {
            return Err(format!("'{method}' is not available on the login screen"));
        }
        match method {
            "shell.state" => Ok(self.state_json()),
            // ---- the login screen (greetd does the authenticating)
            "greeter.info" => Ok(greeter::info_json()),
            "greeter.login" => {
                let user = str_param("user")?;
                let password = params.get("password").and_then(Value::as_str).unwrap_or("").to_string();
                let session = params.get("session").and_then(Value::as_str).unwrap_or("mindos").to_string();
                let exec = greeter::sessions()
                    .into_iter()
                    .find(|s| s.id == session)
                    .map(|s| s.exec)
                    .ok_or_else(|| format!("unknown session '{session}'"))?;
                if let Some(mut old) = self.greeter.borrow_mut().take() {
                    blocking(move || old.cancel()).await;
                }
                let (login, outcome) = blocking(move || greeter::Login::start(&user, &password, vec![exec], Vec::new())).await?;
                self.greeter_outcome(login, outcome, &session)
            }
            "greeter.respond" => {
                let response = params.get("response").and_then(Value::as_str).map(|s| s.to_string());
                let session = params.get("session").and_then(Value::as_str).unwrap_or("mindos").to_string();
                let mut login = self.greeter.borrow_mut().take().ok_or("greeter.respond: no login in progress")?;
                let (login, result) = blocking(move || {
                    let r = login.respond(response);
                    (login, r)
                })
                .await;
                self.greeter_outcome(login, result?, &session)
            }
            "greeter.cancel" => {
                if let Some(mut login) = self.greeter.borrow_mut().take() {
                    blocking(move || login.cancel()).await;
                }
                Ok(Value::Null)
            }
            "greeter.done" => {
                // The session is queued in greetd; it starts when the greeter
                // is gone. The compositor ends, and this process with it.
                tracing::info!("login complete, handing over to the session");
                if self.ipc.is_connected() {
                    let _ = self.ipc.request(json!({ "type": "quit" })).await;
                }
                let _ = self.events.send_blocking(HostEvent::Quit);
                Ok(Value::Null)
            }
            "greeter.power" => {
                let action = str_param("action")?;
                if !matches!(action.as_str(), "poweroff" | "reboot" | "suspend") {
                    return Err(format!("greeter.power: '{action}' is not offered on the login screen"));
                }
                blocking(move || system::power(&action)).await.map(|_| Value::Null)
            }
            "shell.ready" => {
                win.ready.set(true);
                tracing::debug!(kind = win.kind.as_str(), id = win.id, output = win.output, "view ready");
                Ok(Value::Null)
            }
            "shell.setEditMode" => {
                let enabled = params.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                self.set_edit_mode(enabled);
                Ok(json!({ "enabled": enabled }))
            }
            "shell.exec" => {
                let cmd = str_param("cmd")?;
                let terminal = params.get("terminal").and_then(Value::as_bool).unwrap_or(false);
                self.launch(&cmd, terminal).await
            }
            "shell.reload" => {
                for w in self.windows.borrow().iter() {
                    w.ready.set(false);
                    w.view.reload();
                }
                Ok(Value::Null)
            }
            "layout.get" => Ok(self.state.borrow().layout.to_value()),
            "layout.save" => {
                let value = params.get("layout").cloned().ok_or("layout.save: missing 'layout'")?;
                let layout = Layout::from_value(value)?.sanitized();
                layout.save()?;
                self.set_layout(layout.clone());
                Ok(layout.to_value())
            }
            "layout.reset" => {
                let layout = Layout::reset();
                self.set_layout(layout.clone());
                Ok(layout.to_value())
            }
            "popup.open" => self.open_popup(win, &params, false),
            "popup.toggle" => self.open_popup(win, &params, true),
            "panel.fit" => {
                // A fit-to-content panel reports the length its widgets need.
                if win.kind != Kind::Panel {
                    return Err("panel.fit: not a panel window".into());
                }
                let length = params.get("length").and_then(Value::as_f64).unwrap_or(0.0).round() as i32;
                let changed = {
                    let mut spec = win.panel.borrow_mut();
                    match spec.as_mut() {
                        Some(s) if s.is_fit() && s.fit != length && length > 0 => {
                            s.fit = length;
                            true
                        }
                        _ => false,
                    }
                };
                if changed {
                    win.apply_geometry(self.state.borrow().edit_mode);
                }
                Ok(json!({ "applied": changed }))
            }
            "popup.close" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .or_else(|| (win.kind == Kind::Popup).then(|| win.id.clone()))
                    .ok_or("popup.close: missing 'name'")?;
                Ok(json!({ "closed": self.close_popup(&name) }))
            }
            "windows.focus" | "windows.close" | "windows.minimize" | "windows.unminimize" | "windows.toggleMinimize"
            | "windows.toggleFullscreen" | "windows.toggleMaximize" => {
                let id = params.get("id").cloned().ok_or_else(|| format!("{method}: missing 'id'"))?;
                let ty = match method {
                    "windows.focus" => "focus",
                    "windows.close" => "close",
                    "windows.minimize" => "minimize",
                    "windows.unminimize" => "unminimize",
                    "windows.toggleMinimize" => "toggle_minimize",
                    "windows.toggleFullscreen" => "toggle_fullscreen",
                    _ => "toggle_maximize",
                };
                self.ipc.request(json!({ "type": ty, "window": id })).await
            }
            "apps.list" => Ok(self.apps_json()),
            "apps.launch" => {
                let (exec, terminal) = if let Some(id) = params.get("id").and_then(Value::as_str) {
                    let state = self.state.borrow();
                    let app = apps::find(&state.apps, id).ok_or_else(|| format!("unknown application '{id}'"))?;
                    (app.exec.clone(), app.terminal)
                } else {
                    (str_param("exec")?, params.get("terminal").and_then(Value::as_bool).unwrap_or(false))
                };
                self.launch(&exec, terminal).await
            }
            "tray.items" => Ok(self.tray_items_json()),
            "tray.activate" | "tray.secondaryActivate" => {
                let id = str_param("id")?;
                if let Some(icon) = xembed_id(&id) {
                    let button = if method == "tray.secondaryActivate" { 3 } else { 1 };
                    return self.ipc.request(json!({ "type": "tray_click", "icon": icon, "button": button })).await;
                }
                let (x, y) = self.screen_point(win, &params);
                self.tray()?.activate(id, x, y, method == "tray.secondaryActivate").await?;
                Ok(Value::Null)
            }
            "tray.scroll" => {
                let id = str_param("id")?;
                let delta = params.get("delta").and_then(Value::as_f64).unwrap_or(0.0).round() as i32;
                let orientation = params
                    .get("orientation")
                    .and_then(Value::as_str)
                    .unwrap_or("vertical")
                    .to_string();
                if let Some(icon) = xembed_id(&id) {
                    // X wheel buttons: 4 up, 5 down, 6 left, 7 right.
                    let button = match (orientation.as_str(), delta > 0) {
                        ("horizontal", true) => 7,
                        ("horizontal", false) => 6,
                        (_, true) => 5,
                        (_, false) => 4,
                    };
                    return self.ipc.request(json!({ "type": "tray_click", "icon": icon, "button": button })).await;
                }
                self.tray()?.scroll(id, delta, orientation).await?;
                Ok(Value::Null)
            }
            "tray.menu" => {
                let id = str_param("id")?;
                self.tray()?.menu(id).await
            }
            "tray.menuClick" => {
                let id = str_param("id")?;
                let item = params
                    .get("item")
                    .and_then(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())))
                    .ok_or("tray.menuClick: missing 'item'")? as i32;
                self.tray()?.menu_click(id, item).await?;
                Ok(Value::Null)
            }
            "mind.toggle" | "mind.open" | "mind.close" => {
                let action = method.trim_start_matches("mind.");
                let mut req = json!({ "type": "mindbar", "action": action });
                if let Some(text) = params.get("text").and_then(Value::as_str) {
                    req["text"] = json!(text);
                    req["ask"] = json!(params.get("ask").and_then(Value::as_bool).unwrap_or(false));
                }
                self.ipc.request(req).await
            }
            "mind.status" => Ok(self.mind_json()),
            "mind.notices" => Ok(json!({ "notices": self.state.borrow().mind_notices })),
            "mind.dismiss" => {
                let id = str_param("id")?;
                {
                    let mut state = self.state.borrow_mut();
                    state.mind_notices.retain(|n| id != "*" && n.get("id").and_then(Value::as_str) != Some(id.as_str()));
                }
                self.mind_notices_changed(None);
                let req = json!({ "type": "dismiss_notice", "id": id });
                blocking(move || mind::request(req)).await
            }
            "mind.act" => {
                let action = params.get("action").cloned().filter(Value::is_object).ok_or("mind.act: missing 'action'")?;
                self.mind_act(&action).await
            }
            // ---- notifications
            "notify.list" => Ok(Self::notify_json_locked(&self.state.borrow())),
            "notify.close" => {
                let id = params.get("id").and_then(Value::as_u64).ok_or("notify.close: missing 'id'")? as u32;
                let reason = params.get("reason").and_then(Value::as_u64).unwrap_or(2) as u32;
                Ok(json!({ "closed": self.close_notification(id, reason) }))
            }
            "notify.action" => {
                let id = params.get("id").and_then(Value::as_u64).ok_or("notify.action: missing 'id'")? as u32;
                let key = str_param("key")?;
                let resident = self
                    .state
                    .borrow()
                    .notifications
                    .iter()
                    .find(|n| n.get("id").and_then(Value::as_u64) == Some(id as u64))
                    .and_then(|n| n.get("resident").and_then(Value::as_bool))
                    .unwrap_or(false);
                if let Some(n) = &self.notify {
                    n.action(id, key);
                }
                if !resident {
                    self.close_notification(id, 2);
                }
                Ok(Value::Null)
            }
            "notify.clear" => {
                let ids: Vec<u32> = self.state.borrow().notifications.iter().filter_map(|n| n.get("id").and_then(Value::as_u64)).map(|i| i as u32).collect();
                for id in ids {
                    self.close_notification(id, 2);
                }
                Ok(Value::Null)
            }
            "notify.setDnd" => {
                let enabled = params.get("enabled").and_then(Value::as_bool).unwrap_or(false);
                self.state.borrow_mut().dnd = enabled;
                self.notify_changed(None, None);
                Ok(json!({ "enabled": enabled }))
            }
            "toast.fit" => {
                if win.kind != Kind::Toast {
                    return Err("toast.fit: not the toast window".into());
                }
                let w = params.get("w").and_then(Value::as_f64).unwrap_or(0.0).ceil() as i32;
                let h = params.get("h").and_then(Value::as_f64).unwrap_or(0.0).ceil() as i32;
                let show = w > 1 && h > 1;
                let size = if show { (w, h) } else { windows::TOAST_DEFAULT };
                if win.toast.get() != size {
                    win.toast.set(size);
                    win.apply_geometry(self.state.borrow().edit_mode);
                }
                if win.window.is_visible() != show {
                    win.window.set_visible(show);
                }
                Ok(json!({ "visible": show }))
            }
            "polkit.respond" => {
                let id = params.get("id").and_then(Value::as_u64).ok_or("polkit.respond: missing 'id'")?;
                let password = params.get("password").and_then(Value::as_str).unwrap_or("").to_string();
                self.polkit.as_ref().ok_or("polkit.respond: no agent in this window")?.respond(id, Some(password));
                Ok(Value::Null)
            }
            "polkit.cancel" => {
                let id = params.get("id").and_then(Value::as_u64).ok_or("polkit.cancel: missing 'id'")?;
                if let Some(agent) = self.polkit.as_ref() {
                    agent.respond(id, None);
                }
                Ok(Value::Null)
            }
            "shell.run" => {
                let argv: Vec<String> = params
                    .get("argv")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                    .unwrap_or_default();
                if argv.is_empty() {
                    return Err("shell.run: missing 'argv'".into());
                }
                self.run_helper(argv).await
            }
            "system.power" => {
                let action = str_param("action")?;
                if action == "logout" {
                    if self.ipc.is_connected() {
                        return self.ipc.request(json!({ "type": "quit" })).await;
                    }
                    return blocking(system::logout_fallback).await.map(|_| Value::Null);
                }
                blocking(move || system::power(&action)).await.map(|_| Value::Null)
            }
            "system.stats" => {
                let mut v = self.stats.borrow_mut().snapshot();
                v["gpu"] = self.state.borrow().gpu.clone().unwrap_or(Value::Null);
                Ok(v)
            }
            "audio.get" => Ok(system::audio_value(&self.state.borrow().audio)),
            "audio.set" => {
                let volume = params.get("volume").and_then(Value::as_f64).ok_or("audio.set: missing 'volume'")?;
                blocking(move || system::audio_set(volume)).await?;
                self.refresh_audio().await;
                Ok(system::audio_value(&self.state.borrow().audio))
            }
            "audio.toggleMute" => {
                blocking(system::audio_toggle_mute).await?;
                self.refresh_audio().await;
                Ok(system::audio_value(&self.state.borrow().audio))
            }
            "network.status" => Ok(blocking(system::network_status).await),
            "battery.status" => Ok(blocking(system::battery_status).await),
            "icons.resolve" => {
                let name = str_param("name")?;
                let size = params.get("size").and_then(Value::as_u64).unwrap_or(self.config.shell.icon_size as u64) as u16;
                Ok(match icons::resolve(&name, size, &self.icon_theme) {
                    Some(_) => Value::String(icon_url(&name, size)),
                    None => Value::Null,
                })
            }
            "outputs.list" => Ok(self.outputs_json()),
            // ---- compositor: layout modes, prefs, outputs
            "wm.layoutMode" => self.ipc.request(json!({ "type": "get_layout_mode" })).await,
            "wm.setLayoutMode" => {
                let mode = str_param("mode")?;
                self.ipc.request(json!({ "type": "set_layout_mode", "mode": mode })).await
            }
            "wm.cycleLayoutMode" => self.ipc.request(json!({ "type": "cycle_layout_mode" })).await,
            "wm.outputs" => self.ipc.request(json!({ "type": "get_outputs" })).await,
            "wm.setOutput" => {
                let mut change = params.as_object().cloned().ok_or("wm.setOutput: expected an object")?;
                if !change.get("name").map(Value::is_string).unwrap_or(false) {
                    return Err("wm.setOutput: missing 'name'".into());
                }
                change.insert("type".into(), json!("set_output"));
                self.ipc.request(Value::Object(change)).await
            }
            "prefs.get" => self.ipc.request(json!({ "type": "get_prefs" })).await,
            "prefs.set" => {
                let prefs = params.get("prefs").cloned().filter(Value::is_object).ok_or("prefs.set: missing 'prefs'")?;
                self.ipc.request(json!({ "type": "set_prefs", "prefs": prefs })).await
            }
            // ---- the Mind daemon
            "mind.request" => {
                let request = params
                    .get("request")
                    .cloned()
                    .filter(Value::is_object)
                    .ok_or("mind.request: missing 'request'")?;
                blocking(move || mind::request(request)).await
            }
            // ---- wallpapers and the desktop folder
            "wallpaper.list" => Ok(blocking(fs::wallpapers).await),
            "fs.desktop" => Ok(json!({ "path": fs::desktop_dir().to_string_lossy() })),
            "fs.list" => {
                let path = fs::expand(&str_param("path").unwrap_or_default());
                let hidden = params.get("hidden").and_then(Value::as_bool).unwrap_or(false);
                let theme = self.icon_theme.clone();
                let size = self.config.shell.icon_size;
                blocking(move || fs::list(&path, hidden, &theme, size)).await
            }
            "fs.trash" => {
                let paths: Vec<PathBuf> = params
                    .get("paths")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(fs::expand).collect())
                    .unwrap_or_default();
                if paths.is_empty() {
                    return Err("fs.trash: missing 'paths'".into());
                }
                let count = blocking(move || fs::trash(&paths)).await?;
                Ok(json!({ "count": count }))
            }
            "fs.open" => {
                let path = fs::expand(&str_param("path")?);
                let (cmd, terminal) = fs::opener(&path).ok_or("no application is set up to open this file")?;
                self.launch(&cmd, terminal).await
            }
            // ---- app windows
            "shell.openApp" => {
                let name = str_param("name")?;
                let page = params.get("page").and_then(Value::as_str).unwrap_or("");
                let arg = params.get("arg").and_then(Value::as_str).unwrap_or("");
                self.open_app(&name, page, arg)
            }
            "app.close" => {
                if self.app_mode.is_some() {
                    let _ = self.events.send_blocking(HostEvent::Quit);
                }
                Ok(Value::Null)
            }
            "app.setTitle" => {
                let title = str_param("title")?;
                if win.kind == Kind::App {
                    win.window.set_title(Some(&title));
                }
                Ok(Value::Null)
            }
            _ => Err(format!("unknown method '{method}'")),
        }
    }

    /// Keep or drop the login conversation depending on how far it got.
    fn greeter_outcome(&self, login: greeter::Login, outcome: greeter::Outcome, session: &str) -> Result<Value, String> {
        match &outcome {
            greeter::Outcome::Prompt { .. } => *self.greeter.borrow_mut() = Some(login),
            greeter::Outcome::Started => {
                greeter::save_state(&login.user, session);
                tracing::info!(user = login.user, session, "authenticated");
            }
            greeter::Outcome::Failed { .. } => {}
        }
        Ok(greeter::outcome_json(&outcome))
    }

    /// StatusNotifier items first, then the compositor's XEmbed icons.
    fn tray_items_json(&self) -> Value {
        self.tray_items_locked(&self.state.borrow())
    }

    fn tray_items_locked(&self, state: &State) -> Value {
        let mut items = state.tray.as_array().cloned().unwrap_or_default();
        items.extend(state.xtray.as_array().cloned().unwrap_or_default());
        Value::Array(items)
    }

    fn tray(&self) -> Result<&TrayHandle, String> {
        self.tray.as_ref().ok_or_else(|| "the tray is not available in app windows".to_string())
    }

    /// Start `mindshell --app NAME` as a separate process (its own window,
    /// its own web process; the shell keeps running if it crashes).
    fn open_app(&self, name: &str, page: &str, arg: &str) -> Result<Value, String> {
        if !crate::APPS.contains(&name) {
            return Err(format!("unknown app '{name}'"));
        }
        let exe = std::env::current_exe().map_err(|e| format!("cannot find mindshell: {e}"))?;
        let mut cmd = format!("{} --app {}", shell_quote(&exe.to_string_lossy()), shell_quote(name));
        if !page.is_empty() {
            cmd.push_str(&format!(" --page {}", shell_quote(page)));
        }
        if !arg.is_empty() {
            cmd.push_str(&format!(" {}", shell_quote(arg)));
        }
        let mut env: Vec<(String, String)> = Vec::new();
        if let Some(dir) = &self.ui_dir_override {
            env.push(("MINDSHELL_UI_DIR".into(), dir.to_string_lossy().to_string()));
        }
        if self.devtools {
            env.push(("MINDSHELL_DEVTOOLS".into(), "1".into()));
        }
        apps::spawn_detached(&cmd, &env).map(|_| Value::Null)
    }

    // ------------------------------------------------------------------ state

    fn state_json(&self) -> Value {
        let state = self.state.borrow();
        let uptime = std::fs::read_to_string("/proc/uptime")
            .ok()
            .and_then(|t| t.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
            .unwrap_or_else(|| self.started.elapsed().as_secs_f64());
        let c = &self.config;
        json!({
            "user": system::user_name(),
            "host": system::host_name(),
            "uptime": uptime,
            "outputs": self.outputs_json(),
            "windows": if state.windows.is_array() { state.windows.clone() } else { json!([]) },
            "focused": state.focused.clone(),
            "apps": self.apps_json_locked(&state.apps),
            "tray": self.tray_items_locked(&state),
            "layout": state.layout.to_value(),
            "editMode": state.edit_mode,
            "config": {
                "icon_theme": self.icon_theme,
                "hardware_acceleration": c.shell.hardware_acceleration,
                "terminal": c.shell.terminal,
                "icon_size": c.shell.icon_size,
            },
            "app": self.app_mode.as_ref().map(|m| json!({ "name": m.name, "page": m.page, "arg": m.arg })),
            "version": env!("CARGO_PKG_VERSION"),
            "mind": self.mind_json_locked(&state),
            "notify": Self::notify_json_locked(&state),
            "polkit": state.polkit.clone(),
            "audio": system::audio_value(&state.audio),
            "compositor": self.ipc.is_connected(),
            "devtools": self.devtools,
        })
    }

    fn outputs_json(&self) -> Value {
        let state = self.state.borrow();
        let monitors = windows::monitors();
        let mut list: Vec<Value> = monitors
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let name = windows::monitor_name(m, i);
                let mut v = windows::output_info(m, &name);
                if let Some(c) = state.ipc_outputs.iter().find(|o| o["name"] == name) {
                    if v["refresh"].is_null() {
                        v["refresh"] = c["refresh"].clone();
                    }
                    if v["make"].is_null() {
                        v["make"] = c["make"].clone();
                    }
                    if v["model"].is_null() {
                        v["model"] = c["model"].clone();
                    }
                }
                v
            })
            .collect();
        if list.is_empty() {
            list = state.ipc_outputs.clone();
        }
        Value::Array(list)
    }

    fn apps_json(&self) -> Value {
        self.apps_json_locked(&self.state.borrow().apps)
    }

    fn apps_json_locked(&self, apps: &[AppEntry]) -> Value {
        let size = self.config.shell.icon_size;
        Value::Array(
            apps.iter()
                .map(|a| {
                    let icon = match &a.icon_path {
                        Some(path) => icon_url(path, size),
                        None => icon_url("application-x-executable", size),
                    };
                    json!({
                        "id": a.id,
                        "name": a.name,
                        "comment": a.comment,
                        "exec": a.exec,
                        "icon": icon,
                        "iconName": a.icon,
                        "categories": a.categories,
                        "terminal": a.terminal,
                        "wmClass": a.wm_class,
                        "wine": a.wine,
                    })
                })
                .collect(),
        )
    }

    fn mind_json(&self) -> Value {
        self.mind_json_locked(&self.state.borrow())
    }

    fn mind_json_locked(&self, state: &State) -> Value {
        let connected = self.ipc.is_connected();
        let mut v = json!({
            "connected": connected,
            "ready": connected,
            "model": Value::Null,
            "open": state.mind_open,
        });
        if let Value::Object(extra) = &state.mind_extra {
            for (k, val) in extra {
                v[k] = val.clone();
            }
        }
        v["daemon"] = json!(state.mind_daemon);
        v["sleeping"] = json!(state.mind_sleeping);
        v["notices"] = json!(state.mind_notices);
        v["updates"] = state.mind_updates.clone();
        v["health"] = state.mind_health.clone();
        v
    }

    fn notify_json_locked(state: &State) -> Value {
        json!({ "items": state.notifications, "dnd": state.dnd })
    }

    /// Tell every view the notification list changed. `added` is the new
    /// notification (the toast window shows it), `closed` an id that went.
    fn notify_changed(&self, added: Option<&Value>, closed: Option<u32>) {
            // ---- WireGuard tunnels (NetworkManager connections of type wireguard)
            "vpn.list" => Ok(blocking(system::vpn_list).await),
            "vpn.connect" | "vpn.disconnect" => {
                let id = str_param("id")?;
                let up = method == "vpn.connect";
                blocking(move || system::vpn_set_active(&id, up)).await?;
                Ok(self.network_changed().await)
            }
            "vpn.autoconnect" => {
                let id = str_param("id")?;
                let on = params.get("on").and_then(Value::as_bool).ok_or("vpn.autoconnect: missing 'on'")?;
                blocking(move || system::vpn_set_autoconnect(&id, on)).await?;
                Ok(self.network_changed().await)
            }
            "vpn.remove" => {
                let id = str_param("id")?;
                blocking(move || system::vpn_remove(&id)).await?;
                Ok(self.network_changed().await)
            }
            "vpn.import" => {
                // A file chooser for a wg-quick configuration; `path` skips it.
                let path = match params.get("path").and_then(Value::as_str) {
                    Some(p) => PathBuf::from(p),
                    None => match self.pick_file("Import a WireGuard configuration", &[("WireGuard configuration", "*.conf")]).await {
                        Some(p) => p,
                        None => return Ok(json!({ "imported": false })),
                    },
                };
                let id = blocking(move || system::vpn_import(&path)).await?;
                let mut v = self.network_changed().await;
                v["imported"] = json!(true);
                v["id"] = json!(id);
                Ok(v)
            }
        let mut v = Self::notify_json_locked(&self.state.borrow());
        v["added"] = added.cloned().unwrap_or(Value::Null);
        v["closed"] = closed.map(|id| json!(id)).unwrap_or(Value::Null);
        self.broadcast("notify", &v);
    }

    /// Drop notification `id` and tell its application why
    /// (1 expired, 2 dismissed by the user, 3 closed by a call).
    fn close_notification(&self, id: u32, reason: u32) -> bool {
        let removed = {
            let mut state = self.state.borrow_mut();
            let before = state.notifications.len();
            state.notifications.retain(|n| n.get("id").and_then(Value::as_u64) != Some(id as u64));
            state.notifications.len() != before
        };
        if let Some(n) = &self.notify {
            n.closed(id, reason);
        }
        if removed {
            self.notify_changed(None, Some(id));
        }
        removed
    }

    fn mind_notices_changed(&self, added: Option<&Value>) {
        let notices = self.state.borrow().mind_notices.clone();
        self.broadcast("mind_notices", &json!({ "notices": notices, "added": added.cloned().unwrap_or(Value::Null) }));
    }

    /// Run one of the system helpers for the UI and return its output.
    /// Only a fixed set of read-mostly commands is allowed; anything that
    /// changes the system goes through the helper's own sudo rules.
    async fn run_helper(&self, argv: Vec<String>) -> Result<Value, String> {
        let bare: Vec<&str> = argv.iter().map(String::as_str).collect();
        let (sudo, cmd) = match bare.as_slice() {
            ["sudo", "-n", rest @ ..] => (true, rest),
            rest => (false, rest),
        };
        let allowed = match cmd {
            ["mindos-perf", verb, ..] => matches!(*verb, "status" | "get" | "modes" | "set" | "config" | "apply"),
            ["mindos-dlss", ..] => !sudo,
            ["mindos-dev-setup", ..] => true,
            ["mindos-boot", "list", ..] => true,
            ["pacman", flag, ..] => !sudo && flag.starts_with("-Q"),
            ["checkupdates", ..] => !sudo,
            ["nvidia-smi", ..] => !sudo,
            // Settings > Developer, through the authentication dialog.
            ["pkexec", "systemctl", "enable", "--now", "docker.service"] => !sudo,
            ["pkexec", "usermod", "-aG", "docker", who] => !sudo && *who == system::user_name(),
            _ => false,
        };
        if !allowed {
            return Err(format!("shell.run: '{}' is not allowed", argv.join(" ")));
        }
        let argv2 = argv.clone();
        let out = blocking(move || {
            std::process::Command::new(&argv2[0])
                .args(&argv2[1..])
                .stdin(std::process::Stdio::null())
                .output()
                .map_err(|e| format!("{}: {e}", argv2[0]))
        })
        .await?;
            // ---- the pointer: GSettings for applications, IPC for the compositor
            "pointer.get" => Ok(pointer::state()),
            "pointer.set" => {
                let theme = params.get("theme").and_then(Value::as_str).map(str::to_string);
                let size = params.get("size").and_then(Value::as_u64).map(|s| s as u32);
                if theme.is_none() && size.is_none() {
                    return Err("pointer.set: nothing to change".into());
                }
                pointer::set(theme.as_deref(), size)?;
                // The compositor draws its own cursor and does not read
                // GSettings; it also passes the size on to what it starts.
                let mut prefs = serde_json::Map::new();
                if let Some(theme) = &theme {
                    prefs.insert("cursor_theme".into(), json!(theme));
                }
                if let Some(size) = size {
                    prefs.insert("cursor_size".into(), json!(size));
                }
                if let Err(err) = self.ipc.request(json!({ "type": "set_prefs", "prefs": prefs })).await {
                    tracing::warn!(%err, "the compositor kept its old pointer");
                }
                Ok(pointer::state())
            }
            // ---- the screensaver and the lock screen
            "lock.info" => {
                let mut info = crate::auth::user_info();
                info["host"] = json!(system::host_name());
                info["idle"] = self.state.borrow().idle.clone();
                Ok(info)
            }
            "lock.state" => Ok(self.state.borrow().idle.clone()),
            "lock.unlock" => {
                let password = params
                    .get("password")
                    .and_then(Value::as_str)
                    .map(|s| s.to_string())
                    .ok_or("lock.unlock: missing 'password'")?;
                self.unlock(password).await
            }
            "lock.now" => self.ipc.request(json!({ "type": "lock" })).await,
            "lock.wake" => self.ipc.request(json!({ "type": "wake" })).await,
            "lock.blank" => self.ipc.request(json!({ "type": "blank" })).await,
        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        let json = serde_json::from_str::<Value>(stdout.trim()).unwrap_or(Value::Null);
        // A mode switch from one window (the popup) should show in every
        // other window (the bar widget, Settings) at once.
        if out.status.success() && matches!(cmd, ["mindos-perf", "set" | "config" | "apply", ..]) {
            self.broadcast("perf_changed", &json!({ "argv": argv }));
        }
        Ok(json!({
            "status": out.status.code().unwrap_or(-1),
            "ok": out.status.success(),
            "stdout": stdout,
            "stderr": stderr,
            "json": json,
        }))
    }

    /// Carry out a notice action: open the Mind bar with a question, send a
    /// daemon request, run a command, or open a Settings page.
    async fn mind_act(self: &Rc<Self>, action: &Value) -> Result<Value, String> {
        let kind = action.get("kind").and_then(Value::as_str).unwrap_or("");
        let arg = action.get("arg").cloned().unwrap_or(Value::Null);
        match kind {
            "chat" => {
                let text = arg.as_str().unwrap_or("").to_string();
                self.ipc.request(json!({ "type": "mindbar", "action": "open", "text": text, "ask": true })).await
            }
            "request" => {
                let request = arg.clone();
                if !request.is_object() {
                    return Err("mind.act: 'request' needs an object".into());
                }
                blocking(move || mind::request(request)).await
            }
            "command" => {
                let cmd = arg.as_str().ok_or("mind.act: 'command' needs a string")?.to_string();
                self.launch(&cmd, false).await
            }
            "settings" => {
                let page = arg.as_str().unwrap_or("");
                self.open_app("settings", page, "")
            }
            other => Err(format!("mind.act: unknown action kind '{other}'")),
        }
    }

    fn set_edit_mode(&self, enabled: bool) {
        {
            let mut state = self.state.borrow_mut();
            if state.edit_mode == enabled {
                return;
            }
            state.edit_mode = enabled;
        }
        for w in self.windows.borrow().iter() {
            if w.kind != Kind::Popup {
                w.apply_geometry(enabled);
            }
        }
        self.broadcast("edit_mode", &json!({ "enabled": enabled }));
    }

    fn set_layout(self: &Rc<Self>, layout: Layout) {
        self.state.borrow_mut().layout = layout.clone();
        self.sync_windows();
        self.broadcast("layout", &json!({ "layout": layout.to_value() }));
    }

    /// Translate a point in the calling window into global screen coordinates.
    fn screen_point(&self, win: &ShellWindow, params: &Value) -> (i32, i32) {
        let edit = self.state.borrow().edit_mode;
        let (ox, oy) = win.origin(edit);
        let g = win.monitor.geometry();
        let x = params.get("x").and_then(Value::as_f64).unwrap_or(0.0) as i32;
        let y = params.get("y").and_then(Value::as_f64).unwrap_or(0.0) as i32;
        (g.x() + ox + x, g.y() + oy + y)
    }

    async fn launch(&self, exec: &str, terminal: bool) -> Result<Value, String> {
        if self.ipc.is_connected() {
            match self
                .ipc
                .request(json!({ "type": "launch", "exec": exec, "terminal": terminal }))
                .await
            {
                Ok(v) => return Ok(v),
                Err(e) => tracing::debug!(%e, "compositor launch failed; spawning locally"),
            }
        }
        let cmd = if terminal {
            format!("{} -e sh -c {}", self.config.shell.terminal, shell_quote(exec))
        } else {
            exec.to_string()
        };
        apps::spawn_detached(&cmd, &[]).map(|_| Value::Null)
    }

    async fn refresh_audio(&self) {
        let audio = blocking(system::audio_get).await;
        self.set_audio(audio);
    }

    fn set_audio(&self, audio: Option<Audio>) {
        let changed = {
            let mut state = self.state.borrow_mut();
            if state.audio == audio {
                false
            } else {
                state.audio = audio;
                true
            }
        };
        if changed {
            self.broadcast("audio", &system::audio_value(&self.state.borrow().audio));
        }
    }

    // ----------------------------------------------------------------- popups

    fn open_popup(self: &Rc<Self>, caller: &Rc<ShellWindow>, params: &Value, toggle: bool) -> Result<Value, String> {
        if self.app_mode.is_some() {
            return Err("popups are not available in app windows".into());
        }
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or("popup.open: missing 'name'")?
            .to_string();
        let keyboard = params.get("keyboard").and_then(Value::as_bool).unwrap_or(false);
        let edit = self.state.borrow().edit_mode;
        // Anchors arrive in output-local logical pixels (the UI adds the panel
        // window's origin itself); they go to the popup untouched.
        let anchor = params.get("anchor").filter(|a| a.is_object()).cloned();
        let arg = params.get("arg").cloned().unwrap_or(Value::Null);
        let output = caller.output.clone();
        let url = bridge::window_url("popup", &name, &output, Some(&name), Some(&arg), anchor.as_ref());

        if let Some(existing) = self.find_popup(&name) {
            if existing.output == output {
                if toggle {
                    self.close_popup(&name);
                    return Ok(json!({ "name": name, "open": false, "output": output }));
                }
                if *existing.url.borrow() != url {
                    existing.load(&url);
                }
                if existing.keyboard.get() != keyboard {
                    existing.keyboard.set(keyboard);
                    existing.apply_geometry(edit);
                }
                *existing.arg.borrow_mut() = arg;
                existing.window.present();
                self.broadcast("popup_state", &json!({ "name": name, "open": true, "output": output }));
                return Ok(json!({ "name": name, "open": true, "output": output }));
            }
            // Moves to the caller's output.
            self.remove_window(&existing);
            self.broadcast("popup_state", &json!({ "name": name, "open": false, "output": existing.output }));
        }
        let win = self.create_window(Kind::Popup, &name, &output, &caller.monitor, None, keyboard, &url);
        *win.arg.borrow_mut() = arg;
        self.broadcast("popup_state", &json!({ "name": name, "open": true, "output": output }));
        Ok(json!({ "name": name, "open": true, "output": output }))
    }

    /// Show the authentication dialog for the pending polkit request: a
    /// centred popup with the keyboard, on the primary output.
    fn show_auth_popup(self: &Rc<Self>) {
        if self.find_popup("auth").is_some() {
            return;
        }
        let monitors = windows::monitors();
        let Some(monitor) = monitors.first() else { return };
        let name = windows::monitor_name(monitor, 0);
        let url = bridge::window_url("popup", "auth", &name, Some("auth"), Some(&json!({})), None);
        self.create_window(Kind::Popup, "auth", &name, monitor, None, true, &url);
        self.broadcast("popup_state", &json!({ "name": "auth", "open": true, "output": name }));
    }

    pub fn close_popup(self: &Rc<Self>, name: &str) -> bool {
        let Some(win) = self.find_popup(name) else { return false };
        // Escape, or a click beside the authentication dialog, cancels the
        // request the agent is still waiting on.
        if name == "auth" {
            let id = self.state.borrow().polkit.get("id").and_then(Value::as_u64);
            if let (Some(id), Some(agent)) = (id, self.polkit.as_ref()) {
                agent.respond(id, None);
            }
        }
        let output = win.output.clone();
        self.remove_window(&win);
        self.broadcast("popup_state", &json!({ "name": name, "open": false, "output": output }));
        true
    }

    // ----------------------------------------------------------------- events

    pub fn handle_event(self: &Rc<Self>, event: HostEvent) {
        match event {
            HostEvent::Ipc(IpcEvent::Connected) => {
                self.broadcast("mind", &self.mind_json());
                let app = self.clone();
                glib::spawn_future_local(async move {
                    if let Ok(v) = app.ipc.request(json!({ "type": "mind_status" })).await {
                        app.state.borrow_mut().mind_extra = v;
                        app.broadcast("mind", &app.mind_json());
                    }
                });
            }
            HostEvent::Ipc(IpcEvent::Disconnected) => {
                {
                    let mut state = self.state.borrow_mut();
                    state.windows = json!([]);
                    state.focused = Value::Null;
                    state.mind_extra = Value::Null;
                    state.mind_open = false;
                }
                self.broadcast("windows", &json!({ "windows": [], "focused": null }));
                self.broadcast("mind", &self.mind_json());
            }
            HostEvent::Ipc(IpcEvent::Event(name, value)) => match name.as_str() {
                "tray" => {
                    let items = value.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
                    self.state.borrow_mut().xtray = Value::Array(items.iter().filter_map(xembed_item_json).collect());
                    self.broadcast("tray", &json!({ "items": self.tray_items_json() }));
                }
                "windows" => {
                    let windows = value.get("windows").cloned().unwrap_or(json!([]));
                    let focused = value.get("focused").cloned().unwrap_or(Value::Null);
                    {
                        let mut state = self.state.borrow_mut();
                        state.windows = windows.clone();
                        state.focused = focused.clone();
                    }
                    self.broadcast("windows", &json!({ "windows": windows, "focused": focused }));
                }
                "outputs" => {
                    let outputs = value
                        .get("outputs")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    self.state.borrow_mut().ipc_outputs = outputs;
                    self.state.borrow_mut().last_outputs = None;
                    self.schedule_sync();
                }
                "shortcut" => {
                    // Forwarded to the views (the compositor opens the Mind bar for `launcher` itself).
                    let shortcut = value.get("name").cloned().unwrap_or(Value::Null);
                    tracing::debug!(name = %shortcut, "forwarding compositor shortcut");
                    self.broadcast("shortcut", &json!({ "name": shortcut }));
                }
                "mindbar" => {
                    self.state.borrow_mut().mind_open = value.get("open").and_then(Value::as_bool).unwrap_or(false);
                    self.broadcast("mind", &self.mind_json());
                }
                "mind_status" | "mind" => {
                    self.state.borrow_mut().mind_extra = value.clone();
                    self.broadcast("mind", &self.mind_json());
                }
                other => self.broadcast(other, &value),
            },
            HostEvent::Tray(items) => {
                self.state.borrow_mut().tray = items;
                self.broadcast("tray", &json!({ "items": self.tray_items_json() }));
            }
            HostEvent::Apps(list) => {
                self.state.borrow_mut().apps = list;
                self.broadcast("apps", &json!({ "apps": self.apps_json() }));
            }
            HostEvent::Audio(audio) => self.set_audio(audio),
            HostEvent::Gpu(gpu) => self.state.borrow_mut().gpu = gpu,
            HostEvent::LayoutFile => self.layout_file_changed(),
            HostEvent::DesktopDir => self.desktop_dir_changed(),
            HostEvent::Notify(mut n) => {
                let id = n.get("id").and_then(Value::as_u64).unwrap_or(0);
                {
                    let mut state = self.state.borrow_mut();
                    let quiet = state.dnd && n.get("urgency").and_then(Value::as_u64).unwrap_or(1) < 2;
                    n["quiet"] = json!(quiet);
                    state.notifications.retain(|x| x.get("id").and_then(Value::as_u64) != Some(id));
                    state.notifications.push(n.clone());
                    while state.notifications.len() > NOTIFICATION_LIMIT {
                        state.notifications.remove(0);
                    }
                }
                self.notify_changed(Some(&n), None);
            }
            HostEvent::NotifyClosed(id, reason) => {
                self.close_notification(id, reason);
            }
            HostEvent::Mind(v) => self.mind_event(v),
            HostEvent::Polkit(v) => {
                match v.get("type").and_then(Value::as_str).unwrap_or("") {
                    "ask" => {
                        self.state.borrow_mut().polkit = crate::polkit::request_json(&v);
                        let payload = self.state.borrow().polkit.clone();
                        self.broadcast("polkit", &payload);
                        self.show_auth_popup();
                    }
                    "busy" => {
                        {
                            let mut state = self.state.borrow_mut();
                            if state.polkit.is_object() {
                                state.polkit["busy"] = json!(true);
                            }
                        }
                        let payload = self.state.borrow().polkit.clone();
                        self.broadcast("polkit", &payload);
                    }
                    // The agent is done (granted, refused or cancelled).
                    "done" => {
                        self.state.borrow_mut().polkit = Value::Null;
                        self.close_popup("auth");
                        self.broadcast("polkit", &Value::Null);
                    }
                    _ => {}
                }
            }
            HostEvent::Quit => {
                tracing::info!("shutting down");
                let mut all: Vec<Rc<ShellWindow>> = self.windows.borrow_mut().drain(..).collect();
                all.extend(self.retired.borrow_mut().drain(..));
                *self.first_view.borrow_mut() = None;
                for w in all {
                    w.destroy();
                }
                self.main_loop.quit();
            }
        }
    }

    /// A line from the Mind daemon subscription.
    fn mind_event(&self, v: Value) {
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "connected" => {
                self.state.borrow_mut().mind_daemon = true;
                self.broadcast("mind", &self.mind_json());
            }
            "disconnected" => {
                {
                    let mut state = self.state.borrow_mut();
                    state.mind_daemon = false;
                    state.mind_sleeping = false;
                }
                self.broadcast("mind", &self.mind_json());
            }
            "notices" => {
                let list = v.get("notices").and_then(Value::as_array).cloned().unwrap_or_default();
                self.state.borrow_mut().mind_notices = list;
                self.mind_notices_changed(None);
            }
            "notice" => {
                let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                {
                    let mut state = self.state.borrow_mut();
                    state.mind_notices.retain(|n| n.get("id").and_then(Value::as_str) != Some(id.as_str()));
                    state.mind_notices.insert(0, v.clone());
                }
                self.mind_notices_changed(Some(&v));
            }
            "notice_gone" => {
                let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
                self.state.borrow_mut().mind_notices.retain(|n| n.get("id").and_then(Value::as_str) != Some(id.as_str()));
                self.mind_notices_changed(None);
            }
            "updates" => {
                self.state.borrow_mut().mind_updates = v.clone();
                self.broadcast("mind_updates", &v);
            }
            "health" => {
                self.state.borrow_mut().mind_health = v.clone();
                self.broadcast("mind_health", &v);
            }
            "sleep" => {
                self.state.borrow_mut().mind_sleeping = v.get("sleeping").and_then(Value::as_bool).unwrap_or(false);
                self.broadcast("mind", &self.mind_json());
            }
            _ => {}
        }
    }

    /// Background samplers: desktop entries, audio, GPU (the shell only),
    /// plus the standing connection to the Mind daemon.
    pub fn start_background(&self) {
        // Settings (Updates, Mind) shows the daemon's notices, update
        // status and health, so an --app window subscribes too.
        if !self.is_greeter() {
    /// Re-read the network state once the burst of change events has settled:
    /// a tunnel going down is several lines from `nmcli monitor`, and only the
    /// last one shows the final state.
    fn schedule_network_refresh(self: &Rc<Self>) {
        if let Some(id) = self.network_refresh.borrow_mut().take() {
            id.remove();
        }
        let app = self.clone();
        let id = glib::timeout_add_local_once(Duration::from_millis(400), move || {
            app.network_refresh.borrow_mut().take();
            let app = app.clone();
            glib::spawn_future_local(async move {
                app.network_changed().await;
            });
        });
        *self.network_refresh.borrow_mut() = Some(id);
    }

    /// Read the network and tunnel state again and tell every window; the
    /// tunnel list is also the answer to the `vpn.*` calls.
    async fn network_changed(&self) -> Value {
        let vpn = blocking(system::vpn_list).await;
        let network = blocking(system::network_status).await;
        self.broadcast("vpn", &vpn);
        self.broadcast("network", &network);
        vpn
    }

    /// A native open-file dialog; `None` when the user dismissed it.
    async fn pick_file(&self, title: &str, filters: &[(&str, &str)]) -> Option<PathBuf> {
        let dialog = gtk::FileDialog::builder().title(title).modal(true).build();
        let list = gio::ListStore::new::<gtk::FileFilter>();
        for (name, pattern) in filters {
            let f = gtk::FileFilter::new();
            f.set_name(Some(name));
            f.add_pattern(pattern);
            list.append(&f);
        }
        let all = gtk::FileFilter::new();
        all.set_name(Some("All files"));
        all.add_pattern("*");
        list.append(&all);
        dialog.set_filters(Some(&list));
        if let Some(first) = list.item(0).and_downcast::<gtk::FileFilter>() {
            dialog.set_default_filter(Some(&first));
        }
        if let Some(home) = system::home_dir() {
            dialog.set_initial_folder(Some(&gio::File::for_path(home)));
        }
        // A layer-shell surface cannot parent a dialog, so it opens as its own
        // window and the compositor places it.
        match dialog.open_future(None::<&gtk::Window>).await {
            Ok(file) => file.path(),
            Err(e) => {
                if !e.matches(gtk::DialogError::Dismissed) {
                    tracing::warn!(%e, "file dialog");
                }
                None
            }
        }
    }

            crate::mindwatch::start(self.events.clone());
        }
        if self.app_mode.is_some() {
            return;
        }
        let events = self.events.clone();
        let theme = self.icon_theme.clone();
        let size = self.config.shell.icon_size;
        std::thread::Builder::new()
            .name("mindshell-apps".into())
            .spawn(move || {
                let mut fingerprint = apps::fingerprint();
                let _ = events.send_blocking(HostEvent::Apps(apps::load_apps(&theme, size)));
                loop {
                    std::thread::sleep(Duration::from_secs(5));
                    let now = apps::fingerprint();
                    if now != fingerprint {
                        fingerprint = now;
                        if events.send_blocking(HostEvent::Apps(apps::load_apps(&theme, size))).is_err() {
                            return;
                        }
                    }
                }
            })
            .expect("spawn apps thread");

        let events = self.events.clone();
        std::thread::Builder::new()
            .name("mindshell-audio".into())
            .spawn(move || {
                let mut last: Option<Option<Audio>> = None;
                loop {
                    let now = system::audio_get();
                    if last.as_ref() != Some(&now) {
                        last = Some(now.clone());
                        if events.send_blocking(HostEvent::Audio(now)).is_err() {
                            return;
                        }
                    }
                    std::thread::sleep(Duration::from_millis(1500));
                }
            })
            .expect("spawn audio thread");

        let events = self.events.clone();
        std::thread::Builder::new()
            .name("mindshell-gpu".into())
            .spawn(move || {
                let mut stats = system::Stats::default();
                loop {
                    let gpu = stats.gpu_sample();
                    if events.send_blocking(HostEvent::Gpu(gpu)).is_err() {
                        return;
                    }
                    std::thread::sleep(Duration::from_secs(2));
                }
            })
            .expect("spawn gpu thread");
    }
}

pub fn icon_url(name: &str, size: u16) -> String {
    format!("{}/icon/{}?size={}", crate::scheme::ORIGIN, icons::percent_encode(name), size)
}

pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run a blocking function on a throwaway thread and await its result.
pub async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = async_channel::bounded::<T>(1);
    std::thread::spawn(move || {
        let _ = tx.send_blocking(f());
    });
    rx.recv().await.expect("blocking worker vanished")
}

fn dirs_data() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

pub fn dirs_cache() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .unwrap_or_else(|| PathBuf::from("/tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(icon_url("steam", 48), "mindos://shell/icon/steam?size=48");
        assert_eq!(icon_url("/usr/share/pixmaps/a b.png", 32), "mindos://shell/icon/%2Fusr%2Fshare%2Fpixmaps%2Fa%20b.png?size=32");
    }
}

/// The tray item id of an XEmbed icon is `x11:<window>`.
fn xembed_id(id: &str) -> Option<u32> {
    id.strip_prefix("x11:")?.parse().ok()
}

/// A compositor `tray` item (`{ id, title, class, pid, width, height, pixels }`,
/// pixels base64 RGBA) as a shell tray item with a data-URL icon.
fn xembed_item_json(item: &Value) -> Option<Value> {
    let id = item.get("id")?.as_u64()?;
    let width = item.get("width")?.as_u64()? as u32;
    let height = item.get("height")?.as_u64()? as u32;
    let pixels = gtk4::glib::base64_decode(item.get("pixels")?.as_str()?);
    let png = crate::tray::encode_rgba_png(width, height, &pixels)?;
    let title = item.get("title").and_then(Value::as_str).unwrap_or("").to_string();
    let class = item.get("class").and_then(Value::as_str).unwrap_or("").to_string();
    let label = if title.is_empty() { class.clone() } else { title.clone() };
    Some(json!({
        "id": format!("x11:{id}"),
        "title": label,
        "tooltip": title,
        "icon": format!("data:image/png;base64,{}", gtk4::glib::base64_encode(&png)),
        "status": "active",
        "hasMenu": false,
        "xembed": true,
        "app": class,
        "pid": item.get("pid").cloned().unwrap_or(Value::Null),
    }))
}
            HostEvent::Network => self.schedule_network_refresh(),
        // NetworkManager's own change feed: one line per event, which is the
        // cue to read the connection and tunnel state again.
        let events = self.events.clone();
        std::thread::Builder::new()
            .name("mindshell-network".into())
            .spawn(move || {
                use std::io::BufRead;
                let Ok(mut child) = std::process::Command::new("nmcli")
                    .arg("monitor")
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                else {
                    tracing::debug!("nmcli is not available; network changes are polled");
                    return;
                };
                let Some(stdout) = child.stdout.take() else { return };
                let mut lines = std::io::BufReader::new(stdout).lines();
                // Every line is reported; the main loop settles the burst.
                while let Some(Ok(_)) = lines.next() {
                    if events.send_blocking(HostEvent::Network).is_err() {
                        let _ = child.kill();
                        return;
                    }
                }
                let _ = child.wait();
            })
            .expect("spawn network thread");

