//! CLI smoke tests for `crustygen-check`: exit codes and output shape.

use std::process::Command;

use crustygen::compile::compile;
use crustygen::compile::textmap::emit_textmap;
use crustygen::ir::Ir;
use crustygen::pack::pack_udmf;
use crustygen::tables::Tables;

const ENTRADA: &str = include_str!("fixtures/entrada_base.json");

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crustygen-check"))
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

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time moves forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "crustygen-check-broken-{}-{nanos}.wad",
        std::process::id()
    ));
    std::fs::write(&path, &bytes).expect("write temp wad");

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
