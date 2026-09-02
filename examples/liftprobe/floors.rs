//! Pass 3 — the floor-action shape probe: what a *tagged floor action* is in
//! the maps we have, so the IR, the compiler, the reachability flood and the
//! recognizer for sub-project 4a can all state the same thing.
//!
//! A **floor line** is a linedef whose special is one the pinned engine
//! dispatches to `EV_DoFloor` or to one of the two `raiseAndChange` plats
//! (see [`crate::common::FLOOR_FAMILIES`]). Its **target** is every sector
//! carrying its tag, and an **action** is one `(family, target)` pair. The
//! pass evaluates each action's destination at load-time heights, then
//! classifies what the move does to the local graph `{T} ∪ N(T)`: whether it
//! opens a route, closes one, does both, does neither, or is a no-op — and
//! whether a player already standing on the target keeps what it could reach.
//!
//! Every engine constant and formula it reads is cited at its definition in
//! [`crate::common`]; nothing here is written from memory.

use std::collections::{BTreeMap, BTreeSet};

use crustygen::check::scene::{Boundary, Scene};
use crustygen::lift::{self, vocabulary::Vocabulary};
use crustygen::tables::Tables;
use crustywad::map::udmf::{UdmfMap, UdmfSidedef};

use crate::common::{
    self, Activator, CEILING, DONUT, Destination, Effect, EffectFacts, FLOOR_GUN, FlatSource,
    FloorTrigger, FloorType, Hist, OpeningShape, PERPETUAL, Placement, RAISE_CEILING_LOWER_FLOOR,
    REPEATABLE_LIFT, Rider, STAIRS, Shape, TriggerKind, destination, floor_all, floor_type,
    highest_floor_surrounding, lowest_ceiling_surrounding, lowest_floor_surrounding,
    next_highest_floor, pct, percentiles,
};

/// One `(family, target)` action: what the lines of one engine type do to one
/// tagged sector.
struct Action {
    /// The engine type its lines dispatch.
    ty: FloorType,
    /// Where the floor is sent, at load-time heights.
    destination: Destination,
    /// `|d − f|`, or 0 when the destination is unresolved.
    travel: i32,
    /// The classification, absent when the destination is unresolved.
    facts: Option<EffectFacts>,
    /// `P_FindNextHighestFloor` filled its 20-entry list and broke early.
    next_highest_capped: bool,
    /// `P_FindNextHighestFloor` found nothing above and returned the current
    /// height — the action is a no-op for that reason specifically.
    next_highest_noop: bool,
    /// A `lowerFloor`/`turboLower` on a sector with no two-sided neighbor,
    /// whose destination is therefore the engine's −500 sentinel.
    no_neighbor: bool,
    /// The neighbors whose own height defines the destination.
    dest_neighbors: BTreeSet<usize>,
    /// Indices into [`TargetFacts::triggers`] of this family's lines.
    triggers: Vec<usize>,
}

/// Everything the pass reads about one target.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent measured fact about the target (which other special \
              names its tag, which kind of movable sector borders it); they encode no joint state"
)]
struct TargetFacts {
    /// Declaration index of the tagged sector.
    sector: usize,
    /// How many sectors share this target's tag.
    shared_tag_n: usize,
    /// The target's floor at load.
    rest: i32,
    /// The target's ceiling.
    ceiling: i32,
    /// The target's two-sided neighbors, itself excluded.
    neighbors: BTreeSet<usize>,
    /// Every floor line naming the target's tag.
    triggers: Vec<FloorTrigger>,
    /// One per family driving the target.
    actions: Vec<Action>,
    /// A lift special also names this tag.
    conflict_lift: bool,
    /// Another special crustygen can emit (door, exit, teleport) also names it.
    conflict_emittable: bool,
    /// A second floor family also names it.
    conflict_second_family: bool,
    /// Both a raise family and a lower family name it — the two-way elevator.
    conflict_two_way: bool,
    /// Some other special entirely also names it.
    conflict_other: bool,
    /// A neighbor is itself a floor target.
    nb_floor_target: bool,
    /// A neighbor is a lift plat.
    nb_lift_plat: bool,
    /// A neighbor is tagged by an emittable door special.
    nb_door: bool,
    /// The destination-defining neighbor of some action is itself movable.
    dest_neighbor_movable: bool,
    /// The names of the things standing on the target.
    things: Vec<String>,
    /// The target's bounding box, `None` without resolved geometry.
    bbox: Option<(i32, i32)>,
}

impl TargetFacts {
    /// Whether the target could be emitted by a v1 floor construct: one
    /// opening action, no stranded rider, a tag of its own, no conflicting
    /// special, a resolved destination, and at least one non-gun trigger a
    /// player can actually fire.
    fn v1_candidate(&self) -> bool {
        let [action] = &self.actions[..] else {
            return false;
        };
        let Some(facts) = &action.facts else {
            return false;
        };
        facts.effect == Effect::Opening
            && facts.rider != Rider::Loses
            && self.shared_tag_n == 1
            && !self.conflicted()
            && self.triggers.iter().all(|t| t.kind != TriggerKind::Gun)
            && self.triggers.iter().any(has_activator)
    }

    /// Whether any special outside this target's own family names its tag.
    fn conflicted(&self) -> bool {
        self.conflict_lift || self.conflict_emittable || self.conflict_other
    }

    /// The single opening sub-shape of a one-action target, if it has one.
    fn opening(&self) -> Option<OpeningShape> {
        match &self.actions[..] {
            [action] => action.facts.as_ref().and_then(|f| f.opening),
            _ => None,
        }
    }
}

/// Whether some side of a trigger can actually fire it.
fn has_activator(t: &FloorTrigger) -> bool {
    !t.activators.contains(&Activator::None)
}

/// Per-map lookups the target analysis needs beyond [`common::MapIndex`].
struct MapCtx<'a> {
    map: &'a UdmfMap,
    scene: &'a Scene,
    index: common::MapIndex<'a>,
    step: i32,
    player_height: i32,
    /// Sectors some floor line names by tag.
    floor_targets: BTreeSet<usize>,
    /// Sectors some lift line names by tag.
    lift_plats: BTreeSet<usize>,
    /// Sectors an emittable door special names by tag.
    door_sectors: BTreeSet<usize>,
    /// The eight lift specials.
    lift_specials: BTreeSet<i32>,
    /// Every special crustygen can emit that is neither a lift nor a floor.
    other_emittable: BTreeSet<i32>,
}

/// The neighbors whose own height defines `ty`'s destination for `target`.
fn destination_neighbors(
    scene: &Scene,
    target: usize,
    neighbors: &BTreeSet<usize>,
    ty: FloorType,
) -> BTreeSet<usize> {
    let rest = scene.sectors[target].floor;
    let at_floor = |h: i32| -> BTreeSet<usize> {
        neighbors
            .iter()
            .copied()
            .filter(|&n| scene.sectors[n].floor == h)
            .collect()
    };
    match ty {
        FloorType::LowerFloor | FloorType::TurboLower => {
            at_floor(highest_floor_surrounding(scene, target))
        }
        FloorType::LowerFloorToLowest | FloorType::LowerAndChange => {
            let low = lowest_floor_surrounding(scene, target);
            // The search starts at the sector's own floor, so when nothing is
            // below it no neighbor defines the destination.
            if low < rest {
                at_floor(low)
            } else {
                BTreeSet::new()
            }
        }
        FloorType::RaiseFloorToNearest
        | FloorType::RaiseFloorTurbo
        | FloorType::PlatRaiseToNearestAndChange => {
            let next = next_highest_floor(scene, target, rest).height;
            if next > rest {
                at_floor(next)
            } else {
                BTreeSet::new()
            }
        }
        FloorType::RaiseFloor | FloorType::RaiseFloorCrush => {
            let low = lowest_ceiling_surrounding(scene, target);
            neighbors
                .iter()
                .copied()
                .filter(|&n| scene.sectors[n].ceiling == low)
                .collect()
        }
        _ => BTreeSet::new(),
    }
}

/// The two-sided neighbors of `sec`, itself excluded.
fn neighbors_of(scene: &Scene, sec: usize) -> BTreeSet<usize> {
    scene.sectors[sec]
        .boundary
        .iter()
        .filter(|b| b.two_sided)
        .filter_map(|b| b.neighbor)
        .filter(|&n| n != sec)
        .collect()
}

/// Whether the `raiseToNearest`-style search decides `ty`'s destination.
fn uses_next_highest(ty: FloorType) -> bool {
    matches!(
        ty,
        FloorType::RaiseFloorToNearest
            | FloorType::RaiseFloorTurbo
            | FloorType::PlatRaiseToNearestAndChange
    )
}

/// The action one family's lines drive on `sector`.
fn build_action(
    ctx: &MapCtx<'_>,
    sector: usize,
    rest: i32,
    neighbors: &BTreeSet<usize>,
    ty: FloorType,
    lines: Vec<usize>,
) -> Action {
    let dest = destination(ctx.scene, sector, ty);
    let next = next_highest_floor(ctx.scene, sector, rest);
    Action {
        ty,
        destination: dest,
        travel: match dest {
            Destination::Height(d) => (d - rest).abs(),
            Destination::NeedsTexture => 0,
        },
        facts: match dest {
            Destination::Height(d) => Some(common::classify_effect(
                ctx.scene,
                sector,
                neighbors,
                rest,
                d,
                ctx.player_height,
                ctx.step,
            )),
            Destination::NeedsTexture => None,
        },
        next_highest_capped: uses_next_highest(ty) && next.capped,
        next_highest_noop: uses_next_highest(ty) && next.height == rest,
        no_neighbor: matches!(ty, FloorType::LowerFloor | FloorType::TurboLower)
            && neighbors.is_empty(),
        dest_neighbors: destination_neighbors(ctx.scene, sector, neighbors, ty),
        triggers: lines,
    }
}

/// Every special naming `tag` that is not a floor line, split the way §B asks
/// for: `(a lift special, another emittable special, anything else)`.
fn conflicts(ctx: &MapCtx<'_>, tag: i32) -> (bool, bool, bool) {
    let (mut lift, mut emittable, mut other) = (false, false, false);
    for l in &ctx.map.linedefs {
        if l.args[0] != tag || l.special == 0 || floor_type(l.special).is_some() {
            continue;
        }
        if ctx.lift_specials.contains(&l.special) {
            lift = true;
        } else if ctx.other_emittable.contains(&l.special) {
            emittable = true;
        } else {
            other = true;
        }
    }
    (lift, emittable, other)
}

/// Analyzes the target at `sector`. `None` when the sector's tag is 0 or no
/// floor line names it.
fn analyze_target(ctx: &MapCtx<'_>, sector: usize) -> Option<TargetFacts> {
    let tag = ctx.map.sectors[sector].id;
    if tag == 0 {
        return None;
    }
    let neighbors = neighbors_of(ctx.scene, sector);
    let hops = common::hop_distances(ctx.scene, sector);
    let triggers = common::floor_triggers(ctx.map, ctx.scene, sector, &neighbors, &hops, ctx.step);
    if triggers.is_empty() {
        return None;
    }
    let rest = ctx.scene.sectors[sector].floor;

    let mut by_family: BTreeMap<FloorType, Vec<usize>> = BTreeMap::new();
    for (i, t) in triggers.iter().enumerate() {
        by_family.entry(t.ty).or_default().push(i);
    }
    let actions: Vec<Action> = by_family
        .into_iter()
        .map(|(ty, lines)| build_action(ctx, sector, rest, &neighbors, ty, lines))
        .collect();
    let families: BTreeSet<FloorType> = actions.iter().map(|a| a.ty).collect();
    let (conflict_lift, conflict_emittable, conflict_other) = conflicts(ctx, tag);
    let movable = |n: usize| ctx.floor_targets.contains(&n) || ctx.lift_plats.contains(&n);
    let things = ctx
        .index
        .things_in
        .get(&sector)
        .map(|v| {
            v.iter()
                .map(|t| {
                    t.name
                        .clone()
                        .unwrap_or_else(|| format!("type {}", t.type_id))
                })
                .collect()
        })
        .unwrap_or_default();

    Some(TargetFacts {
        sector,
        shared_tag_n: ctx.index.by_tag.get(&tag).map_or(0, Vec::len),
        rest,
        ceiling: ctx.scene.sectors[sector].ceiling,
        conflict_lift,
        conflict_emittable,
        conflict_second_family: families.len() > 1,
        conflict_two_way: families.iter().any(|f| f.raises())
            && families.iter().any(|f| !f.raises()),
        conflict_other,
        nb_floor_target: neighbors.iter().any(|&n| ctx.floor_targets.contains(&n)),
        nb_lift_plat: neighbors.iter().any(|&n| ctx.lift_plats.contains(&n)),
        nb_door: neighbors.iter().any(|&n| ctx.door_sectors.contains(&n)),
        dest_neighbor_movable: actions
            .iter()
            .any(|a| a.dest_neighbors.iter().copied().any(movable)),
        things,
        bbox: common::sector_bbox(ctx.scene, sector).map(|(_, w, h)| (w, h)),
        neighbors,
        triggers,
        actions,
    })
}

// ---------------------------------------------------------------------------
// Aggregates
// ---------------------------------------------------------------------------

/// §D — one opening sub-shape's facts.
#[derive(Default)]
struct OpeningAgg {
    n: u64,
    neighbor_count: Hist,
    solid_at_rest: u64,
    flush_at_rest: u64,
    dims: Hist,
    min_side: Hist,
    geometry_n: u64,
    travel: Vec<i32>,
    joins_neighbor_floor: u64,
    new_pairs: Hist,
    bidirectional: u64,
    flat_eq_nb: u64,
    light_eq_nb: u64,
    ceiling_eq_all: u64,
    things_any: u64,
    thing_names: Hist,
    closets: u64,
    closet_things: Hist,
    /// §E, per sub-shape.
    kind_combo: Hist,
    repeat_split: Hist,
}

/// §F — the sidedefs the engine draws on a target's boundaries.
#[derive(Default)]
struct RenderAgg {
    lower_tex: Hist,
    lower_n: u64,
    lower_unpegged: u64,
    upper_tex: Hist,
    upper_n: u64,
    upper_unpegged: u64,
    middle_n: u64,
    middle_nonblank: u64,
}

/// §I(i) — a perpetual plat, which sub-project 4b inherits.
#[derive(Default)]
struct PerpetualAgg {
    n: u64,
    travel: Vec<i32>,
    rest: Hist,
    neighbor_count: Hist,
    has_stop: u64,
    hops: Hist,
    things_any: u64,
}

/// Everything the report prints.
#[derive(Default)]
struct Agg {
    maps: u64,
    maps_with_floor_line: u64,
    // A
    family_lines: BTreeMap<FloorType, u64>,
    family_maps: BTreeMap<FloorType, u64>,
    kind_lines: Hist,
    kind_maps: Hist,
    also_raises_ceiling: u64,
    adjacent_lines: Hist,
    adjacent_maps: Hist,
    // B
    tag0_lines: u64,
    dangling_lines: u64,
    targets: u64,
    groups_n: u64,
    groups_size: Hist,
    groups_one_floor_connected: u64,
    groups_one_floor_disconnected: u64,
    groups_several_floors: u64,
    conflict_lift: u64,
    conflict_emittable: u64,
    conflict_second_family: u64,
    conflict_two_way: u64,
    conflict_other: u64,
    maps_conflict_lift: u64,
    maps_conflict_emittable: u64,
    maps_conflict_second_family: u64,
    maps_conflict_two_way: u64,
    maps_conflict_other: u64,
    // C
    actions: u64,
    family_effect: BTreeMap<FloorType, Hist>,
    family_travel: BTreeMap<FloorType, Vec<i32>>,
    quirk_no_neighbor: u64,
    quirk_next_highest_noop: u64,
    quirk_next_highest_capped: u64,
    quirk_needs_texture: u64,
    quirk_direction_disagrees: u64,
    effect_table: Hist,
    // D
    openings: BTreeMap<OpeningShape, OpeningAgg>,
    // E
    trigger_special: Hist,
    trigger_kind: Hist,
    trigger_repeatable: Hist,
    trigger_placement: Hist,
    trigger_hops: Hist,
    trigger_activator: Hist,
    switch_lines: u64,
    switch_with_texture: u64,
    walk_lines: u64,
    walk_trip: u64,
    triggers_per_target: Hist,
    // F
    render_wall: RenderAgg,
    render_bridge: RenderAgg,
    bridge_flat_eq_walkway: u64,
    bridge_flat_n: u64,
    bridge_change_flat_eq_walkway: u64,
    bridge_change_flat_n: u64,
    // G
    chain_nb_floor: u64,
    chain_nb_lift: u64,
    chain_nb_door: u64,
    chain_two_families: u64,
    chain_dest_movable: u64,
    // H
    baseline_all_axes: u64,
    naive_line: u64,
    naive_all: u64,
    gated_line: u64,
    gated_all: u64,
    no_remote_line: u64,
    no_remote_all: u64,
    no_chain_line: u64,
    no_chain_all: u64,
    maps_refused: u64,
    refusal: Hist,
    // I
    perpetual: PerpetualAgg,
    one_shot_lift_shapes: BTreeMap<Shape, u64>,
    mixed_lift_shapes: BTreeMap<Shape, u64>,
}

/// Runs the floor-action pass over `dirs` and prints the report for `label`.
pub(crate) fn run(label: &str, dirs: &[String]) {
    let tables = Tables::load().expect("tables");
    let vocab = Vocabulary::from_tables(&tables);
    let mut agg = Agg::default();
    let maps = common::sweep(dirs, |name, map| {
        survey_map(name, map, &tables, &vocab, &mut agg);
    });
    agg.maps = maps;
    report(label, &agg);
}

/// The arbiter facts the shared vocabulary layer decides for one map.
struct MapVerdict {
    /// Every axis but the linedef specials is expressible.
    others_ok: bool,
    /// The map is expressible today, on every axis — the baseline §H
    /// reproduces before it proposes anything.
    expressible: bool,
    /// Every out-of-set linedef special is a non-gun floor special: the
    /// naïve line axis, which accepts the whole §1.1 table but not the two
    /// gun forms, since those stay out of the emittable set.
    line_ok: bool,
}

fn map_verdict(
    name: &str,
    map: &UdmfMap,
    scene: &Scene,
    tables: &Tables,
    vocab: &Vocabulary,
) -> MapVerdict {
    let telemetry = lift::survey(name, map);
    let mut verdict = vocab.classify(&telemetry);
    // The same gating `crustygen-corpus` uses (`src/lift/corpus.rs:295-325`):
    // each recognizer runs only when its specials are present, so a map
    // without them keeps `classify`'s own `true`.
    if tables
        .teleport_specials()
        .into_iter()
        .any(|s| telemetry.linedef_specials.contains_key(&i32::from(s)))
    {
        verdict = verdict.with_teleports(&lift::teleport::recognize(scene, tables));
    }
    if tables
        .lift_specials()
        .into_iter()
        .any(|s| telemetry.linedef_specials.contains_key(&i32::from(s)))
    {
        verdict = verdict.with_lifts(&lift::plat::recognize(scene, tables));
    }
    let non_gun: BTreeSet<i32> = floor_all()
        .into_iter()
        .filter(|s| !FLOOR_GUN.contains(s))
        .collect();
    MapVerdict {
        others_ok: verdict.sector_specials_ok
            && verdict.thing_kinds_ok
            && verdict.teleports_ok
            && verdict.lifts_ok,
        expressible: verdict.expressible,
        line_ok: verdict
            .unknown_line_specials
            .iter()
            .all(|s| non_gun.contains(s)),
    }
}

/// The per-map lookups the target analysis needs.
fn map_ctx<'a>(map: &'a UdmfMap, scene: &'a Scene, tables: &Tables) -> MapCtx<'a> {
    let index = common::MapIndex::build(map, scene);
    let lift_specials: BTreeSet<i32> = tables.lift_specials().into_iter().map(i32::from).collect();
    let door_specials: BTreeSet<i32> = std::iter::once(tables.door_special())
        .chain(tables.locked_door_kinds().into_iter().map(|(_, s)| s))
        .map(i32::from)
        .collect();
    let all_floor = floor_all();
    let other_emittable: BTreeSet<i32> = tables
        .emittable_line_specials()
        .into_iter()
        .map(i32::from)
        .filter(|s| !lift_specials.contains(s) && !all_floor.contains(s))
        .collect();
    let tagged = |specials: &BTreeSet<i32>| -> BTreeSet<usize> {
        let mut out = BTreeSet::new();
        for l in &map.linedefs {
            if l.args[0] != 0
                && specials.contains(&l.special)
                && let Some(v) = index.by_tag.get(&l.args[0])
            {
                out.extend(v.iter().copied());
            }
        }
        out
    };
    MapCtx {
        floor_targets: tagged(&all_floor),
        lift_plats: tagged(&lift_specials),
        door_sectors: tagged(&door_specials),
        map,
        scene,
        step: tables.step_height(),
        player_height: tables.player().height,
        lift_specials,
        other_emittable,
        index,
    }
}

fn survey_map(name: &str, map: &UdmfMap, tables: &Tables, vocab: &Vocabulary, agg: &mut Agg) {
    let scene = Scene::build(map, tables, &mut Vec::new());
    let verdict = map_verdict(name, map, &scene, tables, vocab);
    agg.baseline_all_axes += u64::from(verdict.expressible);

    survey_specials(map, agg);
    let ctx = map_ctx(map, &scene, tables);
    survey_lift_carryover(&ctx, tables, agg);

    let mut bad_tag = false;
    let mut has_gun = false;
    let mut has_floor_line = false;
    for l in &map.linedefs {
        if floor_type(l.special).is_none() {
            continue;
        }
        has_floor_line = true;
        if l.args[0] == 0 {
            agg.tag0_lines += 1;
            bad_tag = true;
        } else if !ctx.index.by_tag.contains_key(&l.args[0]) {
            agg.dangling_lines += 1;
            bad_tag = true;
        }
        has_gun |= FLOOR_GUN.contains(&l.special);
    }

    let mut targets: Vec<TargetFacts> = Vec::new();
    if has_floor_line {
        agg.maps_with_floor_line += 1;
        targets = ctx
            .floor_targets
            .iter()
            .filter_map(|&s| analyze_target(&ctx, s))
            .collect();
        survey_tag_groups(&ctx, agg);
        for t in &targets {
            record_target(&ctx, t, agg);
        }
        agg.maps_conflict_lift += u64::from(targets.iter().any(|t| t.conflict_lift));
        agg.maps_conflict_emittable += u64::from(targets.iter().any(|t| t.conflict_emittable));
        agg.maps_conflict_second_family +=
            u64::from(targets.iter().any(|t| t.conflict_second_family));
        agg.maps_conflict_two_way += u64::from(targets.iter().any(|t| t.conflict_two_way));
        agg.maps_conflict_other += u64::from(targets.iter().any(|t| t.conflict_other));
    }
    record_arbiter(&targets, &verdict, bad_tag, has_gun, has_floor_line, agg);
}

/// §A — every tracked special's line and map counts.
fn survey_specials(map: &UdmfMap, agg: &mut Agg) {
    let mut families: BTreeSet<FloorType> = BTreeSet::new();
    let mut kinds: BTreeSet<&'static str> = BTreeSet::new();
    let mut adjacent: BTreeSet<&'static str> = BTreeSet::new();
    for l in &map.linedefs {
        if let Some((ty, kind, repeatable)) = floor_type(l.special) {
            *agg.family_lines.entry(ty).or_default() += 1;
            families.insert(ty);
            let label = trigger_label(kind, repeatable);
            agg.kind_lines.add(label);
            kinds.insert(label);
            agg.also_raises_ceiling += u64::from(l.special == RAISE_CEILING_LOWER_FLOOR);
        }
        for (name, set) in [
            ("stairs", &STAIRS[..]),
            ("donut", &DONUT[..]),
            ("ceiling", &CEILING[..]),
        ] {
            if set.contains(&l.special) {
                agg.adjacent_lines.add(name);
                adjacent.insert(name);
            }
        }
    }
    for ty in families {
        *agg.family_maps.entry(ty).or_default() += 1;
    }
    for k in kinds {
        agg.kind_maps.add(k);
    }
    for a in adjacent {
        agg.adjacent_maps.add(a);
    }
}

/// The `W1`/`WR`/`S1`/`SR`/`G1` label of a trigger.
fn trigger_label(kind: TriggerKind, repeatable: bool) -> &'static str {
    match (kind, repeatable) {
        (TriggerKind::Walk, false) => "W1",
        (TriggerKind::Walk, true) => "WR",
        (TriggerKind::Switch, false) => "S1",
        (TriggerKind::Switch, true) => "SR",
        (TriggerKind::Gun, _) => "G1",
    }
}

/// §B — a floor tag naming several sectors, classified the way `shapes.rs`
/// classifies a lift tag group.
fn survey_tag_groups(ctx: &MapCtx<'_>, agg: &mut Agg) {
    let tags: BTreeSet<i32> = ctx
        .map
        .linedefs
        .iter()
        .filter(|l| floor_type(l.special).is_some() && l.args[0] != 0)
        .map(|l| l.args[0])
        .collect();
    for tag in tags {
        let Some(secs) = ctx.index.by_tag.get(&tag) else {
            continue;
        };
        if secs.len() < 2 {
            continue;
        }
        agg.groups_n += 1;
        agg.groups_size.add(match secs.len() {
            2 => "2",
            3 => "3",
            4 => "4",
            _ => "5+",
        });
        let floors: BTreeSet<i32> = secs.iter().map(|&s| ctx.scene.sectors[s].floor).collect();
        if floors.len() > 1 {
            agg.groups_several_floors += 1;
            continue;
        }
        let set: BTreeSet<usize> = secs.iter().copied().collect();
        let mut reached: BTreeSet<usize> = BTreeSet::new();
        let mut stack = vec![secs[0]];
        while let Some(s) = stack.pop() {
            if !reached.insert(s) {
                continue;
            }
            stack.extend(
                ctx.scene.sectors[s]
                    .boundary
                    .iter()
                    .filter_map(|b| b.neighbor)
                    .filter(|n| set.contains(n) && !reached.contains(n)),
            );
        }
        if reached.len() == set.len() {
            agg.groups_one_floor_connected += 1;
        } else {
            agg.groups_one_floor_disconnected += 1;
        }
    }
}

/// §I — what sub-project 4b inherits: the perpetual plats and the one-shot
/// lift plats the lift work deliberately left out.
fn survey_lift_carryover(ctx: &MapCtx<'_>, tables: &Tables, agg: &mut Agg) {
    let perpetual_tags: BTreeSet<i32> = ctx
        .map
        .linedefs
        .iter()
        .filter(|l| PERPETUAL[..2].contains(&l.special) && l.args[0] != 0)
        .map(|l| l.args[0])
        .collect();
    let stop_tags: BTreeSet<i32> = ctx
        .map
        .linedefs
        .iter()
        .filter(|l| PERPETUAL[2..].contains(&l.special) && l.args[0] != 0)
        .map(|l| l.args[0])
        .collect();
    for tag in &perpetual_tags {
        for &sec in ctx.index.by_tag.get(tag).map_or(&[][..], Vec::as_slice) {
            let floor = ctx.scene.sectors[sec].floor;
            // `p_plats.c:233-247`: both bounds are clamped back to the
            // sector's own floor when the search overshoots it.
            let low = lowest_floor_surrounding(ctx.scene, sec).min(floor);
            let high = highest_floor_surrounding(ctx.scene, sec).max(floor);
            let p = &mut agg.perpetual;
            p.n += 1;
            p.travel.push(high - low);
            p.rest.add(match (floor == low, floor == high) {
                (true, true) => "dead (low == high)",
                (true, false) => "at low",
                (false, true) => "at high",
                (false, false) => "between",
            });
            let neighbors = neighbors_of(ctx.scene, sec);
            p.neighbor_count.add(bucket_count(neighbors.len()));
            p.has_stop += u64::from(stop_tags.contains(tag));
            p.things_any += u64::from(ctx.index.things_in.contains_key(&sec));
            let hops = common::hop_distances(ctx.scene, sec);
            for (i, l) in ctx.map.linedefs.iter().enumerate() {
                if l.args[0] != *tag || !PERPETUAL.contains(&l.special) {
                    continue;
                }
                let sides = common::trigger_sides(ctx.map, ctx.scene, sec, i, ctx.step, false);
                p.hops.add(hop_bucket(
                    sides.iter().filter_map(|s| hops.get(s).copied()).min(),
                ));
            }
        }
    }

    let step = tables.step_height();
    for plat in ctx.index.plat_sectors(ctx.map) {
        let Some(facts) = common::analyze_plat(ctx.map, ctx.scene, &ctx.index, plat, step) else {
            continue;
        };
        let repeatable: Vec<bool> = facts
            .triggers
            .iter()
            .map(|t| REPEATABLE_LIFT.contains(&t.special))
            .collect();
        if repeatable.iter().all(|r| !r) {
            *agg.one_shot_lift_shapes.entry(facts.shape).or_default() += 1;
        } else if repeatable.iter().any(|r| !r) {
            *agg.mixed_lift_shapes.entry(facts.shape).or_default() += 1;
        }
    }
}

/// The activator class as the floor report names it. `Activator::Plat` is
/// the lift passes' word for "the target's own floor", which is what a
/// trigger on the target's own edge fires from.
fn activator_label(a: Activator) -> &'static str {
    match a {
        Activator::Low => "Low",
        Activator::Level => "Level",
        Activator::Plat => "OnTarget",
        Activator::Above => "Above",
        Activator::None => "none",
    }
}

fn bucket_count(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        4..=5 => "4-5",
        _ => "6+",
    }
}

fn hop_bucket(h: Option<usize>) -> &'static str {
    match h {
        // A trigger on the target's own edge fires from the target itself.
        Some(0) => "0",
        Some(1) => "1",
        Some(2) => "2",
        Some(3) => "3",
        Some(4..=5) => "4-5",
        Some(_) => "6+",
        None => "unreachable",
    }
}

fn min_side_bucket(lo: i32) -> &'static str {
    match lo {
        ..64 => "<64",
        64 => "=64",
        65..=128 => "65..128",
        _ => ">128",
    }
}

/// The sidedef on the *neighbor's* side of a boundary the target holds.
/// `fronts_this` says which mirror this is (`src/check/scene.rs:33-67`), so
/// the other side is the one the engine draws for the neighbor.
fn neighbor_side<'a>(map: &'a UdmfMap, b: &Boundary) -> Option<&'a UdmfSidedef> {
    let l = &map.linedefs[b.linedef];
    if b.fronts_this {
        l.sideback.and_then(|s| common::sidedef(map, s))
    } else {
        common::sidedef(map, l.sidefront)
    }
}

fn record_target(ctx: &MapCtx<'_>, t: &TargetFacts, agg: &mut Agg) {
    agg.targets += 1;
    agg.conflict_lift += u64::from(t.conflict_lift);
    agg.conflict_emittable += u64::from(t.conflict_emittable);
    agg.conflict_second_family += u64::from(t.conflict_second_family);
    agg.conflict_two_way += u64::from(t.conflict_two_way);
    agg.conflict_other += u64::from(t.conflict_other);
    agg.chain_nb_floor += u64::from(t.nb_floor_target);
    agg.chain_nb_lift += u64::from(t.nb_lift_plat);
    agg.chain_nb_door += u64::from(t.nb_door);
    agg.chain_two_families += u64::from(t.conflict_second_family);
    agg.chain_dest_movable += u64::from(t.dest_neighbor_movable);
    agg.triggers_per_target.add(match t.triggers.len() {
        1 => "1",
        2 => "2",
        _ => "3+",
    });

    for tr in &t.triggers {
        agg.trigger_special.add(tr.special.to_string());
        agg.trigger_kind.add(tr.kind.letter());
        agg.trigger_repeatable.add(if tr.repeatable {
            "repeatable"
        } else {
            "one-shot"
        });
        agg.trigger_placement.add(tr.placement.target_label());
        agg.trigger_hops.add(hop_bucket(tr.hops));
        for &a in &tr.activators {
            agg.trigger_activator.add(activator_label(a));
        }
        if tr.kind == TriggerKind::Switch {
            agg.switch_lines += 1;
            agg.switch_with_texture += u64::from(!tr.switch_slots.is_empty());
        }
        if tr.kind == TriggerKind::Walk {
            agg.walk_lines += 1;
            agg.walk_trip += u64::from(tr.same_sector);
        }
    }

    for a in &t.actions {
        agg.actions += 1;
        agg.quirk_no_neighbor += u64::from(a.no_neighbor);
        agg.quirk_next_highest_noop += u64::from(a.next_highest_noop);
        agg.quirk_next_highest_capped += u64::from(a.next_highest_capped);
        let effect = agg.family_effect.entry(a.ty).or_default();
        match &a.facts {
            None => {
                agg.quirk_needs_texture += 1;
                effect.add("unresolved");
            }
            Some(f) => {
                effect.add(format!("{:?}", f.effect));
                if f.effect != Effect::Dead {
                    agg.family_travel.entry(a.ty).or_default().push(a.travel);
                }
                let up = matches!(a.destination, Destination::Height(d) if d > t.rest);
                let down = matches!(a.destination, Destination::Height(d) if d < t.rest);
                // The thinker's own direction and the direction the floor
                // actually travels can disagree: `P_FindHighestFloorSurrounding`
                // can sit above a `lowerFloor` target, and `raiseFloorCrush`'s
                // `- 8` can sit below a `raiseFloor` one. §D's sub-shapes use
                // the geometry, so this table does too, and the disagreement is
                // counted rather than averaged away.
                agg.quirk_direction_disagrees += u64::from((up || down) && a.ty.raises() != up);
                agg.effect_table.add(format!(
                    "{:?}/{}/{}/{:?}",
                    f.effect,
                    if up {
                        "up"
                    } else if down {
                        "down"
                    } else {
                        "level"
                    },
                    if f.enterable_before {
                        "enterable"
                    } else {
                        "sealed"
                    },
                    f.rider
                ));
            }
        }
    }

    if let Some(shape) = t.opening() {
        record_opening(ctx, t, shape, agg);
    }
}

fn record_opening(ctx: &MapCtx<'_>, t: &TargetFacts, shape: OpeningShape, agg: &mut Agg) {
    let action = &t.actions[0];
    let facts = action.facts.as_ref().expect("an opening has a destination");
    let dest = match action.destination {
        Destination::Height(d) => d,
        Destination::NeedsTexture => return,
    };
    let o = agg.openings.entry(shape).or_default();
    o.n += 1;
    o.neighbor_count.add(bucket_count(t.neighbors.len()));
    o.solid_at_rest += u64::from(t.ceiling - t.rest < ctx.player_height);
    o.flush_at_rest += u64::from(t.ceiling == t.rest);
    if let Some((w, h)) = t.bbox {
        o.geometry_n += 1;
        o.dims.add(format!("{}x{}", w.min(h), w.max(h)));
        o.min_side.add(min_side_bucket(w.min(h)));
    }
    o.travel.push(action.travel);
    o.joins_neighbor_floor += u64::from(facts.joins_neighbor_floor);
    o.new_pairs.add(match facts.new_pairs {
        0 => "0".to_owned(),
        1 => "1".to_owned(),
        2 => "2".to_owned(),
        n @ 3..=5 => n.to_string(),
        _ => "6+".to_owned(),
    });
    o.bidirectional += u64::from(facts.new_pair_bidirectional);
    let flat = &ctx.map.sectors[t.sector].texturefloor;
    let light = ctx.scene.sectors[t.sector].light;
    o.flat_eq_nb += u64::from(
        t.neighbors
            .iter()
            .any(|&n| ctx.map.sectors[n].texturefloor == *flat),
    );
    o.light_eq_nb += u64::from(
        t.neighbors
            .iter()
            .any(|&n| ctx.scene.sectors[n].light == light),
    );
    o.ceiling_eq_all += u64::from(
        !t.neighbors.is_empty()
            && t.neighbors
                .iter()
                .all(|&n| ctx.scene.sectors[n].ceiling == t.ceiling),
    );
    o.things_any += u64::from(!t.things.is_empty());
    for name in &t.things {
        o.thing_names.add(name.clone());
    }
    // A closet: a neighbor whose only two-sided boundaries are with the
    // target, so dropping the wall is the only way in or out of it.
    if shape == OpeningShape::DropWall {
        for &n in &t.neighbors {
            if neighbors_of(ctx.scene, n) == BTreeSet::from([t.sector]) {
                o.closets += 1;
                for thing in ctx.index.things_in.get(&n).map_or(&[][..], Vec::as_slice) {
                    o.closet_things.add(
                        thing
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("type {}", thing.type_id)),
                    );
                }
            }
        }
    }
    let kinds: BTreeSet<&'static str> = t.triggers.iter().map(|tr| tr.kind.letter()).collect();
    o.kind_combo
        .add(kinds.into_iter().collect::<Vec<_>>().join("+"));
    let repeatable = t.triggers.iter().filter(|tr| tr.repeatable).count();
    o.repeat_split.add(if repeatable == t.triggers.len() {
        "repeatable only"
    } else if repeatable == 0 {
        "one-shot only"
    } else {
        "mixed"
    });

    // §F — the faces the engine draws.
    match shape {
        OpeningShape::DropWall | OpeningShape::LedgeLower => {
            record_wall_rendering(ctx, t, agg);
        }
        OpeningShape::Bridge => record_bridge_rendering(ctx, t, action, dest, agg),
        OpeningShape::OtherOpening => {}
    }
}

fn record_wall_rendering(ctx: &MapCtx<'_>, t: &TargetFacts, agg: &mut Agg) {
    let r = &mut agg.render_wall;
    for b in &ctx.scene.sectors[t.sector].boundary {
        let Some(n) = b.neighbor.filter(|&n| n != t.sector) else {
            continue;
        };
        let Some(side) = neighbor_side(ctx.map, b) else {
            continue;
        };
        r.lower_n += 1;
        r.lower_tex.add(side.texturebottom.clone());
        r.lower_unpegged += u64::from(b.lower_unpegged);
        if t.ceiling < ctx.scene.sectors[n].ceiling {
            r.upper_n += 1;
            r.upper_tex.add(side.texturetop.clone());
            r.upper_unpegged += u64::from(b.upper_unpegged);
        }
        r.middle_n += 1;
        let own = &ctx.map.sidedefs[b.sidedef].texturemiddle;
        r.middle_nonblank += u64::from(own != "-" || side.texturemiddle != "-");
    }
}

fn record_bridge_rendering(
    ctx: &MapCtx<'_>,
    t: &TargetFacts,
    action: &Action,
    dest: i32,
    agg: &mut Agg,
) {
    let r = &mut agg.render_bridge;
    let mut walkway_flat: Option<&String> = None;
    for b in &ctx.scene.sectors[t.sector].boundary {
        let Some(n) = b.neighbor.filter(|&n| n != t.sector) else {
            continue;
        };
        if ctx.scene.sectors[n].floor != dest {
            continue;
        }
        walkway_flat.get_or_insert(&ctx.map.sectors[n].texturefloor);
        let Some(side) = neighbor_side(ctx.map, b) else {
            continue;
        };
        r.lower_n += 1;
        r.lower_tex.add(side.texturebottom.clone());
        r.lower_unpegged += u64::from(b.lower_unpegged);
    }
    let Some(walkway) = walkway_flat else { return };
    agg.bridge_flat_n += 1;
    agg.bridge_flat_eq_walkway += u64::from(ctx.map.sectors[t.sector].texturefloor == *walkway);
    // The "and change" families overwrite the target's flat as they start;
    // whether the copied flat matches the walkway is what decides if the
    // bridge reads as part of the floor it joins.
    let copied = match action.ty.flat_source() {
        Some(FlatSource::LineFrontSector) => action
            .triggers
            .iter()
            .filter_map(|&i| t.triggers[i].front_sector)
            .map(|s| &ctx.map.sectors[s].texturefloor)
            .next(),
        Some(FlatSource::DestinationNeighbor) => action
            .dest_neighbors
            .iter()
            .map(|&n| &ctx.map.sectors[n].texturefloor)
            .next(),
        None => None,
    };
    if let Some(copied) = copied {
        agg.bridge_change_flat_n += 1;
        agg.bridge_change_flat_eq_walkway += u64::from(copied == walkway);
    }
}

fn record_arbiter(
    targets: &[TargetFacts],
    verdict: &MapVerdict,
    bad_tag: bool,
    has_gun: bool,
    has_floor_line: bool,
    agg: &mut Agg,
) {
    // The shape half of the gate, kept separate from the line axis so the
    // refusal histogram describes only what §H's reason list can name: a map
    // whose *other* unknown specials sink it is not "refused by the gate".
    let shape_ok = !bad_tag && !has_gun && targets.iter().all(TargetFacts::v1_candidate);
    let gate = verdict.line_ok && shape_ok;
    let no_remote = gate
        && targets.iter().all(|t| {
            t.triggers
                .iter()
                .all(|tr| tr.placement != Placement::Remote)
        });
    let no_chain = gate
        && targets.iter().all(|t| {
            !t.nb_floor_target && !t.nb_lift_plat && !t.nb_door && !t.dest_neighbor_movable
        });
    agg.naive_line += u64::from(verdict.line_ok);
    agg.gated_line += u64::from(gate);
    agg.no_remote_line += u64::from(no_remote);
    agg.no_chain_line += u64::from(no_chain);
    agg.naive_all += u64::from(verdict.line_ok && verdict.others_ok);
    agg.gated_all += u64::from(verdict.others_ok && gate);
    agg.no_remote_all += u64::from(verdict.others_ok && no_remote);
    agg.no_chain_all += u64::from(verdict.others_ok && no_chain);
    if has_floor_line && !shape_ok {
        agg.maps_refused += 1;
        agg.refusal.add(refusal_reason(targets, bad_tag, has_gun));
    }
}

/// The first applicable refusal reason, in the order §H fixes.
fn refusal_reason(targets: &[TargetFacts], bad_tag: bool, has_gun: bool) -> &'static str {
    if bad_tag {
        return "dangling/tag-0";
    }
    if has_gun {
        return "gun";
    }
    for t in targets {
        if t.shared_tag_n != 1 {
            return "shared tag";
        }
    }
    for t in targets {
        if t.conflicted() {
            return "conflict";
        }
    }
    for t in targets {
        if t.actions.len() > 1 {
            return ">=2 families";
        }
    }
    for t in targets {
        match t.actions[0].facts.as_ref() {
            None => return "unresolved destination",
            Some(f) if f.effect == Effect::Dead => return "dead",
            Some(f) if f.effect != Effect::Opening => return "closing/mixed/neutral",
            Some(f) if f.rider == Rider::Loses => return "rider loses",
            Some(_) => {}
        }
    }
    "no activator"
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

fn report(label: &str, agg: &Agg) {
    println!("# liftprobe floors — {label}\n\nMaps: {}\n", agg.maps);
    report_usage(agg);
    report_tags(agg);
    report_effects(agg);
    report_openings(agg);
    report_triggers(agg);
    report_rendering(agg);
    report_chains(agg);
    report_arbiter(agg);
    report_carryover(agg);
    report_limits();
}

fn report_usage(agg: &Agg) {
    println!("## A. Special usage\n");
    println!(
        "- maps with ≥1 floor line: {} of {} ({})",
        agg.maps_with_floor_line,
        agg.maps,
        pct(agg.maps_with_floor_line, agg.maps)
    );
    for (ty, _) in common::FLOOR_FAMILIES {
        let lines = agg.family_lines.get(&ty).copied().unwrap_or(0);
        let maps = agg.family_maps.get(&ty).copied().unwrap_or(0);
        println!(
            "- {}: {lines} lines · {maps} maps ({})",
            ty.label(),
            pct(maps, agg.maps)
        );
    }
    println!("- trigger kind, lines: {}", agg.kind_lines.all());
    println!("- trigger kind, maps: {}", agg.kind_maps.all());
    println!(
        "- special 40 (`RaiseCeilingLowerFloor`, also raises the ceiling): {} lines",
        agg.also_raises_ceiling
    );
    println!(
        "- adjacent families, lines: {} · maps: {}",
        agg.adjacent_lines.all(),
        agg.adjacent_maps.all()
    );
}

fn report_tags(agg: &Agg) {
    println!("\n## B. Tags\n");
    println!(
        "- floor lines tagged 0: {} · naming no sector (dangling): {}",
        agg.tag0_lines, agg.dangling_lines
    );
    println!(
        "- targets (unique tagged sectors, over {} maps): {}",
        agg.maps, agg.targets
    );
    println!(
        "- shared-tag groups (a floor tag naming ≥2 sectors): {} · size: {}",
        agg.groups_n,
        agg.groups_size.all()
    );
    println!(
        "- one floor and mutually connected: {} ({}) · one floor, disconnected: {} ({}) · several floors: {} ({})",
        agg.groups_one_floor_connected,
        pct(agg.groups_one_floor_connected, agg.groups_n),
        agg.groups_one_floor_disconnected,
        pct(agg.groups_one_floor_disconnected, agg.groups_n),
        agg.groups_several_floors,
        pct(agg.groups_several_floors, agg.groups_n)
    );
    for (what, n, maps) in [
        ("a lift special", agg.conflict_lift, agg.maps_conflict_lift),
        (
            "another emittable special (door/exit/teleport)",
            agg.conflict_emittable,
            agg.maps_conflict_emittable,
        ),
        (
            "a second floor family",
            agg.conflict_second_family,
            agg.maps_conflict_second_family,
        ),
        (
            "a raise family and a lower family (two-way elevator)",
            agg.conflict_two_way,
            agg.maps_conflict_two_way,
        ),
        (
            "any other special",
            agg.conflict_other,
            agg.maps_conflict_other,
        ),
    ] {
        println!(
            "- targets whose tag is also named by {what}: {n} ({} of {} targets) · maps with ≥1: {maps} ({})",
            pct(n, agg.targets),
            agg.targets,
            pct(maps, agg.maps)
        );
    }
}

fn report_effects(agg: &Agg) {
    println!("\n## C. Destination and effect\n");
    println!("- actions (family × target): {}", agg.actions);
    for (ty, effects) in &agg.family_effect {
        println!(
            "- {}: {} · travel over moving targets: {}",
            ty.label(),
            effects.all(),
            percentiles(agg.family_travel.get(ty).cloned().unwrap_or_default())
        );
    }
    println!(
        "- quirks: `lowerFloor`/`turboLower` with no two-sided neighbor (destination −500): {} · next-highest no-op: {} · next-highest hit the 20-neighbor cap: {} · raise-to-texture unresolved: {}",
        agg.quirk_no_neighbor,
        agg.quirk_next_highest_noop,
        agg.quirk_next_highest_capped,
        agg.quirk_needs_texture
    );
    println!(
        "- actions whose family direction and travel direction disagree (a `lower*` sent up, or a `raise*` sent down): {}",
        agg.quirk_direction_disagrees
    );
    println!(
        "- effect × direction × enterable-before × rider (of {} classified actions): {}",
        agg.actions - agg.quirk_needs_texture,
        agg.effect_table.all()
    );
}

fn report_openings(agg: &Agg) {
    println!("\n## D. Opening shapes\n");
    let empty = OpeningAgg::default();
    for shape in [
        OpeningShape::DropWall,
        OpeningShape::LedgeLower,
        OpeningShape::Bridge,
        OpeningShape::OtherOpening,
    ] {
        let o = agg.openings.get(&shape).unwrap_or(&empty);
        println!("\n### {shape:?} — {} targets\n", o.n);
        if o.n == 0 {
            println!("- none");
            continue;
        }
        println!("- neighbors: {}", o.neighbor_count.all());
        println!(
            "- solid at rest (ceiling − floor < H): {} ({}) · floor == ceiling exactly: {} ({})",
            o.solid_at_rest,
            pct(o.solid_at_rest, o.n),
            o.flush_at_rest,
            pct(o.flush_at_rest, o.n)
        );
        println!(
            "- with geometry: {} · bbox dims top 10: {} · min side: {}",
            o.geometry_n,
            o.dims.top(10),
            o.min_side.all()
        );
        println!("- travel: {}", percentiles(o.travel.clone()));
        println!(
            "- destination equals a neighbor's floor: {} ({}) · new (A, B) pairs: {} · some new pair bidirectional: {} ({})",
            o.joins_neighbor_floor,
            pct(o.joins_neighbor_floor, o.n),
            o.new_pairs.all(),
            o.bidirectional,
            pct(o.bidirectional, o.n)
        );
        println!(
            "- flat == some neighbor's: {} ({}) · light == some neighbor's: {} ({}) · ceiling == every neighbor's: {} ({})",
            o.flat_eq_nb,
            pct(o.flat_eq_nb, o.n),
            o.light_eq_nb,
            pct(o.light_eq_nb, o.n),
            o.ceiling_eq_all,
            pct(o.ceiling_eq_all, o.n)
        );
        println!(
            "- holding ≥1 thing: {} ({}) · thing names top 12: {}",
            o.things_any,
            pct(o.things_any, o.n),
            o.thing_names.top(12)
        );
        if shape == OpeningShape::DropWall {
            println!(
                "- pocket neighbors (their only two-sided edges are with the target): {} · things in them top 12: {}",
                o.closets,
                o.closet_things.top(12)
            );
        }
        println!(
            "- trigger kinds: {} · repeatability: {}",
            o.kind_combo.all(),
            o.repeat_split.all()
        );
    }
}

fn report_triggers(agg: &Agg) {
    println!("\n## E. Triggers\n");
    println!("- kind: {}", agg.trigger_kind.all());
    println!("- one-shot vs repeatable: {}", agg.trigger_repeatable.all());
    println!("- placement: {}", agg.trigger_placement.all());
    println!(
        "- hops from the nearest activator sector to the target: {}",
        agg.trigger_hops.all()
    );
    println!(
        "- activator floor class relative to the target's rest floor: {}",
        agg.trigger_activator.all()
    );
    println!(
        "- S lines: {} · with an SW1/SW2 texture in a front slot: {} ({})",
        agg.switch_lines,
        agg.switch_with_texture,
        pct(agg.switch_with_texture, agg.switch_lines)
    );
    println!(
        "- W lines: {} · whose two sides name the same sector (a trip line): {} ({})",
        agg.walk_lines,
        agg.walk_trip,
        pct(agg.walk_trip, agg.walk_lines)
    );
    println!("- triggers per target: {}", agg.triggers_per_target.all());
}

fn report_rendering(agg: &Agg) {
    println!("\n## F. Rendering\n");
    let w = &agg.render_wall;
    println!(
        "- DropWall/LedgeLower boundaries: {} · neighbor-side lower top 8: {} · lower_unpegged: {} ({})",
        w.lower_n,
        w.lower_tex.top(8),
        w.lower_unpegged,
        pct(w.lower_unpegged, w.lower_n)
    );
    println!(
        "- of those, boundaries where the target's ceiling is below the neighbor's: {} · upper top 8: {} · upper_unpegged: {} ({})",
        w.upper_n,
        w.upper_tex.top(8),
        w.upper_unpegged,
        pct(w.upper_unpegged, w.upper_n)
    );
    println!(
        "- a middle texture on either side: {} of {} ({})",
        w.middle_nonblank,
        w.middle_n,
        pct(w.middle_nonblank, w.middle_n)
    );
    let b = &agg.render_bridge;
    println!(
        "- Bridge walkway boundaries (neighbor's floor == destination): {} · neighbor-side lower top 8: {} · lower_unpegged: {} ({})",
        b.lower_n,
        b.lower_tex.top(8),
        b.lower_unpegged,
        pct(b.lower_unpegged, b.lower_n)
    );
    println!(
        "- Bridges with a walkway: {} · target's rest flat == the walkway's: {} ({})",
        agg.bridge_flat_n,
        agg.bridge_flat_eq_walkway,
        pct(agg.bridge_flat_eq_walkway, agg.bridge_flat_n)
    );
    println!(
        "- of the \"and change\" families: {} · the copied flat == the walkway's: {} ({})",
        agg.bridge_change_flat_n,
        agg.bridge_change_flat_eq_walkway,
        pct(agg.bridge_change_flat_eq_walkway, agg.bridge_change_flat_n)
    );
}

fn report_chains(agg: &Agg) {
    println!("\n## G. Chains\n");
    for (what, n) in [
        (
            "a neighbor that is itself a floor target",
            agg.chain_nb_floor,
        ),
        ("a neighbor that is a lift plat", agg.chain_nb_lift),
        (
            "a neighbor tagged by an emittable door special",
            agg.chain_nb_door,
        ),
        ("lines of ≥2 families", agg.chain_two_families),
        (
            "a movable destination-defining neighbor",
            agg.chain_dest_movable,
        ),
    ] {
        println!(
            "- targets with {what}: {n} ({} of {} targets)",
            pct(n, agg.targets),
            agg.targets
        );
    }
}

fn report_arbiter(agg: &Agg) {
    println!("\n## H. Arbiter\n");
    println!(
        "- baseline, all axes today (teleports + lifts, no floors): {} ({})",
        agg.baseline_all_axes,
        pct(agg.baseline_all_axes, agg.maps)
    );
    for (what, line, all) in [
        (
            "naïve (every non-gun floor special)",
            agg.naive_line,
            agg.naive_all,
        ),
        ("shape-gated", agg.gated_line, agg.gated_all),
        (
            "gated + remote refused",
            agg.no_remote_line,
            agg.no_remote_all,
        ),
        (
            "gated + chains refused",
            agg.no_chain_line,
            agg.no_chain_all,
        ),
    ] {
        println!(
            "- {what}: line axis {line} ({}) · all axes {all} ({})",
            pct(line, agg.maps),
            pct(all, agg.maps)
        );
    }
    println!(
        "- maps with ≥1 floor line: {} · refused by the gate: {} ({} of those)",
        agg.maps_with_floor_line,
        agg.maps_refused,
        pct(agg.maps_refused, agg.maps_with_floor_line)
    );
    println!("- refusal reason (first applicable): {}", agg.refusal.all());
}

fn report_carryover(agg: &Agg) {
    println!("\n## I. Sub-project 4b\n");
    let p = &agg.perpetual;
    println!("- perpetual plats (tags of 53/87): {}", p.n);
    println!("- travel (high − low): {}", percentiles(p.travel.clone()));
    println!("- rest: {}", p.rest.all());
    println!("- neighbors: {}", p.neighbor_count.all());
    println!(
        "- with a 54/89 stop line on the same tag: {} ({}) · holding ≥1 thing: {} ({})",
        p.has_stop,
        pct(p.has_stop, p.n),
        p.things_any,
        pct(p.things_any, p.n)
    );
    println!("- trigger hops to the plat: {}", p.hops.all());
    let shapes = |m: &BTreeMap<Shape, u64>| {
        if m.is_empty() {
            return "none".to_owned();
        }
        m.iter()
            .map(|(s, n)| format!("{s:?}: {n}"))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    println!(
        "- lift plats whose triggers are all one-shot: {}",
        shapes(&agg.one_shot_lift_shapes)
    );
    println!(
        "- lift plats mixing one-shot and repeatable triggers: {}",
        shapes(&agg.mixed_lift_shapes)
    );
}

fn report_limits() {
    println!("\n## J. Not measured\n");
    println!(
        "- **Load-time heights.** Every destination is the one the engine would compute at the heights the map loads with. A chain — a floor that moves after another already has — is therefore measured in its unfired state (§G counts how often that can happen)."
    );
    println!(
        "- **`ML_BLOCKING`, monsters, the player's radius and use-reach.** `pass` is a pure height test: a two-sided fence the player cannot walk through, a blocking thing, a gap narrower than 32 units and a switch out of arm's reach all read as passable here."
    );
    println!(
        "- **UDMF-origin maps.** A map authored in UDMF is read with Doom special numbers, which its namespace need not honor."
    );
    println!(
        "- **`raiseToTexture`.** Its destination needs the bottom textures' patch heights, which the probe does not load; those actions are counted and left unclassified."
    );
    println!(
        "- **Ceiling specials.** Counted as an adjacent family in §A and otherwise ignored; a floor action under a moving ceiling is measured against the ceiling's load-time height."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    use crate::common::is_lift;
    use crate::common::tests::{Fixture, chain, chain_full, fixture, tee};

    /// The floor facts of the target at `sector`, for a fixture with one
    /// tagged floor sector.
    fn target(f: &Fixture, sector: usize) -> TargetFacts {
        let tables = Tables::load().expect("tables");
        let index = common::MapIndex::build(&f.map, &f.scene);
        let all_floor = floor_all();
        let mut floor_targets = BTreeSet::new();
        for l in &f.map.linedefs {
            if l.args[0] != 0
                && all_floor.contains(&l.special)
                && let Some(v) = index.by_tag.get(&l.args[0])
            {
                floor_targets.extend(v.iter().copied());
            }
        }
        let lift_specials: BTreeSet<i32> =
            tables.lift_specials().into_iter().map(i32::from).collect();
        let other_emittable: BTreeSet<i32> = tables
            .emittable_line_specials()
            .into_iter()
            .map(i32::from)
            .filter(|s| !lift_specials.contains(s) && !all_floor.contains(s))
            .collect();
        let ctx = MapCtx {
            map: &f.map,
            scene: &f.scene,
            step: f.step,
            player_height: tables.player().height,
            floor_targets,
            lift_plats: BTreeSet::new(),
            door_sectors: BTreeSet::new(),
            lift_specials,
            other_emittable,
            index,
        };
        analyze_target(&ctx, sector).expect("the target is analyzed")
    }

    /// `(destination, effect, rider, opening sub-shape)` of a single-family
    /// target — the four facts every worked example asserts.
    fn verdict(t: &TargetFacts) -> (Destination, Effect, Rider, Option<OpeningShape>) {
        assert_eq!(t.actions.len(), 1, "the fixture drives one family");
        let a = &t.actions[0];
        let f = a.facts.as_ref().expect("a resolved destination");
        (a.destination, f.effect, f.rider, f.opening)
    }

    /// Appends a one-sided line carrying `special` and `tag` to a `rooms`-long
    /// `chain`, on the last room's east wall and fronted by that room — the
    /// "switch on B's far wall" the worked examples put there. UDMF indices
    /// follow declaration order per type, so appending both records gives
    /// them the next linedef and sidedef index.
    fn far_wall(text: &mut String, rooms: usize, special: i32, tag: i32) {
        let sd = text.matches("sidedef {").count();
        let (v1, v2) = (2 * rooms + 1, 2 * rooms);
        let last = rooms - 1;
        let _ = writeln!(
            text,
            "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sd}; blocking = true; special = {special}; arg0 = {tag}; }}\n\
             sidedef {{ sector = {last}; texturemiddle = \"STARTAN2\"; }}"
        );
    }

    #[test]
    fn example_1_a_wall_that_drops_away() {
        // T is a slab with no headroom at all: nothing can stand on it, and
        // `lowerFloorToLowest` sends it to 0, which is where it already is.
        let mut text = chain_full(
            &[0, 0, 0],
            &[256, 0, 256],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let f = fixture(&text);
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(0),
                Effect::Dead,
                Rider::NotApplicable,
                None
            )
        );

        // Raise it into a wall you could stand on but never climb: the same
        // line now drops it flush with both rooms.
        let mut text = chain_full(
            &[0, 128, 0],
            &[256, 256, 256],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let f = fixture(&text);
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(0),
                Effect::Opening,
                Rider::NotApplicable,
                Some(OpeningShape::DropWall)
            )
        );
        let facts = t.actions[0].facts.as_ref().expect("classified");
        assert!(facts.joins_neighbor_floor);
        assert!(!facts.enterable_before);
        assert_eq!(facts.new_pairs, 2, "A gains B and B gains A");
        assert!(facts.new_pair_bidirectional);
    }

    #[test]
    fn example_2_a_pit_that_rises_into_a_bridge() {
        // A(64) – T(0) – B(64): `raiseToNearestAndChange` lifts T to 64.
        let f = fixture(&chain(
            &[64, 0, 64],
            &[0, 7, 0],
            &[(20, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(64),
                Effect::Opening,
                Rider::Keeps,
                Some(OpeningShape::Bridge)
            )
        );
        assert!(
            t.actions[0]
                .facts
                .as_ref()
                .expect("classified")
                .enterable_before
        );
    }

    #[test]
    fn example_2_the_bridge_holds_with_a_pit_beside_it() {
        // The same action with a fourth neighbor P at the pit's own floor: P
        // loses only T itself, which never counts as a destination.
        let f = fixture(&tee(
            &[64, 0, 64, 0],
            &[256, 256, 256, 256],
            &[0, 7, 0, 0],
            &[(20, 7), (0, 0), (0, 0)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.neighbors, BTreeSet::from([0, 2, 3]));
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(64),
                Effect::Opening,
                Rider::Keeps,
                Some(OpeningShape::Bridge)
            )
        );
    }

    #[test]
    fn example_3_a_pillar_that_rises_to_block() {
        // 101 raises to the lowest neighboring ceiling, capped at T's own:
        // 256, which leaves no headroom at all.
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(101, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(256),
                Effect::Closing,
                Rider::Loses,
                None
            )
        );
    }

    #[test]
    fn example_4_a_pit_trap_closes_two_rooms_off_each_other() {
        // A(0) – T(0) – B(0) with a pit P(−128): lowering T to −128 turns the
        // level crossing into a wall both ways.
        let f = fixture(&tee(
            &[0, 0, 0, -128],
            &[256, 256, 256, 256],
            &[0, 7, 0, 0],
            &[(38, 7), (0, 0), (0, 0)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(-128),
                Effect::Closing,
                Rider::Loses,
                None
            )
        );
    }

    #[test]
    fn example_5_a_descender_is_neutral_for_others_and_strands_its_rider() {
        // 102 lowers to the highest neighbor, which is T's own floor: dead.
        let f = fixture(&chain(
            &[128, 128, 0],
            &[0, 7, 0],
            &[(102, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (Destination::Height(128), Effect::Dead, Rider::Keeps, None)
        );

        // 23 lowers to the lowest, and takes the rider away from A.
        let f = fixture(&chain(
            &[128, 128, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (Destination::Height(0), Effect::Neutral, Rider::Loses, None)
        );
        assert!(!t.v1_candidate(), "a descender is not a v1 candidate");
    }

    #[test]
    fn example_6_a_ledge_that_lowers_to_join() {
        // 96 up is a wall: nobody can step onto T, so lowering it is a drop
        // wall rather than a ledge.
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            verdict(&target(&f, 1)),
            (
                Destination::Height(0),
                Effect::Opening,
                Rider::NotApplicable,
                Some(OpeningShape::DropWall)
            )
        );

        // 16 up is a step, and a step was never a wall.
        let f = fixture(&chain(
            &[0, 16, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            verdict(&target(&f, 1)),
            (Destination::Height(0), Effect::Neutral, Rider::Keeps, None)
        );

        // A ledge level with B and 96 above A: lowering it away from B's own
        // level opens nothing and strands the rider. A ledge that lowers away
        // from its own level is a descender, not an opening.
        let f = fixture(&chain(
            &[0, 96, 96],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            verdict(&target(&f, 1)),
            (Destination::Height(0), Effect::Neutral, Rider::Loses, None)
        );
    }

    #[test]
    fn example_7_next_highest_with_no_higher_neighbor_is_a_no_op() {
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(18, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            verdict(&t),
            (Destination::Height(0), Effect::Dead, Rider::Keeps, None)
        );
        assert!(
            t.actions[0].next_highest_noop,
            "counted under the next-highest no-op quirk"
        );
        assert!(!t.actions[0].next_highest_capped);
    }

    #[test]
    fn example_8_a_target_with_no_neighbor_lowers_to_minus_500() {
        // A fourth sector no linedef references, named by a 19 line elsewhere.
        let extra = "sector { texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 9; }\n";
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 0, 0],
            &[(19, 9, false), (0, 0, false)],
            extra,
        ));
        let t = target(&f, 3);
        assert!(t.neighbors.is_empty());
        assert_eq!(
            verdict(&t),
            (
                Destination::Height(common::NO_NEIGHBOR_FLOOR),
                Effect::Neutral,
                Rider::NotApplicable,
                None
            )
        );
        assert!(t.actions[0].no_neighbor);
    }

    #[test]
    fn example_9_turbo_lower_adds_eight_only_when_it_moves() {
        let f = fixture(&chain(
            &[0, 64, 0],
            &[0, 7, 0],
            &[(71, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(target(&f, 1).actions[0].destination, Destination::Height(8));

        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(71, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.actions[0].destination, Destination::Height(0));
        assert_eq!(
            t.actions[0].facts.as_ref().expect("classified").effect,
            Effect::Dead
        );
    }

    #[test]
    fn example_10_a_shared_tag_and_a_lift_conflict() {
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 5, 5],
            &[(23, 5, false), (0, 0, false)],
            "",
        ));
        for sector in [1, 2] {
            let t = target(&f, sector);
            assert_eq!(t.shared_tag_n, 2);
            assert!(!t.conflict_lift);
            assert!(!t.v1_candidate(), "a shared tag is refused");
        }

        // A 62 line naming tag 5 as well: a lift conflict on the same tag.
        let mut text = chain(
            &[0, 128, 128],
            &[0, 5, 5],
            &[(23, 5, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 62, 5);
        let f = fixture(&text);
        let t = target(&f, 1);
        assert!(t.conflict_lift);
        assert!(t.conflicted());
    }

    #[test]
    fn example_11_trigger_placement_and_hops() {
        // A switch on T's own edge with T as the back sector.
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.triggers.len(), 1);
        assert_eq!(t.triggers[0].placement, Placement::OnPlatBack);
        assert_eq!(t.triggers[0].placement.target_label(), "OnTargetBack");
        assert_eq!(t.triggers[0].kind, TriggerKind::Switch);
        assert_eq!(t.triggers[0].activators, vec![Activator::Low]);
        assert_eq!(t.triggers[0].hops, Some(1));

        // A walkover two rooms away: neither side is the target or a
        // neighbor, and the nearest activator sector is two hops off.
        let f = fixture(&chain(
            &[0, 0, 0, 0],
            &[7, 0, 0, 0],
            &[(0, 0, false), (0, 0, false), (38, 7, false)],
            "",
        ));
        let t = target(&f, 0);
        assert_eq!(t.triggers[0].placement, Placement::Remote);
        assert_eq!(t.triggers[0].hops, Some(2));
        assert!(has_activator(&t.triggers[0]));

        // A line whose front sidedef dangles fires from nowhere.
        let mut f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        f.map.linedefs[0].sidefront = 9_999;
        let t = target(&f, 1);
        assert_eq!(t.triggers[0].activators, vec![Activator::None]);
        assert!(!has_activator(&t.triggers[0]));
        assert!(t.triggers[0].hops.is_none());
        assert!(!t.v1_candidate(), "no side can fire it");
    }

    #[test]
    fn example_12_use_lines_fire_from_the_front_and_walkovers_from_either_step() {
        // The same 23 line flipped so T is the front: a use from the low room
        // reaches the back side, which `P_UseSpecialLine` refuses.
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(23, 7, true), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.triggers[0].placement, Placement::OnPlatFront);
        assert_eq!(t.triggers[0].activators, vec![Activator::Plat]);

        // A walkover on the same edge: the drop from T fires it, the 96-unit
        // climb from A does not.
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(38, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.triggers[0].kind, TriggerKind::Walk);
        assert_eq!(t.triggers[0].activators, vec![Activator::Plat]);

        // A 16-unit step is crossable both ways, so both sides fire it.
        let f = fixture(&chain(
            &[0, 16, 0],
            &[0, 7, 0],
            &[(38, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(
            t.triggers[0].activators,
            vec![Activator::Level, Activator::Plat]
        );
    }

    #[test]
    fn the_dispatch_table_matches_the_engines_switches() {
        // Spot checks across all five trigger kinds and both dispatchers.
        for (special, ty, kind, repeatable) in [
            (19, FloorType::LowerFloor, TriggerKind::Walk, false),
            (45, FloorType::LowerFloor, TriggerKind::Switch, true),
            (40, FloorType::LowerFloorToLowest, TriggerKind::Walk, false),
            (24, FloorType::RaiseFloor, TriggerKind::Gun, false),
            (
                47,
                FloorType::PlatRaiseToNearestAndChange,
                TriggerKind::Gun,
                false,
            ),
            (140, FloorType::RaiseFloor512, TriggerKind::Switch, false),
            (
                15,
                FloorType::PlatRaiseAndChange24,
                TriggerKind::Switch,
                false,
            ),
            (
                67,
                FloorType::PlatRaiseAndChange32,
                TriggerKind::Switch,
                true,
            ),
            (96, FloorType::RaiseToTexture, TriggerKind::Walk, true),
        ] {
            assert_eq!(
                floor_type(special),
                Some((ty, kind, repeatable)),
                "{special}"
            );
        }
        // The families are disjoint and the derived set is the whole table.
        let all = floor_all();
        assert_eq!(
            all.len(),
            common::FLOOR_FAMILIES
                .iter()
                .map(|(_, s)| s.len())
                .sum::<usize>(),
            "no special appears in two families"
        );
        assert_eq!(all.len(), 48);
        assert!(FLOOR_GUN.iter().all(|g| all.contains(g)));
        // Neither dispatcher makes a lift line a floor line, or the reverse.
        assert!(!all.iter().any(|&s| is_lift(s)));
        assert!(floor_type(62).is_none() && floor_type(1).is_none());
        assert_eq!(trigger_label(TriggerKind::Walk, true), "WR");
        assert_eq!(trigger_label(TriggerKind::Gun, false), "G1");
    }

    #[test]
    fn the_remaining_destination_families_resolve_as_the_engine_computes_them() {
        // 58 (`raiseFloor24`) and 140 (`raiseFloor512`) are relative to the
        // target's own floor, so neighbors do not enter.
        for (special, expected) in [(58, 88), (140, 576)] {
            let f = fixture(&chain(
                &[0, 64, 0],
                &[0, 7, 0],
                &[(special, 7, false), (0, 0, false)],
                "",
            ));
            assert_eq!(
                target(&f, 1).actions[0].destination,
                Destination::Height(expected),
                "special {special}"
            );
        }
        // 15 is `raiseAndChange` +24 and 14 is +32.
        for (special, expected) in [(15, 88), (14, 96)] {
            let f = fixture(&chain(
                &[0, 64, 0],
                &[0, 7, 0],
                &[(special, 7, false), (0, 0, false)],
                "",
            ));
            assert_eq!(
                target(&f, 1).actions[0].destination,
                Destination::Height(expected),
                "special {special}"
            );
        }
        // 55 (`raiseFloorCrush`) takes `raiseFloor`'s destination minus 8,
        // after the cap at the target's own ceiling.
        let f = fixture(&chain_full(
            &[0, 0, 0],
            &[256, 192, 256],
            &[0, 7, 0],
            &[(55, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            target(&f, 1).actions[0].destination,
            Destination::Height(184)
        );
        // 30 (`raiseToTexture`) needs texture heights the probe does not read.
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(30, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.actions[0].destination, Destination::NeedsTexture);
        assert!(t.actions[0].facts.is_none());
        assert!(!t.v1_candidate(), "an unresolved destination is refused");
    }

    #[test]
    fn the_thinkers_direction_and_the_floors_direction_can_disagree() {
        // `raiseFloorCrush` (55) subtracts 8 *after* capping at the target's
        // own ceiling, so a target standing above the lowest neighboring
        // ceiling is sent down by a family whose thinker runs up.
        let f = fixture(&chain_full(
            &[0, 200, 0],
            &[192, 256, 192],
            &[0, 7, 0],
            &[(55, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.actions[0].destination, Destination::Height(184));
        assert!(t.actions[0].ty.raises(), "the thinker runs upward");

        // And `lowerFloor` (19) goes to the *highest* neighboring floor,
        // which can be above the target: a lower family sent up.
        let f = fixture(&chain(
            &[0, -64, 128],
            &[0, 7, 0],
            &[(19, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert_eq!(t.actions[0].destination, Destination::Height(128));
        assert!(!t.actions[0].ty.raises(), "the thinker runs downward");
        assert_eq!(t.actions[0].travel, 192);
    }

    #[test]
    fn two_families_on_one_tag_are_two_actions_and_no_candidate() {
        // A 23 (lower) and a 18 (raise) both naming tag 7: the two-way
        // elevator §B counts, and two actions rather than one.
        let mut text = chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 18, 7);
        let f = fixture(&text);
        let t = target(&f, 1);
        assert_eq!(t.actions.len(), 2);
        assert!(t.conflict_second_family && t.conflict_two_way);
        assert!(!t.conflicted(), "a second floor family is not a conflict");
        assert!(!t.v1_candidate());
        assert_eq!(refusal_reason(&[t], false, false), ">=2 families");
    }

    #[test]
    fn refusal_reasons_are_reported_in_the_fixed_order() {
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        let t = target(&f, 1);
        assert!(t.v1_candidate(), "the plain drop wall is a candidate");
        assert_eq!(refusal_reason(&[], true, true), "dangling/tag-0");
        assert_eq!(refusal_reason(&[], false, true), "gun");
        // A dead target is refused for being dead, not for anything later.
        let dead = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(refusal_reason(&[target(&dead, 1)], false, false), "dead");
    }

    #[test]
    fn buckets_name_their_ranges() {
        assert_eq!(bucket_count(0), "0");
        assert_eq!(bucket_count(5), "4-5");
        assert_eq!(bucket_count(9), "6+");
        assert_eq!(hop_bucket(None), "unreachable");
        assert_eq!(hop_bucket(Some(0)), "0");
        assert_eq!(hop_bucket(Some(5)), "4-5");
        assert_eq!(hop_bucket(Some(6)), "6+");
        assert_eq!(min_side_bucket(63), "<64");
        assert_eq!(min_side_bucket(64), "=64");
        assert_eq!(min_side_bucket(129), ">128");
    }
}
