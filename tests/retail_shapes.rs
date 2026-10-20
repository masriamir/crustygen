//! Pins the recognizer's DOOM + DOOM2 lift counts against the real retail IWADs.
//!
//! The retail figures were otherwise recorded only in prose. This test sweeps exactly `DOOM.WAD`
//! and `DOOM2.WAD` and asserts the whole roll-up, so a recognizer change that moves a count fails
//! here instead of silently drifting.
//!
//! It is opt-in: point `CRUSTYGEN_IWAD_DIR` at a directory holding the two IWADs, or run
//! `just test-retail <dir>`. With the variable unset (or the directory unusable) it prints a
//! `skipping:` line and passes, so CI, which has no commercial IWADs, never runs it.
//!
//! The literal was measured on 2026-10-20 at `2a79a6e` against `DOOM.WAD` 12,408,292 bytes /
//! md5 `c4fe9fd920207691a9f493668e0a2083` / 36 maps and `DOOM2.WAD` 14,604,584 bytes / md5
//! `25e1459ca71d321525f84628f45ca8cd` / 32 maps. The figures are recorded in
//! `docs/measurements/lifts-2026-08-30.md` under the 2026-10-20 addendum.

use std::path::{Path, PathBuf};

use crustygen::lift::corpus::{Buckets, LiftAggregate, aggregate, sweep_dir};
use crustygen::lift::plat::PlatCounts;
use crustygen::lift::vocabulary::Vocabulary;
use crustygen::tables::Tables;

const VAR: &str = "CRUSTYGEN_IWAD_DIR";
const IWADS: [&str; 2] = ["DOOM.WAD", "DOOM2.WAD"];
const ADDENDUM: &str = "docs/measurements/lifts-2026-08-30.md (addendum 2026-10-20)";
const PRINTS: &str = "DOOM.WAD 12,408,292 bytes (36 maps), DOOM2.WAD 14,604,584 bytes (32 maps)";

/// The sweep over exactly the two IWADs: two bare WADs, 68 maps, no duplicates, nothing failed.
const EXPECTED_BUCKETS: Buckets = Buckets {
    archives: 0,
    wads: 2,
    maps_raw: 68,
    maps_unique: 68,
    archive_unreadable: 0,
    wad_unreadable: 0,
    no_maps: 0,
    unsupported_format: 0,
    assembly_refused: 0,
    textmap_unparseable: 0,
};

/// The recognizer's DOOM+DOOM2 roll-up, measured 2026-10-20 at `2a79a6e`.
const EXPECTED: LiftAggregate = LiftAggregate {
    maps_with_lifts: 53,
    maps_refused: 37,
    plats: PlatCounts {
        plats: 290,
        lifts: 142,
        pedestals: 47,
        barriers: 9,
        refused: 92,
        dead: 13,
        bank_caller: 7,
        shared_split: 1,
        split_members: 2,
        one_shot: 2,
        mixed_speed: 7,
        unsupported_rest: 32,
        top_only: 11,
        one_way_barrier: 10,
        conflicting: 10,
        broken_lines: 6,
        callable_low: 265,
        with_top_trigger: 111,
        with_things: 136,
        fast: 124,
    },
    lift_maps: 40,
    pedestal_maps: 19,
    barrier_maps: 8,
};

/// The IWAD directory, or `None` after printing why the test skips.
fn iwad_dir() -> Option<PathBuf> {
    let Some(raw) = std::env::var_os(VAR) else {
        eprintln!("skipping: {VAR} not set");
        return None;
    };
    let dir = PathBuf::from(raw);
    if !dir.is_absolute() || !dir.is_dir() {
        eprintln!(
            "skipping: {VAR} is not an absolute path to a directory: {}",
            dir.display()
        );
        return None;
    }
    for name in IWADS {
        if !dir.join(name).is_file() {
            eprintln!("skipping: {name} not present in {}", dir.display());
            return None;
        }
    }
    Some(dir)
}

/// A scratch directory holding only the two IWADs, removed on drop even when an assertion panics.
struct Scratch(PathBuf);

impl Scratch {
    /// Creates `<temp_dir>/crustygen-retail-<pid>-<nanos>` and copies the two IWADs into it.
    /// A copy failure is an environment fault, so it panics with the I/O error and the path.
    fn copy_iwads(from: &Path) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir =
            std::env::temp_dir().join(format!("crustygen-retail-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir)
            .unwrap_or_else(|e| panic!("cannot create {}: {e}", dir.display()));
        // Constructed before the copies so a failed copy still removes the directory.
        let scratch = Self(dir);
        for name in IWADS {
            let dest = scratch.0.join(name);
            std::fs::copy(from.join(name), &dest)
                .unwrap_or_else(|e| panic!("cannot copy {name} to {}: {e}", dest.display()));
        }
        scratch
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            eprintln!("could not remove {}: {e}", self.0.display());
        }
    }
}

#[test]
fn the_recognizer_reproduces_the_retail_shape_counts() {
    let Some(dir) = iwad_dir() else { return };
    let scratch = Scratch::copy_iwads(&dir);
    let tables = Tables::load().expect("tables");
    let vocab = Vocabulary::from_tables(&tables);
    let sweep = sweep_dir(&scratch.0, &vocab, &tables).expect("sweep");

    assert!(
        sweep.failures.is_empty(),
        "sweep reported failures: {:?}",
        sweep.failures
    );
    assert_eq!(
        sweep.buckets, EXPECTED_BUCKETS,
        "the sweep is not the two measured IWADs ({PRINTS}); a different print or an extra copy, not a recognizer change"
    );
    let agg = aggregate(&sweep.maps);
    assert_eq!(
        agg.lifts, EXPECTED,
        "the retail lift aggregate moved: re-measure and add a dated row to {ADDENDUM}; never tune this literal"
    );

    let broken: Vec<(&str, &str, u64)> = sweep
        .maps
        .iter()
        .filter(|m| m.lifts.broken_lines > 0)
        .map(|m| (m.source.as_str(), m.map.as_str(), m.lifts.broken_lines))
        .collect();
    assert_eq!(broken, vec![("DOOM.WAD", "E4M5", 6)]);

    let split: Vec<(&str, &str, u64, u64)> = sweep
        .maps
        .iter()
        .filter(|m| m.lifts.shared_split > 0)
        .map(|m| {
            (
                m.source.as_str(),
                m.map.as_str(),
                m.lifts.shared_split,
                m.lifts.split_members,
            )
        })
        .collect();
    assert_eq!(split, vec![("DOOM2.WAD", "MAP14", 1, 2)]);
}
