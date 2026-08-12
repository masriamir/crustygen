//! `crustygen-check` — layer-4 verifier CLI (docs/design.md §8).
//!
//! Usage: `crustygen-check <wad> [--map NAME] [--spec FILE]`.
//!
//! Reads `<wad>`, selects a map group (`--map NAME`, else the WAD's first map
//! group), parses its `TEXTMAP` lump, and runs [`crustygen::check::run`]
//! against it — optionally judging conformance against a spec document
//! (`--spec FILE`). Findings print one per line via their `Display`;
//! conformance rows (when a spec was supplied) follow as
//! `parameter: verdict (target X, actual Y)`; a one-line summary closes the
//! output. Exit 0 for a clean map, 1 if any finding is `Severity::Error`, 2
//! on any usage, I/O, or parse failure — every such failure names what
//! failed on stderr.

use crustygen::check::{self, CheckReport, Severity, Verdict};
use crustygen::spec::{Spec, SpecDocument};
use crustygen::tables::Tables;
use crustywad::map::MapGroup;
use crustywad::map::udmf::parse_udmf;
use crustywad::{Limits, Wad};

const USAGE: &str = "usage: crustygen-check <wad> [--map NAME] [--spec FILE]";

fn main() {
    std::process::exit(real_main());
}

/// The parsed command line: the positional WAD path, plus the two optional
/// `--map`/`--spec` values.
struct Args {
    wad_path: String,
    map_name: Option<String>,
    spec_path: Option<String>,
}

/// Hand-rolled argument parsing: one positional `<wad>`, plus `--map NAME`
/// and `--spec FILE`, in any order and either flag at most once meaningfully
/// (a repeat simply overwrites the earlier value). Any unknown flag, a flag
/// missing its value, an extra positional, or a missing `<wad>` is an error
/// naming the problem.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut wad_path = None;
    let mut map_name = None;
    let mut spec_path = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--map" => map_name = Some(args.next().ok_or("--map requires a value")?),
            "--spec" => spec_path = Some(args.next().ok_or("--spec requires a value")?),
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
            spec_path,
        })
}

fn real_main() -> i32 {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("crustygen-check: {msg}");
            eprintln!("{USAGE}");
            return 2;
        }
    };

    match check_wad(&args) {
        Ok(exit_code) => exit_code,
        Err(msg) => {
            eprintln!("crustygen-check: {msg}");
            2
        }
    }
}

/// Reads `args.wad_path`, selects a map, parses and checks it, and prints the
/// report. Returns the process exit code (0 clean, 1 an `Error` finding) or
/// `Err` naming the first failure encountered.
fn check_wad(args: &Args) -> Result<i32, String> {
    let bytes = std::fs::read(&args.wad_path)
        .map_err(|err| format!("failed to read `{}`: {err}", args.wad_path))?;
    let wad = Wad::from_bytes(bytes)
        .map_err(|err| format!("failed to parse `{}` as a WAD: {err}", args.wad_path))?;
    let group = select_group(&wad, args.map_name.as_deref(), &args.wad_path)?;
    let text = read_textmap(&wad, &group)?;
    let map = parse_udmf(text, Limits::default())
        .map_err(|err| format!("failed to parse TEXTMAP in map `{}`: {err}", group.name))?;
    let tables = Tables::load().map_err(|err| format!("failed to load tables: {err}"))?;
    let spec = load_spec(args.spec_path.as_deref(), &tables)?;

    let report = check::run(
        &map,
        &group.name,
        &tables,
        spec.as_ref().map(|doc| &doc.spec),
    );
    print_report(&report);

    let has_error = report
        .findings
        .iter()
        .any(|finding| finding.severity == Severity::Error);
    Ok(i32::from(has_error))
}

/// Picks `map_name` (exact match) or, when absent, the WAD's first map
/// group.
fn select_group(wad: &Wad, map_name: Option<&str>, wad_path: &str) -> Result<MapGroup, String> {
    let group = match map_name {
        Some(name) => wad.map_group(name),
        None => wad.map_groups().into_iter().next(),
    };
    group.ok_or_else(|| match map_name {
        Some(name) => format!("no map group named `{name}` in `{wad_path}`"),
        None => format!("`{wad_path}` contains no map groups"),
    })
}

/// Finds `group`'s `TEXTMAP` lump and reads it as UTF-8 text.
fn read_textmap<'wad>(wad: &'wad Wad, group: &MapGroup) -> Result<&'wad str, String> {
    let lumps = wad.lumps();
    let textmap_idx = group
        .data_indices
        .iter()
        .copied()
        .find(|&i| lumps[i].name() == "TEXTMAP")
        .ok_or_else(|| format!("map `{}` has no TEXTMAP lump (not a UDMF map)", group.name))?;
    std::str::from_utf8(wad.lump_data(&lumps[textmap_idx])).map_err(|err| {
        format!(
            "TEXTMAP lump in map `{}` is not valid UTF-8: {err}",
            group.name
        )
    })
}

/// Reads and parses `spec_path` when given; `Ok(None)` when no `--spec` was
/// passed.
fn load_spec(spec_path: Option<&str>, tables: &Tables) -> Result<Option<SpecDocument>, String> {
    let Some(path) = spec_path else {
        return Ok(None);
    };
    let text = std::fs::read_to_string(path)
        .map_err(|err| format!("failed to read spec `{path}`: {err}"))?;
    Spec::from_markdown(&text, tables)
        .map(Some)
        .map_err(|err| format!("failed to parse spec `{path}`: {err}"))
}

/// Prints every finding (one per line, via `Display`), then every
/// conformance row when a spec was supplied, then a one-line summary.
fn print_report(report: &CheckReport) {
    for finding in &report.findings {
        println!("{finding}");
    }

    if let Some(rows) = &report.conformance {
        for row in rows {
            let verdict = verdict_str(row.verdict);
            println!(
                "{}: {verdict} (target {}, actual {})",
                row.parameter, row.target, row.actual
            );
        }
    }

    let blocking = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .count();
    let warnings = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Warning)
        .count();
    let rows = report.conformance.as_ref().map_or(0, Vec::len);
    println!(
        "{blocking} blocking, {warnings} warning(s), {rows} conformance row(s), {} tag(s)",
        report.tag_manifest.len()
    );
}

/// Renders a [`Verdict`] as the lowercase, hyphenated word `conform::rows`'
/// callers expect in prose.
fn verdict_str(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::Info => "info",
        Verdict::NotDerivable => "not-derivable",
        Verdict::NotRun => "not-run",
    }
}
