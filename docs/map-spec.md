# The map-spec document and its parser

The map-spec is the pipeline's first artifact: a filled Markdown template —
YAML frontmatter carrying everything a machine can check, a prose body
carrying what only prose can (design intent, the ordered sequence of events,
what each secret is). `docs/design.md` §5 defines the template; this document
defines how `src/spec` turns a filled copy into a typed value, and exactly
which promises that stage does and does not make.

This stage **reads** the spec. It does not turn a spec into an IR — layout is
authored, not generated (`docs/design.md` §11) — and it does not compare the
spec against a built map; that is the verifier's job. What it produces is the
set of *targets* the verifier and the conformance report check against, which
is why it exists before either of them can.

## The document format

A spec file is YAML frontmatter between `---` fences, followed by a Markdown
body. The fence split is hand-rolled (the format is this project's own
contract; no Markdown library is involved anywhere in the parser).

**Frontmatter.** `spec_version: 1` plus seventeen groups: `identity`,
`players`, `scale`, `progression`, `architecture`, `combat`, `arsenal`,
`sustain`, `secrets`, `difficulty`, `aesthetics`, `flats`, `vertical`,
`scenery`, `pacing`, `compat`, `constraints`. **Every field is required.**
There are no defaults and no optional knobs: the input contract is a *filled*
template, the blank template hands an author every field, and a missing field
is indistinguishable from a forgotten decision. This is the same
reject-don't-degrade posture the IR takes, and the same idiom the template
itself uses (`powerups` entries with `count: 0` state absence explicitly
rather than by omission). Unknown keys are rejected at every level
(`deny_unknown_fields`), because a typo'd optional key that parses cleanly is
exactly the silent-degrade failure this project's culture exists to prevent.

Fields the template types as unions — `auto | explicit list`,
`none | <species>`, a compass direction *or* degrees — are hand-implemented
enums, not serde-untagged ones, so their parse errors name the field and the
allowed forms instead of serde's "did not match any variant".

**Body.** Four `##` sections, all defined in `docs/design.md` §5:

- `Overview` and `Notes` — free text, captured verbatim. Absent is fine
  (empty): they carry mood for humans, and rejecting a map over a missing
  Notes section would police prose, not correctness.
- `Sequence of events` — an ordered list, captured as a list of strings.
  Absent is fine (empty). An item's text may wrap: a line that is not itself
  a list marker (`N. `) but is indented and follows an already-open item
  continues that item, joined onto it with a single space — the shipped
  template wraps its longer steps this way. An indented line with no item
  open yet is rejected the same as any other line that is neither blank nor
  a list marker.
- `Secrets` — one `### Secret N — <name>` subsection per secret, each with
  `Trigger`, `Reward`, and `Hint` bullets. A bullet's value may carry a
  trailing `<!-- ... -->` HTML comment — the shipped template's
  allowed-values annotation on `Trigger` — which is stripped before the
  value is matched or stored. These parse into typed entries: the trigger
  must be one of `misaligned_texture`, `shootable`, `walkover`,
  `lift`, `hidden_switch`, and reward and hint must be non-empty. Secrets are
  structured because the verifier needs them as targets (P18's counting rule
  compares emitted secret sectors against `secrets.count`; the per-secret
  prose is what a future hint-check reads).

An unknown `##` heading is an error, for the same reason as an unknown YAML
key: `## Secert` silently vanishing is worse than a rejection.

**One erratum, canonicalized here.** `docs/design.md` §5's filled example
writes `Trigger: misaligned texture` while its own allowed-values comment
defines the enum as `misaligned_texture`. The comment is the contract; the
shipped template and every fixture use the snake_case form, and the parser
accepts only that form.

## The API

```rust
Spec::from_markdown(&str) -> Result<SpecDocument, SpecError>
// SpecDocument { spec: Spec, sacrifices: Vec<Sacrifice> }
```

`Spec` follows the `ir::Ir` naming idiom; `SpecError` lives beside it the way
`IrError` lives in `ir.rs`. `sacrifices` is empty by construction under
`enforcement: strict` — in that mode the same findings become the error
instead.

## Errors and the enforcement split

Every error names its subject as a field path (`docs/design.md` §9). YAML
deserialization runs through `serde_path_to_error`, so a type or enum mistake
reports `progression.doors.lock_types[1]`, not a line number the author has
to chase back to a field.

`constraints.enforcement` governs **only** the parse-time consistency set.
The split:

**Always errors, in both modes** — malformed or impossible documents:

- serde-layer failures: wrong type, unknown key, out-of-set enum value
  (necessarily reported one at a time — serde stops at the first);
- `min > max` in any declared range pair, including `lighting.min`/`max` and
  `scale.vertical_range`;
- fractions outside 0..=1: `corridor_ratio`, `hitscanner_ratio`,
  `deaf_ratio`, `outdoor_proportion`, `liquid.coverage`, `peak_position`;
- non-positive `difficulty.scaling` factors, `identity.grid`, or
  `arsenal.ammo.ratio`;
- `detail_level` outside 1..=5; light levels outside the engine's own domain
  (`Tables::light_range`, sourced);
- `spec_version` other than 1; `identity.slot` not a valid `MAPnn`;
- `doors.lock_types` not a subset of `progression.keys`;
- `constraints.priority` missing an entry or repeating one — v1 requires a
  total order over all six (`progression_correctness`, `playable_balance`,
  `sector_budget`, `monster_counts`, `detail_level`, `play_time`), or ties
  are unresolvable;
- `coop_starts` or `dm_starts` beyond the engine's start-thing maxima, once
  those bounds carry sourced entries (see the sourcing note below);
- any content name that fails vocabulary resolution (next section);
- a secret with an unknown trigger or an empty reward or hint;
- a powerup whose `count` and `placement` contradict each other (`count: 0`
  without `placement: none`, or vice versa), and a weapon placed `none` —
  self-contradictory documents, not preferences.

**Enforcement-governed** — internally-visible consistency, where `strict`
rejects and `target` records a `Sacrifice` against `constraints.priority`:

- `secrets.count` versus the number of prose `### Secret` sections;
- the lighting band: `base`, `outdoor`, and `base + corridor_delta` each
  within `[lighting.min, lighting.max]`;
- `locked_doors` versus `lock_types` coherence: fewer locked doors than
  declared lock types means some lock type has no door to carry it. (More
  doors than lock types is fine — several doors sharing one key is ordinary
  Doom.)

A `Sacrifice` carries the field path, the target the spec asked for, the
actual the document shows, and a message phrased as which parameter was
sacrificed to hold which — ready for the conformance report to render
verbatim.

Post-deserialize validation findings are **collected, not first-error**: an
author fixing a seventeen-group document deserves the full list in one pass.

## Vocabulary resolution

Structural enums with a closed value set in the template (`shape`,
`backtracking`, `encounter_style`, budget tiers, `placement`, `enforcement`,
the priority entries…) are Rust enums. Content names are **not**: species,
weapon, powerup, and key names, the theme, and light-effect names all
validate against `data/vocabulary.toml` and `data/engine.toml` through
`Tables`, which already declare themselves the resolution target for the
template's fields. A parallel Rust enum would be a second source of truth
waiting to drift from the sourced one.

A few fields sit in between: their value set is closed and pinned as a Rust
enum, but the set itself comes from the template's own allowed-values
comment rather than from either table, because nothing in `engine.toml` or
`vocabulary.toml` enumerates it — there is no engine fact or asset name to
cite. `combat.sound.block_sound_at` (`key_doors | arena_entrances`),
`aesthetics.lighting.effects.forbid_in` (`combat_arenas | secret_rewards`),
`scenery.barrels.keep_clear_of` (`player_start | key_pickup |
secret_reward`), and `difficulty.baseline`'s five skill names (`itytd |
hntr | hmp | uv | nm`) are all compiler-construction vocabulary, pinned
directly as enum variants rather than resolved through a table.
`constraints.forbid` mixes both kinds in one field: an entry is either a
real species name (sourced, resolved through `Tables::species`) or one of
three fixed mechanic concepts — `crusher`, `dark_maze`, `insta_death_pit` —
that name compiler behavior rather than a placeable thing, matched literally
instead of looked up. And `combat.boss`'s `mastermind` short form is a
template convenience, not a vocabulary key: the parser bridges it to
`Boss::Species("spider_mastermind")` itself (`frontmatter::Boss`'s own doc
comment gives the reasoning) rather than adding an alias to
`vocabulary.toml`, since guessing which of the two forms future code would
want is exactly the kind of unsourced choice that table avoids.

Consequences worth stating plainly:

- `theme: hell` is an error today, because `vocabulary.toml` defines exactly
  one theme. That is honest: the compiler genuinely cannot build it.
- Engine-derived bounds obey the sourcing rule like everything else. The
  light domain already has a sourced accessor. Bounds that do not yet have a
  sourced entry — the engine's coop and deathmatch start maxima — get one,
  cited against pinned `a77dfb96`, before the check is written; a bound that
  cannot be sourced leaves the check unwritten and recorded as a gap, never
  guessed.
- Where a lookup does not exist yet, the implementation adds the sourced
  table entry and accessor rather than hardcoding a list in the parser.
  Light-effect names took exactly this path: `aesthetics.lighting.effects.allowed`'s
  four variants now resolve through `Tables::light_effect_special`.
- `flats.liquid.kind` deliberately does not follow it, even though its value
  set (`none | nukage | blood | lava | slime | water`) is just as closed: it
  stays a structural Rust enum, not a `Tables`-resolved content name. Turning
  `nukage` into an actual liquid *flat* name needs the same kind of
  Freedoom-IWAD flat measurement `vocabulary.toml`'s texture tables already
  cite for walls — work that belongs to the liquid-emission stage, which
  does not exist yet (`KNOWN-GAPS.md`'s **P16**/**P17**), not to a parser
  that only reads the spec.

## Deliberately deferred

Deeper conflicts of the kind `docs/design.md` §5.1 expects —
`hitscanner_ratio` feasibility against the per-species min/max ranges,
`sectors.max` against `detail_level` — belong to the resolver and verifier
stages, not the parser, and the parser records **nothing** about them.
This is scope discipline, not missing data: `Tables::hitscan` already
classifies species, so the verifier can compute feasibility cheaply when it
exists. Recording the deferral here keeps the conformance report from ever
implying parse-time coverage that does not exist.

Also out of scope, per issue #1: spec → IR generation, the verifier, the
report, and P18's sector counting (the parser supplies `secrets.count` and
the per-secret entries as targets; the comparison against emitted geometry is
the verifier's).

## Artifacts and tests

`map-spec.template.md` ships at the repository root, **filled** with
`docs/design.md` §5's example values (canonicalized), every field carrying
its allowed-values comment. A blank template could neither parse nor be
tested; the filled one is simultaneously the contract, the documentation,
and a parseable artifact — a test pins that it parses clean with zero
sacrifices. Authors copy it and edit values.

The test posture follows `docs/design.md` §10 and the fixture-diversity
lesson in `KNOWN-GAPS.md`:

- a second, structurally different filled fixture (different shape choices,
  `strict` enforcement, a boss, degrees-valued facing, explicit texture
  lists) so the suite is not one document admired from two angles;
- boundary-pinned pairs for every validation rule — violate by one unit and
  assert the exact field path, then pass at the threshold — built by patching
  one line of the template string per test rather than committing thirty
  near-identical files;
- every enforcement-governed rule exercised in both modes: the same violation
  errors under `strict` and yields the recorded sacrifice under `target`;
- body-grammar cases: unknown heading rejected, missing optional sections
  fine, a secret missing its Hint failing by name.
