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
linedef { v1 = 0; v2 = 1; sidefront = 0; special = 97; }
linedef { v1 = 1; v2 = 2; sidefront = 1; }
linedef { v1 = 2; v2 = 3; sidefront = 2; }
linedef { v1 = 3; v2 = 0; sidefront = 3; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; } sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; }
thing { x = 64; y = 64; type = 1; } thing { x = 80; y = 64; type = 46; }
"#;
    let path = write_temp(&common::wad_with_textmap(textmap), "vocab-unknown");
    let out = bin()
        .args([path.to_str().unwrap(), "--vocabulary"])
        .output()
        .expect("runs");
    std::fs::remove_file(&path).ok();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("expressible: no"), "{stdout}");
    assert!(stdout.contains("line specials unknown: 97"), "{stdout}");
    // 46 is not yet in [things]; Task B11 adds the decoration rows and
    // should switch this literal to a doomednum no vanilla entry defines
    // (e.g. 9999), noting the change in that task's commit.
    assert!(stdout.contains("thing types unknown: 46"), "{stdout}");
}
