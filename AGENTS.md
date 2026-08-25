# AGENTS.md — crustygen

Shared, tool-neutral guidance for any agent working in `crustygen`. Claude reads it via the
`@AGENTS.md` import in `CLAUDE.md`; GitHub Copilot code review reads it directly. Sections marked
with `meta:` markers are canonical blocks synced from `masriamir/.github` — edit them upstream,
not here (see `.meta-manifest.toml` and `just meta-check`).

## Project overview

`crustygen` compiles a hand-authored room-graph IR into a UDMF `TEXTMAP`, packs it into a playable
Doom PWAD, and emits a binary Doom-format twin. It removes coordinate bookkeeping, not layout
design: you describe rooms, portals, doors, things and an exit; it produces watertight geometry,
allocates tags, and refuses to emit a map a player could not walk through. Built on
[crustywad](https://github.com/masriamir/crustywad) (pinned published dependency, `write` +
`nodebuild` features) for WAD I/O and node building. Single crate, Rust 2024 edition, MSRV 1.94.0,
`publish = false`, dual-licensed MIT OR Apache-2.0.

**Read [`KNOWN-GAPS.md`](KNOWN-GAPS.md) first** — every known gap and every decision that looks
wrong without its reason. The compiler works and its output has been played to completion in
Chocolate Doom; the map-spec parser and layer-4 verifier exist, but spec → IR generation does not
(the IR is still authored directly as JSON).

## Layout

```
src/
  lib.rs          # library root
  ir.rs           # the room-graph IR (authored as JSON today)
  ingest.rs       # shared WAD map-group ingest (used by check + lift)
  geom.rs         # geometry primitives
  reach.rs        # reachability core (key-aware flood)
  pack.rs         # TEXTMAP → PWAD packing
  compile/        # the fixed-order compile passes (sectors, portals, doors, exits, heights, things, render)
  check/          # verification layer 4 — re-derives playability from a BUILT WAD
  lift/           # crustygen-lift: survey a WAD's geometry (telemetry only, no spec emission yet)
  bin/            # crustygen-check.rs, crustygen-lift.rs
data/
  engine.toml     # engine constants — every value carries a `source` citation (see below)
  vocabulary.toml # texture/name vocabulary — sourced or `curated`
docs/             # design.md, map-spec.md, check.md, lift.md, geometry.md, verticality.md, measurements/
maps/             # entrada.wad (UDMF) + entrada_doom.wad (binary twin)
map-spec.template.md  # the blank map-spec a parser turns into a typed Spec
tests/            # integration tests + fixtures
```

## Development workflow

Install [just](https://github.com/casey/just), then:

| Recipe | Command |
|---|---|
| Build (all targets) | `just build` |
| Test (all targets) | `just test` |
| Lint (fmt check + clippy, warnings denied) | `just lint` |
| Auto-format | `just fmt` |
| Docs (warnings denied) | `just doc` |
| Dependency audit | `just deny` (requires `cargo-deny`) |
| Pre-push gate (lint + test + doc) | `just ci` |
| Vendored-file drift check | `just meta-check` |

`just ci` (`lint test doc`) is the pre-push gate, cheapest first. CI additionally runs the OS
matrix, the MSRV build on 1.94.0, coverage, `cargo deny`, and the committed-WADs drift guard, so
`gh pr checks` is the source of truth, not a green `just ci`. The crate's own clippy lints are
`warn`; CI's `-D warnings` is what makes them fail.

```bash
cargo test                                   # the full suite
cargo run --bin crustygen-check -- maps/entrada.wad --spec tests/fixtures/entrada.spec.md
```

## Code conventions

### Language

<!-- >>> meta:language-en-us -->
- **American English spelling everywhere** — not only documentation: identifiers, code comments, doc comments, CLI and other user-visible output, commit messages and PR text. Take the American form of every `-ise`/`-ize`, `-our`/`-or`, `-re`/`-er` and `-ae`/`-e` pair: `initialize`, `honor`, `center`, `artifact`, `color`, `behavior`, `analyze`.
- **Third-party vocabulary keeps its own spelling.** GitHub Actions' job-status literal is `cancelled`; a status value, API field or dependency identifier is quoted, never corrected. The rule governs our words, not other people's.
- **Applying or flagging this is not a mechanical find-and-replace.** Skip backticked code spans, and match the *pattern* (`-ise`/`-ize`, and the others above) rather than a literal wrong word — the American forms listed above are the intended spellings, not violations. Because a rule like this must name the very spellings it forbids, a blind sweep rewrites its own counter-examples: a bullet meaning "write `color`, not the `-our` form" gets flattened to "write `color`, not `color`", which forbids nothing.
- **Check spelling as you write, not only when reviewing** — text copied verbatim from upstream is the usual source of slips.
<!-- <<< meta:language-en-us -->

### The data tables are the highest-stakes part

Every engine value in `data/engine.toml` and `data/vocabulary.toml` carries a `source` citation to
the id-Software DOOM release at pinned commit `a77dfb96cb91780ca334d0d4cfd86957558007e0`. Computed
values carry a separate `derivation`; curated judgment calls (which texture names read as a door,
which trim marks a keyed door) carry `curated` instead, and must **not** claim a source.

**This is not ceremony. A wrong constant produces a map that loads, renders correctly, and is
unplayable — and no test catches it, because the test reads the same table the compiler does.**
Never write an engine constant from memory: cite it, derive it, or curate it. A reported gap always
beats a plausible guess. Where a convention has no engine constant behind it, it is *measured*
across the retail IWADs (see [`docs/measurements/`](docs/measurements/)), never guessed.

### Error handling, documentation, lints

- Errors use `thiserror`-derived enums. `missing_docs = "deny"` is enforced — every public item has
  a doc comment.
- `clippy::all` + `clippy::pedantic` are enabled (warn locally, denied in CI). Prefer
  `T::try_from(..)` over `as` casts to stay clean under `pedantic`.
- Compilation runs a fixed pass order, each pass depending on the last; a violation is a **hard
  error, not a warning** — a door the player cannot fit through is a broken map, not a missed target.

## Testing

- `cargo test` runs the full suite. Verification layer 4 (`src/check`, the `crustygen-check` binary)
  re-derives playability from a **built** WAD, reusing the sourced tables and the reachability core
  but **nothing** from `compile/` or `rules.rs` — the logic under cross-examination — so a compiler
  bug that satisfies the IR-time checks is still caught against emitted geometry.
- **Fixture diversity matters more than fixture count.** A suite where every fixture is the same
  shape rotated once hid four Critical geometry defects behind 65 green tests; add fixtures whose
  *shape* is new, and state the cells a fixture does not cover.
- Commercial IWADs are never committed; corpus measurements live in `docs/measurements/`.

## Commit conventions

<!-- >>> meta:commit-conventions -->
Follow [Conventional Commits](https://www.conventionalcommits.org/): `feat` (new functionality), `fix` (bug fix), `docs` (documentation only), `test` (test-only), `refactor` (no behavior change), `chore` (build/tooling), `ci` (CI workflows). Scope is encouraged — `feat(map):`, `fix(cli):`.

**Mark breaking changes** with `!` (`feat(map)!: remove RejectLump`) or a `BREAKING CHANGE:` footer. Release automation derives the version bump from these annotations, so an unmarked breaking change proposes a semver-violating patch release.

**The PR title is the changelog entry and the version bump.** PRs squash-merge to a single commit whose subject is the PR title and whose body is blank — every branch commit subject is discarded. So the PR title alone selects the changelog section and drives the derived bump. Write it as a real Conventional Commit describing the shipped outcome; never `gh pr create --fill` (it takes the title from the branch name). Title a mixed PR by its highest-impact change (`!` > `feat` > `fix` > everything else), or split it into one PR per type when both halves each earn a changelog line. Never hand-force a version to compensate for a title.
<!-- <<< meta:commit-conventions -->

Crustygen specifics: the crate is `publish = false` and cuts no releases — the version stays
`0.1.0` and there is no release automation, so the Conventional Commit type is a changelog/clarity
choice, not a version one. `lefthook`'s `commit-msg` hook and CI's `pr-title` job share
`scripts/check-conventional-subject.py`, so the branch-commit gate and the PR-title gate cannot
drift.

## Git branching workflow

<!-- >>> meta:branch-naming -->
Branch from `main` after a `git pull`. Name every branch `<type>/<slug>` where `type` is one of `feature`, `bugfix`, `hotfix`, `docs`, or `chore`. The slug is descriptive and always required — a bare number such as `feature/42` is rejected — and is prefixed with the issue number when a tracking issue exists (`feature/42-mmap-support`). The number is optional in the pre-push hook but expected for the issue-driven `feature`/`bugfix`/`hotfix` types; `docs`/`chore` branches commonly omit it.

**Release branches are not used.** Release automation handles version bumps, changelog, and tags from the Conventional Commits on `main`; merge the release PR to ship.
<!-- <<< meta:branch-naming -->

## Copilot review

<!-- >>> meta:copilot-review-loop -->
PRs are reviewed automatically by `copilot-pull-request-reviewer`. Work through its comments — review threads **and** the suppressed comments in the review body — across as many rounds as needed. Verify each finding against the actual code before acting; bots are sometimes wrong or working from a stale diff.

A PR is ready for human review only when **all** of these hold:

- every automated review thread is resolved,
- every required CI check passes (`gh pr checks`), and
- the codecov comment reports no uncovered changed lines (or each remaining miss is consciously justified).

Resolved threads over a red required check — or unaddressed missing coverage — do **not** make a PR ready. Whether a fresh review is auto-requested on push or must be requested by hand is a per-repo ruleset detail (`review_on_push`); check the ruleset when a request seems stuck rather than assuming.
<!-- <<< meta:copilot-review-loop -->

Crustygen specifics: `just ci` (`lint test doc`) is the local pre-push gate; the required checks on
the `Main Branch` ruleset (OS-matrix test, MSRV, coverage, `security-deny`, the committed-WADs drift
guard, `pr-title`, `meta-check`) are the source of truth via `gh pr checks`.

## Known gaps

The honest, current list is [`KNOWN-GAPS.md`](KNOWN-GAPS.md). Headlines: spec → IR generation does
not exist (the IR is authored as JSON); no conformance report file; 12 of 25 playability rules are
enforced by the compiler (the verifier re-derives those twelve and adds P20); texture alignment is
minimal (offsets do not accumulate across collinear runs).
