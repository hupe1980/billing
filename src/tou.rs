//! [`TimeOfUsePricing`] and [`DynamicPricing`] — time-banded and interval pricing.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::currency::Currency;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::quantity::{Quantity, UnitPrice};
use crate::tags;

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
/// # Construction
///
/// Built through [`TimeOfUsePricing::builder`], matching
/// [`crate::TariffSchedule`]: the setters are infallible and chainable, and every
/// check happens once in [`TimeOfUsePricingBuilder::build`].
///
/// ```rust
/// use billing::{Amount, Currency, TimeOfUsePricing, TouBand};
/// use rust_decimal::dec;
///
/// let tou = TimeOfUsePricing::builder()
///     .unit("kWh")
///     .currency(Currency::EUR)
///     .band(TouBand::new("HT", Amount::parse("0.32000")?))
///     .band(TouBand::new("NT", Amount::parse("0.18000")?))
///     .build()?;
///
/// assert_eq!(tou.calculate(&[("HT", dec!(100)), ("NT", dec!(50))])?.len(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "TimeOfUsePricingRepr"))]
#[derive(Debug, Clone)]
pub struct TimeOfUsePricing {
    bands: Vec<TouBand>,
    unit: String,
    currency: Currency,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct TimeOfUsePricingRepr {
    bands: Vec<TouBand>,
    unit: String,
    #[serde(default)]
    currency: Currency,
}

#[cfg(feature = "serde")]
impl TryFrom<TimeOfUsePricingRepr> for TimeOfUsePricing {
    type Error = BillingError;
    fn try_from(r: TimeOfUsePricingRepr) -> Result<Self, Self::Error> {
        // Routed through the builder so deserialisation cannot reach a state the
        // constructor rejects — including the `unit`, which an assign-after-new
        // shim used to skip.
        TimeOfUsePricingBuilder {
            bands: r.bands,
            unit: r.unit,
            currency: r.currency,
        }
        .build()
    }
}

/// Builder for [`TimeOfUsePricing`]. Obtain via [`TimeOfUsePricing::builder`].
#[derive(Debug, Clone)]
pub struct TimeOfUsePricingBuilder {
    bands: Vec<TouBand>,
    unit: String,
    currency: Currency,
}

impl Default for TimeOfUsePricingBuilder {
    fn default() -> Self {
        Self {
            bands: Vec::new(),
            unit: "units".into(),
            currency: Currency::XXX,
        }
    }
}

impl TimeOfUsePricingBuilder {
    /// Append a pricing band.
    #[must_use]
    pub fn band(mut self, band: TouBand) -> Self {
        self.bands.push(band);
        self
    }

    /// Append several pricing bands.
    #[must_use]
    pub fn bands(mut self, bands: impl IntoIterator<Item = TouBand>) -> Self {
        self.bands.extend(bands);
        self
    }

    /// Set the quantity unit label (e.g. `"kWh"`, `"m³"`, `"seats"`).
    ///
    /// Defaults to `"units"`. Validated by [`TimeOfUsePricingBuilder::build`].
    #[must_use]
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Set the currency used in generated unit-price labels.
    ///
    /// Defaults to [`Currency::XXX`] — a label reading `XXX/kWh` means no currency
    /// was configured.
    #[must_use]
    pub fn currency(mut self, currency: Currency) -> Self {
        self.currency = currency;
        self
    }

    /// Validate and build.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if there are no bands, if the unit label is
    /// empty, if a band name is empty, reserved (see [`crate::tags`]) or duplicated,
    /// or if a band price is negative.
    pub fn build(self) -> Result<TimeOfUsePricing, BillingError> {
        TimeOfUsePricing::assemble(self.bands, self.unit, self.currency)
    }
}

impl TimeOfUsePricing {
    /// Start building. See [`TimeOfUsePricingBuilder`].
    #[must_use]
    pub fn builder() -> TimeOfUsePricingBuilder {
        TimeOfUsePricingBuilder::default()
    }

    fn assemble(
        bands: Vec<TouBand>,
        unit: String,
        currency: Currency,
    ) -> Result<Self, BillingError> {
        if bands.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "TimeOfUsePricing requires at least one band".into(),
            });
        }
        for (i, b) in bands.iter().enumerate() {
            if b.name.trim().is_empty() {
                return Err(BillingError::InvalidInput {
                    reason: "TimeOfUsePricing band name must not be empty".into(),
                });
            }
            // `calculate` tags each generated position with its band name, and the
            // engine reserves a few tag values to classify positions. A band called
            // "tax" would make its consumption line look like a tax line to
            // `PerUnitLevy`, which excludes those from its base — silently levying
            // on a fraction of the energy actually consumed, with no error.
            if tags::is_reserved(&b.name) {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "TimeOfUsePricing band name {:?} is reserved by the engine \
                         (reserved: {}); rename the band",
                        b.name,
                        tags::RESERVED.join(", ")
                    ),
                });
            }
            if b.price.is_negative() {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "TimeOfUsePricing band {:?} price must be >= 0, got {}",
                        b.name, b.price
                    ),
                });
            }
            if bands[..i].iter().any(|prev| prev.name == b.name) {
                return Err(BillingError::InvalidInput {
                    reason: format!("TimeOfUsePricing duplicate band name {:?}", b.name),
                });
            }
        }
        Ok(Self {
            bands,
            unit: crate::validate_unit(unit)?,
            currency,
        })
    }

    /// The configured band names, in declaration order.
    pub fn band_names(&self) -> impl Iterator<Item = &str> {
        self.bands.iter().map(|b| b.name.as_str())
    }

    /// The quantity unit label.
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The currency used in generated unit-price labels.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.currency
    }

    /// Calculate billing positions from `(band_name, quantity)` pairs.
    ///
    /// # Unknown band names are an error
    ///
    /// A usage entry naming a band this schedule does not define returns
    /// [`BillingError::InvalidInput`].  Earlier versions skipped such entries
    /// silently, which meant a typo in a band name (`"HT "`, `"ht"`, a renamed
    /// tariff) **dropped real consumption from the invoice** with no signal —
    /// systematic under-billing that no test or validation step could detect.
    ///
    /// Negative quantities return `Err` — a negative consumption reading
    /// is a data error, not a valid billing input.
    ///
    /// Zero-quantity entries are skipped (no zero-value line on the invoice).
    ///
    /// Generic over the key type, so a `Vec<(String, Decimal)>` read from config or
    /// a database works directly without rebuilding it as borrowed tuples.
    ///
    /// ```rust
    /// use billing::{TimeOfUsePricing, TouBand, Amount, Currency};
    /// use rust_decimal::dec;
    ///
    /// let tou = TimeOfUsePricing::builder()
    ///     .unit("kWh")
    ///     .currency(Currency::EUR)
    ///     .band(TouBand::new("HT", Amount::parse("0.32000").unwrap()))
    ///     .build()
    ///     .unwrap();
    ///
    /// assert!(tou.calculate(&[("HT", dec!(100))]).is_ok());
    /// // A typo is caught rather than silently dropping 100 kWh:
    /// assert!(tou.calculate(&[("ht", dec!(100))]).is_err());
    /// ```
    pub fn calculate<S: AsRef<str>>(
        &self,
        usage: &[(S, Decimal)],
    ) -> Result<Vec<LineItem>, BillingError> {
        let price_unit = format!("{}/{}", self.currency, self.unit);
        let mut items = Vec::with_capacity(usage.len());
        for (band_name, qty) in usage {
            let band_name = band_name.as_ref();
            if *qty < Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "TimeOfUsePricing: usage quantity must be non-negative".into(),
                });
            }
            let band = self
                .bands
                .iter()
                .find(|b| b.name == band_name)
                .ok_or_else(|| BillingError::InvalidInput {
                    reason: format!(
                        "TimeOfUsePricing: unknown band {band_name:?} (defined bands: {})",
                        self.bands
                            .iter()
                            .map(|b| b.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?;
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
        Ok(items)
    }
}

// ── DynamicPricing ────────────────────────────────────────────────────────────

/// Per-interval price sequence (EPEX Spot, AWS spot, real-time tariffs).
///
/// Each interval supplies `(quantity, price_per_unit)`. The engine does not
/// know about EPEX, day-ahead markets, or any specific price source.
///
/// # Construction
///
/// Built through [`DynamicPricing::builder`], matching every other pricing type
/// in the crate: infallible chainable setters, one fallible `build()`.
///
/// ```rust
/// use billing::{Amount, Currency, DynamicPricing};
/// use rust_decimal::dec;
///
/// let dp = DynamicPricing::builder()
///     .unit("kWh")
///     .currency(Currency::EUR)
///     .interval(dec!(100), Amount::parse("0.10000")?)
///     .interval(dec!(200), Amount::parse("0.20000")?)
///     .build()?;
///
/// assert_eq!(dp.calculate()?.net_amount, Amount::parse("50.00000")?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "DynamicPricingRepr"))]
#[derive(Debug, Clone)]
pub struct DynamicPricing {
    intervals: Vec<(Decimal, Amount<5>)>,
    unit: String,
    currency: Currency,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct DynamicPricingRepr {
    intervals: Vec<(Decimal, Amount<5>)>,
    unit: String,
    #[serde(default)]
    currency: Currency,
}

#[cfg(feature = "serde")]
impl TryFrom<DynamicPricingRepr> for DynamicPricing {
    type Error = BillingError;
    fn try_from(r: DynamicPricingRepr) -> Result<Self, Self::Error> {
        DynamicPricingBuilder {
            intervals: r.intervals,
            unit: r.unit,
            currency: r.currency,
        }
        .build()
    }
}

/// Builder for [`DynamicPricing`]. Obtain via [`DynamicPricing::builder`].
#[derive(Debug, Clone)]
pub struct DynamicPricingBuilder {
    intervals: Vec<(Decimal, Amount<5>)>,
    unit: String,
    currency: Currency,
}

impl Default for DynamicPricingBuilder {
    fn default() -> Self {
        Self {
            intervals: Vec::new(),
            unit: "units".into(),
            currency: Currency::XXX,
        }
    }
}

impl DynamicPricingBuilder {
    /// Append one `(quantity, price_per_unit)` interval.
    #[must_use]
    pub fn interval(mut self, quantity: Decimal, price: Amount<5>) -> Self {
        self.intervals.push((quantity, price));
        self
    }

    /// Append several `(quantity, price_per_unit)` intervals.
    #[must_use]
    pub fn intervals(mut self, intervals: impl IntoIterator<Item = (Decimal, Amount<5>)>) -> Self {
        self.intervals.extend(intervals);
        self
    }

    /// Set the quantity unit label (e.g. `"kWh"`, `"GB"`, `"req"`).
    #[must_use]
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Set the currency used in the generated unit-price label.
    #[must_use]
    pub fn currency(mut self, currency: Currency) -> Self {
        self.currency = currency;
        self
    }

    /// Validate and build.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if there are no intervals, if the unit label
    /// is empty, or if any interval quantity is not strictly positive.
    pub fn build(self) -> Result<DynamicPricing, BillingError> {
        DynamicPricing::assemble(self.intervals, self.unit, self.currency)
    }
}

impl DynamicPricing {
    /// Start building. See [`DynamicPricingBuilder`].
    #[must_use]
    pub fn builder() -> DynamicPricingBuilder {
        DynamicPricingBuilder::default()
    }

    fn assemble(
        intervals: Vec<(Decimal, Amount<5>)>,
        unit: String,
        currency: Currency,
    ) -> Result<Self, BillingError> {
        if intervals.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "DynamicPricing requires at least one interval".into(),
            });
        }
        // Every interval quantity must be positive — negative quantities
        // represent returns/refunds which should be modelled as separate
        // credit line items, not as negative pricing intervals.
        for (qty, _) in &intervals {
            if *qty <= Decimal::ZERO {
                return Err(BillingError::InvalidInput {
                    reason: "DynamicPricing interval quantity must be > 0".into(),
                });
            }
        }
        Ok(Self {
            intervals,
            unit: crate::validate_unit(unit)?,
            currency,
        })
    }

    /// The quantity unit label.
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The currency used in the generated unit-price label.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.currency
    }

    /// Compute a single `LineItem` with the total charge.
    ///
    /// `net = Σ(qty_i × price_i)`. The quantity field shows the total quantity.
    /// The unit-price field shows the weighted average price (informational only —
    /// `net_amount` is set directly from the exact accumulated total, not from
    /// `total_quantity × average_price`, which would introduce rounding error).
    pub fn calculate(&self) -> Result<LineItem, BillingError> {
        // `checked_*` throughout: `Decimal`'s `+=` and `/` panic on overflow and
        // on division by zero, which would break this method's `Result` contract.
        let overflow = || BillingError::MonetaryOverflow {
            precision: 5,
            input_value: None,
        };
        let mut total_qty = Decimal::ZERO;
        let mut total_net = Amount::<5>::ZERO;
        for (qty, price) in &self.intervals {
            total_qty = total_qty.checked_add(*qty).ok_or_else(overflow)?;
            total_net = total_net.checked_add(price.checked_mul_qty(*qty)?)?;
        }
        let avg_price = if total_qty.is_zero() {
            Decimal::ZERO
        } else {
            total_net
                .into_decimal()
                .checked_div(total_qty)
                .ok_or_else(overflow)?
        };
        let price_unit = format!("{}/{} (wtd avg)", self.currency, self.unit);
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
    use rust_decimal::dec;

    #[test]
    fn tou_two_bands_with_unit() {
        let tou = TimeOfUsePricing::builder()
            .unit("kWh")
            .band(TouBand::new("HT", Amount::parse("0.32000").unwrap()))
            .band(TouBand::new("NT", Amount::parse("0.18000").unwrap()))
            .build()
            .unwrap();
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
        let tou = TimeOfUsePricing::builder()
            .band(TouBand::new("peak", Amount::parse("0.10000").unwrap()))
            .build()
            .unwrap();
        let items = tou.calculate(&[("peak", dec!(100))]).unwrap();
        assert_eq!(items[0].unit_label(), Some("units"));
    }

    #[test]
    fn dynamic_pricing_weighted_avg() {
        let dp = DynamicPricing::builder()
            .unit("kWh")
            .interval(dec!(100), Amount::parse("0.10000").unwrap())
            .interval(dec!(200), Amount::parse("0.20000").unwrap())
            .build()
            .unwrap();
        let item = dp.calculate().unwrap();
        assert_eq!(item.quantity_value(), Some(dec!(300)));
        assert_eq!(item.unit_label(), Some("kWh"));
        assert_eq!(item.net_amount, Amount::parse("50.00000").unwrap());
    }
}
