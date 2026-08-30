//! Lifts: `downWaitUpStay` platforms filling a portal gap (lift, barrier)
//! or standing as an island inside a room (pedestal).
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
//! would leave blank.
//!
//! Both are written here lower-pegged (`ML_DONTPEGBOTTOM` clear), but that
//! means a different thing on each face, because `ML_DONTPEGBOTTOM` clear
//! anchors a lower texture to the *back* sector's floor. On the low face the
//! drawn sidedef is the low neighbor's and its back sector is the platform,
//! so that riser rides with the platform — the opposite of a door track,
//! which is lower-unpegged so `DOORTRAK` stays put. On the top face the drawn
//! sidedef is the platform's own and its back sector is the level room,
//! whose floor never moves, so that riser hangs from a fixed top and grows
//! downward as the platform descends. Clear on both is the corpus
//! convention: risers are pegged 96 % of the time
//! (`docs/measurements/lift-shapes-2026-08-29.md` §G).
//!
//! The *upper* flag goes the other way on a platform boundary whose
//! neighbor's ceiling is the taller one: that neighbor's sidedef draws an
//! upper, and `unpeg_landing_upper` sets `ML_DONTPEGTOP` on the line so the
//! upper starts at the landing's own ceiling rather than at the platform's,
//! which is where the one-sided walls beside it start. Cosmetic — the
//! platform's ceiling never moves — and the corpus names no convention to
//! follow, so the rendering argument decides; see that function.
//!
//! A pedestal is that same platform with no portal under it: a hosted
//! island cut inside one room — [`crate::compile::teleports`]'s pad
//! construction, reused — whose floor rests [`crate::ir::Pedestal::rise`]
//! above its host's. The host is its only neighbor, so
//! `P_FindLowestFloorSurrounding` finds the host's floor and the travel is
//! exactly the rise. There is no low face to pick out: all four edges are
//! low faces, the island winding puts the host on the front of each, and
//! every one carries the use special so the player can call the pedestal
//! down from whichever side they walk up to. Each edge's riser goes on that
//! same host sidedef — the host's floor is the lower one, so it is the side
//! `r_segs.c` draws — with the pegging flags clear, exactly as on a portal
//! lift's low face and for the same reason.
//!
//! The gap itself is already open when this pass runs:
//! [`crate::compile::portals::cut_portals`] cuts both rooms' own walls for a
//! lift portal but leaves the void between them empty, exactly as it does
//! for a door — the platform is a sector, not a line.

use crate::compile::portals::{
    Cut, emit_jambs, emit_opening, emit_segment, mark_secret_thresholds, resolve_portal,
};
use crate::compile::tags::TagAllocator;
use crate::compile::teleports::emit_island_edges;
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

/// Unpegs the upper on a platform boundary whose neighbor's ceiling stands
/// above the platform's — the landing over a shaft or an alcove.
///
/// `r_segs.c`'s `R_StoreWallRange` draws a two-sided line's upper only under
/// `if (worldhigh < worldtop)` (lines 565-587), i.e. on the sidedef whose own
/// sector has the *higher* ceiling: the landing's, never the platform's. With
/// `ML_DONTPEGTOP` clear the engine takes `vtop = backsector->ceilingheight +
/// textureheight[...]`, putting the texture's **bottom** row on the
/// platform's ceiling; the one-sided walls flanking that landing take
/// `rw_midtexturemid = worldtop` (lines 456-475, `ML_DONTPEGBOTTOM` clear),
/// putting their **top** row on the landing's own ceiling. Two neighboring
/// surfaces then start the same texture at different rows — a visible seam.
/// Set, `rw_toptexturemid = worldtop` anchors the upper at the landing's
/// ceiling too and the rows line up.
///
/// The corpus states no convention to follow here — `ML_DONTPEGTOP` is on
/// 51.4 % / 6.0 % / 21.5 % of lift top faces across the three populations
/// (`docs/measurements/lift-shapes-2026-08-29.md` §G2) — so the rendering
/// argument decides. Nothing is at stake beyond appearance: a platform's
/// ceiling never moves, so unlike the riser's `ML_DONTPEGBOTTOM` this flag
/// changes no texture's behavior as the platform travels.
///
/// A no-op on a one-sided line (the engine never reads `ML_DONTPEGTOP`
/// there), on a line that does not border `plat`, and where the two ceilings
/// are equal or the neighbor's is the lower one.
fn unpeg_landing_upper(data: &mut MapData, line: usize, plat: usize) {
    let ld = &data.linedefs[line];
    let Some(back) = ld.back else { return };
    let (front_sector, back_sector) = (data.sidedefs[ld.front].sector, data.sidedefs[back].sector);
    let neighbor = if front_sector == plat {
        back_sector
    } else if back_sector == plat {
        front_sector
    } else {
        return;
    };
    if data.sectors[neighbor].ceiling > data.sectors[plat].ceiling {
        data.linedefs[line].upper_unpegged = true;
    }
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
/// [`CompileError::PedestalRiseTooLow`] when a pedestal rises no more than a
/// step, [`CompileError::PedestalTooSmall`] when a pedestal's rectangle is
/// narrower than the player's diameter, [`CompileError::PedestalNoHeadroom`]
/// when a pedestal's risen floor leaves the player less than their own
/// height under the host's ceiling, [`CompileError::TooManyPlats`] when the
/// map wants more platforms than the engine's `MAXPLATS` can run at once,
/// and whatever `resolve_portal` raises
/// (`NotAdjacent`, `PortalOffWall`, `PortalOnDiagonalWall`, `PortalTooWide`)
/// if a lift portal's rooms are not adjacent on a wall v1 can cut.
///
/// # Panics
/// Panics if a pedestal names a room that does not exist, which
/// [`crate::ir::Ir::from_json`] rejects before this pass ever runs.
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
    for (i, p) in ir.pedestals.iter().enumerate() {
        // A pedestal the player can step onto is not a platform: it is a
        // block of scenery carrying a `downWaitUpStay` special nobody needs.
        // Judged on the rise, which for a pedestal is also the travel.
        if p.rise <= step {
            return Err(CompileError::PedestalRiseTooLow {
                pedestal: p.id.clone(),
                rise: p.rise,
                step,
            });
        }

        // And a pedestal the player cannot stand on top of is not one
        // either. Both sides are checked: the player is a cylinder, so the
        // narrower side is what decides, whichever it is.
        let (lo, hi) = p.rect();
        let (w, h) = (hi.x - lo.x, hi.y - lo.y);
        let min = player.radius * 2;
        if w < min || h < min {
            return Err(CompileError::PedestalTooSmall {
                pedestal: p.id.clone(),
                width: w,
                height: h,
                min,
            });
        }

        // The platform keeps its host's ceiling, so rising eats the room's
        // headroom. Checked for the player here, whatever the pedestal
        // carries — they must be able to ride it up; `things::place_things`
        // checks the cargo against the same gap, where thing dimensions are
        // already being resolved.
        let host = ir
            .rooms
            .iter()
            .position(|r| r.id == p.room)
            .expect("validated by Ir::from_json");
        let room = &ir.rooms[host];
        let floor = room.floor + p.rise;
        let have = room.ceiling - floor;
        if have < player.height {
            return Err(CompileError::PedestalNoHeadroom {
                pedestal: p.id.clone(),
                kind: "player".to_owned(),
                have,
                need: player.height,
            });
        }

        // `host` marks the island as a hole inside its host room, which is
        // what `sectors::check_no_sector_overlaps` exempts from the overlap
        // test — a pedestal lies inside its host by construction.
        let sector = data.sectors.len();
        let tag = tags.allocate(sector, &format!("pedestal {}", p.id));
        let mut s = sector_like(room, floor, room.ceiling, &riser, tag);
        s.host = Some(host);
        data.sectors.push(s);

        // Every edge is a low face, so every edge is a switch: the host
        // surrounds the island, and `P_UseSpecialLine` fires from the front
        // side, which the island winding binds to the host. The riser goes
        // on that same sidedef, the lower-floored one `r_segs.c` draws.
        let special = tables.lift_special(true, p.speed == LiftSpeed::Fast);
        for line in emit_island_edges(data, lo, hi, host, sector) {
            data.linedefs[line].special = special;
            data.linedefs[line].tag = tag;
            let front = data.linedefs[line].front;
            riser.clone_into(&mut data.sidedefs[front].lower);
            // A pedestal keeps its host's ceiling, so this never fires
            // today; it is here so the rule is stated once for every
            // platform shape rather than only where it currently bites.
            unpeg_landing_upper(data, line, sector);
        }

        out.push(LiftOut {
            sector,
            shape: LiftShape::Pedestal,
            travel: p.rise,
            callable_from: vec![host],
            tag,
            portal: None,
            pedestal: Some(i),
            low_line: None,
            top_line: None,
        });
    }

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

    // Whichever of the platform's two faces has a taller neighbor draws that
    // neighbor's upper, and wants it anchored at the neighbor's own ceiling.
    for line in [seg.near_line, seg.far_line] {
        unpeg_landing_upper(data, line, plat);
    }

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
    fn emit_lifts_refuses_a_theme_the_texture_table_does_not_name() {
        // `compile` never reaches this pass with an unresolvable theme —
        // `emit_sectors` refuses it first — so the riser and trim lookups
        // are reachable only by driving the pass directly, as `compile_data`
        // already does. Every earlier pass gets the real theme; only
        // `emit_lifts` sees the bad one.
        let mut ir = Ir::from_json(LIFT).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = sectors::emit_sectors(&ir).expect("sectors");
        sectors::resolve_secret_specials(&ir, &tables, &mut data);
        portals::cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        doors::emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");
        exits::emit_exits(&ir, &tables, &mut data, &mut tags).expect("exits");
        teleports::emit_teleports(&ir, &tables, &mut data, &mut tags).expect("teleports");

        ir.theme = "no_such_theme".to_owned();
        let err = emit_lifts(&ir, &tables, &mut data, &mut tags).expect_err(
            "the theme resolves to no \
                 texture set",
        );
        assert!(
            matches!(&err, CompileError::UnknownTheme { theme } if theme == "no_such_theme"),
            "expected UnknownTheme naming the theme asked for, got {err}"
        );
    }

    #[test]
    fn a_lift_portal_emits_one_platform_at_the_high_floor_with_the_switch_on_its_low_face() {
        let Built {
            tables,
            data,
            tags,
            lifts,
        } = compile_data(LIFT);
        let riser = tables
            .texture("lift_riser", "tech_base")
            .expect("the theme names a lift riser");
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
        assert_eq!(plat.wall_tex, riser, "the theme's lift riser");
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
        assert_eq!(data.sidedefs[data.linedefs[low].front].lower, riser);
        assert_eq!(data.sidedefs[data.linedefs[top].back.unwrap()].lower, riser);
        assert_eq!(
            data.sidedefs[data.linedefs[top].front].lower, "",
            "the level room's sidedef shows nothing"
        );
        for i in [low, top] {
            assert!(
                !data.linedefs[i].lower_unpegged,
                "linedef {i}: lower-pegged, so the riser rides with the platform"
            );
        }
        // `LIFT` gives room `a` a 192 ceiling and room `b` a 256 one, so the
        // platform's is 192: the low face's two ceilings match and the top
        // face's neighbor is 64 taller and draws the upper this flag aligns.
        assert!(
            !data.linedefs[low].upper_unpegged,
            "no upper is drawn where the two ceilings are equal"
        );
        assert!(
            data.linedefs[top].upper_unpegged,
            "the landing's upper is anchored at the landing's own ceiling"
        );
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
        let riser = tables
            .texture("lift_riser", "tech_base")
            .expect("the theme names a lift riser");
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
            data.sidedefs[data.linedefs[low].front].lower, riser,
            "the alcove's sidedef carries the riser"
        );
    }

    /// `LIFT` with the two rooms' floors swapped, so room `b` is the low one
    /// and every side of the construction lands on the opposite half: the
    /// alcove is the *far* one, the platform's low face is the segment's
    /// *far* line, and `callable_from` names the far neighbor.
    ///
    /// Its own test rather than a variation on the walkover one because the
    /// `a_is_low == false` arm and the far-alcove block are otherwise
    /// unexercised — every other fixture puts room `a` on the low floor, and
    /// the two that set `alcove_far` are rejected at the depth check before
    /// reaching either.
    #[test]
    fn a_reversed_lift_with_a_far_alcove_puts_everything_on_the_other_side() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":128, "ceiling":256, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":144,
              "floor_tex":"FLAT1", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[ { "a":"a", "b":"b", "kind":"lift", "width":64, "at":[256,128],
                        "trigger":"walkover", "alcove_far":16 } ]
        }"#;
        let Built {
            tables,
            data,
            lifts,
            ..
        } = compile_data(json);
        let riser = tables
            .texture("lift_riser", "tech_base")
            .expect("the theme names a lift riser");
        let l = &lifts[0];
        // Sectors: a, b, the far alcove (the only one, so still the first
        // pushed), platform.
        assert_eq!(data.sectors.len(), 4);
        let alcove = 2;
        assert_eq!(data.sectors[alcove].floor, 0, "room b's floor, the low one");
        assert_eq!(
            l.callable_from,
            vec![alcove],
            "the far alcove, not the near one"
        );
        let outer = data
            .linedefs
            .iter()
            .position(|ld| {
                let (f, b) = (
                    data.sidedefs[ld.front].sector,
                    ld.back.map(|b| data.sidedefs[b].sector),
                );
                f == 1 && b == Some(alcove)
            })
            .expect("the alcove's outer threshold fronts room b");
        assert_eq!(
            data.linedefs[outer].special,
            tables.lift_special(false, false)
        );
        assert_eq!(data.linedefs[outer].tag, l.tag);
        let low = l.low_line.expect("a portal lift has a low face");
        let top = l.top_line.expect("and a top face");
        assert_eq!(sides(&data, low), (alcove, Some(l.sector)));
        assert_eq!(
            data.linedefs[low].special, 0,
            "nothing on the riser for a walkover lift"
        );
        assert_eq!(
            data.sidedefs[data.linedefs[low].front].lower, riser,
            "the alcove's sidedef carries the riser"
        );
        assert_eq!(
            data.sidedefs[data.linedefs[top].back.unwrap()].lower,
            riser,
            "and the platform's own sidedef the top face's"
        );
        assert_eq!(
            data.sidedefs[data.linedefs[top].front].lower, "",
            "room a, the level room, shows nothing"
        );
        for i in [low, top] {
            assert!(
                !data.linedefs[i].lower_unpegged,
                "flags clear on both faces"
            );
        }
    }

    /// The same construction with the two rooms swapped *in space*: room
    /// `a` lies EAST of room `b`, so the facing pair is room `a`'s own west
    /// wall against room `b`'s own east wall.
    ///
    /// Every other lift fixture on the branch puts room `a` west of room
    /// `b`, which pins two of `emit_portal_lift`'s geometric inputs to a
    /// single value each: [`crate::geom::FacingSpan`]'s `near` is always
    /// below its `far`, so `dir` is always `+1` and `pos1`/`pos2` always
    /// walk up from room `a`'s wall; and `geom::facing_spans` always reports
    /// `a_forward == false`, so the far alcove's outer threshold is always
    /// cut with `!a_forward == true`. Facing the other way inverts both at
    /// once — `dir == -1`, `a_forward == true` — which is what makes the
    /// signed depth a subtraction in the other direction and swaps which
    /// winding `emit_opening` gives each threshold. Room `b` is the low one
    /// here too, so the `!a_is_low` arm and the far-alcove block run against
    /// the inverted geometry rather than the familiar one.
    ///
    /// Worked example: room `a`'s own wall sits at x=256 and room `b`'s at
    /// x=192, a 64-unit gap; portal width 64 at (256,128) opens y in
    /// [96,160]; `alcove_far` 16 puts the alcove at x in [192,208] and the
    /// platform at x in [208,256] — a signed depth of
    /// `(208 - 256) * -1 == 48`, which the same expression read with the
    /// sign the other way round would make -48 and refuse.
    #[test]
    fn a_lift_whose_room_a_lies_east_of_room_b_builds_the_same_platform_mirrored() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]], "floor":128, "ceiling":256, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[ { "kind":"player1_start", "at":[384,128], "angle":0 } ] },
            { "id":"b", "footprint":[[-64,0],[-64,256],[192,256],[192,0]], "floor":0, "ceiling":192, "light":144,
              "floor_tex":"FLAT1", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[ { "a":"a", "b":"b", "kind":"lift", "width":64, "at":[256,128],
                        "trigger":"walkover", "alcove_far":16 } ]
        }"#;
        let Built {
            tables,
            data,
            lifts,
            ..
        } = compile_data(json);
        let riser = tables
            .texture("lift_riser", "tech_base")
            .expect("the theme names a lift riser");
        assert_eq!(lifts.len(), 1);
        let l = &lifts[0];
        assert_eq!((l.shape, l.travel), (LiftShape::Lift, 128));

        // Sectors: a, b, the far alcove (the only one), the platform.
        assert_eq!(data.sectors.len(), 4);
        let alcove = 2;
        assert_eq!(
            (data.sectors[alcove].floor, data.sectors[alcove].light),
            (0, 144),
            "the alcove sits at room b's floor, the low one, and borrows its light"
        );
        assert_eq!(
            l.callable_from,
            vec![alcove],
            "called from the far alcove, since room b is the low room"
        );
        let plat = &data.sectors[l.sector];
        assert_eq!(
            (plat.floor, plat.ceiling),
            (128, 192),
            "rests at room a's floor, under the lower of the two ceilings"
        );
        assert_eq!(plat.wall_tex, riser);

        // The whole point of the fixture: the platform is built *westward*
        // from room a's wall, so the alcove is the 16 units nearest room b
        // and the platform is the 48 the depth check measured.
        let x_of = |line: usize| {
            let ld = &data.linedefs[line];
            let (a, b) = (data.vertices[ld.v1], data.vertices[ld.v2]);
            assert_eq!(a.x, b.x, "linedef {line} is a vertical cut");
            a.x
        };
        let low = l.low_line.expect("a portal lift has a low face");
        let top = l.top_line.expect("and a top face");
        assert_eq!(
            (x_of(top), x_of(low)),
            (256, 208),
            "the top face is room a's own wall; the low face stops 16 short of room b's"
        );
        // Front/back: `emit_opening` binds the neighbor to the front and the
        // platform to the back on both faces, whichever way the wall winds.
        assert_eq!(
            (sides(&data, low), sides(&data, top)),
            ((alcove, Some(l.sector)), (0, Some(l.sector))),
            "the alcove fronts the low face, room a the top one"
        );

        let outer = data
            .linedefs
            .iter()
            .position(|ld| {
                let (f, b) = (
                    data.sidedefs[ld.front].sector,
                    ld.back.map(|b| data.sidedefs[b].sector),
                );
                f == 1 && b == Some(alcove)
            })
            .expect("the alcove's outer threshold fronts room b");
        assert_eq!(x_of(outer), 192, "cut at room b's own wall");
        assert_eq!(
            data.linedefs[outer].special,
            tables.lift_special(false, false),
            "the walkover special rides the outer threshold, not the riser"
        );
        assert_eq!(data.linedefs[outer].tag, l.tag);
        assert_eq!(
            data.linedefs[low].special, 0,
            "nothing on the riser for a walkover lift"
        );

        // Both risers, each on the one sidedef `r_segs.c` draws it from: the
        // alcove's on the low face, the platform's own on the top face.
        let lower = |side: usize| data.sidedefs[side].lower.as_str();
        assert_eq!(
            (
                lower(data.linedefs[low].front),
                lower(data.linedefs[top].back.expect("two-sided")),
                lower(data.linedefs[top].front),
            ),
            (riser, riser, ""),
            "room a, the level room, is the one side that shows nothing"
        );
        // Lower-pegged on both faces. The platform takes room `b`'s 192
        // ceiling, which the far alcove in front of its low face shares, so
        // the only upper drawn is room `a`'s, over the top face.
        for (i, upper) in [(low, false), (top, true)] {
            assert!(
                !data.linedefs[i].lower_unpegged && data.linedefs[i].upper_unpegged == upper,
                "linedef {i}: lower-pegged, and the upper flag follows the taller neighbor"
            );
        }
    }

    /// The upper flag follows the *ceilings*, not the floors: it is set on
    /// whichever platform face has the taller neighbor, and on neither when
    /// the ceilings match.
    ///
    /// `LIFT` itself covers the common case (the level room above is also the
    /// taller one). These two variants pin the other two: all three ceilings
    /// equal, where no upper is drawn at all; and a low room taller than the
    /// platform, where the flag lands on the *low* face — the opposite side
    /// from `LIFT`'s, which a rule written off the floors would get wrong.
    #[test]
    fn the_upper_flag_tracks_which_neighbor_has_the_taller_ceiling() {
        let flat = LIFT.replacen(
            r#""floor":128, "ceiling":256"#,
            r#""floor":128, "ceiling":192"#,
            1,
        );
        let Built { data, lifts, .. } = compile_data(&flat);
        let l = &lifts[0];
        assert_eq!(
            data.sectors[l.sector].ceiling, 192,
            "both rooms are at 192, so the platform is too"
        );
        for line in [l.low_line.unwrap(), l.top_line.unwrap()] {
            assert!(
                !data.linedefs[line].upper_unpegged,
                "linedef {line}: equal ceilings draw no upper, so nothing to unpeg"
            );
        }

        // Room `a`, the LOW room, given the tallest ceiling in the map: the
        // platform takes room `b`'s 256, so it is room `a`'s sidedef on the
        // low face that draws an upper.
        let tall_low = LIFT.replacen(
            r#""floor":0, "ceiling":192"#,
            r#""floor":0, "ceiling":320"#,
            1,
        );
        let Built { data, lifts, .. } = compile_data(&tall_low);
        let l = &lifts[0];
        assert_eq!(data.sectors[l.sector].ceiling, 256, "room b's, the lower");
        assert!(
            data.linedefs[l.low_line.unwrap()].upper_unpegged,
            "the low room is the taller side here, so its upper is the one anchored"
        );
        assert!(
            !data.linedefs[l.top_line.unwrap()].upper_unpegged,
            "the level room shares the platform's ceiling"
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
        let riser = tables
            .texture("lift_riser", "tech_base")
            .expect("the theme names a lift riser");
        let l = &lifts[0];
        assert_eq!(l.shape, LiftShape::Barrier);
        let plat = &data.sectors[l.sector];
        assert_eq!(plat.floor, 96);
        // A barrier rests flush with neither room, so it has no "level
        // room" to borrow from and falls back to room `a` — the fixture
        // gives `b` a different light (144) and flat (`FLAT1`), so these
        // would not hold if the fallback picked the other room.
        assert_eq!(plat.light, 160, "room a's light");
        assert_eq!(plat.floor_tex, "FLOOR4_8", "room a's flat");
        assert_eq!(l.travel, 96);
        assert_eq!(l.callable_from, vec![0, 1]);
        for line in [l.low_line.unwrap(), l.top_line.unwrap()] {
            assert_eq!(
                data.linedefs[line].special,
                tables.lift_special(true, false)
            );
            assert_eq!(data.linedefs[line].tag, l.tag);
            assert_eq!(data.sidedefs[data.linedefs[line].front].lower, riser);
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

    /// A 512-unit square room holding one default-size (64x64) pedestal at
    /// (128, 128), risen 128 units above the floor and carrying a soulsphere
    /// at its center.
    ///
    /// The ceiling is 256 rather than something tighter so the risen
    /// platform still clears the player: at floor 128 it leaves 128 units of
    /// headroom, and the rejection fixtures below squeeze that deliberately.
    const PEDESTAL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]], "floor":0, "ceiling":256, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[448,448], "angle":0 } ] }
      ],
      "portals":[],
      "pedestals":[ { "id":"p", "room":"a", "at":[128,128], "rise":128, "speed":"fast",
                      "things":[ { "kind":"soulsphere", "at":[160,160], "angle":0 } ] } ]
    }"#;

    #[test]
    fn a_pedestal_is_a_hosted_island_risen_above_its_room_with_the_switch_on_every_edge() {
        let Built {
            tables,
            data,
            tags,
            lifts,
        } = compile_data(PEDESTAL);
        let riser = tables
            .texture("lift_riser", "tech_base")
            .expect("the theme names a lift riser");
        assert_eq!(lifts.len(), 1);
        let l = &lifts[0];
        assert_eq!(l.shape, LiftShape::Pedestal);
        assert_eq!(
            (l.travel, l.callable_from.clone(), l.pedestal),
            (128, vec![0], Some(0)),
            "it travels its whole rise, is called from its host room, and names its pedestal"
        );
        assert_eq!(
            (l.portal, l.low_line, l.top_line),
            (None, None, None),
            "a pedestal belongs to no portal and has no single low or top face"
        );
        let s = &data.sectors[l.sector];
        assert_eq!(
            (s.floor, s.ceiling, s.host),
            (128, 256, Some(0)),
            "risen by its rise, up to the host's ceiling, and hosted by the host"
        );
        assert_eq!(s.wall_tex, riser, "the theme's lift riser");
        assert_eq!(s.tag, l.tag);
        assert_ne!(l.tag, 0);
        let edges: Vec<usize> = data
            .linedefs
            .iter()
            .enumerate()
            .filter(|(_, ld)| ld.back.is_some_and(|b| data.sidedefs[b].sector == l.sector))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(edges.len(), 4, "one edge per side of the island");
        for i in edges {
            let ld = &data.linedefs[i];
            assert_eq!(
                data.sidedefs[ld.front].sector, 0,
                "linedef {i}: the host is on the front, the side `P_UseSpecialLine` fires from"
            );
            assert_eq!(
                ld.special,
                tables.lift_special(true, true),
                "linedef {i}: the fast use special, since the fixture rides fast"
            );
            assert_eq!(ld.tag, l.tag, "linedef {i}");
            assert_eq!(
                data.sidedefs[ld.front].lower, riser,
                "linedef {i}: the host's sidedef shows the riser"
            );
            assert!(
                !ld.lower_unpegged,
                "linedef {i}: flags clear, so the riser rides with the platform"
            );
        }
        assert!(
            tags.manifest()
                .iter()
                .any(|e| e.tag == l.tag && e.purpose == "pedestal p"),
            "the tag manifest names the pedestal"
        );
    }

    /// The other half of the speed-to-special mapping. The test above pins
    /// only `lift_special(true, true)`, whose two arguments are both `true`
    /// and which a transposed argument order would therefore satisfy just as
    /// well; a pedestal that names no speed rides at the normal one, so its
    /// edges must carry `lift_special(true, false)` — a value the transposed
    /// call could not produce.
    #[test]
    fn a_pedestal_that_names_no_speed_carries_the_normal_use_special() {
        let json = PEDESTAL.replacen(r#" "speed":"fast","#, "", 1);
        let Built {
            tables,
            data,
            lifts,
            ..
        } = compile_data(&json);
        let plat = lifts[0].sector;
        let edges: Vec<usize> = data
            .linedefs
            .iter()
            .enumerate()
            .filter(|(_, ld)| ld.back.is_some_and(|b| data.sidedefs[b].sector == plat))
            .map(|(i, _)| i)
            .collect();
        assert_eq!(edges.len(), 4, "one edge per side of the island");
        for i in edges {
            assert_eq!(
                data.linedefs[i].special,
                tables.lift_special(true, false),
                "linedef {i}: the normal use special"
            );
        }
    }

    /// The accept path for an explicit [`crate::ir::Pedestal::size`]. Every
    /// other pedestal fixture either takes the 64x64 default or names a size
    /// only to be refused (`pedestal_rejections`'s 24-wide one), so nothing
    /// yet showed a *legal* explicit size reaching the emitted geometry: a
    /// `size` the compiler deserialized and then ignored would satisfy all
    /// of them.
    ///
    /// `Pedestal::rect` anchors the rectangle at `at` and adds the size, so
    /// a 96x64 pedestal at (128,128) spans (128,128)-(224,192) — deliberately
    /// non-square, since a square would not tell a transposed `[w, h]` apart.
    #[test]
    fn an_explicit_pedestal_size_becomes_the_emitted_rectangle() {
        let json = PEDESTAL.replacen(r#""rise":128,"#, r#""rise":128, "size":[96,64],"#, 1);
        assert_ne!(json, PEDESTAL, "the splice changed nothing");
        let Built { data, lifts, .. } = compile_data(&json);
        let l = &lifts[0];
        let corners: Vec<(i32, i32)> = data
            .linedefs
            .iter()
            .filter(|ld| ld.back.is_some_and(|b| data.sidedefs[b].sector == l.sector))
            .flat_map(|ld| {
                let (v1, v2) = (data.vertices[ld.v1], data.vertices[ld.v2]);
                [(v1.x, v1.y), (v2.x, v2.y)]
            })
            .collect();
        assert_eq!(corners.len(), 8, "four island edges, two vertices each");
        let (xs, ys): (Vec<i32>, Vec<i32>) = corners.into_iter().unzip();
        let span = |v: &[i32]| {
            (
                *v.iter().min().expect("the island has edges"),
                *v.iter().max().expect("the island has edges"),
            )
        };
        let (lo_x, hi_x) = span(&xs);
        let (lo_y, hi_y) = span(&ys);
        assert_eq!(
            ((lo_x, lo_y), (hi_x, hi_y)),
            ((128, 128), (224, 192)),
            "the rectangle `Pedestal::rect` names, anchored at `at`"
        );
        assert_eq!(
            (hi_x - lo_x, hi_y - lo_y),
            (96, 64),
            "96 wide and 64 deep, not the 64x64 default and not transposed"
        );
    }

    #[test]
    fn pedestal_rejections() {
        let tables = Tables::load().expect("tables");
        let low = PEDESTAL.replacen(r#""rise":128"#, r#""rise":16"#, 1);
        assert!(matches!(
            compile(&Ir::from_json(&low).expect("ir"), &tables),
            Err(CompileError::PedestalRiseTooLow {
                rise: 16,
                step: 24,
                ..
            })
        ));

        // 24 units across is narrower than the player's 32-unit diameter, so
        // they could not stand on it; the thing moves with the shrunken
        // rectangle so `Ir::from_json` still accepts the fixture.
        let small = PEDESTAL
            .replacen(r#""rise":128,"#, r#""rise":128, "size":[24,64],"#, 1)
            .replacen(r#""at":[160,160]"#, r#""at":[140,160]"#, 1);
        assert!(matches!(
            compile(&Ir::from_json(&small).expect("ir"), &tables),
            Err(CompileError::PedestalTooSmall {
                width: 24,
                min: 32,
                ..
            })
        ));

        // A 160 ceiling over a floor risen to 128 leaves 32 units, short of
        // the player's own 56.
        let squat = PEDESTAL.replacen(r#""ceiling":256"#, r#""ceiling":160"#, 1);
        assert!(matches!(
            compile(&Ir::from_json(&squat).expect("ir"), &tables),
            Err(CompileError::PedestalNoHeadroom {
                have: 32,
                need: 56,
                ..
            })
        ));
    }

    /// Every pedestal is a platform of its own, so a map can exhaust
    /// `MAXPLATS` on pedestals alone — with no portal in it at all.
    #[test]
    fn more_than_max_active_platforms_is_refused() {
        let tables = Tables::load().expect("tables");
        let max = tables.plat().max_active;
        assert_eq!(
            max, 30,
            "MAXPLATS, which the lattice below is sized against"
        );

        // A 128-unit lattice of 64-unit squares, 15 to a row, inside a
        // 2048-unit room: every pedestal clears its neighbors and the room's
        // own walls by 64 units, so nothing but the platform count is at
        // issue. Built in a loop rather than by hand — 31 hand-written
        // rectangles would be 31 chances to typo one into an overlap.
        let json = |count: usize| -> String {
            let pedestals: Vec<String> = (0..count)
                .map(|k| {
                    let (x, y) = (64 + 128 * (k % 15), 64 + 128 * (k / 15));
                    format!(r#"{{ "id":"p{k}", "room":"a", "at":[{x},{y}], "rise":64 }}"#)
                })
                .collect();
            format!(
                r#"{{ "seed":1, "grid":64, "theme":"tech_base",
                  "rooms":[
                    {{ "id":"a", "footprint":[[0,0],[0,2048],[2048,2048],[2048,0]],
                       "floor":0, "ceiling":256, "light":160,
                       "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                       "things":[ {{ "kind":"player1_start", "at":[1024,1600], "angle":0 }} ] }}
                  ],
                  "portals":[],
                  "pedestals":[ {} ] }}"#,
                pedestals.join(", ")
            )
        };

        let over = json(max + 1);
        assert!(matches!(
            compile(&Ir::from_json(&over).expect("ir"), &tables),
            Err(CompileError::TooManyPlats { count: 31, max: 30 })
        ));
        let at_max = json(max);
        assert!(
            compile(&Ir::from_json(&at_max).expect("ir"), &tables).is_ok(),
            "exactly `MAXPLATS` pedestals compile clean"
        );
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
