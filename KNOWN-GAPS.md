# crustygen — known gaps and carried decisions

State as of the compiler's completion: IR → validated UDMF `TEXTMAP` → PWAD →
reassembles through crustywad. 100 tests. This file records what is deliberately
absent, what is known-fragile, and the decisions a future contributor would
otherwise have to re-derive.

## Not implemented, by design

The compiler covers structural invariants S1–S6 and playability rules P1, P3,
P4, P8, P9, P11, P13, P14, P19, P24, P25.

Deliberately absent, deferred to the next stage: **P5** (lifts), **P6** (monster
mobility), **P7** (no softlock), **P10** (clean vertical tiling), **P12** (sky
coherence), **P15** (teleport pairing), **P16**/**P17** (liquids and damage
survivability), **P18** (secret accounting), **P20** (pickup accessibility),
**P21** (light sources), **P22** (hanging decorations), **P23** (barrel
safety).

P7 and P20 both need a key-aware reachability flood that does not exist yet.

**P2 (headroom) is only partially covered.** `compile::things::place_things`
rejects a placed thing whose room lacks headroom for it (`NoHeadroom`), which
is a real per-thing check, not a stub — but it only runs where the IR places a
thing. A room with no things in it gets no headroom check at all, so P2's
literal scope ("a walkable sector's ceiling-minus-floor gap must be at least
the height of the tallest thing required to occupy or pass through it — the
player is always in that set") is not fully enforced: an empty corridor too
short for the player to stand in would compile cleanly today. P25 (start
clearance) *is* fully covered by the same code path, since every player start
is itself a thing and always goes through the identical clearance/headroom/
overlap checks.

Also absent: the packer, the verifier, the conformance report, the blank
Markdown template, and any authored map. Specials for lifts, teleports, exits
and sector effects, monster `spawnhealth`, health/armor pickup amounts and
caps, the gore prop set, and the `ML_BLOCKMONSTERS`/`ML_SOUNDBLOCK` linedef
flags are all **sourced and accessible** but nothing emits any of them yet —
only doors are wired end to end.

## Known gaps

**No fixture anywhere has a 45-degree edge.** The design spec permits diagonal
footprints, and two documented limitations rest on diagonal behavior:
`shared_spans` cannot host a portal on a diagonal wall, and `depth_behind_wall`
skips diagonal edges entirely — a room with a chamfered far side reports
`DoorTooDeep { available: 0 }`, which reads as nonsense to an author. This is
the next shape-space hole. Every previous one contained real defects.

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

**Lifts and teleports are repeatable, not one-shot.** A design choice, not a
source fact: P5 requires a lift be operable from both ends, and a one-shot lift
can strand a player. Recorded in the citations; disagree there if you prefer.

**Odd portal widths are rejected rather than rounded**, per the spec's
reject-don't-degrade posture.

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
