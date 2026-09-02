//! Floor actions, re-derived from a built map the way `EV_DoFloor` and the
//! two one-way `EV_DoPlat` raise types read it — once, for the flood
//! ([`crate::check::flood`]), the invariants, the conformance rows and the
//! `lift::floor` recognizer, so all four judge one resolution.
//!
//! Every formula cites `linuxdoom-1.10` at `a77dfb96`. The effect
//! classification is the corpus probe's (`examples/liftprobe/common.rs`,
//! `docs/measurements/floor-shapes-2026-09-02.md`, "Definitions the numbers
//! depend on"), ported here with its worked examples as tests.
//!
//! The engine layer is a **copy** of the probe's, not a sharing of it: an
//! example cannot be a crate dependency, and the probe is the measurement's
//! frozen tool — the same reason `examples/liftprobe/common.rs` already
//! carries its own copy of [`crate::check::plats`]'s searches. What is
//! genuinely shared with `plats` is everything a *tagged line* means, none of
//! which is specific to what the tag drives: the activator rule (its
//! `activator_sides` and [`crate::check::plats::classify`]), which sectors a
//! tag names (`sectors_named_by`), which lines name nothing
//! (`broken_tag_lines`), and which other specials share a tag
//! (`other_specials_on_tag`). A floor line and a lift line are fired by the
//! same three dispatchers and resolve their tag by the same engine rule, so
//! neither question may be answered twice.

use std::collections::{BTreeMap, BTreeSet};

use crate::check::plats::{
    Activator, activator_sides, broken_tag_lines, classify, other_specials_on_tag, resolve_plats,
    sectors_named_by,
};
use crate::check::scene::{Boundary, Scene};
use crate::tables::{FloorEngineType, FloorForm, Tables};

// ---------------------------------------------------------------------------
// The engine layer: the four searches and the per-type destination.
// ---------------------------------------------------------------------------

/// Where a floor action sends its target, evaluated at load-time heights.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Destination {
    /// A resolved height.
    Height(i32),
    /// `raiseToTexture` (`p_floor.c:372-401`): the destination is the least
    /// bottom-texture height on the sector's two-sided lines, and neither the
    /// probe nor this resolution reads texture heights.
    NeedsTexture,
}

/// The sector's line list as the engine builds it (`p_setup.c P_GroupLines`,
/// 520-553): one entry per linedef bordering the sector, and **exactly one**
/// for a self-referencing line — the engine's count guard is
/// `if (li->backsector && li->backsector != li->frontsector)`, while
/// [`Scene`] files two mirrored boundaries for such a line. Declaration order
/// is preserved, which is what [`next_highest_floor`]'s 20-entry cap depends
/// on.
fn sector_lines(scene: &Scene, sec: usize) -> Vec<&Boundary> {
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    scene.sectors[sec]
        .boundary
        .iter()
        .filter(|b| seen.insert(b.linedef))
        .collect()
}

/// `getNextSector` (`p_spec.c:250-262`): the sector across a **two-sided**
/// line, `None` otherwise. A self-referencing line yields the sector itself,
/// exactly as the engine's `frontsector == sec ? backsector : frontsector`
/// does.
fn next_sector(b: &Boundary) -> Option<usize> {
    b.two_sided.then_some(b.neighbor).flatten()
}

/// `P_FindLowestFloorSurrounding` (`p_spec.c:270-291`): starts at the
/// sector's **own** floor, minimum over two-sided neighbors.
#[must_use]
pub fn lowest_floor_surrounding(scene: &Scene, sec: usize) -> i32 {
    sector_lines(scene, sec)
        .iter()
        .filter_map(|b| next_sector(b))
        .fold(scene.sectors[sec].floor, |lo, n| {
            lo.min(scene.sectors[n].floor)
        })
}

/// `P_FindHighestFloorSurrounding`'s starting value, `-500*FRACUNIT`
/// (`p_spec.c:303`). A sector with no two-sided neighbor "lowers" to it.
pub const NO_NEIGHBOR_FLOOR: i32 = -500;

/// `P_FindHighestFloorSurrounding` (`p_spec.c:297-318`): starts at
/// [`NO_NEIGHBOR_FLOOR`], maximum over two-sided neighbors.
#[must_use]
pub fn highest_floor_surrounding(scene: &Scene, sec: usize) -> i32 {
    sector_lines(scene, sec)
        .iter()
        .filter_map(|b| next_sector(b))
        .fold(NO_NEIGHBOR_FLOOR, |hi, n| hi.max(scene.sectors[n].floor))
}

/// `MAX_ADJOINING_SECTORS` (`p_spec.c:326`).
pub const MAX_ADJOINING_SECTORS: usize = 20;

/// What [`next_highest_floor`] found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextHighest {
    /// The least neighboring floor strictly above `currentheight`, or
    /// `currentheight` when no neighbor is above it (`p_spec.c:361-362`).
    pub height: i32,
    /// Whether the search filled its 20-entry list and broke early
    /// (`p_spec.c:349-355`) — the map is then reading a truncated
    /// neighborhood, and the destination may not be the true next height.
    pub capped: bool,
}

/// `P_FindNextHighestFloor(sec, currentheight)` (`p_spec.c:329-375`),
/// including its 20-entry cap: candidates are collected in the sector's own
/// line order and the loop breaks once the list is full.
#[must_use]
pub fn next_highest_floor(scene: &Scene, sec: usize, current: i32) -> NextHighest {
    let mut candidates: Vec<i32> = Vec::new();
    let mut capped = false;
    for b in sector_lines(scene, sec) {
        let Some(n) = next_sector(b) else { continue };
        let floor = scene.sectors[n].floor;
        if floor > current {
            candidates.push(floor);
        }
        if candidates.len() >= MAX_ADJOINING_SECTORS {
            capped = true;
            break;
        }
    }
    NextHighest {
        height: candidates.iter().copied().min().unwrap_or(current),
        capped,
    }
}

/// `P_FindLowestCeilingSurrounding` (`p_spec.c:382-401`): starts at `MAXINT`
/// — [`i32::MAX`] here — and takes the minimum neighboring ceiling.
#[must_use]
pub fn lowest_ceiling_surrounding(scene: &Scene, sec: usize) -> i32 {
    sector_lines(scene, sec)
        .iter()
        .filter_map(|b| next_sector(b))
        .fold(i32::MAX, |lo, n| lo.min(scene.sectors[n].ceiling))
}

/// `raiseFloor`'s destination: `P_FindLowestCeilingSurrounding` capped at the
/// sector's own ceiling (`p_floor.c:322-326`).
fn raise_floor_destination(scene: &Scene, target: usize) -> i32 {
    lowest_ceiling_surrounding(scene, target).min(scene.sectors[target].ceiling)
}

/// The `floordestheight` / `plat->high` the engine would compute for
/// `target` under `ty`, at the heights the map loads with.
#[must_use]
pub fn destination(scene: &Scene, target: usize, ty: FloorEngineType) -> Destination {
    let floor = scene.sectors[target].floor;
    let height = match ty {
        FloorEngineType::LowerFloor => highest_floor_surrounding(scene, target),
        FloorEngineType::LowerFloorToLowest | FloorEngineType::LowerAndChange => {
            lowest_floor_surrounding(scene, target)
        }
        FloorEngineType::TurboLower => {
            // `p_floor.c:313-314`: the `+ 8` is applied only when the
            // destination differs from the sector's current floor.
            let high = highest_floor_surrounding(scene, target);
            if high == floor { high } else { high + 8 }
        }
        FloorEngineType::RaiseFloor => raise_floor_destination(scene, target),
        // `p_floor.c:327-328`: the `- 8` is applied after the ceiling cap.
        FloorEngineType::RaiseFloorCrush => raise_floor_destination(scene, target) - 8,
        FloorEngineType::RaiseFloorToNearest
        | FloorEngineType::RaiseFloorTurbo
        | FloorEngineType::PlatRaiseToNearestAndChange => {
            next_highest_floor(scene, target, floor).height
        }
        FloorEngineType::RaiseFloor24
        | FloorEngineType::RaiseFloor24AndChange
        | FloorEngineType::PlatRaiseAndChange24 => floor + 24,
        FloorEngineType::PlatRaiseAndChange32 => floor + 32,
        FloorEngineType::RaiseFloor512 => floor + 512,
        FloorEngineType::RaiseToTexture => return Destination::NeedsTexture,
    };
    Destination::Height(height)
}

// ---------------------------------------------------------------------------
// The effect classifier: what an action does to who can walk where.
// ---------------------------------------------------------------------------

/// The floor heights one evaluation runs at: the map's, except the target's,
/// which is [`Heights::target_floor`] — its rest height before the action
/// fires, its destination after. Ceilings never move here.
#[derive(Debug, Clone, Copy)]
pub struct Heights {
    /// The moving sector.
    pub target: usize,
    /// The moving sector's floor in this evaluation.
    pub target_floor: i32,
}

impl Heights {
    /// Sector `s`'s floor under these heights.
    #[must_use]
    pub fn floor(self, scene: &Scene, s: usize) -> i32 {
        if s == self.target {
            self.target_floor
        } else {
            scene.sectors[s].floor
        }
    }
}

/// `ceiling(S) − floor(S) ≥ H`: `P_TryMove` refuses a move into a sector the
/// player does not fit in (`p_map.c:468`, `tmceilingz - tmfloorz <
/// thing->height`).
#[must_use]
pub fn standable(scene: &Scene, h: Heights, s: usize, player_height: i32) -> bool {
    scene.sectors[s].ceiling - h.floor(scene, s) >= player_height
}

/// Whether the player can walk from `a` into `b` across a two-sided
/// boundary. `P_LineOpening` (`p_maputl.c:300-332`) sets `opentop = min(front
/// ceiling, back ceiling)` and `openbottom = max(front floor, back floor)`;
/// `P_TryMove` then refuses when the opening is shorter than the player
/// (`p_map.c:468`) and when the step up exceeds 24 (`p_map.c:478`). Descent
/// is free: the drop-off arm (`p_map.c:481`) is gated on
/// `!(thing->flags & (MF_DROPOFF|MF_FLOAT))`, and `MT_PLAYER` carries
/// `MF_DROPOFF` (`info.c:1130`).
#[must_use]
pub fn pass(scene: &Scene, h: Heights, a: usize, b: usize, player_height: i32, step: i32) -> bool {
    if !standable(scene, h, a, player_height) || !standable(scene, h, b, player_height) {
        return false;
    }
    let (fa, fb) = (h.floor(scene, a), h.floor(scene, b));
    let opening = scene.sectors[a].ceiling.min(scene.sectors[b].ceiling) - fa.max(fb);
    opening >= player_height && fb - fa <= step
}

/// The two-sided adjacency among `members`, in both directions. A sector's
/// self-referencing boundaries are dropped: a sector is not its own neighbor
/// for the purpose of walking between rooms.
#[must_use]
pub fn local_adjacency(
    scene: &Scene,
    members: &BTreeSet<usize>,
) -> BTreeMap<usize, BTreeSet<usize>> {
    let mut adjacency: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
    for &m in members {
        for b in &scene.sectors[m].boundary {
            let Some(n) = next_sector(b) else { continue };
            if n != m && members.contains(&n) {
                adjacency.entry(m).or_default().insert(n);
                adjacency.entry(n).or_default().insert(m);
            }
        }
    }
    adjacency
}

/// `reach(A)` for every member of the local graph: the members reachable
/// from `A` through directed [`pass`] edges inside the graph, excluding `A`
/// itself and the target (which may be a *via*, never a destination). A
/// member that is not [`standable`] reaches nothing.
#[must_use]
pub fn reach_sets(
    scene: &Scene,
    members: &BTreeSet<usize>,
    adjacency: &BTreeMap<usize, BTreeSet<usize>>,
    h: Heights,
    player_height: i32,
    step: i32,
) -> BTreeMap<usize, BTreeSet<usize>> {
    let empty = BTreeSet::new();
    let mut out = BTreeMap::new();
    for &start in members {
        let mut visited: BTreeSet<usize> = BTreeSet::new();
        if standable(scene, h, start, player_height) {
            visited.insert(start);
            let mut stack = vec![start];
            while let Some(x) = stack.pop() {
                for &y in adjacency.get(&x).unwrap_or(&empty) {
                    if !visited.contains(&y) && pass(scene, h, x, y, player_height, step) {
                        visited.insert(y);
                        stack.push(y);
                    }
                }
            }
        }
        visited.remove(&start);
        visited.remove(&h.target);
        out.insert(start, visited);
    }
    out
}

/// What a floor action does to everyone *other* than a rider standing on the
/// target when it fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// `d == f`: the thinker runs and changes nothing. Decided first.
    Dead,
    /// Every neighbor's reach set grows or holds, at least one grows.
    Opening,
    /// Every neighbor's reach set shrinks or holds, at least one shrinks.
    Closing,
    /// Some neighbor gains and some neighbor loses.
    Mixed,
    /// Every reach set is unchanged, and `d != f`.
    Neutral,
}

/// What the action does to a player already standing on the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rider {
    /// Nobody can be standing on the target when it fires.
    NotApplicable,
    /// The rider can still reach everything it could before.
    Keeps,
    /// The rider loses a destination — it may be stranded.
    Loses,
}

/// Which opening a floor action carves, when it opens one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpeningShape {
    /// Lowers, and could not be entered before: a wall drops away.
    DropWall,
    /// Lowers from a height the player could already step onto.
    LedgeLower,
    /// Rises from a pit the player could already drop into: a walkway.
    Bridge,
    /// A sealed sector the player can step onto once it has moved, without
    /// any neighbor gaining a new destination: the sunken pedestal that
    /// exposes a pickup, or a panel between two areas already joined. The
    /// effect is [`Effect::Neutral`] — nothing new is *reachable* — but the
    /// target itself becomes standable, which the reach sets never see
    /// because the target is never a destination of its own local graph.
    Reveal,
    /// An opening none of the others describes. Expected to be rare;
    /// reported rather than hidden.
    OtherOpening,
}

/// The classification of one `(engine type, target)` action.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent measured fact about the action (what the target is \
              reachable from before and after, what the destination coincides with, whether the \
              neighbors were joined anyway); they encode no joint state"
)]
pub struct EffectFacts {
    /// The effect on everyone but a rider.
    pub effect: Effect,
    /// The effect on a rider.
    pub rider: Rider,
    /// The opening sub-shape: an [`Effect::Opening`] whose rider is not
    /// stranded, or an [`Effect::Neutral`] that is an
    /// [`OpeningShape::Reveal`].
    pub opening: Option<OpeningShape>,
    /// Whether some neighbor can walk onto the target at its rest floor.
    pub enterable_before: bool,
    /// Whether some neighbor can walk onto the target at its destination.
    pub enterable_after: bool,
    /// Whether two distinct neighbors could already reach each other, both
    /// ways, inside the local graph before the action fired — so the target
    /// was not the only route between them.
    pub neighbors_already_connected: bool,
    /// Whether the destination is exactly some neighbor's floor.
    pub joins_neighbor_floor: bool,
    /// How many `(A, B)` pairs have `B` newly reachable from `A`.
    pub new_pairs: usize,
}

/// Classifies the action that moves `target`'s floor from `rest` to `dest`,
/// against the local graph `{target} ∪ neighbors` at load-time heights.
#[must_use]
pub fn classify_effect(
    scene: &Scene,
    target: usize,
    neighbors: &BTreeSet<usize>,
    rest: i32,
    dest: i32,
    player_height: i32,
    step: i32,
) -> EffectFacts {
    let mut members: BTreeSet<usize> = neighbors.clone();
    members.insert(target);
    let adjacency = local_adjacency(scene, &members);
    let before_h = Heights {
        target,
        target_floor: rest,
    };
    let after_h = Heights {
        target,
        target_floor: dest,
    };
    let before = reach_sets(scene, &members, &adjacency, before_h, player_height, step);
    let after = reach_sets(scene, &members, &adjacency, after_h, player_height, step);

    // `rider_before` is `standable(T) at f ∧ ∃A ∈ N(T): pass(A → T)`, and
    // `pass` already requires `standable(T)` — so it is exactly
    // "enterable before".
    let enterable_before = neighbors
        .iter()
        .any(|&n| n != target && pass(scene, before_h, n, target, player_height, step));
    let enterable_after = neighbors
        .iter()
        .any(|&n| n != target && pass(scene, after_h, n, target, player_height, step));

    let empty = BTreeSet::new();
    let mut new_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut any_loss = false;
    for &a in &members {
        if a == target {
            continue;
        }
        let (was, now) = (
            before.get(&a).unwrap_or(&empty),
            after.get(&a).unwrap_or(&empty),
        );
        new_pairs.extend(now.difference(was).map(|&b| (a, b)));
        any_loss |= was.difference(now).next().is_some();
    }
    let effect = if dest == rest {
        Effect::Dead
    } else {
        match (!new_pairs.is_empty(), any_loss) {
            (true, true) => Effect::Mixed,
            (true, false) => Effect::Opening,
            (false, true) => Effect::Closing,
            (false, false) => Effect::Neutral,
        }
    };

    let rider = if enterable_before {
        let (was, now) = (
            before.get(&target).unwrap_or(&empty),
            after.get(&target).unwrap_or(&empty),
        );
        if now.is_superset(was) {
            Rider::Keeps
        } else {
            Rider::Loses
        }
    } else {
        Rider::NotApplicable
    };

    let opening = if effect == Effect::Opening && rider != Rider::Loses {
        Some(match (dest < rest, enterable_before) {
            (true, false) => OpeningShape::DropWall,
            (true, true) => OpeningShape::LedgeLower,
            (false, true) => OpeningShape::Bridge,
            (false, false) => OpeningShape::OtherOpening,
        })
    } else if effect == Effect::Neutral && !enterable_before && enterable_after {
        // The rider is `NotApplicable` by construction: nobody could be
        // standing on a target no neighbor could walk onto.
        Some(OpeningShape::Reveal)
    } else {
        None
    };

    // Read off the *before* sets: a pair of neighbors that could already
    // reach each other both ways was not depending on the target.
    let neighbors_already_connected = before.iter().any(|(&a, reach)| {
        a != target
            && reach
                .iter()
                .any(|&b| before.get(&b).is_some_and(|back| back.contains(&a)))
    });

    EffectFacts {
        effect,
        rider,
        opening,
        enterable_before,
        enterable_after,
        neighbors_already_connected,
        joins_neighbor_floor: neighbors
            .iter()
            .any(|&n| n != target && scene.sectors[n].floor == dest),
        new_pairs: new_pairs.len(),
    }
}

// ---------------------------------------------------------------------------
// Trigger reading: which lines drive a target, and from where.
// ---------------------------------------------------------------------------

/// Where a floor line sits relative to the target it names.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Placement {
    /// The target is the line's front sector.
    OnTargetFront,
    /// The target is its back sector.
    OnTargetBack,
    /// A side of the line is a neighbor of the target.
    Adjacent,
    /// Neither side is the target nor a neighbor.
    Remote,
}

/// Where a trigger line sits relative to `target`. A dangling front side is
/// simply not the target and not a neighbor.
fn placement_of(
    target: usize,
    neighbors: &BTreeSet<usize>,
    front: Option<usize>,
    back: Option<usize>,
) -> Placement {
    if front == Some(target) {
        Placement::OnTargetFront
    } else if back == Some(target) {
        Placement::OnTargetBack
    } else if front.is_some_and(|f| neighbors.contains(&f))
        || back.is_some_and(|b| neighbors.contains(&b))
    {
        Placement::Adjacent
    } else {
        Placement::Remote
    }
}

/// One floor line naming a target's tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorTrigger {
    /// Declaration index of the linedef.
    pub linedef: usize,
    /// Its special.
    pub special: i32,
    /// The engine type the special dispatches.
    pub engine_type: FloorEngineType,
    /// How it fires.
    pub form: FloorForm,
    /// Its front sector, `None` when the side dangles. Always `Some` for a
    /// trigger [`resolve_floors`] builds — [`Scene::build`] drops a linedef
    /// whose sidedef or sector reference does not resolve, so a line with a
    /// dangling front contributes no front mirror to read. The option is the
    /// shape [`Placement`] is derived in, and what a caller reading a raw
    /// `UdmfMap` rather than a [`Scene`] would have.
    pub front: Option<usize>,
    /// Sectors from which it fires, classified relative to the target's rest
    /// floor.
    pub activators: Vec<(usize, Activator)>,
    /// Where it sits relative to the target.
    pub placement: Placement,
}

// ---------------------------------------------------------------------------
// Targets: one resolution per tagged sector some floor line names.
// ---------------------------------------------------------------------------

/// One `(engine type, target)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FloorAction {
    /// The engine type its lines dispatch.
    pub engine_type: FloorEngineType,
    /// Where the floor goes, at load-time heights.
    pub destination: Destination,
    /// The classification; `None` when the destination is unresolved.
    pub facts: Option<EffectFacts>,
    /// Indices into [`SceneFloor::triggers`] of this type's lines.
    pub triggers: Vec<usize>,
}

/// A sector some floor line names by tag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneFloor {
    /// Declaration index of the target sector.
    pub sector: usize,
    /// The tag floor lines name it by (never 0).
    pub tag: i32,
    /// Its floor at load.
    pub rest: i32,
    /// Its two-sided neighbors, itself excluded.
    pub neighbors: BTreeSet<usize>,
    /// How many sectors carry the tag (1 = this one alone).
    pub shared_tag: usize,
    /// Every floor line naming the tag.
    pub triggers: Vec<FloorTrigger>,
    /// One per engine type driving it.
    pub actions: Vec<FloorAction>,
    /// Non-floor, nonzero specials on lines naming the same tag, sorted and
    /// deduped — a lift, a door, a ceiling, a light sharing the tag.
    pub other_actions: Vec<i32>,
    /// Some neighbor is itself a floor target, a lift platform, or a door
    /// sector — the chain the compiler refuses (rule P30).
    pub borders_mover: bool,
}

impl SceneFloor {
    /// The single action of a one-type target, if it has exactly one.
    #[must_use]
    pub fn single(&self) -> Option<&FloorAction> {
        match self.actions.as_slice() {
            [a] => Some(a),
            _ => None,
        }
    }
}

/// One recognized floor line: its front mirror, the sector that mirror is
/// filed under, and the dispatch the sourced vocabulary gives its special.
///
/// Front mirrors only. `special` and `tag` are linedef-wide, so the back
/// mirror of a two-sided floor line would otherwise contribute the same
/// trigger a second time — the same reason [`resolve_plats`] collects its
/// lines that way.
struct FloorLine<'a> {
    /// Declaration index of the sector the line's front side faces.
    front: usize,
    /// The line's front mirror.
    boundary: &'a Boundary,
    /// The engine type its special dispatches.
    engine_type: FloorEngineType,
    /// How it fires.
    form: FloorForm,
}

/// Every recognized floor line in the scene, once each.
fn floor_lines<'a>(
    scene: &'a Scene,
    recognized: &BTreeMap<i32, (FloorEngineType, FloorForm)>,
) -> Vec<FloorLine<'a>> {
    scene
        .sectors
        .iter()
        .enumerate()
        .flat_map(|(i, s)| s.boundary.iter().map(move |b| (i, b)))
        .filter(|(_, b)| b.fronts_this)
        .filter_map(|(front, boundary)| {
            recognized
                .get(&boundary.special)
                .map(|&(engine_type, form)| FloorLine {
                    front,
                    boundary,
                    engine_type,
                    form,
                })
        })
        .collect()
}

/// `(special, engine type, form)` for every floor special the sourced
/// vocabulary recognizes, widened to the `i32` a [`Boundary`]'s own `special`
/// carries and resolved once per call rather than re-looked-up per line — the
/// same shape `plats::LiftSpecials` uses.
fn recognized_specials(tables: &Tables) -> BTreeMap<i32, (FloorEngineType, FloorForm)> {
    tables
        .recognized_floor_specials()
        .into_iter()
        .map(|(special, engine_type, form)| (i32::from(special), (engine_type, form)))
        .collect()
}

/// The sectors a floor target bordering one is chained to: every other floor
/// target, every lift platform, and every door sector.
///
/// A door sector is read off the **back** side of a door line rather than off
/// a tag, because every door special the vocabulary names
/// ([`Tables::door_special`] and [`Tables::locked_door_kinds`]) is a manual
/// `DR` form: `EV_VerticalDoor` (`p_doors.c`) takes
/// `sides[line->sidenum[1]].sector`, the back side, and such a line carries
/// no tag at all.
fn mover_sectors(scene: &Scene, tables: &Tables, targets: &BTreeSet<usize>) -> BTreeSet<usize> {
    let mut movers = targets.clone();
    movers.extend(resolve_plats(scene, tables).into_iter().map(|p| p.sector));
    let mut doors: Vec<i32> = vec![i32::from(tables.door_special())];
    doors.extend(
        tables
            .locked_door_kinds()
            .into_iter()
            .map(|(_, special)| i32::from(special)),
    );
    movers.extend(
        scene
            .sectors
            .iter()
            .flat_map(|s| s.boundary.iter())
            .filter(|b| b.fronts_this && doors.contains(&b.special))
            .filter_map(|b| b.neighbor),
    );
    movers
}

/// Every floor target, ascending by sector: each sector some floor line names
/// by a nonzero tag.
///
/// A floor line carrying tag 0, or a tag no sector answers to, resolves to no
/// target here at all — it fires nothing, which is [`broken_floor_lines`]'s
/// list rather than a floor to model.
///
/// A target's [`SceneFloor::triggers`] are listed in the scene's own walk
/// order (front sector ascending, then that sector's boundary order), and its
/// [`SceneFloor::actions`] in the order those triggers first name each engine
/// type — the same order [`resolve_plats`] lists a plat's triggers in.
#[must_use]
pub fn resolve_floors(scene: &Scene, tables: &Tables) -> Vec<SceneFloor> {
    let step = tables.step_height();
    let player = tables.player();
    let recognized = recognized_specials(tables);
    let lines = floor_lines(scene, &recognized);
    let named = sectors_named_by(scene, |s| recognized.contains_key(&s));
    let movers = mover_sectors(scene, tables, &named);
    named
        .iter()
        .map(|&sector| {
            let ss = &scene.sectors[sector];
            let neighbors: BTreeSet<usize> = ss
                .boundary
                .iter()
                .filter_map(next_sector)
                .filter(|&n| n != sector)
                .collect();
            let triggers = triggers_for(scene, &lines, sector, &neighbors, (step, player.radius));
            let actions = actions_for(scene, sector, &neighbors, &triggers, (player.height, step));
            SceneFloor {
                sector,
                tag: ss.tag,
                rest: ss.floor,
                shared_tag: scene.sectors.iter().filter(|s| s.tag == ss.tag).count(),
                borders_mover: neighbors.iter().any(|n| movers.contains(n)),
                neighbors,
                triggers,
                actions,
                other_actions: other_specials_on_tag(scene, ss.tag, |s| {
                    recognized.contains_key(&s)
                }),
            }
        })
        .collect()
}

/// Every floor line naming the sector at `target`'s tag, as the trigger it is
/// for that target: which sectors fire it, and each one's [`Activator`] class
/// relative to the target's rest floor.
///
/// `lines` is every recognized floor line in the scene, listed once per
/// linedef; `(step, radius)` are [`Tables::step_height`] and
/// [`Tables::player`]`().radius`.
fn triggers_for(
    scene: &Scene,
    lines: &[FloorLine],
    target: usize,
    neighbors: &BTreeSet<usize>,
    (step, radius): (i32, i32),
) -> Vec<FloorTrigger> {
    let tag = scene.sectors[target].tag;
    lines
        .iter()
        .filter(|l| l.boundary.tag == tag)
        .map(|l| FloorTrigger {
            linedef: l.boundary.linedef,
            special: l.boundary.special,
            engine_type: l.engine_type,
            form: l.form,
            front: Some(l.front),
            activators: activator_sides(
                scene,
                l.boundary,
                l.front,
                l.form.front_only(),
                (step, radius),
            )
            .into_iter()
            .map(|s| (s, classify(scene, target, s, step)))
            .collect(),
            placement: placement_of(target, neighbors, Some(l.front), l.boundary.neighbor),
        })
        .collect()
}

/// One action per engine type `triggers` drives, in the order the triggers
/// first name each type, with the destination that type computes and the
/// classification of the move it makes.
fn actions_for(
    scene: &Scene,
    target: usize,
    neighbors: &BTreeSet<usize>,
    triggers: &[FloorTrigger],
    (player_height, step): (i32, i32),
) -> Vec<FloorAction> {
    let rest = scene.sectors[target].floor;
    let mut actions: Vec<FloorAction> = Vec::new();
    for (i, t) in triggers.iter().enumerate() {
        if let Some(a) = actions.iter_mut().find(|a| a.engine_type == t.engine_type) {
            a.triggers.push(i);
            continue;
        }
        let destination = destination(scene, target, t.engine_type);
        actions.push(FloorAction {
            engine_type: t.engine_type,
            destination,
            facts: match destination {
                Destination::Height(d) => Some(classify_effect(
                    scene,
                    target,
                    neighbors,
                    rest,
                    d,
                    player_height,
                    step,
                )),
                Destination::NeedsTexture => None,
            },
            triggers: vec![i],
        });
    }
    actions
}

/// Floor lines that can never move a floor: tag 0, or a tag naming no sector.
///
/// `plats::broken_tag_lines` documents the ordering and the front-mirror rule.
#[must_use]
pub fn broken_floor_lines(scene: &Scene, tables: &Tables) -> Vec<usize> {
    let recognized = recognized_specials(tables);
    broken_tag_lines(scene, |s| recognized.contains_key(&s))
}

#[cfg(test)]
mod tests {
    use super::{
        Destination, Effect, OpeningShape, Placement, Rider, broken_floor_lines, resolve_floors,
    };
    use crate::check::fixtures::{chain, chain_full, far_wall, scene_of};
    use crate::tables::{FloorEngineType, FloorForm};

    #[test]
    fn a_wall_between_two_rooms_that_drops_flush_is_a_drop_wall() {
        // A(0) – T(128, ceiling 256) – B(0); a 23 S1 on B's far wall, tag 7.
        let mut text = chain_full(
            &[0, 128, 0],
            &[256, 256, 256],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let (scene, tables) = scene_of(&text);
        let floors = resolve_floors(&scene, &tables);
        assert_eq!(floors.len(), 1);
        let f = &floors[0];
        assert_eq!((f.sector, f.tag, f.rest, f.shared_tag), (1, 7, 128, 1));
        let a = &f.actions[0];
        assert_eq!(a.engine_type, FloorEngineType::LowerFloorToLowest);
        assert_eq!(a.destination, Destination::Height(0));
        let facts = a.facts.as_ref().expect("resolved");
        assert_eq!(
            (facts.effect, facts.rider, facts.opening),
            (
                Effect::Opening,
                Rider::NotApplicable,
                Some(OpeningShape::DropWall)
            )
        );
        assert!(facts.joins_neighbor_floor);
        assert_eq!(f.triggers[0].form, FloorForm::S1);
        assert_eq!(f.triggers[0].placement, Placement::Adjacent);
    }

    #[test]
    fn a_pit_strip_that_rises_to_the_walkway_is_a_bridge() {
        // A(64) – T(0) – B(64), 20 S1 (plat raiseToNearestAndChange).
        let mut text = chain(
            &[64, 0, 64],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 20, 7);
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        let facts = f.actions[0].facts.as_ref().expect("resolved");
        assert_eq!(f.actions[0].destination, Destination::Height(64));
        assert_eq!(
            (facts.effect, facts.rider, facts.opening),
            (Effect::Opening, Rider::Keeps, Some(OpeningShape::Bridge))
        );
    }

    #[test]
    fn a_sealed_one_neighbor_cell_that_lowers_into_reach_is_a_reveal() {
        // A(0, ceiling 256) – T(64, ceiling 64): T is sealed at rest (no
        // headroom at all) and its one neighbor can step onto it once the 23
        // has dropped it flush.
        let mut text = chain_full(&[0, 64], &[256, 64], &[0, 7], &[(0, 0, false)], "");
        far_wall(&mut text, 2, 23, 7);
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        let facts = f.actions[0].facts.as_ref().expect("resolved");
        assert_eq!(
            (
                facts.effect,
                facts.opening,
                facts.enterable_before,
                facts.enterable_after
            ),
            (Effect::Neutral, Some(OpeningShape::Reveal), false, true)
        );
    }

    #[test]
    fn a_pillar_that_rises_to_the_ceiling_closes() {
        let mut text = chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        far_wall(&mut text, 3, 101, 7);
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        let facts = f.actions[0].facts.as_ref().expect("resolved");
        assert_eq!(f.actions[0].destination, Destination::Height(256));
        assert_eq!(
            (facts.effect, facts.rider, facts.opening),
            (Effect::Closing, Rider::Loses, None)
        );
    }

    #[test]
    fn a_descender_is_neutral_for_others_and_strands_its_rider() {
        let mut text = chain(
            &[128, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let (scene, tables) = scene_of(&text);
        let facts = resolve_floors(&scene, &tables)[0].actions[0]
            .facts
            .clone()
            .expect("resolved");
        assert_eq!(
            (facts.effect, facts.rider, facts.opening),
            (Effect::Neutral, Rider::Loses, None)
        );
    }

    #[test]
    fn a_target_at_its_destination_is_dead_and_the_no_neighbor_lower_goes_to_minus_500() {
        let mut text = chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        far_wall(&mut text, 3, 18, 7);
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        assert_eq!(f.actions[0].destination, Destination::Height(0));
        assert_eq!(
            f.actions[0].facts.as_ref().expect("resolved").effect,
            Effect::Dead
        );
        // A lone tagged box with a 19 line on its own wall.
        let mut lone = chain(&[0], &[7], &[], "");
        far_wall(&mut lone, 1, 19, 7);
        let (scene, tables) = scene_of(&lone);
        assert_eq!(
            resolve_floors(&scene, &tables)[0].actions[0].destination,
            Destination::Height(-500)
        );
    }

    #[test]
    fn a_raise_capped_at_the_targets_own_ceiling_and_the_turbo_plus_eight() {
        let mut text = chain_full(
            &[0, 0, 0],
            &[256, 128, 256],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 101, 7);
        let (scene, tables) = scene_of(&text);
        assert_eq!(
            resolve_floors(&scene, &tables)[0].actions[0].destination,
            Destination::Height(128)
        );
        let mut text = chain(&[0, 64, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        far_wall(&mut text, 3, 71, 7);
        let (scene, tables) = scene_of(&text);
        assert_eq!(
            resolve_floors(&scene, &tables)[0].actions[0].destination,
            Destination::Height(8)
        );
    }

    #[test]
    fn two_families_on_one_tag_are_two_actions_and_a_lift_special_is_a_conflict() {
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(23, 7, false), (18, 7, false)],
            "",
        );
        far_wall(&mut text, 3, 62, 7);
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        assert_eq!(f.actions.len(), 2);
        assert_eq!(f.other_actions, vec![62]);
        assert!(f.single().is_none(), "two types are not one action");
    }

    /// The seven destination formulas the ten worked examples do not reach,
    /// and the one that stays unresolved. Every expectation is the family's
    /// own arithmetic on a flat 3-room chain (floors 0, ceilings 256): the
    /// target's own floor for the `+ n` families, and the neighbors' ceilings
    /// for `raiseFloorCrush`.
    #[test]
    fn the_remaining_destination_formulas_resolve_and_a_texture_raise_does_not() {
        // (special, engine type, destination)
        let cases = [
            (
                55,
                FloorEngineType::RaiseFloorCrush,
                Destination::Height(248),
            ),
            (58, FloorEngineType::RaiseFloor24, Destination::Height(24)),
            (
                14,
                FloorEngineType::PlatRaiseAndChange32,
                Destination::Height(32),
            ),
            (
                140,
                FloorEngineType::RaiseFloor512,
                Destination::Height(512),
            ),
            (37, FloorEngineType::LowerAndChange, Destination::Height(0)),
            (
                30,
                FloorEngineType::RaiseToTexture,
                Destination::NeedsTexture,
            ),
        ];
        for (special, engine_type, destination) in cases {
            let mut text = chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
            far_wall(&mut text, 3, special, 7);
            let (scene, tables) = scene_of(&text);
            let f = &resolve_floors(&scene, &tables)[0];
            let a = f.single().expect("one type drives the target");
            assert_eq!((a.engine_type, a.destination), (engine_type, destination));
            assert_eq!(
                a.triggers,
                vec![0],
                "the one trigger is the action's own: {special}"
            );
            assert_eq!(
                a.facts.is_none(),
                destination == Destination::NeedsTexture,
                "an unresolved destination classifies nothing: {special}"
            );
        }
    }

    /// A ledge the player can already step onto, lowered flush: the opening
    /// sub-shape that separates a drop wall (sealed before) from a ledge.
    /// The switch sits two rooms away, which is [`Placement::Remote`] — a
    /// side that is neither the target nor one of its neighbors.
    #[test]
    fn a_step_up_ledge_that_lowers_flush_is_a_ledge_lower_fired_from_a_remote_switch() {
        // A(24) – T(48) – B(0) – C(0); the 23 sits on C's far wall.
        let mut text = chain(
            &[24, 48, 0, 0],
            &[0, 7, 0, 0],
            &[(0, 0, false), (0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 4, 23, 7);
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        let a = f.single().expect("one type drives the target");
        assert_eq!(a.destination, Destination::Height(0));
        let facts = a.facts.as_ref().expect("resolved");
        assert_eq!(
            (facts.effect, facts.rider, facts.opening),
            (
                Effect::Opening,
                Rider::Keeps,
                Some(OpeningShape::LedgeLower)
            )
        );
        assert!(facts.enterable_before, "A is one step below T at rest");
        assert_eq!(f.triggers[0].placement, Placement::Remote);
    }

    /// The §G chain fact: a neighbor that is itself a floor target, a lift
    /// platform, or a door sector. The three ways in, and the shape that is
    /// none of them.
    #[test]
    fn a_neighbor_that_moves_is_a_chain() {
        // A lone target between two plain rooms borders nothing that moves.
        let mut text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let (scene, tables) = scene_of(&text);
        let floors = resolve_floors(&scene, &tables);
        assert!(!floors[0].borders_mover);

        // A second sector carrying the same tag: each is the other's mover,
        // and both answer to the one tag.
        let (scene, tables) = scene_of(&chain(
            &[0, 128, 0],
            &[7, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        let floors = resolve_floors(&scene, &tables);
        assert_eq!(floors.len(), 2);
        assert!(floors.iter().all(|f| f.borders_mover && f.shared_tag == 2));

        // A lift platform next door: `resolve_plats` names sector 2, which is
        // the target's neighbor.
        let (scene, tables) = scene_of(&chain(
            &[0, 128, 0],
            &[0, 7, 5],
            &[(23, 7, false), (62, 5, false)],
            "",
        ));
        let f = &resolve_floors(&scene, &tables)[0];
        assert_eq!((f.sector, f.borders_mover), (1, true));

        // A manual door: special 1 carries no tag, and the sector it opens is
        // the line's back side — here the target's east neighbor.
        let (scene, tables) = scene_of(&chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(23, 7, false), (1, 0, false)],
            "",
        ));
        let f = &resolve_floors(&scene, &tables)[0];
        assert_eq!((f.sector, f.borders_mover), (1, true));
        assert!(
            f.other_actions.is_empty(),
            "the door names no tag, so it is no conflict: {:?}",
            f.other_actions
        );
    }

    #[test]
    fn broken_lines_are_tag_zero_and_dangling() {
        let mut text = chain(
            &[0, 0, 0],
            &[0, 0, 0],
            &[(23, 0, false), (38, 9, false)],
            "",
        );
        far_wall(&mut text, 3, 0, 0);
        let (scene, tables) = scene_of(&text);
        assert!(resolve_floors(&scene, &tables).is_empty());
        assert_eq!(broken_floor_lines(&scene, &tables), vec![0, 1]);
    }

    #[test]
    fn use_lines_fire_from_the_front_and_walkovers_from_either_crossable_side() {
        // 23 on the A|T link fronted by A: activator A only. 38 on T|B, which
        // T is 128 above: the step rule is on the crossing *onto* the other
        // side, so T's side fires it (dropping to B is free) and B's does not
        // (climbing 128 is not a step).
        let text = chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(23, 7, false), (38, 7, false)],
            "",
        );
        let (scene, tables) = scene_of(&text);
        let f = &resolve_floors(&scene, &tables)[0];
        let s = f
            .triggers
            .iter()
            .find(|t| t.special == 23)
            .expect("the 23 names tag 7");
        assert_eq!(
            s.activators.iter().map(|&(x, _)| x).collect::<Vec<_>>(),
            vec![0]
        );
        assert_eq!(s.placement, Placement::OnTargetBack);
        let w = f
            .triggers
            .iter()
            .find(|t| t.special == 38)
            .expect("the 38 names tag 7");
        assert_eq!(
            w.activators.iter().map(|&(x, _)| x).collect::<Vec<_>>(),
            vec![1]
        );
    }
}
