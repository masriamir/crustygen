//! Packs compiled UDMF output into WAD bytes.
//!
//! [`pack_udmf`] wraps a compiled [`Compiled::textmap`] into a minimal,
//! **un-noded** UDMF map group — the `name` marker, `TEXTMAP`, and `ENDMAP`,
//! with no `ZNODES` lump. [`pack_udmf_with_nodes`] produces a second,
//! **noded** artifact from the same compiled output: an engine-playable UDMF
//! map group carrying a built `ZNODES` stream.
//!
//! The two are deliberately not interchangeable. `cwad convert --to doom`
//! refuses in strict mode when the source map group carries a `ZNODES` lump
//! (converting to the classic Doom format has no lump to carry it in, and
//! dropping it is data loss `cwad` reports rather than performs silently) —
//! verified directly against the `cwad` 0.4.7 binary before this module was
//! written. A Doom-format downconvert must therefore start from the *un-noded*
//! artifact [`pack_udmf`] produces, keeping both the UDMF-with-nodes path and
//! the Doom-downconvert path strict. See the crate's map-generation report for
//! the end-to-end pipeline this feeds.
//!
//! Building the noded artifact needs a real [`Map`] — the crustywad node
//! builders operate on an assembled map graph, not on `TEXTMAP` text or this
//! crate's own [`MapData`](crate::compile::MapData) — so
//! [`pack_udmf_with_nodes`] round-trips through the un-noded bytes:
//! [`pack_udmf`] to build them, [`Wad::from_bytes`] to parse them back, and
//! [`Map::assemble`] to get the graph
//! [`add_udmf_map_with_nodes`] needs.

use crustywad::map::build::{
    NodeBuildError, NodeBuildOptions, NodeFormat, add_udmf_map_with_nodes,
};
use crustywad::map::{Map, MapAssembleError};
use crustywad::{ParseError, Wad, WadBuilder, WadKind, WriteError, WriteOptions};

use crate::compile::Compiled;

/// Errors raised while packing compiled output into WAD bytes.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// The WAD's lump directory failed to serialize.
    #[error("failed to serialize WAD: {0}")]
    Write(#[from] WriteError),
    /// The freshly serialized, un-noded WAD failed to parse back — needed by
    /// [`pack_udmf_with_nodes`]'s round trip.
    #[error("failed to re-parse the packed WAD: {0}")]
    Parse(#[from] ParseError),
    /// `map_name` was not found as a map group in the freshly serialized WAD
    /// — unreachable in practice, since [`pack_udmf`] always writes exactly
    /// that marker, but handled rather than unwrapped.
    #[error("map {0:?} not found in the packed WAD")]
    MissingMapGroup(String),
    /// The un-noded map group failed to assemble into a [`Map`].
    #[error("failed to assemble map: {0}")]
    Assemble(#[from] MapAssembleError),
    /// Building or serializing the `ZNODES` stream failed.
    #[error("failed to build node lumps: {0}")]
    NodeBuild(#[from] NodeBuildError),
}

/// Packs a compiled map's `TEXTMAP` into a minimal, **un-noded** UDMF WAD:
/// the `name` marker, `TEXTMAP`, and `ENDMAP`, in that order, with no
/// `ZNODES` lump.
///
/// This is the artifact a Doom-format downconvert (`cwad convert --to doom`)
/// must start from — see the module documentation for why the noded artifact
/// cannot serve that role in strict mode.
///
/// # Errors
/// Returns [`PackError::Write`] if the WAD's lump directory fails to
/// serialize (e.g. a non-ASCII or oversized lump name — unreachable for the
/// fixed, ASCII `map_name`/`TEXTMAP`/`ENDMAP` markers this function itself
/// writes, but not for a `map_name` a caller controls).
pub fn pack_udmf(compiled: &Compiled, map_name: &str) -> Result<Vec<u8>, PackError> {
    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump(map_name, b"");
    builder.add_lump("TEXTMAP", compiled.textmap.as_bytes());
    builder.add_lump("ENDMAP", b"");
    Ok(builder.build()?)
}

/// Packs a compiled map into a **noded**, engine-playable UDMF WAD: the
/// `name` marker, `TEXTMAP`, a built `ZNODES` stream, and `ENDMAP`.
///
/// Builds the GL node family ([`NodeFormat::Gl`], auto-selecting the minimal
/// sufficient dialect) rather than the non-GL extended family, matching
/// `cwad`'s own default for a UDMF `--nodes` build.
///
/// # Errors
/// Returns [`PackError::Write`] or [`PackError::Parse`] if the intermediate
/// un-noded round trip fails to serialize or re-parse,
/// [`PackError::MissingMapGroup`] if `map_name` is not found in the
/// freshly parsed intermediate WAD, [`PackError::Assemble`] if the map group
/// fails to assemble, and [`PackError::NodeBuild`] if building or
/// serializing the `ZNODES` stream fails (e.g. the compiled geometry yields
/// no segs to build a tree from).
pub fn pack_udmf_with_nodes(compiled: &Compiled, map_name: &str) -> Result<Vec<u8>, PackError> {
    // The node builders operate on an assembled `Map`, not on this crate's
    // own `MapData`/`TEXTMAP` text — see the module documentation for why
    // this round-trips through the un-noded bytes rather than building the
    // graph directly.
    let plain = pack_udmf(compiled, map_name)?;
    let wad = Wad::from_bytes(plain)?;
    let group = wad
        .map_group(map_name)
        .ok_or_else(|| PackError::MissingMapGroup(map_name.to_owned()))?;
    let map = Map::assemble(&wad, &group)?;

    let mut builder = WadBuilder::new(WadKind::Pwad);
    let write_opts = WriteOptions::strict();
    let mut build_opts = NodeBuildOptions::strict();
    build_opts.format = NodeFormat::Gl;
    add_udmf_map_with_nodes(&mut builder, map_name, &map, &write_opts, &build_opts)?;
    Ok(builder.build()?)
}

#[cfg(test)]
mod tests {
    use super::{pack_udmf, pack_udmf_with_nodes};
    use crate::compile::compile;
    use crate::ir::Ir;
    use crate::tables::Tables;
    use crustywad::map::Map;
    use crustywad::{Wad, WadKind};

    const TWO_ROOM: &str = include_str!("../tests/golden/two_room.json");

    #[test]
    fn pack_udmf_produces_a_wad_with_no_znodes_lump() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let compiled = compile(&ir, &tables).expect("compiles");

        let bytes = pack_udmf(&compiled, "MAP01").expect("packs");
        let wad = Wad::from_bytes(bytes).expect("parses");
        assert_eq!(wad.kind(), WadKind::Pwad);
        assert!(
            !wad.lumps().iter().any(|l| l.name() == "ZNODES"),
            "the un-noded artifact must carry no ZNODES lump"
        );
        let group = wad.map_group("MAP01").expect("group present");
        let map = Map::assemble(&wad, &group).expect("assembles");
        assert_eq!(map.sectors().len(), 2, "two rooms");
    }

    #[test]
    fn pack_udmf_with_nodes_produces_an_assemblable_wad_carrying_znodes() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let compiled = compile(&ir, &tables).expect("compiles");

        let bytes = pack_udmf_with_nodes(&compiled, "MAP01").expect("packs with nodes");
        let wad = Wad::from_bytes(bytes).expect("parses");
        assert!(
            wad.lumps().iter().any(|l| l.name() == "ZNODES"),
            "the noded artifact carries a ZNODES lump"
        );
        let group = wad.map_group("MAP01").expect("group present");
        let map = Map::assemble(&wad, &group).expect("assembles");
        assert_eq!(map.sectors().len(), 2, "two rooms, same geometry");
    }

    #[test]
    fn the_two_artifacts_carry_the_same_geometry() {
        let ir = Ir::from_json(TWO_ROOM).expect("ir");
        let tables = Tables::load().expect("tables");
        let compiled = compile(&ir, &tables).expect("compiles");

        let plain = pack_udmf(&compiled, "MAP01").expect("packs");
        let noded = pack_udmf_with_nodes(&compiled, "MAP01").expect("packs with nodes");

        let plain_wad = Wad::from_bytes(plain).expect("parses");
        let noded_wad = Wad::from_bytes(noded).expect("parses");
        let plain_map = Map::assemble(&plain_wad, &plain_wad.map_group("MAP01").expect("group"))
            .expect("assembles");
        let noded_map = Map::assemble(&noded_wad, &noded_wad.map_group("MAP01").expect("group"))
            .expect("assembles");

        assert_eq!(plain_map.vertices().len(), noded_map.vertices().len());
        assert_eq!(plain_map.linedefs().len(), noded_map.linedefs().len());
        assert_eq!(plain_map.sectors().len(), noded_map.sectors().len());
        assert_eq!(plain_map.things().len(), noded_map.things().len());
    }
}
