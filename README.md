# crustygen

Compiles a hand-authored room-graph IR into a UDMF `TEXTMAP`, packs it into a
playable Doom PWAD, and emits a binary Doom-format twin.

The compiler removes coordinate bookkeeping, not layout design. You describe
rooms, portals, doors, things and an exit; it produces watertight geometry,
allocates tags, and refuses to emit a map a player could not walk through.

```
IR (JSON)  →  validate  →  compile  →  TEXTMAP  →  PWAD
                                          └──────→  Doom-format twin
```

Built on [crustywad](https://github.com/masriamir/crustywad) for WAD I/O and
node building, consumed as a pinned published dependency.

## Status

The compiler works and its output has been played to completion in Chocolate
Doom. What exists is the geometry core; the map-spec front end does not exist
yet. See [Known gaps](#known-gaps).

```bash
cargo test        # 240 tests
cargo run --example ...   # not yet — there is no CLI
```

The sample map lives in `maps/`:

- `maps/entrada.wad` — the **UDMF** build. Needs a ZDoom-family port (GZDoom,
  Odamex, Eternity).
- `maps/entrada_doom.wad` — the binary **Doom-format** twin. This is the one
  for Chocolate Doom or anything else vanilla-accurate.

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
check no two emitted sectors overlap, apply height textures, place things,
check no action sits at tag 0, render `TEXTMAP`, then run the playability
catalog. A violation is a hard error, not a warning — a door the player
cannot fit through is a broken map, not a missed target.

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
| [`docs/geometry.md`](docs/geometry.md) | Worked coordinates for the gap and door-chain constructions |
| [`docs/verticality.md`](docs/verticality.md) | Height differences, and the stairs/lifts phases that follow |
| [`docs/measurements/`](docs/measurements/) | Corpus measurements over the retail IWADs |

## Known gaps

The honest list is [`KNOWN-GAPS.md`](KNOWN-GAPS.md). The headlines:

- **Nothing reads the map-spec template.** The IR is authored directly as
  JSON; the Markdown front end in `docs/design.md` is designed, not built.
- **No verifier and no conformance report.** `docs/design.md` §8 specifies
  five verification layers; layers 2 and 4 do not exist.
- **P20 has no per-pickup check, and P7 passes vacuously without a player
  start or an exit.** The key-aware reachability flood both would need now
  exists in `src/reach.rs`; P7 uses it to reject unfinishable and stranding
  maps, but P20's own per-pickup loop is still unwritten, and P7 itself does
  not run at all on a map missing a start or an exit.
- **12 of 25 playability rules** are implemented.
- **Texture alignment is minimal** — only an exit switch is centred; offsets
  do not accumulate across collinear runs.

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
