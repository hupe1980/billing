+++
title = "Allowances, charges and levies"
description = "Line allowances and charges (BG-27/28), document allowances and charges (BG-20/21), apportioning a discount across VAT groups, and per-unit levies as ordinary lines."
weight = 50
+++

Every amount is a **line**, a **line allowance or charge**, or a **document allowance
or charge**. VAT is an [attribute](../vat-breakdown/), and a levy is a line.

## Line allowances and charges (BG-27 / BG-28)

A `LineAdjustment` changes one line's net (BT-131). Its amount is an exact `Decimal`,
reduced **once** at the document's scale as a leaf of its own:

> BT-131 = reduce(quantity × price ÷ base) + Σ reduce(charges) − Σ reduce(allowances)

Optionally it states the base (BT-137/142) and percentage (BT-138/143) it was computed
from, a reason (BT-139/144) and a reason code (BT-140/145).

```rust
use billing::{Amount, AmountScale, Currency, Document, ErrorKind, Line, LineAdjustment, Location, Percent, Price, Quantity, UnitPrice};
use rust_decimal::dec;

let energy = || Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30))));

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(energy().with_adjustment(
        LineAdjustment::allowance(dec!(30)).with_basis(dec!(300), Percent::new(dec!(10))).with_reason("loyalty"),
    ))
    .build()?;
assert_eq!(doc.line_net(0), Amount::parse("270.00")?);
assert_eq!(doc.line_adjustment_amounts(0), [Amount::parse("30.00")?]);

// A stated basis must reproduce the amount within ±0.02 (PEPPOL-EN16931-R040).
let err = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(energy().with_adjustment(LineAdjustment::allowance(dec!(31)).with_basis(dec!(300), Percent::new(dec!(10)))))
    .build()
    .unwrap_err();
assert_eq!(err.kind(), &ErrorKind::AdjustmentBasis);
assert_eq!(err.location(), Some(&Location::LineAdjustment { line: 0, index: 0 }));
assert_eq!(err.rule(), Some("PEPPOL-EN16931-R040"));
```

The reason-or-code requirement (`BR-42`, `BR-44`) is the format layer's check.

## Document allowances and charges (BG-20 / BG-21)

A `DocumentAdjustment` changes the document's BT-107 or BT-108 and, through its own
VAT treatment (BT-95/96, BT-102/103), the base of one VAT group. It takes the
document's default VAT unless it carries its own:

```rust
use billing::{Amount, AmountScale, Currency, Document, DocumentAdjustment, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("service", dec!(500)))
    .adjustment(DocumentAdjustment::charge("express delivery", dec!(25)))
    .adjustment(DocumentAdjustment::allowance("goodwill", dec!(15)).with_reason_code("95"))
    .build()?;

assert_eq!(doc.charge_total(), Amount::parse("25.00")?);     // BT-108
assert_eq!(doc.allowance_total(), Amount::parse("15.00")?);  // BT-107
assert_eq!(doc.tax_exclusive_total(), Amount::parse("510.00")?);
assert_eq!(doc.vat_total(), Amount::parse("96.90")?);        // 510 × 19 %
```

`BR-32` and `BR-37` require the VAT category on every document allowance and charge;
in a VAT document `build()` refuses one without (`ErrorKind::MissingVat` at
`Location::Adjustment(i)`).

## `apportion`: one discount per VAT group {#apportion}

A 10 % discount on 400 @ 19 % and 100 @ 7 % is **two** allowances — 40 @ 19 % and
10 @ 7 % — because each lowers its own group's base. `apportion` builds one
`DocumentAdjustment` per (category, rate) group:

- `Adjust::Percent(p)`: each group's amount is `round(base × p / 100)` once, with the
  basis `(base, p)` stated (so it satisfies `PEPPOL-EN16931-R040`);
- `Adjust::Fixed(f)`: `f` is split by the group bases with [`split`](../allocation/) —
  exact, largest remainder.

On the builder it applies to the whole document:

```rust
use billing::{Adjust, AdjustmentKind, Amount, AmountScale, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("full rate", dec!(400)))
    .line(Line::fixed("reduced rate", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
    .apportion(AdjustmentKind::Allowance, "10 % discount", Adjust::Percent(Percent::new(dec!(10))))
    .build()?;

assert_eq!(doc.adjustments().len(), 2);
assert_eq!(doc.adjustments()[0].basis(), Some((dec!(400), Percent::new(dec!(10)))));
assert_eq!(doc.allowance_total(), Amount::parse("50.00")?);
assert_eq!(doc.vat_total(), Amount::parse("74.70")?); // 360 × 19 % + 90 × 7 %, not 83.00 − 50 × 19 %
```

The document keeps the `apportion` input, so `into_builder()` and serde re-apply it.

For a discount on **some** lines only, call the free function with a filter and add
its result as ordinary adjustments:

```rust
use billing::{Adjust, AdjustmentKind, Amount, AmountScale, Currency, Document, Line, Percent, Vat, apportion};
use rust_decimal::dec;

let vat = Vat::standard(Percent::new(dec!(19)))?;
let lines = [
    Line::fixed("hardware", dec!(800)).with_vat(vat.clone()).with_label("discountable", "yes"),
    Line::fixed("support", dec!(200)).with_vat(vat.clone()),
];
let scale = AmountScale::EN16931;
let rebate = apportion(
    AdjustmentKind::Allowance,
    "hardware rebate",
    Adjust::Fixed(dec!(50)),
    &lines,
    |l| l.label("discountable").is_some(),
    scale,
)?;
let doc = Document::builder(Currency::EUR, scale).lines(lines).adjustments(rebate).build()?;
assert_eq!(doc.tax_exclusive_total(), Amount::parse("950.00")?);
```

## Per-unit levies

Electricity tax, concession fees: a rate per unit supplied. `per_unit_levy` sums the
selected lines' quantity and returns **one ordinary line**, taxed like any other (the
levy is part of the VAT base, §10 UStG):

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Price, Quantity, UnitPrice, Vat, per_unit_levy};
use rust_decimal::dec;

let energy = [Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30))))];
let stromsteuer = per_unit_levy(
    "Stromsteuer",
    &energy,
    |_| true,
    UnitPrice::new(Price::new(dec!(0.0205))).with_unit("KWH"),
    "KWH",
)?;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .lines(energy)
    .line(stromsteuer)
    .build()?;
assert_eq!(doc.line_net(1), Amount::parse("20.50")?);
assert_eq!(doc.vat_total(), Amount::parse("60.90")?); // 320.50 × 19 %, not 300 × 19 % = 57.00
```

A selected line in another unit is `ErrorKind::PriceBaseUnit`. Whether the VAT base
includes a levy is decided by the VAT treatment of the levy line.
