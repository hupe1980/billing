+++
title = "billing — EN 16931 invoice arithmetic for Rust"
description = "EN 16931 invoice arithmetic for Rust: fixed-point money, sign-symmetric rounding, exact largest-remainder splits, tariffs, VAT derived by grouping, and documents whose totals are derived once from their inputs."
template = "index.html"
sort_by = "weight"
+++

<section class="hero">
<div class="wrap">

<p class="eyebrow">Rust · no_std · zero I/O · no <code>unsafe</code></p>

# Invoice arithmetic a validator will accept

**billing** computes the numbers on an EN 16931 invoice — line nets, allowances and
charges, the VAT breakdown, the totals chain, payments and cash rounding. You state
the inputs; every total is **derived once**, at the document's scale.

<p class="cta">
<a class="btn btn-primary" href="docs/getting-started/">Get started</a>
<a class="btn" href="https://docs.rs/billing" rel="noopener">API reference</a>
<a class="btn btn-ghost" href="https://github.com/hupe1980/billing" rel="noopener">GitHub</a>
</p>

<p class="install"><code>cargo add billing</code></p>

</div>
</section>

<section class="wrap split-section">
<div class="split">
<div>

## The problem it solves

EN 16931 caps every amount at **two decimals** and requires the totals to reconcile
**exactly** at that precision. Rounding at the serialiser breaks that (`BR-CO-10`,
`BR-CO-15`). `billing` rounds each leaf once and derives every total from the rounded
leaves.

[How the precision model works →](docs/e-invoicing/)

</div>
<div>

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Price, Quantity, UnitPrice, Vat};
use rust_decimal::dec;

// 1234.567 kWh × 0.28901 EUR/kWh = 356.80221 — five decimals.
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::new(
        "Arbeit",
        Quantity::new(dec!(1234.567), "KWH"),
        UnitPrice::new(Price::new(dec!(0.28901))),
    ))
    .build()?;

assert_eq!(doc.line_net(0), Amount::parse("356.80")?); // BT-131, rounded once
assert_eq!(doc.vat_total(), Amount::parse("67.79")?);  // BT-110: 356.80 × 19 %
assert_eq!(doc.payable(), Amount::parse("424.59")?);   // BT-115
```

</div>
</div>
</section>

<section class="band">
<div class="wrap">

## What is in the box

<div class="grid">
<article>

### Fixed-point money

`Amount<P>` is an `i64` count of `10⁻ᴾ`, with four sign-symmetric rounding modes and
typed `Percent`s. Overflow never wraps.

[Money and rounding →](docs/amounts/)

</article>
<article>

### Metering and tariffs

Ten aggregations with exact averages; per-unit, graduated, volume and package
tariffs; spot series, a tariff per key, transaction fees, progressive billing and
95th percentiles.

[Pricing →](docs/pricing/)

</article>
<article>

### VAT as an attribute

Every line and adjustment carries a `Vat`; the breakdown (BG-23) is a group-by with
the tax rounded once per group.

[VAT →](docs/vat-breakdown/)

</article>
<article>

### Gross prices

`split_gross` turns a shelf price into net and tax once; the halves re-add
exactly.

[Gross prices →](docs/gross-pricing/)

</article>
<article>

### Exact splitting

Largest remainder in integer arithmetic, down to `Document::split`. No cent is
created or lost.

[Splitting →](docs/allocation/)

</article>
<article>

### Corrections and settlement

`negated`, `concat`, minimum and maximum bounds, advances, customer credits, cash
rounding, Skonto and late-payment interest.

[Settlement →](docs/settlement/)

</article>
</div>
</div>
</section>

<section class="wrap">

## Designed to be checkable

<div class="grid grid-3">
<article>

### Derived, never stored

A `Document` exists only through `build()`; every accessor is infallible and there
is no `validate()`.

</article>
<article>

### Rules, cited

Every error says why, where and which EN 16931 or Peppol rule it enforces; each rule
id is hash-locked against the pinned Schematron.

</article>
<article>

### Mutation-tested and fuzzed

Property tests state the algebraic laws, fuzzers attack the parser, splitter and
serde, and `cargo-mutants` checks the suite.

</article>
</div>

[Design and how correctness is checked →](docs/design/)

</section>

<section class="band">
<div class="wrap narrow">

## What it is not

A library of arithmetic — not a billing platform, not an invoice model. Invoice
numbers, dates, parties, type codes and XML belong to
[`en16931`](https://crates.io/crates/en16931) and
[`en16931-formats`](https://crates.io/crates/en16931-formats).

[Where this crate stops →](docs/e-invoicing/)

</div>
</section>
