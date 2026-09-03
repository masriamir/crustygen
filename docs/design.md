# crustygen — map-spec template and generation pipeline

**Date:** 2026-08-09
**Status:** Design approved, pending implementation plan
**Home:** throwaway worktree `feature/crustygen-map-spec`; promotable to a standalone `crustygen` repo

## 1. Context

crustywad can read, assemble, convert, and write Doom maps, and can build engine-playable
node lumps (`nodebuild`). It cannot *construct* a map from nothing: `Map` exposes accessors
only, and `UdmfMap` is `#[non_exhaustive]`. Maps enter the library by parsing and leave by
writing; there is no public in-memory construction API.

That constraint sets the shape of this project. The generation path is **text first**: produce
UDMF `TEXTMAP` source, pack it with `cwad build --nodes`, and let crustywad do the parsing,
assembly, and node building it already does well.

The experiment: given a Markdown template whose blanks a Doom-literate author fills in, produce
a playable, coherent Doom map.

## 2. Success criteria

A run succeeds when the generated map:

- loads in the target engine without errors,
- has no leaks, HOMs, or unclosed sectors,
- has every area reachable, with the key/door/exit sequence functioning in the specified order,
- honors the monster, weapon, ammo, powerup, and secret counts in the filled spec,
- and passes an automated conformance report against that spec.

Architecture may be simple — rectilinear rooms and corridors — but must be deliberate. Curved
geometry, texture-alignment polish, and hand-made-quality detailing are explicitly not the bar.

## 3. Decisions

| Decision | Choice | Rationale |
|---|---|---|
| Success bar | Playable and coherent | Honest first target; measurable |
| Authoring format | UDMF `TEXTMAP` | The only format constructible as text without a library API |
| Second output | Doom binary via `cwad convert --to doom --nodes` | Free structural oracle; exercises the convert path |
| UDMF namespace | `doom` | Classic Doom line types and no args, so UDMF→Doom is exact. `zdoom` is a later option that trades downconvert fidelity for 5-arg specials |
| Geometry production | Layout IR + deterministic compiler | Correctness becomes structural rather than attentional; the compiler is the promotable core |
| Compiler language | Rust | Repo language; lifts into a `crustygen` repo and a WASM front end without a rewrite (crustyview precedent) |
| Template vocabulary | High-level, with raw escape hatches | Suits a future web form with an advanced mode; still fully specifiable |
| Repository | Throwaway worktree | Promote to a standalone `crustygen` repo if the experiment lands |
| Playability rules | Always hard errors (except P10) | A door the player cannot fit through is a broken map, not a missed target; `enforcement` does not soften it |
| Tags | Central allocator; every action sector uniquely tagged, no action line at tag 0 | Tag 0 is the tag every untagged sector already has, so one stray zero fires against the whole map; uniform tagging leaves no per-special exception to audit |
| Engine thresholds | Sourced `engine.toml`, no hardcoded numbers | A wrong constant yields a map that loads and traps the player, and shared-assumption tests cannot catch it |

## 4. Architecture

```
map-spec.md          filled template: YAML frontmatter (machine-checkable) + Markdown body (design intent)
    │                    design decisions — room graph, progression, encounters (authored per map)
    ▼
layout.json          IR: rooms as footprints on a 64-unit grid, portals, per-room content
    │                    crustygen-compile: pure geometry, no creative choices
    ▼
TEXTMAP              valid UDMF text, namespace "doom"
    │                    crustygen-pack → cwad build --nodes
    ▼
MAP01.wad (UDMF + ZNODES) ──cwad convert --to doom --nodes──▶ MAP01-doom.wad
    │
    ▼
crustygen-check      reparses through crustywad, asserts against the frontmatter
    │
    ▼
report.md            conformance table: parameter, target, actual, verdict
```

### 4.1 Units

**The template** (`map-spec.template.md`) is the input contract. YAML frontmatter holds
everything a machine can check — counts, ranges, enums, key lists. The Markdown body holds what
only prose can carry: the ordered sequence of events, what each secret is and how it is hinted,
and the mood. A web form later renders the frontmatter as fields and the body as textareas.

**The IR** (`layout.json`, validated against `layout.schema.json`) is the only artifact authored
per map. Its vocabulary is deliberately small: rooms with rectilinear footprints, portals between
rooms, content attached to rooms.

**The compiler** (`crustygen-compile`) turns IR into `TEXTMAP`. It owns geometry integrity and
nothing else, which is what makes it unit-testable and portable.

**The packager** (`crustygen-pack`) produces **two** WADs from the one `TEXTMAP`:

- the UDMF artifact, built with node building on, yielding `MAP01` / `TEXTMAP` / `ZNODES` /
  `ENDMAP`;
- a plain, un-noded build used solely as the source for the Doom downconvert.

The second build is not redundant. `cwad convert --to doom` refuses in strict mode when the
source map carries a `ZNODES` lump, since that lump cannot cross into a Doom map group —
converting the noded artifact would require `--lenient` and a dropped-lump warning. Building an
un-noded twin keeps both paths strict. Verified against `cwad` 0.4.7: the un-noded build converts
cleanly to all eleven Doom lumps and passes `cwad validate`.

**The verifier** (`crustygen-check`) reparses the built WAD through crustywad and emits the
conformance report.

### 4.2 Vocabulary mapping

High-level names in the template (`shotgun_guy`, `blue_card`, `tech_base`) resolve to concrete
thing IDs, texture names, line specials, and sector types through a `vocabulary.toml` table.

**Every value in that table must carry a citation, never be written from recall** — a `source`
recording a primary reference (the UDMF specification, the ZDoom wiki, the id-Software DOOM
sources, or a measured IWAD-corpus reading, among others), a `derivation` for a computed value, or
`curated` for a judgment call that has no primary source (which must **not** claim one). The table
is data, not code, precisely so it can be checked against sources independently of the compiler.

## 5. The template

Filled example; every field carries its allowed values as a comment.

````markdown
---
spec_version: 1

# Identity and target
identity:
  slot: MAP01
  title: "Refinery Overrun"
  author: "Amir Masri"
  iwad: doom2                # doom2 | freedoom2
  outputs: [udmf, doom]      # udmf is authored; doom is produced by cwad convert
  seed: 20260809             # reproducibility; same seed + same IR = byte-identical TEXTMAP
  grid: 64                   # map units; all coordinates snap to this

# Players and starts
players:
  start_facing: east         # north | south | east | west, or degrees
  coop_starts: 4             # 0 disables coop; 4 is the conventional set
  dm_starts: 0               # 0 disables deathmatch
  coop_only_items: false     # place extra pickups flagged multiplayer-only

# Scale budget
scale:
  size: { width: 4096, height: 4096 }     # bounding box, map units
  rooms: { min: 8, max: 14 }
  sectors: { min: 40, max: 120 }
  linedefs: { min: 200, max: 600 }
  play_time_minutes: { min: 6, max: 10 }
  vertical_range: { min: 0, max: 256 }    # allowed floor heights; the map's span is max - min

# Progression
progression:
  shape: hub_and_spoke       # linear | hub_and_spoke | branching | gauntlet
  keys: [blue_card, red_skull]
  locked_doors: 2
  backtracking: light        # none | light | heavy
  exit:
    kind: normal             # normal | secret | both
    trigger: switch          # switch | teleport | walkover
  lifts:
    count: { min: 0, max: 2 }
    trigger: both_ends       # walkover | switch | both_ends
    max_travel: 256          # largest floor delta a lift may span
  teleports:
    count: { min: 0, max: 2 }
  doors:
    speed: normal            # normal | fast
    default_behavior: repeatable   # repeatable | one_shot | stays_open
    lock_types: [blue_card, red_skull]   # must be a subset of progression.keys (P24)
  switches:
    count: { min: 2, max: 6 }
    remote_allowed: true     # a switch may act on a distant sector
  walkover_triggers:
    count: { min: 1, max: 4 }

# Architecture
architecture:
  room_shapes: [rectangular, l_shaped, octagonal]  # rectangular | l_shaped | t_shaped | octagonal | irregular
  symmetry: organic          # organic | axial | radial | mixed
  openness: mixed            # tight | mixed | open
  corridor_ratio: 0.3        # fraction of floor area that is transit rather than space
  verticality: moderate      # flat | moderate | strong
  inter_area_windows: true   # sightlines between areas that are not directly connected
  overlooks: { min: 1, max: 3 }   # elevated vantage points over another area
  landmarks: 1               # visually distinct anchors aiding navigation

# Combat
combat:
  encounter_style: ambush    # incidental | ambush | arena | corridor
  hitscanner_ratio: 0.35     # fraction of total monster count that is hitscan
  max_simultaneous: 12       # pressure ceiling: most monsters active at once
  monster_closets: 3
  boss: none                 # none | cyberdemon | mastermind | <species>
  ambush:
    deaf_ratio: 0.4          # fraction of monsters flagged deaf: wake on sight, not on sound
    teleport_ambushes: { min: 1, max: 3 }
  sound:
    propagation: contained   # open | contained | sealed
    block_sound_at: [key_doors, arena_entrances]   # where sound-blocking lines go
  block_monster_lines: true  # keep monsters in their region without a wall
  monsters:
    - { species: zombieman,   min: 10, max: 18 }
    - { species: shotgun_guy, min: 8,  max: 14 }
    - { species: imp,         min: 12, max: 20 }
    - { species: pinky,       min: 4,  max: 8 }
    - { species: cacodemon,   min: 0,  max: 3 }
    - { species: hell_knight, min: 1,  max: 2 }

# Weapons and ammo
arsenal:
  pistol_start: required_viable        # required_viable | not_required
  weapons:
    - { name: shotgun,         placement: early }        # early | mid | late | secret_only
    - { name: chaingun,        placement: mid }
    - { name: super_shotgun,   placement: mid }
    - { name: rocket_launcher, placement: secret_only }
  ammo:
    budget: balanced         # tight | balanced | generous
    ratio: 1.25              # placed ammo damage / total baseline monster HP; overrides budget
    distribution: even       # front_loaded | even | back_loaded
    pickups: auto            # auto (derived from ratio) | explicit counts per pickup type
    backpack: { count: 1, placement: mid }

# Health, armor, powerups
sustain:
  health_budget: balanced    # tight | balanced | generous; explicit counts below override it
  health:
    stimpack: 6
    medikit: 4
    health_bonus: 20         # the +1 bonuses; they matter for a tight-budget map
  armor:
    green: 2
    blue: 0
    armor_bonus: 15
  powerups:                  # count 0 means deliberately absent
    - { name: berserk,         count: 1, placement: secret_only }
    - { name: soulsphere,      count: 1, placement: late }
    - { name: megasphere,      count: 0, placement: none }
    - { name: radsuit,         count: 1, placement: mid }
    - { name: invulnerability, count: 0, placement: none }
    - { name: invisibility,    count: 0, placement: none }
    - { name: light_amp,       count: 0, placement: none }
    - { name: computer_map,    count: 1, placement: secret_only }

# Secrets
secrets:
  count: 3                   # per-secret detail lives in the prose body

# Difficulty
difficulty:
  skills_supported: true     # emit real easy/medium/hard thing flags
  baseline: uv               # the skill the counts above describe
  curve: gentle              # gentle | steep | late_spike
  scaling: { easy: 0.55, medium: 0.75, hard: 1.0 }

# Aesthetics
aesthetics:
  theme: tech_base           # tech_base | hell | gothic | city | cave | marble | wood
  texture_set: auto          # auto (theme-derived) | explicit list of texture names
  detail_level: 3            # 1..5
  lighting:
    style: contrasty         # flat | contrasty | pools_of_dark
    base: 160                # default sector light where nothing else applies
    min: 96                  # floor and ceiling for every emitted light level (P19)
    max: 208
    contrast_step: 32        # the delta that counts as a deliberate light change (P21)
    corridor_delta: -16      # corridor light relative to the rooms it joins
    outdoor: 192             # light level for sky-ceilinged sectors
    effects:
      allowed: [blink, flicker, glow, strobe_slow]  # subset; [] for none
      density: sparse        # none | sparse | medium | dense
      forbid_in: [combat_arenas, secret_rewards]    # no strobing mid-fight
    per_room_overrides: true # rooms may set their own level and effect in the IR
  sky: auto
  music: auto
  texture_scaling: forbidden # forbidden | allowed; v1 never emits scalex/scaley (see P9)

# Flats and liquids
flats:
  floor: auto                # auto (theme-derived) | explicit list of flat names
  ceiling: auto
  outdoor_proportion: 0.15   # fraction of floor area with a sky ceiling
  light_flats: true          # bright ceiling flats beneath light sources
  liquid:
    kind: nukage             # none | nukage | blood | lava | slime | water
    damaging: true           # pair a damaging sector special with the liquid flat (see P16)
    damage_tier: light       # light | medium | heavy; resolved to sector specials via engine.toml
    coverage: 0.08           # fraction of floor area
    crossing_required: true  # must the player enter it to progress?
    radsuit_provided: true   # if crossing_required, radsuit or health budget must cover it (P17)

# Vertical form
vertical:
  stairs:
    flights: { min: 1, max: 3 }
    rise_per_step: 16        # uniform within a flight; must not exceed engine max step height (P1)
    tread_depth: 32          # must be at least the player's diameter (P1)
  standard_ceiling: 128      # default room height where the spec says nothing
  door_opening: 128          # nominal door height; effective opening derived per P4

# Scenery: decoration, light sources, hazards
scenery:
  light_sources:
    density: medium          # none | sparse | medium | dense
    kinds: auto              # auto (theme-derived) | explicit list
    match_lighting: true     # every bright pool gets a visible source (P21)
  decorations:
    density: medium          # none | sparse | medium | dense
    kinds: auto
    blocking_allowed: true   # movement-blocking props, still subject to P3
    hanging_allowed: true    # ceiling-mounted props, subject to headroom (P22)
  gore: light                # none | light | heavy: corpses, blood, impaled bodies
  barrels:
    count: { min: 4, max: 10 }
    placement: near_encounters     # near_encounters | scattered | none
    chain_reactions: allowed       # allowed | avoided
    keep_clear_of: [player_start, key_pickup, secret_reward]   # P23

# Pacing
pacing:
  encounter_beats: { min: 5, max: 8 }
  rest_areas: { min: 2, max: 4 }
  peak_position: 0.8         # where the hardest fight sits, as a fraction of progression
  opening_intensity: low     # low | medium | high

# Compatibility and metadata
compat:
  port: limit_removing       # vanilla_limits | limit_removing | boom | zdoom
  emit_mapinfo: false        # v1 emits no extra lumps; par_time needs this
  par_time_seconds: 300      # ignored unless emit_mapinfo is true
  automap:
    hide_secret_lines: true  # a secret door does not read as a door on the automap
    show_map_lines: auto

# Constraints and priorities
constraints:
  enforcement: target        # strict (ranges are hard limits) | target (ranges are goals)
  forbid: [archvile, crusher, dark_maze, insta_death_pit]
  inspirations:
    - "pacing like Doom II MAP07"
    - "texture discipline like Plutonia"
  must_include:
    - "a window overlooking the final arena, visible from the start"
  priority:                  # highest first; resolves conflicts between everything above
    - progression_correctness
    - playable_balance
    - sector_budget
    - monster_counts
    - detail_level
    - play_time
---

## Overview

One or two paragraphs on what the map is and how it should feel to play.

## Sequence of events

1. Player starts in ...
2. ...

## Secrets

### Secret 1 — <name>
- Trigger: misaligned texture   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->
- Reward: ...
- Hint: ...

## Notes

Anything else.
````

### 5.1 Conflicting parameters are expected

With this many knobs, conflicts are guaranteed — `hitscanner_ratio: 0.35` and the explicit
per-species min/max cannot both bind exactly; `sectors.max: 120` and `detail_level: 5` are
mutually exclusive on a large map. That is what `constraints.priority` is for. Under
`enforcement: target`, the resolver honors the higher-priority parameter and records the
sacrifice in the conformance report. Under `enforcement: strict`, an unsatisfiable set is an
error and nothing is produced.

## 6. The IR

```json
{
  "seed": 1234,
  "grid": 64,
  "rooms": [
    {
      "id": "entry",
      "footprint": [[0, 0], [512, 0], [512, 384], [0, 384]],
      "floor": 0,
      "ceiling": 128,
      "floor_tex": "FLOOR4_8",
      "ceil_tex": "CEIL3_5",
      "wall_tex": "STARTAN3",
      "light": 160,
      "light_effect": null,
      "special": null,
      "tag": 12,
      "content": { "player_start": true, "things": [] }
    }
  ],
  "portals": [
    { "a": "entry", "b": "hall", "kind": "door", "lock": null, "width": 128, "at": [512, 128] },
    { "a": "hall", "b": "vault", "kind": "lift", "width": 128, "at": [768, 384],
      "trigger": "switch", "speed": "normal" },
    { "a": "entry", "b": "pen", "kind": "drop_wall", "width": 64, "at": [512, 96],
      "thickness": 16, "fires_on": "t_wall" }
  ],
  "triggers": [
    { "id": "t_wall", "kind": "switch", "room": "entry", "at": [0, 256] },
    { "id": "t_prize", "kind": "walkover", "portal": ["entry", "hall"] }
  ],
  "reveals": [
    { "id": "prize", "room": "hall", "at": [768, 512], "kind": "pedestal", "rise": 64,
      "things": [{ "kind": "red_card", "at": [800, 544], "angle": 0 }],
      "trigger": "t_prize" }
  ],
  "teleports": [
    { "id": "entry_to_vault",
      "room": "entry",
      "pad": { "island": [320, 192] },
      "to": { "room": "vault", "at": [96, 96], "angle": 90 },
      "monsters_only": false,
      "repeatable": true }
  ],
  "pedestals": [
    { "id": "prize",
      "room": "vault",
      "at": [320, 320],
      "size": [64, 64],
      "rise": 128,
      "speed": "normal",
      "things": [{ "kind": "soulsphere", "at": [352, 352], "angle": 0 }] }
  ],
  "exits": [
    { "room": "vault", "trigger": "teleport", "secret": false, "width": 64, "at": [0, 64] }
  ]
}
```

Footprints are **clockwise**, grid-snapped, and restricted to axis-aligned and 45-degree edges
in v1.

Clockwise is not a stylistic choice: a linedef's front (right) sidedef must face the sector
interior, and the right-hand side of a directed edge only faces inward when the boundary winds
clockwise. Verified empirically rather than assumed — measuring the signed area of every sector
boundary in nine Freedoom maps across both IWADs, oriented so the sector sits on the front side,
gives 2611 clockwise and 0 counter-clockwise. Portal `kind` is one of `plain`, `door`, `locked`,
`lift`, `drop_wall`, `bridge`; `lock` names a key when `kind` is `locked`. Texture names in the IR
are concrete, having already been resolved from the template's high-level vocabulary.

**Teleports are not portals.** A portal joins two rooms through their shared wall; a teleport
relocates whatever crosses it, and the two rooms need not touch. They live in their own
`teleports` list: `{ id, room, pad: { island | wall }, to: { room, at, angle }, monsters_only,
repeatable }`. `pad` places a `PAD_SIZE` square — free-standing inside `room` (`island`) or
recessed out of one of its walls (`wall`) — whose every edge carries the teleport special, and
which is always the trigger line's *back* sector, because `EV_Teleport` refuses a back-side
crossing. **A pad is addressed by its low corner**, never its center: an `island` point is the
square's minimum-x/minimum-y corner (the square is `[x, x+64] x [y, y+64]`), and a `wall` point is
where the pad's 64-unit span *starts* along that wall. `to` is a point with a facing, not a pad:
the compiler synthesizes a destination marker there and tags the sector that holds it — a two-way
pair therefore names the other pad's center itself, since nothing centers an arrival for you.
`monsters_only` selects the 126/125 pair over 97/39, and `repeatable` (default true) the
retriggerable form over the one-shot one. A two-way pair is two independent one-way teleports
whose destinations land on each other's pads. `exits[].trigger` gains `teleport` alongside
`switch` and `walkover`: an exit the player can only arrive at by teleport.

The address is the corner rather than the center because the grid a flat wraps on is the
*renderer's*, anchored to the world origin and not to the sector. `R_MapPlane`
(`linuxdoom-1.10/r_plane.c`, pinned `a77dfb96`) gives a flat span the world coordinates
themselves — `ds_xfrac = viewx + FixedMul(finecosine[angle], length);` and
`ds_yfrac = -viewy - FixedMul(finesine[angle], length);` — and `R_DrawSpan` (`r_draw.c`)
indexes the 64x64 flat with the low six bits of each:
`spot = ((yfrac>>(16-6))&(63*64)) + ((xfrac>>16)&63);`. A flat therefore wraps every 64 units of
world space, and a 64x64 `GATE` pad reads as exactly one tile only when its corners are multiples
of 64. Both the island corner and the wall pad's span start — and the wall's own fixed coordinate,
which is the recess's near edge — must be multiples of `Ir::FLAT_TILE` (64), or `Ir::from_json`
rejects the pad with `TeleportPadOffFlatGrid`. `ir.grid` plays no part: 64 subsumes every grid that
divides it, and a grid that does not divide 64 cannot excuse a pad off the flat grid. The corpus
agrees with the renderer rather than merely permitting it: 321 of 321 `GATE*`-flatted 64x64 pads
across DOOM, DOOM2, TNT and PLUTONIA have their bounding-box minimum congruent to (0,0) mod 64,
against an 80.1 % baseline for DOOM+DOOM2 64x64 sectors at large and 97.9 % for `GATE*` pads in the
idgames sample (`docs/measurements/teleports-2026-08-28.md`).

A `wall` pad's recess is real geometry carved into the void rooms are authored apart across, so
the IR holds it to the same neighbor rules a portal gap obeys: the recess must clear every other
room by at least `MIN_PORTAL_GAP` (`TeleportPadRecessTooClose`), and its span on its host wall
must not overlap or even touch another opening cut into that same wall — a portal's opening or
the level exit's segment (`TeleportPadBesideOpening`). A walkover exit's own alcove is not yet
held to either rule; see `KNOWN-GAPS.md`.

**Lifts and pedestals are platforms.** A `downWaitUpStay` sector (`p_plats.c`, `EV_DoPlat`) rests
at its own floor and travels to the lowest floor among its two-sided neighbors
(`P_FindLowestFloorSurrounding`) and back. The fourth portal `kind`, `lift`, fills the portal gap
with one: between rooms at different floors it is a **lift**, resting level with the higher room
and dropping to the lower room's floor; between rooms at one floor it is a **barrier**, resting
`rise` above the shared floor — required there, rejected when the floors differ, and rejected
outright on any other kind. `speed` picks `normal` (62/88) or `fast` (123/120, `blazeDWUS`), and
`trigger` places the lines: `switch` puts a use special on the platform's low face, so the riser
itself is the switch; `walkover` puts a walkover special on the outer threshold of the low room's
alcove, which that room must therefore declare; `both_ends` is the switch plus a walkover on the
platform's top face. A barrier has no low room and so offers only `switch`. A lift names no
`door_thickness` — its own platform sector is what fills the gap.

**A walkover lift's alcove must be deeper than the player's radius.** `P_TryMove` fires a
walkover from its `spechit` walk, which asks whether `P_PointOnLineSide (thing->x, thing->y, ld)`
changed — the thing's *center* crosses, not its box — but it refuses the move first, at
`tmfloorz - thing->z > 24*FRACUNIT`, and `PIT_CheckLine` has by then raised `tmfloorz` for every
line the *box* straddles, `P_BoxOnLineSide` counting a box edge that merely touches a line as
straddling it. So the center never comes within the player's radius (16) of the platform's face,
and a 16-unit alcove behind a walkover is a slot no center enters: the trigger can never fire.
`Ir::LIFT_ALCOVE_DIMENSIONS` therefore offers 8/16/32/64 where a door's alcove offers 8/16/32,
the compiler refuses a shallower walkover alcove (`CompileError::LiftAlcoveTooShallow`), and the
verifier refuses to credit the same shape in a map it is only reading (`check::plats`). A
playtest of `maps/ascensor.wad` found this the hard way; `KNOWN-GAPS.md` records what still is
not guarded.

Two conventions the platform takes from measurement rather than from the engine
(`docs/measurements/lift-shapes-2026-08-29.md` §G2): it borrows the **level room's flat** and
light, which is what 40–62 % of the corpus's Core lifts do — `STEP1`/`STEP2`, the
"lift-looking" flats, are a 16–28 % minority that belongs in a theme, not in the construction —
and its **jambs take the theme's `trim` texture**, not `DOORTRAK`, which id never puts on a Core
or Pedestal lift (≤ 2.7 % of DOOM+DOOM2 lift jambs overall, and concentrated on the Barrier
shape).

**A pedestal is that same platform with no portal under it**: a raised island cut inside one room,
carried in its own `pedestals` list as `{ id, room, at, size, rise, speed, things }`. `at` is the
rectangle's low corner (minimum x, minimum y) and `size` its width and height, each a positive
multiple of 8, defaulting to `Ir::PEDESTAL_DEFAULT_SIZE` (64) square; the rectangle must lie
strictly inside its room. The host room is the platform's only neighbor, so the travel is exactly
`rise`. All four edges carry the use special, so the platform can be called down from whichever
side the player walks up to, and `things` are the things that ride it — placed at the raised
floor, each strictly inside the rectangle. A room's own `things` may not stand on a pedestal
(they would spawn in the platform's sector rather than on the room floor the author gave them);
they belong in the pedestal's own list instead.

**Floor actions are a fourth mechanism: one trigger, fired once, moving one or more floors.** A
platform rests, travels and comes back under the player's own use; a floor action goes once and
stays. The corpus authors it that way — 77 % of the floor lines in the sample of record are the
one-shot W1/S1 forms (`docs/measurements/floor-shapes-2026-09-02.md` §A) — so the IR states a
*one-time action* and says nothing about repetition. Three nouns state the three shapes the corpus
builds:

- **A drop wall** is a fifth portal `kind`: a sealed wall sector filling the gap between two
  rooms, its floor at its ceiling so it reads as solid rock, lowered once to the lower room's
  floor by `lowerFloorToLowest`. Its depth along the gap is `thickness`, one of
  `Ir::DROP_WALL_THICKNESS` (8/16/32/64). This is the corpus's monster closet: 1,130 sample
  pockets are reachable only through one (§D).
- **A bridge** is the sixth: a pit strip filling the gap between two rooms *at one floor*
  (`IrError::BridgeFloorsDiffer` otherwise), resting `depth` below them — a positive multiple of
  `Ir::BRIDGE_DEPTH_STEP` (8) — and raised once to their floor by `raiseFloorToNearest`.
- **A reveal** is a sealed island inside one room, in its own `reveals` list as
  `{ id, room, at, size, kind, rise, things, trigger }` — the same rectangle a pedestal is, placed
  and validated by the same rules, but lowered once on a shared trigger instead of resting raised
  under the player's use. Its `kind` is `closet` (floor at the host's ceiling, solid at rest) or
  `pedestal` (resting `rise` above the host's floor, its things on top).

Every construct names a trigger by id — **a reveal in its own `trigger` field, a drop wall or a
bridge in the portal's `fires_on`**. The two words are one concept under two names because
`Portal::trigger` was already taken: on a lift portal that word names where the trigger line is
*placed* (`switch`, `walkover`, `both_ends`), not which trigger fires the portal, so the floor
construct's field had to be called something else (`Portal::fires_on`).

The `triggers` list places them:
`{ id, kind, room, at }` for a `switch` — a use line centered on that room's own wall, exactly as
`Exit::at` is read — or `{ id, kind, portal: [a, b] }` for a `walkover`, which lands on the
opening line of the portal joining those two rooms. A walkover may only name a plain or a bridge
portal (`IrError::WalkoverOnNonPlainPortal`): a door or lift portal's opening already carries a
special. **A bridge names itself**, and that is the shape the construct is built around: the
walkover special goes on both of the pit's own thresholds, so stepping down into the pit is the
crossing that raises it, whichever side the player steps off. One trigger is one sector tag and
one line special, so every construct naming it moves
the same way — all lowering or all rising (`IrError::TriggerMixesFamilies`) — and a trigger no
construct names is an error (`IrError::TriggerUnused`), as is a construct naming no trigger. A map
carries at most `Ir::MAX_FLOOR_ACTIONS` (8) of them, the width the reachability mask's action half
holds (§7.3, P7).

**A reveal's things must fit the cell at rest, and a closet therefore holds nothing.** This is an
engine fact, not a convention. A lowering floor holding a shootable thing that does not fit never
moves: `P_ThingHeightClip` (`p_map.c:530-556`) returns false once `ceilingz - floorz` is under the
thing's height, `PIT_ChangeSector` (`p_map.c:1257-1297`) sets `nofit` for a shootable one,
`T_MovePlane`'s floor-down branch (`p_floor.c:66-92`) restores `lastpos` and returns `crushed`,
and `T_MoveFloor` (`p_floor.c:213-222`) removes the thinker only on `pastdest` — so the floor
retries every tic, forever. With `FLOORSPEED` at `FRACUNIT` (`p_spec.h:600`) the first step of a
sealed cell leaves a one-unit gap, which fits nothing. The compiler therefore measures a reveal's
things against `host.ceiling − rest` and raises `CompileError::RevealNoHeadroom`, and it judges
**every** thing rather than only the shootable ones the engine blocks on: an item sealed in a
closet is engine-legal, but the layer-4 verifier's V-P2 judges every thing against its sector's
static heights, so allowing it would ship a map the project's own checker calls broken. The
monster-closet idiom is therefore a **drop wall with a room behind it**, and the pedestal reveal is
the shape that carries cargo. The same rule, through the same `required_height` helper (species
height, else a blocking or hanging prop's, else the player's), governs a pedestal's cargo, an
island's cargo and a room's own things, so the compiler and V-P2 cannot disagree about what fits.

## 7. Compiler contract

### 7.1 Invariants enforced

1. Each footprint becomes exactly one closed sector, with vertices deduplicated across rooms so
   shared walls join rather than meeting at coincident-but-distinct points.
2. A portal cuts the shared wall of both rooms and emits the two-sided linedef with front and
   back sidedefs bound to the correct sectors.
3. A door gets its own thin sector between the two rooms, with its ceiling snapped to its floor.
   The door action itself lives on the portal's linedefs (special resolved from the vocabulary
   table); a sector tag is emitted only for a remotely triggered door. The point of the invariant
   is that a door needs a sector of its own — specs routinely treat a door as a line.
4. Things are placed with full radius clearance from walls and other blocking things, and with
   the headroom required by P2 — not merely tested point-in-polygon against their room.
5. All coordinates are grid-snapped: no near-miss vertices, no sliver sectors.
6. Emission order is fixed (rooms then portals, in IR order), so identical IR yields
   byte-identical `TEXTMAP`.

### 7.2 Rejections

The compiler refuses rather than emitting degraded geometry, naming the offending room or portal
id in each case:

- overlapping room footprints,
- self-intersecting or unclosed footprints,
- a portal between rooms that share no wall,
- off-grid coordinates,
- an edge that is neither axis-aligned nor 45 degrees,
- a thing outside its room's polygon, or without radius clearance.

### 7.3 Playability invariants

Structural validity (§7.1) makes a map *parseable*. These rules make it *playable*. Every
threshold below is read from the engine constants table (§7.4); no rule hardcodes a number.

**Fit and traversal**

- **P1 — Step height.** A floor-height difference between adjacent sectors intended to be
  traversed on foot must not exceed the engine's maximum step height. Within a stair flight
  every step uses the same rise, and each tread is at least the player's diameter deep.
  **Lift portals are exempt** — spanning a larger delta is precisely their purpose — and are
  governed by P5 instead. Teleport pads are not exempt and do not need to be: a pad's floor sits
  `Ir::PAD_FLOOR_STEP` (8) above its host's, well under the engine's step-up cap, so a pad is
  always walkable onto. What a teleport spans is the gap between the pad and its *destination*,
  and that is governed by P15.
- **P2 — Headroom.** A walkable sector's ceiling-minus-floor gap must be at least the height of
  the tallest thing required to occupy or pass through it. The player is always in that set.
- **P3 — Passage width.** A portal's clear width must be at least twice the largest radius among
  things required to pass through it, plus a margin. This is the rule that catches the classic
  case of a wide monster that cannot fit through a nominally normal door. Blocking decorations
  count against the clear width — a prop narrowing a corridor is the same defect as a narrow
  corridor.
- **P4 — Door opening.** A door's effective opening is the lowest adjacent ceiling less the
  engine's door clearance allowance, measured from the door sector's floor. That opening — not
  the nominal door height — must satisfy P2 for everything that passes through it.
- **P5 — Lift travel and return** *(implemented)*. Every platform rests at its own floor and
  travels to the lowest floor among its two-sided neighbors (`P_FindLowestFloorSurrounding`), so
  the floor it lowers to is not an authored choice — it is whatever its lowest neighbor stands
  at. That travel must exceed the engine's step height, since under it the player simply walks
  up, and must not exceed `progression.lifts.max_travel` — a bound only the verifier's
  conformance row grades, since the compiler never sees a spec. Every sector that calls the platform
  must itself stand at that lowest floor, or the platform will not stop where the caller is. And
  some trigger must fire from that floor: a use special from its front sector (`P_UseSpecialLine`
  is front-side only), or a walkover from whichever side can cross the line at rest. A trigger on
  top is optional — the descent is free — but a lift callable only from above is a trap for the
  player below.
- **P6 — Monster mobility.** A monster's roaming region must not be split by a drop exceeding
  the engine's step height, unless the drop is deliberately one-way.
- **P7 — No softlock.** Every region the player can enter must retain a path back to the
  progression graph. A one-way drop must land somewhere that still reaches the exit.

**Rendering correctness**

- **P8 — No missing textures.** Every one-sided line carries a middle texture. Every two-sided
  line carries an upper texture wherever the two ceilings differ and a lower texture wherever the
  two floors differ, except where both sides are sky. This is the single largest source of HOMs.
- **P9 — No texture scaling.** `scalex` and `scaley` are never emitted; every wall texture
  renders at 1:1. Scaling hides alignment errors rather than fixing them.
- **P10 — Clean vertical tiling.** A wall surface taller than its texture must use a texture whose
  height tiles cleanly into the surface, or the surface height is adjusted to a multiple of the
  texture height. Violations degrade to a warning rather than an error — the result is ugly, not
  broken.
- **P11 — Peg flags.** Door and lift portals set the unpegged flags appropriate to the moving
  sector. A door's track is lower-unpegged so `DOORTRAK` stays put while the ceiling animates
  open; a lift riser keeps the unpegged flag **clear**, which anchors the lower texture to the
  back sector's floor — on the platform's low face that back sector is the platform itself, so
  the riser rides with it. Clear is also the corpus's convention: 96 % of risers carry neither
  flag (`docs/measurements/lift-shapes-2026-08-29.md` §G). The *upper* on a platform boundary
  goes the other way: the engine draws it on whichever side has the taller ceiling — the
  landing's — and the compiler sets `ML_DONTPEGTOP` there so it starts at the landing's own
  ceiling, where the one-sided walls beside it start, instead of at the platform's. That one is
  cosmetic (a plat's ceiling never moves) and the verifier does not judge it, because the corpus
  states no convention to judge against: the flag is set on 51 % / 6 % / 22 % of lift top faces
  in the three populations (§G2).
- **P12 — Sky coherence.** Sky ceilings use the sky flat. Two adjacent sky sectors at differing
  heights are permitted; a sky sector adjacent to a non-sky sector still obeys P8.

**Tags and specials**

- **P13 — Central tag allocation.** All tags come from one allocator. Every sector participating
  in any action receives a unique nonzero tag; tags are never shared between unrelated actions.
  The tag manifest (tag → sector → purpose) is emitted with every run.
- **P14 — No action line carries tag 0.** Tag 0 is not "no tag" to the engine — it is a tag every
  untagged sector already has. A tagged action left at 0 therefore matches *every untagged sector
  in the map*: one stray zero lowers every floor or opens every door. Any linedef carrying a
  sector-affecting special must reference an allocated nonzero tag, and tag 0 on such a line is a
  hard error with no lenient path.

  Manual doors are tagged uniformly too, to a tag resolving to exactly their own sector. A manual
  door never consults its tag — it acts on the line's back sector — so the tag is inert on that
  path and correct on any path that does read it. Tagging everything collapses the error class
  into a single auditable rule instead of a per-special exception list.

  **To verify before implementation:** that uniform tagging is inert under the vanilla
  back-sector path *and* correct under ZDoom's tag-based translation of Doom-format door lines.
  Confirm against primary sources and record the finding in `engine.toml`; do not take the
  paragraph above as established.

- **P15 — Teleport pairing** *(implemented)*. Every teleport line's tag resolves to exactly one
  destination sector, and that sector contains exactly one teleport destination thing with P2
  headroom and full radius clearance for the largest thing that will arrive.

**Content coherence**

- **P16 — Liquid and damage agree.** A liquid flat and a damaging sector special appear together
  or not at all: no damaging floor that looks solid, no harmless-looking pool of lava. The spec's
  `flats.liquid.damaging` governs which.
- **P17 — Damage survivability.** If crossing damaging floor is required for progression, a
  radsuit or the spec's health budget must cover the crossing at the baseline skill.
- **P18 — Secret accounting.** The number of sectors carrying the secret special equals
  `secrets.count`, because the engine's secret counter reads sector specials, not intent.
- **P19 — Light bounds.** Every sector's light level lies within the engine's valid range and
  within the spec's configured min and max.
- **P20 — Pickup accessibility.** Every pickup sits somewhere the player can physically collect
  it: full radius clearance, inside a region the reachability flood reaches, and not embedded in
  a blocking decoration. A pickup deliberately placed out of reach must be declared decorative in
  the IR, so the intent is explicit rather than inferred from failure.
- **P21 — Light sources match lighting.** When `scenery.light_sources.match_lighting` is set,
  every sector brighter than its neighbors by more than the theme's contrast step contains a
  visible light source, and no light-source prop stands in an unlit sector. Bright pools with no
  cause are the clearest tell of a generated map.
- **P22 — Hanging decoration headroom.** A ceiling-mounted prop requires its own height of
  clearance below the ceiling, and the remaining gap beneath it must still satisfy P2 for
  anything that walks under it. Hanging bodies in a low room are a classic invisible blocker.
- **P23 — Barrel safety.** No exploding barrel stands within its blast radius of anything named
  in `scenery.barrels.keep_clear_of`. When `chain_reactions: avoided`, no two barrels stand
  within blast radius of each other.
- **P24 — Key and lock coherence.** `progression.doors.lock_types` is a subset of
  `progression.keys`; every key placed on the map opens at least one door; every locked door has
  its key reachable before it, which the P7 flood proves rather than assumes. An orphan key and
  an unopenable door are both silent progression bugs.
- **P25 — Start clearance.** Every player start — single-player and each coop start — has full
  radius clearance and P2 headroom, and no two starts overlap. Overlapping starts telefrag on
  spawn, which reads as a random coop crash rather than a map defect.
- **P26 — Teleport-only exit room** *(implemented)*. An exit with `trigger: teleport` sits in a
  room with no portal and at least one destination marker — the player arrives by teleport and
  steps across the exit line. Retail's own instance of the shape is TNT MAP23.
- **P27 — No sealed monster room** *(implemented)*. A room holding a monster has a portal or is a
  teleport destination, so sight or sound can ever reach it. A sealed pen with no remote release
  strip has no release at all: retail seals a pen only where a monsters-only teleport or a tier-3
  strip special empties it, and the strip specials are outside this vocabulary.

**Floor actions**

- **P28 — Floor destination** *(implemented)*. Every floor action's destination, re-derived over
  the *emitted* geometry the way `EV_DoFloor` (`p_floor.c`) reads it, is the floor the construct
  intends: `P_FindLowestFloorSurrounding` for a lowering action (a drop wall, a reveal),
  `P_FindNextHighestFloor` for a rising one (a bridge). The destination is not an authored
  choice — it is whatever the target's neighbors stand at — so the compiler asserts it while it
  builds each construct and this rule re-derives it from the records that shipped, catching a
  neighbor some later pass added or moved. It cross-checks the recorded rest against the emitted
  sector floor first and reports that alone when they disagree: every number the destination check
  would then print is derived from a floor that should not be there.
- **P29 — Floor opening** *(implemented)*. A floor action changes where the player can walk, in
  the direction its shape promises: a drop wall or a bridge passes both ways between its two
  neighbors once it has moved, and a reveal is sealed against its host at rest and enterable from
  it afterward. Both halves read the same two refusals the reachability flood does — the crossing
  window (`P_LineOpening`, `p_maputl.c:300-329`: the lower of the two ceilings over the higher of
  the two floors, holding the player's full height, `p_map.c:468-469`) and the step
  (`p_map.c:477-479`) — fed the *effective* heights: the target's `rest` before it fires and its
  `dest` after, its neighbors' own floors throughout.
- **P30 — No chained action** *(implemented)*. No floor action's target borders another action's
  target, a lift platform, or a door sector. Both destination searches read the *current* floors
  of the target's neighbors, so a neighbor that moves makes the destination a function of when the
  trigger is pulled; crustygen evaluates it at load, which is exact only without such a chain. The
  corpus prices the exposure: one target in five borders another, but only one in twenty-three has
  its *destination* defined by a movable neighbor
  (`docs/measurements/floor-shapes-2026-09-02.md` §G). A door sector is read off the **back** side
  of a door line rather than off a tag, because every door special this vocabulary names is a
  manual `DR` form carrying no tag at all — `EV_VerticalDoor` (`p_doors.c`) takes
  `sides[line->sidenum[1]].sector`.

  P30 is also what lets P7's flood model a floor action at all. Each action takes one bit of the
  reachability mask and a node carries at most one action, so nothing composes: without the
  no-chain rule a sector's effective floor would depend on the order two triggers were pulled in,
  which the flood does not model. The mask is a `u16` — bits 0..8 the key classes, bits 8..16 the
  floor actions — which is where `Ir::MAX_FLOOR_ACTIONS` (8) comes from.

### 7.4 Engine constants and specials table

The thresholds P1–P30 depend on live in `engine.toml` alongside `vocabulary.toml`: maximum step
height; player radius and height; per-species radius and height; the door clearance allowance;
the `[plat]` block behind every platform (the `downWaitUpStay` and `blazeDWUS` speeds, the wait
at the bottom, and `MAXPLATS`); the `[floor]` block behind every floor action — `FLOORSPEED`, the
turbo multiplier and the raise-and-change plat divisor, all sourced to `p_spec.h`/`p_floor.c`/
`p_plats.c`, alongside the two `curated` authoring bounds `drop_wall_thickness` and
`bridge_depth_step`, which are corpus-bounded choices rather than engine constants and say so in
their citation; barrel blast radius and damage; decoration radii, heights, and blocking flags;
sector damage specials by tier; the secret sector special; the sky flat name; the valid light
range; and which specials consume a tag.

Both tables follow the same sourcing rule: **every entry carries a citation — a `source` field
recording a primary source, a `derivation` for a computed value, or `curated` for a judgment call
that has no primary source (and which must not claim one).** Nothing in either table is written
from recall.

This matters more here than it does for vocabulary. A wrong texture name produces a visible
defect; a wrong step height or monster radius produces a map that loads, looks correct, and
traps the player or strands a monster in a doorway. Tests cannot catch it, because the test and
the compiler would share the same wrong constant.

## 8. Verification

Five layers, cheapest first.

1. **Compiler invariants** — structural (§7.1) and playability (§7.3) — fail before any WAD
   exists.
2. **Texture and flat existence** — every texture and flat named in the IR is checked against the
   target IWAD's own lumps, so a typo cannot reach the engine as a HOM.
3. **`cwad validate` plus the assembler's `MapWarning` set** gives crustywad's verdict on the
   built file.
4. **`crustygen-check`** works on the assembled map rather than the IR, which is what makes it
   more than a restatement of layer 1: it re-derives the playability invariants from the actual
   emitted geometry, so a compiler bug that satisfies its own pre-checks still gets caught. On
   top of that it does spec conformance — every derivable frontmatter number against its actual, a
   key-aware reachability flood from player start to exit, secret-special count, exit trigger
   presence, tag manifest consistency, and the ammo ratio computed against real baseline
   monster HP.
5. **Convert round-trip** — `cwad convert --to doom --nodes`, reparse both artifacts, compare
   counts. Surviving two very different lump paths demonstrates more structural soundness than
   either path alone.

Human playtesting in GZDoom follows, but is not part of the automated gate.

### 8.1 Conformance report

The deliverable of a run is the WAD *and* `report.md`: a table of every parameter, its target,
its actual value, and a verdict, plus an explicit list of sacrifices made under the priority
order. It also carries the playability invariant results (P1–P27, pass or fail with the offending
room, portal, or thing named) and the tag manifest. A run that produces a WAD but no passing report is
not a result.

## 9. Error handling

Errors are structured and name their subject (room id, portal id, or spec field path). The
`constraints.enforcement` field selects behavior for budget violations:

- `strict` — any range violation is an error; nothing is written.
- `target` — a violation is a warning recorded against `constraints.priority`, phrased as which
  parameter was sacrificed to hold which.

Compiler geometry violations (§7.2) are always errors regardless of enforcement mode: they
describe impossible geometry, not an unmet preference.

Playability violations (§7.3) are likewise always errors, with one exception. A door the player
cannot fit through, a stair the player cannot climb, a missing texture, or a teleport whose tag
resolves to nothing are all broken maps, not missed targets — `enforcement: target` does not
soften them. The exception is **P10 (clean vertical tiling)**, which degrades to a warning: a
badly tiled wall is ugly, not broken.

Where a playability rule and a spec parameter genuinely conflict — the spec asks for a wide
monster in a room whose doors are too narrow for it — the compiler reports the conflict and names
both sides. Resolution happens upstream, when the IR is authored, governed by
`constraints.priority`. The compiler never silently widens a door or drops a monster.

## 10. Testing

- Compiler unit tests, one per invariant in §7.1: closure, vertex dedup, portal sidedef pairing,
  door sector emission, point-in-polygon rejection, and determinism (same IR twice, byte-identical
  output).
- Compiler rejection tests, one per case in §7.2.
- A playability test per rule in §7.3: a minimal IR that violates the rule by exactly one unit
  must fail, and the same IR at the threshold must pass. Boundary-pinned, so a wrong comparison
  operator cannot hide.
- A tag-allocator test: unique allocation, no reuse across unrelated actions, manual doors left
  at tag 0, and every tag referenced by a special resolving to at least one sector.
- A golden `TEXTMAP` test over a small fixture IR, so output drift is visible in review.
- An end-to-end test on the fixture IR: compile, pack, convert, reparse, assert counts match.

## 11. Out of scope for v1

No slopes, no 3D floors, no ACS or `BEHAVIOR`, no custom textures or WAD-embedded assets, no
polyobjects, no voodoo-doll mechanics, no friendly monsters, no curved geometry beyond 45-degree
edges, and no procedural layout — room graphs are authored, not generated.

Two entries in the template are accepted but **not enforced** in v1, and the conformance report
says so rather than implying coverage:

- `compat.port: vanilla_limits` is recorded, but visplane and drawseg limits are not modeled.
  A map declaring vanilla limits may still exceed them.
- `players.dm_starts` places deathmatch starts, but weapon and ammo layout is never tuned for
  deathmatch balance.

`compat.emit_mapinfo` defaults to false because v1 emits no lumps beyond the map group;
`par_time_seconds` is therefore inert until that changes.

## 12. Promotion path

If the experiment lands, `crustygen` becomes a standalone repo consuming crustywad as a pinned
dependency (the crustyview pattern). The units transfer unchanged: the template becomes the web
form's schema, the IR becomes the editor's document model, the compiler becomes the WASM core,
and the verifier becomes the product's quality gate. Nothing in this design assumes a CLI-only
front end.

## 13. Risks

- **Geometry vocabulary too narrow.** Axis-aligned plus 45 degrees may not carry "coherent and
  deliberate" for a whole map. Mitigation: the first map is designed against the constraint, not
  in spite of it; widen the compiler only if the result reads as blocky rather than clean.
- **Table accuracy is the highest-stakes data in the project.** Wrong thing IDs or line specials
  in `vocabulary.toml`, or a wrong step height, monster radius, or door clearance in
  `engine.toml`, produce maps that load, look correct, and are unplayable. Unit tests cannot
  catch it: the test and the compiler read the same table. Mitigation: a citation on every entry
  (a `source`, a `derivation`, or `curated` — never written from recall), a table-versus-IWAD check
  in verification, and boundary-pinned
  playability tests that at least prove the comparison logic is right even when a threshold is
  not.
- **Authoring the IR is still work.** The compiler removes coordinate bookkeeping, not layout
  design. A 14-room map is a real design effort per run, which is the honest cost of not going
  procedural.
