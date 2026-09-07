//! Window layout modes.
//!
//! * `floating` (KDE-like): windows keep their own size, open centred on the
//!   output under the pointer, and can be maximised to the usable area.
//! * `dwindle` (Hyprland-like): every window is a tile; a new window splits
//!   the remaining space along its longer side, so the layout spirals.
//! * `columns` (Niri-like): every window is a full-height column of a chosen
//!   width laid out left to right on an infinite strip; the view scrolls so
//!   the focused column stays visible.
//!
//! Dialogs (transient windows) always float and are centred. In the tiling
//! modes a maximised window covers the usable area until it is unmaximised,
//! fullscreen windows own their output as usual, and dragging a tile by its
//! title bar swaps it with the tile it is dropped on. The arrangement is
//! recomputed once per event-loop turn when something changed
//! ([`AnvilState::layout_refresh`]), which keeps every handler simple: they
//! just set `layout.dirty`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use smithay::{
    output::Output,
    reexports::wayland_protocols::xdg::shell::server::xdg_toplevel,
    utils::{IsAlive, Logical, Point, Rectangle, Size},
};

use crate::{
    config::LayoutConfig,
    shell::{usable_area, WindowElement},
    state::{AnvilState, Backend},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LayoutMode {
    #[default]
    Floating,
    Dwindle,
    Columns,
}

impl LayoutMode {
    pub const ALL: [LayoutMode; 3] = [LayoutMode::Floating, LayoutMode::Dwindle, LayoutMode::Columns];

    pub fn parse(s: &str) -> Option<LayoutMode> {
        match s.trim().to_ascii_lowercase().as_str() {
            "floating" | "float" | "kde" | "stacking" => Some(LayoutMode::Floating),
            "dwindle" | "tiles" | "tiling" | "hyprland" => Some(LayoutMode::Dwindle),
            "columns" | "scroll" | "scrolling" | "niri" => Some(LayoutMode::Columns),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            LayoutMode::Floating => "floating",
            LayoutMode::Dwindle => "dwindle",
            LayoutMode::Columns => "columns",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            LayoutMode::Floating => "Floating",
            LayoutMode::Dwindle => "Tiles",
            LayoutMode::Columns => "Columns",
        }
    }

    pub fn next(self) -> LayoutMode {
        match self {
            LayoutMode::Floating => LayoutMode::Dwindle,
            LayoutMode::Dwindle => LayoutMode::Columns,
            LayoutMode::Columns => LayoutMode::Floating,
        }
    }

    pub fn is_tiling(self) -> bool {
        !matches!(self, LayoutMode::Floating)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Column width presets (fraction of the usable width), cycled with Super+R.
const COLUMN_WIDTHS: [f32; 4] = [1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0];
const DEFAULT_COLUMN_WIDTH: f32 = 0.5;
const MIN_TILE: i32 = 120;

/// Per-window layout data, kept in the window's user data.
#[derive(Debug, Default)]
pub struct TileData {
    /// Taken out of the tiling by the user (Super+Shift+F).
    pub floating: Cell<bool>,
    /// Column width as a fraction of the usable width (0 = default).
    pub width: Cell<f32>,
    /// Geometry before the window was maximised (floating mode restores it).
    pub saved: RefCell<Option<Rectangle<i32, Logical>>>,
    /// Geometry before the window became a tile; it goes back there when the
    /// desktop returns to floating or the window is taken out of the tiling.
    pub untiled: RefCell<Option<Rectangle<i32, Logical>>>,
    /// The layout has this window in a tile slot right now.
    pub tiled_now: Cell<bool>,
}

impl WindowElement {
    pub fn tile(&self) -> &TileData {
        self.user_data().insert_if_missing(TileData::default);
        self.user_data().get::<TileData>().unwrap()
    }

    /// A window the tiling modes manage: not a dialog, not floating.
    pub fn tileable(&self) -> bool {
        !self.is_dialog() && !self.tile().floating.get()
    }
}

/// The tiles of one output, in layout order, plus the strip scroll offset
/// of the columns mode.
#[derive(Debug, Default)]
pub struct OutputTiling {
    pub order: Vec<WindowElement>,
    /// Where the strip should be (the focused column fully in view).
    pub scroll: i32,
    /// The strip sliding towards `scroll`; the layout is recomputed every
    /// frame until it arrives.
    pub anim: Option<ScrollAnim>,
}

/// How long the columns strip takes to slide to a newly focused column.
pub const SCROLL_ANIM: Duration = Duration::from_millis(260);

/// A strip scroll in flight: eases out from `from` to `to`.
#[derive(Debug, Clone, Copy)]
pub struct ScrollAnim {
    pub from: i32,
    pub to: i32,
    pub start: Instant,
}

impl ScrollAnim {
    pub fn value(&self, now: Instant) -> i32 {
        let t = (now.saturating_duration_since(self.start).as_secs_f32() / SCROLL_ANIM.as_secs_f32()).clamp(0.0, 1.0);
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + ((self.to - self.from) as f32 * eased).round() as i32
    }

    pub fn done(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.start) >= SCROLL_ANIM
    }
}

#[derive(Debug)]
pub struct LayoutState {
    pub mode: LayoutMode,
    pub gap: i32,
    pub outer_gap: i32,
    open_maximized: bool,
    pub outputs: HashMap<String, OutputTiling>,
    /// Re-arrange on the next event-loop turn.
    pub dirty: bool,
    /// A window being moved with the pointer: left where the pointer put it.
    pub dragging: Option<WindowElement>,
    /// New windows and the window that was focused when they opened; they
    /// join the tiling right after it.
    pending: Vec<(WindowElement, Option<WindowElement>)>,
    /// The focused window at the last refresh: a focus change scrolls the
    /// columns strip.
    last_focus: Option<WindowElement>,
}

impl LayoutState {
    pub fn new(mode: LayoutMode, cfg: &LayoutConfig) -> LayoutState {
        LayoutState {
            mode,
            gap: cfg.gap.clamp(0, 64),
            outer_gap: cfg.outer_gap.clamp(0, 64),
            open_maximized: cfg.open_maximized,
            outputs: HashMap::new(),
            dirty: true,
            dragging: None,
            pending: Vec::new(),
            last_focus: None,
        }
    }

    /// A strip is still sliding on some output.
    pub fn animating(&self) -> bool {
        self.outputs.values().any(|t| t.anim.is_some())
    }

    /// Floating mode with `open_maximized = true`: new windows fill the
    /// usable area (the original MindOS "game mode").
    pub fn open_maximized(&self) -> bool {
        self.mode == LayoutMode::Floating && self.open_maximized
    }

    /// Note a new window; `after` is the window that had the focus.
    pub fn window_opened(&mut self, window: &WindowElement, after: Option<WindowElement>) {
        self.pending.push((window.clone(), after));
        self.dirty = true;
    }

    pub fn is_tiled(&self, window: &WindowElement) -> bool {
        self.mode.is_tiling()
            && window.tileable()
            && self.outputs.values().any(|t| t.order.contains(window))
    }

    fn tiling_of(&mut self, window: &WindowElement) -> Option<&mut OutputTiling> {
        self.outputs.values_mut().find(|t| t.order.contains(window))
    }

    /// Exchange the slots of two tiles (they may be on different outputs).
    pub fn swap(&mut self, a: &WindowElement, b: &WindowElement) {
        if a == b {
            return;
        }
        let pos = |t: &OutputTiling, w: &WindowElement| t.order.iter().position(|x| x == w);
        let mut found: Vec<(String, usize)> = Vec::new();
        for (name, t) in &self.outputs {
            if let Some(i) = pos(t, a) {
                found.push((name.clone(), i));
            }
            if let Some(i) = pos(t, b) {
                found.push((name.clone(), i));
            }
        }
        if found.len() != 2 {
            return;
        }
        let (na, ia) = found[0].clone();
        let (nb, ib) = found[1].clone();
        let wa = self.outputs[&na].order[ia].clone();
        let wb = self.outputs[&nb].order[ib].clone();
        self.outputs.get_mut(&na).unwrap().order[ia] = wb;
        self.outputs.get_mut(&nb).unwrap().order[ib] = wa;
        self.dirty = true;
    }

    /// Move a tile one slot towards the start or the end of its output's order.
    pub fn shift(&mut self, window: &WindowElement, towards_end: bool) -> bool {
        let Some(t) = self.tiling_of(window) else {
            return false;
        };
        let Some(i) = t.order.iter().position(|w| w == window) else {
            return false;
        };
        let j = if towards_end { i + 1 } else { i.wrapping_sub(1) };
        if j >= t.order.len() {
            return false;
        }
        t.order.swap(i, j);
        self.dirty = true;
        true
    }

    /// The next column width preset for a window.
    pub fn cycle_width(&mut self, window: &WindowElement) {
        let current = window.tile().width.get();
        let current = if current <= 0.0 { DEFAULT_COLUMN_WIDTH } else { current };
        let idx = COLUMN_WIDTHS
            .iter()
            .position(|w| (w - current).abs() < 0.01)
            .map(|i| (i + 1) % COLUMN_WIDTHS.len())
            .unwrap_or(1);
        window.tile().width.set(COLUMN_WIDTHS[idx]);
        self.dirty = true;
    }
}

/// The area a maximised or tiled client gets below a title bar of `header` px.
pub fn client_rect(area: Rectangle<i32, Logical>, header: i32) -> Rectangle<i32, Logical> {
    Rectangle::new(
        area.loc + Point::from((0, header)),
        (area.size.w.max(1), (area.size.h - header).max(1)).into(),
    )
}

/// Dwindle: window `i` takes half of what is left (split along the longer
/// side), the rest goes to the windows after it.
pub fn dwindle_rects(area: Rectangle<i32, Logical>, n: usize, gap: i32) -> Vec<Rectangle<i32, Logical>> {
    let mut out = Vec::with_capacity(n);
    let mut rest = area;
    for i in 0..n {
        if i + 1 == n {
            out.push(rest);
            break;
        }
        if rest.size.w >= rest.size.h {
            let w1 = ((rest.size.w - gap) / 2).max(MIN_TILE);
            out.push(Rectangle::new(rest.loc, (w1, rest.size.h).into()));
            rest = Rectangle::new(
                (rest.loc.x + w1 + gap, rest.loc.y).into(),
                ((rest.size.w - w1 - gap).max(MIN_TILE), rest.size.h).into(),
            );
        } else {
            let h1 = ((rest.size.h - gap) / 2).max(MIN_TILE);
            out.push(Rectangle::new(rest.loc, (rest.size.w, h1).into()));
            rest = Rectangle::new(
                (rest.loc.x, rest.loc.y + h1 + gap).into(),
                (rest.size.w, (rest.size.h - h1 - gap).max(MIN_TILE)).into(),
            );
        }
    }
    out
}

/// Columns: full-height columns of the given width fractions on a strip
/// starting at `area.loc.x - scroll`.
pub fn column_rects(
    area: Rectangle<i32, Logical>,
    widths: &[f32],
    gap: i32,
    scroll: i32,
) -> Vec<Rectangle<i32, Logical>> {
    let mut out = Vec::with_capacity(widths.len());
    let mut x = area.loc.x - scroll;
    for frac in widths {
        let w = column_width(area.size.w, *frac, gap);
        out.push(Rectangle::new((x, area.loc.y).into(), (w, area.size.h).into()));
        x += w + gap;
    }
    out
}

fn column_width(area_w: i32, frac: f32, gap: i32) -> i32 {
    let frac = if frac <= 0.0 { DEFAULT_COLUMN_WIDTH } else { frac.clamp(0.1, 1.0) };
    if frac >= 0.999 {
        area_w.max(MIN_TILE)
    } else {
        (((area_w - gap) as f32) * frac).round().max(MIN_TILE as f32) as i32
    }
}

/// Scroll offset that keeps column `focused` inside the area.
fn scroll_to_show(area_w: i32, widths: &[f32], gap: i32, scroll: i32, focused: Option<usize>) -> i32 {
    let total: i32 = widths.iter().map(|f| column_width(area_w, *f, gap)).sum::<i32>()
        + gap * (widths.len().saturating_sub(1)) as i32;
    if total <= area_w {
        return 0;
    }
    let mut scroll = scroll.clamp(0, total - area_w);
    if let Some(f) = focused {
        let mut x = 0;
        for (i, frac) in widths.iter().enumerate() {
            let w = column_width(area_w, *frac, gap);
            if i == f {
                if x < scroll {
                    scroll = x;
                } else if x + w > scroll + area_w {
                    scroll = x + w - area_w;
                }
                break;
            }
            x += w + gap;
        }
    }
    scroll.clamp(0, total - area_w)
}

/// The nearest window in `dir` from `from`, judged by rectangle centres.
pub fn neighbour_in<'a, T>(
    from: Rectangle<i32, Logical>,
    dir: Direction,
    candidates: impl Iterator<Item = (&'a T, Rectangle<i32, Logical>)>,
) -> Option<&'a T> {
    let c = from.loc + Point::from((from.size.w / 2, from.size.h / 2));
    let mut best: Option<(i64, &T)> = None;
    for (w, rect) in candidates {
        let o = rect.loc + Point::from((rect.size.w / 2, rect.size.h / 2));
        let (dx, dy) = ((o.x - c.x) as i64, (o.y - c.y) as i64);
        let (primary, secondary) = match dir {
            Direction::Left => (-dx, dy.abs()),
            Direction::Right => (dx, dy.abs()),
            Direction::Up => (-dy, dx.abs()),
            Direction::Down => (dy, dx.abs()),
        };
        if primary <= 0 {
            continue;
        }
        let score = primary + secondary * 2;
        if best.map(|(s, _)| score < s).unwrap_or(true) {
            best = Some((score, w));
        }
    }
    best.map(|(_, w)| w)
}

impl<BackendData: Backend> AnvilState<BackendData> {
    /// Once per event-loop turn: drop dead tiles, arrange when needed.
    pub fn layout_refresh(&mut self) {
        let mut changed = false;
        for tiling in self.layout.outputs.values_mut() {
            let before = tiling.order.len();
            tiling.order.retain(|w| w.alive());
            changed |= tiling.order.len() != before;
        }
        if let Some(w) = &self.layout.dragging {
            if !w.alive() {
                self.layout.dragging = None;
            }
        }
        // Columns follow the focus: a click into a half-visible column, the
        // dock or Super+arrows slide the strip so the focused column is
        // fully in view.
        let focused = self.focused_window();
        if focused != self.layout.last_focus {
            self.layout.last_focus = focused;
            if self.layout.mode == LayoutMode::Columns {
                self.layout.dirty = true;
            }
        }
        if self.layout.dirty || changed || self.layout.animating() {
            self.layout.dirty = false;
            self.arrange_all();
        }
    }

    pub fn arrange_all(&mut self) {
        let outputs: Vec<Output> = self.space.outputs().cloned().collect();
        for output in outputs {
            self.arrange_output(&output);
        }
    }

    /// The output a window belongs to (the first one it overlaps).
    pub fn window_home(&self, window: &WindowElement) -> Option<Output> {
        self.space
            .outputs_for_element(window)
            .into_iter()
            .next()
            .or_else(|| self.space.outputs().next().cloned())
    }

    /// Lay out every window on `output` for the current mode.
    pub fn arrange_output(&mut self, output: &Output) {
        let Some(area) = usable_area(&self.space, output) else {
            return;
        };
        let mode = self.layout.mode;
        let gap = self.layout.gap;
        let outer = self.layout.outer_gap;
        let focused = self.focused_window();
        let dragging = self.layout.dragging.clone();

        // Windows that live on this output, bottom to top.
        let windows: Vec<WindowElement> = self
            .space
            .elements()
            .filter(|w| self.window_home(w).as_ref() == Some(output))
            .cloned()
            .collect();
        let mut handled: Vec<WindowElement> = Vec::new();
        let mut targets: Vec<(WindowElement, Point<i32, Logical>)> = Vec::new();

        if mode.is_tiling() {
            let name = output.name();
            // Maintain the order: new tiles go after the focused window.
            let pending = std::mem::take(&mut self.layout.pending);
            let tiling = self.layout.outputs.entry(name).or_default();
            tiling.order.retain(|w| w.tileable());
            for (w, after) in pending {
                if !windows.contains(&w) || !w.tileable() || tiling.order.contains(&w) {
                    if !windows.contains(&w) && w.alive() {
                        // opened on another output; it will be picked up there
                        self.layout.pending.push((w, after));
                    }
                    continue;
                }
                let at = after
                    .and_then(|a| tiling.order.iter().position(|x| *x == a))
                    .map(|i| i + 1)
                    .unwrap_or(tiling.order.len());
                tiling.order.insert(at, w);
            }
            for w in &windows {
                if w.tileable() && !tiling.order.contains(w) {
                    tiling.order.push(w.clone());
                }
            }
            let tiles: Vec<WindowElement> = tiling
                .order
                .iter()
                .filter(|w| windows.contains(w) && !w.pending_fullscreen())
                .cloned()
                .collect();
            let inner = Rectangle::new(
                area.loc + Point::from((outer, outer)),
                ((area.size.w - 2 * outer).max(MIN_TILE), (area.size.h - 2 * outer).max(MIN_TILE)).into(),
            );
            let rects = match mode {
                LayoutMode::Dwindle => dwindle_rects(inner, tiles.len(), gap),
                LayoutMode::Columns => {
                    let widths: Vec<f32> = tiles.iter().map(|w| w.tile().width.get()).collect();
                    let focused_idx = focused.as_ref().and_then(|f| tiles.iter().position(|w| w == f));
                    let target = scroll_to_show(inner.size.w, &widths, gap, tiling.scroll, focused_idx);
                    let now = Instant::now();
                    if target != tiling.scroll {
                        // Slide from wherever the strip is right now.
                        let from = tiling.anim.map(|a| a.value(now)).unwrap_or(tiling.scroll);
                        tiling.anim = (from != target).then_some(ScrollAnim { from, to: target, start: now });
                        tiling.scroll = target;
                    }
                    let visual = match tiling.anim {
                        Some(anim) if !anim.done(now) => anim.value(now),
                        _ => {
                            tiling.anim = None;
                            tiling.scroll
                        }
                    };
                    column_rects(inner, &widths, gap, visual)
                }
                LayoutMode::Floating => Vec::new(),
            };
            for (w, rect) in tiles.iter().zip(rects) {
                handled.push(w.clone());
                if dragging.as_ref() == Some(w) {
                    continue;
                }
                self.remember_untiled(w);
                let (rect, tiled) = if w.pending_maximized() { (area, false) } else { (rect, true) };
                let loc = self.apply_rect(w, rect, tiled);
                targets.push((w.clone(), loc));
            }
        }

        for w in &windows {
            if handled.contains(w) || dragging.as_ref() == Some(w) || w.pending_fullscreen() {
                continue;
            }
            if w.pending_maximized() {
                let loc = self.apply_rect(w, area, false);
                targets.push((w.clone(), loc));
            } else if let Some(loc) = self.ensure_untiled(w) {
                targets.push((w.clone(), loc));
            }
        }

        // Move what has to move, keeping the stacking order: `map_element`
        // raises, so every window of this output is re-mapped in order.
        let moved = targets
            .iter()
            .any(|(w, loc)| self.space.element_location(w) != Some(*loc));
        if moved {
            let order: Vec<WindowElement> = self.space.elements().cloned().collect();
            for w in order {
                if !windows.contains(&w) {
                    continue;
                }
                let loc = targets
                    .iter()
                    .find(|(t, _)| t == &w)
                    .map(|(_, l)| *l)
                    .or_else(|| self.space.element_location(&w));
                if let Some(loc) = loc {
                    self.space.map_element(w, loc, false);
                }
            }
        }
    }

    /// Ask a window to take `rect` (bar included); returns where its
    /// element goes. `tiled` sets the xdg tiled states so clients drop
    /// their rounded corners and shadows.
    fn apply_rect(&mut self, window: &WindowElement, rect: Rectangle<i32, Logical>, tiled: bool) -> Point<i32, Logical> {
        // The layout has placed it: no centring on its first commit.
        if let Some(flag) = window.user_data().get::<crate::shell::CenterOnFirstCommit>() {
            flag.0.set(false);
        }
        let header = window.pending_header_height();
        let client: Size<i32, Logical> = (rect.size.w.max(1), (rect.size.h - header).max(1)).into();
        if let Some(toplevel) = window.0.toplevel() {
            let changed = toplevel.with_pending_state(|state| {
                let mut changed = false;
                if state.size != Some(client) {
                    state.size = Some(client);
                    changed = true;
                }
                changed |= set_tiled(state, tiled);
                changed
            });
            if changed && toplevel.is_initial_configure_sent() {
                toplevel.send_pending_configure();
            }
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface() {
            let target = Rectangle::new(rect.loc + Point::from((0, header)), client);
            if surface.geometry() != target {
                let _ = surface.configure(target);
            }
        }
        rect.loc
    }

    /// Note where a window is before its first tile slot, so that it can go
    /// back there. A window that was maximised has just been restored by
    /// `set_layout_mode`, so the size it was asked for counts, not the one
    /// still on screen.
    fn remember_untiled(&mut self, window: &WindowElement) {
        let tile = window.tile();
        if tile.tiled_now.replace(true) {
            return;
        }
        let Some(mut geometry) = self.space.element_geometry(window) else {
            return;
        };
        if let Some(toplevel) = window.0.toplevel() {
            if let Some(size) = toplevel.with_pending_state(|state| state.size) {
                geometry.size = (size.w, size.h + window.pending_header_height()).into();
            }
        }
        *tile.untiled.borrow_mut() = Some(geometry);
    }

    /// Clear the tiled states of a window that is not a tile (any more); one
    /// that was a tile goes back to its floating geometry (or, when it was
    /// born a tile, to a centred window) and the new location is returned.
    fn ensure_untiled(&mut self, window: &WindowElement) -> Option<Point<i32, Logical>> {
        let was_tiled = window.tile().tiled_now.replace(false);
        let saved = window.tile().untiled.borrow_mut().take();
        if !was_tiled {
            if let Some(toplevel) = window.0.toplevel() {
                let changed = toplevel.with_pending_state(|state| set_tiled(state, false));
                if changed && toplevel.is_initial_configure_sent() {
                    toplevel.send_pending_configure();
                }
            }
            return None;
        }
        let rect = self.floating_rect(window, saved)?;
        Some(self.apply_rect(window, rect, false))
    }

    /// Where a window leaving the tiling goes: `saved` if it is still on a
    /// screen, else a window three fifths of the output, centred.
    fn floating_rect(
        &self,
        window: &WindowElement,
        saved: Option<Rectangle<i32, Logical>>,
    ) -> Option<Rectangle<i32, Logical>> {
        let on_screen = |r: &Rectangle<i32, Logical>| {
            self.space
                .outputs()
                .any(|o| self.space.output_geometry(o).map(|g| g.overlaps(*r)).unwrap_or(false))
        };
        if let Some(saved) = saved.filter(on_screen) {
            return Some(saved);
        }
        let area = self.window_home(window).and_then(|o| usable_area(&self.space, &o))?;
        let size: Size<i32, Logical> = (area.size.w * 3 / 5, area.size.h * 3 / 5).into();
        Some(Rectangle::new(crate::shell::centered(area, size), size))
    }

    /// Switch the layout mode (keybinding, shell widget, IPC).
    pub fn set_layout_mode(&mut self, mode: LayoutMode) {
        if self.layout.mode == mode {
            return;
        }
        tracing::info!(mode = mode.name(), "layout mode");
        self.layout.mode = mode;
        self.prefs.layout_mode = Some(mode);
        self.prefs.save();
        if mode.is_tiling() {
            // Start from the stacking order and show the tiles right away:
            // a maximised window would hide them.
            self.layout.outputs.clear();
            self.layout.pending.clear();
            let windows: Vec<WindowElement> = self.space.elements().cloned().collect();
            for w in windows {
                if w.pending_maximized() && w.tileable() {
                    self.unmaximize_window(&w);
                }
            }
        }
        self.layout.dirty = true;
        let line = crate::ipc::layout_mode_event(mode);
        self.ipc_broadcast(&line);
    }

    pub fn cycle_layout_mode(&mut self) {
        self.set_layout_mode(self.layout.mode.next());
    }

    /// Take the focused window out of the tiling, or put it back.
    pub fn toggle_floating_focused(&mut self) {
        let Some(window) = self.focused_window() else {
            return;
        };
        if window.is_dialog() {
            return;
        }
        let floating = !window.tile().floating.get();
        window.tile().floating.set(floating);
        if floating && self.layout.mode.is_tiling() {
            // A floating window in a tiled desktop: back where it was before
            // it became a tile, else a bit smaller and centred.
            window.tile().tiled_now.set(false);
            let saved = window.tile().untiled.borrow_mut().take();
            if let Some(rect) = self.floating_rect(&window, saved) {
                self.apply_rect(&window, rect, false);
                self.space.map_element(window.clone(), rect.loc, true);
            }
        }
        self.layout.dirty = true;
    }

    /// Super+arrows: focus the nearest window in a direction.
    pub fn focus_direction(&mut self, dir: Direction) {
        let Some(from) = self.focused_window() else {
            if let Some(w) = self.space.elements().last().cloned() {
                self.activate_window(&w);
            }
            return;
        };
        let Some(from_geo) = self.space.element_geometry(&from) else {
            return;
        };
        let geos: Vec<(WindowElement, Rectangle<i32, Logical>)> = self
            .space
            .elements()
            .filter(|w| **w != from)
            .filter_map(|w| self.space.element_geometry(w).map(|g| (w.clone(), g)))
            .collect();
        let next = neighbour_in(from_geo, dir, geos.iter().map(|(w, g)| (w, *g))).cloned();
        if let Some(next) = next {
            self.activate_window(&next);
            self.layout.dirty = true;
        }
    }

    /// Super+Shift+arrows: swap the focused tile with its neighbour.
    pub fn move_direction(&mut self, dir: Direction) {
        let Some(from) = self.focused_window() else {
            return;
        };
        if !self.layout.is_tiled(&from) {
            return;
        }
        if self.layout.mode == LayoutMode::Columns {
            match dir {
                Direction::Left => {
                    self.layout.shift(&from, false);
                }
                Direction::Right => {
                    self.layout.shift(&from, true);
                }
                _ => {}
            }
            return;
        }
        let Some(from_geo) = self.space.element_geometry(&from) else {
            return;
        };
        let geos: Vec<(WindowElement, Rectangle<i32, Logical>)> = self
            .space
            .elements()
            .filter(|w| **w != from && self.layout.is_tiled(w))
            .filter_map(|w| self.space.element_geometry(w).map(|g| (w.clone(), g)))
            .collect();
        let target = neighbour_in(from_geo, dir, geos.iter().map(|(w, g)| (w, *g))).cloned();
        if let Some(target) = target {
            self.layout.swap(&from, &target);
        }
    }

    /// Super+R in columns mode: the next width preset for the focused column.
    pub fn cycle_column_width(&mut self) {
        if self.layout.mode != LayoutMode::Columns {
            return;
        }
        if let Some(window) = self.focused_window() {
            if self.layout.is_tiled(&window) {
                self.layout.cycle_width(&window);
            }
        }
    }

    /// The end of a title-bar drag in a tiling mode: swap with the tile under
    /// the pointer, then snap everything back into the grid.
    pub fn drag_finished(&mut self, window: &WindowElement) {
        self.layout.dragging = None;
        if !self.layout.is_tiled(window) {
            self.layout.dirty = true;
            return;
        }
        let pointer = self.pointer.current_location();
        let stack: Vec<WindowElement> = self.space.elements().cloned().collect();
        let target = stack
            .iter()
            .rev()
            .filter(|w| *w != window && self.layout.is_tiled(w))
            .find(|w| {
                self.space
                    .element_geometry(w)
                    .map(|g| g.to_f64().contains(pointer))
                    .unwrap_or(false)
            })
            .cloned();
        if let Some(target) = target {
            self.layout.swap(window, &target);
        }
        self.layout.dirty = true;
    }

    /// Undo maximise on any kind of window.
    pub fn unmaximize_window(&mut self, window: &WindowElement) {
        use smithay::wayland::shell::xdg::XdgShellHandler;
        if let Some(toplevel) = window.0.toplevel().cloned() {
            XdgShellHandler::unmaximize_request(self, toplevel);
            return;
        }
        #[cfg(feature = "xwayland")]
        if let Some(surface) = window.0.x11_surface().cloned() {
            use smithay::xwayland::XwmHandler;
            if let Some(id) = self.xwm.as_ref().map(|wm| wm.id()) {
                XwmHandler::unmaximize_request(self, id, surface);
            }
        }
    }
}

/// Set or clear the four xdg tiled states; returns whether anything changed.
fn set_tiled(state: &mut smithay::wayland::shell::xdg::ToplevelState, tiled: bool) -> bool {
    let mut changed = false;
    for st in [
        xdg_toplevel::State::TiledLeft,
        xdg_toplevel::State::TiledRight,
        xdg_toplevel::State::TiledTop,
        xdg_toplevel::State::TiledBottom,
    ] {
        if tiled {
            changed |= state.states.set(st);
        } else {
            changed |= state.states.unset(st);
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }

    #[test]
    fn modes_parse_and_cycle() {
        assert_eq!(LayoutMode::parse("niri"), Some(LayoutMode::Columns));
        assert_eq!(LayoutMode::parse("Hyprland"), Some(LayoutMode::Dwindle));
        assert_eq!(LayoutMode::parse("kde"), Some(LayoutMode::Floating));
        assert_eq!(LayoutMode::parse("x"), None);
        assert_eq!(LayoutMode::Floating.next().next().next(), LayoutMode::Floating);
        assert_eq!(serde_json::to_string(&LayoutMode::Dwindle).unwrap(), "\"dwindle\"");
    }

    #[test]
    fn dwindle_spirals_along_the_longer_side() {
        let area = rect(0, 0, 1920, 1080);
        assert_eq!(dwindle_rects(area, 1, 8), vec![area]);
        let two = dwindle_rects(area, 2, 8);
        assert_eq!(two[0], rect(0, 0, 956, 1080));
        assert_eq!(two[1], rect(964, 0, 956, 1080));
        let three = dwindle_rects(area, 3, 8);
        assert_eq!(three[0], rect(0, 0, 956, 1080));
        assert_eq!(three[1], rect(964, 0, 956, 536));
        assert_eq!(three[2], rect(964, 544, 956, 536));
        // never overlapping, never empty
        let many = dwindle_rects(area, 9, 8);
        for r in &many {
            assert!(r.size.w >= MIN_TILE && r.size.h >= MIN_TILE);
        }
    }

    #[test]
    fn columns_fill_and_scroll_to_the_focused_one() {
        let area = rect(100, 50, 1000, 600);
        let widths = [0.5, 0.5];
        let cols = column_rects(area, &widths, 10, 0);
        assert_eq!(cols[0], rect(100, 50, 495, 600));
        assert_eq!(cols[1], rect(605, 50, 495, 600));
        assert_eq!(scroll_to_show(1000, &widths, 10, 0, Some(1)), 0);
        let widths = [0.5, 0.5, 0.5];
        // the third column is off-screen until it gets the focus
        assert_eq!(scroll_to_show(1000, &widths, 10, 0, Some(0)), 0);
        let s = scroll_to_show(1000, &widths, 10, 0, Some(2));
        assert_eq!(s, 505);
        // scrolling back to the first column
        assert_eq!(scroll_to_show(1000, &widths, 10, s, Some(0)), 0);
        assert_eq!(column_width(1000, 1.0, 10), 1000);
    }

    #[test]
    fn neighbours_are_picked_by_direction() {
        let (a, b) = ("a", "b");
        let from = rect(0, 0, 100, 100);
        let cands = vec![(&a, rect(200, 0, 100, 100)), (&b, rect(0, 200, 100, 100))];
        assert_eq!(neighbour_in(from, Direction::Right, cands.iter().cloned()), Some(&a));
        assert_eq!(neighbour_in(from, Direction::Down, cands.iter().cloned()), Some(&b));
        assert_eq!(neighbour_in(from, Direction::Left, cands.iter().cloned()), None);
    }
}
