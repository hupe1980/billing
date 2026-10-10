+++
title = "VAT"
description = "VAT as an attribute of every line and adjustment: the ten UNCL 5305 categories, exemption reasons, and the BG-23 breakdown derived by grouping with the tax rounded once per group."
weight = 40
+++

## VAT is an attribute, the breakdown is a group-by

Every line (BT-151/152) and document allowance or charge (BT-95/96, BT-102/103)
carries a `Vat`, or takes the builder's `default_vat`. The breakdown (BG-23,
`vat_groups()`) is a **group-by** per (category, rate): BT-116 is the sum of the
members, BT-117 is `round(base × rate / 100)` **once per group**, never per line.

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, TaxCategory, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("energy", dec!(400)))
    .line(Line::fixed("water", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
    .build()?;

let groups = doc.vat_groups(); // ordered by category code, then rate
assert_eq!(groups.len(), 2);
assert_eq!((groups[0].category(), groups[0].rate_percent()), (TaxCategory::Standard, Percent::new(dec!(7))));
assert_eq!((groups[0].taxable_base(), groups[0].tax()), (Amount::parse("100")?, Amount::parse("7")?));
assert_eq!(groups[1].tax(), Amount::parse("76")?);
assert_eq!(doc.vat_total(), Amount::parse("83.00")?); // BT-110 = Σ BT-117 (BR-CO-14)
```

Two lines of 0.03 at 19 %: rounding per line gives 0.01 + 0.01, but the group's exact
tax 0.0114 rounds to 0.01 — which is what a validator recomputes:

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("a", dec!(0.03)).with_vat(Vat::standard(Percent::new(dec!(19.00)))?)) // same rate, other spelling
    .line(Line::fixed("b", dec!(0.03)))
    .build()?;
assert_eq!(doc.vat_groups().len(), 1); // the rate is normalised: 19.00 == 19
assert_eq!(doc.vat_total(), Amount::parse("0.01")?);
```

`BR-CO-17` and the per-category base rules (`BR-S-08` and siblings) therefore hold by
construction.

## The rate is a `Percent`

`Vat::standard(Percent::new(dec!(19)))` is 19 %, as BT-152 states it; a bare `Decimal`
does not compile, so `0.19` can never be mistaken for 19 %, and Corsica's 0.9 % is
`Percent::new(dec!(0.9))`. Rates are normalised (`19.00` → `19`).

## Constructors, and the ten categories

Every `Vat` comes from a validating constructor, so it satisfies its category's rate
and reason rules:

| Constructor | Categories | Rate | Exemption reason |
|---|---|---|---|
| `Vat::standard(rate)` | `S` | > 0 (`BR-S-05`) | forbidden (`BR-S-10`) |
| `Vat::zero_rated()` | `Z` | 0 (`BR-Z-05`) | forbidden (`BR-Z-10`) |
| `Vat::exempt(category, why)` | `E`, `AE`, `K`, `G`, `O` | 0 | **required** (`BR-E-10`, `BR-AE-10`, `BR-IC-10`, `BR-G-10`, `BR-O-10`) |
| `Vat::regional(category, rate)` | `L`, `M`, `B` | ≥ 0 (`BR-AF-05`, `BR-AG-05`) | forbidden for `L`, `M` (`BR-AF-10`, `BR-AG-10`) |

```rust
use billing::{ErrorKind, Exemption, Percent, TaxCategory, Vat, VatError};
use rust_decimal::dec;

let err = Vat::standard(Percent::new(dec!(0))).unwrap_err();
assert!(matches!(err.kind(), ErrorKind::InvalidVat(VatError::Rate { .. })));
assert_eq!(err.rule(), Some("BR-S-05"));

let ae = Vat::exempt(TaxCategory::ReverseCharge, Exemption::Code("VATEX-EU-AE".into()))?;
assert_eq!((ae.category().code(), ae.rate_percent()), ("AE", Percent::ZERO));

let blank = Vat::exempt(TaxCategory::Exempt, Exemption::Text("  ".into())).unwrap_err();
assert_eq!(blank.rule(), Some("BR-E-10"));

// A 0 % IGIC line is lawful; a 0 % standard-rated one is not.
assert!(Vat::regional(TaxCategory::CanaryIslands, Percent::new(dec!(0))).is_ok());
```

Common pitfalls:

- **`Z` must not have an exemption reason; `E` must.** Both carry zero tax.
- **`B` is not `AE`.** Under Italian split payment the tax **is** stated; nothing
  constrains `B`'s rate or reason, but `BR-B-02` forbids it beside `S`.
- **`S` requires a rate above zero** (`BR-S-05`); `L` and `M` permit zero.

`TaxCategory` is `#[non_exhaustive]`; `TaxCategory::from_code` parses a code
case-insensitively.

### `O` and the rate that must be absent {#out-of-scope}

For `O`, a line, allowance or charge must **not state its rate** (`BR-O-05`,
`BR-O-06`, `BR-O-07`): `Vat::stated_rate()` is `None` (`rate_percent()` is 0 %). The
breakdown rate BT-119 is always stated (`BR-DE-14`).

## Exemption reasons: text, code or both

The reason rules require a code (BT-121) **or** a text (BT-120); `Exemption` is
`Text`, `Code` or `Both`, and a blank reason counts as none. VATEX code-list
membership (`BR-CL-22`) is the format layer's check.

## Document-wide rules

`build()` enforces three document-wide rules:

- **All-VAT or VAT-free.** Either every line and adjustment has a VAT treatment (its
  own or the default), or none has. A line without one in a VAT document is
  `ErrorKind::MissingVat` at `Location::Line(i)`, citing `BR-CO-04`.
- **One reason per group.** Two members of one (category, rate) group with different
  exemption reasons are `ErrorKind::ConflictingExemption`.
- **`O` stands alone, `B` never beside `S`.** `O` beside any other group breaks
  `BR-O-11`; `B` beside `S` breaks `BR-B-02`. Both are `ErrorKind::InvalidVat`.

```rust
use billing::{AmountScale, Currency, Document, ErrorKind, Exemption, Line, Location, Percent, TaxCategory, Vat};
use rust_decimal::dec;

let out_of_scope = Vat::exempt(TaxCategory::OutOfScope, Exemption::Text("not subject to VAT".into()))?;
let err = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("damages", dec!(500)).with_vat(out_of_scope))
    .line(Line::fixed("service", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(19)))?))
    .build()
    .unwrap_err();
assert_eq!(err.rule(), Some("BR-O-11"));

let err = Document::builder(Currency::EUR, AmountScale::EN16931)
    .line(Line::fixed("a", dec!(1)).with_vat(Vat::standard(Percent::new(dec!(19)))?))
    .line(Line::fixed("b", dec!(1)))
    .build()
    .unwrap_err();
assert_eq!(err.kind(), &ErrorKind::MissingVat);
assert_eq!(err.location(), Some(&Location::Line(1)));
assert_eq!(err.rule(), Some("BR-CO-04"));
```

A **VAT-free** document (no `Vat` anywhere) builds with no groups — fine for quotes,
but not an EN 16931 invoice (`BR-CO-18`).

A document discount over several rates must be split per group; see
[`apportion`](../tax-and-discounts/#apportion).
