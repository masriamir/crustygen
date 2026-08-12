//! CLI smoke tests for `crustygen-check`: exit codes and output shape.

use std::path::PathBuf;
use std::process::Command;

use crustygen::compile::compile;
use crustygen::compile::textmap::emit_textmap;
use crustygen::ir::Ir;
use crustygen::pack::pack_udmf;
use crustygen::tables::Tables;
use crustywad::{WadBuilder, WadKind};

mod common;

const ENTRADA: &str = include_str!("fixtures/entrada_base.json");
const ENTRADA_SPEC_PATH: &str = "tests/fixtures/entrada.spec.md";

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crustygen-check"))
}

/// Writes `bytes` to a uniquely named file under `std::env::temp_dir()`
/// (`label` distinguishes call sites in the filename), for a test to point
/// the CLI at and remove afterward.
fn write_temp(bytes: &[u8], label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time moves forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "crustygen-check-{label}-{}-{nanos}.wad",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write temp file");
    path
}

#[test]
fn a_missing_file_exits_2() {
    let out = bin().arg("no-such.wad").output().expect("runs");
    assert_eq!(
        out.status.code(),
        Some(2),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn the_shipped_entrada_wad_exits_0() {
    let out = bin().arg("maps/entrada.wad").output().expect("runs");
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Cheap output sanity: a clean run's own findings and summary must never
    // mention "error" — this only holds because the CLI's summary line is
    // deliberately worded without that substring (see `print_report`).
    let stdout = String::from_utf8_lossy(&out.stdout).to_lowercase();
    assert!(
        !stdout.contains("error"),
        "clean run unexpectedly mentions \"error\": {stdout}"
    );
}

/// Compiles entrada, blanks one lower texture the compiler wrote on the
/// emitted geometry, re-renders `TEXTMAP` from the mutated `MapData`, and
/// packs the result into a real (un-noded) PWAD via [`pack_udmf`] — the
/// reuse boundary binds `src/check/`, not tests, so building the broken
/// fixture through `compile`/`pack` here is fine.
///
/// Any non-empty `SidedefOut::lower` was written by
/// `heights::apply_height_textures` specifically because a genuine floor
/// difference needs it (`compile/heights.rs`); blanking it leaves that
/// difference with no lower texture on the lower-floor side, which is
/// exactly rule V-P8's re-derived condition (`check/invariants.rs`,
/// `check_textures`). Written to a uniquely named file under
/// `std::env::temp_dir()` and removed after the run.
#[test]
fn a_broken_wad_exits_1_and_names_the_finding() {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(ENTRADA).expect("ir parses");
    let mut compiled = compile(&ir, &tables).expect("entrada compiles");

    let broken = compiled
        .data
        .sidedefs
        .iter_mut()
        .find(|sd| !sd.lower.is_empty())
        .expect("entrada has at least one lower texture to blank");
    broken.lower.clear();
    compiled.textmap = emit_textmap(&compiled.data, &compiled.things);

    let bytes = pack_udmf(&compiled, "MAP01").expect("packs into a PWAD");
    let path = write_temp(&bytes, "broken");

    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(1),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("V-P8"),
        "expected a V-P8 finding in stdout: {stdout}"
    );
}

#[test]
fn an_unknown_flag_exits_2_with_usage_on_stderr() {
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--bogus")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unknown flag `--bogus`"), "got: {stderr}");
    assert!(stderr.contains("usage:"), "got: {stderr}");
}

#[test]
fn a_map_flag_missing_its_value_exits_2() {
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--map")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--map requires a value"), "got: {stderr}");
}

#[test]
fn an_extra_positional_argument_exits_2() {
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("extra")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unexpected extra argument `extra`"),
        "got: {stderr}"
    );
}

#[test]
fn a_map_flag_naming_an_absent_group_exits_2() {
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--map")
        .arg("NOSUCHMAP")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no map group named `NOSUCHMAP`"),
        "got: {stderr}"
    );
}

#[test]
fn a_wad_with_no_map_groups_exits_2() {
    let bytes = WadBuilder::new(WadKind::Pwad)
        .add_lump("DUMMY", b"not a map".to_vec())
        .build()
        .expect("builds");
    let path = write_temp(&bytes, "no-map-groups");
    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("contains no map groups"), "got: {stderr}");
}

#[test]
fn a_textmap_lump_with_invalid_utf8_exits_2() {
    let bytes = common::wad_with_textmap(vec![0xFF, 0xFE, 0x00]);
    let path = write_temp(&bytes, "invalid-utf8");
    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("is not valid UTF-8"), "got: {stderr}");
}

#[test]
fn a_spec_flag_naming_an_unreadable_file_exits_2() {
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--spec")
        .arg("no-such-spec.md")
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("failed to read spec `no-such-spec.md`"),
        "got: {stderr}"
    );
}

#[test]
fn a_spec_flag_naming_unparseable_content_exits_2() {
    let path = write_temp(b"not a valid spec document", "bad-spec");
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--spec")
        .arg(&path)
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("failed to parse spec"), "got: {stderr}");
}

#[test]
fn a_spec_run_prints_conformance_rows_and_their_summary_count() {
    // `entrada.spec.md` is hand-paired to `maps/entrada.wad`'s own compiled
    // actuals (see `tests/check_conformance.rs`), so this run's rows span
    // Pass, Info, and NotDerivable verdicts — exercising `print_report`'s
    // conformance-row loop and most of `verdict_str`'s match arms end to
    // end through the actual binary, not just the library's own unit tests.
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--spec")
        .arg(ENTRADA_SPEC_PATH)
        .output()
        .expect("runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout: {stdout}\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        stdout.contains("identity.slot: pass"),
        "expected a pass row: {stdout}"
    );
    assert!(
        stdout.contains("not-derivable"),
        "expected a not-derivable row: {stdout}"
    );
    assert!(
        stdout.contains("combat.hitscanner_ratio: info"),
        "expected an info row: {stdout}"
    );
    assert!(
        stdout
            .lines()
            .next_back()
            .is_some_and(|line| line.contains("conformance row(s)")),
        "expected the summary line to name the conformance row count: {stdout}"
    );
}

#[test]
fn a_spec_mismatch_prints_a_fail_row() {
    let spec_text = std::fs::read_to_string(ENTRADA_SPEC_PATH).expect("reads");
    let wrong = spec_text.replacen(
        "count: 1                   # per-secret detail lives in the prose body",
        "count: 2                   # per-secret detail lives in the prose body",
        1,
    );
    assert_ne!(wrong, spec_text, "the patch changed nothing");
    let path = write_temp(wrong.as_bytes(), "wrong-secret-count-spec");
    let out = bin()
        .arg("maps/entrada.wad")
        .arg("--spec")
        .arg(&path)
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("secrets.count: fail"),
        "expected a fail row: {stdout}"
    );
}

#[test]
fn a_structurally_broken_map_prints_not_run_conformance_rows() {
    // A dangling sidedef->sector cross-reference (the same shape
    // `tests/check_conformance.rs`'s own
    // `a_structurally_broken_scene_marks_every_conformance_row_not_run`
    // pins at the library level) trips `check::run`'s failure containment,
    // forcing every conformance row to `Verdict::NotRun` — exercising
    // `verdict_str`'s `NotRun` arm through the actual binary.
    const DANGLING_SIDEDEF_TEXTMAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 0.000; y = 128.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 9; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; single = true; }
"#;
    let bytes = common::wad_with_textmap(DANGLING_SIDEDEF_TEXTMAP);
    let path = write_temp(&bytes, "structurally-broken");
    let out = bin()
        .arg(&path)
        .arg("--spec")
        .arg(ENTRADA_SPEC_PATH)
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(": not-run "),
        "expected at least one not-run conformance row: {stdout}"
    );
}

#[test]
fn a_binary_format_entrada_checks_with_an_origin_label() {
    let path = write_temp(&common::binary_entrada_wad(), "binary-entrada");
    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout
            .trim_end()
            .ends_with(", assembled from binary format"),
        "summary lacks the origin label: {stdout}"
    );
}

#[test]
fn binary_and_udmf_runs_agree_on_findings() {
    let binary = write_temp(&common::binary_entrada_wad(), "parity-binary");
    let udmf = write_temp(&common::udmf_entrada_wad(), "parity-udmf");
    let bin_out = bin().arg(&binary).output().expect("runs");
    let udmf_out = bin().arg(&udmf).output().expect("runs");
    std::fs::remove_file(&binary).ok();
    std::fs::remove_file(&udmf).ok();

    assert_eq!(bin_out.status.code(), udmf_out.status.code());
    let bin_lines: Vec<&str> = std::str::from_utf8(&bin_out.stdout)
        .expect("utf8")
        .lines()
        .collect();
    let udmf_lines: Vec<&str> = std::str::from_utf8(&udmf_out.stdout)
        .expect("utf8")
        .lines()
        .collect();
    // Every finding line agrees; only the trailing summary differs, and
    // only by the origin suffix.
    let (bin_summary, bin_findings) = bin_lines.split_last().expect("summary");
    let (udmf_summary, udmf_findings) = udmf_lines.split_last().expect("summary");
    assert_eq!(bin_findings, udmf_findings);
    assert_eq!(
        bin_summary.strip_suffix(", assembled from binary format"),
        Some(*udmf_summary)
    );
}
