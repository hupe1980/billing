# billing — development task runner
# Install just: https://just.systems/man/en/

# ── Default ────────────────────────────────────────────────────────────────────
# Show all available recipes.
default:
    @just --list --unsorted

# ── Code quality ───────────────────────────────────────────────────────────────

# Check formatting without making changes.
fmt-check:
    cargo fmt --all --check

# Format all source files.
fmt:
    cargo fmt --all

# Run Clippy on all targets and features (warnings are errors).
lint:
    RUSTFLAGS="-D warnings" cargo clippy --all-targets --all-features -- -D warnings

# Quick type-check (no codegen; fastest feedback loop).
check:
    cargo check --all-targets --all-features

# ── Testing ────────────────────────────────────────────────────────────────────

# Run unit + doc tests with default features.
test *ARGS:
    cargo test {{ ARGS }}

# Run tests with all features enabled.
test-all:
    RUSTFLAGS="-D warnings" cargo test --all-targets --all-features

# Run tests with no default features.
test-no-features:
    RUSTFLAGS="-D warnings" cargo test --all-targets --no-default-features

# Test against the declared MSRV (requires `rustup toolchain install 1.85`).
#
test-msrv:
    cargo +1.85 test --all-targets --all-features

# Run a specific test by name filter.
test-one FILTER:
    cargo test --all-features {{ FILTER }}

# ── Examples ──────────────────────────────────────────────────────────────────

# Run all examples.
examples: example-saas example-water example-cloud

# Run the SaaS billing example.
example-saas:
    cargo run --example saas_billing

# Run the water utility example.
example-water:
    cargo run --example water_utility

# Run the cloud compute example.
example-cloud:
    cargo run --example cloud_compute

# ── Documentation ─────────────────────────────────────────────────────────────

# Build and open documentation in the browser.
doc:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --open

# Build documentation without opening (useful in CI).
doc-build:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# ── Benchmarks ────────────────────────────────────────────────────────────────

# Run criterion benchmarks.
bench *ARGS:
    cargo bench {{ ARGS }}

# Save the current benchmark results as the comparison baseline.
bench-baseline NAME="main":
    cargo bench -- --save-baseline {{ NAME }}

# Compare current performance against a saved baseline.
bench-compare NAME="main":
    cargo bench -- --baseline {{ NAME }}

# ── Security ──────────────────────────────────────────────────────────────────

# Audit dependencies for known vulnerabilities (requires `cargo install cargo-audit`).
audit:
    cargo audit

# Check for accidental semver-breaking API changes
# (requires `cargo install cargo-semver-checks`).
semver:
    cargo semver-checks check-release

# ── Full CI (mirrors GitHub Actions) ──────────────────────────────────────────

# Run every gate that CI runs: format → lint → docs → tests → examples.
ci: fmt-check lint doc-build test-all test-no-features test-msrv bench-check examples
    @echo ""
    @echo "✓ All CI gates passed locally"

# Verify benchmarks still compile (they are not run in CI — too slow and noisy).
bench-check:
    cargo bench --no-run

# ── Release ───────────────────────────────────────────────────────────────────

# Dry-run publish — verify the crate packs correctly without uploading.
release-dry-run:
    cargo publish --dry-run --allow-dirty

# Tag a new release. Creates an annotated git tag; push it to trigger CI+publish.
# Usage: just release 0.7.0
release VERSION:
    @echo "Tagging v{{ VERSION }} …"
    @grep -q 'version.*=.*"{{ VERSION }}"' Cargo.toml \
        || (echo "ERROR: Cargo.toml version is not {{ VERSION }}"; exit 1)
    git tag -a "v{{ VERSION }}" -m "Release v{{ VERSION }}"
    @echo "Run 'git push origin v{{ VERSION }}' to trigger the release workflow."

# ── Utilities ─────────────────────────────────────────────────────────────────

# Remove build artifacts.
clean:
    cargo clean

# Show dependency tree.
tree *ARGS:
    cargo tree {{ ARGS }}

# Show outdated dependencies (requires `cargo install cargo-outdated`).
outdated:
    cargo outdated

# Expand macros for a specific file (requires `cargo install cargo-expand`).
expand FILE:
    cargo expand --lib {{ FILE }}
