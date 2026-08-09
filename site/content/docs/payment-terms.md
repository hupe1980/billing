+++
title = "Payment terms and Skonto"
description = "BT-20 is one free-text field, so an early-payment discount is not machine-readable in the core standard. The BR-DE-18 syntax, and why a Skonto must not touch the taxable base."
weight = 90
+++

## One free-text field

EN 16931 models payment terms as a single free-text term:

> **BT-20** *Payment terms* — A textual description of the payment terms that apply to
> the amount due for payment (Including description of possible penalties).

An early-payment discount — *Skonto*, *escompte*, *sconto*, "2/10 net 30" — is
therefore not machine-readable in the core standard at all, even though it changes what
the buyer actually transfers.

Germany's CIUS closes the gap with a micro-syntax embedded in that free text, made
mandatory by **`BR-DE-18`** and read by ZUGFeRD too:

```text
#SKONTO#TAGE=10#PROZENT=2.00#
#SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#
```

Uppercase, no whitespace, `#` opening every segment and closing the line, the
percentage with exactly two decimals, one line per discount.

## The syntax as a type

`PaymentTerms` renders and parses exactly that form, and neither direction can produce
something a validator rejects:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let terms = PaymentTerms::text("Zahlbar in 30 Tagen.")
    .with_discount(EarlyPaymentDiscount::new(10, dec!(3))?)
    .with_discount(EarlyPaymentDiscount::new(20, dec!(2))?);

assert_eq!(
    terms.to_string(),
    // The trailing newline is the second half of BR-DE-18 — see below.
    "Zahlbar in 30 Tagen.\n#SKONTO#TAGE=10#PROZENT=3.00#\n#SKONTO#TAGE=20#PROZENT=2.00#\n",
);
assert_eq!(terms.to_string().parse::<PaymentTerms>()?, terms);

// Staged terms award the best qualifying window — what the buyer will take.
assert_eq!(terms.discount_within(5).map(|d| d.percent()),  Some(dec!(3)));
assert_eq!(terms.discount_within(15).map(|d| d.percent()), Some(dec!(2)));
assert_eq!(terms.discount_within(30), None);
```

Parsing is **strict by design**. A lowercase keyword, a leading space, a missing
closing `#`, an unknown segment key or a third decimal is refused rather than
normalised, because a validator enforcing `BR-DE-18` refuses it too. Accepting a sloppy
form would let it round-trip into an invoice that fails at the access point instead of
in your code.

A percentage or base amount the syntax cannot write is rejected at construction, not
silently rounded at render time — `2.005 %` is an error, because `#PROZENT=2.00#` would
state something else.

`BASISBETRAG` is only needed when the discount does **not** apply to the whole amount
due: a part-discountable invoice, or one where freight is excluded.

## The discount is not a discount on the invoice

A Skonto is **conditional**. It exists only if the buyer pays early, which has not
happened when the invoice is issued. Deducting it up front would understate the taxable
base for a supply that may well be paid at full price.

So attaching payment terms moves **nothing** — not `net_total`, not `gross_total`, not
`amount_due`, not the VAT breakdown:

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let skonto = EarlyPaymentDiscount::new(14, dec!(2))?;
let doc = BillingDocument::from_positions(
    DocumentMeta {
        currency: Currency::EUR,
        payment_terms: Some(PaymentTerms::text("Zahlbar in 30 Tagen.").with_discount(skonto)),
        ..Default::default()
    },
    vec![LineItem::fixed("Leistung", Amount::parse("1000.00000")?).build()?],
    vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()],
    vec![],
)?;

// BT-115 states the full amount …
assert_eq!(doc.amount_due()?, Amount::parse("1190.00000")?);
// … and this is what is transferred if the discount is taken. It is not BT-115.
assert_eq!(skonto.payable_on(doc.amount_due()?)?, Amount::parse("1166.20000")?);
```

The deduction comes off the **gross**, which is precisely why taking it later requires
a VAT correction rather than a smaller invoice: the consideration shrinks after the
fact and the tax follows it — §17 Abs. 1 UStG in Germany, art. 90 of the VAT Directive
elsewhere.

That correction is triggered by a **payment event**, which the invoice cannot know. So
the crate computes the payable figure and stops there. What it deliberately does *not*
do is rewrite the breakdown for you, because whether the discount was taken is
information that arrives days later, from a bank statement.

## Where this is heading

The revision of EN 16931 approved by CEN in October 2025 and published in 2026 promotes
early payment discounts and late-payment charges to **structured fields of their own**.

The arithmetic above does not change with it — only where the numbers are written. The
accessors (`days()`, `percent()`, `rate()`, `base_amount()`) are what a future
serialiser maps from, and the `#SKONTO#` rendering remains what current XRechnung and
ZUGFeRD validators expect.
