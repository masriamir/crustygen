//! Shared fixture builders for the integration tests: entrada compiled to a
//! UDMF PWAD, the same map round-tripped into a classic binary-format PWAD
//! (with real nodes, via crustywad's one-shot builder), and a raw
//! TEXTMAP-wrapping helper.

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::pack::pack_udmf;
use crustygen::tables::Tables;
use crustywad::map::Map;
use crustywad::map::build::{NodeBuildOptions, add_doom_map_with_nodes};
use crustywad::{Wad, WadBuilder, WadKind, WriteOptions};

/// The entrada base IR fixture, shared with `check_cli.rs`.
pub const ENTRADA: &str = include_str!("../fixtures/entrada_base.json");

/// Compiles entrada and packs it as a minimal un-noded UDMF PWAD.
pub fn udmf_entrada_wad() -> Vec<u8> {
    let tables = Tables::load().expect("tables load");
    let ir = Ir::from_json(ENTRADA).expect("ir parses");
    let compiled = compile(&ir, &tables).expect("entrada compiles");
    pack_udmf(&compiled, "MAP01").expect("packs")
}

/// Compiles entrada, assembles it, and re-emits it as a classic Doom
/// binary-format PWAD with real nodes — the vanilla-input fixture for the
/// ingest/check/lift tests. No retail WAD is involved (redistributability).
pub fn binary_entrada_wad() -> Vec<u8> {
    let udmf = udmf_entrada_wad();
    let wad = Wad::from_bytes(udmf).expect("udmf fixture parses");
    let group = wad
        .map_groups()
        .into_iter()
        .next()
        .expect("udmf fixture has a map group");
    let map = Map::assemble(&wad, &group).expect("udmf fixture assembles");
    let mut builder = WadBuilder::new(WadKind::Pwad);
    add_doom_map_with_nodes(
        &mut builder,
        "MAP01",
        &map,
        &WriteOptions::default(),
        &NodeBuildOptions::default(),
    )
    .expect("binary map with nodes builds");
    builder.build().expect("wad builds")
}

/// The binary entrada WAD plus a second, deliberately unloadable map group:
/// a `MAP02` marker whose data lumps are MAP01's five Doom-format lumps with
/// a zero-length `BEHAVIOR` appended, which flips detection to the Hexen
/// format. Whether assembly then fails on record shape or the format gate
/// refuses it, `MAP02` cannot load — which is the point: a per-map failure
/// among survivors.
pub fn binary_entrada_wad_with_broken_second_map() -> Vec<u8> {
    let base = binary_entrada_wad();
    let wad = Wad::from_bytes(base).expect("binary fixture parses");
    let lumps = wad.lumps();
    let group = wad
        .map_groups()
        .into_iter()
        .next()
        .expect("binary fixture has a map group");
    let mut builder = WadBuilder::new(WadKind::Pwad);
    // MAP01, intact.
    builder.add_lump(group.name.as_str(), Vec::new());
    for &i in &group.data_indices {
        builder.add_lump(lumps[i].name(), wad.lump_data(&lumps[i]).to_vec());
    }
    // MAP02, Hexen-flagged copy.
    builder.add_lump("MAP02", Vec::new());
    for &i in &group.data_indices {
        builder.add_lump(lumps[i].name(), wad.lump_data(&lumps[i]).to_vec());
    }
    builder.add_lump("BEHAVIOR", Vec::new());
    builder.build().expect("wad builds")
}

/// A minimal one-map UDMF PWAD wrapping `textmap` verbatim (same shape as
/// `check_cli.rs`'s private helper).
pub fn wad_with_textmap(textmap: impl Into<Vec<u8>>) -> Vec<u8> {
    WadBuilder::new(WadKind::Pwad)
        .add_lump("MAP01", Vec::new())
        .add_lump("TEXTMAP", textmap)
        .add_lump("ENDMAP", Vec::new())
        .build()
        .expect("builds")
}
