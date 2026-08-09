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
}
