# GitHub Copilot Instructions for `crustygen`

`crustygen` compiles a room-graph IR into a UDMF `TEXTMAP` and a playable Doom PWAD, built on crustywad (Rust 2024, MSRV 1.94.0, `publish = false`, dual-licensed MIT OR Apache-2.0).

**The conventions to review against live in [`AGENTS.md`](../AGENTS.md), which you also read** — error handling (`thiserror`), `missing_docs = "deny"`, `clippy::pedantic`, the American-English spelling rule, and the high-stakes data-table sourcing rule.

## Review focus

- **Data-table values (`data/engine.toml`, `data/vocabulary.toml`) must carry a `source`, `derivation`, or `curated` citation** — never an uncited engine constant. A wrong constant produces a map that loads and renders but is unplayable, and no test catches it because the test reads the same table. This is the highest-stakes review target.
- Public items need doc comments (`missing_docs = "deny"`); prefer `T::try_from(..)` over `as` casts under `clippy::pedantic`.
- A playability violation is a hard error — except the documented warnings (P10 clean vertical tiling, `docs/design.md` §9; and the verifier's V-P11 convention check, `docs/check.md`), which are intentional and should not be flagged as bugs.

## Known false positives (do not flag)

- **"Freedoom"** is the correct project name; suggestions to write it "FreeDoom" are always wrong.
- The **American-English rule lists counter-examples**, so its own text necessarily contains the spellings it names; backticked code spans and third-party vocabulary are exempt (see `AGENTS.md`).
