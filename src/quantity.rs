//! [`Quantity`] and [`UnitPrice`] — value + unit-label pairs used in [`crate::LineItem`].
use rust_decimal::Decimal;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, PartialEq)]
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
}
