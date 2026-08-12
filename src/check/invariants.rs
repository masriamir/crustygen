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

/// The five thing kinds the engine reads as a player spawn point
/// (`playerstarts[MAXPLAYERS]`/`deathmatchstarts` in the pinned Doom
/// source): the four coop starts and the single deathmatch-start kind.
/// Shared by [`check_thing_headroom`] (a start's required height is the
/// player's, not a species') and [`check_starts`] (which starts to check
/// clearance and telefrag distance for).
const START_KINDS: [&str; 5] = [
    "player1_start",
    "player2_start",
    "player3_start",
    "player4_start",
    "deathmatch_start",
];

/// The required headroom for a named thing, or `None` if the vocabulary
/// pins no height requirement for it.
///
/// Tries, in order: a monster species' own height, a blocking/hanging
/// prop's own height, and — only for the five `START_KINDS` — the
/// player's height (a start is not itself a species or a prop, but the
/// player who spawns there needs to fit). Everything else (pickups, keys,
/// ammo, decorative non-blocking props) returns `None`: nothing occupies
/// or must pass through the space above a pickup, so no height is worth
/// pinning for it.
fn required_height(tables: &Tables, name: &str) -> Option<i32> {
    if let Some(dims) = tables.species(name) {
        return Some(dims.height);
    }
    if let Some(dims) = tables.prop(name) {
        return Some(dims.height);
    }
    if START_KINDS.contains(&name) {
        return Some(tables.player().height);
    }
    None
}

/// V-P2: a thing's sector has enough headroom for it to stand (or, for a
/// start, spawn) there.
///
/// For each thing with a resolved name and a resolved sector, the required
/// height is `required_height`: a monster species' height, else a
/// blocking/hanging prop's height, else the player's height for the five
/// start kinds, else no requirement at all (see `required_height`'s doc
/// comment for why pickups and keys are skipped). `ceiling - floor` less
/// than that requirement is an Error naming the thing.
///
/// **Deliberately no door-sector exemption.** A door sector's `TEXTMAP`
/// heights are its *closed* state (`docs/design.md` §7.1: "ceiling snapped
/// to its floor") — a thing placed inside one has zero static headroom by
/// construction, which reads as tempting to wave off as "it's a door, it
/// opens." It is not waved off here: a thing genuinely standing in a closed
/// door's sector is unplayable exactly as reported, whether by an authoring
/// mistake or a compiler bug placing something there it should not have.
/// This check has no notion of "door sector" at all — it treats every
/// sector identically — which is what makes that guarantee possible.
///
/// A thing whose `type_id` names nothing in the vocabulary (`name` is
/// `None`) gets a `Warning` (`"unknown thing type {type_id}"`) here, once,
/// rather than in every check that would otherwise skip it silently —
/// [`check_starts`] and [`check_prop_embedding`] both filter on `name`
/// being recognized, so an unnamed thing is invisible to them, and this is
/// the one place that fact gets surfaced.
pub fn check_thing_headroom(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) {
    for (i, thing) in scene.things.iter().enumerate() {
        let Some(name) = thing.name.as_deref() else {
            findings.push(Finding {
                check: "V-P2",
                severity: Severity::Warning,
                subject: Subject::Thing(i),
                message: format!("unknown thing type {}", thing.type_id),
            });
            continue;
        };
        let Some(required) = required_height(tables, name) else {
            continue;
        };
        let Some(sector_idx) = thing.sector else {
            continue;
        };
        let sector = &scene.sectors[sector_idx];
        let headroom = sector.ceiling - sector.floor;
        if headroom < required {
            findings.push(Finding {
                check: "V-P2",
                severity: Severity::Error,
                subject: Subject::Thing(i),
                message: format!(
                    "{name} needs {required} units of headroom but its sector (floor {}, \
                     ceiling {}) has only {headroom}",
                    sector.floor, sector.ceiling
                ),
            });
        }
    }
}

/// V-P19: every sector's light level lies within the engine's valid range.
///
/// Unconditional — every sector is checked, spec or no spec (the spec's own
/// narrower `min`/`max` bound, if one exists, is a conformance-report
/// concern, not this structural one).
pub fn check_light_bounds(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) {
    let range = tables.light_range();
    for (i, sector) in scene.sectors.iter().enumerate() {
        if !range.contains(&sector.light) {
            findings.push(Finding {
                check: "V-P19",
                severity: Severity::Error,
                subject: Subject::Sector(i),
                message: format!(
                    "light level {} is outside the valid range {}..={}",
                    sector.light,
                    range.start(),
                    range.end()
                ),
            });
        }
    }
}

/// The distance from a point to a line segment, in continuous world
/// coordinates.
///
/// [`crate::geom::dist_to_segment`] exists already but takes
/// [`crate::geom::Pt`], grid-integer coordinates the compiler's own
/// footprints are built from. A [`crate::check::scene::Boundary`]'s
/// endpoints are `f64` — copied verbatim from `UdmfVertex.x`/`.y`, which
/// UDMF itself types as floating point — and a thing's `(x, y)` is `f64`
/// too, so reusing the integer twin would mean rounding both back to `Pt`
/// first and losing precision this check has no reason to discard. Same
/// projection-and-clamp algorithm as the integer version, just without the
/// `Pt` roundtrip.
fn dist_to_segment_f64(px: f64, py: f64, ax: f64, ay: f64, bx: f64, by: f64) -> f64 {
    let (dx, dy) = (bx - ax, by - ay);
    let len2 = dx.mul_add(dx, dy * dy);
    if len2 == 0.0 {
        return (px - ax).hypot(py - ay);
    }
    let t = (((px - ax) * dx + (py - ay) * dy) / len2).clamp(0.0, 1.0);
    (px - dx.mul_add(t, ax)).hypot(py - dy.mul_add(t, ay))
}

/// V-P25: every player start has full radius clearance from its sector's
/// walls, and no two starts are close enough to telefrag each other.
/// (Headroom is covered by [`check_thing_headroom`], since a start is a
/// thing like any other there.)
///
/// For each of the five `START_KINDS` with a resolved sector: the
/// distance from `(x, y)` to every **non-passable** boundary segment of
/// that sector (an open doorway cannot crush the player against it, so only
/// [`crate::check::scene::Boundary::passable`]`() == false` segments count)
/// must be at least [`Tables::player`]'s radius, else an Error naming the
/// start. A start with no resolved sector (already a `"V-S"` Error from
/// [`Scene::build`]) is skipped here rather than double-reported.
///
/// Separately, and regardless of sector resolution: every pair of starts —
/// across all five kinds, not just within one, since a coop start and a
/// deathmatch start spawning on top of each other still telefrags whichever
/// mode is in play — closer than twice the player's radius is an Error.
/// Each pair is reported once, naming the later-declared thing of the pair.
pub fn check_starts(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) {
    let radius = f64::from(tables.player().radius);

    let starts: Vec<usize> = scene
        .things
        .iter()
        .enumerate()
        .filter(|(_, thing)| {
            thing
                .name
                .as_deref()
                .is_some_and(|name| START_KINDS.contains(&name))
        })
        .map(|(i, _)| i)
        .collect();

    for &i in &starts {
        let thing = &scene.things[i];
        let Some(sector_idx) = thing.sector else {
            continue;
        };
        let sector = &scene.sectors[sector_idx];
        let clearance = sector
            .boundary
            .iter()
            .filter(|b| !b.passable())
            .map(|b| dist_to_segment_f64(thing.x, thing.y, b.a.0, b.a.1, b.b.0, b.b.1))
            .fold(f64::INFINITY, f64::min);
        if clearance < radius {
            findings.push(Finding {
                check: "V-P25",
                severity: Severity::Error,
                subject: Subject::Thing(i),
                message: format!(
                    "start is {clearance:.3} units from the nearest wall, less than the \
                     player radius {radius}"
                ),
            });
        }
    }

    for (pos, &i) in starts.iter().enumerate() {
        for &j in &starts[pos + 1..] {
            let (a, b) = (&scene.things[i], &scene.things[j]);
            let dist = (a.x - b.x).hypot(a.y - b.y);
            if dist < 2.0 * radius {
                findings.push(Finding {
                    check: "V-P25",
                    severity: Severity::Error,
                    subject: Subject::Thing(j),
                    message: format!(
                        "start is {dist:.3} units from start {i}, within the telefrag \
                         distance {}",
                        2.0 * radius
                    ),
                });
            }
        }
    }
}

/// The powerup names `sustain.powerups[].name` can carry (`docs/design.md`
/// §5), matching `engine.toml`/`vocabulary.toml`'s doomednum entries
/// exactly. A vocabulary convention mirroring the map-spec frontmatter's
/// `PowerupSpec` name domain, not an engine fact — nothing in the pinned
/// Doom source groups these eight under one heading, so this list cannot be
/// cited to a `source` field the way a `[props.*]` or `[species.*]` entry
/// is.
const POWERUPS: [&str; 8] = [
    "berserk",
    "soulsphere",
    "megasphere",
    "invulnerability",
    "invisibility",
    "radsuit",
    "light_amp",
    "computer_map",
];

/// Whether a named thing is a collectible: something the static half of
/// V-P20 cares about not seeing embedded in a blocking prop. A weapon,
/// ammo pickup, health/armor pickup, `backpack`, a key ([`Tables::locked_door_kinds`]
/// name), or one of the eight [`POWERUPS`].
fn is_collectible(tables: &Tables, name: &str) -> bool {
    tables.pickup(name).is_some()
        || tables.ammo_pickup(name).is_some()
        || tables.weapon_damage(name).is_some()
        || name == "backpack"
        || tables
            .locked_door_kinds()
            .iter()
            .any(|(key, _)| key == name)
        || POWERUPS.contains(&name)
}

/// V-P20 (static half): no collectible sits inside a blocking prop's
/// radius.
///
/// The full P20 also requires reachability (the P7 flood already proves
/// that, per `KNOWN-GAPS.md`'s subsumption note) and radius clearance from
/// walls (a thing's own placement already gets that from the compiler, and
/// this check does not re-derive it) — this is only the piece neither of
/// those covers: a pickup a prop physically obstructs even though the room
/// itself is reachable and the pickup is not touching a wall.
///
/// For each thing whose name `is_collectible` returns true for, compares its distance to
/// every *other* thing whose name resolves to a [`Tables::prop`] with
/// `blocks == true`; a distance less than that prop's radius is an Error
/// naming the collectible (not the prop — the prop is the obstruction,
/// but the pickup is the thing that cannot be reached).
pub fn check_prop_embedding(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) {
    for (i, thing) in scene.things.iter().enumerate() {
        let Some(name) = thing.name.as_deref() else {
            continue;
        };
        if !is_collectible(tables, name) {
            continue;
        }
        for (j, other) in scene.things.iter().enumerate() {
            if i == j {
                continue;
            }
            let Some(other_name) = other.name.as_deref() else {
                continue;
            };
            let Some(prop) = tables.prop(other_name) else {
                continue;
            };
            if !prop.blocks {
                continue;
            }
            let dist = (thing.x - other.x).hypot(thing.y - other.y);
            if dist < f64::from(prop.radius) {
                findings.push(Finding {
                    check: "V-P20",
                    severity: Severity::Error,
                    subject: Subject::Thing(i),
                    message: format!(
                        "{name} is {dist:.3} units from blocking prop {other_name} (thing \
                         {j}), inside its radius {}",
                        prop.radius
                    ),
                });
            }
        }
    }
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

    // --- Task 7: V-P2 thing headroom, V-P19 light bounds, V-P25 start
    // clearance/telefrag, and the V-P20 static prop-embedding check. ---

    /// A single 128x128 closed sector, one-sided walls on every side,
    /// floor 0, and a configurable ceiling height and light level.
    /// `things` is spliced in verbatim (zero or more `thing { ... }`
    /// blocks), so callers build their own fixtures without repeating this
    /// boilerplate room shell.
    fn room(heightceiling: i32, lightlevel: i32, things: &str) -> String {
        format!(
            r#"namespace = "doom";
vertex {{ x = 0.000; y = 0.000; }}
vertex {{ x = 128.000; y = 0.000; }}
vertex {{ x = 128.000; y = 128.000; }}
vertex {{ x = 0.000; y = 128.000; }}
linedef {{ v1 = 0; v2 = 1; sidefront = 0; blocking = true; }}
linedef {{ v1 = 1; v2 = 2; sidefront = 1; blocking = true; }}
linedef {{ v1 = 2; v2 = 3; sidefront = 2; blocking = true; }}
linedef {{ v1 = 3; v2 = 0; sidefront = 3; blocking = true; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = {heightceiling}; lightlevel = {lightlevel}; }}
{things}"#
        )
    }

    /// One `thing` block at `(x, y)`, `type_id`, flagged `single` only —
    /// matching the minimal shape `check::scene`'s own `l_shape` fixture
    /// uses.
    fn thing_at(x: f64, y: f64, type_id: u16) -> String {
        format!("thing {{ x = {x:.3}; y = {y:.3}; type = {type_id}; single = true; }}\n")
    }

    #[test]
    fn an_imp_exactly_at_its_species_height_passes_headroom() {
        let t = Tables::load().expect("tables");
        let imp_height = t.species("imp").expect("imp species").height;
        let text = room(imp_height, 160, &thing_at(64.0, 64.0, 3001));
        let findings = findings_of(&text);
        assert!(
            findings.iter().all(|f| f.check != "V-P2"),
            "ceiling exactly at species height: no headroom finding: {findings:?}"
        );
    }

    #[test]
    fn an_imp_one_unit_below_its_species_height_fails_headroom() {
        let t = Tables::load().expect("tables");
        let imp_height = t.species("imp").expect("imp species").height;
        let text = room(imp_height - 1, 160, &thing_at(64.0, 64.0, 3001));
        let findings = findings_of(&text);
        assert!(
            findings.iter().any(|f| f.check == "V-P2"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(0))),
            "expected a V-P2 error on thing 0: {findings:?}"
        );
    }

    #[test]
    fn an_unrecognized_thing_type_is_a_p2_warning_and_nothing_else_flags_it() {
        let t = Tables::load().expect("tables");
        let text = room(128, 160, &thing_at(64.0, 64.0, 31337));
        let findings = findings_of(&text);
        let warnings: Vec<_> = findings
            .iter()
            .filter(|f| f.check == "V-P2" && matches!(f.subject, Subject::Thing(0)))
            .collect();
        assert_eq!(
            warnings.len(),
            1,
            "exactly one V-P2 finding for the unknown thing: {findings:?}"
        );
        assert_eq!(warnings[0].severity, Severity::Warning);
        assert_eq!(warnings[0].message, "unknown thing type 31337");
        assert!(
            t.thing_kinds().all(|(_, id)| id != 31337),
            "31337 really is unrecognized"
        );
    }

    #[test]
    fn light_level_at_the_max_bound_passes() {
        let t = Tables::load().expect("tables");
        let max = *t.light_range().end();
        let text = room(128, max, "");
        let findings = findings_of(&text);
        assert!(
            findings.iter().all(|f| f.check != "V-P19"),
            "light level at the max bound: no finding: {findings:?}"
        );
    }

    #[test]
    fn light_level_one_above_the_max_bound_fails() {
        let t = Tables::load().expect("tables");
        let max = *t.light_range().end();
        let text = room(128, max + 1, "");
        let findings = findings_of(&text);
        assert!(
            findings.iter().any(|f| f.check == "V-P19"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Sector(0))),
            "expected a V-P19 error on sector 0: {findings:?}"
        );
    }

    #[test]
    fn a_start_exactly_at_radius_clearance_from_a_wall_passes() {
        let t = Tables::load().expect("tables");
        let radius = f64::from(t.player().radius);
        let text = room(128, 160, &thing_at(radius, 64.0, 1));
        let findings = findings_of(&text);
        assert!(
            findings.iter().all(|f| f.check != "V-P25"),
            "start exactly at radius clearance from the west wall: no finding: {findings:?}"
        );
    }

    #[test]
    fn a_start_one_unit_closer_than_radius_to_a_wall_fails() {
        let t = Tables::load().expect("tables");
        let radius = f64::from(t.player().radius);
        let text = room(128, 160, &thing_at(radius - 1.0, 64.0, 1));
        let findings = findings_of(&text);
        assert!(
            findings.iter().any(|f| f.check == "V-P25"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(0))),
            "expected a V-P25 error on thing 0: {findings:?}"
        );
    }

    #[test]
    fn two_coincident_starts_telefrag_each_other() {
        let mut things = thing_at(64.0, 64.0, 1);
        things.push_str(&thing_at(64.0, 64.0, 2));
        let text = room(128, 160, &things);
        let findings = findings_of(&text);
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-P25" && f.severity == Severity::Error),
            "expected a V-P25 error for the coincident starts: {findings:?}"
        );
    }

    #[test]
    fn two_starts_exactly_two_radii_apart_pass() {
        let t = Tables::load().expect("tables");
        let radius = f64::from(t.player().radius);
        let mut things = thing_at(64.0 - radius, 64.0, 1);
        things.push_str(&thing_at(64.0 + radius, 64.0, 2));
        let text = room(128, 160, &things);
        let findings = findings_of(&text);
        assert!(
            findings.iter().all(|f| f.check != "V-P25"),
            "two starts exactly two radii apart: no telefrag finding: {findings:?}"
        );
    }

    #[test]
    fn a_stimpack_exactly_at_a_barrels_radius_passes_embedding() {
        let t = Tables::load().expect("tables");
        let barrel_radius = f64::from(t.prop("barrel").expect("barrel prop").radius);
        let mut things = thing_at(64.0, 64.0, 2035); // barrel
        things.push_str(&thing_at(64.0 + barrel_radius, 64.0, 2011)); // stimpack
        let text = room(128, 160, &things);
        let findings = findings_of(&text);
        assert!(
            findings.iter().all(|f| f.check != "V-P20"),
            "stimpack exactly at the barrel's radius: no finding: {findings:?}"
        );
    }

    #[test]
    fn a_stimpack_inside_a_barrels_radius_fails_embedding() {
        let t = Tables::load().expect("tables");
        let barrel_radius = f64::from(t.prop("barrel").expect("barrel prop").radius);
        let mut things = thing_at(64.0, 64.0, 2035); // barrel
        things.push_str(&thing_at(64.0 + barrel_radius - 2.0, 64.0, 2011)); // stimpack
        let text = room(128, 160, &things);
        let findings = findings_of(&text);
        assert!(
            findings.iter().any(|f| f.check == "V-P20"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(1))),
            "expected a V-P20 error on thing 1 (the stimpack): {findings:?}"
        );
    }
}
