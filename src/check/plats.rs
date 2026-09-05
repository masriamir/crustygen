//! Plats as the engine reads them, shared by the flood (V-P7), V-P5/V-P11,
//! the conformance rows and the `lift::plat` recognizer — one resolution so
//! four consumers cannot drift on `low`, rest or who can fire a trigger.
//!
//! Every value here is re-derived from a parsed [`Scene`] the way the engine
//! reads the same map:
//!
//! - **`low`** is `P_FindLowestFloorSurrounding` (`p_spec.c`), which starts
//!   at the platform's *own* floor and takes the minimum over its two-sided
//!   neighbors — so a platform with no lower neighbor travels 0 and
//!   `EV_DoPlat`'s `downWaitUpStay` (`p_plats.c`) is a no-op on it.
//! - **Who can fire a trigger** is the dispatch rule of the special itself:
//!   a use line fires from its front sector only (`P_UseSpecialLine`), a
//!   walkover from whichever side can cross the line at rest (`P_TryMove`'s
//!   step rule, applied to the two floors as they stand at load).
//! - **[`Activator`]** then places that firing sector relative to the
//!   platform at rest, which is what tells a caller apart from a rider.
//!
//! The classification rules are the lift shape probe's
//! (`examples/liftprobe/common.rs`, `docs/measurements/lift-shapes-2026-08-29.md`),
//! moved crate-side: the probe measures the idgames corpus with them and this
//! module judges an emitted map with them.
//!
//! **One deliberate divergence from the probe.** A walkover trigger is
//! credited here only across a boundary the player can actually cross
//! ([`Boundary::passable`]): `P_CrossSpecialLine` runs from `P_TryMove`'s
//! `spechit` bookkeeping, and `PIT_CheckLine` (pinned `p_map.c:211-217`)
//! returns *before* that list is ever populated (`p_map.c:240-244`) — for a
//! one-sided line unconditionally (`p_map.c:211-212`), and for an
//! `ML_BLOCKING` line whenever the moving thing is not a missile
//! (`p_map.c:214-217`). So a walkover lift line the player cannot walk
//! through fires nothing — the same gate `flood.rs` applies to walkover
//! exits and teleports. The probe's `activator_sides` omits it because it is
//! measuring what map authors *drew*, where crediting the side is the
//! conservative reading; a checker that must not call a map finishable it
//! cannot finish needs the engine's own answer instead.
//!
//! **A second divergence, same argument.** A walkover is credited only when
//! neither side of it is a dead-end pocket no deeper than the player's radius
//! (`dead_end_pocket`). `P_TryMove` fires a walkover from its `spechit`
//! walk, which asks whether the thing's *center* changed sides
//! (`P_PointOnLineSide (thing->x, thing->y, ld)`) — but it refuses the move
//! first, at `tmfloorz - thing->z > 24*FRACUNIT`, and `PIT_CheckLine` raises
//! `tmfloorz` for every line the moving *box* straddles, with
//! `P_BoxOnLineSide` (`p_maputl.c`) counting a box edge that merely touches a
//! line as straddling it. A pocket that shallow therefore admits no center,
//! so nobody crosses into it and nobody stands in it to cross out. The
//! probe's `activator_sides` has no such gate — it measures what authors drew
//! — and the depth alone would be the wrong test anyway: §G3 of the census
//! finds thin *open* trigger strips 16 deep that play perfectly, which is why
//! the closed-on-every-other-side half of the predicate is not optional.

use std::collections::BTreeSet;

use crate::check::scene::{Boundary, Scene};
use crate::tables::Tables;

/// The sector a player must stand in to fire a trigger, relative to the plat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Activator {
    /// More than a step below the plat's floor.
    Low,
    /// Within a step of it, not the plat itself.
    Level,
    /// The plat itself.
    Plat,
    /// More than a step above.
    Above,
}

/// Where a plat rests at load, relative to its two-sided neighbors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Rest {
    /// Travel 0: `P_FindLowestFloorSurrounding` returns its own floor.
    Dead,
    /// Some neighbor within a step: a landing the player walks onto.
    Top,
    /// Every neighbor more than a step below.
    AboveAll,
    /// Some neighbor more than a step above.
    Intermediate,
}

/// One line naming a plat's tag.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent fact the vocabulary states about this special \
              (use-activated, repeatable, fast) or an independent flag bit of the line itself \
              (lower_unpegged) — the same reasoning `check::scene::Boundary` gives for the \
              bitfield it mirrors"
)]
pub struct SceneTrigger {
    /// Declaration index of the triggering linedef.
    pub linedef: usize,
    /// The linedef's special.
    pub special: i32,
    /// `P_UseSpecialLine` (front side only) rather than `P_CrossSpecialLine`.
    pub use_line: bool,
    /// Whether the special is one of the repeatable (SR/WR) forms rather
    /// than a one-shot (S1/W1).
    pub repeatable: bool,
    /// Whether the special is one of the four `blazeDWUS` forms.
    pub fast: bool,
    /// Declaration index of the line's front sector.
    pub front: usize,
    /// Declaration index of its back sector, if two-sided.
    pub back: Option<usize>,
    /// Sectors from which the line fires, each classified relative to the plat.
    pub activators: Vec<(usize, Activator)>,
    /// Whether the line carries `ML_DONTPEGBOTTOM`. Cleared, a lower texture
    /// is anchored to the *back* sector's floor and so rides with whichever
    /// of the two sectors moves — which is why a lift riser wants the flag
    /// clear where a door track wants it set (`compile::lifts`' module doc).
    pub lower_unpegged: bool,
}

/// A sector some lift line names by tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenePlat {
    /// Declaration index of the platform sector.
    pub sector: usize,
    /// The tag lift lines name it by (never 0 — a tag-0 line names nothing).
    pub tag: i32,
    /// The floor `EV_DoPlat` sends it to.
    pub low: i32,
    /// How far it travels: its rest floor minus [`ScenePlat::low`].
    pub travel: i32,
    /// Where it rests relative to its neighbors.
    pub rest: Rest,
    /// Its two-sided neighbors, by declaration index.
    pub neighbors: BTreeSet<usize>,
    /// How many distinct floors those neighbors sit at.
    pub distinct_neighbor_floors: usize,
    /// How many sectors carry this tag (1 = this one alone).
    pub shared_tag: usize,
    /// Every lift line naming this plat's tag.
    pub triggers: Vec<SceneTrigger>,
    /// Non-lift, nonzero specials on lines naming the same tag, sorted and
    /// deduped: another action sharing the tag drives this sector too.
    pub other_actions: Vec<i32>,
}

impl ScenePlat {
    /// Whether some trigger fires from more than a step below the platform —
    /// the caller a lift exists for.
    #[must_use]
    pub fn callable_low(&self) -> bool {
        self.triggers
            .iter()
            .any(|t| t.activators.iter().any(|&(_, a)| a == Activator::Low))
    }

    /// Whether some trigger fires from the platform itself or from within a
    /// step of it — a rider's call, not a caller's.
    #[must_use]
    pub fn callable_top(&self) -> bool {
        self.triggers.iter().any(|t| {
            t.activators
                .iter()
                .any(|&(_, a)| matches!(a, Activator::Level | Activator::Plat))
        })
    }

    /// Neighbors that are `Low` activators of some trigger.
    #[must_use]
    pub fn low_activator_neighbors(&self) -> BTreeSet<usize> {
        self.triggers
            .iter()
            .flat_map(|t| t.activators.iter())
            .filter(|&&(s, a)| a == Activator::Low && self.neighbors.contains(&s))
            .map(|&(s, _)| s)
            .collect()
    }
}

/// Classifies `sector` as an activator of `plat`, `step` being the engine's
/// maximum step-up ([`Tables::step_height`]).
#[must_use]
pub fn classify(scene: &Scene, plat: usize, sector: usize, step: i32) -> Activator {
    if sector == plat {
        return Activator::Plat;
    }
    let (floor, plat_floor) = (scene.sectors[sector].floor, scene.sectors[plat].floor);
    if floor < plat_floor - step {
        Activator::Low
    } else if floor > plat_floor + step {
        Activator::Above
    } else {
        Activator::Level
    }
}

/// The four lift-special sets the resolution reads, widened to the `i32` a
/// [`Boundary`]'s own `special` carries and resolved once per
/// [`resolve_plats`] call rather than re-looked-up per line — the same shape
/// `scene::BoundaryFlagBits` uses for the flag bits it resolves once.
struct LiftSpecials {
    /// Every special `EV_DoPlat` dispatches a `downWaitUpStay`/`blazeDWUS`
    /// for ([`Tables::lift_specials`]).
    all: Vec<i32>,
    /// The use-activated subset (`P_UseSpecialLine`, front side only).
    use_line: Vec<i32>,
    /// The repeatable (SR/WR) subset.
    repeatable: Vec<i32>,
    /// The four `blazeDWUS` forms.
    fast: Vec<i32>,
}

impl LiftSpecials {
    /// Resolves all four sets from `tables`.
    fn resolve(tables: &Tables) -> Self {
        let widen = |s: &[u16]| s.iter().copied().map(i32::from).collect();
        Self {
            all: widen(&tables.lift_specials()),
            use_line: widen(&tables.lift_use_specials()),
            repeatable: widen(&tables.lift_repeatable_specials()),
            fast: widen(&tables.lift_fast_specials()),
        }
    }
}

/// Whether `sector` is a pocket behind `linedef` that no player center can
/// ever occupy: every other boundary of it is one they cannot pass, and no
/// point of it lies more than `radius` beyond that line.
///
/// Both halves are required, and the corpus is why. `P_TryMove` fires a
/// walkover from its `spechit` walk, comparing `P_PointOnLineSide (thing->x,
/// thing->y, ld)` before and after the move — the thing's **center** crosses,
/// not its box — but it returns first at `tmfloorz - thing->z > 24*FRACUNIT`,
/// and `PIT_CheckLine` has by then raised `tmfloorz` for every line the
/// **box** straddles. `PIT_CheckLine`'s own bounding-box early-out
/// (`p_map.c:191-195`, `<=`/`>=`) returns before `P_BoxOnLineSide` is asked
/// when the box merely touches a line, so the center may rest exactly
/// `radius` from a blocking edge but no closer. A pocket exactly `radius`
/// deep therefore puts the center exactly *on* its threshold, which
/// `P_PointOnLineSide` (`p_maputl.c:76-81`, `if (x <= line->v1->x)`)
/// groups with the near side — no side change, no crossing — and a
/// shallower pocket with no other way out admits no center at all. But
/// shallowness
/// alone proves nothing: the census's §G3 found 3 / 30 / 64 low-side
/// walkovers whose far sector reaches no further than 16, and all but 0 / 3 /
/// 6 of them are *open* — thin trigger strips cut across a corridor, which
/// the player crosses without ever being stopped, because the box may
/// legitimately overhang whatever lies past a passable boundary.
///
/// A two-sided boundary flagged `ML_BLOCKING` counts as unpassable here (the
/// engine refuses the move at `PIT_CheckLine`), which is one place this
/// differs from the probe's own `far_depth`, whose brief defined blocking as
/// one-sided-or-a-step only.
fn dead_end_pocket(scene: &Scene, linedef: usize, sector: usize, step: i32, radius: i32) -> bool {
    let ss = &scene.sectors[sector];
    // `sector`'s own mirror of the line, so walking `a` -> `b` always keeps
    // `sector` on the right and `(dy, -dx)` points into it.
    let Some(edge) = ss.boundary.iter().find(|b| b.linedef == linedef) else {
        return false;
    };
    let (dx, dy) = (edge.b.0 - edge.a.0, edge.b.1 - edge.a.1);
    let len = dx.hypot(dy);
    if len == 0.0 {
        return false;
    }
    let (nx, ny) = (dy / len, -dx / len);
    let perp = |p: (f64, f64)| (p.0 - edge.a.0) * nx + (p.1 - edge.a.1) * ny;
    let mut deepest = 0.0_f64;
    for b in &ss.boundary {
        deepest = deepest.max(perp(b.a)).max(perp(b.b));
        // The line being crossed is the way in, not an obstruction.
        if b.linedef == linedef {
            continue;
        }
        let walkable = b
            .neighbor
            .filter(|_| b.passable())
            .is_some_and(|n| scene.sectors[n].floor - ss.floor <= step);
        if walkable {
            return false;
        }
    }
    deepest <= f64::from(radius)
}

/// Which of the three dispatchers fires a trigger line, which is what
/// decides the sides it can be fired from. The distinction is the engine's,
/// not a convenience: each function gates on the side differently, and two
/// of the three do not gate at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dispatch {
    /// `P_CrossSpecialLine` — a walkover, fired by the crossing itself, so
    /// no side gate and every side the player can actually cross from.
    Cross,
    /// `P_UseSpecialLine` — a switch, front side only
    /// (`p_switch.c:284-297`).
    Use,
    /// `P_ShootSpecialLine` — a gun line, fired from either side it faces
    /// (`p_spec.c:955-1000` takes no `side`; `p_map.c:919-920` passes
    /// none). No lift special reaches this dispatcher — it carries only 24,
    /// 46 and 47 — so this arm exists for [`crate::check::floors`]'s two
    /// gun forms.
    Shot,
}

/// The sides of a trigger line that can fire it, in the engine's own terms.
///
/// `b` is the line's **front** mirror and `front` the sector it is filed
/// under; `dispatch` names the function that fires it, which is what selects
/// the rule:
///
/// - [`Dispatch::Use`] — the front side alone. `P_UseSpecialLine`'s opening
///   `if (side)` block (pinned `p_switch.c:284-297`) `return false`s for
///   every special but 124.
/// - [`Dispatch::Shot`] — the front side, plus the back when the line has
///   one. `P_ShootSpecialLine` (pinned `p_spec.c:955-1000`) takes no `side`
///   argument at all, and `PTR_ShootTraverse` passes none: `if
///   (li->special) P_ShootSpecialLine (shootthing, li);`
///   (`p_map.c:919-920`), two lines before `ML_TWOSIDED` is so much as read
///   (`p_map.c:922`). A shot from either bordering sector fires it, and
///   neither the step rule nor [`Boundary::passable`] applies — a bullet
///   crosses what a player cannot. (What a hitscan from *further* away can
///   reach is a line-of-sight question this checker does not model, so this
///   stays the two sectors the line itself faces.)
/// - [`Dispatch::Cross`] — no side gate either (`P_CrossSpecialLine`), but
///   the crossing is the player's own, so it fires from whichever side they
///   can actually cross from at rest under `P_TryMove`'s step rule.
///
/// `(step, radius)` are [`Tables::step_height`] and
/// [`Tables::player`]`().radius`.
///
/// Shared with [`crate::check::floors`], whose lines fire by the same three
/// dispatchers, so the two resolutions cannot drift on who can press what.
/// The walkover arm carries the two deliberate divergences from the probe
/// this module's doc comment argues for: an impassable boundary
/// ([`Boundary::passable`]) fires from neither side, and neither does one
/// with a [`dead_end_pocket`] behind it.
pub(crate) fn activator_sides(
    scene: &Scene,
    b: &Boundary,
    front: usize,
    dispatch: Dispatch,
    (step, radius): (i32, i32),
) -> Vec<usize> {
    let mut sides = Vec::new();
    match dispatch {
        Dispatch::Use => sides.push(front),
        Dispatch::Shot => {
            sides.push(front);
            // A self-referencing line's back sector is the front one; it is
            // one sector to shoot from, not two.
            if let Some(back) = b.neighbor.filter(|&back| back != front) {
                sides.push(back);
            }
        }
        Dispatch::Cross => {
            if let Some(back) = b.neighbor.filter(|_| b.passable()) {
                // A walkover with a dead-end pocket on either side fires
                // from neither: nobody can cross *into* the pocket, and
                // nobody can stand in it to cross *out*. The same shape as
                // the `passable()` gate above, one layer further out.
                let pocket = [front, back]
                    .iter()
                    .any(|&s| dead_end_pocket(scene, b.linedef, s, step, radius));
                let (ff, bf) = (scene.sectors[front].floor, scene.sectors[back].floor);
                if !pocket && bf - ff <= step {
                    sides.push(front);
                }
                if !pocket && ff - bf <= step {
                    sides.push(back);
                }
            }
        }
    }
    sides
}

/// Every sector named by a nonzero tag on some line whose special satisfies
/// `is_special` — the targets one resolution models.
///
/// `P_FindSectorFromLineTag` matches by tag equality, so a line's tag names
/// every sector carrying it, and a tag-0 line names none: an untagged sector
/// does not "have tag 0", it answers to nothing. Front mirrors only —
/// `special` and `tag` are linedef-wide, so the back mirror of a two-sided
/// line would otherwise contribute the same target a second time.
///
/// Shared with [`crate::check::floors`]: a lift line and a floor line resolve
/// a tag by the same engine rule, so which sectors an action drives must not
/// be answered twice.
pub(crate) fn sectors_named_by(scene: &Scene, is_special: impl Fn(i32) -> bool) -> BTreeSet<usize> {
    let tags: BTreeSet<i32> = scene
        .sectors
        .iter()
        .flat_map(|s| s.boundary.iter())
        .filter(|b| b.fronts_this && b.tag != 0 && is_special(b.special))
        .map(|b| b.tag)
        .collect();
    scene
        .sectors
        .iter()
        .enumerate()
        .filter(|(_, s)| tags.contains(&s.tag))
        .map(|(i, _)| i)
        .collect()
}

/// The lines of the family `is_special` selects that can never fire anything:
/// tag 0, or a tag naming no sector.
///
/// Ascending by linedef declaration index, each named once. The walk is over
/// sectors' boundaries, so the raw order would be sector-then-boundary and
/// only coincidentally sorted; a `BTreeSet` makes it the caller-visible index
/// order instead. Front mirrors only — `special` and `tag` are linedef-wide,
/// so the back mirror of a two-sided line would otherwise report the same
/// line twice.
///
/// Shared with [`crate::check::floors`], for the reason
/// [`sectors_named_by`] gives: this is the complement of that set, read off
/// the same rule.
pub(crate) fn broken_tag_lines(scene: &Scene, is_special: impl Fn(i32) -> bool) -> Vec<usize> {
    scene
        .sectors
        .iter()
        .flat_map(|s| s.boundary.iter())
        .filter(|b| b.fronts_this && is_special(b.special))
        .filter(|b| b.tag == 0 || !scene.sectors.iter().any(|s| s.tag == b.tag))
        .map(|b| b.linedef)
        .collect::<BTreeSet<usize>>()
        .into_iter()
        .collect()
}

/// The nonzero specials naming `tag` that `is_own` does not claim, sorted and
/// deduped — the other actions sharing a resolution's tag.
///
/// Another action on the same tag drives the same sector, which is what makes
/// it a conflict to report rather than a trigger to model. Front mirrors
/// only, for the reason [`sectors_named_by`] gives.
///
/// Shared with [`crate::check::floors`], whose "not a floor line" is the same
/// question asked of a different vocabulary.
pub(crate) fn other_specials_on_tag(
    scene: &Scene,
    tag: i32,
    is_own: impl Fn(i32) -> bool,
) -> Vec<i32> {
    let mut others: Vec<i32> = scene
        .sectors
        .iter()
        .flat_map(|s| s.boundary.iter())
        .filter(|b| b.fronts_this && b.tag == tag && b.special != 0 && !is_own(b.special))
        .map(|b| b.special)
        .collect();
    others.sort_unstable();
    others.dedup();
    others
}

/// Every lift line naming `tag`, as the trigger it is for the plat at
/// sector `plat`: which sectors fire it, and each one's [`Activator`] class.
///
/// `lines` is the `(front sector, boundary)` pair of every lift line in the
/// scene, listed once per linedef.
fn triggers_for(
    scene: &Scene,
    lines: &[(usize, &Boundary)],
    specials: &LiftSpecials,
    plat: usize,
    tag: i32,
    (step, radius): (i32, i32),
) -> Vec<SceneTrigger> {
    lines
        .iter()
        .filter(|(_, b)| b.tag == tag)
        .map(|&(front, b)| {
            let is_use = specials.use_line.contains(&b.special);
            // Lift lines are only ever crossed or used: `EV_DoPlat`'s
            // `downWaitUpStay` forms carry no gun line (`P_ShootSpecialLine`
            // dispatches 24, 46 and 47 alone).
            let dispatch = if is_use {
                Dispatch::Use
            } else {
                Dispatch::Cross
            };
            let activators = activator_sides(scene, b, front, dispatch, (step, radius))
                .into_iter()
                .map(|s| (s, classify(scene, plat, s, step)))
                .collect();
            SceneTrigger {
                linedef: b.linedef,
                special: b.special,
                use_line: is_use,
                repeatable: specials.repeatable.contains(&b.special),
                fast: specials.fast.contains(&b.special),
                front,
                back: b.neighbor,
                activators,
                lower_unpegged: b.lower_unpegged,
            }
        })
        .collect()
}

/// Every plat, ascending by sector: each sector some lift line names by a
/// nonzero tag.
///
/// A lift line carrying tag 0, or a tag no sector answers to, resolves to no
/// plat here at all — it fires nothing, which is V-P13/V-P14's finding (and
/// [`broken_lift_lines`]'s list), not a platform to model.
#[must_use]
pub fn resolve_plats(scene: &Scene, tables: &Tables) -> Vec<ScenePlat> {
    let step = tables.step_height();
    let radius = tables.player().radius;
    let specials = LiftSpecials::resolve(tables);
    // (front sector, boundary) of every lift line, once (`fronts_this`):
    // `special` is linedef-wide, so the back mirror of a two-sided lift line
    // would otherwise contribute the same trigger a second time.
    let lines: Vec<(usize, &Boundary)> = scene
        .sectors
        .iter()
        .enumerate()
        .flat_map(|(i, s)| {
            s.boundary
                .iter()
                .filter(|b| b.fronts_this && specials.all.contains(&b.special))
                .map(move |b| (i, b))
        })
        .collect();
    let named = sectors_named_by(scene, |s| specials.all.contains(&s));
    named
        .into_iter()
        .map(|sector| {
            let ss = &scene.sectors[sector];
            let neighbors: BTreeSet<usize> =
                ss.boundary.iter().filter_map(|b| b.neighbor).collect();
            let floors: Vec<i32> = neighbors.iter().map(|&n| scene.sectors[n].floor).collect();
            // `P_FindLowestFloorSurrounding` starts at the sector's own floor.
            let low = floors.iter().copied().fold(ss.floor, i32::min);
            let travel = ss.floor - low;
            let max_nb = floors.iter().copied().max().unwrap_or(ss.floor);
            let rest = if travel == 0 {
                Rest::Dead
            } else if max_nb > ss.floor + step {
                Rest::Intermediate
            } else if max_nb >= ss.floor - step {
                Rest::Top
            } else {
                Rest::AboveAll
            };
            ScenePlat {
                sector,
                tag: ss.tag,
                low,
                travel,
                rest,
                distinct_neighbor_floors: floors.iter().copied().collect::<BTreeSet<i32>>().len(),
                shared_tag: scene.sectors.iter().filter(|s| s.tag == ss.tag).count(),
                triggers: triggers_for(scene, &lines, &specials, sector, ss.tag, (step, radius)),
                other_actions: other_specials_on_tag(scene, ss.tag, |s| specials.all.contains(&s)),
                neighbors,
            }
        })
        .collect()
}

/// Lift lines that can never fire a plat: tag 0, or a tag naming no sector.
///
/// `plats::broken_tag_lines` documents the ordering and the front-mirror rule.
#[must_use]
pub fn broken_lift_lines(scene: &Scene, tables: &Tables) -> Vec<usize> {
    let lift = LiftSpecials::resolve(tables).all;
    broken_tag_lines(scene, |s| lift.contains(&s))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Activator, Rest, ScenePlat, broken_lift_lines, dead_end_pocket, resolve_plats};
    use crate::check::Subject;
    use crate::check::fixtures::{chain, pocket_lift, scene_of};
    use crate::check::invariants::check_lift_return;
    use crate::check::scene::Scene;
    use crate::tables::Tables;
    use crustywad::map::udmf::parse_udmf;

    /// Parses `text`, builds its [`Scene`] and resolves its plats.
    fn plats_of(text: &str) -> (Scene, Tables, Vec<ScenePlat>) {
        let tables = Tables::load().expect("tables");
        let map = parse_udmf(text, crustywad::Limits::default()).expect("fixture parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let plats = resolve_plats(&scene, &tables);
        (scene, tables, plats)
    }

    /// The corpus's dominant shape: a low room, the plat at rest 128 above
    /// it, and a level landing on the far side.
    const LIFT_FLOORS: [i32; 3] = [0, 128, 128];
    /// [`LIFT_FLOORS`]'s tags: only the plat carries one.
    const LIFT_TAGS: [i32; 3] = [0, 7, 0];

    #[test]
    fn a_riser_switch_resolves_to_a_top_plat_callable_from_the_low_room() {
        let (_, _, plats) = plats_of(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(plats.len(), 1);
        let p = &plats[0];
        assert_eq!((p.sector, p.tag, p.low, p.travel), (1, 7, 0, 128));
        assert_eq!(p.rest, Rest::Top);
        assert_eq!(p.neighbors, BTreeSet::from([0, 2]));
        assert_eq!(p.shared_tag, 1);
        assert_eq!(p.triggers.len(), 1);
        let t = &p.triggers[0];
        assert!(t.use_line && t.repeatable && !t.fast);
        assert_eq!(
            (t.linedef, t.special),
            (0, 62),
            "the trigger names the line it came from"
        );
        assert!(
            !t.lower_unpegged,
            "`chain` writes no pegging flags, and the riser wants the flag clear"
        );
        assert_eq!((t.front, t.back), (0, Some(1)));
        assert_eq!(t.activators, vec![(0, Activator::Low)]);
        assert!(p.callable_low() && !p.callable_top());
        assert_eq!(p.low_activator_neighbors(), BTreeSet::from([0]));
        assert!(
            p.other_actions.is_empty(),
            "nothing but the lift line names tag 7: {:?}",
            p.other_actions
        );

        // The same line with `ML_DONTPEGBOTTOM` set, so the flag is read off
        // the linedef rather than assumed clear.
        let (_, _, pegged) = plats_of(
            &chain(
                &LIFT_FLOORS,
                &LIFT_TAGS,
                &[(62, 7, false), (0, 0, false)],
                "",
            )
            .replacen(
                "special = 62; arg0 = 7; }",
                "special = 62; arg0 = 7; dontpegbottom = true; }",
                1,
            ),
        );
        assert!(pegged[0].triggers[0].lower_unpegged);
    }

    #[test]
    fn a_walkover_on_the_low_face_fires_only_from_the_plat() {
        let (_, _, plats) = plats_of(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(88, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(plats[0].triggers[0].activators, vec![(1, Activator::Plat)]);
        assert!(!plats[0].callable_low() && plats[0].callable_top());
    }

    #[test]
    fn a_walkovers_two_sides_are_gated_one_at_a_time() {
        // `a_walkover_on_the_low_face_fires_only_from_the_plat`'s two
        // sectors with the line drawn the other way round, so the platform
        // is its front and the low room its back. `P_TryMove`'s rule is
        // read once per side: the platform steps down into the low room, so
        // the front fires; the low room cannot climb the 128 back onto the
        // platform, so the back does not. The same lone activator, reached
        // through the other of the two guards.
        let (_, _, plats) = plats_of(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(88, 7, true), (0, 0, false)],
            "",
        ));
        let t = &plats[0].triggers[0];
        assert_eq!(
            (t.front, t.back),
            (1, Some(0)),
            "the flipped link puts the platform on the line's front"
        );
        assert_eq!(t.activators, vec![(1, Activator::Plat)]);

        // A walkover between two rooms level with each other and well below
        // the platform they name: each can cross into the other, so both
        // fire it, and the back one is a low room rather than the platform.
        let (_, _, plats) = plats_of(&chain(
            &[0, 0, 128],
            &[0, 0, 7],
            &[(88, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            plats[0].triggers[0].activators,
            vec![(0, Activator::Low), (1, Activator::Low)]
        );
    }

    #[test]
    fn a_walkover_nobody_can_cross_fires_from_neither_side() {
        // `ML_BLOCKING` is legal on a two-sided line — a fence the player
        // sees and shoots across but cannot walk through.
        // `P_CrossSpecialLine` runs off `P_TryMove`, so a walkover on one is
        // crossed from neither side and has no activator at all, even though
        // the line is still the platform's trigger.
        let text = chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(88, 7, false), (0, 0, false)],
            "",
        )
        .replacen(
            "special = 88; arg0 = 7; }",
            "special = 88; arg0 = 7; blocking = true; }",
            1,
        );
        let (_, _, plats) = plats_of(&text);
        let t = &plats[0].triggers[0];
        assert_eq!((t.linedef, t.special), (0, 88));
        assert!(
            t.activators.is_empty(),
            "an uncrossable walkover fires from nowhere: {:?}",
            t.activators
        );
        assert!(!plats[0].callable_low() && !plats[0].callable_top());
    }

    /// A walkover whose far sector is a dead end no deeper than the player's
    /// radius fires from neither side: `P_TryMove` never lets a center in
    /// there, so nobody crosses into the pocket and nobody stands in it to
    /// cross out. The plat is then callable only from above — V-P5's warning.
    ///
    /// The other two cases are the ones the rule must leave alone: the same
    /// pocket 32 deep (a center fits, so both sides fire), and a 16-deep strip
    /// that is *open* at one end, which is what the census's §G3 found in
    /// DOOM E1M3 and MAP04 and which plays perfectly.
    #[test]
    fn a_walkover_into_a_dead_end_shallower_than_the_radius_fires_from_neither_side() {
        let (_, _, plats) = plats_of(&pocket_lift(16, false));
        assert_eq!(plats.len(), 1);
        let p = &plats[0];
        assert_eq!((p.sector, p.tag, p.low, p.travel), (2, 7, 0, 128));
        assert_eq!(
            p.triggers.len(),
            1,
            "the 88 line is still the plat's trigger"
        );
        assert!(
            p.triggers[0].activators.is_empty(),
            "a center that cannot enter the pocket fires nothing: {:?}",
            p.triggers[0].activators
        );
        assert!(!p.callable_low() && !p.callable_top());
        assert!(p.low_activator_neighbors().is_empty());

        // And the finding that follows, read through the check that owns it.
        let mut findings = Vec::new();
        let (scene, tables) = scene_of(&pocket_lift(16, false));
        check_lift_return(&scene, &tables, &mut findings);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].check == "V-P5"
                && findings[0].subject == Subject::Sector(2)
                && findings[0].message.contains("only from above"),
            "{findings:?}"
        );

        // 32 clears the 16-unit radius, so both sides of the same line fire.
        let (_, _, plats) = plats_of(&pocket_lift(32, false));
        assert_eq!(
            plats[0].triggers[0].activators,
            vec![(0, Activator::Low), (1, Activator::Low)]
        );
        assert!(plats[0].callable_low());

        // Still 16 deep, but with a passable boundary of its own: the box may
        // overhang through it, so the crossing is real and the depth alone
        // must not condemn it.
        let (_, _, plats) = plats_of(&pocket_lift(16, true));
        assert_eq!(
            plats[0].triggers[0].activators,
            vec![(0, Activator::Low), (1, Activator::Low)]
        );
        assert!(plats[0].callable_low());
    }

    /// `dead_end_pocket` measures one sector's depth behind one line, so a
    /// sector that line does not bound has nothing behind it to measure and
    /// is no pocket at all — the guard that keeps the depth from being taken
    /// against an edge belonging to some other part of the map.
    ///
    /// Read on `pocket_lift`'s own scene so the negative sits beside the
    /// positive it is not: the pocket really is one, the platform two
    /// sectors along the chain is not, and the only difference between the
    /// two calls is which sector the line bounds.
    #[test]
    fn a_sector_the_line_does_not_bound_is_no_pocket_behind_it() {
        let (scene, tables) = scene_of(&pocket_lift(16, false));
        let (step, radius) = (tables.step_height(), tables.player().radius);
        // Linedef 0 is the `88` walkover between the low room (0) and the
        // pocket (1). The platform (2) lies beyond the pocket and shares no
        // boundary with that line.
        assert!(
            dead_end_pocket(&scene, 0, 1, step, radius),
            "the 16-deep dead end behind the walkover is a pocket"
        );
        assert!(
            !dead_end_pocket(&scene, 0, 2, step, radius),
            "the platform does not border linedef 0, so nothing of it lies behind it"
        );
    }

    /// A zero-length linedef gives no direction to measure a depth along, so
    /// no sector is a pocket behind it.
    ///
    /// Reachable: `Scene::build` validates a linedef's cross-references —
    /// `v1`, `v2`, both sidedefs and their sectors — but never its geometry,
    /// and `process_linedef` never compares `v1` to `v2`, so a degenerate
    /// line becomes a boundary whose two endpoints coincide. Without the
    /// guard the normal is `0.0 / 0.0` and every `perp` is `NaN`, which
    /// `f64::max` discards in favor of the running `0.0` — a depth of zero,
    /// and the function would call this room the shallowest pocket there is.
    #[test]
    fn a_zero_length_line_is_no_ones_pocket() {
        // `chain`'s 3 rooms use sidedefs 0..=11 (2 per link, 8 walls), so
        // the appended one is 12; vertex 0 is the low room's own corner.
        let text = chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "linedef { v1 = 0; v2 = 0; sidefront = 12; blocking = true; }\n\
             sidedef { sector = 0; texturemiddle = \"STARTAN2\"; }\n",
        );
        let (scene, tables) = scene_of(&text);
        let degenerate = scene.sectors[0]
            .boundary
            .iter()
            .find(|b| b.a == b.b)
            .expect("the degenerate line reaches the scene as a zero-length edge");
        assert!(
            !dead_end_pocket(
                &scene,
                degenerate.linedef,
                0,
                tables.step_height(),
                tables.player().radius
            ),
            "a line with no direction encloses nothing"
        );
    }

    #[test]
    fn rests_and_shared_tags() {
        let (_, _, p) = plats_of(&chain(&[0, 128], &[0, 7], &[(62, 7, false)], ""));
        assert_eq!(
            (p[0].rest, p[0].distinct_neighbor_floors),
            (Rest::AboveAll, 1)
        );
        let (_, _, p) = plats_of(&chain(
            &[0, 64, 160],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(p[0].rest, Rest::Intermediate);
        let (_, _, p) = plats_of(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!((p[0].rest, p[0].travel), (Rest::Dead, 0));
        let (_, _, p) = plats_of(&chain(
            &[0, 128, 128],
            &[0, 7, 7],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(p.len(), 2);
        assert!(p.iter().all(|x| x.shared_tag == 2));

        // Three non-lift lines naming the plat's own tag, declared out of
        // order and one of them twice: `other_actions` sorts and dedupes
        // them, and none of them becomes a trigger.
        let (_, _, p) = plats_of(&chain(
            &[0, 128, 128, 0, 0],
            &[0, 7, 0, 0, 0],
            &[
                (62, 7, false),
                (26, 7, false),
                (11, 7, false),
                (26, 7, false),
            ],
            "",
        ));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].other_actions, vec![11, 26]);
        assert_eq!(
            p[0].triggers.len(),
            1,
            "only the lift special is a trigger: {:?}",
            p[0].triggers
        );
    }

    #[test]
    fn the_step_boundary_is_exact_and_broken_lines_are_named() {
        let (_, _, p) = plats_of(&chain(
            &[0, 24, 24],
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(p[0].triggers[0].activators[0].1, Activator::Level);
        let (_, _, p) = plats_of(&chain(
            &[0, 25, 25],
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(p[0].triggers[0].activators[0].1, Activator::Low);
        let (scene, tables, p) = plats_of(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 0, false), (62, 9, false)],
            "",
        ));
        assert!(p.is_empty(), "tag 0 and an unresolved tag name no plat");
        assert_eq!(broken_lift_lines(&scene, &tables), vec![0, 1]);
    }
}
