+++
title = "Errors"
description = "One Error type: kind() says why, location() says where (0-based), rule() names the EN 16931 or Peppol rule it enforces. Match on those, not on the text."
weight = 110
+++

Every fallible call returns `billing::Error` — one type for the whole crate. It has
three parts:

| Accessor | Answers | Type |
|---|---|---|
| `kind()` | why it failed | `&ErrorKind` (`#[non_exhaustive]`) |
| `location()` | where in your input | `Option<&Location>` |
| `rule()` | which EN 16931 / Peppol rule it enforces | `Option<&'static str>` |

```rust
use billing::{AmountScale, Currency, Document, ErrorKind, Line, Location, Price, Quantity, UnitPrice};
use rust_decimal::dec;

let err = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("ok", dec!(10)))
    .line(Line::new("bad", Quantity::one(), UnitPrice::new(Price::new(dec!(-1)))))
    .build()
    .unwrap_err();

assert_eq!(err.kind(), &ErrorKind::NegativePrice { price: dec!(-1) });
assert_eq!(err.location(), Some(&Location::Line(1)));
assert_eq!(err.rule(), Some("BR-27"));
assert_eq!(err.to_string(), "line 1: price -1 is negative (BR-27)");
```

**Match on the kind, the location and the rule**, not on the `Display` text, which may
be reworded. `ErrorKind` is `#[non_exhaustive]`, so a `match` needs a wildcard arm:

```rust
use billing::{Error, ErrorKind};

fn advice(err: &Error) -> &'static str {
    match err.kind() {
        ErrorKind::Overflow => "the amounts are out of range",
        ErrorKind::InvalidVat(_) | ErrorKind::MissingVat => "check the VAT treatment",
        _ if err.rule().is_some() => "the input breaks an EN 16931 rule",
        _ => "invalid input",
    }
}
assert_eq!(advice(&ErrorKind::MissingVat.into()), "check the VAT treatment");
```

## Locations

Every index is **0-based** — the position in your slice or builder call order — and
`Display` prints that same index:

| `Location` | `Display` | Points at |
|---|---|---|
| `Line(0)` | `line 0` | the first line |
| `LineAdjustment { line: 1, index: 0 }` | `line 1 adjustment 0` | the first allowance or charge of the second line |
| `Adjustment(2)` | `adjustment 2` | the third document allowance or charge |
| `Tier(3)` | `tier 3` | the fourth tier |
| `Weight(4)` | `weight 4` | the fifth split weight |
| `Bound(0)` | `bound 0` | the first `Bound` given to the builder |
| `Payment(1)` | `payment 1` | the second `Payment` given to the builder |
| `Sample(2)` | `sample 2` | the third reading given to an [aggregation](../metering/) |
| `Credit(0)` | `credit 0` | the first `Credit` given to the builder |
| `Transaction(3)` | `transaction 3` | the fourth amount given to [`TransactionFee::rate`](../pricing/#transaction-fees-transactionfee) |

Document adjustments are numbered in builder order, followed by those materialised by
[`apportion`](../tax-and-discounts/#apportion). A parse failure or a settlement
setting has no location.

## Rules

`rule()` is `Some` exactly where an artefact rule exists for the failure:

| Kind | Rule |
|---|---|
| `NegativePrice` | `BR-27` |
| `InvalidBaseQuantity` | `PEPPOL-EN16931-R121` |
| `AdjustmentBasis` | `PEPPOL-EN16931-R040` |
| `PriceBaseUnit` | `PEPPOL-EN16931-R130` |
| `InvalidVatAccounting` | `PEPPOL-EN16931-R005` |
| `MissingVat` at a `Location::Line` | `BR-CO-04` |
| `MissingVat` at a document allowance / charge | `BR-32` / `BR-37` |
| `InvalidVat(VatError)` | per variant: `BR-*-05` (rate), `BR-*-10` (reason), `BR-O-11`, `BR-B-02` |

Every other kind — arithmetic or domain validation with no artefact rule, including
`InvalidTaxPartition` (stricter than `BR-CO-17`) — reports `None`. Every rule id
returned is hash-locked against the pinned Schematron (`just check-citations`).

## The kinds, grouped

| Group | Kinds |
|---|---|
| arithmetic | `Overflow`, `ExcessPrecision`, `DivisionByZero`, `InvalidIncrement` |
| text | `Malformed`, `InvalidCurrency` |
| prices and quantities | `NegativePrice`, `InvalidBaseQuantity`, `PriceBaseUnit` |
| pricing | `InvalidTiers(TierError)`, `Uncovered`, `UnknownKey`, `InvalidPercentile`, `InvalidFee`, `InvalidRange` |
| metering | `MissingKey`, `NegativeWeight`, `InvalidConversion` |
| splitting | `InvalidWeights` |
| VAT | `InvalidVat(VatError)`, `MissingVat`, `ConflictingExemption`, `InvalidTaxPartition` |
| adjustments and bounds | `AdjustmentBasis`, `InvalidBound` |
| settlement | `InvalidCashRounding`, `InvalidVatAccounting`, `InvalidRate`, `InvalidCredit`, `UnmatchedAdvance` |
| combining | `CurrencyMismatch`, `ScaleMismatch`, `SettingMismatch`, `ConflictingSetting`, `Empty` |

`Overflow` is never approximated. A value with excess digits that is also out of
range reports `ExcessPrecision`.

## Panics

The crate's fallible functions never panic. Two things do, by contract:

- the `+`, `-`, unary `-`, `+=`, `-=` and `Sum` operators on `Amount`, on overflow —
  use the `checked_*` methods for untrusted magnitudes;
- the indexed accessors `line_net(i)`, `line_adjustment_amounts(i)` and
  `adjustment_amount(i)`, for an index out of range, like slice indexing.
