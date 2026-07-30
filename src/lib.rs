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
//! | [`Tariff`] | Primary extension point for usage-driven pricing |
//! | [`ScalarTariff`] | Pricing for pre-computed settlements — no `Usage` type, no ignored argument |
//! | [`Billing`] | Three outcomes: billable, not billable (with a domain reason), error |
//! | [`BillingDocument`] | Self-validating invoice with ordered positions + totals |
//! | [`AmountScale`] | Assemble every amount at an interchange format's decimal limit |
//! | [`DocumentMeta`] | Invoice header with `labels` bag for domain annotations |
//! | [`LineVat`] | Per-position VAT category + rate (EN 16931 BT-151/152, BT-95/96, BT-102/103) |
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
//!   The `Decimal` → [`Amount`] conversion therefore *refuses* excess precision
//!   ([`Amount::checked_from_decimal`]) rather than rounding it silently, and agrees
//!   with [`Amount::parse`] on exactly what is representable;
//!   [`Amount::from_decimal_rounded`] is the opt-in that rounds.
//! - **Comparisons are total** — [`Amount::within_tolerance_ppm`] returns `bool`, so
//!   a tolerance check can never degrade into a spurious finding via `unwrap_or`.
//! - **"Nothing to bill" is not an error and not an empty list** — [`Billing`]
//!   carries the domain's own reason, and tariffs that always bill opt out at the
//!   type level with `NotBillable = Infallible`.
//! - **No implicit currency** — [`Currency`] defaults to ISO 4217 `XXX`, never `EUR`.
//! - **Self-validating documents** — [`BillingDocument::validate`] returns `Result`;
//!   [`BillingDocument::assert_valid`] panics on failure (convenient for tests).
//! - **Allocation is exact** — [`ProportionalAllocation`] guarantees
//!   `Σ(recipient totals) == original total` with per-document penny correction.
//! - **Invariants survive deserialisation** — validated types re-run their checks
//!   via `#[serde(try_from = ...)]` rather than trusting reconstructed fields.
//! - **Value added tax is distinguishable from a charge** — every tax position a
//!   layer produced with a VAT breakdown is marked [`tags::VAT`], so
//!   [`BillingDocument::vat_total`] (EN 16931 BT-110) and
//!   [`BillingDocument::charge_total`] (BT-108) are exact rather than heuristic.
//!   The mark comes from the layer's own `breakdown` return value, so it is as
//!   accurate for a third-party [`TaxLayer`] as for a built-in one.
//! - **Precision reduction happens at the leaves** — [`AmountScale`] rounds every
//!   position and layer output *before* the totals are summed, so an interchange
//!   format's decimal limit and its totals identities hold at once. Rounding a
//!   finished document satisfies neither.
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
pub use amount::{Amount, AmountScale, EuroAmount, InvoiceAmt, RoundingStrategy};
pub use currency::Currency;
pub use document::{BillingDocument, BillingDocumentBuilder, DocumentMeta};
pub use error::{BillingError, ParseAmountError};
pub use line_item::{AllowanceCharge, LineItem, LineItemBuilder, Sign};
pub use lookup::{RateLookup, RateLookupBuilder};
pub use minimum::minimum_charge;
pub use period::{Period, merge_period_documents, prorate, prorate_amount};
pub use quantity::{Quantity, UnitPrice};
pub use schedule::{TariffBand, TariffSchedule};
pub use settlement::CashRounding;
pub use tariff::{Billed, Billing, Positions, ScalarTariff, Tariff};
pub use tax::{
    DiscountLayer, FixedDiscount, FixedRateTax, PerUnitLevy, PercentageCharge, PercentageDiscount,
    TaxLayer,
};
pub use tou::{
    DynamicPricing, DynamicPricingBuilder, TimeOfUsePricing, TimeOfUsePricingBuilder, TouBand,
};
pub use vat::{LineVat, TaxBreakdownEntry, TaxCategory};

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
    /// Applied to a tax position whose layer contributed a VAT breakdown entry —
    /// in addition to [`TAX`].
    ///
    /// This is the marker that makes the value-added-tax / charge split
    /// **decidable** rather than a guess. EN 16931 puts the two in completely
    /// different places, and a consumer that conflates them produces an invoice no
    /// validator accepts:
    ///
    /// | Position | EN 16931 | Contributes to |
    /// |---|---|---|
    /// | tagged [`VAT`] | value added tax (BG-23) | **BT-110**, the VAT total |
    /// | tagged [`TAX`] but not [`VAT`] | document level charge (BG-21) | **BT-108** → BT-109, i.e. the *taxable base* |
    ///
    /// Mapping the whole of [`crate::BillingDocument::tax_total`] to BT-110 makes
    /// **BR-CO-14** (`BT-110 = Σ BT-117`) fail on every document carrying a levy or
    /// a commission. [`crate::BillingDocument::vat_total`] and
    /// [`crate::BillingDocument::charge_total`] read this tag and give the two
    /// figures directly.
    ///
    /// Unlike a heuristic over [`LEVY`] / [`PERCENTAGE_CHARGE`], this is **total**:
    /// it is written by the engine from the layer's own
    /// [`breakdown`](crate::TaxLayer::breakdown) return value, so a third-party
    /// [`crate::TaxLayer`] is classified as accurately as a built-in one.
    pub const VAT: &str = "vat";
    /// Applied to per-unit levy positions, in addition to [`TAX`].
    pub const LEVY: &str = "levy";
    /// Applied to [`crate::PercentageCharge`] positions.
    pub const PERCENTAGE_CHARGE: &str = "percentage-charge";
    /// Applied to every position produced by a [`crate::DiscountLayer`].
    pub const DISCOUNT: &str = "discount";
    /// Applied to a [`crate::minimum_charge`] shortfall position.
    pub const MINIMUM_CHARGE: &str = "minimum-charge";

    /// Every reserved tag, for diagnostics.
    pub const RESERVED: &[&str] = &[TAX, VAT, LEVY, PERCENTAGE_CHARGE, DISCOUNT, MINIMUM_CHARGE];

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
pub(crate) fn check_unit(what: &str, unit: &str) -> Result<(), BillingError> {
    if unit.trim().is_empty() {
        let subject = if what.is_empty() {
            "unit label".to_owned()
        } else {
            format!("{what} unit label")
        };
        return Err(BillingError::InvalidInput {
            reason: format!("{subject} must not be empty"),
        });
    }
    Ok(())
}

/// [`check_unit`] in the owned-value position the schedule and time-of-use
/// builders use, where the checked label is moved into the built value.
pub(crate) fn validate_unit(unit: String) -> Result<String, BillingError> {
    check_unit("", &unit)?;
    Ok(unit)
}

/// Convenience glob import — covers all primary types and traits.
pub mod prelude {
    pub use crate::{
        AdvancePayment, AllocationRule, AllowanceCharge, Amount, AmountScale, Billed, Billing,
        BillingDocument, BillingDocumentBuilder, BillingError, CashRounding, CountAggregator,
        Currency, DiscountLayer, DocumentKind, DocumentMeta, DynamicPricing, EqualAllocation,
        EuroAmount, FixedDiscount, FixedRateTax, InvoiceAmt, LatestAggregator, LineItem, LineVat,
        MaxAggregator, ParseAmountError, PerUnitLevy, PercentageCharge, PercentageDiscount, Period,
        Positions, Prepayment, ProportionalAllocation, Quantity, RateLookup, RateLookupBuilder,
        RoundingStrategy, ScalarTariff, Sign, SumAggregator, Tariff, TariffBand, TariffSchedule,
        TaxBreakdownEntry, TaxCategory, TaxLayer, TimeOfUsePricing, TouBand, UniqueCountAggregator,
        UnitPrice, UsageAggregator, WeightedSumAggregator, merge_period_documents, minimum_charge,
        proportional_split, prorate, prorate_amount, residual_breakdown,
    };
}
