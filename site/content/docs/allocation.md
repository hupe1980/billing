+++
title = "Allocation and proration"
description = "Split a document across recipients, or a quantity across fractions, with no cent created or lost — and why an allocated document needs re-assembly before emission."
weight = 110
+++

## Splitting a document

`AllocationRule` splits a whole `BillingDocument` across N recipients — a shared meter,
a co-working floor, a building's occupants.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("Verbrauch", Amount::parse("100.00000")?).build()?],
    vec![], vec![],
)?;

let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)])?;
let parts = alloc.allocate(&doc)?;

let sum: Amount<5> = parts.iter().map(|d| d.net_total()).sum();
assert_eq!(sum, doc.net_total());          // exact — no drift
for d in &parts {
    d.assert_valid();                      // each is internally consistent
    d.verify_vat_attribution()?;           // BR-S-08 survives the split
}
```

`EqualAllocation::new(n)` is the `1/n` case; it bypasses the sum-to-one check because
`3 × 0.333…` cannot be exactly 1, and the largest-remainder step inside each position's
split absorbs the difference.

### Five guarantees

1. `Σ(net_total) == original.net_total` — exactly.
2. `Σ(tax_total) == original.tax_total` — exactly.
3. `Σ(gross_total) == original.gross_total` — exactly.
4. Every recipient document passes `validate()`.
5. Every recipient document passes `verify_vat_attribution()` — **BR-S-08** — if the
   document it was split from did.

All five rest on one decision: **each position is split exactly**, so
`Σ(recipients) == the original position` holds line by line and not merely in
aggregate. Every total is then derived by summing a recipient's own positions, which
makes (1)–(3) follow immediately, and each VAT group's taxable base is read off the
very positions BR-S-08 checks it against, which gives (5). (3) additionally derives
gross as `net + tax` rather than rounding it — rounding all three separately breaks
`net + tax == gross` for shares that do not divide evenly.

#### Why not split the totals and correct the positions?

That is the obvious construction, it was the one used until 0.13.0, and it quietly
broke BR-S-08. The breakdown was split per `(category, rate)` group while the
positions were corrected in aggregate, so a group's BT-116 and the positions
attributed to it could land a minor unit apart. On a two-rate document that happened
for roughly **45 %** of recipients — every document internally exact, every one of
them rejected by a validator.

That is also why there is no "this came from an allocation, skip the check" flag:
there is nothing left to skip.

The VAT breakdown, BT-113, BT-114 and BT-111 are all split the same way. Dropping any
of them would re-bill money the customer already handed over, or leave a recipient
naming a VAT accounting currency with nothing in it.

### What allocation refuses

- a document carrying a **cash-rounding rule** — a rounding adjustment is a property of
  one payable total, and scaling it by a share generally lands off the increment.
  Allocate first, then apply rounding per recipient.
- a document carrying **itemised advances** — see [Advance payments](../advances/).
- negative shares, even if they sum to one: that hands one recipient a credit funded by
  the others, and the sum test alone cannot detect it.

## Exactness and invoice precision pull in opposite directions

This is the trade-off worth understanding before you allocate.

`AmountScale` **does not survive allocation, and cannot.** Splitting `100.00` three
ways is `33.333…`, and there is no two-decimal answer. Allocation keeps the *sum* exact
— which is the right trade for money, since inventing or losing a cent is worse than
carrying a third decimal — at the cost of the precision an invoice needs.

So an allocated document must be **re-assembled** at invoice precision before it is
billed onward, and the split is what then absorbs the difference:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![LineItem::fixed("Verbrauch", Amount::parse("47.10000")?).build()?])
    .build()?;
assert!(doc.fits_amount_scale(2));               // the invoice is emittable

let parts = EqualAllocation::new(3)?.allocate(&doc)?;
// 47.10 / 3 = 15.70 exactly here, but in general a share will not fit two decimals.
assert_eq!(parts.iter().map(|d| d.net_total()).sum::<Amount<5>>(), doc.net_total());
```

`fits_amount_scale(2)` and `amount_scale_violation(2)` are how you find out; the second
names the field so you know what a consumer would actually emit.

`reverse()`, by contrast, **does** preserve the scale: a credit note of a two-decimal
invoice is two decimals.

## Splitting a raw quantity

`proportional_split` splits a bare `Decimal` — kWh, m³, capacity — before any document
exists. It uses the **largest-remainder (Hamilton) method**: floor each part to `scale`
decimals, then distribute the deficit one unit at a time to the fractions with the
largest discarded remainders, ties broken by position.

```rust
use billing::proportional_split;
use rust_decimal::dec;

let parts = proportional_split(dec!(100), &[dec!(0.333), dec!(0.333), dec!(0.334)], 3)?;
let total: rust_decimal::Decimal = parts.iter().sum();
assert_eq!(total, dec!(100));
```

`Σ(parts) == total` exactly, every part is rounded to exactly `scale` decimals, and the
correction is spread as evenly as the deficit allows rather than concentrated on one
part.

The share-sum tolerance scales with `total` rather than being a fixed `1e-9`. A fixed
absolute tolerance lets the resulting error grow without bound: at `total = 1e18`,
shares summing to `1 − 5e-10` pass the check and leave a deficit of ~1e9 units, which
the distribution step could never close — silently returning parts that sum to less
than `total`, breaking the function's one guarantee. Tying the tolerance to what the
distribution can actually absorb removes that.

### …and a bare `Amount` by fractional shares

`Amount::allocate` takes **integer** ratios and `Amount::distribute` splits evenly, so
neither covers shares such as `[0.4, 0.35, 0.25]`. Compose `proportional_split` at the
amount's own precision and the round trip is exact:

```rust
use billing::{Amount, proportional_split};
use rust_decimal::dec;

let total = Amount::<5>::parse("100.00000")?;
let parts: Vec<Amount<5>> =
    proportional_split(total.into_decimal(), &[dec!(0.4), dec!(0.35), dec!(0.25)], 5)?
        .into_iter()
        .map(Amount::<5>::checked_from_decimal)
        .collect::<Result<_, _>>()?;

assert_eq!(Amount::checked_sum(parts.into_iter())?, total);
```

Passing `scale = 5` is what makes the conversion total: every part then carries at most
five decimals, so `checked_from_decimal` — which *refuses* excess precision rather than
rounding it — always succeeds, and `Σ(parts) == total` carries straight through. A
coarser scale is fine; a finer one fails loudly instead of silently re-rounding.

## Proration

`prorate` scales a position to the fraction of a period actually used — a customer
joining on the 15th, an annual fee cancelled after 100 days:

```rust
use billing::prelude::*;

let fee = LineItem::fixed("Grundpreis", Amount::parse("30.00000")?).build()?;
let half = prorate(&fee, 15, 30, RoundingStrategy::MidpointAwayFromZero)?;
assert_eq!(half.net_amount, Amount::parse("15.00000")?);
assert!(half.description.contains("prorated 15/30d"));
```

It scales the **quantity** alongside the amount, not just the net. Scaling the net
alone would leave the line self-contradictory — "1000 kWh × 0.30 EUR/kWh = 150.00"
after a half-month proration — and would break `PEPPOL-EN16931-R120`, which recomputes
`BT-131 = BT-129 × (BT-146 ÷ BT-149) + Σ BG-28 − Σ BG-27`. Line allowances and the
BT-93/BT-100 basis are rescaled with it for the same reason. The unit price is left
alone, because a price is a rate and does not scale.

The period is **cleared**, because this function knows day counts and not dates. Set it
explicitly if it matters.

`prorate_amount` is the same arithmetic on a bare `Amount<5>`.

## Merging half-periods

A tariff change mid-period produces two documents for one invoice. `merge_period_documents`
concatenates the positions, sums the totals, and merges the two VAT breakdowns so that
two half-period documents at the same rate produce **one** breakdown line rather than
two.

It refuses a currency mismatch, itemised advances, cash-rounding rules, and an
asymmetric BT-111. It also re-runs `validate()` on the result, because two individually
lawful halves do not always compose into a lawful whole: merging an "outside the scope
of VAT" document with a taxed one produces a breakdown that `BR-O-11` forbids, and only
the combined document can show it.

The merged document keeps `doc_a`'s header. Set the combined period explicitly
afterwards if it matters.
