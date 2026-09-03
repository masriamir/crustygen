# The lifter and its CLI

`crustygen-lift` is the first stage of the crustyllm program: decompiling a
WAD's geometry back into crustygen map-spec documents. Its charter is
*recognition, not approximation* — it will emit only constructs it can prove
the geometry means, and it measures everything it cannot express. Those
measurements are corpus telemetry, and they are what drive the vocabulary
roadmap: which linedef specials, sector specials, and thing types are common
and unambiguous enough across real maps to justify a recognizer.

## Current scope: telemetry, vocabulary membership, and two recognizers

The census interprets nothing: `crustygen-lift` surveys a WAD's maps — UDMF
or classic Doom binary format, through the same shared `crustygen::ingest`
path `crustygen-check` uses — into a raw census (vertex, linedef, sidedef,
sector, thing counts) and three raw histograms per map: non-zero linedef
specials, non-zero sector specials, and thing types, each keyed by its raw
numeric value with an occurrence count. No table lookups, no engine
constants, no vocabulary judgments, and no map-spec output there.
`lift::vocabulary` is the first interpreting layer, and it is table-only: a
membership test against the compiler's emittable sets, nothing about
geometry.

`lift::teleport` is the first recognizer proper — the first thing here that
reasons about geometry rather than about a set. It resolves every teleport
line the way `EV_Teleport` does (walk the sectors carrying the tag in
declaration order; the first holding a destination marker wins), classifies
the shape of the sector the crosser enters as `island`, `alcove`, `boundary`
or `other`, and reports whether the line is a player or monsters-only
trigger, one-shot or repeatable, in a closet, beside an exit, or on a paired
pad. Shape is reported and never gates. Two shapes are **refused**, because
the IR would have to misrepresent them: `self_referencing` (the line's front
and back sector are the same) and `broken` (tag 0, no tagged sector, no
marker on the tag, or a one-sided line that can never fire). A destination
holding several markers is `ambiguous` — reported, not refused, since the
engine's pick is deterministic.

`lift::plat` is the second, over platforms. It reads the same engine-side
resolution the verifier does — `check::plats`, shared by the flood, the
V-P5/V-P11 invariants, the conformance rows and this recognizer, so the four cannot drift on
what a platform travels to or who can fire its triggers. Every sector a lift
line names by tag is resolved the way `EV_DoPlat` reads it (the floor it
travels to is `P_FindLowestFloorSurrounding`'s; a use line fires from its
front sector only, a walkover from whichever side can cross it at rest), and
then classified into the three shapes the IR can state: a **lift** (at rest
level with a landing, dropping to a low room), a **pedestal** (at rest above
its one neighbor) and a **barrier** (at rest above two or more neighbors that
share a floor). Eight refusals name what cannot be stated, judged in a fixed
precedence so a platform wrong in several ways reports its most fundamental
reason rather than an order-of-evaluation accident: `dead` (it travels 0, so
there is no movement to state), `shared_tag` (more than one sector answers to
the tag, where one IR lift is one platform), `one_shot`, `mixed_speed`,
`unsupported_rest`, `top_only` (no trigger fires from below — the lift a
player underneath cannot call), `one_way_barrier` (it lowers for one side
only) and `conflicting_action` (a non-lift special names the tag too). A lift
line that names no platform — tag 0, or a tag no sector answers to — is not a refused platform but a broken
line, counted alongside the refusals. `shared_tag` and `one_way_barrier` are
gates the shape probe behind `docs/measurements/lift-shapes-2026-08-29.md`
never applied, so these shape counts are subsets of that measurement's rather
than the same numbers.

The other recognizers, the ones that name rooms and doors, arrive later
(below).

## The CLI contract

```
usage: crustygen-lift <wad> [--map NAME] [--json] [--vocabulary]
```

Surveys every map group in the WAD, or just `--map NAME`. Default output is
one human-readable census line per surveyed map, e.g.:

```
MAP01: 42 vertices, 40 linedefs, 80 sidedefs, 12 sectors, 15 things; 6 distinct linedef special(s), 2 distinct sector special(s), 9 distinct thing type(s)
```

A map assembled from binary format (rather than parsed from a native
`TEXTMAP`) gets the line suffixed with `(assembled from binary format)`.
`--json` prints a JSON array of the same records instead, one object per
surveyed map in the census/histogram shape above; the origin suffix is a
human-output-only annotation and is not part of the JSON record.

`--vocabulary` appends a verdict per map on **six** axes. Three are
membership: whether every non-zero linedef special and sector special, and
every thing type, is in the compiler's emittable vocabulary
(`Tables::emittable_line_specials`, `named_sector_specials`, `thing_kinds`),
with the unknown values listed per axis, plus — on the line axis only —
whether every non-zero linedef special the map carries is one the pinned
vanilla engine dispatches (`(outside vanilla)` when not). The other three are
the recognizers', and they are the axes that read geometry: a map passes the
fourth when the teleport recognizer refused none of its teleport lines, the
fifth when the plat recognizer refused no platform and every lift line named
one, and the sixth when the floor recognizer refused no target and every
floor line named one. A refusal appends `(teleports refused: N)`,
`(lifts refused: N)` or `(floors refused: N)`, naming the count. A map with no
teleport line and a map whose every teleport line was recognized read the
same — silence on that axis — because there is nothing there the lifter would
have to drop; the lift and floor axes read the same way. `--json` carries the
same six-axis verdict alongside a `teleports` object, a `lifts` object and a
`floors` object, each with the full shape and refusal counts.

The three membership axes are an upper bound on lift yield, not a geometric
judgment; the teleport, lift and floor axes narrow that bound where they can.

**Per-map failure policy.** A group that fails to load through the shared
ingest path — for example an unsupported binary format (Hexen, Doom 64), a
non-UTF-8 or unparseable `TEXTMAP`, or an unassemblable binary map — is named
on stderr and skipped; every other selected group is still surveyed and
reported. This is a WAD-level survey, not a single-map tool, so one bad map
does not stop the others from being counted.

Exit codes:

| Code | Meaning |
|---|---|
| 0 | Every selected group surveyed |
| 1 | At least one selected group failed to load |
| 2 | A usage, I/O, or WAD-level failure — bad flag, missing `<wad>`, unreadable file, not a WAD, no such `--map` group, a WAD with no map groups at all, a tables-load failure under `--vocabulary`, or (rare) a telemetry-serialization failure under `--json`. Every such failure names what failed on stderr. |

## Next stages

- **The remaining recognizers** — rooms, doors, keys, exits. Teleports and
  lifts are done (above); everything else is still a raw histogram. The
  recognizers arrive in blocker order, which the corpus measurement re-ranks
  after every vocabulary release (`docs/corpus.md`).
- **Map-spec emission** — recognized constructs become a crustygen map-spec
  document, with map-level atomicity: a map lifts completely or not at all.
- **Semantic round-trip QA** — a lifted spec recompiles and passes
  `crustygen-check`.
