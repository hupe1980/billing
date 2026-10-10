//! [`Price`], [`UnitPrice`] and [`Quantity`]: the inputs a line multiplies;
//! [`Percent`]: every percentage input, its unit in its type.
//!
//! Constructors are infallible. The checks EN 16931 places on them (price ≥ 0,
//! BR-27; base quantity > 0, PEPPOL-EN16931-R121) run where the value is used,
//! with a [`Location`](crate::Location) saying which line or tier failed.

use alloc::string::String;
use core::fmt;

use rust_decimal::Decimal;

/// A price per unit (BT-146): an exact `Decimal` of any precision, **not** an
/// [`Amount`](crate::Amount). A price such as 0.123456 EUR/kWh is not money until
/// it is multiplied by a quantity and reduced once at the document's scale.
///
/// Serialises as a decimal string.
///
/// ```
/// use billing::Price;
/// use rust_decimal::dec;
///
/// let p = Price::new(dec!(0.123456));
/// assert_eq!(p.value(), dec!(0.123456));
/// assert_eq!(p.to_string(), "0.123456");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Price(#[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))] Decimal);

impl Price {
    /// A price of `value` per unit.
    #[must_use]
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// The price as an exact `Decimal`.
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

impl From<Decimal> for Price {
    fn from(value: Decimal) -> Self {
        Self(value)
    }
}

impl fmt::Display for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// A percentage, exact and with its unit: `Percent::new(dec!(19))` is 19 %,
/// `Percent::new(dec!(0.9))` is 0.9 %. Every percentage input of the crate
/// takes one; there is no `From<Decimal>`, so a bare number never becomes a
/// percentage by accident.
///
/// Construction is infallible; each input checks its own range where it is
/// used (a VAT rate per its category, Skonto and transaction fees `0..=100`).
/// Serialises as the decimal string under a unit-bearing field name
/// (`rate_percent`, `percent`, `annual_percent`).
///
/// ```
/// use billing::Percent;
/// use rust_decimal::dec;
///
/// let corsica = Percent::new(dec!(0.9));
/// assert_eq!(corsica.value(), dec!(0.9));
/// assert_eq!(corsica.to_string(), "0.9 %");
/// assert_eq!(Percent::ZERO.value(), dec!(0));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Percent(
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))] Decimal,
);

impl Percent {
    /// 0 %.
    pub const ZERO: Self = Self(Decimal::ZERO);

    /// `value` percent: `19` for 19 %.
    #[must_use]
    pub const fn new(value: Decimal) -> Self {
        Self(value)
    }

    /// The percent number as an exact `Decimal` (`19` for 19 %).
    #[must_use]
    pub const fn value(self) -> Decimal {
        self.0
    }
}

/// `"19 %"`; honours width and alignment (`format!("{:>6}", p)` is `"  19 %"`).
///
/// ```
/// use billing::Percent;
/// use rust_decimal::dec;
///
/// assert_eq!(format!("{:>6}|", Percent::new(dec!(19))), "  19 %|");
/// assert_eq!(Percent::new(dec!(0.9)).to_string(), "0.9 %");
/// ```
impl fmt::Display for Percent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(&alloc::format!("{} %", self.0))
    }
}

/// A [`Price`] per a base quantity (BT-149) in an optional unit (BT-150):
/// "12.00 per 100 KGM". The base quantity defaults to 1.
///
/// ```
/// use billing::{Price, UnitPrice};
/// use rust_decimal::dec;
///
/// let p = UnitPrice::new(Price::new(dec!(12))).with_base_quantity(dec!(100)).with_unit("KGM");
/// assert_eq!(p.base_quantity(), dec!(100));
/// assert_eq!(p.unit(), Some("KGM"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct UnitPrice {
    price: Price,
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    base_quantity: Decimal,
    unit: Option<String>,
}

impl UnitPrice {
    /// `price` per one unit, no base-quantity unit.
    #[must_use]
    pub const fn new(price: Price) -> Self {
        Self {
            price,
            base_quantity: Decimal::ONE,
            unit: None,
        }
    }

    /// Set the base quantity (BT-149). Must be above zero where it is used.
    #[must_use]
    pub fn with_base_quantity(mut self, base_quantity: Decimal) -> Self {
        self.base_quantity = base_quantity;
        self
    }

    /// Set the base quantity's unit code (BT-150), e.g. `"KWH"`.
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// The price per base quantity.
    #[must_use]
    pub const fn price(&self) -> Price {
        self.price
    }

    /// The base quantity (BT-149).
    #[must_use]
    pub const fn base_quantity(&self) -> Decimal {
        self.base_quantity
    }

    /// The base quantity's unit code (BT-150), if stated.
    #[must_use]
    pub fn unit(&self) -> Option<&str> {
        self.unit.as_deref()
    }
}

impl From<Price> for UnitPrice {
    fn from(price: Price) -> Self {
        Self::new(price)
    }
}

/// A signed quantity (BT-129) in an opaque unit code (BT-130), e.g. `"KWH"` or
/// `"C62"` (one). The sign is the line's direction: negative is a credit.
///
/// ```
/// use billing::Quantity;
/// use rust_decimal::dec;
///
/// let q = Quantity::new(dec!(-3.5), "KWH");
/// assert_eq!((q.value(), q.unit()), (dec!(-3.5), "KWH"));
/// assert_eq!(Quantity::one().unit(), "C62");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Quantity {
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    value: Decimal,
    unit: String,
}

impl Quantity {
    /// `value` of `unit`.
    #[must_use]
    pub fn new(value: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
        }
    }

    /// One piece: 1 `C62`.
    #[must_use]
    pub fn one() -> Self {
        Self::new(Decimal::ONE, "C62")
    }

    /// The signed value.
    #[must_use]
    pub const fn value(&self) -> Decimal {
        self.value
    }

    /// The unit code.
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn constructors_are_infallible_and_keep_inputs() {
        let negative = Price::new(dec!(-1));
        assert_eq!(negative.value(), dec!(-1));
        let p: UnitPrice = Price::from(dec!(2.5)).into();
        assert_eq!(p.price().value(), dec!(2.5));
        assert_eq!(p.base_quantity(), Decimal::ONE);
        assert_eq!(p.unit(), None);
        let zero_base = p.with_base_quantity(Decimal::ZERO);
        assert_eq!(zero_base.base_quantity(), Decimal::ZERO);
        let q = Quantity::one();
        assert_eq!((q.value(), q.unit()), (Decimal::ONE, "C62"));
    }

    #[test]
    fn percent_keeps_its_value_and_shows_its_unit() {
        use alloc::string::ToString;
        let p = Percent::new(dec!(19.5));
        assert_eq!(p.value(), dec!(19.5));
        assert_eq!(p.to_string(), "19.5 %");
        assert_eq!(Percent::ZERO, Percent::default());
        assert_eq!(Percent::ZERO.value(), Decimal::ZERO);
        assert!(Percent::new(dec!(-1)) < Percent::ZERO);
    }
}
