# How idgames and id's maps use perpetual plats and one-shot lifts — a corpus measurement

**Date:** 2026-09-11 · **Populations:** `RETAIL/DOOM.WAD` + `DOOM2.WAD` (68 maps), `RETAIL/TNT.WAD` +
`PLUTONIA.WAD` (Final Doom, 64 maps), and the sample of record
`crustywad/xtask/data/samples/20260828-400/` (1,282 unique maps) · **Tool:** `examples/liftprobe`
in crustygen — `cargo run --release --example liftprobe -- variants <label> <dir-or-file>...`,
committed with this document · **crustywad:** 0.9.6 · **Engine source:** `linuxdoom-1.10` at the
pinned commit `a77dfb96`, read for this probe (`p_spec.h`, `p_spec.c`, `p_plats.c`, `p_switch.c`).

Sub-project 4b of Project G (issue #72). The question: what *are* the two lift forms the lift work
left out — the **perpetual plat** (`perpetualRaise`, started by specials 53/87 and stopped by 54/89)
and the **one-shot `downWaitUpStay` lift** (21/10, and the blazing 122/121) — in the maps we have:
how their tags resolve, where they rest and how far they travel, where their start and stop lines
sit, what faces they show and what stands on them, how many run at once, and what admitting each
form could add to the expressibility figure. The floor measurement
([`floor-shapes-2026-09-02.md`](floor-shapes-2026-09-02.md), §I) counted them; this document
measures them. It records facts for the 4b design brainstorm and makes no design decision.

## Method and its limits

The population is drawn the way `crustygen-corpus` draws it: every `*.zip` opens through crustywad's
archive reader under the same lenient options, every `.wad` member is read, every map group passes
through the same `ingest::load_map` gate, and maps are deduplicated by the same `sha256:` hash. An
unreadable archive, WAD or map group is named on stderr and skipped; the sample's failures are the
same ones every sweep reports. Retail WADs are read as bare files from scratch copies (`RETAIL/` is
only ever read). **1,282 unique sample maps load**, matching every earlier run.

**Arbiters.** Before reporting anything new, the pass reproduces the figures earlier documents
recorded, on the same code paths (`floors.rs`'s §I helpers, now shared rather than copied):

| figure | recorded | reproduced |
|---|---|---|
| perpetual plats, DOOM+DOOM2 / Final Doom / idgames ([floor-shapes](floor-shapes-2026-09-02.md) §I) | 35 / 20 / 936 | **35 / 20 / 936** |
| idgames rest at high / at low / between / dead | 402 / 161 / 370 / 3 | **402 / 161 / 370 / 3** |
| plats with a 54/89 stop line on the tag | 100 % / 0 % / 15.7 % | **100.0 % / 0.0 % / 15.7 %** |
| all-one-shot lift plats | 2 / 5 / 183 | **2 / 5 / 183** |
| idgames all-one-shot what-if shape Core / Pedestal / Barrier / Other | 4 / 60 / 51 / 68 | **4 / 60 / 51 / 68** |
| idgames line axis after 4a ([floors-2026-09-03](floors-2026-09-03.md) "Before and after") | 187 (14.6 %) | **187 (14.6 %)** |
| idgames all axes, floors ignored (the naïve figure) | 122 (9.5 %) | **122 (9.5 %)** |
| idgames all axes, six axes (the **honest** figure) | 119 (9.3 %) | **119 (9.3 %)** |
| lift tag groups, DOOM+DOOM2 / Final Doom / idgames ([lift-shapes](lift-shapes-2026-08-29.md) §L, `shapes.rs`) | 29 / 47 / 328 | **29 / 47 / 328** |
| — split / one floor disconnected / several floors | 1·16·12, 4·19·24, 25·158·145 | **1·16·12, 4·19·24, 25·158·145** |
| idgames `SharedTag` platform refusals ([lifts-2026-08-30](lifts-2026-08-30.md)) | 923 | **923** |
| idgames `shared_split` groups (recognizer) | 25 | **25** |
| single-tag platforms where §I's re-derived verdict disagrees with `lift::plat` | 0 | **0 / 0 / 0** |

Every one reproduces to the unit. The honest 119 is obtained the way `crustygen-corpus` obtains it
(`src/lift/corpus.rs`): the teleport, plat *and floor* recognizers each run when their specials are
present. The floor pass's own arbiter row omits the floor recognizer, which is why it reads 122;
both are printed so neither can be mistaken for the other.

Every special value below was transcribed from the fetched source, not recalled; the engine layer of
the probe cites `file:line` beside each constant (see "Engine facts" below).

Limits the numbers carry (also the probe's §J):

- **Load-time heights.** `low` and `high` are the bounds `EV_DoPlat` would compute when the start
  line first fires at the heights the map loads with; a neighbor that has moved by then is not
  modeled.
- **`P_Random()&1`.** A perpetual plat's first direction is random. "Rest at low / at high" says
  where it starts, not which way it goes first.
- **Stasis semantics.** `EV_StopPlat` freezes every active plat carrying the tag, of any type.
  Whether a stop line ever fires before its start line, or freezes a DWUS lift on a shared tag, is
  not modeled.
- **The probe's own walking model is a pure height test.** For the descriptive rows (§A–§G:
  placement, activators, hops, the strand test) `ML_BLOCKING` fences, monsters, the player's
  radius and use-reach all read as passable, and hops are pure two-sided adjacency. The §H arbiter
  and the §I bank gates are the exception: they run the shipped recognizers over the verifier's
  scene (`check::plats::resolve_plats`), whose activators honor `Boundary::passable` and the
  dead-end-pocket rule, so those yields and A / A′ / B counts carry the verifier's geometry, not
  the probe's.
- **UDMF-origin maps** (66 in the sample) are read with Doom special numbers.

## Definitions the numbers depend on

- **Start line** — a linedef whose special is 53 or 87. **Stop line** — 54 or 89. Both are walkovers
  from either side (`P_CrossSpecialLine`); no use or gun form dispatches `perpetualRaise` or
  `EV_StopPlat`. **Perpetual plat** — a sector whose tag a start line names (a tag-0 line names
  nothing; a tag naming no sector is *dangling*). A tag naming several sectors is several plats.
- **`low` / `high`** — as `EV_DoPlat` computes them (`p_plats.c:233-247`):
  `P_FindLowestFloorSurrounding` clamped *up* to the sector's own floor, and
  `P_FindHighestFloorSurrounding` clamped *down* to it. **Travel** = `high − low`. **Rest** — *at
  low* (`floor == low < high`), *at high* (`low < high == floor`), *between*, or *dead*
  (`low == high`).
- **Shape class** — *two-room*: `low` is exactly one neighbor's floor and `high` exactly one
  neighbor's, with `low < high`; *island*: exactly one neighbor; *residual*: everything else.
- **Placement / activator / hops** — as the lift and floor measurements define them. Placement is
  OnPlatFront / OnPlatBack (the plat is that side of the line), Adjacent (a side is a neighbor of
  the plat), Remote. An activator is a sector a player can cross the line *from* at rest under the
  step rule, classed Low / Level / Above by floor against the plat's, or Plat; hops are the fewest
  two-sided crossings from the nearest activator sector to the plat, heights ignored,
  *unreachable* when no activator sector is connected to the plat. **Per-line rows count a line
  once per plat its tag names** (a tag naming three sectors makes three pairs).
- **Boardable at rest** — some neighbor's floor is within one step (24, `data/engine.toml`) of the
  plat's rest floor.
- **Face** — a two-sided boundary to a neighbor at another floor. The visible lower is on the sidedef
  of the lower-floor sector (the neighbor's when the plat stands above it, the plat's own when it sits
  below), the convention `r_segs.c` draws by. **Flat shared with** — a neighbor at the lowest
  neighboring floor, else at the highest, else any other neighbor, else none.
- **Thing classes** — by what `Tables` knows about the thing's name: *monster* (`species`), *prop*
  (`prop`), *pickup* (health/armor, ammo, weapons, keys, the backpack and the chainsaw), *start*
  (player and deathmatch starts), *other* (powerups, the teleport marker, gore, the candle, and
  types the vocabulary does not name — its top names are printed).
- **One-shot plat rows** — as §I of the floor measurement: *all-one-shot* (every lift trigger is
  21/10/122/121) and *mixed*. The **what-if shape** rewrites each one-shot special to its repeatable
  twin (21 → 62, 10 → 88, 122 → 123, 121 → 120) and re-derives `analyze_plat`'s shape. **Strand
  test** (what-if Core): whether the plat's level room(s) and low room(s) stay connected in the hop
  graph with the plat's sector removed. **Barrier faces**: the distinct neighbor sides of the lift
  lines on the plat's own boundary; **Low-firing neighbors**: neighbors from which some trigger
  fires with a Low activator.
- **Lift tag group (bank)** — a DWUS/blaze lift tag (`is_lift`) naming two or more sectors; its
  **members** are those sectors, each a `ScenePlat` of its own (`p_plats.c:164-169` makes one
  `plat_t` per tagged sector, with its own `low` from its own neighbors, `:207-212`). **Judged as
  if unshared** — `lift::plat`'s eight refusals re-derived in its order (`src/lift/plat.rs:418-442`)
  with the `SharedTag` arm skipped; the probe checks its re-derivation against the recognizer on
  every single-tag platform and reports the disagreements (0). **Own line** — a lift line with the
  member as a side; **own Low line** — one that also fires from a `Low` activator relative to the
  member; **callable from Low** — `ScenePlat::callable_low` (`src/check/plats.rs:149`) over every
  line naming the tag, wherever it sits; **within one hop** — a line on the member's face or with
  a side among its neighbors. **Reach** of a line to the group — on a member face / adjacent to a
  member / remote from every member. **Floor class** — split (one floor, mutually adjacent), one
  floor disconnected, several floors, as `shapes.rs` and `lift::plat` define them. **Outside
  neighbors** — a member's two-sided neighbors that are not themselves members.
- **§I gates** — **A**: every member passes alone *and* has a Low-activator line on its own face
  (the shipped construct's "called from its own face", per member); **A′ (neighbor-called)**: every
  member passes alone *and* some lift line naming the tag fires — per `activator_sides` /
  `Dispatch` semantics, Use from the front side only, Cross from either side that can cross at
  rest — from a sector that is **both** a two-sided neighbor of that member **and** stands at the
  member's own `low`: adjacency plus height, the line itself anywhere (another member's face, the
  host room's wall); **B**: every member passes alone, wherever its callers are (the floor recognizer's rule, `src/lift/floor.rs:63-67`);
  **split as one lift**: a split group read as one platform — the union of the members' outside
  neighbors, the least member `low`, the shared line set with an activator that is itself a member
  counted as the platform — passing the same eight refusals. A column's lift axis holds when no
  lift line is broken, every single-tag platform passes the recognizer as today, and every group
  passes the column's gate; the line axis is unchanged. **Recovered** counts the platforms the
  recognizer refuses `SharedTag` today whose group the column accepts (a group member refused
  `Dead` — precedence 1 — is not among the 923 and is not counted).
- **§H columns** — *line axis*: every out-of-set special the map uses is in the column; *all axes
  as today*: that, and the six axes as shipped (a one-shot plat is still refused by the plat
  recognizer, a perpetual plat is not judged at all); *provisional*: one-shot triggers read as
  repeatable (the recognizer re-run on the twin-rewritten map), and every perpetual plat passing a
  **loose provisional gate** — tag resolves to exactly one sector, not dead, no stop line on the
  tag, no other family (lift, floor or anything else) on the tag, rest at low or at high, and no
  tag-0 or dangling start line in the map. **The provisional gate is a ceiling, not the design.**

Denominators: **U** = unique maps (68 / 64 / 1,282); **plats** = perpetual plats 35 / 20 / 936;
(start line, plat) pairs 84 / 53 / 1,480; (stop line, plat) pairs 133 / 0 / 265; lift tag
**groups** 29 / 47 / 328 with **members** 64 / 141 / 1,085.

---

## A. Perpetual tag resolution and conflicts

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| 53/87 start lines | 10 | 17 | 303 |
| — tagged 0 / naming no sector | 0 / 0 | 0 / 0 | 0 / **1** |
| resolving start tags naming 1 / 2 / 3+ sectors | 2 / 0 / 3 | 7 / 0 / 1 | 79 / 19 / 49 |
| **perpetual plats** · maps with ≥ 1 | **35** · 5 (7.4 %) | **20** · 5 (7.8 %) | **936** · 90 (7.0 %) |
| plats whose tag names 1 / 2 / 3+ sectors | 2 / 0 / 33 | 7 / 0 / 13 | 79 / 38 / **819** |
| plats whose tag also carries a DWUS/blaze lift line | 0 | 0 | 8 (0.9 %) |
| plats whose tag also carries a non-plat tagged special | 0 | 0 | 44 (4.7 %) |
| — those specials | | | 38 ×19, 104 ×8, 12 ×8, 126 ×4, 242 ×3, 272 ×3, 101 ×2, 71 ×2, 97 ×2, 271 ×1 |
| stop lines per plat tag 0 / 1 / 2 / 3+ | 0 / 10 / 0 / 25 | 20 / 0 / 0 / 0 | 789 / 67 / 72 / 8 |
| — form among plats with a stop line W1 only / WR only / both | 0 / 35 / 0 | — | 17 / 130 / 0 |

**Read:** a start line names a *bank*: in DOOM+DOOM2 three of five start tags name three or more
sectors and produce 35 plats from 10 lines; in the sample 68 of 147 resolving tags name several
sectors, and because those tags are the big ones, **857 of 936 plats (91.6 %) share their tag**
with at least one other sector — 819 with two or more (the largest single map names 360, §F). Tag resolution is otherwise clean — one dangling
line in 303 — and tag sharing with another family is rare (4.7 % of plats; the W1 lower-to-lowest
38 leads, then the light specials 104 and 12). Every retail plat has stop lines (all WR); Final Doom
never uses one; the sample pairs one plat in six with a stop, and where it does the stop is WR
seven times in eight.

## B. Perpetual shape

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| rest at high / at low / between / dead | 7 / 13 / 15 / 0 | 14 / 2 / 3 / 1 | **402 / 161 / 370 / 3** |
| neighbors 1 / 2 / 3 / 4–5 / 6+ | 5 / 6 / 17 / 7 / 0 | 13 / 6 / 1 / 0 / 0 | 141 / 125 / 131 / **519** / 20 |
| distinct neighbor floors 1 / 2 / 3+ | 9 / 10 / 16 | 14 / 6 / 0 | 198 / 152 / **586** |
| class two-room / island / residual | 12 / 5 / 18 | 5 / 13 / 2 | 366 / 141 / 429 |
| travel median, p90 (min–max) | 64, 128 (40–384) | 128, 352 (0–576) | **144, 256** (0–1,384) |
| travel ≤ 24 / 25–64 / 65–128 / 129–256 / > 256 | 0 / 18 / 16 / 0 / 1 | 1 / 0 / 10 / 5 / 4 | 9 / 98 / 331 / 426 / 72 |

Rest × class:

| rest × class | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| at high × two-room / island / residual | 0 / 4 / 3 | 0 / 13 / 1 | 39 / **132** / **231** |
| at low × two-room / island / residual | 1 / 1 / 11 | 2 / 0 / 0 | 29 / 8 / 124 |
| between × two-room / island / residual | **11** / 0 / 4 | 3 / 0 / 0 | **298** / 0 / 72 |
| dead × two-room / island / residual | 0 | 0 / 0 / 1 | 0 / 1 / 2 |

**Read:** the two-room bounce rests **between** its rooms (298 of 366 sample two-room plats; 11 of
12 in DOOM+DOOM2) — it is authored mid-travel, at neither floor. The island rests **at high** (132
of 141; every one of Final Doom's 13): a column or block that sinks and rises beside one room. The
residual class is the largest in the sample and it is mostly *at high* with many neighbors — 519 of
936 plats touch four or five sectors and 586 see three or more distinct neighbor floors — a plat set
into a stair or a multi-level room rather than between two flat floors. Travel is longer than a
DWUS lift's (median 144 against the lift measurement's 64–128) and four in five sample plats
travel 65–256 units.

## C. Perpetual start triggers

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| forms per plat W1 only / WR only / both | 0 / 35 / 0 | 16 / 4 / 0 | **674 / 262** / 0 |
| start lines per plat tag 1 / 2 / 3+ | 10 / 1 / 24 | 4 / 0 / 16 | 723 / 77 / 136 |
| (start line, plat) pairs | 84 | 53 | 1,480 |
| placement OnPlatFront / OnPlatBack / Adjacent / **Remote** | 1 / 1 / 10 / **72** | 0 / 8 / 1 / **44** | 30 / 27 / 300 / **1,123 (75.9 %)** |
| — of the W1 pairs | — | 0 / 6 / 0 / 40 | 0 / 14 / 190 / 801 |
| — of the WR pairs | 1 / 1 / 10 / 72 | 0 / 2 / 1 / 4 | 30 / 13 / 110 / 322 |
| activator Low / Level / Above / Plat | 23 / 41 / 28 / 1 | 39 / 14 / 0 / 8 | **922** / 270 / 300 / 39 |
| hops 0 / 1 / 2 / 3 / 4–5 / 6+ / unreachable | 1 / 11 / 20 / 4 / 32 / 16 / 0 | 8 / 1 / 39 / 0 / 3 / 2 / 0 | 39 / 313 / 156 / 180 / 380 / **366** / 46 |
| a start line borders the player-1 start's sector: pairs · plats | 0 · 0 | 0 · 0 | 73 · 59 (6.3 %) |
| **boardable at rest** | 19 (54.3 %) | 7 (35.0 %) | **306 (32.7 %)** |
| — at high / at low / between / dead | 2 of 7 / 5 of 13 / 12 of 15 / — | 1 of 14 / 2 of 2 / 3 of 3 / 1 of 1 | 81 of 402 / 102 of 161 / 120 of 370 / 3 of 3 |

**Read:** a perpetual plat is **remote-started**: three in four sample (line, plat) pairs sit in a
sector that is neither the plat nor a neighbor, a quarter are six or more rooms away, and no plat
anywhere is started from its own face by a W1 line on its front side. The one-shot 53 outnumbers the
repeatable 87 five to two in the sample and four to one in Final Doom; DOOM+DOOM2 uses only 87. No
plat mixes the two. The activator is below the plat in the sample (922 of 1,531 activator entries)
but level or above in DOOM+DOOM2 (69 of 93). A plat resting at high can be stepped onto from some
neighbor only one time in five (81 of 402): at rest it is a wall or a pillar, not a floor.
Forty-six sample pairs are fired from a sector with no two-sided path to the plat at all.

## D. Perpetual stop lines

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| (stop line, plat) pairs · W1 / WR | 133 · 0 / 133 | 0 | 265 · 32 / 233 |
| placement OnPlatFront / OnPlatBack / Adjacent / Remote | 0 / 0 / 3 / **130** | — | 1 / 2 / 17 / **245** |
| activator Low / Level / Above / Plat | 47 / 59 / 35 / 0 | — | 86 / 44 / **137** / 1 |
| hops 0 / 1 / 2 / 3 / 4–5 / 6+ / unreachable | 0 / 3 / 0 / 30 / 36 / 64 / 0 | — | 1 / 17 / 21 / 20 / 37 / **164** / 5 |
| plats with ≥ 1 stop line | 35 (100 %) | 0 | 147 (15.7 %) |
| — nearest stop: own boundary / a neighbor's far threshold / remote only | 0 / 1 / 34 | — | 3 / 13 / 131 |

**Read:** the stop line is a **remote off-switch**, not a station: 130 of 133 retail pairs and 245
of 265 sample pairs are Remote, and the nearest stop line is remote-only for 34 of 35 retail plats
and 131 of 147 sample plats. Nearly two thirds of sample stop pairs are six or more rooms from the
plat, and the sample fires them from *above* the plat more often than from below (137 against 86).
Only three sample plats carry a stop line on their own boundary. Both stop specials are walkovers
and the corpus uses the repeatable one seven times in eight.

## E. Perpetual rendering and cargo

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| faces (two-sided boundaries to a neighbor at another floor) | 112 | 117 | 3,972 |
| visible lower present | 111 (99.1 %) | 117 (100 %) | 3,955 (99.6 %) |
| **lower `ML_DONTPEGBOTTOM`** | **0 (0.0 %)** | **0 (0.0 %)** | **101 (2.5 %)** |
| level boundaries (equal floors, no lower drawn) | 1 | 6 | 411 |
| flat shared with lowest / highest / another neighbor / none | 25 / 0 / 0 / 10 | 3 / 0 / 0 / 17 | 645 / 151 / 6 / 134 |
| light == a lowest neighbor's / == a highest neighbor's | 26 (74.3 %) / 23 (65.7 %) | 17 (85.0 %) / 17 (85.0 %) | 802 (85.7 %) / 796 (85.0 %) |
| plats holding ≥ 1 thing | 22 (62.9 %) | 15 (75.0 %) | 314 (33.5 %) |
| things by class monster / pickup / prop / start / other | 13 / 12 / 0 / 0 / 10 | 25 / 3 / 0 / 1 / 0 | 288 / 231 / 127 / 15 / 29 |
| — `other` names | candle 9, berserk 1 | | candle 8, teleport_dest 4, type 87 4, soulsphere 3, berserk 2, invisibility 2, megasphere 2, radsuit 2, brain_stem 1, type 88 1 |

**Read:** the faces of a perpetual plat follow the shipped lift convention exactly — a lower is
present on the visible side 99–100 % of the time and it is **pegged** (flag clear) on every retail
face and 97.5 % of sample faces, against 3.7 / 3.8 / 4.4 % unpegged risers on DWUS lifts
([lift-shapes](lift-shapes-2026-08-29.md) §G). Where the plat shares a flat it is the lowest
neighbor's (645 of 802 sharing sample plats); Final Doom's pillars keep their own flat (17 of 20
share none). Light matches a neighbor's 74–86 % of the time. Cargo is common and mixed: a third of
sample plats and two thirds of retail plats hold something, and it is a monster, a pickup or a prop
in roughly equal measure in the sample (288 / 231 / 127) — DOOM+DOOM2's cargo includes nine
candles, Final Doom's is 25 monsters.

## F. Concurrency

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| perpetual plats per map 0 / 1 / 2 / 3–5 / 6–10 / 11+ | 63 / 2 / 0 / 0 / 1 / 2 | 59 / 3 / 0 / 1 / 0 / 1 | 1,192 / 26 / 12 / 21 / 15 / 16 |
| — max | 12 | 14 | **360** |
| moving DWUS/blaze plats per map 0 / 1 / 2 / 3–5 / 6–10 / 11+ · max | 16 / 7 / 12 / 15 / 13 / 5 · 21 | 6 / 6 / 8 / 20 / 15 / 9 · 57 | 503 / 183 / 149 / 228 / 147 / 72 · 50 |
| most sectors **one 53/87 tag** names, per map with such a tag: 0–15 / 16–30 / 31+ · max | 5 / 0 / 0 · 12 | 5 / 0 / 0 · 13 | 82 / 6 / **2** · 360 |
| most sectors **one DWUS/blaze tag** names, per map with such a tag: 0–15 / 16–30 / 31+ · max | 53 / 0 / 0 · 4 | 58 / 1 / 0 · 19 | 779 / 7 / **1** · 46 |
| maps with a single tag naming > 30 sectors: 53/87 · lift | 0 · 0 | 0 · 0 | **2 · 1** |
| maps with a perpetual plat where perpetual ∪ moving plats (an upper bound) > 15 / > 30 | 0 / 0 | 0 / 0 | 19 / 5 |
| — max combined among them | 14 | 15 | 360 |

**Read:** `MAXPLATS` is 30 and a perpetual plat holds a slot for the rest of the level. The
per-map union of perpetual and moving plats is an **upper bound** on concurrency, not a proof of
it: five sample maps exceed 30 that way and nineteen exceed 15, but several smaller banks need
never be active together. What `EV_DoPlat` allocates in one call is one `plat_t` per sector
matching **one** line's tag (`p_plats.c:164-181`), so the figure that can overflow the table at
once is the largest single tag — and that is rare: two sample maps have a 53/87 tag naming more
than 30 sectors (the largest 360) and one has a lift tag naming 46; no retail tag names more
than 19. Those three maps fail on the first press of that line in vanilla; the other 21 maps
above the bound depend only on timing. Half the maps with a perpetual plat have several (64 of 90
in the sample have two or more).

## G. One-shot lift breakouts

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| all-one-shot plats · mixed plats | 2 · 0 | 5 · 5 | **183 · 16** |
| all-one-shot forms S1 only / W1 only / both | 1 / 1 / 0 | 0 / 5 / 0 | **164** / 19 / 0 |
| mixed forms S1 only / W1 only / both | — | 0 / 5 / 0 | 6 / 10 / 0 |
| plats holding ≥ 1 thing · classes monster / pickup / start / other | 2 (100 %) · 0 / 1 / 0 / 1 | 8 (80.0 %) · 12 / 1 / 5 / 1 | 36 (18.1 %) · 38 / 30 / 6 / 26 |
| S1 lines (21/122) · with an `SW1*` front slot · `SW2*` | 1 · 1 · 0 | 0 | 149 · 32 (21.5 %) · 3 (2.0 %) |
| S1 front middle/lower names (case-folded), top | SW1STRTN 1 | — | FIRELAVA 8, F_050 6, DOORYEL2 5, ZIMMER5 5, BRICK6 4, BRICKBLK 4, CRATELIT 4, ROCKRED1 4, SW1COMP 4, SW1STON2 4 |

What-if shape × trigger form (the one-shot specials rewritten to their repeatable twins), as
`S1 · W1` counts for the all-one-shot plats / the mixed plats:

| what-if shape | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| Core | 0 / — | 0 · 0 / 0 · 3 | **4** · 0 / 1 · 3 |
| Pedestal | 0 / — | 0 · 5 / 0 · 1 | **58** · 2 / 0 · 6 |
| Barrier | 0 / — | 0 / 0 | **51** · 0 / 5 · 0 |
| Other | 1 · 1 / — | 0 / 0 · 1 | 51 · 17 / 0 · 1 |

Breakouts of the two shapes the design cares about:

| | Final Doom | idgames |
|---|---|---|
| what-if Core: rooms still connected without the plat / **stranded** (plat is the only route) | 0 / **3** | 6 / **2** |
| what-if Barrier: Low-firing neighbors 0 / 1 / 2+ | — | **43** / 7 / 6 |
| what-if Barrier: faces carrying an on-plat lift line 0 / 1 / 2+ | — | **45** / 8 / 3 |

**Read:** the one-shot lift is a **switch-once-from-elsewhere** construct. In the sample 164 of 183
all-one-shot plats are S1-only, and of the 56 that would be barriers if repeatable, 45 have no lift
line on any face of their own and 43 are fired from no neighbor at all — the switch is somewhere
else, and it lowers the block once. Only 32 of 149 S1 lines carry an `SW1*` texture in a front slot:
four in five are bare wall lines, so `P_ChangeSwitchTexture`'s permanent swap has nothing to swap
on them. The Core what-ifs are few (eight in the sample — four all-one-shot, four mixed — and three
in Final Doom) and two of the sample's eight — and all three of Final Doom's — have the plat as the
only route between the rooms it joins: ridden once, the far side is cut off. One-shot plats carry
cargo less often than perpetual ones (18 % against 34 %) but when they do it is monsters and
pickups; Final Doom's ten one-shot plats hold twelve monsters and five player starts between them.

## H. Yield ceilings

The probe reproduces the §H baselines (Method above), then re-judges every map under five line-axis
columns. **Line axis** = every out-of-set linedef special the map uses is in the column. **All axes
as today** = that, and the six axes as shipped (a one-shot plat is refused by `lift::plat`'s
`OneShot`; a perpetual plat is not judged at all, so the column overstates what a recognizer would
pass). **Provisional** = the plat recognizer re-run on the twin-rewritten map, and every perpetual
plat passing the loose provisional gate (Definitions). **The provisional gate is a ceiling, not the
design**: it takes any plat of any shape class — two-room, island or residual — with a clean tag and
a rest at low or at high (never *between* or dead), and asks nothing about where the start line is.

| column (sample, U = 1,282) | line axis | all axes as today | provisional |
|---|---:|---:|---:|
| today | 187 (14.6 %) | **119 (9.3 %)** | 119 (9.3 %) |
| +{53, 87} | 193 (15.1 %) | 122 (9.5 %) | **120 (9.4 %)** |
| +{53, 87, 54, 89} | 194 (15.1 %) | 122 (9.5 %) | 120 (9.4 %) |
| +{21, 10, 122, 121} | 188 (14.7 %) | 119 (9.3 %) | 119 (9.3 %) |
| +all eight | 195 (15.2 %) | 122 (9.5 %) | **120 (9.4 %)** |

Perpetual plats passing the provisional gate: **45 of 936 (4.8 %)** in the sample, 3 of 20 in Final
Doom, 0 of 35 in DOOM+DOOM2 (every retail plat has a stop line). Retail on all axes stays at 0 under
every column (DOOM+DOOM2's line axis is 1 map throughout, Final Doom's 0).

**Read:** the line axis moves by **+6** maps for the perpetual pair, +1 more for the stop pair, +1
for the one-shot four, +8 for all eight. On all axes the honest ceiling is **+1** map: of the three
maps that clear every other axis once 53/87 are admitted, one has every perpetual plat inside the
loose gate; the one map the one-shot four unlock on the line axis is refused elsewhere even with its
triggers read as repeatable. Perpetual plats are rarely gate-clean on their own terms — 4.8 % —
because 92 % of them share a tag with other sectors (only 79 of 936 are alone on theirs), 40 % rest
between, and 16 % carry a stop line; the maps that hold the 45 clean ones mostly fail on other axes
before the plats are asked. The largest number in this section
is the smallest: the two lift variants together lift the honest figure from 119 to 120.

## I. Lift tag groups (banks)

One line drives every sector carrying its tag (`p_plats.c:164-169`), each with its own thinker and
its own `low` (`:207-212`); the recognizer refuses every such member `SharedTag` today (923 in the
sample, the binding refusal of [lifts-2026-08-30](lifts-2026-08-30.md)). This section judges the
members alone and the groups as wholes.

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| groups · size 2 / 3 / 4 / 5+ | 29 · 24 / 4 / 1 / 0 | 47 · 28 / 8 / 9 / 2 | **328** · 199 / 58 / 24 / 47 |
| floor class split / one floor disconnected / several floors | 1 / 16 / 12 | 4 / 19 / 24 | 25 / 158 / 145 |
| members | 64 | 141 | 1,085 |
| member verdict, judged alone: Lift / Pedestal / Barrier | 25 / 8 / 2 | 65 / 10 / 5 | **310 / 134 / 39** |
| — refused: Dead / OneShot / MixedSpeed / UnsupportedRest / TopOnly / OneWayBarrier / ConflictingAction | 6 / 0 / 2 / 9 / 7 / 3 / 2 | 6 / 4 / 0 / 13 / 22 / 11 / 5 | 162 / 125 / 44 / 106 / 52 / 55 / 58 |
| groups with all / some / none of their members accepted | 12 / 7 / 10 | 18 / 16 / 13 | **113 / 59 / 156** |
| composition: all Lift / all Pedestal / all Barrier / mixed shapes / any refused | 9 / 2 / 0 / 1 / 17 | 11 / 1 / 1 / 5 / 29 | 63 / 29 / 3 / 18 / 215 |

The 1,085 sample members are the 923 `SharedTag` refusals plus the 162 members refused `Dead`
ahead of it — the reconciliation [lifts-2026-08-30](lifts-2026-08-30.md) predicted; the 310 Lift
and 134 Pedestal verdicts are exactly the shared-tag probe shapes it cites.

**Who calls the members** (per member, then per group):

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| members with a lift line on their own boundary | 40 (62.5 %) | 60 (42.6 %) | 387 (35.7 %) |
| — callable from Low by any line naming the tag | 52 (81.2 %) | 114 (80.9 %) | 853 (78.6 %) |
| — with a Low-activator line on their **own face** | 33 (51.6 %) | 50 (35.5 %) | **275 (25.3 %)** |
| — called from a two-sided neighbor standing at their own `low` (**A′**) | 42 (65.6 %) | 84 (59.6 %) | **465 (42.9 %)** |
| — with a lift line within one hop | 47 (73.4 %) | 100 (70.9 %) | 557 (51.3 %) |
| groups: all / some / none of the members callable from Low | 21 / 6 / 2 | 32 / 11 / 4 | 239 / 47 / 42 |
| groups: all / some / none with a Low line on their own face | 13 / 5 / 11 | 11 / 8 / 28 | **67 / 82 / 179** |
| groups: all / some / none neighbor-called (A′) | 16 / 7 / 6 | 16 / 21 / 10 | **121 / 100 / 107** |
| lift lines by reach: on a member face / adjacent / remote from every member | 95 / 17 / 19 | 86 / 59 / 14 | 948 / 125 / 244 |
| groups by line reach: all on member faces / mixed / all remote | 13 / 13 / 3 | 13 / 27 / 7 | 142 / 102 / 84 |
| distinct lift lines per group 1 / 2 / 3+ | 5 / 5 / 19 | 14 / 16 / 17 | 98 / 85 / 145 |
| all lines one form (use or walkover) · one speed | 21 (72.4 %) · 28 (96.6 %) | 33 (70.2 %) · 47 (100 %) | 280 (85.4 %) · 315 (96.0 %) |

**Uniformity** (per group):

| | DOOM+DOOM2 | Final Doom | idgames |
|---|---|---|---|
| members all share one rest class | 19 (65.5 %) | 24 (51.1 %) | 216 (65.9 %) |
| — one travel | 15 (51.7 %) | 23 (48.9 %) | 191 (58.2 %) |
| — one low floor | 22 (75.9 %) | 32 (68.1 %) | 217 (66.2 %) |
| — one outside-neighbor count | 20 (69.0 %) | 20 (42.6 %) | 169 (51.5 %) |
| common outside neighbor: all share one / some share / none in common | 17 / 1 / 11 | 25 / 1 / 21 | **140 / 47 / 141** |
| — of the "all share one" groups, every member neighbor-called (A′) | 12 of 17 | 13 of 25 | **84 of 140** |

**Yield** (line axis unchanged: 1 / 0 / 187):

| column | DOOM+DOOM2 all axes · recovered of 58 · groups of 29 | Final Doom all axes · recovered of 135 · groups of 47 | idgames all axes · recovered of 923 · groups of 328 |
|---|---|---|---|
| today | 0 · 0 · 0 | 0 · 0 · 0 | **119 (9.3 %)** · 0 · 0 |
| +A (bank-A gate) | 0 · 20 · 9 | 0 · 35 · 8 | **119 (9.3 %)** · 121 · 43 |
| +A′ (neighbor-called) | 0 · 23 · 10 | 0 · 44 · 12 | **119 (9.3 %)** · 204 · 69 |
| +B (bank-B ceiling) | 0 · 28 · 12 | 0 · 62 · 18 | **121 (9.4 %)** · **351** · **113** |
| +split as one lift | 0 · 2 · 1 | 0 · 9 · 4 | **120 (9.4 %)** · 28 · 11 |

**Read:** judged alone, **45 % of sample bank members are a shape the IR already states** (483 of
1,085: 310 lifts, 134 pedestals, 39 barriers) and 113 of 328 groups (34 %) are clean throughout —
63 banks of lifts, 29 rows of pedestals, 3 of barriers, 18 mixing shapes. The members are usually
*not* called from their own faces: only a quarter have a Low-activator line on their own boundary,
and in 179 of 328 groups no member does — the bank's switch is on one member, or beside the bank,
or elsewhere, while every member is still callable from Low by *some* line in 239 groups. That
gap is the whole difference between the gates: gate A (own face per member) accepts 43 sample
groups and recovers 121 of the 923 refusals for **+0 maps** on all axes; gate A′ (a caller that
touches the member at its low, wherever the line is) accepts 69 groups and recovers 204 — 43 % of
members are neighbor-called, and 84 of the 140 one-room rows are neighbor-called throughout — still
for **+0 maps**; gate B (callers ignored) accepts 113 groups, recovers 351 refusals and lifts the
honest figure **119 → 121**. The split
reading recovers 28 platforms in 11 of the 25 split groups for +1 map. Banks are moderately
uniform — two thirds share one rest class and one low floor, half share travel — and they divide
evenly between a row inside one host room (140 groups whose members all touch one common sector)
and members with no neighbor in common (141). Lines are one form in 85 % of sample groups and one
speed in 96 %.

## Engine facts (fetched, pinned)

From `linuxdoom-1.10` at `a77dfb96cb91780ca334d0d4cfd86957558007e0`, read for this probe.

- **`p_spec.h:304-306`**

  ```c
  #define PLATWAIT		3
  #define PLATSPEED		FRACUNIT
  #define MAXPLATS		30
  ```

  `perpetualRaise` runs at `PLATSPEED` — one unit per tic — where `downWaitUpStay` is
  `PLATSPEED * 4` (`p_plats.c:208`) and `blazeDWUS` `PLATSPEED * 8` (`:221`); the wait at each end
  is `35*PLATWAIT` tics (`:245`).

- **`p_plats.c:233-249`, `EV_DoPlat`, `case perpetualRaise:`**

  ```c
	  case perpetualRaise:
	    plat->speed = PLATSPEED;
	    plat->low = P_FindLowestFloorSurrounding(sec);

	    if (plat->low > sec->floorheight)
		plat->low = sec->floorheight;

	    plat->high = P_FindHighestFloorSurrounding(sec);

	    if (plat->high < sec->floorheight)
		plat->high = sec->floorheight;

	    plat->wait = 35*PLATWAIT;
	    plat->status = P_Random()&1;

	    S_StartSound((mobj_t *)&sec->soundorg,sfx_pstart);
	    break;
  ```

  Both bounds are clamped to the sector's own floor; the first direction is random.

- **`p_plats.c:154-169`, `EV_DoPlat`** — `case perpetualRaise: P_ActivateInStasis(line->tag);`
  runs first (`:156-158`), then `while ((secnum = P_FindSectorFromLineTag(line,secnum)) >= 0)` with
  `if (sec->specialdata) continue;` (`:164-169`): a sector already carrying a thinker ignores a
  re-trigger, so a running perpetual plat cannot be started twice, but a stopped one is woken by
  the same line. The same loop is why **one line drives every sector with its tag** (§I): each
  iteration allocates its own `plat_t` (`:173-181`), and for `downWaitUpStay` each gets
  `plat->low = P_FindLowestFloorSurrounding(sec)` from **its own** neighbors, clamped to its own
  floor (`:207-212`) — a bank is several thinkers on one tag, not one platform.

- **`p_plats.c:52-131`, `T_PlatRaise`** — at `pastdest` going up, only `blazeDWUS`,
  `downWaitUpStay`, `raiseAndChange` and `raiseToNearestAndChange` are removed
  (`P_RemoveActivePlat`, `:89-103`); `default: break;` keeps a `perpetualRaise` thinker forever.
  The `waiting` arm (`:119-127`):

  ```c
	    if (plat->sector->floorheight == plat->low)
		plat->status = up;
	    else
		plat->status = down;
  ```

- **`p_plats.c:258-271`, `P_ActivateInStasis`** — every `activeplats[i]` with the line's tag and
  `status == in_stasis` gets `status = oldstatus` and its thinker function back.
  **`p_plats.c:273-286`, `EV_StopPlat`**:

  ```c
	if (activeplats[j]
	    && ((activeplats[j])->status != in_stasis)
	    && ((activeplats[j])->tag == line->tag))
	{
	    (activeplats[j])->oldstatus = (activeplats[j])->status;
	    (activeplats[j])->status = in_stasis;
	    (activeplats[j])->thinker.function.acv = (actionf_v)NULL;
	}
  ```

  Every active plat with the tag, **of any type**, is frozen.

- **`p_plats.c:288-299`, `P_AddActivePlat`** — the first free slot of `activeplats[MAXPLATS]`, else
  `I_Error ("P_AddActivePlat: no more plats!");`. A perpetual plat holds its slot forever, in stasis
  or not.

- **`p_spec.c`, `P_CrossSpecialLine`** — `case 53:` `EV_DoPlat(line,perpetualRaise,0); line->special
  = 0;` (`:682-686`); `case 54:` `EV_StopPlat(line); line->special = 0;` (`:688-692`); `case 87:`
  `EV_DoPlat(line,perpetualRaise,0);` (`:852-855`); `case 89:` `EV_StopPlat(line);` (`:862-865`).
  All four are walkovers; there is no switch or gun form.

- **The one-shot lifts** — `p_switch.c:389-393` `case 21: if (EV_DoPlat(line,downWaitUpStay,0))
  P_ChangeSwitchTexture(line,0);` and `:479-483` `case 122:` likewise with `blazeDWUS`;
  `p_spec.c:579-583` `case 10: EV_DoPlat(line,downWaitUpStay,0); line->special = 0;` and
  `:754-758` `case 121:` likewise with `blazeDWUS`. `P_ChangeSwitchTexture` (`p_switch.c:201-262`)
  with `useAgain == 0` sets `line->special = 0` (`:211-212`) and swaps whichever front slot matches
  `switchlist[]` permanently (`:229`, `:241`, `:253` — `P_StartButton` runs only `if (useAgain)`).

## Not measured (carry forward)

- Fire-time heights: a perpetual plat whose neighbor has moved before its start line fires (§B
  bounds are load-time).
- Which way a plat goes first (`P_Random()&1`), and therefore where a rider finds it.
- Stasis interactions: a stop line firing before its start, a stop freezing a DWUS lift on a shared
  tag, a start line waking a plat another line stopped.
- `ML_BLOCKING`, monsters, the player's radius and use-reach in the descriptive rows — the probe's
  hops, activators and strand test are pure adjacency and height tests (the §H arbiter and the §I
  gates use the verifier's resolver instead; see Method).
- Which sector a rider *boards* a two-room bounce from, and whether a start line's activator sector
  is on the rider's route (only distance and player-start adjacency are counted).
- Sound: a perpetual plat plays `sfx_pstart`/`sfx_pstop` at every reversal (`T_PlatRaise`).
- Texture *names* on perpetual faces (only presence and pegging are counted).
- Whole-map reachability; the strand test is a local hop-graph fact.
- Bank timing: whether a bank's members, each with its own `low`, arrive and return together
  (§I judges each member's geometry at load, not the thinkers' phase).
- The merged reading of a split group is an approximation (union of outside neighbors, least
  `low`); it does not rebuild the sector.

## Method — the exact commands

From the crustygen checkout on branch `feature/50-lift-banks`, with the sample already fetched in
the crustywad checkout (`just harvest-sample 20260828 400`). Below, `$SAMPLE` is that checkout's
`xtask/data/samples/20260828-400`, `$RETAIL` its `RETAIL/`, and `$SCRATCH` a throwaway directory
outside both repositories:

```console
$ cargo build --release --example liftprobe
```

The retail rounds, over scratch directories holding copies of the IWADs — outside both repositories,
and `RETAIL/` itself is only ever read:

```console
$ mkdir -p "$SCRATCH/retail" "$SCRATCH/final"
$ cp "$RETAIL/DOOM.WAD" "$RETAIL/DOOM2.WAD" "$SCRATCH/retail/"
$ cp "$RETAIL/TNT.WAD" "$RETAIL/PLUTONIA.WAD" "$SCRATCH/final/"
$ cargo run --release --example liftprobe -- variants retail  "$SCRATCH/retail" > "$SCRATCH/variants-retail.md"
$ cargo run --release --example liftprobe -- variants final   "$SCRATCH/final"  > "$SCRATCH/variants-final.md"
```

The sample:

```console
$ cargo run --release --example liftprobe -- variants idgames "$SAMPLE" > "$SCRATCH/variants-idgames.md"
```

Markdown to stdout, load failures to stderr. A path may name a directory (swept non-recursively for
`.zip` and `.wad`) or a single archive or WAD. The three population columns above are the three
outputs merged by hand; every number is traceable to a printed line. The refactor that let this
pass share `floors.rs`'s §I helpers was checked by re-running `census`, `shapes` and `floors` over
all three populations before and after it: all nine outputs are byte-identical.
