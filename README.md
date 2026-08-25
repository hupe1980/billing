# 🧾 billing

[![Crates.io](https://img.shields.io/crates/v/billing.svg)](https://crates.io/crates/billing)
[![Docs.rs](https://img.shields.io/docsrs/billing)](https://docs.rs/billing)
[![CI](https://github.com/hupe1980/billing/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/billing/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license)
[![MSRV](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)

**[Guide and documentation site](https://hupe1980.github.io/billing) · [API reference](https://docs.rs/billing) · [Changelog](CHANGELOG.md)**

> **A pure, domain-agnostic invoice and usage-billing engine.**  
> Zero I/O. No async. No domain assumptions. No `f64` in monetary arithmetic.

`billing` is a calculation *library*, not a platform. It handles the hard
maths — graduated pricing, compound taxes, proportional allocation, exact
rounding — and leaves every domain decision to your crate.

Need to *emit* the invoice as well as compute it? Pair it with
[`en16931`](https://crates.io/crates/en16931) for the semantic model and its
business rules, and [`en16931-formats`](https://crates.io/crates/en16931-formats)
for UBL, CII, XRechnung and ZUGFeRD / Factur-X — see
[The layer above](#the-layer-above-en16931). Neither is a dependency of this crate.

> **Every Rust example in this README is compiled and run as a doctest.**
> If it appears here, it works against the current release.

> **Every EN 16931 rule cited here is verified against the validation artefacts**,
> not against prose or recollection, and both are pinned: CEN
> [`eInvoicing-EN16931`](https://github.com/ConnectingEurope/eInvoicing-EN16931) at
> tag `validation-1.3.16` (`BR-*`, `BR-CO-*`, `BR-CL-*`, `BR-DEC-*`) and
> [`peppol-bis-invoice-3`](https://github.com/OpenPEPPOL/peppol-bis-invoice-3) at
> `master` (`PEPPOL-EN16931-*`, `P01xx`). The target edition is
> **EN 16931-1:2017+A1:2019** — the one every deployed CIUS implements. On
> [EN 16931-1:2026](#the-2026-revision), see below.

This README is the complete reference. The
[documentation site](https://hupe1980.github.io/billing) is the same material
organised as a guide, with search and cross-links — start there if you are reading
rather than looking something up.

---

## ✨ Features at a glance

| Primitive | What it does |
|-----------|-------------|
| [`Amount<P>`](#-amountp--fixed-point-arithmetic) | Fixed-point monetary arithmetic with **compile-time precision**. `Amount<5>` = 5 dp, `Amount<2>` = 2 dp. |
| [`Currency`](#-currency) | ISO 4217 code + minor units; used in labels and enforced when combining documents |
| [`Currency::is_iso4217`](#two-boundary-questions-and-they-are-independent) | Whether BT-5 / BT-6 is a code EN 16931 admits — **BR-CL-03 / 04 / 05**, all fatal, and `XXX` passes |
| [`RateSchedule`](#-rateschedule--four-pricing-modes) | Four modes: **graduated** / **volume** / **block** / **capacity** |
| [`TimeOfUsePricing`](#-time-of-use-and-dynamic-pricing) | N named bands (peak / off-peak / …); caller supplies pre-aggregated consumption |
| [`tags`](#-design-invariants) | The engine's reserved tag namespace, protected against caller collisions |
| [`DynamicPricing`](#-time-of-use-and-dynamic-pricing) | Per-interval price sequence (spot, real-time) |
| [`UsageAggregator<E>`](#-usage-aggregation) | 8 built-in types: SUM · COUNT · UNIQUE_COUNT · MAX · MEAN · PERCENTILE · LATEST · WEIGHTED_SUM — **fallible**, because raw events are the least trusted input |
| [`PercentileAggregator`](#burstable-95th-percentile-billing) | **95th-percentile / burstable billing** — nearest-rank, so every figure billed is a rate that actually occurred |
| [`PricingModel`](#-implementing-pricingmodel) | The extension point: usage in, priced positions and layers out. **Not** a [`RateSchedule`](#pricingmodel-is-not-rateschedule) |
| [`PricingModel::amount_scale`](#the-model-declares-the-precision-it-prices-for) | The model declares the interchange precision it prices for, so `bill` / `settle` produce an emittable document |
| [`Billing<T, R>`](#-three-outcomes-billable-not-billable-error) | Three outcomes: billable, **not billable (with your reason)**, error |
| [`TaxLayer`](#-tax-layers--compound-taxes) | Composable, **ordered** tax stack — each layer sees all prior layers in its base |
| [`PerUnitLevy`](#-tax-layers--compound-taxes) | Per-unit excise duty / environmental levy, matched by unit label |
| [`DiscountLayer`](#-discounts) | Percentage and fixed discounts |
| [`PercentageCharge`](#-percentage-charge) | % of invoice total with min/max guard (platform fee, commission) |
| [`AllocationRule`](#-allocation-across-n-recipients) | Exact proportional / equal split of a document — `Σ(parts) == total`, penny-corrected |
| [`proportional_split()`](#raw-quantity-split-proportional_split) | Penny-correct Hamilton split of a raw `Decimal` quantity (kWh, capacity, …) |
| [`BillingDocument`](#-billingdocument) | Self-validating document: sixteen invariants, checked at build time and re-checked on deserialisation |
| [`BillingDocument::taxable_total`](#the-totals-chain--bt-106--bt-107--bt-108--bt-109) | **BT-109** via BR-CO-13 — `net_total` is *not* BT-109 once a levy exists |
| [`BillingDocument::allowance_total`](#the-totals-chain--bt-106--bt-107--bt-108--bt-109) | **BT-107** via BR-CO-11 — the allowance sum as the *positive magnitude* every syntax emits |
| [`TaxBreakdownEntry`](#-vat-breakdown-en-16931-bg-23) | Per-rate VAT breakdown (EN 16931 BG-23) — **legally required** on any invoice |
| [`TaxCategory`](#-vat-breakdown-en-16931-bg-23) | UNCL 5305 VAT category — all ten BR-CL-17 permits (S / Z / E / AE / K / G / O / L / M / **B**) |
| [`LineVat`](#per-position-vat-attribution) | Per-position VAT category + rate (BT-151/152, BT-95/96, BT-102/103) |
| [`AllowanceCharge`](#per-position-vat-attribution) | Allowance/charge detail: reason code + base + percentage (BT-93/94/98, BT-100/101/105) |
| [`LineAllowanceCharge`](#-line-allowances-and-charges-en-16931-bg-27--bg-28) | **BG-27 / BG-28** line allowance / charge (BT-136 … BT-145) — moves BT-131, not the document totals |
| [`DocumentKind::is_credit_note`](#-credit-notes) | Which document element to emit — `BR-CL-01` polices two element-selected code lists |
| [`DocumentKind::is_peppol_billing_code`](#the-type-code-is-not-interchangeable) | Whether Peppol BIS Billing accepts BT-3 — its lists are narrower than `BR-CL-01`'s |
| [`DocumentKind::is_self_billed`](#the-type-code-is-not-interchangeable) | Whether the buyer issued the document — a different Peppol *profile*, not a variant invoice |
| [`BillingDocument::vat_total`](#value-added-tax-vs-document-level-charges) | **BT-110** alone — separates VAT from document level charges (BG-21) |
| [`BillingDocument::verify_vat_attribution`](#per-position-vat-attribution) | Checks the breakdown against the per-position attribution (**BR-S-08**) |
| [`FixedRateTax::exempt`](#categories-and-exemptions) | Zero-tax layer with a validated category + mandatory exemption reason |
| [`Amount::extract_tax`](#-tax-inclusive-gross-pricing) | The tax **contained in** a gross price — the VAT fraction `r/(1+r)`, not `× r` |
| [`TaxBreakdownEntry::from_gross`](#-tax-inclusive-gross-pricing) | A lawful BG-23 from a tax-inclusive total, losing nothing |
| [`PaymentTerms`](#-payment-terms-and-skonto-bt-20) | **BT-20** — free text plus structured early-payment discounts, rendered and parsed |
| [`EarlyPaymentDiscount`](#-payment-terms-and-skonto-bt-20) | Skonto: `#SKONTO#TAGE=10#PROZENT=2.00#` (BR-DE-18), and what the buyer actually pays |
| [`LatePaymentPenalty`](#late-payment-interest-the-other-half-of-bt-20) | Verzugszinsen: simple interest on the overdue amount plus the art. 6(1) recovery fee |
| [`BillingDocument::unattributed_positions`](#per-position-vat-attribution) | Positions missing BT-151 / BT-95 / BT-102 — the other half of "may I emit this?" |
| [`LineItem::check_price_consistency`](#bt-146-is-not-capped-and-rounding-it-is-fatal) | **`PEPPOL-EN16931-R120`** — BT-131 really is `BT-129 × (BT-146 ÷ BT-149)` ± 0.02 |
| [`Amount::exact_to`](#-amountp--fixed-point-arithmetic) | Narrow precision **without rounding**, or fail — for interchange boundaries |
| [`CashRounding`](#-cash-rounding-and-amount-due) | Rappenrundung / öresavrundning — tender-level rounding (BT-114) |
| [`VatAccountingCurrency`](#-vat-in-the-accounting-currency-bt-6--bt-111) | **BT-6 / BT-111** — the VAT total restated in the currency the tax is reported in |
| [`AmountScale`](#-e-invoicing-en-16931-xrechnung-zugferd) | Assemble every amount at an interchange format's decimal limit (EN 16931: 2) |
| [`BillingDocument::reverse`](#-credit-notes) | Credit note / Storno — negates an entire document |
| [`AdvancePayment`](#-advance-payments-and-final-invoices) | An advance already invoiced and paid, **with the tax it contains** |
| [`Prepayment`](#-advance-payments-and-final-invoices) | What has been paid so far — a flat total or itemised advances, never both |
| [`DocumentKind`](#-advance-payments-and-final-invoices) | UNTDID 1001 document type code (BT-3) |
| [`Amount::distribute`](#-splitting-money-exactly) | Split an amount N ways with no cent created or lost |
| [`Amount::allocate`](#-splitting-money-exactly) | Split by integer ratios, largest-remainder, exact |
| [`RateLookup`](https://docs.rs/billing/latest/billing/lookup/struct.RateLookup.html) | Capacity-based rate table — `at_most(kWp, rate)` + `fallback(rate)`; an uncovered or negative parameter is an error, never the cheapest band |
| [`DocumentMeta.labels`](https://docs.rs/billing/latest/billing/document/struct.DocumentMeta.html) | Key-value domain annotation bag (`malo_id`, `billing_year`, …) |
| [`Note`](#-billingdocument) | **BG-1 INVOICE NOTE** — `0..n` notes, each with its UNCL 4451 subject code (BT-21 / BT-22) |
| [`PartyIdentifier`](#-billingdocument) | **BT-29 / BT-46** — an identifier *and* its ISO 6523 ICD scheme, which `BR-CL-10` makes mandatory to state correctly |
| [`PrecedingInvoice`](#a-credit-note-says-what-it-credits) | **BG-3 / BT-25 / BT-26** — what a credit note credits, filled in by `reverse` |
| [`LineItem::scaled`](#-proration-and-period-merging) | Scale a position, keeping `quantity × unit_price == net_amount` consistent |
| `LineItem::credit_for_usage` | Symmetric credit counterpart of `for_usage` (feed-in, refunds) |
| [`LineItem::flat_fee`](#a-flat-charge-is-still-an-invoice-line) | A standing charge as a **complete** EN 16931 line — satisfies BR-22 / BR-23 / BR-26, which `fixed` cannot |
| [`UnitPrice::per`](#-price-details-en-16931-bg-29) | **BT-149 / BT-150** — "EUR 12,00 per 100 pieces", the divisor in `PEPPOL-EN16931-R120` |
| [`UnitPrice::discounted`](#-price-details-en-16931-bg-29) | **BT-147 / BT-148** — gross price less a price discount, deriving BT-146 (`R046`) |
| `UnitPrice::rounded` | Pin a derived unit price to a scale with an explicit strategy |
| [`minimum_charge()`](#minimum-charges-belong-before-the-tax-layers) | Minimum-spend shortfall stated **net**, as a complete EN 16931 invoice line |
| [`minimum_charge_gross()`](#a-mindestbetrag-is-quoted-brutto) | The same for a minimum quoted **gross** — with the rate it is agreed at *enforced*, not assumed |
| [`merge_period_documents()`](#-proration-and-period-merging) | Merge two half-period documents (tariff change mid-period) |
| [`prorate()`](#-proration-and-period-merging) | Scale a fixed charge to a partial period |

---

## 🚀 Quick start

```sh
cargo add billing

# `dec!` lives in rust_decimal itself, behind the `macros` feature.
cargo add --dev rust_decimal --features macros
```

```rust
use billing::prelude::*;
use rust_decimal::dec;

// Three-tier water tariff (m³)
let schedule = RateSchedule::graduated()
    .unit("m³")                  // ← domain unit
    .currency(Currency::EUR)     // ← no currency is ever assumed
    .band(RateBand::up_to(dec!(5),   Amount::parse("0.80000")?))
    .band(RateBand::between(dec!(5), dec!(20), Amount::parse("1.40000")?))
    .band(RateBand::over(dec!(20),   Amount::parse("2.60000")?))
    .build()?;

let items = schedule.split(dec!(28.5))?;
// → [5 m³ × 0.80 = 4.00, 15 m³ × 1.40 = 21.00, 8.5 m³ × 2.60 = 22.10]

let doc = BillingDocument::from_positions(
    DocumentMeta {
        invoice_number: "WATER-2026-07".into(),
        period_label:   "July 2026".into(),
        currency:       Currency::EUR,
        ..Default::default()
    },
    items,
    vec![FixedRateTax::new("VAT", dec!(0.10))?.boxed()],
    vec![],
)?;

assert_eq!(doc.net_total().to_string(),   "47.10000");
assert_eq!(doc.tax_total().to_string(),   "4.71000");
assert_eq!(doc.gross_total().to_string(), "51.81000");

doc.assert_valid();   // panics if any arithmetic invariant fails
# Ok::<(), Box<dyn std::error::Error>>(())
```

Or use the fluent builder with the [`PricingModel`](#-implementing-pricingmodel) trait:

```rust,ignore
let doc = BillingDocument::builder()
    .meta(meta)
    .pricing(&my_model, &usage)?  // loads positions + tax/discount layers
    .build()?;
```

---

## 💰 `Amount<P>` — fixed-point arithmetic

`Amount<P>` stores money as an `i64` scaled by `10^P`. There is **no `f64`** anywhere
in the arithmetic path. `P` must be ≤ 18 (`10^19` exceeds `i64::MAX`); a larger `P`
is a compile-time error.

```rust
use billing::{Amount, RoundingStrategy};
use rust_decimal::Decimal;
use rust_decimal::dec;

// Parse — rejects strings with more non-zero digits than P
let price: Amount<5> = Amount::parse("0.03456")?;              // ✓ exactly 5 dp
assert!(Amount::<5>::parse("0.034561").is_err());              // ✗ 6th digit is non-zero
assert!(Amount::<5>::parse("0.034560").is_ok());               // ✓ trailing zero is lossless
assert_eq!(Amount::<5>::parse("0,50000")?, Amount::parse("0.50000")?); // ✓ comma separator
assert_eq!(Amount::<5>::parse(".5")?,      Amount::parse("0.50000")?); // ✓ omitted integer part
assert!(Amount::<5>::parse(".").is_err());                     // ✗ a separator is not a number

// Infallible ops panic on overflow; `checked_*` variants return Err.
let a = Amount::<5>::from_int(100);
let b = Amount::<5>::parse("0.50000")?;
let c = a.checked_add(b)?;                                     // 100.50000
let _ = a.mul_qty(Decimal::from(3u32));                        // 300.00000 (panics on overflow)
let _ = a.checked_mul_qty(Decimal::from(3u32))?;               // Ok(300.00000)

// `checked_*` never panics — not even at the extremes of Decimal's range.
assert!(a.checked_mul_qty(Decimal::MAX).is_err());
assert!(Amount::<5>::checked_from_decimal(Decimal::MAX).is_err());

// Decimal → Amount is EXACT, and agrees with `parse`: what is refused as text is
// refused as a Decimal. Rounding is opt-in and must name a strategy.
assert!(Amount::<5>::checked_from_decimal(dec!(0.123456)).is_err());   // ✗ 6th digit
assert!(Amount::<5>::checked_from_decimal(dec!(0.1234500)).is_ok());   // ✓ trailing zeros
assert_eq!(
    Amount::<5>::from_decimal_rounded(dec!(0.123456), RoundingStrategy::MidpointAwayFromZero)?,
    Amount::<5>::parse("0.12346")?,
);

// += and -= (panicking, like + and -)
let mut total = Amount::<5>::ZERO;
total += a;   // 100.00000
total -= b;   //  99.50000
assert_eq!(total.to_string(), "99.50000");

// Bounds
assert_eq!(Amount::<5>::MAX.to_string(), "92233720368547.75807");
assert_eq!(Amount::<5>::MIN.to_string(), "-92233720368547.75808");

// Sign
assert_eq!(Amount::<5>::parse("-3.00000")?.signum(), -1i8);
assert_eq!(Amount::<5>::ZERO.signum(), 0i8);

// Convert to/from Decimal (lossless)
let d: Decimal = Decimal::from(a);
let _back = Amount::<5>::try_from(d)?;

// TryFrom<i64> treats the integer as WHOLE UNITS — it is not the inverse of to_raw().
let from_db = Amount::<5>::try_from(4999i64)?;
assert_eq!(from_db, Amount::<5>::parse("4999.00000")?);
// To rebuild from a stored to_raw() value, use from_raw_units:
assert_eq!(Amount::<5>::from_raw_units(price.to_raw()), price);

// Round to a different precision (explicit strategy required)
let _invoice: Amount<2> = c.round_to(RoundingStrategy::MidpointAwayFromZero);

// Ready-made aliases (exported by the crate — you do not need to declare them):
let _: Amount<5> = Amount::<5>::ZERO;  // the engine's working precision
let _: Amount<2> = Amount::<2>::ZERO;  // EN 16931's two decimals
# Ok::<(), Box<dyn std::error::Error>>(())
```

> **Why not `f64`?**  `f64` cannot represent `0.1` exactly:
> `0.1 + 0.2 == 0.30000000000000004`.
> `Amount<P>` uses exact base-10 arithmetic via [`rust_decimal`](https://crates.io/crates/rust_decimal).

> ⚠️ **`rust_decimal`'s operators panic on overflow** (`Multiplication overflowed`),
> they do not saturate. Every `checked_*` method in this crate uses `Decimal`'s
> checked forms internally, so a documented `Result` is always a `Result`.

---

## 💱 Currency

The engine never assumes a currency. [`Currency`] is an ISO 4217 alphabetic code
used for two things: building unit-price labels, and refusing to combine
documents that are not denominated in the same currency.

```rust
use billing::{Currency, RateSchedule, RateBand, Amount};
use rust_decimal::dec;

let usd = RateSchedule::graduated()
    .unit("GB")
    .currency(Currency::USD)
    .band(RateBand::over(dec!(0), Amount::parse("0.10000")?))
    .build()?;

let items = usd.split(dec!(100))?;
assert_eq!(items[0].unit_price.as_ref().unwrap().unit, "USD/GB");

// Codes are validated and normalised.
assert_eq!(Currency::new("chf")?, Currency::CHF);
assert!(Currency::new("EURO").is_err());

// The default is ISO 4217 XXX — "no currency involved" — NOT a real currency.
// A label reading "XXX/GB" is a loud reminder that nobody configured one,
// which is strictly better than silently printing the wrong symbol.
assert_eq!(Currency::default(), Currency::XXX);
assert!(Currency::default().is_unset());
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Minor units

`Currency` knows the ISO 4217 minor-unit exponent, which is **not** always 2:

```rust
use billing::{Amount, Currency};

assert_eq!(Currency::EUR.minor_units(), Some(2));
assert_eq!(Currency::JPY.minor_units(), Some(0));                  // yen has no sen
assert_eq!(Currency::new("KWD")?.minor_units(), Some(3));          // dinar has fils
assert_eq!(Currency::new("CLF")?.minor_units(), Some(4));
assert_eq!(Currency::XXX.minor_units(), None);                     // no minor unit at all

// The smallest representable step, as an Amount:
assert_eq!(Currency::EUR.minor_unit_increment::<5>(), Some(Amount::parse("0.01000")?));
assert_eq!(Currency::JPY.minor_unit_increment::<5>(), Some(Amount::parse("1.00000")?));
# Ok::<(), Box<dyn std::error::Error>>(())
```

`None` means "no fractional subdivision" (the precious metals, `XDR`, `XXX`) — a
different thing from zero decimals, so the distinction is kept in the type.

> **Minor units are not the smallest transactable amount.** CHF has two minor
> units but its smallest coin is 5 Rappen. That is a payment-law rule, not a
> currency property — see [cash rounding](#-cash-rounding-and-amount-due).

### Two boundary questions, and they are independent

`Currency::new` accepts any three ASCII letters and keeps doing so: this crate
bills things that are not EN 16931 invoices, and a private clearing unit is a
legitimate thing to denominate an internal document in. What it must not do is let
such a code reach an *invoice* unremarked — **BR-CL-03 / BR-CL-04 / BR-CL-05** are
fatal, and the rejection happens at an access point rather than at a boundary you
control.

So there are two questions, and neither answers the other:

```rust
use billing::Currency;

// Is it a code EN 16931 admits? (BR-CL-04, fatal)
assert!(Currency::EUR.is_iso4217());
assert!(!Currency::new("XBT")?.is_iso4217());   // well-formed, constructible, fatal

// Was a currency actually configured?
assert!(Currency::XXX.is_unset());
assert!(Currency::XXX.is_iso4217());            // …and XXX *is* in the list
# Ok::<(), Box<dyn std::error::Error>>(())
```

Ask both before you emit. This is the same split as
[`DocumentKind::is_peppol_billing_code`](#the-type-code-is-not-interchangeable):
model the value, expose the profile question as a predicate.

The list is transcribed from the `contains(' AED AFN … ZWG ')` in BR-CL-03 of the
pinned CEN artefact — 178 codes — so it answers the question a validator will
actually ask, rather than ISO's registry in the abstract.

---

## 🧾 VAT breakdown (EN 16931 BG-23)

**A single tax total is not a lawful invoice.** EU VAT Directive art. 226(8)–(10)
requires "the taxable amount per rate or exemption", the rate, and the tax amount;
§14 Abs. 4 Nr. 7–8 UStG says the same. Any invoice mixing rates must show them
separately.

`BillingDocument` builds that breakdown automatically from the tax layers:

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let positions = vec![
    LineItem::fixed("Elektronik", Amount::parse("100.00000")?).tag("standard").build()?,
    LineItem::fixed("Buch",       Amount::parse("50.00000")?).tag("reduced").build()?,
];
let taxes: Vec<Box<dyn TaxLayer>> = vec![
    FixedRateTax::new("MwSt 19%", dec!(0.19))?.with_tag("standard").boxed(),
    FixedRateTax::new("MwSt 7%",  dec!(0.07))?.with_tag("reduced").boxed(),
];

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    positions, taxes, vec![],
)?;

// One line per (category, rate) — BR-S-08 for the taxed categories, BR-Z-01 and
// its siblings for the zero-tax ones. (Not BR-CO-18: that one says an invoice
// shall have *at least one* BG-23, and is checked separately.)
let bd = doc.tax_breakdown();
assert_eq!(bd.len(), 2);
assert_eq!(bd[0].taxable_base, Amount::parse("100.00000")?);  // BT-116
assert_eq!(bd[0].tax_amount,   Amount::parse("19.00000")?);   // BT-117
assert_eq!(bd[0].category,     TaxCategory::Standard);        // BT-118
assert_eq!(bd[1].rate_percent(), dec!(7));                    // BT-119
# Ok::<(), Box<dyn std::error::Error>>(())
```

Entries sharing a `(category, rate)` merge into one line, with the rate
**normalised** so `0.19` and `0.190` never split into two.

### Categories and exemptions

Three constructors cover the whole matrix, and each refuses the cases that belong
to another:

| Constructor | Categories | Exemption reason |
|-------------|------------|------------------|
| `FixedRateTax::new(name, rate)` | `S`, `L`, `M`, `B` — these levy tax | forbidden for `S`/`L`/`M`; neither required nor forbidden for `B` |
| `FixedRateTax::exempt(name, category, reason)` | `E`, `AE`, `K`, `G`, `O` | **required** — BT-120 free text |
| `FixedRateTax::exempt_coded(name, category, code)` | `E`, `AE`, `K`, `G`, `O` | **required** — BT-121 VATEX code |
| `FixedRateTax::zero_rated(name)` | `Z` | forbidden (infallible) |

BT-120 and BT-121 are **alternatives**, not a pair: BR-E-10, BR-AE-10, BR-IC-10,
BR-G-10 and BR-O-10 each ask for "a VAT exemption reason code (BT-121) **or** a
VAT exemption reason text (BT-120)". A caller holding only the machine-readable
code — the one BR-CL-22 can actually check — should not have to invent matching
prose. Symmetrically, BR-S-10, BR-Z-10, BR-AF-10 and BR-AG-10 forbid **both**, so
a VATEX code on a standard-rated group is refused just as the text is.

`B` — Italian split payment (*scissione dei pagamenti*) — is the odd one out, and
deliberately so. The CEN artefacts contain no `BR-B-05`, no `BR-B-09` and no
`BR-B-10`, so unlike the other "someone else pays" category (`AE`) it is **taxed
at the normal rate** and the tax is stated; only the settlement route differs. It
is also the one category where both `requires_exemption_reason()` and
`forbids_exemption_reason()` are `false`. Its own rules — BR-B-01 (domestic
Italian invoices only) and BR-B-02 (never mixed with `S`) — are jurisdictional and
stay with the caller; the engine refuses only what cannot add up.

Note also that a **standard-rated 0 % layer is refused**: BR-S-05 requires a
standard-rated line's rate to exceed zero, which makes an `(S, 0 %)` breakdown
group unsatisfiable under BR-S-08. Use `zero_rated` for a supply taxed at zero.
`L` and `M` are unaffected — BR-AF-05 and BR-AG-05 explicitly permit a zero rate.

```rust
use billing::{FixedRateTax, TaxCategory, TaxLayer};
use rust_decimal::dec;

// §13b UStG reverse charge: 0%, and a reason is mandatory (BR-AE-10).
let _rc = FixedRateTax::exempt(
    "Reverse charge",
    TaxCategory::ReverseCharge,
    "Steuerschuldnerschaft des Leistungsempfängers (§13b UStG)",
)?;

// Wrong-family arguments are refused at construction, not at breakdown time:
assert!(FixedRateTax::exempt("Z", TaxCategory::ZeroRated, "why").is_err());  // Z forbids one
assert!(FixedRateTax::exempt("S", TaxCategory::Standard,  "why").is_err());  // S levies tax

// The category rules are enforced, not merely documented:
assert!(FixedRateTax::new("Bad", dec!(0.19))?              // a zero-tax category
    .with_category(TaxCategory::ReverseCharge)             // cannot carry a rate
    .with_exemption_reason("x")
    .breakdown(&[]).is_err());

assert!(FixedRateTax::new("Exempt", dec!(0))?              // E *requires* a reason
    .with_category(TaxCategory::Exempt)
    .breakdown(&[]).is_err());

assert!(FixedRateTax::new("Standard", dec!(0.19))?         // S *forbids* one
    .with_exemption_reason("not allowed")
    .breakdown(&[]).is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

> **`Z` vs `E` is the distinction implementers get wrong.** Both carry zero tax,
> but zero-rating must *not* have an exemption reason and exemption *must* —
> input tax stays deductible under `Z` and generally does not under `E`.

**`O` is the other trap.** "Not subject to VAT" is the only category that must not
state a rate *at all* — BR-O-05, BR-O-06 and BR-O-07 say the element "shall not
contain" BT-152 / BT-96 / BT-103, where every other zero-tax category says the
rate "shall be 0 (zero)". Since `rate` is a plain `Decimal` here, an `O` position
stores `0`, and a serialiser must **suppress** the element rather than write
`<cbc:Percent>0</cbc:Percent>`. `TaxCategory::states_rate()` is that instruction
in code. `O` is also exclusive: BR-O-11 forbids any other breakdown group beside
it, and BR-O-12/13/14 forbid any line, allowance or charge in another category —
checked by `validate()` and `verify_vat_attribution()` respectively.

Only layers that actually levy VAT contribute here. A `PercentageCharge`
(commission) and a `PerUnitLevy` (excise) return `None` from `TaxLayer::breakdown`
— the excise is part of the VAT *base*, not a VAT. Implement `breakdown` on your
own layer if it levies VAT.

### The totals chain — BT-106 · BT-107 · BT-108 · BT-109

`net_total` is **not** BT-109, and the difference bites on exactly the documents
this crate is built for. EN 16931 builds the total without VAT in three steps
(**BR-CO-13**):

```text
BT-109 = BT-106 (Σ line net amounts) − BT-107 (allowances) + BT-108 (charges)
```

A document level **charge** — a per-unit levy, a commission — is produced by a
`TaxLayer` and therefore lives in `tax_positions`, even though EN 16931 counts it
inside the taxable base. So `net_total` covers only the first two terms:

| Accessor | EN 16931 | Rule |
|---|---|---|
| `line_total()` | **BT-106** | BR-CO-10 |
| `allowance_total()` | **BT-107** — the positive magnitude every syntax emits | BR-CO-11 |
| `discount_total()` | −BT-107 — the same figure in the engine's own sign | — |
| `charge_total()` | **BT-108** | BR-CO-12 |
| `net_total()` | BT-106 − BT-107 — *not a BT of its own* | — |
| `taxable_total()` | **BT-109** | BR-CO-13 |
| `vat_total()` | **BT-110** | BR-CO-14 |
| `gross_total()` | **BT-112** | BR-CO-15 |
| `amount_due()` | **BT-115** | BR-CO-16 |

> **Emit BT-107 with `allowance_total()`, not with `discount_total()`.**
> `cbc:AllowanceTotalAmount` is unsigned in UBL and in CII, because BR-CO-13
> *subtracts* it. This engine stores allowances as negative amounts so its totals
> stay plain sums, so the two differ by a sign — and an invoice stating `-20.00`
> where BR-CO-13 expects `20.00` fails by **twice** the allowance, with nothing in
> the document to show it. The accessor is that negation, done once, under the
> term's own name.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ).build()?])
    .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert_eq!(doc.line_total()?,      Amount::parse("300.00000")?);  // BT-106
assert_eq!(doc.allowance_total()?, Amount::ZERO);                 // BT-107 — none here
assert_eq!(doc.charge_total()?,    Amount::parse("20.50000")?);   // BT-108
assert_eq!(doc.taxable_total()?,   Amount::parse("320.50000")?);  // BT-109 — includes the levy
assert_eq!(doc.net_total(),        Amount::parse("300.00000")?);  // BT-106 − BT-107

// BR-CO-13, spelled the way the standard spells it.
assert_eq!(
    doc.taxable_total()?,
    doc.line_total()?.checked_sub(doc.allowance_total()?)?.checked_add(doc.charge_total()?)?,
);
// BR-CO-15 pairs BT-109 with BT-110 — not `net_total` with `tax_total`.
assert_eq!(doc.taxable_total()?.checked_add(doc.vat_total()?)?, doc.gross_total());
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Value added tax vs document level charges

`tax_total` is the sum of everything the tax layers produced, and EN 16931 puts
those in **two completely different places**:

| Position | EN 16931 | Contributes to |
|---|---|---|
| its layer returned a breakdown | value added tax (BG-23) | **BT-110**, the VAT total |
| its layer returned `None` | document level charge (BG-21) | **BT-108** → BT-109, the *taxable base* |

Mapping the whole of `tax_total` to BT-110 — the obvious thing to do — makes
**BR-CO-14** (`BT-110 = Σ BT-117`) fail on every document carrying a levy. The
engine marks the difference with the reserved `tags::VAT` tag, written from the
layer's own `breakdown` return value, so the classification is **total**: a
third-party `TaxLayer` is labelled as accurately as a built-in one.

```rust
use billing::prelude::*;
use rust_decimal::dec;

// Stromsteuer 2.05 ct/kWh (a charge), then 19 % MwSt on net + levy.
let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh").with_code("KWH"),   // BT-130
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ).build()?])
    .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert_eq!(doc.tax_total(),      Amount::parse("81.39500")?);  // both together
assert_eq!(doc.charge_total()?,  Amount::parse("20.50000")?);  // BT-108 — the levy
assert_eq!(doc.vat_total()?,     Amount::parse("60.89500")?);  // BT-110 — the VAT

// BR-CO-14 holds against the breakdown, which it would not for `tax_total`.
assert_eq!(doc.vat_total()?, doc.tax_breakdown()[0].tax_amount);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Per-position VAT attribution

EN 16931 requires a VAT category on **every** invoice line (BT-151, rule
BR-CO-04), allowance (BT-95, BR-32) and charge (BT-102, BR-37), and BR-S-08 then
checks the breakdown against them: for each `(category, rate)`, BT-116 must equal
the sum of the lines plus charges minus allowances carrying that pair.

The engine derives this during assembly from `TaxLayer::covers` — the same
predicate a layer uses to select its base — and stores it in `LineItem::vat`:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![
        LineItem::fixed("Beratung", Amount::parse("400.00000")?).tag("full").build()?,
        LineItem::fixed("Fachbuch", Amount::parse("100.00000")?).tag("reduced").build()?,
    ])
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("full").boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.07))?.with_tag("reduced").boxed())
    .build()?;

// Each line knows which group it belongs to — BT-151 / BT-152.
assert_eq!(doc.net_positions()[0].vat.unwrap().rate, dec!(0.19));
assert_eq!(doc.net_positions()[1].vat.unwrap().rate, dec!(0.07));

// And BR-S-08 holds: the breakdown adds up to exactly those lines.
doc.verify_vat_attribution()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

Set it yourself with `LineItemBuilder::vat` when the caller already knows it, or
when the document is assembled without VAT layers. A caller-declared value that
**contradicts** the layer actually taxing the position is reported as a
`LayerError` rather than silently overridden — and so is the case of two VAT
layers covering one position, which taxes it twice and makes BR-S-08
unsatisfiable for both groups.

`verify_vat_attribution` is deliberately *not* part of `validate()`, because it is
not decidable for every document the engine can hold: one assembled without VAT
layers has no attribution to check against and passes vacuously. Run it on the
document you are about to emit — including one that came out of an
`AllocationRule`, which preserves BR-S-08 whenever its source had it.

It also **passes vacuously** on a document with no attributions at all, which is
the right behaviour — this crate bills things that are not invoices — but it means
passing it is not on its own a licence to emit. `unattributed_positions()` answers
the other half: which positions still lack the BT-151 / BT-95 / BT-102 that
BR-CO-04, BR-32 and BR-37 make mandatory.

```rust
use billing::prelude::*;
use billing::{FixedRateTax, PerUnitLevy};
use rust_decimal::dec;

// The VAT layer is restricted to "energy", so nothing attributes a category to
// the levy — which is a BG-21 charge and needs one.
let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ).tag("energy").build()?])
    .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("energy").boxed())
    .build()?;

doc.verify_vat_attribution()?;                       // consistent …
assert_eq!(doc.unattributed_positions().count(), 1); // … but not yet emittable
# Ok::<(), Box<dyn std::error::Error>>(())
```

Fix it by declaring the charge's own treatment — `PerUnitLevy::with_vat`,
`PercentageCharge::with_vat`, `PercentageDiscount::with_vat`,
`FixedDiscount::with_vat` — or by widening the VAT layer's base so it covers the
charge.

Allowances and charges carry their own `LineItem::allowance_charge` detail:

| | Allowance (BG-20) | Charge (BG-21) |
|---|---|---|
| `reason_code` | BT-98 (UNCL 5189) | BT-105 (UNCL 7161) |
| `base_amount` | BT-93 | BT-100 |
| `percentage` | BT-94 | BT-101 |

The reason code is set via `with_reason_code` on all four built-in layers; the
position's description serves as the free-text BT-97 / BT-104, so BR-33 and BR-38
are satisfied either way and the code adds machine-readability.

**The base and the percentage are filled in automatically**, and they are not
free annotation: Peppol *recomputes* them.

> `[PEPPOL-EN16931-R040]` Allowance/charge amount must equal base amount *
> percentage/100 if base amount and percentage exists — **fatal**, ±0.02
>
> `[PEPPOL-EN16931-R041]` / `[R042]` — each is required when the other is present

So the pair is a claim, and any operation that changes the amount has to keep it
true. `LineItem::scaled` (allocation, proration) rescales the base with the
amount; `reverse()` negates both; and where the amount is adjusted on its own — a
min/max clamp, or allocation's penny correction — the basis is **dropped**,
keeping the reason code, because stating none is always valid.
`AllowanceCharge::check_amount` enforces R040 and `LineItem::validate` runs it, so
a transform that forgets fails loudly rather than emitting an invoice Peppol
rejects. `BR-DEC-02` / `BR-DEC-06` cap the base at two decimals in its own right,
so `AmountScale` reduces it and `fits_amount_scale` checks it.

The engine does not validate the code lists themselves — it has no copy of them,
and a stale embedded copy would be worse than none.

---

## 🏷️ Tax-inclusive (gross) pricing

Consumer prices are quoted **including tax** almost everywhere — §1 PAngV in
Germany, and the equivalent across the EU, the UK and Australia. EN 16931 is
uncompromisingly **net**: BT-131 per line, BT-116 per VAT category, BT-109 for the
document, with BT-117 derived from BT-116 under BR-CO-17. A gross catalogue has to
be converted, once, at the boundary.

The conversion is the one people get wrong. A gross price already contains the tax,
so the rate applies to the *net* part — the part that is unknown. Multiplying the
gross by the rate over-states the tax by exactly `rate²` of the net:

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
# Ok::<(), Box<dyn std::error::Error>>(())
```

`net_of_tax` is defined as the **remainder** — `gross − tax` — not as an
independent rounding of `gross / (1 + rate)`. So `net + tax == gross` holds
*exactly* for the amount it is applied to, under every rate and every strategy.
Rounding both halves separately fails that for roughly half of all inputs;
`tests/properties.rs` pins the law.

`TaxBreakdownEntry::from_gross` does the same split straight into a BG-23 group —
and takes an `AmountScale`, so the exact product is rounded **once**, at the
precision the document will be emitted at, rather than at five decimals and again
on the way out:

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
# Ok::<(), Box<dyn std::error::Error>>(())
```

### That does not make the document gross equal the sum of shelf prices

It is worth being blunt about this, because it looks like a bug and is not. Convert
three gross-quoted lines and the invoice will usually total a cent or two away from
adding the price tags up:

| | |
|---|---|
| 3 × 249,99 + 2 × 529,00 + 4 × 89,95 | **2 167,77** — the sum of shelf prices |
| BT-109 1 762,41 + BT-110 405,35 | **2 167,76** — what the invoice states |

Nothing was lost in the conversion. The difference is that VAT on an invoice is
charged **once per category**, not once per line: **BR-CO-17** defines
`BT-117 = BT-116 × rate` as a single rounding of the *group* total, and a validator
recomputes it exactly that way. The per-line taxes sum to 405,36; the tax on the
summed base is 405,35. The shelf-price total is the one built from independently
rounded numbers — and it is the one with no standing in EN 16931.

So: convert at the boundary, then let the totals chain run on net figures. Do not
try to force the document gross back onto the catalogue sum by adjusting a line —
that breaks BR-CO-17, which is checkable, to preserve an identity that is not.
`examples/cross_border_retail.rs` works the whole case end to end.

### Which of the two to use

They are different constructions, not interchangeable ones:

| You have | Use | Because |
|---|---|---|
| gross-quoted **lines** | `net_of_tax` per line, then an ordinary `FixedRateTax` layer | **BR-S-08** fixes BT-116 as the sum of the line net amounts; the breakdown must follow the lines, not the other way round |
| a gross **total** and no lines — a simple receipt, a settlement figure | `TaxBreakdownEntry::from_gross` | there is no BT-131 to sum, so the group total is the only thing to derive from |

Do not mix them. Stating a `from_gross` breakdown next to net lines that sum to
something else is exactly the BR-S-08 violation `verify_vat_attribution` reports.

---

## 🌍 VAT in the accounting currency (BT-6 / BT-111)

The invoice currency is a commercial choice; the currency VAT is *reported* in is
not. Art. 230 of the VAT Directive lets an invoice be denominated in any currency
but requires the VAT amount to be expressed in the national currency of the member
state where the tax is due. A Polish, Czech, Swedish, Danish, Hungarian or Romanian
supplier invoicing in EUR must therefore state the VAT **again** in PLN, CZK, SEK,
DKK, HUF or RON.

EN 16931 models exactly that pair and nothing more — BT-6 names the currency, BT-111
states the amount. **The exchange rate is not a business term**, so `converted`
takes it as an argument and keeps none of it:

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![LineItem::fixed("Leistung", Amount::parse("1000.00000")?).build()?])
    .extra_tax(FixedRateTax::new("VAT", dec!(0.19))?.boxed())
    .build()?
    .with_vat_accounting_currency(VatAccountingCurrency::converted(
        Currency::new("PLN")?, Amount::parse("190.00000")?, dec!(4.30), AmountScale::EN16931,
    )?)?;

assert_eq!(doc.vat_total()?, Amount::parse("190.00000")?);   // BT-110, in EUR
let acct = doc.vat_accounting_currency().unwrap();
assert_eq!(acct.currency(),  Currency::new("PLN")?);         // BT-6
assert_eq!(acct.vat_total(), Amount::parse("817.00000")?);   // BT-111
assert!(doc.fits_amount_scale(2));                           // BR-DEC-15
doc.assert_valid();
# Ok::<(), Box<dyn std::error::Error>>(())
```

Four fatal rules govern the pair, and the type satisfies all four by construction
or by check:

| Rule | Source | Requires | How |
|---|---|---|---|
| **BR-53** | CEN | BT-6 present ⇒ BT-111 provided | they are **one value**, so the pair cannot be half-stated |
| **BR-DEC-15** | CEN | BT-111 ≤ 2 decimals | `converted` rounds the exact product **once** at the target scale; `fits_amount_scale` checks it |
| **`PEPPOL-EN16931-R005`** | Peppol | BT-6 ≠ BT-5 | rejected by `with_vat_accounting_currency`, the first place both are known |
| **`PEPPOL-EN16931-R055`** | Peppol | BT-110 and BT-111 share an operational sign | check 14 of `validate()` |

R055 is why the sign check sits in `validate()` rather than only in the setter:
`reverse()`, `AllocationRule` and `merge_period_documents` all reach the field
without passing through it. Each does the right thing — a credit note negates
BT-111 along with BT-110, allocation splits it with the same last-recipient
remainder that keeps BT-110 exact, and merging sums it — and `validate()` is what
proves it rather than assuming it. Merging **refuses** two shapes it cannot sum: a
currency mismatch, and one half stating BT-111 while the other does not, where the
missing figure is *unstated* rather than zero.

`PEPPOL-EN16931-R051` requires every other amount in the document to carry BT-5;
BT-111 is the single documented exception, and nothing else in the document moves.

---

## 🧾 Payment terms and Skonto (BT-20)

EN 16931 models payment terms as **one free-text field**, BT-20. An early-payment
discount — *Skonto*, *escompte*, "2/10 net 30" — is therefore not machine-readable
in the core standard, even though it changes what the buyer pays. Germany's CIUS
closes the gap with a micro-syntax inside that free text, made mandatory by
**BR-DE-18**, and ZUGFeRD reads the same convention:

```text
#SKONTO#TAGE=10#PROZENT=2.00#
#SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#
```

Uppercase, no whitespace, `#` opening every segment and closing the line, exactly
two decimals, one line per discount, **and a newline after the last one**.
`PaymentTerms` is that syntax as a type — `Display` writes it, `FromStr` reads it
back, and neither can produce a form a validator rejects:

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

// Staged terms award the best qualifying window, which is what the buyer takes.
assert_eq!(terms.discount_within(5).map(|d| d.percent()),  Some(dec!(3)));
assert_eq!(terms.discount_within(15).map(|d| d.percent()), Some(dec!(2)));
assert_eq!(terms.discount_within(30), None);
# Ok::<(), Box<dyn std::error::Error>>(())
```

#### The terminator is half the rule

`BR-DE-18` is **two** assertions folded into one `satisfies`, and the second is
easy to read past because it is not a separate `<assert>`. Verbatim from
[`itplr-kosit/xrechnung-schematron`](https://github.com/itplr-kosit/xrechnung-schematron),
`src/validation/schematron/ubl/XRechnung-UBL-validation.sch`:

```xpath
every $line in cac:PaymentTerms/cbc:Note[1]/tokenize(., '(\r?\n)')
                 [starts-with(normalize-space(.), '#')]
  satisfies matches(normalize-space($line), $XR-SKONTO-REGEX)
        and matches(cac:PaymentTerms/cbc:Note[1]/tokenize(., '#.+#')[last()], '^\s*\n')
```

The second `matches` requires everything after the last `#…#` to begin with a
newline. Without one, `tokenize(., '#.+#')[last()]` is the empty string and the
assertion fails — **fatally**, on every German invoice offering Skonto. The rule's
own prose says so directly (*"Am Ende einer vollständigen Skontoangabe muss ein
XML-konformer Zeilenumbruch folgen"*), and KoSIT ships a fixture marked
`schematron-invalid="xrcii:BR-DE-18"` titled *"Every skonto entry should end with a
newline"*.

It lives in `Display` rather than in a separate `to_bt20_xrechnung()` because the
`#SKONTO#…#` micro-syntax **is** the German CIUS convention — there is no core
EN 16931 form of it — so a rendering without the terminator is valid nowhere at all.
A prose-only BT-20 is unchanged: the
`every … satisfies` is vacuously true when no line starts with `#`.

### The discount is not a discount on the invoice

A Skonto is **conditional**: it exists only if the buyer pays early, which has not
happened when the invoice is issued. Deducting it up front would understate the
taxable base for a supply that may well be paid at full price. So attaching payment
terms moves **nothing** — not `net_total`, not `gross_total`, not `amount_due`, not
the VAT breakdown:

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
# Ok::<(), Box<dyn std::error::Error>>(())
```

The deduction comes off the **gross**, which is precisely why taking it later
requires a VAT correction rather than a smaller invoice: the consideration shrinks
after the fact, and the tax follows it — §17 Abs. 1 UStG in Germany, art. 90 of the
VAT Directive elsewhere. That correction is triggered by a *payment event*, which
the invoice cannot know, so this crate computes the payable figure and stops there.

> The EN 16931 revision approved by CEN in October 2025 and published in 2026
> promotes early payment discounts and late-payment charges to structured fields of
> their own. The arithmetic does not change with it — only where the numbers are
> written — so these accessors are what a future serialiser maps from.

### Late-payment interest: the other half of BT-20

BT-20's definition ends *"(Including description of possible penalties)"*, so the
penalty is squarely inside the field — but unlike a Skonto it has **no wire
syntax**. `BR-DE-18` structures early-payment discounts and nothing else, so a
penalty reaches the invoice only as prose, in your language. `LatePaymentPenalty`
is therefore the arithmetic without the sentence:

```rust
use billing::{Amount, DayCountBasis, LatePaymentPenalty};
use rust_decimal::dec;

// §288 Abs. 2 BGB — 9 percentage points above the base rate, B2B — plus the
// 40,00 € fixed sum of §288 Abs. 5 BGB / art. 6(1) of Directive 2011/7/EU.
let penalty = LatePaymentPenalty::new(dec!(11.62))?
    .with_recovery_fee(Amount::parse("40.00000")?)?;

let overdue = Amount::parse("1190.00000")?;

// Simple interest, never compounded: art. 2(6) of the directive says "simple
// interest for late payment", and §289 BGB forbids interest on interest outright.
assert_eq!(penalty.interest_on(overdue, 30)?, Amount::parse("11.36532")?);
// The fixed sum is charged once, not per day.
assert_eq!(penalty.total_on(overdue, 30)?, Amount::parse("51.36532")?);
// Paying on the due date is free — the fixed sum is contingent on interest
// having become payable at all.
assert_eq!(penalty.total_on(overdue, 0)?, Amount::ZERO);

// The day-count denominator is a convention, so it is explicit. The money-market
// year is 1.4 % shorter than the calendar one, and earns that much more.
assert!(penalty.with_day_count(DayCountBasis::Actual360).interest_on(overdue, 30)?
        > penalty.interest_on(overdue, 30)?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**It is not a field of `PaymentTerms`, on purpose.** Everything `PaymentTerms`
holds, its `Display` writes and its `FromStr` reads back; a field with no
representation in the syntax would vanish on the way out, silently. Write the
sentence yourself and pass it to `PaymentTerms::text`.

**It changes no total, for the same reason a Skonto does not.** The trigger is a
payment event that has not happened when the invoice is issued. Default interest is
also generally **outside the scope of VAT** — art. 63 of the VAT Directive ties the
chargeable event to the supply, and interest compensates for delay rather than
paying for anything supplied (CJEU C-222/81, *BAZ Bausystem*); the art. 6 recovery
fee is the same. So a penalty billed later is its own document, typically carrying
`TaxCategory::OutOfScope`, not a line on this one.

| Jurisdiction | Rate | Fixed sum |
|---|---|---|
| EU floor — Directive 2011/7/EU art. 2(6), art. 6(1) | ECB reference rate **+ 8 pp** | **40 €** |
| Germany — §288 Abs. 2, Abs. 5 BGB (B2B) | base rate **+ 9 pp** | 40 € |
| Germany — §288 Abs. 1 BGB (consumer) | base rate **+ 5 pp** | — |

The reference rate moves twice a year, which is why the type takes the *assembled*
annual percentage rather than a spread over a base it would have to carry a stale
copy of.

---

## 💵 Cash rounding and amount due

Many jurisdictions round the amount actually *tendered* to the smallest coin in
circulation. Three properties hold nearly everywhere, and shape the API:
it applies to the **gross total after tax**, the difference is **not taxable**,
and it is a property of the **tender** — cash only, never a card payment.

```rust
use billing::prelude::*;
use billing::CashRounding;

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::CHF, ..Default::default() },
    vec![LineItem::fixed("Leistung", Amount::parse("12.34000")?).build()?],
    vec![], vec![],
)?;

// Swiss Rappenrundung: nearest 0.05.
let rappen = CashRounding::new(Amount::parse("0.05000")?, RoundingStrategy::MidpointAwayFromZero)?;
let doc = doc.with_cash_rounding(rappen)?;

assert_eq!(doc.gross_total(), Amount::parse("12.34000")?);  // unchanged — VAT base intact
assert_eq!(doc.rounding(),    Amount::parse("0.01000")?);   // BT-114
assert_eq!(doc.amount_due()?, Amount::parse("12.35000")?);  // BT-115
# Ok::<(), Box<dyn std::error::Error>>(())
```

There is deliberately **no `CashRounding::for_currency`**: the increment is a
payment-law fact, not a currency fact. CHF has two minor units but rounds cash to
0.05; EUR rounds to 0.05 in Belgium and not at all in Germany. The midpoint rule
also varies — Norway legislates 0.50 up, Denmark leaves 0.25/0.75 undefined, New
Zealand leaves it to the retailer — so it is a parameter.

### Prepayments

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("Jahresverbrauch", Amount::parse("1000.00000")?).build()?],
    vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()],
    vec![],
)?
.with_prepaid(Amount::parse("900.00000")?)?;   // BT-113 Abschlagszahlungen

// The taxable base is untouched — the supply happened in full.
assert_eq!(doc.gross_total(),                   Amount::parse("1190.00000")?);
assert_eq!(doc.tax_breakdown()[0].taxable_base, Amount::parse("1000.00000")?);
// Only the payable figure moves: BT-115 = BT-112 − BT-113 + BT-114.
assert_eq!(doc.amount_due()?,                   Amount::parse("290.00000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

> ⚠️ **Never model prepayments as negative line items or discounts.** That shrinks
> the taxable base and under-declares output tax. In Germany, failing to deduct
> advances correctly on an Endrechnung makes the entire VAT amount payable a
> second time under §14c Abs. 1 UStG.

`amount_due()` may legitimately be **negative** when prepayments exceed the total
— the ordinary utility credit-balance case. It is not clamped.

`prepaid` and itemised [advances](#-advance-payments-and-final-invoices) are the
same fact at different resolutions, so they are **one value**, not two fields:

```rust
use billing::prelude::*;

// A flat figure, when the tax split is unknown or not required …
let flat = Prepayment::total_of(Amount::parse("900.00000")?)?;
// … or itemised, when the settling document must state the tax in each advance.
assert_eq!(flat.total()?, Amount::parse("900.00000")?);
assert!(flat.advances().is_empty());
assert_eq!(Prepayment::None.total()?, Amount::<5>::ZERO);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Because it is one enum, "a total of 900 alongside advances summing to 476" is not
a state that can be written down — no runtime check is needed to reject it.
`with_prepaid` and `with_advances` are thin wrappers over `with_prepayment`, and
each replaces the whole prepayment rather than merging into it.

---

## 🧮 Advance payments and final invoices

Bill in instalments and the settling document has to do two things at once: the
**taxable base must cover the whole supply**, because that is what was supplied,
while the **amount payable is only the remainder**.

[`with_prepaid`](#prepayments) covers the second half — it is EN 16931's BT-113, a
single flat figure. What BT-113 cannot express is the **tax contained in each
advance**, and several jurisdictions require exactly that. Germany is the sharpest
case: §14 Abs. 5 Satz 2 UStG obliges a final invoice to deduct the advances *"und
die auf sie entfallenden Steuerbeträge"*. Omit it and, per UStAE 14.8 Abs. 10, the
issuer owes the full tax shown **plus** the advance portion again under §14c
Abs. 1 — the same tax billed twice.

[`AdvancePayment`] carries that missing structure. It mirrors ZUGFeRD/Factur-X
EXTENDED's `SpecifiedAdvancePayment` group (BG-X-45), the one standardised place
where per-advance tax data lives.

### Settle by deduction — a final invoice

```rust
use billing::prelude::*;
use billing::{AdvancePayment, FixedRateTax, TaxBreakdownEntry};
use rust_decimal::dec;

// The whole supply: 1000.00 net + 19% VAT.
let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("Jahresverbrauch", Amount::parse("1000.00000")?).build()?],
    vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()],
    vec![],
)?;

// Two advances already invoiced and paid, 375.00 net + 71.25 VAT each.
let advance = |r: &str| AdvancePayment::new(vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("375.00000").unwrap(), Amount::parse("71.25000").unwrap(),
)]).unwrap().with_reference(r);

let doc = doc.with_advances(vec![advance("AB-1"), advance("AB-2")])?;

// The base still describes the whole supply …
assert_eq!(doc.tax_breakdown()[0].taxable_base, Amount::parse("1000.00000")?);
assert_eq!(doc.gross_total(),                   Amount::parse("1190.00000")?);
// … while only the remainder is payable.
assert_eq!(doc.prepaid(),            Amount::parse("892.50000")?);   // BT-113
assert_eq!(doc.advance_tax_total()?, Amount::parse("142.50000")?);   // §14 Abs. 5 S. 2
assert_eq!(doc.amount_due()?,        Amount::parse("297.50000")?);   // BT-115

// The deduction table, merged per rate:
let deductions = doc.advance_deductions()?;
assert_eq!(deductions[0].taxable_base, Amount::parse("750.00000")?);
assert_eq!(deductions[0].tax_amount,   Amount::parse("142.50000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

> ⚠️ **Advances are a gross deduction.** Subtracting them from the *net* base
> understates output tax and breaks EN 16931 rules BR-S-08 and BR-CO-14. The engine
> rejects advances that exceed the supply, or that name a VAT rate the supply does
> not contain.

### Settle by residual — bill only what is left

```rust
use billing::prelude::*;
use billing::{advance::residual_breakdown, AdvancePayment, FixedRateTax, TaxBreakdownEntry};
use rust_decimal::dec;

let full = vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("1000.00000")?, Amount::parse("190.00000")?,
)];
let advances = vec![AdvancePayment::new(vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("750.00000")?, Amount::parse("142.50000")?,
)])?];

let residual = residual_breakdown(&full, &advances)?;
assert_eq!(residual[0].taxable_base, Amount::parse("250.00000")?);
assert_eq!(residual[0].tax_amount,   Amount::parse("47.50000")?);
// Now bill exactly that, and attach no advances.
# Ok::<(), Box<dyn std::error::Error>>(())
```

The residual form is structurally simpler and needs no per-advance tax statement,
which is why the German BMF recommends it for structured e-invoices (Schreiben
v. 15.10.2024, Rn. 48) — EN 16931's core profiles have nowhere to put that data.
The engine supports both and takes no position on which you use.

**Neither form is distinguishable by BT-3**: a final invoice and a residual invoice
are both `380`. What tells them apart is whether `advances()` is populated.

### One value, not two

```rust
use billing::prelude::*;
use billing::{AdvancePayment, FixedRateTax, TaxBreakdownEntry};
use rust_decimal::dec;

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("Supply", Amount::parse("1000.00000")?).build()?],
    vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()],
    vec![],
)?;

let advance = AdvancePayment::new(vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("375.00000")?, Amount::parse("71.25000")?,
)])?;

let doc = doc.with_advances(vec![advance])?;
assert!(matches!(doc.prepayment(), Prepayment::Itemised(_)));

// Setting a flat total REPLACES the itemisation wholesale — there is no
// half-in-force state to reason about.
let doc = doc.with_prepaid(Amount::parse("100.00000")?)?;
assert!(doc.advances().is_empty());
assert!(matches!(doc.prepayment(), Prepayment::Total(_)));
# Ok::<(), Box<dyn std::error::Error>>(())
```

### What the engine refuses

Operations that cannot preserve the advance data are errors, not silent drops:
`merge_period_documents` and `AllocationRule` both refuse a document carrying
itemised advances (each advance references a specific prior invoice, so it cannot
be split or combined meaningfully), and both refuse a document carrying a
cash-rounding rule (a rounding adjustment belongs to one payable total).

> **Scope note.** This is the generic mechanism — progress billing in construction,
> deposits in retail, instalment plans and metered utilities all produce the same
> shape. Jurisdiction-specific identifiers and levy catalogues belong in a crate
> layered on top of this one, not here.

---

## 🔄 Credit notes

`reverse()` produces the Storno of a document: every amount negated, every sign
flipped, the VAT breakdown reversed too.

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let invoice = BillingDocument::from_positions(
    DocumentMeta { invoice_number: "INV-9".into(), currency: Currency::EUR, ..Default::default() },
    vec![LineItem::for_usage("Arbeit", Quantity::new(dec!(1000), "kWh"), UnitPrice::new(dec!(0.30), "EUR/kWh")).build()?],
    vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()],
    vec![],
)?;

let credit = invoice.reverse(DocumentMeta {
    invoice_number: "CN-9".into(), currency: Currency::EUR, ..Default::default()
})?;

assert_eq!(credit.gross_total(), Amount::parse("-357.00000")?);
assert_eq!(credit.tax_breakdown()[0].tax_amount, Amount::parse("-57.00000")?);
assert!(credit.net_positions()[0].is_credit());
// Quantities are NOT negated — a reversal is a negative price, not a negative quantity.
assert_eq!(credit.net_positions()[0].quantity_value(), Some(dec!(1000)));

// Invoice + credit note settles to nothing.
assert_eq!(invoice.gross_total().checked_add(credit.gross_total())?, Amount::<5>::ZERO);

// BT-3 is set for you: 380 was passed in via `..Default::default()`, 381 comes out.
assert_eq!(invoice.meta.kind, DocumentKind::CommercialInvoice); // 380
assert_eq!(credit.meta.kind,  DocumentKind::CreditNote);        // 381
# Ok::<(), Box<dyn std::error::Error>>(())
```

### A credit note says what it credits

`reverse()` also fills in **BG-3**, the preceding invoice reference, from the
document being reversed — its number as **BT-25**, its issue date as **BT-26**. The
engine holds that number already; making the caller copy it back by hand, out of a
field the reversal has just consumed, is how it goes missing.

```rust
use billing::prelude::*;

let invoice = BillingDocument::from_positions(
    DocumentMeta {
        invoice_number: "INV-2026-001".into(),
        issue_date: Some("2026-06-30".into()),
        currency: Currency::EUR,
        ..Default::default()
    },
    vec![LineItem::fixed("Beratung", Amount::parse("1000.00000")?).build()?],
    vec![], vec![],
)?;

let credit = invoice.reverse(DocumentMeta {
    invoice_number: "CN-2026-001".into(), currency: Currency::EUR, ..Default::default()
})?;

assert_eq!(credit.meta.preceding[0].number(), "INV-2026-001");        // BT-25
assert_eq!(credit.meta.preceding[0].issue_date(), Some("2026-06-30")); // BT-26

// BG-3 is 0..n — a consolidated credit note states its own list, and keeps it.
let consolidated = invoice.reverse(DocumentMeta {
    invoice_number: "CN-2026-002".into(),
    currency: Currency::EUR,
    preceding: vec![PrecedingInvoice::new("INV-A")?, PrecedingInvoice::new("INV-B")?],
    ..Default::default()
})?;
assert_eq!(consolidated.meta.preceding.len(), 2);
# Ok::<(), Box<dyn std::error::Error>>(())
```

A credit note that does not say what it credits is an unexplained payment, and the
rules agree without being uniform — which is why this is modelled rather than
assumed:

| Rule | Source | Says |
|---|---|---|
| **BR-55** | CEN, fatal | a BG-3 shall contain a BT-25 |
| **`NL-R-001`** | Peppol, **fatal** | for a Dutch supplier, a credit note **MUST** carry BT-25 |
| **`DE-R-026`** | Peppol, warning | a `384` corrected invoice *should* carry BG-3 at least once |

BR-55 is satisfied by the **type**, not by a check: `PrecedingInvoice::number` is
not an `Option`, and the constructor refuses a blank string — a BG-3 without a BT-25
is not constructible. That is the same move `LineVat` makes for BR-S-05.

### The type code is not interchangeable

`BR-CL-01` reads like one code list but is really **two, selected by the syntax
element**. Its Schematron context matches both elements while the test branches on
which one it found:

```xml
<rule context="cbc:InvoiceTypeCode | cbc:CreditNoteTypeCode" flag="fatal">
  <assert id="BR-CL-01" test="
       (self::cbc:InvoiceTypeCode    and contains(' 71 80 81 … 380 382 … 935 ', …))
    or (self::cbc:CreditNoteTypeCode and contains(' 81 83 261 262 296 308 381 396 … ', …))"/>
```

| Element | Size | Codes (excerpt) |
|---|---|---|
| `cbc:InvoiceTypeCode` | 50 | `81`, `326`, **`380`**, `383`, `384`, `386`, `389`, `875`, `876`, `877`, … |
| `cbc:CreditNoteTypeCode` | 13 | `81`, `83`, `261`, `262`, `296`, `308`, **`381`**, `396`, `420`, `458`, `502`, `503`, `532` |

The two share exactly one code — `81` — and `380` / `381` sit one in each. So
putting `381` on a UBL `<Invoice>`, or `380` on a `<CreditNote>`, is fatal at the
**CEN** layer already, before any profile gets a say. `reverse()` therefore forces
`meta.kind` to a credit-note code rather than trusting whatever the caller's
`DocumentMeta` happened to carry, since the idiomatic `..Default::default()`
yields `380`.

`DocumentKind::is_credit_note()` tells a consumer **which element to emit**: of
the ten codes modelled here it is true only for `CreditNote` — note that
`DebitNote` (`383`) is an *invoice*-family document despite the name.

#### Profiles narrow it further, and one of them is party-dependent

`BR-CL-01` is the floor. Peppol BIS Billing 3.0 cuts the invoice list from 50
codes to 26 (`PEPPOL-EN16931-P0100`) and the credit-note list from 13 to five
(`P0101`), both **fatal** — and `P0112` narrows two of *those* by the parties'
countries:

> `[PEPPOL-EN16931-P0112]` Invoice type code 326 or 384 are only allowed when both
> buyer and seller are German organizations

So the admissible BT-3 set is not merely profile-dependent but **party**-dependent.
That is why `DocumentKind` stays a plain code list here and the narrowing lives in
the layer above, which knows the profile and the parties. Two predicates report
what *is* decidable from the code alone:

```rust
use billing::DocumentKind;

// `389` passes BR-CL-01 but is absent from Peppol's P0100 — self-billing is a
// separate Peppol profile, so this is fatal under the Billing customization.
assert!(!DocumentKind::SelfBilledInvoice.is_peppol_billing_code());
assert!(DocumentKind::CommercialInvoice.is_peppol_billing_code());

// P0112's country condition, flagged but not decided.
assert!(DocumentKind::PartialInvoice.requires_german_parties());   // 326
assert!(DocumentKind::CorrectedInvoice.requires_german_parties()); // 384
```

---

## 🏷️ Price details (EN 16931 BG-29)

`UnitPrice` carries the whole of BG-29, not just the mandatory term:

| BT | Name | Field | Rules |
|---|---|---|---|
| BT-146 | Item net price | `value` | BR-26 (mandatory), BR-27 |
| BT-147 | Item price discount | `price_discount` | `PEPPOL-EN16931-R046` |
| BT-148 | Item gross price | `gross_price` | BR-28, `R046` |
| BT-149 | Item price base quantity | `base_quantity` | `R120`, `R121` |
| BT-150 | …its unit of measure code | `base_quantity_code` | BR-CL-23, `R130` |

Both optional halves are ordinary commercial practice rather than exotica — EN
16931-1 **Annex A** spends two of its eight worked examples on them — and both are
**fatal**-rule territory in Peppol.

### BT-149 / BT-150 — "EUR 12,00 per 100 pieces"

Annex A.1.3. Without a price base quantity the caller must pre-divide to EUR 0,12,
which states a BT-146 the seller never quoted and — for a price that does not
divide evenly — bakes a rounding error into the line before the invoice arithmetic
starts. It is also load-bearing rather than decorative: `PEPPOL-EN16931-R120`
computes the line net amount as `BT-131 = BT-129 × (BT-146 ÷ BT-149) + BG-28 − BG-27`.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let line = LineItem::for_usage(
    "Schrauben",
    Quantity::new(dec!(250), "pcs").with_code("H87"),        // BT-129 / BT-130
    UnitPrice::new(dec!(12.00), "EUR/100 pcs")
        .per(dec!(100))                                      // BT-149
        .with_base_quantity_code("H87"),                     // BT-150
).build()?;

assert_eq!(line.net_amount, Amount::<5>::parse("30.00000")?); // 250 × (12.00/100)
assert_eq!(line.unit_price.unwrap().value, dec!(12.00));      // as quoted, not 0.12
# Ok::<(), Box<dyn std::error::Error>>(())
```

`None` means 1, exactly as `R120`'s own `$baseQuantity` variable is defined.
`build()` reassociates the product to `(quantity × price) ÷ base`, so a
non-terminating quotient like `12,00 / 7` is never rounded before it is scaled up.

Two rules are enforced for you: `R121` (base quantity strictly above zero — it is a
divisor) and `R130`, **fatal**, which is a cross-field rule only the line can see:

> `[PEPPOL-EN16931-R130]` Unit code of price base quantity MUST be same as invoiced
> quantity.

### BT-147 / BT-148 — list price less a line discount

Annex A.1.6, which uses the pattern on every line: gross `9,50` − discount `1,00` =
net `8,50`. BT-147 / BT-148 move the **price**, and nothing else — BT-131 is then
computed from the resulting BT-146. That is what separates them from the crate's
two allowance types, which are three different things in the standard:

| Group | Type | Terms | Moves |
|---|---|---|---|
| BG-27 / BG-28 line allowance / charge | `LineAllowanceCharge` | BT-136 … BT-145 | **BT-131** |
| BG-20 / BG-21 document allowance / charge | `AllowanceCharge` | BT-92 … BT-105 | **BT-107 / BT-108** → BT-109 |
| BG-29 price discount | `UnitPrice::discounted` | BT-147 / BT-148 | **BT-146** |

Peppol keeps the price level apart too — `R044` forbids a *charge* at price level
outright while allowing the discount.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let price = UnitPrice::discounted(dec!(9.50), dec!(1.00), "EUR/pcs");
assert_eq!(price.value, dec!(8.50));  // BT-146 is derived, never passed in

let line = LineItem::for_usage("Ware", Quantity::new(dec!(20), "pcs"), price).build()?;
assert_eq!(line.net_amount, Amount::<5>::parse("170.00000")?);
assert!(line.allowance_charge.is_none());  // the price moved, not the line total
# Ok::<(), Box<dyn std::error::Error>>(())
```

BT-146 is **derived** because `PEPPOL-EN16931-R046` is an *exact* equality — unlike
`R040`'s ±0.02, it carries no `u:slack` — so there is no room for a caller to
compute the net price and be a cent out. `rounded()` honours this too: with a gross
price present it rounds BT-148 and BT-147, the numbers the seller actually quoted,
and recomputes BT-146 from them.

> **BR-27 / BR-28 are the caller's to honour.** EN 16931 says BT-146 and BT-148
> shall not be negative. This crate accepts negative prices anyway, because they
> are legally binding in spot markets (EPEX negative-price hours, §27 EEG 2023) and
> refusing them would make the engine unable to represent a real invoice. A
> consumer must re-model such a line — as a credit position, or an allowance —
> before emitting EN 16931.

### A flat charge is still an invoice line

A Grundpreis, a monthly seat fee, a connection fee — one amount, no natural
quantity. `LineItem::fixed` states exactly that, which is convenient and **three
fatal rules short of an EN 16931 invoice line**. All three have `$Invoice_Line` as
their Schematron context, so they apply to *every* line without exception:

| Rule | Requires |
|---|---|
| BR-22 | Invoiced quantity (BT-129) |
| BR-23 | Invoiced quantity unit of measure code (BT-130) |
| BR-26 | Item net price (BT-146) |

`LineItem::flat_fee` states the same money as the line the standard asks for —
one unit, at a unit price equal to the whole amount:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let fee = LineItem::flat_fee("Grundpreis", Amount::parse("8.50000")?, Currency::EUR).build()?;

assert_eq!(fee.net_amount, Amount::<5>::parse("8.50000")?);
assert_eq!(fee.quantity.as_ref().unwrap().value, dec!(1));                 // BT-129
assert_eq!(fee.quantity.as_ref().unwrap().code.as_deref(),
           Some(UNIT_CODE_ONE));                                           // BT-130 = C62
assert_eq!(fee.unit_price.as_ref().unwrap().value, dec!(8.5));             // BT-146
assert_eq!(fee.unit_price.as_ref().unwrap().unit, "EUR");
# Ok::<(), Box<dyn std::error::Error>>(())
```

`UNIT_CODE_ONE` is `C62`, UN/ECE Rec 20 for *one* — the code for a countable item
with no other unit. `1 × 8,50` is exactly `8,50`, so `R120` holds trivially and
nothing is rounded that was not rounded before.

The **currency is an argument** because the price label is generated and a
generated label cannot invent one. `RateSchedule`, `TimeOfUsePricing`,
`DynamicPricing` and `PerUnitLevy` all take it for the same reason;
`flat_fee` was the last place in the crate that hardcoded `"EUR"`, and so printed
a euro price on every Swiss, sterling and dollar invoice.

`fixed` is still the right constructor when the position is **not** an invoice line
— a document level allowance or charge (BG-20 / BG-21) has no BG-25 terms at all —
or when you map to a syntax with no such requirement.

---

## ➖ Line allowances and charges (EN 16931 BG-27 / BG-28)

A deduction or addition that moves **one line's** net amount — a volume discount on
that line, a packaging charge for that item. `LineAllowanceCharge` carries the
group, and `build()` folds it into `net_amount`:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let line = LineItem::for_usage(
    "Ware",
    Quantity::new(dec!(100), "pcs"),
    UnitPrice::new(dec!(10.00), "EUR/pcs"),
)
// 5 % volume discount — BT-136 with its BT-137 / BT-138 basis.
.line_allowance(
    LineAllowanceCharge::allowance(Amount::parse("50.00000")?, "Mengenrabatt")
        .of(Amount::parse("1000.00000")?, dec!(0.05)),
)
// Flat handling charge — BT-141.
.line_allowance(LineAllowanceCharge::charge(Amount::parse("12.50000")?, "Verpackung"))
.build()?;

// R120 in full: 100 × 10,00 − 50,00 + 12,50
assert_eq!(line.net_amount, Amount::<5>::parse("962.50000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

| BT | Allowance (BG-27) | Charge (BG-28) |
|---|---|---|
| amount | BT-136 | BT-141 |
| base amount | BT-137 | BT-142 |
| percentage | BT-138 | BT-143 |
| reason | BT-139 | BT-144 |
| reason code | BT-140 (UNCL 5189) | BT-145 (UNCL 7161) |

**These reach the document totals only through BT-131.** BT-106 is the sum of the
BT-131s, so the totals chain and the VAT breakdown need no special case — and VAT
is charged on the reduced base, which is the point.

**BR-42 / BR-44** (restated by BR-CO-23 / BR-CO-24) require a reason *or* a reason
code. A document level allowance can lean on the position's `description` for
BT-97 / BT-104; a line allowance has no description of its own, so the constructors
take the reason and `validate()` rejects a pair with neither.

`PEPPOL-EN16931-R040` / `R041` / `R042` list `cac:InvoiceLine/cac:AllowanceCharge`
in their contexts alongside the document level element, so the base-and-percentage
rules apply here **identically** — the checks are shared with `AllowanceCharge`
rather than reimplemented. `scaled()` and `reverse()` move these with the line, so
the stated parts never contradict BT-131.

> **Three things that look alike.** `LineAllowanceCharge` (BG-27/28) moves BT-131;
> `AllowanceCharge` (BG-20/21) moves the document totals; `UnitPrice::discounted`
> (BG-29) moves the price. Picking the wrong one changes the VAT base.

They sit at different *levels*, not just different totals: BG-27 / BG-28 are
children of an invoice line (BG-25), BG-20 / BG-21 are children of the document,
and UBL nests `cac:AllowanceCharge` only under `cac:InvoiceLine`. A position that
is a document level allowance therefore cannot itself carry line allowances, and
`build()` rejects the combination.

---

## ➗ Splitting money exactly

Dividing money is not division. `total / n` either loses cents or invents them;
these two methods do neither.

```rust
use billing::Amount;

// distribute: N equal-as-possible parts, exact sum.
let parts = Amount::<2>::parse("0.10")?.distribute(3)?;
assert_eq!(parts, vec![
    Amount::<2>::parse("0.04")?,
    Amount::<2>::parse("0.03")?,
    Amount::<2>::parse("0.03")?,
]);

// allocate: split by integer ratios, largest-remainder, exact sum.
let parts = Amount::<2>::parse("100.00")?.allocate(&[1, 1, 1])?;
assert_eq!(parts[0], Amount::<2>::parse("33.34")?);   // someone takes the extra cent
let sum: Amount<2> = parts.into_iter().sum();
assert_eq!(sum, Amount::<2>::parse("100.00")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Use [`proportional_split`](#raw-quantity-split-proportional_split) instead when
splitting a physical quantity (kWh, m³) rather than money.

---

## 📊 `RateSchedule` — four pricing modes

```rust
use billing::{RateSchedule, RateBand, Amount, Currency};
use rust_decimal::dec;

// ── Mode 1: Graduated — each tier at its own price ──────────────────────────
let graduated = RateSchedule::graduated()
    .unit("kWh").currency(Currency::EUR)
    .band(RateBand::up_to(dec!(500), Amount::parse("0.32000")?))
    .band(RateBand::over(dec!(500),  Amount::parse("0.28000")?))
    .build()?;
assert_eq!(graduated.split(dec!(1234.5))?.len(), 2);  // 500 × 0.32, then 734.5 × 0.28

// ── Mode 2: Volume — ALL units at the top tier reached ──────────────────────
let volume = RateSchedule::volume()
    .unit("kWh").currency(Currency::EUR)
    .band(RateBand::up_to(dec!(1000), Amount::parse("0.32000")?))
    .band(RateBand::over(dec!(1000),  Amount::parse("0.28000")?))
    .build()?;
let v = volume.split(dec!(1234.5))?;
assert_eq!(v[0].net_amount, Amount::parse("345.66000")?);  // 1234.5 × 0.28

// ── Mode 3: Block — per N-unit block, rounded UP ────────────────────────────
// Use case: parking (30-min slots), telephony, data packs
let block = RateSchedule::block()
    .unit("GB").currency(Currency::EUR)
    .band(RateBand::block(dec!(10), Amount::parse("1.50000")?))
    .build()?;
let b = block.split(dec!(35))?;
assert_eq!(b[0].net_amount, Amount::parse("6.00000")?);    // 4 blocks × 1.50

// ── Mode 4: Capacity — a single value selects the tier, not a cumulative sum ─
// Use case: demand charge (peak kW), bandwidth (P95 or max Mbps), concurrent seats
let capacity = RateSchedule::capacity()
    .unit("Mbps").currency(Currency::EUR)
    .band(RateBand::up_to(dec!(100), Amount::parse("50.00000")?))
    .band(RateBand::over(dec!(100),  Amount::parse("100.00000")?))
    .build()?;
assert_eq!(capacity.apply_peak(dec!(112.8))?.net_amount, Amount::parse("100.00000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Validation at build time.** `build()` returns `Err` when: the band list is
empty; a band price is negative; a bound is negative or non-positive;
`lower >= upper`; upper bounds are not strictly increasing; bands are
non-contiguous; a non-final band is open-ended; block mode does not have exactly
one band; or `block_size <= 0`. A schedule that builds prices correctly.

```rust
use billing::{RateSchedule, RateBand, Amount};
use rust_decimal::dec;

// Descending bounds are rejected up front rather than mispricing later.
assert!(RateSchedule::graduated()
    .band(RateBand::up_to(dec!(100), Amount::parse("1.00000")?))
    .band(RateBand::up_to(dec!(50),  Amount::parse("2.00000")?))
    .build()
    .is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 🕐 Time-of-use and dynamic pricing

```rust
use billing::{TimeOfUsePricing, TimeBand, DynamicPricing, Amount, Currency};
use rust_decimal::dec;

// N-band ToU: caller supplies pre-aggregated consumption per band name.
// The engine has zero knowledge of time zones or grid schedules.
let tou = TimeOfUsePricing::builder()
    .unit("kWh")
    .currency(Currency::EUR)
    .band(TimeBand::new("peak", Amount::parse("0.32000")?))
    .band(TimeBand::new("off-peak", Amount::parse("0.18000")?))
    .build()?;

let items = tou.calculate(&[("peak", dec!(823.4)), ("off-peak", dec!(411.1))])?;
assert_eq!(items.len(), 2);

// An unknown band name is an ERROR, never a silent skip: a typo must not drop
// real consumption off the invoice.
assert!(tou.calculate(&[("Peak", dec!(823.4))]).is_err());

// Dynamic / spot pricing: one (quantity, price) pair per interval.
let dp = DynamicPricing::builder()
    .unit("kWh")
    .currency(Currency::EUR)
    .interval(dec!(100.0), Amount::parse("0.10000")?)
    .interval(dec!(200.0), Amount::parse("0.20000")?)
    .build()?;

// Single LineItem; net is the exact accumulated total, and the unit price shown
// is the weighted average (informational only).
assert_eq!(dp.calculate()?.net_amount, Amount::parse("50.00000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**One rounding over the whole sequence, not one per interval.** An interval is not
a *leaf*: unlike a ToU band or a schedule tier it never becomes its own invoice
line, so there is nothing at that level to round to. `Σ(qty × price)` is therefore
accumulated as exact `Decimal` and reduced once at the end.

The difference is not academic. A settled spot-price year is 8 760 hourly intervals
(35 040 quarter-hourly), and commercial rounding is not unbiased on a repeated
fractional part — so rounding each product to the engine's five decimals first
drifts in *one* direction, by up to `5 × 10⁻⁶` per interval. Over a year that is
`0.0438`: four whole cents on a two-decimal invoice, from arithmetic alone.

---

## 🔢 Usage aggregation

Before applying a pricing model, aggregate raw events to a scalar:

```rust
use billing::{SumAggregator, CountAggregator, UniqueCountAggregator,
              MaxAggregator, MeanAggregator, PercentileAggregator,
              LatestAggregator, WeightedSumAggregator, UsageAggregator};
use rust_decimal::Decimal;
use rust_decimal::dec;

struct ApiCall { user_id: String, tenant_id: u64, bytes: u64 }
struct VmEvent { vcpus: u32, uptime_fraction: Decimal }

let events = vec![
    ApiCall { user_id: "alice".into(), tenant_id: 1, bytes: 100 },
    ApiCall { user_id: "bob".into(),   tenant_id: 2, bytes: 200 },
    ApiCall { user_id: "alice".into(), tenant_id: 1, bytes: 300 },
];
let vms = vec![VmEvent { vcpus: 4, uptime_fraction: dec!(0.5) }];

// SUM: total bytes transferred
assert_eq!(
    SumAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events)?,
    dec!(600)
);

// COUNT: number of API requests
assert_eq!(CountAggregator.aggregate(&events)?, dec!(3));

// UNIQUE_COUNT: unique active tenants. The key can be ANY Hash + Eq type.
// A Copy key such as u64 is zero-allocation — no String per event.
assert_eq!(
    UniqueCountAggregator::new(|e: &ApiCall| e.tenant_id).aggregate(&events)?,
    dec!(2)
);
// An owned key works too, at the cost of one clone per event.
assert_eq!(
    UniqueCountAggregator::new(|e: &ApiCall| e.user_id.clone()).aggregate(&events)?,
    dec!(2)
);
// NOTE: the key type may not BORROW from the event — `|e| e.user_id.as_str()`
// does not compile, because the key type is fixed independently of the
// event's lifetime. Use a Copy key, an owned key, or hold a `&'a str` in the
// event struct itself (see the UniqueCountAggregator docs).

// MAX: peak value → pair with RateSchedule::capacity()
assert_eq!(
    MaxAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events)?,
    dec!(300)
);

// LATEST: end-of-period snapshot (last element in slice order)
assert_eq!(
    LatestAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events)?,
    dec!(300)
);

// MEAN: average over evenly spaced samples — average seats, average daily storage
assert_eq!(
    MeanAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events)?,
    dec!(200)
);

// PERCENTILE: burstable billing — see the next section
assert_eq!(
    PercentileAggregator::p95(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events)?,
    dec!(300)
);

// WEIGHTED_SUM: VM CPU-hours for VMs active only part of the period
assert_eq!(
    WeightedSumAggregator::new(
        |e: &VmEvent| Decimal::from(e.vcpus),
        |e: &VmEvent| e.uptime_fraction,
    ).aggregate(&vms)?,
    dec!(2.0)
);

// `aggregate` is fallible because this is the least trusted boundary in the
// pipeline: `Decimal`'s `+` and `*` PANIC on overflow, so one corrupt event would
// otherwise abort the whole batch run instead of reporting itself.
assert!(SumAggregator::new(|_: &ApiCall| Decimal::MAX).aggregate(&events).is_err());
// COUNT, UNIQUE_COUNT, MAX, PERCENTILE and LATEST select rather than accumulate,
// so they never return Err — the uniform signature is what lets you write code
// that is generic over `UsageAggregator`.
# Ok::<(), billing::BillingError>(())
```

### Burstable (95th-percentile) billing

Every IP transit, CDN egress and DDoS-scrubbing contract settles the same way, and
none of the other aggregators can express it. The provider samples throughput every
five minutes, throws away the top 5 % of the samples, and bills the highest one
left — so the customer may burst above their committed rate for roughly 36 hours in
a 30-day month and pay nothing extra for it. `MAX` would bill the single worst
five-minute window of the month; `MEAN` would bill a rate the link never carried.

```rust
use billing::{PercentileAggregator, UsageAggregator, RateSchedule, RateBand,
              Amount, Currency};
use rust_decimal::{Decimal, dec};

struct Sample { mbps: u32 }
let mbps = |s: &Sample| Decimal::from(s.mbps);

// A month of five-minute samples: a flat 100 Mbps with a short 900 Mbps burst.
let mut month: Vec<Sample> = (0..8_640 - 200).map(|_| Sample { mbps: 100 }).collect();
month.extend((0..200).map(|_| Sample { mbps: 900 }));

// 200 of 8 640 samples is 2.3 % — inside the 5 % that is discarded.
let committed = PercentileAggregator::p95(mbps).aggregate(&month)?;
assert_eq!(committed, dec!(100));

// The tier is selected by the P95 figure, and a capacity charge is flat.
assert_eq!(RateSchedule::capacity()
    .unit("Mbps")
    .currency(Currency::EUR)
    .band(RateBand::over(dec!(0), Amount::parse("0.50000")?))
    .build()?
    .apply_peak(committed)?
    .net_amount, Amount::parse("0.50000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Nearest-rank, never interpolated.** The samples are sorted ascending and the one
at rank `ceil(p ÷ 100 × n)` is returned. With the conventional 8 640 five-minute
samples, "discard the top 5 %" removes 432 and the next highest is sample 8 208 —
which is exactly `ceil(0.95 × 8640)`. The other common percentile definition
averages the two neighbouring samples, which would bill a throughput figure that
never occurred on the link. Every figure this returns is a rate the customer really
reached.

---

## 🏗️ Implementing `PricingModel`

The `PricingModel` trait is the primary extension point. Implement it once per
pricing arrangement in *your* crate.

### PricingModel is not RateSchedule

A **`RateSchedule` does not implement `PricingModel`**, and could not usefully. They
sit at different levels: a [`RateSchedule`](#-rateschedule--four-pricing-modes) is a
**rate table** — the banded prices themselves — alongside `RateLookup`,
`TimeOfUsePricing` and `DynamicPricing`. A `PricingModel` is the **adapter** that
decides which of those to consult, with what quantities, in what order, and which tax
and discount layers apply. A schedule is something a model *uses*; it has nothing to
say about tax layers.

The names are deliberately domain-neutral, like every other public name here — a
utility says *tariff*, a SaaS says *plan* or *rate card*, and the crate assumes
neither.

### Usage-driven — `PricingModel`

```rust
use billing::{PricingModel, Positions, LineItem, Amount, Quantity, UnitPrice,
              TaxLayer, BillingError, DocumentMeta, Currency, FixedRateTax};
use rust_decimal::Decimal;
use rust_decimal::dec;
use std::convert::Infallible;

struct SaasPlan { base_fee: u32 }
struct Seats { count: u32 }

impl PricingModel for SaasPlan {
    type Usage = Seats;
    type Error = BillingError;
    // This plan always produces an invoice, so "nothing to bill" is uninhabited
    // and `.bill()` hands back a document with no extra matching.
    type NotBillable = Infallible;

    fn positions(&self, usage: &Seats) -> Result<Positions<Infallible>, BillingError> {
        Ok(vec![
            LineItem::fixed("Platform fee", Amount::<5>::from_int(self.base_fee.into()))
                .build()?,
            LineItem::debit("Seats")
                .quantity(Quantity::new(Decimal::from(usage.count), "seats"))
                .unit_price(UnitPrice::new(dec!(19), "EUR/seat"))
                .build()?,
        ].into())
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        // `new` is fallible; a hardcoded literal rate is one of the few places
        // `expect` is defensible — it cannot fail for a valid constant.
        vec![FixedRateTax::new("VAT", dec!(0.20)).expect("0.20 is a valid rate").boxed()]
    }
}

// Build a document in one call:
let doc = SaasPlan { base_fee: 49 }.bill(
    DocumentMeta {
        invoice_number: "INV-001".into(),
        period_label:   "2026-07".into(),
        currency:       Currency::EUR,
        ..Default::default()
    },
    &Seats { count: 5 },
)?;

// 49 + 5×19 = 144 net, +20% VAT
assert_eq!(doc.net_total(),   Amount::parse("144.00000")?);
assert_eq!(doc.gross_total(), Amount::parse("172.80000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Settlements with no usage — `settle`

Plenty of settlements are not usage-driven: a subsidy payout, a redispatch
compensation, an EEG or KWKG settlement whose figures were determined upstream. Set
`type Usage = ()` and call `settle`, which is `bill` without the `&()` at the call
site:

```rust
use billing::{PricingModel, Positions, LineItem, Amount, DocumentMeta, Currency, BillingError};
use std::convert::Infallible;

struct EegSettlement { payout_eur: i64 }

impl PricingModel for EegSettlement {
    type Usage = ();
    type Error = BillingError;
    type NotBillable = Infallible;

    fn positions(&self, _: &()) -> Result<Positions<Infallible>, BillingError> {
        Ok(vec![
            LineItem::credit_fixed("EEG Vergütung", Amount::<5>::from_int(self.payout_eur))
                .build()?,
        ].into())
    }
}

let meta = DocumentMeta { currency: Currency::EUR, ..Default::default() };
let doc = EegSettlement { payout_eur: 400 }.settle(meta)?;
assert_eq!(doc.net_total(), Amount::parse("-400.00000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

There is deliberately no second trait for this case. One would save a
`type Usage = ();` line and one ignored parameter, at the cost of a second vocabulary
(`positions` versus `line_items`, `settle` versus `bill`) and a coherence hazard:
implementing both for one type is a compile error. One trait is the better trade.

### The model declares the precision it prices for

A model knows which format its output is destined for; the call site often does
not. `amount_scale` is where it says so, and `bill`, `try_bill`, `settle` and
`BillingDocumentBuilder::pricing` all honour it:

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;
use std::convert::Infallible;

struct MeteredPlan;

impl PricingModel for MeteredPlan {
    type Usage = ();
    type Error = Infallible;
    type NotBillable = Infallible;

    fn positions(&self, _: &()) -> Result<Positions<Infallible>, Infallible> {
        // 1234.567 kWh × 0.28901 EUR/kWh = 356.80221 — five decimals.
        Ok(vec![LineItem::for_usage(
            "Arbeit",
            Quantity::new(dec!(1234.567), "kWh"),
            UnitPrice::new(dec!(0.28901), "EUR/kWh"),
        ).build().unwrap()].into())
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed()]
    }

    // This model prices for EN 16931, and says so.
    fn amount_scale(&self) -> Option<AmountScale> { Some(AmountScale::EN16931) }
}

let doc = MeteredPlan.settle(DocumentMeta {
    currency: Currency::EUR, ..Default::default()
})?;

assert_eq!(doc.net_total(), Amount::parse("356.80000")?);
assert!(doc.fits_amount_scale(2));   // emittable, straight out of `settle`
# Ok::<(), Box<dyn std::error::Error>>(())
```

Without it, `bill` and `settle` could only ever assemble at the engine's five
decimals, so the shortest path from a model to a document produced one that fails
BR-DEC-23 — with no error anywhere. An explicit `amount_scale` on the builder still
wins, so a call site can override the model's choice, but it does not have to
remember to.

---

## 🚦 Three outcomes: billable, not billable, error

`Result<Vec<LineItem>, Error>` offers two answers. Settlements have three.

A settlement can be **not billable yet, for a specific and entirely expected
reason** — no meter reading has arrived, the reference price for the period is
unpublished, the subsidy entitlement has ended. That is neither a set of
positions nor a failure: nothing went wrong, there is simply nothing to invoice.

Flattened into `Ok(vec![])` the reason is destroyed, and *"we billed nothing"*
becomes indistinguishable from *"there was nothing to bill, because X"* — a
distinction every audit trail needs and that no caller can reconstruct afterwards.
Pushed into `Err` it puts an ordinary business state on the error path, where a
missing price is indistinguishable from a genuine arithmetic fault.

So `positions` returns `Positions<Self::NotBillable>` — an alias for
`Billing<Vec<LineItem>, R>` — and `R` is *your* reason type, matched exhaustively:

```rust
use billing::{PricingModel, Billing, Positions, Billed, LineItem, Amount,
              DocumentMeta, Currency, BillingError};
use std::convert::Infallible;
use std::fmt;

#[derive(Debug, PartialEq)]
enum NotYet { NoMeterReading, PriceUnpublished, EntitlementEnded }

impl fmt::Display for NotYet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMeterReading    => f.write_str("no meter reading for the period"),
            Self::PriceUnpublished  => f.write_str("reference price not yet published"),
            Self::EntitlementEnded  => f.write_str("subsidy entitlement has ended"),
        }
    }
}

struct Settlement;
struct Readings { kwh: Option<u32>, price_published: bool }

impl PricingModel for Settlement {
    type Usage = Readings;
    type Error = BillingError;
    type NotBillable = NotYet;

    fn positions(&self, usage: &Readings) -> Result<Positions<NotYet>, BillingError> {
        let Some(kwh) = usage.kwh else {
            return Ok(Billing::NotBillable(NotYet::NoMeterReading));
        };
        if !usage.price_published {
            return Ok(Billing::NotBillable(NotYet::PriceUnpublished));
        }
        Ok(vec![
            LineItem::fixed("Arbeit", Amount::<5>::from_int(kwh.into())).build()?,
        ].into())
    }
}

let meta = || DocumentMeta { currency: Currency::EUR, ..Default::default() };

// Not billable — and the reason survives, typed.
let outcome: Billed<NotYet> =
    Settlement.try_bill(meta(), &Readings { kwh: None, price_published: true })?;
assert_eq!(outcome.reason(), Some(&NotYet::NoMeterReading));

// Billable.
let outcome = Settlement.try_bill(meta(), &Readings { kwh: Some(100), price_published: true })?;
assert_eq!(outcome.billable().unwrap().net_total(), Amount::parse("100.00000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Models that always bill pay nothing for this.** Set
`type NotBillable = Infallible` and the `NotBillable` variant is uninhabited — the
compiler knows the outcome, and you keep the two-outcome API:

| Can decline to bill | `PricingModel` | `BillingDocumentBuilder` |
|---------------------|----------|--------------------------|
| No (`NotBillable = Infallible`) | `.bill(meta, usage) -> Result<BillingDocument, _>` | `.pricing(&m, &u)` |
| Yes | `.try_bill(meta, usage) -> Result<Billed<R>, _>` | `.try_pricing(&t, &u)` |

The bound is what makes it a compile-time distinction: a model that can decline
to bill has no `.bill()` method, so its reason cannot be silently dropped.

---

## 🧮 Tax layers & compound taxes

Tax layers are **ordered and cumulative**: each layer receives all previously
computed positions (net + discounts + prior taxes) in its base. This is
required for jurisdictions where one levy sits inside the base of a later tax
(e.g. an excise duty that is then subject to VAT).

```rust
use billing::{BillingDocument, DocumentMeta, LineItem, Amount, Currency,
              TaxLayer, FixedRateTax, PercentageCharge};
use rust_decimal::dec;

let pos = vec![LineItem::fixed("Net charge", Amount::parse("100.00000")?).build()?];

// Layer 1: 5% levy on the net.
// Layer 2: 19% VAT — base is net (100) + levy (5) = 105.
let taxes: Vec<Box<dyn TaxLayer>> = vec![
    PercentageCharge::new("Levy", dec!(0.05))?.boxed(),
    FixedRateTax::new("VAT", dec!(0.19))?.boxed(),
];

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    pos, taxes, vec![],
)?;

assert_eq!(doc.net_total(),   Amount::parse("100.00000")?);
// Levy = 5.00;  VAT = 105 × 0.19 = 19.95
assert_eq!(doc.tax_total(),   Amount::parse("24.95000")?);
assert_eq!(doc.gross_total(), Amount::parse("124.95000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Tax regimes are layer sets, not types

A "tax regime" is *which layers apply*, over the same positions. That makes it
**data** — build the `Vec<Box<dyn TaxLayer>>` for the regime and return it from
`tax_layers()`. There is no need for one model type, wrapper struct or type alias
per regime.

The German VAT matrix for a PV settlement, in full:

```rust
use billing::{FixedRateTax, TaxCategory, TaxLayer};
use rust_decimal::dec;

enum Regime { Regelbesteuerung, ParagraphTwelveAbsThree, Kleinunternehmer }

fn layers_for(regime: &Regime) -> Result<Vec<Box<dyn TaxLayer>>, billing::BillingError> {
    Ok(match regime {
        // Standard rate.
        Regime::Regelbesteuerung =>
            vec![FixedRateTax::new("USt 19%", dec!(0.19))?.boxed()],
        // §12 Abs. 3 UStG — 0%, input tax still deductible, no reason text.
        Regime::ParagraphTwelveAbsThree =>
            vec![FixedRateTax::zero_rated("§12 Abs. 3 UStG").boxed()],
        // §19 UStG small-business exemption — reason mandatory.
        Regime::Kleinunternehmer => vec![
            FixedRateTax::exempt(
                "§19 UStG",
                TaxCategory::Exempt,
                "Kleinunternehmer gemäß §19 UStG",
            )?.boxed(),
        ],
    })
}

assert_eq!(layers_for(&Regime::Regelbesteuerung)?.len(), 1);
assert_eq!(layers_for(&Regime::Kleinunternehmer)?.len(), 1);
# Ok::<(), Box<dyn std::error::Error>>(())
```

> ⚠️ **Order matters.** Tax layers are applied in declaration order.
> Place levies that form part of the VAT base *before* VAT.

### Per-unit levies

`PerUnitLevy` bills a rate per physical unit rather than a percentage. It sums
quantities from **debit positions whose unit label matches**, so credits
(feed-in, refunds) are correctly excluded from an excise base.

```rust
use billing::{PerUnitLevy, TaxLayer, LineItem, Amount, Currency, Quantity, UnitPrice};
use rust_decimal::dec;

let levy = PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?
    .with_currency(Currency::EUR);

let positions = vec![
    LineItem::for_usage("Arbeit", Quantity::new(dec!(1000), "kWh"), UnitPrice::new(dec!(0.30), "EUR/kWh")).build()?,
    // A credit position — excluded from the levy base.
    LineItem::credit_for_usage("Einspeisung", Quantity::new(dec!(400), "kWh"), UnitPrice::new(dec!(0.08), "EUR/kWh")).build()?,
];

// 1000 kWh × 0.02050 = 20.50 (the 400 kWh credit is not levied)
assert_eq!(levy.compute(&positions)?.net_amount, Amount::parse("20.50000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Per-unit levies **stack safely**. Each layer sees prior layers' output (needed for
percentage taxes to compound), but a levy's own emitted line carries a quantity in
the same unit — so levies exclude positions tagged `"tax"` from their base. Stacking
Stromsteuer and Konzessionsabgabe, both in ct/kWh, bills each against the true
1000 kWh rather than doubling the second one. A custom `TaxLayer` that emits a
quantity should tag its output `"tax"` to participate correctly.

---

## 🏷️ Discounts

```rust
use billing::{PercentageDiscount, FixedDiscount, DiscountLayer,
              BillingDocument, DocumentMeta, LineItem, Amount, Currency};
use rust_decimal::dec;

let discounts: Vec<Box<dyn DiscountLayer>> = vec![
    // 10% loyalty discount on all debit positions
    PercentageDiscount::new("Loyalty -10%", dec!(0.10))?.boxed(),
    // Fixed 15.00 voucher
    FixedDiscount::new("Voucher", Amount::parse("15.00000")?)?.boxed(),
];

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("Item", Amount::parse("100.00000")?).build()?],
    vec![],
    discounts,
)?;

assert_eq!(doc.discount_total(), Amount::parse("-25.00000")?);
assert_eq!(doc.net_total(),      Amount::parse("75.00000")?);

// Restrict a discount to positions carrying a tag:
let _tagged = PercentageDiscount::new("Volume rebate", dec!(0.05))?.with_tag("commodity");
# Ok::<(), Box<dyn std::error::Error>>(())
```

Discounts are applied **before** tax layers, so they reduce the taxable base.

> **Discounts do not compound.** Unlike tax layers, every `DiscountLayer`
> receives the *original* net positions — never a prior discount's output. Two
> stacked 10% discounts take 10% + 10% of the same base (20% total), not
> 10% of 90% (19%).

---

## 💸 Percentage charge

A `PercentageCharge` models a commercial surcharge (platform fee, marketplace
commission, payment processing) with optional floor and ceiling:

```rust
use billing::{PercentageCharge, TaxLayer, LineItem, Amount};
use rust_decimal::dec;

// 3% commission, floored at 2.00 and capped at 50.00
let commission = PercentageCharge::new("Commission", dec!(0.03))?
    .with_min(Amount::parse("2.00000")?)
    .with_max(Amount::parse("50.00000")?);

// 3% of 10.00 = 0.30, raised to the 2.00 floor
let small = vec![LineItem::fixed("Small", Amount::parse("10.00000")?).build()?];
assert_eq!(commission.compute(&small)?.net_amount, Amount::parse("2.00000")?);

// 3% of 10 000 = 300.00, capped at 50.00
let large = vec![LineItem::fixed("Large", Amount::parse("10000.00000")?).build()?];
assert_eq!(commission.compute(&large)?.net_amount, Amount::parse("50.00000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Place it *before* VAT in the tax layer list so the commission is included in
the VAT base.

> **Note:** `PercentageCharge` implements `TaxLayer`, so its output lands in
> `tax_positions` and is included in `tax_total()`. Filter on the
> `"percentage-charge"` tag if you need to present commissions separately, or use
> `charge_total()` (BT-108), which is exact.

### Neither percentage layer reverses its own direction

`PercentageCharge::new` refuses a negative rate — a negative charge is a discount
and belongs in a `DiscountLayer`. The same result is reachable through a negative
**base**, and is refused the same way: both percentage layers clamp a base that
nets out below zero, so there is nothing to take a percentage of.

```rust
use billing::prelude::*;
use billing::{PercentageCharge, PercentageDiscount};
use rust_decimal::dec;

// 1000 kWh at −0,01 EUR/kWh — a debit position at a negative spot price.
let epex = || vec![LineItem::for_usage(
    "EPEX Spot (negativ)",
    Quantity::new(dec!(1000), "kWh"),
    UnitPrice::new(dec!(-0.01), "EUR/kWh"),
).build().unwrap()];

let fee = PercentageCharge::new("Commission", dec!(0.05))?;
assert_eq!(fee.compute(&epex())?.net_amount, Amount::<5>::ZERO);

let rebate = PercentageDiscount::new("Treuerabatt", dec!(0.05))?;
assert_eq!(rebate.compute(&epex())?.net_amount, Amount::<5>::ZERO);
# Ok::<(), Box<dyn std::error::Error>>(())
```

Without the clamp each would move the total the wrong way for what it claims to be:
a "charge" that increases what the supplier owes the customer, and a "discount" that
does the same. A debit at a negative price still **contributes** to the base and
reduces it — that is what separates it from a `Sign::Credit` position, which is
excluded outright — and only a base that ends up below zero is clamped.

---

## 👥 Allocation across N recipients

Split a document proportionally. Allocation is **arithmetically exact**:
`Σ(recipient totals) == original total` and each sub-document passes
`assert_valid()`. Quantities are scaled alongside amounts, so an allocated line
still reads correctly (`400 kWh × 0.30 = 120.00`, not `1000 kWh × 0.30 = 120.00`).

```rust
use billing::{ProportionalAllocation, EqualAllocation, AllocationRule,
              BillingDocument, DocumentMeta, LineItem, Amount, Currency, Quantity, UnitPrice};
use rust_decimal::dec;

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::for_usage("Arbeit", Quantity::new(dec!(1000), "kWh"), UnitPrice::new(dec!(0.30), "EUR/kWh")).build()?],
    vec![], vec![],
)?;

// 40 / 35 / 25 % split
let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)])?;
let tenant_docs = alloc.allocate(&doc)?;

// The first tenant's line is internally consistent:
let first = &tenant_docs[0].net_positions()[0];
assert_eq!(first.quantity_value(), Some(dec!(400)));
assert_eq!(first.net_amount, Amount::parse("120.00000")?);

// Equal 3-way split
let _equal = EqualAllocation::new(3)?.allocate(&doc)?;

// Exactness guarantees:
let sum: Amount<5> = tenant_docs.iter().map(|d| d.net_total()).sum();
assert_eq!(sum, doc.net_total());            // ✓ exact, no drift
for d in &tenant_docs {
    d.assert_valid();                        // ✓ each doc is consistent
    d.verify_vat_attribution()?;             // ✓ BR-S-08 survives the split
}

// Shares are validated: negative entries are rejected even when they sum to 1.0.
assert!(ProportionalAllocation::new(vec![dec!(1.5), dec!(-0.5)]).is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

#### Why every *position* is split, not every total

Splitting the totals and correcting the positions to match is the obvious
construction, and it quietly breaks **BR-S-08**. The VAT breakdown is split per
`(category, rate)` group while the positions are corrected in aggregate, so a
group's BT-116 and the positions attributed to it can land a minor unit apart. On a
two-rate document that happened for roughly **45 %** of recipients — each document
internally exact, each one rejected by a validator.

So each position is split exactly instead (`Σ(recipients) == the original
position`, line by line), and every total is derived by summing a recipient's own
positions. Each group's taxable base is then read off the very positions BR-S-08
checks it against, so:

> **An allocated share satisfies `verify_vat_attribution` whenever the document it
> came from did.**

That is why there is no "this document came from an allocation, skip the check"
escape hatch: there is nothing left to skip. A property test drives multi-rate
documents through 2–7-way splits and asserts it.

### Raw quantity split (`proportional_split`)

For splits that happen *before* a document exists — e.g. distributing kWh
among tenants, or splitting a capacity block — use `proportional_split`.
It uses the **Largest-Remainder (Hamilton) method**, guaranteeing
`Σ(parts) == total` for every input the share check accepts, with the correction
spread as evenly as the deficit allows — no single entry absorbs it. Where the
fractions sum to exactly one, that means at most one unit of adjustment per
fraction.

```rust
use billing::proportional_split;
use rust_decimal::{Decimal, dec};

let kwh_parts = proportional_split(
    dec!(987.654),
    &[dec!(0.45), dec!(0.35), dec!(0.20)],
    3,   // scale = 3 dp
)?;

let total: Decimal = kwh_parts.iter().sum();
assert_eq!(total, dec!(987.654));  // ✓ exact sum
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 📅 Proration and period merging

```rust
use billing::{prorate, merge_period_documents, RoundingStrategy,
              BillingDocument, DocumentMeta, LineItem, Amount, Currency, Quantity, UnitPrice};
use rust_decimal::dec;

// Prorate scales the QUANTITY as well as the amount, so the line stays honest.
let full = LineItem::for_usage("Arbeit", Quantity::new(dec!(1000), "kWh"), UnitPrice::new(dec!(0.30), "EUR/kWh")).build()?;
let half = prorate(&full, 15, 30, RoundingStrategy::MidpointAwayFromZero)?;
assert_eq!(half.quantity_value(), Some(dec!(500)));
assert_eq!(half.net_amount, Amount::parse("150.00000")?);

// Merge two half-period documents after a mid-month tariff change.
let mk = |amount: &str| BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("x", Amount::parse(amount).unwrap()).build().unwrap()],
    vec![], vec![],
).unwrap();

let merged = merge_period_documents(mk("100.00000"), mk("50.00000"))?;
assert_eq!(merged.net_total(), Amount::parse("150.00000")?);

// Merging across currencies is refused rather than silently summing.
let usd = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::USD, ..Default::default() },
    vec![], vec![], vec![],
)?;
assert!(merge_period_documents(mk("10.00000"), usd).is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

The merged document keeps the **first** document's header; the second's is discarded.

---

## 📄 BillingDocument

`BillingDocument` holds ordered positions and pre-computed totals.
**Sixteen** invariants are enforced **exactly** (zero tolerance) at construction,
and re-checked by `validate()` / `assert_valid()` and on deserialisation:

| Check | Invariant |
|-------|-----------|
| 1 | `Σ(net_positions + discount_positions) == net_total` |
| 2 | `Σ(tax_positions) == tax_total` |
| 3 | `net_total + tax_total == gross_total` |
| 4 | `Σ(discount_positions) == discount_total` |
| 5 | every VAT breakdown entry is category-consistent, its tax matches `base × rate` within BR-CO-17's tolerance, and no `(category, rate)` group appears twice |
| 6–7 | `prepaid >= 0`; `rounding` matches the recorded cash-rounding rule |
| 8–9 | `Σ(tax_breakdown)` is a component of `tax_total`; no discount position opposes the document's own direction |
| 10 | `prepaid` equals `prepayment.total()` |
| 11 | every position satisfies `LineItem::validate`, and no invoice line carries document level allowance/charge detail (BG-20 / BG-21 on a BG-25) |
| 12 | a document that charges VAT has a VAT breakdown, and a declared breakdown has a VAT position behind it (**BR-CO-18**) |
| 13 | a "not subject to VAT" (`O`) breakdown group is the only group (**BR-O-11**) |
| 14 | a "split payment" (`B`) group does not coexist with a "standard rated" (`S`) one (**BR-B-02**) |
| 15 | the invoicing period is ordered (**BR-29**) and every line period lies inside it (`PEPPOL-EN16931-R110` / `R111`) |
| 16 | BT-6 differs from BT-5, and BT-111 shares BT-110's operational sign (`PEPPOL-EN16931-R005` / `R055`) |

**"At construction" means the constructor runs `validate()`**, not merely that it
computes the totals. Computing them establishes checks 1–4 and nothing else — 12, 13
and 14 are properties of the VAT breakdown, and an ordinary pair of tagged tax layers
can violate them — so a constructor that only computed totals could hand back a
document that failed its own `assert_valid()`.

Two identities are deliberately **outside** this list, because `AllocationRule`
cannot preserve them — it splits the positions and the breakdown with independent
penny corrections. Run them on the document you are about to emit:
`verify_vat_attribution()` (**BR-S-08**, **BR-CO-14**, **BR-O-12/13/14** and the
position half of **BR-B-02**) and `unattributed_positions()` (**BR-CO-04** /
**BR-32** / **BR-37**).

```rust
use billing::{BillingDocument, DocumentMeta, LineItem, Amount, Currency,
              FixedRateTax, TaxLayer, minimum_charge};
use rust_decimal::dec;

let positions = vec![LineItem::fixed("Service", Amount::parse("200.00000")?).build()?];
let tax_layers: Vec<Box<dyn TaxLayer>> = vec![FixedRateTax::new("VAT", dec!(0.19))?.boxed()];

let doc = BillingDocument::from_positions(
    DocumentMeta {
        invoice_number: "INV-2026-001".into(),
        period_label:   "2026-06".into(),
        currency:       Currency::EUR,
        ..Default::default()
    },
    positions,
    tax_layers,
    vec![],
)?;

doc.assert_valid();                       // panics on inconsistency
doc.validate()?;                          // ... or handle it as a Result
assert_eq!(doc.gross_total(), Amount::parse("238.00000")?);

# Ok::<(), Box<dyn std::error::Error>>(())
```

### Minimum charges belong before the tax layers

A minimum-spend shortfall is part of the consideration, so it is taxable. Settle it
against the net positions and then build the document, rather than appending it
afterwards:

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let mut positions = vec![LineItem::fixed("Verbrauch", Amount::parse("100.00000")?).build()?];

// 1. Settle the minimum against the untaxed net.
let net_only = BillingDocument::from_positions(
    DocumentMeta::default(), positions.clone(), vec![], vec![])?;
if let Some(shortfall) = minimum_charge(
    &net_only, Amount::parse("110.00000")?, "Mindestentgelt", Currency::EUR)?
{
    positions.push(shortfall);
}

// 2. Build the real document — VAT now applies to the shortfall too.
let taxes: Vec<Box<dyn TaxLayer>> = vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()];
let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    positions, taxes, vec![])?;

assert_eq!(doc.net_total(), Amount::parse("110.00000")?);
assert_eq!(doc.tax_total(), Amount::parse("20.90000")?);   // 110 × 19%, not 100 × 19%

// The shortfall is a *complete* invoice line: BR-22 / BR-23 / BR-26 want BT-129,
// BT-130 and BT-146, and `minimum_charge` supplies all three (1 × the shortfall).
let line = &doc.net_positions()[1];
assert_eq!(line.quantity.as_ref().unwrap().code.as_deref(), Some(UNIT_CODE_ONE));
assert_eq!(line.unit_price.as_ref().unwrap().unit, "EUR");
# Ok::<(), Box<dyn std::error::Error>>(())
```

That is why the currency is an argument: the generated BT-146 label is `"EUR"` /
`"CHF"` / …, and a generated label cannot invent the currency.

`with_extra_position` appends without re-running the tax layers, so it is refused
on any document carrying a VAT breakdown.

### A *Mindestbetrag* is quoted brutto

`minimum_charge` compares against the **net**, which is what a B2B *Mindestumsatz*
is. A consumer supply contract states the other one — *"Mindestbetrag 9,90 € brutto
pro Monat"* — because that is the figure §1 PAngV makes the seller show and the one
the customer agreed to. The two are different arithmetic, and deriving the second
from the first by hand is where the money goes missing.

```rust
use billing::prelude::*;
use billing::{FixedRateTax, TaxCategory};   // `minimum_charge_gross` is in the prelude
use rust_decimal::dec;

let positions = vec![
    LineItem::fixed("Trinkwasser", Amount::parse("2.00000")?).tag("reduced").build()?,
    LineItem::fixed("Grundpreis",  Amount::parse("2.00000")?).tag("full").build()?,
];
let layers = || -> Result<Vec<Box<dyn TaxLayer>>, BillingError> { Ok(vec![
    FixedRateTax::new("MwSt 7 %",  dec!(0.07))?.with_tag("reduced").boxed(),
    FixedRateTax::new("MwSt 19 %", dec!(0.19))?.with_tag("full").boxed(),
])};
let meta = || DocumentMeta { currency: Currency::EUR, ..Default::default() };

// 1. Price the period as it stands — WITH the tax layers, so BT-112 is real.
let provisional = BillingDocument::from_positions(meta(), positions.clone(), layers()?, vec![])?;
assert_eq!(provisional.gross_total(), Amount::parse("4.52000")?);

// 2. Top up to 9,90 € brutto, at the rate the CONTRACT names.
let mut positions = positions;
if let Some(mut top_up) = minimum_charge_gross(
    &provisional,
    Amount::parse("9.90000")?,
    LineVat::new(TaxCategory::Standard, dec!(0.19))?,
    "Mindestbetrag",
    Currency::EUR,
)? {
    top_up.tags.push("full".into());   // so the 19 % layer really covers it
    positions.push(top_up);
}

// 3. Rebuild.
let doc = BillingDocument::from_positions(meta(), positions, layers()?, vec![])?;
assert_eq!(doc.gross_total(), Amount::parse("9.90000")?);
doc.verify_vat_attribution()?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

**The rate cannot be inferred, and it is enforced rather than assumed.** A minimum
charge is a *contractual* charge, so the contract names the rate. A Stadtwerk bill
carrying 7 % Trinkwasser beside 19 % Energie has two, and "use the standard one" is
wrong the moment the top-up lands in the other layer: on the document above, the
by-hand version invoices **9,36 €** against a 9,90 € minimum — 0,54 € short, 5,5 % —
with every total reconciling, the VAT breakdown exact, and `validate()` passing.

The returned line therefore carries the `LineVat` you name, so the claim is checked.
A layer covering it at a *different* rate is a `LayerError` at build time (BR-S-08:
a position belongs to exactly one VAT group); no layer covering it at all is what
`verify_vat_attribution()` reports. A zero rate is the degenerate case and is right
by construction — under reverse charge the divisor is 1, so gross and net coincide.

What is *not* promised: the net top-up is one rounding of an exact quotient, so BT-112
lands within a rounding unit of the target rather than always exactly on it. Assert
`doc.gross_total() >= minimum_gross` if the contract reads as a hard floor.

---

## 📤 E-invoicing: EN 16931, XRechnung, ZUGFeRD

**This crate computes invoices; it does not serialise them.** It deliberately stops
at the semantic model, and there are no plans to add XML or PDF output here — see
[Why not in this crate](#why-not-in-this-crate) below. What it *does* do is make its
documents **representable** in those formats, which is the part that is genuinely
hard and that a serialiser cannot fix afterwards.

### The precision problem

EN 16931 — and with it XRechnung, Peppol BIS and ZUGFeRD/Factur-X — caps **every**
monetary amount at two decimals:

| Rule | Amount |
|------|--------|
| BR-DEC-23 | Invoice line net amount (BT-131) |
| BR-DEC-09 | Sum of line net amounts (BT-106) |
| BR-DEC-12 / 13 / 14 | Total without VAT (BT-109), VAT total (BT-110), total with VAT (BT-112) |
| BR-DEC-16 / 17 / 18 | Paid amount (BT-113), rounding amount (BT-114), amount due (BT-115) |
| BR-DEC-19 / 20 | VAT category taxable base (BT-116) and tax amount (BT-117) |

At the same time the totals identities must hold **exactly at that precision**:
BR-CO-10 (`BT-106 = Σ BT-131`), BR-CO-13, BR-CO-14 (`BT-110 = Σ BT-117`),
BR-CO-15 (`BT-112 = BT-109 + BT-110`), BR-CO-16 and BR-CO-17.

Metered billing produces more than two decimals constantly —
`1234.567 kWh × 0.28901 EUR/kWh = 356.80221` — so something has to round. **It
cannot be the serialiser**, because rounding each amount independently breaks the
identities the same standard checks:

- three positions of `0.005` each round to `0.01`, summing to `0.03`, while the
  exact total `0.015` rounds to `0.02` — **BR-CO-10 violated**;
- a net of `0.0042` at 19 % VAT gives `0.00 + 0.00 ≠ 0.01` — **BR-CO-15 violated**.

Both come from ordinary inputs, and both make a validator reject the invoice.

### The fix: round the leaves, recompute the aggregates

`amount_scale` reduces every *leaf* — each position, each discount- and tax-layer
output, each VAT breakdown entry — before any total is computed. Every total is then
a sum of already-reduced values, so it lands on the same precision exactly and every
identity survives:

```rust
use billing::{BillingDocument, DocumentMeta, LineItem, Amount, Currency, AmountScale,
              FixedRateTax, TaxLayer, Quantity, UnitPrice, RoundingStrategy};
use rust_decimal::dec;

let positions = || vec![
    LineItem::for_usage("Arbeit",
        Quantity::new(dec!(1234.567), "kWh"),
        UnitPrice::new(dec!(0.28901), "EUR/kWh")).build().unwrap(),
];

// Full precision — arithmetically correct, but not emittable as EN 16931.
let raw = BillingDocument::builder().currency(Currency::EUR)
    .positions(positions()).build()?;
assert_eq!(raw.net_total(), Amount::parse("356.80221")?);
assert!(!raw.fits_amount_scale(2));
assert!(raw.amount_scale_violation(2).unwrap().0.contains("position[0]"));

// Two decimals throughout, identities intact.
let doc = BillingDocument::builder().currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(positions())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert!(doc.fits_amount_scale(2));
assert_eq!(doc.net_total(),   Amount::parse("356.80000")?);   // BT-106 / BT-109
assert_eq!(doc.tax_total(),   Amount::parse("67.79000")?);    // BT-110
assert_eq!(doc.gross_total(), Amount::parse("424.59000")?);   // BT-112
assert_eq!(doc.net_total() + doc.tax_total(), doc.gross_total()); // BR-CO-15
doc.assert_valid();
# Ok::<(), Box<dyn std::error::Error>>(())
```

`AmountScale::EN16931` is two decimals with commercial rounding.
`AmountScale::new(0, ..)` handles zero-decimal currencies (JPY, KRW);
`fits_amount_scale` / `amount_scale_violation` are the preconditions to assert
before emitting, and `Amount::exact_to::<2>()` is the conversion that carries an
amount across the boundary — narrowing without rounding, or failing loudly:

```rust
use billing::Amount;

let exact = Amount::<5>::parse("356.80000")?;
assert_eq!(exact.exact_to::<2>()?, Amount::<2>::parse("356.80")?);

// 356.80221 does not fit. Rebuild with `amount_scale` — do not round here, or the
// leaves and the aggregates round independently and BR-CO-10 / BR-CO-15 break.
assert!(Amount::<5>::parse("356.80221")?.exact_to::<2>().is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

**Every amount is rounded exactly once**, from the exact value, at the reporting
precision. That is not a detail — rounding twice is a different operation and lands
a whole minor unit away often enough to matter: `0.004999` rounded to five decimals
is `0.005`, which then rounds to `0.01`, while rounding `0.004999` straight to two
decimals gives `0.00`. So:

- a **VAT category tax** is computed from the *reduced* base in one rounding, which
  is what BR-CO-17 specifies and what a validator recomputes;
- the **charged VAT position** carries that same number rather than being reduced on
  its own, so `BT-110 = Σ BT-117` (**BR-CO-14**) holds by construction;
- a **line derived from `quantity × unit_price`** is reduced from the exact value of
  the whole R120 expression — including the division by BT-149 and the BG-27 / BG-28
  leaves, each of which is reduced once in its own right (BR-DEC-24 … BR-DEC-28) and
  then re-summed, so the emitted parts reproduce the emitted total — rather than
  from the engine's five-decimal intermediate, so
  `BT-131 = BT-129 × (BT-146 / BT-149) + BG-28 − BG-27` holds. EN 16931 itself does
  not check that — there is no such rule in the CEN abstract model — but **Peppol**
  does: `PEPPOL-EN16931-R120`, flagged **fatal**, with a ±0.02 tolerance. Rounding
  once keeps the residual under half a minor unit; rounding twice can exceed 0.02
  and every Peppol access point then rejects the invoice. A position carrying an
  explicit `fixed_amount` is authoritative and is reduced verbatim.

Each of those is a property test rather than a fixture — over randomised
multi-line documents, all five rounding strategies, and VAT rates with up to seven
decimals.

> **Allocation breaks the scale, and cannot preserve it.** Splitting `100.00` three
> ways is `33.333…`. `AllocationRule` keeps the split *exact* — the parts still sum
> to the original — at the cost of precision. Re-check an allocated document with
> `fits_amount_scale` before emitting. `reverse()` (credit notes) preserves it.

### BT-146 is not capped, and rounding it is fatal

The `BR-DEC` decimal limits stop at BT-131 (BR-DEC-23) and the BG-27 / BG-28 leaves
(BR-DEC-24 … BR-DEC-28). **No rule caps a unit price** — not BT-146, BT-147, BT-148
or BT-149 — so a twenty-eight-decimal price is lawful EN 16931, and `AmountScale`
deliberately leaves prices alone while reducing every amount around them.

That asymmetry is load-bearing rather than an oversight, because R120 multiplies the
price by the **quantity** before comparing it to BT-131. A `DynamicPricing` line
states the *weighted average* over its intervals, which is a long non-terminating
decimal by construction — and rounding it "to be tidy" scales the error by the whole
period's consumption:

```rust
use billing::prelude::*;
use rust_decimal::dec;

// A settled spot-price year: 8 760 hourly intervals.
let mut b = DynamicPricing::builder().unit("kWh").currency(Currency::EUR);
for i in 0..8_760 {
    b = b.interval(dec!(1.0), Amount::from_raw_units(1_000 + (i % 7)));
}
let line = b.build()?.calculate()?;

// As built, and after reduction to EN 16931's two decimals, R120 holds.
line.check_price_consistency()?;
let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![line.clone()])
    .build()?;
assert_eq!(doc.price_inconsistent_positions().count(), 0);

// Rounding BT-146 to two decimals moves the R120 expression by 0.26 — thirteen
// times the fatal ±0.02 — while `validate()` still passes, because R120 is a
// profile rule and not an arithmetic invariant.
let mut tidied = line;
tidied.unit_price = Some(tidied.unit_price.take().unwrap()
    .rounded(2, RoundingStrategy::MidpointAwayFromZero));
assert!(tidied.validate().is_ok());
assert!(tidied.check_price_consistency().is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

`LineItem::check_price_consistency()` and `BillingDocument::price_inconsistent_positions()`
are the third member of the *"may I emit this?"* family, beside
`verify_vat_attribution()` and `unattributed_positions()`. They sit outside
`validate()` for the same reason those do: R120's tolerance is **absolute**, so a
document assembled at `AmountScale::new(0, …)` — legitimate for a zero-decimal
currency, or for something that is not an invoice at all — can be perfectly
self-consistent and still fall outside ±0.02.

The same applies to `DynamicPricing`'s own output: **render the average it computed,
do not re-derive it.** `net_amount ÷ quantity` divides an already-rounded total and
gives a different, worse number; the line's `unit_price` is derived from the exact
accumulated sum.

### What is still missing for a valid XRechnung

Precision is the deepest gap but not the only one. To emit a document that passes
the KoSIT validator you also need the following — in your own mapping layer, or by
handing off to [`en16931`](#the-layer-above-en16931), which models all of it:

| Gap | What EN 16931 / XRechnung requires | Status here |
|-----|-----------------------------------|-------------|
| **Parties** | Seller and buyer name, postal address with country code, VAT identifier (BT-31), electronic address (BT-34 / BT-49) | not modelled. `DocumentMeta` *does* carry BT-29 / BT-46 as [`PartyIdentifier`](#-billingdocument), with the ISO 6523 scheme **BR-CL-10** requires |
| **Buyer reference** | BT-10 — the Leitweg-ID, mandatory for German B2G | put it in `meta.labels` |
| **Dates** | BT-2 issue date, BT-9 due date as real dates | `Option<String>`, unparsed by design (no chrono dependency). Period *ordering* is checked: **BR-29**, **BR-30** and `PEPPOL-EN16931-R110` / `R111` are enforced wherever the endpoints are zero-padded ISO 8601, and silently skipped otherwise |
| **Payment means** | BG-16 — BT-81 payment means code, BT-84 IBAN, BT-83 remittance information | not modelled; these are settlement instructions, not amounts. BT-20 payment terms **are** modelled, including the BR-DE-18 Skonto syntax — see [Payment terms and Skonto](#-payment-terms-and-skonto-bt-20) |
| **Line identifiers** | BT-126 per line | positions are ordered, not identified |
| **Notes** | BG-1 — BT-21 subject code (UNCL 4451, **BR-CL-08**) and BT-22 text, `0..n` | modelled as `Vec<Note>`; `PEPPOL-EN16931-R002` caps them at one unless both parties are German, which the engine leaves to the layer that knows the countries |
| **Line terms on a flat charge** | BT-129 / BT-130 / BT-146 on *every* line (BR-22 / BR-23 / BR-26) | `LineItem::fixed` states an amount only — use `LineItem::flat_fee`, which supplies all three. Every line the engine generates itself (`RateSchedule::apply_peak`, `minimum_charge`) already does |
| **Code-list membership** | BT-98 ∈ UNCL 5189, BT-105 ∈ UNCL 7161, BT-121 ∈ CEF VATEX, BT-130 / BT-150 ∈ UN/ECE Rec 20/21 (BR-CL-19 / BR-CL-20 / BR-CL-22 / BR-CL-23), BT-3 ∈ UNTDID 1001 (BR-CL-01) | the fields exist and round-trip; the engine carries no copy of the lists and does not check membership |
| **Suppressing the rate under `O`** | BR-O-05 / BR-O-06 / BR-O-07 — BT-152 / BT-96 / BT-103 must be *absent*, not zero | `rate` is a `Decimal` and stores `0`; `TaxCategory::states_rate()` tells the serialiser to omit the element |
| **Currency** | BT-5 ∈ ISO 4217 (BR-CL-04), and a *real* currency for the invoice to mean anything | two independent questions, two predicates: `is_iso4217()` answers BR-CL-04 (`XXX` **passes** it), `is_unset()` answers "was one configured". Ask both. BT-6 is checked for you: `VatAccountingCurrency` refuses `XXX` outright |

Amounts, the whole totals chain (BT-106 / BT-107 / BT-108 / BT-109 / BT-110 /
BT-112), the VAT breakdown (BG-23) with both exemption-reason forms (BT-120 /
BT-121), per-position VAT category and rate (BT-151 / BT-152, BT-95 / BT-96,
BT-102 / BT-103), the whole of BG-29 PRICE DETAILS (BT-146 / BT-147 / BT-148 /
BT-149 / BT-150), line allowances and charges (BG-27 / BG-28, BT-136 … BT-145),
unit codes (BT-130), allowance and charge reason codes (BT-98 /
BT-105), advance payments, cash rounding (BT-114), prepaid (BT-113), amount due
(BT-115), the VAT accounting currency pair (BT-6 / BT-111), payment terms with
structured Skonto (BT-20 / BR-DE-18) and document type codes (BT-3) are all
modelled here.

### The 2026 revision

CEN approved a revised EN 16931 on 23 October 2025; it passed the formal vote on
13 February 2026 and is published as **EN 16931-1:2026**. It is deliberately *not*
backward compatible: new business terms, changed validation rules, and reworked UBL
and CII bindings. The headline additions are structured **early payment discounts**
and **late-payment charges**, margin schemes, further VAT category codes and
exemption reasons, IBAN fields, triangulation indicators, multiple purchase orders
and corrective-invoice sequencing.

Most of that is *syntax and identity* data, which is why it lands in
[`en16931` / `en16931-formats`](#the-layer-above-en16931) rather than here. What
touches arithmetic, this crate already computes: the
[early-payment discount](#-payment-terms-and-skonto-bt-20) and the
[late-payment charge](#late-payment-interest-the-other-half-of-bt-20) above, and —
for margin schemes and every tax-inclusive quote — the
[gross split](#-tax-inclusive-gross-pricing). Where those numbers are *written*
changes with the revision; what they are does not.

**This crate targets the 2017 edition, and says so deliberately.** No validation
artefact has been published for the 2026 one: the CEN repository's newest tag is
still `validation-1.3.16`, which is the 2017 model. Since every rule cited in this
crate is checked against that artefact rather than against prose, there is at
present nothing to check a 2026 citation *against* — and a citation nobody can
verify is exactly what the pinning exists to prevent. The rules will be re-derived
against the new artefacts when they exist, not before.

### Why not in this crate

**ZUGFeRD is out of scope permanently.** It is a PDF/A-3 container with embedded
CII XML and XMP metadata — that needs a PDF writer, font embedding and subsetting,
and ICC colour profiles. This crate has three dependencies, does no I/O and forbids
`unsafe`; a PDF engine is the opposite of all three.

**XML belongs in a separate crate.** UBL and CII serialisation needs an XML writer,
the UN/ECE Rec 20 and UNTDID code lists, and Schematron conformance fixtures to be
trustworthy — none of which a pricing engine should carry, and all of which move on
a different release cadence (XRechnung 4.0, implementing EN 16931-1:2026, is
expected during 2026). Keeping it out means this crate does not inherit that
cadence.

### The layer above: `en16931`

That separate crate exists. [**`en16931`**](https://github.com/hupe1980/en16931)
carries the EN 16931 semantic model and its 316 business rules as Rust types,
validating the *model* rather than a serialised document — so a finding names
BT-151 on line 3 instead of an XPath. Its companion **`en16931-formats`** does the
serialisation this crate refuses: UBL 2.1 and UN/CEFACT CII in both directions, the
XRechnung CIUS, and ZUGFeRD / Factur-X hybrid PDFs.

The division of labour is the one this README argues for throughout:

| Crate | Answers |
|-------|---------|
| **`billing`** | *What are the amounts?* Pricing models, metering, tax and discount layers, allocation, and every total exactly representable at the scale you will emit |
| [**`en16931`**](https://crates.io/crates/en16931) | *Is this a lawful invoice?* The semantic model and the business rules, checked against the model |
| [**`en16931-formats`**](https://crates.io/crates/en16931-formats) | *How does it go on the wire?* UBL, CII, XRechnung, ZUGFeRD / Factur-X |

They are separate dependencies, not a framework: `billing` has no knowledge of
them and does not depend on them. Use it alone if you only need the arithmetic.
Both are younger than this crate and track a faster-moving target — XRechnung 4.0,
implementing EN 16931-1:2026, is expected during 2026 — which is precisely the
reason they are not merged into it.

What `billing` contributes to that stack is the part that is hard to retrofit:
amounts that are already representable at the target scale. A validator can tell
you `BT-112 ≠ BT-109 + BT-110`; it cannot tell you which rounding decision three
layers ago made them disagree.

---

## 🧬 serde

Enable the `serde` feature for `Serialize`/`Deserialize` on all public types:

```sh
cargo add billing --features serde
```

Two properties matter for a monetary type:

**`Amount<P>` serialises as a decimal string, never as a number.**
A raw scaled integer (`3456`) is meaningless without knowing `P` out of band and
silently rescales by `10^ΔP` if the precision ever changes; a JSON float
reintroduces exactly the imprecision fixed-point arithmetic exists to prevent.

**Types with invariants re-validate on the way in.** Deserialisation reconstructs
private fields directly, which would otherwise bypass every constructor check.
`RateSchedule`, `RateLookup`, `TimeOfUsePricing`, `DynamicPricing`,
`ProportionalAllocation`, `EqualAllocation`, the tax/discount layers, `LineItem`,
`AllowanceCharge`, `UnitPrice` and `BillingDocument` all route through their normal
validation, so untrusted config cannot produce a mispricing value — nor a document
that would be rejected as fatal by a Peppol validator.

```rust,ignore
use billing::{Amount, EqualAllocation, BillingDocument};

// Exact decimal-string representation
assert_eq!(serde_json::to_string(&Amount::<5>::parse("0.03456")?)?, "\"0.03456\"");

// Floats and excess precision are refused
assert!(serde_json::from_str::<Amount<5>>("0.03456").is_err());        // bare number
assert!(serde_json::from_str::<Amount<5>>("\"0.123456\"").is_err());   // 6th digit

// Invariants survive a round-trip through untrusted JSON
assert!(serde_json::from_str::<EqualAllocation>(r#"{"n":0}"#).is_err());

// A document whose stored totals disagree with its positions is rejected
assert!(serde_json::from_str::<BillingDocument>(tampered_json).is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 🔬 Design invariants

| Invariant | How enforced |
|-----------|-------------|
| 🚫 No `f64` in monetary arithmetic | `Amount<P>` is `i64 × 10⁻ᴾ`; all intermediate ops use `rust_decimal` |
| 🔒 Encapsulated representation | `Amount<P>`'s inner `i64` is private; `to_raw()` reads it and `from_raw_units()` reconstructs from it — both explicit, neither implicit |
| 💥 Overflow is visible, never silent | `+`, `-`, `+=`, `-=`, `mul_qty`, `from_int`, `abs` **panic**; every `checked_*` variant returns `Err` and **never panics**, including on `Decimal`'s own overflow. `UsageAggregator::aggregate` is fallible too — raw events are the least trusted input in the pipeline |
| 📐 Rounding is explicit wherever it is observable | `RoundingStrategy` is a required parameter on every reduction. The one default — `quantity × price` at the engine's 5 dp — cannot change an emitted digit, because `AmountScale` rounds each line once from the *exact* product rather than from that value |
| ✋ No silent precision loss in `parse` | `Amount::<5>::parse("1.000011")` returns `Err` — the 6th digit cannot be represented |
| 🔢 Precision is bounded at compile time | `Amount<19>` fails to compile with an explanatory const-eval message |
| 📝 Non-empty descriptions enforced | `LineItem::build()` returns `Err` for empty or whitespace-only descriptions |
| ✅ Documents are self-validating | Every constructor runs `validate()`'s 16 exact invariants (zero tolerance), and so does deserialisation; `verify_vat_attribution` and `unattributed_positions` add the two that need the per-position attribution |
| 🔗 Compound taxes accumulate | Each tax layer sees all prior layers in its base |
| ➗ Allocation is exact | every **position** is split exactly, so `Σ(parts) == total` holds line by line — and `verify_vat_attribution` (BR-S-08) survives the split |
| 🏷️ Engine tag namespace is protected | Caller labels that would collide with a reserved tag (`tax`, `levy`, `discount`, …) are rejected — a band named `tax` would otherwise remove its own consumption from a levy base |
| 🔇 No silent under-billing | Unknown ToU band names, uncovered quantities and non-monotonic schedules are all errors, not skips |
| 🛡️ Invariants survive deserialisation | Validated types re-run their checks via `#[serde(try_from)]` |
| 🧾 Invoices are lawful by construction | Per-rate VAT breakdown (EN 16931 BG-23) with the category rules enforced, not merely documented — including the structural ones (`BR-CO-18`, `BR-O-11`, `BR-B-02`) that balance perfectly and still fail a validator |
| 💵 Rounding concepts stay separate | Tax rounding, currency minor units and cash rounding are three independent settings — conflating them is the classic money bug |
| 🪙 Money splits exactly | `distribute` / `allocate` / `proportional_split` never create or destroy a cent |
| 🧹 Zero domain assumptions | No jurisdiction constants and no default currency or cash increment — the caller supplies both |
| 🏗️ A position's bucket is its EN 16931 group | `net_positions` is BG-25, the other two are BG-20 / BG-21 — an invoice line carrying document level allowance detail is refused, because the amount would land in BT-106 instead of BT-107 with every total still balancing |
| 🚷 No I/O, no async, no `unsafe` | `#![forbid(unsafe_code)]`; every `fn` is a pure `fn`, not `async fn` |

---

## ⚖️ Comparison

| System | Language | Notes |
|--------|----------|-------|
| **Kill Bill** | Java | Full billing *platform*; `billing` is a pure calculation *library* |
| **Lago** | TypeScript | API server with event ingestion; `billing` is a pure Rust library |
| **Stripe Billing** | SaaS API | Payment platform; `billing` is standalone and embeddable |
| **Chargebee / Zuora** | SaaS API | Subscription lifecycle management; out of scope |
| `rust_decimal` | Rust | Low-level decimal arithmetic; no billing abstractions. This crate's one runtime dependency |
| `rusty-money` | Rust | 128-bit money with formatting, exchange and `allocate`. No pricing models, no tax layers, no document model — [`Amount::allocate`](#-splitting-money-exactly) covers the overlap |
| `money2` | Rust | Currency exchange only; no billing engine |
| `use-invoice` | Rust | Invoice vocabulary for same-currency totals; explicitly does *not* calculate tax or implement country rules |
| `einvoice` | Rust | CII / UBL **serialisation** with UniFFI language bindings — the other half of the problem, not this one. It reads and writes the XML; it does not decide what the amounts are |

Not a competitor but a **companion**, by the same author:
[`en16931`](https://crates.io/crates/en16931) (semantic model + business rules) and
[`en16931-formats`](https://crates.io/crates/en16931-formats) (UBL, CII, XRechnung,
ZUGFeRD / Factur-X). `billing` computes the amounts; those two decide whether the
document is lawful and put it on the wire. See
[The layer above](#the-layer-above-en16931).

---

## 🛠️ Development

```sh
cargo install just   # or: brew install just

just ci              # full local CI (fmt → lint → docs → tests → examples)
just test            # unit + doc tests
just test-all        # with --all-features
just test-msrv       # verify Rust 1.85 compatibility
just lint            # cargo clippy -D warnings
just doc             # build & open docs
just examples        # run all four examples
just bench           # criterion benchmarks
just mutants-gate    # full mutation sweep, pass/fail (slow — see below)
just mutants-diff    # mutate only what your branch changed
just release 0.7.0   # create an annotated git tag
```

All available tasks: `just --list`

### How correctness is checked

Four levels, each answering a question the one before it cannot:

| Level | Question it answers |
|-------|---------------------|
| ~760 example-based tests | Does this input produce that output? |
| **Property tests** (`proptest`) | Do the algebraic laws hold for *every* input? Money is conserved by every split, rounding is idempotent and bounded, allocation and reversal preserve every total. |
| Every README example as a doctest | Does the documentation still compile and still say something true? |
| **Mutation testing** (`cargo-mutants`) | Would any test fail if the code were wrong? |

> **`cargo test --all-targets` does not run doctests**
> ([rust-lang/cargo#6669](https://github.com/rust-lang/cargo/issues/6669)), and
> neither does `--lib --tests`. A project whose CI uses only those runs the third
> level above in nobody's pipeline — the guarantee real locally, unenforced remotely.
> `just test-doc` and a matching CI step run them explicitly, on stable, beta and the
> MSRV. Worth checking your own project for the same hole; the flag gives no warning.

The fourth is the one that grades the other three. Coverage reports whether a line
*ran*; mutation testing rewrites the line — flipping `<` to `<=`, replacing a
computed amount with its default, deleting a guard — and reports every rewrite the
suite still accepts. In a billing engine that distinction is the whole game: a
rounding rule, a tolerance bound or a comparison operator can be executed by every
test in the suite and still be silently wrong.

It is not a hypothetical. A `proportional_split` deficit bug was found this way,
under a comment asserting the branch was unreachable. The property test covering that
function conserved the total and passed, because the function returned an error
rather than a wrong answer — and "errors instead of splitting" is not something a
conservation law can see.

It also grades the other three levels. That property test built its fractions to
sum to *exactly* one, which is precisely the region where the bug cannot occur, so it
could never have failed however long it ran; a second drew allocation ratios from
`1..1000`, so a recipient allocated nothing never appeared. A property is only as
strong as the inputs it is allowed to draw, and nothing but a deliberate look tells
you which region it never reaches.

```sh
cargo install cargo-mutants --locked

just mutants-gate           # whole crate, pass/fail — run before a release
just mutants-diff           # only the lines your branch touched — the pre-push check
just mutants-file src/vat.rs   # one file, while writing the tests to kill its survivors
just mutants-iterate        # re-run only what was missed last time
just mutants                # whole crate, report only
```

Configuration lives in [`.cargo/mutants.toml`](.cargo/mutants.toml), which also
records why each setting is what it is.

The figures move with every release, so the number worth remembering is the one on
the right: **every mutant that survives is recorded, with the reason it is
tolerated.** The current baseline holds 13 entries, and the release sweep is expected
to reproduce exactly those — no more, and no fewer.

| | |
|---|---|
| **Recorded survivors** | 13 — see [`.cargo/mutants-baseline.txt`](.cargo/mutants-baseline.txt) |
| **New survivors tolerated** | none; the gate fails on one |
| **Stale entries tolerated** | none; the gate fails on those too |

Run `just mutants-gate` for the current counts. Quoting them here would be a number
that goes stale the next time a function is added, and the property that matters is
not the score — it is that the survivor set is enumerated rather than estimated.

### The gate

`just mutants` reports; **`just mutants-gate` decides.** It runs the sweep and
diffs the survivors against [`.cargo/mutants-baseline.txt`](.cargo/mutants-baseline.txt),
which lists every tolerated survivor *with the reason it is tolerated*. The gate
fails two ways, and both matter:

- a mutant survives that is **not** recorded — a genuine new gap;
- a **recorded** one no longer survives — the entry is stale, and the list should
  shrink.

Line and column numbers are stripped before the comparison, so ordinary edits do
not churn the baseline; only a real change in what survives does.

Nothing is suppressed with `exclude_re`. An exclusion is a permanent blind spot —
the regex that hides `builder -> Default::default()` today also hides it once
`builder()` starts doing real work. Every mutant is still generated, still run and
still reported; the baseline only says which results are expected.

Of the 13 recorded survivors, twelve are **provably equivalent** — `X::builder()`
whose entire body *is* `Default::default()`, or a `<` → `<=` whose extra case
assigns a value already there — and one sits in a branch no input reaches. That last
one was checked against every construction path, including deserialisation, rather
than assumed — an identical "unreachable" comment in `proportional_split` turned out
to be wrong.

### Not a CI gate — an ad-hoc tool

Mutation testing is **not wired into CI**, on purpose. A sweep is ~1 000 mutants; even
after the tuning below it is tens of minutes on a laptop and considerably worse on a
four-core runner. Paying that on every pull request buys a signal that changes rarely,
and waiting for it is the fastest way to make people stop reading it.

So it is run **by hand, when it is worth running** — after writing tests for a tricky
invariant, before a release, or when a survivor is suspected:

```sh
just mutants-gate            # whole crate, pass/fail against the baseline
just mutants-diff            # only what your branch changed — the quick one
just mutants-file src/vat.rs # one file, while writing the tests to kill its survivors
just mutants-shard 1 12      # one slice, if you want to spread it over several runs
```

What CI *does* enforce is everything that is fast and deterministic: formatting,
clippy with `-D warnings`, the full test suite on three platforms, the MSRV build,
doctests, `cargo-deny`, `cargo-semver-checks` and the benchmarks compiling.

### Making a sweep affordable

A sweep pays every per-mutant cost a thousand times, so three settings carry almost
all of the wall clock. Together they roughly halve it, and none of them changes which
mutants are caught:

| Setting | Why |
|---|---|
| `[profile.mutants]` — no debuginfo, no incremental | the crate is compiled ~1 000 times and debugged zero times; incremental state never hits, because every mutant is a different tree |
| `additional_cargo_test_args = ["--lib", "--tests"]` | doctests were **17 of the baseline's 18 test seconds**. They exist to show a reader how a call is written, so they exercise the happy path the unit and integration suites already cover |
| the generated `tests/site_examples.rs` is deleted after use | otherwise every mutant rebuilds and reruns a test binary that checks *documentation* |

Excluding doctests surfaces every function whose **only** test is a doctest: those
mutants survive. That is a finding, not a false positive — a function tested only by
an example is a gap whether or not a sweep is what reveals it — so the answer is a
real test, never a baseline entry.

`just test` and CI still run every doctest on the unmutated tree, which is where a
broken example actually matters.

---

## 📦 Dependencies

| Crate | Role |
|-------|------|
| [`rust_decimal`](https://crates.io/crates/rust_decimal) | Exact base-10 arithmetic (no `f64`) |
| [`thiserror`](https://crates.io/crates/thiserror) | Derive macro for `ParseAmountError` |
| [`serde`](https://crates.io/crates/serde) *(optional)* | `Serialize`/`Deserialize` on all public types |

Total non-optional dependency tree: **2 crates** (`rust_decimal` + `thiserror`).
`dec!` comes from `rust_decimal`'s `macros` feature, declared dev-only, so the
proc-macro does not appear in downstream builds.

---

## 🗂️ Crate structure

```text
src/
├── lib.rs          — re-exports, prelude, crate docs
├── amount.rs       — Amount<P>, RoundingStrategy, AmountScale
├── currency.rs     — Currency (ISO 4217 + minor units)
├── quantity.rs     — Quantity (BT-129/130), UnitPrice (BG-29)
├── line_item.rs    — LineItem, Sign, AllowanceCharge (BG-20/21),
│                     LineAllowanceCharge (BG-27/28)
├── schedule.rs     — RateSchedule (graduated/volume/block/capacity)
├── tou.rs          — TimeOfUsePricing, TimeBand, DynamicPricing
├── aggregation.rs  — UsageAggregator trait + 8 built-in implementations
├── tax.rs          — TaxLayer, DiscountLayer + built-in implementations
├── document.rs     — BillingDocument, BillingDocumentBuilder, DocumentMeta
├── allocation.rs   — AllocationRule, ProportionalAllocation, EqualAllocation
├── period.rs       — Period, merge_period_documents(), prorate(), prorate_amount()
├── minimum.rs      — minimum_charge()
├── lookup.rs       — RateLookup, RateLookupBuilder
├── vat.rs          — TaxCategory, TaxBreakdownEntry (EN 16931 BG-23)
├── advance.rs      — AdvancePayment, DocumentKind, residual_breakdown
├── settlement.rs   — CashRounding (BT-114), VatAccountingCurrency (BT-6/BT-111)
├── terms.rs        — PaymentTerms, EarlyPaymentDiscount (BT-20 / Skonto),
│                     LatePaymentPenalty, DayCountBasis
├── pricing.rs      — PricingModel, Billing (three-way outcome)
└── error.rs        — BillingError, ParseAmountError
```

### Examples

Run with `cargo run --example <name>`, or all four with `just examples`.

| Example | Shows |
|---------|-------|
| `saas_billing` | `PricingModel` with usage, graduated pricing with a free tier, a commission reported **separately** from VAT, the BG-23 breakdown, assembled at `AmountScale::EN16931` |
| `cloud_compute` | Four pricing modes + dynamic intervals at full engine precision, then the same document rebuilt so it is emittable — and what `amount_scale_violation` reports in between |
| `water_utility` | Graduated tiers, `minimum_charge`, and the allocation trade-off: the split stays exact while leaving invoice precision behind |
| `cross_border_retail` | A gross-quoted catalogue converted to net, VAT restated in the accounting currency (BT-6 / BT-111), Skonto in BT-20 — and why the invoice total is a cent off the sum of the shelf prices |

---

## 📜 License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
