+++
title = "Migrating from 0.15"
description = "0.16 is a breaking release without deprecation shims. The mechanical steps and the changes that compile but give different values; the CHANGELOG has the full old → new table."
weight = 140
+++

0.16 has no deprecation shims: every removed name is gone, so the compiler lists
every affected site. The
[CHANGELOG](https://github.com/hupe1980/billing/blob/main/CHANGELOG.md) has the full
old → new table; this page is the short version.

## Mechanical steps

1. **`BillingDocument` → `Document`**, built with `Document::builder(currency, scale)`.
2. **Tax layers → VAT attributes.** Put a `Vat` on each line and adjustment, or use
   `default_vat`. A per-unit levy is a line (`per_unit_levy`).
3. **Every percentage is a `Percent`.** `FixedRateTax::new("VAT", dec!(0.19))` becomes
   `Vat::standard(Percent::new(dec!(19)))`; a bare `Decimal` does not compile.
4. **Drop `validate()` and friends.** Their checks run inside `build()`, which returns a
   located [`Error`](../errors/) (`BillingError` and `ParseAmountError` are both `Error`).
5. **Move the header to [`en16931`](../e-invoicing/#the-layer-above):** invoice
   numbers, dates, parties, BT-3 type codes and the BT-20 payment terms text.
6. **Replace the helpers:**

| 0.15 | 0.16 |
|---|---|
| `LineItem::for_usage(..)`, `LineItem::fixed` | `Line::new(d, Quantity::new(q, "KWH"), UnitPrice::new(Price::new(p)))`, `Line::fixed` |
| `RateSchedule`, `DynamicPricing`, `TimeOfUsePricing` | `Tariff`, `rate_series`, `rate_by_key` ([Pricing](../pricing/)) |
| `*Aggregator` | `Aggregation` ([Metering](../metering/)) |
| `distribute`, `allocate`, `prorate` | `split`, `split_matrix`, `Line::segments`, `Document::split` |
| `minimum_charge*`, `maximum_charge*` | `Bound::minimum` / `Bound::maximum` |
| `with_prepaid`, `with_advances` | `.payment(Payment::Gross { .. })`, `.payment(Payment::Advance(..))` |
| `merge_period_documents`, `reverse(meta)` | `Document::concat`, `negated()` |

## Changes that compile and give different values

- **Every leaf rounds once at the document's scale**, not via five decimals first; a
  half-unit amount can move by one cent, towards what a validator recomputes.
- **VAT is rounded once per group**, and document allowances lower their group's
  base (`BR-CO-17`), so former tax-layer users get a different, correct BT-110.
- **Skonto and interest round at the document's scale.**
- **`Rounding` has no floor or ceiling**; a credit rounds like the mirrored debit.
- **`negated` is not `reverse`**: it negates amounts and writes no header.
- **`Amount::parse("1,000")` is refused**, and `{:.2}` of an `Amount` now honours the
  precision.
- **`Quantity::new`'s second argument is the BT-130 unit code** (`"KWH"`, not `"kWh"`).
- **`Tiers::new` refuses unsorted tiers** instead of sorting them.
- **Payments may exceed the supply**: BT-115 goes negative instead of being refused.
