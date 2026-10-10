+++
title = "The totals chain"
description = "BT-106 to BT-115 mapped onto Document's accessors, the order they are derived in, and which EN 16931 rules therefore hold by construction rather than by a check."
weight = 60
+++

## The chain

`DocumentBuilder::build` derives the EN 16931 totals once, in the standard's order;
`Document` exposes each as an infallible accessor:

| Term | Meaning | Accessor | Derived as | Rule |
|---|---|---|---|---|
| BT-131 | line net amount | `line_net(i)` | reduce(q × p ÷ base) + Σ reduce(charges) − Σ reduce(allowances) | — |
| **BT-106** | sum of line net amounts | `line_total()` | Σ BT-131 | `BR-CO-10` |
| **BT-107** | sum of document allowances | `allowance_total()` | Σ BT-92 | `BR-CO-11` |
| **BT-108** | sum of document charges | `charge_total()` | Σ BT-99 | `BR-CO-12` |
| **BT-109** | total without VAT | `tax_exclusive_total()` | BT-106 − BT-107 + BT-108 | `BR-CO-13` |
| BT-116 / BT-117 | VAT group base and tax | `vat_groups()` | Σ members; reduce(base × rate ÷ 100) | `BR-CO-17` |
| **BT-110** | total VAT | `vat_total()` | Σ BT-117 | `BR-CO-14` |
| **BT-111** | VAT in the accounting currency | `vat_accounting_total()` | reduce(BT-110 × rate) | — |
| **BT-112** | total with VAT | `tax_inclusive_total()` | BT-109 + BT-110 | `BR-CO-15` |
| **BT-113** | paid amount | `prepaid()` | Σ payment gross + after-tax credits drawn | — |
| **BT-114** | rounding amount | `rounding()` | cash rounding of BT-112 − BT-113 | — |
| **BT-115** | amount due | `payable()` | BT-112 − BT-113 + BT-114 | `BR-CO-16` |

"reduce" is one rounding of an exact value to the document's `AmountScale`. Leaves
are reduced once; aggregates are sums of reduced leaves, never rounded themselves.

```rust
use billing::{AmountScale, Currency, Document, DocumentAdjustment, Line, LineAdjustment, Percent, Price, Quantity, UnitPrice, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(
        Line::new("energy", Quantity::new(dec!(1234.567), "KWH"), UnitPrice::new(Price::new(dec!(0.28901))))
            .with_adjustment(LineAdjustment::charge(dec!(4.995))),
    )
    .line(Line::fixed("meter", dec!(8.333)))
    .adjustment(DocumentAdjustment::allowance("bonus", dec!(10.005)))
    .adjustment(DocumentAdjustment::charge("paper invoice", dec!(1.50)))
    .build()?;

let s = |a: billing::Amount| format!("{a:.2}");
assert_eq!(s(doc.line_net(0)), "361.80");          // 356.80 + 5.00, each reduced on its own
assert_eq!(s(doc.line_net(1)), "8.33");
assert_eq!(s(doc.line_total()), "370.13");         // BT-106
assert_eq!(s(doc.allowance_total()), "10.01");     // BT-107
assert_eq!(s(doc.charge_total()), "1.50");         // BT-108
assert_eq!(s(doc.tax_exclusive_total()), "361.62"); // BT-109
assert_eq!(s(doc.vat_total()), "68.71");           // BT-110 = round(361.62 × 19 %)
assert_eq!(s(doc.tax_inclusive_total()), "430.33"); // BT-112
assert_eq!(s(doc.payable()), "430.33");            // BT-115

// Every derived amount already fits the scale: there is nothing left to round.
assert!(doc.vat_groups().iter().all(|g| g.taxable_base().fits(2) && g.tax().fits(2)));
```

## What holds by construction

These rules cannot fail on a `Document`, so nothing checks them:

| Rule | Asserts |
|---|---|
| `BR-CO-10` | BT-106 = Σ BT-131 |
| `BR-CO-11`, `BR-CO-12` | BT-107 = Σ BT-92, BT-108 = Σ BT-99 |
| `BR-CO-13` | BT-109 = BT-106 − BT-107 + BT-108 |
| `BR-CO-14` | BT-110 = Σ BT-117 |
| `BR-CO-15` | BT-112 = BT-109 + BT-110 |
| `BR-CO-16` | BT-115 = BT-112 − BT-113 + BT-114 |
| `BR-CO-17` | BT-117 = BT-116 × BT-119 ÷ 100, within ±1.00 — here exactly rounded |
| `BR-S-08`, `BR-AF-08`, `BR-AG-08`, … | each group's base is the sum of its members |
| `BR-DEC-09`, `BR-DEC-12` … `BR-DEC-16`, `BR-DEC-19`, `BR-DEC-20`, `BR-DEC-23`, `BR-DEC-24` | at most two decimals, at `AmountScale::EN16931` |

A `Document` exists only through `build()` (deserialisation included) and its fields
are private, so there is no `validate()`. What `build()` **does** check is everything that depends on the inputs: prices
(`BR-27`), base quantities (`PEPPOL-EN16931-R121`), units (`PEPPOL-EN16931-R130`),
stated bases (`PEPPOL-EN16931-R040`), VAT categories and reasons, and settlement. Each
failure is an [`Error`](../errors/) with a location.

## The one check that is left: `PEPPOL-EN16931-R120`

R120 compares BT-131 with `quantity × price ÷ base + charges − allowances` within
±0.02. A derived net always satisfies it at scale 2 and finer; a line that **states**
its net (an interval series, a split share) can miss it, and `price_inconsistent()`
lists those lines:

```rust
use billing::{AmountScale, Currency, Document, Line, Price, Quantity, UnitPrice};
use rust_decimal::dec;

let line = |net| Line::new("x", Quantity::new(dec!(3), "C62"), UnitPrice::new(Price::new(dec!(1)))).with_net(net);
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(line(dec!(3.02)))
    .line(line(dec!(3.03)))
    .build()?;
assert_eq!(doc.price_inconsistent(), [1]);
```

It is a report, not a build error, because R120's tolerance is absolute.

BT-111, BT-113 and BT-114 come from [settlement](../settlement/) inputs; minimum and
maximum charges are [bounds](../corrections/#bounds).

## `BR-CO-25` does not exist

The often-cited "due date or payment terms when the amount due is positive" rule is
not in the published CEN artefacts (`BR-CO-24` is followed by `BR-CO-26`). If a profile
reinstates it, it belongs to the format layer.
