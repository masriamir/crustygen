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
    write_atomically(std::path::Path::new(&args.out_path), &bytes)
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

/// Writes `bytes` to `target` through a sibling temp file renamed into
/// place, so a failure part-way leaves no partial output at `target`.
fn write_atomically(target: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = target.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", std::process::id()));
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, bytes)?;
    std::fs::rename(&temp, target).inspect_err(|_| {
        // Best effort: the rename error is the one worth reporting.
        std::fs::remove_file(&temp).ok();
    })
}

#[cfg(test)]
mod tests {
    use super::write_atomically;

    fn temp_dir(label: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("time moves forward")
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "crustygen-build-unit-{label}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir(&dir).expect("create temp dir");
        dir
    }

    fn entries(dir: &std::path::Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read dir")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_successful_write_leaves_only_the_target() {
        let dir = temp_dir("ok");
        let target = dir.join("out.wad");
        write_atomically(&target, b"PWAD").expect("writes");
        assert_eq!(std::fs::read(&target).expect("read back"), b"PWAD");
        assert_eq!(entries(&dir), vec!["out.wad".to_owned()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn an_existing_target_file_is_replaced() {
        // `std::fs::rename` replaces an existing destination file on every
        // supported platform (Windows included, via `MoveFileExW`); this
        // pins that a rebuild onto the same `<out.wad>` path succeeds.
        let dir = temp_dir("overwrite");
        let target = dir.join("out.wad");
        std::fs::write(&target, b"stale bytes from an earlier build").expect("seed");
        write_atomically(&target, b"PWAD").expect("replaces the existing file");
        assert_eq!(std::fs::read(&target).expect("read back"), b"PWAD");
        assert_eq!(entries(&dir), vec!["out.wad".to_owned()]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_failed_rename_leaves_no_temp_file_behind() {
        let dir = temp_dir("dirtarget");
        // A non-empty directory at the target path: the temp write beside it
        // succeeds, the rename onto it cannot.
        let target = dir.join("out.wad");
        std::fs::create_dir(&target).expect("create blocking dir");
        std::fs::write(target.join("occupant"), b"x").expect("occupy it");
        let err = write_atomically(&target, b"PWAD").expect_err("cannot replace a directory");
        assert!(!err.to_string().is_empty());
        assert_eq!(
            entries(&dir),
            vec!["out.wad".to_owned()],
            "temp file left behind"
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
