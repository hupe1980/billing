//! [`PaymentTerms`] and [`EarlyPaymentDiscount`] — EN 16931 **BT-20**, and the
//! structured early-payment-discount (Skonto) convention carried inside it.
//!
//! # What BT-20 is, and why it needs help
//!
//! EN 16931 models payment terms as **one free-text field**:
//!
//! > **BT-20** *Payment terms* — A textual description of the payment terms that
//! > apply to the amount due for payment (Including description of possible
//! > penalties).
//!
//! An early-payment discount — *Skonto*, *escompte*, *sconto*, "2/10 net 30" — is
//! therefore not machine-readable in the core standard at all, even though it
//! changes what the buyer actually pays. Germany's CIUS closes the gap with a
//! micro-syntax embedded in that free text, made mandatory by **BR-DE-18**:
//!
//! ```text
//! #SKONTO#TAGE=10#PROZENT=2.00#
//! #SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#
//! ```
//!
//! Uppercase, no whitespace, `#` opening every segment and closing the line, the
//! percentage with exactly two decimals, one line per discount. This module is that
//! syntax as a type: [`Display`](std::fmt::Display) writes it, [`FromStr`] reads it
//! back, and neither can produce a form a validator rejects.
//!
//! The revision of EN 16931 approved by CEN in 2025 (published 2026) promotes early
//! payment discounts and late-payment charges to structured fields of their own. The
//! *arithmetic* below does not change with it — only where the numbers are written —
//! so this type stays useful either way, and its accessors are what a future
//! serialiser maps from.
//!
//! # The discount does not belong in the totals
//!
//! A Skonto is **conditional**: it exists only if the buyer pays early, which has
//! not happened when the invoice is issued. Deducting it up front would understate
//! the taxable base for a supply that may well be paid at full price.
//!
//! When it *is* taken, the consideration shrinks after the fact, and the VAT follows
//! it — §17 Abs. 1 UStG in Germany, art. 90 of the VAT Directive elsewhere. That is a
//! correction triggered by a **payment event**, not by anything the invoice knows, so
//! this module deliberately computes the payable figure and stops there:
//! [`BillingDocument`](crate::BillingDocument) totals and the VAT breakdown are never
//! touched by attaching payment terms.
//!
//! ```rust
//! use billing::{Amount, EarlyPaymentDiscount, PaymentTerms};
//! use rust_decimal::dec;
//!
//! let skonto = EarlyPaymentDiscount::new(10, dec!(2.00))?;
//! let terms = PaymentTerms::text("Zahlbar innerhalb 30 Tagen ohne Abzug.")
//!     .with_discount(skonto);
//!
//! // What the buyer pays if they take it — the invoice still states the full amount.
//! assert_eq!(skonto.payable_on(Amount::parse("1190.00000")?)?, Amount::parse("1166.20000")?);
//!
//! // …and how it goes into BT-20.
//! assert_eq!(
//!     terms.to_string(),
//!     "Zahlbar innerhalb 30 Tagen ohne Abzug.\n#SKONTO#TAGE=10#PROZENT=2.00#",
//! );
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::str::FromStr;

use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::error::BillingError;

/// The number of decimals the BT-20 Skonto syntax writes for a percentage and for
/// a base amount. Fixed by the convention, not a preference.
const SEGMENT_DECIMALS: u32 = 2;

// ── EarlyPaymentDiscount ──────────────────────────────────────────────────────

/// One early-payment discount: *pay within `days` days and deduct `percent` %*.
///
/// See the [module documentation](self) for the BT-20 syntax and for why the
/// discount stays out of the document totals.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "EarlyPaymentDiscountRepr"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EarlyPaymentDiscount {
    days: u32,
    percent: Decimal,
    base_amount: Option<Amount<5>>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct EarlyPaymentDiscountRepr {
    days: u32,
    percent: Decimal,
    #[serde(default)]
    base_amount: Option<Amount<5>>,
}

#[cfg(feature = "serde")]
impl TryFrom<EarlyPaymentDiscountRepr> for EarlyPaymentDiscount {
    type Error = BillingError;
    fn try_from(r: EarlyPaymentDiscountRepr) -> Result<Self, Self::Error> {
        let d = Self::new(r.days, r.percent)?;
        match r.base_amount {
            Some(b) => d.with_base_amount(b),
            None => Ok(d),
        }
    }
}

impl EarlyPaymentDiscount {
    /// Create a discount of `percent` % for payment within `days` days.
    ///
    /// `percent` is a **percentage** (`2.00` for 2 %), matching the wire syntax —
    /// not the fraction the tax and discount layers work in. It must carry at most
    /// two decimals, because the syntax writes exactly two and a third would be
    /// silently dropped.
    ///
    /// ```rust
    /// use billing::EarlyPaymentDiscount;
    /// use rust_decimal::dec;
    ///
    /// assert!(EarlyPaymentDiscount::new(14, dec!(3)).is_ok());
    /// assert!(EarlyPaymentDiscount::new(0, dec!(2.5)).is_ok());   // "on receipt"
    /// assert!(EarlyPaymentDiscount::new(14, dec!(0)).is_err());   // not a discount
    /// assert!(EarlyPaymentDiscount::new(14, dec!(120)).is_err()); // more than the invoice
    /// assert!(EarlyPaymentDiscount::new(14, dec!(2.005)).is_err()); // unwritable
    /// ```
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `percent` is outside `(0, 100]` or needs
    /// more than two decimals.
    pub fn new(days: u32, percent: Decimal) -> Result<Self, BillingError> {
        if percent <= Decimal::ZERO || percent > Decimal::ONE_HUNDRED {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "early payment discount percentage must be in (0, 100], got {percent}"
                ),
            });
        }
        if percent.normalize().scale() > SEGMENT_DECIMALS {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "early payment discount percentage {percent} needs more than \
                     {SEGMENT_DECIMALS} decimals, which the BT-20 syntax cannot state"
                ),
            });
        }
        Ok(Self {
            days,
            percent,
            base_amount: None,
        })
    }

    /// State an explicit base (`BASISBETRAG`) the percentage applies to.
    ///
    /// Only needed when the discount does **not** apply to the whole amount due —
    /// a part-discountable invoice, or one where freight is excluded. Omit it and
    /// [`discount_on`](Self::discount_on) uses the amount it is handed.
    ///
    /// ```rust
    /// use billing::{Amount, EarlyPaymentDiscount};
    /// use rust_decimal::dec;
    ///
    /// let d = EarlyPaymentDiscount::new(10, dec!(2.50))?
    ///     .with_base_amount(Amount::parse("485.98000")?)?;
    /// assert_eq!(d.to_string(), "#SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#");
    /// // The stated base wins over the amount due: 2,50 % of 485,98, not of 1000,00.
    /// assert_eq!(d.discount_on(Amount::parse("1000.00000")?)?, Amount::parse("12.14950")?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `base` is negative or does not fit the two
    /// decimals the syntax writes — see [`Amount::fits_scale`].
    pub fn with_base_amount(mut self, base: Amount<5>) -> Result<Self, BillingError> {
        if base.is_negative() {
            return Err(BillingError::InvalidInput {
                reason: format!("early payment discount base amount must be >= 0, got {base}"),
            });
        }
        if !base.fits_scale(SEGMENT_DECIMALS as u8) {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "early payment discount base amount {base} does not fit \
                     {SEGMENT_DECIMALS} decimals, which the BT-20 syntax cannot state"
                ),
            });
        }
        self.base_amount = Some(base);
        Ok(self)
    }

    /// The payment window in days (`TAGE`).
    #[must_use]
    pub fn days(&self) -> u32 {
        self.days
    }

    /// The discount as a **percentage** (`PROZENT`) — `2.00`, not `0.02`.
    #[must_use]
    pub fn percent(&self) -> Decimal {
        self.percent
    }

    /// The discount as the **fraction** the rest of the crate works in — `0.02`.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] is not reachable for a percentage in
    /// `(0, 100]`; the signature is fallible only because `Decimal`'s division is.
    pub fn rate(&self) -> Result<Decimal, BillingError> {
        self.percent
            .checked_div(Decimal::ONE_HUNDRED)
            .ok_or(BillingError::MonetaryOverflow {
                precision: 5,
                input_value: Some(self.percent),
            })
    }

    /// The explicit base (`BASISBETRAG`), if one was stated.
    #[must_use]
    pub fn base_amount(&self) -> Option<Amount<5>> {
        self.base_amount
    }

    /// The amount deducted if the discount is taken.
    ///
    /// `base × percent ÷ 100`, where `base` is [`base_amount`](Self::base_amount)
    /// when stated and `amount_due` otherwise. Rounded to five decimals half away
    /// from zero, the same commercial rounding every percentage layer in this crate
    /// uses.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn discount_on(&self, amount_due: Amount<5>) -> Result<Amount<5>, BillingError> {
        self.base_amount
            .unwrap_or(amount_due)
            .checked_mul_qty(self.rate()?)
    }

    /// What the buyer pays if the discount is taken — `amount_due` less
    /// [`discount_on`](Self::discount_on).
    ///
    /// This is a *payment* figure. It is deliberately not EN 16931's BT-115, which
    /// states the amount due at full price; see the [module documentation](self).
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn payable_on(&self, amount_due: Amount<5>) -> Result<Amount<5>, BillingError> {
        amount_due.checked_sub(self.discount_on(amount_due)?)
    }
}

impl fmt::Display for EarlyPaymentDiscount {
    /// The BT-20 segment line — uppercase, no whitespace, two decimals.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "#SKONTO#TAGE={}#PROZENT={:.prec$}#",
            self.days,
            self.percent,
            prec = SEGMENT_DECIMALS as usize
        )?;
        if let Some(base) = self.base_amount {
            write!(
                f,
                "BASISBETRAG={:.prec$}#",
                base.into_decimal(),
                prec = SEGMENT_DECIMALS as usize
            )?;
        }
        Ok(())
    }
}

impl FromStr for EarlyPaymentDiscount {
    type Err = BillingError;

    /// Parse one `#SKONTO#…#` line.
    ///
    /// Strict by design: the syntax forbids whitespace and lowercase, and a
    /// validator enforcing BR-DE-18 will reject what this refuses. Accepting a
    /// sloppy form here would let it round-trip into an invoice that fails.
    ///
    /// ```rust
    /// use billing::EarlyPaymentDiscount;
    /// use rust_decimal::dec;
    ///
    /// let d: EarlyPaymentDiscount = "#SKONTO#TAGE=10#PROZENT=2.00#".parse()?;
    /// assert_eq!(d.days(), 10);
    /// assert_eq!(d.percent(), dec!(2.00));
    ///
    /// assert!("#skonto#TAGE=10#PROZENT=2.00#".parse::<EarlyPaymentDiscount>().is_err());
    /// assert!("#SKONTO#TAGE=10#".parse::<EarlyPaymentDiscount>().is_err());
    /// # Ok::<(), billing::BillingError>(())
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = |what: &str| BillingError::InvalidInput {
            reason: format!("{s:?} is not a BT-20 Skonto segment: {what}"),
        };
        let body = s
            .strip_prefix("#SKONTO#")
            .and_then(|b| b.strip_suffix('#'))
            .ok_or_else(|| bad("expected #SKONTO#…# with no surrounding whitespace"))?;

        let mut days = None;
        let mut percent = None;
        let mut base = None;
        for segment in body.split('#') {
            let (key, value) = segment
                .split_once('=')
                .ok_or_else(|| bad(&format!("segment {segment:?} is not KEY=VALUE")))?;
            match key {
                "TAGE" => {
                    days = Some(value.parse::<u32>().map_err(|_| {
                        bad(&format!("TAGE={value:?} is not a whole number of days"))
                    })?);
                }
                "PROZENT" => {
                    percent = Some(
                        Decimal::from_str_exact(value)
                            .map_err(|_| bad(&format!("PROZENT={value:?} is not a decimal")))?,
                    );
                }
                "BASISBETRAG" => {
                    base =
                        Some(Amount::<5>::parse(value).map_err(|_| {
                            bad(&format!("BASISBETRAG={value:?} is not an amount"))
                        })?);
                }
                other => return Err(bad(&format!("unknown segment key {other:?}"))),
            }
        }

        let discount = Self::new(
            days.ok_or_else(|| bad("no TAGE segment"))?,
            percent.ok_or_else(|| bad("no PROZENT segment"))?,
        )?;
        match base {
            Some(b) => discount.with_base_amount(b),
            None => Ok(discount),
        }
    }
}

// ── PaymentTerms ──────────────────────────────────────────────────────────────

/// EN 16931 **BT-20** — free-text payment terms, plus any structured early-payment
/// discounts embedded in them.
///
/// Round-trips: [`Display`](std::fmt::Display) renders the field exactly as it goes
/// on the wire, and [`FromStr`] recovers both halves from a field written by anyone
/// else.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct PaymentTerms {
    text: Option<String>,
    discounts: Vec<EarlyPaymentDiscount>,
}

impl PaymentTerms {
    /// Free-text terms with no structured discount.
    ///
    /// A blank or whitespace-only string is treated as **no prose**, so it cannot
    /// produce a BT-20 field that begins with an empty line — which [`FromStr`]
    /// would then drop, breaking the round trip. It also keeps
    /// [`is_empty`](Self::is_empty) honest.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            text: (!text.trim().is_empty()).then_some(text),
            discounts: Vec::new(),
        }
    }

    /// Add an early-payment discount. Repeatable — staged terms ("3 % in 10 days,
    /// 2 % in 20, net 30") are several segments, one per line.
    #[must_use]
    pub fn with_discount(mut self, discount: EarlyPaymentDiscount) -> Self {
        self.discounts.push(discount);
        self
    }

    /// The free-text part, if any.
    #[must_use]
    pub fn prose(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// The structured discounts, in declaration order.
    #[must_use]
    pub fn discounts(&self) -> &[EarlyPaymentDiscount] {
        &self.discounts
    }

    /// Whether this carries neither prose nor a discount.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.is_none() && self.discounts.is_empty()
    }

    /// The discount applicable to a payment made within `days` days, if any.
    ///
    /// Returns the **most favourable** qualifying discount, which is what staged
    /// terms mean: paying on day 5 under "3 % in 10 days, 2 % in 20" earns 3 %, not
    /// 2 %. Ties keep the first declared.
    ///
    /// ```rust
    /// use billing::{EarlyPaymentDiscount, PaymentTerms};
    /// use rust_decimal::dec;
    ///
    /// let terms = PaymentTerms::default()
    ///     .with_discount(EarlyPaymentDiscount::new(10, dec!(3))?)
    ///     .with_discount(EarlyPaymentDiscount::new(20, dec!(2))?);
    ///
    /// assert_eq!(terms.discount_within(5).map(|d| d.percent()),  Some(dec!(3)));
    /// assert_eq!(terms.discount_within(15).map(|d| d.percent()), Some(dec!(2)));
    /// assert_eq!(terms.discount_within(30).map(|d| d.percent()), None);
    /// # Ok::<(), billing::BillingError>(())
    /// ```
    #[must_use]
    pub fn discount_within(&self, days: u32) -> Option<&EarlyPaymentDiscount> {
        // `max_by` keeps the *last* maximum on a tie; fold keeps the first, which
        // is what "ties keep the first declared" means.
        self.discounts.iter().filter(|d| days <= d.days).fold(
            None,
            |best: Option<&EarlyPaymentDiscount>, d| match best {
                Some(b) if b.percent >= d.percent => Some(b),
                _ => Some(d),
            },
        )
    }
}

impl fmt::Display for PaymentTerms {
    /// The BT-20 field: the prose first, then one `#SKONTO#…#` line per discount.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        if let Some(text) = &self.text {
            f.write_str(text)?;
            first = false;
        }
        for d in &self.discounts {
            if !first {
                f.write_str("\n")?;
            }
            write!(f, "{d}")?;
            first = false;
        }
        Ok(())
    }
}

impl FromStr for PaymentTerms {
    type Err = BillingError;

    /// Split a BT-20 field into prose and structured discounts.
    ///
    /// Every line beginning `#SKONTO#` is parsed as a segment; everything else is
    /// kept verbatim as prose, in order, so nothing a supplier wrote is lost.
    ///
    /// ```rust
    /// use billing::PaymentTerms;
    ///
    /// let terms: PaymentTerms =
    ///     "Zahlbar in 30 Tagen.\n#SKONTO#TAGE=10#PROZENT=2.00#".parse()?;
    /// assert_eq!(terms.prose(), Some("Zahlbar in 30 Tagen."));
    /// assert_eq!(terms.discounts().len(), 1);
    /// # Ok::<(), billing::BillingError>(())
    /// ```
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut prose: Vec<&str> = Vec::new();
        let mut discounts = Vec::new();
        for line in s.split('\n') {
            let trimmed = line.trim_end_matches('\r');
            if trimmed.starts_with("#SKONTO#") {
                discounts.push(trimmed.parse::<EarlyPaymentDiscount>()?);
            } else {
                prose.push(trimmed);
            }
        }
        // Drop the blank lines a segment-only field leaves behind, but keep any
        // internal structure of genuine prose.
        while prose.last().is_some_and(|l| l.trim().is_empty()) {
            prose.pop();
        }
        let text = (!prose.is_empty()).then(|| prose.join("\n"));
        Ok(Self { text, discounts })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn the_segment_syntax_round_trips_in_both_forms() {
        // The syntax is what a KoSIT validator reads under BR-DE-18, so rendering
        // and parsing have to be exact inverses — a `Display` that dropped the
        // trailing `#`, or lost the two decimals on the percentage, would produce
        // a field that still looks right to a human and fails validation.
        for (rendered, days, percent) in [
            ("#SKONTO#TAGE=10#PROZENT=2.00#", 10u32, dec!(2)),
            ("#SKONTO#TAGE=0#PROZENT=100.00#", 0, dec!(100)),
            ("#SKONTO#TAGE=365#PROZENT=0.50#", 365, dec!(0.5)),
        ] {
            let parsed: EarlyPaymentDiscount = rendered.parse().unwrap();
            assert_eq!(parsed.days(), days);
            assert_eq!(parsed.percent(), percent);
            assert_eq!(parsed.to_string(), rendered);
        }

        let with_base = "#SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#";
        let parsed: EarlyPaymentDiscount = with_base.parse().unwrap();
        assert_eq!(
            parsed.base_amount(),
            Some(Amount::parse("485.98000").unwrap())
        );
        assert_eq!(parsed.to_string(), with_base);
    }

    #[test]
    fn malformed_segments_are_refused_rather_than_guessed_at() {
        // Each of these is a form some hand-written emitter produces, and each is
        // fatal under BR-DE-18. Accepting them here would move the failure from a
        // `Result` in the caller's code to a rejected invoice at the access point.
        for bad in [
            "#skonto#TAGE=10#PROZENT=2.00#",       // lowercase
            " #SKONTO#TAGE=10#PROZENT=2.00#",      // leading space
            "#SKONTO#TAGE=10#PROZENT=2.00",        // no closing #
            "#SKONTO#TAGE=10#",                    // no percentage
            "#SKONTO#PROZENT=2.00#",               // no days
            "#SKONTO#TAGE=ten#PROZENT=2.00#",      // days not a number
            "#SKONTO#TAGE=10#PROZENT=2.00#FOO=1#", // unknown key
            "#SKONTO#TAGE=10#PROZENT=0.00#",       // not a discount
        ] {
            assert!(
                bad.parse::<EarlyPaymentDiscount>().is_err(),
                "{bad:?} must be refused"
            );
        }
    }

    #[test]
    fn the_discount_is_taken_off_the_gross_amount_due() {
        // Skonto is deducted from the payable amount *including* VAT, not from the
        // net — that is what makes the later §17 UStG correction necessary at all.
        let d = EarlyPaymentDiscount::new(14, dec!(2)).unwrap();
        let due = Amount::<5>::parse("1190.00000").unwrap();
        assert_eq!(
            d.discount_on(due).unwrap(),
            Amount::parse("23.80000").unwrap()
        );
        assert_eq!(
            d.payable_on(due).unwrap(),
            Amount::parse("1166.20000").unwrap()
        );
        assert_eq!(d.rate().unwrap(), dec!(0.02));
    }

    #[test]
    fn a_stated_base_overrides_the_amount_due() {
        let d = EarlyPaymentDiscount::new(10, dec!(2))
            .unwrap()
            .with_base_amount(Amount::parse("500.00000").unwrap())
            .unwrap();
        // 2 % of the stated 500.00, not of the 1000.00 due.
        assert_eq!(
            d.discount_on(Amount::parse("1000.00000").unwrap()).unwrap(),
            Amount::parse("10.00000").unwrap()
        );
        assert_eq!(
            d.payable_on(Amount::parse("1000.00000").unwrap()).unwrap(),
            Amount::parse("990.00000").unwrap()
        );
    }

    #[test]
    fn a_base_the_syntax_cannot_write_is_refused() {
        // `BASISBETRAG` is written with two decimals. Accepting 485.987 would emit
        // `485.99` and quietly change the number the buyer computes against.
        let d = EarlyPaymentDiscount::new(10, dec!(2)).unwrap();
        assert!(
            d.with_base_amount(Amount::parse("485.98700").unwrap())
                .is_err()
        );
        assert!(
            d.with_base_amount(Amount::parse("-1.00000").unwrap())
                .is_err()
        );
        assert!(
            d.with_base_amount(Amount::parse("485.98000").unwrap())
                .is_ok()
        );
    }

    #[test]
    fn payment_terms_round_trip_through_the_bt20_field() {
        let terms = PaymentTerms::text("Zahlbar in 30 Tagen.")
            .with_discount(EarlyPaymentDiscount::new(10, dec!(3)).unwrap())
            .with_discount(EarlyPaymentDiscount::new(20, dec!(2)).unwrap());
        let rendered = terms.to_string();
        assert_eq!(
            rendered,
            "Zahlbar in 30 Tagen.\n#SKONTO#TAGE=10#PROZENT=3.00#\n#SKONTO#TAGE=20#PROZENT=2.00#"
        );
        assert_eq!(rendered.parse::<PaymentTerms>().unwrap(), terms);

        // Segments only — no prose line survives, and the field starts with `#`.
        let bare =
            PaymentTerms::default().with_discount(EarlyPaymentDiscount::new(7, dec!(1.5)).unwrap());
        assert_eq!(bare.to_string(), "#SKONTO#TAGE=7#PROZENT=1.50#");
        assert_eq!(bare.to_string().parse::<PaymentTerms>().unwrap(), bare);
        assert!(!bare.is_empty());
        assert!(PaymentTerms::default().is_empty());
        assert_eq!(PaymentTerms::default().to_string(), "");
    }

    #[test]
    fn blank_prose_is_no_prose_at_all() {
        // Storing `Some("")` would render a BT-20 field starting with an empty line,
        // which `FromStr` then drops — so the value would not survive its own round
        // trip. It would also make `is_empty` answer "no" for a field with nothing
        // in it.
        for blank in ["", "   ", "\t", "\n"] {
            let terms = PaymentTerms::text(blank);
            assert!(terms.is_empty(), "{blank:?} must count as empty");
            assert_eq!(terms.prose(), None);
            assert_eq!(terms.to_string(), "");
        }

        let with_segment =
            PaymentTerms::text("  ").with_discount(EarlyPaymentDiscount::new(7, dec!(2)).unwrap());
        assert_eq!(with_segment.to_string(), "#SKONTO#TAGE=7#PROZENT=2.00#");
        assert_eq!(
            with_segment.to_string().parse::<PaymentTerms>().unwrap(),
            with_segment
        );

        // Real prose is kept verbatim, including internal blank lines.
        let multi = PaymentTerms::text("Zeile 1\n\nZeile 3");
        assert_eq!(multi.prose(), Some("Zeile 1\n\nZeile 3"));
        assert_eq!(multi.to_string().parse::<PaymentTerms>().unwrap(), multi);
    }

    #[test]
    fn the_discounts_accessor_returns_them_in_declaration_order() {
        // Every other test compares whole `PaymentTerms` values with `assert_eq!`,
        // which reads the private field directly — so the accessor itself had no
        // test outside a doctest, and returning an empty slice passed the suite.
        // Order is part of the contract: `Display` writes one `#SKONTO#` line per
        // discount in this order, and BT-20 is read top to bottom.
        let terms = PaymentTerms::default()
            .with_discount(EarlyPaymentDiscount::new(10, dec!(3)).unwrap())
            .with_discount(EarlyPaymentDiscount::new(20, dec!(2)).unwrap());

        let got = terms.discounts();
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].days(), 10);
        assert_eq!(got[0].percent(), dec!(3));
        assert_eq!(got[1].days(), 20);
        assert_eq!(got[1].percent(), dec!(2));

        assert!(PaymentTerms::text("nur Text").discounts().is_empty());
    }

    #[test]
    fn staged_terms_award_the_best_qualifying_discount() {
        let terms = PaymentTerms::default()
            .with_discount(EarlyPaymentDiscount::new(10, dec!(3)).unwrap())
            .with_discount(EarlyPaymentDiscount::new(20, dec!(2)).unwrap());
        // Paying on day 5 qualifies for both windows; the buyer takes the larger.
        assert_eq!(terms.discount_within(0).map(|d| d.percent()), Some(dec!(3)));
        assert_eq!(
            terms.discount_within(10).map(|d| d.percent()),
            Some(dec!(3))
        );
        assert_eq!(
            terms.discount_within(11).map(|d| d.percent()),
            Some(dec!(2))
        );
        assert_eq!(
            terms.discount_within(20).map(|d| d.percent()),
            Some(dec!(2))
        );
        assert_eq!(terms.discount_within(21), None);
    }

    #[test]
    fn the_best_discount_wins_even_when_it_is_declared_last() {
        // Conventional terms list the shortest window first, so the largest
        // percentage also comes first and "best" and "first" coincide — which
        // makes every ordinary fixture blind to the difference. Nothing requires
        // that order, and a supplier writing "2 % in 20 days, 3 % in 10" would
        // otherwise have a buyer paying on day one quoted 2 %: an under-deduction,
        // and a remittance that no longer matches the invoice.
        let terms = PaymentTerms::default()
            .with_discount(EarlyPaymentDiscount::new(20, dec!(2)).unwrap())
            .with_discount(EarlyPaymentDiscount::new(10, dec!(3)).unwrap());
        assert_eq!(terms.discount_within(1).map(|d| d.percent()), Some(dec!(3)));
        assert_eq!(
            terms.discount_within(10).map(|d| d.percent()),
            Some(dec!(3))
        );
        // Past the 10-day window only the 2 % stage still qualifies.
        assert_eq!(
            terms.discount_within(11).map(|d| d.percent()),
            Some(dec!(2))
        );

        // A tie keeps the first declared — the shorter window, which is the one a
        // buyer paying on day one is actually within.
        let tied = PaymentTerms::default()
            .with_discount(EarlyPaymentDiscount::new(7, dec!(2)).unwrap())
            .with_discount(EarlyPaymentDiscount::new(30, dec!(2)).unwrap());
        assert_eq!(tied.discount_within(1).map(|d| d.days()), Some(7));
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserialisation_re_runs_the_constructor_checks() {
        // Fields are private, but serde reconstructs them directly unless routed
        // through `try_from` — which is the only thing standing between stored
        // config and a percentage the BT-20 syntax cannot write.
        let ok: EarlyPaymentDiscount =
            serde_json::from_str(r#"{"days":10,"percent":"2.00"}"#).unwrap();
        assert_eq!(ok.percent(), dec!(2.00));
        assert!(
            serde_json::from_str::<EarlyPaymentDiscount>(r#"{"days":10,"percent":"0"}"#).is_err()
        );
        assert!(
            serde_json::from_str::<EarlyPaymentDiscount>(r#"{"days":10,"percent":"2.005"}"#)
                .is_err()
        );
        assert!(
            serde_json::from_str::<EarlyPaymentDiscount>(
                r#"{"days":10,"percent":"2","base_amount":"1.001"}"#
            )
            .is_err()
        );
    }
}
