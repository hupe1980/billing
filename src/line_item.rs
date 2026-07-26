//! [`LineItem`] — the atomic billing unit: quantity × unit-price → net amount.
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::amount::Amount;
use crate::error::BillingError;
use crate::period::Period;
use crate::quantity::{Quantity, UnitPrice};

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

        Ok(LineItem {
            description: self.description,
            quantity: self.quantity,
            unit_price: self.unit_price,
            net_amount: net,
            sign: self.sign,
            period: self.period,
            tags: self.tags,
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
