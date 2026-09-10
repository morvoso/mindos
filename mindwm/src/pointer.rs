//! Shared pointer motion for native, absolute and nested-backend input.
use crate::{state::Backend, AnvilState};
use smithay::{
    backend::renderer::utils::with_renderer_surface_state,
    input::pointer::{MotionEvent, RelativeMotionEvent},
    reexports::wayland_server::protocol::wl_surface::WlSurface,
    utils::{Logical, Point, Rectangle, SERIAL_COUNTER},
    wayland::{
        compositor::{with_states, RegionAttributes, SurfaceAttributes},
        pointer_constraints::{with_pointer_constraint, PointerConstraint},
        seat::WaylandFocus,
    },
};

// Rectangle right/bottom edges are exclusive. Keep fractional pointer coordinates
// inside by one Wayland fixed-point unit; smaller gaps round onto the edge.
const EDGE_EPSILON: f64 = 1.0 / 256.0;

pub(crate) fn clamp_to_outputs(
    pos: Point<f64, Logical>,
    outputs: impl Iterator<Item = Rectangle<i32, Logical>>,
) -> Point<f64, Logical> {
    outputs
        .filter(|r| r.size.w > 0 && r.size.h > 0)
        .map(|r| {
            let x = if pos.x.is_finite() {
                pos.x
            } else {
                r.loc.x as f64
            };
            let y = if pos.y.is_finite() {
                pos.y
            } else {
                r.loc.y as f64
            };
            Point::from((
                x.clamp(
                    r.loc.x as f64,
                    r.loc.x as f64 + r.size.w as f64 - EDGE_EPSILON,
                ),
                y.clamp(
                    r.loc.y as f64,
                    r.loc.y as f64 + r.size.h as f64 - EDGE_EPSILON,
                ),
            ))
        })
        .min_by(|a, b| {
            let distance = |p: &Point<f64, Logical>| (p.x - pos.x).hypot(p.y - pos.y);
            distance(a).total_cmp(&distance(b))
        })
        .unwrap_or(pos)
}

/// Clamp a point into one output, with the same edge handling as
/// [`clamp_to_outputs`].
fn clamp_into(r: Rectangle<i32, Logical>, pos: Point<f64, Logical>) -> Point<f64, Logical> {
    let x = if pos.x.is_finite() { pos.x } else { r.loc.x as f64 };
    let y = if pos.y.is_finite() { pos.y } else { r.loc.y as f64 };
    Point::from((
        x.clamp(
            r.loc.x as f64,
            r.loc.x as f64 + r.size.w as f64 - EDGE_EPSILON,
        ),
        y.clamp(
            r.loc.y as f64,
            r.loc.y as f64 + r.size.h as f64 - EDGE_EPSILON,
        ),
    ))
}

/// Clamp `pos` onto the outputs, honouring the direction the pointer is
/// travelling in.
///
/// Screens of different logical size leave a band along their shared edge with
/// nothing opposite it: a 3072x1728 screen beside a 2560x1440 one, aligned at
/// the top, has 288 rows at the bottom bordering empty space. Nearest-output
/// clamping traps the pointer inside that band, because stepping sideways out
/// of the tall screen lands nearer the edge it just left than the short screen
/// beside it, and the pointer is pushed straight back; the screens can only be
/// crossed higher up. So when the pointer leaves every output, look for one
/// across the edge it is heading for and clamp only the other axis: a sideways
/// push always reaches the screen beside it, at the nearest height that screen
/// has.
pub(crate) fn clamp_to_outputs_along(
    previous: Point<f64, Logical>,
    pos: Point<f64, Logical>,
    outputs: impl Iterator<Item = Rectangle<i32, Logical>>,
) -> Point<f64, Logical> {
    let rects: Vec<_> = outputs.filter(|r| r.size.w > 0 && r.size.h > 0).collect();
    if rects.is_empty() {
        return pos;
    }
    if !pos.x.is_finite() || !pos.y.is_finite() {
        return clamp_to_outputs(pos, rects.into_iter());
    }
    // Still on a screen: nothing to resolve.
    if rects.iter().any(|r| clamp_into(*r, pos) == pos) {
        return pos;
    }
    let delta = pos - previous;
    if delta.x != 0.0 || delta.y != 0.0 {
        // The axis being crossed; the other one is the one to clamp.
        let horizontal = delta.x.abs() > delta.y.abs();
        let offset = |r: &Rectangle<i32, Logical>| {
            let c = clamp_into(*r, pos);
            if horizontal {
                (c.y - pos.y).abs()
            } else {
                (c.x - pos.x).abs()
            }
        };
        // The screens the pointer is level with across that edge.
        let across: Vec<Rectangle<i32, Logical>> = rects
            .iter()
            .copied()
            .filter(|r| {
                let c = clamp_into(*r, pos);
                if horizontal {
                    c.x == pos.x
                } else {
                    c.y == pos.y
                }
            })
            .collect();
        if let Some(best) = across
            .into_iter()
            .min_by(|a, b| offset(a).total_cmp(&offset(b)))
        {
            return clamp_into(best, pos);
        }
    }
    clamp_to_outputs(pos, rects.into_iter())
}

pub(crate) fn absolute_position(
    x: f64,
    y: f64,
    outputs: &[Rectangle<i32, Logical>],
) -> Option<Point<f64, Logical>> {
    let mut rects = outputs.iter().filter(|r| r.size.w > 0 && r.size.h > 0);
    let first = rects.next()?;
    let (mut left, mut top) = (first.loc.x as f64, first.loc.y as f64);
    let (mut right, mut bottom) = (left + first.size.w as f64, top + first.size.h as f64);
    for r in rects {
        left = left.min(r.loc.x as f64);
        top = top.min(r.loc.y as f64);
        right = right.max(r.loc.x as f64 + r.size.w as f64);
        bottom = bottom.max(r.loc.y as f64 + r.size.h as f64);
    }
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some(clamp_to_outputs(
        (
            left + x.clamp(0.0, 1.0) * (right - left),
            top + y.clamp(0.0, 1.0) * (bottom - top),
        )
            .into(),
        outputs.iter().copied(),
    ))
}

struct Confinement {
    bounds: Rectangle<i32, Logical>,
    input: Option<RegionAttributes>,
    region: Option<RegionAttributes>,
}

impl Confinement {
    fn for_surface(surface: &WlSurface, region: Option<RegionAttributes>) -> Option<Self> {
        let size = with_renderer_surface_state(surface, |state| state.surface_size()).flatten()?;
        let input = with_states(surface, |states| {
            states
                .cached_state
                .get::<SurfaceAttributes>()
                .current()
                .input_region
                .clone()
        });
        Some(Self {
            bounds: Rectangle::from_size(size),
            input,
            region,
        })
    }

    fn contains(&self, point: Point<f64, Logical>) -> bool {
        let point = point.to_i32_floor();
        self.bounds.contains(point)
            && self.input.as_ref().is_none_or(|r| r.contains(point))
            && self.region.as_ref().is_none_or(|r| r.contains(point))
    }

    // Test every interval between region edges along the segment. Endpoint-only
    // checks allow a fast mouse to jump across holes or disconnected rectangles.
    fn clip_segment(
        &self,
        start: Point<f64, Logical>,
        end: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        if !self.contains(start) {
            return start;
        }
        let delta = end - start;
        let along = |t: f64| start + Point::from((delta.x * t, delta.y * t));
        let mut crossings = vec![0.0, 1.0];
        for rect in std::iter::once(&self.bounds).chain(
            self.input
                .iter()
                .chain(self.region.iter())
                .flat_map(|r| r.rects.iter().map(|(_, rect)| rect)),
        ) {
            for (origin, distance, low, size) in [
                (start.x, delta.x, rect.loc.x as f64, rect.size.w as f64),
                (start.y, delta.y, rect.loc.y as f64, rect.size.h as f64),
            ] {
                if distance == 0.0 {
                    continue;
                }
                for edge in [low, low + size] {
                    let t = (edge - origin) / distance;
                    if t > 0.0 && t < 1.0 {
                        crossings.push(t);
                    }
                }
            }
        }
        crossings.sort_by(f64::total_cmp);
        crossings.dedup();
        let epsilon = EDGE_EPSILON / delta.x.abs().max(delta.y.abs()).max(1.0);
        for interval in crossings.windows(2) {
            let middle = along((interval[0] + interval[1]) * 0.5);
            if !self.contains(middle) {
                return along((interval[0] - epsilon).max(0.0));
            }
        }
        if self.contains(end) {
            end
        } else {
            along((1.0 - epsilon).max(0.0))
        }
    }

    fn constrain(
        &self,
        start: Point<f64, Logical>,
        end: Point<f64, Logical>,
    ) -> Point<f64, Logical> {
        let clipped = self.clip_segment(start, end);
        // Preserve movement along an edge when the other axis hits it.
        let slide_x = self.clip_segment(clipped, (end.x, clipped.y).into());
        self.clip_segment(slide_x, (slide_x.x, end.y).into())
    }
}

impl<B: Backend> AnvilState<B> {
    pub(crate) fn activate_pointer_constraint(&self) {
        let pointer = &self.pointer;
        let Some((target, origin)) = self.surface_under(pointer.current_location()) else {
            return;
        };
        let Some(surface) = target.wl_surface() else {
            return;
        };
        if pointer
            .current_focus()
            .and_then(|f| f.wl_surface().map(|s| s.into_owned()))
            .as_ref()
            != Some(&*surface)
        {
            return;
        }
        // Smithay holds the surface mutex inside with_pointer_constraint.
        // Read renderer/input state only after that callback has returned.
        let region = with_pointer_constraint(&surface, pointer, |constraint| {
            constraint
                .filter(|c| !c.is_active())
                .map(|c| c.region().cloned())
        });
        if region.is_some_and(|region| {
            Confinement::for_surface(&surface, region)
                .is_some_and(|region| region.contains(pointer.current_location() - origin))
        }) {
            with_pointer_constraint(&surface, pointer, |constraint| {
                if let Some(c) = constraint {
                    c.activate();
                }
            });
        }
    }

    pub(crate) fn handle_absolute_pointer(
        &mut self,
        device: String,
        position: Point<f64, Logical>,
        time: u64,
    ) {
        // Track device coordinates independently: a locked cursor stays still,
        // but successive absolute events must still produce incremental deltas.
        let previous = self
            .absolute_pointer_positions
            .insert(device, position)
            .unwrap_or(position);
        let delta = position - previous;
        self.handle_pointer_motion(position, delta, delta, time);
    }

    pub(crate) fn handle_pointer_motion(
        &mut self,
        position: Point<f64, Logical>,
        delta: Point<f64, Logical>,
        raw: Point<f64, Logical>,
        time: u64,
    ) {
        if !position.x.is_finite()
            || !position.y.is_finite()
            || !delta.x.is_finite()
            || !delta.y.is_finite()
        {
            return;
        }
        let pointer = self.pointer.clone();
        let previous = pointer.current_location();
        let under = self.surface_under(previous);
        let mut locked = false;
        let mut confinement = None;
        if let Some((surface, origin)) = under
            .as_ref()
            .and_then(|(target, origin)| Some((target.wl_surface()?, *origin)))
        {
            let constraint = with_pointer_constraint(&surface, &pointer, |constraint| {
                constraint.filter(|c| c.is_active()).map(|c| {
                    (
                        matches!(&*c, PointerConstraint::Locked(_)),
                        c.region().cloned(),
                    )
                })
            });
            if let Some((is_locked, region)) = constraint {
                let effective = Confinement::for_surface(&surface, region);
                if effective
                    .as_ref()
                    .is_some_and(|r| r.contains(previous - origin))
                {
                    locked = is_locked;
                    if !locked {
                        confinement = effective.map(|r| (r, origin));
                    }
                } else {
                    with_pointer_constraint(&surface, &pointer, |constraint| {
                        if let Some(c) = constraint {
                            c.deactivate();
                        }
                    });
                }
            }
        }

        pointer.relative_motion(
            self,
            under.clone(),
            &RelativeMotionEvent {
                delta,
                delta_unaccel: raw,
                utime: time,
            },
        );
        if locked {
            pointer.frame(self);
            return;
        }
        let mut position = clamp_to_outputs_along(
            previous,
            position,
            self.space
                .outputs()
                .filter_map(|o| self.space.output_geometry(o)),
        );
        if let Some((region, origin)) = confinement {
            position = region.constrain(previous - origin, position - origin) + origin;
            // A different surface may have appeared over the constrained region.
            // Leaving that surface releases the constraint through Smithay.
        }
        let new_under = self.surface_under(position);
        pointer.motion(
            self,
            new_under,
            &MotionEvent {
                location: position,
                serial: SERIAL_COUNTER.next_serial(),
                time: (time / 1000) as u32,
            },
        );
        pointer.frame(self);
        self.activate_pointer_constraint();
        self.update_resize_cursor(position);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use smithay::wayland::compositor::RectangleKind::{Add, Subtract};
    fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle<i32, Logical> {
        Rectangle::new((x, y).into(), (w, h).into())
    }
    fn region(
        rects: Vec<(
            smithay::wayland::compositor::RectangleKind,
            Rectangle<i32, Logical>,
        )>,
    ) -> Confinement {
        Confinement {
            bounds: rect(0, 0, 100, 100),
            input: None,
            region: Some(RegionAttributes { rects }),
        }
    }
    #[test]
    fn outputs_support_negative_stacked_and_gapped_layouts() {
        let outputs = [
            rect(-1920, 0, 1920, 1080),
            rect(0, -1200, 1600, 1200),
            rect(200, 200, 800, 600),
        ];
        for p in [(-800.5, 900.0), (1500.0, -800.0), (201.0, 201.0)] {
            assert_eq!(
                clamp_to_outputs(p.into(), outputs.iter().copied()),
                p.into()
            );
        }
        let gap = clamp_to_outputs((100.0, 400.0).into(), outputs.iter().copied());
        assert_eq!(gap, (200.0, 400.0).into());
        let edge = clamp_to_outputs((9000.0, 9000.0).into(), outputs.iter().copied());
        assert!(outputs.iter().any(|r| r.contains(edge.to_i32_floor())));
    }

    #[test]
    fn pointer_crosses_between_screens_of_different_heights() {
        // A 3072x1728 screen beside a 2560x1440 one, aligned at the top: the
        // bottom 288 rows of the tall screen have no neighbour opposite them.
        let outputs = [rect(0, 0, 3072, 1728), rect(3072, 0, 2560, 1440)];
        // Stepping right from inside that band reaches the short screen, at the
        // lowest row it has, instead of being pushed back.
        let crossed = clamp_to_outputs_along(
            (3060.0, 1600.0).into(),
            (3100.0, 1600.0).into(),
            outputs.iter().copied(),
        );
        assert!(outputs[1].contains(crossed.to_i32_floor()), "{crossed:?}");
        assert_eq!(crossed.x, 3100.0);
        assert!(crossed.y > 1439.0 && crossed.y < 1440.0);
        // Coming back the other way is untouched: it is still on a screen.
        let back = clamp_to_outputs_along(
            (3080.0, 1400.0).into(),
            (3050.0, 1400.0).into(),
            outputs.iter().copied(),
        );
        assert_eq!(back, (3050.0, 1400.0).into());
        // Pushing down off the bottom has nothing to cross to, so it stops at
        // the edge it is leaving rather than jumping sideways.
        let floor = clamp_to_outputs_along(
            (1000.0, 1700.0).into(),
            (1000.0, 1800.0).into(),
            outputs.iter().copied(),
        );
        assert_eq!(floor.x, 1000.0);
        assert!(floor.y > 1727.0 && floor.y < 1728.0);
        // Nor is there anything past the far right edge.
        let far = clamp_to_outputs_along(
            (5600.0, 700.0).into(),
            (5700.0, 700.0).into(),
            outputs.iter().copied(),
        );
        assert!(outputs[1].contains(far.to_i32_floor()));
        // A pointer that has not moved is left where it is.
        let still = clamp_to_outputs_along(
            (3100.0, 1600.0).into(),
            (3100.0, 1600.0).into(),
            outputs.iter().copied(),
        );
        assert!(outputs.iter().any(|r| r.contains(still.to_i32_floor())));
    }
    #[test]
    fn absolute_mapping_uses_real_bounds_and_handles_no_outputs() {
        assert!(absolute_position(0.5, 0.5, &[]).is_none());
        let outputs = [rect(-100, -100, 100, 100), rect(-100, 0, 100, 100)];
        assert_eq!(
            absolute_position(0.5, 0.75, &outputs),
            Some((-50.0, 50.0).into())
        );
        assert!(outputs[1].contains(
            absolute_position(1.0, 1.0, &outputs)
                .unwrap()
                .to_i32_floor()
        ));
        assert!(absolute_position(f64::NAN, 0.0, &outputs).is_none());
    }
    #[test]
    fn confinement_slides_at_edges_instead_of_freezing() {
        let r = region(vec![(Add, rect(10, 10, 60, 60))]);
        let p = r.constrain((60.0, 30.0).into(), (200.0, 50.0).into());
        assert!(p.x < 70.0 && p.x > 69.99);
        assert_eq!(p.y, 50.0);
        assert!(r.contains(p));
    }
    #[test]
    fn fast_motion_cannot_jump_holes_or_disconnected_regions() {
        for r in [
            region(vec![
                (Add, rect(0, 0, 100, 100)),
                (Subtract, rect(40, 0, 20, 100)),
            ]),
            region(vec![
                (Add, rect(0, 0, 40, 100)),
                (Add, rect(60, 0, 40, 100)),
            ]),
        ] {
            let p = r.constrain((20.0, 50.0).into(), (80.0, 50.0).into());
            assert!(p.x < 40.0 && p.x > 39.99);
            assert!(r.contains(p));
        }
    }
    #[test]
    fn input_region_intersects_constraint_and_surface_bounds() {
        let mut r = region(vec![(Add, rect(-100, -100, 300, 300))]);
        r.input = Some(RegionAttributes {
            rects: vec![(Add, rect(20, 20, 60, 60))],
        });
        assert!(!r.contains((19.9, 30.0).into()));
        let p = r.constrain((30.0, 30.0).into(), (-100.0, -100.0).into());
        assert!(p.x >= 20.0 && p.x < 20.01 && p.y >= 20.0 && p.y < 20.01);
    }
}
