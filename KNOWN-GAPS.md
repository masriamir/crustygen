# crustygen — known gaps and carried decisions

State as of the compiler's completion: IR → validated UDMF `TEXTMAP` → PWAD →
reassembles through crustywad. 164 tests. This file records what is deliberately
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

**The orphan-sidedef code path is exercised, not merely documented.** It
fires when an opening consumes a wall end to end, so the reusable sidedef
`split_wall_for_opening` sets aside is never taken. `tests/fixtures/entrada_base.json`
reaches it twice — its `armory` room is only 128 units tall along the wall its
`start`-facing portal opens (`width: 128` against a wall exactly 128 units
long), so the portal consumes that whole wall and leaves `armory`'s original
sidedef record unreferenced. Confirmed directly: the compiled map carries 81
sidedef records but only 79 are ever named by a linedef's `front`/`back`.

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

**Rooms are authored apart, not flush — the wall-thickness model.** Two
rooms connected by a portal never share a coincident wall coordinate; instead
`Portal::at`'s across-coordinate is read against room `a`'s own wall, and room
`b`'s facing wall sits some real, solid distance beyond it
(`Ir::MIN_PORTAL_GAP`, currently 8 map units, and the gap must be a whole
multiple of that). `Ir::from_json` validates the gap via
`geom::facing_spans`/`geom::find_facing_span` — the identical geometry
`compile::portals::resolve_portal` later cuts through, so the two can never
disagree about which wall pair a portal resolves to or how wide the gap
between them is. A portal — of any kind — fills that gap with a new sector
(`compile::portals::emit_gap_sector`): an open, walkable passage for
`PortalKind::Plain`, or a closed sector for `PortalKind::Door`/`PortalKind::Locked`,
built from the same shape either way — two threshold lines (room `a` <-> new
sector, new sector <-> room `b`) and two one-sided jambs closing the gap's
long sides, front bound to the new sector with solid rock behind. Neither
room's own declared footprint is ever touched, which is what supersedes the
old, asymmetric carve-into-`b` door construction this section used to
describe: a door's depth is now simply the wall gap itself (already
validated >= `MIN_PORTAL_GAP` at the IR boundary), not a separate
`DOOR_DEPTH` compiler constant carved out of room `b`'s interior. Swapping
`a` and `b` on a portal no longer physically relocates anything — the gap is
filled identically regardless of which room is named first — though `at`'s
convention (anchored to room `a`'s wall) still means the two labels are not
*interchangeable* without also updating `at`. See `ir::Portal`'s doc comment
and the wall-thickness report
(`.superpowers/sdd/2026-08-09-crustygen-compiler/wall-thickness-report.md`)
for the full derivation and worked coordinates.

**`facing_spans` has no distance bound, which can surprise an author moving a
room away from a fixture that used to be adjacent.** Two walls "face" each
other if they run the same axis, opposite directions, with overlapping
along-ranges — regardless of how far apart they are. A room's *recessed*
wall (an L-shape's inner corner, say) can genuinely face a second room parked
far away in the outward direction, even past another, nearer wall of the same
first room that a naive "closest wall" intuition would expect to win instead.
`compile::portals::tests::an_l_shaped_room_is_not_adjacent_where_it_has_no_wall`
pins exactly this: relocating that fixture's room `b` outward without also
moving it clear of the recessed wall's own along-range turned the intended
`NotAdjacent` case into a `PortalOffWall` one instead (a real facing span
exists between the two rooms, just not at the requested coordinate) — caught
only by re-deriving the geometry by hand, not by intuition.

**Portal `width` and `at` are exempt from the grid rule** that binds footprints.
Real doorways are routinely finer than the 64-unit grid their rooms sit on.

**Tag 0 is rejected on any action line.** It is not "no tag" — it is the tag
every untagged sector already carries, so an action left at zero matches every
untagged sector in the map. One stray zero opens every door.

**Portals and exits on diagonal walls remain unsupported by design, not by
omission.** `wall_edges` (and so `facing_spans`) only ever reported
axis-aligned edges, so a portal or exit requested on a genuinely diagonal wall
used to fall through to `NotAdjacent`/`PortalOffWall`/`ExitOffWall` — messages
that read as "there is no wall here" for a wall an author can plainly see.
`resolve_portal` and `resolve_exit` now check `geom::on_diagonal_wall`
before returning those errors and raise `CompileError::PortalOnDiagonalWall`/
`ExitOnDiagonalWall` instead, naming the exact coordinate. Supporting a portal
or exit *on* a diagonal wall properly would need a wall model wider than
`(axis, fixed coordinate)`, which the opening-splitting, jamb, and recess
machinery all assume; a chamfered room with its portals and doors on its
square walls (the common real case) already works today, both proved by
`portals::tests::a_portal_works_on_the_axis_aligned_wall_of_a_diagonally_shaped_room`
and the equivalent exit/door fixtures.

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
the same near-threshold-plus-solid-sides shape `compile::portals::emit_gap_sector`
builds for a two-room gap sector, just with no second room on the far side (a
solid wall instead of a second threshold) — so only the near threshold (front
the room, back the alcove) is passable. A switch exit needs none of this:
`P_UseSpecialLine` fires from a raycast, not a crossing, so the exit stays a
normal solid one-sided wall.

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
