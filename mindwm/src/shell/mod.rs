use std::cell::{Cell, RefCell};

#[cfg(feature = "xwayland")]
use smithay::xwayland::XWaylandClientData;

#[cfg(feature = "udev")]
use smithay::wayland::drm_syncobj::DrmSyncobjCachedState;

use smithay::{
    backend::renderer::utils::on_commit_buffer_handler,
    desktop::{
        layer_map_for_output, space::SpaceElement, LayerSurface, PopupKind, PopupManager, Space,
        WindowSurfaceType,
    },
    input::pointer::{CursorImageStatus, CursorImageSurfaceData},
    output::Output,
    reexports::{
        calloop::Interest,
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_buffer::WlBuffer, wl_output, wl_surface::WlSurface},
            Client, Resource,
        },
    },
    utils::{IsAlive, Logical, Point, Rectangle, Size},
    wayland::{
        buffer::BufferHandler,
        compositor::{
            add_blocker, add_pre_commit_hook, get_parent, is_sync_subsurface, with_states,
            with_surface_tree_upward, BufferAssignment, CompositorClientState, CompositorHandler,
            CompositorState, SurfaceAttributes, TraversalAction,
        },
        dmabuf::get_dmabuf,
        shell::{
            wlr_layer::{
                Layer, LayerSurface as WlrLayerSurface, LayerSurfaceData, WlrLayerShellHandler,
                WlrLayerShellState,
            },
            xdg::{ToplevelSurface, XdgToplevelSurfaceData},
        },
    },
};

use crate::{
    state::{AnvilState, Backend},
    ClientState,
};

mod element;
mod grabs;
pub(crate) mod ssd;
#[cfg(feature = "xwayland")]
mod x11;
mod xdg;

pub use self::element::*;
pub use self::grabs::*;

fn fullscreen_output_geometry(
    wl_surface: &WlSurface,
    wl_output: Option<&wl_output::WlOutput>,
    space: &mut Space<WindowElement>,
) -> Option<Rectangle<i32, Logical>> {
    // First test if a specific output has been requested
    // if the requested output is not found ignore the request
    wl_output
        .and_then(Output::from_resource)
        .or_else(|| {
            let w = space
                .elements()
                .find(|window| window.wl_surface().map(|s| &*s == wl_surface).unwrap_or(false));
            w.and_then(|w| space.outputs_for_element(w).first().cloned())
        })
        .as_ref()
        .and_then(|o| space.output_geometry(o))
}

#[derive(Default)]
pub struct FullscreenSurface(RefCell<Option<WindowElement>>);

impl FullscreenSurface {
    pub fn set(&self, window: WindowElement) {
        *self.0.borrow_mut() = Some(window);
    }

    pub fn get(&self) -> Option<WindowElement> {
        let mut window = self.0.borrow_mut();
        if window.as_ref().map(|w| !w.alive()).unwrap_or(false) {
            *window = None;
        }
        window.clone()
    }

    pub fn clear(&self) -> Option<WindowElement> {
        self.0.borrow_mut().take()
    }
}

impl<BackendData: Backend> BufferHandler for AnvilState<BackendData> {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl<BackendData: Backend> CompositorHandler for AnvilState<BackendData> {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        #[cfg(feature = "xwayland")]
        if let Some(state) = client.get_data::<XWaylandClientData>() {
            return &state.compositor_state;
        }
        if let Some(state) = client.get_data::<ClientState>() {
            return &state.compositor_state;
        }
        panic!("Unknown client data type")
    }

    fn new_surface(&mut self, surface: &WlSurface) {
        add_pre_commit_hook::<Self, _>(surface, move |state, _dh, surface| {
            #[cfg(feature = "udev")]
            let mut acquire_point = None;
            let maybe_dmabuf = with_states(surface, |surface_data| {
                #[cfg(feature = "udev")]
                acquire_point.clone_from(
                    &surface_data
                        .cached_state
                        .get::<DrmSyncobjCachedState>()
                        .pending()
                        .acquire_point,
                );
                surface_data
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .pending()
                    .buffer
                    .as_ref()
                    .and_then(|assignment| match assignment {
                        BufferAssignment::NewBuffer(buffer) => get_dmabuf(buffer).cloned().ok(),
                        _ => None,
                    })
            });
            if let Some(dmabuf) = maybe_dmabuf {
                #[cfg(feature = "udev")]
                if let Some(acquire_point) = acquire_point {
                    if let Ok((blocker, source)) = acquire_point.generate_blocker() {
                        let client = surface.client().unwrap();
                        let res = state.handle.insert_source(source, move |_, _, data| {
                            let dh = data.display_handle.clone();
                            data.client_compositor_state(&client).blocker_cleared(data, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                            return;
                        }
                    }
                }
                if let Ok((blocker, source)) = dmabuf.generate_blocker(Interest::READ) {
                    if let Some(client) = surface.client() {
                        let res = state.handle.insert_source(source, move |_, _, data| {
                            let dh = data.display_handle.clone();
                            data.client_compositor_state(&client).blocker_cleared(data, &dh);
                            Ok(())
                        });
                        if res.is_ok() {
                            add_blocker(surface, blocker);
                        }
                    }
                }
            }
        });
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend_data.early_import(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.0.on_commit();

                if &root == surface {
                    let buffer_offset = with_states(surface, |states| {
                        states
                            .cached_state
                            .get::<SurfaceAttributes>()
                            .current()
                            .buffer_delta
                            .take()
                    });

                    if let Some(buffer_offset) = buffer_offset {
                        let current_loc = self.space.element_location(&window).unwrap();
                        self.space.map_element(window, current_loc + buffer_offset, false);
                    }
                }
            }
        }
        self.popups.commit(surface);

        if matches!(&self.cursor_status, CursorImageStatus::Surface(cursor_surface) if cursor_surface == surface)
        {
            with_states(surface, |states| {
                let cursor_image_attributes = states.data_map.get::<CursorImageSurfaceData>();

                if let Some(mut cursor_image_attributes) =
                    cursor_image_attributes.map(|attrs| attrs.lock().unwrap())
                {
                    let buffer_delta = states
                        .cached_state
                        .get::<SurfaceAttributes>()
                        .current()
                        .buffer_delta
                        .take();
                    if let Some(buffer_delta) = buffer_delta {
                        tracing::trace!(hotspot = ?cursor_image_attributes.hotspot, ?buffer_delta, "decrementing cursor hotspot");
                        cursor_image_attributes.hotspot -= buffer_delta;
                    }
                }
            });
        }

        if matches!(&self.dnd_icon, Some(icon) if &icon.surface == surface) {
            let dnd_icon = self.dnd_icon.as_mut().unwrap();
            with_states(&dnd_icon.surface, |states| {
                let buffer_delta = states
                    .cached_state
                    .get::<SurfaceAttributes>()
                    .current()
                    .buffer_delta
                    .take()
                    .unwrap_or_default();
                tracing::trace!(offset = ?dnd_icon.offset, ?buffer_delta, "moving dnd offset");
                dnd_icon.offset += buffer_delta;
            });
        }

        ensure_initial_configure(surface, &self.space, &mut self.popups)
    }
}

impl<BackendData: Backend> WlrLayerShellHandler for AnvilState<BackendData> {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    fn new_layer_surface(
        &mut self,
        surface: WlrLayerSurface,
        wl_output: Option<wl_output::WlOutput>,
        _layer: Layer,
        namespace: String,
    ) {
        let output = wl_output
            .as_ref()
            .and_then(Output::from_resource)
            .unwrap_or_else(|| self.space.outputs().next().unwrap().clone());
        let mut map = layer_map_for_output(&output);
        map.map_layer(&LayerSurface::new(surface, namespace)).unwrap();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        if let Some((mut map, layer)) = self.space.outputs().find_map(|o| {
            let map = layer_map_for_output(o);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            layer.map(|layer| (map, layer))
        }) {
            map.unmap_layer(&layer);
        }
    }
}

impl<BackendData: Backend> AnvilState<BackendData> {
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<WindowElement> {
        self.space
            .elements()
            .find(|window| window.wl_surface().map(|s| &*s == surface).unwrap_or(false))
            .cloned()
    }
}

#[derive(Default)]
pub struct SurfaceData {
    pub geometry: Option<Rectangle<i32, Logical>>,
    pub resize_state: ResizeState,
}

fn ensure_initial_configure(surface: &WlSurface, space: &Space<WindowElement>, popups: &mut PopupManager) {
    with_surface_tree_upward(
        surface,
        (),
        |_, _, _| TraversalAction::DoChildren(()),
        |_, states, _| {
            states
                .data_map
                .insert_if_missing(|| RefCell::new(SurfaceData::default()));
        },
        |_, _, _| true,
    );

    if let Some(window) = space
        .elements()
        .find(|window| window.wl_surface().map(|s| &*s == surface).unwrap_or(false))
        .cloned()
    {
        // send the initial configure if relevant
        #[cfg_attr(not(feature = "xwayland"), allow(irrefutable_let_patterns))]
        if let Some(toplevel) = window.0.toplevel() {
            let initial_configure_sent = with_states(surface, |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .initial_configure_sent
            });
            if !initial_configure_sent {
                game_mode_initial_state(space, &window, toplevel);
                toplevel.send_configure();
            }
        }

        with_states(surface, |states| {
            let mut data = states
                .data_map
                .get::<RefCell<SurfaceData>>()
                .unwrap()
                .borrow_mut();

            // Finish resizing.
            if let ResizeState::WaitingForCommit(_) = data.resize_state {
                data.resize_state = ResizeState::NotResizing;
            }
        });

        return;
    }

    if let Some(popup) = popups.find_popup(surface) {
        let popup = match popup {
            PopupKind::Xdg(ref popup) => popup,
            // Doesn't require configure
            PopupKind::InputMethod(ref _input_popup) => {
                return;
            }
        };

        if !popup.is_initial_configure_sent() {
            // NOTE: This should never fail as the initial configure is always
            // allowed.
            popup.send_configure().expect("initial configure failed");
        }

        return;
    };

    if let Some(output) = space.outputs().find(|o| {
        let map = layer_map_for_output(o);
        map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
            .is_some()
    }) {
        let initial_configure_sent = with_states(surface, |states| {
            states
                .data_map
                .get::<LayerSurfaceData>()
                .unwrap()
                .lock()
                .unwrap()
                .initial_configure_sent
        });

        let mut map = layer_map_for_output(output);

        // arrange the layers before sending the initial configure
        // to respect any size the client may have sent
        map.arrange();
        // send the initial configure if relevant
        if !initial_configure_sent {
            let layer = map
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .unwrap();

            layer.layer_surface().send_configure();
        }
    };
}

/// Marker: centre this window on its parent/output as soon as its size is known.
#[derive(Default)]
pub struct CenterOnFirstCommit(pub Cell<bool>);

/// The usable area (output minus exclusive layer-shell zones) of the output
/// under the pointer, or of the first output.
pub fn pointer_output_area(space: &Space<WindowElement>, pointer_location: Point<f64, Logical>) -> Rectangle<i32, Logical> {
    let output = space
        .output_under(pointer_location)
        .next()
        .or_else(|| space.outputs().next())
        .cloned();
    output
        .and_then(|o| usable_area(space, &o))
        .unwrap_or_else(|| Rectangle::from_size((1280, 720).into()))
}

pub fn usable_area(space: &Space<WindowElement>, output: &Output) -> Option<Rectangle<i32, Logical>> {
    let geo = space.output_geometry(output)?;
    let map = layer_map_for_output(output);
    let zone = map.non_exclusive_zone();
    Some(Rectangle::new(geo.loc + zone.loc, zone.size))
}

/// Usable area of the output a window lives on.
pub fn window_output_area(space: &Space<WindowElement>, window: &WindowElement) -> Option<Rectangle<i32, Logical>> {
    let output = space
        .outputs_for_element(window)
        .into_iter()
        .next()
        .or_else(|| space.outputs().next().cloned())?;
    usable_area(space, &output)
}

pub fn centered(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    (
        area.loc.x + ((area.size.w - size.w) / 2).max(0),
        area.loc.y + ((area.size.h - size.h) / 2).max(0),
    )
        .into()
}

/// MindOS game mode: a new toplevel fills the output it opens on; dialogs
/// (toplevels with a parent) keep their own size and get centred.
fn game_mode_initial_state(space: &Space<WindowElement>, window: &WindowElement, toplevel: &ToplevelSurface) {
    let Some(area) = window_output_area(space, window) else {
        return;
    };
    let wants_fullscreen = toplevel.with_pending_state(|state| state.states.contains(xdg_toplevel::State::Fullscreen));
    if wants_fullscreen {
        return;
    }
    if toplevel.parent().is_some() {
        window
            .user_data()
            .insert_if_missing(|| CenterOnFirstCommit(Cell::new(true)));
        return;
    }
    toplevel.with_pending_state(|state| {
        state.states.set(xdg_toplevel::State::Maximized);
        state.size = Some(area.size);
    });
}

fn place_new_window(
    space: &mut Space<WindowElement>,
    pointer_location: Point<f64, Logical>,
    window: &WindowElement,
    activate: bool,
) {
    let area = pointer_output_area(space, pointer_location);

    // set the initial toplevel bounds
    #[allow(irrefutable_let_patterns)]
    if let Some(toplevel) = window.0.toplevel() {
        toplevel.with_pending_state(|state| {
            state.bounds = Some(area.size);
        });
    }

    // Game mode: windows start at the origin of the usable area; the initial
    // configure (see `game_mode_initial_state`) asks them to fill it.
    space.map_element(window.clone(), area.loc, activate);
}

pub fn fixup_positions(space: &mut Space<WindowElement>, pointer_location: Point<f64, Logical>) {
    // fixup outputs
    let mut offset = Point::<i32, Logical>::from((0, 0));
    for output in space.outputs().cloned().collect::<Vec<_>>().into_iter() {
        let size = space
            .output_geometry(&output)
            .map(|geo| geo.size)
            .unwrap_or_else(|| Size::from((0, 0)));
        space.map_output(&output, offset);
        layer_map_for_output(&output).arrange();
        offset.x += size.w;
    }

    // fixup windows
    let mut orphaned_windows = Vec::new();
    let outputs = space
        .outputs()
        .flat_map(|o| {
            let geo = space.output_geometry(o)?;
            let map = layer_map_for_output(o);
            let zone = map.non_exclusive_zone();
            Some(Rectangle::new(geo.loc + zone.loc, zone.size))
        })
        .collect::<Vec<_>>();
    for window in space.elements() {
        let window_location = match space.element_location(window) {
            Some(loc) => loc,
            None => continue,
        };
        let geo_loc = window.bbox().loc + window_location;

        if !outputs.iter().any(|o_geo| o_geo.contains(geo_loc)) {
            orphaned_windows.push(window.clone());
        }
    }
    for window in orphaned_windows.into_iter() {
        place_new_window(space, pointer_location, &window, false);
    }
}

impl<BackendData: Backend> AnvilState<BackendData> {
    /// Super+F: toggle fullscreen on the focused window.
    pub fn toggle_fullscreen_focused(&mut self) {
        use smithay::wayland::shell::xdg::XdgShellHandler;
        let Some(window) = self.focused_window() else {
            return;
        };
        if let Some(toplevel) = window.0.toplevel().cloned() {
            if toplevel
                .current_state()
                .states
                .contains(xdg_toplevel::State::Fullscreen)
            {
                XdgShellHandler::unfullscreen_request(self, toplevel);
            } else {
                XdgShellHandler::fullscreen_request(self, toplevel, None);
            }
            return;
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface().cloned() {
            use smithay::xwayland::XwmHandler;
            let Some(id) = self.xwm.as_ref().map(|wm| wm.id()) else {
                return;
            };
            if surface.is_fullscreen() {
                XwmHandler::unfullscreen_request(self, id, surface);
            } else {
                XwmHandler::fullscreen_request(self, id, surface);
            }
        }
    }

    /// Super+M: toggle maximize on the focused window.
    pub fn toggle_maximize_focused(&mut self) {
        use smithay::wayland::shell::xdg::XdgShellHandler;
        let Some(window) = self.focused_window() else {
            return;
        };
        if let Some(toplevel) = window.0.toplevel().cloned() {
            if toplevel
                .current_state()
                .states
                .contains(xdg_toplevel::State::Maximized)
            {
                XdgShellHandler::unmaximize_request(self, toplevel);
            } else {
                XdgShellHandler::maximize_request(self, toplevel);
            }
            return;
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface().cloned() {
            use smithay::xwayland::XwmHandler;
            let Some(id) = self.xwm.as_ref().map(|wm| wm.id()) else {
                return;
            };
            if surface.is_maximized() {
                XwmHandler::unmaximize_request(self, id, surface);
            } else {
                XwmHandler::maximize_request(self, id, surface);
            }
        }
    }
}
