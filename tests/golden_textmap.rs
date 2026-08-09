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

/// A locked door, its key, and the room it guards.
const LOCKED_DOOR: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
  "rooms":[
    { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[
        { "kind":"player1_start", "at":[128,128], "angle":90 },
        { "kind":"blue_card", "at":[64,64], "angle":0 }
      ] },
    { "id":"b", "footprint":[[256,0],[256,256],[512,256],[512,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
  ],
  "portals":[{ "a":"a", "b":"b", "kind":"locked", "lock":"blue_card",
               "width":128, "at":[256,128] }] }"#;

#[test]
fn a_locked_door_survives_the_round_trip_with_its_special_and_tag() {
    // End to end: the special and the sector tag the compiler writes must
    // still be there after crustywad parses the emitted UDMF. In the `doom`
    // namespace crustywad reads a linedef's sector tag from `arg0`, which is
    // exactly what `emit_textmap` writes alongside `special`, so this pins
    // the emission format as well as the values.
    let ir = Ir::from_json(LOCKED_DOOR).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");

    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump("MAP01", b"");
    builder.add_lump("TEXTMAP", out.textmap.as_bytes());
    builder.add_lump("ENDMAP", b"");
    let wad = Wad::from_bytes(builder.build().expect("serializes")).expect("parses");
    let group = wad.map_group("MAP01").expect("group");
    let map = Map::assemble(&wad, &group).expect("assembles");

    assert_eq!(map.sectors().len(), 3, "two rooms plus the door sector");

    let keyed = i32::from(
        tables
            .locked_door_special("blue_card")
            .expect("blue_card special"),
    );
    let door_tag = i32::from(
        out.tags
            .manifest()
            .first()
            .expect("the door allocated a tag")
            .tag,
    );
    assert_ne!(door_tag, 0, "an action line never sits at tag 0");

    let action_lines: Vec<_> = map
        .linedefs()
        .iter()
        .filter(|l| l.special.special != 0)
        .collect();
    assert_eq!(action_lines.len(), 2, "both door faces are usable");
    for line in action_lines {
        assert_eq!(
            line.special.special, keyed,
            "the keyed door special survived"
        );
        assert_eq!(line.special.args[0], door_tag, "the sector tag survived");
    }

    // The key itself is placed, so the lock is actually openable.
    let key = i32::from(tables.thing_id("blue_card").expect("blue_card thing"));
    assert!(
        map.things().iter().any(|t| i32::from(t.type_id) == key),
        "the blue card is in the map"
    );
}
