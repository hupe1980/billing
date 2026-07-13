//! [`TimeOfUsePricing`] and [`DynamicPricing`] — time-banded and interval pricing.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::quantity::{Quantity, UnitPrice};

// ── TouBand ───────────────────────────────────────────────────────────────────

/// A named pricing band for time-of-use billing.
///
/// Time intervals are NOT part of this type — callers supply pre-aggregated
/// consumption per band (e.g. from a smart-meter read).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct TouBand {
    /// Display name for this band (e.g. `"HT"`, `"peak"`).
    pub name: String,
    /// Price per unit for this band.
    pub price: Amount<5>,
}

impl TouBand {
    #[must_use]
    /// Create a new `TouBand`.
    pub fn new(name: impl Into<String>, price: Amount<5>) -> Self {
        Self {
            name: name.into(),
            price,
        }
    }
}

// ── TimeOfUsePricing ──────────────────────────────────────────────────────────

/// N-band time-of-use pricing (HT/NT, peak/off-peak/super-peak, …).
///
/// Callers supply pre-aggregated consumption per band name.
/// The engine has no knowledge of time zones, BNetzA schedules, or
/// any other time-related law.
///
/// # Unit labels
///
/// Call `.with_unit("kWh")` to propagate the correct unit label into all
/// generated `LineItem`s. Defaults to `"units"`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct TimeOfUsePricing {
    bands: Vec<TouBand>,
    unit: String,
}

impl TimeOfUsePricing {
    #[must_use]
    /// Create a `TimeOfUsePricing` from a list of bands. Unit defaults to `"units"`.
    pub fn new(bands: Vec<TouBand>) -> Self {
        Self {
            bands,
            unit: "units".into(),
        }
    }

    /// Set the quantity unit label (e.g. `"kWh"`, `"m³"`, `"seats"`).
    ///
    /// Used in all generated `LineItem` quantity and unit-price labels.
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Calculate billing positions from `(band_name, quantity)` pairs.
    ///
    /// Unknown band names are silently skipped (forward-compatible).
    /// Negative quantities return `Err` — a negative consumption reading
    /// is a data error, not a valid billing input.
    pub fn calculate(&self, usage: &[(&str, Decimal)]) -> Result<Vec<LineItem>, BillingError> {
        let price_unit = format!("EUR/{}", self.unit);
        let mut items = Vec::with_capacity(usage.len());
        for (band_name, qty) in usage {
            if *qty < Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "TimeOfUsePricing: usage quantity must be non-negative",
                });
            }
            if let Some(band) = self.bands.iter().find(|b| b.name == *band_name) {
                if *qty > Decimal::ZERO {
                    items.push(
                        LineItem::debit(format!("{} ({})", self.unit, band_name))
                            .quantity(Quantity::new(*qty, &self.unit))
                            .unit_price(UnitPrice::new(band.price.into_decimal(), &price_unit))
                            .tag(band_name.to_string())
                            .build()?,
                    );
                }
            }
        }
        Ok(items)
    }
}

// ── DynamicPricing ────────────────────────────────────────────────────────────

/// Per-interval price sequence (EPEX Spot, AWS spot, real-time tariffs).
///
/// Each interval supplies `(quantity, price_per_unit)`. The engine does not
/// know about EPEX, day-ahead markets, or any specific price source.
///
/// # Unit labels
///
/// Call `.with_unit("kWh")` to propagate the correct unit label.
/// Defaults to `"units"`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct DynamicPricing {
    intervals: Vec<(Decimal, Amount<5>)>,
    unit: String,
}

impl DynamicPricing {
    /// Construct from `(quantity, price_per_unit)` pairs.
    pub fn from_intervals(intervals: Vec<(Decimal, Amount<5>)>) -> Result<Self, BillingError> {
        if intervals.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "DynamicPricing requires at least one interval",
            });
        }
        // Every interval quantity must be positive — negative quantities
        // represent returns/refunds which should be modelled as separate
        // credit line items, not as negative pricing intervals.
        for (qty, _) in &intervals {
            if *qty <= Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "DynamicPricing interval quantity must be > 0",
                });
            }
        }
        Ok(Self {
            intervals,
            unit: "units".into(),
        })
    }

    /// Set the quantity unit label (e.g. `"kWh"`, `"GB"`, `"req"`).
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Compute a single `LineItem` with the total charge.
    ///
    /// `net = Σ(qty_i × price_i)`. The quantity field shows the total quantity.
    /// The unit-price field shows the weighted average price (informational only —
    /// `net_amount` is set directly from the exact accumulated total, not from
    /// `total_quantity × average_price`, which would introduce rounding error).
    pub fn calculate(&self) -> Result<LineItem, BillingError> {
        let mut total_qty = Decimal::ZERO;
        let mut total_net = Amount::<5>::ZERO;
        for (qty, price) in &self.intervals {
            total_qty += qty;
            total_net = total_net.checked_add(price.checked_mul_qty(*qty)?)?;
        }
        let avg_price = if total_qty.is_zero() {
            Decimal::ZERO
        } else {
            total_net.into_decimal() / total_qty
        };
        let price_unit = format!("EUR/{} (wtd avg)", self.unit);
        // Use fixed_amount to set the net exactly from the accumulated total,
        // not from total_qty × avg_price which may differ by a rounding unit.
        LineItem::debit(format!("Dynamic pricing ({})", self.unit))
            .quantity(Quantity::new(total_qty, &self.unit))
            .unit_price(UnitPrice::new(avg_price, &price_unit))
            .fixed_amount(total_net)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

    #[test]
    fn tou_two_bands_with_unit() {
        let tou = TimeOfUsePricing::new(vec![
            TouBand::new("HT", Amount::parse("0.32000").unwrap()),
            TouBand::new("NT", Amount::parse("0.18000").unwrap()),
        ])
        .with_unit("kWh");
        let items = tou
            .calculate(&[("HT", dec!(823.4)), ("NT", dec!(411.1))])
            .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].unit_label(), Some("kWh"));
        // HT: 823.4 × 0.32 = 263.488
        assert_eq!(items[0].net_amount, Amount::parse("263.48800").unwrap());
        // NT: 411.1 × 0.18 = 73.998
        assert_eq!(items[1].net_amount, Amount::parse("73.99800").unwrap());
    }

    #[test]
    fn tou_default_unit_is_units() {
        let tou = TimeOfUsePricing::new(vec![TouBand::new(
            "peak",
            Amount::parse("0.10000").unwrap(),
        )]);
        let items = tou.calculate(&[("peak", dec!(100))]).unwrap();
        assert_eq!(items[0].unit_label(), Some("units"));
    }

    #[test]
    fn dynamic_pricing_weighted_avg() {
        let dp = DynamicPricing::from_intervals(vec![
            (dec!(100), Amount::parse("0.10000").unwrap()),
            (dec!(200), Amount::parse("0.20000").unwrap()),
        ])
        .unwrap()
        .with_unit("kWh");
        let item = dp.calculate().unwrap();
        assert_eq!(item.quantity_value(), Some(dec!(300)));
        assert_eq!(item.unit_label(), Some("kWh"));
        assert_eq!(item.net_amount, Amount::parse("50.00000").unwrap());
    }
}
