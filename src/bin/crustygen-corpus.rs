//! `crustygen-corpus` — sweep a directory of zips/WADs into an
//! expressibility report (`docs/corpus.md`).
//!
//! Usage: `crustygen-corpus <dir> [--json FILE] [--report FILE]`.
//!
//! Surveys every `.zip`/`.wad` in `<dir>` (non-recursive) through the shared
//! ingestion path, deduplicates maps by content hash, classifies each against
//! crustygen's emittable vocabulary and the teleport recognizer, and renders
//! the aggregate as Markdown —
//! to stdout by default, to `--report FILE` when given. `--json FILE` writes
//! the full document, per-map rows included.
//!
//! Exit 0 when every candidate loaded, 1 when at least one archive, WAD, or
//! map failed to *load* (each named on stderr; survivors still counted), 2 on
//! a usage, I/O, or no-candidates failure.
//!
//! A WAD carrying no map group is ordinary corpus content — a resource WAD —
//! not a load failure. It is named on stderr and counted in the `no_maps`
//! bucket, but it does not affect the exit code; on a real idgames sample,
//! counting it would make a non-zero exit the norm.

use std::path::PathBuf;

use crustygen::lift::corpus::{self, Report};
use crustygen::lift::vocabulary::Vocabulary;
use crustygen::tables::Tables;

const USAGE: &str = "usage: crustygen-corpus <dir> [--json FILE] [--report FILE]";

fn main() {
    std::process::exit(real_main());
}

/// The parsed command line: the positional directory plus the output paths.
struct Args {
    dir: PathBuf,
    json: Option<PathBuf>,
    report: Option<PathBuf>,
}

/// Hand-rolled argument parsing, mirroring `crustygen-lift`'s: one positional
/// `<dir>` plus the value-taking `--json` and `--report`.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut dir = None;
    let mut json = None;
    let mut report = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = Some(PathBuf::from(args.next().ok_or("--json requires a value")?)),
            "--report" => {
                report = Some(PathBuf::from(
                    args.next().ok_or("--report requires a value")?,
                ));
            }
            flag if flag.starts_with("--") => return Err(format!("unknown flag `{flag}`")),
            positional if dir.is_none() => dir = Some(PathBuf::from(positional)),
            extra => return Err(format!("unexpected extra argument `{extra}`")),
        }
    }

    dir.ok_or_else(|| "missing <dir>".to_owned())
        .map(|dir| Args { dir, json, report })
}

fn real_main() -> i32 {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("crustygen-corpus: {msg}");
            eprintln!("{USAGE}");
            return 2;
        }
    };

    match run(&args) {
        Ok(code) => code,
        Err(msg) => {
            eprintln!("crustygen-corpus: {msg}");
            2
        }
    }
}

/// Sweeps the directory, writes the requested outputs, and returns the exit
/// code (0 all loaded, 1 some candidates failed to load). Every failure line
/// is echoed to stderr, map-free WADs included, but only the load-failure
/// buckets move the exit code.
fn run(args: &Args) -> Result<i32, String> {
    let tables = Tables::load().map_err(|e| format!("tables: {e}"))?;
    let vocab = Vocabulary::from_tables(&tables);
    let sweep = corpus::sweep_dir(&args.dir, &vocab, &tables).map_err(|e| e.to_string())?;
    for line in &sweep.failures {
        eprintln!("crustygen-corpus: {line}");
    }
    let provenance = corpus::read_provenance(&args.dir);
    let aggregate = corpus::aggregate(&sweep.maps);
    let markdown = corpus::render_markdown(provenance.as_ref(), &sweep.buckets, &aggregate);
    if let Some(path) = &args.json {
        let report = Report {
            provenance: provenance.as_ref(),
            buckets: &sweep.buckets,
            aggregate: &aggregate,
            maps: &sweep.maps,
        };
        let text = serde_json::to_string_pretty(&report)
            .map_err(|e| format!("serializing report: {e}"))?;
        std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    match &args.report {
        Some(path) => {
            std::fs::write(path, &markdown)
                .map_err(|e| format!("writing {}: {e}", path.display()))?;
        }
        // Without `--report`, the Markdown goes to stdout — unless `--json`
        // asked for the machine-readable form instead.
        None if args.json.is_none() => print!("{markdown}"),
        None => {}
    }
    Ok(i32::from(sweep.buckets.load_failures() > 0))
}
