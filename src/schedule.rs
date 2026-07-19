//! [`TariffSchedule`] — graduated / volume / block / capacity pricing.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::currency::Currency;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::quantity::{Quantity, UnitPrice};

// ── TariffBand ────────────────────────────────────────────────────────────────

/// A single pricing tier within a [`TariffSchedule`].
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct TariffBand {
    /// Override description used in the generated `LineItem`.
    /// If `None`, an auto-generated range string is used.
    pub description: Option<String>,
    /// Lower bound (exclusive). `None` means the band starts from 0.
    /// Stored for contiguity validation; the schedule iterator derives the
    /// effective lower bound from the previous band's `upper`.
    pub lower: Option<Decimal>,
    /// Upper bound (inclusive). `None` means unlimited.
    pub upper: Option<Decimal>,
    /// Price per unit (or per block in block mode).
    pub price: Amount<5>,
    /// Block size for `block` mode only.
    pub block_size: Option<Decimal>,
}

impl TariffBand {
    /// Band from 0 up to and including `upper`.
    #[must_use]
    pub fn up_to(upper: Decimal, price: Amount<5>) -> Self {
        Self {
            description: None,
            lower: None,
            upper: Some(upper),
            price,
            block_size: None,
        }
    }

    /// Band from `lower` (exclusive) upward, unlimited.
    #[must_use]
    pub fn over(lower: Decimal, price: Amount<5>) -> Self {
        Self {
            description: None,
            lower: Some(lower),
            upper: None,
            price,
            block_size: None,
        }
    }

    /// Band from `lower` (exclusive) up to `upper` (inclusive).
    #[must_use]
    pub fn between(lower: Decimal, upper: Decimal, price: Amount<5>) -> Self {
        Self {
            description: None,
            lower: Some(lower),
            upper: Some(upper),
            price,
            block_size: None,
        }
    }

    /// Block pricing: one block = `block_size` units, charged at `price` per block.
    #[must_use]
    pub fn block(block_size: Decimal, price: Amount<5>) -> Self {
        Self {
            description: None,
            lower: None,
            upper: None,
            price,
            block_size: Some(block_size),
        }
    }

    /// Free tier: first `units` at zero price. Alias for `up_to(units, Amount::ZERO)`.
    ///
    /// The auto-generated description is `"Free tier (first {N})"`.  Use
    /// `.with_description("Free tier (first 1000 kWh)")` to override it with a
    /// domain-specific unit label.
    #[must_use]
    pub fn free_up_to(units: Decimal) -> Self {
        Self {
            description: Some(format!("Free tier (first {units})")),
            lower: None,
            upper: Some(units),
            price: Amount::ZERO,
            block_size: None,
        }
    }

    /// Override the auto-generated description.
    #[must_use]
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

// ── Schedule mode ─────────────────────────────────────────────────────────────

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
// The module docs advertise loading schedules from JSON/YAML with lowercase mode
// names; without this the derived names are TitleCase and `"graduated"` fails.
#[cfg_attr(feature = "serde", serde(rename_all = "lowercase"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Graduated,
    Volume,
    Block,
    Capacity,
}

// ── TariffSchedule ────────────────────────────────────────────────────────────

/// Tariff schedule with four billing modes.
///
/// | Mode       | Billing basis                        |
/// |------------|--------------------------------------|
/// | `graduated`| each tier at its own price           |
/// | `volume`   | all units at the top tier reached    |
/// | `block`    | rounded-up blocks × price            |
/// | `capacity` | peak value selects tier; flat charge |
///
/// # Domain-agnostic unit labels
///
/// Call `.unit("kWh")` (or `"seats"`, `"GB"`, `"m³"`, …) on the builder to
/// propagate the correct unit into all generated `LineItem` quantity / price labels.
/// Defaults to `"units"` when not set.
///
/// # Validation on deserialisation
///
/// `TariffSchedule` deserialises through [`TariffScheduleBuilder::build`], so a
/// schedule loaded from JSON/YAML is subject to exactly the same band-contiguity,
/// price and block-size checks as one built in code. An invalid schedule is a
/// deserialisation error, never a silently-mispricing value.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "TariffScheduleRepr"))]
#[derive(Debug, Clone)]
pub struct TariffSchedule {
    mode: Mode,
    bands: Vec<TariffBand>,
    unit: String,
    currency: Currency,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct TariffScheduleRepr {
    mode: Mode,
    bands: Vec<TariffBand>,
    unit: String,
    #[serde(default)]
    currency: Currency,
}

#[cfg(feature = "serde")]
impl TryFrom<TariffScheduleRepr> for TariffSchedule {
    type Error = BillingError;
    fn try_from(r: TariffScheduleRepr) -> Result<Self, Self::Error> {
        TariffScheduleBuilder {
            mode: r.mode,
            bands: r.bands,
            unit: r.unit,
            currency: r.currency,
        }
        .build()
    }
}

/// Builder for [`TariffSchedule`]. Obtained via `TariffSchedule::graduated()` etc.
pub struct TariffScheduleBuilder {
    mode: Mode,
    bands: Vec<TariffBand>,
    unit: String,
    currency: Currency,
}

impl TariffSchedule {
    /// Graduated (Staffeln) — each tier at its own price.
    #[must_use]
    pub fn graduated() -> TariffScheduleBuilder {
        TariffScheduleBuilder {
            mode: Mode::Graduated,
            bands: vec![],
            unit: "units".into(),
            currency: Currency::XXX,
        }
    }

    /// Volume — all units priced at the top tier reached.
    #[must_use]
    pub fn volume() -> TariffScheduleBuilder {
        TariffScheduleBuilder {
            mode: Mode::Volume,
            bands: vec![],
            unit: "units".into(),
            currency: Currency::XXX,
        }
    }

    /// Block — charge per N-unit block (rounded up).
    #[must_use]
    pub fn block() -> TariffScheduleBuilder {
        TariffScheduleBuilder {
            mode: Mode::Block,
            bands: vec![],
            unit: "units".into(),
            currency: Currency::XXX,
        }
    }

    /// Capacity — bill on the MAXIMUM (peak) value in the period.
    #[must_use]
    pub fn capacity() -> TariffScheduleBuilder {
        TariffScheduleBuilder {
            mode: Mode::Capacity,
            bands: vec![],
            unit: "units".into(),
            currency: Currency::XXX,
        }
    }

    /// The quantity unit label configured for this schedule (e.g. `"kWh"`).
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The currency used in generated unit-price labels.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.currency
    }

    /// Split a cumulative quantity across this schedule.
    ///
    /// - `graduated`: returns N `LineItem`s, one per band.
    /// - `volume` and `block`: returns a single `LineItem`.
    /// - `capacity`: use [`TariffSchedule::apply_peak`] instead.
    ///
    /// # Errors
    /// - `quantity < 0`.
    /// - the schedule is in `capacity` mode (use [`TariffSchedule::apply_peak`]).
    /// - `quantity` exceeds the schedule's coverage — the top band is bounded and
    ///   the value is above it.
    pub fn split(&self, quantity: Decimal) -> Result<Vec<LineItem>, BillingError> {
        if quantity < Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: "quantity must be non-negative".into(),
            });
        }
        match self.mode {
            Mode::Graduated => self.split_graduated(quantity),
            Mode::Volume => self.split_volume(quantity),
            Mode::Block => self.split_block(quantity),
            Mode::Capacity => Err(BillingError::InvalidInput {
                reason: "Capacity schedule requires apply_peak(), not split()".into(),
            }),
        }
    }

    /// Apply peak-based pricing. Returns a single flat-fee `LineItem`.
    ///
    /// The `peak` value selects the tier; only valid on `capacity` schedules.
    ///
    /// # Errors
    /// Returns `Err` if `peak < 0` or if this is not a capacity schedule.
    pub fn apply_peak(&self, peak: Decimal) -> Result<LineItem, BillingError> {
        if self.mode != Mode::Capacity {
            return Err(BillingError::InvalidInput {
                reason: "apply_peak() is only valid for capacity schedules".into(),
            });
        }
        if peak < Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: "peak must be non-negative".into(),
            });
        }
        let price = self
            .find_tier_price(peak)
            .ok_or_else(|| BillingError::InvalidSchedule {
                reason: format!(
                    "no tier covers peak {peak}: add an open-ended top band \
                     (TariffBand::over) to cover values above the highest bound"
                ),
            })?;
        LineItem::debit(format!("Capacity charge (peak {peak:.3} {})", self.unit))
            .fixed_amount(price)
            .build()
    }

    // ── private helpers ───────────────────────────────────────────────────────

    fn split_graduated(&self, mut remaining: Decimal) -> Result<Vec<LineItem>, BillingError> {
        let mut items = Vec::with_capacity(self.bands.len());
        let mut prev_upper = Decimal::ZERO;
        let price_unit = format!("{}/{}", self.currency, self.unit);

        for (tier, band) in self.bands.iter().enumerate() {
            let tier_num = tier + 1;
            if remaining <= Decimal::ZERO {
                break;
            }
            let cap = band.upper.map(|u| u - prev_upper).unwrap_or(remaining);
            let qty = remaining.min(cap);
            if qty > Decimal::ZERO {
                let desc = band
                    .description
                    .clone()
                    .unwrap_or_else(|| match band.upper {
                        Some(u) => format!("Tier {tier_num} (up to {u} {})", self.unit),
                        None => format!("Tier {tier_num} (over {prev_upper} {})", self.unit),
                    });
                items.push(
                    LineItem::debit(desc)
                        .quantity(Quantity::new(qty, &self.unit))
                        .unit_price(UnitPrice::new(band.price.into_decimal(), &price_unit))
                        .build()?,
                );
                remaining -= qty;
            }
            if let Some(u) = band.upper {
                prev_upper = u;
            }
        }
        // If any units remain after exhausting all bands, the schedule does not cover
        // the full quantity. Return an explicit error rather than silently under-billing.
        if remaining > Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: "quantity exceeds the schedule's coverage: add an open-ended top band"
                    .into(),
            });
        }
        Ok(items)
    }

    fn split_volume(&self, quantity: Decimal) -> Result<Vec<LineItem>, BillingError> {
        if quantity.is_zero() {
            return Ok(vec![]);
        }
        let price =
            self.find_tier_price(quantity)
                .ok_or_else(|| BillingError::InvalidSchedule {
                    reason: format!(
                        "no tier covers quantity {quantity}: add an open-ended top band \
                     (TariffBand::over) to cover values above the highest bound"
                    ),
                })?;
        let price_unit = format!("{}/{}", self.currency, self.unit);
        Ok(vec![
            LineItem::debit(format!("Usage charge ({} volume)", self.unit))
                .quantity(Quantity::new(quantity, &self.unit))
                .unit_price(UnitPrice::new(price.into_decimal(), &price_unit))
                .build()?,
        ])
    }

    fn split_block(&self, quantity: Decimal) -> Result<Vec<LineItem>, BillingError> {
        if quantity.is_zero() {
            return Ok(vec![]);
        }
        let band = self.bands.first().ok_or(BillingError::InvalidSchedule {
            reason: "block schedule requires at least one band".into(),
        })?;
        let block_size = band.block_size.ok_or(BillingError::InvalidSchedule {
            reason: "block schedule band must have a block_size".into(),
        })?;
        // Rounds UP — partial block is billed as a full block.
        // `checked_div`: `Decimal`'s `/` panics on overflow, and `block_size` is only
        // validated as `> 0` — a very small block size (e.g. 1e-28) against a large
        // quantity overflows, which would abort inside a `Result`-returning method.
        let blocks = quantity
            .checked_div(block_size)
            .ok_or_else(|| BillingError::InvalidSchedule {
                reason: format!(
                    "block schedule: quantity {quantity} / block_size {block_size} overflows"
                ),
            })?
            .ceil();
        let block_label = format!("{}-{}-block", block_size, self.unit);
        let price_unit = format!("{}/{block_label}", self.currency);
        Ok(vec![
            LineItem::debit(format!(
                "Usage charge (block, {block_size} {}/block)",
                self.unit
            ))
            .quantity(Quantity::new(blocks, &block_label))
            .unit_price(UnitPrice::new(band.price.into_decimal(), &price_unit))
            .build()?,
        ])
    }

    /// Find the price for the tier that covers `quantity`.
    ///
    /// Returns the first band whose `upper >= quantity`, or an open-ended band's
    /// price. Returns `None` when `quantity` exceeds every finite bound and no
    /// open-ended band exists — i.e. the schedule does not cover the value.
    ///
    /// The trailing `self.bands.last()` fallback that used to sit here made this
    /// function total, which silently priced anything above a bounded top band at
    /// the top-band rate and rendered the "no tier covers …" errors in
    /// [`TariffSchedule::split`] and [`TariffSchedule::apply_peak`] unreachable.
    /// `graduated` mode already errored on the same input; the three modes now
    /// agree.
    fn find_tier_price(&self, quantity: Decimal) -> Option<Amount<5>> {
        for band in &self.bands {
            match band.upper {
                Some(upper) if quantity <= upper => return Some(band.price),
                Some(_) => {}
                None => return Some(band.price),
            }
        }
        None
    }
}

impl TariffScheduleBuilder {
    #[must_use]
    /// Append a pricing band to the schedule.
    pub fn band(mut self, band: TariffBand) -> Self {
        self.bands.push(band);
        self
    }

    /// Set the quantity unit label (e.g. `"kWh"`, `"seats"`, `"m³"`, `"GB"`).
    ///
    /// Propagated to all auto-generated `LineItem` quantity/price unit strings.
    /// Defaults to `"units"` if not called.
    #[must_use]
    pub fn unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = unit.into();
        self
    }

    /// Set the currency used in generated unit-price labels (e.g. `"EUR/kWh"`).
    ///
    /// Defaults to [`Currency::XXX`] — a schedule whose labels read `XXX/kWh`
    /// was never given a currency.
    #[must_use]
    pub fn currency(mut self, currency: Currency) -> Self {
        self.currency = currency;
        self
    }

    /// Validate and build the [`TariffSchedule`].
    ///
    /// # Errors
    /// - Zero bands.
    /// - Any band has a negative bound.
    /// - Block mode: `block_size` is zero or negative, or more than one band is provided.
    /// - Any band price is negative (credits belong in `DiscountLayer`, not in bands).
    /// - Bands are not contiguous (gap or overlap between adjacent `lower`/`upper` values).
    /// - A non-final band has an unlimited upper bound (only the last band may be open-ended).
    pub fn build(self) -> Result<TariffSchedule, BillingError> {
        if self.bands.is_empty() {
            return Err(BillingError::InvalidSchedule {
                reason: "schedule must have at least one band".into(),
            });
        }

        // Block schedules use exactly one band — multiple bands are silently
        // unserviceable (split_block only reads bands.first()).
        if self.mode == Mode::Block && self.bands.len() != 1 {
            return Err(BillingError::InvalidSchedule {
                reason: "block schedule must have exactly one band".into(),
            });
        }

        // Validate individual band bounds, block sizes, and prices.
        for band in &self.bands {
            if band.price.is_negative() {
                return Err(BillingError::InvalidSchedule {
                    reason: "band price must be non-negative; use DiscountLayer for credits".into(),
                });
            }
            if band.lower.is_some_and(|l| l < Decimal::ZERO) {
                return Err(BillingError::InvalidSchedule {
                    reason: "band lower bound must be non-negative".into(),
                });
            }
            if band.upper.is_some_and(|u| u <= Decimal::ZERO) {
                return Err(BillingError::InvalidSchedule {
                    reason: "band upper bound must be positive".into(),
                });
            }
            // When both bounds are specified, lower must be strictly less than upper.
            if let (Some(lower), Some(upper)) = (band.lower, band.upper) {
                if lower >= upper {
                    return Err(BillingError::InvalidSchedule {
                        reason: "band lower bound must be strictly less than upper bound".into(),
                    });
                }
            }
            if let Some(bs) = band.block_size {
                if bs <= Decimal::ZERO {
                    return Err(BillingError::InvalidSchedule {
                        reason: "block_size must be positive (> 0)".into(),
                    });
                }
            }
            // Block bands must have a block_size; non-block bands must not.
            if self.mode == Mode::Block && band.block_size.is_none() {
                return Err(BillingError::InvalidSchedule {
                    reason: "block schedule band must specify block_size".into(),
                });
            }
            if self.mode != Mode::Block && band.block_size.is_some() {
                return Err(BillingError::InvalidSchedule {
                    reason: "block_size is only valid in block mode schedules".into(),
                });
            }
        }

        // Validate contiguity for non-block schedules.
        //
        // This runs for single-band schedules too. Gating it on `len() > 1` left a
        // hole exactly where a config typo is least likely to be noticed: a lone
        // `between(500, 1000)` band built successfully and then billed from zero,
        // because `split_graduated` derives the lower bound from the previous
        // band's upper and starts at 0. The identical shape with a second band was
        // correctly rejected.
        if self.mode != Mode::Block {
            let mut expected_lower = Decimal::ZERO;
            for (i, band) in self.bands.iter().enumerate() {
                // Check declared lower matches expected position.
                if let Some(lower) = band.lower {
                    if lower != expected_lower {
                        return Err(BillingError::InvalidSchedule {
                            reason: "bands must be contiguous: gap or overlap detected".into(),
                        });
                    }
                }
                // Upper bounds must be strictly increasing.
                //
                // The declared-`lower` check above cannot catch this on its own:
                // `TariffBand::up_to` and `free_up_to` leave `lower` as `None`, so a
                // schedule like `[up_to(100), up_to(50)]` used to build successfully
                // and then either mis-price or fail late inside `split()` with a
                // misleading "quantity exceeds coverage" error.
                if let Some(u) = band.upper {
                    if u <= expected_lower {
                        return Err(BillingError::InvalidSchedule {
                            reason: format!(
                                "band upper bounds must be strictly increasing: \
                                 band {} has upper {u} but the previous band ends at {expected_lower}",
                                i + 1
                            ),
                        });
                    }
                }
                // Non-final band must have a finite upper bound.
                if i < self.bands.len() - 1 {
                    match band.upper {
                        Some(u) => expected_lower = u,
                        None => {
                            return Err(BillingError::InvalidSchedule {
                                reason: "only the last band may have an unlimited upper bound"
                                    .into(),
                            });
                        }
                    }
                }
            }
        }
        Ok(TariffSchedule {
            mode: self.mode,
            bands: self.bands,
            unit: crate::validate_unit(self.unit)?,
            currency: self.currency,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn graduated_two_tiers_with_unit() {
        let sched = TariffSchedule::graduated()
            .unit("kWh")
            .band(TariffBand::up_to(
                dec!(500),
                Amount::parse("0.32000").unwrap(),
            ))
            .band(TariffBand::over(
                dec!(500),
                Amount::parse("0.28000").unwrap(),
            ))
            .build()
            .unwrap();
        let items = sched.split(dec!(1234.5)).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].unit_label(), Some("kWh"));
        // 500 × 0.32 = 160
        assert_eq!(items[0].net_amount, Amount::parse("160.00000").unwrap());
        // 734.5 × 0.28 = 205.66
        assert_eq!(items[1].net_amount, Amount::parse("205.66000").unwrap());
    }

    #[test]
    fn volume_top_tier() {
        let sched = TariffSchedule::volume()
            .unit("seats")
            .band(TariffBand::up_to(
                dec!(1000),
                Amount::parse("0.32000").unwrap(),
            ))
            .band(TariffBand::over(
                dec!(1000),
                Amount::parse("0.28000").unwrap(),
            ))
            .build()
            .unwrap();
        let items = sched.split(dec!(1234.5)).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].unit_label(), Some("seats"));
        assert_eq!(items[0].net_amount, Amount::parse("345.66000").unwrap());
    }

    #[test]
    fn block_rounds_up() {
        let sched = TariffSchedule::block()
            .unit("GB")
            .band(TariffBand::block(
                dec!(100),
                Amount::parse("1.00000").unwrap(),
            ))
            .build()
            .unwrap();
        let items = sched.split(dec!(450)).unwrap();
        // 450 / 100 = 4.5 → ceil = 5 blocks × 1.00 = 5.00
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].net_amount, Amount::parse("5.00000").unwrap());
    }

    #[test]
    fn capacity_peak() {
        let sched = TariffSchedule::capacity()
            .unit("Mbps")
            .band(TariffBand::up_to(
                dec!(50),
                Amount::parse("5.00000").unwrap(),
            ))
            .band(TariffBand::over(
                dec!(50),
                Amount::parse("10.00000").unwrap(),
            ))
            .build()
            .unwrap();
        let item = sched.apply_peak(dec!(63.4)).unwrap();
        assert_eq!(item.net_amount, Amount::parse("10.00000").unwrap());
    }

    #[test]
    fn non_contiguous_bands_rejected() {
        let result = TariffSchedule::graduated()
            .band(TariffBand::up_to(
                dec!(100),
                Amount::parse("1.00000").unwrap(),
            ))
            // Gap: lower=200 but previous upper=100
            .band(TariffBand::between(
                dec!(200),
                dec!(300),
                Amount::parse("0.80000").unwrap(),
            ))
            .band(TariffBand::over(
                dec!(300),
                Amount::parse("0.60000").unwrap(),
            ))
            .build();
        assert!(
            result.is_err(),
            "gap between 100 and 200 should be rejected"
        );
    }

    #[test]
    fn default_unit_is_units() {
        let sched = TariffSchedule::graduated()
            .band(TariffBand::up_to(
                dec!(100),
                Amount::parse("1.00000").unwrap(),
            ))
            .build()
            .unwrap();
        assert_eq!(sched.unit(), "units");
        let items = sched.split(dec!(50)).unwrap();
        assert_eq!(items[0].unit_label(), Some("units"));
    }

    #[test]
    fn zero_block_size_rejected() {
        let result = TariffSchedule::block()
            .band(TariffBand::block(
                dec!(0),
                Amount::parse("1.00000").unwrap(),
            ))
            .build();
        assert!(result.is_err(), "block_size=0 must be rejected");
    }

    #[test]
    fn negative_block_size_rejected() {
        let result = TariffSchedule::block()
            .band(TariffBand::block(
                dec!(-1),
                Amount::parse("1.00000").unwrap(),
            ))
            .build();
        assert!(result.is_err(), "negative block_size must be rejected");
    }
}
