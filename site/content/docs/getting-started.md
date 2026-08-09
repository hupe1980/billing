+++
title = "Getting started"
description = "Install billing, price a tiered rate schedule, add VAT, and assemble a document whose totals are checked rather than assumed."
weight = 10
+++

## Install

```sh
cargo add billing

# `dec!` lives in rust_decimal itself, behind the `macros` feature.
cargo add --dev rust_decimal --features macros
```

The crate is `no_std`-hostile on purpose — it allocates — but it has no other
surprises: three dependencies, no I/O, no async, no `unsafe`, MSRV 1.85.

An optional `serde` feature adds `Serialize`/`Deserialize` to every public type.
Amounts serialise as decimal **strings**, never as JSON numbers, and every type that
carries an invariant re-runs its validation on the way in.

## A first invoice

A three-tier water tariff, billed on 28.5 m³, with 10 % VAT.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let schedule = RateSchedule::graduated()
    .unit("m³")                  // the domain unit — nothing is assumed
    .currency(Currency::EUR)     // no currency is assumed either
    .band(RateBand::up_to(dec!(5),   Amount::parse("0.80000")?))
    .band(RateBand::between(dec!(5), dec!(20), Amount::parse("1.40000")?))
    .band(RateBand::over(dec!(20),   Amount::parse("2.60000")?))
    .build()?;

// → [5 m³ × 0.80 = 4.00, 15 m³ × 1.40 = 21.00, 8.5 m³ × 2.60 = 22.10]
let items = schedule.split(dec!(28.5))?;

let doc = BillingDocument::from_positions(
    DocumentMeta {
        invoice_number: "WATER-2026-07".into(),
        period_label:   "July 2026".into(),
        currency:       Currency::EUR,
        ..Default::default()
    },
    items,
    vec![FixedRateTax::new("VAT", dec!(0.10))?.boxed()],
    vec![],
)?;

assert_eq!(doc.net_total().to_string(),   "47.10000");
assert_eq!(doc.tax_total().to_string(),   "4.71000");
assert_eq!(doc.gross_total().to_string(), "51.81000");

doc.assert_valid();   // panics if any arithmetic invariant fails
```

Three things in that snippet are deliberate and worth noticing.

**`DocumentMeta` is built with `..Default::default()`.** The struct gains fields
between minor releases; struct-update syntax means that is not a breaking change for
you.

**The unit and the currency are arguments.** There is no default. A generated
unit-price label reads `XXX/m³` if you forget the currency — `XXX` is the ISO 4217
code for "no currency involved", and seeing it on a rendered invoice is a visible bug
rather than a silent mislabelling.

**`assert_valid()` is not decoration.** It re-derives the totals from the positions
and compares them exactly. On a document this constructor produced it cannot fail; its
value is on documents that were allocated, merged, deserialised, or built by hand.

## Assembling at invoice precision

The document above carries five decimals, because `28.5 × 2.60` needs them. EN 16931
allows two. Ask for them during assembly, not afterwards:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1234.567), "kWh"),
        UnitPrice::new(dec!(0.28901), "EUR/kWh"),
    ).build()?])
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert!(doc.fits_amount_scale(2));
doc.assert_valid();
```

Why it has to happen there, and what breaks if it does not, is
[E-invoicing and precision](../e-invoicing/).

## The builder, and the `PricingModel` trait

`BillingDocument::builder()` takes positions and layers directly. For anything
reusable, implement [`PricingModel`](../pricing/#the-pricingmodel-trait) instead: it separates
pricing from tax, which is how invoices are audited, and it lets a model say
*"nothing to bill, because …"* as a first-class outcome rather than as an empty
vector or an error.

```rust,ignore
let doc = BillingDocument::builder()
    .meta(meta)
    .pricing(&my_model, &usage)?   // loads positions + tax and discount layers
    .build()?;
```

## Where to go next

| If you want to | Read |
|---|---|
| understand `Amount<P>`, rounding and overflow | [Amounts and rounding](../amounts/) |
| price metered usage, seats, capacity or spot intervals | [Pricing models](../pricing/) |
| stack VAT, excise duties and commissions | [Tax and discount layers](../tax-and-discounts/) |
| emit a lawful VAT breakdown | [VAT breakdown](../vat-breakdown/) |
| map to BT-106 … BT-115 | [The totals chain](../totals/) |
| convert a gross-quoted catalogue | [Tax-inclusive pricing](../gross-pricing/) |
| split a bill across tenants | [Allocation](../allocation/) |
