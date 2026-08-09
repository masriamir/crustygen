//! The room-graph intermediate representation.

use std::collections::HashSet;

use serde::Deserialize;

use crate::geom::Pt;

impl<'de> Deserialize<'de> for Pt {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let [x, y] = <[i32; 2]>::deserialize(d)?;
        Ok(Self { x, y })
    }
}

/// How two rooms connect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortalKind {
    /// An open doorway with no door sector.
    Plain,
    /// A manual door with its own sector.
    Door,
    /// A door requiring a key.
    Locked,
}

/// Returns `true`, for use as a serde field default so an absent
/// `skill1..skill5` key in a partially specified [`ThingSkills`] object
/// means "appears on this skill" rather than "excluded from it".
const fn skill_default() -> bool {
    true
}

/// Which of Doom's five skill levels a thing appears on.
///
/// UDMF's `doom` namespace spells these as five independent booleans
/// (`skill1`..`skill5`, ITYTD through Nightmare); this mirrors that shape
/// directly rather than packing them, matching the convention
/// [`crate::compile::textmap::emit_textmap`] already uses for other
/// per-object flags. Every field defaults to `true`, both at the whole-struct
/// level (an [`IrThing`] with no `skills` key at all) and per-field (a
/// `skills` object that names only some keys) — so a thing that never
/// mentions skills keeps the compiler's original "appears on every skill"
/// behavior, and every fixture written before this field existed is
/// unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these five booleans are not independent flags to model as a state machine — they \
              are a direct, one-to-one mirror of UDMF's own skill1..skill5 fields, and any other \
              shape would need translating back and forth at the emission boundary for no benefit"
)]
pub struct ThingSkills {
    /// UDMF `skill1` — I'm Too Young To Die.
    #[serde(default = "skill_default")]
    pub skill1: bool,
    /// UDMF `skill2` — Hey, Not Too Rough.
    #[serde(default = "skill_default")]
    pub skill2: bool,
    /// UDMF `skill3` — Hurt Me Plenty.
    #[serde(default = "skill_default")]
    pub skill3: bool,
    /// UDMF `skill4` — Ultra-Violence.
    #[serde(default = "skill_default")]
    pub skill4: bool,
    /// UDMF `skill5` — Nightmare!.
    #[serde(default = "skill_default")]
    pub skill5: bool,
}

impl Default for ThingSkills {
    /// All five skills — the behavior every fixture predating this type saw.
    fn default() -> Self {
        Self {
            skill1: true,
            skill2: true,
            skill3: true,
            skill4: true,
            skill5: true,
        }
    }
}

/// A thing placed inside a room.
#[derive(Debug, Clone, Deserialize)]
pub struct IrThing {
    /// Vocabulary name, resolved to a thing ID at compile time.
    pub kind: String,
    /// Position in map units.
    pub at: Pt,
    /// Facing angle in degrees.
    pub angle: u16,
    /// Which skill levels this thing appears on. Defaults to all five when
    /// the key is absent — see [`ThingSkills`].
    #[serde(default)]
    pub skills: ThingSkills,
}

/// One room: a closed footprint plus its surfaces and contents.
#[derive(Debug, Clone, Deserialize)]
pub struct Room {
    /// Unique identifier, used by portals and error messages.
    pub id: String,
    /// Clockwise, grid-snapped boundary.
    pub footprint: Vec<Pt>,
    /// Floor height in map units.
    pub floor: i32,
    /// Ceiling height in map units.
    pub ceiling: i32,
    /// Sector light level.
    pub light: i32,
    /// Floor flat name.
    pub floor_tex: String,
    /// Ceiling flat name.
    pub ceil_tex: String,
    /// Wall texture name.
    pub wall_tex: String,
    /// Sector special, if any.
    ///
    /// This is the escape hatch: an author who sets it directly gets exactly
    /// that raw value, with no interpretation. It is mutually exclusive with
    /// [`Self::secret`] — [`Ir::from_json`] rejects a room that sets both,
    /// rather than picking a silent precedence between them (the same
    /// reject-don't-degrade posture [`Portal::width`] already takes on an odd
    /// width). Use `secret` for the common case; use `special` directly for
    /// anything `secret` cannot express.
    #[serde(default)]
    pub special: Option<u16>,
    /// Whether this sector carries rule P18's secret special
    /// (`Tables::secret_sector_special`) rather than a plain 0.
    ///
    /// This is the high-level path: it spares an author from writing the raw
    /// engine-sourced special number into [`Self::special`] by hand. See
    /// that field's doc comment for how the two interact.
    #[serde(default)]
    pub secret: bool,
    /// Things placed in this room.
    #[serde(default)]
    pub things: Vec<IrThing>,
}

/// A connection between two rooms.
///
/// `a` and `b` are not interchangeable for [`PortalKind::Door`] and
/// [`PortalKind::Locked`]: the compiler always carves the door sector's
/// recess out of room `b`'s side of the shared wall, leaving room `a`'s
/// boundary untouched. Swapping `a` and `b` on such a portal physically
/// relocates the door to the opposite room, not just its label.
#[derive(Debug, Clone, Deserialize)]
pub struct Portal {
    /// Identifier of the first room.
    pub a: String,
    /// Identifier of the second room. For a door portal, this is the room
    /// the compiler recesses to make room for the door sector — see the
    /// struct-level note.
    pub b: String,
    /// The kind of connection.
    pub kind: PortalKind,
    /// Key name when `kind` is [`PortalKind::Locked`].
    #[serde(default)]
    pub lock: Option<String>,
    /// Clear opening width in map units.
    ///
    /// Must be positive and even: the opening is centered on [`Self::at`],
    /// so an odd width could not be split into two equal integer halves, and
    /// a zero or negative one would emit a degenerate or inverted opening.
    /// [`Ir::from_json`] rejects both.
    pub width: i32,
    /// Midpoint of the opening on the shared wall.
    pub at: Pt,
}

/// A complete room graph.
#[derive(Debug, Clone, Deserialize)]
pub struct Ir {
    /// Seed recorded for reproducibility.
    pub seed: u64,
    /// Grid size every coordinate snaps to.
    pub grid: i32,
    /// Theme name, resolved against the vocabulary table.
    pub theme: String,
    /// The rooms.
    pub rooms: Vec<Room>,
    /// The connections between rooms.
    pub portals: Vec<Portal>,
}

/// Errors raised while loading or validating an IR document.
#[derive(Debug, thiserror::Error)]
pub enum IrError {
    /// The document is not valid JSON, or does not match the schema.
    #[error("invalid IR JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// Two rooms share an identifier.
    #[error("duplicate room id `{id}`")]
    DuplicateRoom {
        /// The repeated identifier.
        id: String,
    },
    /// A portal names a room that does not exist.
    #[error("portal references unknown room `{id}`")]
    UnknownRoom {
        /// The unresolvable identifier.
        id: String,
    },
    /// The document declares a grid size that is zero or negative.
    #[error("grid must be positive, got {grid}")]
    InvalidGrid {
        /// The rejected grid size.
        grid: i32,
    },
    /// A coordinate is not a multiple of the grid.
    #[error("room `{room}` has an off-grid coordinate ({x}, {y}); grid is {grid}")]
    OffGrid {
        /// The offending room.
        room: String,
        /// The X coordinate.
        x: i32,
        /// The Y coordinate.
        y: i32,
        /// The configured grid size.
        grid: i32,
    },
    /// A coordinate does not fit the 16-bit vertex range every Doom map
    /// format uses.
    #[error("`{subject}` has a coordinate ({x}, {y}) outside the map range {min}..={max}")]
    CoordinateOutOfRange {
        /// The room or portal that carries it.
        subject: String,
        /// The X coordinate.
        x: i32,
        /// The Y coordinate.
        y: i32,
        /// The lowest representable coordinate.
        min: i32,
        /// The highest representable coordinate.
        max: i32,
    },
    /// A room's ceiling is not above its floor, so it encloses no volume.
    #[error("room `{room}` has ceiling {ceiling} at or below floor {floor}")]
    InvertedRoom {
        /// The offending room.
        room: String,
        /// Its declared floor height.
        floor: i32,
        /// Its declared ceiling height.
        ceiling: i32,
    },
    /// A room height does not fit the 16-bit range every Doom map format
    /// uses for sector planes.
    #[error("room `{room}` has a height {height} outside the map range {min}..={max}")]
    HeightOutOfRange {
        /// The offending room.
        room: String,
        /// The rejected floor or ceiling height.
        height: i32,
        /// The lowest representable height.
        min: i32,
        /// The highest representable height.
        max: i32,
    },
    /// A portal declares a width that is zero or negative.
    #[error("portal `{a}` <-> `{b}` has width {width}, which must be positive")]
    InvalidPortalWidth {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected width.
        width: i32,
    },
    /// A portal declares an odd width, which cannot be centered on `at`
    /// without landing half a unit off the integer grid.
    #[error("portal `{a}` <-> `{b}` has odd width {width}; widths must be even")]
    OddPortalWidth {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
        /// The rejected width.
        width: i32,
    },
    /// A [`PortalKind::Locked`] portal does not name the key that opens it.
    #[error("portal `{a}` <-> `{b}` is locked but names no key")]
    LockedWithoutKey {
        /// The first room.
        a: String,
        /// The second room.
        b: String,
    },
    /// A room sets both [`Room::secret`] and an explicit [`Room::special`],
    /// leaving no way to tell which one the author actually wants.
    #[error("room `{room}` sets both `secret` and an explicit `special`; use one or the other")]
    SecretWithExplicitSpecial {
        /// The offending room.
        room: String,
    },
}

/// The inclusive coordinate and height range every Doom map format stores in
/// a signed 16-bit field.
///
/// UDMF itself is textual and would accept wider values, but the design's
/// second output is a Doom binary WAD produced by `cwad convert --to doom`
/// (see the spec's architecture diagram), whose `VERTEXES` and `SECTORS`
/// lumps are `i16`. Rejecting here keeps the two outputs equivalent instead
/// of letting the binary one silently wrap.
const MAP_RANGE: std::ops::RangeInclusive<i32> = (i16::MIN as i32)..=(i16::MAX as i32);

impl Ir {
    /// Parses and validates an IR document.
    ///
    /// Every numeric field a later pass divides by, halves, or writes into a
    /// fixed-width map record is bounds-checked here, at the untrusted-input
    /// boundary, so no downstream pass has to defend itself: `grid`
    /// ([`IrError::InvalidGrid`]) because `x % 0` panics, `width`
    /// ([`IrError::InvalidPortalWidth`], [`IrError::OddPortalWidth`]) because
    /// a zero width emits a zero-length two-sided linedef and a negative one
    /// inverts the opening, room heights ([`IrError::InvertedRoom`]) because
    /// a ceiling at or below its floor encloses no playable volume, and both
    /// coordinates and heights ([`IrError::CoordinateOutOfRange`],
    /// [`IrError::HeightOutOfRange`]) because the binary Doom output stores
    /// them in `i16`.
    ///
    /// # Errors
    /// Returns [`IrError::Json`] for malformed input, [`IrError::DuplicateRoom`]
    /// for a repeated room id, [`IrError::UnknownRoom`] for a portal naming a
    /// room that does not exist, [`IrError::OffGrid`] for any coordinate that
    /// is not a multiple of `grid`, [`IrError::LockedWithoutKey`] for a locked
    /// portal that names no key, [`IrError::SecretWithExplicitSpecial`] for a
    /// room that sets both `secret` and `special`, and the numeric-range
    /// variants listed above.
    pub fn from_json(s: &str) -> Result<Self, IrError> {
        let ir: Self = serde_json::from_str(s)?;

        // Before anything divides by it: `x % 0` panics in Rust, and this is
        // the untrusted-input boundary, so it must return an error instead.
        if ir.grid <= 0 {
            return Err(IrError::InvalidGrid { grid: ir.grid });
        }

        let seen = Self::validate_rooms(&ir)?;
        Self::validate_portals(&ir, &seen)?;

        Ok(ir)
    }

    /// Validates every room and returns the set of ids seen, for
    /// [`Self::validate_portals`] to check its own room references against.
    fn validate_rooms(ir: &Self) -> Result<HashSet<&str>, IrError> {
        let mut seen = HashSet::new();
        for room in &ir.rooms {
            if !seen.insert(room.id.as_str()) {
                return Err(IrError::DuplicateRoom {
                    id: room.id.clone(),
                });
            }
            if room.ceiling <= room.floor {
                return Err(IrError::InvertedRoom {
                    room: room.id.clone(),
                    floor: room.floor,
                    ceiling: room.ceiling,
                });
            }
            if room.secret && room.special.is_some() {
                return Err(IrError::SecretWithExplicitSpecial {
                    room: room.id.clone(),
                });
            }
            for height in [room.floor, room.ceiling] {
                if !MAP_RANGE.contains(&height) {
                    return Err(IrError::HeightOutOfRange {
                        room: room.id.clone(),
                        height,
                        min: *MAP_RANGE.start(),
                        max: *MAP_RANGE.end(),
                    });
                }
            }
            for p in &room.footprint {
                if p.x % ir.grid != 0 || p.y % ir.grid != 0 {
                    return Err(IrError::OffGrid {
                        room: room.id.clone(),
                        x: p.x,
                        y: p.y,
                        grid: ir.grid,
                    });
                }
                if !MAP_RANGE.contains(&p.x) || !MAP_RANGE.contains(&p.y) {
                    return Err(IrError::CoordinateOutOfRange {
                        subject: room.id.clone(),
                        x: p.x,
                        y: p.y,
                        min: *MAP_RANGE.start(),
                        max: *MAP_RANGE.end(),
                    });
                }
            }
        }
        Ok(seen)
    }

    /// Validates every portal against the room ids [`Self::validate_rooms`]
    /// already collected.
    fn validate_portals(ir: &Self, seen: &HashSet<&str>) -> Result<(), IrError> {
        for portal in &ir.portals {
            for id in [&portal.a, &portal.b] {
                if !seen.contains(id.as_str()) {
                    return Err(IrError::UnknownRoom { id: id.clone() });
                }
            }
            if portal.width <= 0 {
                return Err(IrError::InvalidPortalWidth {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    width: portal.width,
                });
            }
            if portal.width % 2 != 0 {
                return Err(IrError::OddPortalWidth {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                    width: portal.width,
                });
            }
            if !MAP_RANGE.contains(&portal.at.x) || !MAP_RANGE.contains(&portal.at.y) {
                return Err(IrError::CoordinateOutOfRange {
                    subject: format!("{} <-> {}", portal.a, portal.b),
                    x: portal.at.x,
                    y: portal.at.y,
                    min: *MAP_RANGE.start(),
                    max: *MAP_RANGE.end(),
                });
            }
            if portal.kind == PortalKind::Locked && portal.lock.is_none() {
                return Err(IrError::LockedWithoutKey {
                    a: portal.a.clone(),
                    b: portal.b.clone(),
                });
            }
        }
        Ok(())
    }

    /// Looks up a room by identifier.
    #[must_use]
    pub fn room(&self, id: &str) -> Option<&Room> {
        self.rooms.iter().find(|r| r.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::{Ir, IrError, PortalKind};

    const TWO_ROOM: &str = r#"{
      "seed": 1, "grid": 64, "theme": "tech_base",
      "rooms": [
        { "id": "a", "footprint": [[0,0],[0,256],[256,256],[256,0]],
          "floor": 0, "ceiling": 128, "light": 160,
          "floor_tex": "FLOOR4_8", "ceil_tex": "CEIL3_5", "wall_tex": "STARTAN3",
          "things": [{ "kind": "player1_start", "at": [128,128], "angle": 90 }] },
        { "id": "b", "footprint": [[256,0],[256,256],[512,256],[512,0]],
          "floor": 0, "ceiling": 128, "light": 160,
          "floor_tex": "FLOOR4_8", "ceil_tex": "CEIL3_5", "wall_tex": "STARTAN3",
          "things": [] }
      ],
      "portals": [
        { "a": "a", "b": "b", "kind": "plain", "width": 128, "at": [256,128] }
      ]
    }"#;

    #[test]
    fn parses_a_two_room_graph() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir parses");
        assert_eq!(ir.rooms.len(), 2);
        assert_eq!(ir.portals.len(), 1);
        assert_eq!(ir.portals[0].kind, PortalKind::Plain);
        assert_eq!(ir.room("b").expect("room b").floor, 0);
        assert_eq!(ir.rooms[0].things[0].kind, "player1_start");
    }

    #[test]
    fn rejects_duplicate_room_ids() {
        let dup = TWO_ROOM.replace("\"id\": \"b\"", "\"id\": \"a\"");
        assert!(matches!(
            Ir::from_json(&dup),
            Err(IrError::DuplicateRoom { .. })
        ));
    }

    #[test]
    fn rejects_a_portal_naming_an_unknown_room() {
        let bad = TWO_ROOM.replace("\"b\": \"b\"", "\"b\": \"ghost\"");
        assert!(matches!(
            Ir::from_json(&bad),
            Err(IrError::UnknownRoom { .. })
        ));
    }

    #[test]
    fn rejects_off_grid_coordinates() {
        let off = TWO_ROOM.replace("[0,256]", "[0,250]");
        assert!(matches!(Ir::from_json(&off), Err(IrError::OffGrid { .. })));
    }

    #[test]
    fn rejects_a_non_positive_grid_instead_of_panicking() {
        // `x % 0` panics in Rust, so this must be caught before the room walk.
        let zero = TWO_ROOM.replace("\"grid\": 64", "\"grid\": 0");
        assert!(matches!(
            Ir::from_json(&zero),
            Err(IrError::InvalidGrid { grid: 0 })
        ));
        let negative = TWO_ROOM.replace("\"grid\": 64", "\"grid\": -64");
        assert!(matches!(
            Ir::from_json(&negative),
            Err(IrError::InvalidGrid { grid: -64 })
        ));
    }

    #[test]
    fn rejects_malformed_json() {
        assert!(matches!(Ir::from_json("{ not json"), Err(IrError::Json(_))));
    }

    #[test]
    fn rejects_a_zero_or_negative_portal_width() {
        // Width 0 emits a two-sided linedef whose two vertices are the same
        // point; a negative width inverts the opening so `open_lo > open_hi`.
        // Both compiled clean before this guard existed — the same class of
        // hole as the unguarded `grid` above.
        let zero = TWO_ROOM.replace("\"width\": 128", "\"width\": 0");
        assert!(matches!(
            Ir::from_json(&zero),
            Err(IrError::InvalidPortalWidth { width: 0, .. })
        ));
        let negative = TWO_ROOM.replace("\"width\": 128", "\"width\": -64");
        assert!(matches!(
            Ir::from_json(&negative),
            Err(IrError::InvalidPortalWidth { width: -64, .. })
        ));
    }

    #[test]
    fn rejects_an_odd_portal_width() {
        // `width / 2` truncates, so an odd width silently emitted a span one
        // unit narrower than the value P3 validates.
        let odd = TWO_ROOM.replace("\"width\": 128", "\"width\": 63");
        assert!(matches!(
            Ir::from_json(&odd),
            Err(IrError::OddPortalWidth { width: 63, .. })
        ));
    }

    #[test]
    fn rejects_a_room_whose_ceiling_is_not_above_its_floor() {
        let flat = TWO_ROOM.replace("\"ceiling\": 128", "\"ceiling\": 0");
        assert!(matches!(
            Ir::from_json(&flat),
            Err(IrError::InvertedRoom {
                floor: 0,
                ceiling: 0,
                ..
            })
        ));
        let inverted = TWO_ROOM.replace("\"ceiling\": 128", "\"ceiling\": -8");
        assert!(matches!(
            Ir::from_json(&inverted),
            Err(IrError::InvertedRoom { ceiling: -8, .. })
        ));
    }

    #[test]
    fn rejects_coordinates_and_heights_outside_the_binary_map_range() {
        let far = TWO_ROOM.replace("[512,256]", "[65536,256]");
        assert!(matches!(
            Ir::from_json(&far),
            Err(IrError::CoordinateOutOfRange { x: 65536, .. })
        ));
        let high = TWO_ROOM.replace("\"ceiling\": 128", "\"ceiling\": 40000");
        assert!(matches!(
            Ir::from_json(&high),
            Err(IrError::HeightOutOfRange { height: 40000, .. })
        ));
        let off_wall = TWO_ROOM.replace("\"at\": [256,128]", "\"at\": [256,40000]");
        assert!(matches!(
            Ir::from_json(&off_wall),
            Err(IrError::CoordinateOutOfRange { y: 40000, .. })
        ));
    }

    #[test]
    fn rejects_a_locked_portal_that_names_no_key() {
        let locked = TWO_ROOM.replace("\"kind\": \"plain\"", "\"kind\": \"locked\"");
        assert!(matches!(
            Ir::from_json(&locked),
            Err(IrError::LockedWithoutKey { .. })
        ));
        let keyed = TWO_ROOM.replace(
            "\"kind\": \"plain\"",
            "\"kind\": \"locked\", \"lock\": \"blue_card\"",
        );
        assert!(Ir::from_json(&keyed).is_ok(), "a named key is accepted");
    }

    #[test]
    fn rejects_a_room_that_sets_both_secret_and_an_explicit_special() {
        let json = TWO_ROOM.replace(
            "\"id\": \"a\", \"footprint\"",
            "\"id\": \"a\", \"secret\": true, \"special\": 9, \"footprint\"",
        );
        assert!(matches!(
            Ir::from_json(&json),
            Err(IrError::SecretWithExplicitSpecial { .. })
        ));
    }

    #[test]
    fn a_room_may_set_secret_alone_or_special_alone() {
        let secret_only = TWO_ROOM.replace(
            "\"id\": \"a\", \"footprint\"",
            "\"id\": \"a\", \"secret\": true, \"footprint\"",
        );
        assert!(Ir::from_json(&secret_only).is_ok(), "secret alone is fine");

        let special_only = TWO_ROOM.replace(
            "\"id\": \"a\", \"footprint\"",
            "\"id\": \"a\", \"special\": 9, \"footprint\"",
        );
        assert!(
            Ir::from_json(&special_only).is_ok(),
            "special alone is fine"
        );
    }

    #[test]
    fn a_thing_with_no_skills_key_defaults_to_all_five() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let skills = ir.rooms[0].things[0].skills;
        assert!(skills.skill1 && skills.skill2 && skills.skill3 && skills.skill4 && skills.skill5);
    }

    #[test]
    fn a_thing_may_partially_specify_skills_defaulting_the_rest_true() {
        let json = TWO_ROOM.replace(
            "\"kind\": \"player1_start\", \"at\": [128,128], \"angle\": 90",
            "\"kind\": \"player1_start\", \"at\": [128,128], \"angle\": 90, \
             \"skills\": { \"skill1\": false, \"skill5\": false }",
        );
        let ir = Ir::from_json(&json).expect("ir");
        let skills = ir.rooms[0].things[0].skills;
        assert!(!skills.skill1, "explicitly excluded");
        assert!(
            skills.skill2 && skills.skill3 && skills.skill4,
            "unmentioned keys default true"
        );
        assert!(!skills.skill5, "explicitly excluded");
    }
}
