# What verticality actually means in vanilla Doom — a corpus measurement

**Date:** 2026-08-09
**Corpus:** `RETAIL/DOOM.WAD`, `RETAIL/DOOM2.WAD`, `RETAIL/TNT.WAD`, `RETAIL/PLUTONIA.WAD`
**Sample:** 132 maps, 154,365 linedefs, 67,256 two-sided. **Zero assembly failures.**
**Method:** `crustywad::Map::assemble` over every map group in each IWAD; statistics
over the assembled graph. The program was a throwaway; the method below is complete enough to reproduce it
against the `crustywad` crate.

> **Status of these numbers: measured practice, not sourced engine fact.**
> They describe what four id/Final Doom IWADs from 1993–1996 actually do. They are
> evidence about idiom and about which of crustygen's rules match reality — they are
> **not** engine constants and must never be added to `data/engine.toml` beside values
> cited to `linuxdoom-1.10`. Every genuine engine fact this report leans on is cited
> separately below, against the project's pinned commit
> `a77dfb96cb91780ca334d0d4cfd86957558007e0`.

## Engine facts the measurement depends on

Each was read from the pinned source before the program was written, because the
statistics are meaningless if the "which side does the engine draw" question is
answered from memory.

- **`doomdata.h`** — `ML_BLOCKING 1`, `ML_BLOCKMONSTERS 2`, `ML_TWOSIDED 4`,
  `ML_DONTPEGTOP 8`, `ML_DONTPEGBOTTOM 16`, `ML_SECRET 32`, `ML_SOUNDBLOCK 64`,
  `ML_DONTDRAW 128`, `ML_MAPPED 256`.
- **`r_segs.c`, `R_StoreWallRange`** — the bottom (lower) texture is drawn
  `if (worldlow > worldbottom)`, i.e. when the **back** sector's floor is higher than
  the **front** sector's. So the visible lower is the sidedef **whose own sector is the
  lower one**. The top (upper) texture is drawn `if (worldhigh < worldtop)` — visible on
  the sidedef whose own sector has the **higher ceiling**.
- **`r_segs.c`** — sky special case: when
  `frontsector->ceilingpic == skyflatnum && backsector->ceilingpic == skyflatnum`, the
  code sets `worldtop = worldhigh`, collapsing the upper away entirely.
- **`p_map.c`, `P_TryMove`** — `tmfloorz - thing->z > 24*FRACUNIT` rejects the move.
  This caps stepping **up** only; falling is unrestricted.

## Finding 1 — crustygen's P1 contradicts the corpus

`rules.rs` computes `(a.floor - b.floor).abs()` and rejects anything over 24. Measured
against passable (`ML_BLOCKING` clear) two-sided lines:

| `\|delta\|` | passable | %      | blocking | %      |
|-------------|----------|--------|----------|--------|
| 0           | 21,907   | 33.63% | 454      | 21.39% |
| 1–8         | 5,718    | 8.78%  | 61       | 2.87%  |
| 9–16        | 9,383    | 14.41% | 133      | 6.27%  |
| 17–24       | 3,522    | 5.41%  | 208      | 9.80%  |
| 25–32       | 4,082    | 6.27%  | 123      | 5.80%  |
| 33–48       | 1,988    | 3.05%  | 103      | 4.85%  |
| 49–64       | 3,640    | 5.59%  | 233      | 10.98% |
| 65–128      | 9,221    | 14.16% | 363      | 17.11% |
| >128        | 5,673    | 8.71%  | 444      | 20.92% |

**24,604 of 65,134 passable two-sided lines (37.77%) exceed the 24-unit cap.**
Restricted to lines that have any height change at all, it is **56.92%** — the majority
of height changes in vanilla Doom are larger than one step. The largest is **2,200
units**, in `PLUTONIA.WAD:MAP02`.

### The dynamic-geometry confound, ruled out

A large delta could be a lift or a raising floor measured in its lowered state, which
would make the drop temporary rather than a real one-way descent. Using "either
bordering sector carries a nonzero tag" as the proxy for dynamic geometry:

- **9,225 (37.5%)** of the over-step lines border a tagged sector.
- **15,379 (62.5%)** border **no** tagged sector — permanent, static drops.
- The largest static drop is the same **2,200 units**.

### Consistency across the four IWADs

Not one WAD's idiom, and not a Final Doom artifact:

| WAD          | passable lines | over-step | %      | static share |
|--------------|----------------|-----------|--------|--------------|
| DOOM.WAD     | 10,386         | 3,251     | 31.30% | 60.8%        |
| DOOM2.WAD    | 10,106         | 3,967     | 39.25% | 58.8%        |
| TNT.WAD      | 25,457         | 9,508     | 37.35% | 66.0%        |
| PLUTONIA.WAD | 19,185         | 7,878     | 41.06% | 60.8%        |

**Conclusion.** The engine caps the *climb*, not the *fall*. P1's symmetric `abs()`
rejects the ledge-and-drop construction that is the dominant form of verticality in
vanilla Doom. It cannot stand as written.

## Finding 2 — the texture goes on the visible side; the other side is usually bare

Counting only the side `r_segs.c` actually draws:

| case                   | lines  | visible side set | hidden side set | neither     |
|------------------------|--------|------------------|-----------------|-------------|
| floors differ (lower)  | 44,895 | 44,246 (98.6%)   | 4,724 (10.5%)   | 566 (1.3%)  |
| ceilings differ (upper)| 31,059 | 30,056 (96.8%)   | 3,067 (9.9%)    | 894 (2.9%)  |

The visible-side rate is 97.7–99.4% in every WAD. The hidden-side rate collapses over
time — DOOM 29.3%, DOOM2 14.9%, TNT 7.6%, PLUTONIA 1.8% — as mappers learned the engine
never reads it.

**Conclusion.** crustygen's P8 requires the texture on **both** sides
(`front.lower.is_empty() || back.lower.is_empty()` ⇒ violation). Only 10.5% of real
height-change lines satisfy that, so P8 as written would reject roughly nine out of ten
of vanilla Doom's own boundaries. P8 must become directional.

## Finding 3 — step faces reuse wall textures rather than a step family

Top visible **lower** textures: `METAL` 2,573 · `SUPPORT3` 2,352 · `METAL2` 1,876 ·
`GSTONE1` 1,192 · `BSTONE2` 994 · `ROCKRED1` 929 · `COMPSPAN` 907 · `BROWN1` 746 ·
`STEPLAD1` 742 · `STONE4` 721 · `STONE6` 688 · `BROWN96` 625.

Top one-sided **middle** (plain wall) textures: `METAL2` 3,978 · `SUPPORT3` 3,886 ·
`METAL` 3,255 · `GSTONE1` 2,976 · `DOORTRAK` 1,915 · `BSTONE2` 1,912 · `BROWN1` 1,719 ·
`WOOD9` 1,565 · `ROCKRED1` 1,407 · `STONE2` 1,293 · `STONE6` 1,258 · `BRICK7` 1,177.

**Eight of twelve names appear on both lists** (`METAL`, `SUPPORT3`, `METAL2`,
`GSTONE1`, `BSTONE2`, `ROCKRED1`, `BROWN1`, `STONE6`). A dedicated step family exists
but is a minority (`STEPLAD1`, 742).

**Conclusion.** Sourcing a riser from the sector's own wall texture is the
evidence-backed default. A dedicated `riser` theme role would be inventing a convention
the corpus does not support, and would need a `curated` justification like
`[door_texture_catalog]`'s.

## Finding 4 — the sky exception is real and is the majority of absent uppers

Of the 1,003 lines where ceilings differ and the visible upper is missing:

- **605 (60.3%)** have `F_SKY1` on **both** ceilings — the `worldtop = worldhigh` case,
  where the engine draws no upper at all.
- **398 (39.7%)** are not both sky, i.e. genuine missing-texture defects or deliberate
  effects.

**Conclusion.** Any P8 that runs against a map with sky needs this carve-out. crustygen
emits no sky flat today, so the case is currently unreachable; this is recorded as a
prerequisite rather than implemented, so no unsourced constant is added for a branch no
fixture can enter.

## Finding 5 — step rise is quantized (parameterizes the stairs phase)

Passable lines with `|delta|` in 1..=24, by exact value:

| rise | count | share |
|------|-------|-------|
| 16   | 6,388 | 34.3% |
| 8    | 5,192 | 27.9% |
| 24   | 2,411 | 12.9% |
| 10   | 1,942 | 10.4% |
| 20   | 724   | 3.9%  |
| 15   | 324   | 1.7%  |
| 12   | 238   | 1.3%  |

Three values — 8, 16, 24 — account for 75.1% of all steps.

## Finding 6 — lifts are repeatable in practice

| special | form                   | count |
|---------|------------------------|-------|
| 10      | W1 walkover, one-shot  | 1     |
| 21      | S1 switch, one-shot    | 1     |
| 62      | SR switch, repeatable  | 609   |
| 88      | WR walkover, repeatable| 247   |

**856 repeatable against 2 one-shot.** `KNOWN-GAPS.md` currently records the repeatable
choice as "a design choice, not a source fact"; it can now additionally cite measured
practice, which agrees overwhelmingly.

## What this changes in crustygen

1. **P1 retired** as a floor-delta cap, replaced by a structural rejection of an
   inverted gap sector (`max(floors) >= min(ceilings)`) and a P2 headroom check on the
   passage's vertical overlap.
2. **P8 made directional** — the texture is required only where `r_segs.c` draws it.
3. **Riser textures** sourced from the sector's own wall texture.
4. **Sky carve-out** for P8 recorded as a prerequisite, not implemented.
5. **Stairs** (later phase) default to a 16-unit rise, with 8 and 24 as the other
   idiomatic values.
6. **Lifts** (later phase) keep specials 62 and 88.

The one-way-drop hazard that (1) admits is real and is **not** solved here: a descent
over 24 units is a one-way connection, and nothing yet verifies the player can still
finish the map. That belongs to P7's key-aware reachability flood, which remains
deferred.
