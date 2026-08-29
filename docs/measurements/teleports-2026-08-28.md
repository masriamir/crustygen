# Teleports — the corpus before and after the pad construct

**Date:** 2026-08-28
**Tool:** `crustygen-corpus` (before: commit `dbbd950`; after: commit `6229afe`), crustywad 0.9.6
**Sample:** the same sample of record — crustywad `xtask harvest-sample --seed 20260828 --count 400`
→ 374 archives on disk, 1,282 unique maps
**Companion:** [`expressibility-2026-08-28.md`](expressibility-2026-08-28.md), whose numbers are
the "before" column throughout

## Purpose

Project G grows crustygen's vocabulary one construct at a time and re-measures the corpus after
each. The predecessor run named the next construct from its own blocker table: linedef special
**97** blocked 723 of 1,282 unique maps (56.4 %), more than any other value, and the greedy curve
said 97 alone moved the conjunction from 6.5 % to 7.6 %. This document records what the teleport
construct — IR pads and destinations, a compiler pass, three playability rules, two verifier
checks, and a lifter recognizer — actually bought, and what the corpus does with teleports that
the construct deliberately does not model.

Two numbers are reported for the conjunction, and the difference between them is the point:

- the **naïve** figure counts a map expressible when its line specials, sector specials and thing
  types are all in the emittable vocabulary — the three-axis test the predecessor ran, now with
  the four teleport specials inside the line set;
- the **honest** figure additionally requires every teleport line in the map to be one the
  recognizer can state. Adding 97 to the vocabulary without that gate would claim maps whose
  teleports crustygen cannot express.

## Sample and method

Identical population and gate to the 2026-08-28 run, so the two are directly comparable on every
axis except where this document says otherwise:

| Field | Value |
|---|---|
| seed | `20260828` |
| count | 400 |
| frame rows | 15273 |
| fetch list | `blake3:f3e6453f505ecbed25d39c509903267d00b4b9fba6e7663b3055eac2f6c8759b` |
| entries `ok` / `failed` | 374 / 26 |
| archives opened | 371 |
| WAD members read | 403 |
| raw map groups | 1,285 |
| **unique maps** | **1,282** |

Every `*.zip` opens through crustywad's archive reader under lenient options (CRCs still
verified); every `.wad` member is read; every map group passes crustygen's shared strict
`ingest::load_map`; maps are deduplicated by a `sha256:` hash over their lumps. The sweep exits 1
by design — a real idgames sample always carries load failures — and wrote **419 stderr lines**,
the same count both 2026-08-28 runs wrote. The load-failure buckets are unchanged
(235 `unsupported_format`, 117 `assembly_refused`, 35 `no_maps`, 23 `textmap_unparseable`, 6
`wad_unreadable`, 3 `archive_unreadable`); the earlier document breaks them down class by class
and nothing here revises it.

> **Status of these numbers: measured practice, not engine fact — and still an upper bound.**
> Membership on the three special/thing axes reads nothing but numeric sets. The teleport axis is
> the first that reads *geometry* — the recognizer resolves each teleport line the way
> `EV_Teleport` does, and classifies the shape of the sector the crosser enters — but everything
> else about a map (room shape, flags, tags outside teleports, texture names) is still unmeasured.
> A geometry-aware lifter can only do worse than this bound, never better. The 374-of-400 sample
> and the overcounted "unloadable" bucket carry over unchanged from the predecessor document; see
> its Sample provenance and Load failures sections.

## Before — 2026-08-28, after the decoration rows (commit `dbbd950`)

| Axis | All unique maps | Vanilla-only slice |
|---|---:|---:|
| line specials | 7.3 % | 9.3 % |
| sector specials | 60.8 % | 63.1 % |
| thing kinds | 74.5 % | 81.3 % |
| **all three** | **6.5 %** | **8.3 %** |

Vanilla-only slice: 77.7 % of unique maps (996 maps). The all-three figure is 83 maps.

Line-special blocker ranking, top of the table:

| Value | Maps | Share |
|---|---:|---:|
| 97 | 723 | 56.4 % |
| 62 | 616 | 48.0 % |
| 23 | 386 | 30.1 % |
| 109 | 331 | 25.8 % |
| 103 | 329 | 25.7 % |
| 88 | 326 | 25.4 % |

## After — the teleport construct (commit `6229afe`)

| Axis | All unique maps | Vanilla-only slice |
|---|---:|---:|
| line specials | 9.0 % | 11.6 % |
| sector specials | 60.8 % | 63.1 % |
| thing kinds | 74.5 % | 81.3 % |
| teleport lines | 80.8 % | 82.9 % |
| **all axes** | **7.5 %** | **9.6 %** |

Vanilla-only slice: 77.7 % of unique maps (996), unchanged — the slice is defined by
`engine.toml`'s vanilla special list, which the vocabulary does not touch.

### Naïve versus honest

| Figure | Maps | Share of 1,282 | Share of the 996-map slice |
|---|---:|---:|---:|
| before, three axes (`dbbd950`) | 83 | 6.5 % | 8.3 % |
| after, three axes, teleport refusals ignored (**naïve**) | 99 | 7.7 % | 9.9 % |
| after, four axes (**honest**) | **96** | **7.5 %** | **9.6 %** |

Three maps separate the two after-figures: they carry only emittable specials and thing types,
and a teleport line the recognizer refuses. The naïve figure is the one a vocabulary table alone
would report; the honest one is what the instrument reports, and it is the number of record.

The sector and thing axes are unchanged **by construction** — the construct adds line specials
and a new axis, and `Vocabulary::classify` reads each axis from its own set. The line axis moved
7.3 % → 9.0 % (93 → 116 maps) because 97/39/126/125 left the blocker table; value 97 no longer
appears in it at all, and 62 is now the top blocker.

### The teleport axis and its refusals

| Measure | Value |
|---|---:|
| maps with ≥ 1 teleport line | 797 (62.2 % of 1,282) |
| maps with a refused line | 246 (30.9 % of the 797) |
| maps passing the axis | 1,036 (80.8 % of 1,282) |
| teleport lines seen | 20,491 |
| lines: player / monsters-only / one-shot | 15,148 / 5,343 / 953 |
| lines in a closet (front sector holds a monster) | 8,294 (542 maps) |
| lines delivering beside an exit | 169 (47 maps) |
| lines on a paired pad | 2,490 (238 maps) |
| geometry: island / alcove / boundary / other | 5,574 / 1,275 / 9,706 / 3,936 |
| ambiguous (several markers; reported, not refused) | 204 |

The axis has exactly two refusal classes, assigned in a fixed precedence — a line with no back
sector, or whose back sector *is* its front sector, is classified before its tag is resolved:

| Refusal | Lines | Share of 20,491 | Maps |
|---|---:|---:|---:|
| `self_referencing` (front sector == back sector) | 3,844 | 18.8 % | 202 |
| `broken` (tag 0, no tagged sector, no marker on the tag, or a one-sided line) | 748 | 3.7 % | 63 |
| **any refusal** | **4,592** | **22.4 %** | **246** |

19 maps carry both classes, so the map counts do not sum: 183 maps are refused for
self-referencing lines alone, 44 for broken lines alone, 19 for both.

Both classes are refusals rather than IR features on purpose. A self-referencing trigger — the
line's two sides in the same sector — is a mapping trick with no counterpart in id's designs
(round 2 below: 7 lines across DOOM+DOOM2's 657), and the IR has no way to state it that would
not misrepresent it. A `broken` line fires nothing at all.

### Greedy curves

| k | Line axis alone | Conjunction |
|---|---:|---:|
| 1 | 11.5 % | 9.3 % |
| 5 | 15.8 % | 11.6 % |
| 10 | 18.6 % | 13.6 % |
| 21 | 26.9 % | 18.0 % |
| 51 | 45.1 % | 25.8 % |

Line-axis order: 62 → 123 → 117 → 88 → 48 → 63 → 31 → 103 → 114 → 2 → 23 → 38 → 102 → 120 → 109
→ 112 → 118 → 32 → 33 → 34 → 71 → 19 → 18 → 46 → 20

Conjunction order: 62 → 123 → 117 → 88 → 63 → 48 → 114 → 103 → 2 → 120 → 109 → 112 → 118 → 32 →
23 → 38 → 31 → 33 → 102 → 34 → 36 → 271 → 255 → 242 → 6

**The conjunction curve is not comparable checkpoint-for-checkpoint with the 2026-08-28 run's.**
That curve walked the maps already clear on sector specials and thing kinds; this one walks the
maps already clear on sector specials, thing kinds **and** the teleport axis. It is a different
population, so a lower number here is not a regression and a higher one is not a gain — only the
within-run ordering is meaningful. The line-axis curve has the same problem in milder form: its
population is unchanged, but 97 has left the candidate set because it is now emittable, so the
picks are offset by one from the start.

## What the corpus does with teleports

Three throwaway probe rounds measured the shapes before any of this was built: rounds 1 and 2 over
the idgames sample and the retail IWADs, round 3 over the closet-release question. The probes are
deleted; their tables are reproduced here as numbers only, which is how they were written — **no
linedef special is named from memory anywhere below**, and every geometric "size" is a bounding
box over the endpoints of the sector's linedefs, never a true area.

Retail populations, used by rounds 2 and 3: `RETAIL/DOOM.WAD` + `RETAIL/DOOM2.WAD` is the
**headline** (68 map groups, all Doom-format, all assembled); `TNT.WAD` and `PLUTONIA.WAD` are
secondary columns (32 each). BFG duplicates, Freedoom and every non-Doom-format WAD were excluded
deliberately.

### Probe round 1 — idgames only (1,282 maps; T = 797 maps with a teleport line; L = 20,491 lines)

Special usage:

| What | Occurrences | Maps | Share of 1,282 |
|---|---:|---:|---:|
| linedef special 97 | 14,905 | 723 | 56.4 % |
| linedef special 39 | 243 | 54 | 4.2 % |
| linedef special 126 | 4,633 | 222 | 17.3 % |
| linedef special 125 | 710 | 80 | 6.2 % |
| thing 14 | 6,795 | 850 | 66.3 % |
| **any teleport linedef** | **20,491** | **797** | **62.2 %** |

Pairing, read the way `EV_Teleport` reads it (walk every sector carrying the tag, first
destination found wins), over the 20,065 lines whose tag matches ≥ 1 sector:

| Destinations reachable | Lines | Share |
|---|---:|---:|
| 0 | 67 | 0.3 % |
| **1** | **19,452** | **96.9 %** |
| > 1 | 546 | 2.7 % |

The same data as mutually exclusive buckets over all 20,491 lines: tag 0 → 84 lines (13 maps);
tag matches no sector → 342 (17 maps); exactly one sector and one marker → 17,823 (768 maps);
one sector, no marker → 58 (14 maps); one sector, > 1 marker → 207 (19 maps); several sectors
carry the tag → 1,977 (214 maps, 26.9 % of T).

Per-map majority trigger shape, over the 728 maps with ≥ 1 tagged player-teleport line:

| Majority shape | Maps | Share | ≥ 80 % dominant |
|---|---:|---:|---:|
| pad entered by crossing (pad = back sector) | 441 | 60.6 % | 342 |
| line across an ordinary two-sided boundary | 257 | 35.3 % | 192 |
| pad left by crossing (pad = front sector) | 29 | 4.0 % | 11 |
| other | 1 | 0.1 % | — |

Back-side pads outnumber front-side pads **10 : 1** (2,337 small back pads in 567 maps against 235
in 76). Pad edge counts, over the 2,337: 4 edges → 1,075 (46.0 %); 1 edge → 982 (42.0 %); 2 → 165;
3 → 78; > 4 → 37.

Destinations — 6,040 distinct destination sectors across 791 maps:

| Property | Sectors | Share | Maps |
|---|---:|---:|---:|
| holds exactly 1 thing 14 | 5,999 | 99.3 % | 789 |
| holds 2 | 27 | 0.4 % | 16 |
| holds > 2 | 14 | 0.2 % | 9 |
| **one-way** (no teleport line on its boundary) | 4,722 | 78.2 % | 735 |
| is itself a small fully-teleporting pad | 533 | 8.8 % | 187 |
| bbox ≤ 64 | 2,627 | 43.5 % | 492 |
| bbox > 256 | 1,813 | 30.0 % | 517 |

Symmetric A↔B pairs: 497 pairs across 188 maps (23.6 % of T).

Closets (proxy: a teleport line whose front sector holds ≥ 1 standard monster and whose tag
resolves to exactly one marker) — 7,900 lines in 533 maps (66.9 % of T), 38.6 % of every teleport
line in the corpus. Of 2,304 distinct closet sectors: 1,868 (81.1 %) carry tag 0 and so cannot be
remotely referenced at all; 262 (11.4 %) are tagged and referenced by another linedef special;
174 (7.6 %) are tagged and referenced by nothing.

Teleport-to-exit: 52 destination sectors in 52 maps (6.5 % of T, 4.1 % of 1,282), exactly one per
map in every case. Self-teleport (destination sector is one of the line's own two sectors): 125
lines in 39 maps (4.9 % of T).

### Probe round 2 — refined geometry, idgames against retail

Populations: idgames 1,282 maps / 797 with a teleport line (62.2 %); DOOM+DOOM2 68 / 48 (70.6 %);
DOOM 36 / 21 (58.3 %); DOOM2 32 / 27 (84.4 %); TNT 32 / 32 (100 %); PLUTONIA 32 / 31 (96.9 %).

Raw special counts:

| | idgames | DOOM+DOOM2 | TNT | PLUTONIA |
|---|---:|---:|---:|---:|
| 97 | 14,905 | 651 | 666 | 543 |
| 39 | 243 | 6 | 5 | 1 |
| 126 | 4,633 | 29 | 119 | 393 |
| 125 | 710 | 5 | 97 | 32 |
| thing 14 | 6,795 | 257 | 340 | 293 |
| **player trigger lines (97+39)** | **15,148** | **657** | **671** | **544** |

Trigger-line shape, keyed by the sector the crosser **enters** (the back sector, which is what
`EV_Teleport`'s front-side-only rule requires) — per line:

| | idgames | DOOM+DOOM2 | TNT | PLUTONIA |
|---|---:|---:|---:|---:|
| ISLAND | 5,293 (34.9 %) | 338 (51.4 %) | 297 (44.3 %) | 374 (68.8 %) |
| ALCOVE | 1,088 (7.2 %) | 94 (14.3 %) | 49 (7.3 %) | 25 (4.6 %) |
| BOUNDARY | 5,743 (37.9 %) | 218 (33.2 %) | 280 (41.7 %) | 135 (24.8 %) |
| OTHER | 3,024 (20.0 %) | 7 (1.1 %) | 45 (6.7 %) | 10 (1.8 %) |

Island pads, counted per pad sector:

| | idgames (1,395) | DOOM+DOOM2 (83) | TNT (87) | PLUTONIA (75) |
|---|---:|---:|---:|---:|
| exactly 4 edges | 1,336 (95.8 %) | 81 (97.6 %) | 84 (96.6 %) | 62 (82.7 %) |
| every edge carries the trigger | 1,115 (79.9 %) | 82 (98.8 %) | 63 (72.4 %) | 73 (97.3 %) |
| exactly 64×64 | 943 (67.6 %) | 77 (92.8 %) | 61 (70.1 %) | 55 (73.3 %) |
| also holds a marker (two-way) | 317 (22.7 %) | 20 (24.1 %) | 20 (23.0 %) | 24 (32.0 %) |

Island floor delta against the host sector — exact DOOM+DOOM2 values over 83 pads: **+8 → 36**,
+24 → 22, +16 → 13, flush → 7, −8 → 5. Ceiling delta 0 on 45 of 83; light delta 0 on 45. Pad
flat: 79 of 83 are `GATE1`–`GATE4` (`GATE3` 33, `GATE2` 24, `GATE4` 11, `GATE1` 11); idgames
follows the same convention for 723 of 1,395. Pad sector special 0 on 61 of 83 (idgames: 1,019 of
1,395).

Alcove pads: 94 in DOOM+DOOM2, of which opening width 64 on 89, depth 64 on 81 (32 on 8), bbox
≤ 64 on 91, floor +8 on 38 and flush on 30. idgames: 1,088 alcoves, opening width 33–64 on 74.7 %,
depth 33–64 on 79.1 %.

Boundary triggers are still mostly pads that miss the island/alcove test: DOOM+DOOM2 has 191 of
218 back sectors ≤ 64 and 122 of 218 front sectors > 256. idgames is genuinely mixed — 2,708 of
5,743 back sectors are > 256.

Destination markers — distance to the nearest teleport line in the marker's own sector:

| Distance | idgames (6,353) | DOOM+DOOM2 (257) |
|---|---:|---:|
| 17–32 | 778 (12.2 %) | **85 (33.1 %)** |
| any other non-zero bucket | 587 | 17 |
| none in sector (one-way) | 4,982 (78.4 %) | 155 (60.3 %) |

Of the 90 DOOM+DOOM2 markers that sit in a sector carrying a teleport line, **85 are 17–32 units
from it** — a marker in the center of a 64×64 pad is exactly 32 units from each edge.

Closet sectors (same proxy as round 1):

| | idgames (2,415) | DOOM+DOOM2 (97) | TNT (151) | PLUTONIA (109) |
|---|---:|---:|---:|---:|
| sealed — every non-trigger edge one-sided | 468 (19.4 %) | 8 (8.2 %) | 54 (35.8 %) | 24 (22.0 %) |
| ≥ 1 two-sided non-trigger join | 1,947 (80.6 %) | 89 (91.8 %) | 97 (64.2 %) | 85 (78.0 %) |
| tag 0 (no remote reference possible) | 1,972 (81.7 %) | 89 (91.8 %) | 132 (87.4 %) | 96 (88.1 %) |
| tagged and referenced by another special | 267 (11.1 %) | 1 (1.0 %) | 3 (2.0 %) | 6 (5.5 %) |

The retail split is the design rule: of the 8 genuinely sealed DOOM+DOOM2 pens, **7 use the
monsters-only special**; the open holding rooms (59 of 97 closet sectors) use 97. Closet monsters
carrying the ambush flag: 251 of 564 (44.5 %) in DOOM+DOOM2 against 2,619 of 22,258 (11.8 %) in
idgames.

Exit-by-teleport: 55 instances in 52 idgames maps; 1 in DOOM+DOOM2 (DOOM2 MAP08 — DOOM.WAD has
none); TNT 3; PLUTONIA 1.

### Probe round 3 — closet release (a model, not a rule)

Round 3 asked how a sealed or semi-sealed pen actually gets woken, and its answer is explicitly a
**statistic, not an engine verdict**. The "acoustic model" is a geometric proxy — a join whose
opening `min(ceilings) − max(floors)` is greater than zero and which is free of the
sound-blocking flag — and `data/engine.toml` quotes the engine only for the flag's behavior, never
for an opening-range cutoff. Nothing in crustygen is a rule derived from these rows:

| | idgames | DOOM+DOOM2 |
|---|---:|---:|
| closets audible from the player start at load (acoustic model) | 38.2 % | 22.7 % |
| unreachable pens with a remotely referenced *neighbor* strip | 52.5 % | 45.3 % |
| closet monsters ambush-flagged | 11.8 % | 43.5 % vs 44.9 % (audible vs not) |

The strip specials the retail pens' neighbors carry are 62, 36, 109, 20, 2, 123, 103 and 102 —
tier-3 floor and door values outside this vocabulary, which is why "sealed pen released by a
remote strip" is a recorded follow-up rather than a construct.

## The recognizer reproduces the probe

Two arbiter numbers were fixed before the recognizer existed, from the probe and from the
predecessor measurement, and the sweep must hit both exactly:

| Arbiter | Expected | Measured |
|---|---:|---:|
| maps with ≥ 1 teleport line (`aggregate.teleports.maps_with_teleports`) | 797 | **797** |
| maps carrying linedef special 97 (`telemetry.linedef_specials["97"]`) | 723 | **723** |

Both match. Population, dedup and gate are the same as round 1's, round 2's and the 2026-08-28
expressibility run's.

The recognizer was then run over the retail headline population directly — `RETAIL/DOOM.WAD` and
`RETAIL/DOOM2.WAD` copied into a scratch directory and swept with `crustygen-corpus` (exit 0, no
stderr, 68 unique maps, nothing refused for format or assembly; **no retail bytes are committed**).
Its Teleports section against round 2's independent probe:

| Measure | Round 2 probe | Recognizer sweep |
|---|---:|---:|
| unique maps | 68 | 68 |
| maps with a teleport line | 48 (70.6 %) | 48 (70.6 %) |
| player trigger lines (97 + 39) | 657 | 657 |
| monsters-only lines (126 + 125) | 34 | 34 |
| one-shot lines (39 + 125) | 11 | 11 |
| island / alcove trigger lines | 338 / 94 | 338 / 94 |
| exit-by-teleport | 1 instance, 1 map | 1 line, 1 map |

Two rows differ, and both differences are accounted for by scope rather than by disagreement. The
probe classified **player** trigger lines only (338 + 94 + 218 + 7 = 657); the recognizer
classifies all four specials (338 + 94 + 240 + 19 = 691), so the extra 22 `boundary` and 12
`other` lines are the 34 monsters-only lines the probe's shape table never covered. Self-referencing
is the same story: 7 in the probe's player-only view, 19 across all four specials.

The retail axis figures, for the record: teleport lines 83.8 % (11 of 68 maps carry a refused
line), line specials 0.0 %, sector specials 51.5 %, thing kinds 97.1 %. Retail maps are wall-to-wall
tier-3 specials, so no retail map is expressible today and none is expected to be.

## What this fixes for the roadmap

The line axis was, and remains, the binding constraint — but 97 is out of it. The new blocker
ranking:

| Value | Maps | Share | Was |
|---|---:|---:|---|
| 62 | 616 | 48.0 % | #2 |
| 23 | 386 | 30.1 % | #3 |
| 109 | 331 | 25.8 % | #4 |
| 103 | 329 | 25.7 % | #5 |
| 88 | 326 | 25.4 % | #6 |
| 48 | 312 | 24.3 % | #7 |

The greedy conjunction curve now opens at 62 and reaches 11.6 % by k = 5. **62 and 88 are the
lift pair** — the two specials the corpus spends on moving floors — and between them they are
what 62's 48.0 % and 88's 25.4 % measure. That names sub-project 3: **lifts**, exactly as the
predecessor document predicted from the same table one construct earlier.

The teleport axis itself is now a roadmap input in its own right. 246 maps (19.2 % of the corpus,
30.9 % of teleport-carrying maps) fail it, and 183 of those fail on self-referencing triggers
alone — a shape the IR has decided not to express. That ceiling does not move with more line
specials; it moves only if a future IR gains a way to state a trigger whose two sides are the same
sector, which round 2 says id never once needed.

## Caveats

- **Sector extent is a bounding box.** Neither the probes nor the recognizer can compute a sector
  polygon's area, so every "small pad" figure is a bounding box over the endpoints of the sector's
  linedefs. It over-states extent for a non-convex or multi-part sector and never under-states it,
  so every pad count is a lower bound.
- **UDMF/ZDoom-numbered teleports are unmeasured.** 66 of the 1,282 idgames maps are UDMF-origin,
  and a ZDoom-namespace map using the Hexen-style teleport special is invisible to every number
  here. This is a small undercount of "teleport usage" and never a miscount of 97/39/125/126. All
  132 retail maps are binary Doom format, so the retail columns are unaffected.
- **The acoustic model is a statistic.** Round 3's audibility shares are a geometric proxy for a
  behavior the engine decides at runtime. No rule, check or compiler pass in crustygen derives
  anything from them, and none should without a primary source for the cutoff.
- **The `broken` refusal count is higher than the probe's tag-resolution failures.** The probe put
  lines that can never fire at 493 by tag resolution (84 + 342 + 67) plus up to 90 one-sided
  lines; the recognizer refuses 748. The two locate a marker's sector by different means — the
  probe descends the map's own BSP with `P_PointOnLineSide`, the recognizer uses the verifier's
  `Scene`, which is even-odd point-in-polygon over the sector's boundary — and the classes are not
  the same partition either, since the recognizer decides `self_referencing` before it resolves a
  tag. Neither is established as the cause; the discrepancy is recorded rather than smoothed, and
  it moves the headline conjunction by at most the 3 maps that separate the honest
  figure from the naïve one, though it could move the teleport axis share itself by more.
- **The conjunction curve's population changed.** Stated above and repeated here because it is the
  easiest number in this document to mis-read: the 2026-08-28 curve and this one walk different
  populations, so their checkpoints are not comparable.
- **"Unloadable" is still overcounted.** Ingest runs crustywad's *strict* assembly, so maps a
  lenient assembler would load are refused and never reach the classifier. crustygen
  [#34](https://github.com/masriamir/crustygen/issues/34) tracks it; closing it adds maps to the
  denominator, so these shares are not comparable with a post-#34 run.

## Re-running

In the crustywad checkout, `just harvest-sample 20260828 400` re-fetches the same draw (a present,
correctly sized zip is skipped, so it is cheap after the first run). Here, `just corpus
/path/to/20260828-400` writes `docs/measurements/expressibility-<today>.md` plus a gitignored JSON
under `target/`. Exit 1 is expected, not a failure. Compare the new Expressibility table, the
Teleports section and the blocker rankings against the tables above — and re-order the Project G
queue from the new blockers, not from these.
