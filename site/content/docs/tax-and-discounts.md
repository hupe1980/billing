+++
title = "Tax and discount layers"
description = "Ordered, composable overlays: fixed-rate VAT, per-unit excise duties, percentage commissions and discounts — and why the order they are declared in is the compound-tax base."
weight = 40
+++

## Layers, in order

A `TaxLayer` receives every position accumulated so far — the original lines, the
discounts, **and the output of every earlier tax layer** — and returns one position.
That accumulation is not incidental; it is what a compound tax actually is.

German electricity is the standard example. Stromsteuer is charged per kWh, and MwSt
is charged on the net *including* Stromsteuer. Declaring them in the wrong order
produces a plausible invoice that is wrong by 19 % of the levy:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ).build()?])
    // Stromsteuer first — so it is inside the MwSt base.
    .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

// Levy = 1000 × 0.02050 = 20.50; MwSt = (300.00 + 20.50) × 0.19 = 60.895
assert_eq!(doc.charge_total()?, Amount::parse("20.50000")?);   // BT-108
assert_eq!(doc.vat_total()?,    Amount::parse("60.89500")?);   // BT-110
```

Discounts are different, and deliberately so: **every `DiscountLayer` sees the
original net positions only**, never the output of a preceding discount. Two stacked
10 % discounts therefore take 10 % + 10 % of the same base (20 % total), not 10 % of
90 % (19 %). Compounding discounts is a commercial decision that varies by contract,
so the engine takes the interpretation you can compose out of, rather than the one
you would have to unpick.

## The built-in layers

| Type | Trait | Models | EN 16931 |
|---|---|---|---|
| `FixedRateTax` | `TaxLayer` | VAT / MwSt / GST at a rate | value added tax, **BG-23** |
| `PerUnitLevy` | `TaxLayer` | excise duty, environmental levy, per-kWh charge | document level charge, **BG-21** |
| `PercentageCharge` | `TaxLayer` | platform fee, marketplace commission, surcharge | document level charge, **BG-21** |
| `PercentageDiscount` | `DiscountLayer` | percentage rebate | document level allowance, **BG-20** |
| `FixedDiscount` | `DiscountLayer` | fixed-amount rebate | document level allowance, **BG-20** |

That third column is the one to read carefully. A commission is **not** tax. In
EN 16931 it is a charge, it sits *inside* the taxable base, and it feeds BT-108 →
BT-109 rather than BT-110. Putting it in BT-110 breaks `BR-CO-14` on every document
that has one. See [The totals chain](../totals/).

Only a layer that reports a VAT breakdown entry is value added tax, and the engine
decides that from the layer's own `breakdown()` return value rather than from a
guess — so a third-party `TaxLayer` is classified as accurately as a built-in one.

## Targeting a subset

Every layer takes an optional tag filter. Tag the positions, then give each layer the
tag it applies to:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![
        LineItem::fixed("Beratung", Amount::parse("400.00000")?).tag("full").build()?,
        LineItem::fixed("Fachbuch", Amount::parse("100.00000")?).tag("reduced").build()?,
    ])
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("full").boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.07))?.with_tag("reduced").boxed())
    .build()?;

assert_eq!(doc.tax_breakdown().len(), 2);
doc.verify_vat_attribution()?;
```

A tag filter is the *whole* definition of a layer's base: a position without the tag
is excluded from the tax and from the VAT attribution alike, because both route
through the same predicate. They cannot describe different sets.

Two VAT layers claiming the same position means that position is taxed twice, and
`BR-S-08` cannot hold for either group. Assembly rejects it with a `LayerError`
instead of producing a document no validator accepts. So does a layer that would
charge VAT on another layer's VAT — a VAT breakdown group is not a line, an allowance
or a charge, so it cannot appear in any other group's taxable base, and EN 16931 has
no way to express it at all.

## Per-unit levies

`PerUnitLevy` sums the **quantity** of every matching position and multiplies by a
per-unit rate. Matching is by unit label, optionally narrowed by a tag.

Two details are load-bearing:

- It counts `Sign::Debit` positions only. Excise duty applies to consumption, not to
  a feed-in credit. It uses the *sign*, not `net_amount < 0`, so a debit at a negative
  spot price is still consumption.
- It excludes anything already tagged `tax`. A levy emits a debit line carrying a
  quantity in its own unit, so a second levy on the same unit would otherwise count
  that line as consumption and double its base. Stromsteuer + Konzessionsabgabe, both
  in ct/kWh, is the ordinary German stack where that over-billed the second levy by
  100 %.

A custom `TaxLayer` that emits a quantity should tag its output `tax` for the same
reason.

## Neither percentage layer reverses its own direction

`PercentageCharge::new` refuses a negative rate — a negative charge is a discount and
belongs in a `DiscountLayer`. The same result is reachable through a negative **base**,
and is refused the same way: both percentage layers clamp a base that nets out below
zero.

```rust
use billing::prelude::*;
use rust_decimal::dec;

// 1000 kWh at −0.01 EUR/kWh — a debit position at a negative spot price.
let epex = || vec![LineItem::for_usage(
    "EPEX Spot (negativ)",
    Quantity::new(dec!(1000), "kWh"),
    UnitPrice::new(dec!(-0.01), "EUR/kWh"),
).build().unwrap()];

assert_eq!(
    PercentageCharge::new("Commission", dec!(0.05))?.compute(&epex())?.net_amount,
    Amount::<5>::ZERO
);
assert_eq!(
    PercentageDiscount::new("Treuerabatt", dec!(0.05))?.compute(&epex())?.net_amount,
    Amount::<5>::ZERO
);
```

Without the clamp each would move the total the wrong way for what it claims to be: a
"charge" that increases what the supplier owes the customer, and a "discount" that
does the same. A debit at a negative price still **contributes** to the base and
reduces it — that is what separates it from a `Sign::Credit` position, which is
excluded outright — and only a base that ends up below zero is clamped.

## Allowance and charge detail

A document level allowance or charge carries more than an amount. `PercentageCharge`
and `PercentageDiscount` fill in the EN 16931 detail automatically, because only they
know the base the percentage was taken of:

| | Allowance (BG-20) | Charge (BG-21) |
|---|---|---|
| reason code | BT-98 (UNCL 5189) | BT-105 (UNCL 7161) |
| base amount | BT-93 | BT-100 |
| percentage | BT-94 | BT-101 |

The base and the percentage are **not** free annotation: Peppol recomputes them.

> `[PEPPOL-EN16931-R040]` Allowance/charge amount must equal base amount ×
> percentage/100 if base amount and percentage exists — **fatal**, ±0.02.
>
> `[PEPPOL-EN16931-R041]` / `[R042]` — each is required when the other is present.

So the pair is a claim, and any operation that changes the amount has to keep it
true. `LineItem::scaled` (proration, allocation) rescales the base with the amount;
`reverse()` negates both; and where the amount moves on its own — a min/max clamp, or
allocation's penny correction — the basis is **dropped**, keeping the reason code,
because stating none is always valid.

The position's own description serves as the free-text reason (BT-97 / BT-104), so
`BR-CO-21` and `BR-CO-22` are satisfied whether or not you set a code.

## Writing your own layer

Implement `TaxLayer` and, if it is value added tax, override `breakdown()`. Two rules
of thumb:

**Keep `covers()` in lockstep with the filter `compute()` applies.** `covers` is what
attributes a VAT category to each individual position, and `BR-S-08` checks the
breakdown against exactly those positions. If they disagree, the attribution describes
a base that was never taxed.

**Compute `breakdown()` and `compute()` from the same slice.** Both receive it, so the
reported taxable base can be made identical to the base the tax was charged on — the
built-in layers route both through one private predicate so they cannot drift.
