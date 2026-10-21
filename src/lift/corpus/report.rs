//! The Markdown and JSON report: the sample's provenance, the buckets, the
//! shares, the blocker tables, the recognizer sections and the greedy curves.

use std::path::Path;

use super::aggregate::{Aggregate, BLOCKER_TOP, u64_share};
use super::sweep::{Buckets, MapRecord};

/// The identity of a sample, echoed from `sample-manifest.json` when the
/// swept directory holds one (written by crustywad's `xtask harvest-sample`).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Provenance {
    /// The draw's seed.
    pub seed: u64,
    /// The requested sample size.
    pub count: usize,
    /// Rows in the sampling frame.
    pub frame_rows: usize,
    /// Hash of the fetch list the frame was cut from.
    pub fetch_list_hash: String,
    /// Sampled archive ids, ascending.
    pub ids: Vec<u64>,
}

/// Reads `<dir>/sample-manifest.json`; `None` when absent or unparseable.
///
/// An absent manifest is silent — most swept directories are not samples.
/// One that is present but unreadable or malformed warns on stderr, because
/// silently dropping the provenance section would leave a report that looks
/// complete while having lost the identity of its sample. This library
/// function writes to stderr only because stderr is `crustygen-corpus`'s
/// operator channel and this is the sole caller.
#[must_use]
pub fn read_provenance(dir: &Path) -> Option<Provenance> {
    /// One manifest entry; only the archive id is echoed, the rest of the
    /// harvest bookkeeping (directory, filename, size, status) is ignored.
    #[derive(serde::Deserialize)]
    struct Entry {
        id: u64,
    }
    /// The subset of `sample-manifest.json` the report echoes.
    #[derive(serde::Deserialize)]
    struct Manifest {
        seed: u64,
        count: usize,
        frame_rows: usize,
        fetch_list_hash: String,
        entries: Vec<Entry>,
    }
    let path = dir.join("sample-manifest.json");
    let parsed = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Manifest>(&text).ok());
    let Some(m) = parsed else {
        // Reached for a read failure as well as a parse failure; `exists`
        // separates "this directory is simply not a sample" from "its
        // manifest is there but unusable", which is the case worth naming.
        if path.exists() {
            eprintln!(
                "crustygen-corpus: sample-manifest.json is unreadable or malformed; \
                 no provenance recorded"
            );
        }
        return None;
    };
    let mut ids: Vec<u64> = m.entries.iter().map(|e| e.id).collect();
    ids.sort_unstable();
    Some(Provenance {
        seed: m.seed,
        count: m.count,
        frame_rows: m.frame_rows,
        fetch_list_hash: m.fetch_list_hash,
        ids,
    })
}

/// The `--json` document.
#[derive(Debug, serde::Serialize)]
pub struct Report<'a> {
    /// Sample identity, when present.
    pub provenance: Option<&'a Provenance>,
    /// The counts.
    pub buckets: &'a Buckets,
    /// The summary.
    pub aggregate: &'a Aggregate,
    /// Every unique map.
    pub maps: &'a [MapRecord],
}

/// A fraction as a one-decimal percentage.
fn pct(x: f64) -> String {
    format!("{:.1} %", x * 100.0)
}

/// The Markdown report: header caveat, provenance, buckets, shares, blockers,
/// greedy checkpoints. Aggregate tables only — no per-map rows.
#[must_use]
pub fn render_markdown(provenance: Option<&Provenance>, b: &Buckets, a: &Aggregate) -> String {
    use std::fmt::Write as _;

    let mut s = String::new();
    s.push_str("# Corpus expressibility\n\n");
    s.push_str(
        "> **Status of these numbers: measured practice, not engine fact — and an upper bound.** ",
    );
    s.push_str("A map counts as expressible when every non-zero line special and sector special, and every thing type, it carries is in crustygen's emittable vocabulary, every teleport line it carries is one the recognizer can state (see `Teleports`), every platform its lift lines name is one of the three shapes the IR can state (see `Lifts`), and every floor target its floor lines name is one of the three shapes the IR can state (see `Floors`). ");
    s.push_str("Beyond those teleport lines, platforms, and floor targets, geometry, flags, tags, and texture names are not measured; a geometry-aware lifter can only do worse, never better, than this bound.\n\n");
    if let Some(p) = provenance {
        s.push_str("## Sample\n\n");
        let _ = writeln!(
            s,
            "- seed `{}`, count {}, frame rows {}, fetch list `{}`",
            p.seed, p.count, p.frame_rows, p.fetch_list_hash
        );
        let ids: Vec<String> = p.ids.iter().map(ToString::to_string).collect();
        let _ = writeln!(s, "- ids: {}\n", ids.join(" "));
    }
    s.push_str("## Buckets\n\n| Bucket | Count |\n|---|---|\n");
    for (name, n) in [
        ("archives", b.archives),
        ("wads", b.wads),
        ("maps_raw", b.maps_raw),
        ("maps_unique", b.maps_unique),
        ("archive_unreadable", b.archive_unreadable),
        ("wad_unreadable", b.wad_unreadable),
        ("no_maps", b.no_maps),
        ("unsupported_format", b.unsupported_format),
        ("assembly_refused", b.assembly_refused),
        ("textmap_unparseable", b.textmap_unparseable),
    ] {
        let _ = writeln!(s, "| `{name}` | {n} |");
    }
    s.push_str(
        "\n## Expressibility\n\n| Axis | All unique maps | Vanilla-only slice |\n|---|---|---|\n",
    );
    for (name, all, van) in [
        (
            "line specials",
            a.all.line_specials,
            a.vanilla.line_specials,
        ),
        (
            "sector specials",
            a.all.sector_specials,
            a.vanilla.sector_specials,
        ),
        ("thing kinds", a.all.thing_kinds, a.vanilla.thing_kinds),
        ("teleport lines", a.all.teleports, a.vanilla.teleports),
        ("lift platforms", a.all.lifts, a.vanilla.lifts),
        ("floor targets", a.all.floors, a.vanilla.floors),
        ("**all axes**", a.all.expressible, a.vanilla.expressible),
    ] {
        let _ = writeln!(s, "| {name} | {} | {} |", pct(all), pct(van));
    }
    let _ = writeln!(
        s,
        "\nVanilla-only slice: {} of unique maps.",
        pct(a.vanilla_share)
    );
    for (title, list) in [
        ("Line-special blockers", &a.line_blockers),
        ("Sector-special blockers", &a.sector_blockers),
        ("Thing-type blockers", &a.thing_blockers),
    ] {
        let _ = write!(
            s,
            "\n## {title}\n\nTop {BLOCKER_TOP} by maps carrying the value (ties by ascending value). \
             Share is of all unique maps.\n\n| Value | Maps | Share of unique maps |\n|---|---|---|\n"
        );
        if list.is_empty() {
            // No out-of-set value on this axis: say so in a row, mirroring
            // the stepless-curve baseline, rather than leaving a bare header.
            s.push_str("| (none) | | |\n");
            continue;
        }
        for bl in list {
            let _ = writeln!(s, "| {} | {} | {} |", bl.value, bl.maps, pct(bl.share));
        }
    }
    render_teleports(&mut s, a);
    render_lifts(&mut s, a);
    render_floors(&mut s, a);
    render_curves(&mut s, a);
    s
}

/// The teleport recognizer's section. Every shape row carries both a line
/// count and a map count: a line total alone cannot tell one map with forty
/// closets from forty maps with one each, and the difference is the whole
/// question when reading which shapes a corpus actually uses. Counts are
/// over the maps that carry a teleport line, not over every unique map.
fn render_teleports(s: &mut String, a: &Aggregate) {
    use std::fmt::Write as _;

    let t = &a.teleports;
    let c = &t.lines;
    s.push_str("\n## Teleports\n\n| Measure | Value |\n|---|---|\n");
    let _ = writeln!(
        s,
        "| maps with a teleport line | {} ({} of unique maps) |",
        t.maps_with_teleports,
        pct(u64_share(t.maps_with_teleports, a.maps_unique))
    );
    let _ = writeln!(
        s,
        "| maps with a refused line (not expressible) | {} |",
        t.maps_refused
    );
    let _ = writeln!(
        s,
        "| lines: player / monsters-only / one-shot | {} / {} / {} |",
        c.player, c.monsters_only, c.one_shot
    );
    for (label, lines, maps) in [
        (
            "lines in closets (front sector holds a monster)",
            c.closet,
            t.closet_maps,
        ),
        ("lines delivering beside an exit", c.exit, t.exit_maps),
        ("lines on a paired pad", c.paired, t.paired_maps),
    ] {
        let plural = if maps == 1 { "map" } else { "maps" };
        let _ = writeln!(s, "| {label} | {lines} ({maps} {plural}) |");
    }
    let _ = writeln!(
        s,
        "| geometry: island / alcove / boundary / other | {} / {} / {} / {} |",
        c.island, c.alcove, c.boundary, c.other
    );
    let _ = writeln!(
        s,
        "| ambiguous (several markers) / broken / self-referencing | {} / {} / {} |",
        c.ambiguous, c.broken, c.self_referencing
    );
}

/// The plat recognizer's section. Counts are over the maps that carry a lift
/// line, not over every unique map, the way [`render_teleports`]'s are.
///
/// The shape row counts *platforms* and the row under it counts the maps
/// they sit in, because a corpus total alone cannot tell one map with a
/// dozen lifts from a dozen maps with one each. The split-group row tallies
/// tag *groups* that are one platform split by trim, each read as one
/// platform, and the sectors those groups comprise, so `plats` reconciles
/// across the merge; it is neither a platform tally nor a refusal, so it
/// keeps its own row instead of a slot in the refusal list.
fn render_lifts(s: &mut String, a: &Aggregate) {
    use std::fmt::Write as _;

    let l = &a.lifts;
    let c = &l.plats;
    s.push_str("\n## Lifts\n\n| Measure | Value |\n|---|---|\n");
    let _ = writeln!(
        s,
        "| maps with a lift line | {} ({} of unique maps) |",
        l.maps_with_lifts,
        pct(u64_share(l.maps_with_lifts, a.maps_unique))
    );
    let _ = writeln!(
        s,
        "| maps with a refused plat or line (not expressible) | {} |",
        l.maps_refused
    );
    let _ = writeln!(s, "| plats | {} |", c.plats);
    let _ = writeln!(
        s,
        "| lifts / pedestals / barriers | {} / {} / {} |",
        c.lifts, c.pedestals, c.barriers
    );
    let _ = writeln!(
        s,
        "| maps with a lift / pedestal / barrier | {} / {} / {} |",
        l.lift_maps, l.pedestal_maps, l.barrier_maps
    );
    let _ = writeln!(
        s,
        "| refused: dead / one-shot / mixed speed / unsupported rest / top-only / one-way barrier / \
         bank caller / conflicting action | {} / {} / {} / {} / {} / {} / {} / {} |",
        c.dead,
        c.one_shot,
        c.mixed_speed,
        c.unsupported_rest,
        c.top_only,
        c.one_way_barrier,
        c.bank_caller,
        c.conflicting
    );
    let _ = writeln!(
        s,
        "| split groups read as one platform / sectors they comprise | {} / {} |",
        c.shared_split, c.split_members
    );
    let _ = writeln!(s, "| broken lines | {} |", c.broken_lines);
    let _ = writeln!(
        s,
        "| callable from below | {} ({} of plats) |",
        c.callable_low,
        pct(u64_share(c.callable_low, c.plats))
    );
    let _ = writeln!(s, "| with a top trigger | {} |", c.with_top_trigger);
    let _ = writeln!(s, "| holding things | {} |", c.with_things);
    let _ = writeln!(s, "| fast | {} |", c.fast);
}

/// The floor recognizer's section. Counts are over the maps that carry a
/// floor line, not over every unique map, the way [`render_teleports`]'s and
/// [`render_lifts`]'s are.
///
/// The shape row counts *targets* and the row under it counts the maps they
/// sit in, for the reason [`render_lifts`] gives. Refusals are reported in
/// the fixed precedence order of `crate::lift::floor`'s module doc (its
/// twelve [`crate::lift::floor::Refusal`]s), and broken lines get their own
/// row split by [`crate::lift::floor::FloorCounts::tag_zero`] and
/// [`crate::lift::floor::FloorCounts::dangling`] rather than folded into the
/// refusal list, since a broken line names no target at all to refuse.
fn render_floors(s: &mut String, a: &Aggregate) {
    use std::fmt::Write as _;

    let f = &a.floors;
    let c = &f.targets;
    s.push_str("\n## Floors\n\n| Measure | Value |\n|---|---|\n");
    let _ = writeln!(
        s,
        "| maps with a floor line | {} ({} of unique maps) |",
        f.maps_with_floors,
        pct(u64_share(f.maps_with_floors, a.maps_unique))
    );
    let _ = writeln!(
        s,
        "| maps with a refused target or line (not expressible) | {} |",
        f.maps_refused
    );
    let _ = writeln!(s, "| targets | {} |", c.targets);
    let _ = writeln!(
        s,
        "| drop walls / reveals / bridges | {} / {} / {} |",
        c.drop_walls, c.reveals, c.bridges
    );
    let _ = writeln!(
        s,
        "| maps with a drop wall / reveal / bridge | {} / {} / {} |",
        f.drop_wall_maps, f.reveal_maps, f.bridge_maps
    );
    let _ = writeln!(
        s,
        "| refused: gun / conflict / two families / unresolved / dead / closing / mixed / \
         neutral / rider loses / no activator / unsupported shape / neighbors mover | {} / {} \
         / {} / {} / {} / {} / {} / {} / {} / {} / {} / {} |",
        c.gun,
        c.conflict,
        c.two_families,
        c.unresolved,
        c.dead,
        c.closing,
        c.mixed,
        c.neutral,
        c.rider_loses,
        c.no_activator,
        c.unsupported_shape,
        c.neighbors_mover
    );
    let _ = writeln!(
        s,
        "| broken lines: tag 0 / dangling | {} / {} |",
        c.tag_zero, c.dangling
    );
    let _ = writeln!(
        s,
        "| shared-tag members accepted | {} |",
        c.shared_tag_accepted
    );
    let _ = writeln!(s, "| remote triggers | {} |", c.remote_triggers);
    let _ = writeln!(s, "| targets holding a thing | {} |", c.with_things);
}

/// The two greedy-curve sections. Both curves are scored against *all* unique
/// maps, never against the population they walk — spelled out in each table's
/// header and note, so the conjunction curve's plateau below 100 % reads as
/// the remainder blocked on the other five axes it is, not as a truncated
/// curve.
///
/// The order line prints at most the first `BLOCKER_TOP` steps and, when it
/// cuts a longer curve, says so with an ellipsis and the full step count.
fn render_curves(s: &mut String, a: &Aggregate) {
    use std::fmt::Write as _;

    for (title, note, curve, baseline) in [
        (
            "Greedy curve — line axis alone",
            "Share is of all unique maps, with sector specials and thing kinds held expressible.",
            &a.greedy_line_axis,
            a.all.line_specials,
        ),
        (
            "Greedy curve — conjunction (maps already ok on sectors, things, teleports, lifts, \
             and floors)",
            "Share is of **all unique maps**, not of the already-ok population this curve walks, \
             so it plateaus below 100 % by exactly the maps blocked on a sector special, a \
             thing kind, a refused teleport line, a refused platform, or a refused floor target.",
            &a.greedy_conjunction,
            a.all.expressible,
        ),
    ] {
        let _ = write!(
            s,
            "\n## {title}\n\n{note}\n\n| k | Cumulative share of all unique maps |\n|---|---|\n"
        );
        if curve.checkpoints.is_empty() {
            // No checkpoint to print: no map in this population is blocked on
            // the line axis (the population may itself be empty), so there is
            // nothing to add greedily (`checkpoints` is empty exactly when
            // `steps` is): state the standing share at k = 0 rather than
            // emitting a table header with no rows under it.
            let _ = writeln!(s, "| 0 | {} |", pct(baseline));
            s.push_str("\nOrder chosen: (none)\n");
            continue;
        }
        for (k, share) in &curve.checkpoints {
            let _ = writeln!(s, "| {k} | {} |", pct(*share));
        }
        let order: Vec<String> = curve
            .steps
            .iter()
            .take(BLOCKER_TOP)
            .map(|st| st.special.to_string())
            .collect();
        let mut line = order.join(" → ");
        if curve.steps.len() > BLOCKER_TOP {
            let _ = write!(
                line,
                " → … (first {BLOCKER_TOP} of {} steps)",
                curve.steps.len()
            );
        }
        let _ = writeln!(s, "\nOrder chosen: {line}");
    }
}

#[cfg(test)]
mod tests {
    use super::super::aggregate;
    use super::super::fixtures::record;
    use super::*;
    use crate::lift::floor::FloorCounts;
    use crate::lift::plat::PlatCounts;
    use crate::lift::teleport::TeleportCounts;

    #[test]
    fn markdown_carries_the_caveat_and_every_section() {
        let a = aggregate(&[record(&[97], &[97], true, true, true)]);
        let md = render_markdown(None, &Buckets::default(), &a);
        for needle in [
            "upper bound",
            "## Buckets",
            "## Expressibility",
            "## Line-special blockers",
            // Each blocker table names its cut and its share's denominator.
            "Top 25 by maps carrying the value (ties by ascending value). Share is of all unique maps.",
            "| Value | Maps | Share of unique maps |",
            "## Teleports",
            "| teleport lines | 100.0 % | 100.0 % |",
            // A teleport-free corpus states its zeros rather than hiding the
            // section: the reader can tell "measured, none found" from
            // "not measured".
            "| maps with a teleport line | 0 (0.0 % of unique maps) |",
            "| lines on a paired pad | 0 (0 maps) |",
            "## Lifts",
            "| lift platforms | 100.0 % | 100.0 % |",
            // A lift-free corpus states its zeros too, for the same reason.
            "| maps with a lift line | 0 (0.0 % of unique maps) |",
            "| lifts / pedestals / barriers | 0 / 0 / 0 |",
            "## Floors",
            "| floor targets | 100.0 % | 100.0 % |",
            // And so does a floor-free one, the sixth axis included.
            "| maps with a floor line | 0 (0.0 % of unique maps) |",
            "| drop walls / reveals / bridges | 0 / 0 / 0 |",
            "## Greedy curve",
            "| 97 | 1 | 100.0 % |",
            // The conjunction curve names its denominator, so a plateau
            // below 100 % reads as the sector-/thing-blocked remainder.
            "Share is of **all unique maps**",
        ] {
            assert!(md.contains(needle), "missing {needle:?}:\n{md}");
        }
        // Sector and thing axes have no blockers here: an empty axis states
        // so in a row rather than leaving a bare table header.
        assert_eq!(md.matches("| (none) | | |").count(), 2, "{md}");
        assert!(!md.contains("## Sample"));
    }

    /// 26 maps each blocked on its own special make a 26-step curve: the order
    /// line prints the first `BLOCKER_TOP` and says how many it cut. A 25-step
    /// sibling prints every step and no suffix.
    #[test]
    fn a_long_greedy_order_is_cut_at_blocker_top_and_says_so() {
        let curve = |n: i32| {
            let maps: Vec<_> = (1..=n)
                .map(|s| record(&[s], &[s], true, true, true))
                .collect();
            render_markdown(None, &Buckets::default(), &aggregate(&maps))
        };
        let long = curve(26);
        assert!(long.contains("→ 25 → … (first 25 of 26 steps)\n"), "{long}");
        let exact = curve(25);
        assert!(exact.contains("→ 25\n"), "{exact}");
        assert!(!exact.contains("…"), "{exact}");
        assert!(!exact.contains("first 25"), "{exact}");
    }

    /// A map whose only teleport line is refused is not expressible, and the
    /// aggregate counts it in both the "with" and the "refused" populations.
    #[test]
    fn a_refused_line_is_aggregated_and_rendered() {
        let mut m = record(&[97], &[], true, true, true);
        m.teleports = TeleportCounts {
            lines: 2,
            player: 2,
            broken: 1,
            self_referencing: 1,
            closet: 1,
            exit: 1,
            paired: 1,
            ..TeleportCounts::default()
        };
        m.verdict.teleports_ok = false;
        m.verdict.expressible = false;
        let a = aggregate(std::slice::from_ref(&m));
        assert_eq!(a.teleports.maps_with_teleports, 1);
        assert_eq!(a.teleports.maps_refused, 1);
        assert_eq!(a.teleports.lines.broken, 1);
        assert_eq!(a.teleports.closet_maps, 1);
        assert_eq!(a.teleports.exit_maps, 1);
        assert_eq!(a.teleports.paired_maps, 1);
        assert!(a.all.teleports.abs() < 1e-9);
        let md = render_markdown(None, &Buckets::default(), &a);
        assert!(
            md.contains("| maps with a refused line (not expressible) | 1 |"),
            "{md}"
        );
        assert!(
            md.contains("| lines on a paired pad | 1 (1 map) |"),
            "a single map is not \"1 maps\": {md}"
        );
        assert!(
            md.contains("| ambiguous (several markers) / broken / self-referencing | 0 / 1 / 1 |"),
            "{md}"
        );
    }

    /// A map whose only platform is refused is not expressible, and the
    /// aggregate counts it in both the "with" and the "refused" populations.
    #[test]
    fn a_refused_plat_is_aggregated_and_rendered() {
        let mut m = record(&[62], &[], true, true, true);
        m.lifts = PlatCounts {
            plats: 2,
            lifts: 1,
            refused: 1,
            dead: 1,
            broken_lines: 1,
            callable_low: 2,
            with_top_trigger: 1,
            with_things: 1,
            fast: 1,
            ..PlatCounts::default()
        };
        m.verdict.lifts_ok = false;
        m.verdict.expressible = false;
        let a = aggregate(std::slice::from_ref(&m));
        assert_eq!(a.lifts.maps_with_lifts, 1);
        assert_eq!(a.lifts.maps_refused, 1);
        assert_eq!(a.lifts.plats.dead, 1);
        assert_eq!(a.lifts.lift_maps, 1);
        assert_eq!((a.lifts.pedestal_maps, a.lifts.barrier_maps), (0, 0));
        assert!(a.all.lifts.abs() < 1e-9);
        let md = render_markdown(None, &Buckets::default(), &a);
        assert!(
            md.contains("| maps with a refused plat or line (not expressible) | 1 |"),
            "{md}"
        );
        assert!(
            md.contains(
                "| refused: dead / one-shot / mixed speed / unsupported rest / top-only / \
                         one-way barrier / bank caller / conflicting action | 1 / 0 / 0 / 0 \
                         / 0 / 0 / 0 / 0 |"
            ),
            "{md}"
        );
        assert!(md.contains("| broken lines | 1 |"), "{md}");
        assert!(
            md.contains("| callable from below | 2 (100.0 % of plats) |"),
            "{md}"
        );
    }

    /// The split-group row names both the groups and the sectors they
    /// comprise, so a report before the merge reconciles with one after it.
    #[test]
    fn a_split_group_is_rendered_as_groups_and_sectors() {
        let mut m = record(&[62], &[], true, true, true);
        m.lifts = PlatCounts {
            plats: 1,
            lifts: 1,
            shared_split: 1,
            split_members: 2,
            callable_low: 1,
            ..PlatCounts::default()
        };
        let a = aggregate(&[m]);
        let md = render_markdown(None, &Buckets::default(), &a);
        assert!(
            md.contains("| split groups read as one platform / sectors they comprise | 1 / 2 |"),
            "{md}"
        );
        assert!(
            !md.contains("shared-tag groups that are one platform split"),
            "{md}"
        );
    }

    /// A map with a lift line that names no platform at all still counts as
    /// carrying lifts: the broken line is the whole refusal.
    #[test]
    fn a_map_whose_only_lift_line_is_broken_is_counted_as_carrying_lifts() {
        let mut m = record(&[62], &[], true, true, true);
        m.lifts = PlatCounts {
            broken_lines: 1,
            ..PlatCounts::default()
        };
        m.verdict.lifts_ok = false;
        m.verdict.expressible = false;
        let a = aggregate(std::slice::from_ref(&m));
        assert_eq!(a.lifts.maps_with_lifts, 1, "no plat, but a lift line");
        assert_eq!(a.lifts.maps_refused, 1);
        let md = render_markdown(None, &Buckets::default(), &a);
        assert!(md.contains("| plats | 0 |"), "{md}");
        // No plat: the share row states a zero denominator as 0.0 %, not NaN.
        assert!(
            md.contains("| callable from below | 0 (0.0 % of plats) |"),
            "{md}"
        );
    }

    /// A floor target is aggregated and rendered — the minimal end of the
    /// end-to-end path Task 13's golden-backed `tests/corpus_cli.rs` test
    /// completes; this unit test pins the shapes row without a swept WAD.
    #[test]
    fn a_floor_target_is_aggregated_and_rendered() {
        let mut m = record(&[], &[], true, true, true);
        m.floors = FloorCounts {
            targets: 1,
            drop_walls: 1,
            ..FloorCounts::default()
        };
        let a = aggregate(std::slice::from_ref(&m));
        assert_eq!(a.floors.maps_with_floors, 1);
        let md = render_markdown(None, &Buckets::default(), &a);
        assert!(md.contains("## Floors"), "{md}");
        assert!(
            md.contains("| drop walls / reveals / bridges | 1 / 0 / 0 |"),
            "{md}"
        );
    }

    /// The baseline row carries the curve's own standing share, so the two
    /// curves differ when a map is blocked on another axis — a swapped or
    /// hard-coded baseline fails here.
    #[test]
    fn a_stepless_curve_prints_the_standing_share_as_its_baseline() {
        let a = aggregate(&[
            record(&[], &[], true, true, true),
            record(&[], &[], false, true, true),
        ]);
        let md = render_markdown(None, &Buckets::default(), &a);
        // The line-axis curve: nothing is blocked on lines.
        assert!(md.contains("| 0 | 100.0 % |"), "{md}");
        // The conjunction curve: the sector-blocked map is not expressible.
        assert!(md.contains("| 0 | 50.0 % |"), "{md}");
        assert_eq!(md.matches("Order chosen: (none)").count(), 2, "{md}");
    }

    #[test]
    fn provenance_reads_a_sample_manifest_and_sorts_ids() {
        let dir = std::env::temp_dir().join(format!("crustygen-prov-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("sample-manifest.json"),
            r#"{"seed":42,"count":2,"frame_rows":9,"fetch_list_hash":"blake3:00","entries":[{"id":7,"dir":"d/","filename":"b.zip","zip_size":1,"status":"ok"},{"id":3,"dir":"d/","filename":"a.zip","zip_size":1,"status":"failed:x"}]}"#,
        )
        .unwrap();
        let p = read_provenance(&dir).expect("parses");
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(p.ids, vec![3, 7]);
        assert_eq!(p.seed, 42);
        let md = render_markdown(Some(&p), &Buckets::default(), &aggregate(&[]));
        assert!(md.contains("ids: 3 7"), "{md}");
        // An empty aggregate leaves both greedy curves stepless: each gets a
        // single baseline row rather than a table header with nothing under it.
        assert_eq!(md.matches("| 0 | 0.0 % |").count(), 2, "{md}");
        assert_eq!(md.matches("Order chosen: (none)").count(), 2, "{md}");
        assert_eq!(read_provenance(Path::new("/nonexistent-dir")), None);

        // Present but malformed: refused, and named on stderr.
        let bad = std::env::temp_dir().join(format!(
            "crustygen-prov-bad-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join("sample-manifest.json"), b"{ not json").unwrap();
        let got = read_provenance(&bad);
        std::fs::remove_dir_all(&bad).ok();
        assert_eq!(got, None);
    }
}
