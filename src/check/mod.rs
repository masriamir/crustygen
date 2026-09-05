//! Layer-4 verification: re-derives playability from an emitted UDMF map.
//!
//! Works on [`crustywad::map::udmf::UdmfMap`] — the assembled artifact, never
//! the IR — so a compiler bug that satisfies the compiler's own pre-checks is
//! still caught here (`docs/design.md` §8 layer 4). Reuses [`crate::tables`]
//! (the sourced-constants authority) and [`crate::reach`]'s search, plus
//! [`crate::spec`]'s types as conformance targets, and deliberately nothing
//! from `compile/` or `rules.rs`: those are the logic under
//! cross-examination.
//!
//! `docs/check.md` documents the check catalog, the flood's construction
//! rules, the conformance verdict discipline, and the CLI contract.

use crate::spec::Spec;
use crate::tables::Tables;
use crustywad::map::udmf::UdmfMap;

pub mod conform;
pub mod flood;
pub mod floors;
pub mod invariants;
pub mod plats;
pub mod scene;

/// How bad a [`Finding`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A broken map (or a broken input): the run fails.
    Error,
    /// Suspicious but not provably broken (e.g. an unrecognized special).
    Warning,
    /// Informational only.
    Info,
}

/// What a [`Finding`] points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subject {
    /// A sector, by TEXTMAP declaration index.
    Sector(usize),
    /// A linedef, by TEXTMAP declaration index.
    Linedef(usize),
    /// A thing, by TEXTMAP declaration index.
    Thing(usize),
    /// The map as a whole.
    Map,
}

/// Verdict of one [`ConformanceRow`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// The actual satisfies the target.
    Pass,
    /// The actual violates a range target.
    Fail,
    /// Scalar target: reported with its delta, judged by no invented tolerance.
    Info,
    /// The parameter cannot be derived from emitted geometry (reason in `actual`).
    NotDerivable,
    /// A prerequisite check failed, so this row was not computed.
    NotRun,
}

/// One defect or observation, named after the rule it re-derives.
#[derive(Debug, Clone)]
pub struct Finding {
    /// Check id, e.g. `"V-P8"` or `"V-S"`.
    pub check: &'static str,
    /// How bad it is.
    pub severity: Severity,
    /// What it points at.
    pub subject: Subject,
    /// Human-readable detail naming concrete indices and values.
    pub message: String,
}

/// One spec-parameter comparison: target vs what the map actually contains.
#[derive(Debug, Clone)]
pub struct ConformanceRow {
    /// Frontmatter path, e.g. `"combat.monsters.imp"`.
    pub parameter: String,
    /// The spec's target, rendered as text.
    pub target: String,
    /// The measured value, rendered as text.
    pub actual: String,
    /// The judgment.
    pub verdict: Verdict,
}

/// One tag's resolution: which sectors carry it, which lines reference it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagEntry {
    /// The nonzero tag.
    pub tag: i32,
    /// Sectors whose `id` equals the tag, by declaration index.
    pub sectors: Vec<usize>,
    /// Action linedefs whose `args[0]` equals the tag, by declaration index.
    pub lines: Vec<usize>,
}

/// Summary counts the conformance rows and issue #3's report both read.
#[derive(Debug, Clone, Default)]
pub struct MapStats {
    /// Sector count.
    pub sectors: usize,
    /// Linedef count.
    pub linedefs: usize,
    /// Sidedef count.
    pub sidedefs: usize,
    /// Vertex count.
    pub vertices: usize,
    /// Thing count.
    pub things: usize,
    /// Sectors carrying the secret special.
    pub secret_sectors: usize,
}

/// The verifier's full result, shaped as the conformance report's (#3) input.
#[derive(Debug)]
pub struct CheckReport {
    /// Every defect and observation found.
    pub findings: Vec<Finding>,
    /// Spec-vs-actual rows; `Some` iff a spec was supplied.
    pub conformance: Option<Vec<ConformanceRow>>,
    /// Every nonzero tag's resolution.
    pub tag_manifest: Vec<TagEntry>,
    /// Summary counts.
    pub stats: MapStats,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let sev = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let subj = match self.subject {
            Subject::Sector(i) => format!("sector {i}"),
            Subject::Linedef(i) => format!("linedef {i}"),
            Subject::Thing(i) => format!("thing {i}"),
            Subject::Map => "map".to_owned(),
        };
        write!(f, "{} {sev} {subj}: {}", self.check, self.message)
    }
}

/// Runs every wired verification pass over `map` and returns the aggregated
/// report.
///
/// This builds the [`scene::Scene`] (which contributes reference-validity
/// findings), runs the texture ([`invariants::check_textures`], V-P8),
/// scaling ([`invariants::check_scaling`], V-P9), door-pegging
/// ([`invariants::check_door_pegging`], V-P11), lift-pegging
/// ([`invariants::check_lift_pegging`], the lift half of V-P11), tag
/// ([`invariants::check_tags`], V-P13/P14), thing-headroom
/// ([`invariants::check_thing_headroom`], V-P2), light-bounds
/// ([`invariants::check_light_bounds`], V-P19), start-clearance
/// ([`invariants::check_starts`], V-P25), prop-embedding
/// ([`invariants::check_prop_embedding`], the static half of V-P20),
/// passage-width ([`invariants::check_passage_width`], V-P3), door-opening
/// ([`invariants::check_door_openings`], V-P4), and recognized-special
/// ([`invariants::check_recognized_specials`], the flood's soundness
/// precondition), lift-return ([`invariants::check_lift_return`], V-P5),
/// floor-action ([`invariants::check_floor_actions`], V-P28),
/// teleport-pairing
/// ([`invariants::check_teleport_pairing`], V-P15) and sealed-monster-sector
/// ([`invariants::check_sealed_monster_rooms`], V-P27) invariants, runs the
/// key-aware reachability flood
/// ([`flood::run_flood`], V-P7) and, when it ran, the reachability half of
/// pickup accessibility over its result (`invariants::check_pickup_reachability`,
/// V-P20), runs key/lock coherence ([`flood::check_key_lock_coherence`],
/// V-P24), and fills [`MapStats`] from the map's own declaration counts.
///
/// `spec`, when `Some`, is judged against the built [`scene::Scene`] and
/// [`MapStats`] by [`conform::rows`], naming `map_name` as the actual map
/// slot for the `identity.slot` row; `conformance` is `None` iff `spec` is
/// `None`.
///
/// **Failure containment.** If `findings` carries a geometry-corrupting
/// `"V-S"` `Error` — a dangling cross-reference or a `twosided` flag
/// disagreeing with `sideback`'s presence (`Subject::Linedef`), or a sector
/// boundary that does not close (`Subject::Sector`) — `scene` was built from
/// data `Scene::build` itself gave up on: `conform::not_run_rows` runs
/// instead of `conform::rows`, producing the identical row catalog with
/// every verdict forced to [`Verdict::NotRun`] rather than a verdict that
/// looks decided but was judged against corrupt geometry. **Not** triggered
/// by a thing outside every closed sector (`"V-S"` `Error`,
/// `Subject::Thing`) — that thing's own placement already carries its own
/// finding, and every conformance row still reads intact
/// `scene.sectors`/`scene.things` data regardless — nor by either `"V-S"`
/// *Warning* case (unrecognized vocabulary), filtered out by severity alone.
/// See `docs/check.md`'s "Failure containment" section.
///
/// Returns them with the tag manifest `check_tags` produced.
#[must_use]
pub fn run(map: &UdmfMap, map_name: &str, tables: &Tables, spec: Option<&Spec>) -> CheckReport {
    let mut findings = Vec::new();
    let scene = scene::Scene::build(map, tables, &mut findings);

    invariants::check_textures(map, &scene, &mut findings);
    invariants::check_scaling(map, &mut findings);
    invariants::check_door_pegging(&scene, tables, &mut findings);
    invariants::check_lift_pegging(&scene, tables, &mut findings);
    let tag_manifest = invariants::check_tags(map, tables, &mut findings);
    invariants::check_thing_headroom(&scene, tables, &mut findings);
    invariants::check_light_bounds(&scene, tables, &mut findings);
    invariants::check_starts(&scene, tables, &mut findings);
    invariants::check_prop_embedding(&scene, tables, &mut findings);
    invariants::check_passage_width(&scene, tables, &mut findings);
    invariants::check_door_openings(&scene, tables, &mut findings);
    invariants::check_recognized_specials(&scene, tables, &mut findings);
    invariants::check_lift_return(&scene, tables, &mut findings);
    invariants::check_floor_actions(&scene, tables, &mut findings);
    invariants::check_teleport_pairing(&scene, tables, &mut findings);
    invariants::check_sealed_monster_rooms(&scene, tables, &mut findings);
    if let Some(reached) = flood::run_flood(&scene, tables, &mut findings) {
        invariants::check_pickup_reachability(&scene, tables, &reached, &mut findings);
    }
    flood::check_key_lock_coherence(&scene, tables, &mut findings);

    let stats = MapStats {
        sectors: map.sectors.len(),
        linedefs: map.linedefs.len(),
        sidedefs: map.sidedefs.len(),
        vertices: map.vertices.len(),
        things: map.things.len(),
        secret_sectors: map
            .sectors
            .iter()
            .filter(|sector| sector.special == i32::from(tables.secret_sector_special()))
            .count(),
    };

    // Narrowed to the two "V-S" Error producers that actually corrupt
    // geometry: a reference-validity failure (`Scene::build`'s
    // `process_linedef`, `Subject::Linedef` — a linedef contributes no
    // `Boundary` at all) and a sector boundary that fails to close
    // (`sector_is_closed`, `Subject::Sector` — `sector.closed` stays
    // `false`, so nothing resolves into it). Both mean some sector's
    // boundary is missing data conformance would otherwise measure.
    // Deliberately excludes the third "V-S" Error producer — a thing
    // outside every closed sector (`resolve_things`, `Subject::Thing`,
    // `KNOWN-GAPS.md`'s scope for this predicate) — because that thing's
    // own misplacement already carries its own "V-S" finding and does not
    // corrupt anything conformance reads: every row's counts and geometry
    // measurements come from `scene.sectors`/`scene.things`, both still
    // fully populated (`Scene::build` never shrinks either vector; a
    // misresolved thing just carries `sector: None`), so judging conformance
    // against a scene whose only defect is one stray thing is still honest.
    // The two "V-S" *Warning* cases (unrecognized vocabulary) never reach
    // this filter at all — filtered on `Severity::Error` — since those
    // describe a fully-formed scene the checker merely cannot name a
    // finding's vocabulary for, not a corrupt one.
    let structurally_broken = findings.iter().any(|f| {
        f.check == "V-S"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Linedef(_) | Subject::Sector(_))
    });

    let conformance = spec.map(|spec| {
        if structurally_broken {
            conform::not_run_rows(&scene, &stats, map_name, spec, tables)
        } else {
            conform::rows(&scene, &stats, map_name, spec, tables)
        }
    });

    CheckReport {
        findings,
        conformance,
        tag_manifest,
        stats,
    }
}

/// TEXTMAP fixtures shared by more than one `check` submodule's tests.
///
/// A fixture lives here rather than in whichever module first needed it so
/// the flood, the invariants, and the conformance rows all cross-examine the
/// *same* map: a defect that shows up as a missing edge in one and a missing
/// finding in another is only visibly the same defect when both read one
/// text.
#[cfg(test)]
pub(crate) mod fixtures {
    use crate::check::scene::Scene;
    use crate::tables::Tables;
    use crustywad::map::udmf::parse_udmf;

    /// Two disjoint squares. Sector 0 (0..128) holds the player start and an
    /// inner 32-unit island pad (sector 3) whose four edges carry
    /// `special = 97; arg0 = 5;` with sector 0 on their front. Sector 1
    /// (256..384) carries `id = 5` and the marker; sector 2 is a 32-deep
    /// alcove east of it behind a two-sided walkover exit line (52).
    ///
    /// Sector 0's winding is clockwise (north, east, south, west); the
    /// island's four lines wind counter-clockwise around the pad so their
    /// right-hand side — the front, sector 0 — faces outward; sector 1's
    /// east wall is split at `y = 32` and `y = 96` so the alcove threshold
    /// is its own two-sided line.
    pub(crate) const TELEPORT_MAP: &str = r#"namespace = "doom";

vertex { x = 0.0; y = 0.0; }
vertex { x = 0.0; y = 128.0; }
vertex { x = 128.0; y = 128.0; }
vertex { x = 128.0; y = 0.0; }
vertex { x = 64.0; y = 64.0; }
vertex { x = 64.0; y = 96.0; }
vertex { x = 96.0; y = 96.0; }
vertex { x = 96.0; y = 64.0; }
vertex { x = 256.0; y = 0.0; }
vertex { x = 256.0; y = 128.0; }
vertex { x = 384.0; y = 128.0; }
vertex { x = 384.0; y = 0.0; }
vertex { x = 384.0; y = 32.0; }
vertex { x = 384.0; y = 96.0; }
vertex { x = 416.0; y = 96.0; }
vertex { x = 416.0; y = 32.0; }

linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
linedef { v1 = 4; v2 = 7; sidefront = 4; sideback = 5; twosided = true; special = 97; arg0 = 5; }
linedef { v1 = 7; v2 = 6; sidefront = 6; sideback = 7; twosided = true; special = 97; arg0 = 5; }
linedef { v1 = 6; v2 = 5; sidefront = 8; sideback = 9; twosided = true; special = 97; arg0 = 5; }
linedef { v1 = 5; v2 = 4; sidefront = 10; sideback = 11; twosided = true; special = 97; arg0 = 5; }
linedef { v1 = 8; v2 = 9; sidefront = 12; blocking = true; }
linedef { v1 = 9; v2 = 10; sidefront = 13; blocking = true; }
linedef { v1 = 10; v2 = 13; sidefront = 14; blocking = true; }
linedef { v1 = 13; v2 = 12; sidefront = 15; sideback = 16; twosided = true; special = 52; arg0 = 1; }
linedef { v1 = 12; v2 = 11; sidefront = 17; blocking = true; }
linedef { v1 = 11; v2 = 8; sidefront = 18; blocking = true; }
linedef { v1 = 13; v2 = 14; sidefront = 19; blocking = true; }
linedef { v1 = 14; v2 = 15; sidefront = 20; blocking = true; }
linedef { v1 = 15; v2 = 12; sidefront = 21; blocking = true; }

sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturebottom = "STARTAN3"; }
sidedef { sector = 3; }
sidedef { sector = 0; texturebottom = "STARTAN3"; }
sidedef { sector = 3; }
sidedef { sector = 0; texturebottom = "STARTAN3"; }
sidedef { sector = 3; }
sidedef { sector = 0; texturebottom = "STARTAN3"; }
sidedef { sector = 3; }
sidedef { sector = 1; texturemiddle = "STARTAN3"; }
sidedef { sector = 1; texturemiddle = "STARTAN3"; }
sidedef { sector = 1; texturemiddle = "STARTAN3"; }
sidedef { sector = 1; }
sidedef { sector = 2; }
sidedef { sector = 1; texturemiddle = "STARTAN3"; }
sidedef { sector = 1; texturemiddle = "STARTAN3"; }
sidedef { sector = 2; texturemiddle = "STARTAN3"; }
sidedef { sector = 2; texturemiddle = "STARTAN3"; }
sidedef { sector = 2; texturemiddle = "STARTAN3"; }

sector { heightfloor = 0; heightceiling = 128; texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; lightlevel = 160; }
sector { heightfloor = 0; heightceiling = 128; texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; lightlevel = 160; id = 5; }
sector { heightfloor = 0; heightceiling = 128; texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; lightlevel = 160; }
sector { heightfloor = 8; heightceiling = 128; texturefloor = "GATE3"; textureceiling = "CEIL3_5"; lightlevel = 160; }

thing { x = 32.0; y = 32.0; angle = 90; type = 1; single = true; }
thing { x = 320.0; y = 64.0; angle = 0; type = 14; single = true; }
"#;

    /// Parses `text` and builds its [`Scene`], discarding structural
    /// findings (the fixtures here are well-formed; a test that wants them
    /// calls `Scene::build` itself).
    pub(crate) fn scene_of(text: &str) -> (Scene, Tables) {
        let tables = Tables::load().expect("tables");
        let map = parse_udmf(text, crustywad::Limits::default()).expect("fixture parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        (scene, tables)
    }

    /// A walkover lift whose trigger line has a pocket behind it: a low room
    /// (`x ∈ [0, 128]`, floor 0), the pocket (`depth` units wide, floor 0),
    /// the platform (64 wide, floor 128, `id = 7`) and a landing beyond it,
    /// all `y ∈ [0, 128]` under a 256 ceiling. **Linedef 0** is the `88`
    /// walkover naming tag 7, on the low room's own wall at `x = 128`.
    ///
    /// With `open_side` the pocket also opens north into a corridor at its
    /// own floor, so the same depth is a thin *through* strip rather than a
    /// dead end — the shape the census's §G3 found in DOOM E1M3 and MAP04,
    /// which the player crosses without trouble.
    ///
    /// Sectors: 0 low room, 1 pocket, 2 platform, 3 landing, and 4 the
    /// corridor when `open_side`. Every linedef is wound so its own sector
    /// lies on the right of `v1 -> v2`.
    pub(crate) fn pocket_lift(depth: i32, open_side: bool) -> String {
        use std::fmt::Write as _;

        let (x1, x2) = (128, 128 + depth);
        let (x3, x4) = (x2 + 64, x2 + 192);
        let mut text = String::from("namespace = \"doom\";\n");
        for (x, y) in [
            (0, 0),
            (0, 128),
            (x1, 0),
            (x1, 128),
            (x2, 0),
            (x2, 128),
            (x3, 0),
            (x3, 128),
            (x4, 0),
            (x4, 128),
            (x1, 256),
            (x2, 256),
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
        let mut link = |text: &mut String,
                        sidedefs: &mut String,
                        v: (usize, usize),
                        s: (usize, usize),
                        special: &str| {
            let (sf, sb) = (side(sidedefs, s.0, true), side(sidedefs, s.1, true));
            let _ = writeln!(
                text,
                "linedef {{ v1 = {}; v2 = {}; sidefront = {sf}; sideback = {sb}; twosided = true; {special} }}",
                v.0, v.1
            );
        };
        link(
            &mut text,
            &mut sidedefs,
            (3, 2),
            (0, 1),
            "special = 88; arg0 = 7;",
        );
        link(&mut text, &mut sidedefs, (5, 4), (1, 2), "");
        link(&mut text, &mut sidedefs, (7, 6), (2, 3), "");
        if open_side {
            link(&mut text, &mut sidedefs, (3, 5), (1, 4), "");
        }
        let mut wall =
            |text: &mut String, sidedefs: &mut String, v1: usize, v2: usize, s: usize| {
                let sf = side(sidedefs, s, false);
                let _ = writeln!(
                    text,
                    "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; blocking = true; }}"
                );
            };
        let mut walls = vec![
            (0, 1, 0),
            (1, 3, 0),
            (2, 0, 0),
            (4, 2, 1),
            (5, 7, 2),
            (6, 4, 2),
            (7, 9, 3),
            (9, 8, 3),
            (8, 6, 3),
        ];
        if open_side {
            walls.extend([(3, 10, 4), (10, 11, 4), (11, 5, 4)]);
        } else {
            walls.push((3, 5, 1));
        }
        for (v1, v2, s) in walls {
            wall(&mut text, &mut sidedefs, v1, v2, s);
        }
        text.push_str(&sidedefs);
        let mut floors = vec![(0, 0), (0, 0), (128, 7), (128, 0)];
        if open_side {
            floors.push((0, 0));
        }
        for (floor, tag) in floors {
            let _ = writeln!(
                text,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; \
                 heightfloor = {floor}; heightceiling = 256; lightlevel = 160; id = {tag}; }}"
            );
        }
        text
    }

    /// A row of 128×128 boxes, room `i` spanning `x ∈ [i·128, (i+1)·128]`,
    /// `y ∈ [0, 128]`, ceilings at 256. `floors[i]` and `tags[i]` are room
    /// `i`'s floor and sector tag. `links[i]` describes the two-sided line
    /// between rooms `i` and `i+1` as `(special, arg0, front_is_east)`: with
    /// `front_is_east` false the line runs top-to-bottom so its front (right)
    /// side is the west room, the natural clockwise orientation; `true` flips
    /// it so the east room is the front. Every link sidedef carries
    /// `SUPPORT3` as its lower; every one-sided wall is `STARTAN2`. `extra` is
    /// appended verbatim.
    ///
    /// Ported from `examples/liftprobe/common.rs`'s own test module (the lift
    /// shape probe, `docs/measurements/lift-shapes-2026-08-29.md`) so the
    /// probe's measured cases and this crate's re-derivation of them read the
    /// same geometry; `examples/` is not a library, so the probe keeps its
    /// copy.
    pub(crate) fn chain(
        floors: &[i32],
        tags: &[i32],
        links: &[(i32, i32, bool)],
        extra: &str,
    ) -> String {
        chain_full(floors, &vec![256; floors.len()], tags, links, extra)
    }

    /// [`chain`] with a ceiling per room, for the fixtures whose question is
    /// headroom rather than floor height — a slab with no standing room, a
    /// target whose own ceiling caps a raise.
    ///
    /// Ported from `examples/liftprobe/common.rs`'s test module beside
    /// [`chain`], for the same reason.
    pub(crate) fn chain_full(
        floors: &[i32],
        ceilings: &[i32],
        tags: &[i32],
        links: &[(i32, i32, bool)],
        extra: &str,
    ) -> String {
        use std::fmt::Write as _;

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
                "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sf}; sideback = {sb}; \
                 twosided = true; special = {special}; arg0 = {tag}; }}"
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
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; \
                 heightfloor = {floor}; heightceiling = {ceiling}; lightlevel = 160; \
                 id = {tag}; }}"
            );
        }
        text.push_str(extra);
        text
    }

    /// Appends a one-sided line carrying `special` and `tag` to a `rooms`-long
    /// [`chain`] or [`chain_full`], on the last room's east wall and fronted
    /// by that room — the "switch on B's far wall" of the floor-shape worked
    /// examples (`docs/measurements/floor-shapes-2026-09-02.md`), which puts
    /// the trigger somewhere that is neither the target nor a side the action
    /// moves.
    ///
    /// UDMF indices follow declaration order per type, so appending the two
    /// records gives them the next linedef and sidedef index. The wall itself
    /// is already drawn by the builder, so this line doubles it — and that
    /// costs the last room its closure: the doubled segment raises both of
    /// its endpoints to degree 3, and [`Scene::build`] reports the room with
    /// a hard `V-S "boundary does not close"`.
    ///
    /// **So no caller may place a thing in the far-wall room**: a thing only
    /// ever resolves to a *closed* sector, so one standing there resolves to
    /// no sector at all and is invisible to every count that reads
    /// `SceneThing::sector`. What the fixture is for — which lines carry
    /// which specials, what the flood and the recognizer make of the
    /// geometry — does not depend on closure, which is why the tests built
    /// on it read the scene directly (through [`scene_of`], discarding
    /// findings) rather than through a clean-scene assertion.
    ///
    /// Ported from `examples/liftprobe/floors.rs`'s test module.
    pub(crate) fn far_wall(text: &mut String, rooms: usize, special: i32, tag: i32) {
        use std::fmt::Write as _;

        let sd = text.matches("sidedef {").count();
        let (v1, v2) = (2 * rooms + 1, 2 * rooms);
        let last = rooms - 1;
        let _ = writeln!(
            text,
            "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {sd}; blocking = true; \
             special = {special}; arg0 = {tag}; }}\n\
             sidedef {{ sector = {last}; texturemiddle = \"STARTAN2\"; }}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_finding_formats_as_check_severity_subject_and_message() {
        let f = Finding {
            check: "V-P8",
            severity: Severity::Error,
            subject: Subject::Linedef(12),
            message: "two-sided line needs a lower texture on its front side".to_owned(),
        };
        assert_eq!(
            f.to_string(),
            "V-P8 error linedef 12: two-sided line needs a lower texture on its front side"
        );
    }

    #[test]
    fn every_severity_and_subject_variant_formats_distinctly() {
        let cases = [
            (Severity::Error, Subject::Sector(3), "V-S error sector 3: x"),
            (
                Severity::Warning,
                Subject::Linedef(4),
                "V-S warning linedef 4: x",
            ),
            (Severity::Info, Subject::Thing(5), "V-S info thing 5: x"),
            (Severity::Info, Subject::Map, "V-S info map: x"),
        ];
        for (severity, subject, expected) in cases {
            let f = Finding {
                check: "V-S",
                severity,
                subject,
                message: "x".to_owned(),
            };
            assert_eq!(f.to_string(), expected);
        }
    }
}
