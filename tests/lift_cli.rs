//! CLI smoke tests for `crustygen-lift`: exit codes, human output, JSON.

mod common;

use std::path::PathBuf;
use std::process::Command;

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crustygen-lift"))
}

/// Writes `bytes` to a uniquely named temp file (same shape as
/// `check_cli.rs`'s helper).
fn write_temp(bytes: &[u8], label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time moves forward")
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "crustygen-lift-{label}-{}-{nanos}.wad",
        std::process::id()
    ));
    std::fs::write(&path, bytes).expect("write temp file");
    path
}

#[test]
fn a_missing_file_exits_2() {
    let out = bin().arg("no-such.wad").output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn an_unknown_flag_exits_2() {
    let out = bin().args(["x.wad", "--nope"]).output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn a_binary_wad_surveys_with_an_origin_note() {
    let path = write_temp(&common::binary_entrada_wad(), "survey");
    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("MAP01: "), "got: {stdout}");
    assert!(stdout.contains(" vertices, "), "got: {stdout}");
    assert!(
        stdout.contains("(assembled from binary format)"),
        "got: {stdout}"
    );
}

#[test]
fn json_output_matches_the_loaded_map() {
    let bytes = common::binary_entrada_wad();
    let path = write_temp(&bytes, "json");
    let out = bin()
        .args([path.to_str().expect("utf8 path"), "--json"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(0));

    // Independent ground truth: load the same map through the library.
    let wad = crustywad::Wad::from_bytes(bytes).expect("parses");
    let group = wad.map_groups().into_iter().next().expect("group");
    let loaded = crustygen::ingest::load_map(&wad, &group).expect("loads");

    let json: serde_json::Value = serde_json::from_slice(&out.stdout).expect("stdout is JSON");
    let record = &json.as_array().expect("array")[0];
    assert_eq!(record["map"], "MAP01");
    assert_eq!(record["census"]["sectors"], loaded.map.sectors.len());
    assert_eq!(record["census"]["things"], loaded.map.things.len());
}

#[test]
fn a_partly_broken_wad_surveys_survivors_and_exits_1() {
    let path = write_temp(
        &common::binary_entrada_wad_with_broken_second_map(),
        "partial",
    );
    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stdout.starts_with("MAP01: "), "survivor missing: {stdout}");
    assert!(stderr.contains("MAP02"), "failure not named: {stderr}");
}

#[test]
fn an_extra_positional_argument_exits_2() {
    let out = bin().args(["a.wad", "b.wad"]).output().expect("runs");
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unexpected extra argument"),
        "got: {stderr}"
    );
}

#[test]
fn map_selection_surveys_only_the_named_group() {
    let path = write_temp(&common::binary_entrada_wad(), "map-select");
    let out = bin()
        .args([path.to_str().expect("utf8 path"), "--map", "MAP01"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("MAP01: "));
}

#[test]
fn a_nonexistent_map_name_exits_2() {
    let path = write_temp(&common::binary_entrada_wad(), "map-missing");
    let out = bin()
        .args([path.to_str().expect("utf8 path"), "--map", "NOPE"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("no map group named `NOPE`"),
        "got: {stderr}"
    );
}

#[test]
fn a_wad_with_no_map_groups_exits_2() {
    let bytes = crustywad::WadBuilder::new(crustywad::WadKind::Pwad)
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
fn a_udmf_wad_surveys_without_the_binary_origin_note() {
    let path = write_temp(&common::udmf_entrada_wad(), "udmf-survey");
    let out = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(
        out.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("MAP01: "), "got: {stdout}");
    assert!(
        !stdout.contains("assembled from binary format"),
        "UDMF survey must not carry the binary-origin note: {stdout}"
    );
}

#[test]
fn vocabulary_flag_appends_a_verdict_to_human_and_json_output() {
    let path = write_temp(&common::binary_entrada_wad(), "vocab");
    let human = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    let json = bin()
        .args([path.to_str().unwrap(), "--vocabulary", "--json"])
        .output()
        .expect("runs");
    let plain = bin().arg(&path).output().expect("runs");
    std::fs::remove_file(&path).ok();
    assert_eq!(human.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(
        stdout.contains("; expressible: yes"),
        "entrada uses only emittable vocabulary: {stdout}"
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value[0]["verdict"]["expressible"], true);
    assert_eq!(value[0]["verdict"]["vanilla_only"], true);
    assert!(
        !String::from_utf8_lossy(&plain.stdout).contains("expressible"),
        "no flag, no verdict"
    );
}

#[test]
fn vocabulary_flag_names_unknown_values() {
    let textmap = r#"namespace = "doom";
vertex { x = 0; y = 0; } vertex { x = 128; y = 0; } vertex { x = 128; y = 128; } vertex { x = 0; y = 128; }
linedef { v1 = 0; v2 = 1; sidefront = 0; special = 21; }
linedef { v1 = 1; v2 = 2; sidefront = 1; }
linedef { v1 = 2; v2 = 3; sidefront = 2; }
linedef { v1 = 3; v2 = 0; sidefront = 3; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; }
thing { x = 64; y = 64; type = 1; } thing { x = 80; y = 64; type = 9999; }
"#;
    let path = write_temp(&common::wad_with_textmap(textmap), "vocab-unknown");
    let out = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("expressible: no"), "{stdout}");
    assert!(stdout.contains("line specials unknown: 21"), "{stdout}");
    // 46 (the tall red torch) used to stand in for an unknown thing type
    // here; it is a real `[things]` row now, so this asserts on 9999 —
    // a doomednum no vanilla mobjinfo entry defines, which keeps the test
    // about the unknown-value path rather than about a vocabulary gap.
    assert!(stdout.contains("thing types unknown: 9999"), "{stdout}");
}

/// A linedef special outside the pinned engine's vanilla list flips
/// `vanilla_only` off, which the human line reports as an `(outside
/// vanilla)` note alongside the unknown-value breakdown.
#[test]
fn a_non_vanilla_line_special_is_noted_as_outside_vanilla() {
    let textmap = r#"namespace = "doom";
vertex { x = 0; y = 0; } vertex { x = 128; y = 0; } vertex { x = 128; y = 128; } vertex { x = 0; y = 128; }
linedef { v1 = 0; v2 = 1; sidefront = 0; special = 8192; }
linedef { v1 = 1; v2 = 2; sidefront = 1; }
linedef { v1 = 2; v2 = 3; sidefront = 2; }
linedef { v1 = 3; v2 = 0; sidefront = 3; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; }
thing { x = 64; y = 64; type = 1; }
"#;
    let path = write_temp(&common::wad_with_textmap(textmap), "vocab-nonvanilla");
    let out = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("line specials unknown: 8192"), "{stdout}");
    assert!(stdout.contains("(outside vanilla)"), "{stdout}");
}

/// A teleport line that can never fire refuses the map on the fourth axis:
/// membership alone would pass (97 is emittable, the player start is in
/// vocabulary), but the recognizer's refusal flips `expressible` off. The
/// trigger here is two-sided and tagged 9, and no sector carries tag 9 —
/// `EV_Teleport` would find no destination, so `Refusal::Broken`.
#[test]
fn a_broken_teleport_line_refuses_the_map_on_the_teleport_axis() {
    let textmap = r#"namespace = "doom";
vertex { x = 0; y = 0; } vertex { x = 256; y = 0; } vertex { x = 256; y = 256; } vertex { x = 0; y = 256; }
vertex { x = 64; y = 64; } vertex { x = 192; y = 64; } vertex { x = 192; y = 192; } vertex { x = 64; y = 192; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
linedef { v1 = 4; v2 = 5; sidefront = 4; sideback = 5; twosided = true; special = 97; arg0 = 9; }
linedef { v1 = 5; v2 = 6; sidefront = 6; sideback = 7; twosided = true; }
linedef { v1 = 6; v2 = 7; sidefront = 8; sideback = 9; twosided = true; }
linedef { v1 = 7; v2 = 4; sidefront = 10; sideback = 11; twosided = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; } sidedef { sector = 1; }
sidedef { sector = 0; } sidedef { sector = 1; }
sidedef { sector = 0; } sidedef { sector = 1; }
sidedef { sector = 0; } sidedef { sector = 1; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; }
thing { x = 32; y = 32; type = 1; }
"#;
    let path = write_temp(&common::wad_with_textmap(textmap), "teleport-broken");
    let human = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    let json = bin()
        .args([path.to_str().unwrap(), "--vocabulary", "--json"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("; expressible: no"), "{stdout}");
    assert!(stdout.contains("(teleports refused: 1)"), "{stdout}");
    assert!(
        stdout.contains("(line specials ok)"),
        "membership is untouched: {stdout}"
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value[0]["verdict"]["teleports_ok"], false);
    assert_eq!(value[0]["verdict"]["line_specials_ok"], true);
    assert_eq!(value[0]["teleports"]["lines"], 1);
    assert_eq!(value[0]["teleports"]["broken"], 1);
}

/// A map whose every platform the recognizer accepts passes the fifth axis:
/// the JSON verdict says so, the `lifts` object carries the shape census,
/// and the human line stays silent about lifts.
#[test]
fn a_recognized_lift_map_passes_the_lift_axis() {
    let path = write_temp(&common::udmf_lifts_wad(), "lifts-ok");
    let human = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    let json = bin()
        .args([path.to_str().unwrap(), "--vocabulary", "--json"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("; expressible: yes"), "{stdout}");
    assert!(!stdout.contains("lifts refused"), "{stdout}");
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value[0]["verdict"]["lifts_ok"], true);
    assert_eq!(value[0]["lifts"]["plats"], 4);
    assert_eq!(value[0]["lifts"]["lifts"], 2);
    assert_eq!(value[0]["lifts"]["pedestals"], 1);
    assert_eq!(value[0]["lifts"]["barriers"], 1);
}

/// A lift line whose platform cannot move refuses the map on the fifth
/// axis: membership alone would pass (62 is emittable), but the recognizer's
/// `Refusal::Dead` flips `expressible` off. Three rooms all at floor 0, the
/// middle one tagged 7 and named by an SR lift line — `EV_DoPlat` would send
/// it to its own floor, so there is no movement to state.
const DEAD_LIFT_TEXTMAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 0.000; y = 128.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 128.000; }
vertex { x = 384.000; y = 0.000; }
vertex { x = 384.000; y = 128.000; }
linedef { v1 = 3; v2 = 2; sidefront = 0; sideback = 1; twosided = true; special = 62; arg0 = 7; }
linedef { v1 = 5; v2 = 4; sidefront = 2; sideback = 3; twosided = true; }
linedef { v1 = 0; v2 = 1; sidefront = 4; blocking = true; }
linedef { v1 = 1; v2 = 3; sidefront = 5; blocking = true; }
linedef { v1 = 2; v2 = 0; sidefront = 6; blocking = true; }
linedef { v1 = 3; v2 = 5; sidefront = 7; blocking = true; }
linedef { v1 = 4; v2 = 2; sidefront = 8; blocking = true; }
linedef { v1 = 5; v2 = 7; sidefront = 9; blocking = true; }
linedef { v1 = 7; v2 = 6; sidefront = 10; blocking = true; }
linedef { v1 = 6; v2 = 4; sidefront = 11; blocking = true; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
thing { x = 64.0; y = 64.0; angle = 90; type = 1; single = true; }
"#;

#[test]
fn a_dead_lift_refuses_the_map_on_the_lift_axis() {
    let path = write_temp(&common::wad_with_textmap(DEAD_LIFT_TEXTMAP), "lift-dead");
    let human = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    let json = bin()
        .args([path.to_str().unwrap(), "--vocabulary", "--json"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("; expressible: no"), "{stdout}");
    assert!(stdout.contains("(lifts refused: 1)"), "{stdout}");
    assert!(
        stdout.contains("(line specials ok)"),
        "membership is untouched: {stdout}"
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value[0]["verdict"]["lifts_ok"], false);
    assert_eq!(value[0]["verdict"]["line_specials_ok"], true);
    assert_eq!(value[0]["lifts"]["plats"], 1);
    assert_eq!(value[0]["lifts"]["dead"], 1);
}

/// A map whose one floor target the recognizer accepts passes the sixth
/// axis: three rooms in a row, the middle one (sector 1) tagged 7 and
/// standing 128 above its two neighbors, with a `23` (S1
/// `lowerFloorToLowest`) on the A|T link. Pressing it drops the wall flush
/// and joins A to B — the corpus's drop wall.
const FLOOR_TEXTMAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 0.000; y = 128.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 128.000; }
vertex { x = 384.000; y = 0.000; }
vertex { x = 384.000; y = 128.000; }
linedef { v1 = 3; v2 = 2; sidefront = 0; sideback = 1; twosided = true; special = 23; arg0 = 7; }
linedef { v1 = 5; v2 = 4; sidefront = 2; sideback = 3; twosided = true; }
linedef { v1 = 0; v2 = 1; sidefront = 4; blocking = true; }
linedef { v1 = 1; v2 = 3; sidefront = 5; blocking = true; }
linedef { v1 = 2; v2 = 0; sidefront = 6; blocking = true; }
linedef { v1 = 3; v2 = 5; sidefront = 7; blocking = true; }
linedef { v1 = 4; v2 = 2; sidefront = 8; blocking = true; }
linedef { v1 = 5; v2 = 7; sidefront = 9; blocking = true; }
linedef { v1 = 7; v2 = 6; sidefront = 10; blocking = true; }
linedef { v1 = 6; v2 = 4; sidefront = 11; blocking = true; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 128; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
thing { x = 64.0; y = 64.0; angle = 90; type = 1; single = true; }
"#;

#[test]
fn a_recognized_drop_wall_passes_the_floor_axis() {
    let path = write_temp(&common::wad_with_textmap(FLOOR_TEXTMAP), "floors-ok");
    let human = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    let json = bin()
        .args([path.to_str().unwrap(), "--vocabulary", "--json"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("; expressible: yes"), "{stdout}");
    assert!(!stdout.contains("floors refused"), "{stdout}");
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value[0]["verdict"]["floors_ok"], true);
    assert_eq!(value[0]["verdict"]["expressible"], true);
    assert_eq!(value[0]["floors"]["targets"], 1);
    assert_eq!(value[0]["floors"]["drop_walls"], 1);
}

/// A pillar that rises to seal itself refuses the map on the sixth axis, and
/// on the first as well: `101` (S1 `raiseFloor`) sends a level middle room to
/// the lowest neighboring ceiling, which is its own — `Refusal::Closing`,
/// nothing left standable — and `101` is not one of the four floor specials
/// the compiler emits, so membership refuses it too. Both are asserted: this
/// map is not the isolated recognizer refusal `FLOOR_TEXTMAP`'s twin would
/// be, and the test would otherwise pass on the membership axis alone.
const SEALING_PILLAR_TEXTMAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 0.000; y = 128.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 256.000; y = 0.000; }
vertex { x = 256.000; y = 128.000; }
vertex { x = 384.000; y = 0.000; }
vertex { x = 384.000; y = 128.000; }
linedef { v1 = 3; v2 = 2; sidefront = 0; sideback = 1; twosided = true; special = 101; arg0 = 7; }
linedef { v1 = 5; v2 = 4; sidefront = 2; sideback = 3; twosided = true; }
linedef { v1 = 0; v2 = 1; sidefront = 4; blocking = true; }
linedef { v1 = 1; v2 = 3; sidefront = 5; blocking = true; }
linedef { v1 = 2; v2 = 0; sidefront = 6; blocking = true; }
linedef { v1 = 3; v2 = 5; sidefront = 7; blocking = true; }
linedef { v1 = 4; v2 = 2; sidefront = 8; blocking = true; }
linedef { v1 = 5; v2 = 7; sidefront = 9; blocking = true; }
linedef { v1 = 7; v2 = 6; sidefront = 10; blocking = true; }
linedef { v1 = 6; v2 = 4; sidefront = 11; blocking = true; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 1; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 2; texturemiddle = "-"; texturebottom = "SUPPORT3"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sidedef { sector = 2; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 7; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 0; heightceiling = 256; lightlevel = 160; id = 0; }
thing { x = 64.0; y = 64.0; angle = 90; type = 1; single = true; }
"#;

#[test]
fn a_pillar_that_seals_refuses_the_map_on_the_floor_axis() {
    let path = write_temp(
        &common::wad_with_textmap(SEALING_PILLAR_TEXTMAP),
        "floors-closing",
    );
    let human = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    let json = bin()
        .args([path.to_str().unwrap(), "--vocabulary", "--json"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&human.stdout);
    assert!(stdout.contains("; expressible: no"), "{stdout}");
    assert!(stdout.contains("(floors refused: 1)"), "{stdout}");
    assert!(
        stdout.contains("(line specials unknown: 101)"),
        "101 is not emittable either: {stdout}"
    );
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).expect("json");
    assert_eq!(value[0]["verdict"]["floors_ok"], false);
    assert_eq!(value[0]["verdict"]["line_specials_ok"], false);
    assert_eq!(value[0]["floors"]["targets"], 1);
    assert_eq!(value[0]["floors"]["closing"], 1);
}

/// Without a refusal, the human line carries no floor note at all — the same
/// silence the teleport and lift axes keep.
#[test]
fn a_floor_free_map_gets_no_floor_note() {
    let path = write_temp(&common::binary_entrada_wad(), "floors-none");
    let out = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("; expressible: yes"), "{stdout}");
    assert!(!stdout.contains("floors refused"), "{stdout}");
}

/// Without a refusal, the human line carries no teleport note at all.
#[test]
fn a_teleport_free_map_gets_no_teleport_note() {
    let path = write_temp(&common::binary_entrada_wad(), "teleport-none");
    let out = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("; expressible: yes"), "{stdout}");
    assert!(!stdout.contains("teleports refused"), "{stdout}");
}
