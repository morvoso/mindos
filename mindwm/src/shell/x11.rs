use std::{cell::RefCell, os::unix::io::OwnedFd};

use smithay::{
    desktop::Window,
    input::pointer::Focus,
    utils::{Logical, Point, Rectangle, Size, SERIAL_COUNTER},
    wayland::{
        compositor::with_states,
        selection::{
            data_device::{
                clear_data_device_selection, current_data_device_selection_userdata,
                request_data_device_client_selection, set_data_device_selection,
            },
            primary_selection::{
                clear_primary_selection, current_primary_selection_userdata,
                request_primary_client_selection, set_primary_selection,
            },
            SelectionTarget,
        },
        xwayland_shell::{XWaylandShellHandler, XWaylandShellState},
    },
    xwayland::{
        xwm::{Reorder, ResizeEdge as X11ResizeEdge, XwmId},
        xwm::{WmWindowProperty, WmWindowType}, X11Surface, X11Wm, XwmHandler,
    },
};
use tracing::{error, trace, warn};

use crate::{focus::KeyboardFocusTarget, state::Backend, AnvilState};

use super::{
    FullscreenSurface, PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData,
    ResizeState, SurfaceData, TouchMoveSurfaceGrab, WindowElement,
};

/// Window types that never take the keyboard when they appear, and never take
/// the screen either: a splash screen, a notification, a tooltip, a menu the
/// client tore off itself.
fn decorative(window: &X11Surface) -> bool {
    matches!(
        window.window_type(),
        Some(
            WmWindowType::Notification
                | WmWindowType::Tooltip
                | WmWindowType::Splash
                | WmWindowType::DropdownMenu
                | WmWindowType::PopupMenu
        )
    )
}

#[derive(Debug, Default)]
struct OldGeometry(RefCell<Option<Rectangle<i32, Logical>>>);
impl OldGeometry {
    pub fn save(&self, geo: Rectangle<i32, Logical>) {
        *self.0.borrow_mut() = Some(geo);
    }

    pub fn restore(&self) -> Option<Rectangle<i32, Logical>> {
        self.0.borrow_mut().take()
    }
}

/// An X11 request Xwayland would not carry out.
///
/// Every one of these is a round trip to another process, and that process
/// can die: a game that takes Xwayland down with it used to take the whole
/// session with it, because the error came back through an `unwrap` on the
/// compositor's only thread. A window that cannot be told what to do is one
/// window; a compositor that exits is every window, every other client, and
/// the desktop the user was in the middle of something on.
fn xwayland_refused(what: &str, err: impl std::fmt::Debug) {
    warn!("Xwayland would not {what}: {err:?}");
}

impl<BackendData: Backend> XWaylandShellHandler for AnvilState<BackendData> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl<BackendData: Backend> XwmHandler for AnvilState<BackendData> {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        // The trait has to hand back a window manager, so there is no
        // graceful answer here. Every call comes from the XWM dispatching an
        // event it read from the connection this owns: no connection, no
        // event, no call. Taking `xwm` is the one thing that must not happen
        // while its events are being handled.
        self.xwm.as_mut().expect("the X11 window manager handles its own events")
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        if let Err(err) = window.set_mapped(true) {
            xwayland_refused("map a window", err);
        }
        let elem = WindowElement(Window::new_x11_window(window.clone()));
        elem.id(); // ids follow creation order
        // Windows that ask to be undecorated (Steam, games, Wine with its own
        // frames) get no bar; everything else gets the MindOS title bar.
        elem.set_ssd(!window.is_decorated());
        let header = elem.header_height();

        // What the client asked for before anyone mapped its window. Smithay's
        // window manager reads none of these, and they decide both of the
        // things that happen next: the screen and the keyboard.
        let props = self
            .xprops
            .as_ref()
            .map(|x| x.at_map(window.window_id()))
            .unwrap_or_default();
        // ICCCM's input models and EWMH's user time: a window may say that it
        // does not want the keyboard -- Wine's helper windows do, and so does
        // anything that maps itself in the background -- and a splash screen
        // or a notification never wants it.
        let wants_focus = !decorative(&window)
            && !props.no_focus
            && (window.hints().and_then(|h| h.input).unwrap_or(true) || props.take_focus);

        // ICCCM: a client may state where it wants to sit through WM_NORMAL_HINTS,
        // and a user-specified position is meant to be honoured. Steam positions
        // its Settings window that way, over the main Steam window. Placing it by
        // the pointer instead drops it on whichever monitor the pointer happens to
        // be on -- routinely not the one Steam itself is on, so the window looks
        // like it never opened.
        let requested = requested_client_loc(&window)
            .filter(|p| self.space.output_under(p.to_f64()).next().is_some());
        let parent = self.x11_parent(&window);
        let area = requested
            .map(|p| crate::shell::pointer_output_area(&self.space, p.to_f64()))
            .or_else(|| {
                parent
                    .as_ref()
                    .and_then(|e| crate::shell::window_output_area(&self.space, e))
            })
            .unwrap_or_else(|| {
                crate::shell::pointer_output_area(&self.space, self.pointer.current_location())
            });

        // A window that cannot be resized, and the Windows desktop a setup
        // program runs in, both open at the size they asked for. See
        // `WindowElement::is_dialog`, which keeps them out of the tiling too.
        let dialog_like = window.is_popup()
            || window.is_transient_for().is_some()
            || !matches!(window.window_type(), None | Some(WmWindowType::Normal))
            || elem.is_fixed_size()
            || elem.is_setup_desktop();
        // A game in borderless fullscreen says so in _NET_WM_STATE before it
        // maps, or (older Wine, and anything that sizes itself to the monitor)
        // opens undecorated at exactly one display's rectangle. Maximising it
        // to the usable area first would have it pick its render size from a
        // rectangle smaller than the screen and keep drawing at that size once
        // it is made fullscreen after all.
        // Wine, Proton and SDL put _NET_WM_STATE_FULLSCREEN on the window
        // themselves and never send the client message that asks for it; the
        // vendored smithay reads the property before the map, and mindwm's own
        // reader (`xprops`) covers a window whose property arrived late.
        let fullscreen_output = if (window.is_fullscreen() || props.fullscreen) && !decorative(&window) {
            self.x11_fullscreen_output(&window).or_else(|| {
                self.space.output_under(self.pointer.current_location()).next().cloned()
            })
        } else if !window.is_decorated()
            && !window.is_popup()
            && window.is_transient_for().is_none()
            && matches!(window.window_type(), None | Some(WmWindowType::Normal))
        {
            let geo = window.geometry();
            self.space
                .outputs()
                .find(|o| self.space.output_geometry(o) == Some(geo))
                .cloned()
        } else {
            None
        };
        if let Some(output) = fullscreen_output.filter(|o| self.space.output_geometry(o).is_some()) {
            let rect = self.space.output_geometry(&output).unwrap();
            self.space.map_element(elem.clone(), rect.loc, true);
            self.fullscreen_x11(&elem, &window, &output);
        } else if dialog_like || !self.layout.open_maximized() {
            let size = window.geometry().size;
            let mut size = Size::from((size.w.min(area.size.w).max(1), size.h.min(area.size.h - header).max(1)));
            if size.w < 100 || size.h < 60 {
                // no usable size hint: something reasonable
                size = Size::from((area.size.w * 3 / 5, (area.size.h - header) * 3 / 5));
            }
            let full = Size::from((size.w, size.h + header));
            // Where it asked to be, else centred on its parent, else centred.
            let loc = requested
                .map(|p| p - Point::from((0, header)))
                .or_else(|| {
                    parent
                        .as_ref()
                        .and_then(|e| self.space.element_bbox(e))
                        .map(|b| crate::shell::centered(b, full))
                })
                .map(|p| clamp_into(area, p, full))
                .unwrap_or_else(|| crate::shell::centered(area, full));
            self.space.map_element(elem.clone(), loc, true);
            let _ = window.configure(Rectangle::new(loc + Point::from((0, header)), size));
        } else {
            let _ = window.set_maximized(true);
            let client = crate::shell::client_rect(area, header);
            let _ = window.configure(client);
            self.space.map_element(elem.clone(), area.loc, true);
        }
        let previous = self.focused_window();
        // The window may have been placed on an output other than the pointer's.
        // Settled before the keyboard question, which asks which screen it is on.
        *elem.tile().output.borrow_mut() = self
            .space
            .outputs_for_element(&elem)
            .first()
            .map(|o| o.name())
            .or_else(|| {
                self.space
                    .output_under(self.pointer.current_location())
                    .next()
                    .map(|o| o.name())
            });
        if wants_focus {
            self.focus_new_window(&elem);
        }
        self.layout.window_opened(&elem, previous);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        let location = window.geometry().loc;
        let window = WindowElement(Window::new_x11_window(window));
        self.space.map_element(window, location, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        let maybe = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
            .cloned();
        if let Some(elem) = maybe {
            self.space.unmap_elem(&elem)
        }
        self.layout.dirty = true;
        // A minimised window that its client unmaps is gone for the shell too.
        self.minimized
            .retain(|m| !matches!(m.window.0.x11_surface(), Some(w) if w == &window));
        self.desks
            .stashed
            .retain(|m| !matches!(m.window.0.x11_surface(), Some(w) if w == &window));
        if !window.is_override_redirect() {
            if let Err(err) = window.set_mapped(false) {
                xwayland_refused("unmap a window", err);
            }
            // The client withdrew the window itself: say so in WM_STATE, or
            // Wine never shows it again (see XTray::withdraw).
            if let Some(tray) = self.xtray.as_ref() {
                tray.withdraw(window.window_id());
            }
        }
        self.refresh_focus();
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.request_repaint();
        self.minimized
            .retain(|m| !matches!(m.window.0.x11_surface(), Some(w) if w == &window));
        self.desks
            .stashed
            .retain(|m| !matches!(m.window.0.x11_surface(), Some(w) if w == &window));
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        x: Option<i32>,
        y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        // A fullscreen window covers its display, whatever size it asks for:
        // answer with the rectangle it has, so it redraws at that size
        // instead of believing it now sits in a corner of the screen.
        if window.is_fullscreen() {
            let _ = window.configure(window.geometry());
            return;
        }
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
        }
        // A tiled, maximised or fullscreen window is placed by the shell, so its
        // position is not the client's to pick. A floating one may move itself:
        // dropping the request silently leaves the client believing it sits
        // where it asked to be, which misplaces anything it positions relative
        // to that window and offsets its own input handling.
        let elem = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(s) if s == &window))
            .cloned();
        let shell_placed = window.is_maximized()
            || window.is_fullscreen()
            || elem.as_ref().is_some_and(|e| self.layout.is_tiled(e));
        if !shell_placed {
            if let Some(x) = x {
                geo.loc.x = x;
            }
            if let Some(y) = y {
                geo.loc.y = y;
            }
        }
        let _ = window.configure(geo);
    }

    fn configure_notify(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        geometry: Rectangle<i32, Logical>,
        _above: Option<u32>,
    ) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
            .cloned()
        else {
            return;
        };
        let loc = geometry.loc - Point::from((0, elem.header_height()));
        if self.space.element_location(&elem) != Some(loc) {
            self.space.map_element(elem, loc, false);
        }
        self.request_repaint();
        // TODO: We don't properly handle the order of override-redirect windows here,
        //       they are always mapped top and then never reordered.
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        self.maximize_request_x11(&window);
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
            .cloned()
        else {
            return;
        };

        if let Err(err) = window.set_maximized(false) {
            xwayland_refused("unmaximize a window", err);
        }
        if let Some(old_geo) = window
            .user_data()
            .get::<OldGeometry>()
            .and_then(|data| data.restore())
        {
            let header = elem.header_height();
            if let Err(err) =
                window.configure(Rectangle::new(old_geo.loc + Point::from((0, header)), old_geo.size))
            {
                xwayland_refused("restore a window's geometry", err);
            }
            self.space.map_element(elem, old_geo.loc, false);
        }
        self.layout.dirty = true;
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
            .cloned()
        else {
            return;
        };
        let Some(output) = self
            .x11_fullscreen_output(&window)
            .or_else(|| self.space.outputs_for_element(&elem).first().cloned())
            // The window hasn't been mapped yet, use the primary output instead
            .or_else(|| self.space.outputs().next().cloned())
        else {
            // Every display was unplugged, or the last one went away
            // between this request being sent and being read. There is
            // nowhere to be fullscreen; the window stays as it is.
            warn!("a window asked to be fullscreen while no display is connected");
            return;
        };
        self.fullscreen_x11(&elem, &window, &output);
    }

    fn active_window_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _timestamp: u32,
        currently_active_window: Option<X11Surface>,
    ) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
            .cloned()
        else {
            return;
        };
        // A client may move the focus between its own windows (Wine does, for
        // every window of a game that makes one foreground), but not take it
        // from another program the user is typing into.
        let focused = self.focused_window();
        let focused_x11 = focused.as_ref().and_then(|f| f.0.x11_surface().cloned());
        let granted = match (&focused, &focused_x11) {
            (None, _) => true,
            (Some(f), _) if *f == elem => true,
            (Some(_), None) => false,
            (Some(_), Some(current)) => {
                currently_active_window.as_ref() == Some(current)
                    || (current.pid().is_some() && current.pid() == window.pid())
                    || (crate::procinfo::is_wine(current.pid()) && crate::procinfo::is_wine(window.pid()))
            }
        };
        if !granted {
            trace!(?window, "refused a _NET_ACTIVE_WINDOW request from a background client");
            return;
        }
        self.space.raise_element(&elem, true);
        if let Some(xwm) = self.xwm.as_mut() {
            let _ = xwm.raise_window(&window);
        }
        self.focus_window(&elem);
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.request_repaint();
        if let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
        {
            if let Err(err) = window.set_fullscreen(false) {
                xwayland_refused("leave fullscreen", err);
            }
            elem.set_ssd(!window.is_decorated());
            self.layout.dirty = true;
            if let Some(output) = self.space.outputs().find(|o| {
                o.user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|f| f.get())
                    .map(|w| &w == elem)
                    .unwrap_or(false)
            }) {
                trace!("Unfullscreening: {:?}", elem);
                if let Some(fullscreen) = output.user_data().get::<FullscreenSurface>() {
                    fullscreen.clear();
                }
                let mut rect = self.space.element_bbox(elem).unwrap_or_default();
                rect.loc.y += elem.header_height();
                rect.size.h = (rect.size.h - elem.header_height()).max(1);
                if let Err(err) = window.configure(rect) {
                    xwayland_refused("take back its own geometry", err);
                }
                self.backend_data.reset_buffers(output);
            }
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edges: X11ResizeEdge) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        // luckily anvil only supports one seat anyway...
        // A client asks to be resized when the user drags its edge, but
        // nothing stops it asking at any other time, and by then the button
        // may already be up. No grab is an answer, not a reason to exit.
        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };

        let Some(element) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
        else {
            return;
        };
        // Tiles get their size from the layout.
        if self.layout.is_tiled(element) && !window.is_maximized() {
            return;
        }

        let Some(loc) = self.space.element_location(element) else {
            return;
        };
        // the grab sizes the client; the title bar rides on top of it
        let (initial_window_location, initial_window_size) = (loc, element.0.geometry().size);

        // An X11 window has a wl_surface only once Xwayland has associated
        // one, and the resize state only once that surface has committed.
        // Asking to be resized before either is a client that is early, not
        // a compositor that should stop.
        let Some(wl_surface) = element.wl_surface() else {
            return;
        };
        with_states(&wl_surface, move |states| {
            let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() else {
                return;
            };
            data.borrow_mut().resize_state = ResizeState::Resizing(ResizeData {
                edges: edges.into(),
                initial_window_location,
                initial_window_size,
            });
        });

        let grab = PointerResizeSurfaceGrab {
            start_data,
            window: element.clone(),
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }

    fn move_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32) {
        let _phase = crate::stall::enter(crate::stall::Phase::XWayland);
        self.move_request_x11(&window)
    }

    fn property_notify(&mut self, _xwm: XwmId, window: X11Surface, property: WmWindowProperty) {
        // A window can take its own title bar after it is already on screen:
        // a WPF program adopts its custom chrome once the .NET UI is up, and
        // Wine only then tells the window manager to stop decorating it. The
        // bar chosen when the window was mapped would sit above the app's own
        // one -- two title bars on one window.
        if matches!(property, WmWindowProperty::MotifHints) {
            let elem = self
                .space
                .elements()
                .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
                .cloned();
            if let Some(elem) = elem {
                let ssd = !window.is_decorated();
                if elem.decoration_state().is_ssd != ssd {
                    elem.set_ssd(ssd);
                    self.layout.dirty = true;
                    // The program keeps the window it drew into; only the space
                    // above it comes or goes, the same move as leaving
                    // fullscreen.
                    let mut rect = self.space.element_bbox(&elem).unwrap_or_default();
                    let loc = rect.loc;
                    rect.loc.y += elem.header_height();
                    rect.size.h = (rect.size.h - elem.header_height()).max(1);
                    let _ = window.configure(rect);
                    // The bar that just went away leaves its pixels behind
                    // otherwise: the frame's shadow reaches well outside the
                    // window's own rectangle, so the strip it used to cover is
                    // not damaged by the window moving or resizing alone.
                    self.space.map_element(elem.clone(), loc, false);
                    for output in self.space.outputs_for_element(&elem) {
                        self.backend_data.reset_buffers(&output);
                    }
                }
            }
        }
        // A new title or class: the title bar and the shell's window list
        // pick it up at the next turn.
        self.request_repaint();
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        if let Some(keyboard) = self.seat.get_keyboard() {
            // check that an X11 window is focused
            if let Some(KeyboardFocusTarget::Window(w)) = keyboard.current_focus() {
                if let Some(surface) = w.x11_surface() {
                    if surface.xwm_id() == Some(xwm) {
                        return true;
                    }
                }
            }
        }
        false
    }

    fn send_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_type: String, fd: OwnedFd) {
        match selection {
            SelectionTarget::Clipboard => {
                if let Err(err) = request_data_device_client_selection(&self.seat, mime_type, fd) {
                    error!(?err, "Failed to request current wayland clipboard for Xwayland",);
                }
            }
            SelectionTarget::Primary => {
                if let Err(err) = request_primary_client_selection(&self.seat, mime_type, fd) {
                    error!(
                        ?err,
                        "Failed to request current wayland primary selection for Xwayland",
                    );
                }
            }
        }
    }

    fn new_selection(&mut self, _xwm: XwmId, selection: SelectionTarget, mime_types: Vec<String>) {
        trace!(?selection, ?mime_types, "Got Selection from X11",);
        // TODO check, that focused windows is X11 window before doing this
        match selection {
            SelectionTarget::Clipboard => {
                set_data_device_selection(&self.display_handle, &self.seat, mime_types, ())
            }
            SelectionTarget::Primary => {
                set_primary_selection(&self.display_handle, &self.seat, mime_types, ())
            }
        }
    }

    fn cleared_selection(&mut self, _xwm: XwmId, selection: SelectionTarget) {
        match selection {
            SelectionTarget::Clipboard => {
                if current_data_device_selection_userdata(&self.seat).is_some() {
                    clear_data_device_selection(&self.display_handle, &self.seat)
                }
            }
            SelectionTarget::Primary => {
                if current_primary_selection_userdata(&self.seat).is_some() {
                    clear_primary_selection(&self.display_handle, &self.seat)
                }
            }
        }
    }
}

/// The position a client asked for in WM_NORMAL_HINTS, if it gave one.
fn requested_client_loc(window: &X11Surface) -> Option<Point<i32, Logical>> {
    let (_, x, y) = window.size_hints()?.position?;
    Some(Point::from((x, y)))
}

/// Keep a window of `size` placed at `loc` inside `area`.
fn clamp_into(
    area: Rectangle<i32, Logical>,
    loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
) -> Point<i32, Logical> {
    let max_x = area.loc.x + (area.size.w - size.w).max(0);
    let max_y = area.loc.y + (area.size.h - size.h).max(0);
    Point::from((loc.x.clamp(area.loc.x, max_x), loc.y.clamp(area.loc.y, max_y)))
}

impl<BackendData: Backend> AnvilState<BackendData> {
    /// The mapped window a transient names in WM_TRANSIENT_FOR.
    pub fn x11_parent(&self, window: &X11Surface) -> Option<WindowElement> {
        let parent = window.is_transient_for()?;
        self.space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(s) if s.window_id() == parent))
            .cloned()
    }

    /// The display an X11 window asking for fullscreen means: the one its
    /// rectangle covers exactly (Wine places the window on the monitor it
    /// wants before asking), else the one under the middle of it.
    fn x11_fullscreen_output(&self, window: &X11Surface) -> Option<smithay::output::Output> {
        let geo = window.geometry();
        self.space
            .outputs()
            .find(|o| self.space.output_geometry(o) == Some(geo))
            .or_else(|| {
                let centre = geo.loc + Point::from((geo.size.w / 2, geo.size.h / 2));
                self.space.output_under(centre.to_f64()).next()
            })
            .cloned()
    }

    /// Make an X11 window cover `output`: no bar, not maximised, at the
    /// display's rectangle, and drawn by the fullscreen path of that output.
    fn fullscreen_x11(&mut self, elem: &WindowElement, window: &X11Surface, output: &smithay::output::Output) {
        let Some(geometry) = self.space.output_geometry(output) else {
            warn!("a window asked to be fullscreen on a display that is not mapped");
            return;
        };
        if window.is_maximized() {
            if let Err(err) = window.set_maximized(false) {
                xwayland_refused("unmaximize a window", err);
            }
        }
        if let Err(err) = window.set_fullscreen(true) {
            xwayland_refused("go fullscreen", err);
            return;
        }
        if let Err(err) = window.configure(geometry) {
            xwayland_refused("take the fullscreen geometry", err);
            return;
        }
        if self.space.element_location(elem) != Some(geometry.loc - Point::from((0, elem.header_height()))) {
            self.space.map_element(elem.clone(), geometry.loc - Point::from((0, elem.header_height())), true);
        }
        *elem.tile().output.borrow_mut() = Some(output.name());
        // Asking for fullscreen from behind a game does not take the screen
        // from it; the window is fullscreen for when it is next looked at. (A
        // game asking for it has the keyboard, and passes.)
        if !self.may_interrupt_fullscreen(elem) {
            trace!("Fullscreen behind a fullscreen window: {:?}", elem);
            return;
        }
        output.user_data().insert_if_missing(FullscreenSurface::default);
        if let Some(fullscreen) = output.user_data().get::<FullscreenSurface>() {
            fullscreen.set(elem.clone());
        }
        self.layout.dirty = true;
        trace!("Fullscreening: {:?}", elem);
    }

    pub fn maximize_request_x11(&mut self, window: &X11Surface) {
        let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == window))
            .cloned()
        else {
            return;
        };

        let Some(old_geo) = self.space.element_bbox(&elem) else {
            return;
        };
        // Fill the usable area (output minus the layer-shell exclusive zones
        // and the desktop bar, which a game keeps).
        let Some(geometry) = self.maximized_area(&elem) else {
            return;
        };

        let header = elem.header_height();
        if let Err(err) = window.set_maximized(true) {
            xwayland_refused("maximize a window", err);
            return;
        }
        if let Err(err) = window.configure(crate::shell::client_rect(geometry, header)) {
            xwayland_refused("take the maximized geometry", err);
            return;
        }
        window.user_data().insert_if_missing(OldGeometry::default);
        // The element rectangle (bar included), restored by unmaximize.
        if let Some(old) = window.user_data().get::<OldGeometry>() {
            old.save(Rectangle::new(
                old_geo.loc,
                (old_geo.size.w, old_geo.size.h - header).into(),
            ));
        }
        self.space.map_element(elem, geometry.loc, false);
        self.layout.dirty = true;
    }

    pub fn move_request_x11(&mut self, window: &X11Surface) {
        if let Some(touch) = self.seat.get_touch() {
            if let Some(start_data) = touch.grab_start_data() {
                let element = self
                    .space
                    .elements()
                    .find(|e| matches!(e.0.x11_surface(), Some(w) if w == window));

                if let Some(element) = element {
                    let Some(mut initial_window_location) = self.space.element_location(element)
                    else {
                        return;
                    };

                    // If surface is maximized then unmaximize it
                    if window.is_maximized() {
                        if let Err(err) = window.set_maximized(false) {
                            xwayland_refused("unmaximize a window", err);
                        }
                        let pos = start_data.location;
                        initial_window_location = (pos.x as i32, pos.y as i32).into();
                        if let Some(old_geo) = window
                            .user_data()
                            .get::<OldGeometry>()
                            .and_then(|data| data.restore())
                        {
                            let header = element.header_height();
                            if let Err(err) = window.configure(Rectangle::new(
                                initial_window_location + Point::from((0, header)),
                                old_geo.size,
                            )) {
                                xwayland_refused("take its restored geometry", err);
                            }
                        }
                    }

                    if self.layout.is_tiled(element) {
                        self.layout.dragging = Some(element.clone());
                    }
                    let grab = TouchMoveSurfaceGrab {
                        start_data,
                        window: element.clone(),
                        initial_window_location,
                    };

                    touch.set_grab(self, grab, SERIAL_COUNTER.next_serial());
                    return;
                }
            }
        }

        // luckily anvil only supports one seat anyway...
        let Some(start_data) = self.pointer.grab_start_data() else {
            return;
        };

        let Some(element) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == window))
        else {
            return;
        };

        let Some(mut initial_window_location) = self.space.element_location(element) else {
            return;
        };

        // If surface is maximized then unmaximize it
        if window.is_maximized() {
            if let Err(err) = window.set_maximized(false) {
                xwayland_refused("unmaximize a window", err);
            }
            let pos = self.pointer.current_location();
            initial_window_location = (pos.x as i32, pos.y as i32).into();
            if let Some(old_geo) = window
                .user_data()
                .get::<OldGeometry>()
                .and_then(|data| data.restore())
            {
                let header = element.header_height();
                if let Err(err) = window.configure(Rectangle::new(
                    initial_window_location + Point::from((0, header)),
                    old_geo.size,
                )) {
                    xwayland_refused("take its restored geometry", err);
                }
            }
        }

        if self.layout.is_tiled(element) {
            self.layout.dragging = Some(element.clone());
        }
        let grab = PointerMoveSurfaceGrab {
            start_data,
            window: element.clone(),
            initial_window_location,
        };

        let pointer = self.pointer.clone();
        pointer.set_grab(self, grab, SERIAL_COUNTER.next_serial(), Focus::Clear);
    }
}
