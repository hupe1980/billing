+++
title = "E-invoicing and precision"
description = "Why rounding at the serialiser cannot work, what AmountScale does instead, what this crate deliberately does not do, and what EN 16931-1:2026 changes."
weight = 120
+++

## Two requirements that fight

EN 16931 — and with it XRechnung, Peppol BIS and ZUGFeRD / Factur-X — caps **every**
monetary amount at two decimals. Not some of them: each line net amount (BT-131,
`BR-DEC-23`), the line sum (BT-106, `BR-DEC-09`), the total without VAT (BT-109,
`BR-DEC-12`), the VAT total (BT-110, `BR-DEC-13`), the total with VAT (BT-112,
`BR-DEC-14`), the paid and rounding amounts (BT-113 / BT-114), the amount due (BT-115),
each VAT category's base and tax (BT-116 / BT-117, `BR-DEC-19` / `BR-DEC-20`), and the
VAT total in the accounting currency (BT-111, `BR-DEC-15`).

At the same time the totals identities must hold **exactly** at that precision:
`BR-CO-10`, `BR-CO-13`, `BR-CO-14`, `BR-CO-15` and `BR-CO-16`.

Those two demands together are why rounding at the serialiser does not work. Rounding
each amount independently breaks the identities:

- three lines of `0.005` round to `0.01` each, summing to `0.03`, while the exact sum
  `0.015` rounds to `0.02` — **`BR-CO-10` violated**;
- a net of `0.0042` with 19 % VAT gives `0.00 + 0.00 ≠ 0.01` — **`BR-CO-15` violated**.

Both come from ordinary inputs. `1234.567 kWh × 0.28901 EUR/kWh = 356.80221` is not a
contrived number; it is Tuesday.

## Round the leaves, recompute the aggregates

The only construction that satisfies both is to round the **leaves** and then recompute
every aggregate from the rounded leaves — never to round an aggregate. That has to
happen while the document is assembled, because by the time you hold a
`BillingDocument` the aggregates already exist.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1234.567), "kWh"),
        UnitPrice::new(dec!(0.28901), "EUR/kWh"),
    ).build()?])
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert_eq!(doc.net_total(),   Amount::parse("356.80000")?);
assert_eq!(doc.tax_total(),   Amount::parse("67.79000")?);
assert_eq!(doc.gross_total(), Amount::parse("424.59000")?);
assert!(doc.fits_amount_scale(2));
doc.assert_valid();
```

Every position, every discount- and tax-layer output, every VAT breakdown entry, every
line allowance and every stated basis is reduced *before* any total is computed. Each
total is then a sum of already-reduced values and lands on the same precision exactly.

### Let the model say it, not every call site

The precision is a property of the **format the document will be emitted in**, and a
`PricingModel` is the piece that knows it. Declare it once and `bill`, `try_bill`,
`settle`, `try_settle` and `BillingDocumentBuilder::pricing` all honour it:

```rust
use billing::prelude::*;
use rust_decimal::dec;
use std::convert::Infallible;

struct MeteredPlan;

impl PricingModel for MeteredPlan {
    type Usage = ();
    type Error = Infallible;
    type NotBillable = Infallible;

    fn positions(&self, _: &()) -> Result<Positions<Infallible>, Infallible> {
        Ok(vec![LineItem::for_usage(
            "Arbeit",
            Quantity::new(dec!(1234.567), "kWh"),
            UnitPrice::new(dec!(0.28901), "EUR/kWh"),
        ).build().unwrap()].into())
    }

    // This model prices for EN 16931, and says so.
    fn amount_scale(&self) -> Option<AmountScale> { Some(AmountScale::EN16931) }
}

let doc = MeteredPlan.settle(DocumentMeta {
    currency: Currency::EUR, ..Default::default()
})?;

assert_eq!(doc.net_total(), Amount::parse("356.80000")?);
assert!(doc.fits_amount_scale(2));   // emittable, straight out of `settle`
```

Because rounding cannot be applied afterwards, a model with no scale declared could
only ever produce five-decimal documents — so the shortest path from a model to a
document was also the one that produced an invoice failing `BR-DEC-23`, with no error
anywhere. An explicit `amount_scale` on the builder still wins over the model's.

### Rounded once, from the exact value

Where a leaf is *derived*, the reduction rounds the exact value once rather than
reducing an already-rounded figure. Two roundings are not one: `0.004999` rounded to
five decimals half-away-from-zero is `0.005`, which then rounds to `0.01`, while
rounding `0.004999` straight to two decimals gives `0.00`.

That is not a stylistic preference. `BR-CO-17` defines `BT-117 = BT-116 × rate` as a
single rounding of the exact product, and a validator recomputes it that way. So does
Peppol's `R120` for a line:

> `[PEPPOL-EN16931-R120]` BT-131 = BT-129 × (BT-146 ÷ BT-149) + Σ BG-28 − Σ BG-27
> — **fatal**, ±0.02.

The crate derives that expression in exactly **one** function, used both by
`LineItem::build` at the engine's precision and by the scale reduction at the
interchange precision. They drifted apart once: when BT-149 and the line allowance
groups were added to one and not the other, every line using them silently fell back to
rounding twice. Routing both through one derivation makes that failure structural
rather than incidental, and a property test asserts the two agree for random lines.

### Checking before you emit

```rust
use billing::prelude::*;
use rust_decimal::dec;

let raw = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1234.567), "kWh"),
        UnitPrice::new(dec!(0.28901), "EUR/kWh"),
    ).build()?])
    .build()?;

assert!(!raw.fits_amount_scale(2));
let (what, value) = raw.amount_scale_violation(2).unwrap();
// names the field a consumer would actually emit, e.g. position[0] "Arbeit"
assert_eq!(value, Amount::parse("356.80221")?);
```

Do **not** respond to a `false` by rounding on the way out — that is the mistake this
whole page is about. Rebuild with `amount_scale`.

### BT-146 is not capped, and rounding it is fatal

The `BR-DEC` decimal limits stop at BT-131 (`BR-DEC-23`) and the line allowance and
charge leaves (`BR-DEC-24` … `BR-DEC-28`). **No rule caps a unit price** — not BT-146,
BT-147, BT-148 or BT-149 — so a twenty-eight-decimal price is lawful, and
`amount_scale` deliberately leaves prices alone while reducing every amount around
them.

That asymmetry is load-bearing, because `R120` multiplies the price by the
**quantity** before comparing it to BT-131. A `DynamicPricing` line states the
*weighted average* over its intervals, which is a long non-terminating decimal by
construction — so rounding it "to be tidy" scales the error by the whole period's
consumption. On a settled spot-price year of 8 760 hourly intervals, rounding BT-146
to two decimals moves the expression by `0.26`: **thirteen times the fatal ±0.02**,
on a document whose totals all still reconcile.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let mut b = DynamicPricing::builder().unit("kWh").currency(Currency::EUR);
for i in 0..8_760 {
    b = b.interval(dec!(1.0), Amount::from_raw_units(1_000 + (i % 7)));
}
let line = b.build()?.calculate()?;
line.check_price_consistency()?;                       // holds as built

let mut tidied = line;
tidied.unit_price = Some(tidied.unit_price.take().unwrap()
    .rounded(2, RoundingStrategy::MidpointAwayFromZero));
assert!(tidied.validate().is_ok());                    // arithmetic is fine …
assert!(tidied.check_price_consistency().is_err());    // … the profile is not
```

`LineItem::check_price_consistency()` and
`BillingDocument::price_inconsistent_positions()` are the third member of the *"may I
emit this?"* family, beside `verify_vat_attribution()` and
`unattributed_positions()`. They sit outside `validate()` for the same reason those
do: `R120`'s tolerance is **absolute**, so a document assembled at
`AmountScale::new(0, …)` — legitimate for a zero-decimal currency, or for something
that is not an invoice at all — can be perfectly self-consistent and still fall
outside ±0.02.

The corollary for `DynamicPricing`: **render the average it computed, do not
re-derive it.** `net_amount ÷ quantity` divides an already-rounded total and gives a
different, worse number; the line's `unit_price` comes from the exact accumulated sum.

## What is still missing for a valid XRechnung

Precision is the deepest gap but not the only one. To emit a document that passes the
KoSIT validator you also need the following, in your own mapping layer or by handing
off to [`en16931`](#the-layer-above):

| Gap | What the standard requires | Status here |
|---|---|---|
| **Parties** | seller and buyer name, postal address with country code, VAT identifier (BT-31), electronic address (BT-34 / BT-49) | not modelled. `DocumentMeta` does carry BT-29 / BT-46 as `PartyIdentifier`, with the ISO 6523 scheme `BR-CL-10` requires |
| **Buyer reference** | BT-10 — the Leitweg-ID, mandatory for German B2G | put it in `meta.labels` |
| **Dates** | BT-2 issue date, BT-9 due date as real dates | `Option<String>`, unparsed by design (no date dependency). Period *ordering* is checked: `BR-29`, `BR-30` and `PEPPOL-EN16931-R110` / `R111` are enforced wherever the endpoints are zero-padded ISO 8601, and silently skipped otherwise |
| **Payment means** | BG-16 — BT-81 payment means code, BT-84 IBAN | not modelled; these are settlement instructions, not amounts. BT-20 payment terms **are** modelled, including the `BR-DE-18` Skonto syntax |
| **Line identifiers** | BT-126 per line | positions are ordered, not identified |
| **Notes** | BG-1 — BT-21 subject code (UNCL 4451, `BR-CL-08`) and BT-22 text, `0..n` | modelled as `Vec<Note>`; `PEPPOL-EN16931-R002` caps them at one unless both parties are German, which the engine leaves to the layer that knows the countries |
| **Code-list membership** | BT-98 ∈ UNCL 5189, BT-105 ∈ UNCL 7161, BT-121 ∈ CEF VATEX, BT-130 / BT-150 ∈ UN/ECE Rec 20/21, BT-3 ∈ UNTDID 1001 | the fields exist and round-trip; the engine carries no copy of the lists and does not check membership |
| **Suppressing the rate under `O`** | `BR-O-05` / `06` / `07` — BT-152 / BT-96 / BT-103 must be *absent*, not zero | `rate` is a `Decimal` storing `0`; `TaxCategory::states_rate()` tells the serialiser to omit the element |
| **Currency** | BT-5 ∈ ISO 4217 (`BR-CL-04`), and a *real* currency for the invoice to mean anything | two independent questions, two predicates: `is_iso4217()` answers `BR-CL-04` (`XXX` **passes** it), `is_unset()` answers "was one configured". Ask both |

What **is** modelled: amounts, the whole totals chain, the VAT breakdown with both
exemption-reason forms, per-position VAT attribution, the whole of BG-29 price details,
line allowances and charges (BG-27 / BG-28), unit codes, allowance and charge reason
codes, advance payments, cash rounding, prepaid, amount due, the VAT accounting currency
pair, payment terms with structured Skonto, and document type codes.

## Why XML is not in this crate

**ZUGFeRD is out of scope permanently.** It is a PDF/A-3 container with embedded CII XML
and XMP metadata — that needs a PDF writer, font embedding and subsetting, and ICC
colour profiles. This crate has three dependencies, does no I/O and forbids `unsafe`; a
PDF engine is the opposite of all three.

**XML belongs in a separate crate.** UBL and CII serialisation needs an XML writer, the
UN/ECE and UNTDID code lists, and Schematron conformance fixtures to be trustworthy —
none of which a pricing engine should carry, and all of which move on a different
release cadence. Keeping it out means this crate does not inherit that cadence.

## The layer above {#the-layer-above}

That separate crate exists. [**`en16931`**](https://crates.io/crates/en16931) carries
the EN 16931 semantic model and its business rules as Rust types, validating the
*model* rather than a serialised document — so a finding names BT-151 on line 3 instead
of an XPath. Its companion [**`en16931-formats`**](https://crates.io/crates/en16931-formats)
does the serialisation: UBL 2.1 and UN/CEFACT CII in both directions, the XRechnung
CIUS, and ZUGFeRD / Factur-X hybrid PDFs.

| Crate | Answers |
|---|---|
| **`billing`** | *What are the amounts?* |
| **`en16931`** | *Is this a lawful invoice?* |
| **`en16931-formats`** | *How does it go on the wire?* |

They are separate dependencies, not a framework: `billing` has no knowledge of them.
Use it alone if you only need the arithmetic.

What `billing` contributes to that stack is the part that is hard to retrofit: amounts
that are already representable at the target scale. A validator can tell you
`BT-112 ≠ BT-109 + BT-110`; it cannot tell you which rounding decision three layers ago
made them disagree.

## EN 16931-1:2026

CEN approved a revised EN 16931 on 23 October 2025; it passed the formal vote on
13 February 2026 and is published as **EN 16931-1:2026**. It is deliberately **not**
backward compatible: new business terms, changed validation rules, and reworked UBL and
CII bindings.

The headline additions are structured **early payment discounts** and **late-payment
charges**, margin schemes, further VAT category codes and exemption reasons, IBAN
fields, triangulation indicators, multiple purchase orders and corrective-invoice
sequencing.

Most of that is syntax and identity data, which is why it lands in `en16931` /
`en16931-formats` rather than here. What touches arithmetic, this crate already
computes: the [early payment discount](../payment-terms/), and — for margin schemes and
every tax-inclusive quote — the [gross split](../gross-pricing/). Where those numbers
are *written* changes with the revision; what they are does not.
