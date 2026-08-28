//! Emits teleport pads, tags their destination sectors, and synthesizes the
//! destination markers.
//!
//! The pad is a [`Ir::PAD_SIZE`] square whose floor sits
//! [`Ir::PAD_FLOOR_STEP`] above its host's, floored with the theme's `pad`
//! flat, every edge carrying the teleport special. It is always the trigger
//! line's *back* sector: `EV_Teleport` (pinned `p_telept.c`) returns before
//! doing anything when `side == 1`, so only a front-to-back crossing —
//! entering the pad — fires, and an arrival can walk off it. Retail id maps
//! agree on every count that matters (docs/measurements/teleports-*.md): 82
//! of 83 DOOM + DOOM2 island pads trigger on every edge, 77 of 83 are
//! exactly 64x64, and the pad is the back sector ten times out of eleven in
//! idgames.
//!
//! Two placements, one shape. An `island` pad is a new sector carved inside
//! its room — four two-sided linedefs wound counter-clockwise around the
//! square so their front (right) sidedef binds the host and their back the
//! pad; the host's own polygon is untouched, and
//! [`crate::compile::sectors::check_no_sector_overlaps`] exempts the pair
//! through [`SectorOut::host`]. A `wall` pad is `portals::emit_recess` at
//! [`Ir::PAD_SIZE`] depth, the walkover exit's alcove made square, with the
//! threshold as its one trigger edge.
//!
//! Order matters twice. Every wall pad's host wall is split *before* any pad
//! sector is emitted, because `split_wall_for_opening` removes a linedef and
//! shifts every later index — recording a trigger edge and then splitting
//! would invalidate the record. And destinations are resolved *after* every
//! pad exists, because a destination may lie on another teleport's pad (a
//! two-way pair) and must tag that pad's sector, not the room's.
//!
//! The marker is `MT_TELEPORTMAN` (`teleport_dest`, doomednum 14): in no
//! blockmap and no sector list, so it obstructs nothing. The clearance a
//! destination needs is the *arriving* thing's — the player for a pad any
//! thing may cross, the largest species in the pad's room for a
//! `monsters_only` pad — which [`crate::compile::things::place_things`]
//! enforces from the [`Marker`]s this pass returns (rule P15).

use std::collections::BTreeMap;

use crate::compile::portals::{Cut, emit_recess, split_wall_for_opening};
use crate::compile::sectors::vertex_index;
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, LinedefOut, MapData, SectorOut, SidedefOut};
use crate::geom::{Pt, wall_edges};
use crate::ir::{Ir, PadPlacement, Teleport, destination_sector_key, pad_square, square_contains};
use crate::tables::{Tables, ThingDims};

/// A synthesized `teleport_dest` thing, placed by
/// [`crate::compile::things::place_things`].
#[derive(Debug, Clone)]
pub struct Marker {
    /// The emitted sector it lands in — the one carrying the tag.
    pub sector: usize,
    /// Its position.
    pub at: Pt,
    /// Its facing; `EV_Teleport` copies it onto the arrival.
    pub angle: u16,
    /// The largest thing that can arrive here, whose radius and height the
    /// destination must clear.
    pub need: ThingDims,
    /// The ids of every teleport delivering here, in IR order.
    pub teleports: Vec<String>,
}

/// One pad, resolved and validated before anything is emitted.
struct PadPlan {
    /// The host room, which is also its sector index.
    room_idx: usize,
    /// Where the pad's geometry goes.
    kind: PadKind,
}

/// The two pad placements, each resolved to the geometry that emits it.
enum PadKind {
    /// A free-standing square: low and high corners.
    Island {
        /// The square's low corner.
        lo: Pt,
        /// The square's high corner.
        hi: Pt,
    },
    /// A recess: the wall cut and the host wall's edge direction.
    Wall {
        /// The span cut out of the host wall.
        cut: Cut,
        /// Whether the host wall runs in its axis's increasing direction.
        forward: bool,
    },
}

/// Emits every teleport. See the module docs for the order.
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` names no texture
/// set, [`CompileError::TeleportThingOnPad`] when an authored thing stands on
/// a pad square, [`CompileError::TeleportDestinationSectorTagged`] when a
/// destination sector is already tagged, and whatever
/// `portals::split_wall_for_opening` or `portals::emit_recess` raise for a
/// wall pad.
///
/// # Panics
/// Panics if a pad or destination fails to resolve, or if two teleports
/// delivering to one sector name different points — all unreachable, since
/// [`Ir::from_json`] validates each with the same geometry.
pub fn emit_teleports(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
) -> Result<Vec<Marker>, CompileError> {
    let pad_flat = tables
        .texture("pad", &ir.theme)
        .ok_or_else(|| CompileError::UnknownTheme {
            theme: ir.theme.clone(),
        })?
        .to_owned();

    let plans: Vec<PadPlan> = ir
        .teleports
        .iter()
        .map(|t| resolve_pad(ir, tables, t))
        .collect::<Result<_, _>>()?;

    // Phase 1: every wall split, so no later split shifts a recorded index.
    for plan in &plans {
        if let PadKind::Wall { cut, .. } = &plan.kind {
            split_wall_for_opening(data, cut, plan.room_idx, &ir.rooms[plan.room_idx].id)?;
        }
    }

    // Phase 2: emit every pad; remember its sector and trigger edges.
    let (pad_sector, pad_edges) = emit_pads(ir, data, &plans, &pad_flat)?;

    // Phase 3: destinations — one tag and one marker per distinct sector.
    let (markers, line_tag) = tag_destinations(ir, tables, data, tags, &pad_sector)?;

    // Phase 4: triggers.
    for (i, t) in ir.teleports.iter().enumerate() {
        let special = tables.teleport_special(t.monsters_only, t.repeatable);
        for &line in &pad_edges[i] {
            data.linedefs[line].special = special;
            data.linedefs[line].tag = line_tag[i];
        }
    }

    Ok(markers)
}

/// Emits every planned pad, returning each one's sector index and the
/// linedefs that will carry its teleport special, both in `plans` order.
///
/// Nothing here removes a linedef, so an index recorded for one pad stays
/// valid while the next is emitted — the reason every wall split happens
/// before this runs. See the module docs.
///
/// # Errors
/// Returns [`CompileError::RecessOutOfRange`] when a wall pad's far wall
/// would land outside the 16-bit map range.
fn emit_pads(
    ir: &Ir,
    data: &mut MapData,
    plans: &[PadPlan],
    pad_flat: &str,
) -> Result<(Vec<usize>, Vec<Vec<usize>>), CompileError> {
    let mut pad_sector: Vec<usize> = Vec::with_capacity(plans.len());
    let mut pad_edges: Vec<Vec<usize>> = Vec::with_capacity(plans.len());
    for plan in plans {
        let room = &ir.rooms[plan.room_idx];
        let sector_out = SectorOut {
            floor: room.floor + Ir::PAD_FLOOR_STEP,
            ceiling: room.ceiling,
            light: room.light,
            floor_tex: pad_flat.to_owned(),
            ceil_tex: room.ceil_tex.clone(),
            special: 0,
            tag: 0,
            wall_tex: room.wall_tex.clone(),
            host: match plan.kind {
                PadKind::Island { .. } => Some(plan.room_idx),
                PadKind::Wall { .. } => None,
            },
        };
        match &plan.kind {
            PadKind::Island { lo, hi } => {
                let sector = data.sectors.len();
                data.sectors.push(sector_out);
                pad_sector.push(sector);
                pad_edges.push(emit_island_edges(data, *lo, *hi, plan.room_idx, sector));
            }
            PadKind::Wall { cut, forward } => {
                let recess = emit_recess(
                    data,
                    cut,
                    plan.room_idx,
                    *forward,
                    Ir::PAD_SIZE,
                    sector_out,
                    &room.id,
                )?;
                pad_sector.push(recess.sector);
                pad_edges.push(vec![recess.threshold]);
            }
        }
    }
    Ok((pad_sector, pad_edges))
}

/// Emits an island pad's four two-sided edges, returning their indices.
///
/// The corners walk counter-clockwise (east along the south edge first), so
/// the host lies on the right of every directed edge and binds each line's
/// front sidedef — the winding a hole in a Doom sector needs, opposite the
/// clockwise winding of a room's own footprint.
fn emit_island_edges(data: &mut MapData, lo: Pt, hi: Pt, host: usize, sector: usize) -> Vec<usize> {
    let corners = [lo, Pt { x: hi.x, y: lo.y }, hi, Pt { x: lo.x, y: hi.y }];
    (0..corners.len())
        .map(|k| {
            push_two_sided(
                data,
                corners[k],
                corners[(k + 1) % corners.len()],
                host,
                sector,
            )
        })
        .collect()
}

/// Tags each distinct destination sector and builds its marker, returning
/// the markers (ordered by sector, so emission stays deterministic) and the
/// tag each teleport's trigger lines must carry, in `ir.teleports` order.
///
/// Runs only once every pad exists, since a destination may lie on another
/// teleport's pad and must tag that pad's sector rather than the room's.
///
/// # Errors
/// Returns [`CompileError::TeleportDestinationSectorTagged`] when the
/// destination sector already carries a tag from another pass.
///
/// # Panics
/// Panics if a destination fails to resolve, or if two teleports delivering
/// to one sector name different points — both unreachable, since
/// [`Ir::from_json`] rejects them with the same geometry.
fn tag_destinations(
    ir: &Ir,
    tables: &Tables,
    data: &mut MapData,
    tags: &mut TagAllocator,
    pad_sector: &[usize],
) -> Result<(Vec<Marker>, Vec<u16>), CompileError> {
    let mut by_sector: BTreeMap<usize, (u16, Marker)> = BTreeMap::new();
    let mut line_tag: Vec<u16> = vec![0; ir.teleports.len()];
    for (i, t) in ir.teleports.iter().enumerate() {
        let (room_idx, pad) =
            destination_sector_key(ir, &t.to).expect("validated in Ir::from_json");
        let sector = pad.map_or(room_idx, |p| pad_sector[p]);
        let need = arriving_dims(ir, tables, t);
        if let Some((tag, marker)) = by_sector.get_mut(&sector) {
            assert_eq!(
                (marker.at, marker.angle),
                (t.to.at, t.to.angle),
                "validated in Ir::from_json"
            );
            marker.need = larger(marker.need, need);
            marker.teleports.push(t.id.clone());
            line_tag[i] = *tag;
        } else {
            if data.sectors[sector].tag != 0 {
                return Err(CompileError::TeleportDestinationSectorTagged {
                    id: t.id.clone(),
                    sector,
                });
            }
            let tag = tags.allocate(sector, &format!("teleport {} -> {}", t.id, t.to.room));
            data.sectors[sector].tag = tag;
            line_tag[i] = tag;
            by_sector.insert(
                sector,
                (
                    tag,
                    Marker {
                        sector,
                        at: t.to.at,
                        angle: t.to.angle,
                        need,
                        teleports: vec![t.id.clone()],
                    },
                ),
            );
        }
    }
    Ok((by_sector.into_values().map(|(_, m)| m).collect(), line_tag))
}

/// Resolves one pad's geometry and rejects an authored thing standing on it.
///
/// A thing is "standing on" the pad when its own collision circle reaches
/// the square at all, not merely when its center is inside: it would then be
/// partly over a sector 8 units above the floor its room declared, which is
/// not where the author put it.
///
/// # Errors
/// Returns [`CompileError::TeleportThingOnPad`] naming the first such thing.
///
/// # Panics
/// Panics if the teleport names a room absent from `ir.rooms`, or if its pad
/// resolves to no square — both unreachable, since [`Ir::from_json`] rejects
/// them with the same geometry this uses.
fn resolve_pad(ir: &Ir, tables: &Tables, t: &Teleport) -> Result<PadPlan, CompileError> {
    let room_idx = ir
        .rooms
        .iter()
        .position(|r| r.id == t.room)
        .expect("validated in Ir::from_json");
    let room = &ir.rooms[room_idx];
    let (lo, hi) = pad_square(room, t.pad).expect("validated in Ir::from_json");
    for thing in &room.things {
        let r = tables
            .species(&thing.kind)
            .unwrap_or_else(|| tables.player())
            .radius;
        let grown_lo = Pt {
            x: lo.x - r,
            y: lo.y - r,
        };
        let grown_hi = Pt {
            x: hi.x + r,
            y: hi.y + r,
        };
        if square_contains(grown_lo, grown_hi, thing.at) {
            return Err(CompileError::TeleportThingOnPad {
                id: t.id.clone(),
                kind: thing.kind.clone(),
                x: thing.at.x,
                y: thing.at.y,
            });
        }
    }
    let kind = match t.pad {
        PadPlacement::Island(_) => PadKind::Island { lo, hi },
        PadPlacement::Wall(at) => {
            let (axis, fixed, _, _, forward) = wall_edges(&room.footprint)
                .find(|&(axis, fixed, lo, hi, _)| {
                    let (along, across) = axis.split(at);
                    across == fixed && along > lo && along < hi
                })
                .expect("validated in Ir::from_json");
            let (along, _) = axis.split(at);
            let half = Ir::PAD_SIZE / 2;
            PadKind::Wall {
                cut: Cut {
                    axis,
                    fixed,
                    open_lo: along - half,
                    open_hi: along + half,
                },
                forward,
            }
        }
    };
    Ok(PadPlan { room_idx, kind })
}

/// The largest thing that can cross `t`'s pad: the player, or for a
/// `monsters_only` pad the largest species placed in the pad's room (the
/// player's dimensions when the room holds no monster, so the destination
/// still admits something).
///
/// # Panics
/// Panics if the teleport names a room absent from `ir.rooms` — unreachable,
/// since [`Ir::from_json`] rejects that.
fn arriving_dims(ir: &Ir, tables: &Tables, t: &Teleport) -> ThingDims {
    let player = tables.player();
    if !t.monsters_only {
        return player;
    }
    let room = ir.room(&t.room).expect("validated in Ir::from_json");
    room.things
        .iter()
        .filter_map(|th| tables.species(&th.kind))
        .fold(None, |acc: Option<ThingDims>, d| {
            Some(acc.map_or(d, |a| larger(a, d)))
        })
        .unwrap_or(player)
}

/// The element-wise maximum of two dimensions.
fn larger(a: ThingDims, b: ThingDims) -> ThingDims {
    ThingDims {
        radius: a.radius.max(b.radius),
        height: a.height.max(b.height),
    }
}

/// Emits a two-sided, non-blocking linedef from `p1` to `p2`, front bound to
/// `front` and back to `back`. Returns its index.
///
/// The pad's edges use it with the host on the front, so the caller supplies
/// the points in the direction whose right-hand side is the host.
fn push_two_sided(data: &mut MapData, p1: Pt, p2: Pt, front: usize, back: usize) -> usize {
    let v1 = vertex_index(&mut data.vertices, p1);
    let v2 = vertex_index(&mut data.vertices, p2);
    let front_side = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: front,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
        x_offset: 0,
    });
    let back_side = data.sidedefs.len();
    data.sidedefs.push(SidedefOut {
        sector: back,
        upper: String::new(),
        middle: String::new(),
        lower: String::new(),
        x_offset: 0,
    });
    data.linedefs.push(LinedefOut {
        v1,
        v2,
        front: front_side,
        back: Some(back_side),
        blocking: false,
        special: 0,
        tag: 0,
        lower_unpegged: false,
        upper_unpegged: false,
        secret: false,
    });
    data.linedefs.len() - 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::tags::TagAllocator;
    use crate::compile::{
        CompileError, MapData, compile_reporting, doors, exits, portals, sectors,
    };
    use crate::geom::Pt;
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// Two rooms authored apart; `TELEPORTS` and `THINGS_B` filled per test.
    const BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[192,64], "angle":90 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":16, "ceiling":144, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ THINGS_B ] }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }],
      "teleports":[ TELEPORTS ] }"#;

    /// Runs the real pass chain up to and including this one, mirroring
    /// `compile_reporting`'s order, and finishes with the overlap check so
    /// the host/pad exemption is exercised on every fixture here.
    fn compiled(
        teleports: &str,
        things_b: &str,
    ) -> Result<(MapData, Vec<Marker>, TagAllocator), CompileError> {
        let json = BASE
            .replace("TELEPORTS", teleports)
            .replace("THINGS_B", things_b);
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = sectors::emit_sectors(&ir)?;
        portals::cut_portals(&ir, &tables, &mut data)?;
        let mut tags = TagAllocator::new();
        doors::emit_doors(&ir, &tables, &mut data, &mut tags)?;
        exits::emit_exits(&ir, &tables, &mut data, &mut tags)?;
        let markers = emit_teleports(&ir, &tables, &mut data, &mut tags)?;
        sectors::check_no_sector_overlaps(&ir, &data)?;
        Ok((data, markers, tags))
    }

    const ISLAND: &str = r#"{ "id":"t", "room":"a", "pad":{"island":[64,192]},
        "to":{"room":"b","at":[448,128],"angle":90} }"#;

    #[test]
    fn an_island_pad_is_a_hosted_sector_with_four_trigger_edges_facing_the_room() {
        let (data, markers, tags) = compiled(ISLAND, "").expect("compiles");
        let tables = Tables::load().expect("tables");
        // rooms a, b, the passage, then the pad
        assert_eq!(data.sectors.len(), 4);
        let pad = 3;
        assert_eq!(data.sectors[pad].host, Some(0));
        assert_eq!(
            data.sectors[pad].floor,
            data.sectors[0].floor + Ir::PAD_FLOOR_STEP
        );
        assert_eq!(data.sectors[pad].ceiling, 128);
        assert_eq!(data.sectors[pad].floor_tex, "GATE3");
        assert_eq!(data.sectors[pad].special, 0);
        let edges: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_some_and(|b| data.sidedefs[b].sector == pad))
            .collect();
        assert_eq!(edges.len(), 4, "four edges, pad on the back of each");
        for l in &edges {
            assert_eq!(data.sidedefs[l.front].sector, 0, "front = host room");
            assert_eq!(l.special, tables.teleport_special(false, true));
            assert_eq!(l.tag, data.sectors[1].tag, "tag names room b's sector");
            assert!(!l.blocking);
        }
        assert_ne!(
            data.sectors[1].tag, 0,
            "the destination sector carries the tag"
        );
        assert_eq!(markers.len(), 1);
        assert_eq!(
            (markers[0].sector, markers[0].at, markers[0].angle),
            (1, Pt { x: 448, y: 128 }, 90)
        );
        assert_eq!(tags.manifest().len(), 1);
        assert!(tags.manifest()[0].purpose.contains("teleport t"));
        // The pad's corners: center (64,192) +/- 32.
        let xs: Vec<i32> = edges
            .iter()
            .flat_map(|l| [data.vertices[l.v1].x, data.vertices[l.v2].x])
            .collect();
        assert_eq!(
            (xs.iter().min(), xs.iter().max()),
            (Some(&32), Some(&96)),
            "the pad square spans x = 32..96"
        );
    }

    #[test]
    fn island_edges_wind_so_the_host_is_on_the_right() {
        // The right-hand side of v1->v2 must face the host: for the edge
        // along the pad's south side (y = 160) walking +x, the right side
        // is -y, which is the host — so that edge runs from x=32 to x=96.
        let (data, _, _) = compiled(ISLAND, "").expect("compiles");
        let south = data
            .linedefs
            .iter()
            .find(|l| data.vertices[l.v1].y == 160 && data.vertices[l.v2].y == 160)
            .expect("south edge");
        assert!(
            data.vertices[south.v1].x < data.vertices[south.v2].x,
            "counter-clockwise around the pad"
        );
    }

    #[test]
    fn a_wall_pad_is_a_64_deep_recess_whose_threshold_triggers() {
        let (data, _, _) = compiled(
            r#"{ "id":"w", "room":"a", "pad":{"wall":[64,256]},
                 "to":{"room":"b","at":[448,128],"angle":90}, "repeatable":false }"#,
            "",
        )
        .expect("compiles");
        let tables = Tables::load().expect("tables");
        let pad = 3;
        assert_eq!(
            data.sectors[pad].host, None,
            "a recess is not inside its host"
        );
        assert_eq!(data.sectors[pad].floor, Ir::PAD_FLOOR_STEP);
        let triggers: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(triggers.len(), 1, "one threshold");
        assert_eq!(
            triggers[0].special,
            tables.teleport_special(false, false),
            "one-shot"
        );
        assert_eq!(data.sidedefs[triggers[0].front].sector, 0);
        assert_eq!(
            data.sidedefs[triggers[0].back.expect("two-sided")].sector,
            pad
        );
        assert_eq!(
            data.vertices.iter().map(|v| v.y).max(),
            Some(256 + Ir::PAD_SIZE)
        );
    }

    #[test]
    fn monsters_only_pads_take_the_largest_species_as_the_arriving_thing() {
        let (_, markers, _) = compiled(
            r#"{ "id":"m", "room":"b", "pad":{"island":[448,192]},
                 "to":{"room":"a","at":[128,128],"angle":0}, "monsters_only":true }"#,
            r#"{ "kind":"imp", "at":[384,64], "angle":0 },
               { "kind":"pinky", "at":[512,64], "angle":0 }"#,
        )
        .expect("compiles");
        let tables = Tables::load().expect("tables");
        let pinky = tables.species("pinky").expect("pinky dims");
        assert_eq!(
            (markers[0].need.radius, markers[0].need.height),
            (pinky.radius, pinky.height)
        );
    }

    #[test]
    fn identical_destinations_share_one_tag_and_marker() {
        let (data, markers, tags) = compiled(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]},
                 "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"a", "pad":{"island":[192,192]},
                 "to":{"room":"b","at":[448,128],"angle":90} }"#,
            "",
        )
        .expect("compiles");
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].teleports, vec!["t1".to_owned(), "t2".to_owned()]);
        assert_eq!(tags.manifest().len(), 1);
        assert!(
            data.linedefs
                .iter()
                .filter(|l| l.special != 0)
                .all(|l| l.tag == data.sectors[1].tag)
        );
    }

    #[test]
    fn a_two_way_pair_tags_the_other_pad() {
        let (data, markers, _) = compiled(
            r#"{ "id":"t1", "room":"a", "pad":{"island":[64,192]},
                 "to":{"room":"b","at":[448,128],"angle":90} },
               { "id":"t2", "room":"b", "pad":{"island":[448,128]},
                 "to":{"room":"a","at":[64,192],"angle":0} }"#,
            "",
        )
        .expect("compiles");
        let (pad_a, pad_b) = (3, 4);
        assert_ne!(data.sectors[pad_a].tag, 0);
        assert_ne!(data.sectors[pad_b].tag, 0);
        assert_eq!(data.sectors[1].tag, 0, "room b itself is not a destination");
        assert!(
            markers.iter().any(|m| m.sector == pad_a) && markers.iter().any(|m| m.sector == pad_b)
        );
    }

    #[test]
    fn a_thing_standing_on_a_pad_is_rejected() {
        let err = compiled(
            r#"{ "id":"m", "room":"b", "pad":{"island":[448,192]},
                 "to":{"room":"a","at":[128,128],"angle":0} }"#,
            r#"{ "kind":"imp", "at":[448,192], "angle":0 }"#,
        )
        .expect_err("a thing on the pad square must be rejected");
        assert!(
            matches!(err, CompileError::TeleportThingOnPad { .. }),
            "{err}"
        );
    }

    #[test]
    fn an_island_pad_in_a_room_does_not_trip_the_overlap_check() {
        compiled(ISLAND, "").expect("the host/pad pair is exempt");
    }

    #[test]
    fn the_pads_step_gets_its_riser_texture_and_the_map_breaks_no_rule() {
        // This pass deliberately writes no lower texture of its own: the
        // pad's floor sits `PAD_FLOOR_STEP` above the host's, and
        // `heights::apply_height_textures` — which runs after it — fills the
        // riser on the one side `r_segs.c` draws, the lower-floored host's.
        // Run through the whole chain, so P8 (no missing texture) is the
        // arbiter rather than this test's own reading of the rule.
        let json = BASE.replace("TELEPORTS", ISLAND).replace("THINGS_B", "");
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, violations) = compile_reporting(&ir, &tables).expect("compiles");
        assert!(violations.is_empty(), "{violations:?}");

        let pad = 3;
        let edge = out
            .data
            .linedefs
            .iter()
            .find(|l| l.back.is_some_and(|b| out.data.sidedefs[b].sector == pad))
            .expect("a pad edge");
        assert_eq!(
            out.data.sidedefs[edge.front].lower, "STARTAN3",
            "the host side, whose floor is the lower one, carries the riser"
        );
        assert!(
            out.data.sidedefs[edge.back.expect("two-sided")]
                .lower
                .is_empty(),
            "the side the engine never draws stays bare"
        );
    }
}
