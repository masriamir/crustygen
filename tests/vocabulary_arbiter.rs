//! Executable arbiter for `Tables::emittable_line_specials` and
//! `Tables::named_sector_specials`: the curated sets must equal what the
//! compiler really writes. Fixtures below cover every IR construct that
//! emits a special — plain, door and locked portals (one per key color), the
//! four exit kinds, the four teleport specials, a secret room,
//! `tests/golden/lifts.json`'s switch lift, fast walkover lift and fast
//! barrier — between them, every repeatable lift special — and
//! `tests/golden/floors.json` plus [`FLOOR_SWITCH_BRIDGE`], which between
//! them write all four emitted floor specials.
//!
//! These tests still do not detect a new emitting pass on their own: no
//! fixture can author a construct the IR cannot yet express. The teleport
//! pass is the worked example this file's doc used to predict — until the IR
//! grew `teleports[]`, nothing here could emit 97, so landing the pass meant
//! landing its fixtures and growing the curated set in the same change, by
//! rule rather than by detection. What the tests enforce is the other
//! direction — adding a special to the curated set without a fixture that
//! emits it breaks the equality assertion, and adding 21, 10, 122 or 121 —
//! the one-shot lift forms the tables source but no pass ever writes — breaks
//! `sourced_but_unemitted_specials_stay_out_of_the_emittable_set`.

use std::collections::BTreeSet;

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::tables::Tables;

/// Two rooms authored apart (gap 64 on x), `{PORTAL}`, `{KEY}`, `{EXITS}`,
/// `{TELEPORTS}` and `{THINGS_B}` filled per fixture. Geometry mirrors
/// `golden_textmap.rs`'s `LOCKED_DOOR`. Room `b` is secret so the sector
/// half of the arbiter has a value to find.
const BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
  "rooms":[
    { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[
        { "kind":"player1_start", "at":[192,64], "angle":90 }{KEY}
      ] },
    { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
      "floor":0, "ceiling":128, "light":160, "secret": true,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[{THINGS_B}] }
  ],
  "portals":[{PORTAL}],
  "exits":[{EXITS}],
  "teleports":[{TELEPORTS}] }"#;

/// The Task 5 lift golden: a switch lift (`both_ends`), a fast walkover
/// lift, and a fast barrier, plus a pedestal — every repeatable lift
/// special (62, 88, 123, 120) in one fixture. Unlike every fixture above, it
/// is a whole map on its own rather than a `{PORTAL}`/`{EXITS}`/... fill-in
/// for [`BASE`] — [`specials_of`]'s template has no room to author a lift
/// portal, so this fixture is compiled directly in
/// [`every_construct_the_compiler_emits_is_in_the_curated_sets`] instead of
/// going through it.
const LIFTS: &str = include_str!("golden/lifts.json");

/// The Task 13 floor golden: a drop wall and a closet reveal sharing one
/// switch's tag (23), a pedestal reveal on a walkover (38), and a bridge on
/// a walkover of its own (119) — three of the four emitted floor specials in
/// one fixture. Compiled directly for the same reason [`LIFTS`] is:
/// [`specials_of`]'s template has no room to author a floor construct.
const FLOORS: &str = include_str!("golden/floors.json");

/// The fourth emitted floor special, 18 (S1 `raiseFloorToNearest`): a bridge
/// on a *switch* rather than a walkover. [`FLOORS`] carries its bridge on a
/// walkover — the trigger form that cannot strand whoever steps into the pit
/// — so nothing there ever writes 18, and only a second fixture can.
///
/// Two rooms 64 apart over a pit 96 deep, one switch on room `a`'s far wall
/// and a switch exit in room `b`: the `BRIDGE` map from
/// `src/compile/floors.rs`'s own tests, verbatim.
const FLOOR_SWITCH_BRIDGE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
  "rooms":[
    { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ] },
    { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
  ],
  "portals":[ { "a":"a", "b":"b", "kind":"bridge", "width":64, "at":[256,128], "depth":96, "fires_on":"t" } ],
  "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
  "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

const PLAIN: &str = r#"{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }"#;
const DOOR: &str = r#"{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
  "door_thickness":32, "alcove_near":16, "alcove_far":16 }"#;

/// A free-standing pad in room `a` (square 64..128 × 128..192, clear of both
/// the player start and the west wall) delivering into room `b`. Repeatable
/// and crossable by anything: the `97` special. The pad is addressed by its
/// low corner, on the 64-unit flat grid — every aligned pad in this 256x256
/// room reaches the room's center, which is why the player start sits at
/// (192, 64) rather than there.
const TELEPORT_ISLAND: &str = r#"{ "id":"i", "room":"a", "pad":{"island":[64,128]},
  "to":{"room":"b","at":[448,128],"angle":90} }"#;
/// The same delivery from a pad recessed into room `a`'s north wall, fired
/// once: the `39` special.
const TELEPORT_WALL_ONE_SHOT: &str = r#"{ "id":"w", "room":"a", "pad":{"wall":[64,256]},
  "to":{"room":"b","at":[448,128],"angle":90}, "repeatable":false }"#;
/// A monsters-only pad in room `b` delivering into room `a`: the `126`
/// special. The destination is `[64,64]`, 128 units west of the player start
/// at (192, 64): nothing in the compiler polices an arrival point that lands
/// on another thing, so the fixture keeps the two apart by hand rather than
/// leaning on a check that is not there. Pair with
/// [`IMP_B`], whose species sets the clearance the destination must offer
/// (`teleports::arriving_dims` takes the largest species in the pad's own
/// room).
const TELEPORT_MONSTERS: &str = r#"{ "id":"m", "room":"b", "pad":{"island":[448,128]},
  "to":{"room":"a","at":[64,64],"angle":0}, "monsters_only":true }"#;
/// [`TELEPORT_MONSTERS`] fired once: the `125` special.
const TELEPORT_MONSTERS_ONE_SHOT: &str = r#"{ "id":"mo", "room":"b", "pad":{"island":[448,128]},
  "to":{"room":"a","at":[64,64],"angle":0}, "monsters_only":true, "repeatable":false }"#;
/// A monster in room `b`, clear of the pad square there. Rule P27 wants a
/// monster-holding room to have a portal or be a teleport destination, so
/// every fixture that places it also carries the [`PLAIN`] portal.
const IMP_B: &str = r#"{ "kind":"imp", "at":[384,64], "angle":0 }"#;

/// A door locked to `lock`, in place of the `DOOR` portal. One fixture per
/// key color: blue, yellow and red are three distinct specials in the
/// curated set, so only a fixture per color proves the compiler writes them.
/// Pair with [`key`] of the same kind — rule P24 rejects a key that opens no
/// door, so the key rides with the locked portal and never without it.
fn locked(lock: &str) -> String {
    format!(
        r#"{{ "a":"a", "b":"b", "kind":"locked", "lock":"{lock}", "width":128,
  "at":[256,128], "door_thickness":32, "alcove_near":16, "alcove_far":16 }}"#
    )
}

/// The key thing that opens a [`locked`] door of the same kind, appended to
/// room `a`'s things.
fn key(kind: &str) -> String {
    format!(
        r#",
        {{ "kind":"{kind}", "at":[64,64], "angle":0 }}"#
    )
}

/// Exit on room `a`'s west wall (x = 0, y in 0..256), centered at `at_y`.
/// Two exits in one fixture need distinct centers: each is carved from its
/// own wall span, and two sharing a span would leave only the last one's
/// special on the line.
fn exit(trigger: &str, secret: bool, at_y: i32) -> String {
    format!(
        r#"{{ "room":"a", "trigger":"{trigger}", "secret":{secret}, "width":64, "at":[0,{at_y}] }}"#
    )
}

/// A `teleport`-triggered exit on room **`b`**'s east wall (x = 576),
/// centered at `at_y`.
///
/// Not [`exit`] with a different trigger string: rule P26 requires a
/// teleport exit's room to carry no portal and to hold a teleport
/// destination, and room `a` — where the player starts and every teleport
/// fixture's pad sits — satisfies neither. Room `b` is the room the
/// teleports deliver into, so the exit goes there and the fixture drops its
/// portal.
fn teleport_exit(at_y: i32) -> String {
    format!(
        r#"{{ "room":"b", "trigger":"teleport", "secret":false, "width":64, "at":[576,{at_y}] }}"#
    )
}

/// Every non-zero line and sector special a whole-map fixture compiles to.
///
/// [`specials_of`] fills [`BASE`]'s template; this takes a fixture that is
/// already a complete map — [`LIFTS`], [`FLOORS`], [`FLOOR_SWITCH_BRIDGE`] —
/// because the template has no room to author a lift or floor construct.
fn specials_of_whole_map(json: &str) -> (BTreeSet<u16>, BTreeSet<u16>) {
    let ir = Ir::from_json(json).expect("fixture parses");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("fixture compiles");
    let lines = out
        .data
        .linedefs
        .iter()
        .map(|l| l.special)
        .filter(|&s| s != 0)
        .collect();
    let sectors = out
        .data
        .sectors
        .iter()
        .map(|s| s.special)
        .filter(|&s| s != 0)
        .collect();
    (lines, sectors)
}

fn specials_of(
    portal: &str,
    key_thing: &str,
    exits: &str,
    teleports: &str,
    things_b: &str,
) -> (BTreeSet<u16>, BTreeSet<u16>) {
    let json = BASE
        .replace("{PORTAL}", portal)
        .replace("{KEY}", key_thing)
        .replace("{EXITS}", exits)
        .replace("{TELEPORTS}", teleports)
        .replace("{THINGS_B}", things_b);
    let ir = Ir::from_json(&json).expect("fixture parses");
    let tables = Tables::load().expect("tables");
    let out = compile(&ir, &tables).expect("fixture compiles");
    let lines = out
        .data
        .linedefs
        .iter()
        .map(|l| l.special)
        .filter(|&s| s != 0)
        .collect();
    let sectors = out
        .data
        .sectors
        .iter()
        .map(|s| s.special)
        .filter(|&s| s != 0)
        .collect();
    (lines, sectors)
}

#[test]
fn every_construct_the_compiler_emits_is_in_the_curated_sets() {
    let tables = Tables::load().expect("tables");
    let mut lines = BTreeSet::new();
    let mut sectors = BTreeSet::new();
    let fixtures = [
        specials_of(PLAIN, "", "", "", ""),
        specials_of(DOOR, "", "", "", ""),
        specials_of(&locked("blue_card"), &key("blue_card"), "", "", ""),
        specials_of(&locked("yellow_card"), &key("yellow_card"), "", "", ""),
        // A skull rather than a card: the two key forms of one color share a
        // door special, and this fixture pins that half of the mapping too.
        specials_of(&locked("red_skull"), &key("red_skull"), "", "", ""),
        specials_of(
            PLAIN,
            "",
            &format!(
                "{}, {}",
                exit("switch", false, 64),
                exit("walkover", true, 192)
            ),
            "",
            "",
        ),
        specials_of(PLAIN, "", &exit("walkover", false, 128), "", ""),
        specials_of(PLAIN, "", &exit("switch", true, 128), "", ""),
        // One fixture per teleport special: an island pad (97), a wall pad
        // fired once (39), and the monsters-only pair (126, 125), which
        // needs a monster in the pad's room to be worth emitting at all.
        specials_of(PLAIN, "", &exit("switch", false, 128), TELEPORT_ISLAND, ""),
        specials_of(
            PLAIN,
            "",
            &exit("switch", false, 128),
            TELEPORT_WALL_ONE_SHOT,
            "",
        ),
        specials_of(
            PLAIN,
            "",
            &exit("switch", false, 128),
            TELEPORT_MONSTERS,
            IMP_B,
        ),
        specials_of(
            PLAIN,
            "",
            &exit("switch", false, 128),
            TELEPORT_MONSTERS_ONE_SHOT,
            IMP_B,
        ),
        // Rule P26's shape: no portal at all, and the exit sits in the room
        // the teleport delivers into, reached only across the pad.
        specials_of("", "", &teleport_exit(128), TELEPORT_ISLAND, ""),
    ];
    for (l, s) in fixtures {
        lines.extend(l);
        sectors.extend(s);
    }
    // The three whole-map fixtures, compiled directly since none of them
    // fits `BASE`'s template:
    //
    // - the lift golden: a switch lift (62/88, `both_ends`), a fast barrier
    //   (123 on both faces) and a fast walkover lift (120 on the alcove's
    //   outer threshold) — every repeatable lift special in one fixture;
    // - the floor golden: a switch driving a drop wall and a closet on one
    //   shared tag (23), a walkover driving a pedestal (38) and a walkover
    //   driving a bridge (119 on both of the pit's thresholds);
    // - the switch bridge: 18, the one emitted floor special the golden
    //   never writes.
    for (l, s) in [
        specials_of_whole_map(LIFTS),
        specials_of_whole_map(FLOORS),
        specials_of_whole_map(FLOOR_SWITCH_BRIDGE),
    ] {
        lines.extend(l);
        sectors.extend(s);
    }
    for floor in tables.floor_specials() {
        assert!(
            lines.contains(&floor),
            "no fixture emits floor special {floor}"
        );
    }
    assert_eq!(
        lines,
        tables.emittable_line_specials(),
        "curated emittable line specials drifted from what the compiler writes"
    );
    assert!(
        sectors.is_subset(&tables.named_sector_specials()),
        "compiler wrote a sector special the tables cannot name: {sectors:?}"
    );
    assert!(sectors.contains(&tables.secret_sector_special()));
}

#[test]
fn the_curated_sets_hold_their_expected_values_today() {
    let tables = Tables::load().expect("tables");
    assert_eq!(
        tables.emittable_line_specials(),
        BTreeSet::from([
            1, 26, 27, 28, 11, 51, 52, 124, 39, 97, 125, 126, 62, 88, 123, 120, 23, 38, 18, 119
        ])
    );
    assert_eq!(
        tables.named_sector_specials(),
        BTreeSet::from([9, 7, 5, 16, 1, 17, 8, 3])
    );
}

#[test]
fn sourced_but_unemitted_specials_stay_out_of_the_emittable_set() {
    let tables = Tables::load().expect("tables");
    let set = tables.emittable_line_specials();
    let repeatable = tables.lift_repeatable_specials();
    for s in repeatable {
        assert!(
            set.contains(&s),
            "repeatable lift special {s} should be emittable"
        );
    }
    for s in tables.lift_specials() {
        if !repeatable.contains(&s) {
            assert!(
                !set.contains(&s),
                "one-shot lift special {s} has no compiler pass — it is never emitted"
            );
        }
    }
    let emitted_floors = tables.floor_specials();
    for &(s, _, _) in tables.recognized_floor_specials() {
        if emitted_floors.contains(&s) {
            assert!(
                set.contains(&s),
                "emitted floor special {s} should be emittable"
            );
        } else {
            assert!(
                !set.contains(&s),
                "recognized-only floor special {s} has no compiler pass — it is never emitted"
            );
        }
    }
}

#[test]
fn the_vanilla_list_contains_every_emittable_special_and_matches_its_citation() {
    let tables = Tables::load().expect("tables");
    let vanilla = tables.vanilla_line_specials();
    assert_eq!(
        vanilla.len(),
        139,
        "distinct count recorded in the citation"
    );
    assert!(tables.emittable_line_specials().is_subset(&vanilla));
    for s in [tables.lift_switch_special(), tables.lift_walkover_special()]
        .into_iter()
        .chain(tables.teleport_specials())
    {
        assert!(vanilla.contains(&s));
    }
}
