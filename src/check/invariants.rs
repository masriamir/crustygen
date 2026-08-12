//! Layer-4 invariants re-derived directly from the emitted `UdmfMap` and the
//! [`Scene`] built from it — never from `compile/` or `rules.rs`, which is
//! exactly the logic these checks exist to cross-examine (`check/mod.rs`'s
//! module doc explains the reuse boundary). Each pass here independently
//! re-derives its rule from the pinned engine source or the vocabulary
//! rather than calling through the compiler's own decision, so a bug shared
//! by the compiler's pre-check and its texture-filling pass cannot also
//! infect the verifier (see `KNOWN-GAPS.md`'s note on
//! `heights::visible_lower_side`/`visible_upper_side` being the single,
//! un-cross-checked place that comparison is made compile-side).

use crate::check::scene::Scene;
use crate::check::{Finding, Severity, Subject, TagEntry};
use crate::tables::Tables;
use crustywad::map::udmf::UdmfMap;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Builds a `"V-P8"` Error naming `linedef`.
fn texture_error(linedef: usize, message: String) -> Finding {
    Finding {
        check: "V-P8",
        severity: Severity::Error,
        subject: Subject::Linedef(linedef),
        message,
    }
}

/// V-P8: every boundary that needs a texture to stay opaque has one.
///
/// Re-derived from the pinned engine (`r_segs.c`, commit
/// `a77dfb96cb91780ca334d0d4cfd86957558007e0`, `R_StoreWallRange`), not
/// trusted from `heights::visible_lower_side`/`visible_upper_side`, the
/// compile-side functions this check exists to cross-examine:
///
/// - Lines 118-119 and 394 set `frontsector`/`backsector`/`sidedef` for the
///   seg being stored; `sidedef` (line 394, `curline->sidedef`) is the
///   currently-rendered side's **own** sidedef.
/// - Lines 450-451: `worldtop = frontsector->ceilingheight - viewz;
///   worldbottom = frontsector->floorheight - viewz;` — the rendered side's
///   own sector.
/// - Lines 526-527: `worldhigh = backsector->ceilingheight - viewz; worldlow
///   = backsector->floorheight - viewz;` — the neighbor's.
/// - Line 570: `if (worldhigh < worldtop)` selects `sidedef->toptexture`
///   (own sidedef) — i.e. `backsector->ceilingheight <
///   frontsector->ceilingheight`: the side whose **own** sector has the
///   **higher** ceiling draws the upper texture.
/// - Line 589: `if (worldlow > worldbottom)` selects
///   `sidedef->bottomtexture` (own sidedef) — i.e.
///   `backsector->floorheight > frontsector->floorheight`: the side whose
///   **own** sector has the **lower** floor draws the lower texture (the
///   step riser up to the neighbor's higher floor).
///
/// `R_StoreWallRange` runs once per rendered seg with `frontsector`/
/// `backsector` bound to whichever side is being drawn, so the same
/// comparison applies symmetrically to both sides of a two-sided line —
/// this confirms, rather than contradicts, the expected rule.
///
/// Rules, each linedef visited once (`fronts_this` only, so a two-sided
/// line is not double-reported): a one-sided line's front `texturemiddle`
/// must not be `"-"`; a two-sided line with differing floors needs
/// `texturebottom` on the lower-floor side; differing ceilings need
/// `texturetop` on the higher-ceiling side.
///
/// No sky exception: `r_segs.c` skips the upper texture when both sectors'
/// `ceilingpic == skyflatnum` (`worldtop = worldhigh`, same file, a few
/// lines below the `worldhigh`/`worldlow` assignment above, at line 533).
/// crustygen never emits a sky flat, so no fixture can reach that case —
/// deliberately unwritten rather than guessed at (`KNOWN-GAPS.md`, "P8 has
/// no sky exception").
pub fn check_textures(map: &UdmfMap, scene: &Scene, findings: &mut Vec<Finding>) {
    for sector in &scene.sectors {
        for b in &sector.boundary {
            if !b.fronts_this {
                continue;
            }
            let front = &map.sidedefs[b.sidedef];
            if !b.two_sided {
                if front.texturemiddle == "-" {
                    findings.push(texture_error(
                        b.linedef,
                        "one-sided line has no middle texture".to_owned(),
                    ));
                }
                continue;
            }
            let Some(neighbor_idx) = b.neighbor else {
                continue;
            };
            let neighbor = &scene.sectors[neighbor_idx];
            let Some(back_idx) = map.linedefs[b.linedef]
                .sideback
                .and_then(|s| usize::try_from(s).ok())
            else {
                continue;
            };
            let back = &map.sidedefs[back_idx];

            if sector.floor != neighbor.floor {
                let lower = if sector.floor < neighbor.floor {
                    front
                } else {
                    back
                };
                if lower.texturebottom == "-" {
                    findings.push(texture_error(
                        b.linedef,
                        format!(
                            "floors differ ({} vs {}) but the lower-floor side has no lower texture",
                            sector.floor, neighbor.floor
                        ),
                    ));
                }
            }

            if sector.ceiling != neighbor.ceiling {
                let higher = if sector.ceiling > neighbor.ceiling {
                    front
                } else {
                    back
                };
                if higher.texturetop == "-" {
                    findings.push(texture_error(
                        b.linedef,
                        format!(
                            "ceilings differ ({} vs {}) but the higher-ceiling side has no upper texture",
                            sector.ceiling, neighbor.ceiling
                        ),
                    ));
                }
            }
        }
    }
}

/// V-P9: no sidedef carries a UDMF texture-scaling extension.
///
/// Vanilla Doom's renderer has no per-sidedef texture scaling at all —
/// `scalex_*`/`scaley_*` (`ZDoom`'s UDMF extension) have no dedicated field
/// in crustywad's typed [`crustywad::map::udmf::UdmfSidedef`], which covers
/// only the five standard UDMF sidedef fields, so they can only land in
/// [`crustywad::map::udmf::UdmfSidedef::extras`] — and their presence means
/// a source-port-only effect on the vanilla-shaped engine this crate
/// targets.
///
/// Named by the linedef that references the sidedef when one does
/// (`Subject::Linedef`, cheaply derived from `map.linedefs` alone — no
/// `Scene` needed); a sidedef no linedef references (an orphan — this
/// compiler's own output can leave one by design, e.g. unused perimeter
/// sidedefs from geometry helpers) falls back to `Subject::Map`, naming the
/// sidedef's declaration index in the message either way.
pub fn check_scaling(map: &UdmfMap, findings: &mut Vec<Finding>) {
    let mut owner: HashMap<usize, usize> = HashMap::new();
    for (i, line) in map.linedefs.iter().enumerate() {
        if let Ok(front) = usize::try_from(line.sidefront) {
            owner.entry(front).or_insert(i);
        }
        if let Some(back) = line.sideback.and_then(|s| usize::try_from(s).ok()) {
            owner.entry(back).or_insert(i);
        }
    }

    for (i, sidedef) in map.sidedefs.iter().enumerate() {
        for extra in &sidedef.extras {
            if !(extra.name.starts_with("scalex") || extra.name.starts_with("scaley")) {
                continue;
            }
            let message = format!(
                "sidedef {i} sets `{}`, a texture-scaling extension vanilla Doom's renderer \
                 has no notion of",
                extra.name
            );
            let subject = owner
                .get(&i)
                .map_or(Subject::Map, |&linedef| Subject::Linedef(linedef));
            findings.push(Finding {
                check: "V-P9",
                severity: Severity::Error,
                subject,
                message,
            });
        }
    }
}

/// V-P11: no door-special boundary carries either unpegged flag on its own
/// face.
///
/// **This pins the project's authoring convention, not an engine
/// requirement** — unlike V-P8/V-P9, nothing here is a source-verified
/// playability fact, which is why its severity is [`Severity::Warning`],
/// not Error (the same downgrade `docs/design.md` §9 gives P10: "a badly
/// tiled wall is ugly, not broken" — a convention violation, not a broken
/// map). `ML_DONTPEGBOTTOM` only repositions the *lower* texture
/// (`r_segs.c`, `R_StoreWallRange`, the branch this crate already read for
/// V-P8: `if (worldlow > worldbottom) { ... if (linedef->flags &
/// ML_DONTPEGBOTTOM) { rw_bottomtexturemid = worldtop; } else
/// rw_bottomtexturemid = worldlow; }`, lines 589-601). On a typical door
/// face — floors level on both sides, only the ceiling differs while
/// closed — that branch's own guard (`worldlow > worldbottom`, i.e.
/// `backsector->floorheight > frontsector->floorheight`) never fires, so no
/// lower texture is even selected: `ML_DONTPEGBOTTOM` is inert there. The
/// door's own visible texture instead lives in the *upper* texture slot
/// (the sibling `worldhigh < worldtop` branch, lines 570-588), governed by
/// `ML_DONTPEGTOP`, a different bit. Measured against `DOOM2.WAD`: 247 of
/// 255 door-special lines carry neither flag, so this rule mostly confirms
/// an existing near-universal authoring practice.
///
/// The ruling this check enforces: neither flag belongs on a door's own
/// **faces** — only its jamb ("track") sidedefs should be lower-unpegged,
/// so the track's texture stays anchored to the floor as the door's
/// ceiling animates open. A door's track is a separate, unindexed concept a
/// [`crate::check::scene::Boundary`] cannot even name, so the IR's
/// `Portal::track_lower_unpegged` opt-out (which only ever governed the
/// track, per `KNOWN-GAPS.md`) is invisible here by construction, not by an
/// explicit skip — this check judges faces only because faces are all it
/// can see.
///
/// Each linedef is visited once (`fronts_this` only): `special`,
/// `upper_unpegged`, and `lower_unpegged` are all linedef-wide, so both
/// mirrors of a two-sided door line would otherwise report the identical
/// defect twice.
pub fn check_door_pegging(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) {
    let door = i32::from(tables.door_special());
    let locked: Vec<i32> = tables
        .locked_door_kinds()
        .into_iter()
        .map(|(_, special)| i32::from(special))
        .collect();

    for sector in &scene.sectors {
        for b in &sector.boundary {
            if !b.fronts_this || !(b.upper_unpegged || b.lower_unpegged) {
                continue;
            }
            if b.special == door || locked.contains(&b.special) {
                findings.push(Finding {
                    check: "V-P11",
                    severity: Severity::Warning,
                    subject: Subject::Linedef(b.linedef),
                    message: format!(
                        "door special {} carries an unpegged flag on its own face \
                         (dontpegtop={}, dontpegbottom={}) — only the track should be pegged",
                        b.special, b.upper_unpegged, b.lower_unpegged
                    ),
                });
            }
        }
    }
}

/// V-P13/P14: a tag is the compiler's cross-reference between an action
/// line and the sector(s) it addresses, so a mismatch on either side is
/// silently unplayable rather than a build error. Re-derives both rules
/// `docs/design.md`'s "Tags and specials" section states directly, without
/// consulting `compile/tags.rs`'s own `check_no_action_at_tag_zero` (the
/// logic this check exists to cross-examine):
///
/// - **P14**: any linedef whose `special` is nonzero ("action line") must
///   carry a nonzero `args[0]`. Tag 0 is not "no tag" to the engine — it is
///   the tag every untagged sector already has, so a stray 0 on an action
///   line matches *every* untagged sector in the map: one stray zero opens
///   every door.
/// - **P13**: an action line's tag must resolve to at least one sector — an
///   unresolvable tag is a dead action, since nothing happens when it
///   fires. Symmetrically, a sector carrying a nonzero tag that no action
///   line references is a stale tag: suspicious (dead weight, or a
///   forgotten trigger) but not itself broken, hence [`Severity::Warning`]
///   rather than Error.
///
/// This compiler tags every action line uniformly, manual doors included —
/// a manual door's tag resolves to its own back sector even though the
/// vanilla manual-door path never reads it (`KNOWN-GAPS.md`) — so that
/// convention needs no special case here: a manual door's own sector is
/// simply a sector an action line references, like any other.
///
/// Returns the tag manifest: one [`TagEntry`] per distinct nonzero tag seen
/// on either side (an action line's `args[0]` or a sector's `id`), sorted
/// ascending by tag, with `sectors`/`lines` holding the declaration indices
/// that carry/reference it.
pub fn check_tags(map: &UdmfMap, findings: &mut Vec<Finding>) -> Vec<TagEntry> {
    let mut manifest: BTreeMap<i32, TagEntry> = BTreeMap::new();
    let sector_ids: HashSet<i32> = map
        .sectors
        .iter()
        .map(|sector| sector.id)
        .filter(|&id| id != 0)
        .collect();

    for (i, sector) in map.sectors.iter().enumerate() {
        if sector.id == 0 {
            continue;
        }
        manifest
            .entry(sector.id)
            .or_insert_with(|| TagEntry {
                tag: sector.id,
                sectors: Vec::new(),
                lines: Vec::new(),
            })
            .sectors
            .push(i);
    }

    for (i, line) in map.linedefs.iter().enumerate() {
        if line.special == 0 {
            continue;
        }
        let tag = line.args[0];
        if tag == 0 {
            findings.push(Finding {
                check: "V-P14",
                severity: Severity::Error,
                subject: Subject::Linedef(i),
                message: format!(
                    "action line (special {}) carries tag 0, which every untagged sector already has",
                    line.special
                ),
            });
            continue;
        }
        if !sector_ids.contains(&tag) {
            findings.push(Finding {
                check: "V-P13",
                severity: Severity::Error,
                subject: Subject::Linedef(i),
                message: format!("action line references tag {tag}, but no sector has that id"),
            });
        }
        manifest
            .entry(tag)
            .or_insert_with(|| TagEntry {
                tag,
                sectors: Vec::new(),
                lines: Vec::new(),
            })
            .lines
            .push(i);
    }

    for entry in manifest.values() {
        if entry.lines.is_empty() {
            for &sector in &entry.sectors {
                findings.push(Finding {
                    check: "V-P13",
                    severity: Severity::Warning,
                    subject: Subject::Sector(sector),
                    message: format!(
                        "sector carries tag {} but no action line references it",
                        entry.tag
                    ),
                });
            }
        }
    }

    manifest.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check;
    use crustywad::Limits;
    use crustywad::map::udmf::parse_udmf;

    /// `check::scene`'s own `TWO_BOX` test fixture (private to that module,
    /// so re-derived here rather than imported), extended: sector 1's floor
    /// raised to 24 (`heightfloor = 24;`) so the shared line's floors
    /// differ, and sidedef 0 (the shared line's front, sector 0 — the
    /// lower-floor side) given `texturebottom = "STEP1"` so the step is
    /// clean out of the box. Shared by every test in this module.
    const TWO_BOX_STEPPED: &str = r#"namespace = "doom";
vertex { x = 0.000; y = 0.000; }
vertex { x = 64.000; y = 0.000; }
vertex { x = 128.000; y = 0.000; }
vertex { x = 128.000; y = 64.000; }
vertex { x = 64.000; y = 64.000; }
vertex { x = 0.000; y = 64.000; }
linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }
linedef { v1 = 0; v2 = 1; sidefront = 2; blocking = true; }
linedef { v1 = 4; v2 = 5; sidefront = 3; blocking = true; }
linedef { v1 = 5; v2 = 0; sidefront = 4; blocking = true; }
linedef { v1 = 1; v2 = 2; sidefront = 5; blocking = true; }
linedef { v1 = 2; v2 = 3; sidefront = 6; blocking = true; }
linedef { v1 = 3; v2 = 4; sidefront = 7; blocking = true; }
sidedef { sector = 0; texturemiddle = "-"; texturebottom = "STEP1"; }
sidedef { sector = 1; texturemiddle = "-"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 0; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sidedef { sector = 1; texturemiddle = "STARTAN2"; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }
sector { texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightfloor = 24; heightceiling = 128; lightlevel = 160; }
thing { x = 32.000; y = 32.000; type = 1; skill1 = true; skill2 = true; skill3 = true; skill4 = true; skill5 = true; single = true; dm = true; coop = true; }
"#;

    /// Runs the full `check::run` orchestrator over `text` and returns its
    /// findings.
    fn findings_of(text: &str) -> Vec<Finding> {
        let map = parse_udmf(text, Limits::default()).expect("fixture parses");
        let tables = Tables::load().expect("tables");
        check::run(&map, "fixture", &tables, None).findings
    }

    #[test]
    fn a_floor_step_missing_its_lower_texture_is_a_p8_error() {
        let stepped = TWO_BOX_STEPPED.replace("texturebottom = \"STEP1\"; ", "");
        let findings = findings_of(&stepped);
        assert!(
            findings.iter().any(|f| f.check == "V-P8"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Linedef(0))),
            "expected a V-P8 error on linedef 0: {findings:?}"
        );
    }

    #[test]
    fn a_one_sided_line_missing_its_middle_is_a_p8_error() {
        let blanked = TWO_BOX_STEPPED.replacen(
            "sidedef { sector = 0; texturemiddle = \"STARTAN2\"; }",
            "sidedef { sector = 0; texturemiddle = \"-\"; }",
            1,
        );
        let findings = findings_of(&blanked);
        assert!(
            findings.iter().any(|f| f.check == "V-P8"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Linedef(1))),
            "expected a V-P8 error on linedef 1: {findings:?}"
        );
    }

    #[test]
    fn a_scale_factor_on_any_sidedef_is_a_p9_error() {
        let scaled = TWO_BOX_STEPPED.replace(
            "sidedef { sector = 0; texturemiddle = \"STARTAN2\"; }",
            "sidedef { sector = 0; texturemiddle = \"STARTAN2\"; scalex_mid = 2.0; }",
        );
        let findings = findings_of(&scaled);
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-P9" && f.severity == Severity::Error),
            "expected a V-P9 error: {findings:?}"
        );
    }

    #[test]
    fn a_clean_stepped_fixture_raises_no_texture_findings() {
        let findings = findings_of(TWO_BOX_STEPPED);
        assert!(
            findings
                .iter()
                .all(|f| f.check != "V-P8" && f.check != "V-P9"),
            "clean fixture: {findings:?}"
        );
    }

    /// [`TWO_BOX_STEPPED`] with the shared line turned into a manual door
    /// (`special = 1`, `arg0 = 5`) and sector 1 tagged to match (`id = 5`).
    /// `extra_flags` is spliced verbatim into the linedef block (e.g.
    /// `" dontpegbottom = true;"`), empty for the clean case.
    fn door_fixture(extra_flags: &str) -> String {
        TWO_BOX_STEPPED
            .replace(
                "linedef { v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; }",
                &format!(
                    "linedef {{ v1 = 1; v2 = 4; sidefront = 0; sideback = 1; twosided = true; \
                     special = 1; arg0 = 5;{extra_flags} }}"
                ),
            )
            .replace(
                "sector { texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 24; heightceiling = 128; lightlevel = 160; }",
                "sector { texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 24; heightceiling = 128; lightlevel = 160; id = 5; }",
            )
    }

    #[test]
    fn a_door_face_with_dontpegbottom_is_a_p11_warning() {
        let findings = findings_of(&door_fixture(" dontpegbottom = true;"));
        assert!(
            findings.iter().any(|f| f.check == "V-P11"
                && f.severity == Severity::Warning
                && matches!(f.subject, Subject::Linedef(0))),
            "expected a V-P11 warning on linedef 0: {findings:?}"
        );
    }

    #[test]
    fn a_door_face_with_dontpegtop_is_a_p11_warning() {
        let findings = findings_of(&door_fixture(" dontpegtop = true;"));
        assert!(
            findings.iter().any(|f| f.check == "V-P11"
                && f.severity == Severity::Warning
                && matches!(f.subject, Subject::Linedef(0))),
            "expected a V-P11 warning on linedef 0: {findings:?}"
        );
    }

    #[test]
    fn a_door_face_with_neither_unpegged_flag_raises_no_p11_finding() {
        let findings = findings_of(&door_fixture(""));
        assert!(
            findings.iter().all(|f| f.check != "V-P11"),
            "clean fixture: {findings:?}"
        );
    }

    /// Runs [`check_tags`] alone (not the full orchestrator) over `text` and
    /// returns its manifest and findings.
    fn tags_of(text: &str) -> (Vec<TagEntry>, Vec<Finding>) {
        let map = parse_udmf(text, Limits::default()).expect("fixture parses");
        let mut findings = Vec::new();
        let manifest = check_tags(&map, &mut findings);
        (manifest, findings)
    }

    #[test]
    fn a_manual_doors_tag_resolves_to_its_own_sector_in_the_manifest() {
        let (manifest, findings) = tags_of(&door_fixture(""));
        assert_eq!(
            manifest,
            vec![TagEntry {
                tag: 5,
                sectors: vec![1],
                lines: vec![0],
            }]
        );
        assert!(
            findings
                .iter()
                .all(|f| f.check != "V-P13" && f.check != "V-P14"),
            "clean tag fixture: {findings:?}"
        );
    }

    #[test]
    fn an_action_line_tagged_zero_is_a_p14_error() {
        let untagged = door_fixture("").replace("arg0 = 5;", "arg0 = 0;");
        let (_, findings) = tags_of(&untagged);
        assert!(
            findings.iter().any(|f| f.check == "V-P14"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Linedef(0))),
            "expected a V-P14 error on linedef 0: {findings:?}"
        );
    }

    #[test]
    fn an_action_tag_matching_no_sector_is_a_p13_error() {
        let orphaned = door_fixture("").replace("arg0 = 5;", "arg0 = 9;");
        let (_, findings) = tags_of(&orphaned);
        assert!(
            findings.iter().any(|f| f.check == "V-P13"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Linedef(0))),
            "expected a V-P13 error on linedef 0: {findings:?}"
        );
    }

    #[test]
    fn a_sector_tag_with_no_referencing_action_line_is_a_p13_warning() {
        // TWO_BOX_STEPPED has no action lines at all; tag sector 1 anyway.
        let stale = TWO_BOX_STEPPED.replace(
            "sector { texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 24; heightceiling = 128; lightlevel = 160; }",
            "sector { texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; heightfloor = 24; heightceiling = 128; lightlevel = 160; id = 7; }",
        );
        let (_, findings) = tags_of(&stale);
        assert!(
            findings.iter().any(|f| f.check == "V-P13"
                && f.severity == Severity::Warning
                && matches!(f.subject, Subject::Sector(1))),
            "expected a V-P13 warning on sector 1: {findings:?}"
        );
    }
}
