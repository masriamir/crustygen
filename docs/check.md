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

Fourteen ids. `V-Pn` re-derives playability rule `Pn` from §7.3; `V-S` is the
structural/unclassifiable-input family. Every finding names a subject (sector,
linedef, or thing, by TEXTMAP declaration index — or the map as a whole) and
prints as `{id} {severity} {subject}: {message}`.

| Id | What it derives | Severity |
|---|---|---|
| `V-S` | **Structural:** a linedef whose `v1`/`v2`/`sidefront`/`sideback`/sidedef-`sector` reference is out of range; a `twosided` flag disagreeing with `sideback`'s presence; a sector boundary that does not close; a thing inside no closed sector. **Unknown vocabulary:** a thing whose `type_id` names nothing (`unknown thing type {n}`); a linedef special this checker does not model. | Error for the four structural cases; Warning for the two unknown-vocabulary cases |
| `V-P2` | A thing's sector has `ceiling - floor` at least the thing's required height: a monster species' own height, else a blocking/hanging prop's, else the player's height for the five start kinds. Pickups, keys and ammo pin no requirement. **No door-sector exemption** — a door sector's emitted heights are its *closed* state, and something standing in one is unplayable exactly as reported. | Error |
| `V-P3` | Every passable boundary is at least `2 × player radius` long. Door faces are exempt (their clear width is V-P4's business); one visit per linedef. | Error |
| `V-P4` | For each door sector — found structurally, as the `neighbor` of a `fronts_this` door-special boundary, i.e. the line's **back** sector, which is what `EV_DoDoor` operates on — `min(neighbor ceilings across its own door boundaries) − door_clearance_allowance − its own floor` is at least the player's height. Measures the *emitted* door floor, not `rules.rs`'s pre-layout `max(a.floor, b.floor)` proxy. | Error |
| `V-P7` | The key-aware flood: no player 1 start; an extra player 1 start; a start in no sector; no exit line; more lock classes than a `KeyMask` holds; the map is unfinishable; a reachable `(sector, keys)` state can no longer reach an exit; a sector no walk reaches. | Error |
| `V-P8` | A one-sided line's front `texturemiddle` is not `"-"`; two sectors with differing floors give the **lower-floor** side a `texturebottom`; differing ceilings give the **higher-ceiling** side a `texturetop`. Re-derived from `r_segs.c`'s `R_StoreWallRange` (lines 570 and 589), not from the compile-side `visible_*_side` pair. | Error |
| `V-P9` | No sidedef carries a `scalex*`/`scaley*` UDMF extension — vanilla's renderer has no per-sidedef scaling, so their presence means a source-port-only effect. Named by the first linedef referencing the sidedef, else by the map (an orphan sidedef). | Error |
| `V-P11` | No door-special line carries `dontpegtop` or `dontpegbottom` on its own face. **A convention pin, not an engine rule** — `ML_DONTPEGBOTTOM` is inert on a typical door face, whose visible texture lives in the upper slot — which is why it is a Warning, the same downgrade §9 gives P10. Measured: 247 of 255 door-special lines in `DOOM2.WAD` carry neither. | Warning |
| `V-P13` | An action line's tag resolves to at least one sector (a dead action otherwise). **The four exit specials are exempt** — `G_ExitLevel`/`G_SecretExitLevel` are `void (void)` and neither the switch nor the walkover path ever looks a tag up, so an unresolved tag there was never going to be read. Symmetrically, a sector carrying a tag no action line references is a stale tag. | Error for an unresolvable action tag; Warning for a stale sector tag |
| `V-P14` | No action line carries tag 0. Tag 0 is not "no tag" — it is the tag every untagged sector already has, so one stray zero opens every door. | Error |
| `V-P19` | Every sector's `lightlevel` is inside `Tables::light_range()`. Unconditional, spec or no spec. | Error |
| `V-P20` | **Embedding:** no collectible (pickup, ammo, weapon, `backpack`, key, or one of the eight powerups) sits inside a blocking prop's radius. **Reachability:** every collectible's sector is one the V-P7 flood actually reached; runs only when the flood ran. | Error |
| `V-P24` | Every locked-door **class** present has at least one key of that colour placed, and every placed key opens at least one door present. Class-level, because `26` is all an emitted linedef retains — it opens to either `blue_card` or `blue_skull`. Doors dedupe by `(door sector, class)`, so one physical door with two faces reports once. | Error |
| `V-P25` | Every player start clears its sector's **non-passable** walls by at least the player's radius (an open doorway cannot crush you against it); clears every other thing whose name resolves to a blocking prop on **both axes at once** by `prop.radius + player.radius` (`PIT_CheckThing`'s own axis-aligned `blockdist` box, not a circular distance); and no two starts of any kind are within telefrag distance (`2 × radius`) of each other. | Error |

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

**Key classes are interned by lock, not by key name** — a card and a skull of
one colour share a class, because `EV_VerticalDoor` accepts either. Where
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
does turns on their value. Unlike the rest of the module, nothing here
re-derives a playability rule — every row is a target-vs-actual comparison, so
the only sourcing burden is the ammo ratio's damage figures and the two
thing-flag bits (`MTF_AMBUSH` = 8; multiplayer-only = 16, which the pinned
source writes as a raw literal with no named constant).

Thirty-five rows are fixed, plus one per spec monster species, one per placed
species the spec never names (always `Fail`, target `absent`), and one per
`sustain.powerups[]` entry. Entrada against its paired spec produces 48.

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
present`). And `progression.exit.trigger` is `NotDerivable` whenever the spec
targets `teleport`: this compiler emits no teleports, so that target can never
be measured (`no teleports emitted`).

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
0 blocking, 0 warning(s), 48 conformance row(s), 3 tag(s)
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
own door-special lines do — this document's own V-P11 entry (above) already
measures it, at 247 of 255 carrying neither. A binary-sourced map drawing
that warning is the checker doing its job on real content, not a false
positive.

## What the verifier deliberately does not check

- **Lifts and teleports.** Their linedef specials (62/88 and 97) are sourced
  and reachable through `Tables`, but this compiler emits neither and neither
  `invariants.rs` nor `flood.rs` models their traversal semantics. They are
  therefore kept **out** of the recognized-special set on purpose: a map
  carrying one draws a `V-S` warning saying this checker does not model that
  special and the flood cannot vouch for its effect on traversal, instead of a
  silent pass. Recognizing them without understanding them would make the
  flood optimistic — it could call a map finishable that a player diverted or
  blocked by that line could not finish.
- **Sector specials, liquids included — and these *do* pass silently.** The
  warning above is a *linedef*-special check: `check_recognized_specials`
  reads `Boundary::special`, and a damaging floor is a **sector** special
  (`data/engine.toml`'s `[sector.damage]`, a numerically distinct space, as
  that table's own neighbouring comment spells out).
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
  emitted `26` names a colour class, not a card. The two rules deliberately
  disagree on a map that locks `"blue_card"` and places a `blue_skull`
  (`KNOWN-GAPS.md` records why neither should be "fixed" into agreement).
- **The rest of the catalog.** P1 (retired), P5, P6, P10, P12, P15, P16, P17,
  P18, P21, P22, P23 have no `V-` id — the set the compiler leaves uncovered,
  less P20, which the verifier does cover. P18 is the near miss: its counting
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
