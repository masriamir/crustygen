## Summary

- 

## Validation

- [ ] `just ci` (`lint test doc`) passed locally on a toolchain matching CI's `stable`. CI additionally runs the OS matrix, MSRV 1.94.0, coverage, `cargo deny`, and the committed-WADs drift guard — `gh pr checks` is the source of truth.

## Data-table changes

If this PR adds or edits a value in `data/engine.toml` or `data/vocabulary.toml`:

- [ ] Every value carries a citation — a `source`, a `derivation`, or `curated` (never written from recall; a `curated` entry claims no source). A wrong constant ships a map that loads and misbehaves, and no test catches it.
