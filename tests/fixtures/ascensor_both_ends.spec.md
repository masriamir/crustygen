---
spec_version: 1

# Identity and target
identity:
  slot: MAP01
  title: "Ascensor"
  author: "Amir Masri"
  iwad: doom2                # doom2 | freedoom2
  outputs: [udmf, doom]      # udmf is authored; doom is produced by cwad convert
  seed: 20260829             # reproducibility; same seed + same IR = byte-identical TEXTMAP
  grid: 64                   # map units; all coordinates snap to this

# Players and starts
players:
  start_facing: north        # north | south | east | west, or degrees
  coop_starts: 0             # 0 disables coop; 4 is the conventional set
  dm_starts: 0                # 0 disables deathmatch
  coop_only_items: false     # place extra pickups flagged multiplayer-only

# Scale budget
scale:
  size: { width: 2400, height: 1200 }    # bounding box, map units
  rooms: { min: 5, max: 7 }
  sectors: { min: 12, max: 20 }
  linedefs: { min: 50, max: 200 }
  play_time_minutes: { min: 1, max: 3 }
  vertical_range: { min: 0, max: 128 }    # allowed floor heights; the map's span is max - min

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
    count: { min: 4, max: 6 }
    trigger: both_ends       # walkover | switch | both_ends
    max_travel: 256          # largest floor delta a lift may span
  teleports:
    count: { min: 0, max: 0 }
  doors:
    speed: normal            # normal | fast
    default_behavior: repeatable   # repeatable | one_shot | stays_open
    lock_types: []            # must be a subset of progression.keys (P24)
  switches:
    count: { min: 9, max: 9 }
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
  landmarks: 4               # visually distinct anchors aiding navigation

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
    ratio: 7.246              # placed ammo damage / total baseline monster HP; overrides budget
    distribution: front_loaded   # front_loaded | even | back_loaded
    pickups: auto            # auto (derived from ratio) | explicit counts per pickup type
    backpack: { count: 0, placement: none }

# Health, armor, powerups
sustain:
  health_budget: tight       # tight | balanced | generous; explicit counts below override it
  health:
    stimpack: 1
    medikit: 0
    health_bonus: 0          # the +1 bonuses; they matter for a tight-budget map
  armor:
    green: 0
    blue: 0
    armor_bonus: 0
  powerups:                  # count 0 means deliberately absent
    - { name: berserk,         count: 0, placement: none }
    - { name: soulsphere,      count: 1, placement: late }
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
    max: 176
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
    - "a small map whose whole vocabulary is the moving floor"
  must_include:
    - "a lift the player calls with a switch and rides up to a ledge"
    - "a barrier the player lowers from either side to pass"
    - "a pedestal the player calls down to take its prize"
  priority:                  # highest first; resolves conflicts between everything above
    - progression_correctness
    - playable_balance
    - sector_budget
    - monster_counts
    - detail_level
    - play_time
---

## Overview

Ascensor is the platform toolchain's own hand-authored payoff map: a chain of
six rooms in which every floor the player needs is a floor that moves. Three
lift portals, one barrier and one pedestal appear — every platform shape the
compiler emits — and each of the three lift triggers the IR offers is used
once.

The entry room's lift north is a switch call. The ledge's lift east onto the
yard is a `both_ends` call, fast. The yard's east gap is a barrier: a
platform risen 96 above two rooms that share a floor, lowered from either
side. A plain passage runs south out of yard2 into the hall, where a pedestal
holds a soulsphere the player must call down. The hall's lift east into the
exit room is a walkover, recessed into a near alcove.

## Sequence of events

1. The player starts in the entry room at (256, 128) facing north, with a
   shotgun off to the west. The north wall carries a lift's low face: a
   switch that calls the platform down out of the ledge above.
2. Riding it up puts the player on the ledge, 128 above the entry room, where
   an imp waits.
3. The ledge's east wall carries the second lift, a fast one with a trigger on
   both faces, down into the yard — the second imp's room, with a box of
   shells in the near corner.
4. The yard's east gap is filled by a barrier: it rests 96 above both floors,
   too tall to step over, and either side's switch lowers it. Past it is
   yard2, holding a stimpack.
5. A plain passage runs south out of yard2 into the hall. The pedestal there
   rests 128 up with a soulsphere on it; any of its four faces calls it down.
6. The hall's east wall carries the last lift, a walkover in a near alcove,
   up into the exit room — 128 above the hall — whose east wall carries the
   switch that ends the level.

## Notes

Ascensor is the drift-guarded lift fixture: `tests/build_cli.rs` rebuilds it
and compares the bytes against the committed `maps/ascensor.wad`, and
`tests/check_conformance.rs` judges the emitted TEXTMAP against this spec.
Like `salto.spec.md`, every derivable number here was set from ascensor's own
compiled output rather than from a design goal held independent of it — as
the exact value where the row grades one, and as a bound that contains the
measured value where the row grades a range.

**This is the switched variant of `ascensor.spec.md`.** It is that document
with one word changed: `progression.lifts.trigger` asks `both_ends` instead
of `switch`. The row fails either way, with the same `actual` —
`switch ×1, walkover ×1, both_ends ×1` — because the map uses each trigger
once and the row takes one word. Only the target moves, which is what
`tests/check_conformance.rs` uses this file to show. Nothing else here
differs from `ascensor.spec.md`, and nothing else should: the two documents
are compared row for row.

This spec was derived from `salto.spec.md`; the keys that differ, and why:

- `scale.size` reset to 2400x1200 (salto: 1500x1200) — wider, same height budget.
  Ascensor's emitted bounding box is 2240x1088: the six rooms run west to
  east as one chain, and salto's 1500-unit width budget fails on it.
- `scale.sectors` left at salto's 12..20. Ascensor emits 13: 6 rooms + 3
  lift platforms + 1 barrier + 1 plain passage + 1 lift alcove + 1 pedestal.
- `scale.vertical_range` reset to 0..128 (salto: -32..32). The row bounds
  each individual floor, not the span: the ledge, the exit room and the
  pedestal all rest at 128, the barrier at 96, and nothing sits below 0.
- `scale.rooms` set to 5..7 (ascensor has six rooms) — a `NotDerivable` row,
  restated for this map.
- `progression` carries the values the map itself emits: no keys, no locked
  doors, no teleports, a switch exit, no walkover *exit* line (the row counts
  exit lines only, and ascensor's walkover is a lift's), five platforms, and
  the nine switch lines they and the exit add up to (1 exit + 1 entry lift +
  1 ledge lift + 2 barrier faces + 4 pedestal faces). `max_travel` is 256
  against a measured 128 — the largest travel any of the five platforms has.
- `progression.shape` is `linear` and `backtracking` `none`: the six rooms
  form one chain with no branch to come back to.
- `combat` drops salto's closet: two imps, neither deaf, no teleport ambush,
  and `encounter_style: incidental`.
- `combat.hitscanner_ratio`, `combat.ambush.deaf_ratio` and
  `arsenal.ammo.ratio` are `Info` rows, restated to ascensor's measured
  values — 0.000, 0.000 and 7.246: no hitscanner is placed, no monster is
  flagged deaf, and a shotgun plus one box of shells is the map's ammo
  against two imps.
- `sustain.powerups` names the soulsphere the pedestal carries; the rest stay
  at zero, and the stimpack in yard2 is the map's only health.
- `aesthetics.lighting.min`/`.max` set to 144..176 (salto: 96..176) — the
  ledge is authored at 144 and the yard and yard2 at 176, and both bounds
  grade the emitted extremes.
- `flats.floor` drops salto's `GATE3`: ascensor emits no teleport pad, so no
  pad flat.
- The remaining differences (`architecture.verticality`, `arsenal`, `pacing`,
  `constraints`, and the prose) are ungraded descriptive keys restated for a
  six-room lift map instead of salto's five-room teleport one.
