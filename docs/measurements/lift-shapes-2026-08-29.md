# How idgames and id's maps build lifts — a corpus measurement

**Date:** 2026-08-29 · **Populations:** `RETAIL/DOOM.WAD` + `DOOM2.WAD` (headline, 68 maps),
`RETAIL/TNT.WAD` + `PLUTONIA.WAD` (Final Doom, 64 maps), and the sample of record
`crustywad/xtask/data/samples/20260828-400/` (1,282 unique maps) · **Tool:**
`examples/liftprobe` in crustygen — `cargo run --release --example liftprobe -- census <label>
<dir>...` for §A–K, `-- shapes` for §L; the numbers below were first produced by throwaway
single-file versions of the same code at main `45c1579` and reproduced by the committed tool
before it was committed · **crustywad:** 0.9.6 ·
**Committed-tool delta:** four sample plats are boundary-less sectors (the scene resolves no
lines for them); the throwaway let their bounding boxes saturate, the committed tool counts them
as Dead plats with no geometry. That moves only the `Other` shape's size lines in the second pass
(four plats); every number cited below is reproduced exactly. ·
**Engine source:** `linuxdoom-1.10` at the pinned commit `a77dfb96`, fetched for this probe
(`p_spec.h`, `p_plats.c`, `p_switch.c`, `p_spec.c`, `p_floor.c`, `p_map.c`, `r_segs.c`).

Sub-project 3 of Project G. The question: what *is* a lift in the maps we have, so that the IR,
the compiler, rule P5 and the recognizer state the same thing.

## Method and its limits

The population is drawn the way `crustygen-corpus` draws it: every `*.zip` opens through
crustywad's archive reader under the same lenient options, every `.wad` member is read, every map
group passes through the same `ingest::load_map` gate, and maps are deduplicated by the same
`sha256:` hash (`map_hash`). The probe does not bucket failures the way the sweep does — an
unreadable archive, WAD or map group is named on stderr and skipped. Retail
WADs are read as bare files through strict `Wad::from_path`. **1,282 unique sample maps load**,
matching the 2026-08-28 expressibility run and the teleport probe exactly. Three sample zips are
not archives (`10817-noreturn`, `3370-teledoom`, `9470-rocket1`) — the same three every sweep
reports.

**Arbiter:** the probe counts **616** sample maps carrying ≥1 linedef with special 62 — the same
616 (48.0 %) the expressibility report's line-blocker table gives for value 62 — and reproduces
the all-axes baseline **96 / 1,282 = 7.5 %** from `docs/measurements/teleports-2026-08-28.md`.

Every special value in this document was transcribed from the fetched source, not recalled: the
`case N:` label and its comment in `P_UseSpecialLine` (`p_switch.c`), `P_CrossSpecialLine` /
`P_ShootSpecialLine` (`p_spec.c`), and the `EV_DoPlat` type it dispatches to (`p_plats.c`).

Definitions the numbers depend on:

- **Lift line** — a linedef whose special dispatches `EV_DoPlat(..., downWaitUpStay | blazeDWUS, ...)`:
  62 (SR), 21 (S1), 88 (WR), 10 (W1), 123 (SR blazing), 122 (S1 blazing), 120 (WR blazing), 121
  (W1 blazing). "S" = use-activated, "W" = walkover, per the dispatcher the case lives in.
- **Plat** — a sector whose tag is named by ≥1 lift line (`P_FindSectorFromLineTag` semantics:
  `sectors[i].tag == line->tag`). A tag-0 lift line is counted as refused, not resolved against
  every untagged sector.
- **Neighbor** — a sector on the other side of one of the plat's two-sided lines, exactly
  `getNextSector` (`p_spec.c`): `if (!(line->flags & ML_TWOSIDED)) return NULL;`. The plat's
  **low** floor is `P_FindLowestFloorSurrounding`'s answer — starts at the plat's own floor, takes
  the minimum over neighbors — so **travel = floor − low ≥ 0** by construction.
- **Rest position** (at load, relative to neighbors, `step` = `data/engine.toml` `max_step_height`
  = 24): **Dead** travel = 0 · **Top** travel > 0 and some neighbor within a step of the plat's
  floor · **AboveAll** travel > 0 and every neighbor more than a step below · **Intermediate**
  travel > 0 and some neighbor more than a step above.
- **Activator** of a trigger line, relative to the plat it drives — the sector a player must stand
  in to fire it. For an S line that is the *front* sector only (`P_UseSpecialLine`: `if (side)`
  returns false for everything but 124). For a W line, whichever side the crossing is possible
  *from* at rest under `P_TryMove`'s step rule (a side more than 24 below the other cannot cross
  onto it). Classified **Low** (floor more than a step below the plat's), **Level** (within a
  step, not the plat), **Plat** (the plat itself), **Above**. The model ignores headroom, blocking
  flags and use-reach distance, and it is a floor-height model — a switch in a low room the
  player cannot actually reach still counts as Low.
- **Placement** — OnPlatFront / OnPlatBack (the plat is that side of the line), Adjacent (a side
  is a neighbor of the plat), Remote.
- "Sector size" is a **bounding box** over the endpoints of every linedef touching the sector.

## Engine facts (fetched, pinned)

- **`p_spec.h`** — `plattype_e { perpetualRaise, downWaitUpStay, raiseAndChange,
  raiseToNearestAndChange, blazeDWUS }`. There is **no up-wait-down-stay type in vanilla.**
  `PLATWAIT 3`, `PLATSPEED FRACUNIT`, `MAXPLATS 30` (`P_AddActivePlat` calls `I_Error("no more
  plats!")` past 30 simultaneously active plats).
- **`p_plats.c`, `EV_DoPlat`, `case downWaitUpStay`** — `plat->speed = PLATSPEED * 4;
  plat->low = P_FindLowestFloorSurrounding(sec); if (plat->low > sec->floorheight) plat->low =
  sec->floorheight; plat->high = sec->floorheight; plat->wait = 35*PLATWAIT; plat->status = down;`
  — **the plat rests HIGH at its load-time floor, is sent DOWN to the lowest neighbor, waits
  105 tics (3 s) and returns.** `blazeDWUS` is identical with `PLATSPEED * 8`. Both remove the
  thinker when the return completes (`T_PlatRaise`, `case up`, `pastdest`). A busy plat ignores
  re-triggers: `if (sec->specialdata) continue;`. A plat authored *at* its lowest neighbor's
  height has `low == high` and travels nothing — a rest-low DWUS lift cannot exist. Going up, a
  blocked plat reverses: `if (res == crushed && (!plat->crush)) { plat->count = plat->wait;
  plat->status = down; }`. `T_PlatRaise`'s `waiting` branch picks the direction by position:
  `if (plat->sector->floorheight == plat->low) plat->status = up; else plat->status = down;`.
- **`p_switch.c`, `P_UseSpecialLine`** — `// Only the front sides of lines are usable.` … `if
  (side) { switch(line->special) { case 124: break; default: return false; } }`. 62 and 123 fire
  from the **front side only**. `case 62: // PlatDownWaitUpStay — EV_DoPlat(line,downWaitUpStay,1)`
  then `P_ChangeSwitchTexture(line,1)`; `case 123: // Blazing PlatDownWaitUpStay —
  EV_DoPlat(line,blazeDWUS,0)`; one-shots `case 21` and `case 122` call
  `P_ChangeSwitchTexture(line,0)`. `P_ChangeSwitchTexture` inspects and swaps
  `sides[line->sidenum[0]]`'s top/mid/bottom texture — the **front** sidedef — against
  `switchlist`; a front sidedef carrying no switch texture is left as it is and the use still
  fires.
- **`p_spec.c`, `P_CrossSpecialLine`** — `case 88: // PlatDownWaitUp — EV_DoPlat(line,
  downWaitUpStay,0)` and `case 120: // Blazing PlatDownWaitUpStay` sit in the RETRIGGERS block
  with no `line->special = 0`; `case 10` and `case 121` are the W1 forms in the TRIGGERS block.
  No side gate: unlike `EV_Teleport`, a crossing from either side fires.
- **`p_map.c`, `P_TryMove`** — `tmfloorz - thing->z > 24*FRACUNIT` rejects; descent is free
  (already sourced as `max_step_height`). A player standing on a raised plat can always **drop**
  off it; only *climbing* onto it needs the lift.
- **`r_segs.c`, `R_StoreWallRange`** — for the bottom texture: `if (linedef->flags &
  ML_DONTPEGBOTTOM) rw_bottomtexturemid = worldtop; else rw_bottomtexturemid = worldlow;` with
  `worldlow = backsector->floorheight - viewz` and `worldtop = frontsector->ceilingheight - viewz`.
  A **pegged** (default) lower texture is anchored to the *back sector's floor* — on a riser seen
  from the low room the back sector is the plat, so the texture **rides with the platform**; an
  unpegged lower is anchored to the ceiling and stays put while the floor moves out from under it.

## A. Special usage

| special (dispatch) | DOOM+DOOM2 lines / maps | Final Doom lines / maps | idgames lines / maps |
|---|---|---|---|
| 62 SR downWaitUpStay | 273 / 45 (**66.2 %**) | 336 / 29 (45.3 %) | 4,191 / 616 (**48.0 %**) |
| 88 WR downWaitUpStay | 141 / 26 (38.2 %) | 106 / 21 (32.8 %) | 1,515 / 326 (25.4 %) |
| 123 SR blazeDWUS | 321 / 19 (27.9 %) | 274 / 46 (**71.9 %**) | 1,900 / 308 (24.0 %) |
| 120 WR blazeDWUS | 63 / 12 (17.6 %) | 97 / 28 (43.8 %) | 529 / 134 (10.5 %) |
| 21 S1 | 1 / 1 | 0 | 148 / 36 (2.8 %) |
| 10 W1 | 1 / 1 | 0 | 27 / 12 (0.9 %) |
| 122 S1 blazing | 0 | 0 | 1 / 1 |
| 121 W1 blazing | 0 | 12 / 7 (10.9 %) | 3 / 2 |
| 53 / 87 perpetualRaise (W1 / WR) | 0 / 10 (5 maps) | 10 / 7 | 129 / 174 (51 / 43 maps) |
| 54 / 89 EV_StopPlat | 0 / 15 (5 maps) | 0 / 0 | 4 / 79 |
| 20 S1 raiseToNearestAndChange | 30 / 19 (27.9 %) | 16 / 13 (20.3 %) | 202 / 119 (9.3 %) |
| 22 W1 raiseToNearestAndChange | 25 / 11 | 7 / 4 | 143 / 70 (5.5 %) |
| 18 S1 raiseFloorToNearest | 18 / 12 (17.6 %) | 5 / 4 | 132 / 94 (7.3 %) |
| 130 W1 raiseFloorTurbo | 0 | 83 / 28 (43.8 %) | 229 / 62 (4.8 %) |
| 23 S1 lowerFloorToLowest | 34 / 21 (30.9 %) | 117 / 38 (59.4 %) | 1,227 / 386 (30.1 %) |
| 38 W1 lowerFloorToLowest | 67 / 16 (23.5 %) | 168 / 41 (64.1 %) | 1,684 / 309 (24.1 %) |
| 102 S1 lowerFloor | 42 / 22 (32.4 %) | 17 / 8 | 262 / 136 (10.6 %) |
| 19 W1 lowerFloor | 38 / 13 | 88 / 13 | 795 / 180 (14.0 %) |

Maps with ≥1 lift line: **53 / 68 (77.9 %)**, **59 / 64 (92.2 %)**, **795 / 1,282 (62.0 %)**. Maps
with a blazing lift line: 29.4 %, **76.6 %**, 25.9 %. Per moving plat, every trigger is a repeatable
form on **99.3 % / 97.5 % / 95.0 %**; plats with any blazing trigger **46.0 % / 46.3 % / 30.1 %**
(all-blazing 43.5 / 45.6 / 27.6 %); plats mixing speeds 7 / 3 / 91.

**Read:** the repeatable pair is the construct (one-shots are 2 lines in 132 retail maps and 2.8 %
of idgames maps); the blazing pair is not a fringe — it is the majority form in Final Doom and a
third of idgames lifts — so `speed: normal | fast` earns its place exactly as it does for doors.
The perpetual and raise-and-change types are a different mechanism (§I).

## B. Tag resolution

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| lift lines, tag 0 | 0 | 0 | 229 (2.8 %) |
| tag → no sector | 6 | 0 | 75 |
| tag → exactly 1 sector | 663 | 666 | 6,692 |
| tag → N sectors | 131 (16 %) | 159 (19 %) | 1,318 (16 %) |
| plats (unique tagged sectors) | 291 | 415 | 3,876 |

One lift line driving several sectors at once is **a sixth of all lift lines** in every population.
Maps with a refused line (tag 0 / unresolved): 1 / 0 / 42.

## C. Rest position and travel

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| Dead (travel 0) | 13 (4.5 %) | 20 (4.8 %) | 244 (6.3 %) |
| **Top** | **167 (57.4 %)** | **244 (58.8 %)** | **2,097 (54.1 %)** |
| AboveAll | 91 (31.3 %) | 122 (29.4 %) | 1,256 (32.4 %) |
| Intermediate | 20 (6.9 %) | 29 (7.0 %) | 279 (7.2 %) |
| moving plats | 278 | 395 | 3,632 |
| travel median (Top) | 128 | 107 | 128 |
| travel ≤ 256 | 93.2 % | 89.1 % | 87.3 % |
| travel ≤ 128 | 72.7 % | 76.7 % | 68.6 % |
| modal travel values | 128 (69), 64 (33), 88 (19) | 128 (86), 64 (53), 96 (17) | 128 (815), 64 (392), 72 (164) |
| travel ≡ 0 mod 8 | 100 % | 78.5 % | 85.5 % |

**AboveAll is a second construct, not a variant of the first.** Of the AboveAll plats, **86.8 % /
85.2 % / 85.8 %** have *every* neighbor at *one* floor — the low floor. They are not lifts that
overshoot a landing (25..32 above the highest neighbor: 4 / 7 / 64 only); they are raised blocks
that **lower on use and rise back**, in two families by neighbor count:

- **Pedestal** — one neighbor (AboveAll × nb1: 60 / 60 / 659): a raised island or alcove floor
  inside one host, called down from the host. 1-neighbor moving plats' travel: median 128 / 80 / 128.
- **Barrier** — two or more neighbors, all at the one low floor (AboveAll × nb2: 27 / 51 / 502,
  nb3+: 4 / 11 / 95): a raised block *between* areas at the same floor, lowered to pass.

**Intermediate** (7 % everywhere) is a plat with both a higher and a lower neighbor beyond a step —
a middle landing in a multi-level arrangement; its highest neighbor sits 64 / 48 / 77 (median)
above it.

**Rest-low lifts.** The corpus has none by the DWUS mechanism, as the engine predicts: the 4.5–6.3 %
Dead plats are the tagged sectors authored at their lowest neighbor's height, and they move
nothing. What *does* start low and rise is the floor-special family (§I).

## D. Topology (moving plats)

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| 1 distinct neighbor | 22.3 % | 17.0 % | 18.7 % |
| **2 distinct neighbors** | **60.1 %** | 48.1 % | **54.5 %** |
| 3 | 10.4 % | 11.4 % | 12.9 % |
| 4+ | 7.2 % | 23.5 % | 13.9 % |
| 2-neighbor plats where one neighbor is level and the other is the low floor | **80.2 %** | 67.9 % | 70.1 % |
| two-sided edges = 2 / = 4 | 33.8 % / 30.6 % | 26.3 % / 47.1 % | 31.2 % / 34.2 % |
| no one-sided edge at all (island) | 37.8 % | 52.9 % | 43.4 % |

Rest × neighbors: **Top × nb2 = 135 / 131 / 1,418** is the single largest cell in every population
(48 % / 33 % / 39 % of moving plats) — the gap between a level room and a low room, i.e. the
portal shape `docs/design.md` §6 already names. Top × nb3 and nb4+ (30 / 106 / 659) are the same
lift with subdivided or additional level-side neighbors.

## E. Size and grid (moving plats)

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| modal bbox (min × max) | 64×64 (46), 128×128 (37), 64×128 (19), 64×192 (14) | 64×64 (82), 8×64 (31), 128×128 (29), 64×128 (21) | 64×64 (715), 128×128 (293), 64×128 (280), 32×64 (80) |
| bbox min side = 64 / < 64 | 33.8 % / 15.1 % | 38.5 % / 35.9 % | 34.5 % / 25.3 % |
| bbox max side ≤ 128 | 63.7 % | 79.3 % | 70.4 % |
| both sides ≡ 0 mod 64 / mod 32 | 54.0 % / 66.2 % | 42.8 % / 57.0 % | 45.8 % / 60.7 % |
| bbox min corner ≡ (0,0) mod 64 | 49.6 % | 35.9 % | 34.7 % |

**Unlike teleport pads (321 / 321 aligned), lifts are not laid on the flat grid** — barely a third
to a half are. The riser texture is what a player sees, not the flat (§G). Final Doom's 8×64 and
idgames' 16–32-wide plats are thin trim strips tagged along with a lift.

## F. Triggers (moving plats)

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| 1 / 2 / 3 / 4+ trigger lines per plat | 28.8 / 25.9 / 11.5 / 33.8 % | 28.4 / 37.2 / 7.6 / 26.8 % | 36.5 / 31.1 / 9.4 / 23.0 % |
| **callable from a Low activator** | **95.3 %** | **89.1 %** | **93.1 %** |
| — of which by a use-line (only) / by a walkover from Low | 191 / 74 | 261 / 91 | 2,815 / 565 |
| callable from a Level activator (top) | 31.3 % | 35.2 % | 28.4 % |
| both Low and Level | 27.7 % | 25.3 % | 23.9 % |
| neither | 1.1 % | 1.0 % | 2.5 % |

Per trigger line (S = use, W = walkover):

| placement | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| **S OnPlatBack** (the riser face, plat behind it) | **476** | **444** | **4,381** |
| S OnPlatFront | 15 | 15 | 232 |
| S Adjacent (a neighbor's wall) | 64 | 472 | 994 |
| S Remote | 134 | 132 | 2,280 |
| W OnPlatFront / OnPlatBack (the plat's own edge) | 72 / 18 | 39 / 66 | 434 / 563 |
| W Adjacent (a line in the neighbor, in front of the lift) | 94 | 292 | 696 |
| W Remote | 44 | 121 | 1,173 |
| use-lines one-sided / two-sided | 78 / 611 | 135 / 928 | 901 / 6,986 |
| S activators: Low / Level / Plat / Above | 638 / 26 / 15 / 10 | 970 / 69 / 15 / 9 | 6,956 / 555 / 232 / 144 |
| W activators: Low / Level / Plat / Above | 125 / 94 / 90 / 8 | 335 / 180 / 105 / 10 | 1,259 / 1,145 / 966 / 466 |

Trigger-set combos per plat (`S`/`W` = special kind, `!` = blazing, `@plat` on the plat's edge,
`@adj` on a neighbor, `@remote`; the suffix is the activator):

| combo | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| `S@plat/Low` — use the riser from below | 15.8 % | 14.2 % | **22.1 %** |
| `S!@plat/Low` — same, blazing | 15.1 % | 13.7 % | 10.6 % |
| `S@plat/Low + W@plat/Level` — riser switch below, walkover on the top edge | 7.6 % | 6.6 % | 9.4 % |
| `S@remote/Low` — a wall switch elsewhere in the low area | 2.5 % | 3.3 % | 10.2 % |
| `W!@adj/Low` / `W@adj/Low` — a walkover across the approach | 5.4 % | 2.0 % | 2.4 % |
| `S!@plat/Low + W!@plat/Level` | 1.4 % | 3.8 % | 2.2 % |

Switch textures: of 595 / 610 / 6,011 use-lines, only **17.6 % / 22.0 % / 20.7 %** carry an
`SW1*`/`SW2*` texture on the front sidedef (slot when present: bottom 31 / 44 / 602, middle
58 / 79 / 562, top 16 / 11 / 80). **Four lift switches in five are an unmarked riser face**: the
player "uses" the lift's own lower texture. When marked, the switch sits in the *lower* slot on a
two-sided riser or the *middle* of a one-sided wall.

**Read:** the canonical lift is called **from the bottom by using its riser** — the 62/123 line is
the plat's low edge, the plat is its *back* sector, and the front sidedef faces the low room. A top
trigger is present on a quarter to a third of lifts, and when it is, it is a walkover on the top
edge (`W@plat/Level`): stepping onto the plat from the level side sends it down. Because descent
is free (`P_TryMove`), a top trigger is a convenience, never a necessity; P5's "operable from both
ends" is satisfied by the corpus as "callable from below", which 89–95 % of lifts are.

## G. Rendering-facing (moving plats)

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| riser boundaries (neighbor floor below the plat) | 809 | 1,118 | 10,937 |
| visible lower texture missing (`-`) | 0.5 % | 0.0 % | 1.3 % |
| **riser lower-unpegged** | **3.7 %** | **3.8 %** | **4.4 %** |
| plat's own sidedef also carries a lower (redundant) | 35.2 % | 16.7 % | 15.7 % |
| top riser textures | SUPPORT3 211, MARBLE2 130, ROCK5 44, METAL2 22, PLAT1 22 | METAL 113, SILVER1 107, SUPPORT3 98, SUPPORT2 80, SHAWN2 59, PLAT1 37 | SUPPORT3 946, SUPPORT2 519, METAL 512, PLAT1 467, SHAWN2 313 |
| plat flat == level neighbor's / == low neighbor's | 41.8 % / 18.7 % | 60.7 % / 34.7 % | 44.8 % / 27.5 % |
| light equal to every neighbor | 37.4 % | 47.6 % | 49.8 % |
| ceiling equal to every neighbor | 33.5 % | 31.9 % | 42.1 % |
| plat sector special ≠ 0 | 10.8 % | 11.1 % | 9.2 % |

**Read:** risers are **pegged** (the default flag state) 96 % of the time, which per `r_segs.c`
makes the riser texture ride with the platform — the opposite of a door track, which is
lower-unpegged so `DOORTRAK` stays put. P11's "flags appropriate to the moving sector" therefore
means *leave the lift riser pegged*. `SUPPORT3` is the riser texture of record in DOOM/DOOM2 and
idgames (`PLAT1` — the texture literally named for it — is fifth). A lift usually takes the level
room's flat, often its own light, and frequently its own ceiling (a shaft).

## H. Conflicts

Moving plats whose tag is also the target of a non-lift tagged special: **5.8 % / 5.6 % / 7.4 %**.
The other specials, most often: 97 (5 / 8 / 83 — a teleport destination that is also a lift), 126
(0 / 7 / 16), 18 and 23 (idgames 40 / 26 — a floor action on the same sector), 138/139 (light
changes, 21 / 20).

## I. The floor-special "up lift" — what a rest-low lift is in vanilla

| tagged sectors | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| targeted by a raise-to-nearest / raise-and-change special only | 93 (86 rest below a neighbor) | 121 (113) | 1,473 (1,137) |
| targeted by a lower special only | 314 | 448 | 6,868 |
| targeted by **both** a raise and a lower special | **0** | 13 in 8 maps (12.5 %) | 138 in **69 maps (5.4 %)** |

A sector that starts low and rises on a trigger is, in the maps, a *floor raise* (one-way, stays
up: the bridge, the step that comes up, the pillar that rises to block) — sub-project 4's
tagged-floor family, not a lift. The genuine two-way floor elevator (a raise *and* a lower special
on one sector) appears in **0 of 68 DOOM+DOOM2 maps** and 5.4 % of idgames maps. It is real, rare,
and a different construct: two one-way actions on one sector rather than one round trip.

## J. Naïve arbiter — every 62/88/120/123 line accepted

| idgames, U = 1,282 | now | + 62/88 | + 62/88/120/123 |
|---|---|---|---|
| line axis | 116 (9.0 %) | 156 (12.2 %) | **173 (13.5 %)** |
| all axes | 96 (7.5 %) | 122 (9.5 %) | **133 (10.4 %)** |

Retail: DOOM+DOOM2 0 → 1 → 1 map; Final Doom 0 throughout (retail maps remain 0 % expressible on
other axes). This is the ceiling: the yield if the recognizer accepted every lift line.

## K. Shapes and the shape-gated arbiter

A plat is **Core** when it rests Top, is callable from a Low activator, every trigger is repeatable,
all triggers share one speed, and no other tagged action targets its sector. **Pedestal** and
**Barrier** are the AboveAll one-floor shapes under the same trigger and conflict conditions.
Everything else (Dead included) is **Other**.

| moving plats | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| Core | 147 (52.9 %) | 194 (49.1 %) | 1,808 (49.8 %) |
| Pedestal | 50 (18.0 %) | 44 (11.1 %) | 498 (13.7 %) |
| Barrier | 18 (6.5 %) | 40 (10.1 %) | 322 (8.9 %) |
| Other | 63 (22.7 %) | 117 (29.6 %) | 1,004 (27.6 %) |
| — not callable from Low | 13 | 43 | 252 |
| — Intermediate rest | 19 | 26 | 205 |
| — conflicting tagged action | 12 | 21 | 168 |
| — AboveAll with neighbors at several floors | 12 | 15 | 149 |
| — one-shot trigger | 2 | 9 | 146 |
| — mixed speed | 5 | 3 | 84 |

Under map-level atomicity (every plat of a map must be accepted, and no refused line):

| lift maps | DOOM+DOOM2 (53) | Final Doom (59) | idgames (795) |
|---|---|---|---|
| every plat Core | 11 (20.8 %) | 7 (11.9 %) | 199 (25.0 %) |
| every plat Core ∪ Pedestal ∪ Barrier | 17 (32.1 %) | 15 (25.4 %) | 367 (46.2 %) |

| idgames, U = 1,282 | now | Core only | Core ∪ Pedestal ∪ Barrier | naïve ceiling |
|---|---|---|---|---|
| line axis | 9.0 % | 127 (9.9 %) | 154 (12.0 %) | 173 (13.5 %) |
| **all axes** | **7.5 %** | **101 (7.9 %)** | **120 (9.4 %)** | 133 (10.4 %) |

**Read:** the core lift alone moves the all-axes number by **0.4 points**; adding the two AboveAll
shapes moves it by **1.9 points** — because a map with lifts usually has a pedestal or a barrier
too, and atomicity fails the map on the shape it lacks. Geometrically the pedestal is an island —
the sector-with-hole the teleport pad already taught the compiler to cut (`SectorOut::host`) — with
a raised floor and 62 on every edge instead of a `GATE` flat and 97 (its size distribution was not
measured separately from the other shapes; see "Not measured"); the barrier is a plain portal's
gap sector with a raised floor and 62 on both faces. Neither needs new geometry machinery.

## What this says for the IR, the compiler, P5 and the recognizer

1. **One mechanism, three shapes.** Every lift in the corpus is `downWaitUpStay` resting at its
   *high* floor. The IR should state exactly that: a sector with a rest floor, a travel to the
   lowest neighbor, a speed, and where it can be called from. The three shapes differ only in what
   the neighbors are: a level room and a low room (**lift**, a portal); one low host (**pedestal**,
   an island); two low areas (**barrier**, a portal between rooms at one floor). A `rest: low`
   option would be a lie — the engine has no such plat; a rest-low riser is sub-project 4's floor
   raise.
2. **Trigger vocabulary from the data**, not the template's three words: `switch` = 62/123 on the
   plat's low edge with the plat as *back* sector (the dominant form, 4,381 lines) — no switch
   texture needed, though one may be requested; `walkover` = 88/120 across the approach in the low
   room (`W@adj/Low`), which is how a walkover lift is *called* (a walkover on the plat's own low
   edge cannot fire from below — the step blocks the crossing); `both_ends` = the riser switch plus
   88/120 on the *top* edge (`W@plat/Level`, the canonical third combo). Remote wall switches
   (10 % of idgames plats) are `switches.remote_allowed` territory — a later construct, and a
   recognizer question (accept when the switch's room is a Low activator?).
3. **Speed.** `speed: normal | fast` → 62/88 vs 123/120, mirroring `doors.speed`; PLATSPEED×4 vs ×8
   and PLATWAIT go into `engine.toml` with these citations. One-shot forms stay unemitted and are
   refused on lift (2.8 % of idgames maps carry one).
4. **P5, sharpened by the engine.** A DWUS plat's lowered floor is *exactly* the lowest neighbor's,
   and its raised floor is its own — so a clean gap sector between a level room and a low room
   meets P1 at both ends by construction, and P5 reduces to: travel ≤ `max_travel` (256 covers
   87–93 %; the modal travel is 128), a Low-side trigger exists (the trap rule), and the plat's
   neighbors are only the rooms it joins (a third neighbor at another floor changes `low`). A top
   trigger is optional and never load-bearing for reachability, because descent is free.
5. **Reachability.** Level room → plat is an ordinary Open edge (equal floors at rest); plat → low
   room is a free drop; low room → plat is passable *iff* a Low-side trigger exists — a
   bidirectional edge that the step rule would otherwise refuse. Modeling it as `EdgeKind::Lift`
   with "passable when callable from this side" keeps `passable()` honest and lets the verifier
   flood the emitted map the same way. A pedestal is the same edge from its host; a barrier is two
   such edges.
6. **Pegging and textures.** Riser lower textures stay **pegged** (P11 for lifts is the inverse of
   the door track); the riser texture is the theme's `SUPPORT3`-class texture, sourced from the
   plat's own `wall_tex` per `docs/verticality.md`'s rule that every compiler-made sector carries
   one; the plat takes the level room's flat, light and ceiling unless told otherwise.
7. **Recognizer refusals, by count**: tag-0 and unresolved lines (2.8 % + 1 % of idgames lift
   lines); Dead plats (6 %); one-shot and mixed-speed triggers; a tag driving several sectors
   (16 % of lines — accept when the sectors are mutually adjacent at one floor, i.e. one platform
   split by trim; refuse otherwise); Intermediate rest (7 %); AboveAll with neighbors at several
   floors (4 %); conflicting tagged actions (7 %); not callable from Low (7 %, the top-only lifts
   P5 forbids emitting — the lifter cannot state them without a `trigger: top_only` that P5 would
   have to accept only when P7 holds for the low region).
8. **Yield to expect.** 7.5 % → **9.4 %** all-axes (line axis 9.0 % → 12.0 %) with all three
   shapes; 7.9 % with the lift portal alone. The ceiling is 10.4 %.

## Not measured (carry forward)

- Things standing on pedestals (what a pedestal *holds*) and whether barrier plats carry
  `ML_BLOCKING` fences on top — needed before the pedestal/barrier IR is written.
- Jamb (one-sided side wall) pegging on lift shafts, and middle-texture behavior as the plat
  descends; sidedef `x_offset`/`y_offset` on risers.
- Sound: which plats have `ML_SOUNDBLOCK` neighbors; whether lifts wake monsters (`sfx_pstart`
  through `S_StartSound` at the sector's `soundorg` is unconditional — P_NoiseAlert is a separate
  mechanism and was not examined).
- Multi-sector tags: how many of the 16 % are one platform split by trim versus several lifts on
  one switch.
- The remote-switch distance and room (is the switch's room the low room?).
- UDMF-origin maps in the sample (66) read Doom-numbered specials only, as in the teleport probe.

---

## L. Per-shape follow-up (second pass, `liftprobe shapes`)

Same populations, same load rules; shapes classified as in §K. "Host" is the level neighbor for
Core, the single low neighbor for Pedestal, any neighbor for Barrier.

**Multi-sector tag groups** (a lift tag naming ≥ 2 sectors): 29 / 47 / 328 groups. One platform
split by trim (all at one floor *and* mutually connected): **3.4 % / 8.5 % / 7.6 %**. Several lifts
on one trigger (one floor, disconnected): 55.2 % / 40.4 % / 48.2 %. Several floors: 41.4 % /
51.1 % / 44.2 %. Plats whose tag is shared: Core 17 % / 34 % / 17 %; Pedestal 16 % / 23 % / 27 %;
Barrier 28 % / 40 % / 29 %. **The "one platform split by trim" case the recognizer could accept is
rare; the common case is genuinely one trigger driving several sectors.**

| Core | DOOM+DOOM2 (147) | Final Doom (194) | idgames (1,808) |
|---|---|---|---|
| modal bbox | 128×128 (25), 64×128 (19), 64×64 (15) | 64×64 (39), 128×128 (19), 64×128 (15) | 64×64 (366), 128×128 (192), 64×128 (190) |
| min side = 64 / 65..128 / < 64 | 52 / 67 / 13 | 82 / 49 / 52 | 698 / 563 / 308 |
| travel | median 128, p90 248 | median 120, p90 416 | median 128, p90 336 |
| island | 30.6 % | 53.6 % | 37.4 % |
| holds ≥ 1 thing | 40.8 % (imp 42, shotgun_guy 28, pinky 12) | 30.4 % (pinky 28, baron 12, candle 10) | 31.2 % (imp 127, armor_bonus 99, shells 68) |
| any fence (`ML_BLOCKING` two-sided edge) | 0 | 0.5 % | 2.5 % |
| low activators among neighbors: 1 of 2 / 0 of 2 | 102 / 24 | 88 / 27 | 1,057 / 195 |
| any top-side trigger (Level or Plat activator) | 47.6 % | 44.8 % | 42.0 % |
| any use-line with an SW texture | 29.9 % | 33.5 % | 28.8 % |
| riser textures | SUPPORT3 155, PLAT1 22, TANROCK5 14 | SILVER1 67, METAL 50, SUPPORT2 47, SUPPORT3 39, PLAT1 30 | SUPPORT3 498, SUPPORT2 326, PLAT1 298 |
| light == host / flat == host / ceiling == every neighbor | 60.5 / 40.1 / 15.6 % | 75.3 / 61.9 / 19.1 % | 73.6 / 43.8 / 33.2 % |

| Pedestal | DOOM+DOOM2 (50) | Final Doom (44) | idgames (498) |
|---|---|---|---|
| modal bbox | **64×64 (21)**, 16×32 (3), 24×24 (3) | **64×64 (14)**, 32×32 (10), 16×16 (5) | **64×64 (179)**, 128×128 (26), 64×128 (23) |
| island | **82.0 %** | 70.5 % | 74.1 % |
| travel (rise) | median 128, p90 152, max 176 | median 128, p90 256 | median 128, p90 344 |
| holds ≥ 1 thing | **76.0 %** (imp 32, box_of_rockets 6, hell_knight 5, rocket 5) | 70.5 % (chaingunner 7, blue_armor 4, shotgun_guy 4) | **70.3 %** (archvile 403, imp 64, hell_knight 41, medikit 38, box_of_rockets 34, rocket_launcher 28, soulsphere 28) |
| things per plat: 1 / 2 / 4+ | 22 / 5 / 11 | 19 / 7 / 4 | 203 / 52 / 67 |
| any fence | 0 | 0 | 0.6 % |
| edges carrying the special: all / none | 8/8: 18, 4/4: 3, 5/5: 3 / 0/4: 8 | 4/4: 13, 6/6: 2 / 0/4: 9 | 4/4: 103, 8/8: 19 / 0/4: 127 |
| called from the host / from elsewhere | 42 / 8 | 36 / 8 | 328 / 170 |
| any top-side trigger | 12.0 % | 4.5 % | 10.0 % |
| riser textures | MARBLE2 128, ROCK5 37, SUPPORT3 37 | SUPPORT3 38, METAL2 32, SHAWN2 22 | SUPPORT3 187, ROCKRED1 144, METAL 122 |
| light == host / flat == host / ceiling == every neighbor | 48.0 / 14.0 / **78.0 %** | 61.4 / 18.2 / **88.6 %** | 68.5 / 32.7 / **71.3 %** |

| Barrier | DOOM+DOOM2 (18) | Final Doom (40) | idgames (322) |
|---|---|---|---|
| modal bbox | 8×64 (3), 128×192 (2), 64×64 (2) | 64×64 (5), 16×64 (4), 64×192 (4) | 64×64 (50), 16×64 (35), 16×128 (28), 8×64 (27) |
| min side < 64 | 7 of 18 | 20 of 40 | 185 of 322 (57 %) |
| travel (rise) | median 80, p90 128 | median 128, p90 128 | median 92, p90 128 |
| holds ≥ 1 thing | 11.1 % | 32.5 % (lost_soul 12) | 17.4 % |
| **any fence** | **0** | **0** | **0** |
| low activators: 2 of 2 / 1 of 2 / 0 of 2 | 8 / 4 / 3 | 17 / 10 / 6 | **159 / 78 / 35** |
| edges carrying the special: 2/2 / 1/2 / 0/2 | 5 / 1 / 3 | 9 / 9 / 6 | 122 / 54 / 34 |
| any top-side trigger | 0 | 20.0 % | 9.0 % |
| any use-line with an SW texture | 33.3 % | 50.0 % | 35.7 % |
| riser textures | ROCK3 7, SILVER1 6, TANROCK5 5 | MARBGRAY 22, COMPTALL 13, WOODMET1 13 | **SW1SATYR 78**, FIREMAG1 72, BFALL1 70 |
| light == host / flat == host / ceiling == every neighbor | 50.0 / 22.2 / 44.4 % | 55.0 / 50.0 / 47.5 % | 58.7 / 43.8 / 51.6 % |

**Read.**
- The **pedestal** is a 64×64 island (three quarters have no one-sided edge) under the host's
  ceiling, risen 128, called from its host by using any edge (the special is on *every* edge when
  it is on the plat at all), and it **holds something** three times in four: a monster (archviles
  by the hundred in idgames, imps in DOOM/DOOM2) or a reward (rockets, soulsphere, armor). So a
  pedestal is a *content* construct — `things` on it are the point — and the IR must place things
  on the plat, at the raised floor, which the compiler's `place_things` currently does per room.
- The **barrier** is a thin raised strip (min side < 64 in more than half of idgames cases: 16×64,
  16×128, 8×64) between two areas at one floor, risen 64–128, carrying **no fence**, its riser often
  *being* a switch texture (`SW1SATYR` is the top idgames riser). It is callable from **both** sides
  half the time and **one side only** a quarter of the time — the latter is a one-way gate the IR
  must be able to say, or refuse.
- The **core lift**'s footprint is 64 deep (min side 64 most often) and 64–128 wide; a shaft with
  its own ceiling two times in three; risers `SUPPORT3`/`SUPPORT2`/`PLAT1`; a top-side trigger on
  ~45 %; things on it a third of the time (monsters standing on the lift).
- Shared tags are common enough (17–40 % of plats by shape) that refusing them all costs yield;
  accepting only the split-platform case (3–9 % of groups) recovers little. A later construct may
  let one trigger name several lifts.
