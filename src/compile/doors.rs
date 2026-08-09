//! Emits a dedicated, closed sector for each door portal.
//!
//! With adjacent rooms sharing a wall at zero distance (see the task-8
//! report), a door sector needs its area carved out of one side. This pass
//! carves it entirely out of room `b`: room `a`'s boundary is left exactly
//! as `cut_portals` leaves it for a plain portal, and room `b`'s wall at the
//! portal's span steps back by `DOOR_DEPTH` to make room for the door
//! sector between them. Two "face" lines (perpendicular to the direction of
//! travel through the doorway, one bordering room `a` and one bordering room
//! `b`) and two "jamb" lines (parallel to it, closing the recess's short
//! ends, bordering room `b` and the door) give the door sector — and room
//! `b` — real, closed boundaries.
//!
//! This runs after [`crate::compile::portals::cut_portals`], which leaves a
//! door portal's flanking walls in place but no opening line — the opening
//! is exactly what this pass constructs.

use crate::compile::portals::{Axis, Cut, emit_opening, shared_span};
use crate::compile::sectors::vertex_index;
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};
use crate::ir::{Ir, PortalKind};
use crate::tables::Tables;

/// How far a door sector's slab extends into room `b`'s territory, in map
/// units.
///
/// This is a compiler construction constant, not an engine-derived one:
/// unlike the values in `engine.toml`, Doom's engine places no constraint at
/// all on how thick a door recess is — that is purely a mapping convention,
/// so citing a primary engine source for it the way `engine.toml`'s other
/// constants do would misrepresent what kind of fact this is. 16 units is a
/// common, functional door-slab depth in hand-built maps: enough to read as
/// a real doorway without eating excessive room area. It is deliberately not
/// tied to the IR's `grid` — the vertices it produces are compiler output,
/// not author input, and are not subject to `IrError::OffGrid`.
const DOOR_DEPTH: i32 = 16;

/// Emits a dedicated, initially closed sector for every door portal.
///
/// See the module documentation for the construction. Every line touching
/// the new sector is lower-unpegged so its texture does not slide as the
/// sector's ceiling later animates open (P11), and the sector carries a
/// unique nonzero tag from `tags`.
///
/// # Errors
/// Returns [`CompileError::NotAdjacent`] if a door portal's rooms are not
/// adjacent, which indicates `cut_portals` did not validate it first.
///
/// # Panics
/// Panics if a portal names a room id absent from `ir.rooms` — unreachable
/// in practice, since [`Ir::from_json`] already rejects that.
pub fn emit_doors(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
) -> Result<(), CompileError> {
    let door_tex = tables
        .texture("door", &ir.theme)
        .unwrap_or("BIGDOOR2")
        .to_owned();
    let track_tex = tables
        .texture("door_track", &ir.theme)
        .unwrap_or("DOORTRAK")
        .to_owned();

    for portal in &ir.portals {
        if !matches!(portal.kind, PortalKind::Door | PortalKind::Locked) {
            continue;
        }

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

        let on_axis = match axis {
            Axis::Vertical => portal.at.y,
            Axis::Horizontal => portal.at.x,
        };
        let half = portal.width / 2;
        let (open_lo, open_hi) = (on_axis - half, on_axis + half);

        // Recess into room `b`: `far` moves away from room `a`'s side by
        // `DOOR_DEPTH`. This is the sign-flipped mirror of the rule
        // `shared_span` uses to pick `a_forward` for travel *along* the
        // wall — here the travel is *across* it, so vertical and horizontal
        // walls invert relative to `a_forward` in the opposite pattern. See
        // the task-8 report for the full derivation and worked examples.
        let axis_sign = match axis {
            Axis::Vertical => 1,
            Axis::Horizontal => -1,
        };
        let far = fixed + if a_forward { -axis_sign } else { axis_sign } * DOOR_DEPTH;

        let floor = ir.rooms[ia].floor.min(ir.rooms[ib].floor);
        let sector = data.sectors.len();
        let tag = tags.allocate(sector, &format!("door {} <-> {}", portal.a, portal.b));
        data.sectors.push(SectorOut {
            floor,
            // A closed door: ceiling snapped to the floor.
            ceiling: floor,
            light: ir.rooms[ia].light,
            floor_tex: ir.rooms[ia].floor_tex.clone(),
            ceil_tex: ir.rooms[ia].ceil_tex.clone(),
            special: 0,
            tag,
        });

        let near_cut = Cut {
            axis,
            fixed,
            lo,
            open_lo,
            open_hi,
            hi,
        };
        let far_cut = Cut {
            axis,
            fixed: far,
            lo,
            open_lo,
            open_hi,
            hi,
        };

        // The two face lines, perpendicular to the direction of travel
        // through the doorway: room `a` <-> door, then door <-> room `b`.
        // Both use the same `a_forward` — the door sector sits on the same
        // side of each face that room `a` sits on relative to the near
        // face, so the orientation rule `emit_opening` already applies
        // carries over unchanged to the far face.
        let near_line = emit_opening(data, &near_cut, ia, sector, a_forward);
        let far_line = emit_opening(data, &far_cut, sector, ib, a_forward);
        for line in [near_line, far_line] {
            data.linedefs[line].lower_unpegged = true;
            let front = data.linedefs[line].front;
            let back = data.linedefs[line]
                .back
                .expect("emit_opening always emits a two-sided line");
            data.sidedefs[front].upper.clone_from(&door_tex);
            data.sidedefs[back].upper.clone_from(&door_tex);
        }

        // The two jamb lines, parallel to the direction of travel, closing
        // the recess's short ends. Both border room `b` on one side and the
        // door sector on the other, regardless of which `shared_span`
        // sub-case applies — only the `v1`/`v2` direction differs between
        // them, which is exactly what `!a_forward`/`a_forward` picks.
        emit_jamb(
            data, &near_cut, &far_cut, open_lo, !a_forward, ib, sector, &track_tex,
        );
        emit_jamb(
            data, &near_cut, &far_cut, open_hi, a_forward, ib, sector, &track_tex,
        );
    }
    Ok(())
}

/// Emits one jamb: the short line closing one end of a door sector's recess,
/// running between `near_cut`'s and `far_cut`'s coordinate at `along`, front
/// bound to room `b`, back to the door sector.
///
/// `forward` picks the `v1`-to-`v2` direction — `near_cut`'s point to
/// `far_cut`'s, or the reverse — so that front lands on room `b`'s side
/// regardless of which of the four `shared_span` sub-cases applies. Front is
/// always room `b` at both jambs; only this direction varies. See the
/// task-8 report for the derivation.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter names an independent piece of the jamb's geometry; \
              bundling them would just move the same count into a throwaway struct"
)]
fn emit_jamb(
    data: &mut MapData,
    near_cut: &Cut,
    far_cut: &Cut,
    along: i32,
    forward: bool,
    sector_b: usize,
    sector_door: usize,
    texture: &str,
) -> usize {
    let (near_pt, far_pt) = (near_cut.pt(along), far_cut.pt(along));
    let (p1, p2) = if forward {
        (near_pt, far_pt)
    } else {
        (far_pt, near_pt)
    };
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_b,
        upper: texture.to_string(),
        middle: String::new(),
        lower: String::new(),
    });
    let back = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: sector_door,
        upper: texture.to_string(),
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
        lower_unpegged: true,
        upper_unpegged: false,
    });
    data.linedefs.len() - 1
}

#[cfg(test)]
mod tests {
    use crate::compile::MapData;
    use crate::compile::doors::emit_doors;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::tags::TagAllocator;
    use crate::geom::{Pt, contains};
    use crate::ir::Ir;
    use crate::tables::Tables;

    const DOOR_IR: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[256,0],[256,256],[512,256],[512,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128] }] }"#;

    /// Runs the full `emit_sectors` -> `cut_portals` -> `emit_doors`
    /// pipeline and returns the resulting `MapData` plus the door sector's
    /// index (always the last sector, since `emit_doors` only appends).
    fn compiled(ir_json: &str) -> (Ir, MapData, usize) {
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        let door = data.sectors.len() - 1;
        (ir, data, door)
    }

    #[test]
    fn a_door_portal_gets_its_own_closed_sector_with_a_unique_tag() {
        let ir = Ir::from_json(DOOR_IR).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();

        let rooms_before = data.sectors.len();
        emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");

        assert_eq!(
            data.sectors.len(),
            rooms_before + 1,
            "one sector added for the door"
        );
        let door = data.sectors.last().expect("door sector");
        assert_eq!(
            door.ceiling, door.floor,
            "a closed door has its ceiling snapped to its floor"
        );
        assert_ne!(door.tag, 0, "the door sector carries a real tag");
        assert_eq!(tags.manifest().len(), 1, "the allocation is recorded");
    }

    #[test]
    fn door_lines_are_lower_unpegged_so_the_track_does_not_slide() {
        let (_, data, door) = compiled(DOOR_IR);
        let touches_door = |l: &crate::compile::LinedefOut| {
            data.sidedefs[l.front].sector == door
                || l.back.is_some_and(|b| data.sidedefs[b].sector == door)
        };
        assert!(
            data.linedefs
                .iter()
                .filter(|l| touches_door(l))
                .all(|l| l.lower_unpegged),
            "every line on the door sector is lower-unpegged"
        );
    }

    #[test]
    fn a_plain_portal_adds_no_sector() {
        let plain = DOOR_IR.replace("\"door\"", "\"plain\"");
        let ir = Ir::from_json(&plain).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let before = data.sectors.len();
        let mut tags = TagAllocator::new();
        emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        assert_eq!(
            data.sectors.len(),
            before,
            "no door sector for a plain portal"
        );
    }

    /// Whether room `room_idx`'s *original* IR footprint has its interior on
    /// the right of travel from `p` to `q`. Deliberately independent of
    /// `portals`/`doors` — see `portals::tests::interior_is_on_the_right`,
    /// which this mirrors: duplicated rather than shared, because sharing it
    /// with production code would let a shared bug hide from both call
    /// sites. Valid for any line that borders a room's *true* remaining
    /// territory; the door construction never routes a real room-facing
    /// sidedef through the recessed sliver itself (see the task-8 report),
    /// so this check applies unchanged to every room-facing sidedef the
    /// door construction produces.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let probe = Pt {
            x: i32::midpoint(p.x, q.x) + dy.signum(),
            y: i32::midpoint(p.y, q.y) - dx.signum(),
        };
        contains(&ir.rooms[room_idx].footprint, probe)
    }

    /// Asserts one `shared_span` sub-case's door construction against
    /// coordinates hand-derived from the fixture's own footprints — not by
    /// calling `shared_span`/`emit_doors` internals — and checks:
    ///
    /// - the door sector is bounded by exactly four sidedefs (a closed
    ///   quadrilateral, not a dangling single sidedef);
    /// - its four corners are exactly the independently expected ones;
    /// - room `a` keeps its plain-portal boundary shape (six sides: three
    ///   untouched walls, two flanking pieces, the near face's front);
    /// - room `b` gains the recess notch (eight sides: three untouched
    ///   walls, two flanking pieces, two jambs, the far face's back) rather
    ///   than losing a side, proving the carve does not reopen room `b`;
    /// - every sidedef naming a real room (not the door) has that room's
    ///   interior genuinely on the declared side, via the independent
    ///   `interior_is_on_the_right` probe.
    ///
    /// Counts below are taken over sidedefs actually referenced by a
    /// surviving linedef's `front`/`back`, not raw `data.sidedefs` array
    /// membership: `drop_wall_segment` (pre-existing, from `cut_portals`)
    /// removes the dropped linedef but leaves its now-unreferenced sidedef
    /// entries sitting in the array — harmless dead records (never read by
    /// anything downstream), but not part of any sector's actual boundary,
    /// so a raw per-sector array count over-counts by one dead entry for
    /// every room a portal touches.
    fn assert_door_construction(ir_json: &str, corners: [Pt; 4]) {
        let (ir, data, door) = compiled(ir_json);

        let referenced = |sector: usize| -> usize {
            data.linedefs
                .iter()
                .filter(|l| {
                    data.sidedefs[l.front].sector == sector
                        || l.back.is_some_and(|b| data.sidedefs[b].sector == sector)
                })
                .count()
        };

        assert_eq!(
            referenced(door),
            4,
            "the door sector is a closed quadrilateral, not a dangling sidedef"
        );

        for corner in corners {
            assert!(
                data.vertices.contains(&corner),
                "expected door corner {corner:?} is missing from the emitted vertices"
            );
        }

        assert_eq!(
            referenced(0),
            6,
            "room a keeps its plain-portal boundary shape (unaffected by the carve)"
        );
        assert_eq!(
            referenced(1),
            8,
            "room b's boundary grows by the recess notch, closed rather than reopened"
        );

        for l in &data.linedefs {
            let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
            let front_sector = data.sidedefs[l.front].sector;
            if front_sector < ir.rooms.len() {
                assert!(
                    interior_is_on_the_right(&ir, front_sector, p, q),
                    "front sidedef of line {p:?} -> {q:?} names room {front_sector}, \
                     but that room's interior is not on the right of travel"
                );
            }
            if let Some(back) = l.back {
                let back_sector = data.sidedefs[back].sector;
                if back_sector < ir.rooms.len() {
                    assert!(
                        interior_is_on_the_right(&ir, back_sector, q, p),
                        "back sidedef of line {p:?} -> {q:?} names room {back_sector}, \
                         but that room's interior is not on the left of travel"
                    );
                }
            }
        }
    }

    #[test]
    fn door_carves_room_b_when_room_a_is_west_of_a_vertical_wall() {
        // Worked example: room a = [0,0]-[256,256], room b = [256,0]-[512,256],
        // shared wall at x=256, portal width 128 at (256,128) -> open span
        // y in [64,192]. Room a keeps its face at x=256; the recess eats
        // DOOR_DEPTH=16 units into room b, so the far face lands at x=272.
        assert_door_construction(
            DOOR_IR,
            [
                Pt { x: 256, y: 64 },
                Pt { x: 256, y: 192 },
                Pt { x: 272, y: 64 },
                Pt { x: 272, y: 192 },
            ],
        );
    }

    #[test]
    fn door_carves_room_b_when_room_a_is_east_of_a_vertical_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128] }] }"#;
        // Room a (east, x in [256,512]) keeps its face at x=256; the recess
        // eats into room b (west, x in [0,256]), so the far face lands at
        // x=240.
        assert_door_construction(
            ir_json,
            [
                Pt { x: 256, y: 64 },
                Pt { x: 256, y: 192 },
                Pt { x: 240, y: 64 },
                Pt { x: 240, y: 192 },
            ],
        );
    }

    #[test]
    fn door_carves_room_b_when_room_a_is_south_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[128,256] }] }"#;
        // Room a (south, y in [0,256]) keeps its face at y=256; the recess
        // eats into room b (north, y in [256,512]), so the far face lands
        // at y=272.
        assert_door_construction(
            ir_json,
            [
                Pt { x: 64, y: 256 },
                Pt { x: 192, y: 256 },
                Pt { x: 64, y: 272 },
                Pt { x: 192, y: 272 },
            ],
        );
    }

    #[test]
    fn door_carves_room_b_when_room_a_is_north_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[128,256] }] }"#;
        // Room a (north, y in [256,512]) keeps its face at y=256; the recess
        // eats into room b (south, y in [0,256]), so the far face lands at
        // y=240.
        assert_door_construction(
            ir_json,
            [
                Pt { x: 64, y: 256 },
                Pt { x: 192, y: 256 },
                Pt { x: 64, y: 240 },
                Pt { x: 192, y: 240 },
            ],
        );
    }
}
