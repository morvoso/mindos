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
use crate::icons;
use crate::ipc::{IpcClient, IpcEvent};
use crate::layout::{Layout, Panel};
use crate::mind;
use crate::system::{self, Audio};
use crate::tray::TrayHandle;
use crate::windows::{self, Kind, PanelSpec, ShellWindow};
use crate::HostEvent;

pub const DEFAULT_UI_DIR: &str = "/usr/share/mindos/shell/ui";
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
}

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
            name,
            page: opts.page.unwrap_or_default(),
            arg: opts.arg.unwrap_or_default(),
        });
        tracing::info!(ui = %ui_dir.display(), icon_theme, devtools, hw = config.hardware_acceleration(), app = ?app_mode.as_ref().map(|m| &m.name), "mindshell starting");

        let web_context = webkit::WebContext::new();
        web_context.set_cache_model(webkit::CacheModel::DocumentViewer);
        let data_dir = dirs_data().join("mindos/shell");
        let cache_dir = dirs_cache().join("mindos/shell");
        let _ = std::fs::create_dir_all(&data_dir);
        let _ = std::fs::create_dir_all(&cache_dir);
        let network_session = webkit::NetworkSession::new(
            Some(&data_dir.to_string_lossy()),
            Some(&cache_dir.to_string_lossy()),
        );

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
    /// follow (a download landing there, a file renamed in Files).
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

    /// `--app`: the one window this process shows.
    fn sync_app_window(self: &Rc<Self>, mode: &AppMode) {
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
                .filter(|w| w.kind == Kind::Panel && w.output == *name && !wanted.iter().any(|p| p.id == w.id))
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
        view.connect_decide_policy(|_, decision, kind| {
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
            tracing::info!(uri, "opening external link");
            decision.ignore();
            let _ = apps::spawn_detached(&format!("xdg-open {}", shell_quote(&uri)), &[]);
            true
        });

        let window = gtk::Window::new();
        let sw = ShellWindow::new(kind, id, output, monitor, window, view);
        *sw.panel.borrow_mut() = panel;
        sw.keyboard.set(keyboard);
        sw.apply_geometry(self.state.borrow().edit_mode);
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
        match method {
            "shell.state" => Ok(self.state_json()),
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
                self.ipc.request(json!({ "type": "mindbar", "action": action })).await
            }
            "mind.status" => Ok(self.mind_json()),
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
            // ---- wallpapers and files
            "wallpaper.list" => Ok(blocking(fs::wallpapers).await),
            "fs.home" => Ok(json!({ "path": fs::home().to_string_lossy() })),
            "fs.desktop" => Ok(json!({ "path": fs::desktop_dir().to_string_lossy() })),
            "fs.places" => Ok(fs::places()),
            "fs.list" => {
                let path = fs::expand(&str_param("path").unwrap_or_default());
                let hidden = params.get("hidden").and_then(Value::as_bool).unwrap_or(false);
                let theme = self.icon_theme.clone();
                let size = self.config.shell.icon_size;
                blocking(move || fs::list(&path, hidden, &theme, size)).await
            }
            "fs.stat" => {
                let path = fs::expand(&str_param("path")?);
                blocking(move || fs::stat(&path)).await
            }
            "fs.mkdir" => {
                let parent = fs::expand(&str_param("path")?);
                let name = str_param("name")?;
                let made = blocking(move || fs::mkdir(&parent, &name)).await?;
                Ok(json!({ "path": made.to_string_lossy() }))
            }
            "fs.rename" => {
                let path = fs::expand(&str_param("path")?);
                let name = str_param("name")?;
                let renamed = blocking(move || fs::rename(&path, &name)).await?;
                Ok(json!({ "path": renamed.to_string_lossy() }))
            }
            "fs.trash" | "fs.copy" | "fs.move" => {
                let paths: Vec<PathBuf> = params
                    .get("paths")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(fs::expand).collect())
                    .unwrap_or_default();
                if paths.is_empty() {
                    return Err(format!("{method}: missing 'paths'"));
                }
                let count = if method == "fs.trash" {
                    blocking(move || fs::trash(&paths)).await?
                } else {
                    let dest = fs::expand(&str_param("dest")?);
                    let moving = method == "fs.move";
                    blocking(move || fs::transfer(&paths, &dest, moving)).await?
                };
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
        v
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

    pub fn close_popup(self: &Rc<Self>, name: &str) -> bool {
        let Some(win) = self.find_popup(name) else { return false };
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

    /// Background samplers: desktop entries, audio, GPU (the shell only).
    pub fn start_background(&self) {
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
