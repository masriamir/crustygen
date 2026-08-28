//! Corpus sweep: every zip/WAD in a directory → per-map census + verdict,
//! deduplicated by content, with every failure counted in a bucket.
//! Aggregation and the report live in the second half of this module.

use std::collections::{BTreeMap, BTreeSet};
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

impl Buckets {
    /// Candidates that failed to load — every failure bucket except
    /// `no_maps`. A WAD carrying no map group loaded fine and is ordinary
    /// corpus content (a resource WAD), so it is counted and named but must
    /// not read as a failure; on a real idgames sample it would otherwise
    /// make a non-zero exit the norm.
    #[must_use]
    pub fn load_failures(&self) -> u64 {
        self.archive_unreadable
            + self.wad_unreadable
            + self.unsupported_format
            + self.assembly_refused
            + self.textmap_unparseable
    }
}

/// A whole sweep's output.
#[derive(Debug)]
pub struct Sweep {
    /// The counts.
    pub buckets: Buckets,
    /// Unique maps, in discovery order.
    pub maps: Vec<MapRecord>,
    /// One line per counted failure *and* per map-free WAD, for stderr.
    /// A map-free WAD is ordinary corpus content, so these lines are wider
    /// than the exit-code signal: use [`Buckets::load_failures`] for that.
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

/// Lenient archive options: idgames zips carry non-ASCII member names and odd
/// extras that strict mode would refuse outright; a warning-tolerant open
/// still verifies every CRC. `Archive::wad` reuses these same options to
/// parse each member WAD, so a member that only warns under lenient parsing
/// still loads — while the identical bytes as a bare `.wad` file go through
/// strict `Wad::from_path` below and land in `wad_unreadable`. That asymmetry
/// is kept: the sample of record is all zips, so no bare `.wad` file
/// exercises the strict path in practice.
fn archive_options() -> ParseOptions {
    ParseOptions {
        strictness: Strictness::Lenient,
        limits: Limits::default(),
    }
}

/// Surveys `dir` (non-recursive). See the module docs.
///
/// An entry the directory listing itself cannot read (a permission error
/// mid-listing) is skipped silently — no bucket, no failure line.
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
        // COVERAGE: the `None` fallback is unreachable here. Every candidate
        // came from a `read_dir` entry, and such a path always ends in the
        // entry's own file name — `file_name` only returns `None` for a path
        // ending in `..` or a bare root, neither of which a listing yields.
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

/// Where the greedy curve is sampled for the report.
pub const GREEDY_CHECKPOINTS: [usize; 5] = [1, 5, 10, 21, 51];
/// How many blockers per axis the report lists.
pub const BLOCKER_TOP: usize = 25;

/// Expressibility per axis as a fraction of a population of maps.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct AxisShare {
    /// Fraction whose line specials are all emittable.
    pub line_specials: f64,
    /// Fraction whose sector specials are all nameable.
    pub sector_specials: f64,
    /// Fraction whose thing types are all in vocabulary.
    pub thing_kinds: f64,
    /// Fraction expressible on all three.
    pub expressible: f64,
}

/// One out-of-set value and how many maps carry it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Blocker {
    /// The raw value.
    pub value: i32,
    /// Maps carrying it.
    pub maps: u64,
    /// `maps / maps_unique`.
    pub share: f64,
}

/// One greedy step.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct GreedyStep {
    /// 1-based step.
    pub k: usize,
    /// The special added at this step.
    pub special: i32,
    /// Fraction of all unique maps unblocked after this step.
    pub cumulative_share: f64,
}

/// The greedy "add the special that unblocks the most maps" curve.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct GreedyCurve {
    /// Every step until nothing is left to unblock.
    pub steps: Vec<GreedyStep>,
    /// `(k, cumulative_share)` at each of [`GREEDY_CHECKPOINTS`] that the
    /// curve reaches (the share at the last step is used past its end).
    pub checkpoints: Vec<(usize, f64)>,
}

/// The whole-sweep summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Aggregate {
    /// Population size.
    pub maps_unique: u64,
    /// Shares over every unique map.
    pub all: AxisShare,
    /// Fraction of unique maps inside the vanilla special list.
    pub vanilla_share: f64,
    /// Shares over the vanilla-only slice.
    pub vanilla: AxisShare,
    /// Top out-of-set line specials by map share.
    pub line_blockers: Vec<Blocker>,
    /// Top out-of-set sector specials by map share.
    pub sector_blockers: Vec<Blocker>,
    /// Top out-of-set thing types by map share.
    pub thing_blockers: Vec<Blocker>,
    /// Greedy curve with thing kinds and sector specials held expressible.
    pub greedy_line_axis: GreedyCurve,
    /// Greedy curve over maps already ok on the other two axes. Its shares
    /// are still over every unique map, so it plateaus below 1.0 whenever
    /// some map is blocked on a sector special or a thing kind.
    pub greedy_conjunction: GreedyCurve,
}

/// `n / of` as a fraction, with an empty population scoring 0.
#[expect(
    clippy::cast_precision_loss,
    reason = "map counts are always far under f64's 52-bit mantissa"
)]
fn share(n: usize, of: usize) -> f64 {
    if of == 0 { 0.0 } else { n as f64 / of as f64 }
}

fn axis_share(maps: &[&MapRecord]) -> AxisShare {
    let n = maps.len();
    let count = |f: fn(&Verdict) -> bool| share(maps.iter().filter(|m| f(&m.verdict)).count(), n);
    AxisShare {
        line_specials: count(|v| v.line_specials_ok),
        sector_specials: count(|v| v.sector_specials_ok),
        thing_kinds: count(|v| v.thing_kinds_ok),
        expressible: count(|v| v.expressible),
    }
}

/// The out-of-set values `pick` reports, ranked by how many maps carry them
/// (ties by ascending value) and cut to [`BLOCKER_TOP`]. Each `unknown_*`
/// list is already deduplicated, so a count is a map count.
///
/// # Panics
///
/// If a count does not fit a `u64`, which needs more maps than a directory
/// of files can hold.
fn blockers(maps: &[MapRecord], pick: fn(&Verdict) -> &[i32]) -> Vec<Blocker> {
    let mut counts: BTreeMap<i32, usize> = BTreeMap::new();
    for m in maps {
        for &v in pick(&m.verdict) {
            *counts.entry(v).or_insert(0) += 1;
        }
    }
    let mut out: Vec<Blocker> = counts
        .into_iter()
        .map(|(value, n)| Blocker {
            value,
            maps: u64::try_from(n).expect("fits u64"),
            share: share(n, maps.len()),
        })
        .collect();
    out.sort_by(|a, b| b.maps.cmp(&a.maps).then(a.value.cmp(&b.value)));
    out.truncate(BLOCKER_TOP);
    out
}

/// Greedy set cover over `population`: each step adds the out-of-set
/// special that turns the most still-blocked maps expressible on the line
/// axis; ties go to the smaller special number. `population` is the set of
/// maps eligible to be unblocked; `total` is the denominator.
fn greedy(population: &[&MapRecord], total: usize) -> GreedyCurve {
    let mut remaining: Vec<BTreeSet<i32>> = population
        .iter()
        .map(|m| m.verdict.unknown_line_specials.iter().copied().collect())
        .filter(|s: &BTreeSet<i32>| !s.is_empty())
        .collect();
    let mut unblocked = population.len() - remaining.len();
    let mut steps = Vec::new();
    while !remaining.is_empty() {
        let mut gain: BTreeMap<i32, usize> = BTreeMap::new();
        for set in &remaining {
            if set.len() == 1 {
                *gain
                    .entry(*set.iter().next().expect("non-empty"))
                    .or_insert(0) += 1;
            }
        }
        // A special that alone completes no map still makes progress; when
        // no single special completes any map, add the one carried by the
        // most maps.
        let best = if gain.is_empty() {
            let mut carried: BTreeMap<i32, usize> = BTreeMap::new();
            for set in &remaining {
                for &s in set {
                    *carried.entry(s).or_insert(0) += 1;
                }
            }
            carried
                .into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(s, _)| s)
        } else {
            gain.into_iter()
                .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
                .map(|(s, _)| s)
        };
        // Unreachable: the `while` guard keeps `remaining` non-empty, and
        // `retain` above drops every empty set, so at least one non-empty
        // set survives into `carried` (or `gain`) each iteration — `best`
        // is always `Some`. Kept as a defensive exit rather than an
        // `expect`, since a future change to either invariant should fail
        // safely here, not panic.
        let Some(best) = best else { break };
        for set in &mut remaining {
            set.remove(&best);
        }
        let before = remaining.len();
        remaining.retain(|s| !s.is_empty());
        unblocked += before - remaining.len();
        steps.push(GreedyStep {
            k: steps.len() + 1,
            special: best,
            cumulative_share: share(unblocked, total),
        });
    }
    let checkpoints = GREEDY_CHECKPOINTS
        .iter()
        .filter_map(|&k| {
            steps
                .get(k.min(steps.len()).checked_sub(1)?)
                .map(|s| (k, s.cumulative_share))
        })
        .collect();
    GreedyCurve { steps, checkpoints }
}

/// Summarizes a sweep's unique maps.
///
/// # Panics
///
/// If the sweep holds more than `u64::MAX` maps, which no directory of
/// files can produce.
#[must_use]
pub fn aggregate(maps: &[MapRecord]) -> Aggregate {
    let all: Vec<&MapRecord> = maps.iter().collect();
    let vanilla: Vec<&MapRecord> = maps.iter().filter(|m| m.verdict.vanilla_only).collect();
    let conjunction_pop: Vec<&MapRecord> = maps
        .iter()
        .filter(|m| m.verdict.sector_specials_ok && m.verdict.thing_kinds_ok)
        .collect();
    Aggregate {
        maps_unique: u64::try_from(maps.len()).expect("fits u64"),
        all: axis_share(&all),
        vanilla_share: share(vanilla.len(), maps.len()),
        vanilla: axis_share(&vanilla),
        line_blockers: blockers(maps, |v| v.unknown_line_specials.as_slice()),
        sector_blockers: blockers(maps, |v| v.unknown_sector_specials.as_slice()),
        thing_blockers: blockers(maps, |v| v.unknown_thing_types.as_slice()),
        greedy_line_axis: greedy(&all, maps.len()),
        greedy_conjunction: greedy(&conjunction_pop, maps.len()),
    }
}

/// The identity of a sample, echoed from `sample-manifest.json` when the
/// swept directory holds one (written by crustywad's `xtask harvest-sample`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Provenance {
    /// The draw's seed.
    pub seed: u64,
    /// The requested sample size.
    pub count: usize,
    /// Rows in the sampling frame.
    pub frame_rows: usize,
    /// Hash of the fetch list the frame was cut from.
    pub fetch_list_hash: String,
    /// Sampled archive ids, ascending.
    pub ids: Vec<u64>,
}

/// Reads `<dir>/sample-manifest.json`; `None` when absent or unparseable.
///
/// An absent manifest is silent — most swept directories are not samples.
/// One that is present but unreadable or malformed warns on stderr, because
/// silently dropping the provenance section would leave a report that looks
/// complete while having lost the identity of its sample. This library
/// function writes to stderr only because stderr is `crustygen-corpus`'s
/// operator channel and this is the sole caller.
#[must_use]
pub fn read_provenance(dir: &Path) -> Option<Provenance> {
    /// One manifest entry; only the archive id is echoed, the rest of the
    /// harvest bookkeeping (directory, filename, size, status) is ignored.
    #[derive(serde::Deserialize)]
    struct Entry {
        id: u64,
    }
    /// The subset of `sample-manifest.json` the report echoes.
    #[derive(serde::Deserialize)]
    struct Manifest {
        seed: u64,
        count: usize,
        frame_rows: usize,
        fetch_list_hash: String,
        entries: Vec<Entry>,
    }
    let path = dir.join("sample-manifest.json");
    let parsed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Manifest>(&text).ok());
    let Some(m) = parsed else {
        // Reached for a read failure as well as a parse failure; `exists`
        // separates "this directory is simply not a sample" from "its
        // manifest is there but unusable", which is the case worth naming.
        if path.exists() {
            eprintln!(
                "crustygen-corpus: sample-manifest.json is unreadable or malformed; \
                 no provenance recorded"
            );
        }
        return None;
    };
    let mut ids: Vec<u64> = m.entries.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    Some(Provenance {
        seed: m.seed,
        count: m.count,
        frame_rows: m.frame_rows,
        fetch_list_hash: m.fetch_list_hash,
        ids,
    })
}

/// The `--json` document.
#[derive(Debug, serde::Serialize)]
pub struct Report<'a> {
    /// Sample identity, when present.
    pub provenance: Option<&'a Provenance>,
    /// The counts.
    pub buckets: &'a Buckets,
    /// The summary.
    pub aggregate: &'a Aggregate,
    /// Every unique map.
    pub maps: &'a [MapRecord],
}

/// A fraction as a one-decimal percentage.
fn pct(x: f64) -> String {
    format!("{:.1} %", x * 100.0)
}

/// The Markdown report: header caveat, provenance, buckets, shares, blockers,
/// greedy checkpoints. Aggregate tables only — no per-map rows.
#[must_use]
pub fn render_markdown(provenance: Option<&Provenance>, b: &Buckets, a: &Aggregate) -> String {
    use std::fmt::Write as _;

    let mut s = String::new();
    s.push_str("# Corpus expressibility\n\n");
    s.push_str(
        "> **Status of these numbers: measured practice, not engine fact — and an upper bound.** ",
    );
    s.push_str("A map counts as expressible when every non-zero line special and sector special, and every thing type, it carries is in crustygen's emittable vocabulary. ");
    s.push_str("Geometry, flags, tags, and texture names are not measured; a geometry-aware lifter can only do worse, never better, than this membership test.\n\n");
    if let Some(p) = provenance {
        s.push_str("## Sample\n\n");
        let _ = writeln!(
            s,
            "- seed `{}`, count {}, frame rows {}, fetch list `{}`",
            p.seed, p.count, p.frame_rows, p.fetch_list_hash
        );
        let ids: Vec<String> = p.ids.iter().map(ToString::to_string).collect();
        let _ = writeln!(s, "- ids: {}\n", ids.join(" "));
    }
    s.push_str("## Buckets\n\n| Bucket | Count |\n|---|---|\n");
    for (name, n) in [
        ("archives", b.archives),
        ("wads", b.wads),
        ("maps_raw", b.maps_raw),
        ("maps_unique", b.maps_unique),
        ("archive_unreadable", b.archive_unreadable),
        ("wad_unreadable", b.wad_unreadable),
        ("no_maps", b.no_maps),
        ("unsupported_format", b.unsupported_format),
        ("assembly_refused", b.assembly_refused),
        ("textmap_unparseable", b.textmap_unparseable),
    ] {
        let _ = writeln!(s, "| `{name}` | {n} |");
    }
    s.push_str(
        "\n## Expressibility\n\n| Axis | All unique maps | Vanilla-only slice |\n|---|---|---|\n",
    );
    for (name, all, van) in [
        (
            "line specials",
            a.all.line_specials,
            a.vanilla.line_specials,
        ),
        (
            "sector specials",
            a.all.sector_specials,
            a.vanilla.sector_specials,
        ),
        ("thing kinds", a.all.thing_kinds, a.vanilla.thing_kinds),
        ("**all three**", a.all.expressible, a.vanilla.expressible),
    ] {
        let _ = writeln!(s, "| {name} | {} | {} |", pct(all), pct(van));
    }
    let _ = writeln!(
        s,
        "\nVanilla-only slice: {} of unique maps.",
        pct(a.vanilla_share)
    );
    for (title, list) in [
        ("Line-special blockers", &a.line_blockers),
        ("Sector-special blockers", &a.sector_blockers),
        ("Thing-type blockers", &a.thing_blockers),
    ] {
        let _ = write!(
            s,
            "\n## {title}\n\n| Value | Maps | Share |\n|---|---|---|\n"
        );
        if list.is_empty() {
            // No out-of-set value on this axis: say so in a row, mirroring
            // the stepless-curve baseline, rather than leaving a bare header.
            s.push_str("| (none) | | |\n");
            continue;
        }
        for bl in list {
            let _ = writeln!(s, "| {} | {} | {} |", bl.value, bl.maps, pct(bl.share));
        }
    }
    render_curves(&mut s, a);
    s
}

/// The two greedy-curve sections. Both curves are scored against *all* unique
/// maps, never against the population they walk — spelled out in each table's
/// header and note, so the conjunction curve's plateau below 100 % reads as
/// the sector- and thing-blocked remainder it is, not as a truncated curve.
fn render_curves(s: &mut String, a: &Aggregate) {
    use std::fmt::Write as _;

    for (title, note, curve, baseline) in [
        (
            "Greedy curve — line axis alone",
            "Share is of all unique maps, with sector specials and thing kinds held expressible.",
            &a.greedy_line_axis,
            a.all.line_specials,
        ),
        (
            "Greedy curve — conjunction (maps already ok on sectors and things)",
            "Share is of **all unique maps**, not of the already-ok population this curve walks, \
             so it plateaus below 100 % by exactly the maps blocked on a sector special or a \
             thing kind.",
            &a.greedy_conjunction,
            a.all.expressible,
        ),
    ] {
        let _ = write!(
            s,
            "\n## {title}\n\n{note}\n\n| k | Cumulative share of all unique maps |\n|---|---|\n"
        );
        if curve.steps.is_empty() {
            // No map in this population is blocked on the line axis (the
            // population may itself be empty), so there is nothing to add
            // greedily: state the standing share at k = 0 rather than
            // emitting a table header with no rows under it.
            let _ = writeln!(s, "| 0 | {} |", pct(baseline));
            s.push_str("\nOrder chosen: (none)\n");
            continue;
        }
        for (k, share) in &curve.checkpoints {
            let _ = writeln!(s, "| {k} | {} |", pct(*share));
        }
        let order: Vec<String> = curve
            .steps
            .iter()
            .take(BLOCKER_TOP)
            .map(|st| st.special.to_string())
            .collect();
        let _ = writeln!(s, "\nOrder chosen: {}", order.join(" → "));
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

    fn record(
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
        }
    }

    #[test]
    fn aggregate_shares_blockers_and_greedy_curve() {
        let maps = vec![
            record(&[1], &[], true, true, true),             // expressible
            record(&[1, 97], &[97], true, true, true),       // blocked by 97
            record(&[97, 62], &[97, 62], true, false, true), // blocked by 97, 62, and a thing
            record(&[8192], &[8192], false, true, false),    // boom, sector-blocked
        ];
        let a = aggregate(&maps);
        assert_eq!(a.maps_unique, 4);
        assert!((a.all.line_specials - 0.25).abs() < 1e-9);
        assert!((a.all.thing_kinds - 0.75).abs() < 1e-9);
        assert!((a.all.expressible - 0.25).abs() < 1e-9);
        assert!((a.vanilla_share - 0.75).abs() < 1e-9);
        assert!((a.vanilla.line_specials - (1.0 / 3.0)).abs() < 1e-9);

        assert_eq!(a.line_blockers[0].value, 97);
        assert_eq!(a.line_blockers[0].maps, 2);
        assert!((a.line_blockers[0].share - 0.5).abs() < 1e-9);
        assert_eq!(a.thing_blockers[0].value, 46);
        assert_eq!(a.sector_blockers[0].value, 4);

        // Line axis alone: +97 unblocks map 2 (and map 3 needs 62 too).
        let g = &a.greedy_line_axis;
        assert_eq!(g.steps[0].special, 97);
        assert!((g.steps[0].cumulative_share - 0.5).abs() < 1e-9);
        assert_eq!(g.steps[1].special, 62);
        assert!((g.steps[1].cumulative_share - 0.75).abs() < 1e-9);
        assert_eq!(g.steps[2].special, 8192);
        assert_eq!(g.checkpoints[0], (1, 0.5));
        // Conjunction: map 3 stays blocked by its thing; map 4 by its sector.
        let c = &a.greedy_conjunction;
        assert_eq!(c.steps[0].special, 97);
        assert!((c.steps[0].cumulative_share - 0.5).abs() < 1e-9);
        assert!(c.steps.len() <= 3);
    }

    /// The fallback branch: with every remaining map blocked by two
    /// specials, no single special completes a map, so step 1 adds the one
    /// carried by the most maps and unblocks nothing (share 0.0) — the
    /// signature of the fallback having fired.
    #[test]
    fn greedy_falls_back_to_the_most_carried_special_when_no_singleton_completes_a_map() {
        let maps = vec![
            record(&[1, 2], &[1, 2], true, true, true),
            record(&[1, 3], &[1, 3], true, true, true),
        ];
        let g = aggregate(&maps).greedy_line_axis;
        assert_eq!(g.steps.len(), 3);
        assert_eq!(
            g.steps[0].special, 1,
            "carried by both maps, completes neither"
        );
        assert!(
            g.steps[0].cumulative_share.abs() < 1e-9,
            "the fallback step unblocks no map"
        );
        assert_eq!(g.steps[1].special, 2, "singletons tie, the smaller wins");
        assert!((g.steps[1].cumulative_share - 0.5).abs() < 1e-9);
        assert_eq!(g.steps[2].special, 3);
        assert!((g.steps[2].cumulative_share - 1.0).abs() < 1e-9);
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "an empty population must score exactly zero, not merely near-zero — the \
                  approximate form the sibling test uses would pass on a share this test \
                  exists to rule out"
    )]
    fn aggregate_of_nothing_is_all_zero() {
        let a = aggregate(&[]);
        assert_eq!(a.maps_unique, 0);
        assert_eq!(a.all.expressible, 0.0);
        assert!(a.line_blockers.is_empty());
        assert!(a.greedy_line_axis.steps.is_empty());
    }

    #[test]
    fn markdown_carries_the_caveat_and_every_section() {
        let a = aggregate(&[record(&[97], &[97], true, true, true)]);
        let md = render_markdown(None, &Buckets::default(), &a);
        for needle in [
            "upper bound",
            "## Buckets",
            "## Expressibility",
            "## Line-special blockers",
            "## Greedy curve",
            "| 97 | 1 | 100.0 % |",
            // The conjunction curve names its denominator, so a plateau
            // below 100 % reads as the sector-/thing-blocked remainder.
            "Share is of **all unique maps**",
        ] {
            assert!(md.contains(needle), "missing {needle:?}:\n{md}");
        }
        // Sector and thing axes have no blockers here: an empty axis states
        // so in a row rather than leaving a bare table header.
        assert_eq!(md.matches("| (none) | | |").count(), 2, "{md}");
        assert!(!md.contains("## Sample"));
    }

    #[test]
    fn provenance_reads_a_sample_manifest_and_sorts_ids() {
        let dir = std::env::temp_dir().join(format!("crustygen-prov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sample-manifest.json"),
            r#"{"seed":42,"count":2,"frame_rows":9,"fetch_list_hash":"blake3:00","entries":[{"id":7,"dir":"d/","filename":"b.zip","zip_size":1,"status":"ok"},{"id":3,"dir":"d/","filename":"a.zip","zip_size":1,"status":"failed:x"}]}"#,
        )
        .unwrap();
        let p = read_provenance(&dir).expect("parses");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(p.ids, vec![3, 7]);
        assert_eq!(p.seed, 42);
        let md = render_markdown(Some(&p), &Buckets::default(), &aggregate(&[]));
        assert!(md.contains("ids: 3 7"), "{md}");
        // An empty aggregate leaves both greedy curves stepless: each gets a
        // single baseline row rather than a table header with nothing under it.
        assert_eq!(md.matches("| 0 | 0.0 % |").count(), 2, "{md}");
        assert_eq!(md.matches("Order chosen: (none)").count(), 2, "{md}");
        assert_eq!(read_provenance(Path::new("/nonexistent-dir")), None);

        // Present but malformed: refused, and named on stderr.
        let bad = std::env::temp_dir().join(format!(
            "crustygen-prov-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("sample-manifest.json"), b"{ not json").unwrap();
        let got = read_provenance(&bad);
        std::fs::remove_dir_all(&bad).ok();
        assert_eq!(got, None);
    }
}
