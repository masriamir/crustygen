# Gap geometry: how a portal becomes a passage, and a door becomes a chain

Rooms are authored **apart**, never flush. Two rooms joined by a portal do not
share a wall coordinate: room `a`'s wall sits at one coordinate, room `b`'s
facing wall some real, solid distance beyond it, and the compiler fills that
void. This document carries the worked coordinates for both constructions,
which `KNOWN-GAPS.md` summarises as decisions but does not derive.

Everything here is compiler construction, not engine behavior. The engine
constants it obeys (`Ir::MIN_PORTAL_GAP`, the player's height, the door
clearance allowance) are sourced in `data/engine.toml`.

## The wall gap

`Ir::from_json` requires the gap between the two facing walls to be at least
`Ir::MIN_PORTAL_GAP` (8 map units) and a whole multiple of it. The gap is
validated through `geom::facing_spans`/`geom::find_facing_span` — the *same*
geometry `compile::portals::resolve_portal` later cuts through, so validation
and emission can never disagree about which wall pair a portal resolves to.

Note a 64-unit authoring grid cannot express an 8-unit gap. Use a finer
`grid` for tight walls rather than abandoning the model; `Portal::width` and
`Portal::at` are themselves exempt from the grid rule, because real doorways
are routinely finer than the grid their rooms sit on.

### Worked example — a plain portal

Two 256-unit square rooms, `a` spanning x ∈ [0, 256] and `b` spanning
x ∈ [320, 576], both y ∈ [0, 256]. A plain portal at `(256, 128)` with
`width: 128`.

| Quantity | Value | Where it comes from |
|---|---|---|
| `span.near` | x = 256 | room `a`'s own east wall |
| `span.far` | x = 320 | room `b`'s own west wall |
| gap | 64 | `far − near`, a multiple of 8 ✓ |
| `open_lo`, `open_hi` | y = 64, 192 | `at.y ± width/2` |

The compiler emits one passage sector spanning the whole gap, x ∈ [256, 320],
y ∈ [64, 192], plus four lines:

- two **thresholds** — `a` ↔ passage at x = 256, passage ↔ `b` at x = 320 —
  both two-sided and passable;
- two **jambs** — one-sided walls at y = 64 and y = 192, front bound to the
  passage with solid rock behind, closing the gap's long sides.

Each room's own wall is *split* at the opening rather than dropped and
recreated, so the surviving pieces run out to that wall's own endpoints, not
to the facing span's. That is what makes a portal work between rooms whose
walls are different lengths.

The passage sector takes `floor = max(floor_a, floor_b)` and
`ceiling = min(ceiling_a, ceiling_b)`, so it is the *inner* sector on both
counts. `CompileError::PortalNoHeadroom` rejects the portal when
`min(ceilings) − max(floors)` falls under the player's height — a
non-positive value means the sector would be inverted outright.

## The door chain

A door portal fills the same gap differently: an optional near alcove, the
door itself, an optional far alcove. `Ir::from_json` requires

```
door_thickness + alcove_near + alcove_far == gap
```

**exactly**, not "at least". A gap wider than the sum would leave a stretch of
corridor belonging to no sector, disconnecting whatever lies beyond the
shortfall — every inch of the gap must belong to something.

`door_thickness` and each alcove, when present, is one of 8, 16, or 32 units
(`Ir::DOOR_DIMENSIONS`).

### Worked example — a door with two alcoves

Same two rooms, but `b` spans x ∈ [320, 576] with a 64-unit gap, and the
portal is `kind: "door"` with `door_thickness: 32`, `alcove_near: 16`,
`alcove_far: 16`. The chain runs along the gap axis:

| Position | x | Boundary |
|---|---|---|
| `pos0` | 256 | room `a`'s wall → near alcove's outer threshold |
| `pos1` | 272 | near alcove → door (the door's near face) |
| `pos2` | 304 | door → far alcove (the door's far face) |
| `pos3` | 320 | far alcove → room `b`'s wall |

`pos3` always lands exactly on `span.far`, which the exact-sum rule
guarantees; a `debug_assert` pins it.

Three sectors are emitted, and each boundary is built by **exactly one** of
them. The door segment builds both of its own faces (`pos1` and `pos2`),
because neither is shared with a third segment; each alcove builds only its
own *outer* threshold (`pos0`, `pos3`) and its jambs. Building a shared
boundary from both sides would emit the same physical wall twice as two
coincident, overlapping linedefs.

### What each part wears, and why

| Surface | Texture | Reason |
|---|---|---|
| Door's two faces | theme `door` (`BIGDOOR2`) | the panel the player sees |
| Door's jambs — the track | theme `door_track` (`DOORTRAK`) | **always**, so a custom texture WAD has one intended knob |
| Alcove jambs, unlocked | theme `trim` (`SUPPORT3`) | flanking door trim |
| Alcove jambs, locked | the key trim (`DOORBLU`, …) | the trim a player faces walking up to the door |

The door's own faces are lower-unpegged unconditionally so the texture stays
anchored as the ceiling animates open. The track is lower-unpegged by default
with an explicit opt-out (`Portal::track_lower_unpegged`).

An alcove copies its floor, ceiling, light, and flats from whichever real
room it borders — room `a` for the near alcove, room `b` for the far one —
rather than the `min`/`max` blend a plain passage uses, because an alcove
borders only one real room.

A door sector's ceiling is snapped to its floor: that is a *closed* door, and
it is why a door sector can never be inverted and is exempt from the
headroom check that guards a plain passage.

## Two sharp edges worth knowing

**`facing_spans` has no distance bound.** Two walls "face" each other if they
run the same axis in opposite directions with overlapping along-ranges —
however far apart they are. A recessed wall (an L-shape's inner corner) can
genuinely face a room parked far away in the outward direction, even past a
nearer wall of the same room that a naive "closest wall" intuition would
expect to win.

**`find_facing_span` returns the first match, not the nearest.** For a comb-
or zigzag-shaped room, two structurally distinct walls of room `b` can both
face the same stretch of room `a`'s wall. Whichever `wall_edges` enumerates
first wins, silently. This is left unresolved rather than fixed: either
candidate is a real, legal facing wall, so the resulting gap sector is valid
either way and is verified not to collide with anything. Resolving it would
need a policy decision (nearest wins? reject ambiguity?) the spec does not
call for.
