# Build with all targets.
build:
    cargo build --all-targets

# Test, matching CI's test job.
test:
    cargo test --all-targets

# Auto-format.
fmt:
    cargo fmt --all

# Format check + clippy with warnings denied, matching CI's lint job. The
# crate's own lints are `warn`; the -D here is what makes a warning fail.
lint:
    cargo fmt --all --check
    cargo clippy --all-targets -- -D warnings

# Docs with warnings denied, matching CI.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps

# Dependency audit (requires cargo-deny).
deny:
    cargo deny check

# Pre-push gate, cheapest first. CI additionally runs the OS matrix, MSRV,
# coverage, `cargo deny`, and the committed-WADs drift guard — `gh pr checks`
# is the source of truth.
ci: lint test doc

# Check vendored shared files against their pinned canonical sources
# (.meta-manifest.toml). Network-dependent (fetches raw.githubusercontent.com),
# so deliberately NOT part of `just ci`; CI runs it as the meta-check job.
meta-check:
    python3 scripts/meta_sync.py check

# Rewrite vendored shared files from their pinned canonical sources. Bump a
# ref in .meta-manifest.toml first to adopt a canonical change.
meta-sync:
    python3 scripts/meta_sync.py sync
