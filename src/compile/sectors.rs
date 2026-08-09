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

/// A human-readable label for sector `sector`, for
/// [`CompileError::SectorOverlap`].
///
/// A room sector is named by its own id; a compiler-generated sector (a
/// portal's gap sector or a walkover exit's alcove) has no IR-level name, so
/// it is named by its index instead — an author can still find it via the
/// representative coordinate the error carries alongside it.
fn sector_label(ir: &Ir, sector: usize) -> String {
    ir.rooms.get(sector).map_or_else(
        || format!("sector {sector}"),
        |room| format!("room `{}`", room.id),
    )
}

/// The polygon `sector` actually occupies in the emitted geometry.
///
/// A room sector's polygon is its IR-declared footprint, unmodified — rooms
/// are never reshaped by portal, door, or exit construction (see
/// `KNOWN-GAPS.md`'s wall-thickness entry). A compiler-generated sector (a
/// portal's gap sector, built by `portals::emit_gap_sector`, or a walkover
/// exit's alcove, built by `exits::emit_walkover_exit`) has no IR footprint;
/// every one of them is an axis-aligned rectangle by construction, so the
/// bounding box of every vertex on a linedef bordering it *is* its exact
/// shape.
fn sector_polygon(ir: &Ir, data: &MapData, sector: usize) -> Vec<Pt> {
    if let Some(room) = ir.rooms.get(sector) {
        return room.footprint.clone();
    }
    let verts: Vec<Pt> = data
        .linedefs
        .iter()
        .filter(|l| {
            data.sidedefs[l.front].sector == sector
                || l.back.is_some_and(|b| data.sidedefs[b].sector == sector)
        })
        .flat_map(|l| [data.vertices[l.v1], data.vertices[l.v2]])
        .collect();
    let x_lo = verts
        .iter()
        .map(|v| v.x)
        .min()
        .expect("every compiler-generated sector has bordering geometry");
    let x_hi = verts
        .iter()
        .map(|v| v.x)
        .max()
        .expect("every compiler-generated sector has bordering geometry");
    let y_lo = verts
        .iter()
        .map(|v| v.y)
        .min()
        .expect("every compiler-generated sector has bordering geometry");
    let y_hi = verts
        .iter()
        .map(|v| v.y)
        .max()
        .expect("every compiler-generated sector has bordering geometry");
    vec![
        Pt { x: x_lo, y: y_lo },
        Pt { x: x_lo, y: y_hi },
        Pt { x: x_hi, y: y_hi },
        Pt { x: x_hi, y: y_lo },
    ]
}

/// Rejects any two emitted sectors — rooms, portal gap sectors, or exit
/// alcoves — that overlap in 2-D.
///
/// [`emit_sectors`]'s own overlap check only ever compares IR room
/// footprints against each other, so it cannot catch a portal's gap sector
/// driven through a third room's interior, or two gap sectors from
/// different, unrelated portals crossing each other at a right angle —
/// neither involves two room footprints overlapping, so nothing upstream
/// complains. Must run after every sector-emitting pass (`emit_sectors`,
/// `cut_portals`, `emit_doors`, `emit_exits`), since it inspects the final
/// sector count and the final emitted geometry, not the IR alone.
///
/// # Errors
/// Returns [`CompileError::SectorOverlap`] naming both sectors and a
/// representative point (one of the sector's own corners) inside each.
///
/// # Panics
/// Panics if any sector's polygon is empty — unreachable, since a room's
/// footprint always has at least three points ([`emit_sectors`] rejects
/// [`CompileError::Degenerate`] otherwise) and every compiler-generated
/// sector always has at least the four corners
/// `portals::emit_gap_sector`/`exits::emit_walkover_exit` build it from.
pub fn check_no_sector_overlaps(ir: &Ir, data: &MapData) -> Result<(), CompileError> {
    let polygons: Vec<Vec<Pt>> = (0..data.sectors.len())
        .map(|s| sector_polygon(ir, data, s))
        .collect();
    for (i, poly_i) in polygons.iter().enumerate() {
        for (j, poly_j) in polygons.iter().enumerate().skip(i + 1) {
            if overlaps(poly_i, poly_j) {
                let pi = *poly_i.first().expect("a sector's polygon has a vertex");
                let pj = *poly_j.first().expect("a sector's polygon has a vertex");
                return Err(CompileError::SectorOverlap {
                    first: sector_label(ir, i),
                    first_x: pi.x,
                    first_y: pi.y,
                    second: sector_label(ir, j),
                    second_x: pj.x,
                    second_y: pj.y,
                });
            }
        }
    }
    Ok(())
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

    /// An octagon: a 256-unit square with each corner chamfered by 64 units.
    /// Every edge is either axis-aligned or exactly 45 degrees. The spec's
    /// `architecture.room_shapes` names octagonal rooms explicitly, but no
    /// fixture anywhere in this crate had a diagonal edge before this — see
    /// `KNOWN-GAPS.md`'s "no fixture anywhere has a 45-degree edge".
    const OCTAGON: &str = "[[0,64],[0,192],[64,256],[192,256],[256,192],[256,64],[192,0],[64,0]]";

    #[test]
    fn a_diagonally_shaped_room_emits_one_closed_sector() {
        // Room b is dropped entirely here (unlike `ir_with`, which always
        // pairs the fixture against a second square) so this pins the
        // octagon's own emitted counts exactly, not diluted by room b's.
        let ir_json = format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":{OCTAGON},
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ], "portals":[] }}"#
        );
        let ir = Ir::from_json(&ir_json).expect("ir");
        let data = emit_sectors(&ir).expect("emits");
        assert_eq!(data.sectors.len(), 1, "one sector for the one room");
        assert_eq!(data.vertices.len(), 8, "all eight corners, none shared");
        assert_eq!(data.linedefs.len(), 8, "eight walls, four of them diagonal");
        assert!(
            data.linedefs.iter().all(|l| l.back.is_none()),
            "a room with no portals stays fully one-sided, diagonal walls included"
        );
    }

    /// A 320x256 rectangle chamfered by 64 units at each corner — the same
    /// chamfer construction as [`OCTAGON`], just wide enough (past x = 256)
    /// to genuinely overlap `ir_with`'s room b, which spans x in
    /// [256,512]: at y = 128 this shape's straight east flank reaches all
    /// the way to x = 320, well past room b's own x = 256 wall, so the two
    /// interiors — not merely their bounding boxes — actually intersect.
    const WIDE_CHAMFERED: &str =
        "[[0,64],[0,192],[64,256],[256,256],[320,192],[320,64],[256,0],[64,0]]";

    #[test]
    fn a_diagonally_shaped_room_overlapping_a_square_is_rejected() {
        // Exercises `overlaps`'s vertex/edge-midpoint probes against a
        // footprint whose boundary is partly diagonal, not just the
        // axis-aligned case every other overlap test in this file covers.
        let ir = Ir::from_json(&ir_with(WIDE_CHAMFERED)).expect("ir");
        assert!(matches!(
            emit_sectors(&ir),
            Err(CompileError::Overlap { .. })
        ));
    }

    #[test]
    fn two_diagonally_shaped_rooms_that_only_share_a_wall_do_not_overlap() {
        // Two right triangles splitting a 64-unit square along its own
        // diagonal from (0,0) to (64,64): room a is the upper-left half,
        // room b the lower-right half. They share the entire diagonal as a
        // real wall but no interior area — `overlaps` must not confuse "the
        // whole wall is diagonal" with "the footprints overlap".
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,64],[64,64]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[64,64],[64,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ], "portals":[] }"#;
        let ir = Ir::from_json(ir_json).expect("ir");
        let data = emit_sectors(&ir).expect("two triangles sharing only a diagonal wall compile");
        assert_eq!(data.sectors.len(), 2, "one sector per triangle");
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

    // The tests below cover `check_no_sector_overlaps`: gap sectors are
    // compiler-generated geometry with no IR footprint, so
    // `emit_sectors`'s own room-vs-room overlap check (above) cannot see a
    // gap sector colliding with anything. Every fixture here compiles clean
    // through room-footprint overlap checking and portal/door/exit
    // resolution; only the 2-D sector-polygon check added for this task
    // catches them.

    use crate::compile::MapData;
    use crate::compile::doors::emit_doors;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::check_no_sector_overlaps;
    use crate::compile::tags::TagAllocator;
    use crate::tables::Tables;

    /// Runs the full geometry pipeline (sectors -> portals -> doors), the
    /// same passes `compile::compile_reporting` runs before its own call to
    /// `check_no_sector_overlaps`.
    fn compiled_data(ir: &Ir) -> MapData {
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(ir).expect("sectors");
        cut_portals(ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        emit_doors(ir, &tables, &mut data, &mut tags).expect("doors");
        data
    }

    /// Two *perpendicular* plain portals whose gap sectors genuinely
    /// overlap, even though no two rooms overlap and neither portal shares
    /// a room with the other. Room `a` (x 0..256) and room `b` (x 320..576)
    /// face each other across x, gap rectangle [256,320]x[64,192]. Room `c`
    /// (its own north wall at y=56) and room `d` (its own south wall at
    /// y=200) face each other across y over x 264..312, gap rectangle
    /// [272,304]x[56,200] once the portal's width narrows it from the
    /// walls' own 264..312 extent. The two gap rectangles share real 2-D
    /// area: x in [272,304], y in [64,192]. `grid: 8` — none of these
    /// coordinates are multiples of 64.
    const PERPENDICULAR_GAP_SECTORS_COLLIDE: &str = r#"{ "seed":1, "grid":8, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"c", "footprint":[[264,-96],[264,56],[312,56],[312,-96]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"d", "footprint":[[264,200],[264,456],[312,456],[312,200]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[
        { "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] },
        { "a":"c", "b":"d", "kind":"plain", "width":32, "at":[288,56] }
      ] }"#;

    #[test]
    fn two_perpendicular_gap_sectors_that_collide_are_rejected() {
        let ir = Ir::from_json(PERPENDICULAR_GAP_SECTORS_COLLIDE).expect("ir");
        let data = compiled_data(&ir);
        assert!(
            matches!(
                check_no_sector_overlaps(&ir, &data),
                Err(CompileError::SectorOverlap { .. })
            ),
            "two gap sectors from unrelated, perpendicular portals overlap in 2-D and must be \
             rejected, even though no two rooms overlap and neither portal shares a room with \
             the other"
        );
    }

    /// The same fixture, translated so the two gap rectangles no longer
    /// intersect (room `c`/`d`'s corridor moved out to x 600..648, well
    /// clear of the a<->b corridor at x 256..320): proves the rejection
    /// above is really about the 2-D intersection, not merely the presence
    /// of two portals in the same map.
    #[test]
    fn two_perpendicular_gap_sectors_that_do_not_collide_are_accepted() {
        let moved = PERPENDICULAR_GAP_SECTORS_COLLIDE
            .replace("264,-96", "600,-96")
            .replace("264,56", "600,56")
            .replace("312,56", "648,56")
            .replace("312,-96", "648,-96")
            .replace("264,200", "600,200")
            .replace("264,456", "600,456")
            .replace("312,456", "648,456")
            .replace("312,200", "648,200")
            .replace("\"at\":[288,56]", "\"at\":[624,56]");
        let ir = Ir::from_json(&moved).expect("ir");
        let data = compiled_data(&ir);
        assert!(
            check_no_sector_overlaps(&ir, &data).is_ok(),
            "two gap sectors that do not share any 2-D area must not be rejected"
        );
    }

    /// Room `void_room` sits entirely inside the a<->b corridor (a<->b's
    /// gap rectangle is [256,320]x[64,192]; `void_room` is
    /// [264,312]x[96,160], strictly inside it) but overlaps neither `a` nor
    /// `b`'s own footprint, so `emit_sectors`'s room-vs-room overlap check
    /// cannot see it — only the gap-sector-vs-room check can.
    fn portal_through_third_room_ir(kind: &str) -> String {
        format!(
            r#"{{ "seed":1, "grid":8, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"void_room", "footprint":[[264,96],[264,160],[312,160],[312,96]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"{kind}", "width":128, "at":[256,128] }}] }}"#
        )
    }

    #[test]
    fn a_plain_portals_gap_sector_driven_through_a_third_room_is_rejected() {
        let ir = Ir::from_json(&portal_through_third_room_ir("plain")).expect("ir");
        let data = compiled_data(&ir);
        assert!(
            matches!(
                check_no_sector_overlaps(&ir, &data),
                Err(CompileError::SectorOverlap { .. })
            ),
            "a plain portal's passage sector driven through an unrelated third room's \
             interior must be rejected"
        );
    }

    #[test]
    fn a_doors_gap_sector_driven_through_a_third_room_is_rejected() {
        let ir = Ir::from_json(&portal_through_third_room_ir("door")).expect("ir");
        let data = compiled_data(&ir);
        assert!(
            matches!(
                check_no_sector_overlaps(&ir, &data),
                Err(CompileError::SectorOverlap { .. })
            ),
            "a door's own sector driven through an unrelated third room's interior must be \
             rejected"
        );
    }
}
