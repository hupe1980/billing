+++
title = "Design invariants"
description = "The rules the crate holds itself to, how correctness is checked — property tests, mutation testing, a recorded survivor baseline — and how it compares."
weight = 130
+++

## Invariants

| Invariant | How it is enforced |
|---|---|
| No `f64` in monetary arithmetic | `Amount<P>` is `i64 × 10⁻ᴾ`; every intermediate uses `rust_decimal` |
| Encapsulated representation | the inner `i64` is private; `to_raw()` reads it and `from_raw_units()` reconstructs — both explicit, neither implicit |
| Overflow is visible, never silent | infallible ops **panic**; every `checked_*` returns `Err` and never panics, including on `Decimal`'s own panicking operators. `UsageAggregator::aggregate` is fallible too — raw usage events are the least trusted input there is |
| Rounding is explicit wherever it is observable | `RoundingStrategy` is a required parameter on every reduction. The one default — `quantity × price` at the engine's five decimals — cannot change an emitted digit, because `AmountScale` rounds each line once from the *exact* product instead |
| No silent precision loss | `parse` and `checked_from_decimal` refuse excess precision identically; `from_decimal_rounded` is the opt-in |
| Precision is bounded at compile time | `Amount<19>` fails to compile, with an explanatory const-eval message |
| A gross price is split, never multiplied | `extract_tax` applies `r/(1+r)`; `net_of_tax` takes the remainder, so the pair reconstructs the quoted price exactly |
| A conditional discount is not a discount | payment terms change no total and no breakdown |
| A layer never reverses its own direction | both percentage layers refuse a negative rate **and** clamp a negative base |
| Documents are self-validating | sixteen exact invariants, run by every constructor and again on deserialisation |
| Allocation is exact | `Σ(parts) == total`, with per-document penny correction |
| Money splits exactly | `distribute` / `allocate` / `proportional_split` never create or destroy a unit |
| Value added tax is distinguishable from a charge | marked from the layer's own `breakdown()` return value, so it is total rather than heuristic |
| Precision reduction happens at the leaves | totals are recomputed from reduced leaves, never rounded themselves |
| Invariants survive deserialisation | validated types re-run their checks via `#[serde(try_from)]` |
| The engine tag namespace is protected | caller labels that would collide with a reserved tag are rejected |
| No silent under-billing | unknown band names, uncovered quantities and non-monotonic schedules are errors, not skips |
| Zero domain assumptions | no jurisdiction constants, no default currency, no default cash increment |
| A position's bucket is its EN 16931 group | `net_positions` is BG-25; the other two are BG-20 / BG-21. An invoice line carrying document level allowance detail is refused — the amount would land in BT-106 instead of BT-107 with every total still balancing |
| No I/O, no async, no `unsafe` | `#![forbid(unsafe_code)]`; every `fn` is a pure `fn` |

## How correctness is actually checked

Four layers, each answering a question the previous one cannot.

**Example tests** answer *"does the code do what I expect?"* — around 700 of them,
including a suite of regressions named after the audit finding that produced each one.

**Property tests** answer *"does the law hold for inputs I did not think of?"*
`proptest` searches for counterexamples to the algebraic statements: parse/display
round-trips exactly, splitting conserves the total, rounding is idempotent and bounded,
a gross amount splits into net and tax without loss, `R120` is rounded once from the
exact value. Counterexamples are persisted and replayed forever after.

**Doctests** answer *"is the documentation true?"* The entire README is included into
the crate with `#![doc = include_str!]`, so every Rust example in it is compiled and
run — a snippet that stops compiling is a build failure, not a reader's problem.

**Mutation testing** answers the one none of the others can: *"would anything fail if
the code did something else?"* `cargo-mutants` rewrites the source — flipping `<` to
`<=`, deleting a guard, replacing a computed amount with its default — and reports
every rewrite the suite still accepts.

That fourth layer is the one that finds the gaps — code that can be changed, in ways
that alter what an invoice says, without a single test failing. On a suite that passes
every other level, a first sweep routinely surfaces dozens: mostly holes in the tests,
occasionally a bug.

It keeps earning its keep, most often on *new* code: a `discount_within` that
returned the first qualifying discount rather than the best one when the larger was
declared second, and a merge test that could not tell a currency mismatch from an
unstated BT-111 because it only asserted `is_err()`.

### Not a CI gate — an ad-hoc tool

Mutation testing is **not wired into CI**, on purpose. A sweep is ~1 000 mutants: tens
of minutes on a laptop, considerably worse on a shared runner. Paying that on every
pull request buys a signal that changes rarely, and waiting for it is the fastest way
to make people stop reading it. It is run by hand — after writing tests for a tricky
invariant, before a release, or when a survivor is suspected.

Three settings roughly halve the per-mutant cost without changing which mutants are
caught: a build profile with no debuginfo and no incremental state, excluding doctests
from the sweep, and not leaving the generated site-example harness in `tests/`.

Excluding doctests surfaces every function whose **only** test is a doctest: those
mutants survive. That is a finding, not a false positive, so the answer is a real test
— never a baseline entry. A function tested only by an example is a gap whether or not
a sweep is what reveals it.

### The survivor baseline

Surviving mutants are not suppressed with an exclusion regex — an exclusion is a
permanent blind spot, and the pattern that hides a builder today also hides it once the
builder starts doing real work. They are **recorded**, with a written reason each is
tolerated, and the gate fails in *both* directions: a new survivor is a genuine gap, and
a recorded survivor that is now caught means the entry is stale and the list should
shrink.

Every entry is one of two kinds:

- **provably equivalent** — the rewrite cannot change behaviour, e.g. `> ` to `>=` in
  `Amount::parse`, where the widened branch slices an empty tail;
- **unreachable by construction** — the branch exists but no input reaches it, verified
  against every construction path rather than assumed.

That second category is checked rather than trusted for a specific reason: an identical
"unreachable" comment in `proportional_split` turned out to be wrong, and the bug it hid
made the function reject splits its own validation had already accepted.

## Comparison

| System | Language | Notes |
|---|---|---|
| **Kill Bill** | Java | a full billing *platform*; this is a calculation *library* |
| **Lago** | TypeScript | API server with event ingestion; this is an embeddable library |
| **Stripe Billing** | SaaS | payment platform; this is standalone |
| **Chargebee / Zuora** | SaaS | subscription lifecycle management; out of scope |
| `rust_decimal` | Rust | low-level decimal arithmetic, no billing abstractions — and this crate's one runtime dependency |
| `rusty-money` | Rust | 128-bit money with formatting, exchange and `allocate`; no pricing models, tax layers or document model |
| `use-invoice` | Rust | invoice vocabulary for same-currency totals; explicitly does not calculate tax |

The honest summary: if you need subscription lifecycle, dunning, payment capture or a
hosted API, you want a platform and this is not one. If you need the *amounts* to be
right — and to still be right after they have been rounded to two decimals, split three
ways and reversed into a credit note — that is the gap this fills.

## Stability

Pre-1.0, a minor bump may break the API. In exchange, every breaking change is listed in
the [changelog](https://github.com/hupe1980/billing/blob/main/CHANGELOG.md) with the old
and new spelling; semantics changes are called out even when the signature is unchanged
(marked **⚠️ silent**, because they will not fail your build); every release carries a
migration table; and there are no deprecation shims, so the compiler shows you every
affected site in one pass.

`BillingError` is `#[non_exhaustive]` — new variants may appear in a minor release, so
always include a `_ =>` arm. `cargo-semver-checks` runs in CI, so an unintended break
fails the build rather than shipping.
