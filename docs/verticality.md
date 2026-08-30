# crustygen verticality — design

**Date:** 2026-08-09
**Status:** approved, ready for an implementation plan
**Scope of this document:** phase 1 (height differences) in full, plus a sketch of
phases 2 and 3 sufficient to keep phase 1 from boxing them in.

## Problem

Every map crustygen produces is flat. `compile::portals::emit_opening` creates both of
a threshold's sidedefs with `upper`, `middle`, and `lower` all empty
(`portals.rs:316–328`), and nothing fills them in afterward. Rule **P8**
(`rules.rs:152–193`) rejects any two-sided line whose sectors' floors or ceilings differ
without the corresponding texture, so any authored height difference fails the compile.
Rule **P1** (`rules.rs:68–74`) independently rejects any floor difference over 24 units
between portal-connected rooms.

The geometry is already shaped for height differences — a plain portal's gap sector
takes `floor: max(a, b)` and `ceiling: min(a, b)` (`portals.rs:132–140`). Only the
textures and the rules are missing.

## Evidence

Design decisions here are grounded in a corpus measurement over `DOOM.WAD`,
`DOOM2.WAD`, `TNT.WAD`, and `PLUTONIA.WAD` — 132 maps, 154,365 linedefs, 67,256
two-sided, zero assembly failures. Full write-up, method, and the preserved program:
`docs/measurements/verticality-corpus.md`.

Those numbers are **measured practice, not sourced engine fact**. They describe what
four 1990s IWADs do. They must never be added to `data/engine.toml` beside values cited
to `linuxdoom-1.10`. The genuine engine facts this design rests on are cited
individually below, all against the pinned commit
`a77dfb96cb91780ca334d0d4cfd86957558007e0`.

The four results that drive the design:

1. **37.77%** of passable two-sided lines exceed the 24-unit cap P1 enforces (24,604 of
   65,134); **56.92%** of lines that have any height change at all. **62.5%** of those
   over-step lines border no tagged sector, so they are permanent static drops, not
   lifts caught in a lowered state. Largest: 2,200 units. Consistent across all four
   IWADs (31.3%–41.1%).
2. Where floors differ, the side the engine draws is textured **98.6%** of the time; the
   other side only **10.5%**. P8 currently demands both.
3. **Eight of the twelve** most common step-face textures are also among the twelve most
   common plain-wall textures. A dedicated step family exists but is a minority
   (`STEPLAD1`, 742 uses).
4. Of the 1,003 boundaries where ceilings differ and the visible upper is absent, **60.3%**
   are sky-to-sky — i.e. legitimately absent; the remaining 39.7% are genuine missing-texture
   defects.

## The arc

**Phase 1 — height differences (this document).** Rooms may sit at any floor and ceiling
heights whose vertical ranges overlap enough to walk through. Every emitted boundary
carries the textures the engine draws. No new IR fields beyond a range guard.

**Phase 2 — stairs.** A portal gap is filled by a chain of N step sectors rather than
one, each rise within the engine's step limit, so a drop becomes climbable. This is the
door-chain machinery (`compile::doors::emit_doors` builds one to three segments via
`portals::emit_segment`) with different per-segment properties. Corpus-measured
defaults: 16 units (34.3%), 8 (27.9%), 24 (12.9%) — together 75.1% of all steps.

**Phase 3 — lifts. Shipped.** A `downWaitUpStay` platform fills the portal gap, tagged and
carrying **62** (SR switch) or **88** (WR walkover), or their `blazeDWUS` twins **123**/**120** —
all four sourced in `data/vocabulary.toml` `[specials.lift]`. The construction is what this
sketch predicted; the trigger rule is not. `EV_DoPlat` sends a platform to the lowest floor
among its two-sided neighbors and brings it back, so what a lift needs is a trigger on the *low*
side, and a second one on top is optional because the descent is free. `PortalKind::Lift`
therefore carries a `trigger` of `switch`, `walkover` or `both_ends` instead of always emitting
both sides, and the same platform serves two further shapes: a **barrier** (two rooms at one
floor with the platform risen between them) and a **pedestal** (a raised island inside one room,
in the IR's own `pedestals` list). P5 is re-derived from the emitted geometry in
`rules::check_lift_return`, and again at layer 4 as `V-P5`. Shapes, triggers, rest positions and
riser rendering were measured over DOOM/DOOM2, Final Doom and an idgames sample before any of it
was designed: `docs/measurements/lift-shapes-2026-08-29.md`.

**What phases 2 and 3 inherit from phase 1**, and therefore what phase 1 must get right:

- the visible-side texture rule;
- riser textures sourced from a sector's own wall texture — which means **every
  compiler-made sector must carry a wall texture of its own**, since a phase-2 step
  sector and a phase-3 plat belong to no room;
- the retirement of P1's symmetric cap;
- the vertical-overlap invariant.

## Engine facts this design depends on

- **`r_segs.c`, `R_StoreWallRange`** — the bottom (lower) texture is drawn
  `if (worldlow > worldbottom)`, i.e. when the **back** sector's floor is higher than the
  **front** sector's. The visible lower is therefore on the sidedef **whose own sector is
  the lower one**. The top (upper) texture is drawn `if (worldhigh < worldtop)` — visible
  on the sidedef whose own sector has the **higher ceiling**.
- **`r_segs.c`** — when both sectors' `ceilingpic == skyflatnum`, the code sets
  `worldtop = worldhigh`, so no upper is drawn at all.
- **`p_map.c`, `P_TryMove`** — `tmfloorz - thing->z > 24*FRACUNIT` rejects the move. The
  24-unit limit is a cap on stepping **up** only; falling is unrestricted. Already
  sourced in `data/engine.toml` as `max_step_height`.
- **`doomdata.h`** — `ML_BLOCKING 1`, `ML_TWOSIDED 4`, `ML_DONTPEGTOP 8`,
  `ML_DONTPEGBOTTOM 16`.

## Design

### 1. A single texture pass, not threaded parameters

A new module `compile::heights` provides:

```rust
pub(crate) fn apply_height_textures(data: &mut MapData)
```

It walks every linedef in `data.linedefs` that has a `back`, compares the two bordering
`SectorOut`s, and writes the visible side's texture:

- if the back sector's floor is higher than the front's, the **front** sidedef's `lower`
  is the visible one; if lower, the **back** sidedef's; if the two floors are equal,
  neither is drawn and no lower is written at all;
- if the back sector's ceiling is lower than the front's, the **front** sidedef's
  `upper` is visible; if higher, the **back** sidedef's; equal ceilings write no upper;
- the texture written is the **wall texture of the sector that sidedef belongs to**;
- the opposite side is left untouched.

**Only empty slots are filled.** `compile::doors::emit_doors` already writes the theme's
door texture into both door faces' `upper` (`doors.rs:305–306`); an unconditional pass
would overwrite it with a wall texture. Filling only what is empty preserves the door
panel and still covers everything no earlier pass claimed.

**Why a post-pass rather than parameters on the emitters.** `emit_opening`,
`emit_segment`, `emit_jambs`, and `emit_gap_sector` already carry
`#[expect(clippy::too_many_arguments)]`, so adding texture parameters worsens a known
strain. More importantly, this gap exists precisely because each emitter had to remember
independently — a single pass makes forgetting impossible, and phases 2 and 3 get
correct risers with no further work.

**Ordering.** `apply_height_textures` runs in `compile::compile_reporting` immediately
after `sectors::check_no_sector_overlaps` — the point at which geometry is final — and
before `textmap::emit_textmap`. The numbered pass list in `compile`'s doc comment is
renumbered accordingly.

### 2. `SectorOut` gains a wall texture

```rust
pub struct SectorOut {
    // ... existing fields unchanged ...
    /// The wall texture this sector's faces use.
    ///
    /// Not emitted to `TEXTMAP` — a Doom sector has no wall texture. It is
    /// recorded here so `compile::heights::apply_height_textures` can source
    /// the riser a height difference exposes, including for sectors the
    /// compiler creates itself, which belong to no room.
    pub wall_tex: String,
}
```

Set at creation, in every existing site:

| sector | `wall_tex` |
|---|---|
| room (`sectors::emit_sectors`) | that room's `Room::wall_tex` |
| plain gap (`portals::cut_portals`) | room `a`'s — already its jamb texture, `portals.rs:149` |
| door (`doors::emit_doors`) | room `a`'s, matching the plain gap's convention |
| near alcove | room `a`'s, matching how it already inherits flats and light |
| far alcove | room `b`'s, likewise |
| exit alcove (`exits::emit_exits`) | its host room's |

**Alternative considered and rejected:** a `Vec<String>` in `MapData` parallel to
`sectors`. It keeps a non-emitted value off the emitted-record struct, but it can
desync from the vector it shadows; a field cannot.

### 3. P1 is retired, replaced by one invariant

`rules::check_step_height`'s P1 branch (`rules.rs:68–74`) is deleted. Its premise —
"connected rooms must not differ by more than one step" — is contradicted by 24,604
passable lines in the corpus, 15,379 of them static.

Deleting it alone would be wrong, because it was incidentally preventing a real
degeneracy: with `floor: max(a, b)` and `ceiling: min(a, b)`, two rooms whose vertical
ranges do not overlap produce a gap sector whose floor sits at or above its own ceiling.
So P1 is replaced by:

```rust
/// A plain portal's two rooms do not overlap vertically by enough for the
/// player to pass through the gap sector between them.
///
/// `have` is `min(ceilings) - max(floors)`, the gap sector's own headroom; a
/// non-positive value means the sector would be inverted outright.
#[error("portal `{a}` <-> `{b}` has {have} units of headroom but the player needs {need}")]
PortalNoHeadroom { a: String, b: String, have: i32, need: i32 },
```

raised in `portals::cut_portals`, inside the existing `if portal.kind ==
PortalKind::Plain` branch where the gap `SectorOut` is built, when
`min(ceiling_a, ceiling_b) - max(floor_a, floor_b) < tables.player().height`.

`cut_portals` gains a `tables: &Tables` parameter to read the player's height, matching
`doors::emit_doors` and `exits::emit_exits`, which already take one.

**One error rather than two.** A separate "ranges are disjoint" variant was considered.
It is not worth the branch: "has -272 units of headroom but the player needs 56" names
the problem and its magnitude in both the inverted and the merely-too-short case.

**Plain portals only.** A door portal never produces an inverted sector — its door
sector's ceiling is deliberately snapped to its floor (`doors.rs:217–221`, a closed
door) and its alcoves copy a real room's own floor and ceiling. Door portals are already
covered by **P4**, which computes `min(ceilings) - door_clearance_allowance -
max(floors)` and rejects an opening below the player's height, a strictly tighter bound.
Applying the new check to doors as well would double-report the same defect.

With its P1 branch gone, `rules::check_step_height` contains only the P4 door check and
is renamed **`check_door_clearance`**. `Tables::step_height` keeps its only caller in
phase 2.

**The hazard this admits, stated plainly.** A descent over 24 units is now a genuinely
one-way connection: the player can fall in and cannot climb out. Nothing verifies the
map remains completable. That is **P7** (no softlock), which needs the key-aware
reachability flood `KNOWN-GAPS.md` already records as absent. Phase 1 records the new
one-way-drop hazard against P7 rather than pretending it is solved.

### 4. P8 becomes directional

`rules::check_missing_textures` currently raises a violation when *either* side lacks
the texture. Per `r_segs.c` it becomes:

- floors differ ⇒ a **lower** is required on the sidedef whose own sector has the
  **lower floor**; the other side is unconstrained;
- ceilings differ ⇒ an **upper** is required on the sidedef whose own sector has the
  **higher ceiling**; the other side is unconstrained;
- one-sided lines still require a `middle`, unchanged.

The corpus puts the current both-sides demand at odds with 89.5% of vanilla Doom's own
height-change boundaries.

### 5. The sky carve-out is recorded, not implemented

Sky-to-sky boundaries account for 60.3% of the corpus's *absent* uppers — legitimately so —
and `r_segs.c`'s `worldtop = worldhigh` makes them genuinely exempt. crustygen emits no sky
flat, so no fixture can reach the case, and implementing it now would mean adding an
unsourced flat name to satisfy an unreachable branch. It is recorded in
`KNOWN-GAPS.md` as a prerequisite for whenever sky arrives.

### 6. The `i16` range guard already exists — no work needed

An earlier draft of this section proposed a new `IrError::HeightOutOfRange` for the
`i16` limit the Doom-format twin imposes on sector planes. **That guard is already
implemented and tested**, and the draft was wrong to call it new:

- `IrError::HeightOutOfRange { room, height, min, max }` is defined at `ir.rs:358–370`
  and raised by `Ir::from_json` at `ir.rs:630`.
- It is pinned by a test at `ir.rs:1275` asserting a height of 40,000 is rejected.
- `IrError::InvertedRoom { room, floor, ceiling }` (`ir.rs:348–357`, raised at
  `ir.rs:617`) separately rejects any single room whose ceiling is at or below its
  floor.

So unbounded authored heights are already bounded, and a single room can already never
be inverted. `PortalNoHeadroom` covers the case neither of these reaches: the
*compiler-created* gap sector between two individually-valid rooms.

The only thing worth adding is a positive fixture — a large drop *inside* the range that
now compiles — since the existing test only pins the rejection, and until now any large
drop was rejected by P1 before the range guard could matter.

## Testing

The codified lesson from this project is that fixture **diversity**, not mutation
coverage on existing fixtures, is what catches geometry defects: 65 passing tests once
coexisted with four Critical bugs because every fixture was the same square rotated.

1. **Ledge drop** — rooms 128 units apart: the visible lower lands on the low side; the
   hidden side stays empty.
2. **Orientation swap** — fixture 1 with rooms `a` and `b` exchanged, so the lower room
   is the other one. This is the direct analogue of the four-rotations trap and is the
   test that proves the visible-side rule is not hard-coded to one side.
3. **Ceiling-only difference** — an upper appears on the higher-ceiling side; no lower
   anywhere.
4. **Floors and ceilings both differ** — the upper and lower land on *opposite*
   sidedefs, a case fixtures 1–3 cannot expose.
5. **Range bounds** — a large drop *inside* the `i16` range compiles. The rejection half
   is already pinned at `ir.rs:1275`; what is missing is the positive case, which until
   now P1 rejected before the range guard could ever matter.
6. **Disjoint ranges** — room A at floor 0/ceiling 128, room B at floor 400/ceiling 512
   raises `PortalNoHeadroom` with a negative `have`.
7. **Headroom boundary** — an overlap of exactly the player's height compiles; one unit
   less raises `PortalNoHeadroom`. Pinned on both sides so an off-by-one cannot pass.
8. **Door across a height difference** — the **door sector's** own face gains a lower (the
   door sector takes `min(floors)`, so it is the lower side of its far threshold, not the
   higher room), and the door's own theme door texture survives the pass untouched. This is
   also the one place the compiler reaches the `back` arm of the visible-side rule through
   the real pipeline.
9. **Mutation proofs** — placing the texture only on the hidden side must fail P8;
   removing the fill-if-empty guard must break fixture 8. Both must be demonstrated
   failing against a deliberately broken implementation before the tests are trusted.
10. **Goldens and the authored map** — regenerate the `golden_textmap` fixtures, rebuild
    `tests/fixtures/entrada_base.json` with real height variation, re-verify the
    crustywad round trip, and re-playtest.

## Out of scope

Stairs, lifts, sky, P7 reachability, and texture alignment (sidedef `x_offset`) are all
out of scope for phase 1. Alignment in particular is a separate known gap: Doom derives
a texture's horizontal position from `x_offset` plus distance along the line, so offsets
must accumulate across a run of collinear linedefs rather than centering each piece.

## Gaps this phase creates

Both are recorded in `KNOWN-GAPS.md` as part of the implementation, not left implicit:

1. **One-way drops are unverified.** A descent over the step limit cannot be climbed,
   and no reachability check exists. Belongs to P7.
2. **P8 has no sky exception.** Unreachable today because crustygen emits no sky flat;
   required before sky is added.
