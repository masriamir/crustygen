//! The first interpreting layer over the census: vocabulary membership.
//!
//! [`Vocabulary::classify`] answers, per map, "does every value this map
//! carries have a name the compiler can emit?" on three axes — line
//! specials, sector specials, thing kinds — from the same `Tables` the
//! compiler reads. It is a membership test and nothing more: no geometry,
//! flags, tags, or texture names. The verdict is therefore an **upper
//! bound** on what a geometry-aware lifter could express, and every report
//! built on it says so.

use std::collections::{BTreeMap, BTreeSet};

use crate::lift::MapTelemetry;
use crate::tables::Tables;

/// The three emittable sets plus the vanilla membership list.
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
    reason = "these five booleans are independent per-axis verdicts (three membership checks) \
              plus two derived roll-ups (the conjunction and the stricter vanilla-only check) \
              over the same census, not state-machine flags — and their names are the \
              documented, load-bearing JSON field names later tasks and reports key off of"
)]
pub struct Verdict {
    /// Every non-zero linedef special is emittable.
    pub line_specials_ok: bool,
    /// Every non-zero sector special is nameable.
    pub sector_specials_ok: bool,
    /// Every thing type is in `[things]`.
    pub thing_kinds_ok: bool,
    /// The conjunction of the three axes.
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

    /// The emittable linedef specials (for the corpus greedy curve).
    #[must_use]
    pub fn line_specials(&self) -> &BTreeSet<i32> {
        &self.line
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
        Verdict {
            line_specials_ok,
            sector_specials_ok,
            thing_kinds_ok,
            expressible: line_specials_ok && sector_specials_ok && thing_kinds_ok,
            vanilla_only: t
                .linedef_specials
                .keys()
                .all(|k| self.vanilla_line.contains(k)),
            unknown_line_specials,
            unknown_sector_specials,
            unknown_thing_types,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;
    use crate::lift::{Census, survey};
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
        let v = vocab().classify(&telemetry(&[97], 0, &[1]));
        assert!(!v.line_specials_ok && v.sector_specials_ok && v.thing_kinds_ok);
        assert!(!v.expressible);
        assert_eq!(v.unknown_line_specials, vec![97]);
        assert!(
            v.vanilla_only,
            "97 is a vanilla special even though it is not emittable"
        );

        let v = vocab().classify(&telemetry(&[], 4, &[1]));
        assert!(v.line_specials_ok && !v.sector_specials_ok && v.thing_kinds_ok);
        assert_eq!(v.unknown_sector_specials, vec![4]);

        let v = vocab().classify(&telemetry(&[], 0, &[1, 46, 46, 54]));
        assert!(v.line_specials_ok && v.sector_specials_ok && !v.thing_kinds_ok);
        assert_eq!(
            v.unknown_thing_types,
            vec![46, 54],
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
        let _ = Census {
            vertices: 0,
            linedefs: 0,
            sidedefs: 0,
            sectors: 0,
            things: 0,
        };
    }

    #[test]
    fn verdict_serializes_with_the_documented_field_names() {
        let json = serde_json::to_value(vocab().classify(&telemetry(&[97], 0, &[1]))).unwrap();
        assert_eq!(json["expressible"], false);
        assert_eq!(json["unknown_line_specials"], serde_json::json!([97]));
        assert_eq!(json["vanilla_only"], true);
    }
}
