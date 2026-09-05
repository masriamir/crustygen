//! Conformance e2e: each committed fixture's emitted TEXTMAP judged against
//! its hand-paired spec — entrada against `tests/fixtures/entrada.spec.md`,
//! salto (the teleport playtest map) against
//! `tests/fixtures/salto.spec.md`, ascensor (the lift playtest map) against
//! `tests/fixtures/ascensor.spec.md`, muralla (the floor playtest map)
//! against `tests/fixtures/muralla.spec.md`. Every derivable frontmatter
//! number in those specs was hand-set to that map's own compiled actuals, so
//! a clean run must show zero `Fail` rows (ascensor's lift-trigger row is the
//! one deliberate exception, documented on its own test) — proving
//! `check::run`'s `conform::rows` end to end against real compiled maps, not
//! just the unit fixtures `src/check/conform.rs` already carries.
//!
//! One test departs from that pattern deliberately: the floor golden
//! (`tests/golden/floors.json`) has no paired spec, and is judged against
//! `map-spec.template.md` for the two rows a floor action moves, whose
//! `actual` halves are derived from geometry alone.

use crustygen::check::{ConformanceRow, Severity, Subject, Verdict, run};
use crustygen::compile::compile;
use crustygen::compile::textmap::emit_textmap;
use crustygen::ir::Ir;
use crustygen::spec::Spec;
use crustygen::tables::Tables;
use crustywad::Limits;
use crustywad::map::udmf::parse_udmf;

const ENTRADA: &str = include_str!("fixtures/entrada_base.json");
const ENTRADA_SPEC: &str = include_str!("fixtures/entrada.spec.md");
const SALTO: &str = include_str!("fixtures/salto_base.json");
const SALTO_SPEC: &str = include_str!("fixtures/salto.spec.md");
const ASCENSOR: &str = include_str!("fixtures/ascensor_base.json");
const ASCENSOR_SPEC: &str = include_str!("fixtures/ascensor.spec.md");
const ASCENSOR_BOTH_ENDS_SPEC: &str = include_str!("fixtures/ascensor_both_ends.spec.md");
const MURALLA: &str = include_str!("fixtures/muralla_base.json");
const MURALLA_SPEC: &str = include_str!("fixtures/muralla.spec.md");
const FLOORS: &str = include_str!("golden/floors.json");
/// The filled, parseable example authors copy — used here only for its
/// *shape*, so the floor golden gets a conformance report without a paired
/// spec of its own.
const SPEC_TEMPLATE: &str = include_str!("../map-spec.template.md");

/// Compiles `ir_json`, emits its TEXTMAP, parses it back, and runs
/// [`crustygen::check::run`] against `spec_text` parsed through
/// [`Spec::from_markdown`]. Panics (via `expect`) on any stage failure —
/// every call site here passes a committed fixture and its paired spec, both
/// known-good, so a failure is this test's own setup being wrong, not
/// something a caller should have to handle.
fn conformance_rows_for(ir_json: &str, spec_text: &str) -> Vec<ConformanceRow> {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(ir_json).expect("ir parses");
    let compiled = compile(&ir, &tables).expect("compiles");
    let text = emit_textmap(&compiled.data, &compiled.things);
    let map = parse_udmf(&text, Limits::default()).expect("emitted TEXTMAP parses");
    let doc = Spec::from_markdown(spec_text, &tables).expect("spec parses");
    let report = run(&map, "MAP01", &tables, Some(&doc.spec));
    report.conformance.expect("spec supplied")
}

#[test]
fn entrada_conforms_to_its_paired_spec() {
    let rows = conformance_rows_for(ENTRADA, ENTRADA_SPEC);

    let failed: Vec<_> = rows.iter().filter(|r| r.verdict == Verdict::Fail).collect();
    assert!(failed.is_empty(), "unexpected Fail rows: {failed:?}");

    // NotRun means a prerequisite check never ran at all — a broken scene,
    // not a legitimate row outcome (`Verdict`'s own doc comment). Treated as
    // a failure here rather than silently allowed alongside Pass/Info/
    // NotDerivable.
    let not_run: Vec<_> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::NotRun)
        .collect();
    assert!(
        not_run.is_empty(),
        "unexpected NotRun rows (broken scene): {not_run:?}"
    );
}

#[test]
fn a_wrong_secret_count_fails_its_row() {
    let wrong = ENTRADA_SPEC.replacen(
        "count: 1                   # per-secret detail lives in the prose body",
        "count: 2                   # per-secret detail lives in the prose body",
        1,
    );
    assert_ne!(wrong, ENTRADA_SPEC, "the patch changed nothing");

    let good_rows = conformance_rows_for(ENTRADA, ENTRADA_SPEC);
    let bad_rows = conformance_rows_for(ENTRADA, &wrong);
    assert_eq!(
        good_rows.len(),
        bad_rows.len(),
        "the patch must not add or remove rows"
    );

    for (good, bad) in good_rows.iter().zip(bad_rows.iter()) {
        assert_eq!(good.parameter, bad.parameter, "row order drifted");
        if good.parameter == "secrets.count" {
            assert_eq!(good.verdict, Verdict::Pass, "got {good:?}");
            assert_eq!(bad.verdict, Verdict::Fail, "got {bad:?}");
        } else {
            assert_eq!(
                (&good.target, &good.actual, good.verdict),
                (&bad.target, &bad.actual, bad.verdict),
                "row `{}` changed unexpectedly",
                good.parameter
            );
        }
    }
}

/// A single-sector box whose one shared perimeter sidedef claims `sector =
/// 9` — the map declares only one sector (index 0) — a dangling
/// sidedef->sector cross-reference `Scene::build` catches as a hard `"V-S"`
/// Error (the same shape `check::scene`'s own
/// `a_dangling_sidedef_sector_index_is_reported_and_the_linedef_is_skipped`
/// pins at the `Scene` layer). Linedef 2 is the one carrying the broken
/// sidedef.
const DANGLING_SIDEDEF_MAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 0.000; y = 128.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 9; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; single = true; }
"#;

/// Failure containment (issue #2's final-review finding #3): a structurally
/// broken scene must never produce a conformance verdict that looks decided.
/// A dangling sidedef->sector index is a hard `"V-S"` `Error`, so
/// `check::run` must swap in `conform::not_run_rows` — every row `NotRun`,
/// and the exact same row *shape* (`parameter` list, in order) the healthy
/// path against the same spec produces, per `not_run_rows`'s own contract of
/// mapping `rows()`'s output rather than re-deriving the catalog.
#[test]
fn a_structurally_broken_scene_marks_every_conformance_row_not_run() {
    let tables = Tables::load().expect("tables");
    let map = parse_udmf(DANGLING_SIDEDEF_MAP, Limits::default()).expect("fixture parses");
    let doc = Spec::from_markdown(ENTRADA_SPEC, &tables).expect("spec parses");

    let report = run(&map, "MAP01", &tables, Some(&doc.spec));

    assert!(
        report.findings.iter().any(|f| f.check == "V-S"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Linedef(2))),
        "expected a V-S error naming the dangling sidedef's linedef (2): {:?}",
        report.findings
    );

    let broken_rows = report.conformance.expect("spec supplied");
    assert!(
        !broken_rows.is_empty(),
        "not_run_rows must still produce the row catalog, not an empty list"
    );
    assert!(
        broken_rows.iter().all(|r| r.verdict == Verdict::NotRun),
        "every row must be NotRun on a structurally broken scene: {broken_rows:?}"
    );
    assert!(
        broken_rows
            .iter()
            .all(|r| r.actual == "scene failed structural validation"),
        "every row's `actual` must name the failure: {broken_rows:?}"
    );

    let healthy_rows = conformance_rows_for(ENTRADA, ENTRADA_SPEC);
    let broken_params: Vec<&str> = broken_rows.iter().map(|r| r.parameter.as_str()).collect();
    let healthy_params: Vec<&str> = healthy_rows.iter().map(|r| r.parameter.as_str()).collect();
    assert_eq!(
        broken_params, healthy_params,
        "the NotRun row parameter list must equal the healthy run's own list"
    );
}

/// A single valid, closed 128x128 sector — geometry entirely clean — holding
/// a `player1_start` (thing 0) and one barrel (thing 1) placed at (5000,
/// 5000), far outside the sector's boundary: a "V-S" Error naming
/// `Subject::Thing(1)` (`Scene::build`'s `resolve_things`), and the *only*
/// `"V-S"` finding this fixture raises — no dangling reference, no unclosed
/// boundary.
const MISPLACED_THING_MAP: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 128.000; }
vertex { x = 0.000; y = 128.000; }
linedef { v1 = 0; v2 = 1; sidefront = 0; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 1; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 2; blocking = true; }
linedef { v1 = 3; v2 = 0; sidefront = 3; blocking = true; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
thing { x = 64.000; y = 64.000; type = 1; single = true; }
thing { x = 5000.000; y = 5000.000; type = 2035; single = true; }
"#;

/// Failure containment must NOT trip on a misplaced-thing "V-S" Error
/// (`Subject::Thing`): that finding names one thing's own bad placement, not
/// a hole in the geometry conformance reads, so it must not force every
/// conformance row to `NotRun` the way a reference-validity or closure
/// failure does (the test above). This is the negative case pinning
/// `check::run`'s narrowed `structurally_broken` predicate: a subject-based
/// filter on `"V-S"` Errors (`Subject::Linedef`/`Subject::Sector` only), not
/// every `"V-S"` Error indiscriminately.
#[test]
fn a_misplaced_thing_v_s_error_does_not_trigger_containment() {
    let tables = Tables::load().expect("tables");
    let map = parse_udmf(MISPLACED_THING_MAP, Limits::default()).expect("fixture parses");
    let doc = Spec::from_markdown(ENTRADA_SPEC, &tables).expect("spec parses");

    let report = run(&map, "MAP01", &tables, Some(&doc.spec));

    assert!(
        report.findings.iter().any(|f| f.check == "V-S"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Thing(1))),
        "expected a V-S error naming the misplaced thing (1): {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().all(|f| !(f.check == "V-S"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Linedef(_) | Subject::Sector(_)))),
        "this fixture's geometry must stay entirely clean — only the thing is misplaced: {:?}",
        report.findings
    );

    let rows = report.conformance.expect("spec supplied");
    assert!(
        rows.iter().all(|r| r.verdict != Verdict::NotRun),
        "a misplaced-thing V-S error must not trip containment: no row should be NotRun: \
         {rows:?}"
    );
}

/// Salto's own conformance run: the teleport playtest map judged against
/// `tests/fixtures/salto.spec.md`, the same hand-paired shape entrada has.
/// Zero `Fail` rows, plus the four rows the teleport toolchain actually
/// produces — the teleport exit trigger, the player-crossable pad count, the
/// monsters-only ambush pad, and the deaf ratio the closet's three imps give
/// it — named explicitly so a regression in any one of them fails here by
/// name rather than only inside the blanket "no Fail rows" assertion.
#[test]
fn salto_conforms_to_its_paired_spec_including_the_four_teleport_rows() {
    let rows = conformance_rows_for(SALTO, SALTO_SPEC);

    let failed: Vec<_> = rows.iter().filter(|r| r.verdict == Verdict::Fail).collect();
    assert!(failed.is_empty(), "unexpected Fail rows: {failed:?}");

    let not_run: Vec<_> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::NotRun)
        .collect();
    assert!(
        not_run.is_empty(),
        "unexpected NotRun rows (broken scene): {not_run:?}"
    );

    for parameter in [
        "progression.exit.trigger",
        "progression.teleports.count",
        "combat.ambush.teleport_ambushes",
    ] {
        let row = rows
            .iter()
            .find(|r| r.parameter == parameter)
            .expect(parameter);
        assert_eq!(row.verdict, Verdict::Pass, "{row:?}");
    }

    // `deaf_ratio` is an `info_row`, so its verdict is `Info` by
    // construction and can never be `Pass` — the measurement itself is the
    // assertion: all three imps carry the ambush flag.
    let deaf = rows
        .iter()
        .find(|r| r.parameter == "combat.ambush.deaf_ratio")
        .expect("deaf ratio");
    assert_eq!(deaf.verdict, Verdict::Info, "{deaf:?}");
    assert!(deaf.actual.starts_with("1.000"), "{deaf:?}");
}

/// Ascensor's own conformance run: the lift playtest map judged against
/// `tests/fixtures/ascensor.spec.md`. Every derivable number in that spec
/// was set from ascensor's own compiled output, with one deliberate
/// exception — `progression.lifts.trigger`, which asks for `switch` while
/// the map mixes all three trigger shapes, and so must Fail naming what the
/// map actually carries. Every other row is `Pass`, `Info` or
/// `NotDerivable`, and no row is `NotRun`.
#[test]
fn ascensor_conforms_to_its_paired_spec_except_the_lift_trigger_row() {
    let rows = conformance_rows_for(ASCENSOR, ASCENSOR_SPEC);

    let failed: Vec<_> = rows.iter().filter(|r| r.verdict == Verdict::Fail).collect();
    let failed_params: Vec<&str> = failed.iter().map(|r| r.parameter.as_str()).collect();
    assert_eq!(
        failed_params,
        vec!["progression.lifts.trigger"],
        "the trigger row is the only expected Fail: {failed:?}"
    );
    assert_eq!(
        failed[0].actual, "switch ×1, walkover ×1, both_ends ×1",
        "{:?}",
        failed[0]
    );

    let not_run: Vec<_> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::NotRun)
        .collect();
    assert!(
        not_run.is_empty(),
        "unexpected NotRun rows (broken scene): {not_run:?}"
    );

    // The lift rows the platform toolchain actually produces, named so a
    // regression in any one of them fails here by name.
    for parameter in [
        "progression.lifts.count",
        "progression.lifts.max_travel",
        "progression.exit.trigger",
        "progression.switches.count",
    ] {
        let row = rows
            .iter()
            .find(|r| r.parameter == parameter)
            .expect(parameter);
        assert_eq!(row.verdict, Verdict::Pass, "{row:?}");
    }
}

/// The same map against `tests/fixtures/ascensor_both_ends.spec.md` — the
/// identical document asking for `both_ends` instead — showing the trigger
/// row's *other* failure text: the actual is the map's own trigger mix
/// either way, and only the target moves. (The row's Pass case is a unit
/// fixture in `src/check/conform.rs`.)
#[test]
fn the_lift_trigger_row_names_the_same_mix_whichever_trigger_the_spec_asks_for() {
    let asked_switch = conformance_rows_for(ASCENSOR, ASCENSOR_SPEC);
    let asked_both_ends = conformance_rows_for(ASCENSOR, ASCENSOR_BOTH_ENDS_SPEC);
    assert_eq!(
        asked_switch.len(),
        asked_both_ends.len(),
        "the variant must not add or remove rows"
    );

    for (switch, both_ends) in asked_switch.iter().zip(asked_both_ends.iter()) {
        assert_eq!(switch.parameter, both_ends.parameter, "row order drifted");
        if switch.parameter == "progression.lifts.trigger" {
            assert_eq!(switch.target, "switch", "{switch:?}");
            assert_eq!(both_ends.target, "both_ends", "{both_ends:?}");
            assert_eq!(switch.actual, both_ends.actual, "the actual must not move");
            assert_eq!(switch.verdict, Verdict::Fail, "{switch:?}");
            assert_eq!(both_ends.verdict, Verdict::Fail, "{both_ends:?}");
        } else {
            assert_eq!(
                (&switch.target, &switch.actual, switch.verdict),
                (&both_ends.target, &both_ends.actual, both_ends.verdict),
                "row `{}` changed unexpectedly",
                switch.parameter
            );
        }
    }
}

/// Muralla's own conformance run: the floor playtest map judged against
/// `tests/fixtures/muralla.spec.md`. Every derivable number in that spec was
/// set from muralla's own compiled output, and unlike ascensor none is left
/// deliberately failing — every row is `Pass`, `Info` or `NotDerivable`, and
/// no row is `NotRun`.
///
/// The rows the floor toolchain itself produces are named so a regression in
/// one fails here by name: the shape census (`progression.floors`, an `Info`
/// row whose `actual` is the whole point — one of each of the three actions,
/// none refused), the closet the drop wall seals, the three walkover lines
/// (the reveal's, plus both of the bridge's thresholds) and the two switches
/// (the wall's and the exit's).
#[test]
fn muralla_conforms_to_its_paired_spec() {
    let rows = conformance_rows_for(MURALLA, MURALLA_SPEC);

    let failed: Vec<_> = rows.iter().filter(|r| r.verdict == Verdict::Fail).collect();
    assert!(failed.is_empty(), "unexpected Fail rows: {failed:?}");

    let not_run: Vec<_> = rows
        .iter()
        .filter(|r| r.verdict == Verdict::NotRun)
        .collect();
    assert!(
        not_run.is_empty(),
        "unexpected NotRun rows (broken scene): {not_run:?}"
    );

    let floors = rows
        .iter()
        .find(|r| r.parameter == "progression.floors")
        .expect("the floor census row is always emitted");
    assert_eq!(floors.verdict, Verdict::Info, "{floors:?}");
    assert_eq!(
        floors.actual, "drop walls ×1, reveals ×1, bridges ×1, refused ×0",
        "{floors:?}"
    );

    for parameter in [
        "combat.monster_closets",
        "progression.walkover_triggers.count",
        "progression.switches.count",
        "progression.keys",
        "progression.locked_doors",
        "progression.exit.trigger",
    ] {
        let row = rows
            .iter()
            .find(|r| r.parameter == parameter)
            .expect(parameter);
        assert_eq!(row.verdict, Verdict::Pass, "{row:?}");
    }
}

/// The floor golden's own conformance rows, over the two parameters a floor
/// action moves — read against `map-spec.template.md` rather than a paired
/// spec of its own, because both rows' `actual` halves are derived from the
/// map's geometry and owe nothing to the spec beside them. Every other row
/// here is measuring the golden against a template written for a much larger
/// map and is deliberately not read.
///
/// **`combat.monster_closets` is 1: the drop wall.** It qualifies the way
/// `conform::floor_closets` says a drop wall does — the region behind it is
/// closed and holds a monster. `east` and `far` are joined only to each
/// other (across the bridge) and to the rest of the map through the wall,
/// and `east` holds an imp; so the walk out from the wall's far side finds a
/// sealed pocket with a monster in it.
///
/// The rule's **reveal** half — a reveal whose own cell holds a monster —
/// is not exercised here and cannot be exercised by a closet at all any
/// more: ruling R28 refuses a closet that holds anything, since the engine
/// never lowers a floor a shootable thing does not fit in. It stays in the
/// rule because a *pedestal* reveal can still hold a monster (it has real
/// headroom above its risen floor), and because the recognizer classifies
/// foreign-WAD reveals this compiler did not emit.
#[test]
fn the_floor_golden_reports_its_one_monster_closet_and_its_shape_census() {
    let rows = conformance_rows_for(FLOORS, SPEC_TEMPLATE);

    let closets = rows
        .iter()
        .find(|r| r.parameter == "combat.monster_closets")
        .expect("the closet row is always emitted");
    assert_eq!(
        closets.actual, "1",
        "the drop wall's sealed region; the `pen` closet is empty: {closets:?}"
    );

    let floors = rows
        .iter()
        .find(|r| r.parameter == "progression.floors")
        .expect("the floor census row is always emitted");
    assert_eq!(
        floors.actual, "drop walls ×1, reveals ×2, bridges ×1, refused ×0",
        "{floors:?}"
    );
}
