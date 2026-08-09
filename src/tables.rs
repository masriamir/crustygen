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

/// The radius, height, and blocking behavior of a non-monster prop —
/// a barrel, light source, or decoration that can obstruct passage or
/// hang from a ceiling. Rules P3 (passage width), P21 (light sources
/// match lighting), and P22 (hanging decoration headroom) need this.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PropDims {
    /// Collision radius in map units.
    pub radius: i32,
    /// Collision height in map units.
    pub height: i32,
    /// Whether the prop blocks movement (`MF_SOLID` in the pinned Doom
    /// source).
    pub blocks: bool,
    /// Whether the prop spawns hanging from the ceiling rather than
    /// standing on the floor (`MF_SPAWNCEILING` in the pinned Doom source).
    /// Rule P22 (hanging decoration headroom) needs this. Absent entries
    /// default to `false` — every prop recorded before this field existed
    /// stands on the floor.
    #[serde(default)]
    pub hangs: bool,
}

/// A health or armor pickup's amount, ceiling, and absorption class, as
/// loaded from `engine.toml`'s `[pickups.*]` table. See that table's
/// leading comment for the full mechanics (`P_GiveBody`/`P_GiveArmor` vs.
/// the two "bonus" pickups, which bypass both).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct PickupEntry {
    /// How much health or armor this pickup grants. For `green_armor` and
    /// `blue_armor` this is the flat value `P_GiveArmor` sets (skipped
    /// entirely if the player's current armor already meets or exceeds
    /// it), not an amount added to whatever the player already has.
    pub amount: i32,
    /// The health or armor ceiling this pickup cannot push past.
    pub cap: i32,
    /// The absorption class (`armortype` in the pinned Doom source) this
    /// pickup sets, if it sets one at all. `None` for the three pure-health
    /// pickups (`stimpack`, `medikit`, `health_bonus`).
    pub class: Option<i32>,
}

/// A monster species' collision dimensions and attack behavior, as loaded
/// from `engine.toml`'s `[species.*]` table.
///
/// Kept distinct from [`ThingDims`] (used for `[player]`, which has no
/// attack behavior of its own) rather than adding an optional field there.
#[derive(Debug, Clone, Copy, Deserialize)]
struct SpeciesEntry {
    radius: i32,
    height: i32,
    hitscan: bool,
    spawnhealth: i32,
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
    switch: String,
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

/// Linedef attribute flag bits (`doomdata.h`'s `ML_*` constants), a third
/// numeric space distinct from `vocabulary.toml`'s `[specials]` (linedef
/// *specials*) and this file's `[sector]`/`[sector.damage]` (*sector*
/// specials). UDMF's `doom` namespace spells each flag as its own named
/// boolean field on the linedef object rather than packing them — see
/// `emit_textmap`'s existing `blocking`, `dontpegbottom`, `dontpegtop`
/// output, which already follows that convention. The bit values recorded
/// here anchor each flag to its defining constant in the pinned source;
/// they are not themselves written to `TEXTMAP`.
#[derive(Debug, Deserialize)]
struct LinedefFlags {
    block_monsters: u16,
    sound_block: u16,
}

#[derive(Debug, Deserialize)]
struct LinedefAttrs {
    flags: LinedefFlags,
}

#[derive(Debug, Deserialize)]
struct Engine {
    movement: Movement,
    door: Door,
    light: LightRange,
    player: ThingDims,
    species: HashMap<String, SpeciesEntry>,
    props: HashMap<String, PropDims>,
    pickups: HashMap<String, PickupEntry>,
    sector: SectorSpecials,
    linedef: LinedefAttrs,
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
        self.engine.species.get(name).map(|s| ThingDims {
            radius: s.radius,
            height: s.height,
        })
    }

    /// Whether a named monster species' attack is a hitscan
    /// (`P_LineAttack`) rather than a spawned projectile, a melee-only
    /// attack, or something else entirely (see the per-species citation
    /// in `engine.toml` for the archvile and pain elemental, neither of
    /// which fits the hitscan/missile dichotomy cleanly), if the species
    /// is listed.
    #[must_use]
    pub fn hitscan(&self, name: &str) -> Option<bool> {
        self.engine.species.get(name).map(|s| s.hitscan)
    }

    /// A named monster species' `spawnhealth` — its starting hit points
    /// (`mobjinfo[*].spawnhealth` in the pinned Doom source), if the
    /// species is listed. `combat.max_simultaneous` and any future
    /// encounter-strength reasoning need this: `radius`/`height` alone say
    /// nothing about how hard a monster is to kill.
    #[must_use]
    pub fn spawnhealth(&self, name: &str) -> Option<i32> {
        self.engine.species.get(name).map(|s| s.spawnhealth)
    }

    /// The radius, height, and blocking behavior of a named non-monster
    /// prop (barrel, light source, or decoration), if the vocabulary
    /// records dimensions for it. Only props that block movement or hang
    /// from a ceiling carry an entry — see `engine.toml`'s `[props.*]`
    /// table.
    #[must_use]
    pub fn prop(&self, name: &str) -> Option<PropDims> {
        self.engine.props.get(name).copied()
    }

    /// The amount, ceiling, and absorption class of a named health or armor
    /// pickup (`stimpack` | `medikit` | `health_bonus` | `armor_bonus` |
    /// `green_armor` | `blue_armor`), if listed. `sustain.health_budget`
    /// and the explicit per-pickup counts beside it need this.
    #[must_use]
    pub fn pickup(&self, name: &str) -> Option<PickupEntry> {
        self.engine.pickups.get(name).copied()
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

    /// The `doomdata.h` bit value for a named linedef flag (`block_monsters`
    /// | `sound_block`), if known. `combat.block_monster_lines` needs
    /// `block_monsters` (`ML_BLOCKMONSTERS`); `combat.sound.block_sound_at`
    /// needs `sound_block` (`ML_SOUNDBLOCK`).
    ///
    /// UDMF's `doom` namespace spells each flag as its own named boolean
    /// field on the linedef object — `blockmonsters` and `blocksound`
    /// respectively — rather than this packed bit; see `emit_textmap`'s
    /// existing `blocking`/`dontpegbottom`/`dontpegtop` output for the
    /// convention a future emission path should follow. Not wired into any
    /// emission path yet.
    #[must_use]
    pub fn linedef_flag(&self, name: &str) -> Option<u16> {
        match name {
            "block_monsters" => Some(self.engine.linedef.flags.block_monsters),
            "sound_block" => Some(self.engine.linedef.flags.sound_block),
            _ => None,
        }
    }

    /// The texture for a role (`wall`, `floor`, `ceiling`, `door`,
    /// `door_track`, `switch`) under a theme, if both resolve.
    #[must_use]
    pub fn texture(&self, role: &str, theme: &str) -> Option<&str> {
        let set = self.vocabulary.textures.get(theme)?;
        match role {
            "wall" => Some(&set.wall),
            "floor" => Some(&set.floor),
            "ceiling" => Some(&set.ceiling),
            "door" => Some(&set.door),
            "door_track" => Some(&set.door_track),
            "switch" => Some(&set.switch),
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
            t.species("plaid_imp").is_none(),
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

    /// Every monster species the vocabulary now carries — the original
    /// four plus every Doom/Doom II monster added for the map-spec
    /// template's `combat.monsters[].species` and `combat.boss` fields —
    /// must resolve a doomednum, collision dims, a hitscan flag, and its
    /// `spawnhealth` (`mobjinfo[*].spawnhealth` in the pinned Doom source).
    /// Checked individually, not sampled: a name missing from either
    /// table is exactly the defect this test exists to catch.
    #[test]
    fn every_monster_species_resolves() {
        let t = Tables::load().expect("tables load");
        // (name, doomednum, radius, height, hitscan, spawnhealth)
        let monsters: &[(&str, u16, i32, i32, bool, i32)] = &[
            ("zombieman", 3004, 20, 56, true, 20),
            ("shotgun_guy", 9, 20, 56, true, 30),
            ("imp", 3001, 20, 56, false, 60),
            ("pinky", 3002, 30, 56, false, 150),
            ("spectre", 58, 30, 56, false, 150),
            ("chaingunner", 65, 20, 56, true, 70),
            ("cacodemon", 3005, 31, 56, false, 400),
            ("lost_soul", 3006, 16, 56, false, 100),
            ("pain_elemental", 71, 31, 56, false, 400),
            ("hell_knight", 69, 24, 64, false, 500),
            ("baron_of_hell", 3003, 24, 64, false, 1000),
            ("revenant", 66, 20, 56, false, 300),
            ("mancubus", 67, 48, 64, false, 600),
            ("arachnotron", 68, 64, 64, false, 500),
            ("archvile", 64, 20, 56, false, 700),
            ("cyberdemon", 16, 40, 110, false, 4000),
            ("spider_mastermind", 7, 128, 100, true, 3000),
            ("wolfenstein_ss", 84, 20, 56, true, 50),
        ];
        for (name, doomednum, radius, height, hitscan, spawnhealth) in monsters {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t
                .species(name)
                .unwrap_or_else(|| panic!("`{name}` species dims"));
            assert_eq!(dims.radius, *radius, "`{name}` radius");
            assert_eq!(dims.height, *height, "`{name}` height");
            assert_eq!(t.hitscan(name), Some(*hitscan), "`{name}` hitscan");
            assert_eq!(
                t.spawnhealth(name),
                Some(*spawnhealth),
                "`{name}` spawnhealth"
            );
        }
        assert_eq!(monsters.len(), 18, "every listed monster was checked");
        assert!(
            t.species("plaid_imp").is_none(),
            "unlisted species is absent"
        );
        assert_eq!(
            t.hitscan("plaid_imp"),
            None,
            "unlisted species hitscan is absent"
        );
        assert_eq!(
            t.spawnhealth("plaid_imp"),
            None,
            "unlisted species spawnhealth is absent"
        );
    }

    /// Every weapon, ammo, health/armor, and powerup pickup the map-spec
    /// template's `arsenal` and `sustain` sections can name must resolve a
    /// doomednum. Checked individually — see `every_monster_species_resolves`
    /// for why a loop over an explicit table still satisfies that.
    #[test]
    fn every_pickup_resolves() {
        let t = Tables::load().expect("tables load");
        let pickups: &[(&str, u16)] = &[
            // Weapons — match `arsenal.weapons[].name` exactly.
            ("chainsaw", 2005),
            ("shotgun", 2001),
            ("super_shotgun", 82),
            ("chaingun", 2002),
            ("rocket_launcher", 2003),
            ("plasma_rifle", 2004),
            ("bfg9000", 2006),
            // Ammo.
            ("clip", 2007),
            ("box_of_bullets", 2048),
            ("shells", 2008),
            ("box_of_shells", 2049),
            ("rocket", 2010),
            ("box_of_rockets", 2046),
            ("cell_charge", 2047),
            ("cell_pack", 17),
            ("backpack", 8),
            // Health and armor — match `sustain.health.*` exactly;
            // `sustain.armor.{green,blue}` map to `green_armor`/`blue_armor`
            // here (see the `health_armor_source` citation in vocabulary.toml).
            ("stimpack", 2011),
            ("medikit", 2012),
            ("health_bonus", 2014),
            ("armor_bonus", 2015),
            ("green_armor", 2018),
            ("blue_armor", 2019),
            // Powerups — match `sustain.powerups[].name` exactly, short
            // forms included (`radsuit`, `light_amp`).
            ("berserk", 2023),
            ("soulsphere", 2013),
            ("megasphere", 83),
            ("invulnerability", 2022),
            ("invisibility", 2024),
            ("radsuit", 2025),
            ("light_amp", 2045),
            ("computer_map", 2026),
        ];
        for (name, doomednum) in pickups {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
        }
        assert_eq!(pickups.len(), 30, "every listed pickup was checked");
        // Every doomednum above is distinct — a duplicate would mean two
        // spec-visible names silently resolve to the same physical pickup.
        let unique: std::collections::HashSet<_> = pickups.iter().map(|(_, id)| id).collect();
        assert_eq!(
            unique.len(),
            pickups.len(),
            "all pickup doomednums are distinct"
        );
    }

    /// Every health and armor pickup's amount, cap, and absorption class
    /// (`engine.toml`'s `[pickups.*]` table) must resolve, checked
    /// individually. `sustain.health_budget` and the explicit counts
    /// beside it need real numbers, not just a doomednum, to derive from.
    #[test]
    fn every_health_and_armor_pickup_resolves_amount_cap_and_class() {
        let t = Tables::load().expect("tables load");
        // (name, amount, cap, class)
        let pickups: &[(&str, i32, i32, Option<i32>)] = &[
            ("stimpack", 10, 100, None),
            ("medikit", 25, 100, None),
            ("health_bonus", 1, 200, None),
            ("armor_bonus", 1, 200, Some(1)),
            ("green_armor", 100, 100, Some(1)),
            ("blue_armor", 200, 200, Some(2)),
        ];
        for (name, amount, cap, class) in pickups {
            let entry = t.pickup(name).unwrap_or_else(|| panic!("`{name}` pickup"));
            assert_eq!(entry.amount, *amount, "`{name}` amount");
            assert_eq!(entry.cap, *cap, "`{name}` cap");
            assert_eq!(entry.class, *class, "`{name}` class");
        }
        assert_eq!(pickups.len(), 6, "every listed pickup was checked");
        // The two "bonus" pickups can push past the ordinary maximum
        // (stimpack/medikit's 100); their cap must reflect that.
        assert!(
            t.pickup("health_bonus").unwrap().cap > t.pickup("stimpack").unwrap().cap,
            "health_bonus exceeds the ordinary health maximum"
        );
        assert!(
            t.pickup("armor_bonus").unwrap().cap > t.pickup("green_armor").unwrap().cap,
            "armor_bonus exceeds green_armor's own cap"
        );
        assert!(
            t.pickup("plaid_stim").is_none(),
            "unlisted pickup is absent"
        );
    }

    /// The linedef flag bits that `combat.block_monster_lines` and
    /// `combat.sound.block_sound_at` need must resolve, and an unknown flag
    /// name must fail loudly rather than silently fall back.
    #[test]
    fn linedef_flags_resolve() {
        let t = Tables::load().expect("tables load");
        assert_eq!(
            t.linedef_flag("block_monsters"),
            Some(2),
            "ML_BLOCKMONSTERS resolves"
        );
        assert_eq!(
            t.linedef_flag("sound_block"),
            Some(64),
            "ML_SOUNDBLOCK resolves"
        );
        assert_ne!(
            t.linedef_flag("block_monsters"),
            t.linedef_flag("sound_block"),
            "the two linedef flags are distinct bits"
        );
        assert_eq!(
            t.linedef_flag("blocking"),
            None,
            "ML_BLOCKING is structural (LinedefOut::blocking), not a sourced flag entry"
        );
        assert_eq!(
            t.linedef_flag("plaid_flag"),
            None,
            "an unknown flag name must fail loudly, not silently fall back"
        );
    }

    /// The exploding barrel and every scenery/light-source prop must
    /// resolve a doomednum; the four that block movement must also
    /// resolve `[props.*]` dims (rules P3, P21, P22), and the two
    /// non-blocking decorations must NOT carry a `[props.*]` entry —
    /// asserting that distinction directly rather than only checking
    /// `Some`/`None` loosely.
    #[test]
    fn every_scenery_prop_resolves() {
        let t = Tables::load().expect("tables load");
        // (name, doomednum, dims if it blocks: Some(radius, height))
        let blocking: &[(&str, u16, i32, i32)] = &[
            ("barrel", 2035, 10, 42),
            ("floor_lamp", 2028, 16, 16),
            ("techno_lamp", 85, 16, 16),
            ("candelabra", 35, 16, 16),
        ];
        for (name, doomednum, radius, height) in blocking {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t.prop(name).unwrap_or_else(|| panic!("`{name}` prop dims"));
            assert_eq!(dims.radius, *radius, "`{name}` radius");
            assert_eq!(dims.height, *height, "`{name}` height");
            assert!(dims.blocks, "`{name}` blocks movement");
        }

        let non_blocking: &[(&str, u16)] = &[("candle", 34), ("gibs", 24)];
        for (name, doomednum) in non_blocking {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            assert!(
                t.prop(name).is_none(),
                "`{name}` is non-blocking and carries no [props.*] entry"
            );
        }

        assert!(t.prop("plaid_imp").is_none(), "unlisted prop is absent");
    }

    /// Every gore prop `scenery.gore` (none | light | heavy) can place must
    /// resolve a doomednum, checked individually across all four groups —
    /// standing corpses, gib props, blood/bone floor decorations, and
    /// hanging bodies. The nine that block (the three bone props and the
    /// six hanging bodies) must also resolve `[props.*]` dims, and the six
    /// hanging bodies specifically must report `hangs = true` (rule P22)
    /// while the three bone props report `hangs = false` — floor-standing,
    /// not `MF_SPAWNCEILING`, despite sharing `MF_SOLID` with the hanging
    /// set.
    #[test]
    fn every_gore_prop_resolves() {
        let t = Tables::load().expect("tables load");

        // Standing corpses: non-blocking, no [props.*] entry.
        let corpses: &[(&str, u16)] = &[
            ("dead_zombieman", 18),
            ("dead_shotgun_guy", 19),
            ("dead_imp", 20),
            ("dead_pinky", 21),
            ("dead_cacodemon", 22),
            ("dead_lost_soul", 23),
            ("dead_player", 15),
        ];
        for (name, doomednum) in corpses {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            assert!(
                t.prop(name).is_none(),
                "`{name}` is a non-blocking standing corpse"
            );
        }
        assert_eq!(corpses.len(), 7, "every standing corpse was checked");

        // Gib props: non-blocking, no [props.*] entry.
        let gibs: &[(&str, u16)] = &[("gibs", 24), ("bloody_mess", 10), ("bloody_mess_alt", 12)];
        for (name, doomednum) in gibs {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            assert!(
                t.prop(name).is_none(),
                "`{name}` is a non-blocking gib prop"
            );
        }
        assert_eq!(gibs.len(), 3, "every gib prop was checked");

        // Blood floor decorations: non-blocking, no [props.*] entry.
        let blood: &[(&str, u16)] = &[("small_pool", 80), ("colon_gibs", 79)];
        for (name, doomednum) in blood {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            assert!(
                t.prop(name).is_none(),
                "`{name}` is a non-blocking blood decoration"
            );
        }
        assert_eq!(blood.len(), 2, "every blood decoration was checked");

        // Bone floor decorations: blocking, floor-standing (hangs = false).
        let bone: &[(&str, u16)] = &[
            ("heads_on_stick", 28),
            ("head_on_a_stick", 27),
            ("head_candles", 29),
        ];
        for (name, doomednum) in bone {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t.prop(name).unwrap_or_else(|| panic!("`{name}` prop dims"));
            assert_eq!(dims.radius, 16, "`{name}` radius");
            assert_eq!(dims.height, 16, "`{name}` height");
            assert!(dims.blocks, "`{name}` blocks movement");
            assert!(!dims.hangs, "`{name}` is floor-standing, not hanging");
        }
        assert_eq!(bone.len(), 3, "every bone decoration was checked");

        // Hanging bodies: blocking, hanging (hangs = true), P22-relevant.
        let hanging: &[(&str, u16, i32)] = &[
            ("hang_no_guts", 73, 88),
            ("hang_no_brain", 74, 88),
            ("hang_torso_look_down", 75, 64),
            ("hang_torso_skull", 76, 64),
            ("hang_torso_look_up", 77, 64),
            ("hang_torso_no_brain", 78, 64),
        ];
        for (name, doomednum, height) in hanging {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t.prop(name).unwrap_or_else(|| panic!("`{name}` prop dims"));
            assert_eq!(dims.radius, 16, "`{name}` radius");
            assert_eq!(dims.height, *height, "`{name}` height");
            assert!(dims.blocks, "`{name}` blocks movement");
            assert!(dims.hangs, "`{name}` hangs from the ceiling");
        }
        assert_eq!(hanging.len(), 6, "every hanging body was checked");

        // Every gore doomednum above is distinct.
        let all_ids: Vec<u16> = corpses
            .iter()
            .chain(gibs)
            .chain(blood)
            .chain(bone)
            .map(|(_, id)| *id)
            .chain(hanging.iter().map(|(_, id, _)| *id))
            .collect();
        let unique: std::collections::HashSet<_> = all_ids.iter().collect();
        assert_eq!(
            unique.len(),
            all_ids.len(),
            "all gore doomednums are distinct"
        );
    }

    /// An exit switch needs a switch texture to render — the theme's
    /// `[textures.*]` set must resolve one alongside its wall/floor/
    /// ceiling/door textures.
    #[test]
    fn switch_texture_resolves() {
        let t = Tables::load().expect("tables load");
        assert_eq!(
            t.texture("switch", "tech_base"),
            Some("SW1STARG"),
            "tech_base has a switch texture"
        );
        assert_eq!(
            t.texture("switch", "plaid_theme"),
            None,
            "an unknown theme resolves no texture at all"
        );
    }
}
