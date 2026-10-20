//! Corpus sweep: every zip/WAD in a directory → per-map census + verdict,
//! deduplicated by content, with every failure counted in a bucket. Each
//! surviving map also runs through [`crate::lift::teleport`],
//! [`crate::lift::plat`] and [`crate::lift::floor`], whose refusals gate the
//! verdict's fourth, fifth and sixth axes and whose counts the report rolls
//! up.
//! The sweep lives in `sweep`, the roll-up in `aggregate` and the rendering in `report`; every public item is re-exported here, so `lift::corpus::<name>` is the one public path.

mod aggregate;
mod report;
mod sweep;

pub use aggregate::{
    Aggregate, AxisShare, BLOCKER_TOP, Blocker, FloorAggregate, GREEDY_CHECKPOINTS, GreedyCurve,
    GreedyStep, LiftAggregate, TeleportAggregate, aggregate,
};
pub use report::{Provenance, Report, read_provenance, render_markdown};
pub use sweep::{Buckets, CorpusError, MapRecord, Sweep, map_hash, sweep_dir};

#[cfg(test)]
mod fixtures {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;
    use crate::lift::MapTelemetry;
    use crate::lift::floor::FloorCounts;
    use crate::lift::plat::PlatCounts;
    use crate::lift::teleport::TeleportCounts;
    use crate::lift::vocabulary::Verdict;

    pub(super) fn record(
        line: &[i32],
        unknown_line: &[i32],
        sector_ok: bool,
        thing_ok: bool,
        vanilla: bool,
    ) -> MapRecord {
        let telemetry = MapTelemetry {
            map: "M".into(),
            census: crate::lift::Census {
                vertices: 0,
                linedefs: 0,
                sidedefs: 0,
                sectors: 0,
                things: 0,
            },
            linedef_specials: line.iter().map(|&s| (s, 1)).collect(),
            sector_specials: BTreeMap::new(),
            thing_types: BTreeMap::new(),
        };
        let verdict = Verdict {
            line_specials_ok: unknown_line.is_empty(),
            sector_specials_ok: sector_ok,
            thing_kinds_ok: thing_ok,
            teleports_ok: true,
            lifts_ok: true,
            floors_ok: true,
            expressible: unknown_line.is_empty() && sector_ok && thing_ok,
            vanilla_only: vanilla,
            unknown_line_specials: unknown_line.to_vec(),
            unknown_sector_specials: if sector_ok { vec![] } else { vec![4] },
            unknown_thing_types: if thing_ok { vec![] } else { vec![46] },
        };
        MapRecord {
            source: "s".into(),
            map: "M".into(),
            origin: "udmf".into(),
            hash: "h".into(),
            telemetry,
            verdict,
            teleports: TeleportCounts::default(),
            lifts: PlatCounts::default(),
            floors: FloorCounts::default(),
        }
    }

    /// The teleport golden fixture, compiled and packed as a one-map UDMF
    /// PWAD — the same route `tests/common/mod.rs::udmf_entrada_wad` takes.
    pub(super) fn teleport_wad() -> Vec<u8> {
        let tables = crate::tables::Tables::load().expect("tables");
        let ir = crate::ir::Ir::from_json(include_str!("../../../tests/golden/teleports.json"))
            .expect("ir parses");
        let compiled = crate::compile::compile(&ir, &tables).expect("compiles");
        crate::pack::pack_udmf(&compiled, "MAP01").expect("packs")
    }

    /// A fresh temp directory holding `files`.
    pub(super) fn temp_dir_with(label: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time moves forward")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "crustygen-sweep-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("temp dir");
        for (name, bytes) in files {
            std::fs::write(dir.join(name), bytes).expect("write fixture");
        }
        dir
    }

    /// The lift golden fixture, compiled and packed as a one-map UDMF PWAD —
    /// the same route [`teleport_wad`] takes.
    pub(super) fn lifts_wad() -> Vec<u8> {
        let tables = crate::tables::Tables::load().expect("tables");
        let ir = crate::ir::Ir::from_json(include_str!("../../../tests/golden/lifts.json"))
            .expect("ir parses");
        let compiled = crate::compile::compile(&ir, &tables).expect("compiles");
        crate::pack::pack_udmf(&compiled, "MAP01").expect("packs")
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use crate::lift::vocabulary::Vocabulary;

    /// The end-to-end teleport path: a swept WAD carries per-map counts, the
    /// aggregate rolls them up, and the report renders the section.
    #[test]
    fn a_swept_map_carries_its_teleport_counts_into_the_aggregate_and_the_report() {
        let tables = crate::tables::Tables::load().expect("tables");
        let vocab = Vocabulary::from_tables(&tables);
        let dir = temp_dir_with("teleports", &[("t.wad", teleport_wad())]);
        let sweep = sweep_dir(&dir, &vocab, &tables).expect("sweeps");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(sweep.maps.len(), 1);
        let rec = &sweep.maps[0];
        assert_eq!(
            rec.teleports.lines, 9,
            "five player-crossable edges plus the pen's four monsters-only edges"
        );
        assert_eq!(rec.teleports.player, 5);
        assert_eq!(rec.teleports.monsters_only, 4);
        assert_eq!(rec.teleports.refusals(), 0, "the fixture is well formed");
        assert!(rec.verdict.teleports_ok && rec.verdict.expressible);
        // Shape axes, traced line by line through the fixture: the island
        // pad's four edges and the pen's four are all free-standing pads;
        // only the wall pad's single threshold is a recess.
        assert_eq!(rec.teleports.island, 8);
        assert_eq!(rec.teleports.alcove, 1, "the wall pad's threshold");
        assert_eq!(
            rec.teleports.paired, 4,
            "the island's four edges back onto the pad, which holds a marker of its own"
        );
        assert_eq!(
            rec.teleports.exit, 4,
            "the pen's four edges deliver into room a, which hosts the switch exit"
        );

        let a = aggregate(&sweep.maps);
        assert_eq!(a.teleports.maps_with_teleports, 1);
        assert_eq!(a.teleports.maps_refused, 0);
        assert_eq!(a.teleports.lines.lines, 9);
        assert_eq!(a.teleports.closet_maps, 1, "the pen holds an ambush imp");
        let md = render_markdown(None, &sweep.buckets, &a);
        assert!(md.contains("## Teleports"), "{md}");
        assert!(md.contains("| maps with a teleport line | 1"), "{md}");
    }

    /// The end-to-end plat path: a swept WAD carries per-map counts, the
    /// aggregate rolls them up, and the report renders the section.
    #[test]
    fn a_swept_map_carries_its_lift_counts_into_the_aggregate_and_the_report() {
        let tables = crate::tables::Tables::load().expect("tables");
        let vocab = Vocabulary::from_tables(&tables);
        let dir = temp_dir_with("lifts", &[("l.wad", lifts_wad())]);
        let sweep = sweep_dir(&dir, &vocab, &tables).expect("sweeps");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(sweep.maps.len(), 1);
        let rec = &sweep.maps[0];
        assert_eq!(
            rec.lifts.plats, 4,
            "the golden's three lift portals plus its one pedestal"
        );
        assert_eq!(
            (rec.lifts.lifts, rec.lifts.pedestals, rec.lifts.barriers),
            (2, 1, 1)
        );
        assert_eq!(rec.lifts.refusals(), 0, "the fixture is well formed");
        assert!(rec.verdict.lifts_ok && rec.verdict.expressible);

        let a = aggregate(&sweep.maps);
        assert_eq!(a.lifts.maps_with_lifts, 1);
        assert_eq!(a.lifts.maps_refused, 0);
        assert_eq!(a.lifts.plats.plats, 4);
        assert_eq!(
            (
                a.lifts.lift_maps,
                a.lifts.pedestal_maps,
                a.lifts.barrier_maps
            ),
            (1, 1, 1)
        );
        let md = render_markdown(None, &sweep.buckets, &a);
        assert!(md.contains("## Lifts"), "{md}");
        assert!(md.contains("| plats | 4 |"), "{md}");
        assert!(
            md.contains("| lifts / pedestals / barriers | 2 / 1 / 1 |"),
            "{md}"
        );
    }
}
