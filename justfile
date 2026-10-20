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

# Sweep a corpus sample directory into a dated measurement doc. JSON lands
# beside it under target/ (never committed).
corpus dir:
    cargo run --release --bin crustygen-corpus -- {{dir}} --report docs/measurements/expressibility-$(date +%F).md --json target/expressibility-$(date +%F).json

# Run the retail shape-count gate (tests/retail_shapes.rs) against a
# directory holding DOOM.WAD and DOOM2.WAD. The dir must be absolute: cargo
# runs the test binary from the package root, and the test skips a relative
# path. Nothing is fetched and nothing is committed; without the variable the
# test skips and passes, which is what CI does.
test-retail dir:
    CRUSTYGEN_IWAD_DIR="{{dir}}" cargo test --test retail_shapes -- --nocapture
