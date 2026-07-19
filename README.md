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
| [`TaxLayer`](#-tax-layers--compound-taxes) | Composable, **ordered** tax stack — each layer sees all prior layers in its base |
| [`PerUnitLevy`](#-tax-layers--compound-taxes) | Per-unit excise duty / environmental levy, matched by unit label |
| [`DiscountLayer`](#-discounts) | Percentage and fixed discounts |
| [`PercentageCharge`](#-percentage-charge) | % of invoice total with min/max guard (platform fee, commission) |
| [`AllocationRule`](#-allocation-across-n-recipients) | Exact proportional / equal split of a document — `Σ(parts) == total`, penny-corrected |
| [`proportional_split()`](#raw-quantity-split-proportional_split) | Penny-correct Hamilton split of a raw `Decimal` quantity (kWh, capacity, …) |
| [`BillingDocument`](#-billingdocument) | Self-validating document: eleven invariants checked at build time |
| [`TaxBreakdownEntry`](#-vat-breakdown-en-16931-bg-23) | Per-rate VAT breakdown (EN 16931 BG-23) — **legally required** on any invoice |
| [`TaxCategory`](#-vat-breakdown-en-16931-bg-23) | UNTDID 5305 VAT category (S / Z / E / AE / K / G / O / L / M) |
| [`CashRounding`](#-cash-rounding-and-amount-due) | Rappenrundung / öresavrundning — tender-level rounding (BT-114) |
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
| `LineItem::for_usage_rounded` | `for_usage` with explicit unit-price precision (prevents silent drift) |
| [`minimum_charge()`](#-billingdocument) | Minimum-spend shortfall helper |
| [`merge_period_documents()`](#-proration-and-period-merging) | Merge two half-period documents (tariff change mid-period) |
| [`prorate()`](#-proration-and-period-merging) | Scale a fixed charge to a partial period |

---

## 🚀 Quick start

```toml
# Cargo.toml
[dependencies]
billing = "0.7"

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
    vec![Box::new(FixedRateTax::new("VAT", dec!(0.10))?)],
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
assert_eq!(Amount::<5>::from_decimal(Decimal::MAX), None);

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
    Box::new(FixedRateTax::new("MwSt 19%", dec!(0.19))?.with_tag("standard")),
    Box::new(FixedRateTax::new("MwSt 7%",  dec!(0.07))?.with_tag("reduced")),
];

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    positions, taxes, vec![],
)?;

// One line per (category, rate) — EN 16931 BR-CO-18.
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

```rust
use billing::{FixedRateTax, TaxCategory, TaxLayer};
use rust_decimal::dec;

// §13b UStG reverse charge: 0%, and a reason is mandatory (BR-AE-10).
let _rc = FixedRateTax::new("Reverse charge", dec!(0))?
    .with_category(TaxCategory::ReverseCharge)
    .with_exemption_reason("Steuerschuldnerschaft des Leistungsempfängers (§13b UStG)");

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

Only layers that actually levy VAT contribute here. A `PercentageCharge`
(commission) and a `PerUnitLevy` (excise) return `None` from `TaxLayer::breakdown`
— the excise is part of the VAT *base*, not a VAT. Implement `breakdown` on your
own layer if it levies VAT.

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
    vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)],
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
    vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)],
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
    vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)],
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
    vec![LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh").build()?],
    vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)],
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
# Ok::<(), Box<dyn std::error::Error>>(())
```

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
pricing model in *your* crate:

```rust
use billing::{Tariff, LineItem, Amount, Quantity, UnitPrice,
              TaxLayer, BillingError, DocumentMeta, Currency, FixedRateTax};
use rust_decimal::Decimal;
use rust_decimal::dec;

struct SaasPlan { seats: u32, base_fee: u32 }

impl Tariff for SaasPlan {
    type Usage = ();    // usage is embedded in the struct
    type Error = BillingError;

    fn line_items(&self, _: &()) -> Result<Vec<LineItem>, BillingError> {
        Ok(vec![
            LineItem::fixed("Platform fee", Amount::<5>::from_int(self.base_fee.into()))
                .build()?,
            LineItem::debit("Seats")
                .quantity(Quantity::new(Decimal::from(self.seats), "seats"))
                .unit_price(UnitPrice::new(dec!(19), "EUR/seat"))
                .build()?,
        ])
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        // `new` is fallible; a hardcoded literal rate is one of the few places
        // `expect` is defensible — it cannot fail for a valid constant.
        vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)).expect("0.20 is a valid rate"))]
    }
}

// Build a document in one call:
let doc = SaasPlan { seats: 5, base_fee: 49 }.bill(
    DocumentMeta {
        invoice_number: "INV-001".into(),
        period_label:   "2026-07".into(),
        currency:       Currency::EUR,
        ..Default::default()
    },
    &(),
)?;

// 49 + 5×19 = 144 net, +20% VAT
assert_eq!(doc.net_total(),   Amount::parse("144.00000")?);
assert_eq!(doc.gross_total(), Amount::parse("172.80000")?);
# Ok::<(), Box<dyn std::error::Error>>(())
```

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
    Box::new(PercentageCharge::new("Levy", dec!(0.05))?),
    Box::new(FixedRateTax::new("VAT", dec!(0.19))?),
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

> ⚠️ **Order matters.** Tax layers are applied in declaration order.
> Place levies that form part of the VAT base *before* VAT.

### Per-unit levies

`PerUnitLevy` bills a rate per physical unit rather than a percentage. It sums
quantities from **debit positions whose unit label matches**, so credits
(feed-in, refunds) are correctly excluded from an excise base.

```rust
use billing::{PerUnitLevy, TaxLayer, LineItem, Amount, Currency};
use rust_decimal::dec;

let levy = PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?
    .with_currency(Currency::EUR);

let positions = vec![
    LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh").build()?,
    // A credit position — excluded from the levy base.
    LineItem::credit_for_usage("Einspeisung", dec!(400), "kWh", dec!(0.08), "EUR/kWh").build()?,
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
    Box::new(PercentageDiscount::new("Loyalty -10%", dec!(0.10))?),
    // Fixed 15.00 voucher
    Box::new(FixedDiscount::new("Voucher", Amount::parse("15.00000")?)?),
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
              BillingDocument, DocumentMeta, LineItem, Amount, Currency};
use rust_decimal::dec;

let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh").build()?],
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
              BillingDocument, DocumentMeta, LineItem, Amount, Currency};
use rust_decimal::dec;

// Prorate scales the QUANTITY as well as the amount, so the line stays honest.
let full = LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh").build()?;
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
let tax_layers: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.19))?)];

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
let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)];
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

## 🧬 serde

Enable the `serde` feature for `Serialize`/`Deserialize` on all public types:

```toml
billing = { version = "0.7", features = ["serde"] }
```

Two properties matter for a monetary type:

**`Amount<P>` serialises as a decimal string, never as a number.**
A raw scaled integer (`3456`) is meaningless without knowing `P` out of band and
silently rescales by `10^ΔP` if the precision ever changes; a JSON float
reintroduces exactly the imprecision fixed-point arithmetic exists to prevent.

**Types with invariants re-validate on the way in.** Deserialisation reconstructs
private fields directly, which would otherwise bypass every constructor check.
`TariffSchedule`, `RateLookup`, `TimeOfUsePricing`, `DynamicPricing`,
`ProportionalAllocation`, `EqualAllocation`, the tax/discount layers, and
`BillingDocument` all route through their normal validation, so untrusted config
cannot produce a mispricing value.

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
├── amount.rs       — Amount<P>, RoundingStrategy, EuroAmount, InvoiceAmt
├── currency.rs     — Currency (ISO 4217 + minor units)
├── quantity.rs     — Quantity, UnitPrice
├── line_item.rs    — LineItem, LineItemBuilder, Sign
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
├── tariff.rs       — Tariff trait
└── error.rs        — BillingError, ParseAmountError
```

---

## 📜 License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
