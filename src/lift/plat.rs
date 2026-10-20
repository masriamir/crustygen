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
//! the idgames corpus — with a gate the probe's `clean` test never had:
//! [`Refusal::OneWayBarrier`]. A barrier callable from one side scores a
//! shape in the probe and is refused here, so this module's shape buckets
//! are subsets of the measurement's, not the same counts. A shared-tag
//! member the probe also shapes is refused here only when no adjacent line
//! calls it ([`Refusal::BankCaller`]); every other member's shape stands.
//! The probe names refusal reasons only as a corpus histogram (its `why
//! Other` row); this module attaches one to each platform and adds
//! [`Refusal::Dead`], [`Refusal::BankCaller`] and [`Refusal::OneWayBarrier`]
//! to that vocabulary.
//!
//! **Refusal precedence.** A platform is judged against the eight
//! [`Refusal`]s in one fixed order and the first that applies wins, so a
//! platform wrong in more than one way reports its most fundamental reason
//! rather than an order-of-evaluation accident:
//!
//! 1. [`Refusal::Dead`] — it cannot move at all, so nothing else about it
//!    matters.
//! 2. [`Refusal::OneShot`] and 3. [`Refusal::MixedSpeed`] — how its triggers
//!    fire, which is a fact about the lines and holds whatever the geometry.
//! 4. [`Refusal::UnsupportedRest`] — where it rests, which decides whether a
//!    shape exists to name at all.
//! 5. [`Refusal::TopOnly`] and 6. [`Refusal::OneWayBarrier`] — who can call
//!    it, judged only once its rest is one a shape names.
//! 7. [`Refusal::BankCaller`] — a shared-tag member no adjacent line calls,
//!    judged only once it is known to be callable from Low by *something*
//!    ([`Refusal::TopOnly`] having already caught the ones that are not).
//! 8. [`Refusal::ConflictingAction`] — last, because a platform this module
//!    would otherwise accept is the only one for which "something else drives
//!    it too" is the whole story.
//!
//! A shared tag is not itself a refusal. Every member is judged alone, the
//! way the floor recognizer judges a shared-tag target, and only a member no
//! adjacent line calls is refused
//! (`docs/measurements/lift-variants-2026-09-11.md` §I, gate A′) — unless the
//! group is one platform split by trim (below).
//!
//! **Split groups.** A shared-tag group whose members all sit at one floor
//! and are mutually adjacent over two-sided boundaries (`is_split_group`)
//! is one platform split by trim — the mapper drew a seam across one lift so
//! its halves could carry different textures — and is read as one [`Plat`],
//! listed at its least member with every member in [`Plat::members`].
//! "Mutually adjacent" here, and wherever these docs say it, means
//! *connected*: the walk `split_group_tags` describes and `is_split_group`
//! performs, over two-sided boundaries and never leaving the group, reaches
//! every member from any one of them. A chain of three strips whose ends
//! never touch qualifies; two pairs with an untagged room between them do
//! not. The merged platform's outside neighbors are the union of the members'
//! two-sided neighbors minus the members themselves; its `low` is the least
//! of the members' `low`s; its triggers are the tag's lines; and an activator
//! that is itself a member is a [`Activator::Plat`] activator — a rider's
//! call, not a caller's. It is judged against the same eight refusals as any
//! platform, never [`Refusal::BankCaller`]: the group is the whole tag, so
//! there is no other member for the bank's lines to be on. This is an
//! approximation, deliberately: `EV_DoPlat` runs one thinker per sector
//! carrying the tag, each with its own `P_FindLowestFloorSurrounding`
//! (`p_plats.c:164-169`, `:207-212`, as
//! `docs/measurements/lift-variants-2026-09-11.md` §I records), so a trim
//! piece with no lower outside neighbor does not move at all while the merged
//! reading says the whole platform does
//! (`KNOWN-GAPS.md`, "A split platform is read as one lift"). The compiler
//! never emits a split platform, so the merge is a reading of other people's
//! maps, never a round trip of its own.
//!
//! One consequence worth naming: a member of a shared tag that cannot move
//! reports [`Refusal::Dead`] rather than [`Refusal::BankCaller`], and one no
//! trigger calls from Low at all reports [`Refusal::TopOnly`] first — the
//! order judges movement and reachability before this tag's identity, so
//! the members of one group need not all carry the same refusal.
//!
//! A sharper consequence of the same order: [`Refusal::OneWayBarrier`]
//! *preempts* [`Refusal::BankCaller`] for every [`Rest::AboveAll`] platform
//! with two or more neighbors. `BankCaller`'s gate is that no neighbor is a
//! Low activator, which always satisfies `OneWayBarrier`'s "fewer Low
//! activator neighbors than neighbors" — so an uncalled barrier-shaped member
//! reports `OneWayBarrier`, and `bank_caller` can only ever fire for a
//! [`Rest::Top`] platform or a single-neighbor [`Rest::AboveAll`]
//! (pedestal-shaped) one.
//!
//! A lift line that names no platform at all — tag 0, or a tag no sector
//! answers to — is not a refused platform but a broken line, listed in
//! [`PlatReport::broken_lines`] as [`broken_lift_lines`] reports it and
//! counted by [`PlatCounts::refusals`] alongside the refused platforms.

use std::collections::BTreeSet;

use crate::check::plats::{Activator, ScenePlat, SceneTrigger, broken_lift_lines, resolve_plats};
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
    /// The tag names more than one sector ([`ScenePlat::shared_tag`]; never a
    /// split group read as one platform — module doc, "Split groups") and no
    /// trigger fires from a neighbor of *this* member standing more than a
    /// step below it ([`ScenePlat::low_activator_neighbors`], an
    /// [`Activator::Low`] that is also a neighbor): the bank's lines are on
    /// another member or somewhere else, and the IR calls a bank member only
    /// from a sector that touches it (`compile::lifts::resolve_bank_callers`).
    /// The gate here is a *class*, deliberately looser than the compiler's:
    /// `rules::check_lift_return` requires the caller to stand exactly at the
    /// platform's low floor, since a caller the platform does not come down to
    /// cannot board it, while a WAD being read only has to be recognizable.
    ///
    /// Reachable for a [`Rest::Top`] platform and a single-neighbor
    /// [`Rest::AboveAll`] one only: [`Self::OneWayBarrier`] takes every other
    /// uncalled `AboveAll` member first (see the module doc's precedence).
    BankCaller,
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
    /// Declaration index of the platform sector — the least member, for a
    /// split group read as one platform.
    pub sector: usize,
    /// Declaration indices of every sector the platform comprises, ascending:
    /// `[sector]` for a single-sector platform, every member of a split group
    /// read as one (see the module doc's "Split groups"). Never empty.
    pub members: Vec<usize>,
    /// The tag its lift lines name it by.
    pub tag: i32,
    /// Where it rests relative to the two-sided neighbors outside
    /// [`Self::members`] (for a single-sector platform, all of them).
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
    /// How many things stand in any of its [`Self::members`].
    pub things: u64,
    /// Whether every edge of every member is two-sided — a free-standing pad.
    /// The seam edges between the members of a split group are two-sided, so
    /// they never count against it. Reported as a statistic, never a gate.
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
    /// Platforms refused [`Refusal::BankCaller`].
    pub bank_caller: u64,
    /// Shared-tag groups that are one platform split by trim: every member at
    /// one floor and all of them mutually adjacent, each read as one platform
    /// (the module doc's "Split groups") — so also the count of merged
    /// platforms among [`Self::plats`]. Not a sub-count of
    /// [`Self::bank_caller`]: a merged platform is never refused
    /// [`Refusal::BankCaller`], and is accepted or refused on its own facts
    /// like any other. [`Self::split_members`] counts the sectors these groups
    /// comprise.
    pub shared_split: u64,
    /// Sectors the [`Self::shared_split`] groups comprise, so a per-sector
    /// reading reconciles with this one:
    /// `sectors_named = plats + split_members - shared_split`, where
    /// `sectors_named` is the number of sectors some lift line names
    /// ([`resolve_plats`]' length).
    pub split_members: u64,
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
            bank_caller: self.bank_caller.saturating_add(other.bank_caller),
            shared_split: self.shared_split.saturating_add(other.shared_split),
            split_members: self.split_members.saturating_add(other.split_members),
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
/// If a platform's thing count, the shared-tag-group tally or the tally of the
/// sectors those groups comprise does not fit a `u64`, which needs more
/// elements than a parsed map can hold (`usize` is at most 64 bits on every
/// platform this crate builds on).
#[must_use]
pub fn recognize(scene: &Scene, tables: &Tables) -> PlatReport {
    let step = tables.step_height();
    let resolved = resolve_plats(scene, tables);
    let split_groups = split_group_tags(scene, &resolved);
    let mut plats = Vec::with_capacity(resolved.len());
    // `resolved` is ascending by sector, so a split group is met first at its
    // least member and emitted there, once; its other members are skipped.
    let mut emitted: BTreeSet<i32> = BTreeSet::new();
    let mut split_members = 0usize;
    for p in &resolved {
        if split_groups.contains(&p.tag) {
            if !emitted.insert(p.tag) {
                continue;
            }
            let group: Vec<&ScenePlat> = resolved.iter().filter(|q| q.tag == p.tag).collect();
            split_members += group.len();
            plats.push(plat_of(scene, &Subject::merged(scene, &group, step), p.tag));
        } else {
            plats.push(plat_of(scene, &Subject::single(scene, p), p.tag));
        }
    }
    let broken_lines = broken_lift_lines(scene, tables);
    let counts = count(
        &plats,
        &broken_lines,
        u64::try_from(split_groups.len()).expect("fits u64"),
        u64::try_from(split_members).expect("fits u64"),
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

/// What the eight refusals and the shape rule judge: one platform's facts,
/// whether that platform is one sector or a split group read as one.
struct Subject<'a> {
    /// Sectors the platform comprises, ascending (one for a single platform).
    members: Vec<usize>,
    /// Its rest floor.
    floor: i32,
    /// The floor `EV_DoPlat` sends it to.
    low: i32,
    /// Where it rests relative to `outside`.
    rest: Rest,
    /// Its two-sided neighbors (for a merged group, excluding the members).
    /// For a single platform this is [`ScenePlat::neighbors`] unfiltered, so
    /// a self-referencing two-sided line can name the sector itself here.
    outside: BTreeSet<usize>,
    /// Distinct floors `outside` sits at.
    distinct_outside_floors: usize,
    /// Whether the tag names sectors beyond this platform (a bank member).
    shared: bool,
    /// Every lift line naming the tag.
    triggers: &'a [SceneTrigger],
    /// Every (sector, class) pair firing some trigger, classified against this
    /// platform.
    activators: Vec<(usize, Activator)>,
    /// Non-lift specials naming the tag.
    other_actions: &'a [i32],
}

impl<'a> Subject<'a> {
    /// One sector's platform, exactly as the resolver read it.
    fn single(scene: &Scene, p: &'a ScenePlat) -> Self {
        Self {
            members: vec![p.sector],
            floor: scene.sectors[p.sector].floor,
            low: p.low,
            rest: p.rest,
            outside: p.neighbors.clone(),
            distinct_outside_floors: p.distinct_neighbor_floors,
            shared: p.shared_tag >= 2,
            triggers: &p.triggers,
            activators: p
                .triggers
                .iter()
                .flat_map(|t| t.activators.iter().copied())
                .collect(),
            other_actions: &p.other_actions,
        }
    }

    /// A split group read as one platform (the module doc's "Split groups"
    /// approximation). `group` must be *every* [`ScenePlat`] carrying the tag
    /// — the whole tag, not a subset: `shared: false` and "never
    /// [`Refusal::BankCaller`]" both rest on there being no other member.
    /// (Reading the triggers off `group[0]` rests instead on every member
    /// carrying one tag — `triggers_for` filters on the tag alone — which the
    /// one-tag assert below pins.) Assembling the whole tag is
    /// [`recognize`]'s responsibility, and it is checkable from in here:
    /// [`ScenePlat::shared_tag`] counts every scene sector carrying the tag
    /// and [`resolve_plats`] resolves every sector a lift tag names, so the
    /// group is the whole tag exactly when `group.len() == shared_tag`. All
    /// four preconditions are debug-asserted: at least two members, all at
    /// one floor, all carrying `group[0]`'s tag, and together the whole tag.
    /// `step` is [`Tables::step_height`].
    fn merged(scene: &Scene, group: &[&'a ScenePlat], step: i32) -> Self {
        debug_assert!(group.len() >= 2, "a split group has at least two members");
        debug_assert!(
            group
                .iter()
                .all(|p| scene.sectors[p.sector].floor == scene.sectors[group[0].sector].floor),
            "a split group's members all sit at one floor"
        );
        debug_assert!(
            group.iter().all(|p| p.tag == group[0].tag),
            "a split group's members all carry one tag"
        );
        debug_assert!(
            group.iter().all(|p| p.shared_tag == group.len()),
            "a split group is the whole tag"
        );
        let members: Vec<usize> = group
            .iter()
            .map(|p| p.sector)
            .collect::<BTreeSet<usize>>()
            .into_iter()
            .collect();
        let floor = scene.sectors[members[0]].floor;
        let outside: BTreeSet<usize> = group
            .iter()
            .flat_map(|p| p.neighbors.iter().copied())
            .filter(|n| !members.contains(n))
            .collect();
        // Each member's `low` already starts at its own floor
        // (`P_FindLowestFloorSurrounding`), and a sibling member is a neighbor
        // at that same floor, so the least of the members' `low`s is the floor
        // the merged platform would be sent to: the lowest outside neighbor,
        // or its own floor when none is lower.
        let low = group
            .iter()
            .map(|p| p.low)
            .min()
            .expect("a group has members");
        // The rest rule is `check::plats::resolve_plats`'s, applied to the
        // outside neighbors in place of one sector's: `Dead` when nothing is
        // lower, `Intermediate` when some neighbor stands more than a step
        // above, `Top` when the highest neighbor is within a step, and
        // `AboveAll` when every neighbor is more than a step below.
        let max_nb = outside
            .iter()
            .map(|&n| scene.sectors[n].floor)
            .max()
            .unwrap_or(floor);
        let rest = if low == floor {
            Rest::Dead
        } else if max_nb > floor + step {
            Rest::Intermediate
        } else if max_nb >= floor - step {
            Rest::Top
        } else {
            Rest::AboveAll
        };
        let distinct_outside_floors = outside
            .iter()
            .map(|&n| scene.sectors[n].floor)
            .collect::<BTreeSet<i32>>()
            .len();
        // Every member of a tag resolves the same lift lines
        // (`check::plats::triggers_for` filters on the tag alone), so one
        // member's trigger list is the group's.
        let triggers: &[SceneTrigger] = &group[0].triggers;
        // `check::plats::classify` depends only on the platform's floor, which
        // the members share, so one member's view of the activators serves the
        // group — except that a firing sector which is itself a member is the
        // platform, `Activator::Plat`, however that member classified it
        // (`Level` for a sibling, `Plat` for itself).
        let activators = triggers
            .iter()
            .flat_map(|t| t.activators.iter().copied())
            .map(|(s, a)| {
                if members.contains(&s) {
                    (s, Activator::Plat)
                } else {
                    (s, a)
                }
            })
            .collect();
        Self {
            members,
            floor,
            low,
            rest,
            outside,
            distinct_outside_floors,
            shared: false,
            triggers,
            activators,
            // Resolved per tag too (`check::plats::other_specials_on_tag`).
            other_actions: &group[0].other_actions,
        }
    }

    /// How far the platform travels: its rest floor minus the floor
    /// `EV_DoPlat` sends it to ([`ScenePlat::travel`]).
    fn travel(&self) -> i32 {
        self.floor - self.low
    }

    /// [`Speed::Fast`] when every trigger is a `blazeDWUS` form,
    /// [`Speed::Normal`] when none is, [`Speed::Mixed`] in between.
    fn speed(&self) -> Speed {
        let fasts = self.triggers.iter().filter(|t| t.fast).count();
        if fasts == 0 {
            Speed::Normal
        } else if fasts == self.triggers.len() {
            Speed::Fast
        } else {
            Speed::Mixed
        }
    }

    /// Which [`Activator`] classes fire some trigger of the platform.
    fn callable(&self) -> Callable {
        let any = |want: Activator| self.activators.iter().any(|&(_, a)| a == want);
        Callable {
            low: any(Activator::Low),
            level: any(Activator::Level),
            plat: any(Activator::Plat),
            above: any(Activator::Above),
        }
    }

    /// The `outside` sectors that are [`Activator::Low`] of some trigger
    /// ([`ScenePlat::low_activator_neighbors`]).
    fn low_activator_neighbors(&self) -> BTreeSet<usize> {
        self.activators
            .iter()
            .filter(|&&(s, a)| a == Activator::Low && self.outside.contains(&s))
            .map(|&(s, _)| s)
            .collect()
    }

    /// Whether some trigger fires from the platform itself or from within a
    /// step of it ([`ScenePlat::callable_top`]) — a rider's call, not a
    /// caller's.
    fn top_trigger(&self) -> bool {
        self.activators
            .iter()
            .any(|&(_, a)| matches!(a, Activator::Level | Activator::Plat))
    }

    /// The first [`Refusal`] that applies to the platform, in the module
    /// doc's order.
    fn refusal(&self, callable: Callable, speed: Speed) -> Option<Refusal> {
        let one_floor = self.distinct_outside_floors == 1;
        let low_callers = self.low_activator_neighbors();
        if self.rest == Rest::Dead {
            Some(Refusal::Dead)
        } else if !self.triggers.iter().all(|t| t.repeatable) {
            Some(Refusal::OneShot)
        } else if speed == Speed::Mixed {
            Some(Refusal::MixedSpeed)
        } else if self.rest == Rest::Intermediate || (self.rest == Rest::AboveAll && !one_floor) {
            Some(Refusal::UnsupportedRest)
        } else if !callable.low {
            Some(Refusal::TopOnly)
        } else if self.rest == Rest::AboveAll
            && self.outside.len() >= 2
            && low_callers.len() < self.outside.len()
        {
            Some(Refusal::OneWayBarrier)
        } else if self.shared && low_callers.is_empty() {
            Some(Refusal::BankCaller)
        } else if !self.other_actions.is_empty() {
            Some(Refusal::ConflictingAction)
        } else {
            None
        }
    }

    /// The [`Shape`] of an unrefused platform: [`Rest::Top`] is a lift, and
    /// [`Rest::AboveAll`] — already known to have its neighbors at one floor,
    /// or [`Refusal::UnsupportedRest`] would have caught it — is a pedestal
    /// against one neighbor and a barrier against more.
    fn shape(&self, refusal: Option<Refusal>) -> Option<Shape> {
        match (refusal, self.rest, self.outside.len()) {
            (None, Rest::Top, _) => Some(Shape::Lift),
            (None, Rest::AboveAll, 1) => Some(Shape::Pedestal),
            (None, Rest::AboveAll, _) => Some(Shape::Barrier),
            _ => None,
        }
    }
}

/// Builds the report entry for a judged subject.
fn plat_of(scene: &Scene, subject: &Subject<'_>, tag: i32) -> Plat {
    debug_assert!(
        !subject.members.is_empty() && subject.members.windows(2).all(|w| w[0] < w[1]),
        "a subject's members are non-empty and strictly ascending"
    );
    // A single platform's `outside` is `ScenePlat::neighbors` unfiltered, which
    // a self-referencing two-sided line can make name the sector itself; the
    // merge is what filters, so the merge is what the invariant binds.
    debug_assert!(
        subject.members.len() == 1 || subject.outside.iter().all(|n| !subject.members.contains(n)),
        "a merged subject's outside neighbors are disjoint from its members"
    );
    debug_assert!(
        !(subject.shared && subject.members.len() > 1),
        "a merged subject is the whole tag, never a bank member"
    );
    let speed = subject.speed();
    let callable = subject.callable();
    let refusal = subject.refusal(callable, speed);
    Plat {
        sector: subject.members[0],
        members: subject.members.clone(),
        tag,
        rest: subject.rest,
        travel: subject.travel(),
        shape: subject.shape(refusal),
        speed,
        callable,
        triggers: subject.triggers.iter().map(|t| t.linedef).collect(),
        things: u64::try_from(
            scene
                .things
                .iter()
                .filter(|t| t.sector.is_some_and(|s| subject.members.contains(&s)))
                .count(),
        )
        .expect("fits u64"),
        island: subject
            .members
            .iter()
            .all(|&m| scene.sectors[m].boundary.iter().all(|b| b.two_sided)),
        top_trigger: subject.top_trigger(),
        refusal,
    }
}

/// Counts `plats` one field per predicate, `shared_split` being
/// [`split_group_tags`]'s tally and `split_members` the sectors those groups
/// comprise.
fn count(
    plats: &[Plat],
    broken_lines: &[usize],
    shared_split: u64,
    split_members: u64,
) -> PlatCounts {
    let mut c = PlatCounts {
        shared_split,
        split_members,
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
            Some(Refusal::OneShot) => c.one_shot += 1,
            Some(Refusal::MixedSpeed) => c.mixed_speed += 1,
            Some(Refusal::UnsupportedRest) => c.unsupported_rest += 1,
            Some(Refusal::TopOnly) => c.top_only += 1,
            Some(Refusal::OneWayBarrier) => c.one_way_barrier += 1,
            Some(Refusal::BankCaller) => c.bank_caller += 1,
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
    use crate::check::fixtures::{chain, far_wall, scene_of};

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

    /// Two lifts on tag 7, each `Rest::Top` (a landing neighbor at its own
    /// floor, a low neighbor at the far end). The only lift trigger is a
    /// switch on room 6 — adjacent to neither lift — so neither lift's own
    /// neighbors call it: both are `BankCaller` although both are
    /// `callable_low` (room 6 fires Low for both), so `TopOnly` never
    /// claims them first.
    ///
    /// A second far-wall line carries a *non-lift* special on the same tag
    /// (23, `S1 Floor Lower to Lowest`), so `ConflictingAction` applies to
    /// both members as well. That is deliberate: it pins the precedence, since
    /// `BankCaller` comes first and the expected refusal must stay
    /// `BankCaller` rather than drift to the later arm.
    fn bank_caller_case() -> String {
        let mut text = chain(
            &[0, 128, 128, 0, 128, 128, 0],
            &[0, 7, 0, 0, 7, 0, 0],
            &[
                (0, 0, false),
                (0, 0, false),
                (0, 0, false),
                (0, 0, false),
                (0, 0, false),
                (0, 0, false),
            ],
            "",
        );
        far_wall(&mut text, 7, 62, 7);
        far_wall(&mut text, 7, 23, 7);
        text
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
            (bank_caller_case(), Refusal::BankCaller),
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

    /// Two lifts on one tag in a row of rooms `0 | 1 | 2 | 3 | 4` with floors
    /// `0, 128, 128, 128, 0`: each lift rests level with room 2 and above an
    /// outer room, so both are `Rest::Top`. The only switch is on lift 1's
    /// west face, fired from room 0: lift 1 is neighbor-called; lift 3's low
    /// neighbor is room 4, which fires nothing, so it is `BankCaller` — it
    /// is still `callable_low` (room 0 is a Low activator), so `TopOnly`
    /// does not claim it first.
    #[test]
    fn a_bank_member_is_judged_alone_and_refused_only_when_no_adjacent_line_calls_it() {
        let r = report_of(&chain(
            &[0, 128, 128, 128, 0],
            &[0, 7, 0, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        let first = r.plats.iter().find(|p| p.sector == 1).expect("plat 1");
        let second = r.plats.iter().find(|p| p.sector == 3).expect("plat 3");
        assert_eq!((first.shape, first.refusal), (Some(Shape::Lift), None));
        assert_eq!(
            (second.shape, second.refusal),
            (None, Some(Refusal::BankCaller))
        );
        assert_eq!(
            (r.counts.bank_caller, r.counts.refused, r.counts.lifts),
            (1, 1, 1)
        );
    }

    /// Two platforms on one tag both bordering room 2 at floor 0, in a row
    /// `0 | 1 | 2 | 3` with floors `0, 128, 0, 128`: plat 1 has two low
    /// neighbors (a barrier), plat 3 one (a pedestal). Switches on the
    /// 0–1 line and the 2–3 line fire from rooms 0 and 2, so every neighbor
    /// of plat 1 calls it and plat 3's one neighbor calls it: both accepted.
    #[test]
    fn a_shared_tag_whose_members_are_all_neighbor_called_is_accepted_member_by_member() {
        let r = report_of(&chain(
            &[0, 128, 0, 128],
            &[0, 7, 0, 7],
            &[(62, 7, false), (0, 0, false), (62, 7, false)],
            "",
        ));
        assert!(r.plats.iter().all(|p| p.refusal.is_none()), "{:?}", r.plats);
        assert_eq!(
            (r.counts.barriers, r.counts.pedestals, r.counts.bank_caller),
            (1, 1, 0)
        );
    }

    #[test]
    fn a_shared_tag_group_is_split_only_when_it_is_one_floor_and_connected() {
        // Both members at 128 and adjacent: one platform the trim split, read as
        // one. Its outside neighbors are rooms 0 and 3, both at floor 0, so it is
        // `AboveAll` between two low rooms; only room 0 fires, so the merged
        // platform is a `OneWayBarrier` — where the members alone read as one
        // accepted lift and one `BankCaller`.
        let r = report_of(&chain(
            &[0, 128, 128, 0],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!(r.plats.len(), 1);
        assert_eq!(
            (r.plats[0].sector, r.plats[0].members.clone()),
            (1, vec![1, 2])
        );
        assert_eq!(
            (r.plats[0].rest, r.plats[0].refusal),
            (Rest::AboveAll, Some(Refusal::OneWayBarrier))
        );
        assert_eq!(
            (
                r.counts.plats,
                r.counts.one_way_barrier,
                r.counts.bank_caller,
                r.counts.shared_split,
                r.counts.split_members
            ),
            (1, 1, 0, 1, 2)
        );

        // Two members at one floor with an untagged room between them: one
        // floor, but the walk over two-sided boundaries never leaves a member
        // to reach the other, so `shared_split` is 0. Neither member is
        // `BankCaller` either, but for a different reason: the switch is on
        // the 0–1 line, front room 0. Member 0 *is* room 0, so its own
        // trigger fires from itself (`Activator::Plat`, a rider's call, not
        // a caller's); member 2 sits at member 0's own floor, so the same
        // trigger classifies as `Level` against it, not `Low`. Both are
        // `TopOnly` — refused for want of any caller, before the shared tag
        // is ever judged.
        let r = report_of(&chain(
            &[128, 0, 128],
            &[7, 0, 7],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let refusals: Vec<Option<Refusal>> = r.plats.iter().map(|p| p.refusal).collect();
        assert_eq!(
            refusals,
            vec![Some(Refusal::TopOnly), Some(Refusal::TopOnly)]
        );
        assert_eq!((r.counts.bank_caller, r.counts.shared_split), (0, 0));
        assert!(r.plats.iter().all(|p| p.members == vec![p.sector]));

        // Adjacent, but at two floors, so `shared_split` is 0 — and each
        // member's rest disqualifies it before the shared tag would: member
        // 1 (128) neighbors 0 and 96, two distinct floors, `AboveAll`, so
        // `UnsupportedRest`; member 2 (96) neighbors 128, more than a step
        // above it, `Intermediate`, `UnsupportedRest` too.
        let r = report_of(&chain(
            &[0, 128, 96, 0],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        let refusals: Vec<Option<Refusal>> = r.plats.iter().map(|p| p.refusal).collect();
        assert_eq!(
            refusals,
            vec![
                Some(Refusal::UnsupportedRest),
                Some(Refusal::UnsupportedRest)
            ]
        );
        assert_eq!((r.counts.bank_caller, r.counts.shared_split), (0, 0));
        assert!(r.plats.iter().all(|p| p.members == vec![p.sector]));
    }

    /// Three tagged platforms at one floor, each adjacent to the other two:
    /// `A` fills the southwest quarter, `B` the southeast, and `C` spans the
    /// whole north half above them both, so `A|B`, `A|C` and `B|C` are all
    /// shared edges. `L` (west, below `A` and `C`) and `L2` (east, below `B`)
    /// are the low rooms that give each member somewhere to travel to, and
    /// `L`'s edge with `A` carries the riser switch naming tag 7.
    ///
    /// `chain` cannot build this: its rooms sit in a row, so a member only
    /// ever touches the next one along. Mutual adjacency is what makes the
    /// group's walk reach a member twice — the second time from the third
    /// member rather than the seed.
    const TRIANGLE: &str = r#"namespace = "doom";
vertex { x = -128.000; y = 0.000; }
vertex { x = -128.000; y = 256.000; }
vertex { x = 0.000; y = 0.000; }
vertex { x = 0.000; y = 128.000; }
vertex { x = 0.000; y = 256.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 128.000; }
vertex { x = 256.000; y = 256.000; }
vertex { x = 384.000; y = 0.000; }
vertex { x = 384.000; y = 128.000; }

linedef { v1 = 3; v2 = 2; sidefront = 0; sideback = 1; twosided = true; special = 62; arg0 = 7; }
linedef { v1 = 3; v2 = 4; sidefront = 2; sideback = 3; twosided = true; }
linedef { v1 = 5; v2 = 6; sidefront = 4; sideback = 5; twosided = true; }
linedef { v1 = 3; v2 = 6; sidefront = 6; sideback = 7; twosided = true; }
linedef { v1 = 6; v2 = 8; sidefront = 8; sideback = 9; twosided = true; }
linedef { v1 = 7; v2 = 8; sidefront = 10; sideback = 11; twosided = true; }
linedef { v1 = 2; v2 = 0; sidefront = 12; blocking = true; }
linedef { v1 = 0; v2 = 1; sidefront = 13; blocking = true; }
linedef { v1 = 1; v2 = 4; sidefront = 14; blocking = true; }
linedef { v1 = 5; v2 = 2; sidefront = 15; blocking = true; }
linedef { v1 = 7; v2 = 5; sidefront = 16; blocking = true; }
linedef { v1 = 9; v2 = 8; sidefront = 17; blocking = true; }
linedef { v1 = 4; v2 = 9; sidefront = 18; blocking = true; }
linedef { v1 = 10; v2 = 7; sidefront = 19; blocking = true; }
linedef { v1 = 11; v2 = 10; sidefront = 20; blocking = true; }
linedef { v1 = 8; v2 = 11; sidefront = 21; blocking = true; }

sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 3; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 3; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 3; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 4; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sidedef { sector = 3; texturemiddle = "STARTAN2"; }
sidedef { sector = 4; texturemiddle = "STARTAN2"; }
sidedef { sector = 4; texturemiddle = "STARTAN2"; }
sidedef { sector = 4; texturemiddle = "STARTAN2"; }

sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 128; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 128; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 128; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
"#;

    #[test]
    fn a_mutually_adjacent_three_member_group_is_one_platform() {
        // The walk reaches `B` from both `A` and `C`, so it is stacked twice
        // and popped twice; the second pop must be dropped rather than
        // walked again. All three members still count as one split group,
        // and each member touches a low room, so the merged platform has
        // somewhere to go.
        let r = report_of(TRIANGLE);
        assert_eq!(r.plats.len(), 1);
        assert_eq!(r.plats[0].members, vec![1, 2, 3]);
        // Outside neighbors are L and L2, both at 0; only L's edge with A fires.
        assert_eq!(r.plats[0].refusal, Some(Refusal::OneWayBarrier));
        assert!(
            !r.plats[0].island,
            "the triangle's outer walls are one-sided"
        );
        assert_eq!(
            (
                r.counts.plats,
                r.counts.bank_caller,
                r.counts.shared_split,
                r.counts.split_members
            ),
            (1, 0, 1, 3)
        );
    }

    /// A(0)|T1(128)|T2(128)|B(128), tag 7 on T1 and T2, the riser switch on
    /// A|T1. T2 alone has nothing below it and is `Dead`; read as one platform
    /// the pair rests level with B and drops to A: a lift, called from A.
    #[test]
    fn a_split_group_is_read_as_one_platform() {
        let r = report_of(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!(r.plats.len(), 1);
        let p = &r.plats[0];
        assert_eq!((p.sector, p.members.clone(), p.tag), (1, vec![1, 2], 7));
        assert_eq!(
            (p.rest, p.travel, p.shape, p.refusal),
            (Rest::Top, 128, Some(Shape::Lift), None)
        );
        assert_eq!(
            p.callable,
            Callable {
                low: true,
                level: false,
                plat: false,
                above: false
            }
        );
        assert!(!p.top_trigger);
        assert_eq!(
            (
                r.counts.plats,
                r.counts.lifts,
                r.counts.refused,
                r.counts.dead,
                r.counts.shared_split,
                r.counts.split_members,
                r.counts.callable_low
            ),
            (1, 1, 0, 0, 1, 2, 1)
        );
    }

    /// The one-way-barrier pair above, with a second riser switch on the 2–3
    /// line fronted by room 3: both outside rooms call it, so it is a barrier.
    #[test]
    fn a_split_group_called_from_both_sides_is_a_barrier() {
        let r = report_of(&chain(
            &[0, 128, 128, 0],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (62, 7, true)],
            "",
        ));
        assert_eq!(r.plats.len(), 1);
        assert_eq!(
            (r.plats[0].shape, r.plats[0].refusal),
            (Some(Shape::Barrier), None)
        );
        assert_eq!((r.counts.barriers, r.counts.lifts), (1, 0));
    }

    /// The only lift line sits on the seam between the two members, fronted by
    /// member 2. Against member 1 alone that activator would be `Level` — a
    /// sibling at its own floor — and member 1 is the member whose view of the
    /// activators the merge starts from; against the merged platform a member
    /// is the platform itself, so the call is a rider's: `Plat`, not `Level`,
    /// and nothing calls from below.
    #[test]
    fn a_switch_on_the_seam_between_members_is_a_riders_call() {
        let r = report_of(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(0, 0, false), (62, 7, true), (0, 0, false)],
            "",
        ));
        assert_eq!(r.plats.len(), 1);
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
        assert_eq!(r.plats[0].refusal, Some(Refusal::TopOnly));
    }

    /// Two tagged rooms sealed by one-sided walls, the only lift line on their
    /// seam: no outside neighbor, so the merged `low` is the members' own
    /// floor and the platform is `Dead`.
    #[test]
    fn a_split_group_with_no_outside_neighbor_is_dead() {
        let r = report_of(&chain(&[128, 128], &[7, 7], &[(62, 7, false)], ""));
        assert_eq!(r.plats.len(), 1);
        assert_eq!(
            (
                r.plats[0].members.clone(),
                r.plats[0].rest,
                r.plats[0].travel,
                r.plats[0].refusal
            ),
            (vec![0, 1], Rest::Dead, 0, Some(Refusal::Dead))
        );
    }

    /// The merged platform is refused on the tag's lines and on its rest like
    /// any platform: one-shot or two-speed lines, a non-lift special on the
    /// tag, an outside neighbor more than a step above it, and outside
    /// neighbors at two floors both more than a step below it — rooms 0 and 3
    /// at 0 and 32 under a pair at 128, each calling the pair from its own
    /// side, so the merged platform rests `AboveAll` over two distinct floors
    /// and no shape names it.
    #[test]
    fn a_split_group_is_refused_on_its_lines_and_its_rest_like_any_platform() {
        let base = |links: &[(i32, i32, bool)], extra: &str| {
            chain(&[0, 128, 128, 128], &[0, 7, 7, 0], links, extra)
        };
        let one_shot = base(&[(21, 7, false), (0, 0, false), (0, 0, false)], "");
        let mixed = base(&[(62, 7, false), (0, 0, false), (123, 7, true)], "");
        let mut conflicting = base(&[(62, 7, false), (0, 0, false), (0, 0, false)], "");
        far_wall(&mut conflicting, 4, 23, 7);
        let intermediate = chain(
            &[0, 128, 128, 256],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        );
        let two_floors_below = chain(
            &[0, 128, 128, 32],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (62, 7, true)],
            "",
        );
        for (text, expected, rest) in [
            (one_shot, Refusal::OneShot, Rest::Top),
            (mixed, Refusal::MixedSpeed, Rest::Top),
            (conflicting, Refusal::ConflictingAction, Rest::Top),
            (intermediate, Refusal::UnsupportedRest, Rest::Intermediate),
            (two_floors_below, Refusal::UnsupportedRest, Rest::AboveAll),
        ] {
            let r = report_of(&text);
            assert_eq!(r.plats.len(), 1, "{expected:?}");
            assert_eq!(
                (r.plats[0].refusal, r.plats[0].rest),
                (Some(expected), rest)
            );
            assert_eq!(r.plats[0].members, vec![1, 2], "{expected:?}");
        }
    }

    /// One map with an ordinary bank (tag 7, members 1 and 3 around an
    /// untagged room, switch on 0–1) and a split group (tag 9, rooms 6 and 7,
    /// switch on 5–6): the bank is still judged member by member and its
    /// uncalled member refused `BankCaller`; the split group is one lift; the
    /// report stays ascending by sector with the merged entry at its least
    /// member.
    #[test]
    fn a_bank_and_a_split_group_on_one_map_are_each_judged_their_own_way() {
        let r = report_of(&chain(
            &[0, 128, 128, 128, 0, 0, 128, 128, 128],
            &[0, 7, 0, 7, 0, 0, 9, 9, 0],
            &[
                (62, 7, false),
                (0, 0, false),
                (0, 0, false),
                (0, 0, false),
                (0, 0, false),
                (62, 9, false),
                (0, 0, false),
                (0, 0, false),
            ],
            "",
        ));
        let summary: Vec<(usize, Vec<usize>, Option<Refusal>)> = r
            .plats
            .iter()
            .map(|p| (p.sector, p.members.clone(), p.refusal))
            .collect();
        assert_eq!(
            summary,
            vec![
                (1, vec![1], None),
                (3, vec![3], Some(Refusal::BankCaller)),
                (6, vec![6, 7], None),
            ]
        );
        assert_eq!(
            (
                r.counts.plats,
                r.counts.lifts,
                r.counts.bank_caller,
                r.counts.shared_split,
                r.counts.split_members
            ),
            (3, 2, 1, 1, 2)
        );
    }

    /// Rooms `0 | 1 | 2 | 3` with floors `128, 128, 128, 0`, tag 7 on 1 and
    /// 2, the riser switch on the 2–3 line fronted by room 3. The drop side
    /// (room 3, floor 0) and the only caller touch member 2 only: member 1's
    /// neighbors are rooms 0 and 2, both at its own floor, so its own `low`
    /// is 128 and alone it would be `Dead`. The merged `low` is the least of
    /// every member's, not the least member's, so the pair rests level with
    /// room 0 and drops 128 to room 3: a lift.
    #[test]
    fn a_split_lift_whose_drop_touches_only_the_later_member_is_still_a_lift() {
        let r = report_of(&chain(
            &[128, 128, 128, 0],
            &[0, 7, 7, 0],
            &[(0, 0, false), (0, 0, false), (62, 7, true)],
            "",
        ));
        assert_eq!(r.plats.len(), 1);
        let p = &r.plats[0];
        assert_eq!(
            (p.members.clone(), p.rest, p.travel, p.shape, p.refusal),
            (vec![1, 2], Rest::Top, 128, Some(Shape::Lift), None)
        );
    }

    /// Rooms `0 | 1 | 2 | 3 | 4 | 5` with floors `0, 128, 128, 128, 0, 0`,
    /// tag 7 on 1 and 2, and the only riser switch on room 5's far wall — a
    /// Low activator (floor 0 against 128) that neighbors no member. A
    /// single-tag platform called remotely is accepted (`KNOWN-GAPS.md`, "a
    /// remote switch is not expressible", is a writing-side gap, not a
    /// reading-side refusal), and the merged subject is `shared: false`: the
    /// group is the whole tag, so there is no other member for the bank's
    /// lines to be on and it is never `BankCaller`, however far its caller
    /// stands.
    #[test]
    fn a_split_group_called_only_from_afar_is_a_lift_not_a_bank_caller() {
        let mut text = chain(
            &[0, 128, 128, 128, 0, 0],
            &[0, 7, 7, 0, 0, 0],
            &[(0, 0, false); 5],
            "",
        );
        far_wall(&mut text, 6, 62, 7);
        let r = report_of(&text);
        assert_eq!(r.plats.len(), 1);
        assert!(r.plats[0].callable.low);
        assert_eq!(
            (r.plats[0].shape, r.plats[0].refusal, r.counts.bank_caller),
            (Some(Shape::Lift), None, 0)
        );
    }

    /// Two tagged pairs in a row `0 | 1 | 2 | 3 | 4 | 5 | 6` with floors
    /// `0, 128, 128, 0, 128, 128, 0`: tag 7 on 1, 2, 4 and 5, all at one
    /// floor, an untagged low room between the pairs, and a riser switch on
    /// each pair's west face. The walk over two-sided boundaries from the
    /// least member reaches 2 and stops, so the tag is not a split group and
    /// every member is judged alone. This pins today's behavior — the walk
    /// must reach every member from one seed — not a ruling: judging each
    /// connected component on its own is an open design question for a
    /// follow-up issue.
    #[test]
    fn two_split_pairs_on_one_tag_are_not_one_platform() {
        let r = report_of(&chain(
            &[0, 128, 128, 0, 128, 128, 0],
            &[0, 7, 7, 0, 7, 7, 0],
            &[
                (62, 7, false),
                (0, 0, false),
                (0, 0, false),
                (62, 7, false),
                (0, 0, false),
                (0, 0, false),
            ],
            "",
        ));
        assert_eq!(r.counts.shared_split, 0);
        assert_eq!(r.plats.len(), 4);
        assert!(r.plats.iter().all(|p| p.members == vec![p.sector]));
    }

    /// A(0)|T1(128)|T2(128)|B(128), tag 7 on T1 and T2, and a walkover (88,
    /// `WR Lift`) on A|T1 in place of the riser switch. A walkover fires from
    /// whichever side can cross the line at rest (`check::plats`'s module
    /// doc): A cannot step 128 up onto T1, so only T1 fires it — a member,
    /// hence `Plat` against the merged platform — and the pair is `TopOnly`
    /// exactly as a single platform with a walkover on its low face is
    /// (`callable_classes_and_the_top_trigger_are_reported`). The merge adds
    /// no caller the geometry does not have.
    #[test]
    fn a_walkover_on_a_split_groups_low_face_fires_from_the_platform_alone() {
        let r = report_of(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(88, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!(r.plats.len(), 1);
        assert_eq!(r.plats[0].members, vec![1, 2]);
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
            (r.plats[0].shape, r.plats[0].refusal),
            (None, Some(Refusal::TopOnly))
        );
    }

    #[test]
    fn things_on_a_split_group_are_summed() {
        let r = report_of(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "thing { x = 192.0; y = 64.0; angle = 0; type = 2012; single = true; }\n\
             thing { x = 320.0; y = 64.0; angle = 0; type = 2012; single = true; }\n",
        ));
        assert_eq!((r.plats[0].things, r.counts.with_things), (2, 1));
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
        // `a` is the merged split group of `a_split_group_is_read_as_one_platform`:
        // one accepted lift, callable from below, comprising two sectors.
        let a = report_of(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
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
        assert_eq!((sum.split_members, sum.shared_split), (2, 1));
        assert_eq!(sum.refusals(), 0);
        assert_eq!(PlatCounts::default().add(&b), b);
        let c = PlatCounts {
            split_members: 2,
            shared_split: 1,
            ..PlatCounts::default()
        };
        assert_eq!((c.add(&c).split_members, c.add(&c).shared_split), (4, 2));
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
        assert_eq!(json["plats"][0]["members"], serde_json::json!([1]));
        assert_eq!(json["counts"]["split_members"], 0);

        let r = report_of(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let json = serde_json::to_value(&r).expect("serializes");
        assert_eq!(json["plats"][0]["refusal"], "dead");
        assert_eq!(json["counts"]["dead"], 1);

        // The bank refusal's own wire name, which the corpus report row and
        // the JSON record both key on.
        let r = report_of(&bank_caller_case());
        let json = serde_json::to_value(&r).expect("serializes");
        assert_eq!(json["plats"][0]["refusal"], "bank_caller");
        assert_eq!(json["counts"]["bank_caller"], 2);

        // A merged split group: its member list and the two group counts.
        let r = report_of(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        let json = serde_json::to_value(&r).expect("serializes");
        assert_eq!(json["plats"][0]["members"], serde_json::json!([1, 2]));
        assert_eq!(
            (
                json["counts"]["shared_split"].as_u64(),
                json["counts"]["split_members"].as_u64()
            ),
            (Some(1), Some(2))
        );
    }

    /// Every shared-tag fixture the tests above build: the split pairs read
    /// as one platform (the one a non-lift special conflicts with and the one
    /// carrying things among them), the pairs the merge declines, the remotely
    /// called pair, the two pairs on one tag, the map with a bank and a split
    /// group side by side, the triangle, and [`bank_fixtures`]. A new split or
    /// bank fixture belongs here too, so `every_report_keeps_the_member_invariants`
    /// sweeps it.
    fn shared_tag_fixtures() -> Vec<String> {
        let split =
            |links: &[(i32, i32, bool)]| chain(&[0, 128, 128, 128], &[0, 7, 7, 0], links, "");
        let mut conflicting = split(&[(62, 7, false), (0, 0, false), (0, 0, false)]);
        far_wall(&mut conflicting, 4, 23, 7);
        let with_things = chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "thing { x = 192.0; y = 64.0; angle = 0; type = 2012; single = true; }\n\
             thing { x = 320.0; y = 64.0; angle = 0; type = 2012; single = true; }\n",
        );
        let mut remote = chain(
            &[0, 128, 128, 128, 0, 0],
            &[0, 7, 7, 0, 0, 0],
            &[(0, 0, false); 5],
            "",
        );
        far_wall(&mut remote, 6, 62, 7);
        let mut fixtures = vec![
            split(&[(62, 7, false), (0, 0, false), (0, 0, false)]),
            split(&[(0, 0, false), (62, 7, true), (0, 0, false)]),
            split(&[(88, 7, false), (0, 0, false), (0, 0, false)]),
            split(&[(21, 7, false), (0, 0, false), (0, 0, false)]),
            split(&[(62, 7, false), (0, 0, false), (123, 7, true)]),
            conflicting,
            with_things,
            chain(
                &[0, 128, 128, 0],
                &[0, 7, 7, 0],
                &[(62, 7, false), (0, 0, false), (0, 0, false)],
                "",
            ),
            chain(
                &[0, 128, 128, 0],
                &[0, 7, 7, 0],
                &[(62, 7, false), (0, 0, false), (62, 7, true)],
                "",
            ),
            chain(
                &[0, 128, 128, 32],
                &[0, 7, 7, 0],
                &[(62, 7, false), (0, 0, false), (62, 7, true)],
                "",
            ),
            chain(
                &[128, 128, 128, 0],
                &[0, 7, 7, 0],
                &[(0, 0, false), (0, 0, false), (62, 7, true)],
                "",
            ),
            chain(&[128, 128], &[7, 7], &[(62, 7, false)], ""),
            chain(
                &[128, 0, 128],
                &[7, 0, 7],
                &[(62, 7, false), (0, 0, false)],
                "",
            ),
            chain(
                &[0, 128, 96, 0],
                &[0, 7, 7, 0],
                &[(62, 7, false), (0, 0, false), (0, 0, false)],
                "",
            ),
            chain(
                &[0, 128, 128, 0, 128, 128, 0],
                &[0, 7, 7, 0, 7, 7, 0],
                &[
                    (62, 7, false),
                    (0, 0, false),
                    (0, 0, false),
                    (62, 7, false),
                    (0, 0, false),
                    (0, 0, false),
                ],
                "",
            ),
            chain(
                &[0, 128, 128, 128, 0, 0, 128, 128, 128],
                &[0, 7, 0, 7, 0, 0, 9, 9, 0],
                &[
                    (62, 7, false),
                    (0, 0, false),
                    (0, 0, false),
                    (0, 0, false),
                    (0, 0, false),
                    (62, 9, false),
                    (0, 0, false),
                    (0, 0, false),
                ],
                "",
            ),
            remote,
            TRIANGLE.to_owned(),
        ];
        fixtures.extend(bank_fixtures());
        fixtures
    }

    /// The shared-tag fixtures that are banks rather than split groups, as
    /// the tests above build them: the pair with one `BankCaller` member, the
    /// neighbor-called pair, and the remotely switched pair with a conflicting
    /// action. They matter to [`shared_tag_fixtures`]'s sweep because the
    /// counts identity must hold where `shared_split == 0` and
    /// `bank_caller > 0`.
    fn bank_fixtures() -> Vec<String> {
        vec![
            chain(
                &[0, 128, 128, 128, 0],
                &[0, 7, 0, 7, 0],
                &[(62, 7, false), (0, 0, false), (0, 0, false), (0, 0, false)],
                "",
            ),
            chain(
                &[0, 128, 0, 128],
                &[0, 7, 0, 7],
                &[(62, 7, false), (0, 0, false), (62, 7, false)],
                "",
            ),
            bank_caller_case(),
        ]
    }

    /// Over every fixture of [`shared_tag_fixtures`]: every entry's `members`
    /// is non-empty, strictly ascending and led by `sector`; a merged entry
    /// lists the whole tag, so its member count is the `shared_tag` of its
    /// members in the resolver's output; and the counts reconcile with that
    /// output — `plats + split_members - shared_split` is the number of
    /// sectors some lift line names.
    #[test]
    fn every_report_keeps_the_member_invariants() {
        for (i, text) in shared_tag_fixtures().iter().enumerate() {
            let (scene, tables) = scene_of(text);
            let r = recognize(&scene, &tables);
            let resolved = resolve_plats(&scene, &tables);
            for p in &r.plats {
                assert!(!p.members.is_empty(), "fixture {i}: empty members");
                assert_eq!(
                    p.sector, p.members[0],
                    "fixture {i}: sector is the least member"
                );
                assert!(
                    p.members.windows(2).all(|w| w[0] < w[1]),
                    "fixture {i}: members not strictly ascending: {:?}",
                    p.members
                );
                if p.members.len() > 1 {
                    let first = resolved
                        .iter()
                        .find(|q| q.sector == p.members[0])
                        .expect("a member is a resolved platform");
                    assert_eq!(
                        p.members.len(),
                        first.shared_tag,
                        "fixture {i}: a split group is the whole tag"
                    );
                }
            }
            assert_eq!(
                r.counts.plats + r.counts.split_members - r.counts.shared_split,
                u64::try_from(resolved.len()).expect("fits u64"),
                "fixture {i}: the counts reconcile with the per-sector resolution"
            );
        }
    }

    // What the split fixtures above do not cover: a group of more than three
    // members (the triangle is the largest); `island == true` on a merged
    // platform, which needs non-chain geometry whose every outer line is
    // two-sided (every fixture here has one-sided outer walls); and a `fast`
    // merged platform (speed is read per line, the same way as for a single
    // platform, so `MixedSpeed` on a split group is the only speed cell pinned).
}
