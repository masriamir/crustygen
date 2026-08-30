//! Lifts: `downWaitUpStay` platforms filling a portal gap (lift, barrier)
//! or standing as an island inside a room (pedestal — a later pass).
//!
//! Every platform rests at its own floor and travels to the lowest
//! neighboring floor and back (`p_plats.c`, `EV_DoPlat`, `case
//! downWaitUpStay`: `plat->high = sec->floorheight; plat->low =
//! P_FindLowestFloorSurrounding(sec)`). A lift portal's platform therefore
//! rests at the *higher* room's floor; a barrier's at the shared floor plus
//! [`crate::ir::Portal::rise`]. The use special goes on the platform's low
//! face with the platform as the line's back sector, because
//! `P_UseSpecialLine` fires from the front side only and the player who
//! needs the lift stands in the low room.
//!
//! Risers: the engine draws a lower texture on the sidedef whose own sector
//! has the lower floor (`r_segs.c`, `R_StoreWallRange`; see
//! [`crate::compile::heights::visible_lower_side`]). On the low face that is
//! the low neighbor's sidedef, visible at rest; on the top face it is the
//! platform's own sidedef, visible only once the platform has gone down —
//! which the [`crate::compile::heights`] pass, reading load-time floors,
//! would leave blank. Both are written here, flag-clear, so the texture
//! rides with the platform (`ML_DONTPEGBOTTOM` clear anchors the lower to
//! the back sector's floor).
//!
//! The gap itself is already open when this pass runs:
//! [`crate::compile::portals::cut_portals`] cuts both rooms' own walls for a
//! lift portal but leaves the void between them empty, exactly as it does
//! for a door — the platform is a sector, not a line.

use crate::compile::portals::{
    Cut, emit_jambs, emit_opening, emit_segment, mark_secret_thresholds, resolve_portal,
};
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, MapData, SectorOut};
use crate::ir::{Ir, LiftSpeed, LiftTrigger, Portal, PortalKind, Room};
use crate::tables::{Tables, ThingDims};

/// What a platform joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiftShape {
    /// A level room and a low room.
    Lift,
    /// Two rooms at one floor, with the platform risen between them.
    Barrier,
    /// A raised island inside one host room.
    Pedestal,
}

/// One emitted platform, for `reach`, the rules and the conformance report.
#[derive(Debug, Clone)]
pub struct LiftOut {
    /// Index of the platform's own sector in [`MapData::sectors`].
    pub sector: usize,
    /// What the platform joins.
    pub shape: LiftShape,
    /// Rest floor minus the lowest neighbor's floor, as the engine will
    /// compute it (`P_FindLowestFloorSurrounding`).
    pub travel: i32,
    /// Sectors a player calls the lift from and steps onto it from: the
    /// low-face neighbor (a room or its alcove), both neighbors of a
    /// barrier (each a room or its alcove), a pedestal's host.
    pub callable_from: Vec<usize>,
    /// The sector tag the platform's specials act on.
    pub tag: u16,
    /// Index into [`Ir::portals`], for a lift or barrier.
    pub portal: Option<usize>,
    /// Index into [`Ir::pedestals`], for a pedestal.
    pub pedestal: Option<usize>,
    /// The platform's low-face threshold (portals only).
    pub low_line: Option<usize>,
    /// The platform's top-face threshold (portals only).
    pub top_line: Option<usize>,
}

/// A sector borrowing `room`'s light and flats, at explicit heights and with
/// an explicit wall texture and tag.
///
/// The same shape [`crate::compile::doors::emit_doors`] builds its alcoves
/// from: a compiler-made sector belongs to no room, so it takes its
/// appearance from the room it adjoins rather than inventing one.
fn sector_like(room: &Room, floor: i32, ceiling: i32, wall_tex: &str, tag: u16) -> SectorOut {
    SectorOut {
        floor,
        ceiling,
        light: room.light,
        floor_tex: room.floor_tex.clone(),
        ceil_tex: room.ceil_tex.clone(),
        special: 0,
        tag,
        wall_tex: wall_tex.to_owned(),
        host: None,
    }
}

/// Emits every lift portal and barrier as a `downWaitUpStay` platform
/// filling the portal's gap.
///
/// See the module documentation for the construction and the engine
/// reasoning behind the rest floor, the special's side, and the two risers.
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` resolves to no
/// texture set, [`CompileError::LiftTravelTooShort`] when a lift's two
/// rooms differ by no more than a step (the player would simply walk up),
/// [`CompileError::LiftRiseTooLow`] when a barrier's rise is no more than a
/// step, [`CompileError::LiftTooShallow`] when the alcoves leave the
/// platform narrower than the player's diameter,
/// [`CompileError::TooManyPlats`] when the map wants more platforms than the
/// engine's `MAXPLATS` can run at once, and whatever `resolve_portal` raises
/// (`NotAdjacent`, `PortalOffWall`, `PortalOnDiagonalWall`, `PortalTooWide`)
/// if a lift portal's rooms are not adjacent on a wall v1 can cut.
pub fn emit_lifts(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
) -> Result<Vec<LiftOut>, CompileError> {
    let unknown_theme = || CompileError::UnknownTheme {
        theme: ir.theme.clone(),
    };
    let riser = tables
        .texture("lift_riser", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();
    let trim = tables
        .texture("trim", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();
    let step = tables.step_height();
    let player = tables.player();

    let mut out = Vec::new();
    for (pi, portal) in ir.portals.iter().enumerate() {
        if portal.kind != PortalKind::Lift {
            continue;
        }
        out.push(emit_portal_lift(
            ir, tables, data, tags, pi, portal, &riser, &trim, step, player,
        )?);
    }
    // pedestals: Task 4

    // `MAXPLATS` bounds how many plats the engine can have *active* at
    // once, and `P_AddActivePlat` calls `I_Error` past it (the citation on
    // `data/engine.toml`'s `[plat]` entry, which is where `max_active` comes
    // from). Counting every emitted platform is the conservative reading:
    // nothing here can prove a player will not have them all moving at the
    // same moment.
    let max = tables.plat().max_active;
    if out.len() > max {
        return Err(CompileError::TooManyPlats {
            count: out.len(),
            max,
        });
    }
    Ok(out)
}

/// Emits one lift or barrier portal: up to two alcoves, the platform sector
/// filling what the alcoves leave of the gap, its two thresholds and jambs,
/// the trigger specials and the two risers.
#[expect(
    clippy::too_many_arguments,
    reason = "each parameter names an independent input — the IR, the tables, the two \
              accumulators, the portal and its index, and the four values hoisted out of the \
              per-map loop; bundling them would just move the same count into a throwaway struct"
)]
#[expect(
    clippy::too_many_lines,
    reason = "the platform construction (up to three sectors, up to four boundaries, jambs, \
              trigger specials and riser textures) is one coherent unit of work per lift portal, \
              exactly as `doors::emit_doors` is per door; splitting it would scatter the \
              sequential dependency between pos0..pos3 across call boundaries"
)]
fn emit_portal_lift(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
    pi: usize,
    portal: &Portal,
    riser: &str,
    trim: &str,
    step: i32,
    player: ThingDims,
) -> Result<LiftOut, CompileError> {
    // Resolved independently rather than trusting `cut_portals` already
    // ran — the same defense in depth `doors::emit_doors` and
    // `exits::emit_exits` follow.
    let geometry = resolve_portal(ir, portal)?;
    let (room_a, room_b) = (&ir.rooms[geometry.ia], &ir.rooms[geometry.ib]);
    let barrier = ir.is_barrier(portal);

    // `EV_DoPlat`'s `downWaitUpStay` takes the platform's own floor as its
    // high position and `P_FindLowestFloorSurrounding` as its low, so the
    // rest floor is what the author must choose and the travel is derived.
    let rest = room_a.floor.max(room_b.floor) + portal.rise.unwrap_or(0);
    let travel = rest - room_a.floor.min(room_b.floor);

    // A platform the player could simply walk onto is not a lift: it is a
    // step, and it would leave a `downWaitUpStay` special on geometry that
    // needs none. Barriers are judged on their rise, since their two rooms
    // are level by definition and the travel is exactly the rise.
    if barrier {
        let rise = portal
            .rise
            .expect("Ir::from_json requires rise on a barrier");
        if rise <= step {
            return Err(CompileError::LiftRiseTooLow {
                a: portal.a.clone(),
                b: portal.b.clone(),
                rise,
                step,
            });
        }
    } else if travel <= step {
        return Err(CompileError::LiftTravelTooShort {
            a: portal.a.clone(),
            b: portal.b.clone(),
            delta: travel,
            step,
        });
    }

    let axis = geometry.span.axis;
    let a_forward = geometry.span.a_forward;
    let (open_lo, open_hi) = (geometry.open_lo, geometry.open_hi);
    let alcove_near = portal.alcove_near.unwrap_or(0);
    let alcove_far = portal.alcove_far.unwrap_or(0);

    // Positions along the gap axis, from room `a`'s own wall (`pos0`) to
    // room `b`'s (`pos3`). The platform occupies `pos1`..`pos2`, whatever
    // the alcoves leave — unlike a door, a lift declares no thickness of its
    // own, so it always fills the remainder exactly.
    let dir = (geometry.span.far - geometry.span.near).signum();
    let pos0 = geometry.span.near;
    let pos1 = pos0 + dir * alcove_near;
    let pos3 = geometry.span.far;
    let pos2 = pos3 - dir * alcove_far;

    // A platform the player cannot stand on is not a lift either: they
    // would ride it wedged against a wall, or fail to board it at all.
    //
    // Measured *signed* along `dir`, not as a distance: unlike a door,
    // whose thickness and alcoves `Ir::from_json` forces to sum to the gap
    // exactly, a lift's alcoves are free to overrun it — and two 32-unit
    // alcoves in a 32-unit gap put `pos2` behind `pos1`, whose absolute
    // separation is a perfectly healthy-looking 32. Signed, that case is
    // -32 and is refused here rather than emitting a platform whose two
    // faces are in reversed order.
    let depth = (pos2 - pos1) * dir;
    let need = player.radius * 2;
    if depth < need {
        return Err(CompileError::LiftTooShallow {
            a: portal.a.clone(),
            b: portal.b.clone(),
            depth,
            need,
        });
    }

    // Alcoves first, exactly as `emit_doors` pushes them, each at its own
    // room's floor and ceiling: an alcove is a piece of the room it opens
    // off, not of the platform.
    let near_alcove = (alcove_near > 0).then(|| {
        let idx = data.sectors.len();
        data.sectors.push(sector_like(
            room_a,
            room_a.floor,
            room_a.ceiling,
            &room_a.wall_tex,
            0,
        ));
        idx
    });
    let far_alcove = (alcove_far > 0).then(|| {
        let idx = data.sectors.len();
        data.sectors.push(sector_like(
            room_b,
            room_b.floor,
            room_b.ceiling,
            &room_b.wall_tex,
            0,
        ));
        idx
    });

    // The platform takes the level room's flat and light — the room whose
    // floor it rests flush with, so the seam is invisible when it is up. A
    // barrier is flush with neither (it rests above both), so it takes room
    // `a`'s, the way every other compiler-made sector does.
    let level_room = if !barrier && room_b.floor > room_a.floor {
        room_b
    } else {
        room_a
    };
    let plat = data.sectors.len();
    let purpose = format!(
        "{} {} <-> {}",
        if barrier { "barrier" } else { "lift" },
        portal.a,
        portal.b
    );
    let tag = tags.allocate(plat, &purpose);
    data.sectors.push(sector_like(
        level_room,
        rest,
        room_a.ceiling.min(room_b.ceiling),
        riser,
        tag,
    ));

    // The platform's own two faces, built in one call since neither is
    // shared with anything else: each is also the inner boundary of the
    // alcove in front of it, when there is one.
    let near_neighbor = near_alcove.unwrap_or(geometry.ia);
    let far_neighbor = far_alcove.unwrap_or(geometry.ib);
    let seg = emit_segment(
        data,
        axis,
        open_lo,
        open_hi,
        a_forward,
        pos1,
        pos2,
        near_neighbor,
        plat,
        far_neighbor,
        riser,
    );
    debug_assert_eq!(
        seg.sector, plat,
        "the platform segment was pushed at the predicted index"
    );

    // Each present alcove's own *outer* threshold and jambs, exactly as
    // `emit_doors` builds them — its inner threshold is one of the
    // platform's own two faces, already emitted above.
    let mut thresholds = vec![seg.near_line, seg.far_line];
    let mut near_outer = None;
    let mut far_outer = None;
    if let Some(alcove) = near_alcove {
        let line = emit_opening(
            data,
            &Cut {
                axis,
                fixed: pos0,
                open_lo,
                open_hi,
            },
            geometry.ia,
            alcove,
            a_forward,
        );
        emit_jambs(
            data, axis, open_lo, open_hi, a_forward, pos0, pos1, alcove, trim,
        );
        thresholds.push(line);
        near_outer = Some(line);
    }
    if let Some(alcove) = far_alcove {
        let line = emit_opening(
            data,
            &Cut {
                axis,
                fixed: pos3,
                open_lo,
                open_hi,
            },
            geometry.ib,
            alcove,
            !a_forward,
        );
        emit_jambs(
            data, axis, open_lo, open_hi, a_forward, pos2, pos3, alcove, trim,
        );
        thresholds.push(line);
        far_outer = Some(line);
    }
    mark_secret_thresholds(data, room_a.secret != room_b.secret, thresholds);

    // Which face is the low one: the face whose neighbor stands on the low
    // floor. A barrier rests above both rooms, so its near face is "low" by
    // convention only — both faces get the same treatment below.
    let a_is_low = !barrier && room_a.floor < room_b.floor;
    let (low_line, top_line, low_neighbor, low_outer) = if barrier || a_is_low {
        (seg.near_line, seg.far_line, near_neighbor, near_outer)
    } else {
        (seg.far_line, seg.near_line, far_neighbor, far_outer)
    };

    let fast = portal.speed == LiftSpeed::Fast;
    let use_special = tables.lift_special(true, fast);
    let walk_special = tables.lift_special(false, fast);
    let set = |data: &mut MapData, line: usize, special: u16| {
        data.linedefs[line].special = special;
        data.linedefs[line].tag = tag;
    };
    match portal.trigger {
        // The riser is the switch: `P_UseSpecialLine` fires from a line's
        // front side, which is the low neighbor's.
        LiftTrigger::Switch => {
            set(data, low_line, use_special);
            if barrier {
                set(data, top_line, use_special);
            }
        }
        // A walkover line on the riser would be unreachable — the player
        // cannot cross the face of a platform that is up — so it goes on the
        // alcove's outer threshold, which they cross to stand in front of it.
        LiftTrigger::Walkover => {
            let outer = low_outer
                .expect("Ir::from_json requires the low room's alcove for a walkover lift");
            set(data, outer, walk_special);
        }
        // The switch below, plus a walkover on the top face so a player
        // arriving from the level room sends the platform down ahead of them.
        LiftTrigger::BothEnds => {
            set(data, low_line, use_special);
            set(data, top_line, walk_special);
        }
    }

    // Risers, on the one sidedef `r_segs.c` draws each from: the lower
    // neighbor's on the low face (and on both faces of a barrier, which
    // stands above both rooms), and the platform's own on the top face of a
    // lift, where the two floors are level at load time and only part once
    // the platform goes down. Pegging flags stay clear so each rides with
    // the sector that moves.
    let low_front = data.linedefs[low_line].front;
    riser.clone_into(&mut data.sidedefs[low_front].lower);
    let top_side = if barrier {
        data.linedefs[top_line].front
    } else {
        data.linedefs[top_line]
            .back
            .expect("emit_segment's thresholds are always two-sided")
    };
    riser.clone_into(&mut data.sidedefs[top_side].lower);

    let callable_from = if barrier {
        vec![near_neighbor, far_neighbor]
    } else {
        vec![low_neighbor]
    };
    Ok(LiftOut {
        sector: plat,
        shape: if barrier {
            LiftShape::Barrier
        } else {
            LiftShape::Lift
        },
        travel,
        callable_from,
        tag,
        portal: Some(pi),
        pedestal: None,
        low_line: Some(low_line),
        top_line: Some(top_line),
    })
}

#[cfg(test)]
mod tests {
    use super::{LiftOut, LiftShape, emit_lifts};
    use crate::compile::tags::TagAllocator;
    use crate::compile::{
        CompileError, MapData, compile, doors, exits, portals, sectors, teleports,
    };
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// Two rooms 64 units apart, room `b` a full 128 units above room `a`,
    /// joined by a lift portal.
    ///
    /// Room `a`'s ceiling is 192 rather than 128 deliberately: the platform
    /// rests at room `b`'s floor (128), so a 128 ceiling would leave it zero
    /// headroom and `cut_portals` would refuse the portal before `emit_lifts`
    /// ever ran.
    const LIFT: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":128, "ceiling":256, "light":144,
          "floor_tex":"FLAT1", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"lift", "width":64, "at":[256,128] } ]
    }"#;

    /// Everything the passes up to and including `emit_lifts` produced.
    struct Built {
        tables: Tables,
        data: MapData,
        tags: TagAllocator,
        lifts: Vec<LiftOut>,
    }

    /// Runs the passes exactly as `compile_reporting` does, through
    /// `emit_lifts`.
    fn compile_data(json: &str) -> Built {
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = sectors::emit_sectors(&ir).expect("sectors");
        sectors::resolve_secret_specials(&ir, &tables, &mut data);
        portals::cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        doors::emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        exits::emit_exits(&ir, &tables, &mut data, &mut tags).expect("exits");
        teleports::emit_teleports(&ir, &tables, &mut data, &mut tags).expect("teleports");
        let lifts = emit_lifts(&ir, &tables, &mut data, &mut tags).expect("lifts");
        Built {
            tables,
            data,
            tags,
            lifts,
        }
    }

    /// The sector on each side of linedef `i`: (front sector, back sector).
    fn sides(data: &MapData, i: usize) -> (usize, Option<usize>) {
        let l = &data.linedefs[i];
        (
            data.sidedefs[l.front].sector,
            l.back.map(|b| data.sidedefs[b].sector),
        )
    }

    #[test]
    fn a_lift_portal_emits_one_platform_at_the_high_floor_with_the_switch_on_its_low_face() {
        let Built {
            tables,
            data,
            tags,
            lifts,
        } = compile_data(LIFT);
        assert_eq!(lifts.len(), 1);
        let l = &lifts[0];
        assert_eq!(l.shape, LiftShape::Lift);
        assert_eq!(l.travel, 128);
        assert_eq!(
            l.callable_from,
            vec![0],
            "callable from room a, the low room"
        );
        let plat = &data.sectors[l.sector];
        assert_eq!(plat.floor, 128, "rests at the higher room's floor");
        assert_eq!(plat.ceiling, 192, "the lower of the two ceilings");
        assert_eq!(plat.light, 144, "the level room's light");
        assert_eq!(plat.floor_tex, "FLAT1", "the level room's flat");
        assert_eq!(plat.wall_tex, "SUPPORT3", "the theme's lift riser");
        assert_eq!(plat.tag, l.tag);
        assert_ne!(l.tag, 0);
        let low = l.low_line.expect("a portal lift has a low face");
        let top = l.top_line.expect("and a top face");
        assert_eq!(
            sides(&data, low),
            (0, Some(l.sector)),
            "low room on the front, platform on the back"
        );
        assert_eq!(sides(&data, top), (1, Some(l.sector)));
        assert_eq!(data.linedefs[low].special, tables.lift_special(true, false));
        assert_eq!(data.linedefs[low].tag, l.tag);
        assert_eq!(
            data.linedefs[top].special, 0,
            "`switch` puts nothing on the top face"
        );
        // Riser textures: the low room's sidedef on the low face, the
        // platform's own on the top face.
        assert_eq!(data.sidedefs[data.linedefs[low].front].lower, "SUPPORT3");
        assert_eq!(
            data.sidedefs[data.linedefs[top].back.unwrap()].lower,
            "SUPPORT3"
        );
        assert_eq!(
            data.sidedefs[data.linedefs[top].front].lower, "",
            "the level room's sidedef shows nothing"
        );
        for i in [low, top] {
            assert!(
                !data.linedefs[i].lower_unpegged && !data.linedefs[i].upper_unpegged,
                "flags clear: the riser rides with the platform"
            );
        }
        assert!(
            tags.manifest()
                .iter()
                .any(|e| e.tag == l.tag && e.purpose == "lift a <-> b")
        );
    }

    #[test]
    fn both_ends_adds_the_walkover_to_the_top_face_and_fast_selects_the_blaze_pair() {
        let json = LIFT.replacen(
            r#""at":[256,128] }"#,
            r#""at":[256,128], "trigger":"both_ends", "speed":"fast" }"#,
            1,
        );
        let Built {
            tables,
            data,
            lifts,
            ..
        } = compile_data(&json);
        let l = &lifts[0];
        assert_eq!(
            data.linedefs[l.low_line.unwrap()].special,
            tables.lift_special(true, true)
        );
        assert_eq!(
            data.linedefs[l.top_line.unwrap()].special,
            tables.lift_special(false, true)
        );
        assert_eq!(data.linedefs[l.top_line.unwrap()].tag, l.tag);
    }

    #[test]
    fn a_walkover_lift_puts_the_special_on_the_low_alcoves_outer_threshold() {
        let json = LIFT.replacen(
            r#""at":[256,128] }"#,
            r#""at":[256,128], "trigger":"walkover", "alcove_near":16 }"#,
            1,
        );
        let Built {
            tables,
            data,
            lifts,
            ..
        } = compile_data(&json);
        let l = &lifts[0];
        // Sectors: a, b, alcove (pushed first), platform.
        assert_eq!(data.sectors.len(), 4);
        let alcove = 2;
        assert_eq!(
            data.sectors[alcove].floor, 0,
            "the alcove is at the low room's floor"
        );
        assert_eq!(
            l.callable_from,
            vec![alcove],
            "the player is in the alcove when the platform arrives"
        );
        let outer = data
            .linedefs
            .iter()
            .position(|ld| {
                let (f, b) = (
                    data.sidedefs[ld.front].sector,
                    ld.back.map(|b| data.sidedefs[b].sector),
                );
                f == 0 && b == Some(alcove)
            })
            .expect("the alcove's outer threshold fronts room a");
        assert_eq!(
            data.linedefs[outer].special,
            tables.lift_special(false, false)
        );
        assert_eq!(data.linedefs[outer].tag, l.tag);
        let low = l.low_line.unwrap();
        assert_eq!(sides(&data, low), (alcove, Some(l.sector)));
        assert_eq!(
            data.linedefs[low].special, 0,
            "nothing on the riser for a walkover lift"
        );
        assert_eq!(
            data.sidedefs[data.linedefs[low].front].lower, "SUPPORT3",
            "the alcove's sidedef carries the riser"
        );
    }

    #[test]
    fn a_barrier_rests_rise_above_both_rooms_with_the_switch_on_both_faces() {
        let json = LIFT
            .replacen(
                r#""floor":128, "ceiling":256"#,
                r#""floor":0, "ceiling":256"#,
                1,
            )
            .replacen(r#""at":[256,128] }"#, r#""at":[256,128], "rise":96 }"#, 1);
        let Built {
            tables,
            data,
            lifts,
            ..
        } = compile_data(&json);
        let l = &lifts[0];
        assert_eq!(l.shape, LiftShape::Barrier);
        assert_eq!(data.sectors[l.sector].floor, 96);
        assert_eq!(l.travel, 96);
        assert_eq!(l.callable_from, vec![0, 1]);
        for line in [l.low_line.unwrap(), l.top_line.unwrap()] {
            assert_eq!(
                data.linedefs[line].special,
                tables.lift_special(true, false)
            );
            assert_eq!(data.linedefs[line].tag, l.tag);
            assert_eq!(data.sidedefs[data.linedefs[line].front].lower, "SUPPORT3");
            assert!(!data.linedefs[line].lower_unpegged);
        }
    }

    #[test]
    fn table_dependent_rejections() {
        let step = LIFT.replacen(
            r#""floor":128, "ceiling":256"#,
            r#""floor":24, "ceiling":256"#,
            1,
        );
        let ir = Ir::from_json(&step).expect("ir");
        let tables = Tables::load().expect("tables");
        assert!(matches!(
            compile(&ir, &tables),
            Err(CompileError::LiftTravelTooShort {
                delta: 24,
                step: 24,
                ..
            })
        ));

        let low_rise = LIFT
            .replacen(
                r#""floor":128, "ceiling":256"#,
                r#""floor":0, "ceiling":256"#,
                1,
            )
            .replacen(r#""at":[256,128] }"#, r#""at":[256,128], "rise":24 }"#, 1);
        let ir = Ir::from_json(&low_rise).expect("ir");
        assert!(matches!(
            compile(&ir, &tables),
            Err(CompileError::LiftRiseTooLow { rise: 24, .. })
        ));

        // A 64-unit gap minus 32 of alcove leaves 32, the player's diameter —
        // just enough; 48 of alcove does not.
        let shallow = LIFT.replacen(
            r#""at":[256,128] }"#,
            r#""at":[256,128], "alcove_near":32, "alcove_far":16 }"#,
            1,
        );
        let ir = Ir::from_json(&shallow).expect("ir");
        assert!(matches!(
            compile(&ir, &tables),
            Err(CompileError::LiftTooShallow {
                depth: 16,
                need: 32,
                ..
            })
        ));
    }

    /// Alcoves that overrun the gap entirely put the platform's far face
    /// *behind* its near one. Their absolute separation looks like a
    /// perfectly rideable 32 units, which is exactly why the depth check is
    /// signed — nothing downstream catches the reversed geometry: before the
    /// signed check this map compiled clean.
    #[test]
    fn alcoves_that_overrun_the_gap_are_refused_rather_than_reversing_the_platform() {
        let json = r#"{ "seed":1, "grid":32, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
            { "id":"b", "footprint":[[288,0],[288,256],[576,256],[576,0]], "floor":128, "ceiling":256, "light":144,
              "floor_tex":"FLAT1", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[ { "a":"a", "b":"b", "kind":"lift", "width":64, "at":[256,128],
                        "alcove_near":32, "alcove_far":32 } ]
        }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        assert!(matches!(
            compile(&ir, &tables),
            Err(CompileError::LiftTooShallow {
                depth: -32,
                need: 32,
                ..
            })
        ));
    }

    #[test]
    fn a_barrier_too_tall_for_the_player_has_no_headroom() {
        let json = LIFT
            .replacen(
                r#""floor":128, "ceiling":256"#,
                r#""floor":0, "ceiling":128"#,
                1,
            )
            .replacen(r#""at":[256,128] }"#, r#""at":[256,128], "rise":96 }"#, 1);
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        assert!(matches!(
            compile(&ir, &tables),
            Err(CompileError::PortalNoHeadroom {
                have: 32,
                need: 56,
                ..
            })
        ));
    }
}
