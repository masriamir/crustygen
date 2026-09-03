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

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, btree_map};
use std::path::{Path, PathBuf};

use crustygen::check::scene::{Boundary, Scene, SceneThing};
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
/// The walkover lift specials (`P_CrossSpecialLine`, either side) — the
/// complement of [`USE_LIFT`] within [`DWUS`] ∪ [`BLAZE`].
pub(crate) const WALK_LIFT: [i32; 4] = [88, 10, 120, 121];
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
/// `crustygen-corpus`) and calls `visit` once per unique loaded map. An entry
/// that names a file rather than a directory is that one archive or WAD, so a
/// smoke run can point straight at a single map file. Returns the unique-map
/// count. Unreadable archives, WADs and map groups are named on stderr and
/// skipped.
pub(crate) fn sweep(dirs: &[String], mut visit: impl FnMut(&str, &UdmfMap)) -> u64 {
    let options = ParseOptions {
        strictness: Strictness::Lenient,
        limits: Limits::default(),
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut maps = 0;
    for dir in dirs {
        let named = Path::new(dir);
        let mut candidates: Vec<PathBuf> = if named.is_file() {
            vec![named.to_path_buf()]
        } else {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    // A population that cannot be listed is not a partial
                    // result; fail plainly, with the usage exit code, rather
                    // than panic.
                    eprintln!("cannot list {dir}: {e}");
                    std::process::exit(2);
                }
            };
            entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_file() && (has_ext(p, "zip") || has_ext(p, "wad")))
                .collect()
        };
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

/// Where a trigger line sits relative to the sector it drives.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Placement {
    /// The target is the line's front sector.
    OnPlatFront,
    /// The target is the line's back sector.
    OnPlatBack,
    /// A side is a neighbor of the target, but not the target.
    Adjacent,
    /// Neither side is the target or a neighbor.
    Remote,
}

impl Placement {
    /// The label the floor report prints. The lift passes keep saying "plat",
    /// which is what their measurement doc calls the same position.
    pub(crate) fn target_label(self) -> &'static str {
        match self {
            Self::OnPlatFront => "OnTargetFront",
            Self::OnPlatBack => "OnTargetBack",
            Self::Adjacent => "Adjacent",
            Self::Remote => "Remote",
        }
    }
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
    /// For a walkover: one entry per `Low` activator side, measuring the far
    /// sector the crossing would land in. Empty for a use-line, and for a
    /// walkover no `Low` activator can fire.
    pub(crate) low_crossings: Vec<LowWalkCross>,
    /// `Low` activator sides of a walkover with no far sector to measure.
    /// Measured over the corpus, every one of these is a line whose two sides
    /// name the **same** sector — a trigger line drawn inside a single room,
    /// where crossing changes nothing about where the player stands. (The
    /// same arm also catches a degenerate or dangling line, of which the
    /// corpus has none here.) Counted so §G3's denominator reconciles with
    /// §G2's line count rather than being silently short.
    pub(crate) low_unmeasured: usize,
}

/// A riser boundary: a two-sided edge whose neighbor's floor is below the plat's.
pub(crate) struct Riser {
    /// The lower texture on the neighbor's sidedef — the side the engine draws.
    pub(crate) texture: String,
    pub(crate) unpegged: bool,
    /// Whether the plat's own sidedef also carries a lower.
    pub(crate) plat_side_nonblank: bool,
}

/// A jamb: a one-sided edge of the plat — the shaft's side wall. The engine
/// draws only the middle slot of a one-sided sidedef (`r_segs.c`,
/// `R_StoreWallRange`: `midtexture = texturetranslation[sidedef->midtexture]`
/// in the `if (!backsector)` branch).
pub(crate) struct Jamb {
    /// The middle texture on the plat's own (only) sidedef.
    pub(crate) texture: String,
    /// `ML_DONTPEGTOP` (`0x0008`) on the linedef.
    pub(crate) dontpegtop: bool,
    /// `ML_DONTPEGBOTTOM` (`0x0010`) on the linedef.
    pub(crate) dontpegbottom: bool,
}

/// One low-side walkover crossing: a player standing in the activator sector
/// `S` (more than a step below the plat) about to cross a walkover lift line
/// into the far sector `T` on its other side. Both depths are measured
/// perpendicular to the line, positive on `T`'s side, so the line's own
/// endpoints sit at 0.
pub(crate) struct LowWalkCross {
    /// Declaration index of the trigger linedef.
    pub(crate) linedef: usize,
    /// `T`'s greatest perpendicular distance beyond the line, over every
    /// vertex of every boundary of `T`. A crossing whose value is ≤ 16 can
    /// never happen: the player's center would have to sit strictly more than
    /// its 16-unit radius clear of a blocking edge (`P_BoxOnLineSide` counts a
    /// touching box as straddling), and `T` has no room for it.
    pub(crate) max_vertex: f64,
    /// The least perpendicular distance to a *blocking* boundary of `T` — one
    /// -sided, or a step of more than `step` up out of `T` — whose projection
    /// onto the line overlaps the line's own extent. `None` when `T` has no
    /// such boundary in front of the line.
    pub(crate) nearest_blocking: Option<f64>,
}

/// A two-sided side wall: a boundary to a neighbor at *exactly* the plat's
/// floor whose ceiling differs — neither a riser (a neighbor below) nor a
/// landing sharing the plat's ceiling.
pub(crate) struct SideWall {
    /// The middle texture on the plat's own sidedef.
    pub(crate) texture: String,
    /// Whether the neighbor's ceiling is *above* the plat's — in which case
    /// this edge is also one of [`PlatFacts::top_faces`]. The two definitions
    /// overlap by construction, so the report states both counts and the
    /// split.
    pub(crate) nb_ceiling_above: bool,
}

/// A top face: a boundary to a level landing (a neighbor within a step of the
/// plat's floor) whose ceiling is *above* the plat's — so the plat sits in a
/// shaft or alcove under a lower ceiling and the engine draws an upper on the
/// landing's sidedef (`r_segs.c`: the top-texture branch runs when
/// `worldhigh < worldtop`, i.e. when the *back* sector's ceiling is the lower
/// one, so the drawn side is the landing's).
pub(crate) struct TopFace {
    /// The upper texture on the landing's sidedef — the side the engine draws.
    pub(crate) texture: String,
    /// `ML_DONTPEGTOP` on the linedef.
    pub(crate) dontpegtop: bool,
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
    /// The plat's one-sided edges — its side walls.
    pub(crate) jambs: Vec<Jamb>,
    /// The plat's two-sided side walls (see [`SideWall`]).
    pub(crate) two_sided_jambs: Vec<SideWall>,
    /// The plat's top faces (see [`TopFace`]).
    pub(crate) top_faces: Vec<TopFace>,
    /// Two-sided boundaries of the plat where the higher-ceiling side carries
    /// a non-blank upper, so the engine draws one.
    pub(crate) uppers_drawn: usize,
    /// How many of [`PlatFacts::uppers_drawn`] carry `ML_DONTPEGTOP`.
    pub(crate) uppers_drawn_pegtop: usize,
    /// The plat's own floor flat.
    pub(crate) floor_flat: String,
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

/// The sector a player fires a trigger from, classified against the target's
/// floor. Target-agnostic: `target` is a plat for the lift passes and a
/// tagged floor sector for the `floors` pass.
fn classify_activator(
    scene: &Scene,
    target: usize,
    target_floor: i32,
    sector: usize,
    step: i32,
) -> Activator {
    if sector == target {
        return Activator::Plat;
    }
    let floor = scene.sectors[sector].floor;
    if floor < target_floor - step {
        Activator::Low
    } else if floor > target_floor + step {
        Activator::Above
    } else {
        Activator::Level
    }
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
        Placement::OnPlatFront
    } else if back == Some(target) {
        Placement::OnPlatBack
    } else if front.is_some_and(|f| neighbors.contains(&f))
        || back.is_some_and(|b| neighbors.contains(&b))
    {
        Placement::Adjacent
    } else {
        Placement::Remote
    }
}

/// Which of the three dispatchers fires a trigger line, which is what
/// decides the sides it can be fired from. Mirrors
/// `crustygen::check::plats::Dispatch` so the probe and the verifier cannot
/// drift on who can press what.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Dispatch {
    /// `P_CrossSpecialLine` — a walkover. No side gate; a side fires it when
    /// the crossing from it is possible at rest.
    Cross,
    /// `P_UseSpecialLine` — a switch, front side only (`p_switch.c:284-297`:
    /// the opening `if (side)` block `return false`s for every special but
    /// 124).
    Use,
    /// `P_ShootSpecialLine` — a gun line, fired from **either** side it
    /// faces. `P_ShootSpecialLine` (`p_spec.c:955-1000`) takes no `side`
    /// argument and gates on none, and its only caller passes none:
    /// `PTR_ShootTraverse` runs `if (li->special) P_ShootSpecialLine
    /// (shootthing, li);` (`p_map.c:919-920`) two lines before it has read
    /// `ML_TWOSIDED` (`p_map.c:922`). The dispatch carries floor specials 24
    /// and 47 (plus door 46); no lift special reaches it.
    Shot,
}

impl Dispatch {
    /// The dispatcher a [`TriggerKind`] goes through.
    pub(crate) fn of(kind: TriggerKind) -> Self {
        match kind {
            TriggerKind::Walk => Self::Cross,
            TriggerKind::Switch => Self::Use,
            TriggerKind::Gun => Self::Shot,
        }
    }
}

/// The sides of a trigger line that can fire it, per the engine's dispatch
/// rules, as `(sector, class)` pairs. `dispatch` selects the rule — see
/// [`Dispatch`].
fn activator_sides(
    map: &UdmfMap,
    scene: &Scene,
    target: usize,
    target_floor: i32,
    line_idx: usize,
    step: i32,
    dispatch: Dispatch,
) -> Vec<(usize, Activator)> {
    let l = &map.linedefs[line_idx];
    // A dangling side or sector reference cannot fire anything: the engine
    // would read garbage, and `Scene::build` skips such a boundary.
    let Some(front) = side_sector(map, l.sidefront) else {
        return Vec::new();
    };
    let (plat, plat_floor) = (target, target_floor);
    let back = l.sideback.and_then(|b| side_sector(map, b));
    let mut out = Vec::new();
    let push = |s: usize, out: &mut Vec<(usize, Activator)>| {
        out.push((s, classify_activator(scene, plat, plat_floor, s, step)));
    };
    match dispatch {
        // `P_UseSpecialLine`: front side only (`p_switch.c:284-297`).
        Dispatch::Use => push(front, &mut out),
        // `P_ShootSpecialLine` has no side gate and neither does its caller,
        // so a shot from either bordering sector fires it. A
        // self-referencing line's back sector is the front one: one sector
        // to shoot from, not two.
        Dispatch::Shot => {
            push(front, &mut out);
            if let Some(b) = back.filter(|&b| b != front) {
                push(b, &mut out);
            }
        }
        // `P_CrossSpecialLine` has no side gate; a side can activate if the
        // crossing from it is possible at rest under `P_TryMove`'s step rule.
        Dispatch::Cross => {
            if let Some(b) = back {
                let ff = scene.sectors[front].floor;
                let bf = scene.sectors[b].floor;
                if bf - ff <= step {
                    push(front, &mut out);
                }
                if ff - bf <= step {
                    push(b, &mut out);
                }
            }
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

/// The perpendicular depth of sector `t` beyond linedef `line_idx`, measured
/// on `t`'s side, as `(farthest vertex, nearest blocking boundary)` — see
/// [`LowWalkCross`] for what each estimates. `None` when the line is
/// degenerate or its vertex references dangle.
///
/// A two-sided boundary flagged `ML_BLOCKING` (a fence the player cannot walk
/// through) is deliberately **not** treated as blocking here: the definition
/// this measures is the brief's, "one-sided, or floor step > 24 relative to
/// `T`", so the nearest-blocking estimate is an upper bound on how far the
/// player really gets.
fn far_depth(
    map: &UdmfMap,
    scene: &Scene,
    line_idx: usize,
    t: usize,
    step: i32,
) -> Option<(f64, Option<f64>)> {
    let l = &map.linedefs[line_idx];
    let vertex = |v: i32| {
        usize::try_from(v)
            .ok()
            .filter(|&i| i < map.vertices.len())
            .map(|i| (map.vertices[i].x, map.vertices[i].y))
    };
    let (p1, p2) = (vertex(l.v1)?, vertex(l.v2)?);
    let (dx, dy) = (p2.0 - p1.0, p2.1 - p1.1);
    let len = dx.hypot(dy);
    if len == 0.0 {
        return None;
    }
    // Doom's front (right) side of a linedef lies along the normal
    // `(dy, -dx)`; flip it when `t` is the *back* sector, so the normal always
    // points into `t` and every distance below is positive on `t`'s side.
    let sign = if side_sector(map, l.sidefront) == Some(t) {
        1.0
    } else {
        -1.0
    };
    let nrm = (sign * dy / len, sign * -dx / len);
    let unit = (dx / len, dy / len);
    let perp = |p: (f64, f64)| (p.0 - p1.0) * nrm.0 + (p.1 - p1.1) * nrm.1;
    let along = |p: (f64, f64)| (p.0 - p1.0) * unit.0 + (p.1 - p1.1) * unit.1;

    let mut max_vertex = 0.0_f64;
    let mut nearest_blocking: Option<f64> = None;
    let t_floor = scene.sectors[t].floor;
    for b in &scene.sectors[t].boundary {
        max_vertex = max_vertex.max(perp(b.a)).max(perp(b.b));
        // The trigger line itself is at distance 0 and would win every
        // minimum; it is the edge being crossed, not an obstruction.
        if b.linedef == line_idx {
            continue;
        }
        let blocks = !b.two_sided
            || b.neighbor
                .is_some_and(|nb| scene.sectors[nb].floor - t_floor > step);
        if !blocks {
            continue;
        }
        // Only what stands across the line's own extent can stop a crossing of
        // it, so the projections must overlap in positive length. That is not
        // pedantry: a boundary perpendicular to the line projects to a single
        // point, and the far sector's side walls meet the line at its own
        // endpoints — admitting those would report distance 0 for every room
        // in the corpus. It is also the rule the brief's own worked example
        // needs: a 16-deep dead-end alcove must measure 16, which is its back
        // wall, not 0, which is where its side walls start.
        let (sa, sb) = (along(b.a), along(b.b));
        if sa.max(sb).min(len) - sa.min(sb).max(0.0) <= 0.0 {
            continue;
        }
        let d = perp(b.a).min(perp(b.b)).max(0.0);
        nearest_blocking = Some(nearest_blocking.map_or(d, |m: f64| m.min(d)));
    }
    Some((max_vertex, nearest_blocking))
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
        let placement = placement_of(plat, neighbors, Some(front), back);
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
        // No lift special reaches `P_ShootSpecialLine`, so the two lift
        // dispatchers are the use forms and the crossings.
        let dispatch = if USE_LIFT.contains(&l.special) {
            Dispatch::Use
        } else {
            Dispatch::Cross
        };
        let sides = activator_sides(map, scene, plat, plat_floor, i, step, dispatch);
        // For a walkover the player fires from below, measure the far sector
        // the crossing would land in — once per `Low` side, since a line in a
        // flat low room can have two.
        let mut low_crossings = Vec::new();
        let mut low_unmeasured = 0;
        if !USE_LIFT.contains(&l.special) {
            for &(from, class) in &sides {
                if class != Activator::Low {
                    continue;
                }
                let far = if from == front { back } else { Some(front) };
                match far
                    .filter(|&t| t != from)
                    .and_then(|t| far_depth(map, scene, i, t, step))
                {
                    Some((max_vertex, nearest_blocking)) => low_crossings.push(LowWalkCross {
                        linedef: i,
                        max_vertex,
                        nearest_blocking,
                    }),
                    None => low_unmeasured += 1,
                }
            }
        }
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
            low_crossings,
            low_unmeasured,
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

/// The axis-aligned bounding box of a sector's boundary as `(min corner,
/// width, height)`, or `None` when the scene resolved no boundary for it.
pub(crate) fn sector_bbox(scene: &Scene, sec: usize) -> Option<((i64, i64), i32, i32)> {
    let (mut west, mut south, mut east, mut north) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let boundary = &scene.sectors[sec].boundary;
    if boundary.is_empty() {
        return None;
    }
    for b in boundary {
        for (x, y) in [b.a, b.b] {
            west = west.min(x);
            south = south.min(y);
            east = east.max(x);
            north = north.max(y);
        }
    }
    Some((
        (round(west), round(south)),
        i32::try_from(round(east - west)).expect("map extents fit i32"),
        i32::try_from(round(north - south)).expect("map extents fit i32"),
    ))
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
    let bbox = sector_bbox(scene, plat);
    let mut neighbors: BTreeSet<usize> = BTreeSet::new();
    let (mut two, mut one, mut with_special, mut blocking) = (0, 0, 0, 0);
    let mut risers = Vec::new();
    let mut jambs = Vec::new();
    let mut two_sided_jambs = Vec::new();
    let mut top_faces = Vec::new();
    let (mut uppers_drawn, mut uppers_drawn_pegtop) = (0, 0);
    let mut light_eq_all = true;
    let mut ceiling_eq_all = true;
    for b in &ss.boundary {
        if !b.two_sided {
            one += 1;
            // `Scene::build` rejects a linedef whose `two_sided` flag and back
            // sidedef disagree, so a one-sided boundary is a genuine one-sided
            // linedef fronted by the plat: a jamb. The engine draws only its
            // middle slot.
            jambs.push(Jamb {
                texture: map.sidedefs[b.sidedef].texturemiddle.clone(),
                dontpegtop: b.upper_unpegged,
                dontpegbottom: b.lower_unpegged,
            });
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
        // The visible lower is on the sidedef whose sector has the lower floor,
        // and the visible upper on the one whose sector has the higher ceiling
        // (`r_segs.c`, `R_StoreWallRange`). `Scene` resolved this boundary, so
        // the plat's own sidedef is in range; the neighbor's is looked up
        // defensively all the same.
        let l = &map.linedefs[b.linedef];
        let nb_side = if side_sector(map, l.sidefront) == Some(n) {
            sidedef(map, l.sidefront)
        } else {
            l.sideback.and_then(|s| sidedef(map, s))
        };
        let nb = &scene.sectors[n];
        if nb.floor < ss.floor {
            if let Some(nb_side) = nb_side {
                risers.push(Riser {
                    texture: nb_side.texturebottom.clone(),
                    unpegged: b.lower_unpegged,
                    plat_side_nonblank: map.sidedefs[b.sidedef].texturebottom != "-",
                });
            }
        } else if nb.floor == ss.floor && nb.ceiling != ss.ceiling {
            // Neither a riser nor a landing under the plat's own ceiling: a
            // two-sided side wall, the shape a door track takes.
            two_sided_jambs.push(SideWall {
                texture: map.sidedefs[b.sidedef].texturemiddle.clone(),
                nb_ceiling_above: nb.ceiling > ss.ceiling,
            });
        }
        // A level landing under a higher ceiling: the plat is a shaft or
        // alcove, and the upper the engine draws is the landing's.
        if (nb.floor - ss.floor).abs() <= step
            && nb.ceiling > ss.ceiling
            && let Some(nb_side) = nb_side
        {
            top_faces.push(TopFace {
                texture: nb_side.texturetop.clone(),
                dontpegtop: b.upper_unpegged,
            });
        }
        // Every boundary of the plat on which the engine draws an upper at all.
        let drawn = match nb.ceiling.cmp(&ss.ceiling) {
            Ordering::Greater => nb_side.map(|s| s.texturetop.as_str()),
            Ordering::Less => Some(map.sidedefs[b.sidedef].texturetop.as_str()),
            Ordering::Equal => None,
        };
        if drawn.is_some_and(|t| t != "-") {
            uppers_drawn += 1;
            uppers_drawn_pegtop += usize::from(b.upper_unpegged);
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
        bbox_min: bbox.map_or((0, 0), |(min, _, _)| min),
        bbox_w: bbox.map_or(0, |(_, w, _)| w),
        bbox_h: bbox.map_or(0, |(_, _, h)| h),
        travel,
        max_nb_delta: max_nb - ss.floor,
        distinct_nb_floors,
        rest,
        shape,
        risers,
        jambs,
        two_sided_jambs,
        top_faces,
        uppers_drawn,
        uppers_drawn_pegtop,
        floor_flat: sec.texturefloor.clone(),
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

// ---------------------------------------------------------------------------
// Floor actions — the third pass (`floors`, Project G sub-project 4a).
//
// Every special number, destination formula and neighbor search below is
// transcribed from the pinned `linuxdoom-1.10` checkout at
// `a77dfb96cb91780ca334d0d4cfd86957558007e0`, and carries its `file:line`.
// Nothing here is written from memory.
// ---------------------------------------------------------------------------

/// How the engine fires a linedef's special.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum TriggerKind {
    /// Walked over — `P_CrossSpecialLine` (`p_spec.c:492`). No side gate: a
    /// side fires it when the crossing from that side is possible.
    Walk,
    /// Used — `P_UseSpecialLine` (`p_switch.c:276`), front side only:
    /// `p_switch.c:288` returns `false` from the back for every special but
    /// 124.
    Switch,
    /// Shot — `P_ShootSpecialLine` (`p_spec.c:959`).
    Gun,
}

impl TriggerKind {
    /// The letter the report prints.
    pub(crate) fn letter(self) -> &'static str {
        match self {
            Self::Walk => "W",
            Self::Switch => "S",
            Self::Gun => "G",
        }
    }
}

/// Where an "and change" family takes the flat it copies onto its target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum FlatSource {
    /// The line's front sector (`p_floor.c:368`; `p_plats.c:188` and `:202`
    /// take `sides[line->sidenum[0]].sector`, the same sector).
    LineFrontSector,
    /// A neighbor whose floor equals the destination, taken on arrival
    /// (`p_floor.c:413-437`, applied in `T_MoveFloor` `p_floor.c:241-244`).
    DestinationNeighbor,
}

/// The `floor_e` (`EV_DoFloor`, `p_floor.c:259-444`) or `plattype_e`
/// (`EV_DoPlat`, `p_plats.c:138-262`) an engine floor special dispatches to.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum FloorType {
    /// `lowerFloor` — down to `P_FindHighestFloorSurrounding`
    /// (`p_floor.c:291-297`).
    LowerFloor,
    /// `lowerFloorToLowest` — down to `P_FindLowestFloorSurrounding`
    /// (`p_floor.c:299-305`).
    LowerFloorToLowest,
    /// `turboLower` — the highest neighboring floor, `+ 8` only when that
    /// differs from the sector's own floor (`p_floor.c:307-315`).
    TurboLower,
    /// `raiseFloor` — up to `P_FindLowestCeilingSurrounding`, capped at the
    /// sector's own ceiling (`p_floor.c:319-329`).
    RaiseFloor,
    /// `raiseFloorCrush` — `raiseFloor`'s destination minus 8, applied after
    /// the cap (`p_floor.c:317-329`).
    RaiseFloorCrush,
    /// `raiseFloorToNearest` — `P_FindNextHighestFloor` (`p_floor.c:339-345`).
    RaiseFloorToNearest,
    /// `raiseFloorTurbo` — the same destination at `FLOORSPEED*4`
    /// (`p_floor.c:331-337`).
    RaiseFloorTurbo,
    /// `raiseFloor24` — `floorheight + 24` (`p_floor.c:347-353`).
    RaiseFloor24,
    /// `raiseFloor24AndChange` — `+ 24`, and at start the line's front
    /// sector's flat and special (`p_floor.c:362-370`).
    RaiseFloor24AndChange,
    /// `raiseFloor512` — `floorheight + 512` (`p_floor.c:354-360`).
    RaiseFloor512,
    /// `raiseToTexture` — `floorheight` plus the least bottom-texture height
    /// over the sector's two-sided lines (`p_floor.c:372-401`). The probe
    /// reads no texture heights, so this destination stays unresolved.
    RaiseToTexture,
    /// `lowerAndChange` — `P_FindLowestFloorSurrounding`, and on arrival the
    /// flat and special of a neighbor at the destination height
    /// (`p_floor.c:403-438`).
    LowerAndChange,
    /// Plat `raiseAndChange` with `amount = 24` (`p_plats.c:199-207`).
    PlatRaiseAndChange24,
    /// Plat `raiseAndChange` with `amount = 32` (`p_plats.c:199-207`).
    PlatRaiseAndChange32,
    /// Plat `raiseToNearestAndChange` — `P_FindNextHighestFloor`, and at
    /// start the front sector's flat with `sec->special = 0`
    /// (`p_plats.c:185-197`).
    PlatRaiseToNearestAndChange,
}

impl FloorType {
    /// The engine type's own name, as the report prints it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::LowerFloor => "lowerFloor",
            Self::LowerFloorToLowest => "lowerFloorToLowest",
            Self::TurboLower => "turboLower",
            Self::RaiseFloor => "raiseFloor",
            Self::RaiseFloorCrush => "raiseFloorCrush",
            Self::RaiseFloorToNearest => "raiseFloorToNearest",
            Self::RaiseFloorTurbo => "raiseFloorTurbo",
            Self::RaiseFloor24 => "raiseFloor24",
            Self::RaiseFloor24AndChange => "raiseFloor24AndChange",
            Self::RaiseFloor512 => "raiseFloor512",
            Self::RaiseToTexture => "raiseToTexture",
            Self::LowerAndChange => "lowerAndChange",
            Self::PlatRaiseAndChange24 => "plat raiseAndChange +24",
            Self::PlatRaiseAndChange32 => "plat raiseAndChange +32",
            Self::PlatRaiseToNearestAndChange => "plat raiseToNearestAndChange",
        }
    }

    /// Whether the thinker runs upward (`floor->direction = 1`). Every
    /// `raise*` and every `raiseAndChange` plat does; the three `lower*`
    /// families do not.
    pub(crate) fn raises(self) -> bool {
        !matches!(
            self,
            Self::LowerFloor | Self::LowerFloorToLowest | Self::TurboLower | Self::LowerAndChange
        )
    }

    /// Where this family takes the flat it copies, if it copies one.
    pub(crate) fn flat_source(self) -> Option<FlatSource> {
        match self {
            Self::RaiseFloor24AndChange
            | Self::PlatRaiseAndChange24
            | Self::PlatRaiseAndChange32
            | Self::PlatRaiseToNearestAndChange => Some(FlatSource::LineFrontSector),
            Self::LowerAndChange => Some(FlatSource::DestinationNeighbor),
            _ => None,
        }
    }
}

/// One dispatchable form of a floor family: `(special, how it fires,
/// whether it can fire again)`.
pub(crate) type FloorSpecial = (i32, TriggerKind, bool);

/// One family of the §1.1 dispatch table: its engine type and its forms.
pub(crate) type FloorFamily = (FloorType, &'static [FloorSpecial]);

/// `EV_DoFloor(lowerFloor)`: `p_spec.c:609-611` (19, W1) and `:837-839`
/// (83, WR); `p_switch.c:449-451` (102, S1) and `:526-528` (45, SR).
pub(crate) const LOWER_FLOOR: [FloorSpecial; 4] = [
    (19, TriggerKind::Walk, false),
    (83, TriggerKind::Walk, true),
    (102, TriggerKind::Switch, false),
    (45, TriggerKind::Switch, true),
];

/// `EV_DoFloor(lowerFloorToLowest)`: `p_spec.c:652-654` (38, W1),
/// `:664-669` (40, W1 — `RaiseCeilingLowerFloor`, which also calls
/// `EV_DoCeiling(line, raiseToHighest)`) and `:832-834` (82, WR);
/// `p_switch.c:395-398` (23, S1) and `:532-535` (60, SR).
pub(crate) const LOWER_FLOOR_TO_LOWEST: [FloorSpecial; 5] = [
    (38, TriggerKind::Walk, false),
    (40, TriggerKind::Walk, false),
    (82, TriggerKind::Walk, true),
    (23, TriggerKind::Switch, false),
    (60, TriggerKind::Switch, true),
];

/// The one special in [`LOWER_FLOOR_TO_LOWEST`] that also raises the ceiling
/// (`p_spec.c:664-669`: `EV_DoCeiling(line, raiseToHighest)` then
/// `EV_DoFloor(line, lowerFloorToLowest)`).
pub(crate) const RAISE_CEILING_LOWER_FLOOR: i32 = 40;

/// `EV_DoFloor(turboLower)`: `p_spec.c:640-642` (36, W1) and `:909-911`
/// (98, WR); `p_switch.c:413-415` (71, S1) and `:592-594` (70, SR).
pub(crate) const TURBO_LOWER: [FloorSpecial; 4] = [
    (36, TriggerKind::Walk, false),
    (98, TriggerKind::Walk, true),
    (71, TriggerKind::Switch, false),
    (70, TriggerKind::Switch, true),
];

/// `EV_DoFloor(raiseFloor)`: `p_spec.c:561-563` (5, W1), `:872-874`
/// (91, WR) and `:982-985` (24, G1, `P_ShootSpecialLine`);
/// `p_switch.c:443-445` (101, S1) and `:556-558` (64, SR).
pub(crate) const RAISE_FLOOR: [FloorSpecial; 5] = [
    (5, TriggerKind::Walk, false),
    (91, TriggerKind::Walk, true),
    (101, TriggerKind::Switch, false),
    (64, TriggerKind::Switch, true),
    (24, TriggerKind::Gun, false),
];

/// `EV_DoFloor(raiseFloorCrush)`: `p_spec.c:694-696` (56, W1) and
/// `:887-889` (94, WR); `p_switch.c:437-439` (55, S1) and `:574-577`
/// (65, SR).
pub(crate) const RAISE_FLOOR_CRUSH: [FloorSpecial; 4] = [
    (56, TriggerKind::Walk, false),
    (94, TriggerKind::Walk, true),
    (55, TriggerKind::Switch, false),
    (65, TriggerKind::Switch, true),
];

/// `EV_DoFloor(raiseFloorToNearest)`: `p_spec.c:748-750` (119, W1) and
/// `:940-942` (128, WR); `p_switch.c:377-380` (18, S1) and `:586-589`
/// (69, SR).
pub(crate) const RAISE_FLOOR_TO_NEAREST: [FloorSpecial; 4] = [
    (119, TriggerKind::Walk, false),
    (128, TriggerKind::Walk, true),
    (18, TriggerKind::Switch, false),
    (69, TriggerKind::Switch, true),
];

/// `EV_DoFloor(raiseFloorTurbo)`: `p_spec.c:774-776` (130, W1) and
/// `:945-947` (129, WR); `p_switch.c:491-493` (131, S1) and `:622-624`
/// (132, SR).
pub(crate) const RAISE_FLOOR_TURBO: [FloorSpecial; 4] = [
    (130, TriggerKind::Walk, false),
    (129, TriggerKind::Walk, true),
    (131, TriggerKind::Switch, false),
    (132, TriggerKind::Switch, true),
];

/// `EV_DoFloor(raiseFloor24)`: `p_spec.c:706-708` (58, W1) and `:877-879`
/// (92, WR). No switch form exists.
pub(crate) const RAISE_FLOOR_24: [FloorSpecial; 2] = [
    (58, TriggerKind::Walk, false),
    (92, TriggerKind::Walk, true),
];

/// `EV_DoFloor(raiseFloor24AndChange)`: `p_spec.c:712-714` (59, W1) and
/// `:882-884` (93, WR). No switch form exists.
pub(crate) const RAISE_FLOOR_24_AND_CHANGE: [FloorSpecial; 2] = [
    (59, TriggerKind::Walk, false),
    (93, TriggerKind::Walk, true),
];

/// `EV_DoFloor(raiseFloor512)`: `p_switch.c:507-510` (140, S1). The only
/// form the engine dispatches.
pub(crate) const RAISE_FLOOR_512: [FloorSpecial; 1] = [(140, TriggerKind::Switch, false)];

/// `EV_DoFloor(raiseToTexture)`: `p_spec.c:627-630` (30, W1) and `:898-901`
/// (96, WR).
pub(crate) const RAISE_TO_TEXTURE: [FloorSpecial; 2] = [
    (30, TriggerKind::Walk, false),
    (96, TriggerKind::Walk, true),
];

/// `EV_DoFloor(lowerAndChange)`: `p_spec.c:646-648` (37, W1) and `:842-844`
/// (84, WR).
pub(crate) const LOWER_AND_CHANGE: [FloorSpecial; 2] = [
    (37, TriggerKind::Walk, false),
    (84, TriggerKind::Walk, true),
];

/// `EV_DoPlat(raiseAndChange, 24)`: `p_switch.c:371-374` (15, S1) and
/// `:562-565` (66, SR).
pub(crate) const PLAT_RAISE_AND_CHANGE_24: [FloorSpecial; 2] = [
    (15, TriggerKind::Switch, false),
    (66, TriggerKind::Switch, true),
];

/// `EV_DoPlat(raiseAndChange, 32)`: `p_switch.c:365-368` (14, S1) and
/// `:568-571` (67, SR).
pub(crate) const PLAT_RAISE_AND_CHANGE_32: [FloorSpecial; 2] = [
    (14, TriggerKind::Switch, false),
    (67, TriggerKind::Switch, true),
];

/// `EV_DoPlat(raiseToNearestAndChange, 0)`: `p_spec.c:615-617` (22, W1),
/// `:892-895` (95, WR) and `:994-997` (47, G1, `P_ShootSpecialLine`);
/// `p_switch.c:383-386` (20, S1) and `:580-583` (68, SR).
pub(crate) const PLAT_RAISE_TO_NEAREST_AND_CHANGE: [FloorSpecial; 5] = [
    (22, TriggerKind::Walk, false),
    (95, TriggerKind::Walk, true),
    (20, TriggerKind::Switch, false),
    (68, TriggerKind::Switch, true),
    (47, TriggerKind::Gun, false),
];

/// Every floor family, in the order the report prints them.
pub(crate) const FLOOR_FAMILIES: [FloorFamily; 15] = [
    (FloorType::LowerFloor, &LOWER_FLOOR),
    (FloorType::LowerFloorToLowest, &LOWER_FLOOR_TO_LOWEST),
    (FloorType::TurboLower, &TURBO_LOWER),
    (FloorType::RaiseFloor, &RAISE_FLOOR),
    (FloorType::RaiseFloorCrush, &RAISE_FLOOR_CRUSH),
    (FloorType::RaiseFloorToNearest, &RAISE_FLOOR_TO_NEAREST),
    (FloorType::RaiseFloorTurbo, &RAISE_FLOOR_TURBO),
    (FloorType::RaiseFloor24, &RAISE_FLOOR_24),
    (FloorType::RaiseFloor24AndChange, &RAISE_FLOOR_24_AND_CHANGE),
    (FloorType::RaiseFloor512, &RAISE_FLOOR_512),
    (FloorType::RaiseToTexture, &RAISE_TO_TEXTURE),
    (FloorType::LowerAndChange, &LOWER_AND_CHANGE),
    (FloorType::PlatRaiseAndChange24, &PLAT_RAISE_AND_CHANGE_24),
    (FloorType::PlatRaiseAndChange32, &PLAT_RAISE_AND_CHANGE_32),
    (
        FloorType::PlatRaiseToNearestAndChange,
        &PLAT_RAISE_TO_NEAREST_AND_CHANGE,
    ),
];

/// The two floor specials `P_ShootSpecialLine` dispatches (`p_spec.c:982`
/// and `:994`) — the gun forms, which the v1 gate refuses.
pub(crate) const FLOOR_GUN: [i32; 2] = [24, 47];

/// `EV_BuildStairs`: `p_spec.c:573-575` (8, W1 `build8`) and `:736-738`
/// (100, W1 `turbo16`); `p_switch.c:347-350` (7, S1 `build8`) and
/// `:485-488` (127, S1 `turbo16`). An adjacent family: counted, not modeled.
pub(crate) const STAIRS: [i32; 4] = [8, 100, 7, 127];

/// `EV_DoDonut`: `p_switch.c:353-356` (9, S1). An adjacent family: counted,
/// not modeled.
pub(crate) const DONUT: [i32; 1] = [9];

/// Every special dispatching `EV_DoCeiling`, read off the two dispatch
/// switches rather than recalled: `p_spec.c:567-569` (6, W1), `:621-623`
/// (25, W1), `:664-667` (40, W1 — also a floor lower), `:671-673` (44, W1),
/// `:780-782` (141, W1), `:787-789` (72, WR), `:792-794` (73, WR),
/// `:812-814` (77, WR); `p_switch.c:407-410` (41, S1), `:419-422` (49, S1),
/// `:520-522` (43, SR). The crusher-stop specials (57, 74) call
/// `EV_CeilingCrushStop`, not `EV_DoCeiling`, and are not in this set.
pub(crate) const CEILING: [i32; 11] = [6, 25, 40, 44, 141, 72, 73, 77, 41, 49, 43];

/// Every special in the [`FLOOR_FAMILIES`] table, gun forms included. Derived
/// from the table rather than restated, so the two can never disagree.
pub(crate) fn floor_all() -> BTreeSet<i32> {
    FLOOR_FAMILIES
        .iter()
        .flat_map(|(_, specials)| specials.iter().map(|&(special, _, _)| special))
        .collect()
}

/// The engine type, trigger kind and repeatability `special` dispatches, or
/// `None` when it is not a floor line.
pub(crate) fn floor_type(special: i32) -> Option<(FloorType, TriggerKind, bool)> {
    FLOOR_FAMILIES.iter().find_map(|&(ty, specials)| {
        specials
            .iter()
            .find(|&&(s, _, _)| s == special)
            .map(|&(_, kind, repeatable)| (ty, kind, repeatable))
    })
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
    if b.two_sided { b.neighbor } else { None }
}

/// `P_FindLowestFloorSurrounding` (`p_spec.c:270-291`): starts at the
/// sector's **own** floor, minimum over two-sided neighbors.
pub(crate) fn lowest_floor_surrounding(scene: &Scene, sec: usize) -> i32 {
    sector_lines(scene, sec)
        .iter()
        .filter_map(|b| next_sector(b))
        .fold(scene.sectors[sec].floor, |lo, n| {
            lo.min(scene.sectors[n].floor)
        })
}

/// `P_FindHighestFloorSurrounding`'s starting value, `-500*FRACUNIT`
/// (`p_spec.c:303`). A sector with no two-sided neighbor "lowers" to it.
pub(crate) const NO_NEIGHBOR_FLOOR: i32 = -500;

/// `P_FindHighestFloorSurrounding` (`p_spec.c:297-318`): starts at
/// [`NO_NEIGHBOR_FLOOR`], maximum over two-sided neighbors.
pub(crate) fn highest_floor_surrounding(scene: &Scene, sec: usize) -> i32 {
    sector_lines(scene, sec)
        .iter()
        .filter_map(|b| next_sector(b))
        .fold(NO_NEIGHBOR_FLOOR, |hi, n| hi.max(scene.sectors[n].floor))
}

/// `MAX_ADJOINING_SECTORS` (`p_spec.c:326`).
pub(crate) const MAX_ADJOINING_SECTORS: usize = 20;

/// What [`next_highest_floor`] found.
pub(crate) struct NextHighest {
    /// The least neighboring floor strictly above `currentheight`, or
    /// `currentheight` when no neighbor is above it (`p_spec.c:361-362`).
    pub(crate) height: i32,
    /// Whether the search filled its 20-entry list and broke early
    /// (`p_spec.c:349-355`) — the map is then reading a truncated
    /// neighborhood, and the destination may not be the true next height.
    pub(crate) capped: bool,
}

/// `P_FindNextHighestFloor(sec, currentheight)` (`p_spec.c:329-375`),
/// including its 20-entry cap: candidates are collected in the sector's own
/// line order and the loop breaks once the list is full.
pub(crate) fn next_highest_floor(scene: &Scene, sec: usize, current: i32) -> NextHighest {
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
pub(crate) fn lowest_ceiling_surrounding(scene: &Scene, sec: usize) -> i32 {
    sector_lines(scene, sec)
        .iter()
        .filter_map(|b| next_sector(b))
        .fold(i32::MAX, |lo, n| lo.min(scene.sectors[n].ceiling))
}

/// Where a floor action sends its target, evaluated at load-time heights.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Destination {
    /// A resolved height.
    Height(i32),
    /// `raiseToTexture` (`p_floor.c:372-401`): the destination is the least
    /// bottom-texture height on the sector's two-sided lines, and the probe
    /// reads no texture heights.
    NeedsTexture,
}

/// `raiseFloor`'s destination: `P_FindLowestCeilingSurrounding` capped at the
/// sector's own ceiling (`p_floor.c:322-326`).
fn raise_floor_destination(scene: &Scene, target: usize) -> i32 {
    lowest_ceiling_surrounding(scene, target).min(scene.sectors[target].ceiling)
}

/// The `floordestheight` / `plat->high` the engine would compute for
/// `target` under `ty`, at the heights the map loads with.
pub(crate) fn destination(scene: &Scene, target: usize, ty: FloorType) -> Destination {
    let floor = scene.sectors[target].floor;
    let height = match ty {
        FloorType::LowerFloor => highest_floor_surrounding(scene, target),
        FloorType::LowerFloorToLowest | FloorType::LowerAndChange => {
            lowest_floor_surrounding(scene, target)
        }
        FloorType::TurboLower => {
            // `p_floor.c:313-314`: `+ 8` only when the destination differs
            // from the sector's current floor.
            let high = highest_floor_surrounding(scene, target);
            if high == floor { high } else { high + 8 }
        }
        FloorType::RaiseFloor => raise_floor_destination(scene, target),
        // `p_floor.c:327-328`: the `- 8` is applied after the ceiling cap.
        FloorType::RaiseFloorCrush => raise_floor_destination(scene, target) - 8,
        FloorType::RaiseFloorToNearest
        | FloorType::RaiseFloorTurbo
        | FloorType::PlatRaiseToNearestAndChange => next_highest_floor(scene, target, floor).height,
        FloorType::RaiseFloor24
        | FloorType::RaiseFloor24AndChange
        | FloorType::PlatRaiseAndChange24 => floor + 24,
        FloorType::PlatRaiseAndChange32 => floor + 32,
        FloorType::RaiseFloor512 => floor + 512,
        FloorType::RaiseToTexture => return Destination::NeedsTexture,
    };
    Destination::Height(height)
}

/// The floor heights one evaluation runs at: the map's, except the target's,
/// which is `target_floor` — its rest height before the action fires, its
/// destination after. Ceilings never move here.
#[derive(Clone, Copy)]
pub(crate) struct Heights {
    /// The moving sector.
    pub(crate) target: usize,
    /// The moving sector's floor in this evaluation.
    pub(crate) target_floor: i32,
}

impl Heights {
    /// Sector `s`'s floor under these heights.
    pub(crate) fn floor(self, scene: &Scene, s: usize) -> i32 {
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
pub(crate) fn standable(scene: &Scene, h: Heights, s: usize, player_height: i32) -> bool {
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
pub(crate) fn pass(
    scene: &Scene,
    h: Heights,
    a: usize,
    b: usize,
    player_height: i32,
    step: i32,
) -> bool {
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
pub(crate) fn local_adjacency(
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

/// `reach_X(A)` for every member of the local graph: the members reachable
/// from `A` through directed [`pass`] edges inside the graph, excluding `A`
/// itself and the target (which may be a *via*, never a destination). A
/// member that is not [`standable`] reaches nothing.
pub(crate) fn reach_sets(
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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Effect {
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
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Rider {
    /// Nobody can be standing on the target when it fires.
    NotApplicable,
    /// The rider can still reach everything it could before.
    Keeps,
    /// The rider loses a destination — it may be stranded.
    Loses,
}

/// Which opening a floor action carves, when it opens one.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum OpeningShape {
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

/// The classification of one `(family, target)` action.
#[expect(
    clippy::struct_excessive_bools,
    reason = "each bool is an independent measured fact about the action (what the target is \
              reachable from before and after, what the destination coincides with, whether the \
              neighbors were joined anyway); they encode no joint state"
)]
pub(crate) struct EffectFacts {
    /// The effect on everyone but a rider.
    pub(crate) effect: Effect,
    /// The effect on a rider.
    pub(crate) rider: Rider,
    /// The opening sub-shape: an [`Effect::Opening`] whose rider is not
    /// stranded, or an [`Effect::Neutral`] that is an
    /// [`OpeningShape::Reveal`].
    pub(crate) opening: Option<OpeningShape>,
    /// Whether some neighbor can walk onto the target at its rest floor.
    pub(crate) enterable_before: bool,
    /// Whether some neighbor can walk onto the target at its destination.
    pub(crate) enterable_after: bool,
    /// Whether two distinct neighbors could already reach each other, both
    /// ways, inside the local graph before the action fired — so the target
    /// was not the only route between them.
    pub(crate) neighbors_already_connected: bool,
    /// Whether the destination is exactly some neighbor's floor.
    pub(crate) joins_neighbor_floor: bool,
    /// `(A, B)` pairs `B` newly reachable from `A`.
    pub(crate) new_pairs: usize,
    /// Whether some new pair is matched by its reverse.
    pub(crate) new_pair_bidirectional: bool,
}

/// Classifies the action that moves `target`'s floor from `rest` to `dest`,
/// against the local graph `{target} ∪ neighbors` at load-time heights.
pub(crate) fn classify_effect(
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
        new_pair_bidirectional: new_pairs.iter().any(|&(a, b)| new_pairs.contains(&(b, a))),
        new_pairs: new_pairs.len(),
    }
}

/// Hops from `from` to every sector reachable over two-sided adjacency,
/// ignoring heights — the trigger-placement measure of §E.
pub(crate) fn hop_distances(scene: &Scene, from: usize) -> BTreeMap<usize, usize> {
    let mut dist: BTreeMap<usize, usize> = BTreeMap::from([(from, 0)]);
    let mut frontier = vec![from];
    let mut depth = 0;
    while !frontier.is_empty() {
        depth += 1;
        let mut next = Vec::new();
        for s in frontier {
            for b in &scene.sectors[s].boundary {
                let Some(n) = next_sector(b) else { continue };
                if let btree_map::Entry::Vacant(slot) = dist.entry(n) {
                    slot.insert(depth);
                    next.push(n);
                }
            }
        }
        frontier = next;
    }
    dist
}

/// The sectors a trigger line can be fired from — [`activator_sides`]
/// without the classification, for callers that only need the placement.
pub(crate) fn trigger_sides(
    map: &UdmfMap,
    scene: &Scene,
    target: usize,
    line_idx: usize,
    step: i32,
    dispatch: Dispatch,
) -> Vec<usize> {
    let floor = scene.sectors[target].floor;
    activator_sides(map, scene, target, floor, line_idx, step, dispatch)
        .into_iter()
        .map(|(s, _)| s)
        .collect()
}

/// One floor line naming a target's tag.
pub(crate) struct FloorTrigger {
    /// The linedef's special.
    pub(crate) special: i32,
    /// The engine type it dispatches.
    pub(crate) ty: FloorType,
    /// How it is fired.
    pub(crate) kind: TriggerKind,
    /// Whether it can fire more than once.
    pub(crate) repeatable: bool,
    /// Where it sits relative to the target.
    pub(crate) placement: Placement,
    /// The activator classes, deduplicated; `[Activator::None]` when no side
    /// can fire it.
    pub(crate) activators: Vec<Activator>,
    /// The `SW1*`/`SW2*` textures on the front sidedef, by slot.
    pub(crate) switch_slots: Vec<(&'static str, String)>,
    /// A line whose two sides name the same sector — a trip line drawn inside
    /// one room rather than a portal crossing.
    pub(crate) same_sector: bool,
    /// The line's front sector, which the "and change" families copy a flat
    /// from. `None` when the front side or its sector reference dangles.
    pub(crate) front_sector: Option<usize>,
    /// Hops from the nearest activator sector to the target, `None` when no
    /// activator sector reaches it over two-sided adjacency.
    pub(crate) hops: Option<usize>,
}

/// Every floor line naming `target`'s tag, as one trigger each. A tag-0
/// target has none: `P_FindSectorFromLineTag` matches tags by equality, and
/// the probe never resolves a tag-0 line against untagged sectors.
pub(crate) fn floor_triggers(
    map: &UdmfMap,
    scene: &Scene,
    target: usize,
    neighbors: &BTreeSet<usize>,
    hops: &BTreeMap<usize, usize>,
    step: i32,
) -> Vec<FloorTrigger> {
    let tag = map.sectors[target].id;
    if tag == 0 {
        return Vec::new();
    }
    let target_floor = scene.sectors[target].floor;
    let mut triggers = Vec::new();
    for (i, l) in map.linedefs.iter().enumerate() {
        if l.args[0] != tag {
            continue;
        }
        let Some((ty, kind, repeatable)) = floor_type(l.special) else {
            continue;
        };
        let front = side_sector(map, l.sidefront);
        let back = l.sideback.and_then(|b| side_sector(map, b));
        let sides = activator_sides(
            map,
            scene,
            target,
            target_floor,
            i,
            step,
            Dispatch::of(kind),
        );
        let mut switch_slots = Vec::new();
        // `P_ChangeSwitchTexture` rewrites the **front** sidedef, and both
        // the use forms and the two gun floor specials call it
        // (`p_switch.c:284-297`; `p_spec.c:955-1000`), so a walkover is the
        // only kind with no switch face to read.
        if kind != TriggerKind::Walk
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
        triggers.push(FloorTrigger {
            special: l.special,
            ty,
            kind,
            repeatable,
            placement: placement_of(target, neighbors, front, back),
            activators: activators(&sides),
            switch_slots,
            same_sector: back.is_some() && back == front,
            front_sector: front,
            hops: sides
                .iter()
                .filter_map(|&(s, _)| hops.get(&s).copied())
                .min(),
        });
    }
    triggers
}

/// Synthetic-map builders and the fixture type the probe's test modules
/// share: rows of rooms (`chain`, `chain_full`), a T (`tee`), an L whose two
/// arms also touch each other (`panel`), and a lift shaft (`shaft`).
#[cfg(test)]
pub(crate) mod tests {
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
    pub(crate) fn chain(
        floors: &[i32],
        tags: &[i32],
        links: &[(i32, i32, bool)],
        extra: &str,
    ) -> String {
        chain_full(floors, &vec![256; floors.len()], tags, links, extra)
    }

    /// [`chain`] with a ceiling per room, for the fixtures whose question is
    /// headroom rather than floor height.
    pub(crate) fn chain_full(
        floors: &[i32],
        ceilings: &[i32],
        tags: &[i32],
        links: &[(i32, i32, bool)],
        extra: &str,
    ) -> String {
        let n = floors.len();
        assert_eq!(ceilings.len(), n);
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
        for ((floor, ceiling), tag) in floors.iter().zip(ceilings).zip(tags) {
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = {floor}; heightceiling = {ceiling}; lightlevel = 160; id = {tag}; }}"
            );
        }
        text.push_str(extra);
        text
    }

    /// Four rooms: A (sector 0), T (sector 1) and B (sector 2) in a row of
    /// 128×128 boxes at `y ∈ [0, 128]`, plus a pit P (sector 3) south of T at
    /// `y ∈ [-128, 0]`. `links` gives `(special, tag)` for the three
    /// two-sided boundaries A|T, T|B and T|P, whose front sectors are A, T
    /// and T. Every two-sided sidedef carries `SUPPORT3` as its lower and
    /// every one-sided wall `STARTAN2`; `extra` is appended verbatim.
    pub(crate) fn tee(
        floors: &[i32; 4],
        ceilings: &[i32; 4],
        tags: &[i32; 4],
        links: &[(i32, i32); 3],
        extra: &str,
    ) -> String {
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
            (128, -128),
            (256, -128),
        ] {
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = {y}.000; }}");
        }
        // The three shared boundaries, then the one-sided perimeter walls of
        // each room, each wound so its front (right) side faces inward.
        for (i, &(v1, v2)) in [(3, 2), (5, 4), (4, 2)].iter().enumerate() {
            let (special, tag) = links[i];
            let (sf, sb) = (2 * i, 2 * i + 1);
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; sideback = {sb}; twosided = true; special = {special}; arg0 = {tag}; }}"
            );
        }
        for (i, &(v1, v2)) in [
            (0, 1),
            (1, 3),
            (2, 0),
            (3, 5),
            (5, 7),
            (7, 6),
            (6, 4),
            (8, 2),
            (4, 9),
            (9, 8),
        ]
        .iter()
        .enumerate()
        {
            let s = 6 + i;
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {s}; blocking = true; }}"
            );
        }
        for sector in [0, 1, 1, 2, 1, 3, 0, 0, 0, 1, 2, 2, 2, 3, 3, 3] {
            let _ = writeln!(text, "sidedef {{ sector = {sector}; }}");
        }
        for ((floor, ceiling), tag) in floors.iter().zip(ceilings).zip(tags) {
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = {floor}; heightceiling = {ceiling}; lightlevel = 160; id = {tag}; }}"
            );
        }
        text.push_str(extra);
        text
    }

    /// Three rooms in an L: A (sector 0) at `x ∈ [0, 128]`, B (sector 1) at
    /// `x ∈ [128, 256]`, both at `y ∈ [0, 128]` and sharing the wall at
    /// `x = 128`; T (sector 2) spans `x ∈ [0, 256]` at `y ∈ [128, 256]`,
    /// bordering both. Unlike [`chain`] and [`tee`], the target's two
    /// neighbors are *also* neighbors of each other, which is what separates
    /// a panel that reveals itself from a wall that joins two rooms.
    /// `links` gives `(special, tag)` for A|T, B|T and A|B, whose front
    /// sectors are A, B and A.
    pub(crate) fn panel(
        floors: &[i32; 3],
        ceilings: &[i32; 3],
        tags: &[i32; 3],
        links: &[(i32, i32); 3],
        extra: &str,
    ) -> String {
        let mut text = String::from("namespace = \"doom\";\n");
        for (x, y) in [
            (0, 0),
            (128, 0),
            (256, 0),
            (0, 128),
            (128, 128),
            (256, 128),
            (0, 256),
            (256, 256),
        ] {
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = {y}.000; }}");
        }
        for (i, &(v1, v2)) in [(4, 3), (5, 4), (4, 1)].iter().enumerate() {
            let (special, tag) = links[i];
            let (sf, sb) = (2 * i, 2 * i + 1);
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; sideback = {sb}; twosided = true; special = {special}; arg0 = {tag}; }}"
            );
        }
        for (i, &(v1, v2)) in [(0, 3), (1, 0), (2, 1), (5, 2), (3, 6), (6, 7), (7, 5)]
            .iter()
            .enumerate()
        {
            let s = 6 + i;
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {s}; blocking = true; }}"
            );
        }
        for sector in [0, 2, 1, 2, 0, 1, 0, 0, 1, 1, 2, 2, 2] {
            let _ = writeln!(text, "sidedef {{ sector = {sector}; }}");
        }
        for ((floor, ceiling), tag) in floors.iter().zip(ceilings).zip(tags) {
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = {floor}; heightceiling = {ceiling}; lightlevel = 160; id = {tag}; }}"
            );
        }
        text.push_str(extra);
        text
    }

    /// A parsed synthetic map and the scene built from it, shared by the
    /// probe's test modules.
    pub(crate) struct Fixture {
        /// The map as parsed from the UDMF text.
        pub(crate) map: UdmfMap,
        /// The scene `Scene::build` resolved from [`Fixture::map`].
        pub(crate) scene: Scene,
        /// The tables' step height, so a test never spells `24` itself.
        pub(crate) step: i32,
    }

    /// Parses `text` as UDMF and builds its scene under the shipped tables.
    pub(crate) fn fixture(text: &str) -> Fixture {
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

    /// A three-room shaft: room 0 is the low room the lift is called from
    /// (floor 0, ceiling 256), room 1 the plat (floor 64, ceiling
    /// `plat_ceiling`, tag 7, flat `STEP1`), room 2 the landing at the plat's
    /// own floor under `landing_ceiling`. Textures are chosen so every slot
    /// the measures read is distinguishable from every other: the riser's
    /// visible lower is `SUPPORT2` on room 0's sidedef, the plat's own lower
    /// is blank; the landing's upper is `BIGDOOR2` and the plat's own upper on
    /// that same line is `PLATSIDE`, so reading the wrong side is visible; the
    /// plat's two one-sided jambs are `DOORTRAK` (`dontpegtop`) and `STARTAN2`
    /// (`dontpegbottom`); the low room's upper over the shaft is `STARTAN3`,
    /// on a line with neither pegging flag.
    fn shaft(plat_ceiling: i32, landing_ceiling: i32) -> String {
        let mut text = String::from("namespace = \"doom\";\n");
        for i in 0..4 {
            let x = i * 128;
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = 0.000; }}");
            let _ = writeln!(text, "vertex {{ x = {x}.000; y = 128.000; }}");
        }
        // The riser line: front is the low room, so a use fires from below.
        text.push_str(
            "linedef { v1 = 3; v2 = 2; sidefront = 0; sideback = 1; twosided = true; special = 62; arg0 = 7; }\n",
        );
        // The landing line: front is the plat, back the landing, upper-unpegged.
        text.push_str(
            "linedef { v1 = 5; v2 = 4; sidefront = 2; sideback = 3; twosided = true; dontpegtop = true; }\n",
        );
        // The plat's two one-sided jambs.
        text.push_str(
            "linedef { v1 = 3; v2 = 5; sidefront = 4; blocking = true; dontpegtop = true; }\n",
        );
        text.push_str(
            "linedef { v1 = 4; v2 = 2; sidefront = 5; blocking = true; dontpegbottom = true; }\n",
        );
        // The outer walls of rooms 0 and 2, closing both loops.
        for (v1, v2, side) in [
            (0, 1, 6),
            (1, 3, 7),
            (2, 0, 8),
            (5, 7, 9),
            (7, 6, 10),
            (6, 4, 11),
        ] {
            let _ = writeln!(
                text,
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {side}; blocking = true; }}"
            );
        }
        for sd in [
            "sector = 0; texturetop = \"STARTAN3\"; texturemiddle = \"-\"; texturebottom = \"SUPPORT2\";",
            "sector = 1; texturetop = \"-\"; texturemiddle = \"-\"; texturebottom = \"-\";",
            "sector = 1; texturetop = \"PLATSIDE\"; texturemiddle = \"-\"; texturebottom = \"-\";",
            "sector = 2; texturetop = \"BIGDOOR2\"; texturemiddle = \"-\"; texturebottom = \"-\";",
            "sector = 1; texturemiddle = \"DOORTRAK\";",
            "sector = 1; texturemiddle = \"STARTAN2\";",
            "sector = 0; texturemiddle = \"STARTAN2\";",
            "sector = 0; texturemiddle = \"STARTAN2\";",
            "sector = 0; texturemiddle = \"STARTAN2\";",
            "sector = 2; texturemiddle = \"STARTAN2\";",
            "sector = 2; texturemiddle = \"STARTAN2\";",
            "sector = 2; texturemiddle = \"STARTAN2\";",
        ] {
            let _ = writeln!(text, "sidedef {{ {sd} }}");
        }
        for (flat, floor, ceiling, tag) in [
            ("FLOOR4_8", 0, 256, 0),
            ("STEP1", 64, plat_ceiling, 7),
            ("FLOOR4_8", 64, landing_ceiling, 0),
        ] {
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"{flat}\"; textureceiling = \"CEIL3_5\"; heightfloor = {floor}; heightceiling = {ceiling}; lightlevel = 160; id = {tag}; }}"
            );
        }
        text
    }

    #[test]
    fn a_shafts_top_face_is_read_off_the_landings_sidedef() {
        // The landing's ceiling (256) is above the plat's (128), so `r_segs.c`
        // draws the upper on the landing's side — never the plat's `PLATSIDE`.
        let f = fixture(&shaft(128, 256));
        let p = analyze(&f, 1).expect("analyzed");
        assert_eq!(p.shape, Shape::Core);
        assert_eq!(p.top_faces.len(), 1);
        assert_eq!(p.top_faces[0].texture, "BIGDOOR2");
        assert!(p.top_faces[0].dontpegtop);
        // Both two-sided boundaries draw an upper: the low room's `STARTAN3`
        // over the shaft (unpegged flag absent) and the landing's `BIGDOOR2`.
        assert_eq!(p.uppers_drawn, 2);
        assert_eq!(p.uppers_drawn_pegtop, 1);
        assert_eq!(p.risers.len(), 1);
        assert_eq!(p.risers[0].texture, "SUPPORT2");
        assert!(!p.risers[0].plat_side_nonblank);
        assert_eq!(p.floor_flat, "STEP1");
        assert_eq!(p.flat_same_as_level_nb, Some(false));
        assert_eq!(p.flat_same_as_low_nb, Some(false));
        // The landing is at the plat's own floor under a different ceiling, so
        // it is a two-sided side wall as well as a top face — the overlap the
        // report states rather than hides.
        assert_eq!(p.two_sided_jambs.len(), 1);
        assert!(p.two_sided_jambs[0].nb_ceiling_above);
        let mut jambs: Vec<(&str, bool, bool)> = p
            .jambs
            .iter()
            .map(|j| (j.texture.as_str(), j.dontpegtop, j.dontpegbottom))
            .collect();
        jambs.sort_unstable();
        assert_eq!(
            jambs,
            vec![("DOORTRAK", true, false), ("STARTAN2", false, true)]
        );
    }

    #[test]
    fn a_same_floor_neighbor_under_a_lower_ceiling_is_a_side_wall_not_a_top_face() {
        // The landing's ceiling (96) is now *below* the plat's (128): nothing
        // is drawn on the landing's side, the plat's own `PLATSIDE` upper is,
        // and the edge is a side wall only.
        let f = fixture(&shaft(128, 96));
        let p = analyze(&f, 1).expect("analyzed");
        assert!(p.top_faces.is_empty());
        assert_eq!(p.two_sided_jambs.len(), 1);
        assert!(!p.two_sided_jambs[0].nb_ceiling_above);
        assert_eq!(p.uppers_drawn, 2, "STARTAN3 over the shaft, PLATSIDE here");
        assert_eq!(p.uppers_drawn_pegtop, 1);
    }

    #[test]
    fn a_landing_sharing_the_plats_ceiling_is_neither_a_top_face_nor_a_side_wall() {
        let f = fixture(&shaft(256, 256));
        let p = analyze(&f, 1).expect("analyzed");
        assert!(p.top_faces.is_empty());
        assert!(p.two_sided_jambs.is_empty());
        assert_eq!(p.uppers_drawn, 0, "equal ceilings all round draw no upper");
    }

    #[test]
    fn far_sector_depth_is_measured_on_the_far_sectors_own_side() {
        // Three 128×128 rooms in a row; the room 0 / room 1 line is `chain`'s
        // first linedef, at x = 128. Room 2's floor is 32 — more than one
        // step above room 1's — so the room 1 / room 2 line blocks a player
        // walking east out of room 1.
        let f = fixture(&chain(
            &[0, 0, 32],
            &[0, 0, 0],
            &[(0, 0, false), (0, 0, false)],
            "",
        ));
        // Room 1 is the line's *back* sector; room 0 its front. Both are 128
        // deep beyond it, so a normal that followed the linedef's front side
        // instead of the sector asked about would return -128 for one of them.
        for far in [0, 1] {
            let (max_vertex, nearest) =
                far_depth(&f.map, &f.scene, 0, far, f.step).expect("measured");
            assert!(
                (max_vertex - 128.0).abs() < 1e-9,
                "sector {far} spans 128 units beyond the line, got {max_vertex}"
            );
            // 128, not 0: the room's north and south walls run *away* from the
            // line and touch it only at its endpoints, so they are excluded;
            // what stops the player is the wall across the far end.
            assert_eq!(nearest, Some(128.0), "sector {far}");
        }
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
