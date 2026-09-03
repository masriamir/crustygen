//! The executable arbiter for the floor constructs: every action the
//! compiler emits must come back from the recognizer as itself — shape,
//! destination, tag — through the same compile → TEXTMAP → parse → Scene
//! path the corpus uses.

use crustygen::check::scene::Scene;
use crustygen::compile::compile;
use crustygen::compile::floors::FloorShape;
use crustygen::compile::textmap::emit_textmap;
use crustygen::ir::Ir;
use crustygen::lift::floor::{Shape, recognize};
use crustygen::tables::Tables;
use crustywad::Limits;
use crustywad::map::udmf::parse_udmf;

const FLOORS: &str = include_str!("golden/floors.json");

#[test]
fn every_compiled_floor_action_round_trips_through_the_recognizer() {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(FLOORS).expect("ir");
    let out = compile(&ir, &tables).expect("compiles");
    let map = parse_udmf(&emit_textmap(&out.data, &out.things), Limits::default()).expect("parses");
    let scene = Scene::build(&map, &tables, &mut Vec::new());
    let r = recognize(&scene, &tables);
    assert_eq!(r.counts.refusals(), 0, "{:?}", r.floors);
    assert_eq!(
        (
            r.counts.targets,
            r.counts.drop_walls,
            r.counts.reveals,
            r.counts.bridges
        ),
        (4, 1, 2, 1),
        "the drop wall, the closet and the pedestal, and the bridge"
    );
    assert_eq!(
        r.counts.shared_tag_accepted, 2,
        "the wall and the pen share the switch's tag"
    );
    assert!(
        r.counts.remote_triggers >= 1,
        "the switch in `west` is remote from the wall"
    );
    assert_eq!(
        r.counts.with_things, 2,
        "the imp inside the closet and the medikit on the pedestal"
    );
    for f in &out.floors {
        let got = r
            .floors
            .iter()
            .find(|x| x.sector == f.sector)
            .expect("every emitted action is recognized");
        assert_eq!(got.destination, Some(f.dest));
        assert_eq!(got.tag, i32::from(f.tag));
        assert_eq!(got.rest, f.rest);
        let expected = match f.shape {
            FloorShape::DropWall => Shape::DropWall,
            FloorShape::Closet | FloorShape::Pedestal => Shape::Reveal,
            FloorShape::Bridge => Shape::Bridge,
        };
        assert_eq!(got.shape, Some(expected), "{f:?}");
    }
}
