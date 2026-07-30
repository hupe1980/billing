# 🧾 billing

[![Crates.io](https://img.shields.io/crates/v/billing.svg)](https://crates.io/crates/billing)
[![Docs.rs](https://img.shields.io/docsrs/billing)](https://docs.rs/billing)
[![CI](https://github.com/hupe1980/billing/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/billing/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#-license)
[![MSRV](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)

> **A pure, domain-agnostic tariff billing engine.**  
> Zero I/O. No async. No domain assumptions. No `f64` in monetary arithmetic.

`billing` is a calculation *library*, not a platform. It handles the hard
maths — graduated pricing, compound taxes, proportional allocation, exact
rounding — and leaves every domain decision to your crate.

> **Every Rust example in this README is compiled and run as a doctest.**
> If it appears here, it works against the current release.

---

## ✨ Features at a glance

| Primitive | What it does |
|-----------|-------------|
| [`Amount<P>`](#-amountp--fixed-point-arithmetic) | Fixed-point monetary arithmetic with **compile-time precision**. `Amount<5>` = 5 dp, `Amount<2>` = 2 dp. |
| [`Currency`](#-currency) | ISO 4217 code + minor units; used in labels and enforced when combining documents |
| [`TariffSchedule`](#-tariffschedule--four-pricing-modes) | Four modes: **graduated** / **volume** / **block** / **capacity** |
| [`TimeOfUsePricing`](#-time-of-use-and-dynamic-pricing) | N named bands (peak / off-peak / …); caller supplies pre-aggregated consumption |
| [`tags`](#-design-invariants) | The engine's reserved tag namespace, protected against caller collisions |
| [`DynamicPricing`](#-time-of-use-and-dynamic-pricing) | Per-interval price sequence (spot, real-time) |
| [`UsageAggregator<E>`](#-usage-aggregation) | 6 built-in types: SUM · COUNT · UNIQUE_COUNT · MAX · LATEST · WEIGHTED_SUM |
| [`Tariff`](#-implementing-tariff) | Primary extension point for usage-driven pricing |
| [`ScalarTariff`](#pre-computed--scalartariff) | Same, for settlements whose figures are already computed — no `Usage` type |
| [`Billing<T, R>`](#-three-outcomes-billable-not-billable-error) | Three outcomes: billable, **not billable (with your reason)**, error |
| [`TaxLayer`](#-tax-layers--compound-taxes) | Composable, **ordered** tax stack — each layer sees all prior layers in its base |
| [`PerUnitLevy`](#-tax-layers--compound-taxes) | Per-unit excise duty / environmental levy, matched by unit label |
| [`DiscountLayer`](#-discounts) | Percentage and fixed discounts |
| [`PercentageCharge`](#-percentage-charge) | % of invoice total with min/max guard (platform fee, commission) |
| [`AllocationRule`](#-allocation-across-n-recipients) | Exact proportional / equal split of a document — `Σ(parts) == total`, penny-corrected |
| [`proportional_split()`](#raw-quantity-split-proportional_split) | Penny-correct Hamilton split of a raw `Decimal` quantity (kWh, capacity, …) |
| [`BillingDocument`](#-billingdocument) | Self-validating document: thirteen invariants checked at build time |
| [`BillingDocument::taxable_total`](#the-totals-chain--bt-106--bt-107--bt-108--bt-109) | **BT-109** via BR-CO-13 — `net_total` is *not* BT-109 once a levy exists |
| [`TaxBreakdownEntry`](#-vat-breakdown-en-16931-bg-23) | Per-rate VAT breakdown (EN 16931 BG-23) — **legally required** on any invoice |
| [`TaxCategory`](#-vat-breakdown-en-16931-bg-23) | UNCL 5305 VAT category — all ten BR-CL-17 permits (S / Z / E / AE / K / G / O / L / M / **B**) |
| [`LineVat`](#per-position-vat-attribution) | Per-position VAT category + rate (BT-151/152, BT-95/96, BT-102/103) |
| [`AllowanceCharge`](#per-position-vat-attribution) | Allowance/charge detail: reason code + base + percentage (BT-93/94/98, BT-100/101/105) |
| [`LineAllowanceCharge`](#-line-allowances-and-charges-en-16931-bg-27--bg-28) | **BG-27 / BG-28** line allowance / charge (BT-136 … BT-145) — moves BT-131, not the document totals |
| [`DocumentKind::is_credit_note`](#-credit-notes) | Which document element to emit — `BR-CL-01` polices two element-selected code lists |
| [`DocumentKind::is_peppol_billing_code`](#the-type-code-is-not-interchangeable) | Whether Peppol BIS Billing accepts BT-3 — its lists are narrower than `BR-CL-01`'s |
| [`BillingDocument::vat_total`](#value-added-tax-vs-document-level-charges) | **BT-110** alone — separates VAT from document level charges (BG-21) |
| [`BillingDocument::verify_vat_attribution`](#per-position-vat-attribution) | Checks the breakdown against the per-position attribution (**BR-S-08**) |
| [`FixedRateTax::exempt`](#categories-and-exemptions) | Zero-tax layer with a validated category + mandatory exemption reason |
| [`Amount::exact_to`](#-amountp--fixed-point-arithmetic) | Narrow precision **without rounding**, or fail — for interchange boundaries |
| [`CashRounding`](#-cash-rounding-and-amount-due) | Rappenrundung / öresavrundning — tender-level rounding (BT-114) |
| [`AmountScale`](#-e-invoicing-en-16931-xrechnung-zugferd) | Assemble every amount at an interchange format's decimal limit (EN 16931: 2) |
| [`BillingDocument::reverse`](#-credit-notes) | Credit note / Storno — negates an entire document |
| [`AdvancePayment`](#-advance-payments-and-final-invoices) | An advance already invoiced and paid, **with the tax it contains** |
| [`Prepayment`](#-advance-payments-and-final-invoices) | What has been paid so far — a flat total or itemised advances, never both |
| [`DocumentKind`](#-advance-payments-and-final-invoices) | UNTDID 1001 document type code (BT-3) |
| [`Amount::distribute`](#-splitting-money-exactly) | Split an amount N ways with no cent created or lost |
| [`Amount::allocate`](#-splitting-money-exactly) | Split by integer ratios, largest-remainder, exact |
| [`RateLookup`](https://docs.rs/billing/latest/billing/lookup/struct.RateLookup.html) | Capacity-based rate table — `at_most(kWp, rate)` + `fallback(rate)` |
| [`DocumentMeta.labels`](https://docs.rs/billing/latest/billing/document/struct.DocumentMeta.html) | Key-value domain annotation bag (`malo_id`, `billing_year`, …) |
| [`LineItem::scaled`](#-proration-and-period-merging) | Scale a position, keeping `quantity × unit_price == net_amount` consistent |
| `LineItem::credit_for_usage` | Symmetric credit counterpart of `for_usage` (feed-in, refunds) |
| [`LineItem::flat_fee`](#a-flat-charge-is-still-an-invoice-line) | A standing charge as a **complete** EN 16931 line — satisfies BR-22 / BR-23 / BR-26, which `fixed` cannot |
| [`UnitPrice::per`](#-price-details-en-16931-bg-29) | **BT-149 / BT-150** — "EUR 12,00 per 100 pieces", the divisor in `PEPPOL-EN16931-R120` |
| [`UnitPrice::discounted`](#-price-details-en-16931-bg-29) | **BT-147 / BT-148** — gross price less a price discount, deriving BT-146 (`R046`) |
| `UnitPrice::rounded` | Pin a derived unit price to a scale with an explicit strategy |
| [`minimum_charge()`](#-billingdocument) | Minimum-spend shortfall helper |
| [`merge_period_documents()`](#-proration-and-period-merging) | Merge two half-period documents (tariff change mid-period) |
| [`prorate()`](#-proration-and-period-merging) | Scale a fixed charge to a partial period |

---

## 🚀 Quick start

```toml
# Cargo.toml
[dependencies]
billing = "0.10"

[dev-dependencies]
# `dec!` lives in rust_decimal itself behind the `macros` feature.
rust_decimal = { version = "1.42", features = ["macros"] }
```

```rust
use billing::prelude::*;
use rust_decimal::dec;

// Three-tier water tariff (m³)
let schedule = TariffSchedule::graduated()
    .unit("m³")                  // ← domain unit
    .currency(Currency::EUR)     // ← no currency is ever assumed
    .band(TariffBand::up_to(dec!(5),   Amount::parse("0.80000")?))
    .band(TariffBand::between(dec!(5), dec!(20), Amount::parse("1.40000")?))
    .band(TariffBand::over(dec!(20),   Amount::parse("2.60000")?))
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

Or use the fluent builder with the [`Tariff`](#-implementing-tariff) trait:

```rust,ignore
let doc = BillingDocument::builder()
    .meta(meta)
    .tariff(&my_tariff, &usage)?  // loads positions + tax/discount layers
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
let _: billing::EuroAmount = Amount::<5>::ZERO;  // 5 dp
let _: billing::InvoiceAmt = Amount::<2>::ZERO;  // 2 dp
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
use billing::{Currency, TariffSchedule, TariffBand, Amount};
use rust_decimal::dec;

let usd = TariffSchedule::graduated()
    .unit("GB")
    .currency(Currency::USD)
    .band(TariffBand::over(dec!(0), Amount::parse("0.10000")?))
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

| Accessor | EN 16931 |
|---|---|
| `line_total()` | **BT-106** |
| `discount_total()` | **−BT-107** (EN 16931 states it as a positive magnitude) |
| `charge_total()` | **BT-108** |
| `net_total()` | BT-106 − BT-107 — *not a BT of its own* |
| `taxable_total()` | **BT-109** |
| `vat_total()` | **BT-110** |
| `gross_total()` | **BT-112** |

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

assert_eq!(doc.net_total(),      Amount::parse("300.00000")?);  // BT-106 − BT-107
assert_eq!(doc.charge_total()?,  Amount::parse("20.50000")?);   // BT-108
assert_eq!(doc.taxable_total()?, Amount::parse("320.50000")?);  // BT-109 — includes the levy

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

`verify_vat_attribution` is deliberately *not* part of `validate()`:
`AllocationRule` cannot preserve it exactly, because it splits the positions and
the breakdown with independent penny corrections. Run it on the document you are
about to emit.

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

let fee = LineItem::flat_fee("Grundpreis", Amount::parse("8.50000")?).build()?;

assert_eq!(fee.net_amount, Amount::<5>::parse("8.50000")?);
assert_eq!(fee.quantity.as_ref().unwrap().value, dec!(1));                 // BT-129
assert_eq!(fee.quantity.as_ref().unwrap().code.as_deref(),
           Some(UNIT_CODE_ONE));                                           // BT-130 = C62
assert_eq!(fee.unit_price.as_ref().unwrap().value, dec!(8.5));             // BT-146
# Ok::<(), Box<dyn std::error::Error>>(())
```

`UNIT_CODE_ONE` is `C62`, UN/ECE Rec 20 for *one* — the code for a countable item
with no other unit. `1 × 8,50` is exactly `8,50`, so `R120` holds trivially and
nothing is rounded that was not rounded before.

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

## 📊 `TariffSchedule` — four pricing modes

```rust
use billing::{TariffSchedule, TariffBand, Amount, Currency};
use rust_decimal::dec;

// ── Mode 1: Graduated — each tier at its own price ──────────────────────────
let graduated = TariffSchedule::graduated()
    .unit("kWh").currency(Currency::EUR)
    .band(TariffBand::up_to(dec!(500), Amount::parse("0.32000")?))
    .band(TariffBand::over(dec!(500),  Amount::parse("0.28000")?))
    .build()?;
assert_eq!(graduated.split(dec!(1234.5))?.len(), 2);  // 500 × 0.32, then 734.5 × 0.28

// ── Mode 2: Volume — ALL units at the top tier reached ──────────────────────
let volume = TariffSchedule::volume()
    .unit("kWh").currency(Currency::EUR)
    .band(TariffBand::up_to(dec!(1000), Amount::parse("0.32000")?))
    .band(TariffBand::over(dec!(1000),  Amount::parse("0.28000")?))
    .build()?;
let v = volume.split(dec!(1234.5))?;
assert_eq!(v[0].net_amount, Amount::parse("345.66000")?);  // 1234.5 × 0.28

// ── Mode 3: Block — per N-unit block, rounded UP ────────────────────────────
// Use case: parking (30-min slots), telephony, data packs
let block = TariffSchedule::block()
    .unit("GB").currency(Currency::EUR)
    .band(TariffBand::block(dec!(10), Amount::parse("1.50000")?))
    .build()?;
let b = block.split(dec!(35))?;
assert_eq!(b[0].net_amount, Amount::parse("6.00000")?);    // 4 blocks × 1.50

// ── Mode 4: Capacity — bill on PEAK value, not cumulative sum ───────────────
// Use case: demand charge (peak kW), bandwidth (max Mbps), concurrent seats
let capacity = TariffSchedule::capacity()
    .unit("Mbps").currency(Currency::EUR)
    .band(TariffBand::up_to(dec!(100), Amount::parse("50.00000")?))
    .band(TariffBand::over(dec!(100),  Amount::parse("100.00000")?))
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
use billing::{TariffSchedule, TariffBand, Amount};
use rust_decimal::dec;

// Descending bounds are rejected up front rather than mispricing later.
assert!(TariffSchedule::graduated()
    .band(TariffBand::up_to(dec!(100), Amount::parse("1.00000")?))
    .band(TariffBand::up_to(dec!(50),  Amount::parse("2.00000")?))
    .build()
    .is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 🕐 Time-of-use and dynamic pricing

```rust
use billing::{TimeOfUsePricing, TouBand, DynamicPricing, Amount, Currency};
use rust_decimal::dec;

// N-band ToU: caller supplies pre-aggregated consumption per band name.
// The engine has zero knowledge of time zones or grid schedules.
let tou = TimeOfUsePricing::builder()
    .unit("kWh")
    .currency(Currency::EUR)
    .band(TouBand::new("peak", Amount::parse("0.32000")?))
    .band(TouBand::new("off-peak", Amount::parse("0.18000")?))
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

---

## 🔢 Usage aggregation

Before applying a tariff, aggregate raw events to a scalar:

```rust
use billing::{SumAggregator, CountAggregator, UniqueCountAggregator,
              MaxAggregator, LatestAggregator, WeightedSumAggregator,
              UsageAggregator};
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
    SumAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events),
    dec!(600)
);

// COUNT: number of API requests
assert_eq!(CountAggregator.aggregate(&events), dec!(3));

// UNIQUE_COUNT: unique active tenants. The key can be ANY Hash + Eq type.
// A Copy key such as u64 is zero-allocation — no String per event.
assert_eq!(
    UniqueCountAggregator::new(|e: &ApiCall| e.tenant_id).aggregate(&events),
    dec!(2)
);
// An owned key works too, at the cost of one clone per event.
assert_eq!(
    UniqueCountAggregator::new(|e: &ApiCall| e.user_id.clone()).aggregate(&events),
    dec!(2)
);
// NOTE: the key type may not BORROW from the event — `|e| e.user_id.as_str()`
// does not compile, because the key type is fixed independently of the
// event's lifetime. Use a Copy key, an owned key, or hold a `&'a str` in the
// event struct itself (see the UniqueCountAggregator docs).

// MAX: peak value → pair with TariffSchedule::capacity()
assert_eq!(
    MaxAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events),
    dec!(300)
);

// LATEST: end-of-period snapshot (last element in slice order)
assert_eq!(
    LatestAggregator::new(|e: &ApiCall| Decimal::from(e.bytes)).aggregate(&events),
    dec!(300)
);

// WEIGHTED_SUM: VM CPU-hours for VMs active only part of the period
assert_eq!(
    WeightedSumAggregator::new(
        |e: &VmEvent| Decimal::from(e.vcpus),
        |e: &VmEvent| e.uptime_fraction,
    ).aggregate(&vms),
    dec!(2.0)
);
```

---

## 🏗️ Implementing `Tariff`

The `Tariff` trait is the primary extension point. Implement it once per
pricing model in *your* crate.

There are two shapes, and picking the right one costs you nothing:

| Trait | Use when | You write |
|-------|----------|-----------|
| `Tariff` | pricing consumes usage data | `type Usage = YourUsage` + `line_items(&self, usage)` |
| `ScalarTariff` | the figures are already computed | `positions(&self)` — no `Usage`, no ignored argument |

### Usage-driven — `Tariff`

```rust
use billing::{Tariff, Positions, LineItem, Amount, Quantity, UnitPrice,
              TaxLayer, BillingError, DocumentMeta, Currency, FixedRateTax};
use rust_decimal::Decimal;
use rust_decimal::dec;
use std::convert::Infallible;

struct SaasPlan { base_fee: u32 }
struct Seats { count: u32 }

impl Tariff for SaasPlan {
    type Usage = Seats;
    type Error = BillingError;
    // This plan always produces an invoice, so "nothing to bill" is uninhabited
    // and `.bill()` hands back a document with no extra matching.
    type NotBillable = Infallible;

    fn line_items(&self, usage: &Seats) -> Result<Positions<Infallible>, BillingError> {
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

### Pre-computed — `ScalarTariff`

Plenty of settlements are not usage-driven: a subsidy payout, a redispatch
compensation, an EEG or KWKG settlement whose figures were determined upstream.
Those get `ScalarTariff` — no `type Usage = ()`, and no `usage` parameter to
accept and ignore. A blanket impl still makes it a full `Tariff`, so it composes
with `BillingDocumentBuilder` and anything else generic over `Tariff`:

```rust
use billing::{ScalarTariff, Positions, LineItem, Amount, DocumentMeta, Currency, BillingError};
use std::convert::Infallible;

struct EegSettlement { payout_eur: i64 }

impl ScalarTariff for EegSettlement {
    type Error = BillingError;
    type NotBillable = Infallible;

    fn positions(&self) -> Result<Positions<Infallible>, BillingError> {
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

Implement **either** `Tariff` or `ScalarTariff` for a given type, never both —
the blanket impl makes that a coherence error rather than a silent ambiguity.

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

So `line_items` returns `Positions<Self::NotBillable>` — an alias for
`Billing<Vec<LineItem>, R>` — and `R` is *your* reason type, matched exhaustively:

```rust
use billing::{Tariff, Billing, Positions, Billed, LineItem, Amount,
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

impl Tariff for Settlement {
    type Usage = Readings;
    type Error = BillingError;
    type NotBillable = NotYet;

    fn line_items(&self, usage: &Readings) -> Result<Positions<NotYet>, BillingError> {
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

**Tariffs that always bill pay nothing for this.** Set
`type NotBillable = Infallible` and the `NotBillable` variant is uninhabited — the
compiler knows the outcome, and you keep the two-outcome API:

| Can decline to bill | `Tariff` | `BillingDocumentBuilder` |
|---------------------|----------|--------------------------|
| No (`NotBillable = Infallible`) | `.bill(meta, usage) -> Result<BillingDocument, _>` | `.tariff(&t, &u)` |
| Yes | `.try_bill(meta, usage) -> Result<Billed<R>, _>` | `.try_tariff(&t, &u)` |

The bound is what makes it a compile-time distinction: a tariff that can decline
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
`tax_layers()`. There is no need for one tariff type, wrapper struct or type alias
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
> `"percentage-charge"` tag if you need to present commissions separately.

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

// Penny correction guarantees:
let sum: Amount<5> = tenant_docs.iter().map(|d| d.net_total()).sum();
assert_eq!(sum, doc.net_total());            // ✓ exact, no drift
for d in &tenant_docs { d.assert_valid(); }  // ✓ each doc is consistent

// Shares are validated: negative entries are rejected even when they sum to 1.0.
assert!(ProportionalAllocation::new(vec![dec!(1.5), dec!(-0.5)]).is_err());
# Ok::<(), Box<dyn std::error::Error>>(())
```

### Raw quantity split (`proportional_split`)

For splits that happen *before* a document exists — e.g. distributing kWh
among tenants, or splitting a capacity block — use `proportional_split`.
It uses the **Largest-Remainder (Hamilton) method**, guaranteeing
`Σ(parts) == total` with at most one unit of adjustment per fraction
(no single entry absorbs the full deficit).

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
Eleven invariants are enforced **exactly** (zero tolerance) at construction, and
re-checked by `validate()` / `assert_valid()`:

| Check | Invariant |
|-------|-----------|
| 1 | `Σ(net_positions + discount_positions) == net_total` |
| 2 | `Σ(tax_positions) == tax_total` |
| 3 | `net_total + tax_total == gross_total` |
| 4 | `Σ(discount_positions) == discount_total` |
| 5 | every VAT breakdown entry is category-consistent, one line per `(category, rate)` |
| 6–7 | `prepaid >= 0`; `rounding` matches the recorded cash-rounding rule |
| 8–9 | `Σ(tax_breakdown)` is a component of `tax_total`; no discount position is positive |
| 10 | `prepaid` equals the combined gross of the itemised advances |
| 11 | every position satisfies `LineItem::validate` |

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
if let Some(shortfall) =
    minimum_charge(&net_only, Amount::parse("110.00000")?, "Mindestentgelt")?
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
# Ok::<(), Box<dyn std::error::Error>>(())
```

`with_extra_position` appends without re-running the tax layers, so it is refused
on any document carrying a VAT breakdown.

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

This is verified over ~24 000 boundary cases and 6 000 randomised multi-line
documents, across all five rounding strategies and VAT rates with up to seven
decimals.

> **Allocation breaks the scale, and cannot preserve it.** Splitting `100.00` three
> ways is `33.333…`. `AllocationRule` keeps the split *exact* — the parts still sum
> to the original — at the cost of precision. Re-check an allocated document with
> `fits_amount_scale` before emitting. `reverse()` (credit notes) preserves it.

### What is still missing for a valid XRechnung

Precision is the deepest gap but not the only one. To emit a document that passes
the KoSIT validator you also need, in your own mapping layer:

| Gap | What EN 16931 / XRechnung requires | Status here |
|-----|-----------------------------------|-------------|
| **Parties** | Seller and buyer name, postal address with country code, VAT identifier (BT-31), electronic address (BT-34 / BT-49) | `DocumentMeta` carries opaque `issuer_id` / `recipient_id` only |
| **Buyer reference** | BT-10 — the Leitweg-ID, mandatory for German B2G | put it in `meta.labels` |
| **Dates** | BT-2 issue date, BT-9 due date as real dates | `Option<String>`, unparsed by design (no chrono dependency). `Period::is_ordered` checks BR-29 / BR-30 for ISO 8601 strings |
| **Line identifiers** | BT-126 per line | positions are ordered, not identified |
| **Line terms on a flat charge** | BT-129 / BT-130 / BT-146 on *every* line (BR-22 / BR-23 / BR-26) | `LineItem::fixed` states an amount only — use `LineItem::flat_fee`, which supplies all three |
| **Code-list membership** | BT-98 ∈ UNCL 5189, BT-105 ∈ UNCL 7161, BT-121 ∈ CEF VATEX, BT-130 / BT-150 ∈ UN/ECE Rec 20/21 (BR-CL-19 / BR-CL-20 / BR-CL-22 / BR-CL-23), BT-3 ∈ UNTDID 1001 (BR-CL-01) | the fields exist and round-trip; the engine carries no copy of the lists and does not check membership |
| **Suppressing the rate under `O`** | BR-O-05 / BR-O-06 / BR-O-07 — BT-152 / BT-96 / BT-103 must be *absent*, not zero | `rate` is a `Decimal` and stores `0`; `TaxCategory::states_rate()` tells the serialiser to omit the element |
| **Currency** | BT-5 must be a real currency for the invoice to mean anything | `Currency::XXX` **passes** BR-CL-04 — it is a valid ISO 4217 code. Reject it yourself with `is_unset()` |

Amounts, the whole totals chain (BT-106 / BT-107 / BT-108 / BT-109 / BT-110 /
BT-112), the VAT breakdown (BG-23) with both exemption-reason forms (BT-120 /
BT-121), per-position VAT category and rate (BT-151 / BT-152, BT-95 / BT-96,
BT-102 / BT-103), the whole of BG-29 PRICE DETAILS (BT-146 / BT-147 / BT-148 /
BT-149 / BT-150), line allowances and charges (BG-27 / BG-28, BT-136 … BT-145),
unit codes (BT-130), allowance and charge reason codes (BT-98 /
BT-105), advance payments, cash rounding (BT-114), prepaid (BT-113), amount due
(BT-115) and document type codes (BT-3) are all modelled here.

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
cadence. Build it on top: this crate gives you amounts that are already
representable, which is the part that is hard to get right.

---

## 🧬 serde

Enable the `serde` feature for `Serialize`/`Deserialize` on all public types:

```toml
billing = { version = "0.10", features = ["serde"] }
```

Two properties matter for a monetary type:

**`Amount<P>` serialises as a decimal string, never as a number.**
A raw scaled integer (`3456`) is meaningless without knowing `P` out of band and
silently rescales by `10^ΔP` if the precision ever changes; a JSON float
reintroduces exactly the imprecision fixed-point arithmetic exists to prevent.

**Types with invariants re-validate on the way in.** Deserialisation reconstructs
private fields directly, which would otherwise bypass every constructor check.
`TariffSchedule`, `RateLookup`, `TimeOfUsePricing`, `DynamicPricing`,
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
| 💥 Overflow is visible, never silent | `+`, `-`, `+=`, `-=`, `mul_qty`, `from_int`, `abs` **panic**; every `checked_*` variant returns `Err` and **never panics**, including on `Decimal`'s own overflow |
| 📐 Rounding is always explicit | `RoundingStrategy` is a required parameter; no implicit `round()` anywhere |
| ✋ No silent precision loss in `parse` | `Amount::<5>::parse("1.000011")` returns `Err` — the 6th digit cannot be represented |
| 🔢 Precision is bounded at compile time | `Amount<19>` fails to compile with an explanatory const-eval message |
| 📝 Non-empty descriptions enforced | `LineItem::build()` returns `Err` for empty or whitespace-only descriptions |
| ✅ Documents are self-validating | `assert_valid()` checks 11 exact invariants (zero tolerance) |
| 🔗 Compound taxes accumulate | Each tax layer sees all prior layers in its base |
| ➗ Allocation is exact | `Σ(parts) == total` with per-document penny correction; scaled lines keep `quantity × price == net` |
| 🏷️ Engine tag namespace is protected | Caller labels that would collide with a reserved tag (`tax`, `levy`, `discount`, …) are rejected — a band named `tax` would otherwise remove its own consumption from a levy base |
| 🔇 No silent under-billing | Unknown ToU band names, uncovered quantities and non-monotonic schedules are all errors, not skips |
| 🛡️ Invariants survive deserialisation | Validated types re-run their checks via `#[serde(try_from)]` |
| 🧾 Invoices are lawful by construction | Per-rate VAT breakdown (EN 16931 BG-23) with the category rules enforced, not merely documented |
| 💵 Rounding concepts stay separate | Tax rounding, currency minor units and cash rounding are three independent settings — conflating them is the classic money bug |
| 🪙 Money splits exactly | `distribute` / `allocate` / `proportional_split` never create or destroy a cent |
| 🧹 Zero domain assumptions | No jurisdiction constants and no default currency or cash increment — the caller supplies both |
| 🚷 No I/O, no async, no `unsafe` | `#![forbid(unsafe_code)]`; every `fn` is a pure `fn`, not `async fn` |

---

## ⚖️ Comparison

| System | Language | Notes |
|--------|----------|-------|
| **Kill Bill** | Java | Full billing *platform*; `billing` is a pure calculation *library* |
| **Lago** | TypeScript | API server with event ingestion; `billing` is a pure Rust library |
| **Stripe Billing** | SaaS API | Payment platform; `billing` is standalone and embeddable |
| **Chargebee / Zuora** | SaaS API | Subscription lifecycle management; out of scope |
| `rust_decimal` | Rust | Low-level decimal arithmetic; no billing abstractions |
| `money2` | Rust | Currency exchange only; no billing engine |
| `use-invoice` | Rust | Basic invoice primitives; no tariff calculation |

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
just examples        # run all three examples
just bench           # criterion benchmarks
just release 0.7.0   # create an annotated git tag
```

Correctness is covered at three levels: ~400 example-based tests, **property-based
tests** (`proptest`) asserting the algebraic laws — money is conserved by every
split, rounding is idempotent and bounded, allocation and reversal preserve every
total — and every README example compiled as a doctest.

All available tasks: `just --list`

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
├── amount.rs       — Amount<P>, RoundingStrategy, AmountScale, EuroAmount, InvoiceAmt
├── currency.rs     — Currency (ISO 4217 + minor units)
├── quantity.rs     — Quantity (BT-129/130), UnitPrice (BG-29)
├── line_item.rs    — LineItem, Sign, AllowanceCharge (BG-20/21),
│                     LineAllowanceCharge (BG-27/28)
├── schedule.rs     — TariffSchedule (graduated/volume/block/capacity)
├── tou.rs          — TimeOfUsePricing, TouBand, DynamicPricing
├── aggregation.rs  — UsageAggregator trait + 6 built-in implementations
├── tax.rs          — TaxLayer, DiscountLayer + built-in implementations
├── document.rs     — BillingDocument, BillingDocumentBuilder, DocumentMeta
├── allocation.rs   — AllocationRule, ProportionalAllocation, EqualAllocation
├── period.rs       — Period, merge_period_documents(), prorate(), prorate_amount()
├── minimum.rs      — minimum_charge()
├── lookup.rs       — RateLookup, RateLookupBuilder
├── vat.rs          — TaxCategory, TaxBreakdownEntry (EN 16931 BG-23)
├── advance.rs      — AdvancePayment, DocumentKind, residual_breakdown
├── settlement.rs   — CashRounding (BT-114)
├── tariff.rs       — Tariff, ScalarTariff, Billing (three-way outcome)
└── error.rs        — BillingError, ParseAmountError
```

### Examples

Run with `cargo run --example <name>`, or all three with `just examples`.

| Example | Shows |
|---------|-------|
| `saas_billing` | `Tariff` with usage, graduated pricing with a free tier, a commission reported **separately** from VAT, the BG-23 breakdown, assembled at `AmountScale::EN16931` |
| `cloud_compute` | Four pricing modes + dynamic intervals at full engine precision, then the same document rebuilt so it is emittable — and what `amount_scale_violation` reports in between |
| `water_utility` | Graduated tiers, `minimum_charge`, and the allocation trade-off: the split stays exact while leaving invoice precision behind |

---

## 📜 License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
