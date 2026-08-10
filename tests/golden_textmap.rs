//! Pins compiler output and proves it assembles as a real map.

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::tables::Tables;
use crustywad::map::Map;
use crustywad::{Wad, WadBuilder, WadKind};

const TWO_ROOM: &str = include_str!("golden/two_room.json");
const STEPPED: &str = include_str!("golden/stepped_rooms.json");

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

    assert_eq!(
        map.sectors().len(),
        3,
        "two rooms plus the portal's passage sector"
    );
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
    { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
  ],
  "portals":[{ "a":"a", "b":"b", "kind":"locked", "lock":"blue_card",
               "width":128, "at":[256,128], "door_thickness":32,
               "alcove_near":16, "alcove_far":16 }] }"#;

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

    assert_eq!(
        map.sectors().len(),
        5,
        "two rooms plus the door's 3-segment chain (a near and a far trim alcove flanking \
         the door itself)"
    );

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

/// A switch exit on room `a`'s south wall, a secret sector `b`, and one
/// monster with a restricted skill set — the three new capabilities in one
/// map, proving each survives a real crustywad round trip rather than only
/// looking right as compiler-internal `MapData`.
const EXIT_SECRET_AND_SKILLS: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
  "rooms":[
    { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[
        { "kind":"player1_start", "at":[128,128], "angle":90 },
        { "kind":"imp", "at":[64,64], "angle":0,
          "skills": { "skill1":false, "skill2":false, "skill4":false, "skill5":false } }
      ] },
    { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
      "floor":0, "ceiling":128, "light":160, "secret":true,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
  ],
  "exits":[{ "room":"a", "trigger":"switch", "width":64, "at":[128,0] }],
  "portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }] }"#;

#[test]
fn an_exit_a_secret_sector_and_restricted_skills_all_survive_the_round_trip() {
    let ir = Ir::from_json(EXIT_SECRET_AND_SKILLS).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");

    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump("MAP01", b"");
    builder.add_lump("TEXTMAP", out.textmap.as_bytes());
    builder.add_lump("ENDMAP", b"");
    let wad = Wad::from_bytes(builder.build().expect("serializes")).expect("parses");
    let group = wad.map_group("MAP01").expect("group");
    let map = Map::assemble(&wad, &group).expect("assembles");

    // The exit: a real, usable, one-sided switch line.
    let exit_special = i32::from(tables.exit_switch_special());
    let exit_lines: Vec<_> = map
        .linedefs()
        .iter()
        .filter(|l| l.special.special == exit_special)
        .collect();
    assert_eq!(exit_lines.len(), 1, "exactly one exit line assembled");
    assert!(
        map.linedef_left(exit_lines[0]).is_none(),
        "the switch exit stays one-sided"
    );

    // The secret sector: room b carries the sourced secret special.
    let secret_special = i32::from(tables.secret_sector_special());
    assert_eq!(
        map.sectors().len(),
        3,
        "two rooms plus the portal's own passage sector; no door or alcove sector here"
    );
    assert_eq!(
        map.sectors()[1].special,
        secret_special,
        "room b assembled with the secret sector special"
    );
    assert_eq!(
        map.sectors()[0].special,
        0,
        "room a, which never opted in, has no special"
    );

    // The skill-restricted imp: crustywad folds skill1|skill2 -> bit 0,
    // skill3 -> bit 1, skill4|skill5 -> bit 2 (see `map::graph::MapThing`).
    // All four excluded skills clear both outer bits; skill3's default true
    // keeps the middle bit set.
    let imp_id = tables.thing_id("imp").expect("imp thing id");
    let imp = map
        .things()
        .iter()
        .find(|t| t.type_id == imp_id)
        .expect("the imp is in the map");
    assert_eq!(imp.flags & 0b111, 0b010, "only skill3 (bit 1) survived");
}

/// An octagon: a 256-unit square chamfered by 64 units at each corner — the
/// same shape used throughout `src/compile/*.rs`'s own unit tests
/// (`sectors::tests::OCTAGON`, `portals::tests::OCTAGON_ROOM`, etc.), with a
/// player start placed well clear of every wall, diagonal or not. Every
/// diagonal-geometry fixture elsewhere in this crate only ever proves the
/// compiler's own internal `MapData` is well formed; this is the one place
/// that proves a diagonal-edged room also parses and assembles through
/// crustywad's own binary UDMF reader, exactly like `TWO_ROOM` above does
/// for the axis-aligned case — the WAD/UDMF format itself places no
/// axis-alignment constraint on a linedef, so this is expected to just
/// work, but "expected to" is not the same claim as "proven to".
const OCTAGON_ROOM: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
  "rooms":[
    { "id":"a",
      "footprint":[[0,64],[0,192],[64,256],[192,256],[256,192],[256,64],[192,0],[64,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] }
  ],
  "portals":[] }"#;

#[test]
fn a_diagonally_shaped_room_assembles_through_crustywad() {
    let ir = Ir::from_json(OCTAGON_ROOM).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");

    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump("MAP01", b"");
    builder.add_lump("TEXTMAP", out.textmap.as_bytes());
    builder.add_lump("ENDMAP", b"");
    let bytes = builder.build().expect("wad serializes");

    let wad = Wad::from_bytes(bytes).expect("wad parses");
    let group = wad.map_group("MAP01").expect("MAP01 group present");
    let map = Map::assemble(&wad, &group).expect("a diagonal-edged room assembles");

    assert_eq!(map.sectors().len(), 1, "one sector");
    assert_eq!(map.vertices().len(), 8, "all eight octagon corners");
    assert_eq!(
        map.linedefs().len(),
        8,
        "eight walls, four of them diagonal"
    );
    assert_eq!(map.things().len(), 1, "the player start");
}

#[test]
fn stepped_output_matches_the_golden_fixture() {
    let ir = Ir::from_json(STEPPED).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");
    let golden = include_str!("golden/stepped_rooms.textmap");
    assert_eq!(
        out.textmap, golden,
        "compiler output drifted from the stepped golden fixture"
    );
}

/// Rewrites `tests/golden/stepped_rooms.textmap` from the current compiler.
///
/// Ignored by default so a drifting compiler fails the test above rather than
/// silently rewriting its own expectation. Run deliberately with
/// `cargo test --test golden_textmap regenerate -- --ignored`, then read the
/// diff before committing it.
#[test]
#[ignore = "regenerates a golden fixture; run explicitly"]
fn regenerate_stepped_golden() {
    let ir = Ir::from_json(STEPPED).expect("ir");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("compiles");
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/stepped_rooms.textmap");
    std::fs::write(path, &out.textmap).expect("write golden");
}
