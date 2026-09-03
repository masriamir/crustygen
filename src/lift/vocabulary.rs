//! The first interpreting layer over the census: vocabulary membership.
//!
//! [`Vocabulary::classify`] answers, per map, "does every value this map
//! carries have a name the compiler can emit?" on three axes — line
//! specials, sector specials, thing kinds — from the same `Tables` the
//! compiler reads. It is a membership test and nothing more: no geometry,
//! flags, tags, or texture names. The verdict is therefore an **upper
//! bound** on what a geometry-aware lifter could express, and every report
//! built on it says so.
//!
//! [`Verdict::with_teleports`] is where a recognizer narrows that bound: it
//! folds [`crate::lift::teleport`]'s refusals in as a fourth axis, so a map
//! whose every value is nameable can still be refused for a teleport line
//! the IR could not state. [`Verdict::with_lifts`] is the same move for
//! [`crate::lift::plat`]'s refusals, the fifth axis, and
//! [`Verdict::with_floors`] for [`crate::lift::floor`]'s, the sixth.
//! Membership can only ever say "yes"; a recognizer is what turns a "yes"
//! into a "no".

use std::collections::{BTreeMap, BTreeSet};

use crate::lift::MapTelemetry;
use crate::lift::floor::FloorReport;
use crate::lift::plat::PlatReport;
use crate::lift::teleport::TeleportReport;
use crate::tables::Tables;

/// The emittable line set, the named sector set, the thing set, plus the
/// vanilla membership list.
#[derive(Debug, Clone)]
pub struct Vocabulary {
    line: BTreeSet<i32>,
    sector: BTreeSet<i32>,
    thing: BTreeSet<i32>,
    vanilla_line: BTreeSet<i32>,
}

/// One map's classification.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "these eight booleans are independent per-axis verdicts (three membership checks \
              plus the teleport, plat and floor recognizers') and two derived roll-ups (the \
              six-axis conjunction and the stricter vanilla-only check) over the same census, \
              not state-machine flags — and their names are the documented, load-bearing JSON \
              field names later tasks and reports key off of"
)]
pub struct Verdict {
    /// Every non-zero linedef special is emittable.
    pub line_specials_ok: bool,
    /// Every non-zero sector special is nameable.
    pub sector_specials_ok: bool,
    /// Every thing type is in `[things]`.
    pub thing_kinds_ok: bool,
    /// Every teleport line resolves to a destination and is not
    /// self-referencing ([`crate::lift::teleport`]); `true` until
    /// [`Verdict::with_teleports`] is applied.
    pub teleports_ok: bool,
    /// Every platform a lift line names is one of the three shapes the IR
    /// can state, and every lift line names a platform
    /// ([`crate::lift::plat`]); `true` until [`Verdict::with_lifts`] is
    /// applied.
    pub lifts_ok: bool,
    /// Every floor target is one of the three shapes the IR can state and
    /// every floor line names a target ([`crate::lift::floor`]); `true`
    /// until [`Verdict::with_floors`] is applied.
    pub floors_ok: bool,
    /// The conjunction of the six axes.
    pub expressible: bool,
    /// Every non-zero linedef special is one the pinned vanilla engine
    /// dispatches.
    pub vanilla_only: bool,
    /// Out-of-set linedef specials, ascending.
    pub unknown_line_specials: Vec<i32>,
    /// Out-of-set sector specials, ascending.
    pub unknown_sector_specials: Vec<i32>,
    /// Out-of-set thing types, ascending.
    pub unknown_thing_types: Vec<i32>,
}

fn widen(set: impl IntoIterator<Item = u16>) -> BTreeSet<i32> {
    set.into_iter().map(i32::from).collect()
}

fn unknown(histogram: &BTreeMap<i32, u64>, set: &BTreeSet<i32>) -> Vec<i32> {
    histogram
        .keys()
        .copied()
        .filter(|k| !set.contains(k))
        .collect()
}

impl Vocabulary {
    /// Builds the sets from the loaded tables.
    #[must_use]
    pub fn from_tables(tables: &Tables) -> Self {
        Self {
            line: widen(tables.emittable_line_specials()),
            sector: widen(tables.named_sector_specials()),
            thing: widen(tables.thing_kinds().map(|(_, id)| id)),
            vanilla_line: widen(tables.vanilla_line_specials()),
        }
    }

    /// Classifies one map's census. A map with nothing on an axis is `ok`
    /// on that axis — vacuously, a special-free map is expressible by
    /// specials.
    #[must_use]
    pub fn classify(&self, t: &MapTelemetry) -> Verdict {
        let unknown_line_specials = unknown(&t.linedef_specials, &self.line);
        let unknown_sector_specials = unknown(&t.sector_specials, &self.sector);
        let unknown_thing_types = unknown(&t.thing_types, &self.thing);
        let line_specials_ok = unknown_line_specials.is_empty();
        let sector_specials_ok = unknown_sector_specials.is_empty();
        let thing_kinds_ok = unknown_thing_types.is_empty();
        let mut verdict = Verdict {
            line_specials_ok,
            sector_specials_ok,
            thing_kinds_ok,
            // Membership knows nothing about geometry: the teleport, lift
            // and floor axes start clean and only `with_teleports` /
            // `with_lifts` / `with_floors` can refuse them.
            teleports_ok: true,
            lifts_ok: true,
            floors_ok: true,
            expressible: false,
            vanilla_only: t
                .linedef_specials
                .keys()
                .all(|k| self.vanilla_line.contains(k)),
            unknown_line_specials,
            unknown_sector_specials,
            unknown_thing_types,
        };
        verdict.expressible = verdict.conjunction();
        verdict
    }
}

impl Verdict {
    /// The conjunction of every axis — the one definition of `expressible`,
    /// so that no site can compute it from fewer axes than the struct
    /// carries and no later axis can be forgotten at one of them.
    fn conjunction(&self) -> bool {
        self.line_specials_ok
            && self.sector_specials_ok
            && self.thing_kinds_ok
            && self.teleports_ok
            && self.lifts_ok
            && self.floors_ok
    }

    /// Folds the teleport recognizer in: `teleports_ok` is "no refused
    /// line", the fourth axis.
    #[must_use]
    pub fn with_teleports(mut self, report: &TeleportReport) -> Self {
        self.teleports_ok = report.counts.refusals() == 0;
        self.expressible = self.conjunction();
        self
    }

    /// Folds the plat recognizer in: `lifts_ok` is "no refused plat and no
    /// broken lift line", the fifth axis.
    #[must_use]
    pub fn with_lifts(mut self, report: &PlatReport) -> Self {
        self.lifts_ok = report.counts.refusals() == 0;
        self.expressible = self.conjunction();
        self
    }

    /// Folds the floor recognizer in: `floors_ok` is "no refused target and
    /// no broken floor line", the sixth axis.
    #[must_use]
    pub fn with_floors(mut self, report: &FloorReport) -> Self {
        self.floors_ok = report.counts.refusals() == 0;
        self.expressible = self.conjunction();
        self
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::check::fixtures::{chain, far_wall};
    use crate::check::scene::Scene;
    use crate::lift::floor;
    use crate::lift::plat;
    use crate::lift::survey;
    use crate::lift::teleport::TeleportCounts;
    use crustywad::Limits;
    use crustywad::map::udmf::parse_udmf;

    fn vocab() -> Vocabulary {
        Vocabulary::from_tables(&Tables::load().expect("tables"))
    }

    /// A one-sector square whose specials/types are the arguments.
    fn telemetry(line: &[i32], sector: i32, things: &[i32]) -> MapTelemetry {
        let mut text = String::from("namespace = \"doom\";\n");
        for (x, y) in [(0, 0), (128, 0), (128, 128), (0, 128)] {
            let _ = writeln!(text, "vertex {{ x = {x}; y = {y}; }}");
        }
        for i in 0..4 {
            let special = line.get(i).copied().unwrap_or(0);
            let _ = writeln!(
                text,
                "linedef {{ v1 = {i}; v2 = {}; sidefront = {i}; special = {special}; }}",
                (i + 1) % 4
            );
            text.push_str("sidedef { sector = 0; texturemiddle = \"STARTAN2\"; }\n");
        }
        let _ = writeln!(
            text,
            "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightceiling = 128; special = {sector}; }}"
        );
        for t in things {
            let _ = writeln!(text, "thing {{ x = 64; y = 64; type = {t}; }}");
        }
        survey(
            "MAP01",
            &parse_udmf(&text, Limits::default()).expect("fixture parses"),
        )
    }

    #[test]
    fn a_doors_and_exit_map_with_known_things_is_expressible() {
        let v = vocab().classify(&telemetry(&[1, 11], 9, &[1, 3004]));
        assert!(v.expressible && v.vanilla_only);
        assert!(v.unknown_line_specials.is_empty());
        assert!(v.unknown_sector_specials.is_empty());
        assert!(v.unknown_thing_types.is_empty());
    }

    #[test]
    fn each_axis_flips_independently() {
        let v = vocab().classify(&telemetry(&[21], 0, &[1]));
        assert!(!v.line_specials_ok && v.sector_specials_ok && v.thing_kinds_ok);
        assert!(!v.expressible);
        assert_eq!(v.unknown_line_specials, vec![21]);
        assert!(
            v.vanilla_only,
            "21 is a vanilla special even though it is not emittable"
        );

        let v = vocab().classify(&telemetry(&[], 4, &[1]));
        assert!(v.line_specials_ok && !v.sector_specials_ok && v.thing_kinds_ok);
        assert_eq!(v.unknown_sector_specials, vec![4]);

        // 9998 and 9999 are defined by no vanilla mobjinfo entry, so they
        // stay unknown however far `[things]` grows. This block used to use
        // 46 and 54 (the tall red torch and the big tree), which the
        // complete decoration set turned into real rows.
        let v = vocab().classify(&telemetry(&[], 0, &[1, 9999, 9999, 9998]));
        assert!(v.line_specials_ok && v.sector_specials_ok && !v.thing_kinds_ok);
        assert_eq!(
            v.unknown_thing_types,
            vec![9998, 9999],
            "sorted and deduplicated"
        );
    }

    #[test]
    fn a_boom_special_leaves_the_vanilla_slice() {
        // 8192 is far above any vanilla case label (Boom generalized range).
        let v = vocab().classify(&telemetry(&[8192], 0, &[1]));
        assert!(!v.vanilla_only);
        assert!(!v.line_specials_ok);
    }

    #[test]
    fn an_empty_map_is_vacuously_expressible() {
        let v = vocab().classify(&telemetry(&[], 0, &[]));
        assert!(v.expressible && v.vanilla_only);
    }

    #[test]
    fn a_refused_teleport_line_flips_the_verdict_off_without_touching_the_axes() {
        let v = vocab().classify(&telemetry(&[97], 0, &[1, 14]));
        assert!(
            v.teleports_ok && v.expressible,
            "97 is emittable now; membership alone passes"
        );
        let refused = TeleportReport {
            lines: vec![],
            counts: TeleportCounts {
                lines: 1,
                broken: 1,
                ..TeleportCounts::default()
            },
        };
        let v = v.with_teleports(&refused);
        assert!(!v.teleports_ok && !v.expressible && v.line_specials_ok);
        let json = serde_json::to_value(&v).unwrap();
        assert_eq!(json["teleports_ok"], false);
    }

    /// A report with no refusal leaves the verdict exactly as membership
    /// found it — including a `false` `expressible` that an out-of-set value
    /// already earned, which `with_teleports` must not resurrect.
    #[test]
    fn a_clean_report_leaves_the_membership_verdict_alone() {
        let v = vocab()
            .classify(&telemetry(&[21], 0, &[1]))
            .with_teleports(&TeleportReport {
                lines: vec![],
                counts: TeleportCounts::default(),
            });
        assert!(v.teleports_ok);
        assert!(!v.expressible && !v.line_specials_ok);
    }

    #[test]
    fn with_lifts_is_the_fifth_axis() {
        let tables = Tables::load().expect("tables");
        let vocab = Vocabulary::from_tables(&tables);
        let clean_text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        );
        let map = parse_udmf(&clean_text, Limits::default()).expect("parses");
        let telemetry = survey("MAP01", &map);
        let verdict = vocab.classify(&telemetry);
        assert!(
            verdict.lifts_ok && verdict.line_specials_ok,
            "62 is emittable and no plat is refused yet: {verdict:?}"
        );
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let ok = verdict
            .clone()
            .with_lifts(&plat::recognize(&scene, &tables));
        assert!(ok.lifts_ok && ok.expressible);
        let dead_text = chain(&[0, 0, 0], &[0, 7, 0], &[(62, 7, false), (0, 0, false)], "");
        let map = parse_udmf(&dead_text, Limits::default()).expect("parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let refused = vocab
            .classify(&survey("MAP01", &map))
            .with_lifts(&plat::recognize(&scene, &tables));
        assert!(
            !refused.lifts_ok && !refused.expressible,
            "a dead lift is a refusal: {refused:?}"
        );
    }

    /// A clean plat report leaves a membership refusal standing, the way
    /// `with_teleports` does: neither recognizer may resurrect an
    /// `expressible` an out-of-set value already earned.
    #[test]
    fn a_clean_plat_report_leaves_the_membership_verdict_alone() {
        let tables = Tables::load().expect("tables");
        let text = chain(
            &[0, 128, 128],
            &[0, 7, 0],
            &[(62, 7, false), (0, 0, false)],
            "",
        );
        let map = parse_udmf(&text, Limits::default()).expect("parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let report = plat::recognize(&scene, &tables);
        let v = vocab()
            .classify(&telemetry(&[21], 0, &[1]))
            .with_lifts(&report);
        assert!(v.lifts_ok);
        assert!(!v.expressible && !v.line_specials_ok);
    }

    #[test]
    fn with_floors_is_the_sixth_axis() {
        let tables = Tables::load().expect("tables");
        let vocab = Vocabulary::from_tables(&tables);
        // A drop wall: A(0) - T(128) - B(0), with a 23 S1 on B's far wall.
        let mut clean_text = chain(&[0, 128, 0], &[0, 7, 0], &[(0, 0, false); 2], "");
        far_wall(&mut clean_text, 3, 23, 7);
        let map = parse_udmf(&clean_text, Limits::default()).expect("parses");
        let verdict = vocab.classify(&survey("MAP01", &map));
        assert!(
            verdict.floors_ok && verdict.line_specials_ok,
            "23 is emittable and no target is refused yet: {verdict:?}"
        );
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let ok = verdict
            .clone()
            .with_floors(&floor::recognize(&scene, &tables));
        assert!(ok.floors_ok && ok.expressible);
        // The same 23, carrying tag 0: a line that can never move a floor.
        let broken_text = chain(
            &[0, 128, 0],
            &[0, 0, 0],
            &[(23, 0, false), (0, 0, false)],
            "",
        );
        let map = parse_udmf(&broken_text, Limits::default()).expect("parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let refused = vocab
            .classify(&survey("MAP01", &map))
            .with_floors(&floor::recognize(&scene, &tables));
        assert!(
            !refused.floors_ok && !refused.expressible,
            "a tag-0 floor line is a refusal: {refused:?}"
        );
    }

    /// A clean floor report leaves a membership refusal standing, the way
    /// `with_teleports` and `with_lifts` do: no recognizer may resurrect an
    /// `expressible` an out-of-set value already earned.
    #[test]
    fn a_clean_floor_report_leaves_the_membership_verdict_alone() {
        let tables = Tables::load().expect("tables");
        let mut text = chain(&[0, 128, 0], &[0, 7, 0], &[(0, 0, false); 2], "");
        far_wall(&mut text, 3, 23, 7);
        let map = parse_udmf(&text, Limits::default()).expect("parses");
        let scene = Scene::build(&map, &tables, &mut Vec::new());
        let report = floor::recognize(&scene, &tables);
        assert_eq!(report.counts.refusals(), 0);
        let v = vocab()
            .classify(&telemetry(&[21], 0, &[1]))
            .with_floors(&report);
        assert!(v.floors_ok);
        assert!(!v.expressible && !v.line_specials_ok);
    }

    #[test]
    fn verdict_serializes_with_the_documented_field_names() {
        let json = serde_json::to_value(vocab().classify(&telemetry(&[21], 0, &[1]))).unwrap();
        assert_eq!(json["expressible"], false);
        assert_eq!(json["unknown_line_specials"], serde_json::json!([21]));
        assert_eq!(json["vanilla_only"], true);
        assert_eq!(json["lifts_ok"], true);
        assert_eq!(json["floors_ok"], true);
    }
}
