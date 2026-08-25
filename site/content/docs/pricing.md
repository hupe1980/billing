+++
title = "Pricing models"
description = "Graduated, volume, block and capacity rate schedules; time-of-use bands; spot pricing; capacity-keyed rate tables; and the PricingModel trait that ties them to your domain."
weight = 30
+++

## `RateSchedule` — four modes

One type, four billing bases. The mode decides how a cumulative quantity becomes
positions.

| Mode | Billing basis | Returns |
|---|---|---|
| `graduated` | each tier at its own price (Staffelpreise) | one position per tier |
| `volume` | **all** units at the price of the top tier reached | one position |
| `block` | rounded-up blocks × price per block | one position |
| `capacity` | one value selects a tier — a peak, or a P95 — and a flat charge is billed | one position |

```rust
use billing::prelude::*;
use rust_decimal::dec;

let graduated = RateSchedule::graduated()
    .unit("kWh")
    .currency(Currency::EUR)
    .band(RateBand::up_to(dec!(500), Amount::parse("0.32000")?))
    .band(RateBand::over(dec!(500),  Amount::parse("0.28000")?))
    .build()?;

let items = graduated.split(dec!(1234.5))?;
assert_eq!(items.len(), 2);
assert_eq!(items[0].net_amount, Amount::parse("160.00000")?);   // 500 × 0.32
assert_eq!(items[1].net_amount, Amount::parse("205.66000")?);   // 734.5 × 0.28
```

The same bands under `volume` produce a single position of `1234.5 × 0.28`, because
the whole quantity is priced at the tier it reached. That difference is the single
most common source of a mispriced consumption invoice, which is why the two are
distinct modes rather than a boolean.

`RateBand::free_up_to(n)` is a zero-priced first tier — the "first 100 000 API calls
included" shape. Keeping it as a band rather than subtracting it means the invoice
shows what was included next to what was charged.

### What the builder refuses

`build()` is where a mispriced schedule is caught, and it is strict on purpose:

- zero bands, or a negative price (credits belong in a `DiscountLayer`);
- a gap or an overlap between adjacent bands — checked even for a **single** band, so
  a lone `between(500, 1000)` cannot silently bill from zero;
- upper bounds that are not strictly increasing;
- an unbounded upper bound anywhere but the last band;
- `block_size` outside block mode, or missing inside it.

And `split()` refuses a quantity the schedule does not cover, rather than pricing the
overflow at the top-band rate. Silent under-billing is the failure mode no test
catches, so every one of these is an error rather than a skip.

## Time-of-use

N named bands. The engine has no knowledge of time zones, clocks or regulator
schedules — you supply pre-aggregated consumption per band, which is what a smart
meter gives you anyway.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let tou = TimeOfUsePricing::builder()
    .unit("kWh")
    .currency(Currency::EUR)
    .band(TimeBand::new("HT", Amount::parse("0.32000")?))
    .band(TimeBand::new("NT", Amount::parse("0.18000")?))
    .build()?;

let items = tou.calculate(&[("HT", dec!(823.4)), ("NT", dec!(411.1))])?;
assert_eq!(items[0].net_amount, Amount::parse("263.48800")?);

// A typo is an error, not 100 kWh quietly dropped from the invoice.
assert!(tou.calculate(&[("ht", dec!(100))]).is_err());
```

Each generated position is tagged with its band name, so a later tax layer can target
one band. Band names that would collide with the engine's own reserved tags (`tax`,
`levy`, `discount`, …) are rejected at build time: a band called `tax` would make its
own consumption line look like a tax line to `PerUnitLevy`, which excludes those from
its base — a levy silently charged on a fraction of the energy consumed.

## Dynamic (spot) pricing

A sequence of `(quantity, price)` intervals — EPEX day-ahead, AWS spot, real-time
rates:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let dp = DynamicPricing::builder()
    .unit("kWh")
    .currency(Currency::EUR)
    .interval(dec!(100), Amount::parse("0.10000")?)
    .interval(dec!(200), Amount::parse("0.20000")?)
    .build()?;

assert_eq!(dp.calculate()?.net_amount, Amount::parse("50.00000")?);
```

The resulting position shows the total quantity and the **weighted average** price,
which is informational: the net amount is set from the exact accumulated total, not
from `total_quantity × average_price`, so no rounding error is introduced by the
display figure.

### One rounding over the whole sequence

An interval is **not a leaf**. Unlike a time-of-use band or a schedule tier it never
becomes its own invoice line, so there is nothing at that level to round to — and
`Σ(qty × price)` is accumulated as exact `Decimal` and reduced once at the end.

The difference is not academic. A settled spot-price year is 8 760 hourly intervals,
or 35 040 quarter-hourly ones, and commercial rounding is not unbiased on a repeated
fractional part. Rounding each product to the engine's five decimals first therefore
drifts in *one* direction, by up to `5 × 10⁻⁶` per interval — `0.0438` over a year,
which is four whole cents on a two-decimal invoice, from arithmetic alone.

It is the same rule [`AmountScale::apply_decimal`](../amounts/) states at the
interchange boundary: two roundings are not one.

Negative prices are legally binding in spot markets, and the crate represents them.
A `Sign::Debit` position with a negative unit price stays a *debit* — that is what
distinguishes "consumption at a negative price" from "a credit", and levy and
commission layers depend on the distinction.

## `RateLookup` — a rate chosen by a parameter

Unlike a schedule, which prices *bands of consumption*, a rate lookup returns **one
uniform rate** selected by some other parameter — installed capacity, plant size,
contract class. All consumption is then priced at that single rate.

```rust
use billing::{Amount, RateLookup};
use rust_decimal::dec;

let eeg = RateLookup::builder()
    .at_most(dec!(10), Amount::parse("0.00811")?)
    .at_most(dec!(40), Amount::parse("0.00679")?)
    .fallback(          Amount::parse("0.00556")?)
    .build()?;

assert_eq!(eeg.rate_for(dec!(25))?, Amount::parse("0.00679")?);

// A negative parameter is a data error, not the cheapest band: every bound is
// positive, so a bare `<=` search would have returned the *lowest* rate.
assert!(eeg.rate_for(dec!(-1)).is_err());
```

Entries are sorted at build time, duplicate bounds are rejected as unreachable, a
parameter above every bound with no `fallback` is an error rather than a silent
fall-through to the last entry, and a negative parameter is refused — the same
"no silent under-billing" rule `RateSchedule::split` and `apply_peak` follow.

### `.ok()` on a table written as literals swallows the bug

`build()` returns `Result` because a rate table is often loaded from configuration,
where a bad ladder is a runtime condition. When the table is **written in the
source** — a statutory tier list, an EEG ladder, a regulated fee schedule — a
construction failure is an *authorship* bug, and the two must not be spelled the same
way:

```rust,ignore
// ✘ A non-contiguous ladder now returns the same answer as "no table for this
//   year", and the caller is told to look elsewhere rather than that it is wrong.
2023..=2026 => RateLookup::builder().at_most(..).build().ok(),

// ✔ The two states stay distinguishable, and the message names the table.
2023..=2026 => Some(RateLookup::builder().at_most(..).build()
                    .expect("EEG 2023 hydro ladder is malformed")),
_ => None,
```

There is deliberately no `build_static()` doing this for you: `.expect("…")` already
is one, and it carries a message the crate could not have written. The same applies
to every fallible builder here.

## Usage aggregation

Eight aggregators reduce a slice of raw events to the scalar a schedule prices:
`SUM`, `COUNT`, `UNIQUE_COUNT`, `MAX`, `MEAN`, `PERCENTILE`, `LATEST`,
`WEIGHTED_SUM`. Each takes a closure, so they work on your event type without a
trait impl.

```rust
use billing::aggregation::{MaxAggregator, SumAggregator, UsageAggregator};
use rust_decimal::Decimal;

struct Sample { seats: u32 }
let peak = MaxAggregator::new(|s: &Sample| Decimal::from(s.seats));
assert_eq!(
    peak.aggregate(&[Sample { seats: 10 }, Sample { seats: 50 }, Sample { seats: 30 }])?,
    Decimal::from(50u32)
);

// `aggregate` is fallible: this is the least trusted boundary in the pipeline, and
// `Decimal`'s `+` and `*` PANIC on overflow rather than saturating. An aggregator
// that accumulates therefore reports a corrupt magnitude instead of taking the
// batch run down with it.
assert!(SumAggregator::new(|_: &Sample| Decimal::MAX)
    .aggregate(&[Sample { seats: 1 }, Sample { seats: 1 }])
    .is_err());
```

`MAX` pairs with `RateSchedule::capacity()`; `WEIGHTED_SUM` is the CPU-hours shape,
where each resource contributes `value × fraction_of_period` — and is the one most
able to overflow, because it multiplies before it accumulates.

`COUNT`, `UNIQUE_COUNT`, `MAX`, `PERCENTILE` and `LATEST` select or count rather than
accumulate, so they never return `Err`. They share the signature anyway, because that
is what lets you write code generic over `UsageAggregator`.

### Burstable (95th-percentile) billing

Every IP transit, CDN egress and DDoS-scrubbing contract settles the same way, and no
other aggregator can express it. The provider samples throughput every five minutes,
discards the top 5 % of the samples, and bills the highest one left — so a customer
may burst above their committed rate for roughly 36 hours in a 30-day month and pay
nothing extra. `MAX` would bill the single worst five-minute window of the month;
`MEAN` would bill a rate the link never carried.

```rust
use billing::aggregation::{PercentileAggregator, UsageAggregator};
use rust_decimal::{Decimal, dec};

struct Sample { mbps: u32 }
let mbps = |s: &Sample| Decimal::from(s.mbps);

// Nineteen samples at 100 Mbps and one 900 Mbps burst.
let mut samples: Vec<Sample> = (0..19).map(|_| Sample { mbps: 100 }).collect();
samples.push(Sample { mbps: 900 });

assert_eq!(PercentileAggregator::p95(mbps).aggregate(&samples)?, dec!(100));
assert_eq!(PercentileAggregator::new(dec!(100), mbps)?.aggregate(&samples)?, dec!(900));
```

**Nearest-rank, never interpolated.** The samples are sorted ascending and the one at
rank `ceil(p ÷ 100 × n)` is returned. With the conventional 8 640 five-minute samples
in a 30-day month, "discard the top 5 %" removes 432 and the next highest is sample
8 208 — exactly `ceil(0.95 × 8640)`. The other common percentile definition averages
the two neighbouring samples, which would bill a throughput figure that never occurred
on the link.

`MEAN` is its evenly-spaced sibling: the average concurrent seats, the average daily
storage. When the samples are *not* interchangeable — a VM that ran three hours beside
one that ran a month — `WEIGHTED_SUM` is the aggregator that says so.

## The `PricingModel` trait

`PricingModel` is the extension point. It keeps pricing and tax separate — which is how
invoices are audited — and it makes "nothing to bill" a first-class outcome.

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;
use std::convert::Infallible;

struct PlatformPlan { monthly_fee_eur: u32 }
struct Seats { count: u32 }

impl PricingModel for PlatformPlan {
    type Usage = Seats;
    type Error = Infallible;
    type NotBillable = Infallible;   // this model always bills

    fn positions(&self, usage: &Seats) -> Result<Positions<Infallible>, Self::Error> {
        Ok(vec![LineItem::flat_fee(
            "Monthly platform fee",
            Amount::<5>::from_int(i64::from(self.monthly_fee_eur) * i64::from(usage.count)),
            Currency::EUR,
        ).build().unwrap()].into())
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![FixedRateTax::new("VAT", dec!(0.20)).unwrap().boxed()]
    }
}
```

### Three outcomes, not two

`Result<Vec<LineItem>, Error>` offers two answers, and real settlements have three. A
settlement can be **not billable yet, for a specific expected reason** — no meter
reading has arrived, the reference price for the period is unpublished, an entitlement
has ended. That is neither a set of positions nor a failure.

Collapsed into `Ok(vec![])` the reason is gone, and *"we billed nothing"* becomes
indistinguishable from *"there was nothing to bill, because X"* — a distinction every
audit trail needs and no caller can reconstruct afterwards. Collapsed into `Err` it
pollutes the error path with an ordinary business state.

So `positions` returns `Billing<Vec<LineItem>, R>`, where `R` is your own reason
type. Set `type NotBillable = Infallible` and the `NotBillable` variant becomes
uninhabited: the compiler knows the outcome is always billable, and `into_inner()`
unwraps it with no `Result`, no `unwrap` and no runtime check. A model that always
bills pays nothing for the third outcome existing.

### Settlements with no usage

Plenty of settlements are not usage-driven: a subsidy payout, a redispatch
compensation, an EEG or KWKG settlement whose figures were determined upstream. Set
`type Usage = ()` and call `settle`, which is `bill` without the `&()`:

```rust
use billing::prelude::*;
use std::convert::Infallible;

struct EegSettlement { payout_eur: i64 }

impl PricingModel for EegSettlement {
    type Usage = ();
    type Error = Infallible;
    type NotBillable = Infallible;

    fn positions(&self, _: &()) -> Result<Positions<Infallible>, Self::Error> {
        Ok(vec![
            LineItem::credit_fixed("EEG Vergütung", Amount::<5>::from_int(self.payout_eur))
                .build().unwrap(),
        ].into())
    }
}

let meta = DocumentMeta { currency: Currency::EUR, ..Default::default() };
let doc = EegSettlement { payout_eur: 400 }.settle(meta)?;
assert_eq!(doc.net_total(), Amount::<5>::parse("-400.00000")?);
```

There is deliberately no second trait for this case. One would save a
`type Usage = ();` line and one ignored parameter, at the cost of a second vocabulary
(`positions` versus `line_items`, `settle` versus `bill`) and a coherence hazard:
implementing both for one type is a compile error. One trait is the better trade.

## `PricingModel` is not `RateSchedule`

The names invite the wrong guess, so it is worth stating: a `RateSchedule` does
**not** implement `PricingModel`, and could not usefully.

They sit at different levels. A `RateSchedule` is a **rate table** — the banded
prices themselves — alongside `RateLookup`, `TimeOfUsePricing` and `DynamicPricing`. A
`PricingModel` is the **adapter** that decides which of those to consult, with what
quantities, in what order, and which tax and discount layers apply. A schedule is
something a model *uses*; it has nothing to say about tax layers.
