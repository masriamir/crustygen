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

// ---------------------------------------------------------------------
// Weapons and ammo
// ---------------------------------------------------------------------

/// The `arsenal` group: weapon placement and ammo economy.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arsenal {
    /// Whether the map must stay winnable when the player pistol-starts it.
    pub pistol_start: PistolStart,
    /// The weapons the map places, in template order.
    pub weapons: Vec<WeaponSpec>,
    /// The map's ammo economy.
    pub ammo: Ammo,
}

/// Whether the map must stay winnable from a pistol start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PistolStart {
    /// The map must be completable with only the starting pistol and
    /// whatever it places itself.
    RequiredViable,
    /// The map may assume carried-over weapons from earlier maps.
    NotRequired,
}

/// One weapon the map places.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WeaponSpec {
    /// The vocabulary weapon name.
    pub name: String,
    /// Where in the map's progression the weapon appears.
    pub placement: Placement,
}

/// Where in the map's progression something appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// Appears early in the map.
    Early,
    /// Appears in the middle of the map.
    Mid,
    /// Appears late in the map.
    Late,
    /// Appears only in a secret.
    SecretOnly,
    /// Does not appear.
    None,
}

/// The map's ammo economy.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ammo {
    /// The overall ammo budget.
    pub budget: Budget,
    /// The ratio of placed ammo damage to total baseline monster HP;
    /// overrides `budget`.
    pub ratio: f64,
    /// How placed ammo is spread across the map's progression.
    pub distribution: Distribution,
    /// The ammo pickups to place.
    pub pickups: AmmoPickups,
    /// The backpack pickup, if any.
    pub backpack: CountPlacement,
}

/// An overall resource budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Budget {
    /// A tight budget, favoring scarcity.
    Tight,
    /// A balanced budget.
    Balanced,
    /// A generous budget, favoring abundance.
    Generous,
}

/// How placed ammo is spread across the map's progression.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Distribution {
    /// Weighted toward the start of the map.
    FrontLoaded,
    /// Spread evenly across the map.
    Even,
    /// Weighted toward the end of the map.
    BackLoaded,
}

/// The ammo pickups to place, either derived from [`Ammo::ratio`] or spelled
/// out explicitly.
///
/// Deserializes to a [`serde_norway::Value`] first and checks for the `auto`
/// string, mirroring [`AutoOr`]'s shape, but — since the explicit form is
/// itself a bare map rather than a value nested one level under an
/// author-chosen field — reports a fixed message naming both allowed forms
/// on failure rather than forwarding the inner map's own deserialize error.
#[derive(Debug, Clone, PartialEq)]
pub enum AmmoPickups {
    /// The author wrote `auto`: derive pickup counts from [`Ammo::ratio`].
    Auto,
    /// Explicit counts per pickup type, keyed by vocabulary pickup name.
    Explicit(std::collections::BTreeMap<String, u32>),
}

impl<'de> serde::Deserialize<'de> for AmmoPickups {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        const ALLOWED: &str = "expected `auto` or a map of pickup names to counts";
        let value = serde_norway::Value::deserialize(d)?;
        if matches!(&value, serde_norway::Value::String(s) if s == "auto") {
            return Ok(Self::Auto);
        }
        std::collections::BTreeMap::<String, u32>::deserialize(value)
            .map(Self::Explicit)
            .map_err(|_| serde::de::Error::custom(ALLOWED))
    }
}

/// A pickup count paired with where it appears.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountPlacement {
    /// The number of pickups; 0 means deliberately absent.
    pub count: u32,
    /// Where in the map's progression the pickups appear.
    pub placement: Placement,
}

// ---------------------------------------------------------------------
// Health, armor, powerups
// ---------------------------------------------------------------------

/// The `sustain` group: health, armor, and powerup placement.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sustain {
    /// The overall health budget; explicit counts in [`Sustain::health`]
    /// override it.
    pub health_budget: Budget,
    /// The map's health pickup counts.
    pub health: Health,
    /// The map's armor pickup counts.
    pub armor: Armor,
    /// The map's powerup placements.
    pub powerups: Vec<PowerupSpec>,
}

/// The map's health pickup counts.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Health {
    /// The number of stimpacks.
    pub stimpack: u32,
    /// The number of medikits.
    pub medikit: u32,
    /// The number of +1 health bonuses; they matter for a tight budget.
    pub health_bonus: u32,
}

/// The map's armor pickup counts.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Armor {
    /// The number of green armor pickups.
    pub green: u32,
    /// The number of blue armor pickups.
    pub blue: u32,
    /// The number of +1 armor bonuses.
    pub armor_bonus: u32,
}

/// One powerup's placement.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PowerupSpec {
    /// The vocabulary powerup name.
    pub name: String,
    /// The number placed; 0 means deliberately absent.
    pub count: u32,
    /// Where in the map's progression the powerup appears.
    pub placement: Placement,
}

// ---------------------------------------------------------------------
// Secrets
// ---------------------------------------------------------------------

/// The `secrets` group: how many secrets the map has.
///
/// Per-secret detail — what each one is and how it is hinted — lives in the
/// spec's Markdown body, not here.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Secrets {
    /// The number of secrets.
    pub count: u32,
}

// ---------------------------------------------------------------------
// Difficulty
// ---------------------------------------------------------------------

/// The `difficulty` group: skill-level support and scaling.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Difficulty {
    /// Whether to emit real easy/medium/hard thing flags.
    pub skills_supported: bool,
    /// The skill the map's other counts describe.
    pub baseline: SkillName,
    /// The overall shape of the difficulty curve.
    pub curve: Curve,
    /// The scaling factors applied to the baseline counts per skill tier.
    pub scaling: Scaling,
}

/// A Doom skill level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillName {
    /// "I'm too young to die."
    Itytd,
    /// "Hey, not too rough."
    Hntr,
    /// "Hurt me plenty."
    Hmp,
    /// "Ultra-violence."
    Uv,
    /// "Nightmare!"
    Nm,
}

/// The overall shape of the difficulty curve.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// The difficulty rises gradually.
    Gentle,
    /// The difficulty rises sharply.
    Steep,
    /// The difficulty stays easy until a late spike.
    LateSpike,
}

/// The scaling factors applied to the baseline counts per skill tier.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scaling {
    /// The scaling factor for the easy tier (ITYTD/HNTR).
    pub easy: f64,
    /// The scaling factor for the medium tier (HMP).
    pub medium: f64,
    /// The scaling factor for the hard tier (UV/NM).
    pub hard: f64,
}

// ---------------------------------------------------------------------
// Aesthetics
// ---------------------------------------------------------------------

/// The `aesthetics` group: theme, texturing, and lighting.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Aesthetics {
    /// The map's visual theme, as a vocabulary theme name.
    pub theme: String,
    /// The wall texture names to draw from.
    pub texture_set: AutoOr<Vec<String>>,
    /// The detail-pass intensity, from 1 to 5.
    pub detail_level: u8,
    /// The map's lighting design.
    pub lighting: Lighting,
    /// The sky texture.
    pub sky: AutoOr<String>,
    /// The music track.
    pub music: AutoOr<String>,
    /// Whether wall texture scaling may be emitted.
    pub texture_scaling: TextureScaling,
}

/// Whether wall texture scaling may be emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureScaling {
    /// Never emit `scalex`/`scaley` (see rule P9).
    Forbidden,
    /// Texture scaling may be emitted.
    Allowed,
}

/// The map's lighting design.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lighting {
    /// The overall lighting style.
    pub style: LightStyle,
    /// The default sector light level where nothing else applies.
    pub base: i32,
    /// The floor for every emitted light level (rule P19).
    pub min: i32,
    /// The ceiling for every emitted light level (rule P19).
    pub max: i32,
    /// The delta that counts as a deliberate light change (rule P21).
    pub contrast_step: i32,
    /// The corridor light level, relative to the rooms a corridor joins.
    pub corridor_delta: i32,
    /// The light level for sky-ceilinged sectors.
    pub outdoor: i32,
    /// The map's light-effect tuning.
    pub effects: LightEffectsSpec,
    /// Whether individual rooms may set their own light level and effect in
    /// the IR.
    pub per_room_overrides: bool,
}

/// The map's overall lighting style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightStyle {
    /// Uniform lighting throughout.
    Flat,
    /// Deliberate light-level contrast between areas.
    Contrasty,
    /// Small pools of darkness amid brighter surroundings.
    PoolsOfDark,
}

/// The map's light-effect tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightEffectsSpec {
    /// The light effects allowed anywhere in the map; empty means none.
    pub allowed: Vec<LightEffect>,
    /// How often allowed effects appear.
    pub density: Density,
    /// Sites where light effects are forbidden regardless of `allowed`.
    pub forbid_in: Vec<EffectSite>,
}

/// A dynamic light effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LightEffect {
    /// A regular on/off blink.
    Blink,
    /// An irregular flicker.
    Flicker,
    /// A smooth glow, cycling up and down.
    Glow,
    /// A slow strobe.
    StrobeSlow,
}

/// How often something appears.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    /// Never appears.
    None,
    /// Appears rarely.
    Sparse,
    /// Appears at a moderate rate.
    Medium,
    /// Appears often.
    Dense,
}

/// A site where light effects (or other placement) are forbidden.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectSite {
    /// Combat arenas; no strobing mid-fight.
    CombatArenas,
    /// The rooms holding secret rewards.
    SecretRewards,
}

// ---------------------------------------------------------------------
// Flats and liquids
// ---------------------------------------------------------------------

/// The `flats` group: floor and ceiling flats, and liquid hazards.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flats {
    /// The floor flat names to draw from.
    pub floor: AutoOr<Vec<String>>,
    /// The ceiling flat names to draw from.
    pub ceiling: AutoOr<Vec<String>>,
    /// The fraction of floor area with a sky ceiling.
    pub outdoor_proportion: f64,
    /// Whether to use bright ceiling flats beneath light sources.
    pub light_flats: bool,
    /// The map's liquid hazard, if any.
    pub liquid: Liquid,
}

/// The map's liquid hazard.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Liquid {
    /// The liquid's kind.
    pub kind: LiquidKind,
    /// Whether the liquid pairs a damaging sector special with its flat
    /// (see rule P16).
    pub damaging: bool,
    /// The damage tier, resolved to a sector special via `engine.toml`.
    pub damage_tier: DamageTier,
    /// The fraction of floor area the liquid covers.
    pub coverage: f64,
    /// Whether the player must cross the liquid to progress.
    pub crossing_required: bool,
    /// Whether a radsuit is provided; if `crossing_required`, either this or
    /// the health budget must cover the crossing (rule P17).
    pub radsuit_provided: bool,
}

/// A liquid kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LiquidKind {
    /// No liquid.
    None,
    /// Nukage.
    Nukage,
    /// Blood.
    Blood,
    /// Lava.
    Lava,
    /// Slime.
    Slime,
    /// Water.
    Water,
}

/// A damage tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DamageTier {
    /// Light damage.
    Light,
    /// Medium damage.
    Medium,
    /// Heavy damage.
    Heavy,
}

// ---------------------------------------------------------------------
// Vertical form
// ---------------------------------------------------------------------

/// The `vertical` group: stairs and standard vertical dimensions.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Vertical {
    /// The map's stair tuning.
    pub stairs: Stairs,
    /// The default room height where the spec says nothing.
    pub standard_ceiling: i32,
    /// The nominal door height; the effective opening is derived per rule
    /// P4.
    pub door_opening: i32,
}

/// The map's stair tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stairs {
    /// The number of stair flights.
    pub flights: MinMax<u32>,
    /// The rise per step, uniform within a flight; must not exceed the
    /// engine's maximum step height (rule P1).
    pub rise_per_step: i32,
    /// The tread depth; must be at least the player's diameter (rule P1).
    pub tread_depth: i32,
}

// ---------------------------------------------------------------------
// Scenery: decoration, light sources, hazards
// ---------------------------------------------------------------------

/// The `scenery` group: decoration, light sources, gore, and barrels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scenery {
    /// The map's light-source prop tuning.
    pub light_sources: LightSources,
    /// The map's decoration prop tuning.
    pub decorations: Decorations,
    /// How much gore the map places.
    pub gore: Gore,
    /// The map's explosive barrel tuning.
    pub barrels: Barrels,
}

/// The map's light-source prop tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightSources {
    /// How often light-source props appear.
    pub density: Density,
    /// The light-source prop kinds to draw from.
    pub kinds: AutoOr<Vec<String>>,
    /// Whether every bright pool gets a visible source (rule P21).
    pub match_lighting: bool,
}

/// The map's decoration prop tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decorations {
    /// How often decoration props appear.
    pub density: Density,
    /// The decoration prop kinds to draw from.
    pub kinds: AutoOr<Vec<String>>,
    /// Whether movement-blocking props are allowed, still subject to rule
    /// P3.
    pub blocking_allowed: bool,
    /// Whether ceiling-mounted props are allowed, subject to headroom
    /// (rule P22).
    pub hanging_allowed: bool,
}

/// How much gore the map places: corpses, blood, impaled bodies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Gore {
    /// No gore.
    None,
    /// A light amount of gore.
    Light,
    /// A heavy amount of gore.
    Heavy,
}

/// The map's explosive barrel tuning.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Barrels {
    /// The number of barrels.
    pub count: MinMax<u32>,
    /// Where barrels are placed.
    pub placement: BarrelPlacement,
    /// Whether one barrel's explosion may chain into another's.
    pub chain_reactions: ChainReactions,
    /// Sites barrels must be kept clear of (rule P23).
    pub keep_clear_of: Vec<KeepClearSite>,
}

/// Where barrels are placed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BarrelPlacement {
    /// Near encounters, to be used as a weapon.
    NearEncounters,
    /// Scattered without regard to encounters.
    Scattered,
    /// No barrels are placed.
    None,
}

/// Whether one barrel's explosion may chain into another's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChainReactions {
    /// Chain reactions are allowed.
    Allowed,
    /// Barrels are placed to avoid chain reactions.
    Avoided,
}

/// A site barrels must be kept clear of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeepClearSite {
    /// The player start.
    PlayerStart,
    /// A key pickup.
    KeyPickup,
    /// A secret's reward.
    SecretReward,
}

// ---------------------------------------------------------------------
// Pacing
// ---------------------------------------------------------------------

/// The `pacing` group: how the map's intensity rises and falls.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pacing {
    /// The number of distinct encounter beats.
    pub encounter_beats: MinMax<u32>,
    /// The number of rest areas between encounters.
    pub rest_areas: MinMax<u32>,
    /// Where the hardest fight sits, as a fraction of progression.
    pub peak_position: f64,
    /// The intensity of the map's opening.
    pub opening_intensity: Intensity,
}

/// An intensity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Intensity {
    /// Low intensity.
    Low,
    /// Medium intensity.
    Medium,
    /// High intensity.
    High,
}

// ---------------------------------------------------------------------
// Compatibility and metadata
// ---------------------------------------------------------------------

/// The `compat` group: source port targeting and metadata emission.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Compat {
    /// The source port the map targets.
    pub port: Port,
    /// Whether to emit a `MAPINFO` lump; v1 otherwise emits no extra lumps.
    pub emit_mapinfo: bool,
    /// The par time, in seconds; ignored unless `emit_mapinfo` is true.
    pub par_time_seconds: u32,
    /// The map's automap behavior.
    pub automap: Automap,
}

/// A source port compatibility target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Port {
    /// Vanilla Doom's hard engine limits.
    VanillaLimits,
    /// A limit-removing port.
    LimitRemoving,
    /// Boom or a Boom-compatible port.
    Boom,
    /// `ZDoom` or a `ZDoom`-family port.
    Zdoom,
}

/// The map's automap behavior.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Automap {
    /// Whether a secret door is hidden from reading as a door on the
    /// automap.
    pub hide_secret_lines: bool,
    /// Whether map lines are shown before the player discovers them.
    pub show_map_lines: AutoOr<bool>,
}

// ---------------------------------------------------------------------
// Constraints and priorities
// ---------------------------------------------------------------------

/// The `constraints` group: enforcement mode and conflict-resolution
/// priorities.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Constraints {
    /// Whether ranges elsewhere in the spec are hard limits or goals.
    pub enforcement: Enforcement,
    /// Vocabulary names (monsters, mechanics) the map must not use.
    pub forbid: Vec<String>,
    /// Free-text inspirations guiding generation.
    pub inspirations: Vec<String>,
    /// Free-text requirements the map must satisfy.
    pub must_include: Vec<String>,
    /// The order, highest first, in which conflicts between everything
    /// above are resolved.
    pub priority: Vec<Priority>,
}

/// Whether ranges elsewhere in the spec are hard limits or goals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    /// Ranges are hard limits.
    Strict,
    /// Ranges are goals.
    Target,
}

/// A conflict-resolution priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    /// The progression graph must be structurally correct.
    ProgressionCorrectness,
    /// The map must be playable and balanced.
    PlayableBalance,
    /// The sector count budget.
    SectorBudget,
    /// The monster count budgets.
    MonsterCounts,
    /// The detail-pass intensity.
    DetailLevel,
    /// The expected play time.
    PlayTime,
}

// ---------------------------------------------------------------------
// The frontmatter root
// ---------------------------------------------------------------------

/// The full frontmatter of a filled map-spec template: `spec_version` plus
/// all seventeen groups, per `docs/map-spec.md`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Frontmatter {
    /// The frontmatter schema version this document was written against.
    pub spec_version: u32,
    /// The `identity` group: which map this is and what it targets.
    pub identity: Identity,
    /// The `players` group: starts and multiplayer behavior.
    pub players: Players,
    /// The `scale` group: the size and count budgets the map should land
    /// within.
    pub scale: Scale,
    /// The `progression` group: how the player moves through the map.
    pub progression: Progression,
    /// The `architecture` group: the geometric character of the map's
    /// rooms.
    pub architecture: Architecture,
    /// The `combat` group: encounter design and monster placement.
    pub combat: Combat,
    /// The `arsenal` group: weapon placement and ammo economy.
    pub arsenal: Arsenal,
    /// The `sustain` group: health, armor, and powerup placement.
    pub sustain: Sustain,
    /// The `secrets` group: how many secrets the map has.
    pub secrets: Secrets,
    /// The `difficulty` group: skill-level support and scaling.
    pub difficulty: Difficulty,
    /// The `aesthetics` group: theme, texturing, and lighting.
    pub aesthetics: Aesthetics,
    /// The `flats` group: floor and ceiling flats, and liquid hazards.
    pub flats: Flats,
    /// The `vertical` group: stairs and standard vertical dimensions.
    pub vertical: Vertical,
    /// The `scenery` group: decoration, light sources, gore, and barrels.
    pub scenery: Scenery,
    /// The `pacing` group: how the map's intensity rises and falls.
    pub pacing: Pacing,
    /// The `compat` group: source port targeting and metadata emission.
    pub compat: Compat,
    /// The `constraints` group: enforcement mode and conflict-resolution
    /// priorities.
    pub constraints: Constraints,
}

/// Parses the YAML frontmatter of a spec document.
///
/// Deserialization runs through `serde_path_to_error`, so a type or enum
/// mistake names the exact field (`progression.doors.lock_types[1]`)
/// rather than a line number.
///
/// # Errors
///
/// Returns [`crate::spec::SpecError::Frontmatter`] if `yaml` does not
/// deserialize into [`Frontmatter`], naming the field path and the inner
/// serde message.
pub fn parse(yaml: &str) -> Result<Frontmatter, crate::spec::SpecError> {
    let de = serde_norway::Deserializer::from_str(yaml);
    serde_path_to_error::deserialize(de).map_err(|e| crate::spec::SpecError::Frontmatter {
        path: e.path().to_string(),
        message: e.inner().to_string(),
    })
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

    #[test]
    fn the_arsenal_group_from_the_design_doc_parses() {
        let y = r"
pistol_start: required_viable
weapons:
  - { name: shotgun,         placement: early }
  - { name: chaingun,        placement: mid }
  - { name: super_shotgun,   placement: mid }
  - { name: rocket_launcher, placement: secret_only }
ammo:
  budget: balanced
  ratio: 1.25
  distribution: even
  pickups: auto
  backpack: { count: 1, placement: mid }
";
        let a: Arsenal = serde_norway::from_str(y).unwrap();
        assert_eq!(a.pistol_start, PistolStart::RequiredViable);
        assert_eq!(a.weapons.len(), 4);
        assert_eq!(
            a.weapons[0],
            WeaponSpec {
                name: "shotgun".to_string(),
                placement: Placement::Early
            }
        );
        assert_eq!(a.ammo.budget, Budget::Balanced);
        assert!((a.ammo.ratio - 1.25).abs() < f64::EPSILON);
        assert_eq!(a.ammo.distribution, Distribution::Even);
        assert_eq!(a.ammo.pickups, AmmoPickups::Auto);
        assert_eq!(
            a.ammo.backpack,
            CountPlacement {
                count: 1,
                placement: Placement::Mid
            }
        );
    }

    #[test]
    fn the_sustain_group_from_the_design_doc_parses() {
        let y = r"
health_budget: balanced
health:
  stimpack: 6
  medikit: 4
  health_bonus: 20
armor:
  green: 2
  blue: 0
  armor_bonus: 15
powerups:
  - { name: berserk,         count: 1, placement: secret_only }
  - { name: soulsphere,      count: 1, placement: late }
  - { name: megasphere,      count: 0, placement: none }
  - { name: radsuit,         count: 1, placement: mid }
  - { name: invulnerability, count: 0, placement: none }
  - { name: invisibility,    count: 0, placement: none }
  - { name: light_amp,       count: 0, placement: none }
  - { name: computer_map,    count: 1, placement: secret_only }
";
        let s: Sustain = serde_norway::from_str(y).unwrap();
        assert_eq!(s.health_budget, Budget::Balanced);
        assert_eq!(
            s.health,
            Health {
                stimpack: 6,
                medikit: 4,
                health_bonus: 20
            }
        );
        assert_eq!(
            s.armor,
            Armor {
                green: 2,
                blue: 0,
                armor_bonus: 15
            }
        );
        assert_eq!(s.powerups.len(), 8);
        assert_eq!(
            s.powerups[0],
            PowerupSpec {
                name: "berserk".to_string(),
                count: 1,
                placement: Placement::SecretOnly
            }
        );
    }

    #[test]
    fn the_secrets_group_from_the_design_doc_parses() {
        let secrets: Secrets = serde_norway::from_str("count: 3\n").unwrap();
        assert_eq!(secrets.count, 3);
    }

    #[test]
    fn the_difficulty_group_from_the_design_doc_parses() {
        let y = r"
skills_supported: true
baseline: uv
curve: gentle
scaling: { easy: 0.55, medium: 0.75, hard: 1.0 }
";
        let d: Difficulty = serde_norway::from_str(y).unwrap();
        assert!(d.skills_supported);
        assert_eq!(d.baseline, SkillName::Uv);
        assert_eq!(d.curve, Curve::Gentle);
        assert!((d.scaling.easy - 0.55).abs() < f64::EPSILON);
        assert!((d.scaling.medium - 0.75).abs() < f64::EPSILON);
        assert!((d.scaling.hard - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn the_aesthetics_group_from_the_design_doc_parses() {
        let y = r"
theme: tech_base
texture_set: auto
detail_level: 3
lighting:
  style: contrasty
  base: 160
  min: 96
  max: 208
  contrast_step: 32
  corridor_delta: -16
  outdoor: 192
  effects:
    allowed: [blink, flicker, glow, strobe_slow]
    density: sparse
    forbid_in: [combat_arenas, secret_rewards]
  per_room_overrides: true
sky: auto
music: auto
texture_scaling: forbidden
";
        let a: Aesthetics = serde_norway::from_str(y).unwrap();
        assert_eq!(a.theme, "tech_base");
        assert_eq!(a.texture_set, AutoOr::Auto);
        assert_eq!(a.detail_level, 3);
        assert_eq!(a.lighting.style, LightStyle::Contrasty);
        assert_eq!(a.lighting.base, 160);
        assert_eq!(
            a.lighting.effects.allowed,
            vec![
                LightEffect::Blink,
                LightEffect::Flicker,
                LightEffect::Glow,
                LightEffect::StrobeSlow
            ]
        );
        assert_eq!(a.lighting.effects.density, Density::Sparse);
        assert_eq!(a.sky, AutoOr::Auto);
        assert_eq!(a.texture_scaling, TextureScaling::Forbidden);
    }

    #[test]
    fn the_flats_group_from_the_design_doc_parses() {
        let y = r"
floor: auto
ceiling: auto
outdoor_proportion: 0.15
light_flats: true
liquid:
  kind: nukage
  damaging: true
  damage_tier: light
  coverage: 0.08
  crossing_required: true
  radsuit_provided: true
";
        let f: Flats = serde_norway::from_str(y).unwrap();
        assert_eq!(f.floor, AutoOr::Auto);
        assert!(f.light_flats);
        assert_eq!(f.liquid.kind, LiquidKind::Nukage);
        assert_eq!(f.liquid.damage_tier, DamageTier::Light);
        assert!(f.liquid.crossing_required);
        assert!((f.outdoor_proportion - 0.15).abs() < f64::EPSILON);
    }

    #[test]
    fn the_vertical_group_from_the_design_doc_parses() {
        let y = r"
stairs:
  flights: { min: 1, max: 3 }
  rise_per_step: 16
  tread_depth: 32
standard_ceiling: 128
door_opening: 128
";
        let v: Vertical = serde_norway::from_str(y).unwrap();
        assert_eq!(v.stairs.flights, MinMax { min: 1, max: 3 });
        assert_eq!(v.stairs.rise_per_step, 16);
        assert_eq!(v.standard_ceiling, 128);
        assert_eq!(v.door_opening, 128);
    }

    #[test]
    fn the_scenery_group_from_the_design_doc_parses() {
        let y = r"
light_sources:
  density: medium
  kinds: auto
  match_lighting: true
decorations:
  density: medium
  kinds: auto
  blocking_allowed: true
  hanging_allowed: true
gore: light
barrels:
  count: { min: 4, max: 10 }
  placement: near_encounters
  chain_reactions: allowed
  keep_clear_of: [player_start, key_pickup, secret_reward]
";
        let s: Scenery = serde_norway::from_str(y).unwrap();
        assert_eq!(s.light_sources.density, Density::Medium);
        assert_eq!(s.light_sources.kinds, AutoOr::Auto);
        assert!(s.light_sources.match_lighting);
        assert_eq!(s.gore, Gore::Light);
        assert_eq!(s.barrels.count, MinMax { min: 4, max: 10 });
        assert_eq!(s.barrels.placement, BarrelPlacement::NearEncounters);
        assert_eq!(
            s.barrels.keep_clear_of,
            vec![
                KeepClearSite::PlayerStart,
                KeepClearSite::KeyPickup,
                KeepClearSite::SecretReward
            ]
        );
    }

    #[test]
    fn the_pacing_group_from_the_design_doc_parses() {
        let y = r"
encounter_beats: { min: 5, max: 8 }
rest_areas: { min: 2, max: 4 }
peak_position: 0.8
opening_intensity: low
";
        let p: Pacing = serde_norway::from_str(y).unwrap();
        assert_eq!(p.encounter_beats, MinMax { min: 5, max: 8 });
        assert_eq!(p.rest_areas, MinMax { min: 2, max: 4 });
        assert!((p.peak_position - 0.8).abs() < f64::EPSILON);
        assert_eq!(p.opening_intensity, Intensity::Low);
    }

    #[test]
    fn the_compat_group_from_the_design_doc_parses() {
        let y = r"
port: limit_removing
emit_mapinfo: false
par_time_seconds: 300
automap:
  hide_secret_lines: true
  show_map_lines: auto
";
        let c: Compat = serde_norway::from_str(y).unwrap();
        assert_eq!(c.port, Port::LimitRemoving);
        assert!(!c.emit_mapinfo);
        assert_eq!(c.par_time_seconds, 300);
        assert!(c.automap.hide_secret_lines);
        assert_eq!(c.automap.show_map_lines, AutoOr::Auto);
    }

    #[test]
    fn the_constraints_group_from_the_design_doc_parses() {
        let y = r#"
enforcement: target
forbid: [archvile, crusher, dark_maze, insta_death_pit]
inspirations:
  - "pacing like Doom II MAP07"
  - "texture discipline like Plutonia"
must_include:
  - "a window overlooking the final arena, visible from the start"
priority:
  - progression_correctness
  - playable_balance
  - sector_budget
  - monster_counts
  - detail_level
  - play_time
"#;
        let c: Constraints = serde_norway::from_str(y).unwrap();
        assert_eq!(c.enforcement, Enforcement::Target);
        assert_eq!(
            c.forbid,
            vec![
                "archvile".to_string(),
                "crusher".to_string(),
                "dark_maze".to_string(),
                "insta_death_pit".to_string()
            ]
        );
        assert_eq!(
            c.must_include,
            vec!["a window overlooking the final arena, visible from the start".to_string()]
        );
        assert_eq!(c.priority.len(), 6);
        assert_eq!(c.priority[0], Priority::ProgressionCorrectness);
        assert_eq!(c.priority[5], Priority::PlayTime);
    }

    #[test]
    fn ammo_pickups_auto_and_an_explicit_count_map_both_parse() {
        assert_eq!(
            serde_norway::from_str::<AmmoPickups>("auto").unwrap(),
            AmmoPickups::Auto
        );
        let mut expected = std::collections::BTreeMap::new();
        expected.insert("shells".to_string(), 4);
        expected.insert("rocket".to_string(), 2);
        assert_eq!(
            serde_norway::from_str::<AmmoPickups>("{ shells: 4, rocket: 2 }").unwrap(),
            AmmoPickups::Explicit(expected)
        );
    }

    #[test]
    fn an_unknown_priority_entry_is_rejected() {
        let y = r"
enforcement: target
forbid: []
inspirations: []
must_include: []
priority: [speed]
";
        assert!(serde_norway::from_str::<Constraints>(y).is_err());
    }

    #[test]
    fn the_shipped_template_frontmatter_parses_end_to_end() {
        let text = include_str!("../../map-spec.template.md");
        let (yaml, _) = crate::spec::split_frontmatter(text).unwrap();
        let fm = parse(&yaml).unwrap();
        assert_eq!(fm.spec_version, 1);
        assert_eq!(fm.secrets.count, 3);
        assert_eq!(fm.combat.monsters.len(), 6);
        assert_eq!(fm.constraints.enforcement, Enforcement::Target);
    }

    #[test]
    fn a_type_error_reports_the_exact_field_path() {
        let text = include_str!("../../map-spec.template.md");
        let (yaml, _) = crate::spec::split_frontmatter(text).unwrap();
        let broken = yaml.replace("locked_doors: 2", "locked_doors: two");
        let err = parse(&broken).unwrap_err();
        let crate::spec::SpecError::Frontmatter { path, .. } = err else {
            panic!("expected a Frontmatter error, got {err:?}")
        };
        assert_eq!(path, "progression.locked_doors");
    }

    #[test]
    fn an_unknown_top_level_group_is_rejected_with_its_name() {
        let text = include_str!("../../map-spec.template.md");
        let (yaml, _) = crate::spec::split_frontmatter(text).unwrap();
        let broken = format!("{yaml}extras:\n  cake: true\n");
        assert!(parse(&broken).is_err());
    }

    #[test]
    fn a_missing_group_is_rejected_naming_the_group() {
        let text = include_str!("../../map-spec.template.md");
        let (yaml, _) = crate::spec::split_frontmatter(text).unwrap();
        let broken: String = yaml
            .lines()
            .filter(|l| *l != "secrets:" && !l.starts_with("  count: 3"))
            .fold(String::new(), |mut acc, l| {
                acc.push_str(l);
                acc.push('\n');
                acc
            });
        let err = parse(&broken).unwrap_err();
        assert!(err.to_string().contains("secrets"), "got: {err}");
    }

    #[test]
    fn a_facing_that_is_neither_string_nor_integer_is_rejected() {
        let err = serde_norway::from_str::<Facing>("[east]").unwrap_err();
        assert!(err.to_string().contains("north"), "got: {err}");
    }

    #[test]
    fn a_boss_that_is_not_a_string_is_rejected() {
        let err = serde_norway::from_str::<Boss>("7").unwrap_err();
        assert!(err.to_string().contains("mastermind"), "got: {err}");
    }
}
