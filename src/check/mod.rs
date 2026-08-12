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
pub mod invariants;
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
    /// The judgement.
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
/// ([`invariants::check_door_pegging`], V-P11), tag
/// ([`invariants::check_tags`], V-P13/P14), thing-headroom
/// ([`invariants::check_thing_headroom`], V-P2), light-bounds
/// ([`invariants::check_light_bounds`], V-P19), start-clearance
/// ([`invariants::check_starts`], V-P25), prop-embedding
/// ([`invariants::check_prop_embedding`], the static half of V-P20),
/// passage-width ([`invariants::check_passage_width`], V-P3), door-opening
/// ([`invariants::check_door_openings`], V-P4), and recognized-special
/// ([`invariants::check_recognized_specials`], the flood's soundness
/// precondition) invariants, runs the key-aware reachability flood
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
    let tag_manifest = invariants::check_tags(map, tables, &mut findings);
    invariants::check_thing_headroom(&scene, tables, &mut findings);
    invariants::check_light_bounds(&scene, tables, &mut findings);
    invariants::check_starts(&scene, tables, &mut findings);
    invariants::check_prop_embedding(&scene, tables, &mut findings);
    invariants::check_passage_width(&scene, tables, &mut findings);
    invariants::check_door_openings(&scene, tables, &mut findings);
    invariants::check_recognized_specials(&scene, tables, &mut findings);
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
}
