//! The floor recognizer: classifies every sector a floor line names into the
//! three shapes the IR can state — a drop wall, a reveal, a bridge — or
//! refuses it with the reason it cannot be stated.
//!
//! Recognition, not approximation. Every engine-side reading here is
//! [`crate::check::floors`]'s, resolved once for the flood, the invariants,
//! the conformance rows and this module: which `EV_DoFloor` (or one-way
//! `EV_DoPlat`) type drives the target and where it sends it
//! ([`FloorAction::destination`]), what that move does to who can walk where
//! ([`EffectFacts`]), and which lines name it, from where and fired by whom
//! ([`FloorTrigger`]). This module reads no geometry of its own; it judges
//! that resolution against what the IR can carry.
//!
//! **Shape.** An unrefused target is one of three, read straight off
//! [`EffectFacts::opening`]: a [`Shape::DropWall`] (a sealed strip that
//! lowers and joins two areas), a [`Shape::Reveal`] (a sealed cell that
//! comes into reach — the monster closet, or the sunken pedestal with a
//! prize on it) and a [`Shape::Bridge`] (a pit strip that rises to the
//! walkway). Those are the corpus probe's definitions
//! (`examples/liftprobe`, `docs/measurements/floor-shapes-2026-09-02.md`),
//! measured over 9,443 targets in 1,282 idgames maps: **`Reveal` 3,157,
//! `DropWall` 2,544, `Bridge` 236, `LedgeLower` 17, anything else 0** — the
//! corpus carries no fourth opening shape. The three are the IR's
//! [`crate::ir::PortalKind::DropWall`], [`crate::ir::Reveal`] and
//! [`crate::ir::PortalKind::Bridge`]; [`OpeningShape::LedgeLower`] has no IR
//! construct to state it, so it is refused here
//! ([`Refusal::UnsupportedShape`]).
//!
//! **Refusal precedence.** A target is judged against the twelve per-target
//! [`Refusal`]s in one fixed order and the first that applies wins, so a
//! target wrong in more than one way reports its most fundamental reason
//! rather than an order-of-evaluation accident:
//!
//! 1. [`Refusal::Gun`] — some line naming it is a `P_ShootSpecialLine` form
//!    (`FloorForm::G1`). The IR has no gun trigger at all, and this is a
//!    fact about the lines that holds whatever the geometry.
//! 2. [`Refusal::Conflict`] — a non-floor special names the same tag
//!    ([`SceneFloor::other_actions`]), so something besides a floor action
//!    drives this sector. The corpus's commonest reason: 172 of the sample's
//!    590 refused maps.
//! 3. [`Refusal::TwoFamilies`] and 4. [`Refusal::Unresolved`] — whether
//!    there is *one* move to state at all, and whether its destination is
//!    known (`raiseToTexture` needs texture heights nothing here loads).
//! 5. [`Refusal::Dead`], 6. [`Refusal::Closing`], 7. [`Refusal::Mixed`],
//!    8. [`Refusal::Neutral`] and 9. [`Refusal::RiderLoses`] — what the move
//!    *does*, judged before who can call it, because a move the IR would not
//!    state is refused however it is fired. [`Effect::Dead`] leads: a floor
//!    that cannot move makes every later question moot.
//! 10. [`Refusal::NoActivator`] — no sector can fire any of its lines, so
//!     the move never happens.
//! 11. [`Refusal::UnsupportedShape`] — the opening is one no IR construct
//!     names ([`OpeningShape::LedgeLower`] or
//!     [`OpeningShape::OtherOpening`]), judged once the move is known to
//!     open something a player could call for.
//! 12. [`Refusal::NeighborsMover`] — last, because a target this module
//!     would otherwise accept is the only one for which "the sector next to
//!     it moves too" is the whole story. It is the chain the compiler
//!     refuses (rule P30); the measurement's §G puts it at one target in
//!     five.
//!
//! **What is not a refusal.** Three things a reader might expect to be one:
//!
//! - **A shared tag.** One trigger driving several targets is how the corpus
//!   builds a multi-sector wall or a row of bars — 1,511 tag groups over
//!   9,443 targets, 55 % of them spanning several floors. Each member is
//!   judged alone and [`FloorCounts::shared_tag_accepted`] counts the members
//!   of groups of two or more that pass, so a group is expressible exactly
//!   when every one of its members is (the measurement's gate G).
//! - **A remote trigger.** 79 % of floor triggers sit somewhere that is
//!   neither the target nor a neighbor of it, and a third are six or more
//!   rooms away: a floor trigger is *placed*, not attached.
//!   [`FloorCounts::remote_triggers`] measures it rather than gating on it.
//! - **A repeatable form.** A floor action is one-way — `T_MoveFloor`
//!   removes its thinker on `pastdest` — so an `SR`/`WR` line fires a move
//!   that has already happened, which changes nothing the IR must state.
//!   This is where the floor recognizer parts company with
//!   [`crate::lift::plat`], whose [`crate::lift::plat::Refusal::OneShot`]
//!   exists because a lift *returns*.
//!
//! A floor line that names no target at all — tag 0, or a tag no sector
//! answers to — is not a refused target but a broken line, listed in
//! [`FloorReport::broken_lines`] as [`broken_floor_lines`] reports it,
//! split by [`FloorCounts::tag_zero`] and [`FloorCounts::dangling`], and
//! counted by [`FloorCounts::refusals`] alongside the refused targets.
//!
//! **The price.** The measurement's gate G — no tag-0 or dangling floor
//! line, no gun line, every target a candidate, a shared tag accepted when
//! every member qualifies alone — lifts the sample's all-axes expressible
//! count from **114 / 1,282 (8.9 %)** to **122 (9.5 %)**. This module is
//! stricter than G by the two restrictions the measurement costed
//! separately: it refuses `LedgeLower` (17 targets) and the §G chain
//! (G + no chain: 121 maps). Its accepted set is therefore a subset of G's,
//! and its yield at most 121 maps.
//!
//! [`EffectFacts`]: crate::check::floors::EffectFacts
//! [`EffectFacts::opening`]: crate::check::floors::EffectFacts::opening
//! [`FloorTrigger`]: crate::check::floors::FloorTrigger

use std::collections::BTreeSet;

use crate::check::floors::{
    Destination, Effect, FloorAction, OpeningShape, Placement, Rider, SceneFloor,
    broken_floor_lines, resolve_floors,
};
use crate::check::scene::Scene;
use crate::tables::{FloorForm, Tables};

/// What an accepted floor target is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// [`OpeningShape::DropWall`]: a sealed strip that lowers and joins the
    /// areas on either side of it — the corpus's monster closet between two
    /// rooms.
    DropWall,
    /// [`OpeningShape::Reveal`]: a sealed cell no neighbor could enter that
    /// becomes standable once it has moved, joining nothing new — the closet
    /// with the monster inside it, or the pedestal that sinks to expose a
    /// prize.
    Reveal,
    /// [`OpeningShape::Bridge`]: a pit strip the player could already drop
    /// into, rising to the walkway's floor.
    Bridge,
}

/// Why a floor target cannot be expressed as a map-spec floor action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// Some line naming the target is a [`FloorForm::G1`] gun form. The IR
    /// states no gun trigger.
    Gun,
    /// A non-floor special names the target's tag
    /// ([`SceneFloor::other_actions`]) — a lift, a door, a ceiling or a
    /// light drives the same sector.
    Conflict,
    /// More than one engine type drives the target
    /// ([`SceneFloor::actions`]), so it is not one action to state.
    TwoFamilies,
    /// The destination is [`Destination::NeedsTexture`] (`raiseToTexture`),
    /// which needs texture heights this resolution does not load, so there
    /// is nothing classified to judge.
    Unresolved,
    /// [`Effect::Dead`]: the destination is the target's own floor, so the
    /// thinker runs and changes nothing.
    Dead,
    /// [`Effect::Closing`]: every neighbor's reach set shrinks or holds and
    /// at least one shrinks. The IR states openings, not closures.
    Closing,
    /// [`Effect::Mixed`]: some neighbor gains a destination and some
    /// neighbor loses one.
    Mixed,
    /// [`Effect::Neutral`] with no [`EffectFacts::opening`]: the floor moves
    /// and nobody's reach changes, and the target does not come into reach
    /// either — nothing for the IR to say.
    ///
    /// [`EffectFacts::opening`]: crate::check::floors::EffectFacts::opening
    Neutral,
    /// [`Effect::Opening`] whose [`Rider::Loses`]: the move opens a route for
    /// everyone else while stranding a player standing on the target.
    RiderLoses,
    /// No trigger of the target has an activator
    /// ([`FloorTrigger::activators`]): no sector can fire any line naming
    /// it, so the move never happens.
    ///
    /// [`FloorTrigger::activators`]: crate::check::floors::FloorTrigger::activators
    NoActivator,
    /// The opening is [`OpeningShape::LedgeLower`] or
    /// [`OpeningShape::OtherOpening`] — real, but no IR construct names it.
    UnsupportedShape,
    /// [`SceneFloor::borders_mover`]: a neighbor is itself a floor target, a
    /// lift platform or a door sector, so the target's destination may be
    /// computed from a floor that has since moved — the chain rule P30
    /// refuses.
    NeighborsMover,
}

/// One recognized floor target.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Floor {
    /// Declaration index of the target sector.
    pub sector: usize,
    /// The tag its floor lines name it by (never 0).
    pub tag: i32,
    /// Its floor at load, before anything fires.
    pub rest: i32,
    /// Where its one action sends it, when it has exactly one whose
    /// destination resolves; `None` for a target with none, with more than
    /// one, or with a [`Destination::NeedsTexture`].
    pub destination: Option<i32>,
    /// What it is, for a target the recognizer accepts; `None` for one it
    /// refuses.
    pub shape: Option<Shape>,
    /// Declaration indices of every floor line naming its tag.
    pub triggers: Vec<usize>,
    /// Whether some trigger is [`Placement::Remote`] — neither on the target
    /// nor on one of its neighbors. Reported as a statistic, never a gate.
    pub remote: bool,
    /// How many sectors carry the tag (1 = this one alone).
    pub shared_tag: usize,
    /// How many things stand in the target sector.
    pub things: u64,
    /// Why it is not expressible, if it is not.
    pub refusal: Option<Refusal>,
}

/// One count per predicate over a map's [`Floor`]s.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct FloorCounts {
    /// Targets recognized.
    pub targets: u64,
    /// Targets accepted as [`Shape::DropWall`].
    pub drop_walls: u64,
    /// Targets accepted as [`Shape::Reveal`].
    pub reveals: u64,
    /// Targets accepted as [`Shape::Bridge`].
    pub bridges: u64,
    /// Targets carrying any [`Refusal`].
    pub refused: u64,
    /// Broken floor lines carrying tag 0.
    pub tag_zero: u64,
    /// Broken floor lines whose tag names no sector.
    pub dangling: u64,
    /// Targets refused [`Refusal::Gun`].
    pub gun: u64,
    /// Targets refused [`Refusal::Conflict`].
    pub conflict: u64,
    /// Targets refused [`Refusal::TwoFamilies`].
    pub two_families: u64,
    /// Targets refused [`Refusal::Unresolved`].
    pub unresolved: u64,
    /// Targets refused [`Refusal::Dead`].
    pub dead: u64,
    /// Targets refused [`Refusal::Closing`].
    pub closing: u64,
    /// Targets refused [`Refusal::Mixed`].
    pub mixed: u64,
    /// Targets refused [`Refusal::Neutral`].
    pub neutral: u64,
    /// Targets refused [`Refusal::RiderLoses`].
    pub rider_loses: u64,
    /// Targets refused [`Refusal::NoActivator`].
    pub no_activator: u64,
    /// Targets refused [`Refusal::UnsupportedShape`].
    pub unsupported_shape: u64,
    /// Targets refused [`Refusal::NeighborsMover`].
    pub neighbors_mover: u64,
    /// Floor lines that name no target at all
    /// ([`FloorReport::broken_lines`]) — the sum of [`Self::tag_zero`] and
    /// [`Self::dangling`].
    pub broken_lines: u64,
    /// Members of shared-tag groups (two or more sectors on one tag) the
    /// recognizer accepts. A count of *targets*, not of groups: a group is
    /// expressible exactly when this counts every one of its members.
    pub shared_tag_accepted: u64,
    /// Triggers sitting [`Placement::Remote`] from the target they drive,
    /// counted once per `(target, line)` pair — one line naming a shared tag
    /// is a remote trigger of each target it drives.
    pub remote_triggers: u64,
}

impl FloorCounts {
    /// Everything the recognizer refused: the refused targets plus the floor
    /// lines that name no target at all.
    #[must_use]
    pub fn refusals(&self) -> u64 {
        self.refused + self.broken_lines
    }

    /// Field-wise sum, for rolling per-map counts into a corpus total.
    ///
    /// Saturating rather than wrapping, for the reason
    /// [`crate::lift::teleport::TeleportCounts::add`] gives: a corpus large
    /// enough to overflow a `u64` target count cannot exist, and a pinned
    /// ceiling is a better report than a wrapped one if that ever stops
    /// being true.
    #[must_use]
    pub fn add(&self, other: &Self) -> Self {
        Self {
            targets: self.targets.saturating_add(other.targets),
            drop_walls: self.drop_walls.saturating_add(other.drop_walls),
            reveals: self.reveals.saturating_add(other.reveals),
            bridges: self.bridges.saturating_add(other.bridges),
            refused: self.refused.saturating_add(other.refused),
            tag_zero: self.tag_zero.saturating_add(other.tag_zero),
            dangling: self.dangling.saturating_add(other.dangling),
            gun: self.gun.saturating_add(other.gun),
            conflict: self.conflict.saturating_add(other.conflict),
            two_families: self.two_families.saturating_add(other.two_families),
            unresolved: self.unresolved.saturating_add(other.unresolved),
            dead: self.dead.saturating_add(other.dead),
            closing: self.closing.saturating_add(other.closing),
            mixed: self.mixed.saturating_add(other.mixed),
            neutral: self.neutral.saturating_add(other.neutral),
            rider_loses: self.rider_loses.saturating_add(other.rider_loses),
            no_activator: self.no_activator.saturating_add(other.no_activator),
            unsupported_shape: self
                .unsupported_shape
                .saturating_add(other.unsupported_shape),
            neighbors_mover: self.neighbors_mover.saturating_add(other.neighbors_mover),
            broken_lines: self.broken_lines.saturating_add(other.broken_lines),
            shared_tag_accepted: self
                .shared_tag_accepted
                .saturating_add(other.shared_tag_accepted),
            remote_triggers: self.remote_triggers.saturating_add(other.remote_triggers),
        }
    }
}

/// What [`recognize`] says about one map's floor actions.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct FloorReport {
    /// One entry per sector some floor line names, ascending by sector.
    pub floors: Vec<Floor>,
    /// Floor lines that name no target at all, ascending by declaration
    /// index ([`broken_floor_lines`]).
    pub broken_lines: Vec<usize>,
    /// The census of [`Self::floors`] and [`Self::broken_lines`].
    pub counts: FloorCounts,
}

/// Classifies every sector a floor line names in `scene`.
///
/// # Panics
///
/// If a target's thing count or a tally over the scene does not fit a `u64`,
/// which needs more elements than a parsed map can hold (`usize` is at most
/// 64 bits on every platform this crate builds on).
#[must_use]
pub fn recognize(scene: &Scene, tables: &Tables) -> FloorReport {
    let resolved = resolve_floors(scene, tables);
    let mut floors = Vec::with_capacity(resolved.len());
    for f in &resolved {
        let refusal = refusal_of(f);
        let single = f.single();
        floors.push(Floor {
            sector: f.sector,
            tag: f.tag,
            rest: f.rest,
            destination: single.and_then(|a| match a.destination {
                Destination::Height(h) => Some(h),
                Destination::NeedsTexture => None,
            }),
            shape: if refusal.is_none() {
                shape_of(single)
            } else {
                None
            },
            triggers: f.triggers.iter().map(|t| t.linedef).collect(),
            remote: f.triggers.iter().any(|t| t.placement == Placement::Remote),
            shared_tag: f.shared_tag,
            things: u64::try_from(
                scene
                    .things
                    .iter()
                    .filter(|t| t.sector == Some(f.sector))
                    .count(),
            )
            .expect("fits u64"),
            refusal,
        });
    }
    let broken_lines = broken_floor_lines(scene, tables);
    let counts = count(&floors, &resolved, &broken_lines, scene);
    FloorReport {
        floors,
        broken_lines,
        counts,
    }
}

/// The first [`Refusal`] that applies to `f`, in the module doc's order.
fn refusal_of(f: &SceneFloor) -> Option<Refusal> {
    if f.triggers.iter().any(|t| t.form == FloorForm::G1) {
        return Some(Refusal::Gun);
    }
    if !f.other_actions.is_empty() {
        return Some(Refusal::Conflict);
    }
    let Some(a) = f.single() else {
        return Some(Refusal::TwoFamilies);
    };
    let Some(facts) = &a.facts else {
        return Some(Refusal::Unresolved);
    };
    match (facts.effect, facts.rider, facts.opening) {
        (Effect::Dead, ..) => return Some(Refusal::Dead),
        (Effect::Closing, ..) => return Some(Refusal::Closing),
        (Effect::Mixed, ..) => return Some(Refusal::Mixed),
        (Effect::Neutral, _, None) => return Some(Refusal::Neutral),
        (Effect::Opening, Rider::Loses, _) => return Some(Refusal::RiderLoses),
        _ => {}
    }
    if !f.triggers.iter().any(|t| !t.activators.is_empty()) {
        return Some(Refusal::NoActivator);
    }
    match facts.opening {
        Some(OpeningShape::DropWall | OpeningShape::Reveal | OpeningShape::Bridge) => {}
        _ => return Some(Refusal::UnsupportedShape),
    }
    if f.borders_mover {
        return Some(Refusal::NeighborsMover);
    }
    None
}

/// The [`Shape`] of an unrefused target, read off its one action's
/// [`EffectFacts::opening`]. The three arms `refusal_of` lets through are the
/// three the IR states; anything else is [`Refusal::UnsupportedShape`] and
/// never reaches here.
///
/// [`EffectFacts::opening`]: crate::check::floors::EffectFacts::opening
fn shape_of(single: Option<&FloorAction>) -> Option<Shape> {
    match single
        .and_then(|a| a.facts.as_ref())
        .and_then(|f| f.opening)
    {
        Some(OpeningShape::DropWall) => Some(Shape::DropWall),
        Some(OpeningShape::Reveal) => Some(Shape::Reveal),
        Some(OpeningShape::Bridge) => Some(Shape::Bridge),
        _ => None,
    }
}

/// How many of `broken_lines` carry tag 0 rather than a tag naming no
/// sector, read back off the scene's front mirrors — the same walk
/// `broken_floor_lines` built the list from, so every entry is found.
fn tag_zero_lines(scene: &Scene, broken_lines: &[usize]) -> u64 {
    let broken: BTreeSet<usize> = broken_lines.iter().copied().collect();
    u64::try_from(
        scene
            .sectors
            .iter()
            .flat_map(|s| s.boundary.iter())
            .filter(|b| b.fronts_this && b.tag == 0 && broken.contains(&b.linedef))
            .count(),
    )
    .expect("fits u64")
}

/// Counts `floors` one field per predicate. `resolved` is the resolution
/// `floors` was built from, read for the per-trigger placements a [`Floor`]
/// summarizes into one flag; `broken_lines` is split into tag-0 and dangling
/// against `scene`.
fn count(
    floors: &[Floor],
    resolved: &[SceneFloor],
    broken_lines: &[usize],
    scene: &Scene,
) -> FloorCounts {
    let tag_zero = tag_zero_lines(scene, broken_lines);
    let total_broken = u64::try_from(broken_lines.len()).expect("fits u64");
    let mut c = FloorCounts {
        tag_zero,
        dangling: total_broken - tag_zero,
        broken_lines: total_broken,
        remote_triggers: u64::try_from(
            resolved
                .iter()
                .flat_map(|f| f.triggers.iter())
                .filter(|t| t.placement == Placement::Remote)
                .count(),
        )
        .expect("fits u64"),
        ..FloorCounts::default()
    };
    for f in floors {
        c.targets += 1;
        match f.shape {
            Some(Shape::DropWall) => c.drop_walls += 1,
            Some(Shape::Reveal) => c.reveals += 1,
            Some(Shape::Bridge) => c.bridges += 1,
            None => {}
        }
        match f.refusal {
            Some(Refusal::Gun) => c.gun += 1,
            Some(Refusal::Conflict) => c.conflict += 1,
            Some(Refusal::TwoFamilies) => c.two_families += 1,
            Some(Refusal::Unresolved) => c.unresolved += 1,
            Some(Refusal::Dead) => c.dead += 1,
            Some(Refusal::Closing) => c.closing += 1,
            Some(Refusal::Mixed) => c.mixed += 1,
            Some(Refusal::Neutral) => c.neutral += 1,
            Some(Refusal::RiderLoses) => c.rider_loses += 1,
            Some(Refusal::NoActivator) => c.no_activator += 1,
            Some(Refusal::UnsupportedShape) => c.unsupported_shape += 1,
            Some(Refusal::NeighborsMover) => c.neighbors_mover += 1,
            None => {}
        }
        c.refused += u64::from(f.refusal.is_some());
        c.shared_tag_accepted += u64::from(f.shared_tag >= 2 && f.refusal.is_none());
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::fixtures::{chain, chain_full, far_wall, scene_of};

    /// Parses `text`, builds its [`Scene`] and recognizes its floor targets.
    fn report_of(text: &str) -> FloorReport {
        let (scene, tables) = scene_of(text);
        recognize(&scene, &tables)
    }

    /// A target with three two-sided neighbors, which `chain`'s row of rooms
    /// cannot build: `T` (sector 1, tagged) spans `x ∈ [128, 256]`,
    /// `y ∈ [0, 128]`, with `W` (sector 0) west of it, `E` (sector 2) east
    /// and `N` (sector 3) north. `floors` are `[W, T, E, N]`; every ceiling
    /// is 256, and every linedef is wound so its own sector lies to the
    /// right of `v1 -> v2`. The `special`/`tag` pair goes on `E`'s outer
    /// east wall — the "switch on the far wall" placement of the
    /// floor-shape worked examples, here two rooms from `T`.
    ///
    /// Three neighbors is the smallest hub in which one move can open a
    /// route while closing another ([`Refusal::Mixed`]) or strand its rider
    /// while opening a route for everyone else ([`Refusal::RiderLoses`]).
    /// With two neighbors both are unreachable: the only pair through the
    /// target is `(A, B)` and its reverse, and every way of making one of
    /// them new makes the other new too.
    fn hub(floors: [i32; 4], special: i32, tag: i32) -> String {
        use std::fmt::Write as _;

        let mut text = String::from("namespace = \"doom\";\n");
        for (x, y) in [
            (0, 0),
            (0, 128),
            (128, 0),
            (128, 128),
            (256, 0),
            (256, 128),
            (384, 0),
            (384, 128),
            (128, 256),
            (256, 256),
        ] {
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = {y}.000; }}");
        }
        let mut sidedefs = String::new();
        let mut next = 0usize;
        let mut side = |sidedefs: &mut String, sector: usize, two_sided: bool| {
            let tex = if two_sided {
                "texturemiddle = \"-\"; texturebottom = \"SUPPORT3\";"
            } else {
                "texturemiddle = \"STARTAN2\";"
            };
            let _ = writeln!(sidedefs, "sidedef {{ sector = {sector}; {tex} }}");
            next += 1;
            next - 1
        };
        // The three two-sided links, as (v1, v2, front sector, back sector):
        // W|T, T|E and T|N.
        for (v1, v2, front, back) in [(3, 2, 0, 1), (5, 4, 1, 2), (3, 5, 1, 3)] {
            let sf = side(&mut sidedefs, front, true);
            let sb = side(&mut sidedefs, back, true);
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; sideback = {sb}; \
                 twosided = true; }}"
            );
        }
        // The one-sided walls; the seventh is `E`'s east wall, which carries
        // the trigger.
        for (v1, v2, sector, trigger) in [
            (0, 1, 0, false),
            (1, 3, 0, false),
            (2, 0, 0, false),
            (4, 2, 1, false),
            (6, 4, 2, false),
            (5, 7, 2, false),
            (7, 6, 2, true),
            (3, 8, 3, false),
            (8, 9, 3, false),
            (9, 5, 3, false),
        ] {
            let sf = side(&mut sidedefs, sector, false);
            let action = if trigger {
                format!(" special = {special}; arg0 = {tag};")
            } else {
                String::new()
            };
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; blocking = true;{action} }}"
            );
        }
        text.push_str(&sidedefs);
        for (i, floor) in floors.into_iter().enumerate() {
            let id = if i == 1 { tag } else { 0 };
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; \
                 heightfloor = {floor}; heightceiling = 256; lightlevel = 160; id = {id}; }}"
            );
        }
        text
    }

    #[test]
    fn the_three_shapes_are_recognized_and_a_shared_tag_is_accepted_member_by_member() {
        // Two drop walls on one tag between three rooms: A(0)–T1(128)–B(0)–T2(128)–C(0), one 23 S1.
        let mut text = chain(
            &[0, 128, 0, 128, 0],
            &[0, 7, 0, 7, 0],
            &[(0, 0, false); 4],
            "",
        );
        far_wall(&mut text, 5, 23, 7);
        let r = report_of(&text);
        assert_eq!(r.counts.refusals(), 0, "{:?}", r.floors);
        assert_eq!(
            (
                r.counts.targets,
                r.counts.drop_walls,
                r.counts.shared_tag_accepted
            ),
            (2, 2, 2)
        );
        assert!(
            r.floors
                .iter()
                .all(|f| f.shape == Some(Shape::DropWall) && f.shared_tag == 2)
        );

        // A sealed one-neighbor cell with no headroom, lowered flush: the
        // reveal.
        let mut text = chain_full(&[0, 64], &[256, 64], &[0, 7], &[(0, 0, false)], "");
        far_wall(&mut text, 2, 23, 7);
        let r = report_of(&text);
        assert_eq!(
            (r.floors[0].shape, r.floors[0].refusal),
            (Some(Shape::Reveal), None)
        );
        assert_eq!((r.counts.reveals, r.counts.targets), (1, 1));

        // A pit strip between two walkways, raised to their floor: the
        // bridge.
        let mut text = chain(
            &[64, 0, 64],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 20, 7);
        let r = report_of(&text);
        assert_eq!(
            (r.floors[0].shape, r.floors[0].destination),
            (Some(Shape::Bridge), Some(64))
        );
        assert_eq!((r.counts.bridges, r.counts.refusals()), (1, 0));
    }

    #[test]
    fn refusals_follow_the_fixed_order() {
        // 101 S1 pillar: Closing.
        let mut text = chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        far_wall(&mut text, 3, 101, 7);
        assert_eq!(report_of(&text).floors[0].refusal, Some(Refusal::Closing));
        // A gun form beats everything.
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 24, 7);
        assert_eq!(report_of(&text).floors[0].refusal, Some(Refusal::Gun));
        // A lift special on the tag is a conflict even for a fine drop wall.
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        assert_eq!(report_of(&text).floors[0].refusal, Some(Refusal::Conflict));
        // Dead: 18 with nothing above.
        let mut text = chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        far_wall(&mut text, 3, 18, 7);
        assert_eq!(report_of(&text).floors[0].refusal, Some(Refusal::Dead));
        // A repeatable form (60 SR) is accepted.
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 60, 7);
        assert_eq!(report_of(&text).floors[0].refusal, None);
    }

    #[test]
    fn broken_lines_count_as_refusals_and_a_remote_trigger_does_not() {
        let mut text = chain(
            &[0, 128, 0, 0],
            &[0, 7, 0, 0],
            &[(23, 0, false), (0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 4, 23, 7);
        let r = report_of(&text);
        assert_eq!(
            (
                r.counts.tag_zero,
                r.counts.broken_lines,
                r.counts.refusals()
            ),
            (1, 1, 1)
        );
        assert_eq!(r.floors[0].refusal, None);
        assert!(
            r.floors[0].remote,
            "the far-wall switch is two rooms from T"
        );
        assert_eq!(r.counts.remote_triggers, 1);
    }

    /// The seven refusals the fixed-order test does not reach, each on the
    /// geometry that earns it and nothing earlier in the order.
    #[test]
    fn every_other_refusal_reason_is_reachable() {
        // Two families on one tag: a 23 (lowerFloorToLowest) and an 18
        // (raiseFloorToNearest) both naming 7. Neither is a conflict — both
        // are floor specials — so the target is two actions, not one.
        let mut two_families = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        );
        far_wall(&mut two_families, 3, 18, 7);
        // `raiseToTexture` (30 W1): the destination needs texture heights
        // this resolution does not load, so nothing is classified.
        let unresolved = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(30, 7, false), (0, 0, false)],
            "",
        );
        // A descender whose neighbors' routes are unchanged and whose own
        // rider is stranded: Neutral wins, being judged before the rider.
        let mut neutral = chain(
            &[128, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut neutral, 3, 23, 7);
        // A 38 (W1) on a one-sided wall: `P_CrossSpecialLine` fires from a
        // side the player can cross from, and a one-sided line has none.
        let mut no_activator = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut no_activator, 3, 38, 7);
        // A ledge one step above its west neighbor, lowered flush: a real
        // opening, but `LedgeLower` is a shape no IR construct states.
        let mut ledge = chain(
            &[24, 48, 0, 0],
            &[0, 7, 0, 0],
            &[(0, 0, false), (0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut ledge, 4, 23, 7);
        // A fine drop wall whose east neighbor is a lift platform (tag 5).
        let chained = chain(
            &[0, 128, 0],
            &[0, 7, 5],
            &[(23, 7, false), (62, 5, false)],
            "",
        );

        let cases = [
            (two_families, Refusal::TwoFamilies),
            (unresolved, Refusal::Unresolved),
            (neutral, Refusal::Neutral),
            // W(100) – T(100) – E(0)/N(0), 23 S1: T drops flush with E and
            // N, newly joining them to each other, and loses its own step up
            // to W. Opening for everyone but the rider.
            (hub([100, 100, 0, 0], 23, 7), Refusal::RiderLoses),
            // W(0) – T(0) – E(30)/N(-10), 15 S1 (plat raiseAndChange +24):
            // the rise puts T within a step of E, newly joining W to it, and
            // out of N's reach.
            (hub([0, 0, 30, -10], 15, 7), Refusal::Mixed),
            (no_activator, Refusal::NoActivator),
            (ledge, Refusal::UnsupportedShape),
            (chained, Refusal::NeighborsMover),
        ];
        for (text, expected) in cases {
            let r = report_of(&text);
            let target = r
                .floors
                .iter()
                .find(|f| f.tag == 7)
                .unwrap_or_else(|| panic!("{expected:?}: no target on tag 7"));
            assert_eq!(target.refusal, Some(expected), "{:?}", r.floors);
            assert!(target.shape.is_none(), "a refused target names no shape");
        }
    }

    #[test]
    fn an_unresolved_destination_and_a_resolved_one_are_both_reported() {
        let r = report_of(&chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(30, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            (r.floors[0].destination, r.counts.unresolved),
            (None, 1),
            "raiseToTexture resolves no height"
        );
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let r = report_of(&text);
        assert_eq!(r.floors[0].destination, Some(0));
        assert_eq!(r.floors[0].rest, 128);
    }

    #[test]
    fn broken_lines_split_into_tag_zero_and_dangling() {
        let mut text = chain(
            &[0, 0, 0],
            &[0, 0, 0],
            &[(23, 0, false), (38, 9, false)],
            "",
        );
        far_wall(&mut text, 3, 0, 0);
        let r = report_of(&text);
        assert_eq!(r.broken_lines, vec![0, 1]);
        assert_eq!(
            (
                r.counts.tag_zero,
                r.counts.dangling,
                r.counts.broken_lines,
                r.counts.targets
            ),
            (1, 1, 2, 0)
        );
        assert_eq!(r.counts.refusals(), 2, "a broken line is a refusal");
    }

    #[test]
    fn things_standing_on_a_target_are_counted() {
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "thing { x = 192.0; y = 64.0; angle = 0; type = 2012; single = true; }\n",
        );
        far_wall(&mut text, 3, 23, 7);
        let r = report_of(&text);
        assert_eq!(r.floors[0].things, 1);
        assert_eq!(r.floors[0].triggers.len(), 1);
    }

    #[test]
    fn counts_add_field_wise() {
        // Four rooms, so the far wall is two rooms from the target and its
        // trigger counts as remote in both maps.
        let mut drop_wall = chain(&[0, 128, 0, 0], &[0, 7, 0, 0], &[(0, 0, false); 3], "");
        far_wall(&mut drop_wall, 4, 23, 7);
        let a = report_of(&drop_wall).counts;
        let mut dead = chain(&[0, 0, 0, 0], &[0, 7, 0, 0], &[(0, 0, false); 3], "");
        far_wall(&mut dead, 4, 18, 7);
        let b = report_of(&dead).counts;
        let sum = a.add(&b);
        assert_eq!(
            (
                sum.targets,
                sum.drop_walls,
                sum.dead,
                sum.refused,
                sum.remote_triggers
            ),
            (2, 1, 1, 1, 2)
        );
        assert_eq!(sum.refusals(), 1);
        assert_eq!(FloorCounts::default().add(&b), b);
    }

    #[test]
    fn a_report_serializes_to_json_with_snake_case_names() {
        let mut text = chain(
            &[64, 0, 64],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 20, 7);
        let json = serde_json::to_value(report_of(&text)).expect("serializes");
        assert_eq!(json["counts"]["bridges"], 1);
        assert_eq!(json["floors"][0]["shape"], "bridge");
        assert_eq!(json["floors"][0]["destination"], 64);
        assert_eq!(json["floors"][0]["refusal"], serde_json::Value::Null);
        assert_eq!(json["floors"][0]["remote"], serde_json::Value::Bool(false));

        let mut text = chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        far_wall(&mut text, 3, 101, 7);
        let json = serde_json::to_value(report_of(&text)).expect("serializes");
        assert_eq!(json["floors"][0]["refusal"], "closing");
        assert_eq!(json["floors"][0]["shape"], serde_json::Value::Null);
        assert_eq!(json["counts"]["closing"], 1);
    }
}
