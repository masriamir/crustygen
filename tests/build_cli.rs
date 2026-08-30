//! CLI tests for `crustygen-build`: exit codes per pipeline stage, the
//! stdout summary, and byte-identity with the committed `maps/entrada.wad`,
//! `maps/salto.wad` and `maps/ascensor.wad`.

use std::path::PathBuf;
use std::process::{Command, Output};

use crustywad::Wad;

mod common;

use common::{ASCENSOR, ENTRADA, SALTO};

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crustygen-build"))
}

/// A uniquely named path in the temp dir (same shape as the sibling CLI
/// tests' helper); the file is not created.
fn temp_path(label: &str, ext: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time moves forward")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "crustygen-build-{label}-{}-{nanos}.{ext}",
        std::process::id()
    ))
}

/// Writes `ir` to a temp file, runs the binary on it with an output path,
/// removes both files, and returns the process output plus the WAD bytes
/// (when one was written).
fn build(ir: &str, label: &str, extra: &[&str]) -> (Output, Option<Vec<u8>>) {
    let ir_path = temp_path(label, "json");
    std::fs::write(&ir_path, ir).expect("write temp IR");
    let out_path = temp_path(label, "wad");
    let out = bin()
        .arg(&ir_path)
        .arg(&out_path)
        .args(extra)
        .output()
        .expect("runs");
    std::fs::remove_file(&ir_path).ok();
    let wad = std::fs::read(&out_path).ok();
    std::fs::remove_file(&out_path).ok();
    (out, wad)
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// Two rooms joined by a plain portal, with a tunable opening width and a
/// tunable x for room `b`'s left wall (256 = flush against `a`'s right wall,
/// which overlaps once `b` is pulled further left).
fn two_rooms(width: i32, b_left: i32) -> String {
    let b_right = b_left + 256;
    format!(
        r#"{{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
               "things":[{{ "kind":"player1_start", "at":[128,128], "angle":90 }}] }},
            {{ "id":"b", "footprint":[[{b_left},0],[{b_left},256],[{b_right},256],[{b_right},0]],
               "floor":0, "ceiling":128, "light":160,
               "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }}
          ],
          "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":{width}, "at":[256,128] }}] }}"#
    )
}

#[test]
fn no_arguments_exits_2_with_usage() {
    let out = bin().output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("usage:"), "stderr: {}", stderr(&out));
}

#[test]
fn a_missing_output_path_exits_2() {
    let out = bin().arg("only-one.json").output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("<out.wad>"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn an_unknown_flag_exits_2() {
    let out = bin()
        .args(["x.json", "y.wad", "--nope"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("--nope"), "stderr: {}", stderr(&out));
}

#[test]
fn an_extra_positional_argument_exits_2() {
    let out = bin().args(["x.json", "y.wad", "z"]).output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("unexpected extra argument `z`"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn a_missing_input_file_exits_2() {
    let out = bin()
        .args(["no-such.json", "y.wad"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("no-such.json"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn the_entrada_fixture_builds_byte_identical_to_the_committed_wad() {
    let (out, wad) = build(ENTRADA, "golden", &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let committed =
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("maps/entrada.wad"))
            .expect("read maps/entrada.wad");
    assert_eq!(wad.expect("a WAD was written"), committed);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("MAP01: 8 rooms, 7 portals"),
        "got: {stdout}"
    );
    assert!(stdout.contains("18 sectors"), "got: {stdout}");
    assert!(out.stderr.is_empty(), "stderr: {}", stderr(&out));
}

/// Salto, the teleport playtest map: the same drift guard entrada carries,
/// over the fixture that exercises every teleport shape the compiler emits.
/// 14 sectors = 5 rooms + 1 door + 2 door alcoves + 1 passage + 1
/// walkover-exit alcove + 4 teleport pads.
#[test]
fn the_salto_fixture_builds_byte_identical_to_the_committed_wad() {
    let (out, wad) = build(SALTO, "salto", &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let committed =
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("maps/salto.wad"))
            .expect("read maps/salto.wad");
    assert_eq!(wad.expect("a WAD was written"), committed);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("MAP01: 5 rooms, 2 portals"),
        "got: {stdout}"
    );
    assert!(stdout.contains("14 sectors"), "got: {stdout}");
    assert!(out.stderr.is_empty(), "stderr: {}", stderr(&out));
}

/// Ascensor, the lift playtest map: the same drift guard entrada and salto
/// carry, over the fixture that exercises every platform shape the compiler
/// emits. 13 sectors = 6 rooms + 3 lift platforms + 1 barrier + 1 plain
/// passage + 1 lift alcove + 1 pedestal.
#[test]
fn the_ascensor_fixture_builds_byte_identical_to_the_committed_wad() {
    let (out, wad) = build(ASCENSOR, "ascensor", &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let committed =
        std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("maps/ascensor.wad"))
            .expect("read maps/ascensor.wad");
    assert_eq!(wad.expect("a WAD was written"), committed);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("MAP01: 6 rooms, 5 portals"),
        "got: {stdout}"
    );
    assert!(stdout.contains("13 sectors"), "got: {stdout}");
    assert!(out.stderr.is_empty(), "stderr: {}", stderr(&out));
}

#[test]
fn map_selects_the_map_group_name() {
    let (out, wad) = build(ENTRADA, "mapname", &["--map", "E1M1"]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let wad = Wad::from_bytes(wad.expect("a WAD was written")).expect("parses");
    assert!(wad.map_group("E1M1").is_some());
    assert!(wad.map_group("MAP01").is_none());
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("E1M1: "));
}

#[test]
fn invalid_json_exits_1_as_an_ir_rejection() {
    let (out, wad) = build("{ not json", "badjson", &[]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(wad.is_none(), "no WAD may be written on rejection");
    assert!(
        stderr(&out).contains("crustygen-build: ir: "),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn an_off_grid_coordinate_exits_1_naming_the_room() {
    let ir = two_rooms(128, 320).replace("[256,256]", "[256,250]");
    let (out, wad) = build(&ir, "offgrid", &[]);
    assert_eq!(out.status.code(), Some(1), "stderr: {}", stderr(&out));
    assert!(wad.is_none());
    let err = stderr(&out);
    assert!(
        err.contains("ir: ") && err.contains("off-grid") && err.contains("`a`"),
        "stderr: {err}"
    );
}

#[test]
fn overlapping_rooms_exit_3_as_a_structural_refusal() {
    let (out, wad) = build(&two_rooms(128, 128), "overlap", &[]);
    assert_eq!(out.status.code(), Some(3), "stderr: {}", stderr(&out));
    assert!(wad.is_none());
    let err = stderr(&out);
    assert!(
        err.contains("compile: ") && err.contains("overlap"),
        "stderr: {err}"
    );
}

#[test]
fn a_too_narrow_portal_exits_4_naming_the_rule() {
    let (out, wad) = build(&two_rooms(16, 320), "narrow", &[]);
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
    assert!(wad.is_none());
    let err = stderr(&out);
    assert!(err.contains("playability: P3 (a <-> b)"), "stderr: {err}");
}

#[test]
fn a_clean_two_room_map_builds() {
    let (out, wad) = build(&two_rooms(128, 320), "clean", &[]);
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let wad = Wad::from_bytes(wad.expect("a WAD was written")).expect("parses");
    assert!(wad.map_group("MAP01").is_some());
}

#[test]
fn an_unwritable_output_path_exits_2() {
    let ir_path = temp_path("unwritable", "json");
    std::fs::write(&ir_path, ENTRADA).expect("write temp IR");
    let out_path = temp_path("unwritable-dir", "d").join("out.wad");
    let out = bin().arg(&ir_path).arg(&out_path).output().expect("runs");
    std::fs::remove_file(&ir_path).ok();
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("out.wad"), "stderr: {}", stderr(&out));
}

#[test]
fn map_without_a_value_exits_2() {
    let out = bin()
        .args(["x.json", "y.wad", "--map"])
        .output()
        .expect("runs");
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("--map requires a value"),
        "stderr: {}",
        stderr(&out)
    );
}

#[test]
fn an_output_path_that_is_a_directory_exits_2_leaving_nothing_behind() {
    let ir_path = temp_path("dirout", "json");
    std::fs::write(&ir_path, ENTRADA).expect("write temp IR");
    let parent = temp_path("dirout-parent", "d");
    let out_dir = parent.join("out.wad");
    std::fs::create_dir_all(&out_dir).expect("create blocking dir");
    std::fs::write(out_dir.join("occupant"), b"x").expect("occupy it");
    let out = bin().arg(&ir_path).arg(&out_dir).output().expect("runs");
    std::fs::remove_file(&ir_path).ok();
    let mut leftovers: Vec<String> = std::fs::read_dir(&parent)
        .expect("read parent")
        .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
        .collect();
    leftovers.sort();
    std::fs::remove_dir_all(&parent).ok();
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("out.wad"), "stderr: {}", stderr(&out));
    assert_eq!(
        leftovers,
        vec!["out.wad".to_owned()],
        "temp file left behind"
    );
}
