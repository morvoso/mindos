//! Output capture for screenshot tools and the desktop portal.
//!
//! The wlr protocol is retained for existing clients. Rendering, validation
//! and damage tracking are kept separate so newer capture protocols can use
//! the same output path. No rendering or readback is done without a request.

use crate::state::{AnvilState, Backend, ClientState};
use smithay::{
    backend::{
        allocator::Fourcc,
        renderer::{
            damage::OutputDamageTracker, element::RenderElement, gles::GlesRenderbuffer, Color32F,
            ErasedContextId, ExportMem, Offscreen, Renderer,
        },
    },
    output::Output,
    reexports::wayland_server::{
        protocol::{wl_buffer::WlBuffer, wl_output::WlOutput, wl_shm},
        Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
    },
    utils::{Buffer, Physical, Rectangle, Size, Transform},
    wayland::shm::{with_buffer_contents, with_buffer_contents_mut, BufferData},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use wayland_protocols_wlr::screencopy::v1::server::{
    zwlr_screencopy_frame_v1::{self as frame, ZwlrScreencopyFrameV1},
    zwlr_screencopy_manager_v1::{self as manager, ZwlrScreencopyManagerV1},
};

const MAX_PENDING: usize = 32;
const MAX_PIXELS: i64 = 64 * 1024 * 1024;
type Key = (String, bool);

#[derive(Debug, Default)]
pub struct ManagerData {
    last: Mutex<HashMap<Key, (u64, Rectangle<i32, Buffer>)>>,
}

#[derive(Debug, Clone)]
struct FrameInfo {
    output: Output,
    mode: Size<i32, Physical>,
    transform: Transform,
    scale: f64,
    region: Rectangle<i32, Buffer>,
    cursor: bool,
}

#[derive(Debug)]
pub struct FrameData {
    info: Option<FrameInfo>,
    manager: Arc<ManagerData>,
    used: Mutex<bool>,
}

#[derive(Debug)]
struct Pending {
    frame: ZwlrScreencopyFrameV1,
    buffer: WlBuffer,
    info: FrameInfo,
    manager: Arc<ManagerData>,
    damage: bool,
    since: Instant,
}

impl Pending {
    fn fail(self) {
        if self.buffer.is_alive() {
            self.buffer.release();
        }
        if self.frame.is_alive() {
            self.frame.failed();
        }
    }
}

#[derive(Debug)]
struct Cache {
    context: ErasedContextId,
    mode: Size<i32, Physical>,
    scale: f64,
    transform: Transform,
    target: GlesRenderbuffer,
    damage: OutputDamageTracker,
    generation: u64,
    last_used: Instant,
}

#[derive(Debug, Default)]
pub struct CaptureState {
    pending: Vec<Pending>,
    caches: HashMap<Key, Cache>,
    generation: u64,
}

/// Clip before doing i32 arithmetic: protocol coordinates are untrusted.
fn capture_region(
    mode: Size<i32, Physical>,
    scale: f64,
    transform: Transform,
    requested: Option<[i32; 4]>,
) -> Option<Rectangle<i32, Buffer>> {
    if mode.w <= 0
        || mode.h <= 0
        || i64::from(mode.w) * i64::from(mode.h) > MAX_PIXELS
        || !scale.is_finite()
        || scale <= 0.0
    {
        return None;
    }
    let oriented = transform.transform_size(mode);
    let bounds = (f64::from(oriented.w) / scale, f64::from(oriented.h) / scale);
    let (x, y, right, bottom) = match requested {
        Some([x, y, w, h]) if w > 0 && h > 0 => (
            f64::from(x).max(0.0),
            f64::from(y).max(0.0),
            (f64::from(x) + f64::from(w)).min(bounds.0),
            (f64::from(y) + f64::from(h)).min(bounds.1),
        ),
        Some(_) => return None,
        None => (0.0, 0.0, bounds.0, bounds.1),
    };
    if right <= x || bottom <= y {
        return None;
    }
    let left = (x * scale).floor() as i32;
    let top = (y * scale).floor() as i32;
    let right = (right * scale).ceil().min(f64::from(oriented.w)) as i32;
    let bottom = (bottom * scale).ceil().min(f64::from(oriented.h)) as i32;
    let rect =
        Rectangle::<i32, Physical>::new((left, top).into(), (right - left, bottom - top).into());
    let raw = transform.invert().transform_rect_in(rect, &oriented);
    Some(Rectangle::new(
        (raw.loc.x, raw.loc.y).into(),
        (raw.size.w, raw.size.h).into(),
    ))
}

fn valid_shm(data: BufferData, pool_len: usize, size: Size<i32, Buffer>) -> bool {
    let Some(bytes) = i64::from(size.w)
        .checked_mul(4)
        .and_then(|stride| stride.checked_mul(i64::from(size.h)))
    else {
        return false;
    };
    data.format == wl_shm::Format::Xrgb8888
        && data.width == size.w
        && data.height == size.h
        && i64::from(data.stride) == i64::from(size.w) * 4
        && data.offset >= 0
        && bytes > 0
        && (data.offset as u64)
            .checked_add(bytes as u64)
            .is_some_and(|end| end <= pool_len as u64)
}

pub fn register<B: Backend + 'static>(dh: &DisplayHandle) {
    dh.create_global::<AnvilState<B>, ZwlrScreencopyManagerV1, _>(3, ());
}

impl<B: Backend + 'static> GlobalDispatch<ZwlrScreencopyManagerV1, ()> for AnvilState<B> {
    fn bind(
        _: &mut Self,
        _: &DisplayHandle,
        _: &Client,
        resource: New<ZwlrScreencopyManagerV1>,
        _: &(),
        data: &mut DataInit<'_, Self>,
    ) {
        data.init(resource, Arc::new(ManagerData::default()));
    }

    fn can_view(client: Client, _: &()) -> bool {
        // Security-context clients must go through the portal's user consent flow.
        client
            .get_data::<ClientState>()
            .is_some_and(|s| s.security_context.is_none())
    }
}

impl<B: Backend + 'static> Dispatch<ZwlrScreencopyManagerV1, Arc<ManagerData>> for AnvilState<B> {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ZwlrScreencopyManagerV1,
        request: manager::Request,
        manager: &Arc<ManagerData>,
        _: &DisplayHandle,
        data: &mut DataInit<'_, Self>,
    ) {
        let (new, cursor, output, region) = match request {
            manager::Request::CaptureOutput {
                frame,
                overlay_cursor,
                output,
            } => (frame, overlay_cursor != 0, output, None),
            manager::Request::CaptureOutputRegion {
                frame,
                overlay_cursor,
                output,
                x,
                y,
                width,
                height,
            } => (
                frame,
                overlay_cursor != 0,
                output,
                Some([x, y, width, height]),
            ),
            _ => return,
        };
        let info = state.capture_info(&output, cursor, region);
        let object = data.init(
            new,
            FrameData {
                info: info.clone(),
                manager: manager.clone(),
                used: Mutex::new(info.is_none()),
            },
        );
        if let Some(info) = info {
            let size = info.region.size;
            object.buffer(
                wl_shm::Format::Xrgb8888,
                size.w as u32,
                size.h as u32,
                size.w as u32 * 4,
            );
            if object.version() >= 3 {
                object.buffer_done();
            }
        } else {
            object.failed();
        }
    }
}

impl<B: Backend + 'static> AnvilState<B> {
    fn capture_info(
        &self,
        resource: &WlOutput,
        cursor: bool,
        region: Option<[i32; 4]>,
    ) -> Option<FrameInfo> {
        if self.idle.locked
            || self.idle.stage == crate::idle::Stage::Blank
            || self.config.session.kiosk
        {
            return None;
        }
        let output = Output::from_resource(resource)?;
        if !self.space.outputs().any(|o| o == &output) {
            return None;
        }
        let mode = output.current_mode()?.size;
        let transform = output.current_transform();
        let scale = output.current_scale().fractional_scale();
        Some(FrameInfo {
            region: capture_region(mode, scale, transform, region)?,
            output,
            mode,
            transform,
            scale,
            cursor,
        })
    }
}

impl<B: Backend + 'static> Dispatch<ZwlrScreencopyFrameV1, FrameData> for AnvilState<B> {
    fn request(
        state: &mut Self,
        _: &Client,
        object: &ZwlrScreencopyFrameV1,
        request: frame::Request,
        data: &FrameData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        let (buffer, damage) = match request {
            frame::Request::Copy { buffer } => (buffer, false),
            frame::Request::CopyWithDamage { buffer } => (buffer, true),
            _ => return,
        };
        let mut used = data.used.lock().unwrap();
        if *used {
            object.post_error(
                frame::Error::AlreadyUsed,
                "capture frame has already been used",
            );
            return;
        }
        *used = true;
        let Some(info) = &data.info else {
            object.failed();
            return;
        };
        if !with_buffer_contents(&buffer, |_, len, b| valid_shm(b, len, info.region.size))
            .unwrap_or(false)
        {
            object.post_error(
                frame::Error::InvalidBuffer,
                "expected the advertised XRGB shared-memory buffer",
            );
            return;
        }
        if state.idle.locked
            || state.idle.stage == crate::idle::Stage::Blank
            || state.capture.pending.len() >= MAX_PENDING
        {
            object.failed();
            return;
        }
        state.capture.pending.push(Pending {
            frame: object.clone(),
            buffer,
            info: info.clone(),
            manager: data.manager.clone(),
            damage,
            since: Instant::now(),
        });
        // The copy is made during a repaint; an idle output would never make one.
        state.request_repaint();
    }
}

impl CaptureState {
    /// Called even with displays off so detached outputs and dead clients
    /// cannot keep requests or GPU allocations alive indefinitely.
    pub fn maintain(&mut self, outputs: &[Output], blocked: bool) {
        let mut keep = Vec::new();
        for pending in self.pending.drain(..) {
            if blocked
                || !pending.frame.is_alive()
                || !pending.buffer.is_alive()
                || !outputs.contains(&pending.info.output)
                || (!pending.damage && pending.since.elapsed() > Duration::from_secs(5))
            {
                pending.fail();
            } else {
                keep.push(pending);
            }
        }
        self.pending = keep;
        self.caches.retain(|(name, _), cache| {
            !blocked
                && outputs.iter().any(|o| o.name() == *name)
                && cache.last_used.elapsed() < Duration::from_secs(2)
        });
    }

    pub fn render<R, E>(
        &mut self,
        output: &Output,
        renderer: &mut R,
        elements: &[E],
        clear: Color32F,
        time: Duration,
        is_cursor: impl Fn(&E) -> bool,
    ) where
        R: Renderer + ExportMem + Offscreen<GlesRenderbuffer>,
        R::TextureId: 'static,
        E: RenderElement<R>,
    {
        for cursor in [false, true] {
            if !self
                .pending
                .iter()
                .any(|p| p.info.output == *output && p.info.cursor == cursor)
            {
                continue;
            }
            let key = (output.name(), cursor);
            let Some(mode) = output.current_mode().map(|m| m.size) else {
                continue;
            };
            let transform = output.current_transform();
            let scale = output.current_scale().fractional_scale();
            let context = renderer.context_id().erased();
            let (jobs, other): (Vec<_>, Vec<_>) = std::mem::take(&mut self.pending)
                .into_iter()
                .partition(|p| p.info.output == *output && p.info.cursor == cursor);
            self.pending = other;
            let jobs: Vec<_> = jobs
                .into_iter()
                .filter_map(|p| {
                    if p.info.mode != mode
                        || p.info.scale != scale
                        || p.info.transform != transform
                        || !p.frame.is_alive()
                        || !p.buffer.is_alive()
                    {
                        p.fail();
                        None
                    } else {
                        Some(p)
                    }
                })
                .collect();
            if jobs.is_empty() {
                continue;
            }
            let replace = self.caches.get(&key).is_none_or(|c| {
                c.context != context
                    || c.mode != mode
                    || c.transform != transform
                    || c.scale != scale
            });
            if replace {
                match renderer.create_buffer(Fourcc::Argb8888, (mode.w, mode.h).into()) {
                    Ok(target) => {
                        self.caches.insert(
                            key.clone(),
                            Cache {
                                context,
                                mode,
                                scale,
                                transform,
                                target,
                                damage: OutputDamageTracker::new(mode, scale, transform),
                                generation: 0,
                                last_used: Instant::now(),
                            },
                        );
                    }
                    Err(err) => {
                        tracing::warn!(?err, "cannot allocate capture target");
                        for p in jobs {
                            p.fail();
                        }
                        continue;
                    }
                }
            }
            let cache = self.caches.get_mut(&key).unwrap();
            cache.last_used = Instant::now();
            let mut fb = match renderer.bind(&mut cache.target) {
                Ok(fb) => fb,
                Err(err) => {
                    tracing::warn!(?err, "cannot bind capture target");
                    for p in jobs {
                        p.fail();
                    }
                    continue;
                }
            };
            let visible: Vec<_> = elements
                .iter()
                .filter(|e| cursor || !is_cursor(e))
                .collect();
            match cache
                .damage
                .render_output(renderer, &mut fb, 1, &visible, clear)
            {
                Ok(result) => {
                    if result.damage.is_some() {
                        self.generation = self.generation.wrapping_add(1);
                        cache.generation = self.generation;
                    }
                }
                Err(err) => {
                    tracing::warn!(?err, "cannot render capture target");
                    for p in jobs {
                        p.fail();
                    }
                    continue;
                }
            }
            for p in jobs {
                let previous = p.manager.last.lock().unwrap().get(&key).copied();
                if p.damage && previous == Some((cache.generation, p.info.region)) {
                    self.pending.push(p);
                    continue;
                }
                let copied = (|| {
                    let mapping = renderer
                        .copy_framebuffer(&fb, p.info.region, Fourcc::Argb8888)
                        .ok()?;
                    let bytes = renderer.map_texture(&mapping).ok()?;
                    let size = p.info.region.size;
                    let count = size.w as usize * size.h as usize * 4;
                    if bytes.len() < count {
                        return None;
                    }
                    let wrote = with_buffer_contents_mut(&p.buffer, |ptr, len, data| {
                        if !valid_shm(data, len, size) {
                            return false;
                        }
                        // The pool guard handles truncation/SIGBUS. No Rust
                        // reference is created into concurrently writable shm.
                        unsafe {
                            std::ptr::copy_nonoverlapping(
                                bytes.as_ptr(),
                                ptr.add(data.offset as usize),
                                count,
                            );
                        }
                        true
                    })
                    .ok()?;
                    wrote.then_some(())
                })();
                if copied.is_some() {
                    p.manager
                        .last
                        .lock()
                        .unwrap()
                        .insert(key.clone(), (cache.generation, p.info.region));
                    p.buffer.release();
                    // Smithay renders these offscreen targets with top-down rows.
                    // TextureMapping::flipped describes GL texture coordinates,
                    // not the row order of this compositor-rendered framebuffer.
                    p.frame.flags(frame::Flags::empty());
                    if p.damage {
                        p.frame.damage(
                            0,
                            0,
                            p.info.region.size.w as u32,
                            p.info.region.size.h as u32,
                        );
                    }
                    p.frame.ready(
                        (time.as_secs() >> 32) as u32,
                        time.as_secs() as u32,
                        time.subsec_nanos(),
                    );
                } else {
                    p.fail();
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regions_clip_before_scaling_and_rotation() {
        let mode = (1920, 1080).into();
        assert_eq!(
            capture_region(mode, 1.5, Transform::Normal, Some([-20, -10, 30, 20])),
            Some(Rectangle::new((0, 0).into(), (15, 15).into()))
        );
        assert_eq!(
            capture_region(mode, 1.0, Transform::_90, Some([10, 20, 100, 200])),
            Some(Rectangle::new((20, 970).into(), (200, 100).into()))
        );
        assert_eq!(
            capture_region(mode, 1.0, Transform::Flipped, Some([10, 20, 100, 200])),
            Some(Rectangle::new((1810, 20).into(), (100, 200).into()))
        );
    }

    #[test]
    fn full_capture_preserves_raw_mode_for_every_transform_and_scale() {
        for transform in [
            Transform::Normal,
            Transform::_90,
            Transform::_180,
            Transform::_270,
            Transform::Flipped,
            Transform::Flipped90,
            Transform::Flipped180,
            Transform::Flipped270,
        ] {
            for scale in [1.0, 1.25, 1.5, 2.0] {
                assert_eq!(
                    capture_region((2560, 1440).into(), scale, transform, None),
                    Some(Rectangle::from_size((2560, 1440).into()))
                );
            }
        }
    }

    #[test]
    fn hostile_or_empty_geometry_is_rejected() {
        for request in [
            [i32::MAX, 0, i32::MAX, 1],
            [i32::MIN, 0, i32::MAX, 1],
            [0, 0, -1, 30],
            [0, 0, 40, 0],
            [1920, 0, 1, 1],
        ] {
            assert!(
                capture_region((1920, 1080).into(), 1.0, Transform::Normal, Some(request))
                    .is_none()
            );
        }
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(capture_region((1920, 1080).into(), scale, Transform::Normal, None).is_none());
        }
        assert!(
            capture_region((i32::MAX, i32::MAX).into(), 1.0, Transform::Normal, None).is_none()
        );
    }

    #[test]
    fn shm_format_stride_offset_and_truncated_pool_are_checked() {
        let mut data = BufferData {
            offset: 8,
            width: 16,
            height: 8,
            stride: 64,
            format: wl_shm::Format::Xrgb8888,
        };
        assert!(valid_shm(data, 520, (16, 8).into()));
        assert!(!valid_shm(data, 519, (16, 8).into()));
        data.offset = -1;
        assert!(!valid_shm(data, 520, (16, 8).into()));
        data.offset = 8;
        data.stride = 65;
        assert!(!valid_shm(data, 600, (16, 8).into()));
        data.stride = 64;
        data.format = wl_shm::Format::Argb8888;
        assert!(!valid_shm(data, 520, (16, 8).into()));
    }
}
