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
| `capacity` | the peak selects a tier; a flat charge is billed | one position |

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
```

Entries are sorted at build time, duplicate bounds are rejected as unreachable, and a
parameter above every bound with no `fallback` is an error rather than a silent
fall-through to the last entry.

## Usage aggregation

Six aggregators reduce a slice of raw events to the scalar a schedule prices:
`SUM`, `COUNT`, `UNIQUE_COUNT`, `MAX`, `LATEST`, `WEIGHTED_SUM`. Each takes a closure,
so they work on your event type without a trait impl.

```rust
use billing::aggregation::{MaxAggregator, UsageAggregator};
use rust_decimal::Decimal;

struct Sample { seats: u32 }
let peak = MaxAggregator::new(|s: &Sample| Decimal::from(s.seats));
assert_eq!(
    peak.aggregate(&[Sample { seats: 10 }, Sample { seats: 50 }, Sample { seats: 30 }]),
    Decimal::from(50u32)
);
```

`MAX` pairs with `RateSchedule::capacity()`; `WEIGHTED_SUM` is the CPU-hours shape,
where each resource contributes `value × fraction_of_period`.

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

An earlier release had a **second trait** for this, with its own method names and a
blanket impl. It saved one `type Usage = ();` line and one ignored parameter, and cost
a second vocabulary (`positions` versus `line_items`, `settle` versus `bill`) plus a
coherence hazard the documentation had to warn about: implementing both traits for one
type was a compile error. One trait is the better trade, and 0.12.0 removed the second.

## Why `PricingModel` and not `Tariff`

Until 0.12.0 this trait was `Tariff`, and the rate table was `TariffSchedule`.

*Tariff* is perfectly good English for a published schedule of rates — the sense is as
old as the customs-duty one, both attested from the 1590s (Arabic `taʿrīf` → Italian
`tariffa`), and UK primary legislation is titled the *Domestic Gas and Electricity
(Tariff Cap) Act 2018*. The word was never the problem.

Being the **neutral** word was. This crate advertises zero domain assumptions, and
every other public name keeps that promise — `LineItem`, `Quantity`, `UnitPrice`,
`TaxLayer`, `UsageAggregator`, `AllocationRule`, `RateLookup`. `Tariff*` were the only
three exceptions out of roughly forty, and they carry the vocabulary of utilities and
telecoms specifically. Two of the four worked examples are SaaS and cloud, where the
word is *plan*, *pricing model* or *rate card* — and in American English the dominant
sense of *tariff* is an import duty, which this crate has nothing to do with.

`PricingModel` is the phrase all of those domains already use for "how we work out what
to charge". `RateSchedule` and `RateBand` join `RateLookup` in a neutral family of rate
tables.

## `PricingModel` is not `RateSchedule`

The names invite the wrong guess, so it is worth stating: a `RateSchedule` does
**not** implement `PricingModel`, and could not usefully.

They sit at different levels. A `RateSchedule` is a **rate table** — the banded
prices themselves — alongside `RateLookup`, `TimeOfUsePricing` and `DynamicPricing`. A
`PricingModel` is the **adapter** that decides which of those to consult, with what
quantities, in what order, and which tax and discount layers apply. A schedule is
something a model *uses*; it has nothing to say about tax layers.
