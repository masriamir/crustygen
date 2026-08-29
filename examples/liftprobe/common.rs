//! Shared loading and per-plat analysis for both probe passes.
//!
//! Every special value here is transcribed from the fetched pinned source
//! (`linuxdoom-1.10` at `a77dfb96`): the `case N:` labels in
//! `P_UseSpecialLine` (`p_switch.c`), `P_CrossSpecialLine` and
//! `P_ShootSpecialLine` (`p_spec.c`), and the `EV_DoPlat` / `EV_DoFloor` type
//! each dispatches to. The loader shares `crustygen-corpus`'s pieces
//! (`src/lift/corpus.rs`): the same lenient archive options, the same
//! `ingest::load_map` gate, and `map_hash` deduplication — so the population
//! reproduces the sweep's unique-map count — but it does not bucket failures:
//! an unreadable archive, WAD or map group is named on stderr and skipped, and
//! an unlistable directory is fatal (exit 2).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crustygen::check::scene::{Scene, SceneThing};
use crustygen::ingest;
use crustygen::lift::corpus::map_hash;
use crustywad::archive::Archive;
use crustywad::map::udmf::UdmfMap;
use crustywad::{Limits, ParseOptions, Strictness, Wad};

/// `downWaitUpStay`: `p_switch.c` case 62 (SR, `EV_DoPlat(line,downWaitUpStay,1)`)
/// and case 21 (S1); `p_spec.c` case 88 (WR, RETRIGGERS block) and case 10 (W1).
pub(crate) const DWUS: [i32; 4] = [62, 21, 88, 10];
/// `blazeDWUS`: `p_switch.c` case 123 (SR) and 122 (S1); `p_spec.c` 120 (WR) and 121 (W1).
pub(crate) const BLAZE: [i32; 4] = [123, 122, 120, 121];
/// The use-activated lift specials (`P_UseSpecialLine`, front side only).
pub(crate) const USE_LIFT: [i32; 4] = [62, 21, 123, 122];
/// The repeatable forms (`useAgain=1` / the RETRIGGERS block).
pub(crate) const REPEATABLE_LIFT: [i32; 4] = [62, 88, 123, 120];
/// `perpetualRaise`: `p_spec.c` 53 (W1), 87 (WR); `EV_StopPlat`: 54 (W1), 89 (WR).
pub(crate) const PERPETUAL: [i32; 4] = [53, 87, 54, 89];
/// `raiseAndChange`: `p_switch.c` 14 (S1, 32), 15 (S1, 24), 66 (SR, 24), 67 (SR, 32);
/// `raiseToNearestAndChange`: `p_switch.c` 20 (S1), 68 (SR); `p_spec.c` 22 (W1),
/// 95 (WR); `P_ShootSpecialLine` 47 (G1).
pub(crate) const RAISE_CHANGE: [i32; 9] = [14, 15, 66, 67, 20, 68, 22, 95, 47];
/// `EV_DoFloor(raiseFloorToNearest)`: `p_switch.c` 18 (S1), 69 (SR); `p_spec.c`
/// 119 (W1), 128 (WR); `raiseFloorTurbo`: `p_switch.c` 131 (S1), 132 (SR);
/// `p_spec.c` 130 (W1), 129 (WR).
pub(crate) const RAISE_NEAREST: [i32; 8] = [18, 69, 119, 128, 131, 132, 130, 129];
/// `lowerFloorToLowest`: `p_switch.c` 23 (S1), 60 (SR); `p_spec.c` 38 (W1), 82 (WR);
/// `lowerFloor`: `p_switch.c` 102 (S1), 45 (SR); `p_spec.c` 19 (W1), 83 (WR);
/// `turboLower`: `p_switch.c` 71 (S1), 70 (SR); `p_spec.c` 36 (W1), 98 (WR).
pub(crate) const LOWER: [i32; 12] = [23, 60, 38, 82, 102, 45, 19, 83, 71, 70, 36, 98];

/// Whether `special` dispatches to a `downWaitUpStay` or `blazeDWUS` plat.
pub(crate) fn is_lift(special: i32) -> bool {
    DWUS.contains(&special) || BLAZE.contains(&special)
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// The sidedef a linedef's `sidefront`/`sideback` names, or `None` when the
/// reference is negative or dangling — a UDMF map parses without
/// cross-reference validation, and a corrupt one must not abort a sweep.
pub(crate) fn sidedef(map: &UdmfMap, side: i32) -> Option<&crustywad::map::udmf::UdmfSidedef> {
    usize::try_from(side).ok().and_then(|i| map.sidedefs.get(i))
}

/// The sector behind a linedef's side, or `None` when the side or its sector
/// reference dangles.
pub(crate) fn side_sector(map: &UdmfMap, side: i32) -> Option<usize> {
    sidedef(map, side)
        .and_then(|sd| usize::try_from(sd.sector).ok())
        .filter(|&s| s < map.sectors.len())
}

/// Walks every `.zip`/`.wad` in `dirs` (non-recursively, like
/// `crustygen-corpus`) and calls `visit` once per unique loaded map. Returns
/// the unique-map count. Unreadable archives, WADs and map groups are named
/// on stderr and skipped.
pub(crate) fn sweep(dirs: &[String], mut visit: impl FnMut(&str, &UdmfMap)) -> u64 {
    let options = ParseOptions {
        strictness: Strictness::Lenient,
        limits: Limits::default(),
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut maps = 0;
    for dir in dirs {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                // A population that cannot be listed is not a partial result;
                // fail plainly, with the usage exit code, rather than panic.
                eprintln!("cannot list {dir}: {e}");
                std::process::exit(2);
            }
        };
        let mut candidates: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_file() && (has_ext(p, "zip") || has_ext(p, "wad")))
            .collect();
        candidates.sort();
        for path in &candidates {
            if has_ext(path, "zip") {
                match Archive::from_path_with_options(path, options) {
                    Ok(archive) => {
                        for member in archive
                            .members()
                            .iter()
                            .filter(|m| has_ext(Path::new(m.path()), "wad"))
                        {
                            let source = format!("{}!{}", path.display(), member.path());
                            match archive.wad(member) {
                                Ok(wad) => maps += survey_wad(&wad, &source, &mut seen, &mut visit),
                                Err(e) => eprintln!("{source}: {e}"),
                            }
                        }
                    }
                    Err(e) => eprintln!("{}: {e}", path.display()),
                }
            } else {
                let source = path.display().to_string();
                match Wad::from_path(path) {
                    Ok(wad) => maps += survey_wad(&wad, &source, &mut seen, &mut visit),
                    Err(e) => eprintln!("{source}: {e}"),
                }
            }
        }
    }
    maps
}

fn survey_wad(
    wad: &Wad,
    source: &str,
    seen: &mut BTreeSet<String>,
    visit: &mut impl FnMut(&str, &UdmfMap),
) -> u64 {
    let mut maps = 0;
    for group in &wad.map_groups() {
        let loaded = match ingest::load_map(wad, group) {
            Ok(loaded) => loaded,
            Err(e) => {
                // Named, never silently dropped: a skipped map changes the
                // population, and the count must be explainable.
                eprintln!("{source} {}: {e}", group.name);
                continue;
            }
        };
        if !seen.insert(map_hash(wad, group)) {
            continue;
        }
        maps += 1;
        visit(&group.name, &loaded.map);
    }
    maps
}

/// A string-keyed histogram.
#[derive(Default)]
pub(crate) struct Hist(pub(crate) BTreeMap<String, u64>);

impl Hist {
    pub(crate) fn add(&mut self, key: impl Into<String>) {
        *self.0.entry(key.into()).or_insert(0) += 1;
    }

    /// The `n` most frequent keys as `key:count`, ties broken by key.
    pub(crate) fn top(&self, n: usize) -> String {
        let mut v: Vec<(&String, &u64)> = self.0.iter().collect();
        v.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        v.truncate(n);
        v.iter()
            .map(|(k, c)| format!("{k}:{c}"))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Every key as `key: count`, in key order.
    pub(crate) fn all(&self) -> String {
        self.0
            .iter()
            .map(|(k, c)| format!("{k}: {c}"))
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Every key as `key: count (share of `of`)`, in key order.
    pub(crate) fn shares(&self, of: u64) -> String {
        self.0
            .iter()
            .map(|(k, c)| format!("{k}: {c} ({})", pct(*c, of)))
            .collect::<Vec<_>>()
            .join(" · ")
    }
}

/// `n` as a percentage of `of`, or `n/a` for an empty denominator.
#[expect(
    clippy::cast_precision_loss,
    reason = "counts are far below 2^52; a percentage does not need the low bits"
)]
pub(crate) fn pct(n: u64, of: u64) -> String {
    if of == 0 {
        "n/a".to_owned()
    } else {
        format!("{:.1} %", 100.0 * n as f64 / of as f64)
    }
}

/// `min · p10 · median · p90 · max` of `values`, or `n/a` when empty.
#[expect(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a percentile index is computed from a length that fits comfortably in f64 and \
              rounded back to an in-range index"
)]
pub(crate) fn percentiles(mut values: Vec<i32>) -> String {
    if values.is_empty() {
        return "n/a".to_owned();
    }
    values.sort_unstable();
    let at = |q: f64| values[((values.len() - 1) as f64 * q).round() as usize];
    format!(
        "min {} · p10 {} · median {} · p90 {} · max {}",
        values[0],
        at(0.10),
        at(0.50),
        at(0.90),
        values[values.len() - 1]
    )
}

/// Where a plat rests at load, relative to its neighbors.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Rest {
    /// Travel 0: no neighbor lower than the plat.
    Dead,
    /// Travel > 0 and some neighbor within a step of the plat's floor.
    Top,
    /// Travel > 0 and every neighbor more than a step below.
    AboveAll,
    /// Travel > 0 and some neighbor more than a step above.
    Intermediate,
}

/// Where a trigger line sits relative to the plat it drives.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Placement {
    /// The plat is the line's front sector.
    OnPlatFront,
    /// The plat is the line's back sector.
    OnPlatBack,
    /// A side is a neighbor of the plat, but not the plat.
    Adjacent,
    /// Neither side is the plat or a neighbor.
    Remote,
}

/// The sector a player must stand in to fire a trigger, relative to the plat.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Activator {
    /// Floor more than a step below the plat's.
    Low,
    /// Within a step of the plat's floor, and not the plat itself.
    Level,
    /// The plat itself.
    Plat,
    /// More than a step above.
    Above,
    /// No side can activate (a walkover crossable from neither side at rest).
    None,
}

/// What a plat joins.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Shape {
    /// Rests `Top`, callable from `Low`, repeatable-only, one speed, no conflicting action.
    Core,
    /// `AboveAll` with one neighbor at one floor, same trigger and conflict conditions.
    Pedestal,
    /// `AboveAll` with two or more neighbors all at one floor, same conditions.
    Barrier,
    /// Anything else, `Dead` included.
    Other,
}

/// One trigger line of a plat.
pub(crate) struct Trigger {
    pub(crate) special: i32,
    pub(crate) placement: Placement,
    pub(crate) activators: Vec<Activator>,
    /// The sectors classified `Low` from which this line fires.
    pub(crate) low_sides: Vec<usize>,
    pub(crate) one_sided: bool,
    /// The `SW1*`/`SW2*` textures on the front sidedef, by slot (`top`/`mid`/`bot`).
    pub(crate) switch_slots: Vec<(&'static str, String)>,
}

/// A riser boundary: a two-sided edge whose neighbor's floor is below the plat's.
pub(crate) struct Riser {
    /// The lower texture on the neighbor's sidedef — the side the engine draws.
    pub(crate) texture: String,
    pub(crate) unpegged: bool,
    /// Whether the plat's own sidedef also carries a lower.
    pub(crate) plat_side_nonblank: bool,
}

/// Everything the passes read about one plat.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent measured fact about the plat (what its flat, light \
              and ceiling match; which activators it has); they encode no joint state"
)]
pub(crate) struct PlatFacts {
    /// Whether the scene resolved any boundary for the sector; without one the
    /// bounding-box facts below are meaningless and left at zero.
    pub(crate) has_geometry: bool,
    pub(crate) neighbors: BTreeSet<usize>,
    pub(crate) two_sided_edges: usize,
    pub(crate) one_sided_edges: usize,
    pub(crate) edges_with_special: usize,
    pub(crate) blocking_two_sided: usize,
    pub(crate) bbox_min: (i64, i64),
    pub(crate) bbox_w: i32,
    pub(crate) bbox_h: i32,
    pub(crate) travel: i32,
    /// The highest neighbor's floor minus the plat's (negative when every neighbor is below).
    pub(crate) max_nb_delta: i32,
    pub(crate) distinct_nb_floors: usize,
    pub(crate) rest: Rest,
    pub(crate) shape: Shape,
    pub(crate) triggers: Vec<Trigger>,
    pub(crate) risers: Vec<Riser>,
    pub(crate) flat_same_as_level_nb: Option<bool>,
    pub(crate) flat_same_as_low_nb: Option<bool>,
    pub(crate) light_eq_all: bool,
    pub(crate) ceiling_eq_all: bool,
    pub(crate) light_eq_host: bool,
    pub(crate) flat_eq_host: bool,
    pub(crate) sector_special: i32,
    pub(crate) other_tagged_specials: Vec<i32>,
    pub(crate) two_nb_is_level_and_low: Option<bool>,
    pub(crate) any_blaze: bool,
    pub(crate) all_blaze: bool,
    pub(crate) callable_low: bool,
    /// Some trigger fires from a `Level` or `Plat` activator (a top-side trigger).
    pub(crate) callable_level: bool,
    /// Some trigger fires from a `Level` activator — the census's narrower
    /// "callable from the top" measure, which excludes the plat's own edge.
    pub(crate) callable_level_only: bool,
    /// Neighbors that are `Low` activators of some trigger.
    pub(crate) low_activator_nbs: usize,
    pub(crate) things: u64,
    pub(crate) thing_names: Vec<String>,
    pub(crate) shared_tag_n: usize,
}

impl PlatFacts {
    pub(crate) fn moving(&self) -> bool {
        self.travel > 0
    }

    pub(crate) fn aligned64(&self) -> bool {
        self.bbox_min.0.rem_euclid(64) == 0 && self.bbox_min.1.rem_euclid(64) == 0
    }

    pub(crate) fn island(&self) -> bool {
        self.one_sided_edges == 0
    }
}

/// Per-map lookups both passes need.
pub(crate) struct MapIndex<'a> {
    /// Sector tag → declaration indices.
    pub(crate) by_tag: BTreeMap<i32, Vec<usize>>,
    /// Tag → the non-lift, non-zero specials of the lines naming it.
    pub(crate) other_by_tag: BTreeMap<i32, Vec<i32>>,
    /// Sector → the things located in it.
    pub(crate) things_in: BTreeMap<usize, Vec<&'a SceneThing>>,
}

impl<'a> MapIndex<'a> {
    pub(crate) fn build(map: &UdmfMap, scene: &'a Scene) -> Self {
        let mut by_tag: BTreeMap<i32, Vec<usize>> = BTreeMap::new();
        for (i, s) in map.sectors.iter().enumerate() {
            by_tag.entry(s.id).or_default().push(i);
        }
        let mut other_by_tag: BTreeMap<i32, Vec<i32>> = BTreeMap::new();
        for l in &map.linedefs {
            if l.special != 0 && !is_lift(l.special) && l.args[0] != 0 {
                other_by_tag.entry(l.args[0]).or_default().push(l.special);
            }
        }
        let mut things_in: BTreeMap<usize, Vec<&SceneThing>> = BTreeMap::new();
        for t in &scene.things {
            if let Some(s) = t.sector {
                things_in.entry(s).or_default().push(t);
            }
        }
        Self {
            by_tag,
            other_by_tag,
            things_in,
        }
    }

    /// Every sector some lift line names by tag (tag-0 lines resolve to nothing here).
    pub(crate) fn plat_sectors(&self, map: &UdmfMap) -> BTreeSet<usize> {
        let mut plats = BTreeSet::new();
        for l in &map.linedefs {
            if is_lift(l.special)
                && l.args[0] != 0
                && let Some(v) = self.by_tag.get(&l.args[0])
            {
                plats.extend(v.iter().copied());
            }
        }
        plats
    }
}

fn classify_activator(
    scene: &Scene,
    plat: usize,
    plat_floor: i32,
    sector: usize,
    step: i32,
) -> Activator {
    if sector == plat {
        return Activator::Plat;
    }
    let floor = scene.sectors[sector].floor;
    if floor < plat_floor - step {
        Activator::Low
    } else if floor > plat_floor + step {
        Activator::Above
    } else {
        Activator::Level
    }
}

/// The sides of a lift line that can fire it, per the engine's dispatch
/// rules, as `(sector, class)` pairs.
fn activator_sides(
    map: &UdmfMap,
    scene: &Scene,
    plat: usize,
    plat_floor: i32,
    line_idx: usize,
    step: i32,
) -> Vec<(usize, Activator)> {
    let l = &map.linedefs[line_idx];
    // A dangling side or sector reference cannot fire anything: the engine
    // would read garbage, and `Scene::build` skips such a boundary.
    let Some(front) = side_sector(map, l.sidefront) else {
        return Vec::new();
    };
    let back = l.sideback.and_then(|b| side_sector(map, b));
    let mut out = Vec::new();
    if USE_LIFT.contains(&l.special) {
        // `P_UseSpecialLine`: front side only.
        out.push((
            front,
            classify_activator(scene, plat, plat_floor, front, step),
        ));
    } else if let Some(b) = back {
        // `P_CrossSpecialLine` has no side gate; a side can activate if the
        // crossing from it is possible at rest under `P_TryMove`'s step rule.
        let ff = scene.sectors[front].floor;
        let bf = scene.sectors[b].floor;
        if bf - ff <= step {
            out.push((
                front,
                classify_activator(scene, plat, plat_floor, front, step),
            ));
        }
        if ff - bf <= step {
            out.push((b, classify_activator(scene, plat, plat_floor, b, step)));
        }
    }
    out
}

/// The activator classes of a lift line, deduplicated; `None` when no side
/// can fire it.
fn activators(sides: &[(usize, Activator)]) -> Vec<Activator> {
    let mut out: Vec<Activator> = sides.iter().map(|&(_, a)| a).collect();
    if out.is_empty() {
        out.push(Activator::None);
    }
    out.sort();
    out.dedup();
    out
}

fn triggers_of(
    map: &UdmfMap,
    scene: &Scene,
    plat: usize,
    neighbors: &BTreeSet<usize>,
    step: i32,
) -> Vec<Trigger> {
    let plat_floor = scene.sectors[plat].floor;
    let tag = map.sectors[plat].id;
    let mut triggers = Vec::new();
    for (i, l) in map.linedefs.iter().enumerate() {
        if !is_lift(l.special) || l.args[0] != tag {
            continue;
        }
        // A lift line whose front side dangles contributes no trigger: there
        // is no sector a player could fire it from.
        let Some(front) = side_sector(map, l.sidefront) else {
            continue;
        };
        let back = l.sideback.and_then(|b| side_sector(map, b));
        let placement = if front == plat {
            Placement::OnPlatFront
        } else if back == Some(plat) {
            Placement::OnPlatBack
        } else if neighbors.contains(&front) || back.is_some_and(|b| neighbors.contains(&b)) {
            Placement::Adjacent
        } else {
            Placement::Remote
        };
        let mut switch_slots = Vec::new();
        if USE_LIFT.contains(&l.special)
            && let Some(sd) = sidedef(map, l.sidefront)
        {
            for (slot, tex) in [
                ("top", &sd.texturetop),
                ("mid", &sd.texturemiddle),
                ("bot", &sd.texturebottom),
            ] {
                if tex.starts_with("SW1") || tex.starts_with("SW2") {
                    switch_slots.push((slot, tex.clone()));
                }
            }
        }
        let sides = activator_sides(map, scene, plat, plat_floor, i, step);
        triggers.push(Trigger {
            special: l.special,
            placement,
            activators: activators(&sides),
            low_sides: sides
                .iter()
                .filter(|&&(_, a)| a == Activator::Low)
                .map(|&(s, _)| s)
                .collect(),
            one_sided: back.is_none(),
            switch_slots,
        });
    }
    triggers
}

/// Rounds a world coordinate to the integer grid the map was authored on.
#[expect(
    clippy::cast_possible_truncation,
    reason = "map coordinates are 16-bit integers in every binary map and integral in every \
              UDMF map this probe reads; rounding is a no-op that satisfies the type"
)]
fn round(v: f64) -> i64 {
    v.round() as i64
}

/// Analyzes the plat at sector `plat`. Returns `None` when no lift line names
/// it (which cannot happen for a sector from [`MapIndex::plat_sectors`]).
#[expect(
    clippy::too_many_lines,
    reason = "one plat's facts are one coherent measurement; splitting the boundary walk from \
              the trigger and riser walks would scatter the shared neighbor set across calls"
)]
pub(crate) fn analyze_plat(
    map: &UdmfMap,
    scene: &Scene,
    index: &MapIndex<'_>,
    plat: usize,
    step: i32,
) -> Option<PlatFacts> {
    let ss = &scene.sectors[plat];
    let sec = &map.sectors[plat];
    // A sector the scene resolved no boundary for is still a plat the engine
    // would run (`EV_DoPlat` over zero lines: `low == high`, a no-op) — it is
    // counted as Dead, but it has no geometry to measure.
    let has_geometry = !ss.boundary.is_empty();
    let mut neighbors: BTreeSet<usize> = BTreeSet::new();
    let (mut two, mut one, mut with_special, mut blocking) = (0, 0, 0, 0);
    let (mut west, mut south, mut east, mut north) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let mut risers = Vec::new();
    let mut light_eq_all = true;
    let mut ceiling_eq_all = true;
    for b in &ss.boundary {
        for (x, y) in [b.a, b.b] {
            west = west.min(x);
            south = south.min(y);
            east = east.max(x);
            north = north.max(y);
        }
        if !b.two_sided {
            one += 1;
            continue;
        }
        two += 1;
        if is_lift(b.special) {
            with_special += 1;
        }
        if b.blocking {
            blocking += 1;
        }
        let Some(n) = b.neighbor else { continue };
        neighbors.insert(n);
        if scene.sectors[n].light != ss.light {
            light_eq_all = false;
        }
        if scene.sectors[n].ceiling != ss.ceiling {
            ceiling_eq_all = false;
        }
        if scene.sectors[n].floor < ss.floor {
            // The visible lower is on the sidedef whose sector has the lower
            // floor (`r_segs.c`, `R_StoreWallRange`): the neighbor's.
            let l = &map.linedefs[b.linedef];
            let nb_side = if side_sector(map, l.sidefront) == Some(n) {
                sidedef(map, l.sidefront)
            } else {
                l.sideback.and_then(|s| sidedef(map, s))
            };
            // `Scene` resolved this boundary, so its own sidedef is in range;
            // the neighbor's side is looked up defensively all the same.
            if let Some(nb_side) = nb_side {
                risers.push(Riser {
                    texture: nb_side.texturebottom.clone(),
                    unpegged: b.lower_unpegged,
                    plat_side_nonblank: map.sidedefs[b.sidedef].texturebottom != "-",
                });
            }
        }
    }
    let nb_floors: Vec<i32> = neighbors.iter().map(|&n| scene.sectors[n].floor).collect();
    // `P_FindLowestFloorSurrounding`: starts at the sector's own floor.
    let low = nb_floors.iter().copied().fold(ss.floor, i32::min);
    let travel = ss.floor - low;
    let max_nb = nb_floors.iter().copied().max().unwrap_or(ss.floor);
    let rest = if travel == 0 {
        Rest::Dead
    } else if max_nb > ss.floor + step {
        Rest::Intermediate
    } else if max_nb >= ss.floor - step {
        Rest::Top
    } else {
        Rest::AboveAll
    };
    let triggers = triggers_of(map, scene, plat, &neighbors, step);
    if triggers.is_empty() {
        return None;
    }
    let any_blaze = triggers.iter().any(|t| BLAZE.contains(&t.special));
    let all_blaze = triggers.iter().all(|t| BLAZE.contains(&t.special));
    let callable_low = triggers
        .iter()
        .any(|t| t.activators.contains(&Activator::Low));
    let callable_level = triggers.iter().any(|t| {
        t.activators
            .iter()
            .any(|a| matches!(a, Activator::Level | Activator::Plat))
    });
    let callable_level_only = triggers
        .iter()
        .any(|t| t.activators.contains(&Activator::Level));
    let all_repeat = triggers
        .iter()
        .all(|t| REPEATABLE_LIFT.contains(&t.special));
    let one_speed = !any_blaze || all_blaze;
    let mut other_tagged_specials = index.other_by_tag.get(&sec.id).cloned().unwrap_or_default();
    other_tagged_specials.sort_unstable();
    other_tagged_specials.dedup();
    let distinct_nb_floors = nb_floors.iter().copied().collect::<BTreeSet<i32>>().len();
    let clean = callable_low && all_repeat && one_speed && other_tagged_specials.is_empty();
    let shape = if !clean {
        Shape::Other
    } else if rest == Rest::Top {
        Shape::Core
    } else if rest == Rest::AboveAll && distinct_nb_floors == 1 && neighbors.len() == 1 {
        Shape::Pedestal
    } else if rest == Rest::AboveAll && distinct_nb_floors == 1 {
        Shape::Barrier
    } else {
        Shape::Other
    };
    // Neighbors from which some trigger fires with a `Low` activator.
    let low_activator_nbs: BTreeSet<usize> = triggers
        .iter()
        .flat_map(|t| t.low_sides.iter().copied())
        .filter(|s| neighbors.contains(s))
        .collect();
    let level_nb = neighbors
        .iter()
        .copied()
        .find(|&n| (scene.sectors[n].floor - ss.floor).abs() <= step);
    let low_nb = neighbors
        .iter()
        .copied()
        .find(|&n| scene.sectors[n].floor == low && low < ss.floor);
    let host = match shape {
        Shape::Core => level_nb,
        _ => neighbors.iter().copied().next(),
    };
    let things = index.things_in.get(&plat).map_or(&[][..], Vec::as_slice);
    Some(PlatFacts {
        two_sided_edges: two,
        one_sided_edges: one,
        edges_with_special: with_special,
        blocking_two_sided: blocking,
        has_geometry,
        bbox_min: if has_geometry {
            (round(west), round(south))
        } else {
            (0, 0)
        },
        bbox_w: if has_geometry {
            i32::try_from(round(east - west)).expect("map extents fit i32")
        } else {
            0
        },
        bbox_h: if has_geometry {
            i32::try_from(round(north - south)).expect("map extents fit i32")
        } else {
            0
        },
        travel,
        max_nb_delta: max_nb - ss.floor,
        distinct_nb_floors,
        rest,
        shape,
        risers,
        flat_same_as_level_nb: level_nb.map(|n| map.sectors[n].texturefloor == sec.texturefloor),
        flat_same_as_low_nb: low_nb.map(|n| map.sectors[n].texturefloor == sec.texturefloor),
        light_eq_all,
        ceiling_eq_all,
        light_eq_host: host.is_some_and(|h| scene.sectors[h].light == ss.light),
        flat_eq_host: host.is_some_and(|h| map.sectors[h].texturefloor == sec.texturefloor),
        sector_special: sec.special,
        other_tagged_specials,
        two_nb_is_level_and_low: (neighbors.len() == 2)
            .then(|| level_nb.is_some() && low_nb.is_some() && level_nb != low_nb),
        any_blaze,
        all_blaze,
        callable_low,
        callable_level,
        callable_level_only,
        low_activator_nbs: low_activator_nbs.len(),
        things: u64::try_from(things.len()).expect("fits u64"),
        thing_names: things
            .iter()
            .map(|t| {
                t.name
                    .clone()
                    .unwrap_or_else(|| format!("type {}", t.type_id))
            })
            .collect(),
        shared_tag_n: index.by_tag.get(&sec.id).map_or(0, Vec::len),
        triggers,
        neighbors,
    })
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use crustygen::tables::Tables;
    use crustywad::map::udmf::parse_udmf;

    use super::*;

    /// A row of 128×128 boxes, room `i` spanning `x ∈ [i·128, (i+1)·128]`,
    /// `y ∈ [0, 128]`, ceilings at 256. `floors[i]` and `tags[i]` are room
    /// `i`'s floor and sector tag. `links[i]` describes the two-sided line
    /// between rooms `i` and `i+1` as `(special, arg0, front_is_east)`: with
    /// `front_is_east` false the line runs top-to-bottom so its front (right)
    /// side is the west room, the natural clockwise orientation; `true` flips
    /// it so the east room is the front. Every link sidedef carries
    /// `SUPPORT3` as its lower; every one-sided wall is `STARTAN2`. `extra` is
    /// appended verbatim.
    fn chain(floors: &[i32], tags: &[i32], links: &[(i32, i32, bool)], extra: &str) -> String {
        let n = floors.len();
        assert_eq!(tags.len(), n);
        assert_eq!(links.len(), n - 1);
        let mut text = String::from("namespace = \"doom\";\n");
        for i in 0..=n {
            let x = i * 128;
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = 0.000; }}");
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = 128.000; }}");
        }
        let mut sidedefs = String::new();
        let mut next = 0usize;
        let mut side = |sidedefs: &mut String, sector: usize, lower: bool| {
            let tex = if lower {
                "texturemiddle = \"-\"; texturebottom = \"SUPPORT3\";"
            } else {
                "texturemiddle = \"STARTAN2\";"
            };
            let _ = writeln!(sidedefs, "sidedef {{ sector = {sector}; {tex} }}");
            next += 1;
            next - 1
        };
        for (i, &(special, tag, east_front)) in links.iter().enumerate() {
            let (top, bottom) = (2 * i + 3, 2 * i + 2);
            let (v1, v2, front, back) = if east_front {
                (bottom, top, i + 1, i)
            } else {
                (top, bottom, i, i + 1)
            };
            let sf = side(&mut sidedefs, front, true);
            let sb = side(&mut sidedefs, back, true);
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; sideback = {sb}; twosided = true; special = {special}; arg0 = {tag}; }}"
            );
        }
        for i in 0..n {
            let (bl, tl, br, tr) = (2 * i, 2 * i + 1, 2 * i + 2, 2 * i + 3);
            let mut wall = |text: &mut String, v1: usize, v2: usize| {
                let s = side(&mut sidedefs, i, false);
                let _ = writeln!(
                    text,
                    "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {s}; blocking = true; }}"
                );
            };
            if i == 0 {
                wall(&mut text, bl, tl);
            }
            wall(&mut text, tl, tr);
            if i == n - 1 {
                wall(&mut text, tr, br);
            }
            wall(&mut text, br, bl);
        }
        text.push_str(&sidedefs);
        for (floor, tag) in floors.iter().zip(tags) {
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = {floor}; heightceiling = 256; lightlevel = 160; id = {tag}; }}"
            );
        }
        text.push_str(extra);
        text
    }

    struct Fixture {
        map: UdmfMap,
        scene: Scene,
        step: i32,
    }

    fn fixture(text: &str) -> Fixture {
        let tables = Tables::load().expect("tables");
        let map = parse_udmf(text, Limits::default()).expect("fixture parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        Fixture {
            map,
            scene,
            step: tables.step_height(),
        }
    }

    fn analyze(f: &Fixture, plat: usize) -> Option<PlatFacts> {
        let index = MapIndex::build(&f.map, &f.scene);
        analyze_plat(&f.map, &f.scene, &index, plat, f.step)
    }

    // The low room is the front of the riser line, the plat its back: the
    // corpus's dominant `S OnPlatBack` form. Room 2 is the level landing.
    const LIFT_FLOORS: [i32; 3] = [0, 128, 128];
    const LIFT_TAGS: [i32; 3] = [0, 7, 0];

    #[test]
    fn a_riser_switch_from_the_low_room_is_a_core_lift() {
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&f, 1).expect("the plat is analyzed");
        assert!(p.has_geometry);
        assert_eq!(p.rest, Rest::Top);
        assert_eq!(
            p.travel, 128,
            "travel is the plat's floor minus the lowest neighbor's"
        );
        assert_eq!(p.shape, Shape::Core);
        assert_eq!(p.neighbors, BTreeSet::from([0, 2]));
        assert_eq!(p.two_nb_is_level_and_low, Some(true));
        assert!(p.callable_low && !p.callable_level && !p.callable_level_only);
        assert_eq!(p.low_activator_nbs, 1);
        assert_eq!(p.triggers.len(), 1);
        let t = &p.triggers[0];
        assert_eq!(t.special, 62);
        assert_eq!(t.placement, Placement::OnPlatBack);
        assert_eq!(t.activators, vec![Activator::Low]);
        assert_eq!(t.low_sides, vec![0]);
        assert!(!t.one_sided);
        assert!(
            t.switch_slots.is_empty(),
            "no switch texture on a bare riser"
        );
        assert_eq!(p.risers.len(), 1, "one neighbor below the plat");
        assert_eq!(p.risers[0].texture, "SUPPORT3");
        assert!(!p.risers[0].unpegged);
        assert!(
            p.risers[0].plat_side_nonblank,
            "the plat's own lower is set too"
        );
        assert_eq!(p.flat_same_as_level_nb, Some(true));
        assert_eq!((p.bbox_w, p.bbox_h), (128, 128));
        assert!(p.aligned64(), "the plat's low corner is (128, 0)");
        assert!(!p.island(), "the plat has one-sided top and bottom walls");
        assert_eq!(p.shared_tag_n, 1);
    }

    #[test]
    fn a_walkover_on_the_top_face_adds_a_level_activator_and_keeps_the_shape() {
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (88, 7, false)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.shape, Shape::Core);
        assert!(p.callable_low && p.callable_level && p.callable_level_only);
        let top = &p.triggers[1];
        assert_eq!(top.special, 88);
        assert_eq!(top.placement, Placement::OnPlatFront);
        // Equal floors: the crossing works from either side.
        assert_eq!(top.activators, vec![Activator::Level, Activator::Plat]);
        assert!(top.low_sides.is_empty());
    }

    #[test]
    fn a_walkover_on_the_low_face_cannot_fire_from_below() {
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(88, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        // `P_TryMove` refuses the 128-unit climb, so only the plat side can
        // cross the line — the lift is not callable from the low room.
        assert_eq!(p.triggers[0].activators, vec![Activator::Plat]);
        assert!(!p.callable_low);
        assert!(p.callable_level && !p.callable_level_only);
        assert_eq!(p.shape, Shape::Other);
    }

    #[test]
    fn a_switch_only_on_the_level_side_is_top_only_and_refused() {
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(0, 0, false), (62, 7, true)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.triggers[0].placement, Placement::OnPlatBack);
        assert_eq!(p.triggers[0].activators, vec![Activator::Level]);
        assert!(!p.callable_low && p.callable_level_only);
        assert_eq!(p.rest, Rest::Top);
        assert_eq!(p.shape, Shape::Other);
    }

    #[test]
    fn a_use_line_fires_from_its_front_side_only() {
        // The same riser line, flipped so the plat is its front: a use from
        // the low room hits the back side and `P_UseSpecialLine` refuses it.
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, true), (0, 0, false)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.triggers[0].placement, Placement::OnPlatFront);
        assert_eq!(p.triggers[0].activators, vec![Activator::Plat]);
        assert!(!p.callable_low);
        assert_eq!(p.shape, Shape::Other);
    }

    #[test]
    fn a_raised_block_with_one_low_neighbor_is_a_pedestal() {
        let f = fixture(&chain(&[0, 128], &[0, 7], &[(62, 7, false)], ""));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.rest, Rest::AboveAll);
        assert_eq!(p.max_nb_delta, -128);
        assert_eq!(p.distinct_nb_floors, 1);
        assert_eq!(p.shape, Shape::Pedestal);
        assert_eq!(p.low_activator_nbs, 1);
    }

    #[test]
    fn a_raised_block_between_two_low_rooms_is_a_barrier_when_both_can_call_it() {
        let f = fixture(&chain(
            &[0, 96, 0],
            &[0, 7, 0],
            &[(62, 7, false), (62, 7, true)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.rest, Rest::AboveAll);
        assert_eq!(p.travel, 96);
        assert_eq!(p.shape, Shape::Barrier);
        assert_eq!(p.low_activator_nbs, 2, "callable from both neighbors");
        assert!(
            p.triggers
                .iter()
                .all(|t| t.activators == vec![Activator::Low])
        );
    }

    #[test]
    fn a_block_above_neighbors_at_several_floors_is_neither_pedestal_nor_barrier() {
        let f = fixture(&chain(
            &[0, 160, 64],
            &[0, 7, 0],
            &[(62, 7, false), (62, 7, true)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.rest, Rest::AboveAll);
        assert_eq!(p.distinct_nb_floors, 2);
        assert_eq!(p.shape, Shape::Other);
    }

    #[test]
    fn a_middle_landing_rests_intermediate() {
        let f = fixture(&chain(
            &[0, 64, 160],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.rest, Rest::Intermediate);
        assert_eq!(p.max_nb_delta, 96);
        assert_eq!(p.shape, Shape::Other);
    }

    #[test]
    fn a_plat_at_its_lowest_neighbors_height_is_dead() {
        let f = fixture(&chain(
            &[0, 0, 0],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.travel, 0);
        assert_eq!(p.rest, Rest::Dead);
        assert!(!p.moving());
        assert_eq!(p.shape, Shape::Other);
        assert!(p.risers.is_empty(), "no neighbor is below a dead plat");
    }

    #[test]
    fn one_shot_and_mixed_speed_triggers_refuse_an_otherwise_core_lift() {
        let one_shot = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(21, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&one_shot, 1).expect("analyzed");
        assert!(p.callable_low, "the S1 form still fires from below");
        assert_eq!(p.shape, Shape::Other, "but the one-shot form is refused");

        let mixed = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (120, 7, false)],
            "",
        ));
        let p = analyze(&mixed, 1).expect("analyzed");
        assert!(p.any_blaze && !p.all_blaze);
        assert_eq!(p.shape, Shape::Other);

        let fast = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(123, 7, false), (120, 7, false)],
            "",
        ));
        let p = analyze(&fast, 1).expect("analyzed");
        assert!(p.any_blaze && p.all_blaze);
        assert_eq!(p.shape, Shape::Core, "one speed throughout is fine");
    }

    #[test]
    fn another_tagged_action_on_the_plat_refuses_it() {
        // A floor-lower special (23) naming the plat's tag from the level room's wall.
        let extra = "linedef { v1 = 5; v2 = 4; sidefront = 99; blocking = true; special = 23; arg0 = 7; }\n";
        let mut text = chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        );
        // Give the extra line a sidedef in room 2 (index 99 is replaced by the real next index).
        let sidedef_count = text.matches("sidedef {").count();
        text.push_str(&extra.replace("99", &sidedef_count.to_string()));
        text.push_str("sidedef { sector = 2; texturemiddle = \"STARTAN2\"; }\n");
        let f = fixture(&text);
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.other_tagged_specials, vec![23]);
        assert_eq!(p.shape, Shape::Other);
    }

    #[test]
    fn a_tag_naming_several_sectors_is_counted_on_each() {
        let f = fixture(&chain(
            &[0, 128, 128],
            &[0, 7, 7],
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let index = MapIndex::build(&f.map, &f.scene);
        assert_eq!(index.plat_sectors(&f.map), BTreeSet::from([1, 2]));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.shared_tag_n, 2);
    }

    #[test]
    fn a_tag_zero_lift_line_names_no_plat() {
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 0, false), (0, 0, false)],
            "",
        ));
        let index = MapIndex::build(&f.map, &f.scene);
        assert!(index.plat_sectors(&f.map).is_empty());
    }

    #[test]
    fn dangling_side_references_are_non_activatable_not_fatal() {
        let mut f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        assert!(sidedef(&f.map, -1).is_none());
        assert!(sidedef(&f.map, 9_999).is_none());
        assert!(side_sector(&f.map, 0).is_some());
        // A sidedef whose sector reference dangles.
        f.map.sidedefs[0].sector = 9_999;
        assert!(side_sector(&f.map, 0).is_none());
        // The riser line's front side dangles: the line contributes no
        // trigger, so the plat has none and is not analyzed — and nothing
        // panics on the way.
        f.map.linedefs[0].sidefront = 9_999;
        let index = MapIndex::build(&f.map, &f.scene);
        assert!(analyze_plat(&f.map, &f.scene, &index, 1, f.step).is_none());
    }

    #[test]
    fn a_sector_with_no_boundary_is_a_dead_plat_without_geometry() {
        // A fourth sector no linedef references, tagged 9, named by the far link.
        let extra = "sector { texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 9; }\n";
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (62, 9, false)],
            extra,
        ));
        let index = MapIndex::build(&f.map, &f.scene);
        assert!(index.plat_sectors(&f.map).contains(&3));
        let p = analyze(&f, 3).expect("a trigger names it, so it is analyzed");
        assert!(!p.has_geometry);
        assert_eq!(p.rest, Rest::Dead);
        assert!(p.neighbors.is_empty());
        assert_eq!((p.bbox_w, p.bbox_h, p.bbox_min), (0, 0, (0, 0)));
        assert_eq!(p.shape, Shape::Other);
        assert_eq!(p.triggers[0].placement, Placement::Remote);
    }

    #[test]
    fn things_are_counted_on_the_plat_that_holds_them() {
        let extra = "thing { x = 192.000; y = 64.000; type = 3001; single = true; }\n";
        let f = fixture(&chain(
            &LIFT_FLOORS,
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            extra,
        ));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.things, 1);
        assert_eq!(p.thing_names, vec!["imp".to_owned()]);
    }

    #[test]
    fn the_step_boundary_is_exact() {
        // 24 units is one step (`tmfloorz - thing->z > 24*FRACUNIT` rejects
        // only beyond it): the low room is Level, the plat a step, not a lift.
        let step = fixture(&chain(
            &[0, 24, 24],
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&step, 1).expect("analyzed");
        assert_eq!(p.rest, Rest::Top);
        assert_eq!(p.triggers[0].activators, vec![Activator::Level]);
        assert!(!p.callable_low);
        // 25 units is not.
        let lift = fixture(&chain(
            &[0, 25, 25],
            &LIFT_TAGS,
            &[(62, 7, false), (0, 0, false)],
            "",
        ));
        let p = analyze(&lift, 1).expect("analyzed");
        assert_eq!(p.triggers[0].activators, vec![Activator::Low]);
        assert!(p.callable_low);
        assert_eq!(p.shape, Shape::Core);
    }

    #[test]
    fn helpers_format_shares_and_percentiles() {
        assert_eq!(pct(1, 4), "25.0 %");
        assert_eq!(pct(0, 0), "n/a");
        assert_eq!(percentiles(Vec::new()), "n/a");
        assert_eq!(
            percentiles(vec![5, 1, 3]),
            "min 1 · p10 1 · median 3 · p90 5 · max 5"
        );
        let mut h = Hist::default();
        for k in ["b", "a", "b", "c"] {
            h.add(k);
        }
        assert_eq!(h.top(2), "b:2, a:1");
        assert_eq!(h.all(), "a: 1 · b: 2 · c: 1");
        assert_eq!(h.shares(4), "a: 1 (25.0 %) · b: 2 (50.0 %) · c: 1 (25.0 %)");
        assert!(is_lift(62) && is_lift(121) && !is_lift(97));
    }
}
