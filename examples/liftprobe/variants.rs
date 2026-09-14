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

use crustygen::check::plats::{
    Activator as SceneActivator, ScenePlat, SceneTrigger, resolve_plats,
};
use crustygen::check::scene::{Scene, SceneThing};
use crustygen::lift::plat::{Refusal, Rest as PlatRest, Shape as PlatShape, Speed};
use crustygen::lift::{self, vocabulary::Vocabulary};
use crustygen::tables::Tables;
use crustywad::map::udmf::UdmfMap;

use crate::common::{
    self, Activator, BLAZE, DWUS, Dispatch, Hist, PERPETUAL, Placement, PlatFacts, REPEATABLE_LIFT,
    Shape, USE_LIFT, is_lift, pct, percentiles,
};
use crate::floors::{
    MapCtx, PerpetualPlat, PerpetualRest, bucket_count, count_len, hop_bucket, map_ctx,
    neighbor_side, neighbors_of, one_shot_split, perpetual_plats, repeatable_twin_map,
};

/// The perpetual start specials, the first half of [`PERPETUAL`] (the one
/// place the probe spells these numbers): `p_spec.c:682-686` (53, W1 — the case ends
/// `line->special = 0`) and `:852-855` (87, WR). Both are in
/// `P_CrossSpecialLine`, so a perpetual plat is walkover-started from either
/// side; no use or gun form dispatches `perpetualRaise`.
const START: [i32; 2] = [PERPETUAL[0], PERPETUAL[1]];

/// The stop specials, the second half of [`PERPETUAL`]: `p_spec.c:688-692` (54, W1) and `:862-865` (89, WR),
/// both calling `EV_StopPlat` (`p_plats.c:273-286`), which puts **every**
/// active plat carrying the line's tag — of any type — into `in_stasis`.
const STOP: [i32; 2] = [PERPETUAL[2], PERPETUAL[3]];

/// The one-shot `downWaitUpStay` / `blazeDWUS` forms, the S1/W1 entries of
/// [`DWUS`] and [`BLAZE`]: 21 (S1,
/// `p_switch.c:389-393`), 122 (S1 blazing, `:479-483`), 10 (W1,
/// `p_spec.c:579-583`), 121 (W1 blazing, `:754-758`). The two S1 forms call
/// `P_ChangeSwitchTexture(line, 0)`, whose `useAgain == 0` arm clears the
/// special and swaps the front texture for good (`p_switch.c:211-212`,
/// `:229`, `:241`, `:253`).
const ONE_SHOT_LIFT: [i32; 4] = [DWUS[1], DWUS[3], BLAZE[1], BLAZE[3]];

/// The use-activated half of [`ONE_SHOT_LIFT`], the S1 entries of [`USE_LIFT`].
const ONE_SHOT_USE: [i32; 2] = [USE_LIFT[1], USE_LIFT[3]];

/// `MAXPLATS` (`p_spec.h:306`), read from the sourced table
/// ([`Tables::plat`]`().max_active`) rather than restated here:
/// `P_AddActivePlat` `I_Error`s once the table is full (`p_plats.c:288-299`).
/// A perpetual plat is never removed (`T_PlatRaise`, `p_plats.c:89-103`: only
/// the four one-way types are), so it holds a slot for the rest of the level,
/// in stasis or not. The `tag_size_bucket` labels spell the same bound out;
/// `tables::tests::plat_constants_are_the_pinned_engines` pins it.
fn max_plats(tables: &Tables) -> usize {
    tables.plat().max_active
}

/// Every variant special at once, [`PERPETUAL`] then [`ONE_SHOT_LIFT`], for
/// the last §H column — built from the named arrays so the admitted set can
/// never drift from the cited engine definitions.
const ALL_VARIANTS: [i32; 8] = [
    PERPETUAL[0],
    PERPETUAL[1],
    PERPETUAL[2],
    PERPETUAL[3],
    ONE_SHOT_LIFT[0],
    ONE_SHOT_LIFT[1],
    ONE_SHOT_LIFT[2],
    ONE_SHOT_LIFT[3],
];

/// The §H columns: a label and the specials the line axis admits beyond
/// today's set — slices of the named arrays, never literals.
const COLUMNS: [(&str, &[i32]); 5] = [
    ("today", &[]),
    ("+{53,87}", &START),
    ("+{53,87,54,89}", &PERPETUAL),
    ("+{21,10,122,121}", &ONE_SHOT_LIFT),
    ("+all eight", &ALL_VARIANTS),
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
    ///
    /// These are the gate's five **per-plat** clauses. It has a sixth,
    /// map-level one — no tag-0 or dangling start line anywhere in the map —
    /// which `record_arbiter` applies to both the column and the plat count.
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

/// The sector holding the first player-1 start the scene could place, the
/// start's type resolved through the vocabulary (`[things] player1_start`,
/// the name `check::conform` reads it by) rather than spelled here.
fn player1_sector(scene: &Scene, tables: &Tables) -> Option<usize> {
    let start = i32::from(
        tables
            .thing_id("player1_start")
            .expect("player1_start is in the vocabulary"),
    );
    scene
        .things
        .iter()
        .find(|t| t.type_id == start)
        .and_then(|t| t.sector)
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
        // `(false, false)` is unreachable: `common::analyze_plat` returns
        // `None` for a plat with no trigger at all — a lift line whose front
        // sidedef dangles contributes none — so every plat `one_shot_split`
        // yields has at least one non-repeatable trigger, which sets `s1` or
        // `w1` above.
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
    perpetual_tag_max: Hist,
    lift_tag_max: Hist,
    perpetual_tag_max_n: u64,
    lift_tag_max_n: u64,
    maps_perpetual_tag_over_30: u64,
    maps_lift_tag_over_30: u64,
    combined_over_15: u64,
    combined_over_30: u64,
    /// `MAXPLATS` as the table reports it, carried for the report's labels.
    max_plats: usize,
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
    // I
    groups_n: u64,
    groups_size: Hist,
    groups_floor_class: Hist,
    members_n: u64,
    member_verdict: Hist,
    group_accept: Hist,
    group_composition: Hist,
    member_own_line: u64,
    member_callable_low: u64,
    member_own_low_line: u64,
    member_neighbor_low_line: u64,
    member_within_one_hop: u64,
    group_self_callable: Hist,
    group_neighbor_called: Hist,
    common_neighbor_all_a_prime: u64,
    group_callable_low: Hist,
    line_reach: Hist,
    group_line_reach: Hist,
    group_lines_n: Hist,
    group_one_form: u64,
    group_one_speed: u64,
    uniform_rest: u64,
    uniform_travel: u64,
    uniform_low: u64,
    uniform_neighbors: u64,
    common_neighbor: Hist,
    bank_caller_refusals: u64,
    /// The historical `SharedTag` population: every member of a multi-sector
    /// lift tag that is not refused `Dead` — what `lift::plat` refused on
    /// sight before the bank construct (`Dead` was precedence 1 and took
    /// those members first). Unchanged by the construct, and the denominator
    /// every §I "recovered of N" is measured against.
    historical_shared_members: u64,
    recognizer_split: u64,
    unshared_mismatch: u64,
    bank_columns: Vec<BankColumn>,
}

/// One §I yield column's counts.
#[derive(Default)]
struct BankColumn {
    /// Maps expressible on all six axes with this column's lift axis: every
    /// bank group accepted by this column's gate **or** by the shipped
    /// recognizer ([`BankVerdict::fold_group`]).
    all_honest: u64,
    /// Platforms of the historical `SharedTag` population
    /// ([`Agg::historical_shared_members`]) whose group this column's **gate**
    /// accepts (the relaxation clause is map-level and does not enter this
    /// count).
    recovered: u64,
    /// Groups this column's **gate** accepts.
    groups: u64,
}

/// The §I column labels, in report order.
///
/// Each label after `today` names the gate the column *adds*: a map counts
/// when every bank group is accepted by that gate **or** by the shipped
/// recognizer, so every column is a superset of `today`
/// ([`BankVerdict::fold_group`]).
const BANK_COLUMNS: [&str; 5] = [
    "today",
    "+A (bank-A gate)",
    "+A′ (neighbor-called)",
    "+B (bank-B ceiling)",
    "+split as one lift",
];

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
        bank_columns: (0..BANK_COLUMNS.len())
            .map(|_| BankColumn::default())
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
        p1_sector: player1_sector(&scene, tables),
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
    let perpetual_sectors: BTreeSet<usize> = plats.iter().map(|p| p.plat.sector).collect();
    survey_concurrency(&v, &perpetual_sectors, agg);
    survey_one_shot(&v, agg);
    let banks = survey_banks(&v, agg);
    record_arbiter(&arbiter, &plats, bad_start, banks, agg);
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

/// The most sectors any one tag among `specials`' lines names in the map,
/// `None` when no such line resolves.
fn max_sectors_per_tag(v: &VarCtx<'_>, is_special: impl Fn(i32) -> bool) -> Option<usize> {
    v.ctx
        .map
        .linedefs
        .iter()
        .filter(|l| is_special(l.special) && l.args[0] != 0)
        .filter_map(|l| v.ctx.index.by_tag.get(&l.args[0]).map(Vec::len))
        .max()
}

/// The bucket for the most sectors one tag names.
fn tag_size_bucket(n: usize) -> &'static str {
    match n {
        0..=15 => "0-15",
        16..=30 => "16-30",
        _ => "31+",
    }
}

/// §F — how many plat thinkers a map can have alive at once. `perpetual`
/// is the set of perpetual plat sectors; the moving DWUS/blaze plats are
/// unioned with it, so a sector named by both a 53/87 and a lift tag
/// counts once. Returns the count of moving DWUS/blaze plats.
///
/// The per-map sum is an upper bound on concurrency. What `EV_DoPlat`
/// allocates in one call is one `plat_t` per sector matching **one** line's
/// tag (`p_plats.c:164-181`), so the per-tag maximum is the figure that
/// can overflow `MAXPLATS` at once; several smaller banks need never be
/// active together.
fn survey_concurrency(v: &VarCtx<'_>, perpetual: &BTreeSet<usize>, agg: &mut Agg) -> usize {
    let max_plats = max_plats(v.tables);
    agg.max_plats = max_plats;
    let moving_set: BTreeSet<usize> = v
        .ctx
        .index
        .plat_sectors(v.ctx.map)
        .into_iter()
        .filter(|&s| {
            common::analyze_plat(v.ctx.map, v.ctx.scene, &v.ctx.index, s, v.ctx.step)
                .is_some_and(|f| f.moving())
        })
        .collect();
    let moving = moving_set.len();
    if let Some(n) = max_sectors_per_tag(v, |s| START.contains(&s)) {
        agg.perpetual_tag_max.add(tag_size_bucket(n));
        agg.perpetual_tag_max_n = agg.perpetual_tag_max_n.max(count_len(n));
        agg.maps_perpetual_tag_over_30 += u64::from(n > max_plats);
    }
    if let Some(n) = max_sectors_per_tag(v, is_lift) {
        agg.lift_tag_max.add(tag_size_bucket(n));
        agg.lift_tag_max_n = agg.lift_tag_max_n.max(count_len(n));
        agg.maps_lift_tag_over_30 += u64::from(n > max_plats);
    }
    let perpetual_n = perpetual.len();
    let bucket = |n: usize| match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3..=5 => "3-5",
        6..=10 => "6-10",
        _ => "11+",
    };
    agg.perpetual_per_map.add(bucket(perpetual_n));
    agg.perpetual_max = agg.perpetual_max.max(count_len(perpetual_n));
    agg.moving_per_map.add(bucket(moving));
    agg.moving_max = agg.moving_max.max(count_len(moving));
    // The combined rows are over maps that have a perpetual plat at all:
    // a map of DWUS lifts alone never holds a slot for the whole level.
    if perpetual_n > 0 {
        let combined = perpetual.union(&moving_set).count();
        agg.combined_max = agg.combined_max.max(count_len(combined));
        agg.combined_over_15 += u64::from(combined > max_plats / 2);
        agg.combined_over_30 += u64::from(combined > max_plats);
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

fn record_arbiter(
    a: &MapArbiter,
    plats: &[PerpetualFacts],
    bad_start: bool,
    banks: BankVerdict,
    agg: &mut Agg,
) {
    let line_today = a.unknown.is_empty();
    let base = line_today && a.others_ok && a.floors_ok;
    for (i, lifts) in [a.lifts_today, banks.a, banks.a_prime, banks.b, banks.split]
        .into_iter()
        .enumerate()
    {
        agg.bank_columns[i].all_honest += u64::from(base && lifts);
    }
    agg.line_today += u64::from(line_today);
    agg.all_floors_ignored += u64::from(line_today && a.others_ok && a.lifts_today);
    agg.all_honest += u64::from(line_today && a.others_ok && a.lifts_today && a.floors_ok);
    let perpetual_ok = !bad_start && plats.iter().all(PerpetualFacts::provisional);
    // The gate has a map-level clause as well as the five per-plat ones: a
    // tag-0 or dangling start line anywhere in the map fails it, so no plat
    // of that map passes it either (`perpetual_ok` above, and the legend).
    agg.provisional_plats += if bad_start {
        0
    } else {
        count_len(plats.iter().filter(|p| p.provisional()).count())
    };
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
// §I — lift tag groups (banks)
//
// One line drives every sector carrying its tag: `EV_DoPlat` loops
// `while ((secnum = P_FindSectorFromLineTag(line,secnum)) >= 0)`
// (`p_plats.c:164-169`), and each sector gets its own `plat_t` whose `low` is
// `P_FindLowestFloorSurrounding(sec)` over ITS neighbors (`p_plats.c:207-212`
// for `downWaitUpStay`). A bank is one tag, several thinkers, each judged
// from its own neighborhood — which is why every member below is judged
// alone before the group is.
// ---------------------------------------------------------------------------

/// `lift::plat`'s per-platform verdict re-derived on a platform judged **as
/// if its tag were unshared**: the same eight refusals in the same order as
/// `src/lift/plat.rs:432-456`, with the `BankCaller` arm skipped, and the
/// same shape rule (`:462-469`). Skipping that one arm, rather than reading
/// the recognizer's own `Some(Refusal::BankCaller)` back as accepted,
/// matters for a bank member: a platform the recognizer stops at
/// `BankCaller` has not yet been checked against `ConflictingAction`, the
/// arm that follows it, and this function still runs that check before
/// accepting. `lift::plat` is not changed; its refusal function is private
/// and takes the resolved `shared_tag` as read, so the order is re-derived
/// here — checked against the recognizer on every single-tag platform,
/// where the skipped arm can never fire either way (`Agg::unshared_mismatch`),
/// and used unchanged as a bank member's own verdict (`analyze_group`).
fn unshared_verdict(p: &ScenePlat) -> Result<PlatShape, Refusal> {
    let speed = speed_of(&p.triggers);
    let one_floor = p.distinct_neighbor_floors == 1;
    if p.rest == PlatRest::Dead {
        Err(Refusal::Dead)
    } else if !p.triggers.iter().all(|t| t.repeatable) {
        Err(Refusal::OneShot)
    } else if speed == Speed::Mixed {
        Err(Refusal::MixedSpeed)
    } else if p.rest == PlatRest::Intermediate || (p.rest == PlatRest::AboveAll && !one_floor) {
        Err(Refusal::UnsupportedRest)
    } else if !p.callable_low() {
        Err(Refusal::TopOnly)
    } else if p.rest == PlatRest::AboveAll
        && p.neighbors.len() >= 2
        && p.low_activator_neighbors().len() < p.neighbors.len()
    {
        Err(Refusal::OneWayBarrier)
    } else if !p.other_actions.is_empty() {
        Err(Refusal::ConflictingAction)
    } else {
        Ok(shape_of(p.rest, p.neighbors.len()))
    }
}

/// `lift::plat`'s speed rule (`src/lift/plat.rs:385-395`).
fn speed_of(triggers: &[SceneTrigger]) -> Speed {
    let fasts = triggers.iter().filter(|t| t.fast).count();
    if fasts == 0 {
        Speed::Normal
    } else if fasts == triggers.len() {
        Speed::Fast
    } else {
        Speed::Mixed
    }
}

/// `lift::plat`'s shape rule for an unrefused platform
/// (`src/lift/plat.rs:462-469`).
fn shape_of(rest: PlatRest, neighbors: usize) -> PlatShape {
    match (rest, neighbors) {
        (PlatRest::Top, _) => PlatShape::Lift,
        (_, 1) => PlatShape::Pedestal,
        _ => PlatShape::Barrier,
    }
}

/// The label a member verdict prints.
fn verdict_label(v: Result<PlatShape, Refusal>) -> String {
    match v {
        Ok(shape) => format!("{shape:?}"),
        Err(r) => format!("refused: {r:?}"),
    }
}

/// Where a lift line sits relative to one member.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Reach {
    /// A side of the line is the member (any face).
    OnFace,
    /// A side of the line is a neighbor of the member.
    Adjacent,
    /// Neither.
    Remote,
}

impl Reach {
    fn of(t: &SceneTrigger, sector: usize, neighbors: &BTreeSet<usize>) -> Self {
        if t.front == sector || t.back == Some(sector) {
            Self::OnFace
        } else if neighbors.contains(&t.front) || t.back.is_some_and(|b| neighbors.contains(&b)) {
            Self::Adjacent
        } else {
            Self::Remote
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::OnFace => "on a member face",
            Self::Adjacent => "adjacent to a member",
            Self::Remote => "remote from every member",
        }
    }
}

/// One member of a bank, judged alone.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent measured fact about who calls the member; they \
              encode no joint state"
)]
struct Member {
    verdict: Result<PlatShape, Refusal>,
    /// Some lift line lies on the member's own boundary.
    own_line: bool,
    /// Some lift line fires from a `Low` activator relative to the member
    /// (`ScenePlat::callable_low`, `src/check/plats.rs:149`).
    callable_low: bool,
    /// Some lift line on the member's own boundary fires from `Low` — the
    /// shipped construct's "called from its own face".
    own_low_line: bool,
    /// Some lift line naming the tag fires from a sector that is both a
    /// two-sided neighbor of the member and at the member's own `low` —
    /// adjacency plus height. The line itself may sit anywhere.
    neighbor_low_line: bool,
    /// Some lift line is on the member's face or adjacent to it.
    within_one_hop: bool,
    rest: PlatRest,
    travel: i32,
    low: i32,
    /// Two-sided neighbors that are not themselves members.
    outside_neighbors: BTreeSet<usize>,
}

/// How a group's members sit.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum FloorClass {
    /// One floor and mutually adjacent — one platform split by trim.
    Split,
    /// One floor, not mutually adjacent — several lifts on one trigger.
    OneFloorDisconnected,
    /// Members at several floors.
    SeveralFloors,
}

impl FloorClass {
    fn label(self) -> &'static str {
        match self {
            Self::Split => "one floor, mutually adjacent (split)",
            Self::OneFloorDisconnected => "one floor, disconnected",
            Self::SeveralFloors => "several floors",
        }
    }
}

/// One lift tag naming two or more sectors.
struct Group {
    members: Vec<Member>,
    /// Every distinct lift line naming the tag, with its nearest reach to
    /// any member.
    line_reach: Vec<Reach>,
    one_form: bool,
    one_speed: bool,
    floor_class: FloorClass,
    /// The verdict on the split group read as one lift, when it is one.
    merged: Option<Result<PlatShape, Refusal>>,
}

impl Group {
    /// Every member passes alone.
    fn gate_b(&self) -> bool {
        self.members.iter().all(|m| m.verdict.is_ok())
    }

    /// Gate B, and every member has a `Low`-activator line on its own face.
    fn gate_a(&self) -> bool {
        self.gate_b() && self.members.iter().all(|m| m.own_low_line)
    }

    /// Gate B, and every member is called from a neighbor standing at its
    /// own `low`.
    fn gate_a_prime(&self) -> bool {
        self.gate_b() && self.members.iter().all(|m| m.neighbor_low_line)
    }

    /// A split group whose merged reading is a shape.
    fn gate_split(&self) -> bool {
        self.merged.is_some_and(|v| v.is_ok())
    }

    /// `all / some / none` of the members satisfy `pred`.
    fn split_label(&self, pred: impl Fn(&Member) -> bool) -> &'static str {
        let n = self.members.iter().filter(|m| pred(m)).count();
        if n == self.members.len() {
            "all"
        } else if n == 0 {
            "none"
        } else {
            "some"
        }
    }

    fn composition(&self) -> &'static str {
        if self.members.iter().any(|m| m.verdict.is_err()) {
            return "any refused";
        }
        let first = self.members[0].verdict.ok();
        if self.members.iter().any(|m| m.verdict.ok() != first) {
            return "mixed shapes";
        }
        match first {
            Some(PlatShape::Lift) => "all Lift",
            Some(PlatShape::Pedestal) => "all Pedestal",
            Some(PlatShape::Barrier) => "all Barrier",
            None => "any refused",
        }
    }

    fn line_reach_label(&self) -> &'static str {
        if self.line_reach.iter().all(|&r| r == Reach::OnFace) {
            "all on member faces"
        } else if self.line_reach.iter().all(|&r| r == Reach::Remote) {
            "all remote"
        } else {
            "mixed"
        }
    }

    /// `all share one / some share / none in common` over the members'
    /// outside neighbors.
    fn common_neighbor(&self) -> &'static str {
        let mut sets = self.members.iter().map(|m| &m.outside_neighbors);
        let Some(first) = sets.next() else {
            return "none in common";
        };
        let all: BTreeSet<usize> = sets.fold(first.clone(), |acc, s| &acc & s);
        if !all.is_empty() {
            return "all share one neighbor";
        }
        let any_pair = self.members.iter().enumerate().any(|(i, a)| {
            self.members[i + 1..]
                .iter()
                .any(|b| !(&a.outside_neighbors & &b.outside_neighbors).is_empty())
        });
        if any_pair {
            "some share"
        } else {
            "none in common"
        }
    }
}

/// Whether `members` all sit at one floor and are mutually adjacent over
/// two-sided boundaries — `shapes.rs`'s and `lift::plat`'s split test.
fn floor_class(scene: &Scene, members: &BTreeSet<usize>) -> FloorClass {
    let floors: BTreeSet<i32> = members.iter().map(|&s| scene.sectors[s].floor).collect();
    if floors.len() > 1 {
        return FloorClass::SeveralFloors;
    }
    let seed = *members.iter().next().expect("a group has members");
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
    if reached.len() == members.len() {
        FloorClass::Split
    } else {
        FloorClass::OneFloorDisconnected
    }
}

/// A split group read as one lift: the union of the members' outside
/// neighbors is the merged platform's neighborhood, the members' shared line
/// set its triggers, and an activator that is itself a member is the
/// platform. `low` is the least member `low` (a sibling member at the same
/// floor never lowers it), and the rest classes follow `check::plats`'
/// definitions. The eight refusals then run in `lift::plat`'s order.
fn merged_verdict(scene: &Scene, plats: &[&ScenePlat], step: i32) -> Result<PlatShape, Refusal> {
    let members: BTreeSet<usize> = plats.iter().map(|p| p.sector).collect();
    let floor = scene.sectors[plats[0].sector].floor;
    let outside: BTreeSet<usize> = plats
        .iter()
        .flat_map(|p| p.neighbors.iter().copied())
        .filter(|n| !members.contains(n))
        .collect();
    let low = plats.iter().map(|p| p.low).min().unwrap_or(floor);
    let nb_floors: BTreeSet<i32> = outside.iter().map(|&n| scene.sectors[n].floor).collect();
    let max_nb = nb_floors.iter().copied().max().unwrap_or(floor);
    let rest = if low == floor {
        PlatRest::Dead
    } else if max_nb > floor + step {
        PlatRest::Intermediate
    } else if max_nb >= floor - step {
        PlatRest::Top
    } else {
        PlatRest::AboveAll
    };
    // Every member's trigger list is the same set of lines (they name one
    // tag); only the activator classes differ per member.
    let triggers = &plats[0].triggers;
    let low_from: BTreeSet<usize> = plats
        .iter()
        .flat_map(|p| p.triggers.iter())
        .flat_map(|t| t.activators.iter())
        .filter(|&&(s, a)| a == SceneActivator::Low && !members.contains(&s))
        .map(|&(s, _)| s)
        .collect();
    let one_floor = nb_floors.len() == 1;
    if rest == PlatRest::Dead {
        Err(Refusal::Dead)
    } else if !triggers.iter().all(|t| t.repeatable) {
        Err(Refusal::OneShot)
    } else if speed_of(triggers) == Speed::Mixed {
        Err(Refusal::MixedSpeed)
    } else if rest == PlatRest::Intermediate || (rest == PlatRest::AboveAll && !one_floor) {
        Err(Refusal::UnsupportedRest)
    } else if low_from.is_empty() {
        Err(Refusal::TopOnly)
    } else if rest == PlatRest::AboveAll
        && outside.len() >= 2
        && low_from.iter().filter(|s| outside.contains(s)).count() < outside.len()
    {
        Err(Refusal::OneWayBarrier)
    } else if !plats[0].other_actions.is_empty() {
        Err(Refusal::ConflictingAction)
    } else {
        Ok(shape_of(rest, outside.len()))
    }
}

/// The map-level lift axis under each §I column.
///
/// Every column is a **relaxation** of *today*, never a replacement for it:
/// a group counts when the column's gate accepts it **or** when the shipped
/// recognizer already does ([`BankVerdict::fold_group`]). The gates are not
/// each a superset of the recognizer on their own — gate A wants a Low line
/// on every member's own face where the recognizer needs only some adjacent
/// Low activator, and the split reading is false for a group that is not
/// split — so without that clause a column could *drop* a map the `today`
/// column counts, and the §I yield would not be a yield.
#[derive(Clone, Copy, Default)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is one §I column's verdict on the map; the columns are reported \
              side by side and never combined into a state"
)]
struct BankVerdict {
    /// No broken lift line, every single-tag platform accepted by the
    /// recognizer, every group accepted by gate A or by the recognizer.
    a: bool,
    /// The same with gate A′.
    a_prime: bool,
    /// The same with gate B.
    b: bool,
    /// The same with the split reading.
    split: bool,
}

impl BankVerdict {
    /// Folds one group into the verdict, `gates` being the five §I columns'
    /// verdicts on it (index 0 unused — the `today` column reads the
    /// recognizer directly) and `accepted_today` whether the shipped
    /// recognizer accepts every member of the group.
    ///
    /// Each column keeps the map only while every group clears its gate *or*
    /// is already accepted today, which makes the columns supersets of
    /// `today` by construction. Gate B is monotone on its own; the clause is
    /// applied to it too so that the four fold alike.
    fn fold_group(&mut self, gates: [bool; 5], accepted_today: bool) {
        self.a &= gates[1] || accepted_today;
        self.a_prime &= gates[2] || accepted_today;
        self.b &= gates[3] || accepted_today;
        self.split &= gates[4] || accepted_today;
    }
}

/// Builds one group's facts from its resolved members.
fn analyze_group(scene: &Scene, plats: &[&ScenePlat], step: i32) -> Group {
    let members_set: BTreeSet<usize> = plats.iter().map(|p| p.sector).collect();
    let members: Vec<Member> = plats
        .iter()
        .map(|p| {
            let reach = |t: &SceneTrigger| Reach::of(t, p.sector, &p.neighbors);
            Member {
                verdict: unshared_verdict(p),
                own_line: p.triggers.iter().any(|t| reach(t) == Reach::OnFace),
                callable_low: p.callable_low(),
                own_low_line: p.triggers.iter().any(|t| {
                    reach(t) == Reach::OnFace
                        && t.activators.iter().any(|&(_, a)| a == SceneActivator::Low)
                }),
                neighbor_low_line: p.triggers.iter().any(|t| {
                    t.activators
                        .iter()
                        .any(|&(s, _)| p.neighbors.contains(&s) && scene.sectors[s].floor == p.low)
                }),
                within_one_hop: p.triggers.iter().any(|t| reach(t) != Reach::Remote),
                rest: p.rest,
                travel: p.travel,
                low: p.low,
                outside_neighbors: p
                    .neighbors
                    .iter()
                    .copied()
                    .filter(|n| !members_set.contains(n))
                    .collect(),
            }
        })
        .collect();
    let triggers = &plats[0].triggers;
    let line_reach: Vec<Reach> = triggers
        .iter()
        .map(|t| {
            plats
                .iter()
                .map(|p| Reach::of(t, p.sector, &p.neighbors))
                .min()
                .unwrap_or(Reach::Remote)
        })
        .collect();
    let floor_class = floor_class(scene, &members_set);
    Group {
        members,
        line_reach,
        one_form: triggers.iter().all(|t| t.use_line == triggers[0].use_line),
        one_speed: triggers.iter().all(|t| t.fast == triggers[0].fast),
        floor_class,
        merged: (floor_class == FloorClass::Split).then(|| merged_verdict(scene, plats, step)),
    }
}

/// §I — every lift tag group of the map, and the map's bank verdicts.
fn survey_banks(v: &VarCtx<'_>, agg: &mut Agg) -> BankVerdict {
    let (scene, tables) = (v.ctx.scene, v.tables);
    let report = lift::plat::recognize(scene, tables);
    let resolved = resolve_plats(scene, tables);
    agg.bank_caller_refusals += report.counts.bank_caller;
    agg.historical_shared_members += count_len(
        resolved
            .iter()
            .filter(|p| p.shared_tag >= 2 && p.rest != PlatRest::Dead)
            .count(),
    );
    agg.recognizer_split += report.counts.shared_split;
    let recognizer_refusal = |sector: usize| {
        report
            .plats
            .iter()
            .find(|r| r.sector == sector)
            .and_then(|r| r.refusal)
    };
    // The re-derivation must agree with the recognizer wherever the tag is
    // unshared; a disagreement here would mean the eight refusals drifted.
    for p in &resolved {
        if p.shared_tag == 1 {
            agg.unshared_mismatch +=
                u64::from(unshared_verdict(p).err() != recognizer_refusal(p.sector));
        }
    }
    // A group member's own refusal is the group gate's business: a member
    // refused `Dead` (precedence 1, ahead of `BankCaller`) is still a trim
    // piece the split reading can absorb.
    let singles_ok = report.broken_lines.is_empty()
        && resolved
            .iter()
            .filter(|p| p.shared_tag == 1)
            .all(|p| recognizer_refusal(p.sector).is_none());
    let mut by_tag: BTreeMap<i32, Vec<&ScenePlat>> = BTreeMap::new();
    for p in &resolved {
        if p.shared_tag >= 2 {
            by_tag.entry(p.tag).or_default().push(p);
        }
    }
    let mut verdict = BankVerdict {
        a: singles_ok,
        a_prime: singles_ok,
        b: singles_ok,
        split: singles_ok,
    };
    for plats in by_tag.values() {
        let g = analyze_group(scene, plats, v.ctx.step);
        // Recovery is counted against the *historical* population — every
        // non-`Dead` member of the group, which is what `Refusal::SharedTag`
        // refused on sight before the bank construct — not against the
        // shipped recognizer's much smaller `bank_caller` count. The two
        // measure different things: what a gate would win back from the
        // pre-construct recognizer, and what this branch's recognizer still
        // refuses.
        let historical_members = plats.iter().filter(|p| p.rest != PlatRest::Dead).count();
        let gates = [
            false,
            g.gate_a(),
            g.gate_a_prime(),
            g.gate_b(),
            g.gate_split(),
        ];
        // `groups` and `recovered` count what the gate itself accepts, as §I's
        // definitions read them; only the map-level axis is relaxed.
        for (i, pass) in gates.into_iter().enumerate() {
            if pass {
                agg.bank_columns[i].groups += 1;
                agg.bank_columns[i].recovered += count_len(historical_members);
            }
        }
        let accepted_today = plats.iter().all(|p| recognizer_refusal(p.sector).is_none());
        verdict.fold_group(gates, accepted_today);
        record_group(&g, agg);
    }
    verdict
}

fn record_group(g: &Group, agg: &mut Agg) {
    agg.groups_n += 1;
    agg.groups_size.add(match g.members.len() {
        2 => "2",
        3 => "3",
        4 => "4",
        _ => "5+",
    });
    agg.groups_floor_class.add(g.floor_class.label());
    for m in &g.members {
        agg.members_n += 1;
        agg.member_verdict.add(verdict_label(m.verdict));
        agg.member_own_line += u64::from(m.own_line);
        agg.member_callable_low += u64::from(m.callable_low);
        agg.member_own_low_line += u64::from(m.own_low_line);
        agg.member_neighbor_low_line += u64::from(m.neighbor_low_line);
        agg.member_within_one_hop += u64::from(m.within_one_hop);
    }
    agg.group_neighbor_called
        .add(g.split_label(|m| m.neighbor_low_line));
    agg.common_neighbor_all_a_prime += u64::from(
        g.common_neighbor() == "all share one neighbor"
            && g.members.iter().all(|m| m.neighbor_low_line),
    );
    agg.group_accept.add(g.split_label(|m| m.verdict.is_ok()));
    agg.group_composition.add(g.composition());
    agg.group_self_callable
        .add(g.split_label(|m| m.own_low_line));
    agg.group_callable_low
        .add(g.split_label(|m| m.callable_low));
    for &r in &g.line_reach {
        agg.line_reach.add(r.label());
    }
    agg.group_line_reach.add(g.line_reach_label());
    agg.group_lines_n.add(match g.line_reach.len() {
        1 => "1",
        2 => "2",
        _ => "3+",
    });
    agg.group_one_form += u64::from(g.one_form);
    agg.group_one_speed += u64::from(g.one_speed);
    let uniform =
        |key: &dyn Fn(&Member) -> i64| g.members.iter().all(|m| key(m) == key(&g.members[0]));
    agg.uniform_rest += u64::from(uniform(&|m| m.rest as i64));
    agg.uniform_travel += u64::from(uniform(&|m| i64::from(m.travel)));
    agg.uniform_low += u64::from(uniform(&|m| i64::from(m.low)));
    agg.uniform_neighbors += u64::from(uniform(&|m| {
        i64::try_from(m.outside_neighbors.len()).expect("a count fits i64")
    }));
    agg.common_neighbor.add(g.common_neighbor());
}

fn report_banks(agg: &Agg) {
    println!("\n## I. Lift tag groups (banks)\n");
    println!(
        "- groups (a lift tag naming ≥2 sectors): {} · size: {}",
        agg.groups_n,
        agg.groups_size.all()
    );
    println!("- floor class: {}", agg.groups_floor_class.all());
    println!(
        "- arbiters: `bank_caller` refusals (recognizer): {} · `shared_split` groups (recognizer): {} · single-tag platforms where the re-derived verdict disagrees with the recognizer: {}",
        agg.bank_caller_refusals, agg.recognizer_split, agg.unshared_mismatch
    );
    println!(
        "\n**Members judged as if the tag were unshared** ({} members).\n",
        agg.members_n
    );
    println!("- member verdict: {}", agg.member_verdict.all());
    println!(
        "- groups with all / some / none of their members accepted: {}",
        agg.group_accept.all()
    );
    println!("- group composition: {}", agg.group_composition.all());
    println!("\n**Who calls the members.**\n");
    println!(
        "- members with a lift line on their own boundary: {} ({}) · callable from Low by any line: {} ({}) · with a Low-activator line on their own face: {} ({}) · with a lift line within one hop: {} ({})",
        agg.member_own_line,
        pct(agg.member_own_line, agg.members_n),
        agg.member_callable_low,
        pct(agg.member_callable_low, agg.members_n),
        agg.member_own_low_line,
        pct(agg.member_own_low_line, agg.members_n),
        agg.member_within_one_hop,
        pct(agg.member_within_one_hop, agg.members_n)
    );
    println!(
        "- members called from a two-sided neighbor standing at their own low (A′): {} ({})",
        agg.member_neighbor_low_line,
        pct(agg.member_neighbor_low_line, agg.members_n)
    );
    println!(
        "- groups where all / some / none of the members are callable from Low: {} · have a Low line on their own face: {} · are neighbor-called (A′): {}",
        agg.group_callable_low.all(),
        agg.group_self_callable.all(),
        agg.group_neighbor_called.all()
    );
    println!(
        "- lift lines by reach to the group: {}",
        agg.line_reach.all()
    );
    println!("- groups by line reach: {}", agg.group_line_reach.all());
    println!(
        "- distinct lift lines per group: {} · all lines one form (use or walkover): {} ({}) · one speed: {} ({})",
        agg.group_lines_n.all(),
        agg.group_one_form,
        pct(agg.group_one_form, agg.groups_n),
        agg.group_one_speed,
        pct(agg.group_one_speed, agg.groups_n)
    );
    println!("\n**Uniformity.**\n");
    println!(
        "- groups whose members all share one rest class: {} ({}) · one travel: {} ({}) · one low floor: {} ({}) · one outside-neighbor count: {} ({})",
        agg.uniform_rest,
        pct(agg.uniform_rest, agg.groups_n),
        agg.uniform_travel,
        pct(agg.uniform_travel, agg.groups_n),
        agg.uniform_low,
        pct(agg.uniform_low, agg.groups_n),
        agg.uniform_neighbors,
        pct(agg.uniform_neighbors, agg.groups_n)
    );
    println!(
        "- common outside neighbor: {} · of the \"all share one neighbor\" groups, all members neighbor-called (A′): {}",
        agg.common_neighbor.all(),
        agg.common_neighbor_all_a_prime
    );
    println!(
        "\n**Yield.** Line axis unchanged at {} ({}). Per column: honest all-axes maps, \
historical `SharedTag` refusals recovered (every non-`Dead` bank member, the population the \
recognizer refused on sight before the bank construct — not its `bank_caller` count), groups \
accepted. A = every member passes alone and \
has a Low-activator lift line on its own face; A′ = every member passes alone and is called from \
a two-sided neighbor standing at its own low; B = every member passes alone (callers ignored, \
the floor recognizer's rule); split = a one-floor, mutually adjacent group read as one lift. \
A column counts a map when every bank group is accepted by its gate **or** by the shipped \
recognizer, so each column is a superset of today; recovered and groups count what the gate \
itself accepts.\n",
        agg.line_today,
        pct(agg.line_today, agg.maps)
    );
    for (i, name) in BANK_COLUMNS.iter().enumerate() {
        let c = &agg.bank_columns[i];
        println!(
            "- {name}: all axes {} ({}) · recovered {} of {} · groups {} of {}",
            c.all_honest,
            pct(c.all_honest, agg.maps),
            c.recovered,
            agg.historical_shared_members,
            c.groups,
            agg.groups_n
        );
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
    report_banks(agg);
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
        "- perpetual plats (sectors named by a 53/87 tag): {} · maps with ≥1: {} ({})",
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
        "- most sectors one 53/87 tag names, per map with such a tag (0-15 / 16-30 / 31+): {} · max: {} · maps with a single 53/87 tag naming > {}: {}",
        agg.perpetual_tag_max.all(),
        agg.perpetual_tag_max_n,
        agg.max_plats,
        agg.maps_perpetual_tag_over_30
    );
    println!(
        "- most sectors one DWUS/blaze tag names, per map with such a tag: {} · max: {} · maps with a single lift tag naming > {}: {}",
        agg.lift_tag_max.all(),
        agg.lift_tag_max_n,
        agg.max_plats,
        agg.maps_lift_tag_over_30
    );
    println!(
        "- maps with ≥1 perpetual plat where perpetual ∪ moving plats (an upper bound on concurrency) > {}: {} · > {}: {} · max combined among them: {}",
        agg.max_plats / 2,
        agg.combined_over_15,
        agg.max_plats,
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
        "- perpetual plats passing the provisional gate (its map-level clause included, so \
none in a map with a tag-0 or dangling start line): {} of {} ({})",
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
    println!("\n## J. Not measured\n");
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
            p1_sector: player1_sector(&f.scene, tables),
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

    /// The §H columns admit exactly the named arrays: no column carries a
    /// literal that could drift from `START`, `PERPETUAL` or `ONE_SHOT_LIFT`.
    #[test]
    fn the_arbiter_columns_are_slices_of_the_named_arrays() {
        assert_eq!(COLUMNS[0].1, &[] as &[i32]);
        assert_eq!(COLUMNS[1].1, &START[..]);
        assert_eq!(COLUMNS[2].1, &PERPETUAL[..]);
        assert_eq!(COLUMNS[3].1, &ONE_SHOT_LIFT[..]);
        let all: Vec<i32> = PERPETUAL
            .iter()
            .chain(ONE_SHOT_LIFT.iter())
            .copied()
            .collect();
        assert_eq!(COLUMNS[4].1, &all[..]);
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

    /// An `Agg` with its column vectors sized, as `run` builds it.
    fn agg_with_columns() -> Agg {
        Agg {
            columns: (0..COLUMNS.len())
                .map(|_| ArbiterColumn::default())
                .collect(),
            bank_columns: (0..BANK_COLUMNS.len())
                .map(|_| BankColumn::default())
                .collect(),
            ..Agg::default()
        }
    }

    #[test]
    fn the_arbiter_columns_admit_exactly_their_specials() {
        // A map whose only out-of-set special is 87 clears the two
        // perpetual columns and the all-eight column, but not the one-shot
        // one; with a stop line it needs the four-special column.
        let plats: Vec<PerpetualFacts> = Vec::new();
        let mut agg = agg_with_columns();
        let a = MapArbiter {
            unknown: vec![87],
            others_ok: true,
            floors_ok: true,
            lifts_today: true,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, false, BankVerdict::default(), &mut agg);
        let lines: Vec<u64> = agg.columns.iter().map(|c| c.line).collect();
        assert_eq!(lines, vec![0, 1, 1, 0, 1]);
        assert_eq!(agg.line_today, 0);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 1, 1, 0, 1]);

        // The same map with a tag-0 start line fails the provisional gate
        // on every perpetual column but keeps the line axis.
        let mut agg = agg_with_columns();
        record_arbiter(&a, &plats, true, BankVerdict::default(), &mut agg);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 0, 0, 0, 0]);
        let all_today: Vec<u64> = agg.columns.iter().map(|c| c.all_today).collect();
        assert_eq!(all_today, vec![0, 1, 1, 0, 1]);

        // A one-shot map: the recognizer refuses it today and accepts the
        // twin, so only the one-shot columns' provisional count moves.
        let mut agg = agg_with_columns();
        let a = MapArbiter {
            unknown: vec![21, 54],
            others_ok: true,
            floors_ok: true,
            lifts_today: false,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, false, BankVerdict::default(), &mut agg);
        let lines: Vec<u64> = agg.columns.iter().map(|c| c.line).collect();
        assert_eq!(lines, vec![0, 0, 0, 0, 1]);
        let all_today: Vec<u64> = agg.columns.iter().map(|c| c.all_today).collect();
        assert_eq!(all_today, vec![0, 0, 0, 0, 0]);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 0, 0, 0, 1]);

        // An expressible map today counts on every row and column.
        let mut agg = agg_with_columns();
        let a = MapArbiter {
            unknown: vec![],
            others_ok: true,
            floors_ok: false,
            lifts_today: true,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, false, BankVerdict::default(), &mut agg);
        assert_eq!(
            (agg.line_today, agg.all_floors_ignored, agg.all_honest),
            (1, 1, 0)
        );
        assert!(agg.columns.iter().all(|c| c.line == 1 && c.all_today == 0));
    }

    #[test]
    fn a_broken_start_line_fails_the_provisional_gate_for_every_plat_of_the_map() {
        let tables = Tables::load().expect("tables");
        // One gate-clean perpetual plat (tag 7 on sector 1, resting at high,
        // no stop line, no other family) and, elsewhere in the map, a 53 line
        // naming tag 99 — dangling. The gate's map-level clause fails, so no
        // plat of this map passes it.
        let mut text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 7, false), (0, 0, false)],
            "",
        );
        far_wall(&mut text, 3, 53, 99);
        let f = fixture(&text);
        let v = var_ctx(&f, &tables);
        let mut agg = agg_with_columns();
        let bad_start = survey_start_lines(&v, &mut agg);
        assert!(bad_start);
        assert_eq!(agg.start_dangling, 1);
        let plats: Vec<PerpetualFacts> = perpetual_plats(&v.ctx)
            .into_iter()
            .map(|p| analyze_perpetual(&v, p))
            .collect();
        assert_eq!(plats.len(), 1);
        assert!(
            plats[0].provisional(),
            "the plat clears all five per-plat clauses"
        );
        let a = MapArbiter {
            unknown: vec![53, 87],
            others_ok: true,
            floors_ok: true,
            lifts_today: true,
            lifts_twin: true,
        };
        record_arbiter(&a, &plats, bad_start, BankVerdict::default(), &mut agg);
        assert_eq!(
            agg.provisional_plats, 0,
            "the map-level clause is part of the gate the plat count reports"
        );
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 0, 0, 0, 0]);

        // The same plat in a map with no broken start line does pass.
        let mut agg = agg_with_columns();
        record_arbiter(&a, &plats, false, BankVerdict::default(), &mut agg);
        assert_eq!(agg.provisional_plats, 1);
        let provisional: Vec<u64> = agg.columns.iter().map(|c| c.provisional).collect();
        assert_eq!(provisional, vec![0, 1, 1, 0, 1]);
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
        let sectors: BTreeSet<usize> = plats.iter().map(|p| p.sector).collect();
        let moving = survey_concurrency(&v, &sectors, &mut agg);
        assert_eq!(moving, 1);
        assert_eq!(agg.perpetual_per_map.all(), "1: 1");
        assert_eq!(agg.moving_per_map.all(), "1: 1");
        assert_eq!((agg.combined_max, agg.combined_over_15), (2, 0));
        assert_eq!(agg.perpetual_tag_max.all(), "0-15: 1");
        assert_eq!(agg.lift_tag_max.all(), "0-15: 1");
        assert_eq!((agg.perpetual_tag_max_n, agg.lift_tag_max_n), (1, 1));

        // A sector whose tag carries both an 87 and a 62 line is one
        // thinker slot, not two: the union counts it once.
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(87, 7, false), (62, 7, false)],
            "",
        ));
        let v = var_ctx(&f, &tables);
        let mut agg = Agg::default();
        let sectors: BTreeSet<usize> = perpetual_plats(&v.ctx).iter().map(|p| p.sector).collect();
        assert_eq!(survey_concurrency(&v, &sectors, &mut agg), 1);
        assert_eq!(agg.combined_max, 1, "the shared sector counts once");
        assert_eq!(tag_size_bucket(16), "16-30");
        assert_eq!(tag_size_bucket(31), "31+");

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

    /// The bank verdict and the aggregate of one fixture's tag groups.
    fn banks_of(f: &Fixture, tables: &Tables) -> (BankVerdict, Agg) {
        let v = var_ctx(f, tables);
        let mut agg = agg_with_columns();
        let verdict = survey_banks(&v, &mut agg);
        (verdict, agg)
    }

    // Two Core lifts on one tag, each with its riser switch on its own low
    // face: A(0)|T1(128)|B(128)|C(0)|T2(128)|D(128).
    const BANK_FLOORS: [i32; 6] = [0, 128, 128, 0, 128, 128];
    const BANK_TAGS: [i32; 6] = [0, 7, 0, 0, 7, 0];

    #[test]
    fn the_unshared_verdict_agrees_with_the_recognizer_on_a_single_tag() {
        let tables = Tables::load().expect("tables");
        for (links, expected) in [
            ([(62, 7, false), (0, 0, false)], Ok(PlatShape::Lift)),
            ([(62, 7, true), (0, 0, false)], Err(Refusal::TopOnly)),
            ([(21, 7, false), (0, 0, false)], Err(Refusal::OneShot)),
            ([(62, 7, false), (120, 7, false)], Err(Refusal::MixedSpeed)),
        ] {
            let f = fixture(&chain(&[0, 128, 128], &[0, 7, 0], &links, ""));
            let resolved = resolve_plats(&f.scene, &tables);
            assert_eq!(resolved.len(), 1);
            assert_eq!(unshared_verdict(&resolved[0]), expected, "{links:?}");
            let report = lift::plat::recognize(&f.scene, &tables);
            assert_eq!(report.plats[0].refusal, expected.err());
            assert_eq!(report.plats[0].shape, expected.ok());
        }
        // The dead and the intermediate rests, and the shape rule.
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            unshared_verdict(&resolve_plats(&f.scene, &tables)[0]),
            Err(Refusal::Dead)
        );
        let f = fixture(&chain(&[0, 128], &[0, 7], &[(62, 7, false)], ""));
        assert_eq!(
            unshared_verdict(&resolve_plats(&f.scene, &tables)[0]),
            Ok(PlatShape::Pedestal)
        );
        assert_eq!(shape_of(PlatRest::AboveAll, 3), PlatShape::Barrier);
        assert_eq!(verdict_label(Ok(PlatShape::Lift)), "Lift");
        assert_eq!(verdict_label(Err(Refusal::Dead)), "refused: Dead");
    }

    #[test]
    fn a_bank_of_two_self_called_lifts_passes_every_gate_but_split() {
        let tables = Tables::load().expect("tables");
        let f = fixture(&chain(
            &BANK_FLOORS,
            &BANK_TAGS,
            &[
                (62, 7, false),
                (0, 0, false),
                (0, 0, false),
                (62, 7, false),
                (0, 0, false),
            ],
            "",
        ));
        // Both members carry the tag; the recognizer no longer refuses a
        // member for that alone.
        let resolved = resolve_plats(&f.scene, &tables);
        assert!(resolved.iter().all(|p| p.shared_tag >= 2));
        let (verdict, agg) = banks_of(&f, &tables);
        // Which gates accept this group is pinned by `bank_columns[..].groups`
        // below: A, A′ and B do, `split` does not (the members do not touch).
        // The map-level verdict is a *relaxation* of today rather than a
        // replacement for it, so the split column keeps the map anyway — the
        // bank-aware recognizer accepts both members, and an accepted group
        // carries its map past the gate it fails.
        assert!(verdict.a && verdict.a_prime && verdict.b && verdict.split);
        assert_eq!(agg.groups_n, 1);
        assert_eq!(agg.groups_size.all(), "2: 1");
        assert_eq!(agg.groups_floor_class.all(), "one floor, disconnected: 1");
        assert_eq!(agg.member_verdict.all(), "Lift: 2");
        assert_eq!(agg.group_composition.all(), "all Lift: 1");
        assert_eq!(agg.group_accept.all(), "all: 1");
        assert_eq!(
            (
                agg.member_own_line,
                agg.member_callable_low,
                agg.member_own_low_line,
                agg.member_within_one_hop
            ),
            (2, 2, 2, 2)
        );
        assert_eq!(agg.group_self_callable.all(), "all: 1");
        assert_eq!(agg.line_reach.all(), "on a member face: 2");
        assert_eq!(agg.group_line_reach.all(), "all on member faces: 1");
        assert_eq!(agg.group_lines_n.all(), "2: 1");
        assert_eq!((agg.group_one_form, agg.group_one_speed), (1, 1));
        assert_eq!(
            (
                agg.uniform_rest,
                agg.uniform_travel,
                agg.uniform_low,
                agg.uniform_neighbors
            ),
            (1, 1, 1, 1)
        );
        assert_eq!(agg.common_neighbor.all(), "none in common: 1");
        // Each member's own line calls it, so the recognizer refuses
        // neither `BankCaller`: `bank_caller_refusals` is 0, where the old
        // blanket `SharedTag` refusal counted the pair. That pair is still
        // the historical population every column's recovery is measured
        // against, so both members are recovered by every column that
        // accepts the group.
        assert_eq!(
            (
                agg.bank_caller_refusals,
                agg.historical_shared_members,
                agg.recognizer_split,
                agg.unshared_mismatch
            ),
            (0, 2, 0, 0)
        );
        let recovered: Vec<u64> = agg.bank_columns.iter().map(|c| c.recovered).collect();
        assert_eq!(recovered, vec![0, 2, 2, 2, 0]);
        let groups: Vec<u64> = agg.bank_columns.iter().map(|c| c.groups).collect();
        assert_eq!(groups, vec![0, 1, 1, 1, 0]);
        assert_eq!(
            groups[2], 1,
            "each riser switch fires from the member's own low room, so gate A′ accepts"
        );
    }

    #[test]
    fn the_bank_columns_relax_today_rather_than_replacing_it() {
        let tables = Tables::load().expect("tables");
        let step = tables.step_height();
        // The row-of-pedestals witness: P1(96) | H(0) | P2(96) on tag 7, the
        // only switch on P1's face. Gate A wants a Low line on every member's
        // own face and the group is not split, so A and split refuse it while
        // A′ and B accept — the recognizer, which asks only for some adjacent
        // Low activator, is stricter on neither count.
        let f = fixture(&chain(
            &[96, 0, 96],
            &[7, 0, 7],
            &[(62, 7, true), (0, 0, false)],
            "",
        ));
        let resolved = resolve_plats(&f.scene, &tables);
        let members: Vec<&ScenePlat> = resolved.iter().filter(|p| p.shared_tag >= 2).collect();
        assert_eq!(members.len(), 2);
        let g = analyze_group(&f.scene, &members, step);
        let gates = [
            false,
            g.gate_a(),
            g.gate_a_prime(),
            g.gate_b(),
            g.gate_split(),
        ];
        assert_eq!(gates, [false, false, true, true, false]);

        // A group the recognizer accepts stays in every column, gates or no
        // gates: the columns are relaxations of today, not replacements.
        let mut verdict = BankVerdict {
            a: true,
            a_prime: true,
            b: true,
            split: true,
        };
        verdict.fold_group(gates, true);
        assert!(verdict.a && verdict.a_prime && verdict.b && verdict.split);
        let a = MapArbiter {
            unknown: Vec::new(),
            others_ok: true,
            floors_ok: true,
            lifts_today: true,
            lifts_twin: true,
        };
        let mut agg = agg_with_columns();
        record_arbiter(&a, &[], false, verdict, &mut agg);
        let all_honest: Vec<u64> = agg.bank_columns.iter().map(|c| c.all_honest).collect();
        assert_eq!(
            all_honest,
            vec![1, 1, 1, 1, 1],
            "+A and +split count the map exactly as today does"
        );

        // Only a group today refuses is judged by the gate, and only then may
        // a column withhold the map.
        let mut verdict = BankVerdict {
            a: true,
            a_prime: true,
            b: true,
            split: true,
        };
        verdict.fold_group(gates, false);
        assert!(!verdict.a && verdict.a_prime && verdict.b && !verdict.split);

        // The self-called bank witness (two Core lifts, each with its riser
        // switch on its own low face) clears A, A′ and B and fails split; the
        // relaxation keeps it in the split column when today accepts it.
        let f = fixture(&chain(
            &BANK_FLOORS,
            &BANK_TAGS,
            &[
                (62, 7, false),
                (0, 0, false),
                (0, 0, false),
                (62, 7, false),
                (0, 0, false),
            ],
            "",
        ));
        let resolved = resolve_plats(&f.scene, &tables);
        let members: Vec<&ScenePlat> = resolved.iter().filter(|p| p.shared_tag >= 2).collect();
        let g = analyze_group(&f.scene, &members, step);
        let gates = [
            false,
            g.gate_a(),
            g.gate_a_prime(),
            g.gate_b(),
            g.gate_split(),
        ];
        assert_eq!(gates, [false, true, true, true, false]);
        let mut verdict = BankVerdict {
            a: true,
            a_prime: true,
            b: true,
            split: true,
        };
        verdict.fold_group(gates, true);
        assert!(verdict.split, "today's acceptance carries the split column");
    }

    #[test]
    fn a_bank_called_from_beside_its_members_passes_b_but_not_a() {
        let tables = Tables::load().expect("tables");
        // One switch across the B|C threshold, fronted by C (floor 0): Low
        // for both members and adjacent to both, on neither's face.
        let f = fixture(&chain(
            &BANK_FLOORS,
            &BANK_TAGS,
            &[
                (0, 0, false),
                (0, 0, false),
                (62, 7, true),
                (0, 0, false),
                (0, 0, false),
            ],
            "",
        ));
        let (verdict, agg) = banks_of(&f, &tables);
        assert!(!verdict.a && verdict.b);
        assert_eq!(agg.member_verdict.all(), "Lift: 2");
        assert_eq!(
            (
                agg.member_own_line,
                agg.member_callable_low,
                agg.member_own_low_line
            ),
            (0, 2, 0)
        );
        assert_eq!(agg.member_within_one_hop, 2);
        assert_eq!(agg.group_callable_low.all(), "all: 1");
        assert_eq!(agg.group_self_callable.all(), "none: 1");
        assert_eq!(agg.line_reach.all(), "adjacent to a member: 1");
        assert_eq!(agg.group_line_reach.all(), "mixed: 1");
        assert_eq!(agg.group_lines_n.all(), "1: 1");
        // B and C are shared outside neighbors of nobody: T1 sees A and B,
        // T2 sees C and D.
        assert_eq!(agg.common_neighbor.all(), "none in common: 1");
        let groups: Vec<u64> = agg.bank_columns.iter().map(|c| c.groups).collect();
        assert_eq!(groups, vec![0, 0, 0, 1, 0]);
        // C is at both members' low (0) but is a neighbor of T2 only, and
        // T1's low room A fires nothing: A′ fails for T1.
        assert!(!verdict.a_prime);
        assert_eq!(agg.member_neighbor_low_line, 1);
        assert_eq!(agg.group_neighbor_called.all(), "some: 1");
    }

    #[test]
    fn a_member_refused_alone_refuses_the_group_and_names_its_reason() {
        let tables = Tables::load().expect("tables");
        // The second lift's switch faces the plat: use from C hits its back.
        let f = fixture(&chain(
            &BANK_FLOORS,
            &BANK_TAGS,
            &[
                (62, 7, false),
                (0, 0, false),
                (0, 0, false),
                (62, 7, true),
                (0, 0, false),
            ],
            "",
        ));
        let (verdict, agg) = banks_of(&f, &tables);
        // Every line names the tag, so T2 is still callable from Low by
        // T1's riser switch (A is Low for T2 too) and passes alone; but its
        // own face line fires only from the plat, so gate A refuses it.
        assert!(!verdict.a && verdict.b && !verdict.split);
        assert_eq!(agg.member_verdict.all(), "Lift: 2");
        assert_eq!(agg.group_self_callable.all(), "some: 1");
        assert_eq!(agg.line_reach.all(), "on a member face: 2");

        // Flip T1's switch as well: no line fires from Low for anyone, and
        // both members are refused `TopOnly` alone.
        let f = fixture(&chain(
            &BANK_FLOORS,
            &BANK_TAGS,
            &[
                (62, 7, true),
                (0, 0, false),
                (0, 0, false),
                (62, 7, true),
                (0, 0, false),
            ],
            "",
        ));
        let (verdict, agg) = banks_of(&f, &tables);
        assert!(!verdict.a && !verdict.b && !verdict.split);
        assert_eq!(agg.member_verdict.all(), "refused: TopOnly: 2");
        assert_eq!(agg.group_accept.all(), "none: 1");
        assert_eq!(agg.group_composition.all(), "any refused: 1");
    }

    #[test]
    fn a_split_group_reads_as_one_lift_where_its_members_do_not() {
        let tables = Tables::load().expect("tables");
        // A(0)|T1(128)|T2(128)|B(128), one tag on T1 and T2, the riser
        // switch on A|T1. T2 alone has nothing below it: dead. Merged, the
        // pair is a plain Core lift called from A.
        let f = fixture(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        let (verdict, agg) = banks_of(&f, &tables);
        assert!(!verdict.a && !verdict.b && verdict.split);
        assert_eq!(
            agg.groups_floor_class.all(),
            "one floor, mutually adjacent (split): 1"
        );
        assert_eq!(agg.recognizer_split, 1);
        assert_eq!(agg.member_verdict.all(), "Lift: 1 · refused: Dead: 1");
        assert_eq!(agg.group_accept.all(), "some: 1");
        assert_eq!(agg.group_composition.all(), "any refused: 1");
        // The dead member reports `Dead`; T1's own neighbor (A) calls it, so
        // neither member is `BankCaller`. `Dead` took precedence over the old
        // `SharedTag` too, so the historical population is T1 alone — the one
        // platform the split column recovers.
        assert_eq!(
            (agg.bank_caller_refusals, agg.historical_shared_members),
            (0, 1)
        );
        let recovered: Vec<u64> = agg.bank_columns.iter().map(|c| c.recovered).collect();
        assert_eq!(recovered, vec![0, 0, 0, 0, 1]);
        let resolved = resolve_plats(&f.scene, &tables);
        let plats: Vec<&ScenePlat> = resolved.iter().collect();
        assert_eq!(
            merged_verdict(&f.scene, &plats, f.step),
            Ok(PlatShape::Lift)
        );

        // The same pair with the switch facing the plat: merged, nobody
        // outside can call it.
        let f = fixture(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, true), (0, 0, false), (0, 0, false)],
            "",
        ));
        let resolved = resolve_plats(&f.scene, &tables);
        let plats: Vec<&ScenePlat> = resolved.iter().collect();
        assert_eq!(
            merged_verdict(&f.scene, &plats, f.step),
            Err(Refusal::TopOnly)
        );
    }

    #[test]
    fn several_floors_and_reach_are_classified_as_shapes_rs_does() {
        let tables = Tables::load().expect("tables");
        let f = fixture(&chain(
            &[0, 128, 64, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            floor_class(&f.scene, &BTreeSet::from([1, 2])),
            FloorClass::SeveralFloors
        );
        let f = fixture(&chain(
            &[0, 128, 128, 128],
            &[0, 7, 7, 0],
            &[(62, 7, false), (0, 0, false), (0, 0, false)],
            "",
        ));
        assert_eq!(
            floor_class(&f.scene, &BTreeSet::from([1, 2])),
            FloorClass::Split
        );
        assert_eq!(
            floor_class(&f.scene, &BTreeSet::from([1, 3])),
            FloorClass::OneFloorDisconnected
        );
        let resolved = resolve_plats(&f.scene, &tables);
        let t = &resolved[0].triggers[0];
        assert_eq!(Reach::of(t, 1, &BTreeSet::from([0, 2])), Reach::OnFace);
        assert_eq!(Reach::of(t, 2, &BTreeSet::from([1, 3])), Reach::Adjacent);
        assert_eq!(Reach::of(t, 3, &BTreeSet::from([2])), Reach::Remote);
        assert_eq!(Reach::Remote.label(), "remote from every member");
        assert_eq!(
            FloorClass::Split.label(),
            "one floor, mutually adjacent (split)"
        );
        let _ = tables;
    }

    #[test]
    fn group_labels_are_pure_functions_of_their_members() {
        let member = |verdict, own_low_line, nbs: &[usize]| Member {
            verdict,
            own_line: own_low_line,
            callable_low: true,
            own_low_line,
            neighbor_low_line: own_low_line,
            within_one_hop: true,
            rest: PlatRest::Top,
            travel: 128,
            low: 0,
            outside_neighbors: nbs.iter().copied().collect(),
        };
        let group = |members: Vec<Member>, reach: Vec<Reach>| Group {
            members,
            line_reach: reach,
            one_form: true,
            one_speed: true,
            floor_class: FloorClass::OneFloorDisconnected,
            merged: None,
        };
        let g = group(
            vec![
                member(Ok(PlatShape::Pedestal), true, &[5]),
                member(Ok(PlatShape::Pedestal), false, &[5]),
            ],
            vec![Reach::OnFace, Reach::Remote],
        );
        assert_eq!(g.composition(), "all Pedestal");
        assert!(g.gate_b() && !g.gate_a() && !g.gate_a_prime() && !g.gate_split());
        assert_eq!(g.split_label(|m| m.own_low_line), "some");
        assert_eq!(g.line_reach_label(), "mixed");
        assert_eq!(g.common_neighbor(), "all share one neighbor");

        let g = group(
            vec![
                member(Ok(PlatShape::Lift), true, &[1, 2]),
                member(Ok(PlatShape::Barrier), true, &[2, 3]),
                member(Ok(PlatShape::Lift), true, &[9]),
            ],
            vec![Reach::Remote],
        );
        assert_eq!(g.composition(), "mixed shapes");
        assert!(g.gate_a());
        assert_eq!(g.line_reach_label(), "all remote");
        assert_eq!(g.common_neighbor(), "some share");

        let g = group(
            vec![
                member(Err(Refusal::TopOnly), false, &[1]),
                member(Ok(PlatShape::Lift), true, &[2]),
            ],
            vec![Reach::OnFace],
        );
        assert_eq!(g.composition(), "any refused");
        assert!(!g.gate_b());
        assert_eq!(g.split_label(|m| m.verdict.is_ok()), "some");
        assert_eq!(g.split_label(|_| false), "none");
        assert_eq!(g.line_reach_label(), "all on member faces");
        assert_eq!(g.common_neighbor(), "none in common");

        let g = group(vec![], vec![]);
        assert_eq!(g.common_neighbor(), "none in common");
        assert_eq!(speed_of(&[]), Speed::Normal);
    }

    #[test]
    fn a_row_of_pedestals_in_one_room_is_neighbor_called_but_not_self_called() {
        let tables = Tables::load().expect("tables");
        // P1(96) | H(0) | P2(96): two pedestals on tag 7 in one host room,
        // the switch on P1's face fronted by H. P2 has no line of its own,
        // but H — its neighbor, at its low — fires the switch.
        let f = fixture(&chain(
            &[96, 0, 96],
            &[7, 0, 7],
            &[(62, 7, true), (0, 0, false)],
            "",
        ));
        let (verdict, agg) = banks_of(&f, &tables);
        assert_eq!(agg.member_verdict.all(), "Pedestal: 2");
        assert_eq!(agg.group_composition.all(), "all Pedestal: 1");
        // Gate A refuses this group (P2 has no Low line on its own face) and
        // so does split (the pedestals do not touch) — `bank_columns[..].groups`
        // below pins that. The map still counts in every column: the
        // bank-aware recognizer accepts both pedestals, which the columns'
        // relaxation clause carries past the gates they fail.
        assert!(verdict.a && verdict.a_prime && verdict.b && verdict.split);
        assert_eq!(
            (agg.member_own_low_line, agg.member_neighbor_low_line),
            (1, 2)
        );
        assert_eq!(agg.group_self_callable.all(), "some: 1");
        assert_eq!(agg.group_neighbor_called.all(), "all: 1");
        assert_eq!(agg.common_neighbor.all(), "all share one neighbor: 1");
        assert_eq!(agg.common_neighbor_all_a_prime, 1);
        let groups: Vec<u64> = agg.bank_columns.iter().map(|c| c.groups).collect();
        assert_eq!(groups, vec![0, 0, 1, 1, 0]);

        // P1(96) | H(0) | X(0) | P2(96): P2's only caller is H, two rooms
        // away. It passes alone (H is Low for it) but no neighbor at its low
        // fires anything: A′ fails, B passes.
        let f = fixture(&chain(
            &[96, 0, 0, 96],
            &[7, 0, 0, 7],
            &[(62, 7, true), (0, 0, false), (0, 0, false)],
            "",
        ));
        let (verdict, agg) = banks_of(&f, &tables);
        assert_eq!(agg.member_verdict.all(), "Pedestal: 2");
        assert!(!verdict.a && !verdict.a_prime && verdict.b);
        assert_eq!(agg.member_neighbor_low_line, 1);
        assert_eq!(agg.group_neighbor_called.all(), "some: 1");
        assert_eq!(agg.common_neighbor.all(), "none in common: 1");
        assert_eq!(agg.common_neighbor_all_a_prime, 0);
    }
}
