//! Loads the sourced engine-constant and vocabulary tables.

use std::collections::HashMap;

use serde::Deserialize;

/// The radius and height of a thing, in map units.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ThingDims {
    /// Collision radius in map units.
    pub radius: i32,
    /// Collision height in map units.
    pub height: i32,
}

#[derive(Debug, Deserialize)]
struct Movement {
    max_step_height: i32,
}

#[derive(Debug, Deserialize)]
struct Door {
    clearance_allowance: i32,
}

#[derive(Debug, Deserialize)]
struct LightRange {
    min: i32,
    max: i32,
}

#[derive(Debug, Deserialize)]
struct TextureSet {
    wall: String,
    floor: String,
    ceiling: String,
    door: String,
    door_track: String,
}

#[derive(Debug, Deserialize)]
struct Engine {
    movement: Movement,
    door: Door,
    light: LightRange,
    player: ThingDims,
    species: HashMap<String, ThingDims>,
}

#[derive(Debug, Deserialize)]
struct Vocabulary {
    things: HashMap<String, toml::Value>,
    textures: HashMap<String, TextureSet>,
}

/// Errors raised while loading the data tables.
#[derive(Debug, thiserror::Error)]
pub enum TableError {
    /// A table could not be parsed as TOML.
    #[error("{file}: {source}")]
    Parse {
        /// The table that failed to parse.
        file: &'static str,
        /// The underlying TOML error.
        source: toml::de::Error,
    },
}

/// The engine constants and vocabulary, loaded together.
#[derive(Debug)]
pub struct Tables {
    engine: Engine,
    vocabulary: Vocabulary,
}

impl Tables {
    /// Loads both tables, which are embedded at compile time.
    ///
    /// # Errors
    /// Returns [`TableError::Parse`] if either table is not valid TOML or is
    /// missing a required field.
    pub fn load() -> Result<Self, TableError> {
        let engine: Engine =
            toml::from_str(include_str!("../data/engine.toml")).map_err(|source| {
                TableError::Parse {
                    file: "engine.toml",
                    source,
                }
            })?;
        let vocabulary: Vocabulary = toml::from_str(include_str!("../data/vocabulary.toml"))
            .map_err(|source| TableError::Parse {
                file: "vocabulary.toml",
                source,
            })?;
        Ok(Self { engine, vocabulary })
    }

    /// The maximum height the player can step up, in map units.
    #[must_use]
    pub fn step_height(&self) -> i32 {
        self.engine.movement.max_step_height
    }

    /// The player's collision dimensions.
    #[must_use]
    pub fn player(&self) -> ThingDims {
        self.engine.player
    }

    /// How far a door's open ceiling falls short of the lowest neighboring
    /// ceiling, in map units.
    #[must_use]
    pub fn door_clearance_allowance(&self) -> i32 {
        self.engine.door.clearance_allowance
    }

    /// The inclusive range of valid sector light levels.
    #[must_use]
    pub fn light_range(&self) -> std::ops::RangeInclusive<i32> {
        self.engine.light.min..=self.engine.light.max
    }

    /// The collision dimensions of a named monster species, if listed.
    #[must_use]
    pub fn species(&self, name: &str) -> Option<ThingDims> {
        self.engine.species.get(name).copied()
    }

    /// The concrete thing ID for a high-level name, if listed.
    #[must_use]
    pub fn thing_id(&self, name: &str) -> Option<u16> {
        self.vocabulary
            .things
            .get(name)?
            .as_integer()?
            .try_into()
            .ok()
    }

    /// The texture for a role (`wall`, `floor`, `ceiling`, `door`,
    /// `door_track`) under a theme, if both resolve.
    #[must_use]
    pub fn texture(&self, role: &str, theme: &str) -> Option<&str> {
        let set = self.vocabulary.textures.get(theme)?;
        match role {
            "wall" => Some(&set.wall),
            "floor" => Some(&set.floor),
            "ceiling" => Some(&set.ceiling),
            "door" => Some(&set.door),
            "door_track" => Some(&set.door_track),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Tables;

    #[test]
    fn loads_and_resolves_known_entries() {
        let t = Tables::load().expect("tables load");
        assert!(t.step_height() > 0, "step height is positive");
        assert!(t.player().height > 0, "player height is positive");
        assert_eq!(t.thing_id("player1_start"), Some(1));
        assert!(t.species("imp").is_some(), "imp is a known species");
        assert!(
            t.species("archvile").is_none(),
            "unlisted species is absent"
        );
        assert!(
            t.texture("wall", "tech_base").is_some(),
            "theme wall texture resolves"
        );
        assert!(t.door_clearance_allowance() >= 0, "door allowance loads");
        assert!(
            t.light_range().contains(&128),
            "a mid light level is in range"
        );
    }
}
