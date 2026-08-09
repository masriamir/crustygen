//! Opens portals in shared walls and pairs the resulting two-sided linedefs.

use crate::compile::sectors::vertex_index;
use crate::compile::{CompileError, LinedefOut, MapData, SidedefOut};
use crate::geom::Pt;
use crate::ir::{Ir, Portal};

/// The axis a shared wall runs along.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Axis {
    /// The wall is vertical; X is constant.
    Vertical,
    /// The wall is horizontal; Y is constant.
    Horizontal,
}

/// Opens every portal in the shared wall of its two rooms.
///
/// Each side's wall segment is replaced by two flanking one-sided pieces, and a
/// single two-sided linedef carries the opening with its front and back
/// sidedefs bound to the two rooms' sectors.
///
/// # Errors
/// Returns [`CompileError::NotAdjacent`] when the rooms share no wall,
/// [`CompileError::PortalOffWall`] when the midpoint is not on that wall, and
/// [`CompileError::PortalTooWide`] when the opening exceeds the shared span.
pub fn cut_portals(ir: &Ir, data: &mut MapData) -> Result<(), CompileError> {
    for portal in &ir.portals {
        cut_one(ir, data, portal)?;
    }
    Ok(())
}

fn cut_one(ir: &Ir, data: &mut MapData, portal: &Portal) -> Result<(), CompileError> {
    let ia = ir
        .rooms
        .iter()
        .position(|r| r.id == portal.a)
        .expect("validated in Ir::from_json");
    let ib = ir
        .rooms
        .iter()
        .position(|r| r.id == portal.b)
        .expect("validated in Ir::from_json");

    let (axis, fixed, lo, hi, a_forward) =
        shared_span(ir, ia, ib).ok_or_else(|| CompileError::NotAdjacent {
            a: portal.a.clone(),
            b: portal.b.clone(),
        })?;

    let (on_axis, across) = match axis {
        Axis::Vertical => (portal.at.y, portal.at.x),
        Axis::Horizontal => (portal.at.x, portal.at.y),
    };
    if across != fixed || on_axis <= lo || on_axis >= hi {
        return Err(CompileError::PortalOffWall {
            a: portal.a.clone(),
            b: portal.b.clone(),
            x: portal.at.x,
            y: portal.at.y,
        });
    }

    let half = portal.width / 2;
    let (open_lo, open_hi) = (on_axis - half, on_axis + half);
    if open_lo < lo || open_hi > hi {
        return Err(CompileError::PortalTooWide {
            a: portal.a.clone(),
            b: portal.b.clone(),
            width: portal.width,
            available: hi - lo,
        });
    }

    let cut = Cut {
        axis,
        fixed,
        lo,
        open_lo,
        open_hi,
        hi,
    };

    // Drop the two solid wall segments the opening replaces — one per room.
    drop_wall_segment(data, cut.pt(lo), cut.pt(hi));

    let wall_tex = ir.rooms[ia].wall_tex.clone();
    emit_flanking_walls(data, &cut, ia, ib, a_forward, &wall_tex);
    emit_opening(data, &cut, ia, ib, a_forward);

    Ok(())
}

/// The span geometry of one portal's cut: which axis and fixed coordinate the
/// shared wall runs along, and the four along-axis boundaries from the solid
/// wall's low end, through the opening, to the solid wall's high end.
struct Cut {
    /// The axis the shared wall runs along.
    axis: Axis,
    /// The coordinate held constant along the wall (X for vertical, Y for
    /// horizontal).
    fixed: i32,
    /// The low end of the shared wall span.
    lo: i32,
    /// The low end of the opening.
    open_lo: i32,
    /// The high end of the opening.
    open_hi: i32,
    /// The high end of the shared wall span.
    hi: i32,
}

impl Cut {
    /// The point at `along` distance along this cut's axis.
    fn pt(&self, along: i32) -> Pt {
        match self.axis {
            Axis::Vertical => Pt {
                x: self.fixed,
                y: along,
            },
            Axis::Horizontal => Pt {
                x: along,
                y: self.fixed,
            },
        }
    }
}

/// Drops the one-sided linedef spanning `wall_a`..`wall_b`, in either
/// direction, that the opening replaces.
fn drop_wall_segment(data: &mut MapData, wall_a: Pt, wall_b: Pt) {
    data.linedefs.retain(|l| {
        let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
        let spans = (p == wall_a && q == wall_b) || (p == wall_b && q == wall_a);
        !(spans && l.back.is_none())
    });
}

/// Emits the flanking one-sided wall pieces either side of the opening, on
/// both rooms' sides.
///
/// `a_forward` records which side of the wall room A's interior sits on (from
/// `shared_span`); room B is always on the opposite side, so it takes the
/// opposite `v1`-to-`v2` direction to keep its front sidedef on the
/// geometric right.
fn emit_flanking_walls(
    data: &mut MapData,
    cut: &Cut,
    sector_a: usize,
    sector_b: usize,
    a_forward: bool,
    wall_tex: &str,
) {
    for (sector, forward) in [(sector_a, a_forward), (sector_b, !a_forward)] {
        for (s, e) in [(cut.lo, cut.open_lo), (cut.open_hi, cut.hi)] {
            if s == e {
                continue;
            }
            let (from, to) = if forward {
                (cut.pt(s), cut.pt(e))
            } else {
                (cut.pt(e), cut.pt(s))
            };
            let v1 = vertex_index(&mut data.vertices, from);
            let v2 = vertex_index(&mut data.vertices, to);
            let front = data.sidedefs.len();
            data.sidedefs.push(SidedefOut {
                sector,
                upper: String::new(),
                middle: wall_tex.to_string(),
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
}

/// Emits the opening itself: one two-sided line, front on room A, back on
/// room B.
///
/// `a_forward` (see `shared_span`) picks the `v1`-to-`v2` direction so room
/// A's front sidedef lands on the geometric right regardless of which side
/// of the wall room A sits on — the same rule `emit_flanking_walls` applies,
/// so front/back here name the correct bordering sector even after later
/// stages give the two rooms different floor or ceiling heights, or write
/// upper/lower textures onto these sidedefs (e.g. door tracks).
fn emit_opening(data: &mut MapData, cut: &Cut, sector_a: usize, sector_b: usize, a_forward: bool) {
    let (p1, p2) = if a_forward {
        (cut.pt(cut.open_lo), cut.pt(cut.open_hi))
    } else {
        (cut.pt(cut.open_hi), cut.pt(cut.open_lo))
    };
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_a,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
    });
    let back = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_b,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
    });
    data.linedefs.push(LinedefOut {
        v1,
        v2,
        front,
        back: Some(back),
        blocking: false,
        special: 0,
        tag: 0,
        lower_unpegged: false,
        upper_unpegged: false,
    });
}

/// Finds the axis, fixed coordinate, and overlapping span of two rooms' shared
/// wall, if they have one.
///
/// The returned `bool` is `a_forward`: whether room `ia` needs `cut_one`'s
/// increasing-coordinate walk direction for its flanking linedefs' front
/// sidedef to land on room `ia`'s side of the wall (room `ib`, always on the
/// opposite side, takes the negation). A linedef's front sidedef must name
/// the sector to the right of travel from `v1` to `v2`, matching the
/// convention `emit_sectors` establishes for every wall it emits. Rotating a
/// `+along` direction vector by -90 degrees lands on `+across` for a vertical
/// wall (`along` = Y, `across` = X) but `-across` for a horizontal wall
/// (`along` = X, `across` = Y) — the two axes are not mirror images of each
/// other, so `a_forward` is derived independently for each of the four
/// `(axis, which edge of ia touched)` combinations below rather than shared
/// across them.
fn shared_span(ir: &Ir, ia: usize, ib: usize) -> Option<(Axis, i32, i32, i32, bool)> {
    let bbox = |i: usize| {
        let f = &ir.rooms[i].footprint;
        let xs: Vec<i32> = f.iter().map(|p| p.x).collect();
        let ys: Vec<i32> = f.iter().map(|p| p.y).collect();
        (
            *xs.iter().min().expect("non-empty"),
            *xs.iter().max().expect("non-empty"),
            *ys.iter().min().expect("non-empty"),
            *ys.iter().max().expect("non-empty"),
        )
    };
    let (ax0, ax1, ay0, ay1) = bbox(ia);
    let (bx0, bx1, by0, by1) = bbox(ib);

    // `ia`'s east edge (max X) touches `ib`: `ia` is west of the wall, so its
    // interior is on the low-X side, opposite the increasing-Y walk direction.
    for (fixed, lo, hi, a_forward) in [
        (ax1, ay0.max(by0), ay1.min(by1), false),
        (ax0, ay0.max(by0), ay1.min(by1), true),
    ] {
        let touching = (fixed == bx0 || fixed == bx1) && lo < hi;
        if touching {
            return Some((Axis::Vertical, fixed, lo, hi, a_forward));
        }
    }
    // `ia`'s north edge (max Y) touches `ib`: `ia` is south of the wall, which
    // matches the increasing-X walk direction for a horizontal wall.
    for (fixed, lo, hi, a_forward) in [
        (ay1, ax0.max(bx0), ax1.min(bx1), true),
        (ay0, ax0.max(bx0), ax1.min(bx1), false),
    ] {
        let touching = (fixed == by0 || fixed == by1) && lo < hi;
        if touching {
            return Some((Axis::Horizontal, fixed, lo, hi, a_forward));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use crate::compile::CompileError;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::geom::{Pt, contains};
    use crate::ir::Ir;

    fn ir_with_portal(width: i32, at: (i32, i32), b_origin: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b", "footprint":[[{b_origin},0],[{b_origin},256],[{},256],[{},0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":{width}, "at":[{},{}] }}] }}"#,
            b_origin + 256,
            b_origin + 256,
            at.0,
            at.1
        )
    }

    #[test]
    fn a_portal_becomes_one_two_sided_linedef_pairing_both_sectors() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals cut");

        let two_sided: Vec<_> = data.linedefs.iter().filter(|l| l.back.is_some()).collect();
        assert_eq!(two_sided.len(), 1, "exactly one two-sided line");

        let l = two_sided[0];
        let front_sector = data.sidedefs[l.front].sector;
        let back_sector = data.sidedefs[l.back.expect("back")].sector;
        assert_ne!(
            front_sector, back_sector,
            "front and back name different sectors"
        );
        assert!(!l.blocking, "a portal does not block movement");
    }

    #[test]
    fn cutting_leaves_each_room_watertight() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        let before = data.linedefs.len();
        cut_portals(&ir, &mut data).expect("portals cut");
        // Each side's wall splits into two flanking one-sided pieces plus the
        // shared two-sided opening: 8 - 2 + 4 + 1 = 11.
        assert_eq!(
            data.linedefs.len(),
            before - 2 + 5,
            "wall split accounted for"
        );
    }

    #[test]
    fn rejects_a_portal_between_rooms_that_share_no_wall() {
        let ir = Ir::from_json(&ir_with_portal(128, (256, 128), 512)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::NotAdjacent { .. })
        ));
    }

    #[test]
    fn rejects_an_opening_wider_than_the_shared_wall() {
        let ir = Ir::from_json(&ir_with_portal(512, (256, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::PortalTooWide { .. })
        ));
    }

    #[test]
    fn rejects_an_opening_that_is_not_on_the_shared_wall() {
        let ir = Ir::from_json(&ir_with_portal(128, (128, 128), 256)).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(matches!(
            cut_portals(&ir, &mut data),
            Err(CompileError::PortalOffWall { .. })
        ));
    }

    /// Whether room `room_idx`'s original footprint has its interior on the
    /// right of travel from `p` to `q`.
    ///
    /// This is computed independently of `cut_portals`'s own `Cut`/`a_forward`
    /// bookkeeping: rotate the direction vector -90 degrees (`(dy, -dx)`,
    /// reduced to a one-unit step since every segment under test is
    /// axis-aligned) from the segment's midpoint, and ask `geom::contains`
    /// whether that probe point lands inside the room's footprint. A test
    /// that instead re-derived `a_forward` would just restate the
    /// implementation and could not catch a regression in it.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let probe = Pt {
            x: i32::midpoint(p.x, q.x) + dy.signum(),
            y: i32::midpoint(p.y, q.y) - dx.signum(),
        };
        contains(&ir.rooms[room_idx].footprint, probe)
    }

    /// Asserts that every linedef `cut_portals` leaves behind — the
    /// untouched walls, the flanking one-sided pieces, and the two-sided
    /// opening — has its front sidedef naming the sector whose interior is
    /// on the right of `v1`-to-`v2` travel, and (for the two-sided line) its
    /// back sidedef naming the sector on the left. This is the structural
    /// invariant the whole pass exists to guarantee: a linedef's declared
    /// sides must match its actual geometry, or the engine (and crustywad's
    /// own BSP/GL-node builder) attributes the wrong sector to the wrong
    /// side.
    fn assert_sidedefs_face_their_sectors(ir_json: &str) {
        let ir = Ir::from_json(ir_json).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals cut");

        for l in &data.linedefs {
            let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
            let front_sector = data.sidedefs[l.front].sector;
            assert!(
                interior_is_on_the_right(&ir, front_sector, p, q),
                "front sidedef of line {p:?} -> {q:?} names sector {front_sector}, \
                 but that room's interior is not on the right of travel"
            );
            if let Some(back) = l.back {
                let back_sector = data.sidedefs[back].sector;
                assert!(
                    interior_is_on_the_right(&ir, back_sector, q, p),
                    "back sidedef of line {p:?} -> {q:?} names sector {back_sector}, \
                     but that room's interior is not on the left of travel"
                );
            }
        }
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_west_of_a_vertical_wall() {
        assert_sidedefs_face_their_sectors(&ir_with_portal(128, (256, 128), 256));
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_east_of_a_vertical_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;
        assert_sidedefs_face_their_sectors(ir_json);
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_south_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[128,256] }] }"#;
        assert_sidedefs_face_their_sectors(ir_json);
    }

    #[test]
    fn sidedefs_face_their_sectors_when_room_a_is_north_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[128,256] }] }"#;
        assert_sidedefs_face_their_sectors(ir_json);
    }
}
