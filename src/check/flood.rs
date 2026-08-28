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
//! # Key classes
//!
//! Interned the same way [`reach::graph_from_compiled`] does: by the locked
//! special a key opens ([`Tables::locked_door_kinds`]), not by key-thing
//! name, so a card and skull of one colour share a class (`EV_VerticalDoor`,
//! pinned `p_doors.c:371-403`, accepts either). [`run_flood`] reports a hard
//! finding rather than panicking when the vocabulary ever lists more classes
//! than a [`reach::KeyMask`] can hold — this module runs on arbitrary input,
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

use crate::check::scene::Scene;
use crate::check::{Finding, Severity, Subject};
use crate::reach::{self, Edge, EdgeKind, KeyClass, KeyMask, Limits, Node, ReachGraph};
use crate::tables::Tables;
use std::collections::BTreeSet;

/// Interns the vocabulary's locked-door specials into key classes: sorted,
/// deduped `special` values alongside the key-kind names each class covers
/// (mirrors [`reach::graph_from_compiled`]'s own interning, computed
/// independently per this module's doc). `class_names[c]` is every key kind
/// sharing class `c`'s special, e.g. `["blue_card", "blue_skull"]`.
///
/// Returns `None` — pushing no finding itself, since callers react
/// differently to it — when the vocabulary lists more classes than a
/// [`KeyMask`] can represent.
fn intern_lock_classes(tables: &Tables) -> Option<(Vec<u16>, Vec<Vec<String>>)> {
    let kinds = tables.locked_door_kinds();
    let mut specials: Vec<u16> = kinds.iter().map(|&(_, s)| s).collect();
    specials.sort_unstable();
    specials.dedup();
    // COVERAGE: unreachable through the public `Tables` API. `Tables::load`
    // only ever parses the two `include_str!`-embedded tables compiled into
    // this crate, and the pinned `data/vocabulary.toml` lists exactly three
    // distinct locked-door specials (26/27/28 — blue/yellow/red, card and
    // skull of a colour sharing one special), far under `KeyMask::BITS`
    // (8). Exercising this branch would need a `Tables` built from a
    // vocabulary this crate does not ship, which no constructor offers.
    if specials.len() > KeyMask::BITS as usize {
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
/// colour class with more than one kind joins those with `/`, matching
/// `rules.rs`'s own `check_reachability` wording), or `"no keys"` for an
/// empty mask.
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

/// Builds one [`Node`] per scene sector: floor/ceiling verbatim, `keys` set
/// from every thing whose name is a key kind ([`Tables::locked_door_kinds`])
/// with a resolved sector, unioned bit by interned [`KeyClass`].
fn build_nodes(scene: &Scene, specials: &[u16], kinds: &[(String, u16)]) -> Vec<Node> {
    let mut nodes: Vec<Node> = scene
        .sectors
        .iter()
        .map(|s| Node {
            floor: s.floor,
            ceiling: s.ceiling,
            keys: 0,
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
/// per `fronts_this` **passable** boundary carrying a player teleport
/// special, from that boundary's own sector to whatever
/// [`resolve_teleport_destination`] resolves its tag to — built ahead of
/// the `neighbor` filter above, since a teleport's destination is its
/// tag's, not its own back sector's, but behind the same crossability gate
/// every other edge answers to. Passing
/// `false` builds the same graph with those edges left out, which is how
/// [`teleport_only_sectors`] measures what a teleport is load-bearing for.
fn build_edges(scene: &Scene, tables: &Tables, specials: &[u16], teleports: bool) -> Vec<Edge> {
    let plain_door = tables.door_special();
    let player_teleports = tables.player_teleport_specials();
    let mut edges = Vec::new();
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
            {
                edges.push(Edge {
                    a: i,
                    b: dest,
                    kind: EdgeKind::Teleport,
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
            });
        }
    }
    edges
}

/// Maps a completed [`reach::Findings`] onto [`Finding`]s: `unfinishable` is
/// one `Subject::Map` Error; `stranded` entries are reported only when
/// finishable (an unfinishable map's stranded list is the degenerate "every
/// visited state" case — that fact is already the unfinishable finding's
/// story, not a fresh one per node), each naming its sector and the key
/// classes held, in words, via [`keys_in_words`]; every `unreachable` sector
/// is its own `Subject::Sector` Error.
fn push_flood_findings(
    result: &reach::Findings,
    class_names: &[Vec<String>],
    findings: &mut Vec<Finding>,
) {
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
                    "reachable holding {}, but no walk from there reaches an exit",
                    keys_in_words(mask, class_names)
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

/// Runs the V-P7 flood over `scene` and pushes its findings.
///
/// Returns `Some(reached)` — one entry per scene sector, `reached[i]` true
/// iff sector `i` is forward-reachable from the player 1 start — when the
/// flood ran at all, for [`crate::check::invariants::check_pickup_reachability`]
/// (V-P20) to consume. Returns `None`, the reason already pushed as a
/// [`Finding`] by `resolve_start` or `resolve_goals`, when it could not
/// run: no `player1_start` thing, the first start resolved to no sector, no
/// exit line, or (below) more locked-door classes than a [`KeyMask`] can
/// represent.
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
                "the vocabulary lists more than {} distinct lock classes, which a KeyMask \
                 cannot represent — the flood cannot run",
                KeyMask::BITS
            ),
        });
        return None;
    };

    let kinds = tables.locked_door_kinds();
    let nodes = build_nodes(scene, &specials, &kinds);
    let edges = build_edges(scene, tables, &specials, true);

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
    push_flood_findings(&result, &class_names, findings);

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
    let nodes = build_nodes(scene, &specials, &kinds);
    let limits = Limits {
        player_height: tables.player().height,
        max_step: tables.step_height(),
    };
    let reachable_with = |teleports: bool| {
        let graph = ReachGraph {
            nodes: nodes.clone(),
            edges: build_edges(scene, tables, &specials, teleports),
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
/// key thing of its colour class placed, and every placed key thing opens
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
/// more lock classes than a [`KeyMask`] can hold — [`run_flood`] is the one
/// that reports that as its own hard finding, and it always runs first in
/// [`crate::check::run`]'s wiring.
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
        // share_a_colour_class` pins at the `reach.rs` layer.
        assert!(
            stranding.message.contains("blue_card/blue_skull"),
            "expected the colour class's full kind list in the stranded wording: {stranding:?}"
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
        let nodes = build_nodes(&scene, &specials, &kinds);
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
        let edges = build_edges(&scene, &tables, &specials, true);
        assert!(
            edges.is_empty(),
            "a blocking twosided boundary is a wall to the flood: {edges:?}"
        );
    }

    // --- Task 7: teleport edges. `TELEPORT_MAP` and its `scene_of` live in
    // `check::fixtures` so the invariants and conformance tests read the
    // same text; `fixtures::scene_of` is spelled out here rather than
    // imported bare because this module already has a `scene_of` of its own
    // (a different return shape). ---

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
}
