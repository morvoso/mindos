use std::{
    collections::HashMap,
    os::unix::io::OwnedFd,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use tracing::{info, warn};

use smithay::utils::IsAlive;
use smithay::{
    backend::{
        input::TabletToolDescriptor,
        renderer::element::{
            default_primary_scanout_output_compare, utils::select_dmabuf_feedback, RenderElementStates,
        },
    },
    delegate_compositor, delegate_data_control, delegate_data_device, delegate_fractional_scale,
    delegate_input_method_manager, delegate_keyboard_shortcuts_inhibit, delegate_layer_shell,
    delegate_output, delegate_pointer_constraints, delegate_pointer_gestures, delegate_presentation,
    delegate_primary_selection, delegate_relative_pointer, delegate_seat, delegate_security_context,
    delegate_shm, delegate_tablet_manager, delegate_text_input_manager, delegate_viewporter,
    delegate_virtual_keyboard_manager, delegate_xdg_activation, delegate_xdg_decoration, delegate_xdg_shell,
    desktop::{
        space::SpaceElement,
        utils::{
            surface_presentation_feedback_flags_from_states, surface_primary_scanout_output,
            update_surface_primary_scanout_output, with_surfaces_surface_tree, OutputPresentationFeedback,
        },
        PopupKind, PopupManager, Space,
    },
    input::{
        keyboard::{LedState, XkbConfig},
        pointer::{CursorIcon, CursorImageStatus, CursorImageSurfaceData, PointerHandle},
        Seat, SeatHandler, SeatState,
    },
    output::Output,
    reexports::{
        calloop::{generic::Generic, Interest, LoopHandle, Mode, PostAction},
        wayland_protocols::xdg::decoration::{
            self as xdg_decoration, zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode,
        },
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::{wl_data_source::WlDataSource, wl_surface::WlSurface},
            Client, Display, DisplayHandle, Resource,
        },
    },
    utils::{Clock, Logical, Monotonic, Point, Rectangle, Time},
    wayland::{
        commit_timing::{CommitTimerBarrierStateUserData, CommitTimingManagerState},
        compositor::{get_parent, with_states, CompositorClientState, CompositorHandler, CompositorState},
        dmabuf::DmabufFeedback,
        fifo::{FifoBarrierCachedState, FifoManagerState},
        fractional_scale::{with_fractional_scale, FractionalScaleHandler, FractionalScaleManagerState},
        input_method::{InputMethodHandler, InputMethodManagerState, PopupSurface},
        keyboard_shortcuts_inhibit::{
            KeyboardShortcutsInhibitHandler, KeyboardShortcutsInhibitState, KeyboardShortcutsInhibitor,
        },
        output::{OutputHandler, OutputManagerState},
        pointer_constraints::{PointerConstraintsHandler, PointerConstraintsState},
        pointer_gestures::PointerGesturesState,
        presentation::PresentationState,
        relative_pointer::RelativePointerManagerState,
        seat::WaylandFocus,
        security_context::{
            SecurityContext, SecurityContextHandler, SecurityContextListenerSource, SecurityContextState,
        },
        selection::{
            data_device::{
                set_data_device_focus, ClientDndGrabHandler, DataDeviceHandler, DataDeviceState,
                ServerDndGrabHandler,
            },
            primary_selection::{set_primary_focus, PrimarySelectionHandler, PrimarySelectionState},
            wlr_data_control::{DataControlHandler, DataControlState},
            SelectionHandler,
        },
        shell::{
            wlr_layer::WlrLayerShellState,
            xdg::{
                decoration::{XdgDecorationHandler, XdgDecorationState},
                ToplevelSurface, XdgShellState,
            },
        },
        shm::{ShmHandler, ShmState},
        single_pixel_buffer::SinglePixelBufferState,
        socket::ListeningSocketSource,
        tablet_manager::{TabletManagerState, TabletSeatHandler},
        text_input::TextInputManagerState,
        viewporter::ViewporterState,
        virtual_keyboard::VirtualKeyboardManagerState,
        xdg_activation::{
            XdgActivationHandler, XdgActivationState, XdgActivationToken, XdgActivationTokenData,
        },
        xdg_foreign::{XdgForeignHandler, XdgForeignState},
    },
};

#[cfg(feature = "xwayland")]
use crate::cursor::Cursor;
use smithay::wayland::cursor_shape::CursorShapeManagerState;
use smithay::wayland::idle_inhibit::IdleInhibitManagerState;
use smithay::wayland::idle_notify::IdleNotifierState;
use crate::idle::IdleState;
use crate::ipc::{prefs_event, ModeInfo, OutputChange};
use crate::layout::{LayoutMode, LayoutState};
use crate::prefs::{mode_key, Prefs};
use crate::{
    config::Config,
    focus::{KeyboardFocusTarget, PointerFocusTarget},
    ipc::{IpcServer, OutputInfo, WindowInfo, WindowsSnapshot},
    launcher,
    mind::{Event as MindDaemonEvent, MindClient, MindEvent},
    mindbar::{BarAction, MindBar},
    shell::{FullscreenSurface, WindowElement},
    text::TextRenderer,
};
use smithay::{
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    wayland::shell::xdg::XdgToplevelSurfaceData,
};
use serde_json::{json, Value};
use smithay::reexports::calloop::channel;
#[cfg(feature = "xwayland")]
use smithay::{
    delegate_xwayland_keyboard_grab, delegate_xwayland_shell,
    utils::Size,
    wayland::selection::{SelectionSource, SelectionTarget},
    wayland::xwayland_keyboard_grab::{XWaylandKeyboardGrabHandler, XWaylandKeyboardGrabState},
    wayland::xwayland_shell,
    xwayland::{X11Wm, XWayland, XWaylandEvent},
};

#[derive(Debug, Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
    pub security_context: Option<SecurityContext>,
}
impl ClientData for ClientState {
    /// Notification that a client was initialized
    fn initialized(&self, _client_id: ClientId) {}
    /// Notification that a client is disconnected
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}

#[derive(Debug)]
pub struct AnvilState<BackendData: Backend + 'static> {
    pub backend_data: BackendData,
    pub socket_name: Option<String>,
    pub display_handle: DisplayHandle,
    pub running: Arc<AtomicBool>,
    pub handle: LoopHandle<'static, AnvilState<BackendData>>,

    // desktop
    pub space: Space<WindowElement>,
    pub popups: PopupManager,

    // smithay state
    pub compositor_state: CompositorState,
    pub data_device_state: DataDeviceState,
    pub layer_shell_state: WlrLayerShellState,
    pub output_manager_state: OutputManagerState,
    pub primary_selection_state: PrimarySelectionState,
    pub data_control_state: DataControlState,
    pub seat_state: SeatState<AnvilState<BackendData>>,
    pub keyboard_shortcuts_inhibit_state: KeyboardShortcutsInhibitState,
    pub shm_state: ShmState,
    pub capture: crate::capture::CaptureState,
    pub viewporter_state: ViewporterState,
    pub xdg_activation_state: XdgActivationState,
    pub xdg_decoration_state: XdgDecorationState,
    pub xdg_shell_state: XdgShellState,
    pub presentation_state: PresentationState,
    pub fractional_scale_manager_state: FractionalScaleManagerState,
    pub xdg_foreign_state: XdgForeignState,
    #[cfg(feature = "xwayland")]
    pub xwayland_shell_state: xwayland_shell::XWaylandShellState,
    pub single_pixel_buffer_state: SinglePixelBufferState,
    pub fifo_manager_state: FifoManagerState,
    pub commit_timing_manager_state: CommitTimingManagerState,

    pub dnd_icon: Option<DndIcon>,

    // input-related fields
    pub suppressed_keys: Vec<u32>,
    pub window_cycle: crate::window_cycle::WindowCycle,
    pub media_keys: crate::media::MediaKeys,
    pub cursor_status: CursorImageStatus,
    /// The resize cursor the compositor is showing over a window's resize
    /// ring, so it knows the arrow is its own to put back.
    pub resize_cursor: Option<CursorIcon>,
    pub seat_name: String,
    pub seat: Seat<AnvilState<BackendData>>,
    pub clock: Clock<Monotonic>,
    pub pointer: PointerHandle<AnvilState<BackendData>>,
    pub absolute_pointer_positions: HashMap<String, Point<f64, Logical>>,

    #[cfg(feature = "xwayland")]
    pub xwm: Option<X11Wm>,
    #[cfg(feature = "xwayland")]
    pub xdisplay: Option<u32>,
    /// The XEmbed tray host, up once XWayland is.
    #[cfg(feature = "xwayland")]
    pub xtray: Option<crate::xtray::XTray>,

    #[cfg(feature = "debug")]
    pub renderdoc: Option<renderdoc::RenderDoc<renderdoc::V141>>,

    pub show_window_preview: bool,

    // MindOS
    pub config: Config,
    pub mindbar: MindBar,
    pub mind: MindClient,
    pub startup_done: bool,
    pub xwayland_ready: bool,
    /// Shell IPC (docs/SHELL.md): socket, clients, last snapshots sent.
    pub ipc: IpcServer,
    /// Windows hidden by the shell; they leave the space and come back where they were.
    pub minimized: Vec<Minimized>,
    /// Super + mouse wheel: notches counted since the last window step.
    pub super_scroll: f64,
    /// Window layout mode (floating / dwindle / columns) and its tiling state.
    pub layout: LayoutState,
    /// Preferences kept between sessions (layout mode, Mind bar, displays).
    pub prefs: Prefs,
    /// The screensaver, the lock screen and switching the displays off.
    pub idle: IdleState,
    pub idle_notifier: IdleNotifierState<AnvilState<BackendData>>,
    pub idle_inhibit_manager_state: IdleInhibitManagerState,
}

/// A minimised window: unmapped from the space, restored at `location`.
#[derive(Debug, Clone)]
pub struct Minimized {
    pub window: WindowElement,
    pub location: Point<i32, Logical>,
    pub output: Option<String>,
}

#[derive(Debug)]
pub struct DndIcon {
    pub surface: WlSurface,
    pub offset: Point<i32, Logical>,
}

delegate_compositor!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> DataDeviceHandler for AnvilState<BackendData> {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl<BackendData: Backend> ClientDndGrabHandler for AnvilState<BackendData> {
    fn started(&mut self, _source: Option<WlDataSource>, icon: Option<WlSurface>, _seat: Seat<Self>) {
        let offset = if let CursorImageStatus::Surface(ref surface) = self.cursor_status {
            with_states(surface, |states| {
                let hotspot = states
                    .data_map
                    .get::<CursorImageSurfaceData>()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .hotspot;
                Point::from((-hotspot.x, -hotspot.y))
            })
        } else {
            (0, 0).into()
        };
        self.dnd_icon = icon.map(|surface| DndIcon { surface, offset });
        self.request_repaint();
    }
    fn dropped(&mut self, _target: Option<WlSurface>, _validated: bool, _seat: Seat<Self>) {
        self.dnd_icon = None;
        self.request_repaint();
    }
}
impl<BackendData: Backend> ServerDndGrabHandler for AnvilState<BackendData> {
    fn send(&mut self, _mime_type: String, _fd: OwnedFd, _seat: Seat<Self>) {
        unreachable!("Anvil doesn't do server-side grabs");
    }
}
delegate_data_device!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> OutputHandler for AnvilState<BackendData> {}
delegate_output!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> SelectionHandler for AnvilState<BackendData> {
    type SelectionUserData = ();

    #[cfg(feature = "xwayland")]
    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.new_selection(ty, source.map(|source| source.mime_types())) {
                warn!(?err, ?ty, "Failed to set Xwayland selection");
            }
        }
    }

    #[cfg(feature = "xwayland")]
    fn send_selection(
        &mut self,
        ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &(),
    ) {
        if let Some(xwm) = self.xwm.as_mut() {
            if let Err(err) = xwm.send_selection(ty, mime_type, fd, self.handle.clone()) {
                warn!(?err, "Failed to send primary (X11 -> Wayland)");
            }
        }
    }
}

impl<BackendData: Backend> PrimarySelectionHandler for AnvilState<BackendData> {
    fn primary_selection_state(&self) -> &PrimarySelectionState {
        &self.primary_selection_state
    }
}
delegate_primary_selection!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> DataControlHandler for AnvilState<BackendData> {
    fn data_control_state(&self) -> &DataControlState {
        &self.data_control_state
    }
}

delegate_data_control!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> ShmHandler for AnvilState<BackendData> {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}
delegate_shm!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> SeatHandler for AnvilState<BackendData> {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = PointerFocusTarget;
    type TouchFocus = PointerFocusTarget;

    fn seat_state(&mut self) -> &mut SeatState<AnvilState<BackendData>> {
        &mut self.seat_state
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, target: Option<&KeyboardFocusTarget>) {
        if let Some(KeyboardFocusTarget::Window(window)) = target {
            let window = WindowElement(window.clone());
            let available: Vec<_> = self.space.elements().map(|w| w.id()).collect();
            self.window_cycle.retain(&available);
            self.window_cycle.focus(window.id());
            // Alt+Tab must reveal the selected app even when a game retains
            // its fullscreen state. Re-enable the fullscreen render path when
            // that game regains focus; don't resize or unfullscreen it.
            if let Some(output) = self.window_home(&window) {
                output.user_data().insert_if_missing(FullscreenSurface::default);
                let fullscreen = output.user_data().get::<FullscreenSurface>().unwrap();
                if window_is_fullscreen(&window) { fullscreen.set(window); }
                else { fullscreen.clear(); }
            }
            self.layout.dirty = true;
        } else {
            self.window_cycle.finish();
        }
        let dh = &self.display_handle;

        let wl_surface = target.and_then(WaylandFocus::wl_surface);

        let focus = wl_surface.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, focus.clone());
        set_primary_focus(dh, seat, focus);
    }
    fn cursor_image(&mut self, _seat: &Seat<Self>, image: CursorImageStatus) {
        // Over a window's resize ring the compositor owns the shape. Whatever
        // is under the ring — the desktop, another window's shadow — keeps
        // asking for its own arrow, and it must not paint over the handle.
        if self.resize_cursor.is_some() {
            return;
        }
        self.cursor_status = image;
        self.request_repaint();
    }

    fn led_state_changed(&mut self, _seat: &Seat<Self>, led_state: LedState) {
        self.backend_data.update_led_state(led_state)
    }
}
delegate_seat!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> TabletSeatHandler for AnvilState<BackendData> {
    fn tablet_tool_image(&mut self, _tool: &TabletToolDescriptor, image: CursorImageStatus) {
        // TODO: tablet tools should have their own cursors
        self.cursor_status = image;
        self.request_repaint();
    }
}
delegate_tablet_manager!(@<BackendData: Backend + 'static> AnvilState<BackendData>);
smithay::delegate_cursor_shape!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_text_input_manager!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> InputMethodHandler for AnvilState<BackendData> {
    fn new_popup(&mut self, surface: PopupSurface) {
        if let Err(err) = self.popups.track_popup(PopupKind::from(surface)) {
            warn!("Failed to track popup: {}", err);
        }
    }

    fn popup_repositioned(&mut self, _: PopupSurface) {}

    fn dismiss_popup(&mut self, surface: PopupSurface) {
        if let Some(parent) = surface.get_parent().map(|parent| parent.surface.clone()) {
            let _ = PopupManager::dismiss_popup(&parent, &PopupKind::from(surface));
        }
    }

    fn parent_geometry(&self, parent: &WlSurface) -> Rectangle<i32, smithay::utils::Logical> {
        self.space
            .elements()
            .find_map(|window| (window.wl_surface().as_deref() == Some(parent)).then(|| window.geometry()))
            .unwrap_or_default()
    }
}

delegate_input_method_manager!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> KeyboardShortcutsInhibitHandler for AnvilState<BackendData> {
    fn keyboard_shortcuts_inhibit_state(&mut self) -> &mut KeyboardShortcutsInhibitState {
        &mut self.keyboard_shortcuts_inhibit_state
    }

    fn new_inhibitor(&mut self, inhibitor: KeyboardShortcutsInhibitor) {
        // Just grant the wish for everyone
        inhibitor.activate();
    }
}

delegate_keyboard_shortcuts_inhibit!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_virtual_keyboard_manager!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_pointer_gestures!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_relative_pointer!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> PointerConstraintsHandler for AnvilState<BackendData> {
    fn new_constraint(&mut self, _surface: &WlSurface, _pointer: &PointerHandle<Self>) {
        self.activate_pointer_constraint();
    }

    fn cursor_position_hint(
        &mut self,
        _surface: &WlSurface,
        _pointer: &PointerHandle<Self>,
        _location: Point<f64, Logical>,
    ) {
        // Hints are advisory positions for *after* unlocking. Smithay calls this
        // on commit, while the lock is still active; moving here breaks the lock.
        // Keep the cursor at its existing position when the lock is released.
    }

}
delegate_pointer_constraints!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_viewporter!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> XdgActivationHandler for AnvilState<BackendData> {
    fn activation_state(&mut self) -> &mut XdgActivationState {
        &mut self.xdg_activation_state
    }

    fn token_created(&mut self, _token: XdgActivationToken, data: XdgActivationTokenData) -> bool {
        if let Some((serial, seat)) = data.serial {
            let keyboard = self.seat.get_keyboard().unwrap();
            Seat::from_resource(&seat) == Some(self.seat.clone())
                && keyboard
                    .last_enter()
                    .map(|last_enter| serial.is_no_older_than(&last_enter))
                    .unwrap_or(false)
        } else {
            false
        }
    }

    fn request_activation(
        &mut self,
        _token: XdgActivationToken,
        token_data: XdgActivationTokenData,
        surface: WlSurface,
    ) {
        if token_data.timestamp.elapsed().as_secs() < 10 {
            // Just grant the wish
            let w = self
                .space
                .elements()
                .find(|window| window.wl_surface().map(|s| *s == surface).unwrap_or(false))
                .cloned();
            if let Some(window) = w {
                self.activate_window(&window);
            }
        }
    }
}
delegate_xdg_activation!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> XdgDecorationHandler for AnvilState<BackendData> {
    // MindOS draws the title bars (shell/ssd.rs): every window that talks
    // xdg-decoration gets server-side decorations unless it insists on
    // drawing its own.
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
    }
    fn request_mode(&mut self, toplevel: ToplevelSurface, mode: DecorationMode) {
        use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;

        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(match mode {
                DecorationMode::ClientSide => Mode::ClientSide,
                _ => Mode::ServerSide,
            });
        });

        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
        self.layout.dirty = true;
    }
    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        use xdg_decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });

        if toplevel.is_initial_configure_sent() {
            toplevel.send_pending_configure();
        }
        self.layout.dirty = true;
    }
}
delegate_xdg_decoration!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

delegate_xdg_shell!(@<BackendData: Backend + 'static> AnvilState<BackendData>);
delegate_layer_shell!(@<BackendData: Backend + 'static> AnvilState<BackendData>);
delegate_presentation!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> FractionalScaleHandler for AnvilState<BackendData> {
    fn new_fractional_scale(
        &mut self,
        surface: smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    ) {
        // Here we can set the initial fractional scale
        //
        // First we look if the surface already has a primary scan-out output, if not
        // we test if the surface is a subsurface and try to use the primary scan-out output
        // of the root surface. If the root also has no primary scan-out output we just try
        // to use the first output of the toplevel.
        // If the surface is the root we also try to use the first output of the toplevel.
        //
        // If all the above tests do not lead to a output we just use the first output
        // of the space (which in case of anvil will also be the output a toplevel will
        // initially be placed on)
        #[allow(clippy::redundant_clone)]
        let mut root = surface.clone();
        while let Some(parent) = get_parent(&root) {
            root = parent;
        }

        with_states(&surface, |states| {
            let primary_scanout_output = surface_primary_scanout_output(&surface, states)
                .or_else(|| {
                    if root != surface {
                        with_states(&root, |states| {
                            surface_primary_scanout_output(&root, states).or_else(|| {
                                self.window_for_surface(&root).and_then(|window| {
                                    self.space.outputs_for_element(&window).first().cloned()
                                })
                            })
                        })
                    } else {
                        self.window_for_surface(&root)
                            .and_then(|window| self.space.outputs_for_element(&window).first().cloned())
                    }
                })
                .or_else(|| self.space.outputs().next().cloned());
            if let Some(output) = primary_scanout_output {
                with_fractional_scale(states, |fractional_scale| {
                    fractional_scale.set_preferred_scale(output.current_scale().fractional_scale());
                });
            }
        });
    }
}
delegate_fractional_scale!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend + 'static> SecurityContextHandler for AnvilState<BackendData> {
    fn context_created(&mut self, source: SecurityContextListenerSource, security_context: SecurityContext) {
        self.handle
            .insert_source(source, move |client_stream, _, data| {
                let client_state = ClientState {
                    security_context: Some(security_context.clone()),
                    ..ClientState::default()
                };
                if let Err(err) = data
                    .display_handle
                    .insert_client(client_stream, Arc::new(client_state))
                {
                    warn!("Error adding wayland client: {}", err);
                };
            })
            .expect("Failed to init wayland socket source");
    }
}
delegate_security_context!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

#[cfg(feature = "xwayland")]
impl<BackendData: Backend + 'static> XWaylandKeyboardGrabHandler for AnvilState<BackendData> {
    fn keyboard_focus_for_xsurface(&self, surface: &WlSurface) -> Option<KeyboardFocusTarget> {
        let elem = self
            .space
            .elements()
            .find(|elem| elem.wl_surface().as_deref() == Some(surface))?;
        Some(KeyboardFocusTarget::Window(elem.0.clone()))
    }
}
#[cfg(feature = "xwayland")]
delegate_xwayland_keyboard_grab!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

#[cfg(feature = "xwayland")]
delegate_xwayland_shell!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend> XdgForeignHandler for AnvilState<BackendData> {
    fn xdg_foreign_state(&mut self) -> &mut XdgForeignState {
        &mut self.xdg_foreign_state
    }
}
smithay::delegate_xdg_foreign!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

smithay::delegate_single_pixel_buffer!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

smithay::delegate_fifo!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

smithay::delegate_commit_timing!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

smithay::delegate_idle_notify!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

smithay::delegate_idle_inhibit!(@<BackendData: Backend + 'static> AnvilState<BackendData>);

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    /// Choose the display at invocation, then keep the prompt there as the
    /// pointer moves. A disconnected display is replaced by the next active one.
    pub fn target_mindbar(&mut self) {
        if self.mindbar.open && self.mindbar.output.as_ref().is_some_and(|name| self.space.outputs().any(|o| o.name() == *name)) {
            return;
        }
        self.mindbar.output = self.space.output_under(self.pointer.current_location()).next().cloned()
            .or_else(|| self.focused_window().and_then(|w| self.window_home(&w)))
            .or_else(|| self.space.outputs().next().cloned()).map(|o| o.name());
    }

    pub fn init(
        display: Display<AnvilState<BackendData>>,
        handle: LoopHandle<'static, AnvilState<BackendData>>,
        backend_data: BackendData,
        listen_on_socket: bool,
    ) -> AnvilState<BackendData> {
        let dh = display.handle();

        let clock = Clock::new();

        // MindOS: configuration, theme and the Mind daemon connection.
        let config = Config::load();
        crate::drawing::set_background(config.background());
        let (mind_tx, mind_rx) = channel::channel::<MindEvent>();
        handle
            .insert_source(mind_rx, |event, _, state| {
                if let channel::Event::Msg(event) = event {
                    state.on_mind_event(event);
                    state.request_repaint();
                }
            })
            .expect("Failed to insert the Mind channel into the event loop");
        let mind = MindClient::start(std::path::PathBuf::from(&config.mind.socket), mind_tx);
        let (media_tx, media_rx) = channel::channel::<crate::media::Feedback>();
        handle.insert_source(media_rx, |event, _, state| {
            if let channel::Event::Msg(message) = event {
                if !state.idle.locked && !state.config.session.kiosk {
                    state.mindbar.show_osd(message);
                    state.request_repaint();
                }
            }
        }).expect("insert media feedback channel");
        let media_keys = crate::media::MediaKeys::start(media_tx);
        let mut mindbar = MindBar::new(
            TextRenderer::new(),
            Vec::new(),
            config.foreground(),
            config.accent(),
        );
        let (apps_tx, apps_rx) = channel::channel::<Vec<launcher::AppEntry>>();
        handle.insert_source(apps_rx, |event, _, state| {
            if let channel::Event::Msg(apps) = event {
                state.mindbar.set_apps(apps);
                state.request_repaint();
            }
        }).expect("insert application index channel");
        launcher::watch_apps(apps_tx);
        let mut prefs = Prefs::load();
        if let Err(error) = prefs.input.validate() {
            tracing::warn!(%error, "invalid input preferences; using defaults");
            prefs.input = Default::default();
        }
        mindbar.set_show_tools(prefs.mind_show_tools.unwrap_or(config.mind.show_tools));
        // The chosen theme outranks mindwm.toml, and has to be in place before
        // the first frame: the window frames and title bars read it as they draw.
        if let Some(rgb) = prefs.accent.as_deref().and_then(crate::config::parse_accent) {
            crate::config::set_accent_rgb(rgb);
        } else if let Some(rgb) = crate::config::parse_accent(&config.theme.accent) {
            crate::config::set_accent_rgb(rgb);
        }
        if let Some(rgb) = crate::config::parse_accent(&config.theme.seam) {
            crate::config::set_seam_rgb(rgb);
        }
        mindbar.set_accent(crate::config::accent_color());
        let layout_mode = prefs
            .layout_mode
            .or_else(|| LayoutMode::parse(&config.layout.mode))
            .unwrap_or_default();
        info!(mode = layout_mode.name(), "layout mode");
        let layout = LayoutState::new(layout_mode, &config.layout);

        // init wayland clients
        let socket_name = if listen_on_socket {
            let source = ListeningSocketSource::new_auto().unwrap();
            let socket_name = source.socket_name().to_string_lossy().into_owned();
            handle
                .insert_source(source, |client_stream, _, data| {
                    if let Err(err) = data
                        .display_handle
                        .insert_client(client_stream, Arc::new(ClientState::default()))
                    {
                        warn!("Error adding wayland client: {}", err);
                    };
                })
                .expect("Failed to init wayland socket source");
            info!(name = socket_name, "Listening on wayland socket");
            Some(socket_name)
        } else {
            None
        };
        handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, data| {
                    profiling::scope!("dispatch_clients");
                    // Safety: we don't drop the display
                    unsafe {
                        display.get_mut().dispatch_clients(data).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .expect("Failed to init wayland server source");

        // init globals
        let compositor_state = CompositorState::new::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let layer_shell_state = WlrLayerShellState::new::<Self>(&dh);
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let primary_selection_state = PrimarySelectionState::new::<Self>(&dh);
        let data_control_state =
            DataControlState::new::<Self, _>(&dh, Some(&primary_selection_state), |_| true);
        let mut seat_state = SeatState::new();
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        if !config.session.kiosk {
            crate::capture::register::<BackendData>(&dh);
            handle.insert_source(
                smithay::reexports::calloop::timer::Timer::from_duration(Duration::from_secs(1)),
                |_, _, state| {
                    let outputs: Vec<_> = state.space.outputs().cloned().collect();
                    let blocked = state.idle.locked || state.idle.stage == crate::idle::Stage::Blank;
                    state.capture.maintain(&outputs, blocked);
                    smithay::reexports::calloop::timer::TimeoutAction::ToDuration(Duration::from_secs(1))
                },
            ).expect("failed to install capture maintenance timer");
        }
        let viewporter_state = ViewporterState::new::<Self>(&dh);
        let xdg_activation_state = XdgActivationState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let presentation_state = PresentationState::new::<Self>(&dh, clock.id() as u32);
        let fractional_scale_manager_state = FractionalScaleManagerState::new::<Self>(&dh);
        let xdg_foreign_state = XdgForeignState::new::<Self>(&dh);
        let single_pixel_buffer_state = SinglePixelBufferState::new::<Self>(&dh);
        let fifo_manager_state = FifoManagerState::new::<Self>(&dh);
        let commit_timing_manager_state = CommitTimingManagerState::new::<Self>(&dh);
        // ext-idle-notify-v1 / zwp_idle_inhibit_v1: programs ask how long the
        // machine has been left alone, and ask it not to idle at all (a video
        // player, a game). See `idle.rs`.
        let idle_notifier = IdleNotifierState::<Self>::new(&dh, handle.clone());
        let idle_inhibit_manager_state = IdleInhibitManagerState::new::<Self>(&dh);
        TextInputManagerState::new::<Self>(&dh);
        InputMethodManagerState::new::<Self, _>(&dh, |_client| true);
        VirtualKeyboardManagerState::new::<Self, _>(&dh, |_client| true);
        // Expose global only if backend supports relative motion events
        if BackendData::HAS_RELATIVE_MOTION {
            RelativePointerManagerState::new::<Self>(&dh);
        }
        PointerConstraintsState::new::<Self>(&dh);
        if BackendData::HAS_GESTURES {
            PointerGesturesState::new::<Self>(&dh);
        }
        TabletManagerState::new::<Self>(&dh);
        // wp_cursor_shape_v1: a client names a shape and the compositor draws
        // it from the MindOS theme. GTK 4 and most toolkits prefer this to
        // attaching their own cursor surface, which is what keeps the pointer
        // the same everywhere.
        CursorShapeManagerState::new::<Self>(&dh);
        SecurityContextState::new::<Self, _>(&dh, |client| {
            client
                .get_data::<ClientState>()
                .map_or(true, |client_state| client_state.security_context.is_none())
        });

        // init input
        let seat_name = backend_data.seat_name();
        let mut seat = seat_state.new_wl_seat(&dh, seat_name.clone());

        let pointer = seat.add_pointer();
        if seat.add_keyboard(prefs.input.xkb(), prefs.input.repeat_delay, prefs.input.repeat_rate).is_err() {
            tracing::warn!("saved keyboard layout cannot be loaded; using the system layout");
            prefs.input.keyboard_layout.clear();
            prefs.input.keyboard_variant.clear();
            prefs.input.keyboard_options.clear();
            seat.add_keyboard(XkbConfig::default(), prefs.input.repeat_delay, prefs.input.repeat_rate)
                .expect("Failed to initialize the keyboard");
        }

        let keyboard_shortcuts_inhibit_state = KeyboardShortcutsInhibitState::new::<Self>(&dh);

        #[cfg(feature = "xwayland")]
        let xwayland_shell_state = xwayland_shell::XWaylandShellState::new::<Self>(&dh.clone());

        #[cfg(feature = "xwayland")]
        XWaylandKeyboardGrabState::new::<Self>(&dh.clone());

        let mut state = AnvilState {
            backend_data,
            display_handle: dh,
            socket_name,
            running: Arc::new(AtomicBool::new(true)),
            handle,
            space: Space::default(),
            popups: PopupManager::default(),
            compositor_state,
            data_device_state,
            layer_shell_state,
            output_manager_state,
            primary_selection_state,
            data_control_state,
            seat_state,
            keyboard_shortcuts_inhibit_state,
            shm_state,
            capture: crate::capture::CaptureState::default(),
            viewporter_state,
            xdg_activation_state,
            xdg_decoration_state,
            xdg_shell_state,
            presentation_state,
            fractional_scale_manager_state,
            xdg_foreign_state,
            single_pixel_buffer_state,
            fifo_manager_state,
            commit_timing_manager_state,
            dnd_icon: None,
            suppressed_keys: Vec::new(),
            window_cycle: Default::default(),
            media_keys,
            cursor_status: CursorImageStatus::default_named(),
            resize_cursor: None,
            seat_name,
            seat,
            pointer,
            absolute_pointer_positions: HashMap::new(),
            clock,

            #[cfg(feature = "xwayland")]
            xwayland_shell_state,
            #[cfg(feature = "xwayland")]
            xwm: None,
            #[cfg(feature = "xwayland")]
            xdisplay: None,
            #[cfg(feature = "xwayland")]
            xtray: None,
            #[cfg(feature = "debug")]
            renderdoc: renderdoc::RenderDoc::new().ok(),
            show_window_preview: false,
            config,
            mindbar,
            mind,
            startup_done: false,
            xwayland_ready: false,
            ipc: IpcServer::default(),
            minimized: Vec::new(),
            super_scroll: 0.0,
            layout,
            prefs,
            idle: IdleState::default(),
            idle_notifier,
            idle_inhibit_manager_state,
        };
        state.idle.since = Some(std::time::Instant::now());
        state.start_ipc();
        state.arm_idle_timer();
        state
    }

    #[cfg(feature = "xwayland")]
    /// Own the XEmbed system tray on the XWayland display and feed its
    /// events and a read-back timer into the loop. Failure just means no
    /// legacy tray icons; the session goes on.
    #[cfg(feature = "xwayland")]
    fn start_xtray(&mut self, display: u32) {
        use smithay::reexports::calloop::channel::Event as ChannelEvent;
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};

        let (tray, source) = match crate::xtray::XTray::start(display) {
            Ok(x) => x,
            Err(err) => {
                warn!(err, "XEmbed tray host unavailable; legacy tray icons will not show");
                return;
            }
        };
        if let Err(err) = self.handle.insert_source(source, |event, _, data| {
            if let ChannelEvent::Msg(event) = event {
                if let Some(tray) = data.xtray.as_mut() {
                    tray.handle_event(event);
                }
            }
        }) {
            warn!(%err, "cannot listen for tray events");
            return;
        }
        let poll = Timer::from_duration(crate::xtray::POLL);
        if let Err(err) = self.handle.insert_source(poll, |_, _, data| {
            if let Some(tray) = data.xtray.as_mut() {
                tray.poll();
            }
            TimeoutAction::ToDuration(crate::xtray::POLL)
        }) {
            warn!(%err, "cannot schedule tray read-back");
            return;
        }
        self.xtray = Some(tray);
    }

    pub fn start_xwayland(&mut self) {
        use std::process::Stdio;

        use smithay::wayland::compositor::CompositorHandler;

        let spawned = XWayland::spawn(
            &self.display_handle,
            None,
            std::iter::empty::<(String, String)>(),
            true,
            Stdio::null(),
            Stdio::null(),
            |_| (),
        );
        let (xwayland, client) = match spawned {
            Ok(x) => x,
            Err(err) => {
                warn!(%err, "XWayland unavailable; X11 applications will not run");
                self.xwayland_ready = true;
                self.run_startup();
                return;
            }
        };

        let ret = self
            .handle
            .insert_source(xwayland, move |event, _, data| match event {
                XWaylandEvent::Ready {
                    x11_socket,
                    display_number,
                } => {
                    let xwayland_scale = std::env::var("ANVIL_XWAYLAND_SCALE")
                        .ok()
                        .and_then(|s| s.parse::<f64>().ok())
                        .unwrap_or(1.);
                    data.client_compositor_state(&client)
                        .set_client_scale(xwayland_scale);
                    let mut wm = X11Wm::start_wm(data.handle.clone(), x11_socket, client.clone())
                        .expect("Failed to attach X11 Window Manager");

                    let mut cursor = Cursor::load();
                    let (_, image) = cursor.get_image(CursorIcon::Default, 1, Duration::ZERO);
                    wm.set_cursor(
                        &image.pixels_rgba,
                        Size::from((image.width as u16, image.height as u16)),
                        Point::from((image.xhot as u16, image.yhot as u16)),
                    )
                    .expect("Failed to set xwayland default cursor");
                    data.xwm = Some(wm);
                    data.xdisplay = Some(display_number);
                    data.start_xtray(display_number);
                    data.xwayland_ready = true;
                    data.run_startup();
                }
                XWaylandEvent::Error => {
                    warn!("XWayland crashed on startup");
                    data.xwayland_ready = true;
                    data.run_startup();
                }
            });
        if let Err(e) = ret {
            tracing::error!("Failed to insert the XWaylandSource into the event loop: {}", e);
        }
    }
}

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    /// Something on screen changed (a commit, the pointer, a window moving,
    /// the bar): the outputs need a frame.
    pub fn request_repaint(&mut self) {
        BackendData::request_repaint(self);
    }

    /// A client scheduled a commit for a later frame (`wp_commit_timing_v1`):
    /// `pre_repaint` releases it, so the repaint loop has to keep coming
    /// round until it is due.
    pub fn commit_timers_pending(&self) -> bool {
        let mut pending = false;
        let mut check = |states: &smithay::wayland::compositor::SurfaceData| {
            pending |= states
                .data_map
                .get::<CommitTimerBarrierStateUserData>()
                .is_some_and(|timer| timer.lock().unwrap().next_deadline().is_some());
        };
        for window in self.space.elements() {
            window.with_surfaces(|_, states| check(states));
        }
        for output in self.space.outputs() {
            let map = smithay::desktop::layer_map_for_output(output);
            for layer_surface in map.layers() {
                layer_surface.with_surfaces(|_, states| check(states));
            }
        }
        if let CursorImageStatus::Surface(ref surface) = self.cursor_status {
            with_surfaces_surface_tree(surface, |_, states| check(states));
        }
        if let Some(surface) = self.dnd_icon.as_ref().map(|icon| &icon.surface) {
            with_surfaces_surface_tree(surface, |_, states| check(states));
        }
        pending
    }

    pub fn pre_repaint(&mut self, output: &Output, frame_target: impl Into<Time<Monotonic>>) {
        let frame_target = frame_target.into();

        #[allow(clippy::mutable_key_type)]
        let mut clients: HashMap<ClientId, Client> = HashMap::new();
        self.space.elements().for_each(|window| {
            window.with_surfaces(|surface, states| {
                if let Some(mut commit_timer_state) = states
                    .data_map
                    .get::<CommitTimerBarrierStateUserData>()
                    .map(|commit_timer| commit_timer.lock().unwrap())
                {
                    commit_timer_state.signal_until(frame_target);
                    let client = surface.client().unwrap();
                    clients.insert(client.id(), client);
                }
            });
        });

        let map = smithay::desktop::layer_map_for_output(output);
        for layer_surface in map.layers() {
            layer_surface.with_surfaces(|surface, states| {
                if let Some(mut commit_timer_state) = states
                    .data_map
                    .get::<CommitTimerBarrierStateUserData>()
                    .map(|commit_timer| commit_timer.lock().unwrap())
                {
                    commit_timer_state.signal_until(frame_target);
                    let client = surface.client().unwrap();
                    clients.insert(client.id(), client);
                }
            });
        }
        // Drop the lock to the layer map before calling blocker_cleared, which might end up
        // calling the commit handler which in turn again could access the layer map.
        std::mem::drop(map);

        if let CursorImageStatus::Surface(ref surface) = self.cursor_status {
            with_surfaces_surface_tree(surface, |surface, states| {
                if let Some(mut commit_timer_state) = states
                    .data_map
                    .get::<CommitTimerBarrierStateUserData>()
                    .map(|commit_timer| commit_timer.lock().unwrap())
                {
                    commit_timer_state.signal_until(frame_target);
                    let client = surface.client().unwrap();
                    clients.insert(client.id(), client);
                }
            });
        }

        if let Some(surface) = self.dnd_icon.as_ref().map(|icon| &icon.surface) {
            with_surfaces_surface_tree(surface, |surface, states| {
                if let Some(mut commit_timer_state) = states
                    .data_map
                    .get::<CommitTimerBarrierStateUserData>()
                    .map(|commit_timer| commit_timer.lock().unwrap())
                {
                    commit_timer_state.signal_until(frame_target);
                    let client = surface.client().unwrap();
                    clients.insert(client.id(), client);
                }
            });
        }

        let dh = self.display_handle.clone();
        for client in clients.into_values() {
            self.client_compositor_state(&client).blocker_cleared(self, &dh);
        }
    }

    pub fn post_repaint(
        &mut self,
        output: &Output,
        time: impl Into<Duration>,
        dmabuf_feedback: Option<SurfaceDmabufFeedback>,
        render_element_states: &RenderElementStates,
    ) {
        let time = time.into();
        let throttle = Some(Duration::from_secs(1));
        // A fullscreen window is drawn on the output that holds it, which is
        // not always the output the space still has it on: fullscreening does
        // not move the window. Without this it would miss the frame callbacks
        // of the output it is actually on and fall back to the one-second beat
        // meant for windows nobody can see — one frame a second, full screen.
        let fullscreen = output
            .user_data()
            .get::<FullscreenSurface>()
            .and_then(|surface| surface.get());

        #[allow(clippy::mutable_key_type)]
        let mut clients: HashMap<ClientId, Client> = HashMap::new();

        self.space.elements().for_each(|window| {
            window.with_surfaces(|surface, states| {
                let primary_scanout_output = surface_primary_scanout_output(surface, states);

                if let Some(output) = primary_scanout_output.as_ref() {
                    with_fractional_scale(states, |fraction_scale| {
                        fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                    });
                }

                if primary_scanout_output
                    .as_ref()
                    .map(|o| o == output)
                    .unwrap_or(true)
                {
                    let fifo_barrier = states
                        .cached_state
                        .get::<FifoBarrierCachedState>()
                        .current()
                        .barrier
                        .take();

                    if let Some(fifo_barrier) = fifo_barrier {
                        fifo_barrier.signal();
                        let client = surface.client().unwrap();
                        clients.insert(client.id(), client);
                    }
                }
            });

            if self.space.outputs_for_element(window).contains(output) || fullscreen.as_ref() == Some(window) {
                window.send_frame(output, time, throttle, surface_primary_scanout_output);
                if let Some(dmabuf_feedback) = dmabuf_feedback.as_ref() {
                    window.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                        select_dmabuf_feedback(
                            surface,
                            render_element_states,
                            &dmabuf_feedback.render_feedback,
                            &dmabuf_feedback.scanout_feedback,
                        )
                    });
                }
            }
        });
        let map = smithay::desktop::layer_map_for_output(output);
        for layer_surface in map.layers() {
            layer_surface.with_surfaces(|surface, states| {
                let primary_scanout_output = surface_primary_scanout_output(surface, states);

                if let Some(output) = primary_scanout_output.as_ref() {
                    with_fractional_scale(states, |fraction_scale| {
                        fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                    });
                }

                if primary_scanout_output
                    .as_ref()
                    .map(|o| o == output)
                    .unwrap_or(true)
                {
                    let fifo_barrier = states
                        .cached_state
                        .get::<FifoBarrierCachedState>()
                        .current()
                        .barrier
                        .take();

                    if let Some(fifo_barrier) = fifo_barrier {
                        fifo_barrier.signal();
                        let client = surface.client().unwrap();
                        clients.insert(client.id(), client);
                    }
                }
            });

            layer_surface.send_frame(output, time, throttle, surface_primary_scanout_output);
            if let Some(dmabuf_feedback) = dmabuf_feedback.as_ref() {
                layer_surface.send_dmabuf_feedback(output, surface_primary_scanout_output, |surface, _| {
                    select_dmabuf_feedback(
                        surface,
                        render_element_states,
                        &dmabuf_feedback.render_feedback,
                        &dmabuf_feedback.scanout_feedback,
                    )
                });
            }
        }
        // Drop the lock to the layer map before calling blocker_cleared, which might end up
        // calling the commit handler which in turn again could access the layer map.
        std::mem::drop(map);

        if let CursorImageStatus::Surface(ref surface) = self.cursor_status {
            with_surfaces_surface_tree(surface, |surface, states| {
                let primary_scanout_output = surface_primary_scanout_output(surface, states);

                if let Some(output) = primary_scanout_output.as_ref() {
                    with_fractional_scale(states, |fraction_scale| {
                        fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                    });
                }

                if primary_scanout_output
                    .as_ref()
                    .map(|o| o == output)
                    .unwrap_or(true)
                {
                    let fifo_barrier = states
                        .cached_state
                        .get::<FifoBarrierCachedState>()
                        .current()
                        .barrier
                        .take();

                    if let Some(fifo_barrier) = fifo_barrier {
                        fifo_barrier.signal();
                        let client = surface.client().unwrap();
                        clients.insert(client.id(), client);
                    }
                }
            });
        }

        if let Some(surface) = self.dnd_icon.as_ref().map(|icon| &icon.surface) {
            with_surfaces_surface_tree(surface, |surface, states| {
                let primary_scanout_output = surface_primary_scanout_output(surface, states);

                if let Some(output) = primary_scanout_output.as_ref() {
                    with_fractional_scale(states, |fraction_scale| {
                        fraction_scale.set_preferred_scale(output.current_scale().fractional_scale());
                    });
                }

                if primary_scanout_output
                    .as_ref()
                    .map(|o| o == output)
                    .unwrap_or(true)
                {
                    let fifo_barrier = states
                        .cached_state
                        .get::<FifoBarrierCachedState>()
                        .current()
                        .barrier
                        .take();

                    if let Some(fifo_barrier) = fifo_barrier {
                        fifo_barrier.signal();
                        let client = surface.client().unwrap();
                        clients.insert(client.id(), client);
                    }
                }
            });
        }

        let dh = self.display_handle.clone();
        for client in clients.into_values() {
            self.client_compositor_state(&client).blocker_cleared(self, &dh);
        }
    }
}

pub fn update_primary_scanout_output(
    space: &Space<WindowElement>,
    output: &Output,
    dnd_icon: &Option<DndIcon>,
    cursor_status: &CursorImageStatus,
    render_element_states: &RenderElementStates,
) {
    space.elements().for_each(|window| {
        window.with_surfaces(|surface, states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                states,
                render_element_states,
                default_primary_scanout_output_compare,
            );
        });
    });
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.with_surfaces(|surface, states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                states,
                render_element_states,
                default_primary_scanout_output_compare,
            );
        });
    }

    if let CursorImageStatus::Surface(ref surface) = cursor_status {
        with_surfaces_surface_tree(surface, |surface, states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                states,
                render_element_states,
                default_primary_scanout_output_compare,
            );
        });
    }

    if let Some(surface) = dnd_icon.as_ref().map(|icon| &icon.surface) {
        with_surfaces_surface_tree(surface, |surface, states| {
            update_surface_primary_scanout_output(
                surface,
                output,
                states,
                render_element_states,
                default_primary_scanout_output_compare,
            );
        });
    }
}

#[derive(Debug, Clone)]
pub struct SurfaceDmabufFeedback {
    pub render_feedback: DmabufFeedback,
    pub scanout_feedback: DmabufFeedback,
}

#[profiling::function]
pub fn take_presentation_feedback(
    output: &Output,
    space: &Space<WindowElement>,
    render_element_states: &RenderElementStates,
) -> OutputPresentationFeedback {
    let mut output_presentation_feedback = OutputPresentationFeedback::new(output);

    space.elements().for_each(|window| {
        if space.outputs_for_element(window).contains(output) {
            window.take_presentation_feedback(
                &mut output_presentation_feedback,
                surface_primary_scanout_output,
                |surface, _| surface_presentation_feedback_flags_from_states(surface, render_element_states),
            );
        }
    });
    let map = smithay::desktop::layer_map_for_output(output);
    for layer_surface in map.layers() {
        layer_surface.take_presentation_feedback(
            &mut output_presentation_feedback,
            surface_primary_scanout_output,
            |surface, _| surface_presentation_feedback_flags_from_states(surface, render_element_states),
        );
    }

    output_presentation_feedback
}

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    /// Environment for programs spawned by the compositor.
    pub fn child_env(&self) -> Vec<(String, String)> {
        let mut env = vec![
            ("XDG_CURRENT_DESKTOP".to_string(), "MindOS".to_string()),
            ("XDG_SESSION_TYPE".to_string(), "wayland".to_string()),
        ];
        if let Some(socket) = &self.socket_name {
            env.push(("WAYLAND_DISPLAY".into(), socket.clone()));
        }
        #[cfg(feature = "xwayland")]
        if let Some(display) = self.xdisplay {
            env.push(("DISPLAY".into(), format!(":{display}")));
        }
        if let Some(path) = self.ipc.path() {
            env.push(("MINDWM_SOCKET".into(), path.to_string_lossy().into_owned()));
        }
        env
    }

    /// Run a command line through `sh -c` inside the session.
    pub fn spawn_shell(&self, cmd: &str) {
        info!(cmd, "spawning");
        let result = std::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .envs(self.child_env())
            .stdin(std::process::Stdio::null())
            .spawn();
        match result {
            Ok(mut child) => {
                // Reap launched apps without waiting on the input/render loop.
                // A stale Octopi process also confuses its transaction helper's
                // check for the running software manager.
                std::thread::spawn(move || { let _ = child.wait(); });
            }
            Err(err) => tracing::error!(cmd, %err, "failed to spawn"),
        }
    }

    pub fn spawn_terminal(&self) {
        let terminal = self.config.apps.terminal.clone();
        self.spawn_shell(&terminal);
    }

    /// Spawn `[startup].exec` once the Wayland socket (and XWayland) are usable.
    pub fn run_startup(&mut self) {
        if self.startup_done {
            return;
        }
        #[cfg(feature = "xwayland")]
        if !self.xwayland_ready {
            return;
        }
        self.startup_done = true;
        for cmd in self.config.startup.exec.clone() {
            self.spawn_shell(&cmd);
        }
    }

    pub fn focused_window(&self) -> Option<WindowElement> {
        let keyboard = self.seat.get_keyboard()?;
        match keyboard.current_focus()? {
            KeyboardFocusTarget::Window(window) => Some(WindowElement(window)),
            _ => None,
        }
    }

    /// Give a window keyboard focus.
    pub fn focus_window(&mut self, window: &WindowElement) {
        if self.idle.locked {
            return;
        }
        let serial = crate::input_handler::next_serial();
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.set_focus(self, Some(KeyboardFocusTarget::Window(window.0.clone())), serial);
        }
    }

    /// Game mode: whatever window is on top gets the keyboard when the focused
    /// window went away (closed, unmapped, crashed), so the user never has to
    /// click into the next window.
    pub fn refresh_focus(&mut self) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        if self.idle.locked {
            // The lock screen owns the keyboard; nothing behind it gets it back.
            let holds_lock = matches!(
                keyboard.current_focus(),
                Some(KeyboardFocusTarget::LayerSurface(layer))
                    if layer.alive() && layer.namespace() == crate::idle::LOCK_NAMESPACE
            );
            if !holds_lock && keyboard.current_focus().is_some() {
                let serial = crate::input_handler::next_serial();
                keyboard.set_focus(self, None, serial);
            }
            return;
        }
        let focus_ok = match keyboard.current_focus() {
            None => false,
            Some(KeyboardFocusTarget::Window(window)) => {
                window.alive() && self.space.elements().any(|e| e.0 == window)
            }
            // a panel or popup that is gone must not keep the keyboard
            Some(KeyboardFocusTarget::LayerSurface(layer)) => layer.alive(),
            // xdg popups, grabs: leave them alone
            Some(_) => true,
        };
        if focus_ok {
            return;
        }
        let top = self.space.elements().last().cloned();
        if top.is_none() && keyboard.current_focus().is_none() {
            // nothing to focus and nothing stale to clear
            return;
        }
        let serial = crate::input_handler::next_serial();
        match top {
            Some(window) => {
                keyboard.set_focus(self, Some(KeyboardFocusTarget::Window(window.0.clone())), serial)
            }
            None => keyboard.set_focus(self, None, serial),
        }
    }

    /// Surfaces come and go under a pointer that does not move (a panel
    /// maps, a window closes, a popup disappears): point the pointer at
    /// whatever is under it now, as a real motion would, so the next click
    /// lands on it. Called once per event-loop turn.
    pub fn refresh_pointer_focus(&mut self) {
        if self.pointer.is_grabbed() {
            return;
        }
        let location = self.pointer.current_location();
        // Windows move and resize out from under a still pointer too, so the
        // resize ring it is on — or has just left — is re-read here as well.
        self.update_resize_cursor(location);
        let under = self.surface_under(location);
        if under.as_ref().map(|(target, _)| target) == self.pointer.current_focus().as_ref() {
            return;
        }
        let pointer = self.pointer.clone();
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location,
                serial: crate::input_handler::next_serial(),
                time: self.clock.now().as_millis(),
            },
        );
        pointer.frame(self);
    }

    /// Switch recent visible windows, preserving order until Alt/Super release.
    pub fn cycle_windows(&mut self, reverse: bool, modifier: crate::window_cycle::CycleModifier) {
        let available: Vec<_> = self.space.elements().rev().map(|w| w.id()).collect();
        let focused = self.focused_window().map(|w| w.id());
        if let Some(id) = self.window_cycle.step(&available, focused, reverse, modifier) {
            if let Some(window) = self.window_by_id(id) { self.activate_window(&window); }
        }
    }

    pub fn cancel_window_cycle(&mut self) {
        if let Some(id) = self.window_cycle.cancel() {
            if let Some(window) = self.window_by_id(id).filter(|w| !self.is_minimized(w)) {
                self.activate_window(&window);
            }
        }
    }

    pub fn on_mind_event(&mut self, event: MindEvent) {
        if let MindEvent::Event(MindDaemonEvent::ClientTool { id, name, args }) = &event {
            let (ok, result) = self.run_client_tool(name, args);
            self.mind.tool_result(id, ok, result);
        }
        self.mindbar.on_mind_event(event);
    }

    fn run_client_tool(&mut self, name: &str, args: &Value) -> (bool, Value) {
        match name {
            "launch_app" => {
                let wanted = args.get("name").and_then(Value::as_str).unwrap_or("");
                match launcher::find_by_name(self.mindbar.apps(), wanted) {
                    Some(app) => {
                        let cmd = if app.terminal {
                            self.config.apps.terminal_command(&app.exec)
                        } else {
                            app.launch_command()
                        };
                        let name = app.name.clone();
                        self.spawn_shell(&cmd);
                        (true, json!({"launched": name, "exec": cmd}))
                    }
                    None => (false, json!({"error": format!("no installed application matches '{wanted}'")})),
                }
            }
            "open_terminal" => {
                self.spawn_terminal();
                (true, json!({"opened": self.config.apps.terminal}))
            }
            "run_in_terminal" => {
                let command = args.get("command").and_then(Value::as_str).unwrap_or("").to_string();
                if command.is_empty() {
                    return (false, json!({"error": "command is required"}));
                }
                let script = format!("{command}; echo; echo '[finished - press Enter to close]'; read _");
                let cmd = self.config.apps.terminal_command(&script);
                self.spawn_shell(&cmd);
                (true, json!({"started": command}))
            }
            _ => (false, json!({"error": format!("unknown client tool {name}")})),
        }
    }

    pub fn handle_bar_action(&mut self, action: BarAction) {
        match action {
            BarAction::None | BarAction::Close => {}
            BarAction::Launch { exec, terminal } => {
                let cmd = if terminal {
                    self.config.apps.terminal_command(&exec)
                } else {
                    exec
                };
                self.spawn_shell(&cmd);
            }
            BarAction::Ask(text) => {
                let session = self.mindbar.session();
                self.mind.chat(session.as_deref(), &text, self.config.mind.autopilot);
            }
            BarAction::RunShell(cmd) => self.spawn_shell(&cmd),
            BarAction::Confirm { id, approve } => self.mind.confirm(&id, approve),
            BarAction::Cancel => self.mind.cancel(),
            BarAction::Copy(text) => self.copy_to_clipboard(&text),
        }
    }

    /// Put text on the clipboard through wl-copy (part of the session's
    /// wl-clipboard dependency); the compositor itself has no clipboard
    /// source of its own to offer.
    pub fn copy_to_clipboard(&self, text: &str) {
        use std::io::Write;
        let child = std::process::Command::new("wl-copy")
            .arg("--type").arg("text/plain;charset=utf-8")
            .envs(self.child_env())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                // wl-copy stays alive as the selection owner; do not wait on it.
            }
            Err(err) => tracing::warn!(%err, "wl-copy is unavailable, nothing was copied"),
        }
    }

    // ---------------------------------------------------------------------
    // Shell-facing window management (docs/SHELL.md)
    // ---------------------------------------------------------------------

    /// A mapped or minimised window by its IPC id.
    pub fn window_by_id(&self, id: u64) -> Option<WindowElement> {
        self.space
            .elements()
            .find(|w| w.id() == id)
            .cloned()
            .or_else(|| self.minimized.iter().find(|m| m.window.id() == id).map(|m| m.window.clone()))
    }

    pub fn is_minimized(&self, window: &WindowElement) -> bool {
        self.minimized.iter().any(|m| &m.window == window)
    }

    /// Raise a window to the top of the stack and give it the keyboard.
    pub fn activate_window(&mut self, window: &WindowElement) {
        if self.idle.locked || self.is_minimized(window) {
            return;
        }
        // A parent must never cover its dialogs when clicked or activated.
        // Keep nested native and Wine dialogs above it, in their existing order.
        let mut group = vec![window.clone()];
        let mut index = 0;
        while let Some(parent) = group.get(index).cloned() {
            let children: Vec<_> = self.space.elements().filter(|child| {
                if group.contains(child) { return false; }
                if let Some(surface) = child.0.toplevel().and_then(|t| t.parent()) {
                    return parent.wl_surface().as_deref() == Some(&surface);
                }
                #[cfg(feature = "xwayland")]
                if let (Some(child), Some(parent)) = (child.0.x11_surface(), parent.0.x11_surface()) {
                    return child.is_transient_for() == Some(parent.window_id());
                }
                false
            }).cloned().collect();
            group.extend(children);
            index += 1;
        }
        let top = group.last().unwrap().clone();
        for member in group {
            self.space.raise_element(&member, member == top);
            #[cfg(feature = "xwayland")]
            if let Some(surface) = member.0.x11_surface() {
                if let Some(xwm) = self.xwm.as_mut() {
                    let _ = xwm.raise_window(surface);
                }
            }
        }
        self.focus_window(&top);
    }

    /// Hide a window: it leaves the space (no rendering, no input, no frame
    /// callbacks) and the top-most remaining window takes the keyboard.
    pub fn minimize_window(&mut self, window: &WindowElement) {
        if self.is_minimized(window) {
            return;
        }
        let Some(location) = self.space.element_location(window) else {
            return;
        };
        let output = self.window_home(window).map(|o| o.name());
        // A fullscreen window owns its output's scanout slot; give it back while hidden.
        for o in self.space.outputs() {
            if let Some(fullscreen) = o.user_data().get::<FullscreenSurface>() {
                if fullscreen.get().as_ref() == Some(window) {
                    fullscreen.clear();
                    self.backend_data.reset_buffers(o);
                }
            }
        }
        self.space.unmap_elem(window);
        self.minimized.push(Minimized {
            window: window.clone(),
            location,
            output,
        });
        self.refresh_focus();
    }

    /// Bring a minimised window back where it was, on top and focused.
    pub fn unminimize_window(&mut self, window: &WindowElement) {
        let Some(pos) = self.minimized.iter().position(|m| &m.window == window) else {
            return;
        };
        let minimized = self.minimized.remove(pos);
        if !minimized.window.alive() {
            return;
        }
        self.space.map_element(minimized.window.clone(), minimized.location, true);
        if window_is_fullscreen(&minimized.window) {
            if let Some(output) = self.space.outputs_for_element(&minimized.window).first() {
                output.user_data().insert_if_missing(FullscreenSurface::default);
                output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .unwrap()
                    .set(minimized.window.clone());
            }
        }
        self.activate_window(&minimized.window);
    }

    /// Everything the shell needs to draw a taskbar, in creation order.
    pub fn windows_snapshot(&self) -> WindowsSnapshot {
        let focused = self.focused_window();
        let mut windows: Vec<WindowInfo> = self
            .space
            .elements()
            .filter_map(|w| {
                let output = self.window_home(w).map(|o| o.name());
                window_info(w, focused.as_ref() == Some(w), false, output, &self.display_handle)
            })
            .collect();
        windows.extend(
            self.minimized
                .iter()
                .filter_map(|m| window_info(&m.window, false, true, m.output.clone(), &self.display_handle)),
        );
        windows.sort_by_key(|w| w.id);
        WindowsSnapshot {
            focused: focused.map(|w| w.id()),
            windows,
        }
    }

    pub fn outputs_snapshot(&self) -> Vec<OutputInfo> {
        let primary = self.primary_output_name();
        let mut outputs: Vec<OutputInfo> = self
            .space
            .outputs()
            .map(|o| {
                let geo = self
                    .space
                    .output_geometry(o)
                    .unwrap_or_else(|| Rectangle::from_size((0, 0).into()));
                let props = o.physical_properties();
                let current = o.current_mode();
                let preferred = o.preferred_mode();
                let (vrr_supported, vrr) = self.backend_data.output_vrr(o);
                let name = o.name();
                OutputInfo {
                    software_rendering: self.backend_data.software_rendering(),
                    primary: primary.as_deref() == Some(name.as_str()),
                    name,
                    make: props.make,
                    model: props.model,
                    x: geo.loc.x,
                    y: geo.loc.y,
                    width: geo.size.w,
                    height: geo.size.h,
                    scale: o.current_scale().fractional_scale(),
                    refresh: current.map(|m| m.refresh as f64 / 1000.0).unwrap_or(0.0),
                    transform: transform_name(o.current_transform()).into(),
                    modes: o
                        .modes()
                        .iter()
                        .map(|m| ModeInfo {
                            width: m.size.w,
                            height: m.size.h,
                            refresh: m.refresh,
                            preferred: Some(*m) == preferred,
                            current: Some(*m) == current,
                        })
                        .collect(),
                    enabled: true,
                    vrr,
                    vrr_supported,
                    mm_width: props.size.w,
                    mm_height: props.size.h,
                }
            })
            .collect();
        outputs.extend(self.backend_data.disabled_outputs());
        outputs
    }

    /// The output the shell puts its main panels on: the user's choice when
    /// it is connected, else the first output.
    pub fn primary_output_name(&self) -> Option<String> {
        self.prefs
            .primary_output
            .clone()
            .filter(|name| self.space.outputs().any(|o| o.name() == *name))
            .or_else(|| self.space.outputs().next().map(|o| o.name()))
    }

    /// Keep every server-side title bar in step with its window (title,
    /// focus, maximised state); called once per event-loop turn.
    pub fn refresh_decorations(&mut self) {
        let focused = self.focused_window();
        let mut changed = false;
        for window in self.space.elements() {
            let mut state = window.decoration_state();
            if !state.is_ssd {
                continue;
            }
            let was_dirty = state.header_bar.is_dirty();
            window.with_title(|title| state.header_bar.set_title(title));
            state.header_bar.set_focused(focused.as_ref() == Some(window));
            state.header_bar.set_maximized(window.is_maximized());
            state.header_bar.set_tiled(self.layout.mode.is_tiling() && window.tileable());
            changed |= !was_dirty && state.header_bar.is_dirty();
        }
        if changed {
            self.request_repaint();
        }
    }

    /// Save the preferences and tell the shell.
    pub fn prefs_changed(&mut self) {
        self.prefs.save();
        let line = prefs_event(&self.prefs);
        self.ipc_broadcast(&line);
    }

    /// The shell's `set_prefs`: any subset of the preference keys.
    pub fn apply_prefs(&mut self, value: Value) -> Result<(), String> {
        let object = value.as_object().ok_or("prefs must be an object")?;
        if let Some(patch) = object.get("input") {
            let next = self.prefs.input.patched(patch.clone())?;
            let keyboard = self.seat.get_keyboard().ok_or("No keyboard available")?;
            if next.keyboard_layout != self.prefs.input.keyboard_layout
                || next.keyboard_variant != self.prefs.input.keyboard_variant
                || next.keyboard_options != self.prefs.input.keyboard_options {
                keyboard.set_xkb_config(self, next.xkb())
                    .map_err(|_| "That keyboard layout, variant or option could not be loaded")?;
            }
            keyboard.change_repeat_info(next.repeat_rate, next.repeat_delay);
            self.backend_data.set_mouse_settings(&next);
            self.prefs.input = next;
        }
        if let Some(mode) = object.get("layout_mode") {
            let name = mode.as_str().ok_or("layout_mode must be a string")?;
            let mode = LayoutMode::parse(name).ok_or_else(|| format!("unknown layout mode: {name}"))?;
            self.set_layout_mode(mode);
        }
        if let Some(show) = object.get("mind_show_tools") {
            let show = show.as_bool().ok_or("mind_show_tools must be true or false")?;
            self.prefs.mind_show_tools = Some(show);
            self.mindbar.set_show_tools(show);
        }
        if let Some(primary) = object.get("primary_output") {
            self.prefs.primary_output = match primary {
                Value::Null => None,
                Value::String(name) => Some(name.clone()),
                _ => return Err("primary_output must be a string or null".into()),
            };
        }
        if object.contains_key("cursor_theme") || object.contains_key("cursor_size") {
            let (mut theme, mut size) = crate::cursor::configured();
            if let Some(value) = object.get("cursor_theme") {
                theme = value.as_str().ok_or("cursor_theme must be a string")?.to_string();
            }
            if let Some(value) = object.get("cursor_size") {
                size = value
                    .as_u64()
                    .filter(|s| (8..=256).contains(s))
                    .ok_or("cursor_size must be a number between 8 and 256")? as u32;
            }
            self.prefs.cursor_theme = Some(theme.clone());
            self.prefs.cursor_size = Some(size);
            // The environment so programs started from now on agree, then the
            // compositor's own pointer.
            crate::cursor::configure(&theme, size);
            self.backend_data.set_cursor(&theme, size);
            tracing::info!(%theme, size, "the pointer changed");
        }
        if let Some(idle) = object.get("idle") {
            let patch = idle.as_object().ok_or("idle must be an object")?;
            let mut merged = serde_json::to_value(&self.prefs.idle)
                .ok()
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default();
            for (key, value) in patch {
                if !merged.contains_key(key) {
                    return Err(format!("unknown idle setting: {key}"));
                }
                merged.insert(key.clone(), value.clone());
            }
            let settings: crate::prefs::IdleSettings = serde_json::from_value(Value::Object(merged))
                .map_err(|err| format!("invalid idle settings: {err}"))?;
            self.prefs.idle = settings;
            // A new timeout counts from now, so shortening one does not lock
            // the screen the instant it is saved.
            self.idle.since = Some(std::time::Instant::now());
            self.arm_idle_timer();
            self.idle_changed();
        }
        if let Some(accent) = object.get("accent") {
            let rgb = match accent {
                // Back to the colour the configuration file asks for.
                Value::Null => {
                    self.prefs.accent = None;
                    crate::config::parse_accent(&self.config.theme.accent).unwrap_or(0x3d_dc97)
                }
                Value::String(text) => {
                    let rgb = crate::config::parse_accent(text)
                        .ok_or("accent must be a colour like #67dce5")?;
                    self.prefs.accent = Some(format!("#{rgb:06x}"));
                    rgb
                }
                _ => return Err("accent must be a colour or null".into()),
            };
            crate::config::set_accent_rgb(rgb);
            // The frames and title bars notice by themselves: the colour is
            // part of what they cache. The Mind bar keeps a tinted shadow.
            self.mindbar.set_accent(crate::config::accent_color());
            self.request_repaint();
        }
        self.prefs_changed();
        Ok(())
    }

    /// The Displays settings: apply a change to one output and remember it.
    pub fn set_output_config(&mut self, name: &str, change: &OutputChange) -> Result<(), String> {
        if let Some(enabled) = change.enabled {
            let was_on = self.space.outputs().any(|o| o.name() == name);
            if enabled != was_on {
                BackendData::set_output_enabled(self, name, enabled)?;
            }
            self.prefs.output_mut(name).enabled = Some(enabled);
            if !enabled {
                self.prefs_changed();
                self.after_output_change(None);
                return Ok(());
            }
        }
        let output = self
            .space
            .outputs()
            .find(|o| o.name() == name)
            .cloned()
            .ok_or_else(|| format!("no such output: {name}"))?;
        if let Some(mode) = &change.mode {
            let wl_mode = smithay::output::Mode {
                size: (mode.width, mode.height).into(),
                refresh: mode.refresh,
            };
            if !output.modes().contains(&wl_mode) {
                return Err(format!(
                    "{name} has no {}x{} @ {:.3} Hz mode",
                    mode.width,
                    mode.height,
                    mode.refresh as f64 / 1000.0
                ));
            }
            if output.current_mode() != Some(wl_mode) {
                self.backend_data.set_output_mode(&output, wl_mode)?;
                output.change_current_state(Some(wl_mode), None, None, None);
            }
            self.prefs.output_mut(name).mode = Some(mode_key(mode.width, mode.height, mode.refresh));
        }
        if let Some(scale) = change.scale {
            if !(0.5..=4.0).contains(&scale) {
                return Err("scale must be between 0.5 and 4".into());
            }
            output.change_current_state(None, None, Some(smithay::output::Scale::Fractional(scale)), None);
            self.prefs.output_mut(name).scale = Some(scale);
        }
        if let Some(transform) = &change.transform {
            let t = parse_transform(transform).ok_or_else(|| format!("unknown transform: {transform}"))?;
            output.change_current_state(None, Some(t), None, None);
            self.prefs.output_mut(name).transform = Some(transform_name(t).into());
        }
        if let Some(position) = change.position {
            self.prefs.output_mut(name).position = Some(position);
        }
        if let Some(vrr) = change.vrr {
            self.backend_data.set_output_vrr(&output, vrr)?;
            self.prefs.output_mut(name).vrr = Some(vrr);
        }
        if change.primary == Some(true) {
            self.prefs.primary_output = Some(name.to_string());
        }
        self.prefs_changed();
        self.after_output_change(Some(&output));
        Ok(())
    }

    /// Place outputs and windows again after a display change and keep the
    /// pointer on a screen.
    fn after_output_change(&mut self, output: Option<&Output>) {
        self.request_repaint();
        crate::shell::fixup_positions(
            &mut self.space,
            self.pointer.current_location(),
            &self.prefs.pinned_positions(),
        );
        self.layout.dirty = true;
        let current = self.pointer.current_location();
        let location = if self.space.output_under(current).next().is_some() {
            current
        } else {
            self.space
                .outputs()
                .next()
                .and_then(|o| self.space.output_geometry(o))
                .map(|g| {
                    (
                        g.loc.x as f64 + g.size.w as f64 / 2.0,
                        g.loc.y as f64 + g.size.h as f64 / 2.0,
                    )
                        .into()
                })
                .unwrap_or(current)
        };
        let pointer = self.pointer.clone();
        let under = self.surface_under(location);
        pointer.motion(
            self,
            under,
            &smithay::input::pointer::MotionEvent {
                location,
                serial: smithay::utils::SERIAL_COUNTER.next_serial(),
                time: self.clock.now().as_millis(),
            },
        );
        pointer.frame(self);
        if let Some(output) = output {
            self.backend_data.reset_buffers(output);
        }
    }

    /// Super+W: the shell's overview, or the built-in window preview.
    pub fn overview_shortcut(&mut self) {
        if !self.ipc_shortcut("overview") {
            self.show_window_preview = !self.show_window_preview;
        }
    }
}

fn window_is_fullscreen(window: &WindowElement) -> bool {
    if let Some(toplevel) = window.0.toplevel() {
        return toplevel
            .current_state()
            .states
            .contains(xdg_toplevel::State::Fullscreen);
    }
    #[cfg(feature = "xwayland")]
    if let Some(surface) = window.0.x11_surface() {
        return surface.is_fullscreen();
    }
    false
}

/// The IPC view of one window; `None` for windows the shell should not list
/// (override-redirect X11 windows, toplevels that have not drawn yet).
fn window_info(
    window: &WindowElement,
    focused: bool,
    minimized: bool,
    output: Option<String>,
    dh: &DisplayHandle,
) -> Option<WindowInfo> {
    let (title, app_id, fullscreen, maximized, x11, pid) = if let Some(toplevel) = window.0.toplevel() {
        let (title, app_id) = with_states(toplevel.wl_surface(), |states| {
            let data = states.data_map.get::<XdgToplevelSurfaceData>()?.lock().ok()?;
            Some((data.title.clone().unwrap_or_default(), data.app_id.clone().unwrap_or_default()))
        })
        .unwrap_or_default();
        let states = toplevel.current_state().states;
        let pid = toplevel
            .wl_surface()
            .client()
            .and_then(|client| client.get_credentials(dh).ok())
            .map(|credentials| credentials.pid as u32);
        (
            title,
            app_id,
            states.contains(xdg_toplevel::State::Fullscreen),
            states.contains(xdg_toplevel::State::Maximized),
            false,
            pid,
        )
    } else {
        #[cfg(feature = "xwayland")]
        {
            let surface = window.0.x11_surface()?;
            if surface.is_override_redirect() {
                return None;
            }
            (
                surface.title(),
                surface.class(),
                surface.is_fullscreen(),
                surface.is_maximized(),
                true,
                surface.pid(),
            )
        }
        #[cfg(not(feature = "xwayland"))]
        {
            return None;
        }
    };
    if !minimized {
        let size = window.0.geometry().size;
        if size.w <= 0 || size.h <= 0 {
            return None;
        }
    }
    Some(WindowInfo {
        id: window.id(),
        title,
        app_id,
        focused,
        fullscreen,
        maximized,
        minimized,
        x11,
        wine: crate::procinfo::is_wine(pid),
        pid,
        output,
    })
}

pub trait Backend {
    fn software_rendering(&self) -> bool { false }
    fn set_mouse_settings(&mut self, _settings: &crate::input_config::InputSettings) {}
    fn mouse_devices(&self) -> Vec<Value> { Vec::new() }
    const HAS_RELATIVE_MOTION: bool = false;
    const HAS_GESTURES: bool = false;
    fn seat_name(&self) -> String;
    fn reset_buffers(&mut self, output: &Output);
    fn early_import(&mut self, surface: &WlSurface);
    fn update_led_state(&mut self, led_state: LedState);

    /// Switch an output to one of its advertised modes.
    fn set_output_mode(&mut self, _output: &Output, _mode: smithay::output::Mode) -> Result<(), String> {
        Err("this backend cannot change video modes".into())
    }

    /// Turn variable refresh rate on or off.
    fn set_output_vrr(&mut self, _output: &Output, _enabled: bool) -> Result<(), String> {
        Err("this backend has no variable refresh rate".into())
    }

    /// `(supported, enabled)` for variable refresh rate.
    fn output_vrr(&self, _output: &Output) -> (bool, bool) {
        (false, false)
    }

    /// Reload the compositor's own pointer (the cursor it draws for a client
    /// that only names a shape). Backends that never draw one do nothing.
    fn set_cursor(&mut self, _theme: &str, _size: u32) {}

    /// Connected outputs that are switched off.
    fn disabled_outputs(&self) -> Vec<OutputInfo> {
        Vec::new()
    }

    /// Switch a connected output on or off.
    fn set_output_enabled(_state: &mut AnvilState<Self>, _name: &str, _enabled: bool) -> Result<(), String>
    where
        Self: Sized + 'static,
    {
        Err("this backend cannot switch outputs off".into())
    }

    /// Switch every display off (DPMS) or light them again. Backends without
    /// real displays do nothing, so the screensaver and the lock still work
    /// in a nested session.
    fn set_blanked(_state: &mut AnvilState<Self>, _blanked: bool)
    where
        Self: Sized + 'static,
    {
    }

    /// Something on screen changed: draw a frame at the next repaint point,
    /// or at once when the output had nothing to do. Backends that repaint
    /// on their own clock (the nested window) ignore it.
    fn request_repaint(_state: &mut AnvilState<Self>)
    where
        Self: Sized + 'static,
    {
    }

    /// A fullscreen client committed a buffer: draw the frame now rather
    /// than at the repaint point, so its flip catches the next vblank.
    fn repaint_now(_state: &mut AnvilState<Self>, _output: &Output)
    where
        Self: Sized + 'static,
    {
    }
}

/// Parse a transform the way the preferences and the shell name it.
pub fn parse_transform(name: &str) -> Option<smithay::utils::Transform> {
    use smithay::utils::Transform;
    Some(match name.trim().to_ascii_lowercase().as_str() {
        "normal" | "0" => Transform::Normal,
        "90" => Transform::_90,
        "180" => Transform::_180,
        "270" => Transform::_270,
        "flipped" => Transform::Flipped,
        "flipped-90" | "flipped90" => Transform::Flipped90,
        "flipped-180" | "flipped180" => Transform::Flipped180,
        "flipped-270" | "flipped270" => Transform::Flipped270,
        _ => return None,
    })
}

pub fn transform_name(transform: smithay::utils::Transform) -> &'static str {
    use smithay::utils::Transform;
    match transform {
        Transform::Normal => "normal",
        Transform::_90 => "90",
        Transform::_180 => "180",
        Transform::_270 => "270",
        Transform::Flipped => "flipped",
        Transform::Flipped90 => "flipped-90",
        Transform::Flipped180 => "flipped-180",
        Transform::Flipped270 => "flipped-270",
    }
}
