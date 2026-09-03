# The layer-4 verifier and its CLI

`crustygen-check` is verification layer 4 (`docs/design.md` §8). It reads an
**assembled** map — a UDMF `TEXTMAP` lump out of a built PWAD, or a classic
Doom binary-format map group assembled and rendered to the same UDMF form
(see "Binary-format input", below) — and re-derives the playability
invariants from the geometry that actually shipped. That is what makes it
more than a restatement of layer 1: the compiler's own pre-checks
(`src/rules.rs`) run against the IR, before any coordinate exists, so a
compiler bug that satisfies them still produces a broken map. This stage
never sees the IR at all.

The distinction is not theoretical for this project.
`heights::visible_lower_side`/`visible_upper_side` are the single place the
compiler decides which side of a two-sided line draws a texture, and both the
pass that *fills* one and the rule that *requires* one call through them — so
a wrong answer there is wrong in both places and no test can see it
(`KNOWN-GAPS.md` records the hole). V-P8
re-derives that comparison from `r_segs.c` independently, which is the whole
point of a second layer.

## Inputs

Two entry points, one implementation.

**Library.** `crustygen::check::run(&UdmfMap, map_name, &Tables, Option<&Spec>)
-> CheckReport` (`src/check/mod.rs`). The map is `crustywad`'s parsed UDMF
value; `map_name` is the map lump's own name (used only by the
`identity.slot` conformance row); `spec` is optional, and `CheckReport::conformance`
is `Some` exactly when it was supplied.

**CLI.** `crustygen-check <wad> [--map NAME] [--spec FILE]`
(`src/bin/crustygen-check.rs`). It reads the WAD, selects a map group
(`--map NAME` by exact match, else the WAD's first), and loads it through
`crustygen::ingest::load_map`, which accepts either on-disk format: a UDMF
group's `TEXTMAP` lump parses directly, while a classic Doom binary group is
assembled via crustywad's `Map::assemble`, rendered to UDMF text, and
re-parsed — the same `UdmfMap` either way, so `check::run` never sees which
path ran. It then loads `Tables`, optionally parses `--spec` through
`Spec::from_markdown`, and calls the same `check::run`. See "Binary-format
input", below, for why the round trip is sound.

A `CheckReport` carries four things: the `findings` (defects and
observations), the optional `conformance` rows, the `tag_manifest` (one entry
per distinct nonzero tag, listing the sectors that carry it and the action
lines that reference it), and `stats` (sector/linedef/sidedef/vertex/thing
counts plus the number of sectors carrying the secret special).

## The reuse boundary

The verifier is only worth having if it is not the compiler wearing a
different hat. What it is allowed to reuse, and why:

- **`crate::tables`** — the sourced-constants authority. Both layers read the
  same `engine.toml`/`vocabulary.toml`, deliberately: a *constant* shared
  between compiler and checker is the project's single source of truth, and
  duplicating it would create two places to be wrong instead of one. What is
  never shared is the *derivation* that consumes it.
- **`crate::reach`** — the flood core, written as verifier-grade from the
  start for exactly this reuse (`src/reach.rs`'s module doc). `flood.rs`
  builds its own `ReachGraph` from a `Scene` and runs `reach::check` over it
  untouched. It does **not** call `reach::graph_from_compiled`, which reads
  `Ir`/`Compiled`.
- **`crate::spec`** — the frontmatter types only, as the *targets* conformance
  compares against. Nothing in `spec/` derives anything about a map.

Off-limits: **`compile/` and `rules.rs`**, in full. Those are the logic under
cross-examination; calling into them would make the verifier agree with the
compiler by construction. `src/check/invariants.rs`'s module doc states this
at the top of the file, and the five checks that have a compile-side
counterpart name it in their own doc comments as the function they
deliberately do not call: V-P8 (`heights::visible_lower_side`/
`visible_upper_side`), V-P13/P14 (`compile::tags::check_no_action_at_tag_zero`),
V-P3 and V-P4 (`rules.rs`'s `check_passage_width` and `check_door_clearance`),
and V-P24 (`rules.rs`'s own `check_key_lock_coherence`).

`crate::geom` is a near miss worth recording: it already has
`dist_to_segment`, but typed over `geom::Pt`, the grid **integer** coordinates
the compiler's footprints are built from. A `Boundary`'s endpoints are `f64`
copied verbatim out of `UdmfVertex`, and a thing's position is `f64` too, so
reusing the integer twin would mean rounding both first. `invariants.rs`
carries its own `dist_to_segment_f64` — same projection-and-clamp algorithm,
no `Pt` round trip.

## The Scene

`Scene::build` (`src/check/scene.rs`) is the one-time index every check reads.
It does three things, in order, each pushing its own `V-S` findings.

**Boundaries.** Every linedef is walked exactly once. Its cross-references —
`v1`, `v2`, `sidefront`, `sideback`, and each referenced sidedef's `sector` —
are validated against the map's own declaration counts rather than trusted,
and the `twosided` flag must agree with whether a `sideback` is present. A
linedef failing any of these contributes no boundary at all and raises exactly
one finding. A well-formed linedef becomes one `Boundary` per side, filed
under the sector that side faces: the front mirror runs `v1` → `v2`, the back
mirror `v2` → `v1`, so **each sector always sees its own edges in its own
winding**. A `Boundary` carries the linedef's flags read through sourced bit
values (`two_sided`, `blocking`, `upper_unpegged`, `lower_unpegged`), its
`special` and `args[0]` tag, its `neighbor` sector, whether this mirror is the
front one (`fronts_this`), and which sidedef it came from.

`Boundary::passable()` is `two_sided && !blocking` — a two-sided line can
still be solid (a fence you can see and shoot across but not walk through),
and `PIT_CheckLine` (pinned `p_map.c:214-217`) rejects `ML_BLOCKING` for any
non-missile before `P_LineOpening` or any door state is consulted.

**Closure.** Each sector's boundary must close: every endpoint's degree,
counted across that sector's own segments, is even. Endpoints are keyed by
their raw `f64::to_bits` pattern, not by value and not with an epsilon —
every coordinate here was copied verbatim from `map.vertices` and never
computed, so two endpoints at the "same" vertex are bit-identical, and
bit-equality sidesteps picking an arbitrary tolerance. The first odd-degree
vertex found reports the sector and stops.

**Things.** Each thing resolves to the **first** closed sector (declaration
order) whose boundary contains it, by even-odd containment — gated on
`closed`, since even-odd is only sound for a closed boundary. A thing inside
no closed sector is a `V-S` Error. A thing's `name` is resolved from its
`type_id` through the vocabulary's reverse map; an unresolvable id leaves
`name` as `None`, which is what makes it invisible to the name-driven checks
and why `check_thing_headroom` surfaces the fact once (below). A point sitting
*exactly* on a boundary edge is unspecified under even-odd; the compiler never
emits a thing there.

## The check catalog

Eighteen ids in nineteen rows — `V-P11` earns one row for doors and one for
lifts. `V-Pn` re-derives playability rule `Pn` from §7.3 (`V-P28` re-derives
three of them, P28–P30, since one resolution answers all three); `V-S` is the
structural/unclassifiable-input family. Every finding names a subject (sector,
linedef, or thing, by TEXTMAP declaration index — or the map as a whole) and
prints as `{id} {severity} {subject}: {message}`.

| Id | What it derives | Severity |
|---|---|---|
| `V-S` | **Structural:** a linedef whose `v1`/`v2`/`sidefront`/`sideback`/sidedef-`sector` reference is out of range; a `twosided` flag disagreeing with `sideback`'s presence; a sector boundary that does not close; a thing inside no closed sector. **Unknown vocabulary:** a thing whose `type_id` names nothing (`unknown thing type {n}`); a linedef special this checker does not model — the eight lift specials are modeled now (`V-P5` and the flood's lift edges), so they no longer draw it. | Error for the four structural cases; Warning for the two unknown-vocabulary cases |
| `V-P2` | A thing's sector has `ceiling - floor` at least the thing's required height: a monster species' own height, else a blocking/hanging prop's, else the player's height for the five start kinds. Pickups, keys and ammo pin no requirement. **No door-sector exemption** — a door sector's emitted heights are its *closed* state, and something standing in one is unplayable exactly as reported. | Error |
| `V-P3` | Every passable boundary is at least `2 × player radius` long. Door faces are exempt (their clear width is V-P4's business); one visit per linedef. | Error |
| `V-P4` | For each door sector — found structurally, as the `neighbor` of a `fronts_this` door-special boundary, i.e. the line's **back** sector, which is what `EV_DoDoor` operates on — `min(neighbor ceilings across its own door boundaries) − door_clearance_allowance − its own floor` is at least the player's height. Measures the *emitted* door floor, not `rules.rs`'s pre-layout `max(a.floor, b.floor)` proxy. | Error |
| `V-P5` | Every platform (a sector named by a lift line's tag) travels — its floor is above its lowest two-sided neighbor's, the floor `EV_DoPlat` sends it to — and can be called from that floor: some trigger line's activator sector (a use special's front sector; either side a walkover can be crossed from at rest, excluding a walkover with a dead-end pocket no deeper than the player's radius on either side, which `P_TryMove` lets no center into) stands at the low floor. A platform callable only from above is P5's trap; whether the region below still finishes is V-P7's verdict, so both findings are Warnings. | Warning |
| `V-P7` | The key-aware flood: no player 1 start; an extra player 1 start; a start in no sector; no exit line; more lock classes than a `KeyMask` holds; the map is unfinishable; a reachable `(sector, keys)` state can no longer reach an exit; a sector no walk reaches. | Error |
| `V-P8` | A one-sided line's front `texturemiddle` is not `"-"`; two sectors with differing floors give the **lower-floor** side a `texturebottom`; differing ceilings give the **higher-ceiling** side a `texturetop`. Re-derived from `r_segs.c`'s `R_StoreWallRange` (lines 570 and 589), not from the compile-side `visible_*_side` pair. | Error |
| `V-P9` | No sidedef carries a `scalex*`/`scaley*` UDMF extension — vanilla's renderer has no per-sidedef scaling, so their presence means a source-port-only effect. Named by the first linedef referencing the sidedef, else by the map (an orphan sidedef). | Error |
| `V-P11` (doors) | No door-special line carries `dontpegtop` or `dontpegbottom` on its own face. **A convention pin, not an engine rule** — `ML_DONTPEGBOTTOM` is inert on a typical door face, whose visible texture lives in the upper slot — which is why it is a Warning, the same downgrade §9 gives P10. Measured: 247 of 255 door-special lines in `DOOM2.WAD` carry neither. | Warning |
| `V-P11` (lifts) | No riser — a two-sided boundary of a platform whose neighbor's floor is below it — carries `dontpegbottom`: flag-clear anchors the lower to the platform's floor so it rides with it (`r_segs.c`); 96 % of corpus risers are flag-clear (`docs/measurements/lift-shapes-2026-08-29.md` §G). Rendering only. | Warning |
| `V-P13` | An action line's tag resolves to at least one sector (a dead action otherwise). **The four exit specials are exempt** — `G_ExitLevel`/`G_SecretExitLevel` are `void (void)` and neither the switch nor the walkover path ever looks a tag up, so an unresolved tag there was never going to be read. Symmetrically, a sector carrying a tag no action line references is a stale tag. | Error for an unresolvable action tag; Warning for a stale sector tag |
| `V-P14` | No action line carries tag 0. Tag 0 is not "no tag" — it is the tag every untagged sector already has, so one stray zero opens every door. | Error |
| `V-P15` | Every teleport line's tag resolves the way `EV_Teleport` resolves it — the first sector, in declaration order, that both carries the tag and holds a `teleport_dest` marker — to a sector holding **exactly one** marker, with the player's headroom and radius clearance at the marker. Clearance is measured against the destination's **non-passable** boundary segments only, the same rule V-P25 applies to a start. Judged once per linedef, from its front mirror: `EV_Teleport` returns immediately on a back-side crossing, so the back mirror triggers nothing to check. A tag-0 teleport line is V-P14's finding, not repeated here. | Error |
| `V-P19` | Every sector's `lightlevel` is inside `Tables::light_range()`. Unconditional, spec or no spec. | Error |
| `V-P20` | **Embedding:** no collectible (pickup, ammo, weapon, `backpack`, key, or one of the eight powerups) sits inside a blocking prop's radius. **Reachability:** every collectible's sector is one the V-P7 flood actually reached; runs only when the flood ran. | Error |
| `V-P24` | Every locked-door **class** present has at least one key of that color placed, and every placed key opens at least one door present. Class-level, because `26` is all an emitted linedef retains — it opens to either `blue_card` or `blue_skull`. Doors dedupe by `(door sector, class)`, so one physical door with two faces reports once. | Error |
| `V-P25` | Every player start clears its sector's **non-passable** walls by at least the player's radius (an open doorway cannot crush you against it); clears every other thing whose name resolves to a blocking prop on **both axes at once** by `prop.radius + player.radius` (`PIT_CheckThing`'s own axis-aligned `blockdist` box, not a circular distance); and no two starts of any kind are within telefrag distance (`2 × radius`) of each other. | Error |
| `V-P27` | Every sector holding a monster has at least one two-sided boundary, or is a teleport destination. A fully one-sided monster sector can never be woken by sight or sound and is never entered, so its monsters are scenery the player never meets. **Two-sided**, not passable: sound and sight both travel through a two-sided line the player cannot walk across (a window, a fence), so a blocking two-sided boundary is still a way in for the wake-up this rule is about. | Error |
| `V-P28` | Every floor target, resolved the way `EV_DoFloor` reads it (`check::floors`, the resolution the flood and the `lift::floor` recognizer share), is one of the three opening shapes — a drop wall, a reveal or a bridge — with a rider who is not stranded (P28/P29). A target driven by lines of two engine types, one raising to a texture height this checker does not resolve, a `LedgeLower`, and a dead, closing, mixed or neutral move each report by name; so, separately, does a target whose tag is driven by some other special. A third finding is rule **P30**'s: a target bordering another moving sector, reported only for a crustygen-emitted target and always as an Error, since the chain is a build defect rather than a shape this checker cannot read. **Severity otherwise turns on the specials, not the shape**: a target every one of whose lines carries one of the four specials this compiler writes (`Tables::floor_specials`) is a build defect and an Error, while the same shape under any other special is a map this checker merely cannot vouch for, and a Warning. A floor line at tag 0 is `V-P14`'s finding and one whose tag names no sector is `V-P13`'s; neither is repeated here. | Error on a crustygen-emitted target; Warning otherwise |

Severity is a discipline, not a mood. **Error** means the map (or the input)
is provably broken and the CLI exits 1. **Warning** means suspicious but not
proven: an authoring convention violated (V-P11), dead weight (a stale sector
tag), or — the two `V-S` warnings — an input this checker cannot interpret.
Nothing produces `Severity::Info` today; the variant exists on the type.

Each pass that reads a linedef-wide property visits it once, via
`fronts_this`, so a two-sided line does not report the identical defect twice.

## The flood's construction rules

`src/check/flood.rs` turns a `Scene` into a `reach::ReachGraph` and runs
`reach::check` over it. The two passability rules (the 24-unit step-up cap and
the `min(ceilings) − max(floors)` crossing window) belong to `reach.rs` and
are documented in `docs/reachability.md`; what this module owns is the
*translation*, and three rules govern it.

**Blocking lines are walls.** One edge per `fronts_this` boundary with a
resolved neighbor — but only after `Boundary::passable()`. A boundary that
fails it contributes **no edge at all**, before its special is even read.
`PIT_CheckLine` rejects `ML_BLOCKING` for any non-missile ahead of
`P_LineOpening`, door state notwithstanding, so an open door on a blocking
line still cannot be crossed. Once past that filter, `Tables::door_special()`
becomes a `Door { lock: None }` edge, a locked special becomes
`Door { lock: Some(class) }`, and anything else becomes `Open`.

**Walkover goals need crossability; switch goals are front-side only.** A
switch exit fires from `P_UseSpecialLine`, and vanilla triggers use-activated
specials only from a line's front side — so a switch exit's goal is the sector
whose mirror has `fronts_this`. A walkover exit fires from
`P_CrossSpecialLine`, which has no side gate (unlike special 97, which checks
`side == 1` inside `EV_Teleport` despite being walkover-triggered), so **both**
bordering sectors are goals — but only when the boundary is `passable()`,
since `P_CrossSpecialLine` runs from `P_TryMove`'s `spechit` bookkeeping,
which a rejected move never reaches. An uncrossable walkover exit is not a
goal from either side; it would never fire in the engine.

This is a strictly more conservative goal set than `graph_from_compiled`'s
"only the carved recess" convention. That builder knows, from the IR, which
side of the line was the host room and which the recess; a `TEXTMAP` alone
cannot recover that, so this module names both and accepts a goal set that can
only ever be too generous, never falsely unfinishable.

**Floor bits ride the same mask as the keys.** `floors::resolve_floors`
resolves every sector a recognized floor line names, and each target the
flood can model becomes one bit of the `KeyMask` above `ACTION_BIT_BASE`
(8) — the node carries `(bit, destination)`, so `Node::effective_floor`
stands that sector at its destination in every state whose bit is set, and
each trigger driving it ORs the bit in where the player fires it. Which
sector fires what is the engine's dispatch question, so the three forms
differ: a **use** form's bit goes on its line's front sector, a **gun**
form's on both sectors the line faces (`P_ShootSpecialLine` takes no `side`
argument and its caller passes none), and a **crossing** form's on the edge
built for the line itself, unioned in on arrival from either side. Bits are
handed out to the targets the flood models, in target order, so a declined
one costs no bit.

Every recognized form is modeled this way, one-shot and repeatable alike.
That is exact for the one-shot forms — the four this compiler emits are
S1/W1 — and partial for a repeatable one: a mask bit only accumulates, so an
SR floor that can be sent back down is modeled as moved once and never
returned.

**Five kinds of target get no bit**, stand at their rest floor, and earn a
`V-P7` Warning naming the sector and why: one driven by lines of more than
one engine type; one whose destination is a texture height this checker does
not resolve; one whose sector already carries an action; **a lowering target
holding a shootable thing that does not fit it** (the engine restores a
blocked floor and leaves the thinker running, so it retries every tic and
never arrives); and every target past the eighth, which is what fits above
the key classes. Leaving such a target at rest is the conservative reading —
the flood judges the map as if the action never fired — and the warning is
what keeps that silence from passing for a verdict. `flood.rs` states the
two ways the fourth is narrower than the engine.

**Key classes are interned by lock, not by key name** — a card and a skull of
one color share a class, because `EV_VerticalDoor` accepts either. Where
`graph_from_compiled` `assert!`s that the vocabulary fits a `KeyMask`, this
module reports a hard finding instead: it runs on arbitrary input.

**No vacuous pass.** `graph_from_compiled` returns `None` for a map with no
player 1 start or no exit, on the reasoning that "no exit" belongs to spec
conformance. This module has no such elsewhere: either is a hard `V-P7`
Error here. `check_adversarial.rs`'s
`removing_the_player_start_is_a_hard_error_not_a_vacuous_pass` pins it.

`run_flood` returns per-sector forward reachability when it ran, which is what
V-P20's reachability half consumes. `check_key_lock_coherence` (V-P24) is
independent of it and still finds defects on a map with no start or exit at
all.

## Conformance

Supplied a `Spec`, `conform::rows` (`src/check/conform.rs`) produces one row
per parameter in its fixed catalog — `parameter`, `target`, `actual`,
`verdict` — plus one row per spec monster species, one per placed species the
spec never names, and one per `sustain.powerups[]` entry. **This is not one
row per frontmatter parameter declared** — a parameter with no sourced
geometric definition, or nothing emitted to measure it against, is instead one
of the explicit `NotDerivable` rows below, not a silent omission, and several
frontmatter fields (`identity.title`/`.author`/`.iwad`/`.outputs`/`.seed`,
most of `combat`'s administrative fields, `progression.doors`'s
speed/behavior settings, and others) have no row at all: nothing this checker
does turns on their value. Unlike the rest of the module, all of these rows
but six re-derive no playability rule — each is a target-vs-actual
comparison — so the only sourcing burden is the ammo ratio's damage figures,
the two thing-flag bits (`MTF_AMBUSH` = 8; multiplayer-only = 16, which the
pinned source writes as a raw literal with no named constant), the
teleport specials the two pad counts read, and the four floor specials the
two trigger counts read. The exceptions are
`progression.exit.trigger`, which borrows the flood's teleport-only
predicate (see the `NotDerivable` discussion below); the three
`progression.lifts.*` rows, which read `check::plats`' engine-style plat
resolution — platforms and who can call them — rather than counting lift
lines; and `progression.floors` with `combat.monster_closets`, which read
`lift::floor::recognize`'s engine-style resolution of what each floor action
*does*, which is not a thing a line's special says.
`progression.lifts.trigger` grades only the platforms that rest at
the top, since a barrier or a pedestal is not a lift the player rides, and a
map with no such platform passes it vacuously with actual `no lifts`.

Two rows count a floor action's *trigger* rather than the action:
`progression.switches.count` counts floor use lines beside exit and lift
ones (a switch lowering a four-sector wall is one switch, so counting lines
is the right reading), and `progression.walkover_triggers.count` counts floor
walkovers beside the two exit walkovers. `progression.floors` is the row that
says what those triggers *drive* — drop walls, reveals and bridges by shape,
plus the refusals — and it is always `Info` with target `any`, because the
frontmatter has no floor parameter to grade against yet.
`combat.monster_closets` counts the pockets of monsters a map releases into
the fight over the two mechanisms this checker can re-derive: one a floor
action opens (a reveal whose cell holds a monster, or a drop wall with a
closed region of them behind it) and one staged behind a monsters-only
teleport pad. **The sealing test belongs to the floor half alone** — a drop
wall is an ordinary wall until something says the monsters past it are shut
in, while a monsters-only pad *is* the statement that its occupants arrive by
teleport, wherever they were standing.

Forty rows are fixed, plus one per spec monster species, one per
placed species the spec never names (always `Fail`, target `absent`), and one
per `sustain.powerups[]` entry. Entrada against its paired spec produces 53.

**Verdict discipline.** A range (`MinMax`) or exact-count or boolean target is
`Pass`/`Fail` — those are decidable. A **scalar continuous** target
(`combat.hitscanner_ratio`, `combat.ambush.deaf_ratio`, `arsenal.ammo.ratio`)
is always `Info`, its actual formatted `"<value> (target <t>, delta <d>)"`.
This is deliberate and load-bearing: judging a ratio Pass/Fail requires a
tolerance, and there is no sourced or measured tolerance to use. Inventing one
here would bake a number into the checker that the **resolver** — the stage
that will eventually trade parameters against `constraints.priority` — is the
right owner of. Reporting the delta and declining to grade it is the honest
form. A ratio row over a map with no monsters reads `"no monsters"`, still
`Info`, never `Fail`.

`Verdict::NotRun` is for a row whose prerequisite failed. `conform::rows`
itself never produces one; when the map's findings carry a geometry-
corrupting `"V-S"` `Error` (not every `"V-S"` `Error` — see "Failure
containment" below for exactly which), `check::run` calls
`conform::not_run_rows` instead. `tests/check_conformance.rs` asserts a
clean run carries none, and separately proves the structural-failure path
forces every row to `NotRun`.

**NotDerivable**, with the reason carried in `actual`. Six rows are always
undecidable:

| Row | Reason |
|---|---|
| `identity.grid` | `portal width/at are exempt from the grid rule, so a vertex-grid check false-positives on every opening` |
| `scale.rooms` | `rooms are an IR concept; emitted sectors include passages/doors/alcoves` |
| `scale.play_time_minutes` | `runtime property` |
| `combat.encounter_style` | `no sourced geometric definition exists` |
| `combat.sound.propagation` | `no sourced geometric definition exists` |
| `combat.max_simultaneous` | `runtime property` |

Five more become `NotDerivable` only when the map gives them nothing to
measure: `scale.size` (`no boundary geometry to measure`),
`scale.vertical_range` (`no sectors present`), `players.start_facing` (`no
player1_start placed`), `aesthetics.lighting.min` and `.max` (`no sectors
present`).

`progression.exit.trigger` is decidable for all three targets, including
`teleport`. A teleport exit emits exactly a plain walkover exit's specials
(`compile::exits`), so nothing on the line tells them apart — what does is
where the line sits. The row reads `teleport` when the map carries walkover
exit specials, no switch ones, and *every* sector holding a crossable
walkover exit line is teleport-only in `flood::teleport_only_sectors`'s
sense: the flood with teleport edges reaches it and the flood without them
does not. A map with no teleports, or one whose exit sector can also be
walked to, reads `walkover` — which is rule P26's own shape, measured rather
than assumed.

Two readings worth pinning, both recorded at their functions:

- **`scale.vertical_range` bounds each floor**, not the map's computed span.
  The field's own doc comment ("the allowed floor height range") is the
  authority; the template's "the map's span is max - min" describes what the
  allowed band spans, not an instruction to compare a span. A map with floors
  at −16 and 128 fails `min: 0, max: 256` under the implemented reading and
  would pass under the other, so the two are not interchangeable. `actual`
  reports the observed floor extremes, so a failure names which one is out of
  band.
- **`scale.size` passes iff both measured dimensions are ≤ the budget** — it
  is a budget, not a target to hit.

### The ammo ratio, as implemented

`arsenal.ammo.ratio` is placed ammo's damage capacity over total baseline
monster HP. The modeling decisions are choices, not derivations, so they are
stated rather than implied:

- **Pool rate.** Each of the four ammo pools takes the **maximum**
  `expected_damage_per_ammo` among the weapons drawing that pool that are
  either placed on the map or the pistol (always carried, so a bullets figure
  always exists even with no weapon thing placed). "Placed" is classified by
  having a `[weapons.damage.*]` entry, not by a hardcoded name list, so a
  weapon added to that table later joins automatically. A pool with no
  available weapon contributes zero — never `NaN`, never a fallback guess.
- **Units.** Every placed ammo pickup's `amount`, plus `backpack` count times
  the sourced backpack grant (credited to all four pools), plus the ammo a
  placed *weapon* grants on pickup.
- **Denominator.** The sum of `spawnhealth` over every placed monster. Zero
  reads `"no monsters"` rather than dividing by an invented baseline.

## Failure containment

`Scene::build` (`src/check/scene.rs`) produces `"V-S"` `Error` findings from
three distinct places, only two of which mean the `Scene` itself is data the
builder gave up on:

- **Reference validity** (`process_linedef`, `Subject::Linedef`) — a
  dangling `v1`/`v2`/`sidefront`/`sideback`/sidedef-`sector` index, or a
  `twosided` flag disagreeing with `sideback`'s presence. The offending
  linedef contributes no `Boundary` to either sector at all.
- **Closure** (`sector_is_closed`, `Subject::Sector`) — a sector boundary
  whose endpoints do not all have even degree. `sector.closed` stays
  `false`, so nothing (not even a legitimately-placed thing) can resolve
  into that sector by even-odd containment.
- **Misplaced things** (`resolve_things`, `Subject::Thing`) — a thing
  outside every closed sector. This one is different in kind: it names a
  single thing's own bad placement, not a hole in the geometry. Every other
  sector and boundary in the `Scene` is exactly as valid as if that thing
  did not exist.

**Conformance goes `NotRun`, not `Fail` or a wrong `Pass` — but only for the
first two.** When `findings` carries a `"V-S"` `Error` naming
`Subject::Linedef` or `Subject::Sector`, `check::run` calls
`conform::not_run_rows` instead of `conform::rows`: the identical row
catalog `rows()` would have produced — same `parameter`, same `target`, in
the same order — with every `verdict` forced to `Verdict::NotRun` and
`actual` reading `"scene failed structural validation"`. Judging a spec
against geometry the scene builder itself rejected would produce a verdict
that looks decided but is not; `NotRun` says plainly that it was never
computed.

A `"V-S"` `Error` naming `Subject::Thing` does **not** trip this — deliberately,
not an oversight. Every conformance row's counts and geometry measurements
read `scene.sectors`/`scene.things` directly, and `Scene::build` never
shrinks either vector: a thing that fails to resolve just carries
`sector: None`, and rows that count things by name (`sustain.health.*`,
`players.coop_starts`, and the like) still count it correctly regardless.
The misplaced thing already has its own `"V-S"` finding telling that story;
forcing every *other* row to `NotRun` over it would be over-cautious, not
honest. Nor do the two `"V-S"` *Warning* cases (unrecognized vocabulary)
trip it — filtered out by `Severity::Error` alone, since those describe a
fully-formed scene the checker merely cannot name a finding's vocabulary
for, not a corrupt one.

**The flood cascades pessimistically, on purpose.** A linedef that fails
`Scene::build`'s validation contributes no boundary to either sector, and
`flood.rs` builds one `reach::Edge` per `fronts_this` boundary — so a dropped
linedef is a wall to the flood, not a hole cut for it to ignore. A `"V-S"`
reference error on a linedef that would otherwise have joined two rooms can
therefore cascade into a `V-P7` "never reached" finding on the far side of it.
This is intentional, the same conservative posture `Boundary::passable()`
already takes for a flagged-blocking line: the flood has no notion of "trust
this edge anyway" for a cross-reference the scene itself gave up on, so
treating it as impassable is the reading that can only ever be too
pessimistic, never falsely reachable.

## The CLI contract

```
usage: crustygen-check <wad> [--map NAME] [--spec FILE]
```

Output, in order: every finding, one per line, as
`{check} {severity} {subject}: {message}`; then, when `--spec` was given,
every conformance row as `{parameter}: {verdict} (target {target}, actual
{actual})`, verdicts rendered `pass`/`fail`/`info`/`not-derivable`/`not-run`;
then a one-line summary:

```
0 blocking, 0 warning(s), 53 conformance row(s), 3 tag(s)
```

Exit codes:

| Code | Meaning |
|---|---|
| 0 | No finding carried `Severity::Error` |
| 1 | At least one `Error` finding |
| 2 | Usage, I/O, or parse failure — bad flag, missing `<wad>`, unreadable file, not a WAD, no such map group, unassemblable binary map, a binary map strict UDMF rendering refuses (e.g. a frontless linedef), unsupported binary format (Hexen, Doom 64), non-UTF-8 `TEXTMAP`, unloadable tables, or an unparseable `--spec`. Every one names what failed on stderr. |

**Warnings do not change the exit code, and neither do conformance verdicts.**
A map with ten `Fail` rows and no `Error` finding exits 0: the rows are a
report, and grading a spec violation as a build failure is
`constraints.enforcement`'s decision to make (§9), not this binary's.

### Binary-format input (issue #21)

`crustygen-check` accepts a classic Doom binary-format map group, not just a
UDMF one. `crustygen::ingest::load_map` assembles it via crustywad's
`Map::assemble`, renders the result to UDMF text (`write_udmf`), and
re-parses that text into the same `UdmfMap` type the checker always
consumes — no check in the catalog above runs any differently. The round
trip is sound because a binary Doom-format map already carries
doom-namespace semantics: the special and tag numbering every check here
models. Hexen and Doom 64 groups are refused by name
(`IngestError::UnsupportedBinaryFormat`) — Hexen-style specials are a
different numbering the checks do not model.

Checking vanilla retail content this way can legitimately surface documented
authoring-convention warnings, which are expected, not defects: V-P11 flags
a door face carrying an unpegged flag, and a small minority of `DOOM2.WAD`'s
own door-special lines do — the V-P11 door row (above) already measures it, at
247 of 255 carrying neither — and the V-P11 lift row flags a lower-unpegged
platform riser, which about 4 % of retail and idgames risers are (§G of the
lift measurement). A binary-sourced map drawing either warning is the checker
doing its job on real content, not a false positive.

## What the verifier deliberately does not check

- **Specials this checker does not model.** A linedef special outside
  `check_recognized_specials`'s set draws a `V-S` warning saying exactly
  that — and that the flood cannot vouch for its effect on traversal —
  instead of a silent pass: recognizing a special without understanding it
  would make the flood optimistic, letting it call a map finishable that a
  player diverted or blocked by that line could not finish. The eight lift
  specials sat in that unmodeled set until the flood learned to ride a
  platform; they are recognized now (`plats::resolve_plats`,
  `EdgeKind::Lift`, and V-P5 alongside V-P11's lift row), so a lift line no
  longer draws the warning. The **forty-eight** floor specials left it the
  same way: `Tables::recognized_floor_specials` is the whole dispatch table,
  not the four this compiler emits, because `check::floors` resolves any of
  them and the flood carries the modelable ones as mask bits. The set the
  checker *recognizes* is therefore wider than the set it *emits*, and wider
  again than the set the `lift::floor` recognizer *accepts* — three different
  questions, deliberately.

  **Teleports are modeled the same way, and the contrast with an unmodeled
  special is the rule.** All four teleport specials (97/39/126/125) *are* in
  the recognized set, because the flood does model them. What is modeled: a
  `fronts_this` boundary carrying either **player** teleport special
  contributes one **directed** `EdgeKind::Teleport` edge from its own sector
  to the sector its tag resolves to. Front side only — `EV_Teleport` returns
  before doing anything when `side == 1`, so the back mirror of the same
  linedef builds no edge. Engine-style resolution — the destination is the
  first sector, in declaration order, that both carries the tag and holds a
  `teleport_dest` marker; a tag matching sectors that hold no marker resolves
  past them, and a tag matching none at all yields no edge, because the line
  fires nothing (which is V-P15's finding, not the flood's). Directed, because
  a teleport relocates the player rather than opening a way back. The edge is
  gated on `Boundary::passable`, exactly as the walkover exits are. V-P15 then
  checks that every teleport line's tag pairs with exactly one marker that the
  player fits at, and V-P27 that no monster sector is sealed away from both
  sight and a teleport arrival.

  What is **not** modeled: the monsters-only specials contribute no edge at
  all, since they move no player. There is no acoustic or line-of-sight
  model — whether a closet's monsters actually wake is a runtime behavior,
  and the corpus measurement's audibility figures are a statistic, not a
  rule (`docs/measurements/teleports-2026-08-28.md`). And V-P15 sizes
  headroom and clearance for the **player** even on a monsters-only line,
  which is optimistic in one direction: a species wider than the player can
  arrive where this check calls the destination clear and land embedded in
  the wall. The engine does not refuse that arrival — `P_TeleportMove`
  (pinned `p_map.c`) sets `tmbbox`, takes floor and ceiling from
  `R_PointInSubsector`, runs `PIT_StompThing` over *things* only, links the
  thing and returns true, consulting no line; its one false return is
  `PIT_StompThing` refusing a non-player stomp. The mobj is simply stuck
  afterwards, because `PIT_CheckLine` fails every later `P_TryMove` whose
  box still straddles the one-sided line. Sizing it properly needs the set
  of species that can reach the trigger line — the acoustic model this
  checker does not have.
- **Sector specials, liquids included — and these *do* pass silently.** The
  warning above is a *linedef*-special check: `check_recognized_specials`
  reads `Boundary::special`, and a damaging floor is a **sector** special
  (`data/engine.toml`'s `[sector.damage]`, a numerically distinct space, as
  that table's own neighboring comment spells out).
  `SceneSector::special` is populated by `Scene::build` but read nowhere in
  `src/check/` — the secret-sector count in `MapStats` reads the raw
  `UdmfMap`'s own `sector.special` directly (`check/mod.rs`), not the
  `Scene`'s copy of it. So a nukage sector this checker has never heard of
  draws neither a finding nor a warning. That is honest only while the
  compiler emits no liquids; the day it does, sector specials need their own
  recognized set, and P16/P17 need V- ids.
- **Sky.** V-P8 has no sky exception: `r_segs.c` legitimately needs no upper
  between two sky ceilings, but crustygen emits no sky flat, so the branch is
  unwritten rather than guessed at.
- **Door track pegging.** The IR's `Portal::track_lower_unpegged` opt-out
  governs a door's *track* sidedefs, and a track is not a concept a
  `TEXTMAP` names — a `Boundary` cannot even address it. V-P11 judges door
  **faces** only, and does so because faces are all it can see, not because
  the track was excluded by an explicit skip.
- **P24's authored-intent form.** `rules.rs`'s IR-side `check_key_lock_coherence`
  compares the authored lock string (`"blue_card"`) against placed thing kinds
  by exact equality — it polices what the author *said*. V-P24 cannot: an
  emitted `26` names a color class, not a card. The two rules deliberately
  disagree on a map that locks `"blue_card"` and places a `blue_skull`
  (`KNOWN-GAPS.md` records why neither should be "fixed" into agreement).
- **The rest of the catalog.** P1 (retired), P6, P10, P12, P16, P17, P18,
  P21, P22, P23 have no `V-` id — the set the compiler leaves uncovered, less
  P20, which the verifier does cover. P26 is the one compiler-side rule with
  no `V-` id: a teleport exit emits exactly a plain walkover exit's specials,
  so nothing on the line distinguishes them, and the verifier grades P26's
  shape as the `progression.exit.trigger` conformance row instead of raising
  a finding. P18 is the other near miss: its counting
  rule ("emitted secret sectors equal `secrets.count`") lives here as the
  `secrets.count` conformance row rather than as a finding, so it needs a spec
  and it grades rather than fails.
- **A wide opening split across collinear linedefs.** V-P3 measures one
  boundary segment at a time. This compiler never tiles an opening that way,
  so the check is sound today; a future one that did would false-positive.
  Recorded rather than guarded against.

## Testing

- `src/check/*.rs`'s own unit tests write `TEXTMAP` text by hand, parse it
  through `parse_udmf`, and check the resulting map — the same path the CLI
  takes, so a fixture cannot accidentally bypass the parser.
- `tests/check_adversarial.rs` is the layer-4 proof: compile entrada clean,
  break **exactly one** property on the parsed map — or, for V-P9, on the
  emitted text before parsing: a scale factor has no field to set
  (`UdmfSidedef`/`UdmfAssignment` are `#[non_exhaustive]` and cannot be
  constructed outside crustywad), so it is spliced into the `TEXTMAP` and
  re-parsed, which is also how a ZDoom-aware editor re-saving the file would
  introduce one — then assert the specific finding. Each test re-establishes
  the zero baseline for its check id before
  mutating, so a pass proves the mutation caused the finding rather than the
  property already being broken. It includes the historical −32 `key_room`
  pit — the map that actually shipped unfinishable — caught as stranding.
- `tests/check_conformance.rs` runs the rows end to end against entrada and
  its hand-paired spec, whose derivable numbers were set to entrada's own
  actuals, so a clean run must show zero `Fail` rows. It also proves failure
  containment end to end: a dangling sidedef->sector index against a real
  spec must produce a `"V-S"` `Error` and a conformance row list that is
  entirely `NotRun`, with the same parameter list the healthy run produces.
- `tests/check_cli.rs` covers the three exit codes and the output shape.
