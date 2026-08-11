//! Typed frontmatter groups: the fields of a filled map-spec template,
//! deserialized straight from the YAML frontmatter
//! [`crate::spec::split_frontmatter`] splits off. Each group here mirrors
//! one top-level key of the template in
//! `docs/design.md` §5; validating cross-field rules (P-numbered) and
//! resolving vocabulary names against [`crate::tables::Tables`] both happen
//! in later stages, not here.

use serde::Deserialize;

// ---------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------

/// An inclusive `[min, max]` range read from a two-key YAML mapping such as
/// `{ min: 8, max: 14 }`, used throughout the template for count and
/// dimension budgets.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MinMax<T> {
    /// The lower bound, inclusive.
    pub min: T,
    /// The upper bound, inclusive.
    pub max: T,
}

/// A field that is either the literal YAML string `auto` or an explicit
/// value of type `T`.
///
/// Several template fields (`ammo.pickups`, `aesthetics.texture_set`, and
/// others in later groups) let the author write `auto` to mean "derive this
/// from other fields" instead of spelling out a value. This deserializes the
/// raw YAML into a [`serde_norway::Value`] first and checks for the `auto`
/// string before falling back to `T`'s own `Deserialize` impl; the
/// trade-off is that an error inside `T` is reported at this field's own
/// path rather than pointing deeper into `T`'s structure — acceptable,
/// since the field is still named in the error.
#[derive(Debug, Clone, PartialEq)]
pub enum AutoOr<T> {
    /// The author wrote `auto`: derive the value elsewhere.
    Auto,
    /// The author wrote an explicit value.
    Given(T),
}

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for AutoOr<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_norway::Value::deserialize(d)?;
        if matches!(&value, serde_norway::Value::String(s) if s == "auto") {
            return Ok(Self::Auto);
        }
        T::deserialize(value)
            .map(Self::Given)
            .map_err(serde::de::Error::custom)
    }
}

// ---------------------------------------------------------------------
// Identity and target
// ---------------------------------------------------------------------

/// The `identity` group: which map this is and what it targets.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// The map lump name, e.g. `MAP01`.
    pub slot: String,
    /// The map's display title.
    pub title: String,
    /// The map's author.
    pub author: String,
    /// Which IWAD the map targets.
    pub iwad: Iwad,
    /// Which output formats to produce.
    pub outputs: Vec<Output>,
    /// The generation seed; the same seed and IR produce a byte-identical
    /// `TEXTMAP`.
    pub seed: u64,
    /// The grid size, in map units, that all coordinates snap to.
    pub grid: i32,
}

/// Which IWAD a map targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Iwad {
    /// The commercial `DOOM2.WAD`.
    Doom2,
    /// The free `freedoom2.wad` replacement.
    Freedoom2,
}

/// An output format the compiler produces from the IR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Output {
    /// UDMF `TEXTMAP`, authored directly.
    Udmf,
    /// The vanilla binary map format, produced by converting the UDMF
    /// output.
    Doom,
}

// ---------------------------------------------------------------------
// Players and starts
// ---------------------------------------------------------------------

/// The `players` group: starts and multiplayer behavior.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Players {
    /// The direction (or explicit angle) the player 1 start faces.
    pub start_facing: Facing,
    /// The number of cooperative starts to place; 0 disables coop.
    pub coop_starts: u32,
    /// The number of deathmatch starts to place; 0 disables deathmatch.
    pub dm_starts: u32,
    /// Whether to place extra pickups flagged multiplayer-only.
    pub coop_only_items: bool,
}

/// The starting player's initial facing direction.
///
/// A YAML string parses to one of the four compass points; a YAML integer
/// parses to [`Facing::Degrees`]. Checking that the degree value falls in
/// 0..=359 is deferred to validation, not enforced here.
#[derive(Debug, Clone, PartialEq)]
pub enum Facing {
    /// Facing north.
    North,
    /// Facing south.
    South,
    /// Facing east.
    East,
    /// Facing west.
    West,
    /// An explicit angle in degrees, not necessarily aligned to a compass
    /// point.
    Degrees(u16),
}

impl<'de> serde::Deserialize<'de> for Facing {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        const ALLOWED: &str = "expected north, south, east, west, or an integer angle in degrees";
        let value = serde_norway::Value::deserialize(d)?;
        match &value {
            serde_norway::Value::String(s) => match s.as_str() {
                "north" => Ok(Self::North),
                "south" => Ok(Self::South),
                "east" => Ok(Self::East),
                "west" => Ok(Self::West),
                _ => Err(serde::de::Error::custom(ALLOWED)),
            },
            serde_norway::Value::Number(n) => n
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .map(Self::Degrees)
                .ok_or_else(|| serde::de::Error::custom(ALLOWED)),
            _ => Err(serde::de::Error::custom(ALLOWED)),
        }
    }
}

// ---------------------------------------------------------------------
// Scale budget
// ---------------------------------------------------------------------

/// The `scale` group: the size and count budgets the map should land
/// within.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scale {
    /// The map's bounding box, in map units.
    pub size: Size,
    /// The room count budget.
    pub rooms: MinMax<u32>,
    /// The sector count budget.
    pub sectors: MinMax<u32>,
    /// The linedef count budget.
    pub linedefs: MinMax<u32>,
    /// The expected play time, in minutes.
    pub play_time_minutes: MinMax<u32>,
    /// The allowed floor height range; the map's vertical span is
    /// `max - min`.
    pub vertical_range: MinMax<i32>,
}

/// A bounding box, in map units.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Size {
    /// The width, in map units.
    pub width: i32,
    /// The height, in map units.
    pub height: i32,
}

// ---------------------------------------------------------------------
// Progression
// ---------------------------------------------------------------------

/// The `progression` group: how the player moves through the map.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Progression {
    /// The overall shape of the progression graph.
    pub shape: Shape,
    /// The keys required to finish the map, as vocabulary names.
    pub keys: Vec<String>,
    /// The number of key-locked doors.
    pub locked_doors: u32,
    /// How much the map asks the player to retrace earlier ground.
    pub backtracking: Backtracking,
    /// The map's exit.
    pub exit: Exit,
    /// The map's lifts.
    pub lifts: Lifts,
    /// The map's teleports.
    pub teleports: Teleports,
    /// The map's doors.
    pub doors: Doors,
    /// The map's switches.
    pub switches: Switches,
    /// The map's walkover triggers.
    pub walkover_triggers: WalkoverTriggers,
}

/// The overall shape of the progression graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A single path from start to exit.
    Linear,
    /// A central area with spokes the player returns to between forays.
    HubAndSpoke,
    /// Multiple paths that reconverge.
    Branching,
    /// A tight, largely linear gauntlet with little room to explore.
    Gauntlet,
}

/// How much the map asks the player to retrace earlier ground.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Backtracking {
    /// The player never needs to retrace ground already covered.
    None,
    /// Occasional, short backtracking.
    Light,
    /// Substantial backtracking is part of the map's structure.
    Heavy,
}

/// The map's exit.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exit {
    /// Whether the exit is the map's normal exit, a secret exit, or both.
    pub kind: ExitKind,
    /// How the player activates the exit.
    pub trigger: ExitTrigger,
}

/// Whether an exit is normal, secret, or both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitKind {
    /// The map's normal exit.
    Normal,
    /// A secret exit, e.g. to a bonus map.
    Secret,
    /// Both a normal and a secret exit are present.
    Both,
}

/// How the player activates an exit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitTrigger {
    /// A switch activates the exit.
    Switch,
    /// A teleporter delivers the player to the exit.
    Teleport,
    /// Walking onto a line activates the exit.
    Walkover,
}

/// The map's lifts.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lifts {
    /// The number of lifts.
    pub count: MinMax<u32>,
    /// How a lift is activated.
    pub trigger: LiftTrigger,
    /// The largest floor delta a single lift may span, in map units.
    pub max_travel: i32,
}

/// How a lift is activated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiftTrigger {
    /// Walking onto the lift activates it.
    Walkover,
    /// A switch activates the lift.
    Switch,
    /// The lift can be activated from either end.
    BothEnds,
}

/// The map's teleports.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Teleports {
    /// The number of teleports.
    pub count: MinMax<u32>,
}

/// The map's doors.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Doors {
    /// How fast doors move.
    pub speed: DoorSpeed,
    /// The default behavior for a door with no more specific setting.
    pub default_behavior: DoorBehavior,
    /// The key types that lock doors, as vocabulary names. Must be a subset
    /// of [`Progression::keys`] (rule P24); that check is deferred to
    /// validation, not enforced here.
    pub lock_types: Vec<String>,
}

/// How fast a door moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorSpeed {
    /// The vanilla door speed.
    Normal,
    /// A faster door speed.
    Fast,
}

/// The default behavior for a door.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoorBehavior {
    /// The door can be opened and closed repeatedly.
    Repeatable,
    /// The door can only be used once.
    OneShot,
    /// The door opens once and stays open.
    StaysOpen,
}

/// The map's switches.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Switches {
    /// The number of switches.
    pub count: MinMax<u32>,
    /// Whether a switch may act on a sector other than the one it sits in.
    pub remote_allowed: bool,
}

/// The map's walkover triggers.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WalkoverTriggers {
    /// The number of walkover triggers.
    pub count: MinMax<u32>,
}

// ---------------------------------------------------------------------
// Architecture
// ---------------------------------------------------------------------

/// The `architecture` group: the geometric character of the map's rooms.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Architecture {
    /// The room footprint shapes the map may use.
    pub room_shapes: Vec<RoomShape>,
    /// The map's overall symmetry.
    pub symmetry: Symmetry,
    /// How open the map's spaces feel.
    pub openness: Openness,
    /// The fraction of floor area that is transit rather than usable space.
    pub corridor_ratio: f64,
    /// How much the map varies in height.
    pub verticality: Verticality,
    /// Whether sightlines exist between areas that are not directly
    /// connected.
    pub inter_area_windows: bool,
    /// The number of elevated vantage points over another area.
    pub overlooks: MinMax<u32>,
    /// The number of visually distinct anchors aiding navigation.
    pub landmarks: u32,
}

/// A room footprint shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoomShape {
    /// A plain rectangle.
    Rectangular,
    /// An L-shaped footprint.
    LShaped,
    /// A T-shaped footprint.
    TShaped,
    /// A regular octagon.
    Octagonal,
    /// A footprint that does not fit the other named shapes.
    Irregular,
}

/// The map's overall symmetry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Symmetry {
    /// No deliberate symmetry.
    Organic,
    /// Mirrored across a single axis.
    Axial,
    /// Symmetric about a central point.
    Radial,
    /// Some areas are symmetric and others are not.
    Mixed,
}

/// How open the map's spaces feel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Openness {
    /// Narrow, enclosed spaces throughout.
    Tight,
    /// A mix of tight and open spaces.
    Mixed,
    /// Large, open spaces throughout.
    Open,
}

/// How much the map varies in height.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verticality {
    /// Little to no height variation.
    Flat,
    /// Some height variation.
    Moderate,
    /// Substantial height variation.
    Strong,
}

// ---------------------------------------------------------------------
// Combat
// ---------------------------------------------------------------------

/// The `combat` group: encounter design and monster placement.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Combat {
    /// The dominant encounter style.
    pub encounter_style: EncounterStyle,
    /// The fraction of total monster count that is hitscan.
    pub hitscanner_ratio: f64,
    /// The pressure ceiling: the most monsters active at once.
    pub max_simultaneous: u32,
    /// The number of monster closets.
    pub monster_closets: u32,
    /// The map's boss monster, if any.
    pub boss: Boss,
    /// The map's ambush tuning.
    pub ambush: Ambush,
    /// The map's sound propagation tuning.
    pub sound: Sound,
    /// Whether monsters are kept in their region with sound/monster-blocking
    /// lines rather than a wall.
    pub block_monster_lines: bool,
    /// The monster population, by species.
    pub monsters: Vec<MonsterSpec>,
}

/// The dominant encounter style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EncounterStyle {
    /// Monsters encountered along the way, not as a set-piece.
    Incidental,
    /// Monsters that spring on the player from hiding or behind.
    Ambush,
    /// A large set-piece fight in an open arena.
    Arena,
    /// A fight fought along a narrow corridor.
    Corridor,
}

/// The map's boss monster, if any.
///
/// The string `none` parses to [`Boss::None`]. The string `mastermind` — the
/// template's short form for the boss, not a vocabulary key — bridges to
/// `Boss::Species("spider_mastermind")`: `data/vocabulary.toml`'s `[things]`
/// comment names `spider_mastermind` as the vocabulary key for this monster
/// and explicitly leaves bridging the short form `mastermind` to "the
/// compiler layer that reads `combat.boss`" rather than adding an alias to
/// the table itself, since guessing which of the two forms future code
/// would want was exactly the kind of unsourced choice that table avoids —
/// this parser is that layer. Any other string is taken as a vocabulary
/// species name directly; resolving it against [`crate::tables::Tables`] is
/// deferred to a later stage.
///
/// Deserializes from a bare YAML string with a custom impl rather than
/// deriving, since the mapping from string to variant is not a fixed set of
/// enum names.
#[derive(Debug, Clone, PartialEq)]
pub enum Boss {
    /// No boss monster.
    None,
    /// A boss monster, identified by its vocabulary species name.
    Species(String),
}

impl<'de> serde::Deserialize<'de> for Boss {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let value = serde_norway::Value::deserialize(d)?;
        match &value {
            serde_norway::Value::String(s) if s == "none" => Ok(Self::None),
            serde_norway::Value::String(s) if s == "mastermind" => {
                Ok(Self::Species("spider_mastermind".into()))
            }
            serde_norway::Value::String(s) => Ok(Self::Species(s.clone())),
            _ => Err(serde::de::Error::custom(
                "expected `none`, `mastermind`, or a monster species name",
            )),
        }
    }
}

/// The map's ambush tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ambush {
    /// The fraction of monsters flagged deaf: they wake on sight, not on
    /// sound.
    pub deaf_ratio: f64,
    /// The number of teleport-in ambushes.
    pub teleport_ambushes: MinMax<u32>,
}

/// The map's sound propagation tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sound {
    /// How far sound carries between areas.
    pub propagation: Propagation,
    /// Where sound-blocking lines go.
    pub block_sound_at: Vec<SoundBlockSite>,
}

/// How far sound carries between areas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Propagation {
    /// Sound carries freely between areas.
    Open,
    /// Sound is blocked at a few deliberate points.
    Contained,
    /// Sound is blocked at every area boundary.
    Sealed,
}

/// A site where a sound-blocking line should be placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SoundBlockSite {
    /// At key-locked doors.
    KeyDoors,
    /// At the entrances to combat arenas.
    ArenaEntrances,
}

/// One monster species' population budget.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonsterSpec {
    /// The vocabulary species name.
    pub species: String,
    /// The minimum count.
    pub min: u32,
    /// The maximum count.
    pub max: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_group_from_the_design_doc_parses() {
        let y = r#"
slot: MAP01
title: "Refinery Overrun"
author: "Amir Masri"
iwad: doom2
outputs: [udmf, doom]
seed: 20260809
grid: 64
"#;
        let id: Identity = serde_norway::from_str(y).unwrap();
        assert_eq!(id.slot, "MAP01");
        assert_eq!(id.iwad, Iwad::Doom2);
        assert_eq!(id.outputs, vec![Output::Udmf, Output::Doom]);
        assert_eq!(id.grid, 64);
    }

    #[test]
    fn an_unknown_key_in_a_group_is_rejected() {
        let y = "slot: MAP01\ntitle: t\nauthor: a\niwad: doom2\noutputs: [udmf]\nseed: 1\ngrid: 64\nslto: oops\n";
        assert!(serde_norway::from_str::<Identity>(y).is_err());
    }

    #[test]
    fn a_compass_facing_parses_and_a_number_becomes_degrees() {
        assert_eq!(
            serde_norway::from_str::<Facing>("east").unwrap(),
            Facing::East
        );
        assert_eq!(
            serde_norway::from_str::<Facing>("135").unwrap(),
            Facing::Degrees(135)
        );
    }

    #[test]
    fn a_facing_string_that_is_not_a_compass_point_is_rejected() {
        assert!(serde_norway::from_str::<Facing>("upward").is_err());
    }

    #[test]
    fn boss_none_and_the_mastermind_bridge_both_parse() {
        assert_eq!(serde_norway::from_str::<Boss>("none").unwrap(), Boss::None);
        assert_eq!(
            serde_norway::from_str::<Boss>("mastermind").unwrap(),
            Boss::Species("spider_mastermind".into())
        );
        assert_eq!(
            serde_norway::from_str::<Boss>("cyberdemon").unwrap(),
            Boss::Species("cyberdemon".into())
        );
    }

    #[test]
    fn an_inline_min_max_pair_parses() {
        let r: MinMax<u32> = serde_norway::from_str("{ min: 8, max: 14 }").unwrap();
        assert_eq!((r.min, r.max), (8, 14));
    }

    #[test]
    fn an_auto_scalar_parses_as_auto_and_other_values_pass_through() {
        assert_eq!(
            serde_norway::from_str::<AutoOr<i32>>("auto").unwrap(),
            AutoOr::Auto
        );
        assert_eq!(
            serde_norway::from_str::<AutoOr<i32>>("5").unwrap(),
            AutoOr::Given(5)
        );
    }

    #[test]
    fn the_players_group_from_the_design_doc_parses() {
        let y = r"
start_facing: east
coop_starts: 4
dm_starts: 0
coop_only_items: false
";
        let players: Players = serde_norway::from_str(y).unwrap();
        assert_eq!(players.start_facing, Facing::East);
        assert_eq!(players.coop_starts, 4);
        assert_eq!(players.dm_starts, 0);
        assert!(!players.coop_only_items);
    }

    #[test]
    fn the_scale_group_from_the_design_doc_parses() {
        let y = r"
size: { width: 4096, height: 4096 }
rooms: { min: 8, max: 14 }
sectors: { min: 40, max: 120 }
linedefs: { min: 200, max: 600 }
play_time_minutes: { min: 6, max: 10 }
vertical_range: { min: 0, max: 256 }
";
        let scale: Scale = serde_norway::from_str(y).unwrap();
        assert_eq!(
            scale.size,
            Size {
                width: 4096,
                height: 4096
            }
        );
        assert_eq!(scale.rooms, MinMax { min: 8, max: 14 });
        assert_eq!(scale.sectors, MinMax { min: 40, max: 120 });
        assert_eq!(scale.linedefs, MinMax { min: 200, max: 600 });
        assert_eq!(scale.play_time_minutes, MinMax { min: 6, max: 10 });
        assert_eq!(scale.vertical_range, MinMax { min: 0, max: 256 });
    }

    #[test]
    fn the_progression_group_from_the_design_doc_parses() {
        let y = r"
shape: hub_and_spoke
keys: [blue_card, red_skull]
locked_doors: 2
backtracking: light
exit:
  kind: normal
  trigger: switch
lifts:
  count: { min: 0, max: 2 }
  trigger: both_ends
  max_travel: 256
teleports:
  count: { min: 0, max: 2 }
doors:
  speed: normal
  default_behavior: repeatable
  lock_types: [blue_card, red_skull]
switches:
  count: { min: 2, max: 6 }
  remote_allowed: true
walkover_triggers:
  count: { min: 1, max: 4 }
";
        let p: Progression = serde_norway::from_str(y).unwrap();
        assert_eq!(p.shape, Shape::HubAndSpoke);
        assert_eq!(
            p.keys,
            vec!["blue_card".to_string(), "red_skull".to_string()]
        );
        assert_eq!(p.locked_doors, 2);
        assert_eq!(p.backtracking, Backtracking::Light);
        assert_eq!(
            p.exit,
            Exit {
                kind: ExitKind::Normal,
                trigger: ExitTrigger::Switch
            }
        );
        assert_eq!(
            p.lifts,
            Lifts {
                count: MinMax { min: 0, max: 2 },
                trigger: LiftTrigger::BothEnds,
                max_travel: 256
            }
        );
        assert_eq!(
            p.teleports,
            Teleports {
                count: MinMax { min: 0, max: 2 }
            }
        );
        assert_eq!(p.doors.speed, DoorSpeed::Normal);
        assert_eq!(p.doors.default_behavior, DoorBehavior::Repeatable);
        assert_eq!(
            p.doors.lock_types,
            vec!["blue_card".to_string(), "red_skull".to_string()]
        );
        assert_eq!(
            p.switches,
            Switches {
                count: MinMax { min: 2, max: 6 },
                remote_allowed: true
            }
        );
        assert_eq!(
            p.walkover_triggers,
            WalkoverTriggers {
                count: MinMax { min: 1, max: 4 }
            }
        );
    }

    #[test]
    fn a_progression_shape_that_is_not_a_known_variant_is_rejected() {
        let y = r"
shape: spiral
keys: [blue_card]
locked_doors: 0
backtracking: none
exit:
  kind: normal
  trigger: switch
lifts:
  count: { min: 0, max: 0 }
  trigger: switch
  max_travel: 0
teleports:
  count: { min: 0, max: 0 }
doors:
  speed: normal
  default_behavior: repeatable
  lock_types: []
switches:
  count: { min: 0, max: 0 }
  remote_allowed: false
walkover_triggers:
  count: { min: 0, max: 0 }
";
        assert!(serde_norway::from_str::<Progression>(y).is_err());
    }

    #[test]
    fn the_architecture_group_from_the_design_doc_parses() {
        let y = r"
room_shapes: [rectangular, l_shaped, octagonal]
symmetry: organic
openness: mixed
corridor_ratio: 0.3
verticality: moderate
inter_area_windows: true
overlooks: { min: 1, max: 3 }
landmarks: 1
";
        let a: Architecture = serde_norway::from_str(y).unwrap();
        assert_eq!(
            a.room_shapes,
            vec![
                RoomShape::Rectangular,
                RoomShape::LShaped,
                RoomShape::Octagonal
            ]
        );
        assert_eq!(a.symmetry, Symmetry::Organic);
        assert_eq!(a.openness, Openness::Mixed);
        assert!((a.corridor_ratio - 0.3).abs() < f64::EPSILON);
        assert_eq!(a.verticality, Verticality::Moderate);
        assert!(a.inter_area_windows);
        assert_eq!(a.overlooks, MinMax { min: 1, max: 3 });
        assert_eq!(a.landmarks, 1);
    }

    #[test]
    fn the_combat_group_from_the_design_doc_parses() {
        let y = r"
encounter_style: ambush
hitscanner_ratio: 0.35
max_simultaneous: 12
monster_closets: 3
boss: none
ambush:
  deaf_ratio: 0.4
  teleport_ambushes: { min: 1, max: 3 }
sound:
  propagation: contained
  block_sound_at: [key_doors, arena_entrances]
block_monster_lines: true
monsters:
  - { species: zombieman,   min: 10, max: 18 }
  - { species: shotgun_guy, min: 8,  max: 14 }
  - { species: imp,         min: 12, max: 20 }
  - { species: pinky,       min: 4,  max: 8 }
  - { species: cacodemon,   min: 0,  max: 3 }
  - { species: hell_knight, min: 1,  max: 2 }
";
        let c: Combat = serde_norway::from_str(y).unwrap();
        assert_eq!(c.encounter_style, EncounterStyle::Ambush);
        assert!((c.hitscanner_ratio - 0.35).abs() < f64::EPSILON);
        assert_eq!(c.max_simultaneous, 12);
        assert_eq!(c.monster_closets, 3);
        assert_eq!(c.boss, Boss::None);
        assert!((c.ambush.deaf_ratio - 0.4).abs() < f64::EPSILON);
        assert_eq!(c.ambush.teleport_ambushes, MinMax { min: 1, max: 3 });
        assert_eq!(c.sound.propagation, Propagation::Contained);
        assert_eq!(
            c.sound.block_sound_at,
            vec![SoundBlockSite::KeyDoors, SoundBlockSite::ArenaEntrances]
        );
        assert!(c.block_monster_lines);
        assert_eq!(c.monsters.len(), 6);
        assert_eq!(
            c.monsters[0],
            MonsterSpec {
                species: "zombieman".to_string(),
                min: 10,
                max: 18
            }
        );
    }
}
