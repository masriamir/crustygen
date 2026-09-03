//! Spec conformance rows: the fixed catalog of frontmatter parameters this
//! checker can measure — plus one row per spec monster species, one per a
//! placed species the spec never names, and one per `sustain.powerups[]`
//! entry — each judged against what the parsed [`Scene`] and [`MapStats`]
//! actually show (`docs/design.md` §8.1's conformance report). **This is not
//! every frontmatter parameter [`crate::spec::Spec`] declares.** A parameter
//! with no sourced geometric definition, or nothing emitted to measure it
//! against, gets an explicit [`Verdict::NotDerivable`] row instead of being
//! silently absent — six rows are always `NotDerivable` (`identity.grid`,
//! `scale.rooms`, `scale.play_time_minutes`, `combat.encounter_style`,
//! `combat.sound.propagation`, `combat.max_simultaneous`), and several more
//! become `NotDerivable` only when the map itself gives [`rows`] nothing to
//! measure; `docs/check.md`'s "Conformance" section has the full list. Unlike
//! `scene.rs`/`invariants.rs`/`flood.rs`, all but six rows here are plain
//! target-vs-actual comparisons that re-derive no playability rule from the
//! pinned engine, so the only sourcing burden is the ammo ratio's
//! damage-per-ammo figures ([`crate::tables::Tables::weapon_damage`],
//! [`crate::tables::Tables::weapon_ammo_grant`]), the `MTF_AMBUSH` bit
//! ([`crate::tables::Tables::thing_flag`], sourced in `engine.toml`'s
//! `[thing.flags]`), the teleport specials the two pad counts read
//! ([`crate::tables::Tables::player_teleport_specials`],
//! [`crate::tables::Tables::monster_teleport_specials`]), the four floor
//! specials the two trigger counts read
//! ([`crate::tables::Tables::floor_special`]), and the engine fact
//! cited on the `MULTIPLAYER_ONLY_BIT` thing-flag constant below. The six
//! exceptions are `progression.exit.trigger` — a teleport exit emits the
//! same specials as a plain walkover one, so the row borrows
//! [`crate::check::flood::teleport_only_sectors`]'s reachability predicate
//! rather than reading the line — the three `progression.lifts.*` rows,
//! which read [`crate::check::plats::resolve_plats`]'s engine-style
//! resolution of what each platform is and who can call it, rather than
//! counting lift lines — and `progression.floors` with
//! `combat.monster_closets`, which read
//! [`crate::lift::floor::recognize`]'s engine-style resolution of what each
//! floor action *does* (which is not a thing a line's special says) on top
//! of [`crate::check::floors`]'s.
//!
//! [`rows`] implements the row catalog in the Task 10 brief, in the brief's
//! own order, plus the two rows the floor construct added after it
//! (`progression.floors` closing the `progression` block and
//! `combat.monster_closets` opening the `combat` one), and follows the
//! brief's verdict rules: a `MinMax` or exact-count
//! target is [`Verdict::Pass`]/[`Verdict::Fail`]; a scalar continuous target
//! (`hitscanner_ratio`, `deaf_ratio`, `ammo.ratio`) is always
//! [`Verdict::Info`], its `actual` formatted `"<value> (target <t>, delta
//! <d>)"` rather than judged against an invented tolerance; a parameter this
//! checker cannot derive from emitted geometry at all is
//! [`Verdict::NotDerivable`], its `actual` carrying the reason.

use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};

use crate::check::floors::local_adjacency;
use crate::check::plats::{Activator, Rest, ScenePlat, resolve_plats};
use crate::check::scene::{Scene, SceneThing};
use crate::check::{ConformanceRow, MapStats, Verdict};
use crate::lift::floor::{Shape, recognize};
use crate::spec::Spec;
use crate::spec::frontmatter::{
    EncounterStyle, ExitKind, ExitTrigger, Facing, Frontmatter, LiftTrigger, MinMax, Propagation,
};
use crate::tables::{AmmoType, FloorFamily, Tables};

// ---------------------------------------------------------------------
// Generic verdict constructors, shared by every row below.
// ---------------------------------------------------------------------

/// A `MinMax<T>` target versus a measured `actual`: [`Verdict::Pass`] iff
/// `actual` falls within `[mm.min, mm.max]` inclusive, else
/// [`Verdict::Fail`].
fn range_row<T>(parameter: String, mm: &MinMax<T>, actual: T) -> ConformanceRow
where
    T: PartialOrd + std::fmt::Display + Copy,
{
    let pass = actual >= mm.min && actual <= mm.max;
    ConformanceRow {
        parameter,
        target: format!("{}..={}", mm.min, mm.max),
        actual: actual.to_string(),
        verdict: if pass { Verdict::Pass } else { Verdict::Fail },
    }
}

/// An exact-count `u32` target versus a measured `actual`: [`Verdict::Pass`]
/// iff equal.
fn exact_row(parameter: String, target: u32, actual: u32) -> ConformanceRow {
    ConformanceRow {
        parameter,
        target: target.to_string(),
        actual: actual.to_string(),
        verdict: if target == actual {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// A `bool` target versus a measured `actual`: [`Verdict::Pass`] iff equal.
fn bool_row(parameter: String, target: bool, actual: bool) -> ConformanceRow {
    ConformanceRow {
        parameter,
        target: target.to_string(),
        actual: actual.to_string(),
        verdict: if target == actual {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// A parameter this checker cannot derive from emitted geometry:
/// [`Verdict::NotDerivable`], `reason` carried verbatim in `actual` per the
/// brief.
fn not_derivable(parameter: String, target: String, reason: &str) -> ConformanceRow {
    ConformanceRow {
        parameter,
        target,
        actual: reason.to_owned(),
        verdict: Verdict::NotDerivable,
    }
}

/// A scalar continuous target (a fraction or ratio): always
/// [`Verdict::Info`] — reported with its delta, per the brief, "judged by no
/// invented tolerance" ([`Verdict::Info`]'s own doc comment in `check/mod.rs`).
fn info_row(parameter: String, target: f64, actual: f64) -> ConformanceRow {
    let delta = actual - target;
    ConformanceRow {
        parameter,
        target: format!("{target:.3}"),
        actual: format!("{actual:.3} (target {target:.3}, delta {delta:.3})"),
        verdict: Verdict::Info,
    }
}

/// `numerator / denominator` as a fraction. Both counts are always far under
/// `f64`'s 52-bit mantissa (a real map's monster count is in the hundreds at
/// most), so this is a controlled precision loss, not a silent one.
#[expect(
    clippy::cast_precision_loss,
    reason = "monster counts are always far under f64's 52-bit mantissa"
)]
fn ratio_of(numerator: usize, denominator: usize) -> f64 {
    numerator as f64 / denominator as f64
}

/// Every thing in `scene` whose name is in `names`, as a `u32` count.
fn count_names(scene: &Scene, names: &[&str]) -> u32 {
    let n = scene
        .things
        .iter()
        .filter(|t| t.name.as_deref().is_some_and(|name| names.contains(&name)))
        .count();
    u32::try_from(n).expect("thing counts fit u32")
}

/// Every `fronts_this` boundary in `scene` carrying one of `specials`, as a
/// `u32` count — one linedef, one count, not one per mirror of a two-sided
/// line.
fn count_specials(scene: &Scene, specials: &[i32]) -> u32 {
    let n = scene
        .sectors
        .iter()
        .flat_map(|s| &s.boundary)
        .filter(|b| b.fronts_this && specials.contains(&b.special))
        .count();
    u32::try_from(n).expect("linedef counts fit u32")
}

/// Whether any `fronts_this` boundary in `scene` carries one of `specials`.
fn any_special_present(scene: &Scene, specials: &[i32]) -> bool {
    count_specials(scene, specials) > 0
}

/// Distinct pads any thing may cross: the back sectors of `fronts_this`
/// boundaries carrying a player teleport special. A pad is a square, so its
/// four edges share one back sector — this counts pads, not edges, which is
/// what `progression.teleports.count` means by "teleports".
fn count_player_pads(scene: &Scene, tables: &Tables) -> u32 {
    let specials: Vec<i32> = tables
        .player_teleport_specials()
        .into_iter()
        .map(i32::from)
        .collect();
    let pads: BTreeSet<usize> = scene
        .sectors
        .iter()
        .flat_map(|s| &s.boundary)
        .filter(|b| b.fronts_this && specials.contains(&b.special))
        .filter_map(|b| b.neighbor)
        .collect();
    u32::try_from(pads.len()).expect("pad counts fit u32")
}

/// Distinct monsters-only pads whose host room holds at least one monster —
/// a teleport ambush.
///
/// The host is the pad edge's *front* sector, the one whose boundary list
/// the `fronts_this` mirror sits in: the pad is always the trigger line's
/// back sector ([`crate::ir::Teleport`]), so the front side is the room the
/// monsters walk in from. A monsters-only pad in a room with no monster is
/// no ambush, and one room's monsters can stage several.
fn count_teleport_ambushes(scene: &Scene, tables: &Tables) -> u32 {
    let specials: Vec<i32> = tables
        .monster_teleport_specials()
        .into_iter()
        .map(i32::from)
        .collect();
    let monster_sectors = monster_sectors(scene, tables);
    let pads: BTreeSet<usize> = scene
        .sectors
        .iter()
        .enumerate()
        .filter(|(i, _)| monster_sectors.contains(i))
        .flat_map(|(_, s)| &s.boundary)
        .filter(|b| b.fronts_this && specials.contains(&b.special))
        .filter_map(|b| b.neighbor)
        .collect();
    u32::try_from(pads.len()).expect("pad counts fit u32")
}

/// The sectors of `scene` holding at least one monster
/// ([`monsters`], so a thing whose name resolves a `spawnhealth`).
fn monster_sectors(scene: &Scene, tables: &Tables) -> BTreeSet<usize> {
    monsters(scene, tables)
        .iter()
        .filter_map(|t| t.sector)
        .collect()
}

/// Every thing in `scene` classified as a monster: `tables.spawnhealth(name)`
/// resolves, per the brief's "Monster classification" note.
fn monsters<'a>(scene: &'a Scene, tables: &Tables) -> Vec<&'a SceneThing> {
    scene
        .things
        .iter()
        .filter(|t| {
            t.name
                .as_deref()
                .is_some_and(|name| tables.spawnhealth(name).is_some())
        })
        .collect()
}

// ---------------------------------------------------------------------
// identity
// ---------------------------------------------------------------------

/// `identity.slot` versus `map_name`: [`Verdict::Pass`] iff equal,
/// case-insensitive (a slot is conventionally upper-cased, but a WAD's own
/// map lump name casing is not this checker's business to police).
fn identity_row(fm: &Frontmatter, map_name: &str) -> ConformanceRow {
    ConformanceRow {
        parameter: "identity.slot".to_owned(),
        target: fm.identity.slot.clone(),
        actual: map_name.to_owned(),
        verdict: if fm.identity.slot.eq_ignore_ascii_case(map_name) {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// `identity.grid`: always [`Verdict::NotDerivable`]. `KNOWN-GAPS.md`:
/// "Portal `width` and `at` are exempt from the grid rule" that binds room
/// footprints — real doorways are routinely finer than the grid their rooms
/// sit on — so a vertex-grid check would false-positive on every emitted
/// opening rather than actually re-deriving the rule the spec states.
fn identity_grid_row(fm: &Frontmatter) -> ConformanceRow {
    not_derivable(
        "identity.grid".to_owned(),
        fm.identity.grid.to_string(),
        "portal width/at are exempt from the grid rule, so a vertex-grid check false-positives \
         on every opening",
    )
}

// ---------------------------------------------------------------------
// scale
// ---------------------------------------------------------------------

/// The axis-aligned bounding box of every boundary endpoint in `scene`
/// (`(min_x, max_x, min_y, max_y)`), or `None` if `scene` has no boundary
/// geometry at all (no sectors, or every sector empty).
fn bounding_box(scene: &Scene) -> Option<(f64, f64, f64, f64)> {
    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for sector in &scene.sectors {
        for b in &sector.boundary {
            for (x, y) in [b.a, b.b] {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }
    if min_x.is_finite() {
        Some((min_x, max_x, min_y, max_y))
    } else {
        None
    }
}

/// `scale.size`: the emitted bounding box's width and height, each
/// [`Verdict::Pass`] iff no larger than the spec's own budget (the brief:
/// "Pass iff both ≤ target").
fn scale_size_row(fm: &Frontmatter, scene: &Scene) -> ConformanceRow {
    let target = format!("{}x{}", fm.scale.size.width, fm.scale.size.height);
    let Some((min_x, max_x, min_y, max_y)) = bounding_box(scene) else {
        return not_derivable(
            "scale.size".to_owned(),
            target,
            "no boundary geometry to measure",
        );
    };
    let width = max_x - min_x;
    let height = max_y - min_y;
    let pass = width <= f64::from(fm.scale.size.width) && height <= f64::from(fm.scale.size.height);
    ConformanceRow {
        parameter: "scale.size".to_owned(),
        target,
        actual: format!("{width}x{height}"),
        verdict: if pass { Verdict::Pass } else { Verdict::Fail },
    }
}

/// `scale.vertical_range`: every sector's floor height must lie within
/// `[min, max]`.
///
/// The authoritative definition is the field's own doc comment
/// (`Scale::vertical_range` in `frontmatter.rs`): "The allowed floor height
/// range" — an inclusive bound on every individual floor, not a target for
/// the map's *computed* vertical span. `map-spec.template.md`'s inline
/// comment (`# allowed floor heights; the map's span is max - min`, which
/// `docs/design.md` §5 repeats verbatim) reads as ambiguous in isolation,
/// but its own leading clause agrees with `frontmatter.rs`: "allowed floor
/// heights" names a per-floor bound, and "the map's span is max - min" is
/// describing what the *allowed band itself* spans (`max - min` map units
/// wide), not instructing this check to compare the map's actual floor
/// span against it — those are different claims. A map whose floors sit at
/// -16 and 128 fails `vertical_range: { min: 0, max: 256 }` under the
/// per-floor reading (the -16 floor is below `min`), even though its
/// *span* (128 - (-16) = 144) would pass under the span reading — the
/// per-floor reading is what this row implements. `actual` reports the
/// observed floor extremes rather than their difference, so a failure
/// names which one is out of band.
fn vertical_range_row(fm: &Frontmatter, scene: &Scene) -> ConformanceRow {
    let mm = &fm.scale.vertical_range;
    let target = format!("{}..={}", mm.min, mm.max);
    let floors: Vec<i32> = scene.sectors.iter().map(|s| s.floor).collect();
    let (Some(&lo), Some(&hi)) = (floors.iter().min(), floors.iter().max()) else {
        return not_derivable(
            "scale.vertical_range".to_owned(),
            target,
            "no sectors present",
        );
    };
    let pass = floors.iter().all(|&f| f >= mm.min && f <= mm.max);
    ConformanceRow {
        parameter: "scale.vertical_range".to_owned(),
        target,
        actual: format!("floors {lo}..{hi}"),
        verdict: if pass { Verdict::Pass } else { Verdict::Fail },
    }
}

// ---------------------------------------------------------------------
// players
// ---------------------------------------------------------------------

/// The five thing kinds the engine reads as a player spawn point, shared
/// with `invariants.rs`'s own `START_KINDS` (re-declared here rather than
/// imported since that constant is private to that module — this module's
/// domain of "which thing names are starts" is small enough to duplicate
/// rather than plumb a new `pub(crate)` export for).
const START_KINDS: [&str; 5] = [
    "player1_start",
    "player2_start",
    "player3_start",
    "player4_start",
    "deathmatch_start",
];

/// A [`Facing`] as its `player1_start.angle` degree value.
///
/// Sourced, not guessed: `p_mobj.c`'s `P_SpawnMissile` (pinned commit
/// `a77dfb96cb91780ca334d0d4cfd86957558007e0`) reads `momx =
/// FixedMul(speed, finecosine[an]); momy = FixedMul(speed, finesine[an]);`
/// — at `an` = 0 (east), `finecosine[0]` is maximal and `finesine[0]` is
/// zero, so angle 0 moves along `+x`; at `an` = 90 degrees (north),
/// `finesine` is maximal and `finecosine` is zero, so angle 90 moves along
/// `+y`. `docs/geometry.md` already calls the higher-`x` wall of a room
/// "east" (`span.near`/`span.far`'s table), i.e. `+x` = east in this
/// project's own convention, confirming (not contradicting) the standard
/// Doom angle system this maps onto: 0 = east, 90 = north, 180 = west, 270 =
/// south, increasing counterclockwise.
fn facing_degrees(facing: &Facing) -> i32 {
    match *facing {
        Facing::East => 0,
        Facing::North => 90,
        Facing::West => 180,
        Facing::South => 270,
        Facing::Degrees(d) => i32::from(d),
    }
}

/// The document-vocabulary name of a [`Facing`] variant (`docs/map-spec.md`'s
/// `north | south | east | west` spelling), or the bare degree number for
/// [`Facing::Degrees`].
fn facing_name(facing: &Facing) -> String {
    match *facing {
        Facing::North => "north".to_owned(),
        Facing::South => "south".to_owned(),
        Facing::East => "east".to_owned(),
        Facing::West => "west".to_owned(),
        Facing::Degrees(d) => d.to_string(),
    }
}

/// `players.start_facing`: the first `player1_start` thing's `angle`
/// (declaration order, matching `flood.rs`'s `resolve_start` convention)
/// versus the spec's `Facing` mapped to degrees ([`facing_degrees`]).
/// [`Verdict::NotDerivable`] if no `player1_start` is placed at all.
fn start_facing_row(fm: &Frontmatter, scene: &Scene) -> ConformanceRow {
    let target_deg = facing_degrees(&fm.players.start_facing);
    let target = format!(
        "{} ({target_deg} degrees)",
        facing_name(&fm.players.start_facing)
    );
    let Some(start) = scene
        .things
        .iter()
        .find(|t| t.name.as_deref() == Some("player1_start"))
    else {
        return not_derivable(
            "players.start_facing".to_owned(),
            target,
            "no player1_start placed",
        );
    };
    ConformanceRow {
        parameter: "players.start_facing".to_owned(),
        target,
        actual: format!("{} degrees", start.angle),
        verdict: if start.angle == target_deg {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// The `!single` multiplayer-only thing-flag bit — bit 4, value 16.
///
/// Sourced from `p_mobj.c`'s `P_SpawnMapThing` (pinned commit
/// `a77dfb96cb91780ca334d0d4cfd86957558007e0`): `if (!netgame &&
/// (mthing->options & 16)) return;` — a thing carrying this bit is skipped
/// entirely in single-player. Unlike `MTF_AMBUSH` (sourced in `engine.toml`'s
/// `[thing.flags]`), this bit has no named `MTF_*` constant in the pinned
/// `doomdef.h`; the source above uses the raw literal `16` itself.
const MULTIPLAYER_ONLY_BIT: u32 = 16;

/// `players.coop_only_items`: whether any non-start thing in `scene` carries
/// [`MULTIPLAYER_ONLY_BIT`].
fn coop_only_items_row(fm: &Frontmatter, scene: &Scene) -> ConformanceRow {
    let actual = scene.things.iter().any(|t| {
        !t.name
            .as_deref()
            .is_some_and(|name| START_KINDS.contains(&name))
            && t.flags & MULTIPLAYER_ONLY_BIT != 0
    });
    bool_row(
        "players.coop_only_items".to_owned(),
        fm.players.coop_only_items,
        actual,
    )
}

// ---------------------------------------------------------------------
// progression
// ---------------------------------------------------------------------

/// `progression.keys`: the spec's key list versus the *set* of key thing
/// names placed on the map. [`Verdict::Pass`] iff the sets are equal — order
/// and duplicates in the spec's `Vec<String>` do not matter.
fn keys_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let key_names: HashSet<String> = tables
        .locked_door_kinds()
        .into_iter()
        .map(|(k, _)| k)
        .collect();
    let spec_keys: BTreeSet<String> = fm.progression.keys.iter().cloned().collect();
    let placed_keys: BTreeSet<String> = scene
        .things
        .iter()
        .filter_map(|t| t.name.clone())
        .filter(|name| key_names.contains(name))
        .collect();
    let render = |keys: &BTreeSet<String>| {
        if keys.is_empty() {
            "none".to_owned()
        } else {
            keys.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    };
    ConformanceRow {
        parameter: "progression.keys".to_owned(),
        target: render(&spec_keys),
        actual: render(&placed_keys),
        verdict: if spec_keys == placed_keys {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// `progression.locked_doors`: the count of distinct door **sectors** behind
/// a locked special, deduped by back sector the same way `flood.rs`'s
/// `check_key_lock_coherence` dedups a two-faced door's keyless-lock finding
/// (one physical door, two faces on two linedefs, must count once).
fn locked_doors_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let locked_specials: Vec<i32> = tables
        .locked_door_kinds()
        .into_iter()
        .map(|(_, s)| i32::from(s))
        .collect();
    let mut door_sectors: HashSet<usize> = HashSet::new();
    for sector in &scene.sectors {
        for b in &sector.boundary {
            if b.fronts_this
                && locked_specials.contains(&b.special)
                && let Some(neighbor) = b.neighbor
            {
                door_sectors.insert(neighbor);
            }
        }
    }
    let actual = u32::try_from(door_sectors.len()).expect("door counts fit u32");
    exact_row(
        "progression.locked_doors".to_owned(),
        fm.progression.locked_doors,
        actual,
    )
}

/// The document-vocabulary name of an [`ExitKind`] variant.
fn exit_kind_name(kind: ExitKind) -> &'static str {
    match kind {
        ExitKind::Normal => "normal",
        ExitKind::Secret => "secret",
        ExitKind::Both => "both",
    }
}

/// The document-vocabulary name of an [`ExitTrigger`] variant.
fn exit_trigger_name(trigger: ExitTrigger) -> &'static str {
    match trigger {
        ExitTrigger::Switch => "switch",
        ExitTrigger::Teleport => "teleport",
        ExitTrigger::Walkover => "walkover",
    }
}

/// `progression.exit.kind`: which of the four exit specials (11/52 normal,
/// 51/124 secret) are present on any `fronts_this` boundary.
fn exit_kind_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let normal = [
        i32::from(tables.exit_switch_special()),
        i32::from(tables.exit_walkover_special()),
    ];
    let secret = [
        i32::from(tables.secret_exit_switch_special()),
        i32::from(tables.secret_exit_walkover_special()),
    ];
    let has_normal = any_special_present(scene, &normal);
    let has_secret = any_special_present(scene, &secret);
    let actual_kind = match (has_normal, has_secret) {
        (true, true) => Some(ExitKind::Both),
        (true, false) => Some(ExitKind::Normal),
        (false, true) => Some(ExitKind::Secret),
        (false, false) => None,
    };
    ConformanceRow {
        parameter: "progression.exit.kind".to_owned(),
        target: exit_kind_name(fm.progression.exit.kind).to_owned(),
        actual: actual_kind.map_or_else(
            || "none present".to_owned(),
            |k| exit_kind_name(k).to_owned(),
        ),
        verdict: if actual_kind == Some(fm.progression.exit.kind) {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// `progression.exit.trigger`: switch (11/51) versus walkover (52/124)
/// specials found — and, among the walkover exits, the
/// [`ExitTrigger::Teleport`] ones.
///
/// A teleport exit emits the same specials as a plain walkover exit
/// ([`crate::compile::exits`]), so nothing on the line itself tells the two
/// apart. What does is where the line sits: rule P26's teleport exit is a
/// walkover line in a room the player can only arrive in by teleport, which
/// is exactly [`crate::check::flood::teleport_only_sectors`]'s predicate.
/// Every sector carrying a crossable walkover exit line must be
/// teleport-only for the map to read `teleport`; one that can also be walked
/// to reads `walkover`.
///
/// The set is required to be non-empty rather than left to `all`, which
/// answers `true` over nothing. That emptiness clause is belt and braces
/// today — when no walkover line is crossable, `resolve_goals` finds no goal
/// (it applies the identical
/// [`Boundary::passable`](crate::check::scene::Boundary::passable) test) and
/// [`crate::check::flood::teleport_only_sectors`] returns `None`, which
/// already sinks the row to `walkover`. Stating it here keeps the row's own
/// meaning — *every crossable walkover goal sector is teleport-only, and
/// there is at least one* — from resting on that coupling in another module.
/// A map whose only walkover exit is one-sided or blocking has an exit
/// `P_CrossSpecialLine` would never fire, and grades `walkover`, which fails
/// against a `teleport` target instead of passing by accident.
///
/// [`crate::check::flood::teleport_only_sectors`] runs two full reachability
/// floods (with and without teleport edges), so it is only ever invoked
/// behind `has_walkover && !has_switch` — a switch exit, or a map with no
/// walkover exit at all, settles the trigger without paying for either
/// flood.
fn exit_trigger_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let target = exit_trigger_name(fm.progression.exit.trigger).to_owned();
    let switches = [
        i32::from(tables.exit_switch_special()),
        i32::from(tables.secret_exit_switch_special()),
    ];
    let walkovers = [
        i32::from(tables.exit_walkover_special()),
        i32::from(tables.secret_exit_walkover_special()),
    ];
    let has_switch = any_special_present(scene, &switches);
    let has_walkover = any_special_present(scene, &walkovers);
    let walkover_goal_sectors: Vec<usize> = scene
        .sectors
        .iter()
        .enumerate()
        .filter(|(_, s)| {
            s.boundary
                .iter()
                .any(|b| walkovers.contains(&b.special) && b.passable())
        })
        .map(|(i, _)| i)
        .collect();
    // `&&` short-circuits: the flood call below only runs once `has_walkover
    // && !has_switch` is known true (see the function doc for why that
    // matters).
    let is_teleport_exit = has_walkover
        && !has_switch
        && crate::check::flood::teleport_only_sectors(scene, tables).is_some_and(|only| {
            !walkover_goal_sectors.is_empty() && walkover_goal_sectors.iter().all(|&s| only[s])
        });
    let actual_trigger = match (has_switch, has_walkover, is_teleport_exit) {
        (true, false, _) => Some(ExitTrigger::Switch),
        (false, true, true) => Some(ExitTrigger::Teleport),
        (false, true, false) => Some(ExitTrigger::Walkover),
        _ => None,
    };
    let actual = actual_trigger.map_or_else(
        || {
            if has_switch && has_walkover {
                "both switch and walkover present".to_owned()
            } else {
                "none present".to_owned()
            }
        },
        |t| exit_trigger_name(t).to_owned(),
    );
    ConformanceRow {
        parameter: "progression.exit.trigger".to_owned(),
        target,
        actual,
        verdict: if actual_trigger == Some(fm.progression.exit.trigger) {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
    }
}

/// `progression.lifts.count`: distinct tag-resolved plat sectors
/// ([`resolve_plats`]) — platforms, not lift lines. A pedestal's four
/// use-line faces, or a barrier's two, drive one platform each; a lift line
/// whose tag names no sector drives none at all (that is V-P13/V-P14's
/// finding, not a platform to count).
fn count_plats(scene: &Scene, tables: &Tables) -> u32 {
    u32::try_from(resolve_plats(scene, tables).len()).expect("plat counts fit u32")
}

/// The form a [`Rest::Top`] plat's triggers take, in the template's own
/// `walkover | switch | both_ends` vocabulary — or `"top_only"` for a
/// platform no trigger calls from below, which is a form no template word
/// names (see [`lifts_trigger_row`]).
fn lift_form(p: &ScenePlat) -> &'static str {
    let low_use = p
        .triggers
        .iter()
        .any(|t| t.use_line && t.activators.iter().any(|&(_, a)| a == Activator::Low));
    let low_walk = p
        .triggers
        .iter()
        .any(|t| !t.use_line && t.activators.iter().any(|&(_, a)| a == Activator::Low));
    let top = p.callable_top();
    match (low_use || low_walk, top) {
        (true, true) => "both_ends",
        (true, false) if low_use => "switch",
        (true, false) => "walkover",
        (false, _) => "top_only",
    }
}

/// `progression.lifts.trigger`: every [`Rest::Top`] platform's [`lift_form`]
/// against the spec's one word. [`Verdict::Pass`] iff every one of them
/// matches; `actual` tallies the forms found (`"switch ×2, both_ends ×1"`).
///
/// Only platforms that rest at the top are judged. A barrier or a pedestal
/// rests above every neighbor ([`Rest::AboveAll`]) and is not a lift the
/// player rides up — the spec's `trigger` word is about lifts, so grading a
/// pedestal's four switch faces against `walkover` would fail a map that is
/// exactly what its spec asked for.
///
/// A `"top_only"` platform — one at rest on top with no trigger reachable
/// from its low floor — equals no template word, so it fails this row
/// whichever word the spec chose. That is deliberate: the same platform is
/// already V-P5's "callable only from above" warning, and a row that could
/// pass it would contradict the finding.
///
/// With no `Top` platform at all the row is a vacuous [`Verdict::Pass`],
/// `actual` reading `"no lifts"` — a map with no lift conforms to any
/// `trigger` word, the way a map with no monster gets `"no monsters"` rather
/// than a graded ratio.
fn lifts_trigger_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let target = match fm.progression.lifts.trigger {
        LiftTrigger::Switch => "switch",
        LiftTrigger::Walkover => "walkover",
        LiftTrigger::BothEnds => "both_ends",
    };
    let tops: Vec<ScenePlat> = resolve_plats(scene, tables)
        .into_iter()
        .filter(|p| p.rest == Rest::Top)
        .collect();
    if tops.is_empty() {
        return ConformanceRow {
            parameter: "progression.lifts.trigger".to_owned(),
            target: target.to_owned(),
            actual: "no lifts".to_owned(),
            verdict: Verdict::Pass,
        };
    }
    let count = |form: &str| tops.iter().filter(|p| lift_form(p) == form).count();
    let actual = format!(
        "switch ×{}, walkover ×{}, both_ends ×{}",
        count("switch"),
        count("walkover"),
        count("both_ends")
    );
    let pass = tops.iter().all(|p| lift_form(p) == target);
    ConformanceRow {
        parameter: "progression.lifts.trigger".to_owned(),
        target: target.to_owned(),
        actual,
        verdict: if pass { Verdict::Pass } else { Verdict::Fail },
    }
}

/// `progression.lifts.max_travel`: the largest [`ScenePlat::travel`] on the
/// map against the spec's ceiling, [`Verdict::Pass`] iff no platform travels
/// further. Every platform counts here, not only the [`Rest::Top`] ones
/// [`lifts_trigger_row`] judges — the parameter bounds how far a floor may
/// move, which a barrier or a pedestal does just as much as a lift.
///
/// With no platform at all, `actual` reads `"no lifts"` and the row passes
/// vacuously.
fn lifts_max_travel_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let max = resolve_plats(scene, tables).iter().map(|p| p.travel).max();
    let target = fm.progression.lifts.max_travel;
    ConformanceRow {
        parameter: "progression.lifts.max_travel".to_owned(),
        target: target.to_string(),
        actual: max.map_or_else(|| "no lifts".to_owned(), |m| m.to_string()),
        verdict: match max {
            None => Verdict::Pass,
            Some(m) if m <= target => Verdict::Pass,
            Some(_) => Verdict::Fail,
        },
    }
}

/// Every `progression.*` row: keys, locked doors, exit kind/trigger, the
/// four switch/walkover/lift/teleport `MinMax` counts, and the lift trigger
/// and travel rows. Extracted out of [`rows`] itself (alongside
/// [`monster_rows`]/[`sustain_rows`]/[`lighting_rows`]) purely to keep that
/// function under clippy's line-count lint — the row order and content are
/// unchanged either way.
fn progression_rows(
    fm: &Frontmatter,
    scene: &Scene,
    tables: &Tables,
    rows: &mut Vec<ConformanceRow>,
) {
    rows.push(keys_row(fm, scene, tables));
    rows.push(locked_doors_row(fm, scene, tables));
    rows.push(exit_kind_row(fm, scene, tables));
    rows.push(exit_trigger_row(fm, scene, tables));
    // A lift's use-line face is a switch the player walks up to and presses,
    // so it counts here as well as toward `progression.lifts.count` below.
    // The two rows count different things — switch *lines* against
    // *platforms* — so one pedestal contributes four to this row and one to
    // that one, with no double-count between them.
    let mut switch_specials = vec![
        i32::from(tables.exit_switch_special()),
        i32::from(tables.secret_exit_switch_special()),
    ];
    switch_specials.extend(tables.lift_use_specials().into_iter().map(i32::from));
    // A floor action's use line is pressed exactly as a lift's is, so it
    // counts here too. One such line drives every target sharing its tag,
    // and this row counts switches rather than what they drive, so counting
    // lines is right: a switch lowering a four-sector wall is one switch.
    switch_specials.extend(floor_specials(tables, true));
    rows.push(range_row(
        "progression.switches.count".to_owned(),
        &fm.progression.switches.count,
        count_specials(scene, &switch_specials),
    ));
    let mut walkover_specials = vec![
        i32::from(tables.exit_walkover_special()),
        i32::from(tables.secret_exit_walkover_special()),
    ];
    // The floor walkovers, by the same reasoning. (A lift walkover is still
    // absent from this row — that asymmetry is issue #53's, not this row's.)
    walkover_specials.extend(floor_specials(tables, false));
    rows.push(range_row(
        "progression.walkover_triggers.count".to_owned(),
        &fm.progression.walkover_triggers.count,
        count_specials(scene, &walkover_specials),
    ));
    rows.push(range_row(
        "progression.lifts.count".to_owned(),
        &fm.progression.lifts.count,
        count_plats(scene, tables),
    ));
    rows.push(lifts_trigger_row(fm, scene, tables));
    rows.push(lifts_max_travel_row(fm, scene, tables));
    rows.push(range_row(
        "progression.teleports.count".to_owned(),
        &fm.progression.teleports.count,
        count_player_pads(scene, tables),
    ));
    rows.push(floors_row(scene, tables));
}

/// The two emitted floor specials of one trigger form, as `i32`s: the use
/// lines with `use_line`, the walkovers without
/// ([`Tables::floor_special`]).
fn floor_specials(tables: &Tables, use_line: bool) -> [i32; 2] {
    [FloorFamily::LowerToLowest, FloorFamily::RaiseToNearest]
        .map(|family| i32::from(tables.floor_special(family, use_line)))
}

/// `progression.floors`: the recognized floor targets by shape, plus the
/// refusals ([`crate::lift::floor::recognize`], which also counts a floor
/// line naming no target at all as a refusal).
///
/// Informational, never graded: the map-spec frontmatter has no floor word
/// yet — no `progression.floors` parameter to compare against — so the row's
/// target is `"any"` and its verdict [`Verdict::Info`]. It exists because
/// the shapes are the one thing about a floor action a reader of the report
/// cannot get from any other row: the four specials are spread across
/// `switches.count` and `walkover_triggers.count`, which say how the actions
/// are fired and nothing about what they do.
fn floors_row(scene: &Scene, tables: &Tables) -> ConformanceRow {
    let r = recognize(scene, tables);
    ConformanceRow {
        parameter: "progression.floors".to_owned(),
        target: "any".to_owned(),
        actual: format!(
            "drop walls ×{}, reveals ×{}, bridges ×{}, refused ×{}",
            r.counts.drop_walls,
            r.counts.reveals,
            r.counts.bridges,
            r.counts.refusals()
        ),
        verdict: Verdict::Info,
    }
}

// ---------------------------------------------------------------------
// combat
// ---------------------------------------------------------------------

/// The document-vocabulary name of an [`EncounterStyle`] variant
/// (`map-spec.template.md`'s `incidental | ambush | arena | corridor`
/// spelling).
fn encounter_style_name(style: EncounterStyle) -> &'static str {
    match style {
        EncounterStyle::Incidental => "incidental",
        EncounterStyle::Ambush => "ambush",
        EncounterStyle::Arena => "arena",
        EncounterStyle::Corridor => "corridor",
    }
}

/// `combat.encounter_style`: always [`Verdict::NotDerivable`] — no sourced
/// definition of what geometry an "ambush" versus a "corridor" fight
/// actually looks like exists to measure against.
fn encounter_style_row(fm: &Frontmatter) -> ConformanceRow {
    not_derivable(
        "combat.encounter_style".to_owned(),
        encounter_style_name(fm.combat.encounter_style).to_owned(),
        "no sourced geometric definition exists",
    )
}

/// The document-vocabulary name of a [`Propagation`] variant
/// (`map-spec.template.md`'s `open | contained | sealed` spelling).
fn propagation_name(propagation: Propagation) -> &'static str {
    match propagation {
        Propagation::Open => "open",
        Propagation::Contained => "contained",
        Propagation::Sealed => "sealed",
    }
}

/// `combat.sound.propagation`: always [`Verdict::NotDerivable`] — no sourced
/// definition of how far sound actually carries between emitted sectors
/// exists to measure against.
fn sound_propagation_row(fm: &Frontmatter) -> ConformanceRow {
    not_derivable(
        "combat.sound.propagation".to_owned(),
        propagation_name(fm.combat.sound.propagation).to_owned(),
        "no sourced geometric definition exists",
    )
}

/// Whether the region reached from `start` without ever entering `wall`
/// holds a monster **and** is closed — no other neighbor of `wall` lies in
/// it, so `wall` is the region's only way in.
///
/// Breadth-first over `adjacency`, which is two-sided adjacency
/// ([`local_adjacency`]) rather than passability: a closet is sealed by
/// where its lines are, not by whether a player could squeeze through one,
/// and the wall this walk starts beside is by definition a boundary nobody
/// can cross yet. Reaching another neighbor of `wall` ends the walk at once:
/// the region is open whatever it holds.
fn closed_monster_region(
    adjacency: &BTreeMap<usize, BTreeSet<usize>>,
    wall: usize,
    start: usize,
    monster_sectors: &BTreeSet<usize>,
) -> bool {
    let empty = BTreeSet::new();
    let siblings = adjacency.get(&wall).unwrap_or(&empty);
    let mut seen: BTreeSet<usize> = BTreeSet::from([start]);
    let mut queue: VecDeque<usize> = VecDeque::from([start]);
    let mut holds_monster = false;
    while let Some(sector) = queue.pop_front() {
        holds_monster |= monster_sectors.contains(&sector);
        for &next in adjacency.get(&sector).unwrap_or(&empty) {
            if next == wall || !seen.insert(next) {
                continue;
            }
            if siblings.contains(&next) {
                return false;
            }
            queue.push_back(next);
        }
    }
    holds_monster
}

/// The floor-driven monster closets: a recognized floor target
/// ([`recognize`]) that releases monsters when it fires.
///
/// A [`Shape::Reveal`] counts when its own cell holds a monster — the sealed
/// island that lowers flush, whose contents step out. A [`Shape::DropWall`]
/// counts when a [`closed_monster_region`] lies behind it, tried from each
/// of the wall's two-sided neighbors and counted once however many of them
/// qualify: the wall is one closet, not two. The walk has to *walk*, because
/// the compiler puts a passage sector on either side of a drop wall — a
/// one-neighbor "does this pocket hold a monster" test would only ever see
/// the passage.
///
/// What this cannot see: whether the player starts inside the region. A
/// sealed dead-end wing of a map, with a drop wall between it and a monster
/// standing in the room the player begins in, reads as a closet here; the
/// region test asks whether a region is sealed, not which side of it the
/// fight is on.
///
/// Two foreign-WAD shapes it counts oddly, neither of them emittable today:
/// two drop walls opening into one pocket count as two closets (each wall's
/// own walk finds the same monsters), and a wall built of several adjacent
/// segments on one tag counts as none at all — each segment's neighbors
/// include the segment beside it, so every walk reaches a sibling of the
/// wall it started from and reads the region as open.
fn floor_closets(scene: &Scene, tables: &Tables) -> usize {
    let report = recognize(scene, tables);
    if !report.floors.iter().any(|f| f.refusal.is_none()) {
        // The common case by far — a map with no floor action at all, or
        // none this recognizes — walked without building the adjacency of
        // every sector on the map first.
        return 0;
    }
    let monsters = monster_sectors(scene, tables);
    let all: BTreeSet<usize> = (0..scene.sectors.len()).collect();
    let adjacency = local_adjacency(scene, &all);
    let empty = BTreeSet::new();
    report
        .floors
        .iter()
        .filter(|f| f.refusal.is_none())
        .filter(|f| match f.shape {
            Some(Shape::Reveal) => monsters.contains(&f.sector),
            Some(Shape::DropWall) => adjacency
                .get(&f.sector)
                .unwrap_or(&empty)
                .iter()
                .any(|&n| closed_monster_region(&adjacency, f.sector, n, &monsters)),
            Some(Shape::Bridge) | None => false,
        })
        .count()
}

/// The teleport-driven monster closets: the rooms holding a monster that a
/// monsters-only teleport line fronts — the staging cell whose occupants
/// teleport into the fight rather than walking out of it.
///
/// This is [`count_teleport_ambushes`]'s host room, counted by *host* rather
/// than by pad, because a cell with two pads in it is one closet. Salto's
/// own paired spec (`tests/fixtures/salto.spec.md`) counts its one teleport
/// ambush as its one `monster_closets`, which is what says the parameter
/// means the pocket rather than the mechanism.
fn teleport_closets(scene: &Scene, tables: &Tables) -> usize {
    let specials: Vec<i32> = tables
        .monster_teleport_specials()
        .into_iter()
        .map(i32::from)
        .collect();
    let monsters = monster_sectors(scene, tables);
    scene
        .sectors
        .iter()
        .enumerate()
        .filter(|(i, _)| monsters.contains(i))
        .filter(|(_, s)| {
            s.boundary
                .iter()
                .any(|b| b.fronts_this && specials.contains(&b.special))
        })
        .count()
}

/// `combat.monster_closets`: how many pockets of monsters this map releases
/// into the fight, over the two mechanisms the checker can re-derive from
/// emitted geometry — a pocket a floor action opens ([`floor_closets`]: a
/// reveal whose cell holds a monster, or a drop wall with a closed region of
/// them behind it) or one staged behind a monsters-only teleport pad
/// ([`teleport_closets`]).
///
/// **The sealing test belongs to the floor half alone**, and it is there for
/// a reason particular to that half: a drop wall is an ordinary wall until
/// something says the monsters past it are shut in, so the closed region is
/// the only thing separating a closet from a wall with a fight somewhere
/// beyond it. Nothing of the sort is asked of the teleport half — a
/// monsters-only pad *is* the statement that its occupants arrive by
/// teleport, wherever they were standing — and asking would be wrong:
/// salto's own closet (`tests/fixtures/salto_base.json`) opens onto its
/// arena through a plain portal, and a sealing test would count the map's
/// one closet as none.
///
/// The two counts are summed rather than merged, so a pocket that used both
/// mechanisms — a region behind a drop wall that itself holds a
/// monsters-only pad, which nothing in the compiler emits — would count
/// twice. That is the reading this keeps: such a pocket really does release
/// its monsters two ways.
fn count_monster_closets(scene: &Scene, tables: &Tables) -> u32 {
    let closets = floor_closets(scene, tables) + teleport_closets(scene, tables);
    u32::try_from(closets).expect("closet counts fit u32")
}

/// One [`range_row`] per [`crate::spec::frontmatter::MonsterSpec`] in
/// `fm.combat.monsters` (placed count of that species versus its
/// `min..=max`), plus an extra [`Verdict::Fail`] row, target `"absent"`, for
/// every species `scene` places that the spec's list never names at all —
/// after the one [`exact_row`] this block opens with,
/// `combat.monster_closets`.
fn monster_rows(fm: &Frontmatter, scene: &Scene, tables: &Tables, rows: &mut Vec<ConformanceRow>) {
    rows.push(exact_row(
        "combat.monster_closets".to_owned(),
        fm.combat.monster_closets,
        count_monster_closets(scene, tables),
    ));

    let mut spec_species: HashSet<&str> = HashSet::new();
    for m in &fm.combat.monsters {
        spec_species.insert(m.species.as_str());
        let count = count_names(scene, &[m.species.as_str()]);
        rows.push(range_row(
            format!("combat.monsters.{}", m.species),
            &MinMax {
                min: m.min,
                max: m.max,
            },
            count,
        ));
    }

    let mut placed_species: BTreeSet<&str> = BTreeSet::new();
    for t in &scene.things {
        if let Some(name) = t.name.as_deref()
            && tables.spawnhealth(name).is_some()
        {
            placed_species.insert(name);
        }
    }
    for species in placed_species {
        if !spec_species.contains(species) {
            let count = count_names(scene, &[species]);
            rows.push(ConformanceRow {
                parameter: format!("combat.monsters.{species}"),
                target: "absent".to_owned(),
                actual: count.to_string(),
                verdict: Verdict::Fail,
            });
        }
    }
}

/// `combat.hitscanner_ratio`: hitscan monsters over total monsters
/// (`tables.hitscan(name)`). `actual` reads `"no monsters"` when none are
/// placed, per the brief — still [`Verdict::Info`], never [`Verdict::Fail`].
fn hitscanner_ratio_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let placed = monsters(scene, tables);
    if placed.is_empty() {
        return ConformanceRow {
            parameter: "combat.hitscanner_ratio".to_owned(),
            target: format!("{:.3}", fm.combat.hitscanner_ratio),
            actual: "no monsters".to_owned(),
            verdict: Verdict::Info,
        };
    }
    let hitscan = placed
        .iter()
        .filter(|t| {
            t.name
                .as_deref()
                .is_some_and(|name| tables.hitscan(name) == Some(true))
        })
        .count();
    info_row(
        "combat.hitscanner_ratio".to_owned(),
        fm.combat.hitscanner_ratio,
        ratio_of(hitscan, placed.len()),
    )
}

/// `combat.ambush.deaf_ratio`: monsters carrying the `MTF_AMBUSH` bit
/// (`Tables::thing_flag("ambush")`) over total monsters. `actual` reads
/// `"no monsters"` when none are placed, per the brief.
fn deaf_ratio_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let placed = monsters(scene, tables);
    if placed.is_empty() {
        return ConformanceRow {
            parameter: "combat.ambush.deaf_ratio".to_owned(),
            target: format!("{:.3}", fm.combat.ambush.deaf_ratio),
            actual: "no monsters".to_owned(),
            verdict: Verdict::Info,
        };
    }
    let ambush = tables
        .thing_flag("ambush")
        .expect("the ambush flag is sourced");
    let deaf = placed.iter().filter(|t| t.flags & ambush != 0).count();
    info_row(
        "combat.ambush.deaf_ratio".to_owned(),
        fm.combat.ambush.deaf_ratio,
        ratio_of(deaf, placed.len()),
    )
}

// ---------------------------------------------------------------------
// arsenal.ammo.ratio
// ---------------------------------------------------------------------

/// Ammo units placed on the map, one running total per ammo pool.
#[derive(Debug, Default)]
struct AmmoUnits {
    bullets: i32,
    shells: i32,
    cells: i32,
    rockets: i32,
}

impl AmmoUnits {
    /// Credits `amount` units of `ammo_type` to the matching pool.
    fn add(&mut self, ammo_type: AmmoType, amount: i32) {
        match ammo_type {
            AmmoType::Bullets => self.bullets += amount,
            AmmoType::Shells => self.shells += amount,
            AmmoType::Cells => self.cells += amount,
            AmmoType::Rockets => self.rockets += amount,
        }
    }
}

/// `arsenal.ammo.ratio` (documented modeling decision, always
/// [`Verdict::Info`]): placed ammo damage capacity over total baseline
/// monster HP.
///
/// **Pool rate.** Each of the four ammo pools' damage-per-unit is the
/// **max** [`crate::tables::WeaponDamage::expected_damage_per_ammo`] among
/// weapons drawing that pool that are either **placed on the map** or the
/// **pistol** (always carried, so its `bullets` figure is always available
/// even with no weapon thing placed at all). "Placed on the map" is
/// classified by `tables.weapon_damage(name).is_some()` — every thing name
/// with a `[weapons.damage.*]` entry — rather than a hardcoded name list,
/// so a weapon added to that table later is picked up automatically instead
/// of silently falling out of the pool-rate set. A pool with no available
/// weapon (e.g. no cell weapon placed) contributes zero damage — never
/// `NaN` or a fallback guess.
///
/// **Ammo units.** Sum of `tables.ammo_pickup(name).amount` over every
/// placed ammo thing, plus `backpack` count times
/// [`Tables::ammo_backpack_grant`] (credited to all four pools at once, per
/// its own shape), plus [`Tables::weapon_ammo_grant`] for every placed
/// weapon thing (picking up a weapon grants ammo too, not only the ammo
/// pickups proper).
///
/// **Denominator.** Sum of `tables.spawnhealth(species)` over every placed
/// monster. `actual` reads `"no monsters"` when that sum is zero, per the
/// brief — division by a real baseline, not an invented one.
fn ammo_ratio_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let mut available: HashSet<&str> = scene
        .things
        .iter()
        .filter_map(|t| t.name.as_deref())
        .filter(|&name| tables.weapon_damage(name).is_some())
        .collect();
    available.insert("pistol");

    let pool_rate = |pool: AmmoType| -> f64 {
        available
            .iter()
            .filter_map(|&name| tables.weapon_damage(name))
            .filter(|wd| wd.ammo_type == pool)
            .map(|wd| wd.expected_damage_per_ammo)
            .fold(0.0_f64, f64::max)
    };

    let mut units = AmmoUnits::default();
    let mut backpacks = 0_i32;
    for thing in &scene.things {
        let Some(name) = thing.name.as_deref() else {
            continue;
        };
        if name == "backpack" {
            backpacks += 1;
            continue;
        }
        if let Some(pickup) = tables.ammo_pickup(name) {
            units.add(pickup.ammo_type, pickup.amount);
        }
        if let Some(grant) = tables.weapon_ammo_grant(name) {
            units.add(grant.ammo_type, grant.amount);
        }
    }
    let backpack_grant = tables.ammo_backpack_grant();
    units.bullets += backpacks * backpack_grant.bullets;
    units.shells += backpacks * backpack_grant.shells;
    units.cells += backpacks * backpack_grant.cells;
    units.rockets += backpacks * backpack_grant.rockets;

    let total_damage = f64::from(units.bullets) * pool_rate(AmmoType::Bullets)
        + f64::from(units.shells) * pool_rate(AmmoType::Shells)
        + f64::from(units.cells) * pool_rate(AmmoType::Cells)
        + f64::from(units.rockets) * pool_rate(AmmoType::Rockets);

    let total_hp: i32 = scene
        .things
        .iter()
        .filter_map(|t| t.name.as_deref())
        .filter_map(|name| tables.spawnhealth(name))
        .sum();

    if total_hp == 0 {
        return ConformanceRow {
            parameter: "arsenal.ammo.ratio".to_owned(),
            target: format!("{:.3}", fm.arsenal.ammo.ratio),
            actual: "no monsters".to_owned(),
            verdict: Verdict::Info,
        };
    }

    info_row(
        "arsenal.ammo.ratio".to_owned(),
        fm.arsenal.ammo.ratio,
        total_damage / f64::from(total_hp),
    )
}

// ---------------------------------------------------------------------
// sustain
// ---------------------------------------------------------------------

/// `sustain.health.*`, `sustain.armor.*`, and one row per
/// `sustain.powerups[]` entry: exact thing counts. `sustain.armor.green` /
/// `.blue` compare against the `green_armor` / `blue_armor` thing names —
/// the frontmatter path and the placed-thing name deliberately differ here
/// (`docs/map-spec.md`'s template uses the short `green`/`blue` labels;
/// `data/vocabulary.toml`'s `[things]` table names the pickups
/// `green_armor`/`blue_armor`).
fn sustain_rows(fm: &Frontmatter, scene: &Scene, rows: &mut Vec<ConformanceRow>) {
    rows.push(exact_row(
        "sustain.health.stimpack".to_owned(),
        fm.sustain.health.stimpack,
        count_names(scene, &["stimpack"]),
    ));
    rows.push(exact_row(
        "sustain.health.medikit".to_owned(),
        fm.sustain.health.medikit,
        count_names(scene, &["medikit"]),
    ));
    rows.push(exact_row(
        "sustain.health.health_bonus".to_owned(),
        fm.sustain.health.health_bonus,
        count_names(scene, &["health_bonus"]),
    ));
    rows.push(exact_row(
        "sustain.armor.green".to_owned(),
        fm.sustain.armor.green,
        count_names(scene, &["green_armor"]),
    ));
    rows.push(exact_row(
        "sustain.armor.blue".to_owned(),
        fm.sustain.armor.blue,
        count_names(scene, &["blue_armor"]),
    ));
    rows.push(exact_row(
        "sustain.armor.armor_bonus".to_owned(),
        fm.sustain.armor.armor_bonus,
        count_names(scene, &["armor_bonus"]),
    ));
    for p in &fm.sustain.powerups {
        rows.push(exact_row(
            format!("sustain.powerups.{}", p.name),
            p.count,
            count_names(scene, &[p.name.as_str()]),
        ));
    }
}

// ---------------------------------------------------------------------
// aesthetics
// ---------------------------------------------------------------------

/// `aesthetics.lighting.min`/`.max`: the emitted min/max sector `lightlevel`
/// against each bound separately — [`Verdict::Pass`] iff the emitted minimum
/// is not below the spec's floor, and separately iff the emitted maximum is
/// not above the spec's ceiling (together, "within" the declared band).
fn lighting_rows(fm: &Frontmatter, scene: &Scene, rows: &mut Vec<ConformanceRow>) {
    let lights: Vec<i32> = scene.sectors.iter().map(|s| s.light).collect();
    match lights.iter().min() {
        Some(&min_light) => rows.push(ConformanceRow {
            parameter: "aesthetics.lighting.min".to_owned(),
            target: fm.aesthetics.lighting.min.to_string(),
            actual: min_light.to_string(),
            verdict: if min_light >= fm.aesthetics.lighting.min {
                Verdict::Pass
            } else {
                Verdict::Fail
            },
        }),
        None => rows.push(not_derivable(
            "aesthetics.lighting.min".to_owned(),
            fm.aesthetics.lighting.min.to_string(),
            "no sectors present",
        )),
    }
    match lights.iter().max() {
        Some(&max_light) => rows.push(ConformanceRow {
            parameter: "aesthetics.lighting.max".to_owned(),
            target: fm.aesthetics.lighting.max.to_string(),
            actual: max_light.to_string(),
            verdict: if max_light <= fm.aesthetics.lighting.max {
                Verdict::Pass
            } else {
                Verdict::Fail
            },
        }),
        None => rows.push(not_derivable(
            "aesthetics.lighting.max".to_owned(),
            fm.aesthetics.lighting.max.to_string(),
            "no sectors present",
        )),
    }
}

// ---------------------------------------------------------------------
// The entry point
// ---------------------------------------------------------------------

/// Judges `spec`'s frontmatter against `scene`/`stats`/`map_name`, returning
/// the row catalog in the Task 10 brief's own order (identity, scale,
/// players, progression, combat, arsenal, sustain, secrets, aesthetics).
///
/// # Panics
///
/// If `scene`/`stats` somehow carry more than [`u32::MAX`] of any counted
/// item (sectors, linedefs, things, action lines, platforms, or secret
/// sectors) — not reachable through any map this compiler or a hand-authored
/// `TEXTMAP` this checker's own `Scene::build` accepts, since a UDMF
/// declaration index is far narrower than that.
#[must_use]
pub fn rows(
    scene: &Scene,
    stats: &MapStats,
    map_name: &str,
    spec: &Spec,
    tables: &Tables,
) -> Vec<ConformanceRow> {
    let fm = &spec.frontmatter;
    let mut rows = Vec::new();

    // identity
    rows.push(identity_row(fm, map_name));
    rows.push(identity_grid_row(fm));

    // scale
    rows.push(range_row(
        "scale.sectors".to_owned(),
        &fm.scale.sectors,
        u32::try_from(stats.sectors).expect("sector counts fit u32"),
    ));
    rows.push(range_row(
        "scale.linedefs".to_owned(),
        &fm.scale.linedefs,
        u32::try_from(stats.linedefs).expect("linedef counts fit u32"),
    ));
    rows.push(scale_size_row(fm, scene));
    rows.push(vertical_range_row(fm, scene));
    rows.push(not_derivable(
        "scale.rooms".to_owned(),
        format!("{}..={}", fm.scale.rooms.min, fm.scale.rooms.max),
        "rooms are an IR concept; emitted sectors include passages/doors/alcoves",
    ));
    rows.push(not_derivable(
        "scale.play_time_minutes".to_owned(),
        format!(
            "{}..={}",
            fm.scale.play_time_minutes.min, fm.scale.play_time_minutes.max
        ),
        "runtime property",
    ));

    // players
    rows.push(exact_row(
        "players.coop_starts".to_owned(),
        fm.players.coop_starts,
        count_names(scene, &["player2_start", "player3_start", "player4_start"]),
    ));
    rows.push(exact_row(
        "players.dm_starts".to_owned(),
        fm.players.dm_starts,
        count_names(scene, &["deathmatch_start"]),
    ));
    rows.push(start_facing_row(fm, scene));
    rows.push(coop_only_items_row(fm, scene));

    // progression
    progression_rows(fm, scene, tables, &mut rows);

    // combat
    rows.push(encounter_style_row(fm));
    monster_rows(fm, scene, tables, &mut rows);
    rows.push(hitscanner_ratio_row(fm, scene, tables));
    rows.push(deaf_ratio_row(fm, scene, tables));
    rows.push(range_row(
        "combat.ambush.teleport_ambushes".to_owned(),
        &fm.combat.ambush.teleport_ambushes,
        count_teleport_ambushes(scene, tables),
    ));
    rows.push(sound_propagation_row(fm));
    rows.push(not_derivable(
        "combat.max_simultaneous".to_owned(),
        fm.combat.max_simultaneous.to_string(),
        "runtime property",
    ));

    // arsenal
    rows.push(ammo_ratio_row(fm, scene, tables));

    // sustain
    sustain_rows(fm, scene, &mut rows);

    // secrets
    rows.push(exact_row(
        "secrets.count".to_owned(),
        fm.secrets.count,
        u32::try_from(stats.secret_sectors).expect("secret sector counts fit u32"),
    ));

    // aesthetics
    lighting_rows(fm, scene, &mut rows);

    rows
}

/// Judges nothing: maps [`rows`]'s own output onto the identical row
/// catalog — same `parameter`, same `target` (every `target` value is
/// spec-derived, never scene-derived, so it stays meaningful even when
/// `scene` is corrupt) — with every `verdict` forced to [`Verdict::NotRun`]
/// and `actual` replaced with `"scene failed structural validation"`.
///
/// Used by [`crate::check::run`] when the findings list carries a hard
/// `"V-S"` `Error` (a dangling cross-reference or an unclosed sector
/// boundary): `scene` is then built from data `Scene::build` itself gave up
/// on, so judging a spec against it would produce a verdict that looks
/// decided but is not. Building on [`rows`]'s own output, rather than
/// re-deriving the parameter catalog a second time, is what guarantees the
/// row *shape* (which parameters appear, and in what order) never drifts
/// between the healthy and structurally-broken paths.
#[must_use]
pub fn not_run_rows(
    scene: &Scene,
    stats: &MapStats,
    map_name: &str,
    spec: &Spec,
    tables: &Tables,
) -> Vec<ConformanceRow> {
    rows(scene, stats, map_name, spec, tables)
        .into_iter()
        .map(|row| ConformanceRow {
            actual: "scene failed structural validation".to_owned(),
            verdict: Verdict::NotRun,
            ..row
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::scene::SceneSector;
    use crustywad::Limits;
    use crustywad::map::udmf::parse_udmf;

    /// A single 256x256 sector holding a `player1_start` facing east (angle
    /// 0, matching the template's `start_facing: east` default), two
    /// `zombieman`s (doomednum 3004, hitscan, `spawnhealth` 20 each), one
    /// `imp` (doomednum 3001, not hitscan, `spawnhealth` 60 — a species the
    /// test spec's own `combat.monsters` list never names), and one `clip`
    /// ammo pickup (doomednum 2007, grants 10 bullets).
    const FIXTURE_MAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 256.000; }
vertex { x = 0.000; y = 256.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; angle = 0; single = true; }
thing { x = 64.000; y = 32.000; type = 3004; single = true; }
thing { x = 96.000; y = 32.000; type = 3004; single = true; }
thing { x = 128.000; y = 32.000; type = 3001; single = true; }
thing { x = 160.000; y = 32.000; type = 2007; single = true; }
"#;

    /// `map-spec.template.md`, patched exactly twice: `scale.sectors`
    /// narrowed to a range the fixture's one sector satisfies, and
    /// `combat.monsters` trimmed to a single `zombieman` entry whose `min`
    /// the fixture's two placed zombiemen fall short of (so the row reads
    /// `Fail`) — leaving `imp` unnamed by the spec (so it gets the "extra
    /// species" `Fail` row). Every other field is the template's own
    /// default, per Task 10's brief: "hand-write a tiny matching spec in the
    /// test via `Spec::from_markdown` on a string built from
    /// `map-spec.template.md`'s shape."
    fn test_spec_text() -> String {
        let template = include_str!("../../map-spec.template.md");
        let patched = template
            .replace(
                "sectors: { min: 40, max: 120 }",
                "sectors: { min: 1, max: 5 }",
            )
            .replace(
                "  monsters:\n    - { species: zombieman,   min: 10, max: 18 }\n    \
                 - { species: shotgun_guy, min: 8,  max: 14 }\n    \
                 - { species: imp,         min: 12, max: 20 }\n    \
                 - { species: pinky,       min: 4,  max: 8 }\n    \
                 - { species: cacodemon,   min: 0,  max: 3 }\n    \
                 - { species: hell_knight, min: 1,  max: 2 }\n",
                "  monsters:\n    - { species: zombieman,   min: 3, max: 5 }\n",
            );
        assert_ne!(patched, template, "the patches changed nothing");
        patched
    }

    /// Builds `text`'s `Scene`/`MapStats` and [`test_spec_text`]'s parsed
    /// [`Spec`], and runs [`rows`] over them. Panics (via `expect`) rather
    /// than returning `Result` — every fixture here is known-good, so a
    /// failure here is this test's own setup being wrong, not something a
    /// caller should have to handle.
    fn rows_for(text: &str) -> Vec<ConformanceRow> {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        rows_against(text, &doc.spec)
    }

    /// [`rows_for`]'s spec-parameterized form: the same `Scene`/[`MapStats`]
    /// build over `text`, judged against `spec` rather than
    /// [`test_spec_text`]'s own frontmatter.
    fn rows_against(text: &str, spec: &Spec) -> Vec<ConformanceRow> {
        let tables = Tables::load().expect("tables");
        let map = parse_udmf(text, Limits::default()).expect("fixture parses");
        let mut findings = Vec::new();
        let scene = Scene::build(&map, &tables, &mut findings);
        assert!(findings.is_empty(), "clean fixture: {findings:?}");
        let stats = MapStats {
            sectors: map.sectors.len(),
            linedefs: map.linedefs.len(),
            sidedefs: map.sidedefs.len(),
            vertices: map.vertices.len(),
            things: map.things.len(),
            secret_sectors: 0,
        };
        rows(&scene, &stats, "MAP01", spec, &tables)
    }

    /// [`rows_for`] over [`FIXTURE_MAP`].
    fn fixture_rows() -> Vec<ConformanceRow> {
        rows_for(FIXTURE_MAP)
    }

    fn row<'a>(rows: &'a [ConformanceRow], parameter: &str) -> &'a ConformanceRow {
        rows.iter()
            .find(|r| r.parameter == parameter)
            .unwrap_or_else(|| panic!("expected a `{parameter}` row: {rows:?}"))
    }

    #[test]
    fn a_matching_sectors_range_row_is_pass() {
        let rows = fixture_rows();
        assert_eq!(row(&rows, "scale.sectors").verdict, Verdict::Pass);
    }

    #[test]
    fn an_out_of_range_monster_count_row_is_fail() {
        let rows = fixture_rows();
        let r = row(&rows, "combat.monsters.zombieman");
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(r.actual, "2", "two zombiemen are placed: {r:?}");
    }

    #[test]
    fn hitscanner_ratio_is_info_and_carries_a_delta() {
        let rows = fixture_rows();
        let r = row(&rows, "combat.hitscanner_ratio");
        assert_eq!(r.verdict, Verdict::Info);
        // 2 hitscan zombiemen of 3 total monsters = 0.667.
        assert!(r.actual.contains("0.667"), "got {r:?}");
        assert!(r.actual.contains("delta"), "got {r:?}");
    }

    #[test]
    fn scale_rooms_is_not_derivable() {
        let rows = fixture_rows();
        assert_eq!(row(&rows, "scale.rooms").verdict, Verdict::NotDerivable);
    }

    #[test]
    fn identity_grid_encounter_style_and_sound_propagation_are_not_derivable() {
        let rows = fixture_rows();
        assert_eq!(row(&rows, "identity.grid").verdict, Verdict::NotDerivable);
        assert_eq!(
            row(&rows, "combat.encounter_style").verdict,
            Verdict::NotDerivable
        );
        assert_eq!(
            row(&rows, "combat.sound.propagation").verdict,
            Verdict::NotDerivable
        );
    }

    #[test]
    fn a_placed_species_absent_from_the_spec_is_an_extra_fail_row() {
        let rows = fixture_rows();
        let r = row(&rows, "combat.monsters.imp");
        assert_eq!(r.verdict, Verdict::Fail);
        assert_eq!(r.target, "absent");
        assert_eq!(r.actual, "1", "one imp is placed: {r:?}");
    }

    #[test]
    fn ammo_ratio_matches_the_hand_computed_expectation() {
        // Hand-computed from Tables entries (see this module's doc comment
        // for the modeling decision): no weapon is placed, so only the
        // always-carried pistol is available, and only for the `bullets`
        // pool. Units: one placed `clip` grants 10 bullets (no backpack, no
        // weapon-pickup grants). Pool rate: `pistol`'s
        // `expected_damage_per_ammo` = 10.3515625. Total damage = 10 *
        // 10.3515625 = 103.515625. Denominator: 2 zombiemen (spawnhealth 20
        // each) + 1 imp (spawnhealth 60) = 100. Ratio = 103.515625 / 100 =
        // 1.03515625, which rounds to 1.035 at the row's 3-decimal
        // formatting.
        let tables = Tables::load().expect("tables");
        let clip = tables.ammo_pickup("clip").expect("clip pickup");
        let pistol = tables.weapon_damage("pistol").expect("pistol damage");
        let expected = f64::from(clip.amount) * pistol.expected_damage_per_ammo / 100.0;
        assert!(
            (expected - 1.035_156_25).abs() < 1e-9,
            "hand-computation itself: got {expected}"
        );

        let rows = fixture_rows();
        let r = row(&rows, "arsenal.ammo.ratio");
        assert_eq!(r.verdict, Verdict::Info);
        assert!(r.actual.contains("1.035"), "got {r:?}");
    }

    /// A single 256x256 sector holding a `player1_start`, one `zombieman`
    /// (`spawnhealth` 20), one `imp` (`spawnhealth` 60), one `backpack`
    /// (doomednum 8), one `shotgun` (doomednum 2001), and one
    /// `super_shotgun` (doomednum 82) — both shell weapons placed together
    /// so [`ammo_ratio_combines_max_pool_rate_backpack_and_weapon_grants`]
    /// can exercise the max-of-several-weapons pool rate, the backpack
    /// grant, and the per-weapon pickup grant in one fixture.
    const AMMO_COMBO_FIXTURE_MAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 256.000; }
vertex { x = 0.000; y = 256.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; angle = 0; single = true; }
thing { x = 64.000; y = 32.000; type = 3004; single = true; }
thing { x = 96.000; y = 32.000; type = 3001; single = true; }
thing { x = 128.000; y = 32.000; type = 8; single = true; }
thing { x = 160.000; y = 32.000; type = 2001; single = true; }
thing { x = 192.000; y = 32.000; type = 82; single = true; }
"#;

    #[test]
    fn ammo_ratio_combines_max_pool_rate_backpack_and_weapon_grants() {
        // Hand-computed from data/engine.toml, exercising three arithmetic
        // paths together: (a) two placed weapons on the SAME pool at
        // different expected_damage_per_ammo (the shotgun and the super
        // shotgun both draw `shells`) — the pool rate must take the max,
        // not the first or an average; (b) a placed backpack, granting all
        // four pools at once; (c) each placed weapon's own pickup grant
        // (picking up a weapon grants ammo too, not only dedicated ammo
        // pickups).
        //
        // Pool rates: bullets = pistol's expected_damage_per_ammo
        // (10.3515625, always carried — no bullet weapon is placed here);
        // shells = max(shotgun 72.4609375, super_shotgun 103.515625) =
        // 103.515625; cells = 0 and rockets = 0 (no weapon on either pool
        // is placed).
        //
        // Units: the one placed backpack grants bullets 10, shells 4,
        // cells 20, rockets 1 ([ammo.pickups.backpack]); the placed
        // shotgun and super_shotgun each also grant their own pickup ammo
        // ([ammo.weapon_grant.shotgun] = 8 shells,
        // [ammo.weapon_grant.super_shotgun] = 8 shells) — shells units =
        // 4 + 8 + 8 = 20.
        //
        // Total damage = 10*10.3515625 (bullets) + 20*103.515625 (shells)
        // + 20*0 (cells) + 1*0 (rockets) = 103.515625 + 2070.3125 =
        // 2173.828125. Denominator = zombieman 20 + imp 60 = 80
        // spawnhealth. Ratio = 2173.828125 / 80 = 27.1728515625, which
        // rounds to "27.173" at the row's 3-decimal formatting.
        let tables = Tables::load().expect("tables");
        let pistol = tables.weapon_damage("pistol").expect("pistol damage");
        let shotgun = tables.weapon_damage("shotgun").expect("shotgun damage");
        let super_shotgun = tables
            .weapon_damage("super_shotgun")
            .expect("super shotgun damage");
        let backpack = tables.ammo_backpack_grant();
        let shotgun_grant = tables
            .weapon_ammo_grant("shotgun")
            .expect("shotgun ammo grant");
        let ssg_grant = tables
            .weapon_ammo_grant("super_shotgun")
            .expect("super shotgun ammo grant");

        let shells_rate = shotgun
            .expected_damage_per_ammo
            .max(super_shotgun.expected_damage_per_ammo);
        let shells_units = f64::from(backpack.shells + shotgun_grant.amount + ssg_grant.amount);
        let bullets_damage = f64::from(backpack.bullets) * pistol.expected_damage_per_ammo;
        let shells_damage = shells_units * shells_rate;
        let expected = (bullets_damage + shells_damage) / 80.0;
        assert!(
            (expected - 27.172_851_562_5).abs() < 1e-9,
            "hand-computation itself: got {expected}"
        );

        let rows = rows_for(AMMO_COMBO_FIXTURE_MAP);
        let r = row(&rows, "arsenal.ammo.ratio");
        assert_eq!(r.verdict, Verdict::Info);
        assert!(r.actual.contains("27.173"), "got {r:?}");
    }

    /// [`FIXTURE_MAP`] with its one sector's floor lowered to -16, below the
    /// template's default `vertical_range: { min: 0, max: 256 }` floor.
    const BELOW_VERTICAL_RANGE_FIXTURE_MAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 256.000; }
vertex { x = 0.000; y = 256.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = -16; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; angle = 0; single = true; }
"#;

    #[test]
    fn vertical_range_fails_a_below_min_floor_even_though_its_span_alone_would_pass() {
        // A single sector at floor -16 would read as trivially Pass under a
        // "span" comparison against the template's default `vertical_range:
        // { min: 0, max: 256 }` — max(floor) - min(floor) = 0, well within
        // [0, 256] — but the field's authoritative meaning
        // (`Scale::vertical_range`'s own doc comment: "The allowed floor
        // height range") is a per-floor bound, which -16 violates directly.
        // This is the exact regression a span-based implementation would
        // miss.
        let rows = rows_for(BELOW_VERTICAL_RANGE_FIXTURE_MAP);
        let r = row(&rows, "scale.vertical_range");
        assert_eq!(r.verdict, Verdict::Fail, "got {r:?}");
        assert!(r.actual.contains("-16"), "got {r:?}");

        // The unmodified fixture (floor 0, within the same [0, 256] band)
        // stays Pass, pinning the non-regressed side too.
        let clean = fixture_rows();
        assert_eq!(row(&clean, "scale.vertical_range").verdict, Verdict::Pass);
    }

    #[test]
    fn not_run_rows_keeps_the_parameter_list_and_forces_every_verdict_to_not_run() {
        let tables = Tables::load().expect("tables");
        let map = parse_udmf(FIXTURE_MAP, Limits::default()).expect("fixture parses");
        let mut findings = Vec::new();
        let scene = Scene::build(&map, &tables, &mut findings);
        let stats = MapStats {
            sectors: map.sectors.len(),
            linedefs: map.linedefs.len(),
            sidedefs: map.sidedefs.len(),
            vertices: map.vertices.len(),
            things: map.things.len(),
            secret_sectors: 0,
        };
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");

        let healthy = rows(&scene, &stats, "MAP01", &doc.spec, &tables);
        let not_run = not_run_rows(&scene, &stats, "MAP01", &doc.spec, &tables);

        assert_eq!(
            healthy.len(),
            not_run.len(),
            "not_run_rows must not add or drop rows"
        );
        for (h, nr) in healthy.iter().zip(not_run.iter()) {
            assert_eq!(h.parameter, nr.parameter, "row order/identity drifted");
            assert_eq!(h.target, nr.target, "target must survive unchanged");
            assert_eq!(nr.verdict, Verdict::NotRun);
            assert_eq!(nr.actual, "scene failed structural validation");
        }
    }

    /// A [`Boundary`] with `special`, minimal everywhere else — enough to
    /// drive [`count_specials`]/[`any_special_present`] without a parsed
    /// fixture.
    fn boundary_with_special(special: i32) -> crate::check::scene::Boundary {
        crate::check::scene::Boundary {
            a: (0.0, 0.0),
            b: (1.0, 0.0),
            linedef: 0,
            neighbor: None,
            two_sided: false,
            blocking: false,
            upper_unpegged: false,
            lower_unpegged: false,
            special,
            tag: 0,
            fronts_this: true,
            sidedef: 0,
        }
    }

    #[test]
    fn identity_slot_mismatch_is_fail() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        let r = identity_row(&doc.spec.frontmatter, "WRONGNAME");
        assert_eq!(r.verdict, Verdict::Fail, "got {r:?}");
    }

    #[test]
    fn bounding_box_scale_size_vertical_range_and_lighting_are_not_derivable_with_no_sectors() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        let scene = Scene {
            sectors: vec![],
            things: vec![],
        };

        assert!(
            bounding_box(&scene).is_none(),
            "no boundary geometry at all"
        );

        let size = scale_size_row(&doc.spec.frontmatter, &scene);
        assert_eq!(size.verdict, Verdict::NotDerivable);
        assert!(size.actual.contains("no boundary geometry"), "got {size:?}");

        let vr = vertical_range_row(&doc.spec.frontmatter, &scene);
        assert_eq!(vr.verdict, Verdict::NotDerivable);
        assert!(vr.actual.contains("no sectors present"), "got {vr:?}");

        let mut lighting = Vec::new();
        lighting_rows(&doc.spec.frontmatter, &scene, &mut lighting);
        assert_eq!(lighting.len(), 2);
        assert_eq!(
            lighting[0].verdict,
            Verdict::NotDerivable,
            "got {lighting:?}"
        );
        assert_eq!(
            lighting[1].verdict,
            Verdict::NotDerivable,
            "got {lighting:?}"
        );

        let start = start_facing_row(&doc.spec.frontmatter, &scene);
        assert_eq!(start.verdict, Verdict::NotDerivable);
        assert!(
            start.actual.contains("no player1_start placed"),
            "got {start:?}"
        );
    }

    #[test]
    fn scale_size_actual_renders_full_precision_not_rounded() {
        // A 2100.4-wide bounding box must report its exact width in
        // `actual`, not a rounded one — `{width:.0}x{height:.0}` used to
        // print "2100" for a box that is really 2100.4 wide, silently
        // disagreeing with the `pass`/`fail` judgment just above, which
        // already compares at full precision.
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        let scene = Scene {
            sectors: vec![SceneSector {
                floor: 0,
                ceiling: 128,
                light: 160,
                special: 0,
                tag: 0,
                boundary: vec![crate::check::scene::Boundary {
                    a: (0.0, 0.0),
                    b: (2100.4, 50.7),
                    linedef: 0,
                    neighbor: None,
                    two_sided: false,
                    blocking: false,
                    upper_unpegged: false,
                    lower_unpegged: false,
                    special: 0,
                    tag: 0,
                    fronts_this: true,
                    sidedef: 0,
                }],
                closed: true,
            }],
            things: vec![],
        };

        let row = scale_size_row(&doc.spec.frontmatter, &scene);
        assert_eq!(
            row.actual, "2100.4x50.7",
            "the actual size must render at full precision, not rounded: {row:?}"
        );
    }

    #[test]
    fn lighting_rows_fail_outside_the_declared_band() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        // Template default: aesthetics.lighting min 96, max 208.
        let scene = Scene {
            sectors: vec![
                SceneSector {
                    floor: 0,
                    ceiling: 128,
                    light: 50,
                    special: 0,
                    tag: 0,
                    boundary: vec![],
                    closed: true,
                },
                SceneSector {
                    floor: 0,
                    ceiling: 128,
                    light: 255,
                    special: 0,
                    tag: 0,
                    boundary: vec![],
                    closed: true,
                },
            ],
            things: vec![],
        };
        let mut rows = Vec::new();
        lighting_rows(&doc.spec.frontmatter, &scene, &mut rows);
        let (min_row, max_row) = (&rows[0], &rows[1]);
        assert_eq!(
            min_row.verdict,
            Verdict::Fail,
            "50 is below the min-96 floor: {min_row:?}"
        );
        assert_eq!(
            max_row.verdict,
            Verdict::Fail,
            "255 is above the max-208 ceiling: {max_row:?}"
        );
    }

    #[test]
    fn start_facing_mismatch_is_fail() {
        let tables = Tables::load().expect("tables");
        // Template default: start_facing east (0 degrees).
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        let scene = Scene {
            sectors: vec![],
            things: vec![SceneThing {
                x: 0.0,
                y: 0.0,
                angle: 180,
                type_id: 1,
                flags: 0,
                sector: None,
                name: Some("player1_start".to_owned()),
            }],
        };
        let r = start_facing_row(&doc.spec.frontmatter, &scene);
        assert_eq!(r.verdict, Verdict::Fail, "got {r:?}");
    }

    #[test]
    fn coop_only_items_present_but_not_targeted_is_fail() {
        let tables = Tables::load().expect("tables");
        // Template default: coop_only_items: false.
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        let scene = Scene {
            sectors: vec![],
            things: vec![SceneThing {
                x: 0.0,
                y: 0.0,
                angle: 0,
                type_id: 2001,
                flags: MULTIPLAYER_ONLY_BIT,
                sector: None,
                name: Some("shotgun".to_owned()),
            }],
        };
        let r = coop_only_items_row(&doc.spec.frontmatter, &scene);
        assert_eq!(r.verdict, Verdict::Fail, "got {r:?}");
    }

    #[test]
    fn facing_degrees_and_name_cover_every_variant() {
        assert_eq!(facing_degrees(&Facing::East), 0);
        assert_eq!(facing_degrees(&Facing::North), 90);
        assert_eq!(facing_degrees(&Facing::West), 180);
        assert_eq!(facing_degrees(&Facing::South), 270);
        assert_eq!(facing_degrees(&Facing::Degrees(45)), 45);

        assert_eq!(facing_name(&Facing::East), "east");
        assert_eq!(facing_name(&Facing::North), "north");
        assert_eq!(facing_name(&Facing::South), "south");
        assert_eq!(facing_name(&Facing::West), "west");
        assert_eq!(facing_name(&Facing::Degrees(45)), "45");
    }

    #[test]
    fn exit_kind_and_trigger_names_cover_every_variant() {
        assert_eq!(exit_kind_name(ExitKind::Normal), "normal");
        assert_eq!(exit_kind_name(ExitKind::Secret), "secret");
        assert_eq!(exit_kind_name(ExitKind::Both), "both");

        assert_eq!(exit_trigger_name(ExitTrigger::Switch), "switch");
        assert_eq!(exit_trigger_name(ExitTrigger::Teleport), "teleport");
        assert_eq!(exit_trigger_name(ExitTrigger::Walkover), "walkover");
    }

    #[test]
    fn encounter_style_and_propagation_names_cover_every_variant() {
        assert_eq!(
            encounter_style_name(EncounterStyle::Incidental),
            "incidental"
        );
        assert_eq!(encounter_style_name(EncounterStyle::Ambush), "ambush");
        assert_eq!(encounter_style_name(EncounterStyle::Arena), "arena");
        assert_eq!(encounter_style_name(EncounterStyle::Corridor), "corridor");

        assert_eq!(propagation_name(Propagation::Open), "open");
        assert_eq!(propagation_name(Propagation::Contained), "contained");
        assert_eq!(propagation_name(Propagation::Sealed), "sealed");
    }

    #[test]
    fn exit_kind_row_reports_both_and_secret_only() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");

        let secret_only = Scene {
            sectors: vec![SceneSector {
                floor: 0,
                ceiling: 128,
                light: 160,
                special: 0,
                tag: 0,
                boundary: vec![boundary_with_special(i32::from(
                    tables.secret_exit_switch_special(),
                ))],
                closed: true,
            }],
            things: vec![],
        };
        let r = exit_kind_row(&doc.spec.frontmatter, &secret_only, &tables);
        assert_eq!(r.actual, "secret", "got {r:?}");

        let both = Scene {
            sectors: vec![SceneSector {
                floor: 0,
                ceiling: 128,
                light: 160,
                special: 0,
                tag: 0,
                boundary: vec![
                    boundary_with_special(i32::from(tables.exit_switch_special())),
                    boundary_with_special(i32::from(tables.secret_exit_switch_special())),
                ],
                closed: true,
            }],
            things: vec![],
        };
        let r = exit_kind_row(&doc.spec.frontmatter, &both, &tables);
        assert_eq!(r.actual, "both", "got {r:?}");
    }

    /// [`test_spec_text`]'s frontmatter with `progression.exit.trigger`
    /// replaced by `trigger`. The template names a trigger exactly once, so
    /// this patches that one line and nothing else.
    fn frontmatter_with_exit_trigger(trigger: ExitTrigger) -> Frontmatter {
        let tables = Tables::load().expect("tables");
        let base = test_spec_text();
        let patched = base.replace(
            "trigger: switch",
            &format!("trigger: {}", exit_trigger_name(trigger)),
        );
        Spec::from_markdown(&patched, &tables)
            .expect("spec parses")
            .spec
            .frontmatter
    }

    #[test]
    fn exit_trigger_teleport_passes_only_for_a_teleport_only_exit_sector() {
        let (scene, tables) =
            crate::check::fixtures::scene_of(crate::check::fixtures::TELEPORT_MAP);
        let fm = frontmatter_with_exit_trigger(ExitTrigger::Teleport);
        let row = exit_trigger_row(&fm, &scene, &tables);
        assert_eq!(row.verdict, Verdict::Pass, "{row:?}");
        assert_eq!(row.actual, "teleport");
        // A walkover target on the same map fails: the exit's sectors are
        // reachable only across the pad, so the trigger reads `teleport`.
        let row = exit_trigger_row(
            &frontmatter_with_exit_trigger(ExitTrigger::Walkover),
            &scene,
            &tables,
        );
        assert_eq!(row.verdict, Verdict::Fail, "{row:?}");
    }

    #[test]
    fn an_uncrossable_walkover_exit_does_not_grade_as_a_teleport_exit() {
        // The one walkover exit line made blocking: `P_CrossSpecialLine`
        // never fires on it, so no sector holds a *crossable* walkover exit
        // and the teleport-only test would otherwise run `all` over an empty
        // set, which answers `true`. Two independent guards stop it reading
        // `teleport`: `teleport_only_sectors` returns `None` (its own
        // `resolve_goals` finds no goal), and the emptiness clause in the
        // row. This pins the outcome so neither can be dropped silently.
        let blocked = crate::check::fixtures::TELEPORT_MAP.replace(
            "twosided = true; special = 52; arg0 = 1;",
            "twosided = true; blocking = true; special = 52; arg0 = 1;",
        );
        assert_ne!(
            blocked,
            crate::check::fixtures::TELEPORT_MAP,
            "the patch changed nothing"
        );
        let (scene, tables) = crate::check::fixtures::scene_of(&blocked);
        assert!(
            crate::check::flood::teleport_only_sectors(&scene, &tables).is_none(),
            "a blocking exit threshold leaves the flood no goal to run toward"
        );
        let row = exit_trigger_row(
            &frontmatter_with_exit_trigger(ExitTrigger::Teleport),
            &scene,
            &tables,
        );
        assert_eq!(row.actual, "walkover", "{row:?}");
        assert_eq!(row.verdict, Verdict::Fail, "{row:?}");
    }

    #[test]
    fn teleports_count_counts_pads_not_edges() {
        let (scene, tables) =
            crate::check::fixtures::scene_of(crate::check::fixtures::TELEPORT_MAP);
        assert_eq!(count_player_pads(&scene, &tables), 1, "four edges, one pad");
    }

    #[test]
    fn teleport_ambushes_counts_monsters_only_pads_in_monster_sectors() {
        const START: &str = "thing { x = 32.0; y = 32.0; angle = 90; type = 1; single = true; }";
        const IMP: &str = "thing { x = 32.0; y = 96.0; angle = 0; type = 3001; single = true; }";
        let base = crate::check::fixtures::TELEPORT_MAP;

        // The fixture's own pad carries 97 — a special any thing may cross —
        // so a monster standing beside it is not a teleport ambush.
        let player_pad = base.replace(START, &format!("{START}\n{IMP}"));
        assert_ne!(player_pad, base, "the patch changed nothing");
        let (scene, tables) = crate::check::fixtures::scene_of(&player_pad);
        assert_eq!(
            count_teleport_ambushes(&scene, &tables),
            0,
            "97 is not a monsters-only special"
        );

        // The same pad made monsters-only: one ambush, counted once for all
        // four of the pad's edges.
        let ambush = player_pad.replace("special = 97;", "special = 126;");
        assert_ne!(ambush, player_pad, "the patch changed nothing");
        let (scene, tables) = crate::check::fixtures::scene_of(&ambush);
        assert_eq!(count_teleport_ambushes(&scene, &tables), 1);

        // A monsters-only pad in a room holding no monster is not one: the
        // ambush is the monsters, not the pad.
        let no_monster = base.replace("special = 97;", "special = 126;");
        assert_ne!(no_monster, base, "the patch changed nothing");
        let (scene, tables) = crate::check::fixtures::scene_of(&no_monster);
        assert_eq!(count_teleport_ambushes(&scene, &tables), 0);
    }

    #[test]
    fn exit_trigger_row_reports_walkover_only_and_both_present() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");

        let walkover_only = Scene {
            sectors: vec![SceneSector {
                floor: 0,
                ceiling: 128,
                light: 160,
                special: 0,
                tag: 0,
                boundary: vec![boundary_with_special(i32::from(
                    tables.exit_walkover_special(),
                ))],
                closed: true,
            }],
            things: vec![],
        };
        let r = exit_trigger_row(&doc.spec.frontmatter, &walkover_only, &tables);
        assert_eq!(r.actual, "walkover", "got {r:?}");

        let both = Scene {
            sectors: vec![SceneSector {
                floor: 0,
                ceiling: 128,
                light: 160,
                special: 0,
                tag: 0,
                boundary: vec![
                    boundary_with_special(i32::from(tables.exit_switch_special())),
                    boundary_with_special(i32::from(tables.exit_walkover_special())),
                ],
                closed: true,
            }],
            things: vec![],
        };
        let r = exit_trigger_row(&doc.spec.frontmatter, &both, &tables);
        assert_eq!(r.actual, "both switch and walkover present", "got {r:?}");
    }

    /// A switch-only exit has no walkover special at all, so
    /// `has_walkover && !has_switch` is `false` on its first operand and the
    /// flood-backed teleport check short-circuits away entirely — this pins
    /// the `switch` reading through that short-circuited path.
    #[test]
    fn exit_trigger_row_reports_switch_only() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");

        let switch_only = Scene {
            sectors: vec![SceneSector {
                floor: 0,
                ceiling: 128,
                light: 160,
                special: 0,
                tag: 0,
                boundary: vec![boundary_with_special(i32::from(
                    tables.exit_switch_special(),
                ))],
                closed: true,
            }],
            things: vec![],
        };
        let r = exit_trigger_row(&doc.spec.frontmatter, &switch_only, &tables);
        assert_eq!(r.actual, "switch", "got {r:?}");
        assert_eq!(r.verdict, Verdict::Pass, "got {r:?}");
    }

    #[test]
    fn ammo_ratio_row_skips_things_with_no_resolved_name() {
        let tables = Tables::load().expect("tables");
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        let scene = Scene {
            sectors: vec![],
            things: vec![
                SceneThing {
                    x: 0.0,
                    y: 0.0,
                    angle: 0,
                    type_id: 31337,
                    flags: 0,
                    sector: None,
                    name: None,
                },
                SceneThing {
                    x: 0.0,
                    y: 0.0,
                    angle: 0,
                    type_id: 3004,
                    flags: 0,
                    sector: None,
                    name: Some("zombieman".to_owned()),
                },
            ],
        };
        let r = ammo_ratio_row(&doc.spec.frontmatter, &scene, &tables);
        assert_eq!(r.verdict, Verdict::Info, "got {r:?}");
        assert!(
            r.actual.contains("0.000"),
            "no ammo placed, so the ratio is zero: {r:?}"
        );
    }

    /// `base` with one frontmatter block replaced: `path`'s last dotted
    /// component names a two-space-indented block key
    /// (`"progression.lifts"` names the `lifts:` block), and `body` replaces
    /// every line under it. `body`'s first line is written at the block's own
    /// four-space indent; each later line carries its own, the way the caller
    /// writes it.
    ///
    /// Returns the patched text rather than a [`Spec`] so a caller needing
    /// two blocks moved at once ([`floor_spec`]) can nest the calls;
    /// [`spec_with`] is the one-block form.
    fn patch_block(base: &str, path: &str, body: &str) -> String {
        let key = path.rsplit('.').next().expect("a dotted path names a key");
        let head = format!("\n  {key}:\n");
        let start = base
            .find(&head)
            .unwrap_or_else(|| panic!("no `{key}:` block in the template"))
            + head.len();
        // The block runs to the first line not indented past the key itself.
        let block: usize = base[start..]
            .lines()
            .take_while(|l| l.starts_with("    "))
            .map(|l| l.len() + 1)
            .sum();
        assert!(block > 0, "`{key}:` names an empty block");
        let patched = format!("{}    {body}\n{}", &base[..start], &base[start + block..]);
        assert_ne!(patched, base, "the patch changed nothing");
        patched
    }

    /// `text` parsed as a spec document. Panics (via `expect`) rather than
    /// returning `Result`: every caller here hands it the template with a
    /// known patch applied, so a parse failure is the test's own setup being
    /// wrong.
    ///
    /// Returns the whole [`Spec`] rather than its [`Frontmatter`] alone
    /// because [`rows`] takes a `&Spec`; the sibling
    /// [`frontmatter_with_exit_trigger`] can hand back a `Frontmatter` only
    /// because it feeds a single row function directly.
    fn spec_of(text: &str) -> Spec {
        let tables = Tables::load().expect("tables");
        Spec::from_markdown(text, &tables)
            .expect("spec parses")
            .spec
    }

    /// [`test_spec_text`] with one frontmatter block replaced by
    /// [`patch_block`], parsed.
    fn spec_with(path: &str, body: &str) -> Spec {
        spec_of(&patch_block(&test_spec_text(), path, body))
    }

    /// [`test_spec_text`] with the three frontmatter values a floor fixture
    /// moves: the whole `switches` and `walkover_triggers` blocks (a floor
    /// trigger counts in one or the other), and the scalar
    /// `combat.monster_closets`.
    fn floor_spec(switches: &str, walkovers: &str, closets: u32) -> Spec {
        let counts = patch_block(
            &patch_block(&test_spec_text(), "progression.switches", switches),
            "progression.walkover_triggers",
            walkovers,
        );
        assert!(
            counts.contains("monster_closets: 3"),
            "the template's `monster_closets: 3` line moved"
        );
        spec_of(&counts.replace("monster_closets: 3", &format!("monster_closets: {closets}")))
    }

    /// A drop wall with an imp sealed behind it: a four-room
    /// [`crate::check::fixtures::chain`] — `A(0)` | `T(128, tag 7)` |
    /// `B(0)` | `C(0)` — whose `B|C` line is the `23` S1 switch naming tag
    /// 7, with the imp standing in `C`. `T` is the only two-sided way into
    /// `{B, C}`, so that pair is the closed region the closet rule looks
    /// for.
    ///
    /// The imp stands one room past the wall's own neighbor on purpose: `B`
    /// stands in for the passage sector the compiler puts on each side of a
    /// drop wall, so a rule that only looked at the neighbor would find an
    /// empty room and call the closet empty.
    ///
    /// The switch rides a real line rather than
    /// [`crate::check::fixtures::far_wall`]'s doubled one: that helper
    /// leaves the doubled sector's two vertices at odd degree, which
    /// `Scene::build` reports as a hard `V-S` "boundary does not close" —
    /// harmless to the recognizer's own tests, which read the scene
    /// directly, but fatal here, since an unclosed sector also stops every
    /// thing inside it from resolving to a sector at all.
    fn drop_wall_closet() -> String {
        crate::check::fixtures::chain(
            &[0, 128, 0, 0],
            &[0, 7, 0, 0],
            &[(0, 0, false), (0, 0, false), (23, 7, false)],
            "thing { x = 448.000; y = 64.000; type = 3001; single = true; }\n",
        )
    }

    #[test]
    fn a_floor_switch_counts_as_a_switch_and_a_closet_as_a_monster_closet() {
        let spec = floor_spec(
            "count: { min: 1, max: 1 }\n    remote_allowed: true",
            "count: { min: 0, max: 0 }",
            1,
        );
        let rows = rows_against(&drop_wall_closet(), &spec);

        let switches = row(&rows, "progression.switches.count");
        assert_eq!(
            (switches.actual.as_str(), switches.verdict),
            ("1", Verdict::Pass),
            "the 23 S1 line is a switch the player presses: {switches:?}"
        );
        let walkovers = row(&rows, "progression.walkover_triggers.count");
        assert_eq!(
            (walkovers.actual.as_str(), walkovers.verdict),
            ("0", Verdict::Pass),
            "a floor use line is not also a walkover: {walkovers:?}"
        );
        let closets = row(&rows, "combat.monster_closets");
        assert_eq!(
            (
                closets.target.as_str(),
                closets.actual.as_str(),
                closets.verdict
            ),
            ("1", "1", Verdict::Pass),
            "the imp behind the drop wall is a closet: {closets:?}"
        );
    }

    /// A pedestal-shaped reveal holding an imp: a two-room
    /// [`crate::check::fixtures::chain_full`] whose east cell (`x ∈ [128,
    /// 256]`, floor 64 under a ceiling of 64 — no headroom, so no neighbor
    /// can enter it) carries tag 7, with the `23` S1 switch on the cell's
    /// own face and the imp inside. Lowering it flush with the room joins
    /// nothing new, which is what makes it a [`Shape::Reveal`] rather than a
    /// drop wall.
    ///
    /// [`Shape::Reveal`]: crate::lift::floor::Shape::Reveal
    fn reveal_closet() -> String {
        crate::check::fixtures::chain_full(
            &[0, 64],
            &[256, 64],
            &[0, 7],
            &[(23, 7, false)],
            "thing { x = 192.000; y = 64.000; type = 3001; single = true; }\n",
        )
    }

    #[test]
    fn the_floors_row_names_the_shapes() {
        // A pit strip between two walkways, raised to their floor: the
        // bridge. A `101` S1 pillar rising between two level rooms only
        // takes reach away: the recognizer refuses it, and the row counts
        // the refusal.
        let bridge = crate::check::fixtures::chain(
            &[64, 0, 64],
            &[0, 7, 0],
            &[(18, 7, false), (0, 0, false)],
            "",
        );
        let refused = crate::check::fixtures::chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(101, 7, false), (0, 0, false)],
            "",
        );
        for (text, expected) in [
            (
                drop_wall_closet(),
                "drop walls ×1, reveals ×0, bridges ×0, refused ×0",
            ),
            (
                reveal_closet(),
                "drop walls ×0, reveals ×1, bridges ×0, refused ×0",
            ),
            (bridge, "drop walls ×0, reveals ×0, bridges ×1, refused ×0"),
            (refused, "drop walls ×0, reveals ×0, bridges ×0, refused ×1"),
        ] {
            let rows = rows_for(&text);
            let r = row(&rows, "progression.floors");
            assert_eq!(
                (r.target.as_str(), r.actual.as_str(), r.verdict),
                ("any", expected, Verdict::Info),
                "{r:?}"
            );
        }
    }

    #[test]
    fn a_floor_walkover_counts_as_a_walkover_trigger_and_not_as_a_switch() {
        let spec = floor_spec(
            "count: { min: 0, max: 0 }\n    remote_allowed: true",
            "count: { min: 1, max: 1 }",
            0,
        );
        // The same drop wall, fired by a `38` W1 line instead of the `23`
        // S1 one, and with no monster behind it.
        let rows = rows_against(
            &crate::check::fixtures::chain(
                &[0, 128, 0, 0],
                &[0, 7, 0, 0],
                &[(0, 0, false), (0, 0, false), (38, 7, false)],
                "",
            ),
            &spec,
        );
        let switches = row(&rows, "progression.switches.count");
        assert_eq!(
            (switches.actual.as_str(), switches.verdict),
            ("0", Verdict::Pass),
            "a floor walkover is not a switch: {switches:?}"
        );
        let walkovers = row(&rows, "progression.walkover_triggers.count");
        assert_eq!(
            (walkovers.actual.as_str(), walkovers.verdict),
            ("1", Verdict::Pass),
            "{walkovers:?}"
        );
        let closets = row(&rows, "combat.monster_closets");
        assert_eq!(
            (closets.actual.as_str(), closets.verdict),
            ("0", Verdict::Pass),
            "an empty pocket is no closet: {closets:?}"
        );
    }

    #[test]
    fn a_reveal_holding_a_monster_is_a_closet() {
        let spec = floor_spec(
            "count: { min: 1, max: 1 }\n    remote_allowed: true",
            "count: { min: 0, max: 0 }",
            1,
        );
        let rows = rows_against(&reveal_closet(), &spec);
        assert_eq!(
            row(&rows, "progression.floors").actual,
            "drop walls ×0, reveals ×1, bridges ×0, refused ×0"
        );
        let closets = row(&rows, "combat.monster_closets");
        assert_eq!(
            (closets.actual.as_str(), closets.verdict),
            ("1", Verdict::Pass),
            "the imp sealed in the cell is a closet: {closets:?}"
        );
    }

    #[test]
    fn a_monsters_only_pad_in_a_monster_room_is_a_closet_and_a_player_pad_is_not() {
        const START: &str = "thing { x = 32.0; y = 32.0; angle = 90; type = 1; single = true; }";
        const IMP: &str = "thing { x = 32.0; y = 96.0; angle = 0; type = 3001; single = true; }";
        let base = crate::check::fixtures::TELEPORT_MAP;

        // The same three cases `teleport_ambushes_counts_monsters_only_pads_
        // in_monster_sectors` pins, read as closets: a pad any thing may
        // cross (97) is no closet however many monsters stand by it, a
        // monsters-only pad (126) in a room with a monster is one — counted
        // once for all four of the pad's edges, because the count is by host
        // room — and one in an empty room is none.
        let player_pad = base.replace(START, &format!("{START}\n{IMP}"));
        assert_ne!(player_pad, base, "the patch changed nothing");
        let (scene, tables) = crate::check::fixtures::scene_of(&player_pad);
        assert_eq!(count_monster_closets(&scene, &tables), 0);

        let ambush = player_pad.replace("special = 97;", "special = 126;");
        assert_ne!(ambush, player_pad, "the patch changed nothing");
        let (scene, tables) = crate::check::fixtures::scene_of(&ambush);
        assert_eq!(count_monster_closets(&scene, &tables), 1);

        let no_monster = base.replace("special = 97;", "special = 126;");
        assert_ne!(no_monster, base, "the patch changed nothing");
        let (scene, tables) = crate::check::fixtures::scene_of(&no_monster);
        assert_eq!(count_monster_closets(&scene, &tables), 0);
    }

    /// The [`drop_wall_closet`]'s topological opposite: `T` (sector 1, floor
    /// 128, tag 7) still separates `A` (sector 0) from `B` (sector 2), and
    /// `B` still holds the imp, but a U-shaped corridor `C` (sector 3) runs
    /// north of all three and joins `A` to `B` around the wall. `C` shares
    /// no line with `T` — the notch at `y ∈ [128, 160]`, `x ∈ [128, 256]` is
    /// void, so `T`'s north wall and `C`'s southern step are separate
    /// one-sided lines — which is what keeps the wall a recognized
    /// `DropWall`: [`crate::check::floors::classify_effect`] reads only the
    /// local graph `{T} ∪ neighbors(T)`, in which `A` and `B` are still
    /// joined by nothing but `T`.
    ///
    /// The `23` S1 switch rides `C`'s north wall — a remote trigger, which
    /// the recognizer accepts.
    ///
    /// Every linedef is wound so the sector named by its front sidedef lies
    /// to the right of `v1 -> v2`, and every sector's vertex degrees are
    /// even, so all four close.
    const BYPASSED_DROP_WALL: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 0.000; y = 128.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 128.000; }
vertex { x = 384.000; y = 0.000; }
vertex { x = 384.000; y = 128.000; }
vertex { x = 128.000; y = 160.000; }
vertex { x = 256.000; y = 160.000; }
vertex { x = 384.000; y = 256.000; }
vertex { x = 0.000; y = 256.000; }
linedef { v1 = 3; v2 = 2; sidefront = 0; sideback = 1; twosided = true; }
linedef { v1 = 5; v2 = 4; sidefront = 2; sideback = 3; twosided = true; }
linedef { v1 = 1; v2 = 3; sidefront = 4; sideback = 5; twosided = true; }
linedef { v1 = 5; v2 = 7; sidefront = 6; sideback = 7; twosided = true; }
linedef { v1 = 0; v2 = 1; sidefront = 8; blocking = true; }
linedef { v1 = 2; v2 = 0; sidefront = 9; blocking = true; }
linedef { v1 = 4; v2 = 2; sidefront = 10; blocking = true; }
linedef { v1 = 3; v2 = 5; sidefront = 11; blocking = true; }
linedef { v1 = 6; v2 = 4; sidefront = 12; blocking = true; }
linedef { v1 = 7; v2 = 6; sidefront = 13; blocking = true; }
linedef { v1 = 8; v2 = 3; sidefront = 14; blocking = true; }
linedef { v1 = 9; v2 = 8; sidefront = 15; blocking = true; }
linedef { v1 = 5; v2 = 9; sidefront = 16; blocking = true; }
linedef { v1 = 7; v2 = 10; sidefront = 17; blocking = true; }
linedef { v1 = 11; v2 = 10; sidefront = 18; blocking = true; special = 23; arg0 = 7; }
linedef { v1 = 1; v2 = 11; sidefront = 19; blocking = true; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 3; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 3; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "SW1COMP"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 128; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; }
thing { x = 320.000; y = 64.000; type = 3001; single = true; }
"#;

    #[test]
    fn a_drop_wall_whose_far_side_is_reachable_another_way_is_no_closet() {
        let spec = floor_spec(
            "count: { min: 1, max: 1 }\n    remote_allowed: true",
            "count: { min: 0, max: 0 }",
            0,
        );
        let rows = rows_against(BYPASSED_DROP_WALL, &spec);
        assert_eq!(
            row(&rows, "progression.floors").actual,
            "drop walls ×1, reveals ×0, bridges ×0, refused ×0",
            "the wall is still a recognized drop wall"
        );
        let closets = row(&rows, "combat.monster_closets");
        assert_eq!(
            (closets.actual.as_str(), closets.verdict),
            ("0", Verdict::Pass),
            "the imp's room is reachable around the wall, so nothing is closeted: {closets:?}"
        );
    }

    /// The lift golden (`tests/golden/lifts.json`) compiled and emitted as
    /// TEXTMAP: one both-ends lift, one fast barrier, one fast walkover lift
    /// and one pedestal. The same compile -> emit round trip
    /// `tests/check_conformance.rs` runs, so these rows judge the artifact
    /// the checker actually sees rather than the IR behind it.
    fn lifts_golden_textmap() -> String {
        let tables = Tables::load().expect("tables");
        let ir = crate::ir::Ir::from_json(include_str!("../../tests/golden/lifts.json"))
            .expect("the lift golden parses");
        let compiled = crate::compile::compile(&ir, &tables).expect("the lift golden compiles");
        crate::compile::textmap::emit_textmap(&compiled.data, &compiled.things)
    }

    #[test]
    fn lift_rows_on_the_lift_golden() {
        let spec = spec_with(
            "progression.lifts",
            "count: { min: 4, max: 4 }\n    trigger: both_ends\n    max_travel: 256",
        );
        let rows = rows_against(&lifts_golden_textmap(), &spec);
        assert_eq!(
            row(&rows, "progression.lifts.count").actual,
            "4",
            "plats, not trigger lines"
        );
        assert_eq!(
            row(&rows, "progression.switches.count").actual,
            "8",
            "1 exit + 1 riser + 2 barrier faces + 4 pedestal edges"
        );
        let trigger = row(&rows, "progression.lifts.trigger");
        assert_eq!(trigger.actual, "switch ×0, walkover ×1, both_ends ×1");
        assert_eq!(
            trigger.verdict,
            Verdict::Fail,
            "the walkover lift does not match both_ends"
        );
        let travel = row(&rows, "progression.lifts.max_travel");
        assert_eq!(
            (travel.actual.as_str(), travel.verdict),
            ("128", Verdict::Pass)
        );
    }

    #[test]
    fn a_single_switch_lift_matches_switch_and_no_lifts_pass_vacuously() {
        let spec = spec_with(
            "progression.lifts",
            "count: { min: 1, max: 1 }\n    trigger: switch\n    max_travel: 64",
        );
        let rows = rows_against(
            &crate::check::fixtures::chain(
                &[0, 128, 128],
                &[0, 7, 0],
                &[(62, 7, false), (0, 0, false)],
                "",
            ),
            &spec,
        );
        assert_eq!(
            row(&rows, "progression.lifts.trigger").verdict,
            Verdict::Pass
        );
        let travel = row(&rows, "progression.lifts.max_travel");
        assert_eq!(
            (travel.actual.as_str(), travel.verdict),
            ("128", Verdict::Fail)
        );

        let rows = rows_against(
            &crate::check::fixtures::chain(&[0, 0], &[0, 0], &[(0, 0, false)], ""),
            &spec,
        );
        let trigger = row(&rows, "progression.lifts.trigger");
        assert_eq!(
            (trigger.actual.as_str(), trigger.verdict),
            ("no lifts", Verdict::Pass)
        );
        assert_eq!(
            row(&rows, "progression.lifts.max_travel").actual,
            "no lifts"
        );
    }

    /// A platform at rest on top whose only trigger fires from a `Level`
    /// side (the link is flipped so the *east*, ledge-height room is the use
    /// line's front): [`lift_form`] reads `"top_only"`, a form no template
    /// word names, so the row fails whichever word the spec chose. The same
    /// platform is V-P5's "callable only from above" warning, and this pins
    /// that the two never disagree.
    #[test]
    fn a_top_only_lift_matches_no_template_word() {
        let text = crate::check::fixtures::chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(0, 0, false), (62, 7, true)],
            "",
        );
        for word in ["switch", "walkover", "both_ends"] {
            let spec = spec_with(
                "progression.lifts",
                &format!("count: {{ min: 1, max: 1 }}\n    trigger: {word}\n    max_travel: 256"),
            );
            let rows = rows_against(&text, &spec);
            let r = row(&rows, "progression.lifts.trigger");
            assert_eq!(r.target, word);
            assert_eq!(r.actual, "switch ×0, walkover ×0, both_ends ×0", "{r:?}");
            assert_eq!(r.verdict, Verdict::Fail, "{r:?}");
        }
    }
}
