+++
title = "Settlement"
description = "BT-113 payments and advances, BT-114 cash rounding, BT-111 VAT in an accounting currency, customer credits drawn before or after tax, and the residual VAT of a final invoice."
weight = 90
+++

Payments, cash rounding and VAT accounting act after the tax-inclusive total (BT-112)
and never change a line, a VAT group or BT-109 … BT-112:

```text
BT-115 = BT-112 − BT-113 + BT-114
```

## Payments (BT-113)

What was already paid is a list of `Payment` entries, appended with
`DocumentBuilder::payment`:

- `Payment::Advance(Advance { vat, net, tax })` — an advance that was **invoiced**
  earlier, restated as it was invoiced, not re-derived;
- `Payment::Gross { description, amount }` — a gross amount without VAT information,
  e.g. a deposit or a bank transfer.

BT-113 is the sum of the entries' grosses. `payments()` returns the entries given;
`paid_entries()` adds each drawn after-tax credit — everything BT-113 sums. Payments
may exceed the supply: BT-115 then goes negative (a *Guthaben*), never clamped.

```rust
use billing::{Advance, Amount, AmountScale, Currency, Document, Line, Percent, Payment, Vat};
use rust_decimal::dec;

let vat = Vat::standard(Percent::new(dec!(19)))?;
let monthly = Advance { vat: vat.clone(), net: dec!(100), tax: dec!(19) };

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(vat)
    .line(Line::fixed("annual supply", dec!(1000)))
    .payments(vec![monthly; 12].into_iter().map(Payment::Advance))
    .build()?;

assert_eq!(doc.tax_inclusive_total(), Amount::parse("1190")?); // the supply, in full
assert_eq!(doc.vat_total(), Amount::parse("190")?);            // VAT on the full base
assert_eq!(doc.prepaid(), Amount::parse("1428")?);             // BT-113: 12 × 119
assert_eq!(doc.payable(), Amount::parse("-238")?);             // BT-115: the refund
```

> **Never model prepayments as negative lines or as allowances.** That shrinks the
> taxable base and under-declares output tax. In Germany, an *Endrechnung* that does
> not deduct its advances correctly makes the VAT stated on them payable a second time
> (§14c Abs. 1 UStG).

Every payment amount must fit the document's scale (`ErrorKind::ExcessPrecision` at
`Location::Payment(i)`). `negated`, `split` and `concat` keep every entry, so a split
or reversed final invoice still knows its per-rate advances.

### Advance invoices are ordinary documents

An advance invoice (*Abschlagsrechnung*) is a `Document` with a line for the advance;
type code 386 and references are [`en16931`](https://crates.io/crates/en16931)'s. The
final invoice restates each advance's net and tax as an `Advance`.

## Cash rounding (BT-114)

Where the smallest coin is larger than the smallest unit (0.05 CHF), the amount due
is rounded to a payable increment and the difference is BT-114:

```rust
use billing::{Amount, AmountScale, CashRounding, Currency, Document, Line, Percent, Rounding, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::CHF, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(8.1)))?)
    .line(Line::fixed("service", dec!(9.27)))
    .cash_rounding(CashRounding { increment: dec!(0.05), rounding: Rounding::HalfAwayFromZero })
    .build()?;

assert_eq!(doc.tax_inclusive_total(), Amount::parse("10.02")?); // 9.27 + 0.75
assert_eq!(doc.rounding(), Amount::parse("-0.02")?);            // BT-114
assert_eq!(doc.payable(), Amount::parse("10.00")?);             // BT-115
```

The rounding acts on `BT-112 − BT-113`, outside the VAT base. The increment must be
above zero and fit the scale (`ErrorKind::InvalidCashRounding`). A
[split](../allocation/#document-split) share is cash-rounded on its own.

## VAT in an accounting currency (BT-6, BT-111)

A `VatAccounting` (currency and rate) sets BT-6 and BT-111, e.g. the PLN VAT total of
a EUR invoice. BT-111 is `round(BT-110 × rate)` **once**:

```rust
use billing::{Amount, AmountScale, Currency, Document, ErrorKind, Line, Percent, Vat, VatAccounting};
use rust_decimal::dec;

let pln = Currency::new("PLN")?;
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("consulting", dec!(1000)))
    .vat_accounting(VatAccounting { currency: pln, rate: dec!(4.2817) })
    .build()?;
assert_eq!(doc.vat_accounting_total(), Some((pln, Amount::parse("813.52")?))); // 190 × 4.2817 = 813.523

// The accounting currency must differ from the invoice currency.
let err = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("x", dec!(1)))
    .vat_accounting(VatAccounting { currency: Currency::EUR, rate: dec!(1) })
    .build()
    .unwrap_err();
assert_eq!(err.kind(), &ErrorKind::InvalidVatAccounting);
assert_eq!(err.rule(), Some("PEPPOL-EN16931-R005"));
```

The rate is an input only (EN 16931 has no exchange-rate term). A rate ≤ 0 is
`ErrorKind::InvalidRate`, so BT-111 always has BT-110's sign (`PEPPOL-EN16931-R055`).

## Credits

A `Credit` is a customer balance from your ledger (a credit grant, a wallet). The
builder draws the credits against **one** document, in call order, and reports what it
applied (`credit_applications()`). The build order is:

```text
lines → apportioned adjustments → bounds → before-tax credits → VAT
      → payments → after-tax credits → cash rounding
```

**Before tax** (`Credit::before_tax`, a promotional credit) draws at most BT-109 and
becomes one document allowance per VAT group, split by the group bases, so VAT is
derived on the credited bases.

**After tax** (`Credit::after_tax`, a balance already paid for) draws at most what is
still due and is added to BT-113 as a gross payment entry. The tax is untouched.

```rust
use billing::{Amount, AmountScale, Credit, Currency, Document, Line, Percent, Payment, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("subscription", dec!(100)))
    .credit(Credit::before_tax("A", dec!(30)))
    .credit(Credit::after_tax("B", dec!(100)))
    .build()?;
assert_eq!(doc.tax_exclusive_total(), Amount::parse("70")?); // A drawn before tax
assert_eq!(doc.vat_total(), Amount::parse("13.30")?);        // 70 × 19 %
assert_eq!(doc.prepaid(), Amount::parse("83.30")?);          // B, like a payment
assert_eq!(doc.payable(), Amount::parse("0")?);
let b = doc.credit_applications()[1];
assert_eq!((b.applied, b.remaining), (Amount::parse("83.30")?, Amount::parse("16.70")?));

// With 100.00 of the 119.00 already prepaid, a wallet of 50.00 covers only 19.00.
let paid = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("subscription", dec!(100)))
    .payment(Payment::Gross { description: "prepayment".into(), amount: dec!(100) })
    .credit(Credit::after_tax("wallet", dec!(50)))
    .build()?;
let w = paid.credit_applications()[0];
assert_eq!((w.applied, w.remaining), (Amount::parse("19")?, Amount::parse("31")?));
```

A balance must be above zero and fit the scale (`ErrorKind::InvalidCredit` at
`Location::Credit(i)`). A credit note draws nothing, and no credit makes BT-115
negative. Credits are inputs: `into_builder` re-applies them; `negated`, `split` and
`concat` turn the drawn amounts into ordinary allowances and `Payment::Gross` entries.

### Scoped credits

A credit on some lines only is an allowance built with `apportion` and a filter:

```rust
use billing::{Adjust, AdjustmentKind, Amount, AmountScale, Currency, Document, Line, Percent, Vat, apportion};
use rust_decimal::dec;

let v19 = Vat::standard(Percent::new(dec!(19)))?;
let lines = [
    Line::fixed("platform fee", dec!(100)).with_vat(v19.clone()),
    Line::fixed("usage", dec!(300)).with_vat(v19),
    Line::fixed("e-books", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?),
];
let scale = AmountScale::EN16931;
let credit = apportion(AdjustmentKind::Allowance, "usage credit", Adjust::Fixed(dec!(40)),
    &lines, |l| l.description() != "platform fee", scale)?;
let doc = Document::builder(Currency::EUR, scale).lines(lines).adjustments(credit).build()?;
assert_eq!(doc.tax_exclusive_total(), Amount::parse("460")?);
assert_eq!(doc.vat_total(), Amount::parse("76.60")?); // 370 × 19 % + 90 × 7 %
```

## Commitments

A commitment true-up is a minimum bound on the net total. Bounds settle before
credits, so a before-tax credit is drawn from the trued-up total:

```rust
use billing::{Amount, AmountScale, Basis, Bound, Credit, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("usage", dec!(600)))
    .bound(Bound::minimum(Basis::Net, dec!(1000), "commitment true-up"))
    .credit(Credit::before_tax("prepaid credit", dec!(100)))
    .build()?;
assert_eq!(doc.line_net(1), Amount::parse("400")?);           // the top-up
assert_eq!(doc.tax_exclusive_total(), Amount::parse("900")?);
```

## Final invoice: the residual VAT

A final invoice after advances (*Schlussrechnung*) must state, per rate, the tax left
after the advances (§14 Abs. 5 S. 2 UStG). BG-23 still covers the whole supply, so
`Document::residual_vat` reports the per-group residual beside it:

```rust
use billing::{Advance, Amount, AmountScale, Currency, Document, ErrorKind, Line, Location, Percent, Payment, Vat};
use rust_decimal::dec;

let (v19, v7) = (Vat::standard(Percent::new(dec!(19)))?, Vat::standard(Percent::new(dec!(7)))?);
let supply = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("works", dec!(1000)).with_vat(v19.clone()))
    .line(Line::fixed("books", dec!(200)).with_vat(v7.clone()));
let doc = supply.clone()
    .payments(vec![
        Advance { vat: v19, net: dec!(300), tax: dec!(57) },
        Advance { vat: v7, net: dec!(100), tax: dec!(7) },
    ].into_iter().map(Payment::Advance))
    .build()?;
let r = doc.residual_vat()?;
assert_eq!((r[0].taxable_base(), r[0].tax()), (Amount::parse("100")?, Amount::parse("7")?));   // 7 %
assert_eq!((r[1].taxable_base(), r[1].tax()), (Amount::parse("700")?, Amount::parse("133")?)); // 19 %
assert_eq!(doc.vat_total(), Amount::parse("204")?); // BG-23 still covers the whole supply

// An advance at a rate the document does not have cannot be deducted anywhere.
let odd = supply
    .payments(vec![
        Advance { vat: Vat::standard(Percent::new(dec!(16)))?, net: dec!(100), tax: dec!(16) },
    ].into_iter().map(Payment::Advance))
    .build()?;
let err = odd.residual_vat().unwrap_err();
assert_eq!((err.kind(), err.location()), (&ErrorKind::UnmatchedAdvance, Some(&Location::Payment(0))));
```

Gross payments never enter the statement. It survives transforms: split halves state
their share, a reversal the negated residual.

```rust
use billing::{Advance, Amount, AmountScale, Credit, Currency, Document, Line, Payment, Percent, Vat};
use rust_decimal::dec;

let (v19, v7) = (Vat::standard(Percent::new(dec!(19)))?, Vat::standard(Percent::new(dec!(7)))?);
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("works", dec!(1000)).with_vat(v19.clone()))
    .line(Line::fixed("books", dec!(200)).with_vat(v7.clone()))
    .payment(Payment::Advance(Advance { vat: v19, net: dec!(300), tax: dec!(57) }))
    .payment(Payment::Advance(Advance { vat: v7, net: dec!(100), tax: dec!(7) }))
    .credit(Credit::after_tax("wallet", dec!(20)))
    .build()?;
assert_eq!(doc.prepaid(), Amount::parse("484")?); // 357 + 107 + 20
assert_eq!(doc.paid_entries().len(), 3);           // two advances, one gross payment

// Split between two co-owners: every entry halves, each half states its residual.
for half in doc.split(&[dec!(1), dec!(1)])? {
    let r = half.residual_vat()?;
    assert_eq!((r[1].taxable_base(), r[1].tax()), (Amount::parse("350")?, Amount::parse("66.50")?));
    assert_eq!((r[0].taxable_base(), r[0].tax()), (Amount::parse("50")?, Amount::parse("3.50")?));
}
// Reversed: the residual negates.
let r = doc.negated().residual_vat()?;
assert_eq!(r[1].tax(), Amount::parse("-133")?);
```
