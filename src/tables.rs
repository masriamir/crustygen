//! Loads the sourced engine-constant and vocabulary tables.

use std::collections::{BTreeSet, HashMap};

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

/// The four ammunition pools weapons draw from (`ammotype_t` in the pinned
/// Doom source: `am_clip`, `am_shell`, `am_cell`, `am_misl`, in that order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmmoType {
    /// Bullets (`am_clip`) — pistol and chaingun ammo.
    Bullets,
    /// Shotgun shells (`am_shell`) — shotgun and super shotgun ammo.
    Shells,
    /// Plasma cells (`am_cell`) — plasma rifle and BFG9000 ammo.
    Cells,
    /// Rockets (`am_misl`) — rocket launcher ammo.
    Rockets,
}

/// A weapon's ammo draw and expected damage, as loaded from `engine.toml`'s
/// `[weapons.damage.*]` table.
///
/// Doom weapon damage is randomized per shot, so there is no "damage per
/// ammo unit" constant to read out of the engine — every field here beyond
/// `ammo_type`/`ammo_per_shot` is a COMPUTED value, derived from the
/// relevant fire-function formula and the real `P_Random()` lookup table's
/// empirical distribution (not an assumed-uniform one). See the matching
/// `[weapons.damage.*]` entry's `derivation` field for the arithmetic, and
/// its `source` field for where the underlying formula lives. `rocket_launcher`
/// and `bfg9000` in particular carry an explicit modeling assumption in
/// their `derivation` — see `engine.toml` for the full statement.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct WeaponDamage {
    /// Which ammo pool this weapon draws from.
    pub ammo_type: AmmoType,
    /// Units of `ammo_type` consumed per trigger pull.
    pub ammo_per_shot: i32,
    /// Expected damage dealt to a directly hit target per trigger pull.
    pub expected_damage_per_shot: f64,
    /// Expected damage per unit of `ammo_type` consumed
    /// (`expected_damage_per_shot / ammo_per_shot`) — the figure
    /// `arsenal.ammo.ratio` ("placed ammo damage / total baseline monster
    /// HP") needs.
    pub expected_damage_per_ammo: f64,
}

/// How much of which ammo pool a placed ammo pickup grants, as loaded from
/// `engine.toml`'s `[ammo.pickups.*]` table (all entries but `backpack`,
/// which has its own shape — see [`BackpackGrant`]).
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct AmmoPickup {
    /// How many units of `ammo_type` this pickup grants.
    pub amount: i32,
    /// Which ammo pool this pickup grants into.
    pub ammo_type: AmmoType,
}

/// The ammo a backpack grants — one full clipammo load of every ammo type
/// simultaneously (`P_TouchSpecialThing`'s `SPR_BPAK` case in the pinned
/// Doom source), unlike every other ammo pickup, which grants one amount of
/// one type. Loaded from `engine.toml`'s `[ammo.pickups.backpack]` table.
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct BackpackGrant {
    /// Bullets granted.
    pub bullets: i32,
    /// Shells granted.
    pub shells: i32,
    /// Cells granted.
    pub cells: i32,
    /// Rockets granted.
    pub rockets: i32,
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
struct Flat {
    tile: i32,
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
    /// The texture for a door's optional trim alcove sectors
    /// ([`crate::ir::Portal::alcove_near`]/[`crate::ir::Portal::alcove_far`]).
    trim: String,
    /// The floor flat of a teleport pad — the GATE family.
    pad: String,
    /// Width in pixels of the `switch` texture's canvas, so an exit line
    /// narrower than it can centre the texture rather than showing its
    /// left edge.
    switch_width: i32,
}

/// Sector specials keyed by tier name, as used for
/// `flats.liquid.damage_tier` (`light` | `medium` | `heavy`).
#[derive(Debug, Deserialize)]
struct DamageTiers {
    light: u16,
    medium: u16,
    heavy: u16,
}

/// Sector light-effect specials keyed by the template's own name, as used
/// for `sector.light_effects`. See `engine.toml`'s `[sector.light_effects]`
/// leading comment for the naming judgment mapping these four names onto
/// the engine's `P_SpawnSpecials` case comments and spawn-function names.
#[derive(Debug, Deserialize)]
struct LightEffects {
    blink: u16,
    flicker: u16,
    glow: u16,
    strobe_slow: u16,
}

/// Sector (not linedef) specials: a distinct numeric space from
/// `vocabulary.toml`'s `[specials]` table.
#[derive(Debug, Deserialize)]
struct SectorSpecials {
    secret: u16,
    damage: DamageTiers,
    light_effects: LightEffects,
}

/// The engine's hard caps on player spawn spots (`engine.toml`'s
/// `[starts]` table): `coop_max` bounds `player*_start` things, `dm_max`
/// bounds `deathmatch_start` things.
#[derive(Debug, Deserialize)]
struct Starts {
    coop_max: u32,
    dm_max: u32,
}

/// Engine-wide bounds unrelated to a specific mobj/sector/linedef family
/// (`engine.toml`'s `[game]` table): currently just the commercial map-slot
/// ceiling.
#[derive(Debug, Deserialize)]
struct Game {
    commercial_map_slots: u32,
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
    secret: u16,
    sound_block: u16,
    blocking: u16,
    two_sided: u16,
    upper_unpegged: u16,
    lower_unpegged: u16,
}

/// `[linedef.vanilla_specials]` — the membership list of every special the
/// pinned engine acts on; see the table's leading comment.
#[derive(Debug, Deserialize)]
struct VanillaSpecials {
    values: Vec<u16>,
}

#[derive(Debug, Deserialize)]
struct LinedefAttrs {
    flags: LinedefFlags,
    vanilla_specials: VanillaSpecials,
}

/// The empirical distribution of `P_Random()`'s real 256-entry lookup table
/// (`rndtable` in the pinned Doom source's `m_random.c`), computed by
/// reading the table rather than assuming it is uniform. Every
/// `[weapons.damage.*]` entry's `expected_damage_per_shot` derives from one
/// of these two means.
#[derive(Debug, Deserialize)]
struct RandomLut {
    /// The mean of `rndtable[i] % 3` for i in 0..256 — used by the
    /// `pistol`/`shotgun`/`chaingun`/`super_shotgun` bullet-and-pellet
    /// formula.
    mod3_mean: f64,
    /// The mean of `rndtable[i] % 8` (equivalently `& 7`) for i in 0..256 —
    /// used by the rocket/plasma/BFG missile direct-hit multiplier and the
    /// BFG's spray-tracer formula.
    mod8_mean: f64,
}

#[derive(Debug, Deserialize)]
struct WeaponsDamage {
    pistol: WeaponDamage,
    shotgun: WeaponDamage,
    super_shotgun: WeaponDamage,
    chaingun: WeaponDamage,
    rocket_launcher: WeaponDamage,
    plasma_rifle: WeaponDamage,
    bfg9000: WeaponDamage,
}

#[derive(Debug, Deserialize)]
struct Weapons {
    damage: WeaponsDamage,
}

#[derive(Debug, Deserialize)]
struct AmmoPickups {
    clip: AmmoPickup,
    box_of_bullets: AmmoPickup,
    shells: AmmoPickup,
    box_of_shells: AmmoPickup,
    rocket: AmmoPickup,
    box_of_rockets: AmmoPickup,
    cell_charge: AmmoPickup,
    cell_pack: AmmoPickup,
    backpack: BackpackGrant,
}

/// How much ammo a placed weapon pickup itself grants on first pickup
/// (`engine.toml`'s `[ammo.weapon_grant.*]` table) — distinct from
/// [`AmmoPickups`], which covers ammo-only pickups. The pistol (never a
/// placed pickup thing) and chainsaw (draws no ammo) are deliberately
/// absent; see that table's header comment.
#[derive(Debug, Deserialize)]
struct WeaponGrant {
    chaingun: AmmoPickup,
    shotgun: AmmoPickup,
    super_shotgun: AmmoPickup,
    rocket_launcher: AmmoPickup,
    plasma_rifle: AmmoPickup,
    bfg9000: AmmoPickup,
}

#[derive(Debug, Deserialize)]
struct AmmoTable {
    pickups: AmmoPickups,
    weapon_grant: WeaponGrant,
}

/// Thing attribute flag bits (`engine.toml` `[thing.flags]`).
#[derive(Debug, Deserialize)]
struct ThingFlags {
    ambush: u32,
}

/// `[thing]`: attribute tables for map things.
#[derive(Debug, Deserialize)]
struct ThingAttrs {
    flags: ThingFlags,
}

#[derive(Debug, Deserialize)]
struct Engine {
    movement: Movement,
    door: Door,
    flat: Flat,
    light: LightRange,
    player: ThingDims,
    species: HashMap<String, SpeciesEntry>,
    props: HashMap<String, PropDims>,
    pickups: HashMap<String, PickupEntry>,
    sector: SectorSpecials,
    linedef: LinedefAttrs,
    random: RandomLut,
    weapons: Weapons,
    ammo: AmmoTable,
    starts: Starts,
    game: Game,
    thing: ThingAttrs,
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

/// The four teleporter line specials, keyed by who may cross and whether
/// the line survives its first use.
#[derive(Debug, Deserialize)]
struct TeleportSpecials {
    repeatable: u16,
    one_shot: u16,
    monsters_only: u16,
    monsters_only_one_shot: u16,
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
    teleport: TeleportSpecials,
}

/// The curated (not sourced — see [`Tables::is_door_texture`]) list of
/// texture names recognized as genuine door-panel textures, loaded from
/// `vocabulary.toml`'s `[door_texture_catalog]` table.
#[derive(Debug, Deserialize)]
struct DoorTextureCatalog {
    names: Vec<String>,
}

/// The alcove trim marking a locked door, keyed by the key that opens it.
#[derive(Debug, Deserialize)]
struct KeyTrim {
    blue_card: String,
    blue_skull: String,
    red_card: String,
    red_skull: String,
    yellow_card: String,
    yellow_skull: String,
}

#[derive(Debug, Deserialize)]
struct Vocabulary {
    things: HashMap<String, toml::Value>,
    specials: Specials,
    textures: HashMap<String, TextureSet>,
    door_texture_catalog: DoorTextureCatalog,
    key_trim: KeyTrim,
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

    /// The side of one flat tile in world space, in map units.
    ///
    /// The renderer wraps a flat every `tile` units of world space, so a
    /// sector shows a 64x64 flat as exactly one tile only when its corners
    /// are multiples of it — the rule teleport pads are placed by. See
    /// [`crate::ir::Ir::FLAT_TILE`], which carries the same value for the
    /// table-free IR validation, and `data/engine.toml`'s `[flat]` citation.
    #[must_use]
    pub fn flat_tile(&self) -> i32 {
        self.engine.flat.tile
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

    /// Every `[things]` vocabulary entry, `(name, doomednum)`, skipping the
    /// `*_source` citation strings (their values are not integers). The reverse
    /// direction of [`Self::thing_id`], for classifying an emitted thing.
    pub fn thing_kinds(&self) -> impl Iterator<Item = (&str, u16)> + '_ {
        self.vocabulary.things.iter().filter_map(|(name, v)| {
            v.as_integer()
                .and_then(|i| u16::try_from(i).ok())
                .map(|id| (name.as_str(), id))
        })
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

    /// Every `(key kind, keyed-door special)` pair in the vocabulary,
    /// sorted by kind name.
    ///
    /// Sorted because the backing table is a `HashMap`. Class *numbering* is
    /// made deterministic downstream — [`crate::reach::graph_from_compiled`]
    /// sorts and dedups the specials themselves — but the per-class key-kind
    /// *names* (`class_names`, and so P7's report wording) inherit their
    /// order from this list, and unsorted iteration would make that wording
    /// nondeterministic.
    #[must_use]
    pub fn locked_door_kinds(&self) -> Vec<(String, u16)> {
        let mut kinds: Vec<(String, u16)> = self
            .vocabulary
            .specials
            .locked
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), u16::try_from(v.as_integer()?).ok()?)))
            .collect();
        kinds.sort();
        kinds
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

    /// The linedef special for a teleporter line. `monsters_only` selects the
    /// `!thing->player`-guarded pair (126/125); `repeatable` selects the
    /// RETRIGGERS form, which keeps its special after firing.
    #[must_use]
    pub fn teleport_special(&self, monsters_only: bool, repeatable: bool) -> u16 {
        let t = &self.vocabulary.specials.teleport;
        match (monsters_only, repeatable) {
            (false, true) => t.repeatable,
            (false, false) => t.one_shot,
            (true, true) => t.monsters_only,
            (true, false) => t.monsters_only_one_shot,
        }
    }

    /// All four teleporter specials, ascending.
    #[must_use]
    pub fn teleport_specials(&self) -> [u16; 4] {
        let t = &self.vocabulary.specials.teleport;
        let mut all = [
            t.repeatable,
            t.one_shot,
            t.monsters_only,
            t.monsters_only_one_shot,
        ];
        all.sort_unstable();
        all
    }

    /// The two specials any thing may cross: `[repeatable, one_shot]`.
    #[must_use]
    pub fn player_teleport_specials(&self) -> [u16; 2] {
        let t = &self.vocabulary.specials.teleport;
        [t.repeatable, t.one_shot]
    }

    /// The two monsters-only specials: `[repeatable, one_shot]`.
    #[must_use]
    pub fn monster_teleport_specials(&self) -> [u16; 2] {
        let t = &self.vocabulary.specials.teleport;
        [t.monsters_only, t.monsters_only_one_shot]
    }

    /// A thing attribute flag bit by its sourced name (`engine.toml`
    /// `[thing.flags]`): `"ambush"` is `MTF_AMBUSH`.
    #[must_use]
    pub fn thing_flag(&self, name: &str) -> Option<u32> {
        match name {
            "ambush" => Some(self.engine.thing.flags.ambush),
            _ => None,
        }
    }

    /// The sector special marking a sector "secret" (rule P18).
    #[must_use]
    pub fn secret_sector_special(&self) -> u16 {
        self.engine.sector.secret
    }

    /// Every linedef special a compiler pass writes today: the manual door,
    /// the keyed doors, the four exits, and the four teleport specials
    /// ([`crate::compile::teleports`]). Curated rather than "every
    /// accessor" — the lift specials are sourced in the table but no pass
    /// emits them. `tests/vocabulary_arbiter.rs` compiles a fixture per
    /// construct and asserts this set equals what came out. That does not
    /// detect a new pass on its own: no fixture can author a construct the
    /// IR cannot yet express, so a pass that lands before its IR construct
    /// leaves the fixtures' union unchanged. What it does enforce is that
    /// growing this list without a fixture that emits the new special breaks
    /// the equality — and that adding 62 or 88 breaks
    /// `sourced_but_unemitted_specials_stay_out_of_the_emittable_set` too.
    /// A new pass therefore lands its fixture and updates both tests by
    /// rule.
    #[must_use]
    pub fn emittable_line_specials(&self) -> BTreeSet<u16> {
        let mut set = BTreeSet::from([
            self.door_special(),
            self.exit_switch_special(),
            self.exit_walkover_special(),
            self.secret_exit_switch_special(),
            self.secret_exit_walkover_special(),
        ]);
        set.extend(self.locked_door_kinds().into_iter().map(|(_, s)| s));
        set.extend(self.teleport_specials());
        set
    }

    /// Every sector special `engine.toml` names — secret, the three damage
    /// tiers, the four light effects. "Nameable" is the expressibility
    /// criterion; which IR field carries the value is irrelevant.
    #[must_use]
    pub fn named_sector_specials(&self) -> BTreeSet<u16> {
        let s = &self.engine.sector;
        BTreeSet::from([
            s.secret,
            s.damage.light,
            s.damage.medium,
            s.damage.heavy,
            s.light_effects.blink,
            s.light_effects.flicker,
            s.light_effects.glow,
            s.light_effects.strobe_slow,
        ])
    }

    /// Every linedef special the pinned vanilla engine acts on
    /// (`engine.toml` `[linedef.vanilla_specials]`). Membership, not
    /// vocabulary: it defines the corpus's vanilla-only slice.
    #[must_use]
    pub fn vanilla_line_specials(&self) -> BTreeSet<u16> {
        self.engine
            .linedef
            .vanilla_specials
            .values
            .iter()
            .copied()
            .collect()
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

    /// The sector special for a named light effect (`blink` | `flicker` |
    /// `glow` | `strobe_slow`, matching the template's own vocabulary), if
    /// the name is known. See `engine.toml`'s `[sector.light_effects]`
    /// leading comment for how these four names map onto
    /// `P_SpawnSpecials`'s own case comments and spawn-function names — the
    /// mapping is an editorial judgment where the two vocabularies
    /// disagree, not a mechanical transcription.
    #[must_use]
    pub fn light_effect_special(&self, name: &str) -> Option<u16> {
        let effects = &self.engine.sector.light_effects;
        match name {
            "blink" => Some(effects.blink),
            "flicker" => Some(effects.flicker),
            "glow" => Some(effects.glow),
            "strobe_slow" => Some(effects.strobe_slow),
            _ => None,
        }
    }

    /// The maximum number of `player*_start` things the engine reads
    /// (`playerstarts[MAXPLAYERS]` in the pinned Doom source) — placing more
    /// than this is inert, not merely redundant.
    #[must_use]
    pub fn max_coop_starts(&self) -> u32 {
        self.engine.starts.coop_max
    }

    /// The maximum number of `deathmatch_start` things the engine's
    /// deathmatch spawn array is sized for (`deathmatchstarts[MAX_DM_STARTS]`
    /// in the pinned Doom source).
    #[must_use]
    pub fn max_dm_starts(&self) -> u32 {
        self.engine.starts.dm_max
    }

    /// The number of commercial (Doom II) map slots the engine's
    /// intermission and par-time tables are sized for.
    #[must_use]
    pub fn commercial_map_slots(&self) -> u32 {
        self.engine.game.commercial_map_slots
    }

    /// The `doomdata.h` bit value for a named linedef flag (`block_monsters`
    /// | `secret` | `sound_block` | `blocking` | `two_sided` |
    /// `upper_unpegged` | `lower_unpegged`), if known. `combat.block_monster_lines`
    /// needs `block_monsters` (`ML_BLOCKMONSTERS`); `combat.sound.block_sound_at`
    /// needs `sound_block` (`ML_SOUNDBLOCK`). `blocking`, `two_sided`
    /// (`ML_BLOCKING`, `ML_TWOSIDED`), `upper_unpegged`
    /// (`ML_DONTPEGTOP`), and `lower_unpegged` (`ML_DONTPEGBOTTOM`) are read
    /// back by [`crate::check::scene`] to re-derive a parsed map's own
    /// boundary passability and texture-pegging from its linedef `flags`
    /// bits, rather than trusting the compiler's structural output.
    ///
    /// UDMF's `doom` namespace spells each flag as its own named boolean
    /// field on the linedef object — `blockmonsters` and `blocksound`
    /// respectively — rather than this packed bit; see `emit_textmap`'s
    /// existing `blocking`/`dontpegbottom`/`dontpegtop` output for the
    /// convention a future emission path should follow. `block_monsters` and
    /// `sound_block` are sourced and accessible but unemitted; `secret` is
    /// wired into `compile::portals`.
    #[must_use]
    pub fn linedef_flag(&self, name: &str) -> Option<u16> {
        match name {
            "block_monsters" => Some(self.engine.linedef.flags.block_monsters),
            "secret" => Some(self.engine.linedef.flags.secret),
            "sound_block" => Some(self.engine.linedef.flags.sound_block),
            "blocking" => Some(self.engine.linedef.flags.blocking),
            "two_sided" => Some(self.engine.linedef.flags.two_sided),
            "upper_unpegged" => Some(self.engine.linedef.flags.upper_unpegged),
            "lower_unpegged" => Some(self.engine.linedef.flags.lower_unpegged),
            _ => None,
        }
    }

    /// The mean of `P_Random()`'s real 256-entry lookup table's values,
    /// taken modulo 3 — the distribution the `pistol`/`shotgun`/`chaingun`/
    /// `super_shotgun` bullet-and-pellet damage formula
    /// (`5*(P_Random()%3+1)`) actually draws from, not an assumed-uniform
    /// one. `weapon_damage`'s
    /// `expected_damage_per_shot` for those four weapons derives from this
    /// value; see `engine.toml`'s `[random]` table for the full histogram.
    #[must_use]
    pub fn random_mod3_mean(&self) -> f64 {
        self.engine.random.mod3_mean
    }

    /// The mean of `P_Random()`'s real 256-entry lookup table's values,
    /// taken modulo 8 (equivalently `& 7`) — the distribution the rocket
    /// launcher's and plasma rifle's direct-hit multiplier and the BFG's
    /// spray-tracer formula actually draw from, not an assumed-uniform one.
    /// See `engine.toml`'s `[random]` table for the full histogram.
    #[must_use]
    pub fn random_mod8_mean(&self) -> f64 {
        self.engine.random.mod8_mean
    }

    /// A named weapon's ammo draw and expected damage (`pistol` | `shotgun`
    /// | `super_shotgun` | `chaingun` | `rocket_launcher` | `plasma_rifle` |
    /// `bfg9000`), if listed. The chainsaw and fist draw no ammo and are
    /// deliberately absent — see `engine.toml`'s `[weapons.damage.*]`
    /// header comment.
    #[must_use]
    pub fn weapon_damage(&self, name: &str) -> Option<WeaponDamage> {
        let damage = &self.engine.weapons.damage;
        match name {
            "pistol" => Some(damage.pistol),
            "shotgun" => Some(damage.shotgun),
            "super_shotgun" => Some(damage.super_shotgun),
            "chaingun" => Some(damage.chaingun),
            "rocket_launcher" => Some(damage.rocket_launcher),
            "plasma_rifle" => Some(damage.plasma_rifle),
            "bfg9000" => Some(damage.bfg9000),
            _ => None,
        }
    }

    /// A named ammo pickup's grant amount and ammo type (`clip` |
    /// `box_of_bullets` | `shells` | `box_of_shells` | `rocket` |
    /// `box_of_rockets` | `cell_charge` | `cell_pack`), if listed.
    /// `backpack` is not a valid name here — it grants all four ammo types
    /// at once, so it has its own shape; see [`Tables::ammo_backpack_grant`].
    #[must_use]
    pub fn ammo_pickup(&self, name: &str) -> Option<AmmoPickup> {
        let pickups = &self.engine.ammo.pickups;
        match name {
            "clip" => Some(pickups.clip),
            "box_of_bullets" => Some(pickups.box_of_bullets),
            "shells" => Some(pickups.shells),
            "box_of_shells" => Some(pickups.box_of_shells),
            "rocket" => Some(pickups.rocket),
            "box_of_rockets" => Some(pickups.box_of_rockets),
            "cell_charge" => Some(pickups.cell_charge),
            "cell_pack" => Some(pickups.cell_pack),
            _ => None,
        }
    }

    /// The ammo a backpack grants — one full clipammo load of every ammo
    /// type simultaneously, unlike every other ammo pickup.
    #[must_use]
    pub fn ammo_backpack_grant(&self) -> BackpackGrant {
        self.engine.ammo.pickups.backpack
    }

    /// A named weapon's ammo grant from the weapon pickup itself (`chaingun`
    /// | `shotgun` | `super_shotgun` | `rocket_launcher` | `plasma_rifle` |
    /// `bfg9000`), if listed — distinct from [`Self::ammo_pickup`], which
    /// covers ammo-only pickups. The pistol (never a placed pickup thing)
    /// and chainsaw (draws no ammo) are deliberately absent from the
    /// vocabulary this covers; see `engine.toml`'s `[ammo.weapon_grant.*]`
    /// header comment.
    #[must_use]
    pub fn weapon_ammo_grant(&self, name: &str) -> Option<AmmoPickup> {
        let grant = &self.engine.ammo.weapon_grant;
        match name {
            "chaingun" => Some(grant.chaingun),
            "shotgun" => Some(grant.shotgun),
            "super_shotgun" => Some(grant.super_shotgun),
            "rocket_launcher" => Some(grant.rocket_launcher),
            "plasma_rifle" => Some(grant.plasma_rifle),
            "bfg9000" => Some(grant.bfg9000),
            _ => None,
        }
    }

    /// The texture for a role (`wall`, `floor`, `ceiling`, `door`,
    /// `door_track`, `switch`, `trim`, `pad`) under a theme, if both
    /// resolve.
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
            "trim" => Some(&set.trim),
            "pad" => Some(&set.pad),
            _ => None,
        }
    }

    /// The width in pixels of `theme`'s switch texture, or `None` when the
    /// theme is unknown.
    ///
    /// `compile::exits` centres the switch texture on an exit line narrower
    /// than the texture; without this an exit shows the texture's left edge
    /// and the switch graphic reads as off-centre.
    #[must_use]
    pub fn switch_width(&self, theme: &str) -> Option<i32> {
        Some(self.vocabulary.textures.get(theme)?.switch_width)
    }

    /// The alcove trim texture marking a door locked by `key`, or `None`
    /// for a key with no trim of its own.
    ///
    /// This is the trim on a locked portal's **alcove** jambs — the wall a
    /// player faces walking up to the door. A door's own track is never
    /// affected; it stays the theme's `door_track` unconditionally. See
    /// `vocabulary.toml`'s `[key_trim]` for the card-versus-skull
    /// convention and the corpus measurement behind it.
    #[must_use]
    pub fn key_trim(&self, key: &str) -> Option<&str> {
        let t = &self.vocabulary.key_trim;
        match key {
            "blue_card" => Some(&t.blue_card),
            "blue_skull" => Some(&t.blue_skull),
            "red_card" => Some(&t.red_card),
            "red_skull" => Some(&t.red_skull),
            "yellow_card" => Some(&t.yellow_card),
            "yellow_skull" => Some(&t.yellow_skull),
            _ => None,
        }
    }

    /// Whether `name` is a texture the project's curated catalog recognizes
    /// as a genuine door-panel texture — as opposed to a door's track, a
    /// plain wall, or any other role.
    ///
    /// **Not sourced from the engine.** Every other table in this file
    /// carries a `source` citation to the pinned Doom release or a primary
    /// spec; this one cannot, because which texture *names* "read as a
    /// door" is an asset-naming convention, not an engine constant and not
    /// derivable from one — nothing in `linuxdoom-1.10` enumerates "the
    /// door textures". `vocabulary.toml`'s `[door_texture_catalog]` table
    /// carries a `curated` field in place of the usual `source` for exactly
    /// this reason: it names how the list was built (the classic
    /// `BIGDOOR`/`DOOR*`/`SPCDOOR`/`ZDOOR*` id-Software/Freedoom texture
    /// families) and how it was verified (every name confirmed present as a
    /// composite texture in both Freedoom IWADs' `TEXTURE1` lump).
    #[must_use]
    pub fn is_door_texture(&self, name: &str) -> bool {
        self.vocabulary
            .door_texture_catalog
            .names
            .iter()
            .any(|n| n == name)
    }
}

#[cfg(test)]
mod tests {
    use super::{AmmoType, Tables};

    /// Compares two `f64` values within a small absolute tolerance rather
    /// than `==` — every derived weapon-damage/ammo-ratio figure this file
    /// checks is an exact dyadic rational (denominators are powers of two,
    /// per the `[random]`/`[weapons.damage.*]` derivations in
    /// `engine.toml`) and IEEE 754 arithmetic reproduces it exactly, but an
    /// epsilon comparison is still the idiomatic, robust way to assert on
    /// floats.
    fn approx_eq(actual: f64, expected: f64) -> bool {
        (actual - expected).abs() < 1e-9
    }

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
        assert_eq!(
            t.flat_tile(),
            crate::ir::Ir::FLAT_TILE,
            "the engine table's flat tile and the IR's own copy cannot drift"
        );
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

    #[test]
    fn locked_door_kinds_lists_every_key_sorted() {
        let t = Tables::load().expect("tables");
        let kinds = t.locked_door_kinds();
        let names: Vec<&str> = kinds.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            names,
            [
                "blue_card",
                "blue_skull",
                "red_card",
                "red_skull",
                "yellow_card",
                "yellow_skull"
            ]
        );
        for (kind, special) in &kinds {
            assert_eq!(
                t.locked_door_special(kind),
                Some(*special),
                "consistent with the single lookup"
            );
        }
    }

    /// Every name the compiler, the playability rules, or the design's
    /// template frontmatter (`docs/design.md`
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
        for s in t.teleport_specials() {
            assert_ne!(s, 0, "a teleport special exists");
        }
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
            Some(1),
            "ML_BLOCKING is now sourced too, for check::scene's Boundary::blocking"
        );
        assert_eq!(
            t.linedef_flag("plaid_flag"),
            None,
            "an unknown flag name must fail loudly, not silently fall back"
        );
    }

    #[test]
    fn thing_kinds_inverts_thing_id_and_skips_source_entries() {
        let t = Tables::load().expect("tables");
        let kinds: std::collections::HashMap<&str, u16> = t.thing_kinds().collect();
        assert_eq!(kinds.get("imp"), Some(&3001));
        assert_eq!(kinds.get("player1_start"), Some(&1));
        assert!(!kinds.contains_key("source"));
        for (name, id) in t.thing_kinds() {
            assert_eq!(t.thing_id(name), Some(id));
        }
    }

    #[test]
    fn the_verifier_linedef_flags_and_start_things_are_sourced() {
        let t = Tables::load().expect("tables");
        assert_eq!(t.linedef_flag("blocking"), Some(1));
        assert_eq!(t.linedef_flag("two_sided"), Some(4));
        assert_eq!(t.linedef_flag("upper_unpegged"), Some(8));
        assert_eq!(t.linedef_flag("lower_unpegged"), Some(16));
        assert_eq!(t.thing_id("player2_start"), Some(2));
        assert_eq!(t.thing_id("player3_start"), Some(3));
        assert_eq!(t.thing_id("player4_start"), Some(4));
        assert_eq!(t.thing_id("deathmatch_start"), Some(11));
    }

    /// The exploding barrel and the four original scenery/light-source
    /// props must resolve a doomednum, and the ones that block movement
    /// must also resolve `[props.*]` dims (rules P3, P21, P22), while the
    /// non-blocking ones must NOT carry a `[props.*]` entry — asserting
    /// that distinction directly rather than only checking `Some`/`None`
    /// loosely.
    ///
    /// This test covers the five rows that block was born with. The other
    /// twenty rows of vocabulary.toml's `# Hazards and scenery` block — the
    /// rest of the vanilla decoration set — are pinned by
    /// [`every_decoration_prop_resolves`] below. Between them the two tests
    /// cover all 25 rows and all 24 that block; `candle` is the single
    /// non-blocking one.
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
    /// resolve a doomednum, checked individually. vocabulary.toml's
    /// `# Gore` block names six groups over 34 rows, sixteen of which
    /// block; this test covers four of those groups — standing corpses (7),
    /// gib props (3), blood/bone floor decorations (6), and hanging bodies
    /// (6) — and the nine blocking rows among them (the three bone props
    /// and the six hanging bodies) must also resolve `[props.*]` dims. The
    /// six hanging bodies specifically must report `hangs = true` (rule
    /// P22) while the three bone props report `hangs = false` —
    /// floor-standing, not `MF_SPAWNCEILING`, despite sharing `MF_SOLID`
    /// with the hanging set.
    ///
    /// The remaining two groups — impaled bodies (2) and the meat-hook set
    /// (10), whose seven blocking rows complete the sixteen — are pinned by
    /// [`every_impaled_and_meat_hook_prop_resolves`] below.
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
        // `brain_stem` (MF_NOBLOCKMAP) joined this group with the complete
        // decoration set; vocabulary.toml files it beside the other two.
        let blood: &[(&str, u16)] = &[("small_pool", 80), ("colon_gibs", 79), ("brain_stem", 81)];
        for (name, doomednum) in blood {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            assert!(
                t.prop(name).is_none(),
                "`{name}` is a non-blocking blood decoration"
            );
        }
        assert_eq!(blood.len(), 3, "every blood decoration was checked");

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

    /// The rest of the vanilla decoration set — the twenty rows the 2026-08
    /// corpus measurement added to vocabulary.toml's `# Hazards and
    /// scenery` block. Every one is `MF_SOLID` with no `MF_SPAWNCEILING`,
    /// so every one must resolve `[props.*]` dims that block and do not
    /// hang. The dims are the ones `mobjinfo` gives at the pinned commit:
    /// 16x16 throughout, except `big_tree` at radius 32. A wrong radius
    /// here yields a map that loads, renders, and quietly embeds a prop in
    /// a wall — exactly the class of defect no other test catches, since
    /// every other test reads the same table the compiler does (see
    /// KNOWN-GAPS.md's sourcing rule).
    #[test]
    fn every_decoration_prop_resolves() {
        let t = Tables::load().expect("tables load");
        // (name, doomednum, radius, height) — all MF_SOLID, none hanging.
        let columns: &[(&str, u16, i32, i32)] = &[
            ("tall_green_column", 30, 16, 16),
            ("short_green_column", 31, 16, 16),
            ("tall_red_column", 32, 16, 16),
            ("short_red_column", 33, 16, 16),
            ("heart_column", 36, 16, 16),
            ("skull_column", 37, 16, 16),
            ("tech_pillar", 48, 16, 16),
        ];
        let torches: &[(&str, u16, i32, i32)] = &[
            ("tall_blue_torch", 44, 16, 16),
            ("tall_green_torch", 45, 16, 16),
            ("tall_red_torch", 46, 16, 16),
            ("short_blue_torch", 55, 16, 16),
            ("short_green_torch", 56, 16, 16),
            ("short_red_torch", 57, 16, 16),
        ];
        // Trees, cave rock, the second techno lamp, and the free-standing
        // light/skull/fire props. `big_tree` is the one radius-32 entry.
        let others: &[(&str, u16, i32, i32)] = &[
            ("torch_tree", 43, 16, 16),
            ("big_tree", 54, 32, 16),
            ("stalagmite", 47, 16, 16),
            ("short_techno_lamp", 86, 16, 16),
            ("evil_eye", 41, 16, 16),
            ("floating_skull", 42, 16, 16),
            ("burning_barrel", 70, 16, 16),
        ];

        let mut checked = 0;
        for (name, doomednum, radius, height) in columns.iter().chain(torches).chain(others) {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t.prop(name).unwrap_or_else(|| panic!("`{name}` prop dims"));
            assert_eq!(dims.radius, *radius, "`{name}` radius");
            assert_eq!(dims.height, *height, "`{name}` height");
            assert!(dims.blocks, "`{name}` blocks movement");
            assert!(!dims.hangs, "`{name}` stands on the floor");
            checked += 1;
        }
        assert_eq!(checked, 20, "every decoration was checked");
    }

    /// The two remaining gore groups: the impaled bodies (`MF_SOLID`,
    /// floor-standing) and the ten-strong meat-hook set, which `mobjinfo`
    /// splits down the middle — 49-53 carry `MF_SOLID` at radius 16 with
    /// per-entry heights and hang from the ceiling, while 59-63 are the
    /// same five sprites at radius 20 *without* `MF_SOLID`. That split is
    /// the point of the test: the walk-through twins must resolve a
    /// doomednum and still report no `[props.*]` entry at all, alongside
    /// `brain_stem` (`MF_NOBLOCKMAP`). Together with
    /// [`every_gore_prop_resolves`] this pins all sixteen blocking gore
    /// rows.
    #[test]
    fn every_impaled_and_meat_hook_prop_resolves() {
        let t = Tables::load().expect("tables load");

        // Impaled bodies: blocking, floor-standing (hangs = false).
        let impaled: &[(&str, u16, i32, i32)] = &[
            ("impaled_body", 25, 16, 16),
            ("twitching_impaled_body", 26, 16, 16),
        ];
        for (name, doomednum, radius, height) in impaled {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t.prop(name).unwrap_or_else(|| panic!("`{name}` prop dims"));
            assert_eq!(dims.radius, *radius, "`{name}` radius");
            assert_eq!(dims.height, *height, "`{name}` height");
            assert!(dims.blocks, "`{name}` blocks movement");
            assert!(!dims.hangs, "`{name}` stands on the floor");
        }
        assert_eq!(impaled.len(), 2, "every impaled body was checked");

        // Meat hooks, blocking half: hangs = true, P22-relevant.
        let hooks: &[(&str, u16, i32, i32)] = &[
            ("hanging_bloody_twitch", 49, 16, 68),
            ("hanging_meat2", 50, 16, 84),
            ("hanging_meat3", 51, 16, 84),
            ("hanging_meat4", 52, 16, 68),
            ("hanging_meat5", 53, 16, 52),
        ];
        for (name, doomednum, radius, height) in hooks {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            let dims = t.prop(name).unwrap_or_else(|| panic!("`{name}` prop dims"));
            assert_eq!(dims.radius, *radius, "`{name}` radius");
            assert_eq!(dims.height, *height, "`{name}` height");
            assert!(dims.blocks, "`{name}` blocks movement");
            assert!(dims.hangs, "`{name}` hangs from the ceiling");
        }
        assert_eq!(hooks.len(), 5, "every blocking meat hook was checked");

        // The walk-through twins and `brain_stem`: real rows, no dims.
        let no_props: &[(&str, u16)] = &[
            ("hanging_meat2_passable", 59),
            ("hanging_meat4_passable", 60),
            ("hanging_meat3_passable", 61),
            ("hanging_meat5_passable", 62),
            ("hanging_bloody_twitch_passable", 63),
            ("brain_stem", 81),
        ];
        for (name, doomednum) in no_props {
            assert_eq!(t.thing_id(name), Some(*doomednum), "`{name}` doomednum");
            assert!(
                t.prop(name).is_none(),
                "`{name}` lacks MF_SOLID and carries no [props.*] entry"
            );
        }
        assert_eq!(no_props.len(), 6, "every non-blocking row was checked");
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

    /// A door's optional trim alcove sectors need a texture role of their
    /// own, alongside `wall`/`floor`/`ceiling`/`door`/`door_track`/`switch`.
    #[test]
    fn switch_width_resolves_and_is_unknown_for_an_unknown_theme() {
        let t = Tables::load().expect("tables load");
        assert_eq!(
            t.switch_width("tech_base"),
            Some(128),
            "SW1STARG is 128 wide in DOOM.WAD, DOOM2.WAD and freedoom2.wad alike"
        );
        assert_eq!(t.switch_width("plaid_theme"), None);
    }

    #[test]
    fn key_trim_distinguishes_a_card_from_a_skull() {
        let t = Tables::load().expect("tables load");
        // The measured convention: plain name for a keycard, `2` variant for
        // a skull key. See vocabulary.toml's [key_trim] for the corpus tally.
        assert_eq!(t.key_trim("blue_card"), Some("DOORBLU"));
        assert_eq!(t.key_trim("blue_skull"), Some("DOORBLU2"));
        assert_eq!(t.key_trim("red_card"), Some("DOORRED"));
        assert_eq!(t.key_trim("red_skull"), Some("DOORRED2"));
        assert_eq!(t.key_trim("yellow_card"), Some("DOORYEL"));
        assert_eq!(t.key_trim("yellow_skull"), Some("DOORYEL2"));
        assert_eq!(
            t.key_trim("chartreuse_card"),
            None,
            "an unknown key has no trim"
        );
    }

    #[test]
    fn trim_texture_resolves() {
        let t = Tables::load().expect("tables load");
        assert_eq!(
            t.texture("trim", "tech_base"),
            Some("SUPPORT3"),
            "tech_base's trim is the flanking door trim, not another wall texture"
        );
        assert_eq!(
            t.texture("trim", "plaid_theme"),
            None,
            "an unknown theme resolves no texture at all"
        );
    }

    /// The curated (not sourced) door-texture catalog: `tech_base`'s own
    /// configured `door` texture (BIGDOOR2) must be recognized, a
    /// representative sample of the rest of the curated family must resolve
    /// too, and — the actual point of the check — a texture that is
    /// unambiguously NOT a door (a plain wall, a flat, an unrelated door
    /// *role* this project already treats distinctly, and a name that
    /// merely contains the substring "DOOR") must all be rejected.
    #[test]
    fn the_curated_door_texture_catalog_distinguishes_real_door_textures() {
        let t = Tables::load().expect("tables load");
        assert!(
            t.is_door_texture("BIGDOOR2"),
            "tech_base's own configured door texture must be recognized"
        );
        for name in ["BIGDOOR1", "DOOR1", "DOORBLU", "SPCDOOR1", "ZELDOOR"] {
            assert!(
                t.is_door_texture(name),
                "`{name}` is a curated door texture"
            );
        }
        assert!(
            !t.is_door_texture("STARTAN3"),
            "a plain wall texture is not a door texture"
        );
        assert!(
            !t.is_door_texture("FLOOR4_8"),
            "a flat is not a door texture"
        );
        assert!(
            !t.is_door_texture("DOORTRAK"),
            "the door's own TRACK texture is a distinct role, not the door panel itself"
        );
        assert!(
            !t.is_door_texture("DOORSTOP"),
            "a doorstop prop texture is not the door panel itself"
        );
        assert!(
            !t.is_door_texture("M_BDOOR"),
            "a menu HUD graphic that merely contains the substring DOOR is not a door texture"
        );
        assert!(
            !t.is_door_texture("plaid_door"),
            "an unknown name must fail loudly, not fall back to a fuzzy match"
        );
    }

    /// The `[random]` table's two means — the empirical distribution of the
    /// real `rndtable`'s values modulo 3 and modulo 8 — must resolve to the
    /// exact figures computed from the pinned source's 256-entry table, not
    /// an assumed-uniform 0..255 distribution (see `engine.toml`'s
    /// `[random]` derivation for the full histogram this checks against).
    #[test]
    fn random_distribution_resolves() {
        let t = Tables::load().expect("tables load");
        assert!(
            approx_eq(t.random_mod3_mean(), 1.070_312_5),
            "mod3_mean is the real rndtable's empirical mean, not the \
             uniform-assumption value of 255/256 ~ 0.996"
        );
        assert!(
            approx_eq(t.random_mod8_mean(), 3.476_562_5),
            "mod8_mean is the real rndtable's empirical mean, close to but \
             not exactly the uniform-assumption value of 3.5"
        );
    }

    /// Every weapon in `[weapons.damage.*]` — the compiler's first COMPUTED
    /// (not read) figures — must resolve its ammo type, ammo cost, and both
    /// expected-damage figures, checked individually against the exact
    /// arithmetic recorded in each entry's `derivation` field. The chainsaw
    /// and fist draw no ammo and must resolve nothing, and an unknown
    /// weapon name must fail loudly rather than silently fall back.
    #[test]
    fn every_weapon_damage_resolves() {
        let t = Tables::load().expect("tables load");
        // (name, ammo_type, ammo_per_shot, expected_damage_per_shot, expected_damage_per_ammo)
        let weapons: &[(&str, AmmoType, i32, f64, f64)] = &[
            ("pistol", AmmoType::Bullets, 1, 10.351_562_5, 10.351_562_5),
            ("shotgun", AmmoType::Shells, 1, 72.460_937_5, 72.460_937_5),
            (
                "super_shotgun",
                AmmoType::Shells,
                2,
                207.031_25,
                103.515_625,
            ),
            ("chaingun", AmmoType::Bullets, 1, 10.351_562_5, 10.351_562_5),
            (
                "rocket_launcher",
                AmmoType::Rockets,
                1,
                217.531_25,
                217.531_25,
            ),
            (
                "plasma_rifle",
                AmmoType::Cells,
                1,
                22.382_812_5,
                22.382_812_5,
            ),
            (
                "bfg9000",
                AmmoType::Cells,
                40,
                514.804_687_5,
                12.870_117_187_5,
            ),
        ];
        for (name, ammo_type, ammo_per_shot, dmg_per_shot, dmg_per_ammo) in weapons {
            let w = t
                .weapon_damage(name)
                .unwrap_or_else(|| panic!("`{name}` weapon damage"));
            assert_eq!(w.ammo_type, *ammo_type, "`{name}` ammo_type");
            assert_eq!(w.ammo_per_shot, *ammo_per_shot, "`{name}` ammo_per_shot");
            assert!(
                approx_eq(w.expected_damage_per_shot, *dmg_per_shot),
                "`{name}` expected_damage_per_shot: got {}, want {}",
                w.expected_damage_per_shot,
                dmg_per_shot
            );
            assert!(
                approx_eq(w.expected_damage_per_ammo, *dmg_per_ammo),
                "`{name}` expected_damage_per_ammo: got {}, want {}",
                w.expected_damage_per_ammo,
                dmg_per_ammo
            );
        }
        assert_eq!(weapons.len(), 7, "every listed weapon was checked");
        assert!(
            t.weapon_damage("chainsaw").is_none(),
            "the chainsaw draws no ammo and carries no [weapons.damage.*] entry"
        );
        assert!(
            t.weapon_damage("fist").is_none(),
            "the fist draws no ammo and carries no [weapons.damage.*] entry"
        );
        assert!(
            t.weapon_damage("plaid_gun").is_none(),
            "an unknown weapon name must fail loudly, not silently fall back"
        );
    }

    /// Every named ammo pickup in `[ammo.pickups.*]` must resolve its grant
    /// amount and ammo type, checked individually; the backpack's distinct
    /// all-four-types shape must resolve through
    /// [`Tables::ammo_backpack_grant`] and must NOT resolve through
    /// [`Tables::ammo_pickup`], which only handles the single-amount/
    /// single-type shape the other entries share.
    #[test]
    fn every_ammo_pickup_resolves() {
        let t = Tables::load().expect("tables load");
        // (name, amount, ammo_type)
        let pickups: &[(&str, i32, AmmoType)] = &[
            ("clip", 10, AmmoType::Bullets),
            ("box_of_bullets", 50, AmmoType::Bullets),
            ("shells", 4, AmmoType::Shells),
            ("box_of_shells", 20, AmmoType::Shells),
            ("rocket", 1, AmmoType::Rockets),
            ("box_of_rockets", 5, AmmoType::Rockets),
            ("cell_charge", 20, AmmoType::Cells),
            ("cell_pack", 100, AmmoType::Cells),
        ];
        for (name, amount, ammo_type) in pickups {
            let p = t
                .ammo_pickup(name)
                .unwrap_or_else(|| panic!("`{name}` ammo pickup"));
            assert_eq!(p.amount, *amount, "`{name}` amount");
            assert_eq!(p.ammo_type, *ammo_type, "`{name}` ammo_type");
        }
        assert_eq!(pickups.len(), 8, "every listed ammo pickup was checked");

        let backpack = t.ammo_backpack_grant();
        assert_eq!(backpack.bullets, 10, "backpack bullets");
        assert_eq!(backpack.shells, 4, "backpack shells");
        assert_eq!(backpack.cells, 20, "backpack cells");
        assert_eq!(backpack.rockets, 1, "backpack rockets");

        assert!(
            t.ammo_pickup("backpack").is_none(),
            "backpack grants all four ammo types and must be read through \
             ammo_backpack_grant, not ammo_pickup"
        );
        assert!(
            t.ammo_pickup("plaid_ammo").is_none(),
            "an unknown ammo pickup name must fail loudly, not silently fall back"
        );
    }

    /// Every named weapon in `[ammo.weapon_grant.*]` must resolve its
    /// pickup-grant amount and ammo type, checked individually and
    /// exhaustively over the six weapons the table lists — mirrors
    /// [`every_ammo_pickup_resolves`]'s own shape for `[ammo.pickups.*]`.
    /// The pistol (no placed pickup thing exists for it) and the chainsaw
    /// (`am_noammo`) are deliberately absent from the table and must not
    /// resolve here either.
    #[test]
    fn every_weapon_ammo_grant_resolves() {
        let t = Tables::load().expect("tables load");
        // (name, amount, ammo_type)
        let grants: &[(&str, i32, AmmoType)] = &[
            ("chaingun", 20, AmmoType::Bullets),
            ("shotgun", 8, AmmoType::Shells),
            ("super_shotgun", 8, AmmoType::Shells),
            ("rocket_launcher", 2, AmmoType::Rockets),
            ("plasma_rifle", 40, AmmoType::Cells),
            ("bfg9000", 40, AmmoType::Cells),
        ];
        for (name, amount, ammo_type) in grants {
            let g = t
                .weapon_ammo_grant(name)
                .unwrap_or_else(|| panic!("`{name}` weapon ammo grant"));
            assert_eq!(g.amount, *amount, "`{name}` amount");
            assert_eq!(g.ammo_type, *ammo_type, "`{name}` ammo_type");
        }
        assert_eq!(grants.len(), 6, "every listed weapon grant was checked");

        assert!(
            t.weapon_ammo_grant("pistol").is_none(),
            "the pistol is never a placed pickup thing and must not resolve"
        );
        assert!(
            t.weapon_ammo_grant("chainsaw").is_none(),
            "the chainsaw draws am_noammo and must not resolve"
        );
        assert!(
            t.weapon_ammo_grant("plaid_gun").is_none(),
            "an unknown weapon name must fail loudly, not silently fall back"
        );
    }

    /// End-to-end proof that the ammo-damage model is actually usable, not
    /// merely present: computes `arsenal.ammo.ratio` ("placed ammo damage /
    /// total baseline monster HP") for a small synthetic case — three
    /// placed clips (pistol ammo, unambiguous: the chaingun draws on the
    /// same pool at an identical per-bullet rate) and one placed box of
    /// rockets (rocket launcher ammo, the only weapon on that pool) against
    /// a room holding one zombieman and one imp — entirely by chaining the
    /// public `Tables` accessors, and asserts the exact resulting ratio.
    #[test]
    fn ammo_ratio_end_to_end_synthetic_case() {
        let t = Tables::load().expect("tables load");

        let clip = t.ammo_pickup("clip").expect("clip pickup");
        let pistol = t.weapon_damage("pistol").expect("pistol damage");
        assert_eq!(
            clip.ammo_type, pistol.ammo_type,
            "the placed clips must feed the weapon being costed"
        );
        let clip_count = 3;
        let clip_damage = f64::from(clip_count * clip.amount) * pistol.expected_damage_per_ammo;

        let box_of_rockets = t.ammo_pickup("box_of_rockets").expect("rocket box pickup");
        let rocket_launcher = t
            .weapon_damage("rocket_launcher")
            .expect("rocket launcher damage");
        assert_eq!(
            box_of_rockets.ammo_type, rocket_launcher.ammo_type,
            "the placed rocket box must feed the weapon being costed"
        );
        let rocket_box_count = 1;
        let rocket_damage = f64::from(rocket_box_count * box_of_rockets.amount)
            * rocket_launcher.expected_damage_per_ammo;

        let total_ammo_damage = clip_damage + rocket_damage;

        let zombieman_hp = t.spawnhealth("zombieman").expect("zombieman spawnhealth");
        let imp_hp = t.spawnhealth("imp").expect("imp spawnhealth");
        let total_monster_hp = f64::from(zombieman_hp + imp_hp);

        let ratio = total_ammo_damage / total_monster_hp;

        assert!(
            approx_eq(total_ammo_damage, 1_398.203_125),
            "total placed ammo damage: got {total_ammo_damage}"
        );
        assert!(
            approx_eq(total_monster_hp, 80.0),
            "total baseline monster HP: got {total_monster_hp}"
        );
        assert!(
            approx_eq(ratio, 17.477_539_062_5),
            "arsenal.ammo.ratio for this synthetic case: got {ratio}"
        );
    }

    #[test]
    fn start_maxima_and_map_slot_bound_are_exposed() {
        let t = Tables::load().unwrap();
        assert!(t.max_coop_starts() >= 1);
        assert!(t.max_dm_starts() >= t.max_coop_starts());
        assert!(t.commercial_map_slots() >= 1);
    }

    #[test]
    fn every_template_light_effect_name_resolves_to_a_sector_special() {
        let t = Tables::load().unwrap();
        for name in ["blink", "flicker", "glow", "strobe_slow"] {
            assert!(t.light_effect_special(name).is_some(), "unresolved: {name}");
        }
        assert!(t.light_effect_special("disco").is_none());
    }

    #[test]
    fn the_four_teleport_specials_are_distinct_and_selected_by_both_flags() {
        let t = Tables::load().expect("tables load");
        let all = t.teleport_specials();
        assert_eq!(
            all,
            [39, 97, 125, 126],
            "ascending, the four vanilla teleport lines"
        );
        assert_eq!(
            t.teleport_special(false, true),
            97,
            "player, repeatable (WR)"
        );
        assert_eq!(
            t.teleport_special(false, false),
            39,
            "player, one-shot (W1)"
        );
        assert_eq!(
            t.teleport_special(true, true),
            126,
            "monsters only, repeatable"
        );
        assert_eq!(
            t.teleport_special(true, false),
            125,
            "monsters only, one-shot"
        );
        assert_eq!(t.player_teleport_specials(), [97, 39]);
        assert_eq!(t.monster_teleport_specials(), [126, 125]);
        for s in all {
            assert!(t.vanilla_line_specials().contains(&s), "{s} is vanilla");
        }
    }

    #[test]
    fn the_pad_flat_resolves_for_every_theme() {
        let t = Tables::load().expect("tables load");
        assert_eq!(t.texture("pad", "tech_base"), Some("GATE3"));
        assert_eq!(t.texture("pad", "no_such_theme"), None);
    }

    #[test]
    fn the_ambush_thing_flag_is_bit_three() {
        let t = Tables::load().expect("tables load");
        assert_eq!(t.thing_flag("ambush"), Some(8));
        assert_eq!(t.thing_flag("deaf"), None, "only the sourced name resolves");
    }
}
