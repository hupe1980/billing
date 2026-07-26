//! [`Quantity`] and [`UnitPrice`] — value + unit-label pairs used in [`crate::LineItem`].
use rust_decimal::Decimal;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
/// A measured quantity: a numeric value paired with a unit label.
pub struct Quantity {
    /// The numeric value.
    pub value: Decimal,
    /// Unit label, e.g. `"kWh"`, `"m³"`, `"seats"`, `"GB"`.
    pub unit: String,
}

impl Quantity {
    #[must_use]
    /// Create a new `Quantity`.
    pub fn new(value: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
        }
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
/// A unit price: a [`Decimal`] value paired with a unit label.
///
/// Stored as [`rust_decimal::Decimal`] (not [`crate::Amount`]) because unit
/// prices often require higher precision than invoice totals.
pub struct UnitPrice {
    /// The price per unit as an exact decimal.
    pub value: Decimal,
    /// Price unit label, e.g. `"EUR/kWh"`, `"EUR/seat/month"`.
    pub unit: String, // e.g. "EUR/kWh", "EUR/seat/month"
}

impl UnitPrice {
    #[must_use]
    /// Create a new `UnitPrice`.
    pub fn new(value: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
        }
    }

    /// Round the price to `scale` decimal places with an explicit strategy.
    ///
    /// Use when a price is *derived* rather than quoted — a `ct/kWh → EUR/kWh`
    /// division, or a total divided by a quantity — and the exact quotient carries
    /// more decimals than the price should display. Without this the stored
    /// `unit_price` and the invoice's own `quantity × unit_price` arithmetic can
    /// disagree in the last digits.
    ///
    /// `scale` is clamped to 28, [`Decimal`]'s maximum: above that
    /// `round_dp_with_strategy` silently no-ops, which would leave the price
    /// unrounded while the caller was promised `scale` decimals.
    ///
    /// This replaces the former `LineItem::for_usage_rounded` /
    /// `credit_for_usage_rounded` constructors, which needed seven positional
    /// arguments to say the same thing.
    ///
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice, RoundingStrategy};
    /// use rust_decimal::dec;
    ///
    /// // 1/3 ct/kWh as EUR/kWh is non-terminating — pin it to 6 decimals.
    /// let price = UnitPrice::new(dec!(1) / dec!(300), "EUR/kWh")
    ///     .rounded(6, RoundingStrategy::MidpointAwayFromZero);
    /// assert_eq!(price.value, dec!(0.003333));
    ///
    /// let item = LineItem::for_usage("Arbeit", Quantity::new(dec!(500), "kWh"), price)
    ///     .build().unwrap();
    /// assert_eq!(item.net_amount, Amount::<5>::parse("1.66650").unwrap());
    /// ```
    #[must_use]
    pub fn rounded(mut self, scale: u32, strategy: crate::amount::RoundingStrategy) -> Self {
        /// `Decimal`'s maximum representable scale.
        const MAX_DECIMAL_SCALE: u32 = 28;
        self.value = self
            .value
            .round_dp_with_strategy(scale.min(MAX_DECIMAL_SCALE), strategy.into());
        self
    }
}
