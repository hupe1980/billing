+++
title = "Metering"
description = "Readings reduced to a billable quantity as data — sum, count, unique count, min, max, latest, mean, weighted sum, weighted mean and percentile — exactly, with averages kept as fractions all the way into the invoice line."
weight = 25
+++

Metering reduces a period's readings to one billable quantity, which a
[tariff](../pricing/) then prices. Pass the readings of one period **in chronological
order**; ingestion, deduplication and clocks belong to your metering system.

## `Sample` and `Aggregation`

A `Sample` is one reading: a value, a weight (default 1) and an optional key. An
`Aggregation` is one of ten kinds:

| Kind | Result | No readings |
|---|---|---|
| `Sum` | Σ value | 0 |
| `Count` | number of readings | 0 |
| `UniqueCount` | number of distinct keys | 0 |
| `Min` / `Max` | least / greatest value | no value |
| `Latest` | the last reading's value | no value |
| `Mean` | Σ value ÷ count, an exact fraction | no value |
| `WeightedSum` | Σ value × weight | 0 |
| `WeightedMean` | Σ value × weight ÷ Σ weight, an exact fraction | no value (also when Σ weight = 0) |
| `Percentile(p)` | nearest rank, always a measured value | no value |

`apply` returns `Ok(None)` for "no value": absence is not zero.

```rust
use billing::{Aggregation, Percentile, Sample};
use rust_decimal::{Decimal, dec};

let readings = || [3, 9, 4, 7, 5].map(|v| Sample::new(Decimal::from(v)));
let value = |a: Aggregation| a.apply(readings()).map(|u| u.unwrap().to_decimal());
assert_eq!(value(Aggregation::Sum)?, dec!(28));
assert_eq!(value(Aggregation::Count)?, dec!(5));
assert_eq!(value(Aggregation::Min)?, dec!(3));
assert_eq!(value(Aggregation::Max)?, dec!(9));
assert_eq!(value(Aggregation::Latest)?, dec!(5));
assert_eq!(value(Aggregation::Mean)?, dec!(5.6));
assert_eq!(value(Aggregation::Percentile(Percentile::P95))?, dec!(9));

assert_eq!(Aggregation::Sum.apply([])?.unwrap().to_decimal(), dec!(0));
assert_eq!(Aggregation::Max.apply([])?, None);
```

## Unique counts and gauges

`UniqueCount` counts distinct keys and refuses a reading without one. The weighted
kinds bill gauges (storage, seats), each reading weighted by its share of the period;
a negative weight is refused. Refusals name the reading's 0-based position.

```rust
use billing::{Aggregation, ErrorKind, Location, Sample};
use rust_decimal::dec;

let users = ["ana", "bo", "ana", "cy"].map(|k| Sample::new(dec!(1)).with_key(k));
assert_eq!(Aggregation::UniqueCount.apply(users)?.unwrap().to_decimal(), dec!(3));

let err = Aggregation::UniqueCount
    .apply([Sample::new(dec!(1)).with_key("ana"), Sample::new(dec!(1)).with_key("bo"), Sample::new(dec!(1))])
    .unwrap_err();
assert_eq!((err.kind(), err.location()), (&ErrorKind::MissingKey, Some(&Location::Sample(2))));

// 10 GB for a quarter of the month, then 20 GB.
let storage = || [Sample::new(dec!(10)).with_weight(dec!(0.25)), Sample::new(dec!(20)).with_weight(dec!(0.75))];
assert_eq!(Aggregation::WeightedSum.apply(storage())?.unwrap().to_decimal(), dec!(17.5));
assert_eq!(Aggregation::WeightedMean.apply(storage())?.unwrap().to_decimal(), dec!(17.5));

let err = Aggregation::WeightedSum.apply([Sample::new(dec!(10)).with_weight(dec!(-1))]).unwrap_err();
assert_eq!(err.kind(), &ErrorKind::NegativeWeight { weight: dec!(-1) });
assert_eq!(err.location(), Some(&Location::Sample(0)));
```

## Exact averages: `Usage`

Every result is a `Usage`: `numerator ÷ denominator`, an exact fraction when an
average has no exact decimal; `to_decimal()` is for display. The mean of 1, 0, 0 is
1/3: × 0.015 that bills **0.01** exactly, but **0.00** from the 28-digit decimal.
`Line::from_usage` shows the decimal as BT-129 and derives BT-131 from the fraction;
`Line::exact_quantity` returns the fraction.

```rust
use billing::{Aggregation, AmountScale, Currency, Document, Line, Price, Quantity, Sample, UnitPrice};
use rust_decimal::dec;

let mean = Aggregation::Mean.apply([dec!(1), dec!(0), dec!(0)].map(Sample::new))?.unwrap();
assert_eq!((mean.numerator(), mean.denominator()), (dec!(1), dec!(3)));
assert!(!mean.is_exact_decimal());

let price = UnitPrice::new(Price::new(dec!(0.015)));
let exact = Line::from_usage("average seats", mean, "C62", price.clone());
let doc = Document::builder(Currency::EUR, AmountScale::EN16931).line(exact).build()?;
assert_eq!(doc.line_total().to_decimal(), dec!(0.01));

let rounded_first = Line::new("average seats", Quantity::new(mean.to_decimal(), "C62"), price);
let doc = Document::builder(Currency::EUR, AmountScale::EN16931).line(rounded_first).build()?;
assert_eq!(doc.line_total().to_decimal(), dec!(0.00));
```

A `WeightedMean` is a fraction too: readings 10 (weight 1) and 20 (weight 2) give 50/3,
whose reading is 16.666666666666666666666666667.

```rust
use billing::{Aggregation, Sample};
use rust_decimal::dec;

let u = Aggregation::WeightedMean
    .apply([Sample::new(dec!(10)), Sample::new(dec!(20)).with_weight(dec!(2))])?
    .unwrap();
assert_eq!((u.numerator(), u.denominator()), (dec!(50), dec!(3)));
assert_eq!(u.to_decimal(), dec!(16.666666666666666666666666667));
```

## Pricing a usage through a tariff

`Tariff::rate` takes a `Usage` (or a `Decimal`); tiers compare the exact fraction and
`Piece::into_lines` uses `Line::from_usage`:

```rust
use billing::{Aggregation, AmountScale, Currency, Document, Price, Sample, Tariff, Tier, Tiers, Usage};
use rust_decimal::dec;

let tiers = Tiers::new([Tier::up_to(dec!(1), Price::new(dec!(0.015))), Tier::above(Price::new(dec!(0.01)))])?;
let third = Aggregation::Mean.apply([dec!(1), dec!(0), dec!(0)].map(Sample::new))?.unwrap();
let pieces = Tariff::Graduated(tiers).rate(third)?;
assert_eq!(pieces[0].quantity, third);
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .lines(pieces.into_iter().flat_map(|p| p.into_lines("api calls", "C62")))
    .build()?;
assert_eq!(doc.line_total().to_decimal(), dec!(0.01));

assert_eq!(Usage::from(dec!(150)).denominator(), dec!(1)); // a plain quantity
```

## Unit conversion: `Meter`

A `Meter` is an aggregation plus an optional `UnitConversion`: divide by a positive
divisor, then round **once** to `decimals` (0–28) with one of the
[rounding modes](../amounts/). An average converts from its exact fraction.

```rust
use billing::{Aggregation, ErrorKind, Meter, Rounding, Sample, UnitConversion};
use rust_decimal::dec;

let hours = Meter::new(Aggregation::Sum)
    .with_conversion(UnitConversion::new(dec!(3600), 0, Rounding::AwayFromZero)?);
assert_eq!(hours.apply([Sample::new(dec!(7201))])?.unwrap().to_decimal(), dec!(3));
assert_eq!(hours.apply([Sample::new(dec!(7200))])?.unwrap().to_decimal(), dec!(2));

let gb = Meter::new(Aggregation::Sum)
    .with_conversion(UnitConversion::new(dec!(1024), 0, Rounding::TowardZero)?);
assert_eq!(gb.apply([Sample::new(dec!(1536))])?.unwrap().to_decimal(), dec!(1));

let cents = Meter::new(Aggregation::Mean)
    .with_conversion(UnitConversion::new(dec!(1), 2, Rounding::AwayFromZero)?);
assert_eq!(cents.apply([dec!(1), dec!(0), dec!(0)].map(Sample::new))?.unwrap().to_decimal(), dec!(0.34));

assert_eq!(Meter::new(Aggregation::Max).with_conversion(UnitConversion::new(dec!(60), 0, Rounding::HalfEven)?).apply([])?, None);
assert_eq!(UnitConversion::new(dec!(0), 0, Rounding::TowardZero).unwrap_err().kind(), &ErrorKind::InvalidConversion);
```

## As data

With the `serde` feature, `Sample`, `Aggregation`, `UnitConversion`, `Meter` and
`Usage` serialise with decimals as strings; invalid values are refused on
deserialisation:

```rust
use billing::{Aggregation, Meter, Rounding, Sample, UnitConversion};
use rust_decimal::dec;

let json = r#"{"aggregation":"Sum","conversion":{"divisor":"3600","decimals":0,"rounding":"AwayFromZero"}}"#;
let meter: Meter = serde_json::from_str(json)?;
assert_eq!(meter, Meter::new(Aggregation::Sum).with_conversion(UnitConversion::new(dec!(3600), 0, Rounding::AwayFromZero)?));
assert_eq!(serde_json::to_string(&meter)?, json);
assert_eq!(meter.apply([Sample::new(dec!(5400))])?.unwrap().to_decimal(), dec!(2));
assert!(serde_json::from_str::<Meter>(&json.replace(r#""3600""#, "3600")).is_err());
assert!(serde_json::from_str::<Aggregation>(r#"{"Percentile":0}"#).is_err());
```
