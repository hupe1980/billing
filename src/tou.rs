//! [`TimeOfUsePricing`] and [`DynamicPricing`] — time-banded and interval pricing.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::currency::Currency;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::quantity::{Quantity, UnitPrice};
use crate::tags;

// ── TimeBand ───────────────────────────────────────────────────────────────────

/// A named pricing band for time-of-use billing.
///
/// Time intervals are NOT part of this type — callers supply pre-aggregated
/// consumption per band (e.g. from a smart-meter read).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct TimeBand {
    /// Display name for this band (e.g. `"HT"`, `"peak"`).
    pub name: String,
    /// Price per unit for this band.
    pub price: Amount<5>,
}

impl TimeBand {
    #[must_use]
    /// Create a new `TimeBand`.
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
/// [`crate::RateSchedule`]: the setters are infallible and chainable, and every
/// check happens once in [`TimeOfUsePricingBuilder::build`].
///
/// ```rust
/// use billing::{Amount, Currency, TimeOfUsePricing, TimeBand};
/// use rust_decimal::dec;
///
/// let tou = TimeOfUsePricing::builder()
///     .unit("kWh")
///     .currency(Currency::EUR)
///     .band(TimeBand::new("HT", Amount::parse("0.32000")?))
///     .band(TimeBand::new("NT", Amount::parse("0.18000")?))
///     .build()?;
///
/// assert_eq!(tou.calculate(&[("HT", dec!(100)), ("NT", dec!(50))])?.len(), 2);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "TimeOfUsePricingRepr"))]
#[derive(Debug, Clone)]
pub struct TimeOfUsePricing {
    bands: Vec<TimeBand>,
    unit: String,
    currency: Currency,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct TimeOfUsePricingRepr {
    bands: Vec<TimeBand>,
    unit: String,
    #[serde(default)]
    currency: Currency,
}

#[cfg(feature = "serde")]
impl TryFrom<TimeOfUsePricingRepr> for TimeOfUsePricing {
    type Error = BillingError;
    fn try_from(r: TimeOfUsePricingRepr) -> Result<Self, Self::Error> {
        // Routed through the builder so deserialisation cannot reach a state the
        // constructor rejects — including the `unit`, which an assign-after-new shim
        // would skip.
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
    bands: Vec<TimeBand>,
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
    pub fn band(mut self, band: TimeBand) -> Self {
        self.bands.push(band);
        self
    }

    /// Append several pricing bands.
    #[must_use]
    pub fn bands(mut self, bands: impl IntoIterator<Item = TimeBand>) -> Self {
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
        bands: Vec<TimeBand>,
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
    /// [`BillingError::InvalidInput`] rather than being skipped. Skipping it means a
    /// typo in a band name (`"HT "`, `"ht"`, a renamed tariff) **drops real
    /// consumption from the invoice** with no signal — systematic under-billing that
    /// no test or validation step can detect.
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
    /// use billing::{TimeOfUsePricing, TimeBand, Amount, Currency};
    /// use rust_decimal::dec;
    ///
    /// let tou = TimeOfUsePricing::builder()
    ///     .unit("kWh")
    ///     .currency(Currency::EUR)
    ///     .band(TimeBand::new("HT", Amount::parse("0.32000").unwrap()))
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
    /// `net = Σ(qty_i × price_i)`, accumulated **exactly** and rounded once at the
    /// end. The quantity field shows the total quantity, and the unit-price field
    /// the weighted average price.
    ///
    /// # Render the average, do not re-derive it
    ///
    /// The returned line is a *complete* position: the weighted average is already
    /// on it, as the [`crate::UnitPrice`] labelled `"EUR/kWh (wtd avg)"`. That is
    /// the figure to print next to the amount — §41a EnWG dynamic-tariff bills show
    /// it, and the customer divides one by the other expecting to find it.
    ///
    /// Computing it yourself as `net_amount ÷ quantity` gives a *different* number,
    /// and a worse one. `net_amount` has been rounded to the engine's five decimals;
    /// dividing it is a second rounding of an already-rounded figure, which is the
    /// one thing this crate never does (see
    /// [`crate::AmountScale::apply_decimal`]). The average below is derived from the
    /// **exact** accumulated total, so it is the price that actually reproduces the
    /// interval sequence.
    ///
    /// The temptation is structural rather than careless: [`crate::RateSchedule`]
    /// returns one line *per band*, so an aggregate here reads as a summary that a
    /// caller then takes fields off piecemeal. It is not — it is one invoice line,
    /// fully populated, and `PEPPOL-EN16931-R120` is checkable on it with
    /// [`crate::LineItem::check_price_consistency`].
    ///
    /// # One rounding, not one per interval
    ///
    /// A day-ahead sequence is 24 intervals; a settled year is 8 760, and a
    /// quarter-hourly one 35 040. Rounding each interval's product to the engine's
    /// five decimals before adding it would bias the total by up to `5 × 10⁻⁶` per
    /// interval *in the same direction*, because commercial rounding is not
    /// unbiased on a repeated fractional part. Over 8 760 hourly intervals that is
    /// up to `0.0438` — four whole cents on a two-decimal invoice, from arithmetic
    /// alone.
    ///
    /// So the products are accumulated as exact [`Decimal`]s and reduced **once**,
    /// which is the same rule [`crate::AmountScale::apply_decimal`] states for the
    /// interchange boundary: two roundings are not one. Unlike a
    /// [`crate::TimeOfUsePricing`] band or a [`crate::RateSchedule`] tier, an
    /// interval is *not* a leaf — it never becomes its own invoice line, so there
    /// is nothing at that level to round to.
    ///
    /// ```rust
    /// use billing::{Amount, Currency, DynamicPricing};
    /// use rust_decimal::dec;
    ///
    /// // Three intervals, each exactly half a smallest unit: 0.5 × 0.00001.
    /// let dp = DynamicPricing::builder()
    ///     .unit("kWh")
    ///     .currency(Currency::EUR)
    ///     .intervals(std::iter::repeat_n((dec!(0.5), Amount::parse("0.00001")?), 3))
    ///     .build()?;
    ///
    /// // Exact total 0.000015 → 0.00002. Rounding each interval first would have
    /// // taken every one of them up to 0.00001 and reported 0.00003.
    /// assert_eq!(dp.calculate()?.net_amount, Amount::parse("0.00002")?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn calculate(&self) -> Result<LineItem, BillingError> {
        // `checked_*` throughout: `Decimal`'s `+`, `*` and `/` panic on overflow and
        // on division by zero, which would break this method's `Result` contract.
        let overflow = || BillingError::MonetaryOverflow {
            precision: 5,
            input_value: None,
        };
        let mut total_qty = Decimal::ZERO;
        let mut exact_net = Decimal::ZERO;
        for (qty, price) in &self.intervals {
            total_qty = total_qty.checked_add(*qty).ok_or_else(overflow)?;
            let product = price
                .into_decimal()
                .checked_mul(*qty)
                .ok_or_else(overflow)?;
            exact_net = exact_net.checked_add(product).ok_or_else(overflow)?;
        }
        // The single rounding, from the exact sum.
        let total_net = Amount::<5>::from_decimal_rounded(
            exact_net,
            crate::RoundingStrategy::MidpointAwayFromZero,
        )?;
        // Derived from the exact total too: an average taken from the rounded net
        // would restate the rounding error as a price.
        let avg_price = if total_qty.is_zero() {
            Decimal::ZERO
        } else {
            exact_net.checked_div(total_qty).ok_or_else(overflow)?
        };
        let price_unit = format!("{}/{} (wtd avg)", self.currency, self.unit);
        // `fixed_amount` sets the net from the accumulated total, not from
        // `total_qty × avg_price`, which would round a second time.
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
            .band(TimeBand::new("HT", Amount::parse("0.32000").unwrap()))
            .band(TimeBand::new("NT", Amount::parse("0.18000").unwrap()))
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
            .band(TimeBand::new("peak", Amount::parse("0.10000").unwrap()))
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

    /// A settled spot-price year is 8 760 hourly intervals. Rounding each
    /// interval's product to the engine's five decimals before summing biases the
    /// total in one direction — `0.0438` here, four cents on a two-decimal
    /// invoice — because commercial rounding is not unbiased on a repeated
    /// fractional part. The products are accumulated exactly and reduced once.
    #[test]
    fn dynamic_pricing_rounds_once_over_the_whole_sequence() {
        let mut b = DynamicPricing::builder()
            .unit("kWh")
            .currency(Currency::EUR);
        for _ in 0..8_760 {
            b = b.interval(dec!(1.000005), Amount::parse("1.00000").unwrap());
        }
        let item = b.build().unwrap().calculate().unwrap();

        // 8760 × 1.000005 = 8760.0438 exactly, which is representable at 5 dp.
        assert_eq!(item.net_amount, Amount::parse("8760.04380").unwrap());
        // Per-interval rounding would have produced this instead.
        assert_ne!(item.net_amount, Amount::parse("8760.08760").unwrap());
        assert_eq!(item.quantity_value(), Some(dec!(8760.043800)));
    }

    /// The weighted-average price is derived from the *exact* accumulated total,
    /// not from the rounded net: deriving it from the rounded figure would restate
    /// the rounding error as a price and make `quantity × unit_price` disagree with
    /// the stated amount by more than one rounding.
    #[test]
    fn dynamic_pricing_average_price_comes_from_the_exact_total() {
        let dp = DynamicPricing::builder()
            .unit("kWh")
            .intervals(std::iter::repeat_n(
                (dec!(0.5), Amount::parse("0.00001").unwrap()),
                3,
            ))
            .build()
            .unwrap();
        let item = dp.calculate().unwrap();
        // Exact: 3 × 0.000005 = 0.000015 → 0.00002 at five decimals.
        assert_eq!(item.net_amount, Amount::parse("0.00002").unwrap());
        // 0.000015 / 1.5 = 0.00001 — the price actually quoted, not 0.0000133….
        assert_eq!(item.unit_price.as_ref().unwrap().value, dec!(0.00001));
    }

    #[test]
    fn the_plural_band_setter_and_the_currency_reach_the_built_pricing() {
        // `bands` and `currency` are the two builder steps nothing else in the
        // suite goes through — every other test appends bands one at a time and
        // leaves the currency at its default. A setter that dropped what it was
        // handed would leave `XXX/kWh` on the invoice, which reads as a price and
        // is not one.
        let pricing = TimeOfUsePricing::builder()
            .unit("kWh")
            .currency(Currency::EUR)
            .bands([
                TimeBand::new("peak", Amount::parse("0.35000").unwrap()),
                TimeBand::new("offpeak", Amount::parse("0.22000").unwrap()),
            ])
            .band(TimeBand::new("night", Amount::parse("0.15000").unwrap()))
            .build()
            .unwrap();

        // All three arrived, in declaration order, with the plural setter's bands
        // first because it was called first.
        assert_eq!(
            pricing.band_names().collect::<Vec<_>>(),
            ["peak", "offpeak", "night"]
        );
        assert_eq!(pricing.unit(), "kWh");
        assert_eq!(pricing.currency(), Currency::EUR);
        assert_ne!(pricing.currency(), Currency::default());

        // And the price labels carry that currency rather than the placeholder.
        let items = pricing
            .calculate(&[
                ("peak", dec!(300)),
                ("offpeak", dec!(700)),
                ("night", dec!(100)),
            ])
            .unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0].unit_price.as_ref().unwrap().unit, "EUR/kWh");
        // 300 x 0.35 + 700 x 0.22 + 100 x 0.15 = 105 + 154 + 15 = 274.
        let total: Amount<5> = items.iter().map(|i| i.net_amount).sum();
        assert_eq!(total, Amount::parse("274.00000").unwrap());

        // An empty plural call is a no-op, not a reset.
        let still_there = TimeOfUsePricing::builder()
            .unit("kWh")
            .band(TimeBand::new("flat", Amount::parse("0.30000").unwrap()))
            .bands([])
            .build()
            .unwrap();
        assert_eq!(still_there.band_names().collect::<Vec<_>>(), ["flat"]);
    }

    #[test]
    fn dynamic_pricing_plural_intervals_and_currency_reach_the_built_value() {
        // The same pair as the time-of-use builder: `intervals` and `currency` are
        // the steps the rest of the suite never takes. A weighted-average price
        // labelled `XXX/kWh` is the visible symptom, and it appears on the invoice
        // rather than in any assertion.
        let dp = DynamicPricing::builder()
            .unit("kWh")
            .currency(Currency::EUR)
            .intervals([
                (dec!(100), Amount::parse("0.10000").unwrap()),
                (dec!(200), Amount::parse("0.20000").unwrap()),
            ])
            .interval(dec!(100), Amount::parse("0.30000").unwrap())
            .build()
            .unwrap();

        assert_eq!(dp.unit(), "kWh");
        assert_eq!(dp.currency(), Currency::EUR);
        assert_ne!(dp.currency(), Currency::default());

        // 100 x 0.10 + 200 x 0.20 + 100 x 0.30 = 10 + 40 + 30 = 80 over 400 kWh.
        let item = dp.calculate().unwrap();
        assert_eq!(item.quantity_value(), Some(dec!(400)));
        assert_eq!(item.net_amount, Amount::parse("80.00000").unwrap());
        assert_eq!(item.unit_price.as_ref().unwrap().unit, "EUR/kWh (wtd avg)");

        // An empty plural call adds nothing and removes nothing.
        let kept = DynamicPricing::builder()
            .unit("kWh")
            .interval(dec!(10), Amount::parse("0.50000").unwrap())
            .intervals([])
            .build()
            .unwrap();
        assert_eq!(
            kept.calculate().unwrap().net_amount,
            Amount::parse("5.00000").unwrap()
        );
    }
}
