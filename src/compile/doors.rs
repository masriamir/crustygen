//! Emits a dedicated, closed sector for each door portal, optionally
//! flanked by trim alcove sectors.
//!
//! Rooms are authored apart (see [`crate::ir::Portal`]'s doc comment), so a
//! door portal's wall gap already exists before this pass ever runs — there
//! is nowhere left to carve. This pass fills that gap with a chain of one to
//! three sectors, laid out along the gap axis from room `a`'s wall to room
//! `b`'s: an optional near alcove ([`Portal::alcove_near`]), the door itself
//! ([`Portal::door_thickness`] deep), and an optional far alcove
//! ([`Portal::alcove_far`]). [`Ir::from_json`] already guarantees these three
//! lengths sum to exactly the gap, so the chain always reaches from room
//! `a`'s wall to room `b`'s with nothing left over and nothing to trim.
//!
//! Every segment in the chain has the same shape
//! `crate::compile::portals::emit_segment` builds: two "face" lines
//! (perpendicular to the direction of travel through the doorway, one on
//! each side) and two one-sided "jamb" lines (parallel to it, closing the
//! segment's long sides, front bound to the segment's own sector with solid
//! rock behind). Only the door segment's own two faces carry the door
//! special and its sector's tag — an alcove's two faces are plain,
//! non-blocking passages, exactly like a [`PortalKind::Plain`] portal's own
//! gap sector. Neither room's own declared footprint is touched.
//!
//! This runs after [`crate::compile::portals::cut_portals`], which leaves a
//! door portal's flanking walls in place but the gap itself still empty —
//! the chain this pass builds is exactly what fills it.

use crate::compile::portals::{
    Cut, emit_jambs, emit_opening, emit_segment, mark_secret_thresholds, resolve_portal,
};
use crate::compile::tags::TagAllocator;
use crate::compile::{CompileError, MapData, SectorOut};
use crate::ir::{Ir, Portal, PortalKind, Room};
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

/// Validates that `texture` is recognized by the project's curated
/// door-texture catalog (see [`Tables::is_door_texture`]'s doc comment for
/// why this is curated rather than sourced).
///
/// Extracted as a standalone, directly callable function rather than inlined
/// in [`emit_doors`] specifically so it can be unit tested against both an
/// accepted and a rejected texture name using the real, loaded [`Tables`]:
/// the shipped `vocabulary.toml` currently defines exactly one theme
/// (`tech_base`), and its `door` texture is correctly configured, so no
/// legitimate IR/theme combination can drive [`emit_doors`] itself into the
/// rejection path today — see the door-redesign report for the full
/// reasoning.
///
/// # Errors
/// Returns [`CompileError::NotADoorTexture`] when `texture` is not in the
/// curated catalog.
fn validate_door_texture(tables: &Tables, theme: &str, texture: &str) -> Result<(), CompileError> {
    if tables.is_door_texture(texture) {
        Ok(())
    } else {
        Err(CompileError::NotADoorTexture {
            theme: theme.to_owned(),
            texture: texture.to_owned(),
        })
    }
}

/// Emits a dedicated, initially closed sector — flanked by up to two
/// optional trim alcove sectors — for every door portal, filling the wall
/// gap [`crate::compile::portals::cut_portals`] already cut into both
/// rooms' own walls.
///
/// See the module documentation for the construction. The door sector's own
/// two faces carry the door special (so the door can actually be opened,
/// from either room, or from an alcove standing in front of it) and the
/// door sector's tag, but neither pegging flag: a face's visible texture is
/// its upper, which `ML_DONTPEGBOTTOM`/`ML_DONTPEGTOP` never govern (P11).
/// Only the door's own jambs — its track — are lower-unpegged by default,
/// so the DOORTRAK texture does not slide as the sector's ceiling animates
/// open, gated on [`Portal::track_lower_unpegged`]. An alcove's own faces
/// and jambs are never lower-unpegged (nothing there ever moves) and never
/// carry a special, matching a [`PortalKind::Plain`] portal's own passage.
///
/// Resolves each door portal's geometry independently via
/// `crate::compile::portals::resolve_portal` rather than trusting
/// `cut_portals` already ran — the same defense-in-depth
/// [`crate::compile::exits::emit_exits`] follows for its own resolution.
///
/// # Errors
/// Returns [`CompileError::UnknownTheme`] when `ir.theme` resolves to no
/// texture set, [`CompileError::NotADoorTexture`] when the theme's `door`
/// texture is not in the project's curated door-texture catalog,
/// [`CompileError::UnknownLock`] when a locked portal names a key the
/// vocabulary has no special for, and whatever `resolve_portal` raises
/// (`NotAdjacent`, `PortalOffWall`, `PortalOnDiagonalWall`, `PortalTooWide`)
/// if a door portal's rooms are not adjacent on a wall v1 can cut.
///
/// # Panics
/// Panics (debug builds only) if the door segment is ever pushed at an index
/// other than predicted, or if the emitted far boundary does not land
/// exactly on room `b`'s own wall — both unreachable, since
/// [`Ir::from_json`] already guarantees `door_thickness + alcove_near +
/// alcove_far` equals the gap exactly.
#[expect(
    clippy::too_many_lines,
    reason = "the chain construction (up to three sectors, up to four boundaries, jambs, \
              texture/special/pegging assignment) is one coherent unit of work per door portal; \
              splitting it into smaller functions would just scatter the sequential dependency \
              between pos0..pos3 across call boundaries without making any single piece simpler"
)]
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
    let trim_tex = tables
        .texture("trim", &ir.theme)
        .ok_or_else(unknown_theme)?
        .to_owned();
    validate_door_texture(tables, &ir.theme, &door_tex)?;

    for portal in &ir.portals {
        if !matches!(portal.kind, PortalKind::Door | PortalKind::Locked) {
            continue;
        }

        let geometry = resolve_portal(ir, portal)?;
        let special = door_special(tables, portal)?;

        // The alcove jambs are the trim a player faces walking up to the
        // door, so a locked door announces its key there. The door's own
        // track is never touched — it stays the theme's `door_track`
        // (DOORTRAK) unconditionally, which is what a custom texture WAD
        // would override. `key_trim` returns `None` for a key with no trim
        // of its own, in which case the alcove keeps the plain theme trim
        // rather than silently losing its texture.
        //
        // A locked portal declaring no alcoves has nowhere to carry this,
        // and gets no key trim — see KNOWN-GAPS.md.
        let alcove_tex: &str = portal
            .lock
            .as_deref()
            .and_then(|key| tables.key_trim(key))
            .unwrap_or(&trim_tex);

        let door_thickness = portal
            .door_thickness
            .expect("Ir::from_json guarantees every door/locked portal names a door_thickness");
        let alcove_near = portal.alcove_near.unwrap_or(0);
        let alcove_far = portal.alcove_far.unwrap_or(0);

        let room_a = &ir.rooms[geometry.ia];
        let room_b = &ir.rooms[geometry.ib];
        let axis = geometry.span.axis;
        let a_forward = geometry.span.a_forward;
        let (open_lo, open_hi) = (geometry.open_lo, geometry.open_hi);

        // Positions along the gap axis, from room a's own wall (`pos0`) to
        // room b's own wall (`pos3`). `pos1`/`pos2` are the door's own near
        // and far faces — equal to `pos0`/`pos3` respectively when the
        // matching alcove is absent, so the door then borders a real room
        // directly, exactly like the pre-alcove construction. `Ir::from_json`
        // guarantees `door_thickness + alcove_near + alcove_far` equals the
        // gap exactly, so `pos3` always lands on `geometry.span.far` with
        // nothing left over and nothing to trim.
        let dir = (geometry.span.far - geometry.span.near).signum();
        let pos0 = geometry.span.near;
        let pos1 = pos0 + dir * alcove_near;
        let pos2 = pos1 + dir * door_thickness;
        let pos3 = pos2 + dir * alcove_far;
        debug_assert_eq!(
            pos3, geometry.span.far,
            "Ir::from_json guarantees the alcoves and door thickness sum to the gap"
        );

        let alcove_sector_out = |room: &Room| SectorOut {
            floor: room.floor,
            ceiling: room.ceiling,
            light: room.light,
            floor_tex: room.floor_tex.clone(),
            ceil_tex: room.ceil_tex.clone(),
            special: 0,
            tag: 0,
            wall_tex: room.wall_tex.clone(),
        };

        let near_alcove = (alcove_near > 0).then(|| {
            let idx = data.sectors.len();
            data.sectors.push(alcove_sector_out(room_a));
            idx
        });
        let far_alcove = (alcove_far > 0).then(|| {
            let idx = data.sectors.len();
            data.sectors.push(alcove_sector_out(room_b));
            idx
        });

        let door_sector = data.sectors.len();
        let tag = tags.allocate(door_sector, &format!("door {} <-> {}", portal.a, portal.b));
        let floor = room_a.floor.min(room_b.floor);
        data.sectors.push(SectorOut {
            floor,
            // A closed door: ceiling snapped to the floor.
            ceiling: floor,
            light: room_a.light,
            floor_tex: room_a.floor_tex.clone(),
            ceil_tex: room_a.ceil_tex.clone(),
            special: 0,
            tag,
            wall_tex: room_a.wall_tex.clone(),
        });

        // The door's own two faces — whichever segment stands immediately
        // in front of each (a real room, or an alcove) is on the front.
        // Builds *both* of the door's own boundaries (its near and far
        // threshold) in one call, since neither is shared with anything
        // else: the door's near threshold IS the near alcove's inner
        // boundary when one is present, so it must be built exactly once,
        // here — not again by the alcove's own construction below.
        let door_near_neighbor = near_alcove.unwrap_or(geometry.ia);
        let door_far_neighbor = far_alcove.unwrap_or(geometry.ib);
        let door_seg = emit_segment(
            data,
            axis,
            open_lo,
            open_hi,
            a_forward,
            pos1,
            pos2,
            door_near_neighbor,
            door_sector,
            door_far_neighbor,
            &track_tex,
        );
        debug_assert_eq!(
            door_seg.sector, door_sector,
            "the door segment was pushed at the predicted index"
        );

        // Each present alcove's own *outer* boundary only — a plain
        // passage, exactly like a Plain portal's own gap sector — plus its
        // own jambs. Its *inner* boundary (bordering the door) was already
        // built above, as one of the door's own two faces; building it again
        // here would emit the same physical wall twice as two coincident,
        // overlapping linedefs.
        // Every two-sided line along the chain, outermost inward. A door
        // into a secret room conceals all of them on the automap, not just
        // the outer pair — see `mark_secret_thresholds`.
        let mut thresholds = vec![door_seg.near_line, door_seg.far_line];

        if let Some(alcove) = near_alcove {
            thresholds.push(emit_opening(
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
            ));
            emit_jambs(
                data, axis, open_lo, open_hi, a_forward, pos0, pos1, alcove, alcove_tex,
            );
        }
        if let Some(alcove) = far_alcove {
            thresholds.push(emit_opening(
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
            ));
            emit_jambs(
                data, axis, open_lo, open_hi, a_forward, pos2, pos3, alcove, alcove_tex,
            );
        }

        mark_secret_thresholds(data, room_a.secret != room_b.secret, thresholds);

        // The door's own two faces carry neither pegging flag: unlike the
        // track, their visible texture is the upper (the door slab itself,
        // drawn while the sector's ceiling is below the neighboring
        // sectors' — see the module doc comment), and `ML_DONTPEGBOTTOM`
        // never affects upper-texture rendering (`r_segs.c`, pinned commit
        // a77dfb96) — so the flag was inert there. 247/255 door-special
        // lines in DOOM2.WAD ship unflagged, confirming this is the
        // convention, not an oversight.
        for line in [door_seg.near_line, door_seg.far_line] {
            data.linedefs[line].special = special;
            data.linedefs[line].tag = tag;
            let front = data.linedefs[line].front;
            let back = data.linedefs[line]
                .back
                .expect("emit_segment's thresholds are always two-sided");
            data.sidedefs[front].upper.clone_from(&door_tex);
            data.sidedefs[back].upper.clone_from(&door_tex);
        }
        // The jambs (the door's track) always carry DOORTRAK, but whether
        // they are lower-unpegged is the author's call
        // (`Portal::track_lower_unpegged`, default `true`): when left on,
        // their middle texture must not slide as the door sector's ceiling
        // animates open (P11) — unlike the faces above, whose visible
        // texture is unaffected by either pegging flag.
        for line in door_seg.jamb_lines {
            data.linedefs[line].lower_unpegged = portal.track_lower_unpegged;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_door_texture;
    use crate::compile::MapData;
    use crate::compile::doors::emit_doors;
    use crate::compile::portals::cut_portals;
    use crate::compile::sectors::emit_sectors;
    use crate::compile::tags::TagAllocator;
    use crate::geom::{Pt, contains};
    use crate::ir::Ir;
    use crate::tables::Tables;

    /// A locked door announces its key on the **alcove** jambs — the trim a
    /// player faces walking up to it — while the door's own track keeps
    /// `DOORTRAK` unconditionally.
    ///
    /// The card/skull split is the measured convention recorded in
    /// `vocabulary.toml`'s `[key_trim]`: the plain name for a keycard, the
    /// `2` variant for a skull key. Both are checked here, because a
    /// mapping that ignored the key's kind would satisfy either one alone.
    #[test]
    fn a_locked_door_marks_its_alcove_jambs_with_the_key_trim() {
        for (lock, expected) in [("blue_card", "DOORBLU"), ("blue_skull", "DOORBLU2")] {
            let json = format!(
                r#"{{ "seed":1, "grid":8, "theme":"tech_base",
                  "rooms":[
                    {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                      "floor":0, "ceiling":128, "light":160,
                      "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }},
                    {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                      "floor":0, "ceiling":128, "light":160,
                      "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }}
                  ],
                  "portals":[{{ "a":"a", "b":"b", "kind":"locked", "lock":"{lock}",
                                "width":128, "at":[256,128],
                                "door_thickness":32, "alcove_near":16, "alcove_far":16 }}] }}"#
            );
            let ir = Ir::from_json(&json).expect("ir");
            let tables = Tables::load().expect("tables");
            let mut data = emit_sectors(&ir).expect("sectors");
            cut_portals(&ir, &tables, &mut data).expect("portals");
            let mut tags = TagAllocator::new();
            emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");

            // A door sector is the one whose ceiling is snapped to its floor
            // (closed); an alcove is any other compiler-made sector.
            let mut alcove_jambs = 0;
            let mut track_jambs = 0;
            for line in data.linedefs.iter().filter(|l| l.back.is_none()) {
                let side = &data.sidedefs[line.front];
                let sector = &data.sectors[side.sector];
                if side.sector < ir.rooms.len() {
                    continue; // a room's own wall, not part of the chain
                }
                if sector.ceiling == sector.floor {
                    track_jambs += 1;
                    assert_eq!(
                        side.middle, "DOORTRAK",
                        "{lock}: the door's own track is never keyed"
                    );
                } else {
                    alcove_jambs += 1;
                    assert_eq!(
                        side.middle, expected,
                        "{lock}: alcove jambs carry the key trim"
                    );
                }
            }
            assert_eq!(alcove_jambs, 4, "{lock}: two alcoves, two jambs each");
            assert_eq!(track_jambs, 2, "{lock}: the door has two track jambs");
        }
    }

    /// An unlocked door's alcoves keep the theme's plain trim — the key
    /// texture is tied to the lock, not to being a door.
    #[test]
    fn an_unlocked_door_keeps_the_themes_plain_trim() {
        let json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");

        let trim = tables.texture("trim", "tech_base").expect("trim");
        let alcove_jambs: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none())
            .map(|l| &data.sidedefs[l.front])
            .filter(|s| {
                s.sector >= ir.rooms.len()
                    && data.sectors[s.sector].ceiling != data.sectors[s.sector].floor
            })
            .collect();
        assert_eq!(alcove_jambs.len(), 4);
        assert!(alcove_jambs.iter().all(|s| s.middle == trim));
    }

    /// A door into a secret room conceals **every** two-sided line of its
    /// chain on the automap — the door's own two faces and both alcoves'
    /// outer thresholds — not just the pair nearest the ordinary room.
    ///
    /// A door chain is the case a plain portal cannot cover: it has up to
    /// four thresholds rather than two, built by two different code paths
    /// (`emit_segment` for the door itself, `emit_opening` for each alcove),
    /// so a fix applied to only one path would still pass the plain-portal
    /// test in `portals.rs`.
    #[test]
    fn a_door_into_a_secret_room_conceals_its_whole_chain() {
        let json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W", "secret":true }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
        let mut tags = TagAllocator::new();
        emit_doors(&ir, &tables, &mut data, &mut tags).expect("doors");

        let two_sided: Vec<_> = data.linedefs.iter().filter(|l| l.back.is_some()).collect();
        assert_eq!(
            two_sided.len(),
            4,
            "near alcove's outer threshold, the door's two faces, far alcove's outer threshold"
        );
        assert!(
            two_sided.iter().all(|l| l.secret),
            "every threshold in the chain is concealed, not just the outermost"
        );
        assert!(
            data.linedefs
                .iter()
                .filter(|l| l.back.is_none())
                .all(|l| !l.secret),
            "the jambs and room walls are one-sided; the flag would do nothing there"
        );
    }

    #[test]
    fn validate_door_texture_accepts_a_curated_name_and_rejects_others() {
        // `emit_doors` calls this exact function with the theme's resolved
        // `door` texture; unit tested directly against the real, loaded
        // `Tables` because the shipped vocabulary.toml has exactly one
        // theme, correctly configured, so no IR/theme combination can drive
        // `emit_doors` itself into the rejection path — see this function's
        // own doc comment.
        let tables = Tables::load().expect("tables");
        assert!(
            validate_door_texture(&tables, "tech_base", "BIGDOOR2").is_ok(),
            "tech_base's own configured door texture is accepted"
        );
        let err = validate_door_texture(&tables, "tech_base", "STARTAN3")
            .expect_err("a wall texture is not a door texture");
        assert!(matches!(
            err,
            crate::compile::CompileError::NotADoorTexture {
                texture,
                ..
            } if texture == "STARTAN3"
        ));
    }

    /// Room `a` and room `b` face each other across a legal 32-unit gap
    /// (room `a`'s east wall at x = 256, room `b`'s west wall at x = 288),
    /// exactly filled by a single 32-unit-thick door with no alcove — the
    /// simplest legal chain (one segment), used by every test in this file
    /// that doesn't care about the alcove feature specifically.
    const DOOR_IR: &str = r#"{ "seed":1, "grid":8, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[288,0],[288,256],[544,256],[544,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                    "door_thickness":32 }] }"#;

    /// Runs the full `emit_sectors` -> `cut_portals` -> `emit_doors`
    /// pipeline and returns the resulting `MapData` plus the door sector's
    /// index (always the last sector, since `emit_doors` only appends).
    fn compiled(ir_json: &str) -> (Ir, MapData, usize) {
        let ir = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
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
        cut_portals(&ir, &tables, &mut data).expect("portals");
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
    fn each_alcove_takes_its_own_adjoining_rooms_floor_ceiling_light_and_textures() {
        // Load bearing for pinning "near alcove copies room a, far alcove
        // copies room b" rather than restating it: room a and room b differ
        // in every value an alcove sector could copy (floor, ceiling,
        // light, floor_tex, ceil_tex) — a fixture where the two rooms agree
        // would let "always copy room a" or "always copy room b" pass
        // silently, exactly the failure mode the project's own mutation
        // testing culture warns about (see the wall-thickness report's own
        // "Mutation I").
        let ir_json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,64],[64,64],[64,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FA", "ceil_tex":"CA", "wall_tex":"W" },
            { "id":"b", "footprint":[[96,0],[96,64],[160,64],[160,0]],
              "floor":16, "ceiling":112, "light":96,
              "floor_tex":"FB", "ceil_tex":"CB", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":32, "at":[64,32],
                        "door_thickness":16, "alcove_near":8, "alcove_far":8 }] }"#;
        let (ir, data, door) = compiled(ir_json);
        // Sectors: 0=a, 1=b, then near alcove, far alcove, door (emission
        // order — see `emit_doors`'s own note on why the door is always
        // pushed last).
        let near_alcove = &data.sectors[2];
        let far_alcove = &data.sectors[3];
        assert_eq!(
            data.sectors.len() - ir.rooms.len(),
            3,
            "near alcove, far alcove, door"
        );
        assert_eq!(door, 4, "the door is pushed last, after both alcoves");

        assert_eq!(
            near_alcove.floor, ir.rooms[0].floor,
            "near alcove floor matches room a"
        );
        assert_eq!(
            near_alcove.ceiling, ir.rooms[0].ceiling,
            "near alcove ceiling matches room a"
        );
        assert_eq!(
            near_alcove.light, ir.rooms[0].light,
            "near alcove light matches room a"
        );
        assert_eq!(
            near_alcove.floor_tex, ir.rooms[0].floor_tex,
            "near alcove floor texture matches room a"
        );
        assert_eq!(
            near_alcove.ceil_tex, ir.rooms[0].ceil_tex,
            "near alcove ceiling texture matches room a"
        );

        assert_eq!(
            far_alcove.floor, ir.rooms[1].floor,
            "far alcove floor matches room b"
        );
        assert_eq!(
            far_alcove.ceiling, ir.rooms[1].ceiling,
            "far alcove ceiling matches room b"
        );
        assert_eq!(
            far_alcove.light, ir.rooms[1].light,
            "far alcove light matches room b"
        );
        assert_eq!(
            far_alcove.floor_tex, ir.rooms[1].floor_tex,
            "far alcove floor texture matches room b"
        );
        assert_eq!(
            far_alcove.ceil_tex, ir.rooms[1].ceil_tex,
            "far alcove ceiling texture matches room b"
        );
    }

    #[test]
    fn the_door_track_is_lower_unpegged_so_it_does_not_slide() {
        // The track (the door sector's one-sided jambs, DOORTRAK) is the
        // only geometry this setting was ever meant to govern — see
        // `door_faces_carry_neither_pegging_flag` below for the corrected
        // face behavior.
        let (_, data, door) = compiled(DOOR_IR);
        let jambs: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == door)
            .collect();
        assert_eq!(jambs.len(), 2, "a door has two track jambs");
        assert!(
            jambs.iter().all(|l| l.lower_unpegged),
            "the door track is lower-unpegged by default"
        );
    }

    #[test]
    fn door_faces_carry_neither_pegging_flag() {
        // Human ruling (project owner, during Task 5 review): only the
        // track is lower-unpegged by default; `ML_DONTPEGBOTTOM` never
        // touches upper-texture rendering (`r_segs.c`, pinned commit
        // a77dfb96), which is what a door face's visible texture is, and
        // 247/255 door-special lines in DOOM2.WAD ship unflagged. Asserted
        // directly against the compiled `MapData`, not through `check::run`
        // — the compiler pins its own emission.
        let (_, data, door) = compiled(DOOR_IR);
        let faces: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_some_and(|b| data.sidedefs[b].sector == door))
            .collect();
        assert_eq!(faces.len(), 2, "a door has two faces");
        assert!(
            faces.iter().all(|l| !l.lower_unpegged && !l.upper_unpegged),
            "a door's own two faces carry neither dontpegbottom nor dontpegtop"
        );
    }

    #[test]
    fn track_lower_unpegged_can_be_disabled_while_the_texture_stays_doortrak() {
        // Default `true` is already pinned by
        // `the_door_track_is_lower_unpegged_so_it_does_not_slide` above
        // (DOOR_IR sets no `track_lower_unpegged` at all). This pins the
        // other half: an explicit `false` disables pegging on the jambs
        // specifically — the door's own two faces carry neither pegging
        // flag either way, since `track_lower_unpegged` only ever governs
        // the track — while the DOORTRAK texture is unaffected either way.
        let disabled = DOOR_IR.replace(
            "\"door_thickness\":32",
            "\"door_thickness\":32, \"track_lower_unpegged\":false",
        );
        let (_, data, door) = compiled(&disabled);
        let jambs: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_none() && data.sidedefs[l.front].sector == door)
            .collect();
        assert_eq!(jambs.len(), 2, "a door has two jambs");
        assert!(
            jambs.iter().all(|l| !l.lower_unpegged),
            "track_lower_unpegged:false disables pegging on the jambs"
        );
        assert!(
            jambs
                .iter()
                .all(|l| data.sidedefs[l.front].middle == "DOORTRAK"),
            "the jamb texture is unaffected by the pegging toggle"
        );

        let faces: Vec<_> = data
            .linedefs
            .iter()
            .filter(|l| l.back.is_some() && data.sidedefs[l.front].sector != door)
            .collect();
        assert_eq!(faces.len(), 2, "a door has two faces");
        assert!(
            faces.iter().all(|l| !l.lower_unpegged && !l.upper_unpegged),
            "the door's own faces carry neither pegging flag regardless of \
             track_lower_unpegged, which only governs the jambs"
        );
    }

    #[test]
    fn a_plain_portal_adds_no_sector_via_emit_doors() {
        // A plain portal's own passage sector is added by `cut_portals`
        // itself, not `emit_doors` — this pins that `emit_doors` skips
        // `PortalKind::Plain` entirely, adding zero *further* sectors. The
        // door-only `door_thickness` field is stripped too: `Ir::from_json`
        // rejects a plain portal that sets it (`DoorFieldsOnPlainPortal`).
        let plain = DOOR_IR
            .replace("\"door\"", "\"plain\"")
            .replace(",\n                    \"door_thickness\":32", "");
        let ir = Ir::from_json(&plain).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        cut_portals(&ir, &tables, &mut data).expect("portals");
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

    /// The axis-aligned bounding rectangle of every vertex touching
    /// `sector`'s boundary. Mirrors
    /// `portals::tests::gap_sector_bbox` (duplicated, not shared, per this
    /// module's own convention) — valid for the door sector or any alcove
    /// sector, every one of which is a plain axis-aligned rectangle by
    /// construction.
    fn sector_bbox(data: &MapData, sector: usize) -> (i32, i32, i32, i32) {
        let verts: Vec<Pt> = data
            .linedefs
            .iter()
            .filter(|l| {
                data.sidedefs[l.front].sector == sector
                    || l.back.is_some_and(|b| data.sidedefs[b].sector == sector)
            })
            .flat_map(|l| [data.vertices[l.v1], data.vertices[l.v2]])
            .collect();
        (
            verts
                .iter()
                .map(|v| v.x)
                .min()
                .expect("sector has geometry"),
            verts
                .iter()
                .map(|v| v.x)
                .max()
                .expect("sector has geometry"),
            verts
                .iter()
                .map(|v| v.y)
                .min()
                .expect("sector has geometry"),
            verts
                .iter()
                .map(|v| v.y)
                .max()
                .expect("sector has geometry"),
        )
    }

    /// Whether `sector`'s interior is on the right of travel from `p` to
    /// `q`: [`interior_is_on_the_right`] for a real room (`sector <
    /// ir.rooms.len()`), or a bounding-box containment test via
    /// [`sector_bbox`] for any compiler-generated sector — the door, or
    /// either alcove, all of them indistinguishable from this test's point
    /// of view, unlike [`assert_door_chain`]'s own more specific checks.
    fn chain_sector_interior_is_on_the_right(
        ir: &Ir,
        data: &MapData,
        sector: usize,
        p: Pt,
        q: Pt,
    ) -> bool {
        if sector < ir.rooms.len() {
            interior_is_on_the_right(ir, sector, p, q)
        } else {
            let (x_lo, x_hi, y_lo, y_hi) = sector_bbox(data, sector);
            let pt = probe(p, q);
            pt.x >= x_lo && pt.x <= x_hi && pt.y >= y_lo && pt.y <= y_hi
        }
    }

    /// Asserts one door portal's full chain construction — one to three
    /// segments (an optional near alcove, the door, an optional far alcove)
    /// — against coordinates hand-derived from the fixture's own footprints,
    /// generalizing the project's original single-segment
    /// `assert_door_construction` (no longer able to describe an
    /// alcove-bearing door, since the door no longer necessarily spans the
    /// whole gap) to a chain of arbitrary length. Checks:
    ///
    /// - `emit_doors` added exactly one sector per present chain component
    ///   (1 to 3: the door, plus one per present alcove);
    /// - the door sector is a closed quadrilateral (`referenced == 4`) at
    ///   exactly `door_corners`, with its ceiling snapped to its floor;
    /// - exactly two linedefs carry the door special, and both name the door
    ///   sector on their *back* side, never their front (`P_UseSpecialLine`
    ///   triggers from the front, and the front must be whichever segment a
    ///   player can actually stand in to press "use");
    /// - each present alcove (`near_alcove_corners`/`far_alcove_corners`,
    ///   matched to whichever emitted sector's own bounding box equals the
    ///   expected corners, not by assumed index order) is a closed
    ///   quadrilateral at those corners, open rather than closed (its
    ///   ceiling above its floor);
    /// - both room `a` and room `b` keep their own plain-portal boundary
    ///   shape (`edges + 2`), unaffected by the door or its alcoves —
    ///   unchanged from the original single-segment invariant, since
    ///   splitting the *gap* into more segments never touches a room's own
    ///   wall a second time;
    /// - every sector's boundary is a closed loop
    ///   (`assert_sector_boundaries_are_closed`);
    /// - every sidedef's declared sector has its interior genuinely on the
    ///   declared side of the line
    ///   (`chain_sector_interior_is_on_the_right`), covering every sidedef
    ///   in the fixture — a real room, the door, or either alcove alike.
    ///
    /// Counts are taken over sidedefs actually referenced by a surviving
    /// linedef's `front`/`back`, not raw `data.sidedefs` array membership —
    /// see the original `assert_door_construction`'s own doc comment (before
    /// this generalization) for why. Assumes rooms `a` and `b` are
    /// `ir.rooms[0]` and `ir.rooms[1]` respectively, true for every door
    /// portal this project builds.
    #[expect(
        clippy::too_many_lines,
        reason = "one coherent assertion helper covering every invariant the chain construction \
                  must hold; splitting it into smaller functions would just scatter the shared \
                  `referenced`/`door`/`ir` context across call boundaries for no real benefit, \
                  and test helpers are exempt from the same modularity pressure production code is \
                  under"
    )]
    fn assert_door_chain(
        ir_json: &str,
        door_corners: [Pt; 4],
        near_alcove_corners: Option<[Pt; 4]>,
        far_alcove_corners: Option<[Pt; 4]>,
    ) {
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

        let expected_new_sectors = 1
            + usize::from(near_alcove_corners.is_some())
            + usize::from(far_alcove_corners.is_some());
        assert_eq!(
            data.sectors.len(),
            ir.rooms.len() + expected_new_sectors,
            "emit_doors added exactly one sector per present chain component"
        );

        // The door sector itself.
        assert_eq!(
            referenced(door),
            4,
            "the door sector is a closed quadrilateral, not a dangling sidedef"
        );
        assert_eq!(
            data.sectors[door].ceiling, data.sectors[door].floor,
            "the door sector is closed"
        );
        for corner in door_corners {
            assert!(
                data.vertices.contains(&corner),
                "expected door corner {corner:?} is missing from the emitted vertices"
            );
        }
        let faces: Vec<_> = data.linedefs.iter().filter(|l| l.special != 0).collect();
        assert_eq!(
            faces.len(),
            2,
            "exactly the door's own two faces carry the special"
        );
        for face in &faces {
            let back = face.back.expect("a face is two-sided");
            assert_ne!(
                data.sidedefs[face.front].sector, door,
                "a face's front never names the door sector — only its back does"
            );
            assert_eq!(
                data.sidedefs[back].sector, door,
                "a face's back names the door sector, the one EV_VerticalDoor acts on"
            );
        }

        // Each present alcove, matched to its expected corners by bounding
        // box rather than assumed index order — `emit_doors` always pushes
        // the door last, but a near-only vs far-only alcove otherwise
        // occupies the same relative index.
        let alcove_sectors: Vec<usize> = (ir.rooms.len()..data.sectors.len())
            .filter(|&s| s != door)
            .collect();
        let expected_alcoves: Vec<[Pt; 4]> = [near_alcove_corners, far_alcove_corners]
            .into_iter()
            .flatten()
            .collect();
        assert_eq!(
            alcove_sectors.len(),
            expected_alcoves.len(),
            "alcove sector count matches the present alcove fields"
        );
        for corners in &expected_alcoves {
            let matched = alcove_sectors.iter().copied().find(|&s| {
                let (x_lo, x_hi, y_lo, y_hi) = sector_bbox(&data, s);
                corners.iter().all(|c| c.x == x_lo || c.x == x_hi)
                    && corners.iter().all(|c| c.y == y_lo || c.y == y_hi)
                    && corners
                        .iter()
                        .map(|c| c.x)
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == 2
                    && corners
                        .iter()
                        .map(|c| c.y)
                        .collect::<std::collections::HashSet<_>>()
                        .len()
                        == 2
            });
            let sector = matched
                .unwrap_or_else(|| panic!("no alcove sector matches expected corners {corners:?}"));
            assert_eq!(
                referenced(sector),
                4,
                "the alcove sector is a closed quadrilateral"
            );
            assert!(
                data.sectors[sector].ceiling > data.sectors[sector].floor,
                "an alcove is an open passage, unlike the closed door"
            );
        }

        // Room a/b keep their own plain-portal boundary shape.
        let edges_a = ir.rooms[0].footprint.len();
        let edges_b = ir.rooms[1].footprint.len();
        assert_eq!(
            referenced(0),
            edges_a + 2,
            "room a keeps its plain-portal boundary shape (unaffected by the door or its \
             alcoves)"
        );
        assert_eq!(
            referenced(1),
            edges_b + 2,
            "room b also keeps its plain-portal boundary shape (unaffected)"
        );

        assert_sector_boundaries_are_closed(&data);
        for l in &data.linedefs {
            let (p, q) = (data.vertices[l.v1], data.vertices[l.v2]);
            let front_sector = data.sidedefs[l.front].sector;
            assert!(
                chain_sector_interior_is_on_the_right(&ir, &data, front_sector, p, q),
                "front sidedef of line {p:?} -> {q:?} names sector {front_sector}, but that \
                 sector's interior is not on the right of travel"
            );
            if let Some(back) = l.back {
                let back_sector = data.sidedefs[back].sector;
                assert!(
                    chain_sector_interior_is_on_the_right(&ir, &data, back_sector, q, p),
                    "back sidedef of line {p:?} -> {q:?} names sector {back_sector}, but that \
                     sector's interior is not on the left of travel"
                );
            }
        }
    }

    // The four tests below cross all four wall orientations against a
    // different alcove configuration each — none, near-only, far-only, and
    // both — so the coverage grid is a genuine 4x4 crossing rather than
    // "four rectangles, four rotations, one config repeated". See the
    // door-redesign report's coverage grid for the full table.

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_west_of_a_vertical_wall() {
        // Worked example (no alcove): room a = [0,0]-[256,256], room b =
        // [288,0]-[544,256], facing wall pair at x=256 (room a) / x=288
        // (room b), portal width 128 at (256,128) -> open span y in
        // [64,192]. Gap 32 == door_thickness 32 exactly, so the door alone
        // fills it.
        assert_door_chain(
            DOOR_IR,
            [
                Pt { x: 256, y: 64 },
                Pt { x: 256, y: 192 },
                Pt { x: 288, y: 64 },
                Pt { x: 288, y: 192 },
            ],
            None,
            None,
        );
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_east_of_a_vertical_wall() {
        // Near-alcove-only: room a's own wall stays at x=256 (west edge,
        // since room a sits east of the gap); room b's own wall sits at
        // x=224, a legal 32-unit gap away (thickness 16 + alcove_near 16).
        // The alcove sits against room a (x in [240,256]); the door sits
        // against room b (x in [224,240]).
        let ir_json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[256,0],[256,256],[512,256],[512,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[-32,0],[-32,256],[224,256],[224,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                        "door_thickness":16, "alcove_near":16 }] }"#;
        assert_door_chain(
            ir_json,
            [
                Pt { x: 240, y: 64 },
                Pt { x: 240, y: 192 },
                Pt { x: 224, y: 64 },
                Pt { x: 224, y: 192 },
            ],
            Some([
                Pt { x: 256, y: 64 },
                Pt { x: 256, y: 192 },
                Pt { x: 240, y: 64 },
                Pt { x: 240, y: 192 },
            ]),
            None,
        );
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_south_of_a_horizontal_wall() {
        // Far-alcove-only: room a's own wall stays at y=256; room b's own
        // wall sits at y=296, a legal 40-unit gap away (thickness 8 +
        // alcove_far 32). The door sits against room a (y in [256,264]);
        // the alcove sits against room b (y in [264,296]).
        let ir_json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,296],[0,552],[256,552],[256,296]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[128,256],
                        "door_thickness":8, "alcove_far":32 }] }"#;
        assert_door_chain(
            ir_json,
            [
                Pt { x: 64, y: 256 },
                Pt { x: 192, y: 256 },
                Pt { x: 64, y: 264 },
                Pt { x: 192, y: 264 },
            ],
            None,
            Some([
                Pt { x: 64, y: 264 },
                Pt { x: 192, y: 264 },
                Pt { x: 64, y: 296 },
                Pt { x: 192, y: 296 },
            ]),
        );
    }

    #[test]
    fn a_door_fills_the_gap_when_room_a_is_north_of_a_horizontal_wall() {
        // Both alcoves: room a's own wall stays at y=256; room b's own wall
        // sits at y=192, a legal 64-unit gap away (thickness 32 + alcove_near
        // 16 + alcove_far 16). The near alcove sits against room a (y in
        // [240,256]), the door in the middle (y in [208,240]), the far
        // alcove against room b (y in [192,208]).
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,256],[0,512],[256,512],[256,256]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[0,-64],[0,192],[256,192],[256,-64]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[128,256],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;
        assert_door_chain(
            ir_json,
            [
                Pt { x: 64, y: 240 },
                Pt { x: 192, y: 240 },
                Pt { x: 64, y: 208 },
                Pt { x: 192, y: 208 },
            ],
            Some([
                Pt { x: 64, y: 256 },
                Pt { x: 192, y: 256 },
                Pt { x: 64, y: 240 },
                Pt { x: 192, y: 240 },
            ]),
            Some([
                Pt { x: 64, y: 208 },
                Pt { x: 192, y: 208 },
                Pt { x: 64, y: 192 },
                Pt { x: 192, y: 192 },
            ]),
        );
    }

    /// A single-corner chamfer (pentagon), like `portals::tests::CHAMFERED_ROOM_WITH_PORTAL`
    /// but with a door instead of a plain portal, and a no-alcove config —
    /// the "shape" axis of the coverage grid crossed with the config the
    /// four orientation tests above already establish, rather than repeating
    /// one shape four times.
    #[test]
    fn a_door_works_on_the_axis_aligned_wall_of_a_chamfered_room() {
        let ir_json = r#"{ "seed":1, "grid":8, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[192,256],[256,192],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[-288,0],[-288,256],[-32,256],[-32,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[0,128],
                        "door_thickness":32 }] }"#;
        assert_door_chain(
            ir_json,
            [
                Pt { x: 0, y: 64 },
                Pt { x: 0, y: 192 },
                Pt { x: -32, y: 64 },
                Pt { x: -32, y: 192 },
            ],
            None,
            None,
        );
    }

    #[test]
    fn a_door_works_at_the_minimum_legal_gap() {
        // The wall gap is exactly `Ir::MIN_PORTAL_GAP` (8) — the tightest
        // legal thickness a door can sit in, filled by the door alone (no
        // alcove fits in 8 units, since the smallest legal alcove is itself
        // 8). Fine grid (4) since a 64-unit grid cannot express an 8-unit
        // gap at all.
        let ir_json = r#"{ "seed":1, "grid":4, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,64],[64,64],[64,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
            { "id":"b", "footprint":[[72,0],[72,64],[136,64],[136,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":32, "at":[64,32],
                        "door_thickness":8 }] }"#;
        assert_door_chain(
            ir_json,
            [
                Pt { x: 64, y: 16 },
                Pt { x: 64, y: 48 },
                Pt { x: 72, y: 16 },
                Pt { x: 72, y: 48 },
            ],
            None,
            None,
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
            { "a":"left", "b":"middle", "kind":"door", "width":32, "at":[64,32],
              "door_thickness":8 },
            { "a":"right", "b":"middle", "kind":"door", "width":32, "at":[104,32],
              "door_thickness":8 }
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
        cut_portals(&ir, &tables, &mut data).expect("portals");
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
        cut_portals(&ir, &tables, &mut data).expect("portals");
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
      "portals":[{ "a":"a", "b":"b", "kind":"door", "width":64, "at":[0,128],
                    "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;

    #[test]
    fn a_door_into_an_octagonal_room_on_its_axis_aligned_wall_works() {
        // The door sits on the octagon's west wall (x = 64, y in 64..192,
        // away from every chamfer), flanked by both alcoves — the richest
        // chain (all three segments) crossed with this crate's one
        // non-rectangular door-adjacent room shape. Routed through
        // `assert_door_chain` so the full sidedef-facing invariant runs
        // against a genuinely diagonal-edged room `b`, not merely a
        // vertex-membership check.
        assert_door_chain(
            OCTAGON_ROOM_B,
            [
                Pt { x: 16, y: 96 },
                Pt { x: 16, y: 160 },
                Pt { x: 48, y: 96 },
                Pt { x: 48, y: 160 },
            ],
            Some([
                Pt { x: 0, y: 96 },
                Pt { x: 0, y: 160 },
                Pt { x: 16, y: 96 },
                Pt { x: 16, y: 160 },
            ]),
            Some([
                Pt { x: 48, y: 96 },
                Pt { x: 48, y: 160 },
                Pt { x: 64, y: 96 },
                Pt { x: 64, y: 160 },
            ]),
        );
    }

    /// Two right triangles splitting a 64-unit square along its own
    /// diagonal, exactly like `portals::tests::DIAGONAL_TWIN_TRIANGLES`, but
    /// with `"kind":"door"` instead of `"plain"`. Carries a `door_thickness`
    /// even though the diagonal wall means no facing span is ever found
    /// (`Ir::validate_door_gap` skips a portal whose `at` matches no span,
    /// same as `Ir::validate_portal_gaps`) — `Ir::validate_door_dimensions`
    /// still requires the field unconditionally, since a missing thickness
    /// is wrong regardless of where the portal sits.
    const DOOR_ON_DIAGONAL_WALL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,64],[64,64]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" },
        { "id":"b", "footprint":[[0,0],[64,64],[64,0]],
           "floor":0, "ceiling":128, "light":160,
           "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
      ],
      "portals":[{ "a":"a", "b":"b", "kind":"door", "width":16, "at":[32,32],
                    "door_thickness":16 }] }"#;

    #[test]
    fn a_door_portal_requested_on_a_diagonal_wall_is_rejected_before_any_sector_is_built() {
        // In the real pipeline `cut_portals` resolves every portal — door or
        // plain — before anything is cut, so this is what an author
        // actually sees: the diagonal-wall check `portals::tests` pins for a
        // plain portal must reach a door portal too, not just fall through
        // to `NotAdjacent` because the portal happened to be a door.
        let ir = Ir::from_json(DOOR_ON_DIAGONAL_WALL).expect("ir");
        let tables = Tables::load().expect("tables");
        let mut data = emit_sectors(&ir).expect("sectors");
        assert!(
            matches!(
                cut_portals(&ir, &tables, &mut data),
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
