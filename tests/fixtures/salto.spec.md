---
spec_version: 1

# Identity and target
identity:
  slot: MAP01
  title: "Salto"
  author: "Amir Masri"
  iwad: doom2                # doom2 | freedoom2
  outputs: [udmf, doom]      # udmf is authored; doom is produced by cwad convert
  seed: 20260828             # reproducibility; same seed + same IR = byte-identical TEXTMAP
  grid: 64                   # map units; all coordinates snap to this

# Players and starts
players:
  start_facing: north        # north | south | east | west, or degrees
  coop_starts: 0             # 0 disables coop; 4 is the conventional set
  dm_starts: 0                # 0 disables deathmatch
  coop_only_items: false     # place extra pickups flagged multiplayer-only

# Scale budget
scale:
  size: { width: 1500, height: 1200 }    # bounding box, map units
  rooms: { min: 4, max: 6 }
  sectors: { min: 12, max: 20 }
  linedefs: { min: 50, max: 200 }
  play_time_minutes: { min: 1, max: 3 }
  vertical_range: { min: -32, max: 32 }    # allowed floor heights; the map's span is max - min

# Progression
progression:
  shape: hub_and_spoke       # linear | hub_and_spoke | branching | gauntlet
  keys: []
  locked_doors: 0
  backtracking: light        # none | light | heavy
  exit:
    kind: normal             # normal | secret | both
    trigger: teleport        # switch | teleport | walkover
  lifts:
    count: { min: 0, max: 0 }
    trigger: both_ends       # walkover | switch | both_ends
    max_travel: 256          # largest floor delta a lift may span
  teleports:
    count: { min: 3, max: 3 }
  doors:
    speed: normal            # normal | fast
    default_behavior: repeatable   # repeatable | one_shot | stays_open
    lock_types: []            # must be a subset of progression.keys (P24)
  switches:
    count: { min: 0, max: 0 }
    remote_allowed: false    # a switch may act on a distant sector
  walkover_triggers:
    count: { min: 1, max: 1 }

# Architecture
architecture:
  room_shapes: [rectangular]  # rectangular | l_shaped | t_shaped | octagonal | irregular
  symmetry: axial            # organic | axial | radial | mixed
  openness: open             # tight | mixed | open
  corridor_ratio: 0.1        # fraction of floor area that is transit rather than space
  verticality: flat          # flat | moderate | strong
  inter_area_windows: false   # sightlines between areas that are not directly connected
  overlooks: { min: 0, max: 0 }   # elevated vantage points over another area
  landmarks: 4               # visually distinct anchors aiding navigation

# Combat
combat:
  encounter_style: ambush        # incidental | ambush | arena | corridor
  hitscanner_ratio: 0.0            # fraction of total monster count that is hitscan
  max_simultaneous: 3           # pressure ceiling: most monsters active at once
  monster_closets: 1
  boss: none                 # none | mastermind | <species> — mastermind is the spider_mastermind species
  ambush:
    deaf_ratio: 1.0          # fraction of monsters flagged deaf: wake on sight, not on sound
    teleport_ambushes: { min: 1, max: 1 }
  sound:
    propagation: open   # open | contained | sealed
    block_sound_at: []            # key_doors | arena_entrances
  block_monster_lines: false  # keep monsters in their region without a wall
  monsters:
    - { species: imp, min: 3, max: 3 }

# Weapons and ammo
arsenal:
  pistol_start: required_viable        # required_viable | not_required
  weapons:
    - { name: shotgun, placement: early }        # early | mid | late | secret_only
  ammo:
    budget: generous         # tight | balanced | generous
    ratio: 3.220              # placed ammo damage / total baseline monster HP; overrides budget
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
    - { name: soulsphere,      count: 0, placement: none }
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
    min: 96                    # floor and ceiling for every emitted light level (P19)
    max: 176
    contrast_step: 32          # the delta that counts as a deliberate light change (P21)
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
  floor: [FLOOR4_8, GATE3]         # auto (theme-derived) | explicit list of flat names
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
  peak_position: 0.6         # where the hardest fight sits, as a fraction of progression
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
    - "a small map whose whole vocabulary is the teleporter"
  must_include:
    - "a two-way teleport pair the player can ride in both directions"
    - "a monsters-only pad that empties a sealed closet into the arena"
    - "an exit room reachable by nothing but a one-shot teleporter"
  priority:                  # highest first; resolves conflicts between everything above
    - progression_correctness
    - playable_balance
    - sector_budget
    - monster_counts
    - detail_level
    - play_time
---

## Overview

Salto is the teleport toolchain's own hand-authored payoff map: a 512x512 hub
carrying two pads, a manual door north into a taller arena, an imp closet
joined to that arena by a plain passage, a vault the hub's island pad reaches,
and an exit room no portal touches at all.

Both pad placements and three of the compiler's four teleport specials appear.
Two repeatable any-thing pads (special 97) form a two-way pair — a
free-standing island in the hub and a wall recess in the vault, each one the
other's destination. A repeatable monsters-only pad (126) sits in the closet.
A one-shot any-thing pad (39) is recessed into the hub's south wall. The
monsters-only one-shot form (125) is the one shape salto leaves out.

## Sequence of events

1. The player starts in the hub at (256, 128) facing north: the arena's door
   is straight ahead in the north wall, the island pad is off to the
   northwest, and the one-shot pad is recessed into the south wall behind and
   to the right.
2. The island pad delivers to the vault's own wall pad, facing south into the
   vault, where a stimpack sits.
3. The vault's wall pad is the return leg of the same pair: crossing it
   delivers back onto the hub's island pad, facing east. The two pads are each
   other's destinations, which is what makes the pair rideable in both
   directions.
4. The hub's north door opens into the arena, where the shotgun sits just
   inside.
5. A plain passage runs east out of the arena into the imp closet, which holds
   three deaf imps and a monsters-only pad. An imp that walks onto that pad is
   delivered into the arena's north half; the player cannot trigger it.
6. The one-shot pad in the hub's south wall delivers into the exit room — a
   room with no portal and no other teleport destination — where a walkover
   line across the room's own north alcove ends the level.

## Notes

Salto is the drift-guarded teleport fixture: `tests/build_cli.rs` rebuilds it
and compares the bytes against the committed `maps/salto.wad`, and
`tests/check_conformance.rs` judges the emitted TEXTMAP against this spec.
Like `entrada.spec.md`, every derivable number here was hand-set to salto's
own compiled actuals rather than to a design goal held independent of them.

This spec was derived from `entrada.spec.md`; the keys that differ, and why:

- `scale.size` reset to 1500x1200 (entrada: 2100x900) — narrower, taller.
  Salto's emitted bounding box is 1472x1152: the wall pad recessed out of the
  hub's south wall reaches y = -64 and the arena's north wall reaches
  y = 1088, so entrada's 900-unit height budget fails on it.
- `scale.sectors` reset to 12..20 (entrada: 15..25). Salto emits 14:
  5 rooms + 1 door + 2 door alcoves + 1 passage + 1 walkover-exit alcove +
  4 teleport pads, which is under entrada's floor of 15.
- `scale.rooms` set to 4..6 (salto has five rooms, entrada eight) and
  `scale.play_time_minutes` to 1..3. Both are `NotDerivable` rows, restated
  for this map rather than left at entrada's values.
- `players.start_facing` set to `north`: salto's `player1_start` carries
  angle 90, and the row grades the emitted angle.
- `progression`, `combat.ambush`, `combat.monsters`, and `secrets` carry the
  values the map itself emits: no keys, no locked doors, a teleport exit, one
  walkover trigger (the exit line), three player-crossable pads, one
  monsters-only pad in a monster room, three deaf imps out of three monsters,
  and no secret sectors.
- `sustain.*` and `scenery.barrels.count` set to zero except the vault's one
  stimpack: salto places one weapon and one pickup and nothing else.
- `aesthetics.lighting.min`/`.max` widened to 96..176 (entrada: 160..160) —
  the closet is authored at 96 and the vault at 176, and both bounds grade the
  emitted extremes.
- `combat.hitscanner_ratio` set to 0.0 and `arsenal.ammo.ratio` to 3.220 —
  both `Info` rows, restated to salto's measured values: no hitscanner is
  placed, and the arena's shotgun is the map's only ammo against the closet's
  three imps.
- `flats.floor` lists `GATE3` alongside `FLOOR4_8`: the compiler floors every
  teleport pad with the theme's own pad flat.
- The remaining differences (`architecture`, `arsenal`, `pacing`, `compat`,
  `constraints`, and the prose) are ungraded descriptive keys restated for a
  five-room teleport map instead of entrada's eight-room one.
