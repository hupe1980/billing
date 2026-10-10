+++
title = "EN 16931 and scope"
description = "Why rounding at the serialiser cannot work, how a Document rounds the leaves once, why BT-146 must never be rounded, and what belongs to en16931 instead."
weight = 120
+++

## Two requirements that fight

EN 16931 — and with it XRechnung, Peppol BIS and ZUGFeRD / Factur-X — caps **every**
monetary amount at two decimals: each line net (BT-131, `BR-DEC-23`), the line sum
(BT-106, `BR-DEC-09`), the total without VAT (BT-109, `BR-DEC-12`), the VAT total
(BT-110, `BR-DEC-13`), the total with VAT (BT-112, `BR-DEC-14`), the VAT total in the
accounting currency (BT-111, `BR-DEC-15`), the paid amount (BT-113, `BR-DEC-16`), each
VAT group's base and tax (BT-116 / BT-117, `BR-DEC-19` / `BR-DEC-20`).

At the same time the totals identities must hold **exactly** at that precision:
`BR-CO-10`, `BR-CO-13`, `BR-CO-14`, `BR-CO-15` and `BR-CO-16`.

Rounding each amount independently at the serialiser breaks the identities:

- three lines of `0.005` round to `0.01` each, summing to `0.03`, while the exact sum
  `0.015` rounds to `0.02` — **`BR-CO-10` violated**;
- a net of `0.0042` with 19 % VAT gives `0.00 + 0.00 ≠ 0.01` — **`BR-CO-15`
  violated**.

## Round the leaves, derive the aggregates

The only construction that satisfies both is to round the **leaves** once and derive
every aggregate from the rounded leaves — never to round an aggregate. A `Document`
does exactly that at build, and holds nothing else:

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let three = Document::builder(Currency::EUR, AmountScale::EN16931)
    .lines((0..3).map(|i| Line::fixed(format!("item {i}"), dec!(0.005))))
    .build()?;
assert_eq!(three.line_net(0), Amount::parse("0.01")?);
assert_eq!(three.line_total(), Amount::parse("0.03")?); // Σ BT-131: BR-CO-10 holds

let tiny = Document::builder(Currency::EUR, AmountScale::EN16931)
    .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    .line(Line::fixed("x", dec!(0.0042)))
    .build()?;
assert_eq!(tiny.tax_exclusive_total(), Amount::ZERO);
assert_eq!(tiny.vat_total(), Amount::ZERO);
assert_eq!(tiny.tax_inclusive_total(), Amount::ZERO);   // BR-CO-15 holds
```

The leaves — each line's price part, each allowance and charge, each VAT group's
tax — are reduced **once, from their exact value** (`0.004999` → `0.00`, not via
`0.005` → `0.01`). Every [total](../totals/) is a sum of reduced leaves, so a
`Document` at `AmountScale::EN16931` is two-decimal by construction.

## BT-146 is not capped, and rounding it is fatal

**No rule caps a unit price** (BT-146), so the crate keeps prices as exact
`Decimal`s. `PEPPOL-EN16931-R120` multiplies the price by the quantity and compares it
to BT-131 within ±0.02, so rounding an average price "to be tidy" scales the error by
the whole period's consumption:

```rust
use billing::{AmountScale, Currency, Decimal, Document, Line, Price, Quantity, UnitPrice, rate_series};
use rust_decimal::dec;

// A year of hourly spot prices between 0.10000 and 0.10006 EUR/kWh, 1 kWh each.
let series = rate_series((0..8760i64).map(|h| (dec!(1), Price::new(Decimal::new(10_000 + h % 7, 5)))))?;
assert_eq!(series.net, dec!(876.26274));

let build = |line: Line| Document::builder(Currency::EUR, AmountScale::EN16931).line(line).build();
assert!(build(series.into_line("spot", "KWH"))?.price_inconsistent().is_empty());

// The same line with the average rounded to 0.10: off by 0.26, thirteen times R120's tolerance.
let tidy = Price::new(series.average_price.value().round_dp(2));
let tidied = Line::new("spot", Quantity::new(series.quantity, "KWH"), UnitPrice::new(tidy)).with_net(series.net);
assert_eq!(build(tidied)?.price_inconsistent(), [0]);
```

Render the price the engine computed; never re-derive it from a rounded total.

## Where this crate stops {#the-layer-above}

`billing` answers *what are the amounts?* Everything else an EN 16931 invoice needs is
identity, syntax or code-list data, and lives one layer up, in
[**`en16931`**](https://crates.io/crates/en16931) (the semantic model and its business
rules as Rust types) and [**`en16931-formats`**](https://crates.io/crates/en16931-formats)
(UBL 2.1 and UN/CEFACT CII in both directions, the XRechnung CIUS, ZUGFeRD /
Factur-X).

| Crate | Answers |
|---|---|
| **`billing`** | *What are the amounts?* |
| **`en16931`** | *Is this a lawful invoice?* |
| **`en16931-formats`** | *How does it go on the wire?* |

These are **owned by `en16931`**, not this crate:

| Concern | Business terms and rules |
|---|---|
| invoice number, issue and due dates, buyer reference | BT-1, BT-2, BT-9, BT-10 |
| document type codes — commercial invoice, credit note, corrected invoice, prepayment invoice | BT-3 (`BR-CL-01`, `PEPPOL-EN16931-P0100`, `PEPPOL-EN16931-P0112`, `BR-DE-17`) |
| invoicing periods and line periods, and their ordering | BG-14, BG-26 (`BR-29`, `BR-30`, `BR-CO-20`, `PEPPOL-EN16931-R110`) |
| the payment terms text, including the Skonto micro-syntax | BT-20 (`BR-DE-18`) |
| notes | BG-1 (`BR-CL-08`, `PEPPOL-EN16931-R002`) |
| preceding invoice references | BG-3 (`BR-55`) |
| parties and their identifiers | BG-4, BG-7 (`BR-CL-10`, `BR-CO-26`) |
| code-list membership: currency, units, reason codes, VATEX | `BR-CL-04`, `BR-CL-05`, `BR-CL-22`, … |

What stays here is the arithmetic: lines, allowances and charges, VAT attributes and
the breakdown, the totals chain, paid amounts, cash rounding, the VAT accounting total,
and the [Skonto and interest arithmetic](../payment-terms/) behind BT-20. `billing`
does not depend on either crate.

## EN 16931-1:2026

The revised **EN 16931-1:2026** is not backward compatible, but its additions are
mostly syntax and identity data for `en16931` / `en16931-formats`. Its arithmetic —
[early payment discount](../payment-terms/), [gross split](../gross-pricing/) — is
already here; `TaxCategory` is `#[non_exhaustive]`.
