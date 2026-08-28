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
//! `scene.rs`/`invariants.rs`/`flood.rs`, all but one row here is a plain
//! target-vs-actual comparison that re-derives no playability rule from the
//! pinned engine, so the only sourcing burden is the ammo ratio's
//! damage-per-ammo figures ([`crate::tables::Tables::weapon_damage`],
//! [`crate::tables::Tables::weapon_ammo_grant`]), the `MTF_AMBUSH` bit
//! ([`crate::tables::Tables::thing_flag`], sourced in `engine.toml`'s
//! `[thing.flags]`), the teleport specials the two pad counts read
//! ([`crate::tables::Tables::player_teleport_specials`],
//! [`crate::tables::Tables::monster_teleport_specials`]), and the engine fact
//! cited on the `MULTIPLAYER_ONLY_BIT` thing-flag constant below. The
//! exception is `progression.exit.trigger`: a teleport exit emits the same
//! specials as a plain walkover one, so the row borrows
//! [`crate::check::flood::teleport_only_sectors`]'s reachability predicate
//! rather than reading the line.
//!
//! [`rows`] implements exactly the row catalog in the Task 10 brief, in the
//! brief's own order, and follows its verdict rules: a `MinMax` or exact-count
//! target is [`Verdict::Pass`]/[`Verdict::Fail`]; a scalar continuous target
//! (`hitscanner_ratio`, `deaf_ratio`, `ammo.ratio`) is always
//! [`Verdict::Info`], its `actual` formatted `"<value> (target <t>, delta
//! <d>)"` rather than judged against an invented tolerance; a parameter this
//! checker cannot derive from emitted geometry at all is
//! [`Verdict::NotDerivable`], its `actual` carrying the reason.

use std::collections::{BTreeSet, HashSet};

use crate::check::scene::{Scene, SceneThing};
use crate::check::{ConformanceRow, MapStats, Verdict};
use crate::spec::Spec;
use crate::spec::frontmatter::{
    EncounterStyle, ExitKind, ExitTrigger, Facing, Frontmatter, MinMax, Propagation,
};
use crate::tables::{AmmoType, Tables};

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
    let monster_sectors: BTreeSet<usize> = monsters(scene, tables)
        .iter()
        .filter_map(|t| t.sector)
        .collect();
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
/// The "crossable" qualifier makes that set non-empty whenever a walkover
/// special is present at all, so the `all` below is never vacuously true on
/// a map worth grading: `P_CrossSpecialLine` fires on neither a one-sided
/// line nor a blocking one, so [`crate::compile::exits`] builds every
/// walkover threshold two-sided and non-blocking, and a hand-authored map
/// that did otherwise has an exit the engine would never fire.
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
    let teleport_only = crate::check::flood::teleport_only_sectors(scene, tables);
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
    let is_teleport_exit = has_walkover
        && !has_switch
        && teleport_only
            .as_ref()
            .is_some_and(|only| walkover_goal_sectors.iter().all(|&s| only[s]));
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

/// Every `progression.*` row: keys, locked doors, exit kind/trigger, and the
/// four switch/walkover/lift/teleport `MinMax` counts. Extracted out of
/// [`rows`] itself (alongside [`monster_rows`]/[`sustain_rows`]/
/// [`lighting_rows`]) purely to keep that function under clippy's line-count
/// lint — the row order and content are unchanged either way.
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
    rows.push(range_row(
        "progression.switches.count".to_owned(),
        &fm.progression.switches.count,
        count_specials(
            scene,
            &[
                i32::from(tables.exit_switch_special()),
                i32::from(tables.secret_exit_switch_special()),
            ],
        ),
    ));
    rows.push(range_row(
        "progression.walkover_triggers.count".to_owned(),
        &fm.progression.walkover_triggers.count,
        count_specials(
            scene,
            &[
                i32::from(tables.exit_walkover_special()),
                i32::from(tables.secret_exit_walkover_special()),
            ],
        ),
    ));
    rows.push(range_row(
        "progression.lifts.count".to_owned(),
        &fm.progression.lifts.count,
        count_specials(
            scene,
            &[
                i32::from(tables.lift_switch_special()),
                i32::from(tables.lift_walkover_special()),
            ],
        ),
    ));
    rows.push(range_row(
        "progression.teleports.count".to_owned(),
        &fm.progression.teleports.count,
        count_player_pads(scene, tables),
    ));
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

/// One [`range_row`] per [`crate::spec::frontmatter::MonsterSpec`] in
/// `fm.combat.monsters` (placed count of that species versus its
/// `min..=max`), plus an extra [`Verdict::Fail`] row, target `"absent"`, for
/// every species `scene` places that the spec's list never names at all.
fn monster_rows(fm: &Frontmatter, scene: &Scene, tables: &Tables, rows: &mut Vec<ConformanceRow>) {
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
/// item (sectors, linedefs, things, action lines, or secret sectors) — not
/// reachable through any map this compiler or a hand-authored `TEXTMAP` this
/// checker's own `Scene::build` accepts, since a UDMF declaration index is
/// far narrower than that.
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
        let doc = Spec::from_markdown(&test_spec_text(), &tables).expect("spec parses");
        rows(&scene, &stats, "MAP01", &doc.spec, &tables)
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
}
