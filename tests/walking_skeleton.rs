//! Proves that hand-written UDMF `TEXTMAP` text becomes an assemblable map.

use crustywad::map::Map;
use crustywad::{Wad, WadKind, WadBuilder};

/// One 256x256 room, wound clockwise so each front sidedef faces inward.
const TEXTMAP: &str = r#"namespace = "doom";

vertex { x = 0.0; y = 0.0; }
vertex { x = 0.0; y = 256.0; }
vertex { x = 256.0; y = 256.0; }
vertex { x = 256.0; y = 0.0; }

linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }

sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }
sidedef { sector = 0; texturemiddle = "STARTAN3"; }

sector { heightfloor = 0; heightceiling = 128; texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; lightlevel = 160; }

thing { x = 128.0; y = 128.0; angle = 90; type = 1; skill1 = true; skill2 = true; skill3 = true; skill4 = true; skill5 = true; single = true; }
"#;

#[test]
fn textmap_packs_into_an_assemblable_map() {
    let mut builder = WadBuilder::new(WadKind::Pwad);
    builder.add_lump("MAP01", b"");
    builder.add_lump("TEXTMAP", TEXTMAP.as_bytes());
    builder.add_lump("ENDMAP", b"");
    let bytes = builder.build().expect("wad serializes");

    let wad = Wad::from_bytes(bytes).expect("wad parses");
    let group = wad.map_group("MAP01").expect("MAP01 group present");
    let map = Map::assemble(&wad, &group).expect("map assembles");

    assert_eq!(map.sectors().len(), 1, "one sector");
    assert_eq!(map.vertices().len(), 4, "four vertices");
    assert_eq!(map.linedefs().len(), 4, "four linedefs");
    assert_eq!(map.sidedefs().len(), 4, "four sidedefs");
    assert_eq!(map.things().len(), 1, "one thing");
}
