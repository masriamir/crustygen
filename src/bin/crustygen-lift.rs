//! `crustygen-lift` — survey a WAD's maps into raw telemetry (the lifter
//! skeleton; `docs/lift.md`).
//!
//! Usage: `crustygen-lift <wad> [--map NAME] [--json]`.
//!
//! Surveys every map group (or just `--map NAME`) through the shared
//! ingestion path — UDMF or classic Doom binary — and prints one
//! human-readable census line per map, or a JSON array with `--json`.
//! Groups that fail to load are named on stderr and skipped; survivors are
//! still reported. Exit 0 when every selected group surveyed, 1 when some
//! failed, 2 on a usage, I/O, or WAD-level failure (every such failure
//! names what failed on stderr).

use crustygen::ingest::{self, MapOrigin};
use crustygen::lift::{self, MapTelemetry};
use crustywad::Wad;

const USAGE: &str = "usage: crustygen-lift <wad> [--map NAME] [--json]";

fn main() {
    std::process::exit(real_main());
}

/// The parsed command line: the positional WAD path plus the two flags.
struct Args {
    wad_path: String,
    map_name: Option<String>,
    json: bool,
}

/// Hand-rolled argument parsing, mirroring `crustygen-check`'s: one
/// positional `<wad>`, `--map NAME`, and the boolean `--json`.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut wad_path = None;
    let mut map_name = None;
    let mut json = false;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--map" => map_name = Some(args.next().ok_or("--map requires a value")?),
            "--json" => json = true,
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

    let mut records: Vec<(MapTelemetry, MapOrigin)> = Vec::new();
    let mut failures = 0usize;
    for group in &groups {
        match ingest::load_map(&wad, group) {
            Ok(loaded) => {
                for note in &loaded.notes {
                    eprintln!("crustygen-lift: note: map `{}`: {note}", group.name);
                }
                records.push((lift::survey(&group.name, &loaded.map), loaded.origin));
            }
            Err(err) => {
                failures += 1;
                eprintln!("crustygen-lift: map `{}`: {err}", group.name);
            }
        }
    }

    if args.json {
        let telemetry: Vec<&MapTelemetry> = records.iter().map(|(t, _)| t).collect();
        let text = serde_json::to_string_pretty(&telemetry)
            .map_err(|err| format!("failed to serialize telemetry: {err}"))?;
        println!("{text}");
    } else {
        for (telemetry, origin) in &records {
            println!("{}", human_line(telemetry, *origin));
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
