//! The lifter's telemetry-first skeleton (crustyllm program, `docs/lift.md`).
//!
//! The lifter's charter is *recognition, not approximation*: it will emit
//! map-spec constructs only where it can prove the geometry means them, and
//! it measures everything it cannot express. [`survey`] reduces a parsed map
//! to a [`MapTelemetry`] census — raw element counts and raw value histograms
//! — and interprets **nothing**: no table lookups, no engine constants, no
//! vocabulary judgments. [`vocabulary`] is the first interpreting layer: a
//! membership test against the compiler's emittable sets, and only an upper
//! bound on what a geometry-aware lifter could express. [`corpus`] sweeps a
//! directory of zips and WADs, surveying and classifying every map it finds.
//! Recognizers proper — the ones that reason about geometry — arrive later.

use std::collections::BTreeMap;

use crustywad::map::udmf::UdmfMap;

pub mod corpus;
pub mod vocabulary;

/// Raw element counts for one map.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Census {
    /// Number of `vertex` blocks.
    pub vertices: usize,
    /// Number of `linedef` blocks.
    pub linedefs: usize,
    /// Number of `sidedef` blocks.
    pub sidedefs: usize,
    /// Number of `sector` blocks.
    pub sectors: usize,
    /// Number of `thing` blocks.
    pub things: usize,
}

/// The telemetry record [`survey`] produces for one map.
///
/// Histogram keys are the raw values the map carries. Linedef and sector
/// specials of 0 are omitted — 0 is the UDMF default meaning "no special"
/// (crustywad `map::udmf::model` documents the default); the census carries
/// the totals. Thing types have no such null value and are all counted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MapTelemetry {
    /// The map (group) name this record describes.
    pub map: String,
    /// Raw element counts.
    pub census: Census,
    /// Non-zero `linedef.special` value → occurrence count.
    pub linedef_specials: BTreeMap<i32, u64>,
    /// Non-zero `sector.special` value → occurrence count.
    pub sector_specials: BTreeMap<i32, u64>,
    /// `thing.type` value → occurrence count.
    pub thing_types: BTreeMap<i32, u64>,
}

/// Reduces `map` to its raw census and histograms. Interprets nothing.
#[must_use]
pub fn survey(map_name: &str, map: &UdmfMap) -> MapTelemetry {
    let mut linedef_specials = BTreeMap::new();
    for linedef in &map.linedefs {
        if linedef.special != 0 {
            *linedef_specials.entry(linedef.special).or_insert(0u64) += 1;
        }
    }
    let mut sector_specials = BTreeMap::new();
    for sector in &map.sectors {
        if sector.special != 0 {
            *sector_specials.entry(sector.special).or_insert(0u64) += 1;
        }
    }
    let mut thing_types = BTreeMap::new();
    for thing in &map.things {
        *thing_types.entry(thing.type_id).or_insert(0u64) += 1;
    }
    MapTelemetry {
        map: map_name.to_owned(),
        census: Census {
            vertices: map.vertices.len(),
            linedefs: map.linedefs.len(),
            sidedefs: map.sidedefs.len(),
            sectors: map.sectors.len(),
            things: map.things.len(),
        },
        linedef_specials,
        sector_specials,
        thing_types,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crustywad::Limits;
    use crustywad::map::udmf::parse_udmf;

    /// A square room: 4 vertices, 4 linedefs (one carrying special 1), 4
    /// sidedefs, 1 sector (special 9), 2 things. The special/type values
    /// are arbitrary raw numbers — telemetry interprets nothing.
    const SQUARE: &str = r#"namespace = "doom";
vertex { x = 0; y = 0; }
vertex { x = 128; y = 0; }
vertex { x = 128; y = 128; }
vertex { x = 0; y = 128; }
linedef { v1 = 0; v2 = 1; sidefront = 0; }
linedef { v1 = 1; v2 = 2; sidefront = 1; special = 1; }
linedef { v1 = 2; v2 = 3; sidefront = 2; }
linedef { v1 = 3; v2 = 0; sidefront = 3; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; special = 9; }
thing { x = 64; y = 64; type = 1; }
thing { x = 80; y = 64; type = 2035; }
"#;

    #[test]
    fn survey_counts_and_histograms_the_square() {
        let map = parse_udmf(SQUARE, Limits::default()).expect("fixture parses");
        let t = survey("MAP01", &map);
        assert_eq!(t.map, "MAP01");
        assert_eq!(
            t.census,
            Census {
                vertices: 4,
                linedefs: 4,
                sidedefs: 4,
                sectors: 1,
                things: 2
            }
        );
        assert_eq!(t.linedef_specials.get(&1), Some(&1));
        assert_eq!(
            t.linedef_specials.len(),
            1,
            "special 0 (none) is not counted"
        );
        assert_eq!(t.sector_specials.get(&9), Some(&1));
        assert_eq!(t.thing_types.get(&1), Some(&1));
        assert_eq!(t.thing_types.get(&2035), Some(&1));
        assert_eq!(t.thing_types.len(), 2);
    }

    #[test]
    fn telemetry_serializes_to_json_with_string_keys() {
        let map = parse_udmf(SQUARE, Limits::default()).expect("fixture parses");
        let json = serde_json::to_value(survey("MAP01", &map)).expect("serializes");
        assert_eq!(json["census"]["sectors"], 1);
        assert_eq!(json["thing_types"]["2035"], 1);
    }
}
