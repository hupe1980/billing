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
/// without a semver-major bump.  Always include a `_ =>` arm when matching.
///
/// ## Error context
///
/// [`BillingError::MonetaryOverflow`] carries `input_value: Option<Decimal>` —
/// the input that caused the overflow when known.  This lets callers log the
/// offending value:
///
/// ```rust
/// use billing::{Amount, BillingError};
/// use rust_decimal::Decimal;
///
/// let huge = Decimal::from(i64::MAX / 100_000 + 1);
/// match Amount::<5>::checked_from_decimal(huge) {
///     Ok(a)  => println!("ok: {a}"),
///     Err(BillingError::MonetaryOverflow { input_value: Some(v), precision }) => {
///         eprintln!("overflow: {v} does not fit in Amount<{precision}>");
///     }
///     Err(e) => eprintln!("other error: {e}"),
/// }
/// ```
///
/// [`BillingError::InvalidInput`] and [`BillingError::InvalidSchedule`] carry a
/// `reason: String` which accepts both `&'static str` literals (via `.into()`) and
/// dynamic messages (`format!("{}", value)`).
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
    /// The `check` field identifies which invariant failed — one of
    /// `"net_total"`, `"tax_total"`, `"gross_total"`, `"discount_total"`,
    /// `"tax_breakdown"`, `"tax_breakdown_total"`, `"discount_positions"`,
    /// `"prepaid"`, `"rounding"`, `"prepaid_vs_advances"`, `"net_positions"` or
    /// `"tax_positions"`.
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

    /// Two amounts or documents in different currencies were combined.
    ///
    /// Produced by [`crate::merge_period_documents`]. Adding amounts across
    /// currencies is never meaningful, so the engine refuses rather than silently
    /// producing a nonsense total.
    CurrencyMismatch {
        /// The currency of the left-hand / first operand.
        left: crate::currency::Currency,
        /// The currency of the right-hand / second operand.
        right: crate::currency::Currency,
    },

    /// An [`crate::Amount`] could not be parsed from its textual form.
    ///
    /// Wraps [`ParseAmountError`] so that parsing composes with `?` inside
    /// functions returning [`BillingError`].
    Parse(ParseAmountError),
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
            Self::CurrencyMismatch { left, right } => {
                write!(f, "currency mismatch: cannot combine {left} with {right}")
            }
            Self::Parse(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for BillingError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Parse(e) => Some(e),
            _ => None,
        }
    }
}

/// Lets `Amount::parse(..)?` compose inside functions returning [`BillingError`].
impl From<ParseAmountError> for BillingError {
    fn from(e: ParseAmountError) -> Self {
        Self::Parse(e)
    }
}

/// `Infallible` → `BillingError` conversion needed when `Tariff::Error = Infallible`.
impl From<std::convert::Infallible> for BillingError {
    fn from(x: std::convert::Infallible) -> Self {
        match x {}
    }
}
