//! Pass 1 — the lift census: special usage, tag resolution, rest position and
//! travel, topology, size, triggers, rendering-facing properties, conflicts,
//! the floor-special "up lift" pairing, and the expressibility arbiter.

use std::collections::BTreeSet;

use crustygen::check::scene::Scene;
use crustygen::lift::{self, vocabulary::Vocabulary};
use crustygen::tables::Tables;
use crustywad::map::udmf::UdmfMap;

use crate::common::{
    self, Activator, BLAZE, DWUS, Hist, LOWER, PERPETUAL, Placement, PlatFacts, RAISE_CHANGE,
    RAISE_NEAREST, REPEATABLE_LIFT, Rest, Shape, USE_LIFT, is_lift, pct, percentiles,
};

#[derive(Default)]
struct Agg {
    maps: u64,
    maps_with_lift: u64,
    maps_with_blaze: u64,
    special_lines: Hist,
    special_maps: Hist,
    lines_tag_resolves: Hist,
    plats: Vec<PlatFacts>,
    switch_tex: Hist,
    switch_slot: Hist,
    switch_sw_prefix: u64,
    switch_lines: u64,
    sectors_raise_only: u64,
    sectors_lower_only: u64,
    sectors_raise_and_lower: u64,
    sectors_raise_rest_low: u64,
    maps_raise_and_lower: u64,
    // naïve arbiter
    expr_now: u64,
    expr_with_dwus: u64,
    expr_with_dwus_blaze: u64,
    line_ok_now: u64,
    line_ok_with_dwus: u64,
    line_ok_with_dwus_blaze: u64,
    // shape-gated arbiter
    maps_lift_refused: u64,
    maps_all_core: u64,
    maps_all_ext: u64,
    line_core: u64,
    line_ext: u64,
    expr_core: u64,
    expr_ext: u64,
}

/// Runs the census over `dirs` and prints the report for `label`.
pub(crate) fn run(label: &str, dirs: &[String]) {
    let tables = Tables::load().expect("tables");
    let vocab = Vocabulary::from_tables(&tables);
    let step = tables.step_height();
    let mut agg = Agg::default();
    agg.maps = common::sweep(dirs, |name, map| {
        survey_map(name, map, &tables, &vocab, step, &mut agg);
    });
    report(label, &agg, step);
}

fn survey_map(
    name: &str,
    map: &UdmfMap,
    tables: &Tables,
    vocab: &Vocabulary,
    step: i32,
    agg: &mut Agg,
) {
    let telemetry = lift::survey(name, map);
    let mut verdict = vocab.classify(&telemetry);
    let scene = Scene::build(map, tables, &mut Vec::new());
    let has_teleports = tables
        .teleport_specials()
        .into_iter()
        .any(|s| telemetry.linedef_specials.contains_key(&i32::from(s)));
    if has_teleports {
        let report = lift::teleport::recognize(&scene, tables);
        verdict = verdict.with_teleports(&report);
    }

    // Naïve arbiter: every repeatable lift line accepted — first 62/88, then
    // 120/123 as well. The one-shot forms (21/10/122/121) stay unknown, as the
    // design keeps them out of the emittable set; a map carrying one still
    // fails the line axis here.
    let others_ok = verdict.sector_specials_ok && verdict.thing_kinds_ok && verdict.teleports_ok;
    let unknown: BTreeSet<i32> = verdict.unknown_line_specials.iter().copied().collect();
    let dwus_set: BTreeSet<i32> = [62, 88].into_iter().collect();
    let dwus_blaze_set: BTreeSet<i32> = [62, 88, 120, 123].into_iter().collect();
    let line_dwus = unknown.is_subset(&dwus_set);
    let line_dwus_blaze = unknown.is_subset(&dwus_blaze_set);
    agg.line_ok_now += u64::from(unknown.is_empty());
    agg.line_ok_with_dwus += u64::from(line_dwus);
    agg.line_ok_with_dwus_blaze += u64::from(line_dwus_blaze);
    agg.expr_now += u64::from(verdict.expressible);
    agg.expr_with_dwus += u64::from(others_ok && line_dwus);
    agg.expr_with_dwus_blaze += u64::from(others_ok && line_dwus_blaze);

    survey_specials(map, agg);
    let index = common::MapIndex::build(map, &scene);
    survey_floor_pairing(map, &scene, step, agg);

    let any_lift = map.linedefs.iter().any(|l| is_lift(l.special));
    if !any_lift {
        // No lift line: the shape-gated verdicts equal the +62/88/120/123 ones.
        agg.line_core += u64::from(line_dwus_blaze);
        agg.line_ext += u64::from(line_dwus_blaze);
        agg.expr_core += u64::from(others_ok && line_dwus_blaze);
        agg.expr_ext += u64::from(others_ok && line_dwus_blaze);
        return;
    }
    agg.maps_with_lift += 1;
    if map.linedefs.iter().any(|l| BLAZE.contains(&l.special)) {
        agg.maps_with_blaze += 1;
    }

    let lift_refused = survey_lift_lines(map, &index, agg);
    let mut shapes = Vec::new();
    for plat in index.plat_sectors(map) {
        if let Some(facts) = common::analyze_plat(map, &scene, &index, plat, step) {
            shapes.push(facts.shape);
            agg.plats.push(facts);
        }
    }

    // Shape-gated arbiter: every plat accepted and no refused line.
    if lift_refused {
        agg.maps_lift_refused += 1;
    }
    let all_core = !lift_refused && shapes.iter().all(|s| *s == Shape::Core);
    let all_ext = !lift_refused && shapes.iter().all(|s| *s != Shape::Other);
    agg.maps_all_core += u64::from(all_core);
    agg.maps_all_ext += u64::from(all_ext);
    agg.line_core += u64::from(line_dwus_blaze && all_core);
    agg.line_ext += u64::from(line_dwus_blaze && all_ext);
    agg.expr_core += u64::from(others_ok && line_dwus_blaze && all_core);
    agg.expr_ext += u64::from(others_ok && line_dwus_blaze && all_ext);
}

fn tracked(special: i32) -> bool {
    is_lift(special)
        || PERPETUAL.contains(&special)
        || RAISE_CHANGE.contains(&special)
        || RAISE_NEAREST.contains(&special)
        || LOWER.contains(&special)
}

fn survey_specials(map: &UdmfMap, agg: &mut Agg) {
    let mut in_map: BTreeSet<i32> = BTreeSet::new();
    for l in &map.linedefs {
        if tracked(l.special) {
            agg.special_lines.add(l.special.to_string());
            in_map.insert(l.special);
        }
    }
    for s in in_map {
        agg.special_maps.add(s.to_string());
    }
}

/// Tagged sectors targeted by floor-raise and floor-lower specials — the
/// "rests low and rises" mechanism, which is not a plat.
fn survey_floor_pairing(map: &UdmfMap, scene: &Scene, step: i32, agg: &mut Agg) {
    let mut raise_tags: BTreeSet<i32> = BTreeSet::new();
    let mut lower_tags: BTreeSet<i32> = BTreeSet::new();
    for l in &map.linedefs {
        if l.args[0] == 0 {
            continue;
        }
        if RAISE_NEAREST.contains(&l.special) || RAISE_CHANGE.contains(&l.special) {
            raise_tags.insert(l.args[0]);
        }
        if LOWER.contains(&l.special) {
            lower_tags.insert(l.args[0]);
        }
    }
    let mut map_has_pair = false;
    for (i, sec) in map.sectors.iter().enumerate() {
        if sec.id == 0 {
            continue;
        }
        match (raise_tags.contains(&sec.id), lower_tags.contains(&sec.id)) {
            (true, true) => {
                agg.sectors_raise_and_lower += 1;
                map_has_pair = true;
            }
            (true, false) => {
                agg.sectors_raise_only += 1;
                let ss = &scene.sectors[i];
                let rests_low = ss
                    .boundary
                    .iter()
                    .filter_map(|b| b.neighbor)
                    .any(|n| scene.sectors[n].floor > ss.floor + step);
                agg.sectors_raise_rest_low += u64::from(rests_low);
            }
            (false, true) => agg.sectors_lower_only += 1,
            (false, false) => {}
        }
    }
    agg.maps_raise_and_lower += u64::from(map_has_pair);
}

/// Tag resolution and switch textures per lift line. Returns whether any
/// line is refused (tag 0, or a tag naming no sector).
fn survey_lift_lines(map: &UdmfMap, index: &common::MapIndex<'_>, agg: &mut Agg) -> bool {
    let mut refused = false;
    for l in &map.linedefs {
        if !is_lift(l.special) {
            continue;
        }
        let tag = l.args[0];
        if tag == 0 {
            // A tag-0 lift line is refused outright and is not a switch the
            // texture statistic should describe.
            agg.lines_tag_resolves.add("tag0");
            refused = true;
            continue;
        }
        match index.by_tag.get(&tag).map_or(0, Vec::len) {
            0 => {
                agg.lines_tag_resolves.add("0");
                refused = true;
            }
            1 => agg.lines_tag_resolves.add("1"),
            _ => agg.lines_tag_resolves.add("N"),
        }
        if USE_LIFT.contains(&l.special) {
            agg.switch_lines += 1;
            // A dangling front side still counts as a switch line; it just has
            // no texture to inspect.
            let Some(sd) = common::sidedef(map, l.sidefront) else {
                continue;
            };
            for (slot, tex) in [
                ("top", &sd.texturetop),
                ("mid", &sd.texturemiddle),
                ("bot", &sd.texturebottom),
            ] {
                if tex.starts_with("SW1") || tex.starts_with("SW2") {
                    agg.switch_slot.add(slot);
                    agg.switch_tex.add(tex.clone());
                    agg.switch_sw_prefix += 1;
                }
            }
        }
    }
    refused
}

fn family(special: i32) -> &'static str {
    if DWUS.contains(&special) {
        "DWUS"
    } else if BLAZE.contains(&special) {
        "blaze DWUS"
    } else if PERPETUAL.contains(&special) {
        "perpetual/stop"
    } else if RAISE_CHANGE.contains(&special) {
        "raise&change (plat)"
    } else if RAISE_NEAREST.contains(&special) {
        "floor raise-to-nearest"
    } else {
        "floor lower"
    }
}

fn count_len(n: usize) -> u64 {
    u64::try_from(n).expect("fits u64")
}

fn report(label: &str, agg: &Agg, step: i32) {
    let plats = &agg.plats;
    let np = count_len(plats.len());
    let moving: Vec<&PlatFacts> = plats.iter().filter(|p| p.moving()).collect();
    let nmv = count_len(moving.len());

    println!("# liftprobe census — {label}\n");
    println!(
        "Maps (unique, loaded): **{}** · with ≥1 DWUS/blaze lift line: **{}** ({}) · with ≥1 blaze line: {} ({})\n",
        agg.maps,
        agg.maps_with_lift,
        pct(agg.maps_with_lift, agg.maps),
        agg.maps_with_blaze,
        pct(agg.maps_with_blaze, agg.maps)
    );

    report_specials(agg);
    report_tags(agg, np);
    report_rest(plats, np, nmv, step);
    report_topology(&moving, nmv);
    report_size(&moving, nmv);
    report_triggers(agg, &moving, nmv);
    report_rendering(&moving, nmv);
    report_conflicts(&moving, nmv);
    report_floor_pairing(agg);
    report_arbiter(agg, &moving, nmv);
}

fn report_specials(agg: &Agg) {
    println!("## A. Special usage (lines / maps)\n");
    println!("| family | special | lines | maps | maps % |\n|---|---|---|---|---|");
    let all: Vec<i32> = DWUS
        .iter()
        .chain(BLAZE.iter())
        .chain(PERPETUAL.iter())
        .chain(RAISE_CHANGE.iter())
        .chain(RAISE_NEAREST.iter())
        .chain(LOWER.iter())
        .copied()
        .collect();
    for s in all {
        let key = s.to_string();
        let lines = agg.special_lines.0.get(&key).copied().unwrap_or(0);
        let maps = agg.special_maps.0.get(&key).copied().unwrap_or(0);
        println!(
            "| {} | {s} | {lines} | {maps} | {} |",
            family(s),
            pct(maps, agg.maps)
        );
    }
}

fn report_tags(agg: &Agg, np: u64) {
    println!("\n## B. Tag resolution (lift lines)\n");
    for (k, v) in &agg.lines_tag_resolves.0 {
        println!("- tag → {k} sector(s): {v}");
    }
    println!("- plats (unique tagged sectors reached by ≥1 lift line): **{np}**");
}

fn report_rest(plats: &[PlatFacts], np: u64, nmv: u64, step: i32) {
    println!("\n## C. Rest position and travel (step = {step})\n");
    for rest in [Rest::Dead, Rest::Top, Rest::AboveAll, Rest::Intermediate] {
        let travels: Vec<i32> = plats
            .iter()
            .filter(|p| p.rest == rest)
            .map(|p| p.travel)
            .collect();
        println!(
            "- {rest:?}: {} ({}) — travel {}",
            travels.len(),
            pct(count_len(travels.len()), np),
            percentiles(travels.clone())
        );
    }
    let travels: Vec<i32> = plats
        .iter()
        .filter(|p| p.moving())
        .map(|p| p.travel)
        .collect();
    let within = |limit: i32| count_len(travels.iter().filter(|&&t| t <= limit).count());
    println!(
        "- moving plats: {nmv}; travel ≤ 256: {} ({}); ≤ 128: {} ({}); ≤ 64: {} ({}); > 512: {}",
        within(256),
        pct(within(256), nmv),
        within(128),
        pct(within(128), nmv),
        within(64),
        pct(within(64), nmv),
        travels.iter().filter(|&&t| t > 512).count()
    );
    let mut values = Hist::default();
    for t in &travels {
        values.add(t.to_string());
    }
    println!("- travel values, top 12: {}", values.top(12));
    let multiple = |m: i32| count_len(travels.iter().filter(|&&t| t % m == 0).count());
    println!(
        "- travel multiple of 8: {} ({}) · of 16: {} ({}) · of 32: {} ({})",
        multiple(8),
        pct(multiple(8), nmv),
        multiple(16),
        pct(multiple(16), nmv),
        multiple(32),
        pct(multiple(32), nmv)
    );
    let intermediate: Vec<i32> = plats
        .iter()
        .filter(|p| p.rest == Rest::Intermediate)
        .map(|p| p.max_nb_delta)
        .collect();
    println!(
        "- Intermediate plats: height of the highest neighbor above the plat: {}",
        percentiles(intermediate)
    );
    report_rest_shapes(plats, nmv);
}

/// The `AboveAll` split, the rest × neighbor cross-tab, and the blaze share.
fn report_rest_shapes(plats: &[PlatFacts], nmv: u64) {
    let above: Vec<&PlatFacts> = plats.iter().filter(|p| p.rest == Rest::AboveAll).collect();
    let na = count_len(above.len());
    let mut overshoot = Hist::default();
    for p in &above {
        let o = -p.max_nb_delta;
        overshoot.add(if o <= 32 {
            "25..32"
        } else if o <= 64 {
            "33..64"
        } else if o <= 128 {
            "65..128"
        } else {
            ">128"
        });
    }
    let one_floor = count_len(above.iter().filter(|p| p.distinct_nb_floors == 1).count());
    let over_eq_travel = count_len(above.iter().filter(|p| -p.max_nb_delta == p.travel).count());
    println!(
        "- AboveAll plats ({na}): plat floor minus highest neighbor: {} · every neighbor at ONE floor (pillar/barrier): {} ({}) · highest neighbor == the low floor (nothing above the low floor): {} ({})",
        overshoot.all(),
        one_floor,
        pct(one_floor, na),
        over_eq_travel,
        pct(over_eq_travel, na)
    );
    let mut cross = Hist::default();
    for p in plats.iter().filter(|p| p.moving()) {
        cross.add(format!("{:?} × nb{}", p.rest, bucket_nb(p.neighbors.len())));
    }
    println!("- rest × neighbor count (moving): {}", cross.all());
    let one_nb: Vec<i32> = plats
        .iter()
        .filter(|p| p.moving() && p.neighbors.len() == 1)
        .map(|p| p.travel)
        .collect();
    println!(
        "- 1-neighbor moving plats: {} — travel {}",
        one_nb.len(),
        percentiles(one_nb)
    );
    let any_blaze = count_len(plats.iter().filter(|p| p.moving() && p.any_blaze).count());
    let all_blaze = count_len(plats.iter().filter(|p| p.moving() && p.all_blaze).count());
    println!(
        "- moving plats with any blaze trigger: {} ({}) · all triggers blaze: {} ({})",
        any_blaze,
        pct(any_blaze, nmv),
        all_blaze,
        pct(all_blaze, nmv)
    );
}

fn bucket_nb(n: usize) -> &'static str {
    match n {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        _ => "4+",
    }
}

fn report_topology(moving: &[&PlatFacts], nmv: u64) {
    println!("\n## D. Topology (moving plats only)\n");
    let mut nb = Hist::default();
    let mut two_sided = Hist::default();
    for p in moving {
        nb.add(bucket_nb(p.neighbors.len()));
        two_sided.add(match p.two_sided_edges {
            0 => "0",
            1 => "1",
            2 => "2",
            3 => "3",
            4 => "4",
            _ => "5+",
        });
    }
    println!("- distinct neighbor count: {}", nb.shares(nmv));
    println!("- two-sided edge count: {}", two_sided.shares(nmv));
    let two_nb: Vec<&&PlatFacts> = moving.iter().filter(|p| p.neighbors.len() == 2).collect();
    let gap = count_len(
        two_nb
            .iter()
            .filter(|p| p.two_nb_is_level_and_low == Some(true))
            .count(),
    );
    println!(
        "- 2-neighbor plats: {} — of which one neighbor is level (walk-on) and the other is the low floor: {} ({})",
        two_nb.len(),
        gap,
        pct(gap, count_len(two_nb.len()))
    );
    let islands = count_len(moving.iter().filter(|p| p.island()).count());
    println!(
        "- plats with no one-sided edge (islands): {} ({})",
        islands,
        pct(islands, nmv)
    );
}

fn report_size(moving: &[&PlatFacts], nmv: u64) {
    println!("\n## E. Size and grid (moving plats)\n");
    let mut max_side = Hist::default();
    let mut min_side = Hist::default();
    let mut dims = Hist::default();
    for p in moving {
        let (lo, hi) = (p.bbox_w.min(p.bbox_h), p.bbox_w.max(p.bbox_h));
        max_side.add(if hi <= 64 {
            "≤64"
        } else if hi <= 128 {
            "≤128"
        } else if hi <= 256 {
            "≤256"
        } else {
            ">256"
        });
        min_side.add(match lo {
            ..64 => "<64",
            64 => "=64",
            65..=128 => "65..128",
            129..=256 => "129..256",
            _ => ">256",
        });
        dims.add(format!("{lo}x{hi}"));
    }
    println!("- bbox max side: {}", max_side.shares(nmv));
    println!("- bbox min side: {}", min_side.shares(nmv));
    println!("- bbox dims (min x max), top 12: {}", dims.top(12));
    let aligned = count_len(moving.iter().filter(|p| p.aligned64()).count());
    println!(
        "- bbox min corner ≡ (0,0) mod 64: {} ({})",
        aligned,
        pct(aligned, nmv)
    );
    let sides = |m: i32| {
        count_len(
            moving
                .iter()
                .filter(|p| p.bbox_w % m == 0 && p.bbox_h % m == 0)
                .count(),
        )
    };
    println!(
        "- both bbox sides multiples of 64: {} ({}); of 32: {} ({})",
        sides(64),
        pct(sides(64), nmv),
        sides(32),
        pct(sides(32), nmv)
    );
}

fn combo_label(t: &common::Trigger) -> String {
    let kind = if USE_LIFT.contains(&t.special) {
        "S"
    } else {
        "W"
    };
    let blaze = if BLAZE.contains(&t.special) { "!" } else { "" };
    // The side a player fires it from: prefer a sector other than the plat
    // itself when a line fires from several, so a walkover on the plat's
    // edge is labeled by the side it is entered from.
    let activator = [
        Activator::Low,
        Activator::Level,
        Activator::Above,
        Activator::Plat,
    ]
    .into_iter()
    .find(|a| t.activators.contains(a))
    .unwrap_or(Activator::None);
    let place = match t.placement {
        Placement::Remote => "remote",
        Placement::Adjacent => "adj",
        Placement::OnPlatFront | Placement::OnPlatBack => "plat",
    };
    format!("{kind}{blaze}@{place}/{activator:?}")
}

/// Per-trigger-line histograms over the moving plats.
#[derive(Default)]
struct LineHists {
    placement: Hist,
    activators: Hist,
    use_on_plat_side: Hist,
    use_sided: Hist,
    walk_edges: Hist,
    combos: Hist,
}

fn line_hists(moving: &[&PlatFacts]) -> LineHists {
    let mut h = LineHists::default();
    for p in moving {
        let mut combo: Vec<String> = Vec::new();
        for t in &p.triggers {
            let kind = if USE_LIFT.contains(&t.special) {
                "S"
            } else {
                "W"
            };
            h.placement.add(format!("{kind} {:?}", t.placement));
            if kind == "S" {
                if matches!(t.placement, Placement::OnPlatFront | Placement::OnPlatBack) {
                    h.use_on_plat_side.add(format!("{:?}", t.placement));
                }
                h.use_sided.add(if t.one_sided {
                    "one-sided"
                } else {
                    "two-sided"
                });
            }
            for a in &t.activators {
                h.activators.add(format!("{kind} from {a:?}"));
                if kind == "W" {
                    h.walk_edges
                        .add(format!("{:?} edge, from {a:?}", t.placement));
                }
            }
            combo.push(combo_label(t));
        }
        combo.sort();
        combo.dedup();
        h.combos.add(combo.join(" + "));
    }
    h
}

fn report_triggers(agg: &Agg, moving: &[&PlatFacts], nmv: u64) {
    println!("\n## F. Triggers (moving plats)\n");
    let mut per_plat = Hist::default();
    let (mut callable_low, mut callable_level, mut both, mut neither) = (0, 0, 0, 0);
    let (mut low_use_only, mut low_walk, mut repeat_only, mut mixed_speed) = (0, 0, 0, 0);
    for p in moving {
        per_plat.add(bucket_nb(p.triggers.len()));
        let low_by = |use_line: bool| {
            p.triggers.iter().any(|t| {
                USE_LIFT.contains(&t.special) == use_line && t.activators.contains(&Activator::Low)
            })
        };
        let (low_use, low_walk_here) = (low_by(true), low_by(false));
        callable_low += u64::from(p.callable_low);
        callable_level += u64::from(p.callable_level_only);
        both += u64::from(p.callable_low && p.callable_level_only);
        neither += u64::from(!p.callable_low && !p.callable_level_only);
        low_use_only += u64::from(low_use && !low_walk_here);
        low_walk += u64::from(low_walk_here);
        repeat_only += u64::from(
            p.triggers
                .iter()
                .all(|t| REPEATABLE_LIFT.contains(&t.special)),
        );
        mixed_speed += u64::from(p.any_blaze && !p.all_blaze);
    }
    println!("- trigger lines per plat: {}", per_plat.shares(nmv));
    println!(
        "- callable from a LOW activator (P5 'from the bottom'): {} ({}) — via use-line only: {}; via a walkover from low: {}",
        callable_low,
        pct(callable_low, nmv),
        low_use_only,
        low_walk
    );
    println!(
        "- callable from a LEVEL activator (top / walk-on): {} ({}) · both low and level: {} ({}) · neither: {} ({})",
        callable_level,
        pct(callable_level, nmv),
        both,
        pct(both, nmv),
        neither,
        pct(neither, nmv)
    );
    println!(
        "- plats whose every trigger is a repeatable form: {} ({}) · plats mixing normal and blaze specials: {}",
        repeat_only,
        pct(repeat_only, nmv),
        mixed_speed
    );
    report_trigger_lines(agg, &line_hists(moving), nmv);
}

fn report_trigger_lines(agg: &Agg, h: &LineHists, nmv: u64) {
    println!(
        "- trigger placement (per trigger line): {}",
        h.placement.all()
    );
    println!(
        "- use-line on the plat: which side is the plat: {}",
        h.use_on_plat_side.all()
    );
    println!("- use-line sidedness: {}", h.use_sided.all());
    println!("- activators (per trigger line): {}", h.activators.all());
    println!("- walkover edges: {}", h.walk_edges.all());
    println!("- trigger-set combos, top 15:");
    let mut top: Vec<(&String, &u64)> = h.combos.0.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    for (combo, n) in top.iter().take(15) {
        println!("  - `{combo}`: {n} ({})", pct(**n, nmv));
    }
    println!(
        "- switch lines: {} · with an SW1*/SW2* texture on the front sidedef: {} ({}) · slot: {} · top textures: {}",
        agg.switch_lines,
        agg.switch_sw_prefix,
        pct(agg.switch_sw_prefix, agg.switch_lines),
        agg.switch_slot.all(),
        agg.switch_tex.top(8)
    );
}

fn report_rendering(moving: &[&PlatFacts], nmv: u64) {
    println!("\n## G. Rendering-facing (moving plats)\n");
    let mut textures = Hist::default();
    let (mut risers, mut missing, mut unpegged, mut plat_side_nonblank) = (0, 0, 0, 0);
    for p in moving {
        for r in &p.risers {
            risers += 1;
            if r.texture == "-" {
                missing += 1;
            } else {
                textures.add(r.texture.clone());
            }
            unpegged += u64::from(r.unpegged);
            plat_side_nonblank += u64::from(r.plat_side_nonblank);
        }
    }
    println!(
        "- riser boundaries (neighbor floor below plat): {risers} · lower texture missing ('-') on the visible side: {} ({}) · lower-unpegged: {} ({})",
        missing,
        pct(missing, risers),
        unpegged,
        pct(unpegged, risers)
    );
    println!("- riser textures, top 15: {}", textures.top(15));
    println!(
        "- the plat's OWN sidedef on a riser line carries a non-blank lower: {} of {} ({})",
        plat_side_nonblank,
        risers,
        pct(plat_side_nonblank, risers)
    );
    let level_same = count_len(
        moving
            .iter()
            .filter(|p| p.flat_same_as_level_nb == Some(true))
            .count(),
    );
    let level_n = count_len(
        moving
            .iter()
            .filter(|p| p.flat_same_as_level_nb.is_some())
            .count(),
    );
    let low_same = count_len(
        moving
            .iter()
            .filter(|p| p.flat_same_as_low_nb == Some(true))
            .count(),
    );
    let low_n = count_len(
        moving
            .iter()
            .filter(|p| p.flat_same_as_low_nb.is_some())
            .count(),
    );
    println!(
        "- plat flat == level neighbor's flat: {} of {} ({}) · == low neighbor's flat: {} of {} ({})",
        level_same,
        level_n,
        pct(level_same, level_n),
        low_same,
        low_n,
        pct(low_same, low_n)
    );
    let light = count_len(moving.iter().filter(|p| p.light_eq_all).count());
    let ceiling = count_len(moving.iter().filter(|p| p.ceiling_eq_all).count());
    println!(
        "- light equal to every neighbor: {} ({}) · ceiling equal to every neighbor: {} ({})",
        light,
        pct(light, nmv),
        ceiling,
        pct(ceiling, nmv)
    );
    let special = count_len(moving.iter().filter(|p| p.sector_special != 0).count());
    println!(
        "- plat sector special nonzero: {} ({})",
        special,
        pct(special, nmv)
    );
}

fn report_conflicts(moving: &[&PlatFacts], nmv: u64) {
    println!("\n## H. Conflicts\n");
    let conflicts = count_len(
        moving
            .iter()
            .filter(|p| !p.other_tagged_specials.is_empty())
            .count(),
    );
    let mut others = Hist::default();
    for p in moving {
        for s in &p.other_tagged_specials {
            others.add(s.to_string());
        }
    }
    println!(
        "- moving plats whose tag is also the target of a non-lift tagged special: {} ({}) · those specials, top 10: {}",
        conflicts,
        pct(conflicts, nmv),
        others.top(10)
    );
}

fn report_floor_pairing(agg: &Agg) {
    println!("\n## I. Floor-special 'up lifts' (rest-low candidates)\n");
    println!(
        "- tagged sectors targeted by a raise-to-nearest / raise&change special only: {} — of which resting more than a step below some neighbor: {}",
        agg.sectors_raise_only, agg.sectors_raise_rest_low
    );
    println!(
        "- tagged sectors targeted by a lower special only: {}",
        agg.sectors_lower_only
    );
    println!(
        "- tagged sectors targeted by BOTH a raise and a lower special (two-way floor elevator): {} in {} maps ({})",
        agg.sectors_raise_and_lower,
        agg.maps_raise_and_lower,
        pct(agg.maps_raise_and_lower, agg.maps)
    );
}

fn report_arbiter(agg: &Agg, moving: &[&PlatFacts], nmv: u64) {
    println!("\n## J. Arbiter — expressibility if lifts join the emittable set\n");
    println!(
        "- line axis: now {} ({}) · +62/88: {} ({}) · +62/88/120/123: {} ({})",
        agg.line_ok_now,
        pct(agg.line_ok_now, agg.maps),
        agg.line_ok_with_dwus,
        pct(agg.line_ok_with_dwus, agg.maps),
        agg.line_ok_with_dwus_blaze,
        pct(agg.line_ok_with_dwus_blaze, agg.maps)
    );
    println!(
        "- all axes: now {} ({}) · +62/88: {} ({}) · +62/88/120/123: {} ({})",
        agg.expr_now,
        pct(agg.expr_now, agg.maps),
        agg.expr_with_dwus,
        pct(agg.expr_with_dwus, agg.maps),
        agg.expr_with_dwus_blaze,
        pct(agg.expr_with_dwus_blaze, agg.maps)
    );

    println!("\n## K. Shapes and the shape-gated arbiter\n");
    let mut shapes = Hist::default();
    let mut why = Hist::default();
    for p in moving {
        shapes.add(format!("{:?}", p.shape));
        if p.shape != Shape::Other {
            continue;
        }
        let repeatable = p
            .triggers
            .iter()
            .all(|t| REPEATABLE_LIFT.contains(&t.special));
        let reason = if !p.callable_low {
            "not callable from low"
        } else if !repeatable {
            "one-shot trigger"
        } else if p.any_blaze && !p.all_blaze {
            "mixed speed"
        } else if !p.other_tagged_specials.is_empty() {
            "conflicting tagged action"
        } else {
            match p.rest {
                Rest::Intermediate => "intermediate rest",
                Rest::AboveAll => "above-all, neighbors at several floors",
                Rest::Dead | Rest::Top => "other",
            }
        };
        why.add(reason);
    }
    println!("- moving plats by shape: {}", shapes.shares(nmv));
    println!("- why Other: {}", why.all());
    println!(
        "- lift maps: {} · with a refused line (tag 0 / unresolved tag): {} · every plat Core: {} ({}) · every plat Core∪Pedestal∪Barrier: {} ({})",
        agg.maps_with_lift,
        agg.maps_lift_refused,
        agg.maps_all_core,
        pct(agg.maps_all_core, agg.maps_with_lift),
        agg.maps_all_ext,
        pct(agg.maps_all_ext, agg.maps_with_lift)
    );
    println!(
        "- line axis, shape-gated: Core only {} ({}) · Core∪Pedestal∪Barrier {} ({})",
        agg.line_core,
        pct(agg.line_core, agg.maps),
        agg.line_ext,
        pct(agg.line_ext, agg.maps)
    );
    println!(
        "- all axes, shape-gated: Core only {} ({}) · Core∪Pedestal∪Barrier {} ({})",
        agg.expr_core,
        pct(agg.expr_core, agg.maps),
        agg.expr_ext,
        pct(agg.expr_ext, agg.maps)
    );
}
