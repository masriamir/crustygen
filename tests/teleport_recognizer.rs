//! The executable arbiter for the teleport construct: every teleport the
//! compiler emits must come back from the recognizer as itself — kind,
//! repeatability, geometry, destination, pairing — through the same
//! compile → TEXTMAP → parse → Scene path the corpus uses.

use crustygen::check::scene::Scene;
use crustygen::compile::compile;
use crustygen::compile::textmap::emit_textmap;
use crustygen::ir::Ir;
use crustygen::lift::teleport::{Geometry, TeleportKind, recognize};
use crustygen::tables::Tables;
use crustywad::Limits;
use crustywad::map::udmf::parse_udmf;

const TELEPORTS: &str = include_str!("golden/teleports.json");

#[test]
fn every_compiled_teleport_round_trips_through_the_recognizer() {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(TELEPORTS).expect("ir");
    let out = compile(&ir, &tables).expect("compiles");
    let map = parse_udmf(&emit_textmap(&out.data, &out.things), Limits::default()).expect("parses");
    let scene = Scene::build(&map, &tables, &mut Vec::new());
    let r = recognize(&scene, &tables);
    assert_eq!(r.counts.refusals(), 0, "{:?}", r.lines);
    assert_eq!(
        r.counts.lines, 9,
        "4 island + 4 pen edges + 1 wall threshold"
    );
    assert_eq!(
        (
            r.counts.island,
            r.counts.alcove,
            r.counts.boundary,
            r.counts.other
        ),
        (8, 1, 0, 0)
    );
    assert_eq!(
        (r.counts.player, r.counts.monsters_only, r.counts.one_shot),
        (5, 4, 0)
    );
    assert_eq!(
        r.counts.closet, 5,
        "the wall pad's threshold plus the pen's four edges front room b, which holds the imp"
    );
    assert_eq!(
        r.counts.paired, 4,
        "the wall pad delivers onto the island pad, which therefore holds a marker"
    );
    assert_eq!(
        r.counts.exit, 4,
        "the pen's four edges deliver into room a, which hosts the switch exit"
    );
    assert!(
        r.lines
            .iter()
            .all(|l| l.destination.is_some() && !l.ambiguous)
    );
    let kinds: Vec<TeleportKind> = r
        .lines
        .iter()
        .filter(|l| l.geometry == Geometry::Alcove)
        .map(|l| l.kind)
        .collect();
    assert_eq!(kinds, vec![TeleportKind::Player]);
}
