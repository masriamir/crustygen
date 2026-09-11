//! Pass 4 — the lift-variant probe: what the two lift forms sub-project 4b
//! adds *are* in the maps we have. The **perpetual plat** (`perpetualRaise`,
//! started by 53/87 and stopped by 54/89) and the **one-shot
//! `downWaitUpStay` lift** (21/10 and the blazing 122/121).
//!
//! It builds on the floor pass's §I carry-over ([`crate::floors`]:
//! [`perpetual_plats`], [`one_shot_split`], [`repeatable_twin_map`]) rather
//! than re-deriving it, and reuses the lift passes' placement and activator
//! vocabulary ([`crate::common`]) so a perpetual start line is classified the
//! way a lift trigger is. Every engine constant it reads is cited at its
//! definition; nothing here is written from memory.

use std::collections::{BTreeMap, BTreeSet};

use crustygen::check::scene::{Scene, SceneThing};
use crustygen::lift::{self, vocabulary::Vocabulary};
use crustygen::tables::Tables;
use crustywad::map::udmf::UdmfMap;

use crate::common::{
    self, Activator, Dispatch, Hist, PERPETUAL, Placement, PlatFacts, REPEATABLE_LIFT, Shape,
    is_lift, pct, percentiles,
};
use crate::floors::{
    MapCtx, PerpetualPlat, PerpetualRest, bucket_count, count_len, hop_bucket, map_ctx,
    neighbor_side, neighbors_of, one_shot_split, perpetual_plats, repeatable_twin_map,
};

/// The perpetual start specials: `p_spec.c:682-686` (53, W1 — the case ends
/// `line->special = 0`) and `:852-855` (87, WR). Both are in
/// `P_CrossSpecialLine`, so a perpetual plat is walkover-started from either
/// side; no use or gun form dispatches `perpetualRaise`.
const START: [i32; 2] = [53, 87];

/// The stop specials: `p_spec.c:688-692` (54, W1) and `:862-865` (89, WR),
/// both calling `EV_StopPlat` (`p_plats.c:273-286`), which puts **every**
/// active plat carrying the line's tag — of any type — into `in_stasis`.
const STOP: [i32; 2] = [54, 89];

/// The one-shot `downWaitUpStay` / `blazeDWUS` forms: 21 (S1,
/// `p_switch.c:389-393`), 122 (S1 blazing, `:479-483`), 10 (W1,
/// `p_spec.c:579-583`), 121 (W1 blazing, `:754-758`). The two S1 forms call
/// `P_ChangeSwitchTexture(line, 0)`, whose `useAgain == 0` arm clears the
/// special and swaps the front texture for good (`p_switch.c:211-212`,
/// `:229`, `:241`, `:253`).
const ONE_SHOT_LIFT: [i32; 4] = [21, 10, 122, 121];

/// The use-activated half of [`ONE_SHOT_LIFT`].
const ONE_SHOT_USE: [i32; 2] = [21, 122];

/// `MAXPLATS` (`p_spec.h:306`): `P_AddActivePlat` `I_Error`s once the table
/// is full (`p_plats.c:288-299`). A perpetual plat is never removed
/// (`T_PlatRaise`, `p_plats.c:89-103`: only the four one-way types are), so
/// it holds a slot for the rest of the level, in stasis or not.
const MAX_PLATS: usize = 30;

/// The §H columns: a label and the specials the line axis admits beyond
/// today's set.
const COLUMNS: [(&str, &[i32]); 5] = [
    ("today", &[]),
    ("+{53,87}", &[53, 87]),
    ("+{53,87,54,89}", &[53, 87, 54, 89]),
    ("+{21,10,122,121}", &[21, 10, 122, 121]),
    ("+all eight", &[53, 87, 54, 89, 21, 10, 122, 121]),
];

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// What stands on a plat, by the crate's thing vocabulary.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum ThingClass {
    /// A named species (`Tables::species`).
    Monster,
    /// Health, armor, ammo, a weapon or a key (`Tables::pickup`,
    /// `ammo_pickup`, `weapon_ammo_grant`, `key_trim`, plus the backpack
    /// and the chainsaw, which grant no ammo and so have no accessor).
    Pickup,
    /// A blocking prop or decoration (`Tables::prop`).
    Prop,
    /// A player or deathmatch start (`[things]` names ending `_start`).
    Start,
    /// Anything else the vocabulary names (powerups, the teleport marker,
    /// gore, the candle) or does not name at all.
    Other,
}

impl ThingClass {
    fn label(self) -> &'static str {
        match self {
            Self::Monster => "monster",
            Self::Pickup => "pickup",
            Self::Prop => "prop",
            Self::Start => "start",
            Self::Other => "other",
        }
    }
}

/// Classifies `thing` by what [`Tables`] knows about its name.
fn thing_class(tables: &Tables, thing: &SceneThing) -> ThingClass {
    let Some(name) = thing.name.as_deref() else {
        return ThingClass::Other;
    };
    if name.ends_with("_start") {
        ThingClass::Start
    } else if tables.species(name).is_some() {
        ThingClass::Monster
    } else if tables.prop(name).is_some() {
        ThingClass::Prop
    } else if tables.pickup(name).is_some()
        || tables.ammo_pickup(name).is_some()
        || tables.weapon_ammo_grant(name).is_some()
        || tables.key_trim(name).is_some()
        || matches!(name, "backpack" | "chainsaw")
    {
        ThingClass::Pickup
    } else {
        ThingClass::Other
    }
}

/// One start or stop line of a perpetual plat, placed the way a lift
/// trigger is.
struct TriggerLine {
    special: i32,
    placement: Placement,
    /// The activator classes, deduplicated; `[Activator::None]` when no side
    /// can cross it at rest.
    activators: Vec<Activator>,
    /// Hops from the nearest activator sector to the plat.
    hops: Option<usize>,
    /// A side of the line is the sector holding the player-1 start.
    borders_p1: bool,
}

/// The `W1`/`WR` label of a walkover special.
fn walk_form(special: i32) -> &'static str {
    if special == 53 || special == 54 {
        "W1"
    } else {
        "WR"
    }
}

/// What a perpetual plat's bounds say about its neighborhood.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum ShapeClass {
    /// `low` is exactly one neighbor's floor and `high` exactly one
    /// (other) neighbor's: the plat bounces between two rooms.
    TwoRoom,
    /// Exactly one neighbor: the plat rises and falls beside one room.
    Island,
    /// Everything else.
    Residual,
}

impl ShapeClass {
    fn label(self) -> &'static str {
        match self {
            Self::TwoRoom => "two-room",
            Self::Island => "island",
            Self::Residual => "residual",
        }
    }
}

/// Which neighbor's floor flat a plat shares.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum FlatShare {
    /// A neighbor at the lowest neighboring floor.
    Lowest,
    /// A neighbor at the highest neighboring floor (and none at the lowest).
    Highest,
    /// Some other neighbor only.
    Other,
    /// No neighbor at all.
    None,
}

impl FlatShare {
    fn label(self) -> &'static str {
        match self {
            Self::Lowest => "lowest neighbor",
            Self::Highest => "highest neighbor",
            Self::Other => "another neighbor",
            Self::None => "none",
        }
    }
}

/// One two-sided boundary of a plat on which the engine draws a lower.
struct Face {
    /// The visible side's lower texture is not `-`.
    lower_present: bool,
    /// `ML_DONTPEGBOTTOM` on the linedef.
    dontpegbottom: bool,
}

/// Everything the pass reads about one perpetual plat.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent measured fact about the plat (which family shares \
              its tag, what it can be boarded from, what its light matches); they encode no \
              joint state"
)]
struct PerpetualFacts {
    plat: PerpetualPlat,
    neighbors: BTreeSet<usize>,
    distinct_nb_floors: usize,
    class: ShapeClass,
    /// How many sectors carry the plat's tag.
    shared_tag_n: usize,
    /// A DWUS/blaze lift line also names the tag.
    tag_has_lift: bool,
    /// The non-lift, non-plat specials naming the tag, sorted and
    /// deduplicated.
    other_specials: Vec<i32>,
    starts: Vec<TriggerLine>,
    stops: Vec<TriggerLine>,
    /// Some neighbor's floor is within a step of the plat's rest floor.
    boardable_at_rest: bool,
    /// Boundaries to a neighbor at a different floor.
    faces: Vec<Face>,
    /// Boundaries to a neighbor at the plat's own floor (no lower drawn).
    level_faces: usize,
    flat_share: FlatShare,
    light_eq_lowest: bool,
    light_eq_highest: bool,
    things: Vec<ThingClass>,
    /// The names of the things classified [`ThingClass::Other`].
    other_thing_names: Vec<String>,
}

impl PerpetualFacts {
    /// The **loose provisional gate** §H prices — a ceiling, not the design:
    /// the tag resolves to exactly one sector, the plat is not dead, no stop
    /// line names the tag, no other family shares it, and it rests at `low`
    /// or at `high`.
    fn provisional(&self) -> bool {
        self.shared_tag_n == 1
            && matches!(self.plat.rest, PerpetualRest::AtLow | PerpetualRest::AtHigh)
            && self.stops.is_empty()
            && !self.tag_has_lift
            && self.other_specials.is_empty()
    }

    /// The `W1 / WR / both` split of the plat's start lines.
    fn start_forms(&self) -> &'static str {
        forms_label(self.starts.iter().map(|s| s.special), 53)
    }

    /// The same split over its stop lines.
    fn stop_forms(&self) -> &'static str {
        forms_label(self.stops.iter().map(|s| s.special), 54)
    }
}

/// `W1 only` / `WR only` / `both` over a set of walkover specials whose W1
/// form is `once`.
fn forms_label(specials: impl Iterator<Item = i32>, once: i32) -> &'static str {
    let (mut w1, mut wr) = (false, false);
    for s in specials {
        if s == once {
            w1 = true;
        } else {
            wr = true;
        }
    }
    match (w1, wr) {
        (true, false) => "W1 only",
        (false, true) => "WR only",
        (true, true) => "both",
        (false, false) => "none",
    }
}

/// The class of a plat with the given clamped bounds and neighbor floors.
fn shape_class(low: i32, high: i32, nb_floors: &[i32]) -> ShapeClass {
    let at = |h: i32| nb_floors.iter().filter(|&&f| f == h).count();
    if low < high && at(low) == 1 && at(high) == 1 {
        ShapeClass::TwoRoom
    } else if nb_floors.len() == 1 {
        ShapeClass::Island
    } else {
        ShapeClass::Residual
    }
}

/// The flat-sharing class of a plat whose flat is `flat`, over its
/// neighbors' `(floor, flat)` pairs.
fn flat_share(flat: &str, neighbors: &[(i32, &str)]) -> FlatShare {
    let Some(lowest) = neighbors.iter().map(|&(f, _)| f).min() else {
        return FlatShare::None;
    };
    let highest = neighbors.iter().map(|&(f, _)| f).max().unwrap_or(lowest);
    let shares = |h: i32| neighbors.iter().any(|&(f, t)| f == h && t == flat);
    if shares(lowest) {
        FlatShare::Lowest
    } else if shares(highest) {
        FlatShare::Highest
    } else if neighbors.iter().any(|&(_, t)| t == flat) {
        FlatShare::Other
    } else {
        FlatShare::None
    }
}

/// The travel bucket the §B histogram uses.
fn travel_bucket(travel: i32) -> &'static str {
    match travel {
        ..=24 => "≤24",
        25..=64 => "25-64",
        65..=128 => "65-128",
        129..=256 => "129-256",
        _ => ">256",
    }
}

/// The per-map state the perpetual and one-shot analyses share.
struct VarCtx<'a> {
    ctx: MapCtx<'a>,
    tables: &'a Tables,
    /// The sector holding the player-1 start (thing type 1), if the map has
    /// one the scene could place.
    p1_sector: Option<usize>,
}

/// The lines carrying `tag` whose special is in `specials`, placed and
/// classified against `sector` as walkovers (`Dispatch::Cross`: 53/87/54/89
/// are all in `P_CrossSpecialLine`).
fn walkover_lines(
    v: &VarCtx<'_>,
    sector: usize,
    tag: i32,
    neighbors: &BTreeSet<usize>,
    hops: &BTreeMap<usize, usize>,
    specials: &[i32],
) -> Vec<TriggerLine> {
    let (map, scene) = (v.ctx.map, v.ctx.scene);
    let floor = scene.sectors[sector].floor;
    let mut out = Vec::new();
    for (i, l) in map.linedefs.iter().enumerate() {
        if l.args[0] != tag || !specials.contains(&l.special) {
            continue;
        }
        let front = common::side_sector(map, l.sidefront);
        let back = l.sideback.and_then(|b| common::side_sector(map, b));
        let sides =
            common::activator_sides(map, scene, sector, floor, i, v.ctx.step, Dispatch::Cross);
        out.push(TriggerLine {
            special: l.special,
            placement: common::placement_of(sector, neighbors, front, back),
            activators: common::activators(&sides),
            hops: sides
                .iter()
                .filter_map(|&(s, _)| hops.get(&s).copied())
                .min(),
            borders_p1: v.p1_sector.is_some() && (front == v.p1_sector || back == v.p1_sector),
        });
    }
    out
}

/// §E — the faces the engine draws on `sector`'s two-sided boundaries, and
/// the count of level boundaries it draws no lower on. The visible lower is
/// on the sidedef whose sector has the lower floor (`r_segs.c`,
/// `R_StoreWallRange`), so it is the neighbor's when the plat stands above
/// it and the plat's own when the plat sits below.
fn faces_of(map: &UdmfMap, scene: &Scene, sector: usize) -> (Vec<Face>, usize) {
    let ss = &scene.sectors[sector];
    let mut faces = Vec::new();
    let mut level_faces = 0;
    for b in &ss.boundary {
        let Some(n) = b.neighbor.filter(|&n| n != sector) else {
            continue;
        };
        let nb_floor = scene.sectors[n].floor;
        if nb_floor == ss.floor {
            level_faces += 1;
            continue;
        }
        let visible = if nb_floor < ss.floor {
            neighbor_side(map, b).map(|s| s.texturebottom.as_str())
        } else {
            Some(map.sidedefs[b.sidedef].texturebottom.as_str())
        };
        let Some(tex) = visible else { continue };
        faces.push(Face {
            lower_present: tex != "-",
            dontpegbottom: b.lower_unpegged,
        });
    }
    (faces, level_faces)
}

/// Analyzes one perpetual plat.
fn analyze_perpetual(v: &VarCtx<'_>, plat: PerpetualPlat) -> PerpetualFacts {
    let (map, scene) = (v.ctx.map, v.ctx.scene);
    let sector = plat.sector;
    let ss = &scene.sectors[sector];
    let neighbors = neighbors_of(scene, sector);
    let nb_floors: Vec<i32> = neighbors.iter().map(|&n| scene.sectors[n].floor).collect();
    let distinct_nb_floors = nb_floors.iter().copied().collect::<BTreeSet<i32>>().len();
    let class = shape_class(plat.low, plat.high, &nb_floors);
    let hops = common::hop_distances(scene, sector);
    let starts = walkover_lines(v, sector, plat.tag, &neighbors, &hops, &START);
    let stops = walkover_lines(v, sector, plat.tag, &neighbors, &hops, &STOP);

    let tag_has_lift = map
        .linedefs
        .iter()
        .any(|l| l.args[0] == plat.tag && is_lift(l.special));
    let mut other_specials: Vec<i32> = v
        .ctx
        .index
        .other_by_tag
        .get(&plat.tag)
        .map(|v| {
            v.iter()
                .copied()
                .filter(|s| !PERPETUAL.contains(s))
                .collect()
        })
        .unwrap_or_default();
    other_specials.sort_unstable();
    other_specials.dedup();

    let (faces, level_faces) = faces_of(map, scene, sector);
    let nb_flats: Vec<(i32, &str)> = neighbors
        .iter()
        .map(|&n| (scene.sectors[n].floor, map.sectors[n].texturefloor.as_str()))
        .collect();
    let lowest = nb_floors.iter().copied().min();
    let highest = nb_floors.iter().copied().max();
    let light_at = |h: Option<i32>| {
        h.is_some_and(|h| {
            neighbors
                .iter()
                .any(|&n| scene.sectors[n].floor == h && scene.sectors[n].light == ss.light)
        })
    };
    let things: &[&SceneThing] = v
        .ctx
        .index
        .things_in
        .get(&sector)
        .map_or(&[][..], Vec::as_slice);
    let classes: Vec<ThingClass> = things.iter().map(|t| thing_class(v.tables, t)).collect();
    let other_thing_names = things
        .iter()
        .zip(&classes)
        .filter(|&(_, &c)| c == ThingClass::Other)
        .map(|(t, _)| {
            t.name
                .clone()
                .unwrap_or_else(|| format!("type {}", t.type_id))
        })
        .collect();

    PerpetualFacts {
        shared_tag_n: v.ctx.index.by_tag.get(&plat.tag).map_or(0, Vec::len),
        tag_has_lift,
        other_specials,
        starts,
        stops,
        boardable_at_rest: nb_floors
            .iter()
            .any(|&f| (f - ss.floor).abs() <= v.ctx.step),
        faces,
        level_faces,
        flat_share: flat_share(&map.sectors[sector].texturefloor, &nb_flats),
        light_eq_lowest: light_at(lowest),
        light_eq_highest: light_at(highest),
        things: classes,
        other_thing_names,
        distinct_nb_floors,
        class,
        neighbors,
        plat,
    }
}

/// The trigger forms among a one-shot plat's one-shot triggers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum OneShotForms {
    S1Only,
    W1Only,
    Both,
}

impl OneShotForms {
    fn label(self) -> &'static str {
        match self {
            Self::S1Only => "S1 only",
            Self::W1Only => "W1 only",
            Self::Both => "S1+W1",
        }
    }
}

/// Everything the pass reads about one one-shot lift plat.
struct OneShotFacts {
    /// The plat also has a repeatable trigger (the §I "mixed" row).
    mixed: bool,
    forms: OneShotForms,
    /// The shape `analyze_plat` gives the plat once every one-shot special
    /// is rewritten to its repeatable twin.
    what_if: Shape,
    things: Vec<ThingClass>,
    /// For a what-if `Core` plat: whether its level room and its low room
    /// stay connected in the hop graph with the plat's sector removed.
    /// `None` when the plat is not `Core` or one of the rooms is absent.
    core_connected: Option<bool>,
    /// For a what-if `Barrier` plat: the neighbors from which some trigger
    /// fires with a `Low` activator.
    barrier_low_nbs: Option<usize>,
    /// For a what-if `Barrier` plat: the distinct faces (neighbor sides) its
    /// on-plat trigger lines sit on.
    barrier_faces: Option<usize>,
}

/// Whether some sector in `from` reaches some sector in `to` over two-sided
/// adjacency, heights ignored, never entering `without`.
fn connected_without(
    scene: &Scene,
    from: &BTreeSet<usize>,
    to: &BTreeSet<usize>,
    without: usize,
) -> bool {
    let mut seen: BTreeSet<usize> = from.iter().copied().filter(|&s| s != without).collect();
    let mut stack: Vec<usize> = seen.iter().copied().collect();
    while let Some(s) = stack.pop() {
        if to.contains(&s) {
            return true;
        }
        for b in &scene.sectors[s].boundary {
            if !b.two_sided {
                continue;
            }
            let Some(n) = b.neighbor else { continue };
            if n != without && seen.insert(n) {
                stack.push(n);
            }
        }
    }
    false
}

/// The distinct neighbor sides of the lift lines on `plat`'s own boundary.
fn trigger_faces(map: &UdmfMap, plat: usize, tag: i32) -> usize {
    let mut faces: BTreeSet<usize> = BTreeSet::new();
    for l in &map.linedefs {
        if l.args[0] != tag || !is_lift(l.special) {
            continue;
        }
        let front = common::side_sector(map, l.sidefront);
        let back = l.sideback.and_then(|b| common::side_sector(map, b));
        let other = if front == Some(plat) {
            back
        } else if back == Some(plat) {
            front
        } else {
            None
        };
        if let Some(o) = other.filter(|&o| o != plat) {
            faces.insert(o);
        }
    }
    faces.len()
}

/// Analyzes one one-shot plat: `real` is its analysis on the map as shipped,
/// `what_if` the same plat on the twin-rewritten map and scene.
fn analyze_one_shot(
    v: &VarCtx<'_>,
    plat: usize,
    mixed: bool,
    real: &PlatFacts,
    what_if: &PlatFacts,
    what_if_map: &UdmfMap,
    what_if_scene: &Scene,
) -> OneShotFacts {
    let (mut s1, mut w1) = (false, false);
    for t in &real.triggers {
        if REPEATABLE_LIFT.contains(&t.special) {
            continue;
        }
        if ONE_SHOT_USE.contains(&t.special) {
            s1 = true;
        } else {
            w1 = true;
        }
    }
    let forms = match (s1, w1) {
        (true, true) => OneShotForms::Both,
        (false, true) => OneShotForms::W1Only,
        // `one_shot_split` only yields a plat with some one-shot trigger.
        _ => OneShotForms::S1Only,
    };
    let things = v
        .ctx
        .index
        .things_in
        .get(&plat)
        .map(|t| t.iter().map(|t| thing_class(v.tables, t)).collect())
        .unwrap_or_default();
    let floor = what_if_scene.sectors[plat].floor;
    let core_connected = (what_if.shape == Shape::Core).then(|| {
        let level: BTreeSet<usize> = what_if
            .neighbors
            .iter()
            .copied()
            .filter(|&n| (what_if_scene.sectors[n].floor - floor).abs() <= v.ctx.step)
            .collect();
        let low: BTreeSet<usize> = what_if
            .neighbors
            .iter()
            .copied()
            .filter(|&n| what_if_scene.sectors[n].floor < floor - v.ctx.step)
            .collect();
        (!level.is_empty() && !low.is_empty())
            .then(|| connected_without(what_if_scene, &level, &low, plat))
    });
    let barrier = what_if.shape == Shape::Barrier;
    OneShotFacts {
        mixed,
        forms,
        what_if: what_if.shape,
        things,
        core_connected: core_connected.flatten(),
        barrier_low_nbs: barrier.then_some(what_if.low_activator_nbs),
        barrier_faces: barrier
            .then(|| trigger_faces(what_if_map, plat, what_if_map.sectors[plat].id)),
    }
}

// ---------------------------------------------------------------------------
// Aggregates
// ---------------------------------------------------------------------------

/// One §H column's three counts.
#[derive(Default)]
struct ArbiterColumn {
    /// Maps whose linedef specials are all in today's set plus the column's.
    line: u64,
    /// That, and every other axis as today — one-shot plats still refused
    /// by the recognizer, perpetual plats not judged at all.
    all_today: u64,
    /// That, with one-shot triggers read as repeatable and every perpetual
    /// plat passing the provisional gate.
    provisional: u64,
}

/// Everything the report prints.
#[derive(Default)]
struct Agg {
    maps: u64,
    // A
    start_lines: u64,
    start_tag0: u64,
    start_dangling: u64,
    start_tag_sectors: Hist,
    perpetual_n: u64,
    maps_with_perpetual: u64,
    shared_tag: Hist,
    tag_has_lift: u64,
    tag_has_other: u64,
    other_specials: Hist,
    stops_per_tag: Hist,
    stop_forms: Hist,
    // B
    rest: Hist,
    neighbor_count: Hist,
    nb_floors: Hist,
    class: Hist,
    rest_x_class: Hist,
    travel: Vec<i32>,
    travel_bucket: Hist,
    // C
    start_forms: Hist,
    starts_per_tag: Hist,
    start_placement: Hist,
    start_activator: Hist,
    start_hops: Hist,
    start_borders_p1_plats: u64,
    start_borders_p1_lines: u64,
    boardable: u64,
    boardable_x_rest: Hist,
    // D
    stop_lines: u64,
    stop_placement: Hist,
    stop_activator: Hist,
    stop_hops: Hist,
    stop_form_lines: Hist,
    plats_with_stop: u64,
    stop_nearest: Hist,
    // E
    faces: u64,
    faces_lower_present: u64,
    faces_unpegged: u64,
    level_faces: u64,
    flat_share: Hist,
    light_lowest: u64,
    light_highest: u64,
    things_any: u64,
    thing_class: Hist,
    other_thing_names: Hist,
    // F
    perpetual_per_map: Hist,
    perpetual_max: u64,
    moving_per_map: Hist,
    moving_max: u64,
    combined_over_15: u64,
    combined_over_30: u64,
    combined_max: u64,
    // G
    one_shot_n: u64,
    mixed_n: u64,
    one_shot_forms: Hist,
    shape_x_form: Hist,
    one_shot_things_any: u64,
    one_shot_thing_class: Hist,
    core_connected: Hist,
    barrier_low_nbs: Hist,
    barrier_faces: Hist,
    s1_lines: u64,
    s1_sw1: u64,
    s1_sw2: u64,
    s1_textures: Hist,
    // H
    line_today: u64,
    all_floors_ignored: u64,
    all_honest: u64,
    columns: Vec<ArbiterColumn>,
    maps_with_bad_start: u64,
    provisional_plats: u64,
}

/// The arbiter facts of one map.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one independent axis of the arbiter's verdict; §H combines them \
              per column and never stores a combined state"
)]
struct MapArbiter {
    /// Out-of-set linedef specials, as the live vocabulary reports them.
    unknown: Vec<i32>,
    /// Sector specials, thing kinds and teleports pass.
    others_ok: bool,
    /// The floor recognizer refuses nothing (the honest sixth axis).
    floors_ok: bool,
    /// The plat recognizer refuses nothing on the map as shipped.
    lifts_today: bool,
    /// The plat recognizer refuses nothing on the twin-rewritten map.
    lifts_twin: bool,
}

/// Runs the six-axis verdict the way `crustygen-corpus` does
/// (`src/lift/corpus.rs`): each recognizer only when its specials are
/// present. The floor recognizer is included, which is what makes the
/// all-axes baseline the **honest** figure rather than the floor pass's
/// floors-ignored one.
fn map_arbiter(
    name: &str,
    map: &UdmfMap,
    scene: &Scene,
    tables: &Tables,
    vocab: &Vocabulary,
) -> MapArbiter {
    let telemetry = lift::survey(name, map);
    let verdict = vocab.classify(&telemetry);
    let has = |s: u16| telemetry.linedef_specials.contains_key(&i32::from(s));
    let teleports_ok = if tables.teleport_specials().into_iter().any(has) {
        lift::teleport::recognize(scene, tables).counts.refusals() == 0
    } else {
        true
    };
    let has_lifts = tables.lift_specials().into_iter().any(has);
    let lifts_today = if has_lifts {
        lift::plat::recognize(scene, tables).counts.refusals() == 0
    } else {
        true
    };
    let has_one_shot = ONE_SHOT_LIFT
        .iter()
        .any(|&s| telemetry.linedef_specials.contains_key(&s));
    let lifts_twin = if has_one_shot {
        let twin = repeatable_twin_map(map);
        let twin_scene = Scene::build(&twin, tables, &mut Vec::new());
        lift::plat::recognize(&twin_scene, tables).counts.refusals() == 0
    } else {
        lifts_today
    };
    let floors_ok = if tables
        .recognized_floor_specials()
        .iter()
        .any(|&(s, ..)| has(s))
    {
        lift::floor::recognize(scene, tables).counts.refusals() == 0
    } else {
        true
    };
    MapArbiter {
        unknown: verdict.unknown_line_specials,
        others_ok: verdict.sector_specials_ok && verdict.thing_kinds_ok && teleports_ok,
        floors_ok,
        lifts_today,
        lifts_twin,
    }
}

/// Runs the variants pass over `dirs` and prints the report for `label`.
pub(crate) fn run(label: &str, dirs: &[String]) {
    let tables = Tables::load().expect("tables");
    let vocab = Vocabulary::from_tables(&tables);
    let mut agg = Agg {
        columns: (0..COLUMNS.len())
            .map(|_| ArbiterColumn::default())
            .collect(),
        ..Agg::default()
    };
    let maps = common::sweep(dirs, |name, map| {
        survey_map(name, map, &tables, &vocab, &mut agg);
    });
    agg.maps = maps;
    report(label, &agg);
}

fn survey_map(name: &str, map: &UdmfMap, tables: &Tables, vocab: &Vocabulary, agg: &mut Agg) {
    let scene = Scene::build(map, tables, &mut Vec::new());
    let arbiter = map_arbiter(name, map, &scene, tables, vocab);
    let ctx = map_ctx(map, &scene, tables);
    let v = VarCtx {
        p1_sector: scene
            .things
            .iter()
            .find(|t| t.type_id == 1)
            .and_then(|t| t.sector),
        ctx,
        tables,
    };

    let bad_start = survey_start_lines(&v, agg);
    let plats: Vec<PerpetualFacts> = perpetual_plats(&v.ctx)
        .into_iter()
        .map(|p| analyze_perpetual(&v, p))
        .collect();
    agg.maps_with_perpetual += u64::from(!plats.is_empty());
    for p in &plats {
        record_perpetual(p, agg);
    }
    survey_concurrency(&v, plats.len(), agg);
    survey_one_shot(&v, agg);
    record_arbiter(&arbiter, &plats, bad_start, agg);
}

/// §A — every 53/87 line's tag resolution. Returns whether some start line
/// is tagged 0 or names no sector.
fn survey_start_lines(v: &VarCtx<'_>, agg: &mut Agg) -> bool {
    let mut bad = false;
    let mut tags: BTreeSet<i32> = BTreeSet::new();
    for l in &v.ctx.map.linedefs {
        if !START.contains(&l.special) {
            continue;
        }
        agg.start_lines += 1;
        if l.args[0] == 0 {
            agg.start_tag0 += 1;
            bad = true;
        } else if let Some(secs) = v.ctx.index.by_tag.get(&l.args[0]) {
            if tags.insert(l.args[0]) {
                agg.start_tag_sectors.add(match secs.len() {
                    1 => "1",
                    2 => "2",
                    _ => "3+",
                });
            }
        } else {
            agg.start_dangling += 1;
            bad = true;
        }
    }
    agg.maps_with_bad_start += u64::from(bad);
    bad
}

fn record_perpetual(p: &PerpetualFacts, agg: &mut Agg) {
    // A
    agg.perpetual_n += 1;
    agg.shared_tag.add(match p.shared_tag_n {
        1 => "1",
        2 => "2",
        _ => "3+",
    });
    agg.tag_has_lift += u64::from(p.tag_has_lift);
    agg.tag_has_other += u64::from(!p.other_specials.is_empty());
    for s in &p.other_specials {
        agg.other_specials.add(s.to_string());
    }
    agg.stops_per_tag.add(match p.stops.len() {
        0 => "0",
        1 => "1",
        2 => "2",
        _ => "3+",
    });
    if !p.stops.is_empty() {
        agg.stop_forms.add(p.stop_forms());
    }
    // B
    let rest = p.plat.rest.label();
    let class = p.class.label();
    agg.rest.add(rest);
    agg.neighbor_count.add(bucket_count(p.neighbors.len()));
    agg.nb_floors.add(match p.distinct_nb_floors {
        0 => "0",
        1 => "1",
        2 => "2",
        _ => "3+",
    });
    agg.class.add(class);
    agg.rest_x_class.add(format!("{rest} × {class}"));
    let travel = p.plat.high - p.plat.low;
    agg.travel.push(travel);
    agg.travel_bucket.add(travel_bucket(travel));
    // C
    agg.start_forms.add(p.start_forms());
    agg.starts_per_tag.add(match p.starts.len() {
        0 => "0",
        1 => "1",
        2 => "2",
        _ => "3+",
    });
    for s in &p.starts {
        agg.start_placement
            .add(format!("{} {:?}", walk_form(s.special), s.placement));
        for &a in &s.activators {
            agg.start_activator.add(format!("{a:?}"));
        }
        agg.start_hops.add(hop_bucket(s.hops));
        agg.start_borders_p1_lines += u64::from(s.borders_p1);
    }
    agg.start_borders_p1_plats += u64::from(p.starts.iter().any(|s| s.borders_p1));
    agg.boardable += u64::from(p.boardable_at_rest);
    agg.boardable_x_rest.add(format!(
        "{rest} × {}",
        if p.boardable_at_rest {
            "boardable"
        } else {
            "not boardable"
        }
    ));
    record_stops(p, agg);
    record_rendering(p, agg);
}

/// §D — a plat's stop lines.
fn record_stops(p: &PerpetualFacts, agg: &mut Agg) {
    for s in &p.stops {
        agg.stop_lines += 1;
        agg.stop_form_lines.add(walk_form(s.special));
        agg.stop_placement.add(format!("{:?}", s.placement));
        for &a in &s.activators {
            agg.stop_activator.add(format!("{a:?}"));
        }
        agg.stop_hops.add(hop_bucket(s.hops));
    }
    if !p.stops.is_empty() {
        agg.plats_with_stop += 1;
        let nearest = if p
            .stops
            .iter()
            .any(|s| matches!(s.placement, Placement::OnPlatFront | Placement::OnPlatBack))
        {
            "on the plat's own boundary"
        } else if p.stops.iter().any(|s| s.placement == Placement::Adjacent) {
            "on a neighbor's far threshold"
        } else {
            "remote only"
        };
        agg.stop_nearest.add(nearest);
    }
}

/// §E — a plat's faces, flat, light and cargo.
fn record_rendering(p: &PerpetualFacts, agg: &mut Agg) {
    for f in &p.faces {
        agg.faces += 1;
        agg.faces_lower_present += u64::from(f.lower_present);
        agg.faces_unpegged += u64::from(f.dontpegbottom);
    }
    agg.level_faces += count_len(p.level_faces);
    agg.flat_share.add(p.flat_share.label());
    agg.light_lowest += u64::from(p.light_eq_lowest);
    agg.light_highest += u64::from(p.light_eq_highest);
    agg.things_any += u64::from(!p.things.is_empty());
    for c in &p.things {
        agg.thing_class.add(c.label());
    }
    for n in &p.other_thing_names {
        agg.other_thing_names.add(n.clone());
    }
}

/// §F — how many plat thinkers a map can have alive at once. Returns the
/// count of moving DWUS/blaze plats.
fn survey_concurrency(v: &VarCtx<'_>, perpetual: usize, agg: &mut Agg) -> usize {
    let moving = v
        .ctx
        .index
        .plat_sectors(v.ctx.map)
        .into_iter()
        .filter(|&s| {
            common::analyze_plat(v.ctx.map, v.ctx.scene, &v.ctx.index, s, v.ctx.step)
                .is_some_and(|f| f.moving())
        })
        .count();
    let bucket = |n: usize| match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3..=5 => "3-5",
        6..=10 => "6-10",
        _ => "11+",
    };
    agg.perpetual_per_map.add(bucket(perpetual));
    agg.perpetual_max = agg.perpetual_max.max(count_len(perpetual));
    agg.moving_per_map.add(bucket(moving));
    agg.moving_max = agg.moving_max.max(count_len(moving));
    // The combined rows are over maps that have a perpetual plat at all:
    // a map of DWUS lifts alone never holds a slot for the whole level.
    if perpetual > 0 {
        let combined = perpetual + moving;
        agg.combined_max = agg.combined_max.max(count_len(combined));
        agg.combined_over_15 += u64::from(combined > MAX_PLATS / 2);
        agg.combined_over_30 += u64::from(combined > MAX_PLATS);
    }
    moving
}

/// §G — the one-shot lift plats, on the map as shipped and on the what-if.
fn survey_one_shot(v: &VarCtx<'_>, agg: &mut Agg) {
    let (map, scene, step) = (v.ctx.map, v.ctx.scene, v.ctx.step);
    for l in &map.linedefs {
        if !ONE_SHOT_USE.contains(&l.special) {
            continue;
        }
        agg.s1_lines += 1;
        let Some(sd) = common::sidedef(map, l.sidefront) else {
            continue;
        };
        let slots = [&sd.texturetop, &sd.texturemiddle, &sd.texturebottom];
        agg.s1_sw1 += u64::from(
            slots
                .iter()
                .any(|t| t.to_ascii_uppercase().starts_with("SW1")),
        );
        agg.s1_sw2 += u64::from(
            slots
                .iter()
                .any(|t| t.to_ascii_uppercase().starts_with("SW2")),
        );
        for t in [&sd.texturemiddle, &sd.texturebottom] {
            if t != "-" {
                agg.s1_textures.add(t.to_ascii_uppercase());
            }
        }
    }

    let (all_one_shot, mixed) = one_shot_split(&v.ctx, step);
    if all_one_shot.is_empty() && mixed.is_empty() {
        return;
    }
    let what_if_map = repeatable_twin_map(map);
    let what_if_scene = Scene::build(&what_if_map, v.tables, &mut Vec::new());
    let what_if_index = common::MapIndex::build(&what_if_map, &what_if_scene);
    for (plat, is_mixed) in all_one_shot
        .iter()
        .map(|&p| (p, false))
        .chain(mixed.iter().map(|&p| (p, true)))
    {
        let (Some(real), Some(what_if)) = (
            common::analyze_plat(map, scene, &v.ctx.index, plat, step),
            common::analyze_plat(&what_if_map, &what_if_scene, &what_if_index, plat, step),
        ) else {
            continue;
        };
        let facts = analyze_one_shot(
            v,
            plat,
            is_mixed,
            &real,
            &what_if,
            &what_if_map,
            &what_if_scene,
        );
        record_one_shot(&facts, agg);
    }
}

fn record_one_shot(f: &OneShotFacts, agg: &mut Agg) {
    let row = if f.mixed { "mixed" } else { "all-one-shot" };
    if f.mixed {
        agg.mixed_n += 1;
    } else {
        agg.one_shot_n += 1;
    }
    agg.one_shot_forms
        .add(format!("{row} / {}", f.forms.label()));
    agg.shape_x_form
        .add(format!("{row} / {:?} / {}", f.what_if, f.forms.label()));
    agg.one_shot_things_any += u64::from(!f.things.is_empty());
    for c in &f.things {
        agg.one_shot_thing_class.add(c.label());
    }
    if f.what_if == Shape::Core {
        agg.core_connected.add(match f.core_connected {
            Some(true) => "still connected without the plat",
            Some(false) => "stranded (plat is the only route)",
            None => "rooms not both present",
        });
    }
    if let Some(n) = f.barrier_low_nbs {
        agg.barrier_low_nbs.add(match n {
            0 => "0",
            1 => "1",
            _ => "2+",
        });
    }
    if let Some(n) = f.barrier_faces {
        agg.barrier_faces.add(match n {
            0 => "0 (adjacent/remote only)",
            1 => "1",
            _ => "2+",
        });
    }
}

fn record_arbiter(a: &MapArbiter, plats: &[PerpetualFacts], bad_start: bool, agg: &mut Agg) {
    let line_today = a.unknown.is_empty();
    agg.line_today += u64::from(line_today);
    agg.all_floors_ignored += u64::from(line_today && a.others_ok && a.lifts_today);
    agg.all_honest += u64::from(line_today && a.others_ok && a.lifts_today && a.floors_ok);
    let perpetual_ok = !bad_start && plats.iter().all(PerpetualFacts::provisional);
    agg.provisional_plats += count_len(plats.iter().filter(|p| p.provisional()).count());
    for (i, (_, extra)) in COLUMNS.iter().enumerate() {
        let line = a.unknown.iter().all(|s| extra.contains(s));
        let admits_one_shot = extra.iter().any(|s| ONE_SHOT_LIFT.contains(s));
        let admits_perpetual = extra.iter().any(|s| START.contains(s));
        let lifts = if admits_one_shot {
            a.lifts_twin
        } else {
            a.lifts_today
        };
        let provisional = lifts && (!admits_perpetual || perpetual_ok);
        let col = &mut agg.columns[i];
        col.line += u64::from(line);
        col.all_today += u64::from(line && a.others_ok && a.floors_ok && a.lifts_today);
        col.provisional += u64::from(line && a.others_ok && a.floors_ok && provisional);
    }
}

// ---------------------------------------------------------------------------
// Report
// ---------------------------------------------------------------------------

fn report(label: &str, agg: &Agg) {
    println!("# liftprobe variants — {label}\n\nMaps: {}\n", agg.maps);
    report_resolution(agg);
    report_shape(agg);
    report_starts(agg);
    report_stops(agg);
    report_rendering(agg);
    report_concurrency(agg);
    report_one_shot(agg);
    report_arbiter(agg);
    report_limits();
}

fn report_resolution(agg: &Agg) {
    println!("## A. Perpetual tag resolution and conflicts\n");
    println!(
        "- 53/87 start lines: {} · tagged 0: {} · naming no sector: {} · maps with either: {}",
        agg.start_lines, agg.start_tag0, agg.start_dangling, agg.maps_with_bad_start
    );
    println!(
        "- sectors per resolving start tag (1 / 2 / 3+): {}",
        agg.start_tag_sectors.all()
    );
    println!(
        "- perpetual plats (sectors a 53/87 tag names): {} · maps with ≥1: {} ({})",
        agg.perpetual_n,
        agg.maps_with_perpetual,
        pct(agg.maps_with_perpetual, agg.maps)
    );
    println!(
        "- plats whose tag names 1 / 2 / 3+ sectors: {}",
        agg.shared_tag.all()
    );
    println!(
        "- plats whose tag also carries a DWUS/blaze lift line: {} ({})",
        agg.tag_has_lift,
        pct(agg.tag_has_lift, agg.perpetual_n)
    );
    println!(
        "- plats whose tag also carries a non-plat tagged special: {} ({}) · those specials: {}",
        agg.tag_has_other,
        pct(agg.tag_has_other, agg.perpetual_n),
        agg.other_specials.top(10)
    );
    println!(
        "- 54/89 stop lines per plat tag (0 / 1 / 2 / 3+): {} · form among plats with a stop line: {}",
        agg.stops_per_tag.all(),
        agg.stop_forms.all()
    );
}

fn report_shape(agg: &Agg) {
    println!("\n## B. Perpetual shape\n");
    println!("- rest at load: {}", agg.rest.all());
    println!("- neighbors: {}", agg.neighbor_count.all());
    println!("- distinct neighbor floors: {}", agg.nb_floors.all());
    println!("- shape class: {}", agg.class.all());
    println!("- rest × class: {}", agg.rest_x_class.all());
    println!(
        "- travel (high − low): {} · buckets: {}",
        percentiles(agg.travel.clone()),
        agg.travel_bucket.all()
    );
}

fn report_starts(agg: &Agg) {
    println!("\n## C. Perpetual start triggers\n");
    println!("- forms per plat: {}", agg.start_forms.all());
    println!("- start lines per plat tag: {}", agg.starts_per_tag.all());
    println!(
        "\nPer (start line, plat) pair — a line whose tag names several sectors is \
counted once per plat.\n"
    );
    println!("- placement: {}", agg.start_placement.all());
    println!("- activator: {}", agg.start_activator.all());
    println!(
        "- hops from the nearest activator sector: {}",
        agg.start_hops.all()
    );
    println!(
        "- borders the player-1 start's sector: {} pairs · {} plats ({})",
        agg.start_borders_p1_lines,
        agg.start_borders_p1_plats,
        pct(agg.start_borders_p1_plats, agg.perpetual_n)
    );
    println!(
        "- rest floor within one step of some neighbor's (boardable at rest): {} ({}) · by rest: {}",
        agg.boardable,
        pct(agg.boardable, agg.perpetual_n),
        agg.boardable_x_rest.all()
    );
}

fn report_stops(agg: &Agg) {
    println!("\n## D. Perpetual stop lines\n");
    println!(
        "- (stop line, plat) pairs: {} · form: {}",
        agg.stop_lines,
        agg.stop_form_lines.all()
    );
    println!("- placement: {}", agg.stop_placement.all());
    println!("- activator: {}", agg.stop_activator.all());
    println!("- hops: {}", agg.stop_hops.all());
    println!(
        "- plats with ≥1 stop line: {} ({}) · nearest stop line: {}",
        agg.plats_with_stop,
        pct(agg.plats_with_stop, agg.perpetual_n),
        agg.stop_nearest.all()
    );
}

fn report_rendering(agg: &Agg) {
    println!("\n## E. Perpetual rendering and cargo\n");
    println!(
        "- faces (two-sided boundaries to a neighbor at another floor): {} · visible lower present: {} ({}) · dontpegbottom: {} ({}) · level boundaries (no lower drawn): {}",
        agg.faces,
        agg.faces_lower_present,
        pct(agg.faces_lower_present, agg.faces),
        agg.faces_unpegged,
        pct(agg.faces_unpegged, agg.faces),
        agg.level_faces
    );
    println!("- floor flat shared with: {}", agg.flat_share.all());
    println!(
        "- light equal to a lowest neighbor's: {} ({}) · to a highest neighbor's: {} ({})",
        agg.light_lowest,
        pct(agg.light_lowest, agg.perpetual_n),
        agg.light_highest,
        pct(agg.light_highest, agg.perpetual_n)
    );
    println!(
        "- plats holding ≥1 thing: {} ({}) · things by class: {} · `other` names top 10: {}",
        agg.things_any,
        pct(agg.things_any, agg.perpetual_n),
        agg.thing_class.all(),
        agg.other_thing_names.top(10)
    );
}

fn report_concurrency(agg: &Agg) {
    println!("\n## F. Concurrency\n");
    println!(
        "- perpetual plats per map: {} · max: {}",
        agg.perpetual_per_map.all(),
        agg.perpetual_max
    );
    println!(
        "- moving DWUS/blaze plats per map: {} · max: {}",
        agg.moving_per_map.all(),
        agg.moving_max
    );
    println!(
        "- maps with ≥1 perpetual plat where perpetual + moving plats > {}: {} · > {}: {} · max combined among them: {}",
        MAX_PLATS / 2,
        agg.combined_over_15,
        MAX_PLATS,
        agg.combined_over_30,
        agg.combined_max
    );
}

fn report_one_shot(agg: &Agg) {
    println!("\n## G. One-shot lift breakouts\n");
    println!(
        "- plats whose lift triggers are all one-shot: {} · mixing one-shot and repeatable: {}",
        agg.one_shot_n, agg.mixed_n
    );
    println!("- one-shot trigger forms: {}", agg.one_shot_forms.all());
    println!(
        "- what-if shape × form (one-shot specials rewritten to their repeatable twins): {}",
        agg.shape_x_form.all()
    );
    println!(
        "- plats holding ≥1 thing: {} ({}) · things by class: {}",
        agg.one_shot_things_any,
        pct(agg.one_shot_things_any, agg.one_shot_n + agg.mixed_n),
        agg.one_shot_thing_class.all()
    );
    println!(
        "- what-if Core: level room and low room in the hop graph without the plat: {}",
        agg.core_connected.all()
    );
    println!(
        "- what-if Barrier: neighbors firing a trigger from Low (0 / 1 / 2+): {} · faces carrying an on-plat lift line: {}",
        agg.barrier_low_nbs.all(),
        agg.barrier_faces.all()
    );
    println!(
        "- S1 lines (21/122): {} · front slot starts with SW1: {} ({}) · SW2: {} ({}) · front middle/lower names top 10 (case-folded): {}",
        agg.s1_lines,
        agg.s1_sw1,
        pct(agg.s1_sw1, agg.s1_lines),
        agg.s1_sw2,
        pct(agg.s1_sw2, agg.s1_lines),
        agg.s1_textures.top(10)
    );
}

fn report_arbiter(agg: &Agg) {
    println!("\n## H. Yield ceilings\n");
    println!(
        "- baseline today: line axis {} ({}) · all axes, floors ignored {} ({}) · all axes, honest {} ({})",
        agg.line_today,
        pct(agg.line_today, agg.maps),
        agg.all_floors_ignored,
        pct(agg.all_floors_ignored, agg.maps),
        agg.all_honest,
        pct(agg.all_honest, agg.maps)
    );
    println!(
        "- perpetual plats passing the provisional gate: {} of {} ({})",
        agg.provisional_plats,
        agg.perpetual_n,
        pct(agg.provisional_plats, agg.perpetual_n)
    );
    println!(
        "\nPer column: **naive line axis** (every out-of-set special is in the column), \
**all axes as today** (that, and the six axes as shipped — a one-shot plat is still \
refused, a perpetual plat is not judged), and **provisional** (one-shot triggers read as \
repeatable; every perpetual plat passes the loose gate: one sector per tag, not dead, no \
stop line, no other family on the tag, rest at low or at high; no tag-0 or dangling start \
line). The provisional gate is a ceiling, not the design.\n"
    );
    for (i, (name, _)) in COLUMNS.iter().enumerate() {
        let c = &agg.columns[i];
        println!(
            "- {name}: line {} ({}) · all axes as today {} ({}) · provisional {} ({})",
            c.line,
            pct(c.line, agg.maps),
            c.all_today,
            pct(c.all_today, agg.maps),
            c.provisional,
            pct(c.provisional, agg.maps)
        );
    }
}

fn report_limits() {
    println!("\n## I. Not measured\n");
    println!(
        "- **Load-time heights.** `low` and `high` are the bounds `EV_DoPlat` would compute when the start line first fires at the heights the map loads with; a neighbor that has moved by then is not modeled."
    );
    println!(
        "- **`P_Random()&1`.** A perpetual plat's first direction is random; rest-at-low and rest-at-high say where it starts, not which way it goes first."
    );
    println!(
        "- **Stasis semantics.** `EV_StopPlat` freezes every active plat carrying the tag, of any type; whether a stop line ever fires before its start line, or freezes a DWUS lift on a shared tag, is not modeled."
    );
    println!(
        "- **`ML_BLOCKING`, monsters, the player's radius and use-reach.** Hop distances and the strand test are pure adjacency; activators are a height test."
    );
    println!(
        "- **UDMF-origin maps.** A map authored in UDMF is read with Doom special numbers, which its namespace need not honor."
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    use crate::common::tests::{Fixture, chain, fixture};

    /// The variant context of a fixture, with the player-1 start resolved.
    fn var_ctx<'a>(f: &'a Fixture, tables: &'a Tables) -> VarCtx<'a> {
        let ctx = map_ctx(&f.map, &f.scene, tables);
        VarCtx {
            p1_sector: f
                .scene
                .things
                .iter()
                .find(|t| t.type_id == 1)
                .and_then(|t| t.sector),
            ctx,
            tables,
        }
    }

    /// The perpetual facts of the one plat a fixture tags.
    fn perpetual(f: &Fixture, tables: &Tables, sector: usize) -> PerpetualFacts {
        let v = var_ctx(f, tables);
        let plat = perpetual_plats(&v.ctx)
            .into_iter()
            .find(|p| p.sector == sector)
            .expect("the sector is a perpetual plat");
        analyze_perpetual(&v, plat)
    }

    /// Appends a one-sided line on the last room's east wall, fronted by that
    /// room — the "remote line elsewhere" of the worked examples.
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

    /// A thing of `type_id` at the center of room `room` of a `chain`.
    fn thing(room: usize, type_id: i32) -> String {
        let x = room * 128 + 64;
        format!("thing {{ x = {x}.000; y = 64.000; type = {type_id}; single = true; }}\n")
    }

    #[test]
    fn the_special_tables_agree_with_common() {
        let mut both: Vec<i32> = START.iter().chain(&STOP).copied().collect();
        both.sort_unstable();
        let mut perpetual = PERPETUAL.to_vec();
        perpetual.sort_unstable();
        assert_eq!(both, perpetual);
        for s in ONE_SHOT_LIFT {
            assert!(is_lift(s) && !REPEATABLE_LIFT.contains(&s), "{s}");
        }
        assert!(ONE_SHOT_USE.iter().all(|s| ONE_SHOT_LIFT.contains(s)));
        assert_eq!(walk_form(53), "W1");
        assert_eq!(walk_form(87), "WR");
        assert_eq!(walk_form(54), "W1");
        assert_eq!(walk_form(89), "WR");
    }

    #[test]
    fn a_two_room_bounce_rests_at_high_and_is_boardable() {
        // A(0) – T(128) – B(128), started by an 87 line on the A|T edge.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 7, false), (0, 0, false)],
            "",
        ));
        let tables = Tables::load().expect("tables");
        let p = perpetual(&f, &tables, 1);
        assert_eq!((p.plat.low, p.plat.high), (0, 128));
        assert_eq!(p.plat.rest, PerpetualRest::AtHigh);
        assert_eq!(p.class, ShapeClass::TwoRoom);
        assert_eq!(p.distinct_nb_floors, 2);
        assert!(p.boardable_at_rest, "B is level with the plat");
        assert_eq!(p.starts.len(), 1);
        assert_eq!(p.starts[0].placement, Placement::OnPlatBack);
        // The 128-unit climb from A cannot cross the line; only the plat
        // side can, so the start fires from the plat itself.
        assert_eq!(p.starts[0].activators, vec![Activator::Plat]);
        assert_eq!(p.starts[0].hops, Some(0));
        assert!(!p.starts[0].borders_p1);
        assert_eq!(p.start_forms(), "WR only");
        assert!(p.stops.is_empty());
        assert!(p.provisional());
        assert!(!p.tag_has_lift && p.other_specials.is_empty());
        assert_eq!(p.shared_tag_n, 1);
    }

    #[test]
    fn rest_classes_follow_the_clamped_bounds() {
        let tables = Tables::load().expect("tables");
        // At low: the plat is the lowest of its neighborhood.
        let f = fixture(&chain(
            &[0, 0, 128],
            &[0, 7, 0],
            &[(53, 7, false), (0, 0, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!((p.plat.low, p.plat.high), (0, 128));
        assert_eq!(p.plat.rest, PerpetualRest::AtLow);
        assert_eq!(p.class, ShapeClass::TwoRoom);
        assert_eq!(p.start_forms(), "W1 only");
        assert!(p.provisional());

        // Between: a neighbor above and one below.
        let f = fixture(&chain(
            &[0, 64, 128],
            &[0, 7, 0],
            &[(53, 7, false), (0, 0, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.plat.rest, PerpetualRest::Between);
        assert!(!p.provisional(), "between is outside the gate");

        // Dead: every neighbor at the plat's own floor.
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(53, 7, false), (0, 0, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.plat.rest, PerpetualRest::Dead);
        assert_eq!(p.plat.high - p.plat.low, 0);
        assert_eq!(p.class, ShapeClass::Residual);
        assert!(!p.provisional());
        assert_eq!(p.faces.len(), 0);
        assert_eq!(p.level_faces, 2);
    }

    #[test]
    fn an_island_has_one_neighbor_and_one_clamped_bound() {
        let f = fixture(&chain(&[0, 96], &[0, 7], &[(87, 7, false)], ""));
        let tables = Tables::load().expect("tables");
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.neighbors.len(), 1);
        // `high` clamps to the plat's own floor: nothing is above it.
        assert_eq!((p.plat.low, p.plat.high), (0, 96));
        assert_eq!(p.plat.rest, PerpetualRest::AtHigh);
        assert_eq!(p.class, ShapeClass::Island);
        assert!(!p.boardable_at_rest);
        assert_eq!(
            p.flat_share,
            FlatShare::Lowest,
            "the fixture shares one flat"
        );
        assert!(p.light_eq_lowest && p.light_eq_highest);
    }

    #[test]
    fn shape_and_flat_classes_are_pure_functions_of_their_inputs() {
        assert_eq!(shape_class(0, 128, &[0, 128]), ShapeClass::TwoRoom);
        assert_eq!(shape_class(0, 128, &[0, 128, 64]), ShapeClass::TwoRoom);
        assert_eq!(
            shape_class(0, 128, &[0, 0, 128]),
            ShapeClass::Residual,
            "two neighbors at low"
        );
        assert_eq!(shape_class(0, 96, &[0]), ShapeClass::Island);
        assert_eq!(shape_class(0, 0, &[0]), ShapeClass::Island);
        assert_eq!(shape_class(0, 0, &[]), ShapeClass::Residual);

        assert_eq!(flat_share("A", &[]), FlatShare::None);
        assert_eq!(flat_share("A", &[(0, "A"), (128, "B")]), FlatShare::Lowest);
        assert_eq!(flat_share("B", &[(0, "A"), (128, "B")]), FlatShare::Highest);
        assert_eq!(
            flat_share("C", &[(0, "A"), (64, "C"), (128, "B")]),
            FlatShare::Other
        );
        assert_eq!(flat_share("Z", &[(0, "A"), (128, "B")]), FlatShare::None);
        assert_eq!(
            flat_share("A", &[(0, "A"), (0, "B")]),
            FlatShare::Lowest,
            "one neighboring floor: lowest wins"
        );

        assert_eq!(travel_bucket(24), "≤24");
        assert_eq!(travel_bucket(25), "25-64");
        assert_eq!(travel_bucket(128), "65-128");
        assert_eq!(travel_bucket(129), "129-256");
        assert_eq!(travel_bucket(257), ">256");
        assert_eq!(forms_label([53, 87].into_iter(), 53), "both");
        assert_eq!(forms_label([89, 89].into_iter(), 54), "WR only");
        assert_eq!(forms_label(std::iter::empty(), 53), "none");
    }

    #[test]
    fn a_stop_line_and_a_shared_family_refuse_the_provisional_gate() {
        let tables = Tables::load().expect("tables");
        // An 89 stop line across the B|C threshold, two rooms from the
        // plat: B is a neighbor, so the line is Adjacent and one hop off.
        let f = fixture(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 0, 0],
            &[(87, 7, false), (0, 0, false), (89, 7, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.stops.len(), 1);
        assert_eq!(p.stops[0].placement, Placement::Adjacent);
        assert_eq!(p.stops[0].activators, vec![Activator::Level]);
        assert_eq!(p.stops[0].hops, Some(1));
        assert_eq!(p.stop_forms(), "WR only");
        assert!(!p.provisional());

        // A stop line on the plat's own top edge.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 7, false), (54, 7, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.stops[0].placement, Placement::OnPlatFront);
        assert_eq!(p.stop_forms(), "W1 only");

        // A DWUS line on the same tag.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 7, false), (62, 7, true)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert!(p.tag_has_lift);
        assert!(p.other_specials.is_empty(), "a lift line is not `other`");
        assert!(!p.provisional());

        // A floor special on the tag is `other`; the start lines themselves
        // are not.
        let mut text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 7, false), (53, 7, false)],
            "",
        );
        far_wall(&mut text, 3, 23, 7);
        let f = fixture(&text);
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.other_specials, vec![23]);
        assert_eq!(p.start_forms(), "both");
        assert!(!p.provisional());

        // A tag naming two sectors.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 7],
            &[(87, 7, false), (0, 0, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.shared_tag_n, 2);
        assert!(!p.provisional());
    }

    #[test]
    fn start_lines_are_placed_and_the_player_start_is_noticed() {
        let tables = Tables::load().expect("tables");
        // A walkover across the A|T edge with a 16-unit step: crossable both
        // ways, and the player-1 start stands in A.
        let f = fixture(&chain(
            &[0, 16, 128],
            &[0, 7, 0],
            &[(87, 7, false), (0, 0, false)],
            &thing(0, 1),
        ));
        let v = var_ctx(&f, &tables);
        assert_eq!(v.p1_sector, Some(0));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(
            p.starts[0].activators,
            vec![Activator::Level, Activator::Plat]
        );
        assert!(p.starts[0].borders_p1);
        assert!(p.boardable_at_rest);

        // A remote start across the D|E threshold of a five-room row:
        // neither side is the plat or a neighbor, and D is two hops off.
        let f = fixture(&chain(
            &[0, 128, 128, 128, 128],
            &[0, 7, 0, 0, 0],
            &[(0, 0, false), (0, 0, false), (0, 0, false), (87, 7, false)],
            "",
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.starts[0].placement, Placement::Remote);
        assert_eq!(p.starts[0].activators, vec![Activator::Level]);
        assert_eq!(p.starts[0].hops, Some(2));
        assert!(!p.starts[0].borders_p1);

        // A one-sided walkover cannot be crossed at all: no activator, no
        // hop count (`P_CrossSpecialLine` needs a side to come from).
        let mut text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 87, 7);
        let f = fixture(&text);
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.starts[0].placement, Placement::Adjacent);
        assert_eq!(p.starts[0].activators, vec![Activator::None]);
        assert_eq!(p.starts[0].hops, None);
    }

    #[test]
    fn faces_read_the_visible_lower_and_things_are_classified() {
        let tables = Tables::load().expect("tables");
        // The plat stands above A (its riser is A's lower, `SUPPORT3`) and
        // below B (its own lower, also `SUPPORT3` in the fixture). An imp, a
        // medikit, a barrel, the player start and a soulsphere stand on it.
        let extra = [
            thing(1, 3001),
            thing(1, 2012),
            thing(1, 2035),
            thing(1, 1),
            thing(1, 2013),
            thing(1, 9_999),
        ]
        .concat();
        let f = fixture(&chain(
            &[0, 64, 128],
            &[0, 7, 0],
            &[(87, 7, false), (0, 0, false)],
            &extra,
        ));
        let p = perpetual(&f, &tables, 1);
        assert_eq!(p.faces.len(), 2);
        assert!(p.faces.iter().all(|f| f.lower_present && !f.dontpegbottom));
        assert_eq!(p.level_faces, 0);
        let mut classes = p.things.clone();
        classes.sort();
        assert_eq!(
            classes,
            vec![
                ThingClass::Monster,
                ThingClass::Pickup,
                ThingClass::Prop,
                ThingClass::Start,
                ThingClass::Other,
                ThingClass::Other,
            ]
        );
        assert_eq!(p.other_thing_names, vec!["soulsphere", "type 9999"]);

        // The dontpegbottom flag is read off the linedef.
        let mut f = fixture(&chain(
            &[0, 64, 128],
            &[0, 7, 0],
            &[(87, 7, false), (0, 0, false)],
            "",
        ));
        f.map.linedefs[0].flags |= u32::from(
            tables
                .linedef_flag("lower_unpegged")
                .expect("sourced in engine.toml"),
        );
        f.scene = Scene::build(&f.map, &tables, &mut Vec::new());
        let p = perpetual(&f, &tables, 1);
        assert_eq!(
            p.faces.iter().filter(|f| f.dontpegbottom).count(),
            1,
            "one of the two faces is unpegged"
        );
    }

    #[test]
    fn a_one_shot_core_lift_is_stranded_when_the_plat_is_the_only_route() {
        let tables = Tables::load().expect("tables");
        // A(0) – T(128) – B(128) with a 21 riser: the twin is a Core lift,
        // and without T there is no way from B to A.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(21, 7, false), (0, 0, false)],
            "",
        ));
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        survey_one_shot(&v, &mut agg);
        assert_eq!((agg.one_shot_n, agg.mixed_n), (1, 0));
        assert_eq!(agg.shape_x_form.all(), "all-one-shot / Core / S1 only: 1");
        assert_eq!(
            agg.core_connected.all(),
            "stranded (plat is the only route): 1"
        );
        assert_eq!(agg.s1_lines, 1);
        assert_eq!(agg.s1_sw1, 0, "the riser is bare SUPPORT3");
        assert_eq!(agg.s1_textures.all(), "SUPPORT3: 1");

        // Add a fourth room joining A and B around the plat, and the rooms
        // stay connected without it.
        let extra = "vertex { x = 0.000; y = -128.000; }\nvertex { x = 384.000; y = -128.000; }\n";
        let mut text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(21, 7, false), (10, 7, false)],
            extra,
        );
        // Vertices 0 (0,0) and 6 (384,0) are the row's south corners; the
        // new room D (sector 3) spans below the whole row, sharing A's and
        // B's south walls, which the chain drew one-sided. Replace those two
        // walls: re-declare them two-sided is not possible after the fact, so
        // instead give D its own two-sided lines and leave the chain's walls
        // as they are — the hop graph only needs D to border A and B.
        let sd = text.matches("sidedef {").count();
        let _ = writeln!(
            text,
            "linedef {{ v1 = 0; v2 = 2; sidefront = {sd}; sideback = {}; twosided = true; }}\n\
             linedef {{ v1 = 4; v2 = 6; sidefront = {}; sideback = {}; twosided = true; }}\n\
             linedef {{ v1 = 8; v2 = 0; sidefront = {}; blocking = true; }}\n\
             linedef {{ v1 = 9; v2 = 8; sidefront = {}; blocking = true; }}\n\
             linedef {{ v1 = 6; v2 = 9; sidefront = {}; blocking = true; }}\n\
             sidedef {{ sector = 3; }}\nsidedef {{ sector = 0; }}\n\
             sidedef {{ sector = 3; }}\nsidedef {{ sector = 2; }}\n\
             sidedef {{ sector = 3; }}\nsidedef {{ sector = 3; }}\nsidedef {{ sector = 3; }}\n\
             sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }}",
            sd + 1,
            sd + 2,
            sd + 3,
            sd + 4,
            sd + 5,
            sd + 6
        );
        let f = fixture(&text);
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        survey_one_shot(&v, &mut agg);
        assert_eq!(
            agg.one_shot_forms.all(),
            "all-one-shot / S1+W1: 1",
            "a 21 and a 10 on one plat"
        );
        assert_eq!(
            agg.core_connected.all(),
            "still connected without the plat: 1"
        );
    }

    #[test]
    fn a_one_shot_barrier_counts_its_faces_and_low_callers() {
        let tables = Tables::load().expect("tables");
        // A(0) – T(96) – B(0), a 21 on the A face only.
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(21, 7, false), (0, 0, false)],
            &thing(1, 3001),
        ));
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        survey_one_shot(&v, &mut agg);
        assert_eq!(
            agg.shape_x_form.all(),
            "all-one-shot / Barrier / S1 only: 1"
        );
        assert_eq!(agg.barrier_low_nbs.all(), "1: 1");
        assert_eq!(agg.barrier_faces.all(), "1: 1");
        assert_eq!(agg.one_shot_things_any, 1);
        assert_eq!(agg.one_shot_thing_class.all(), "monster: 1");
        assert!(agg.core_connected.0.is_empty());

        // Both faces, one of them the repeatable twin already: mixed.
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(21, 7, false), (62, 7, true)],
            "",
        ));
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        survey_one_shot(&v, &mut agg);
        assert_eq!((agg.one_shot_n, agg.mixed_n), (0, 1));
        assert_eq!(agg.barrier_low_nbs.all(), "2+: 1");
        assert_eq!(agg.barrier_faces.all(), "2+: 1");
    }

    #[test]
    fn connectivity_and_trigger_faces_are_pure_graph_facts() {
        let f = fixture(&chain(
            &[0, 0, 0, 0],
            &[0, 0, 0, 0],
            &[(0, 0, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        let (a, b) = (BTreeSet::from([0]), BTreeSet::from([3]));
        assert!(connected_without(&f.scene, &a, &b, 9));
        assert!(!connected_without(&f.scene, &a, &b, 2));
        assert!(
            !connected_without(&f.scene, &a, &b, 0),
            "removing the source itself"
        );

        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(62, 7, false), (62, 7, true)],
            "",
        ));
        assert_eq!(trigger_faces(&f.map, 1, 7), 2);
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(trigger_faces(&f.map, 1, 7), 1);
    }

    #[test]
    fn thing_classes_come_from_the_tables() {
        let tables = Tables::load().expect("tables");
        let named = |name: &str, type_id: i32| SceneThing {
            x: 0.0,
            y: 0.0,
            angle: 0,
            type_id,
            flags: 0,
            sector: None,
            name: Some(name.to_owned()),
        };
        for (name, class) in [
            ("imp", ThingClass::Monster),
            ("cyberdemon", ThingClass::Monster),
            ("barrel", ThingClass::Prop),
            ("tall_blue_torch", ThingClass::Prop),
            ("medikit", ThingClass::Pickup),
            ("box_of_rockets", ThingClass::Pickup),
            ("shotgun", ThingClass::Pickup),
            ("chainsaw", ThingClass::Pickup),
            ("backpack", ThingClass::Pickup),
            ("blue_card", ThingClass::Pickup),
            ("player1_start", ThingClass::Start),
            ("deathmatch_start", ThingClass::Start),
            ("soulsphere", ThingClass::Other),
            ("teleport_dest", ThingClass::Other),
            ("candle", ThingClass::Other),
        ] {
            assert_eq!(thing_class(&tables, &named(name, 0)), class, "{name}");
        }
        let unnamed = SceneThing {
            name: None,
            ..named("x", 9_999)
        };
        assert_eq!(thing_class(&tables, &unnamed), ThingClass::Other);
    }

    #[test]
    fn the_arbiter_columns_admit_exactly_their_specials() {
        // A map whose only out-of-set special is 87 clears the two
        // perpetual columns and the all-eight column, but not the one-shot
        // one; with a stop line it needs the four-special column.
        let plats: Vec<PerpetualFacts> = Vec::new();
        let mut agg = Agg {
            columns: (0..COLUMNS.len())
                .map(|_| ArbiterColumn::default())
                .collect(),
            ..Agg::default()
        };
        let a = MapArbiter {
            unknown: vec![87],
            others_ok: true,
            floors_ok: true,
            lifts_today: true,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, false, &mut agg);
        let lines: Vec<u64> = agg.columns.iter().map(|c| c.line).collect();
        assert_eq!(lines, vec![0, 1, 1, 0, 1]);
        assert_eq!(agg.line_today, 0);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 1, 1, 0, 1]);

        // The same map with a tag-0 start line fails the provisional gate
        // on every perpetual column but keeps the line axis.
        let mut agg = Agg {
            columns: (0..COLUMNS.len())
                .map(|_| ArbiterColumn::default())
                .collect(),
            ..Agg::default()
        };
        record_arbiter(&a, &plats, true, &mut agg);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 0, 0, 0, 0]);
        let all_today: Vec<u64> = agg.columns.iter().map(|c| c.all_today).collect();
        assert_eq!(all_today, vec![0, 1, 1, 0, 1]);

        // A one-shot map: the recognizer refuses it today and accepts the
        // twin, so only the one-shot columns' provisional count moves.
        let mut agg = Agg {
            columns: (0..COLUMNS.len())
                .map(|_| ArbiterColumn::default())
                .collect(),
            ..Agg::default()
        };
        let a = MapArbiter {
            unknown: vec![21, 54],
            others_ok: true,
            floors_ok: true,
            lifts_today: false,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, false, &mut agg);
        let lines: Vec<u64> = agg.columns.iter().map(|c| c.line).collect();
        assert_eq!(lines, vec![0, 0, 0, 0, 1]);
        let all_today: Vec<u64> = agg.columns.iter().map(|c| c.all_today).collect();
        assert_eq!(all_today, vec![0, 0, 0, 0, 0]);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 0, 0, 0, 1]);

        // An expressible map today counts on every row and column.
        let mut agg = Agg {
            columns: (0..COLUMNS.len())
                .map(|_| ArbiterColumn::default())
                .collect(),
            ..Agg::default()
        };
        let a = MapArbiter {
            unknown: vec![],
            others_ok: true,
            floors_ok: false,
            lifts_today: true,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, false, &mut agg);
        assert_eq!(
            (agg.line_today, agg.all_floors_ignored, agg.all_honest),
            (1, 1, 0)
        );
        assert!(agg.columns.iter().all(|c| c.line == 1 && c.all_today == 0));
    }

    #[test]
    fn the_honest_verdict_folds_in_all_three_recognizers() {
        let tables = Tables::load().expect("tables");
        let vocab = Vocabulary::from_tables(&tables);
        // A plain Core lift on 62: expressible on every axis.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let a = map_arbiter("MAP01", &f.map, &f.scene, &tables, &vocab);
        assert!(a.unknown.is_empty() && a.others_ok && a.floors_ok && a.lifts_today);

        // The one-shot twin: refused today, accepted once rewritten, and 21
        // is the one unknown special.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(21, 7, false), (0, 0, false)],
            "",
        ));
        let a = map_arbiter("MAP01", &f.map, &f.scene, &tables, &vocab);
        assert_eq!(a.unknown, vec![21]);
        assert!(!a.lifts_today && a.lifts_twin);

        // A dead floor target is what the floor recognizer refuses.
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(23, 7, false), (0, 0, false)],
            "",
        ));
        let a = map_arbiter("MAP01", &f.map, &f.scene, &tables, &vocab);
        assert!(a.unknown.is_empty() && !a.floors_ok);
    }

    #[test]
    fn concurrency_counts_perpetual_and_moving_plats_per_map() {
        let tables = Tables::load().expect("tables");
        // One perpetual plat and one moving DWUS lift in one map.
        let f = fixture(&chain(
            &[0, 128, 128, 0],
            &[0, 7, 8, 0],
            &[(87, 7, false), (0, 0, false), (62, 8, true)],
            "",
        ));
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        let plats = perpetual_plats(&v.ctx);
        assert_eq!(plats.len(), 1);
        let moving = survey_concurrency(&v, plats.len(), &mut agg);
        assert_eq!(moving, 1);
        assert_eq!(agg.perpetual_per_map.all(), "1: 1");
        assert_eq!(agg.moving_per_map.all(), "1: 1");
        assert_eq!((agg.combined_max, agg.combined_over_15), (2, 0));

        // §A's tag resolution: a tag-0 and a dangling start line.
        let mut text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 0, false), (53, 99, false)],
            "",
        );
        far_wall(&mut text, 3, 87, 7);
        let f = fixture(&text);
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        assert!(survey_start_lines(&v, &mut agg));
        assert_eq!(
            (agg.start_lines, agg.start_tag0, agg.start_dangling),
            (3, 1, 1)
        );
        assert_eq!(agg.start_tag_sectors.all(), "1: 1");
    }
}
