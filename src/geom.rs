//! Grid points and the exact-integer polygon predicates the compiler needs.

/// A point on the map grid, in whole map units.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pt {
    /// X coordinate in map units.
    pub x: i32,
    /// Y coordinate in map units.
    pub y: i32,
}

/// Twice the signed area of a closed polygon — the shoelace sum.
///
/// Negative for clockwise winding, which is the Doom convention.
#[must_use]
pub fn shoelace2(poly: &[Pt]) -> i64 {
    let n = poly.len();
    (0..n)
        .map(|i| {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            i64::from(a.x) * i64::from(b.y) - i64::from(b.x) * i64::from(a.y)
        })
        .sum()
}

/// Whether the polygon winds clockwise, as every sector footprint must.
#[must_use]
pub fn is_clockwise(poly: &[Pt]) -> bool {
    shoelace2(poly) < 0
}

/// Iterates the polygon's directed edges, closing the loop.
pub fn edges(poly: &[Pt]) -> impl Iterator<Item = (Pt, Pt)> + '_ {
    let n = poly.len();
    (0..n).map(move |i| (poly[i], poly[(i + 1) % n]))
}

/// Whether an edge is axis-aligned or at exactly 45 degrees.
///
/// Widened to `i64` like every other predicate here: an `i32` subtraction can
/// overflow, and `i32::MIN.abs()` panics outright.
#[must_use]
pub fn is_axis_or_diagonal(a: Pt, b: Pt) -> bool {
    let dx = (i64::from(b.x) - i64::from(a.x)).abs();
    let dy = (i64::from(b.y) - i64::from(a.y)).abs();
    (dx == 0) != (dy == 0) || (dx == dy && dx != 0)
}

/// Whether the point lies strictly inside the polygon.
///
/// Even-odd ray casting in exact integer arithmetic — the crossing test is
/// cross-multiplied rather than divided, so no rounding enters the topology.
///
/// A polygon of fewer than three points encloses no area, so nothing is
/// inside it and `false` is returned. That guard is load-bearing rather than
/// cosmetic: the ray cast starts from the last vertex, so an empty slice
/// would underflow `n - 1` and panic.
///
/// Note this is *not* a strict interior test at the boundary — a point lying
/// exactly on an edge may return either answer, since the ray-cast tie-break
/// is undefined there. Callers that must exclude the boundary pair this with
/// an explicit on-boundary test.
#[must_use]
#[allow(clippy::many_single_char_names)]
pub fn contains(poly: &[Pt], p: Pt) -> bool {
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) {
            let dy = i64::from(b.y) - i64::from(a.y);
            let cross = i64::from(a.x) * dy
                + (i64::from(p.y) - i64::from(a.y)) * (i64::from(b.x) - i64::from(a.x));
            let px = i64::from(p.x) * dy;
            if (dy > 0 && px < cross) || (dy < 0 && px > cross) {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// The axis a wall runs along.
///
/// `pub(crate)` so [`crate::ir`] can validate a portal's facing-wall gap with
/// the same geometry [`crate::compile::portals`] later cuts through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Axis {
    /// The wall is vertical; X is constant.
    Vertical,
    /// The wall is horizontal; Y is constant.
    Horizontal,
}

impl Axis {
    /// Splits a point into `(along, across)` for this axis: the coordinate
    /// that varies along the wall, and the one held constant across it.
    pub(crate) fn split(self, p: Pt) -> (i32, i32) {
        match self {
            Self::Vertical => (p.y, p.x),
            Self::Horizontal => (p.x, p.y),
        }
    }
}

/// Every axis-aligned edge of a footprint, as `(axis, fixed, lo, hi,
/// forward)` where `forward` records whether the edge runs in the increasing
/// `along` direction.
///
/// Edges at 45 degrees are skipped: a diagonal wall cannot host a portal or
/// exit in v1, since the opening's endpoints and the flanking wall pieces
/// would all have to land on non-integer coordinates to stay flush with it.
pub(crate) fn wall_edges(poly: &[Pt]) -> impl Iterator<Item = (Axis, i32, i32, i32, bool)> + '_ {
    edges(poly).filter_map(|(p, q)| {
        let axis = if p.x == q.x && p.y != q.y {
            Axis::Vertical
        } else if p.y == q.y && p.x != q.x {
            Axis::Horizontal
        } else {
            return None;
        };
        let (along_p, fixed) = axis.split(p);
        let (along_q, _) = axis.split(q);
        Some((
            axis,
            fixed,
            along_p.min(along_q),
            along_p.max(along_q),
            along_q > along_p,
        ))
    })
}

/// Whether an edge is diagonal (exactly 45 degrees) rather than axis-aligned.
///
/// [`is_axis_or_diagonal`] admits both; this distinguishes the two, since
/// [`on_diagonal_wall`] only cares about the one [`wall_edges`] drops.
fn is_diagonal_edge(a: Pt, b: Pt) -> bool {
    let dx = (i64::from(b.x) - i64::from(a.x)).abs();
    let dy = (i64::from(b.y) - i64::from(a.y)).abs();
    dx == dy && dx != 0
}

/// Whether `p` lies on (not just inside) a diagonal edge of `poly`.
///
/// `pub(crate)` so both [`crate::compile::portals::resolve_portal`] and
/// [`crate::compile::exits::resolve_exit`] can give an honest, specific error
/// naming a diagonal wall a requested opening sits on, rather than folding it
/// into "not on any wall" for a point that demonstrably is on one.
pub(crate) fn on_diagonal_wall(poly: &[Pt], p: Pt) -> bool {
    edges(poly).any(|(a, b)| {
        if !is_diagonal_edge(a, b) {
            return false;
        }
        let cross = (i64::from(p.y) - i64::from(a.y)) * (i64::from(b.x) - i64::from(a.x))
            - (i64::from(p.x) - i64::from(a.x)) * (i64::from(b.y) - i64::from(a.y));
        cross == 0
            && p.x >= a.x.min(b.x)
            && p.x <= a.x.max(b.x)
            && p.y >= a.y.min(b.y)
            && p.y <= a.y.max(b.y)
    })
}

/// The sign, along the across-axis, of the direction that leads away from the
/// interior of a room whose wall runs in the `forward` direction on `axis`.
///
/// A room's interior lies to the right of its own boundary edge direction
/// (footprints wind clockwise). Rotating a `+along` direction vector by -90
/// degrees gives `+across` for a vertical wall (`along` = Y, `across` = X)
/// but `-across` for a horizontal wall (`along` = X, `across` = Y) — the two
/// axes are not mirror images — so the two cases are derived separately.
///
/// `pub(crate)` so [`crate::compile::exits`] can derive a walkover alcove's
/// outward direction from a single wall, without needing a second room to
/// build a [`FacingSpan`] against.
pub(crate) fn outward_sign(axis: Axis, forward: bool) -> i32 {
    match axis {
        Axis::Vertical => {
            if forward {
                -1
            } else {
                1
            }
        }
        Axis::Horizontal => {
            if forward {
                1
            } else {
                -1
            }
        }
    }
}

/// One pair of walls, from two different rooms, that face each other across
/// a void — coincident (`near == far`, the legacy flush case) or separated by
/// a real gap.
///
/// `pub(crate)` so [`crate::ir::Ir::from_json`] can validate a portal's gap
/// with the identical geometry [`crate::compile::portals::resolve_portal`]
/// later cuts through — computed once, here, so the two can never diverge.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FacingSpan {
    /// The axis the two walls run along.
    pub(crate) axis: Axis,
    /// Room `a`'s own wall coordinate.
    pub(crate) near: i32,
    /// Room `b`'s own wall coordinate.
    pub(crate) far: i32,
    /// The low end of the run both walls have in common.
    pub(crate) lo: i32,
    /// The high end of the run both walls have in common.
    pub(crate) hi: i32,
    /// Whether room `a`'s own boundary edge along this wall runs in the
    /// increasing-`along` direction. Room `b`'s own edge direction is always
    /// the opposite — see [`facing_spans`]'s doc comment.
    pub(crate) a_forward: bool,
}

impl FacingSpan {
    /// The width of the void between the two walls, in map units. Zero for
    /// the legacy flush case (`near == far`).
    pub(crate) fn gap(self) -> i32 {
        (self.far - self.near).abs()
    }
}

/// Every stretch of wall two footprints face each other across: real,
/// collinear-or-parallel runs where both interiors lie on opposite sides of
/// the void between them, not merely two edges that happen to reach the same
/// bounding-box coordinate.
///
/// Two walls face each other either when they are coincident (`fixed_a ==
/// fixed_b`) and wind in opposite directions — the classic flush-adjacency
/// case, where the shared line has one interior on each side — or when they
/// are genuinely separated and each wall's own outward direction
/// ([`outward_sign`]) points toward the other. A room may face another along
/// more than one run (an L wrapped around a rectangle faces two); every one
/// is returned, in `poly_a`'s edge order then `poly_b`'s, so the result is
/// deterministic.
///
/// `pub(crate)` for the same reason [`FacingSpan`] is: both the IR's gap
/// validation and the compiler's portal-cutting pass need the identical
/// geometry.
pub(crate) fn facing_spans(poly_a: &[Pt], poly_b: &[Pt]) -> Vec<FacingSpan> {
    let mut spans = Vec::new();
    for (axis_a, fixed_a, lo_a, hi_a, forward_a) in wall_edges(poly_a) {
        for (axis_b, fixed_b, lo_b, hi_b, forward_b) in wall_edges(poly_b) {
            if axis_a != axis_b {
                continue;
            }
            let facing = if fixed_a == fixed_b {
                forward_a != forward_b
            } else {
                let gap_sign = (fixed_b - fixed_a).signum();
                outward_sign(axis_a, forward_a) == gap_sign
                    && outward_sign(axis_a, forward_b) == -gap_sign
            };
            if !facing {
                continue;
            }
            // Strict: edges that meet at a single point face across no real
            // span.
            let (lo, hi) = (lo_a.max(lo_b), hi_a.min(hi_b));
            if lo < hi {
                spans.push(FacingSpan {
                    axis: axis_a,
                    near: fixed_a,
                    far: fixed_b,
                    lo,
                    hi,
                    a_forward: forward_a,
                });
            }
        }
    }
    spans
}

/// Finds the one [`FacingSpan`] (if any) whose run `at` lands on: the same
/// `across == span.near` (room `a`'s own wall) and strict-interior `along`
/// predicate both [`crate::ir::Ir::from_json`]'s gap validation and
/// [`crate::compile::portals::resolve_portal`] need, kept in one place so
/// the two can never disagree about which span a point resolves to.
pub(crate) fn find_facing_span(spans: &[FacingSpan], at: Pt) -> Option<FacingSpan> {
    spans
        .iter()
        .find(|s| {
            let (along, across) = s.axis.split(at);
            across == s.near && along > s.lo && along < s.hi
        })
        .copied()
}

/// The distance from a point to a line segment, in map units.
#[must_use]
pub fn dist_to_segment(p: Pt, a: Pt, b: Pt) -> f64 {
    let (px, py) = (f64::from(p.x), f64::from(p.y));
    let (ax, ay) = (f64::from(a.x), f64::from(a.y));
    let (bx, by) = (f64::from(b.x), f64::from(b.y));
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx.mul_add(dx, dy * dy);
    if len2 == 0.0 {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    (px - dx.mul_add(t, ax)).hypot(py - dy.mul_add(t, ay))
}

/// The distance from a point to the nearest polygon edge, in map units.
#[must_use]
pub fn clearance(poly: &[Pt], p: Pt) -> f64 {
    edges(poly)
        .map(|(a, b)| dist_to_segment(p, a, b))
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
mod tests {
    use super::{Pt, clearance, contains, is_axis_or_diagonal, is_clockwise, shoelace2};

    /// A 256-unit square wound clockwise, matching the Doom convention.
    fn square() -> Vec<Pt> {
        vec![
            Pt { x: 0, y: 0 },
            Pt { x: 0, y: 256 },
            Pt { x: 256, y: 256 },
            Pt { x: 256, y: 0 },
        ]
    }

    #[test]
    fn clockwise_square_has_negative_shoelace() {
        assert_eq!(shoelace2(&square()), -131_072);
        assert!(is_clockwise(&square()));
    }

    #[test]
    fn reversed_square_is_counter_clockwise() {
        let mut ccw = square();
        ccw.reverse();
        assert!(!is_clockwise(&ccw));
    }

    #[test]
    fn containment_distinguishes_inside_from_outside() {
        let s = square();
        assert!(contains(&s, Pt { x: 128, y: 128 }));
        assert!(!contains(&s, Pt { x: 300, y: 128 }));
        assert!(!contains(&s, Pt { x: -1, y: 128 }));
    }

    #[test]
    fn clearance_is_distance_to_the_nearest_wall() {
        let s = square();
        assert!((clearance(&s, Pt { x: 128, y: 128 }) - 128.0).abs() < 1e-9);
        assert!((clearance(&s, Pt { x: 16, y: 128 }) - 16.0).abs() < 1e-9);
    }

    #[test]
    fn edges_must_be_axis_aligned_or_exactly_diagonal() {
        assert!(is_axis_or_diagonal(Pt { x: 0, y: 0 }, Pt { x: 64, y: 0 }));
        assert!(is_axis_or_diagonal(Pt { x: 0, y: 0 }, Pt { x: 0, y: 64 }));
        assert!(is_axis_or_diagonal(Pt { x: 0, y: 0 }, Pt { x: 64, y: 64 }));
        assert!(!is_axis_or_diagonal(Pt { x: 0, y: 0 }, Pt { x: 64, y: 32 }));
    }

    #[test]
    fn a_polygon_with_no_area_contains_nothing() {
        // An empty slice reached `j = n - 1` before this was guarded, so the
        // subtraction underflowed and panicked rather than answering.
        assert!(!contains(&[], Pt { x: 0, y: 0 }));
        assert!(!contains(&[Pt { x: 0, y: 0 }], Pt { x: 0, y: 0 }));
        assert!(!contains(
            &[Pt { x: 0, y: 0 }, Pt { x: 64, y: 0 }],
            Pt { x: 32, y: 0 }
        ));
    }

    #[test]
    fn extreme_coordinates_do_not_panic() {
        assert!(is_axis_or_diagonal(
            Pt { x: i32::MIN, y: 0 },
            Pt { x: i32::MAX, y: 0 }
        ));
    }

    /// An octagon: a 256-unit square with each corner chamfered by 64 units,
    /// wound clockwise in the same "west-up, north-right, east-down,
    /// south-left" traversal `square` uses. Every edge is either
    /// axis-aligned or exactly 45 degrees, per `is_axis_or_diagonal` — the
    /// spec's `architecture.room_shapes` names octagonal rooms explicitly,
    /// but no fixture anywhere in this crate had a diagonal edge before this
    /// (see `KNOWN-GAPS.md`'s "no fixture anywhere has a 45-degree edge").
    fn octagon() -> Vec<Pt> {
        vec![
            Pt { x: 0, y: 64 },
            Pt { x: 0, y: 192 },
            Pt { x: 64, y: 256 },
            Pt { x: 192, y: 256 },
            Pt { x: 256, y: 192 },
            Pt { x: 256, y: 64 },
            Pt { x: 192, y: 0 },
            Pt { x: 64, y: 0 },
        ]
    }

    #[test]
    fn the_octagon_fixture_winds_clockwise() {
        assert!(
            is_clockwise(&octagon()),
            "fixture must match the Doom winding convention"
        );
    }

    #[test]
    fn containment_holds_across_a_diagonal_edge_not_only_axis_aligned_ones() {
        let o = octagon();
        // The NW chamfer (0,192)-(64,256) lies on the line x - y + 192 = 0;
        // its interior side (matching the square's own center) is positive.
        // (40,220) sits just inside it: 40 - 220 + 192 = 12 > 0.
        assert!(
            contains(&o, Pt { x: 40, y: 220 }),
            "just inside the diagonal chamfer"
        );
        // (16,240) sits in the corner the chamfer cuts away, on the outside
        // of the same line: 16 - 240 + 192 = -32 < 0 — inside the original
        // square's bounding box, but outside the octagon.
        assert!(
            !contains(&o, Pt { x: 16, y: 240 }),
            "just outside the diagonal chamfer, in the corner it cuts off"
        );
        // The center is comfortably inside, far from every edge.
        assert!(contains(&o, Pt { x: 128, y: 128 }));
    }

    #[test]
    fn clearance_measures_perpendicular_distance_to_a_diagonal_edge() {
        let o = octagon();
        // (40,220) sits near the NW chamfer (0,192)-(64,256), whose line is
        // x - y + 192 = 0. The perpendicular distance is
        // |40 - 220 + 192| / sqrt(2) = 12 / sqrt(2) = 6*sqrt(2), and the
        // foot of that perpendicular falls within the segment itself
        // (t = 0.53125 along it, computed independently by hand), so this
        // really is the nearest-edge distance, not just the nearest-line
        // one — and it is well short of the distance to any axis-aligned
        // wall (>= 36 in every direction from this point).
        let expected = 12.0 / std::f64::consts::SQRT_2;
        let d = clearance(&o, Pt { x: 40, y: 220 });
        assert!(
            (d - expected).abs() < 1e-9,
            "clearance {d} does not match the hand-derived perpendicular distance {expected}"
        );
    }
}
