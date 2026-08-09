//! Playability invariants checked against compiled output.
//!
//! `check_all` implements a deliberately partial rule catalog: **P1** (step
//! height between connected rooms), **P3** (passage width), **P4** (door
//! opening clearance), **P8** (no missing textures), **P9** (no texture
//! scaling), **P19** (light bounds), and **P24** (key and lock coherence).
//!
//! **P7** (no softlock) and **P20** (pickup accessibility) are deliberately
//! *not* implemented here — both need a key-aware reachability flood over the
//! room graph, which is a later-stage concern that belongs with the verifier,
//! not this stage-one structural pass. Do not read the presence of this
//! module as covering them.

use crate::compile::Compiled;
use crate::ir::{Ir, PortalKind};
use crate::tables::Tables;

/// One failed playability check.
#[derive(Debug, Clone)]
pub struct RuleViolation {
    /// The rule identifier, e.g. `"P1"`.
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
    check_step_height(ir, tables, &mut v);
    check_passage_width(ir, tables, &mut v);
    check_light_bounds(ir, tables, &mut v);
    check_no_scaling(out, &mut v);
    check_missing_textures(out, &mut v);
    check_key_lock_coherence(ir, &mut v);
    v
}

/// P1 and P4: connected rooms must not differ by more than one step, and a
/// door's opening must clear the player.
///
/// The two checks share this function because both walk the same portal
/// loop and both need the same pair of rooms already resolved; splitting
/// them would just duplicate that lookup.
fn check_step_height(ir: &Ir, tables: &Tables, v: &mut Vec<RuleViolation>) {
    let limit = tables.step_height();
    let player = tables.player();
    for p in &ir.portals {
        let (Some(a), Some(b)) = (ir.room(&p.a), ir.room(&p.b)) else {
            continue;
        };
        let delta = (a.floor - b.floor).abs();
        if delta > limit {
            v.push(RuleViolation {
                rule: "P1",
                subject: format!("{} <-> {}", p.a, p.b),
                detail: format!("floor delta {delta} exceeds max step height {limit}"),
            });
        }
        if matches!(p.kind, PortalKind::Door | PortalKind::Locked) {
            // A door's open ceiling stops short of the lowest neighboring
            // ceiling by the engine's clearance allowance (P_DoorDoor:
            // `topheight = P_FindLowestCeilingSurrounding(sec) - 4`), so the
            // usable opening is smaller than the nominal room height. The
            // clearance is measured from the *higher* of the two floors
            // (`max`, not the door sector's own carved floor, which is the
            // lower of the two): a player standing on the higher-floor side
            // has less headroom to the door's open ceiling than one on the
            // lower-floor side, so the higher floor is the binding
            // constraint on whether the door actually clears the player.
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
        if front_sector.floor != back_sector.floor
            && (front.lower.is_empty() || back.lower.is_empty())
        {
            v.push(RuleViolation {
                rule: "P8",
                subject: format!("linedef {i}"),
                detail: format!(
                    "floors differ ({} vs {}) but a lower texture is missing",
                    front_sector.floor, back_sector.floor
                ),
            });
        }
        if front_sector.ceiling != back_sector.ceiling
            && (front.upper.is_empty() || back.upper.is_empty())
        {
            v.push(RuleViolation {
                rule: "P8",
                subject: format!("linedef {i}"),
                detail: format!(
                    "ceilings differ ({} vs {}) but an upper texture is missing",
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
    /// ceilings so the P4 boundary can be pinned without perturbing P1
    /// (both floors are 0) or P3 (width is well above the player's
    /// diameter).
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
    fn p1_a_step_at_the_limit_passes_and_one_unit_over_fails() {
        let tables = Tables::load().expect("tables");
        let limit = tables.step_height();
        assert!(!violations(&ir(limit, 128, 160)).contains(&"P1".to_owned()));
        assert!(violations(&ir(limit + 1, 128, 160)).contains(&"P1".to_owned()));
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

    #[test]
    fn p8_a_floor_change_without_a_lower_texture_fails() {
        // Ceilings equal (128 == 128); only the floor differs, isolating the
        // lower-texture branch from the upper-texture branch. The two-sided
        // line needs a lower texture, but the compiler leaves portal
        // sidedefs bare by default (see `compile::portals::emit_opening`).
        assert!(violations(&portal_ir(16, 128, 128)).contains(&"P8".to_owned()));
    }

    #[test]
    fn p8_a_ceiling_change_without_an_upper_texture_fails() {
        // Floors equal (0 == 0); only the ceiling differs, isolating the
        // upper-texture branch from the lower-texture branch.
        assert!(violations(&portal_ir(0, 160, 128)).contains(&"P8".to_owned()));
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
}
