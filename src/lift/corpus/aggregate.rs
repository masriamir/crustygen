//! Rolling a sweep's unique maps up into an [`Aggregate`]: per-axis shares,
//! ranked blockers, the two greedy curves and the three recognizer roll-ups.

use std::collections::{BTreeMap, BTreeSet};

use super::MapRecord;
use crate::lift::floor::FloorCounts;
use crate::lift::plat::PlatCounts;
use crate::lift::teleport::TeleportCounts;
use crate::lift::vocabulary::Verdict;

/// Where the greedy curve is sampled for the report.
pub const GREEDY_CHECKPOINTS: [usize; 5] = [1, 5, 10, 21, 51];
const _: () = assert!(
    !GREEDY_CHECKPOINTS.is_empty(),
    "the report's stepless-curve guard relies on a non-empty checkpoint list"
);
/// How many blockers per axis the report lists.
pub const BLOCKER_TOP: usize = 25;

/// Expressibility per axis as a fraction of a population of maps.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct AxisShare {
    /// Fraction whose line specials are all emittable.
    pub line_specials: f64,
    /// Fraction whose sector specials are all nameable.
    pub sector_specials: f64,
    /// Fraction whose thing types are all in vocabulary.
    pub thing_kinds: f64,
    /// Fraction whose teleport lines are all recognized (no refusal). A map
    /// with no teleport line passes vacuously.
    pub teleports: f64,
    /// Fraction whose platforms are all recognized and whose every lift line
    /// names one (no refusal). A map with no lift line passes vacuously.
    pub lifts: f64,
    /// Fraction whose floor targets are all recognized and whose every floor
    /// line names one (no refusal). A map with no floor line passes
    /// vacuously.
    pub floors: f64,
    /// Fraction expressible on all six axes.
    pub expressible: f64,
}

/// One out-of-set value and how many maps carry it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Blocker {
    /// The raw value.
    pub value: i32,
    /// Maps carrying it.
    pub maps: u64,
    /// `maps / maps_unique`.
    pub share: f64,
}

/// One greedy step.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct GreedyStep {
    /// 1-based step.
    pub k: usize,
    /// The special added at this step.
    pub special: i32,
    /// Fraction of all unique maps unblocked after this step.
    pub cumulative_share: f64,
}

/// The greedy "add the special that unblocks the most maps" curve.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GreedyCurve {
    /// Every step until nothing is left to unblock.
    pub steps: Vec<GreedyStep>,
    /// `(k, cumulative_share)` at each of [`GREEDY_CHECKPOINTS`] that the
    /// curve reaches (the share at the last step is used past its end).
    pub checkpoints: Vec<(usize, f64)>,
}

/// The teleport recognizer's roll-up over a sweep. Every count except
/// [`Self::maps_with_teleports`] is scoped to the maps that carry at least
/// one teleport line, so a corpus of teleport-free maps reports zeros rather
/// than diluting every ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct TeleportAggregate {
    /// Maps carrying at least one teleport line.
    pub maps_with_teleports: u64,
    /// Maps carrying at least one refused line — the maps the teleport axis
    /// alone makes inexpressible.
    pub maps_refused: u64,
    /// Field-wise sum of every such map's counts.
    pub lines: TeleportCounts,
    /// Maps with at least one closet line.
    pub closet_maps: u64,
    /// Maps with at least one line delivering beside an exit.
    pub exit_maps: u64,
    /// Maps with at least one line on a paired pad.
    pub paired_maps: u64,
}

/// The plat recognizer's roll-up over a sweep. Every count except
/// [`Self::maps_with_lifts`] is scoped to the maps that carry at least one
/// lift line, for the reason [`TeleportAggregate`] gives: a corpus of
/// lift-free maps reports zeros rather than diluting every ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct LiftAggregate {
    /// Maps carrying at least one lift line — one that resolved to a
    /// platform, or one that named none and is counted broken.
    pub maps_with_lifts: u64,
    /// Maps carrying at least one refused platform or broken lift line —
    /// the maps the lift axis alone makes inexpressible.
    pub maps_refused: u64,
    /// Field-wise sum of every such map's counts.
    pub plats: PlatCounts,
    /// Maps with at least one platform accepted as a lift.
    pub lift_maps: u64,
    /// Maps with at least one platform accepted as a pedestal.
    pub pedestal_maps: u64,
    /// Maps with at least one platform accepted as a barrier.
    pub barrier_maps: u64,
}

/// The floor recognizer's roll-up over a sweep. Every count except
/// [`Self::maps_with_floors`] is scoped to the maps that carry at least one
/// floor line, for the reason [`TeleportAggregate`] gives: a corpus of
/// floor-free maps reports zeros rather than diluting every ratio.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct FloorAggregate {
    /// Maps carrying at least one floor line — one that resolved to a
    /// target, or one that named none and is counted broken.
    pub maps_with_floors: u64,
    /// Maps carrying at least one refused target or broken floor line — the
    /// maps the floor axis alone makes inexpressible.
    pub maps_refused: u64,
    /// Field-wise sum of every such map's counts.
    pub targets: FloorCounts,
    /// Maps with at least one target accepted as a drop wall.
    pub drop_wall_maps: u64,
    /// Maps with at least one target accepted as a reveal.
    pub reveal_maps: u64,
    /// Maps with at least one target accepted as a bridge.
    pub bridge_maps: u64,
}

/// The whole-sweep summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Aggregate {
    /// Population size.
    pub maps_unique: u64,
    /// Shares over every unique map.
    pub all: AxisShare,
    /// Fraction of unique maps inside the vanilla special list.
    pub vanilla_share: f64,
    /// Shares over the vanilla-only slice.
    pub vanilla: AxisShare,
    /// Top out-of-set line specials by map share.
    pub line_blockers: Vec<Blocker>,
    /// Top out-of-set sector specials by map share.
    pub sector_blockers: Vec<Blocker>,
    /// Top out-of-set thing types by map share.
    pub thing_blockers: Vec<Blocker>,
    /// Greedy curve with thing kinds and sector specials held expressible.
    pub greedy_line_axis: GreedyCurve,
    /// Greedy curve over maps already ok on the other five axes. Its shares
    /// are still over every unique map, so it plateaus below 1.0 whenever
    /// some map is blocked on a sector special, a thing kind, a refused
    /// teleport line, a refused platform, or a refused floor target.
    pub greedy_conjunction: GreedyCurve,
    /// The teleport recognizer's roll-up.
    pub teleports: TeleportAggregate,
    /// The plat recognizer's roll-up.
    pub lifts: LiftAggregate,
    /// The floor recognizer's roll-up.
    pub floors: FloorAggregate,
}

/// `n / of` as a fraction, with an empty population scoring 0.
#[expect(
    clippy::cast_precision_loss,
    reason = "map counts are always far under f64's 52-bit mantissa"
)]
fn share(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { n as f64 / of as f64 }
}

/// [`share`] over the `u64` counts the aggregate stores.
#[expect(
    clippy::cast_precision_loss,
    reason = "map counts are always far under f64's 52-bit mantissa"
)]
pub(super) fn u64_share(n: u64, of: u64) -> f64 {
    if of == 0 { 0.0 } else { n as f64 / of as f64 }
}

fn axis_share(maps: &[&MapRecord]) -> AxisShare {
    let n = maps.len();
    let count = |f: fn(&Verdict) -> bool| share(maps.iter().filter(|m| f(&m.verdict)).count(), n);
    AxisShare {
        line_specials: count(|v| v.line_specials_ok),
        sector_specials: count(|v| v.sector_specials_ok),
        thing_kinds: count(|v| v.thing_kinds_ok),
        teleports: count(|v| v.teleports_ok),
        lifts: count(|v| v.lifts_ok),
        floors: count(|v| v.floors_ok),
        expressible: count(|v| v.expressible),
    }
}

/// The out-of-set values `pick` reports, ranked by how many maps carry them
/// (ties by ascending value) and cut to [`BLOCKER_TOP`]. Each `unknown_*`
/// list is already deduplicated, so a count is a map count.
///
/// # Panics
///
/// If a count does not fit a `u64`, which needs more maps than a directory
/// of files can hold.
fn blockers(maps: &[MapRecord], pick: fn(&Verdict) -> &[i32]) -> Vec<Blocker> {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for m in maps {
        for &v in pick(&m.verdict) {
            *counts.entry(v).or_insert(0) += 1;
        }
    }
    let mut out: Vec<Blocker> = counts
        .into_iter()
        .map(|(value, n)| Blocker {
            value,
            maps: u64::try_from(n).expect("fits u64"),
            share: share(n, maps.len()),
        })
        .collect();
    out.sort_by(|a, b| b.maps.cmp(&a.maps).then(a.value.cmp(&b.value)));
    out.truncate(BLOCKER_TOP);
    out
}

/// Greedy set cover over `population`: each step adds the out-of-set
/// special that turns the most still-blocked maps expressible on the line
/// axis; ties go to the smaller special number. `population` is the set of
/// maps eligible to be unblocked; `total` is the denominator.
fn greedy(population: &[&MapRecord], total: usize) -> GreedyCurve {
    let mut remaining: Vec<BTreeSet<i32>> = population
        .iter()
        .map(|m| m.verdict.unknown_line_specials.iter().copied().collect())
        .filter(|s: &BTreeSet<i32>| !s.is_empty())
        .collect();
    let mut unblocked = population.len() - remaining.len();
    let mut steps = Vec::new();
    while !remaining.is_empty() {
        let mut gain: BTreeMap<i32, usize> = BTreeMap::new();
        for set in &remaining {
            if set.len() == 1 {
                *gain
                    .entry(*set.iter().next().expect("non-empty"))
                    .or_insert(0) += 1;
            }
        }
        // A special that alone completes no map still makes progress; when
        // no single special completes any map, add the one carried by the
        // most maps.
        let best = if gain.is_empty() {
            let mut carried: BTreeMap<i32, usize> = BTreeMap::new();
            for set in &remaining {
                for &s in set {
                    *carried.entry(s).or_insert(0) += 1;
                }
            }
            carried
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(s, _)| s)
        } else {
            gain.into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(s, _)| s)
        };
        // Unreachable: the `while` guard keeps `remaining` non-empty, and
        // `retain` above drops every empty set, so at least one non-empty
        // set survives into `carried` (or `gain`) each iteration — `best`
        // is always `Some`. Kept as a defensive exit rather than an
        // `expect`, since a future change to either invariant should fail
        // safely here, not panic.
        let Some(best) = best else { break };
        for set in &mut remaining {
            set.remove(&best);
        }
        let before = remaining.len();
        remaining.retain(|s| !s.is_empty());
        unblocked += before - remaining.len();
        steps.push(GreedyStep {
            k: steps.len() + 1,
            special: best,
            cumulative_share: share(unblocked, total),
        });
    }
    let checkpoints = GREEDY_CHECKPOINTS
        .iter()
        .filter_map(|&k| {
            steps
                .get(k.min(steps.len()).checked_sub(1)?)
                .map(|s| (k, s.cumulative_share))
        })
        .collect();
    GreedyCurve { steps, checkpoints }
}

/// Rolls the per-map teleport counts up over the maps that carry a teleport
/// line. Maps with none are excluded outright rather than summed as zeros:
/// every "N lines, M maps" pair in the report is then a statement about the
/// teleporting slice of the corpus.
///
/// # Panics
///
/// If a map count does not fit a `u64`, which needs more maps than a
/// directory of files can hold.
fn teleport_aggregate(maps: &[MapRecord]) -> TeleportAggregate {
    let with: Vec<&MapRecord> = maps.iter().filter(|m| m.teleports.lines > 0).collect();
    let mut lines = TeleportCounts::default();
    for m in &with {
        lines = lines.add(&m.teleports);
    }
    let count = |f: fn(&TeleportCounts) -> bool| {
        u64::try_from(with.iter().filter(|m| f(&m.teleports)).count()).expect("fits u64")
    };
    TeleportAggregate {
        maps_with_teleports: u64::try_from(with.len()).expect("fits u64"),
        maps_refused: count(|c| c.refusals() > 0),
        lines,
        closet_maps: count(|c| c.closet > 0),
        exit_maps: count(|c| c.exit > 0),
        paired_maps: count(|c| c.paired > 0),
    }
}

/// Rolls the per-map plat counts up over the maps that carry a lift line.
/// Maps with none are excluded outright rather than summed as zeros, for the
/// reason [`teleport_aggregate`] gives.
///
/// "Carries a lift line" is `plats + broken_lines > 0`: every lift line
/// either names at least one platform or is counted broken, so the pair
/// covers the whole population the recognizer ran on.
///
/// # Panics
///
/// If a map count does not fit a `u64`, which needs more maps than a
/// directory of files can hold.
fn lift_aggregate(maps: &[MapRecord]) -> LiftAggregate {
    let with: Vec<&MapRecord> = maps
        .iter()
        .filter(|m| m.lifts.plats > 0 || m.lifts.broken_lines > 0)
        .collect();
    let mut plats = PlatCounts::default();
    for m in &with {
        plats = plats.add(&m.lifts);
    }
    let count = |f: fn(&PlatCounts) -> bool| {
        u64::try_from(with.iter().filter(|m| f(&m.lifts)).count()).expect("fits u64")
    };
    LiftAggregate {
        maps_with_lifts: u64::try_from(with.len()).expect("fits u64"),
        maps_refused: count(|c| c.refusals() > 0),
        plats,
        lift_maps: count(|c| c.lifts > 0),
        pedestal_maps: count(|c| c.pedestals > 0),
        barrier_maps: count(|c| c.barriers > 0),
    }
}

/// Rolls the per-map floor counts up over the maps that carry a floor line.
/// Maps with none are excluded outright rather than summed as zeros, for the
/// reason [`teleport_aggregate`] gives.
///
/// "Carries a floor line" is `targets + broken_lines > 0`: every floor line
/// either names at least one target or is counted broken, so the pair covers
/// the whole population the recognizer ran on.
///
/// # Panics
///
/// If a map count does not fit a `u64`, which needs more maps than a
/// directory of files can hold.
fn floor_aggregate(maps: &[MapRecord]) -> FloorAggregate {
    let with: Vec<&MapRecord> = maps
        .iter()
        .filter(|m| m.floors.targets > 0 || m.floors.broken_lines > 0)
        .collect();
    let mut targets = FloorCounts::default();
    for m in &with {
        targets = targets.add(&m.floors);
    }
    let count = |f: fn(&FloorCounts) -> bool| {
        u64::try_from(with.iter().filter(|m| f(&m.floors)).count()).expect("fits u64")
    };
    FloorAggregate {
        maps_with_floors: u64::try_from(with.len()).expect("fits u64"),
        maps_refused: count(|c| c.refusals() > 0),
        targets,
        drop_wall_maps: count(|c| c.drop_walls > 0),
        reveal_maps: count(|c| c.reveals > 0),
        bridge_maps: count(|c| c.bridges > 0),
    }
}

/// Summarizes a sweep's unique maps.
///
/// # Panics
///
/// If the sweep holds more than `u64::MAX` maps, which no directory of
/// files can produce.
#[must_use]
pub fn aggregate(maps: &[MapRecord]) -> Aggregate {
    let all: Vec<&MapRecord> = maps.iter().collect();
    let vanilla: Vec<&MapRecord> = maps.iter().filter(|m| m.verdict.vanilla_only).collect();
    // The three recognizer axes belong in this filter, not just in the
    // shares: no number of added line specials can un-refuse a teleport
    // line, a platform, or a floor target, so a map blocked on any of them
    // that this curve "unblocked" would be counted expressible when it is
    // not.
    let conjunction_pop: Vec<&MapRecord> = maps
        .iter()
        .filter(|m| {
            m.verdict.sector_specials_ok
                && m.verdict.thing_kinds_ok
                && m.verdict.teleports_ok
                && m.verdict.lifts_ok
                && m.verdict.floors_ok
        })
        .collect();
    Aggregate {
        maps_unique: u64::try_from(maps.len()).expect("fits u64"),
        all: axis_share(&all),
        vanilla_share: share(vanilla.len(), maps.len()),
        vanilla: axis_share(&vanilla),
        line_blockers: blockers(maps, |v| v.unknown_line_specials.as_slice()),
        sector_blockers: blockers(maps, |v| v.unknown_sector_specials.as_slice()),
        thing_blockers: blockers(maps, |v| v.unknown_thing_types.as_slice()),
        greedy_line_axis: greedy(&all, maps.len()),
        greedy_conjunction: greedy(&conjunction_pop, maps.len()),
        teleports: teleport_aggregate(maps),
        lifts: lift_aggregate(maps),
        floors: floor_aggregate(maps),
    }
}

#[cfg(test)]
mod tests {
    use super::super::fixtures::record;
    use super::*;

    #[test]
    fn aggregate_shares_blockers_and_greedy_curve() {
        let maps = vec![
            record(&[1], &[], true, true, true),             // expressible
            record(&[1, 97], &[97], true, true, true),       // blocked by 97
            record(&[97, 62], &[97, 62], true, false, true), // blocked by 97, 62, and a thing
            record(&[8192], &[8192], false, true, false),    // boom, sector-blocked
        ];
        let a = aggregate(&maps);
        assert_eq!(a.maps_unique, 4);
        assert!((a.all.line_specials - 0.25).abs() < 1e-9);
        assert!((a.all.thing_kinds - 0.75).abs() < 1e-9);
        assert!((a.all.expressible - 0.25).abs() < 1e-9);
        assert!((a.vanilla_share - 0.75).abs() < 1e-9);
        assert!((a.vanilla.line_specials - (1.0 / 3.0)).abs() < 1e-9);

        assert_eq!(a.line_blockers[0].value, 97);
        assert_eq!(a.line_blockers[0].maps, 2);
        assert!((a.line_blockers[0].share - 0.5).abs() < 1e-9);
        assert_eq!(a.thing_blockers[0].value, 46);
        assert_eq!(a.sector_blockers[0].value, 4);

        // Line axis alone: +97 unblocks map 2 (and map 3 needs 62 too).
        let g = &a.greedy_line_axis;
        assert_eq!(g.steps[0].special, 97);
        assert!((g.steps[0].cumulative_share - 0.5).abs() < 1e-9);
        assert_eq!(g.steps[1].special, 62);
        assert!((g.steps[1].cumulative_share - 0.75).abs() < 1e-9);
        assert_eq!(g.steps[2].special, 8192);
        assert_eq!(g.checkpoints[0], (1, 0.5));
        // Conjunction: map 3 stays blocked by its thing; map 4 by its sector.
        let c = &a.greedy_conjunction;
        assert_eq!(c.steps[0].special, 97);
        assert!((c.steps[0].cumulative_share - 0.5).abs() < 1e-9);
        assert!(c.steps.len() <= 3);
    }

    /// The fallback branch: with every remaining map blocked by two
    /// specials, no single special completes a map, so step 1 adds the one
    /// carried by the most maps and unblocks nothing (share 0.0) — the
    /// signature of the fallback having fired.
    #[test]
    fn greedy_falls_back_to_the_most_carried_special_when_no_singleton_completes_a_map() {
        let maps = vec![
            record(&[1, 2], &[1, 2], true, true, true),
            record(&[1, 3], &[1, 3], true, true, true),
        ];
        let g = aggregate(&maps).greedy_line_axis;
        assert_eq!(g.steps.len(), 3);
        assert_eq!(
            g.steps[0].special, 1,
            "carried by both maps, completes neither"
        );
        assert!(
            g.steps[0].cumulative_share.abs() < 1e-9,
            "the fallback step unblocks no map"
        );
        assert_eq!(g.steps[1].special, 2, "singletons tie, the smaller wins");
        assert!((g.steps[1].cumulative_share - 0.5).abs() < 1e-9);
        assert_eq!(g.steps[2].special, 3);
        assert!((g.steps[2].cumulative_share - 1.0).abs() < 1e-9);
        assert_eq!(
            g.checkpoints.len(),
            5,
            "every checkpoint is reported, clamped to the last step"
        );
        assert_eq!(
            g.checkpoints[1],
            (5, 1.0),
            "a checkpoint past the last step repeats its share"
        );
    }

    /// Ranking is by map count, then by ascending value: 62 and 8192 are each
    /// carried by one map, so 62 comes first.
    #[test]
    fn blockers_rank_by_map_count_then_by_ascending_value() {
        let maps = vec![
            record(&[1], &[], true, true, true),
            record(&[1, 97], &[97], true, true, true),
            record(&[97, 62], &[97, 62], true, false, true),
            record(&[8192], &[8192], false, true, false),
        ];
        let a = aggregate(&maps);
        let values: Vec<i32> = a.line_blockers.iter().map(|b| b.value).collect();
        assert_eq!(values, vec![97, 62, 8192]);
    }

    /// 26 distinct specials carried once each plus one carried twice is 27
    /// values; the table keeps `BLOCKER_TOP` of them, the doubled one first.
    #[test]
    fn blockers_are_cut_to_blocker_top() {
        let mut maps: Vec<MapRecord> = (1000..1026)
            .map(|s| record(&[s], &[s], true, true, true))
            .collect();
        maps.push(record(&[5000], &[5000], true, true, true));
        maps.push(record(&[5000], &[5000], true, true, true));
        let a = aggregate(&maps);
        assert_eq!(a.line_blockers.len(), BLOCKER_TOP);
        assert_eq!(a.line_blockers[0].value, 5000);
        assert_eq!(a.line_blockers[0].maps, 2);
        assert_eq!(
            a.line_blockers[1].value, 1000,
            "then ascending among the singletons"
        );
    }

    #[test]
    #[allow(
        clippy::float_cmp,
        reason = "an empty population must score exactly zero, not merely near-zero — the \
                  approximate form the sibling test uses would pass on a share this test \
                  exists to rule out"
    )]
    fn aggregate_of_nothing_is_all_zero() {
        let a = aggregate(&[]);
        assert_eq!(a.maps_unique, 0);
        assert_eq!(a.vanilla_share, 0.0);
        for share in [a.all, a.vanilla] {
            assert_eq!(share.line_specials, 0.0);
            assert_eq!(share.sector_specials, 0.0);
            assert_eq!(share.thing_kinds, 0.0);
            assert_eq!(share.teleports, 0.0);
            assert_eq!(share.lifts, 0.0);
            assert_eq!(share.floors, 0.0);
            assert_eq!(share.expressible, 0.0);
        }
        assert!(a.line_blockers.is_empty(), "{:?}", a.line_blockers);
        assert!(a.sector_blockers.is_empty(), "{:?}", a.sector_blockers);
        assert!(a.thing_blockers.is_empty(), "{:?}", a.thing_blockers);
        assert!(
            a.greedy_line_axis.steps.is_empty(),
            "{:?}",
            a.greedy_line_axis.steps
        );
        assert!(
            a.greedy_line_axis.checkpoints.is_empty(),
            "{:?}",
            a.greedy_line_axis.checkpoints
        );
        assert!(
            a.greedy_conjunction.steps.is_empty(),
            "{:?}",
            a.greedy_conjunction.steps
        );
        assert!(
            a.greedy_conjunction.checkpoints.is_empty(),
            "{:?}",
            a.greedy_conjunction.checkpoints
        );
        assert_eq!(a.teleports, TeleportAggregate::default());
        assert_eq!(a.lifts, LiftAggregate::default());
        assert_eq!(a.floors, FloorAggregate::default());
    }

    /// A teleport-refused map is out of the conjunction curve's population:
    /// no number of added line specials can un-refuse a teleport line, so
    /// counting the map as unblocked would credit the curve with an
    /// expressibility it does not buy.
    #[test]
    fn the_conjunction_curve_excludes_a_teleport_refused_map() {
        let mut blocked = record(&[97], &[97], true, true, true);
        blocked.teleports = TeleportCounts {
            lines: 1,
            broken: 1,
            ..TeleportCounts::default()
        };
        blocked.verdict.teleports_ok = false;
        blocked.verdict.expressible = false;
        let a = aggregate(&[blocked, record(&[1], &[], true, true, true)]);
        assert_eq!(
            a.greedy_line_axis.steps[0].special, 97,
            "the line axis alone still walks the blocked map"
        );
        assert!(
            a.greedy_conjunction.steps.is_empty(),
            "the only line-blocked map is teleport-refused, so nothing is left to unblock"
        );
        assert!((a.all.expressible - 0.5).abs() < 1e-9);
    }

    /// A lift-refused map is out of the conjunction curve's population, for
    /// the reason a teleport-refused one is: no number of added line
    /// specials can un-refuse a platform.
    #[test]
    fn the_conjunction_curve_excludes_a_lift_refused_map() {
        let mut blocked = record(&[97], &[97], true, true, true);
        blocked.lifts = PlatCounts {
            plats: 1,
            refused: 1,
            dead: 1,
            ..PlatCounts::default()
        };
        blocked.verdict.lifts_ok = false;
        blocked.verdict.expressible = false;
        let a = aggregate(&[blocked, record(&[1], &[], true, true, true)]);
        assert_eq!(
            a.greedy_line_axis.steps[0].special, 97,
            "the line axis alone still walks the blocked map"
        );
        assert!(
            a.greedy_conjunction.steps.is_empty(),
            "the only line-blocked map is lift-refused, so nothing is left to unblock"
        );
        assert!((a.all.expressible - 0.5).abs() < 1e-9);
    }

    /// The conjunction curve's floor term, the sibling of
    /// [`the_conjunction_curve_excludes_a_lift_refused_map`]: no number of
    /// added line specials can un-refuse a floor target either, so a
    /// floor-blocked map must be filtered out of the curve's population
    /// rather than counted as one the curve could unblock.
    #[test]
    fn the_conjunction_curve_excludes_a_floor_refused_map() {
        let mut blocked = record(&[97], &[97], true, true, true);
        blocked.floors = FloorCounts {
            targets: 1,
            refused: 1,
            dead: 1,
            ..FloorCounts::default()
        };
        blocked.verdict.floors_ok = false;
        blocked.verdict.expressible = false;
        let a = aggregate(&[blocked, record(&[1], &[], true, true, true)]);
        assert_eq!(
            a.greedy_line_axis.steps[0].special, 97,
            "the line axis alone still walks the blocked map"
        );
        assert!(
            a.greedy_conjunction.steps.is_empty(),
            "the only line-blocked map is floor-refused, so nothing is left to unblock"
        );
        assert!((a.all.expressible - 0.5).abs() < 1e-9);
    }
}
