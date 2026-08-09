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

/// Sector specials keyed by tier name, as used for
/// `flats.liquid.damage_tier` (`light` | `medium` | `heavy`).
#[derive(Debug, Deserialize)]
struct DamageTiers {
    light: u16,
    medium: u16,
    heavy: u16,
}

/// Sector (not linedef) specials: a distinct numeric space from
/// `vocabulary.toml`'s `[specials]` table.
#[derive(Debug, Deserialize)]
struct SectorSpecials {
    secret: u16,
    damage: DamageTiers,
}

#[derive(Debug, Deserialize)]
struct Engine {
    movement: Movement,
    door: Door,
    light: LightRange,
    player: ThingDims,
    species: HashMap<String, ThingDims>,
    sector: SectorSpecials,
}

/// The linedef specials for the level exit, keyed by
/// `progression.exit.kind` (`normal` uses `switch`/`walkover`, `secret` uses
/// `secret_switch`/`secret_walkover`; `both` places one of each).
#[derive(Debug, Deserialize)]
struct ExitSpecials {
    switch: u16,
    walkover: u16,
    secret_switch: u16,
    secret_walkover: u16,
}

/// The linedef specials for a lift, keyed by `progression.lifts.trigger`.
#[derive(Debug, Deserialize)]
struct LiftSpecials {
    switch: u16,
    walkover: u16,
}

/// The linedef special for a teleporter line.
#[derive(Debug, Deserialize)]
struct TeleportSpecial {
    line: u16,
}

/// The linedef specials that open a door, keyed the same way the engine
/// keys them.
///
/// `locked` is a `toml::Value` map for the same reason `Vocabulary::things`
/// is: the table carries a `source` citation alongside its numbers, and a
/// citation is not a special.
#[derive(Debug, Deserialize)]
struct Specials {
    door: u16,
    locked: HashMap<String, toml::Value>,
    exit: ExitSpecials,
    lift: LiftSpecials,
    teleport: TeleportSpecial,
}

#[derive(Debug, Deserialize)]
struct Vocabulary {
    things: HashMap<String, toml::Value>,
    specials: Specials,
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

    /// The linedef special that opens a manual door.
    #[must_use]
    pub fn door_special(&self) -> u16 {
        self.vocabulary.specials.door
    }

    /// The linedef special that opens a door locked to the named key, if the
    /// vocabulary lists one.
    #[must_use]
    pub fn locked_door_special(&self, key: &str) -> Option<u16> {
        self.vocabulary
            .specials
            .locked
            .get(key)?
            .as_integer()?
            .try_into()
            .ok()
    }

    /// The linedef special for a switch-activated normal exit
    /// (`progression.exit.kind = normal`, `trigger = switch`).
    #[must_use]
    pub fn exit_switch_special(&self) -> u16 {
        self.vocabulary.specials.exit.switch
    }

    /// The linedef special for a walkover-activated normal exit
    /// (`progression.exit.kind = normal`, `trigger = walkover`).
    #[must_use]
    pub fn exit_walkover_special(&self) -> u16 {
        self.vocabulary.specials.exit.walkover
    }

    /// The linedef special for a switch-activated secret exit
    /// (`progression.exit.kind = secret`, `trigger = switch`).
    #[must_use]
    pub fn secret_exit_switch_special(&self) -> u16 {
        self.vocabulary.specials.exit.secret_switch
    }

    /// The linedef special for a walkover-activated secret exit
    /// (`progression.exit.kind = secret`, `trigger = walkover`).
    #[must_use]
    pub fn secret_exit_walkover_special(&self) -> u16 {
        self.vocabulary.specials.exit.secret_walkover
    }

    /// The linedef special for a switch-triggered lift.
    #[must_use]
    pub fn lift_switch_special(&self) -> u16 {
        self.vocabulary.specials.lift.switch
    }

    /// The linedef special for a walkover-triggered lift.
    #[must_use]
    pub fn lift_walkover_special(&self) -> u16 {
        self.vocabulary.specials.lift.walkover
    }

    /// The linedef special for a teleporter line.
    #[must_use]
    pub fn teleport_special(&self) -> u16 {
        self.vocabulary.specials.teleport.line
    }

    /// The sector special marking a sector "secret" (rule P18).
    #[must_use]
    pub fn secret_sector_special(&self) -> u16 {
        self.engine.sector.secret
    }

    /// The sector special for a liquid's damage tier (`flats.liquid.damage_tier`:
    /// `light`, `medium`, or `heavy`), if the tier name is known.
    #[must_use]
    pub fn damage_special(&self, tier: &str) -> Option<u16> {
        match tier {
            "light" => Some(self.engine.sector.damage.light),
            "medium" => Some(self.engine.sector.damage.medium),
            "heavy" => Some(self.engine.sector.damage.heavy),
            _ => None,
        }
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

    #[test]
    fn door_specials_and_key_things_resolve() {
        let t = Tables::load().expect("tables load");
        assert_ne!(t.door_special(), 0, "a manual door has a real special");
        // A card and the skull of the same color open the same door type —
        // the engine's key check accepts either.
        for (card, skull) in [
            ("blue_card", "blue_skull"),
            ("yellow_card", "yellow_skull"),
            ("red_card", "red_skull"),
        ] {
            let by_card = t.locked_door_special(card).expect("card special");
            assert_eq!(t.locked_door_special(skull), Some(by_card));
            assert_ne!(
                by_card,
                t.door_special(),
                "a keyed door differs from a plain one"
            );
            assert!(t.thing_id(card).is_some(), "`{card}` has a thing ID");
            assert!(t.thing_id(skull).is_some(), "`{skull}` has a thing ID");
        }
        assert_eq!(t.locked_door_special("plaid_card"), None);
        // The citation strings living alongside the numbers must never be
        // mistaken for one.
        assert_eq!(t.locked_door_special("source"), None);
    }

    /// Every name the compiler, the playability rules, or the design's
    /// template frontmatter (`docs/superpowers/specs/2026-08-09-crustygen-map-spec-design.md`
    /// section 5) references for exits, lifts, teleports, secrets, and
    /// liquid damage must resolve through `Tables` — and a name that is not
    /// in either table must fail loudly (`None`), never fall back to a
    /// guessed value.
    #[test]
    fn exit_lift_teleport_and_sector_specials_resolve() {
        let t = Tables::load().expect("tables load");

        // `progression.exit.kind` (normal | secret) crossed with
        // `progression.exit.trigger` (switch | walkover); `kind: both`
        // places one of each rather than needing a fifth special.
        let exits = [
            t.exit_switch_special(),
            t.exit_walkover_special(),
            t.secret_exit_switch_special(),
            t.secret_exit_walkover_special(),
        ];
        for special in exits {
            assert_ne!(special, 0, "an exit special must be a real linedef special");
        }
        assert_eq!(
            exits.iter().collect::<std::collections::HashSet<_>>().len(),
            4,
            "the four exit specials are all distinct"
        );

        // `progression.lifts.trigger` (walkover | switch | both_ends): the
        // repeatable form for each of the two trigger kinds.
        assert_ne!(t.lift_switch_special(), 0, "a lift switch special exists");
        assert_ne!(
            t.lift_walkover_special(),
            0,
            "a lift walkover special exists"
        );
        assert_ne!(
            t.lift_switch_special(),
            t.lift_walkover_special(),
            "the switch and walkover lift specials are distinct"
        );

        // `progression.teleports`: the line special and the destination
        // thing a `teleport` portal needs on both ends.
        assert_ne!(t.teleport_special(), 0, "a teleport special exists");
        assert!(
            t.thing_id("teleport_dest").is_some(),
            "`teleport_dest` has a thing ID"
        );

        // Rule P18's secret sector special.
        assert_ne!(
            t.secret_sector_special(),
            0,
            "a secret sector special exists"
        );

        // `flats.liquid.damage_tier` (light | medium | heavy).
        let light = t.damage_special("light").expect("light tier resolves");
        let medium = t.damage_special("medium").expect("medium tier resolves");
        let heavy = t.damage_special("heavy").expect("heavy tier resolves");
        assert!(
            light != medium && medium != heavy && light != heavy,
            "the three damage tiers are distinct sector specials"
        );
        assert_eq!(
            t.damage_special("radioactive"),
            None,
            "an unknown damage tier must fail loudly, not silently fall back"
        );
    }
}
