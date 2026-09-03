//! Playability invariants checked against compiled output.
//!
//! `check_all` implements a deliberately partial rule catalog: **P3** (passage
//! width), **P4** (door opening clearance), **P5** (lift travel and return),
//! **P7** (no softlock), **P8** (no missing textures), **P9** (no texture
//! scaling), **P15** (teleport pairing), **P19** (light bounds), **P24** (key
//! and lock coherence), **P26** (teleport-only exit rooms), **P27** (no
//! sealed monster rooms), **P28** (a floor action's destination), **P29** (a
//! floor action opens what it is meant to) and **P30** (no floor action
//! chained to another moving sector). **P7** floods `(sector, keys-held)`
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

use std::collections::BTreeSet;

use crate::compile::floors::FloorShape;
use crate::compile::heights::{visible_lower_side, visible_upper_side};
use crate::compile::{Compiled, MapData};
use crate::ir::{ExitTrigger, Ir, PortalKind};
use crate::reach;
use crate::tables::{FloorFamily, Tables};

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
    check_teleport_pairing(tables, out, &mut v);
    check_teleport_exit_rooms(ir, out, &mut v);
    check_sealed_monster_rooms(ir, tables, out, &mut v);
    check_lift_return(tables, out, &mut v);
    check_floor_destinations(out, &mut v);
    check_floor_openings(tables, out, &mut v);
    check_floor_chains(tables, out, &mut v);
    check_reachability(ir, tables, out, &mut v);
    v
}

/// P7: no softlock — the map is finishable, nowhere the player can get to
/// is a dead end they cannot finish from, and every sector is visitable.
///
/// Delegates to [`crate::reach`]: a breadth-first search over
/// `(sector, keys held, floor actions fired)` states built from the
/// *emitted* geometry. Vacuously satisfied when the map has no player 1
/// start or no exit — see [`crate::reach::graph_from_compiled`].
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

    // Which floor actions had NOT fired in the state being reported. A room
    // the player is stranded in is often one whose way out is a wall still
    // standing or a pit still sunk, so naming the actions at rest says which
    // switch or walkover the author has to make reachable first.
    // `action_names` and `Compiled::floors` are one list indexed two ways —
    // `graph_from_compiled` builds each name from the entry at that same
    // position — so zipping them pairs a name with its own action.
    let pending = |mask: reach::KeyMask| -> String {
        let parts: Vec<String> = built
            .action_names
            .iter()
            .zip(&out.floors)
            .enumerate()
            .filter(|&(i, _)| {
                let bit = reach::ACTION_BIT_BASE + u32::try_from(i).expect("at most 8 actions");
                mask & (1 << bit) == 0
            })
            .map(|(_, (name, action))| {
                let verb = match action.family {
                    FloorFamily::LowerToLowest => "lowered",
                    FloorFamily::RaiseToNearest => "raised",
                };
                format!("{name} not {verb}")
            })
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("; {}", parts.join(", "))
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
                "the player can reach this sector{} but can no longer reach an exit from it{}",
                held(mask),
                pending(mask)
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
///
/// "Placed" means placed *anywhere a player can pick it up*, which is every
/// authored thing, not only a room's own: island cargo — a pedestal's
/// [`crate::ir::Pedestal::things`] and a reveal's
/// [`crate::ir::Reveal::things`] — is picked up by standing on the island,
/// and the layer-4 verifier's own V-P24
/// (`check::flood::check_key_lock_coherence`) already reads it that way,
/// scanning every emitted thing that resolves to a sector. Scanning only
/// `Room::things` here made the two layers disagree in the refusing
/// direction: a map whose only red card sat on a pedestal was rejected as
/// "locked by `red_card`, which is never placed" while the verifier passed
/// the very same emitted geometry.
fn check_key_lock_coherence(ir: &Ir, v: &mut Vec<RuleViolation>) {
    let placed: Vec<&str> = ir
        .rooms
        .iter()
        .flat_map(|r| r.things.iter())
        .chain(ir.pedestals.iter().flat_map(|p| p.things.iter()))
        .chain(ir.reveals.iter().flat_map(|r| r.things.iter()))
        .map(|t| t.kind.as_str())
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

/// P15: every teleport line's tag resolves to exactly one emitted sector,
/// and exactly one marker stands in it. Headroom and clearance for the
/// arriving thing are enforced at placement (`CompileError::TeleportMarker*`),
/// so a compiled map can only fail here if a later pass disturbed the pairing.
fn check_teleport_pairing(tables: &Tables, out: &Compiled, v: &mut Vec<RuleViolation>) {
    let specials = tables.teleport_specials();
    for (i, line) in out.data.linedefs.iter().enumerate() {
        if !specials.contains(&line.special) {
            continue;
        }
        let sectors = out
            .data
            .sectors
            .iter()
            .filter(|s| s.tag == line.tag && line.tag != 0)
            .count();
        if sectors != 1 {
            v.push(RuleViolation {
                rule: "P15",
                subject: format!("linedef {i}"),
                detail: format!("tag {} resolves to {sectors} sectors, not one", line.tag),
            });
            continue;
        }
        let sector = out
            .data
            .sectors
            .iter()
            .position(|s| s.tag == line.tag)
            .expect("counted above");
        let markers = out.markers.iter().filter(|m| m.sector == sector).count();
        if markers != 1 {
            v.push(RuleViolation {
                rule: "P15",
                subject: format!("linedef {i}"),
                detail: format!("destination sector {sector} holds {markers} markers, not one"),
            });
        }
    }
}

/// Whether some teleport marker delivers into `room_idx` — either directly
/// (the marker's sector *is* the room's own sector) or onto an island pad
/// hosted inside it (`SectorOut::host == Some(room_idx)`). Shared by P26 and
/// P27, both of which treat a hosted-pad destination the same as a direct
/// one: the pad is still physically inside the room, so anything arriving on
/// it is inside the room too.
fn is_teleport_destination(out: &Compiled, room_idx: usize) -> bool {
    out.markers
        .iter()
        .any(|m| m.sector == room_idx || out.data.sectors[m.sector].host == Some(room_idx))
}

/// P26: an exit with `trigger: teleport` sits in a room with no portal and at
/// least one destination marker — the player arrives by teleport and steps
/// across the exit line (TNT MAP23's shape).
fn check_teleport_exit_rooms(ir: &Ir, out: &Compiled, v: &mut Vec<RuleViolation>) {
    for exit in ir
        .exits
        .iter()
        .filter(|e| e.trigger == ExitTrigger::Teleport)
    {
        let room_idx = ir
            .rooms
            .iter()
            .position(|r| r.id == exit.room)
            .expect("validated");
        if ir
            .portals
            .iter()
            .any(|p| p.a == exit.room || p.b == exit.room)
        {
            v.push(RuleViolation {
                rule: "P26",
                subject: exit.room.clone(),
                detail: "a teleport exit's room must have no portal; the player arrives by \
                          teleport only"
                    .to_owned(),
            });
        }
        if !is_teleport_destination(out, room_idx) {
            v.push(RuleViolation {
                rule: "P26",
                subject: exit.room.clone(),
                detail: "a teleport exit's room holds no teleport destination".to_owned(),
            });
        }
    }
}

/// P27: no sealed monster room — a room holding a monster has a portal or is
/// a teleport destination, so sight or sound can ever reach it.
///
/// Both of retail's release shapes are now buildable, and either satisfies
/// this rule. A monsters-only teleport pad makes the pen a teleport
/// destination; a **drop wall** gives it a portal, since `has_portal` below
/// counts every [`PortalKind`] and a drop wall is one — a sealed strip
/// between the pen and the room in front of it, lowered by a trigger placed
/// anywhere. What is still out of the vocabulary is retail's *other* release
/// strip, a remote special aimed at a zero-height sliver beside the pen
/// (62/36/109/20/2/123/103/102 in the corpus, none of them emittable), and
/// so is a `RevealKind::Closet` with a monster inside it — a closet rests
/// with its floor at its ceiling, so nothing fits in it
/// ([`CompileError::RevealNoHeadroom`](crate::compile::CompileError::RevealNoHeadroom)).
/// A pen with neither a portal nor a pad still has no release at all, which
/// is what this refuses.
fn check_sealed_monster_rooms(
    ir: &Ir,
    tables: &Tables,
    out: &Compiled,
    v: &mut Vec<RuleViolation>,
) {
    for (i, room) in ir.rooms.iter().enumerate() {
        if !room
            .things
            .iter()
            .any(|t| tables.species(&t.kind).is_some())
        {
            continue;
        }
        let has_portal = ir.portals.iter().any(|p| p.a == room.id || p.b == room.id);
        if !has_portal && !is_teleport_destination(out, i) {
            v.push(RuleViolation {
                rule: "P27",
                subject: room.id.clone(),
                detail: "holds monsters but has no portal and is no teleport destination; \
                          nothing can ever wake them"
                    .to_owned(),
            });
        }
    }
}

/// P5: every platform travels more than a step, lowers to the floor of the
/// sector(s) it serves, and can be called from that floor. Re-derived from
/// the emitted geometry the way `EV_DoPlat` reads it: `low` is
/// [`lowest_floor_surrounding`], the same `P_FindLowestFloorSurrounding` walk
/// P28 runs over a floor target, and a use special fires from its front
/// sector only (`P_UseSpecialLine`) while a walkover fires from whichever
/// side can cross at rest (`P_TryMove`'s step rule).
fn check_lift_return(tables: &Tables, out: &Compiled, v: &mut Vec<RuleViolation>) {
    let step = tables.step_height();
    let use_specials = tables.lift_use_specials();
    let walk_specials = tables.lift_walkover_specials();
    let floor = |s: usize| out.data.sectors[s].floor;
    for lift in &out.lifts {
        let subject = format!("sector {} ({:?} tag {})", lift.sector, lift.shape, lift.tag);
        let low = lowest_floor_surrounding(&out.data, lift.sector);
        // Which neighbor `low` came from, for the message below. Only read
        // past the `travel <= step` guard, and past it some neighbor is
        // strictly below the platform's own floor — otherwise `low` would be
        // that floor and `travel` zero — so this is `Some`, and the sector it
        // names stands at `low`. Ties go to the lowest sector index
        // (`min_by_key` keeps the first minimum) rather than to whichever
        // linedef the emitter happened to write first.
        let lowest_neighbor = emitted_neighbors(&out.data, lift.sector)
            .into_iter()
            .min_by_key(|&n| floor(n));
        let travel = floor(lift.sector) - low;
        if travel <= step {
            v.push(RuleViolation {
                rule: "P5",
                subject: subject.clone(),
                detail: format!("the platform travels {travel}, no more than the {step}-unit step"),
            });
            continue;
        }
        for &c in &lift.callable_from {
            if floor(c) != low {
                v.push(RuleViolation {
                    rule: "P5",
                    subject: subject.clone(),
                    detail: format!(
                        "sector {c} calls the lift from floor {}, but its lowest neighbor is sector {} at {low}: the platform will not stop at the caller",
                        floor(c),
                        lowest_neighbor.unwrap_or(c)
                    ),
                });
            }
        }
        let callable_from_low = out.data.linedefs.iter().any(|line| {
            if line.tag != lift.tag {
                return false;
            }
            let f = out.data.sidedefs[line.front].sector;
            let b = line.back.map(|s| out.data.sidedefs[s].sector);
            if use_specials.contains(&line.special) {
                floor(f) == low
            } else if walk_specials.contains(&line.special) {
                let Some(b) = b else { return false };
                (floor(f) == low && floor(b) - floor(f) <= step)
                    || (floor(b) == low && floor(f) - floor(b) <= step)
            } else {
                false
            }
        });
        if !callable_from_low {
            v.push(RuleViolation {
                rule: "P5",
                subject,
                detail: "no trigger fires from the low floor: the lift is callable only from \
                          above, a trap for a player below"
                    .to_owned(),
            });
        }
    }
}

/// P28: every floor action's destination, re-derived over the emitted
/// geometry the way `EV_DoFloor` (`p_floor.c`) reads it, is the floor the
/// construct intends — [`lowest_floor_surrounding`] for a lowering action,
/// [`next_highest_floor`] for a rising one.
///
/// The compiler asserts a destination while it builds each construct; this
/// re-derives it from the records that actually shipped, so a neighbor some
/// later pass added or moved is caught rather than assumed away.
fn check_floor_destinations(out: &Compiled, v: &mut Vec<RuleViolation>) {
    for f in &out.floors {
        // Both searches start from the target's *own* emitted floor, and P29
        // reads the recorded `rest` instead, so a record that disagrees with
        // the geometry would let each rule judge a different map. Checked
        // first and reported alone: every number the destination check would
        // then print is derived from a floor that should not be there, so
        // stopping here is the same call P5 makes at a platform that cannot
        // travel.
        let emitted = out.data.sectors[f.sector].floor;
        if emitted != f.rest {
            v.push(RuleViolation {
                rule: "P28",
                subject: format!("sector {}", f.sector),
                detail: format!(
                    "the {:?} is recorded at rest {} but its emitted sector floor is {emitted}",
                    f.shape, f.rest
                ),
            });
            continue;
        }
        let (engine, search) = match f.family {
            FloorFamily::LowerToLowest => (
                lowest_floor_surrounding(&out.data, f.sector),
                "P_FindLowestFloorSurrounding",
            ),
            FloorFamily::RaiseToNearest => (
                next_highest_floor(&out.data, f.sector),
                "P_FindNextHighestFloor",
            ),
        };
        if engine != f.dest {
            v.push(RuleViolation {
                rule: "P28",
                subject: format!("sector {}", f.sector),
                detail: format!(
                    "the {:?} resting at {} intends floor {}, but {search} over its emitted \
                     neighbors lands on {engine}",
                    f.shape, f.rest, f.dest
                ),
            });
        }
    }
}

/// P29: a floor action changes where the player can walk, in the direction
/// its shape promises — a drop wall or a bridge passes both ways between its
/// two neighbors once it has moved, and a reveal is sealed against its host
/// at rest and enterable from it afterward.
///
/// Both halves read the same two refusals [`crate::reach`] does: the player
/// crosses only where `P_LineOpening`'s window (`p_maputl.c:300-329`) — the
/// lower of the two ceilings over the higher of the two floors — holds their
/// full height (`p_map.c:468-469`), and only where the climb is no greater
/// than the step (`p_map.c:477-479`). The heights fed in are the *effective*
/// ones: a target's `rest` before it fires and its `dest` after, its
/// neighbors' own floors throughout.
fn check_floor_openings(tables: &Tables, out: &Compiled, v: &mut Vec<RuleViolation>) {
    let step = tables.step_height();
    let h = tables.player().height;
    // Whether the player can walk from `a` at `a_floor` onto `b` at
    // `b_floor`. `P_LineOpening`'s window — the lower of the two ceilings
    // over the higher of the two floors — already subsumes each sector's own
    // standing room, so the height test is stated once rather than three
    // times, and this reads as exactly what `reach::passable`'s `Open` arm
    // does over flood nodes.
    let pass = |data: &MapData, a: usize, a_floor: i32, b: usize, b_floor: i32| -> bool {
        b_floor - a_floor <= step
            && data.sectors[a].ceiling.min(data.sectors[b].ceiling) - a_floor.max(b_floor) >= h
    };
    for f in &out.floors {
        let n: Vec<usize> = emitted_neighbors(&out.data, f.sector).into_iter().collect();
        let floor = |s: usize| out.data.sectors[s].floor;
        // A neighbor as the message needs it: which sector, and the two
        // heights the window above was computed from.
        let named = |x: usize| {
            format!(
                "sector {x} (floor {}, ceiling {})",
                out.data.sectors[x].floor, out.data.sectors[x].ceiling
            )
        };
        let reason = match f.shape {
            FloorShape::DropWall | FloorShape::Bridge => {
                if n.len() == 2 {
                    n.iter().find_map(|&x| {
                        if !pass(&out.data, x, floor(x), f.sector, f.dest) {
                            Some(format!(
                                "at destination {}, {} cannot cross onto it",
                                f.dest,
                                named(x)
                            ))
                        } else if !pass(&out.data, f.sector, f.dest, x, floor(x)) {
                            Some(format!(
                                "at destination {}, it cannot cross onto {}",
                                f.dest,
                                named(x)
                            ))
                        } else {
                            None
                        }
                    })
                } else {
                    Some(format!(
                        "it has {} two-sided neighbors ({n:?}), not the two passages it joins",
                        n.len()
                    ))
                }
            }
            FloorShape::Closet | FloorShape::Pedestal => {
                if let [host] = *n.as_slice() {
                    if pass(&out.data, host, floor(host), f.sector, f.rest) {
                        Some(format!(
                            "at rest {}, its host {} can already cross onto it, so it is not \
                             sealed",
                            f.rest,
                            named(host)
                        ))
                    } else if !pass(&out.data, host, floor(host), f.sector, f.dest) {
                        Some(format!(
                            "at destination {}, its host {} still cannot cross onto it",
                            f.dest,
                            named(host)
                        ))
                    } else {
                        None
                    }
                } else {
                    Some(format!(
                        "it has {} two-sided neighbors ({n:?}), not the one host it is carved \
                         into",
                        n.len()
                    ))
                }
            }
        };
        if let Some(reason) = reason {
            v.push(RuleViolation {
                rule: "P29",
                subject: format!("sector {}", f.sector),
                detail: format!(
                    "the {:?} (ceiling {}, rest {}, destination {}) does not open as intended: \
                     {reason}, against the {h}-unit player height and the {step}-unit step",
                    f.shape, out.data.sectors[f.sector].ceiling, f.rest, f.dest
                ),
            });
        }
    }
}

/// P30: no floor action's target borders another action's target, a lift
/// platform, or a door sector.
///
/// Both destination searches read the *current* floors of the target's
/// neighbors, so a neighbor that moves makes the destination a function of
/// when the trigger is pulled; a load-time destination is exact only without
/// such a chain (`docs/measurements/floor-shapes-2026-09-02.md` §G).
///
/// A door sector is read off the **back** side of a door line rather than off
/// a tag, because every door special this vocabulary names
/// ([`Tables::door_special`] and [`Tables::locked_door_kinds`]) is a manual
/// `DR` form: `EV_VerticalDoor` (`p_doors.c`) takes
/// `sides[line->sidenum[1]].sector`, the back side, and such a line carries
/// no tag at all. [`crate::check::floors`]'s `mover_sectors` reads the
/// verifier's own `Scene` the same way.
fn check_floor_chains(tables: &Tables, out: &Compiled, v: &mut Vec<RuleViolation>) {
    let mut door_specials = vec![tables.door_special()];
    door_specials.extend(tables.locked_door_kinds().into_iter().map(|(_, s)| s));
    let movers: BTreeSet<usize> = out
        .floors
        .iter()
        .map(|f| f.sector)
        .chain(out.lifts.iter().map(|l| l.sector))
        .chain(
            out.data
                .linedefs
                .iter()
                .filter(|l| door_specials.contains(&l.special))
                .filter_map(|l| l.back)
                .map(|b| out.data.sidedefs[b].sector),
        )
        .collect();
    for f in &out.floors {
        for n in emitted_neighbors(&out.data, f.sector) {
            if movers.contains(&n) {
                v.push(RuleViolation {
                    rule: "P30",
                    subject: format!("sector {}", f.sector),
                    detail: format!(
                        "the {:?} bound for floor {} borders moving sector {n}, whose own floor \
                         at {} is what the engine's search would read",
                        f.shape, f.dest, out.data.sectors[n].floor
                    ),
                });
            }
        }
    }
}

/// The two-sided neighbors of an emitted sector — `getNextSector`
/// (`p_spec.c`) read over [`MapData`]: every linedef with a back side, in
/// both directions.
///
/// The rule layer's own reading, rather than [`crate::check::floors`]'s: that
/// one walks a `Scene` recovered from a built WAD, and this one walks the
/// records the compiler is about to emit.
fn emitted_neighbors(data: &MapData, sector: usize) -> BTreeSet<usize> {
    let mut n = BTreeSet::new();
    for l in &data.linedefs {
        let Some(back) = l.back else { continue };
        let (f, b) = (data.sidedefs[l.front].sector, data.sidedefs[back].sector);
        if f == sector && b != sector {
            n.insert(b);
        }
        if b == sector && f != sector {
            n.insert(f);
        }
    }
    n
}

/// `P_FindLowestFloorSurrounding` (`p_spec.c:270-289`): the least floor over
/// a sector's two-sided neighbors, starting at the sector's own floor — so a
/// sector already lower than every neighbor stays where it is.
fn lowest_floor_surrounding(data: &MapData, sector: usize) -> i32 {
    emitted_neighbors(data, sector)
        .into_iter()
        .fold(data.sectors[sector].floor, |lo, n| {
            lo.min(data.sectors[n].floor)
        })
}

/// `P_FindNextHighestFloor` (`p_spec.c:329-375`): the least neighboring floor
/// strictly above the sector's current one, or that current floor when no
/// neighbor stands above it.
///
/// The engine's `MAX_ADJOINING_SECTORS` cap of 20 (`p_spec.c:326`) cannot
/// bite anything this compiler emits — a bridge has exactly two two-sided
/// neighbors, its two rooms.
fn next_highest_floor(data: &MapData, sector: usize) -> i32 {
    let cur = data.sectors[sector].floor;
    emitted_neighbors(data, sector)
        .into_iter()
        .map(|n| data.sectors[n].floor)
        .filter(|&f| f > cur)
        .min()
        .unwrap_or(cur)
}

#[cfg(test)]
mod tests {
    use crate::compile::{
        CompileError, LinedefOut, MapData, SectorOut, SidedefOut, compile, compile_reporting,
    };
    use crate::ir::Ir;
    use crate::rules::{
        RuleViolation, check_all, check_lift_return, check_missing_textures,
        check_teleport_pairing, emitted_neighbors, lowest_floor_surrounding, next_highest_floor,
    };
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

    /// A key can be island cargo rather than a room thing, and both key-aware
    /// rules have to see it there: P24 (is it placed at all?) and P7 (can the
    /// player hold it before the door?). The pedestal's card is picked up by
    /// standing on the platform once it has been called down, so its node is
    /// the *pedestal's* sector, not the host room's.
    ///
    /// Muralla — `tests/fixtures/muralla_base.json` — is the reveal half of
    /// the same thing end to end; this is the pedestal half, and the smallest
    /// map that has it: a start, a pedestal carrying the only blue card, the
    /// door it opens, and an exit beyond so the P7 flood has a goal at all.
    #[test]
    fn a_key_placed_on_a_pedestal_is_placed_for_p24_and_held_for_p7() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]],
              "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"b", "footprint":[[576,0],[576,512],[1088,512],[1088,0]],
              "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"locked", "lock":"blue_card", "width":64, "at":[512,256],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "pedestals":[
            { "id":"prize", "room":"a", "at":[256,256], "rise":64,
              "things":[{ "kind":"blue_card", "at":[288,288], "angle":0 }] }
          ],
          "exits":[{ "room":"b", "trigger":"switch", "at":[1088,256], "width":64 }] }"#;
        assert!(
            violations(ir_json).is_empty(),
            "a card on a pedestal is a placed, reachable key: {:?}",
            violations(ir_json)
        );
    }

    /// The reveal half of the rule above, in the same minimal shape: the blue
    /// card is sealed on a pedestal *reveal* that a switch calls down. P7 has
    /// to grant the key only after that switch is used — the reveal rests 64
    /// above the floor, too tall to step onto — and P24 has to count it as
    /// placed at all.
    #[test]
    fn a_key_sealed_in_a_reveal_is_placed_for_p24_and_held_for_p7() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]],
              "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"b", "footprint":[[576,0],[576,512],[1088,512],[1088,0]],
              "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"locked", "lock":"blue_card", "width":64, "at":[512,256],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "triggers":[{ "id":"t", "kind":"switch", "room":"a", "at":[0,256] }],
          "reveals":[
            { "id":"prize", "room":"a", "at":[256,256], "kind":"pedestal", "rise":64,
              "things":[{ "kind":"blue_card", "at":[288,288], "angle":0 }], "trigger":"t" }
          ],
          "exits":[{ "room":"b", "trigger":"switch", "at":[1088,256], "width":64 }] }"#;
        assert!(
            violations(ir_json).is_empty(),
            "a card on a reveal the switch lowers is a placed, reachable key: {:?}",
            violations(ir_json)
        );
    }

    /// The negative that the two tests above cannot state: a key on a reveal
    /// is held **only once the reveal fires**, so its bit belongs to the
    /// reveal's own node and not to the host room's.
    ///
    /// The map is the smallest one where the two placements disagree. The
    /// blue card is sealed in a reveal standing in the *start* room, and the
    /// switch that lowers it is in room `b` — behind the very door the card
    /// opens. Nothing can be done: no card without the switch, no switch
    /// without the door, no door without the card, and P7 must say so. Put
    /// the card's bit on the host instead and the flood hands it over the
    /// moment the player spawns, the door opens, and this map compiles clean
    /// — which is exactly the silent defect the placement prevents. (A
    /// pedestal cannot make this case: its platform is callable from its
    /// host unconditionally, so island and host are reachable together and
    /// the two placements agree.)
    #[test]
    fn a_key_on_a_reveal_is_held_only_once_the_reveal_fires() {
        let ir_json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"a", "footprint":[[0,0],[0,512],[512,512],[512,0]],
              "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"b", "footprint":[[576,0],[576,512],[1088,512],[1088,0]],
              "floor":0, "ceiling":192, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"a", "b":"b", "kind":"locked", "lock":"blue_card", "width":64, "at":[512,256],
                        "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "triggers":[{ "id":"t", "kind":"switch", "room":"b", "at":[832,512] }],
          "reveals":[
            { "id":"prize", "room":"a", "at":[256,256], "kind":"pedestal", "rise":64,
              "things":[{ "kind":"blue_card", "at":[288,288], "angle":0 }], "trigger":"t" }
          ],
          "exits":[{ "room":"b", "trigger":"switch", "at":[1088,256], "width":64 }] }"#;
        let found = violations(ir_json);
        assert!(
            found.contains(&"P7".to_owned()),
            "the card is unreachable until a switch behind its own lock fires: {found:?}"
        );
        // And P24 stays quiet: the card *is* placed and it *does* open a
        // door, so the only thing wrong with this map is when the player can
        // hold it. A P24 here would mean the placement scan had regressed
        // instead.
        assert!(
            !found.contains(&"P24".to_owned()),
            "the card is placed and opens a door; only its timing is wrong: {found:?}"
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

    /// Like the `violations` helper above, but returns the full
    /// `RuleViolation` list rather than just the rule ids — the P26/P27/P15
    /// tests below need `subject` too. Named differently to avoid colliding
    /// with the pre-existing `violations` (`Vec<String>`) helper used
    /// throughout the rest of this module.
    fn all_violations(json: &str) -> Vec<RuleViolation> {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(json).expect("ir");
        let (_, v) = crate::compile::compile_reporting(&ir, &tables).expect("geometry compiles");
        v
    }

    const TWO_ROOMS_HEAD: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[192,64], "angle":90 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3", "things":[ THINGS_B ] }
      ],"#;

    #[test]
    fn p26_a_teleport_exit_room_must_be_portal_less_with_a_marker() {
        let ok = format!(
            r#"{} "portals":[],
               "exits":[{{ "room":"b", "trigger":"teleport", "at":[448,256], "width":64 }}],
               "teleports":[{{ "id":"t", "room":"a", "pad":{{"island":[64,128]}}, "to":{{"room":"b","at":[448,128],"angle":90}} }}] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", "")
        );
        let clean = all_violations(&ok);
        assert!(clean.iter().all(|v| v.rule != "P26"), "{clean:?}");
        let with_portal = ok.replace(
            r#""portals":[]"#,
            r#""portals":[{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }]"#,
        );
        let v = all_violations(&with_portal);
        assert!(
            v.iter().any(|v| v.rule == "P26" && v.subject == "b"),
            "{v:?}"
        );
        // The rule's other half: `b` stays portal-less, but nothing
        // teleports into it, so the exit is unreachable by any means.
        let no_destination = ok.replace(
            r#""to":{"room":"b","at":[448,128],"angle":90}"#,
            r#""to":{"room":"a","at":[192,192],"angle":90}"#,
        );
        let v = all_violations(&no_destination);
        assert!(
            v.iter().any(|v| v.rule == "P26"
                && v.subject == "b"
                && v.detail.contains("holds no teleport destination")),
            "{v:?}"
        );
    }

    /// The hosted-pad disjunct of P26's destination check
    /// (`SectorOut::host == Some(room_idx)`): the exit's room `b` has no
    /// portal, and its only marker lands not on `b`'s own room sector but on
    /// an island pad hosted inside it — `t1` (triggered from `a`) delivers
    /// straight onto `t2`'s pad center in `b`, exactly the two-way-pair
    /// shape `compile::teleports::a_two_way_pair_tags_the_other_pad` already
    /// pins at the compiler level. No prior fixture exercised this branch of
    /// the rule; every other destination in this file lands directly in the
    /// room's own sector.
    #[test]
    fn p26_a_hosted_pad_destination_satisfies_the_exit_room() {
        let json = format!(
            r#"{} "portals":[],
               "exits":[{{ "room":"b", "trigger":"teleport", "at":[448,256], "width":64 }}],
               "teleports":[
                 {{ "id":"t1", "room":"a", "pad":{{"island":[64,128]}},
                    "to":{{"room":"b","at":[480,160],"angle":90}} }},
                 {{ "id":"t2", "room":"b", "pad":{{"island":[448,128]}},
                    "to":{{"room":"a","at":[200,200],"angle":0}} }}
               ] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", "")
        );
        let v = all_violations(&json);
        assert!(v.iter().all(|v| v.rule != "P26"), "{v:?}");
    }

    #[test]
    fn p27_a_sealed_room_with_monsters_is_rejected_unless_it_is_a_destination() {
        let sealed = format!(
            r#"{} "portals":[],
               "exits":[{{ "room":"a", "trigger":"switch", "at":[128,0], "width":64 }}],
               "teleports":[] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", r#"{ "kind":"imp", "at":[448,128], "angle":0 }"#)
        );
        let v = all_violations(&sealed);
        assert!(
            v.iter().any(|v| v.rule == "P27" && v.subject == "b"),
            "{v:?}"
        );
        let destination = sealed.replace(
            r#""teleports":[]"#,
            r#""teleports":[{ "id":"t", "room":"a", "pad":{"island":[64,128]}, "to":{"room":"b","at":[384,64],"angle":90} }]"#,
        );
        assert!(all_violations(&destination).iter().all(|v| v.rule != "P27"));
    }

    /// The hosted-pad disjunct of P27's destination check, mirroring
    /// `p26_a_hosted_pad_destination_satisfies_the_exit_room` above: room
    /// `b` is sealed (no portal) and holds an imp, but is a destination only
    /// because another teleport (`t1`, triggered from `a`) delivers onto
    /// `b`'s own island pad (`t2`'s), not into `b`'s room sector directly.
    /// The imp sits well clear of the pad's grown square.
    #[test]
    fn p27_a_hosted_pad_destination_satisfies_the_sealed_room() {
        let json = format!(
            r#"{} "portals":[],
               "exits":[{{ "room":"a", "trigger":"switch", "at":[128,0], "width":64 }}],
               "teleports":[
                 {{ "id":"t1", "room":"a", "pad":{{"island":[64,128]}},
                    "to":{{"room":"b","at":[480,160],"angle":90}} }},
                 {{ "id":"t2", "room":"b", "pad":{{"island":[448,128]}},
                    "to":{{"room":"a","at":[200,200],"angle":0}} }}
               ] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", r#"{ "kind":"imp", "at":[384,64], "angle":0 }"#)
        );
        let v = all_violations(&json);
        assert!(v.iter().all(|v| v.rule != "P27"), "{v:?}");
    }

    #[test]
    fn p15_holds_for_every_compiled_teleport() {
        // The compiler constructs pairing; P15 re-checks it on the emitted
        // data. A clean fixture yields no P15 violation.
        let json = format!(
            r#"{} "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }}],
               "exits":[{{ "room":"a", "trigger":"switch", "at":[128,0], "width":64 }}],
               "teleports":[{{ "id":"t", "room":"a", "pad":{{"island":[64,128]}}, "to":{{"room":"b","at":[448,128],"angle":90}} }}] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", "")
        );
        assert!(all_violations(&json).iter().all(|v| v.rule != "P15"));
    }

    #[test]
    fn p15_a_second_sector_sharing_the_trigger_tag_is_flagged() {
        // Mutate the emitted `Compiled` directly: give some other sector the
        // same tag as the teleport's trigger lines, so the tag no longer
        // resolves to exactly one sector. `check_teleport_pairing` walks
        // every trigger *linedef*, not every teleport, so an island pad's
        // four trigger edges (all sharing the one tag) each report their own
        // violation — one per trigger edge affected, not one per teleport.
        let json = format!(
            r#"{} "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }}],
               "exits":[{{ "room":"a", "trigger":"switch", "at":[128,0], "width":64 }}],
               "teleports":[{{ "id":"t", "room":"a", "pad":{{"island":[64,128]}}, "to":{{"room":"b","at":[448,128],"angle":90}} }}] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", "")
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (mut out, found) = crate::compile::compile_reporting(&ir, &tables).expect("compiles");
        assert!(!found.iter().any(|v| v.rule == "P15"), "{found:?}");

        let trigger_edges = out
            .data
            .linedefs
            .iter()
            .filter(|l| tables.teleport_specials().contains(&l.special))
            .count();
        let trigger_tag = out
            .data
            .linedefs
            .iter()
            .find(|l| tables.teleport_specials().contains(&l.special))
            .expect("a teleport trigger line")
            .tag;
        // Room `a` (sector 0) is not the destination sector; give it the
        // same tag so the trigger's tag now resolves to two sectors.
        out.data.sectors[0].tag = trigger_tag;

        let mut v = Vec::new();
        check_teleport_pairing(&tables, &out, &mut v);
        assert_eq!(
            v.len(),
            trigger_edges,
            "one P15 violation per trigger edge affected: {v:?}"
        );
        assert!(v.iter().all(|x| x.rule == "P15"));
    }

    #[test]
    fn p15_a_missing_marker_is_flagged() {
        let json = format!(
            r#"{} "portals":[{{ "a":"a", "b":"b", "kind":"plain", "width":128, "at":[256,128] }}],
               "exits":[{{ "room":"a", "trigger":"switch", "at":[128,0], "width":64 }}],
               "teleports":[{{ "id":"t", "room":"a", "pad":{{"island":[64,128]}}, "to":{{"room":"b","at":[448,128],"angle":90}} }}] }}"#,
            TWO_ROOMS_HEAD.replace("THINGS_B", "")
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (mut out, found) = crate::compile::compile_reporting(&ir, &tables).expect("compiles");
        assert!(!found.iter().any(|v| v.rule == "P15"), "{found:?}");

        let trigger_edges = out
            .data
            .linedefs
            .iter()
            .filter(|l| tables.teleport_specials().contains(&l.special))
            .count();
        out.markers.clear();

        let mut v = Vec::new();
        check_teleport_pairing(&tables, &out, &mut v);
        assert_eq!(
            v.len(),
            trigger_edges,
            "one P15 violation per trigger edge affected: {v:?}"
        );
        assert!(v.iter().all(|x| x.rule == "P15"));
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

    /// Compiles the `lifts` golden fixture (rooms `low`, `ledge`, `far`,
    /// `north` at sectors 0-3; platforms 4 (low<->ledge), 5 (the
    /// ledge<->far barrier), 7 (the low<->north walkover lift), and pedestal
    /// 8), returning the tables alongside the compiled output so P5 tests can
    /// damage `Compiled` by hand.
    fn lifts_compiled() -> (Tables, crate::compile::Compiled) {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(include_str!("../tests/golden/lifts.json")).expect("ir");
        let out = compile(&ir, &tables).expect("compiles");
        (tables, out)
    }

    #[test]
    fn p5_passes_on_the_lift_golden() {
        let (tables, out) = lifts_compiled();
        let mut v = Vec::new();
        check_lift_return(&tables, &out, &mut v);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn p5_reads_a_plat_boundary_from_whichever_side_carries_the_platform() {
        let (tables, mut out) = lifts_compiled();
        // Every plat boundary the compiler emits puts the platform on the
        // line's back side, so the neighbor scan's other arm never runs on
        // emitted geometry. The walkover lift's low face — platform to
        // alcove — carries no special, so which side it is drawn from is
        // nothing the engine reads: redraw it the other way round and P5
        // must reach the same verdict.
        let plats: Vec<usize> = out.lifts.iter().map(|l| l.sector).collect();
        let low_face = out
            .data
            .linedefs
            .iter()
            .position(|line| {
                let Some(back) = line.back else { return false };
                let (f, b) = (
                    out.data.sidedefs[line.front].sector,
                    out.data.sidedefs[back].sector,
                );
                line.special == 0
                    && plats.contains(&b)
                    && out.data.sectors[f].floor < out.data.sectors[b].floor
            })
            .expect("a special-free plat boundary down to the platform's lowest neighbor");
        let line = &mut out.data.linedefs[low_face];
        let back = line.back.expect("two-sided");
        line.back = Some(line.front);
        line.front = back;

        let mut v = Vec::new();
        check_lift_return(&tables, &out, &mut v);
        assert!(v.is_empty(), "{v:?}");
    }

    #[test]
    fn p5_catches_a_platform_that_travels_no_more_than_a_step() {
        let (tables, mut out) = lifts_compiled();
        let plat = out.lifts[0].sector;
        out.data.sectors[plat].floor = 24; // room `low` is at 0
        let mut v = Vec::new();
        check_lift_return(&tables, &out, &mut v);
        assert!(
            v.iter()
                .any(|x| x.rule == "P5" && x.detail.contains("travels 24")),
            "{v:?}"
        );
    }

    #[test]
    fn p5_catches_a_neighbor_lower_than_the_room_the_lift_serves() {
        let (tables, mut out) = lifts_compiled();
        // Sink the ledge (sector 1) below the low room: the engine now sends the
        // low<->ledge platform to -64, the ledge's floor, not to room `low` at 0,
        // which is the sector that calls it.
        out.data.sectors[1].floor = -64;
        let mut v = Vec::new();
        check_lift_return(&tables, &out, &mut v);
        assert!(
            v.iter()
                .any(|x| x.rule == "P5" && x.detail.contains("lowest neighbor is sector 1")),
            "{v:?}"
        );
    }

    #[test]
    fn p5_catches_a_lift_callable_only_from_above() {
        let (tables, mut out) = lifts_compiled();
        let low_line = out.lifts[0].low_line.unwrap();
        out.data.linedefs[low_line].special = 0; // strip the riser switch; the top-face walkover remains
        let mut v = Vec::new();
        check_lift_return(&tables, &out, &mut v);
        assert!(
            v.iter()
                .any(|x| x.rule == "P5" && x.detail.contains("only from above")),
            "{v:?}"
        );
    }
    /// Two rooms 64 units apart, sealed by a 16-deep drop wall that one
    /// switch on room `a`'s far wall lowers — a verbatim copy of
    /// `compile::floors`'s own `WALL` fixture, which lives in that module's
    /// private test module and so cannot be shared.
    const WALL_MAP: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":256, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"imp", "at":[448,128], "angle":180 } ] }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"drop_wall", "width":64, "at":[256,128], "thickness":16, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// Compiles [`WALL_MAP`] through [`compile`], which raises
    /// `CompileError::Playability` on any violation: a drop wall is a clean
    /// map under the whole catalog now that P7's flood carries a fired
    /// floor action in its state. Each mutation below then re-runs
    /// `check_all` over the damaged output.
    fn wall_compiled() -> (Tables, crate::compile::Compiled) {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let out = compile(&ir, &tables).expect("a drop wall is a legal map");
        (tables, out)
    }

    /// `hub` (start) has a dead-end pit 100 below it and a plain portal on
    /// to `b`, whose switch drops the wall sealing `c` and its exit. The map
    /// is finishable — go to `b` first — but the player who takes the pit
    /// branch is stranded, and takes it before ever reaching the switch. So
    /// the reported state is the one where the wall is still standing,
    /// which is what the violation has to say.
    const PIT_AND_WALL: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"pit", "footprint":[[0,320],[0,576],[256,576],[256,320]], "floor":-100, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[
        { "a":"hub", "b":"pit", "kind":"plain", "width":64, "at":[128,256] },
        { "a":"hub", "b":"b", "kind":"plain", "width":64, "at":[256,128] },
        { "a":"b", "b":"c", "kind":"drop_wall", "width":64, "at":[576,128], "thickness":16, "fires_on":"t" }
      ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"b", "at":[448,0] } ],
      "exits":[ { "room":"c", "trigger":"switch", "at":[896,128], "width":64 } ] }"#;

    #[test]
    fn p7_names_the_floor_actions_still_at_rest_in_a_stranded_state() {
        let v = p7_violations(PIT_AND_WALL);
        assert!(
            !v.iter().any(|x| x.detail.contains("no feasible walk")),
            "the switch route finishes the map: {v:?}"
        );
        let pit = v
            .iter()
            .find(|x| x.subject.contains("pit"))
            .unwrap_or_else(|| panic!("the pit strands: {v:?}"));
        assert_eq!(
            pit.detail,
            "the player can reach this sector but can no longer reach an exit from it; \
             drop wall b <-> c not lowered",
            "the state names the action still at rest, with the direction it moves: {v:?}"
        );
    }

    /// Four rooms in a row, with two lowering walkovers of the *same*
    /// family: `w1` on the `s`|`b` opening lowers an empty closet in `b`,
    /// and `w2` on the `c`|`d` opening lowers the drop wall between `b` and
    /// `c`. The only way to `w2` is through the very wall it lowers, so the
    /// map is unfinishable — but only because a walkover's special fires the
    /// trigger whose *tag* its line carries, and not every W1 line of the
    /// family.
    ///
    /// Both triggers therefore write the same special, 38, and differ only
    /// in their tag: exactly the discrimination `reach::wire_floor_actions`
    /// makes, and the one no other fixture on this branch exercises, since
    /// no other has two walkovers at all.
    const TWO_WALKOVERS: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"s", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":0, "ceiling":192, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"d", "footprint":[[960,0],[960,256],[1216,256],[1216,0]], "floor":0, "ceiling":192, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[
        { "a":"s", "b":"b", "kind":"plain", "width":64, "at":[256,128] },
        { "a":"b", "b":"c", "kind":"drop_wall", "width":64, "at":[576,128], "thickness":16, "fires_on":"w2" },
        { "a":"c", "b":"d", "kind":"plain", "width":64, "at":[896,128] }
      ],
      "triggers":[
        { "id":"w1", "kind":"walkover", "portal":["s","b"] },
        { "id":"w2", "kind":"walkover", "portal":["c","d"] }
      ],
      "reveals":[ { "id":"pen", "room":"b", "at":[448,128], "kind":"closet", "trigger":"w1" } ],
      "exits":[ { "room":"d", "trigger":"switch", "at":[1216,128], "width":64 } ] }"#;

    /// A walkover fires the action its own tag names, not every action of
    /// its family: crossing `w1` opens the closet in `b` and leaves the
    /// `b`|`c` wall standing, so nothing reaches `w2` and nothing reaches
    /// the exit.
    ///
    /// Dropping the tag test from `reach::wire_floor_actions`'s walkover
    /// wiring makes this map finishable and this assertion fail; without the
    /// fixture that deletion passes the whole suite.
    #[test]
    fn p7_refuses_a_map_whose_only_walkover_lies_behind_the_wall_it_lowers() {
        let v = p7_violations(TWO_WALKOVERS);
        assert!(
            v.iter()
                .any(|x| x.detail == "no feasible walk from the player start reaches an exit"),
            "the second walkover is sealed behind the wall it fires: {v:?}"
        );
    }

    /// `hub` (start) has the same dead-end pit [`PIT_AND_WALL`] does, but the
    /// way on to the exit is a **bridge** rather than a drop wall: a pit
    /// strip 96 below `b` and `c`, raised by a switch in `b`. So the action
    /// still at rest in the stranded state is a rising one, which the P7
    /// wording has to call "not raised" rather than "not lowered".
    const PIT_AND_BRIDGE: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"sink", "footprint":[[0,320],[0,576],[256,576],[256,320]], "floor":-100, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"c", "footprint":[[640,0],[640,256],[896,256],[896,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[
        { "a":"hub", "b":"sink", "kind":"plain", "width":64, "at":[128,256] },
        { "a":"hub", "b":"b", "kind":"plain", "width":64, "at":[256,128] },
        { "a":"b", "b":"c", "kind":"bridge", "width":64, "at":[576,128], "depth":96, "fires_on":"t" }
      ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"b", "at":[448,0] } ],
      "exits":[ { "room":"c", "trigger":"switch", "at":[896,128], "width":64 } ] }"#;

    #[test]
    fn p7_says_a_rising_action_is_not_raised() {
        let v = p7_violations(PIT_AND_BRIDGE);
        assert!(
            !v.iter().any(|x| x.detail.contains("no feasible walk")),
            "the switch route finishes the map: {v:?}"
        );
        assert!(
            v.iter()
                .any(|x| x.detail.ends_with("bridge b <-> c not raised")),
            "the rising family takes the other verb: {v:?}"
        );
    }

    /// [`WALL_MAP`] with room `a` a full step above room `b` — the only
    /// shape in which a fired drop wall's own side is the lower one at the
    /// boundary, and so the only one whose *fired* geometry P8 has anything
    /// to say about.
    const STEPPED_WALL_MAP: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":24, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[128,128], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":256, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"imp", "at":[448,128], "angle":180 } ] }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"drop_wall", "width":64, "at":[256,128], "thickness":16, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// P8 judges the map at load, where a drop wall stands at its ceiling and
    /// every face is drawn from the room side. Put the wall where the switch
    /// leaves it and the boundary flips: toward the passage a step above, the
    /// wall's own side is now the lower one and its lower is what `r_segs.c`
    /// draws. `emit_drop_wall` writes that slot, so P8 stays quiet on the
    /// fired geometry too — a HOM strip no golden can see, since every
    /// golden's drop wall joins rooms that are level.
    #[test]
    fn a_fired_drop_wall_between_unlevel_rooms_leaves_no_untextured_face() {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(STEPPED_WALL_MAP).expect("ir");
        let mut out = compile(&ir, &tables).expect("a stepped drop wall is a legal map");
        assert_eq!(
            (out.floors[0].rest, out.floors[0].dest),
            (192, 0),
            "the wall rests at room `a`'s ceiling and falls to room `b`'s floor"
        );
        let wall = out.floors[0].sector;
        out.data.sectors[wall].floor = out.floors[0].dest;
        let mut v = Vec::new();
        check_missing_textures(&out, &mut v);
        assert!(v.is_empty(), "the fired wall shows no blank face: {v:?}");
    }

    #[test]
    fn p28_p29_p30_pass_on_a_compiled_drop_wall() {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let out = compile(&ir, &tables).expect("a drop wall is a legal map");
        assert!(
            check_all(&ir, &tables, &out).is_empty(),
            "a drop wall breaks no rule at all, the three floor rules included"
        );
        assert_eq!(out.floors.len(), 1);
    }

    #[test]
    fn p28_fails_when_the_emitted_destination_is_not_the_intended_floor() {
        let (tables, mut out) = wall_compiled();
        // Sink the "after" passage below the room: the engine's own search
        // now lands 64 under the floor the wall was meant to come to rest on.
        let wall = out.floors[0].sector;
        let after = emitted_neighbors(&out.data, wall)
            .into_iter()
            .max()
            .expect("the wall has two passage neighbors");
        out.data.sectors[after].floor = -64;
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let v = check_all(&ir, &tables, &out);
        assert!(
            v.iter()
                .any(|x| x.rule == "P28" && x.detail.contains("lands on -64")),
            "{v:?}"
        );
    }

    #[test]
    fn p29_fails_when_a_lowered_wall_still_blocks_one_side() {
        let (tables, mut out) = wall_compiled();
        let wall = out.floors[0].sector;
        // Drop the wall's ceiling so that, lowered, the crossing window is
        // under the player's height on the way through. The floor and the
        // recorded `rest` come down with it — the floor because a sector
        // whose floor stood above its ceiling is not a map the engine could
        // load, and `rest` because P28 now cross-checks it against the
        // emitted floor: moving both keeps the damage under test the opening
        // alone rather than the bookkeeping.
        out.data.sectors[wall].ceiling = 40;
        out.data.sectors[wall].floor = 40;
        out.floors[0].rest = 40;
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let v = check_all(&ir, &tables, &out);
        let hit = v
            .iter()
            .find(|x| x.rule == "P29")
            .unwrap_or_else(|| panic!("expected a P29, got {v:?}"));
        // The destination and the condition that failed, not just the id: at
        // destination 0 the 40-tall window is 16 under the 56-unit player, so
        // the first of the wall's two passages cannot cross onto it.
        let before = emitted_neighbors(&out.data, wall)
            .into_iter()
            .min()
            .expect("the wall has two passage neighbors");
        assert!(
            hit.detail
                .contains("the DropWall (ceiling 40, rest 40, destination 0)"),
            "{hit}"
        );
        assert!(
            hit.detail.contains(&format!(
                "at destination 0, sector {before} (floor 0, ceiling 192) cannot cross onto it"
            )),
            "{hit}"
        );
        assert!(
            v.iter().all(|x| x.rule != "P28"),
            "the rest cross-check stays satisfied: {v:?}"
        );
    }

    #[test]
    fn p28_fails_when_the_emitted_floor_is_not_the_recorded_rest() {
        let (tables, mut out) = wall_compiled();
        // The record still says the wall is up; the emitted sector says it is
        // already down. Both searches read the sector, P29 reads the record,
        // so without this check each rule would judge a different map — and
        // the destination search still lands on 0, so nothing else catches it.
        let wall = out.floors[0].sector;
        out.data.sectors[wall].floor = 0;
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let v = check_all(&ir, &tables, &out);
        assert!(
            v.iter().any(|x| x.rule == "P28"
                && x.detail
                    .contains("recorded at rest 192 but its emitted sector floor is 0")),
            "{v:?}"
        );
    }

    /// One room hosting an empty closet that a switch on its west wall
    /// lowers, plus a second room past a plain portal to hold the exit — the
    /// reveal counterpart of [`WALL_MAP`], and the only shape whose P29 half
    /// is the sealed-at-rest one.
    ///
    /// The second room is not decoration: it gives the map a sector that is
    /// neither the cell nor its host, which is what the neighbor-count
    /// mutation below needs to graft onto the cell.
    const CLOSET_MAP: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":144,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"plain", "width":64, "at":[256,128] } ],
      "triggers":[ { "id":"t", "kind":"switch", "room":"a", "at":[0,128] } ],
      "reveals":[ { "id":"pen", "room":"a", "at":[128,128], "kind":"closet", "trigger":"t" } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// [`wall_compiled`] for [`CLOSET_MAP`].
    fn closet_compiled() -> (Tables, crate::compile::Compiled) {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(CLOSET_MAP).expect("ir");
        let out = compile(&ir, &tables).expect("a closet reveal is a legal map");
        (tables, out)
    }

    /// The P29 violation `check_all` reports, or a panic naming what it found
    /// instead.
    fn one_p29(ir: &str, tables: &Tables, out: &crate::compile::Compiled) -> RuleViolation {
        let ir = Ir::from_json(ir).expect("ir");
        let v = check_all(&ir, tables, out);
        v.iter()
            .find(|x| x.rule == "P29")
            .unwrap_or_else(|| panic!("expected a P29, got {v:?}"))
            .clone()
    }

    /// Grafts one more two-sided linedef between `sector` and `other`,
    /// borrowing a real pair of vertices so the record is well formed. P29
    /// counts a target's two-sided neighbors and reads nothing else off the
    /// line, which is exactly what this damages.
    fn graft_neighbor(data: &mut MapData, sector: usize, other: usize) {
        let (v1, v2) = (data.linedefs[0].v1, data.linedefs[0].v2);
        let front = data.sidedefs.len();
        for s in [sector, other] {
            let lower = data.sectors[s].wall_tex.clone();
            data.sidedefs.push(SidedefOut {
                sector: s,
                upper: lower.clone(),
                middle: String::new(),
                lower,
                x_offset: 0,
            });
        }
        data.linedefs.push(LinedefOut {
            v1,
            v2,
            front,
            back: Some(front + 1),
            blocking: false,
            special: 0,
            tag: 0,
            lower_unpegged: false,
            upper_unpegged: false,
            secret: false,
        });
    }

    #[test]
    fn p29_fails_when_a_reveal_is_already_open_at_rest() {
        let (tables, mut out) = closet_compiled();
        let cell = out.floors[0].sector;
        let host = *emitted_neighbors(&out.data, cell)
            .iter()
            .next()
            .expect("a closet has one host");
        // Rest the cell flush with its host instead of at the host's
        // ceiling. The record moves with the emitted floor so that P28's
        // rest cross-check stays satisfied and the damage under test is the
        // seal alone — as in `p29_fails_when_a_lowered_wall_still_blocks_one_side`.
        out.data.sectors[cell].floor = 0;
        out.floors[0].rest = 0;
        let hit = one_p29(CLOSET_MAP, &tables, &out);
        assert_eq!(
            hit.detail,
            format!(
                "the Closet (ceiling 192, rest 0, destination 0) does not open as intended: at \
                 rest 0, its host sector {host} (floor 0, ceiling 192) can already cross onto it, \
                 so it is not sealed, against the 56-unit player height and the 24-unit step"
            )
        );
    }

    #[test]
    fn p29_fails_when_a_lowered_reveal_is_still_under_the_players_height() {
        let (tables, mut out) = closet_compiled();
        let cell = out.floors[0].sector;
        let host = *emitted_neighbors(&out.data, cell)
            .iter()
            .next()
            .expect("a closet has one host");
        // A 40-unit lid: still sealed at rest (the 40-unit step up exceeds
        // the 24-unit one), and lowered it leaves a 40-unit window under the
        // 56-unit player, so the host never gets in.
        out.data.sectors[cell].ceiling = 40;
        out.data.sectors[cell].floor = 40;
        out.floors[0].rest = 40;
        let hit = one_p29(CLOSET_MAP, &tables, &out);
        assert_eq!(
            hit.detail,
            format!(
                "the Closet (ceiling 40, rest 40, destination 0) does not open as intended: at \
                 destination 0, its host sector {host} (floor 0, ceiling 192) still cannot cross \
                 onto it, against the 56-unit player height and the 24-unit step"
            )
        );
    }

    #[test]
    fn p29_fails_when_a_dropped_wall_cannot_be_left_on_the_far_side() {
        let (tables, mut out) = wall_compiled();
        let wall = out.floors[0].sector;
        // Raise the far passage a step and a half above the wall's
        // destination. Walking *into* the dropped wall still works — descent
        // is free — but the climb out the far side is 40, so the wall opens
        // one way only. The near passage is still at 0, so
        // `P_FindLowestFloorSurrounding` still lands on 0 and P28 stays
        // satisfied: this is the direction P29 alone can see.
        let after = emitted_neighbors(&out.data, wall)
            .into_iter()
            .max()
            .expect("the wall has two passage neighbors");
        out.data.sectors[after].floor = 40;
        let hit = one_p29(WALL_MAP, &tables, &out);
        assert_eq!(
            hit.detail,
            format!(
                "the DropWall (ceiling 192, rest 192, destination 0) does not open as intended: \
                 at destination 0, it cannot cross onto sector {after} (floor 40, ceiling 256), \
                 against the 56-unit player height and the 24-unit step"
            )
        );
    }

    #[test]
    fn p29_fails_when_a_dropped_wall_has_a_third_neighbor() {
        let (tables, mut out) = wall_compiled();
        let wall = out.floors[0].sector;
        let n = emitted_neighbors(&out.data, wall);
        let stranger = (0..out.data.sectors.len())
            .find(|s| *s != wall && !n.contains(s))
            .expect("the map has a sector that is neither the wall nor a passage");
        graft_neighbor(&mut out.data, wall, stranger);
        let mut all: Vec<usize> = n.iter().copied().collect();
        all.push(stranger);
        all.sort_unstable();
        let hit = one_p29(WALL_MAP, &tables, &out);
        assert!(
            hit.detail.contains(&format!(
                "it has 3 two-sided neighbors ({all:?}), not the two passages it joins"
            )),
            "{hit}"
        );
    }

    #[test]
    fn p29_fails_when_a_reveal_has_a_second_neighbor() {
        let (tables, mut out) = closet_compiled();
        let cell = out.floors[0].sector;
        let n = emitted_neighbors(&out.data, cell);
        let stranger = (0..out.data.sectors.len())
            .find(|s| *s != cell && !n.contains(s))
            .expect("the map has a sector that is neither the cell nor its host");
        graft_neighbor(&mut out.data, cell, stranger);
        let mut all: Vec<usize> = n.iter().copied().collect();
        all.push(stranger);
        all.sort_unstable();
        let hit = one_p29(CLOSET_MAP, &tables, &out);
        assert!(
            hit.detail.contains(&format!(
                "it has 2 two-sided neighbors ({all:?}), not the one host it is carved into"
            )),
            "{hit}"
        );
    }

    #[test]
    fn p30_fails_when_a_target_borders_a_door_sector() {
        let (tables, mut out) = wall_compiled();
        let wall = out.floors[0].sector;
        // The door clause reads a door sector off the *back* of a door line
        // — `EV_VerticalDoor` takes `sides[line->sidenum[1]].sector` — so
        // claiming the far passage's own outer threshold as a manual door
        // makes that passage a door sector without touching any tag.
        let after = emitted_neighbors(&out.data, wall)
            .into_iter()
            .max()
            .expect("the wall has two passage neighbors");
        let threshold = (0..out.data.linedefs.len())
            .find(|&i| {
                out.data.linedefs[i]
                    .back
                    .is_some_and(|b| out.data.sidedefs[b].sector == after)
            })
            .expect("the far passage fronts room b across its own threshold");
        out.data.linedefs[threshold].special = tables.door_special();
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let v = check_all(&ir, &tables, &out);
        assert!(
            v.iter()
                .any(|x| x.rule == "P30"
                    && x.detail.contains(&format!("borders moving sector {after}"))),
            "{v:?}"
        );
    }

    #[test]
    fn p30_fails_when_a_target_borders_a_platform() {
        let (tables, mut out) = wall_compiled();
        let wall = out.floors[0].sector;
        // The wall's own two neighbors are its passages, so the chain is
        // built by giving one of them a mover of its own: claim the "after"
        // passage as a platform, which is what P30 refuses next to a target.
        let after = emitted_neighbors(&out.data, wall)
            .into_iter()
            .max()
            .expect("the wall has two passage neighbors");
        out.lifts.push(crate::compile::lifts::LiftOut {
            sector: after,
            shape: crate::compile::lifts::LiftShape::Lift,
            travel: 64,
            callable_from: Vec::new(),
            tag: 99,
            portal: None,
            pedestal: None,
            low_line: None,
            top_line: None,
        });
        let ir = Ir::from_json(WALL_MAP).expect("ir");
        let v = check_all(&ir, &tables, &out);
        assert!(
            v.iter()
                .any(|x| x.rule == "P30" && x.detail.contains(&format!("sector {after}"))),
            "{v:?}"
        );
    }
    /// A bare [`MapData`] of one sector per entry in `floors`, joined by one
    /// two-sided linedef per pair in `joins`.
    ///
    /// Enough for the two destination searches, which read nothing but sector
    /// floors and the sector each sidedef belongs to — no vertices, no
    /// textures, no geometry. Hand-built rather than compiled because the
    /// cases below are ones the emitters cannot produce: the compiler never
    /// leaves a bridge with no neighbor above it.
    fn neighbor_graph(floors: &[i32], joins: &[(usize, usize)]) -> MapData {
        let mut data = MapData {
            sectors: floors
                .iter()
                .map(|&floor| SectorOut {
                    floor,
                    ceiling: 128,
                    light: 160,
                    floor_tex: "F".to_owned(),
                    ceil_tex: "C".to_owned(),
                    special: 0,
                    tag: 0,
                    wall_tex: "W".to_owned(),
                    host: None,
                })
                .collect(),
            ..MapData::default()
        };
        for &(a, b) in joins {
            let front = data.sidedefs.len();
            for sector in [a, b] {
                data.sidedefs.push(SidedefOut {
                    sector,
                    upper: String::new(),
                    middle: String::new(),
                    lower: String::new(),
                    x_offset: 0,
                });
            }
            data.linedefs.push(LinedefOut {
                v1: 0,
                v2: 0,
                front,
                back: Some(front + 1),
                blocking: false,
                special: 0,
                tag: 0,
                lower_unpegged: false,
                upper_unpegged: false,
                secret: false,
            });
        }
        data
    }

    /// `P_FindNextHighestFloor`'s two edges, neither of which an emitted map
    /// exercises: a neighbor *level* with the current floor is not above it,
    /// and with nothing above the search returns the current floor rather
    /// than the least neighbor.
    #[test]
    fn the_next_highest_search_is_strict_and_falls_back_to_the_current_floor() {
        // Sector 0 at 0, joined to one neighbor level with it and one above.
        let data = neighbor_graph(&[0, 0, 64], &[(0, 1), (0, 2)]);
        assert_eq!(
            next_highest_floor(&data, 0),
            64,
            "sector 1 is level with the current floor, so `other->floorheight > height` skips it \
             and the answer is sector 2's 64, not 0"
        );

        // Now nothing stands above: `if (!h) return currentheight` returns
        // the floor the sector already has, so the action is a no-op rather
        // than a drop to the least neighbor.
        let flat = neighbor_graph(&[32, 0, 32], &[(0, 1), (0, 2)]);
        assert_eq!(
            next_highest_floor(&flat, 0),
            32,
            "no neighbor is above 32, so the search returns 32 — not sector 1's 0"
        );

        // The same graph read the other way, for contrast: the lowest search
        // does take that lower neighbor, and starts at the sector's own floor.
        assert_eq!(lowest_floor_surrounding(&flat, 0), 0);
        assert_eq!(
            lowest_floor_surrounding(&data, 0),
            0,
            "nothing is below 0, so `P_FindLowestFloorSurrounding` returns the sector's own floor"
        );
    }
}
