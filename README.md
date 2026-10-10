# billing

[![Crates.io](https://img.shields.io/crates/v/billing.svg)](https://crates.io/crates/billing)
[![Docs.rs](https://img.shields.io/docsrs/billing)](https://docs.rs/billing)
[![CI](https://github.com/hupe1980/billing/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/billing/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)

**[Guide](https://hupe1980.github.io/billing) · [API reference](https://docs.rs/billing) · [Changelog](https://github.com/hupe1980/billing/blob/main/CHANGELOG.md)**

`billing` does the arithmetic of an EN 16931 invoice exactly. You give it lines,
allowances, charges, VAT treatments and settlement inputs; it derives every total and
the VAT breakdown once, each amount rounded once at the interchange scale. Money is a
fixed-point `i64`, prices are exact decimals, and every rounding is sign-symmetric.

It is arithmetic only: no invoice header, no code lists, no XML.

## Install

```toml
[dependencies]
billing = "0.16"
# billing = { version = "0.16", features = ["serde"] }
rust_decimal = { version = "1.43", features = ["macros"] } # for `dec!`
```

## Quick start

```rust
use billing::{Adjust, AdjustmentKind, AmountScale, Currency, Document, Percent, Price, Tariff, Tier, Tiers, Vat};
use rust_decimal::dec;

let tiers = Tiers::new([Tier::up_to(dec!(100), Price::new(dec!(0.30))), Tier::above(Price::new(dec!(0.25)))])?;
let pieces = Tariff::Graduated(tiers).rate(dec!(150))?; // 100 × 0.30 + 50 × 0.25
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .lines(pieces.into_iter().flat_map(|p| p.into_lines("energy", "KWH")))
    .apportion(AdjustmentKind::Allowance, "10 % loyalty", Adjust::Percent(Percent::new(dec!(10))))
    .build()?;
assert_eq!(format!("{:.2} {:.2} {:.2}", doc.tax_exclusive_total(), doc.vat_total(), doc.payable()), "38.25 7.27 45.52");
# Ok::<(), billing::Error>(())
```

150 kWh rate to 42.50; the 10 % allowance (4.25) lowers the VAT base, so VAT is
38.25 × 19 % = 7.2675, rounded once to 7.27.

## Derived once, per VAT group

A `Document` is built from inputs only; nothing derived can be set by hand. VAT is an
attribute of each line and adjustment. Members are grouped by (category, rate), and
each group's tax is `round(base × rate / 100)`, once. A discount across mixed rates
lowers each group's base:

```rust
use billing::{Adjust, AdjustmentKind, AmountScale, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("full rate", dec!(400)))
    .line(Line::fixed("reduced rate", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
    .apportion(AdjustmentKind::Allowance, "10 % discount", Adjust::Percent(Percent::new(dec!(10))))
    .build()?;
assert_eq!(doc.vat_groups().len(), 2);
assert_eq!(doc.vat_total().to_string(), "74.70000"); // 360 × 19 % + 90 × 7 %
assert_eq!(doc.negated().payable(), -doc.payable()); // a credit note is the exact negative
# Ok::<(), billing::Error>(())
```

A price is never negative (BR-27): a refund is a line with a negative quantity.
Builder call order never changes the result.

## Splitting exactly

`split` is a largest-remainder partition on integers; `Document::split` turns one
document into complete shares that add up to the parent exactly:

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Vat, split};
use rust_decimal::dec;

assert_eq!(split(dec!(100.00), &[dec!(1); 3], 2)?, [dec!(33.34), dec!(33.33), dec!(33.33)]);

let water = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("water", dec!(47.10)))
    .build()?;
let shares = water.split(&[dec!(40), dec!(35), dec!(25)])?;
assert_eq!(shares.iter().map(Document::tax_inclusive_total).sum::<Amount>(), water.tax_inclusive_total());
# Ok::<(), billing::Error>(())
```

## Features

- [Amounts](https://hupe1980.github.io/billing/docs/amounts/): `Amount<P>` (fixed-point `i64`), `Price`, `AmountScale`, four sign-symmetric `Rounding` modes, strict parsing.
- [Pricing](https://hupe1980.github.io/billing/docs/pricing/): `Tariff` (per unit, graduated, volume, package), `rate_series`, `rate_by_key`, `TransactionFee`, progressive billing with `Billed`.
- [Metering](https://hupe1980.github.io/billing/docs/metering/): `Aggregation`, `Meter`, `UnitConversion`, exact `Usage` (averages stay fractions), `nearest_rank` percentiles.
- [VAT breakdown](https://hupe1980.github.io/billing/docs/vat-breakdown/): `Vat` constructors that enforce each category's rules; BG-23 groups.
- [Tax and discounts](https://hupe1980.github.io/billing/docs/tax-and-discounts/): line and document allowances/charges, `apportion` across VAT groups, `per_unit_levy`.
- [Gross pricing](https://hupe1980.github.io/billing/docs/gross-pricing/): `split_gross` splits a VAT-inclusive price exactly.
- [Settlement](https://hupe1980.github.io/billing/docs/settlement/): `Payment`/`Advance` (BT-113), `Credit`, `CashRounding` (BT-114), `VatAccounting` (BT-6/BT-111), `residual_vat`.
- [Payment terms](https://hupe1980.github.io/billing/docs/payment-terms/): `EarlyPaymentDiscount` (Skonto, with VAT correction), `late_payment_interest`.
- [Corrections](https://hupe1980.github.io/billing/docs/corrections/): `Document::negated`, `Document::concat`, `Bound` minimums and caps.
- [Allocation](https://hupe1980.github.io/billing/docs/allocation/): `split`, `split_matrix`, `Document::split`, `Line::segments`.
- [Totals](https://hupe1980.github.io/billing/docs/totals/): BT-106 … BT-115 by the BR-CO identities.
- [Errors](https://hupe1980.github.io/billing/docs/errors/): one `Error` with `kind()`, `location()` and the enforced `rule()`.
- [E-invoicing](https://hupe1980.github.io/billing/docs/e-invoicing/): the EN 16931 rules covered and how to hand a document to a format.
- [Design](https://hupe1980.github.io/billing/docs/design/) and [Getting started](https://hupe1980.github.io/billing/docs/getting-started/).

Full API: [docs.rs/billing](https://docs.rs/billing).

## Guarantees

- **Rounded once.** Each leaf is reduced once from its exact value; each VAT group's
  tax once from its exact base × rate. Totals are exact sums of reduced amounts.
- **Sign-symmetric.** `round(−x) == −round(x)` for every mode, so `negated` is exact
  and a credit rounds like its debit.
- **Splits partition.** `split`, `split_matrix`, `Document::split`, `Line::segments`
  and fixed `apportion` conserve their total exactly; ties are deterministic.
- **Derived, not stored.** A `Document` exists only through `build`; serde
  deserialises its inputs and builds again, so no stored total can disagree.
- **Order-free.** Builder call order does not matter, except between credits (their
  order is their priority).
- **Exact or refused.** Overflow, excess precision and malformed input are errors,
  never approximations. Nothing panics on bad input except `Amount`'s `+`/`-`/`Sum`
  on `i64` overflow and indexed accessors out of range; `checked_*` never panics.
- **Rules by construction.** The totals identities (BR-CO-10 … BR-CO-16), group tax
  and base (BR-CO-17, BR-S-08), amounts fitting the scale (BR-DEC-*) and
  PEPPOL-EN16931-R120 at scale ≥ 2 hold for every built document; `tests/rules.rs`
  checks them. Category rules (BR-S-05 …, BR-O-11, BR-B-02) are refusals.

## What it is not

- **The invoice header** (number, dates, type code, parties, periods, terms text) and
  **code lists** (ISO 4217 and friends): see [`en16931`](https://crates.io/crates/en16931).
- **Serialisation** to UBL, CII, XRechnung, ZUGFeRD / Factur-X: see
  [`en16931-formats`](https://crates.io/crates/en16931-formats).
- **Dates and calendars**: weights and day counts are numbers you pass in.

Neither crate is a dependency of `billing`.

## MSRV, `no_std` and features

- **MSRV** Rust 1.85 (edition 2024).
- **`#![no_std]`** with `alloc`; no I/O.
- **Features**: `serde` (off by default). Amounts and decimals serialise as strings;
  JSON numbers are refused. A `Document` serialises its inputs only.
- **Dependencies**: `rust_decimal` ≥ 1.43 and optional `serde`.

## Migrating from 0.15

0.16 is a breaking release: see the
[migration guide](https://hupe1980.github.io/billing/docs/migrating/) and the
[CHANGELOG](https://github.com/hupe1980/billing/blob/main/CHANGELOG.md).

## License

Licensed under either of [Apache License, Version 2.0](https://github.com/hupe1980/billing/blob/main/LICENSE-APACHE) or
[MIT license](https://github.com/hupe1980/billing/blob/main/LICENSE-MIT) at your option.
