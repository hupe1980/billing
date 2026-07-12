//! [`LineItem`] — the atomic billing unit: quantity × unit-price → net amount.
use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::amount::Amount;
use crate::error::BillingError;
use crate::quantity::{Quantity, UnitPrice};

// ── LineItem ──────────────────────────────────────────────────────────────────

/// The atomic billing unit: a single charge or credit position.
///
/// Every `LineItem` has a `net_amount`: positive = debit (charge), negative = credit.
///
/// Tags allow selective tax/discount application without brittle string matching.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
pub struct LineItem {
    /// Human-readable description shown on the invoice.
    pub description: String,
    /// Measured quantity (value + unit label), if applicable.
    pub quantity: Option<Quantity>,
    /// Price per unit (value + unit label), if applicable.
    pub unit_price: Option<UnitPrice>,
    /// Pre-computed net amount; positive = charge, negative = credit.
    pub net_amount: Amount<5>,
    /// Arbitrary labels for selective tax/discount filtering and ERP categorization.
    pub tags: Vec<String>,
    /// Arbitrary key-value metadata for ERP export.
    pub metadata: HashMap<String, String>,
}

impl LineItem {
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

    /// Returns `true` if this position has the given tag.
    #[must_use]
    pub fn has_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

    /// Build the `LineItem`.
    ///
    /// Net amount is:
    /// 1. `fixed_amount` if set (ignores quantity/unit_price)
    /// 2. `quantity.value × unit_price.value` rounded to 5dp
    /// 3. `Err` if neither is provided
    ///
    /// # Errors
    /// Returns `Err` if description is empty, quantity is negative, unit price
    /// is negative (on the qty×price path), or neither `fixed_amount` nor
    /// `quantity + unit_price` is provided.
    pub fn build(self) -> Result<LineItem, BillingError> {
        // A line item without a description is not auditable.
        if self.description.trim().is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "LineItem description must not be empty",
            });
        }
        let net = if let Some(fixed) = self.fixed_amount {
            fixed
        } else if let (Some(qty), Some(price)) = (&self.quantity, &self.unit_price) {
            // Quantity must be non-negative; a negative quantity on a debit or
            // credit line is a caller error (model refunds via Sign::Credit, not
            // by negating the quantity).
            if qty.value < rust_decimal::Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "LineItem quantity must be non-negative",
                });
            }
            // Unit price must be non-negative on the qty×price path.
            // To model a credit, use Sign::Credit (LineItem::credit) with
            // a positive price, or use fixed_amount directly.
            if price.value < rust_decimal::Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "LineItem unit price must be non-negative; use LineItem::credit() for credits",
                });
            }
            let raw = qty.value * price.value;
            let rounded =
                raw.round_dp_with_strategy(5, rust_decimal::RoundingStrategy::MidpointAwayFromZero);
            Amount::<5>::from_decimal(rounded)
                .ok_or(BillingError::MonetaryOverflow { precision: 5 })?
        } else {
            return Err(BillingError::InvalidInput {
                reason: "LineItem requires either fixed_amount or both quantity and unit_price",
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
            tags: self.tags,
            metadata: self.metadata,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

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
