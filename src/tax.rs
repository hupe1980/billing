//! Tax and discount overlays: [`TaxLayer`], [`DiscountLayer`], and built-in implementations.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::quantity::{Quantity, UnitPrice};

// ── TaxLayer trait ────────────────────────────────────────────────────────────

/// Composable tax / levy overlay.
///
/// Tax layers are applied in declaration order — sequence matters for compound
/// taxes (e.g. Stromsteuer BEFORE MwSt so Stromsteuer is part of the MwSt base).
///
/// Each layer receives all net positions so it can filter by tag or unit label.
/// Returns a `Result` so user-defined layers can propagate domain errors.
pub trait TaxLayer {
    /// The display name of this layer (used in generated `LineItem` descriptions).
    fn name(&self) -> &str;
    /// Compute the tax from the current net positions.
    ///
    /// Returns a single `LineItem`:
    /// - positive → additional charge (tax, surcharge)
    /// - negative → tax rebate / reverse charge
    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError>;
}

// ── FixedRateTax ──────────────────────────────────────────────────────────────

/// Fixed-percentage tax applied to the net total of all (or tagged) positions.
///
/// Example: MwSt 19%, VAT 20%, GST 10%.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct FixedRateTax {
    /// Display name (e.g. `"MwSt"`, `"VAT"`).
    pub name: String,
    /// Tax rate as a fraction (e.g. `0.19` for 19%).
    pub rate: Decimal,
    /// If set, only positions with this tag contribute to the tax base.
    pub require_tag: Option<String>,
}

impl FixedRateTax {
    /// Create a `FixedRateTax` with no tag filter.
    ///
    /// # Panics
    /// Panics if `rate < 0`. A negative rate is a discount; use [`DiscountLayer`] instead.
    #[must_use]
    pub fn new(name: impl Into<String>, rate: Decimal) -> Self {
        assert!(
            rate >= Decimal::ZERO,
            "FixedRateTax rate must be >= 0, got {rate}"
        );
        Self {
            name: name.into(),
            rate,
            require_tag: None,
        }
    }

    #[must_use]
    /// Restrict the tax base to positions carrying `tag`.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.require_tag = Some(tag.into());
        self
    }
}

impl TaxLayer for FixedRateTax {
    fn name(&self) -> &str {
        &self.name
    }

    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError> {
        let base = Amount::checked_sum(
            positions
                .iter()
                .filter(|p| self.require_tag.as_deref().is_none_or(|t| p.has_tag(t)))
                .map(|p| p.net_amount),
        )?;
        let tax = base.checked_mul_qty(self.rate)?;
        // Use normalize() to strip trailing zeros: 19.00 → 19, 19.50 → 19.5
        let rate_pct = (self.rate * Decimal::from(100)).normalize();
        LineItem::debit(format!("{} ({}%)", self.name, rate_pct))
            .fixed_amount(tax)
            .tag("tax")
            .build()
    }
}

// ── PerUnitLevy ───────────────────────────────────────────────────────────────

/// Per-unit levy (e.g. Stromsteuer 2.05 ct/kWh, CO₂ levy, excise duty).
///
/// Sums units from positions whose `unit_label` matches `unit`.
/// Optionally further restricted to positions with a specific `require_tag`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct PerUnitLevy {
    /// Display name of this levy (e.g. `"Stromsteuer"`).
    pub name: String,
    /// Rate per unit in the invoice currency (e.g. 0.02050 EUR/kWh).
    pub rate: Amount<5>,
    /// Unit label to match (e.g. `"kWh"`, `"MWh"`, `"m³"`).
    pub unit: String,
    /// Only apply to positions with this tag. `None` = apply to all matching units.
    pub require_tag: Option<String>,
}

impl PerUnitLevy {
    /// Create a `PerUnitLevy` with no tag filter.
    ///
    /// # Panics
    /// Panics if `rate` is negative (use [`FixedDiscount`] for per-unit credits).
    #[must_use]
    pub fn new(name: impl Into<String>, rate: Amount<5>, unit: impl Into<String>) -> Self {
        assert!(
            !rate.is_negative(),
            "PerUnitLevy rate must be >= 0, got {rate}"
        );
        Self {
            name: name.into(),
            rate,
            unit: unit.into(),
            require_tag: None,
        }
    }

    #[must_use]
    /// Restrict this levy to positions carrying `tag`.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.require_tag = Some(tag.into());
        self
    }
}

impl TaxLayer for PerUnitLevy {
    fn name(&self) -> &str {
        &self.name
    }

    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError> {
        // Sum physical quantities from Sign::Debit positions only.
        // Credit positions (returns/feed-in) are excluded because per-unit levies
        // (excise duties, environmental fees) apply to consumption, not to credits.
        // Using `p.sign` rather than `p.net_amount.is_negative()` correctly handles
        // Sign::Debit positions that have a negative net_amount due to a negative
        // unit_price (e.g. EPEX negative-price hours under §27 EEG 2023).
        let total_units: Decimal = positions
            .iter()
            .filter(|p| p.is_debit())
            .filter(|p| p.unit_label() == Some(&self.unit))
            .filter(|p| self.require_tag.as_deref().is_none_or(|t| p.has_tag(t)))
            .filter_map(|p| p.quantity_value())
            .sum();
        let price_unit = format!("EUR/{}", self.unit);
        LineItem::debit(format!("{} ({}/{})", self.name, self.rate, self.unit))
            .quantity(Quantity::new(total_units, &self.unit))
            .unit_price(UnitPrice::new(self.rate.into_decimal(), &price_unit))
            .tag("tax")
            .tag("levy")
            .build()
    }
}

// ── PercentageCharge ──────────────────────────────────────────────────────────

/// Charge X% of the net total of selected positions.
///
/// Different from `FixedRateTax` (regulatory levy) and `PercentageDiscount`
/// (credit). Used for platform fees, marketplace commissions, payment
/// processing surcharges.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct PercentageCharge {
    /// Display name (e.g. `"Platform commission"`).
    pub name: String,
    /// Charge rate as a fraction (e.g. `0.025` = 2.5%).
    pub rate: Decimal,
    /// Only apply to positions with this tag. `None` = apply to entire net total.
    pub apply_to_tag: Option<String>,
    /// Floor: minimum charge amount.
    pub min_amount: Option<Amount<5>>,
    /// Ceiling: maximum charge amount.
    pub max_amount: Option<Amount<5>>,
}

impl PercentageCharge {
    /// Create a `PercentageCharge` with no tag filter and no min/max guard.
    ///
    /// # Panics
    /// Panics if `rate < 0`. A negative charge is a discount; use [`DiscountLayer`] instead.
    #[must_use]
    pub fn new(name: impl Into<String>, rate: Decimal) -> Self {
        assert!(
            rate >= Decimal::ZERO,
            "PercentageCharge rate must be >= 0, got {rate}"
        );
        Self {
            name: name.into(),
            rate,
            apply_to_tag: None,
            min_amount: None,
            max_amount: None,
        }
    }

    #[must_use]
    /// Restrict this charge to positions carrying `tag`.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.apply_to_tag = Some(tag.into());
        self
    }

    #[must_use]
    /// Set a minimum charge floor.
    pub fn with_min(mut self, min: Amount<5>) -> Self {
        self.min_amount = Some(min);
        self
    }

    #[must_use]
    /// Set a maximum charge ceiling.
    pub fn with_max(mut self, max: Amount<5>) -> Self {
        self.max_amount = Some(max);
        self
    }
}

impl TaxLayer for PercentageCharge {
    fn name(&self) -> &str {
        &self.name
    }

    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError> {
        // Guard: if both min and max are set, min must not exceed max.
        // Violating this would cause the clamping logic to produce a result
        // below the declared minimum, silently breaking the contract.
        if let (Some(min), Some(max)) = (self.min_amount, self.max_amount) {
            if min > max {
                return Err(BillingError::InvalidInput {
                    reason: "PercentageCharge: min_amount must not exceed max_amount".into(),
                });
            }
        }
        let base = Amount::checked_sum(
            positions
                .iter()
                .filter(|p| self.apply_to_tag.as_deref().is_none_or(|t| p.has_tag(t)))
                .filter(|p| p.is_debit()) // charges only; exclude Sign::Credit positions
                .map(|p| p.net_amount),
        )?;
        let mut charge = base.checked_mul_qty(self.rate)?;
        if let Some(min) = self.min_amount {
            if charge < min {
                charge = min;
            }
        }
        if let Some(max) = self.max_amount {
            if charge > max {
                charge = max;
            }
        }
        let rate_pct = (self.rate * Decimal::from(100)).normalize();
        LineItem::debit(format!("{} ({}%)", self.name, rate_pct))
            .fixed_amount(charge)
            .tag("percentage-charge")
            .build()
    }
}

// ── DiscountLayer trait ───────────────────────────────────────────────────────

/// Composable discount overlay — always produces a credit (negative) position.
pub trait DiscountLayer {
    /// The display name of this layer.
    fn name(&self) -> &str;
    /// Compute the discount from the current net positions.
    ///
    /// Always returns a credit (negative `net_amount`).
    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError>;
}

// ── PercentageDiscount ────────────────────────────────────────────────────────

/// Discount of X% of the net total of selected positions.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct PercentageDiscount {
    /// Display name (e.g. `"Loyalty discount"`).
    pub name: String,
    /// Discount rate as a fraction (e.g. `0.05` = 5%).
    pub rate: Decimal,
    /// Only apply to positions with this tag. `None` = apply to entire net total.
    pub apply_to_tag: Option<String>,
}

impl PercentageDiscount {
    /// Create a `PercentageDiscount` with no tag filter.
    ///
    /// # Panics
    /// Panics if `rate` is outside `[0, 1]`. A rate above 100% would produce a
    /// net charge rather than a credit. Use [`FixedDiscount`] for fixed amounts.
    #[must_use]
    pub fn new(name: impl Into<String>, rate: Decimal) -> Self {
        assert!(
            rate >= Decimal::ZERO && rate <= Decimal::ONE,
            "PercentageDiscount rate must be in [0, 1], got {rate}"
        );
        Self {
            name: name.into(),
            rate,
            apply_to_tag: None,
        }
    }

    #[must_use]
    /// Restrict this discount to positions carrying `tag`.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.apply_to_tag = Some(tag.into());
        self
    }
}

impl DiscountLayer for PercentageDiscount {
    fn name(&self) -> &str {
        &self.name
    }

    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError> {
        let base = Amount::checked_sum(
            positions
                .iter()
                .filter(|p| self.apply_to_tag.as_deref().is_none_or(|t| p.has_tag(t)))
                .filter(|p| p.is_debit()) // discount base is debit positions only
                .map(|p| p.net_amount),
        )?;
        let discount = base.checked_mul_qty(self.rate)?;
        let rate_pct = (self.rate * Decimal::from(100)).normalize();
        LineItem::credit(format!("{} (-{}%)", self.name, rate_pct))
            .fixed_amount(discount)
            .tag("discount")
            .build()
    }
}

// ── FixedDiscount ─────────────────────────────────────────────────────────────

/// Fixed-amount discount (always a credit).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct FixedDiscount {
    /// Display name (e.g. `"Promo voucher"`).
    pub name: String,
    /// Fixed discount amount (always applied as a credit).
    pub amount: Amount<5>,
}

impl FixedDiscount {
    #[must_use]
    /// Create a `FixedDiscount`.
    pub fn new(name: impl Into<String>, amount: Amount<5>) -> Self {
        Self {
            name: name.into(),
            amount,
        }
    }
}

impl DiscountLayer for FixedDiscount {
    fn name(&self) -> &str {
        &self.name
    }

    fn compute(&self, _positions: &[LineItem]) -> Result<LineItem, BillingError> {
        LineItem::credit(&self.name)
            .fixed_amount(self.amount)
            .tag("discount")
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    fn make_item(amount: &str, tag: Option<&str>) -> LineItem {
        let mut b = LineItem::debit("test").fixed_amount(Amount::parse(amount).unwrap());
        if let Some(t) = tag {
            b = b.tag(t);
        }
        b.build().unwrap()
    }

    #[test]
    fn fixed_rate_tax() {
        let positions = vec![make_item("100.00000", None)];
        let tax = FixedRateTax::new("MwSt", dec!(0.19));
        let item = tax.compute(&positions).unwrap();
        assert_eq!(item.net_amount, Amount::parse("19.00000").unwrap());
        assert!(item.has_tag("tax"));
    }

    #[test]
    fn fixed_rate_tax_with_tag_filter() {
        let positions = vec![
            make_item("100.00000", Some("commodity")),
            make_item("20.00000", None),
        ];
        let tax = FixedRateTax::new("MwSt", dec!(0.19)).with_tag("commodity");
        let item = tax.compute(&positions).unwrap();
        // Only 100 × 0.19 = 19, not (100+20) × 0.19
        assert_eq!(item.net_amount, Amount::parse("19.00000").unwrap());
    }

    #[test]
    fn percentage_charge_with_min() {
        let positions = vec![make_item("5.00000", None)];
        let charge = PercentageCharge::new("Platform fee", dec!(0.05))
            .with_min(Amount::parse("0.50000").unwrap());
        let item = charge.compute(&positions).unwrap();
        // 5 × 0.05 = 0.25, but min = 0.50
        assert_eq!(item.net_amount, Amount::parse("0.50000").unwrap());
    }

    #[test]
    fn percentage_charge_with_max() {
        let positions = vec![make_item("10000.00000", None)];
        let charge =
            PercentageCharge::new("Fee", dec!(0.05)).with_max(Amount::parse("100.00000").unwrap());
        let item = charge.compute(&positions).unwrap();
        assert_eq!(item.net_amount, Amount::parse("100.00000").unwrap());
    }

    #[test]
    fn percentage_discount() {
        let positions = vec![make_item("200.00000", None)];
        let disc = PercentageDiscount::new("Loyalty", dec!(0.10));
        let item = disc.compute(&positions).unwrap();
        assert!(item.net_amount.is_negative());
        assert_eq!(item.net_amount, Amount::parse("-20.00000").unwrap());
    }

    #[test]
    fn fixed_discount() {
        let disc = FixedDiscount::new("Voucher", Amount::parse("15.00000").unwrap());
        let item = disc.compute(&[]).unwrap();
        assert_eq!(item.net_amount, Amount::parse("-15.00000").unwrap());
    }
}
