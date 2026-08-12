//! Conformance e2e: entrada's emitted TEXTMAP judged against its hand-paired
//! spec (`tests/fixtures/entrada.spec.md`). Every derivable frontmatter
//! number in that spec was hand-set to entrada's own compiled actuals, so a
//! clean run must show zero `Fail` rows — proving `check::run`'s
//! `conform::rows` end to end against a real compiled map, not just the unit
//! fixtures `src/check/conform.rs` already carries.

use crustygen::check::{ConformanceRow, Verdict, run};
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
