+++
title = "Design"
description = "The model — inputs in, every total derived once, nothing stored that a caller could edit — the invariants the crate holds itself to, and how correctness is checked."
weight = 130
+++

## The model

There are two kinds of value in the crate, and they never mix.

**Inputs** are what you state: `Line`s (quantity, price, optional stated net, VAT,
line allowances and charges, labels), `DocumentAdjustment`s, `apportion` inputs,
`Bound`s, `Payment`s, a `CashRounding`, a `VatAccounting`. Their constructors are
infallible and their fields are yours.

**Derived values** are what EN 16931 computes from them: BT-131 per line, the reduced
allowance and charge amounts, the VAT breakdown, BT-106 … BT-115, BT-111.
`DocumentBuilder::build` validates the inputs and derives all of them **once**; a
`Document` exposes them through infallible accessors and nothing derived can be set.
Consequences:

- **No `validate()`.** The arithmetic rules hold by construction
  ([the totals chain](../totals/#what-holds-by-construction)).
- **Builder order never matters** — except between credits, whose call order is their
  priority. Setting a single-valued setting twice is `ErrorKind::ConflictingSetting`.
- **Editing re-derives.** `Document::into_builder` returns the inputs — bounds, apportion
  inputs and credits included, so they re-settle — and the next `build()` derives
  everything again.
- **Serde carries inputs.** A serialised `Document` is its inputs; deserialising runs
  `build()`, so untrusted JSON is validated exactly like code.
- **Transforms are on inputs.** `negated`, `concat`, `split` and `segments` produce
  new inputs and re-derive (a split share additionally carries its VAT partition).

```rust
use billing::{AmountScale, Currency, Document, ErrorKind, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("service", dec!(100)))
    .build()?;

// A serialised document is its inputs; deserialising re-builds and re-checks.
let json = serde_json::to_string(&doc)?;
assert!(!json.contains("vat_total"));
let back: Document = serde_json::from_str(&json)?;
assert_eq!(back, doc);

// Setting a single-valued setting twice is refused, whatever the order.
let twice = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .default_vat(Vat::standard(Percent::new(dec!(7)))?)
    .build()
    .unwrap_err();
assert_eq!(twice.kind(), &ErrorKind::ConflictingSetting { setting: "default_vat" });
```

## Invariants

| Invariant | How it is enforced |
|---|---|
| No `f64` | `Amount<P>` is an `i64` of units; prices and quantities are exact `Decimal`s |
| Every leaf is rounded once, from its exact value | `AmountScale::reduce` on exact rationals; no intermediate precision exists |
| Aggregates are never rounded | every total is a sum of reduced leaves |
| VAT is rounded once per group | the breakdown is a group-by over `Vat` attributes |
| Rounding is sign-symmetric | four `Rounding` modes, no floor or ceiling; `negated` is exact |
| Overflow is visible | operators panic, everything else returns `Err(Overflow)`; nothing wraps |
| No silent precision loss | `parse` and `from_decimal` refuse excess digits; rounding is an explicit call |
| Splits are exact | largest remainder in `i128`; `Σ parts == total`, per-part quota |
| A price is not money | `Price` is a `Decimal`; only a line's net becomes an `Amount` |
| A conditional discount is not a discount | Skonto changes no total; `vat_correction` is a separate result |
| Direction lives in the quantity | prices are ≥ 0 (`BR-27`); credits are negative quantities |
| No domain assumptions | no default currency, scale, VAT rate, unit or cash increment |
| No header | numbers, dates, parties, type codes and text belong to `en16931` |
| `no_std`, no I/O, no `unsafe` | `#![no_std]` with `alloc`, `#![forbid(unsafe_code)]`; `just nostd` builds for a bare-metal target |
| Two runtime dependencies | `rust_decimal` and optional `serde`, checked by `just deps-check` |

## How correctness is checked

| Check | Question | How |
|---|---|---|
| Example tests | does the code do what I expect? | golden EN 16931 documents; every fixed defect is a named regression |
| Property tests | does the law hold for inputs I did not think of? | `proptest`: round-trips, split conservation and quota, sign symmetry, `negated`, `concat` |
| Fuzzing | does anything panic on hostile input? | `cargo-fuzz` on the parser, the splitter and serde (`just fuzz <target>`) |
| Doctests, site examples | is the documentation true? | README doctests; `just site-verify` runs every block on this site |
| Mutation testing | would anything fail if the code did something else? | `cargo-mutants` against a baseline of justified survivors; full sweep before a release, diff-scoped in CI |
| Citations | does the cited rule say what we claim? | every rule id is hash-locked against the pinned Schematron in `citations.lock` (`just check-citations`) |

## Comparison

| System | Language | Notes |
|---|---|---|
| **Kill Bill** | Java | a billing *platform*; this is a calculation *library* |
| **Lago**, **Orb** | various | metering and billing services; this is embeddable arithmetic |
| **Stripe Billing**, **Chargebee**, **Zuora** | SaaS | subscription lifecycle and payments; out of scope |
| `rust_decimal` | Rust | decimal arithmetic, no billing semantics — this crate's one required dependency |
| `rusty-money` | Rust | money with formatting and exchange; no tariffs, VAT breakdown or EN 16931 totals |
| `en16931` | Rust | the EN 16931 model and rules; uses amounts like these, does not compute them |

## Stability

Pre-1.0, a minor bump may break the API. Breaking and silent changes are listed in
the [changelog](https://github.com/hupe1980/billing/blob/main/CHANGELOG.md).
`ErrorKind`, `TaxCategory`, `Tariff` and a few other enums are `#[non_exhaustive]`;
`cargo-semver-checks` (`just semver`) guards against unintended breaks.
