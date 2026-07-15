//! Error types for the billing engine.
use rust_decimal::Decimal;
use std::fmt;
use thiserror::Error;

/// Error returned when parsing an [`crate::Amount`] from a string or decimal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("cannot parse amount: {input:?}")]
pub struct ParseAmountError {
    /// The original input string that could not be parsed.
    pub input: String,
}

/// All errors produced by the billing engine.
///
/// This enum is `#[non_exhaustive]`: new variants may be added in **minor** releases
/// without a semver-major bump.  Always include a `_ =>` arm when matching:
///
/// ```rust
/// use billing::BillingError;
/// fn describe(e: &BillingError) -> &'static str {
///     match e {
///         BillingError::MonetaryOverflow { .. } => "overflow",
///         BillingError::InvalidInput { .. }    => "bad input",
///         _ => "other billing error",  // ← required; handles future variants
///     }
/// }
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum BillingError {
    /// An arithmetic operation on an [`crate::Amount`] exceeded the `i64` range.
    ///
    /// `input_value` carries the original `Decimal` that caused the overflow
    /// when known (e.g. from [`crate::Amount::checked_from_decimal`]). It is
    /// `None` for internal arithmetic operations (add / sub / mul) where no
    /// single input value is solely responsible.
    MonetaryOverflow {
        /// The precision `P` of the overflowing `Amount<P>`.
        precision: u8,
        /// The input value that caused the overflow, when known.
        /// Callers can log this to identify which amount triggered the overflow.
        input_value: Option<Decimal>,
    },

    /// A [`crate::TariffSchedule`] was built or used incorrectly.
    InvalidSchedule {
        /// Human-readable explanation. Accepts static literals (`"msg".into()`)
        /// and dynamic messages (`format!(...)`).
        reason: String,
    },

    /// A function argument was invalid.
    InvalidInput {
        /// Human-readable explanation. Accepts static literals (`"msg".into()`)
        /// and dynamic messages (`format!(...)`).
        reason: String,
    },

    /// [`crate::BillingDocument::assert_valid`] detected an arithmetic inconsistency.
    ///
    /// The `check` field identifies which invariant failed
    /// (`"net_total"`, `"tax_total"`, or `"gross_total"`).
    ValidationFailed {
        /// Which consistency check failed.
        check: String,
        /// The value computed from positions.
        actual: String,
        /// The value stored in the totals field.
        expected: String,
    },

    /// [`crate::ProportionalAllocation`] shares do not sum to `1.0 ± 1e-9`.
    InvalidAllocationShares {
        /// The actual sum of the provided shares.
        sum: String,
    },

    /// [`crate::prorate`] was called with `total_days = 0`.
    ZeroPeriod,

    /// A [`crate::TaxLayer`] or [`crate::DiscountLayer`] `compute` call failed.
    LayerError {
        /// Human-readable explanation from the layer implementation.
        reason: String,
    },
}

impl fmt::Display for BillingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MonetaryOverflow {
                precision,
                input_value: Some(v),
            } => write!(
                f,
                "monetary overflow: input {v} exceeds representable range for Amount<{precision}>"
            ),
            Self::MonetaryOverflow {
                precision,
                input_value: None,
            } => write!(
                f,
                "monetary overflow: amount exceeds representable range for Amount<{precision}>"
            ),
            Self::InvalidSchedule { reason } => write!(f, "invalid tariff schedule: {reason}"),
            Self::InvalidInput { reason } => write!(f, "invalid input: {reason}"),
            Self::ValidationFailed {
                check,
                actual,
                expected,
            } => write!(
                f,
                "document validation failed ({check}): expected {expected}, got {actual}"
            ),
            Self::InvalidAllocationShares { sum } => write!(
                f,
                "allocation shares must sum to 1.0 \u{00b1} 1e-9 (got {sum})"
            ),
            Self::ZeroPeriod => write!(f, "proration requires total_days > 0"),
            Self::LayerError { reason } => write!(f, "tax or discount layer error: {reason}"),
        }
    }
}

impl std::error::Error for BillingError {}

/// `Infallible` → `BillingError` conversion needed when `Tariff::Error = Infallible`.
impl From<std::convert::Infallible> for BillingError {
    fn from(x: std::convert::Infallible) -> Self {
        match x {}
    }
}
