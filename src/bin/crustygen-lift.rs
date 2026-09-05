//! `crustygen-lift` — survey a WAD's maps into raw telemetry (the lifter
//! skeleton; `docs/lift.md`).
//!
//! Usage: `crustygen-lift <wad> [--map NAME] [--json] [--vocabulary]`.
//!
//! Surveys every map group (or just `--map NAME`) through the shared
//! ingestion path — UDMF or classic Doom binary — and prints one
//! human-readable census line per map, or a JSON array with `--json`.
//! `--vocabulary` appends a per-map verdict from `crustygen::lift::vocabulary`,
//! with `crustygen::lift::teleport`'s refusals folded in as a fourth axis,
//! `crustygen::lift::plat`'s as a fifth and `crustygen::lift::floor`'s as a
//! sixth (and all three recognizers' per-map counts alongside the verdict
//! under `--json`).
//! Groups that fail to load are named on stderr and skipped; survivors are
//! still reported. Exit 0 when every selected group surveyed, 1 when some
//! failed, 2 on a usage, I/O, or WAD-level failure (every such failure
//! names what failed on stderr).

use crustygen::check::scene::Scene;
use crustygen::ingest::{self, MapOrigin};
use crustygen::lift::floor::{self, FloorCounts};
use crustygen::lift::plat::{self, PlatCounts};
use crustygen::lift::teleport::{self, TeleportCounts};
use crustygen::lift::vocabulary::{Verdict, Vocabulary};
use crustygen::lift::{self, MapTelemetry};
use crustygen::tables::Tables;
use crustywad::Wad;
use crustywad::map::udmf::UdmfMap;

const USAGE: &str = "usage: crustygen-lift <wad> [--map NAME] [--json] [--vocabulary]";

/// One surveyed map's row: its census, which ingest path produced it, and —
/// only under `--vocabulary` — its verdict paired with the teleport, plat and
/// floor counts standing behind that verdict's fourth, fifth and sixth axes.
type Record = (
    MapTelemetry,
    MapOrigin,
    Option<(Verdict, TeleportCounts, PlatCounts, FloorCounts)>,
);

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
                    classify_and_recognize(tables, vocabulary, &telemetry, &loaded.map)
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
                let (verdict, teleports, lifts, floors) = verdict
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
                object.insert(
                    "lifts".to_owned(),
                    serde_json::to_value(lifts)
                        .map_err(|err| format!("failed to serialize plat counts: {err}"))?,
                );
                object.insert(
                    "floors".to_owned(),
                    serde_json::to_value(floors)
                        .map_err(|err| format!("failed to serialize floor counts: {err}"))?,
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
                .map(|(v, teleports, lifts, floors)| verdict_suffix(v, teleports, lifts, floors))
                .unwrap_or_default();
            println!("{}{suffix}", human_line(telemetry, *origin));
        }
    }

    Ok(i32::from(failures > 0))
}

/// The membership verdict for `telemetry`, with whichever recognizers the
/// map's linedef specials call for folded into it, and their counts.
///
/// The histogram is already computed: a map carrying none of the four
/// teleport specials among its linedef specials has no teleport line for the
/// recognizer to recognize, one carrying none of the eight lift specials has
/// no platform, and one carrying none of the 48 recognized floor specials has
/// no floor target, so each recognizer is skipped when its specials are
/// absent — the verdict then stays `Vocabulary::classify`'s own
/// (`teleports_ok` / `lifts_ok` / `floors_ok == true`) and the counts are the
/// all-zero default. The `Scene` is built once and shared by all three.
fn classify_and_recognize(
    tables: &Tables,
    vocabulary: &Vocabulary,
    telemetry: &MapTelemetry,
    map: &UdmfMap,
) -> (Verdict, TeleportCounts, PlatCounts, FloorCounts) {
    let mut verdict = vocabulary.classify(telemetry);
    let has = |specials: &[u16]| {
        specials
            .iter()
            .any(|&s| telemetry.linedef_specials.contains_key(&i32::from(s)))
    };
    let has_teleports = has(&tables.teleport_specials());
    let has_lifts = has(&tables.lift_specials());
    // No intermediate `Vec<u16>`: scan the borrowed recognized-floor-specials
    // slice directly, the same way `lift::corpus::survey_wad`'s identical
    // gate does.
    let has_floors = tables
        .recognized_floor_specials()
        .iter()
        .any(|&(special, _, _)| telemetry.linedef_specials.contains_key(&i32::from(special)));
    let mut teleports = TeleportCounts::default();
    let mut lifts = PlatCounts::default();
    let mut floors = FloorCounts::default();
    if has_teleports || has_lifts || has_floors {
        // `Scene::build`'s findings are dropped: this is a survey, not a
        // verification run. The recognizers read whatever boundaries
        // resolved and refuse what they cannot state — `crustygen-check` is
        // where structural faults get reported.
        let scene = Scene::build(map, tables, &mut Vec::new());
        if has_teleports {
            let report = teleport::recognize(&scene, tables);
            verdict = verdict.with_teleports(&report);
            teleports = report.counts;
        }
        if has_lifts {
            let report = plat::recognize(&scene, tables);
            verdict = verdict.with_lifts(&report);
            lifts = report.counts;
        }
        if has_floors {
            let report = floor::recognize(&scene, tables);
            verdict = verdict.with_floors(&report);
            floors = report.counts;
        }
    }
    (verdict, teleports, lifts, floors)
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
/// list, and — only when a recognizer refused something — how many lines,
/// platforms or floor targets it refused. A map with no teleport line, and a
/// map whose every teleport line was recognized, both read the same: silence
/// on that axis, because there is nothing there the lifter would have to
/// drop. The lift and floor axes read the same way.
fn verdict_suffix(
    v: &Verdict,
    teleports: &TeleportCounts,
    lifts: &PlatCounts,
    floors: &FloorCounts,
) -> String {
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
    if !v.lifts_ok {
        let _ = write!(s, " (lifts refused: {})", lifts.refusals());
    }
    if !v.floors_ok {
        let _ = write!(s, " (floors refused: {})", floors.refusals());
    }
    s
}
