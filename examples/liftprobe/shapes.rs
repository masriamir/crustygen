//! Pass 2 — per-shape facts: multi-sector tag groups, and for each shape the
//! footprint, rise, contents, fences, trigger faces and host relations.

use std::collections::{BTreeMap, BTreeSet};

use crustygen::check::scene::Scene;
use crustygen::tables::Tables;
use crustywad::map::udmf::UdmfMap;

use crate::common::{self, Hist, PlatFacts, Shape, USE_LIFT, is_lift, pct, percentiles};

#[derive(Default)]
struct ShapeAgg {
    n: u64,
    dims: Hist,
    travel: Vec<i32>,
    things_any: u64,
    thing_names: Hist,
    thing_count: Hist,
    fence_any: u64,
    island: u64,
    edges_with_special: Hist,
    callable_nb_share: Hist,
    top_trigger_any: u64,
    switch_tex_any: u64,
    riser_tex: Hist,
    aligned64: u64,
    sides64: u64,
    min_side: Hist,
    host_light_eq: u64,
    host_flat_eq: u64,
    ceiling_eq_all: u64,
    tag_group_n: Hist,
}

#[derive(Default)]
struct Agg {
    maps: u64,
    shapes: BTreeMap<Shape, ShapeAgg>,
    groups_n: u64,
    groups_one_floor_connected: u64,
    groups_one_floor_disconnected: u64,
    groups_several_floors: u64,
    groups_size: Hist,
}

/// Runs the per-shape pass over `dirs` and prints the report for `label`.
pub(crate) fn run(label: &str, dirs: &[String]) {
    let tables = Tables::load().expect("tables");
    let step = tables.step_height();
    let mut agg = Agg::default();
    agg.maps = common::sweep(dirs, |_, map| survey_map(map, &tables, step, &mut agg));
    report(label, &agg);
}

fn survey_map(map: &UdmfMap, tables: &Tables, step: i32, agg: &mut Agg) {
    if !map.linedefs.iter().any(|l| is_lift(l.special)) {
        return;
    }
    let scene = Scene::build(map, tables, &mut Vec::new());
    let index = common::MapIndex::build(map, &scene);
    survey_tag_groups(map, &scene, &index, agg);
    for plat in index.plat_sectors(map) {
        if let Some(facts) = common::analyze_plat(map, &scene, &index, plat, step) {
            record(&facts, agg);
        }
    }
}

/// A lift tag naming several sectors: one platform split by trim, several
/// lifts on one trigger, or sectors at several floors.
fn survey_tag_groups(map: &UdmfMap, scene: &Scene, index: &common::MapIndex<'_>, agg: &mut Agg) {
    let lift_tags: BTreeSet<i32> = map
        .linedefs
        .iter()
        .filter(|l| is_lift(l.special) && l.args[0] != 0)
        .map(|l| l.args[0])
        .collect();
    for tag in lift_tags {
        let Some(secs) = index.by_tag.get(&tag) else {
            continue;
        };
        if secs.len() < 2 {
            continue;
        }
        agg.groups_n += 1;
        agg.groups_size.add(match secs.len() {
            2 => "2",
            3 => "3",
            4 => "4",
            _ => "5+",
        });
        let floors: BTreeSet<i32> = secs.iter().map(|&s| scene.sectors[s].floor).collect();
        if floors.len() > 1 {
            agg.groups_several_floors += 1;
            continue;
        }
        let set: BTreeSet<usize> = secs.iter().copied().collect();
        let mut reached: BTreeSet<usize> = BTreeSet::new();
        let mut stack = vec![secs[0]];
        while let Some(s) = stack.pop() {
            if !reached.insert(s) {
                continue;
            }
            stack.extend(
                scene.sectors[s]
                    .boundary
                    .iter()
                    .filter_map(|b| b.neighbor)
                    .filter(|n| set.contains(n) && !reached.contains(n)),
            );
        }
        if reached.len() == set.len() {
            agg.groups_one_floor_connected += 1;
        } else {
            agg.groups_one_floor_disconnected += 1;
        }
    }
}

fn record(p: &PlatFacts, agg: &mut Agg) {
    let sa = agg.shapes.entry(p.shape).or_default();
    sa.n += 1;
    let (lo, hi) = (p.bbox_w.min(p.bbox_h), p.bbox_w.max(p.bbox_h));
    if p.has_geometry {
        sa.dims.add(format!("{lo}x{hi}"));
    }
    sa.travel.push(p.travel);
    sa.things_any += u64::from(p.things > 0);
    sa.thing_count.add(match p.things {
        0 => "0",
        1 => "1",
        2 => "2",
        3 => "3",
        _ => "4+",
    });
    for name in &p.thing_names {
        sa.thing_names.add(name.clone());
    }
    sa.fence_any += u64::from(p.blocking_two_sided > 0);
    sa.island += u64::from(p.island());
    sa.edges_with_special
        .add(format!("{}/{}", p.edges_with_special, p.two_sided_edges));
    sa.callable_nb_share
        .add(format!("{} of {}", p.low_activator_nbs, p.neighbors.len()));
    sa.top_trigger_any += u64::from(p.callable_level);
    sa.switch_tex_any += u64::from(
        p.triggers
            .iter()
            .any(|t| USE_LIFT.contains(&t.special) && !t.switch_slots.is_empty()),
    );
    for r in &p.risers {
        sa.riser_tex.add(r.texture.clone());
    }
    if p.has_geometry {
        sa.aligned64 += u64::from(p.aligned64());
        sa.sides64 += u64::from(p.bbox_w % 64 == 0 && p.bbox_h % 64 == 0);
        sa.min_side.add(match lo {
            ..64 => "<64",
            64 => "=64",
            65..=128 => "65..128",
            _ => ">128",
        });
    }
    sa.host_light_eq += u64::from(p.light_eq_host);
    sa.host_flat_eq += u64::from(p.flat_eq_host);
    sa.ceiling_eq_all += u64::from(p.ceiling_eq_all);
    sa.tag_group_n.add(match p.shared_tag_n {
        1 => "1",
        2 => "2",
        _ => "3+",
    });
}

fn report(label: &str, agg: &Agg) {
    println!("# liftprobe shapes — {label}\n\nMaps: {}\n", agg.maps);
    println!("## Multi-sector tag groups\n");
    println!(
        "- groups (a lift tag naming ≥2 sectors): {} · size: {}",
        agg.groups_n,
        agg.groups_size.all()
    );
    println!(
        "- all at one floor and mutually connected (one platform split by trim): {} ({}) · one floor but disconnected (several lifts on one trigger): {} ({}) · several floors: {} ({})",
        agg.groups_one_floor_connected,
        pct(agg.groups_one_floor_connected, agg.groups_n),
        agg.groups_one_floor_disconnected,
        pct(agg.groups_one_floor_disconnected, agg.groups_n),
        agg.groups_several_floors,
        pct(agg.groups_several_floors, agg.groups_n)
    );
    for (shape, sa) in &agg.shapes {
        println!("\n## {shape:?} — {} plats\n", sa.n);
        println!("- bbox dims top 10: {}", sa.dims.top(10));
        println!(
            "- min side: {} · bbox min corner ≡ (0,0) mod 64: {} ({}) · both sides ≡ 0 mod 64: {} ({})",
            sa.min_side.all(),
            sa.aligned64,
            pct(sa.aligned64, sa.n),
            sa.sides64,
            pct(sa.sides64, sa.n)
        );
        println!("- travel (rise): {}", percentiles(sa.travel.clone()));
        println!(
            "- island (no one-sided edge): {} ({})",
            sa.island,
            pct(sa.island, sa.n)
        );
        println!(
            "- plats holding ≥1 thing: {} ({}) · things per plat: {} · thing names top 12: {}",
            sa.things_any,
            pct(sa.things_any, sa.n),
            sa.thing_count.all(),
            sa.thing_names.top(12)
        );
        println!(
            "- any two-sided edge with ML_BLOCKING (fence): {} ({})",
            sa.fence_any,
            pct(sa.fence_any, sa.n)
        );
        println!(
            "- two-sided edges carrying a lift special, k/n top 8: {}",
            sa.edges_with_special.top(8)
        );
        println!(
            "- neighbors that are Low activators, k of n top 6: {}",
            sa.callable_nb_share.top(6)
        );
        println!(
            "- any top-side trigger (Level/Plat activator): {} ({}) · any S line with an SW texture: {} ({})",
            sa.top_trigger_any,
            pct(sa.top_trigger_any, sa.n),
            sa.switch_tex_any,
            pct(sa.switch_tex_any, sa.n)
        );
        println!("- riser textures top 8: {}", sa.riser_tex.top(8));
        println!(
            "- light == host: {} ({}) · flat == host: {} ({}) · ceiling == every neighbor: {} ({})",
            sa.host_light_eq,
            pct(sa.host_light_eq, sa.n),
            sa.host_flat_eq,
            pct(sa.host_flat_eq, sa.n),
            sa.ceiling_eq_all,
            pct(sa.ceiling_eq_all, sa.n)
        );
        println!(
            "- sectors sharing this plat's tag: {}",
            sa.tag_group_n.all()
        );
    }
}
