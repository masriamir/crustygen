//! Places things with real clearance, not merely inside their room.

use crate::compile::lifts::LiftOut;
use crate::compile::teleports::Marker;
use crate::compile::{CompileError, MapData};
use crate::geom::{Pt, contains, dist_to_segment};
use crate::ir::{Ir, ThingSkills, square_contains};
use crate::tables::{Tables, ThingDims};

/// A thing as it will be emitted.
#[derive(Debug, Clone, Copy)]
pub struct ThingOut {
    /// X coordinate in map units.
    pub x: i32,
    /// Y coordinate in map units.
    pub y: i32,
    /// Facing angle in degrees.
    pub angle: u16,
    /// Concrete Doom thing ID.
    pub kind: u16,
    /// Which skill levels this thing appears on.
    pub skills: ThingSkills,
    /// `MTF_AMBUSH`: wakes on sight only.
    pub ambush: bool,
}

/// The minimum distance from `p` to any emitted linedef bordering `sector`,
/// in map units.
///
/// [`crate::compile::sectors::emit_sectors`] pushes exactly one sector per
/// room, in `ir.rooms` order, so a room's index doubles as its sector index
/// — verified directly against that module rather than assumed. Every
/// linedef whose front or back sidedef names that sector is a real wall the
/// player can collide with in the *emitted* geometry. Rooms are authored
/// apart and neither room's own footprint is ever reshaped by portal or door
/// construction, so this ordinarily agrees exactly with `room.footprint`;
/// gathering from `data` instead keeps that guarantee automatic rather than
/// assumed, and would still pick up any future pass that *does* reshape a
/// room's own boundary, without this function needing to know what changed.
///
/// Returns `None` when the sector has no bordering linedef at all. An
/// unbounded room cannot be measured, and folding to `f64::INFINITY` instead
/// would have silently *passed* every clearance check — the failure mode
/// where a broken room looks safest.
///
/// `pub(crate)` and taking a sector rather than a room, because a teleport
/// destination marker can land in a compiler-generated sector — the other
/// pad of a two-way pair — which is no room and so has no room index.
pub(crate) fn emitted_clearance(data: &MapData, sector: usize, p: Pt) -> Option<f64> {
    data.linedefs
        .iter()
        .filter(|line| {
            data.sidedefs[line.front].sector == sector
                || line
                    .back
                    .is_some_and(|back| data.sidedefs[back].sector == sector)
        })
        .map(|line| dist_to_segment(p, data.vertices[line.v1], data.vertices[line.v2]))
        .reduce(f64::min)
}

/// Places every thing, proving it fits where it stands.
///
/// Containment is checked against the room's IR-declared footprint via
/// [`contains`]. That alone cannot reject a thing sitting exactly on a wall
/// — `contains` returns `true` for a point on the polygon boundary, it is
/// not a strict interior test — but the clearance check below catches that
/// case instead, since a wall-hugging point has at most zero real clearance.
///
/// Clearance and headroom are checked against the *emitted* geometry in
/// `data`, not the declared footprint — see `emitted_clearance`'s doc
/// comment for why the two ordinarily agree exactly now that rooms are
/// authored apart, and why measuring the emitted geometry is still the
/// principled choice rather than an incidental one.
///
/// A room thing standing on one of its room's pedestals is refused rather
/// than placed: the engine would spawn it in the pedestal's sector at the
/// raised floor, which is not the floor the room's `things` list describes.
/// After every room's own things come the pedestals' own — see
/// `place_pedestal_things`. After every authored thing, each of `markers` —
/// [`crate::compile::teleports::emit_teleports`]'s destination markers — is
/// emitted as a `teleport_dest` thing, held to the clearance and headroom
/// its *arriving* thing needs rather than the marker's own (the marker is in
/// no blockmap and obstructs nothing). That is rule P15, and it is checked
/// here rather than in the teleport pass because this is where clearance is
/// measured against the finished geometry.
///
/// # Errors
/// Returns [`CompileError::UnknownThing`] for an unresolvable name,
/// [`CompileError::ThingOutsideRoom`] for a point outside its room,
/// [`CompileError::ThingOnPedestal`] for a room thing standing on one of its
/// room's pedestals, [`CompileError::ThingTooClose`] when radius clearance
/// fails,
/// [`CompileError::PedestalNoHeadroom`] when a pedestal's raised floor
/// leaves its own thing too little room under the host's ceiling,
/// [`CompileError::NoHeadroom`] when the room is too short — for the
/// tallest placed thing, or for the player if the room is empty or shorter
/// than the player alone — [`CompileError::UnboundedRoom`] when a room's
/// sector has no emitted linedef to measure against,
/// [`CompileError::OverlappingStarts`] for coincident player starts —
/// counted across room things and pedestal things alike, since a start on a
/// pedestal is still a start —
/// [`CompileError::TeleportMarkerTooClose`] when a destination is closer to
/// its sector's walls than the arriving thing's radius, and
/// [`CompileError::TeleportMarkerNoHeadroom`] when that sector is too short
/// for it.
///
/// `lifts` carries the emitted platforms, and is read for the sector each
/// pedestal was cut into.
///
/// # Panics
/// Panics only where `place_pedestal_things` and `place_markers` document:
/// on a missing `teleport_dest` vocabulary entry, which `tables`'s own
/// `exit_lift_teleport_and_sector_specials_resolve` test pins present, and
/// on geometry or IR shapes an earlier pass has already ruled out.
pub fn place_things(
    ir: &Ir,
    tables: &Tables,
    data: &MapData,
    markers: &[Marker],
    lifts: &[LiftOut],
) -> Result<Vec<ThingOut>, CompileError> {
    let mut out = Vec::new();
    let mut starts: Vec<(i32, i32)> = Vec::new();
    let player = tables.player();

    for (room_idx, room) in ir.rooms.iter().enumerate() {
        let headroom = room.ceiling - room.floor;

        // P2's stated scope is "the player is always in that set" — a
        // walkable room must admit the player regardless of what else it
        // contains. Checked here, once per room, rather than only inside the
        // per-thing loop below: a room with no `things` at all previously
        // skipped this rule entirely, so an empty corridor too short for the
        // player to stand in compiled clean (see `KNOWN-GAPS.md`).
        if headroom < player.height {
            return Err(CompileError::NoHeadroom {
                room: room.id.clone(),
                kind: "player".to_owned(),
                have: headroom,
                need: player.height,
            });
        }

        for thing in &room.things {
            let id = tables
                .thing_id(&thing.kind)
                .ok_or_else(|| CompileError::UnknownThing {
                    room: room.id.clone(),
                    kind: thing.kind.clone(),
                })?;

            // The player's dimensions are the floor for anything not listed as
            // a monster species — pickups are small, but the player must still
            // be able to stand where one sits to collect it.
            let dims: ThingDims = tables
                .species(&thing.kind)
                .unwrap_or_else(|| tables.player());

            if !contains(&room.footprint, thing.at) {
                return Err(CompileError::ThingOutsideRoom {
                    room: room.id.clone(),
                    kind: thing.kind.clone(),
                    x: thing.at.x,
                    y: thing.at.y,
                });
            }

            // A thing on a pedestal's rectangle would spawn in the
            // pedestal's sector at its raised floor, not on the room floor
            // this list describes — the refusal `teleports::resolve_pad`
            // makes for a pad, on the same grounds. Checked before clearance
            // so the author is told *why* the point is wrong: measured
            // against the room, such a point is also within a radius of the
            // island's edges, and `ThingTooClose` would name the symptom
            // rather than the cause. Closed on both axes: a point on the
            // rectangle's edge already sits on the platform's boundary line.
            if let Some(pedestal) = ir
                .pedestals
                .iter()
                .find(|p| p.room == room.id && square_contains(p.rect().0, p.rect().1, thing.at))
            {
                return Err(CompileError::ThingOnPedestal {
                    pedestal: pedestal.id.clone(),
                    kind: thing.kind.clone(),
                    x: thing.at.x,
                    y: thing.at.y,
                });
            }

            let have = emitted_clearance(data, room_idx, thing.at).ok_or_else(|| {
                CompileError::UnboundedRoom {
                    room: room.id.clone(),
                }
            })?;
            if have < f64::from(dims.radius) {
                return Err(CompileError::ThingTooClose {
                    room: room.id.clone(),
                    kind: thing.kind.clone(),
                    x: thing.at.x,
                    y: thing.at.y,
                    have,
                    need: dims.radius,
                });
            }

            if headroom < dims.height {
                return Err(CompileError::NoHeadroom {
                    room: room.id.clone(),
                    kind: thing.kind.clone(),
                    have: headroom,
                    need: dims.height,
                });
            }

            if thing.kind.ends_with("_start") {
                if starts.contains(&(thing.at.x, thing.at.y)) {
                    return Err(CompileError::OverlappingStarts {
                        x: thing.at.x,
                        y: thing.at.y,
                    });
                }
                starts.push((thing.at.x, thing.at.y));
            }

            out.push(ThingOut {
                x: thing.at.x,
                y: thing.at.y,
                angle: thing.angle,
                kind: id,
                skills: thing.skills,
                ambush: thing.ambush,
            });
        }
    }

    out.extend(place_pedestal_things(ir, tables, data, lifts, &mut starts)?);
    out.extend(place_markers(tables, data, markers)?);
    Ok(out)
}

/// Places every thing standing on a pedestal, proving it fits up there.
///
/// A pedestal's things sit on the platform, not on the host room's floor, so
/// both measurements move with it. Clearance is taken against the platform's
/// own four edges — `lifts` names the sector `emit_lifts` cut — because
/// stepping off an island is a fall the author did not ask for; headroom is
/// what the rise left under the host's ceiling, which the platform keeps.
///
/// Containment needs no check here:
/// [`Ir::from_json`](crate::ir::Ir::from_json) already required every one of
/// these to sit strictly inside its pedestal's rectangle. A `_start` is
/// legal on a pedestal — beginning the level on one is a fine thing to
/// author — but it joins `starts`, the coincident-start ledger `place_things`
/// keeps for room things, so two starts at one point are refused wherever
/// they were authored. `starts` arrives already carrying every room start,
/// since the room loop runs first.
///
/// # Errors
/// Returns [`CompileError::UnknownThing`] for an unresolvable name,
/// [`CompileError::ThingTooClose`] when the thing stands closer to the
/// pedestal's edge than its own radius,
/// [`CompileError::PedestalNoHeadroom`] when the risen floor leaves it less
/// than its own height — `emit_lifts` already held that gap to the
/// *player's* height, so only a thing taller than the player can fail that
/// one — and [`CompileError::OverlappingStarts`] for a start sharing a point
/// with one already placed.
///
/// # Panics
/// Panics if a pedestal has no emitted platform, or names a room that does
/// not exist — [`crate::compile::lifts::emit_lifts`] emits one platform per
/// pedestal and [`Ir::from_json`](crate::ir::Ir::from_json) resolves every
/// `room` before either runs — or if a pedestal's sector has no bordering
/// linedef, impossible for a square cut as four two-sided edges.
fn place_pedestal_things(
    ir: &Ir,
    tables: &Tables,
    data: &MapData,
    lifts: &[LiftOut],
    starts: &mut Vec<(i32, i32)>,
) -> Result<Vec<ThingOut>, CompileError> {
    let mut out = Vec::new();
    for (pi, pedestal) in ir.pedestals.iter().enumerate() {
        let lift = lifts
            .iter()
            .find(|l| l.pedestal == Some(pi))
            .expect("emit_lifts emits one platform per pedestal");
        let host = ir.room(&pedestal.room).expect("validated by Ir::from_json");
        let headroom = host.ceiling - (host.floor + pedestal.rise);
        for thing in &pedestal.things {
            let id = tables
                .thing_id(&thing.kind)
                .ok_or_else(|| CompileError::UnknownThing {
                    room: pedestal.room.clone(),
                    kind: thing.kind.clone(),
                })?;

            // The player's dimensions are the floor for anything not listed
            // as a monster species, exactly as in `place_things`.
            let dims: ThingDims = tables
                .species(&thing.kind)
                .unwrap_or_else(|| tables.player());

            let have = emitted_clearance(data, lift.sector, thing.at)
                .expect("a pedestal is cut as four two-sided edges");
            if have < f64::from(dims.radius) {
                return Err(CompileError::ThingTooClose {
                    room: pedestal.room.clone(),
                    kind: thing.kind.clone(),
                    x: thing.at.x,
                    y: thing.at.y,
                    have,
                    need: dims.radius,
                });
            }

            if headroom < dims.height {
                return Err(CompileError::PedestalNoHeadroom {
                    pedestal: pedestal.id.clone(),
                    kind: thing.kind.clone(),
                    have: headroom,
                    need: dims.height,
                });
            }

            // Raised height is no exemption from telefragging: two starts at
            // one point spawn one player inside the other whatever they are
            // standing on.
            if thing.kind.ends_with("_start") {
                if starts.contains(&(thing.at.x, thing.at.y)) {
                    return Err(CompileError::OverlappingStarts {
                        x: thing.at.x,
                        y: thing.at.y,
                    });
                }
                starts.push((thing.at.x, thing.at.y));
            }

            out.push(ThingOut {
                x: thing.at.x,
                y: thing.at.y,
                angle: thing.angle,
                kind: id,
                skills: thing.skills,
                ambush: thing.ambush,
            });
        }
    }
    Ok(out)
}

/// Emits one `teleport_dest` thing per destination marker, proving each
/// destination clears the thing that will arrive on it (rule P15).
///
/// The marker itself is `MF_NOBLOCKMAP|MF_NOSECTOR` and obstructs nothing,
/// so its own nominal dimensions are irrelevant; what must fit is
/// [`Marker::need`], the largest thing any teleport delivering here can
/// send. Clearance is measured against the marker's *sector*, which for the
/// far half of a two-way pair is another pad rather than a room.
///
/// [`emitted_clearance`] counts **every** linedef bordering that sector, an
/// open threshold included, while the verifier's V-P15
/// (`check::invariants::check_teleport_pairing`) measures only the
/// destination's non-passable segments — a doorway cannot crush an arrival,
/// so it does not deny the radius. The compile side is therefore strictly
/// stricter, which is the safe direction for the pass that decides what gets
/// written: nothing this accepts can fail V-P15 for clearance.
///
/// # Errors
/// Returns [`CompileError::TeleportMarkerTooClose`] when the marker stands
/// closer to its sector's walls than the arriving thing's radius, and
/// [`CompileError::TeleportMarkerNoHeadroom`] when the sector is too short
/// for it.
///
/// # Panics
/// Panics if `teleport_dest` is absent from the vocabulary table, or if a
/// marker's sector has no bordering linedef at all — impossible, since every
/// destination sector is one `compile::sectors` or `compile::teleports`
/// emitted as a closed polygon, and neither can emit a sector without
/// pushing its own boundary linedefs.
fn place_markers(
    tables: &Tables,
    data: &MapData,
    markers: &[Marker],
) -> Result<Vec<ThingOut>, CompileError> {
    let marker_kind = tables
        .thing_id("teleport_dest")
        .expect("`teleport_dest` is in the vocabulary");
    let mut out = Vec::with_capacity(markers.len());
    for m in markers {
        let id = m.teleports.join(", ");
        let have = emitted_clearance(data, m.sector, m.at)
            .expect("every emitted sector is closed, so it has bordering linedefs");
        if have < f64::from(m.need.radius) {
            return Err(CompileError::TeleportMarkerTooClose {
                id,
                x: m.at.x,
                y: m.at.y,
                have,
                need: m.need.radius,
            });
        }
        let sector = &data.sectors[m.sector];
        let headroom = sector.ceiling - sector.floor;
        if headroom < m.need.height {
            return Err(CompileError::TeleportMarkerNoHeadroom {
                id,
                have: headroom,
                need: m.need.height,
            });
        }
        out.push(ThingOut {
            x: m.at.x,
            y: m.at.y,
            angle: m.angle,
            kind: marker_kind,
            skills: ThingSkills::default(),
            ambush: false,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::compile::CompileError;
    use crate::compile::MapData;
    use crate::compile::doors::emit_doors;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::tags::TagAllocator;
    use crate::compile::things::place_things;
    use crate::compile::{compile, compile_reporting};
    use crate::geom::{Pt, clearance};
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// Runs the full geometry pipeline (sectors -> portals -> doors) so
    /// tests exercise `place_things` against real emitted geometry, matching
    /// what the eventual `compile` orchestrator hands it.
    fn compiled_data(ir: &Ir, tables: &Tables) -> MapData {
        let mut data = emit_sectors(ir).expect("sectors");
        cut_portals(ir, tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        emit_doors(ir, tables, &mut data, &mut tags).expect("doors");
        data
    }

    fn ir_with_thing(kind: &str, at: (i32, i32), ceiling: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[{{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                "floor":0, "ceiling":{ceiling}, "light":160,
                "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
                "things":[{{ "kind":"{kind}", "at":[{},{}], "angle":90 }}] }}],
              "portals":[] }}"#,
            at.0, at.1
        )
    }

    #[test]
    fn a_centered_thing_resolves_to_its_vocabulary_id() {
        let ir = Ir::from_json(&ir_with_thing("player1_start", (128, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        let things = place_things(&ir, &tables, &data, &[], &[]).expect("placed");
        assert_eq!(things.len(), 1);
        assert_eq!(things[0].kind, 1, "player 1 start");
        assert_eq!(things[0].x, 128);
    }

    #[test]
    fn a_thing_outside_its_room_is_rejected() {
        let ir = Ir::from_json(&ir_with_thing("player1_start", (512, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        assert!(matches!(
            place_things(&ir, &tables, &data, &[], &[]),
            Err(CompileError::ThingOutsideRoom { .. })
        ));
    }

    #[test]
    fn clearance_is_boundary_pinned_to_the_player_radius() {
        let tables = Tables::load().expect("tables");
        let r = tables.player().radius;
        // Standing exactly its radius from the x = 0 wall: fits.
        let ok = Ir::from_json(&ir_with_thing("player1_start", (r, 128), 128)).expect("ir");
        let data_ok = compiled_data(&ok, &tables);
        assert!(
            place_things(&ok, &tables, &data_ok, &[], &[]).is_ok(),
            "at the radius it fits"
        );
        // One unit closer: it does not.
        let bad = Ir::from_json(&ir_with_thing("player1_start", (r - 1, 128), 128)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(matches!(
            place_things(&bad, &tables, &data_bad, &[], &[]),
            Err(CompileError::ThingTooClose { .. })
        ));
    }

    /// An octagon: a 256-unit square chamfered by 64 units at each corner —
    /// the same shape as `sectors::tests::OCTAGON`,
    /// `portals::tests::OCTAGON_ROOM`, and `exits::tests::OCTAGON_ROOM`. The
    /// NW chamfer runs (0,192)-(64,256), on the line `x - y + 192 = 0`
    /// (positive on the interior side, matching the center (128,128): 192).
    /// A point offset `(k, -k)` from that edge's midpoint (32,224) lands at
    /// perpendicular distance `k * sqrt(2)` from it (the offset direction is
    /// exactly the line's normal), which is irrational for any nonzero
    /// integer `k` — unlike the axis-aligned case above, a diagonal wall
    /// cannot be pinned to an *exact* integer-radius boundary the way
    /// `clearance_is_boundary_pinned_to_the_player_radius` pins one; the
    /// nearest achievable pair of integer points that still cleanly
    /// straddles the player's actual radius is used instead.
    fn ir_with_thing_near_octagon_chamfer(k: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[{{ "id":"a",
                "footprint":[[0,64],[0,192],[64,256],[192,256],[256,192],[256,64],[192,0],[64,0]],
                "floor":0, "ceiling":128, "light":160,
                "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
                "things":[{{ "kind":"player1_start", "at":[{},{}], "angle":90 }}] }}],
              "portals":[] }}"#,
            32 + k,
            224 - k
        )
    }

    #[test]
    fn a_thing_too_close_to_a_diagonal_wall_is_rejected() {
        let tables = Tables::load().expect("tables");
        let r = tables.player().radius;
        assert_eq!(r, 16, "the fixture's k values below assume this radius");
        // k = 8 -> perpendicular distance 8*sqrt(2) ~ 11.3, well inside the
        // player's own 16-unit radius, and near no other wall of the
        // octagon (every other edge is at least 40+ units away from here).
        let ir = Ir::from_json(&ir_with_thing_near_octagon_chamfer(8)).expect("ir");
        let data = compiled_data(&ir, &tables);
        assert!(
            matches!(
                place_things(&ir, &tables, &data, &[], &[]),
                Err(CompileError::ThingTooClose { .. })
            ),
            "a point 8*sqrt(2) ~ 11.3 units from the diagonal chamfer must be rejected \
             against a 16-unit radius"
        );
    }

    #[test]
    fn a_thing_clear_of_a_diagonal_wall_by_more_than_its_radius_is_accepted() {
        let tables = Tables::load().expect("tables");
        let r = tables.player().radius;
        assert_eq!(r, 16, "the fixture's k values below assume this radius");
        // k = 12 -> perpendicular distance 12*sqrt(2) ~ 17.0, just past the
        // 16-unit radius — close enough to the diagonal chamfer that an
        // implementation measuring against the wrong (or no) edge would be
        // exposed by moving k down to 8 in the sibling test above, but far
        // enough to genuinely fit.
        let ir = Ir::from_json(&ir_with_thing_near_octagon_chamfer(12)).expect("ir");
        let data = compiled_data(&ir, &tables);
        assert!(
            place_things(&ir, &tables, &data, &[], &[]).is_ok(),
            "a point 12*sqrt(2) ~ 17.0 units from the diagonal chamfer must fit against a \
             16-unit radius"
        );
    }

    #[test]
    fn headroom_is_boundary_pinned_to_the_thing_height() {
        let tables = Tables::load().expect("tables");
        let h = tables.player().height;
        let ok = Ir::from_json(&ir_with_thing("player1_start", (128, 128), h)).expect("ir");
        let data_ok = compiled_data(&ok, &tables);
        assert!(
            place_things(&ok, &tables, &data_ok, &[], &[]).is_ok(),
            "at exactly its height it fits"
        );
        let bad = Ir::from_json(&ir_with_thing("player1_start", (128, 128), h - 1)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(matches!(
            place_things(&bad, &tables, &data_bad, &[], &[]),
            Err(CompileError::NoHeadroom { .. })
        ));
    }

    #[test]
    fn an_unknown_vocabulary_name_is_rejected() {
        let ir = Ir::from_json(&ir_with_thing("nonesuch", (128, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        assert!(matches!(
            place_things(&ir, &tables, &data, &[], &[]),
            Err(CompileError::UnknownThing { .. })
        ));
    }

    #[test]
    fn two_player_starts_at_the_same_point_are_rejected() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
            "floor":0, "ceiling":128, "light":160,
            "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
            "things":[
              { "kind":"player1_start", "at":[128,128], "angle":90 },
              { "kind":"player1_start", "at":[128,128], "angle":180 }
            ] }],
          "portals":[] }"#;
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        assert!(matches!(
            place_things(&ir, &tables, &data, &[], &[]),
            Err(CompileError::OverlappingStarts { .. })
        ));
    }

    #[test]
    fn a_room_with_no_emitted_geometry_is_rejected_rather_than_passing() {
        // Folding an empty set of bounding linedefs to `f64::INFINITY` made
        // an unbounded room pass *every* clearance check — the failure mode
        // where the most broken input looks safest. It was unreachable while
        // the only wall-removing pass required an exact full-span match; the
        // splitting pass makes wall removal routine, so the fallback had to
        // become an error.
        let ir = Ir::from_json(&ir_with_thing("player1_start", (128, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        assert!(matches!(
            place_things(&ir, &tables, &MapData::default(), &[], &[]),
            Err(CompileError::UnboundedRoom { .. })
        ));
    }

    #[test]
    fn clearance_near_a_door_matches_room_bs_own_declared_wall_now() {
        // Rooms are authored apart, so a door's own sector fills the wall
        // gap without touching either room's declared footprint — unlike
        // the old carve-into-`b` design, where a door recess pushed the
        // *emitted* wall out past the footprint and the two silently
        // disagreed. This pins that they now agree exactly: a thing at
        // exactly the player's radius from room b's own wall (x = 320, a
        // legal 64-unit gap from room a's wall at x = 256) fits; one unit
        // closer does not.
        let tables = Tables::load().expect("tables");
        let r = tables.player().radius;
        let wall = 320;

        let ir_json = |x: i32| -> String {
            format!(
                r#"{{ "seed":1, "grid":64, "theme":"tech_base",
                  "rooms":[
                    {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                       "floor":0, "ceiling":128, "light":160,
                       "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                    {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                       "floor":0, "ceiling":128, "light":160,
                       "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
                       "things":[{{ "kind":"player1_start", "at":[{x},128], "angle":90 }}] }}
                  ],
                  "portals":[{{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                                "door_thickness":32, "alcove_near":16, "alcove_far":16 }}] }}"#
            )
        };

        let ok = Ir::from_json(&ir_json(wall + r)).expect("ir");
        let data_ok = compiled_data(&ok, &tables);
        assert!(
            place_things(&ok, &tables, &data_ok, &[], &[]).is_ok(),
            "exactly the radius from room b's own wall fits"
        );

        let bad = Ir::from_json(&ir_json(wall + r - 1)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(
            matches!(
                place_things(&bad, &tables, &data_bad, &[], &[]),
                Err(CompileError::ThingTooClose { .. })
            ),
            "one unit closer than the radius does not fit"
        );

        // The emitted-geometry clearance and the declared-footprint
        // clearance now agree exactly at the rejected point — there is no
        // remaining divergence for a door portal to hide, unlike the old
        // carve-into-b design this replaces (which this same test used to
        // pin the *opposite* of).
        let footprint_clearance = clearance(
            &bad.rooms[1].footprint,
            Pt {
                x: wall + r - 1,
                y: 128,
            },
        );
        assert!(
            footprint_clearance < f64::from(r),
            "footprint-only clearance ({footprint_clearance}) must report the same violation \
             the emitted geometry does, now that neither is stale relative to the other"
        );
    }

    /// A 512x64 corridor with no things at all — a shape distinct from every
    /// other fixture in this file (all 256-squares carrying an explicit
    /// thing), and specifically a room P2's blanket check must still cover
    /// even though nothing is ever placed inside it.
    fn empty_corridor_json(ceiling: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[{{ "id":"corridor", "footprint":[[0,0],[0,64],[512,64],[512,0]],
                "floor":0, "ceiling":{ceiling}, "light":160,
                "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W", "things":[] }}],
              "portals":[] }}"#
        )
    }

    #[test]
    fn p2_an_empty_room_too_short_for_the_player_is_rejected() {
        // Before this fix, `NoHeadroom` only fired inside the per-thing
        // loop, so a room with no `things` at all skipped the check
        // entirely — an empty corridor too short to stand in compiled
        // clean. See `KNOWN-GAPS.md`'s former "P2 is only partially
        // covered" note.
        let tables = Tables::load().expect("tables");
        let h = tables.player().height;

        let ok = Ir::from_json(&empty_corridor_json(h)).expect("ir");
        let data_ok = compiled_data(&ok, &tables);
        assert!(
            place_things(&ok, &tables, &data_ok, &[], &[]).is_ok(),
            "at exactly the player's height, an empty room is fine"
        );

        let bad = Ir::from_json(&empty_corridor_json(h - 1)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(
            matches!(
                place_things(&bad, &tables, &data_bad, &[], &[]),
                Err(CompileError::NoHeadroom { .. })
            ),
            "an empty room one unit too short must still be rejected"
        );
    }

    /// Two rooms, an island teleport pad in `a` delivering into `b`, and an
    /// authored ambush imp in `b` — the smallest fixture that exercises both
    /// halves of this pass's new work at once (a synthesized marker and an
    /// authored `MTF_AMBUSH` flag). Run through `compile_reporting`, which is
    /// the only way to get the teleport pass's markers into `place_things`.
    const TELEPORT_MAP: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[{ "kind":"player1_start", "at":[192,64], "angle":90 }] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":16, "ceiling":144, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[{ "kind":"imp", "at":[384,64], "angle":0, "ambush":true }] }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }],
      "teleports":[{ "id":"t", "room":"a", "pad":{"island":[64,128]},
                     "to":{"room":"b","at":[448,128],"angle":90} }] }"#;

    #[test]
    fn a_marker_is_placed_with_the_arriving_things_clearance_and_no_ambush() {
        let ir = Ir::from_json(TELEPORT_MAP).expect("ir");
        let tables = Tables::load().expect("tables");
        let marker_kind = tables.thing_id("teleport_dest").expect("teleport_dest");
        let (out, _) = compile_reporting(&ir, &tables).expect("compiles");

        let marker = out
            .things
            .iter()
            .find(|t| t.kind == marker_kind)
            .expect("the destination marker is emitted");
        assert_eq!(
            (marker.x, marker.y, marker.angle),
            (448, 128, 90),
            "the marker sits at the destination, facing its angle"
        );
        assert!(!marker.ambush, "a marker is never an ambush thing");

        let imp_kind = tables.thing_id("imp").expect("imp");
        let imp = out
            .things
            .iter()
            .find(|t| t.kind == imp_kind)
            .expect("the authored imp is emitted");
        assert!(
            imp.ambush,
            "the authored `ambush` flag survives into ThingOut"
        );
        let start_kind = tables.thing_id("player1_start").expect("player1_start");
        assert!(
            out.things.iter().all(|t| t.kind != start_kind || !t.ambush),
            "the player start, which authored no flag, stays unambushed"
        );
    }

    /// A destination 14 units from room `b`'s own west wall — inside the
    /// player's 16-unit radius. Destinations are points, deliberately not
    /// grid-snapped by `Ir::from_json`, so a fixture can sit at any offset.
    const MARKER_TOO_CLOSE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[{ "kind":"player1_start", "at":[192,64], "angle":90 }] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":16, "ceiling":144, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }],
      "teleports":[{ "id":"t", "room":"a", "pad":{"island":[64,128]},
                     "to":{"room":"b","at":[334,128],"angle":90} }] }"#;

    #[test]
    fn a_destination_inside_the_arriving_things_radius_is_rejected() {
        let ir = Ir::from_json(MARKER_TOO_CLOSE).expect("ir");
        let tables = Tables::load().expect("tables");
        let err = compile_reporting(&ir, &tables).expect_err("14 < the player's 16-unit radius");
        assert!(
            matches!(
                &err,
                CompileError::TeleportMarkerTooClose { id, x, y, have, need }
                    if (id.as_str(), *x, *y, *need) == ("t", 334, 128, 16)
                        && (have - 14.0).abs() < f64::EPSILON
            ),
            "expected TeleportMarkerTooClose at (334, 128) with have = 14, got {err}"
        );
    }

    /// A two-way pair whose far half lands on room `b`'s own pad. Room `b` is
    /// exactly the player's height (56), which it may be — but the pad's
    /// floor sits `PAD_FLOOR_STEP` above it, leaving the pad 48 units of
    /// headroom, which the arriving player does not fit in.
    const MARKER_NO_HEADROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[{ "kind":"player1_start", "at":[192,64], "angle":90 }] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":0, "ceiling":56, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }],
      "teleports":[
        { "id":"t1", "room":"a", "pad":{"island":[64,128]},
          "to":{"room":"b","at":[480,160],"angle":90} },
        { "id":"t2", "room":"b", "pad":{"island":[448,128]},
          "to":{"room":"a","at":[192,192],"angle":0} }] }"#;

    #[test]
    fn a_destination_pad_too_short_for_the_arriving_thing_is_rejected() {
        let tables = Tables::load().expect("tables");
        assert_eq!(
            (tables.player().height, crate::ir::Ir::PAD_FLOOR_STEP),
            (56, 8),
            "the fixture's 56-unit room and its 48-unit pad assume these"
        );
        let ir = Ir::from_json(MARKER_NO_HEADROOM).expect("ir");
        let err = compile_reporting(&ir, &tables).expect_err("48 < the player's 56-unit height");
        assert!(
            matches!(
                &err,
                CompileError::TeleportMarkerNoHeadroom { id, have, need }
                    if (id.as_str(), *have, *need) == ("t1", 48, 56)
            ),
            "expected TeleportMarkerNoHeadroom naming the teleport delivering onto b's pad, \
             got {err}"
        );
    }

    /// A 512-unit square room with one 64x64 pedestal risen 128 units above
    /// it, carrying a soulsphere at its center.
    ///
    /// A copy of `lifts::tests::PEDESTAL` rather than a shared constant: a
    /// `#[cfg(test)] mod tests` is private to its own file, and what this
    /// pass owns is the thing standing on the platform rather than the
    /// platform under it.
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
    fn a_thing_on_a_pedestal_is_placed_at_its_point_with_clearance_against_the_pedestals_edges() {
        let tables = Tables::load().expect("tables");
        let soulsphere = tables.thing_id("soulsphere").expect("soulsphere");
        let out = compile(&Ir::from_json(PEDESTAL).expect("ir"), &tables).expect("compiles");
        let sphere = out
            .things
            .iter()
            .find(|t| t.kind == soulsphere)
            .expect("the pedestal's own thing is placed");
        assert_eq!(
            (sphere.x, sphere.y),
            (160, 160),
            "at its authored point, on the platform rather than the room floor"
        );

        // Too close to the pedestal's edge for the player's radius: 8 units
        // from the x = 128 edge, against a 16-unit radius. Measured against
        // the pedestal's own four edges — the room's walls are 128 away.
        let tight = PEDESTAL.replacen(r#""at":[160,160]"#, r#""at":[136,160]"#, 1);
        assert!(matches!(
            compile(&Ir::from_json(&tight).expect("ir"), &tables),
            Err(CompileError::ThingTooClose { .. })
        ));

        // A monster taller than the headroom over the pedestal. At ceiling
        // 184 the risen floor leaves 56 units: exactly the player's height,
        // so `emit_lifts` accepts the pedestal itself, and 8 short of the
        // baron's 64 — which only this loop can catch.
        let tall = PEDESTAL
            .replacen(r#""kind":"soulsphere""#, r#""kind":"baron_of_hell""#, 1)
            .replacen(r#""ceiling":256"#, r#""ceiling":184"#, 1);
        assert!(matches!(
            compile(&Ir::from_json(&tall).expect("ir"), &tables),
            Err(CompileError::PedestalNoHeadroom { kind, have: 56, need: 64, .. })
                if kind == "baron_of_hell"
        ));
    }

    /// `PEDESTAL`'s room and pedestal, with the pedestal carrying nothing
    /// and a soulsphere authored in the *room* at `at` instead — the mistake
    /// `ThingOnPedestal` exists to catch.
    fn room_thing_at(at: (i32, i32)) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]], "floor":0, "ceiling":256, "light":160,
                  "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                  "things":[ {{ "kind":"player1_start", "at":[448,448], "angle":0 }},
                             {{ "kind":"soulsphere", "at":[{},{}], "angle":0 }} ] }}
              ],
              "portals":[],
              "pedestals":[ {{ "id":"p", "room":"a", "at":[128,128], "rise":128 }} ] }}"#,
            at.0, at.1
        )
    }

    #[test]
    fn a_room_thing_standing_on_a_pedestal_is_refused_rather_than_placed_on_the_room_floor() {
        let tables = Tables::load().expect("tables");
        // The pedestal's rectangle is (128, 128)-(192, 192). A point in its
        // interior is authored on the room's floor but would spawn 128 units
        // up, in the platform's sector.
        let inside = room_thing_at((160, 160));
        assert!(
            matches!(
                compile(&Ir::from_json(&inside).expect("ir"), &tables),
                Err(CompileError::ThingOnPedestal { ref pedestal, ref kind, x: 160, y: 160 })
                    if pedestal == "p" && kind == "soulsphere"
            ),
            "a room thing inside the pedestal's rectangle must be refused"
        );

        // On the rectangle's own edge (x = 192, its high corner's x): the
        // test is closed on both axes, so this is refused too.
        let on_edge = room_thing_at((192, 160));
        assert!(
            matches!(
                compile(&Ir::from_json(&on_edge).expect("ir"), &tables),
                Err(CompileError::ThingOnPedestal { x: 192, y: 160, .. })
            ),
            "a room thing on the pedestal's boundary must be refused too"
        );

        // Clear of it, and clear of the island's edges by more than the
        // player's radius: placed as authored.
        let beside = room_thing_at((256, 160));
        assert!(
            compile(&Ir::from_json(&beside).expect("ir"), &tables).is_ok(),
            "64 units clear of the rectangle is not standing on it"
        );
    }

    /// One room, two pedestals, each carrying a player start of its own at
    /// its center — legal, and the fixture the overlapping-start case below
    /// is derived from.
    const TWO_PEDESTALS: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]], "floor":0, "ceiling":256, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[448,448], "angle":0 } ] }
      ],
      "portals":[],
      "pedestals":[
        { "id":"p", "room":"a", "at":[128,128], "rise":128,
          "things":[ { "kind":"player2_start", "at":[160,160], "angle":0 } ] },
        { "id":"q", "room":"a", "at":[256,128], "rise":128,
          "things":[ { "kind":"player3_start", "at":[288,160], "angle":0 } ] }
      ] }"#;

    #[test]
    fn starts_placed_on_pedestals_join_the_coincident_start_check() {
        let tables = Tables::load().expect("tables");
        let out = compile(&Ir::from_json(TWO_PEDESTALS).expect("ir"), &tables).expect("compiles");
        for name in ["player2_start", "player3_start"] {
            let kind = tables.thing_id(name).expect(name);
            assert!(
                out.things.iter().any(|t| t.kind == kind),
                "{name} is emitted from its pedestal"
            );
        }

        // The same start listed twice on one pedestal: two players would
        // spawn inside each other, exactly as two room starts at one point
        // would.
        let duplicated = TWO_PEDESTALS.replacen(
            r#"{ "kind":"player2_start", "at":[160,160], "angle":0 }"#,
            r#"{ "kind":"player2_start", "at":[160,160], "angle":0 },
                    { "kind":"player2_start", "at":[160,160], "angle":90 }"#,
            1,
        );
        assert!(matches!(
            compile(&Ir::from_json(&duplicated).expect("ir"), &tables),
            Err(CompileError::OverlappingStarts { x: 160, y: 160 })
        ));
    }

    #[test]
    fn a_thing_with_no_skills_specified_emits_all_five_true() {
        let ir = Ir::from_json(&ir_with_thing("player1_start", (128, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        let things = place_things(&ir, &tables, &data, &[], &[]).expect("placed");
        let skills = things[0].skills;
        assert!(
            skills.skill1 && skills.skill2 && skills.skill3 && skills.skill4 && skills.skill5,
            "no skills key means every skill, matching the compiler's original behavior"
        );
    }

    #[test]
    fn a_things_selected_skills_survive_into_thing_out() {
        let json = ir_with_thing("imp", (128, 128), 128).replace(
            "\"angle\":90",
            "\"angle\":90, \"skills\": { \"skill1\": false, \"skill2\": false }",
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        let things = place_things(&ir, &tables, &data, &[], &[]).expect("placed");
        let skills = things[0].skills;
        assert!(!skills.skill1 && !skills.skill2, "explicitly excluded");
        assert!(
            skills.skill3 && skills.skill4 && skills.skill5,
            "unmentioned skills default true"
        );
    }
}
