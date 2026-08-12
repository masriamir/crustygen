//! Spec conformance rows: every frontmatter parameter [`crate::spec::Spec`]
//! declares, judged against what the parsed [`Scene`] and [`MapStats`]
//! actually show (`docs/design.md` §8.1's conformance report). Unlike
//! `scene.rs`/`invariants.rs`/`flood.rs`, nothing here re-derives a
//! playability rule from the pinned engine — every row is a plain
//! target-vs-actual comparison, so the only sourcing burden is the ammo
//! ratio's damage-per-ammo figures ([`crate::tables::Tables::weapon_damage`],
//! [`crate::tables::Tables::weapon_ammo_grant`]) and the two engine facts
//! cited on the `AMBUSH_BIT`/`MULTIPLAYER_ONLY_BIT` thing-flag constants
//! below.
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
use crate::spec::frontmatter::{ExitKind, ExitTrigger, Facing, Frontmatter, MinMax};
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
        actual: format!("{width:.0}x{height:.0}"),
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
/// entirely in single-player. Unlike [`AMBUSH_BIT`], this bit has no named
/// `MTF_*` constant in the pinned `doomdef.h`; the source above uses the
/// raw literal `16` itself.
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
/// specials found. [`Verdict::NotDerivable`] when the spec targets
/// [`ExitTrigger::Teleport`] — this compiler emits no teleports yet
/// (`KNOWN-GAPS.md`), so that target can never be measured.
fn exit_trigger_row(fm: &Frontmatter, scene: &Scene, tables: &Tables) -> ConformanceRow {
    let target = exit_trigger_name(fm.progression.exit.trigger).to_owned();
    if fm.progression.exit.trigger == ExitTrigger::Teleport {
        return not_derivable(
            "progression.exit.trigger".to_owned(),
            target,
            "no teleports emitted",
        );
    }
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
    let actual_trigger = match (has_switch, has_walkover) {
        (true, false) => Some(ExitTrigger::Switch),
        (false, true) => Some(ExitTrigger::Walkover),
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

// ---------------------------------------------------------------------
// combat
// ---------------------------------------------------------------------

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

/// The `MTF_AMBUSH` "deaf" thing-flag bit — bit 3, value 8.
///
/// Sourced from `doomdef.h` (pinned commit
/// `a77dfb96cb91780ca334d0d4cfd86957558007e0`): `#define MTF_AMBUSH 8`, and
/// `p_mobj.c`'s `P_SpawnMapThing`: `if (mthing->options & MTF_AMBUSH)
/// mobj->flags |= MF_AMBUSH;` — the flag that makes a monster wake on sight
/// rather than on hearing the player.
const AMBUSH_BIT: u32 = 8;

/// `combat.ambush.deaf_ratio`: monsters carrying [`AMBUSH_BIT`] over total
/// monsters. `actual` reads `"no monsters"` when none are placed, per the
/// brief.
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
    let deaf = placed.iter().filter(|t| t.flags & AMBUSH_BIT != 0).count();
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
        count_specials(scene, &[i32::from(tables.teleport_special())]),
    ));

    // combat
    monster_rows(fm, scene, tables, &mut rows);
    rows.push(hitscanner_ratio_row(fm, scene, tables));
    rows.push(deaf_ratio_row(fm, scene, tables));
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
