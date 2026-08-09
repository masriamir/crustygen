//! Builds the hand-authored "Entrada Base" fixture — the payoff map proving
//! the compiler produces something a player could actually finish, not just
//! something that parses. See the map-generation report for the full
//! honest accounting of what this does and does not verify.

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::pack::{pack_udmf, pack_udmf_with_nodes};
use crustygen::tables::Tables;
use crustywad::map::Map;
use crustywad::{Wad, WadKind};

const ENTRADA: &str = include_str!("fixtures/entrada_base.json");

#[test]
fn entrada_base_compiles_and_reassembles_through_crustywad() {
    let ir = Ir::from_json(ENTRADA).expect("ir parses");
    let tables = Tables::load().expect("tables load");
    let compiled = compile(&ir, &tables).expect("map compiles clean, no playability violations");

    assert_eq!(
        compiled.data.sectors.len(),
        16,
        "7 rooms + 4 plain passages + the manual door's 3-segment chain (a near and a far \
         trim alcove flanking the door itself) + the locked door's 2-segment chain (a near \
         trim alcove flanking the door itself, no far alcove) = 7 + 4 + 3 + 2 = 16"
    );
    assert!(
        !compiled.things.is_empty(),
        "the map places at least one thing"
    );

    // The un-noded twin: the artifact a Doom-format downconvert must start
    // from.
    let plain_bytes = pack_udmf(&compiled, "MAP01").expect("packs un-noded");
    let plain_wad = Wad::from_bytes(plain_bytes.clone()).expect("un-noded WAD parses");
    assert!(
        !plain_wad.lumps().iter().any(|l| l.name() == "ZNODES"),
        "the un-noded twin carries no ZNODES lump"
    );
    let plain_group = plain_wad.map_group("MAP01").expect("MAP01 present");
    let plain_map = Map::assemble(&plain_wad, &plain_group).expect("un-noded map assembles");
    assert_eq!(plain_map.sectors().len(), 16);
    assert_eq!(plain_map.linedefs().len(), compiled.data.linedefs.len());

    // The noded artifact: MAP01/TEXTMAP/ZNODES/ENDMAP, engine-playable.
    let noded_bytes = pack_udmf_with_nodes(&compiled, "MAP01").expect("packs with nodes");
    let noded_wad = Wad::from_bytes(noded_bytes.clone()).expect("noded WAD parses");
    assert_eq!(noded_wad.kind(), WadKind::Pwad);
    assert!(
        noded_wad.lumps().iter().any(|l| l.name() == "ZNODES"),
        "the noded artifact carries a ZNODES lump"
    );
    let noded_group = noded_wad.map_group("MAP01").expect("MAP01 present");
    let noded_map = Map::assemble(&noded_wad, &noded_group).expect("noded map assembles");
    assert_eq!(
        noded_map.sectors().len(),
        16,
        "same geometry, both artifacts"
    );

    // Structural progression proof: exactly one key, one locked door special
    // gating it, and one exit special, all present in the reassembled map.
    let key_id = i32::from(tables.thing_id("blue_card").expect("blue_card thing id"));
    assert_eq!(
        plain_map
            .things()
            .iter()
            .filter(|t| i32::from(t.type_id) == key_id)
            .count(),
        1,
        "exactly one blue card is placed"
    );

    let keyed_special = i32::from(
        tables
            .locked_door_special("blue_card")
            .expect("blue_card door special"),
    );
    let keyed_lines: Vec<_> = plain_map
        .linedefs()
        .iter()
        .filter(|l| l.special.special == keyed_special)
        .collect();
    assert_eq!(keyed_lines.len(), 2, "both faces of the locked door");

    let exit_special = i32::from(tables.exit_switch_special());
    let exit_lines: Vec<_> = plain_map
        .linedefs()
        .iter()
        .filter(|l| l.special.special == exit_special)
        .collect();
    assert_eq!(exit_lines.len(), 1, "exactly one switch exit line");
    assert!(
        plain_map.linedef_left(exit_lines[0]).is_none(),
        "the switch exit stays one-sided, as P_UseSpecialLine expects"
    );

    // Write both artifacts out for the manual cwad-convert verification step
    // (Doom-format downconvert + cwad validate), same as the report
    // documents. Written under target/ so it never needs to be committed.
    let out_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/entrada");
    std::fs::create_dir_all(&out_dir).expect("create output dir");
    std::fs::write(out_dir.join("entrada_noded.wad"), &noded_bytes).expect("write noded wad");
    std::fs::write(out_dir.join("entrada_plain.wad"), &plain_bytes).expect("write plain wad");

    // Drift guard: `maps/entrada.wad` is the committed, human-loadable
    // artifact this fixture produces. If the IR or the compiler changes
    // without regenerating it, this fails loudly instead of leaving a stale
    // WAD sitting in the tree looking current. `maps/entrada_doom.wad` (the
    // Doom-format downconvert, built by shelling out to `cwad convert --to
    // doom --nodes` on the un-noded twin) is not re-checked here — it
    // depends on a prebuilt `cwad` binary this crate does not control the
    // path to — see the map-generation report for the exact command used to
    // produce it.
    let committed = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("maps/entrada.wad");
    let committed_bytes = std::fs::read(&committed).expect("read committed maps/entrada.wad");
    assert_eq!(
        committed_bytes, noded_bytes,
        "maps/entrada.wad is stale relative to the current IR/compiler output; \
         regenerate it from target/entrada/entrada_noded.wad"
    );
}
