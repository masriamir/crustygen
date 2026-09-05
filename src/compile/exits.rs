//! Carves the level exit into one wall of its host room.
//!
//! An exit is a linedef special on a segment of one room's own boundary
//! wall — not a connection to a second room, unlike [`crate::ir::Portal`].
//! The span-resolution and wall-splitting machinery is exactly
//! [`crate::compile::portals`]'s, reused directly: "the same machinery,
//! minus the second room."
//!
//! The three [`crate::ir::ExitTrigger`] kinds need different geometry,
//! because they trigger differently in the pinned engine (see the citations
//! in `specials-report.md` and the two "engine facts" already recorded in
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
//!   carves a small dead-end alcove out of the host room's own wall — one
//!   real threshold plus three solid one-sided walls closing off the
//!   remaining three sides, the same shape
//!   `crate::compile::portals::emit_recess` builds. Only the near threshold
//!   (front bound to the host room, back to the alcove) carries the walkover
//!   special. The player steps into the alcove, crosses the threshold, and
//!   the level ends.
//! - **Teleport** — the walkover construction again, in a room that rule P26
//!   requires to be portal-less and carry a teleport marker: the player
//!   teleports in and simply steps across the alcove's threshold to end the
//!   level, exactly like a walkover exit. See `crate::ir::Teleport` for the
//!   marker itself.
//!
//! Every kind resolves its span with `resolve_exit` and carves the host wall
//! with `portals::split_wall_for_opening` before emitting anything — the
//! same "resolve and validate everything, then emit" discipline `cut_portals`
//! and `doors::emit_doors` already follow, so a rejected exit leaves no
//! partially-cut geometry behind.
//!
//! Every exit is tagged from the shared [`TagAllocator`], even though
//! `G_ExitLevel`/`G_SecretExitLevel` read no tag at all. This mirrors
//! `doors::emit_doors`'s own precedent — a manual door is tagged uniformly
//! too, though `EV_VerticalDoor` acts on the line's back sector rather than
//! consulting the tag — so [`crate::compile::tags::check_no_action_at_tag_zero`]
//! stays a single, exception-free invariant and the tag manifest records
//! every action a run took, not just the ones that mechanically need one.

use crate::compile::portals::{Cut, emit_recess, emit_switch_line, split_wall_for_opening};
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, MapData, SectorOut};
use crate::geom::{Axis, on_diagonal_wall, wall_edges};
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
    /// along direction — [`crate::geom::FacingSpan::a_forward`]'s
    /// single-room analogue.
    forward: bool,
}

/// Resolves one exit against its host room's real wall.
///
/// # Errors
/// Returns [`CompileError::ExitOnDiagonalWall`] when `exit.at` sits on a
/// diagonal edge of the room — a real wall, just not one [`wall_edges`]
/// considers, since v1 cannot carve an exit into one; checked before the
/// less specific error below so a diagonal wall is never misreported as
/// no wall at all. Returns [`CompileError::ExitOffWall`] when `exit.at` lies
/// on none of the room's axis-aligned walls, and
/// [`CompileError::ExitTooWide`] when the requested width would run past the
/// ends of the wall it does lie on.
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
        .ok_or_else(|| {
            // A wall v1 simply cannot carve an exit into deserves an
            // honest, specific message rather than "not on any wall" for a
            // point that demonstrably is on one — see `on_diagonal_wall`'s
            // doc comment.
            if on_diagonal_wall(&room.footprint, exit.at) {
                CompileError::ExitOnDiagonalWall {
                    room: exit.room.clone(),
                    x: exit.at.x,
                    y: exit.at.y,
                }
            } else {
                CompileError::ExitOffWall {
                    room: exit.room.clone(),
                    x: exit.at.x,
                    y: exit.at.y,
                }
            }
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
        // A teleport-only exit has no independent trigger special of its
        // own — it is a walkover line in a room reachable only by teleport.
        (ExitTrigger::Walkover | ExitTrigger::Teleport, false) => tables.exit_walkover_special(),
        (ExitTrigger::Walkover | ExitTrigger::Teleport, true) => {
            tables.secret_exit_walkover_special()
        }
    }
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
/// texture set, [`CompileError::RecessOutOfRange`] when a walkover exit's
/// alcove would land outside the 16-bit map range, and whatever
/// `resolve_exit` (including [`CompileError::ExitOnDiagonalWall`], if the
/// requested position sits on a diagonal wall) or
/// `portals::split_wall_for_opening` raise.
///
/// # Panics
/// Panics if `emit_opening` (used via `portals::emit_recess` for a walkover
/// exit's threshold) ever returns a one-sided line — unreachable, as it
/// always emits both sidedefs of the line it pushes.
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
    let switch_width =
        tables
            .switch_width(&ir.theme)
            .ok_or_else(|| CompileError::UnknownTheme {
                theme: ir.theme.clone(),
            })?;
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
                emit_switch_exit(data, &cut, &plan, special, tag, &switch_tex, switch_width);
            }
            ExitTrigger::Walkover | ExitTrigger::Teleport => {
                emit_walkover_exit(ir, data, &cut, &plan, &exit.room, special, tag)?;
            }
        }
    }
    Ok(())
}

/// Emits a switch exit's line: the exit's span, replaced by a single
/// one-sided linedef carrying the special and the switch texture. See the
/// module documentation for why this needs no alcove, and
/// [`emit_switch_line`] — shared with the floor-action switch trigger, which
/// wants the same line on a room's wall — for the texture centering.
fn emit_switch_exit(
    data: &mut MapData,
    cut: &Cut,
    plan: &ExitPlan,
    special: u16,
    tag: u16,
    switch_tex: &str,
    switch_width: i32,
) {
    emit_switch_line(
        data,
        cut,
        plan.room_idx,
        plan.forward,
        special,
        tag,
        switch_tex,
        switch_width,
    );
}

/// Emits a walkover exit's construction: a new closed alcove sector behind
/// the host wall, a passable threshold carrying the special, and the three
/// alcove-only walls that close the recess — via
/// [`crate::compile::portals::emit_recess`], the same construction a
/// teleport-triggered exit (and a teleport's own wall pad) share. See the
/// module documentation for the full derivation.
///
/// # Errors
/// Returns [`CompileError::RecessOutOfRange`] when the alcove's far wall
/// would land outside the 16-bit map range — see
/// [`crate::compile::portals::MAP_RANGE`]'s doc comment for why this needs
/// an explicit check rather than being implied by an already-validated
/// bound.
fn emit_walkover_exit(
    ir: &Ir,
    data: &mut MapData,
    cut: &Cut,
    plan: &ExitPlan,
    exit_room: &str,
    special: u16,
    tag: u16,
) -> Result<(), CompileError> {
    let room = &ir.rooms[plan.room_idx];
    let recess = emit_recess(
        data,
        cut,
        plan.room_idx,
        plan.forward,
        EXIT_ALCOVE_DEPTH,
        SectorOut {
            floor: room.floor,
            ceiling: room.ceiling,
            light: room.light,
            floor_tex: room.floor_tex.clone(),
            ceil_tex: room.ceil_tex.clone(),
            special: 0,
            tag: 0,
            wall_tex: room.wall_tex.clone(),
            host: None,
        },
        exit_room,
    )?;
    data.linedefs[recess.threshold].special = special;
    data.linedefs[recess.threshold].tag = tag;
    Ok(())
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
        cut_portals(&ir, &tables, &mut data).expect("portals");
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
    fn a_switch_exit_centers_its_texture_on_the_line() {
        // SW1STARG is 128 wide; a 32-unit exit line with no offset would
        // show texture columns 0..31 — the far left of the texture, with
        // the switch graphic (which sits near the middle) off the line
        // entirely or hard against one edge. Centring puts the line over
        // columns 48..79, straddling the graphic.
        let tables = Tables::load().expect("tables");
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":32, "at":[256,32] }],
               "portals":[]"#,
        );
        let (_, data) = compiled(&json);
        let special = tables.exit_switch_special();
        let line = data
            .linedefs
            .iter()
            .find(|l| l.special == special)
            .expect("switch exit line");
        let width = tables.switch_width("tech_base").expect("switch width");
        assert_eq!(
            data.sidedefs[line.front].x_offset,
            (width - 32) / 2,
            "the switch texture is centered on its line"
        );
    }

    #[test]
    fn a_wider_exit_still_centers_and_never_offsets_negatively() {
        // A line as wide as the texture needs no shift at all, and the
        // arithmetic must not produce a negative offset for a line wider
        // than its texture.
        let tables = Tables::load().expect("tables");
        let width = tables.switch_width("tech_base").expect("switch width");
        // The L's south wall runs x = 0..256 at y = 0, so a 128-wide exit
        // centered at x = 128 fits inside it.
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":128, "at":[128,0] }],
               "portals":[]"#,
        );
        let (_, data) = compiled(&json);
        let line = data
            .linedefs
            .iter()
            .find(|l| l.special == tables.exit_switch_special())
            .expect("switch exit line");
        let off = data.sidedefs[line.front].x_offset;
        assert_eq!(
            off, 0,
            "a line exactly as wide as its texture needs no shift"
        );
        assert!((0..width).contains(&off), "offset stays inside the texture");
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

    /// A teleport-triggered exit routes to exactly the same construction as
    /// a walkover exit — `exit_special` and `emit_exits`' own match both
    /// fold `ExitTrigger::Teleport` into the `Walkover` arm — so this proves
    /// the routing rather than re-proving the alcove geometry itself, which
    /// the walkover tests above already cover in full.
    #[test]
    fn a_teleport_trigger_exit_is_a_walkover_alcove_with_the_walkover_special() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"teleport", "width":32, "at":[256,32] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        let tables = Tables::load().expect("tables");
        assert_well_formed(&ir, &data);
        assert_eq!(data.sectors.len(), 2, "host plus the alcove");
        let threshold = data
            .linedefs
            .iter()
            .find(|l| l.special != 0)
            .expect("the threshold carries the special");
        assert_eq!(threshold.special, tables.exit_walkover_special());
        assert!(threshold.back.is_some() && !threshold.blocking);
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
        cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(matches!(
            emit_exits(&ir, &tables, &mut data, &mut tags),
            Err(crate::compile::CompileError::ExitTooWide { .. })
        ));
    }

    #[test]
    fn a_walkover_exit_alcove_landing_outside_the_map_range_is_rejected() {
        // A room whose east wall sits at x = 32750: DOOR_DEPTH-style recesses
        // are bounded by the containing room's own already-validated far
        // wall, but a walkover exit's alcove has no containing room, so its
        // far coordinate (32750 + EXIT_ALCOVE_DEPTH = 32782) can genuinely
        // exceed i16::MAX (32767) with nothing else to catch it.
        let json = r#"{ "seed":1, "grid":2, "theme":"tech_base",
          "rooms":[
            { "id":"edge", "footprint":[[32000,0],[32000,64],[32750,64],[32750,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "exits":[{ "room":"edge", "trigger":"walkover", "width":32, "at":[32750,32] }],
          "portals":[] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(matches!(
            emit_exits(&ir, &tables, &mut data, &mut tags),
            Err(crate::compile::CompileError::RecessOutOfRange { .. })
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
        cut_portals(&ir, &tables, &mut data).expect("portals");
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

    // The four tests above cover (Vertical, forward=false) [east, both
    // triggers], (Horizontal, forward=false) [south, switch only so far],
    // and (Horizontal, forward=true) [north, both triggers]. That leaves
    // (Vertical, forward=true) — the L's *west* wall — untested for either
    // trigger, and the south wall untested for walkover. Every `(axis,
    // forward)` combination must be exercised for both trigger kinds: the
    // axis-handling itself is well covered elsewhere (portals, doors), but
    // this project has twice shipped Critical geometry defects from exactly
    // this kind of orientation undercount, so the exit construction gets its
    // own complete matrix rather than resting on that mitigation alone.
    // `ExitTrigger::Teleport` does not get its own copy of this matrix: its
    // construction is byte-identical to `Walkover`'s (both route through
    // `emit_walkover_exit`), so a per-orientation sweep would only re-prove
    // geometry the walkover matrix above already covers —
    // `a_teleport_trigger_exit_is_a_walkover_alcove_with_the_walkover_special`
    // proves the routing once, which is the only thing specific to the
    // teleport trigger.

    /// West wall (edge `(0,0)->(0,256)`, `Axis::Vertical`, `forward = true`)
    /// — the missing fourth `(axis, forward)` combination for a switch exit.
    #[test]
    fn a_switch_exit_works_on_the_west_wall_where_forward_is_true() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":64, "at":[0,128] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        let lines: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].back.is_none(), "a switch exit stays one-sided");
    }

    /// Same west wall, walkover kind — the missing fourth `(axis, forward)`
    /// combination for a walkover exit. `across_sign_toward_b` for
    /// `(Vertical, forward=true)` is `-1`, so the alcove extends toward
    /// *negative* x — the opposite sign from every other walkover test in
    /// this file, none of which previously exercised a negative-direction
    /// alcove at all.
    #[test]
    fn a_walkover_exit_works_on_the_west_wall_where_forward_is_true() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":64, "at":[0,128] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        assert_eq!(data.sectors.len(), 2);
        let far_x = -EXIT_ALCOVE_DEPTH;
        assert!(
            data.vertices.contains(&Pt { x: far_x, y: 96 })
                && data.vertices.contains(&Pt { x: far_x, y: 160 }),
            "the alcove extends west (negative x), outward from the room"
        );
    }

    /// South wall (edge `(256,0)->(0,0)`, `Axis::Horizontal`,
    /// `forward = false`), walkover kind — the south wall was previously
    /// exercised for switch only. `across_sign_toward_b` for `(Horizontal,
    /// forward=false)` is `-1`, so the alcove extends toward negative y.
    #[test]
    fn a_walkover_exit_works_on_the_south_wall_where_forward_is_false() {
        let json = L_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":64, "at":[128,0] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        assert_eq!(data.sectors.len(), 2);
        let far_y = -EXIT_ALCOVE_DEPTH;
        assert!(
            data.vertices.contains(&Pt { x: 96, y: far_y })
                && data.vertices.contains(&Pt { x: 160, y: far_y }),
            "the alcove extends south (negative y), outward from the room"
        );
    }

    /// An octagon: a 256-unit square chamfered by 64 units at each corner —
    /// the same shape as `sectors::tests::OCTAGON` and
    /// `portals::tests::OCTAGON_ROOM`, genuinely diagonal rather than merely
    /// L-shaped like [`L_ROOM`] above.
    const OCTAGON_ROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a",
          "footprint":[[0,64],[0,192],[64,256],[192,256],[256,192],[256,64],[192,0],[64,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[] }"#;

    #[test]
    fn a_switch_exit_works_on_the_axis_aligned_wall_of_a_diagonally_shaped_room() {
        // South wall, x in 64..192 — a straight run between two diagonal
        // chamfers on either side, proving those chamfers do not confuse
        // `wall_edges` into misreporting the span or its ends.
        let json = OCTAGON_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":64, "at":[128,0] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        assert_eq!(data.sectors.len(), 1, "a switch exit adds no sector");
        let lines: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].back.is_none(), "a switch exit stays one-sided");
    }

    #[test]
    fn a_walkover_exit_works_on_the_axis_aligned_wall_of_a_diagonally_shaped_room() {
        let json = OCTAGON_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":64, "at":[128,0] }],
               "portals":[]"#,
        );
        let (ir, data) = compiled(&json);
        assert_well_formed(&ir, &data);
        assert_eq!(data.sectors.len(), 2, "the alcove is a real extra sector");
        let far_y = -EXIT_ALCOVE_DEPTH;
        assert!(
            data.vertices.contains(&Pt { x: 96, y: far_y })
                && data.vertices.contains(&Pt { x: 160, y: far_y }),
            "the alcove extends south, outward from the room, unaffected by the chamfers"
        );
    }

    #[test]
    fn an_exit_requested_on_a_diagonal_wall_names_the_diagonal_wall_not_no_wall_at_all() {
        // (32,224) is the midpoint of the octagon's NW chamfer
        // (0,192)-(64,256) — a real wall, just not one `wall_edges`
        // considers. Before this fix this fell into the same
        // `ExitOffWall` catch-all as a point that is not on any wall at
        // all, e.g. the L's inner corner in `an_exit_not_on_any_wall_is_rejected`
        // above — an honest author reading "not on any wall of the room"
        // for a point they can see sitting exactly on one would have no way
        // to tell the two situations apart.
        let json = OCTAGON_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"switch", "width":16, "at":[32,224] }],
               "portals":[]"#,
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(
            matches!(
                emit_exits(&ir, &tables, &mut data, &mut tags),
                Err(crate::compile::CompileError::ExitOnDiagonalWall { x: 32, y: 224, .. })
            ),
            "a diagonal wall must be named specifically, not folded into ExitOffWall"
        );
    }

    #[test]
    fn a_walkover_exit_requested_on_a_diagonal_wall_is_also_named_specifically() {
        // The switch-exit case above proves `resolve_exit`'s diagonal check
        // fires at all; this proves it is not somehow specific to the
        // switch trigger — `resolve_exit` runs identically before either
        // trigger's own construction (`emit_switch_exit`/
        // `emit_walkover_exit`) ever sees the plan.
        let json = OCTAGON_ROOM.replace(
            "\"portals\":[]",
            r#""exits":[{ "room":"a", "trigger":"walkover", "width":16, "at":[32,224] }],
               "portals":[]"#,
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        assert!(
            matches!(
                emit_exits(&ir, &tables, &mut data, &mut tags),
                Err(crate::compile::CompileError::ExitOnDiagonalWall { x: 32, y: 224, .. })
            ),
            "a walkover exit on a diagonal wall must be named specifically too"
        );
    }
}
