---
spec_version: 1

# Identity and target
identity:
  slot: MAP07
  title: "The Gauntlet"
  author: "Amir Masri"
  iwad: doom2
  outputs: [udmf, doom]
  seed: 20260811
  grid: 64

# Players and starts
players:
  start_facing: 135          # degrees, not a compass point — deliberately off-axis
  coop_starts: 4
  dm_starts: 4
  coop_only_items: false

# Scale budget
scale:
  size: { width: 3072, height: 1024 }
  rooms: { min: 6, max: 9 }
  sectors: { min: 30, max: 70 }
  linedefs: { min: 150, max: 400 }
  play_time_minutes: { min: 4, max: 7 }
  vertical_range: { min: 0, max: 192 }

# Progression
progression:
  shape: gauntlet
  keys: [yellow_skull]
  locked_doors: 1
  backtracking: none
  exit:
    kind: secret
    trigger: walkover
  lifts:
    count: { min: 0, max: 1 }
    trigger: switch
    max_travel: 128
  teleports:
    count: { min: 0, max: 0 }
  doors:
    speed: fast
    default_behavior: one_shot
    lock_types: [yellow_skull]
  switches:
    count: { min: 1, max: 3 }
    remote_allowed: false
  walkover_triggers:
    count: { min: 1, max: 2 }

# Architecture
architecture:
  room_shapes: [l_shaped, t_shaped]
  symmetry: axial
  openness: tight
  corridor_ratio: 0.55
  verticality: strong
  inter_area_windows: false
  overlooks: { min: 0, max: 1 }
  landmarks: 0

# Combat
combat:
  encounter_style: corridor
  hitscanner_ratio: 0.5
  max_simultaneous: 8
  monster_closets: 1
  boss: cyberdemon
  ambush:
    deaf_ratio: 0.2
    teleport_ambushes: { min: 0, max: 1 }
  sound:
    propagation: sealed
    block_sound_at: [key_doors]
  block_monster_lines: true
  monsters:
    - { species: zombieman,   min: 6,  max: 10 }
    - { species: chaingunner, min: 2,  max: 4 }
    - { species: revenant,    min: 1,  max: 3 }
    - { species: cyberdemon,  min: 1,  max: 1 }

# Weapons and ammo
arsenal:
  pistol_start: not_required
  weapons:
    - { name: chaingun,        placement: early }
    - { name: rocket_launcher, placement: mid }
    - { name: plasma_rifle,    placement: late }
  ammo:
    budget: tight
    ratio: 1.5
    distribution: front_loaded
    pickups: { shells: 8, box_of_rockets: 2 }
    backpack: { count: 1, placement: early }

# Health, armor, powerups
sustain:
  health_budget: tight
  health:
    stimpack: 4
    medikit: 2
    health_bonus: 10
  armor:
    green: 1
    blue: 0
    armor_bonus: 5
  powerups:
    - { name: berserk,         count: 0, placement: none }
    - { name: soulsphere,      count: 0, placement: none }
    - { name: megasphere,      count: 1, placement: late }
    - { name: radsuit,         count: 0, placement: none }
    - { name: invulnerability, count: 1, placement: secret_only }
    - { name: invisibility,    count: 0, placement: none }
    - { name: light_amp,       count: 0, placement: none }
    - { name: computer_map,    count: 0, placement: none }

# Secrets
secrets:
  count: 1                   # per-secret detail lives in the prose body

# Difficulty
difficulty:
  skills_supported: false
  baseline: hmp
  curve: steep
  scaling: { easy: 0.6, medium: 0.8, hard: 1.0 }

# Aesthetics
aesthetics:
  theme: tech_base
  texture_set: [STARTAN3]
  detail_level: 2
  lighting:
    style: pools_of_dark
    base: 96
    min: 64
    max: 160
    contrast_step: 32
    corridor_delta: -16
    outdoor: 128
    effects:
      allowed: [strobe_slow]
      density: sparse
      forbid_in: [combat_arenas]
    per_room_overrides: false
  sky: auto
  music: auto
  texture_scaling: forbidden

# Flats and liquids
flats:
  floor: [FLOOR4_8]
  ceiling: [CEIL3_5]
  outdoor_proportion: 0.05
  light_flats: false
  liquid:
    kind: none
    damaging: false
    damage_tier: light
    coverage: 0.0
    crossing_required: false
    radsuit_provided: false

# Vertical form
vertical:
  stairs:
    flights: { min: 0, max: 1 }
    rise_per_step: 16
    tread_depth: 32
  standard_ceiling: 112
  door_opening: 96

# Scenery: decoration, light sources, hazards
scenery:
  light_sources:
    density: sparse
    kinds: [techno_lamp]
    match_lighting: false
  decorations:
    density: none
    kinds: auto
    blocking_allowed: false
    hanging_allowed: false
  gore: heavy
  barrels:
    count: { min: 0, max: 2 }
    placement: scattered
    chain_reactions: avoided
    keep_clear_of: [key_pickup, secret_reward, player_start]

# Pacing
pacing:
  encounter_beats: { min: 3, max: 5 }
  rest_areas: { min: 1, max: 2 }
  peak_position: 0.6
  opening_intensity: high

# Compatibility and metadata
compat:
  port: boom
  emit_mapinfo: false
  par_time_seconds: 180
  automap:
    hide_secret_lines: true
    show_map_lines: false

# Constraints and priorities
constraints:
  enforcement: strict
  forbid: [archvile, crusher]
  inspirations:
    - "tight sightlines like Doom II MAP08"
  must_include:
    - "a cyberdemon boss fight visible from the entrance"
  priority:
    - playable_balance
    - progression_correctness
    - monster_counts
    - detail_level
    - sector_budget
    - play_time
---

## Overview

The Gauntlet is a short, brutal corridor run through a collapsed tech-base
annex. There is no hub and no backtracking: the player advances through an
L-shaped access corridor into a T-shaped chamber, fighting through
increasingly dangerous encounters toward a cyberdemon guarding the level's
only way out.

## Sequence of events

1. The player starts at the annex entrance, facing the corridor at an angle
   rather than square down its length.
2. The yellow skull key sits past the first ambush, visible from the
   entrance but locked behind it.
3. The yellow-locked door opens onto the T-shaped chamber, where the
   cyberdemon waits across an open sightline.
4. A walkover line at the chamber's far wall triggers the secret exit once
   the cyberdemon is down.

## Secrets

### Secret 1 — Sealed alcove
- Trigger: shootable   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->
- Reward: invulnerability sphere (`sustain.powerups`, secret_only)
- Hint: a cracked switch panel beside the first ambush responds to gunfire.

## Notes

Pistol start is explicitly not required for this map; playtest with carried
weapons from the previous map to confirm the cyberdemon fight is fair with
only a chaingun and a rocket launcher.
