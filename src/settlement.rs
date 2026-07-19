//! [`CashRounding`] — rounding the payable total to the smallest coin in circulation.
//!
//! # What this models
//!
//! Many jurisdictions withdrew their smallest coins and round the amount actually
//! *tendered* to a coarser step: 0.05 in Switzerland, Belgium, Ireland, Italy,
//! Canada and Australia; 0.10 in New Zealand; 0.50 in Denmark; 1.00 in Sweden
//! and Norway.
//!
//! Three properties are consistent across every one of those regimes, and this
//! type is shaped by them:
//!
//! 1. **It applies to the gross total, after tax** — never to line items, never
//!    before VAT. Denmark's guidance forbids pre-VAT rounding outright.
//! 2. **The difference is not taxable.** VAT is computed on the exact
//!    pre-rounding consideration; the delta is a settlement item. (Switzerland
//!    is the one jurisdiction that folds it into the consideration — model that
//!    by adjusting the positions instead of using this type.)
//! 3. **It is a property of the tender, not the invoice.** It applies to cash
//!    only; card and transfer settle to the exact minor unit. Rounding a card
//!    payment is affirmatively unlawful in Denmark.
//!
//! The resulting difference is EN 16931 **BT-114** (rounding amount), which
//! feeds the amount-due identity BR-CO-16:
//!
//! ```text
//! BT-115 (amount due) = BT-112 (gross) − BT-113 (prepaid) + BT-114 (rounding)
//! ```
//!
//! # No jurisdiction is assumed
//!
//! There is no `CashRounding::for_currency` and no default increment, because
//! the increment is a *payment-law* fact rather than a currency fact: CHF has
//! two ISO minor units but rounds cash to 0.05, and EUR rounds to 0.05 in
//! Belgium while not rounding at all in Germany. The caller states the rule.
//!
//! ```rust
//! use billing::{Amount, CashRounding, RoundingStrategy};
//!
//! // Swiss Rappenrundung: nearest 0.05, commercial rounding.
//! let chf = CashRounding::new(
//!     Amount::<5>::parse("0.05000").unwrap(),
//!     RoundingStrategy::MidpointAwayFromZero,
//! ).unwrap();
//!
//! let gross = Amount::<5>::parse("12.34000").unwrap();
//! assert_eq!(chf.round(gross).unwrap(),      Amount::parse("12.35000").unwrap());
//! assert_eq!(chf.difference(gross).unwrap(), Amount::parse("0.01000").unwrap());
//! ```

use crate::amount::{Amount, RoundingStrategy};
use crate::error::BillingError;

/// A cash-rounding rule: an increment plus the strategy used at the midpoint.
///
/// See the [module documentation](self) for the legal background and for why no
/// per-currency default is provided.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "CashRoundingRepr"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashRounding {
    increment: Amount<5>,
    strategy: RoundingStrategy,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct CashRoundingRepr {
    increment: Amount<5>,
    strategy: RoundingStrategy,
}

#[cfg(feature = "serde")]
impl TryFrom<CashRoundingRepr> for CashRounding {
    type Error = BillingError;
    fn try_from(r: CashRoundingRepr) -> Result<Self, Self::Error> {
        Self::new(r.increment, r.strategy)
    }
}

impl CashRounding {
    /// Create a cash-rounding rule.
    ///
    /// # Choosing a strategy
    ///
    /// The midpoint rule genuinely differs between jurisdictions and is not
    /// always settled even within one: Norway legislates 0.50 → up, Finland's
    /// 5-cent step makes midpoints unreachable, Denmark's "nearest multiple of
    /// 50" leaves 0.25 and 0.75 undefined in the statute, and New Zealand leaves
    /// the 5-cent case to retailer discretion. Hence it is a parameter.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `increment` is not strictly positive.
    pub fn new(increment: Amount<5>, strategy: RoundingStrategy) -> Result<Self, BillingError> {
        if !increment.is_positive() {
            return Err(BillingError::InvalidInput {
                reason: format!("cash-rounding increment must be > 0, got {increment}"),
            });
        }
        Ok(Self {
            increment,
            strategy,
        })
    }

    /// The rounding increment (e.g. `0.05`).
    pub fn increment(&self) -> Amount<5> {
        self.increment
    }

    /// The midpoint strategy.
    #[must_use]
    pub fn strategy(&self) -> RoundingStrategy {
        self.strategy
    }

    /// Round `amount` to the nearest multiple of the increment.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] for amounts near the representable limit.
    pub fn round(&self, amount: Amount<5>) -> Result<Amount<5>, BillingError> {
        amount.round_to_increment(self.increment, self.strategy)
    }

    /// The adjustment to add to `amount` to reach the rounded figure — EN 16931
    /// **BT-114**.
    ///
    /// Positive when rounding up, negative when rounding down, zero when the
    /// amount is already a multiple of the increment.
    ///
    /// ```rust
    /// use billing::{Amount, CashRounding, RoundingStrategy};
    /// let r = CashRounding::new(
    ///     Amount::<5>::parse("0.05000").unwrap(),
    ///     RoundingStrategy::MidpointAwayFromZero,
    /// ).unwrap();
    /// // 12.32 rounds down to 12.30 → BT-114 is −0.02
    /// assert_eq!(
    ///     r.difference(Amount::parse("12.32000").unwrap()).unwrap(),
    ///     Amount::parse("-0.02000").unwrap()
    /// );
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] for amounts near the representable limit.
    pub fn difference(&self, amount: Amount<5>) -> Result<Amount<5>, BillingError> {
        self.round(amount)?.checked_sub(amount)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chf() -> CashRounding {
        CashRounding::new(
            Amount::<5>::parse("0.05000").unwrap(),
            RoundingStrategy::MidpointAwayFromZero,
        )
        .unwrap()
    }

    #[test]
    fn rejects_non_positive_increment() {
        assert!(CashRounding::new(Amount::ZERO, RoundingStrategy::Floor).is_err());
        assert!(
            CashRounding::new(Amount::parse("-0.05000").unwrap(), RoundingStrategy::Floor).is_err()
        );
    }

    #[test]
    fn swiss_five_rappen_table() {
        let r = chf();
        for (input, expected) in [
            ("12.30000", "12.30000"), // exact multiple
            ("12.31000", "12.30000"), // .01 → down
            ("12.32000", "12.30000"), // .02 → down
            ("12.33000", "12.35000"), // .03 → up
            ("12.34000", "12.35000"), // .04 → up
            ("12.35000", "12.35000"),
        ] {
            assert_eq!(
                r.round(Amount::parse(input).unwrap()).unwrap(),
                Amount::parse(expected).unwrap(),
                "rounding {input}"
            );
        }
    }

    #[test]
    fn midpoint_goes_away_from_zero_on_both_signs() {
        let r = chf();
        // 12.025 is exactly between 12.00 and 12.05.
        assert_eq!(
            r.round(Amount::parse("12.02500").unwrap()).unwrap(),
            Amount::parse("12.05000").unwrap()
        );
        assert_eq!(
            r.round(Amount::parse("-12.02500").unwrap()).unwrap(),
            Amount::parse("-12.05000").unwrap()
        );
    }

    #[test]
    fn difference_is_signed_and_sums_back() {
        let r = chf();
        for input in ["12.31000", "12.34000", "12.35000", "-12.31000", "0.00000"] {
            let a = Amount::parse(input).unwrap();
            assert_eq!(
                a.checked_add(r.difference(a).unwrap()).unwrap(),
                r.round(a).unwrap(),
                "amount + difference must equal the rounded value for {input}"
            );
        }
    }

    #[test]
    fn whole_krona_rounding() {
        // Sweden / Norway: round to 1.00
        let r = CashRounding::new(
            Amount::<5>::parse("1.00000").unwrap(),
            RoundingStrategy::MidpointAwayFromZero,
        )
        .unwrap();
        assert_eq!(
            r.round(Amount::parse("30.40000").unwrap()).unwrap(),
            Amount::parse("30.00000").unwrap()
        );
        assert_eq!(
            r.round(Amount::parse("45.60000").unwrap()).unwrap(),
            Amount::parse("46.00000").unwrap()
        );
    }

    #[test]
    fn floor_and_ceiling_strategies() {
        let inc = Amount::<5>::parse("0.05000").unwrap();
        let floor = CashRounding::new(inc, RoundingStrategy::Floor).unwrap();
        let ceil = CashRounding::new(inc, RoundingStrategy::Ceiling).unwrap();
        let a = Amount::parse("12.34000").unwrap();
        assert_eq!(floor.round(a).unwrap(), Amount::parse("12.30000").unwrap());
        assert_eq!(ceil.round(a).unwrap(), Amount::parse("12.35000").unwrap());
    }
}
