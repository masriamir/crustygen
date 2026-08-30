//! The plat recognizer: classifies every platform a lift line names into the
//! three shapes the IR can state — a lift, a pedestal, a barrier — or
//! refuses it with the reason it cannot be stated.
//!
//! Recognition, not approximation. Every engine-side reading here is
//! [`crate::check::plats`]'s, resolved once for the flood, the rules, the
//! conformance rows and this module: the floor `EV_DoPlat` sends a platform
//! to and how far it travels ([`ScenePlat::low`], [`ScenePlat::travel`]),
//! where the platform rests relative to its two-sided neighbors ([`Rest`]),
//! and which sector fires each lift line, classified as an [`Activator`] of
//! the platform. This module reads no geometry of its own; it judges that
//! resolution against what the IR can carry.
//!
//! **Shape.** An unrefused platform is one of three. A [`Rest::Top`] platform
//! is a [`Shape::Lift`]: it rests level with a landing and drops to a low
//! room. A [`Rest::AboveAll`] platform whose neighbors all sit at one floor
//! is a [`Shape::Pedestal`] when that is a single host and a
//! [`Shape::Barrier`] when there are two or more. Those are the lift shape
//! probe's rules (`examples/liftprobe/common.rs`,
//! `docs/measurements/lift-shapes-2026-08-29.md`), which measured them across
//! the idgames corpus; the probe folds every other platform into one `Other`
//! bucket, where this module names the reason instead.
//!
//! **Refusal precedence.** A platform is judged against the eight
//! [`Refusal`]s in one fixed order and the first that applies wins, so a
//! platform wrong in more than one way reports its most fundamental reason
//! rather than an order-of-evaluation accident:
//!
//! 1. [`Refusal::Dead`] — it cannot move at all, so nothing else about it
//!    matters.
//! 2. [`Refusal::SharedTag`] — it is not one platform, so per-platform
//!    judgments do not apply to it.
//! 3. [`Refusal::OneShot`] and 4. [`Refusal::MixedSpeed`] — how its triggers
//!    fire, which is a fact about the lines and holds whatever the geometry.
//! 5. [`Refusal::UnsupportedRest`] — where it rests, which decides whether a
//!    shape exists to name at all.
//! 6. [`Refusal::TopOnly`] and 7. [`Refusal::OneWayBarrier`] — who can call
//!    it, judged only once its rest is one a shape names.
//! 8. [`Refusal::ConflictingAction`] — last, because a platform this module
//!    would otherwise accept is the only one for which "something else drives
//!    it too" is the whole story.
//!
//! One consequence worth naming: a member of a shared-tag group that cannot
//! move reports [`Refusal::Dead`] rather than [`Refusal::SharedTag`], because
//! the order judges movement before identity — the members of one group need
//! not all carry the same refusal.
//!
//! A lift line that names no platform at all — tag 0, or a tag no sector
//! answers to — is not a refused platform but a broken line, listed in
//! [`PlatReport::broken_lines`] as [`broken_lift_lines`] reports it and
//! counted by [`PlatCounts::refusals`] alongside the refused platforms.

use std::collections::BTreeSet;

use crate::check::plats::{Activator, ScenePlat, broken_lift_lines, resolve_plats};
use crate::check::scene::Scene;
use crate::tables::Tables;

pub use crate::check::plats::Rest;

/// What an accepted platform joins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A platform at rest level with a landing ([`Rest::Top`]) that drops to
    /// a low room and returns — the corpus's dominant shape.
    Lift,
    /// A platform at rest above its one neighbor: a raised island inside a
    /// single host room.
    Pedestal,
    /// A platform at rest above two or more neighbors that all sit at one
    /// floor: a wall between rooms that lowers to let the player across.
    Barrier,
}

/// How fast a platform's triggers drive it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Speed {
    /// No trigger is one of the `blazeDWUS` forms
    /// ([`Tables::lift_fast_specials`]).
    Normal,
    /// Every trigger is.
    Fast,
    /// Some are and some are not — one platform driven at two speeds.
    Mixed,
}

/// Why a platform cannot be expressed as a map-spec lift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// [`Rest::Dead`]: `P_FindLowestFloorSurrounding` returns the platform's
    /// own floor, so it travels 0 and `EV_DoPlat`'s `downWaitUpStay` is a
    /// no-op on it — there is no movement to state.
    Dead,
    /// More than one sector carries the tag ([`ScenePlat::shared_tag`]), so
    /// every line naming it drives them all; one IR lift is one platform.
    SharedTag,
    /// Some trigger is a one-shot (S1/W1) form rather than a repeatable
    /// (SR/WR) one ([`SceneTrigger::repeatable`]) — a platform the player can
    /// call at most once, which no IR lift states.
    ///
    /// [`SceneTrigger::repeatable`]: crate::check::plats::SceneTrigger::repeatable
    OneShot,
    /// [`Speed::Mixed`]: the IR carries one speed per lift.
    MixedSpeed,
    /// The platform rests where no shape names it: [`Rest::Intermediate`]
    /// (some neighbor more than a step above it), or [`Rest::AboveAll`] with
    /// its neighbors at more than one floor
    /// ([`ScenePlat::distinct_neighbor_floors`]).
    UnsupportedRest,
    /// No trigger fires from more than a step below the platform
    /// ([`ScenePlat::callable_low`]) — the caller a lift exists for. Only a
    /// rider standing on it, or beside it, can send it down.
    TopOnly,
    /// A [`Rest::AboveAll`] platform with two or more neighbors, one of which
    /// fires no trigger from below ([`ScenePlat::low_activator_neighbors`]):
    /// it lowers for one side only, where an IR barrier lowers for both.
    OneWayBarrier,
    /// A non-lift special names the platform's tag too
    /// ([`ScenePlat::other_actions`]), so something besides the lift drives
    /// this sector.
    ConflictingAction,
}

/// Which classes of sector fire some trigger of a platform, by the
/// [`Activator`] class of the firing sector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "the four fields are the four `check::plats::Activator` classes, each an \
              independent fact about who fires this platform's triggers — a set of four \
              flags, with no state they jointly encode"
)]
pub struct Callable {
    /// Some trigger fires from more than a step below the platform
    /// ([`Activator::Low`]).
    pub low: bool,
    /// Some trigger fires from within a step of it, the platform itself
    /// excepted ([`Activator::Level`]).
    pub level: bool,
    /// Some trigger fires from the platform itself ([`Activator::Plat`]).
    pub plat: bool,
    /// Some trigger fires from more than a step above it
    /// ([`Activator::Above`]).
    pub above: bool,
}

/// One recognized platform.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Plat {
    /// Declaration index of the platform sector.
    pub sector: usize,
    /// The tag its lift lines name it by.
    pub tag: i32,
    /// Where it rests relative to its two-sided neighbors.
    pub rest: Rest,
    /// Its rest floor minus the floor `EV_DoPlat` sends it to.
    pub travel: i32,
    /// What it is, for a platform the recognizer accepts; `None` for one it
    /// refuses.
    pub shape: Option<Shape>,
    /// How fast its triggers drive it.
    pub speed: Speed,
    /// Which classes of sector fire its triggers.
    pub callable: Callable,
    /// Declaration indices of every lift line naming its tag.
    pub triggers: Vec<usize>,
    /// How many things stand in the platform sector.
    pub things: u64,
    /// Whether every edge of the platform sector is two-sided — a
    /// free-standing pad. Reported as a statistic, never a gate.
    pub island: bool,
    /// Whether some trigger fires from the platform itself or from within a
    /// step of it ([`ScenePlat::callable_top`]) — a rider's call.
    pub top_trigger: bool,
    /// Why it is not expressible, if it is not.
    pub refusal: Option<Refusal>,
}

/// One count per predicate over a map's [`Plat`]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct PlatCounts {
    /// Platforms recognized.
    pub plats: u64,
    /// Platforms accepted as [`Shape::Lift`].
    pub lifts: u64,
    /// Platforms accepted as [`Shape::Pedestal`].
    pub pedestals: u64,
    /// Platforms accepted as [`Shape::Barrier`].
    pub barriers: u64,
    /// Platforms carrying any [`Refusal`].
    pub refused: u64,
    /// Platforms refused [`Refusal::Dead`].
    pub dead: u64,
    /// Platforms refused [`Refusal::SharedTag`].
    pub shared_tag: u64,
    /// Shared-tag groups that are one platform split by trim: every member at
    /// one floor and all of them mutually adjacent. A sub-count of
    /// [`Self::shared_tag`]'s groups, not of its platforms — the shape a
    /// geometry-aware lifter could still recognize as a single lift.
    pub shared_split: u64,
    /// Platforms refused [`Refusal::OneShot`].
    pub one_shot: u64,
    /// Platforms refused [`Refusal::MixedSpeed`].
    pub mixed_speed: u64,
    /// Platforms refused [`Refusal::UnsupportedRest`].
    pub unsupported_rest: u64,
    /// Platforms refused [`Refusal::TopOnly`].
    pub top_only: u64,
    /// Platforms refused [`Refusal::OneWayBarrier`].
    pub one_way_barrier: u64,
    /// Platforms refused [`Refusal::ConflictingAction`].
    pub conflicting: u64,
    /// Lift lines that name no platform at all
    /// ([`PlatReport::broken_lines`]).
    pub broken_lines: u64,
    /// Platforms some trigger fires at from more than a step below.
    pub callable_low: u64,
    /// Platforms with a [`Plat::top_trigger`].
    pub with_top_trigger: u64,
    /// Platforms holding at least one thing.
    pub with_things: u64,
    /// Platforms driven at [`Speed::Fast`].
    pub fast: u64,
}

impl PlatCounts {
    /// Everything the recognizer refused: the refused platforms plus the lift
    /// lines that name no platform at all.
    #[must_use]
    pub fn refusals(&self) -> u64 {
        self.refused + self.broken_lines
    }

    /// Field-wise sum, for rolling per-map counts into a corpus total.
    ///
    /// Saturating rather than wrapping, for the reason
    /// [`crate::lift::teleport::TeleportCounts::add`] gives: a corpus large
    /// enough to overflow a `u64` platform count cannot exist, and a pinned
    /// ceiling is a better report than a wrapped one if that ever stops being
    /// true.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        Self {
            plats: self.plats.saturating_add(other.plats),
            lifts: self.lifts.saturating_add(other.lifts),
            pedestals: self.pedestals.saturating_add(other.pedestals),
            barriers: self.barriers.saturating_add(other.barriers),
            refused: self.refused.saturating_add(other.refused),
            dead: self.dead.saturating_add(other.dead),
            shared_tag: self.shared_tag.saturating_add(other.shared_tag),
            shared_split: self.shared_split.saturating_add(other.shared_split),
            one_shot: self.one_shot.saturating_add(other.one_shot),
            mixed_speed: self.mixed_speed.saturating_add(other.mixed_speed),
            unsupported_rest: self.unsupported_rest.saturating_add(other.unsupported_rest),
            top_only: self.top_only.saturating_add(other.top_only),
            one_way_barrier: self.one_way_barrier.saturating_add(other.one_way_barrier),
            conflicting: self.conflicting.saturating_add(other.conflicting),
            broken_lines: self.broken_lines.saturating_add(other.broken_lines),
            callable_low: self.callable_low.saturating_add(other.callable_low),
            with_top_trigger: self.with_top_trigger.saturating_add(other.with_top_trigger),
            with_things: self.with_things.saturating_add(other.with_things),
            fast: self.fast.saturating_add(other.fast),
        }
    }
}

/// What [`recognize`] says about one map's platforms.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct PlatReport {
    /// One entry per platform some lift line names, ascending by sector.
    pub plats: Vec<Plat>,
    /// Lift lines that name no platform at all, ascending by declaration
    /// index ([`broken_lift_lines`]).
    pub broken_lines: Vec<usize>,
    /// The census of [`Self::plats`] and [`Self::broken_lines`].
    pub counts: PlatCounts,
}

/// Classifies every platform a lift line names in `scene`.
///
/// # Panics
///
/// If a platform's thing count or the shared-tag-group tally does not fit a
/// `u64`, which needs more elements than a parsed map can hold (`usize` is at
/// most 64 bits on every platform this crate builds on).
#[must_use]
pub fn recognize(scene: &Scene, tables: &Tables) -> PlatReport {
    let resolved = resolve_plats(scene, tables);
    let split_groups = split_group_tags(scene, &resolved);
    let mut plats = Vec::with_capacity(resolved.len());
    for p in &resolved {
        let speed = speed_of(p);
        let callable = callable_of(p);
        let refusal = refusal_of(p, callable, speed);
        plats.push(Plat {
            sector: p.sector,
            tag: p.tag,
            rest: p.rest,
            travel: p.travel,
            shape: shape_of(refusal, p),
            speed,
            callable,
            triggers: p.triggers.iter().map(|t| t.linedef).collect(),
            things: u64::try_from(
                scene
                    .things
                    .iter()
                    .filter(|t| t.sector == Some(p.sector))
                    .count(),
            )
            .expect("fits u64"),
            island: scene.sectors[p.sector].boundary.iter().all(|b| b.two_sided),
            top_trigger: p.callable_top(),
            refusal,
        });
    }
    let broken_lines = broken_lift_lines(scene, tables);
    let counts = count(
        &plats,
        &broken_lines,
        u64::try_from(split_groups.len()).expect("fits u64"),
    );
    PlatReport {
        plats,
        broken_lines,
        counts,
    }
}

/// The tags of the shared-tag groups that are one platform split by trim:
/// every member sitting at one floor, and all of them mutually adjacent.
///
/// Adjacency is a walk over two-sided boundaries that never leaves the group:
/// the group is one platform's worth of geometry exactly when that walk,
/// started anywhere in it, reaches every member.
fn split_group_tags(scene: &Scene, resolved: &[ScenePlat]) -> BTreeSet<i32> {
    resolved
        .iter()
        .filter(|p| p.shared_tag >= 2)
        .map(|p| p.tag)
        .collect::<BTreeSet<i32>>()
        .into_iter()
        .filter(|&tag| is_split_group(scene, resolved, tag))
        .collect()
}

/// Whether the platforms carrying `tag` are one platform split by trim.
fn is_split_group(scene: &Scene, resolved: &[ScenePlat], tag: i32) -> bool {
    let members: BTreeSet<usize> = resolved
        .iter()
        .filter(|p| p.tag == tag)
        .map(|p| p.sector)
        .collect();
    let Some(&seed) = members.iter().next() else {
        return false;
    };
    let floors: BTreeSet<i32> = members.iter().map(|&s| scene.sectors[s].floor).collect();
    if floors.len() != 1 {
        return false;
    }
    let mut reached: BTreeSet<usize> = BTreeSet::new();
    let mut stack = vec![seed];
    while let Some(s) = stack.pop() {
        if !reached.insert(s) {
            continue;
        }
        stack.extend(
            scene.sectors[s]
                .boundary
                .iter()
                .filter_map(|b| b.neighbor)
                .filter(|n| members.contains(n) && !reached.contains(n)),
        );
    }
    reached.len() == members.len()
}

/// [`Speed::Fast`] when every trigger is a `blazeDWUS` form,
/// [`Speed::Normal`] when none is, [`Speed::Mixed`] in between.
fn speed_of(p: &ScenePlat) -> Speed {
    let fasts = p.triggers.iter().filter(|t| t.fast).count();
    if fasts == 0 {
        Speed::Normal
    } else if fasts == p.triggers.len() {
        Speed::Fast
    } else {
        Speed::Mixed
    }
}

/// Which [`Activator`] classes fire some trigger of `p`.
fn callable_of(p: &ScenePlat) -> Callable {
    let any = |want: Activator| {
        p.triggers
            .iter()
            .any(|t| t.activators.iter().any(|&(_, a)| a == want))
    };
    Callable {
        low: p.callable_low(),
        level: any(Activator::Level),
        plat: any(Activator::Plat),
        above: any(Activator::Above),
    }
}

/// The first [`Refusal`] that applies to `p`, in the module doc's order.
fn refusal_of(p: &ScenePlat, callable: Callable, speed: Speed) -> Option<Refusal> {
    let one_floor = p.distinct_neighbor_floors == 1;
    if p.rest == Rest::Dead {
        Some(Refusal::Dead)
    } else if p.shared_tag >= 2 {
        Some(Refusal::SharedTag)
    } else if !p.triggers.iter().all(|t| t.repeatable) {
        Some(Refusal::OneShot)
    } else if speed == Speed::Mixed {
        Some(Refusal::MixedSpeed)
    } else if p.rest == Rest::Intermediate || (p.rest == Rest::AboveAll && !one_floor) {
        Some(Refusal::UnsupportedRest)
    } else if !callable.low {
        Some(Refusal::TopOnly)
    } else if p.rest == Rest::AboveAll
        && p.neighbors.len() >= 2
        && p.low_activator_neighbors().len() < p.neighbors.len()
    {
        Some(Refusal::OneWayBarrier)
    } else if !p.other_actions.is_empty() {
        Some(Refusal::ConflictingAction)
    } else {
        None
    }
}

/// The [`Shape`] of an unrefused platform: [`Rest::Top`] is a lift, and
/// [`Rest::AboveAll`] — already known to have its neighbors at one floor,
/// or [`Refusal::UnsupportedRest`] would have caught it — is a pedestal
/// against one neighbor and a barrier against more.
fn shape_of(refusal: Option<Refusal>, p: &ScenePlat) -> Option<Shape> {
    match (refusal, p.rest, p.neighbors.len()) {
        (None, Rest::Top, _) => Some(Shape::Lift),
        (None, Rest::AboveAll, 1) => Some(Shape::Pedestal),
        (None, Rest::AboveAll, _) => Some(Shape::Barrier),
        _ => None,
    }
}

/// Counts `plats` one field per predicate, `shared_split` being
/// [`split_group_tags`]'s tally.
fn count(plats: &[Plat], broken_lines: &[usize], shared_split: u64) -> PlatCounts {
    let mut c = PlatCounts {
        shared_split,
        broken_lines: u64::try_from(broken_lines.len()).expect("fits u64"),
        ..PlatCounts::default()
    };
    for p in plats {
        c.plats += 1;
        match p.shape {
            Some(Shape::Lift) => c.lifts += 1,
            Some(Shape::Pedestal) => c.pedestals += 1,
            Some(Shape::Barrier) => c.barriers += 1,
            None => {}
        }
        match p.refusal {
            Some(Refusal::Dead) => c.dead += 1,
            Some(Refusal::SharedTag) => c.shared_tag += 1,
            Some(Refusal::OneShot) => c.one_shot += 1,
            Some(Refusal::MixedSpeed) => c.mixed_speed += 1,
            Some(Refusal::UnsupportedRest) => c.unsupported_rest += 1,
            Some(Refusal::TopOnly) => c.top_only += 1,
            Some(Refusal::OneWayBarrier) => c.one_way_barrier += 1,
            Some(Refusal::ConflictingAction) => c.conflicting += 1,
            None => {}
        }
        c.refused += u64::from(p.refusal.is_some());
        c.callable_low += u64::from(p.callable.low);
        c.with_top_trigger += u64::from(p.top_trigger);
        c.with_things += u64::from(p.things > 0);
        c.fast += u64::from(p.speed == Speed::Fast);
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::fixtures::{chain, scene_of};

    /// Parses `text`, builds its [`Scene`] and recognizes its platforms.
    fn report_of(text: &str) -> PlatReport {
        let (scene, tables) = scene_of(text);
        recognize(&scene, &tables)
    }

    #[test]
    fn shapes_are_recognized() {
        let r = report_of(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            (r.plats[0].shape, r.plats[0].refusal, r.plats[0].speed),
            (Some(Shape::Lift), None, Speed::Normal)
        );
        let r = report_of(&chain(&[0, 128], &[0, 7], &[(123, 7, false)], ""));
        assert_eq!(
            (r.plats[0].shape, r.plats[0].speed),
            (Some(Shape::Pedestal), Speed::Fast)
        );
        let r = report_of(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(62, 7, false), (62, 7, true)],
            "",
        ));
        assert_eq!(r.plats[0].shape, Some(Shape::Barrier));
        assert_eq!(
            (r.counts.plats, r.counts.barriers, r.counts.refused),
            (1, 1, 0)
        );
    }

    #[test]
    fn refusals_in_order() {
        let cases: [(String, Refusal); 8] = [
            (
                chain(&[0, 0, 0], &[0, 7, 0], &[(62, 7, false), (0, 0, false)], ""),
                Refusal::Dead,
            ),
            (
                chain(
                    &[0, 128, 128, 0],
                    &[0, 7, 7, 0],
                    &[(62, 7, false), (0, 0, false), (0, 0, false)],
                    "",
                ),
                Refusal::SharedTag,
            ),
            (
                chain(
                    &[0, 128, 128],
                    &[0, 7, 0],
                    &[(21, 7, false), (0, 0, false)],
                    "",
                ),
                Refusal::OneShot,
            ),
            (
                chain(
                    &[0, 128, 128],
                    &[0, 7, 0],
                    &[(62, 7, false), (120, 7, false)],
                    "",
                ),
                Refusal::MixedSpeed,
            ),
            (
                chain(
                    &[0, 64, 160],
                    &[0, 7, 0],
                    &[(62, 7, false), (0, 0, false)],
                    "",
                ),
                Refusal::UnsupportedRest,
            ),
            (
                chain(
                    &[0, 128, 128],
                    &[0, 7, 0],
                    &[(0, 0, false), (62, 7, true)],
                    "",
                ),
                Refusal::TopOnly,
            ),
            (
                chain(
                    &[0, 96, 0],
                    &[0, 7, 0],
                    &[(62, 7, false), (0, 0, false)],
                    "",
                ),
                Refusal::OneWayBarrier,
            ),
            (
                chain(
                    &[0, 128, 128],
                    &[0, 7, 0],
                    &[(62, 7, false), (0, 0, false)],
                    "linedef { v1 = 5; v2 = 4; sidefront = 99; blocking = true; special = 23; arg0 = 7; }\nsidedef { sector = 2; texturemiddle = \"STARTAN2\"; }\n",
                ),
                Refusal::ConflictingAction,
            ),
        ];
        for (text, expected) in cases {
            // The conflicting-action fixture's extra sidedef is appended after
            // the chain's own, so its index is one less than the total.
            let text = text.replacen(
                "sidefront = 99",
                &format!("sidefront = {}", text.matches("sidedef {").count() - 1),
                1,
            );
            let r = report_of(&text);
            let refusals: Vec<Option<Refusal>> = r.plats.iter().map(|p| p.refusal).collect();
            assert!(!refusals.is_empty(), "{expected:?}: no plat resolved");
            assert!(
                refusals.iter().all(|x| *x == Some(expected)),
                "{expected:?}: {refusals:?}"
            );
            assert!(r.plats.iter().all(|p| p.shape.is_none()));
        }
        let r = report_of(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 0, false), (62, 9, false)],
            "",
        ));
        assert_eq!(r.broken_lines, vec![0, 1]);
        assert_eq!(r.counts.refusals(), 2);
    }

    #[test]
    fn a_shared_tag_group_is_split_only_when_it_is_one_floor_and_connected() {
        // Both members at 128 and adjacent: one platform the trim split. A
        // low room at each end, so neither member is `Dead` — `travel == 0`
        // is judged before the shared tag is.
        let r = report_of(&chain(
            &[0, 128, 128, 0],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!((r.counts.shared_tag, r.counts.shared_split), (2, 1));

        // Two members at one floor with an untagged room between them: one
        // floor, but the walk over two-sided boundaries never leaves a member
        // to reach the other.
        let r = report_of(&chain(
            &[128, 0, 128],
            &[7, 0, 7],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!((r.counts.shared_tag, r.counts.shared_split), (2, 0));

        // Adjacent, but at two floors.
        let r = report_of(&chain(
            &[0, 128, 96, 0],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!((r.counts.shared_tag, r.counts.shared_split), (2, 0));
    }

    #[test]
    fn callable_classes_and_the_top_trigger_are_reported() {
        // A walkover on the low face fires from the platform alone.
        let r = report_of(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(88, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            r.plats[0].callable,
            Callable {
                low: false,
                level: false,
                plat: true,
                above: false
            }
        );
        assert!(r.plats[0].top_trigger);
        assert_eq!(
            (
                r.counts.top_only,
                r.counts.with_top_trigger,
                r.counts.callable_low
            ),
            (1, 1, 0)
        );

        // A use line whose front is the level room beyond the platform.
        let r = report_of(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(0, 0, false), (62, 7, true)],
            "",
        ));
        assert!(r.plats[0].callable.level && r.plats[0].top_trigger);

        // A use line whose front sits more than a step above the platform.
        // `Above` is no caller, so the platform is refused — on its rest
        // first, which is judged before who can call it.
        let r = report_of(&chain(
            &[0, 64, 160],
            &[0, 7, 0],
            &[(0, 0, false), (62, 7, true)],
            "",
        ));
        assert_eq!(
            r.plats[0].callable,
            Callable {
                low: false,
                level: false,
                plat: false,
                above: true
            }
        );
        assert_eq!(r.plats[0].refusal, Some(Refusal::UnsupportedRest));
        assert!(!r.plats[0].top_trigger);
    }

    #[test]
    fn things_on_a_platform_are_counted() {
        let r = report_of(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "thing { x = 192.0; y = 64.0; angle = 0; type = 2012; single = true; }\n",
        ));
        assert_eq!(r.plats[0].things, 1);
        assert_eq!(r.counts.with_things, 1);
        assert!(
            !r.plats[0].island,
            "a chain room's outer walls are one-sided"
        );
    }

    #[test]
    fn counts_add_field_wise() {
        let a = report_of(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ))
        .counts;
        let b = report_of(&chain(&[0, 128], &[0, 7], &[(123, 7, false)], "")).counts;
        let sum = a.add(&b);
        assert_eq!(
            (
                sum.plats,
                sum.lifts,
                sum.pedestals,
                sum.fast,
                sum.callable_low
            ),
            (2, 1, 1, 1, 2)
        );
        assert_eq!(sum.refusals(), 0);
        assert_eq!(PlatCounts::default().add(&b), b);
    }

    #[test]
    fn a_report_serializes_to_json_with_snake_case_names() {
        let r = report_of(&chain(&[0, 128], &[0, 7], &[(123, 7, false)], ""));
        let json = serde_json::to_value(&r).expect("serializes");
        assert_eq!(json["counts"]["pedestals"], 1);
        assert_eq!(json["plats"][0]["shape"], "pedestal");
        assert_eq!(json["plats"][0]["speed"], "fast");
        assert_eq!(json["plats"][0]["rest"], "above_all");
        assert_eq!(json["plats"][0]["refusal"], serde_json::Value::Null);
        assert_eq!(
            json["plats"][0]["callable"]["low"],
            serde_json::Value::Bool(true)
        );

        let r = report_of(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let json = serde_json::to_value(&r).expect("serializes");
        assert_eq!(json["plats"][0]["refusal"], "dead");
        assert_eq!(json["counts"]["dead"], 1);
    }
}
