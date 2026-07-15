# 🧾 billing

[![Crates.io](https://img.shields.io/crates/v/billing.svg)](https://crates.io/crates/billing)
[![Docs.rs](https://img.shields.io/docsrs/billing)](https://docs.rs/billing)
[![CI](https://github.com/hupe1980/billing/actions/workflows/ci.yml/badge.svg)](https://github.com/hupe1980/billing/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![MSRV](https://img.shields.io/badge/rustc-1.85+-orange.svg)](https://blog.rust-lang.org/2025/02/20/Rust-1.85.0.html)

> **A pure, domain-agnostic tariff billing engine.**  
> Zero I/O. No async. No domain assumptions. No `f64` in monetary arithmetic.

`billing` is a calculation *library*, not a platform. It handles the hard
maths — graduated pricing, compound taxes, proportional allocation, exact
rounding — and leaves every domain decision to your crate.

---

## ✨ Features at a glance

| Primitive | What it does |
|-----------|-------------|
| [`Amount<P>`](#-amountp--fixed-point-arithmetic) | Fixed-point monetary arithmetic with **compile-time precision**. `Amount<5>` = 5 dp, `Amount<2>` = 2 dp. |
| [`TariffSchedule`](#-tariffschedule--four-pricing-modes) | Four modes: **graduated** / **volume** / **block** / **capacity** |
| [`TimeOfUsePricing`](#-time-of-use-and-dynamic-pricing) | N named bands (peak / off-peak / …); caller supplies pre-aggregated consumption |
| [`DynamicPricing`](#-time-of-use-and-dynamic-pricing) | Per-interval price sequence (spot, real-time) |
| [`UsageAggregator<E>`](#-usage-aggregation) | 6 built-in types: SUM · COUNT · UNIQUE_COUNT · MAX · LATEST · WEIGHTED_SUM |
| [`TaxLayer`](#-tax-layers--compound-taxes) | Composable, **ordered** tax stack — each layer sees all prior layers in its base |
| [`DiscountLayer`](#-discounts) | Percentage and fixed discounts |
| [`PercentageCharge`](#-percentage-charge) | % of invoice total with min/max guard (platform fee, commission) |
| [`AllocationRule`](#-allocation-across-n-recipients) | Exact proportional / equal split of a document — `Σ(parts) == total`, penny-corrected |
| [`proportional_split()`](#-allocation-across-n-recipients) | Penny-correct Hamilton split of a raw `Decimal` quantity (kWh, capacity, …) |
| [`BillingDocument`](#-billingdocument) | Self-validating document: three exact invariants checked at build time |
| [`RateLookup`](https://docs.rs/billing) | Capacity-based rate table (EEG §21 style) — `at_most(kWp, rate)` + `fallback(rate)` |
| [`DocumentMeta.labels`](https://docs.rs/billing) | Key-value domain annotation bag (`malo_id`, `billing_year`, …) |
| `LineItem::credit_for_usage` | Symmetric credit counterpart of `for_usage` (feed-in, refunds) |
| `LineItem::for_usage_rounded` | `for_usage` with explicit unit-price precision (prevents silent drift) |
| `LineItem::credit_for_usage_rounded` | Credit counterpart of `for_usage_rounded` (EEG feed-in with precision rounding) |
| `Period::from_display` | Construct a `Period` from any `Display` type — avoids `.to_string()` round-trip |
| `Amount::to_decimal()` | Non-consuming alias for `into_decimal()` — handy for BO4E `Betrag.wert` |
| [`minimum_charge()`](https://docs.rs/billing) | Minimum-spend shortfall helper |
| [`merge_period_documents()`](https://docs.rs/billing) | Merge two half-period documents (tariff change mid-period) |
| [`prorate()`](https://docs.rs/billing) | Scale a fixed charge to a partial period |

---

## 🚀 Quick start

```toml
# Cargo.toml
[dependencies]
billing             = "0.6"
rust_decimal_macros = "1"   # dec!() macro for constants
```

```rust
use billing::prelude::*;
use rust_decimal_macros::dec;

// Three-tier water tariff (m³)
let schedule = TariffSchedule::graduated()
    .unit("m³")                                                       // ← domain unit
    .band(TariffBand::up_to(dec!(5),   Amount::parse("0.80000").unwrap()))
    .band(TariffBand::between(dec!(5), dec!(20), Amount::parse("1.40000").unwrap()))
    .band(TariffBand::over(dec!(20),   Amount::parse("2.60000").unwrap()))
    .build()?;

let items = schedule.split(dec!(28.5))?;
// → [LineItem{5 m³ × 0.80 = 4.00}, LineItem{15 m³ × 1.40 = 21.00}, LineItem{8.5 m³ × 2.60 = 22.10}]

let doc = BillingDocument::from_positions(
    DocumentMeta {
        invoice_number: "WATER-2026-07".into(),
        period_label:   "July 2026".into(),
        ..Default::default()
    },
    items,
    vec![Box::new(FixedRateTax::new("VAT", dec!(0.10)))],
    vec![],
)?

println!("Net:   {}", doc.net_total());    // 47.10000
println!("VAT:   {}", doc.tax_total());    // 4.71000
println!("Gross: {}", doc.gross_total());  // 51.81000

doc.assert_valid();   // panics if any of the 3 arithmetic invariants fail
# Ok::<(), Box<dyn std::error::Error>>(())
```

Or use the fluent builder with the `Tariff` trait:

```rust,ignore
let doc = BillingDocument::builder()
    .meta(meta)
    .tariff(&my_tariff, &usage)?  // loads positions + tax/discount layers
    .build()?;
```

---

## 💰 `Amount<P>` — fixed-point arithmetic

`Amount<P>` stores money as an `i64` scaled by `10^P`. There is **no `f64`** anywhere
in the arithmetic path.

```rust
use billing::{Amount, RoundingStrategy};

// Parse — rejects strings with more non-zero digits than P
let price: Amount<5> = Amount::parse("0.03456")?;   // ✓ exactly 5 dp
let nope:  Result<Amount<5>, _> = Amount::parse("0.034561"); // ✗ 6th digit is non-zero

// Overflow panics (infallible ops), or returns Err (checked ops)
let a = Amount::<5>::from_int(100);
let b = Amount::<5>::parse("0.50000")?;
let c = a.checked_add(b)?;                          // Ok(100.50000)
let d = a.mul_qty(rust_decimal::Decimal::from(3u32)); // 300.00000, panics on overflow
let e = a.checked_mul_qty(rust_decimal::Decimal::from(3u32))?; // Ok(300.00000)

// += and -= (panicking, like + and -)
let mut total = Amount::<5>::ZERO;
total += a;   // 100.00000
total -= b;   // 99.50000

// Bounds
assert_eq!(Amount::<5>::MAX.to_string(), "92233720368547.75807");
assert_eq!(Amount::<5>::MIN.to_string(), "-92233720368547.75808");

// Sign
assert_eq!(Amount::<5>::parse("-3.00000").unwrap().signum(), -1i8);
assert_eq!(Amount::<5>::ZERO.signum(), 0i8);
assert_eq!(Amount::<5>::parse("1.00000").unwrap().signum(), 1i8);

// Convert to/from Decimal (lossless)
let d: rust_decimal::Decimal = rust_decimal::Decimal::from(a); // From<Amount<P>> for Decimal
let back = Amount::<5>::try_from(d)?;                          // TryFrom<Decimal>

// Convert from an integer (e.g. a database value)
let from_db = Amount::<5>::try_from(4999i64)?;  // TryFrom<i64>
assert_eq!(from_db, Amount::<5>::parse("4999.00000").unwrap());

// Round to a different precision (explicit strategy required)
let invoice: Amount<2> = c.round_to(RoundingStrategy::MidpointAwayFromZero);

// Common aliases
type EuroAmount = Amount<5>;   // 5 decimal places
type InvoiceAmt = Amount<2>;   // 2 decimal places
# Ok::<(), Box<dyn std::error::Error>>(())
```

> **Why not `f64`?**  `f64` cannot represent `0.1` exactly.
> `0.1 + 0.2 == 0.30000000000000004` in floating-point.
> `Amount<P>` uses exact base-10 arithmetic via [`rust_decimal`](https://crates.io/crates/rust_decimal).

---

## 📊 `TariffSchedule` — four pricing modes

```rust
use billing::{TariffSchedule, TariffBand, Amount};
use rust_decimal_macros::dec;

// ── Mode 1: Graduated — each tier at its own price ──────────────────────────
let graduated = TariffSchedule::graduated()
    .unit("kWh")
    .band(TariffBand::up_to(dec!(500),  Amount::parse("0.32000").unwrap()))
    .band(TariffBand::over(dec!(500),   Amount::parse("0.28000").unwrap()))
    .build().unwrap();
// split(1234.5) → [500 kWh × 0.32, 734.5 kWh × 0.28]

// ── Mode 2: Volume — all units at the top tier reached ──────────────────────
let volume = TariffSchedule::volume()
    .unit("kWh")
    .band(TariffBand::up_to(dec!(1000), Amount::parse("0.32000").unwrap()))
    .band(TariffBand::over(dec!(1000),  Amount::parse("0.28000").unwrap()))
    .build().unwrap();
// split(1234.5) → [1234.5 kWh × 0.28]

// ── Mode 3: Block — per N-unit block, rounded up ────────────────────────────
// Use case: parking (30-min slots), telephony, data packs
let block = TariffSchedule::block()
    .unit("GB")
    .band(TariffBand::block(dec!(10), Amount::parse("1.50000").unwrap()))
    .build().unwrap();
// split(35) → 4 blocks × 1.50 = 6.00  (partial block rounds UP)

// ── Mode 4: Capacity — bill on peak value, not cumulative sum ───────────────
// Use case: demand charge (peak kW), bandwidth (max Mbps), concurrent seats
let capacity = TariffSchedule::capacity()
    .unit("Mbps")
    .band(TariffBand::up_to(dec!(100), Amount::parse("50.00000").unwrap()))
    .band(TariffBand::over(dec!(100),  Amount::parse("100.00000").unwrap()))
    .build().unwrap();
// apply_peak(112.8) → 1 × 100.00  (tier selected by peak, not by sum)
```

**Validation at build time:** bands must be contiguous (no gaps, no overlaps), the
last band may be open-ended, and `block_size` must be `> 0`.

---

## 🕐 Time-of-use and dynamic pricing

```rust
use billing::{TimeOfUsePricing, TouBand, DynamicPricing, Amount};
use rust_decimal_macros::dec;

// N-band ToU: caller supplies pre-aggregated consumption per band name.
// The engine has zero knowledge of time zones or grid schedules.
let tou = TimeOfUsePricing::new(vec![
    TouBand::new("peak",    Amount::parse("0.32000").unwrap()),
    TouBand::new("off-peak", Amount::parse("0.18000").unwrap()),
]).with_unit("kWh");

let items = tou.calculate(&[("peak", dec!(823.4)), ("off-peak", dec!(411.1))])?;

// Dynamic / spot pricing: one (quantity, price) pair per interval.
let dp = DynamicPricing::from_intervals(vec![
    (dec!(100.0), Amount::parse("0.10000").unwrap()),
    (dec!(200.0), Amount::parse("0.20000").unwrap()),
])?.with_unit("kWh");
let item = dp.calculate()?;  // → single LineItem with weighted-average price
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
use rust_decimal_macros::dec;

struct ApiCall { user_id: String, bytes: u64 }
struct VmEvent { vcpus: u32, uptime_fraction: Decimal }

let events: Vec<ApiCall> = vec![];
let vms:    Vec<VmEvent> = vec![];

// SUM: total bytes transferred
let _bytes = SumAggregator::new(|e: &ApiCall| Decimal::from(e.bytes))
    .aggregate(&events);

// COUNT: number of API requests
let _reqs = CountAggregator.aggregate(&events);

// UNIQUE_COUNT: unique active users — key type is generic (zero-alloc with &str)
// The key function can return any Hash + Eq type, not just String.
let _users = UniqueCountAggregator::new(|e: &ApiCall| e.user_id.as_str())
    .aggregate(&events);
// Or with an integer key (e.g. tenant_id: u64) — no heap allocation at all.

// MAX: peak concurrent seats → pair with TariffSchedule::capacity()
let _peak = MaxAggregator::new(|e: &ApiCall| Decimal::from(e.bytes))
    .aggregate(&events);

// LATEST: current storage snapshot at end of period
let _storage = LatestAggregator::new(|e: &ApiCall| Decimal::from(e.bytes))
    .aggregate(&events);

// WEIGHTED_SUM: VM CPU-hours (VMs active for a fraction of the period)
let _cpu_h = WeightedSumAggregator::new(
    |e: &VmEvent| Decimal::from(e.vcpus) * dec!(744),  // vCPU-hours if full month
    |e: &VmEvent| e.uptime_fraction,
).aggregate(&vms);
```

---

## 🏗️ Implementing `Tariff`

The `Tariff` trait is the primary extension point. Implement it once per
pricing model in *your* crate:

```rust
use billing::{Tariff, LineItem, Amount, Quantity, UnitPrice,
              TaxLayer, DiscountLayer, BillingError};
use billing::tax::FixedRateTax;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

struct SaasPlan {
    seats:     u32,
    base_fee:  u32,
}

impl Tariff for SaasPlan {
    type Usage = ();    // usage is embedded in the struct
    type Error = BillingError;

    fn line_items(&self, _: &()) -> Result<Vec<LineItem>, BillingError> {
        Ok(vec![
            LineItem::fixed("Platform fee",
                Amount::<5>::from_int(self.base_fee.into()))
                .build()?,
            LineItem::debit("Seats")
                .quantity(Quantity::new(Decimal::from(self.seats), "seats"))
                .unit_price(UnitPrice::new(dec!(19), "EUR/seat"))
                .build()?,
        ])
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))]
    }
}

// Build a document in one call:
let doc = SaasPlan { seats: 5, base_fee: 49 }
    .bill(
        billing::DocumentMeta {
            invoice_number: "INV-001".into(),
            period_label:   "2026-07".into(),
            ..Default::default()
        },
        &(),
    )?
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 🧮 Tax layers & compound taxes

Tax layers are **ordered and cumulative**: each layer receives all previously
computed positions (net + discounts + prior taxes) in its base. This is
required for jurisdictions where one levy sits inside the base of a later tax
(e.g. an excise duty that is then subject to VAT).

```rust
use billing::{BillingDocument, DocumentMeta, LineItem, Amount,
              TaxLayer, FixedRateTax};
use billing::tax::PercentageCharge;
use rust_decimal_macros::dec;

let pos = vec![LineItem::fixed("Net charge", Amount::parse("100.00000").unwrap()).build()?];

// Layer 1: 5% levy on the net.
// Layer 2: 19% VAT — base is net (100) + levy (5) = 105.
let taxes: Vec<Box<dyn TaxLayer>> = vec![
    Box::new(PercentageCharge::new("Levy",  dec!(0.05))),
    Box::new(FixedRateTax    ::new("VAT",   dec!(0.19))),
];

let doc = BillingDocument::from_positions(
    DocumentMeta::default(), pos, taxes, vec![],
)?;

assert_eq!(doc.net_total(),   Amount::parse("100.00000").unwrap());
// Levy = 5.00;  VAT = 105 × 0.19 = 19.95
assert_eq!(doc.tax_total(),   Amount::parse("24.95000").unwrap());
assert_eq!(doc.gross_total(), Amount::parse("124.95000").unwrap());
# Ok::<(), Box<dyn std::error::Error>>(())
```

> ⚠️ **Order matters.** Tax layers are applied in declaration order.
> Place levies that form part of the VAT base *before* VAT.

---

## 🏷️ Discounts

```rust
use billing::tax::{PercentageDiscount, FixedDiscount};
use billing::{Amount, DiscountLayer};
use rust_decimal_macros::dec;

// 10% loyalty discount on all positions
let pct  = PercentageDiscount::new("Loyalty -10%", dec!(0.10));

// Fixed EUR 15 voucher
let fixed = FixedDiscount::new("Voucher", Amount::parse("15.00000").unwrap());

// Restrict a discount to positions with a specific tag
let tagged = PercentageDiscount::new("Volume rebate", dec!(0.05))
    .with_tag("commodity");
```

Discounts are applied **before** tax layers, so they reduce the taxable base.

---

## 💸 Percentage charge

A `PercentageCharge` acts like a tax layer but models a commercial surcharge
(platform fee, marketplace commission, payment processing):

```rust
use billing::tax::PercentageCharge;
use billing::Amount;
use rust_decimal_macros::dec;

// 3% commission on all positions, minimum EUR 2.00
let commission = PercentageCharge::new("Commission", dec!(0.03))
    .with_min(Amount::parse("2.00000").unwrap())
    .with_max(Amount::parse("50.00000").unwrap());
```

Place it *before* VAT in the tax layer list so the commission is included in
the VAT base.

---

## 👥 Allocation across N recipients

Split a document proportionally. Allocation is **arithmetically exact**:
`Σ(recipient totals) == original total` and each sub-document passes
`assert_valid()`.

```rust
use billing::{ProportionalAllocation, EqualAllocation, AllocationRule};
use rust_decimal_macros::dec;
# use billing::{BillingDocument, DocumentMeta, LineItem, Amount};
# let doc = BillingDocument::from_positions(DocumentMeta::default(),
#     vec![LineItem::fixed("x", Amount::parse("100.00000").unwrap()).build().unwrap()],
#     vec![], vec![]).unwrap();

// 40 / 35 / 25 % split
let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)])?;
let tenant_docs = alloc.allocate(&doc)?;

// Equal 3-way split
let equal_docs = EqualAllocation::new(3).allocate(&doc)?;

// Penny correction guarantees:
let sum: billing::Amount<5> = tenant_docs.iter().map(|d| d.net_total()).sum();
assert_eq!(sum, doc.net_total());           // ✓ exact, no drift
for d in &tenant_docs { d.assert_valid(); }  // ✓ each doc is consistent
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
use rust_decimal_macros::dec;

// §42b EEG 2023: 987.654 kWh split by occupant consumption fractions
let kwh_parts = proportional_split(
    dec!(987.654),
    &[dec!(0.45), dec!(0.35), dec!(0.20)],
    3,   // scale = 3 dp
)?;

let total: rust_decimal::Decimal = kwh_parts.iter().sum();
assert_eq!(total, dec!(987.654));  // ✓ exact sum
# Ok::<(), Box<dyn std::error::Error>>(())
```

---

## 📄 BillingDocument

`BillingDocument` holds ordered positions and pre-computed totals.
Three arithmetic invariants are enforced **exactly** (zero tolerance) at
every construction and mutation:

| Check | Invariant |
|-------|-----------|
| 1 | `Σ(net_positions + discount_positions) == net_total` |
| 2 | `Σ(tax_positions) == tax_total` |
| 3 | `net_total + tax_total == gross_total` |

```rust
use billing::{BillingDocument, DocumentMeta, LineItem, Amount, FixedRateTax};
use rust_decimal_macros::dec;

let doc = BillingDocument::from_positions(
    DocumentMeta {
        invoice_number: "INV-2026-001".into(),
        period_label:   "2026-06".into(),
        ..Default::default()
    },
    positions,
    tax_layers,
    discount_layers,
)?

doc.assert_valid();                      // all three checks
println!("Net:   {}", doc.net_total());
println!("Tax:   {}", doc.tax_total());
println!("Gross: {}", doc.gross_total());

// Append a minimum-charge shortfall after the fact:
if let Some(shortfall) = billing::minimum_charge(&doc, Amount::parse("500.00000")?, "Min.")? {
    let doc = doc.with_extra_position(shortfall)?;
}
```

---

## 🔬 Design invariants

| Invariant | How enforced |
|-----------|-------------|
| 🚫 No `f64` in monetary arithmetic | `Amount<P>` is `i64 × 10⁻ᴾ`; all intermediate ops use `rust_decimal` |
| 🔒 Private inner representation | `Amount<P>` inner `i64` is not `pub`; only `to_raw()` read access is exposed |
| 💥 Overflow is visible, never silent | `+`, `-`, `+=`, `-=`, `mul_qty`, `from_int`, `abs` all **panic**; `checked_*` variants return `Err` |
| 📐 Rounding is always explicit | `RoundingStrategy` is a required parameter; no implicit `round()` anywhere |
| ✋ No silent precision loss in `parse` | `Amount::<5>::parse("1.000011")` returns `Err` — the 6th digit cannot be represented without loss |
| 📝 Non-empty descriptions enforced | `LineItem::build()` returns `Err` for empty or whitespace-only descriptions |
| ✅ Documents are self-validating | `assert_valid()` checks 3 exact invariants (zero tolerance) |
| 🔗 Compound taxes accumulate | Each tax layer sees all prior layers in its base; `MwSt` on a `[levy, MwSt]` stack correctly includes the levy |
| ➗ Allocation is exact | `Σ(parts) == total` with per-document penny correction; each part passes `assert_valid()` |
| 🧹 Zero domain assumptions | No energy law, no BO4E, no EDIFACT, no jurisdiction constants |
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
# Install just (task runner)
cargo install just   # or: brew install just

just ci              # full local CI (fmt → lint → docs → tests → examples)
just test            # run unit + doc tests
just test-all        # with --all-features
just test-msrv       # verify Rust 1.85 compatibility
just lint            # cargo clippy -D warnings
just doc             # build & open docs
just examples        # run all three examples
just release 0.6.0   # create an annotated git tag
```

All available tasks: `just --list`

---

## 📦 Dependencies

| Crate | Role |
|-------|------|
| [`rust_decimal`](https://crates.io/crates/rust_decimal) | Exact base-10 arithmetic (no `f64`) |
| [`thiserror`](https://crates.io/crates/thiserror) | Derive macro for `ParseAmountError`; `BillingError` uses manual `Display` impl |
| [`serde`](https://crates.io/crates/serde) *(optional)* | `Serialize`/`Deserialize` on all public types |

Total non-optional dependency tree: **2 crates** (`rust_decimal` + `thiserror`).

---

## 🗂️ Crate structure

```
src/
├── lib.rs          — re-exports, prelude, crate docs
├── amount.rs       — Amount<P>, RoundingStrategy, EuroAmount, InvoiceAmt
├── quantity.rs     — Quantity, UnitPrice
├── line_item.rs    — LineItem, LineItemBuilder, Sign
├── schedule.rs     — TariffSchedule (graduated/volume/block/capacity)
├── tou.rs          — TimeOfUsePricing, TouBand, DynamicPricing
├── aggregation.rs  — UsageAggregator trait + 6 built-in implementations
├── tax.rs          — TaxLayer, DiscountLayer + built-in implementations
├── document.rs     — BillingDocument, BillingDocumentBuilder, DocumentMeta
├── allocation.rs   — AllocationRule, ProportionalAllocation, EqualAllocation
├── period.rs       — merge_period_documents(), prorate(), prorate_amount()
├── minimum.rs      — minimum_charge()
├── lookup.rs       — RateLookup, RateLookupBuilder
├── tariff.rs       — Tariff trait
└── error.rs        — BillingError, ParseAmountError
```

---

## 📜 License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT license](LICENSE-MIT)

at your option.
