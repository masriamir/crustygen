//! Corpus sweep: every zip/WAD in a directory → per-map census + verdict,
//! deduplicated by content, with every failure counted in a bucket.
//! Aggregation and the report live in the second half of this module.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crustywad::archive::Archive;
use crustywad::map::MapGroup;
use crustywad::{Limits, ParseOptions, Strictness, Wad};
use sha2::{Digest, Sha256};

use crate::ingest::{self, IngestError, MapOrigin};
use crate::lift::vocabulary::{Verdict, Vocabulary};
use crate::lift::{self, MapTelemetry};

/// One surveyed, classified map.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MapRecord {
    /// `<file>` for a bare WAD, `<file>!<member>` for a zip member.
    pub source: String,
    /// The map group name.
    pub map: String,
    /// `"udmf"` or `"assembled_from_binary"`.
    pub origin: String,
    /// `sha256:<hex>` over the map's lumps (see [`map_hash`]).
    pub hash: String,
    /// The raw census.
    pub telemetry: MapTelemetry,
    /// The vocabulary verdict.
    pub verdict: Verdict,
}

/// Every count a sweep produces. Failure buckets are per candidate (an
/// archive, a WAD, or a map), never aborting the sweep.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct Buckets {
    /// Zips opened.
    pub archives: u64,
    /// WADs read (bare files plus zip members).
    pub wads: u64,
    /// Maps surveyed, duplicates included.
    pub maps_raw: u64,
    /// Distinct maps by content hash.
    pub maps_unique: u64,
    /// Zips that failed to open.
    pub archive_unreadable: u64,
    /// WADs (bare or member) that failed to parse.
    pub wad_unreadable: u64,
    /// WADs with no map group (resource WADs).
    pub no_maps: u64,
    /// Hexen/Doom 64-format maps the ingest path refuses.
    pub unsupported_format: u64,
    /// Binary maps that failed strict assembly.
    pub assembly_refused: u64,
    /// A `TEXTMAP` that is not UTF-8 or fails to parse, plus binary maps
    /// that failed the UDMF round trip.
    pub textmap_unparseable: u64,
}

/// A whole sweep's output.
#[derive(Debug)]
pub struct Sweep {
    /// The counts.
    pub buckets: Buckets,
    /// Unique maps, in discovery order.
    pub maps: Vec<MapRecord>,
    /// One line per counted failure, for stderr.
    pub failures: Vec<String>,
}

/// Errors that stop a sweep before it starts.
#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    /// The directory could not be listed, or an output could not be written.
    #[error("{path}: {source}")]
    Io {
        /// What was being accessed.
        path: String,
        /// The underlying error.
        #[source]
        source: std::io::Error,
    },
    /// Nothing in the directory ends in `.zip` or `.wad`.
    #[error("no .zip or .wad candidates in {dir}")]
    NoCandidates {
        /// The swept directory.
        dir: String,
    },
}

/// `sha256:<hex>` over `name, len(u64 LE), bytes` for each of the group's
/// data lumps in order — identical maps repackaged across zips hash equal.
///
/// # Panics
///
/// If a lump's length does not fit a `u64`, which no platform this crate
/// builds on can produce (`usize` is at most 64 bits everywhere).
#[must_use]
pub fn map_hash(wad: &Wad, group: &MapGroup) -> String {
    let lumps = wad.lumps();
    let mut hasher = Sha256::new();
    for &i in &group.data_indices {
        let data = wad.lump_data(&lumps[i]);
        hasher.update(lumps[i].name().as_bytes());
        hasher.update(u64::try_from(data.len()).expect("fits u64").to_le_bytes());
        hasher.update(data);
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn has_ext(path: &Path, ext: &str) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case(ext))
}

/// Lenient archive options: idgames zips carry non-ASCII member names and
/// odd extras that strict mode would refuse outright; a warning-tolerant
/// open still verifies every CRC.
fn archive_options() -> ParseOptions {
    ParseOptions {
        strictness: Strictness::Lenient,
        limits: Limits::default(),
    }
}

/// Surveys `dir` (non-recursive). See the module docs.
///
/// # Errors
/// [`CorpusError::Io`] when `dir` cannot be listed; [`CorpusError::NoCandidates`]
/// when it holds no `.zip`/`.wad` file.
///
/// # Panics
///
/// If the sweep collects more than `u64::MAX` unique maps, which no
/// directory of files can hold.
pub fn sweep_dir(dir: &Path, vocab: &Vocabulary) -> Result<Sweep, CorpusError> {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|source| CorpusError::Io {
            path: dir.display().to_string(),
            source,
        })?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && (has_ext(p, "zip") || has_ext(p, "wad")))
        .collect();
    candidates.sort();
    if candidates.is_empty() {
        return Err(CorpusError::NoCandidates {
            dir: dir.display().to_string(),
        });
    }
    let mut sweep = Sweep {
        buckets: Buckets::default(),
        maps: Vec::new(),
        failures: Vec::new(),
    };
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for path in &candidates {
        let label = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if has_ext(path, "zip") {
            match Archive::from_path_with_options(path, archive_options()) {
                Ok(archive) => {
                    sweep.buckets.archives += 1;
                    for member in archive
                        .members()
                        .iter()
                        .filter(|m| has_ext(Path::new(m.path()), "wad"))
                    {
                        let source = format!("{label}!{}", member.path());
                        match archive.wad(member) {
                            Ok(wad) => survey_wad(&wad, &source, vocab, &mut sweep, &mut seen),
                            Err(err) => {
                                sweep.buckets.wad_unreadable += 1;
                                sweep.failures.push(format!("{source}: {err}"));
                            }
                        }
                    }
                }
                Err(err) => {
                    sweep.buckets.archive_unreadable += 1;
                    sweep.failures.push(format!("{label}: {err}"));
                }
            }
        } else {
            match Wad::from_path(path) {
                Ok(wad) => survey_wad(&wad, &label, vocab, &mut sweep, &mut seen),
                Err(err) => {
                    sweep.buckets.wad_unreadable += 1;
                    sweep.failures.push(format!("{label}: {err}"));
                }
            }
        }
    }
    sweep.buckets.maps_unique = u64::try_from(sweep.maps.len()).expect("fits u64");
    Ok(sweep)
}

fn survey_wad(
    wad: &Wad,
    source: &str,
    vocab: &Vocabulary,
    sweep: &mut Sweep,
    seen: &mut BTreeSet<String>,
) {
    sweep.buckets.wads += 1;
    let groups = wad.map_groups();
    if groups.is_empty() {
        sweep.buckets.no_maps += 1;
        sweep.failures.push(format!("{source}: no map groups"));
        return;
    }
    for group in &groups {
        match ingest::load_map(wad, group) {
            Ok(loaded) => {
                sweep.buckets.maps_raw += 1;
                let hash = map_hash(wad, group);
                if !seen.insert(hash.clone()) {
                    continue;
                }
                let telemetry = lift::survey(&group.name, &loaded.map);
                let verdict = vocab.classify(&telemetry);
                sweep.maps.push(MapRecord {
                    source: source.to_owned(),
                    map: group.name.clone(),
                    origin: match loaded.origin {
                        MapOrigin::Udmf => "udmf".to_owned(),
                        MapOrigin::AssembledFromBinary => "assembled_from_binary".to_owned(),
                    },
                    hash,
                    telemetry,
                    verdict,
                });
            }
            Err(err) => {
                match err {
                    IngestError::UnsupportedBinaryFormat { .. } => {
                        sweep.buckets.unsupported_format += 1;
                    }
                    IngestError::Assemble(_) => sweep.buckets.assembly_refused += 1,
                    IngestError::NonUtf8Textmap(_)
                    | IngestError::UdmfParse(_)
                    | IngestError::Render(_)
                    | IngestError::Reparse(_) => sweep.buckets.textmap_unparseable += 1,
                }
                sweep
                    .failures
                    .push(format!("{source} {}: {err}", group.name));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crustywad::{WadBuilder, WadKind};

    #[test]
    fn has_ext_is_ascii_case_insensitive() {
        assert!(has_ext(Path::new("a/B.ZIP"), "zip"));
        assert!(has_ext(Path::new("x.Wad"), "wad"));
        assert!(!has_ext(Path::new("x.txt"), "wad"));
        assert!(!has_ext(Path::new("noext"), "wad"));
    }

    #[test]
    fn map_hash_depends_on_lump_bytes_and_names_not_on_marker() {
        let build = |marker: &str, payload: &[u8]| {
            WadBuilder::new(WadKind::Pwad)
                .add_lump(marker, Vec::new())
                .add_lump("TEXTMAP", payload.to_vec())
                .add_lump("ENDMAP", Vec::new())
                .build()
                .expect("builds")
        };
        let a = Wad::from_bytes(build("MAP01", b"namespace = \"doom\";")).unwrap();
        let b = Wad::from_bytes(build("MAP02", b"namespace = \"doom\";")).unwrap();
        let c = Wad::from_bytes(build("MAP01", b"namespace = \"doom\"; ")).unwrap();
        let h = |w: &Wad| map_hash(w, &w.map_groups()[0]);
        assert_eq!(
            h(&a),
            h(&b),
            "same lumps under another marker are the same map"
        );
        assert_ne!(h(&a), h(&c));
        assert!(h(&a).starts_with("sha256:"));
    }
}
