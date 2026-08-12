---
spec_version: 1

# Identity and target
identity:
  slot: MAP01
  title: "Entrada"
  author: "Amir Masri"
  iwad: doom2                # doom2 | freedoom2
  outputs: [udmf, doom]      # udmf is authored; doom is produced by cwad convert
  seed: 20260809             # reproducibility; same seed + same IR = byte-identical TEXTMAP
  grid: 64                   # map units; all coordinates snap to this

# Players and starts
players:
  start_facing: east         # north | south | east | west, or degrees
  coop_starts: 0             # 0 disables coop; 4 is the conventional set
  dm_starts: 0                # 0 disables deathmatch
  coop_only_items: false     # place extra pickups flagged multiplayer-only

# Scale budget
scale:
  size: { width: 2100, height: 900 }     # bounding box, map units
  rooms: { min: 6, max: 10 }
  sectors: { min: 15, max: 25 }
  linedefs: { min: 50, max: 200 }
  play_time_minutes: { min: 2, max: 5 }
  vertical_range: { min: -32, max: 32 }    # allowed floor heights; the map's span is max - min

# Progression
progression:
  shape: branching           # linear | hub_and_spoke | branching | gauntlet
  keys: [blue_card]
  locked_doors: 1
  backtracking: light        # none | light | heavy
  exit:
    kind: normal             # normal | secret | both
    trigger: switch          # switch | teleport | walkover
  lifts:
    count: { min: 0, max: 0 }
    trigger: both_ends       # walkover | switch | both_ends
    max_travel: 256          # largest floor delta a lift may span
  teleports:
    count: { min: 0, max: 0 }
  doors:
    speed: normal            # normal | fast
    default_behavior: repeatable   # repeatable | one_shot | stays_open
    lock_types: [blue_card]   # must be a subset of progression.keys (P24)
  switches:
    count: { min: 1, max: 1 }
    remote_allowed: true     # a switch may act on a distant sector
  walkover_triggers:
    count: { min: 0, max: 0 }

# Architecture
architecture:
  room_shapes: [rectangular, octagonal]  # rectangular | l_shaped | t_shaped | octagonal | irregular
  symmetry: organic          # organic | axial | radial | mixed
  openness: mixed            # tight | mixed | open
  corridor_ratio: 0.25       # fraction of floor area that is transit rather than space
  verticality: moderate      # flat | moderate | strong
  inter_area_windows: false   # sightlines between areas that are not directly connected
  overlooks: { min: 0, max: 1 }   # elevated vantage points over another area
  landmarks: 1               # visually distinct anchors aiding navigation

# Combat
combat:
  encounter_style: incidental    # incidental | ambush | arena | corridor
  hitscanner_ratio: 0.333          # fraction of total monster count that is hitscan
  max_simultaneous: 3           # pressure ceiling: most monsters active at once
  monster_closets: 0
  boss: none                 # none | mastermind | <species> — mastermind is the spider_mastermind species
  ambush:
    deaf_ratio: 0.0          # fraction of monsters flagged deaf: wake on sight, not on sound
    teleport_ambushes: { min: 0, max: 0 }
  sound:
    propagation: open   # open | contained | sealed
    block_sound_at: [key_doors]   # key_doors | arena_entrances
  block_monster_lines: false  # keep monsters in their region without a wall
  monsters:
    - { species: zombieman,   min: 1, max: 1 }
    - { species: shotgun_guy, min: 1, max: 1 }
    - { species: imp,         min: 2, max: 2 }
    - { species: pinky,       min: 1, max: 1 }
    - { species: hell_knight, min: 1, max: 1 }

# Weapons and ammo
arsenal:
  pistol_start: required_viable        # required_viable | not_required
  weapons:
    - { name: shotgun,         placement: early }        # early | mid | late | secret_only
    - { name: chaingun,        placement: mid }
    - { name: rocket_launcher, placement: late }
  ammo:
    budget: generous         # tight | balanced | generous
    ratio: 5.215              # placed ammo damage / total baseline monster HP; overrides budget
    distribution: even       # front_loaded | even | back_loaded
    pickups: auto            # auto (derived from ratio) | explicit counts per pickup type
    backpack: { count: 0, placement: none }

# Health, armor, powerups
sustain:
  health_budget: balanced    # tight | balanced | generous; explicit counts below override it
  health:
    stimpack: 2
    medikit: 1
    health_bonus: 1          # the +1 bonuses; they matter for a tight-budget map
  armor:
    green: 1
    blue: 0
    armor_bonus: 0
  powerups:                  # count 0 means deliberately absent
    - { name: berserk,         count: 1, placement: mid }
    - { name: soulsphere,      count: 2, placement: late }
    - { name: megasphere,      count: 0, placement: none }
    - { name: radsuit,         count: 0, placement: none }
    - { name: invulnerability, count: 0, placement: none }
    - { name: invisibility,    count: 0, placement: none }
    - { name: light_amp,       count: 0, placement: none }
    - { name: computer_map,    count: 0, placement: none }

# Secrets
secrets:
  count: 1                   # per-secret detail lives in the prose body

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
    style: flat               # flat | contrasty | pools_of_dark
    base: 160                 # default sector light where nothing else applies
    min: 160                   # floor and ceiling for every emitted light level (P19)
    max: 160
    contrast_step: 32          # the delta that counts as a deliberate light change (P21)
    corridor_delta: 0        # corridor light relative to the rooms it joins
    outdoor: 160               # light level for sky-ceilinged sectors
    effects:
      allowed: []              # subset; [] for none
      density: none            # none | sparse | medium | dense
      forbid_in: [combat_arenas, secret_rewards]    # combat_arenas | secret_rewards
    per_room_overrides: false # rooms may set their own level and effect in the IR
  sky: auto
  music: auto
  texture_scaling: forbidden # forbidden | allowed; v1 never emits scalex/scaley (see P9)

# Flats and liquids
flats:
  floor: [FLOOR4_8]                # auto (theme-derived) | explicit list of flat names
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
    density: sparse           # none | sparse | medium | dense
    kinds: [techno_lamp]      # auto (theme-derived) | explicit list
    match_lighting: false     # every bright pool gets a visible source (P21)
  decorations:
    density: none          # none | sparse | medium | dense
    kinds: auto
    blocking_allowed: false   # movement-blocking props, still subject to P3
    hanging_allowed: false    # ceiling-mounted props, subject to headroom (P22)
  gore: none                # none | light | heavy: corpses, blood, impaled bodies
  barrels:
    count: { min: 1, max: 1 }
    placement: scattered     # near_encounters | scattered | none
    chain_reactions: allowed       # allowed | avoided
    keep_clear_of: [player_start, key_pickup, secret_reward]   # player_start | key_pickup | secret_reward

# Pacing
pacing:
  encounter_beats: { min: 2, max: 4 }
  rest_areas: { min: 1, max: 2 }
  peak_position: 0.7         # where the hardest fight sits, as a fraction of progression
  opening_intensity: low     # low | medium | high

# Compatibility and metadata
compat:
  port: limit_removing       # vanilla_limits | limit_removing | boom | zdoom
  emit_mapinfo: false        # v1 emits no extra lumps; par_time needs this
  par_time_seconds: 120      # ignored unless emit_mapinfo is true
  automap:
    hide_secret_lines: true  # a secret door does not read as a door on the automap
    show_map_lines: auto

# Constraints and priorities
constraints:
  enforcement: target        # strict (ranges are hard limits) | target (ranges are goals)
  forbid: [archvile, crusher, dark_maze, insta_death_pit]  # species names, or: crusher | dark_maze | insta_death_pit
  inspirations:
    - "a short, welcoming first map"
  must_include:
    - "a visible armory across from the entry hall"
  priority:                  # highest first; resolves conflicts between everything above
    - progression_correctness
    - playable_balance
    - sector_budget
    - monster_counts
    - detail_level
    - play_time
---

## Overview

Entrada is the compiler's own hand-authored payoff map: a short UAC entry
sector that fans out into an armory, a central hub, a key room, a combat
yard, a locked vault, and an exit hall, with one hidden cache reachable from
the combat yard.

## Sequence of events

1. The player starts in the entry sector, facing the armory across the
   room.
2. Crossing into the armory clears a lone shotgun guy and picks up the
   shotgun.
3. A manual door leads into the hub, where a berserk pack sits in the open.
4. From the hub, a plain passage opens south into the key room, where an
   imp guards the blue keycard.
5. Back through the hub, a plain passage opens east into the combat yard,
   where an imp and a pinky demon guard the chaingun.
6. A misaligned pillar in the combat yard hides a hidden cache holding a
   second soulsphere.
7. The blue keycard opens the locked door into the vault, where a hell
   knight guards a green armor and a soulsphere.
8. The vault opens into the exit hall, where the rocket launcher waits
   beside the exit switch.

## Secrets

### Secret 1 — Combat yard cache
- Trigger: misaligned_texture   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->
- Reward: soulsphere (`sustain.powerups`, secret_only)
- Hint: a support pillar in the combat yard doesn't quite line up with its neighbors.

## Notes

Entrada is the drift-guarded fixture `tests/first_map.rs` compiles and
reassembles on every run; this spec exists to judge that same emitted map
through the conformance checker, not to describe a design goal independent
of it.
