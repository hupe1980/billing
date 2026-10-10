//! The crate's single error type: what went wrong ([`ErrorKind`]) and where
//! ([`Location`]).

use alloc::string::String;
use core::fmt;

use rust_decimal::Decimal;

use crate::Currency;

/// `Result` with this crate's [`Error`] as the default error type.
pub type Result<T, E = Error> = core::result::Result<T, E>;

/// An error: a [`kind`](Error::kind) saying why, an optional
/// [`location`](Error::location) saying where, and the [`rule`](Error::rule) it
/// enforces, if one exists.
///
/// Match on the kind, the location and the rule; the `Display` text is for
/// people and may be reworded in any release. It is one lowercase sentence with
/// the location first (see [`Location`] for its forms), e.g.
/// `line 1: price -1 is negative (BR-27)`.
///
/// ```
/// use billing::{Amount, ErrorKind};
///
/// let err = Amount::<2>::parse("1,000").unwrap_err();
/// assert!(matches!(err.kind(), ErrorKind::Malformed { .. }));
/// assert_eq!(err.location(), None);
/// assert_eq!(err.rule(), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    kind: ErrorKind,
    at: Option<Location>,
    /// A rule the raise site knows and neither kind nor location carries
    /// (BR-32 vs BR-37 for a document adjustment without VAT).
    rule: Option<&'static str>,
}

impl Error {
    /// Why the operation failed.
    #[must_use]
    pub fn kind(&self) -> &ErrorKind {
        &self.kind
    }

    /// Where the failure was found, when the input had a position.
    #[must_use]
    pub fn location(&self) -> Option<&Location> {
        self.at.as_ref()
    }

    /// The EN 16931 or Peppol BIS rule id this error enforces, if any.
    ///
    /// | kind | rule |
    /// |---|---|
    /// | [`NegativePrice`](ErrorKind::NegativePrice) | BR-27 |
    /// | [`InvalidBaseQuantity`](ErrorKind::InvalidBaseQuantity) | PEPPOL-EN16931-R121 |
    /// | [`AdjustmentBasis`](ErrorKind::AdjustmentBasis) | PEPPOL-EN16931-R040 |
    /// | [`PriceBaseUnit`](ErrorKind::PriceBaseUnit) | PEPPOL-EN16931-R130 |
    /// | [`InvalidVatAccounting`](ErrorKind::InvalidVatAccounting) | PEPPOL-EN16931-R005 |
    /// | [`MissingVat`](ErrorKind::MissingVat) at a [`Location::Line`] | BR-CO-04 |
    /// | [`MissingVat`](ErrorKind::MissingVat) at a document allowance ([`Location::Adjustment`]) | BR-32 |
    /// | [`MissingVat`](ErrorKind::MissingVat) at a document charge ([`Location::Adjustment`]) | BR-37 |
    /// | [`InvalidVat`](ErrorKind::InvalidVat) | per [`VatError`](crate::VatError): BR-*-05, BR-*-10, BR-O-11, BR-B-02 |
    ///
    /// Every other kind is arithmetic or domain validation with no artefact
    /// rule: `None`.
    ///
    /// ```
    /// use billing::{Error, ErrorKind};
    /// use rust_decimal::dec;
    ///
    /// let err = Error::from(ErrorKind::NegativePrice { price: dec!(-1) });
    /// assert_eq!(err.rule(), Some("BR-27"));
    /// ```
    #[must_use]
    pub fn rule(&self) -> Option<&'static str> {
        if self.rule.is_some() {
            return self.rule;
        }
        match (&self.kind, self.at) {
            (ErrorKind::MissingVat, Some(Location::Line(_))) => Some(rules::BR_CO_04),
            (kind, _) => kind.rule(),
        }
    }

    /// Attach (or replace) the location.
    #[must_use]
    pub(crate) fn at(mut self, location: Location) -> Self {
        self.at = Some(location);
        self
    }

    /// Record the rule the raise site knows to be broken.
    #[must_use]
    pub(crate) fn with_rule(mut self, rule: &'static str) -> Self {
        self.rule = Some(rule);
        self
    }
}

impl From<ErrorKind> for Error {
    fn from(kind: ErrorKind) -> Self {
        Self {
            kind,
            at: None,
            rule: None,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(at) = &self.at {
            write!(f, "{at}: ")?;
        }
        fmt::Display::fmt(&self.kind, f)
    }
}

impl core::error::Error for Error {}

/// Rule ids cited by [`Error::rule`]. Kept as constants so the citation check
/// sees every id the crate claims to enforce.
pub(crate) mod rules {
    /// BR-27: the item net price (BT-146) shall not be negative.
    pub(super) const BR_27: &str = "BR-27";
    /// PEPPOL-EN16931-R121: base quantity must be a positive number above zero.
    pub(super) const PEPPOL_R121: &str = "PEPPOL-EN16931-R121";
    /// PEPPOL-EN16931-R040: allowance/charge amount = base × percentage / 100.
    pub(super) const PEPPOL_R040: &str = "PEPPOL-EN16931-R040";
    /// PEPPOL-EN16931-R130: unit code of the price base quantity must be the
    /// invoiced quantity's.
    pub(super) const PEPPOL_R130: &str = "PEPPOL-EN16931-R130";
    /// PEPPOL-EN16931-R005: VAT accounting currency must differ from the
    /// invoice currency.
    pub(super) const PEPPOL_R005: &str = "PEPPOL-EN16931-R005";
    /// BR-CO-04: each invoice line shall be categorised with a VAT category
    /// code (BT-151).
    pub(super) const BR_CO_04: &str = "BR-CO-04";
    /// BR-32: each document level allowance shall have a VAT category code
    /// (BT-95).
    pub(crate) const BR_32: &str = "BR-32";
    /// BR-37: each document level charge shall have a VAT category code
    /// (BT-102).
    pub(crate) const BR_37: &str = "BR-37";
}

/// Why an operation failed.
///
/// `#[non_exhaustive]`: later releases add kinds without a breaking change, so
/// a `match` needs a wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// A result does not fit the representation (`i64` raw units for
    /// [`Amount`](crate::Amount), `i128` units inside [`split`](crate::split),
    /// 96 bits for `Decimal`). Never approximated. Where a value also has
    /// excess digits, [`ExcessPrecision`](Self::ExcessPrecision) is reported
    /// instead.
    Overflow,
    /// `value` has non-zero digits beyond `decimals` and the operation is exact,
    /// so it refuses rather than round.
    ExcessPrecision {
        /// The value that does not fit.
        value: Decimal,
        /// The number of decimals it had to fit.
        decimals: u8,
    },
    /// Text is not a decimal in the grammar `[+-]digits[.digits]`.
    Malformed {
        /// The rejected text.
        input: String,
    },
    /// A divisor is zero.
    DivisionByZero,
    /// A rounding increment is not above zero
    /// ([`Amount::round_to_increment`](crate::Amount::round_to_increment)).
    InvalidIncrement,
    /// A price is negative (BR-27). Direction lives in the quantity.
    NegativePrice {
        /// The offending price.
        price: Decimal,
    },
    /// A price base quantity (BT-149) is not above zero (PEPPOL-EN16931-R121).
    InvalidBaseQuantity,
    /// A currency code is not three ASCII letters.
    InvalidCurrency {
        /// The rejected text.
        input: String,
    },
    /// Two inputs that must share a currency do not.
    CurrencyMismatch {
        /// The currency required.
        expected: Currency,
        /// The currency found.
        found: Currency,
    },
    /// A scale (number of decimals) is out of range or two scales disagree.
    ScaleMismatch,
    /// Split weights are empty, negative, or all zero while the total is not.
    InvalidWeights,
    /// A value is not covered by any tier or key.
    Uncovered {
        /// The uncovered value.
        value: Decimal,
    },
    /// Tiers (or a package size) are invalid; see [`TierError`](crate::TierError).
    InvalidTiers(crate::TierError),
    /// A keyed usage names a key that has no tariff
    /// ([`rate_by_key`](crate::rate_by_key)).
    UnknownKey {
        /// The key, as displayed.
        key: String,
    },
    /// A percentile is outside `1..=100`.
    InvalidPercentile,
    /// A rate is out of range.
    InvalidRate {
        /// The offending rate.
        rate: Decimal,
    },
    /// Some lines or adjustments carry VAT and others do not; a document is
    /// either all-VAT or VAT-free.
    MissingVat,
    /// Two exemption reasons conflict within one VAT group.
    ConflictingExemption,
    /// An adjustment's amount disagrees with its base × percentage
    /// (PEPPOL-EN16931-R040).
    AdjustmentBasis,
    /// A price's base-quantity unit differs from the line's quantity unit
    /// (PEPPOL-EN16931-R130).
    PriceBaseUnit,
    /// A cash-rounding increment is zero, negative or finer than the scale.
    InvalidCashRounding,
    /// VAT accounting is invalid, e.g. its currency is the document currency
    /// (PEPPOL-EN16931-R005).
    InvalidVatAccounting,
    /// A minimum or maximum bound cannot be applied.
    InvalidBound,
    /// A VAT treatment breaks an EN 16931 category rule; see
    /// [`VatError`](crate::VatError).
    InvalidVat(crate::VatError),
    /// A single-valued builder setting was given more than once.
    ConflictingSetting {
        /// The setting, e.g. `"cash_rounding"`.
        setting: &'static str,
    },
    /// A VAT tax partition does not match the groups, or a partitioned tax is
    /// outside BR-CO-17's tolerance.
    InvalidTaxPartition,
    /// An operation needs at least one input and got none.
    Empty,
    /// Documents combined by [`Document::concat`](crate::Document::concat)
    /// differ in a setting that must be equal.
    SettingMismatch {
        /// The setting, e.g. `"cash_rounding"`.
        setting: &'static str,
    },
    /// A reading counted by [`Aggregation::UniqueCount`](crate::Aggregation::UniqueCount)
    /// has no key.
    MissingKey,
    /// A reading's weight is negative where a weighted aggregation uses it.
    NegativeWeight {
        /// The offending weight.
        weight: Decimal,
    },
    /// A [`UnitConversion`](crate::UnitConversion) has a divisor ≤ 0 or more
    /// than 28 decimals.
    InvalidConversion,
    /// A [`Credit`](crate::Credit) balance is not above zero or does not fit
    /// the document's scale.
    InvalidCredit,
    /// An itemised [`Advance`](crate::Advance) matches no VAT group (category
    /// and rate) of the document, so no residual VAT can be stated for it.
    UnmatchedAdvance,
    /// A [`TransactionFee`](crate::TransactionFee) is invalid: a percentage
    /// outside 0–100, a negative fixed fee, minimum or maximum, or a minimum
    /// above the maximum.
    InvalidFee,
    /// A cumulative range is decreasing or negative
    /// ([`Tariff::rate_increment`](crate::Tariff::rate_increment)): `0 ≤ from ≤
    /// to` is required, `from` being 0 for [`Billed::Nothing`](crate::Billed::Nothing).
    InvalidRange {
        /// The cumulative value already billed (its decimal reading; 0 for
        /// `Billed::Nothing`).
        from: Decimal,
        /// The cumulative value billed up to (its decimal reading).
        to: Decimal,
    },
}

impl ErrorKind {
    fn rule(&self) -> Option<&'static str> {
        match self {
            Self::NegativePrice { .. } => Some(rules::BR_27),
            Self::InvalidBaseQuantity => Some(rules::PEPPOL_R121),
            Self::AdjustmentBasis => Some(rules::PEPPOL_R040),
            Self::PriceBaseUnit => Some(rules::PEPPOL_R130),
            Self::InvalidVatAccounting => Some(rules::PEPPOL_R005),
            Self::InvalidVat(e) => e.rule(),
            _ => None,
        }
    }
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Overflow => f.write_str("arithmetic overflow: the result does not fit"),
            Self::ExcessPrecision { value, decimals } => {
                write!(f, "{value} has more than {decimals} decimals")
            }
            Self::Malformed { input } => {
                write!(f, "malformed decimal {input:?}: expected [+-]digits[.digits]")
            }
            Self::DivisionByZero => f.write_str("division by zero"),
            Self::InvalidIncrement => f.write_str("a rounding increment must be above zero"),
            Self::NegativePrice { price } => {
                write!(f, "price {price} is negative (BR-27)")
            }
            Self::InvalidBaseQuantity => {
                f.write_str("price base quantity must be above zero (PEPPOL-EN16931-R121)")
            }
            Self::InvalidCurrency { input } => {
                write!(f, "invalid currency code {input:?}: expected three ASCII letters")
            }
            Self::CurrencyMismatch { expected, found } => {
                write!(f, "currency mismatch: expected {expected}, found {found}")
            }
            Self::ScaleMismatch => f.write_str("scale out of range or mismatched"),
            Self::InvalidWeights => f.write_str(
                "weights must be non-empty and non-negative, and not all zero for a non-zero total",
            ),
            Self::Uncovered { value } => write!(f, "{value} is not covered"),
            Self::InvalidTiers(e) => write!(f, "invalid tiers: {e}"),
            Self::UnknownKey { key } => write!(f, "no tariff for key {key:?}"),
            Self::InvalidPercentile => f.write_str("percentile must be within 1..=100"),
            Self::InvalidRate { rate } => write!(f, "rate {rate} is out of range"),
            Self::MissingVat => {
                f.write_str("vat is missing: a document is either all-vat or vat-free")
            }
            Self::ConflictingExemption => f.write_str("conflicting vat exemption reasons"),
            Self::AdjustmentBasis => f.write_str(
                "adjustment amount differs from base × percentage / 100 (PEPPOL-EN16931-R040)",
            ),
            Self::PriceBaseUnit => f.write_str(
                "price base quantity unit differs from the quantity unit (PEPPOL-EN16931-R130)",
            ),
            Self::InvalidCashRounding => f.write_str("invalid cash rounding increment"),
            Self::InvalidVatAccounting => f.write_str(
                "invalid vat accounting: its currency must differ from the document currency (PEPPOL-EN16931-R005)",
            ),
            Self::InvalidBound => f.write_str("invalid minimum or maximum bound"),
            Self::InvalidVat(e) => write!(f, "invalid vat: {e}"),
            Self::ConflictingSetting { setting } => {
                write!(f, "{setting} was set more than once")
            }
            Self::InvalidTaxPartition => f.write_str(
                "tax partition does not match the vat groups or is outside the BR-CO-17 tolerance",
            ),
            Self::Empty => f.write_str("nothing to operate on: at least one input is required"),
            Self::SettingMismatch { setting } => {
                write!(f, "the documents differ in {setting}")
            }
            Self::MissingKey => f.write_str("the reading has no key to count"),
            Self::NegativeWeight { weight } => write!(f, "weight {weight} is negative"),
            Self::InvalidConversion => f.write_str(
                "a unit conversion needs a divisor above zero and at most 28 decimals",
            ),
            Self::InvalidCredit => f.write_str(
                "a credit balance must be above zero and fit the document's scale",
            ),
            Self::UnmatchedAdvance => {
                f.write_str("the advance matches no vat group (category and rate) of the document")
            }
            Self::InvalidFee => f.write_str(
                "a transaction fee needs a percentage within 0–100, non-negative fixed, minimum and maximum, and a minimum not above the maximum",
            ),
            Self::InvalidRange { from, to } => {
                write!(f, "cumulative range {from} to {to} must satisfy 0 ≤ from ≤ to")
            }
        }
    }
}

/// Where in the input an error was found.
///
/// Every index is **0-based**: the position in the slice (or builder call
/// order) the caller supplied, so `Line(0)` is the first line. `Display`
/// prints the same 0-based index, not an ordinal:
///
/// | location | `Display` |
/// |---|---|
/// | `Line(0)` | `line 0` |
/// | `LineAdjustment { line: 1, index: 0 }` | `line 1 adjustment 0` |
/// | `Adjustment(2)` | `adjustment 2` |
/// | `Tier(3)` | `tier 3` |
/// | `Weight(4)` | `weight 4` |
/// | `Bound(5)` | `bound 5` |
/// | `Payment(6)` | `payment 6` |
/// | `Sample(7)` | `sample 7` |
/// | `Credit(8)` | `credit 8` |
/// | `Transaction(9)` | `transaction 9` |
///
/// Document adjustments are numbered in the order the builder received them,
/// followed by those materialised by
/// [`DocumentBuilder::apportion`](crate::DocumentBuilder::apportion).
/// Bounds are numbered in the order [`DocumentBuilder::bound`](crate::DocumentBuilder::bound)
/// received them; payments by their position in the payment list
/// ([`Document::payments`](crate::Document::payments), builder call order); samples by their
/// position in the readings given to [`Aggregation::apply`](crate::Aggregation::apply);
/// credits in the order [`DocumentBuilder::credit`](crate::DocumentBuilder::credit)
/// received them (their priority); transactions by their position in the
/// amounts given to [`TransactionFee::rate`](crate::TransactionFee::rate).
///
/// `#[non_exhaustive]`: later releases add locations without a breaking
/// change, so a `match` needs a wildcard arm.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Location {
    /// The line at this 0-based index.
    Line(usize),
    /// The `index`-th adjustment of the line at `line`.
    LineAdjustment {
        /// Line index.
        line: usize,
        /// Adjustment index within the line.
        index: usize,
    },
    /// The document-level adjustment at this index.
    Adjustment(usize),
    /// The tier at this index.
    Tier(usize),
    /// The weight at this index.
    Weight(usize),
    /// The minimum or maximum bound at this index, in builder call order.
    Bound(usize),
    /// The [`Payment`](crate::Payment) at this index of the payment list.
    Payment(usize),
    /// The reading at this index of the samples given to an aggregation.
    Sample(usize),
    /// The credit at this index, in builder call order (priority).
    Credit(usize),
    /// The transaction at this index of the amounts given to
    /// [`TransactionFee::rate`](crate::TransactionFee::rate).
    Transaction(usize),
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Line(i) => write!(f, "line {i}"),
            Self::LineAdjustment { line, index } => {
                write!(f, "line {line} adjustment {index}")
            }
            Self::Adjustment(i) => write!(f, "adjustment {i}"),
            Self::Tier(i) => write!(f, "tier {i}"),
            Self::Weight(i) => write!(f, "weight {i}"),
            Self::Bound(i) => write!(f, "bound {i}"),
            Self::Payment(i) => write!(f, "payment {i}"),
            Self::Sample(i) => write!(f, "sample {i}"),
            Self::Credit(i) => write!(f, "credit {i}"),
            Self::Transaction(i) => write!(f, "transaction {i}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    /// The one formatter golden test: every `Location` form, location first.
    /// Every other error test asserts kind, location and rule only.
    #[test]
    fn display_golden() {
        let msg = ErrorKind::Overflow.to_string();
        assert_eq!(msg, "arithmetic overflow: the result does not fit");
        let cases = [
            (Location::Line(0), "line 0"),
            (
                Location::LineAdjustment { line: 1, index: 0 },
                "line 1 adjustment 0",
            ),
            (Location::Adjustment(2), "adjustment 2"),
            (Location::Tier(3), "tier 3"),
            (Location::Weight(4), "weight 4"),
            (Location::Bound(0), "bound 0"),
            (Location::Payment(0), "payment 0"),
            (Location::Bound(5), "bound 5"),
            (Location::Payment(6), "payment 6"),
            (Location::Sample(7), "sample 7"),
            (Location::Credit(8), "credit 8"),
            (Location::Transaction(9), "transaction 9"),
        ];
        for (at, shown) in cases {
            let err = Error::from(ErrorKind::Overflow).at(at);
            assert_eq!(err.to_string(), alloc::format!("{shown}: {msg}"));
            assert_eq!(err.location(), Some(&at));
        }
        assert_eq!(Error::from(ErrorKind::Overflow).to_string(), msg);
    }

    #[test]
    fn rules_map_to_cited_ids() {
        let rule = |k: ErrorKind| Error::from(k).rule();
        assert_eq!(
            rule(ErrorKind::InvalidBaseQuantity),
            Some("PEPPOL-EN16931-R121")
        );
        assert_eq!(
            rule(ErrorKind::AdjustmentBasis),
            Some("PEPPOL-EN16931-R040")
        );
        assert_eq!(rule(ErrorKind::PriceBaseUnit), Some("PEPPOL-EN16931-R130"));
        assert_eq!(
            rule(ErrorKind::InvalidVatAccounting),
            Some("PEPPOL-EN16931-R005")
        );
        assert_eq!(rule(ErrorKind::Overflow), None);
        // Stricter than BR-CO-17's ±1.00 by design, so it cites no rule: BR-CO-17
        // would accept the refused input (README "EN 16931 rule map").
        assert_eq!(rule(ErrorKind::InvalidTaxPartition), None);
        assert_eq!(rule(ErrorKind::InvalidBound), None);
        assert_eq!(rule(ErrorKind::MissingKey), None);
        assert_eq!(
            rule(ErrorKind::NegativeWeight {
                weight: Decimal::NEGATIVE_ONE
            }),
            None
        );
        assert_eq!(rule(ErrorKind::InvalidConversion), None);
        assert_eq!(rule(ErrorKind::InvalidCredit), None);
        assert_eq!(rule(ErrorKind::UnmatchedAdvance), None);
        assert_eq!(rule(ErrorKind::InvalidFee), None);
        assert_eq!(
            rule(ErrorKind::InvalidRange {
                from: Decimal::ONE,
                to: Decimal::ZERO
            }),
            None
        );
        let missing = Error::from(ErrorKind::MissingVat);
        assert_eq!(
            missing.clone().at(Location::Line(3)).rule(),
            Some("BR-CO-04")
        );
        assert_eq!(missing.clone().at(Location::Adjustment(0)).rule(), None);
        let adj = missing.clone().at(Location::Adjustment(0));
        assert_eq!(adj.clone().with_rule(rules::BR_32).rule(), Some("BR-32"));
        assert_eq!(adj.with_rule(rules::BR_37).rule(), Some("BR-37"));
        assert_eq!(missing.rule(), None);
    }

    #[test]
    fn every_message_starts_lowercase() {
        let kinds = [
            ErrorKind::Overflow,
            ErrorKind::DivisionByZero,
            ErrorKind::InvalidIncrement,
            ErrorKind::InvalidBaseQuantity,
            ErrorKind::ScaleMismatch,
            ErrorKind::InvalidWeights,
            ErrorKind::InvalidPercentile,
            ErrorKind::MissingVat,
            ErrorKind::ConflictingExemption,
            ErrorKind::AdjustmentBasis,
            ErrorKind::PriceBaseUnit,
            ErrorKind::InvalidCashRounding,
            ErrorKind::InvalidVatAccounting,
            ErrorKind::InvalidBound,
            ErrorKind::MissingKey,
            ErrorKind::NegativeWeight {
                weight: Decimal::NEGATIVE_ONE,
            },
            ErrorKind::InvalidConversion,
            ErrorKind::InvalidCredit,
            ErrorKind::UnmatchedAdvance,
            ErrorKind::InvalidFee,
            ErrorKind::InvalidRange {
                from: Decimal::ONE,
                to: Decimal::ZERO,
            },
            ErrorKind::Malformed { input: "x".into() },
            ErrorKind::InvalidCurrency { input: "x".into() },
            ErrorKind::CurrencyMismatch {
                expected: Currency::EUR,
                found: Currency::USD,
            },
        ];
        for k in kinds {
            let s = k.to_string();
            assert!(s.starts_with(|c: char| c.is_ascii_lowercase()), "{s}");
        }
    }
}
