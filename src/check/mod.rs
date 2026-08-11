//! Layer-4 verification: re-derives playability from an emitted UDMF map.
//!
//! Works on [`crustywad::map::udmf::UdmfMap`] — the assembled artifact, never
//! the IR — so a compiler bug that satisfies the compiler's own pre-checks is
//! still caught here (`docs/design.md` §8 layer 4). Reuses [`crate::tables`]
//! (the sourced-constants authority) and [`crate::reach`]'s search, and
//! deliberately nothing from `compile/` or `rules.rs`: those are the logic
//! under cross-examination.

use crate::spec::Spec;
use crate::tables::Tables;
use crustywad::map::udmf::UdmfMap;

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
#[derive(Debug, Clone)]
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
/// `map_name` and `spec` are accepted now so this signature is final;
/// `map_name` starts being read once a later task's report needs it, and
/// `spec` once a conformance pass exists to compare against it. For now,
/// this builds the [`scene::Scene`] (which contributes reference-validity
/// findings), runs the texture ([`invariants::check_textures`], V-P8),
/// scaling ([`invariants::check_scaling`], V-P9), and door-pegging
/// ([`invariants::check_door_pegging`], V-P11) invariants, fills
/// [`MapStats`] from the map's own declaration counts, and returns them
/// with `conformance: None` and an empty tag manifest — later tasks append
/// more passes.
#[must_use]
pub fn run(map: &UdmfMap, _map_name: &str, tables: &Tables, _spec: Option<&Spec>) -> CheckReport {
    let mut findings = Vec::new();
    let scene = scene::Scene::build(map, tables, &mut findings);

    invariants::check_textures(map, &scene, &mut findings);
    invariants::check_scaling(map, &mut findings);
    invariants::check_door_pegging(&scene, tables, &mut findings);

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

    CheckReport {
        findings,
        conformance: None,
        tag_manifest: Vec::new(),
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
