+++
title = "billing — a domain-agnostic Rust invoice and usage-billing engine"
description = "Domain-agnostic billing engine for Rust: fixed-point money, tiered and time-of-use pricing, composable VAT layers, and invoice totals that satisfy EN 16931."
template = "index.html"
sort_by = "weight"
+++

<section class="hero">
<div class="wrap">

<p class="eyebrow">Rust · zero I/O · no async · no <code>unsafe</code></p>

# Invoice arithmetic a validator will accept

**billing** is a pure, domain-agnostic calculation engine. It does the hard part —
graduated pricing, compound tax layers, tax-inclusive prices, exact allocation, and
totals that still add up after you round them to two decimals — and leaves every
domain decision to your crate.

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

Every e-invoicing standard asks for two things at once, and they fight. EN 16931 caps
every monetary amount at **two decimals**, and it also requires the totals to reconcile
**exactly** at that precision.

Round at the serialiser and you fail. Three lines of `0.005` each become `0.01`,
summing to `0.03`, while the exact total `0.015` becomes `0.02` — `BR-CO-10` violated.
A net of `0.0042` with 19 % VAT gives `0.00 + 0.00 ≠ 0.01` — `BR-CO-15` violated. Both
come from ordinary inputs.

The only construction that satisfies both is to round the *leaves* and recompute every
aggregate from them. That has to happen while the document is assembled, not on the way
out — by the time you hold a finished document, the aggregates already exist.

[How the precision model works →](docs/e-invoicing/)

</div>
<div>

```rust
use billing::prelude::*;
use rust_decimal::dec;

// 1234.567 kWh × 0.28901 EUR/kWh = 356.80221 — five decimals.
let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)   // two decimals, everywhere
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1234.567), "kWh"),
        UnitPrice::new(dec!(0.28901), "EUR/kWh"),
    ).build()?])
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert_eq!(doc.net_total(),   Amount::parse("356.80000")?);
assert_eq!(doc.tax_total(),   Amount::parse("67.79000")?);
assert_eq!(doc.gross_total(), Amount::parse("424.59000")?);

assert!(doc.fits_amount_scale(2));  // emittable as EN 16931
doc.assert_valid();                 // and the identities still hold
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

`Amount<P>` is `i64 × 10⁻ᴾ` with the precision in the type. No `f64` anywhere in the
arithmetic path, overflow is never silent, and rounding is always an explicit argument.

[Amounts and rounding →](docs/amounts/)

</article>
<article>

### Pricing models

Graduated, volume, block and capacity rate schedules; time-of-use bands; spot
pricing; capacity-keyed rate tables; six usage aggregators.

[Pricing models →](docs/pricing/)

</article>
<article>

### Tax and discount layers

Ordered, composable, compounding. Each layer sees every prior layer in its base — which
is exactly what a per-unit excise sitting inside a VAT base requires.

[Tax layers →](docs/tax-and-discounts/)

</article>
<article>

### VAT breakdown (BG-23)

Per-rate taxable base and tax, the ten UNCL 5305 categories with their exemption rules,
and per-position attribution checked back against the breakdown.

[VAT breakdown →](docs/vat-breakdown/)

</article>
<article>

### Tax-inclusive prices

Consumer catalogues are quoted gross; EN 16931 is net. The conversion is the VAT
fraction `r/(1+r)`, not `× r` — and that is the one people get wrong.

[Gross to net →](docs/gross-pricing/)

</article>
<article>

### Exact allocation

Split a document across N recipients, or a raw quantity by fractions, with
largest-remainder correction. No cent is created or lost.

[Allocation →](docs/allocation/)

</article>
</div>
</div>
</section>

<section class="wrap">

## Designed to be checkable

<div class="grid grid-3">
<article>

### Self-validating documents

`validate()` re-derives sixteen invariants from the positions, exactly and with no
tolerance. Every constructor runs it, and so does deserialisation — so neither an
awkward combination of layers nor untrusted JSON can produce a document that fails
its own checks.

</article>
<article>

### Rules, cited

Every EN 16931 and Peppol rule the engine enforces is named in the source beside the
code that enforces it, so you can check it against the published Schematron instead of
trusting a summary.

</article>
<article>

### Mutation-tested

Property tests state the algebraic laws; `cargo-mutants` then rewrites the code and
reports every rewrite the suite still accepts. Surviving mutants are recorded with the
reason each one is tolerated.

</article>
</div>

[Design invariants and how correctness is checked →](docs/design/)

</section>

<section class="band">
<div class="wrap narrow">

## What it is not

It is a *library*, not a platform. No subscription lifecycle, no event ingestion, no
payment provider, no database, no scheduler. It does no I/O, spawns nothing, forbids
`unsafe`, and has three dependencies.

It also does not serialise XML. UBL, CII, XRechnung and ZUGFeRD need an XML writer, the
UN/ECE and UNTDID code lists, and Schematron conformance fixtures — none of which
belongs in a pricing engine, and all of which move on a different release cadence. Those
live in [`en16931`](https://crates.io/crates/en16931) and
[`en16931-formats`](https://crates.io/crates/en16931-formats), which are separate
dependencies rather than a framework.

[Where this crate stops →](docs/e-invoicing/)

</div>
</section>
