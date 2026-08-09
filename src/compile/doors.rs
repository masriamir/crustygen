//! Emits a dedicated, closed sector for each door portal.
//!
//! Rooms are authored apart (see [`crate::ir::Portal`]'s doc comment), so a
//! door portal's wall gap already exists before this pass ever runs — there
//! is nowhere left to carve. This pass simply fills that gap with a closed
//! sector, the exact shape `crate::compile::portals::emit_gap_sector`
//! builds for a plain portal's open passage: two "face" lines (perpendicular
//! to the direction of travel through the doorway, one bordering room `a`
//! and one bordering room `b`) and two one-sided "jamb" lines (parallel to
//! it, closing the gap's long sides, front bound to the door sector with
//! solid rock behind) give the door sector real, closed boundaries. Neither
//! room's own declared footprint is touched.
//!
//! This runs after [`crate::compile::portals::cut_portals`], which leaves a
//! door portal's flanking walls in place but the gap itself still empty —
//! the sector this pass builds is exactly what fills it.

use crate::compile::portals::{emit_gap_sector, resolve_portal};
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, MapData, SectorOut};
use crate::ir::{Ir, Portal, PortalKind};
use crate::tables::Tables;

/// Resolves the linedef special that opens one door.
///
/// A [`PortalKind::Door`] gets the manual door special; a
/// [`PortalKind::Locked`] gets the keyed one for the key it names. Both come
/// from `vocabulary.toml`, never from a literal here — a wrong special
/// produces a map that loads and does the wrong thing, which no test of ours
/// can catch.
///
/// # Errors
/// Returns [`CompileError::UnknownLock`] when the named key has no keyed door
/// special in the vocabulary table.
fn door_special(tables: &Tables, portal: &Portal) -> Result<u16, CompileError> {
    match &portal.lock {
        // `Ir::from_json` guarantees a locked portal names a key, so a
        // `None` here is a plain door.
        None => Ok(tables.door_special()),
        Some(lock) => tables
            .locked_door_special(lock)
            .ok_or_else(|| CompileError::UnknownLock {
                a: portal.a.clone(),
                b: portal.b.clone(),
                lock: lock.clone(),
            }),
    }
}

/// Emits a dedicated, initially closed sector for every door portal, filling
/// the wall gap [`crate::compile::portals::cut_portals`] already cut into
/// both rooms' own walls.
///
/// See the module documentation for the construction. Both face lines carry
/// the door special (so the door can actually be opened, from either room)
/// and the door sector's tag; every line touching the new sector is
/// lower-unpegged so its texture does not slide as the sector's ceiling
/// animates open (P11).
///
/// Resolves each door portal's geometry independently via
/// `crate::compile::portals::resolve_portal` rather than trusting
/// `cut_portals` already ran — the same defense-in-depth
/// [`crate::compile::exits::emit_exits`] follows for its own resolution.
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` resolves to no
/// texture set, [`CompileError::UnknownLock`] when a locked portal names a
/// key the vocabulary has no special for, and whatever `resolve_portal`
/// raises (`NotAdjacent`, `PortalOffWall`, `PortalOnDiagonalWall`,
/// `PortalTooWide`) if a door portal's rooms are not adjacent on a wall v1
/// can cut.
///
/// # Panics
/// Panics if `emit_gap_sector` ever returns a one-sided threshold line —
/// unreachable, as it always builds two-sided near/far thresholds.
pub fn emit_doors(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
) -> Result<(), CompileError> {
    // An unresolvable theme is an authoring error, not something to paper
    // over with a hardcoded texture: silently substituting one meant a
    // misspelled theme produced a map that looked deliberate. An unknown
    // *thing* has always been a hard error; this makes the two consistent.
    let unknown_theme = || CompileError::UnknownTheme {
        theme: ir.theme.clone(),
    };
    let door_tex = tables
        .texture("door", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();
    let track_tex = tables
        .texture("door_track", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();

    for portal in &ir.portals {
        if !matches!(portal.kind, PortalKind::Door | PortalKind::Locked) {
            continue;
        }

        let geometry = resolve_portal(ir, portal)?;
        let special = door_special(tables, portal)?;

        let floor = ir.rooms[geometry.ia].floor.min(ir.rooms[geometry.ib].floor);
        let sector_index = data.sectors.len();
        let tag = tags.allocate(sector_index, &format!("door {} <-> {}", portal.a, portal.b));
        let sector_out = SectorOut {
            floor,
            // A closed door: ceiling snapped to the floor.
            ceiling: floor,
            light: ir.rooms[geometry.ia].light,
            floor_tex: ir.rooms[geometry.ia].floor_tex.clone(),
            ceil_tex: ir.rooms[geometry.ia].ceil_tex.clone(),
            special: 0,
            tag,
        };

        let gap = emit_gap_sector(
            data,
            &geometry.span,
            geometry.open_lo,
            geometry.open_hi,
            geometry.ia,
            geometry.ib,
            sector_out,
            &track_tex,
        );
        debug_assert_eq!(
            gap.sector, sector_index,
            "emit_gap_sector pushed at the predicted index"
        );

        for line in [gap.near_line, gap.far_line] {
            data.linedefs[line].lower_unpegged = true;
            data.linedefs[line].special = special;
            data.linedefs[line].tag = tag;
            let front = data.linedefs[line].front;
            let back = data.linedefs[line]
                .back
                .expect("emit_gap_sector's thresholds are always two-sided");
            data.sidedefs[front].upper.clone_from(&door_tex);
            data.sidedefs[back].upper.clone_from(&door_tex);
        }
        // The jambs (the door's track) are lower-unpegged too, for the same
        // reason as the faces (P11) — their middle texture must not slide
        // as the door sector's ceiling animates open.
        for line in gap.jamb_lines {
            data.linedefs[line].lower_unpegged = true;
        }
    }
    Ok(())
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

    /// Room `a` and room `b` face each other across a legal 64-unit gap
    /// (room `a`'s east wall at x = 256, room `b`'s west wall at x = 320).
    const DOOR_IR: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
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
    fn a_plain_portal_adds_no_sector_via_emit_doors() {
        // A plain portal's own passage sector is added by `cut_portals`
        // itself, not `emit_doors` — this pins that `emit_doors` skips
        // `PortalKind::Plain` entirely, adding zero *further* sectors.
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

    /// The point used to decide which side of `p -> q` is "the right":
    /// offset one unit from the segment's midpoint, in the direction
    /// obtained by rotating `p -> q` -90 degrees (reduced to a one-unit step
    /// via `.signum()`, since every segment under test is axis-aligned).
    /// Extracted so both `interior_is_on_the_right` and the door-rectangle
    /// checks below test the *same* point, rather than each computing their
    /// own and silently drifting apart.
    fn probe(p: Pt, q: Pt) -> Pt {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        Pt {
            x: i32::midpoint(p.x, q.x) + dy.signum(),
            y: i32::midpoint(p.y, q.y) - dx.signum(),
        }
    }

    /// Whether room `room_idx`'s *original* IR footprint has its interior on
    /// the right of travel from `p` to `q`. Deliberately independent of
    /// `portals`/`doors` — see `portals::tests::interior_is_on_the_right`,
    /// which this mirrors: duplicated rather than shared, because sharing it
    /// with production code would let a shared bug hide from both call
    /// sites.
    ///
    /// This is now sufficient on its own to prove a room-facing sidedef near
    /// a door is correct: rooms are authored apart, so neither footprint is
    /// ever modified by door construction, unlike the old carve-into-`b`
    /// design where the declared footprint still contained the recessed
    /// sliver.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        contains(&ir.rooms[room_idx].footprint, probe(p, q))
    }

    /// Asserts every sector's boundary is a closed loop — mirrors
    /// `portals::tests::assert_sector_boundaries_are_closed` (not reused
    /// directly: it is private to that module, and per this project's
    /// convention duplicating a small independent check is preferable to
    /// sharing it with the code under test).
    fn assert_sector_boundaries_are_closed(data: &MapData) {
        let mut balance: std::collections::HashMap<(usize, Pt), i32> =
            std::collections::HashMap::new();
        for line in &data.linedefs {
            let (p, q) = (data.vertices[line.v1], data.vertices[line.v2]);
            *balance
                .entry((data.sidedefs[line.front].sector, p))
                .or_default() += 1;
            *balance
                .entry((data.sidedefs[line.front].sector, q))
                .or_default() -= 1;
            if let Some(back) = line.back {
                let sector = data.sidedefs[back].sector;
                *balance.entry((sector, q)).or_default() += 1;
                *balance.entry((sector, p)).or_default() -= 1;
            }
        }
        for ((sector, point), net) in balance {
            assert_eq!(
                net, 0,
                "sector {sector}'s boundary is not a closed loop: vertex {point:?} is left \
                 {net} more times than it is entered"
            );
        }
    }

    /// Asserts one door construction against coordinates hand-derived from
    /// the fixture's own footprints — not by calling
    /// `portals::resolve_portal`/`emit_gap_sector` internals — and checks:
    ///
    /// - the door sector is bounded by exactly four sidedefs (a closed
    ///   quadrilateral, not a dangling single sidedef) — true regardless of
    ///   room `a`/`b`'s shape, since a door is always built from exactly two
    ///   faces and two jambs;
    /// - its four corners are exactly the independently expected ones;
    /// - both room `a` and room `b` keep their own plain-portal boundary
    ///   shape: each room's own edge count plus two (its own wall splits
    ///   into two flanking pieces, and its own threshold face adds one
    ///   more) — symmetric now that neither room is carved, unlike the old
    ///   `edges_a + 2` / `edges_b + 4` asymmetry;
    /// - every sector's boundary is a closed loop
    ///   (`assert_sector_boundaries_are_closed`);
    /// - every sidedef naming a real room has that room's interior genuinely
    ///   on the declared side (`interior_is_on_the_right`), and every
    ///   sidedef naming the door sector has its probe point fall inside the
    ///   door's own rectangle (derived from `corners`) — together these
    ///   cover every sidedef in the fixture, not just the ones bordering a
    ///   real room.
    ///
    /// Counts below are taken over sidedefs actually referenced by a
    /// surviving linedef's `front`/`back`, not raw `data.sidedefs` array
    /// membership: `split_wall_for_opening` (pre-existing, from
    /// `cut_portals`) removes the dropped linedef but leaves its
    /// now-unreferenced sidedef entry sitting in the array — harmless dead
    /// records (never read by anything downstream), but not part of any
    /// sector's actual boundary, so a raw per-sector array count over-counts
    /// by one dead entry for every room a portal touches.
    ///
    /// Assumes rooms `a` and `b` are `ir.rooms[0]` and `ir.rooms[1]`
    /// respectively (true for every fixture below, and for every door
    /// portal this project builds, since a door portal always names exactly
    /// two rooms).
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

        let edges_a = ir.rooms[0].footprint.len();
        let edges_b = ir.rooms[1].footprint.len();
        assert_eq!(
            referenced(0),
            edges_a + 2,
            "room a keeps its plain-portal boundary shape (unaffected by the door)"
        );
        assert_eq!(
            referenced(1),
            edges_b + 2,
            "room b also keeps its plain-portal boundary shape (unaffected, unlike the old \
             carve-into-b design)"
        );

        assert_sector_boundaries_are_closed(&data);

        let dx0 = corners.iter().map(|c| c.x).min().expect("four corners");
        let dx1 = corners.iter().map(|c| c.x).max().expect("four corners");
        let dy0 = corners.iter().map(|c| c.y).min().expect("four corners");
        let dy1 = corners.iter().map(|c| c.y).max().expect("four corners");
        let in_door_rect = |pt: Pt| pt.x >= dx0 && pt.x <= dx1 && pt.y >= dy0 && pt.y <= dy1;

        let check_side = |sector: usize, from: Pt, to: Pt| {
            let p = probe(from, to);
            if sector < ir.rooms.len() {
                assert!(
                    interior_is_on_the_right(&ir, sector, from, to),
                    "sidedef names room {sector} for line {from:?} -> {to:?}, but that \
                     room's interior is not on the right of travel"
                );
            } else {
                assert_eq!(
                    sector, door,
                    "sidedef names neither a real room nor the door"
                );
                assert!(
                    in_door_rect(p),
                    "sidedef names the door sector for line {from:?} -> {to:?}, but its probe \
                     {p:?} falls outside the door's rectangle"
                );
            }
        };

        for l in &data.linedefs {
            let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
            check_side(data.sidedefs[l.front].sector, p, q);
            if let Some(back) = l.back {
                check_side(data.sidedefs[back].sector, q, p);
            }
        }
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_west_of_a_vertical_wall() {
        // Worked example: room a = [0,0]-[256,256], room b = [320,0]-[576,256],
        // shared wall pair at x=256 (room a) / x=320 (room b), portal width
        // 128 at (256,128) -> open span y in [64,192].
        assert_door_construction(
            DOOR_IR,
            [
                Pt { x: 256, y: 64 },
                Pt { x: 256, y: 192 },
                Pt { x: 320, y: 64 },
                Pt { x: 320, y: 192 },
            ],
        );
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_east_of_a_vertical_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[-64,0],[-64,256],[192,256],[192,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128] }] }"#;
        // Room a (east, x in [256,512]) keeps its face at x=256; room b
        // (west, x in [-64,192]) keeps its face at x=192, a legal 64-unit
        // gap away.
        assert_door_construction(
            ir_json,
            [
                Pt { x: 256, y: 64 },
                Pt { x: 256, y: 192 },
                Pt { x: 192, y: 64 },
                Pt { x: 192, y: 192 },
            ],
        );
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_south_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,320],[0,576],[256,576],[256,320]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[128,256] }] }"#;
        // Room a (south, y in [0,256]) keeps its face at y=256; room b
        // (north, y in [320,576]) keeps its face at y=320.
        assert_door_construction(
            ir_json,
            [
                Pt { x: 64, y: 256 },
                Pt { x: 192, y: 256 },
                Pt { x: 64, y: 320 },
                Pt { x: 192, y: 320 },
            ],
        );
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_north_of_a_horizontal_wall() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,-64],[0,192],[256,192],[256,-64]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[128,256] }] }"#;
        // Room a (north, y in [256,512]) keeps its face at y=256; room b
        // (south, y in [-64,192]) keeps its face at y=192.
        assert_door_construction(
            ir_json,
            [
                Pt { x: 64, y: 256 },
                Pt { x: 192, y: 256 },
                Pt { x: 64, y: 192 },
                Pt { x: 192, y: 192 },
            ],
        );
    }

    #[test]
    fn a_door_works_at_the_minimum_legal_gap() {
        // The wall gap is exactly `Ir::MIN_PORTAL_GAP` (8) — the tightest
        // legal thickness a door can sit in. Fine grid (4) since a 64-unit
        // grid cannot express an 8-unit gap at all.
        let ir_json = r#"{ "seed":1, "grid":4, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,64],[64,64],[64,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[72,0],[72,64],[136,64],[136,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":32, "at":[64,32] }] }"#;
        assert_door_construction(
            ir_json,
            [
                Pt { x: 64, y: 16 },
                Pt { x: 64, y: 48 },
                Pt { x: 72, y: 16 },
                Pt { x: 72, y: 48 },
            ],
        );
    }

    #[test]
    fn two_doors_on_opposite_walls_of_a_narrow_middle_room_both_compile_now() {
        // The old carve-into-b design rejected this: two `DOOR_DEPTH`-deep
        // recesses eating into the same narrow middle room from opposite
        // walls used to overlap. Since a door's own sector now fills the
        // *existing* wall gap rather than carving into either room, the two
        // gap sectors sit entirely outside the middle room's own territory,
        // on opposite sides of it — structurally unable to collide, however
        // narrow the middle room is (down to grid-snapped minimums).
        let ir_json = r#"{ "seed":1, "grid":4, "theme":"tech_base",
          "rooms":[
            { "id":"left", "footprint":[[0,0],[0,64],[64,64],[64,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"middle", "footprint":[[72,0],[72,64],[96,64],[96,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"right", "footprint":[[104,0],[104,64],[168,64],[168,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[
            { "a":"left", "b":"middle", "kind":"door", "width":32, "at":[64,32] },
            { "a":"right", "b":"middle", "kind":"door", "width":32, "at":[104,32] }
          ] }"#;
        let (_, data, _) = compiled(ir_json);
        // left, middle, right, plus two door sectors.
        assert_eq!(data.sectors.len(), 5);
    }

    #[test]
    fn both_door_faces_carry_the_door_special_and_the_door_sector_tag() {
        // Without a special the door is a permanently sealed slab that
        // disconnects the two rooms in-engine, which compiled clean.
        let (_, data, door) = compiled(DOOR_IR);
        let tables = Tables::load().expect("tables");
        let expected = tables.door_special();
        assert_ne!(expected, 0, "the vocabulary lists a real door special");

        let faces: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.special != 0)
            .collect::<Vec<_>>();
        assert_eq!(faces.len(), 2, "both faces of the door carry the special");
        for face in faces {
            assert_eq!(face.special, expected);
            assert_eq!(
                face.tag, data.sectors[door].tag,
                "the action names the door sector's own tag, never tag 0"
            );
            assert!(
                data.sidedefs[face.front].sector != door,
                "a face's front names the room, not the door — vanilla only \
                 triggers a special from a line's front side"
            );
            assert_eq!(
                data.sidedefs[face.back.expect("two-sided")].sector,
                door,
                "the door sector is the back sector EV_VerticalDoor acts on"
            );
        }
    }

    #[test]
    fn a_locked_door_carries_the_special_for_the_key_that_opens_it() {
        // `Locked` compiled identically to `Door` before this: the key was
        // recorded in the IR and then dropped on the floor.
        let tables = Tables::load().expect("tables");
        for key in ["blue_card", "yellow_skull"] {
            let locked = DOOR_IR.replace(
                "\"kind\":\"door\"",
                &format!("\"kind\":\"locked\", \"lock\":\"{key}\""),
            );
            let (_, data, _) = compiled(&locked);
            let expected = tables
                .locked_door_special(key)
                .expect("the vocabulary lists this key");
            assert_ne!(
                expected,
                tables.door_special(),
                "a locked door is not the same special as a plain one"
            );
            let specials: Vec<u16> = data
                .linedefs
                .iter()
                .map(|l| l.special)
                .filter(|s| *s != 0)
                .collect();
            assert_eq!(
                specials,
                vec![expected; 2],
                "both faces carry `{key}`'s keyed door special"
            );
        }
    }

    #[test]
    fn a_lock_the_vocabulary_does_not_know_is_rejected() {
        let locked = DOOR_IR.replace(
            "\"kind\":\"door\"",
            "\"kind\":\"locked\", \"lock\":\"plaid_card\"",
        );
        let ir = Ir::from_json(&locked).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(matches!(
            emit_doors(&ir, &tables, &mut data, &mut tags),
            Err(crate::compile::CompileError::UnknownLock { .. })
        ));
    }

    #[test]
    fn an_unknown_theme_is_rejected_rather_than_silently_substituted() {
        // An unknown *thing* has always been a hard error; an unknown theme
        // quietly fell back to hardcoded textures, so a misspelled theme
        // produced a map that looked deliberate.
        let themed = DOOR_IR.replace("\"theme\":\"tech_base\"", "\"theme\":\"tech_bass\"");
        let ir = Ir::from_json(&themed).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(matches!(
            emit_doors(&ir, &tables, &mut data, &mut tags),
            Err(crate::compile::CompileError::UnknownTheme { .. })
        ));
    }

    #[test]
    fn door_jambs_block_movement_so_the_player_cannot_stand_in_the_track() {
        let (_, data, door) = compiled(DOOR_IR);
        // The jambs are the door sector's one-sided lines, front bound to
        // the door with solid rock behind — the faces are the two-sided
        // ones, which carry the special.
        let jambs: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == door)
            .collect();
        assert_eq!(jambs.len(), 2, "a door has two jambs");
        assert!(
            jambs.iter().all(|l| l.blocking),
            "a door track blocks movement, like the equivalent one-sided \
             track lines in hand-built maps"
        );
    }

    /// The same octagon fixture used elsewhere in this crate
    /// (`sectors::tests::OCTAGON`, `portals::tests::OCTAGON_ROOM`), shifted
    /// 64 units east so its west wall (x = 64) sits a legal gap beyond room
    /// `a`'s east wall (x = 0). Room `a` sits to the west of it.
    const OCTAGON_ROOM_B: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[-256,0],[-256,256],[0,256],[0,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b",
          "footprint":[[64,64],[64,192],[128,256],[256,256],[320,192],[320,64],[256,0],[128,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"door", "width":64, "at":[0,128] }] }"#;

    #[test]
    fn a_door_into_an_octagonal_room_on_its_axis_aligned_wall_works() {
        // The door sits on the octagon's west wall, x = 64, y in 64..192 —
        // away from every chamfer. Routed through `assert_door_construction`
        // so the full sidedef-facing invariant runs against a genuinely
        // diagonal-edged room `b`, not merely a vertex-membership check.
        assert_door_construction(
            OCTAGON_ROOM_B,
            [
                Pt { x: 0, y: 96 },
                Pt { x: 0, y: 160 },
                Pt { x: 64, y: 96 },
                Pt { x: 64, y: 160 },
            ],
        );
    }

    /// Two right triangles splitting a 64-unit square along its own
    /// diagonal, exactly like `portals::tests::DIAGONAL_TWIN_TRIANGLES`, but
    /// with `"kind":"door"` instead of `"plain"`.
    const DOOR_ON_DIAGONAL_WALL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,64],[64,64]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[0,0],[64,64],[64,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"door", "width":16, "at":[32,32] }] }"#;

    #[test]
    fn a_door_portal_requested_on_a_diagonal_wall_is_rejected_before_any_sector_is_built() {
        // In the real pipeline `cut_portals` resolves every portal — door or
        // plain — before anything is cut, so this is what an author
        // actually sees: the diagonal-wall check `portals::tests` pins for a
        // plain portal must reach a door portal too, not just fall through
        // to `NotAdjacent` because the portal happened to be a door.
        let ir = Ir::from_json(DOOR_ON_DIAGONAL_WALL).expect("ir");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(
            matches!(
                cut_portals(&ir, &mut data),
                Err(crate::compile::CompileError::PortalOnDiagonalWall { .. })
            ),
            "cut_portals must reject a door portal on a diagonal wall before doors ever run"
        );
    }

    #[test]
    fn emit_doors_independently_rejects_a_diagonal_wall_too() {
        // `emit_doors` re-derives its own geometry via
        // `portals::resolve_portal` rather than trusting `cut_portals`
        // already ran. This calls `emit_doors` directly without
        // `cut_portals` first (unlike every other fixture in this module,
        // and not how the real `compile_reporting` pipeline sequences
        // things) specifically to prove `emit_doors`'s own resolution
        // independently agrees, rather than merely relying on `cut_portals`
        // to have already caught it upstream.
        let ir = Ir::from_json(DOOR_ON_DIAGONAL_WALL).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        let mut tags = TagAllocator::new();
        assert!(
            matches!(
                emit_doors(&ir, &tables, &mut data, &mut tags),
                Err(crate::compile::CompileError::PortalOnDiagonalWall { .. })
            ),
            "emit_doors's own resolve_portal call must reject this independently of cut_portals"
        );
    }
}
