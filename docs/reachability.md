# crustygen P7 reachability — design

**Date:** 2026-08-10
**Status:** approved, ready for an implementation plan
**Issue:** #4 — P7: key-aware reachability flood (a one-way drop can strand the player)
**Scope of this document:** the flood core, its compiled-geometry builder, and the three
P7 checks. P20 and the `crustygen-check` verifier (#2) are consumers, not deliverables.

## Problem

Nothing verifies a compiled map can be finished. Retiring P1 made one-way drops legal by
design — correctly, since 37.77% of vanilla's passable two-sided lines exceed the step
cap (`docs/measurements/verticality-corpus.md`) — and the project has already shipped
the failure once: `key_room` authored 32 units below `hub`, a dead end holding the only
key, compiled clean through 198 tests and a whole-branch review. §2's success criterion
"every area reachable, with the key/door/exit sequence functioning" has no
implementation, and `KNOWN-GAPS.md` records authoring discipline as the only guard.

## The finding that shapes the design

**The flood issue #4 literally asks for cannot catch the bug that motivates it.** A
set-union flood — accumulate reachable rooms, unlock doors as their keys become
reachable, iterate to fixpoint — passes the shipped `key_room` map: the room is
reachable (drop in), so the blue card counts as obtained, so the locked door opens, so
the exit reads reachable. The real player who takes that drop is stranded, holding the
key. Reachability of *places* and *items* is not the property; the property is over
**states** — (where the player is, which keys they hold) — so that "holding the blue
card" and "being stuck in `key_room`" are the same state and the search cannot pretend
otherwise. The state space is sectors × 2^keys with at most six key kinds in the
vocabulary, so this is a small product-graph BFS, not a solver.

## Engine facts this design depends on

All verified against the pinned commit `a77dfb96cb91780ca334d0d4cfd86957558007e0`
(re-fetched and re-read for this document, not recalled):

- **`p_maputl.c`, `P_LineOpening` (lines 300–329)** — the window through a two-sided
  line is `opentop = min(ceilingheight)`, `openbottom = max(floorheight)` of the two
  sectors. **`p_map.c`, `P_TryMove` (468–469)** rejects the move when
  `tmceilingz - tmfloorz < thing->height`. Together: a boundary is crossable only if
  `min(ceilings) − max(floors) ≥` the mover's height.
- **`p_map.c`, `P_TryMove` (477–479)** — `tmfloorz - thing->z > 24*FRACUNIT` rejects
  the move: the cap is on stepping **up** only. Already sourced in `data/engine.toml`
  as `max_step_height`.
- **`p_map.c`, `P_TryMove` (481–483)** — the dropoff rejection is gated on
  `!(thing->flags & (MF_DROPOFF|MF_FLOAT))`, and **`info.c` (1130)** gives `MT_PLAYER`
  `MF_DROPOFF`: the player is never dropoff-blocked, so descents are unrestricted.
- **`p_map.c`, `P_TryMove` (474–475)** — the "mobj must lower itself to fit" check
  (`tmceilingz - thing->z < thing->height`) is **subsumed** by the window check for
  ground movement: a walking player's `z` is a floor at or below `tmfloorz`, so
  `tmceilingz - thing->z ≥ tmceilingz - tmfloorz`. The model therefore needs exactly
  two crossing rules, not three.

## Design

### 1. A pure flood core in `src/reach.rs`

A new top-level module — a sibling of `rules.rs`, deliberately not a compile pass,
because the algorithm is verifier-grade: issue #2's `crustygen-check` will re-derive
the same graph from emitted `TEXTMAP` and must reuse the core untouched.

The core owns an abstract traversal graph and the search over it:

- **Nodes** are sectors: floor, ceiling, and the keys collectible there.
- **Edges** are shared boundaries: two node indices and a kind — `Open`, or
  `Door { lock: Option<KeyClass> }`.
- **`start`** is the node holding the player 1 start; **`goals`** are the nodes from
  which an exit fires.
- **Locks are interned by color, not by key kind.** `p_doors.c`'s `EV_VerticalDoor`
  (371–403, pinned commit) opens special 26/27/28 for the card **or** the skull of the
  color — `!p->cards[it_bluecard] && !p->cards[it_blueskull]` — and the vocabulary
  maps both kinds of a color to the same special. A blue skull opens a door authored
  `lock: "blue_card"`, and the flood must agree with the engine, so a `KeyClass` is the
  color class and both key things of a color contribute it. A state is `(node, mask)`
  over a small bitmask (three colors); entering a node unions its keys into the mask,
  so masks only grow along a walk.

An edge is passable in a given direction when:

- **step up:** `floor[to] − floor[from] ≤ max_step_height`; descents are free;
- **crossing window** (`Open` edges only): `min(ceilings) − max(floors) ≥` player
  height;
- **lock** (`Door` edges only): `lock` is `None` or the mask holds the key. Door edges
  **skip the window check** — a closed door's ceiling equals its floor, and
  clearance-when-open is P4's already-covered job; the flood asks only whether the door
  is unlockable. The step rule still applies to door floors, which is what catches a
  one-way passage *through* a door (a door sector floor is the `min` of its two rooms,
  so the step out to a far room more than `max_step_height` above it is rejected).

Node labels for messages are derived, not stored: a node index below `ir.rooms.len()`
is that room's id — the room-index-equals-sector-index invariant
`compile::things` already documents and verifies — and any other sector is described
by the rooms adjacent to it ("the passage between `hub` and `key_room`"). No
bookkeeping threads through the emit passes.

### 2. The three checks, all rule `P7`

1. **Finishable.** Forward BFS over states from `(start, ∅)`; some goal node must be
   reached in some state. Failure is the headline: "no feasible walk from the player
   start reaches an exit."
2. **No stranding.** Backward reachability from every goal state over the same product
   graph; any forward-reachable state that cannot reach a goal is a softlock. Reported
   per node with the keys held — "the player can reach `key_room` holding `blue_card`
   but can no longer finish from there" — which for the shipped bug is the actionable
   message naming the culprit room. When no exit is reachable at all — the finishable
   check above already failed — every visited state is trivially doomed, so the
   stranding report is narrowed to only the key-collecting sectors — the likely
   culprits — rather than burying the finishability headline under every room in the
   map.
3. **Coverage** (§2's "every area reachable"). Any node never forward-reached in any
   state. Rooms are named by id; compiler-made sectors by adjacency.

All violations are collected and returned, matching `check_all`'s existing
report-everything convention; `rules.rs` formats the core's typed findings into
`RuleViolation`s, and a violation is a hard error like every other playability rule.

### 3. The builder, `graph_from_compiled`

Derives the graph from what was actually emitted — never from authored intent, so it
cannot drift from the geometry the way an IR-level re-derivation could, and phase-2
stair chains and phase-3 lift sectors will appear in it automatically when they exist
(lifts will need a new edge kind for "traversable by riding"; that extension point is
the enum, not speculative support now):

- **Nodes** from `MapData.sectors` (floor, ceiling); **edges** from every linedef with
  a `back` sidedef, connecting the front and back sectors.
- **Door edges** recognized by the linedef special: `tables.door_special()` →
  `Door { lock: None }`; a keyed special → `Door { lock: Some(color) }`. The reverse
  lookup (built once per compile) is deliberately **many-to-one**: both key kinds of a
  color name the same special, so it resolves to the color class, matching
  `EV_VerticalDoor` above.
- **Goals** from the four exit specials: a switch exit's line is one-sided and its
  front sector is the host room — being in that room suffices, since
  `P_UseSpecialLine` fires from the front side the player faces; a walkover exit's
  goal is the alcove sector past its threshold, since reaching it means crossing the
  firing line.
- **Keys and the start** from IR room things via the index invariant: no
  point-in-sector machinery.

### 4. The vacuous-pass gate

**P7 runs only when the map has a player 1 start and at least one exit; otherwise it
passes vacuously**, and a test pins that. A softlock presupposes a goal; dozens of
existing two-room fixtures have neither; and "this map has no exit" is a spec
conformance finding that belongs to the stage that reads the map-spec (#1, #3), not to
this rule. The gate is documented in the module doc so its absence from a future
no-exit map's violations is legible as a decision, not an oversight.

## Testing

Fixture *shapes* that are new, per the fixture-diversity lesson (65 green tests once
hid four Critical defects because every fixture was the same square rotated):

1. **The entrada regression** — three rooms, the only key in a dead-end pit 32 below,
   a locked exit door: must report both Unfinishable and Stranded. The motivating case.
2. **Its climbable twin** — the same fixture at −16 compiles clean, plus an exact
   boundary pair at `max_step_height` (−24 passes, −25 fails) so an off-by-one cannot
   survive.
3. **A bare pit** — a no-key dead-end drop with a soulsphere: finishable, but the
   stranding check flags it. This is the fixture that distinguishes "no softlock" from
   "finishable"; a finishability-only implementation must fail it.
4. **A key behind its own door** — the blue card reachable only through the blue door:
   unfinishable, no state ever holds the key.
5. **A two-key chain** — red behind the blue door, blue in the open: passes, and
   exercises multi-key masks. A variant places the blue **skull** against a door
   authored `lock: "blue_card"` and must pass, pinning the color-class equivalence to
   the engine's behavior rather than to key-kind string equality.
6. **An isolated room** — no portals to it: the coverage check fires.
7. **A one-way door** — rooms more than `max_step_height` apart joined by a door
   portal: the step rule fires on the door-sector floor, not only on plain portals.
8. **Both exit triggers** — goal recognition for a switch exit and a walkover exit.
9. **The vacuous gate pinned** — an exit-less fixture compiles unchanged.

The core is additionally unit-tested on tiny hand-built graphs, one per semantic rule
(step, window, lock, monotone masks, backward stranding). Every check gets a **mutation
proof** — demonstrated failing against a deliberately broken flood (keys ignored,
direction ignored, stranding skipped) before it is trusted. All existing tests stay
green; `entrada` is the live positive fixture and keeps every drop climbable on
purpose.

## Records updated with the implementation

- `KNOWN-GAPS.md`: P7 moves out of "deliberately absent"; the one-way-drop gap and the
  entrada authoring-discipline entry are rewritten to point at the enforcement; a note
  records what P20 still lacks (a per-pickup check, meaningful once intra-room
  verticality exists — today "every room reachable" subsumes it at room granularity).
- `rules.rs` module doc: P7 joins the implemented catalog; P20 stays listed as absent,
  now pointing at `reach.rs` as the flood it will consume.
- `docs/verticality.md` is a dated record and stays untouched.

## Out of scope

P20's explicit per-pickup loop, the `crustygen-check` verifier and its
`TEXTMAP`-parsing builder (#2), the conformance report (#3), monster mobility (P6),
and any lift-riding edge kind (phase 3). The "specified order" clause of §2 — keys and
doors in the *authored* sequence — needs the map-spec (#1) and belongs to the
conformance stage; P7 asserts an order exists, not that it matches the spec.

Reconciling P7's colour-class lock checking with P24's exact-string lock checking is
also out of scope — the two rules deliberately disagree; see `KNOWN-GAPS.md`'s
"P24 is stricter than the engine about key kinds, and P7 is not."
