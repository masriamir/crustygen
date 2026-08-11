//! Assembles the map-spec parser's stages end to end through
//! `Spec::from_markdown`: the shipped template, a structurally diverse
//! second fixture, and a CRLF-rendered copy of the template. See
//! `docs/map-spec.md` for the format contract these fixtures satisfy.

use crustygen::spec::Spec;
use crustygen::spec::frontmatter::{Boss, Enforcement, Facing, Shape};
use crustygen::tables::Tables;

#[test]
fn the_shipped_template_is_a_valid_spec_document_with_no_sacrifices() {
    let doc = Spec::from_markdown(
        include_str!("../map-spec.template.md"),
        &Tables::load().unwrap(),
    )
    .unwrap();
    assert!(doc.sacrifices.is_empty());
    assert_eq!(
        doc.spec.frontmatter.secrets.count as usize,
        doc.spec.body.secrets.len()
    );
}

#[test]
fn the_strict_gauntlet_fixture_parses_clean() {
    let doc = Spec::from_markdown(
        include_str!("fixtures/gauntlet_strict.spec.md"),
        &Tables::load().unwrap(),
    )
    .unwrap();
    assert!(doc.sacrifices.is_empty());
    assert_eq!(
        doc.spec.frontmatter.constraints.enforcement,
        Enforcement::Strict
    );
    assert_eq!(doc.spec.frontmatter.progression.shape, Shape::Gauntlet);
    assert_eq!(
        doc.spec.frontmatter.players.start_facing,
        Facing::Degrees(135)
    );
    assert_eq!(
        doc.spec.frontmatter.combat.boss,
        Boss::Species("cyberdemon".into())
    );
    assert_eq!(doc.spec.body.secrets.len(), 1);
}

#[test]
fn a_crlf_rendition_of_the_template_parses_identically() {
    let lf = include_str!("../map-spec.template.md");
    let crlf = lf.replace('\n', "\r\n");
    let tables = Tables::load().unwrap();
    let a = Spec::from_markdown(lf, &tables).unwrap();
    let b = Spec::from_markdown(&crlf, &tables).unwrap();
    assert_eq!(a, b);
}
