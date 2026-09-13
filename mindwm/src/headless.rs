// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! A compositor with no displays and no GPU: all of mindwm -- every protocol,
//! the shell, the layout, the IPC socket and the drawing -- on a pixman
//! renderer that draws into memory, turned one step at a time by whoever
//! holds it.
//!
//! This is what the integration tests run. The backend is the thin part of a
//! compositor. The freeze that took both displays on 2026-09-12 lived in the
//! thick part every backend shares, and until this existed the only way to
//! exercise that part was to log into a desktop and find out. Here it runs
//! under `cargo test` with the loop watchdog armed, so a lock taken twice or
//! a callback that never returns fails a test instead of a session.
//!
//! What is left out is exactly what needs hardware: DRM, page flips, vblank
//! timing, dmabufs and Xwayland. Everything else is the code the DRM backend
//! runs, called in the same order -- `render_output` follows
//! `udev::render_surface` step for step, and the outputs come and go the way
//! connectors do.

use std::{sync::Mutex, time::Duration};

use serde_json::{json, Value};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker,
            element::{memory::MemoryRenderBuffer, AsRenderElements},
            pixman::PixmanRenderer,
            Bind, ExportMem, ImportMemWl, Offscreen,
        },
    },
    desktop::space::SurfaceTree,
    input::{
        keyboard::LedState,
        pointer::{CursorImageAttributes, CursorImageStatus},
    },
    output::{Mode, Output, PhysicalProperties, Subpixel},
    reexports::{
        calloop::EventLoop,
        pixman::Image,
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::{backend::GlobalId, protocol::wl_surface::WlSurface, Display, DisplayHandle},
    },
    utils::{IsAlive, Rectangle, Scale, Transform},
    wayland::{compositor, presentation::Refresh},
};

use crate::{
    config::Config,
    drawing::PointerElement,
    ipc::{ModeInfo, OutputInfo},
    recover::LockAnyway,
    render::{output_elements, CustomRenderElements},
    state::{take_presentation_feedback, update_primary_scanout_output, AnvilState, Backend},
};

/// What a frame failed on.
pub type RenderError = Box<dyn std::error::Error>;

/// The backend: a renderer, and an image for every output to draw into.
pub struct HeadlessData {
    renderer: PixmanRenderer,
    screens: Vec<Screen>,
    /// Outputs switched off in the Displays settings, which can be switched
    /// on again. A connector that is still plugged in, in DRM terms.
    switched_off: Vec<SwitchedOff>,
    pointer_element: PointerElement,
    /// The arrow, where the DRM backend would load the cursor theme: a
    /// plain square, so nothing on the machine running the tests changes
    /// what they draw.
    pointer_image: MemoryRenderBuffer,
    /// How many times anything has asked for a frame.
    pub repaint_requests: u64,
    /// Whether the displays are off, as `set_blanked` left them.
    pub blanked: bool,
}

/// One output and the memory it is drawn into.
struct Screen {
    dh: DisplayHandle,
    output: Output,
    global: Option<GlobalId>,
    image: Image<'static, 'static>,
    damage: OutputDamageTracker,
    /// How many frames old what the image holds is: 0 before the first
    /// frame and after anything that makes the old contents meaningless.
    /// There is no swapchain, so a drawn image is always one frame old.
    age: usize,
    /// Something asked for a frame since the last one.
    dirty: bool,
    /// Frames that actually drew something.
    frames: u64,
}

impl Drop for Screen {
    fn drop(&mut self) {
        // As `udev::SurfaceData` does: the output's global goes with it.
        if let Some(global) = self.global.take() {
            self.dh.remove_global::<AnvilState<HeadlessData>>(global);
        }
    }
}

struct SwitchedOff {
    name: String,
    modes: Vec<Mode>,
}

impl HeadlessData {
    pub fn new() -> Result<HeadlessData, RenderError> {
        let pixels: Vec<u8> = [0xff_u8; 4].repeat(8 * 8);
        Ok(HeadlessData {
            renderer: PixmanRenderer::new()?,
            screens: Vec::new(),
            switched_off: Vec::new(),
            pointer_element: PointerElement::default(),
            pointer_image: MemoryRenderBuffer::from_slice(&pixels, Fourcc::Argb8888, (8, 8), 1, Transform::Normal, None),
            repaint_requests: 0,
            blanked: false,
        })
    }

    fn screen_mut(&mut self, output: &Output) -> Option<&mut Screen> {
        self.screens.iter_mut().find(|screen| &screen.output == output)
    }

    /// Frames drawn on the output called `name`, or `None` if it is not on.
    pub fn frames(&self, name: &str) -> Option<u64> {
        self.screens.iter().find(|screen| screen.output.name() == name).map(|screen| screen.frames)
    }

    /// Whether the output called `name` has asked for a frame it has not had.
    pub fn is_dirty(&self, name: &str) -> bool {
        self.screens.iter().any(|screen| screen.output.name() == name && screen.dirty)
    }

    fn mark_dirty(&mut self, output: Option<&Output>) {
        self.repaint_requests += 1;
        for screen in &mut self.screens {
            if output.is_none_or(|output| &screen.output == output) {
                screen.dirty = true;
            }
        }
    }
}

impl Backend for HeadlessData {
    fn software_rendering(&self) -> bool {
        true
    }

    fn seat_name(&self) -> String {
        "headless".into()
    }

    fn reset_buffers(&mut self, output: &Output) {
        if let Some(screen) = self.screen_mut(output) {
            screen.age = 0;
            screen.dirty = true;
        }
    }

    fn early_import(&mut self, _surface: &WlSurface) {}

    fn update_led_state(&mut self, _led_state: LedState) {}

    fn set_output_mode(&mut self, output: &Output, _mode: Mode) -> Result<(), String> {
        // `set_output_config` changes the output's own mode once this says
        // yes; the image is made again at the new size by the next frame.
        let screen = self.screen_mut(output).ok_or("the output is off")?;
        screen.dirty = true;
        Ok(())
    }

    fn disabled_outputs(&self) -> Vec<OutputInfo> {
        self.switched_off
            .iter()
            .map(|off| OutputInfo {
                software_rendering: true,
                name: off.name.clone(),
                make: "MindOS".into(),
                model: "Headless".into(),
                x: 0,
                y: 0,
                width: 0,
                height: 0,
                scale: 1.0,
                refresh: 0.0,
                transform: "normal".into(),
                modes: off
                    .modes
                    .iter()
                    .enumerate()
                    .map(|(i, mode)| ModeInfo {
                        width: mode.size.w,
                        height: mode.size.h,
                        refresh: mode.refresh,
                        preferred: i == 0,
                        current: false,
                    })
                    .collect(),
                enabled: false,
                vrr: false,
                vrr_supported: false,
                primary: false,
                mm_width: 0,
                mm_height: 0,
            })
            .collect()
    }

    fn graphics_stats(state: &AnvilState<Self>) -> Value {
        let outputs: Vec<Value> = state
            .backend_data
            .screens
            .iter()
            .map(|screen| {
                json!({
                    "output": screen.output.name(),
                    "refresh_mhz": screen.output.current_mode().map(|mode| mode.refresh).unwrap_or(0),
                    "frames": screen.frames,
                    "dirty": screen.dirty,
                })
            })
            .collect();
        json!({
            "software_rendering": true,
            "blanked": state.backend_data.blanked,
            "outputs": outputs,
        })
    }

    fn set_output_enabled(state: &mut AnvilState<Self>, name: &str, enabled: bool) -> Result<(), String> {
        // The same rules as `udev::UdevData::set_output_enabled`.
        if enabled {
            let index = state
                .backend_data
                .switched_off
                .iter()
                .position(|off| off.name == name)
                .ok_or_else(|| format!("{name} is not a switched-off output"))?;
            let off = state.backend_data.switched_off.remove(index);
            state.prefs.output_mut(name).enabled = Some(true);
            state.connect_output(&off.name, &off.modes);
            if !state.space.outputs().any(|o| o.name() == name) {
                return Err(format!("{name} could not be switched on"));
            }
            Ok(())
        } else {
            if state.space.outputs().count() <= 1 {
                return Err("the last display cannot be switched off".into());
            }
            let modes = state
                .space
                .outputs()
                .find(|o| o.name() == name)
                .map(|o| o.modes())
                .ok_or_else(|| format!("no such output: {name}"))?;
            state.disconnect_output(name);
            state.backend_data.switched_off.push(SwitchedOff {
                name: name.to_string(),
                modes,
            });
            Ok(())
        }
    }

    fn set_blanked(state: &mut AnvilState<Self>, blanked: bool) {
        state.backend_data.blanked = blanked;
        if !blanked {
            state.mindbar.invalidate_graphics();
            for screen in &mut state.backend_data.screens {
                screen.age = 0;
                screen.dirty = true;
            }
        }
    }

    fn request_repaint(state: &mut AnvilState<Self>) {
        state.backend_data.mark_dirty(None);
    }

    fn request_repaint_on(state: &mut AnvilState<Self>, output: &Output) {
        // An output this backend does not know gets the same answer as on
        // DRM: every output repaints.
        let known = state.backend_data.screens.iter().any(|screen| &screen.output == output);
        state.backend_data.mark_dirty(known.then_some(output));
    }

    fn repaint_now(state: &mut AnvilState<Self>, output: &Output) {
        // The DRM backend draws on the next idle callback; the next turn is
        // the same moment here.
        Self::request_repaint_on(state, output);
    }
}

impl AnvilState<HeadlessData> {
    /// Plug in a display with these modes, the first one preferred, the way
    /// a connector appearing does: remembered settings apply, and one the
    /// user switched off stays off.
    pub fn add_output(&mut self, name: &str, modes: &[Mode]) {
        assert!(!modes.is_empty(), "a display has at least one mode");
        if self.prefs.outputs.get(name).and_then(|prefs| prefs.enabled) == Some(false) {
            self.backend_data.switched_off.push(SwitchedOff {
                name: name.to_string(),
                modes: modes.to_vec(),
            });
            return;
        }
        self.connect_output(name, modes);
        // What `udev::device_changed` does once its connectors are scanned.
        crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
        self.relayout_all_outputs();
    }

    /// Unplug a display.
    pub fn remove_output(&mut self, name: &str) {
        self.backend_data.switched_off.retain(|off| off.name != name);
        self.disconnect_output(name);
        crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
        self.relayout_all_outputs();
    }

    /// `udev::connector_connected`, less the hardware.
    fn connect_output(&mut self, name: &str, modes: &[Mode]) {
        let prefs = self.prefs.outputs.get(name).cloned().unwrap_or_default();
        let mode = prefs
            .mode
            .as_deref()
            .and_then(crate::prefs::parse_mode_key)
            .and_then(|(w, h, refresh)| {
                modes
                    .iter()
                    .find(|mode| mode.size.w == w && mode.size.h == h && mode.refresh == refresh)
            })
            .copied()
            .unwrap_or(modes[0]);

        let output = Output::new(
            name.to_string(),
            PhysicalProperties {
                size: (0, 0).into(),
                subpixel: Subpixel::Unknown,
                make: "MindOS".into(),
                model: "Headless".into(),
            },
        );
        let global = output.create_global::<AnvilState<HeadlessData>>(&self.display_handle);

        let x = self.space.outputs().fold(0, |acc, o| {
            acc + self.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0)
        });
        let position = prefs.position.map(|[x, y]| (x, y).into()).unwrap_or_else(|| (x, 0).into());
        let scale = prefs
            .scale
            .filter(|s| (0.5..=4.0).contains(s))
            .map(smithay::output::Scale::Fractional);
        let transform = prefs.transform.as_deref().and_then(crate::state::parse_transform);

        for mode in modes {
            output.add_mode(*mode);
        }
        output.set_preferred(modes[0]);
        output.change_current_state(Some(mode), transform, scale, Some(position));
        self.space.map_output(&output, position);

        let image = match self
            .backend_data
            .renderer
            .create_buffer(Fourcc::Xrgb8888, (mode.size.w, mode.size.h).into())
        {
            Ok(image) => image,
            Err(err) => panic!("cannot allocate a {}x{} image: {err}", mode.size.w, mode.size.h),
        };
        self.backend_data.screens.push(Screen {
            dh: self.display_handle.clone(),
            damage: OutputDamageTracker::from_output(&output),
            output,
            global: Some(global),
            image,
            age: 0,
            dirty: true,
            frames: 0,
        });
    }

    /// `udev::connector_disconnected`, less the hardware.
    fn disconnect_output(&mut self, name: &str) {
        let Some(index) = self.backend_data.screens.iter().position(|s| s.output.name() == name) else {
            return;
        };
        let screen = self.backend_data.screens.remove(index);
        self.space.unmap_output(&screen.output);
        if self.mindbar.output.as_deref() == Some(name) {
            self.mindbar.close();
            self.mindbar.output = None;
        }
    }

    /// Draw every output that asked for a frame, as the vblanks and repaint
    /// timers of the DRM backend would.
    pub fn render_dirty(&mut self) -> Result<(), RenderError> {
        let dirty: Vec<Output> = self
            .backend_data
            .screens
            .iter()
            .filter(|screen| screen.dirty)
            .map(|screen| screen.output.clone())
            .collect();
        for output in dirty {
            self.render_output(&output)?;
        }
        Ok(())
    }

    /// Draw one output: what `udev::render_surface` does, in the same order,
    /// into memory. Returns whether anything changed on it.
    pub fn render_output(&mut self, output: &Output) -> Result<bool, RenderError> {
        if self.idle.stage == crate::idle::Stage::Blank {
            // The displays are off: no frame, and no frame callbacks.
            return Ok(false);
        }
        let output_geometry = self.space.output_geometry(output).ok_or("the output is not mapped")?;
        let mode = output.current_mode().ok_or("the output has no mode")?;
        let refresh = Duration::from_secs_f64(1_000.0 / mode.refresh.max(1) as f64);
        let frame_target = self.clock.now() + refresh;

        self.pre_repaint(output, frame_target);

        let backend = &mut self.backend_data;
        let screen = backend.screens.iter_mut().find(|s| &s.output == output).ok_or("the output is off")?;
        screen.dirty = false;
        // A new mode leaves the image the wrong size.
        if (screen.image.width(), screen.image.height()) != (mode.size.w as usize, mode.size.h as usize) {
            screen.image = backend.renderer.create_buffer(Fourcc::Xrgb8888, (mode.size.w, mode.size.h).into())?;
            screen.age = 0;
        }
        let renderer = &mut backend.renderer;
        let scale = Scale::from(output.current_scale().fractional_scale());
        let pointer_location = self.pointer.current_location();

        let mut custom_elements: Vec<CustomRenderElements<PixmanRenderer>> = Vec::new();
        if output_geometry.to_f64().contains(pointer_location) {
            let cursor_hotspot = if let CursorImageStatus::Surface(ref surface) = self.cursor_status {
                compositor::with_states(surface, |states| {
                    states
                        .data_map
                        .get::<Mutex<CursorImageAttributes>>()
                        .map(|attrs| attrs.lock_anyway())
                        .map(|attrs| attrs.hotspot)
                        .unwrap_or_default()
                })
            } else {
                (0, 0).into()
            };
            let cursor_pos = pointer_location - output_geometry.loc.to_f64();
            backend.pointer_element.set_buffer(backend.pointer_image.clone(), (0, 0).into());
            if let CursorImageStatus::Surface(ref surface) = self.cursor_status {
                if !surface.alive() {
                    self.cursor_status = CursorImageStatus::default_named();
                }
            }
            backend.pointer_element.set_status(self.cursor_status.clone());
            custom_elements.extend(backend.pointer_element.render_elements(
                renderer,
                (cursor_pos - cursor_hotspot.to_f64()).to_physical(scale).to_i32_round(),
                scale,
                1.0,
            ));
            if let Some(icon) = self.dnd_icon.as_ref() {
                if icon.surface.alive() {
                    custom_elements.extend(AsRenderElements::<PixmanRenderer>::render_elements(
                        &SurfaceTree::from_surface(&icon.surface),
                        renderer,
                        (cursor_pos + icon.offset.to_f64()).to_physical(scale).to_i32_round(),
                        scale,
                        1.0,
                    ));
                }
            }
        }

        let locked = self.idle.locked;
        if !locked {
            if let Some(osd) = self.mindbar.render_osd(renderer, &output.name(), output_geometry.size, scale.x) {
                custom_elements.push(CustomRenderElements::Overlay(osd));
            }
        }
        if let Some(bar) = self.mindbar.render_element(renderer, &output.name(), output_geometry.size, scale.x) {
            custom_elements.push(CustomRenderElements::Overlay(bar));
        }
        let desktop_up = crate::shell::desktop_up(output);
        let backdrop: Vec<_> = self
            .mindbar
            .backdrop_elements(renderer, output_geometry.size, scale.x, desktop_up, self.config.theme.show_wordmark)
            .into_iter()
            .map(CustomRenderElements::Overlay)
            .collect();

        let (elements, clear_color) = output_elements(
            output,
            &self.space,
            custom_elements,
            renderer,
            self.show_window_preview,
            backdrop,
            locked,
        );

        let age = screen.age;
        let mut target = renderer.bind(&mut screen.image)?;
        let result = screen.damage.render_output(renderer, &mut target, age, &elements, clear_color)?;
        let rendered = result.damage.is_some();
        let states = result.states;
        drop(target);
        screen.age = 1;
        if rendered {
            screen.frames += 1;
        }

        update_primary_scanout_output(&self.space, output, &self.dnd_icon, &self.cursor_status, &states);
        if rendered {
            let mut feedback = take_presentation_feedback(output, &self.space, &states);
            feedback.presented(frame_target, Refresh::fixed(refresh), 0, wp_presentation_feedback::Kind::Vsync);
        }
        self.post_repaint(output, frame_target, None, &states);
        Ok(rendered)
    }

    /// The colour at one pixel of what the output called `name` last showed,
    /// as `0xRRGGBB`, in the image's own pixels.
    pub fn pixel(&mut self, name: &str, x: i32, y: i32) -> Option<u32> {
        let backend = &mut self.backend_data;
        let screen = backend.screens.iter_mut().find(|s| s.output.name() == name)?;
        let target = backend.renderer.bind(&mut screen.image).ok()?;
        let copy = backend
            .renderer
            .copy_framebuffer(&target, Rectangle::new((x, y).into(), (1, 1).into()), Fourcc::Xrgb8888)
            .ok()?;
        let bytes = backend.renderer.map_texture(&copy).ok()?;
        Some(u32::from_ne_bytes(bytes.get(..4)?.try_into().ok()?) & 0x00ff_ffff)
    }
}

/// A compositor and the loop that runs it.
pub struct Headless {
    pub event_loop: EventLoop<'static, AnvilState<HeadlessData>>,
    pub state: AnvilState<HeadlessData>,
}

impl Headless {
    /// Start a compositor with this configuration. It listens on a Wayland
    /// socket and an IPC socket in `$XDG_RUNTIME_DIR`, as the real one does,
    /// and has no outputs until `add_output`.
    ///
    /// The loop watchdog watches the calling thread from here on: a turn
    /// that never returns aborts the process rather than hang it, the way it
    /// ends a session.
    pub fn new(config: Config) -> Result<Headless, RenderError> {
        let event_loop = EventLoop::try_new()?;
        let display = Display::new()?;
        let mut state = AnvilState::init_with_config(display, event_loop.handle(), HeadlessData::new()?, true, config);
        let formats: Vec<_> = state.backend_data.renderer.shm_formats().collect();
        state.shm_state.update_formats(formats);
        // No session to start, and no Xwayland to wait for.
        state.startup_done = true;
        state.loop_watch.watch();
        Ok(Headless { event_loop, state })
    }

    /// One turn of the loop, without waiting: what came in is handled, the
    /// outputs that asked for a frame get one, and the replies go out.
    pub fn turn(&mut self) {
        self.turn_waiting(Duration::ZERO);
    }

    /// One turn, waiting up to `timeout` for something to arrive first.
    ///
    /// A panic is not caught: the DRM backend's `recover::Recovery` would
    /// log it and go on, and a test wants to see it.
    pub fn turn_waiting(&mut self, timeout: Duration) {
        if let Err(err) = self.event_loop.dispatch(Some(timeout), &mut self.state) {
            panic!("the event loop failed: {err}");
        }
        self.state.loop_watch.beat();
        if let Err(err) = self.state.render_dirty() {
            panic!("a frame failed: {err}");
        }
        self.state.after_dispatch();
    }
}
