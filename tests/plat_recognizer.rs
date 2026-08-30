//! The executable arbiter for the lift construct: every platform the
//! compiler emits must come back from the recognizer as itself — shape,
//! travel, speed and who can call it — through the same compile → TEXTMAP →
//! parse → Scene path the corpus uses.

use crustygen::check::scene::Scene;
use crustygen::compile::compile;
use crustygen::compile::lifts::LiftShape;
use crustygen::compile::textmap::emit_textmap;
use crustygen::ir::Ir;
use crustygen::lift::plat::{Shape, recognize};
use crustygen::tables::Tables;
use crustywad::Limits;
use crustywad::map::udmf::parse_udmf;

const LIFTS: &str = include_str!("golden/lifts.json");

#[test]
fn every_compiled_plat_round_trips_through_the_recognizer() {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(LIFTS).expect("ir");
    let out = compile(&ir, &tables).expect("compiles");
    let map = parse_udmf(&emit_textmap(&out.data, &out.things), Limits::default()).expect("parses");
    let scene = Scene::build(&map, &tables, &mut Vec::new());
    let r = recognize(&scene, &tables);
    assert_eq!(r.counts.refusals(), 0, "{:?}", r.plats);
    assert_eq!(
        (
            r.counts.plats,
            r.counts.lifts,
            r.counts.barriers,
            r.counts.pedestals
        ),
        (4, 2, 1, 1)
    );
    assert_eq!(
        (
            r.counts.fast,
            r.counts.with_top_trigger,
            r.counts.with_things,
            r.counts.callable_low
        ),
        (2, 1, 1, 4),
        "the barrier and the walkover lift are fast; only the both-ends lift's top face is a \
         trigger; only the pedestal holds a thing"
    );
    for lift in &out.lifts {
        let plat = r
            .plats
            .iter()
            .find(|p| p.sector == lift.sector)
            .expect("every emitted plat is recognized");
        assert_eq!(plat.travel, lift.travel);
        let expected = match lift.shape {
            LiftShape::Lift => Shape::Lift,
            LiftShape::Barrier => Shape::Barrier,
            LiftShape::Pedestal => Shape::Pedestal,
        };
        assert_eq!(plat.shape, Some(expected));
    }
    let pedestal = r
        .plats
        .iter()
        .find(|p| p.shape == Some(Shape::Pedestal))
        .expect("the golden's pedestal");
    assert!(pedestal.island, "every edge of a pedestal is two-sided");
    assert_eq!(pedestal.things, 1, "the medikit on the perch");
    assert!(
        r.plats
            .iter()
            .filter(|p| p.shape != Some(Shape::Pedestal))
            .all(|p| !p.island),
        "a lift or a barrier fills a gap cut through two rooms' walls"
    );
}
