//! Carves the level exit into one wall of its host room.
//!
//! An exit is a linedef special on a segment of one room's own boundary
//! wall — not a connection to a second room, unlike [`crate::ir::Portal`].
//! The span-resolution and wall-splitting machinery is exactly
//! [`crate::compile::portals`]'s, reused directly: "the same machinery,
//! minus the second room."
//!
//! The two [`crate::ir::ExitTrigger`] kinds need different geometry, because
//! they trigger differently in the pinned engine (see the citations in
//! `specials-report.md` and the two "engine facts" already recorded in
//! `KNOWN-GAPS.md`):
//!
//! - **Switch** (`P_UseSpecialLine`, front-side only) needs nothing more than
//!   a normal solid wall. The player presses "use" against it, exactly like a
//!   door. The exit's span is carved out of the host wall and replaced with a
//!   single one-sided linedef: front bound to the host room, `blocking:
//!   true`, the resolved switch special, and the theme's switch texture on
//!   its middle. The room stays a fully closed one-sided sector on that wall
//!   — nothing is opened.
//! - **Walkover** (`P_CrossSpecialLine`, via `PIT_CheckLine`) only fires for a
//!   line the mover's movement actually crosses, and `PIT_CheckLine` returns
//!   `false` — rejecting the move before ever reaching the `spechit`
//!   bookkeeping — for both a one-sided line (`!ld->backsector`) and a
//!   two-sided line carrying `ML_BLOCKING`. A walkover exit line therefore
//!   *must* be two-sided and non-blocking, which rules out placing it flush
//!   on the room's true perimeter: a passable line straight on the boundary
//!   would open the room to the undefined void beyond it. So a walkover exit
//!   carves a small dead-end alcove out of the host room's own wall — the
//!   same recess construction [`crate::compile::doors`] uses to carve room
//!   `b`, but with no second room on the other side: the alcove's far and
//!   side walls are solid, closing it off, and only the near threshold
//!   (front bound to the host room, back to the alcove) carries the walkover
//!   special. The player steps into the alcove, crosses the threshold, and
//!   the level ends.
//!
//! Both kinds resolve their span with `resolve_exit` and carve the host wall
//! with `portals::split_wall_for_opening` before emitting anything — the
//! same "resolve and validate everything, then emit" discipline
//! `cut_portals` and `doors::plan_doors` already follow, so a rejected exit
//! leaves no partially-cut geometry behind.
//!
//! Every exit is tagged from the shared [`TagAllocator`], even though
//! `G_ExitLevel`/`G_SecretExitLevel` read no tag at all. This mirrors
//! `doors::emit_doors`'s own precedent — a manual door is tagged uniformly
//! too, though `EV_VerticalDoor` acts on the line's back sector rather than
//! consulting the tag — so [`crate::compile::tags::check_no_action_at_tag_zero`]
//! stays a single, exception-free invariant and the tag manifest records
//! every action a run took, not just the ones that mechanically need one.

use crate::compile::portals::{
    Axis, Cut, SharedSpan, emit_opening, split_wall_for_opening, wall_edges,
};
use crate::compile::sectors::vertex_index;
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};
use crate::geom::Pt;
use crate::ir::{Exit, ExitTrigger, Ir};
use crate::tables::Tables;

/// How far a walkover exit's alcove extends beyond the host room's wall, in
/// map units.
///
/// A compiler construction constant, like `doors::DOOR_DEPTH`, not an
/// engine-sourced one: nothing in the Doom engine constrains how deep an
/// exit alcove is, since it is not a door and consults no clearance formula.
/// 32 units is enough to read as a real threshold the player steps into
/// rather than a slit, while staying small relative to the rooms this
/// compiler targets.
const EXIT_ALCOVE_DEPTH: i32 = 32;

/// Everything resolved about one exit's placement on its host room's wall.
struct ExitPlan {
    /// Index of the host room in `ir.rooms`, which is also its sector index.
    room_idx: usize,
    /// The axis the host wall runs along.
    axis: Axis,
    /// The coordinate held constant along the host wall.
    fixed: i32,
    /// The low end of the exit's span.
    open_lo: i32,
    /// The high end of the exit's span.
    open_hi: i32,
    /// Whether the host wall's own edge direction runs in the increasing-
    /// along direction — [`SharedSpan::a_forward`]'s single-room analogue.
    forward: bool,
}

/// Resolves one exit against its host room's real wall.
///
/// # Errors
/// Returns [`CompileError::ExitOffWall`] when `exit.at` lies on none of the
/// room's walls, and [`CompileError::ExitTooWide`] when the requested width
/// would run past the ends of the wall it does lie on.
///
/// # Panics
/// Panics if the exit names a room absent from `ir.rooms` — unreachable,
/// since [`Ir::from_json`] rejects that.
fn resolve_exit(ir: &Ir, exit: &Exit) -> Result<ExitPlan, CompileError> {
    let room_idx = ir
        .rooms
        .iter()
        .position(|r| r.id == exit.room)
        .expect("validated in Ir::from_json");
    let room = &ir.rooms[room_idx];

    let (axis, fixed, lo, hi, forward) = wall_edges(&room.footprint)
        .find(|&(axis, fixed, lo, hi, _)| {
            let (on_axis, across) = axis.split(exit.at);
            across == fixed && on_axis > lo && on_axis < hi
        })
        .ok_or_else(|| CompileError::ExitOffWall {
            room: exit.room.clone(),
            x: exit.at.x,
            y: exit.at.y,
        })?;

    // `width` is positive and even, so the halves are exact (Ir::from_json).
    let half = exit.width / 2;
    let (on_axis, _) = axis.split(exit.at);
    let (open_lo, open_hi) = (on_axis - half, on_axis + half);
    if open_lo < lo || open_hi > hi {
        return Err(CompileError::ExitTooWide {
            room: exit.room.clone(),
            width: exit.width,
            available: hi - lo,
        });
    }

    Ok(ExitPlan {
        room_idx,
        axis,
        fixed,
        open_lo,
        open_hi,
        forward,
    })
}

/// Resolves the linedef special for one exit from its trigger and secrecy.
fn exit_special(tables: &Tables, exit: &Exit) -> u16 {
    match (exit.trigger, exit.secret) {
        (ExitTrigger::Switch, false) => tables.exit_switch_special(),
        (ExitTrigger::Switch, true) => tables.secret_exit_switch_special(),
        (ExitTrigger::Walkover, false) => tables.exit_walkover_special(),
        (ExitTrigger::Walkover, true) => tables.secret_exit_walkover_special(),
    }
}

/// Emits one alcove-only, one-sided wall from `p1` to `p2`, front bound to
/// `sector`.
fn emit_alcove_wall(data: &mut MapData, p1: Pt, p2: Pt, sector: usize, wall_tex: &str) {
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector,
        upper: String::new(),
        middle: wall_tex.to_owned(),
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

/// Carves every exit into its host room's wall.
///
/// See the module documentation for the two constructions. Runs after
/// [`crate::compile::doors::emit_doors`] and before
/// [`crate::compile::things::place_things`], so a thing's clearance is
/// measured against the exit's final geometry.
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` resolves to no
/// texture set, and whatever `resolve_exit` or
/// `portals::split_wall_for_opening` raise.
///
/// # Panics
/// Panics if `emit_opening` (used for a walkover exit's threshold) ever
/// returns a one-sided line — unreachable, as it always emits both
/// sidedefs of the line it pushes.
pub fn emit_exits(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
) -> Result<(), CompileError> {
    // Resolved unconditionally, mirroring `doors::emit_doors`'s own eager
    // resolution of its textures: an unresolvable theme is an authoring
    // error that should surface the same way regardless of which exits (if
    // any) actually use a switch.
    let unknown_theme = || CompileError::UnknownTheme {
        theme: ir.theme.clone(),
    };
    let switch_tex = tables
        .texture("switch", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();

    for exit in &ir.exits {
        let plan = resolve_exit(ir, exit)?;
        let cut = Cut {
            axis: plan.axis,
            fixed: plan.fixed,
            open_lo: plan.open_lo,
            open_hi: plan.open_hi,
        };
        split_wall_for_opening(data, &cut, plan.room_idx, &exit.room)?;

        let special = exit_special(tables, exit);
        let purpose = format!(
            "exit ({:?}{}) in {}",
            exit.trigger,
            if exit.secret { ", secret" } else { "" },
            exit.room
        );
        let tag = tags.allocate(plan.room_idx, &purpose);

        match exit.trigger {
            ExitTrigger::Switch => {
                emit_switch_exit(data, &cut, &plan, special, tag, &switch_tex);
            }
            ExitTrigger::Walkover => {
                emit_walkover_exit(ir, data, &cut, &plan, special, tag);
            }
        }
    }
    Ok(())
}

/// Emits a switch exit's line: the exit's span, replaced by a single
/// one-sided linedef carrying the special and the switch texture. See the
/// module documentation for why this needs no alcove.
fn emit_switch_exit(
    data: &mut MapData,
    cut: &Cut,
    plan: &ExitPlan,
    special: u16,
    tag: u16,
    switch_tex: &str,
) {
    let (p1, p2) = if plan.forward {
        (cut.pt(cut.open_lo), cut.pt(cut.open_hi))
    } else {
        (cut.pt(cut.open_hi), cut.pt(cut.open_lo))
    };
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: plan.room_idx,
        upper: String::new(),
        middle: switch_tex.to_owned(),
        lower: String::new(),
    });
    data.linedefs.push(LinedefOut {
        v1,
        v2,
        front,
        back: None,
        blocking: true,
        special,
        tag,
        lower_unpegged: false,
        upper_unpegged: false,
    });
}

/// Emits a walkover exit's construction: a new closed alcove sector behind
/// the host wall, a passable threshold carrying the special, and the three
/// alcove-only walls that close the recess. See the module documentation for
/// the full derivation.
fn emit_walkover_exit(
    ir: &Ir,
    data: &mut MapData,
    cut: &Cut,
    plan: &ExitPlan,
    special: u16,
    tag: u16,
) {
    let room = &ir.rooms[plan.room_idx];

    // The recess's outward direction is exactly the "toward b" direction a
    // two-room portal would have — there is simply no room `b` for it to be
    // measuring toward.
    let span = SharedSpan {
        axis: plan.axis,
        fixed: plan.fixed,
        lo: plan.open_lo,
        hi: plan.open_hi,
        a_forward: plan.forward,
    };
    let sign = span.across_sign_toward_b();
    let far = plan.fixed + sign * EXIT_ALCOVE_DEPTH;
    let far_cut = Cut {
        axis: plan.axis,
        fixed: far,
        open_lo: plan.open_lo,
        open_hi: plan.open_hi,
    };

    let alcove = data.sectors.len();
    data.sectors.push(SectorOut {
        floor: room.floor,
        ceiling: room.ceiling,
        light: room.light,
        floor_tex: room.floor_tex.clone(),
        ceil_tex: room.ceil_tex.clone(),
        special: 0,
        tag: 0,
    });

    let near_line = emit_opening(data, cut, plan.room_idx, alcove, plan.forward);
    data.linedefs[near_line].special = special;
    data.linedefs[near_line].tag = tag;

    // The alcove's three remaining sides, closing the recess: near_end ->
    // far_end -> far_start -> near_start, where near_start/near_end are the
    // two ends of the threshold this sector's interior lies to the right of
    // (see `emit_opening`'s doc comment for the `a_forward` rule).
    let (near_start, near_end) = if plan.forward {
        (plan.open_hi, plan.open_lo)
    } else {
        (plan.open_lo, plan.open_hi)
    };
    emit_alcove_wall(
        data,
        cut.pt(near_end),
        far_cut.pt(near_end),
        alcove,
        &room.wall_tex,
    );
    emit_alcove_wall(
        data,
        far_cut.pt(near_end),
        far_cut.pt(near_start),
        alcove,
        &room.wall_tex,
    );
    emit_alcove_wall(
        data,
        far_cut.pt(near_start),
        cut.pt(near_start),
        alcove,
        &room.wall_tex,
    );
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::EXIT_ALCOVE_DEPTH;
    use crate::compile::MapData;
    use crate::compile::doors::emit_doors;
    use crate::compile::exits::emit_exits;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::tags::TagAllocator;
    use crate::geom::{Pt, contains};
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// An L-shaped single room: its bounding box is 256x256, but the actual
    /// footprint is an L — a 64-wide vertical arm running the full height
    /// plus a 192-wide horizontal arm along the bottom, with a 192x192
    /// notch missing from the northeast corner. Distinct in shape from every
    /// 256-square fixture elsewhere in this crate, per the project's
    /// fixture-diversity rule (see `KNOWN-GAPS.md`).
    const L_ROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a",
          "footprint":[[0,0],[0,256],[64,256],[64,64],[256,64],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[] }"#;

    /// Runs the full `emit_sectors -> cut_portals -> emit_doors -> emit_exits`
    /// pipeline (there being no doors here, `emit_doors` is a no-op, but
    /// included so this mirrors the real `compile_reporting` order exactly).
    fn compiled(ir_json: &str) -> (Ir, MapData) {
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        emit_exits(&ir, &tables, &mut data, &mut tags).expect("exits");
        (ir, data)
    }

    /// Whether room `room_idx`'s *original* IR footprint has its interior on
    /// the right of travel from `p` to `q`. Deliberately duplicated rather
    /// than shared with `portals`/`doors` — see
    /// `doors::tests::interior_is_on_the_right`'s own doc comment for why.
    fn interior_is_on_the_right(ir: &Ir, room_idx: usize, p: Pt, q: Pt) -> bool {
        let (dx, dy) = (q.x - p.x, q.y - p.y);
        let probe = Pt {
            x: i32::midpoint(p.x, q.x) + dy.signum(),
            y: i32::midpoint(p.y, q.y) - dx.signum(),
        };
        contains(&ir.rooms[room_idx].footprint, probe)
    }

    /// Asserts every sector's boundary closes and every sidedef faces its
    /// real sector, mirroring `portals::tests::assert_well_formed` (not
    /// reused directly — it is private to that module, and per this
    /// project's convention duplicating a small independent check is
    /// preferable to sharing it with the code under test).
    fn assert_well_formed(ir: &Ir, data: &MapData) {
        let mut balance: HashMap<(usize, Pt), i32> = HashMap::new();
        for line in &data.linedefs {
            let (p, q) = (data.vertices[line.v1], data.vertices[line.v2]);
            let front_sector = data.sidedefs[line.front].sector;
            *balance.entry((front_sector, p)).or_default() += 1;
            *balance.entry((front_sector, q)).or_default() -= 1;
            if front_sector < ir.rooms.len() {
                assert!(
                    interior_is_on_the_right(ir, front_sector, p, q),
                    "front sidedef of line {p:?} -> {q:?} names room {front_sector}, but that \
                     room's interior is not on the right of travel"
                );
            }
            if let Some(back) = line.back {
                let back_sector = data.sidedefs[back].sector;
                *balance.entry((back_sector, q)).or_default() += 1;
                *balance.entry((back_sector, p)).or_default() -= 1;
                if back_sector < ir.rooms.len() {
                    assert!(
                        interior_is_on_the_right(ir, back_sector, q, p),
                        "back sidedef of line {p:?} -> {q:?} names room {back_sector}, but that \
                         room's interior is not on the left of travel"
                    );
                }
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

    #[test]
    fn a_switch_exit_stays_one_sided_and_carries_the_special_and_switch_texture() {
        // Exit on the L's east wall (x = 256, y in 0..64), a wall no
        // 256-square fixture exercises.
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":32, "at":[256,32] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        let tables = Tables::load().expect("tables");

        assert_eq!(data.sectors.len(), 1, "a switch exit adds no sector");
        assert_well_formed(&ir, &data);

        let exit_lines: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(exit_lines.len(), 1, "exactly one exit line");
        let line = exit_lines[0];
        assert!(line.back.is_none(), "a switch exit stays one-sided");
        assert!(line.blocking, "the host room stays a closed sector");
        assert_eq!(line.special, tables.exit_switch_special());
        assert_ne!(line.tag, 0, "the exit is tagged uniformly, like a door");
        assert_eq!(data.sidedefs[line.front].middle, "SW1STARG");
    }

    #[test]
    fn a_secret_switch_exit_carries_the_secret_special() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "secret":true, "width":32,
                          "at":[256,32] }],
               "portals":[]"#,
        );
        let (_, data) = compiled(&json);
        let tables = Tables::load().expect("tables");
        let exit_lines: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(exit_lines[0].special, tables.secret_exit_switch_special());
        assert_ne!(
            tables.secret_exit_switch_special(),
            tables.exit_switch_special(),
            "a secret exit really is a different special"
        );
    }

    #[test]
    fn a_walkover_exit_carves_a_passable_two_sided_threshold_into_a_closed_alcove() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":32, "at":[256,32] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        let tables = Tables::load().expect("tables");

        assert_eq!(data.sectors.len(), 2, "the alcove is a real extra sector");
        assert_well_formed(&ir, &data);

        let alcove = 1;
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
            referenced(alcove),
            4,
            "the alcove is a closed quadrilateral: one threshold plus three walls"
        );

        let threshold: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(threshold.len(), 1);
        let line = threshold[0];
        assert!(!line.blocking, "a walkover line must be passable to fire");
        let back = line.back.expect("the threshold is two-sided");
        assert_eq!(
            data.sidedefs[back].sector, alcove,
            "the alcove is the back sector, the room the front"
        );
        assert_eq!(line.special, tables.exit_walkover_special());
        assert_ne!(line.tag, 0);

        // Every alcove-only wall carries the host room's own wall texture.
        for l in data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == alcove)
        {
            assert_eq!(data.sidedefs[l.front].middle, "W");
        }
    }

    #[test]
    fn a_walkover_exit_alcove_extends_outward_not_into_the_room() {
        // The room's east wall over y in 0..64 sits at x = 256; the alcove
        // must sit at x in [256, 256+32], never inside the room's own
        // x < 256 territory.
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":32, "at":[256,32] }],
               "portals":[]"#,
        );
        let (_, data) = compiled(&json);
        let far_x = 256 + 32;
        assert!(
            data.vertices.contains(&Pt { x: far_x, y: 16 })
                && data.vertices.contains(&Pt { x: far_x, y: 48 }),
            "the alcove's far wall lands 32 units outward from the host wall"
        );
        assert!(
            !data.vertices.iter().any(|v| v.x < 0 || v.x > far_x),
            "no emitted vertex falls outside the room's own span extended by the alcove"
        );
    }

    #[test]
    fn an_exit_wider_than_its_wall_is_rejected() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":128, "at":[256,32] }],
               "portals":[]"#,
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(matches!(
            emit_exits(&ir, &tables, &mut data, &mut tags),
            Err(crate::compile::CompileError::ExitTooWide { .. })
        ));
    }

    #[test]
    fn an_exit_not_on_any_wall_is_rejected() {
        // (128, 128) is the L's own inner corner — inside the footprint's
        // bounding box, but not on any of its wall edges.
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":32, "at":[128,128] }],
               "portals":[]"#,
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(matches!(
            emit_exits(&ir, &tables, &mut data, &mut tags),
            Err(crate::compile::CompileError::ExitOffWall { .. })
        ));
    }

    /// The same switch-exit fixture as
    /// `a_switch_exit_stays_one_sided_and_carries_the_special_and_switch_texture`,
    /// on the L's *south* wall (y = 0) instead of its east wall, proving the
    /// construction is not accidentally pinned to one axis.
    #[test]
    fn a_switch_exit_works_on_a_horizontal_wall_too() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":64, "at":[128,0] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        assert_eq!(data.linedefs.iter().filter(|l| l.special != 0).count(), 1);
    }

    /// The east and south walls used by the two switch-exit tests above both
    /// happen to have `forward == false` in `wall_edges`'s sense. This test
    /// puts a switch exit on the L's north wall instead, which has
    /// `forward == true`, so the `plan.forward` branch of
    /// `emit_switch_exit`'s direction choice is actually exercised — without
    /// it, a bug in that specific branch could hide behind every other test
    /// in this file (see the report's mutation-testing section).
    #[test]
    fn a_switch_exit_works_on_a_wall_where_forward_is_true() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":32, "at":[32,256] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        let lines: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].back.is_none());
    }

    /// The same walkover fixture, on the L's north wall (the short arm at
    /// y = 256, x in 0..64) — a wall whose interior lies to the *south*,
    /// the opposite orientation from the east-wall walkover test above.
    #[test]
    fn a_walkover_exit_works_on_a_wall_with_the_opposite_forward_sign() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":32, "at":[32,256] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        assert_eq!(data.sectors.len(), 2);
        let far_y = 256 + EXIT_ALCOVE_DEPTH;
        assert!(
            data.vertices.contains(&Pt { x: 16, y: far_y })
                && data.vertices.contains(&Pt { x: 48, y: far_y }),
            "the alcove extends north, outward from the room"
        );
    }
}
