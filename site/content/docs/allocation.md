+++
title = "Splitting"
description = "Exact largest-remainder splits — split, split_matrix, Document::split and Line::segments — what they guarantee and what they cannot."
weight = 70
+++

The crate has one apportionment method, used everywhere: **largest remainder**
(Hamilton), in exact integer arithmetic. An input past `i128` is
`ErrorKind::Overflow`.

## `split`

```rust
use billing::{Amount, Decimal, split};
use rust_decimal::dec;

let parts = split(dec!(100.00), &[dec!(1); 3], 2)?;
assert_eq!(parts, [dec!(33.34), dec!(33.33), dec!(33.33)]);
assert_eq!(parts.iter().sum::<Decimal>(), dec!(100.00));

// The sign mirrors the total: a credit splits like the mirrored debit.
assert_eq!(split(dec!(-100.00), &[dec!(1); 3], 2)?, [dec!(-33.34), dec!(-33.33), dec!(-33.33)]);

// A zero weight gets exactly zero.
assert_eq!(split(dec!(10.00), &[dec!(3), dec!(0), dec!(1)], 2)?, [dec!(7.50), dec!(0.00), dec!(2.50)]);

// The same on an Amount, at its own precision.
let shares = Amount::<2>::parse("47.10")?.split(&[dec!(40), dec!(35), dec!(25)])?;
assert_eq!(shares.iter().map(|a| a.to_string()).collect::<Vec<_>>(), ["18.84", "16.49", "11.77"]);
```

For `qᵢ = total · wᵢ / Σw` in units, `split` guarantees:

- **Conservation**: `Σ parts == total`, exactly.
- **Quota**: each `|partᵢ|` is `⌊|qᵢ|⌋` or `⌈|qᵢ|⌉` units — no recipient is more than
  one unit from its exact share.
- **Ties** go to the lowest index; a zero weight gets zero.
- **Sign**: `split(−t) == −split(t)` element-wise.
- **Permutation**: permuting weights with distinct remainders permutes the parts.

A total with more decimals than `decimals` is `ExcessPrecision`; empty, negative or
(for a non-zero total) all-zero weights are `InvalidWeights`, located at
`Location::Weight(i)`.

### The Alabama paradox

Largest remainder is **not** monotone (no quota method can be, by Balinski–Young): a
larger total can give a recipient less:

```rust
use billing::split;
use rust_decimal::dec;

let w = [dec!(6), dec!(6), dec!(2)];
assert_eq!(split(dec!(0.10), &w, 2)?, [dec!(0.04), dec!(0.04), dec!(0.02)]);
assert_eq!(split(dec!(0.11), &w, 2)?, [dec!(0.05), dec!(0.05), dec!(0.01)]); // the third gets less
```

So **never split cumulative totals and bill the differences**; split each increment
on its own.

## `split_matrix`: many totals, one set of weights

Splitting row by row lets the same recipient win every tie. `split_matrix` keeps
every row's guarantees — exact row sums, per-cell quota, sign — but breaks ties by each
recipient's **running deficit** across the rows so far:

```rust
use billing::{Decimal, split, split_matrix};
use rust_decimal::dec;

let column = |rows: &[Vec<Decimal>], j: usize| rows.iter().map(|r| r[j]).sum::<Decimal>();

let per_row: Vec<Vec<Decimal>> = (0..10).map(|_| split(dec!(0.01), &[dec!(1); 3], 2)).collect::<Result<_, _>>()?;
assert_eq!([column(&per_row, 0), column(&per_row, 1), column(&per_row, 2)], [dec!(0.10), dec!(0), dec!(0)]);

let matrix = split_matrix(&[dec!(0.01); 10], &[dec!(1); 3], 2)?;
assert_eq!([column(&matrix, 0), column(&matrix, 1), column(&matrix, 2)], [dec!(0.04), dec!(0.03), dec!(0.03)]);
```

A column's sum stays within ½ unit of its exact share for two recipients. For three
or more there is no proven bound and it can exceed one unit (tests assert < 1.5 units
up to eight recipients); row-by-row splitting grows without bound.

## `Document::split` {#document-split}

`Document::split(&weights)` splits a whole document — a shared meter, a building's
tenants, a co-working floor — into one complete `Document` per recipient:

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("water", dec!(47.10)))
    .build()?;
assert_eq!(doc.vat_total(), Amount::parse("8.95")?); // 47.10 × 19 % = 8.949

let shares = doc.split(&[dec!(40), dec!(35), dec!(25)])?;
let show = |f: fn(&Document) -> Amount| shares.iter().map(|s| format!("{:.2}", f(s))).collect::<Vec<_>>();
assert_eq!(show(Document::tax_exclusive_total), ["18.84", "16.49", "11.77"]);
assert_eq!(show(Document::vat_total), ["3.58", "3.13", "2.24"]);

// The shares re-assemble to the parent exactly — 56.05, not 56.06.
let gross: Amount = shares.iter().map(Document::tax_inclusive_total).sum();
assert_eq!(gross, doc.tax_inclusive_total());
```

Every **leaf** — line, adjustment, paid entry — is split in one `split_matrix`, so
the shares' leaves sum to the parent's exactly.

**VAT is partitioned, not re-rounded.** Each group's tax is distributed by largest
remainder of each share's exact `base × rate`, so every share's tax is within one unit
of its own exact tax (`BR-CO-17`) and the shares sum to the parent. The partition is
recorded (`Document::tax_partition`); a deserialised share with a tampered partition is
`ErrorKind::InvalidTaxPartition`.

| Conserved exactly | Re-derived per share |
|---|---|
| every leaf; BT-106, BT-107, BT-108, BT-109, BT-110, BT-112, BT-113; every group's BT-116 and BT-117 | BT-114 and BT-115 under cash rounding (each share is rounded to its own increment); BT-111 |

Adjustment bases are dropped from the shares (a split amount does not reproduce
`base × percent`, `PEPPOL-EN16931-R040`), and bound lines and apportioned adjustments
become ordinary ones.

## `Line::segments`: prorating one line

Split one line by weights such as days — a tariff change mid-month, a partial
period. The segments' nets sum to the line's net:

```rust
use billing::{AmountScale, Line, Price, Quantity, UnitPrice};
use rust_decimal::dec;

let fee = Line::new("base fee", Quantity::new(dec!(1), "MON"), UnitPrice::new(Price::new(dec!(10))));
let parts = fee.segments(&[dec!(14), dec!(17)], AmountScale::EN16931)?;
assert_eq!(parts[0].stated_net(), Some(dec!(4.52)));
assert_eq!(parts[1].stated_net(), Some(dec!(5.48)));
```

The engine takes weights, not a calendar.
