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
    count: { min: 2, max: 6 }   # exit and lift switches plus floor-action ones (23/18)
    remote_allowed: true     # a switch may act on a distant sector — which a floor action's
                             # switch usually does: four floor trigger lines in five sit
                             # neither on their target nor beside it
  walkover_triggers:
    count: { min: 1, max: 4 }   # exit walkovers plus floor-action ones (38/119); a lift's
                                # walkover is not counted here (crustygen issue #53)

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
  boss: none                 # none | mastermind | <species> — mastermind is the spider_mastermind species
  ambush:
    deaf_ratio: 0.4          # fraction of monsters flagged deaf: wake on sight, not on sound
    teleport_ambushes: { min: 1, max: 3 }
  sound:
    propagation: contained   # open | contained | sealed
    block_sound_at: [key_doors, arena_entrances]   # key_doors | arena_entrances
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
  baseline: uv               # itytd | hntr | hmp | uv | nm — the skill the counts above describe
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
      forbid_in: [combat_arenas, secret_rewards]    # combat_arenas | secret_rewards
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
    keep_clear_of: [player_start, key_pickup, secret_reward]   # player_start | key_pickup | secret_reward

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
  forbid: [archvile, crusher, dark_maze, insta_death_pit]  # species names, or: crusher | dark_maze | insta_death_pit
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

Refinery Overrun sends the player through a disused UAC chemical refinery,
converted into a waystation and now overrun after a containment breach. The
map favors incidental skirmishes punctuated by ambushes, working outward from
a central hub to two key-locked spokes and a reactor core before looping back
to the exit.

## Sequence of events

1. The player starts in the entry hall — the map's hub — with the shotgun
   visible across the room.
2. Clearing the loading dock spoke of zombiemen and shotgun guys, the player
   recovers the blue keycard from a pinky's holding cell.
3. The blue door back in the hub opens onto the operations wing, where the
   chaingun waits behind a service window.
4. A walkover trigger partway down the operations corridor drops the player
   into an ambush of imps and cacodemons.
5. Beyond the ambush, the red skull key sits on a pedestal in the overlook
   room.
6. The red door in the hub opens onto the reactor spoke, where the super
   shotgun and a hell knight guard the lift controls.
7. A switch at the base of the reactor spoke calls the lift back to the hub,
   exposing the exit switch behind one last group of zombiemen.
8. The player throws the exit switch to leave the map.

## Secrets

### Secret 1 — Supply cache
- Trigger: misaligned_texture   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->
- Reward: berserk pack (`sustain.powerups`, secret_only)
- Hint: a support pillar in the loading dock doesn't quite line up with its neighbors.

### Secret 2 — Overlook ledge
- Trigger: walkover   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->
- Reward: rocket launcher (`arsenal.weapons`, secret_only)
- Hint: crossing the catwalk above the operations wing lowers a ledge, revealing the launcher.

### Secret 3 — Operations map room
- Trigger: hidden_switch   <!-- misaligned_texture | shootable | walkover | lift | hidden_switch -->
- Reward: computer map (`sustain.powerups`, secret_only)
- Hint: a wall-mounted panel near the reactor spoke doubles as a switch.

## Notes

Playtest with a pistol start to confirm the operations-corridor ambush doesn't outpace the chaingun pickup.
