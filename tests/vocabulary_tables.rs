//! Table-consistency tests for `[things]` and `[props.*]`.

use std::collections::BTreeMap;

use crustygen::tables::Tables;

#[test]
fn no_doomednum_is_listed_twice() {
    let tables = Tables::load().expect("tables");
    let mut by_id: BTreeMap<u16, Vec<&str>> = BTreeMap::new();
    for (name, id) in tables.thing_kinds() {
        by_id.entry(id).or_default().push(name);
    }
    let dupes: Vec<_> = by_id.iter().filter(|(_, names)| names.len() > 1).collect();
    assert!(dupes.is_empty(), "duplicate doomednums: {dupes:?}");
}

#[test]
fn every_prop_names_a_thing_and_every_thing_resolves() {
    let tables = Tables::load().expect("tables");
    for (name, id) in tables.thing_kinds() {
        assert_eq!(tables.thing_id(name), Some(id));
    }
    let text = include_str!("../data/engine.toml");
    for line in text.lines().filter(|l| l.starts_with("[props.")) {
        let name = line.trim_start_matches("[props.").trim_end_matches(']');
        assert!(
            tables.thing_id(name).is_some(),
            "[props.{name}] has no [things] row"
        );
        assert!(tables.prop(name).is_some());
    }
}

#[test]
fn the_corpus_ranked_decorations_are_all_present() {
    let tables = Tables::load().expect("tables");
    let ids: Vec<u16> = tables.thing_kinds().map(|(_, id)| id).collect();
    for wanted in [46, 54, 43, 44, 57, 41, 26, 45, 48, 47, 25, 63] {
        assert!(
            ids.contains(&wanted),
            "doomednum {wanted} missing from [things]"
        );
    }
}
