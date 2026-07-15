//! # billing — Generic tariff billing engine
//!
//! `billing` is a **pure, domain-agnostic, dependency-minimal** calculation engine.
//! It knows nothing about energy, currencies, or jurisdiction-specific law.
//! All domain knowledge lives in the *caller*.
//!
//! ## Core concepts
//!
//! | Type / Function | Purpose |
//! |-----------------|---------|
//! | [`Amount<P>`] | Fixed-point monetary arithmetic with compile-time precision |
//! | [`LineItem`] | Atomic billing unit: quantity × unit-price → net amount |
//! | [`Period`] | Billing period — construct from strings or any `Display` type via [`Period::from_display`] |
//! | [`TariffSchedule`] | Graduated / volume / block / capacity pricing |
//! | [`TimeOfUsePricing`] | N-band time-of-use pricing (peak / off-peak / …) |
//! | [`DynamicPricing`] | Per-interval price sequence (spot, real-time) |
//! | [`UsageAggregator`] | Pre-billing event aggregation (SUM / MAX / COUNT / …) |
//! | [`TaxLayer`] | Composable tax and surcharge overlays |
//! | [`DiscountLayer`] | Composable discount overlays |
//! | [`Tariff`] | Primary extension point for domain-specific pricing |
//! | [`BillingDocument`] | Self-validating invoice with ordered positions + totals |
//! | [`DocumentMeta`] | Invoice header with `labels` bag for domain annotations |
//! | [`AllocationRule`] | Proportional split of a [`BillingDocument`] across N recipients |
//! | [`proportional_split`] | Penny-correct proportional split of a raw `Decimal` quantity (kWh, capacity, …) |
//! | [`RateLookup`] | Capacity-based rate table (EEG §21 style) |
//!
//! ## Quick start
//!
//! ```rust
//! use billing::prelude::*;
//! use rust_decimal_macros::dec;
//!
//! // Three-tier water tariff
//! let schedule = TariffSchedule::graduated()
//!     .unit("m³")
//!     .band(TariffBand::up_to(dec!(5),  Amount::parse("0.80000").unwrap()))
//!     .band(TariffBand::between(dec!(5), dec!(20), Amount::parse("1.40000").unwrap()))
//!     .band(TariffBand::over(dec!(20),  Amount::parse("2.60000").unwrap()))
//!     .build()
//!     .unwrap();
//!
//! let items = schedule.split(dec!(28.5)).unwrap();
//! let doc = BillingDocument::from_positions(
//!     DocumentMeta::default(), items, vec![], vec![],
//! ).unwrap();
//! doc.assert_valid();
//! ```
//!
//! ## Design invariants
//!
//! - **No `f64` in monetary arithmetic** — `Amount<P>` is `i64 × 10⁻ᴾ`.
//! - **No I/O, no async, no `unsafe`** — every function is a pure `fn`.
//! - **Overflow panics** — `+`, `-` and `mul_qty` panic on overflow; use
//!   [`Amount::checked_add`] / [`Amount::checked_sub`] for fallible arithmetic.
//! - **Rounding is always explicit** — [`RoundingStrategy`] is a required parameter.
//! - **Self-validating documents** — [`BillingDocument::validate`] returns `Result`;
//!   [`BillingDocument::assert_valid`] panics on failure (convenient for tests).
//! - **Allocation is exact** — [`ProportionalAllocation`] guarantees
//!   `Σ(recipient totals) == original total` with per-document penny correction.

#![forbid(unsafe_code)]
// Warnings are promoted to errors in CI via RUSTFLAGS="-D warnings".
// Not set here to avoid breaking downstream users on new compiler releases.
#![warn(missing_docs, unreachable_pub, rust_2018_idioms, clippy::all)]

pub mod aggregation;
pub mod allocation;
pub mod amount;
pub mod document;
pub mod error;
pub mod line_item;
pub mod lookup;
pub mod minimum;
pub mod period;
pub mod quantity;
pub mod schedule;
pub mod tariff;
pub mod tax;
pub mod tou;

pub use aggregation::{
    CountAggregator, LatestAggregator, MaxAggregator, SumAggregator, UniqueCountAggregator,
    UsageAggregator, WeightedSumAggregator,
};
pub use allocation::{AllocationRule, EqualAllocation, ProportionalAllocation, proportional_split};
pub use amount::{Amount, EuroAmount, InvoiceAmt, RoundingStrategy};
pub use document::{BillingDocument, BillingDocumentBuilder, DocumentMeta};
pub use error::{BillingError, ParseAmountError};
pub use line_item::{LineItem, LineItemBuilder, Period, Sign};
pub use lookup::{RateLookup, RateLookupBuilder};
pub use minimum::minimum_charge;
pub use period::{merge_period_documents, prorate, prorate_amount};
pub use quantity::{Quantity, UnitPrice};
pub use schedule::{TariffBand, TariffSchedule};
pub use tariff::Tariff;
pub use tax::{
    DiscountLayer, FixedDiscount, FixedRateTax, PerUnitLevy, PercentageCharge, PercentageDiscount,
    TaxLayer,
};
pub use tou::{DynamicPricing, TimeOfUsePricing, TouBand};

/// Convenience glob import — covers all primary types and traits.
pub mod prelude {
    pub use crate::{
        AllocationRule, Amount, BillingDocument, BillingDocumentBuilder, BillingError,
        CountAggregator, DiscountLayer, DocumentMeta, DynamicPricing, EqualAllocation, EuroAmount,
        FixedDiscount, FixedRateTax, InvoiceAmt, LatestAggregator, LineItem, MaxAggregator,
        ParseAmountError, PerUnitLevy, PercentageCharge, PercentageDiscount, Period,
        ProportionalAllocation, Quantity, RateLookup, RateLookupBuilder, RoundingStrategy, Sign,
        SumAggregator, Tariff, TariffBand, TariffSchedule, TaxLayer, TimeOfUsePricing, TouBand,
        UniqueCountAggregator, UnitPrice, UsageAggregator, WeightedSumAggregator,
        merge_period_documents, minimum_charge, proportional_split, prorate, prorate_amount,
    };
}
