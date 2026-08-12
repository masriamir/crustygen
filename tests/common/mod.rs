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
