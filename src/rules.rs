//! Playability invariants checked against compiled output.
//!
//! `check_all` implements a deliberately partial rule catalog: **P3** (passage
//! width), **P4** (door opening clearance), **P7** (no softlock), **P8** (no
//! missing textures), **P9** (no texture scaling), **P19** (light bounds), and
//! **P24** (key and lock coherence). **P7** floods `(sector, keys-held)`
//! states over the emitted geometry — see [`crate::reach`].
//!
//! **P1** (step height between connected rooms) has been **retired**: it
//! capped the floor delta between connected rooms in either direction, but
//! `P_TryMove` caps only the climb and leaves falling unrestricted, and a
//! corpus sweep found the majority of vanilla Doom's height-changing
//! boundaries — 56.92% of them — exceeding it. See
//! [`CompileError::PortalNoHeadroom`](crate::compile::CompileError::PortalNoHeadroom)
//! for what replaced it.
//!
//! **P20** (pickup accessibility) is deliberately *not* implemented here. It
//! needs the same key-aware reachability flood P7 runs, applied to every
//! pickup rather than to the exit, so it will consume [`crate::reach`] rather
//! than re-derive anything — but which pickups a map *must* make reachable is
//! a spec-conformance question this stage-one structural pass does not yet
//! answer. Do not read the presence of this module as covering it.

use crate::compile::Compiled;
use crate::compile::heights::{visible_lower_side, visible_upper_side};
use crate::ir::{Ir, PortalKind};
use crate::reach;
use crate::tables::Tables;

/// One failed playability check.
#[derive(Debug, Clone)]
pub struct RuleViolation {
    /// The rule identifier, e.g. `"P4"`.
    pub rule: &'static str,
    /// What failed — a room id, portal, or line index.
    pub subject: String,
    /// Human-readable detail, including the threshold and the actual value.
    pub detail: String,
}

impl std::fmt::Display for RuleViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({}): {}", self.rule, self.subject, self.detail)
    }
}

/// Runs every stage-one playability check and returns all violations.
///
/// Violations are returned rather than raised so the conformance report can
/// list all of them at once instead of only the first.
/// [`crate::compile::compile`] calls this itself and turns a non-empty result
/// into [`crate::compile::CompileError::Playability`], so a violation is a
/// hard error for anyone compiling a map; use
/// [`crate::compile::compile_reporting`] to get the list without the failure.
#[must_use]
pub fn check_all(ir: &Ir, tables: &Tables, out: &Compiled) -> Vec<RuleViolation> {
    let mut v = Vec::new();
    check_door_clearance(ir, tables, &mut v);
    check_passage_width(ir, tables, &mut v);
    check_light_bounds(ir, tables, &mut v);
    check_no_scaling(out, &mut v);
    check_missing_textures(out, &mut v);
    check_key_lock_coherence(ir, &mut v);
    check_reachability(ir, tables, out, &mut v);
    v
}

/// P7: no softlock — the map is finishable, nowhere the player can get to
/// is a dead end they cannot finish from, and every sector is visitable.
///
/// Delegates to [`crate::reach`]: a breadth-first search over
/// `(sector, keys-held)` states built from the *emitted* geometry. Vacuously
/// satisfied when the map has no player 1 start or no exit — see
/// [`crate::reach::graph_from_compiled`].
fn check_reachability(ir: &Ir, tables: &Tables, out: &Compiled, v: &mut Vec<RuleViolation>) {
    let Some(built) = reach::graph_from_compiled(ir, tables, out) else {
        return;
    };
    let limits = reach::Limits {
        player_height: tables.player().height,
        max_step: tables.step_height(),
    };
    let findings = reach::check(&built.graph, &limits);

    let held = |mask: reach::KeyMask| -> String {
        let names: Vec<String> = built
            .class_names
            .iter()
            .enumerate()
            .filter(|&(c, _)| mask & (1 << c) != 0)
            .map(|(_, kinds)| kinds.join("/"))
            .collect();
        if names.is_empty() {
            String::new()
        } else {
            format!(" holding `{}`", names.join(", "))
        }
    };

    if findings.unfinishable {
        v.push(RuleViolation {
            rule: "P7",
            subject: reach::node_label(built.graph.start, ir, &built.graph),
            detail: "no feasible walk from the player start reaches an exit".to_owned(),
        });
    }
    for &(node, mask) in &findings.stranded {
        // When nothing can finish, every visited state is trivially doomed;
        // naming them all would bury the signal. The key-collecting sectors
        // are the likely culprits (the shipped defect was exactly one), so
        // only they are named in that case.
        if findings.unfinishable && built.graph.nodes[node].keys == 0 {
            continue;
        }
        v.push(RuleViolation {
            rule: "P7",
            subject: reach::node_label(node, ir, &built.graph),
            detail: format!(
                "the player can reach this sector{} but can no longer reach an exit from it",
                held(mask)
            ),
        });
    }
    for &node in &findings.unreachable {
        v.push(RuleViolation {
            rule: "P7",
            subject: reach::node_label(node, ir, &built.graph),
            detail: "can never be visited from the player start".to_owned(),
        });
    }
}

/// P4: a door's opening must clear the player.
///
/// P1 — "connected rooms must not differ by more than one step" — used to
/// share this loop and has been retired. It capped the floor delta in either
/// direction, but `P_TryMove` caps only the climb, and a corpus sweep of
/// DOOM, DOOM2, TNT, and PLUTONIA found 37.77% of passable two-sided lines
/// over that cap, 62.5% of them permanent static drops. The degeneracy it
/// incidentally prevented is now caught by
/// [`CompileError::PortalNoHeadroom`](crate::compile::CompileError::PortalNoHeadroom),
/// at compile time and on the passage sector itself.
fn check_door_clearance(ir: &Ir, tables: &Tables, v: &mut Vec<RuleViolation>) {
    let player = tables.player();
    for p in &ir.portals {
        if !matches!(p.kind, PortalKind::Door | PortalKind::Locked) {
            continue;
        }
        let (Some(a), Some(b)) = (ir.room(&p.a), ir.room(&p.b)) else {
            continue;
        };
        // A door's open ceiling stops short of the lowest neighboring
        // ceiling by the engine's clearance allowance (P_DoorDoor:
        // `topheight = P_FindLowestCeilingSurrounding(sec) - 4`), so the
        // usable opening is smaller than the nominal room height. The
        // clearance is measured from the *higher* of the two floors (`max`,
        // not the door sector's own carved floor, which is the lower of the
        // two): a player standing on the higher-floor side has less headroom
        // to the door's open ceiling than one on the lower-floor side, so the
        // higher floor is the binding constraint.
        let opening =
            a.ceiling.min(b.ceiling) - tables.door_clearance_allowance() - a.floor.max(b.floor);
        if opening < player.height {
            v.push(RuleViolation {
                rule: "P4",
                subject: format!("{} <-> {}", p.a, p.b),
                detail: format!(
                    "door opening {opening} is below player height {}",
                    player.height
                ),
            });
        }
    }
}

/// P3: a passage must admit everything required to pass through it.
fn check_passage_width(ir: &Ir, tables: &Tables, v: &mut Vec<RuleViolation>) {
    let need = tables.player().radius * 2;
    for p in &ir.portals {
        if p.width < need {
            v.push(RuleViolation {
                rule: "P3",
                subject: format!("{} <-> {}", p.a, p.b),
                detail: format!(
                    "opening {} is narrower than the {need} the player needs",
                    p.width
                ),
            });
        }
    }
}

/// P19: every light level lies inside the engine's valid range.
fn check_light_bounds(ir: &Ir, tables: &Tables, v: &mut Vec<RuleViolation>) {
    let range = tables.light_range();
    for room in &ir.rooms {
        if !range.contains(&room.light) {
            v.push(RuleViolation {
                rule: "P19",
                subject: room.id.clone(),
                detail: format!(
                    "light level {} is outside {}..={}",
                    room.light,
                    range.start(),
                    range.end()
                ),
            });
        }
    }
}

/// P9: no emitted surface carries a scale factor.
fn check_no_scaling(out: &Compiled, v: &mut Vec<RuleViolation>) {
    if out.textmap.contains("scalex") || out.textmap.contains("scaley") {
        v.push(RuleViolation {
            rule: "P9",
            subject: "TEXTMAP".to_owned(),
            detail: "emitted output contains a texture scale factor".to_owned(),
        });
    }
}

/// P8: one-sided lines need a middle texture, and two-sided lines need an
/// upper or lower wherever the sectors' ceilings or floors differ.
fn check_missing_textures(out: &Compiled, v: &mut Vec<RuleViolation>) {
    for (i, l) in out.data.linedefs.iter().enumerate() {
        let front = &out.data.sidedefs[l.front];
        let Some(back_idx) = l.back else {
            if front.middle.is_empty() {
                v.push(RuleViolation {
                    rule: "P8",
                    subject: format!("linedef {i}"),
                    detail: "one-sided line has no middle texture".to_owned(),
                });
            }
            continue;
        };
        let back = &out.data.sidedefs[back_idx];
        let front_sector = &out.data.sectors[front.sector];
        let back_sector = &out.data.sectors[back.sector];
        // Which side the engine draws is decided in exactly one place —
        // `heights::visible_lower_side`/`visible_upper_side`, the same
        // functions `heights::apply_height_textures` calls to fill the
        // texture in the first place — so the pass that fills a texture and
        // the rule that requires one cannot independently drift on which
        // side is right. Requiring the other side as well would reject the
        // overwhelming majority of vanilla Doom's own boundaries — measured
        // at 89.5% across DOOM, DOOM2, TNT, and PLUTONIA.
        if let Some(visible) =
            visible_lower_side(front_sector.floor, back_sector.floor, l.front, back_idx)
            && out.data.sidedefs[visible].lower.is_empty()
        {
            v.push(RuleViolation {
                rule: "P8",
                subject: format!("linedef {i}"),
                detail: format!(
                    "floors differ ({} vs {}) but the lower side has no lower texture",
                    front_sector.floor, back_sector.floor
                ),
            });
        }
        if let Some(visible) =
            visible_upper_side(front_sector.ceiling, back_sector.ceiling, l.front, back_idx)
            && out.data.sidedefs[visible].upper.is_empty()
        {
            v.push(RuleViolation {
                rule: "P8",
                subject: format!("linedef {i}"),
                detail: format!(
                    "ceilings differ ({} vs {}) but the higher-ceiling side has no upper texture",
                    front_sector.ceiling, back_sector.ceiling
                ),
            });
        }
    }
}

/// P24: every locked door's key is placed somewhere, and every placed key
/// opens something.
fn check_key_lock_coherence(ir: &Ir, v: &mut Vec<RuleViolation>) {
    let placed: Vec<&str> = ir
        .rooms
        .iter()
        .flat_map(|r| r.things.iter().map(|t| t.kind.as_str()))
        .collect();

    for p in &ir.portals {
        if let Some(lock) = &p.lock
            && !placed.contains(&lock.as_str())
        {
            v.push(RuleViolation {
                rule: "P24",
                subject: format!("{} <-> {}", p.a, p.b),
                detail: format!("locked by `{lock}`, which is never placed"),
            });
        }
    }

    for key in placed
        .iter()
        .filter(|k| k.ends_with("_card") || k.ends_with("_skull"))
    {
        if !ir.portals.iter().any(|p| p.lock.as_deref() == Some(*key)) {
            v.push(RuleViolation {
                rule: "P24",
                subject: (*key).to_owned(),
                detail: "key is placed but opens no door".to_owned(),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compile::{CompileError, compile, compile_reporting};
    use crate::ir::Ir;
    use crate::rules::{RuleViolation, check_all};
    use crate::tables::Tables;

    /// Two rooms joined by a plain portal, with tunable floors, width, and
    /// light.
    fn ir(floor_b: i32, width: i32, light: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":{light},
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                   "things":[{{ "kind":"player1_start", "at":[128,128], "angle":90 }}] }},
                {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                   "floor":{floor_b}, "ceiling":{}, "light":{light},
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":{width}, "at":[256,128] }}] }}"#,
            floor_b + 128
        )
    }

    /// Two rooms joined by a door portal, with independently tunable
    /// ceilings so the P4 boundary can be pinned without perturbing P8
    /// (both floors are 0, so no floor difference is ever introduced) or P3
    /// (width is well above the player's diameter).
    fn door_ir(ceiling_a: i32, ceiling_b: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":{ceiling_a}, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                   "things":[{{ "kind":"player1_start", "at":[128,128], "angle":90 }}] }},
                {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                   "floor":0, "ceiling":{ceiling_b}, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                            "door_thickness":32, "alcove_near":16, "alcove_far":16 }}] }}"#
        )
    }

    /// Two rooms joined by a plain portal, with room `b`'s floor and ceiling
    /// tunable *independently*. Unlike `ir`, which derives `ceiling_b` from
    /// `floor_b` (so the two always move together), this lets a P8 test
    /// isolate the floor-difference branch from the ceiling-difference
    /// branch. That isolation matters: with both differing at once, a
    /// mutation that breaks only one branch can still pass, because the
    /// other branch independently reports a P8 violation and masks it — this
    /// was observed directly during the mutation pass on `check_missing_textures`
    /// (see the task-11 report).
    fn portal_ir(floor_b: i32, ceiling_b: i32, width: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                   "things":[{{ "kind":"player1_start", "at":[128,128], "angle":90 }}] }},
                {{ "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                   "floor":{floor_b}, "ceiling":{ceiling_b}, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }}
              ],
              "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":{width}, "at":[256,128] }}] }}"#
        )
    }

    /// The shipped `key_room` defect, reduced to three rooms: the only key
    /// sits in a dead-end pit `pit_floor` below the hub, and the exit is
    /// behind the blue door. At -32 the pit is one-way (`P_TryMove` caps the
    /// climb at 24) and the map is unfinishable; at -16 every drop reverses.
    fn key_pit_ir(pit_floor: i32) -> String {
        format!(
            r#"{{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                {{ "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                   "things":[{{ "kind":"player1_start", "at":[128,128], "angle":90 }}] }},
                {{ "id":"pit", "footprint":[[320,0],[320,256],[576,256],[576,0]],
                   "floor":{pit_floor}, "ceiling":128, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                   "things":[{{ "kind":"blue_card", "at":[448,128], "angle":0 }}] }},
                {{ "id":"vault", "footprint":[[0,320],[0,576],[256,576],[256,320]],
                   "floor":0, "ceiling":128, "light":160,
                   "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }}
              ],
              "portals":[
                {{ "a":"hub", "b":"pit", "kind":"plain", "width":64, "at":[256,128] }},
                {{ "a":"hub", "b":"vault", "kind":"locked", "lock":"blue_card",
                   "width":128, "at":[128,256],
                   "door_thickness":32, "alcove_near":16, "alcove_far":16 }}
              ],
              "exits":[{{ "room":"vault", "trigger":"switch", "width":32, "at":[128,576] }}] }}"#
        )
    }

    fn p7_violations(json: &str) -> Vec<RuleViolation> {
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (_, violations) = compile_reporting(&ir, &tables).expect("compile");
        violations.into_iter().filter(|v| v.rule == "P7").collect()
    }

    /// The rule ids a fixture violates.
    ///
    /// Goes through `compile_reporting` rather than `compile`: `compile`
    /// turns any violation into `CompileError::Playability`, which is the
    /// point of these rules, but a test that has to distinguish *which* rule
    /// fired needs the geometry compiled and the list returned.
    fn violations(json: &str) -> Vec<String> {
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (_, found) = compile_reporting(&ir, &tables).expect("compiles");
        found.into_iter().map(|v| v.rule.to_owned()).collect()
    }

    #[test]
    fn compile_refuses_a_map_that_breaks_a_playability_rule() {
        // The spec makes playability violations hard errors: "a door the
        // player cannot fit through is a broken map, not a missed target".
        // `compile` never ran these checks, so every rule in this module was
        // inert unless a caller remembered to invoke `check_all` itself.
        let tables = Tables::load().expect("tables");
        let narrow = ir(0, tables.player().radius * 2 - 2, 160);
        let parsed = Ir::from_json(&narrow).expect("ir");
        let err = compile(&parsed, &tables).expect_err("a P3 violation must fail the compile");
        let CompileError::Playability { violations } = err else {
            panic!("expected a playability failure, got {err}");
        };
        assert!(violations.iter().any(|v| v.rule == "P3"));
        // The error carries every violation, so an author can fix them in
        // one pass rather than one recompile each.
        assert!(
            format!(
                "{}",
                CompileError::Playability {
                    violations: violations.clone()
                }
            )
            .contains("P3"),
            "the message names the rules it collected"
        );
    }

    #[test]
    fn compile_reporting_returns_the_map_alongside_its_violations() {
        let tables = Tables::load().expect("tables");
        let narrow = ir(0, tables.player().radius * 2 - 2, 160);
        let parsed = Ir::from_json(&narrow).expect("ir");
        let (out, found) = compile_reporting(&parsed, &tables).expect("geometry still compiles");
        assert!(!out.textmap.is_empty(), "the map is still emitted");
        assert!(found.iter().any(|v| v.rule == "P3"));
    }

    #[test]
    fn compile_accepts_a_map_that_breaks_no_rule() {
        let tables = Tables::load().expect("tables");
        let parsed = Ir::from_json(&ir(0, 128, 160)).expect("ir");
        assert!(compile(&parsed, &tables).is_ok());
    }

    #[test]
    fn a_large_drop_compiles_now_that_the_step_cap_is_gone() {
        // 128 units down: the player walks off a ledge. `P_TryMove` caps the
        // climb, not the fall, and 62.5% of the corpus's over-step lines are
        // permanent static drops exactly like this one.
        let violations = violations(&portal_ir(-128, 128, 128));
        assert!(
            violations.is_empty(),
            "a one-way drop is legal Doom, got {violations:?}"
        );
    }

    #[test]
    fn a_drop_far_inside_the_16_bit_range_compiles() {
        // The rejection half of the range guard is already pinned in `ir.rs`;
        // until the step cap was retired, no positive case could reach it.
        let violations = violations(&portal_ir(-30000, 128, 128));
        assert!(violations.is_empty(), "got {violations:?}");
    }

    #[test]
    fn rooms_that_do_not_overlap_vertically_are_rejected() {
        // Room b floats entirely above room a, so the passage sector between
        // them would take floor 400 and ceiling 128 — a sector whose floor
        // is above its own ceiling.
        let ir = Ir::from_json(&portal_ir(400, 512, 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let err = compile(&ir, &tables).expect_err("an inverted passage must be rejected");
        let CompileError::PortalNoHeadroom { have, need, .. } = err else {
            panic!("expected PortalNoHeadroom, got {err}");
        };
        assert_eq!(have, -272);
        assert_eq!(need, tables.player().height);
    }

    #[test]
    fn the_passage_headroom_boundary_is_exact() {
        let tables = Tables::load().expect("tables");
        let need = tables.player().height;
        // Room a is 0..=128, so a room b floored at `128 - need` leaves
        // exactly `need` units of overlap, and one unit higher leaves one
        // too few.
        let exact = Ir::from_json(&portal_ir(128 - need, 512, 128)).expect("ir");
        compile(&exact, &tables).expect("exactly enough headroom compiles");

        let short = Ir::from_json(&portal_ir(128 - need + 1, 512, 128)).expect("ir");
        let err = compile(&short, &tables).expect_err("one unit short must be rejected");
        assert!(matches!(
            err,
            CompileError::PortalNoHeadroom { have, .. } if have == need - 1
        ));
    }

    #[test]
    fn p3_a_passage_at_the_player_diameter_passes_and_one_step_under_fails() {
        let tables = Tables::load().expect("tables");
        let need = tables.player().radius * 2;
        assert!(!violations(&ir(0, need, 160)).contains(&"P3".to_owned()));
        // `need` is a doubled radius and therefore always even, and
        // `Ir::from_json` rejects odd widths outright (they cannot be
        // centered on `at` in whole units), so the nearest expressible width
        // below the threshold is two units under, not one. That still pins
        // the `<` boundary: it is the largest legal value that must fail.
        assert!(violations(&ir(0, need - 2, 160)).contains(&"P3".to_owned()));
    }

    #[test]
    fn p4_a_door_opening_at_the_player_height_passes_and_one_unit_under_fails() {
        let tables = Tables::load().expect("tables");
        // The door opens to `min(ceiling) - clearance_allowance`, measured
        // above the higher of the two floors (both 0 here). Pin ceilings so
        // that value lands exactly at, then one unit under, player height.
        let need = tables.player().height + tables.door_clearance_allowance();
        assert!(!violations(&door_ir(need, need)).contains(&"P4".to_owned()));
        assert!(violations(&door_ir(need - 1, need - 1)).contains(&"P4".to_owned()));
    }

    #[test]
    fn p19_light_at_the_engine_max_passes_and_one_unit_over_fails() {
        let tables = Tables::load().expect("tables");
        let max = *tables.light_range().end();
        assert!(!violations(&ir(0, 128, max)).contains(&"P19".to_owned()));
        assert!(violations(&ir(0, 128, max + 1)).contains(&"P19".to_owned()));
    }

    #[test]
    fn p19_light_at_the_engine_min_passes_and_one_unit_under_fails() {
        let tables = Tables::load().expect("tables");
        let min = *tables.light_range().start();
        assert!(!violations(&ir(0, 128, min)).contains(&"P19".to_owned()));
        assert!(violations(&ir(0, 128, min - 1)).contains(&"P19".to_owned()));
    }

    #[test]
    fn p9_compiled_output_never_carries_scaling() {
        assert!(!violations(&ir(0, 128, 160)).contains(&"P9".to_owned()));
    }

    /// The (front, back) sidedef indices of the linedef joining room `a`
    /// (sector 0) and the passage `portal_ir`'s single plain portal emits
    /// (sector 2). Fixed by construction, not recomputed: `cut_portals`
    /// always calls `emit_opening` with the *room* as `sector_a`, and
    /// `emit_opening` always makes `sector_a` the linedef's front regardless
    /// of orientation — so room `a`'s own sidedef is always `front` here.
    /// Verified directly against `compile_reporting`'s output for this exact
    /// fixture before being hard-coded (see the task-2 fix report).
    fn room_a_passage_boundary(out: &crate::compile::Compiled) -> (usize, usize) {
        out.data
            .linedefs
            .iter()
            .filter_map(|l| l.back.map(|b| (l.front, b)))
            .find(|(f, b)| out.data.sidedefs[*f].sector == 0 && out.data.sidedefs[*b].sector == 2)
            .expect("room a borders the passage sector directly")
    }

    /// The mirror of `room_a_passage_boundary` for the *far* threshold: the
    /// (front, back) sidedef indices of the linedef joining room `b`
    /// (sector 1) and the passage (sector 2). Also fixed by construction:
    /// `compile::portals::emit_segment`'s far threshold always calls
    /// `emit_opening` with the far room as `sector_a`, and `emit_opening`
    /// always makes `sector_a` the linedef's front — so room `b`'s own
    /// sidedef is always `front` here, regardless of orientation.
    fn room_b_passage_boundary(out: &crate::compile::Compiled) -> (usize, usize) {
        out.data
            .linedefs
            .iter()
            .filter_map(|l| l.back.map(|b| (l.front, b)))
            .find(|(f, b)| out.data.sidedefs[*f].sector == 1 && out.data.sidedefs[*b].sector == 2)
            .expect("room b borders the passage sector directly")
    }

    #[test]
    fn p8_fires_when_the_drawn_side_loses_its_lower_texture() {
        // The compiler now fills this in, so the rule can only be exercised
        // by taking it back out — which is also the mutation proof that the
        // rule is watching the side the renderer actually samples. Which
        // side is "drawn" is a fixed expectation of this fixture
        // (`room_a_passage_boundary`'s doc comment), not something
        // recomputed here — a test that re-derived the visibility rule
        // could not detect the rule itself changing.
        let ir = Ir::from_json(&portal_ir(16, 128, 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let (mut out, found) = compile_reporting(&ir, &tables).expect("compiles");
        assert!(
            !found.iter().any(|v| v.rule == "P8"),
            "a compiled height difference is textured, so P8 is quiet"
        );

        // Room a's floor (0) is below the passage's (16), so room a's own
        // sidedef — the boundary's front, by construction — is the drawn
        // side.
        let (front, _back) = room_a_passage_boundary(&out);
        out.data.sidedefs[front].lower.clear();

        let violations = check_all(&ir, &tables, &out);
        assert!(violations.iter().any(|v| v.rule == "P8"));
    }

    #[test]
    fn p8_ignores_a_bare_side_the_renderer_never_samples() {
        // The other half of the same rule: vanilla leaves the unsampled side
        // bare 89.5% of the time, so a bare hidden side must not be a
        // violation. A rule that still demanded both sides passes the test
        // above and fails this one. As above, which side is hidden (the
        // passage's own sidedef, the boundary's back) is a fixed
        // expectation, not recomputed.
        let ir = Ir::from_json(&portal_ir(16, 128, 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, _) = compile_reporting(&ir, &tables).expect("compiles");
        let (_front, back) = room_a_passage_boundary(&out);
        assert!(
            out.data.sidedefs[back].lower.is_empty(),
            "the unsampled side is left bare, and that is legal"
        );
        assert!(check_all(&ir, &tables, &out).iter().all(|v| v.rule != "P8"));
    }

    #[test]
    fn p8_fires_when_the_drawn_side_loses_its_upper_texture() {
        // The mirror of `p8_fires_when_the_drawn_side_loses_its_lower_texture`,
        // isolating the *ceiling* branch: this was the gap a mutation pass
        // found in `check_missing_textures` — deleting its upper branch
        // entirely left all tests green, because no test cleared a filled
        // upper and re-checked P8. `portal_ir`'s floors are equal here (both
        // 0), so only the ceiling branch can fire, mirroring how
        // `p8_fires_when_the_drawn_side_loses_its_lower_texture` isolates the
        // floor branch by holding the ceilings equal instead.
        let ir = Ir::from_json(&portal_ir(0, 160, 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let (mut out, found) = compile_reporting(&ir, &tables).expect("compiles");
        assert!(
            !found.iter().any(|v| v.rule == "P8"),
            "a compiled ceiling difference is textured, so P8 is quiet"
        );

        // Room b's ceiling (160) is above the passage's (128, the min of
        // the two), so room b's own sidedef — the far boundary's front, by
        // construction — is the drawn side.
        let (front, _back) = room_b_passage_boundary(&out);
        out.data.sidedefs[front].upper.clear();

        let violations = check_all(&ir, &tables, &out);
        assert!(violations.iter().any(|v| v.rule == "P8"));
    }

    #[test]
    fn p8_ignores_a_bare_upper_the_renderer_never_samples() {
        // The other half of the ceiling branch: vanilla leaves the
        // unsampled side bare 89.5% of the time (see
        // `check_missing_textures`'s own doc comment), so a bare hidden
        // upper must not be a violation either. As above, which side is
        // hidden (the passage's own sidedef, the far boundary's back) is a
        // fixed expectation, not recomputed.
        let ir = Ir::from_json(&portal_ir(0, 160, 128)).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, _) = compile_reporting(&ir, &tables).expect("compiles");
        let (_front, back) = room_b_passage_boundary(&out);
        assert!(
            out.data.sidedefs[back].upper.is_empty(),
            "the unsampled side is left bare, and that is legal"
        );
        assert!(check_all(&ir, &tables, &out).iter().all(|v| v.rule != "P8"));
    }

    #[test]
    fn p8_a_one_sided_line_without_a_middle_texture_fails() {
        // A portal-less room is entirely one-sided. `emit_sectors` copies
        // `wall_tex` onto every one-sided line's middle texture verbatim, so
        // an empty `wall_tex` leaves every wall bare, isolating the
        // one-sided branch from the two (two-sided) branches above.
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[{ "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
            "floor":0, "ceiling":128, "light":160,
            "floor_tex":"F", "ceil_tex":"C", "wall_tex":"",
            "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] }],
          "portals":[] }"#;
        assert!(violations(ir_json).contains(&"P8".to_owned()));
    }

    #[test]
    fn p24_a_locked_door_naming_an_unplaced_key_fails() {
        let locked = ir(0, 128, 160).replace(
            "\"kind\":\"plain\"",
            "\"kind\":\"locked\", \"lock\":\"blue_card\", \"door_thickness\":32, \
             \"alcove_near\":16, \"alcove_far\":16",
        );
        assert!(violations(&locked).contains(&"P24".to_owned()));
    }

    // Both halves of P24 now run through the public path: the vocabulary
    // lists every key thing, so an IR that *places* one compiles instead of
    // being rejected as an unknown thing, and a locked portal gets a real
    // keyed door special. Until those tables existed, no locked-door
    // progression was constructible end to end and these two tests had to
    // call the private helper directly.
    #[test]
    fn p24_a_placed_key_that_opens_no_door_is_flagged() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
              "things":[
                { "kind":"player1_start", "at":[128,128], "angle":90 },
                { "kind":"blue_card", "at":[64,64], "angle":0 }
              ] }
          ],
          "portals":[] }"#;
        assert!(violations(ir_json).contains(&"P24".to_owned()));
    }

    #[test]
    fn p24_a_locked_door_whose_key_is_placed_is_coherent() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W",
              "things":[
                { "kind":"player1_start", "at":[128,128], "angle":90 },
                { "kind":"blue_card", "at":[64,64], "angle":0 }
              ] },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"F", "ceil_tex":"C", "wall_tex":"W" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"locked", "lock":"blue_card", "width":128, "at":[256,128],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;
        // The whole loop: a locked door, the key that opens it placed in a
        // reachable room, and a compile that succeeds because nothing is
        // violated. This is the smallest map that proves key progression is
        // constructible at all.
        let parsed = Ir::from_json(ir_json).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&parsed, &tables).expect("a coherent locked door compiles");
        let keyed = tables
            .locked_door_special("blue_card")
            .expect("blue_card special");
        assert!(
            out.data.linedefs.iter().any(|l| l.special == keyed),
            "the locked door carries blue_card's keyed special"
        );
    }

    #[test]
    fn a_doors_own_texture_survives_the_height_pass() {
        // `emit_doors` writes the theme door texture onto both door faces'
        // `upper` before the height pass runs. Without the fill-if-empty
        // guard the pass would overwrite it with a plain wall texture, and
        // the door would stop reading as a door.
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":24, "ceiling":152, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("compiles");
        let door_tex = tables.texture("door", "tech_base").expect("door texture");
        let door_faces = out
            .data
            .sidedefs
            .iter()
            .filter(|s| s.upper == door_tex)
            .count();
        // `emit_doors` writes the door texture onto *both* sidedefs (front
        // and back) of *both* the near and far door lines — 4 in total,
        // independent of the height pass — so the theme's door texture
        // reads correctly no matter which side the player approaches from.
        // Without the fill-if-empty guard, the height pass would overwrite
        // the room/alcove-facing sidedef of each line with that neighbor's
        // own wall texture (since it is always the higher-ceiling, "visible"
        // side here), dropping this count to 2; verified directly by running
        // the pass with the guard removed.
        assert_eq!(
            door_faces, 4,
            "all four door-face sidedefs keep the theme's door texture"
        );
    }

    #[test]
    fn a_door_across_a_floor_difference_puts_the_lower_on_the_doors_own_side() {
        // I1: the `back` arm of `heights::visible_lower_side` is not merely
        // a theoretical possibility exercised by a hand-built `MapData`
        // (`heights::tests::a_higher_front_floor_and_lower_front_ceiling_textures_the_back_side`)
        // — a door portal across a floor difference produces it directly
        // through the real pipeline. `doors::emit_doors` always gives the
        // door sector `min(floors)`, and `portals::emit_segment`'s far
        // threshold always puts the far neighbor (here, room b's alcove) on
        // the front and the door sector on the back, so when the far
        // neighbor sits higher (room b's floor 24 against room a's 0), the
        // door sector's own sidedef is the visible lower side of its own
        // far threshold — reusing the same fixture as
        // `a_doors_own_texture_survives_the_height_pass` above.
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":24, "ceiling":152, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"door", "width":128, "at":[256,128],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }] }"#;
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let out = compile(&ir, &tables).expect("compiles");

        // Only the door's own two faces carry a nonzero tag. Of the two, the
        // far one is whichever has the higher-floored front sidedef: the
        // near alcove copies room a's floor (0), the far alcove copies room
        // b's (24) — `doors::emit_doors`'s own doc comment for why each
        // alcove copies the room it directly borders.
        let far_line = out
            .data
            .linedefs
            .iter()
            .filter(|l| l.tag != 0)
            .max_by_key(|l| out.data.sectors[out.data.sidedefs[l.front].sector].floor)
            .expect("the door has two tagged faces");
        let door_side = far_line.back.expect("a door's faces are two-sided");
        assert_eq!(
            out.data.sidedefs[door_side].lower, "STARTAN3",
            "the door sector's own sidedef carries the lower on its higher-floored far side"
        );
    }

    /// The regression for the shipped unfinishable map. A set-union flood
    /// passes this fixture — the pit is reachable, so the key "is obtained",
    /// so the exit "is reachable" — which is exactly why P7 searches states.
    #[test]
    fn p7_a_key_in_a_one_way_pit_is_unfinishable_and_names_the_pit() {
        let v = p7_violations(&key_pit_ir(-32));
        assert!(
            v.iter().any(|x| x.detail.contains("no feasible walk")),
            "unfinishable is the headline: {v:?}"
        );
        let stranded: Vec<_> = v
            .iter()
            .filter(|x| x.detail.contains("can no longer reach an exit"))
            .collect();
        assert!(
            stranded.iter().any(|x| x.subject.contains("pit")),
            "the stranding report names the culprit room: {v:?}"
        );
        assert!(
            stranded.iter().all(|x| x.subject.contains("pit")),
            "when nothing can finish, only key-collecting sectors are named: {v:?}"
        );
        assert!(
            stranded[0].detail.contains("blue_card/blue_skull"),
            "and says which keys are held there: {v:?}"
        );
    }

    #[test]
    fn p7_the_same_map_with_a_climbable_pit_is_clean() {
        assert!(p7_violations(&key_pit_ir(-16)).is_empty());
    }

    /// The exact boundary: -24 is one step (climbable), -25 is not.
    #[test]
    fn p7_the_step_boundary_is_exact() {
        assert!(
            p7_violations(&key_pit_ir(-24)).is_empty(),
            "-24 is one step"
        );
        let v = p7_violations(&key_pit_ir(-25));
        assert!(
            v.iter().any(|x| x.detail.contains("no feasible walk")),
            "-25 is a softlock, and unfinishable is the headline (unreachable-only output must not \
             satisfy this): {v:?}"
        );
    }

    /// The vacuous gate, pinned: no exit means P7 does not run — dozens of
    /// structural fixtures (this file's own `ir()` among them) have no exit
    /// and must stay green. Same for a map with no player start.
    #[test]
    fn p7_is_vacuous_without_an_exit_or_without_a_start() {
        assert!(
            p7_violations(&ir(0, 128, 160)).is_empty(),
            "ir() has no exits"
        );
        let no_start = key_pit_ir(-32).replace(
            r#""things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }]"#,
            r#""things":[]"#,
        );
        assert!(p7_violations(&no_start).is_empty(), "no start: vacuous");
    }

    /// A pit with nothing required in it is still a softlock — this is the
    /// fixture that separates "no softlock" from mere finishability. An
    /// implementation checking only that the exit is reachable passes it
    /// wrongly.
    #[test]
    fn p7_a_bare_pit_strands_even_though_the_map_is_finishable() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"pit", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":-32, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"soulsphere", "at":[448,128], "angle":0 }] }
          ],
          "portals":[{ "a":"hub", "b":"pit", "kind":"plain", "width":64, "at":[256,128] }],
          "exits":[{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] }] }"#;
        let v = p7_violations(json);
        assert!(
            !v.iter().any(|x| x.detail.contains("no feasible walk")),
            "the exit is in the hub — finishable: {v:?}"
        );
        assert!(
            v.iter()
                .any(|x| x.subject.contains("pit")
                    && x.detail.contains("can no longer reach an exit")),
            "but the pit strands: {v:?}"
        );
        // No key sits in the pit, so `held` contributes no ` holding ...`
        // segment — pin the exact mask-0 wording, not just a substring, so a
        // regression that reintroduces a stray space or an empty backtick
        // pair is caught.
        let stranded = v
            .iter()
            .find(|x| x.subject.contains("pit"))
            .expect("asserted to exist above");
        assert_eq!(
            stranded.detail,
            "the player can reach this sector but can no longer reach an exit from it",
            "empty-held-mask wording, exactly: {v:?}"
        );
    }

    /// The blue card behind the blue door: no state ever holds the key.
    #[test]
    fn p7_a_key_behind_its_own_door_is_unfinishable() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"vault", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"blue_card", "at":[448,128], "angle":0 }] }
          ],
          "portals":[{ "a":"hub", "b":"vault", "kind":"locked", "lock":"blue_card",
                       "width":128, "at":[256,128],
                       "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "exits":[{ "room":"vault", "trigger":"switch", "width":32, "at":[576,128] }] }"#;
        let v = p7_violations(json);
        assert!(
            v.iter().any(|x| x.detail.contains("no feasible walk")),
            "{v:?}"
        );
        assert!(
            v.iter()
                .any(|x| x.subject.contains("vault") && x.detail.contains("never be visited")),
            "the vault is unreachable too: {v:?}"
        );
    }

    /// Red behind the blue door, blue in the open: a two-key ordering chain
    /// that must pass, exercising multi-key masks end to end.
    #[test]
    fn p7_a_two_key_chain_in_order_is_clean() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 },
                        { "kind":"blue_card", "at":[64,64], "angle":0 }] },
            { "id":"mid", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"red_card", "at":[448,128], "angle":0 }] },
            { "id":"vault", "footprint":[[640,0],[640,256],[896,256],[896,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[
            { "a":"hub", "b":"mid", "kind":"locked", "lock":"blue_card",
              "width":128, "at":[256,128],
              "door_thickness":32, "alcove_near":16, "alcove_far":16 },
            { "a":"mid", "b":"vault", "kind":"locked", "lock":"red_card",
              "width":128, "at":[576,128],
              "door_thickness":32, "alcove_near":16, "alcove_far":16 }
          ],
          "exits":[{ "room":"vault", "trigger":"switch", "width":32, "at":[896,128] }] }"#;
        assert!(p7_violations(json).is_empty());
    }

    /// The engine accepts the skull for a card lock (`EV_VerticalDoor`,
    /// pinned p_doors.c:371-403), so P7 must too. P24's string-equality
    /// coherence check fires on this map (lock names `blue_card`, placed key
    /// is `blue_skull`) — that asymmetry is P24's recorded posture
    /// (authoring-intent, stricter than the engine), and this test filters
    /// to P7, which must be clean.
    #[test]
    fn p7_a_skull_key_satisfies_a_card_lock() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 },
                        { "kind":"blue_skull", "at":[64,64], "angle":0 }] },
            { "id":"vault", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"hub", "b":"vault", "kind":"locked", "lock":"blue_card",
                       "width":128, "at":[256,128],
                       "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "exits":[{ "room":"vault", "trigger":"switch", "width":32, "at":[576,128] }] }"#;
        assert!(p7_violations(json).is_empty());
    }

    /// A room no portal connects: coverage, not finishability, is what
    /// catches authored dead content.
    #[test]
    fn p7_an_isolated_room_is_flagged_by_coverage() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"island", "footprint":[[320,320],[320,576],[576,576],[576,320]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[],
          "exits":[{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] }] }"#;
        let v = p7_violations(json);
        assert_eq!(v.len(), 1, "{v:?}");
        assert!(v[0].subject.contains("island"));
        assert!(v[0].detail.contains("never be visited"));
    }

    /// A door across a >24 floor delta is one-way *through the door*: the
    /// door sector's floor is the min of its rooms, so the step out to the
    /// higher room is the full delta. The step rule must bind on door
    /// floors, not just plain portals.
    #[test]
    fn p7_a_door_across_a_tall_step_is_one_way() {
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"high", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":48, "ceiling":176, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"hub", "b":"high", "kind":"door",
                       "width":128, "at":[256,128],
                       "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "exits":[{ "room":"high", "trigger":"switch", "width":32, "at":[576,128] }] }"#;
        let v = p7_violations(json);
        assert!(
            v.iter().any(|x| x.detail.contains("no feasible walk")),
            "{v:?}"
        );
        assert!(
            v.iter()
                // The room's own entry, not the recess off it: `contains`
                // would accept either.
                .any(|x| x.subject == "room `high`" && x.detail.contains("never be visited")),
            "{v:?}"
        );
    }
}
