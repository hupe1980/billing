# billing — development task runner (https://just.systems). `just ci` mirrors CI.

# The pinned validation artefacts. Spelled once: `references` and the citation check read them.
cen_pin := "validation-1.3.16"
cen_zip := "eInvoicing-EN16931-validation-1.3.16.zip"
cen_url := "https://github.com/ConnectingEurope/eInvoicing-EN16931/archive/refs/tags/validation-1.3.16.zip"
peppol_pin := "v3.0.20"
peppol_zip := "peppol-bis-invoice-3-v3.0.20.zip"
peppol_url := "https://github.com/OpenPEPPOL/peppol-bis-invoice-3/archive/refs/tags/v3.0.20.zip"
kosit_pin := "v2.6.0"
kosit_zip := "xrechnung-3.0.2-schematron-2.6.0.zip"
kosit_url := "https://github.com/itplr-kosit/xrechnung-schematron/releases/download/v2.6.0/xrechnung-3.0.2-schematron-2.6.0.zip"

default:
    @just --list --unsorted

# ── Code quality ──────────────────────────────────────────────────────────────

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

lint:
    cargo clippy --all-targets --all-features -- -D warnings

# ── Tests ─────────────────────────────────────────────────────────────────────
# `--all-targets` excludes doctests (rust-lang/cargo#6669); the README is the crate docs.

alias test-all := test

test:
    RUSTFLAGS="-D warnings" cargo test --all-targets --all-features

test-doc:
    RUSTFLAGS="-D warnings" cargo test --doc --all-features

test-no-features:
    RUSTFLAGS="-D warnings" cargo test --all-targets --no-default-features

# The declared MSRV, doctests included (`rustup toolchain install 1.85`).
test-msrv:
    cargo +1.85 test --all-targets --all-features
    cargo +1.85 test --doc --all-features

# ── Constitution I: no_std, two runtime dependencies, a sound floor ───────────

# A crate that builds for a bare-metal target cannot do I/O.
nostd:
    cargo build --no-default-features --target thumbv7em-none-eabihf
    cargo build --no-default-features --features serde --target thumbv7em-none-eabihf

# Exactly `rust_decimal` (+ optional `serde`) as direct runtime dependencies.
deps-check:
    #!/usr/bin/env bash
    set -euo pipefail
    check() { # EXPECTED FLAGS...
        want="$1"; shift
        got=$(cargo tree -e normal --depth 1 --prefix none --format '{p}' "$@" | tail -n +2 | awk '{print $1}' | sort -u | paste -sd' ' -)
        if [ "$got" != "$want" ]; then
            echo "✗ runtime dependencies with $*: '$got', expected '$want'" >&2
            echo "  extra: $(comm -13 <(tr ' ' '\n' <<<"$want" | sort) <(tr ' ' '\n' <<<"$got" | sort) | paste -sd' ' -)" >&2
            exit 1
        fi
    }
    check "rust_decimal serde" --all-features
    check "rust_decimal" --no-default-features
    echo "✓ runtime dependencies: rust_decimal, serde (optional)"

# Resolve every direct dependency, library and dev alike, to its declared floor (nightly), then build and
# test on 1.85. Runs in a copy so the working tree's Cargo.lock is never touched.
minimal-versions:
    #!/usr/bin/env bash
    set -euo pipefail
    root=$(pwd); tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
    rsync -a --exclude target --exclude .git --exclude concepts --exclude specs --exclude fuzz ./ "$tmp/"
    cd "$tmp"
    export CARGO_TARGET_DIR="$root/target/minimal-versions"
    cargo +nightly update -Z direct-minimal-versions
    floor=$(sed -nE 's/^rust_decimal *= *\{ *version *= *"([0-9.]+)".*/\1/p' Cargo.toml | head -1)
    [[ "$floor" =~ ^[0-9]+\.[0-9]+$ ]] && floor="$floor.0"
    resolved=$(cargo +1.85 pkgid -p rust_decimal | sed -E 's/.*[@#]//')
    if [ "$resolved" != "$floor" ]; then
        echo "✗ rust_decimal resolved to $resolved, but the declared floor is $floor" >&2; exit 1
    fi
    cargo +1.85 test --locked --all-targets --no-default-features
    cargo +1.85 test --locked --all-targets --all-features
    cargo +1.85 test --locked --doc --all-features
    echo "✓ minimal versions (rust_decimal $resolved) build and pass on 1.85"

# ── Docs, benches, examples, site ─────────────────────────────────────────────

doc-build:
    RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features

# Benchmarks compile in CI; timings on shared runners are noise, so nothing gates on them.
bench-check:
    cargo bench --all-features --no-run

bench *ARGS:
    cargo bench --all-features {{ ARGS }}

# Run every example (each ends by asserting a conservation or a total).
examples:
    #!/usr/bin/env bash
    set -euo pipefail
    shopt -s nullglob
    for f in examples/*.rs; do cargo run --quiet --all-features --example "$(basename "$f" .rs)"; done

site-serve:
    cd site && zola serve

site-build:
    cd site && zola build

site-check:
    cd site && zola check

# Compile and run every Rust block on the site (generates git-ignored tests/site_examples.rs).
site-verify:
    python3 site/verify-examples.py

# ── Supply chain and API ──────────────────────────────────────────────────────
# One advisory checker: cargo-deny (advisories, licenses, bans, sources). It resolves
# features, so it sees the build graph rather than every optional crate in Cargo.lock.

deny:
    cargo deny --all-features check

# Against the newest published release. 0.15 → 0.16 is a declared breaking bump for a
# 0.x crate, so the intended break passes; from 0.16.1 on, a break fails again.
semver:
    cargo semver-checks check-release

# ── Fuzzing (cargo-fuzz, nightly; fuzz/ is its own unpublished package) ───────

fuzz-build:
    #!/usr/bin/env bash
    if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
        echo "skipped: needs nightly and cargo-fuzz (cargo install cargo-fuzz --locked)"; exit 0
    fi
    cargo +nightly fuzz build

# Run one target (parse, split, serde). New inputs go to target/, seeds stay in fuzz/corpus/.
fuzz TARGET SECS="60":
    mkdir -p target/fuzz-corpus/{{ TARGET }}
    cargo +nightly fuzz run {{ TARGET }} "{{ justfile_directory() }}/target/fuzz-corpus/{{ TARGET }}" \
        "{{ justfile_directory() }}/fuzz/corpus/{{ TARGET }}" -- -max_total_time={{ SECS }}

# ── Mutation testing (cargo-mutants; config in .cargo/mutants.toml) ───────────
# The full sweep is local (`mutants-gate`, before a release); CI runs `mutants-diff-gate`.
# Baseline entries are `survivor  # reason`; the comparator strips the reason.

mutants *ARGS:
    cargo mutants -j "${CARGO_MUTANTS_JOBS:-$(just _cores)}" {{ ARGS }}

# Full sweep; FAIL on a new survivor and on a recorded one that is now caught.
mutants-gate:
    #!/usr/bin/env bash
    set -uo pipefail
    rm -rf target/mutants-full
    cargo mutants -j "${CARGO_MUTANTS_JOBS:-$(just _cores)}" --output target/mutants-full
    just _mutants-check target/mutants-full exact $?

# Mutate only the lines DIFF touches; FAIL on any survivor the baseline does not record.
mutants-diff-gate DIFF:
    #!/usr/bin/env bash
    set -uo pipefail
    rm -rf target/mutants-diff
    cargo mutants --no-shuffle -vV --in-diff {{ DIFF }} -j "${CARGO_MUTANTS_JOBS:-$(just _cores)}" --output target/mutants-diff
    just _mutants-check target/mutants-diff subset $?

# The pre-push form of the CI job.
mutants-diff BASE="origin/main":
    mkdir -p target && git diff {{ BASE }}.. > target/mutants.diff
    just mutants-diff-gate target/mutants.diff

# Rewrite the baseline from the last full sweep, keeping every recorded reason.
mutants-baseline-update:
    python3 -I scripts/mutants.py update

[private]
_cores:
    @n=$( (sysctl -n hw.ncpu 2>/dev/null || nproc 2>/dev/null || echo 4) ); echo $(( n / 2 > 0 ? n / 2 : 1 ))

[private]
_mutants-check OUTDIR MODE STATUS:
    python3 -I scripts/mutants.py check {{ OUTDIR }} {{ MODE }} {{ STATUS }}

# ── Citations (constitution IV) ───────────────────────────────────────────────
# Every rule id cited in src/, the README, the site, tests, examples and benches must
# exist in the pinned artefacts, and its (context, test, flag), binding parameters
# resolved, must hash to its entry in citations.lock. Artefacts come from
# concepts/reference/cen/ when present, else are fetched into target/citations/.

check-citations: (_citations "check")

# Rewrite citations.lock, printing every added, removed and changed entry.
citations-lock-update: (_citations "update")

[private]
_citations MODE:
    #!/usr/bin/env bash
    set -euo pipefail
    dir=target/citations; mkdir -p "$dir"
    get() { # ZIP URL → local path
        if [ -s "concepts/reference/cen/$1" ]; then echo "concepts/reference/cen/$1"; return; fi
        [ -s "$dir/$1" ] || curl -fsSL --retry 3 -o "$dir/$1" "$2"
        echo "$dir/$1"
    }
    python3 -I scripts/citations.py {{ MODE }} \
        "cen@{{ cen_pin }}=$(get {{ cen_zip }} {{ cen_url }})" \
        "peppol@{{ peppol_pin }}=$(get {{ peppol_zip }} {{ peppol_url }})" \
        "kosit@{{ kosit_pin }}=$(get {{ kosit_zip }} {{ kosit_url }})"

# Download the cited standards, artefacts, directives, statutes and code lists into
# concepts/reference/ (gitignored; concepts/REFERENCES.md is the index).
references:
    CEN_ZIP={{ cen_zip }} CEN_URL={{ cen_url }} PEPPOL_ZIP={{ peppol_zip }} PEPPOL_URL={{ peppol_url }} \
        KOSIT_ZIP={{ kosit_zip }} KOSIT_URL={{ kosit_url }} bash scripts/references.sh

# Local only: roadmap items and spec folders match one-to-one; nothing published cites concepts/, specs/ or .specify/.
concepts-check:
    bash scripts/concepts-check.sh

# ── CI and release ────────────────────────────────────────────────────────────

# The local core of CI. Not here: test-msrv, minimal-versions, semver, deny, fuzz-build
# (toolchains/tools), and mutants-gate (tens of minutes; run before a release).
ci: fmt-check lint doc-build test test-doc test-no-features nostd deps-check check-citations
    @echo "✓ local CI core passed. Before release: just test-msrv minimal-versions mutants-gate"

release-dry-run:
    cargo publish --dry-run

# Tag a release; pushing the tag runs release.yml, which publishes by crates.io Trusted
# Publishing (OIDC, environment `crates-io`). No token is stored anywhere.
#
# Break-glass, only when OIDC publishing is down and the release cannot wait (the default
# answer is to wait and re-run the workflow): an owner (1) turns trusted-publishing-only
# off on crates.io, (2) creates a token scoped to publish-update for `billing` only, with
# the shortest expiry offered, (3) publishes from a clean checkout of the tagged commit
# after `just release-dry-run`, (4) revokes the token, (5) turns trusted-publishing-only
# back on.
release VERSION:
    @grep -q '^version *= *"{{ VERSION }}"' Cargo.toml || (echo "Cargo.toml version is not {{ VERSION }}" >&2; exit 1)
    git tag -a "v{{ VERSION }}" -m "Release v{{ VERSION }}"
    @echo "Run 'git push origin v{{ VERSION }}' to trigger release.yml."
