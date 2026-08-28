# The lifter and its CLI

`crustygen-lift` is the first stage of the crustyllm program: decompiling a
WAD's geometry back into crustygen map-spec documents. Its charter is
*recognition, not approximation* — it will emit only constructs it can prove
the geometry means, and it measures everything it cannot express. Those
measurements are corpus telemetry, and they are what drive the vocabulary
roadmap: which linedef specials, sector specials, and thing types are common
and unambiguous enough across real maps to justify a recognizer.

## Current scope: the telemetry skeleton and vocabulary membership

The census interprets nothing: `crustygen-lift` surveys a WAD's maps — UDMF
or classic Doom binary format, through the same shared `crustygen::ingest`
path `crustygen-check` uses — into a raw census (vertex, linedef, sidedef,
sector, thing counts) and three raw histograms per map: non-zero linedef
specials, non-zero sector specials, and thing types, each keyed by its raw
numeric value with an occurrence count. No table lookups, no engine
constants, no vocabulary judgments, and no map-spec output there.
`lift::vocabulary` is the first interpreting layer, and it is table-only: a
membership test against the compiler's emittable sets, nothing about
geometry. Recognizers proper — the ones that reason about geometry — arrive
later (below).

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

`--vocabulary` appends a verdict per map: whether every non-zero linedef
special, sector special, and thing type is in the compiler's emittable
vocabulary (`Tables::emittable_line_specials`, `named_sector_specials`,
`thing_kinds`), the unknown values on each axis, and whether the map stays
inside the pinned engine's vanilla special list. This is membership only —
an upper bound on lift yield, not a geometric judgment.

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
| 2 | A usage, I/O, or WAD-level failure — bad flag, missing `<wad>`, unreadable file, not a WAD, no such `--map` group, a WAD with no map groups at all, or (rare) a telemetry-serialization failure under `--json`. Every such failure names what failed on stderr. |

## Next stages

- **Recognizers** — turn the raw histograms into named constructs: rooms,
  doors, keys, exits.
- **Map-spec emission** — recognized constructs become a crustygen map-spec
  document, with map-level atomicity: a map lifts completely or not at all.
- **Semantic round-trip QA** — a lifted spec recompiles and passes
  `crustygen-check`.
