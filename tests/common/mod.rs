//! Shared fixture builders for the integration tests: entrada (and
//! ascensor, the lift map, and muralla, the floor map) compiled to a UDMF
//! PWAD, the same map round-tripped into a classic binary-format PWAD (with
//! real nodes, via crustywad's one-shot builder), and a raw TEXTMAP-wrapping
//! helper.

// Each `tests/*.rs` file that does `mod common;` compiles this module into
// its own independent integration-test crate, and no single consumer uses
// every fixture below (e.g. `lift_cli.rs` never calls `wad_with_textmap`;
// `check_cli.rs`/`vanilla_wad.rs` never call
// `binary_entrada_wad_with_broken_second_map`). Per-crate `dead_code` is
// therefore a false positive here, not a real unused-code signal — allow it
// file-wide rather than chasing it fixture-by-fixture as consumers change.
#![allow(dead_code)]

use crustygen::compile::compile;
use crustygen::ir::Ir;
use crustygen::pack::pack_udmf;
use crustygen::tables::Tables;
use crustywad::map::Map;
use crustywad::map::build::{NodeBuildOptions, add_doom_map_with_nodes};
use crustywad::{Wad, WadBuilder, WadKind, WriteOptions};

/// The entrada base IR fixture, shared with `check_cli.rs`.
pub const ENTRADA: &str = include_str!("../fixtures/entrada_base.json");

/// The salto base IR fixture — the teleport playtest map, paired with the
/// committed `maps/salto.wad`.
pub const SALTO: &str = include_str!("../fixtures/salto_base.json");

/// The ascensor base IR fixture — the lift playtest map, paired with the
/// committed `maps/ascensor.wad`.
pub const ASCENSOR: &str = include_str!("../fixtures/ascensor_base.json");

/// The muralla base IR fixture — the floor playtest map, paired with the
/// committed `maps/muralla.wad`.
pub const MURALLA: &str = include_str!("../fixtures/muralla_base.json");

/// The lift golden IR — one lift, one barrier, one walkover lift and a
/// pedestal — for the CLI tests that need a map every platform of which the
/// recognizer accepts.
pub const LIFTS: &str = include_str!("../golden/lifts.json");

/// The floor golden IR — a drop wall and a closet on one switch's shared
/// tag, a pedestal on a walkover, and a bridge on a walkover of its own —
/// for the CLI tests that need a map every floor target of which the
/// recognizer accepts.
pub const FLOORS: &str = include_str!("../golden/floors.json");

/// Compiles `ir_json` and packs it as a minimal un-noded UDMF PWAD.
fn udmf_wad(ir_json: &str) -> Vec<u8> {
    let tables = Tables::load().expect("tables load");
    let ir = Ir::from_json(ir_json).expect("ir parses");
    let compiled = compile(&ir, &tables).expect("fixture compiles");
    pack_udmf(&compiled, "MAP01").expect("packs")
}

/// Assembles a UDMF PWAD and re-emits it as a classic Doom binary-format
/// PWAD with real nodes — the vanilla-input shape the ingest/check/lift
/// tests read. No retail WAD is involved (redistributability).
fn binary_wad(udmf: Vec<u8>) -> Vec<u8> {
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

/// Compiles entrada and packs it as a minimal un-noded UDMF PWAD.
pub fn udmf_entrada_wad() -> Vec<u8> {
    udmf_wad(ENTRADA)
}

/// Compiles entrada, assembles it, and re-emits it as a classic Doom
/// binary-format PWAD with real nodes.
pub fn binary_entrada_wad() -> Vec<u8> {
    binary_wad(udmf_entrada_wad())
}

/// Compiles ascensor — the lift playtest map — and packs it as a minimal
/// un-noded UDMF PWAD.
pub fn udmf_ascensor_wad() -> Vec<u8> {
    udmf_wad(ASCENSOR)
}

/// Compiles ascensor, assembles it, and re-emits it as a classic Doom
/// binary-format PWAD with real nodes — the lift map's vanilla twin.
pub fn binary_ascensor_wad() -> Vec<u8> {
    binary_wad(udmf_ascensor_wad())
}

/// Compiles muralla — the floor playtest map — and packs it as a minimal
/// un-noded UDMF PWAD.
pub fn udmf_muralla_wad() -> Vec<u8> {
    udmf_wad(MURALLA)
}

/// Compiles muralla, assembles it, and re-emits it as a classic Doom
/// binary-format PWAD with real nodes — the floor map's vanilla twin.
pub fn binary_muralla_wad() -> Vec<u8> {
    binary_wad(udmf_muralla_wad())
}

/// Compiles the lift golden and packs it as a minimal un-noded UDMF PWAD.
pub fn udmf_lifts_wad() -> Vec<u8> {
    udmf_wad(LIFTS)
}

/// Compiles the floor golden and packs it as a minimal un-noded UDMF PWAD.
pub fn udmf_floors_wad() -> Vec<u8> {
    udmf_wad(FLOORS)
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

/// CRC-32 (IEEE, reflected) — the zip member checksum crustywad verifies.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A minimal stored-method zip holding `members` (`(path, bytes)`), enough
/// for crustywad's archive reader: local headers, central directory, EOCD.
pub fn stored_zip(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (path, data) in members {
        let offset = u32::try_from(out.len()).expect("fixture fits u32");
        let crc = crc32(data);
        let len = u32::try_from(data.len()).expect("fixture fits u32");
        let name = path.as_bytes();
        let nlen = u16::try_from(name.len()).expect("name fits u16");
        // Local file header.
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&0u16.to_le_bytes()); // method: stored
        out.extend_from_slice(&[0, 0, 0, 0]); // time, date
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&nlen.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(name);
        out.extend_from_slice(data);
        // Central directory entry.
        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&0u16.to_le_bytes()); // flags
        central.extend_from_slice(&0u16.to_le_bytes()); // method
        central.extend_from_slice(&[0, 0, 0, 0]); // time, date
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&len.to_le_bytes());
        central.extend_from_slice(&nlen.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attrs
        central.extend_from_slice(&0u32.to_le_bytes()); // external attrs
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name);
    }
    let cd_offset = u32::try_from(out.len()).expect("fits u32");
    let cd_len = u32::try_from(central.len()).expect("fits u32");
    let n = u16::try_from(members.len()).expect("fits u16");
    out.extend_from_slice(&central);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&[0, 0, 0, 0]); // disk numbers
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&cd_len.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

/// The binary entrada WAD rewritten as a *loadable* Hexen-format map: the
/// same geometry, with `THINGS` and `LINEDEFS` re-encoded into the Hexen
/// record layouts and an empty `BEHAVIOR` lump appended (the lump map-format
/// detection keys on). Unlike
/// [`binary_entrada_wad_with_broken_second_map`], this one assembles
/// cleanly — so it reaches, and is refused by, the ingest path's
/// Doom-format gate.
///
/// Record layouts mirror crustywad 0.9.6 `src/map/hexen.rs`:
/// `Thing` (20 bytes) = `tid u16, x i16, y i16, z i16, angle u16,
/// type_id u16, flags u16, special u8, args [u8; 5]`; `Linedef` (16 bytes) =
/// `start_vertex u16, end_vertex u16, flags u16, special u8, args [u8; 5],
/// right_sidedef u16, left_sidedef u16`. The Doom sources (`src/map/doom/
/// mod.rs`) are `Thing` (10 bytes) = `x i16, y i16, angle u16, type_id u16,
/// flags u16` and `Linedef` (14 bytes) = `start_vertex u16, end_vertex u16,
/// flags u16, special_type u16, sector_tag u16, right_sidedef u16,
/// left_sidedef u16`; the Doom special and tag have no Hexen counterpart and
/// are dropped (Hexen's `special`/`args` are zeroed).
pub fn hexen_entrada_wad() -> Vec<u8> {
    /// Doom `THINGS` (10-byte records) → Hexen `THINGS` (20-byte records).
    fn hexen_things(doom: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(doom.len() * 2);
        for r in doom.as_chunks::<10>().0 {
            out.extend_from_slice(&0u16.to_le_bytes()); // tid
            out.extend_from_slice(&r[0..4]); // x, y
            out.extend_from_slice(&0i16.to_le_bytes()); // z
            out.extend_from_slice(&r[4..10]); // angle, type_id, flags
            out.extend_from_slice(&[0u8; 6]); // special, args
        }
        out
    }

    /// Doom `LINEDEFS` (14-byte records) → Hexen `LINEDEFS` (16-byte records).
    fn hexen_linedefs(doom: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(doom.len() * 2);
        for r in doom.as_chunks::<14>().0 {
            out.extend_from_slice(&r[0..6]); // start, end, flags
            out.extend_from_slice(&[0u8; 6]); // special, args
            out.extend_from_slice(&r[10..14]); // right, left
        }
        out
    }

    let wad = Wad::from_bytes(binary_entrada_wad()).expect("binary fixture parses");
    let lumps = wad.lumps();
    let group = wad
        .map_groups()
        .into_iter()
        .next()
        .expect("binary fixture has a map group");
    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump(group.name.as_str(), Vec::new());
    for &i in &group.data_indices {
        let data = wad.lump_data(&lumps[i]);
        let rewritten = match lumps[i].name() {
            "THINGS" => hexen_things(data),
            "LINEDEFS" => hexen_linedefs(data),
            _ => data.to_vec(),
        };
        builder.add_lump(lumps[i].name(), rewritten);
    }
    builder.add_lump("BEHAVIOR", Vec::new());
    builder.build().expect("wad builds")
}
