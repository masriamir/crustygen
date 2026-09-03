//! Rule P7 (no softlock): a key-aware reachability flood over the map.
//!
//! P7 asks the only question the rest of the catalog cannot: can the compiled
//! map actually be *finished*, and can the player be trapped on the way? This
//! module is the pure core of that answer — an abstract traversal graph and the
//! search over it, deliberately not a compile pass, because the algorithm is
//! verifier-grade: a `TEXTMAP`-parsing checker must be able to re-derive the
//! same graph from emitted output and reuse this search untouched.
//!
//! # Why states, not a set-union flood
//!
//! The obvious implementation — accumulate the set of reachable sectors, unlock
//! a door once its key is anywhere in that set, iterate to a fixpoint — cannot
//! catch the defect that motivates the rule. A shipped map authored `key_room`
//! as a dead-end pit 32 units below `hub` holding the only blue card: the room
//! is reachable (you can drop in), so the card counts as obtained, so the
//! locked door opens, so the exit reads reachable. The player who takes that
//! drop is stranded, holding the key. Reachability of *places* and *items* is
//! not the property being checked. The property is over **states** —
//! `(where the player stands, which keys they hold, which floor actions have
//! fired)` — so that "holds the blue card" and "is stuck in `key_room`" are one
//! state and the search cannot pretend otherwise. The state space is sectors ×
//! 2^bits over a handful of key classes and floor actions, so this is a small
//! product-graph BFS, not a solver.
//!
//! # Floor actions are bits in the same mask
//!
//! A floor action — a drop wall, a revealed closet or pedestal, a bridge —
//! moves one sector's floor exactly once, and whether it has fired is as much
//! part of the player's situation as which keys they hold: the wall that seals
//! the exit is a wall until its switch is pressed, and the pit that strands
//! them is a pit until its walkover is crossed. So a fired action is one more
//! bit in the same [`KeyMask`] (bits at or above [`ACTION_BIT_BASE`]), set by
//! entering a node that fires it ([`Node::fires`] — a switch's room) or by
//! crossing an edge that does ([`Edge::fires`] — a walkover's line), and read
//! back by [`Node::effective_floor`], which is the floor every geometric rule
//! measures. Like keys, these bits only ever accumulate along a walk: the four
//! specials the compiler writes are the one-shot S1/W1 forms, so an action
//! that has fired stays fired.
//!
//! # The two passability rules
//!
//! Both are `P_TryMove`'s, verified against the pinned engine commit
//! `a77dfb96cb91780ca334d0d4cfd86957558007e0` and recorded with their line
//! numbers in `docs/reachability.md`:
//!
//! - **Step up.** `tmfloorz - thing->z > 24*FRACUNIT` rejects the move
//!   (`p_map.c:477-479`) — the cap is on climbing only. Descents are free: the
//!   dropoff rejection (`p_map.c:481-483`) is gated on
//!   `!(thing->flags & (MF_DROPOFF|MF_FLOAT))` and `MT_PLAYER` carries
//!   `MF_DROPOFF` (`info.c:1130`), so it never applies to the player. This is
//!   why retired rule P1's direction-blind height cap was wrong.
//! - **Crossing window.** `P_LineOpening` (`p_maputl.c:300-329`) opens a
//!   two-sided line to `min(ceilings) - max(floors)`, and `P_TryMove` rejects
//!   when `tmceilingz - tmfloorz < thing->height` (`p_map.c:468-469`). The
//!   third check, "the mobj must lower itself to fit" (`p_map.c:474-475`), is
//!   subsumed for ground movement — a walking player's `z` is a floor at or
//!   below `tmfloorz` — so the model needs exactly two rules, not three.
//!
//! [`EdgeKind::Door`] edges skip the window check: a closed door sector's
//! ceiling equals its floor, so its window is zero, and clearance *when open*
//! is rule P4's already-covered job. The flood asks only whether the door is
//! unlockable. The step rule still applies to door floors, which is what
//! catches a one-way passage *through* a door.
//!
//! [`EdgeKind::Teleport`] edges skip **both** rules. Neither describes a
//! teleport: `EV_Teleport` calls `P_TeleportMove`, not `P_TryMove`, so no
//! step cap and no crossing window is ever consulted between the pad and the
//! destination — the arriving thing is unlinked and relinked at the marker,
//! taking its floor and ceiling from whatever subsector it lands in. What
//! the destination must clear is checked instead where it belongs, as rule
//! P15 over the marker (`compile::things`, `check::invariants`). These edges
//! are also the graph's only directed ones (see [`Edge`]).
//!
//! [`EdgeKind::Lift`] edges skip both rules too, for a different reason: the
//! platform *moves*. `lift_edges` adds one undirected edge per
//! `(caller, platform)` pair the compiler recorded on
//! [`crate::compile::lifts::LiftOut::callable_from`] — a lift's low room (or
//! its alcove), both neighbors of a barrier, a pedestal's host — because
//! `downWaitUpStay` (`p_plats.c`) brings the platform down to that caller's
//! floor and carries them back up. The step rule would refuse that crossing
//! on the platform's *rest* heights, which is the one position the player
//! never has to climb from; the crossing window is guaranteed instead by the
//! compiler's headroom check on the platform at rest, its tightest position.
//! Only the call is special-cased: the platform node keeps its rest floor, so
//! every other boundary it has stays an ordinary [`EdgeKind::Open`] edge
//! under both rules — level room ↔ platform at equal floors, and the drop
//! back down to the low room as a free descent.
//!
//! # The vacuous-pass gate
//!
//! P7 runs only when the map has a player 1 start and at least one exit;
//! otherwise it passes vacuously. A softlock presupposes a goal, and "this map
//! has no exit" is a spec-conformance finding belonging to the stage that reads
//! the map spec, not to this rule. The gate lives in the builder that derives a
//! [`ReachGraph`] from compiled geometry; it is documented here so that its
//! absence from an exit-less map's violations reads as a decision rather than
//! an oversight.

use std::collections::HashSet;

use crate::compile::Compiled;
use crate::compile::floors::{FloorShape, NamedConstruct, construct_name};
use crate::ir::Ir;
use crate::tables::Tables;

/// Index of a node (sector) in a [`ReachGraph`].
pub type NodeIdx = usize;

/// A key class: a bit position in a [`KeyMask`]. Classes are interned by the
/// keyed-door special the key satisfies, so the card and skull of a colour
/// share one class — `EV_VerticalDoor` (pinned `p_doors.c:371-403`) accepts
/// either: `!p->cards[it_bluecard] && !p->cards[it_blueskull]`.
///
/// **Invariant: a class is always below [`ACTION_BIT_BASE`] (8).** Nothing in
/// the type says so, and a class at or past that base aliases onto a floor
/// action's bit — a locked door that opens because a wall dropped, with no
/// test able to see it. Interning is the one place that can enforce it, and
/// [`graph_from_compiled`] does, asserting the vocabulary yields at most 8
/// distinct lock classes.
pub type KeyClass = u8;

/// A set of key classes held and floor actions fired, one bit each: bits
/// `0..ACTION_BIT_BASE` are key classes ([`KeyClass`]), bits
/// `ACTION_BIT_BASE..16` are floor actions
/// ([`crate::ir::Ir::MAX_FLOOR_ACTIONS`] = 8 of them). The two halves are one
/// word because they are one thing to the search — everything the player has
/// irreversibly acquired on the way to where they stand.
pub type KeyMask = u16;

/// The first floor-action bit: bits below it are [`KeyClass`]es, bits from it
/// up are floor actions.
pub const ACTION_BIT_BASE: u32 = 8;

// The two halves must fit one mask. Raising either constant without widening
// `KeyMask` would shift a floor action's bit off the end — a debug panic in
// `Node::effective_floor`, but in release a silent alias onto key class 0,
// which is a locked door that opens because a wall dropped. A compile error
// is the only place that can be caught for certain.
const _: () = assert!(
    ACTION_BIT_BASE as usize + Ir::MAX_FLOOR_ACTIONS <= KeyMask::BITS as usize,
    "the key classes and the floor actions must fit one KeyMask"
);

/// One sector, reduced to what traversal needs.
#[derive(Debug, Clone)]
pub struct Node {
    /// Floor height in map units.
    pub floor: i32,
    /// Ceiling height in map units.
    pub ceiling: i32,
    /// Key classes collectible here. Entering the node collects them all.
    pub keys: KeyMask,
    /// Floor actions fired by *entering* this node — the room a switch
    /// trigger stands in. Bits at or above [`ACTION_BIT_BASE`], unioned
    /// into the mask on entry exactly as [`Node::keys`] is.
    pub fires: KeyMask,
    /// This node's own floor action, if it has one: `(bit, destination
    /// floor)`. Until `bit` is set the node stands at [`Node::floor`];
    /// afterwards it stands at the destination. See
    /// [`Node::effective_floor`].
    ///
    /// **Invariant: `bit` is below `KeyMask::BITS - ACTION_BIT_BASE` (8).**
    /// The bit is shifted up by [`ACTION_BIT_BASE`], so one at or past that
    /// count shifts off the end of the mask — a debug panic in
    /// [`Node::effective_floor`], but in release `1 << 16` masks to
    /// `1 << 0` and aliases the action onto key class 0.
    /// [`graph_from_compiled`] is the enforcement point, capping the action
    /// list at [`Ir::MAX_FLOOR_ACTIONS`].
    pub action: Option<(u8, i32)>,
}

impl Node {
    /// The floor the player stands on in state `mask`: the destination once
    /// this node's action has fired, the rest floor before.
    ///
    /// Only the floor moves, which is why the crossing window opens up as a
    /// drop wall comes down rather than sliding with it: `T_MoveFloor`
    /// (pinned `p_floor.c:208-234`) calls `T_MovePlane` with
    /// `floorOrCeiling = 0`, and that case writes `sector->floorheight`
    /// alone (`p_floor.c:62-127`) — the ceiling is the `case 1` branch it
    /// never takes. So [`Node::ceiling`] is read unchanged by both
    /// passability rules, in either state.
    #[must_use]
    pub fn effective_floor(&self, mask: KeyMask) -> i32 {
        let Some((bit, dest)) = self.action else {
            return self.floor;
        };
        // The mirror of `passable`'s assert on a key class: an action bit at
        // or past the action half's width shifts off the end of the mask.
        // See [`Node::action`]'s invariant for what that costs in release.
        debug_assert!(
            u32::from(bit) < KeyMask::BITS - ACTION_BIT_BASE,
            "floor action {bit} does not fit {} action bits",
            KeyMask::BITS - ACTION_BIT_BASE
        );
        if mask & (1 << (ACTION_BIT_BASE + u32::from(bit))) != 0 {
            dest
        } else {
            self.floor
        }
    }
}

/// How a shared boundary between two sectors traverses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EdgeKind {
    /// An ordinary passable two-sided boundary.
    Open,
    /// A door face: passable once the lock, if any, is satisfied.
    ///
    /// Door edges skip the crossing-window rule — a closed door sector's
    /// ceiling equals its floor, and clearance when open is rule P4's
    /// already-covered job. The step rule still applies to door floors,
    /// which is what catches a one-way passage *through* a door.
    Door {
        /// The key class that opens this door; `None` for a plain door.
        lock: Option<KeyClass>,
    },
    /// A teleport: crossing `a`'s trigger edge relocates the player to `b`.
    ///
    /// The only directed edge in the graph — `EV_Teleport` (pinned
    /// `p_telept.c`) fires only for a front-side crossing (`if (side == 1)
    /// return 0;`) and relocates rather than moves, so neither the step cap
    /// nor the crossing window applies, and nothing leads back. `check`
    /// expands it in one direction only. A one-shot (W1) line is the same
    /// edge: a walk uses an edge once, which is exactly what W1 permits; the
    /// unmodeled case — returning to reuse it — is recorded in
    /// `KNOWN-GAPS.md` as a P7 limitation.
    Teleport,
    /// A platform a player can call from `a` and ride: the one crossing the
    /// step rule wrongly refuses — low side → platform — is exactly what a
    /// callable lift permits (`p_plats.c`, `downWaitUpStay`: the platform
    /// comes to the caller's floor and carries them up). The platform's node
    /// keeps its rest floor, so its other edges stay ordinary: level room ↔
    /// platform is `Open` at equal floors, platform → low room is a free
    /// descent. Bidirectional, and exempt from both geometric rules — the
    /// crossing window is guaranteed by the compiler's headroom check on the
    /// platform at rest, its tightest position.
    Lift,
}

/// A boundary between two nodes. Undirected — passability is evaluated per
/// crossing direction — except for [`EdgeKind::Teleport`], which `check`
/// expands `a → b` only.
#[derive(Debug, Clone)]
pub struct Edge {
    /// One side.
    pub a: NodeIdx,
    /// The other side.
    pub b: NodeIdx,
    /// How the boundary traverses.
    pub kind: EdgeKind,
    /// Floor actions fired by *crossing* this boundary — a walkover
    /// trigger's line. Bits at or above [`ACTION_BIT_BASE`], unioned into
    /// the mask on arrival, in either direction: the specials the compiler
    /// writes are the W1 forms, which `P_CrossSpecialLine` runs from both
    /// sides (see [`crate::compile::floors`]).
    pub fires: KeyMask,
}

/// The traversal graph the flood searches.
#[derive(Debug)]
pub struct ReachGraph {
    /// One node per sector.
    pub nodes: Vec<Node>,
    /// Every passable two-sided boundary.
    pub edges: Vec<Edge>,
    /// The node holding the player 1 start.
    pub start: NodeIdx,
    /// Nodes from which an exit fires.
    pub goals: Vec<NodeIdx>,
}

/// Engine limits the search applies, passed in so the core stays pure.
#[derive(Debug)]
pub struct Limits {
    /// The player's collision height (`data/engine.toml` `player.height`).
    pub player_height: i32,
    /// The step-up cap (`data/engine.toml` `max_step_height`).
    pub max_step: i32,
}

/// What the flood found. See [`check`].
#[derive(Debug)]
pub struct Findings {
    /// No feasible walk from the start reaches any goal.
    pub unfinishable: bool,
    /// Forward-reachable states that cannot reach any goal — one
    /// representative `(node, keys held)` per node, in discovery order: the
    /// first doomed state the search found there. A node is named when *some*
    /// reachable state of it is doomed, which is precisely the softlock
    /// condition — there exists a way to arrive that cannot be walked out of,
    /// even if another arrival could have been. Doomed states of a node the
    /// report has already named are folded into that one entry, so the list is
    /// a set of rooms to fix rather than an enumeration of key combinations.
    /// Degenerate case: when `unfinishable` is set the backward search has no
    /// goal to seed from, so *every* visited node lands here — the rules layer
    /// filters on exactly that rather than reporting a map that was never
    /// finishable as a map full of softlocks.
    pub stranded: Vec<(NodeIdx, KeyMask)>,
    /// Nodes never visited in any state, ascending.
    pub unreachable: Vec<NodeIdx>,
}

/// Whether one crossing of `kind` from `from` to `to` is possible in state
/// `mask`.
///
/// Both geometric rules measure [`Node::effective_floor`] rather than
/// [`Node::floor`], so a node whose floor action has fired is judged where
/// it now stands: that is the whole of how a dropped wall becomes a doorway
/// and a raised bridge becomes a walk.
///
/// The two geometric rules are `P_TryMove`'s, verified at the pinned commit
/// (`docs/reachability.md`, "Engine facts"):
/// - step up: `tmfloorz - thing->z > 24*FRACUNIT` rejects (`p_map.c:477-479`);
///   descents are free — `MT_PLAYER` has `MF_DROPOFF` (`info.c:1130`), so the
///   dropoff rejection (`p_map.c:481-483`) never applies to the player.
/// - crossing window: `P_LineOpening` (`p_maputl.c:300-329`) gives
///   `min(ceilings) - max(floors)`, and `tmceilingz - tmfloorz <
///   thing->height` rejects (`p_map.c:468-469`).
fn passable(from: &Node, to: &Node, kind: &EdgeKind, mask: KeyMask, limits: &Limits) -> bool {
    let (from_floor, to_floor) = (from.effective_floor(mask), to.effective_floor(mask));
    match kind {
        EdgeKind::Open => {
            to_floor - from_floor <= limits.max_step
                && to.ceiling.min(from.ceiling) - to_floor.max(from_floor) >= limits.player_height
        }
        EdgeKind::Door { lock } => {
            to_floor - from_floor <= limits.max_step
                && lock.is_none_or(|k| {
                    // A class at or past `ACTION_BIT_BASE` aliases onto a
                    // floor action's bit: a debug panic here, but in release
                    // a locked door that opens because a wall dropped
                    // somewhere else, with no test able to see it. Interning
                    // must keep classes under this cap.
                    debug_assert!(
                        u32::from(k) < ACTION_BIT_BASE,
                        "key class {k} does not fit {ACTION_BIT_BASE} key classes"
                    );
                    mask & (1 << k) != 0
                })
        }
        EdgeKind::Teleport | EdgeKind::Lift => true,
    }
}

/// Runs the P7 flood over `(node, keys-held)` states: a forward search from
/// the start, then a backward search from the goals over what the first one
/// found.
///
/// A set-union flood over sectors alone cannot express "holding the key
/// strands you" — the shipped `key_room` defect — so the state carries the
/// mask, and entering a node unions that node's keys and the floor actions
/// it fires in, while crossing an edge unions the actions *it* fires in
/// (masks only grow along a walk).
///
/// The forward search answers [`Findings::unfinishable`] and, by omission,
/// [`Findings::unreachable`]. Stranding needs the second direction: a state is
/// doomed when it is forward-reachable yet no walk from it reaches a goal, so
/// the backward pass seeds every discovered goal state and floods the reverse
/// of the same edges. Reversing the *discovered* state graph rather than the
/// map's edges is what keeps the two passes consistent — an edge only appears
/// reversed if the forward search found it passable under that exact mask, so
/// a door the player could not have opened on the way in cannot be walked back
/// through on the way out.
///
/// # Panics
///
/// If the forward search discovers more than `u32::MAX` states, which the
/// backward pass indexes with a `u32`. The state space is
/// `nodes × 2^bits-in-use` and the mask is 16 bits wide, so even at its
/// widest this needs over 65 thousand sectors — past the `u16` sector index
/// the map format itself can address.
#[must_use]
pub fn check(graph: &ReachGraph, limits: &Limits) -> Findings {
    let n = graph.nodes.len();
    let mut adj: Vec<Vec<(NodeIdx, &EdgeKind, KeyMask)>> = vec![Vec::new(); n];
    for e in &graph.edges {
        adj[e.a].push((e.b, &e.kind, e.fires));
        // A walkover fires from either side, so the fires bits ride both
        // directions; only the traversal of a teleport is one-way.
        if e.kind != EdgeKind::Teleport {
            adj[e.b].push((e.a, &e.kind, e.fires));
        }
    }
    let norm =
        |node: NodeIdx, mask: KeyMask| mask | graph.nodes[node].keys | graph.nodes[node].fires;

    // `seen` and `pos` below are dense over the whole state space, so the
    // mask width they are sized for is derived from the bits *this* graph
    // can ever set rather than fixed at `KeyMask::BITS`. A map with no keys
    // and no floor actions gets one slot per node; an eight-class,
    // eight-action map pays the full 2^16. Sizing unconditionally at the
    // mask's width would multiply both tables by 256 for every action-free
    // map — including the real WADs the verifier floods, where a
    // 1,000-sector map alone would want 262 MB for `pos`.
    let mut used: KeyMask = 0;
    for node in &graph.nodes {
        used |= node.keys | node.fires;
    }
    for e in &graph.edges {
        used |= e.fires;
    }
    let width = KeyMask::BITS - used.leading_zeros();
    // A state index packs (node, mask); every mask a walk can reach is a
    // subset of `used`, so it fits in `width` bits by construction.
    let idx = |node: NodeIdx, mask: KeyMask| (node << width) | mask as usize;

    let mut seen = vec![false; n << width];
    // `order` doubles as the BFS queue: Vec-backed, so traversal (and every
    // report derived from it) is deterministic.
    let mut order: Vec<(NodeIdx, KeyMask)> = Vec::new();
    let start_mask = norm(graph.start, 0);
    seen[idx(graph.start, start_mask)] = true;
    order.push((graph.start, start_mask));
    let mut head = 0;
    while head < order.len() {
        let (at, mask) = order[head];
        head += 1;
        for &(to, kind, fires) in &adj[at] {
            if !passable(&graph.nodes[at], &graph.nodes[to], kind, mask, limits) {
                continue;
            }
            let to_mask = norm(to, mask | fires);
            if !seen[idx(to, to_mask)] {
                seen[idx(to, to_mask)] = true;
                order.push((to, to_mask));
            }
        }
    }

    let mut is_goal = vec![false; n];
    for &g in &graph.goals {
        is_goal[g] = true;
    }
    let unfinishable = !order.iter().any(|&(node, _)| is_goal[node]);

    // Backward pass over the *discovered* states: which of them can still
    // reach a goal? Position lookup packs into the same (node, mask) space.
    let mut pos = vec![u32::MAX; n << width];
    for (i, &(node, mask)) in order.iter().enumerate() {
        pos[idx(node, mask)] = u32::try_from(i).expect("state count fits u32");
    }
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); order.len()];
    for (i, &(node, mask)) in order.iter().enumerate() {
        for &(to, kind, fires) in &adj[node] {
            if !passable(&graph.nodes[node], &graph.nodes[to], kind, mask, limits) {
                continue;
            }
            // The same successor expression the forward search used, or the
            // two passes would disagree about which state an edge leads to.
            let j = pos[idx(to, norm(to, mask | fires))];
            debug_assert_ne!(j, u32::MAX, "forward search discovered every successor");
            preds[j as usize].push(u32::try_from(i).expect("state count fits u32"));
        }
    }
    let mut can_finish = vec![false; order.len()];
    let mut queue: Vec<u32> = (0..order.len())
        .filter(|&i| is_goal[order[i].0])
        .map(|i| u32::try_from(i).expect("state count fits u32"))
        .collect();
    for &i in &queue {
        can_finish[i as usize] = true;
    }
    let mut back_head = 0;
    while back_head < queue.len() {
        let i = queue[back_head] as usize;
        back_head += 1;
        for &p in &preds[i] {
            if !can_finish[p as usize] {
                can_finish[p as usize] = true;
                queue.push(p);
            }
        }
    }

    // One representative doomed state per node, in discovery order.
    let mut node_flagged = vec![false; n];
    let mut stranded = Vec::new();
    for (i, &(node, mask)) in order.iter().enumerate() {
        if !can_finish[i] && !node_flagged[node] {
            node_flagged[node] = true;
            stranded.push((node, mask));
        }
    }

    let mut visited = vec![false; n];
    for &(node, _) in &order {
        visited[node] = true;
    }
    let unreachable: Vec<NodeIdx> = (0..n).filter(|&i| !visited[i]).collect();

    Findings {
        unfinishable,
        stranded,
        unreachable,
    }
}

/// A [`ReachGraph`] plus the naming data reports need.
#[derive(Debug)]
pub struct BuiltGraph {
    /// The traversal graph.
    pub graph: ReachGraph,
    /// For each key class, the key kinds that satisfy it, sorted — e.g.
    /// class 0 -> `["blue_card", "blue_skull"]`. Used to word violations.
    pub class_names: Vec<Vec<String>>,
    /// One name per floor action — `"drop wall a <-> b"`, `"reveal pen"`,
    /// `"bridge a <-> b"` — indexed by action bit, so `action_names[i]`
    /// describes mask bit `ACTION_BIT_BASE + i` and, position for position,
    /// [`Compiled::floors`]`[i]`. Used to word violations.
    pub action_names: Vec<String>,
}

/// Derives the traversal graph from what was actually emitted.
///
/// Geometry comes from [`MapData`](crate::compile::MapData) — sectors as
/// nodes, two-sided non-blocking linedefs as edges, plus one directed
/// [`EdgeKind::Teleport`] edge per distinct `(host sector, destination
/// sector)` pair a player-usable teleport line reaches, where the
/// destination is whichever sector [`crate::compile::teleports`] tagged —
/// never from authored intent, so the graph cannot drift from the map. Keys
/// and the start use the room-index-equals-sector-index invariant
/// [`crate::compile::things`] documents and verifies.
///
/// This is also where [`check`]'s indexing preconditions are established.
/// `check` indexes `nodes` by `start`, by every `goals` entry, and by each
/// edge's `a`/`b`, and panics if any is out of range; here every one of those
/// is a sector index read back out of the emitted `MapData` — an edge's ends
/// come from `sidedefs[..].sector`, a goal from the sidedef of an emitted exit
/// line, and the start from a room index, which that same invariant makes a
/// valid sector index — so they are all in range by construction, and a
/// hand-built graph is the only way to violate them.
///
/// The interning below is likewise the enforcement point for [`KeyClass`]'s
/// invariant that every class index is below [`ACTION_BIT_BASE`] — the class
/// *count* may be exactly [`ACTION_BIT_BASE`], which is what the assert
/// admits.
///
/// Floor actions come from [`Compiled::floors`] and [`Compiled::triggers`],
/// again as emitted rather than as authored: action `i` is mask bit
/// `ACTION_BIT_BASE + i` and moves its target sector's floor to that
/// action's destination, a switch trigger sets every bit it drives on the
/// sector it is used from, and a walkover trigger sets them on the edges
/// built from the lines that actually carry its tag and its walkover
/// special — which is both thresholds of a bridge it names, since
/// [`crate::compile::floors`] writes the special on each.
///
/// Returns `None` when the map has no player 1 start or no exit line —
/// the vacuous-pass gate: P7 presupposes a goal, and "this map has no
/// exit" is a spec-conformance finding for the stage that reads the
/// map-spec, not a softlock.
///
/// The start is looked for among the **rooms'** things only, and that is
/// exhaustive rather than a narrowing: island cargo cannot hold a player 1
/// start, since `compile::things::place_island_things` refuses one on a
/// reveal ([`CompileError::StartOnReveal`](crate::compile::CompileError::StartOnReveal))
/// and on a pedestal
/// ([`CompileError::PlayerStartOnPedestal`](crate::compile::CompileError::PlayerStartOnPedestal)),
/// the second precisely so this search cannot come up empty on a map that
/// does have one. `None` here therefore means the map truly has no start,
/// not that this pass failed to find one.
///
/// # Panics
///
/// If the vocabulary lists more than [`ACTION_BIT_BASE`] distinct keyed-door
/// specials, or the map carries more than [`Ir::MAX_FLOOR_ACTIONS`] floor
/// actions — either would need a wider [`KeyMask`]. `Ir::from_json` refuses
/// the second ([`crate::ir::IrError::TooManyFloorActions`]) before this pass
/// can run.
///
/// Also if a pedestal has no emitted platform or a reveal no emitted floor
/// action, which [`crate::compile::lifts`] and [`crate::compile::floors`]
/// establish one-for-one before this pass runs — the same invariant
/// `compile::things`'s own island placement relies on.
#[must_use]
pub fn graph_from_compiled(ir: &Ir, tables: &Tables, out: &Compiled) -> Option<BuiltGraph> {
    // The start: the first room placing a `player1_start` (the IR vocabulary
    // name; resolved to engine thing 1 by the tables at emission). Rooms are
    // the only place one can be — see this function's own doc comment on the
    // `None` path.
    let start = ir
        .rooms
        .iter()
        .position(|r| r.things.iter().any(|t| t.kind == "player1_start"))?;

    let exit_specials = [
        tables.exit_switch_special(),
        tables.secret_exit_switch_special(),
        tables.exit_walkover_special(),
        tables.secret_exit_walkover_special(),
    ];
    let mut goals = Vec::new();
    for line in &out.data.linedefs {
        if !exit_specials.contains(&line.special) {
            continue;
        }
        match line.back {
            // A walkover exit's threshold fronts the host room and backs the
            // carved recess; reaching the recess means the line was crossed.
            Some(back) => goals.push(out.data.sidedefs[back].sector),
            // A switch exit's one-sided line fronts its host room, and
            // `P_UseSpecialLine` fires from the front side the player faces.
            None => goals.push(out.data.sidedefs[line.front].sector),
        }
    }
    if goals.is_empty() {
        return None;
    }
    goals.sort_unstable();
    goals.dedup();

    // Intern key classes by keyed-door special: the card and skull of a
    // colour share a special (`EV_VerticalDoor`, pinned p_doors.c:371-403),
    // so they must share a class.
    let kinds = tables.locked_door_kinds();
    let mut specials: Vec<u16> = kinds.iter().map(|&(_, s)| s).collect();
    specials.sort_unstable();
    specials.dedup();
    assert!(
        specials.len() <= ACTION_BIT_BASE as usize,
        "a vocabulary with more than {ACTION_BIT_BASE} lock classes needs a wider KeyMask"
    );
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

    let mut nodes: Vec<Node> = out
        .data
        .sectors
        .iter()
        .map(|s| Node {
            floor: s.floor,
            ceiling: s.ceiling,
            keys: 0,
            fires: 0,
            action: None,
        })
        .collect();
    // A room's own things sit in the room's sector, and `emit_sectors`
    // pushes one sector per room in `ir.rooms` order, so the room index is
    // the node index.
    for (i, room) in ir.rooms.iter().enumerate() {
        for thing in &room.things {
            add_key_bit(tables, &specials, &thing.kind, &mut nodes[i].keys);
        }
    }
    // Island cargo is picked up by standing on the *island*, not in its host
    // room: a key on a pedestal's top or sealed in a reveal's cell belongs
    // to that construct's own node, so the flood only grants it once the
    // platform has been called down or the reveal has fired. Putting it on
    // the host would hand the player a key they cannot yet reach; leaving it
    // off entirely (what this pass did before) strands them beside a key
    // they can, and the map is refused. The verifier reads the same
    // placement from geometry — `check::flood::build_nodes` keys a node by
    // the sector each emitted thing resolves to — so this is the compile
    // side agreeing with it rather than a second convention.
    for (pi, pedestal) in ir.pedestals.iter().enumerate() {
        let lift = out
            .lifts
            .iter()
            .find(|l| l.pedestal == Some(pi))
            .expect("emit_lifts emits one platform per pedestal");
        for thing in &pedestal.things {
            add_key_bit(tables, &specials, &thing.kind, &mut nodes[lift.sector].keys);
        }
    }
    for (ri, reveal) in ir.reveals.iter().enumerate() {
        let action = out
            .floors
            .iter()
            .find(|f| f.reveal == Some(ri))
            .expect("emit_floors emits one action per reveal");
        for thing in &reveal.things {
            add_key_bit(
                tables,
                &specials,
                &thing.kind,
                &mut nodes[action.sector].keys,
            );
        }
    }

    let (mut edges, edge_of_line) = boundary_edges(tables, out, &specials);
    wire_floor_actions(tables, out, &mut nodes, &mut edges, &edge_of_line);
    let action_names = floor_action_names(ir, out);

    edges.extend(teleport_edges(tables, out));
    edges.extend(lift_edges(out));

    Some(BuiltGraph {
        graph: ReachGraph {
            nodes,
            edges,
            start,
            goals,
        },
        class_names,
        action_names,
    })
}

/// Sets `kind`'s key-class bit in `keys`, if `kind` is a key thing at all.
///
/// One helper rather than three copies of the same `locked_door_special` →
/// [`class_of`] → shift chain, because the three placements a key can have
/// — a room's floor, a pedestal's top, a reveal's cell — differ only in
/// which node's mask they set.
fn add_key_bit(tables: &Tables, specials: &[u16], kind: &str, keys: &mut KeyMask) {
    if let Some(special) = tables.locked_door_special(kind)
        && let Some(class) = class_of(specials, special)
    {
        *keys |= 1 << class;
    }
}

/// The [`KeyClass`] `special` interns to under `specials` (as built by
/// [`graph_from_compiled`]), if any.
fn class_of(specials: &[u16], special: u16) -> Option<KeyClass> {
    specials
        .iter()
        .position(|&s| s == special)
        .map(|i| KeyClass::try_from(i).expect("at most ACTION_BIT_BASE classes"))
}

/// One [`EdgeKind::Open`] or [`EdgeKind::Door`] edge per passable two-sided
/// linedef, alongside the linedef-index → edge-index lookup a walkover
/// trigger needs to put its bits on the crossing that fires it.
fn boundary_edges(
    tables: &Tables,
    out: &Compiled,
    specials: &[u16],
) -> (Vec<Edge>, Vec<Option<usize>>) {
    let plain_door = tables.door_special();
    let mut edges = Vec::new();
    let mut edge_of_line: Vec<Option<usize>> = vec![None; out.data.linedefs.len()];
    for (li, line) in out.data.linedefs.iter().enumerate() {
        let Some(back) = line.back else { continue };
        // ML_BLOCKING stops a non-missile even on a two-sided line:
        // PIT_CheckLine rejects it (pinned p_map.c:214-217).
        if line.blocking {
            continue;
        }
        let kind = if line.special == plain_door {
            EdgeKind::Door { lock: None }
        } else if let Some(class) = class_of(specials, line.special) {
            EdgeKind::Door { lock: Some(class) }
        } else {
            EdgeKind::Open
        };
        edge_of_line[li] = Some(edges.len());
        edges.push(Edge {
            a: out.data.sidedefs[line.front].sector,
            b: out.data.sidedefs[back].sector,
            kind,
            fires: 0,
        });
    }
    (edges, edge_of_line)
}

/// Puts the emitted floor actions into the graph: action `a` — the entry at
/// `out.floors[a]` — is mask bit `ACTION_BIT_BASE + a`, its target sector's
/// [`Node::action`] moves that sector's floor to the action's destination,
/// and every trigger driving it sets the bit where the player fires it. A
/// switch sets it on the sector it is used from — its
/// [`TriggerOut::activator`](crate::compile::floors::TriggerOut::activator)
/// — and a walkover sets it on the edges built from the lines that carry
/// the trigger's tag and its walkover special.
///
/// Those lines are read back off the emitted map rather than taken from
/// [`crate::compile::floors::TriggerOut::line`], which is only the *first*
/// one written: a walkover naming a bridge carries its special on both of
/// that bridge's thresholds, and either crossing fires it.
///
/// # Panics
/// Panics if the map carries more than [`Ir::MAX_FLOOR_ACTIONS`] floor
/// actions, whose bits would not fit above the key classes.
/// `Ir::from_json` refuses that before this pass can run.
fn wire_floor_actions(
    tables: &Tables,
    out: &Compiled,
    nodes: &mut [Node],
    edges: &mut [Edge],
    edge_of_line: &[Option<usize>],
) {
    assert!(
        out.floors.len() <= Ir::MAX_FLOOR_ACTIONS,
        "a map with more than {} floor actions needs a wider KeyMask",
        Ir::MAX_FLOOR_ACTIONS
    );
    for (a, f) in out.floors.iter().enumerate() {
        let bit = u8::try_from(a).expect("at most Ir::MAX_FLOOR_ACTIONS actions");
        // A node carries one action, so two of them on one sector would
        // leave only the last with no diagnostic. The compiler cannot emit
        // that — one construct per sector, and P30 refuses an action chained
        // onto another moving sector — so this states the precondition
        // rather than handling it.
        debug_assert!(
            nodes[f.sector].action.is_none(),
            "sector {} is the target of two floor actions; a node carries one",
            f.sector
        );
        nodes[f.sector].action = Some((bit, f.dest));
    }
    for (i, t) in out.triggers.iter().enumerate() {
        // The bits this trigger sets: every action it drives.
        let bits: KeyMask = out
            .floors
            .iter()
            .enumerate()
            .filter(|(_, f)| f.trigger == i)
            .map(|(a, _)| 1 << (ACTION_BIT_BASE + u32::try_from(a).expect("at most 8 actions")))
            .fold(0, |acc: KeyMask, bit| acc | bit);
        if t.walkover {
            let special = tables.floor_special(t.family, false);
            for (li, line) in out.data.linedefs.iter().enumerate() {
                if line.special == special
                    && line.tag == t.tag
                    && let Some(e) = edge_of_line[li]
                {
                    edges[e].fires |= bits;
                }
            }
        } else {
            // A switch fires from the room it is used in: `P_UseSpecialLine`
            // runs on the front side the player faces, which is that room.
            nodes[t.activator].fires |= bits;
        }
    }
}

/// One name per emitted floor action, in [`Compiled::floors`] order — the
/// wording [`BuiltGraph::action_names`] carries into violations.
///
/// The words themselves come from [`construct_name`], which the tag manifest
/// also uses, so a violation and the manifest row for the same construct
/// cannot drift apart.
///
/// # Panics
/// Panics if a drop wall or bridge names no portal, or a closet or pedestal
/// no reveal, which [`crate::compile::floors`] sets on every action it
/// emits.
fn floor_action_names(ir: &Ir, out: &Compiled) -> Vec<String> {
    out.floors
        .iter()
        .map(|f| match f.shape {
            FloorShape::DropWall | FloorShape::Bridge => construct_name(NamedConstruct::Portal(
                &ir.portals[f.portal.expect("a drop wall or bridge names its portal")],
            )),
            FloorShape::Closet | FloorShape::Pedestal => construct_name(NamedConstruct::Reveal(
                &ir.reveals[f.reveal.expect("a closet or pedestal names its reveal")],
            )),
        })
        .collect()
}

/// Directed [`EdgeKind::Teleport`] edges for every player-usable teleport
/// line: player-usable specials only (a monster-only pad's trigger carries a
/// different special the player vocabulary never matches), tagged to exactly
/// the destination sector [`crate::compile::teleports`] tags. The edge runs
/// host → destination; `check` never expands it back.
///
/// An island pad carries its special on all four boundary linedefs (any
/// approach fires it), so the same host → destination pair recurs; a `seen`
/// set keeps the result to one edge per distinct pair.
///
/// # Panics
/// Panics if a teleport line's tag resolves to no sector. Unreachable on
/// compiled output: [`crate::compile::teleports`] writes a line's tag and
/// the destination sector's tag from the same [`crate::compile::tags`]
/// allocation, in one pass, so a tagged trigger line always has its sector.
fn teleport_edges(tables: &Tables, out: &Compiled) -> Vec<Edge> {
    let player_teleports = tables.player_teleport_specials();
    let mut seen: HashSet<(NodeIdx, NodeIdx)> = HashSet::new();
    let mut edges = Vec::new();
    for line in &out.data.linedefs {
        if !player_teleports.contains(&line.special) || line.tag == 0 {
            continue;
        }
        let dest = out
            .data
            .sectors
            .iter()
            .position(|s| s.tag == line.tag)
            .expect("the compiler tags every destination");
        let from = out.data.sidedefs[line.front].sector;
        if seen.insert((from, dest)) {
            edges.push(Edge {
                a: from,
                b: dest,
                kind: EdgeKind::Teleport,
                fires: 0,
            });
        }
    }
    edges
}

/// One [`EdgeKind::Lift`] edge per distinct `(caller, platform)` pair the
/// compiler recorded on [`Compiled::lifts`].
fn lift_edges(out: &Compiled) -> Vec<Edge> {
    let mut seen: HashSet<(NodeIdx, NodeIdx)> = HashSet::new();
    let mut edges = Vec::new();
    for lift in &out.lifts {
        for &from in &lift.callable_from {
            if seen.insert((from, lift.sector)) {
                edges.push(Edge {
                    a: from,
                    b: lift.sector,
                    kind: EdgeKind::Lift,
                    fires: 0,
                });
            }
        }
    }
    edges
}

/// A human-readable name for a node: a room's own id, or a compiler-made
/// sector described by the nearest rooms around it, found by breadth-first
/// search over plain adjacency (passability is irrelevant to naming). The
/// search treats every edge, including [`EdgeKind::Teleport`], as symmetric —
/// naming a node needs only some path to a room, not a walkable one.
#[must_use]
pub fn node_label(node: NodeIdx, ir: &Ir, graph: &ReachGraph) -> String {
    if node < ir.rooms.len() {
        return format!("room `{}`", ir.rooms[node].id);
    }
    let mut adj: Vec<Vec<NodeIdx>> = vec![Vec::new(); graph.nodes.len()];
    for e in &graph.edges {
        adj[e.a].push(e.b);
        adj[e.b].push(e.a);
    }
    let mut seen = vec![false; graph.nodes.len()];
    seen[node] = true;
    let mut frontier = vec![node];
    while !frontier.is_empty() {
        let mut rooms: Vec<&str> = frontier
            .iter()
            .filter(|&&n| n < ir.rooms.len())
            .map(|&n| ir.rooms[n].id.as_str())
            .collect();
        if !rooms.is_empty() {
            rooms.sort_unstable();
            rooms.dedup();
            return match rooms.as_slice() {
                [a] => format!("the recess off `{a}`"),
                [a, b, ..] => format!("the passage between `{a}` and `{b}`"),
                [] => unreachable!("guarded by is_empty above"),
            };
        }
        let mut next = Vec::new();
        for &n in &frontier {
            for &to in &adj[n] {
                if !seen[to] {
                    seen[to] = true;
                    next.push(to);
                }
            }
        }
        frontier = next;
    }
    format!("sector {node}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::compile_reporting;

    /// Engine limits for hand-built graphs: the real player height and step
    /// cap, restated here as literals because these tests exercise the
    /// algorithm, not the tables. Builder tests (rules.rs) use `Tables`.
    const LIMITS: Limits = Limits {
        player_height: 56,
        max_step: 24,
    };

    fn node(floor: i32, ceiling: i32, keys: KeyMask) -> Node {
        Node {
            floor,
            ceiling,
            keys,
            fires: 0,
            action: None,
        }
    }
    fn open(a: NodeIdx, b: NodeIdx) -> Edge {
        Edge {
            a,
            b,
            kind: EdgeKind::Open,
            fires: 0,
        }
    }
    fn door(a: NodeIdx, b: NodeIdx, lock: Option<KeyClass>) -> Edge {
        Edge {
            a,
            b,
            kind: EdgeKind::Door { lock },
            fires: 0,
        }
    }
    /// A node whose own floor moves: it rests at `floor` and stands at
    /// `dest` once floor action `bit` has fired.
    fn action_node(floor: i32, ceiling: i32, bit: u8, dest: i32) -> Node {
        Node {
            floor,
            ceiling,
            keys: 0,
            fires: 0,
            action: Some((bit, dest)),
        }
    }
    /// A node that fires floor action `bit` on entry — a switch's room.
    fn firing_node(floor: i32, ceiling: i32, bit: u8) -> Node {
        Node {
            floor,
            ceiling,
            keys: 0,
            fires: 1 << (ACTION_BIT_BASE + u32::from(bit)),
            action: None,
        }
    }
    fn graph(
        nodes: Vec<Node>,
        edges: Vec<Edge>,
        start: NodeIdx,
        goals: Vec<NodeIdx>,
    ) -> ReachGraph {
        ReachGraph {
            nodes,
            edges,
            start,
            goals,
        }
    }

    #[test]
    fn a_step_up_at_the_limit_passes_and_one_over_is_blocked() {
        let ok = graph(
            vec![node(0, 128, 0), node(24, 152, 0)],
            vec![open(0, 1)],
            0,
            vec![1],
        );
        assert!(!check(&ok, &LIMITS).unfinishable, "24 is exactly one step");
        let blocked = graph(
            vec![node(0, 128, 0), node(25, 153, 0)],
            vec![open(0, 1)],
            0,
            vec![1],
        );
        assert!(check(&blocked, &LIMITS).unfinishable, "25 exceeds the cap");
    }

    #[test]
    fn a_drop_of_any_size_passes_downhill_and_blocks_uphill() {
        let nodes = vec![node(0, 128, 0), node(-300, 128, 0)];
        let down = graph(nodes.clone(), vec![open(0, 1)], 0, vec![1]);
        assert!(
            !check(&down, &LIMITS).unfinishable,
            "falling is unrestricted"
        );
        let up = graph(nodes, vec![open(0, 1)], 1, vec![0]);
        assert!(check(&up, &LIMITS).unfinishable, "climbing 300 is not");
    }

    #[test]
    fn a_crossing_window_below_player_height_is_blocked() {
        // A DOWNWARD crossing, so the step rule cannot mask the window rule:
        // from floor 80 down to floor 0, window = min(128, 300) - 80 = 48 < 56.
        let tight = graph(
            vec![node(80, 300, 0), node(0, 128, 0)],
            vec![open(0, 1)],
            0,
            vec![1],
        );
        assert!(check(&tight, &LIMITS).unfinishable, "48-unit window blocks");
        // Raising the low side's ceiling to 136 widens the window to exactly
        // the player's height.
        let fits = graph(
            vec![node(80, 300, 0), node(0, 136, 0)],
            vec![open(0, 1)],
            0,
            vec![1],
        );
        assert!(!check(&fits, &LIMITS).unfinishable, "56-unit window admits");
    }

    #[test]
    fn a_locked_door_needs_its_key_and_an_unlocked_one_does_not() {
        let nodes = vec![node(0, 128, 0), node(0, 0, 0), node(0, 128, 0)];
        let edges = vec![door(0, 1, Some(0)), door(1, 2, Some(0))];
        let locked_out = graph(nodes.clone(), edges.clone(), 0, vec![2]);
        assert!(check(&locked_out, &LIMITS).unfinishable, "no key, no entry");
        let unlocked = graph(nodes, vec![door(0, 1, None), door(1, 2, None)], 0, vec![2]);
        assert!(
            !check(&unlocked, &LIMITS).unfinishable,
            "a plain door opens"
        );
    }

    #[test]
    fn a_key_collected_elsewhere_opens_the_door_on_the_way_back() {
        // start(0) - open - key room(1); start - door(lock 0) - goal(2).
        // The only walk is start -> key room -> start -> goal: the search
        // must revisit `start` with a bigger mask than it first arrived with.
        let g = graph(
            vec![
                node(0, 128, 0),
                node(0, 128, 0b1),
                node(0, 0, 0),
                node(0, 128, 0),
            ],
            vec![open(0, 1), door(0, 2, Some(0)), door(2, 3, Some(0))],
            0,
            vec![3],
        );
        assert!(!check(&g, &LIMITS).unfinishable);
    }

    #[test]
    fn a_door_edge_skips_the_window_check_but_an_open_edge_does_not() {
        // A closed door sector's ceiling equals its floor, so its window is
        // zero; the flood must not reject it (clearance is P4's job).
        let nodes = vec![node(0, 128, 0), node(0, 0, 0), node(0, 128, 0)];
        let through_door = graph(
            nodes.clone(),
            vec![door(0, 1, None), door(1, 2, None)],
            0,
            vec![2],
        );
        assert!(!check(&through_door, &LIMITS).unfinishable);
        let through_open = graph(nodes, vec![open(0, 1), open(1, 2)], 0, vec![2]);
        assert!(
            check(&through_open, &LIMITS).unfinishable,
            "zero window blocks an open edge"
        );
    }

    #[test]
    fn a_lift_edge_climbs_more_than_a_step_where_an_open_edge_refuses() {
        let limits = Limits {
            player_height: 56,
            max_step: 24,
        };
        let low = node(0, 128, 0);
        let plat = node(128, 256, 0);
        assert!(
            !passable(&low, &plat, &EdgeKind::Open, 0, &limits),
            "128 up is not a step"
        );
        assert!(
            passable(&low, &plat, &EdgeKind::Lift, 0, &limits),
            "a callable lift carries the player up"
        );
        assert!(
            passable(&plat, &low, &EdgeKind::Lift, 0, &limits),
            "and descent is free either way"
        );
    }

    #[test]
    fn a_wall_that_lowers_once_its_switch_room_is_entered_opens_the_way() {
        // start(0) — wall(floor 192 = ceiling 192, lowers to 0, bit 0) —
        // exit(0); the start room fires bit 0 on entry.
        let nodes = vec![
            firing_node(0, 192, 0),
            action_node(192, 192, 0, 0),
            node(0, 192, 0),
        ];
        let g = graph(nodes.clone(), vec![open(0, 1), open(1, 2)], 0, vec![2]);
        assert!(
            !check(&g, &LIMITS).unfinishable,
            "the wall drops on entering the start room"
        );
        // Without the firing node the wall is a wall.
        let mut sealed = nodes;
        sealed[0].fires = 0;
        let g = graph(sealed, vec![open(0, 1), open(1, 2)], 0, vec![2]);
        assert!(check(&g, &LIMITS).unfinishable);
    }

    #[test]
    fn a_walkover_fires_on_crossing_its_edge_in_either_direction() {
        // start — hall (the crossing fires bit 0) — wall(lowers) — exit
        let nodes = vec![
            node(0, 192, 0),
            node(0, 192, 0),
            action_node(192, 192, 0, 0),
            node(0, 192, 0),
        ];
        let mut e1 = open(0, 1);
        e1.fires = 1 << ACTION_BIT_BASE;
        let g = graph(nodes, vec![e1, open(1, 2), open(2, 3)], 0, vec![3]);
        let f = check(&g, &LIMITS);
        assert!(!f.unfinishable);
        assert!(f.stranded.is_empty());

        // The same edge crossed the other way, and *only* the other way.
        // `check` expands an undirected edge as `a -> b` and `b -> a`
        // separately, so pinning the second expansion needs a fixture that
        // cannot fall back on the first: node 0 sits 100 below node 1, so
        // the crossing is a free descent one way and an impossible climb
        // back, and the only firing traversal is 1 -> 0. (A fixture whose
        // edge is crossable both ways proves nothing here — the walk simply
        // re-crosses it in the working direction.)
        //
        // This is the direction real geometry takes: a bridge walkover's
        // far threshold has the pit as its front sector, so entering from
        // the far room is a b -> a crossing.
        //
        // nodes: 0 landing, 100 below the start; 1 start; 2 a sealed wall
        // that lowers to the landing's own floor; 3 the exit beyond it.
        let nodes = vec![
            node(-100, 200, 0),
            node(0, 200, 0),
            action_node(92, 92, 0, -100),
            node(-100, 200, 0),
        ];
        let mut back = open(0, 1);
        back.fires = 1 << ACTION_BIT_BASE;
        let g = graph(nodes, vec![back, open(0, 2), open(2, 3)], 1, vec![3]);
        let f = check(&g, &LIMITS);
        assert!(
            !f.unfinishable,
            "the drop from 1 into 0 is a b -> a crossing, and it fires: {f:?}"
        );
        assert!(f.stranded.is_empty(), "{f:?}");
    }

    #[test]
    fn a_door_onto_a_dropped_wall_is_judged_at_the_floor_the_action_left() {
        // A door edge skips the crossing window but keeps the step rule, so
        // it has to read the effective floor too. Here the door's far side
        // is a wall sector resting 192 up that lowers to 0: a 192 step
        // before the action fires, level after. Nothing else in this file
        // puts an action behind a `Door` edge, so the Door arm's own call
        // is otherwise unexercised.
        let nodes = vec![
            firing_node(0, 192, 0),
            action_node(192, 192, 0, 0),
            node(0, 192, 0),
        ];
        let edges = vec![door(0, 1, None), door(1, 2, None)];
        let g = graph(nodes.clone(), edges.clone(), 0, vec![2]);
        assert!(
            !check(&g, &LIMITS).unfinishable,
            "the wall came down, so the step through the door is 0"
        );
        let mut sealed = nodes;
        sealed[0].fires = 0;
        let g = graph(sealed, edges, 0, vec![2]);
        assert!(
            check(&g, &LIMITS).unfinishable,
            "192 is not a step, door or no door"
        );
    }

    #[test]
    fn a_bridge_pit_entered_before_its_trigger_is_a_stranded_state() {
        // start(0) — pit(-96, rises to 0 on bit 0) — far(0, fires bit 0 on
        // entry) — exit. Dropping into the pit first: bit unset, pit->far is
        // a 96 climb, so nothing ever reaches the far room to fire it.
        let nodes = vec![
            node(0, 192, 0),
            action_node(-96, 192, 0, 0),
            firing_node(0, 192, 0),
            node(0, 192, 0),
        ];
        let g = graph(nodes, vec![open(0, 1), open(1, 2), open(2, 3)], 0, vec![3]);
        let f = check(&g, &LIMITS);
        assert!(
            f.unfinishable,
            "the only way forward is through the pit, which never rises"
        );
        // Put the firing IN the pit (a walkover across the pit floor):
        let nodes = vec![
            node(0, 192, 0),
            Node {
                floor: -96,
                ceiling: 192,
                keys: 0,
                fires: 1 << ACTION_BIT_BASE,
                action: Some((0, 0)),
            },
            node(0, 192, 0),
            node(0, 192, 0),
        ];
        let g = graph(nodes, vec![open(0, 1), open(1, 2), open(2, 3)], 0, vec![3]);
        let f = check(&g, &LIMITS);
        assert!(!f.unfinishable && f.stranded.is_empty(), "{f:?}");
    }

    #[test]
    fn keys_and_actions_share_the_mask_without_aliasing() {
        // Key class 7 is the top key bit and action 0 is the first action
        // bit — adjacent in one mask. The wall between the start and the
        // door lowers on action 0; the door beyond it is locked to class 7.
        // If either half aliased onto the other, one of the two negative
        // cases below would read as finishable.
        let wall = action_node(192, 192, 0, 0);
        let build = |keys: KeyMask, fires: KeyMask| {
            let mut start = node(0, 192, keys);
            start.fires = fires;
            graph(
                vec![start, wall.clone(), node(0, 192, 0)],
                vec![open(0, 1), door(1, 2, Some(7))],
                0,
                vec![2],
            )
        };
        assert!(
            !check(&build(1 << 7, 1 << ACTION_BIT_BASE), &LIMITS).unfinishable,
            "the action drops the wall and the key opens the door"
        );
        assert!(
            check(&build(1 << 7, 0), &LIMITS).unfinishable,
            "holding the top key class does not fire action 0"
        );
        assert!(
            check(&build(0, 1 << ACTION_BIT_BASE), &LIMITS).unfinishable,
            "firing action 0 does not unlock a class-7 door"
        );
    }

    #[test]
    fn the_start_being_a_goal_is_trivially_finishable() {
        let g = graph(vec![node(0, 128, 0)], vec![], 0, vec![0]);
        assert!(!check(&g, &LIMITS).unfinishable);
    }

    #[test]
    fn holding_one_key_class_does_not_open_a_different_class() {
        // The start hands out class 0; both doors want class 1. Every other
        // key fixture here uses class 0 against mask 0b1, where "holds any
        // key" and "holds the right key" are indistinguishable.
        let nodes = vec![node(0, 128, 0b1), node(0, 0, 0), node(0, 128, 0)];
        let wrong_key = graph(
            nodes,
            vec![door(0, 1, Some(1)), door(1, 2, Some(1))],
            0,
            vec![2],
        );
        assert!(
            check(&wrong_key, &LIMITS).unfinishable,
            "class 0 is not class 1"
        );
        let right_key = graph(
            vec![node(0, 128, 0b10), node(0, 0, 0), node(0, 128, 0)],
            vec![door(0, 1, Some(1)), door(1, 2, Some(1))],
            0,
            vec![2],
        );
        assert!(
            !check(&right_key, &LIMITS).unfinishable,
            "class 1 opens its own door"
        );
    }

    #[test]
    fn a_pit_you_can_fall_into_but_not_leave_strands() {
        // goal is the start room itself; the pit is a 100-unit drop.
        let g = graph(
            vec![node(0, 128, 0), node(-100, 128, 0)],
            vec![open(0, 1)],
            0,
            vec![0],
        );
        let f = check(&g, &LIMITS);
        assert!(!f.unfinishable, "the exit is right there");
        assert_eq!(f.stranded, vec![(1, 0)], "but the pit is a softlock");
        assert!(f.unreachable.is_empty());
    }

    #[test]
    fn stranding_reports_the_keys_held() {
        // The pit holds key class 0 — the shipped key_room shape reduced to
        // two nodes. The stranded state must name the key, because "you can
        // reach the key" is exactly what made the set-union flood blind.
        let g = graph(
            vec![node(0, 128, 0), node(-100, 128, 0b1)],
            vec![open(0, 1)],
            0,
            vec![0],
        );
        let f = check(&g, &LIMITS);
        assert_eq!(f.stranded, vec![(1, 0b1)]);
    }

    #[test]
    fn an_isolated_node_is_unreachable() {
        let g = graph(
            vec![node(0, 128, 0), node(0, 128, 0), node(0, 128, 0)],
            vec![open(0, 1)],
            0,
            vec![1],
        );
        let f = check(&g, &LIMITS);
        assert!(!f.unfinishable);
        assert!(f.stranded.is_empty());
        assert_eq!(f.unreachable, vec![2]);
    }

    #[test]
    fn a_clean_map_reports_nothing() {
        let g = graph(
            vec![node(0, 128, 0), node(16, 144, 0)],
            vec![open(0, 1)],
            0,
            vec![1],
        );
        let f = check(&g, &LIMITS);
        assert!(!f.unfinishable);
        assert!(f.stranded.is_empty());
        assert!(f.unreachable.is_empty());
    }

    #[test]
    fn when_no_goal_is_reachable_every_visited_state_is_doomed() {
        // The core reports facts; presentation-level filtering of this
        // trivial case is rules.rs's job, tested there.
        let g = graph(
            vec![node(0, 128, 0), node(-100, 128, 0), node(0, 128, 0)],
            vec![open(0, 1)],
            0,
            vec![2], // the goal exists but nothing connects to it
        );
        let f = check(&g, &LIMITS);
        assert!(f.unfinishable);
        assert_eq!(
            f.stranded,
            vec![(0, 0), (1, 0)],
            "both visited nodes are doomed"
        );
        assert_eq!(f.unreachable, vec![2]);
    }

    #[test]
    fn a_doomed_branch_does_not_taint_its_sibling() {
        // Two branches off the start: a safe descent (floor, then a level
        // goal) and a 40-unit pocket whose only edge climbs back out — over
        // the step cap, so the pocket is doomed. Nodes on the safe branch must
        // NOT be reported merely because the other branch dead-ends.
        let g = graph(
            vec![
                node(0, 200, 0),    // 0 start
                node(-100, 200, 0), // 1 floor: safe, drops from start
                node(-100, 200, 0), // 2 goal, level with floor
                node(-40, 200, 0),  // 3 pocket: 40 back up to start, blocked
            ],
            vec![open(0, 1), open(1, 2), open(0, 3)],
            0,
            vec![2],
        );
        let f = check(&g, &LIMITS);
        assert!(!f.unfinishable);
        assert_eq!(f.stranded, vec![(3, 0)], "only the pocket is doomed");
    }

    #[test]
    fn a_pit_survivable_only_with_the_key_still_strands_the_keyless() {
        // The central semantic: a node is stranded when SOME arrival is
        // doomed, not when every one is. Node 2 is a 100-unit pit whose only
        // way on is a door locked to the key in node 1, so it is survivable as
        // (2, 0b1) and fatal as (2, 0) — the player who dives in before
        // detouring for the key is softlocked, and that is the state reported.
        // Requiring all arrivals to be doomed would report nothing here and
        // silently gut the rule.
        //
        // nodes: 0 start, 1 key(class 0), 2 pit 100 below start,
        //        3 exit behind the pit's locked door.
        let g = graph(
            vec![
                node(0, 128, 0),
                node(0, 128, 0b1),
                node(-100, 128, 0),
                node(-100, 128, 0),
            ],
            vec![open(0, 1), open(0, 2), door(2, 3, Some(0))],
            0,
            vec![3],
        );
        let f = check(&g, &LIMITS);
        assert!(
            !f.unfinishable,
            "grab the key first and the exit is reachable"
        );
        assert_eq!(
            f.stranded,
            vec![(2, 0)],
            "entering the pit keyless is the softlock"
        );
    }

    /// hub (start) — plain portal — annex, with a switch exit on hub's west
    /// wall. Two compiler-made sectors: the portal's passage. Everything
    /// level, so the graph is fully traversable.
    const HUB_ANNEX: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
        { "id":"annex", "footprint":[[320,0],[320,256],[576,256],[576,0]],
          "floor":0, "ceiling":128, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[{ "a":"hub", "b":"annex", "kind":"plain", "width":64, "at":[256,128] }],
      "exits":[{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] }] }"#;

    fn built(json: &str) -> BuiltGraph {
        let ir = Ir::from_json(json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, _) = compile_reporting(&ir, &tables).expect("compile");
        graph_from_compiled(&ir, &tables, &out).expect("graph")
    }

    #[test]
    fn the_builder_maps_rooms_starts_and_goals() {
        let b = built(HUB_ANNEX);
        assert_eq!(b.graph.start, 0, "hub is room 0 is sector 0");
        assert_eq!(
            b.graph.goals,
            vec![0],
            "a switch exit fires from its host room"
        );
        assert_eq!(b.graph.nodes[0].floor, 0);
        assert_eq!(b.graph.nodes.len(), 3, "two rooms and one passage sector");
        // Two thresholds: hub<->passage and passage<->annex, both Open.
        assert_eq!(b.graph.edges.len(), 2);
        assert!(b.graph.edges.iter().all(|e| e.kind == EdgeKind::Open));
    }

    #[test]
    fn a_walkover_exits_goal_is_the_recess_beyond_its_threshold() {
        let json = HUB_ANNEX.replace(
            r#""trigger":"switch", "width":32, "at":[0,128]"#,
            r#""trigger":"walkover", "width":64, "at":[0,128]"#,
        );
        let ir = Ir::from_json(&json).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, _) = compile_reporting(&ir, &tables).expect("compile");
        let b = graph_from_compiled(&ir, &tables, &out).expect("graph");
        // Derived from the emitted map, not written down: the sector behind
        // the line that actually carries the walkover special. `>= 2` would
        // also accept the portal's passage sector, which is not the goal.
        let walkover = tables.exit_walkover_special();
        let recesses: Vec<NodeIdx> = out
            .data
            .linedefs
            .iter()
            .filter(|l| l.special == walkover)
            .map(|l| {
                let back = l.back.expect("a walkover threshold is two-sided");
                out.data.sidedefs[back].sector
            })
            .collect();
        assert_eq!(recesses.len(), 1, "one exit, one threshold");
        assert_eq!(b.graph.goals, recesses, "the goal is that exact recess");
        assert_ne!(b.graph.goals[0], 0, "not the host room");
        // The recess is reachable, so the whole map is finishable.
        let limits = Limits {
            player_height: tables.player().height,
            max_step: tables.step_height(),
        };
        assert!(!check(&b.graph, &limits).unfinishable);
    }

    #[test]
    fn an_unlocked_door_portal_yields_door_edges_with_no_lock() {
        // Load-bearing and otherwise uncovered: an unlocked door portal's
        // faces carry `tables.door_special()` (doors.rs `door_special`), and
        // a door sector is emitted with ceiling == floor. `passable` skips
        // the crossing-window check only for `EdgeKind::Door`, so if these
        // faces were classified `Open` their zero-height window would make
        // every map with an ordinary door read unfinishable.
        let json = r#"{ "seed":1, "grid":64, "theme":"tech_base",
          "rooms":[
            { "id":"hub", "footprint":[[0,0],[0,256],[256,256],[256,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
              "things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },
            { "id":"annex", "footprint":[[320,0],[320,256],[576,256],[576,0]],
              "floor":0, "ceiling":128, "light":160,
              "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
          ],
          "portals":[{ "a":"hub", "b":"annex", "kind":"door",
                       "width":128, "at":[256,128],
                       "door_thickness":32, "alcove_near":16, "alcove_far":16 }],
          "exits":[{ "room":"annex", "trigger":"switch", "width":32, "at":[576,128] }] }"#;
        let b = built(json);
        let locks: Vec<Option<KeyClass>> = b
            .graph
            .edges
            .iter()
            .filter_map(|e| match &e.kind {
                EdgeKind::Door { lock } => Some(*lock),
                EdgeKind::Open | EdgeKind::Teleport | EdgeKind::Lift => None,
            })
            .collect();
        assert_eq!(locks, vec![None, None], "both door faces, neither locked");
        // Keyless, and the exit is on the far side of the door.
        assert!(b.graph.nodes.iter().all(|n| n.keys == 0));
        let tables = Tables::load().expect("tables");
        let limits = Limits {
            player_height: tables.player().height,
            max_step: tables.step_height(),
        };
        assert!(
            !check(&b.graph, &limits).unfinishable,
            "a plain door opens, so the exit beyond it is reachable"
        );
    }

    #[test]
    fn a_locked_door_edge_and_the_matching_key_share_a_colour_class() {
        // The lock is blue_card; the placed key is blue_SKULL. EV_VerticalDoor
        // accepts either of a colour, so the classes must collapse.
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
        let b = built(json);
        let lock_classes: Vec<Option<KeyClass>> = b
            .graph
            .edges
            .iter()
            .filter_map(|e| match &e.kind {
                EdgeKind::Door { lock } => Some(*lock),
                EdgeKind::Open | EdgeKind::Teleport | EdgeKind::Lift => None,
            })
            .collect();
        assert!(!lock_classes.is_empty(), "the door faces are Door edges");
        let class = lock_classes[0].expect("the door is locked");
        assert!(
            lock_classes.iter().all(|l| *l == Some(class)),
            "both faces, same class"
        );
        assert_eq!(
            b.graph.nodes[0].keys,
            1 << class,
            "the skull satisfies the card's lock"
        );
        let tables = Tables::load().expect("tables");
        let limits = Limits {
            player_height: tables.player().height,
            max_step: tables.step_height(),
        };
        let f = check(&b.graph, &limits);
        assert!(!f.unfinishable, "the skull opens the blue door");
        // class_names for messages: both kinds of the colour, sorted.
        assert_eq!(b.class_names[class as usize], ["blue_card", "blue_skull"]);
    }

    #[test]
    fn a_secret_switch_exit_is_a_goal_and_two_exits_in_a_room_collapse_to_one() {
        // The two secret specials are live in `exit_specials` but were
        // otherwise unfixtured: if `secret_exit_switch_special` were missing
        // from that list, `goals` would be empty and the builder would gate
        // out to `None`, so `built` panicking is the assertion.
        let secret_only = HUB_ANNEX.replace(
            r#"{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] }"#,
            r#"{ "room":"hub", "trigger":"switch", "secret":true, "width":32, "at":[0,128] }"#,
        );
        let b = built(&secret_only);
        assert_eq!(
            b.graph.goals,
            vec![0],
            "a secret switch exit fires from its host room, same as a normal one"
        );

        // Two exits on two of hub's walls, both fronting hub: `goals.dedup()`
        // is what keeps the same sector from being listed twice.
        let two_exits = HUB_ANNEX.replace(
            r#"{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] }"#,
            r#"{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] },
        { "room":"hub", "trigger":"switch", "secret":true, "width":32, "at":[128,0] }"#,
        );
        let ir = Ir::from_json(&two_exits).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, _) = compile_reporting(&ir, &tables).expect("compile");
        let exit_lines = out
            .data
            .linedefs
            .iter()
            .filter(|l| {
                l.special == tables.exit_switch_special()
                    || l.special == tables.secret_exit_switch_special()
            })
            .count();
        assert_eq!(exit_lines, 2, "both exits emitted, or dedup proves nothing");
        let b = graph_from_compiled(&ir, &tables, &out).expect("graph");
        assert_eq!(b.graph.goals, vec![0], "one goal, not one per exit line");
    }

    #[test]
    fn labels_name_rooms_directly_and_passages_by_their_rooms() {
        let b = built(HUB_ANNEX);
        let ir = Ir::from_json(HUB_ANNEX).expect("ir");
        assert_eq!(node_label(0, &ir, &b.graph), "room `hub`");
        assert_eq!(node_label(1, &ir, &b.graph), "room `annex`");
        assert_eq!(
            node_label(2, &ir, &b.graph),
            "the passage between `annex` and `hub`"
        );
    }

    #[test]
    fn the_gate_returns_none_without_a_start_or_without_an_exit() {
        let ir_no_exit = HUB_ANNEX.replace(
            r#""exits":[{ "room":"hub", "trigger":"switch", "width":32, "at":[0,128] }]"#,
            r#""exits":[]"#,
        );
        let ir = Ir::from_json(&ir_no_exit).expect("ir");
        let tables = Tables::load().expect("tables");
        let (out, _) = compile_reporting(&ir, &tables).expect("compile");
        assert!(
            graph_from_compiled(&ir, &tables, &out).is_none(),
            "no exit: vacuous"
        );

        let ir_no_start = HUB_ANNEX.replace(
            r#""things":[{ "kind":"player1_start", "at":[128,128], "angle":90 }] },"#,
            r#""things":[] },"#,
        );
        let ir = Ir::from_json(&ir_no_start).expect("ir");
        let (out, _) = compile_reporting(&ir, &tables).expect("compile");
        assert!(
            graph_from_compiled(&ir, &tables, &out).is_none(),
            "no start: vacuous"
        );
    }

    #[test]
    fn a_teleport_edge_is_one_way_and_ignores_heights() {
        // 0: start (floor 0); 1: destination on a ledge 200 up; edge 0→1 teleport.
        let graph = ReachGraph {
            nodes: vec![
                Node {
                    floor: 0,
                    ceiling: 128,
                    keys: 0,
                    fires: 0,
                    action: None,
                },
                Node {
                    floor: 200,
                    ceiling: 328,
                    keys: 0,
                    fires: 0,
                    action: None,
                },
            ],
            edges: vec![Edge {
                a: 0,
                b: 1,
                kind: EdgeKind::Teleport,
                fires: 0,
            }],
            start: 0,
            goals: vec![1],
        };
        let f = check(
            &graph,
            &Limits {
                player_height: 56,
                max_step: 24,
            },
        );
        assert!(
            !f.unfinishable,
            "the teleport reaches the ledge despite the 200-unit rise"
        );
        assert!(f.unreachable.is_empty());
        // Reversed, the edge is not traversable: start on the ledge, goal below.
        let back = ReachGraph {
            start: 1,
            goals: vec![0],
            ..graph
        };
        let f = check(
            &back,
            &Limits {
                player_height: 56,
                max_step: 24,
            },
        );
        assert!(f.unfinishable, "a teleport is directed a → b");
        assert_eq!(f.unreachable, vec![0]);
    }

    #[test]
    fn compiled_teleports_add_player_edges_but_not_monster_edges() {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(
            r#"{ "seed":1, "grid":64, "theme":"tech_base",
              "rooms":[
                { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":128, "light":160,
                  "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                  "things":[ { "kind":"player1_start", "at":[192,64], "angle":90 } ] },
                { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":128, "light":160,
                  "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
                  "things":[ { "kind":"imp", "at":[384,64], "angle":0 } ] }
              ],
              "portals":[],
              "exits":[{ "room":"b", "trigger":"walkover", "at":[448,256], "width":64 }],
              "teleports":[
                { "id":"go", "room":"a", "pad":{"island":[64,128]}, "to":{"room":"b","at":[512,128],"angle":90} },
                { "id":"pen", "room":"b", "pad":{"island":[384,128]}, "to":{"room":"a","at":[192,192],"angle":0}, "monsters_only":true }
              ] }"#,
        )
        .expect("ir");
        let out = crate::compile::compile(&ir, &tables).expect("compiles");
        let built = graph_from_compiled(&ir, &tables, &out).expect("graph");
        let teleports: Vec<&Edge> = built
            .graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Teleport)
            .collect();
        assert_eq!(
            teleports.len(),
            1,
            "the monsters-only pad adds no player edge"
        );
        assert_eq!(
            (teleports[0].a, teleports[0].b),
            (0, 1),
            "from the host room to room b"
        );
    }

    /// A bridge whose walkover names its own two rooms: the special lands
    /// on *both* of the pit's thresholds, so stepping down into it from
    /// either side raises it under the player. A verbatim copy of
    /// `compile::floors`'s own `BRIDGE_WALKOVER`, which lives in that
    /// module's private test module and so cannot be shared — the same
    /// arrangement `rules.rs`'s `WALL_MAP` already has.
    const BRIDGE_WALKOVER: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[64,64], "angle":0 } ] },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"bridge", "width":64, "at":[256,128], "depth":96, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"walkover", "portal":["a","b"] } ],
      "exits":[ { "room":"b", "trigger":"switch", "at":[576,128], "width":64 } ] }"#;

    /// [`BRIDGE_WALKOVER`] with the start and the exit swapped: the player
    /// begins in room `b` and must cross the pit's *far* threshold — the
    /// one `TriggerOut::line` does not name — to reach the exit in `a`.
    const BRIDGE_WALKOVER_FROM_FAR: &str = r#"{ "seed":1, "grid":64, "theme":"tech_base",
      "rooms":[
        { "id":"a", "footprint":[[0,0],[0,256],[256,256],[256,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3" },
        { "id":"b", "footprint":[[320,0],[320,256],[576,256],[576,0]], "floor":0, "ceiling":192, "light":160,
          "floor_tex":"FLOOR4_8", "ceil_tex":"CEIL3_5", "wall_tex":"STARTAN3",
          "things":[ { "kind":"player1_start", "at":[512,64], "angle":0 } ] }
      ],
      "portals":[ { "a":"a", "b":"b", "kind":"bridge", "width":64, "at":[256,128], "depth":96, "fires_on":"t" } ],
      "triggers":[ { "id":"t", "kind":"walkover", "portal":["a","b"] } ],
      "exits":[ { "room":"a", "trigger":"switch", "at":[0,128], "width":64 } ] }"#;

    /// The `(a, b)` sector pair of each linedef in `lines`, sorted — the
    /// shape an edge built from that linedef carries.
    fn sector_pairs(out: &Compiled, lines: &[usize]) -> Vec<(NodeIdx, NodeIdx)> {
        let mut pairs: Vec<(NodeIdx, NodeIdx)> = lines
            .iter()
            .map(|&l| {
                let line = &out.data.linedefs[l];
                let back = line.back.expect("a gap threshold is two-sided");
                (
                    out.data.sidedefs[line.front].sector,
                    out.data.sidedefs[back].sector,
                )
            })
            .collect();
        pairs.sort_unstable();
        pairs
    }

    #[test]
    fn a_walkover_bridge_fires_from_both_of_its_thresholds() {
        // `emit_trigger_line` writes the walkover special onto BOTH of a
        // bridge's thresholds, and `TriggerOut::line` records only the
        // first. Reading the emitted lines back — rather than that one
        // index — is what makes stepping in from the far side raise the
        // pit; taking only `line` leaves the far threshold inert.
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(BRIDGE_WALKOVER).expect("ir");
        let out = crate::compile::compile(&ir, &tables).expect("compiles");
        let b = graph_from_compiled(&ir, &tables, &out).expect("graph");
        assert_eq!(out.floors.len(), 1, "one bridge, one action");
        assert_eq!(out.floors[0].lines.len(), 2, "a bridge has two thresholds");

        let mut firing: Vec<(NodeIdx, NodeIdx)> = b
            .graph
            .edges
            .iter()
            .filter(|e| e.fires != 0)
            .map(|e| (e.a, e.b))
            .collect();
        firing.sort_unstable();
        assert_eq!(
            firing,
            sector_pairs(&out, &out.floors[0].lines),
            "both thresholds fire, and nothing else does"
        );
        assert!(
            b.graph
                .edges
                .iter()
                .filter(|e| e.fires != 0)
                .all(|e| e.fires == 1 << ACTION_BIT_BASE),
            "the map's one action is bit 0 of the action half"
        );
    }

    #[test]
    fn a_walkover_bridge_entered_from_the_far_room_still_raises() {
        // The same pit approached from the side `TriggerOut::line` does not
        // name. Dropping in is free either way; walking back out of a
        // 96-deep pit is not, so a far threshold that failed to fire would
        // leave the player stranded in it and the map unfinishable.
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(BRIDGE_WALKOVER_FROM_FAR).expect("ir");
        let out = crate::compile::compile(&ir, &tables).expect("compiles");
        let b = graph_from_compiled(&ir, &tables, &out).expect("graph");
        let f = check(
            &b.graph,
            &Limits {
                player_height: tables.player().height,
                max_step: tables.step_height(),
            },
        );
        assert!(!f.unfinishable, "{f:?}");
        assert!(f.stranded.is_empty(), "{f:?}");
        assert!(f.unreachable.is_empty(), "{f:?}");
    }

    #[test]
    fn an_actions_name_is_the_one_its_tag_manifest_row_carries() {
        // `action_names` words P7's violations and the manifest row is what
        // an author reads beside them; both come from
        // `floors::construct_name`, and this is what holds them to it.
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(BRIDGE_WALKOVER).expect("ir");
        let out = crate::compile::compile(&ir, &tables).expect("compiles");
        let b = graph_from_compiled(&ir, &tables, &out).expect("graph");
        assert_eq!(b.action_names, ["bridge a <-> b"]);
        let row = out
            .tags
            .manifest()
            .iter()
            .find(|e| e.tag == out.floors[0].tag)
            .expect("the trigger's tag is in the manifest");
        assert!(
            row.purpose.contains(&b.action_names[0]),
            "the manifest row `{}` must quote the action the same way",
            row.purpose
        );
    }

    #[test]
    fn the_lift_golden_is_finishable_and_the_pedestal_is_reachable() {
        let tables = Tables::load().expect("tables");
        let ir = Ir::from_json(include_str!("../tests/golden/lifts.json")).expect("ir");
        let out = crate::compile::compile(&ir, &tables).expect("compiles");
        let built = graph_from_compiled(&ir, &tables, &out).expect("start and exit");
        let lifts: Vec<&Edge> = built
            .graph
            .edges
            .iter()
            .filter(|e| e.kind == EdgeKind::Lift)
            .collect();
        assert_eq!(
            lifts.len(),
            5,
            "lift (from low), barrier (from ledge and far), walkover lift (from the alcove), pedestal (from low)"
        );
        let findings = check(
            &built.graph,
            &Limits {
                player_height: tables.player().height,
                max_step: tables.step_height(),
            },
        );
        assert!(!findings.unfinishable);
        assert!(
            findings.unreachable.is_empty(),
            "{:?}",
            findings.unreachable
        );
        assert!(findings.stranded.is_empty());
    }
}
