---
spec_version: 1

# Identity and target
identity:
  slot: MAP01
  title: "Muralla"
  author: "Amir Masri"
  iwad: doom2                # doom2 | freedoom2
  outputs: [udmf, doom]      # udmf is authored; doom is produced by cwad convert
  seed: 20260902             # reproducibility; same seed + same IR = byte-identical TEXTMAP
  grid: 64                   # map units; all coordinates snap to this

# Players and starts
players:
  start_facing: north        # north | south | east | west, or degrees
  coop_starts: 0             # 0 disables coop; 4 is the conventional set
  dm_starts: 0                # 0 disables deathmatch
  coop_only_items: false     # place extra pickups flagged multiplayer-only

# Scale budget
scale:
  size: { width: 2240, height: 832 }     # bounding box, map units
  rooms: { min: 5, max: 7 }
  sectors: { min: 12, max: 20 }
  linedefs: { min: 50, max: 200 }
  play_time_minutes: { min: 1, max: 3 }
  vertical_range: { min: -96, max: 192 }  # allowed floor heights; the map's span is max - min

# Progression
progression:
  shape: linear              # linear | hub_and_spoke | branching | gauntlet
  keys: [red_card]
  locked_doors: 1
  backtracking: none         # none | light | heavy
  exit:
    kind: normal             # normal | secret | both
    trigger: switch          # switch | teleport | walkover
  lifts:
    count: { min: 0, max: 0 }
    trigger: switch          # walkover | switch | both_ends
    max_travel: 256          # largest floor delta a lift may span
  teleports:
    count: { min: 0, max: 0 }
  doors:
    speed: normal            # normal | fast
    default_behavior: repeatable   # repeatable | one_shot | stays_open
    lock_types: [red_card]    # must be a subset of progression.keys (P24)
  switches:
    count: { min: 2, max: 2 }
    remote_allowed: true    # a switch may act on a distant sector
  walkover_triggers:
    count: { min: 3, max: 3 }

# Architecture
architecture:
  room_shapes: [rectangular]  # rectangular | l_shaped | t_shaped | octagonal | irregular
  symmetry: mixed            # organic | axial | radial | mixed
  openness: open             # tight | mixed | open
  corridor_ratio: 0.1        # fraction of floor area that is transit rather than space
  verticality: moderate      # flat | moderate | strong
  inter_area_windows: false   # sightlines between areas that are not directly connected
  overlooks: { min: 0, max: 0 }   # elevated vantage points over another area
  landmarks: 3               # visually distinct anchors aiding navigation

# Combat
combat:
  encounter_style: ambush        # incidental | ambush | arena | corridor
  hitscanner_ratio: 0.0            # fraction of total monster count that is hitscan
  max_simultaneous: 3           # pressure ceiling: most monsters active at once
  monster_closets: 1
  boss: none                 # none | mastermind | <species> — mastermind is the spider_mastermind species
  ambush:
    deaf_ratio: 0.0          # fraction of monsters flagged deaf: wake on sight, not on sound
    teleport_ambushes: { min: 0, max: 0 }
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
    budget: balanced         # tight | balanced | generous
    ratio: 4.831              # placed ammo damage / total baseline monster HP; overrides budget
    distribution: front_loaded   # front_loaded | even | back_loaded
    pickups: auto            # auto (derived from ratio) | explicit counts per pickup type
    backpack: { count: 0, placement: none }

# Health, armor, powerups
sustain:
  health_budget: tight       # tight | balanced | generous; explicit counts below override it
  health:
    stimpack: 0
    medikit: 1
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
    min: 128                   # floor and ceiling for every emitted light level (P19)
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
  encounter_beats: { min: 2, max: 3 }
  rest_areas: { min: 1, max: 2 }
  peak_position: 0.25        # where the hardest fight sits, as a fraction of progression
  opening_intensity: medium  # low | medium | high

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
    - "a small map whose whole vocabulary is the floor that gets out of the way"
  must_include:
    - "a wall the player drops with a switch, with a monster closet behind it"
    - "a pedestal that lowers as the player walks into the room holding it"
    - "a bridge that rises under the player who steps down into its pit"
  priority:                  # highest first; resolves conflicts between everything above
    - progression_correctness
    - playable_balance
    - sector_budget
    - monster_counts
    - detail_level
    - play_time
---

## Overview

Muralla is the floor toolchain's own hand-authored payoff map, the counterpart
to ascensor: where ascensor's every floor rises and falls under the player,
muralla's floors get *out of the way*. All three floor actions the compiler
emits appear once — a drop wall, a reveal, a bridge — and each is driven by a
different trigger.

Five rooms. `entry` is the start; `closet` is sealed behind the drop wall,
which the switch on entry's west wall lowers; `hall` holds the pedestal reveal
carrying the red card, which the walkover on the entry-to-hall opening lowers;
`yard` is across the bridge, whose pit rises under whoever steps down into it;
and `exit` is behind the red-card door, with the exit switch on its east wall.
The map is a chain with one pocket: the closet is the only room the player
does not have to pass through.

## Sequence of events

1. The player starts in `entry` at (128, 128) facing north, with a shotgun off
   to the north-east at (256, 384).
2. Entry's west wall carries a switch at (0, 256). It lowers the wall filling
   the gap in entry's east wall — 16 units thick, resting solid at 192, which
   is the ceiling both `entry` and `closet` share — to the lowest floor
   around it. The wall drops flush and the closet's two imps come out.
3. The plain opening at (512, 416) leads north-east into `hall`. Crossing it
   fires the walkover that lowers the pedestal at (768, 512): a 64x64 cell
   resting 64 above hall's floor with the red card on top, called down to the
   floor so the card can be taken.
4. Hall's east wall opens onto the bridge: a 64-deep pit sunk to -96 between
   `hall` and `yard`. Both of its thresholds carry the same walkover, so
   stepping down into it from either side raises the floor under the player to
   the rooms' own 0.
5. `yard` holds the third imp and a medikit. Its east wall is the red-card
   door — a 32-thick door sector between two 16-deep key-trimmed alcoves.
6. Past it, `exit`'s east wall carries the switch at (2240, 576) that ends the
   level.

## Notes

Muralla is the drift-guarded floor fixture: `tests/build_cli.rs` rebuilds it
and compares the bytes against the committed `maps/muralla.wad`,
`tests/check_conformance.rs` judges the emitted TEXTMAP against this spec, and
`tests/check_adversarial.rs` cross-examines the built map for unmodeled
specials, softlocks and misshapen floor actions. Like `ascensor.spec.md`,
every derivable number here was set from muralla's own compiled output rather
than from a design goal held independent of it — as the exact value where the
row grades one, and as a bound that contains the measured value where the row
grades a range.

**No row is left failing.** Every row this spec produces is `Pass`, `Info` or
`NotDerivable`; `tests/check_conformance.rs` asserts exactly that, plus the
census the `progression.floors` row reports: `drop walls ×1, reveals ×1,
bridges ×1, refused ×0`.

The map has **no secrets** — `secrets.count` is 0 and there is no prose
`Secrets` section to disagree with it.

This spec was derived from `ascensor.spec.md`; the keys that differ, and why:

- `identity` names muralla and its own seed, 20260902 — the seed the fixture
  carries, so the byte-identity guard and the spec agree on it.
- `scale.size` set to 2240x832, muralla's emitted bounding box exactly. The
  row is a budget (`Pass` iff both dimensions are no larger than the target),
  so the measured box is the tightest honest target rather than a slack one.
- `scale.sectors` left at ascensor's 12..20. Muralla emits 14: 5 rooms + 1
  plain passage + 1 door sector + 2 door alcoves + 1 drop wall + the 2
  approach passages that flank it + 1 bridge pit + 1 pedestal reveal cell.
- `scale.linedefs` left at 50..200, which contains the 64 emitted.
- `scale.vertical_range` reset to -96..192 (ascensor: 0..128). The row bounds
  each individual floor, not the span: the bridge's pit rests at -96 and the
  drop wall at 192, and every other floor is 0 or the pedestal's 64.
- `scale.rooms` left at 5..7; muralla has five rooms. A `NotDerivable` row,
  restated for this map.
- `progression.keys`, `locked_doors` and `doors.lock_types` carry the red card
  and its one door. The card is the reveal's cargo rather than a room thing —
  it is picked up off the pedestal cell once that cell has come down.
- `progression.lifts.count` and `progression.teleports.count` are 0..0:
  muralla emits no platform and no pad. With no lift present,
  `lifts.trigger` and `lifts.max_travel` grade as "no lifts" and are left at
  ascensor's words rather than given a meaning this map does not have.
- `progression.switches.count` is 2..2: the wall switch in entry and the exit
  switch in `exit`. Nothing else on this map is used rather than crossed.
- `progression.walkover_triggers.count` is 3..3: the entry-to-hall opening
  that lowers the pedestal, plus **both** thresholds of the bridge's pit —
  the compiler writes the rise on each, so the player fires it from whichever
  side they step down.
- `progression.shape` is `linear` and `backtracking` `none`: the rooms form
  one chain, and the closet is a pocket off it rather than a branch to return
  from.
- `combat.monster_closets` is 1 — the closet behind the drop wall: a region
  that is sealed until the wall drops and holds monsters, which is what
  `conform::floor_closets` counts. `combat.monsters` is three imps, two in
  that closet and one in the yard; `encounter_style` is `ambush` for the same
  reason.
- `combat.hitscanner_ratio`, `combat.ambush.deaf_ratio` and
  `arsenal.ammo.ratio` are `Info` rows, restated to muralla's measured values
  — 0.000, 0.000 and 4.831: no hitscanner is placed, no monster is flagged
  deaf, and a shotgun plus one box of shells is the map's ammo against three
  imps.
- `sustain` drops ascensor's stimpack and soulsphere for the single medikit in
  the yard; every other count is 0.
- `aesthetics.lighting.min` set to 128 (ascensor: 144) — the closet is
  authored at 128, the dimmest sector on the map, and `max` stays 176 for the
  yard.
- The remaining differences (`architecture`, `pacing`, `constraints` and the
  prose) are ungraded descriptive keys restated for a five-room floor map
  instead of ascensor's six-room lift one.
