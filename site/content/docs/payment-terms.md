+++
title = "Payment terms"
description = "The arithmetic of payment terms: the Skonto amount and the per-group VAT correction it implies, and statutory late-payment interest rounded once across rate changes. The BT-20 text is the format's."
weight = 100
+++

The BT-20 text (including the `BR-DE-18` Skonto syntax) belongs to
[`en16931`](https://crates.io/crates/en16931); what a term means in money is here.

## Skonto: `EarlyPaymentDiscount`

An early-payment discount is a percentage of the amount due (BT-115).
`amount(&doc)` is `round(BT-115 × percent / 100)` once, at the document's scale:

```rust
use billing::{Amount, AmountScale, Currency, Document, EarlyPaymentDiscount, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("service", dec!(1000)))
    .build()?;
let skonto = EarlyPaymentDiscount { percent: Percent::new(dec!(2)) };

assert_eq!(doc.payable(), Amount::parse("1190")?); // BT-115 states the full amount …
assert_eq!(skonto.amount(&doc)?, Amount::parse("23.80")?);
assert_eq!(doc.payable() - skonto.amount(&doc)?, Amount::parse("1166.20")?); // … this is paid if taken
```

### The discount is not a discount on the invoice

A Skonto is **conditional**, so it changes no total of the invoice. When it is taken
the VAT follows (§17 Abs. 1 UStG, art. 90 VAT Directive): `vat_correction(&doc)` splits
the discount across the VAT groups by gross, then each share into net and tax with
[`split_gross`](../gross-pricing/). The rows sum to the discount exactly:

```rust
use billing::{Amount, AmountScale, Currency, Document, EarlyPaymentDiscount, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("goods", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(19)))?))
    .line(Line::fixed("books", dec!(50)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
    .build()?;
let skonto = EarlyPaymentDiscount { percent: Percent::new(dec!(3)) };
assert_eq!(skonto.amount(&doc)?, Amount::parse("5.18")?); // 172.50 × 3 % = 5.175

// 5.18 split 53.50 : 119.00, then each share split into net and tax.
let rows = skonto.vat_correction(&doc)?;
assert_eq!((rows[0].rate_percent(), rows[0].taxable_base(), rows[0].tax()), (Percent::new(dec!(7)), Amount::parse("1.50")?, Amount::parse("0.11")?));
assert_eq!((rows[1].rate_percent(), rows[1].taxable_base(), rows[1].tax()), (Percent::new(dec!(19)), Amount::parse("3.00")?, Amount::parse("0.57")?));
let total: Amount = rows.iter().map(|g| g.taxable_base() + g.tax()).sum();
assert_eq!(total, skonto.amount(&doc)?);

// Sign-symmetric: the correction of a credit is the negated correction.
let credit = skonto.vat_correction(&doc.negated())?;
assert_eq!(credit[1].tax(), -rows[1].tax());
```

Booking the correction is the caller's step; a VAT-free document gets none.

## Late-payment interest

Statutory default interest is **simple** interest (Directive 2011/7/EU; §288 BGB)
whose rate can change during the delay. Each `InterestSegment` has its own annual
rate, days and day-count denominator; `late_payment_interest` sums them **exactly** and
rounds once:

```rust
use billing::{Amount, AmountScale, InterestSegment, Percent, late_payment_interest};
use rust_decimal::dec;

let interest = late_payment_interest(
    Amount::parse("1000")?,
    &[
        InterestSegment { annual_percent: Percent::new(dec!(9.12)), days: 37, year_days: 365 },
        InterestSegment { annual_percent: Percent::new(dec!(8.62)), days: 30, year_days: 365 },
    ],
    AmountScale::EN16931,
)?;
// 9.2448 + 7.0849 = 16.3297 → 16.33; rounding each segment gives 9.24 + 7.08 = 16.32.
assert_eq!(interest, Amount::parse("16.33")?);
```

The day-count denominator (365, 366, 360) is the contract's choice; counting days is
the caller's. Default interest is generally outside the scope of VAT — bill it with an
[`O` line](../vat-breakdown/#out-of-scope).
