//! Layer-4 proof: entrada, broken one property at a time, is caught each
//! time.
//!
//! Each test compiles entrada clean, mutates exactly one property on the
//! *parsed* `UdmfMap` (or, for V-P9, the emitted text before parsing — see
//! that test), and asserts the specific finding [`crustygen::check::run`]
//! raises in response. Every test also re-establishes the 0 baseline for the
//! check id it targets before mutating, so a passing assertion proves the
//! mutation is what broke the property, not that the property was already
//! broken.

use crustygen::check::{Finding, Severity, Subject, run};
use crustygen::compile::textmap::emit_textmap;
use crustygen::compile::{compile, compile_reporting};
use crustygen::ir::Ir;
use crustygen::tables::Tables;
use crustywad::Limits;
use crustywad::map::udmf::{UdmfMap, parse_udmf};

const ENTRADA: &str = include_str!("fixtures/entrada_base.json");
const TELEPORTS: &str = include_str!("golden/teleports.json");
const LIFTS: &str = include_str!("golden/lifts.json");

/// Compiles entrada, emits its TEXTMAP, and parses it back — the same
/// compile -> emit -> parse round trip `tests/check_conformance.rs` uses, so
/// every mutation below starts from the artifact the verifier actually
/// judges, not the IR.
fn entrada_udmf() -> (UdmfMap, Tables) {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(ENTRADA).expect("ir");
    let compiled = compile(&ir, &tables).expect("compiles");
    let text = emit_textmap(&compiled.data, &compiled.things);
    (
        parse_udmf(&text, Limits::default()).expect("parses"),
        tables,
    )
}

/// Compiles the teleport golden fixture (island pad, wall alcove, monsters-
/// only pen — see `tests/golden/teleports.json`), emits its TEXTMAP, and
/// parses it back, the same round trip `entrada_udmf` uses.
fn teleports_udmf() -> (UdmfMap, Tables) {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(TELEPORTS).expect("ir");
    let compiled = compile(&ir, &tables).expect("compiles");
    let text = emit_textmap(&compiled.data, &compiled.things);
    (
        parse_udmf(&text, Limits::default()).expect("parses"),
        tables,
    )
}

/// Compiles the lift golden fixture (switch lift, fast barrier, walkover
/// lift with its alcove, and a pedestal — see `tests/golden/lifts.json`),
/// emits its TEXTMAP, and parses it back, the same round trip
/// `entrada_udmf` uses.
fn lifts_udmf() -> (UdmfMap, Tables) {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(LIFTS).expect("ir");
    let compiled = compile(&ir, &tables).expect("compiles");
    let text = emit_textmap(&compiled.data, &compiled.things);
    (
        parse_udmf(&text, Limits::default()).expect("parses"),
        tables,
    )
}

/// How many findings carry check id `check`.
fn count(findings: &[Finding], check: &str) -> usize {
    findings.iter().filter(|f| f.check == check).count()
}

#[test]
fn pristine_entrada_raises_no_errors() {
    let (map, tables) = entrada_udmf();
    let report = run(&map, "MAP01", &tables, None);
    let errors: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.severity == Severity::Error)
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    // The pristine run is not merely error-free — it is finding-free: no
    // warnings either. Asserted here so a future warning-producing check
    // does not silently start firing on the shipped fixture unnoticed.
    assert!(
        report.findings.is_empty(),
        "expected zero findings of any severity on clean entrada: {:?}",
        report.findings
    );
}

/// V-P8, re-derived from `r_segs.c`: a two-sided line whose sectors' floors
/// differ needs a lower texture on the lower-floor side, or the engine
/// renders a hall-of-mirrors gap where the step riser belongs.
#[test]
fn blanking_a_visible_lower_texture_is_caught_as_p8() {
    let (mut map, tables) = entrada_udmf();
    let baseline = count(&run(&map, "MAP01", &tables, None).findings, "V-P8");
    assert_eq!(baseline, 0, "entrada starts P8-clean");

    // Find a two-sided linedef whose sides' sector floors differ.
    let mut target = None;
    for (i, l) in map.linedefs.iter().enumerate() {
        let Some(back) = l.sideback.and_then(|s| usize::try_from(s).ok()) else {
            continue;
        };
        let front = usize::try_from(l.sidefront).expect("nonnegative sidefront");
        let front_sector = usize::try_from(map.sidedefs[front].sector).expect("valid sector");
        let back_sector = usize::try_from(map.sidedefs[back].sector).expect("valid sector");
        if map.sectors[front_sector].heightfloor != map.sectors[back_sector].heightfloor {
            target = Some((i, front, back, front_sector, back_sector));
            break;
        }
    }
    let (linedef, front, back, front_sector, back_sector) =
        target.expect("entrada has at least one differing-floor two-sided line");

    // Blank the lower-floor side's texturebottom — the side V-P8 requires.
    let lower = if map.sectors[front_sector].heightfloor < map.sectors[back_sector].heightfloor {
        front
    } else {
        back
    };
    assert_ne!(
        map.sidedefs[lower].texturebottom, "-",
        "the lower-floor side must actually carry a texture for blanking it to be a mutation"
    );
    map.sidedefs[lower].texturebottom = "-".to_owned();

    let report = run(&map, "MAP01", &tables, None);
    let p8: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-P8")
        .collect();
    assert_eq!(
        p8.len(),
        1,
        "exactly the one lower texture blanked, no more: {p8:?}"
    );
    assert_eq!(p8[0].severity, Severity::Error);
    assert!(
        matches!(p8[0].subject, Subject::Linedef(l) if l == linedef),
        "expected the finding to name linedef {linedef}: {p8:?}"
    );
    let errors = count(&report.findings, "V-P8");
    assert!(errors > baseline, "0 -> >0 for V-P8");
    assert_eq!(
        report.findings.len(),
        1,
        "no unrelated finding joins it: {:?}",
        report.findings
    );
}

/// V-P14: an action line's `args[0]` (its tag) must be nonzero — tag 0 is
/// the tag every untagged sector already carries, so a stray zero on an
/// action line matches every untagged sector in the map.
#[test]
fn zeroing_an_action_tag_is_caught_as_p14() {
    let (mut map, tables) = entrada_udmf();
    let baseline = count(&run(&map, "MAP01", &tables, None).findings, "V-P14");
    assert_eq!(baseline, 0, "entrada starts P14-clean");

    let linedef = map
        .linedefs
        .iter()
        .position(|l| l.special != 0)
        .expect("entrada has at least one action line");
    map.linedefs[linedef].args[0] = 0;

    let report = run(&map, "MAP01", &tables, None);
    let p14: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-P14")
        .collect();
    assert_eq!(p14.len(), 1, "exactly the one tag zeroed: {p14:?}");
    assert_eq!(p14[0].severity, Severity::Error);
    assert!(
        matches!(p14[0].subject, Subject::Linedef(l) if l == linedef),
        "expected the finding to name linedef {linedef}: {p14:?}"
    );
    assert!(
        count(&report.findings, "V-P14") > baseline,
        "0 -> >0 for V-P14"
    );
    assert_eq!(
        report.findings.len(),
        1,
        "no unrelated finding joins it: {:?}",
        report.findings
    );
}

/// Deleting the only key that opens the `combat` <-> `vault` lock is a
/// double defect: V-P24 (engine form) reports the door's lock as keyless,
/// and V-P7 independently proves the map is now unfinishable — the only
/// exit sits past that lock, so with no key anywhere no walk from the start
/// can ever reach it. The two checks are wired independently in
/// `check::run` (`check_key_lock_coherence` does not consult the flood, and
/// `run_flood` does not consult key placement beyond what the flood itself
/// derives), so this is two checks catching the same authoring mistake from
/// two angles, not one finding implying the other.
#[test]
fn deleting_the_blue_card_makes_the_map_unfinishable_and_orphans_the_lock() {
    let (mut map, tables) = entrada_udmf();
    let baseline_findings = run(&map, "MAP01", &tables, None).findings;
    assert_eq!(
        count(&baseline_findings, "V-P7"),
        0,
        "entrada starts P7-clean"
    );
    assert_eq!(
        count(&baseline_findings, "V-P24"),
        0,
        "entrada starts P24-clean"
    );

    let card_id = i32::from(tables.thing_id("blue_card").expect("blue_card thing id"));
    let before = map.things.len();
    map.things.retain(|t| t.type_id != card_id);
    assert_eq!(
        before - map.things.len(),
        1,
        "entrada places exactly one blue card"
    );

    let report = run(&map, "MAP01", &tables, None);
    assert!(
        report.findings.iter().any(|f| f.check == "V-P7"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Map)
            && f.message.contains("no feasible walk")),
        "expected a V-P7 unfinishable Map error: {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().any(|f| f.check == "V-P24"
            && f.severity == Severity::Error
            && f.message.contains("blue_card")
            && f.message.contains("no such key is placed")),
        "expected a V-P24 keyless-lock error naming blue_card: {:?}",
        report.findings
    );
    assert!(count(&report.findings, "V-P7") > 0, "0 -> >0 for V-P7");
    assert!(count(&report.findings, "V-P24") > 0, "0 -> >0 for V-P24");
}

/// The historical shipped defect (`KNOWN-GAPS.md`): an earlier revision of
/// entrada put `key_room` at floor -32 instead of -16. It compiled clean and
/// passed every test that existed at the time, and was unfinishable — the
/// player drops in, takes the only blue card, and cannot climb the 32 units
/// back out (`P_TryMove` rejects a step over 24). `key_room` is a dead end
/// (its only edge is to `hub`) and the only key for the `combat` <-> `vault`
/// lock guarding the map's only exit, so once no walk can leave `key_room`
/// with the key, no walk can reach the exit either: the whole map reads
/// unfinishable, not merely one stranded room.
///
/// This shapes what `run_flood` actually reports, and it is not the
/// "stranded `(sector, keys)`" shape a pit with an *independent* exit would
/// get (`check::flood`'s own `a_pit_the_player_cannot_climb_out_of_is_stranding`
/// fixture, where the exit sits in the start room, unlocked by the pit).
/// `push_flood_findings` (`src/check/flood.rs`) deliberately suppresses the
/// per-sector `stranded` list once `unfinishable` is set — reporting "every
/// visited state" as its own defect would be noise once the single Map-level
/// headline already says the walk fails — but it still reports every node
/// `unreachable` (never visited at all) independently of that branch.
/// `key_room` itself *is* visited (you can drop into it), so it is named by
/// neither list; what does get named is the sector the flood can no longer
/// reach at all: the exit's own sector, now unreachable because the only key
/// that could open the lock in front of it is permanently trapped upstream.
#[test]
fn the_historical_minus_32_key_room_pit_is_caught_as_stranding() {
    let (mut map, tables) = entrada_udmf();
    let baseline = count(&run(&map, "MAP01", &tables, None).findings, "V-P7");
    assert_eq!(baseline, 0, "entrada starts P7-clean");

    let key_room = map
        .sectors
        .iter()
        .position(|s| s.heightfloor == -16)
        .expect("key_room is the only sector at floor -16");

    // The exit's own sector, resolved the same way `run_flood`'s
    // `resolve_goals` does for a switch exit: the front side of the
    // exit-special boundary.
    let exit_special = i32::from(tables.exit_switch_special());
    let exit_linedef = map
        .linedefs
        .iter()
        .find(|l| l.special == exit_special)
        .expect("entrada has a switch exit");
    let exit_sector = usize::try_from(
        map.sidedefs[usize::try_from(exit_linedef.sidefront).expect("nonnegative sidefront")]
            .sector,
    )
    .expect("valid sector");

    map.sectors[key_room].heightfloor = -32;

    let report = run(&map, "MAP01", &tables, None);

    assert!(
        report.findings.iter().any(|f| f.check == "V-P7"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Map)
            && f.message.contains("no feasible walk")),
        "expected the historical unfinishable headline: {:?}",
        report.findings
    );
    assert!(
        !report
            .findings
            .iter()
            .any(|f| matches!(f.subject, Subject::Sector(s) if s == key_room)),
        "key_room itself is forward-reachable (you can drop in) — it must not be named as \
         unreachable, and its stranded state is suppressed once the whole map is unfinishable: \
         {:?}",
        report.findings
    );
    assert!(
        report.findings.iter().any(|f| f.check == "V-P7"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Sector(s) if s == exit_sector)
            && f.message.contains("never reached")),
        "expected the exit's own sector ({exit_sector}) reported unreachable — the direct \
         consequence of the only key being trapped in key_room ({key_room}): {:?}",
        report.findings
    );
    assert!(
        count(&report.findings, "V-P7") > baseline,
        "0 -> >0 for V-P7"
    );
}

/// The vacuous-pass hole `check::flood`'s own module doc names directly:
/// `reach::graph_from_compiled` returns `None` (silently) for a map with no
/// player 1 start, on the reasoning that IR-level compilation with no start
/// is a spec-conformance concern belonging elsewhere. The verifier has no
/// such elsewhere — a `TEXTMAP` with no start is a hard V-P7 finding here.
#[test]
fn removing_the_player_start_is_a_hard_error_not_a_vacuous_pass() {
    let (mut map, tables) = entrada_udmf();
    let baseline = count(&run(&map, "MAP01", &tables, None).findings, "V-P7");
    assert_eq!(baseline, 0, "entrada starts P7-clean");

    let start_id = i32::from(
        tables
            .thing_id("player1_start")
            .expect("player1_start thing id"),
    );
    assert_eq!(start_id, 1, "player 1 start is DoomEd type 1");
    let before = map.things.len();
    map.things.retain(|t| t.type_id != start_id);
    assert_eq!(
        before - map.things.len(),
        1,
        "entrada places exactly one start"
    );

    let report = run(&map, "MAP01", &tables, None);
    assert!(
        report.findings.iter().any(|f| f.check == "V-P7"
            && f.severity == Severity::Error
            && matches!(f.subject, Subject::Map)
            && f.message.contains("no player 1 start")),
        "expected a V-P7 Map error, not a silent pass: {:?}",
        report.findings
    );
    assert!(
        count(&report.findings, "V-P7") > baseline,
        "0 -> >0 for V-P7"
    );
    assert_eq!(
        report.findings.len(),
        1,
        "no unrelated finding joins it: {:?}",
        report.findings
    );
}

/// V-P9: vanilla Doom's renderer has no per-sidedef texture scaling.
/// `scalex_*`/`scaley_*` have no dedicated field on
/// [`crustywad::map::udmf::UdmfSidedef`] — `emit_textmap` never writes them
/// — so the only way to reintroduce one is to splice the assignment into the
/// TEXTMAP text before parsing, which is also how a ZDoom-aware editor
/// re-saving this file could accidentally introduce one.
/// `UdmfSidedef`/`UdmfAssignment` are `#[non_exhaustive]`, so this cannot be
/// constructed directly outside the crate; splicing the source text and
/// re-parsing stays within the parse-only construction the fixture pattern
/// allows.
#[test]
fn reintroducing_a_scale_factor_is_caught_as_p9() {
    let tables = Tables::load().expect("tables");
    let ir = Ir::from_json(ENTRADA).expect("ir");
    let compiled = compile(&ir, &tables).expect("compiles");
    let text = emit_textmap(&compiled.data, &compiled.things);
    let baseline_map = parse_udmf(&text, Limits::default()).expect("parses");
    let baseline = count(&run(&baseline_map, "MAP01", &tables, None).findings, "V-P9");
    assert_eq!(baseline, 0, "entrada starts P9-clean");

    let modified = text.replacen("sidedef { sector", "sidedef { scalex_mid = 2.0; sector", 1);
    assert_ne!(modified, text, "the splice changed nothing");

    let map = parse_udmf(&modified, Limits::default()).expect("spliced text still parses");
    let report = run(&map, "MAP01", &tables, None);
    let p9: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-P9")
        .collect();
    assert_eq!(p9.len(), 1, "exactly the one scale factor spliced: {p9:?}");
    assert_eq!(p9[0].severity, Severity::Error);
    assert!(
        p9[0].message.contains("scalex_mid"),
        "expected the finding to name the extension: {p9:?}"
    );
    assert!(
        count(&report.findings, "V-P9") > baseline,
        "0 -> >0 for V-P9"
    );
    assert_eq!(
        report.findings.len(),
        1,
        "no unrelated finding joins it: {:?}",
        report.findings
    );
}

/// V-P15, the tag side: retagging one of the island pad's four `special 97`
/// (`WR Teleport`, player, repeatable) edges to a dangling tag makes its own
/// destination unresolvable, while the other three edges — still on the
/// original tag — keep resolving fine. `check_tags` (V-P13) independently
/// flags the same dangling tag on the same linedef (`args[0]` names no
/// sector), so this mutation is deliberately a double hit; only V-P15's
/// count is asserted.
#[test]
fn retagging_a_teleport_line_is_caught_as_v_p15() {
    let (mut map, tables) = teleports_udmf();
    let baseline = count(&run(&map, "MAP01", &tables, None).findings, "V-P15");
    assert_eq!(baseline, 0, "the teleport fixture starts P15-clean");

    let special97 = i32::from(tables.teleport_special(false, true));
    let linedef = map
        .linedefs
        .iter()
        .position(|l| l.special == special97)
        .expect("the fixture has at least one player, repeatable teleport line");
    let dangling_tag = 9;
    assert_ne!(
        map.linedefs[linedef].args[0], dangling_tag,
        "the retag must actually change the tag"
    );
    map.linedefs[linedef].args[0] = dangling_tag;

    let report = run(&map, "MAP01", &tables, None);
    let p15: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-P15")
        .collect();
    assert_eq!(p15.len(), 1, "exactly the one line retagged: {p15:?}");
    assert_eq!(p15[0].severity, Severity::Error);
    assert!(
        matches!(p15[0].subject, Subject::Linedef(l) if l == linedef),
        "expected the finding to name linedef {linedef}: {p15:?}"
    );
    assert!(
        count(&report.findings, "V-P15") > baseline,
        "0 -> >0 for V-P15"
    );
    assert!(
        report.findings.iter().any(|f| f.check == "V-P13"),
        "V-P13 also fires on the same dangling tag, which this test does not gate on: {:?}",
        report.findings
    );
}

/// V-P15, the marker side: deleting the island pad's destination marker
/// (`type 14` at `(448, 192)`, the only `teleport_dest` in the sector the
/// island's four edges tag) leaves that tag resolving to no marker at all,
/// so every one of the island's four edges — not just one — reports broken.
/// The island's four edges are derived rather than hand-indexed: among the
/// fixture's `special 97` (player, repeatable) lines, they are the only tag
/// group with four members (the wall pad's threshold is the other `special
/// 97` line, alone on its own tag).
#[test]
fn removing_the_marker_is_caught_as_v_p15() {
    let (mut map, tables) = teleports_udmf();
    let baseline = count(&run(&map, "MAP01", &tables, None).findings, "V-P15");
    assert_eq!(baseline, 0, "the teleport fixture starts P15-clean");

    let special97 = i32::from(tables.teleport_special(false, true));
    let mut by_tag: std::collections::HashMap<i32, Vec<usize>> = std::collections::HashMap::new();
    for (i, l) in map.linedefs.iter().enumerate() {
        if l.special == special97 {
            by_tag.entry(l.args[0]).or_default().push(i);
        }
    }
    let mut island_edges: Vec<usize> = by_tag
        .values()
        .find(|group| group.len() == 4)
        .expect("the island pad's four edges share one tag")
        .clone();
    island_edges.sort_unstable();
    assert_eq!(island_edges.len(), 4);

    let marker_id = i32::from(
        tables
            .thing_id("teleport_dest")
            .expect("teleport_dest thing id"),
    );
    let before = map.things.len();
    map.things.retain(|t| {
        !(t.type_id == marker_id
            && (t.x - 448.0).abs() < f64::EPSILON
            && (t.y - 192.0).abs() < f64::EPSILON)
    });
    assert_eq!(
        before - map.things.len(),
        1,
        "exactly one marker sits at the island's destination"
    );

    let report = run(&map, "MAP01", &tables, None);
    let mut p15: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-P15")
        .collect();
    assert_eq!(
        p15.len(),
        4,
        "the destination marker is gone, so every island edge is broken: {p15:?}"
    );
    p15.sort_unstable_by_key(|f| match f.subject {
        Subject::Linedef(l) => l,
        _ => usize::MAX,
    });
    let found: Vec<usize> = p15
        .iter()
        .map(|f| match f.subject {
            Subject::Linedef(l) => l,
            other => panic!("expected a Linedef subject, got {other:?}"),
        })
        .collect();
    assert_eq!(
        found, island_edges,
        "expected exactly the island's four edges named: {p15:?}"
    );
    assert!(p15.iter().all(|f| f.severity == Severity::Error));
    assert!(
        count(&report.findings, "V-P15") > baseline,
        "0 -> >0 for V-P15"
    );
}

/// V-P27: a third room with no portal and no teleport destination, holding
/// a monster, is a sealed pen — nothing can ever wake it. Built as a
/// separate IR fixture (spliced onto the golden teleport map's own JSON,
/// after room `b`, so the existing island/wall/pen teleports are exercised
/// alongside it) rather than by mutating the parsed `UdmfMap` directly: a
/// whole new connected sector with its own walls, vertices, and sidedefs is
/// exactly what the compiler builds from a room declaration, and
/// `UdmfSector`/`UdmfLinedef` are `#[non_exhaustive]`, so assembling one by
/// hand outside the crate is not an option. `compile_reporting` is used
/// (not `compile`) because the sealed room is *also* a compile-side P27
/// violation (`src/rules.rs`, covered by task 6) that would otherwise fail
/// the compile outright before a TEXTMAP is ever emitted for the checker to
/// examine.
#[test]
fn sealing_the_pen_is_caught_as_v_p27() {
    let (baseline_map, tables) = teleports_udmf();
    let baseline = count(
        &run(&baseline_map, "MAP01", &tables, None).findings,
        "V-P27",
    );
    assert_eq!(baseline, 0, "the teleport fixture starts P27-clean");

    let room_b_tail = "\"things\": [ { \"kind\": \"imp\", \"at\": [384,64], \"angle\": 0, \
                        \"ambush\": true } ] }\n  ],\n  \"portals\":";
    let sealed_room_c = "\"things\": [ { \"kind\": \"imp\", \"at\": [384,64], \"angle\": 0, \
                          \"ambush\": true } ] },\n    { \"id\": \"c\", \"footprint\": \
                          [[640,0],[640,256],[896,256],[896,0]], \"floor\": 0, \"ceiling\": 128, \
                          \"light\": 160, \"floor_tex\": \"FLOOR4_8\", \"ceil_tex\": \
                          \"CEIL3_5\", \"wall_tex\": \"STARTAN3\", \"things\": [ { \"kind\": \
                          \"imp\", \"at\": [768,128], \"angle\": 0 } ] }\n  ],\n  \"portals\":";
    let sealed_json = TELEPORTS.replacen(room_b_tail, sealed_room_c, 1);
    assert_ne!(sealed_json, TELEPORTS, "the splice changed nothing");

    let ir = Ir::from_json(&sealed_json).expect("ir");
    let (compiled, _violations) =
        compile_reporting(&ir, &tables).expect("geometry compiles despite the P27 violation");
    let text = emit_textmap(&compiled.data, &compiled.things);
    let map = parse_udmf(&text, Limits::default()).expect("parses");

    let report = run(&map, "MAP01", &tables, None);
    let p27: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-P27")
        .collect();
    assert_eq!(
        p27.len(),
        1,
        "only room c's imp is sealed — room b's is still behind its portal: {p27:?}"
    );
    assert_eq!(p27[0].severity, Severity::Error);
    assert!(
        matches!(p27[0].subject, Subject::Sector(_)),
        "expected the finding to name a sector: {p27:?}"
    );
    assert!(
        p27[0].message.contains("nothing can ever wake them"),
        "expected the sealed-room message: {p27:?}"
    );
}

/// The lift golden is the verifier's own cross-examination of Task 5's
/// emission: every lift special is one the checker models, and the flood
/// rides each platform rather than calling the map softlocked.
#[test]
fn the_lift_golden_is_modeled_not_warned_about() {
    let (map, tables) = lifts_udmf();
    let report = run(&map, "MAP01", &tables, None);
    let unmodeled: Vec<_> = report
        .findings
        .iter()
        .filter(|f| f.check == "V-S" && f.message.contains("does not model"))
        .collect();
    assert!(unmodeled.is_empty(), "{unmodeled:?}");
    assert!(
        !report.findings.iter().any(|f| f.check == "V-P7"),
        "{:?}",
        report.findings
    );
}
