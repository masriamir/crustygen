# The build stage and its CLI

`crustygen-build` is the command-line face of the compiler: it takes an IR
document (the room-graph JSON of `docs/design.md` §6) and produces a playable
UDMF PWAD with real nodes. It is the stage the other two binaries bracket —
`crustygen-check` verifies what it wrote, `crustygen-lift` will one day
produce what it reads — and it exists so that nothing outside this crate has
to link against `crustygen::compile` to turn an IR into a map: a generation
loop, a hand author, or a contributor reproducing `maps/entrada.wad` all get
the same one-command path.

## The CLI contract

```
usage: crustygen-build <ir.json> <out.wad> [--map NAME]
```

Reads `<ir.json>`, validates it through `Ir::from_json`, compiles it with
`compile::compile`, packs the result with nodes via `pack_udmf_with_nodes`
under the map name `NAME` (default `MAP01`), and writes `<out.wad>`. On
success one summary line prints to stdout:

```
MAP01: 8 rooms, 7 portals → 18 sectors, 86 linedefs, 25 things → out.wad
```

**It refuses rather than reports.** The binary uses the compiler's refusing
entry point, so a map that breaks a playability rule is never written —
that is the compiler contract of `docs/design.md` §7.2, and it is what a
caller feeding rejections back to a generator wants. The reporting posture
(emit the map *and* the list of what it breaks) belongs to `crustygen-check`,
which reads the built WAD.

**Exit codes follow the pipeline stages**, one code per stage, so a caller
gets the funnel from the status alone. Every failure names what failed on
stderr, prefixed `crustygen-build:` and then the stage:

| Code | Stage | stderr prefix | Meaning |
|---|---|---|---|
| 0 | built | — | `<out.wad>` was written |
| 1 | IR | `ir:` | invalid JSON, or an `IrError` validation failure (off-grid coordinate, odd width, unknown room, …) |
| 2 | usage / I/O | — | bad flag, missing positional, unreadable `<ir.json>`, unwritable `<out.wad>`, unloadable tables, or a pack failure |
| 3 | compile | `compile:` | a structural refusal — overlapping rooms, a portal between rooms that share no wall, a thing outside its room, … |
| 4 | playability | `playability:` | the map compiled but breaks playability rules; one line per rule, e.g. `playability: P3 (a <-> b): opening 16 is narrower than the 32 the player needs` |

No WAD is written on any non-zero exit.

## Reproducibility

Emission order is fixed (`docs/design.md` §7.1), so the same IR yields the
same bytes: `crustygen-build tests/fixtures/entrada_base.json out.wad` writes
a file byte-identical to the committed `maps/entrada.wad`, and
`tests/build_cli.rs` pins that. The committed map's binary Doom-format twin,
`maps/entrada_doom.wad`, is not produced here — it is the `cwad convert`
downconvert described in `tests/first_map.rs`.

## Tests

`tests/build_cli.rs`: each usage failure exits 2 naming the problem; the
entrada fixture builds with exit 0, the expected summary line, and bytes
equal to `maps/entrada.wad`; `--map E1M1` names the map group; invalid JSON
and an off-grid coordinate exit 1 with `ir:`; overlapping rooms exit 3 with
`compile:`; a portal narrower than the player exits 4 naming `P3`; an
unwritable output path exits 2. The IR fixtures are the two-room map from the
rules' own unit tests, patched in code.
