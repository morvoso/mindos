use std::{
    borrow::Cow,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use smithay::{
    backend::renderer::{
        element::{
            memory::MemoryRenderBufferRenderElement, surface::WaylandSurfaceRenderElement, utils::CropRenderElement, AsRenderElements,
        },
        ImportAll, ImportMem, Renderer, Texture,
    },
    desktop::{
        space::SpaceElement, utils::OutputPresentationFeedback, Window, WindowSurface, WindowSurfaceType,
    },
    input::{
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent, GesturePinchBeginEvent,
            GesturePinchEndEvent, GesturePinchUpdateEvent, GestureSwipeBeginEvent, GestureSwipeEndEvent,
            GestureSwipeUpdateEvent, MotionEvent, PointerTarget, RelativeMotionEvent,
        },
        touch::TouchTarget,
        Seat,
    },
    output::Output,
    reexports::{
        wayland_protocols::wp::presentation_time::server::wp_presentation_feedback,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    render_elements,
    utils::{user_data::UserDataMap, IsAlive, Logical, Physical, Point, Rectangle, Scale, Serial},
    wayland::{compositor::SurfaceData as WlSurfaceData, dmabuf::DmabufFeedback, seat::WaylandFocus},
};

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::{compositor::with_states, shell::xdg::XdgToplevelSurfaceData};

use crate::{focus::PointerFocusTarget, state::Backend, AnvilState};

use super::{
    frame::{FrameStyle, SHADOW, TILE_SHADOW},
    ssd::{WindowState, RADIUS},
};

#[derive(Debug, Clone, PartialEq)]
pub struct WindowElement(pub Window);

/// The id a window has on the shell IPC, stable for the window's life.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(pub u64);

static NEXT_WINDOW_ID: AtomicU64 = AtomicU64::new(1);

impl WindowElement {
    /// The window's IPC id, assigned the first time it is asked for.
    pub fn id(&self) -> u64 {
        self.0
            .user_data()
            .insert_if_missing_threadsafe(|| WindowId(NEXT_WINDOW_ID.fetch_add(1, Ordering::Relaxed)));
        self.0.user_data().get::<WindowId>().map(|id| id.0).unwrap_or(0)
    }

    pub fn surface_under(
        &self,
        location: Point<f64, Logical>,
        window_type: WindowSurfaceType,
    ) -> Option<(PointerFocusTarget, Point<i32, Logical>)> {
        let header = self.header_height();
        if header > 0 && location.y < header as f64 {
            return Some((PointerFocusTarget::SSD(SSD(self.clone())), Point::default()));
        }
        let offset = Point::from((0, header));

        let surface_under = self.0.surface_under(location - offset.to_f64(), window_type);
        let (under, loc) = match self.0.underlying_surface() {
            WindowSurface::Wayland(_) => {
                surface_under.map(|(surface, loc)| (PointerFocusTarget::WlSurface(surface), loc))
            }
            #[cfg(feature = "xwayland")]
            WindowSurface::X11(s) => {
                surface_under.map(|(_, loc)| (PointerFocusTarget::X11Surface(s.clone()), loc))
            }
        }?;
        Some((under, loc + offset))
    }

    pub fn with_surfaces<F>(&self, processor: F)
    where
        F: FnMut(&WlSurface, &WlSurfaceData),
    {
        self.0.with_surfaces(processor);
    }

    pub fn send_frame<T, F>(
        &self,
        output: &Output,
        time: T,
        throttle: Option<Duration>,
        primary_scan_out_output: F,
    ) where
        T: Into<Duration>,
        F: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
    {
        self.0.send_frame(output, time, throttle, primary_scan_out_output)
    }

    pub fn send_dmabuf_feedback<'a, P, F>(
        &self,
        output: &Output,
        primary_scan_out_output: P,
        select_dmabuf_feedback: F,
    ) where
        P: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F: Fn(&WlSurface, &WlSurfaceData) -> &'a DmabufFeedback + Copy,
    {
        self.0
            .send_dmabuf_feedback(output, primary_scan_out_output, select_dmabuf_feedback)
    }

    pub fn take_presentation_feedback<F1, F2>(
        &self,
        output_feedback: &mut OutputPresentationFeedback,
        primary_scan_out_output: F1,
        presentation_feedback_flags: F2,
    ) where
        F1: FnMut(&WlSurface, &WlSurfaceData) -> Option<Output> + Copy,
        F2: FnMut(&WlSurface, &WlSurfaceData) -> wp_presentation_feedback::Kind + Copy,
    {
        self.0.take_presentation_feedback(
            output_feedback,
            primary_scan_out_output,
            presentation_feedback_flags,
        )
    }

    #[cfg(feature = "xwayland")]
    #[inline]
    pub fn is_x11(&self) -> bool {
        self.0.is_x11()
    }

    #[inline]
    pub fn is_wayland(&self) -> bool {
        self.0.is_wayland()
    }

    #[inline]
    pub fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.0.wl_surface()
    }

    #[inline]
    /// Ask the client to close this window (xdg `close` or X11 WM_DELETE_WINDOW).
    pub fn close(&self) {
        if let Some(toplevel) = self.0.toplevel() {
            toplevel.send_close();
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = self.0.x11_surface() {
            let _ = surface.close();
        }
    }

    pub fn user_data(&self) -> &UserDataMap {
        self.0.user_data()
    }

    /// The window's title as the client last set it.
    pub fn title(&self) -> String {
        if let Some(toplevel) = self.0.toplevel() {
            return with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().and_then(|d| d.title.clone()))
            })
            .unwrap_or_default();
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = self.0.x11_surface() {
            return surface.title();
        }
        String::new()
    }

    /// The window's app id (xdg) or class (X11).
    pub fn app_id(&self) -> String {
        if let Some(toplevel) = self.0.toplevel() {
            return with_states(toplevel.wl_surface(), |states| {
                states
                    .data_map
                    .get::<XdgToplevelSurfaceData>()
                    .and_then(|d| d.lock().ok().and_then(|d| d.app_id.clone()))
            })
            .unwrap_or_default();
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = self.0.x11_surface() {
            return surface.class();
        }
        String::new()
    }

    pub fn is_fullscreen(&self) -> bool {
        if let Some(toplevel) = self.0.toplevel() {
            return toplevel
                .current_state()
                .states
                .contains(xdg_toplevel::State::Fullscreen);
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = self.0.x11_surface() {
            return surface.is_fullscreen();
        }
        false
    }

    pub fn is_maximized(&self) -> bool {
        if let Some(toplevel) = self.0.toplevel() {
            return toplevel
                .current_state()
                .states
                .contains(xdg_toplevel::State::Maximized);
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = self.0.x11_surface() {
            return surface.is_maximized();
        }
        false
    }

    /// Maximised or fullscreen as far as the *pending* state goes (what the
    /// next configure will tell the client).
    pub fn pending_maximized(&self) -> bool {
        if let Some(toplevel) = self.0.toplevel() {
            return toplevel.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Maximized));
        }
        self.is_maximized()
    }

    pub fn pending_fullscreen(&self) -> bool {
        if let Some(toplevel) = self.0.toplevel() {
            return toplevel.with_pending_state(|s| s.states.contains(xdg_toplevel::State::Fullscreen));
        }
        self.is_fullscreen()
    }

    /// Dialogs, utility windows, transient windows: never tiled, kept
    /// floating and centred.
    pub fn is_dialog(&self) -> bool {
        if let Some(toplevel) = self.0.toplevel() {
            return toplevel.parent().is_some();
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = self.0.x11_surface() {
            use smithay::xwayland::xwm::WmWindowType;
            return surface.is_popup()
                || surface.is_override_redirect()
                || surface.is_transient_for().is_some()
                || !matches!(surface.window_type(), None | Some(WmWindowType::Normal));
        }
        false
    }
}

impl IsAlive for WindowElement {
    #[inline]
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SSD(WindowElement);

impl IsAlive for SSD {
    #[inline]
    fn alive(&self) -> bool {
        self.0.alive()
    }
}

impl WaylandFocus for SSD {
    #[inline]
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        self.0.wl_surface()
    }
}

impl<BackendData: Backend> PointerTarget<AnvilState<BackendData>> for SSD {
    fn enter(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        event: &MotionEvent,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
        }
    }
    fn motion(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        event: &MotionEvent,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
        }
    }
    fn relative_motion(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &RelativeMotionEvent,
    ) {
    }
    fn button(
        &self,
        seat: &Seat<AnvilState<BackendData>>,
        data: &mut AnvilState<BackendData>,
        event: &ButtonEvent,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            let pressed = event.state == smithay::backend::input::ButtonState::Pressed;
            state
                .header_bar
                .button(seat, data, &self.0, event.serial, event.button, pressed);
        }
    }
    fn axis(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _frame: AxisFrame,
    ) {
    }
    fn frame(&self, _seat: &Seat<AnvilState<BackendData>>, _data: &mut AnvilState<BackendData>) {}
    fn leave(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _serial: Serial,
        _time: u32,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_leave();
        }
    }
    fn gesture_swipe_begin(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GestureSwipeBeginEvent,
    ) {
    }
    fn gesture_swipe_update(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GestureSwipeUpdateEvent,
    ) {
    }
    fn gesture_swipe_end(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GestureSwipeEndEvent,
    ) {
    }
    fn gesture_pinch_begin(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GesturePinchBeginEvent,
    ) {
    }
    fn gesture_pinch_update(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GesturePinchUpdateEvent,
    ) {
    }
    fn gesture_pinch_end(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GesturePinchEndEvent,
    ) {
    }
    fn gesture_hold_begin(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GestureHoldBeginEvent,
    ) {
    }
    fn gesture_hold_end(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &GestureHoldEndEvent,
    ) {
    }
}

impl<BackendData: Backend> TouchTarget<AnvilState<BackendData>> for SSD {
    fn down(
        &self,
        seat: &Seat<AnvilState<BackendData>>,
        data: &mut AnvilState<BackendData>,
        event: &smithay::input::touch::DownEvent,
        _seq: Serial,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
            state.header_bar.touch_down(seat, data, &self.0, event.serial);
        }
    }

    fn up(
        &self,
        seat: &Seat<AnvilState<BackendData>>,
        data: &mut AnvilState<BackendData>,
        event: &smithay::input::touch::UpEvent,
        _seq: Serial,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.touch_up(seat, data, &self.0, event.serial);
        }
    }

    fn motion(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        event: &smithay::input::touch::MotionEvent,
        _seq: Serial,
    ) {
        let mut state = self.0.decoration_state();
        if state.is_ssd {
            state.header_bar.pointer_enter(event.location);
        }
    }

    fn frame(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _seq: Serial,
    ) {
    }

    fn cancel(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _seq: Serial,
    ) {
    }

    fn shape(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &smithay::input::touch::ShapeEvent,
        _seq: Serial,
    ) {
    }

    fn orientation(
        &self,
        _seat: &Seat<AnvilState<BackendData>>,
        _data: &mut AnvilState<BackendData>,
        _event: &smithay::input::touch::OrientationEvent,
        _seq: Serial,
    ) {
    }
}

impl SpaceElement for WindowElement {
    fn geometry(&self) -> Rectangle<i32, Logical> {
        let mut geo = SpaceElement::geometry(&self.0);
        geo.size.h += self.header_height();
        geo
    }
    fn bbox(&self) -> Rectangle<i32, Logical> {
        let mut bbox = SpaceElement::bbox(&self.0);
        bbox.size.h += self.header_height();
        bbox
    }
    fn is_in_input_region(&self, point: &Point<f64, Logical>) -> bool {
        if self.tile().clip.get().is_some_and(|clip| !clip.to_f64().contains(*point)) { return false; }
        let header = self.header_height();
        if header > 0 {
            point.y < header as f64
                || SpaceElement::is_in_input_region(&self.0, &(*point - Point::from((0.0, header as f64))))
        } else {
            SpaceElement::is_in_input_region(&self.0, point)
        }
    }
    fn z_index(&self) -> u8 {
        SpaceElement::z_index(&self.0)
    }

    fn set_activate(&self, activated: bool) {
        SpaceElement::set_activate(&self.0, activated);
    }
    fn output_enter(&self, output: &Output, overlap: Rectangle<i32, Logical>) {
        SpaceElement::output_enter(&self.0, output, overlap);
    }
    fn output_leave(&self, output: &Output) {
        SpaceElement::output_leave(&self.0, output);
    }
    #[profiling::function]
    fn refresh(&self) {
        SpaceElement::refresh(&self.0);
    }
}

render_elements!(
    pub WindowRenderElement<R> where R: ImportAll + ImportMem;
    Window=WaylandSurfaceRenderElement<R>,
    Decoration=MemoryRenderBufferRenderElement<R>,
    CroppedWindow=CropRenderElement<WaylandSurfaceRenderElement<R>>,
    CroppedDecoration=CropRenderElement<MemoryRenderBufferRenderElement<R>>,
);

impl<R: Renderer> std::fmt::Debug for WindowRenderElement<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Window(arg0) => f.debug_tuple("Window").field(arg0).finish(),
            Self::Decoration(arg0) => f.debug_tuple("Decoration").field(arg0).finish(),
            Self::CroppedWindow(arg0) => f.debug_tuple("CroppedWindow").field(arg0).finish(),
            Self::CroppedDecoration(arg0) => f.debug_tuple("CroppedDecoration").field(arg0).finish(),
            Self::_GenericCatcher(arg0) => f.debug_tuple("_GenericCatcher").field(arg0).finish(),
        }
    }
}

impl<R> AsRenderElements<R> for WindowElement
where
    R: Renderer + ImportAll + ImportMem,
    R::TextureId: Clone + Send + Texture + 'static,
{
    type RenderElement = WindowRenderElement<R>;

    fn render_elements<C: From<Self::RenderElement>>(
        &self,
        renderer: &mut R,
        mut location: Point<i32, Physical>,
        scale: Scale<f64>,
        alpha: f32,
    ) -> Vec<C> {
        let clip = self.tile().clip.get().map(|r| {
            let mut r = r.to_physical_precise_round(scale);
            r.loc += location;
            r
        });
        let crop = |element: WindowRenderElement<R>| -> Option<C> {
            let Some(rect) = clip else { return Some(C::from(element)); };
            match element {
                WindowRenderElement::Window(e) => CropRenderElement::from_element(e, scale, rect).map(WindowRenderElement::CroppedWindow).map(C::from),
                WindowRenderElement::Decoration(e) => CropRenderElement::from_element(e, scale, rect).map(WindowRenderElement::CroppedDecoration).map(C::from),
                other => Some(C::from(other)),
            }
        };
        let window_bbox = SpaceElement::bbox(&self.0);
        let header = self.header_height();

        if header > 0 && !window_bbox.is_empty() {
            let window_geo = SpaceElement::geometry(&self.0);

            let mut state = self.decoration_state();
            let width = window_geo.size.w;
            let int_scale = scale.x.ceil().max(1.0) as i32;
            state.header_bar.redraw(width, int_scale);
            let mut vec = AsRenderElements::<R>::render_elements::<WindowRenderElement<R>>(
                &state.header_bar,
                renderer,
                location,
                scale,
                alpha,
            );

            let top = location;
            location.y += (scale.y * header as f64).round() as i32;

            let window_elements =
                AsRenderElements::render_elements(&self.0, renderer, location, scale, alpha);
            vec.extend(window_elements);

            // The frame (shadow and border) goes under everything. A
            // maximised window touches the edges of its area and gets none.
            let bar = &state.header_bar;
            if !bar.is_maximized() {
                let style = FrameStyle {
                    focused: bar.is_focused(),
                    radius: RADIUS,
                    shadow: if bar.is_tiled() { TILE_SHADOW } else { SHADOW },
                };
                let size = (window_geo.size.w, window_geo.size.h + header).into();
                let WindowState { frame, .. } = &mut *state;
                vec.extend(
                    frame
                        .render_elements(renderer, top, size, scale, style, alpha)
                        .into_iter()
                        .map(WindowRenderElement::Decoration),
                );
            }
            vec.into_iter().filter_map(crop).collect()
        } else {
            AsRenderElements::render_elements::<WindowRenderElement<R>>(&self.0, renderer, location, scale, alpha)
                .into_iter()
                .filter_map(crop)
                .collect()
        }
    }
}
