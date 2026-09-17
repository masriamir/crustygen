---
spec_version: 1

# Identity and target
identity:
  slot: MAP01
  title: "Hilera"
  author: "Amir Masri"
  iwad: doom2                # doom2 | freedoom2
  outputs: [udmf, doom]      # udmf is authored; doom is produced by cwad convert
  seed: 20260912             # reproducibility; same seed + same IR = byte-identical TEXTMAP
  grid: 64                   # map units; all coordinates snap to this

# Players and starts
players:
  start_facing: south        # north | south | east | west, or degrees
  coop_starts: 0             # 0 disables coop; 4 is the conventional set
  dm_starts: 0                # 0 disables deathmatch
  coop_only_items: false     # place extra pickups flagged multiplayer-only

# Scale budget
scale:
  size: { width: 1792, height: 640 }    # bounding box, map units
  rooms: { min: 3, max: 3 }
  sectors: { min: 10, max: 20 }
  linedefs: { min: 50, max: 200 }
  play_time_minutes: { min: 1, max: 3 }
  vertical_range: { min: 0, max: 224 }    # allowed floor heights; the map's span is max - min

# Progression
progression:
  shape: linear              # linear | hub_and_spoke | branching | gauntlet
  keys: []
  locked_doors: 0
  backtracking: none         # none | light | heavy
  exit:
    kind: normal             # normal | secret | both
    trigger: switch          # switch | teleport | walkover
  lifts:
    count: { min: 7, max: 7 }
    trigger: switch          # walkover | switch | both_ends
    max_travel: 128          # largest floor delta a lift may span
  teleports:
    count: { min: 0, max: 0 }
  doors:
    speed: normal            # normal | fast
    default_behavior: repeatable   # repeatable | one_shot | stays_open
    lock_types: []            # must be a subset of progression.keys (P24)
  switches:
    count: { min: 8, max: 8 }
    remote_allowed: false    # a switch may act on a distant sector
  walkover_triggers:
    count: { min: 0, max: 0 }

# Architecture
architecture:
  room_shapes: [rectangular]  # rectangular | l_shaped | t_shaped | octagonal | irregular
  symmetry: axial            # organic | axial | radial | mixed
  openness: open             # tight | mixed | open
  corridor_ratio: 0.1        # fraction of floor area that is transit rather than space
  verticality: strong        # flat | moderate | strong
  inter_area_windows: false   # sightlines between areas that are not directly connected
  overlooks: { min: 0, max: 0 }   # elevated vantage points over another area
  landmarks: 3               # visually distinct anchors aiding navigation

# Combat
combat:
  encounter_style: incidental    # incidental | ambush | arena | corridor
  hitscanner_ratio: 0.0            # fraction of total monster count that is hitscan
  max_simultaneous: 2           # pressure ceiling: most monsters active at once
  monster_closets: 0
  boss: none                 # none | mastermind | <species> — mastermind is the spider_mastermind species
  ambush:
    deaf_ratio: 0.0          # fraction of monsters flagged deaf: wake on sight, not on sound
    teleport_ambushes: { min: 0, max: 0 }
  sound:
    propagation: open   # open | contained | sealed
    block_sound_at: []            # key_doors | arena_entrances
  block_monster_lines: false  # keep monsters in their region without a wall
  monsters:
    - { species: imp, min: 2, max: 2 }

# Weapons and ammo
arsenal:
  pistol_start: required_viable        # required_viable | not_required
  weapons:
    - { name: shotgun, placement: early }        # early | mid | late | secret_only
  ammo:
    budget: generous         # tight | balanced | generous
    ratio: 7.246               # placed ammo damage / total baseline monster HP; overrides budget
    distribution: front_loaded   # front_loaded | even | back_loaded
    pickups: auto            # auto (derived from ratio) | explicit counts per pickup type
    backpack: { count: 0, placement: none }

# Health, armor, powerups
sustain:
  health_budget: tight       # tight | balanced | generous; explicit counts below override it
  health:
    stimpack: 1
    medikit: 1
    health_bonus: 0          # the +1 bonuses; they matter for a tight-budget map
  armor:
    green: 0
    blue: 0
    armor_bonus: 0
  powerups:                  # count 0 means deliberately absent
    - { name: berserk,         count: 0, placement: none }
    - { name: soulsphere,      count: 1, placement: early }
    - { name: megasphere,      count: 0, placement: none }
    - { name: radsuit,         count: 0, placement: none }
    - { name: invulnerability, count: 0, placement: none }
    - { name: invisibility,    count: 0, placement: none }
    - { name: light_amp,       count: 0, placement: none }
    - { name: computer_map,    count: 0, placement: none }

# Secrets
secrets:
  count: 0                   # per-secret detail lives in the prose body

# Difficulty
difficulty:
  skills_supported: true     # emit real easy/medium/hard thing flags
  baseline: uv               # itytd | hntr | hmp | uv | nm — the skill the counts above describe
  curve: gentle               # gentle | steep | late_spike
  scaling: { easy: 0.55, medium: 0.75, hard: 1.0 }

# Aesthetics
aesthetics:
  theme: tech_base           # tech_base | hell | gothic | city | cave | marble | wood
  texture_set: [STARTAN3]    # auto (theme-derived) | explicit list of texture names
  detail_level: 2             # 1..5
  lighting:
    style: contrasty          # flat | contrasty | pools_of_dark
    base: 160                 # default sector light where nothing else applies
    min: 144                   # floor and ceiling for every emitted light level (P19)
    max: 160
    contrast_step: 16          # the delta that counts as a deliberate light change (P21)
    corridor_delta: 0        # corridor light relative to the rooms it joins
    outdoor: 160               # light level for sky-ceilinged sectors
    effects:
      allowed: []              # subset; [] for none
      density: none            # none | sparse | medium | dense
      forbid_in: [combat_arenas, secret_rewards]    # combat_arenas | secret_rewards
    per_room_overrides: true  # rooms may set their own level and effect in the IR
  sky: auto
  music: auto
  texture_scaling: forbidden # forbidden | allowed; v1 never emits scalex/scaley (see P9)

# Flats and liquids
flats:
  floor: [FLOOR4_8]         # auto (theme-derived) | explicit list of flat names
  ceiling: [CEIL3_5]
  outdoor_proportion: 0.0    # fraction of floor area with a sky ceiling
  light_flats: false          # bright ceiling flats beneath light sources
  liquid:
    kind: none              # none | nukage | blood | lava | slime | water
    damaging: false           # pair a damaging sector special with the liquid flat (see P16)
    damage_tier: light         # light | medium | heavy; resolved to sector specials via engine.toml
    coverage: 0.0             # fraction of floor area
    crossing_required: false  # must the player enter it to progress?
    radsuit_provided: false    # if crossing_required, radsuit or health budget must cover it (P17)

# Vertical form
vertical:
  stairs:
    flights: { min: 0, max: 0 }
    rise_per_step: 16        # uniform within a flight; must not exceed engine max step height (P1)
    tread_depth: 32          # must be at least the player's diameter (P1)
  standard_ceiling: 128      # default room height where the spec says nothing
  door_opening: 128          # nominal door height; effective opening derived per P4

# Scenery: decoration, light sources, hazards
scenery:
  light_sources:
    density: none             # none | sparse | medium | dense
    kinds: auto               # auto (theme-derived) | explicit list
    match_lighting: false     # every bright pool gets a visible source (P21)
  decorations:
    density: none          # none | sparse | medium | dense
    kinds: auto
    blocking_allowed: false   # movement-blocking props, still subject to P3
    hanging_allowed: false    # ceiling-mounted props, subject to headroom (P22)
  gore: none                # none | light | heavy: corpses, blood, impaled bodies
  barrels:
    count: { min: 0, max: 0 }
    placement: none          # near_encounters | scattered | none
    chain_reactions: avoided       # allowed | avoided
    keep_clear_of: [player_start, key_pickup, secret_reward]   # player_start | key_pickup | secret_reward

# Pacing
pacing:
  encounter_beats: { min: 1, max: 2 }
  rest_areas: { min: 1, max: 2 }
  peak_position: 0.5         # where the hardest fight sits, as a fraction of progression
  opening_intensity: low     # low | medium | high

# Compatibility and metadata
compat:
  port: limit_removing       # vanilla_limits | limit_removing | boom | zdoom
  emit_mapinfo: false        # v1 emits no extra lumps; par_time needs this
  par_time_seconds: 90       # ignored unless emit_mapinfo is true
  automap:
    hide_secret_lines: true  # a secret door does not read as a door on the automap
    show_map_lines: auto

# Constraints and priorities
constraints:
  enforcement: target        # strict (ranges are hard limits) | target (ranges are goals)
  forbid: [archvile, crusher, dark_maze, insta_death_pit]  # species names, or: crusher | dark_maze | insta_death_pit
  inspirations:
    - "a small map whose whole vocabulary is a bank of platforms called by one switch"
  must_include:
    - "a lift pair the player calls with one switch and rides up together"
    - "a barrier pair the player lowers together from either side to pass"
    - "a row of three pedestals the player calls down together with one switch"
  priority:                  # highest first; resolves conflicts between everything above
    - progression_correctness
    - playable_balance
    - sector_budget
    - monster_counts
    - detail_level
    - play_time
---

## Overview

Hilera ("row") is the lift-bank construct's own hand-authored payoff map:
three rooms joined by two portal banks, plus a third bank of pedestals with
no portal at all — every bank *shape* the construct offers (a lift pair, a
barrier pair, and a row of three prizes), each called from a single switch
that moves every member sharing its tag.

`hall` is the start; its south wall holds three pedestals in a row, called
together from the first one's switch. Its east wall holds a lift pair into
`ledge`, called together from the southern strip's switch. `ledge`'s east
wall holds a barrier pair into `gate`, lowered together from either side.
`gate`'s east wall carries the switch that ends the level.

## Sequence of events

1. The player starts in `hall` at (320, 512) facing south, with a shotgun
   off to the west at (128, 512). Ahead, along the south wall, sit three
   pedestals in a row — `p1`, `p2`, `p3` — each resting 64 above the floor
   with a pickup on top: a box of shells, a medikit, a soulsphere.
2. Pressing the switch on `p1`'s faces (the only member of the `prizes` bank
   with a line of its own) lowers all three pedestals together, since every
   member shares one sector tag — the player collects all three pickups from
   one call.
3. `hall`'s east wall carries the `pair` bank: two lift strips resting flush
   with `ledge`'s floor, 128 above `hall`'s. The southern strip's switch
   calls both down to `hall`'s floor; the player steps onto either and rides
   back up onto `ledge`, where two imps wait.
4. `ledge`'s east wall carries the `bars` bank: two barrier strips resting 96
   above the shared floor `ledge` and `gate` sit at (128), too tall to step
   over. Either strip's switch — reachable from both `ledge` and `gate` —
   lowers both together to that shared floor, and they rise back once
   crossed. Past them, `gate` holds a stimpack.
5. `gate`'s east wall carries the switch at (1792, 320) that ends the level.

## Notes

Hilera is the drift-guarded lift-bank fixture: `tests/build_cli.rs` rebuilds
it and compares the bytes against the committed `maps/hilera.wad`,
`tests/check_conformance.rs` judges the emitted TEXTMAP against this spec,
and `tests/check_adversarial.rs` cross-examines the built map for unmodeled
specials and softlocks. Like `ascensor.spec.md` and `muralla.spec.md`, every
derivable number here was set from hilera's own compiled output rather than
from a design goal held independent of it — as the exact value where the row
grades one, and as a bound that contains the measured value where the row
grades a range.

**No row is left failing.** Every row this spec produces is `Pass`, `Info` or
`NotDerivable`; `tests/check_conformance.rs` asserts exactly that.

This spec was derived from `ascensor.spec.md`; the keys that differ, and why:

- `identity` names hilera and its own seed, 20260912 — the seed the fixture
  carries.
- `scale.size` reset to 1792x640 (ascensor: 2400x1200), hilera's emitted
  bounding box exactly: three 640-wide rooms in a single east-west row, none
  of them offset in `y`.
- `scale.rooms` set to 3..3 (hilera has exactly three rooms) — a
  `NotDerivable` row, restated for this map.
- `scale.sectors` left at 10..20 (ascensor: 12..20, widened at the floor to
  contain hilera's 10). Hilera emits 10: 3 rooms + 2 `pair`-bank lift
  platforms + 2 `bars`-bank barrier platforms + 3 `prizes`-bank pedestal
  islands. No alcove is emitted anywhere — every trigger in this map is
  `switch` or `none`, and only a `walkover` lift needs an alcove.
- `scale.vertical_range` reset to 0..224 (ascensor: 0..128). The row bounds
  each individual floor, not the span: `hall` sits at 0, `ledge` and `gate`
  at 128, the `prizes` pedestals rest at 64, and the `bars` barrier rests at
  224 (128 + the 96 rise) — the highest floor on the map.
- `progression` carries the values the map itself emits: no keys, no locked
  doors, no teleports, a switch exit, seven platforms (2 + 2 + 3 across the
  three banks) and the eight switch lines they and the exit add up to: 1 exit
  switch, 1 switch on the `pair` bank's own member (its low face, in `hall`),
  2 barrier faces on the `bars` bank's own member (both faces, since a
  barrier's switch sits on each side), and 4 pedestal faces on `p1`, the
  `prizes` bank's own member (every edge of the island). Each bank's other
  member sets `trigger: none` and places no line of its own — the shared tag
  is what moves it. `max_travel` is 128 against a measured 128 — `pair`'s own
  travel (`hall`'s 0 to `ledge`'s 128) is the largest of the seven platforms',
  ahead of `bars`'s 96 and `prizes`'s 64.
- `progression.lifts.trigger` asks for `switch` and passes: the map's only
  `Rest::Top` platforms are `pair`'s two lift strips, and the checker credits
  every sector sharing a tag with that tag's lines, so both read `switch` —
  the one switch line the bank's own member carries. `bars` and `prizes` rest
  `AboveAll` (a barrier and a pedestal, not a lift a player rides up), so
  neither is judged by this row.
- `progression.shape` is `linear` and `backtracking` `none`: the three rooms
  form one chain with no branch to come back to.
- `combat` keeps ascensor's two imps, both in `ledge`, neither deaf, no
  teleport ambush, `encounter_style: incidental`.
- `combat.hitscanner_ratio`, `combat.ambush.deaf_ratio` and
  `arsenal.ammo.ratio` are `Info` rows, restated to hilera's measured values —
  0.000, 0.000 and 7.246, the same figure ascensor reports: no hitscanner is
  placed, no monster is flagged deaf, and a shotgun plus one box of shells is
  the map's ammo against `ledge`'s two imps — the same weapon, ammo and
  monster count ascensor's own lift-then-yard leg carries.
- `sustain.health` carries hilera's own pickups: the stimpack in `gate` and
  the medikit `p2` holds; `sustain.powerups` names the soulsphere `p3` holds.
  Every other count stays at zero.
- `aesthetics.lighting.min`/`.max` set to 144..160 (ascensor: 144..176): the
  emitted lights are `hall` and `gate` at 160 and `ledge` at 144, so 176 is
  no longer a bound anything reaches.
- The remaining differences (`architecture.landmarks`, `arsenal`, `pacing`,
  `constraints`, and the prose) are ungraded descriptive keys restated for a
  three-room, three-bank map instead of ascensor's six-room single-platform
  one.
