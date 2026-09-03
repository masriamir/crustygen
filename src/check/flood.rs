//! V-P7 (no softlock), V-P24 (key/lock coherence), and the reachability half
//! of V-P20 (pickup accessibility): the flood re-derived over *parsed*
//! geometry rather than the compiler's own IR.
//!
//! [`run_flood`] builds a [`reach::ReachGraph`] straight from a [`Scene`] and
//! runs [`reach::check`] over it **untouched** — the same core `reach.rs`'s
//! own module doc records as verifier-grade for exactly this reuse. It
//! deliberately does not call [`reach::graph_from_compiled`]: that builder
//! reads `Ir`/`Compiled`, structures this checker exists to cross-examine
//! (`check/mod.rs`'s module doc), and it also encodes a compile-time fact a
//! `TEXTMAP` alone cannot recover — which side of a two-sided line the
//! compiler *intended* as a walkover exit's "host room" versus its carved
//! "recess". This module knows only what the emitted linedef says: a special
//! number and two bordering sectors. See "Exit goals" below for what that
//! forces.
//!
//! The two passability rules ([`reach::check`]'s step-up and crossing-window
//! math) are entirely `reach.rs`'s concern and are not restated here; this
//! module's own job is narrower — turning [`Scene`] boundaries into
//! [`reach::Edge`]s and [`Scene`] things into [`reach::Node`] keys/goals/
//! start, then turning [`reach::Findings`] back into [`Finding`]s a report
//! can print.
//!
//! # Exit goals
//!
//! A switch exit (`P_UseSpecialLine`, pinned `p_switch.c`) fires only from a
//! line's front side ("Only the front sides of lines are usable" —
//! `KNOWN-GAPS.md`'s "two engine facts" note), so its goal is the sector
//! whose boundary entry has [`Boundary::fronts_this`](crate::check::scene::Boundary::fronts_this)
//! true for that linedef.
//!
//! A walkover exit (`P_CrossSpecialLine`, pinned `p_spec.c`) has **no side
//! gate** — unlike the teleport specials, also walkover-triggered yet
//! deliberately checking `side == 1` in `EV_Teleport` to stay front-only
//! (`data/vocabulary.toml`'s `[specials.teleport]` `source` field records
//! this exact contrast: "`EV_Teleport`... gates activation to the line's
//! front side despite being walkover-triggered"), a gate "Edges" below now
//! models: the teleport specials *are* in the flood's graph, as directed
//! front-side edges. A walkover exit carries no such override, so crossing
//! it from *either* side fires
//! `G_ExitLevel`/`G_SecretExitLevel`. Both sectors bordering the line are
//! therefore goals here — both mirrors [`Scene`] files under their own
//! sector, so this falls out of "any boundary carrying the special, in any
//! sector's own boundary list, names that sector a goal" without needing to
//! know which mirror is "front".
//!
//! **But only when the line is actually crossable.** `P_CrossSpecialLine`
//! only ever runs from `P_TryMove`'s `spechit` bookkeeping, reached solely
//! after a move that same function accepted — and `PIT_CheckLine` rejects a
//! blocking two-sided line for any non-missile before that bookkeeping is
//! ever reached (pinned `p_map.c:214-217`, the same fact "Edges" below
//! cites for walls in general; `KNOWN-GAPS.md`'s note on a walkover exit's
//! carved alcove already records this consequence: a solid-walled crossing
//! never fires). A walkover boundary that fails
//! [`Boundary::passable`](crate::check::scene::Boundary::passable) is
//! therefore not a goal from either side — the exit line itself would never
//! fire in the real engine, not merely "not yet reached". A switch exit
//! needs no such gate: `P_UseSpecialLine` fires from a raycast the player
//! aims, not a crossing, so its solid one-sided wall is not an obstacle to
//! triggering it.
//!
//! This is a strictly more conservative (never falsely-unfinishable) goal
//! set than [`reach::graph_from_compiled`]'s "only the recess" convention:
//! every recess this compiler ever emits is still a goal here, plus the host
//! room, which is sound for a checker that cannot assume a `TEXTMAP` it did
//! not compile keeps the same front/back convention.
//!
//! # Edges
//!
//! One [`reach::Edge`] per `fronts_this` boundary with a resolved neighbor.
//! `PIT_CheckLine` (pinned `p_map.c:214-217`) rejects `ML_BLOCKING` for any
//! non-missile *before* `P_LineOpening` or any door state is even
//! consulted, so a boundary that fails
//! [`Boundary::passable`](crate::check::scene::Boundary::passable) — not
//! two-sided, or two-sided but flagged blocking — contributes no edge at
//! all, full stop: a blocking two-sided line is a wall to the flood
//! regardless of what special it carries, door included. Only once that
//! filter passes does the special matter: [`Tables::door_special`] or a
//! locked special from [`Tables::locked_door_kinds`] becomes
//! [`reach::EdgeKind::Door`]; anything else becomes
//! [`reach::EdgeKind::Open`] — a teleport line's own two sides included,
//! since walking across the pad's rim is an ordinary crossing whatever the
//! line also triggers.
//!
//! **Teleport edges**, on top of that and independent of it. A
//! `fronts_this` boundary carrying either *player* teleport special
//! ([`Tables::player_teleport_specials`]; the two monsters-only forms move
//! no player, so they add nothing to a player flood) also contributes one
//! directed [`reach::EdgeKind::Teleport`] edge from that boundary's own
//! sector to the sector its tag resolves to. Three engine facts shape it,
//! all from `EV_Teleport` (pinned `p_telept.c`):
//!
//! - **Front side only** — "`if (side == 1) return 0;`" — so only the
//!   `fronts_this` mirror builds an edge, and the back mirror of the same
//!   linedef builds none.
//! - **Engine-style resolution** — the destination is the *first* sector,
//!   in declaration order, that both carries the tag and holds a
//!   `teleport_dest` marker, which is what `resolve_teleport_destination`
//!   re-derives. A tag matching sectors that hold no marker resolves past
//!   them; a tag matching none at all yields no edge, because the line
//!   fires nothing (that is V-P15's finding, not the flood's).
//! - **Directed** — a teleport relocates the player rather than opening a
//!   way back, so [`reach::check`] expands the edge `a → b` alone.
//!
//! The edge is built ahead of the `neighbor` filter above — a teleport
//! needs no back sector of its own, since its destination is wherever the
//! tag points rather than what the line happens to border — but it is
//! still gated on
//! [`Boundary::passable`](crate::check::scene::Boundary::passable), exactly
//! as the walkover exits under "Exit goals" are: `PIT_CheckLine` rejects a
//! one-sided or `ML_BLOCKING` line before `P_TryMove`'s `spechit`
//! bookkeeping ever runs `P_CrossSpecialLine`, so a teleport line the
//! player cannot cross fires no more than an exit line on the same
//! boundary would. [`teleport_only_sectors`] builds the same graph twice,
//! with and without these edges, to name the sectors a teleport is
//! load-bearing for.
//!
//! **Lift edges**, likewise on top of the boundary edges and independent of
//! the `teleports` flag. Every platform [`plats::resolve_plats`] resolves —
//! each sector some lift line names by a nonzero tag — contributes one
//! [`reach::EdgeKind::Lift`] edge per distinct caller, written caller →
//! platform (the edge is undirected all the same: [`reach::check`] walks a
//! `Lift` both ways). `reach.rs` owns what that edge *means* — bidirectional
//! and exempt from both geometric rules, because `EV_DoPlat`'s
//! `downWaitUpStay` (`p_plats.c`) brings the floor down to the caller and
//! carries them up; this module's job is only naming the callers, which it
//! does in two cases:
//!
//! - **A trigger the caller can reach on foot.** When some trigger fires
//!   from a sector that is both a neighbor of the platform and more than a
//!   step below it ([`plats::ScenePlat::low_activator_neighbors`]), those
//!   neighbors are the callers, and nothing else is: a player standing
//!   there presses or crosses the line and rides.
//! - **A remote trigger only.** When every `Low` activator is some sector
//!   the platform does not border, the callers are instead every neighbor
//!   more than a step below the platform's rest floor — the ones that
//!   cannot climb onto it as it stands. This is deliberately optimistic:
//!   the flood cannot model the walk from a remote switch back to the
//!   platform, nor the wait before it rises again, so it credits the ride
//!   rather than inventing a softlock. A platform no trigger fires from
//!   below ([`plats::ScenePlat::callable_low`] false) gets no edge at all —
//!   that is the top-only lift a player below cannot call, and the flood
//!   should see the wall it really is.
//!
//! Those activator sets already exclude a walkover either side of which is a
//! dead-end pocket no deeper than the player's radius
//! ([`crate::check::plats`]'s `dead_end_pocket`): the engine never lets the
//! player's center in, so the flood must not credit a lift called across one.
//!
//! One-shot triggers (S1/W1: 21, 10, 122, 121 — [`Tables::lift_specials`]
//! minus [`Tables::lift_repeatable_specials`]) earn the same edge as the
//! repeatable ones. The flood computes what a walk can *ever* reach, and a
//! platform called once carries its caller up once, which is all a
//! reachability set needs; the drop back down is the ordinary boundary
//! edge and needs no trigger. What a one-shot does lose is the second call,
//! which no node's reachability depends on. The one hazard this does not
//! model is a W1 line spent before the player arrives: `P_CrossSpecialLine`
//! (pinned `p_spec.c:503-535`) lets a non-player thing fire specials 10 and
//! 88 too, so a monster wandering across a W1 plat line consumes it — a
//! timing question the flood, which has no monsters, cannot pose. Recorded
//! in `KNOWN-GAPS.md` beside the remote-switch optimism above.
//!
//! Either way the caller must share a
//! [`Boundary::passable`](crate::check::scene::Boundary::passable) boundary
//! with the platform: boarding is a crossing, and `PIT_CheckLine` refuses a
//! blocking one exactly as it does for the boundary edges above. (A blocking
//! two-sided line can still *fire* a use trigger — `P_UseSpecialLine` is a
//! raycast, not a crossing — so a switch pressed through a window lowers a
//! platform the presser cannot then board.) The platform's own neighbor set
//! stays wider on purpose: `P_FindLowestFloorSurrounding` counts every
//! two-sided neighbor when it picks the floor to travel to, blocking or not.
//!
//! Pairs are deduped, as the teleport edges are: a pedestal's four island
//! edges all name the same (host, platform) pair, and a barrier is called
//! from both of the rooms it stands between.
//!
//! # Floor actions
//!
//! [`floors::resolve_floors`] resolves every sector a recognized floor line
//! names by tag, and each such target the flood can model becomes one bit of
//! the [`KeyMask`] above [`ACTION_BIT_BASE`]: the target sector's
//! [`Node::action`] carries `(bit, destination)`, so [`Node::effective_floor`]
//! stands that sector at its destination in every state whose bit is set, and
//! each trigger driving it ORs the bit in where the player fires it. Bits are
//! handed out in target order (ascending by sector) to the targets the flood
//! models, so one it declines below costs no bit.
//!
//! - A **use** form (S1/SR — [`crate::tables::FloorForm::front_only`], gated
//!   to the front by `P_UseSpecialLine`'s "use the back sides of VERY
//!   SPECIAL lines" block, pinned `p_switch.c:284-297`, which returns false
//!   from the back for every special but 124) fires from the line's front
//!   sector, so its bit goes on that sector's [`Node::fires`] and is unioned
//!   in on entering the room.
//! - A **gun** form (G1 — [`crate::tables::FloorForm::shot`]) fires from
//!   *either* sector the line faces, and its bit goes on [`Node::fires`] of
//!   both: `P_ShootSpecialLine` (pinned `p_spec.c:955-1000`) takes no `side`
//!   argument and its only caller passes none (`p_map.c:919-920`), so a shot
//!   from the back side fires it exactly as one from the front does. Both
//!   this form and the use form read their sectors off `check::plats`'s
//!   `activator_sides` — the derivation `floors` and `plats` already share —
//!   rather than restating the rule here.
//! - A **crossing** form (W1/WR) fires from the line itself, so its bit goes
//!   on the [`Edge`] this module builds for that linedef and is unioned in on
//!   arrival from either side — `P_CrossSpecialLine` has no side gate (the
//!   same fact "Exit goals" above cites for a walkover exit), so a bridge
//!   whose walkover is written on both of its thresholds fires from either
//!   one. Which crossings actually happen is [`reach::check`]'s per-state
//!   decision rather than a rest-height one, so a walkover on the far lip of
//!   a pit fires only from the side a walk can reach. A crossing form on a
//!   line no edge is built from — one-sided, or blocking — fires nothing,
//!   for the reason "Edges" gives: `PIT_CheckLine` rejects the move before
//!   `P_TryMove`'s `spechit` bookkeeping ever runs the special.
//!
//! Every recognized form is modeled this way, one-shot and repeatable alike.
//! That is exact for the one-shot forms (the four this compiler emits are
//! S1/W1) and partial for a repeatable one: a mask bit only ever
//! accumulates, so an SR floor that can be sent back down is modeled as
//! moved once and never returned — the flood may then miss a way back that
//! a second press would open.
//!
//! Five kinds of target get **no** bit, stand at their rest floor, and earn
//! a `V-P7` [`Severity::Warning`] naming the sector and why:
//!
//! 1. one driven by lines of more than one engine type (they give it no one
//!    destination);
//! 2. one whose destination is a texture height this checker does not
//!    resolve ([`floors::Destination::NeedsTexture`]);
//! 3. one whose sector already carries an action (a node holds one);
//! 4. **a lowering target holding a shootable thing that does not fit it**
//!    — the engine restores a blocked floor and leaves the thinker running,
//!    so the floor retries every tic and never arrives. Ruling R28;
//!    `blocking_thing` carries the pinned line ranges
//!    (`p_floor.c:83-91`, `p_map.c:1290-1296`, `p_map.c:1337`,
//!    `p_floor.c:209-222`);
//! 5. and — once the first four have taken their bits — every target past
//!    the eighth, `KeyMask::BITS - ACTION_BIT_BASE` being what fits above
//!    the key classes.
//!
//! Leaving such a target at rest is the conservative reading — the flood
//! then judges the map as if the action never fired — and the warning is
//! what keeps that silence from passing for a verdict.
//!
//! The fourth is narrower than the engine in two ways, both deliberate and
//! both restated on `blocking_thing`. Shootability is read as
//! [`Tables::spawnhealth`] resolving, so a **barrel** — `MF_SHOOTABLE`, but
//! a prop rather than a species — does not decline a target, and the flood
//! stays optimistic there. And the fit is tested against the gap **at
//! rest**, while `P_ChangeSector` runs after the floor has already moved one
//! `speed`, so a thing needing exactly one unit more than the cell rests
//! with (four, under `turboLower`) is declined though the engine would
//! squeeze it through. Declining is the safe direction here: it costs a
//! warning, where the opposite error models an opening that does not
//! exist.
//!
//! # Key classes
//!
//! Interned the same way [`reach::graph_from_compiled`] does: by the locked
//! special a key opens ([`Tables::locked_door_kinds`]), not by key-thing
//! name, so a card and skull of one color share a class (`EV_VerticalDoor`,
//! pinned `p_doors.c:371-403`, accepts either). [`run_flood`] reports a hard
//! finding rather than panicking when the vocabulary ever lists more classes
//! than the key half of a [`reach::KeyMask`] can hold — the bits below
//! [`reach::ACTION_BIT_BASE`] — this module runs on arbitrary input,
//! unlike `graph_from_compiled`'s `assert!` over a vocabulary this crate
//! itself controls.
//!
//! # The vacuous-pass hole this module closes
//!
//! `reach::graph_from_compiled` returns `None` — a vacuous pass — for a map
//! with no player 1 start or no exit, on the reasoning that "no exit" is a
//! spec-conformance concern belonging elsewhere. This module has no such
//! elsewhere: a `TEXTMAP` with neither is a hard `V-P7` finding here (see
//! the design doc's verifier catalog), not a silent pass.

use crate::check::plats;
use crate::check::scene::Scene;
use crate::check::{Finding, Severity, Subject, floors};
use crate::reach::{
    self, ACTION_BIT_BASE, Edge, EdgeKind, KeyClass, KeyMask, Limits, Node, ReachGraph,
};
use crate::tables::Tables;
use std::collections::{BTreeMap, BTreeSet};

/// Interns the vocabulary's locked-door specials into key classes: sorted,
/// deduped `special` values alongside the key-kind names each class covers
/// (mirrors [`reach::graph_from_compiled`]'s own interning, computed
/// independently per this module's doc). `class_names[c]` is every key kind
/// sharing class `c`'s special, e.g. `["blue_card", "blue_skull"]`.
///
/// Returns `None` — pushing no finding itself, since callers react
/// differently to it — when the vocabulary lists more classes than the key
/// half of a [`KeyMask`] can represent: the bits below [`ACTION_BIT_BASE`].
/// The bits from there up are floor actions, which this builder does not yet
/// set.
fn intern_lock_classes(tables: &Tables) -> Option<(Vec<u16>, Vec<Vec<String>>)> {
    let kinds = tables.locked_door_kinds();
    let mut specials: Vec<u16> = kinds.iter().map(|&(_, s)| s).collect();
    specials.sort_unstable();
    specials.dedup();
    // COVERAGE: unreachable through the public `Tables` API. `Tables::load`
    // only ever parses the two `include_str!`-embedded tables compiled into
    // this crate, and the pinned `data/vocabulary.toml` lists exactly three
    // distinct locked-door specials (26/27/28 — blue/yellow/red, card and
    // skull of a color sharing one special), far under `ACTION_BIT_BASE`
    // (8). Exercising this branch would need a `Tables` built from a
    // vocabulary this crate does not ship, which no constructor offers.
    if specials.len() > ACTION_BIT_BASE as usize {
        return None;
    }
    let class_names: Vec<Vec<String>> = specials
        .iter()
        .map(|&s| {
            kinds
                .iter()
                .filter(|&&(_, ks)| ks == s)
                .map(|(k, _)| k.clone())
                .collect()
        })
        .collect();
    Some((specials, class_names))
}

/// The [`KeyClass`] `special` interns to under `specials` (as built by
/// [`intern_lock_classes`]), if any.
fn class_of(specials: &[u16], special: u16) -> Option<KeyClass> {
    specials
        .iter()
        .position(|&s| s == special)
        .and_then(|i| KeyClass::try_from(i).ok())
}

/// Renders a [`KeyMask`] as the key-kind names it holds, comma-joined (a
/// color class with more than one kind joins those with `/`, matching
/// `rules.rs`'s own `check_reachability` wording), or `"no keys"` for an
/// empty mask.
///
/// Only the key half of the mask is read: `class_names` has one entry per
/// interned lock class and [`intern_lock_classes`] caps that at
/// [`ACTION_BIT_BASE`], so the enumeration never reaches a floor-action bit.
fn keys_in_words(mask: KeyMask, class_names: &[Vec<String>]) -> String {
    let names: Vec<String> = class_names
        .iter()
        .enumerate()
        .filter(|&(c, _)| mask & (1 << c) != 0)
        .map(|(_, kinds)| kinds.join("/"))
        .collect();
    if names.is_empty() {
        "no keys".to_owned()
    } else {
        names.join(", ")
    }
}

/// Resolves the flood's `start` node: which `player1_start` thing to use
/// (reporting every extra one as its own `V-P7` error — the flood traces
/// only the first, in declaration order, but every extra is still a defect
/// worth naming) and which sector it resolved to. `None`, with the finding
/// already pushed, covers both "no start at all" and "the first start
/// resolved to no sector" (already a `"V-S"` finding from [`Scene::build`])
/// — both are the same "the flood cannot run" story, worded slightly
/// differently for which is true.
fn resolve_start(scene: &Scene, findings: &mut Vec<Finding>) -> Option<usize> {
    let starts: Vec<usize> = scene
        .things
        .iter()
        .enumerate()
        .filter(|(_, t)| t.name.as_deref() == Some("player1_start"))
        .map(|(i, _)| i)
        .collect();
    let Some(&first) = starts.first() else {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Map,
            message: "no player 1 start — the flood cannot run".to_owned(),
        });
        return None;
    };
    for &extra in &starts[1..] {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Thing(extra),
            message: "extra player 1 start; the flood traces only the first".to_owned(),
        });
    }
    let Some(start) = scene.things[first].sector else {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Map,
            message: "the player 1 start could not be located in any sector — the flood \
                      cannot run"
                .to_owned(),
        });
        return None;
    };
    Some(start)
}

/// Resolves the flood's `goals`: sectors bordering a boundary that carries
/// one of the four exit specials, per "Exit goals" above (switch specials
/// only from `fronts_this`; walkover specials from either mirror, but only
/// when the boundary is actually crossable — `PIT_CheckLine` rejects a
/// blocking line before `P_CrossSpecialLine`'s `spechit` bookkeeping is ever
/// reached, so an uncrossable walkover line never fires). Sorted and
/// deduped. `None`, with the finding already pushed, when the map carries no
/// exit at all.
fn resolve_goals(
    scene: &Scene,
    tables: &Tables,
    findings: &mut Vec<Finding>,
) -> Option<Vec<usize>> {
    let switch_specials = [
        tables.exit_switch_special(),
        tables.secret_exit_switch_special(),
    ];
    let walkover_specials = [
        tables.exit_walkover_special(),
        tables.secret_exit_walkover_special(),
    ];
    let mut goals = Vec::new();
    for (i, sector) in scene.sectors.iter().enumerate() {
        for b in &sector.boundary {
            let Ok(special) = u16::try_from(b.special) else {
                continue;
            };
            if (walkover_specials.contains(&special) && b.passable())
                || (switch_specials.contains(&special) && b.fronts_this)
            {
                goals.push(i);
            }
        }
    }
    if goals.is_empty() {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Map,
            message: "no exit line — the flood cannot run".to_owned(),
        });
        return None;
    }
    goals.sort_unstable();
    goals.dedup();
    Some(goals)
}

/// The floor actions the flood models, one resolution feeding both builders
/// — see "Floor actions" in the module doc for the rules behind it.
struct FloorBits {
    /// Per scene sector: the action that sector *is* the target of, as
    /// [`Node::action`]'s `(bit, destination floor)`.
    actions: Vec<Option<(u8, i32)>>,
    /// Per scene sector: the bits entering it fires — the activator
    /// sectors of a use or gun trigger, [`Node::fires`].
    node_fires: Vec<KeyMask>,
    /// Per linedef index: the bits crossing it fires — a walkover trigger's
    /// own line, which [`build_edges`] ORs into that linedef's [`Edge`].
    line_fires: BTreeMap<usize, KeyMask>,
}

/// The number of floor actions a [`KeyMask`] can hold above the key classes.
///
/// The mirror of [`intern_lock_classes`]'s cap on the key half: a bit at or
/// past this shifts off the end of the mask, which [`Node::effective_floor`]
/// documents (and `debug_assert!`s) as an alias onto key class 0. The `as`
/// widening is how [`intern_lock_classes`] already spells its own.
const MAX_MODELED_FLOORS: usize = (KeyMask::BITS - ACTION_BIT_BASE) as usize;

/// A `V-P7` Warning naming a floor target the flood leaves at rest, and why.
fn unmodeled_floor(sector: usize, rest: i32, why: &str) -> Finding {
    Finding {
        check: "V-P7",
        severity: Severity::Warning,
        subject: Subject::Sector(sector),
        message: format!("floor target {why}; the flood leaves it at its rest floor {rest}"),
    }
}

/// The first thing in `sector` whose species the engine would block a
/// lowering floor on: a `MF_SHOOTABLE` mobj that does not fit the sector as
/// it rests, as `(its scene index, its name, its required height)`.
///
/// Verified at the pinned commit
/// `a77dfb96cb91780ca334d0d4cfd86957558007e0`. `T_MovePlane`'s floor-down
/// branch (`p_floor.c:83-91`) lowers by `speed`, calls `P_ChangeSector`, and
/// on a true return puts the floor back and returns `crushed`;
/// `P_ChangeSector` (`p_map.c:1321`) returns `nofit` (`p_map.c:1337`), which
/// `PIT_ChangeSector` (`p_map.c:1257`) sets at `p_map.c:1296` for any thing
/// `P_ThingHeightClip` (`p_map.c:530`) rejects on
/// `ceilingz - floorz < height` that is not a corpse, is not `MF_DROPPED`,
/// and *is* `MF_SHOOTABLE` (`p_map.c:1290`). `nofit` is set before the
/// `crushchange` guard, so a non-crushing floor is blocked identically. And
/// `T_MoveFloor` (`p_floor.c:209`) drops the thinker only on `pastdest`, so
/// a blocked floor keeps retrying every tic and never arrives.
///
/// **Two approximations, both stated rather than hidden.**
///
/// *The rest gap, not the first step's.* `P_ChangeSector` runs after the
/// floor has already moved one `speed` (`FLOORSPEED` is `FRACUNIT`,
/// `p_spec.h:600`, and `lowerFloorToLowest` takes it unscaled at
/// `p_floor.c:302`), so the gap the engine tests is one unit — four, for a
/// `turboLower` — wider than the gap at rest. This tests the rest gap, so a
/// thing needing exactly one more unit than the cell rests with is declined
/// here though the engine would squeeze it through. Declining is the safe
/// direction for a reachability flood: the cost is a Warning and a target
/// left at rest, where the opposite error models an opening that does not
/// exist.
///
/// *Species only.* Shootability is read as [`Tables::spawnhealth`]
/// resolving — the same monster test `check::conform` uses. A barrel is
/// `MF_SOLID|MF_SHOOTABLE` too (`data/engine.toml`'s `[props.barrel]`
/// citation) and is not caught, so a target blocked only by a barrel is
/// still modeled as opening.
fn blocking_thing<'a>(
    scene: &'a Scene,
    tables: &Tables,
    sector: usize,
) -> Option<(usize, &'a str, i32)> {
    let s = &scene.sectors[sector];
    let rest_gap = s.ceiling - s.floor;
    scene.things.iter().enumerate().find_map(|(i, t)| {
        if t.sector != Some(sector) {
            return None;
        }
        let name = t.name.as_deref()?;
        // Shootable, per the monster test; then the engine's own fit test.
        tables.spawnhealth(name)?;
        let height = tables.species(name)?.height;
        (rest_gap < height).then_some((i, name, height))
    })
}

/// Resolves the map's floor actions into the bits [`build_nodes`] and
/// [`build_edges`] set, pushing a `V-P7` Warning for every target this
/// checker declines to model (see "Floor actions" in the module doc).
///
/// Reads [`floors::resolve_floors`] — the one resolution the invariants
/// (V-P28), the conformance rows and the `lift` recognizer share — rather
/// than anything in `compile/`, per this module's own doc.
fn resolve_floor_bits(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) -> FloorBits {
    let mut bits = FloorBits {
        actions: vec![None; scene.sectors.len()],
        node_fires: vec![0; scene.sectors.len()],
        line_fires: BTreeMap::new(),
    };
    // Bits are handed out only to the targets that get one, so a target
    // declined below leaves the mask as wide as it found it.
    let mut next_bit = 0usize;
    for f in floors::resolve_floors(scene, tables) {
        let Some(action) = f.single() else {
            findings.push(unmodeled_floor(
                f.sector,
                f.rest,
                &format!(
                    "is driven by lines of {} engine types, which give it no one destination",
                    f.actions.len()
                ),
            ));
            continue;
        };
        let floors::Destination::Height(dest) = action.destination else {
            findings.push(unmodeled_floor(
                f.sector,
                f.rest,
                "raises to a texture height this checker does not resolve",
            ));
            continue;
        };
        // COVERAGE: unreachable. `resolve_floors` builds one `SceneFloor` per
        // sector — its targets come from `sectors_named_by`'s `BTreeSet` —
        // so no two entries can name one sector. Handled rather than
        // asserted because this checker floods arbitrary WADs: a node holds
        // exactly one action, so a second would silently replace the first,
        // and the flood would judge a floor nobody can move.
        if bits.actions[f.sector].is_some() {
            findings.push(unmodeled_floor(
                f.sector,
                f.rest,
                "is the second action resolved onto this sector, and a node carries one",
            ));
            continue;
        }
        // A lowering floor with something in it that does not fit never
        // lowers: the engine restores the floor and leaves the thinker
        // running (see `blocking_thing` for the pinned lines). The target
        // stays at rest and the flood must not model the opening — V-P28
        // says nothing about this, and V-P2 Errors on the thing itself, so
        // this Warning is what keeps the *reachability* answer honest.
        if dest < f.rest
            && let Some((thing, name, need)) = blocking_thing(scene, tables, f.sector)
        {
            let gap = scene.sectors[f.sector].ceiling - f.rest;
            findings.push(unmodeled_floor(
                f.sector,
                f.rest,
                &format!(
                    "is blocked by `{name}` (thing {thing}), which does not fit in it — {gap} \
                     units of headroom against the {need} it needs — and a floor a shootable \
                     thing does not fit in never lowers"
                ),
            ));
            continue;
        }
        // Last of the five, so a target declined for a reason of its own is
        // reported for that reason rather than for the mask being full.
        if next_bit >= MAX_MODELED_FLOORS {
            findings.push(unmodeled_floor(
                f.sector,
                f.rest,
                &format!("is past the first {MAX_MODELED_FLOORS}, all the reachability mask holds"),
            ));
            continue;
        }
        let bit = u8::try_from(next_bit).expect("the cap above keeps it under MAX_MODELED_FLOORS");
        next_bit += 1;
        bits.actions[f.sector] = Some((bit, dest));
        let mask: KeyMask = 1 << (ACTION_BIT_BASE + u32::from(bit));
        for &t in &action.triggers {
            let trigger = &f.triggers[t];
            if trigger.form.front_only() || trigger.form.shot() {
                // A use or a gun line fires from where the player stands
                // rather than from a crossing, so the bit goes on the nodes
                // `activator_sides` names: the front sector for a switch,
                // either sector a gun line faces.
                for &(sector, _) in &trigger.activators {
                    bits.node_fires[sector] |= mask;
                }
            } else {
                *bits.line_fires.entry(trigger.linedef).or_default() |= mask;
            }
        }
    }
    bits
}

/// Builds one [`Node`] per scene sector: floor/ceiling verbatim, `keys` set
/// from every thing whose name is a key kind ([`Tables::locked_door_kinds`])
/// with a resolved sector, unioned bit by interned [`KeyClass`], and the
/// floor action `bits` resolved for that sector — the action it is the
/// target of, and the actions entering it fires.
fn build_nodes(
    scene: &Scene,
    specials: &[u16],
    kinds: &[(String, u16)],
    bits: &FloorBits,
) -> Vec<Node> {
    let mut nodes: Vec<Node> = scene
        .sectors
        .iter()
        .enumerate()
        .map(|(i, s)| Node {
            floor: s.floor,
            ceiling: s.ceiling,
            keys: 0,
            fires: bits.node_fires[i],
            action: bits.actions[i],
        })
        .collect();
    for thing in &scene.things {
        let Some(name) = thing.name.as_deref() else {
            continue;
        };
        let Some(&(_, special)) = kinds.iter().find(|(k, _)| k == name) else {
            continue;
        };
        let (Some(class), Some(sector)) = (class_of(specials, special), thing.sector) else {
            continue;
        };
        nodes[sector].keys |= 1 << class;
    }
    nodes
}

/// Resolves a teleport line's `tag` the way `EV_Teleport` does (pinned
/// `p_telept.c`): walk the sectors in declaration order and take the first
/// whose tag matches *and* which holds a `teleport_dest` thing ("`if
/// (m->type != MT_TELEPORTMAN) continue;`" ... "`if (sector-sectors != i)
/// continue;`"). `None` when no such sector exists — the line can never
/// fire, which is V-P15's finding, not an edge.
pub(crate) fn resolve_teleport_destination(
    scene: &Scene,
    tables: &Tables,
    tag: i32,
) -> Option<usize> {
    if tag == 0 {
        return None;
    }
    let marker = i32::from(
        tables
            .thing_id("teleport_dest")
            .expect("`teleport_dest` is in the vocabulary"),
    );
    scene.sectors.iter().enumerate().position(|(i, s)| {
        s.tag == tag
            && scene
                .things
                .iter()
                .any(|t| t.type_id == marker && t.sector == Some(i))
    })
}

/// Builds one [`Edge`] per `fronts_this` boundary with a resolved neighbor,
/// per "Edges" above: a boundary that fails
/// [`Boundary::passable`](crate::check::scene::Boundary::passable) —
/// one-sided (already excluded by `neighbor` being `None`) or flagged
/// blocking — contributes no edge at all, *before* its special is even
/// read; `PIT_CheckLine` (pinned `p_map.c:214-217`) rejects a blocking line
/// for any non-missile ahead of `P_LineOpening`, door state notwithstanding,
/// so an open door on a blocking line still cannot be crossed. Once that
/// filter passes: [`Tables::door_special`] becomes
/// [`EdgeKind::Door`]`{ lock: None }`; a special that interns to a lock
/// class ([`Tables::locked_door_kinds`]) becomes
/// [`EdgeKind::Door`]`{ lock: Some(class) }`; anything else becomes
/// [`EdgeKind::Open`].
///
/// Plus, when `teleports` is set, one directed [`EdgeKind::Teleport`] edge
/// per distinct (front, destination) pair among the `fronts_this`
/// **passable** boundaries carrying a player teleport special — several
/// boundaries of the same pad can share a trigger special and tag, and this
/// dedupes them exactly as `reach::teleport_edges` already does, rather than
/// pushing one identical edge per triggering boundary. Each edge runs from
/// its shared front sector to whatever [`resolve_teleport_destination`]
/// resolves the tag to — built ahead of the `neighbor` filter above, since a
/// teleport's destination is its tag's, not its own back sector's, but
/// behind the same crossability gate every other edge answers to. Passing
/// `false` builds the same graph with those edges left out, which is how
/// [`teleport_only_sectors`] measures what a teleport is load-bearing for.
///
/// And, unconditionally, one [`EdgeKind::Lift`] edge per distinct (caller,
/// platform) pair [`plats::resolve_plats`] resolves — see "Lift edges" in
/// the module doc for which sectors count as callers. Unconditional because
/// a platform is geometry, not a teleport's optional modeling: the
/// `teleports` flag exists to measure what a *teleport* is load-bearing for,
/// and taking the lifts out alongside it would confound that measurement.
///
/// A boundary edge also carries whatever floor actions crossing its linedef
/// fires ([`FloorBits::line_fires`], "Floor actions" in the module doc). The
/// teleport and lift edges carry none: a teleport line's special is its own,
/// never a floor special, and a lift edge is not a linedef's crossing at all
/// but a ride the platform gives.
fn build_edges(
    scene: &Scene,
    tables: &Tables,
    specials: &[u16],
    teleports: bool,
    bits: &FloorBits,
) -> Vec<Edge> {
    let plain_door = tables.door_special();
    let player_teleports = tables.player_teleport_specials();
    let mut edges = Vec::new();
    // Dedupes teleport edges by (front sector, destination sector), mirroring
    // `reach::teleport_edges`'s own `HashSet<(NodeIdx, NodeIdx)>`: several
    // boundaries of the same pad (e.g. all four sides of an island) share one
    // trigger special and one tag, so without this, one identical edge would
    // be pushed per triggering boundary rather than once per distinct pair.
    let mut teleport_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    // The same dedupe for lift edges: every edge of an island pedestal
    // carries the platform's tag, and a barrier is called from both of its
    // neighbors, so one (caller, platform) pair can be named several times.
    let mut lift_pairs: BTreeSet<(usize, usize)> = BTreeSet::new();
    for (i, sector) in scene.sectors.iter().enumerate() {
        for b in &sector.boundary {
            if !b.fronts_this {
                continue;
            }
            // Ahead of the `neighbor` filter below — a teleport's
            // destination is wherever the tag resolves, not what the line
            // borders — but still behind `passable()`: an uncrossable line
            // never reaches `P_CrossSpecialLine` at all.
            if teleports
                && b.passable()
                && u16::try_from(b.special).is_ok_and(|s| player_teleports.contains(&s))
                && let Some(dest) = resolve_teleport_destination(scene, tables, b.tag)
                && teleport_pairs.insert((i, dest))
            {
                edges.push(Edge {
                    a: i,
                    b: dest,
                    kind: EdgeKind::Teleport,
                    fires: 0,
                });
            }
            // The line itself is still an ordinary boundary below.
            let Some(neighbor) = b.neighbor else {
                continue;
            };
            // A blocking two-sided line is a wall to the flood regardless
            // of its special, door included — see the doc comment above.
            if !b.passable() {
                continue;
            }
            let special = u16::try_from(b.special).ok();
            let kind = if special == Some(plain_door) {
                EdgeKind::Door { lock: None }
            } else if let Some(class) = special.and_then(|s| class_of(specials, s)) {
                EdgeKind::Door { lock: Some(class) }
            } else {
                EdgeKind::Open
            };
            edges.push(Edge {
                a: i,
                b: neighbor,
                kind,
                fires: bits.line_fires.get(&b.linedef).copied().unwrap_or(0),
            });
        }
    }
    let step = tables.step_height();
    for plat in plats::resolve_plats(scene, tables) {
        // Boarding is a crossing like any other, so a caller needs at least
        // one *passable* boundary with the platform. `ScenePlat::neighbors`
        // is deliberately the engine's wider set — `EV_DoPlat`'s
        // `P_FindLowestFloorSurrounding` counts every two-sided neighbor,
        // `ML_BLOCKING` or not, because a blocking line still bounds the
        // sector it borders — so the flood narrows it here rather than there.
        let boardable: BTreeSet<usize> = scene.sectors[plat.sector]
            .boundary
            .iter()
            .filter(|b| b.passable())
            .filter_map(|b| b.neighbor)
            .collect();
        // Every neighbor more than a step below the platform at rest: the
        // ones that cannot climb onto it as it stands, and so are the ones a
        // ride is worth an edge for. No lower bound against `plat.low` — a
        // neighbor above it still boards, by dropping onto the lowered
        // platform, and drops are free.
        let neighbors_at_low: Vec<usize> = plat
            .neighbors
            .iter()
            .copied()
            .filter(|&n| scene.sectors[n].floor < scene.sectors[plat.sector].floor - step)
            .collect();
        let adjacent_callers = plat.low_activator_neighbors();
        // Remote-only activators: every low neighbor gets the edge, ungated
        // — the flood cannot model the wait or the walk from a switch that
        // is nowhere near the platform, so it stays optimistic rather than
        // pessimistic, the same trade `KNOWN-GAPS.md` already records for
        // the teleport edge that ignores its pad's step height.
        let callers: Vec<usize> = if adjacent_callers.is_empty() {
            if plat.callable_low() {
                neighbors_at_low
            } else {
                Vec::new()
            }
        } else {
            adjacent_callers.into_iter().collect()
        };
        for c in callers.into_iter().filter(|c| boardable.contains(c)) {
            if lift_pairs.insert((c, plat.sector)) {
                edges.push(Edge {
                    a: c,
                    b: plat.sector,
                    kind: EdgeKind::Lift,
                    fires: 0,
                });
            }
        }
    }
    edges
}

/// Maps a completed [`reach::Findings`] onto [`Finding`]s: `unfinishable` is
/// one `Subject::Map` Error; `stranded` entries are reported only when
/// finishable (an unfinishable map's stranded list is the degenerate "every
/// visited state" case — that fact is already the unfinishable finding's
/// story, not a fresh one per node), each naming its sector, the key
/// classes held, in words, via [`keys_in_words`], and the floor targets the
/// state still leaves at rest; every `unreachable` sector is its own
/// `Subject::Sector` Error.
fn push_flood_findings(
    scene: &Scene,
    result: &reach::Findings,
    class_names: &[Vec<String>],
    bits: &FloorBits,
    findings: &mut Vec<Finding>,
) {
    // Which floor targets had *not* fired in the state being reported, in
    // the wording the compiler's own `pending` uses (`crate::rules`): a room
    // the player is stranded in is often one whose way out is a wall still
    // standing or a pit still sunk, so naming the targets at rest says what
    // has to be made reachable first. The compiler names the construct it
    // emitted; a built map has only the sector, which is the same fact in
    // the vocabulary this side has.
    let pending = |mask: KeyMask| -> String {
        let parts: Vec<String> = bits
            .actions
            .iter()
            .enumerate()
            .filter_map(|(sector, action)| {
                let (bit, dest) = (*action)?;
                if mask & (1 << (ACTION_BIT_BASE + u32::from(bit))) != 0 {
                    return None;
                }
                let verb = if dest > scene.sectors[sector].floor {
                    "raised"
                } else {
                    "lowered"
                };
                Some(format!("sector {sector} not {verb}"))
            })
            .collect();
        if parts.is_empty() {
            String::new()
        } else {
            format!("; {}", parts.join(", "))
        }
    };

    if result.unfinishable {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Map,
            message: "no feasible walk from the start reaches any exit".to_owned(),
        });
    } else {
        for &(node, mask) in &result.stranded {
            findings.push(Finding {
                check: "V-P7",
                severity: Severity::Error,
                subject: Subject::Sector(node),
                message: format!(
                    "reachable holding {}, but no walk from there reaches an exit{}",
                    keys_in_words(mask, class_names),
                    pending(mask)
                ),
            });
        }
    }
    for &node in &result.unreachable {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Sector(node),
            message: "never reached by any walk from the player start".to_owned(),
        });
    }
}

/// Runs the V-P7 flood over `scene` and pushes its findings, including one
/// [`Severity::Warning`] per floor target it declines to model (see "Floor
/// actions" in the module doc). Those warnings ride the flood rather than
/// standing alone, so a map that returns `None` below — no start, no exit,
/// too many lock classes — says nothing about its floor targets either: the
/// flood they qualify never ran.
///
/// Returns `Some(reached)` — one entry per scene sector, `reached[i]` true
/// iff sector `i` is forward-reachable from the player 1 start — when the
/// flood ran at all, for [`crate::check::invariants::check_pickup_reachability`]
/// (V-P20) to consume. Returns `None`, the reason already pushed as a
/// [`Finding`] by `resolve_start` or `resolve_goals`, when it could not
/// run: no `player1_start` thing, the first start resolved to no sector, no
/// exit line, or (below) more locked-door classes than the key half of a
/// [`KeyMask`] can represent.
#[must_use]
pub fn run_flood(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) -> Option<Vec<bool>> {
    let start = resolve_start(scene, findings)?;
    let goals = resolve_goals(scene, tables, findings)?;
    // COVERAGE: unreachable — see `intern_lock_classes`'s own comment on its
    // identical `None` branch; the pinned vocabulary never triggers it.
    let Some((specials, class_names)) = intern_lock_classes(tables) else {
        findings.push(Finding {
            check: "V-P7",
            severity: Severity::Error,
            subject: Subject::Map,
            message: format!(
                "the vocabulary lists more than {ACTION_BIT_BASE} distinct lock classes, \
                 which a KeyMask cannot represent — the flood cannot run"
            ),
        });
        return None;
    };

    let kinds = tables.locked_door_kinds();
    let bits = resolve_floor_bits(scene, tables, findings);
    let nodes = build_nodes(scene, &specials, &kinds, &bits);
    let edges = build_edges(scene, tables, &specials, true, &bits);

    let graph = ReachGraph {
        nodes,
        edges,
        start,
        goals,
    };
    let limits = Limits {
        player_height: tables.player().height,
        max_step: tables.step_height(),
    };
    let result = reach::check(&graph, &limits);
    push_flood_findings(scene, &result, &class_names, &bits, findings);

    let mut reached = vec![true; scene.sectors.len()];
    for &node in &result.unreachable {
        reached[node] = false;
    }
    Some(reached)
}

/// Sectors reachable from the player start *only* through a teleport: one
/// entry per sector, `true` when the flood with teleport edges reaches it
/// and the flood without them does not. `None` when the flood cannot run
/// (no start or no exit) — the reasons [`run_flood`] already reports, so
/// this pass stays silent rather than reporting them twice.
///
/// This is `progression.exit.trigger = teleport`'s measurement: a walkover
/// exit whose sector is teleport-only is a teleport exit.
#[must_use]
pub fn teleport_only_sectors(scene: &Scene, tables: &Tables) -> Option<Vec<bool>> {
    let mut sink = Vec::new();
    let start = resolve_start(scene, &mut sink)?;
    let goals = resolve_goals(scene, tables, &mut sink)?;
    // COVERAGE: unreachable — see `intern_lock_classes`'s own comment on
    // its identical `None` branch; the pinned vocabulary never triggers it.
    let (specials, _) = intern_lock_classes(tables)?;
    let kinds = tables.locked_door_kinds();
    // Into the same sink: the floor warnings are `run_flood`'s to report,
    // and `check::run` always calls it, so repeating them here would double
    // every one.
    let bits = resolve_floor_bits(scene, tables, &mut sink);
    let nodes = build_nodes(scene, &specials, &kinds, &bits);
    let limits = Limits {
        player_height: tables.player().height,
        max_step: tables.step_height(),
    };
    let reachable_with = |teleports: bool| {
        let graph = ReachGraph {
            nodes: nodes.clone(),
            edges: build_edges(scene, tables, &specials, teleports, &bits),
            start,
            goals: goals.clone(),
        };
        let result = reach::check(&graph, &limits);
        let mut reached = vec![true; scene.sectors.len()];
        for &node in &result.unreachable {
            reached[node] = false;
        }
        reached
    };
    let (with, without) = (reachable_with(true), reachable_with(false));
    Some(
        with.iter()
            .zip(&without)
            .map(|(&w, &wo)| w && !wo)
            .collect(),
    )
}

/// V-P24 (engine form): every locked-door special present has at least one
/// key thing of its color class placed, and every placed key thing opens
/// at least one door present.
///
/// Re-derived at the class level, not the specific key-kind level
/// `rules.rs`'s IR-side `check_key_lock_coherence` uses, because a class is
/// all an emitted linedef's `special` retains — `26` opens to *either*
/// `blue_card` or `blue_skull` ([`Tables::locked_door_kinds`]), not
/// whichever one the room's author had in mind. The ordering half of P24
/// ("every locked door has its key reachable before it") is [`run_flood`]'s
/// job, not this one's — an unfinishable finding from a key trapped behind
/// its own lock is a `V-P7` finding, not a `V-P24` one (`docs/design.md`
/// §7.3's P24 entry: "which the P7 flood proves rather than assumes").
///
/// Independent of [`run_flood`]: runs (and can find defects) even on a map
/// with no start or exit. Silently reports nothing for a vocabulary with
/// more lock classes than the key half of a [`KeyMask`] can hold —
/// [`run_flood`] is the one that reports that as its own hard finding, and
/// it always runs first in [`crate::check::run`]'s wiring.
///
/// A key thing with `thing.sector == None` — outside every closed sector —
/// counts toward neither half: it cannot satisfy a lock's keyless-lock check
/// (a key nobody can pick up does not make the lock openable) and it gets no
/// orphan-key analysis of its own (its placement is already invalid
/// geometry, reported once as its own `"V-S"` Error by
/// [`Scene::build`](crate::check::scene::Scene::build)). This mirrors
/// `build_nodes`, which already excludes such a thing from a node's key
/// bits for the same reason — only a key the player can actually reach and
/// pick up should ever count as "present".
pub fn check_key_lock_coherence(scene: &Scene, tables: &Tables, findings: &mut Vec<Finding>) {
    let Some((specials, class_names)) = intern_lock_classes(tables) else {
        // Silent here, not a gap: `run_flood` reports this same overflow as
        // its own hard `V-P7` finding, and `check::run` always calls it
        // first, so this pass need not duplicate the report.
        //
        // COVERAGE: unreachable for the same reason `intern_lock_classes`'s
        // own `None` branch is — the pinned vocabulary never triggers it.
        return;
    };
    let kinds = tables.locked_door_kinds();

    let mut key_present = vec![false; specials.len()];
    for thing in &scene.things {
        if thing.sector.is_none() {
            continue;
        }
        let Some(name) = thing.name.as_deref() else {
            continue;
        };
        if let Some(&(_, special)) = kinds.iter().find(|(k, _)| k == name)
            && let Some(class) = class_of(&specials, special)
        {
            key_present[class as usize] = true;
        }
    }

    // A door is identified by its own (back) sector, the same convention
    // `check_door_openings` (V-P4) uses: a `fronts_this` boundary's
    // `neighbor` is the sector `EV_VerticalDoor`/`EV_DoDoor` actually act
    // on. A single physical door can front two rooms on two separate
    // linedefs that share that one back sector (a thin door sector between
    // two rooms) — deduping by `(door sector, class)` rather than by
    // linedef is what keeps such a door from reporting its own keyless lock
    // twice.
    let mut door_locks: BTreeSet<(usize, KeyClass)> = BTreeSet::new();
    for sector in &scene.sectors {
        for b in &sector.boundary {
            if !b.fronts_this {
                continue;
            }
            let Some(neighbor) = b.neighbor else {
                continue;
            };
            let Some(class) = u16::try_from(b.special)
                .ok()
                .and_then(|s| class_of(&specials, s))
            else {
                continue;
            };
            door_locks.insert((neighbor, class));
        }
    }
    for &(door_sector, class) in &door_locks {
        if !key_present[class as usize] {
            findings.push(Finding {
                check: "V-P24",
                severity: Severity::Error,
                subject: Subject::Sector(door_sector),
                message: format!(
                    "door locked to `{}`, but no such key is placed anywhere in the map",
                    class_names[class as usize].join("/")
                ),
            });
        }
    }

    let mut lock_present = vec![false; specials.len()];
    for sector in &scene.sectors {
        for b in &sector.boundary {
            if let Some(class) = u16::try_from(b.special)
                .ok()
                .and_then(|s| class_of(&specials, s))
            {
                lock_present[class as usize] = true;
            }
        }
    }

    for (i, thing) in scene.things.iter().enumerate() {
        if thing.sector.is_none() {
            continue;
        }
        let Some(name) = thing.name.as_deref() else {
            continue;
        };
        let Some(&(_, special)) = kinds.iter().find(|(k, _)| k == name) else {
            continue;
        };
        // COVERAGE: unreachable — `special` came from `kinds` (`tables.
        // locked_door_kinds()`), and `specials` is that same call's deduped
        // second elements (via `intern_lock_classes`, above), so `special`
        // is always a member of `specials` and `class_of` always resolves.
        let Some(class) = class_of(&specials, special) else {
            continue;
        };
        if !lock_present[class as usize] {
            findings.push(Finding {
                check: "V-P24",
                severity: Severity::Error,
                subject: Subject::Thing(i),
                message: format!("{name} is placed but opens no door in this map"),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check::fixtures::{self, TELEPORT_MAP};
    use crate::check::scene::{Boundary, SceneSector, SceneThing};
    use crustywad::map::udmf::parse_udmf;

    /// A [`room_chain_ex`] fixture's text, plus the next unused
    /// vertex/sidedef/sector index — so a caller needing to append more
    /// geometry (an isolated sector, say) can keep its own indices
    /// consistent without re-deriving the numbering scheme by hand.
    struct Chain {
        text: String,
        next_vertex: usize,
        next_sidedef: usize,
        next_sector: usize,
    }

    /// Room-row layout: each room is `SIZE` map units square.
    const SIZE: f64 = 128.0;

    /// Declaration index of room `i`'s top-row vertex, in an `n`-room row.
    /// Room `i`'s *bottom*-row vertex is declared first and is simply `i`
    /// itself — every call site below uses the bare index rather than a
    /// same-named identity function.
    fn top_vertex(n: usize, i: usize) -> usize {
        n + 1 + i
    }

    /// The `2*(n+1)` vertex declarations for an `n`-room row: bottom row
    /// left to right, then top row left to right.
    #[expect(
        clippy::cast_precision_loss,
        reason = "room-row fixtures never exceed a handful of rooms, far under f64's 52-bit \
                  mantissa"
    )]
    fn chain_vertices(n: usize) -> String {
        use std::fmt::Write as _;
        let mut vertices = String::new();
        for i in 0..=n {
            let _ = writeln!(
                vertices,
                "vertex {{ x = {:.3}; y = 0.000; }}",
                i as f64 * SIZE
            );
        }
        for i in 0..=n {
            let _ = writeln!(
                vertices,
                "vertex {{ x = {:.3}; y = {SIZE:.3}; }}",
                i as f64 * SIZE
            );
        }
        vertices
    }

    /// The shared two-sided linedef (plus its two sidedefs) between room `i`
    /// and room `i + 1`, appending to `linedefs`/`sidedefs` and advancing
    /// `next_sidedef` by 2.
    fn write_link(
        n: usize,
        i: usize,
        (special, tag, blocking): (i32, i32, bool),
        linedefs: &mut String,
        sidedefs: &mut String,
        next_sidedef: &mut usize,
    ) {
        use std::fmt::Write as _;
        let extra = if special == 0 {
            String::new()
        } else {
            format!(" special = {special}; arg0 = {tag};")
        };
        let blocking_s = if blocking { " blocking = true;" } else { "" };
        let _ = writeln!(
            linedefs,
            "linedef {{ v1 = {}; v2 = {}; sidefront = {}; sideback = {}; \
             twosided = true;{extra}{blocking_s} }}",
            top_vertex(n, i + 1),
            i + 1,
            next_sidedef,
            *next_sidedef + 1
        );
        for sector in [i, i + 1] {
            let _ = writeln!(
                sidedefs,
                "sidedef {{ sector = {sector}; texturemiddle = \"-\"; texturetop = \"STARTAN2\"; \
                 texturebottom = \"STARTAN2\"; }}"
            );
        }
        *next_sidedef += 2;
    }

    /// Room `i`'s own four (or two, for an interior room) perimeter walls —
    /// bottom and top always, plus a left wall if `i == 0` and a right wall
    /// if `i == n - 1` — appending to `linedefs`/`sidedefs` and advancing
    /// `next_sidedef`. `exit`, if it names room `i`, adds `special`/`arg0`
    /// to the bottom wall.
    fn write_perimeter(
        n: usize,
        i: usize,
        exit: Option<(usize, u16, i32)>,
        linedefs: &mut String,
        sidedefs: &mut String,
        next_sidedef: &mut usize,
    ) {
        use std::fmt::Write as _;
        let mut wall =
            |v1: usize, v2: usize, extra: &str, linedefs: &mut String, sidedefs: &mut String| {
                let _ = writeln!(
                    linedefs,
                    "linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {next_sidedef};{extra} \
                 blocking = true; }}"
                );
                let _ = writeln!(
                    sidedefs,
                    "sidedef {{ sector = {i}; texturemiddle = \"STARTAN2\"; }}"
                );
                *next_sidedef += 1;
            };

        let bottom_extra = match exit {
            Some((room, special, tag)) if room == i => {
                format!(" special = {special}; arg0 = {tag};")
            }
            _ => String::new(),
        };
        wall(i, i + 1, &bottom_extra, linedefs, sidedefs);
        wall(
            top_vertex(n, i + 1),
            top_vertex(n, i),
            "",
            linedefs,
            sidedefs,
        );
        if i == 0 {
            wall(top_vertex(n, 0), 0, "", linedefs, sidedefs);
        }
        if i == n - 1 {
            wall(n, top_vertex(n, n), "", linedefs, sidedefs);
        }
    }

    /// A row of `rooms.len()` 128×128 boxes, room `i` spanning
    /// `x ∈ [i*128, (i+1)*128]`, `y ∈ [0, 128]`, each adjacent pair sharing
    /// a two-sided vertical linedef. `links[i]` is `(special, tag,
    /// blocking)` for the boundary between room `i` and room `i+1`
    /// (`links.len() == rooms.len() - 1`) — `special = 0` for a plain open
    /// boundary. `exit`, if present, is `(room, special, tag)`: adds
    /// `special`/`arg0` to that room's own one-sided *bottom* wall, the
    /// switch-exit shape (`P_UseSpecialLine` fires from a raycast, not a
    /// crossing, so the exit line stays a normal solid one-sided wall —
    /// `KNOWN-GAPS.md`). `things` is spliced in verbatim; callers place
    /// things at `x ∈ [i*128, (i+1)*128]` for room `i`.
    fn room_chain_ex(
        rooms: &[(i32, i32, i32)],
        links: &[(i32, i32, bool)],
        exit: Option<(usize, u16, i32)>,
        things: &str,
    ) -> Chain {
        use std::fmt::Write as _;

        let n = rooms.len();
        assert_eq!(
            links.len(),
            n - 1,
            "one link between each pair of adjacent rooms"
        );

        let vertices = chain_vertices(n);

        let mut linedefs = String::new();
        let mut sidedefs = String::new();
        let mut next_sidedef = 0usize;
        for (i, &link) in links.iter().enumerate() {
            write_link(n, i, link, &mut linedefs, &mut sidedefs, &mut next_sidedef);
        }
        for i in 0..n {
            write_perimeter(n, i, exit, &mut linedefs, &mut sidedefs, &mut next_sidedef);
        }

        let mut sectors = String::new();
        for &(floor, ceiling, light) in rooms {
            let _ = writeln!(
                sectors,
                "sector {{ texturefloor = \"FLOOR4_8\"; textureceiling = \"CEIL3_5\"; \
                 heightfloor = {floor}; heightceiling = {ceiling}; lightlevel = {light}; }}"
            );
        }

        Chain {
            text: format!("namespace = \"doom\";\n{vertices}{linedefs}{sidedefs}{sectors}{things}"),
            next_vertex: 2 * (n + 1),
            next_sidedef,
            next_sector: n,
        }
    }

    fn room_chain(
        rooms: &[(i32, i32, i32)],
        links: &[(i32, i32, bool)],
        exit: Option<(usize, u16, i32)>,
        things: &str,
    ) -> String {
        room_chain_ex(rooms, links, exit, things).text
    }

    /// A closed, one-sided 128×128 box with no linedef connecting it to
    /// anything else, at `vbase`/`sbase`/`sector_idx` — the indices
    /// [`Chain::next_vertex`]/[`Chain::next_sidedef`]/[`Chain::next_sector`]
    /// give, so it appends cleanly after a [`room_chain_ex`] fixture with no
    /// index collisions. Placed far in `x` so it cannot coincide with the
    /// chain's own geometry.
    fn isolated_box(vbase: usize, sbase: usize, sector_idx: usize) -> String {
        format!(
            r#"vertex {{ x = 4000.000; y = 0.000; }}
vertex {{ x = 4128.000; y = 0.000; }}
vertex {{ x = 4128.000; y = 128.000; }}
vertex {{ x = 4000.000; y = 128.000; }}
linedef {{ v1 = {v0}; v2 = {v1}; sidefront = {s0}; blocking = true; }}
linedef {{ v1 = {v1}; v2 = {v2}; sidefront = {s1}; blocking = true; }}
linedef {{ v1 = {v2}; v2 = {v3}; sidefront = {s2}; blocking = true; }}
linedef {{ v1 = {v3}; v2 = {v0}; sidefront = {s3}; blocking = true; }}
sidedef {{ sector = {sector_idx}; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = {sector_idx}; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = {sector_idx}; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = {sector_idx}; texturemiddle = "STARTAN2"; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }}
"#,
            v0 = vbase,
            v1 = vbase + 1,
            v2 = vbase + 2,
            v3 = vbase + 3,
            s0 = sbase,
            s1 = sbase + 1,
            s2 = sbase + 2,
            s3 = sbase + 3,
        )
    }

    /// One `thing` block, flagged `single` only.
    fn thing_at(x: f64, y: f64, type_id: u16) -> String {
        format!("thing {{ x = {x:.3}; y = {y:.3}; type = {type_id}; single = true; }}\n")
    }

    /// The scene's floor actions, discarding the warnings — for the tests
    /// whose subject is a node or an edge built beside them rather than the
    /// floor resolution itself.
    fn floor_bits(scene: &Scene, tables: &Tables) -> FloorBits {
        resolve_floor_bits(scene, tables, &mut Vec::new())
    }

    /// Parses `text` and builds the [`Scene`] it resolves to, returning both
    /// it and whatever `"V-S"` findings `Scene::build` raised.
    fn scene_of(text: &str, tables: &Tables) -> (Scene, Vec<Finding>) {
        let map = parse_udmf(text, crustywad::Limits::default()).expect("fixture parses");
        let mut findings = Vec::new();
        let scene = Scene::build(&map, tables, &mut findings);
        (scene, findings)
    }

    #[test]
    fn a_map_with_no_player_start_is_a_hard_error_not_a_vacuous_pass() {
        let tables = Tables::load().expect("tables");
        let exit_special = tables.exit_switch_special();
        let text = room_chain(&[(0, 128, 160)], &[], Some((0, exit_special, 0)), "");
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(reached.is_none(), "no start: the flood cannot run");
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Map)
                && f.message.contains("no player 1 start")),
            "expected a V-P7 Map error naming the missing start: {findings:?}"
        );
    }

    #[test]
    fn a_map_with_no_exit_is_a_hard_error_not_a_vacuous_pass() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let things = thing_at(64.0, 64.0, start_id);
        let text = room_chain(&[(0, 128, 160)], &[], None, &things);
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(reached.is_none(), "no exit: the flood cannot run");
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Map)
                && f.message.contains("no exit")),
            "expected a V-P7 Map error naming the missing exit: {findings:?}"
        );
    }

    #[test]
    fn a_key_behind_its_own_locked_door_is_unfinishable() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let card_id = tables.thing_id("blue_card").expect("blue_card id");
        let locked = tables
            .locked_door_special("blue_card")
            .expect("blue_card has a locked-door special");
        let exit_special = tables.exit_switch_special();

        let mut things = thing_at(64.0, 64.0, start_id); // start room (0)
        things += &thing_at(192.0, 64.0, card_id); // the card room (1), behind the lock

        let text = room_chain(
            &[(0, 128, 160), (0, 128, 160)],
            &[(i32::from(locked), 0, false)],
            Some((1, exit_special, 0)),
            &things,
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(
            reached.is_some(),
            "start, exit, and class count are all fine — only the walk fails: {findings:?}"
        );
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Map)
                && f.message.contains("no feasible walk")),
            "the only card is behind the door it opens: expected unfinishable: {findings:?}"
        );
    }

    #[test]
    fn a_skull_key_satisfies_a_card_locked_door() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let skull_id = tables.thing_id("blue_skull").expect("blue_skull id");
        let locked = tables
            .locked_door_special("blue_card")
            .expect("blue_card has a locked-door special");
        let exit_special = tables.exit_switch_special();

        // Same shape as the unfinishable fixture, but the skull sits in the
        // START room instead of behind the door.
        let mut things = thing_at(32.0, 64.0, start_id);
        things += &thing_at(96.0, 64.0, skull_id);

        let text = room_chain(
            &[(0, 128, 160), (0, 128, 160)],
            &[(i32::from(locked), 0, false)],
            Some((1, exit_special, 0)),
            &things,
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(reached.is_some());
        assert!(
            findings.iter().all(|f| f.check != "V-P7"),
            "the skull opens the card's lock: no finding: {findings:?}"
        );
    }

    #[test]
    fn a_pit_the_player_cannot_climb_out_of_is_stranding() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let card_id = tables.thing_id("blue_card").expect("blue_card id");
        let exit_special = tables.exit_switch_special();
        let pit_floor = -(tables.step_height() + 8);

        let mut things = thing_at(64.0, 64.0, start_id); // start room also hosts the exit
        things += &thing_at(192.0, 64.0, card_id); // a key in the pit itself (room 1)
        let text = room_chain(
            &[(0, 128, 160), (pit_floor, 128, 160)],
            &[(0, 0, false)],
            Some((0, exit_special, 0)),
            &things,
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(reached.is_some());
        assert!(
            findings
                .iter()
                .all(|f| !(f.check == "V-P7" && f.message.contains("no feasible walk"))),
            "the exit is right there in the start room: {findings:?}"
        );

        // The pit must read as forward-*reachable but doomed* (stranded),
        // not merely absent from the graph the way a genuinely unreachable
        // sector would be — a mutation that drops the open edge into the
        // pit would turn this into an "unreachable" finding on the same
        // sector, which a subject-only assertion cannot tell apart from the
        // intended "stranded" finding.
        let reached = reached.expect("checked above");
        assert!(
            reached[1],
            "the pit is forward-reachable (you can walk into it); it is doomed, not unvisited"
        );
        let stranding = findings
            .iter()
            .find(|f| f.check == "V-P7" && matches!(f.subject, Subject::Sector(1)))
            .unwrap_or_else(|| panic!("expected a V-P7 finding naming the pit: {findings:?}"));
        assert!(
            stranding
                .message
                .contains("no walk from there reaches an exit"),
            "expected the stranded wording, not the unreachable one: {stranding:?}"
        );
        // The pit holds a blue_card: keys_in_words must render the class's
        // full kind list, not just the placed kind, pinning the same
        // card-or-skull wording `a_locked_door_edge_and_the_matching_key_
        // share_a_color_class` pins at the `reach.rs` layer.
        assert!(
            stranding.message.contains("blue_card/blue_skull"),
            "expected the color class's full kind list in the stranded wording: {stranding:?}"
        );
    }

    #[test]
    fn a_clean_two_room_map_is_fully_reached_with_no_findings() {
        // The plain positive case for `reached[]` and edge classification:
        // two level rooms joined by an ordinary open boundary, exit in the
        // far room. Both sectors reached, nothing to report.
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let exit_special = tables.exit_switch_special();
        let things = thing_at(64.0, 64.0, start_id);
        let text = room_chain(
            &[(0, 128, 160), (0, 128, 160)],
            &[(0, 0, false)],
            Some((1, exit_special, 0)),
            &things,
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(
            findings.is_empty(),
            "clean map: no findings at all: {findings:?}"
        );
        assert_eq!(
            reached,
            Some(vec![true, true]),
            "both sectors are forward-reachable"
        );
    }

    #[test]
    fn an_unreachable_sector_is_reported() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let exit_special = tables.exit_switch_special();
        let things = thing_at(64.0, 64.0, start_id);
        let chain = room_chain_ex(
            &[(0, 128, 160), (0, 128, 160)],
            &[(0, 0, false)],
            Some((0, exit_special, 0)),
            &things,
        );
        let text =
            chain.text + &isolated_box(chain.next_vertex, chain.next_sidedef, chain.next_sector);
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(reached.is_some());
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Sector(2))),
            "expected the isolated third sector (2) reported unreachable: {findings:?}"
        );
        let reached = reached.expect("checked above");
        assert!(!reached[2], "the isolated sector is not forward-reachable");
    }

    #[test]
    fn a_walkover_exit_is_a_goal_from_both_sides() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let walkover = tables.exit_walkover_special();
        // More than the step cap higher than the back room: climbing from
        // the back room to the front is blocked, even though the shared
        // line is a genuinely open (non-blocking) two-sided boundary — an
        // uncrossable line would never fire `P_CrossSpecialLine` in the
        // real engine, so this exit must actually be crossable both ways.
        let front_floor = tables.step_height() + 8;

        // Start in the BACK room (room 1) — the side
        // `reach::graph_from_compiled`'s "recess only" convention, or a
        // front-only bug, would *not* treat as a goal. The only way this
        // reads finishable is if the start's own room is already a goal:
        // climbing to the front room is blocked by the step cap, so a
        // front-only goal set would report this map unfinishable.
        let things = thing_at(192.0, 64.0, start_id);
        let text = room_chain(
            &[(front_floor, front_floor + 128, 160), (0, 128, 160)],
            &[(i32::from(walkover), 0, false)],
            None,
            &things,
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(reached.is_some());
        assert!(
            findings
                .iter()
                .all(|f| !(f.check == "V-P7" && f.message.contains("no feasible walk"))),
            "a walkover exit fires from either crossing side, so the start's own room (the \
             back room, unable to climb to the front) is already a goal: {findings:?}"
        );
    }

    #[test]
    fn a_blocking_walkover_exit_is_not_a_goal_and_reads_as_no_exit() {
        // The map's only exit line carries a walkover special but is also
        // flagged BLOCKING — `PIT_CheckLine` rejects it for any non-missile
        // before `P_TryMove`'s `spechit` bookkeeping is ever reached, so
        // `P_CrossSpecialLine` can never fire it ("Exit goals" above,
        // `Boundary::passable`'s own doc). `resolve_goals`'s `b.passable()`
        // gate must therefore find no goal anywhere, which is the same
        // shape as a map with no exit line at all: a `V-P7` Map error
        // reading "no exit line", not a flood that runs and finds the map
        // unfinishable (it never gets that far) and not a silent pass.
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let walkover = tables.exit_walkover_special();
        let things = thing_at(64.0, 64.0, start_id);
        let text = room_chain(
            &[(0, 128, 160), (0, 128, 160)],
            &[(i32::from(walkover), 0, true)],
            None,
            &things,
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(
            reached.is_none(),
            "an uncrossable walkover exit is not a goal from either side; the flood cannot \
             run: {findings:?}"
        );
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Map)
                && f.message.contains("no exit line")),
            "expected the no-exit-line V-P7 Map error: {findings:?}"
        );
        assert_eq!(
            findings.len(),
            1,
            "no unrelated finding joins it: {findings:?}"
        );
    }

    #[test]
    fn an_extra_player_start_is_reported_but_the_flood_still_runs_on_the_first() {
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let exit_special = tables.exit_switch_special();
        let mut things = thing_at(32.0, 64.0, start_id); // thing 0, the first start
        things += &thing_at(96.0, 64.0, start_id); // thing 1, the extra start
        let text = room_chain(&[(0, 128, 160)], &[], Some((0, exit_special, 0)), &things);
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(
            reached.is_some(),
            "an extra start does not stop the flood from running: {findings:?}"
        );
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(1))),
            "expected a V-P7 error naming the extra start (thing 1): {findings:?}"
        );
        assert!(
            findings
                .iter()
                .all(|f| !(f.check == "V-P7" && f.message.contains("no feasible walk"))),
            "the first start still reaches the exit: {findings:?}"
        );
    }

    #[test]
    fn an_orphan_key_and_a_keyless_lock_are_both_p24_errors() {
        let tables = Tables::load().expect("tables");
        let yellow_locked = tables
            .locked_door_special("yellow_card")
            .expect("yellow_card has a locked-door special");
        let red_skull_id = tables.thing_id("red_skull").expect("red_skull id");

        // room 0 -- yellow-locked door --> room 1 -- open --> room 2.
        // No yellow key anywhere (keyless lock); a red_skull placed in room
        // 2, with no red-locked door anywhere (orphan key).
        let things = thing_at(320.0, 64.0, red_skull_id);
        let text = room_chain(
            &[(0, 128, 160), (0, 128, 160), (0, 128, 160)],
            &[(i32::from(yellow_locked), 0, false), (0, 0, false)],
            None,
            &things,
        );
        let (scene, _) = scene_of(&text, &tables);
        let mut coherence = Vec::new();
        check_key_lock_coherence(&scene, &tables, &mut coherence);

        assert!(
            coherence.iter().any(|f| f.check == "V-P24"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Sector(1))
                && f.message.contains("yellow")),
            "expected a keyless-lock error naming the door's own (back) sector, 1: {coherence:?}"
        );
        assert!(
            coherence.iter().any(|f| f.check == "V-P24"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(0))),
            "expected an orphan-key error naming the red_skull thing: {coherence:?}"
        );
        assert_eq!(
            coherence.len(),
            2,
            "exactly these two defects, no more: {coherence:?}"
        );
    }

    #[test]
    fn a_misplaced_only_key_does_not_suppress_the_keyless_lock_error() {
        // The blue_card sits far outside both rooms' geometry — outside
        // every closed sector, already its own `V-S` Error from
        // `Scene::build` — and it is the map's ONLY blue key. Before the
        // fix, `key_present` counted every key thing regardless of
        // `thing.sector`, so this misplaced card silently satisfied the
        // blue-locked door's keyless-lock check even though the flood
        // correctly treats the door as unopenable (nobody can ever reach
        // the card to pick it up).
        let tables = Tables::load().expect("tables");
        let card_id = tables.thing_id("blue_card").expect("blue_card id");
        let locked = tables
            .locked_door_special("blue_card")
            .expect("blue_card has a locked-door special");

        // room 0 -- blue-locked door --> room 1.
        let things = thing_at(9000.0, 9000.0, card_id);
        let text = room_chain(
            &[(0, 128, 160), (0, 128, 160)],
            &[(i32::from(locked), 0, false)],
            None,
            &things,
        );
        let (scene, scene_findings) = scene_of(&text, &tables);
        assert!(
            scene_findings.iter().any(|f| f.check == "V-S"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(0))),
            "the misplaced card is its own V-S Error: {scene_findings:?}"
        );

        let mut coherence = Vec::new();
        check_key_lock_coherence(&scene, &tables, &mut coherence);
        assert!(
            coherence.iter().any(|f| f.check == "V-P24"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Sector(1))
                && f.message.contains("blue")),
            "a key nobody can reach must not satisfy the lock: expected the keyless-lock error \
             to still fire: {coherence:?}"
        );
    }

    #[test]
    fn a_misplaced_key_that_opens_no_door_is_not_reported_as_orphan() {
        // The red_skull sits far outside the map's one room — already its
        // own `V-S` Error — and no red-locked door exists anywhere in the
        // map. Before the fix, the orphan-key half ignored `thing.sector`,
        // so this misplaced key would also earn its own V-P24 orphan
        // finding on top of the V-S one it already has.
        let tables = Tables::load().expect("tables");
        let skull_id = tables.thing_id("red_skull").expect("red_skull id");

        let things = thing_at(9000.0, 9000.0, skull_id);
        let text = room_chain(&[(0, 128, 160)], &[], None, &things);
        let (scene, scene_findings) = scene_of(&text, &tables);
        assert!(
            scene_findings.iter().any(|f| f.check == "V-S"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Thing(0))),
            "the misplaced skull is its own V-S Error: {scene_findings:?}"
        );

        let mut coherence = Vec::new();
        check_key_lock_coherence(&scene, &tables, &mut coherence);
        assert!(
            coherence.is_empty(),
            "a key nobody can reach gets no orphan analysis of its own — only the geometry \
             error: {coherence:?}"
        );
    }

    #[test]
    fn a_two_faced_locked_door_reports_the_keyless_lock_once() {
        // Two linedefs, each fronting its own room, both backing onto the
        // SAME door sector (sector 1) — the shape `check_door_openings`
        // (V-P4) fixtures as `door_chain`: a real door has two faces. Dedup
        // must key on the door's own sector, not the linedef, or this
        // reports the identical keyless lock twice.
        let tables = Tables::load().expect("tables");
        let locked = tables
            .locked_door_special("yellow_card")
            .expect("yellow_card has a locked-door special");
        let text = format!(
            r#"namespace = "doom";
vertex {{ x = 0.000; y = 0.000; }}
vertex {{ x = 64.000; y = 0.000; }}
vertex {{ x = 96.000; y = 0.000; }}
vertex {{ x = 160.000; y = 0.000; }}
vertex {{ x = 160.000; y = 64.000; }}
vertex {{ x = 96.000; y = 64.000; }}
vertex {{ x = 64.000; y = 64.000; }}
vertex {{ x = 0.000; y = 64.000; }}
linedef {{ v1 = 1; v2 = 6; sidefront = 0; sideback = 1; twosided = true; special = {locked}; }}
linedef {{ v1 = 2; v2 = 5; sidefront = 2; sideback = 3; twosided = true; special = {locked}; }}
linedef {{ v1 = 0; v2 = 1; sidefront = 4; blocking = true; }}
linedef {{ v1 = 7; v2 = 0; sidefront = 5; blocking = true; }}
linedef {{ v1 = 6; v2 = 7; sidefront = 6; blocking = true; }}
linedef {{ v1 = 1; v2 = 2; sidefront = 7; blocking = true; }}
linedef {{ v1 = 6; v2 = 5; sidefront = 8; blocking = true; }}
linedef {{ v1 = 2; v2 = 3; sidefront = 9; blocking = true; }}
linedef {{ v1 = 3; v2 = 4; sidefront = 10; blocking = true; }}
linedef {{ v1 = 4; v2 = 5; sidefront = 11; blocking = true; }}
sidedef {{ sector = 0; texturemiddle = "-"; }}
sidedef {{ sector = 1; texturemiddle = "-"; }}
sidedef {{ sector = 2; texturemiddle = "-"; }}
sidedef {{ sector = 1; texturemiddle = "-"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 1; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 1; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 2; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 2; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 2; texturemiddle = "STARTAN2"; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 0; lightlevel = 160; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }}
"#
        );
        let (scene, _) = scene_of(&text, &tables);
        let mut coherence = Vec::new();
        check_key_lock_coherence(&scene, &tables, &mut coherence);
        let keyless: Vec<_> = coherence
            .iter()
            .filter(|f| f.check == "V-P24" && matches!(f.subject, Subject::Sector(1)))
            .collect();
        assert_eq!(
            keyless.len(),
            1,
            "the door sector's keyless lock is one defect, not one per face: {coherence:?}"
        );
    }

    #[test]
    fn an_l_shaped_room_in_the_chain_is_flooded_correctly() {
        // Fixture-diversity check: every other fixture in this module is a
        // row of rectangles. This one borders a non-convex L-shaped sector
        // (the same shape `check::scene`'s own `an_l_shaped_sector_contains_
        // its_notch_correctly` test uses) against an ordinary box via an
        // open boundary on one of the L's own segments, exercising boundary
        // iteration over a sector with more than four segments.
        let tables = Tables::load().expect("tables");
        let start_id = tables.thing_id("player1_start").expect("player1_start id");
        let exit_special = tables.exit_switch_special();
        let text = format!(
            r#"namespace = "doom";
vertex {{ x = 0.000; y = 0.000; }}
vertex {{ x = 96.000; y = 0.000; }}
vertex {{ x = 96.000; y = 32.000; }}
vertex {{ x = 32.000; y = 32.000; }}
vertex {{ x = 32.000; y = 96.000; }}
vertex {{ x = 0.000; y = 96.000; }}
vertex {{ x = 224.000; y = 0.000; }}
vertex {{ x = 224.000; y = 32.000; }}
linedef {{ v1 = 0; v2 = 1; sidefront = 0; blocking = true; }}
linedef {{ v1 = 1; v2 = 2; sidefront = 1; sideback = 2; twosided = true; }}
linedef {{ v1 = 2; v2 = 3; sidefront = 3; blocking = true; }}
linedef {{ v1 = 3; v2 = 4; sidefront = 4; blocking = true; }}
linedef {{ v1 = 4; v2 = 5; sidefront = 5; blocking = true; }}
linedef {{ v1 = 5; v2 = 0; sidefront = 6; blocking = true; }}
linedef {{ v1 = 2; v2 = 7; sidefront = 7; blocking = true; }}
linedef {{ v1 = 7; v2 = 6; sidefront = 8; special = {exit_special}; arg0 = 0; blocking = true; }}
linedef {{ v1 = 6; v2 = 1; sidefront = 9; blocking = true; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "-"; texturetop = "STARTAN2"; texturebottom = "STARTAN2"; }}
sidedef {{ sector = 1; texturemiddle = "-"; texturetop = "STARTAN2"; texturebottom = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 0; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 1; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 1; texturemiddle = "STARTAN2"; }}
sidedef {{ sector = 1; texturemiddle = "STARTAN2"; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }}
sector {{ texturefloor = "FLOOR4_8"; textureceiling = "CEIL3_5"; heightceiling = 128; lightlevel = 160; }}
thing {{ x = 16.000; y = 64.000; type = {start_id}; single = true; }}
"#
        );
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings);
        assert!(
            findings.iter().all(|f| f.check != "V-P7"),
            "clean L-shaped map: no V-P7 findings: {findings:?}"
        );
        assert_eq!(
            reached,
            Some(vec![true, true]),
            "both the L-shaped room and the box are forward-reachable"
        );
    }

    /// A [`Boundary`] with `special`, minimal everywhere else.
    fn boundary(
        special: i32,
        two_sided: bool,
        blocking: bool,
        neighbor: Option<usize>,
    ) -> Boundary {
        Boundary {
            a: (0.0, 0.0),
            b: (64.0, 0.0),
            linedef: 0,
            neighbor,
            two_sided,
            blocking,
            upper_unpegged: false,
            lower_unpegged: false,
            special,
            tag: 0,
            fronts_this: true,
            sidedef: 0,
        }
    }

    fn empty_sector() -> SceneSector {
        SceneSector {
            floor: 0,
            ceiling: 128,
            light: 160,
            special: 0,
            tag: 0,
            boundary: vec![],
            closed: true,
        }
    }

    #[test]
    fn keys_in_words_reports_no_keys_for_an_empty_mask() {
        assert_eq!(keys_in_words(0, &[]), "no keys");
    }

    #[test]
    fn resolve_goals_skips_a_boundary_whose_special_is_out_of_u16_range() {
        let tables = Tables::load().expect("tables");
        let mut sector = empty_sector();
        // -1 does not fit a u16, so `u16::try_from` fails and this boundary
        // must be skipped rather than panicking or miscounting as an exit.
        sector.boundary.push(boundary(-1, false, false, None));
        let scene = Scene {
            sectors: vec![sector],
            things: vec![],
        };
        let mut findings = Vec::new();
        let goals = resolve_goals(&scene, &tables, &mut findings);
        assert_eq!(goals, None, "the only boundary was skipped: no exit found");
        assert!(
            findings
                .iter()
                .any(|f| f.check == "V-P7" && f.message.contains("no exit line")),
            "got {findings:?}"
        );
    }

    #[test]
    fn build_nodes_skips_an_unnamed_thing_and_a_key_outside_every_sector() {
        let tables = Tables::load().expect("tables");
        let (specials, _class_names) = intern_lock_classes(&tables).expect("small vocabulary");
        let kinds = tables.locked_door_kinds();
        let scene = Scene {
            sectors: vec![empty_sector()],
            things: vec![
                SceneThing {
                    x: 0.0,
                    y: 0.0,
                    angle: 0,
                    type_id: 31337,
                    flags: 0,
                    sector: Some(0),
                    name: None,
                },
                SceneThing {
                    x: 0.0,
                    y: 0.0,
                    angle: 0,
                    type_id: 5,
                    flags: 0,
                    sector: None,
                    name: Some("blue_card".to_owned()),
                },
            ],
        };
        let nodes = build_nodes(&scene, &specials, &kinds, &floor_bits(&scene, &tables));
        assert_eq!(nodes.len(), 1);
        assert_eq!(
            nodes[0].keys, 0,
            "the unnamed thing and the unresolved key both contribute no key bit"
        );
    }

    #[test]
    fn build_edges_skips_a_blocking_twosided_boundary() {
        let tables = Tables::load().expect("tables");
        let (specials, _class_names) = intern_lock_classes(&tables).expect("small vocabulary");
        let mut front = empty_sector();
        front.boundary.push(boundary(0, true, true, Some(1)));
        let scene = Scene {
            sectors: vec![front, empty_sector()],
            things: vec![],
        };
        let edges = build_edges(
            &scene,
            &tables,
            &specials,
            true,
            &floor_bits(&scene, &tables),
        );
        assert!(
            edges.is_empty(),
            "a blocking twosided boundary is a wall to the flood: {edges:?}"
        );
    }

    #[test]
    fn build_edges_dedupes_teleport_edges_by_destination() {
        let (scene, tables) = fixtures::scene_of(TELEPORT_MAP);
        let (specials, _class_names) = intern_lock_classes(&tables).expect("small vocabulary");
        let edges = build_edges(
            &scene,
            &tables,
            &specials,
            true,
            &floor_bits(&scene, &tables),
        );
        let teleports: Vec<&Edge> = edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Teleport)
            .collect();
        assert_eq!(
            teleports.len(),
            1,
            "the island pad's four boundaries share one (front, destination) pair: {edges:?}"
        );
        assert_eq!(teleports[0].a, 0, "the pad's front sector is sector 0");
        assert_eq!(teleports[0].b, 1, "the tag resolves to the marker sector");
    }

    // --- Teleport edges: the directed edge a trigger line adds, and the
    // teleport-only predicate built by running the flood twice.
    // `TELEPORT_MAP` and its `scene_of` live in `check::fixtures` so the
    // invariants and conformance tests read the same text;
    // `fixtures::scene_of` is spelled out here rather than imported bare
    // because this module already has a `scene_of` of its own (a different
    // return shape). ---

    #[test]
    fn a_teleport_line_adds_a_one_way_edge_to_the_marker_sector() {
        let (scene, tables) = fixtures::scene_of(TELEPORT_MAP);
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("flood ran");
        assert!(findings.is_empty(), "{findings:?}");
        assert!(reached[1], "the marker sector is reached by teleport");
        assert!(reached[2], "and the exit alcove beyond it");
        let only = teleport_only_sectors(&scene, &tables).expect("both floods ran");
        assert_eq!(
            only,
            vec![false, true, true, false],
            "sectors 1 and 2 are reachable only by teleport"
        );
    }

    #[test]
    fn a_monsters_only_line_adds_no_player_edge() {
        let (scene, tables) =
            fixtures::scene_of(&TELEPORT_MAP.replace("special = 97;", "special = 126;"));
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("flood ran");
        assert!(
            !reached[1],
            "a monsters-only teleport moves no player, so the marker sector stays unreached"
        );
        assert!(
            findings.iter().any(|f| f.check == "V-P7"),
            "no walk reaches the exit: {findings:?}"
        );
    }

    #[test]
    fn a_blocking_teleport_line_yields_no_edge() {
        // `PIT_CheckLine` rejects the crossing before `P_CrossSpecialLine`
        // ever runs, so the teleport fires no more than a walkover exit on
        // the same line would.
        let (scene, tables) = fixtures::scene_of(&TELEPORT_MAP.replace(
            "special = 97; arg0 = 5; }",
            "special = 97; arg0 = 5; blocking = true; }",
        ));
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("flood ran");
        assert!(!reached[1], "an uncrossable teleport line moves nobody");
        assert!(
            findings.iter().any(|f| f.check == "V-P7"),
            "no walk reaches the exit: {findings:?}"
        );
        let only = teleport_only_sectors(&scene, &tables).expect("both floods ran");
        assert!(
            only.iter().all(|&t| !t),
            "no sector is reachable only by a teleport that never fires: {only:?}"
        );
    }

    #[test]
    fn resolution_takes_the_first_tagged_sector_holding_a_marker() {
        // Give sector 0 the same tag as sector 1: sector 0 holds no marker,
        // so sector 1 still resolves — EV_Teleport's own order.
        let (scene, tables) = fixtures::scene_of(&TELEPORT_MAP.replacen(
            "lightlevel = 160; }",
            "lightlevel = 160; id = 5; }",
            1,
        ));
        assert_eq!(resolve_teleport_destination(&scene, &tables, 5), Some(1));
    }

    #[test]
    fn a_lift_called_from_below_makes_the_climb_finishable_and_a_top_only_one_strands_the_start() {
        let tables = Tables::load().expect("tables");
        let exit = tables.exit_switch_special();
        // `room_chain_ex` writes every sector as `sector { texturefloor ...`
        // and declares no `id`: tag the *second* one (the platform) by
        // splicing an `id` into the first two and undoing the first.
        let tagged = |text: String| {
            text.replacen("sector { texturefloor", "sector { id = 7; texturefloor", 2)
                .replacen("sector { id = 7; texturefloor", "sector { texturefloor", 1)
        };
        let text = tagged(room_chain(
            &[(0, 128, 160), (128, 256, 160), (128, 256, 160)],
            &[(62, 7, false), (0, 0, false)],
            Some((2, exit, 0)),
            &thing_at(64.0, 64.0, 1),
        ));
        let (scene, mut findings) = scene_of(&text, &tables);
        let reached = run_flood(&scene, &tables, &mut findings).expect("flood ran");
        assert!(
            reached.iter().all(|&r| r),
            "every sector reachable via the lift: {findings:?}"
        );
        assert!(!findings.iter().any(|f| f.check == "V-P7"), "{findings:?}");

        // Flip the switch line so its front is the level room: no way up
        // from the start. `write_link` fronts each link on its *west* room,
        // so moving the special from the first link (rooms 0 | 1) to the
        // second (rooms 1 | 2) makes the platform itself the only side that
        // can fire it.
        let flipped = text.replacen("special = 62;", "special = 0;", 1).replacen(
            "twosided = true; }",
            "twosided = true; special = 62; arg0 = 7; }",
            1,
        );
        let (scene, mut findings) = scene_of(&flipped, &tables);
        // Pin what the splice produced, so a stranded start proves the
        // *top-only* lift rather than a fixture that lost its lift line.
        let plats = plats::resolve_plats(&scene, &tables);
        assert_eq!(plats.len(), 1, "the platform is still named by tag 7");
        assert!(
            !plats[0].callable_low() && plats[0].callable_top(),
            "the moved switch fires only from the platform's own side: {:?}",
            plats[0].triggers
        );
        // The flood's own return is irrelevant here; the finding it pushes
        // is the assertion (`run_flood` is `#[must_use]`).
        let _ = run_flood(&scene, &tables, &mut findings);
        assert!(
            findings.iter().any(|f| f.check == "V-P7"),
            "the start is stranded below a top-only lift: {findings:?}"
        );
    }

    #[test]
    fn a_platform_only_a_remote_switch_calls_is_still_boarded_from_its_low_neighbor() {
        // Five rooms in a row. The platform is room 1 (tag 7); it borders
        // room 0 (low) and room 2 (level). The switch sits on the line
        // between rooms 3 and 4 and fronts room 3 — a `Low` activator the
        // platform does not border, so `low_activator_neighbors` is empty
        // while `callable_low` holds, which is the remote-only branch.
        let (scene, tables) = fixtures::scene_of(&fixtures::chain(
            &[0, 128, 128, 0, 0],
            &[0, 7, 0, 0, 0],
            &[(0, 0, false), (0, 0, false), (0, 0, false), (62, 7, false)],
            "",
        ));
        let (specials, _class_names) = intern_lock_classes(&tables).expect("small vocabulary");
        let plats = plats::resolve_plats(&scene, &tables);
        assert_eq!(plats.len(), 1);
        assert_eq!(
            plats[0].triggers[0].activators,
            vec![(3, plats::Activator::Low)],
            "the switch fires from room 3 alone"
        );
        assert!(
            plats[0].low_activator_neighbors().is_empty() && plats[0].callable_low(),
            "a Low activator the platform does not border"
        );
        let edges = build_edges(
            &scene,
            &tables,
            &specials,
            true,
            &floor_bits(&scene, &tables),
        );
        let lifts: Vec<&Edge> = edges.iter().filter(|e| e.kind == EdgeKind::Lift).collect();
        assert_eq!(
            lifts.len(),
            1,
            "the low room boards the platform the remote switch lowers: {edges:?}"
        );
        assert_eq!(
            (lifts[0].a, lifts[0].b),
            (0, 1),
            "from the low neighbor, not the level one"
        );
    }

    #[test]
    fn a_platform_behind_a_blocking_line_is_not_boardable_across_it() {
        // The same riser switch, on a two-sided line flagged solid: a window
        // the player presses through but cannot walk through.
        let (scene, tables) = fixtures::scene_of(
            &fixtures::chain(
                &[0, 128, 128],
                &[0, 7, 0],
                &[(62, 7, false), (0, 0, false)],
                "",
            )
            .replacen(
                "special = 62; arg0 = 7; }",
                "special = 62; arg0 = 7; blocking = true; }",
                1,
            ),
        );
        let (specials, _class_names) = intern_lock_classes(&tables).expect("small vocabulary");
        let plats = plats::resolve_plats(&scene, &tables);
        assert_eq!(
            plats[0].low_activator_neighbors(),
            BTreeSet::from([0]),
            "the switch still fires: `P_UseSpecialLine` is a raycast, not a crossing"
        );
        let edges = build_edges(
            &scene,
            &tables,
            &specials,
            true,
            &floor_bits(&scene, &tables),
        );
        assert!(
            !edges.iter().any(|e| e.kind == EdgeKind::Lift),
            "but nobody can board across a blocking line: {edges:?}"
        );
    }

    /// One `player1_start` in room `i` of a [`fixtures::chain`], centered
    /// in its 128-unit box. The floor-action fixtures below spell their
    /// start this way.
    fn start_in_room(i: usize) -> String {
        let tables = Tables::load().expect("tables");
        let start = tables.thing_id("player1_start").expect("player1_start id");
        #[expect(
            clippy::cast_precision_loss,
            reason = "these fixtures are a handful of rooms wide, far under f64's mantissa"
        )]
        let x = i as f64 * 128.0 + 64.0;
        thing_at(x, 64.0, start)
    }

    #[test]
    fn a_drop_wall_a_remote_switch_lowers_is_crossed_once_the_switch_room_is_entered() {
        // A(0, start) — T(128, ceiling 128: a solid slab) — B(0, exit), with
        // a 23 (S1 lowerFloorToLowest) naming T's tag on a wall of A itself.
        let mut text = fixtures::chain_full(
            &[0, 128, 0],
            &[256, 128, 256],
            &[0, 7, 0],
            &[(0, 0, false), (0, 0, false)],
            &start_in_room(0),
        );
        fixtures::far_wall(&mut text, 3, 11, 0);
        // Room 0's west wall is the first `v1 = 0; v2 = 1;` line the builder
        // writes; rewriting it in place is how the switch lands on a wall
        // that is neither the target nor a side the action moves.
        let text = text.replacen(
            "linedef { v1 = 0; v2 = 1; sidefront = ",
            "linedef { v1 = 0; v2 = 1; special = 23; arg0 = 7; sidefront = ",
            1,
        );
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("start and exit exist");
        assert_eq!(
            reached,
            vec![true, true, true],
            "the slab drops flush the moment the start room is entered: {findings:?}"
        );
        assert!(!findings.iter().any(|f| f.check == "V-P7"), "{findings:?}");
    }

    /// The verifier's own half of the compiler's
    /// `p7_names_the_floor_actions_still_at_rest_in_a_stranded_state`
    /// (`crate::rules`): a stranded state names the floor targets it leaves
    /// at rest, with the direction each moves, alongside the keys held.
    #[test]
    fn a_stranded_state_names_the_floor_target_it_leaves_at_rest() {
        // P(-32, a dead-end drop off the start) — hub(0, start) — b(0) —
        // T(128, tag 7, the wall) — c(0, exit). The 23 S1 sits on the b|T
        // link with b on its front, so the wall is only lowered from b:
        // the player who walks into P first never fired it, which is the
        // state the finding has to describe.
        let tables = Tables::load().expect("tables");
        let mut text = fixtures::chain(
            &[-32, 0, 0, 128, 0],
            &[0, 0, 0, 7, 0],
            &[(0, 0, false), (0, 0, false), (23, 7, false), (0, 0, false)],
            &start_in_room(1),
        );
        fixtures::far_wall(&mut text, 5, i32::from(tables.exit_switch_special()), 0);
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("start and exit exist");
        assert!(reached[0], "the player can walk down into the dead end");
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("no feasible walk")),
            "the route through b finishes the map: {findings:?}"
        );
        let stranding = findings
            .iter()
            .find(|f| f.check == "V-P7" && matches!(f.subject, Subject::Sector(0)))
            .unwrap_or_else(|| panic!("expected a V-P7 naming the dead end: {findings:?}"));
        assert_eq!(
            stranding.message,
            "reachable holding no keys, but no walk from there reaches an exit; sector 3 not \
             lowered"
        );
    }

    #[test]
    fn a_bridge_pit_whose_walkover_lies_beyond_it_strands_the_player_who_drops_in() {
        // A(0, start) — T(-96, rises to 0 on a 119 W1) — B(0, exit), with the
        // W1 on the T|B link: crossing T -> B is a 96 climb the engine
        // refuses, so the line fires only from B, which is beyond the pit.
        let mut text = fixtures::chain(
            &[0, -96, 0],
            &[0, 7, 0],
            &[(0, 0, false), (119, 7, false)],
            &start_in_room(0),
        );
        fixtures::far_wall(&mut text, 3, 11, 0);
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("start and exit exist");
        assert_eq!(
            reached,
            vec![true, true, false],
            "the start drops into the pit and never climbs out: {findings:?}"
        );
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Error
                && matches!(f.subject, Subject::Map)
                && f.message
                    .contains("no feasible walk from the start reaches any exit")),
            "unfinishable, which is how `push_flood_findings` words it: {findings:?}"
        );
    }

    #[test]
    fn the_same_bridge_whose_walkover_is_on_the_near_threshold_carries_the_player_across() {
        // The pit above with its 119 moved to the A|T link: dropping in
        // fires the line on the way, so T stands at 0 and B is a walk away.
        let mut text = fixtures::chain(
            &[0, -96, 0],
            &[0, 7, 0],
            &[(119, 7, false), (0, 0, false)],
            &start_in_room(0),
        );
        fixtures::far_wall(&mut text, 3, 11, 0);
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("start and exit exist");
        assert_eq!(
            reached,
            vec![true, true, true],
            "the bridge rises under the player who crossed it: {findings:?}"
        );
        assert!(!findings.iter().any(|f| f.check == "V-P7"), "{findings:?}");
    }

    /// A closet-shaped target — floor resting on its own ceiling — holding
    /// a monster gets no bit: the engine restores a blocked floor and keeps
    /// the thinker running, so it never opens. The control half is the same
    /// map with the imp removed, which *is* modeled — without it the
    /// assertion could not tell "declined for the thing" from "declined for
    /// the shape".
    ///
    /// Room 1 rests at 256 under a 256 ceiling (zero headroom) and is driven
    /// by a `23` on the link from room 0; `P_FindLowestFloorSurrounding`
    /// finds room 0's and room 2's floors at 0, so the action lowers and the
    /// blocked-thing rule applies. Built with [`fixtures::chain_full`] rather
    /// than [`fixtures::far_wall`] because the thing has to land in a room
    /// whose sector the scene resolves.
    #[test]
    fn a_lowering_target_holding_a_monster_that_does_not_fit_gets_no_bit() {
        let tables = Tables::load().expect("tables");
        let imp = tables.thing_id("imp").expect("imp");
        let sealed = |things: &str| {
            fixtures::chain_full(
                &[0, 256, 0],
                &[256, 256, 256],
                &[0, 7, 0],
                &[(23, 7, false), (0, 0, false)],
                things,
            )
        };

        // Control: nothing inside, so the cell is modeled and takes a bit.
        let (scene, tables_c) = fixtures::scene_of(&sealed(""));
        let mut findings = Vec::new();
        let bits = resolve_floor_bits(&scene, &tables_c, &mut findings);
        assert!(
            bits.actions[1].is_some(),
            "an empty sealed cell is modeled: {findings:?}"
        );
        assert!(
            findings.iter().all(|f| f.check != "V-P7"),
            "and warns about nothing: {findings:?}"
        );

        // The imp stands at room 1's center (room i spans x in
        // [i*128, (i+1)*128], y in [0, 128]).
        let (scene, tables_b) = fixtures::scene_of(&sealed(&thing_at(192.0, 64.0, imp)));
        let mut findings = Vec::new();
        let bits = resolve_floor_bits(&scene, &tables_b, &mut findings);
        assert_eq!(
            bits.actions[1], None,
            "no bit for a floor the imp blocks: {findings:?}"
        );
        let blocked = findings
            .iter()
            .find(|f| f.check == "V-P7" && matches!(f.subject, Subject::Sector(1)))
            .unwrap_or_else(|| panic!("expected a V-P7 naming the sealed cell: {findings:?}"));
        assert_eq!(blocked.severity, Severity::Warning);
        assert!(
            blocked.message.contains("is blocked by `imp`")
                && blocked
                    .message
                    .contains("0 units of headroom against the 56")
                && blocked.message.contains("never lowers")
                && blocked.message.contains("rest floor 256"),
            "expected the blocked-thing wording naming the species and both heights: {blocked:?}"
        );
    }

    #[test]
    fn floor_targets_the_flood_cannot_model_stay_at_rest_and_warn() {
        // Two engine types on one tag: 23 (lowerFloorToLowest) and 18
        // (raiseFloorToNearest) name the same sector, so it has no one
        // destination.
        let (scene, tables) = fixtures::scene_of(&fixtures::chain(
            &[0, 128, 0],
            &[0, 7, 0],
            &[(23, 7, false), (18, 7, false)],
            "",
        ));
        let mut findings = Vec::new();
        let bits = resolve_floor_bits(&scene, &tables, &mut findings);
        assert_eq!(bits.actions[1], None, "no bit for an ambiguous destination");
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Warning
                && matches!(f.subject, Subject::Sector(1))
                && f.message.contains("engine types")),
            "{findings:?}"
        );

        // A 30 (W1 raiseToTexture): the destination is a texture height
        // neither this checker nor the probe resolves.
        let mut text = fixtures::chain(&[0, 0, 0], &[0, 7, 0], &[(0, 0, false), (0, 0, false)], "");
        fixtures::far_wall(&mut text, 3, 30, 7);
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let bits = resolve_floor_bits(&scene, &tables, &mut findings);
        assert_eq!(bits.actions[1], None, "no bit for an unresolved height");
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && f.severity == Severity::Warning
                && matches!(f.subject, Subject::Sector(1))
                && f.message.contains("texture height")),
            "{findings:?}"
        );

        // Eleven targets, one per link of a twelve-room chain, with an 18
        // added on the tags of the *first* and the *tenth* so those two are
        // two-type. The mask holds eight, and this pins three things at
        // once: an early decline costs no bit (sectors 2..=9 are modeled,
        // not 2..=8), a late decline is reported for its own reason rather
        // than for the mask being full (sector 10 reads "engine types"), and
        // the target after that is the one the cap actually stops.
        let floors = [0; 12];
        let tags: Vec<i32> = (0..12).collect();
        let links: Vec<(i32, i32, bool)> = (1..12).map(|tag| (23, tag, false)).collect();
        let mut text = fixtures::chain(&floors, &tags, &links, "");
        fixtures::far_wall(&mut text, 12, 18, 1);
        fixtures::far_wall(&mut text, 12, 18, 10);
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let bits = resolve_floor_bits(&scene, &tables, &mut findings);
        let modeled: Vec<usize> = (0..12).filter(|&i| bits.actions[i].is_some()).collect();
        assert_eq!(
            modeled,
            (2..10).collect::<Vec<_>>(),
            "eight bits, and the declined sector 1 spent none of them"
        );
        let warnings: Vec<(Subject, &str)> = findings
            .iter()
            .filter(|f| f.check == "V-P7" && f.severity == Severity::Warning)
            .map(|f| (f.subject, f.message.as_str()))
            .collect();
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        assert_eq!(
            warnings.iter().map(|&(s, _)| s).collect::<Vec<_>>(),
            vec![Subject::Sector(1), Subject::Sector(10), Subject::Sector(11)],
            "{warnings:?}"
        );
        assert!(warnings[0].1.contains("engine types"), "{warnings:?}");
        assert!(
            warnings[1].1.contains("engine types"),
            "declined for its own reason, not for the full mask: {warnings:?}"
        );
        assert!(warnings[2].1.contains("past the first 8"), "{warnings:?}");
    }

    /// A gun line fires from either sector it faces, so a target it names is
    /// raised by a player standing on the line's **back** side —
    /// `P_ShootSpecialLine` (pinned `p_spec.c:955-1000`) takes no `side`
    /// argument, and `PTR_ShootTraverse` passes none (`p_map.c:919-920`).
    ///
    /// The fixture puts the shot's front sector out of reach so the two
    /// readings differ: S (floor 128 under a 128 ceiling) is a sealed slab
    /// no walk can enter, and the `47` on the S|A line names the pit P. Read
    /// as front-only, the bit would wait on a room the player can never
    /// stand in, the pit would stay at −96, and the map would be
    /// unfinishable.
    #[test]
    fn a_gun_line_raises_its_target_for_a_player_on_the_lines_back_side() {
        // S(128, sealed) | A(0, start) — P(-96, tag 7) — E(0, exit).
        let mut text = fixtures::chain_full(
            &[128, 0, -96, 0],
            &[128, 256, 256, 256],
            &[0, 0, 7, 0],
            &[(47, 7, false), (0, 0, false), (0, 0, false)],
            &start_in_room(1),
        );
        fixtures::far_wall(&mut text, 4, 11, 0);
        let (scene, tables) = fixtures::scene_of(&text);
        let mut findings = Vec::new();
        let reached = run_flood(&scene, &tables, &mut findings).expect("start and exit exist");
        assert_eq!(
            reached,
            vec![false, true, true, true],
            "the shot crosses the sealed slab's face from the start room: {findings:?}"
        );
        assert!(
            !findings
                .iter()
                .any(|f| f.message.contains("no feasible walk")),
            "the pit rises, so the exit is a walk away: {findings:?}"
        );
        // The slab itself is the one sector no walk reaches: floor and
        // ceiling both 128 leave no opening to cross.
        assert!(
            findings.iter().any(|f| f.check == "V-P7"
                && matches!(f.subject, Subject::Sector(0))
                && f.message.contains("never reached")),
            "{findings:?}"
        );
    }
}
