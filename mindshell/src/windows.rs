//! Layer-shell windows, one WebKit view each.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use gtk4 as gtk;
use gtk::gdk;
use gtk::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use serde_json::Value;
use webkit6 as webkit;
use webkit::prelude::*;

use crate::layout::Panel;

/// How much a panel window grows toward the screen centre in edit mode
/// (room for the panel settings strip).
pub const EDIT_STRIP: i32 = 140;

/// Gap between the toast stack and the screen corner (logical px).
pub const TOAST_MARGIN: i32 = 12;

/// The toast window's size until the UI measures its content.
pub const TOAST_DEFAULT: (i32, i32) = (380, 1);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Desktop,
    Panel,
    Popup,
    /// An ordinary decorated window (`mindshell --app`).
    App,
    /// The login screen (`mindshell --app greeter`): one full-screen overlay
    /// per output with the keyboard to itself.
    Greeter,
    /// Notification toasts: a small overlay in the top-right corner of the
    /// primary output, sized by the UI (`toast.fit`), hidden when empty.
    Toast,
    /// The screensaver and the lock screen: one full-screen overlay per
    /// output. The compositor knows these windows by their namespace
    /// (`mindshell-lock`) and draws nothing else while the session is locked.
    Lock,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Desktop => "desktop",
            Kind::Panel => "panel",
            Kind::Popup => "popup",
            Kind::App => "app",
            Kind::Greeter => "greeter",
            Kind::Toast => "toast",
            Kind::Lock => "lock",
        }
    }
}

/// The part of a panel definition that decides the window geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct PanelSpec {
    pub edge: String,
    pub size: i32,
    pub length: i32,
    pub align: String,
    pub margin: i32,
    pub layer: String,
    pub autohide: bool,
    /// Exclusive zones of the horizontal panels a vertical panel sits between
    /// (top / bottom), as the UI computes them; 0 for horizontal panels.
    pub inset_start: i32,
    pub inset_end: i32,
    /// For `length == 0` (fit the widgets): the length the UI measured, 0
    /// until it reports one.
    pub fit: i32,
}

impl PanelSpec {
    pub fn is_fit(&self) -> bool {
        self.length <= 0
    }
}

impl From<&Panel> for PanelSpec {
    fn from(p: &Panel) -> Self {
        PanelSpec {
            edge: p.edge.clone(),
            size: p.size,
            length: p.length,
            align: p.align.clone(),
            margin: p.margin,
            layer: p.layer.clone(),
            autohide: p.autohide,
            inset_start: 0,
            inset_end: 0,
            fit: 0,
        }
    }
}

pub struct ShellWindow {
    pub kind: Kind,
    /// Panel id, popup name or "desktop".
    pub id: String,
    /// Connector name of the output this window is on.
    pub output: String,
    pub monitor: gdk::Monitor,
    pub window: gtk::Window,
    pub view: webkit::WebView,
    pub panel: RefCell<Option<PanelSpec>>,
    pub keyboard: Cell<bool>,
    /// Toast window size as measured by the UI (`toast.fit`).
    pub toast: Cell<(i32, i32)>,
    pub ready: Cell<bool>,
    pub url: RefCell<String>,
    /// Popup argument as passed to `popup.open`, echoed in `popup_state`.
    pub arg: RefCell<Value>,
    destroyed: Cell<bool>,
}

impl ShellWindow {
    pub fn new(
        kind: Kind,
        id: &str,
        output: &str,
        monitor: &gdk::Monitor,
        window: gtk::Window,
        view: webkit::WebView,
    ) -> Rc<ShellWindow> {
        if kind == Kind::App {
            // A normal toplevel. GTK would draw its own (Adwaita) title bar, so
            // the window opens undecorated and the compositor gives it the same
            // bar as every other window (it decorates `mindos-*` app ids).
            window.set_decorated(false);
            window.set_default_size(1040, 700);
            window.set_title(Some(id));
        } else {
            window.init_layer_shell();
            window.set_namespace(Some(&format!("mindshell-{}", kind.as_str())));
            window.set_monitor(Some(monitor));
            window.set_decorated(false);
            window.add_css_class("mindshell");
        }
        window.set_child(Some(&view));
        Rc::new(ShellWindow {
            kind,
            id: id.to_string(),
            output: output.to_string(),
            monitor: monitor.clone(),
            window,
            view,
            panel: RefCell::new(None),
            keyboard: Cell::new(false),
            toast: Cell::new(TOAST_DEFAULT),
            ready: Cell::new(false),
            url: RefCell::new(String::new()),
            arg: RefCell::new(Value::Null),
            destroyed: Cell::new(false),
        })
    }

    pub fn monitor_size(&self) -> (i32, i32) {
        let g = self.monitor.geometry();
        (g.width().max(1), g.height().max(1))
    }

    /// Window thickness and length in the panel's own terms.
    fn panel_box(&self, spec: &PanelSpec, edit_mode: bool) -> (i32, i32, bool) {
        let (mw, mh) = self.monitor_size();
        let vertical = spec.edge == "left" || spec.edge == "right";
        let thickness = spec.size + if edit_mode { EDIT_STRIP } else { 0 };
        // Fit-to-content panels stretch out in edit mode so the settings strip has room.
        let full = spec.length >= 100 || (spec.is_fit() && edit_mode);
        let span = (if vertical { mh } else { mw }) - spec.inset_start - spec.inset_end;
        let length = if full {
            span
        } else if spec.is_fit() {
            if spec.fit > 0 { spec.fit.min(span) } else { span / 3 }
        } else {
            ((span * spec.length.clamp(10, 100)) as f64 / 100.0).round() as i32
        };
        (thickness, length.max(1), full)
    }

    /// Apply the layer-shell geometry for this window's kind and state.
    pub fn apply_geometry(&self, edit_mode: bool) {
        let w = &self.window;
        if self.kind == Kind::App {
            return;
        }
        for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
            w.set_anchor(edge, false);
            w.set_margin(edge, 0);
        }
        match self.kind {
            Kind::Desktop => {
                w.set_layer(Layer::Background);
                for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                    w.set_anchor(edge, true);
                }
                w.set_exclusive_zone(-1);
                // The game library has a search field and keyboard navigation.
                // OnDemand focuses it only after the user interacts with it.
                w.set_keyboard_mode(KeyboardMode::OnDemand);
                self.view.set_size_request(-1, -1);
            }
            Kind::Popup => {
                w.set_layer(Layer::Overlay);
                for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                    w.set_anchor(edge, true);
                }
                w.set_exclusive_zone(-1);
                w.set_keyboard_mode(if self.keyboard.get() {
                    KeyboardMode::Exclusive
                } else {
                    KeyboardMode::OnDemand
                });
                self.view.set_size_request(-1, -1);
            }
            Kind::Greeter => {
                w.set_layer(Layer::Overlay);
                for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                    w.set_anchor(edge, true);
                }
                w.set_exclusive_zone(-1);
                // Only the window with the login card takes the keyboard;
                // the others just show the wallpaper.
                w.set_keyboard_mode(if self.keyboard.get() {
                    KeyboardMode::Exclusive
                } else {
                    KeyboardMode::None
                });
                self.view.set_size_request(-1, -1);
            }
            Kind::Lock => {
                w.set_layer(Layer::Overlay);
                for edge in [Edge::Top, Edge::Bottom, Edge::Left, Edge::Right] {
                    w.set_anchor(edge, true);
                }
                w.set_exclusive_zone(-1);
                // The screensaver takes no keyboard at all — the compositor
                // swallows the key that dismisses it. Once the session is
                // locked, the window with the password field takes it.
                w.set_keyboard_mode(if self.keyboard.get() {
                    KeyboardMode::Exclusive
                } else {
                    KeyboardMode::None
                });
                self.view.set_size_request(-1, -1);
            }
            Kind::Toast => {
                w.set_layer(Layer::Overlay);
                w.set_anchor(Edge::Top, true);
                w.set_anchor(Edge::Right, true);
                w.set_margin(Edge::Top, TOAST_MARGIN);
                w.set_margin(Edge::Right, TOAST_MARGIN);
                w.set_exclusive_zone(0);
                w.set_keyboard_mode(KeyboardMode::None);
                let (tw, th) = self.toast.get();
                self.view.set_size_request(tw, th);
                w.set_default_size(tw, th);
            }
            Kind::Panel => {
                let spec = self.panel.borrow().clone().unwrap_or_else(|| PanelSpec::from(&Panel::default()));
                let (thickness, length, full) = self.panel_box(&spec, edit_mode);
                let vertical = spec.edge == "left" || spec.edge == "right";
                let edge = match spec.edge.as_str() {
                    "top" => Edge::Top,
                    "left" => Edge::Left,
                    "right" => Edge::Right,
                    _ => Edge::Bottom,
                };
                w.set_layer(if spec.layer == "bottom" { Layer::Bottom } else { Layer::Top });
                w.set_anchor(edge, true);
                w.set_margin(edge, spec.margin.max(0));
                let (side_a, side_b) = if vertical { (Edge::Top, Edge::Bottom) } else { (Edge::Left, Edge::Right) };
                if full {
                    w.set_anchor(side_a, true);
                    w.set_anchor(side_b, true);
                } else if spec.is_fit() {
                    // No side anchor: the compositor centres the surface along the edge.
                } else {
                    match spec.align.as_str() {
                        "start" => w.set_anchor(side_a, true),
                        "end" => w.set_anchor(side_b, true),
                        _ => {}
                    }
                }
                w.set_exclusive_zone(if spec.autohide { 0 } else { spec.size + spec.margin.max(0) });
                w.set_keyboard_mode(KeyboardMode::None);
                let (req_w, req_h) = if vertical {
                    (thickness, if full { -1 } else { length })
                } else {
                    (if full { -1 } else { length }, thickness)
                };
                self.view.set_size_request(req_w, req_h);
                w.set_default_size(req_w, req_h);
            }
            Kind::App => {}
        }
    }

    /// Where this window's top-left corner sits on its output (logical px).
    pub fn origin(&self, edit_mode: bool) -> (i32, i32) {
        match self.kind {
            Kind::Desktop | Kind::Popup | Kind::App | Kind::Greeter | Kind::Lock => (0, 0),
            Kind::Toast => {
                let (mw, _) = self.monitor_size();
                (mw - TOAST_MARGIN - self.toast.get().0, TOAST_MARGIN)
            }
            Kind::Panel => {
                let spec = self.panel.borrow().clone().unwrap_or_else(|| PanelSpec::from(&Panel::default()));
                let (mw, mh) = self.monitor_size();
                let (thickness, length, full) = self.panel_box(&spec, edit_mode);
                let margin = spec.margin.max(0);
                let along = |extent: i32| -> i32 {
                    let span = extent - spec.inset_start - spec.inset_end;
                    spec.inset_start
                        + if full {
                            0
                        } else if spec.is_fit() {
                            (span - length) / 2
                        } else {
                            match spec.align.as_str() {
                                "start" => 0,
                                "end" => span - length,
                                _ => (span - length) / 2,
                            }
                        }
                };
                match spec.edge.as_str() {
                    "top" => (along(mw), margin),
                    "left" => (margin, along(mh)),
                    "right" => (mw - margin - thickness, along(mh)),
                    _ => (along(mw), mh - margin - thickness),
                }
            }
        }
    }

    pub fn load(&self, url: &str) {
        *self.url.borrow_mut() = url.to_string();
        self.ready.set(false);
        self.view.load_uri(url);
    }

    pub fn eval(&self, js: &str) {
        let id = format!("{}:{}@{}", self.kind.as_str(), self.id, self.output);
        self.view.evaluate_javascript(js, None, None, gtk::gio::Cancellable::NONE, move |r| {
            if let Err(e) = r {
                // Views navigating away or being destroyed produce cancellations; not interesting.
                tracing::debug!(window = id, error = %e, "evaluate_javascript failed");
            }
        });
    }

    /// Unmap the window (the layer surface goes away) while the view and its
    /// page live on, so a closing popup can still talk to the host.
    pub fn hide(&self) {
        self.window.set_visible(false);
    }

    pub fn destroy(&self) {
        if self.destroyed.replace(true) {
            return;
        }
        self.window.set_child(gtk::Widget::NONE);
        self.window.destroy();
    }
}

/// Every monitor of the default display, top-left first.
pub fn monitors() -> Vec<gdk::Monitor> {
    let Some(display) = gdk::Display::default() else { return Vec::new() };
    let model = display.monitors();
    let mut list: Vec<gdk::Monitor> = (0..model.n_items())
        .filter_map(|i| model.item(i).and_then(|o| o.downcast::<gdk::Monitor>().ok()))
        .filter(|m| m.is_valid())
        .collect();
    list.sort_by_key(|m| {
        let g = m.geometry();
        (g.y(), g.x())
    });
    list
}

pub fn monitor_name(m: &gdk::Monitor, index: usize) -> String {
    m.connector()
        .map(|c| c.to_string())
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| format!("output-{index}"))
}

pub fn output_info(m: &gdk::Monitor, name: &str) -> Value {
    let g = m.geometry();
    let refresh = m.refresh_rate();
    serde_json::json!({
        "name": name,
        "make": m.manufacturer().map(|s| s.to_string()),
        "model": m.model().map(|s| s.to_string()),
        "x": g.x(),
        "y": g.y(),
        "width": g.width(),
        "height": g.height(),
        "scale": m.scale(),
        "refresh": if refresh > 0 { Some(refresh as f64 / 1000.0) } else { None },
    })
}

/// Install the CSS that makes the GTK windows transparent (the UI paints
/// its own backgrounds).
pub fn install_css() {
    let Some(display) = gdk::Display::default() else { return };
    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        "window.mindshell, window.mindshell.background, window.mindshell.csd { background: transparent; background-color: transparent; box-shadow: none; border: none; }",
    );
    gtk::style_context_add_provider_for_display(&display, &provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 10);
}
