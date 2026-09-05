//! Library-level tests for `crustygen::ingest` (issue #21): the binary
//! round-trip path, exercised against binary re-emissions of entrada, of
//! ascensor (the lift playtest map) and of muralla (the floor one).

mod common;

use crustygen::ingest::{self, IngestError, MapOrigin};
use crustywad::Wad;
use crustywad::map::MapGroup;

/// Parses `bytes` and returns the WAD plus its first map group.
fn first_group(bytes: Vec<u8>) -> (Wad, MapGroup) {
    let wad = Wad::from_bytes(bytes).expect("fixture parses as a WAD");
    let group = wad
        .map_groups()
        .into_iter()
        .next()
        .expect("fixture has a map group");
    (wad, group)
}

#[test]
fn binary_entrada_loads_via_assembly() {
    let (wad, group) = first_group(common::binary_entrada_wad());
    let loaded = ingest::load_map(&wad, &group).expect("binary map loads");
    assert_eq!(loaded.origin, MapOrigin::AssembledFromBinary);
    assert!(!loaded.map.sectors.is_empty(), "assembled map has sectors");
    assert!(!loaded.map.things.is_empty(), "assembled map has things");
    // The expected NamespaceDefaulted warning is filtered, and entrada's
    // geometry loses nothing a strict doom-format write would warn about.
    assert!(
        loaded.notes.is_empty(),
        "unexpected notes: {:?}",
        loaded.notes
    );
}

/// Ascensor, the lift playtest map, through the same binary round trip:
/// the platform sectors, their tags and their use/walkover specials all
/// survive the downconvert and come back through assembly.
#[test]
fn binary_ascensor_loads_via_assembly() {
    let (wad, group) = first_group(common::binary_ascensor_wad());
    let loaded = ingest::load_map(&wad, &group).expect("binary map loads");
    assert_eq!(loaded.origin, MapOrigin::AssembledFromBinary);
    assert!(!loaded.map.sectors.is_empty(), "assembled map has sectors");
    assert!(!loaded.map.things.is_empty(), "assembled map has things");
    // The four lift specials the map emits — 62 (lift and pedestal
    // switches), 88 and 120 (walkover lifts), 123 (fast switches on the
    // ledge lift and the barrier) — must all come back from LINEDEFS.
    let specials: std::collections::BTreeSet<i32> =
        loaded.map.linedefs.iter().map(|l| l.special).collect();
    for special in [62, 88, 120, 123] {
        assert!(
            specials.contains(&special),
            "lift special {special} lost in the round trip; present: {specials:?}"
        );
    }
    assert!(
        loaded.notes.is_empty(),
        "unexpected notes: {:?}",
        loaded.notes
    );
}

/// Muralla, the floor playtest map, through the same binary round trip: the
/// drop wall, the reveal cell and the bridge pit, their tags and their
/// use/walkover specials all survive the downconvert and come back through
/// assembly.
#[test]
fn binary_muralla_loads_via_assembly() {
    let (wad, group) = first_group(common::binary_muralla_wad());
    let loaded = ingest::load_map(&wad, &group).expect("binary map loads");
    assert_eq!(loaded.origin, MapOrigin::AssembledFromBinary);
    assert!(!loaded.map.sectors.is_empty(), "assembled map has sectors");
    assert!(!loaded.map.things.is_empty(), "assembled map has things");
    // The three floor specials the map emits — 23 (the switch that drops the
    // wall), 38 (the walkover that lowers the pedestal reveal) and 119 (the
    // walkover on each of the bridge's two thresholds) — plus 28, the red-card
    // door the reveal's card opens, must all come back from LINEDEFS.
    let specials: std::collections::BTreeSet<i32> =
        loaded.map.linedefs.iter().map(|l| l.special).collect();
    for special in [23, 28, 38, 119] {
        assert!(
            specials.contains(&special),
            "special {special} lost in the round trip; present: {specials:?}"
        );
    }
    // Both bridge thresholds carry the rise, not just the near one: the
    // player fires it from whichever side they step down into the pit.
    assert_eq!(
        loaded
            .map
            .linedefs
            .iter()
            .filter(|l| l.special == 119)
            .count(),
        2,
        "the bridge's rise is written on both of the pit's thresholds"
    );
    assert!(
        loaded.notes.is_empty(),
        "unexpected notes: {:?}",
        loaded.notes
    );
}

#[test]
fn udmf_entrada_loads_directly() {
    let (wad, group) = first_group(common::udmf_entrada_wad());
    let loaded = ingest::load_map(&wad, &group).expect("udmf map loads");
    assert_eq!(loaded.origin, MapOrigin::Udmf);
    assert!(loaded.notes.is_empty());
}

#[test]
fn binary_round_trip_preserves_element_counts() {
    let (uwad, ugroup) = first_group(common::udmf_entrada_wad());
    let (bwad, bgroup) = first_group(common::binary_entrada_wad());
    let udmf = ingest::load_map(&uwad, &ugroup).expect("udmf loads");
    let binary = ingest::load_map(&bwad, &bgroup).expect("binary loads");
    assert_eq!(udmf.map.linedefs.len(), binary.map.linedefs.len());
    assert_eq!(udmf.map.sidedefs.len(), binary.map.sidedefs.len());
    assert_eq!(udmf.map.sectors.len(), binary.map.sectors.len());
    assert_eq!(udmf.map.things.len(), binary.map.things.len());
    // Vertices are >=, not ==: the vanilla node build may append seg-split
    // vertices to VERTEXES, which assembly then reads back.
    assert!(binary.map.vertices.len() >= udmf.map.vertices.len());
}

#[test]
fn a_non_utf8_textmap_is_a_named_error() {
    let (wad, group) = first_group(common::wad_with_textmap(vec![0xFF, 0xFE, 0x00]));
    let err = ingest::load_map(&wad, &group).expect_err("non-UTF-8 must fail");
    assert!(matches!(err, IngestError::NonUtf8Textmap(_)), "got: {err}");
}
