//! `crustygen-build` — compile an IR JSON document into a PWAD (the build
//! stage of the pipeline; `docs/build.md`).
//!
//! Usage: `crustygen-build <ir.json> <out.wad> [--map NAME]`.
//!
//! Reads `<ir.json>`, validates it through [`crustygen::ir::Ir::from_json`],
//! compiles it with [`crustygen::compile::compile`] — the refusing variant,
//! so a map that breaks a playability rule is never written; the reporting
//! posture belongs to `crustygen-check` — packs the result with real nodes
//! via [`crustygen::pack::pack_udmf_with_nodes`] under the map name `NAME`
//! (default `MAP01`), and writes `<out.wad>`. One summary line prints to
//! stdout on success.
//!
//! Exit codes follow the pipeline stages, one code per stage, so a caller
//! running the loop gets its funnel from the status alone: 0 built; 1 the IR
//! was rejected (invalid JSON or a validation failure — `ir:` on stderr); 2 a
//! usage, I/O, tables, or pack failure; 3 a structural compile refusal
//! (`compile:`); 4 playability violations (`playability:`, one line per
//! rule). Every failure names what failed on stderr.

use crustygen::compile::{self, CompileError};
use crustygen::ir::Ir;
use crustygen::pack;
use crustygen::tables::Tables;

const USAGE: &str = "usage: crustygen-build <ir.json> <out.wad> [--map NAME]";
const DEFAULT_MAP: &str = "MAP01";

/// Exit code for an IR the validator rejected.
const EXIT_IR: i32 = 1;
/// Exit code for a usage, I/O, tables, or pack failure.
const EXIT_USAGE: i32 = 2;
/// Exit code for a structural compile refusal.
const EXIT_COMPILE: i32 = 3;
/// Exit code for a map that compiled but breaks a playability rule.
const EXIT_PLAYABILITY: i32 = 4;

fn main() {
    std::process::exit(real_main());
}

/// The parsed command line: the two positionals plus the optional map name.
struct Args {
    ir_path: String,
    out_path: String,
    map_name: String,
}

/// Hand-rolled argument parsing, mirroring the sibling binaries: two
/// positionals `<ir.json> <out.wad>` and `--map NAME` in any order (a repeat
/// overwrites the earlier value). Any unknown flag, a flag missing its
/// value, an extra positional, or a missing positional is an error naming
/// the problem.
fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut ir_path = None;
    let mut out_path = None;
    let mut map_name = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--map" => map_name = Some(args.next().ok_or("--map requires a value")?),
            flag if flag.starts_with("--") => return Err(format!("unknown flag `{flag}`")),
            positional if ir_path.is_none() => ir_path = Some(positional.to_owned()),
            positional if out_path.is_none() => out_path = Some(positional.to_owned()),
            extra => return Err(format!("unexpected extra argument `{extra}`")),
        }
    }

    let ir_path = ir_path.ok_or("missing <ir.json> path")?;
    let out_path = out_path.ok_or("missing <out.wad> path")?;
    Ok(Args {
        ir_path,
        out_path,
        map_name: map_name.unwrap_or_else(|| DEFAULT_MAP.to_owned()),
    })
}

fn real_main() -> i32 {
    let args = match parse_args(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(msg) => {
            eprintln!("crustygen-build: {msg}");
            eprintln!("{USAGE}");
            return EXIT_USAGE;
        }
    };

    match build(&args) {
        Ok(()) => 0,
        Err(failure) => {
            for line in failure.lines() {
                eprintln!("crustygen-build: {line}");
            }
            failure.exit_code()
        }
    }
}

/// Why a build stopped, by pipeline stage — the stage picks the exit code
/// and the stderr prefix.
enum Failure {
    /// Usage, I/O, tables, or pack: nothing about the map itself.
    Usage(String),
    /// [`Ir::from_json`] rejected the document.
    Ir(String),
    /// The compiler refused the geometry.
    Compile(String),
    /// The map compiled but breaks playability rules, one string per rule.
    Playability(Vec<String>),
}

impl Failure {
    fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => EXIT_USAGE,
            Self::Ir(_) => EXIT_IR,
            Self::Compile(_) => EXIT_COMPILE,
            Self::Playability(_) => EXIT_PLAYABILITY,
        }
    }

    /// The stderr lines, each already carrying its stage prefix.
    fn lines(&self) -> Vec<String> {
        match self {
            Self::Usage(msg) => vec![msg.clone()],
            Self::Ir(msg) => vec![format!("ir: {msg}")],
            Self::Compile(msg) => vec![format!("compile: {msg}")],
            Self::Playability(rules) => rules.iter().map(|r| format!("playability: {r}")).collect(),
        }
    }
}

/// Reads, validates, compiles, packs, and writes. Returns the first failure
/// by stage.
fn build(args: &Args) -> Result<(), Failure> {
    let text = std::fs::read_to_string(&args.ir_path)
        .map_err(|err| Failure::Usage(format!("failed to read `{}`: {err}", args.ir_path)))?;
    let ir = Ir::from_json(&text).map_err(|err| Failure::Ir(err.to_string()))?;
    let tables =
        Tables::load().map_err(|err| Failure::Usage(format!("failed to load tables: {err}")))?;
    let compiled = compile::compile(&ir, &tables).map_err(|err| match err {
        CompileError::Playability { violations } => {
            Failure::Playability(violations.iter().map(ToString::to_string).collect())
        }
        other => Failure::Compile(other.to_string()),
    })?;
    let bytes = pack::pack_udmf_with_nodes(&compiled, &args.map_name)
        .map_err(|err| Failure::Usage(format!("failed to pack `{}`: {err}", args.map_name)))?;
    std::fs::write(&args.out_path, bytes)
        .map_err(|err| Failure::Usage(format!("failed to write `{}`: {err}", args.out_path)))?;
    println!(
        "{}: {} rooms, {} portals → {} sectors, {} linedefs, {} things → {}",
        args.map_name,
        ir.rooms.len(),
        ir.portals.len(),
        compiled.data.sectors.len(),
        compiled.data.linedefs.len(),
        compiled.things.len(),
        args.out_path
    );
    Ok(())
}
