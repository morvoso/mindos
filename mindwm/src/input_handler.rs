use std::{convert::TryInto, process::Command, sync::atomic::Ordering};

use std::cell::RefCell;

use crate::{
    focus::PointerFocusTarget,
    layout::LayoutMode,
    shell::{
        usable_area, FullscreenSurface, PointerMoveSurfaceGrab, PointerResizeSurfaceGrab, ResizeData,
        ResizeEdge, ResizeState, SurfaceData, TileResizeGrab, WindowElement,
    },
    AnvilState,
};

#[cfg(feature = "udev")]
use crate::udev::UdevData;
#[cfg(feature = "udev")]
use smithay::backend::renderer::DebugFlags;

use smithay::{
    backend::input::{
        self, Axis, AxisSource, Device, Event, InputBackend, InputEvent, KeyState, KeyboardKeyEvent,
        PointerAxisEvent, PointerButtonEvent,
    },
    desktop::{layer_map_for_output, WindowSurfaceType},
    input::{
        keyboard::{keysyms as xkb, FilterResult, Keysym, ModifiersState},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData as PointerGrabStartData, MotionEvent},
    },
    output::Scale,
    reexports::{
        wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1,
        wayland_server::protocol::wl_pointer,
    },
    utils::{IsAlive, Logical, Point, Serial, Transform, SERIAL_COUNTER as SCOUNTER},
    wayland::{
        compositor::with_states,
        seat::WaylandFocus,
        input_method::InputMethodSeat,
        keyboard_shortcuts_inhibit::KeyboardShortcutsInhibitorSeat,
        shell::wlr_layer::{KeyboardInteractivity, Layer as WlrLayer, LayerSurfaceCachedState},
    },
};

#[cfg(any(feature = "winit", feature = "udev"))]
use smithay::backend::input::AbsolutePositionEvent;

#[cfg(feature = "winit")]
use smithay::output::Output;
use tracing::{debug, error, info};

/// How far outside a window frame a plain drag still grabs the frame, and how
/// far along a side an end still counts as a corner. The ring is entirely
/// outside the window, so a client never loses a pixel of its own to it.
const RESIZE_BAND: f64 = 8.0;
const RESIZE_CORNER: f64 = 28.0;

use crate::state::Backend;
#[cfg(feature = "udev")]
use smithay::{
    backend::{
        input::{
            DeviceCapability, GestureBeginEvent, GestureEndEvent, GesturePinchUpdateEvent as _,
            GestureSwipeUpdateEvent as _, PointerMotionEvent, ProximityState, TabletToolButtonEvent,
            TabletToolEvent, TabletToolProximityEvent, TabletToolTipEvent, TabletToolTipState, TouchEvent,
        },
        session::Session,
    },
    input::{
        pointer::{
            GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent, GesturePinchEndEvent,
            GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent,
        },
        touch::{DownEvent, UpEvent},
    },
    reexports::wayland_server::DisplayHandle,
    wayland::{
        tablet_manager::{TabletDescriptor, TabletSeatTrait},
    },
};

/// Somebody touching the machine, as opposed to a device being plugged in.
fn is_activity<B: InputBackend>(event: &InputEvent<B>) -> bool {
    !matches!(
        event,
        InputEvent::DeviceAdded { .. } | InputEvent::DeviceRemoved { .. } | InputEvent::Special(_)
    )
}

/// The edges a drag from `location` moves, for a frame spanning `x0..x1` by
/// `y0..y1`. The point is known to be in the ring around the frame: the side
/// it is on gives one edge, and being within `RESIZE_CORNER` of an end of
/// that side adds the other, the way a corner works on a frame you can see.
fn resize_edges(x0: f64, y0: f64, x1: f64, y1: f64, location: Point<f64, Logical>) -> ResizeEdge {
    let mut edges = ResizeEdge::empty();
    if location.x < x0 {
        edges |= ResizeEdge::LEFT;
    } else if location.x > x1 {
        edges |= ResizeEdge::RIGHT;
    }
    if location.y < y0 {
        edges |= ResizeEdge::TOP;
    } else if location.y > y1 {
        edges |= ResizeEdge::BOTTOM;
    }
    let corner = |lo: f64, hi: f64, v: f64, first: ResizeEdge, second: ResizeEdge| {
        let (a, b) = (v - lo, hi - v);
        if a <= RESIZE_CORNER && a <= b {
            first
        } else if b <= RESIZE_CORNER {
            second
        } else {
            ResizeEdge::empty()
        }
    };
    edges
        | if edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            corner(y0, y1, location.y, ResizeEdge::TOP, ResizeEdge::BOTTOM)
        } else {
            corner(x0, x1, location.x, ResizeEdge::LEFT, ResizeEdge::RIGHT)
        }
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn familiar_shortcuts_and_reverse_switching() {
        let alt = ModifiersState { alt: true, ..Default::default() };
        assert!(matches!(process_keyboard_shortcut(alt, Keysym::F4), Some(KeyAction::CloseWindow)));
        assert!(process_keyboard_shortcut(ModifiersState { ctrl: true, ..alt }, Keysym::F4).is_none());
        assert!(matches!(process_keyboard_shortcut(alt, Keysym::Tab),
            Some(KeyAction::CycleWindow { reverse: false, .. })));
        assert!(matches!(process_keyboard_shortcut(ModifiersState { shift: true, ..alt }, Keysym::ISO_Left_Tab),
            Some(KeyAction::CycleWindow { reverse: true, .. })));
        assert!(process_keyboard_shortcut(ModifiersState { ctrl: true, ..alt }, Keysym::Tab).is_none());
    }

    #[test]
    fn caps_lock_does_not_disable_or_invert_super_shortcuts() {
        let logo = ModifiersState { logo: true, ..Default::default() };
        assert!(matches!(process_keyboard_shortcut(logo, Keysym::Q), Some(KeyAction::CloseWindow)));
        assert!(matches!(process_keyboard_shortcut(logo, Keysym::F), Some(KeyAction::ToggleFullscreen)));
        assert!(matches!(process_keyboard_shortcut(ModifiersState { shift: true, ..logo }, Keysym::f),
            Some(KeyAction::ToggleFloating)));
        assert!(process_keyboard_shortcut(ModifiersState::default(), Keysym::q).is_none());
    }

    #[test]
    fn the_resize_ring_gives_sides_and_corners() {
        use ResizeEdge as E;
        let at = |x: f64, y: f64| resize_edges(100.0, 100.0, 500.0, 400.0, (x, y).into());
        // The middle of a side is that side alone.
        assert_eq!(at(94.0, 250.0), E::LEFT);
        assert_eq!(at(506.0, 250.0), E::RIGHT);
        assert_eq!(at(300.0, 94.0), E::TOP);
        assert_eq!(at(300.0, 406.0), E::BOTTOM);
        // The end of a side reaches round the corner.
        assert_eq!(at(94.0, 110.0), E::TOP_LEFT);
        assert_eq!(at(506.0, 395.0), E::BOTTOM_RIGHT);
        assert_eq!(at(120.0, 406.0), E::BOTTOM_LEFT);
        assert_eq!(at(480.0, 94.0), E::TOP_RIGHT);
        // Diagonally outside is the corner it is diagonal from.
        assert_eq!(at(95.0, 95.0), E::TOP_LEFT);
        assert_eq!(at(505.0, 405.0), E::BOTTOM_RIGHT);
    }
}

impl<BackendData: Backend> AnvilState<BackendData> {
    fn process_common_key_action(&mut self, action: KeyAction) {
        if self.idle.locked && !matches!(action, KeyAction::None | KeyAction::VtSwitch(_)) {
            // Locked: nothing but switching virtual terminals.
            debug!(?action, "ignored while the session is locked");
            return;
        }
        if self.config.session.kiosk && !matches!(action, KeyAction::None | KeyAction::VtSwitch(_)) {
            // The login screen: nothing starts a program, opens the Mind bar
            // or ends the compositor from the keyboard.
            debug!(?action, "ignored in kiosk mode");
            return;
        }
        match action {
            KeyAction::None => (),

            KeyAction::Quit => {
                info!("Quitting.");
                self.running.store(false, Ordering::SeqCst);
            }

            KeyAction::Run(cmd) => {
                info!(cmd, "Starting program");

                if let Err(e) = Command::new(&cmd)
                    .envs(
                        self.socket_name
                            .clone()
                            .map(|v| ("WAYLAND_DISPLAY", v))
                            .into_iter()
                            .chain(
                                #[cfg(feature = "xwayland")]
                                self.xdisplay.map(|v| ("DISPLAY", format!(":{}", v))),
                                #[cfg(not(feature = "xwayland"))]
                                None,
                            ),
                    )
                    .spawn()
                {
                    error!(cmd, err = %e, "Failed to start program");
                }
            }

            KeyAction::TogglePreview => {
                self.show_window_preview = !self.show_window_preview;
            }

            KeyAction::ToggleMindBar => {
                self.target_mindbar();
                self.mindbar.toggle();
            }
            KeyAction::Overview => self.overview_shortcut(),
            KeyAction::Terminal => self.spawn_terminal(),
            KeyAction::Screenshot { screen } => self.spawn_shell(if screen {
                "mindos-screenshot screen"
            } else {
                "mindos-screenshot area"
            }),
            KeyAction::CloseWindow => {
                if let Some(window) = self.focused_window() {
                    window.close();
                }
            }
            KeyAction::LockScreen => self.set_locked(true),
            KeyAction::ToggleFullscreen => self.toggle_fullscreen_focused(),
            KeyAction::ToggleMaximize => self.toggle_maximize_focused(),
            KeyAction::CycleWindow { reverse, modifier } => self.cycle_windows(reverse, modifier),
            KeyAction::CancelWindowCycle => self.cancel_window_cycle(),
            KeyAction::Media(action, key) => {
                let output = self.focused_window().and_then(|w| self.window_home(&w))
                    .or_else(|| self.space.output_under(self.pointer.current_location()).next().cloned())
                    .map(|o| o.name());
                self.media_keys.press(key, action, output);
            }
            KeyAction::CycleLayout => self.cycle_layout_mode(),
            KeyAction::ToggleFloating => self.toggle_floating_focused(),
            KeyAction::FocusDir(dir) => self.focus_direction(dir),
            KeyAction::MoveDir(dir) => self.move_direction(dir),
            KeyAction::CycleColumnWidth => self.cycle_column_width(),
            KeyAction::Bar(action) => self.handle_bar_action(action),

            KeyAction::ToggleDecorations => {
                for element in self.space.elements() {
                    #[allow(irrefutable_let_patterns)]
                    if let Some(toplevel) = element.0.toplevel() {
                        let mode_changed = toplevel.with_pending_state(|state| {
                            if let Some(current_mode) = state.decoration_mode {
                                let new_mode =
                                    if current_mode == zxdg_toplevel_decoration_v1::Mode::ClientSide {
                                        zxdg_toplevel_decoration_v1::Mode::ServerSide
                                    } else {
                                        zxdg_toplevel_decoration_v1::Mode::ClientSide
                                    };
                                state.decoration_mode = Some(new_mode);
                                true
                            } else {
                                false
                            }
                        });

                        if mode_changed && toplevel.is_initial_configure_sent() {
                            toplevel.send_pending_configure();
                        }
                    }
                }
            }

            action => {
                // Backend-specific actions (VT switch, output scale/rotate, ...) that a
                // backend chose not to handle. Never panic on a key press.
                tracing::debug!(?action, "key action not supported by this backend");
            }
        }
    }

    fn keyboard_key_to_action<B: InputBackend>(&mut self, evt: B::KeyboardKeyEvent) -> KeyAction {
        let keycode = evt.key_code();
        let state = evt.state();
        if state == KeyState::Released { self.media_keys.release(keycode.raw()); }
        debug!(?keycode, ?state, "key");
        let serial = SCOUNTER.next_serial();
        let time = Event::time_msec(&evt);
        let keyboard = self.seat.get_keyboard().unwrap();

        for layer in self.layer_shell_state.layer_surfaces().rev() {
            let data = with_states(layer.wl_surface(), |states| {
                *states.cached_state.get::<LayerSurfaceCachedState>().current()
            });
            if data.keyboard_interactivity == KeyboardInteractivity::Exclusive
                && (data.layer == WlrLayer::Top || data.layer == WlrLayer::Overlay)
            {
                let surface = self.space.outputs().find_map(|o| {
                    let map = layer_map_for_output(o);
                    let cloned = map.layers().find(|l| l.layer_surface() == &layer).cloned();
                    cloned
                });
                if let Some(surface) = surface {
                    if self.idle.locked && surface.namespace() != crate::idle::LOCK_NAMESPACE {
                        // Locked: only the lock screen may take the keyboard.
                        continue;
                    }
                    keyboard.set_focus(self, Some(surface.into()), serial);
                    keyboard.input::<(), _>(self, keycode, state, serial, time, |data, modifiers, _| {
                        data.window_cycle.release(modifiers.alt, modifiers.logo);
                        if state == KeyState::Released && data.suppressed_keys.contains(&keycode.raw()) {
                            data.suppressed_keys.retain(|key| *key != keycode.raw());
                            FilterResult::Intercept(())
                        } else { FilterResult::Forward }
                    });
                    return KeyAction::None;
                };
            }
        }

        let inhibited = keyboard.current_focus()
            .and_then(|focus| focus.wl_surface().map(|surface| surface.into_owned()))
            .and_then(|surface| self.seat.keyboard_shortcuts_inhibitor_for_surface(&surface))
            .is_some_and(|inhibitor| inhibitor.is_active());

        let action = keyboard
            .input(self, keycode, state, serial, time, |data, modifiers, handle| {
                let keysym = handle.modified_sym();
                data.window_cycle.release(modifiers.alt, modifiers.logo);

                debug!(
                    ?state,
                    mods = ?modifiers,
                    keysym = ::xkbcommon::xkb::keysym_get_name(keysym),
                    "keysym"
                );

                // If the key is pressed and triggered a action
                // we will not forward the key to the client.
                // Additionally add the key to the suppressed keys
                // so that we can decide on a release if the key
                // should be forwarded to the client or not.
                if let KeyState::Pressed = state {
                    if data.window_cycle.active() && keysym == Keysym::Escape && !inhibited {
                        data.suppressed_keys.push(keycode.raw());
                        FilterResult::Intercept(KeyAction::CancelWindowCycle)
                    } else if data.mindbar.open && !(modifiers.logo || (modifiers.ctrl && modifiers.alt)) {
                        // The Mind bar owns the keyboard while it is open.
                        let utf8 = ::xkbcommon::xkb::keysym_to_utf8(keysym);
                        let bar_action = data.mindbar.handle_key(keysym, &utf8, *modifiers);
                        data.suppressed_keys.push(keycode.raw());
                        FilterResult::Intercept(KeyAction::Bar(bar_action))
                    } else if !inhibited {
                        let action = process_keyboard_shortcut(*modifiers, keysym).map(|action| match action {
                            KeyAction::Media(action, _) => KeyAction::Media(action, keycode.raw()),
                            other => other,
                        });

                        if action.is_some() {
                            data.suppressed_keys.push(keycode.raw());
                        }

                        action
                            .map(FilterResult::Intercept)
                            .unwrap_or(FilterResult::Forward)
                    } else {
                        FilterResult::Forward
                    }
                } else {
                    let suppressed = data.suppressed_keys.contains(&keycode.raw());
                    if suppressed {
                        data.suppressed_keys.retain(|key| *key != keycode.raw());
                        FilterResult::Intercept(KeyAction::None)
                    } else {
                        FilterResult::Forward
                    }
                }
            })
            .unwrap_or(KeyAction::None);

        action
    }

    fn on_pointer_button<B: InputBackend>(&mut self, evt: B::PointerButtonEvent) {
        let serial = SCOUNTER.next_serial();
        let button = evt.button_code();
        if evt.state() == smithay::backend::input::ButtonState::Released && self.mindbar.release_button(button) {
            return;
        }
        if evt.state() == smithay::backend::input::ButtonState::Pressed && self.mindbar.open {
            self.mindbar.swallow_button(button);
            let position = self.pointer.current_location();
            let output = self.space.output_under(position).next().cloned();
            if let Some(output) = output {
                if let Some(geometry) = self.space.output_geometry(&output) {
                    let action = if button == 0x110 {
                        self.mindbar.click(&output.name(), position - geometry.loc.to_f64(), geometry.size)
                    } else { crate::mindbar::BarAction::None };
                    self.handle_bar_action(action);
                }
            }
            return;
        }

        let state = wl_pointer::ButtonState::from(evt.state());
        tracing::debug!(
            button,
            ?state,
            grabbed = self.pointer.is_grabbed(),
            focus = ?self.pointer.current_focus().map(|f| format!("{f:?}").chars().take(60).collect::<String>()),
            "pointer button"
        );

        if wl_pointer::ButtonState::Pressed == state {
            // Super + drag takes the window itself; the click never reaches
            // the client, but the pointer still has to see the button so the
            // grab ends when it comes back up.
            if !self.start_super_drag(button, serial) && !self.start_border_resize(button, serial) {
                self.update_keyboard_focus(self.pointer.current_location(), serial);
            }
        };
        let pointer = self.pointer.clone();
        pointer.button(
            self,
            &ButtonEvent {
                button,
                state: state.try_into().unwrap(),
                serial,
                time: evt.time_msec(),
            },
        );
        pointer.frame(self);
    }

    /// Super + drag, the way niri and hyprland do it: the left button moves
    /// the window under the pointer, the right button resizes it. In the
    /// tiling modes the right button moves the divider the window owns rather
    /// than resizing the window on its own. Returns true when the drag took
    /// the click.
    fn start_super_drag(&mut self, button: u32, serial: Serial) -> bool {
        if (button != 0x110 && button != 0x111) || self.idle.locked || self.mindbar.open || self.pointer.is_grabbed() {
            return false;
        }
        let held = self.seat.get_keyboard().map(|k| k.modifier_state()).unwrap_or_default();
        if !held.logo || held.ctrl || held.alt {
            return false;
        }
        let location = self.pointer.current_location();
        // A panel or a popup over the window still takes its own clicks.
        if let Some(output) = self.space.output_under(location).next().cloned() {
            if let Some(geo) = self.space.output_geometry(&output) {
                let map = layer_map_for_output(&output);
                let local = location - geo.loc.to_f64();
                if map.layer_under(WlrLayer::Overlay, local).or_else(|| map.layer_under(WlrLayer::Top, local)).is_some() {
                    return false;
                }
            }
        }
        let window = self.space.element_under(location).map(|(w, _)| w.clone());
        let Some(window) = window else {
            return false;
        };
        if window.pending_fullscreen() {
            return false;
        }
        self.activate_window(&window);
        let start = || PointerGrabStartData { focus: None, button, location };
        let pointer = self.pointer.clone();
        if button == 0x110 {
            let mut initial_window_location = self.space.element_location(&window).unwrap_or_default();
            if window.pending_maximized() {
                if let Some(toplevel) = window.0.toplevel().cloned() {
                    initial_window_location = self.unmaximize_for_drag(&toplevel, &window, location);
                }
            }
            if self.layout.is_tiled(&window) || window.tile().snap.borrow().is_some() {
                self.layout.dragging = Some(window.clone());
            }
            let grab = PointerMoveSurfaceGrab { start_data: start(), window, initial_window_location };
            pointer.set_grab(self, grab, serial, Focus::Clear);
            return true;
        }
        if self.layout.is_tiled(&window) {
            let Some(grab) = self.tile_resize_grab(&window, start()) else {
                return false;
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
            return true;
        }
        let Some(geo) = self.space.element_geometry(&window) else {
            return false;
        };
        // The quadrant the pointer is in picks the corner it drags.
        let rel = location - geo.loc.to_f64();
        let mut edges = if rel.x * 2.0 < geo.size.w as f64 { ResizeEdge::LEFT } else { ResizeEdge::RIGHT };
        edges |= if rel.y * 2.0 < geo.size.h as f64 { ResizeEdge::TOP } else { ResizeEdge::BOTTOM };
        self.start_floating_resize(window, edges, start(), serial)
    }

    /// A plain drag on the ring just outside a window frame. Windows the
    /// compositor decorates draw no resize handles of their own, so without
    /// this a terminal can only be resized with Super + right drag. Floating
    /// windows resize from the edge under the pointer; tiled ones move the
    /// divider the gap sits in, the way i3 and hyprland do.
    fn start_border_resize(&mut self, button: u32, serial: Serial) -> bool {
        if button != 0x110 || self.pointer.is_grabbed() {
            return false;
        }
        let held = self.seat.get_keyboard().map(|k| k.modifier_state()).unwrap_or_default();
        if held.logo || held.ctrl || held.alt || held.shift {
            return false;
        }
        let location = self.pointer.current_location();
        let Some((window, edges)) = self.resize_border_at(location) else {
            return false;
        };
        self.activate_window(&window);
        let start_data = PointerGrabStartData { focus: None, button, location };
        if self.layout.is_tiled(&window) {
            // A column's left edge is the previous column's right edge, and
            // that is the tile whose share of the strip the drag changes.
            let target = if edges.contains(ResizeEdge::LEFT) && self.layout.mode == LayoutMode::Columns {
                self.tile_before(&window).unwrap_or_else(|| window.clone())
            } else {
                window.clone()
            };
            let Some(grab) = self.tile_resize_grab(&target, start_data) else {
                return false;
            };
            self.pointer.clone().set_grab(self, grab, serial, Focus::Clear);
            return true;
        }
        self.start_floating_resize(window, edges, start_data, serial)
    }

    /// Resize a window from `edges`, leaving any tiling or snap behind: a
    /// pinned window comes loose where it is and resizes from there.
    fn start_floating_resize(
        &mut self,
        window: WindowElement,
        edges: ResizeEdge,
        start_data: PointerGrabStartData<Self>,
        serial: Serial,
    ) -> bool {
        window.tile().snap.borrow_mut().take();
        window.tile().snap_saved.borrow_mut().take();
        let Some(geo) = self.space.element_geometry(&window) else {
            return false;
        };
        let Some(surface) = window.wl_surface().map(|s| s.into_owned()) else {
            return false;
        };
        let initial_window_location = geo.loc;
        let initial_window_size = window.0.geometry().size;
        with_states(&surface, |states| {
            if let Some(data) = states.data_map.get::<RefCell<SurfaceData>>() {
                data.borrow_mut().resize_state = ResizeState::Resizing(ResizeData { edges, initial_window_location, initial_window_size });
            }
        });
        let grab = PointerResizeSurfaceGrab {
            start_data,
            window,
            edges,
            initial_window_location,
            initial_window_size,
            last_window_size: initial_window_size,
        };
        self.pointer.clone().set_grab(self, grab, serial, Focus::Clear);
        true
    }

    /// The tile laid out before `window` on its own display.
    fn tile_before(&self, window: &WindowElement) -> Option<WindowElement> {
        let home = self.window_home(window)?;
        let tiles: Vec<&WindowElement> = self
            .layout
            .outputs
            .get(&home.name())?
            .order
            .iter()
            .filter(|w| w.alive() && !w.pending_fullscreen())
            .collect();
        let i = tiles.iter().position(|w| *w == window)?;
        i.checked_sub(1).map(|k| tiles[k].clone())
    }

    /// The window frame the pointer is resting against and the edges a drag
    /// from there would move. Only the empty ring around a frame counts: over
    /// a window, a panel or a popup the answer is `None` and the click goes
    /// where it was aimed.
    pub(crate) fn resize_border_at(
        &self,
        location: Point<f64, Logical>,
    ) -> Option<(WindowElement, ResizeEdge)> {
        // Nothing is resizable behind the lock screen, and the login screen
        // owns its whole display.
        if self.idle.locked || self.mindbar.open || self.config.session.kiosk {
            return None;
        }
        let mut found: Option<(WindowElement, ResizeEdge)> = None;
        for window in self.space.elements().rev() {
            if !window.alive() {
                continue;
            }
            let Some(geo) = self.space.element_geometry(window).map(|g| g.to_f64()) else {
                continue;
            };
            let (x0, y0) = (geo.loc.x, geo.loc.y);
            let (x1, y1) = (x0 + geo.size.w, y0 + geo.size.h);
            // Every pixel a client draws stays the client's, and a window in
            // front of another hides that one's ring along with the rest of it.
            if (x0..=x1).contains(&location.x) && (y0..=y1).contains(&location.y) {
                return None;
            }
            // A window filling its display has no room to grow into.
            if window.pending_fullscreen() || window.pending_maximized() {
                continue;
            }
            if location.x < x0 - RESIZE_BAND
                || location.x > x1 + RESIZE_BAND
                || location.y < y0 - RESIZE_BAND
                || location.y > y1 + RESIZE_BAND
            {
                continue;
            }
            let edges = resize_edges(x0, y0, x1, y1, location);
            // In the gap between two tiles both frames are in reach; the one
            // whose right or bottom edge it is owns the divider there.
            let owns_divider = edges.intersects(ResizeEdge::RIGHT | ResizeEdge::BOTTOM);
            if found.is_none() || owns_divider {
                found = Some((window.clone(), edges));
                if owns_divider {
                    break;
                }
            }
        }
        found.as_ref()?;
        // A panel or a popup over that ring still takes its own clicks.
        if let Some(output) = self.space.output_under(location).next().cloned() {
            if let Some(geo) = self.space.output_geometry(&output) {
                let map = layer_map_for_output(&output);
                let local = location - geo.loc.to_f64();
                if map
                    .layer_under(WlrLayer::Overlay, local)
                    .or_else(|| map.layer_under(WlrLayer::Top, local))
                    .is_some()
                {
                    return None;
                }
            }
        }
        found
    }

    /// Dress the pointer for the resize ring it is over, and undress it again
    /// on the way out. A client that sets its own cursor takes it back.
    pub(crate) fn update_resize_cursor(&mut self, location: Point<f64, Logical>) {
        use smithay::input::pointer::{CursorIcon, CursorImageStatus};
        let icon = if self.pointer.is_grabbed() {
            self.resize_cursor
        } else {
            self.resize_border_at(location).map(|(_, e)| {
                let (l, r) = (e.intersects(ResizeEdge::LEFT), e.intersects(ResizeEdge::RIGHT));
                let (t, b) = (e.intersects(ResizeEdge::TOP), e.intersects(ResizeEdge::BOTTOM));
                match (l, r, t, b) {
                    (true, _, true, _) => CursorIcon::NwResize,
                    (_, true, true, _) => CursorIcon::NeResize,
                    (true, _, _, true) => CursorIcon::SwResize,
                    (_, true, _, true) => CursorIcon::SeResize,
                    (true, ..) => CursorIcon::WResize,
                    (_, true, ..) => CursorIcon::EResize,
                    (_, _, true, _) => CursorIcon::NResize,
                    _ => CursorIcon::SResize,
                }
            })
        };
        if icon == self.resize_cursor {
            return;
        }
        self.resize_cursor = icon;
        self.cursor_status = match icon {
            Some(icon) => CursorImageStatus::Named(icon),
            None => CursorImageStatus::default_named(),
        };
        self.request_repaint();
    }

    /// The divider a Super + right drag on `window` moves. Columns drag the
    /// tile's share of the strip sideways; dwindle drags the split the tile
    /// was carved out of, and the tile at the end of a dwindle owns no split,
    /// so it borrows the one before it and pushes it the other way.
    fn tile_resize_grab(
        &mut self,
        window: &WindowElement,
        start_data: PointerGrabStartData<Self>,
    ) -> Option<TileResizeGrab<BackendData>> {
        let home = self.window_home(window)?;
        let tiles: Vec<WindowElement> = self
            .layout
            .outputs
            .get(&home.name())?
            .order
            .iter()
            .filter(|w| w.alive() && !w.pending_fullscreen())
            .cloned()
            .collect();
        let i = tiles.iter().position(|w| w == window)?;
        let gap = self.layout.gap as f32;
        let frac_of = |w: &WindowElement| {
            let f = w.tile().width.get();
            if f <= 0.0 { 0.5 } else { f }
        };
        match self.layout.mode {
            LayoutMode::Columns => {
                let area = usable_area(&self.space, &home)?;
                let span = (area.size.w - 2 * self.layout.outer_gap) as f32 - gap;
                (span > 1.0).then(|| TileResizeGrab {
                    start_data,
                    target: window.clone(),
                    axis_x: true,
                    invert: false,
                    span,
                    start_frac: frac_of(window),
                    min: 0.15,
                    max: 1.0,
                })
            }
            LayoutMode::Dwindle => {
                let k = if i + 1 == tiles.len() { i.checked_sub(1)? } else { i };
                // What the split divided: this tile and everything after it.
                let rest = tiles[k..]
                    .iter()
                    .filter_map(|w| self.space.element_geometry(w))
                    .reduce(|a, b| a.merge(b))?;
                let axis_x = tiles[k].tile().split_x.get();
                let span = (if axis_x { rest.size.w } else { rest.size.h }) as f32 - gap;
                (span > 1.0).then(|| TileResizeGrab {
                    start_data,
                    target: tiles[k].clone(),
                    axis_x,
                    invert: k != i,
                    span,
                    start_frac: frac_of(&tiles[k]),
                    min: 0.1,
                    max: 0.9,
                })
            }
            LayoutMode::Floating => None,
        }
    }

    fn update_keyboard_focus(&mut self, location: Point<f64, Logical>, serial: Serial) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let touch = self.seat.get_touch();
        let input_method = self.seat.input_method();
        // change the keyboard focus unless the pointer or keyboard is grabbed
        // We test for any matching surface type here but always use the root
        // (in case of a window the toplevel) surface for the focus.
        // So for example if a user clicks on a subsurface or popup the toplevel
        // will receive the keyboard focus. Directly assigning the focus to the
        // matching surface leads to issues with clients dismissing popups and
        // subsurface menus (for example firefox-wayland).
        // see here for a discussion about that issue:
        // https://gitlab.freedesktop.org/wayland/wayland/-/issues/294
        if !self.pointer.is_grabbed()
            && (!keyboard.is_grabbed() || input_method.keyboard_grabbed())
            && !touch.map(|touch| touch.is_grabbed()).unwrap_or(false)
        {
            let output = self.space.output_under(location).next().cloned();
            if let Some(output) = output.as_ref() {
                let output_geo = self.space.output_geometry(output).unwrap();
                if let Some(window) = output
                    .user_data()
                    .get::<FullscreenSurface>()
                    .and_then(|f| f.get())
                {
                    if let Some((_, _)) =
                        window.surface_under(location - output_geo.loc.to_f64(), WindowSurfaceType::ALL)
                    {
                        #[cfg(feature = "xwayland")]
                        if let Some(surface) = window.0.x11_surface() {
                            self.xwm.as_mut().unwrap().raise_window(surface).unwrap();
                        }
                        keyboard.set_focus(self, Some(window.into()), serial);
                        return;
                    }
                }

                let layers = layer_map_for_output(output);
                if let Some(layer) = layers
                    .layer_under(WlrLayer::Overlay, location - output_geo.loc.to_f64())
                    .or_else(|| layers.layer_under(WlrLayer::Top, location - output_geo.loc.to_f64()))
                {
                    if let Some((_, _)) = layer.surface_under(
                        location
                            - output_geo.loc.to_f64()
                            - layers.layer_geometry(layer).unwrap().loc.to_f64(),
                        WindowSurfaceType::ALL,
                    ) {
                        // A panel that wants the keyboard (exclusive / on-demand) gets
                        // it; one that does not (`none`) leaves the focus where it is
                        // rather than handing it to whatever window sits underneath.
                        if layer.can_receive_keyboard_focus() {
                            keyboard.set_focus(self, Some(layer.clone().into()), serial);
                        }
                        return;
                    }
                }
            }

            if let Some((window, _)) = self.space.element_under(location).map(|(w, p)| (w.clone(), p)) {
                self.activate_window(&window);
                return;
            }

            if let Some(output) = output.as_ref() {
                let output_geo = self.space.output_geometry(output).unwrap();
                let layers = layer_map_for_output(output);
                if let Some(layer) = layers
                    .layer_under(WlrLayer::Bottom, location - output_geo.loc.to_f64())
                    .or_else(|| layers.layer_under(WlrLayer::Background, location - output_geo.loc.to_f64()))
                {
                    if layer.can_receive_keyboard_focus() {
                        if let Some((_, _)) = layer.surface_under(
                            location
                                - output_geo.loc.to_f64()
                                - layers.layer_geometry(layer).unwrap().loc.to_f64(),
                            WindowSurfaceType::ALL,
                        ) {
                            keyboard.set_focus(self, Some(layer.clone().into()), serial);
                        }
                    }
                }
            };
        }
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(PointerFocusTarget, Point<f64, Logical>)> {
        let output = self.space.outputs().find(|o| {
            let geometry = self.space.output_geometry(o).unwrap();
            geometry.contains(pos.to_i32_floor())
        })?;
        let output_geo = self.space.output_geometry(output).unwrap();
        let layers = layer_map_for_output(output);

        if self.idle.locked {
            // Locked: the pointer reaches the lock screen and nothing else.
            return layers
                .layers_on(WlrLayer::Overlay)
                .rev()
                .filter(|l| l.namespace() == crate::idle::LOCK_NAMESPACE)
                .find_map(|layer| {
                    let layer_loc = layers.layer_geometry(layer)?.loc;
                    layer
                        .surface_under(
                            pos - output_geo.loc.to_f64() - layer_loc.to_f64(),
                            WindowSurfaceType::ALL,
                        )
                        .map(|(surface, loc)| {
                            (
                                PointerFocusTarget::from(surface),
                                (loc + layer_loc + output_geo.loc).to_f64(),
                            )
                        })
                });
        }

        let mut under = None;
        if let Some((surface, loc)) = output
            .user_data()
            .get::<FullscreenSurface>()
            .and_then(|f| f.get())
            .and_then(|w| w.surface_under(pos - output_geo.loc.to_f64(), WindowSurfaceType::ALL))
        {
            under = Some((surface, loc + output_geo.loc));
        } else if let Some(focus) = layers
            .layer_under(WlrLayer::Overlay, pos - output_geo.loc.to_f64())
            .or_else(|| layers.layer_under(WlrLayer::Top, pos - output_geo.loc.to_f64()))
            .and_then(|layer| {
                let layer_loc = layers.layer_geometry(layer).unwrap().loc;
                layer
                    .surface_under(
                        pos - output_geo.loc.to_f64() - layer_loc.to_f64(),
                        WindowSurfaceType::ALL,
                    )
                    .map(|(surface, loc)| {
                        (
                            PointerFocusTarget::from(surface),
                            loc + layer_loc + output_geo.loc,
                        )
                    })
            })
        {
            under = Some(focus)
        } else if let Some(focus) = self.space.element_under(pos).and_then(|(window, loc)| {
            window
                .surface_under(pos - loc.to_f64(), WindowSurfaceType::ALL)
                .map(|(surface, surf_loc)| (surface, surf_loc + loc))
        }) {
            under = Some(focus);
        } else if let Some(focus) = layers
            .layer_under(WlrLayer::Bottom, pos - output_geo.loc.to_f64())
            .or_else(|| layers.layer_under(WlrLayer::Background, pos - output_geo.loc.to_f64()))
            .and_then(|layer| {
                let layer_loc = layers.layer_geometry(layer).unwrap().loc;
                layer
                    .surface_under(
                        pos - output_geo.loc.to_f64() - layer_loc.to_f64(),
                        WindowSurfaceType::ALL,
                    )
                    .map(|(surface, loc)| {
                        (
                            PointerFocusTarget::from(surface),
                            loc + layer_loc + output_geo.loc,
                        )
                    })
            })
        {
            under = Some(focus)
        };
        under.map(|(s, l)| (s, l.to_f64()))
    }

    fn on_pointer_axis<B: InputBackend>(&mut self, evt: B::PointerAxisEvent) {
        let horizontal_amount = evt
            .amount(input::Axis::Horizontal)
            .unwrap_or_else(|| evt.amount_v120(input::Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.);
        let vertical_amount = evt
            .amount(input::Axis::Vertical)
            .unwrap_or_else(|| evt.amount_v120(input::Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.);
        let horizontal_amount_discrete = evt.amount_v120(input::Axis::Horizontal);
        let vertical_amount_discrete = evt.amount_v120(input::Axis::Vertical);

        // Super + the wheel steps through the windows instead of scrolling
        // whatever is under the pointer. Only the tiling modes have an order
        // to step through; floating passes the scroll on like any other.
        let logo = self
            .seat
            .get_keyboard()
            .map(|k| k.modifier_state().logo)
            .unwrap_or(false);
        if logo && !self.mindbar.open && !self.idle.locked && self.layout.mode.is_tiling() {
            // One wheel notch is 120 in v120 units, or ~15 in the fallback
            // amount; a touchpad sends small continuous values, which add up
            // to a step the same way.
            let notches = match (vertical_amount_discrete, horizontal_amount_discrete) {
                (Some(v), _) if v != 0.0 => v / 120.0,
                (_, Some(h)) if h != 0.0 => h / 120.0,
                _ => (vertical_amount + horizontal_amount) / 15.0,
            };
            if notches != 0.0 {
                if self.super_scroll != 0.0 && self.super_scroll.signum() != notches.signum() {
                    // Turning the other way starts counting again.
                    self.super_scroll = 0.0;
                }
                self.super_scroll += notches;
                while self.super_scroll.abs() >= 1.0 {
                    let forward = self.super_scroll > 0.0;
                    self.super_scroll -= if forward { 1.0 } else { -1.0 };
                    self.focus_step(forward);
                }
            }
            return;
        }
        self.super_scroll = 0.0;

        {
            let mut frame = AxisFrame::new(evt.time_msec()).source(evt.source());
            if horizontal_amount != 0.0 {
                frame = frame.relative_direction(Axis::Horizontal, evt.relative_direction(Axis::Horizontal));
                frame = frame.value(Axis::Horizontal, horizontal_amount);
                if let Some(discrete) = horizontal_amount_discrete {
                    frame = frame.v120(Axis::Horizontal, discrete as i32);
                }
            }
            if vertical_amount != 0.0 {
                frame = frame.relative_direction(Axis::Vertical, evt.relative_direction(Axis::Vertical));
                frame = frame.value(Axis::Vertical, vertical_amount);
                if let Some(discrete) = vertical_amount_discrete {
                    frame = frame.v120(Axis::Vertical, discrete as i32);
                }
            }
            if evt.source() == AxisSource::Finger {
                if evt.amount(Axis::Horizontal) == Some(0.0) {
                    frame = frame.stop(Axis::Horizontal);
                }
                if evt.amount(Axis::Vertical) == Some(0.0) {
                    frame = frame.stop(Axis::Vertical);
                }
            }
            let pointer = self.pointer.clone();
            pointer.axis(self, frame);
            pointer.frame(self);
        }
    }
}

#[cfg(feature = "winit")]
impl<BackendData: Backend> AnvilState<BackendData> {
    pub fn process_input_event_windowed<B: InputBackend>(&mut self, event: InputEvent<B>, output_name: &str) {
        self.request_repaint();
        if is_activity(&event) && self.note_activity() {
            return;
        }
        match event {
            InputEvent::Keyboard { event } => match self.keyboard_key_to_action::<B>(event) {
                KeyAction::ScaleUp => {
                    let output = self
                        .space
                        .outputs()
                        .find(|o| o.name() == output_name)
                        .unwrap()
                        .clone();

                    let current_scale = output.current_scale().fractional_scale();
                    let new_scale = current_scale + 0.25;
                    output.change_current_state(None, None, Some(Scale::Fractional(new_scale)), None);

                    crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
                    self.backend_data.reset_buffers(&output);
                }

                KeyAction::ScaleDown => {
                    let output = self
                        .space
                        .outputs()
                        .find(|o| o.name() == output_name)
                        .unwrap()
                        .clone();

                    let current_scale = output.current_scale().fractional_scale();
                    let new_scale = f64::max(1.0, current_scale - 0.25);
                    output.change_current_state(None, None, Some(Scale::Fractional(new_scale)), None);

                    crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
                    self.backend_data.reset_buffers(&output);
                }

                KeyAction::RotateOutput => {
                    let output = self
                        .space
                        .outputs()
                        .find(|o| o.name() == output_name)
                        .unwrap()
                        .clone();

                    let current_transform = output.current_transform();
                    let new_transform = match current_transform {
                        Transform::Normal => Transform::_90,
                        Transform::_90 => Transform::_180,
                        Transform::_180 => Transform::_270,
                        Transform::_270 => Transform::Flipped,
                        Transform::Flipped => Transform::Flipped90,
                        Transform::Flipped90 => Transform::Flipped180,
                        Transform::Flipped180 => Transform::Flipped270,
                        Transform::Flipped270 => Transform::Normal,
                    };
                    tracing::info!(?current_transform, ?new_transform, output = ?output.name(), "changing output transform");
                    output.change_current_state(None, Some(new_transform), None, None);
                    crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
                    self.backend_data.reset_buffers(&output);
                }

                action => self.process_common_key_action(action),
            },

            InputEvent::PointerMotionAbsolute { event } => {
                let output = self
                    .space
                    .outputs()
                    .find(|o| o.name() == output_name)
                    .unwrap()
                    .clone();
                self.on_pointer_move_absolute_windowed::<B>(event, &output)
            }
            InputEvent::PointerButton { event } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event } => self.on_pointer_axis::<B>(event),
            _ => (), // other events are not handled in anvil (yet)
        }
    }

    fn on_pointer_move_absolute_windowed<B: InputBackend>(
        &mut self,
        evt: B::PointerMotionAbsoluteEvent,
        output: &Output,
    ) {
        let Some(output_geo) = self.space.output_geometry(output) else { return; };
        if let Some(pos) = crate::pointer::absolute_position(evt.x_transformed(1), evt.y_transformed(1), &[output_geo]) {
            self.handle_absolute_pointer(evt.device().id(), pos, evt.time());
        }
    }

    pub fn release_all_keys(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        for keycode in keyboard.pressed_keys() {
            keyboard.input(
                self,
                keycode,
                KeyState::Released,
                SCOUNTER.next_serial(),
                0,
                |_, _, _| FilterResult::Forward::<bool>,
            );
        }
    }
}

#[cfg(feature = "udev")]
impl AnvilState<UdevData> {
    pub fn process_input_event<B: InputBackend>(&mut self, dh: &DisplayHandle, event: InputEvent<B>) {
        // Whatever the event does (move the pointer, open the bar, switch
        // windows) shows on screen: ask for a frame once, up front.
        self.request_repaint();
        // The first key or click after the screen went to sleep only wakes it:
        // it must not reach the window (or the password field) underneath.
        if is_activity(&event) && self.note_activity() {
            return;
        }
        match event {
            InputEvent::Keyboard { event, .. } => match self.keyboard_key_to_action::<B>(event) {
                #[cfg(feature = "udev")]
                KeyAction::VtSwitch(vt) => {
                    info!(to = vt, "Trying to switch vt");
                    if let Err(err) = self.backend_data.session.change_vt(vt) {
                        error!(vt, "Error switching vt: {}", err);
                    }
                }
                KeyAction::Screen(num) => {
                    let geometry = self
                        .space
                        .outputs()
                        .nth(num)
                        .map(|o| self.space.output_geometry(o).unwrap());

                    if let Some(geometry) = geometry {
                        let x = geometry.loc.x as f64 + geometry.size.w as f64 / 2.0;
                        let y = geometry.size.h as f64 / 2.0;
                        let location = (x, y).into();
                        let pointer = self.pointer.clone();
                        let under = self.surface_under(location);
                        pointer.motion(
                            self,
                            under,
                            &MotionEvent {
                                location,
                                serial: SCOUNTER.next_serial(),
                                time: self.clock.now().as_millis(),
                            },
                        );
                        pointer.frame(self);
                    }
                }
                KeyAction::ScaleUp => {
                    let pos = self.pointer.current_location().to_i32_round();
                    let output = self
                        .space
                        .outputs()
                        .find(|o| self.space.output_geometry(o).unwrap().contains(pos))
                        .cloned();

                    if let Some(output) = output {
                        let (output_location, scale) = (
                            self.space.output_geometry(&output).unwrap().loc,
                            output.current_scale().fractional_scale(),
                        );
                        let new_scale = scale + 0.25;
                        output.change_current_state(None, None, Some(Scale::Fractional(new_scale)), None);

                        let rescale = scale / new_scale;
                        let output_location = output_location.to_f64();
                        let mut pointer_output_location = self.pointer.current_location() - output_location;
                        pointer_output_location.x *= rescale;
                        pointer_output_location.y *= rescale;
                        let pointer_location = output_location + pointer_output_location;

                        crate::shell::fixup_positions(&mut self.space, pointer_location, &self.prefs.pinned_positions());
                        let pointer = self.pointer.clone();
                        let under = self.surface_under(pointer_location);
                        pointer.motion(
                            self,
                            under,
                            &MotionEvent {
                                location: pointer_location,
                                serial: SCOUNTER.next_serial(),
                                time: self.clock.now().as_millis(),
                            },
                        );
                        pointer.frame(self);
                        self.backend_data.reset_buffers(&output);
                    }
                }
                KeyAction::ScaleDown => {
                    let pos = self.pointer.current_location().to_i32_round();
                    let output = self
                        .space
                        .outputs()
                        .find(|o| self.space.output_geometry(o).unwrap().contains(pos))
                        .cloned();

                    if let Some(output) = output {
                        let (output_location, scale) = (
                            self.space.output_geometry(&output).unwrap().loc,
                            output.current_scale().fractional_scale(),
                        );
                        let new_scale = f64::max(1.0, scale - 0.25);
                        output.change_current_state(None, None, Some(Scale::Fractional(new_scale)), None);

                        let rescale = scale / new_scale;
                        let output_location = output_location.to_f64();
                        let mut pointer_output_location = self.pointer.current_location() - output_location;
                        pointer_output_location.x *= rescale;
                        pointer_output_location.y *= rescale;
                        let pointer_location = output_location + pointer_output_location;

                        crate::shell::fixup_positions(&mut self.space, pointer_location, &self.prefs.pinned_positions());
                        let pointer = self.pointer.clone();
                        let under = self.surface_under(pointer_location);
                        pointer.motion(
                            self,
                            under,
                            &MotionEvent {
                                location: pointer_location,
                                serial: SCOUNTER.next_serial(),
                                time: self.clock.now().as_millis(),
                            },
                        );
                        pointer.frame(self);
                        self.backend_data.reset_buffers(&output);
                    }
                }
                KeyAction::RotateOutput => {
                    let pos = self.pointer.current_location().to_i32_round();
                    let output = self
                        .space
                        .outputs()
                        .find(|o| self.space.output_geometry(o).unwrap().contains(pos))
                        .cloned();

                    if let Some(output) = output {
                        let current_transform = output.current_transform();
                        let new_transform = match current_transform {
                            Transform::Normal => Transform::_90,
                            Transform::_90 => Transform::_180,
                            Transform::_180 => Transform::_270,
                            Transform::_270 => Transform::Flipped,
                            Transform::Flipped => Transform::Flipped90,
                            Transform::Flipped90 => Transform::Flipped180,
                            Transform::Flipped180 => Transform::Flipped270,
                            Transform::Flipped270 => Transform::Normal,
                        };
                        output.change_current_state(None, Some(new_transform), None, None);
                        crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
                        self.backend_data.reset_buffers(&output);
                    }
                }
                KeyAction::ToggleTint => {
                    let mut debug_flags = self.backend_data.debug_flags();
                    debug_flags.toggle(DebugFlags::TINT);
                    self.backend_data.set_debug_flags(debug_flags);
                }

                // Everything else (quit, terminal, Mind bar, window actions, ...) is
                // backend independent.
                action => self.process_common_key_action(action),
            },
            InputEvent::PointerMotion { event, .. } => self.on_pointer_move::<B>(dh, event),
            InputEvent::PointerMotionAbsolute { event, .. } => self.on_pointer_move_absolute::<B>(dh, event),
            InputEvent::PointerButton { event, .. } => self.on_pointer_button::<B>(event),
            InputEvent::PointerAxis { event, .. } => self.on_pointer_axis::<B>(event),
            InputEvent::TabletToolAxis { event, .. } => self.on_tablet_tool_axis::<B>(event),
            InputEvent::TabletToolProximity { event, .. } => self.on_tablet_tool_proximity::<B>(dh, event),
            InputEvent::TabletToolTip { event, .. } => self.on_tablet_tool_tip::<B>(event),
            InputEvent::TabletToolButton { event, .. } => self.on_tablet_button::<B>(event),
            InputEvent::GestureSwipeBegin { event, .. } => self.on_gesture_swipe_begin::<B>(event),
            InputEvent::GestureSwipeUpdate { event, .. } => self.on_gesture_swipe_update::<B>(event),
            InputEvent::GestureSwipeEnd { event, .. } => self.on_gesture_swipe_end::<B>(event),
            InputEvent::GesturePinchBegin { event, .. } => self.on_gesture_pinch_begin::<B>(event),
            InputEvent::GesturePinchUpdate { event, .. } => self.on_gesture_pinch_update::<B>(event),
            InputEvent::GesturePinchEnd { event, .. } => self.on_gesture_pinch_end::<B>(event),
            InputEvent::GestureHoldBegin { event, .. } => self.on_gesture_hold_begin::<B>(event),
            InputEvent::GestureHoldEnd { event, .. } => self.on_gesture_hold_end::<B>(event),

            InputEvent::TouchDown { event } => self.on_touch_down::<B>(event),
            InputEvent::TouchUp { event } => self.on_touch_up::<B>(event),
            InputEvent::TouchMotion { event } => self.on_touch_motion::<B>(event),
            InputEvent::TouchFrame { event } => self.on_touch_frame::<B>(event),
            InputEvent::TouchCancel { event } => self.on_touch_cancel::<B>(event),

            InputEvent::DeviceAdded { device } => {
                if device.has_capability(DeviceCapability::TabletTool) {
                    self.seat
                        .tablet_seat()
                        .add_tablet::<Self>(dh, &TabletDescriptor::from(&device));
                }
                if device.has_capability(DeviceCapability::Touch) && self.seat.get_touch().is_none() {
                    self.seat.add_touch();
                }
            }
            InputEvent::DeviceRemoved { device } => {
                self.absolute_pointer_positions.remove(&device.id());
                if device.has_capability(DeviceCapability::TabletTool) {
                    let tablet_seat = self.seat.tablet_seat();

                    tablet_seat.remove_tablet(&TabletDescriptor::from(&device));

                    // If there are no tablets in seat we can remove all tools
                    if tablet_seat.count_tablets() == 0 {
                        tablet_seat.clear_tools();
                    }
                }
            }
            _ => {
                // other events are not handled in anvil (yet)
            }
        }
    }

    fn on_pointer_move<B: InputBackend>(&mut self, _dh: &DisplayHandle, evt: B::PointerMotionEvent) {
        self.handle_pointer_motion(self.pointer.current_location() + evt.delta(), evt.delta(), evt.delta_unaccel(), evt.time());
    }

    fn on_pointer_move_absolute<B: InputBackend>(&mut self, _dh: &DisplayHandle, evt: B::PointerMotionAbsoluteEvent) {
        let outputs: Vec<_> = self.space.outputs().filter_map(|o| self.space.output_geometry(o)).collect();
        if let Some(pos) = crate::pointer::absolute_position(evt.x_transformed(1), evt.y_transformed(1), &outputs) {
            self.handle_absolute_pointer(evt.device().id(), pos, evt.time());
        }
    }

    fn on_tablet_tool_axis<B: InputBackend>(&mut self, evt: B::TabletToolAxisEvent) {
        let tablet_seat = self.seat.tablet_seat();

        if let Some(pointer_location) = self.touch_location_transformed(&evt) {
            let pointer = self.pointer.clone();
            let under = self.surface_under(pointer_location);
            let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&evt.device()));
            let tool = tablet_seat.get_tool(&evt.tool());

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: self.clock.now().as_millis(),
                },
            );

            if let (Some(tablet), Some(tool)) = (tablet, tool) {
                if evt.pressure_has_changed() {
                    tool.pressure(evt.pressure());
                }
                if evt.distance_has_changed() {
                    tool.distance(evt.distance());
                }
                if evt.tilt_has_changed() {
                    tool.tilt(evt.tilt());
                }
                if evt.slider_has_changed() {
                    tool.slider_position(evt.slider_position());
                }
                if evt.rotation_has_changed() {
                    tool.rotation(evt.rotation());
                }
                if evt.wheel_has_changed() {
                    tool.wheel(evt.wheel_delta(), evt.wheel_delta_discrete());
                }

                tool.motion(
                    pointer_location,
                    under.and_then(|(f, loc)| f.wl_surface().map(|s| (s.into_owned(), loc))),
                    &tablet,
                    SCOUNTER.next_serial(),
                    evt.time_msec(),
                );
            }

            pointer.frame(self);
        }
    }

    fn on_tablet_tool_proximity<B: InputBackend>(
        &mut self,
        dh: &DisplayHandle,
        evt: B::TabletToolProximityEvent,
    ) {
        let tablet_seat = self.seat.tablet_seat();

        if let Some(pointer_location) = self.touch_location_transformed(&evt) {
            let tool = evt.tool();
            tablet_seat.add_tool::<Self>(self, dh, &tool);

            let pointer = self.pointer.clone();
            let under = self.surface_under(pointer_location);
            let tablet = tablet_seat.get_tablet(&TabletDescriptor::from(&evt.device()));
            let tool = tablet_seat.get_tool(&tool);

            pointer.motion(
                self,
                under.clone(),
                &MotionEvent {
                    location: pointer_location,
                    serial: SCOUNTER.next_serial(),
                    time: evt.time_msec(),
                },
            );
            pointer.frame(self);

            if let (Some(under), Some(tablet), Some(tool)) = (
                under.and_then(|(f, loc)| f.wl_surface().map(|s| (s.into_owned(), loc))),
                tablet,
                tool,
            ) {
                match evt.state() {
                    ProximityState::In => tool.proximity_in(
                        pointer_location,
                        under,
                        &tablet,
                        SCOUNTER.next_serial(),
                        evt.time_msec(),
                    ),
                    ProximityState::Out => tool.proximity_out(evt.time_msec()),
                }
            }
        }
    }

    fn on_tablet_tool_tip<B: InputBackend>(&mut self, evt: B::TabletToolTipEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            match evt.tip_state() {
                TabletToolTipState::Down => {
                    let serial = SCOUNTER.next_serial();
                    tool.tip_down(serial, evt.time_msec());

                    // change the keyboard focus
                    self.update_keyboard_focus(self.pointer.current_location(), serial);
                }
                TabletToolTipState::Up => {
                    tool.tip_up(evt.time_msec());
                }
            }
        }
    }

    fn on_tablet_button<B: InputBackend>(&mut self, evt: B::TabletToolButtonEvent) {
        let tool = self.seat.tablet_seat().get_tool(&evt.tool());

        if let Some(tool) = tool {
            tool.button(
                evt.button(),
                evt.button_state(),
                SCOUNTER.next_serial(),
                evt.time_msec(),
            );
        }
    }

    fn on_gesture_swipe_begin<B: InputBackend>(&mut self, evt: B::GestureSwipeBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_swipe_begin(
            self,
            &GestureSwipeBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    fn on_gesture_swipe_update<B: InputBackend>(&mut self, evt: B::GestureSwipeUpdateEvent) {
        let pointer = self.pointer.clone();
        pointer.gesture_swipe_update(
            self,
            &GestureSwipeUpdateEvent {
                time: evt.time_msec(),
                delta: evt.delta(),
            },
        );
    }

    fn on_gesture_swipe_end<B: InputBackend>(&mut self, evt: B::GestureSwipeEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_swipe_end(
            self,
            &GestureSwipeEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    fn on_gesture_pinch_begin<B: InputBackend>(&mut self, evt: B::GesturePinchBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_pinch_begin(
            self,
            &GesturePinchBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    fn on_gesture_pinch_update<B: InputBackend>(&mut self, evt: B::GesturePinchUpdateEvent) {
        let pointer = self.pointer.clone();
        pointer.gesture_pinch_update(
            self,
            &GesturePinchUpdateEvent {
                time: evt.time_msec(),
                delta: evt.delta(),
                scale: evt.scale(),
                rotation: evt.rotation(),
            },
        );
    }

    fn on_gesture_pinch_end<B: InputBackend>(&mut self, evt: B::GesturePinchEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_pinch_end(
            self,
            &GesturePinchEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    fn on_gesture_hold_begin<B: InputBackend>(&mut self, evt: B::GestureHoldBeginEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_hold_begin(
            self,
            &GestureHoldBeginEvent {
                serial,
                time: evt.time_msec(),
                fingers: evt.fingers(),
            },
        );
    }

    fn on_gesture_hold_end<B: InputBackend>(&mut self, evt: B::GestureHoldEndEvent) {
        let serial = SCOUNTER.next_serial();
        let pointer = self.pointer.clone();
        pointer.gesture_hold_end(
            self,
            &GestureHoldEndEvent {
                serial,
                time: evt.time_msec(),
                cancelled: evt.cancelled(),
            },
        );
    }

    fn touch_location_transformed<B: InputBackend, E: AbsolutePositionEvent<B>>(
        &self,
        evt: &E,
    ) -> Option<Point<f64, Logical>> {
        let output = self
            .space
            .outputs()
            .find(|output| output.name().starts_with("eDP"))
            .or_else(|| self.space.outputs().next());

        let output = output?;
        let output_geometry = self.space.output_geometry(output)?;

        let transform = output.current_transform();
        let size = transform.invert().transform_size(output_geometry.size);
        Some(
            transform.transform_point_in(evt.position_transformed(size), &size.to_f64())
                + output_geometry.loc.to_f64(),
        )
    }

    fn on_touch_down<B: InputBackend>(&mut self, evt: B::TouchDownEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };

        let Some(touch_location) = self.touch_location_transformed(&evt) else {
            return;
        };

        let serial = SCOUNTER.next_serial();
        self.update_keyboard_focus(touch_location, serial);

        let under = self.surface_under(touch_location);
        handle.down(
            self,
            under,
            &DownEvent {
                slot: evt.slot(),
                location: touch_location,
                serial,
                time: evt.time_msec(),
            },
        );
    }
    fn on_touch_up<B: InputBackend>(&mut self, evt: B::TouchUpEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        let serial = SCOUNTER.next_serial();
        handle.up(
            self,
            &UpEvent {
                slot: evt.slot(),
                serial,
                time: evt.time_msec(),
            },
        )
    }
    fn on_touch_motion<B: InputBackend>(&mut self, evt: B::TouchMotionEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        let Some(touch_location) = self.touch_location_transformed(&evt) else {
            return;
        };

        let under = self.surface_under(touch_location);
        handle.motion(
            self,
            under,
            &smithay::input::touch::MotionEvent {
                slot: evt.slot(),
                location: touch_location,
                time: evt.time_msec(),
            },
        );
    }
    fn on_touch_frame<B: InputBackend>(&mut self, _evt: B::TouchFrameEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        handle.frame(self);
    }
    fn on_touch_cancel<B: InputBackend>(&mut self, _evt: B::TouchCancelEvent) {
        let Some(handle) = self.seat.get_touch() else {
            return;
        };
        handle.cancel(self);
    }


}

/// Possible results of a keyboard action
#[allow(dead_code)] // some of these are only read if udev is enabled
#[derive(Debug)]
enum KeyAction {
    /// Quit the compositor
    Quit,
    /// Trigger a vt-switch
    VtSwitch(i32),
    /// run a command
    Run(String),
    /// Switch the current screen
    Screen(usize),
    ScaleUp,
    ScaleDown,
    TogglePreview,
    RotateOutput,
    ToggleTint,
    ToggleDecorations,
    /// Open/close the Mind bar (launcher + conversation)
    ToggleMindBar,
    /// Super+W: the shell's overview (or the built-in window preview)
    Overview,
    /// Open the configured terminal
    Terminal,
    Screenshot { screen: bool },
    CloseWindow,
    /// Lock the session now (Super+L).
    LockScreen,
    ToggleFullscreen,
    ToggleMaximize,
    CycleWindow { reverse: bool, modifier: crate::window_cycle::CycleModifier },
    CancelWindowCycle,
    Media(crate::media::Action, u32),
    /// Super+T: floating -> tiles -> columns
    CycleLayout,
    /// Super+Shift+F: take the focused window out of the tiling, or put it back
    ToggleFloating,
    /// Super+arrows: focus the nearest window in that direction
    FocusDir(crate::layout::Direction),
    /// Super+Shift+arrows: swap the focused tile with its neighbour
    MoveDir(crate::layout::Direction),
    /// Super+R: the next width preset for the focused column
    CycleColumnWidth,
    /// A key handled by the Mind bar produced this
    Bar(crate::mindbar::BarAction),
    /// Do nothing more
    None,
}

pub fn next_serial() -> Serial {
    SCOUNTER.next_serial()
}

/// MindOS keybindings. Super is the compositor modifier; everything else
/// goes to the focused application (games see unmodified keys).
fn process_keyboard_shortcut(modifiers: ModifiersState, keysym: Keysym) -> Option<KeyAction> {
    if !modifiers.ctrl && !modifiers.alt && !modifiers.logo && !modifiers.shift {
        use crate::media::Action;
        let action = match keysym {
            Keysym::XF86_AudioRaiseVolume => Some(Action::VolumeUp),
            Keysym::XF86_AudioLowerVolume => Some(Action::VolumeDown),
            Keysym::XF86_AudioMute => Some(Action::Mute),
            Keysym::XF86_AudioMicMute => Some(Action::MicMute),
            Keysym::XF86_MonBrightnessUp => Some(Action::BrightnessUp),
            Keysym::XF86_MonBrightnessDown => Some(Action::BrightnessDown),
            Keysym::XF86_AudioPlay => Some(Action::PlayPause),
            Keysym::XF86_AudioPause => Some(Action::Pause),
            Keysym::XF86_AudioStop => Some(Action::Stop),
            Keysym::XF86_AudioNext => Some(Action::Next),
            Keysym::XF86_AudioPrev => Some(Action::Previous),
            _ => None,
        };
        if let Some(action) = action { return Some(KeyAction::Media(action, 0)); }
    }
    // Caps Lock changes letters, not the meaning of desktop shortcuts.
    let keysym = if (xkb::KEY_A..=xkb::KEY_Z).contains(&keysym.raw()) && !modifiers.shift {
        Keysym::new(keysym.raw() + xkb::KEY_a - xkb::KEY_A)
    } else if (xkb::KEY_a..=xkb::KEY_z).contains(&keysym.raw()) && modifiers.shift {
        Keysym::new(keysym.raw() + xkb::KEY_A - xkb::KEY_a)
    } else { keysym };
    if keysym == Keysym::Print && !modifiers.ctrl && !modifiers.alt && !modifiers.logo {
        Some(KeyAction::Screenshot { screen: modifiers.shift })
    } else if modifiers.logo && modifiers.shift && !modifiers.ctrl && !modifiers.alt
        && matches!(keysym, Keysym::S | Keysym::s) {
        Some(KeyAction::Screenshot { screen: false })
    } else if (modifiers.ctrl && modifiers.alt && keysym == Keysym::BackSpace)
        || (modifiers.logo && modifiers.shift && (keysym == Keysym::E || keysym == Keysym::e))
    {
        // ctrl+alt+backspace / super+shift+e = quit
        Some(KeyAction::Quit)
    } else if (xkb::KEY_XF86Switch_VT_1..=xkb::KEY_XF86Switch_VT_12).contains(&keysym.raw()) {
        // VTSwitch
        Some(KeyAction::VtSwitch(
            (keysym.raw() - xkb::KEY_XF86Switch_VT_1 + 1) as i32,
        ))
    } else if modifiers.logo && (keysym == Keysym::Return || keysym == Keysym::KP_Enter) {
        Some(KeyAction::Terminal)
    } else if modifiers.logo && keysym == Keysym::space {
        Some(KeyAction::ToggleMindBar)
    } else if (modifiers.logo && !modifiers.shift && keysym == Keysym::q)
        || (modifiers.alt && !modifiers.ctrl && !modifiers.logo && !modifiers.shift && keysym == Keysym::F4) {
        Some(KeyAction::CloseWindow)
    } else if modifiers.logo && !modifiers.shift && keysym == Keysym::f {
        Some(KeyAction::ToggleFullscreen)
    } else if modifiers.logo && !modifiers.shift && keysym == Keysym::l {
        Some(KeyAction::LockScreen)
    } else if modifiers.logo && !modifiers.shift && keysym == Keysym::m {
        Some(KeyAction::ToggleMaximize)
    } else if modifiers.logo && !modifiers.shift && keysym == Keysym::w {
        Some(KeyAction::Overview)
    } else if (modifiers.logo ^ modifiers.alt) && !modifiers.ctrl
        && (keysym == Keysym::Tab || keysym == Keysym::ISO_Left_Tab) {
        Some(KeyAction::CycleWindow {
            reverse: modifiers.shift || keysym == Keysym::ISO_Left_Tab,
            modifier: if modifiers.logo { crate::window_cycle::CycleModifier::Super }
                      else { crate::window_cycle::CycleModifier::Alt },
        })
    } else if modifiers.logo && !modifiers.shift && keysym == Keysym::t {
        Some(KeyAction::CycleLayout)
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::F {
        Some(KeyAction::ToggleFloating)
    } else if modifiers.logo && !modifiers.shift && keysym == Keysym::r {
        Some(KeyAction::CycleColumnWidth)
    } else if modifiers.logo && matches!(keysym, Keysym::Left | Keysym::Right | Keysym::Up | Keysym::Down) {
        use crate::layout::Direction;
        let dir = match keysym {
            Keysym::Left => Direction::Left,
            Keysym::Right => Direction::Right,
            Keysym::Up => Direction::Up,
            _ => Direction::Down,
        };
        Some(if modifiers.shift {
            KeyAction::MoveDir(dir)
        } else {
            KeyAction::FocusDir(dir)
        })
    } else if modifiers.logo && (xkb::KEY_1..=xkb::KEY_9).contains(&keysym.raw()) {
        Some(KeyAction::Screen((keysym.raw() - xkb::KEY_1) as usize))
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::M {
        Some(KeyAction::ScaleDown)
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::P {
        Some(KeyAction::ScaleUp)
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::W {
        Some(KeyAction::TogglePreview)
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::R {
        Some(KeyAction::RotateOutput)
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::T {
        Some(KeyAction::ToggleTint)
    } else if modifiers.logo && modifiers.shift && keysym == Keysym::D {
        Some(KeyAction::ToggleDecorations)
    } else {
        None
    }
}
