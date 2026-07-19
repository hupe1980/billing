//! # billing — Generic tariff billing engine
//!
//! `billing` is a **pure, domain-agnostic, dependency-minimal** calculation engine.
//! It knows nothing about energy, jurisdictions, or currencies beyond an ISO 4217
//! label. All domain knowledge lives in the *caller*.
//!
//! ## Core concepts
//!
//! | Type / Function | Purpose |
//! |-----------------|---------|
//! | [`Amount<P>`] | Fixed-point monetary arithmetic with compile-time precision (`P` ≤ 18) |
//! | [`Currency`] | ISO 4217 code for generated labels and cross-document checks |
//! | [`LineItem`] | Atomic billing unit: quantity × unit-price → net amount |
//! | [`Period`] | Billing period — construct from strings or any `Display` type |
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
//! | [`proportional_split`] | Penny-correct Hamilton split of a raw `Decimal` quantity |
//! | [`RateLookup`] | Parameter-keyed rate table (installed capacity → rate) |
//!
//! ## Design invariants
//!
//! - **No `f64` in monetary arithmetic** — [`Amount<P>`] is `i64 × 10⁻ᴾ`.
//! - **No I/O, no async, no `unsafe`** — every function is a pure `fn`.
//! - **Overflow is visible** — `+`, `-` and `mul_qty` panic; every `checked_*`
//!   variant returns `Err` and never panics, including on `Decimal`'s own
//!   overflow (whose operators panic rather than saturating).
//! - **Rounding is always explicit** — [`RoundingStrategy`] is a required parameter.
//! - **No implicit currency** — [`Currency`] defaults to ISO 4217 `XXX`, never `EUR`.
//! - **Self-validating documents** — [`BillingDocument::validate`] returns `Result`;
//!   [`BillingDocument::assert_valid`] panics on failure (convenient for tests).
//! - **Allocation is exact** — [`ProportionalAllocation`] guarantees
//!   `Σ(recipient totals) == original total` with per-document penny correction.
//! - **Invariants survive deserialisation** — validated types re-run their checks
//!   via `#[serde(try_from = ...)]` rather than trusting reconstructed fields.
//!
//! ## README
//!
//! The crate README is included below so that **every Rust example in it is
//! compiled and run as a doctest**. Before this was wired up, several README
//! snippets had drifted into code that did not compile (missing semicolons,
//! undefined identifiers, constructors whose signatures had changed). Keeping it
//! here makes that class of documentation rot impossible.
#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
// Warnings are promoted to errors in CI via RUSTFLAGS="-D warnings".
// Not set here to avoid breaking downstream users on new compiler releases.
#![warn(missing_docs, unreachable_pub, rust_2018_idioms, clippy::all)]

pub mod advance;
pub mod aggregation;
pub mod allocation;
pub mod amount;
pub mod currency;
pub mod document;
pub mod error;
pub mod line_item;
pub mod lookup;
pub mod minimum;
pub mod period;
pub mod quantity;
pub mod schedule;
pub mod settlement;
pub mod tariff;
pub mod tax;
pub mod tou;
pub mod vat;

pub use advance::{AdvancePayment, DocumentKind, Prepayment, residual_breakdown};
pub use aggregation::{
    CountAggregator, LatestAggregator, MaxAggregator, SumAggregator, UniqueCountAggregator,
    UsageAggregator, WeightedSumAggregator,
};
pub use allocation::{AllocationRule, EqualAllocation, ProportionalAllocation, proportional_split};
pub use amount::{Amount, EuroAmount, InvoiceAmt, RoundingStrategy};
pub use currency::Currency;
pub use document::{BillingDocument, BillingDocumentBuilder, DocumentMeta};
pub use error::{BillingError, ParseAmountError};
pub use line_item::{LineItem, LineItemBuilder, Sign};
pub use lookup::{RateLookup, RateLookupBuilder};
pub use minimum::minimum_charge;
pub use period::{Period, merge_period_documents, prorate, prorate_amount};
pub use quantity::{Quantity, UnitPrice};
pub use schedule::{TariffBand, TariffSchedule};
pub use settlement::CashRounding;
pub use tariff::Tariff;
pub use tax::{
    DiscountLayer, FixedDiscount, FixedRateTax, PerUnitLevy, PercentageCharge, PercentageDiscount,
    TaxLayer,
};
pub use tou::{
    DynamicPricing, DynamicPricingBuilder, TimeOfUsePricing, TimeOfUsePricingBuilder, TouBand,
};
pub use vat::{TaxBreakdownEntry, TaxCategory};

/// Tag values the engine assigns to generated positions to classify them.
///
/// These are written by the built-in tax and discount layers and read back by
/// other layers — `PerUnitLevy`, for instance, excludes anything tagged `"tax"`
/// from its base so that stacked levies do not compound. Because they are load
/// bearing, caller-supplied labels that would land in this namespace (a
/// time-of-use band named `"tax"`, say) are rejected rather than silently
/// changing how a document is priced.
pub mod tags {
    /// Applied to every position produced by a [`crate::TaxLayer`].
    pub const TAX: &str = "tax";
    /// Applied to per-unit levy positions, in addition to [`TAX`].
    pub const LEVY: &str = "levy";
    /// Applied to [`crate::PercentageCharge`] positions.
    pub const PERCENTAGE_CHARGE: &str = "percentage-charge";
    /// Applied to every position produced by a [`crate::DiscountLayer`].
    pub const DISCOUNT: &str = "discount";
    /// Applied to a [`crate::minimum_charge`] shortfall position.
    pub const MINIMUM_CHARGE: &str = "minimum-charge";

    /// Every reserved tag, for diagnostics.
    pub const RESERVED: &[&str] = &[TAX, LEVY, PERCENTAGE_CHARGE, DISCOUNT, MINIMUM_CHARGE];

    /// Whether `tag` is reserved by the engine.
    #[must_use]
    pub fn is_reserved(tag: &str) -> bool {
        RESERVED.contains(&tag)
    }
}

/// Reject an empty or whitespace-only unit label.
///
/// An empty unit renders as `"EUR/"` in a generated unit-price label and as a
/// bare space in a description — visible nonsense on an invoice, and cheap to
/// prevent at the boundary.
pub(crate) fn validate_unit(unit: String) -> Result<String, BillingError> {
    if unit.trim().is_empty() {
        return Err(BillingError::InvalidInput {
            reason: "unit label must not be empty".into(),
        });
    }
    Ok(unit)
}

/// Convenience glob import — covers all primary types and traits.
pub mod prelude {
    pub use crate::{
        AdvancePayment, AllocationRule, Amount, BillingDocument, BillingDocumentBuilder,
        BillingError, CashRounding, CountAggregator, Currency, DiscountLayer, DocumentKind,
        DocumentMeta, DynamicPricing, EqualAllocation, EuroAmount, FixedDiscount, FixedRateTax,
        InvoiceAmt, LatestAggregator, LineItem, MaxAggregator, ParseAmountError, PerUnitLevy,
        PercentageCharge, PercentageDiscount, Period, Prepayment, ProportionalAllocation, Quantity,
        RateLookup, RateLookupBuilder, RoundingStrategy, Sign, SumAggregator, Tariff, TariffBand,
        TariffSchedule, TaxBreakdownEntry, TaxCategory, TaxLayer, TimeOfUsePricing, TouBand,
        UniqueCountAggregator, UnitPrice, UsageAggregator, WeightedSumAggregator,
        merge_period_documents, minimum_charge, proportional_split, prorate, prorate_amount,
        residual_breakdown,
    };
}
