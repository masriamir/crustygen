//! Places things with real clearance, not merely inside their room.

use crate::compile::{CompileError, MapData};
use crate::geom::{Pt, contains, dist_to_segment};
use crate::ir::{Ir, ThingSkills};
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
}

/// The minimum distance from `p` to any emitted linedef bordering the sector
/// for room `room_idx`, in map units.
///
/// [`crate::compile::sectors::emit_sectors`] pushes exactly one sector per
/// room, in `ir.rooms` order, so a room's index doubles as its sector index
/// — verified directly against that module rather than assumed. Every
/// linedef whose front or back sidedef names that sector is a real wall the
/// player can collide with in the *emitted* geometry, which is what matters
/// near a door: a door portal carves a `DOOR_DEPTH`-deep recess out of room
/// `b`'s side of the shared wall (see `compile::doors`), so the true nearest
/// wall there sits closer than the room's IR-declared footprint suggests.
/// Gathering from `data` rather than `room.footprint` picks that recess up
/// automatically — and would pick up any other reshaping a later pass
/// performs, without this function needing to know what changed.
///
/// Returns `None` when the sector has no bordering linedef at all. An
/// unbounded room cannot be measured, and folding to `f64::INFINITY` instead
/// would have silently *passed* every clearance check — the failure mode
/// where a broken room looks safest.
fn emitted_clearance(data: &MapData, room_idx: usize, p: Pt) -> Option<f64> {
    data.linedefs
        .iter()
        .filter(|line| {
            data.sidedefs[line.front].sector == room_idx
                || line
                    .back
                    .is_some_and(|back| data.sidedefs[back].sector == room_idx)
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
/// `data`, not the declared footprint: a door portal's recess (see
/// `compile::doors`) makes room `b`'s real playable area near a doorway
/// smaller than its footprint, and only the emitted linedefs reflect that.
///
/// # Errors
/// Returns [`CompileError::UnknownThing`] for an unresolvable name,
/// [`CompileError::ThingOutsideRoom`] for a point outside its room,
/// [`CompileError::ThingTooClose`] when radius clearance fails,
/// [`CompileError::NoHeadroom`] when the room is too short — for the
/// tallest placed thing, or for the player if the room is empty or shorter
/// than the player alone — [`CompileError::UnboundedRoom`] when a room's
/// sector has no emitted linedef to measure against, and
/// [`CompileError::OverlappingStarts`] for coincident player starts.
pub fn place_things(
    ir: &Ir,
    tables: &Tables,
    data: &MapData,
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
            });
        }
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
    use crate::geom::{Pt, clearance};
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// Runs the full geometry pipeline (sectors -> portals -> doors) so
    /// tests exercise `place_things` against real emitted geometry, matching
    /// what the eventual `compile` orchestrator hands it.
    fn compiled_data(ir: &Ir, tables: &Tables) -> MapData {
        let mut data = emit_sectors(ir).expect("sectors");
        cut_portals(ir, &mut data).expect("portals");
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
        let things = place_things(&ir, &tables, &data).expect("placed");
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
            place_things(&ir, &tables, &data),
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
            place_things(&ok, &tables, &data_ok).is_ok(),
            "at the radius it fits"
        );
        // One unit closer: it does not.
        let bad = Ir::from_json(&ir_with_thing("player1_start", (r - 1, 128), 128)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(matches!(
            place_things(&bad, &tables, &data_bad),
            Err(CompileError::ThingTooClose { .. })
        ));
    }

    #[test]
    fn headroom_is_boundary_pinned_to_the_thing_height() {
        let tables = Tables::load().expect("tables");
        let h = tables.player().height;
        let ok = Ir::from_json(&ir_with_thing("player1_start", (128, 128), h)).expect("ir");
        let data_ok = compiled_data(&ok, &tables);
        assert!(
            place_things(&ok, &tables, &data_ok).is_ok(),
            "at exactly its height it fits"
        );
        let bad = Ir::from_json(&ir_with_thing("player1_start", (128, 128), h - 1)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(matches!(
            place_things(&bad, &tables, &data_bad),
            Err(CompileError::NoHeadroom { .. })
        ));
    }

    #[test]
    fn an_unknown_vocabulary_name_is_rejected() {
        let ir = Ir::from_json(&ir_with_thing("nonesuch", (128, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        assert!(matches!(
            place_things(&ir, &tables, &data),
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
            place_things(&ir, &tables, &data),
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
            place_things(&ir, &tables, &MapData::default()),
            Err(CompileError::UnboundedRoom { .. })
        ));
    }

    #[test]
    fn clearance_is_measured_against_the_emitted_geometry_not_the_declared_footprint() {
        // Mirrors `compile::doors::DOOR_DEPTH`, which is private to that
        // module. See its doc comment there for why 16 is a compiler
        // construction constant rather than an engine-sourced one — this
        // test only needs its value to derive the recessed wall's position,
        // not to re-litigate where it comes from.
        const DOOR_DEPTH: i32 = 16;

        let tables = Tables::load().expect("tables");
        let r = tables.player().radius;

        let ir_json = |x: i32| -> String {
            format!(
                r#"{{ "seed":1, "grid":64, "theme":"tech_base",
                  "rooms":[
                    {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                       "floor":0, "ceiling":128, "light":160,
                       "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                    {{ "id":"b", "footprint":[[256,0],[256,256],[512,256],[512,0]],
                       "floor":0, "ceiling":128, "light":160,
                       "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
                       "things":[{{ "kind":"player1_start", "at":[{x},128], "angle":90 }}] }}
                  ],
                  "portals":[{{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128] }}] }}"#
            )
        };

        // The recess pushes room b's real wall from x = 256 (the declared
        // footprint edge) out to x = 256 + DOOR_DEPTH = 272 across the door
        // span (see `compile::doors`'s worked example for this exact
        // fixture). Exactly `r` past that recessed face fits; one unit
        // closer does not.
        let far_face = 256 + DOOR_DEPTH;

        let ok = Ir::from_json(&ir_json(far_face + r)).expect("ir");
        let data_ok = compiled_data(&ok, &tables);
        assert!(
            place_things(&ok, &tables, &data_ok).is_ok(),
            "exactly the radius from the recessed far face fits"
        );

        let bad = Ir::from_json(&ir_json(far_face + r - 1)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(
            matches!(
                place_things(&bad, &tables, &data_bad),
                Err(CompileError::ThingTooClose { .. })
            ),
            "one unit closer than the radius does not fit"
        );

        // Prove this is the defect the deviation guards against: the stale,
        // IR-declared footprint alone reports the rejected point as
        // comfortably clear, because it knows nothing about the recess.
        let stale = clearance(
            &bad.rooms[1].footprint,
            Pt {
                x: far_face + r - 1,
                y: 128,
            },
        );
        assert!(
            stale > f64::from(r),
            "footprint-only clearance ({stale}) looks safe even though the point is rejected \
             against the emitted geometry — that gap is exactly what data-based clearance closes"
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
            place_things(&ok, &tables, &data_ok).is_ok(),
            "at exactly the player's height, an empty room is fine"
        );

        let bad = Ir::from_json(&empty_corridor_json(h - 1)).expect("ir");
        let data_bad = compiled_data(&bad, &tables);
        assert!(
            matches!(
                place_things(&bad, &tables, &data_bad),
                Err(CompileError::NoHeadroom { .. })
            ),
            "an empty room one unit too short must still be rejected"
        );
    }

    #[test]
    fn a_thing_with_no_skills_specified_emits_all_five_true() {
        let ir = Ir::from_json(&ir_with_thing("player1_start", (128, 128), 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let data = compiled_data(&ir, &tables);
        let things = place_things(&ir, &tables, &data).expect("placed");
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
        let things = place_things(&ir, &tables, &data).expect("placed");
        let skills = things[0].skills;
        assert!(!skills.skill1 && !skills.skill2, "explicitly excluded");
        assert!(
            skills.skill3 && skills.skill4 && skills.skill5,
            "unmentioned skills default true"
        );
    }
}
