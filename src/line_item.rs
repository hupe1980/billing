//! [`LineItem`] — the atomic billing unit: quantity × unit-price → net amount.
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::amount::Amount;
use crate::error::BillingError;
use crate::period::Period;
use crate::quantity::{Quantity, UnitPrice};
use crate::vat::LineVat;

/// EN 16931 detail for a document level **allowance** (BG-20) or **charge**
/// (BG-21) — the fields that only make sense when a position is one of those.
///
/// | Field | Allowance (BG-20) | Charge (BG-21) |
/// |---|---|---|
/// | [`reason_code`](Self::reason_code) | BT-98 (UNCL 5189) | BT-105 (UNCL 7161) |
/// | [`base_amount`](Self::base_amount) | BT-93 | BT-100 |
/// | [`percentage`](Self::percentage) | BT-94 | BT-101 |
///
/// # Why the base and the percentage travel together
///
/// Because a validator rejects one without the other. Peppol makes it a matched
/// pair, in both directions and both **fatal**:
///
/// > `[PEPPOL-EN16931-R041]` Allowance/charge base amount MUST be provided when
/// > allowance/charge percentage is provided.
/// >
/// > `[PEPPOL-EN16931-R042]` Allowance/charge percentage MUST be provided when
/// > allowance/charge base amount is provided.
///
/// [`AllowanceCharge::validate`] enforces exactly that, so a position built here
/// cannot fail those rules. [`crate::PercentageDiscount`] and
/// [`crate::PercentageCharge`] populate both automatically — they compute
/// `base × rate` and previously discarded both operands, leaving a consumer able
/// to emit only the resulting amount.
///
/// The reason is independent: BR-33 / BR-38 (and BR-CO-21 / BR-CO-22) require a
/// free-text reason *or* a coded one, and [`LineItem::description`] serves as the
/// free text (BT-97 / BT-104), so the code is always optional. BR-CL-19 and
/// BR-CL-20 constrain its value; the engine does not check membership — it has no
/// copy of the code lists, and a stale embedded copy would be worse than none.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "AllowanceChargeRepr"))]
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowanceCharge {
    /// BT-98 (allowance, UNCL 5189) / BT-105 (charge, UNCL 7161).
    pub reason_code: Option<String>,
    /// BT-93 (allowance) / BT-100 (charge) — the amount the percentage was applied
    /// to. Must be present exactly when [`percentage`](Self::percentage) is.
    pub base_amount: Option<Amount<5>>,
    /// BT-94 (allowance) / BT-101 (charge) — the rate as a **percentage**
    /// (`10` for 10 %, matching the wire format), not a fraction. Must be present
    /// exactly when [`base_amount`](Self::base_amount) is.
    pub percentage: Option<Decimal>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct AllowanceChargeRepr {
    #[serde(default)]
    reason_code: Option<String>,
    #[serde(default)]
    base_amount: Option<Amount<5>>,
    #[serde(default)]
    percentage: Option<Decimal>,
}

#[cfg(feature = "serde")]
impl TryFrom<AllowanceChargeRepr> for AllowanceCharge {
    type Error = BillingError;
    fn try_from(r: AllowanceChargeRepr) -> Result<Self, Self::Error> {
        let ac = AllowanceCharge {
            reason_code: r.reason_code,
            base_amount: r.base_amount,
            percentage: r.percentage,
        };
        ac.validate()?;
        Ok(ac)
    }
}

impl AllowanceCharge {
    /// A reason code alone — no percentage basis.
    ///
    /// The right shape for a fixed-amount allowance or a per-unit charge, where
    /// there is no percentage to state.
    #[must_use]
    pub fn coded(reason_code: impl Into<String>) -> Self {
        Self {
            reason_code: Some(reason_code.into()),
            ..Self::default()
        }
    }

    /// A percentage-derived allowance or charge: `percentage` % of `base`.
    ///
    /// `rate` is the fraction the engine works in (`0.10`); it is stored as the
    /// percentage the wire format expects (`10`).
    ///
    /// ```rust
    /// use billing::{AllowanceCharge, Amount};
    /// use rust_decimal::dec;
    ///
    /// let ac = AllowanceCharge::percentage_of(Amount::parse("1000.00000").unwrap(), dec!(0.10));
    /// assert_eq!(ac.base_amount, Some(Amount::parse("1000.00000").unwrap())); // BT-93
    /// assert_eq!(ac.percentage,  Some(dec!(10)));                             // BT-94
    /// assert!(ac.validate().is_ok());
    /// ```
    #[must_use]
    pub fn percentage_of(base: Amount<5>, rate: Decimal) -> Self {
        Self {
            reason_code: None,
            base_amount: Some(base),
            percentage: Some(
                rate.checked_mul(Decimal::ONE_HUNDRED)
                    .map(|d| d.normalize())
                    .unwrap_or(rate),
            ),
        }
    }

    /// Attach the BT-98 / BT-105 reason code.
    #[must_use]
    pub fn with_reason_code(mut self, code: impl Into<String>) -> Self {
        self.reason_code = Some(code.into());
        self
    }

    /// Check the PEPPOL-EN16931-R041 / R042 pairing.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if exactly one of `base_amount` and
    /// `percentage` is set.
    pub fn validate(&self) -> Result<(), BillingError> {
        match (self.base_amount, self.percentage) {
            (Some(_), None) => Err(BillingError::InvalidInput {
                reason: "allowance/charge base amount (BT-93/BT-100) is set without a \
                         percentage (BT-94/BT-101); PEPPOL-EN16931-R042 requires both"
                    .into(),
            }),
            (None, Some(p)) => Err(BillingError::InvalidInput {
                reason: format!(
                    "allowance/charge percentage {p} (BT-94/BT-101) is set without a base \
                     amount (BT-93/BT-100); PEPPOL-EN16931-R041 requires both"
                ),
            }),
            _ => Ok(()),
        }
    }

    /// Peppol's tolerance on the allowance/charge arithmetic, from `u:slack` in
    /// `PEPPOL-EN16931-R040`.
    const R040_SLACK: &'static str = "0.02";

    /// Check that the stated basis reproduces `amount` — **PEPPOL-EN16931-R040**.
    ///
    /// > Allowance/charge amount must equal base amount * percentage/100 if base
    /// > amount and percentage exists
    ///
    /// — **fatal**, with a ±0.02 tolerance. Stating a base and a percentage is
    /// therefore not free annotation: it is a claim a validator recomputes. An
    /// operation that changes the amount without changing the base (a penny
    /// correction, a min/max clamp) must either rescale the base or drop the pair.
    ///
    /// Compares magnitudes, so it holds for an allowance (negative `net_amount`,
    /// positive base) and for a reversed document (both negated) alike. Vacuously
    /// true when no basis is stated.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if the basis is outside tolerance, or
    /// [`BillingError::MonetaryOverflow`] if the product cannot be represented.
    pub fn check_amount(&self, amount: Amount<5>) -> Result<(), BillingError> {
        let (Some(base), Some(pct)) = (self.base_amount, self.percentage) else {
            return Ok(());
        };
        let expected = base
            .into_decimal()
            .checked_mul(pct)
            .and_then(|p| p.checked_div(Decimal::ONE_HUNDRED))
            .ok_or(BillingError::MonetaryOverflow {
                precision: 5,
                input_value: None,
            })?;
        let slack = Decimal::from_str_exact(Self::R040_SLACK).unwrap_or_default();
        let diff = match amount.into_decimal().abs().checked_sub(expected.abs()) {
            Some(d) => d.abs(),
            // Too large to represent is, of course, also outside the tolerance.
            None => Decimal::MAX,
        };
        if diff > slack {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "allowance/charge states {pct}% of {base}, which is {expected}, but the \
                     amount is {amount}; PEPPOL-EN16931-R040 allows ±{}",
                    Self::R040_SLACK
                ),
            });
        }
        Ok(())
    }

    /// Scale the base amount by `factor`, keeping [`check_amount`](Self::check_amount)
    /// true when the position's own amount is scaled by the same factor.
    ///
    /// The percentage is a *rate* and does not scale.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if the scaled base leaves range.
    pub fn scaled(
        &self,
        factor: Decimal,
        strategy: crate::amount::RoundingStrategy,
    ) -> Result<Self, BillingError> {
        let mut out = self.clone();
        if let Some(base) = self.base_amount {
            let scaled =
                base.into_decimal()
                    .checked_mul(factor)
                    .ok_or(BillingError::MonetaryOverflow {
                        precision: 5,
                        input_value: None,
                    })?;
            out.base_amount = Some(Amount::<5>::from_decimal_rounded(scaled, strategy)?);
        }
        Ok(out)
    }

    /// Negate the base amount, for [`crate::BillingDocument::reverse`].
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if the base is `Amount::MIN`.
    pub fn negated(&self) -> Result<Self, BillingError> {
        let mut out = self.clone();
        if let Some(base) = self.base_amount {
            out.base_amount = Some(base.checked_neg()?);
        }
        Ok(out)
    }

    /// Drop the percentage basis, keeping the reason code.
    ///
    /// The honest response to an operation that changes the amount in a way the
    /// base cannot follow — a min/max clamp, or a penny correction applied to one
    /// line of a split. Stating a basis that no longer reproduces the amount is a
    /// fatal `PEPPOL-EN16931-R040` failure; stating none is always valid.
    #[must_use]
    pub fn without_basis(mut self) -> Self {
        self.base_amount = None;
        self.percentage = None;
        self
    }
}

/// The atomic billing unit: a single charge or credit position.
///
/// Every `LineItem` has a `net_amount`: positive = debit (charge), negative = credit.
///
/// Tags allow selective tax/discount application without brittle string matching.
///
/// The `sign` field records the **original intent** of the position — `Sign::Debit` for
/// charges (including debits at a negative unit price, e.g. EPEX negative-price hours),
/// `Sign::Credit` for credits and refunds.  Tax/discount layers should use `sign` to
/// distinguish consumption from return positions rather than testing `net_amount < 0`,
/// which is ambiguous after the introduction of negative unit prices.
///
/// # Fields are public — validation is your responsibility after mutation
///
/// Unlike the tax and schedule types, `LineItem`'s fields are public: it is a
/// data record, and callers legitimately need to retag, annotate or re-period an
/// item after construction. The invariants that [`LineItemBuilder::build`]
/// enforces are therefore **not** guaranteed for an item built by struct literal
/// or mutated afterwards. Call [`LineItem::validate`] if you have done either.
///
/// Deserialisation *is* checked: `LineItem` re-runs [`LineItem::validate`] via
/// `#[serde(try_from)]`, so untrusted JSON cannot introduce a description-less,
/// negative-quantity or sign-inconsistent position.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "LineItemRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineItem {
    /// Human-readable description shown on the invoice.
    pub description: String,
    /// Measured quantity (value + unit label), if applicable.
    pub quantity: Option<Quantity>,
    /// Price per unit (value + unit label), if applicable.
    pub unit_price: Option<UnitPrice>,
    /// Pre-computed net amount; positive = charge, negative = credit.
    pub net_amount: Amount<5>,
    /// Original sign intent: `Debit` = charge (even if `net_amount` is negative due to
    /// a negative unit price); `Credit` = refund / discount.
    pub sign: Sign,
    /// Sub-period this position covers, if different from the document period.
    ///
    /// Set when a single invoice contains positions spanning different time windows
    /// (e.g. a tariff change mid-month: one position for days 1–14, another for 15–30).
    /// Stored as ISO 8601 date strings (`"2026-06-01"`) — not parsed by the engine.
    pub period: Option<Period>,
    /// Arbitrary labels for selective tax/discount filtering and ERP categorization.
    pub tags: Vec<String>,
    /// The VAT treatment of this position — EN 16931 **BT-151 / BT-152** on an
    /// invoice line, **BT-95 / BT-96** on a document level allowance, **BT-102 /
    /// BT-103** on a document level charge.
    ///
    /// EN 16931 makes this mandatory on every one of those (BR-CO-04, BR-32,
    /// BR-37), and checks it against the VAT breakdown (BR-S-08 and siblings).
    /// [`crate::BillingDocument`] fills it in during assembly from the layer whose
    /// base the position is in, so it does not have to be set by hand — see
    /// [`crate::TaxLayer::covers`]. Setting it explicitly is still allowed and, on
    /// a mixed-rate document assembled without VAT layers, necessary.
    ///
    /// `None` means "not attributed": no VAT layer covered this position and the
    /// caller did not say. A consumer emitting EN 16931 must then either ask the
    /// caller or refuse — it may not guess.
    #[cfg_attr(feature = "serde", serde(default))]
    pub vat: Option<LineVat>,
    /// The EN 16931 allowance (BG-20) / charge (BG-21) detail for this position,
    /// where it is one.
    ///
    /// `None` on an ordinary invoice line, which is neither — see
    /// [`AllowanceCharge`] for what it carries and why the parts travel together.
    #[cfg_attr(feature = "serde", serde(default))]
    pub allowance_charge: Option<AllowanceCharge>,
    /// Arbitrary key-value metadata for ERP export.
    pub metadata: HashMap<String, String>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct LineItemRepr {
    description: String,
    #[serde(default)]
    quantity: Option<Quantity>,
    #[serde(default)]
    unit_price: Option<UnitPrice>,
    net_amount: Amount<5>,
    sign: Sign,
    #[serde(default)]
    period: Option<Period>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    vat: Option<LineVat>,
    #[serde(default)]
    allowance_charge: Option<AllowanceCharge>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

#[cfg(feature = "serde")]
impl TryFrom<LineItemRepr> for LineItem {
    type Error = BillingError;
    fn try_from(r: LineItemRepr) -> Result<Self, Self::Error> {
        let item = LineItem {
            description: r.description,
            quantity: r.quantity,
            unit_price: r.unit_price,
            net_amount: r.net_amount,
            sign: r.sign,
            period: r.period,
            tags: r.tags,
            vat: r.vat,
            allowance_charge: r.allowance_charge,
            metadata: r.metadata,
        };
        item.validate()?;
        Ok(item)
    }
}

impl LineItem {
    /// Set [`LineItem::sign`] to match the sign of `net_amount`.
    ///
    /// Positive → [`Sign::Debit`], negative → [`Sign::Credit`], zero → unchanged
    /// (the direction of a zero-amount position carries no information).
    ///
    /// Needed wherever an amount is transformed in a way that can cross zero —
    /// reversal, and the penny correction in allocation. Without it a `Credit`
    /// line can end up with a positive `net_amount`, which
    /// [`LineItem::validate`] rejects and which corrupts the sign-based filtering
    /// that tax and discount layers rely on.
    pub fn normalize_sign(&mut self) {
        if self.net_amount.is_positive() {
            self.sign = Sign::Debit;
        } else if self.net_amount.is_negative() {
            self.sign = Sign::Credit;
        }
    }

    /// Re-check the invariants [`LineItemBuilder::build`] enforces.
    ///
    /// Because `LineItem`'s fields are public, an item can be constructed by
    /// struct literal or mutated after building, bypassing those checks. This
    /// method re-establishes them; it runs automatically on deserialisation.
    ///
    /// Checks:
    /// 1. `description` is not empty or whitespace-only (an unlabelled position
    ///    is not auditable).
    /// 2. `quantity.value` is non-negative (refunds are modelled with
    ///    [`Sign::Credit`], not with a negative quantity).
    /// 3. A [`Sign::Credit`] position does not carry a positive `net_amount` —
    ///    tax and discount layers filter on `sign`, so a "credit" that adds to
    ///    the total would corrupt their bases.
    /// 4. Unit labels, where present, are not empty or whitespace-only. An empty
    ///    quantity unit renders as a bare space on the invoice and — because
    ///    [`crate::PerUnitLevy`] selects its base by matching `unit_label` — silently
    ///    excludes the position from every per-unit levy. An empty price unit
    ///    renders as `"EUR/"`. [`crate::TariffSchedule`] and
    ///    [`crate::TimeOfUsePricing`] have always rejected empty units in their
    ///    builders; positions built by hand now get the same guarantee.
    /// 5. `vat`, where present, is category/rate-consistent — see
    ///    [`LineVat::validate`].
    /// 6. `allowance_charge`, where present, states a base amount and a percentage
    ///    together or not at all ([`AllowanceCharge::validate`]), and the stated
    ///    basis reproduces `net_amount` within Peppol's tolerance
    ///    ([`AllowanceCharge::check_amount`], rule PEPPOL-EN16931-R040).
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] naming the violated invariant.
    ///
    /// ```rust
    /// use billing::{LineItem, Amount};
    /// let item = LineItem::fixed("Grundpreis", Amount::<5>::parse("8.50000").unwrap())
    ///     .build().unwrap();
    /// assert!(item.validate().is_ok());
    ///
    /// let mut broken = item.clone();
    /// broken.description = "   ".into();
    /// assert!(broken.validate().is_err());
    /// ```
    pub fn validate(&self) -> Result<(), BillingError> {
        if self.description.trim().is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "LineItem description must not be empty".into(),
            });
        }
        if let Some(q) = &self.quantity {
            if q.value < Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: format!("LineItem quantity must be non-negative, got {}", q.value),
                });
            }
            crate::check_unit("LineItem quantity", &q.unit)?;
        }
        if let Some(p) = &self.unit_price {
            crate::check_unit("LineItem unit_price", &p.unit)?;
        }
        if self.sign == Sign::Credit && self.net_amount.is_positive() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "LineItem with Sign::Credit must not have a positive net_amount (got {})",
                    self.net_amount
                ),
            });
        }
        if let Some(vat) = &self.vat {
            vat.validate()?;
        }
        if let Some(ac) = &self.allowance_charge {
            ac.validate()?;
            ac.check_amount(self.net_amount)?;
        }
        Ok(())
    }

    /// Start building a debit (charge) position.
    #[must_use]
    pub fn debit(description: impl Into<String>) -> LineItemBuilder {
        LineItemBuilder::new(description.into(), Sign::Debit)
    }

    /// Start building a credit position (negative net amount).
    #[must_use]
    pub fn credit(description: impl Into<String>) -> LineItemBuilder {
        LineItemBuilder::new(description.into(), Sign::Credit)
    }

    /// Create a fixed-amount debit position (no quantity × price).
    ///
    /// Returns a `LineItemBuilder` so you can add `.tag()` / `.meta()` before `.build()`.
    ///
    /// # Example
    /// ```rust
    /// use billing::{LineItem, Amount};
    /// let item = LineItem::fixed("Grundpreis", Amount::<5>::parse("8.50000").unwrap())
    ///     .tag("fixed")
    ///     .build()
    ///     .unwrap();
    /// assert_eq!(item.net_amount, Amount::<5>::parse("8.50000").unwrap());
    /// ```
    #[must_use]
    pub fn fixed(description: impl Into<String>, amount: Amount<5>) -> LineItemBuilder {
        LineItemBuilder::new(description.into(), Sign::Debit).fixed_amount(amount)
    }

    /// Create a fixed-amount credit position (negative net amount).
    ///
    /// Symmetric counterpart to [`LineItem::fixed`]. The `amount` is stored as-is;
    /// if it is positive it is flipped to negative during `build()`.
    ///
    /// # Example
    /// ```rust
    /// use billing::{LineItem, Amount};
    /// let item = LineItem::credit_fixed("§25 EEG-Sanktion", Amount::<5>::parse("0.00000").unwrap())
    ///     .tag("sanction")
    ///     .build()
    ///     .unwrap();
    /// assert!(item.net_amount.is_zero());
    /// ```
    #[must_use]
    pub fn credit_fixed(description: impl Into<String>, amount: Amount<5>) -> LineItemBuilder {
        LineItemBuilder::new(description.into(), Sign::Credit).fixed_amount(amount)
    }

    /// Convenience constructor for the most common pattern: `quantity × unit_price`.
    ///
    /// A negative `unit_price` produces a negative `net_amount` automatically
    /// (no need to switch to `Sign::Credit`). This is correct for real-time
    /// spot markets where negative prices are legally binding (e.g. EPEX negative hours).
    ///
    /// # Why [`Quantity`] and [`UnitPrice`] rather than four loose arguments
    ///
    /// This constructor used to take the value and unit of each as separate
    /// positional parameters — `(desc, qty, qty_unit, price, price_unit)` — which
    /// put two free-form `&str` units side by side. Swapping them
    /// (`"EUR/kWh"` for `"kWh"`) still compiled and still produced an invoice, just
    /// a wrong one. Passing the pairs pre-assembled makes that transposition a type
    /// error instead: [`Quantity`] and [`UnitPrice`] are distinct types, and each
    /// keeps its value glued to the unit that describes it.
    ///
    /// # Example
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// // Normal positive EPEX price
    /// let pos = LineItem::for_usage(
    ///     "Arbeit",
    ///     Quantity::new(dec!(1000), "kWh"),
    ///     UnitPrice::new(dec!(0.289), "EUR/kWh"),
    /// ).build().unwrap();
    /// assert_eq!(pos.net_amount, Amount::<5>::parse("289.00000").unwrap());
    ///
    /// // Negative EPEX spot price (§27 EEG 2023 — post-EEG plant)
    /// let neg = LineItem::for_usage(
    ///     "EPEX Spot (negativ)",
    ///     Quantity::new(dec!(1000), "kWh"),
    ///     UnitPrice::new(dec!(-0.005), "EUR/kWh"),
    /// ).build().unwrap();
    /// assert_eq!(neg.net_amount, Amount::<5>::parse("-5.00000").unwrap());
    /// ```
    #[must_use]
    pub fn for_usage(
        description: impl Into<String>,
        quantity: Quantity,
        unit_price: UnitPrice,
    ) -> LineItemBuilder {
        LineItemBuilder::new(description.into(), Sign::Debit)
            .quantity(quantity)
            .unit_price(unit_price)
    }

    /// Convenience constructor for a **credit** usage position (quantity × rate, negated).
    ///
    /// The symmetric counterpart of [`LineItem::for_usage`] for refund / feed-in credit
    /// positions where the charge direction is `Credit` (e.g. EEG Einspeisevergütung,
    /// Mindermengen-Gutschrift).  The resulting `net_amount` is automatically negated.
    ///
    /// Use `for_usage` (debit) when the unit price itself is already negative
    /// (e.g. EPEX negative-price hours under §27 EEG 2023) so that `Sign::Debit`
    /// is preserved for levy-base calculations.
    ///
    /// # Example
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// // EEG feed-in credit: 500 kWh × 0.0811 EUR/kWh → net = -40.55000
    /// let credit = LineItem::credit_for_usage(
    ///     "EEG Einspeisevergütung",
    ///     Quantity::new(dec!(500), "kWh"),
    ///     UnitPrice::new(dec!(0.0811), "EUR/kWh"),
    /// ).build().unwrap();
    /// assert_eq!(credit.net_amount, Amount::<5>::parse("-40.55000").unwrap());
    /// assert!(credit.is_credit());
    /// ```
    #[must_use]
    pub fn credit_for_usage(
        description: impl Into<String>,
        quantity: Quantity,
        unit_price: UnitPrice,
    ) -> LineItemBuilder {
        LineItemBuilder::new(description.into(), Sign::Credit)
            .quantity(quantity)
            .unit_price(unit_price)
    }

    /// Returns `true` if this position has the given tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    /// Returns `true` if this position was built with [`Sign::Debit`].
    ///
    /// Note: a debit position may have a **negative** `net_amount` when the
    /// `unit_price` was negative (e.g. EPEX negative-price hours).  Use this
    /// method rather than `net_amount > 0` to identify consumption positions.
    #[must_use]
    pub fn is_debit(&self) -> bool {
        self.sign == Sign::Debit
    }

    /// Returns `true` if this position was built with [`Sign::Credit`].
    ///
    /// Credit positions are refunds, discounts, and return-feed-in credits.
    /// Their `net_amount` is always ≤ 0 by construction.
    #[must_use]
    pub fn is_credit(&self) -> bool {
        self.sign == Sign::Credit
    }

    /// Look up a metadata value by key.
    ///
    /// Returns `Some(&str)` if the key exists, `None` otherwise.
    /// Equivalent to `item.metadata.get(key).map(String::as_str)` but more ergonomic.
    #[must_use]
    pub fn get_meta(&self, key: &str) -> Option<&str> {
        self.metadata.get(key).map(String::as_str)
    }

    /// Scale this position by `factor`, keeping it internally consistent.
    ///
    /// Both `net_amount` **and** `quantity` are multiplied by `factor`; `unit_price`
    /// is left untouched.  This preserves the invoice-line identity
    /// `quantity × unit_price ≈ net_amount`, which naive net-only scaling breaks.
    ///
    /// `net_amount` is rounded to 5 dp with `strategy`.  `quantity` is scaled
    /// exactly (no rounding) so that the displayed quantity keeps full precision.
    ///
    /// Used by [`crate::prorate`] and by the [`crate::AllocationRule`]
    /// implementations.  The `description` is left unchanged — callers annotate it.
    ///
    /// # Example
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice, RoundingStrategy};
    /// use rust_decimal::dec;
    ///
    /// let full = LineItem::for_usage(
    ///     "Arbeit",
    ///     Quantity::new(dec!(1000), "kWh"),
    ///     UnitPrice::new(dec!(0.30), "EUR/kWh"),
    /// ).build().unwrap();
    /// let half = full.scaled(dec!(0.5), RoundingStrategy::MidpointAwayFromZero).unwrap();
    ///
    /// // Quantity is scaled too — the line still reads correctly.
    /// assert_eq!(half.quantity_value(), Some(dec!(500)));
    /// assert_eq!(half.net_amount, Amount::<5>::parse("150.00000").unwrap());
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if the scaled amount or quantity overflows.
    pub fn scaled(
        &self,
        factor: Decimal,
        strategy: crate::amount::RoundingStrategy,
    ) -> Result<Self, BillingError> {
        let overflow = || BillingError::MonetaryOverflow {
            precision: 5,
            input_value: None,
        };
        let scaled_net = self
            .net_amount
            .into_decimal()
            .checked_mul(factor)
            .ok_or_else(overflow)?;
        let mut out = self.clone();
        out.net_amount = Amount::<5>::from_decimal_rounded(scaled_net, strategy)?;
        // BT-93 / BT-100 must follow the amount, or the pair stops reproducing it
        // and PEPPOL-EN16931-R040 fails — fatally, and by the whole difference
        // rather than by a rounding residual.
        if let Some(ac) = out.allowance_charge.as_ref() {
            out.allowance_charge = Some(ac.scaled(factor, strategy)?);
        }
        if let Some(q) = out.quantity.as_mut() {
            // Bound the scale of the scaled quantity. An exact product such as
            // 1000 × (1/3) carries 28 significant decimals, which both renders
            // absurdly on an invoice ("99.99999999999999999999999999 kWh") and
            // walks the value toward Decimal's 28-digit ceiling under repeated
            // scaling. `QUANTITY_SCALE` is far beyond any real metering precision,
            // so this never loses meaningful information.
            const QUANTITY_SCALE: u32 = 12;
            q.value = q
                .value
                .checked_mul(factor)
                .ok_or_else(overflow)?
                .round_dp_with_strategy(
                    QUANTITY_SCALE,
                    rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                )
                .normalize();
        }
        Ok(out)
    }

    /// Returns the quantity value if present.
    #[must_use]
    pub fn quantity_value(&self) -> Option<Decimal> {
        self.quantity.as_ref().map(|q| q.value)
    }

    /// Returns the unit label from the quantity if present.
    #[must_use]
    pub fn unit_label(&self) -> Option<&str> {
        self.quantity.as_ref().map(|q| q.unit.as_str())
    }
}

// ── Sign ──────────────────────────────────────────────────────────────────────

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// Sign of a [`LineItem`]: debit (charge) or credit (discount / refund).
pub enum Sign {
    /// A positive charge added to the invoice total.
    Debit,
    /// A negative credit subtracted from the invoice total.
    Credit,
}

// ── Builder ───────────────────────────────────────────────────────────────────

#[derive(Debug)]
/// Builder for [`LineItem`]. Obtain via [`LineItem::debit`], [`LineItem::credit`], or [`LineItem::fixed`].
pub struct LineItemBuilder {
    description: String,
    sign: Sign,
    quantity: Option<Quantity>,
    unit_price: Option<UnitPrice>,
    fixed_amount: Option<Amount<5>>,
    period: Option<Period>,
    tags: Vec<String>,
    vat: Option<LineVat>,
    allowance_charge: Option<AllowanceCharge>,
    metadata: HashMap<String, String>,
}

impl LineItemBuilder {
    fn new(description: String, sign: Sign) -> Self {
        Self {
            description,
            sign,
            quantity: None,
            unit_price: None,
            fixed_amount: None,
            period: None,
            tags: vec![],
            vat: None,
            allowance_charge: None,
            metadata: HashMap::new(),
        }
    }

    #[must_use]
    /// Set the quantity.
    pub fn quantity(mut self, q: Quantity) -> Self {
        self.quantity = Some(q);
        self
    }

    #[must_use]
    /// Set the unit price.
    pub fn unit_price(mut self, p: UnitPrice) -> Self {
        self.unit_price = Some(p);
        self
    }

    #[must_use]
    /// Set a fixed net amount (bypasses quantity × price).
    pub fn fixed_amount(mut self, a: Amount<5>) -> Self {
        self.fixed_amount = Some(a);
        self
    }

    #[must_use]
    /// Add a tag for selective tax / discount filtering.
    pub fn tag(mut self, t: impl Into<String>) -> Self {
        self.tags.push(t.into());
        self
    }

    /// Declare this position's VAT category and rate — EN 16931 BT-151 / BT-152
    /// (line), BT-95 / BT-96 (allowance), BT-102 / BT-103 (charge).
    ///
    /// Usually unnecessary: [`crate::BillingDocument`] derives the attribution
    /// during assembly from the VAT layer whose base the position falls in. Set it
    /// here when the caller already knows the answer, or when the document is
    /// assembled without VAT layers at all.
    ///
    /// If both are present they must agree — assembly reports a
    /// [`BillingError::LayerError`] rather than silently preferring one, because a
    /// disagreement means the tagging and the caller's intent have diverged.
    ///
    /// ```rust
    /// use billing::{LineItem, Amount, LineVat, TaxCategory};
    /// use rust_decimal::dec;
    ///
    /// let item = LineItem::fixed("Beratung", Amount::parse("100.00000").unwrap())
    ///     .vat(LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap())
    ///     .build().unwrap();
    /// assert_eq!(item.vat.unwrap().category, TaxCategory::Standard);
    /// ```
    #[must_use]
    pub fn vat(mut self, vat: LineVat) -> Self {
        self.vat = Some(vat);
        self
    }

    /// Mark this position as a document level allowance (BG-20) or charge (BG-21)
    /// and attach its EN 16931 detail. See [`AllowanceCharge`].
    #[must_use]
    pub fn allowance_charge(mut self, ac: AllowanceCharge) -> Self {
        self.allowance_charge = Some(ac);
        self
    }

    #[must_use]
    /// Add a key-value metadata pair.
    pub fn meta(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.metadata.insert(k.into(), v.into());
        self
    }

    /// Set the sub-period this position covers.
    ///
    /// Use when a single invoice contains positions spanning different time windows
    /// (e.g. a tariff change mid-month). Dates should be ISO 8601 strings (`"2026-06-01"`).
    ///
    /// # Example
    /// ```rust
    /// use billing::{LineItem, Amount};
    /// let item = LineItem::fixed("Grundpreis (1.–14. Juni)", Amount::<5>::parse("14.00000").unwrap())
    ///     .period("2026-06-01", "2026-06-14")
    ///     .build()
    ///     .unwrap();
    /// assert_eq!(item.period.as_ref().unwrap().from, "2026-06-01");
    /// assert_eq!(item.period.as_ref().unwrap().to,   "2026-06-14");
    /// ```
    #[must_use]
    pub fn period(mut self, from: impl Into<String>, to: impl Into<String>) -> Self {
        self.period = Some(Period::new(from, to));
        self
    }

    /// Build the `LineItem`.
    ///
    /// Net amount is:
    /// 1. `fixed_amount` if set (ignores quantity/unit_price)
    /// 2. `quantity.value × unit_price.value` rounded to 5dp — **both signs allowed**
    /// 3. `Err` if neither is provided
    ///
    /// Negative `unit_price` is valid and produces a negative `net_amount` (e.g.
    /// EPEX negative-price hours under §27 EEG 2023).
    ///
    /// # Errors
    /// Returns `Err` if description is empty, quantity is negative, a unit label is
    /// empty, or neither `fixed_amount` nor `quantity + unit_price` is provided.
    pub fn build(self) -> Result<LineItem, BillingError> {
        // A line item without a description is not auditable.
        if self.description.trim().is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "LineItem description must not be empty".into(),
            });
        }
        // Unit labels are load bearing, not cosmetic: `PerUnitLevy` matches its base
        // on `unit_label`, so an empty unit silently drops the position out of every
        // per-unit levy in addition to rendering as a bare space on the invoice.
        if let Some(q) = &self.quantity {
            crate::check_unit("LineItem quantity", &q.unit)?;
        }
        if let Some(p) = &self.unit_price {
            crate::check_unit("LineItem unit_price", &p.unit)?;
        }
        let net = if let Some(fixed) = self.fixed_amount {
            fixed
        } else if let (Some(qty), Some(price)) = (&self.quantity, &self.unit_price) {
            // Quantity must be non-negative; a negative quantity on a debit or
            // credit line is a caller error (model refunds via Sign::Credit, not
            // by negating the quantity).
            if qty.value < rust_decimal::Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "LineItem quantity must be non-negative".into(),
                });
            }
            // Negative unit_price is allowed — it produces a negative net amount.
            // This is correct for spot-market negative prices (e.g. EPEX §27 EEG 2023).
            //
            // `Decimal * Decimal` panics on overflow, so the checked form is required
            // to honour this method's `Result` contract for extreme quantities/prices.
            let raw = qty
                .value
                .checked_mul(price.value)
                .ok_or(BillingError::MonetaryOverflow {
                    precision: 5,
                    input_value: None,
                })?;
            Amount::<5>::from_decimal_rounded(raw, crate::RoundingStrategy::MidpointAwayFromZero)?
        } else {
            return Err(BillingError::InvalidInput {
                reason: "LineItem requires either fixed_amount or both quantity and unit_price"
                    .into(),
            });
        };

        let net = if self.sign == Sign::Credit && net.is_positive() {
            -net
        } else {
            net
        };

        // A half-stated percentage basis is fatal in Peppol (R041 / R042), and a
        // basis that does not reproduce the amount is fatal under R040. Checked
        // here — after `net` is final — rather than leaving `LineItem::validate` as
        // the only thing between them and an emitted document.
        if let Some(ac) = &self.allowance_charge {
            ac.validate()?;
            ac.check_amount(net)?;
        }

        Ok(LineItem {
            description: self.description,
            quantity: self.quantity,
            unit_price: self.unit_price,
            net_amount: net,
            sign: self.sign,
            period: self.period,
            tags: self.tags,
            vat: self.vat,
            allowance_charge: self.allowance_charge,
            metadata: self.metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn debit_position() {
        let item = LineItem::debit("Arbeit")
            .quantity(Quantity::new(dec!(1000), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.28901), "EUR/kWh"))
            .build()
            .unwrap();
        assert_eq!(item.net_amount, Amount::<5>::parse("289.01000").unwrap());
        assert!(!item.net_amount.is_negative());
    }

    #[test]
    fn credit_position() {
        let item = LineItem::credit("EEG Vergütung")
            .quantity(Quantity::new(dec!(500), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.0832), "EUR/kWh"))
            .build()
            .unwrap();
        assert!(item.net_amount.is_negative());
        assert_eq!(item.net_amount, Amount::<5>::parse("-41.60000").unwrap());
    }

    #[test]
    fn fixed_position() {
        let item = LineItem::fixed("Grundpreis", Amount::<5>::parse("8.50000").unwrap())
            .build()
            .unwrap();
        assert_eq!(item.net_amount, Amount::<5>::parse("8.50000").unwrap());
    }

    #[test]
    fn tag_filtering() {
        let item = LineItem::debit("Arbeit")
            .tag("commodity")
            .tag("energy")
            .quantity(Quantity::new(dec!(100), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .build()
            .unwrap();
        assert!(item.has_tag("commodity"));
        assert!(!item.has_tag("fixed"));
    }
}
