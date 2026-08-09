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

/// A thing placed inside a room.
#[derive(Debug, Clone, Deserialize)]
pub struct IrThing {
    /// Vocabulary name, resolved to a thing ID at compile time.
    pub kind: String,
    /// Position in map units.
    pub at: Pt,
    /// Facing angle in degrees.
    pub angle: u16,
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
    #[serde(default)]
    pub special: Option<u16>,
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
}

impl Ir {
    /// Parses and validates an IR document.
    ///
    /// # Errors
    /// Returns [`IrError::Json`] for malformed input, [`IrError::DuplicateRoom`]
    /// for a repeated room id, [`IrError::UnknownRoom`] for a portal naming a
    /// room that does not exist, and [`IrError::OffGrid`] for any coordinate
    /// that is not a multiple of `grid`.
    pub fn from_json(s: &str) -> Result<Self, IrError> {
        let ir: Self = serde_json::from_str(s)?;

        // Before anything divides by it: `x % 0` panics in Rust, and this is
        // the untrusted-input boundary, so it must return an error instead.
        if ir.grid <= 0 {
            return Err(IrError::InvalidGrid { grid: ir.grid });
        }

        let mut seen = HashSet::new();
        for room in &ir.rooms {
            if !seen.insert(room.id.as_str()) {
                return Err(IrError::DuplicateRoom {
                    id: room.id.clone(),
                });
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
            }
        }

        for portal in &ir.portals {
            for id in [&portal.a, &portal.b] {
                if !seen.contains(id.as_str()) {
                    return Err(IrError::UnknownRoom { id: id.clone() });
                }
            }
        }

        Ok(ir)
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
}
