# Contributing to crustygen

crustygen compiles a hand-authored room-graph IR into UDMF `TEXTMAP` and playable Doom
PWADs, built on [crustywad](https://github.com/masriamir/crustywad) as a pinned crates.io
dependency. Issues and pull requests are welcome.

By participating you agree to the
[Code of Conduct](https://github.com/masriamir/.github/blob/main/CODE_OF_CONDUCT.md).
To report a security issue, use
[private vulnerability reporting](https://github.com/masriamir/crustygen/security/advisories/new)
rather than a public issue.

## Setup

1. Install Rust **1.94.0** or newer and [just](https://github.com/casey/just).
2. Install [lefthook](https://github.com/evilmartians/lefthook) and run `lefthook install` —
   the hooks gate branch names and commit subjects and run fmt/clippy before each commit.
   A fresh clone has no hooks until this runs; skip it and every local gate silently does
   nothing.
3. Python **3.11 or newer** (`python3`) powers the commit-subject validator and the
   meta-sync drift check — `meta_sync.py` uses the stdlib `tomllib`, which older
   Pythons lack.

## Everyday commands

| Command | What it does |
|---|---|
| `just build` / `just test` | build / test with all targets |
| `just lint` | `cargo fmt --check` + clippy with warnings denied |
| `just doc` | docs with warnings denied |
| `just deny` | dependency audit (`cargo deny check`; requires `cargo-deny`) |
| `just ci` | the pre-push gate: lint, test, doc |
| `just meta-check` | vendored shared files match their pinned canonical sources |

**Run `just ci` before pushing.** `gh pr checks` on the PR is the source of truth — CI
additionally runs the OS matrix, MSRV, coverage, `cargo deny`, and the committed-WADs
drift guard, which the local gate does not.

## Branches, commits, and PR titles

Branch `<type>/<slug>` from `main` (`feature | bugfix | hotfix | docs | chore`), with the
issue number in the slug when one exists. Commits follow
[Conventional Commits](https://www.conventionalcommits.org/), enforced by lefthook via
`scripts/check-conventional-subject.py`. PRs squash-merge, so **the PR title becomes the
only commit on `main`** — write it as a real Conventional Commit; CI's `pr-title` check
validates the form.

## Committed WAD artifacts

`maps/*.wad` are **committed on purpose**, and CI's `committed WADs match the compiler`
job recompiles the fixture and fails on drift — when the compiler or IR changes,
regenerate and commit the artifacts (`cargo test --test first_map` locally names the
failure). Do not add `*.wad` to `.gitignore`.

## Shared conventions

Account-wide conventions (issue/PR flow, review loop, action pinning) live in
[masriamir/.github](https://github.com/masriamir/.github); crusty-family specifics
(inter-repo pinning rules, cross-repo ADRs) in
[crusty-meta](https://github.com/masriamir/crusty-meta).
