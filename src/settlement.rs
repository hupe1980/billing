//! Settlement-level terms: [`CashRounding`] (EN 16931 **BT-114**) and
//! [`VatAccountingCurrency`] (**BT-6** / **BT-111**).
//!
//! Both sit *after* the totals chain and describe how the document is settled and
//! reported rather than what was supplied, so neither moves the taxable base.
//!
//! # Cash rounding — what this models
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

use crate::amount::{Amount, AmountScale, RoundingStrategy};
use crate::currency::Currency;
use crate::error::BillingError;
use rust_decimal::Decimal;

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

// ── VatAccountingCurrency ─────────────────────────────────────────────────────

/// EN 16931 **BT-6** and **BT-111** — the VAT total restated in the currency the
/// seller accounts for VAT in.
///
/// # Why an invoice carries two VAT totals
///
/// The invoice currency (BT-5) is a commercial choice; the currency VAT is
/// *reported* in is not. Art. 230 of the VAT Directive lets an invoice be
/// denominated in any currency but requires the VAT amount to be expressed in the
/// national currency of the member state where the tax is due. So a Polish, Czech,
/// Swedish, Danish, Hungarian or Romanian supplier invoicing in EUR must state the
/// VAT again in PLN, CZK, SEK, DKK, HUF or RON — the same tax, converted.
///
/// EN 16931 models exactly that pair and nothing more: BT-6 names the currency,
/// BT-111 states the amount. **The exchange rate is not a business term.** The
/// standard carries the converted figure, not the arithmetic that produced it,
/// which is why [`converted`](Self::converted) takes the rate as an argument and
/// keeps none of it.
///
/// # The rules this satisfies
///
/// | Rule | Source | Requires |
/// |---|---|---|
/// | **BR-53** | CEN, fatal | BT-6 present ⇒ BT-111 provided — they are one value here, so the pair cannot be half-stated |
/// | **BR-DEC-15** | CEN, fatal | BT-111 at most two decimals — [`converted`](Self::converted) rounds once at the target scale, and [`fits_amount_scale`](crate::BillingDocument::fits_amount_scale) checks it |
/// | **`PEPPOL-EN16931-R005`** | Peppol, fatal | BT-6 ≠ BT-5 — checked by [`with_vat_accounting_currency`](crate::BillingDocument::with_vat_accounting_currency), which is the first place both are known |
/// | **`PEPPOL-EN16931-R055`** | Peppol, fatal | BT-110 and BT-111 share an operational sign — checked by [`validate`](crate::BillingDocument::validate), so a reversal or an allocation cannot break it silently |
///
/// ```rust
/// use billing::{Amount, AmountScale, Currency, VatAccountingCurrency};
/// use rust_decimal::dec;
///
/// // 190,00 € of VAT reported in złoty at 4.30 PLN/EUR.
/// let bt111 = VatAccountingCurrency::converted(
///     Currency::new("PLN")?,
///     Amount::parse("190.00000")?,
///     dec!(4.30),
///     AmountScale::EN16931,
/// )?;
///
/// assert_eq!(bt111.currency(), Currency::new("PLN")?);   // BT-6
/// assert_eq!(bt111.vat_total(), Amount::parse("817.00000")?); // BT-111
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "VatAccountingCurrencyRepr"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VatAccountingCurrency {
    currency: Currency,
    vat_total: Amount<5>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct VatAccountingCurrencyRepr {
    currency: Currency,
    vat_total: Amount<5>,
}

#[cfg(feature = "serde")]
impl TryFrom<VatAccountingCurrencyRepr> for VatAccountingCurrency {
    type Error = BillingError;
    fn try_from(r: VatAccountingCurrencyRepr) -> Result<Self, Self::Error> {
        Self::new(r.currency, r.vat_total)
    }
}

impl VatAccountingCurrency {
    /// State BT-111 directly, in the currency BT-6 names.
    ///
    /// Use when the converted figure comes from elsewhere — an ERP that already
    /// applied the official rate, or a rate this crate has no business holding.
    /// [`converted`](Self::converted) does the arithmetic instead.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `currency` is [`Currency::XXX`]. BT-6
    /// names the currency a tax authority is reported in; "no currency involved"
    /// is not one, and it would pass `BR-CL-04` unnoticed.
    pub fn new(currency: Currency, vat_total: Amount<5>) -> Result<Self, BillingError> {
        if currency.is_unset() {
            return Err(BillingError::InvalidInput {
                reason: "VAT accounting currency (BT-6) must be a real currency, not XXX".into(),
            });
        }
        Ok(Self {
            currency,
            vat_total,
        })
    }

    /// Convert a VAT total at `rate`, rounding **once** at `scale`.
    ///
    /// `rate` is units of `currency` per unit of the invoice currency: converting
    /// EUR to PLN at 4.30 means `dec!(4.30)`.
    ///
    /// The single rounding is the point. BT-111 is capped at two decimals by
    /// **BR-DEC-15**, and computing the product at the engine's five decimals and
    /// reducing afterwards rounds twice — which can land a minor unit away from the
    /// figure a tax authority recomputes from the same rate. Passing
    /// [`AmountScale::EN16931`] rounds the exact product straight onto two decimals.
    ///
    /// # Errors
    /// - [`BillingError::InvalidInput`] if `currency` is [`Currency::XXX`] or `rate`
    ///   is not strictly positive — a non-positive rate would flip or erase the sign
    ///   that `PEPPOL-EN16931-R055` requires to match BT-110.
    /// - [`BillingError::MonetaryOverflow`] if the converted amount leaves range.
    pub fn converted(
        currency: Currency,
        vat_total: Amount<5>,
        rate: Decimal,
        scale: AmountScale,
    ) -> Result<Self, BillingError> {
        if rate <= Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: format!("VAT accounting currency exchange rate must be > 0, got {rate}"),
            });
        }
        let exact =
            vat_total
                .into_decimal()
                .checked_mul(rate)
                .ok_or(BillingError::MonetaryOverflow {
                    precision: 5,
                    input_value: None,
                })?;
        Self::new(currency, scale.apply_decimal(exact)?)
    }

    /// EN 16931 **BT-6** — the currency VAT is accounted for in.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.currency
    }

    /// EN 16931 **BT-111** — the VAT total in that currency.
    pub fn vat_total(&self) -> Amount<5> {
        self.vat_total
    }

    /// The same currency carrying a different BT-111 — for the transforms that move
    /// the VAT total with it.
    pub(crate) fn with_vat_total(self, vat_total: Amount<5>) -> Self {
        Self { vat_total, ..self }
    }

    /// Negate BT-111, for [`crate::BillingDocument::reverse`].
    pub(crate) fn negated(self) -> Result<Self, BillingError> {
        Ok(self.with_vat_total(self.vat_total.checked_neg()?))
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

    #[test]
    fn a_cash_rounding_rule_reports_the_increment_and_strategy_it_was_built_with() {
        // BT-114 is derived from these two, and a document carrying a rounding
        // rule re-checks its own remainder against them on `validate`. Reading
        // back a zero increment would make every amount look already-rounded.
        let inc = Amount::<5>::parse("0.05000").unwrap();
        let rule = CashRounding::new(inc, RoundingStrategy::MidpointAwayFromZero).unwrap();
        assert_eq!(rule.increment(), inc);
        assert_ne!(rule.increment(), Amount::<5>::default());
        assert_eq!(rule.strategy(), RoundingStrategy::MidpointAwayFromZero);

        // A different increment reads back differently — Swedish krona rounds to
        // the whole unit, Swiss francs to five centimes.
        let krona = CashRounding::new(
            Amount::<5>::parse("1.00000").unwrap(),
            RoundingStrategy::MidpointToEven,
        )
        .unwrap();
        assert_eq!(krona.increment(), Amount::<5>::parse("1.00000").unwrap());
        assert_eq!(krona.strategy(), RoundingStrategy::MidpointToEven);
        assert_ne!(krona.increment(), rule.increment());
    }
}
