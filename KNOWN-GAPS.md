# crustygen — known gaps and carried decisions

State as of the compiler's completion: IR → validated UDMF `TEXTMAP` → PWAD →
reassembles through crustywad. 184 tests (177 lib + 1 first_map + 5
golden_textmap + 1 walking_skeleton). This file records what is deliberately
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
sidedef record unreferenced. Confirmed directly: the compiled map carries 93
sidedef records but only 91 are ever named by a linedef's `front`/`back` (up
from 81/79 before the door-thickness/alcove redesign added more sidedefs to
the map's two door chains — the two orphaned records are still exactly
`armory`'s own two end-to-end-consumed walls, unaffected by that change).

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
between them is. A `PortalKind::Plain` portal fills that gap with a single
new sector (`compile::portals::emit_gap_sector`): an open, walkable passage,
two threshold lines (room `a` <-> new sector, new sector <-> room `b`) and two
one-sided jambs closing the gap's long sides, front bound to the new sector
with solid rock behind. Neither room's own declared footprint is ever
touched. Swapping `a` and `b` on a portal no longer physically relocates
anything — the gap is filled identically regardless of which room is named
first — though `at`'s convention (anchored to room `a`'s wall) still means
the two labels are not *interchangeable* without also updating `at`. See
`ir::Portal`'s doc comment and the wall-thickness report
(`.superpowers/sdd/2026-08-09-crustygen-compiler/wall-thickness-report.md`)
for the full derivation and worked coordinates. A door portal fills the same
gap differently — see the next entry.

**A door portal's gap decomposes into a chain: an optional near alcove, the
door itself, an optional far alcove — the door-thickness/alcove model.**
Superseding this section's own earlier claim that "a door's depth is simply
the wall gap itself": a `PortalKind::Door`/`PortalKind::Locked` portal now
requires `Portal::door_thickness` (one of 8, 16, or 32 map units — see
`Ir::DOOR_DIMENSIONS`) and accepts two optional buffer sectors,
`Portal::alcove_near` (adjacent to room `a`'s wall) and `Portal::alcove_far`
(adjacent to room `b`'s), each from the same three-value set when present.
`Ir::from_json` requires the facing-wall gap to equal
`door_thickness + alcove_near + alcove_far` **exactly** — not merely "at
least", which the feature's own requester proposed and which is unsound: a
gap wider than the sum would leave a stretch of the corridor with no sector
to fill it, disconnecting whatever lies beyond the shortfall, since every
inch of the gap must belong to some emitted sector or the passage breaks.
`compile::doors::emit_doors` builds the chain as one to three
axis-aligned sectors in sequence (near alcove, door, far alcove — any
absent), each via `compile::portals::emit_segment`/`emit_jambs`/`emit_opening`
directly rather than through `emit_gap_sector` (which only ever builds a
single segment spanning the *entire* gap, the shape `cut_portals` still uses
for a plain portal). Only the door segment's own two faces carry the door
special and its sector's tag; an alcove's two faces are a plain,
non-blocking passage exactly like a plain portal's own gap sector, and its
floor, ceiling, light, and floor/ceiling textures copy whichever real room it
directly borders (room `a` for the near alcove, room `b` for the far one) —
not `min`/`max`-blended the way the plain-portal passage sector is, since an
alcove borders only one real room, not two. An alcove's own walls (its
jambs) use the theme's new `trim` texture role (`STARGR2` for `tech_base`);
the door's own jambs — "the track" — use `door_track` (`DOORTRAK`) as
before, and are lower-unpegged by default so the texture stays anchored to
the floor as the door sector's ceiling animates open, now with an explicit
opt-out (`Portal::track_lower_unpegged: false`) — the door's own two faces
stay lower-unpegged unconditionally, since that setting only ever governed
the track. `compile::doors::validate_door_texture` additionally rejects a
theme whose `door` texture is not in `vocabulary.toml`'s curated (not
sourced — see that table's own leading comment)
`[door_texture_catalog]`. See the door-redesign report
(`.superpowers/sdd/2026-08-09-crustygen-compiler/door-redesign-report.md`)
for the full derivation, worked coordinates, and why "at least" was rejected
in favor of exact equality.

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

**`find_facing_span` returns the *first* matching span, not necessarily the
nearest one.** For a genuinely comb- or zigzag-shaped room, `facing_spans`
can return two spans that share the same `near` (room `a`'s own wall
coordinate and along-range) but different `far` values — two structurally
distinct walls of room `b`, one nearer and one farther, both legitimately
facing the same stretch of room `a`'s wall. `Vec::iter().find()`'s
first-match semantics mean whichever one `wall_edges(poly_b)` happens to
enumerate first wins, silently, with no signal to the author that a second,
equally valid candidate existed. This is deliberately left unresolved rather
than fixed: it is not a soundness bug — either candidate is a real, legal
facing wall, so whichever one is picked, the resulting gap sector is
structurally valid and (as of the sector-overlap check above) verified not
to collide with anything — only a *which of two valid walls did you mean*
ambiguity for the rare non-convex room shape that presents it. Resolving it
would need a policy decision (nearest wins? reject ambiguity outright?) that
the spec does not currently call for; flagging it here is the honest
alternative to guessing one.

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

**One deliberate, clearly-labeled exception: `vocabulary.toml`'s
`[door_texture_catalog]`.** Which texture names read as a door is an
asset-naming convention, not an engine constant and not derivable from one —
there is no `linuxdoom-1.10` table of "the door textures" to cite. That table
carries a `curated` field in place of `source` for exactly this reason, and
`Tables::is_door_texture`'s doc comment repeats the distinction at the call
site. Do not add a `source` field to it, and do not extend this exception to
anything that *is* sourceable.

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
