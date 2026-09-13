use std::{
    collections::hash_map::HashMap,
    io,
    ops::Not,
    os::fd::{AsFd, BorrowedFd},
    path::Path,
    sync::{atomic::Ordering, Mutex, Once},
    time::{Duration, Instant},
};

use crate::{
    config::DirectScanout,
    drawing::*,
    ipc::{ModeInfo, OutputInfo},
    recovery::{self, Cause, Failure, Step},
    render::*,
    shell::WindowElement,
    stall::{self, Phase},
    state::{take_presentation_feedback, update_primary_scanout_output, AnvilState, Backend},
};
use crate::{
    recover::LockAnyway,
    shell::WindowRenderElement,
    state::{DndIcon, SurfaceDmabufFeedback},
};
#[cfg(feature = "renderer_sync")]
use smithay::backend::drm::compositor::PrimaryPlaneElement;
#[cfg(feature = "egl")]
use smithay::backend::renderer::ImportEgl;
#[cfg(feature = "debug")]
use smithay::backend::renderer::{multigpu::MultiTexture, ImportMem};
use smithay::{
    backend::{
        allocator::{
            dmabuf::Dmabuf,
            format::FormatSet,
            gbm::{GbmAllocator, GbmBufferFlags, GbmDevice},
            Fourcc, Modifier,
        },
        drm::{
            compositor::{DrmCompositor, FrameFlags},
            VrrSupport,
            exporter::gbm::GbmFramebufferExporter,
            output::{DrmOutput, DrmOutputManager, DrmOutputRenderElements},
            CreateDrmNodeError, DrmDevice, DrmDeviceFd, DrmError, DrmEvent, DrmEventMetadata,
            DrmEventTime, DrmNode, DrmSurface, GbmBufferedSurface, NodeType,
        },
        egl::{self, context::ContextPriority, EGLDevice, EGLDisplay},
        input::InputEvent,
        libinput::{LibinputInputBackend, LibinputSessionInterface},
        renderer::{
            damage::Error as OutputDamageTrackerError,
            element::{
                memory::MemoryRenderBuffer, AsRenderElements, Element as RenderElement,
                Id as RenderElementId, RenderElementStates,
            },
            gles::GlesRenderer,
            multigpu::{gbm::GbmGlesBackend, GpuManager, MultiRenderer},
            DebugFlags, ImportDma, ImportMemWl,
        },
        session::{
            libseat::{self, LibSeatSession},
            Event as SessionEvent, Session,
        },
        udev::{all_gpus, primary_gpu, UdevBackend, UdevEvent},
        SwapBuffersError,
    },
    delegate_dmabuf, delegate_drm_lease,
    desktop::{
        space::{Space, SurfaceTree},
        utils::OutputPresentationFeedback,
    },
    input::{
        keyboard::LedState,
        pointer::{CursorIcon, CursorImageAttributes, CursorImageStatus},
    },
    output::{Mode as WlMode, Output, PhysicalProperties},
    reexports::{
        calloop::{
            channel,
            timer::{TimeoutAction, Timer},
            EventLoop, LoopHandle, RegistrationToken,
        },
        drm::{
            control::{
                atomic::AtomicModeReq, connector, crtc, AtomicCommitFlags, Device, Mode as DrmMode,
                ModeTypeFlags,
            },
            Device as _,
        },
        input::{DeviceCapability, Libinput},
        rustix::fs::OFlags,
        wayland_protocols::wp::{
            linux_dmabuf::zv1::server::zwp_linux_dmabuf_feedback_v1,
            presentation_time::server::wp_presentation_feedback,
        },
        wayland_server::{backend::GlobalId, protocol::wl_surface, Display, DisplayHandle},
    },
    utils::{DeviceFd, IsAlive, Logical, Monotonic, Point, Scale, Time, Transform},
    wayland::{
        compositor,
        dmabuf::{DmabufFeedbackBuilder, DmabufGlobal, DmabufHandler, DmabufState, ImportNotifier},
        drm_lease::{
            DrmLease, DrmLeaseBuilder, DrmLeaseHandler, DrmLeaseRequest, DrmLeaseState, LeaseRejected,
        },
        drm_syncobj::{supports_syncobj_eventfd, DrmSyncobjHandler, DrmSyncobjState},
        presentation::Refresh,
    },
};
use crate::edid;
use smithay_drm_extras::{
    drm_scanner::{DrmScanEvent, DrmScanner},
};
use tracing::{debug, error, info, trace, warn};

// we cannot simply pick the first supported format of the intersection of *all* formats, because:
// - we do not want something like Abgr4444, which looses color information, if something better is available
// - some formats might perform terribly
// - we might need some work-arounds, if one supports modifiers, but the other does not
//
// So lets just pick `ARGB2101010` (10-bit) or `ARGB8888` (8-bit) for now, they are widely supported.
const SUPPORTED_FORMATS: &[Fourcc] = &[
    Fourcc::Abgr2101010,
    Fourcc::Argb2101010,
    Fourcc::Abgr8888,
    Fourcc::Argb8888,
];
const SUPPORTED_FORMATS_8BIT_ONLY: &[Fourcc] = &[Fourcc::Abgr8888, Fourcc::Argb8888];

type UdevRenderer<'a> = MultiRenderer<
    'a,
    'a,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
    GbmGlesBackend<GlesRenderer, DrmDeviceFd>,
>;

#[derive(Debug, Clone, Copy, PartialEq)]
struct UdevOutputId {
    device_id: DrmNode,
    crtc: crtc::Handle,
}

pub struct UdevData {
    software_rendering: bool,
    pub session: LibSeatSession,
    dh: DisplayHandle,
    dmabuf_state: Option<(DmabufState, DmabufGlobal)>,
    syncobj_state: Option<DrmSyncobjState>,
    primary_gpu: DrmNode,
    gpus: GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    backends: HashMap<DrmNode, BackendData>,
    /// One texture per animation frame of every shape drawn so far,
    /// keyed by `(shape, frame)`.
    pointer_images: HashMap<(CursorIcon, usize), MemoryRenderBuffer>,
    pointer_element: PointerElement,
    #[cfg(feature = "debug")]
    fps_texture: Option<MultiTexture>,
    pointer_image: crate::cursor::Cursor,
    debug_flags: DebugFlags,
    keyboards: Vec<smithay::reexports::input::Device>,
    mice: Vec<smithay::reexports::input::Device>,
    /// Where the helper threads that wait on the kernel send their answers.
    answers: channel::Sender<Answer>,
    /// Devices with udev change events gathered until `hotplug_timer` fires.
    hotplug: Vec<DrmNode>,
    hotplug_timer: Option<RegistrationToken>,
    /// Failures injected on purpose, when `MINDWM_DEBUG_FAULTS` is set.
    faults: Option<Faults>,
}

impl UdevData {
    pub fn set_debug_flags(&mut self, flags: DebugFlags) {
        if self.debug_flags != flags {
            self.debug_flags = flags;

            for (_, backend) in self.backends.iter_mut() {
                for (_, surface) in backend.surfaces.iter_mut() {
                    surface.drm_output.set_debug_flags(flags);
                }
            }
        }
    }

    pub fn debug_flags(&self) -> DebugFlags {
        self.debug_flags
    }
}

impl DmabufHandler for AnvilState<UdevData> {
    fn dmabuf_state(&mut self) -> &mut DmabufState {
        &mut self.backend_data.dmabuf_state.as_mut().unwrap().0
    }

    fn dmabuf_imported(&mut self, _global: &DmabufGlobal, dmabuf: Dmabuf, notifier: ImportNotifier) {
        if self
            .backend_data
            .gpus
            .single_renderer(&self.backend_data.primary_gpu)
            .and_then(|mut renderer| renderer.import_dmabuf(&dmabuf, None))
            .is_ok()
        {
            dmabuf.set_node(self.backend_data.primary_gpu);
            let _ = notifier.successful::<AnvilState<UdevData>>();
        } else {
            notifier.failed();
        }
    }
}
delegate_dmabuf!(AnvilState<UdevData>);

impl Backend for UdevData {
    fn software_rendering(&self) -> bool { self.software_rendering }
    fn set_mouse_settings(&mut self, settings: &crate::input_config::InputSettings) {
        for device in &mut self.mice { crate::input_config::apply_mouse(device, settings); }
    }
    fn mouse_devices(&self) -> Vec<serde_json::Value> {
        self.mice.iter().map(crate::input_config::mouse_info).collect()
    }
    const HAS_RELATIVE_MOTION: bool = true;
    const HAS_GESTURES: bool = true;

    fn set_cursor(&mut self, theme: &str, size: u32) {
        self.pointer_image = crate::cursor::Cursor::load_theme(theme, size);
        // The frame cache is keyed on the image; drop it or the old size stays.
        self.pointer_images.clear();
    }

    fn seat_name(&self) -> String {
        self.session.seat()
    }

    fn reset_buffers(&mut self, output: &Output) {
        if let Some(id) = output.user_data().get::<UdevOutputId>() {
            if let Some(gpu) = self.backends.get_mut(&id.device_id) {
                if let Some(surface) = gpu.surfaces.get_mut(&id.crtc) {
                    // Forget what each buffer holds, so the next frames are
                    // drawn whole. The buffers themselves stay: dropping them
                    // makes the next frames allocate and register new ones on
                    // this thread, a hitch at 4K for nothing a whole redraw
                    // does not already do.
                    surface.drm_output.with_compositor(|compositor| compositor.reset_buffer_ages());
                }
            }
        }
    }

    fn early_import(&mut self, surface: &wl_surface::WlSurface) {
        if let Err(err) = self.gpus.early_import(self.primary_gpu, surface) {
            warn!("Early buffer import failed: {}", err);
        }
    }

    fn update_led_state(&mut self, led_state: LedState) {
        for keyboard in self.keyboards.iter_mut() {
            keyboard.led_update(led_state.into());
        }
    }

    fn set_output_mode(&mut self, output: &Output, mode: WlMode) -> Result<(), String> {
        let id = *output
            .user_data()
            .get::<UdevOutputId>()
            .ok_or("not a DRM output")?;
        let UdevData {
            backends,
            gpus,
            primary_gpu,
            ..
        } = self;
        let device = backends.get_mut(&id.device_id).ok_or("the GPU is gone")?;
        // A new mode can mean new buffers and a test of every output on the
        // GPU; in the middle of a reset it would test against CRTCs a helper
        // thread is switching off.
        if device.surfaces.values().any(SurfaceData::recovering) {
            return Err("a display is being reset; try again in a moment".into());
        }
        // Changing a mode can modeset the device, so nothing on it can rely
        // on the frame it had in flight. `after_output_change` repaints, and
        // that is where the flips are dropped: this runs on `UdevData`, which
        // has no way to reach the event loop.
        device.reconfigured = true;
        let render_node = device.render_node.unwrap_or(*primary_gpu);
        let surface = device.surfaces.get_mut(&id.crtc).ok_or("the output is off")?;
        let drm_mode = surface
            .modes
            .iter()
            .copied()
            .find(|m| WlMode::from(*m) == mode)
            .ok_or_else(|| {
                format!(
                    "{} has no {}x{} @ {:.3} Hz mode",
                    output.name(),
                    mode.size.w,
                    mode.size.h,
                    mode.refresh as f64 / 1000.0
                )
            })?;
        let mut renderer = gpus.single_renderer(&render_node).map_err(|err| err.to_string())?;
        surface
            .drm_output
            .use_mode::<_, OutputRenderElements<UdevRenderer<'_>, WindowRenderElement<UdevRenderer<'_>>>>(
                drm_mode,
                &mut renderer,
                &DrmOutputRenderElements::default(),
            )
            .map_err(|err| err.to_string())?;
        // The refresh rate is very likely different now, so every frame time
        // this output remembers belongs to a display that no longer exists.
        // Keeping them would aim the next frame at a vblank from the old
        // cadence and, at a lower rate, make the vblank throttle believe the
        // display is running fast. `after_output_change` repaints.
        surface.last_presentation_time = None;
        surface.frame_target = None;
        surface.render_times = crate::timing::RenderTimes::default();
        surface.last_render = Instant::now();
        surface.dirty = true;
        info!(output = output.name(), width = mode.size.w, height = mode.size.h, refresh = mode.refresh, "mode set");
        Ok(())
    }

    fn set_output_vrr(&mut self, output: &Output, enabled: bool) -> Result<(), String> {
        let id = *output
            .user_data()
            .get::<UdevOutputId>()
            .ok_or("not a DRM output")?;
        let surface = self
            .backends
            .get_mut(&id.device_id)
            .and_then(|d| d.surfaces.get_mut(&id.crtc))
            .ok_or("the output is off")?;
        if !surface.vrr_supported {
            return Err(format!("{} does not support variable refresh rate", output.name()));
        }
        if surface.recovering() {
            return Err(format!("{} is being reset; try again in a moment", output.name()));
        }
        surface
            .drm_output
            .with_compositor(|compositor| compositor.use_vrr(enabled))
            .map_err(|err| err.to_string())?;
        // With variable refresh the vblank cadence is no longer the mode's,
        // so the frame times from before it was switched mean nothing.
        surface.last_presentation_time = None;
        surface.frame_target = None;
        surface.last_render = Instant::now();
        surface.dirty = true;
        Ok(())
    }

    fn output_vrr(&self, output: &Output) -> (bool, bool) {
        let Some(id) = output.user_data().get::<UdevOutputId>() else {
            return (false, false);
        };
        let Some(surface) = self
            .backends
            .get(&id.device_id)
            .and_then(|d| d.surfaces.get(&id.crtc))
        else {
            return (false, false);
        };
        (
            surface.vrr_supported,
            surface.drm_output.with_compositor(|compositor| compositor.vrr_enabled()),
        )
    }

    fn disabled_outputs(&self) -> Vec<OutputInfo> {
        self.backends
            .values()
            .flat_map(|device| device.disabled.iter())
            .map(|d| {
                let (mm_w, mm_h) = d.connector.size().unwrap_or((0, 0));
                OutputInfo {
                    software_rendering: self.software_rendering,
                    name: d.name.clone(),
                    make: d.make.clone(),
                    model: d.model.clone(),
                    x: 0,
                    y: 0,
                    width: 0,
                    height: 0,
                    scale: 1.0,
                    refresh: 0.0,
                    transform: "normal".into(),
                    modes: d
                        .connector
                        .modes()
                        .iter()
                        .map(|m| {
                            let wl = WlMode::from(*m);
                            ModeInfo {
                                width: wl.size.w,
                                height: wl.size.h,
                                refresh: wl.refresh,
                                preferred: m.mode_type().contains(ModeTypeFlags::PREFERRED),
                                current: false,
                            }
                        })
                        .collect(),
                    enabled: false,
                    vrr: false,
                    vrr_supported: false,
                    primary: false,
                    mm_width: mm_w as i32,
                    mm_height: mm_h as i32,
                }
            })
            .collect()
    }

    /// Switch every display off with DPMS, or light them again. Clearing a
    /// DRM surface stops its page flips, so the render loop for that output
    /// stops with it; lighting up has to kick a frame to start it again.
    fn request_repaint(state: &mut AnvilState<Self>) {
        repaint_outputs(state, None);
    }

    fn request_repaint_on(state: &mut AnvilState<Self>, output: &Output) {
        // An output this backend has never heard of is no reason to leave a
        // display without a frame: repaint every one of them and lose a
        // little time rather than risk a screen that stops.
        match output.user_data().get::<UdevOutputId>().copied() {
            Some(id) => repaint_outputs(state, Some(id)),
            None => repaint_outputs(state, None),
        }
    }

    fn repaint_now(state: &mut AnvilState<Self>, output: &Output) {
        let Some(id) = output.user_data().get::<UdevOutputId>().copied() else {
            return;
        };
        let Some(surface) = state
            .backend_data
            .backends
            .get_mut(&id.device_id)
            .and_then(|device| device.surfaces.get_mut(&id.crtc))
        else {
            return;
        };
        surface.dirty = true;
        // A frame in flight, a reset, or the retry of a refused frame
        // continues the loop by itself, and drawing now would get in its way.
        if surface.busy() || surface.health.refused > 0 {
            return;
        }
        // A timer is only ever armed once the previous flip completed, so
        // drawing now cannot queue a second frame behind one in flight.
        let Some(armed) = surface.repaint_timer.take() else {
            return;
        };
        state.handle.remove(armed.token);
        let (node, crtc) = (id.device_id, id.crtc);
        state.handle.insert_idle(move |data| {
            let target = armed.target.unwrap_or_else(|| data.repaint_target(node, crtc));
            data.render(node, Some(crtc), target);
        });
    }

    fn set_blanked(state: &mut AnvilState<Self>, blanked: bool) {
        if blanked {
            let handle = state.handle.clone();
            for device in state.backend_data.backends.values_mut() {
                let active = device.drm_output_manager.device().is_active();
                for surface in device.surfaces.values_mut() {
                    // The loop stops here and starts from scratch when the
                    // displays light up, so whatever it was waiting on goes:
                    // the vblank of a frame that clearing drops is not
                    // coming, and a reset waiting out its backoff is moot.
                    cancel_timers(&handle, surface);
                    surface.flip = None;
                    surface.flip_scanout = None;
                    surface.health.refusals_over();
                    if surface.releasing.is_some() || !active {
                        // A helper thread holds the CRTC, and switches the
                        // display off when it is done; or the session is
                        // paused and resuming switches everything off anyway.
                        continue;
                    }
                    blank_surface(surface);
                }
            }
            return;
        }
        let nodes: Vec<DrmNode> = state.backend_data.backends.keys().copied().collect();
        state.mindbar.invalidate_graphics();
        for node in nodes {
            if let Some(device) = state.backend_data.backends.get_mut(&node) {
                for surface in device.surfaces.values_mut() {
                    surface.drm_output.with_compositor(|compositor| compositor.reset_buffer_ages());
                    // Lighting up starts the schedule over. Keeping the old
                    // presentation time would aim the first frame at a vblank
                    // that belongs to before the displays went off.
                    surface.last_presentation_time = None;
                    surface.frame_target = None;
                    surface.last_render = Instant::now();
                    surface.dead_ticks = 0;
                    surface.dirty = true;
                }
            }
            state
                .handle
                .insert_idle(move |data| data.render(node, None, data.clock.now()));
        }
    }

    fn reset_displays(state: &mut AnvilState<Self>) {
        if state.idle.stage == crate::idle::Stage::Blank {
            // Lighting the displays up again redraws everything from scratch.
            return;
        }
        let outputs: Vec<(UdevOutputId, String)> = state
            .space
            .outputs()
            .filter_map(|o| o.user_data().get::<UdevOutputId>().map(|id| (*id, o.name())))
            .collect();
        info!(displays = outputs.len(), "resetting every display, as asked");
        for (id, name) in outputs {
            state.recover_output(id.device_id, id.crtc, Cause::Requested, &format!("{name} was asked to reset"));
        }
    }

    fn debug_fault(
        state: &mut AnvilState<Self>,
        fault: &str,
        output: Option<&str>,
        count: Option<u32>,
        ms: Option<u64>,
    ) -> Result<(), String> {
        let target = output.unwrap_or("*").to_string();
        if target != "*" && !state.space.outputs().any(|o| o.name() == target) {
            return Err(format!("no such output: {target}"));
        }
        let Some(faults) = state.backend_data.faults.as_mut() else {
            return Err("faults are only injected when mindwm runs with MINDWM_DEBUG_FAULTS set".into());
        };
        match fault {
            "lose_vblank" => {
                let count = count.unwrap_or(1).max(1);
                warn!(output = %target, count, "debug fault: page flip events will be dropped");
                *faults.lose_vblanks.entry(target).or_default() += count;
            }
            "reject_frames" => {
                let ms = ms.unwrap_or(3000);
                warn!(output = %target, ms, "debug fault: frames will be refused");
                faults
                    .reject_until
                    .insert(target, Instant::now() + Duration::from_millis(ms));
            }
            other => {
                return Err(format!(
                    "unknown fault {other}: expected lose_vblank, reject_frames or stall"
                ))
            }
        }
        Ok(())
    }

    fn graphics_stats(state: &AnvilState<Self>) -> serde_json::Value {
        let mut outputs = Vec::new();
        for (node, device) in &state.backend_data.backends {
            let active = device.drm_output_manager.device().is_active();
            for (crtc, surface) in &device.surfaces {
                let id = UdevOutputId { device_id: *node, crtc: *crtc };
                let output = state
                    .space
                    .outputs()
                    .find(|o| o.user_data().get::<UdevOutputId>() == Some(&id));
                let name = output.map(|o| o.name()).unwrap_or_else(|| format!("{crtc:?}"));
                let refresh = output.and_then(|o| o.current_mode()).map(|m| m.refresh).unwrap_or(0);
                outputs.push(serde_json::json!({
                    "output": name,
                    "device_active": active,
                    "refresh_mhz": refresh,
                    "frames": surface.frames,
                    "repaints": surface.repaints,
                    "late_frames": surface.late_frames,
                    "resets": surface.resets,
                    "recovering": surface.recovering(),
                    "recoveries": surface.health.recoveries,
                    "refused": surface.health.refused,
                    "render_last_us": surface.render_times.last.as_micros() as u64,
                    "render_recent_us": surface.render_times.estimate().as_micros() as u64,
                    "render_worst_us": surface.render_times.worst.as_micros() as u64,
                    "since_render_ms": surface.last_render.elapsed().as_millis() as u64,
                    "flip_in_flight": surface.flip.is_some(),
                    "timer_armed": surface.repaint_timer.is_some(),
                    "direct_scanout": match surface.scanout {
                        DirectScanout::Any => "any",
                        DirectScanout::Matching => "matching",
                        DirectScanout::Off => "off",
                    },
                }));
            }
        }
        serde_json::json!({
            "session_active": state.backend_data.session.is_active(),
            "software_rendering": state.backend_data.software_rendering,
            // The event loop's own numbers are added by `graphics_report`,
            // which is the one place that has them for both backends.
            "outputs": outputs,
        })
    }

    fn set_output_enabled(state: &mut AnvilState<Self>, name: &str, enabled: bool) -> Result<(), String> {
        // Lighting an output tests it against every other output on the GPU,
        // and switching one off commits; in the middle of a reset either
        // would wait on the CRTCs a helper thread is switching off.
        const RESETTING: &str = "a display is being reset; try again in a moment";
        if enabled {
            let found = state.backend_data.backends.iter_mut().find_map(|(node, device)| {
                let i = device.disabled.iter().position(|d| d.name == name)?;
                Some((*node, device, i))
            });
            let (node, device, i) = found.ok_or_else(|| format!("{name} is not a switched-off output"))?;
            if device.surfaces.values().any(SurfaceData::recovering) {
                return Err(RESETTING.into());
            }
            let disabled = device.disabled.remove(i);
            state.prefs.output_mut(name).enabled = Some(true);
            state.connector_connected(node, disabled.connector, disabled.crtc);
            if !state.space.outputs().any(|o| o.name() == name) {
                return Err(format!("{name} could not be switched on"));
            }
            Ok(())
        } else {
            if state.space.outputs().count() <= 1 {
                return Err("the last display cannot be switched off".into());
            }
            let output = state
                .space
                .outputs()
                .find(|o| o.name() == name)
                .cloned()
                .ok_or_else(|| format!("no such output: {name}"))?;
            let id = *output
                .user_data()
                .get::<UdevOutputId>()
                .ok_or("not a DRM output")?;
            let device = state
                .backend_data
                .backends
                .get_mut(&id.device_id)
                .ok_or("the GPU is gone")?;
            if device.surfaces.values().any(SurfaceData::recovering) {
                return Err(RESETTING.into());
            }
            let handle = device
                .surfaces
                .get(&id.crtc)
                .map(|s| s.connector)
                .ok_or("the output is off")?;
            let info = device
                .drm_output_manager
                .device()
                .get_connector(handle, false)
                .map_err(|err| err.to_string())?;
            let props = output.physical_properties();
            state.connector_disconnected(id.device_id, info.clone(), id.crtc);
            if let Some(device) = state.backend_data.backends.get_mut(&id.device_id) {
                device.disabled.push(DisabledConnector {
                    connector: info,
                    crtc: id.crtc,
                    name: name.to_string(),
                    make: props.make,
                    model: props.model,
                });
            }
            Ok(())
        }
    }
}

pub fn run_udev() {
    let mut event_loop = EventLoop::try_new().unwrap();
    let display = Display::new().unwrap();
    let display_handle = display.handle();

    /*
     * Initialize session
     */
    let (session, notifier) = match LibSeatSession::new() {
        Ok(ret) => ret,
        Err(err) => {
            error!("Could not initialize a session: {}", err);
            return;
        }
    };

    /*
     * Initialize the compositor
     */
    let mut primary_gpu = if let Ok(var) = std::env::var("ANVIL_DRM_DEVICE") {
        DrmNode::from_path(var).expect("Invalid drm device path")
    } else {
        primary_gpu(session.seat())
            .unwrap()
            .and_then(|x| DrmNode::from_path(x).ok()?.node_with_type(NodeType::Render)?.ok())
            .unwrap_or_else(|| {
                all_gpus(session.seat())
                    .unwrap()
                    .into_iter()
                    .find_map(|x| DrmNode::from_path(x).ok())
                    .expect("No GPU!")
            })
    };
    info!("Using {} as primary gpu.", primary_gpu);

    let gpus = GpuManager::new(GbmGlesBackend::with_context_priority(ContextPriority::High)).unwrap();

    let (answers, answers_rx) = channel::channel::<Answer>();
    let faults = std::env::var_os("MINDWM_DEBUG_FAULTS").map(|_| {
        warn!("MINDWM_DEBUG_FAULTS is set: the debug_fault request can freeze displays on purpose");
        Faults::default()
    });

    let data = UdevData {
        software_rendering: false,
        dh: display_handle.clone(),
        dmabuf_state: None,
        syncobj_state: None,
        session,
        primary_gpu,
        gpus,
        backends: HashMap::new(),
        pointer_image: crate::cursor::Cursor::load(),
        pointer_images: HashMap::new(),
        pointer_element: PointerElement::default(),
        #[cfg(feature = "debug")]
        fps_texture: None,
        debug_flags: DebugFlags::empty(),
        keyboards: Vec::new(),
        mice: Vec::new(),
        answers,
        hotplug: Vec::new(),
        hotplug_timer: None,
        faults,
    };
    let mut state = AnvilState::init(display, event_loop.handle(), data, true);

    event_loop
        .handle()
        .insert_source(answers_rx, |event, _, data| {
            let channel::Event::Msg(answer) = event else {
                return;
            };
            match answer {
                Answer::Released {
                    node,
                    crtc,
                    started,
                    result,
                } => data.release_done(node, crtc, started, result),
                Answer::Probed { node, took } => data.probe_done(node, took),
            }
        })
        .expect("failed to listen to the display helper threads");

    /*
     * Initialize the udev backend
     */
    let udev_backend = match UdevBackend::new(&state.seat_name) {
        Ok(ret) => ret,
        Err(err) => {
            error!(error = ?err, "Failed to initialize udev backend");
            return;
        }
    };

    /*
     * Initialize libinput backend
     */
    let mut libinput_context = Libinput::new_with_udev::<LibinputSessionInterface<LibSeatSession>>(
        state.backend_data.session.clone().into(),
    );
    libinput_context.udev_assign_seat(&state.seat_name).unwrap();
    let libinput_backend = LibinputInputBackend::new(libinput_context.clone());

    /*
     * Bind all our objects that get driven by the event loop
     */
    event_loop
        .handle()
        .insert_source(libinput_backend, move |mut event, _, data| {
            let _phase = stall::enter(Phase::Input);
            let dh = data.backend_data.dh.clone();
            if let InputEvent::DeviceAdded { device } = &mut event {
                if crate::input_config::is_mouse(device) {
                    crate::input_config::apply_mouse(device, &data.prefs.input);
                    data.backend_data.mice.push(device.clone());
                }
                if device.has_capability(DeviceCapability::Keyboard) {
                    if let Some(led_state) = data.seat.get_keyboard().map(|keyboard| keyboard.led_state()) {
                        device.led_update(led_state.into());
                    }
                    data.backend_data.keyboards.push(device.clone());
                }
            } else if let InputEvent::DeviceRemoved { ref device } = event {
                data.backend_data.mice.retain(|item| item != device);
                if device.has_capability(DeviceCapability::Keyboard) {
                    data.backend_data.keyboards.retain(|item| item != device);
                    data.media_keys.stop();
                }
            }

            data.process_input_event(&dh, event)
        })
        .unwrap();

    event_loop
        .handle()
        .insert_source(notifier, move |event, &mut (), data| match event {
            SessionEvent::PauseSession => {
                let _phase = stall::enter(Phase::Session);
                data.media_keys.stop();
                data.mindbar.clear_osd();
                libinput_context.suspend();
                info!("pausing session");

                for backend in data.backend_data.backends.values_mut() {
                    backend.drm_output_manager.pause();
                    backend.active_leases.clear();
                    if let Some(lease_global) = backend.leasing_global.as_mut() {
                        lease_global.suspend();
                    }
                }
            }
            SessionEvent::ActivateSession => {
                let _phase = stall::enter(Phase::Session);
                info!("resuming session");
                data.mindbar.invalidate_graphics();

                if let Err(err) = libinput_context.resume() {
                    error!("Failed to resume libinput context: {:?}", err);
                }
                let handle = data.handle.clone();
                for (node, backend) in data
                    .backend_data
                    .backends
                    .iter_mut()
                    .map(|(handle, backend)| (*handle, backend))
                {
                    // Everything the loop was waiting on belongs to the time
                    // before the handover. The displays were off, so the page
                    // flips in flight lost their events, and the frames the
                    // compositor still holds for them would keep every new
                    // frame from being queued. A reset under way is overtaken
                    // by the one below; its answer finds nothing to finish.
                    for surface in backend.surfaces.values_mut() {
                        cancel_timers(&handle, surface);
                        surface.flip = None;
                        surface.flip_scanout = None;
                        surface.releasing = None;
                        forget_frame(surface);
                        surface.health.refusals_over();
                        surface.last_presentation_time = None;
                        surface.frame_target = None;
                        surface.dead_ticks = 0;
                        surface.dirty = true;
                    }
                    // Hardware state cannot be trusted after suspend or a VT
                    // handoff: every CRTC goes off, and each is lit again by
                    // its next frame.
                    if let Err(err) = backend.drm_output_manager.activate(true) {
                        // Not the end of this device: the watchdog tries
                        // again every couple of seconds rather than leave
                        // every display on it dark for the rest of the
                        // session, which is what this used to do.
                        error!(?err, "failed to reactivate DRM backend, trying again shortly");
                        backend.inactive_since = Some(Instant::now());
                        continue;
                    }
                    backend.inactive_since = None;
                    for surface in backend.surfaces.values_mut() {
                        surface.drm_output.with_compositor(|compositor| {
                            // A mode blob from before the handover can be
                            // refused from now on; lighting up with a new one
                            // cannot. And what each buffer held is anyone's
                            // guess, so the first frames are drawn whole.
                            if let Err(err) = compositor.use_mode(compositor.pending_mode()) {
                                warn!(?err, "cannot set the mode again after resuming");
                            }
                            compositor.reset_buffer_ages();
                        });
                        // The loop starts again here, so the watchdog counts
                        // from the resume and not from before the handover.
                        surface.last_render = Instant::now();
                    }
                    if let Some(lease_global) = backend.leasing_global.as_mut() {
                        lease_global.resume::<AnvilState<UdevData>>();
                    }
                    data.handle
                        .insert_idle(move |data| data.render(node, None, data.clock.now()));
                }
            }
        })
        .unwrap();

    // We try to initialize the primary node before others to make sure
    // any display only node can fall back to the primary node for rendering
    let primary_node = primary_gpu
        .node_with_type(NodeType::Primary)
        .and_then(|node| node.ok());
    let primary_device = udev_backend.device_list().find(|(device_id, _)| {
        primary_node
            .map(|primary_node| *device_id == primary_node.dev_id())
            .unwrap_or(false)
            || *device_id == primary_gpu.dev_id()
    });

    if let Some((device_id, path)) = primary_device {
        match DrmNode::from_dev_id(device_id) {
            Ok(node) => {
                if let Err(err) = state.device_added(node, path) {
                    error!("failed to initialize primary device {node}: {err}");
                }
            }
            Err(err) => error!("failed to get primary node: {err}"),
        }
    }

    let primary_device_id = primary_device.map(|(device_id, _)| device_id);
    for (device_id, path) in udev_backend.device_list() {
        if Some(device_id) == primary_device_id {
            continue;
        }

        if let Err(err) = DrmNode::from_dev_id(device_id)
            .map_err(DeviceAddError::DrmNode)
            .and_then(|node| state.device_added(node, path))
        {
            error!("Skipping device {device_id}: {err}");
        }
    }
    // If the udev-selected primary GPU could not be initialised (no Mesa
    // driver, display-only device, virtual machine without a GPU, ...), fall
    // back to whichever device did come up with a renderer.
    if state.backend_data.gpus.single_renderer(&primary_gpu).is_err() {
        let fallback = state
            .backend_data
            .backends
            .values()
            .find_map(|backend| backend.render_node)
            .filter(|node| state.backend_data.gpus.single_renderer(node).is_ok());
        match fallback {
            Some(node) => {
                warn!(
                    "primary gpu {primary_gpu} has no usable renderer, using {node} instead"
                );
                primary_gpu = node;
                state.backend_data.primary_gpu = node;
            }
            None => {
                error!(
                    "no usable GPU found: neither hardware acceleration nor Mesa software \
                     rendering could be initialised on any DRM device"
                );
                return;
            }
        }
    }

    state.shm_state.update_formats(
        state
            .backend_data
            .gpus
            .single_renderer(&primary_gpu)
            .unwrap()
            .shm_formats(),
    );

    #[cfg_attr(not(feature = "egl"), allow(unused_mut))]
    let mut renderer = state.backend_data.gpus.single_renderer(&primary_gpu).unwrap();

    #[cfg(feature = "debug")]
    {
        #[allow(deprecated)]
        let fps_image =
            image::io::Reader::with_format(std::io::Cursor::new(FPS_NUMBERS_PNG), image::ImageFormat::Png)
                .decode()
                .unwrap();
        let fps_texture = renderer
            .import_memory(
                &fps_image.to_rgba8(),
                Fourcc::Abgr8888,
                (fps_image.width() as i32, fps_image.height() as i32).into(),
                false,
            )
            .expect("Unable to upload FPS texture");

        for backend in state.backend_data.backends.values_mut() {
            for surface in backend.surfaces.values_mut() {
                surface.fps_element = Some(FpsElement::new(fps_texture.clone()));
            }
        }
        state.backend_data.fps_texture = Some(fps_texture);
    }

    #[cfg(feature = "egl")]
    {
        info!(?primary_gpu, "Trying to initialize EGL Hardware Acceleration",);
        match renderer.bind_wl_display(&display_handle) {
            Ok(_) => info!("EGL hardware-acceleration enabled"),
            Err(err) => info!(?err, "Failed to initialize EGL hardware-acceleration"),
        }
    }

    // init dmabuf support with format list from our primary gpu
    let dmabuf_formats = renderer.dmabuf_formats();
    let default_feedback = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), dmabuf_formats)
        .build()
        .unwrap();
    let mut dmabuf_state = DmabufState::new();
    let global = dmabuf_state
        .create_global_with_default_feedback::<AnvilState<UdevData>>(&display_handle, &default_feedback);
    state.backend_data.dmabuf_state = Some((dmabuf_state, global));

    let gpus = &mut state.backend_data.gpus;
    state
        .backend_data
        .backends
        .iter_mut()
        .for_each(|(node, backend_data)| {
            // Update the per drm surface dmabuf feedback
            backend_data.surfaces.values_mut().for_each(|surface_data| {
                surface_data.dmabuf_feedback = surface_data.dmabuf_feedback.take().or_else(|| {
                    surface_data.drm_output.with_compositor(|compositor| {
                        get_surface_dmabuf_feedback(
                            primary_gpu,
                            surface_data.render_node,
                            *node,
                            gpus,
                            compositor.surface(),
                        )
                    })
                });
            });
        });

    // Expose syncobj protocol if supported by primary GPU
    if let Some(primary_node) = state
        .backend_data
        .primary_gpu
        .node_with_type(NodeType::Primary)
        .and_then(|x| x.ok())
    {
        if let Some(backend) = state.backend_data.backends.get(&primary_node) {
            let import_device = backend.drm_output_manager.device().device_fd().clone();
            if supports_syncobj_eventfd(&import_device) {
                let syncobj_state =
                    DrmSyncobjState::new::<AnvilState<UdevData>>(&display_handle, import_device);
                state.backend_data.syncobj_state = Some(syncobj_state);
            }
        }
    }

    event_loop
        .handle()
        .insert_source(udev_backend, move |event, _, data| match event {
            UdevEvent::Added { device_id, path } => {
                let _phase = stall::enter(Phase::Hotplug);
                if let Err(err) = DrmNode::from_dev_id(device_id)
                    .map_err(DeviceAddError::DrmNode)
                    .and_then(|node| data.device_added(node, &path))
                {
                    error!("Skipping device {device_id}: {err}");
                }
            }
            UdevEvent::Changed { device_id } => {
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.hotplug_changed(node)
                }
            }
            UdevEvent::Removed { device_id } => {
                let _phase = stall::enter(Phase::Hotplug);
                if let Ok(node) = DrmNode::from_dev_id(device_id) {
                    data.device_removed(node)
                }
            }
        })
        .unwrap();

    // An output whose page flip never completes, or whose repaint loop
    // stopped without a word, would otherwise stay frozen until the
    // session ends. Look at every output twice a second.
    event_loop
        .handle()
        .insert_source(Timer::from_duration(WATCHDOG_TICK), move |_, _, data| {
            data.check_outputs_alive();
            TimeoutAction::ToDuration(WATCHDOG_TICK)
        })
        .expect("failed to arm the output watchdog");

    /*
     * Start XWayland if supported
     */
    #[cfg(feature = "xwayland")]
    state.start_xwayland();
    state.run_startup();

    /*
     * And run our loop
     */

    // Every producer is an event source (client sockets, libinput, vblanks,
    // timers, channels), so the loop sleeps until one of them has something;
    // the refresh below then runs once per batch of events, the only time it
    // can have work. A second thread watches the loop turn, and says what the
    // thread is doing when a turn takes a second or more (`watchdog`).
    // Last, so that the renderer's and the driver's own threads -- made
    // while the backend came up -- keep the ordinary priority, and only the
    // loop and what it forks from here inherit this one.
    crate::sched::prioritise_loop();
    state.loop_watch.watch();
    // A panic in any callback used to unwind out of here and end the
    // session: see `recover`.
    let mut recovery = crate::recover::Recovery::default();
    while state.running.load(Ordering::SeqCst) {
        let turn = recovery.turn(|| {
            let result = event_loop.dispatch(None, &mut state);
            state.loop_watch.beat();
            if result.is_err() {
                state.running.store(false, Ordering::SeqCst);
            } else {
                let _phase = stall::enter(Phase::Refresh);
                // The same step the headless backend takes after every turn,
                // which is what the integration tests run.
                state.after_dispatch();
            }
        });
        if turn.is_none() {
            // The turn is lost, and with it whatever it was drawing. Ask
            // every display for a fresh frame so nothing is left waiting on
            // a repaint that panicked half-way through.
            state.loop_watch.beat();
            UdevData::request_repaint(&mut state);
        }
        if recovery.giving_up() {
            error!("the compositor cannot get past a panic and is stopping");
            // Not the 0 of a logout: EX_SOFTWARE tells mindos-session this is
            // a failure, and it starts the compositor again. `_exit` without
            // tearing down, because a state that keeps panicking cannot be
            // trusted to take itself apart, nor the graphics driver's exit
            // handlers to run under it; the kernel lets go of the seat and the
            // displays, as it does after a crash.
            unsafe { libc::_exit(70) };
        }
    }
}

impl DrmLeaseHandler for AnvilState<UdevData> {
    fn drm_lease_state(&mut self, node: DrmNode) -> &mut DrmLeaseState {
        self.backend_data
            .backends
            .get_mut(&node)
            .unwrap()
            .leasing_global
            .as_mut()
            .unwrap()
    }

    fn lease_request(
        &mut self,
        node: DrmNode,
        request: DrmLeaseRequest,
    ) -> Result<DrmLeaseBuilder, LeaseRejected> {
        let backend = self
            .backend_data
            .backends
            .get(&node)
            .ok_or(LeaseRejected::default())?;

        let drm_device = backend.drm_output_manager.device();
        let mut builder = DrmLeaseBuilder::new(drm_device);
        for conn in request.connectors {
            if let Some((_, crtc)) = backend
                .non_desktop_connectors
                .iter()
                .find(|(handle, _)| *handle == conn)
            {
                builder.add_connector(conn);
                builder.add_crtc(*crtc);
                let planes = drm_device.planes(crtc).map_err(LeaseRejected::with_cause)?;
                let (primary_plane, primary_plane_claim) = planes
                    .primary
                    .iter()
                    .find_map(|plane| {
                        drm_device
                            .claim_plane(plane.handle, *crtc)
                            .map(|claim| (plane, claim))
                    })
                    .ok_or_else(LeaseRejected::default)?;
                builder.add_plane(primary_plane.handle, primary_plane_claim);
                if let Some((cursor, claim)) = planes.cursor.iter().find_map(|plane| {
                    drm_device
                        .claim_plane(plane.handle, *crtc)
                        .map(|claim| (plane, claim))
                }) {
                    builder.add_plane(cursor.handle, claim);
                }
            } else {
                tracing::warn!(?conn, "Lease requested for desktop connector, denying request");
                return Err(LeaseRejected::default());
            }
        }

        Ok(builder)
    }

    fn new_active_lease(&mut self, node: DrmNode, lease: DrmLease) {
        // The device can be unplugged between the request and the lease.
        if let Some(backend) = self.backend_data.backends.get_mut(&node) {
            backend.active_leases.push(lease);
        }
    }

    fn lease_destroyed(&mut self, node: DrmNode, lease: u32) {
        if let Some(backend) = self.backend_data.backends.get_mut(&node) {
            backend.active_leases.retain(|l| l.id() != lease);
        }
    }
}

delegate_drm_lease!(AnvilState<UdevData>);

impl DrmSyncobjHandler for AnvilState<UdevData> {
    fn drm_syncobj_state(&mut self) -> Option<&mut DrmSyncobjState> {
        self.backend_data.syncobj_state.as_mut()
    }
}
smithay::delegate_drm_syncobj!(AnvilState<UdevData>);

pub type RenderSurface = GbmBufferedSurface<GbmAllocator<DrmDeviceFd>, Option<OutputPresentationFeedback>>;

pub type GbmDrmCompositor = DrmCompositor<
    GbmAllocator<DrmDeviceFd>,
    GbmDevice<DrmDeviceFd>,
    Option<OutputPresentationFeedback>,
    DrmDeviceFd,
>;

struct SurfaceData {
    dh: DisplayHandle,
    render_node: Option<DrmNode>,
    global: Option<GlobalId>,
    drm_output: DrmOutput<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        Option<OutputPresentationFeedback>,
        DrmDeviceFd,
    >,
    connector: connector::Handle,
    /// Every mode the connector advertises (for the Displays settings).
    modes: Vec<DrmMode>,
    vrr_supported: bool,
    #[cfg(feature = "debug")]
    fps: fps_ticker::Fps,
    #[cfg(feature = "debug")]
    fps_element: Option<FpsElement<MultiTexture>>,
    dmabuf_feedback: Option<SurfaceDmabufFeedback>,
    last_presentation_time: Option<Time<Monotonic>>,
    /// A vblank that arrived early, held back until the frame it reports
    /// is really on screen.
    vblank_throttle_timer: Option<ThrottleTimer>,
    /// What the last repaint aimed at, so one kicked off while the output
    /// is idle aims at the next vblank rather than at "now".
    frame_target: Option<Time<Monotonic>>,
    /// The armed repaint timer, if any. Nothing is armed while a frame is
    /// in flight: the vblank arms the next one.
    repaint_timer: Option<RepaintTimer>,
    /// Something changed since the last repaint began.
    dirty: bool,
    /// The page flip in flight, if any: when it was queued and the deadline
    /// that fires if its vblank never arrives.
    flip: Option<Flip>,
    /// What the frame in flight put straight on the primary plane, if
    /// anything. A buffer the compositor does not own is the kind that can
    /// hold a flip on a fence that never signals, so a flip that times out
    /// with this set costs the output its direct scan-out.
    flip_scanout: Option<RenderElementId>,
    /// When a repaint last ran to completion on this output, whatever it
    /// then decided to do. This is the one thing that must keep happening:
    /// every lit output repaints at least once a second, so a longer gap is
    /// a stopped loop however healthy its timers look.
    last_render: Instant,
    /// How long recent repaints took, which is what decides how late the
    /// next one may start and still make its vblank.
    render_times: crate::timing::RenderTimes,
    /// Frames that reached the screen, and how many of those arrived at
    /// least half a refresh late while the output was drawing continuously.
    /// Late frames are what stutter is made of, so they are counted rather
    /// than guessed at.
    frames: u64,
    late_frames: u64,
    /// How many times a repaint has run on this output, whether or not it
    /// produced a frame. The gap between this and `frames` is work spent
    /// finding out that nothing had changed.
    repaints: u64,
    /// How many times this output has had to be reset.
    resets: u64,
    /// Watchdog ticks in a row that found neither a timer armed nor a frame
    /// in flight. One is a coincidence; two is a dead loop.
    dead_ticks: u8,
    /// The frames the kernel refused in a row, and the recoveries so far,
    /// which pick how hard the next one goes.
    health: Health,
    /// The recovery under way while a helper thread releases the CRTC.
    releasing: Option<Release>,
    /// A recovery waiting out its backoff.
    recovery_timer: Option<RegistrationToken>,
    /// What this output is allowed to put on the planes. Starts at the
    /// configured policy and only ever gets narrower, when scanning out
    /// costs us a frozen output.
    scanout: DirectScanout,
}

impl SurfaceData {
    /// A recovery is waiting or under way: nothing may be drawn or queued
    /// until it is done, and it starts the loop again itself.
    fn recovering(&self) -> bool {
        self.releasing.is_some() || self.recovery_timer.is_some()
    }

    /// Something other than a repaint timer continues the loop: a frame in
    /// flight, a vblank held back to its time, or a recovery.
    fn busy(&self) -> bool {
        self.flip.is_some() || self.vblank_throttle_timer.is_some() || self.recovering()
    }
}

/// The page flip waiting for its vblank.
struct Flip {
    queued_at: Instant,
    /// Fires if the vblank does not arrive. Removed when it does, and `None`
    /// if it could not be armed, which leaves the flip to the watchdog.
    deadline: Option<RegistrationToken>,
}

/// How an output has been doing.
#[derive(Debug, Default)]
struct Health {
    /// When the current run of refused frames began.
    refused_since: Option<Instant>,
    /// Frames refused in a row.
    refused: u32,
    /// Why the last one was refused, so a new reason gets logged.
    last_refusal: Option<Failure>,
    /// When the run was last logged above debug level.
    logged_at: Option<Instant>,
    /// Recoveries since the output last settled; picks the next step.
    recoveries: u8,
    /// When the last recovery started.
    recovered_at: Option<Instant>,
    /// A frame reached the screen since the last recovery started.
    flipped: bool,
}

impl Health {
    /// An output that shows frames `SETTLE` after its last recovery started
    /// is well again: its next incident starts from the gentlest step.
    fn settle(&mut self) {
        if self.flipped && self.recovered_at.is_none_or(|at| at.elapsed() >= recovery::SETTLE) {
            self.recoveries = 0;
            self.recovered_at = None;
        }
    }

    /// The kernel took a frame, or the run of refusals is overtaken by a
    /// recovery.
    fn refusals_over(&mut self) {
        self.refused_since = None;
        self.refused = 0;
        self.last_refusal = None;
        self.logged_at = None;
    }
}

/// A recovery whose CRTCs a helper thread is releasing.
struct Release {
    started: Instant,
    step: Step,
    cause: Cause,
    /// Taking longer than `RELEASE_SLOW` has been logged.
    reported: bool,
}

/// What the helper threads send back to the event loop.
enum Answer {
    /// The kernel let go of the CRTCs of the recovery started at `started`,
    /// for the output on `crtc`.
    Released {
        node: DrmNode,
        crtc: crtc::Handle,
        started: Instant,
        result: Result<Duration, String>,
    },
    /// Every connector of the device was probed, so a scan can read what the
    /// kernel found without waiting on a monitor.
    Probed { node: DrmNode, took: Duration },
}

/// Failures injected on purpose to exercise the recovery paths: the
/// `debug_fault` request, only honoured when `MINDWM_DEBUG_FAULTS` is set.
/// Keys are output names, or `*` for any output.
#[derive(Default)]
struct Faults {
    /// Page flip events still to be dropped.
    lose_vblanks: HashMap<String, u32>,
    /// Frames are refused until then.
    reject_until: HashMap<String, Instant>,
}

impl Faults {
    /// Whether this output's page flip event is to be dropped.
    fn take_vblank(&mut self, output: &str) -> bool {
        for key in [output, "*"] {
            if let Some(left) = self.lose_vblanks.get_mut(key) {
                *left = left.saturating_sub(1);
                if *left == 0 {
                    self.lose_vblanks.remove(key);
                }
                return true;
            }
        }
        false
    }

    /// Whether this output's frames are to be refused.
    fn rejects(&mut self, output: &str) -> bool {
        let now = Instant::now();
        self.reject_until.retain(|_, until| *until > now);
        self.reject_until.contains_key(output) || self.reject_until.contains_key("*")
    }
}

/// A device whose connector queries return what the kernel already knows.
/// Probing a connector reads the monitor's EDID over its cable with the
/// device's mode config lock held, which holds up commits on every display
/// for as long as the monitor takes to answer. The probe thread has just
/// done that, so the scan on the event loop reads its results instead.
struct CachedProbe<'a>(&'a DrmDeviceFd);

impl AsFd for CachedProbe<'_> {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl smithay::reexports::drm::Device for CachedProbe<'_> {}

impl Device for CachedProbe<'_> {
    fn get_connector(&self, handle: connector::Handle, _force_probe: bool) -> io::Result<connector::Info> {
        self.0.get_connector(handle, false)
    }
}

/// A vblank held back because it came in early: the timer that will deliver
/// it, and when. Like a repaint timer, it counts as this output's proof of
/// life only until it is due.
struct ThrottleTimer {
    token: RegistrationToken,
    due: Instant,
}

/// How often the watchdog looks at every output. It is the backstop; the
/// per-flip deadline is what normally catches a stuck output.
const WATCHDOG_TICK: Duration = Duration::from_millis(500);
/// A release still not back after this long gets logged. NVIDIA's driver
/// gives up on a stuck flip after three seconds, others after ten.
const RELEASE_SLOW: Duration = Duration::from_secs(12);
/// How long the change events of one device are gathered before its
/// connectors are probed: a monitor waking up sends several.
const HOTPLUG_SETTLE: Duration = Duration::from_millis(400);
/// How long past its due time an armed timer stops counting as proof that
/// this output is alive. Nothing is ever scheduled further out than
/// `IDLE_REPAINT`, so a timer this late is not late: it is not coming.
const REPAINT_OVERDUE: Duration = Duration::from_secs(1);
/// How long an output may go without completing a single repaint before it
/// is taken to be stuck. The idle tick is a second, so a lit output that has
/// not drawn in this long is not idle: it has stopped.
const STALL_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a DRM device may keep refusing frames while the session is
/// running before the resume is tried again.
const REACTIVATE_AFTER: Duration = Duration::from_secs(2);
/// A change is drawn at once when the repaint already armed is further out
/// than this. A frame is milliseconds; anything past this is not the repaint
/// loop running, it is an output that has been parked.
const PULL_FORWARD_AFTER: Duration = Duration::from_millis(200);

/// A timer that will repaint one CRTC: the one 0.6 frames after a flip,
/// the retry a frame after an empty repaint, or the slow idle tick.
struct RepaintTimer {
    token: RegistrationToken,
    /// When it fires. An output whose timer is long past due has stopped,
    /// however healthy the timer itself looks.
    due: Instant,
    /// The presentation time the repaint aims at; `None` for the idle tick,
    /// which works it out when it fires.
    target: Option<Time<Monotonic>>,
    /// The idle tick of an output with nothing to draw. A change replaces
    /// it with an immediate repaint instead of waiting for it.
    idle: bool,
}

/// How often an output with nothing to draw looks again. Frame callbacks of
/// surfaces that are not on screen (covered, or on another output) are only
/// sent when they are this overdue, and a change that slipped past
/// `request_repaint` gets drawn at the latest by the next tick.
const IDLE_REPAINT: Duration = Duration::from_secs(1);

/// Arm `timer` to repaint `crtc`, replacing whatever was armed before, and
/// remember it so a fullscreen commit or a change while idle can pull the
/// repaint forward.
fn arm_repaint(
    handle: &LoopHandle<'static, AnvilState<UdevData>>,
    surface: &mut SurfaceData,
    node: DrmNode,
    crtc: crtc::Handle,
    delay: Duration,
    target: Option<Time<Monotonic>>,
    idle: bool,
) {
    if let Some(armed) = surface.repaint_timer.take() {
        handle.remove(armed.token);
    }
    // Never park an output. The delay is worked out from the last
    // presentation time, and a driver that reports one from the future makes
    // it arbitrarily long: the output then sits there with a timer armed,
    // looking alive to everything that asks, and never draws again. A frame
    // is milliseconds and the idle tick is a second, so a longer delay is a
    // mistake by definition. Drawing a second late is recoverable; not
    // drawing at all is what this whole loop exists to prevent.
    let delay = delay.min(IDLE_REPAINT);
    let due = Instant::now() + delay;
    let token = handle.insert_source(Timer::from_duration(delay), move |_, _, data| {
        if let Some(surface) = data
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|device| device.surfaces.get_mut(&crtc))
        {
            surface.repaint_timer = None;
        }
        let target = target.unwrap_or_else(|| data.repaint_target(node, crtc));
        data.render(node, Some(crtc), target);
        TimeoutAction::Drop
    });
    match token {
        Ok(token) => surface.repaint_timer = Some(RepaintTimer { token, due, target, idle }),
        Err(err) => {
            // One output has no way to ask for its next frame. The watchdog
            // finds it within a second and starts it again; taking the whole
            // session down over it would be worse.
            error!(?crtc, ?err, "cannot arm the repaint timer for this output");
            surface.repaint_timer = None;
        }
    }
}

/// How far from the clock a reported vblank time may be and still be
/// believed: a little ahead, because the clock is read after the event, and
/// a whole second behind, because a queued-up event can be that old.
const VBLANK_TIME_AHEAD: Duration = Duration::from_millis(50);
const VBLANK_TIME_BEHIND: Duration = Duration::from_secs(1);

/// The presentation time the driver reported, unless it is nowhere near now.
///
/// This one timestamp times everything that follows: each frame is aimed one
/// refresh after the last target, and the next after that one, so a single
/// reading from the future is not a late frame, it is a schedule that runs
/// away and an output that never draws again. The clock is a worse
/// presentation time than the hardware's and a far better one to plan from.
fn sane_vblank_time(tp: Duration, now: Duration) -> Option<Duration> {
    if tp <= now.saturating_add(VBLANK_TIME_AHEAD) && tp.saturating_add(VBLANK_TIME_BEHIND) >= now {
        return Some(tp);
    }
    static WARN_ONCE: Once = Once::new();
    WARN_ONCE.call_once(|| {
        warn!("the driver reports vblank times that are not near the clock; timing the outputs from the clock instead")
    });
    None
}

/// Why a lit output with nothing in flight has stopped, or `None` if it is
/// still going.
///
/// `since_render` is how long ago a repaint last ran to completion, and
/// `overdue` how long ago the most distant of its armed timers was due, or
/// `None` when it has none armed at all.
fn stopped_reason(since_render: Duration, overdue: Option<Duration>) -> Option<String> {
    if since_render > STALL_TIMEOUT {
        return Some(format!("nothing has been drawn for {since_render:?}"));
    }
    match overdue {
        None => Some("no timer armed and no frame in flight".to_string()),
        Some(late) if late > REPAINT_OVERDUE => Some(format!("its repaint timer is {late:?} overdue")),
        Some(_) => None,
    }
}

/// Start the clock on the page flip just queued for `crtc`. If its vblank
/// does not arrive before the deadline, the output is reset rather than left
/// waiting for an event that is not coming.
///
/// A loop that was stuck past the deadline does not reset a display that
/// flipped in time: calloop hands out the file descriptors that are ready
/// before the timers that expired in the same turn, so the vblank waiting on
/// the DRM device is read, and the flip forgotten, before this timer fires.
fn arm_flip_deadline(
    handle: &LoopHandle<'static, AnvilState<UdevData>>,
    surface: &mut SurfaceData,
    node: DrmNode,
    crtc: crtc::Handle,
    refresh: Option<i32>,
) {
    if let Some(deadline) = surface.flip.take().and_then(|flip| flip.deadline) {
        handle.remove(deadline);
    }
    let queued_at = Instant::now();
    let deadline = handle
        .insert_source(
            Timer::from_duration(crate::timing::flip_deadline(refresh)),
            move |_, _, data| {
                data.flip_timed_out(node, crtc, queued_at);
                TimeoutAction::Drop
            },
        )
        .inspect_err(|err| {
            // The watchdog still sees the flip, a little later.
            warn!(?crtc, ?err, "cannot arm the page flip deadline");
        })
        .ok();
    surface.flip = Some(Flip { queued_at, deadline });
}

/// Remove every timer that would act on this output: the repaint, a vblank
/// held back, a recovery waiting out its backoff, and the deadline of the
/// flip in flight. The flip itself stays recorded until the caller drops it.
fn cancel_timers(handle: &LoopHandle<'static, AnvilState<UdevData>>, surface: &mut SurfaceData) {
    if let Some(armed) = surface.repaint_timer.take() {
        handle.remove(armed.token);
    }
    if let Some(throttle) = surface.vblank_throttle_timer.take() {
        handle.remove(throttle.token);
    }
    if let Some(token) = surface.recovery_timer.take() {
        handle.remove(token);
    }
    if let Some(token) = surface.flip.as_mut().and_then(|flip| flip.deadline.take()) {
        handle.remove(token);
    }
}

/// A modeset elsewhere on the device dropped the frame this output had in
/// flight, and its event is not coming. Forget it and draw again from
/// scratch, rather than let its deadline reset a display nothing is wrong
/// with. An output being reset is left to the reset, which does the same.
fn strand_frame(handle: &LoopHandle<'static, AnvilState<UdevData>>, surface: &mut SurfaceData) {
    if surface.recovering() {
        return;
    }
    let held = surface.flip.is_some() || surface.vblank_throttle_timer.is_some();
    if let Some(throttle) = surface.vblank_throttle_timer.take() {
        handle.remove(throttle.token);
    }
    if let Some(token) = surface.flip.take().and_then(|flip| flip.deadline) {
        handle.remove(token);
    }
    if held {
        forget_frame(surface);
    }
    surface.flip_scanout = None;
    surface.last_presentation_time = None;
    surface.frame_target = None;
    surface.last_render = Instant::now();
    surface.dead_ticks = 0;
    surface.dirty = true;
}

/// Give up on the frame the compositor holds for a page flip whose event is
/// not coming, or came for a CRTC that has since been released. Until it is
/// handed back no new frame can be queued, and the clients waiting on it are
/// told it was never shown.
fn forget_frame(surface: &mut SurfaceData) {
    for _ in 0..2 {
        match surface.drm_output.frame_submitted() {
            Ok(Some(Some(mut feedback))) => feedback.discarded(),
            Ok(Some(None)) => {}
            Ok(None) | Err(_) => break,
        }
    }
}

/// Switch one display off for the idle blank: its planes and CRTC off, and
/// its buffers let go of, since nothing shows them any more.
fn blank_surface(surface: &mut SurfaceData) {
    match surface.drm_output.with_compositor(|compositor| compositor.clear()) {
        Ok(()) => surface.drm_output.reset_buffers(),
        Err(err) => warn!(?err, "cannot switch a display off"),
    }
}

/// Ask every connector of the device what is plugged in, the slow way: the
/// kernel reads each monitor's EDID again. Runs on the probe thread.
fn probe_all(fd: &DrmDeviceFd) -> io::Result<()> {
    for connector in fd.resource_handles()?.connectors() {
        if let Err(err) = fd.get_connector(*connector, true) {
            debug!(?connector, %err, "cannot probe a connector");
        }
    }
    Ok(())
}

/// Make the kernel let go of these CRTCs. Runs on a helper thread, because it
/// blocks: a blocking commit waits for any page flip still queued on a CRTC,
/// and NVIDIA's driver gives up on a stuck one only after three seconds, on
/// every CRTC, then drops it. With `off`, the CRTCs are also switched off.
/// Without, the commit sets nothing new and queues no flip of its own.
fn release_crtcs(fd: &DrmDeviceFd, crtcs: &[crtc::Handle], off: bool) -> Result<Duration, String> {
    let started = Instant::now();
    let mut req = AtomicModeReq::new();
    let mut any = false;
    for crtc in crtcs {
        let props = fd
            .get_properties(*crtc)
            .map_err(|err| format!("cannot read the properties of {crtc:?}: {err}"))?;
        let (handles, values) = props.as_props_and_values();
        let active = handles.iter().zip(values).find(|(handle, _)| {
            fd.get_property(**handle)
                .is_ok_and(|info| info.name().to_bytes() == b"ACTIVE")
        });
        // No ACTIVE property: the device is not driven atomically, and has
        // no queue of flips to wait out.
        let Some((prop, value)) = active else {
            continue;
        };
        if off && *value == 0 {
            continue;
        }
        req.add_raw_property((*crtc).into(), *prop, if off { 0 } else { *value });
        any = true;
    }
    if any {
        let flags = if off {
            AtomicCommitFlags::ALLOW_MODESET
        } else {
            AtomicCommitFlags::empty()
        };
        fd.atomic_commit(flags, req)
            .map_err(|err| format!("the kernel did not release {crtcs:?}: {err}"))?;
    }
    Ok(started.elapsed())
}

impl Drop for SurfaceData {
    fn drop(&mut self) {
        if let Some(global) = self.global.take() {
            self.dh.remove_global::<AnvilState<UdevData>>(global);
        }
    }
}

/// A connector switched off in the Displays settings (or by the preferences
/// file at start-up), kept so it can be switched back on.
struct DisabledConnector {
    connector: connector::Info,
    crtc: crtc::Handle,
    name: String,
    make: String,
    model: String,
}

struct BackendData {
    surfaces: HashMap<crtc::Handle, SurfaceData>,
    non_desktop_connectors: Vec<(connector::Handle, crtc::Handle)>,
    disabled: Vec<DisabledConnector>,
    leasing_global: Option<DrmLeaseState>,
    active_leases: Vec<DrmLease>,
    drm_output_manager: DrmOutputManager<
        GbmAllocator<DrmDeviceFd>,
        GbmFramebufferExporter<DrmDeviceFd>,
        Option<OutputPresentationFeedback>,
        DrmDeviceFd,
    >,
    drm_scanner: DrmScanner,
    render_node: Option<DrmNode>,
    registration_token: RegistrationToken,
    /// The probe thread is reading this device's connectors.
    probing: bool,
    /// A change came in while probing or resetting a display: scan again
    /// once that is over.
    rescan_pending: bool,
    /// A modeset changed this device out from under its surfaces, and the
    /// frames they had in flight went with it. Set from the places that have
    /// no way to reach the event loop and cleared by the repaint that
    /// follows, which drops the flips and starts every display again.
    reconfigured: bool,
    /// Since when this device has been handing back `DeviceInactive`. A
    /// paused session is normal and brief; a device that stays inactive
    /// while the session is running is a resume that did not take, and
    /// every display on it is frozen until somebody tries again.
    inactive_since: Option<Instant>,
}

#[derive(Debug, thiserror::Error)]
enum DeviceAddError {
    #[error("Failed to create a renderer for the device: {0}")]
    NoRenderer(String),
    #[error("Failed to open device using libseat: {0}")]
    DeviceOpen(libseat::Error),
    #[error("Failed to initialize drm device: {0}")]
    DrmDevice(DrmError),
    #[error("Failed to initialize gbm device: {0}")]
    GbmDevice(std::io::Error),
    #[error("Failed to access drm node: {0}")]
    DrmNode(CreateDrmNodeError),
    #[error("Failed to add device to GpuManager: {0}")]
    AddNode(egl::Error),
    #[error("Failed to watch the device for vblanks: {0}")]
    Watch(String),
    #[error("The device has no render node")]
    NoRenderNode,
    #[error("Primary GPU is missing")]
    PrimaryGpuMissing,
}

fn get_surface_dmabuf_feedback(
    primary_gpu: DrmNode,
    render_node: Option<DrmNode>,
    scanout_node: DrmNode,
    gpus: &mut GpuManager<GbmGlesBackend<GlesRenderer, DrmDeviceFd>>,
    surface: &DrmSurface,
) -> Option<SurfaceDmabufFeedback> {
    let primary_formats = gpus.single_renderer(&primary_gpu).ok()?.dmabuf_formats();
    let render_formats = if let Some(render_node) = render_node {
        gpus.single_renderer(&render_node).ok()?.dmabuf_formats()
    } else {
        FormatSet::default()
    };

    let all_render_formats = primary_formats
        .iter()
        .chain(render_formats.iter())
        .copied()
        .collect::<FormatSet>();

    let planes = surface.planes().clone();

    // We limit the scan-out tranche to formats we can also render from
    // so that there is always a fallback render path available in case
    // the supplied buffer can not be scanned out directly
    let planes_formats = surface
        .plane_info()
        .formats
        .iter()
        .copied()
        .chain(planes.overlay.into_iter().flat_map(|p| p.formats))
        .collect::<FormatSet>()
        .intersection(&all_render_formats)
        .copied()
        .collect::<FormatSet>();

    let builder = DmabufFeedbackBuilder::new(primary_gpu.dev_id(), primary_formats);
    let render_feedback = if let Some(render_node) = render_node {
        builder
            .clone()
            .add_preference_tranche(render_node.dev_id(), None, render_formats.clone())
            .build()
            .ok()?
    } else {
        builder.clone().build().ok()?
    };

    let scanout_feedback = builder
        .add_preference_tranche(
            surface.device_fd().dev_id().ok()?,
            Some(zwp_linux_dmabuf_feedback_v1::TrancheFlags::Scanout),
            planes_formats,
        )
        .add_preference_tranche(scanout_node.dev_id(), None, render_formats)
        .build()
        .ok()?;

    Some(SurfaceDmabufFeedback {
        render_feedback,
        scanout_feedback,
    })
}

impl AnvilState<UdevData> {
    fn device_added(&mut self, node: DrmNode, path: &Path) -> Result<(), DeviceAddError> {
        // Try to open the device
        let fd = self
            .backend_data
            .session
            .open(
                path,
                OFlags::RDWR | OFlags::CLOEXEC | OFlags::NOCTTY | OFlags::NONBLOCK,
            )
            .map_err(DeviceAddError::DeviceOpen)?;

        let fd = DrmDeviceFd::new(DeviceFd::from(fd));

        let (drm, notifier) = DrmDevice::new(fd.clone(), true).map_err(DeviceAddError::DrmDevice)?;
        let gbm = GbmDevice::new(fd).map_err(DeviceAddError::GbmDevice)?;

        let registration_token = self
            .handle
            .insert_source(
                notifier,
                move |event, metadata, data: &mut AnvilState<_>| match event {
                    DrmEvent::VBlank(crtc) => {
                        profiling::scope!("vblank", &format!("{crtc:?}"));
                        data.frame_finish(node, crtc, metadata, false);
                    }
                    DrmEvent::Error(error) => {
                        error!("{:?}", error);
                    }
                },
            )
            .map_err(|err| DeviceAddError::Watch(err.error.to_string()))?;

        let primary_gpu = self.backend_data.primary_gpu;
        let own_render_node = node.node_with_type(NodeType::Render).and_then(|n| n.ok());
        let is_primary_device = node == primary_gpu
            || own_render_node == Some(primary_gpu)
            || primary_gpu
                .node_with_type(NodeType::Primary)
                .and_then(|n| n.ok())
                == Some(node);

        let mut try_initialize_gpu = || {
            let display = unsafe { EGLDisplay::new(gbm.clone()).map_err(DeviceAddError::AddNode)? };
            let egl_device = EGLDevice::device_for_display(&display).map_err(DeviceAddError::AddNode)?;
            if is_primary_device { self.backend_data.software_rendering = egl_device.is_software(); }

            if egl_device.is_software() {
                // Display-only devices (no Mesa driver) render on the primary
                // GPU and get their frames copied over. The primary device
                // itself, however, is allowed to fall back to Mesa's software
                // renderer so that MindOS still comes up in virtual machines
                // and on GPUs without a working driver.
                if !is_primary_device {
                    return Err(DeviceAddError::NoRenderNode);
                }
                warn!(
                    %node,
                    "no hardware-accelerated EGL device, rendering with Mesa software rasterizer"
                );
            }

            let render_node = egl_device
                .try_get_render_node()
                .ok()
                .flatten()
                .or(own_render_node)
                .unwrap_or(node);
            info!(%node, %render_node, software = egl_device.is_software(), "initialized gpu");
            self.backend_data
                .gpus
                .as_mut()
                .add_node(render_node, gbm.clone())
                .map_err(DeviceAddError::AddNode)?;

            std::result::Result::<DrmNode, DeviceAddError>::Ok(render_node)
        };

        let render_node = try_initialize_gpu()
            .inspect_err(|err| {
                warn!(?err, "failed to initialize gpu");
            })
            .ok();

        let allocator = render_node
            .is_some()
            .then(|| GbmAllocator::new(gbm.clone(), GbmBufferFlags::RENDERING | GbmBufferFlags::SCANOUT))
            .or_else(|| {
                self.backend_data
                    .backends
                    .get(&self.backend_data.primary_gpu)
                    .or_else(|| {
                        self.backend_data
                            .backends
                            .values()
                            .find(|backend| backend.render_node == Some(self.backend_data.primary_gpu))
                    })
                    .map(|backend| backend.drm_output_manager.allocator().clone())
            })
            .ok_or(DeviceAddError::PrimaryGpuMissing)?;

        let framebuffer_exporter = GbmFramebufferExporter::new(gbm.clone(), render_node);

        let color_formats = if std::env::var("ANVIL_DISABLE_10BIT").is_ok() {
            SUPPORTED_FORMATS_8BIT_ONLY
        } else {
            SUPPORTED_FORMATS
        };
        let mut renderer = self
            .backend_data
            .gpus
            .single_renderer(&render_node.unwrap_or(self.backend_data.primary_gpu))
            .map_err(|err| DeviceAddError::NoRenderer(err.to_string()))?;
        let render_formats = renderer
            .as_mut()
            .egl_context()
            .dmabuf_render_formats()
            .iter()
            .filter(|format| render_node.is_some() || format.modifier == Modifier::Linear)
            .copied()
            .collect::<FormatSet>();

        let drm_output_manager = DrmOutputManager::new(
            drm,
            allocator,
            framebuffer_exporter,
            Some(gbm),
            color_formats.iter().copied(),
            render_formats,
        );

        self.backend_data.backends.insert(
            node,
            BackendData {
                registration_token,
                reconfigured: false,
                drm_output_manager,
                inactive_since: None,
                drm_scanner: DrmScanner::new(),
                non_desktop_connectors: Vec::new(),
                disabled: Vec::new(),
                render_node,
                surfaces: HashMap::new(),
                leasing_global: DrmLeaseState::new::<AnvilState<UdevData>>(&self.display_handle, &node)
                    .inspect_err(|err| {
                        warn!(?err, "Failed to initialize drm lease global for: {}", node);
                    })
                    .ok(),
                active_leases: Vec::new(),
                probing: false,
                rescan_pending: false,
            },
        );

        self.device_changed(node, false);

        Ok(())
    }

    fn connector_connected(&mut self, node: DrmNode, connector: connector::Info, crtc: crtc::Handle) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let render_node = device.render_node.unwrap_or(self.backend_data.primary_gpu);
        let mut renderer = match self.backend_data.gpus.single_renderer(&render_node) {
            Ok(renderer) => renderer,
            Err(err) => {
                // Plugging a display in must never take the session down with
                // the displays that are already working.
                error!(?crtc, %err, "no renderer for the GPU this connector is on");
                return;
            }
        };

        let output_name = format!("{}-{}", connector.interface().as_str(), connector.interface_id());
        info!(?crtc, "Trying to setup connector {}", output_name,);

        let drm_device = device.drm_output_manager.device();

        let non_desktop = drm_device
            .get_properties(connector.handle())
            .ok()
            .and_then(|props| {
                let (info, value) = props
                    .into_iter()
                    .filter_map(|(handle, value)| {
                        let info = drm_device.get_property(handle).ok()?;

                        Some((info, value))
                    })
                    .find(|(info, _)| info.name().to_str() == Ok("non-desktop"))?;

                info.value_type().convert_value(value).as_boolean()
            })
            .unwrap_or(false);

        let display_info = edid::for_connector(drm_device, connector.handle());

        let make = display_info
            .as_ref()
            .map(|info| info.make.clone())
            .unwrap_or_else(|| "Unknown".into());

        let model = display_info
            .as_ref()
            .map(|info| info.model.clone())
            .unwrap_or_else(|| "Unknown".into());

        if non_desktop {
            info!("Connector {} is non-desktop, setting up for leasing", output_name);
            device.non_desktop_connectors.push((connector.handle(), crtc));
            if let Some(lease_state) = device.leasing_global.as_mut() {
                lease_state.add_connector::<AnvilState<UdevData>>(
                    connector.handle(),
                    output_name,
                    format!("{} {}", make, model),
                );
            }
        } else {
            let prefs = self.prefs.outputs.get(&output_name).cloned().unwrap_or_default();
            if prefs.enabled == Some(false) {
                info!("Connector {} is switched off by the user", output_name);
                device.disabled.push(DisabledConnector {
                    connector: connector.clone(),
                    crtc,
                    name: output_name,
                    make,
                    model,
                });
                return;
            }

            let modes: Vec<DrmMode> = connector.modes().to_vec();
            if modes.is_empty() {
                // A monitor that is still waking up can report itself
                // connected before the driver has a single mode for it.
                warn!(
                    "Connector {} is connected but reports no modes; it stays dark until it is plugged in again",
                    output_name
                );
                return;
            }
            let preferred_id = modes
                .iter()
                .position(|mode| mode.mode_type().contains(ModeTypeFlags::PREFERRED))
                .unwrap_or(0);
            let mode_id = prefs
                .mode
                .as_deref()
                .and_then(crate::prefs::parse_mode_key)
                .and_then(|(w, h, refresh)| {
                    modes.iter().position(|m| {
                        let wl = WlMode::from(*m);
                        wl.size.w == w && wl.size.h == h && wl.refresh == refresh
                    })
                })
                .unwrap_or(preferred_id);

            let drm_mode = modes[mode_id];
            let wl_mode = WlMode::from(drm_mode);

            let (phys_w, phys_h) = connector.size().unwrap_or((0, 0));
            let output = Output::new(
                output_name,
                PhysicalProperties {
                    size: (phys_w as i32, phys_h as i32).into(),
                    subpixel: connector.subpixel().into(),
                    make,
                    model,
                },
            );
            let global = output.create_global::<AnvilState<UdevData>>(&self.display_handle);

            let x = self
                .space
                .outputs()
                .fold(0, |acc, o| {
                    acc + self.space.output_geometry(o).map(|g| g.size.w).unwrap_or(0)
                });
            let position: Point<i32, Logical> = prefs
                .position
                .map(|[x, y]| (x, y).into())
                .unwrap_or_else(|| (x, 0).into());
            let scale = prefs.scale.filter(|s| (0.5..=4.0).contains(s)).map(smithay::output::Scale::Fractional);
            let transform = prefs.transform.as_deref().and_then(crate::state::parse_transform);

            for mode in &modes {
                output.add_mode(WlMode::from(*mode));
            }
            output.set_preferred(WlMode::from(modes[preferred_id]));
            output.change_current_state(Some(wl_mode), transform, scale, Some(position));
            self.space.map_output(&output, position);

            output.user_data().insert_if_missing(|| UdevOutputId {
                crtc,
                device_id: node,
            });

            #[cfg(feature = "debug")]
            let fps_element = self.backend_data.fps_texture.clone().map(FpsElement::new);

            let driver = match drm_device.get_driver() {
                Ok(driver) => driver,
                Err(err) => {
                    warn!("Failed to query drm driver: {}", err);
                    return;
                }
            };

            let mut planes = match drm_device.planes(&crtc) {
                Ok(planes) => planes,
                Err(err) => {
                    warn!("Failed to query crtc planes: {}", err);
                    return;
                }
            };

            // Using an overlay plane on a nvidia card breaks
            if driver.name().to_string_lossy().to_lowercase().contains("nvidia")
                || driver
                    .description()
                    .to_string_lossy()
                    .to_lowercase()
                    .contains("nvidia")
            {
                planes.overlay = vec![];
            }

            let drm_output = match device
                .drm_output_manager
                .initialize_output::<_, OutputRenderElements<UdevRenderer<'_>, WindowRenderElement<UdevRenderer<'_>>>>(
                    crtc,
                    drm_mode,
                    &[connector.handle()],
                    &output,
                    Some(planes),
                    &mut renderer,
                    &DrmOutputRenderElements::default(),
                ) {
                Ok(drm_output) => drm_output,
                Err(err) => {
                    warn!("Failed to initialize drm output: {}", err);
                    return;
                }
            };

            // The environment wins over the config, so a machine that will not
            // behave can be booted into a safe compositor without a rebuild.
            let scanout = if std::env::var("MINDWM_DISABLE_DIRECT_SCANOUT").is_ok()
                || std::env::var("ANVIL_DISABLE_DIRECT_SCANOUT").is_ok()
            {
                DirectScanout::Off
            } else {
                self.config.graphics.direct_scanout
            };

            let vrr_supported = drm_output.with_compositor(|compositor| {
                matches!(
                    compositor.vrr_supported(connector.handle()),
                    Ok(VrrSupport::Supported) | Ok(VrrSupport::RequiresModeset)
                )
            });
            if let Some(vrr) = prefs.vrr {
                if vrr_supported {
                    if let Err(err) = drm_output.with_compositor(|compositor| compositor.use_vrr(vrr)) {
                        warn!(output = output.name(), %err, "cannot apply the VRR preference");
                    }
                } else if vrr {
                    info!(output = output.name(), "VRR preference ignored: not supported");
                }
            }

            let dmabuf_feedback = drm_output.with_compositor(|compositor| {
                compositor.set_debug_flags(self.backend_data.debug_flags);

                get_surface_dmabuf_feedback(
                    self.backend_data.primary_gpu,
                    device.render_node,
                    node,
                    &mut self.backend_data.gpus,
                    compositor.surface(),
                )
            });

            let surface = SurfaceData {
                dh: self.display_handle.clone(),
                render_node: device.render_node,
                global: Some(global),
                drm_output,
                connector: connector.handle(),
                modes,
                vrr_supported,
                #[cfg(feature = "debug")]
                fps: fps_ticker::Fps::default(),
                #[cfg(feature = "debug")]
                fps_element,
                dmabuf_feedback,
                last_presentation_time: None,
                vblank_throttle_timer: None,
                frame_target: None,
                repaint_timer: None,
                last_render: Instant::now(),
                render_times: crate::timing::RenderTimes::default(),
                frames: 0,
                late_frames: 0,
                repaints: 0,
                resets: 0,
                dirty: true,
                flip: None,
                flip_scanout: None,
                dead_ticks: 0,
                health: Health::default(),
                releasing: None,
                recovery_timer: None,
                scanout,
            };

            device.surfaces.insert(crtc, surface);

            // Adding an output can re-allocate the whole device: the manager
            // looks for a buffer format every surface on it can share, which
            // modesets the displays that were already running and strands
            // whatever each of them had in flight. Their deadlines would
            // catch that a second later, as a freeze long enough to see.
            for (other, surface) in device.surfaces.iter_mut() {
                if *other != crtc {
                    strand_frame(&self.handle, surface);
                }
            }

            // kick-off rendering
            self.handle.insert_idle(move |state| {
                state.render(node, None, state.clock.now());
            });
        }
    }

    fn connector_disconnected(&mut self, node: DrmNode, connector: connector::Info, crtc: crtc::Handle) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };
        device.disabled.retain(|d| d.connector.handle() != connector.handle());

        if let Some(pos) = device
            .non_desktop_connectors
            .iter()
            .position(|(handle, _)| *handle == connector.handle())
        {
            let _ = device.non_desktop_connectors.remove(pos);
            if let Some(leasing_state) = device.leasing_global.as_mut() {
                leasing_state.withdraw_connector(connector.handle());
            }
        } else {
            if let Some(mut surface) = device.surfaces.remove(&crtc) {
                // Its timers would fire for a CRTC that no longer draws. A
                // release under way finishes on its own and finds nothing.
                cancel_timers(&self.handle, &mut surface);
            }

            let output = self
                .space
                .outputs()
                .find(|o| {
                    o.user_data()
                        .get::<UdevOutputId>()
                        .map(|id| id.device_id == node && id.crtc == crtc)
                        .unwrap_or(false)
                })
                .cloned();

            if let Some(output) = output {
                self.space.unmap_output(&output);
                if self.mindbar.output.as_deref() == Some(&output.name()) {
                    self.mindbar.close();
                    self.mindbar.output = None;
                }
            }
        }

        let render_node = device.render_node.unwrap_or(self.backend_data.primary_gpu);
        let Ok(mut renderer) = self.backend_data.gpus.single_renderer(&render_node) else {
            // Unplugging a display must never take the session down with the
            // displays that are still plugged in.
            error!(?crtc, "no renderer for this GPU while a connector went away");
            return;
        };
        let _ = device.drm_output_manager.try_to_restore_modifiers::<_, OutputRenderElements<
            UdevRenderer<'_>,
            WindowRenderElement<UdevRenderer<'_>>,
        >>(
            &mut renderer,
            // FIXME: For a flicker free operation we should return the actual elements for this output..
            // Instead we just use black to "simulate" a modeset :)
            &DrmOutputRenderElements::default(),
        );
        // Restoring the modifiers modesets every surface on the device, which
        // strands whatever frame each of the *other* displays had in flight.
        // Their deadlines would catch it a second later; drawing them again
        // now is a hitch nobody sees instead.
        for surface in device.surfaces.values_mut() {
            strand_frame(&self.handle, surface);
        }
        if !device.surfaces.is_empty() {
            self.handle
                .insert_idle(move |data| data.render(node, None, data.clock.now()));
        }
    }

    /// Look at what is plugged into the device and light or drop outputs to
    /// match. With `cached`, the probe thread has just asked every monitor,
    /// and the scan reads what the kernel found instead of asking again.
    fn device_changed(&mut self, node: DrmNode, cached: bool) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let drm_device = device.drm_output_manager.device();
        let scan = if cached {
            device
                .drm_scanner
                .scan_connectors(&CachedProbe(drm_device.device_fd()))
        } else {
            device.drm_scanner.scan_connectors(drm_device)
        };
        let scan_result = match scan {
            Ok(scan_result) => scan_result,
            Err(err) => {
                tracing::warn!(?err, "Failed to scan connectors");
                return;
            }
        };

        for event in scan_result {
            match event {
                DrmScanEvent::Connected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    self.connector_connected(node, connector, crtc);
                }
                DrmScanEvent::Disconnected {
                    connector,
                    crtc: Some(crtc),
                } => {
                    self.connector_disconnected(node, connector, crtc);
                }
                _ => {}
            }
        }

        // fixup window coordinates
        crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
        self.relayout_all_outputs();
    }

    /// udev says something about the device changed: usually a monitor
    /// plugged, unplugged, woken or put to sleep. Such events come in bursts,
    /// so they are gathered for a moment and handled once.
    fn hotplug_changed(&mut self, node: DrmNode) {
        if !self.backend_data.hotplug.contains(&node) {
            self.backend_data.hotplug.push(node);
        }
        if self.backend_data.hotplug_timer.is_some() {
            return;
        }
        let token = self
            .handle
            .insert_source(Timer::from_duration(HOTPLUG_SETTLE), |_, _, data| {
                data.backend_data.hotplug_timer = None;
                data.probe_connectors();
                TimeoutAction::Drop
            });
        match token {
            Ok(token) => self.backend_data.hotplug_timer = Some(token),
            Err(err) => {
                warn!(?err, "cannot gather display changes; looking at them now");
                self.probe_connectors();
            }
        }
    }

    /// Probe the connectors of every device with changes waiting, on a thread
    /// of its own: a monitor can take a long time to answer, and the event
    /// loop would be frozen for every moment of it.
    fn probe_connectors(&mut self) {
        let _phase = stall::enter(Phase::Hotplug);
        for node in std::mem::take(&mut self.backend_data.hotplug) {
            let Some(device) = self.backend_data.backends.get_mut(&node) else {
                continue;
            };
            if device.probing {
                device.rescan_pending = true;
                continue;
            }
            device.probing = true;
            let fd = device.drm_output_manager.device().device_fd().clone();
            let answers = self.backend_data.answers.clone();
            let spawned = std::thread::Builder::new()
                .name("mindwm-probe".into())
                .spawn(move || {
                    let started = Instant::now();
                    if let Err(err) = probe_all(&fd) {
                        debug!(%err, "cannot list the connectors to probe");
                    }
                    let _ = answers.send(Answer::Probed {
                        node,
                        took: started.elapsed(),
                    });
                });
            if let Err(err) = spawned {
                warn!(%err, "no thread to probe the displays on; probing them on the event loop");
                if let Some(device) = self.backend_data.backends.get_mut(&node) {
                    device.probing = false;
                }
                self.device_changed(node, false);
            }
        }
    }

    /// The probe thread is done with a device: light and drop outputs to
    /// match what it found.
    fn probe_done(&mut self, node: DrmNode, took: Duration) {
        let _phase = stall::enter(Phase::Hotplug);
        let Some(device) = self.backend_data.backends.get_mut(&node) else {
            return;
        };
        device.probing = false;
        if took >= Duration::from_millis(500) {
            info!(%node, ?took, "probing the connected displays took a while");
        } else {
            debug!(%node, ?took, "probed the connected displays");
        }
        if device.surfaces.values().any(|surface| surface.releasing.is_some()) {
            // Lighting or dropping an output commits on the device, which
            // would wait on the CRTC a helper thread is releasing. The reset
            // scans again when it is done.
            device.rescan_pending = true;
            return;
        }
        let again = std::mem::take(&mut device.rescan_pending);
        self.device_changed(node, true);
        if again {
            self.hotplug_changed(node);
        }
    }

    fn device_removed(&mut self, node: DrmNode) {
        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            return;
        };

        let crtcs: Vec<_> = device
            .drm_scanner
            .crtcs()
            .map(|(info, crtc)| (info.clone(), crtc))
            .collect();

        for (connector, crtc) in crtcs {
            self.connector_disconnected(node, connector, crtc);
        }

        debug!("Surfaces dropped");

        // drop the backends on this side
        if let Some(mut backend_data) = self.backend_data.backends.remove(&node) {
            if let Some(mut leasing_global) = backend_data.leasing_global.take() {
                leasing_global.disable_global::<AnvilState<UdevData>>();
            }

            if let Some(render_node) = backend_data.render_node {
                self.backend_data.gpus.as_mut().remove_node(&render_node);
            }

            self.handle.remove(backend_data.registration_token);

            debug!("Dropping device");
        }

        crate::shell::fixup_positions(&mut self.space, self.pointer.current_location(), &self.prefs.pinned_positions());
        self.relayout_all_outputs();
    }

    /// A page flip completed. `throttled` is a vblank that came in early and
    /// was held back until now, handed over by its timer.
    fn frame_finish(
        &mut self,
        dev_id: DrmNode,
        crtc: crtc::Handle,
        metadata: &mut Option<DrmEventMetadata>,
        throttled: bool,
    ) {
        profiling::scope!("frame_finish", &format!("{crtc:?}"));
        let _phase = stall::enter(Phase::Vblank);

        let tp = metadata.as_ref().and_then(|metadata| match metadata.time {
            smithay::backend::drm::DrmEventTime::Monotonic(tp) => tp.is_zero().not().then_some(tp),
            smithay::backend::drm::DrmEventTime::Realtime(_) => None,
        });
        // Only if it is anywhere near now: see `sane_vblank_time`.
        let tp = tp.and_then(|tp| {
            let now: Duration = self.clock.now().into();
            let sane = sane_vblank_time(tp, now);
            if sane.is_none() {
                debug!(?crtc, ?tp, ?now, "ignoring a vblank time that is nowhere near the clock");
            }
            sane
        });
        let seq = metadata.as_ref().map(|metadata| metadata.sequence).unwrap_or(0);

        let output = self.output_of(dev_id, crtc);
        let handle = self.handle.clone();
        let Some(surface) = self
            .backend_data
            .backends
            .get_mut(&dev_id)
            .and_then(|device| device.surfaces.get_mut(&crtc))
        else {
            debug!(?crtc, "page flip finished on a CRTC that no longer draws");
            return;
        };
        if surface.releasing.is_some() {
            // The flip the recovery gave up on came back after all. The
            // recovery hands its frame back when it is done.
            trace!(?crtc, "page flip finished on an output being reset");
            return;
        }
        // The event's timestamp cannot tell a late event of a flip that was
        // given up on from this one: the first event after a modeset can
        // carry the CRTC's last vblank from before it went dark (virtio-gpu
        // does), which would leave every relit display looking stuck. Nor
        // does it need to. A reset only queues its first frame once the
        // kernel let go of the CRTC, and the kernel sends the event of any
        // flip it was holding before it lets go, while the reset is still
        // under way.
        if surface.flip.is_none() {
            // A flip that was given up on: the display was switched off, the
            // session was resumed, or the output was reset since.
            debug!(?crtc, "page flip finished with no frame in flight");
            return;
        }
        if !throttled {
            if let (Some(faults), Some(output)) = (self.backend_data.faults.as_mut(), output.as_ref()) {
                if faults.take_vblank(&output.name()) {
                    warn!(output = output.name(), "debug fault: this page flip's event is dropped");
                    return;
                }
            }
        }

        if let Some(throttle) = surface.vblank_throttle_timer.take() {
            handle.remove(throttle.token);
        }

        let Some(output) = output else {
            // Nothing will draw on this CRTC again. Hand the frame back, so
            // the surface is clean if its output comes back.
            if let Some(token) = surface.flip.take().and_then(|flip| flip.deadline) {
                handle.remove(token);
            }
            surface.flip_scanout = None;
            forget_frame(surface);
            return;
        };

        // 60 Hz is the fallback everywhere a mode is missing: it only times
        // the next repaint, and a wrong guess costs a late frame.
        let frame_duration = crate::timing::frame_duration(
            output.current_mode().map(|mode| mode.refresh).unwrap_or(60_000),
        );

        let (clock, flags) = if let Some(tp) = tp {
            (
                tp.into(),
                wp_presentation_feedback::Kind::Vsync
                    | wp_presentation_feedback::Kind::HwClock
                    | wp_presentation_feedback::Kind::HwCompletion,
            )
        } else {
            (self.clock.now(), wp_presentation_feedback::Kind::Vsync)
        };

        let vblank_remaining_time = surface.last_presentation_time.map(|last_presentation_time| {
            frame_duration.saturating_sub(Time::elapsed(&last_presentation_time, clock))
        });

        if let Some(vblank_remaining_time) = vblank_remaining_time {
            if vblank_remaining_time > frame_duration / 2 {
                static WARN_ONCE: Once = Once::new();
                WARN_ONCE.call_once(|| {
                    warn!("display running faster than expected, throttling vblanks and disabling HwClock")
                });
                let throttled_time = tp
                    .map(|tp| tp.saturating_add(vblank_remaining_time))
                    .unwrap_or(Duration::ZERO);
                let throttled_metadata = DrmEventMetadata {
                    sequence: seq,
                    time: DrmEventTime::Monotonic(throttled_time),
                };
                match self
                    .handle
                    .insert_source(Timer::from_duration(vblank_remaining_time), move |_, _, data| {
                        // This timer is the source being dispatched, and it
                        // drops itself below. Forget the token first, or
                        // `frame_finish` removes a source that is already on
                        // its way out and the loop loses track of the slot.
                        if let Some(surface) = data
                            .backend_data
                            .backends
                            .get_mut(&dev_id)
                            .and_then(|device| device.surfaces.get_mut(&crtc))
                        {
                            surface.vblank_throttle_timer = None;
                        }
                        data.frame_finish(dev_id, crtc, &mut Some(throttled_metadata), true);
                        TimeoutAction::Drop
                    }) {
                    Ok(token) => {
                        // The vblank came; the timer hands it over at its
                        // time, so the flip needs no deadline any more.
                        surface.vblank_throttle_timer = Some(ThrottleTimer {
                            token,
                            due: Instant::now() + vblank_remaining_time,
                        });
                        if let Some(token) = surface.flip.as_mut().and_then(|flip| flip.deadline.take()) {
                            self.handle.remove(token);
                        }
                        return;
                    }
                    Err(err) => {
                        // Without the timer this vblank would be dropped and
                        // never redelivered, which stops the output. Take it
                        // as it came instead: an early frame, not a dead one.
                        error!(?crtc, ?err, "cannot throttle this vblank");
                    }
                }
            }
        }
        // How the frame actually landed, before this one becomes the past.
        // A gap of half a refresh or more, while the output was drawing
        // frame after frame, is a frame that did not make its vblank: this
        // is stutter, counted rather than guessed at. A gap longer than a
        // few frames means the output was idle in between, which is not the
        // same thing at all.
        if surface.last_presentation_time.is_some() {
            surface.frames = surface.frames.saturating_add(1);
        }
        // Late means this frame missed the vblank its repaint was drawn for,
        // and not that the output is running below the panel's top rate --
        // which it does whenever nothing needs more. See `missed_vblank`.
        if let Some(target) = surface.frame_target {
            if crate::timing::missed_vblank(target.into(), clock.into(), frame_duration) {
                surface.late_frames = surface.late_frames.saturating_add(1);
            }
        }
        surface.last_presentation_time = Some(clock);

        // The flip came back: nothing waits on its deadline any more, and
        // what was on the plane reached the screen, so it is no evidence
        // against direct scan-out.
        if let Some(token) = surface.flip.take().and_then(|flip| flip.deadline) {
            self.handle.remove(token);
        }
        surface.flip_scanout = None;
        surface.dead_ticks = 0;
        if !surface.health.flipped {
            surface.health.flipped = true;
            if let Some(at) = surface.health.recovered_at {
                info!(
                    output = output.name(),
                    "the display is showing frames again, {:?} after it was reset",
                    at.elapsed()
                );
            }
        }
        surface.health.settle();

        let submit_result = surface
            .drm_output
            .frame_submitted()
            .map_err(Into::<SwapBuffersError>::into);

        let after = match submit_result {
            Ok(user_data) => {
                if let Some(mut feedback) = user_data.flatten() {
                    feedback.presented(clock, Refresh::fixed(frame_duration), seq as u64, flags);
                }

                AfterFlip::Repaint
            }
            // The session is paused (a VT switch, or suspend). Its resume
            // handler repaints every output.
            Err(err) if Failure::of(&err).is_none() => {
                trace!(?crtc, "page flip finished on an inactive device");
                AfterFlip::Wait
            }
            // Handing the frame back submits the one queued behind it, if
            // any, and the kernel did not take that. Draw the next one rather
            // than stop; a frame that keeps being refused is caught there.
            Err(err) => {
                debug!(?crtc, "the frame queued behind the last flip was not taken: {err:?}");
                AfterFlip::Repaint
            }
        };

        if let AfterFlip::Repaint = after {
            let next_frame_target = clock + frame_duration;

            // What are we trying to solve by introducing a delay here:
            //
            // Basically it is all about latency of client provided buffers.
            // A client driven by frame callbacks will wait for a frame callback
            // to repaint and submit a new buffer. As we send frame callbacks
            // as part of the repaint in the compositor the latency would always
            // be approx. 2 frames. By introducing a delay before we repaint in
            // the compositor we can reduce the latency to approx. 1 frame + the
            // remaining duration from the repaint to the next VBlank.
            //
            // With the delay it is also possible to further reduce latency if
            // the client is driven by presentation feedback. As the presentation
            // feedback is directly sent after a VBlank the client can submit a
            // new buffer during the repaint delay that can hit the very next
            // VBlank, thus reducing the potential latency to below one frame.
            //
            // Choosing a good delay is a topic on its own so we just implement
            // a simple strategy here. We just split the duration between two
            // VBlanks into two steps, one for the client repaint and one for the
            // compositor repaint. Theoretically the repaint in the compositor should
            // be faster so we give the client a bit more time to repaint. On a typical
            // modern system the repaint in the compositor should not take more than 2ms
            // so this should be safe for refresh rates up to at least 120 Hz. For 120 Hz
            // this results in approx. 3.33ms time for repainting in the compositor.
            // A too big delay could result in missing the next VBlank in the compositor.
            //
            // A more complete solution could work on a sliding window analyzing past repaints
            // and do some prediction for the next repaint.
            // How late the repaint can start and still make the vblank
            // depends on how long a repaint takes on this output, which is
            // measured rather than assumed: see `timing::repaint_delay`.
            let repaint_delay =
                crate::timing::repaint_delay(frame_duration, surface.render_times.estimate());

            let delay = if surface
                .render_node
                .map(|render_node| render_node != self.backend_data.primary_gpu)
                .unwrap_or(true)
            {
                // However, if we need to do a copy, that might not be enough.
                // (And without actual comparision to previous frames we cannot really know.)
                // So lets ignore that in those cases to avoid thrashing performance.
                trace!("scheduling repaint timer immediately on {:?}", crtc);
                Duration::ZERO
            } else {
                trace!(
                    "scheduling repaint timer with delay {:?} on {:?}",
                    repaint_delay,
                    crtc
                );
                repaint_delay
            };

            arm_repaint(&self.handle, surface, dev_id, crtc, delay, Some(next_frame_target), false);
        }
    }

    /// A page flip whose vblank never arrived inside its deadline. Unless
    /// the display is off on purpose, the output is stuck.
    fn flip_timed_out(&mut self, node: DrmNode, crtc: crtc::Handle, queued_at: Instant) {
        let blanked = self.idle.stage == crate::idle::Stage::Blank;
        let inactive = self
            .backend_data
            .backends
            .get(&node)
            .is_none_or(|device| !device.drm_output_manager.device().is_active());
        let Some(surface) = self.surface_mut(node, crtc) else {
            return;
        };
        let Some(flip) = surface.flip.as_mut().filter(|flip| flip.queued_at == queued_at) else {
            // The flip was answered for, or given up on: this deadline
            // belongs to nothing.
            return;
        };
        // The timer that just fired removes itself, so forget the token
        // before anything else can try to remove it again.
        flip.deadline = None;
        let waited = flip.queued_at.elapsed();
        if blanked || inactive {
            // The display is off or the session is paused: a page flip that
            // never completes is exactly what the hardware should do, and
            // waking up repaints from scratch.
            trace!(?crtc, "page flip outlived its deadline while the display was off");
            surface.flip = None;
            return;
        }
        self.recover_output(node, crtc, Cause::StuckFlip, &format!("no page flip for {waited:?}"));
    }

    fn output_of(&self, node: DrmNode, crtc: crtc::Handle) -> Option<Output> {
        self.space
            .outputs()
            .find(|o| o.user_data().get::<UdevOutputId>() == Some(&UdevOutputId { device_id: node, crtc }))
            .cloned()
    }

    fn surface_mut(&mut self, node: DrmNode, crtc: crtc::Handle) -> Option<&mut SurfaceData> {
        self.backend_data
            .backends
            .get_mut(&node)
            .and_then(|device| device.surfaces.get_mut(&crtc))
    }

    /// The presentation time a repaint started now should aim at: the next
    /// vblank after the last target, or now when there was none yet.
    fn repaint_target(&self, node: DrmNode, crtc: crtc::Handle) -> Time<Monotonic> {
        let now = self.clock.now();
        let last = self
            .backend_data
            .backends
            .get(&node)
            .and_then(|device| device.surfaces.get(&crtc))
            .and_then(|surface| surface.frame_target);
        let (Some(last), Some(refresh)) = (last, self.output_refresh(node, crtc)) else {
            return now;
        };
        now + crate::timing::next_repaint(now.into(), last.into(), refresh).saturating_sub(now.into())
    }

    fn output_refresh(&self, node: DrmNode, crtc: crtc::Handle) -> Option<i32> {
        self.space
            .outputs()
            .find(|o| {
                o.user_data().get::<UdevOutputId>()
                    == Some(&UdevOutputId {
                        device_id: node,
                        crtc,
                    })
            })
            .and_then(|o| o.current_mode())
            .map(|mode| mode.refresh)
    }

    /// Something on screen moves by itself, so the next frame is wanted even
    /// though nothing asked for one: the columns sliding into place, the Mind
    /// bar's spinner, a fading feedback card, the start-up backdrop, an
    /// animated cursor, or a commit a client timed for a later frame.
    fn wants_frame(&mut self) -> bool {
        if self.layout.animating() || self.mindbar.animating() {
            return true;
        }
        if let CursorImageStatus::Named(icon) = self.cursor_status {
            if self.backend_data.pointer_image.animated(icon, 1 /*scale*/) {
                return true;
            }
        }
        self.commit_timers_pending()
    }

    // If crtc is `Some()`, render it, else render all crtcs
    fn render(&mut self, node: DrmNode, crtc: Option<crtc::Handle>, frame_target: Time<Monotonic>) {
        let device_backend = match self.backend_data.backends.get_mut(&node) {
            Some(backend) => backend,
            None => {
                error!("Trying to render on non-existent backend {}", node);
                return;
            }
        };

        if let Some(crtc) = crtc {
            self.render_surface(node, crtc, frame_target);
        } else {
            let crtcs: Vec<_> = device_backend.surfaces.keys().copied().collect();
            for crtc in crtcs {
                self.render_surface(node, crtc, frame_target);
            }
        };
    }

    fn render_surface(&mut self, node: DrmNode, crtc: crtc::Handle, frame_target: Time<Monotonic>) {
        profiling::scope!("render_surface", &format!("{crtc:?}"));
        let _phase = stall::enter(Phase::Render);

        if self.idle.stage == crate::idle::Stage::Blank {
            // The displays are off: no frame, no page flip, no vblank. The
            // loop starts again from `set_blanked`.
            return;
        }

        let Some(output) = self.output_of(node, crtc) else {
            // The surface outlived the output it draws, which draws nothing
            // by design; connecting the output again starts it.
            debug!(?crtc, "a repaint was asked for on an output the space does not know");
            return;
        };

        match self.surface_mut(node, crtc) {
            Some(surface) if surface.busy() => {
                // A frame in flight, a vblank held back, or a reset: each
                // continues the loop itself, and a frame queued now would be
                // refused or dropped. Draw what changed when it does.
                surface.dirty = true;
                return;
            }
            Some(_) => {}
            None => return,
        }

        self.pre_repaint(&output, frame_target);

        let rejected = self
            .backend_data
            .faults
            .as_mut()
            .is_some_and(|faults| faults.rejects(&output.name()));

        let device = if let Some(device) = self.backend_data.backends.get_mut(&node) {
            device
        } else {
            warn!(?node, "repaint on a device that is gone");
            return;
        };

        let surface = if let Some(surface) = device.surfaces.get_mut(&crtc) {
            surface
        } else {
            warn!(?crtc, "repaint on a surface that is gone");
            return;
        };

        // A repaint kicked off from elsewhere (the displays lighting up, the
        // session resuming) supersedes whatever timer was armed.
        if let Some(armed) = surface.repaint_timer.take() {
            self.handle.remove(armed.token);
        }
        // Changes from here on belong to the next frame.
        surface.dirty = false;
        surface.frame_target = Some(frame_target);

        let start = Instant::now();

        // The shape the client asked for (wp_cursor_shape_v1), or the arrow.
        let icon = match &self.cursor_status {
            CursorImageStatus::Named(icon) => *icon,
            _ => CursorIcon::Default,
        };
        // TODO get scale from the rendersurface when supporting HiDPI
        let (index, frame) = self
            .backend_data
            .pointer_image
            .get_image(icon, 1 /*scale*/, self.clock.now().into());

        let primary_gpu = self.backend_data.primary_gpu;
        let render_node = surface.render_node.unwrap_or(primary_gpu);
        let renderer = if primary_gpu == render_node {
            self.backend_data.gpus.single_renderer(&render_node)
        } else {
            let format = surface.drm_output.format();
            self.backend_data
                .gpus
                .renderer(&primary_gpu, &render_node, format)
        };
        let renderer = match renderer {
            Ok(renderer) => Some(renderer),
            Err(err) => {
                // No renderer for this GPU right now: a device being added or
                // taken away, a driver that went missing, a GPU reset. This
                // used to be an `unwrap`, which took the whole session — every
                // other display included — down with it. Look again shortly.
                error!(?crtc, %err, "no renderer for this output, trying again shortly");
                None
            }
        };
        let Some(mut renderer) = renderer else {
            surface.last_render = Instant::now();
            arm_repaint(&self.handle, surface, node, crtc, IDLE_REPAINT, None, false);
            return;
        };

        let pointer_image = self
            .backend_data
            .pointer_images
            .entry((icon, index))
            .or_insert_with(|| {
                MemoryRenderBuffer::from_slice(
                    &frame.pixels_rgba,
                    Fourcc::Argb8888,
                    (frame.width as i32, frame.height as i32),
                    1,
                    Transform::Normal,
                    None,
                )
            })
            .clone();

        let result = if rejected {
            Err(SwapBuffersError::ContextLost(Box::new(io::Error::from_raw_os_error(libc::EINVAL))))
        } else {
            render_surface(
                surface,
                &mut renderer,
                &self.space,
                &output,
                self.pointer.current_location(),
                &pointer_image,
                (frame.xhot as i32, frame.yhot as i32).into(),
                &mut self.backend_data.pointer_element,
                &self.dnd_icon,
                &mut self.cursor_status,
                self.show_window_preview,
                &mut self.mindbar,
                self.config.theme.show_wordmark,
                self.idle.locked,
                &mut self.capture,
                self.clock.now().into(),
            )
        };
        let next = match result {
            Ok((has_rendered, states)) => {
                if has_rendered {
                    if surface.health.logged_at.is_some() {
                        info!(
                            output = output.name(),
                            refused = surface.health.refused,
                            "the display takes frames again"
                        );
                    }
                    surface.health.refusals_over();
                }
                let dmabuf_feedback = surface.dmabuf_feedback.clone();
                self.post_repaint(&output, frame_target, dmabuf_feedback, &states);
                if has_rendered {
                    // The frame is queued: its vblank arms the next repaint,
                    // and its deadline covers the vblank that never comes.
                    Repaint::Flipped
                } else if self.surface_dirty(node, crtc) || self.wants_frame() {
                    Repaint::NextFrame
                } else {
                    Repaint::Idle
                }
            }
            Err(err) => match Failure::of(&err) {
                // A VT switch or suspend. Resuming repaints.
                None => {
                    trace!(?crtc, "skipping a frame on an inactive device");
                    Repaint::Wait
                }
                Some(failure) => self.frame_refused(node, crtc, &output, failure, &err),
            },
        };

        // 60 Hz is the fallback everywhere a mode is missing: it is only
        // ever used to time the next look, and a wrong guess costs a late
        // frame where returning would cost the whole output.
        let output_refresh = output.current_mode().map(|mode| mode.refresh).unwrap_or(60_000);

        // A repaint ran to completion. This is what the watchdog looks for:
        // not a timer, not a flip, but the loop actually going round.
        if let Some(surface) = self
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|device| device.surfaces.get_mut(&crtc))
        {
            surface.last_render = Instant::now();
            surface.repaints = surface.repaints.saturating_add(1);
            if matches!(next, Repaint::Flipped) {
                // Only frames that were really drawn and queued: an empty
                // repaint says nothing about how long a full one takes.
                surface.render_times.push(start.elapsed());
            }
        }

        match next {
            Repaint::Flipped => {
                let elapsed = start.elapsed();
                tracing::trace!(?elapsed, "rendered surface");
                self.arm_surface_flip_deadline(node, crtc, Some(output_refresh));
            }
            Repaint::Wait => {
                trace!(?crtc, "the repaint loop stops until the session resumes");
            }
            Repaint::Retry(delay) => {
                let target = self.clock.now() + delay;
                self.arm_surface_repaint(node, crtc, delay, Some(target), false);
            }
            Repaint::Recovering => {
                trace!(?crtc, "the repaint loop waits for the reset");
            }
            Repaint::NextFrame => {
                // Either a temporary failure, or more likely nothing changed on
                // screen while something else still wants a frame (an animation,
                // a change that came in during the repaint). Look again after
                // approx. one frame.
                let now = self.clock.now();
                let next_frame_target = now + crate::timing::next_repaint(
                    now.into(), frame_target.into(), output_refresh,
                ).saturating_sub(now.into());
                let reschedule_timeout =
                    Duration::from(next_frame_target).saturating_sub(now.into());
                trace!(
                    "reschedule repaint timer with delay {:?} on {:?}",
                    reschedule_timeout,
                    crtc,
                );
                self.arm_surface_repaint(node, crtc, reschedule_timeout, Some(next_frame_target), false);
            }
            Repaint::Idle => {
                // Nothing to draw and nothing moving: stop repainting every
                // frame. The next change (`request_repaint`) starts the loop
                // again at once; the slow tick catches the rest.
                trace!("output idle, ticking every {:?} on {:?}", IDLE_REPAINT, crtc);
                self.arm_surface_repaint(node, crtc, IDLE_REPAINT, None, true);
            }
        }

        profiling::finish_frame!();
    }

    /// The watchdog. Every lit output must have a repaint timer armed, a
    /// frame in flight, or a reset under way; a flip must come back within
    /// its deadline. The per-flip deadline normally catches a stuck output
    /// first; this catches whatever slipped past it.
    fn check_outputs_alive(&mut self) {
        if self.idle.stage == crate::idle::Stage::Blank {
            return;
        }
        let session_active = self.backend_data.session.is_active();
        let mut reactivate = Vec::new();
        let known: Vec<(UdevOutputId, Option<i32>)> = self
            .space
            .outputs()
            .filter_map(|o| {
                let id = o.user_data().get::<UdevOutputId>().copied()?;
                Some((id, o.current_mode().map(|mode| mode.refresh)))
            })
            .collect();
        let mut stuck = Vec::new();
        let mut stopped = Vec::new();
        for (node, device) in self.backend_data.backends.iter_mut() {
            if !device.drm_output_manager.device().is_active() {
                // While the session is paused — a VT switch, suspend — this
                // is exactly right, and resuming repaints everything. A
                // device still refusing frames while the session is running
                // is a resume that did not take: `activate` is allowed to
                // fail, and every display on the device then stayed dark for
                // the rest of the session. Try again instead.
                let since = *device.inactive_since.get_or_insert_with(Instant::now);
                if session_active && since.elapsed() > REACTIVATE_AFTER {
                    reactivate.push(*node);
                }
                continue;
            }
            device.inactive_since = None;
            for (crtc, surface) in device.surfaces.iter_mut() {
                // A surface whose output the space has forgotten draws
                // nothing by design; leaving it alone is not a freeze.
                let Some((_, refresh)) = known
                    .iter()
                    .find(|(id, _)| id.device_id == *node && id.crtc == *crtc)
                else {
                    continue;
                };
                if let Some(release) = surface.releasing.as_mut() {
                    surface.dead_ticks = 0;
                    let waited = release.started.elapsed();
                    if !release.reported && waited >= RELEASE_SLOW {
                        release.reported = true;
                        warn!(
                            ?crtc,
                            step = ?release.step,
                            ?waited,
                            "the kernel still has not let go of the display; its driver may be stuck"
                        );
                    }
                    continue;
                }
                if surface.recovery_timer.is_some() {
                    surface.dead_ticks = 0;
                    continue;
                }
                if let Some(flip) = surface.flip.as_ref() {
                    surface.dead_ticks = 0;
                    let waited = flip.queued_at.elapsed();
                    if waited > crate::timing::flip_deadline(*refresh) + WATCHDOG_TICK * 2 {
                        stuck.push((*node, *crtc, waited));
                    }
                    continue;
                }
                // Nothing is in flight, so the loop has to be coming round on
                // its own. Two questions, and either answer is enough.
                //
                // The timers, first: an armed timer is not proof of life, one
                // that is long past due belongs to a loop that has stopped
                // just as surely as no timer at all.
                let overdue = surface
                    .repaint_timer
                    .as_ref()
                    .map(|armed| armed.due)
                    .into_iter()
                    .chain(surface.vblank_throttle_timer.as_ref().map(|throttle| throttle.due))
                    .map(|due| due.elapsed())
                    .max();
                // And then the question that cannot be answered wrongly:
                // has a repaint actually run? Every lit output repaints at
                // least once a second, whether anything changed or not. This
                // needs no theory about which timer should have fired or
                // which event went missing — a frozen display is one that
                // has not drawn, and that is precisely what this measures.
                let reason = stopped_reason(surface.last_render.elapsed(), overdue);
                match reason {
                    // Two ticks in a row, so a loop that was merely blocked
                    // for a moment (a modeset, a slow GPU) is not reset.
                    Some(reason) => {
                        surface.dead_ticks = surface.dead_ticks.saturating_add(1);
                        if surface.dead_ticks >= 2 {
                            surface.dead_ticks = 0;
                            stopped.push((*node, *crtc, reason));
                        }
                    }
                    None => surface.dead_ticks = 0,
                }
            }
        }
        for node in reactivate {
            self.reactivate_device(node);
        }
        for (node, crtc, waited) in stuck {
            self.recover_output(
                node,
                crtc,
                Cause::StuckFlip,
                &format!("no page flip for {waited:?}, and its deadline never fired"),
            );
        }
        for (node, crtc, reason) in stopped {
            // Nothing is wrong with the display, only with the bookkeeping:
            // drawing again is all it takes.
            warn!(?crtc, "the repaint loop had stopped ({reason}); starting it again");
            self.arm_surface_repaint(node, crtc, Duration::ZERO, None, false);
        }
    }

    /// A DRM device that is still refusing frames long after the session came
    /// back. Take it again from the top, as resuming does: activate, throw
    /// away everything that belonged to the state it lost, and repaint every
    /// output on it.
    fn reactivate_device(&mut self, node: DrmNode) {
        let handle = self.handle.clone();
        let Some(device) = self.backend_data.backends.get_mut(&node) else {
            return;
        };
        warn!(%node, "the device is inactive while the session is running; taking it again");
        if let Err(err) = device.drm_output_manager.activate(true) {
            error!(%node, ?err, "cannot take the device back; trying again shortly");
            // Start the clock again so this is retried, rather than giving up
            // on every display the device drives.
            device.inactive_since = Some(Instant::now());
            return;
        }
        device.inactive_since = None;
        for surface in device.surfaces.values_mut() {
            cancel_timers(&handle, surface);
            surface.flip = None;
            surface.flip_scanout = None;
            surface.releasing = None;
            forget_frame(surface);
            surface.health.refusals_over();
            surface.drm_output.with_compositor(|compositor| {
                if let Err(err) = compositor.use_mode(compositor.pending_mode()) {
                    warn!(?err, "cannot set the mode again after taking the device back");
                }
                compositor.reset_buffer_ages();
            });
            surface.last_presentation_time = None;
            surface.frame_target = None;
            surface.last_render = Instant::now();
            surface.dead_ticks = 0;
            surface.dirty = true;
        }
        self.mindbar.invalidate_graphics();
        self.handle
            .insert_idle(move |data| data.render(node, None, data.clock.now()));
    }

    /// A frame the kernel did not take. A hiccup is retried at the next
    /// frame and a run of them ever more slowly; a run that lasts
    /// `REFUSED_FOR` resets the output. What no reset can fix, the device
    /// belonging to another process or the renderer being gone, is only
    /// retried.
    fn frame_refused(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        output: &Output,
        failure: Failure,
        err: &SwapBuffersError,
    ) -> Repaint {
        let frame = crate::timing::frame_duration(output.current_mode().map(|mode| mode.refresh).unwrap_or(60_000));
        let Some(surface) = self.surface_mut(node, crtc) else {
            return Repaint::Wait;
        };
        // The frame was drawn into a buffer the display never showed, so the
        // damage history is out of step with the screen: draw the next whole.
        surface
            .drm_output
            .with_compositor(|compositor| compositor.reset_buffer_ages());
        let now = Instant::now();
        let health = &mut surface.health;
        let since = *health.refused_since.get_or_insert(now);
        health.refused = health.refused.saturating_add(1);
        let refused = health.refused;
        let refused_for = now.duration_since(since);
        let changed = health.last_refusal.replace(failure) != Some(failure);
        let due = health
            .logged_at
            .is_none_or(|at| now.duration_since(at) >= recovery::REFUSALS_LOGGED_EVERY);
        if changed || due {
            health.logged_at = Some(now);
            warn!(
                output = output.name(),
                refused,
                ?refused_for,
                ?failure,
                "the display did not take a frame: {err}"
            );
        } else {
            debug!(output = output.name(), refused, ?failure, "the display did not take a frame: {err}");
        }
        match failure {
            Failure::Permission | Failure::Renderer => Repaint::Retry(recovery::retry_delay(refused, frame)),
            Failure::TestFailed if refused == 1 => {
                // The kernel's idea of the CRTC no longer matches ours, most
                // likely because another DRM master changed it during a VT
                // switch. Read it back, so the next frame commits whatever
                // it takes to get there.
                if let Err(err) = surface
                    .drm_output
                    .with_compositor(|compositor| compositor.reset_state())
                {
                    warn!(output = output.name(), ?err, "cannot read the display's state back from the kernel");
                }
                Repaint::Retry(frame)
            }
            _ if refused_for >= recovery::REFUSED_FOR => {
                self.recover_output(
                    node,
                    crtc,
                    Cause::Refused(failure),
                    &format!("the display refused every frame for {refused_for:?}"),
                );
                Repaint::Recovering
            }
            _ => Repaint::Retry(recovery::retry_delay(refused, frame)),
        }
    }

    /// Reset an output that stopped taking frames. The cause and the
    /// recoveries since the output last settled pick how hard, and the
    /// backoff how soon. Nothing here waits on the kernel: a helper thread
    /// releases the CRTC, and `release_done` takes it from there.
    fn recover_output(&mut self, node: DrmNode, crtc: crtc::Handle, cause: Cause, reason: &str) {
        let name = self
            .output_of(node, crtc)
            .map(|output| output.name())
            .unwrap_or_else(|| format!("{crtc:?}"));
        let now = self.clock.now();
        let handle = self.handle.clone();
        let Some(surface) = self.surface_mut(node, crtc) else {
            return;
        };
        if surface.releasing.is_some() || (surface.recovery_timer.is_some() && cause != Cause::Requested) {
            debug!(output = %name, ?cause, "{reason}; a reset is already under way");
            return;
        }
        cancel_timers(&handle, surface);
        surface.flip = None;
        let scanout = surface.flip_scanout.take();
        surface.resets = surface.resets.saturating_add(1);
        if cause == Cause::StuckFlip && scanout.is_some() && surface.scanout != DirectScanout::Off {
            // The frame that never reached the screen had a client's own
            // buffer on the primary plane, waiting on a fence only that
            // client can signal. A game that has to be composed costs a
            // little latency; a monitor that never updates costs everything.
            warn!(
                output = %name,
                "the stuck frame was scanned out directly, so this output composes from now on; \
                 set graphics.direct_scanout in mindwm.toml to keep it that way"
            );
            surface.scanout = DirectScanout::Off;
        }
        let last_presented = surface.last_presentation_time.map(|t| Time::elapsed(&t, now));
        let health = &mut surface.health;
        health.settle();
        health.recoveries = health
            .recoveries
            .saturating_add(1)
            .max(cause.first_step().recoveries());
        health.refusals_over();
        let recoveries = health.recoveries;
        let step = Step::of(recoveries);
        let wait = match (cause, health.recovered_at) {
            (Cause::Requested, _) | (_, None) => Duration::ZERO,
            (_, Some(at)) => recovery::backoff(recoveries).saturating_sub(at.elapsed()),
        };
        warn!(
            output = %name,
            ?crtc,
            ?cause,
            recoveries,
            ?scanout,
            ?last_presented,
            "{reason}; resetting the output by {}",
            step.describe()
        );
        if wait.is_zero() {
            self.start_release(node, crtc, step, cause);
            return;
        }
        info!(output = %name, "the display keeps failing; waiting {wait:?} before resetting it again");
        let timer = handle.insert_source(Timer::from_duration(wait), move |_, _, data| {
            if let Some(surface) = data.surface_mut(node, crtc) {
                surface.recovery_timer = None;
            }
            data.start_release(node, crtc, step, cause);
            TimeoutAction::Drop
        });
        match timer {
            Ok(token) => {
                if let Some(surface) = self.surface_mut(node, crtc) {
                    surface.recovery_timer = Some(token);
                }
            }
            Err(err) => {
                warn!(output = %name, ?err, "cannot wait before resetting the output; resetting it now");
                self.start_release(node, crtc, step, cause);
            }
        }
    }

    /// Hand the CRTCs a recovery needs to a helper thread, which makes the
    /// kernel let go of them: it waits out a page flip that is stuck, and for
    /// the harder steps switches them off. The other displays and the pointer
    /// keep moving meanwhile.
    fn start_release(&mut self, node: DrmNode, crtc: crtc::Handle, step: Step, cause: Cause) {
        let handle = self.handle.clone();
        let answers = self.backend_data.answers.clone();
        let Some(device) = self.backend_data.backends.get_mut(&node) else {
            return;
        };
        if !device.surfaces.contains_key(&crtc) {
            return;
        }
        if !device.drm_output_manager.device().is_active() {
            // The session is paused, and resuming resets every output anyway.
            debug!(?crtc, "not resetting a display while the session is paused");
            return;
        }
        if step == Step::ResetDevice
            && device
                .surfaces
                .iter()
                .any(|(other, surface)| *other != crtc && surface.releasing.is_some())
        {
            // Another output is being released on its own; switching the
            // whole device off on top of that would release it twice.
            let timer = handle.insert_source(Timer::from_duration(Duration::from_millis(250)), move |_, _, data| {
                if let Some(surface) = data.surface_mut(node, crtc) {
                    surface.recovery_timer = None;
                }
                data.start_release(node, crtc, step, cause);
                TimeoutAction::Drop
            });
            if let (Ok(token), Some(surface)) = (timer, device.surfaces.get_mut(&crtc)) {
                surface.recovery_timer = Some(token);
            }
            return;
        }

        let crtcs: Vec<crtc::Handle> = if step == Step::ResetDevice {
            device.surfaces.keys().copied().collect()
        } else {
            vec![crtc]
        };
        let started = Instant::now();
        for each in &crtcs {
            let Some(surface) = device.surfaces.get_mut(each) else {
                continue;
            };
            cancel_timers(&handle, surface);
            surface.flip = None;
            surface.flip_scanout = None;
            surface.health.refusals_over();
            surface.releasing = Some(Release {
                started,
                step,
                cause: if *each == crtc { cause } else { Cause::Device },
                reported: false,
            });
        }
        if let Some(surface) = device.surfaces.get_mut(&crtc) {
            surface.health.recovered_at = Some(started);
            surface.health.flipped = false;
        }

        let fd = device.drm_output_manager.device().device_fd().clone();
        let off = matches!(step, Step::Relight | Step::ResetDevice);
        let spawned = std::thread::Builder::new().name("mindwm-release".into()).spawn({
            let fd = fd.clone();
            let crtcs = crtcs.clone();
            move || {
                let result = release_crtcs(&fd, &crtcs, off);
                let _ = answers.send(Answer::Released {
                    node,
                    crtc,
                    started,
                    result,
                });
            }
        });
        if let Err(err) = spawned {
            warn!(%err, "no thread to reset the display on; resetting it on the event loop");
            let result = release_crtcs(&fd, &crtcs, off);
            handle.insert_idle(move |data| data.release_done(node, crtc, started, result));
        }
    }

    /// The kernel let go of the CRTCs of a recovery, or gave up trying: take
    /// the step the recovery was for, and draw.
    fn release_done(&mut self, node: DrmNode, crtc: crtc::Handle, started: Instant, result: Result<Duration, String>) {
        let _phase = stall::enter(Phase::Recovery);
        self.finish_release(node, crtc, started, result);
        // Connectors that changed during the reset were left for now.
        let Some(device) = self.backend_data.backends.get_mut(&node) else {
            return;
        };
        if device.rescan_pending
            && !device.probing
            && !device.surfaces.values().any(|surface| surface.releasing.is_some())
        {
            device.rescan_pending = false;
            self.hotplug_changed(node);
        }
    }

    fn finish_release(&mut self, node: DrmNode, crtc: crtc::Handle, started: Instant, result: Result<Duration, String>) {
        let blanked = self.idle.stage == crate::idle::Stage::Blank;
        let name = self
            .output_of(node, crtc)
            .map(|output| output.name())
            .unwrap_or_else(|| format!("{crtc:?}"));
        let Some(device) = self.backend_data.backends.get_mut(&node) else {
            return;
        };
        // The outputs this recovery released: the one it was for, and with
        // `ResetDevice` every other output on the device.
        let mut released = Vec::new();
        let mut taken = None;
        for (each, surface) in device.surfaces.iter_mut() {
            let ours = surface
                .releasing
                .as_ref()
                .is_some_and(|release| release.started == started && (release.step == Step::ResetDevice || *each == crtc));
            if !ours {
                continue;
            }
            if let Some(release) = surface.releasing.take() {
                if *each == crtc {
                    taken = Some((release.step, release.cause));
                }
                released.push(*each);
            }
        }
        let Some((step, cause)) = taken else {
            // Overtaken: the session resumed, or the output went away.
            debug!(?crtc, "a display reset finished after it was overtaken");
            return;
        };
        match &result {
            Ok(took) if *took >= Duration::from_millis(500) => {
                info!(output = %name, ?cause, "the kernel let go of the display after {took:?}")
            }
            Ok(took) => debug!(output = %name, ?cause, ?took, "the kernel let go of the display"),
            Err(err) => warn!(output = %name, ?cause, "{err}; resetting the display anyway"),
        }

        for each in &released {
            if let Some(surface) = device.surfaces.get_mut(each) {
                forget_frame(surface);
                surface.last_presentation_time = None;
                surface.frame_target = None;
                surface.last_render = Instant::now();
                surface.dead_ticks = 0;
                surface.dirty = true;
            }
        }
        if !device.drm_output_manager.device().is_active() {
            // The session was paused meanwhile; resuming resets everything.
            return;
        }
        if blanked {
            // The displays went off meanwhile, and lighting them up redraws
            // everything from scratch.
            for each in &released {
                if let Some(surface) = device.surfaces.get_mut(each) {
                    blank_surface(surface);
                }
            }
            return;
        }

        match step {
            Step::Resubmit => {
                for each in &released {
                    if let Some(surface) = device.surfaces.get_mut(each) {
                        surface
                            .drm_output
                            .with_compositor(|compositor| compositor.reset_buffer_ages());
                    }
                }
            }
            Step::ResetState => {
                for each in &released {
                    if let Some(surface) = device.surfaces.get_mut(each) {
                        surface.drm_output.with_compositor(|compositor| {
                            if let Err(err) = compositor.reset_state() {
                                warn!(output = %name, ?err, "cannot read the display's state back from the kernel");
                            }
                            compositor.reset_buffer_ages();
                        });
                    }
                }
            }
            Step::Relight => {
                for each in &released {
                    if let Some(surface) = device.surfaces.get_mut(each) {
                        relight_surface(surface);
                    }
                }
            }
            Step::ResetDevice => {
                // Everything off, including what no output of ours drives:
                // a CRTC left lit by someone else can hold the link or the
                // bandwidth an output needs.
                if let Err(err) = device.drm_output_manager.device_mut().reset_state() {
                    warn!(?err, "cannot switch every display on the GPU off");
                }
                for each in &released {
                    if let Some(surface) = device.surfaces.get_mut(each) {
                        relight_surface(surface);
                    }
                }
            }
        }

        // Lit one after the other, the most demanding mode first: the one
        // most likely to need what a less demanding one could do without.
        let mut order: Vec<(u64, crtc::Handle)> = released
            .iter()
            .filter_map(|each| {
                let surface = device.surfaces.get(each)?;
                let mode = surface.drm_output.with_compositor(|compositor| compositor.pending_mode());
                let (w, h) = mode.size();
                Some((u64::from(w) * u64::from(h) * u64::from(mode.vrefresh()), *each))
            })
            .collect();
        order.sort_by(|a, b| b.0.cmp(&a.0));
        self.mindbar.invalidate_graphics();
        for (_, each) in order {
            self.handle
                .insert_idle(move |data| data.render(node, Some(each), data.clock.now()));
        }
    }

    fn surface_dirty(&self, node: DrmNode, crtc: crtc::Handle) -> bool {
        self.backend_data
            .backends
            .get(&node)
            .and_then(|device| device.surfaces.get(&crtc))
            .is_some_and(|surface| surface.dirty)
    }

    /// Start the deadline for the flip just queued on one output.
    fn arm_surface_flip_deadline(&mut self, node: DrmNode, crtc: crtc::Handle, refresh: Option<i32>) {
        if let Some(surface) = self
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|device| device.surfaces.get_mut(&crtc))
        {
            arm_flip_deadline(&self.handle, surface, node, crtc, refresh);
        }
    }

    fn arm_surface_repaint(
        &mut self,
        node: DrmNode,
        crtc: crtc::Handle,
        delay: Duration,
        target: Option<Time<Monotonic>>,
        idle: bool,
    ) {
        if let Some(surface) = self
            .backend_data
            .backends
            .get_mut(&node)
            .and_then(|device| device.surfaces.get_mut(&crtc))
        {
            arm_repaint(&self.handle, surface, node, crtc, delay, target, idle);
        }
    }
}

/// What to do once a page flip has been answered for.
enum AfterFlip {
    /// Draw the next frame.
    Repaint,
    /// The session is paused; its resume handler repaints.
    Wait,
}

/// Ask outputs for a frame: `only` names one, `None` means all of them.
///
/// Scoping this matters more than it looks. Every client commit used to ask
/// every display to repaint, and a display with nothing on it that changed
/// still has to collect its render elements and work out that it has no
/// damage before it can say so. With a video playing on one screen and a
/// game on another, each was dragged through the other's frames as well as
/// its own, on the one thread that has to render them both.
fn repaint_outputs(state: &mut AnvilState<UdevData>, only: Option<UdevOutputId>) {
    for (node, device) in state.backend_data.backends.iter_mut() {
        // A modeset throws away every frame in flight on the device, not
        // just the one that asked, so it is always handled in full.
        let reconfigured = std::mem::take(&mut device.reconfigured);
        for (crtc, surface) in device.surfaces.iter_mut() {
            if !reconfigured
                && only.is_some_and(|id| id.device_id != *node || id.crtc != *crtc)
            {
                continue;
            }
            if reconfigured {
                // The device was modeset: every frame in flight on it is
                // gone, and waiting on one that is never coming is what
                // freezes a display.
                strand_frame(&state.handle, surface);
            }
            surface.dirty = true;
            let now = Instant::now();
            let pull_forward = match &surface.repaint_timer {
                // A frame in flight, a vblank held back, or a reset each
                // continue the loop themselves, and a frame queued behind
                // them would be refused or dropped.
                _ if surface.busy() => false,
                // Nothing armed and nothing else to continue the loop is a
                // stopped loop: this is the last place that can notice
                // before the watchdog does.
                None => true,
                // A frame the kernel refused is retried on its own backoff,
                // which a change must not cut short.
                Some(_) if surface.health.refused > 0 => false,
                // The idle tick, or a repaint aimed so far out that the
                // output is parked rather than running. A timer object is
                // not proof that a frame is coming: what matters is when.
                Some(armed) => armed.idle || armed.due.saturating_duration_since(now) > PULL_FORWARD_AFTER,
            };
            if pull_forward {
                // At the next turn of the loop, once this batch of events
                // has been handled -- but never more than once a frame.
                // Waiting out the rest of the frame costs the update at most
                // one refresh and gives that time back to the output that is
                // actually moving.
                let frame = state
                    .space
                    .outputs()
                    .find(|o| {
                        o.user_data().get::<UdevOutputId>()
                            == Some(&UdevOutputId {
                                device_id: *node,
                                crtc: *crtc,
                            })
                    })
                    .and_then(|o| o.current_mode())
                    .map(|mode| crate::timing::frame_duration(mode.refresh))
                    .unwrap_or_default();
                let delay = frame.saturating_sub(surface.last_render.elapsed());
                arm_repaint(&state.handle, surface, *node, *crtc, delay, None, false);
            }
        }
    }
}

/// What to do once a repaint is over. Every variant arms something, except
/// `Wait` and `Recovering`, which are for a loop deliberately stopped by
/// something that will start it again.
enum Repaint {
    /// A frame was queued. Its vblank continues the loop, and a deadline
    /// watches for the vblank that never comes.
    Flipped,
    /// The session is paused or the displays are off. Resuming repaints.
    Wait,
    /// Look again after about one frame.
    NextFrame,
    /// Nothing changes on its own: wait for a change.
    Idle,
    /// The kernel refused the frame: try again after this long.
    Retry(Duration),
    /// The output is being reset, which repaints when it is done.
    Recovering,
}

/// Switch one display off and give it a new mode blob, so its next frame
/// lights it with a full modeset: what unplugging the monitor and plugging
/// it back in would do.
fn relight_surface(surface: &mut SurfaceData) {
    let crtc = surface.drm_output.crtc();
    surface.drm_output.with_compositor(|compositor| {
        match compositor.clear() {
            // Nothing shows the buffers any more, so they can go.
            Ok(()) => compositor.reset_buffers(),
            Err(err) => warn!(?crtc, ?err, "cannot switch the display off"),
        }
        if let Err(err) = compositor.use_mode(compositor.pending_mode()) {
            warn!(?crtc, ?err, "cannot set the display's mode again");
        }
    });
}

#[allow(clippy::too_many_arguments)]
#[profiling::function]
fn render_surface<'a>(
    surface: &'a mut SurfaceData,
    renderer: &mut UdevRenderer<'a>,
    space: &Space<WindowElement>,
    output: &Output,
    pointer_location: Point<f64, Logical>,
    pointer_image: &MemoryRenderBuffer,
    pointer_hotspot: Point<i32, Logical>,
    pointer_element: &mut PointerElement,
    dnd_icon: &Option<DndIcon>,
    cursor_status: &mut CursorImageStatus,
    show_window_preview: bool,
    mindbar: &mut crate::mindbar::MindBar,
    show_wordmark: bool,
    locked: bool,
    capture: &mut crate::capture::CaptureState,
    time: Duration,
) -> Result<(bool, RenderElementStates), SwapBuffersError> {
    // An output the space has not placed yet has nowhere to draw. This is a
    // moment during a hotplug, not a reason to take the session down: asking
    // again on the next frame is what the caller does with this error.
    let Some(output_geometry) = space.output_geometry(output) else {
        return Err(SwapBuffersError::TemporaryFailure(Box::new(std::io::Error::other(
            "the output is not mapped",
        ))));
    };
    let scale = Scale::from(output.current_scale().fractional_scale());

    let mut custom_elements: Vec<CustomRenderElements<_>> = Vec::new();

    if output_geometry.to_f64().contains(pointer_location) {
        let cursor_hotspot = if let CursorImageStatus::Surface(ref surface) = cursor_status {
            // Read on every frame the pointer is over this output. A client
            // surface without a hotspot draws from its own corner rather
            // than taking the whole session down at 240 frames a second.
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

        // set cursor
        pointer_element.set_buffer(pointer_image.clone(), pointer_hotspot);

        // draw the cursor as relevant
        {
            // reset the cursor if the surface is no longer alive
            let mut reset = false;
            if let CursorImageStatus::Surface(ref surface) = *cursor_status {
                reset = !surface.alive();
            }
            if reset {
                *cursor_status = CursorImageStatus::default_named();
            }

            pointer_element.set_status(cursor_status.clone());
        }

        custom_elements.extend(
            pointer_element.render_elements(
                renderer,
                (cursor_pos - cursor_hotspot.to_f64())
                    .to_physical(scale)
                    .to_i32_round(),
                scale,
                1.0,
            ),
        );

        // draw the dnd icon if applicable
        {
            if let Some(icon) = dnd_icon.as_ref() {
                let dnd_icon_pos = (cursor_pos + icon.offset.to_f64())
                    .to_physical(scale)
                    .to_i32_round();
                if icon.surface.alive() {
                    custom_elements.extend(AsRenderElements::<UdevRenderer<'a>>::render_elements(
                        &SurfaceTree::from_surface(&icon.surface),
                        renderer,
                        dnd_icon_pos,
                        scale,
                        1.0,
                    ));
                }
            }
        }
    }

    #[cfg(feature = "debug")]
    if let Some(element) = surface.fps_element.as_mut() {
        element.update_fps(surface.fps.avg().round() as u32);
        surface.fps.tick();
        custom_elements.push(CustomRenderElements::Fps(element.clone()));
    }

    if !locked {
        if let Some(osd) = mindbar.render_osd(renderer, &output.name(), output_geometry.size, scale.x) {
            custom_elements.push(CustomRenderElements::Overlay(osd));
        }
    }
    if let Some(bar) = mindbar.render_element(renderer, &output.name(), output_geometry.size, scale.x) {
        custom_elements.push(CustomRenderElements::Overlay(bar));
    }
    // The startup screen goes away as soon as the shell's home screen is up,
    // not when a window opens -- and stays away while that surface moves
    // between layers, which is what Super+D does to it.
    let desktop_up = crate::shell::desktop_up(output);
    let backdrop: Vec<_> = mindbar
        .backdrop_elements(renderer, output_geometry.size, scale.x, desktop_up, show_wordmark)
        .into_iter()
        .map(CustomRenderElements::Overlay)
        .collect();

    let (elements, clear_color) =
        output_elements(output, space, custom_elements, renderer, show_window_preview, backdrop, locked);

    if !locked {
        capture.render(output, renderer, &elements, clear_color, time,
            |e| matches!(e, OutputRenderElements::Custom(CustomRenderElements::Pointer(_))));
    }

    // A game hands over 8-bit buffers while the swapchain is 10-bit: without
    // ALLOW_PRIMARY_PLANE_SCANOUT_ANY they never match the plane's current
    // format and every frame gets composed instead of scanned out.
    let frame_mode = match surface.scanout {
        DirectScanout::Any => FrameFlags::DEFAULT | FrameFlags::ALLOW_PRIMARY_PLANE_SCANOUT_ANY,
        DirectScanout::Matching => FrameFlags::DEFAULT,
        DirectScanout::Off => FrameFlags::empty(),
    };
    let (rendered, scanout, states) = surface
        .drm_output
        .render_frame(renderer, &elements, clear_color, frame_mode)
        .map(|render_frame_result| {
            // Which client element, if any, is going straight onto the
            // primary plane. Named here so a flip that never comes back can
            // say what was in front of the display controller.
            let scanout = match &render_frame_result.primary_element {
                smithay::backend::drm::compositor::PrimaryPlaneElement::Element(element) => {
                    Some(element.id().clone())
                }
                _ => None,
            };
            #[cfg(feature = "renderer_sync")]
            if let PrimaryPlaneElement::Swapchain(element) = render_frame_result.primary_element {
                element.sync.wait();
            }
            (!render_frame_result.is_empty, scanout, render_frame_result.states)
        })
        .map_err(|err| match err {
            smithay::backend::drm::compositor::RenderFrameError::PrepareFrame(err) => {
                SwapBuffersError::from(err)
            }
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                OutputDamageTrackerError::Rendering(err),
            ) => SwapBuffersError::from(err),
            // An output with no mode: a display part-way through being
            // reconfigured, unplugged, or woken. It gets a mode and a
            // repaint from the hotplug path; the frame is simply skipped.
            smithay::backend::drm::compositor::RenderFrameError::RenderFrame(
                OutputDamageTrackerError::OutputNoMode(err),
            ) => SwapBuffersError::TemporaryFailure(err.into()),
        })?;

    update_primary_scanout_output(space, output, dnd_icon, cursor_status, &states);

    if rendered {
        let output_presentation_feedback = take_presentation_feedback(output, space, &states);
        surface
            .drm_output
            .queue_frame(Some(output_presentation_feedback))
            .map_err(Into::<SwapBuffersError>::into)?;
        surface.flip_scanout = scanout;
    }

    Ok((rendered, states))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_vblank_time_from_the_future_is_not_believed() {
        let now = Duration::from_secs(1000);
        // What the hardware normally reports: a moment ago.
        assert_eq!(sane_vblank_time(now - Duration::from_millis(4), now), Some(now - Duration::from_millis(4)));
        // Read a hair after the event; still the truth.
        assert_eq!(sane_vblank_time(now + Duration::from_millis(1), now), Some(now + Duration::from_millis(1)));
        // Far ahead: believing this aims every frame after it further and
        // further out, and the output stops for as long as the driver was
        // wrong by.
        assert_eq!(sane_vblank_time(now + Duration::from_secs(60), now), None);
        // A timestamp from another epoch entirely.
        assert_eq!(sane_vblank_time(Duration::from_secs(3), now), None);
        assert_eq!(sane_vblank_time(Duration::ZERO, now), None);
    }

    #[test]
    fn an_output_that_has_not_drawn_is_dead_however_healthy_its_timers_look() {
        // The freeze that got past the old watchdog: a timer armed, not yet
        // due, and no frame on screen for seconds.
        let reason = stopped_reason(Duration::from_secs(5), Some(Duration::ZERO));
        assert!(reason.is_some(), "a display that has drawn nothing for five seconds is frozen");
        // The other one: no timer and no flip at all.
        assert!(stopped_reason(Duration::ZERO, None).is_some());
        // A timer long past due belongs to a loop that stopped.
        assert!(stopped_reason(Duration::ZERO, Some(REPAINT_OVERDUE + Duration::from_millis(1))).is_some());
        // And an output that is simply running is left alone: it drew a
        // moment ago and its next repaint is not due yet.
        assert!(stopped_reason(Duration::from_millis(20), Some(Duration::ZERO)).is_none());
        // The idle tick is a second, so an output ticking along at that rate
        // is never mistaken for a frozen one.
        assert!(stopped_reason(IDLE_REPAINT, Some(Duration::from_millis(10))).is_none());
    }

    #[test]
    fn no_repaint_is_ever_scheduled_past_the_point_the_watchdog_calls_dead() {
        // `arm_repaint` clamps every delay to `IDLE_REPAINT`, and the
        // watchdog resets an output whose timer is `REPAINT_OVERDUE` late.
        // If the clamp were the looser of the two, a legitimately scheduled
        // repaint could be reset out from under itself.
        assert!(REPAINT_OVERDUE >= IDLE_REPAINT);
        // And a change never waits longer than a frame or two to be drawn.
        assert!(PULL_FORWARD_AFTER < IDLE_REPAINT);
    }
}
