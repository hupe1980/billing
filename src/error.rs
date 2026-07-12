//! Error types for the billing engine.
use thiserror::Error;

/// Error returned when parsing an [`crate::Amount`] from a string or decimal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("cannot parse amount: {input:?}")]
pub struct ParseAmountError {
    /// The original input string that could not be parsed.
    pub input: String,
}

/// All errors produced by the billing engine.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Error)]
pub enum BillingError {
    /// An arithmetic operation on an [`crate::Amount`] exceeded the `i64` range.
    #[error("monetary overflow: amount exceeds representable range for Amount<{precision}>")]
    MonetaryOverflow {
        /// The precision `P` of the overflowing `Amount<P>`.
        precision: u8,
    },

    /// A [`crate::TariffSchedule`] was built or used incorrectly.
    #[error("invalid tariff schedule: {reason}")]
    InvalidSchedule {
        /// Human-readable explanation.
        reason: &'static str,
    },

    /// A function argument was invalid.
    #[error("invalid input: {reason}")]
    InvalidInput {
        /// Human-readable explanation.
        reason: &'static str,
    },

    /// [`crate::BillingDocument::assert_valid`] detected an arithmetic inconsistency.
    ///
    /// The `check` field identifies which invariant failed
    /// (`"net_total"`, `"tax_total"`, or `"gross_total"`).
    #[error("document validation failed ({check}): expected {expected}, got {actual}")]
    ValidationFailed {
        /// Which consistency check failed.
        check: &'static str,
        /// The value computed from positions.
        actual: String,
        /// The value stored in the totals field.
        expected: String,
    },

    /// [`crate::ProportionalAllocation`] shares do not sum to `1.0 ± 1e-9`.
    #[error("allocation shares must sum to 1.0 ± 1e-9 (got {sum})")]
    InvalidAllocationShares {
        /// The actual sum of the provided shares.
        sum: String,
    },

    /// [`crate::prorate`] was called with `total_days = 0`.
    #[error("proration requires total_days > 0")]
    ZeroPeriod,

    /// A [`crate::TaxLayer`] or [`crate::DiscountLayer`] `compute` call failed.
    #[error("tax or discount layer error: {reason}")]
    LayerError {
        /// Human-readable explanation from the layer implementation.
        reason: String,
    },
}

/// `Infallible` → `BillingError` conversion needed when `Tariff::Error = Infallible`.
impl From<std::convert::Infallible> for BillingError {
    fn from(x: std::convert::Infallible) -> Self {
        match x {}
    }
}
