//! `crustygen-lift` — survey a WAD's maps into raw telemetry (the lifter
//! skeleton; `docs/lift.md`).
//!
//! Usage: `crustygen-lift <wad> [--map NAME] [--json] [--vocabulary]`.
//!
//! Surveys every map group (or just `--map NAME`) through the shared
//! ingestion path — UDMF or classic Doom binary — and prints one
//! human-readable census line per map, or a JSON array with `--json`.
//! `--vocabulary` appends a per-map verdict from `crustygen::lift::vocabulary`,
//! with `crustygen::lift::teleport`'s refusals folded in as a fourth axis
//! (and its per-map counts alongside the verdict under `--json`).
//! Groups that fail to load are named on stderr and skipped; survivors are
//! still reported. Exit 0 when every selected group surveyed, 1 when some
//! failed, 2 on a usage, I/O, or WAD-level failure (every such failure
//! names what failed on stderr).

use crustygen::check::scene::Scene;
use crustygen::ingest::{self, MapOrigin};
use crustygen::lift::teleport::{self, TeleportCounts};
use crustygen::lift::vocabulary::{Verdict, Vocabulary};
use crustygen::lift::{self, MapTelemetry};
use crustygen::tables::Tables;
use crustywad::Wad;

const USAGE: &str = "usage: crustygen-lift <wad> [--map NAME] [--json] [--vocabulary]";

/// One surveyed map's row: its census, which ingest path produced it, and —
/// only under `--vocabulary` — its verdict paired with the teleport counts
/// standing behind that verdict's fourth axis.
type Record = (MapTelemetry, MapOrigin, Option<(Verdict, TeleportCounts)>);

fn main() {
    std::process::exit(real_main());
}

/// The parsed command line: the positional WAD path plus the flags.
struct Args {
    wad_path: String,
    map_name: Option<String>,
    json: bool,
    vocabulary: bool,
}

/// Hand-rolled argument parsing, mirroring `crustygen-check`'s: one
/// positional `<wad>`, `--map NAME`, and the boolean `--json`/`--vocabulary`.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut wad_path = None;
    let mut map_name = None;
    let mut json = false;
    let mut vocabulary = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--map" => map_name = Some(args.next().ok_or("--map requires a value")?),
            "--json" => json = true,
            "--vocabulary" => vocabulary = true,
            flag if flag.starts_with("--") => return Err(format!("unknown flag `{flag}`")),
            positional if wad_path.is_none() => wad_path = Some(positional.to_owned()),
            extra => return Err(format!("unexpected extra argument `{extra}`")),
        }
    }

    wad_path
        .ok_or_else(|| "missing <wad> path".to_owned())
        .map(|wad_path| Args {
            wad_path,
            map_name,
            json,
            vocabulary,
        })
}

fn real_main() -> i32 {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("crustygen-lift: {msg}");
            eprintln!("{USAGE}");
            return 2;
        }
    };

    match survey_wad(&args) {
        Ok(exit_code) => exit_code,
        Err(msg) => {
            eprintln!("crustygen-lift: {msg}");
            2
        }
    }
}

/// Reads the WAD, surveys the selected groups, prints the report, and
/// returns the exit code (0 all surveyed, 1 some groups failed).
fn survey_wad(args: &Args) -> Result<i32, String> {
    let bytes = std::fs::read(&args.wad_path)
        .map_err(|err| format!("failed to read `{}`: {err}", args.wad_path))?;
    let wad = Wad::from_bytes(bytes)
        .map_err(|err| format!("failed to parse `{}` as a WAD: {err}", args.wad_path))?;

    let groups = match args.map_name.as_deref() {
        Some(name) => vec![
            wad.map_group(name)
                .ok_or_else(|| format!("no map group named `{name}` in `{}`", args.wad_path))?,
        ],
        None => wad.map_groups(),
    };
    if groups.is_empty() {
        return Err(format!("`{}` contains no map groups", args.wad_path));
    }

    // Loaded once, up front, when `--vocabulary` is set — `classify` is
    // cheap and the same `Vocabulary` classifies every surveyed map. The
    // `Tables` are kept alongside it: the teleport recognizer reads them
    // directly, not through the vocabulary's membership sets.
    let vocab = if args.vocabulary {
        let tables = Tables::load().map_err(|err| format!("tables: {err}"))?;
        let vocabulary = Vocabulary::from_tables(&tables);
        Some((tables, vocabulary))
    } else {
        None
    };

    let mut records: Vec<Record> = Vec::new();
    let mut failures = 0usize;
    for group in &groups {
        match ingest::load_map(&wad, group) {
            Ok(loaded) => {
                // COVERAGE: the loop body is unreachable today — under strict
                // `WriteOptions` the only warning a Doom-format binary map can
                // produce is `NamespaceDefaulted`, which `ingest` filters out.
                // Future-proofing for warnings a later crustywad could surface.
                for note in &loaded.notes {
                    eprintln!("crustygen-lift: note: map `{}`: {note}", group.name);
                }
                let telemetry = lift::survey(&group.name, &loaded.map);
                let verdict = vocab.as_ref().map(|(tables, vocabulary)| {
                    // `Scene::build`'s findings are dropped: this is a
                    // survey, not a verification run. The recognizer reads
                    // whatever boundaries resolved and refuses what it
                    // cannot state — `crustygen-check` is where structural
                    // faults get reported.
                    let scene = Scene::build(&loaded.map, tables, &mut Vec::new());
                    let report = teleport::recognize(&scene, tables);
                    (
                        vocabulary.classify(&telemetry).with_teleports(&report),
                        report.counts,
                    )
                });
                records.push((telemetry, loaded.origin, verdict));
            }
            Err(err) => {
                failures += 1;
                eprintln!("crustygen-lift: map `{}`: {err}", group.name);
            }
        }
    }

    if args.json {
        let text = if args.vocabulary {
            let mut values = Vec::with_capacity(records.len());
            for (telemetry, _, verdict) in &records {
                let mut value = serde_json::to_value(telemetry)
                    .map_err(|err| format!("failed to serialize telemetry: {err}"))?;
                let (verdict, teleports) = verdict
                    .as_ref()
                    .expect("--vocabulary set: every record carries a verdict");
                let object = value
                    .as_object_mut()
                    .expect("MapTelemetry serializes to a JSON object");
                object.insert(
                    "verdict".to_owned(),
                    serde_json::to_value(verdict)
                        .map_err(|err| format!("failed to serialize verdict: {err}"))?,
                );
                object.insert(
                    "teleports".to_owned(),
                    serde_json::to_value(teleports)
                        .map_err(|err| format!("failed to serialize teleport counts: {err}"))?,
                );
                values.push(value);
            }
            serde_json::to_string_pretty(&values)
        } else {
            let telemetry: Vec<&MapTelemetry> = records.iter().map(|(t, _, _)| t).collect();
            serde_json::to_string_pretty(&telemetry)
        }
        .map_err(|err| format!("failed to serialize telemetry: {err}"))?;
        println!("{text}");
    } else {
        for (telemetry, origin, verdict) in &records {
            let suffix = verdict
                .as_ref()
                .map(|(v, counts)| verdict_suffix(v, counts))
                .unwrap_or_default();
            println!("{}{suffix}", human_line(telemetry, *origin));
        }
    }

    Ok(i32::from(failures > 0))
}

/// One human-readable census line for a surveyed map.
fn human_line(t: &MapTelemetry, origin: MapOrigin) -> String {
    let origin_note = match origin {
        MapOrigin::Udmf => "",
        MapOrigin::AssembledFromBinary => " (assembled from binary format)",
    };
    format!(
        "{}: {} vertices, {} linedefs, {} sidedefs, {} sectors, {} things; \
         {} distinct linedef special(s), {} distinct sector special(s), \
         {} distinct thing type(s){origin_note}",
        t.map,
        t.census.vertices,
        t.census.linedefs,
        t.census.sidedefs,
        t.census.sectors,
        t.census.things,
        t.linedef_specials.len(),
        t.sector_specials.len(),
        t.thing_types.len(),
    )
}

/// The `--vocabulary` suffix appended to a [`human_line`] census line: the
/// overall verdict, an ok/unknown breakdown per membership axis, a
/// vanilla-only note when the map leaves the pinned engine's vanilla special
/// list, and — only when the teleport recognizer refused something — how
/// many of its lines it refused. A map with no teleport line, and a map
/// whose every teleport line was recognized, both read the same: silence on
/// that axis, because there is nothing there the lifter would have to drop.
fn verdict_suffix(v: &Verdict, teleports: &TeleportCounts) -> String {
    use std::fmt::Write as _;

    let mut s = format!(
        "; expressible: {}",
        if v.expressible { "yes" } else { "no" }
    );
    let mut part = |label: &str, ok: bool, unknown: &[i32]| {
        if ok {
            let _ = write!(s, " ({label} ok)");
        } else {
            let list: Vec<String> = unknown.iter().map(ToString::to_string).collect();
            let _ = write!(s, " ({label} unknown: {})", list.join(" "));
        }
    };
    part(
        "line specials",
        v.line_specials_ok,
        &v.unknown_line_specials,
    );
    part(
        "sector specials",
        v.sector_specials_ok,
        &v.unknown_sector_specials,
    );
    part("thing types", v.thing_kinds_ok, &v.unknown_thing_types);
    if !v.vanilla_only {
        s.push_str(" (outside vanilla)");
    }
    if !v.teleports_ok {
        let _ = write!(s, " (teleports refused: {})", teleports.refusals());
    }
    s
}
