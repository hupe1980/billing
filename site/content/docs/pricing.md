+++
title = "Pricing"
description = "Tariffs as data — per-unit, graduated, volume and package — plus interval series rated exactly, a tariff per key, transaction fees, progressive billing and nearest-rank percentiles."
weight = 30
+++

Pricing turns a measured quantity into lines. Results are exact `Decimal`s; the
document reduces each leaf once, so nothing below takes a rounding parameter except
`increment_lines`.

## `Tariff`

A `Tariff` is a price model as data:

| Variant | Meaning |
|---|---|
| `PerUnit(price)` | every unit at one price |
| `Graduated(tiers)` | each tier's share of the quantity at that tier's price, plus the flat fee of every tier reached |
| `Volume(tiers)` | the whole quantity at the price of the tier it falls in, plus that tier's flat fee |
| `Package { size, price, rounding }` | whole packages of `size` units, a started package counted (`Up`) or not (`Down`) |

`Tariff::rate(usage)` takes a `Decimal` or an exact [`Usage`](../metering/) and returns
`Piece`s — tier index, exact quantity (a `Usage`), unit price, optional flat fee — and
`Piece::into_lines(description, unit)` turns each into ordinary
[`Line`](../tax-and-discounts/)s. A fractional usage (an average) is rated exactly: tier
bounds are compared with the fraction, and each line derives its net from it.

```rust
use billing::{Price, Tariff, Tier, Tiers};
use rust_decimal::dec;

let p = |x| Price::new(x);
let tiers = Tiers::new([Tier::up_to(dec!(100), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?;

// Graduated: 100 × 0.30 + 50 × 0.25.
let pieces = Tariff::Graduated(tiers.clone()).rate(dec!(150))?;
assert_eq!((pieces[0].quantity.to_decimal(), pieces[1].quantity.to_decimal()), (dec!(100), dec!(50)));
assert_eq!(pieces[0].exact_net()? + pieces[1].exact_net()?, dec!(42.50));

// Volume: every unit at the reached tier's price — and not monotone.
let at = |q| Tariff::Volume(tiers.clone()).rate(q).and_then(|v| v[0].exact_net());
assert_eq!(at(dec!(100))?, dec!(30.00));
assert_eq!(at(dec!(101))?, dec!(25.25)); // the "volume cliff"

// Package: 25 units in packages of 10, every started package billed.
let pkg = Tariff::Package { size: dec!(10), price: p(dec!(2)), rounding: billing::PackageRounding::Up };
let piece = &pkg.rate(dec!(25))?[0];
assert_eq!((piece.quantity.to_decimal(), piece.measured, piece.exact_net()?), (dec!(3), Some(dec!(25)), dec!(6)));
```

### Semantics every tariff shares

- **Upper bounds are inclusive**, as Stripe's `up_to`: with tiers *up to 100* and
  *above*, 100 units are all in tier 0 and 100.5 put 0.5 in tier 1.
- **A flat fee applies when its tier is reached.** Quantity 0 reaches tier 0, so a
  first-tier flat fee is charged even for zero usage. A graduated tier `i > 0` is
  reached when the quantity exceeds tier `i − 1`'s bound.
- **No slivers.** A piece is emitted only if its quantity is non-zero or it carries a
  non-zero flat fee.
- **Direction lives in the quantity.** `rate(−q)` is the exact negation of `rate(q)`
  — the credit for the same usage — never a delta against earlier usage. Prices stay
  ≥ 0 (`BR-27`).

A capacity or stair-step price is a flat-only tier (`unit_price` 0) in a `Volume`
tariff.

### Each piece is its own leaf

Each piece becomes its own line with its own reduced net, so two pieces of 0.005
make 0.02 at two decimals:

```rust
use billing::{Amount, AmountScale, Currency, Document, Price, Tariff, Tier, Tiers};
use rust_decimal::dec;

let tiers = Tiers::new([Tier::up_to(dec!(1), Price::new(dec!(0.005))), Tier::above(Price::new(dec!(0.005)))])?;
let lines = Tariff::Graduated(tiers).rate(dec!(2))?.into_iter().flat_map(|p| p.into_lines("api calls", "C62"));
let doc = Document::builder(Currency::EUR, AmountScale::EN16931).lines(lines).build()?;
assert_eq!(doc.lines().len(), 2);
assert_eq!(doc.line_total(), Amount::parse("0.02")?);
```

### What `Tiers::new` refuses

At least one tier, every `up_to` above zero and strictly increasing (unsorted input
is refused, not sorted), only the last tier open, prices and flat fees ≥ 0. The error
carries a `TierError` and the `Location::Tier`:

```rust
use billing::{ErrorKind, Location, Price, Tier, TierError, Tiers};
use rust_decimal::dec;

let err = Tiers::new([Tier::up_to(dec!(100), Price::new(dec!(1))), Tier::up_to(dec!(50), Price::new(dec!(1)))])
    .unwrap_err();
assert_eq!(err.kind(), &ErrorKind::InvalidTiers(TierError::NotIncreasing));
assert_eq!(err.location(), Some(&Location::Tier(1)));
```

A quantity above the last bound of tiers that end closed is `ErrorKind::Uncovered`
— never silently billed at zero.

## Interval series: `rate_series`

Spot prices arrive as `(quantity, price)` intervals. `rate_series` accumulates
`Σ qᵢ·pᵢ` **exactly**, and `SeriesTotal::into_line` states that net on one line, reduced
once:

```rust
use billing::{Amount, AmountScale, Currency, Document, Price, rate_series};
use rust_decimal::dec;

let series = rate_series([
    (dec!(1.5), Price::new(dec!(0.1123))),
    (dec!(2.0), Price::new(dec!(-0.0150))), // spot prices go below zero
    (dec!(0.75), Price::new(dec!(0.0899))),
])?;
assert_eq!(series.net, dec!(0.205875));
assert_eq!(series.quantity, dec!(4.25));

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(series.into_line("spot energy", "KWH"))
    .build()?;
assert_eq!(doc.line_net(0), Amount::parse("0.21")?);
assert!(doc.price_inconsistent().is_empty()); // PEPPOL-EN16931-R120 holds
```

The line's price (BT-146) is the average `|net ÷ quantity|` to 28 digits; never round
it ([why](../e-invoicing/#bt-146-is-not-capped-and-rounding-it-is-fatal)). When the
average is negative, the price stays ≥ 0 (`BR-27`) and the direction moves into the
quantity:

```rust
use billing::{Amount, AmountScale, Currency, Document, Price, rate_series};
use rust_decimal::dec;

let feed_in = rate_series([(dec!(1), Price::new(dec!(-0.1)))])?;
assert!(feed_in.is_negative_average());
let line = feed_in.into_line("feed-in", "KWH");
assert_eq!((line.quantity().value(), line.price().price().value()), (dec!(-1), dec!(0.1)));
let doc = Document::builder(Currency::EUR, AmountScale::EN16931).line(line).build()?;
assert_eq!(doc.line_net(0), Amount::parse("-0.10")?);
```

## Keyed tariffs: `rate_by_key`

HT/NT, a region, a storage class: each key maps to its **own `Tariff`**. Usage per
key is summed exactly; the result lists each key once, in order of first appearance,
with its pieces. A key without a tariff is `ErrorKind::UnknownKey`; a key that rates
to nothing is left out.

```rust
use std::collections::BTreeMap;
use billing::{Amount, AmountScale, Currency, Document, ErrorKind, Price, Tariff, Tier, Tiers, rate_by_key};
use rust_decimal::dec;

let p = |x| Price::new(x);
let tariffs = BTreeMap::from([
    ("HT", Tariff::Graduated(Tiers::new([Tier::up_to(dec!(100), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?)),
    ("NT", Tariff::PerUnit(p(dec!(0.20)))),
]);
let rated = rate_by_key(&tariffs, [("HT", dec!(80)), ("NT", dec!(50)), ("HT", dec!(40))])?;
assert_eq!(rated.iter().map(|(k, _)| *k).collect::<Vec<_>>(), ["HT", "NT"]);

let lines = rated.into_iter().flat_map(|(key, pieces)| {
    let description = format!("energy {key}");
    pieces.into_iter().flat_map(move |x| x.into_lines(&description, "KWH"))
});
let doc = Document::builder(Currency::EUR, AmountScale::EN16931).lines(lines).build()?;
// HT 120 kWh: 100 × 0.30 + 20 × 0.25 = 35.00; NT 50 kWh × 0.20 = 10.00.
assert_eq!(doc.line_total(), Amount::parse("45.00")?);

let err = rate_by_key(&tariffs, [("XX", dec!(1))]).unwrap_err();
assert_eq!(err.kind(), &ErrorKind::UnknownKey { key: "XX".into() });
```

Use `with_label` to carry the key on the line; the engine never reads labels.

## Transaction fees: `TransactionFee`

A percentage of each transaction plus a fixed amount, bounded per transaction, after
some free transactions. Per charged transaction the fee is, exactly,

`sign(amount) × clamp(|amount| × percent ÷ 100 + fixed, min, max)`

- `0 ≤ percent ≤ 100`, `fixed ≥ 0`, `min, max ≥ 0` and `min ≤ max` — otherwise
  `ErrorKind::InvalidFee`, also on deserialisation;
- a refund (a negative amount) returns the fee of the purchase it mirrors;
- a zero amount is a transaction, charged the bounded fixed fee;
- the first `free_transactions` amounts, in input order and refunds included, are
  counted but not charged.

```rust
use billing::{Amount, AmountScale, Currency, Document, ErrorKind, Percent, TransactionFee};
use rust_decimal::dec;

let fee = TransactionFee::new(Percent::new(dec!(2.9)))
    .with_fixed(dec!(0.30))
    .with_min(dec!(0.50))
    .with_max(dec!(5.00))
    .with_free_transactions(2);
// 10 and 100 free; 1 000 → 29.30 capped at 5.00; 5 → 0.445 raised to 0.50; −100 → −3.20.
let total = fee.rate([dec!(10), dec!(100), dec!(1000), dec!(5), dec!(-100)])?;
assert_eq!((total.transactions, total.charged), (5, 3));
assert_eq!((total.base, total.fee), (dec!(905), dec!(2.30)));

// One line, the exact fee stated as its net and rounded once by the document.
let line = total.into_line("card fees", "C62");
assert_eq!(line.quantity().value(), dec!(3));
let doc = Document::builder(Currency::EUR, AmountScale::EN16931).line(line).build()?;
assert_eq!(doc.line_net(0), Amount::parse("2.30")?);

let err = TransactionFee::new(Percent::new(dec!(101))).rate([dec!(1)]).unwrap_err();
assert_eq!(err.kind(), &ErrorKind::InvalidFee);
```

To keep the fixed fee on refunds, rate refunds under a fee without it.

### Graduated percentage

A percentage that steps down with volume is a graduated tariff over the transaction
**volume** with the rates as prices:

```rust
use billing::{Decimal, Price, Tariff, Tier, Tiers};
use rust_decimal::dec;

let rates = Tiers::new([Tier::up_to(dec!(10000), Price::new(dec!(0.029))), Tier::above(Price::new(dec!(0.019)))])?;
let pieces = Tariff::Graduated(rates).rate(dec!(15000))?;
let fee: Decimal = pieces.iter().map(|x| x.exact_net()).sum::<Result<_, _>>()?;
assert_eq!(fee, dec!(385.00)); // 10 000 × 0.029 + 5 000 × 0.019
```

## Progressive billing: `rate_increment` and `increment_lines`

An annual tier schedule billed monthly rates **cumulative** usage; each invoice bills
the increment. The state is explicit: `Billed::Nothing` before the first invoice,
`Billed::UpTo(usage)` after. Each call returns the next state for you to store (it
serialises). A target below what was billed is `ErrorKind::InvalidRange`.

`rate_increment(billed, to)` returns exact pieces and the next state. From `Nothing`
it is the full rating of `to`, base fee included; from `UpTo(from)` it is the
difference. The increments always sum exactly to rating the final value:

```rust
use billing::{Billed, Decimal, PackageRounding, Price, Tariff, Tier, Tiers, Usage};
use rust_decimal::dec;

let p = |x| Price::new(x);
let tiers = Tiers::new([Tier::up_to(dec!(1000), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?;
let net = |t: &Tariff, billed, to| -> Result<Decimal, billing::Error> {
    t.rate_increment(billed, to)?.0.iter().map(|x| x.exact_net()).sum()
};
let up_to = |x| Billed::UpTo(Usage::from(x));

let graduated = Tariff::Graduated(tiers.clone());
assert_eq!(net(&graduated, Billed::Nothing, dec!(400))?, dec!(120.00));
assert_eq!(net(&graduated, up_to(dec!(400)), dec!(900))?, dec!(150.00));
assert_eq!(net(&graduated, up_to(dec!(900)), dec!(1300))?, dec!(105.00)); // 100 × 0.30 + 300 × 0.25
// 120 + 150 + 105 = 375 = rating 1 300 at once.

// Volume: crossing into the cheaper tier lowers the total (the volume cliff).
assert_eq!(net(&Tariff::Volume(tiers), up_to(dec!(990)), dec!(1010))?, dec!(-44.50)); // 252.50 − 297.00

// Package: blocks of 100 at 5.00, started blocks billed: 3 → 5 blocks.
let package = Tariff::Package { size: dec!(100), price: p(dec!(5.00)), rounding: PackageRounding::Up };
assert_eq!(net(&package, up_to(dec!(250)), dec!(410))?, dec!(10.00));
```

A base fee with idle months:

```rust
use billing::{Billed, Decimal, Price, Tariff, Tier, Tiers};
use rust_decimal::dec;

let t = Tariff::Graduated(Tiers::new([
    Tier::up_to(dec!(100), Price::new(dec!(0.30))).with_flat(dec!(10.00)),
    Tier::above(Price::new(dec!(0.25))),
])?);
let (mut billed, mut months) = (Billed::Nothing, Vec::new());
for to in [dec!(0), dec!(0), dec!(50)] {
    let (pieces, next) = t.rate_increment(billed, to)?;
    months.push(pieces.iter().map(|x| x.exact_net()).sum::<Result<Decimal, _>>()?);
    billed = next;
}
assert_eq!(months, [dec!(10.00), dec!(0.00), dec!(15.00)]); // 25.00 = rating 50 at once
```

Increments can be negative (a volume reversal); those lines are ordinary credits.

### Rounded increments that still add up

`increment_lines(billed, to, scale, description, unit)` states each tier's net as
`round(net at to) − round(net at from)`, so the **rounded** invoices add up to the
rounded total:

```rust
use billing::{Amount, AmountScale, Billed, Currency, Decimal, Document, Price, Tariff};
use rust_decimal::dec;

// 0.4, 0.8 and 1.2 kWh cumulative at 0.015 per kWh: each exact increment is 0.006.
let t = Tariff::PerUnit(Price::new(dec!(0.015)));
let s = AmountScale::EN16931;
let (mut state, mut billed) = (Billed::Nothing, Vec::new());
for to in [dec!(0.4), dec!(0.8), dec!(1.2)] {
    let (lines, next) = t.increment_lines(state, to, s, "energy", "KWH")?;
    let doc = Document::builder(Currency::EUR, s).lines(lines).build()?;
    billed.push(doc.line_total().to_decimal());
    state = next;
}
// round(0.006) − 0, round(0.012) − round(0.006), round(0.018) − round(0.012):
assert_eq!(billed, [dec!(0.01), dec!(0.00), dec!(0.01)]);
assert_eq!(billed.iter().sum::<Decimal>(), dec!(0.02)); // = round(0.018); per increment: 0.03

let at_once = Document::builder(Currency::EUR, s)
    .lines(t.increment_lines(Billed::Nothing, dec!(1.2), s, "energy", "KWH")?.0)
    .build()?;
assert_eq!(at_once.line_total(), Amount::parse("0.02")?);
```

`price_inconsistent()` (R120) stays empty at two decimals and finer.

## Burstable billing: `nearest_rank`

IP transit and CDN egress bill the 95th percentile. `nearest_rank` returns the sample
at rank `⌈p/100 · n⌉` — **never interpolated**, always a measured value.

```rust
use billing::{Decimal, Percentile, nearest_rank};

// A 30-day month of five-minute samples: 8 640 of them.
let mut samples: Vec<Decimal> = (1..=8640).rev().map(Decimal::from).collect();
assert_eq!(nearest_rank(&mut samples, Percentile::P95), Some(Decimal::from(8208)));
assert_eq!(nearest_rank(&mut [], Percentile::P95), None);
assert!(Percentile::new(0).is_err()); // 1..=100, in whole percent
```

`Percentile` wraps a whole percent (`u8`), so `0.95` does not compile. To reduce
readings first, see [Metering](../metering/).
