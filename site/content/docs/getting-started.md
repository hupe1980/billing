+++
title = "Getting started"
description = "Install billing, rate a tiered water tariff, attach VAT, and read every EN 16931 total from a document that derived them once."
weight = 10
+++

## Install

```sh
cargo add billing

# `dec!` lives in rust_decimal itself, behind the `macros` feature.
cargo add --dev rust_decimal --features macros
```

`#![no_std]` with `alloc`, no `unsafe`, no I/O; one runtime dependency, `rust_decimal`
(re-exported as `billing::Decimal`). MSRV 1.85. The optional `serde` feature
serialises decimals as strings, and deserialising a `Document` re-runs `build()`.

## A first invoice

A three-tier water tariff, billed on 28.5 m³, with 7 % VAT.

```rust
use billing::{Amount, AmountScale, Currency, Document, Percent, Price, Tariff, Tier, Tiers, Vat};
use rust_decimal::dec;

let tariff = Tariff::Graduated(Tiers::new([
    Tier::up_to(dec!(5), Price::new(dec!(0.80))),
    Tier::up_to(dec!(20), Price::new(dec!(1.40))),
    Tier::above(Price::new(dec!(2.60))),
])?);

// 5 m³ × 0.80 + 15 m³ × 1.40 + 8.5 m³ × 2.60, one line per tier.
let lines = tariff
    .rate(dec!(28.5))?
    .into_iter()
    .flat_map(|piece| piece.into_lines("water", "MTQ"));

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(7)))?)
    .lines(lines)
    .build()?;

assert_eq!(doc.lines().len(), 3);
assert_eq!(doc.line_net(2), Amount::parse("22.10")?);
assert_eq!(doc.line_total(), Amount::parse("47.10")?);   // BT-106
assert_eq!(doc.vat_total(), Amount::parse("3.30")?);     // BT-110: 47.10 × 7 % = 3.297
assert_eq!(doc.payable(), Amount::parse("50.40")?);      // BT-115
```

- **Currency and scale are required.** `AmountScale::EN16931` is two decimals, half
  away from zero.
- **The unit is a code you choose** (`MTQ`, UN/ECE Rec. 20); the engine only checks a
  stated price unit matches it (`PEPPOL-EN16931-R130`).
- **VAT is an attribute.** `default_vat` applies wherever none is set; the breakdown
  rounds once per group.
- **Every check runs in `build()`**, which returns an [`Error`](../errors/) naming the
  rule and the line.

## Reading the result

Every total is an accessor returning an `Amount` (an `i64` count of 10⁻⁵) on the
document's scale; [the totals chain](../totals/) maps each to its business term.

```rust
use billing::{AmountScale, Currency, Document, DocumentAdjustment, Line, Percent, Price, Quantity, UnitPrice, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30)))))
    .adjustment(DocumentAdjustment::allowance("loyalty", dec!(10)))
    .build()?;

assert_eq!(doc.line_total().to_string(), "300.00000");          // BT-106
assert_eq!(doc.allowance_total().to_string(), "10.00000");      // BT-107
assert_eq!(doc.tax_exclusive_total().to_string(), "290.00000"); // BT-109
assert_eq!(doc.vat_total().to_string(), "55.10000");            // BT-110
assert_eq!(doc.tax_inclusive_total().to_string(), "345.10000"); // BT-112
assert_eq!(format!("{:.2}", doc.payable()), "345.10");          // BT-115, as emitted
```

An `Amount<5>` prints five decimals; `{:.2}` only drops zeros here, since every
derived amount already fits the scale.

## Where to go next

| If you want to | Read |
|---|---|
| understand `Amount<P>`, `Price`, rounding and overflow | [Money and rounding](../amounts/) |
| rate metered usage, spot intervals, HT/NT or a 95th percentile | [Pricing](../pricing/) |
| choose VAT categories and exemption reasons | [VAT](../vat-breakdown/) |
| invoice from gross (consumer) prices | [Gross prices](../gross-pricing/) |
| add discounts, surcharges or a per-unit levy | [Allowances, charges and levies](../tax-and-discounts/) |
| map to BT-106 … BT-115 | [The totals chain](../totals/) |
| split a bill across tenants or prorate a fee | [Splitting](../allocation/) |
| issue a credit, merge documents, cap or floor a total | [Corrections and bounds](../corrections/) |
| deduct advances, cash-round, report VAT in another currency | [Settlement](../settlement/) |
| compute Skonto or late-payment interest | [Payment terms](../payment-terms/) |
| handle errors | [Errors](../errors/) |
| upgrade from 0.15 | [Migrating from 0.15](../migrating/) |
