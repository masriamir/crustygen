# crustygen — known gaps and carried decisions

State as of the compiler's completion: IR → validated UDMF `TEXTMAP` → PWAD →
reassembles through crustywad. 100 tests. This file records what is deliberately
absent, what is known-fragile, and the decisions a future contributor would
otherwise have to re-derive.

## Not implemented, by design

The compiler covers structural invariants S1–S6 and playability rules P1, P2,
P3, P4, P8, P9, P11, P13, P14, P19, P24, P25.

Deliberately absent, deferred to the next stage: **P5** (lifts), **P6** (monster
mobility), **P7** (no softlock), **P10** (clean vertical tiling), **P12** (sky
coherence), **P15** (teleport pairing), **P16**/**P17** (liquids and damage
survivability), **P18** (secret accounting), **P20** (pickup accessibility),
**P21** (light sources), **P22** (hanging decorations), **P23** (barrel
safety).

P7 and P20 both need a key-aware reachability flood that does not exist yet.

**P2 (headroom) is now fully covered.** `compile::things::place_things` checks
every room's headroom against the player's own height once per room,
regardless of whether that room places any things at all, in addition to the
existing per-thing check for anything taller than the player. Previously the
check only ran inside the per-thing loop, so a room with no things skipped it
entirely and an empty corridor too short for the player to stand in compiled
clean; `things::tests::p2_an_empty_room_too_short_for_the_player_is_rejected`
pins the fix. P25 (start clearance) is fully covered by the same code path,
since every player start is itself a thing and always goes through the
identical clearance/headroom/overlap checks.

**P18's mechanism exists; its counting rule does not.** `Room::secret`
(`compile::sectors::resolve_secret_specials`) gives a room a high-level way to
carry the sourced secret sector special (`Tables::secret_sector_special`)
instead of requiring an author to write the raw number into `Room::special` —
the two are mutually exclusive, rejected at parse time
(`IrError::SecretWithExplicitSpecial`) rather than resolved by silent
precedence. What is still absent is P18's actual *rule* — "the number of
secret sectors equals `secrets.count`" — since `secrets.count` is a map-spec
concept with no representation in this IR; that check belongs at the stage
that reads the map-spec, not here.

Also absent: the packer, the verifier, the conformance report, the blank
Markdown template, and any authored map. Specials for lifts, teleports, and
liquid sector effects, monster `spawnhealth`, health/armor pickup amounts and
caps, the gore prop set, and the `ML_BLOCKMONSTERS`/`ML_SOUNDBLOCK` linedef
flags are all **sourced and accessible** but nothing emits any of them yet.
Doors, exits (`compile::exits`), and the secret sector special are wired end
to end.

## Known gaps

**The orphan-sidedef code path is documented but unexercised.** It fires only
when an opening consumes a wall end to end so the reusable sidedef is never
taken. No fixture reaches it.

**crustygen runs in no CI.** It declares its own `[workspace]`, so the parent
repo's `cargo fmt --all` and `cargo clippy --workspace` do not reach it, and
its lints are `warn` where crustywad uses `-D warnings`. Wire this up at
repository-promotion time.

## Decisions that look wrong without their reason

**Sector footprints wind clockwise.** A linedef's front (right) sidedef only
faces the sector interior under clockwise winding. Verified empirically: 2611
of 2611 sector boundaries across nine Freedoom maps in both IWADs.

**`geom::contains` returns `true` for points exactly on a boundary.** Even-odd
ray casting has no defined tie-break there. `point_on_polygon_boundary` guards
the overlap test, and the radius-clearance check is the backstop everywhere
else. Do not assume it is a strict interior test.

**Door carving is asymmetric — always into room `b`.** Room `a` is never
modified, which is what lets the far face reuse `emit_opening` unchanged.
Consequence: swapping `a` and `b` in a portal declaration physically relocates
the door. Documented on `Portal`.

**Clearance measures emitted geometry, not the IR footprint.** Door carving
makes room `b`'s declared polygon larger than its real playable area. Measuring
the footprint reports a thing embedded in carved-away wall as comfortably clear.

**Portal `width` and `at` are exempt from the grid rule** that binds footprints.
Real doorways are routinely finer than the 64-unit grid their rooms sit on.

**Tag 0 is rejected on any action line.** It is not "no tag" — it is the tag
every untagged sector already carries, so an action left at zero matches every
untagged sector in the map. One stray zero opens every door.

**Portals and exits on diagonal walls remain unsupported by design, not by
omission.** `wall_edges` (and so `shared_spans`) only ever reported
axis-aligned edges, so a portal or exit requested on a genuinely diagonal wall
used to fall through to `NotAdjacent`/`PortalOffWall`/`ExitOffWall` — messages
that read as "there is no wall here" for a wall an author can plainly see.
`resolve_portal` and `resolve_exit` now check `portals::on_diagonal_wall`
before returning those errors and raise `CompileError::PortalOnDiagonalWall`/
`ExitOnDiagonalWall` instead, naming the exact coordinate. Supporting a portal
or exit *on* a diagonal wall properly would need a wall model wider than
`(axis, fixed coordinate)`, which the opening-splitting, jamb, and recess
machinery all assume; a chamfered room with its portals and doors on its
square walls (the common real case) already works today, both proved by
`portals::tests::a_portal_works_on_the_axis_aligned_wall_of_a_diagonally_shaped_room`
and the equivalent exit/door fixtures.

**`depth_behind_wall` measures diagonal far walls too, not just parallel
ones.** It used to filter any edge with `across_p != across_q`, which
correctly dropped a perpendicular side wall but *also* silently dropped a
diagonal far wall — the two are geometrically distinct (a side wall bounds a
single `along` position, a diagonal wall bounds a range with a linearly
varying depth) but shared that one boolean. A room whose far side was
chamfered therefore always measured `available: 0`, rejecting a door a deep
room could plainly fit. The fix evaluates a diagonal edge's contribution at
the two ends of its overlap with the opening (the minimum of a linear
function over an interval always falls at an endpoint) instead of skipping it
outright.

**Lifts and teleports are repeatable, not one-shot.** A design choice, not a
source fact: P5 requires a lift be operable from both ends, and a one-shot lift
can strand a player. Recorded in the citations; disagree there if you prefer.

**Odd portal widths are rejected rather than rounded**, per the spec's
reject-don't-degrade posture. The same posture governs
`IrError::SecretWithExplicitSpecial`: a room that sets both `Room::secret` and
`Room::special` is rejected outright rather than letting one silently win.

**A thing's unspecified `skillN` fields default to `true`, not `false`.**
Inverted from bare Rust `bool` defaults on purpose: the pre-existing behavior
(every thing on every skill) had to survive a thing that names no `skills` key
at all, and `ThingSkills` needed the same "unless you say otherwise" default
for a partially specified object too, so a per-field serde default function is
used rather than `#[derive(Default)]`.

**A walkover exit carves its own dead-end alcove; a switch exit does not.**
The pinned engine's `PIT_CheckLine` rejects a mover's crossing — and never
reaches the `spechit` bookkeeping that fires a walkover special — for both a
one-sided line and a two-sided `ML_BLOCKING` one. A walkover exit therefore
has to be a genuinely passable two-sided line, and placing one flush on a
room's true perimeter would open the room to the void beyond it. `compile::exits`
carves a small solid-walled recess out of the host room's own wall instead —
the same recess construction `compile::doors` uses for a door portal's room
`b`, but with no second room on the far side — so only the near threshold
(front the room, back the alcove) is passable. A switch exit needs none of
this: `P_UseSpecialLine` fires from a raycast, not a crossing, so the exit
stays a normal solid one-sided wall.

**Every exit is tagged, even though neither `G_ExitLevel` nor
`G_SecretExitLevel` reads a tag.** Mirrors the existing precedent for manual
doors (above): uniform tagging keeps `tags::check_no_action_at_tag_zero` a
single exception-free invariant and the tag manifest complete.

## Sourcing rule — do not relax this

Every value in `data/engine.toml` and `data/vocabulary.toml` carries a `source`
citation, all against the id-Software DOOM release at pinned commit
`a77dfb96cb91780ca334d0d4cfd86957558007e0`. Textures were verified against the
Freedoom IWADs directly.

A wrong constant produces a map that loads, renders correctly, and is
unplayable. **No test can catch it, because the test reads the same table the
compiler does.** If a source is unreachable, leave the value unsourced and say
so — a reported gap beats a plausible guess.

Two engine facts worth keeping visible, both non-obvious and both found only by
reading the source:

- Vanilla triggers **use**-activated specials only from a line's front side
  (`P_UseSpecialLine`: "Only the front sides of lines are usable"). This is not
  true of walkover or shoot specials.
- `EV_Teleport` nonetheless begins `if (side == 1) return 0;` — a teleport line
  is walkover-triggered yet still front-side-only, contrary to the general rule.

## A note on the tests

Sixty-five passing tests once coexisted with four Critical geometry defects,
because every geometry fixture was two equal, flush, axis-aligned 256-unit
squares — including the four "sub-cases" of the orientation test, which were the
same rectangle rotated four ways.

Fixture **diversity** caught what mutation testing on the existing fixtures
could not. When adding a rule here, add a fixture whose shape differs from the
ones already present, and prove the test fails against a deliberately broken
implementation before trusting it.
