# crustygen

Compiles a hand-authored room-graph IR into a UDMF `TEXTMAP`, packs it into a
playable Doom PWAD, and emits a binary Doom-format twin.

The compiler removes coordinate bookkeeping, not layout design. You describe
rooms, portals, doors, things and an exit; it produces watertight geometry,
allocates tags, and refuses to emit a map a player could not walk through.

```
map-spec (Markdown)  →  parse  →  Spec        (not yet wired to the IR)

IR (JSON)  →  validate  →  compile  →  TEXTMAP  →  PWAD      (crustygen-build)
                                          └──────→  Doom-format twin

PWAD (+ optional Spec)  →  crustygen-check  →  findings + conformance rows
```

Built on [crustywad](https://github.com/masriamir/crustywad) for WAD I/O and
node building, consumed as a pinned published dependency.

## Status

The compiler works and its output has been played to completion in Chocolate
Doom. What exists is the geometry core, a map-spec parser that turns a filled
copy of [`map-spec.template.md`](map-spec.template.md) into a typed `Spec`
(see [`docs/map-spec.md`](docs/map-spec.md)), and the layer-4 verifier that
re-derives playability from a *built* WAD (see
[`docs/check.md`](docs/check.md)). The spec is not yet wired to the compiler:
spec → IR generation does not exist, so the IR below is still authored
directly as JSON and built with `crustygen-build` (see
[`docs/build.md`](docs/build.md)). See [Known gaps](#known-gaps).

```bash
cargo test                                   # 623 tests
cargo run --bin crustygen-build -- tests/fixtures/entrada_base.json out.wad
cargo run --bin crustygen-check -- maps/entrada.wad \
    --spec tests/fixtures/entrada.spec.md
```

The sample maps live in `maps/` — two maps, each shipped in both formats:

- `maps/entrada.wad` — the **UDMF** build of entrada, the eight-room map with
  a key, a locked door, a secret and a switch exit. Needs a ZDoom-family port
  (GZDoom, Odamex, Eternity).
- `maps/entrada_doom.wad` — entrada's binary **Doom-format** twin. This is the
  one for Chocolate Doom or anything else vanilla-accurate.
- `maps/salto.wad` — the **UDMF** build of salto, the five-room teleport map:
  a two-way pad pair, a monsters-only ambush closet, and a one-shot pad into a
  teleport-only exit room. Same ZDoom-family requirement.
- `maps/salto_doom.wad` — salto's binary **Doom-format** twin, same guidance as
  entrada's.

Loading the UDMF build in a vanilla port dies with
`W_LumpLength: <n> >= numlumps`, because `P_SetupLevel` addresses map data as
fixed offsets from the marker and a UDMF group has only three lumps after its
own. Load the twin that matches your engine.

```bash
chocolate-doom -iwad doom2.wad -file maps/entrada_doom.wad -warp 1
```

## How it works

An IR document names rooms (clockwise, grid-snapped footprints with a floor,
ceiling, light and textures), portals between them, things inside them, and
an exit. Rooms are authored **apart**: the void between two rooms is real,
solid wall, and the compiler fills it with a passage — or, for a door, a
chain of up to three sectors. See [`docs/geometry.md`](docs/geometry.md).

Compilation runs a fixed pass order, each pass depending on the last: emit
room sectors, resolve secret specials, cut portals, emit doors, carve exits,
emit teleport pads and their destination markers (`emit_teleports`), check no
two emitted sectors overlap, apply height textures, place things, check no
action sits at tag 0, render `TEXTMAP`, then run the playability catalog. A violation is a hard error, not a warning — a door the player
cannot fit through is a broken map, not a missed target.

## A second opinion on the built map

Those checks run against the IR, before a coordinate exists — so a compiler
bug that satisfies them still ships. `src/check` and its `crustygen-check`
binary are verification layer 4 (`docs/design.md` §8): they read a built
PWAD's map group — a UDMF `TEXTMAP`, or a classic Doom binary-format group
assembled and rendered to the same form — and re-derive the same invariants
from the emitted geometry, reusing the sourced tables and the reachability
core but nothing from `compile/` or `rules.rs` — the logic under
cross-examination.
Sixteen checks, from dangling cross-references to a key-aware flood — with
directed teleport edges resolved the way `EV_Teleport` resolves them — that
proves the map can still be finished. Given a map-spec it also grades a fixed
catalog of frontmatter parameters against their actual values — a parameter
with no sourced geometric meaning is an explicit not-derivable row rather than
a silent gap, and a structurally broken map marks every row not-run rather
than judging one against corrupt geometry. Exit 0 clean, 1 on a defect, 2 on
bad input. See [`docs/check.md`](docs/check.md).

## Surveying a WAD

`crustygen-lift` is the first stage of decompiling a WAD's geometry back into
a map-spec. Its default output only surveys: reading a WAD's maps through the
same shared ingest path `crustygen-check` uses, and reporting raw element
counts and linedef/sector/thing-type histograms per map, human-readable or as
`--json`. That layer interprets nothing — no table lookups, no spec emission.
See [`docs/lift.md`](docs/lift.md).

`crustygen-lift --vocabulary` adds a per-map verdict on four axes: whether
every special and thing type is in the compiler's emittable vocabulary, plus
the **teleport recognizer**, which resolves every teleport line the way
`EV_Teleport` does, classifies the pad shape it lands on, and refuses the
shapes the IR cannot state. That fourth axis is the first thing here that
reads geometry rather than a table. `crustygen-corpus <dir>` sweeps a
directory of idgames zips into the corpus expressibility report the vocabulary
roadmap is re-ordered from — still an upper bound. See
[`docs/corpus.md`](docs/corpus.md).

## The data tables are the highest-stakes part

Every engine value in `data/engine.toml` and `data/vocabulary.toml` carries a
`source` citation to the id-Software DOOM release at pinned commit
`a77dfb96cb91780ca334d0d4cfd86957558007e0`. Computed values carry a separate
`derivation`. Curated judgment calls — which texture names read as a door,
which trim marks a keyed door — carry `curated` instead, and must not claim a
source.

This is not ceremony. **A wrong constant produces a map that loads, renders
correctly, and is unplayable — and no test catches it, because the test reads
the same table the compiler does.** A reported gap always beats a plausible
guess.

Where a convention has no engine constant behind it, it is *measured* rather
than guessed: the card-versus-skull key trim, for instance, was counted across
the four id/Final Doom IWADs. See
[`docs/measurements/`](docs/measurements/).

## Documentation

| Document | What it covers |
|---|---|
| [`KNOWN-GAPS.md`](KNOWN-GAPS.md) | **Read this first.** Every known gap and every decision that looks wrong without its reason |
| [`docs/design.md`](docs/design.md) | The map-spec template, the IR, the compiler contract, and the v1 bar |
| [`docs/map-spec.md`](docs/map-spec.md) | The map-spec document format, the parser's API, and the enforcement split |
| [`docs/build.md`](docs/build.md) | The build stage: the `crustygen-build` CLI contract, its per-stage exit codes, and byte-reproducibility of the committed map |
| [`docs/check.md`](docs/check.md) | The layer-4 verifier: the check catalog, the flood's construction rules, conformance verdicts, and the CLI contract |
| [`docs/lift.md`](docs/lift.md) | The lifter's charter, its telemetry and teleport-recognizer scope, and the `crustygen-lift` CLI contract |
| [`docs/corpus.md`](docs/corpus.md) | The corpus sweep: what "expressible" means on its four axes (and does not), the `crustygen-corpus` CLI contract, and the per-release re-run procedure |
| [`docs/geometry.md`](docs/geometry.md) | Worked coordinates for the gap and door-chain constructions |
| [`docs/verticality.md`](docs/verticality.md) | Height differences, and the stairs/lifts phases that follow |
| [`docs/measurements/`](docs/measurements/) | Corpus measurements: the retail-IWAD verticality survey, the idgames expressibility instrument, and the teleport before/after |

## Known gaps

The honest list is [`KNOWN-GAPS.md`](KNOWN-GAPS.md). The headlines:

- **The map-spec parser reads the template; nothing turns it into an IR
  yet.** `src/spec` parses a filled `map-spec.template.md` copy into a typed
  `Spec` (see [`docs/map-spec.md`](docs/map-spec.md)); the IR is still
  authored directly as JSON (and built with `crustygen-build`), and spec → IR
  generation does not exist.
- **No conformance report file.** `docs/design.md` §8 specifies five
  verification layers; layer 4 now exists and layer 2 does not. §8.1's
  `report.md` is likewise unwritten — `crustygen-check` computes everything it
  needs and prints plain lines, but nothing renders the table or the sacrifice
  list, or writes a file alongside the WAD.
- **The compiler's P7 still passes vacuously without a player start or an
  exit**, and it has no P20 pass at all. The verifier closes both at layer 4 —
  a missing start or exit is a hard finding there, and V-P20 checks each
  pickup for prop embedding and flood reachability — but on a built WAD, one
  stage later.
- **15 of 27 playability rules** are enforced by the compiler; the verifier
  re-derives fourteen of them from the emitted map and adds P20. The exception
  is P26 (teleport-only exit room), which the verifier grades as a conformance
  row rather than as a check — a teleport exit emits exactly a plain walkover
  exit's specials, so nothing on the line tells them apart.
- **Texture alignment is minimal** — only an exit switch is centred; offsets
  do not accumulate across collinear runs.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
