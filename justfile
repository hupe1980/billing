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

# ── Mutation testing ──────────────────────────────────────────────────────────
#
# Coverage answers "was this line executed?". Mutation testing answers the
# question that actually matters for a billing engine: "would any test fail if
# it were wrong?".
#
# `mutants` reports, `mutants-gate` decides. Config lives in
# `.cargo/mutants.toml`; the survivors the gate tolerates, and the reason each
# one is tolerated, live in `.cargo/mutants-baseline.txt`.
#
# Requires `cargo install cargo-mutants --locked`.

# Sweep the whole crate and report survivors (slow — tens of minutes).
mutants *ARGS:
    cargo mutants {{ ARGS }}

# Sweep the whole crate and FAIL on any survivor not in the baseline.
mutants-gate:
    #!/usr/bin/env bash
    # The pass/fail form, and the one to run before a release. Exact: it fails on
    # a mutant that survives and is not recorded — a genuine new gap — and equally
    # on a recorded one that no longer survives, because that entry has gone stale
    # and the list should shrink.
    set -uo pipefail
    # Clear first. cargo-mutants can bail before writing anything — an empty diff,
    # a filter that matches no mutant — and leave the previous run's missed.txt in
    # place, which the check below would then read as this run's result.
    rm -rf target/mutants-full
    cargo mutants --output target/mutants-full
    just _mutants-check target/mutants-full exact $?

# Mutate only the lines a diff touches, and FAIL on any survivor not in the baseline.
mutants-diff-gate DIFF:
    #!/usr/bin/env bash
    # Subset: only the mutants the diff reaches are run, so the baseline is a
    # permitted set rather than an expected one. A recorded survivor whose line
    # this diff does not touch is simply absent, and that is not a failure.
    set -uo pipefail
    # See `mutants-gate`: a stale missed.txt from a previous run would otherwise be
    # read as this one's, and a docs-only diff writes nothing at all.
    rm -rf target/mutants-diff
    cargo mutants --no-shuffle -vV --in-diff {{ DIFF }} --output target/mutants-diff
    just _mutants-check target/mutants-diff subset $?

# Mutate only the lines your branch changed — the pre-push check CI also runs.
mutants-diff BASE="origin/main":
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p target
    git diff {{ BASE }}.. > target/mutants.diff
    just mutants-diff-gate target/mutants.diff

# Compare a finished run's survivors against the recorded baseline.
#
# MODE is `exact` (every recorded survivor must still survive) or `subset` (only
# a survivor that is *not* recorded is a failure). STATUS is cargo-mutants' own
# exit code, which the caller must pass in — a recipe runs in its own shell, so
# `$?` here would describe nothing.
[private]
_mutants-check OUTDIR MODE STATUS:
    #!/usr/bin/env bash
    set -uo pipefail
    # cargo-mutants exits 0 when nothing survived and 2 when something did.
    # Anything else — a baseline suite that already fails, a usage error, an
    # internal error — is not a survivor question and must not be reported as one.
    if [ "{{ STATUS }}" -ne 0 ] && [ "{{ STATUS }}" -ne 2 ]; then
        echo "cargo mutants exited {{ STATUS }} — see https://mutants.rs/exit-codes.html" >&2
        exit {{ STATUS }}
    fi
    # No missed.txt means cargo-mutants never got as far as writing results.
    # The caller cleared the directory first, so its absence is unambiguous — but
    # what it *means* depends on the mode. Under `subset` it is the ordinary
    # docs-only pull request: no Rust changed, no mutant ran, nothing to check.
    # Under `exact` a full sweep always produces one, so its absence is a broken
    # run, and reporting that as a pass would be the worst outcome available.
    missed="{{ OUTDIR }}/mutants.out/missed.txt"
    if [ ! -f "$missed" ]; then
        if [ "{{ MODE }}" = "subset" ]; then
            echo "✓ no mutants were generated for this range — nothing to check"
            exit 0
        fi
        echo "A full sweep produced no missed.txt under {{ OUTDIR }}/mutants.out." >&2
        echo "That is a broken run, not a clean one — investigate before trusting it." >&2
        exit 70
    fi
    # A missing baseline must not read as "nothing is recorded", which would turn
    # every known-equivalent survivor into a spurious failure and invite someone
    # to regenerate the file from whatever this run happened to produce.
    if [ ! -f .cargo/mutants-baseline.txt ]; then
        echo ".cargo/mutants-baseline.txt is missing — restore it rather than regenerating." >&2
        echo "It carries the reason each survivor is tolerated; a rebuilt file has none." >&2
        exit 70
    fi
    # Strip `:LINE:COL` so the baseline survives ordinary edits: only a real
    # change in *what* survives should ever churn that file.
    #
    # Plain `sort`, never `sort -u`. Two different mutants can normalise to the
    # same text — the same rewrite of the same operator twice in one function —
    # and collapsing them would let a second survivor hide behind the first.
    # Keeping duplicates means the baseline has to carry one line per survivor,
    # and a new twin changes the count and fails the gate.
    sed -E 's/^([^:]+):[0-9]+:[0-9]+: /\1: /' "$missed" | sort > "{{ OUTDIR }}/survivors.txt"
    grep -Ev '^[[:space:]]*(#|$)' .cargo/mutants-baseline.txt | sort > "{{ OUTDIR }}/baseline.txt"
    found=$(wc -l < "{{ OUTDIR }}/survivors.txt" | tr -d ' ')
    if [ "{{ MODE }}" = "subset" ]; then
        comm -13 "{{ OUTDIR }}/baseline.txt" "{{ OUTDIR }}/survivors.txt" > "{{ OUTDIR }}/unrecorded.txt"
        if [ ! -s "{{ OUTDIR }}/unrecorded.txt" ]; then
            echo "✓ $found survivor(s) in the mutated range, all recorded in .cargo/mutants-baseline.txt"
            exit 0
        fi
        echo "" >&2
        echo "These mutants survived and are NOT recorded in .cargo/mutants-baseline.txt:" >&2
        echo "" >&2
        sed 's/^/    /' "{{ OUTDIR }}/unrecorded.txt" >&2
        echo "" >&2
        echo "Each one is a change to the code that no test would notice." >&2
        echo "Write a test that fails when it is applied — or, if it genuinely cannot" >&2
        echo "change behaviour, add it to the baseline WITH the reason." >&2
        exit 1
    fi
    if diff -u "{{ OUTDIR }}/baseline.txt" "{{ OUTDIR }}/survivors.txt"; then
        echo "✓ $found survivors, every one recorded in .cargo/mutants-baseline.txt"
        exit 0
    fi
    echo "" >&2
    echo "Mutation survivors differ from .cargo/mutants-baseline.txt:" >&2
    echo "  '-' a recorded survivor is now caught — delete its entry and its comment." >&2
    echo "  '+' a NEW survivor — write a test that kills it, or justify it and record it." >&2
    echo "" >&2
    echo "'just mutants-baseline-update' rewrites the file, but read the diff first:" >&2
    echo "an unexplained entry is a gap someone decided not to close." >&2
    exit 1

# Rewrite the baseline from the last FULL sweep (drops every comment — restore them).
mutants-baseline-update:
    #!/usr/bin/env bash
    # Reads `target/mutants-full` and nothing else. A diff-scoped run only ever
    # sees the mutants its diff reached, so writing its survivors here would
    # silently delete every entry the diff did not happen to touch — turning a
    # narrow check into a wholesale erasure of the record.
    set -euo pipefail
    if [ ! -f target/mutants-full/survivors.txt ]; then
        echo "No full sweep to update from — run 'just mutants-gate' first." >&2
        echo "(A diff-scoped run cannot rewrite the baseline; it sees only part of it.)" >&2
        exit 1
    fi
    cp target/mutants-full/survivors.txt .cargo/mutants-baseline.txt
    echo "Baseline replaced with the last full sweep's survivors, WITHOUT comments."
    echo "Restore the reasoning for each entry before committing."

# Mutate one file, while writing the tests that kill its survivors.
mutants-file FILE:
    cargo mutants --no-shuffle --file {{ FILE }}

# Re-run only what was missed last time (finish with a full `just mutants-gate`).
mutants-iterate *ARGS:
    cargo mutants --iterate {{ ARGS }}

# List every mutant that would be generated, without running any of them.
mutants-list *ARGS:
    cargo mutants --list {{ ARGS }}

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
#
# Deliberately excludes the mutation sweep. CI mutates only a pull request's own
# diff (minutes); the full sweep is `just mutants-gate` and takes tens of them, so
# it belongs before a release rather than in this loop.
ci: fmt-check lint doc-build test-all test-no-features test-msrv bench-check examples
    @echo ""
    @echo "✓ All CI gates passed locally"
    @echo "  Before a release, also run: just mutants-gate"

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
