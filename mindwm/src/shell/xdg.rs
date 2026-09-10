use std::cell::RefCell;

use smithay::{
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, layer_map_for_output, space::SpaceElement,
        PopupKeyboardGrab, PopupKind, PopupPointerGrab, PopupUngrabStrategy, Space, Window,
        WindowSurfaceType,
    },
    input::{pointer::Focus, Seat},
    output::Output,
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_output, wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Logical, Point, Rectangle, Serial, Size},
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        shell::xdg::{
            Configure, PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};
use tracing::{trace, warn};

use crate::{
    focus::KeyboardFocusTarget,
    shell::{TouchMoveSurfaceGrab, TouchResizeSurfaceGrab},
    state::{AnvilState, Backend},
};

use super::{
    fullscreen_output, place_new_window, FullscreenSurface, PointerMoveSurfaceGrab,
    PointerResizeSurfaceGrab, ResizeData, ResizeEdge, ResizeState, SurfaceData, WindowElement,
};

impl<BackendData: Backend> XdgShellHandler for AnvilState<BackendData> {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured
        let window = WindowElement(Window::new_wayland_window(surface.clone()));
        window.id(); // ids follow creation order
        self.request_repaint();
        place_new_window(&mut self.space, self.pointer.current_location(), &window, true);
        // A new window gets the keyboard right away, no click needed; the
        // layout puts it next to the window that had the focus.
        let previous = self.focused_window();
        self.focus_window(&window);
        *window.tile().output.borrow_mut() = self.space.output_under(self.pointer.current_location()).next().map(|o| o.name());
        self.layout.window_opened(&window, previous);

    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        // Do not send a configure here, the initial configure
        // of a xdg_surface has to be sent during the commit if
        // the surface is not already configured

        self.unconstrain_popup(&surface);

        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            warn!("Failed to track popup: {}", err);
        }
        self.request_repaint();
    }

    fn toplevel_destroyed(&mut self, _surface: ToplevelSurface) {
        self.request_repaint();
    }

    fn popup_destroyed(&mut self, _surface: PopupSurface) {
        self.request_repaint();
    }

    fn title_changed(&mut self, _surface: ToplevelSurface) {
        // The title bar (and the shell's window list) follow at the next turn.
        self.request_repaint();
    }

    fn app_id_changed(&mut self, _surface: ToplevelSurface) {
        self.request_repaint();
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        self.request_repaint();
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<AnvilState<BackendData>> = Seat::from_resource(&seat).unwrap();
        self.move_request_xdg(&surface, &seat, serial)
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat: Seat<AnvilState<BackendData>> = Seat::from_resource(&seat).unwrap();

        // Tiles get their size from the layout.
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if self.layout.is_tiled(&window) && !window.pending_maximized() {
                return;
            }
        }

        if let Some(touch) = seat.get_touch() {
            if touch.has_grab(serial) {
                let start_data = touch.grab_start_data().unwrap();
                tracing::info!(?start_data);

                // If the client disconnects after requesting a move
                // we can just ignore the request
                let Some(window) = self.window_for_surface(surface.wl_surface()) else {
                    tracing::info!("no window");
                    return;
                };

                // If the focus was for a different surface, ignore the request.
                if start_data.focus.is_none()
                    || !start_data
                        .focus
                        .as_ref()
                        .unwrap()
                        .0
                        .same_client_as(&surface.wl_surface().id())
                {
                    tracing::info!("different surface");
                    return;
                }
                let geometry = window.geometry();
                let loc = self.space.element_location(&window).unwrap();
                let (initial_window_location, initial_window_size) = (loc, geometry.size);

                with_states(surface.wl_surface(), move |states| {
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

                let grab = TouchResizeSurfaceGrab {
                    start_data,
                    window,
                    edges: edges.into(),
                    initial_window_location,
                    initial_window_size,
                    last_window_size: initial_window_size,
                };

                touch.set_grab(self, grab, serial);
                return;
            }
        }

        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        let window = self.window_for_surface(surface.wl_surface()).unwrap();

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .same_client_as(&surface.wl_surface().id())
        {
            return;
        }

        let geometry = window.geometry();
        let loc = self.space.element_location(&window).unwrap();
        let (initial_window_location, initial_window_size) = (loc, geometry.size);

        with_states(surface.wl_surface(), move |states| {
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
            window,
            edges: edges.into(),
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    fn ack_configure(&mut self, surface: WlSurface, configure: Configure) {
        if let Configure::Toplevel(configure) = configure {
            if let Some(serial) = with_states(&surface, |states| {
                if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
                    if let ResizeState::WaitingForFinalAck(_, serial) = data.borrow().resize_state {
                        return Some(serial);
                    }
                }

                None
            }) {
                // When the resize grab is released the surface
                // resize state will be set to WaitingForFinalAck
                // and the client will receive a configure request
                // without the resize state to inform the client
                // resizing has finished. Here we will wait for
                // the client to acknowledge the end of the
                // resizing. To check if the surface was resizing
                // before sending the configure we need to use
                // the current state as the received acknowledge
                // will no longer have the resize state set
                let is_resizing = with_states(&surface, |states| {
                    states
                        .data_map
                        .get::<XdgToplevelSurfaceData>()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .current
                        .states
                        .contains(xdg_toplevel::State::Resizing)
                });

                if configure.serial >= serial && is_resizing {
                    with_states(&surface, |states| {
                        let mut data = states
                            .data_map
                            .get::<RefCell<SurfaceData>>()
                            .unwrap()
                            .borrow_mut();
                        if let ResizeState::WaitingForFinalAck(resize_data, _) = data.resize_state {
                            data.resize_state = ResizeState::WaitingForCommit(resize_data);
                        } else {
                            unreachable!()
                        }
                    });
                }
            }

            let window = self
                .space
                .elements()
                .find(|element| element.wl_surface().as_deref() == Some(&surface));
            if let Some(window) = window {
                let is_ssd = crate::shell::ssd::wants_ssd(configure.state.decoration_mode, &window.app_id());
                window.set_ssd(is_ssd);
            }
        }
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, mut wl_output: Option<wl_output::WlOutput>) {
        self.request_repaint();
        if surface
            .current_state()
            .capabilities
            .contains(xdg_toplevel::WmCapabilities::Fullscreen)
        {
            // NOTE: This is only one part of the solution. We can set the
            // location and configure size here, but the surface should be rendered fullscreen
            // independently from its buffer size
            let wl_surface = surface.wl_surface();

            let target = fullscreen_output(wl_surface, wl_output.as_ref(), &self.space)
                .and_then(|output| self.space.output_geometry(&output).map(|geometry| (output, geometry)));

            if let Some((output, geometry)) = target {
                let client = match self.display_handle.get_client(wl_surface.id()) {
                    Ok(client) => client,
                    Err(_) => return,
                };
                wl_output = None;
                for output in output.client_outputs(&client) {
                    wl_output = Some(output);
                }
                let Some(window) = self.window_for_surface(wl_surface) else { return; };
                // Moving an already-fullscreen client must release its old output.
                for old_output in self.space.outputs().filter(|o| *o != &output) {
                    if let Some(fullscreen) = old_output.user_data().get::<FullscreenSurface>() {
                        if fullscreen.get().as_ref() == Some(&window) {
                            fullscreen.clear();
                            self.backend_data.reset_buffers(old_output);
                        }
                    }
                }

                surface.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = Some(geometry.size);
                    state.fullscreen_output = wl_output;
                });
                output.user_data().insert_if_missing(FullscreenSurface::default);
                output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .unwrap()
                    .set(window.clone());
                trace!("Fullscreening: {:?}", window);
            }
        }

        // The protocol demands us to always reply with a configure,
        // regardless of we fulfilled the request or not
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        } else {
            // Will be sent during initial configure
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        self.request_repaint();
        if !surface
            .current_state()
            .states
            .contains(xdg_toplevel::State::Fullscreen)
        {
            return;
        }

        let ret = surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.size = None;
            state.fullscreen_output.take()
        });
        if let Some(output) = ret {
            let output = Output::from_resource(&output).unwrap();
            if let Some(fullscreen) = output.user_data().get::<FullscreenSurface>() {
                trace!("Unfullscreening: {:?}", fullscreen.get());
                fullscreen.clear();
                self.backend_data.reset_buffers(&output);
            }
        }

        surface.send_pending_configure();
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        self.request_repaint();
        // Maximised means "fill the usable area": the output minus the
        // exclusive zones of layer-shell panels.
        if surface
            .current_state()
            .capabilities
            .contains(xdg_toplevel::WmCapabilities::Maximize)
        {
            let Some(window) = self.window_for_surface(surface.wl_surface()) else {
                return;
            };
            let Some(area) = crate::shell::window_output_area(&self.space, &window) else {
                return;
            };
            if !window.pending_maximized() {
                *window.tile().saved.borrow_mut() = self.space.element_geometry(&window);
            }
            let header = window.pending_header_height();
            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Maximized);
                state.size = Some(crate::shell::client_rect(area, header).size);
            });
            self.space.map_element(window, area.loc, true);
            self.layout.dirty = true;
        }

        // The protocol demands us to always reply with a configure,
        // regardless of we fulfilled the request or not
        if surface.is_initial_configure_sent() {
            surface.send_configure();
        } else {
            // Will be sent during initial configure
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        self.request_repaint();
        if !surface
            .current_state()
            .states
            .contains(xdg_toplevel::State::Maximized)
        {
            return;
        }

        surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Maximized);
            state.size = None;
        });
        // A floating window goes back to where it was; a tile gets its slot
        // from the layout on the next turn.
        if let Some(window) = self.window_for_surface(surface.wl_surface()) {
            if !self.layout.is_tiled(&window) {
                let saved = window.tile().saved.borrow_mut().take();
                let on_screen = |r: &Rectangle<i32, Logical>| {
                    self.space
                        .outputs()
                        .any(|o| self.space.output_geometry(o).map(|g| g.overlaps(*r)).unwrap_or(false))
                };
                if let Some(saved) = saved.filter(on_screen) {
                    let header = window.pending_header_height();
                    surface.with_pending_state(|state| {
                        state.size = Some((saved.size.w.max(1), (saved.size.h - header).max(1)).into());
                    });
                    self.space.map_element(window, saved.loc, false);
                }
            }
        }
        self.layout.dirty = true;
        surface.send_pending_configure();
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        self.request_repaint();
        let seat: Seat<AnvilState<BackendData>> = Seat::from_resource(&seat).unwrap();
        let kind = PopupKind::Xdg(surface);
        if let Some(root) = find_popup_root_surface(&kind).ok().and_then(|root| {
            self.space
                .elements()
                .find(|w| w.wl_surface().map(|s| *s == root).unwrap_or(false))
                .cloned()
                .map(KeyboardFocusTarget::from)
                .or_else(|| {
                    self.space
                        .outputs()
                        .find_map(|o| {
                            let map = layer_map_for_output(o);
                            map.layer_for_surface(&root, WindowSurfaceType::TOPLEVEL).cloned()
                        })
                        .map(KeyboardFocusTarget::LayerSurface)
                })
        }) {
            let ret = self.popups.grab_popup(root, kind, &seat, serial);

            if let Ok(mut grab) = ret {
                if let Some(keyboard) = seat.get_keyboard() {
                    if keyboard.is_grabbed()
                        && !(keyboard.has_grab(serial)
                            || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    keyboard.set_focus(self, grab.current_grab(), serial);
                    keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
                }
                if let Some(pointer) = seat.get_pointer() {
                    if pointer.is_grabbed()
                        && !(pointer.has_grab(serial)
                            || pointer.has_grab(grab.previous_serial().unwrap_or_else(|| grab.serial())))
                    {
                        grab.ungrab(PopupUngrabStrategy::All);
                        return;
                    }
                    pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
                }
            }
        }
    }
}

impl<BackendData: Backend> AnvilState<BackendData> {
    pub fn move_request_xdg(&mut self, surface: &ToplevelSurface, seat: &Seat<Self>, serial: Serial) {
        if let Some(touch) = seat.get_touch() {
            if touch.has_grab(serial) {
                let start_data = touch.grab_start_data().unwrap();

                // If the client disconnects after requesting a move
                // we can just ignore the request
                let Some(window) = self.window_for_surface(surface.wl_surface()) else {
                    return;
                };

                // If the focus was for a different surface, ignore the request.
                if start_data.focus.is_none()
                    || !start_data
                        .focus
                        .as_ref()
                        .unwrap()
                        .0
                        .same_client_as(&surface.wl_surface().id())
                {
                    return;
                }

                let mut initial_window_location = self.space.element_location(&window).unwrap();

                // If surface is maximized then unmaximize it
                let current_state = surface.current_state();
                if current_state.states.contains(xdg_toplevel::State::Maximized) {
                    initial_window_location = self.unmaximize_for_drag(surface, &window, start_data.location);
                }
                if self.layout.is_tiled(&window) || window.tile().snap.borrow().is_some() {
                    self.layout.dragging = Some(window.clone());
                }

                let grab = TouchMoveSurfaceGrab {
                    start_data,
                    window,
                    initial_window_location,
                };

                touch.set_grab(self, grab, serial);
                return;
            }
        }

        let pointer = seat.get_pointer().unwrap();

        // Check that this surface has a click grab.
        if !pointer.has_grab(serial) {
            return;
        }

        let start_data = pointer.grab_start_data().unwrap();

        // If the client disconnects after requesting a move
        // we can just ignore the request
        let Some(window) = self.window_for_surface(surface.wl_surface()) else {
            return;
        };

        // If the focus was for a different surface, ignore the request.
        if start_data.focus.is_none()
            || !start_data
                .focus
                .as_ref()
                .unwrap()
                .0
                .same_client_as(&surface.wl_surface().id())
        {
            return;
        }

        let mut initial_window_location = self.space.element_location(&window).unwrap();

        // If surface is maximized then unmaximize it
        let current_state = surface.current_state();
        if current_state.states.contains(xdg_toplevel::State::Maximized) {
            initial_window_location = self.unmaximize_for_drag(surface, &window, pointer.current_location());
        }
        if self.layout.is_tiled(&window) || window.tile().snap.borrow().is_some() {
            self.layout.dragging = Some(window.clone());
        }

        let grab = PointerMoveSurfaceGrab {
            start_data,
            window,
            initial_window_location,
        };

        pointer.set_grab(self, grab, serial, Focus::Clear);
    }

    /// Unmaximise a window at the start of a drag: it takes its saved size
    /// (or lets the client pick one) and hangs from the pointer by its title
    /// bar. Returns where the window's element starts.
    pub(crate) fn unmaximize_for_drag(
        &mut self,
        surface: &ToplevelSurface,
        window: &WindowElement,
        pointer: Point<f64, Logical>,
    ) -> Point<i32, Logical> {
        let saved = window.tile().saved.borrow_mut().take();
        let header = window.pending_header_height();
        let size: Option<Size<i32, Logical>> =
            saved.map(|r| (r.size.w.max(1), (r.size.h - header).max(1)).into());
        surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Maximized);
            state.size = size;
        });
        surface.send_configure();
        self.layout.dirty = true;
        let w = size.map(|s| s.w).unwrap_or(0);
        (pointer.x as i32 - w / 2, pointer.y as i32 - header / 2).into()
    }

    pub(crate) fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some((screen, root_loc)) = self.popup_screen(&root) else {
            return;
        };

        // The target geometry for the positioner should be relative to its parent's geometry, so
        // we will compute that here.
        let mut target = screen;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= root_loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }

    /// The screen area a popup has to stay inside, and where the surface it
    /// hangs off sits in it. A popup belongs either to a window — which may
    /// straddle several outputs, so it gets all of them — or to a layer
    /// surface (a panel, the desktop), which lives on one output and keeps its
    /// popups there.
    ///
    /// Layer surfaces matter as much as windows here: a tooltip on a bottom
    /// panel asks to open below its icon, off the bottom of the screen, and
    /// only gets flipped above it if the compositor answers with an area.
    fn popup_screen(&self, root: &WlSurface) -> Option<(Rectangle<i32, Logical>, Point<i32, Logical>)> {
        if let Some(window) = self.window_for_surface(root) {
            let mut outputs = self.space.outputs_for_element(&window);
            // A union of every output the window is on.
            let mut geo = self.space.output_geometry(&outputs.pop()?)?;
            for output in outputs {
                if let Some(other) = self.space.output_geometry(&output) {
                    geo = geo.merge(other);
                }
            }
            return Some((geo, self.space.element_geometry(&window)?.loc));
        }

        self.space.outputs().find_map(|output| {
            let map = layer_map_for_output(output);
            let layer = map.layer_for_surface(root, WindowSurfaceType::TOPLEVEL)?;
            // A layer's geometry is relative to its output.
            let layer_loc = map.layer_geometry(layer)?.loc;
            let geo = self.space.output_geometry(output)?;
            Some((geo, geo.loc + layer_loc))
        })
    }
}

/// Should be called on `WlSurface::commit` of xdg toplevel
pub(super) fn handle_toplevel_commit(space: &mut Space<WindowElement>, surface: &WlSurface) -> Option<()> {
    let window = space
        .elements()
        .find(|w| w.wl_surface().as_deref() == Some(surface))
        .cloned()?;

    let mut window_loc = space.element_location(&window)?;
    let geometry = window.geometry();

    // MindOS: a dialog is centred over its parent (or the output) on its first commit.
    if let Some(flag) = window.user_data().get::<crate::shell::CenterOnFirstCommit>() {
        if flag.0.get() && geometry.size.w > 0 && geometry.size.h > 0 {
            flag.0.set(false);
            let parent_geo = window
                .0
                .toplevel()
                .and_then(|t| t.parent())
                .and_then(|parent| {
                    space
                        .elements()
                        .find(|w| w.wl_surface().as_deref() == Some(&parent))
                        .and_then(|w| space.element_geometry(w))
                });
            let area = parent_geo.or_else(|| crate::shell::window_output_area(space, &window));
            if let Some(area) = area {
                let mut loc = crate::shell::centered(area, geometry.size);
                if parent_geo.is_none() {
                    loc = crate::shell::cascade(space, area, loc, geometry.size, &window);
                }
                space.map_element(window, loc, false);
                return Some(());
            }
        }
    }

    let new_loc: Point<Option<i32>, Logical> = with_states(window.wl_surface().as_deref()?, |states| {
        let data = states.data_map.get::<RefCell<SurfaceData>>()?.borrow_mut();

        if let ResizeState::Resizing(resize_data) = data.resize_state {
            let edges = resize_data.edges;
            let loc = resize_data.initial_window_location;
            let size = resize_data.initial_window_size;

            // If the window is being resized by top or left, its location must be adjusted
            // accordingly.
            edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                let new_x = edges
                    .intersects(ResizeEdge::LEFT)
                    .then_some(loc.x + (size.w - geometry.size.w));

                let new_y = edges
                    .intersects(ResizeEdge::TOP)
                    .then_some(loc.y + (size.h - geometry.size.h));

                (new_x, new_y).into()
            })
        } else {
            None
        }
    })?;

    if let Some(new_x) = new_loc.x {
        window_loc.x = new_x;
    }
    if let Some(new_y) = new_loc.y {
        window_loc.y = new_y;
    }

    if new_loc.x.is_some() || new_loc.y.is_some() {
        // If TOP or LEFT side of the window got resized, we have to move it
        space.map_element(window, window_loc, false);
    }

    Some(())
}
