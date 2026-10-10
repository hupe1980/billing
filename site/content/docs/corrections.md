+++
title = "Corrections and bounds"
description = "Document::negated for credits, Document::concat for merging with VAT rounded once over the union, and minimum and maximum bounds settled into an ordinary line."
weight = 80
+++

## `negated`: the exact negative document

`negated()` negates every input. Because every
[rounding mode is sign-symmetric](../amounts/#rounding), every derived amount negates
exactly:

```rust
use billing::{Amount, AmountScale, Currency, Document, DocumentAdjustment, Line, LineAdjustment, Percent, Payment, Price, Quantity, UnitPrice, Vat};
use rust_decimal::dec;

let invoice = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(
        Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30))))
            .with_adjustment(LineAdjustment::allowance(dec!(5))),
    )
    .adjustment(DocumentAdjustment::allowance("loyalty", dec!(10)))
    .payment(Payment::Gross { description: "prepayment".into(), amount: dec!(100) })
    .build()?;
assert_eq!(invoice.payable(), Amount::parse("239.15")?); // 285 + 54.15 − 100

let credit = invoice.negated();
assert_eq!(credit.payable(), Amount::parse("-239.15")?);
assert_eq!(credit.vat_total(), -invoice.vat_total());
assert_eq!(credit.lines()[0].quantity().value(), dec!(-1000)); // direction lives in the quantity
assert_eq!(credit.lines()[0].price().price().value(), dec!(0.30)); // the price stays ≥ 0 (BR-27)
assert_eq!(credit.negated(), invoice);                          // an involution
```

Emit `negated()` as a negative commercial invoice (type code 380). A credit note
(381) states the original, positive amounts — `negated()` under 381 would negate
twice. Type code and preceding-invoice reference (BG-3) belong to
[`en16931`](https://crates.io/crates/en16931).

Bound lines and apportioned adjustments become ordinary ones in the negated document,
so the involution holds only for documents without them.

## `concat`: one document from several

`Document::concat` merges the *inputs* and builds once, so VAT is rounded once per
group over the union — generally **not** the sum of the parts' VAT:

```rust
use billing::{Amount, AmountScale, Currency, Document, ErrorKind, Line, Percent, Vat};
use rust_decimal::dec;

let part = || -> Result<Document, billing::Error> {
    Document::builder(Currency::EUR, AmountScale::EN16931)
        .default_vat(Vat::standard(Percent::new(dec!(19)))?)
        .line(Line::fixed("api calls", dec!(0.03)))
        .build()
};
let (a, b) = (part()?, part()?);
assert_eq!(a.vat_total() + b.vat_total(), Amount::parse("0.02")?); // 0.0057 → 0.01, twice

let merged = Document::concat([a, b])?;
assert_eq!(merged.lines().len(), 2);
assert_eq!(merged.vat_total(), Amount::parse("0.01")?); // 0.06 × 19 % = 0.0114

let chf = Document::builder(Currency::CHF, AmountScale::EN16931).line(Line::fixed("x", dec!(1))).build()?;
let err = Document::concat([part()?, chf]).unwrap_err();
assert!(matches!(err.kind(), ErrorKind::CurrencyMismatch { .. }));
assert_eq!(Document::concat([]).unwrap_err().kind(), &ErrorKind::Empty);
```

The documents must share currency, scale, cash rounding and VAT accounting
(`SettingMismatch` otherwise). Paid entries are kept in order, not merged.

## Bounds

A minimum charge (*Mindestumsatz*, *Mindestbetrag*) or a cap (*Preisobergrenze*, an
OCPI `max_price`) is a `Bound` on the net (`Basis::Net`, BT-109) or gross
(`Basis::Gross`, BT-112) total, **settled into one ordinary line** at build:

- a **minimum** adds the *smallest* amount, in whole units of the scale, that makes the
  basis reach the bound;
- a **maximum** adds the *largest* (negative) amount that keeps it at or below.

```rust
use billing::{Amount, AmountScale, Basis, Bound, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

// "Mindestbetrag 5,18 € brutto" on 3.10 € net of usage.
let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("usage", dec!(3.10)))
    .bound(Bound::minimum(Basis::Gross, dec!(5.18), "minimum charge"))
    .build()?;
assert_eq!(doc.bound_lines(), 1);
assert_eq!(doc.line_net(1), Amount::parse("1.25")?);
assert_eq!(doc.tax_inclusive_total(), Amount::parse("5.18")?); // 4.35 + 0.83, not 5.17

// "Höchstens 100 € netto" on 120 € of usage.
let capped = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("usage", dec!(120)))
    .bound(Bound::maximum(Basis::Net, dec!(100), "price cap"))
    .build()?;
assert_eq!(capped.line_net(1), Amount::parse("-20")?);
assert_eq!(capped.tax_exclusive_total(), Amount::parse("100")?);
```

A gross bound is settled against the document's own VAT derivation, so it holds after
group rounding (grossing down by hand gives 5.17 above). Set the bound line's VAT with
`Bound::with_vat`. When a gross total is
[unreachable](../gross-pricing/#shelf-prices), the bound lands on the nearest
reachable total on the right side:

```rust
use billing::{Amount, AmountScale, Basis, Bound, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("basket", dec!(25.19))) // 29.98 gross
    .bound(Bound::maximum(Basis::Gross, dec!(29.97), "price cap"))
    .build()?;
assert_eq!(doc.tax_inclusive_total(), Amount::parse("29.96")?); // 29.97 is unreachable
```

### Bounds stay bounds

The document keeps the `Bound`; `into_builder()` drops the settled line and
re-settles on the next build:

```rust
use billing::{Amount, AmountScale, Basis, Bound, Currency, Document, Line};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("usage", dec!(30)))
    .bound(Bound::minimum(Basis::Net, dec!(50), "minimum"))
    .build()?;
assert_eq!(doc.line_net(1), Amount::parse("20")?);

let edited = doc.into_builder().line(Line::fixed("late reading", dec!(40))).build()?;
assert_eq!(edited.bound_lines(), 0); // 70 ≥ 50: no top-up any more
assert_eq!(edited.tax_exclusive_total(), Amount::parse("70")?);
```

At most one minimum and one maximum, on the same basis, each above zero and fitting
the scale, minimum ≤ maximum. A minimum on a credit (a negative basis) would turn it
into a debit and is refused. All of these are `ErrorKind::InvalidBound` at
`Location::Bound(i)`, the bound's 0-based position in builder call order; a bound
without VAT in a VAT document is `ErrorKind::MissingVat` at the same location.
