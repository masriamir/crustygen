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
//! `(where the player stands, which keys they hold)` — so that "holds the blue
//! card" and "is stuck in `key_room`" are one state and the search cannot
//! pretend otherwise. The state space is sectors × 2^keys over a handful of key
//! classes, so this is a small product-graph BFS, not a solver.
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
//! # The vacuous-pass gate
//!
//! P7 runs only when the map has a player 1 start and at least one exit;
//! otherwise it passes vacuously. A softlock presupposes a goal, and "this map
//! has no exit" is a spec-conformance finding belonging to the stage that reads
//! the map spec, not to this rule. The gate lives in the builder that derives a
//! [`ReachGraph`] from compiled geometry; it is documented here so that its
//! absence from an exit-less map's violations reads as a decision rather than
//! an oversight.

/// Index of a node (sector) in a [`ReachGraph`].
pub type NodeIdx = usize;

/// A key class: a bit position in a [`KeyMask`]. Classes are interned by the
/// keyed-door special the key satisfies, so the card and skull of a colour
/// share one class — `EV_VerticalDoor` (pinned `p_doors.c:371-403`) accepts
/// either: `!p->cards[it_bluecard] && !p->cards[it_blueskull]`.
pub type KeyClass = u8;

/// A set of key classes held, one bit per [`KeyClass`].
pub type KeyMask = u8;

/// One sector, reduced to what traversal needs.
#[derive(Debug, Clone)]
pub struct Node {
    /// Floor height in map units.
    pub floor: i32,
    /// Ceiling height in map units.
    pub ceiling: i32,
    /// Key classes collectible here. Entering the node collects them all.
    pub keys: KeyMask,
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
}

/// An undirected boundary between two nodes; passability is evaluated
/// per crossing direction.
#[derive(Debug, Clone)]
pub struct Edge {
    /// One side.
    pub a: NodeIdx,
    /// The other side.
    pub b: NodeIdx,
    /// How the boundary traverses.
    pub kind: EdgeKind,
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
    pub stranded: Vec<(NodeIdx, KeyMask)>,
    /// Nodes never visited in any state, ascending.
    pub unreachable: Vec<NodeIdx>,
}

/// Whether one crossing of `kind` from `from` to `to` is possible while
/// holding `mask`.
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
    if to.floor - from.floor > limits.max_step {
        return false;
    }
    match kind {
        EdgeKind::Open => {
            to.ceiling.min(from.ceiling) - to.floor.max(from.floor) >= limits.player_height
        }
        EdgeKind::Door { lock } => lock.is_none_or(|k| {
            // A class at or past the mask's width would shift its bit off the
            // end: a debug panic, but in release `1u8 << 8` wraps to `1u8 << 0`
            // and silently aliases the class onto class 0 — a locked door that
            // opens to the wrong key, with no test able to see it. Interning
            // must keep classes under this cap.
            debug_assert!(
                u32::from(k) < KeyMask::BITS,
                "key class {k} does not fit a {}-class KeyMask",
                KeyMask::BITS
            );
            mask & (1 << k) != 0
        }),
    }
}

/// Runs the P7 flood over `(node, keys-held)` states: a forward search from
/// the start, then a backward search from the goals over what the first one
/// found.
///
/// A set-union flood over sectors alone cannot express "holding the key
/// strands you" — the shipped `key_room` defect — so the state carries the
/// mask, and entering a node unions that node's keys in (masks only grow
/// along a walk).
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
/// backward pass indexes with a `u32`. The state space is `nodes × 2^8`, so
/// this needs over 16 million sectors — two orders of magnitude past the
/// `u16` sector index the map format itself can address.
#[must_use]
pub fn check(graph: &ReachGraph, limits: &Limits) -> Findings {
    let n = graph.nodes.len();
    let mut adj: Vec<Vec<(NodeIdx, &EdgeKind)>> = vec![Vec::new(); n];
    for e in &graph.edges {
        adj[e.a].push((e.b, &e.kind));
        adj[e.b].push((e.a, &e.kind));
    }
    let norm = |node: NodeIdx, mask: KeyMask| mask | graph.nodes[node].keys;
    // A state index packs (node, mask); a KeyMask is 8 bits by construction.
    let idx = |node: NodeIdx, mask: KeyMask| (node << 8) | mask as usize;

    let mut seen = vec![false; n << 8];
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
        for &(to, kind) in &adj[at] {
            if !passable(&graph.nodes[at], &graph.nodes[to], kind, mask, limits) {
                continue;
            }
            let to_mask = norm(to, mask);
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
    let mut pos = vec![u32::MAX; n << 8];
    for (i, &(node, mask)) in order.iter().enumerate() {
        pos[idx(node, mask)] = u32::try_from(i).expect("state count fits u32");
    }
    let mut preds: Vec<Vec<u32>> = vec![Vec::new(); order.len()];
    for (i, &(node, mask)) in order.iter().enumerate() {
        for &(to, kind) in &adj[node] {
            if !passable(&graph.nodes[node], &graph.nodes[to], kind, mask, limits) {
                continue;
            }
            let j = pos[idx(to, norm(to, mask))];
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

#[cfg(test)]
mod tests {
    use super::*;

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
        }
    }
    fn open(a: NodeIdx, b: NodeIdx) -> Edge {
        Edge {
            a,
            b,
            kind: EdgeKind::Open,
        }
    }
    fn door(a: NodeIdx, b: NodeIdx, lock: Option<KeyClass>) -> Edge {
        Edge {
            a,
            b,
            kind: EdgeKind::Door { lock },
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
    fn a_node_safe_on_one_walk_is_not_reported_stranded() {
        // Two branches off the start: a safe descent (floor, then a level
        // goal) and a 40-unit pocket whose only edge climbs back out — over
        // the step cap, so the pocket is doomed. Nodes on the safe branch
        // must NOT be reported merely because a doomed state exists elsewhere.
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
}
