use crate::recover::LockAnyway;
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

#[cfg(feature = "xwayland")]
use smithay::xwayland::XWaylandClientData;

#[cfg(feature = "udev")]
use smithay::wayland::drm_syncobj::DrmSyncobjCachedState;

use smithay::{
    backend::renderer::utils::{on_commit_buffer_handler, with_renderer_surface_state},
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
            Client, DisplayHandle, Resource,
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
                KeyboardInteractivity, Layer, LayerSurface as WlrLayerSurface, LayerSurfaceCachedState,
                LayerSurfaceData, WlrLayerShellHandler, WlrLayerShellState,
            },
            xdg::{PopupSurface as XdgPopupSurface, ToplevelSurface, XdgToplevelSurfaceData},
        },
    },
};

use crate::{
    focus::KeyboardFocusTarget,
    state::{AnvilState, Backend},
    ClientState,
};

mod element;
pub mod frame;
mod grabs;
pub mod ssd;
#[cfg(feature = "xwayland")]
mod x11;
mod xdg;

pub use self::element::*;
pub use self::grabs::*;
pub use crate::layout::client_rect;

fn fullscreen_output(
    wl_surface: &WlSurface,
    wl_output: Option<&wl_output::WlOutput>,
    space: &Space<WindowElement>,
) -> Option<Output> {
    wl_output
        .and_then(Output::from_resource)
        .filter(|output| space.output_geometry(output).is_some())
        .or_else(|| {
            let window = space.elements().find(|w| w.wl_surface().as_deref() == Some(wl_surface))?;
            space.outputs_for_element(window).first().cloned().or_else(|| {
                // A startup fullscreen request can precede the first buffer,
                // so Space has not associated this window with an output yet.
                let location = space.element_location(window)?;
                space.outputs().find(|o| space.output_geometry(o).is_some_and(|r| r.contains(location))).cloned()
            })
        })
        .or_else(|| space.outputs().next().cloned())
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
                        // The client is gone: nothing is waiting on this
                        // fence and nobody is left to tell about it.
                        let Some(client) = surface.client() else {
                            return;
                        };
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

    fn destroyed(&mut self, _surface: &WlSurface) {
        self.request_repaint();
    }

    fn commit(&mut self, surface: &WlSurface) {
        on_commit_buffer_handler::<Self>(surface);
        self.backend_data.early_import(surface);
        // Only the displays this surface is on. Every client's every frame
        // comes through here; see `request_repaint_for`.
        self.request_repaint_for(surface);

        if !is_sync_subsurface(surface) {
            let mut root = surface.clone();
            while let Some(parent) = get_parent(&root) {
                root = parent;
            }
            if let Some(window) = self.window_for_surface(&root) {
                window.0.on_commit();
                // A game's frame goes to the screen as soon as it arrives
                // rather than at the repaint point: the flip catches the next
                // vblank, and with VRR the display follows the game's pace.
                if let Some(output) = self.fullscreen_output_of(&window) {
                    BackendData::repaint_now(self, &output);
                }

                if &root == surface {
                    let is_toplevel = window.0.toplevel().is_some();
                    let buffer_offset = with_states(surface, |states| {
                        states
                            .cached_state
                            .get::<SurfaceAttributes>()
                            .current()
                            .buffer_delta
                            .take()
                    });

                    if let Some(buffer_offset) = buffer_offset {
                        if let Some(current_loc) = self.space.element_location(&window) {
                            self.space.map_element(window, current_loc + buffer_offset, false);
                        }
                    }
                    if is_toplevel {
                        // Post-commit hooks run before on_commit_buffer_handler
                        // and Window::on_commit. Place using the new geometry
                        // here so a dialog never flashes at the output origin.
                        xdg::handle_toplevel_commit(&mut self.space, surface);
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
                    cursor_image_attributes.map(|attrs| attrs.lock_anyway())
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

        let open_maximized = self.layout.open_maximized();
        if let Some(output) = ensure_initial_configure(surface, &self.space, &mut self.popups, open_maximized) {
            // A panel changed its exclusive zone: maximised windows follow the usable area.
            self.relayout_output(&output);
        }
        self.focus_new_layer(surface);
    }
}

/// Set once an on-demand layer surface has been handed the keyboard while it
/// is up; cleared when it drops back to a lower layer.
struct LayerFocusGiven(Cell<bool>);

impl<BackendData: Backend> AnvilState<BackendData> {
    /// A panel or popup that asked for on-demand keyboard interactivity gets
    /// the keyboard when it appears (sway does the same): a context menu or
    /// the layout picker can then be dismissed with Escape without a click
    /// into it first. Exclusive layers are handled per key press in the input
    /// handler; `none` layers never get the keyboard.
    fn focus_new_layer(&mut self, surface: &WlSurface) {
        let layer = self.space.outputs().find_map(|o| {
            layer_map_for_output(o)
                .layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)
                .cloned()
        });
        let Some(layer) = layer else { return };
        let (interactivity, wlr_layer) = with_states(surface, |states| {
            let current = *states.cached_state.get::<LayerSurfaceCachedState>().current();
            (current.keyboard_interactivity, current.layer)
        });
        if interactivity != KeyboardInteractivity::OnDemand || !matches!(wlr_layer, Layer::Top | Layer::Overlay) {
            // Down on a lower layer again: the next rise counts as a new
            // appearance. The desktop comes forward and sinks back many times
            // in a session, and it needs the keyboard every time it is up.
            with_states(surface, |states| {
                if let Some(given) = states.data_map.get::<LayerFocusGiven>() {
                    given.0.set(false);
                }
            });
            return;
        }
        let mapped = with_renderer_surface_state(surface, |state| state.buffer().is_some()).unwrap_or(false);
        if !mapped {
            return;
        }
        let first = with_states(surface, |states| {
            states.data_map.insert_if_missing(|| LayerFocusGiven(Cell::new(false)));
            !states.data_map.get::<LayerFocusGiven>().unwrap().0.replace(true)
        });
        if !first {
            return;
        }
        let Some(keyboard) = self.seat.get_keyboard() else { return };
        if keyboard.is_grabbed() {
            return;
        }
        if self.idle.locked && layer.namespace() != crate::idle::LOCK_NAMESPACE {
            // Locked: a panel or popup mapping behind the lock screen gets nothing.
            return;
        }
        let serial = crate::input_handler::next_serial();
        keyboard.set_focus(self, Some(KeyboardFocusTarget::LayerSurface(layer)), serial);
    }
}

impl<BackendData: Backend> WlrLayerShellHandler for AnvilState<BackendData> {
    fn shell_state(&mut self) -> &mut WlrLayerShellState {
        &mut self.layer_shell_state
    }

    /// A popup on a panel: a tooltip, a menu. The xdg popup is created before
    /// the layer surface adopts it, so `XdgShellHandler::new_popup` runs while
    /// it is still an orphan and cannot tell what screen it belongs to. This
    /// is where the parent is known, so this is where it gets placed.
    fn new_popup(&mut self, _parent: WlrLayerSurface, popup: XdgPopupSurface) {
        self.unconstrain_popup(&popup);
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
            .or_else(|| self.space.outputs().next().cloned());
        // A panel or a wallpaper that names no output goes on the first one.
        // With every display unplugged there is no first one, and the shell
        // does exactly this while a monitor is being switched: the surface is
        // simply not mapped, and the shell maps it again when a display is
        // back.
        let Some(output) = output else {
            tracing::warn!("a layer surface arrived while no display is connected");
            return;
        };
        let mut map = layer_map_for_output(&output);
        if let Err(err) = map.map_layer(&LayerSurface::new(surface, namespace)) {
            tracing::warn!("cannot place a layer surface on {}: {err:?}", output.name());
        }
        drop(map);
        self.request_repaint();
    }

    fn layer_destroyed(&mut self, surface: WlrLayerSurface) {
        self.request_repaint();
        let mut changed = None;
        for output in self.space.outputs() {
            let mut map = layer_map_for_output(output);
            let layer = map
                .layers()
                .find(|&layer| layer.layer_surface() == &surface)
                .cloned();
            if let Some(layer) = layer {
                let before = map.non_exclusive_zone();
                map.unmap_layer(&layer);
                map.arrange();
                if map.non_exclusive_zone() != before {
                    changed = Some(output.clone());
                }
                break;
            }
        }
        if let Some(output) = changed {
            self.relayout_output(&output);
        }
        // A popup that held the keyboard went away: type into the top window again.
        self.refresh_focus();
    }
}

impl<BackendData: Backend> AnvilState<BackendData> {
    pub fn window_for_surface(&self, surface: &WlSurface) -> Option<WindowElement> {
        self.space
            .elements()
            .find(|window| window.wl_surface().map(|s| &*s == surface).unwrap_or(false))
            .cloned()
    }

    /// The area a maximised window fills. Normally the usable area; a game
    /// gets the whole screen, because a borderless game is only maximised and
    /// is meant to cover everything. The panels on that screen go away for as
    /// long as it does (`game_screen`), so there is nothing left to leave room
    /// for -- and leaving room would show a strip of desktop under the game.
    pub fn maximized_area(&self, window: &WindowElement) -> Option<Rectangle<i32, Logical>> {
        let output = window_output(&self.space, window)?;
        if self.is_game(window) {
            self.space.output_geometry(&output)
        } else {
            usable_area(&self.space, &output)
        }
    }

    /// Is this window a game? Steam says so in the app id it gives a game it
    /// started, and everything else it knows is that the program runs under
    /// Wine or Proton.
    fn is_game(&self, window: &WindowElement) -> bool {
        if window.app_id().to_ascii_lowercase().starts_with("steam_app_") {
            return true;
        }
        crate::procinfo::is_wine(window_pid(window, &self.display_handle))
    }

    /// How much of an output's top edge the shell's own bar covers. A size
    /// over zero is for one screen: every other one is cleared, since there
    /// is only ever the one bar.
    pub fn set_desktop_bar(&mut self, output: Option<&str>, size: i32) {
        let size = size.max(0);
        let mut changed = false;
        for o in self.space.outputs() {
            let named = output.is_none_or(|name| name == o.name());
            let wanted = match (named, size > 0) {
                (true, _) => size,
                (false, true) => 0,
                (false, false) => continue,
            };
            o.user_data().insert_if_missing(DesktopBar::default);
            let bar = o.user_data().get::<DesktopBar>().unwrap();
            if bar.0.replace(wanted) != wanted {
                changed = true;
            }
        }
        if changed {
            self.layout.dirty = true;
            self.request_repaint();
        }
    }

    /// Work out, for every screen, whether a game is covering the whole of it
    /// (see [`game_screen`]). Run once per turn of the event loop, on what the
    /// last turn's commits left behind.
    pub fn refresh_game_screens(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        let mut changed = false;
        for output in outputs {
            let covered = self.game_covers_output(&output);
            output.user_data().insert_if_missing(GameScreen::default);
            let game = output.user_data().get::<GameScreen>().unwrap();
            if game.0.replace(covered) != covered {
                changed = true;
            }
        }
        // The panels appear and disappear with this, so the screen needs
        // redrawing even if nothing else on it moved.
        if changed {
            self.request_repaint();
        }
    }

    /// A game filling `output`: fullscreen on it, or a window whose own
    /// geometry covers it edge to edge. A borderless game is only maximised,
    /// and some ask for neither and simply size themselves to the screen, so
    /// what the window covers is the question rather than what it asked for.
    fn game_covers_output(&self, output: &Output) -> bool {
        let Some(screen) = self.space.output_geometry(output) else { return false };
        // The home screen summoned over the top of the game (Super+D) is what
        // the user asked to look at; the panels belong with it.
        if desktop_presenting(output) {
            return false;
        }
        self.space.elements().any(|window| {
            if !self.is_game(window) {
                return false;
            }
            let fullscreen = self.fullscreen_output_of(window).as_ref() == Some(output);
            let geo = self
                .space
                .outputs_for_element(window)
                .contains(output)
                .then(|| self.space.element_geometry(window))
                .flatten();
            covers_screen(screen, fullscreen, geo)
        })
    }

    /// The output `window` is drawn fullscreen on, if any.
    pub fn fullscreen_output_of(&self, window: &WindowElement) -> Option<Output> {
        self.space
            .outputs()
            .find(|output| {
                output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|fullscreen| fullscreen.get())
                    .as_ref()
                    == Some(window)
            })
            .cloned()
    }
}

#[derive(Default)]
pub struct SurfaceData {
    pub geometry: Option<Rectangle<i32, Logical>>,
    pub resize_state: ResizeState,
}

/// Returns the output whose usable area changed because a layer surface
/// (re)arranged itself on this commit.
fn ensure_initial_configure(
    surface: &WlSurface,
    space: &Space<WindowElement>,
    popups: &mut PopupManager,
    open_maximized: bool,
) -> Option<Output> {
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
                    .lock_anyway()
                    .initial_configure_sent
            });
            if !initial_configure_sent {
                initial_state(space, &window, toplevel, open_maximized);
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

        return None;
    }

    if let Some(popup) = popups.find_popup(surface) {
        let popup = match popup {
            PopupKind::Xdg(ref popup) => popup,
            // Doesn't require configure
            PopupKind::InputMethod(ref _input_popup) => {
                return None;
            }
        };

        if !popup.is_initial_configure_sent() {
            // This is meant to be always allowed, and the one way it is not
            // is a client that has gone away in between. A menu that never
            // opens is a menu.
            if let Err(err) = popup.send_configure() {
                tracing::trace!("a popup could not be configured: {err:?}");
            }
        }

        return None;
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
                .lock_anyway()
                .initial_configure_sent
        });

        let mut map = layer_map_for_output(output);

        // arrange the layers before sending the initial configure
        // to respect any size the client may have sent
        let zone_before = map.non_exclusive_zone();
        map.arrange();
        let zone_changed = map.non_exclusive_zone() != zone_before;
        // send the initial configure if relevant
        if !initial_configure_sent {
            if let Some(layer) = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL) {
                layer.layer_surface().send_configure();
            }
        }
        if zone_changed {
            return Some(output.clone());
        }
    };
    None
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

/// The output minus the exclusive zones of layer-shell panels, with the
/// shell's own bar left in: what a game fills when it goes borderless (only
/// maximised, never fullscreen) and should still cover the screen.
pub fn panel_area(space: &Space<WindowElement>, output: &Output) -> Option<Rectangle<i32, Logical>> {
    let geo = space.output_geometry(output)?;
    let map = layer_map_for_output(output);
    let zone = map.non_exclusive_zone();
    Some(Rectangle::new(geo.loc + zone.loc, zone.size))
}

/// Where a window is allowed to be: the output minus the exclusive zones of
/// the user's layer-shell panels, minus the strip the shell's own bar covers.
/// The home screen below that bar comes and goes with the windows, but the
/// bar itself is always there, so the room it takes is always reserved.
pub fn usable_area(space: &Space<WindowElement>, output: &Output) -> Option<Rectangle<i32, Logical>> {
    Some(below_desktop_bar(panel_area(space, output)?, desktop_bar(output)))
}

/// `screen` with the desktop bar's strip taken off the top. A bar that is
/// missing, nonsense, or so tall it would leave a window nowhere to be gives
/// the screen back whole: a wrong measurement must not cost the user the
/// screen.
fn below_desktop_bar(screen: Rectangle<i32, Logical>, bar: i32) -> Rectangle<i32, Logical> {
    if bar <= 0 || screen.size.h - bar < MIN_ROOM {
        return screen;
    }
    Rectangle::new(screen.loc + Point::from((0, bar)), Size::from((screen.size.w, screen.size.h - bar)))
}

/// The least room the desktop bar may leave a window.
const MIN_ROOM: i32 = 200;

/// How much of an output's top edge the shell's desktop bar covers, kept in
/// the output's user data (see `desktop_bar`).
#[derive(Default)]
pub struct DesktopBar(pub Cell<i32>);

/// The bar height the windows on an output were last arranged around (see
/// `layout_refresh`).
#[derive(Default)]
pub struct DesktopBarApplied(pub Cell<i32>);

/// How much of the top of `output` the shell's bar covers (logical px), 0 for
/// a screen without one. The shell draws that bar inside its desktop window,
/// which is anchored to every edge and so reserves nothing through
/// layer-shell; it reports the height instead (`desktop_bar` IPC). The strip
/// only counts while that desktop is up, so a shell that goes away hands the
/// screen back.
pub fn desktop_bar(output: &Output) -> i32 {
    let bar = output.user_data().get::<DesktopBar>().map_or(0, |bar| bar.0.get());
    if bar <= 0 || !desktop_up(output) {
        return 0;
    }
    bar
}

/// The namespace the shell gives its desktop window (`mindshell/src/windows.rs`).
pub const DESKTOP_NAMESPACE: &str = "mindshell-desktop";

/// Whether the shell's home screen is up on `output`, whichever layer it is
/// sitting on. The startup screen is drawn only until this is true, and the
/// shell moves that one surface between the background and the top every
/// time the user asks for the home screen back: matching the namespace
/// rather than the background layer is what keeps the startup screen from
/// blinking through those swaps.
///
/// A surface on the background layer that is not the shell's still counts --
/// someone else's wallpaper is a desktop too.
pub fn desktop_up(output: &Output) -> bool {
    let mapped = {
        let map = layer_map_for_output(output);
        map.layers().any(|layer| layer.namespace() == DESKTOP_NAMESPACE)
            || map.layers_on(Layer::Background).next().is_some()
    };
    output.user_data().insert_if_missing(DesktopSeen::default);
    let seen = output.user_data().get::<DesktopSeen>().unwrap();
    if mapped {
        seen.0.set(Some(Instant::now()));
        return true;
    }
    // A client that remaps its surface to change layer leaves a frame or two
    // with nothing there. Hold the answer over that gap, but not over a shell
    // that has died: then the startup screen is the right thing to show.
    seen.0.get().is_some_and(|at| at.elapsed() < DESKTOP_GONE)
}

/// How long the home screen counts as still up after its surface goes.
const DESKTOP_GONE: Duration = Duration::from_millis(1500);

/// When the shell's home screen was last up on an output (see `desktop_up`).
#[derive(Default)]
pub struct DesktopSeen(pub Cell<Option<Instant>>);

/// Whether the shell's home screen has been raised over the windows on
/// `output` (Super+D): the desktop window swaps to the top layer for as long
/// as it is summoned. A game covering the screen does not take the panels
/// away while the user is looking at the desktop over the top of it.
pub fn desktop_presenting(output: &Output) -> bool {
    layer_map_for_output(output)
        .layers_on(Layer::Top)
        .any(|layer| layer.namespace() == DESKTOP_NAMESPACE)
}

/// Does a window leave nothing of `screen` showing? A fullscreen window does
/// by definition -- it is drawn over the whole output whatever the space says
/// its geometry is. Anything else has to reach all four edges itself, which
/// is what a borderless game does and what a merely maximised window, kept
/// clear of the panels, does not.
fn covers_screen(
    screen: Rectangle<i32, Logical>,
    fullscreen: bool,
    geo: Option<Rectangle<i32, Logical>>,
) -> bool {
    fullscreen || geo.is_some_and(|geo| geo.contains_rect(screen))
}

/// Whether a game has the whole of this screen, kept in the output's user
/// data (see [`game_screen`]).
#[derive(Default)]
pub struct GameScreen(pub Cell<bool>);

/// Is a game covering the whole of `output`? The panels belong under a game
/// that fills the screen -- the bar is not meant to sit over the bottom of it
/// -- so while this is true the top-layer surfaces on that screen are neither
/// drawn nor clickable, exactly as they are under a fullscreen window.
pub fn game_screen(output: &Output) -> bool {
    output.user_data().get::<GameScreen>().is_some_and(|game| game.0.get())
}

/// The output a window lives on: the first one it overlaps, falling back to
/// the first there is (a window that has not drawn yet overlaps nothing).
pub fn window_output(space: &Space<WindowElement>, window: &WindowElement) -> Option<Output> {
    space
        .outputs_for_element(window)
        .into_iter()
        .next()
        .or_else(|| space.outputs().next().cloned())
}

/// Usable area of the output a window lives on.
pub fn window_output_area(space: &Space<WindowElement>, window: &WindowElement) -> Option<Rectangle<i32, Logical>> {
    usable_area(space, &window_output(space, window)?)
}

/// The pid behind a window, for the little the compositor asks `/proc`
/// (`procinfo`): the client's credentials, or what XWayland was told.
pub fn window_pid(window: &WindowElement, dh: &DisplayHandle) -> Option<u32> {
    if let Some(toplevel) = window.0.toplevel() {
        return toplevel
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(dh).ok())
            .map(|credentials| credentials.pid as u32);
    }
    #[cfg(feature = "xwayland")]
    if let Some(surface) = window.0.x11_surface() {
        return surface.pid();
    }
    None
}

pub fn centered(area: Rectangle<i32, Logical>, size: Size<i32, Logical>) -> Point<i32, Logical> {
    (
        area.loc.x + ((area.size.w - size.w) / 2).max(0),
        area.loc.y + ((area.size.h - size.h) / 2).max(0),
    )
        .into()
}

/// KDE-style cascade for a new floating window: when another window already
/// sits where this one would go, step it down and right so both stay
/// visible, as long as it still fits inside `area`.
pub fn cascade(
    space: &Space<WindowElement>,
    area: Rectangle<i32, Logical>,
    mut loc: Point<i32, Logical>,
    size: Size<i32, Logical>,
    except: &WindowElement,
) -> Point<i32, Logical> {
    const STEP: i32 = 40;
    let taken: Vec<Point<i32, Logical>> = space
        .elements()
        .filter(|w| *w != except)
        .filter_map(|w| space.element_location(w))
        .collect();
    for _ in 0..16 {
        let clash = taken
            .iter()
            .any(|p| (p.x - loc.x).abs() < STEP && (p.y - loc.y).abs() < STEP);
        if !clash {
            break;
        }
        let next = loc + Point::from((STEP, STEP));
        if next.x + size.w > area.loc.x + area.size.w || next.y + size.h > area.loc.y + area.size.h {
            break;
        }
        loc = next;
    }
    loc
}

/// The first configure of a toplevel. A tile already has its size from the
/// layout; a dialog, or any window in floating mode, keeps its own size and
/// is centred on its first commit; with `open_maximized` (the old "game
/// mode") a new window fills the usable area of its output.
fn initial_state(space: &Space<WindowElement>, window: &WindowElement, toplevel: &ToplevelSurface, open_maximized: bool) {
    let Some(area) = window_output_area(space, window) else {
        return;
    };
    let (wants_fullscreen, sized) = toplevel.with_pending_state(|state| {
        (
            state.states.contains(xdg_toplevel::State::Fullscreen),
            state.size.is_some(),
        )
    });
    if wants_fullscreen || sized {
        return;
    }
    if toplevel.parent().is_some() || !open_maximized {
        window
            .user_data()
            .insert_if_missing(|| CenterOnFirstCommit(Cell::new(true)));
        return;
    }
    let header = window.pending_header_height();
    toplevel.with_pending_state(|state| {
        state.states.set(xdg_toplevel::State::Maximized);
        state.size = Some(client_rect(area, header).size);
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

    // Windows start at the origin of the usable area; the initial configure
    // (see `initial_state`) and the layout decide where they end up.
    space.map_element(window.clone(), area.loc, activate);
}

/// Place the outputs (left to right, unless the user pinned a position in the
/// Displays settings) and bring back windows that ended up off-screen.
pub fn fixup_positions(
    space: &mut Space<WindowElement>,
    pointer_location: Point<f64, Logical>,
    pinned: &BTreeMap<String, [i32; 2]>,
) {
    // fixup outputs
    let mut offset = 0;
    for output in space.outputs().cloned().collect::<Vec<_>>().into_iter() {
        let size = space
            .output_geometry(&output)
            .map(|geo| geo.size)
            .unwrap_or_else(|| Size::from((0, 0)));
        let location: Point<i32, Logical> = match pinned.get(&output.name()) {
            Some([x, y]) => (*x, *y).into(),
            None => (offset, 0).into(),
        };
        if output.current_location() != location {
            output.change_current_state(None, None, None, Some(location));
        }
        space.map_output(&output, location);
        layer_map_for_output(&output).arrange();
        offset = offset.max(location.x + size.w);
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
    /// The usable area of `output` changed (a shell panel appeared or went
    /// away): every window on it is placed again on the next turn.
    pub fn relayout_output(&mut self, _output: &Output) {
        self.layout.dirty = true;
    }

    pub fn relayout_all_outputs(&mut self) {
        self.layout.dirty = true;
    }

    /// Super+F: toggle fullscreen on the focused window.
    pub fn toggle_fullscreen_focused(&mut self) {
        if let Some(window) = self.focused_window() {
            self.toggle_fullscreen_window(&window);
        }
    }

    /// Super+M: toggle maximize on the focused window.
    pub fn toggle_maximize_focused(&mut self) {
        if let Some(window) = self.focused_window() {
            self.toggle_maximize_window(&window);
        }
    }

    pub fn toggle_fullscreen_window(&mut self, window: &WindowElement) {
        use smithay::wayland::shell::xdg::XdgShellHandler;
        if !self.space.elements().any(|w| w == window) {
            return;
        }
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

    pub fn toggle_maximize_window(&mut self, window: &WindowElement) {
        use smithay::wayland::shell::xdg::XdgShellHandler;
        if !self.space.elements().any(|w| w == window) {
            return;
        }
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

#[cfg(test)]
mod placement_tests {
    use super::*;
    use smithay::output::{Mode, PhysicalProperties, Subpixel};
    use smithay::utils::Transform;

    fn display(name: &str, at: (i32, i32), size: (i32, i32)) -> Output {
        let output = Output::new(
            name.to_string(),
            PhysicalProperties { size: (600, 340).into(), subpixel: Subpixel::Unknown, make: "MindOS".into(), model: "Test".into() },
        );
        let mode = Mode { size: (size.0, size.1).into(), refresh: 60_000 };
        output.change_current_state(Some(mode), Some(Transform::Normal), None, Some(at.into()));
        output.set_preferred(mode);
        output
    }

    /// A window opens where the pointer is: a shortcut fired with the mouse on
    /// the second display must not put its window on the first one.
    #[test]
    fn a_new_window_lands_on_the_display_the_pointer_is_on() {
        let mut space: Space<WindowElement> = Space::default();
        let left = display("left", (0, 0), (1920, 1080));
        let right = display("right", (1920, 0), (2560, 1440));
        space.map_output(&left, (0, 0));
        space.map_output(&right, (1920, 0));

        let on_left = pointer_output_area(&space, (400.0, 500.0).into());
        assert_eq!(on_left.loc, (0, 0).into());
        assert_eq!(on_left.size, (1920, 1080).into());

        let on_right = pointer_output_area(&space, (2400.0, 900.0).into());
        assert_eq!(on_right.loc, (1920, 0).into());
        assert_eq!(on_right.size, (2560, 1440).into());

        // Just over the seam is still the right-hand display.
        assert_eq!(pointer_output_area(&space, (1921.0, 10.0).into()).loc, (1920, 0).into());
        // A pointer nowhere at all falls back to a real display, never to nothing.
        assert!(space.outputs().any(|o| space.output_geometry(o).map(|g| g.loc) == Some(pointer_output_area(&space, (-500.0, -500.0).into()).loc)));
    }

    /// The user's panels are the only thing that takes room off a screen: the
    /// shell's home screen reserves nothing, because it is never on screen at
    /// the same time as the windows.
    #[test]
    fn only_panels_take_room_off_a_screen() {
        let mut space: Space<WindowElement> = Space::default();
        let screen = display("only", (1920, 0), (2560, 1440));
        space.map_output(&screen, (1920, 0));
        let area = usable_area(&space, &screen).unwrap();
        assert_eq!(area.loc, (1920, 0).into());
        assert_eq!(area.size, (2560, 1440).into());
    }

    /// The desktop bar takes a strip off the top of the screen, and gives it
    /// back rather than leaving a window nowhere to be.
    #[test]
    fn the_desktop_bar_keeps_windows_below_it() {
        let screen = Rectangle::new(Point::from((1920, 0)), Size::from((2560, 1440)));
        let below = below_desktop_bar(screen, 45);
        assert_eq!(below.loc, (1920, 45).into());
        assert_eq!(below.size, (2560, 1395).into());
        // No bar, a nonsense bar, and one that would leave a sliver: the
        // screen is left as it is.
        assert_eq!(below_desktop_bar(screen, 0), screen);
        assert_eq!(below_desktop_bar(screen, -20), screen);
        assert_eq!(below_desktop_bar(screen, 1439), screen);
        assert_eq!(below_desktop_bar(screen, MIN_ROOM), Rectangle::new((1920, MIN_ROOM).into(), (2560, 1440 - MIN_ROOM).into()));
    }

    /// The strip is only reserved while the shell's desktop is up: a screen
    /// with no desktop on it hands it straight back.
    #[test]
    fn a_screen_without_a_desktop_reserves_nothing() {
        let mut space: Space<WindowElement> = Space::default();
        let screen = display("bare", (0, 0), (1920, 1080));
        space.map_output(&screen, (0, 0));
        screen.user_data().insert_if_missing(DesktopBar::default);
        screen.user_data().get::<DesktopBar>().unwrap().0.set(45);
        assert_eq!(desktop_bar(&screen), 0);
        assert_eq!(usable_area(&space, &screen), panel_area(&space, &screen));
    }

    /// The startup screen goes the moment the shell's home screen arrives,
    /// and must not blink back while that one surface changes layer.
    #[test]
    fn the_home_screen_covers_a_layer_swap() {
        let screen = display("only", (0, 0), (1920, 1080));
        // Nothing mapped and nothing ever seen: the startup screen is all
        // there is to show.
        assert!(!desktop_up(&screen));
        let seen = screen.user_data().get::<DesktopSeen>().unwrap();

        // A surface that was there a moment ago: a client remapping itself to
        // change layer, which is what Super+D does.
        seen.0.set(Some(Instant::now()));
        assert!(desktop_up(&screen));

        // A shell that died and stayed dead hands the screen back.
        seen.0.set(Instant::now().checked_sub(DESKTOP_GONE + Duration::from_millis(1)));
        assert!(!desktop_up(&screen));
    }

    /// What counts as a game having the whole screen: fullscreen whatever the
    /// space thinks, and otherwise every edge reached. A window kept inside
    /// the panels does not count, which is the case the bar used to be
    /// visible in.
    #[test]
    fn a_game_has_the_screen_only_when_it_covers_it() {
        let screen = Rectangle::new(Point::from((1920, 0)), Size::from((2560, 1440)));
        assert!(covers_screen(screen, true, None));
        assert!(covers_screen(screen, false, Some(screen)));
        // Borderless, but held above a 72px bar at the bottom.
        let above_bar = Rectangle::new(Point::from((1920, 0)), Size::from((2560, 1368)));
        assert!(!covers_screen(screen, false, Some(above_bar)));
        // Larger than the screen still covers it; a window on another screen
        // has no geometry here at all.
        assert!(covers_screen(screen, false, Some(Rectangle::new(Point::from((1900, -10)), Size::from((2600, 1460))))));
        assert!(!covers_screen(screen, false, None));
    }

    /// The answer is kept per screen, and a screen nobody has asked about is
    /// not a game screen.
    #[test]
    fn the_game_screen_flag_is_per_display() {
        let left = display("left", (0, 0), (1920, 1080));
        let right = display("right", (1920, 0), (2560, 1440));
        assert!(!game_screen(&left) && !game_screen(&right));
        left.user_data().insert_if_missing(GameScreen::default);
        left.user_data().get::<GameScreen>().unwrap().0.set(true);
        assert!(game_screen(&left));
        assert!(!game_screen(&right));
    }
}
