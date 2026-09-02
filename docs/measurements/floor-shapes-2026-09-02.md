# How idgames and id's maps use tagged floor actions — a corpus measurement

**Date:** 2026-09-02 · **Populations:** `RETAIL/DOOM.WAD` + `DOOM2.WAD` (68 maps), `RETAIL/TNT.WAD` +
`PLUTONIA.WAD` (Final Doom, 64 maps), and the sample of record
`crustywad/xtask/data/samples/20260828-400/` (1,282 unique maps) · **Tool:** `examples/liftprobe`
in crustygen — `cargo run --release --example liftprobe -- floors <label> <dir-or-file>...`,
committed with this document · **crustywad:** 0.9.6 · **Engine source:** `linuxdoom-1.10` at the
pinned commit `a77dfb96`, read for this probe (`p_spec.h`, `p_spec.c`, `p_floor.c`, `p_plats.c`,
`p_switch.c`, `p_map.c`, `p_maputl.c`, `info.c`).

Sub-project 4a of Project G. The question: what *is* a tagged floor action in the maps we have —
what it moves, where it sends it, what that does to where the player can walk, and how it is
fired — so that the IR, the compiler, rule P7 and the recognizer can state the same thing. The
lift measurement ([`lift-shapes-2026-08-29.md`](lift-shapes-2026-08-29.md), §I) named this family
as sub-project 4's territory; the after-measurement for lifts
([`lifts-2026-08-30.md`](lifts-2026-08-30.md)) put its two commonest specials, 23 and 38, at the
top of the line-blocker table.

## Method and its limits

The population is drawn the way `crustygen-corpus` draws it: every `*.zip` opens through crustywad's
archive reader under the same lenient options, every `.wad` member is read, every map group passes
through the same `ingest::load_map` gate, and maps are deduplicated by the same `sha256:` hash. An
unreadable archive, WAD or map group is named on stderr and skipped; the sample's failures are the
same ones every sweep reports (`9470-rocket1` is not an archive; `8721-inva_19`'s REJECT lumps are
one byte short; `9060-nbdevkit` and `9656-mind` carry broken map groups). Retail WADs are read as
bare files. **1,282 unique sample maps load**, matching every earlier run.

**Arbiter:** before proposing anything, the probe reproduces the all-axes baseline the lift
after-measurement recorded: **114 / 1,282 = 8.9 %** expressible today, with the same
teleport-and-lift gating `crustygen-corpus` applies (`src/lift/corpus.rs`). It does.

Every special value below was transcribed from the fetched source, not recalled: the `case N:` label
in `P_CrossSpecialLine` / `P_ShootSpecialLine` (`p_spec.c`) or `P_UseSpecialLine` (`p_switch.c`),
and the `EV_DoFloor` / `EV_DoPlat` type it dispatches to. The engine layer of the probe cites
`file:line` beside each formula.

Limits the numbers carry (also §J):

- **Load-time heights.** Every destination is the one the engine computes at the heights the map
  loads with. A floor that moves after another already has is measured in its unfired state; §G
  counts how often that can happen.
- **The walking model is a pure height test.** `ML_BLOCKING` fences, monsters, the player's radius
  and use-reach all read as passable. A switch the player cannot actually reach still counts.
- **Local, not global.** Reachability effects are judged on the target and its immediate neighbors
  (definitions below), not by flooding the whole map from the player start.
- **UDMF-origin maps** (66 in the sample) are read with Doom special numbers.
- **`raiseToTexture`** (30/96) needs texture heights the probe does not load; those actions are
  counted and left unclassified.
- **Ceiling specials** are counted as an adjacent family and otherwise ignored.

## Engine facts (fetched, pinned)

- **`p_spec.h`** — `floor_e { lowerFloor, lowerFloorToLowest, turboLower, raiseFloor,
  raiseFloorToNearest, raiseToTexture, lowerAndChange, raiseFloor24, raiseFloor24AndChange,
  raiseFloorCrush, raiseFloorTurbo, donutRaise, raiseFloor512 }`; `FLOORSPEED FRACUNIT` (line 600);
  `PLATSPEED FRACUNIT` (305); `MAX_ADJOINING_SECTORS 20` (`p_spec.c:326`).
- **`p_floor.c`, `EV_DoFloor` (260–444)** — one thinker per tagged sector not already moving
  (`if (sec->specialdata) continue;`, 277). Destinations: `lowerFloor` → `P_FindHighestFloorSurrounding`
  (291–297); `lowerFloorToLowest` → `P_FindLowestFloorSurrounding` (299–305); `turboLower` → highest
  neighbor floor `+ 8` **only if** that differs from the current floor, at `FLOORSPEED * 4`
  (307–315); `raiseFloor` → `P_FindLowestCeilingSurrounding` capped at the sector's own ceiling
  (319–329), `raiseFloorCrush` the same minus 8 **after** the cap (327–328); `raiseFloorTurbo` and
  `raiseFloorToNearest` → `P_FindNextHighestFloor(sec, sec->floorheight)` (331–345); `raiseFloor24`,
  `raiseFloor512` (347–360); `raiseFloor24AndChange` also copies the trigger line's front-sector
  flat and special **at start** (362–370); `raiseToTexture` → the least bottom-texture height on the
  sector's two-sided lines (372–401); `lowerAndChange` → lowest neighbor floor, taking the flat and
  special of a neighbor at that height **on arrival** (403–438, `T_MoveFloor` 241–244).
- **`p_floor.c`, `T_MoveFloor` (209–254)** — on `pastdest` the thinker clears `sector->specialdata`
  and removes itself. **Every floor action is one-way.** `T_MovePlane` snaps a floor that is already
  past its destination straight to it, so a `lower*` whose destination lies *above* the floor
  raises it instantly — the corpus uses this (§C, "direction disagrees").
- **`p_plats.c`, `EV_DoPlat`** — `raiseToNearestAndChange`: `speed = PLATSPEED/2`, flat copied from
  the line's front sector **at start**, `sec->special = 0`, `high = P_FindNextHighestFloor`;
  `raiseAndChange`: `high = floorheight + amount` (24 or 32 by dispatch), flat copied likewise. Both
  are removed in `T_PlatRaise` on `pastdest` (96–99): one-way like the floors.
- **`p_spec.c`** — `P_FindLowestFloorSurrounding` (270–291) starts at the sector's **own** floor;
  `P_FindHighestFloorSurrounding` (297–318) starts at **`-500*FRACUNIT`**, so a sector with no
  two-sided neighbor "lowers" to −500; `P_FindNextHighestFloor` (329–375) returns the least neighbor
  floor strictly above the current one, the current one when none is, and stops collecting at 20
  adjoining sectors; `P_FindLowestCeilingSurrounding` (382–401) starts at `MAXINT`. `getNextSector`
  crosses two-sided lines only, and a self-referencing line makes a sector its own neighbor.
- **Dispatch.** `P_CrossSpecialLine` (`p_spec.c`): W1 cases 541–784 end `line->special = 0`, WR
  cases from 786 do not. `P_UseSpecialLine` (`p_switch.c`): front side only (`if (side)`, 288);
  S1 "SWITCHES" 347–511 call `P_ChangeSwitchTexture(line,0)`, SR "BUTTONS" 513–647 call it with `1`.
  `P_ShootSpecialLine` (`p_spec.c` ~980–1000): 24 → `raiseFloor`, 47 → `raiseToNearestAndChange`.
  Case 40 (`p_spec.c:664–669`) is `RaiseCeilingLowerFloor`: `EV_DoCeiling(raiseToHighest)` **and**
  `EV_DoFloor(lowerFloorToLowest)`.
- **Walking.** `P_LineOpening` (`p_maputl.c:300–332`): `opentop = min(ceilings)`, `openbottom =
  max(floors)`. `P_TryMove` (`p_map.c`) rejects when `tmceilingz - tmfloorz < thing->height` (468)
  and when `tmfloorz - thing->z > 24*FRACUNIT` (478); the drop-off arm (481) is gated on
  `!(thing->flags & (MF_DROPOFF|MF_FLOAT))` and `MT_PLAYER` carries `MF_DROPOFF` (`info.c:1130`), so
  descent is free.

The fifteen families and their specials, by trigger form (W = walkover, S = use, G = gunshot; 1 =
once, R = repeatable):

| family (engine type) | W1 | WR | S1 | SR | G1 |
|---|---|---|---|---|---|
| `lowerFloor` (to highest neighbor floor) | 19 | 83 | 102 | 45 | |
| `lowerFloorToLowest` | 38, 40 | 82 | 23 | 60 | |
| `turboLower` | 36 | 98 | 71 | 70 | |
| `raiseFloor` (to lowest neighbor ceiling) | 5 | 91 | 101 | 64 | 24 |
| `raiseFloorCrush` | 56 | 94 | 55 | 65 | |
| `raiseFloorToNearest` | 119 | 128 | 18 | 69 | |
| `raiseFloorTurbo` | 130 | 129 | 131 | 132 | |
| `raiseFloor24` | 58 | 92 | | | |
| `raiseFloor24AndChange` | 59 | 93 | | | |
| `raiseFloor512` | | | 140 | | |
| `raiseToTexture` | 30 | 96 | | | |
| `lowerAndChange` | 37 | 84 | | | |
| plat `raiseAndChange` +24 | | | 15 | 66 | |
| plat `raiseAndChange` +32 | | | 14 | 67 | |
| plat `raiseToNearestAndChange` | 22 | 95 | 20 | 68 | 47 |

## Definitions the numbers depend on

- **Floor line** — a linedef whose special is in the table above. **Target** — a sector whose tag a
  floor line names (`P_FindSectorFromLineTag`: tag equality; a tag-0 line is counted as tag 0, not
  resolved against untagged sectors; a tag naming no sector is *dangling*). **Action** — one
  `(family, target)` pair; a target driven by lines of *k* families carries *k* actions.
- **Destination `d`** — per the family's formula at load-time heights; `f` = the target's floor at
  load; **travel** = `|d − f|`. **Dead** — `d == f`.
- **Standable(S)** — `ceiling(S) − floor(S) ≥ 56` (the player's height from `data/engine.toml`).
  A target that is not standable at rest is **sealed**.
- **pass(A → B)** across a two-sided boundary — both standable, `min(ceilings) − max(floors) ≥ 56`,
  and `floor(B) − floor(A) ≤ 24`. Descent is free.
- **Local graph** — the target `T` and its two-sided neighbors `N(T)`, with every `pass` edge among
  them, neighbor-to-neighbor edges included, at the map's heights except `T`'s floor (`f` before, `d`
  after). **reach(A)** for a neighbor `A` — the neighbors reachable from `A` inside the local graph,
  `T` allowed as a via but never counted as a destination. A non-standable `A` reaches nothing.
- **Effect** (over every neighbor `A`, `T` excluded as a destination) — **Opening**: every
  `reach(A)` grows or holds and one grows; **Closing**: every one shrinks or holds and one shrinks;
  **Mixed**: some gain and some loss; **Neutral**: no change and `d ≠ f`.
- **Enterable before / after** — some neighbor can `pass` onto `T` at `f` / at `d`.
- **Rider** — when `T` is enterable before, whether `reach(T)` after ⊇ `reach(T)` before
  (**Keeps**) or not (**Loses**); otherwise n/a.
- **Opening sub-shapes** (effect Opening, rider not Loses): **DropWall** — lowers and was not
  enterable before; **LedgeLower** — lowers and was enterable; **Bridge** — rises and was enterable
  (a pit or gap the player could drop into). **Reveal** — effect Neutral, not enterable before,
  enterable after: a sealed sector that lowers into reach without joining two neighbors (the sunken
  pedestal, the panel between areas already connected).
- **Activator** of a trigger line — the sector a player fires it from: the front sector of a use or
  gun line; for a walkover, whichever side can cross at rest under the step rule. Classified
  **Low** / **Level** / **Above** by floor relative to the target's rest floor, or **OnTarget**.
  **Placement** — OnTargetFront / OnTargetBack (the target is that side of the line), Adjacent (a
  side is a neighbor of the target), Remote. **Hops** — the fewest two-sided crossings from an
  activator sector to the target, heights ignored.
- "Sector size" is a **bounding box** over the endpoints of every linedef touching the sector.

Denominators: **U** = unique maps (68 / 64 / 1,282); **targets** = 472 / 651 / 9,443; **actions** =
495 / 684 / 9,912; classified actions (destination resolved) = 476 / 675 / 9,874.

---

## A. Special usage

Per family, lines / maps (share of U):

| family | DOOM+DOOM2 | Final Doom | idgames sample |
|---|---|---|---|
| `lowerFloor` | 91 / 26 (38.2 %) | 106 / 21 (32.8 %) | 1,320 / 293 (22.9 %) |
| `lowerFloorToLowest` | 117 / 30 (44.1 %) | 328 / 51 (**79.7 %**) | 3,496 / 559 (**43.6 %**) |
| `turboLower` | 69 / 24 (35.3 %) | 94 / 23 (35.9 %) | 1,822 / 287 (22.4 %) |
| `raiseFloor` | 35 / 9 (13.2 %) | 36 / 11 (17.2 %) | 1,108 / 148 (11.5 %) |
| `raiseFloorCrush` | 8 / 5 | 10 / 5 | 38 / 17 (1.3 %) |
| `raiseFloorToNearest` | 31 / 15 (22.1 %) | 18 / 11 (17.2 %) | 320 / 168 (13.1 %) |
| `raiseFloorTurbo` | 2 / 1 | 86 / 29 (45.3 %) | 516 / 102 (8.0 %) |
| `raiseFloor24` | 5 / 1 | 11 / 5 | 38 / 22 (1.7 %) |
| `raiseFloor24AndChange` | 13 / 3 | 1 / 1 | 13 / 6 |
| `raiseFloor512` | 1 / 1 | 0 | 8 / 5 |
| `raiseToTexture` | 6 / 5 | 8 / 7 | 20 / 14 (1.1 %) |
| `lowerAndChange` | 42 / 9 (13.2 %) | 22 / 8 (12.5 %) | 555 / 77 (6.0 %) |
| plat `raiseAndChange` +24 | 1 / 1 | 1 / 1 | 34 / 23 (1.8 %) |
| plat `raiseAndChange` +32 | 10 / 3 | 4 / 3 | 27 / 18 (1.4 %) |
| plat `raiseToNearestAndChange` | 60 / 29 (42.6 %) | 29 / 19 (29.7 %) | 447 / 194 (15.1 %) |
| **maps with ≥ 1 floor line** | **59 (86.8 %)** | **62 (96.9 %)** | **788 (61.5 %)** |

By trigger form, lines / maps: DOOM+DOOM2 — W1 263 / 42, S1 160 / 50, WR 38 / 8, SR 26 / 13, G1 4 / 3.
Final Doom — W1 509 / 59, S1 186 / 51, SR 33 / 17, WR 20 / 10, G1 6 / 4. Sample — W1 4,487 / 556,
S1 2,986 / 590, SR 1,423 / 187, WR 748 / 145, G1 118 / 44. Special 40 (the ceiling-raising lower):
4 / 15 / 199 lines. Adjacent families, lines / maps: stairs 28 / 18, 26 / 16, 692 / 150; donut
2 / 2, 12 / 2, 449 / 42; ceiling 42 / 11, 79 / 18, 1,013 / 248.

Per special, sample, lines / maps — the values that carry ≥ 1 % of maps (the full 48-row list is in
the probe's output). These reproduce the corpus blocker table
([`lifts-2026-08-30.md`](lifts-2026-08-30.md): 23 = 386, 38 = 309, 19 = 180, 102 = 136, 20 = 119,
18 = 94) and the lift measurement's §A transcription line for line.

| special | form, family | lines / maps |
|---|---|---|
| 23 | S1 `lowerFloorToLowest` | 1,227 / 386 (**30.1 %**) |
| 38 | W1 `lowerFloorToLowest` | 1,684 / 309 (**24.1 %**) |
| 19 | W1 `lowerFloor` | 795 / 180 (14.0 %) |
| 36 | W1 `turboLower` | 561 / 155 (12.1 %) |
| 71 | S1 `turboLower` | 320 / 152 (11.9 %) |
| 102 | S1 `lowerFloor` | 262 / 136 (10.6 %) |
| 20 | S1 plat `raiseToNearestAndChange` | 202 / 119 (9.3 %) |
| 18 | S1 `raiseFloorToNearest` | 132 / 94 (7.3 %) |
| 60 | SR `lowerFloorToLowest` | 176 / 94 (7.3 %) |
| 40 | W1 `lowerFloorToLowest` + ceiling | 199 / 76 (5.9 %) |
| 22 | W1 plat `raiseToNearestAndChange` | 143 / 70 (5.5 %) |
| 37 | W1 `lowerAndChange` | 505 / 69 (5.4 %) |
| 82 | WR `lowerFloorToLowest` | 210 / 62 (4.8 %) |
| 130 | W1 `raiseFloorTurbo` | 229 / 62 (4.8 %) |
| 70 | SR `turboLower` | 879 / 61 (4.8 %) |
| 119 | W1 `raiseFloorToNearest` | 100 / 57 (4.4 %) |
| 5 | W1 `raiseFloor` | 181 / 56 (4.4 %) |
| 101 | S1 `raiseFloor` | 712 / 48 (3.7 %) |
| 83 | WR `lowerFloor` | 198 / 36 (2.8 %) |
| 91 | WR `raiseFloor` | 136 / 36 (2.8 %) |
| 45 | SR `lowerFloor` | 65 / 35 (2.7 %) |
| 131 | S1 `raiseFloorTurbo` | 79 / 28 (2.2 %) |
| 69 | SR `raiseFloorToNearest` | 46 / 24 (1.9 %) |
| 47 | G1 plat `raiseToNearestAndChange` | 78 / 23 (1.8 %) |
| 24 | G1 `raiseFloor` | 40 / 21 (1.6 %) |
| 58 | W1 `raiseFloor24` | 37 / 21 (1.6 %) |
| 15 | S1 plat `raiseAndChange` +24 | 22 / 17 (1.3 %) |
| 64 | SR `raiseFloor` | 39 / 16 (1.2 %) |
| 128 | WR `raiseFloorToNearest` | 42 / 16 (1.2 %) |
| 14 | S1 plat `raiseAndChange` +32 | 20 / 13 (1.0 %) |
| 129 | WR `raiseFloorTurbo` | 28 / 13 (1.0 %) |

Everything else is under 1 % of maps; 93 (WR `raiseFloor24AndChange`) does not occur at all.

**Read:** the floor family is in **three of five sample maps** and nearly every retail map; the
lowering forms dominate (lower-to-lowest alone is in 43.6 % of sample maps and 79.7 % of Final
Doom's), and the **one-shot forms are the norm** — W1 + S1 are 77 % of sample floor lines (7,473 of
9,762; 84 % of triggers when a line naming several targets is counted per target, §E), the
opposite of lifts, where repeatable forms were 95 %. A floor action is fired once; the corpus
authors it that way.

## B. Tags

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| floor lines tagged 0 / dangling | 6 / 1 | 0 / 1 | 418 / 923 |
| targets (unique tagged sectors) | 472 | 651 | 9,443 |
| shared-tag groups (a tag naming ≥ 2 sectors) | 55 | 116 | 1,511 |
| — size 2 / 3 / 4 / 5+ | 27 / 8 / 5 / 15 | 77 / 14 / 11 / 14 | 645 / 276 / 215 / 375 |
| — one floor and mutually connected / one floor, disconnected / several floors | 2 / 26 / 27 | 7 / 43 / 66 | 79 / 608 / 824 |
| targets whose tag a lift special also names (maps) | 10 (7) | 21 (15) | 265 (107) |
| — another emittable special (door/exit/teleport) | 6 (4) | 30 (12) | 351 (82) |
| — a second floor family | 23 (14) | 33 (20) | 447 (155) |
| — of which a raise family **and** a lower family (two-way elevator) | 18 (13) | 30 (18) | 333 (138) |
| — any other special | 6 (3) | 7 (3) | 569 (101) |

**Read:** a floor tag naming several sectors is not the exception it was for lifts — **1,511
groups over 9,443 targets, and 54.5 % of them span several floors**. One switch dropping several
wall sectors, or several bars, is the idiom. The "one sector split by trim" case (one floor,
mutually connected) is 5.2 % of groups. The two-way elevator (a raise and a lower on one tag)
is on 3.5 % of targets and in 10.8 % of sample maps.

## C. Destination and effect

Effect class per family, sample (classified actions; Dead first):

| family | Dead | Opening | Closing | Mixed | Neutral | travel of moving targets (median, p90) |
|---|---:|---:|---:|---:|---:|---|
| `lowerFloor` | 74 | 455 | 31 | 1 | 839 | 128, 368 |
| `lowerFloorToLowest` | 387 | **1,666** | 186 | 133 | **2,178** | 120, 256 |
| `turboLower` | 55 | 494 | 33 | 9 | 764 | 112, 184 |
| `raiseFloor` | 70 | 60 | 68 | 16 | 284 | 120, 328 |
| `raiseFloorCrush` | 0 | 1 | 9 | 0 | 39 | 120, 280 |
| `raiseFloorToNearest` | 77 | 69 | 36 | 30 | 225 | 112, 224 |
| `raiseFloorTurbo` | 108 | 38 | 26 | 22 | 351 | 116, 368 |
| `raiseFloor24` | 0 | 10 | 1 | 0 | 27 | 24 |
| `raiseFloor24AndChange` | 0 | 3 | 0 | 0 | 20 | 24 |
| `raiseFloor512` | 0 | 1 | 5 | 4 | 8 | 512 |
| `lowerAndChange` | 23 | 105 | 16 | 8 | 170 | 84, 208 |
| plat `raiseAndChange` +24 | 0 | 8 | 2 | 0 | 22 | 24 |
| plat `raiseAndChange` +32 | 0 | 1 | 1 | 0 | 55 | 32 |
| plat `raiseToNearestAndChange` | 113 | 106 | 27 | 52 | 252 | 100, 232 |
| **all** | **907** | **3,017** | **441** | **275** | **5,234** | |

`raiseToTexture`: 38 actions unresolved. Quirks that actually fire in the sample: a `lowerFloor` /
`turboLower` with no two-sided neighbor (destination −500) **23**; next-highest no-op (nothing above)
**298**; next-highest hitting the 20-neighbor cap **18**; **760** actions (7.7 %) whose family
direction and travel direction disagree — a `lower*` whose highest neighbor is above it (the
instant-raise trick) or a `raise*` whose capped destination is below.

Effect × direction × enterable-before × rider, sample (9,874 classified actions), the cells that
matter:

| cell | count | what it is |
|---|---:|---|
| Neutral / down / sealed / n/a | **3,363** | a sealed sector lowers and joins no two neighbors (§D Reveal) |
| Opening / down / sealed / n/a | **2,616** | the drop wall |
| Neutral / up / enterable / Keeps | 1,335 | a rise that changes no reach (within a step, or nothing to join) |
| Dead / level / enterable / Keeps | 462 | |
| Dead / level / sealed / n/a | 445 | |
| Opening / up / enterable / Keeps | 251 | the bridge |
| Closing / down / enterable / Loses | 220 | the floor drops away under the player |
| Neutral / up / enterable / Loses | 157 | a 1-neighbor pedestal rises with the player on it |
| Mixed / down / enterable / Loses | 154 | |
| Neutral / down / enterable / Loses | 149 | the descender |
| Opening / down / enterable / Loses | 132 | a ledge lowers to join lower areas, stranding its rider |
| Neutral / down / enterable / Keeps | 126 | |
| Closing / up / enterable / Keeps | 118 | a pillar rises and seals (rider still fine) |
| Closing / up / enterable / Loses | 103 | a pillar rises and seals with the player on it |
| Neutral / up / sealed / n/a | 104 | |
| Mixed / up / enterable / Keeps | 93 | a bridge that also splits the pit it rises from |
| Mixed / up / enterable / Loses | 28 | |
| Opening / down / enterable / Keeps | 18 | the ledge-lower |

DOOM+DOOM2 and Final Doom have the same shape: Opening/down/sealed 132 and 217, Neutral/down/sealed
166 and 211, Opening/up/enterable/Keeps 24 and 30, everything else in the tens or below.

Neutral decomposed, sample (5,234 actions): down and sealed → enterable **3,236** (3,173 of them by
more than a step); up and enterable → enterable 1,206 (1,039 by more than a step — a rise that joins
nothing new); up and enterable → sealed 286 (a pillar closing over nobody's route); down and
enterable → enterable 275; sealed → sealed 231 (127 down, 104 up).

The **Neutral / down / sealed** cell (3,363 actions), opened up: neighbors 1 / 2 / 3 / 4+ =
**2,598** / 492 / 194 / 79; standable once moved **96.2 %**; destination exactly a neighbor's
floor 78.2 %; two neighbors already mutually reachable before only 6.1 %; **holds ≥ 1 thing
38.7 %** — imp 570, hell_knight 378, pinky 350, revenant 259, shotgun_guy 259, cacodemon 163,
baron_of_hell 152, box_of_rockets 146 (and one map's 1,345 blue skull keys). So this is not a
decorative panel between joined rooms: it is a **one-neighbor sealed alcove with a monster or a
prize standing inside it**, lowered into reach. §D names it the Reveal.

**Read:** **Opening actions are 31 % of the sample's, and 87 % of those are the drop wall.**
Closing and mixed together are 7 %. The largest class is Neutral, and its largest cell is a sealed
sector lowering — which the pair-based definition cannot see as a gain because the moving sector is
never a destination; that cell is the Reveal (§D).

## D. Opening shapes

| DropWall | DOOM+DOOM2 (131) | Final Doom (214) | idgames (2,544) |
|---|---|---|---|
| neighbors 2 / 3 / 4–5 / 6+ | 109 / 10 / 11 / 1 | 174 / 17 / 20 / 3 | 1,981 / 261 / 237 / 65 |
| solid at rest (ceiling − floor < 56) / floor == ceiling | 80.9 % / 78.6 % | 79.9 % / 71.0 % | **76.7 % / 67.6 %** |
| modal bbox | 16×64 (10), 8×16 (8), 8×128 (7) | 8×64 (14), 24×128 (13), 64×64 (13) | **16×64 (166), 64×64 (136), 16×128 (127)** |
| min side < 64 / = 64 / 65..128 / > 128 | 73 / 12 / 20 / 26 | 100 / 25 / 28 / 61 | 1,365 / 339 / 369 / 471 |
| travel median, p90 | 120, 160 | 128, 200 | **112, 224** |
| destination == a neighbor's floor | 73.3 % | 86.0 % | **81.1 %** |
| new pairs: 1 / 2 / 3+ · some pair bidirectional | 6 / 120 / 5 · 94.7 % | 15 / 175 / 24 · 91.6 % | 166 / 2,098 / 280 · 92.1 % |
| flat == a neighbor's / light == / ceiling == every neighbor's | 84.0 / 87.8 / 51.1 % | 90.2 / 87.9 / 51.4 % | 78.4 / 80.0 / 44.8 % |
| holds ≥ 1 thing | 12.2 % (imp 17, pinky 9, hell_knight 8) | 15.9 % (imp 60, box_of_rockets 45) | 11.6 % (imp 360, shotgun_guy 113, zombieman 109, armor_bonus 107) |
| pocket neighbors (reachable only through the wall) · their things | 79 · imp 75, lost_soul 45, cacodemon 35 | 99 · shotgun_guy 99, box_of_rockets 70, revenant 66 | **1,130** · imp 557, shotgun_guy 321, cacodemon 320, pinky 237 |
| trigger kinds S / W / S+W / G | 39 / 90 / 2 / 0 | 88 / 121 / 3 / 2 | 1,248 / 1,241 / 48 / 7 |
| one-shot only / repeatable only | 124 / 7 | 202 / 12 | **2,426 / 118** |

| Bridge | DOOM+DOOM2 (23) | Final Doom (26) | idgames (236) |
|---|---|---|---|
| neighbors 2 / 3 / 4–5 / 6+ | 3 / 14 / 3 / 3 | 4 / 13 / 9 / 0 | 64 / 114 / 46 / 12 |
| modal bbox | 128×192, 192×192, 192×320 (2 each) | 64×128 (2) | 32×64 (12), 64×128 (10), 64×192 (8) |
| min side < 64 / = 64 / 65..128 / > 128 | 2 / 6 / 7 / 8 | 1 / 5 / 3 / 17 | 43 / 46 / 48 / 99 |
| travel median, p90 | 80, 168 | 96, 256 | **96, 288** |
| destination == a neighbor's floor | 87.0 % | 96.2 % | **91.1 %** |
| new pairs: 1 / 2 / 3+ · some pair bidirectional | 3 / 14 / 6 · 69.6 % | 7 / 17 / 2 · 69.2 % | 28 / 166 / 42 · 82.6 % |
| flat == a neighbor's / light == / ceiling == every neighbor's | 78.3 / 87.0 / 26.1 % | 96.2 / 100 / 38.5 % | 85.6 / 91.9 / 41.5 % |
| holds ≥ 1 thing | 47.8 % (health_bonus 11) | 38.5 % (chaingunner 23) | **37.7 %** (imp 341, pinky 62, tall_blue_torch 39) |
| trigger kinds S / W / G | 15 / 7 / 1 | 15 / 11 / 0 | 159 / 70 / 6 |
| one-shot only / repeatable only | 22 / 1 | 26 / 0 | 221 / 15 |

**LedgeLower** is 0 / 2 / 17 targets and **OtherOpening** is 0 everywhere: the two opening shapes
the definitions predicted are the two the corpus builds.

| Reveal | DOOM+DOOM2 (160) | Final Doom (196) | idgames (3,157) |
|---|---|---|---|
| neighbors 1 / 2 / 3 / 4–5 / 6+ | **147** / 8 / 1 / 4 / 0 | **151** / 26 / 9 / 10 / 0 | **2,534** / 399 / 176 / 38 / 10 |
| solid at rest (ceiling − floor < 56) / floor == ceiling | 19.4 % / 9.4 % | 45.9 % / 25.5 % | 48.6 % / 31.7 % |
| modal bbox | **16×16 (51)**, 64×64 (30), 192×192 (11) | **64×64 (26)**, 16×16 (12), 128×128 (8) | **64×64 (550)**, 16×16 (351), 128×128 (168), 8×8 (164), 32×32 (158) |
| min side < 64 / = 64 / 65..128 / > 128 | 84 / 44 / 14 / 18 | 85 / 36 / 29 / 46 | 1,556 / 736 / 470 / 395 |
| travel median, p90 | 80, 176 | 120, 248 | **120, 248** |
| destination == a neighbor's floor | 83.8 % | 79.1 % | 78.6 % |
| flat == a neighbor's / light == / ceiling == every neighbor's | 27.5 / 76.2 / 71.9 % | 68.4 / 80.1 / 51.5 % | 65.0 / 73.2 / 65.3 % |
| holds ≥ 1 thing | **40.0 %** (imp 46, shotgun_guy 14, revenant 12, cell_charge 9) | **40.3 %** (imp 36, box_of_shells 30, box_of_rockets 26, hell_knight 26) | **39.5 %** (imp 561, pinky 350, shotgun_guy 254, revenant 247, cacodemon 163, box_of_rockets 142, archvile 106, armor_bonus 97) |
| two neighbors already mutually reachable before | 6.9 % | 8.7 % | 6.2 % |
| trigger kinds S / W / S+W / G | 113 / 47 / 0 / 0 | 99 / 85 / 11 / 1 | 1,777 / 1,298 / 49 / 30 |
| one-shot only / repeatable only / mixed | 138 / 22 / 0 | 191 / 5 / 0 | 3,004 / 150 / 3 |

The Reveal is by count the **largest opening shape in every population**: more targets than the
drop wall in the sample (3,157 against 2,544) and in retail (160 against 131; 196 against 214 is
the one exception). Four in five have exactly one neighbor. It comes in two sizes: the 16×16 (and
8×8, 32×32) **sunken pedestal** — a pillar in the floor that drops to expose the item on it — and the
64×64 (and 128×128) **closet**, a sealed room with monsters standing inside its solid wall, released
when its floor drops (a thing spawns at its sector's floor regardless of headroom, so an imp can be
authored inside a floor-at-ceiling sector). A tenth of reveals are panels between areas already
joined. Reveals are one-shot to the same degree as drop walls (95 %) and are switched more often
than walked (56 % S in the sample, 71 % in DOOM+DOOM2).

**Read.** The **drop wall** is a thin sealed strip between exactly two areas (78 % have two
neighbors; the modal footprint is 16×64; three quarters have floor at or near ceiling), lowered
once by a single trigger (95 % one-shot only) to exactly a neighbor's floor (81 %), and **it is the
monster closet**: 1,130 sample pockets open only through a drop wall, holding imps, shotgun guys
and cacodemons by the hundred. The **bridge** is larger and squarer, rises once (94 % one-shot) to
exactly the walkway's floor (91 %), and holds things more than a third of the time — it is as much a
platform to fight on as a way across.

## E. Triggers

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| lines: S / W / G | 266 / 544 / 4 | 402 / 733 / 7 | 8,562 / 11,917 / 199 |
| one-shot / repeatable | 710 / 104 | 1,066 / 76 | **17,433 / 3,245** |
| placement OnTargetFront / OnTargetBack / Adjacent / **Remote** | 47 / 107 / 264 / **396** | 22 / 77 / 223 / **820** | 323 / 1,056 / 2,923 / **16,376 (79 %)** |
| hops 0 / 1 / 2 / 3 / 4–5 / 6+ / unreachable | 100 / 290 / 104 / 46 / 86 / 156 / 32 | 50 / 211 / 209 / 152 / 229 / 279 / 12 | 563 / 3,346 / 2,780 / 2,489 / 2,884 / **6,453** / 2,163 |
| activator floor Low / Level / Above / OnTarget | 424 / 181 / 121 / 100 | 652 / 206 / 255 / 50 | 11,058 / 4,202 / 5,100 / 563 |
| S lines with an `SW1`/`SW2` texture in a front slot | 80.5 % | 74.6 % | 56.8 % |
| W lines that are trip lines (both sides one sector) | 11.9 % | 6.1 % | 17.0 % |
| triggers per target 1 / 2 / 3+ | 350 / 36 / 86 | 460 / 74 / 117 | 6,777 / 917 / 1,749 |

**Read:** **a floor trigger is remote.** Four in five sample floor lines sit in a sector that is
neither the target nor a neighbor of it, and a third are six or more rooms away. The lift model —
called from its own face — does not describe this construct; a floor action has a *trigger placed
somewhere* and a *target somewhere else*. Walkovers outnumber switches, and both are overwhelmingly
one-shot. Most walkovers span a portal (a threshold), not a trip line inside a room.

## F. Rendering-facing

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| drop-wall boundaries measured | 499 | 1,201 | 11,883 |
| neighbor-side lower, top | `-` 49, BROWNHUG 48, METAL7 25, COMPSPAN 21 | `-` 93, SILVER1 69, METAL 62, BROWN1 58 | `-` 1,453, METAL 541, REDWALL 474, CRATE1 238, METAL2 235 |
| lower `ML_DONTPEGBOTTOM` | 5.2 % | 23.2 % | **5.5 %** |
| boundaries where the wall's ceiling is below the neighbor's · upper unpegged | 78 · 17.9 % | 237 · 71.7 % | 2,415 · 24.2 % |
| a middle texture on either side | 0 % | 2.4 % | 1.0 % |
| bridge walkway boundaries · neighbor-side lower | 52 · `-` 37 | 50 · `-` 47 | 592 · `-` 534 |
| walkway lower unpegged | 42.3 % | 34.0 % | 5.6 % |
| bridge rest flat == the walkway's | 20.0 % | 28.0 % | 31.2 % |
| "and change" bridges whose copied flat == the walkway's | 66.7 % | 63.6 % | **76.0 %** |

**Read:** the face that lowers with a drop wall is the neighbor's **lower** texture, pegged
(anchored to the wall's own floor, so it rides down with it) 95 % of the time in the sample — the
lift riser's convention again. Where the wall also has a lower ceiling than its neighbor, an upper
is drawn too, and there the corpus is split on pegging (24 % unpegged; Final Doom 72 %). A bridge's
riser is rarely textured at all at rest (the walkway side's lower is blank 90 % of the time — the
bridge starts level with its pit and no lower is visible), and the "and change" forms exist to give
the risen bridge the walkway's flat: three quarters copy exactly it.

## G. Chains

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| targets with a neighbor that is itself a floor target | 15.0 % | 13.2 % | **19.8 %** |
| — a neighbor that is a lift plat | 1.3 % | 1.7 % | 1.9 % |
| — a neighbor tagged by an emittable door special | 0 | 0 | 0.0 % |
| targets driven by ≥ 2 families | 4.9 % | 5.1 % | 4.7 % |
| targets whose destination-defining neighbor is movable | 4.0 % | 3.2 % | 4.3 % |

**Read:** one target in five borders another; only one in twenty-three has its *destination* defined
by a movable neighbor. Load-time evaluation is right for the rest.

## H. Arbiters

The probe reproduces the baseline, then re-judges every sample map under candidate gates. **Line
axis** = every linedef special the map uses is emittable (today's set plus the floor specials the
row accepts); **all axes** = that and sector specials, thing kinds, teleports and lifts already
pass. A **candidate** target has one family, a resolved destination, an opening sub-shape (DropWall,
LedgeLower, Bridge with a rider who Keeps, or Reveal), no non-floor special on its tag, no gun
trigger, and a trigger some sector can fire. Gate **G** = no tag-0 or dangling floor line, no gun
line, every target a candidate — a **shared tag is accepted when every member sector qualifies on
its own**. The four rows under G each add one restriction, so its price is read directly.

| gate (sample, U = 1,282) | line axis | all axes |
|---|---:|---:|
| baseline today (teleports + lifts, no floors) | 173 (13.5 %) | **114 (8.9 %)** |
| naïve — every non-gun floor special accepted | 216 (16.8 %) | **128 (10.0 %)** |
| **G** | 191 (14.9 %) | **122 (9.5 %)** |
| G + every floor tag names one sector | 186 (14.5 %) | 119 (9.3 %) |
| G + no Reveal target | 183 (14.3 %) | 118 (9.2 %) |
| G + no Remote trigger | 177 (13.8 %) | 116 (9.0 %) |
| G + no chain (§G) | 190 (14.8 %) | 121 (9.4 %) |
| G + all four restrictions (strict) | 176 (13.7 %) | 116 (9.0 %) |

Maps with ≥ 1 floor line: 788; refused by G: **590 (74.9 %)**. First applicable reason: conflict
(a non-floor special names the tag) **172** · neutral, not a reveal **125** · ≥ 2 families **73** ·
dangling or tag-0 **62** · closing or mixed **55** · dead **48** · gun **42** · rider loses 10 ·
unresolved destination 2 · no activator 1.

Retail: DOOM+DOOM2 stays at 0 on all axes under every gate (its line axis goes 1 → 2 maps under G;
47 of 59 floor maps are refused, conflict 11, closing/mixed 12, ≥ 2 families 9); Final Doom stays at
0 (56 of 62 refused, conflict 23, neutral 14, ≥ 2 families 9).

**Read.** The ceiling floors alone can add is **+14 maps** (10.0 %); the gate this measurement
argues for reaches **+8** (9.5 %); the strict gate — one sector per tag, no reveals, no remote
triggers, no chains — reaches +2. Each restriction's price, in all-axes maps: **remote triggers 6**,
reveals 4, single-sector tags 3, chains 1. On the line axis the same order holds (14, 8, 5, 1).
The refusals that remain under G are mostly tag *sharing with other families* (172 maps — a floor
tag also driving a ceiling, a light or a door) and the shapes v1 rejects on purpose (neutral rises,
two-family targets, traps).

## I. Sub-project 4b — the lift variants

| perpetual plats (tags of 53/87) | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| plats | 35 | 20 | 936 |
| travel (high − low) median, p90 | 64, 128 | 128, 352 | 144, 256 |
| rest at high / at low / between / dead | 7 / 13 / 15 / 0 | 14 / 2 / 3 / 1 | 402 / 161 / 370 / 3 |
| neighbors 1 / 2 / 3 / 4–5 / 6+ | 5 / 6 / 17 / 7 / 0 | 13 / 6 / 1 / 0 / 0 | 141 / 125 / 131 / 519 / 20 |
| a 54/89 stop line on the same tag | **100 %** | 0 % | **15.7 %** |
| holds ≥ 1 thing | 62.9 % | 75.0 % | 33.5 % |
| trigger hops 0 / 1 / 2 / 3 / 4–5 / 6+ | 1 / 14 / 20 / 34 / 68 / 80 | 8 / 1 / 39 / 0 / 3 / 2 | 40 / 330 / 177 / 200 / 417 / 530 |

**One-shot lift plats.** The shape probe classifies any plat with a one-shot trigger as `Other`
by construction (its `clean` gate requires repeatable triggers), so the row below is a **what-if**:
each one-shot special rewritten to its repeatable twin (21 → 62, 10 → 88, 122 → 123, 121 → 120 —
the same `EV_DoPlat` type at the same speed from the same side) and the shape re-derived.

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| plats whose lift triggers are all one-shot · what-if shape | 2 · Other 2 | 5 · Pedestal 5 | **183** · Core 4, **Pedestal 60, Barrier 51**, Other 68 |
| plats mixing one-shot and repeatable triggers · what-if shape | 0 | 5 · Core 3, Pedestal 1, Other 1 | 16 · Core 4, Pedestal 6, Barrier 5, Other 1 |

**Read:** the perpetual plat is a real construct (936 sample plats; 53 and 87 are in 51 and 43
sample maps by the lift measurement's §A) but not the retail one: id's maps always pair it with a
stop line, idgames maps do so one time in six, and in the sample it rests at its high floor more
often than at its low one. A one-shot lift, read as if it were repeatable, is **a pedestal or a
barrier three times in five** — a block that lowers once to let the player past or onto it, which is
closer to a Reveal than to a lift.

## J. Not measured (carry forward)

- Chained actions at fire-time heights (§G bounds the exposure).
- `ML_BLOCKING`, monsters, the player's radius and use-reach — `pass` is a pure height test.
- `raiseToTexture` destinations (38 sample actions).
- Ceiling specials, stairs and the donut, beyond their share rows.
- Sound: whether floor actions wake monsters (`sfx_stnmov` every 8 tics through `S_StartSound`).
- Whole-map reachability: the effect classes are local to the target's neighborhood.

## What this says for the IR, the compiler, P7 and the recognizer

1. **One mechanism, one-way, fired once.** Every floor action moves a floor to a destination and
   stops for good, and the corpus authors it that way: 77 % of floor lines are the one-shot forms.
   The IR should state a *one-time action* — a trigger, the targets it drives, and where each goes —
   and nothing about repetition. The compiler emits W1/S1.
2. **Three opening shapes cover the corpus, and the definitions found them rather than assumed
   them.** DropWall 2,544, Reveal 3,157, Bridge 236, LedgeLower 17, anything else **0** — together
   5,954 of 9,874 classified actions (60 %). Closing and mixed, the shapes v1 refuses on purpose, are
   7 %; dead 9 %; the rest are rises and drops that change no route.
3. **Two of the three are the monster closet.** The drop wall is a thin sealed strip between two
   areas (modal 16×64; 1,130 sample pockets open only through one, holding imps, shotgun guys and
   cacodemons by the hundred); the reveal is the sealed one-neighbor cell with the monster standing
   *inside* it (40 % hold a thing) or the sunken pedestal with a prize on it. The map-spec's
   `monster_closets` word has its mechanism, in two forms, and the pedestal has a "rises when you
   arrive / sinks when you press" verb.
4. **The bridge is a platform first.** It rises exactly to the walkway's floor (91 %), copies the
   walkway's flat when it is an "and change" form (76 %), and holds things 38 % of the time.
5. **A trigger is placed, not attached.** 79 % of floor triggers are remote from their target and a
   third are six or more rooms away; walkovers outnumber switches; most walkovers span a threshold.
   The lift's "called from its own face" cannot describe this. The IR needs a trigger placed on a
   named wall or across a named threshold, driving named targets — the `switches.remote_allowed`
   the spec frontmatter already carries.
6. **One trigger drives several targets.** 1,511 tag groups over 9,443 targets, 55 % spanning
   several floors: the multi-sector wall, the row of bars. Refusing shared tags cost the lift
   construct a quarter of its platforms; here it would cost more and gain nothing, and accepting a
   group when each member qualifies on its own is what gate G does. An IR action names a list of
   targets.
7. **Destination vocabulary is the engine's own.** Destinations equal a neighbor's floor exactly
   for 79–91 % of opening targets, and the engine computes them from the neighbors (lowest, highest,
   next-highest, lowest ceiling). The IR says *which* floor the target joins; the compiler chooses the
   special whose rule lands there and refuses when none does. Travel: median 112 (walls), 120
   (reveals), 96 (bridges); p90 under 300 everywhere.
8. **P7 is already the right shape.** A fired action is a state bit: a switch fires it on entering
   the activator sector, a walkover on crossing its threshold, and the target's floor becomes a
   function of the state — the same per-state flood that catches a stranded key holder. Load-time
   destinations are exact for 95.7 % of targets (a movable destination-defining neighbor is 4.3 %);
   the compiler can forbid chains by construction and the verifier can re-derive destinations the
   same way.
9. **Rendering.** The face that moves is the neighbor's lower texture, pegged so it rides with the
   floor (95 %); two thirds of drop walls are exactly floor == ceiling at rest and take the
   neighbor's flat and light (78–80 %); a bridge's riser is unseen at rest and its flat should be
   the walkway's once risen.
10. **Yield to expect.** G lifts all-axes expressibility **8.9 → 9.5 %** (+8 maps) against a
    ceiling of 10.0 %, and the line axis 13.5 → 14.9 %. Remote triggers are the most expensive thing
    to leave out (6 of the 8), then reveals (4), then shared tags (3). The larger blocks that remain
    are tags shared with ceiling, light and door specials (172 maps) and two-family targets (73) —
    later families' territory.
11. **For 4b.** The perpetual plat rests high as often as low, rarely has a stop line in idgames, and
    is a walkover-only construct; a one-shot lift is mostly a pedestal or barrier that moves once —
    the lift variants are small and can follow the reveal's shape.

## Re-running

```bash
cargo build --release --example liftprobe
R=/Users/amir/workspace/crustywad/RETAIL
./target/release/examples/liftprobe floors "DOOM+DOOM2" $R/DOOM.WAD $R/DOOM2.WAD
./target/release/examples/liftprobe floors "Final Doom" $R/TNT.WAD $R/PLUTONIA.WAD
./target/release/examples/liftprobe floors "idgames sample 20260828-400" \
  /Users/amir/workspace/crustywad/xtask/data/samples/20260828-400
```

Markdown to stdout, load failures to stderr. A path may name a directory (swept non-recursively for
`.zip` and `.wad`) or a single archive or WAD.
