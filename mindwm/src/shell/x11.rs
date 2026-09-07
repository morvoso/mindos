use std::{cell::RefCell, os::unix::io::OwnedFd};

use smithay::{
    desktop::{space::SpaceElement, Window},
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
        xwm::WmWindowType, X11Surface, X11Wm, XwmHandler,
    },
};
use tracing::{error, trace};

use crate::{focus::KeyboardFocusTarget, state::Backend, AnvilState};

use super::{
    FullscreenSurface, PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData,
    ResizeState, SurfaceData, TouchMoveSurfaceGrab, WindowElement,
};

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

impl<BackendData: Backend> XWaylandShellHandler for AnvilState<BackendData> {
    fn xwayland_shell_state(&mut self) -> &mut XWaylandShellState {
        &mut self.xwayland_shell_state
    }
}

impl<BackendData: Backend> XwmHandler for AnvilState<BackendData> {
    fn xwm_state(&mut self, _xwm: XwmId) -> &mut X11Wm {
        self.xwm.as_mut().unwrap()
    }

    fn new_window(&mut self, _xwm: XwmId, _window: X11Surface) {}
    fn new_override_redirect_window(&mut self, _xwm: XwmId, _window: X11Surface) {}

    fn map_window_request(&mut self, _xwm: XwmId, window: X11Surface) {
        window.set_mapped(true).unwrap();
        let elem = WindowElement(Window::new_x11_window(window.clone()));
        elem.id(); // ids follow creation order
        // Windows that ask to be undecorated (Steam, games, Wine with its own
        // frames) get no bar; everything else gets the MindOS title bar.
        elem.set_ssd(!window.is_decorated());
        let header = elem.header_height();
        let area = crate::shell::pointer_output_area(&self.space, self.pointer.current_location());
        let dialog_like = window.is_popup()
            || window.is_transient_for().is_some()
            || !matches!(window.window_type(), None | Some(WmWindowType::Normal));
        if dialog_like || !self.layout.open_maximized() {
            let size = window.geometry().size;
            let mut size = Size::from((size.w.min(area.size.w).max(1), size.h.min(area.size.h - header).max(1)));
            if size.w < 100 || size.h < 60 {
                // no usable size hint: something reasonable
                size = Size::from((area.size.w * 3 / 5, (area.size.h - header) * 3 / 5));
            }
            let loc = crate::shell::centered(area, (size.w, size.h + header).into());
            self.space.map_element(elem.clone(), loc, true);
            let _ = window.configure(Rectangle::new(loc + Point::from((0, header)), size));
        } else {
            let _ = window.set_maximized(true);
            let client = crate::shell::client_rect(area, header);
            let _ = window.configure(client);
            self.space.map_element(elem.clone(), area.loc, true);
        }
        let previous = self.focused_window();
        self.focus_window(&elem);
        self.layout.window_opened(&elem, previous);
    }

    fn mapped_override_redirect_window(&mut self, _xwm: XwmId, window: X11Surface) {
        let location = window.geometry().loc;
        let window = WindowElement(Window::new_x11_window(window));
        self.space.map_element(window, location, true);
    }

    fn unmapped_window(&mut self, _xwm: XwmId, window: X11Surface) {
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
        if !window.is_override_redirect() {
            window.set_mapped(false).unwrap();
            // The client withdrew the window itself: say so in WM_STATE, or
            // Wine never shows it again (see XTray::withdraw).
            if let Some(tray) = self.xtray.as_ref() {
                tray.withdraw(window.window_id());
            }
        }
        self.refresh_focus();
    }

    fn destroyed_window(&mut self, _xwm: XwmId, window: X11Surface) {
        self.minimized
            .retain(|m| !matches!(m.window.0.x11_surface(), Some(w) if w == &window));
    }

    fn configure_request(
        &mut self,
        _xwm: XwmId,
        window: X11Surface,
        _x: Option<i32>,
        _y: Option<i32>,
        w: Option<u32>,
        h: Option<u32>,
        _reorder: Option<Reorder>,
    ) {
        // we just set the new size, but don't let windows move themselves around freely
        let mut geo = window.geometry();
        if let Some(w) = w {
            geo.size.w = w as i32;
        }
        if let Some(h) = h {
            geo.size.h = h as i32;
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
        // TODO: We don't properly handle the order of override-redirect windows here,
        //       they are always mapped top and then never reordered.
    }

    fn maximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        self.maximize_request_x11(&window);
    }

    fn unmaximize_request(&mut self, _xwm: XwmId, window: X11Surface) {
        let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
            .cloned()
        else {
            return;
        };

        window.set_maximized(false).unwrap();
        if let Some(old_geo) = window
            .user_data()
            .get::<OldGeometry>()
            .and_then(|data| data.restore())
        {
            let header = elem.header_height();
            window
                .configure(Rectangle::new(old_geo.loc + Point::from((0, header)), old_geo.size))
                .unwrap();
            self.space.map_element(elem, old_geo.loc, false);
        }
        self.layout.dirty = true;
    }

    fn fullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
        {
            let outputs_for_window = self.space.outputs_for_element(elem);
            let output = outputs_for_window
                .first()
                // The window hasn't been mapped yet, use the primary output instead
                .or_else(|| self.space.outputs().next())
                // Assumes that at least one output exists
                .expect("No outputs found");
            let geometry = self.space.output_geometry(output).unwrap();

            window.set_fullscreen(true).unwrap();
            window.configure(geometry).unwrap();
            output.user_data().insert_if_missing(FullscreenSurface::default);
            output
                .user_data()
                .get::<FullscreenSurface>()
                .unwrap()
                .set(elem.clone());
            trace!("Fullscreening: {:?}", elem);
        }
    }

    fn unfullscreen_request(&mut self, _xwm: XwmId, window: X11Surface) {
        if let Some(elem) = self
            .space
            .elements()
            .find(|e| matches!(e.0.x11_surface(), Some(w) if w == &window))
        {
            window.set_fullscreen(false).unwrap();
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
                output.user_data().get::<FullscreenSurface>().unwrap().clear();
                let mut rect = self.space.element_bbox(elem).unwrap_or_default();
                rect.loc.y += elem.header_height();
                rect.size.h = (rect.size.h - elem.header_height()).max(1);
                window.configure(rect).unwrap();
                self.backend_data.reset_buffers(output);
            }
        }
    }

    fn resize_request(&mut self, _xwm: XwmId, window: X11Surface, _button: u32, edges: X11ResizeEdge) {
        // luckily anvil only supports one seat anyway...
        let start_data = self.pointer.grab_start_data().unwrap();

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

        let geometry = element.geometry();
        let loc = self.space.element_location(element).unwrap();
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        with_states(&element.wl_surface().unwrap(), move |states| {
            states
                .data_map
                .get::<RefCell<SurfaceData>>()
                .unwrap()
                .borrow_mut()
                .resize_state = ResizeState::Resizing(ResizeData {
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
        self.move_request_x11(&window)
    }

    fn allow_selection_access(&mut self, xwm: XwmId, _selection: SelectionTarget) -> bool {
        if let Some(keyboard) = self.seat.get_keyboard() {
            // check that an X11 window is focused
            if let Some(KeyboardFocusTarget::Window(w)) = keyboard.current_focus() {
                if let Some(surface) = w.x11_surface() {
                    if surface.xwm_id().unwrap() == xwm {
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

impl<BackendData: Backend> AnvilState<BackendData> {
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
        // Fill the usable area (output minus layer-shell exclusive zones).
        let Some(geometry) = crate::shell::window_output_area(&self.space, &elem) else {
            return;
        };

        let header = elem.header_height();
        window.set_maximized(true).unwrap();
        window
            .configure(crate::shell::client_rect(geometry, header))
            .unwrap();
        window.user_data().insert_if_missing(OldGeometry::default);
        // The element rectangle (bar included), restored by unmaximize.
        window.user_data().get::<OldGeometry>().unwrap().save(Rectangle::new(
            old_geo.loc,
            (old_geo.size.w, old_geo.size.h - header).into(),
        ));
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
                    let mut initial_window_location = self.space.element_location(element).unwrap();

                    // If surface is maximized then unmaximize it
                    if window.is_maximized() {
                        window.set_maximized(false).unwrap();
                        let pos = start_data.location;
                        initial_window_location = (pos.x as i32, pos.y as i32).into();
                        if let Some(old_geo) = window
                            .user_data()
                            .get::<OldGeometry>()
                            .and_then(|data| data.restore())
                        {
                            let header = element.header_height();
                            window
                                .configure(Rectangle::new(
                                    initial_window_location + Point::from((0, header)),
                                    old_geo.size,
                                ))
                                .unwrap();
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

        let mut initial_window_location = self.space.element_location(element).unwrap();

        // If surface is maximized then unmaximize it
        if window.is_maximized() {
            window.set_maximized(false).unwrap();
            let pos = self.pointer.current_location();
            initial_window_location = (pos.x as i32, pos.y as i32).into();
            if let Some(old_geo) = window
                .user_data()
                .get::<OldGeometry>()
                .and_then(|data| data.restore())
            {
                let header = element.header_height();
                window
                    .configure(Rectangle::new(
                        initial_window_location + Point::from((0, header)),
                        old_geo.size,
                    ))
                    .unwrap();
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
