//! CLI tests for `crustygen-corpus` over a temp directory.

mod common;

use std::path::PathBuf;
use std::process::Command;

#[test]
fn the_stored_zip_helper_produces_an_archive_crustywad_can_read() {
    let wad = common::binary_entrada_wad();
    let zip = common::stored_zip(&[("README.TXT", b"hi"), ("ENTRADA.WAD", &wad)]);
    let archive = crustywad::archive::Archive::from_bytes(zip).expect("opens");
    assert_eq!(archive.members().len(), 2);
    let member = archive.member("ENTRADA.WAD").expect("member");
    let parsed = archive.wad(member).expect("reads as a WAD");
    assert_eq!(parsed.map_groups().len(), 1);
}

/// The binary under test.
fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_crustygen-corpus"))
}

/// A fresh temp directory populated with `files`.
fn corpus_dir(label: &str, files: &[(&str, Vec<u8>)]) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!(
        "crustygen-corpus-{label}-{}-{nanos}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    dir
}

/// A WAD with a lump but no map group.
fn resource_wad() -> Vec<u8> {
    crustywad::WadBuilder::new(crustywad::WadKind::Pwad)
        .add_lump("DUMMY", b"not a map".to_vec())
        .build()
        .unwrap()
}

#[test]
fn a_mixed_directory_sweeps_dedups_and_buckets() {
    let entrada = common::binary_entrada_wad();
    let dir = corpus_dir(
        "mixed",
        &[
            ("a.wad", common::udmf_entrada_wad()),
            ("b.WAD", entrada.clone()),
            (
                "c.zip",
                common::stored_zip(&[("README.TXT", b"x"), ("sub/ENTRADA.WAD", &entrada)]),
            ),
            ("d.zip", common::stored_zip(&[("RES.WAD", &resource_wad())])),
            ("broken.zip", b"PK\x03\x04junk".to_vec()),
            ("e.wad", common::binary_entrada_wad_with_broken_second_map()),
            ("notes.txt", b"ignored".to_vec()),
        ],
    );
    let json_path = dir.join("out.json");
    let out = bin()
        .args([dir.to_str().unwrap(), "--json", json_path.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(1),
        "broken.zip and e.wad's MAP02 fail: {stderr}"
    );
    assert!(stderr.contains("broken.zip"), "{stderr}");
    assert!(stderr.contains("MAP02"), "{stderr}");
    // The map-free member is named even though it does not move the exit code.
    assert!(stderr.contains("RES.WAD: no map groups"), "{stderr}");
    // `--json` alone routes the report to the file, leaving stdout empty.
    assert!(out.stdout.is_empty(), "{:?}", out.stdout);
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let b = &report["buckets"];
    assert_eq!(
        b["archives"], 2,
        "c.zip and d.zip open; broken.zip does not"
    );
    assert_eq!(b["archive_unreadable"], 1);
    assert_eq!(b["wads"], 5, "a, b, c!sub/ENTRADA, d!RES, e");
    assert_eq!(b["no_maps"], 1);
    assert_eq!(
        b["unsupported_format"].as_u64().unwrap() + b["assembly_refused"].as_u64().unwrap(),
        1,
        "e.wad's Hexen-flagged MAP02 fails either at strict assembly or at the format gate"
    );
    assert_eq!(b["maps_raw"], 4, "a, b, c, e MAP01");
    assert_eq!(
        b["maps_unique"], 2,
        "the binary entrada appears three times; the UDMF one once"
    );
    assert_eq!(report["aggregate"]["all"]["expressible"], 1.0);
    assert!(report["provenance"].is_null());
    assert_eq!(report["maps"].as_array().unwrap().len(), 2);
    assert_eq!(report["maps"][0]["origin"], "udmf");
    // Entrada has no teleport line, so the fourth axis passes vacuously and
    // the teleport aggregate is empty.
    assert_eq!(report["aggregate"]["teleports"]["maps_with_teleports"], 0);
    assert_eq!(report["maps"][0]["verdict"]["teleports_ok"], true);
    assert_eq!(report["maps"][0]["teleports"]["lines"], 0);
}

#[test]
fn report_goes_to_stdout_by_default_and_to_a_file_with_the_flag() {
    let dir = corpus_dir("report", &[("a.wad", common::binary_entrada_wad())]);
    let out = bin().arg(&dir).output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.starts_with("# Corpus expressibility"), "{stdout}");
    assert!(stdout.contains("upper bound"));
    assert!(stdout.contains("## Teleports"), "{stdout}");
    let md = dir.join("r.md");
    let out = bin()
        .args([dir.to_str().unwrap(), "--report", md.to_str().unwrap()])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(out.stdout.is_empty());
    assert!(
        std::fs::read_to_string(&md)
            .unwrap()
            .contains("## Expressibility")
    );
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn provenance_is_echoed_when_a_sample_manifest_is_present() {
    let dir = corpus_dir(
        "prov",
        &[
            (
                "1-a.zip",
                common::stored_zip(&[("A.WAD", &common::binary_entrada_wad())]),
            ),
            (
                "sample-manifest.json",
                br#"{"seed":5,"count":1,"frame_rows":3,"fetch_list_hash":"blake3:ab","entries":[{"id":1,"dir":"d/","filename":"a.zip","zip_size":1,"status":"ok"}]}"#
                    .to_vec(),
            ),
        ],
    );
    let out = bin().arg(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("seed `5`"), "{stdout}");
    assert!(stdout.contains("ids: 1"), "{stdout}");
}

#[test]
fn usage_and_empty_directories_exit_2() {
    assert_eq!(bin().output().unwrap().status.code(), Some(2));
    assert_eq!(
        bin().args(["x", "--nope"]).output().unwrap().status.code(),
        Some(2)
    );
    assert_eq!(
        bin().args(["x", "y"]).output().unwrap().status.code(),
        Some(2)
    );
    assert_eq!(
        bin().args(["x", "--json"]).output().unwrap().status.code(),
        Some(2)
    );
    assert_eq!(
        bin()
            .args(["x", "--report"])
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        bin()
            .arg("/nonexistent-dir")
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let dir = corpus_dir("empty", &[("notes.txt", b"x".to_vec())]);
    let out = bin().arg(&dir).output().unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("no .zip or .wad candidates"));
}

/// A map-free WAD is ordinary corpus content: counted and named, but not a
/// load failure, so a corpus of nothing but resource WADs still exits 0.
#[test]
fn a_map_free_wad_is_counted_but_does_not_fail_the_run() {
    let dir = corpus_dir("resource", &[("res.wad", resource_wad())]);
    let json_path = dir.join("out.json");
    let out = bin()
        .args([dir.to_str().unwrap(), "--json", json_path.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(stderr.contains("res.wad: no map groups"), "{stderr}");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(report["buckets"]["no_maps"], 1);
    assert_eq!(report["buckets"]["maps_unique"], 0);
    assert_eq!(report["buckets"]["wads"], 1);
    assert_eq!(report["maps"].as_array().unwrap().len(), 0);
}

/// Both unreadable-WAD paths — a bare file that is not a WAD, and a zip
/// member that opens as an archive entry but not as a WAD — land in the
/// same bucket and are each named.
#[test]
fn unreadable_wads_are_bucketed_from_both_the_bare_and_the_member_path() {
    let dir = corpus_dir(
        "unreadable",
        &[
            ("junk.wad", b"not a wad at all".to_vec()),
            ("j.zip", common::stored_zip(&[("JUNK.WAD", b"not a wad")])),
        ],
    );
    let json_path = dir.join("out.json");
    let out = bin()
        .args([dir.to_str().unwrap(), "--json", json_path.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("junk.wad"), "{stderr}");
    assert!(stderr.contains("j.zip!JUNK.WAD"), "{stderr}");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(report["buckets"]["wad_unreadable"], 2);
    assert_eq!(report["buckets"]["archives"], 1, "the zip itself opens");
    assert_eq!(report["buckets"]["maps_unique"], 0);
}

/// A `TEXTMAP` that is not UTF-8 is a per-map load failure, not a sweep
/// abort: the bucket counts it, the map is dropped, and the run exits 1.
#[test]
fn a_non_utf8_textmap_is_bucketed_as_unparseable() {
    let dir = corpus_dir(
        "textmap",
        &[("bad.wad", common::wad_with_textmap(&[0xff, 0xfe, b'x'][..]))],
    );
    let json_path = dir.join("out.json");
    let out = bin()
        .args([dir.to_str().unwrap(), "--json", json_path.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("bad.wad MAP01"), "{stderr}");
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(report["buckets"]["textmap_unparseable"], 1);
    assert_eq!(report["buckets"]["maps_unique"], 0);
    assert_eq!(report["buckets"]["wads"], 1);
}

/// A Hexen map that assembles cleanly reaches the ingest path's Doom-format
/// gate and is refused there — the `unsupported_format` bucket, distinct
/// from the `assembly_refused` one.
#[test]
fn a_loadable_hexen_map_is_bucketed_as_an_unsupported_format() {
    let dir = corpus_dir("hexen", &[("hexen.wad", common::hexen_entrada_wad())]);
    let json_path = dir.join("out.json");
    let out = bin()
        .args([dir.to_str().unwrap(), "--json", json_path.to_str().unwrap()])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains("unsupported binary map format Hexen"),
        "{stderr}"
    );
    let report: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&json_path).unwrap()).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(report["buckets"]["unsupported_format"], 1);
    assert_eq!(
        report["buckets"]["assembly_refused"], 0,
        "the fixture must assemble, so the format gate is what refuses it"
    );
    assert_eq!(report["buckets"]["maps_unique"], 0);
}
