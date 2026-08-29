# The corpus sweep and its CLI

`crustygen-corpus` turns a directory of idgames zips (or bare WADs) into one
number the vocabulary roadmap is re-ordered from: the share of maps whose
every line special, sector special, and thing type is in crustygen's
emittable vocabulary **and** whose every teleport line resolves to a shape
the lifter's recognizer can state. It is the measuring half of Project G —
every vocabulary release re-runs it against the same sample so the yield
moves with the vocabulary, not with the sample.

## What "expressible" means — and does not

Four axes. The first three are membership, read from the same tables the
compiler reads; the fourth reads geometry:

| Axis | Set |
|---|---|
| line specials | `Tables::emittable_line_specials()` — what a compiler pass writes today (door, keyed doors, four exits, and the four teleports 97/39/126/125); the lift specials are sourced but not yet emitted, so they are *out* |
| sector specials | `Tables::named_sector_specials()` — secret, the three damage tiers, the four light effects |
| thing kinds | `Tables::thing_kinds()` |
| teleports | no set — `lift::teleport::recognize` resolves every teleport line the way `EV_Teleport` does and refuses the shapes the IR cannot state; the axis passes when the map has no refusal |

`tests/vocabulary_arbiter.rs` compiles a fixture per construct and asserts
the curated line set equals what came out. Adding a special to the curated
set without a fixture that emits it fails the arbiter — that direction is
enforced. The other direction is not: no fixture can author a construct the
IR cannot express yet, so a landed emitting pass does not fail the test on
its own; it must add its fixture and grow the curated set by rule.

The teleport axis is the first that reads a map's geometry rather than a
set. Its report section counts the shapes the recognizer saw — player
versus monsters-only lines, one-shot lines, closet and exit and paired
lines, and the island/alcove/boundary/other classification of the sector
the crosser enters — and the two refusal classes that gate the verdict:
`self_referencing` (the line's front and back sector are the same, a
mapping trick the IR has no way to state) and `broken` (tag 0, no tagged
sector, no marker on the tag, or a one-sided line that can never fire).
A destination sector holding several markers is reported as `ambiguous`
and does **not** refuse: the engine's pick is deterministic and the IR
expresses it with one marker.

**Not measured:** room geometry, linedef flags, sector tags outside the
teleport resolution, texture and flat names, thing flags. The number is an
**upper bound** on what a geometry-aware lifter could express; every report
says so in its header.

The **vanilla-only slice** is defined by `engine.toml`
`[linedef.vanilla_specials]` — the union of every numeric `case` label in
the pinned engine's three special-line activation dispatchers
(`P_CrossSpecialLine`, `P_UseSpecialLine`, `P_ShootSpecialLine`) plus the
linedef-special switch `P_SpawnSpecials` runs over `lines[]` at level load,
which installs effect specials such as 48 (the scrolling wall) so the
engine can service them every tic. Maps outside the slice (Boom/MBF/ZDoom
numbering) are reported as their own share rather than as blockers.

## CLI contract

```
usage: crustygen-corpus <dir> [--json FILE] [--report FILE]
```

- `<dir>` is walked non-recursively, via a single directory listing. An
  entry the listing itself cannot read (a permission error mid-listing) is
  skipped silently — no bucket, no stderr line. `*.zip` opens through
  crustywad's archive reader (lenient options, CRCs still verified) and
  every `.wad` member is read through those same lenient options; `*.wad`
  files are read directly through crustywad's strict reader instead, so a
  WAD that only warns under lenient parsing loads as a zip member but
  counts as `wad_unreadable` when read bare — kept as-is since the sample
  of record is all zips. Everything else is ignored.
- Every map group goes through the shared `ingest::load_map` path, then
  `lift::survey` and `Vocabulary::classify`. Maps are deduplicated by
  `sha256:` over their lumps (name, length, bytes), so a map repackaged in
  several zips counts once.
- Failures never abort the sweep. Each is named on stderr and counted in a
  bucket: `archive_unreadable`, `wad_unreadable`, `no_maps`,
  `unsupported_format` (Hexen, Doom 64), `assembly_refused`,
  `textmap_unparseable`. `no_maps` (a WAD that loaded fine but carries no
  map group — ordinary resource-WAD content) is named and counted but is
  not a failure; see Exit codes below. `assembly_refused` counts binary maps
  refused by crustywad's **strict** `Map::assemble` — the shared ingest
  path — which also catches near-misses a lenient assembler would load (a
  REJECT lump one byte short has been observed on real corpus content); a
  lenient-ingest option is a possible follow-up, not implemented today.
- `--json FILE` writes `{provenance, buckets, aggregate, maps[]}` —
  `provenance` is `null` when `<dir>` carries no `sample-manifest.json`.
  `--report FILE` writes the Markdown aggregate (header caveat, sample,
  buckets, per-axis shares over all maps and the vanilla slice, top-25
  blockers per axis by map share, the Teleports section described above,
  and the greedy curve at k = 1, 5, 10, 21, 51 — once with the other axes
  held expressible, once over maps already ok on them). With neither flag
  the Markdown goes to stdout.
- Percentages in the Markdown render as `12.3 %` (one decimal place, a
  space before the sign). An axis with no out-of-set value renders `(none)`
  in its blocker table instead of a bare header, and a greedy curve with
  nothing left to add — its population already fully unblocked, or empty —
  renders a single baseline row at `k = 0` instead of an empty table. Both
  greedy curves report cumulative share **against all unique maps**, never
  against the population they walk: the conjunction curve's population is
  only the maps already ok on sector specials, thing kinds **and** the
  teleport axis, so its plateau below 100 % is exactly the maps still
  blocked on one of those three, not a truncated curve. Because the
  teleport axis joined that population, a conjunction checkpoint is not
  comparable with one from a run that predates it — only the ordering
  within a single run is.
- If `<dir>/sample-manifest.json` exists (written by crustywad's
  `xtask harvest-sample`), its seed, count, frame rows, fetch-list hash, and
  sorted ids are echoed into both outputs. A manifest that is present but
  unreadable or malformed is skipped with a warning on stderr rather than
  silently dropped; a directory with no manifest at all produces neither a
  warning nor a `## Sample` section.

Exit codes: `0` every candidate surveyed; `1` at least one archive, WAD, or
map **failed to load** — the `archive_unreadable`, `wad_unreadable`,
`unsupported_format`, `assembly_refused`, or `textmap_unparseable` buckets
(a map-free WAD is named on stderr and counted in `no_maps`, but does not
move the exit code); `2` usage, I/O, no candidates, or a serialization
failure.

## Re-running the measurement (every vocabulary release)

1. In the crustywad checkout: `just harvest-sample <seed> <count>` with the
   seed recorded in the latest measurement under `docs/measurements/`. A
   present, correctly sized zip is skipped, so this is cheap after the first run.
2. Here: `just corpus <path-to-sample-dir>` → writes
   `docs/measurements/expressibility-<today>.md` and a gitignored JSON under
   `target/`.
3. Compare the new "all axes" share, the Teleports section and the blocker
   tables with the previous doc; re-order the Project G queue from the blocker
   tables, not from memory.
4. **Only compare like with like.** A per-axis share and a blocker table are
   comparable across runs; a greedy-conjunction checkpoint is not, whenever
   the axes that define its population have changed between the two runs. The
   teleport axis joining that population is exactly such a change — see
   [`docs/measurements/teleports-2026-08-28.md`](measurements/teleports-2026-08-28.md),
   which records the before/after for the teleport construct and says so at
   both its curve table and its caveats.
   The lift shape probe ([`docs/measurements/lift-shapes-2026-08-29.md`](measurements/lift-shapes-2026-08-29.md))
   is the same kind of before-design measurement, and its tool is committed as
   `examples/liftprobe` (`cargo run --release --example liftprobe -- census <label> <dir>...`)
   so the numbers can be re-derived when the sample or the loader changes.
