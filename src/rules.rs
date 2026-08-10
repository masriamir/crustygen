//! Playability invariants checked against compiled output.
//!
//! `check_all` implements a deliberately partial rule catalog: **P3** (passage
//! width), **P4** (door opening clearance), **P8** (no missing textures),
//! **P9** (no texture scaling), **P19** (light bounds), and **P24** (key and
//! lock coherence).
//!
//! **P1** (step height between connected rooms) has been **retired**: it
//! capped the floor delta between connected rooms in either direction, but
//! `P_TryMove` caps only the climb and leaves falling unrestricted, and a
//! corpus sweep found the majority of vanilla Doom's height-changing
//! boundaries — 56.92% of them — exceeding it. See
//! [`CompileError::PortalNoHeadroom`](crate::compile::CompileError::PortalNoHeadroom)
//! for what replaced it.
//!
//! **P7** (no softlock) and **P20** (pickup accessibility) are deliberately
//! *not* implemented here — both need a key-aware reachability flood over the
//! room graph, which is a later-stage concern that belongs with the verifier,
//! not this stage-one structural pass. Do not read the presence of this
//! module as covering them.

use crate::compile::Compiled;
use crate::compile::heights::{visible_lower_side, visible_upper_side};
use crate::ir::{Ir, PortalKind};
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
    v
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
    use crate::rules::check_all;
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
}
