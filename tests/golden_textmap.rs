//! Pins compiler output and proves it assembles as a real map.

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::tables::Tables;
use crustywad::map::Map;
use crustywad::{Wad, WadBuilder, WadKind};

const TWO_ROOM: &str = include_str!("golden/two_room.json");

#[test]
fn compiler_output_matches_the_golden_fixture() {
    let ir = Ir::from_json(TWO_ROOM).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");
    let golden = include_str!("golden/two_room.textmap");
    assert_eq!(
        out.textmap, golden,
        "compiler output drifted from the golden fixture"
    );
}

#[test]
fn compiler_output_assembles_through_crustywad() {
    let ir = Ir::from_json(TWO_ROOM).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");

    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump("MAP01", b"");
    builder.add_lump("TEXTMAP", out.textmap.as_bytes());
    builder.add_lump("ENDMAP", b"");
    let bytes = builder.build().expect("serializes");

    let wad = Wad::from_bytes(bytes).expect("parses");
    let group = wad.map_group("MAP01").expect("group");
    let map = Map::assemble(&wad, &group).expect("assembles");

    assert_eq!(map.sectors().len(), 2, "two rooms");
    assert_eq!(map.things().len(), 1, "one player start");
    assert!(
        map.linedefs().iter().any(|l| map.linedef_left(l).is_some()),
        "the portal produced a two-sided line"
    );
}
