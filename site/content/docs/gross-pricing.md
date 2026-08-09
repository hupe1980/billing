+++
title = "Tax-inclusive pricing"
description = "Consumer catalogues are quoted gross and EN 16931 is net. The conversion is the VAT fraction, not the rate — and the invoice still will not total the shelf prices."
weight = 70
+++

## The conversion people get wrong

Consumer prices are quoted **including tax** almost everywhere: §1 PAngV in Germany,
and the equivalent across the EU, the UK and Australia. EN 16931 is uncompromisingly
**net** — BT-131 per line, BT-116 per VAT category, BT-109 for the document, with
BT-117 derived from BT-116 under `BR-CO-17`.

A gross price already contains the tax, so the rate applies to the *net* part — the
part that is unknown. Multiplying the gross by the rate over-states the tax by exactly
`rate²` of the net:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let gross = Amount::<5>::parse("119.00000")?;

// The plausible, wrong answer — 19 % too much output tax on every B2C line.
assert_eq!(gross.checked_mul_qty(dec!(0.19))?, Amount::parse("22.61000")?);

// The VAT fraction: 19/119, which is what every tax authority publishes for retail.
let tax = gross.extract_tax(dec!(0.19), RoundingStrategy::MidpointAwayFromZero)?;
let net = gross.net_of_tax(dec!(0.19), RoundingStrategy::MidpointAwayFromZero)?;
assert_eq!(tax, Amount::parse("19.00000")?);
assert_eq!(net, Amount::parse("100.00000")?);
```

The fraction is `r/(1+r)`: `19/119` at 19 %, `7/107` at 7 %, `1/6` at 20 %.

`net_of_tax` is defined as the **remainder** — `gross − tax` — not as an independent
rounding of `gross ÷ (1 + rate)`. So `net + tax == gross` holds *exactly* for the
amount it is applied to, under every rate and every strategy. Rounding both halves
separately fails that for roughly half of all inputs; the property test suite pins the
law across generated rates, strategies and magnitudes.

## Straight into a breakdown

`TaxBreakdownEntry::from_gross` does the same split directly into a BG-23 group. It
takes an `AmountScale` rather than a bare strategy, so the exact product is rounded
**once**, at the precision the document will be emitted at:

```rust
use billing::prelude::*;
use rust_decimal::dec;

// A shelf price of 9,99 € including 19 % VAT.
let entry = TaxBreakdownEntry::from_gross(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("9.99000")?, AmountScale::EN16931,
)?;

assert_eq!(entry.taxable_base, Amount::parse("8.39000")?); // BT-116
assert_eq!(entry.tax_amount,   Amount::parse("1.60000")?); // BT-117
entry.validate()?;                                         // BR-CO-17 holds
assert_eq!(entry.taxable_base.checked_add(entry.tax_amount)?, Amount::parse("9.99000")?);
```

Extracting at five decimals and reducing afterwards rounds twice, and two roundings
are not one. `BR-CO-17` defines BT-117 as the single rounding of `BT-116 × rate`, and
a validator recomputes it exactly that way.

## Which of the two to use

They are different constructions, not interchangeable ones:

| You have | Use | Because |
|---|---|---|
| gross-quoted **lines** | `net_of_tax` per line, then an ordinary `FixedRateTax` layer | `BR-S-08` fixes BT-116 as the sum of the line net amounts — the breakdown must follow the lines, not the other way round |
| a gross **total** and no lines — a receipt, a settlement figure | `TaxBreakdownEntry::from_gross` | there is no BT-131 to sum, so the group total is the only thing to derive from |

Do not mix them. Stating a `from_gross` breakdown next to net lines that sum to
something else is exactly the `BR-S-08` violation
[`verify_vat_attribution`](../vat-breakdown/#per-position-attribution) reports.

## The invoice will not total the sum of the shelf prices

This is worth stating bluntly, because it looks like a bug and is not.

Convert three gross-quoted lines and the document will usually land a cent or two away
from adding the price tags up:

| | |
|---|---|
| 3 × 249,99 + 2 × 529,00 + 4 × 89,95 | **2 167,77** — the sum of shelf prices |
| BT-109 1 762,41 + BT-110 405,35 | **2 167,76** — what the invoice states |

Nothing was lost in the conversion. VAT on an invoice is charged **once per category**,
not once per line: `BR-CO-17` defines BT-117 as a single rounding of the *group* total,
and a validator recomputes it that way. Here the per-line taxes sum to 405,36 while the
tax on the summed base is 405,35.

The shelf-price total is the figure built from independently rounded numbers — and it
is the one with no standing in EN 16931. The gap is bounded by roughly one minor unit
per line, because each line's discarded half-cent is folded into one per-category
rounding instead of being rounded on its own.

> **Do not try to force the two to agree.** Nudging a line so the document totals the
> catalogue sum trades a rule a validator checks (`BR-CO-17`) for an identity the
> standard does not have.

`examples/cross_border_retail.rs` in the repository works the whole case end to end and
prints the difference rather than hiding it.

## Margin schemes

The margin scheme for second-hand goods, works of art and travel services (arts.
312–325 and 306–310 of the VAT Directive, §25a UStG) is the same arithmetic applied to
a different base: the tax is contained in the **margin**, not in the selling price.

```rust
use billing::{Amount, RoundingStrategy};
use rust_decimal::dec;

let purchase = Amount::<5>::parse("800.00000")?;
let selling  = Amount::<5>::parse("1000.00000")?;
let margin   = selling.checked_sub(purchase)?;

// The tax is inside the margin, so it is extracted, not added:
// 200.00 × 19/119 = 31.93277, not 200.00 × 0.19 = 38.00.
let tax = margin.extract_tax(dec!(0.19), RoundingStrategy::MidpointAwayFromZero)?;
assert_eq!(tax, Amount::parse("31.93277")?);
```

What the *invoice* then shows is a jurisdictional question the engine does not answer:
a margin-scheme invoice must generally not state the VAT separately, and must carry the
prescribed wording instead. The revision of EN 16931 published in 2026 adds structured
support for margin schemes; the arithmetic above does not change with it.
