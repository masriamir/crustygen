//! Turns room footprints into sectors, sidedefs, and one-sided linedefs.

use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};
use crate::geom::{Pt, contains, edges, is_axis_or_diagonal, is_clockwise, shoelace2};
use crate::ir::Ir;
use crate::tables::Tables;

/// Returns the index of `p`, appending it if it is not already present.
///
/// Deduplication is what makes shared walls actually join rather than meet at
/// coincident-but-distinct vertices.
pub fn vertex_index(verts: &mut Vec<Pt>, p: Pt) -> usize {
    if let Some(i) = verts.iter().position(|&v| v == p) {
        return i;
    }
    verts.push(p);
    verts.len() - 1
}

/// Emits one closed sector per room, with all walls one-sided.
///
/// Portals reopen these walls in a later pass; starting fully closed means a
/// room is watertight by construction.
///
/// # Errors
/// Returns [`CompileError::Degenerate`], [`CompileError::NotClockwise`],
/// [`CompileError::BadEdge`], or [`CompileError::Overlap`] as described on each
/// variant. Nothing is emitted when any room fails.
pub fn emit_sectors(ir: &Ir) -> Result<MapData, CompileError> {
    for room in &ir.rooms {
        if room.footprint.len() < 3 || shoelace2(&room.footprint) == 0 {
            return Err(CompileError::Degenerate {
                room: room.id.clone(),
            });
        }
        if !is_clockwise(&room.footprint) {
            return Err(CompileError::NotClockwise {
                room: room.id.clone(),
            });
        }
        for (a, b) in edges(&room.footprint) {
            if !is_axis_or_diagonal(a, b) {
                return Err(CompileError::BadEdge {
                    room: room.id.clone(),
                    x1: a.x,
                    y1: a.y,
                    x2: b.x,
                    y2: b.y,
                });
            }
        }
    }

    for (i, a) in ir.rooms.iter().enumerate() {
        for b in &ir.rooms[i + 1..] {
            if overlaps(&a.footprint, &b.footprint) {
                return Err(CompileError::Overlap {
                    a: a.id.clone(),
                    b: b.id.clone(),
                });
            }
        }
    }

    let mut data = MapData::default();
    for room in &ir.rooms {
        let sector = data.sectors.len();
        data.sectors.push(SectorOut {
            floor: room.floor,
            ceiling: room.ceiling,
            light: room.light,
            floor_tex: room.floor_tex.clone(),
            ceil_tex: room.ceil_tex.clone(),
            special: room.special.unwrap_or(0),
            tag: 0,
        });

        for (a, b) in edges(&room.footprint) {
            let v1 = vertex_index(&mut data.vertices, a);
            let v2 = vertex_index(&mut data.vertices, b);
            let front = data.sidedefs.len();
            data.sidedefs.push(SidedefOut {
                sector,
                upper: String::new(),
                middle: room.wall_tex.clone(),
                lower: String::new(),
            });
            data.linedefs.push(LinedefOut {
                v1,
                v2,
                front,
                back: None,
                blocking: true,
                special: 0,
                tag: 0,
                lower_unpegged: false,
                upper_unpegged: false,
            });
        }
    }
    Ok(data)
}

/// Resolves each room's [`crate::ir::Room::secret`] flag into rule P18's
/// secret sector special, unless [`crate::ir::Room::special`] already set one
/// explicitly.
///
/// `Room::special` is the escape hatch documented on that field:
/// [`Ir::from_json`] already rejects a room that sets both, so by the time
/// this runs, a room with `secret == true` is guaranteed to have
/// `special == None` — no precedence to pick between them here, just a
/// straight substitution. Must run after [`emit_sectors`], which is what
/// populates `data.sectors` in the first place, and before anything that
/// might read `.special` (nothing downstream currently does, but this keeps
/// the ordering obvious).
///
/// Relies on `emit_sectors` pushing exactly one sector per room, in
/// `ir.rooms` order — the same invariant `compile::things` documents and
/// depends on.
pub fn resolve_secret_specials(ir: &Ir, tables: &Tables, data: &mut MapData) {
    for (i, room) in ir.rooms.iter().enumerate() {
        if room.secret {
            data.sectors[i].special = tables.secret_sector_special();
        }
    }
}

/// Twice the signed area of triangle `abc`; positive when `c` is left of `ab`.
fn orient(a: Pt, b: Pt, c: Pt) -> i64 {
    let (abx, aby) = (i64::from(b.x - a.x), i64::from(b.y - a.y));
    let (acx, acy) = (i64::from(c.x - a.x), i64::from(c.y - a.y));
    abx * acy - aby * acx
}

/// Whether two segments cross at a point interior to both.
///
/// Proper crossings only — segments that merely touch at an endpoint or run
/// collinearly do not count, so rooms sharing a wall are not overlapping.
fn segments_properly_cross(a1: Pt, a2: Pt, b1: Pt, b2: Pt) -> bool {
    let (d1, d2) = (orient(a1, a2, b1), orient(a1, a2, b2));
    let (d3, d4) = (orient(b1, b2, a1), orient(b1, b2, a2));
    d1 != 0 && d2 != 0 && d3 != 0 && d4 != 0 && (d1 > 0) != (d2 > 0) && (d3 > 0) != (d4 > 0)
}

/// Whether a point lies on an edge of the polygon.
fn point_on_polygon_boundary(poly: &[Pt], p: Pt) -> bool {
    edges(poly).any(|(a, b)| {
        let cross = (i64::from(p.y) - i64::from(a.y)) * (i64::from(b.x) - i64::from(a.x))
            - (i64::from(p.x) - i64::from(a.x)) * (i64::from(b.y) - i64::from(a.y));
        if cross != 0 {
            return false;
        }
        let min_x = a.x.min(b.x);
        let max_x = a.x.max(b.x);
        let min_y = a.y.min(b.y);
        let max_y = a.y.max(b.y);
        p.x >= min_x && p.x <= max_x && p.y >= min_y && p.y <= max_y
    })
}

/// Whether two footprints share interior area.
///
/// Three probes, because no one of them is sufficient. Vertex containment
/// catches nesting; edge-midpoint containment catches a shared band where two
/// rooms overlap without either's corner falling strictly inside the other;
/// proper edge crossing catches a plus-shaped overlap where neither holds.
/// Footprints that merely share a wall are allowed — that is how portals work.
///
/// This is a heuristic tuned for the rectilinear, grid-snapped footprints v1
/// accepts, not a general polygon boolean.
fn overlaps(a: &[Pt], b: &[Pt]) -> bool {
    let probes = |poly: &[Pt]| -> Vec<Pt> {
        let mut pts: Vec<Pt> = poly.to_vec();
        pts.extend(edges(poly).map(|(p, q)| Pt {
            x: i32::midpoint(p.x, q.x),
            y: i32::midpoint(p.y, q.y),
        }));
        pts
    };
    if probes(a)
        .into_iter()
        .any(|p| !point_on_polygon_boundary(b, p) && contains(b, p))
        || probes(b)
            .into_iter()
            .any(|p| !point_on_polygon_boundary(a, p) && contains(a, p))
    {
        return true;
    }
    edges(a).any(|(p1, p2)| edges(b).any(|(q1, q2)| segments_properly_cross(p1, p2, q1, q2)))
}

#[cfg(test)]
mod tests {
    use crate::compile::CompileError;
    use crate::compile::sectors::emit_sectors;
    use crate::ir::Ir;

    fn ir_with(footprint_a: &str) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":{footprint_a},
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }},
                {{ "id":"b", "footprint":[[256,0],[256,256],[512,256],[512,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }}
              ], "portals":[] }}"#
        )
    }

    const CW: &str = "[[0,0],[0,256],[256,256],[256,0]]";

    #[test]
    fn emits_one_sector_per_room_with_shared_vertices_deduplicated() {
        let ir = Ir::from_json(&ir_with(CW)).expect("ir");
        let data = emit_sectors(&ir).expect("emits");
        assert_eq!(data.sectors.len(), 2, "one sector per room");
        // Two 256-squares meeting along x=256 share two corners: 8 - 2 = 6.
        assert_eq!(data.vertices.len(), 6, "shared corners deduplicated");
        assert_eq!(data.linedefs.len(), 8, "four walls per room");
        assert!(
            data.linedefs.iter().all(|l| l.back.is_none()),
            "all one-sided so far"
        );
    }

    #[test]
    fn rejects_a_counter_clockwise_footprint() {
        let ccw = "[[256,0],[256,256],[0,256],[0,0]]";
        let ir = Ir::from_json(&ir_with(ccw)).expect("ir");
        assert!(matches!(
            emit_sectors(&ir),
            Err(CompileError::NotClockwise { .. })
        ));
    }

    #[test]
    fn rejects_an_overlapping_footprint() {
        let overlapping = "[[0,0],[0,256],[320,256],[320,0]]";
        let ir = Ir::from_json(&ir_with(overlapping)).expect("ir");
        assert!(matches!(
            emit_sectors(&ir),
            Err(CompileError::Overlap { .. })
        ));
    }

    #[test]
    fn rejects_an_edge_that_is_neither_axis_aligned_nor_diagonal() {
        let skew = "[[0,0],[0,256],[256,192],[256,0]]";
        let ir = Ir::from_json(&ir_with(skew)).expect("ir");
        assert!(matches!(
            emit_sectors(&ir),
            Err(CompileError::BadEdge { .. })
        ));
    }

    #[test]
    fn rejects_a_degenerate_footprint() {
        let two_points = "[[0,0],[0,256]]";
        let ir = Ir::from_json(&ir_with(two_points)).expect("ir");
        assert!(matches!(
            emit_sectors(&ir),
            Err(CompileError::Degenerate { .. })
        ));
    }

    /// A room shaped 128x64 (not the ubiquitous 256-square), one secret and
    /// one not, so a fixture-diversity mutation cannot hide behind a shared
    /// dimension.
    const SECRET_ROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"vault", "footprint":[[0,0],[0,64],[128,64],[128,0]],
          "floor":0, "ceiling":128, "light":160, "secret":true,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"hall", "footprint":[[128,0],[128,64],[256,64],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ], "portals":[] }"#;

    #[test]
    fn a_secret_room_gets_the_sourced_secret_special_and_a_plain_one_gets_zero() {
        use crate::compile::sectors::resolve_secret_specials;
        use crate::tables::Tables;

        let ir = Ir::from_json(SECRET_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert_eq!(
            data.sectors[0].special, 0,
            "before resolution, secret is not yet reflected in the raw special"
        );
        resolve_secret_specials(&ir, &tables, &mut data);
        assert_eq!(
            data.sectors[0].special,
            tables.secret_sector_special(),
            "the secret room's sector carries the sourced secret special"
        );
        assert_eq!(
            data.sectors[1].special, 0,
            "a room that never set secret or special stays at 0"
        );
    }

    #[test]
    fn an_explicit_special_is_left_untouched_when_secret_is_not_set() {
        use crate::compile::sectors::resolve_secret_specials;
        use crate::tables::Tables;

        let ir_json = SECRET_ROOM.replace("\"secret\":true", "\"special\":42");
        let ir = Ir::from_json(&ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        resolve_secret_specials(&ir, &tables, &mut data);
        assert_eq!(
            data.sectors[0].special, 42,
            "the escape-hatch special is untouched by secret resolution"
        );
    }
}
