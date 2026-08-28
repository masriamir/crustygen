//! Executable arbiter for `Tables::emittable_line_specials` and
//! `Tables::named_sector_specials`: the curated sets must equal what the
//! compiler really writes. Fixtures below cover every IR construct that
//! emits a special — plain, door and locked portals (one per key color), the
//! four exit kinds, and a secret room.
//!
//! These tests do not detect a new emitting pass on their own: no fixture can
//! author a construct the IR cannot yet express, so a landed teleport pass
//! leaves the fixtures' union unchanged. What they enforce is the other
//! direction — adding a special to the curated set without a fixture that
//! emits it breaks the equality assertion, and adding 62, 88 or 97 breaks
//! `sourced_but_unemitted_specials_stay_out_of_the_emittable_set`. So a new
//! pass lands its fixture and updates both tests by rule, not by detection.

use std::collections::BTreeSet;

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::tables::Tables;

/// Two rooms authored apart (gap 64 on x), `{PORTAL}`, `{KEY}` and `{EXITS}`
/// filled per fixture. Geometry mirrors `golden_textmap.rs`'s `LOCKED_DOOR`.
/// Room `b` is secret so the sector half of the arbiter has a value to find.
const BASE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
  "rooms":[
    { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
      "floor":0, "ceiling":128, "light":160,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
      "things":[
        { "kind":"player1_start", "at":[128,128], "angle":90 }{KEY}
      ] },
    { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
      "floor":0, "ceiling":128, "light":160, "secret": true,
      "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
  ],
  "portals":[{PORTAL}],
  "exits":[{EXITS}] }"#;

const PLAIN: &str = r#"{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }"#;
const DOOR: &str = r#"{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
  "door_thickness":32, "alcove_near":16, "alcove_far":16 }"#;

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

fn specials_of(portal: &str, key_thing: &str, exits: &str) -> (BTreeSet<u16>, BTreeSet<u16>) {
    let json = BASE
        .replace("{PORTAL}", portal)
        .replace("{KEY}", key_thing)
        .replace("{EXITS}", exits);
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
    let fixtures: [(String, String, String); 8] = [
        (PLAIN.to_owned(), String::new(), String::new()),
        (DOOR.to_owned(), String::new(), String::new()),
        (locked("blue_card"), key("blue_card"), String::new()),
        (locked("yellow_card"), key("yellow_card"), String::new()),
        // A skull rather than a card: the two key forms of one color share a
        // door special, and this fixture pins that half of the mapping too.
        (locked("red_skull"), key("red_skull"), String::new()),
        (
            PLAIN.to_owned(),
            String::new(),
            format!(
                "{}, {}",
                exit("switch", false, 64),
                exit("walkover", true, 192)
            ),
        ),
        (
            PLAIN.to_owned(),
            String::new(),
            exit("walkover", false, 128),
        ),
        (PLAIN.to_owned(), String::new(), exit("switch", true, 128)),
    ];
    for (portal, key_thing, exits) in &fixtures {
        let (l, s) = specials_of(portal, key_thing, exits);
        lines.extend(l);
        sectors.extend(s);
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
        BTreeSet::from([1, 26, 27, 28, 11, 51, 52, 124])
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
    for s in [
        tables.lift_switch_special(),
        tables.lift_walkover_special(),
        tables.teleport_special(),
    ] {
        assert!(!set.contains(&s), "special {s} has no compiler pass yet");
    }
}

#[test]
fn the_vanilla_list_contains_every_emittable_special_and_matches_its_citation() {
    let tables = Tables::load().expect("tables");
    let vanilla = tables.vanilla_line_specials();
    assert_eq!(
        vanilla.len(),
        138,
        "distinct count recorded in the citation"
    );
    assert!(tables.emittable_line_specials().is_subset(&vanilla));
    for s in [
        tables.lift_switch_special(),
        tables.lift_walkover_special(),
        tables.teleport_special(),
    ] {
        assert!(vanilla.contains(&s));
    }
}
