//! Conformance e2e: entrada's emitted TEXTMAP judged against its hand-paired
//! spec (`tests/fixtures/entrada.spec.md`). Every derivable frontmatter
//! number in that spec was hand-set to entrada's own compiled actuals, so a
//! clean run must show zero `Fail` rows — proving `check::run`'s
//! `conform::rows` end to end against a real compiled map, not just the unit
//! fixtures `src/check/conform.rs` already carries.

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

/// Compiles entrada, emits its TEXTMAP, parses it back, and runs
/// [`crustygen::check::run`] against `spec_text` parsed through
/// [`Spec::from_markdown`]. Panics (via `expect`) on any stage failure —
/// entrada and `spec_text` are known-good inputs in every call site here, so
/// a failure is this test's own setup being wrong, not something a caller
/// should have to handle.
fn conformance_rows_for(spec_text: &str) -> Vec<ConformanceRow> {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(ENTRADA).expect("ir parses");
    let compiled = compile(&ir, &tables).expect("compiles");
    let text = emit_textmap(&compiled.data, &compiled.things);
    let map = parse_udmf(&text, Limits::default()).expect("emitted TEXTMAP parses");
    let doc = Spec::from_markdown(spec_text, &tables).expect("spec parses");
    let report = run(&map, "MAP01", &tables, Some(&doc.spec));
    report.conformance.expect("spec supplied")
}

#[test]
fn entrada_conforms_to_its_paired_spec() {
    let rows = conformance_rows_for(ENTRADA_SPEC);

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

    let good_rows = conformance_rows_for(ENTRADA_SPEC);
    let bad_rows = conformance_rows_for(&wrong);
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

    let healthy_rows = conformance_rows_for(ENTRADA_SPEC);
    let broken_params: Vec<&str> = broken_rows.iter().map(|r| r.parameter.as_str()).collect();
    let healthy_params: Vec<&str> = healthy_rows.iter().map(|r| r.parameter.as_str()).collect();
    assert_eq!(
        broken_params, healthy_params,
        "the NotRun row parameter list must equal the healthy run's own list"
    );
}
