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

use crate::compile::portals::{Axis, Cut, emit_opening, resolve_portal};
use crate::compile::sectors::vertex_index;
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};
use crate::geom::{Pt, edges};
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

/// Precomputed geometry for one door portal — shared-wall span, recess
/// direction, and the resulting rectangle — gathered up front so degenerate
/// carves can be rejected before anything is emitted, and so the emission
/// pass and the validation pass read from the same numbers instead of each
/// recomputing them.
struct DoorPlan {
    /// The near room's id, for error messages.
    a: String,
    /// The recessed room's id, for error messages.
    b: String,
    /// The key that opens it, for a [`PortalKind::Locked`] portal.
    lock: Option<String>,
    ia: usize,
    ib: usize,
    axis: Axis,
    fixed: i32,
    open_lo: i32,
    open_hi: i32,
    a_forward: bool,
    far: i32,
}

impl DoorPlan {
    /// The door's carved rectangle in absolute `(x_lo, x_hi, y_lo, y_hi)`.
    fn rect(&self) -> (i32, i32, i32, i32) {
        let (near, far) = (self.fixed.min(self.far), self.fixed.max(self.far));
        match self.axis {
            Axis::Vertical => (near, far, self.open_lo, self.open_hi),
            Axis::Horizontal => (self.open_lo, self.open_hi, near, far),
        }
    }
}

/// Whether two axis-aligned rectangles, each `(x_lo, x_hi, y_lo, y_hi)`,
/// share interior area. Rectangles that merely touch at an edge (as two
/// unrelated doors' recesses legitimately might, laid end to end) do not
/// count — mirroring `sectors::overlaps`'s "a shared wall is fine, shared
/// interior is not" philosophy.
///
/// Indexes rather than names its tuple fields on purpose: naming them (e.g.
/// `ax_lo`/`ay_lo`) trips `clippy::similar_names`, and a short pure function
/// over two opaque 4-tuples reads fine without them.
fn rects_overlap(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    a.0 < b.1 && b.0 < a.1 && a.2 < b.3 && b.2 < a.3
}

/// How much room `b` has behind the shared wall, measured *across the
/// opening* rather than over its whole bounding box.
///
/// Only footprint edges parallel to the wall bound a recess's depth, and only
/// those whose along-axis extent overlaps the opening: a room can be deep
/// everywhere except right where the door goes. The minimum over those edges
/// is the depth the recess actually has to work with. Measuring the bounding
/// box instead reported a non-rectangular room's *widest* depth no matter
/// where the door sat, so an L-shaped room `b` with a shallow arm passed the
/// `DoorTooDeep` guard and the recess punched straight through its far wall.
///
/// Diagonal edges are not considered, matching `portals::wall_edges`: a 45
/// degree wall cannot host a portal in v1, so a recess never runs up against
/// one. A room with no parallel edge beyond the wall measures zero and is
/// rejected.
fn depth_behind_wall(poly: &[Pt], axis: Axis, fixed: i32, sign: i32, lo: i32, hi: i32) -> i32 {
    let mut depth: Option<i32> = None;
    for (p, q) in edges(poly) {
        let (along_p, across_p) = match axis {
            Axis::Vertical => (p.y, p.x),
            Axis::Horizontal => (p.x, p.y),
        };
        let (along_q, across_q) = match axis {
            Axis::Vertical => (q.y, q.x),
            Axis::Horizontal => (q.x, q.y),
        };
        if across_p != across_q {
            continue;
        }
        // Positive only for edges strictly beyond the wall on room `b`'s
        // side — which also excludes the shared wall itself.
        let candidate = (across_p - fixed) * sign;
        if candidate <= 0 {
            continue;
        }
        let (edge_lo, edge_hi) = (along_p.min(along_q), along_p.max(along_q));
        if edge_lo < hi && lo < edge_hi {
            depth = Some(depth.map_or(candidate, |best: i32| best.min(candidate)));
        }
    }
    depth.unwrap_or(0)
}

/// Gathers and validates every door portal's geometry before anything is
/// emitted, so a degenerate carve is rejected cleanly instead of leaving
/// partially-emitted geometry behind.
///
/// # Errors
/// Returns [`CompileError::NotAdjacent`] if a door portal's rooms are not
/// adjacent, which indicates `cut_portals` did not validate it first;
/// [`CompileError::DoorTooDeep`] if room `b` is not at least `DOOR_DEPTH`
/// deep behind the opening, which would let the recess punch through (or
/// invert past) room `b`'s far wall; and
/// [`CompileError::OverlappingDoorRecesses`] if two door portals recess into
/// the same room and their carved rectangles overlap.
fn plan_doors(ir: &Ir) -> Result<Vec<DoorPlan>, CompileError> {
    let mut plans = Vec::new();
    for portal in &ir.portals {
        if !matches!(portal.kind, PortalKind::Door | PortalKind::Locked) {
            continue;
        }

        // The same resolution `cut_portals` performed, so the recess lands
        // at exactly the span the flanking walls were split for.
        let geometry = resolve_portal(ir, portal)?;
        let span = geometry.span;
        let (open_lo, open_hi) = (geometry.open_lo, geometry.open_hi);

        // Recess into room `b`: `far` moves away from room `a`'s side by
        // `DOOR_DEPTH`.
        let sign = span.across_sign_toward_b();
        let far = span.fixed + sign * DOOR_DEPTH;

        // Guard: room `b` must have strictly more than `DOOR_DEPTH` of depth
        // behind the opening, or the recess reaches (`==`) or overshoots
        // (`>`) room `b`'s own far wall — either punching through it or
        // leaving room `b` with zero real depth beyond the door.
        let available = depth_behind_wall(
            &ir.rooms[geometry.ib].footprint,
            span.axis,
            span.fixed,
            sign,
            open_lo,
            open_hi,
        );
        if DOOR_DEPTH >= available {
            return Err(CompileError::DoorTooDeep {
                a: portal.a.clone(),
                b: portal.b.clone(),
                needed: DOOR_DEPTH,
                available,
            });
        }

        plans.push(DoorPlan {
            a: portal.a.clone(),
            b: portal.b.clone(),
            lock: portal.lock.clone(),
            ia: geometry.ia,
            ib: geometry.ib,
            axis: span.axis,
            fixed: span.fixed,
            open_lo,
            open_hi,
            a_forward: span.a_forward,
            far,
        });
    }

    // Guard: two door portals recessing into the same room must not carve
    // overlapping rectangles out of it.
    for i in 0..plans.len() {
        for j in (i + 1)..plans.len() {
            if plans[i].ib == plans[j].ib && rects_overlap(plans[i].rect(), plans[j].rect()) {
                return Err(CompileError::OverlappingDoorRecesses {
                    room: plans[i].b.clone(),
                    first_a: plans[i].a.clone(),
                    second_a: plans[j].a.clone(),
                });
            }
        }
    }

    Ok(plans)
}

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
fn door_special(tables: &Tables, plan: &DoorPlan) -> Result<u16, CompileError> {
    match &plan.lock {
        // `Ir::from_json` guarantees a locked portal names a key, so a
        // `None` here is a plain door.
        None => Ok(tables.door_special()),
        Some(lock) => tables
            .locked_door_special(lock)
            .ok_or_else(|| CompileError::UnknownLock {
                a: plan.a.clone(),
                b: plan.b.clone(),
                lock: lock.clone(),
            }),
    }
}

/// Emits a dedicated, initially closed sector for every door portal.
///
/// See the module documentation for the construction. Both face lines carry
/// the door special (so the door can actually be opened, from either room)
/// and the door sector's tag; every line touching the new sector is
/// lower-unpegged so its texture does not slide as the sector's ceiling
/// animates open (P11).
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` resolves to no
/// texture set, [`CompileError::UnknownLock`] when a locked portal names a
/// key the vocabulary has no special for, and whatever `plan_doors` raises —
/// which runs before anything is emitted.
///
/// # Panics
/// Panics if `emit_opening` ever returns a one-sided line — unreachable, as
/// it always emits both sidedefs of the line it pushes.
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

    let plans = plan_doors(ir)?;

    for plan in &plans {
        let special = door_special(tables, plan)?;
        let floor = ir.rooms[plan.ia].floor.min(ir.rooms[plan.ib].floor);
        let sector = data.sectors.len();
        let tag = tags.allocate(sector, &format!("door {} <-> {}", plan.a, plan.b));
        data.sectors.push(SectorOut {
            floor,
            // A closed door: ceiling snapped to the floor.
            ceiling: floor,
            light: ir.rooms[plan.ia].light,
            floor_tex: ir.rooms[plan.ia].floor_tex.clone(),
            ceil_tex: ir.rooms[plan.ia].ceil_tex.clone(),
            special: 0,
            tag,
        });

        let near_cut = Cut {
            axis: plan.axis,
            fixed: plan.fixed,
            open_lo: plan.open_lo,
            open_hi: plan.open_hi,
        };
        let far_cut = Cut {
            axis: plan.axis,
            fixed: plan.far,
            open_lo: plan.open_lo,
            open_hi: plan.open_hi,
        };

        // The two face lines, perpendicular to the direction of travel
        // through the doorway: room `a` <-> door, then room `b` <-> door.
        // Both are oriented with the *room* on the front and the door sector
        // on the back, which is how a door is built: `EV_VerticalDoor` acts
        // on the sector across the line from the player, and vanilla's
        // `P_UseLines` only triggers a special from a line's front side, so
        // a face whose front named the door sector would be usable only from
        // inside the door itself. The far face therefore takes the reversed
        // direction (`!a_forward`), which puts room `b` on the geometric
        // right exactly as `a_forward` puts room `a` there on the near face.
        let near_line = emit_opening(data, &near_cut, plan.ia, sector, plan.a_forward);
        let far_line = emit_opening(data, &far_cut, plan.ib, sector, !plan.a_forward);
        for line in [near_line, far_line] {
            data.linedefs[line].lower_unpegged = true;
            data.linedefs[line].special = special;
            data.linedefs[line].tag = tag;
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
            data,
            &near_cut,
            &far_cut,
            plan.open_lo,
            !plan.a_forward,
            plan.ib,
            sector,
            &track_tex,
        );
        emit_jamb(
            data,
            &near_cut,
            &far_cut,
            plan.open_hi,
            plan.a_forward,
            plan.ib,
            sector,
            &track_tex,
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
/// regardless of which shared-wall orientation applies. Front is always room
/// `b` at both jambs; only this direction varies. See the task-8 report for
/// the derivation.
///
/// A jamb is the door's *track*: it blocks movement, as the equivalent
/// two-sided track lines do in hand-built maps. Leaving it passable let the
/// player step sideways into the recess and stand inside the door once it
/// opened. It stays two-sided so the track texture renders from both sides
/// and the door sector keeps a closed boundary.
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
        blocking: true,
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
    /// This alone is *not* sufficient to prove a room-facing sidedef near a
    /// door is correct: the original footprint still contains the recessed
    /// sliver (the IR knows nothing about the carve), so a sidedef wrongly
    /// attributed to room `b` from *inside* the recess would still pass this
    /// check. `assert_door_construction` below pairs it with an explicit
    /// check that the same probe point falls outside the door's carved
    /// rectangle.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        contains(&ir.rooms[room_idx].footprint, probe(p, q))
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
    /// - every sidedef naming a real room has that room's interior genuinely
    ///   on the declared side (`interior_is_on_the_right`) *and* its probe
    ///   point falls outside the door's carved rectangle;
    /// - every sidedef naming the door sector has its probe point fall
    ///   *inside* the door's carved rectangle.
    ///
    /// The last two points are what actually pin down the far face and the
    /// jambs, not just the near face: `interior_is_on_the_right` alone
    /// cannot, because room `b`'s original footprint still contains the
    /// recessed sliver, so a sidedef misattributed to room `b` from inside
    /// the recess passes it anyway. The door-rectangle check closes that
    /// gap — see the fix-round-1 section of the task-8 report for the
    /// mutation that exposed it and confirmation this catches it.
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
                assert!(
                    !in_door_rect(p),
                    "sidedef names room {sector} for line {from:?} -> {to:?}, but its probe \
                     {p:?} falls inside the door's carved rectangle — a sidedef pointed into \
                     the recess instead of at room {sector}'s real territory would land here"
                );
            } else {
                assert_eq!(
                    sector, door,
                    "sidedef names neither a real room nor the door"
                );
                assert!(
                    in_door_rect(p),
                    "sidedef names the door sector for line {from:?} -> {to:?}, but its probe \
                     {p:?} falls outside the door's carved rectangle"
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

    #[test]
    fn a_recess_deeper_than_room_b_is_rejected() {
        // Room b is only 8 units deep (x in [100,108]) — shallower than
        // DOOR_DEPTH (16) — so the recess would punch through its far wall.
        // grid is 4 here (not the usual 64): only room *footprint* vertices
        // are grid-validated (portal `at`/`width` are not), and a 64-unit
        // grid cannot express an 8-unit room at all.
        let ir_json = r#"{ "seed":1, "grid":4, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,100],[100,100],[100,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[100,0],[100,100],[108,100],[108,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":64, "at":[100,50] }] }"#;
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(
            matches!(
                emit_doors(&ir, &tables, &mut data, &mut tags),
                Err(crate::compile::CompileError::DoorTooDeep { .. })
            ),
            "a room shallower than DOOR_DEPTH must be rejected, not silently punched through"
        );
    }

    /// Room `b` is L-shaped: 136 units deep over y 0..32, but only 8 units
    /// deep over y 32..64, where the second door below is placed. Its
    /// bounding box is 136 wide either way, so a bounding-box depth
    /// measurement reports the deep arm no matter where the door goes.
    fn l_shaped_b_ir(door_at: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":8, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,64],[64,64],[64,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                {{ "id":"b",
                   "footprint":[[64,0],[64,64],[72,64],[72,32],[200,32],[200,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"door", "width":16, "at":[64,{door_at}] }}] }}"#
        )
    }

    #[test]
    fn a_door_into_a_shallow_arm_of_an_l_shaped_room_is_rejected() {
        // The door sits at y = 48, in the 8-unit-deep arm — shallower than
        // DOOR_DEPTH (16), so the recess would punch through room b's far
        // wall at x = 72. The bounding box says 136 units are available,
        // which is why measuring it passed this straight through.
        let ir = Ir::from_json(&l_shaped_b_ir(48)).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(
            matches!(
                emit_doors(&ir, &tables, &mut data, &mut tags),
                Err(crate::compile::CompileError::DoorTooDeep { available: 8, .. })
            ),
            "the depth behind the opening (8) must be measured, not the bounding box (136)"
        );
    }

    #[test]
    fn a_door_into_the_deep_arm_of_the_same_l_shaped_room_is_accepted() {
        // Same room, door moved to y = 16 where room b really is 136 deep.
        // Pairing this with the test above is what pins the measurement to
        // the opening's position rather than to the room as a whole.
        let (_, data, door) = compiled(&l_shaped_b_ir(16));
        assert_eq!(data.sectors.len(), 3, "the door sector was carved");
        assert!(
            data.vertices.contains(&Pt { x: 80, y: 8 }),
            "the far face lands 16 units into the deep arm"
        );
        assert_ne!(data.sectors[door].tag, 0, "the door sector is tagged");
    }

    #[test]
    fn a_door_between_offset_rooms_carves_a_closed_sector() {
        // Rooms sharing only part of their wall (y 128..256 of x = 256).
        // Every previous door fixture was two flush equal squares.
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[256,128],[256,384],[512,384],[512,128]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":64, "at":[256,192] }] }"#;
        assert_door_construction(
            ir_json,
            [
                Pt { x: 256, y: 160 },
                Pt { x: 256, y: 224 },
                Pt { x: 272, y: 160 },
                Pt { x: 272, y: 224 },
            ],
        );
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
        // The jambs are the door sector's two-sided lines that carry no
        // special — the faces carry it.
        let jambs: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.special == 0 && l.back.is_some_and(|b| data.sidedefs[b].sector == door))
            .collect();
        assert_eq!(jambs.len(), 2, "a door recess has two jambs");
        assert!(
            jambs.iter().all(|l| l.blocking),
            "a door track blocks movement, like the equivalent two-sided \
             track lines in hand-built maps"
        );
    }

    #[test]
    fn two_doors_recessing_into_the_same_narrow_room_from_opposite_walls_are_rejected() {
        // middle is 24 units wide (x in [100,124]) — wider than one recess
        // (16) but narrower than two (32) — with a door on each of its
        // vertical walls, so the two 16-unit recesses (x in [100,116] and
        // x in [108,124]) overlap in x in [108,116]. grid is 4 here for the
        // same reason as the test above.
        let ir_json = r#"{ "seed":1, "grid":4, "theme":"tech_base",
          "rooms":[
            { "id":"left", "footprint":[[0,0],[0,100],[100,100],[100,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"middle", "footprint":[[100,0],[100,100],[124,100],[124,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"right", "footprint":[[124,0],[124,100],[224,100],[224,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[
            { "a":"left", "b":"middle", "kind":"door", "width":60, "at":[100,50] },
            { "a":"right", "b":"middle", "kind":"door", "width":60, "at":[124,50] }
          ] }"#;
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(
            matches!(
                emit_doors(&ir, &tables, &mut data, &mut tags),
                Err(crate::compile::CompileError::OverlappingDoorRecesses { .. })
            ),
            "two overlapping recesses into the same room must be rejected, not silently emitted"
        );
    }
}
