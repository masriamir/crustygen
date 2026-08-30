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
//! moved crate-side: the probe measures the idgames corpus with them, this
//! module judges an emitted map with them, and both read one set of rules.

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
    step: i32,
) -> Vec<SceneTrigger> {
    lines
        .iter()
        .filter(|(_, b)| b.tag == tag)
        .map(|&(front, b)| {
            let is_use = specials.use_line.contains(&b.special);
            let mut activators = Vec::new();
            if is_use {
                // `P_UseSpecialLine`: the front side alone.
                activators.push((front, classify(scene, plat, front, step)));
            } else if let Some(back) = b.neighbor.filter(|_| b.passable()) {
                // `P_CrossSpecialLine` has no side gate, so either side fires
                // it — from whichever the player can actually cross at rest
                // (`P_TryMove`'s step rule).
                let (ff, bf) = (scene.sectors[front].floor, scene.sectors[back].floor);
                if bf - ff <= step {
                    activators.push((front, classify(scene, plat, front, step)));
                }
                if ff - bf <= step {
                    activators.push((back, classify(scene, plat, back, step)));
                }
            }
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
    let named: BTreeSet<usize> = lines
        .iter()
        .filter(|(_, b)| b.tag != 0)
        .flat_map(|(_, b)| {
            scene
                .sectors
                .iter()
                .enumerate()
                .filter(move |(_, s)| s.tag == b.tag)
                .map(|(i, _)| i)
        })
        .collect();
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
            let mut other_actions: Vec<i32> = scene
                .sectors
                .iter()
                .flat_map(|s| s.boundary.iter())
                .filter(|b| {
                    b.fronts_this
                        && b.tag == ss.tag
                        && b.special != 0
                        && !specials.all.contains(&b.special)
                })
                .map(|b| b.special)
                .collect();
            other_actions.sort_unstable();
            other_actions.dedup();
            ScenePlat {
                sector,
                tag: ss.tag,
                low,
                travel,
                rest,
                distinct_neighbor_floors: floors.iter().copied().collect::<BTreeSet<i32>>().len(),
                shared_tag: scene.sectors.iter().filter(|s| s.tag == ss.tag).count(),
                triggers: triggers_for(scene, &lines, &specials, sector, ss.tag, step),
                other_actions,
                neighbors,
            }
        })
        .collect()
}

/// Lift lines that can never fire a plat: tag 0, or a tag naming no sector.
///
/// Each linedef is named once, from its front mirror, in sector declaration
/// order — `special` and `tag` are linedef-wide, so the back mirror of a
/// two-sided lift line would otherwise report the same line twice.
#[must_use]
pub fn broken_lift_lines(scene: &Scene, tables: &Tables) -> Vec<usize> {
    let lift: Vec<i32> = tables.lift_specials().into_iter().map(i32::from).collect();
    scene
        .sectors
        .iter()
        .flat_map(|s| s.boundary.iter())
        .filter(|b| b.fronts_this && lift.contains(&b.special))
        .filter(|b| b.tag == 0 || !scene.sectors.iter().any(|s| s.tag == b.tag))
        .map(|b| b.linedef)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Activator, Rest, ScenePlat, broken_lift_lines, resolve_plats};
    use crate::check::fixtures::chain;
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
        assert_eq!((t.front, t.back), (0, Some(1)));
        assert_eq!(t.activators, vec![(0, Activator::Low)]);
        assert!(p.callable_low() && !p.callable_top());
        assert_eq!(p.low_activator_neighbors(), BTreeSet::from([0]));
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
