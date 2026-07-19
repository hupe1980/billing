//! Tax and discount overlays: [`TaxLayer`], [`DiscountLayer`], and built-in implementations.
//!
//! # Invariant enforcement
//!
//! Every built-in layer keeps its fields **private** and validates in its
//! constructor.  Public fields would let a caller bypass validation with a struct
//! literal (`FixedRateTax { rate: dec!(-1), .. }`), and `#[derive(Deserialize)]`
//! would bypass it again from untrusted JSON.  Both holes are closed: constructors
//! return [`Result`], and deserialisation routes through the same validation via
//! `#[serde(try_from = ...)]`.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::currency::Currency;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::quantity::{Quantity, UnitPrice};
use crate::vat::{TaxBreakdownEntry, TaxCategory};

/// Multiply by 100 for a display percentage without risking `Decimal`'s
/// panicking `Mul`, and strip trailing zeros: 0.19 → `19`, 0.195 → `19.5`.
fn rate_as_percent(rate: Decimal) -> Result<Decimal, BillingError> {
    rate.checked_mul(Decimal::ONE_HUNDRED)
        .map(|d| d.normalize())
        .ok_or(BillingError::InvalidInput {
            reason: format!("rate {rate} cannot be expressed as a percentage"),
        })
}

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

    /// This layer's contribution to the EN 16931 VAT breakdown (BG-23), if it is
    /// a value-added tax at all.
    ///
    /// Returns `None` by default, which is correct for layers that are **not**
    /// VAT: a platform commission ([`PercentageCharge`]) is a commercial charge,
    /// and a per-unit excise ([`PerUnitLevy`]) is part of the VAT *base* rather
    /// than a VAT itself. Only layers that actually levy VAT should override it.
    ///
    /// `positions` is the same slice passed to [`TaxLayer::compute`], so the
    /// taxable base reported here is exactly the base the tax was computed on.
    ///
    /// # Errors
    /// Implementations may propagate arithmetic errors from summing the base.
    fn breakdown(&self, positions: &[LineItem]) -> Result<Option<TaxBreakdownEntry>, BillingError> {
        let _ = positions;
        Ok(None)
    }
}

// ── FixedRateTax ──────────────────────────────────────────────────────────────

/// Fixed-percentage tax applied to the net total of all (or tagged) positions.
///
/// Example: MwSt 19%, VAT 20%, GST 10%.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "FixedRateTaxRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedRateTax {
    name: String,
    rate: Decimal,
    require_tag: Option<String>,
    category: TaxCategory,
    exemption_reason: Option<String>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct FixedRateTaxRepr {
    name: String,
    rate: Decimal,
    #[serde(default)]
    require_tag: Option<String>,
    #[serde(default = "default_category")]
    category: TaxCategory,
    #[serde(default)]
    exemption_reason: Option<String>,
}

#[cfg(feature = "serde")]
fn default_category() -> TaxCategory {
    TaxCategory::Standard
}

#[cfg(feature = "serde")]
impl TryFrom<FixedRateTaxRepr> for FixedRateTax {
    type Error = BillingError;
    fn try_from(r: FixedRateTaxRepr) -> Result<Self, Self::Error> {
        let mut t = Self::new(r.name, r.rate)?;
        t.require_tag = r.require_tag;
        t.category = r.category;
        t.exemption_reason = r.exemption_reason;
        t.check_category()?;
        Ok(t)
    }
}

impl FixedRateTax {
    /// Create a `FixedRateTax` with no tag filter.
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if `rate < 0`. A negative rate is a
    /// discount; use a [`DiscountLayer`] instead.
    ///
    /// ```rust
    /// use billing::FixedRateTax;
    /// use rust_decimal::dec;
    /// assert!(FixedRateTax::new("MwSt", dec!(0.19)).is_ok());
    /// assert!(FixedRateTax::new("bad", dec!(-0.19)).is_err());
    /// ```
    pub fn new(name: impl Into<String>, rate: Decimal) -> Result<Self, BillingError> {
        if rate < Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: format!("FixedRateTax rate must be >= 0, got {rate}"),
            });
        }
        Ok(Self {
            name: name.into(),
            rate,
            require_tag: None,
            category: TaxCategory::Standard,
            exemption_reason: None,
        })
    }

    /// Set the EN 16931 VAT category (BT-118). Defaults to
    /// [`TaxCategory::Standard`].
    ///
    /// Categories that carry no tax require an exemption reason — set it with
    /// [`FixedRateTax::with_exemption_reason`], or [`TaxLayer::breakdown`] will
    /// report the omission.
    ///
    /// ```rust
    /// use billing::{FixedRateTax, TaxCategory};
    /// use rust_decimal::dec;
    ///
    /// // §13b UStG reverse charge: 0%, recipient accounts for the tax.
    /// let rc = FixedRateTax::new("Reverse charge", dec!(0)).unwrap()
    ///     .with_category(TaxCategory::ReverseCharge)
    ///     .with_exemption_reason("Steuerschuldnerschaft des Leistungsempfängers (§13b UStG)");
    /// assert_eq!(rc.category(), TaxCategory::ReverseCharge);
    /// ```
    #[must_use]
    pub fn with_category(mut self, category: TaxCategory) -> Self {
        self.category = category;
        self
    }

    /// Set the EN 16931 VAT exemption reason text (BT-120).
    #[must_use]
    pub fn with_exemption_reason(mut self, reason: impl Into<String>) -> Self {
        self.exemption_reason = Some(reason.into());
        self
    }

    /// The configured VAT category.
    #[must_use]
    pub fn category(&self) -> TaxCategory {
        self.category
    }

    /// The configured exemption reason, if any.
    #[must_use]
    pub fn exemption_reason(&self) -> Option<&str> {
        self.exemption_reason.as_deref()
    }

    /// The taxable base — shared by `compute` and `breakdown` so the reported
    /// BT-116 can never disagree with the amount the tax was actually charged on.
    fn taxable_base(&self, positions: &[LineItem]) -> Result<Amount<5>, BillingError> {
        Amount::checked_sum(
            positions
                .iter()
                .filter(|p| self.require_tag.as_deref().is_none_or(|t| p.has_tag(t)))
                .map(|p| p.net_amount),
        )
    }

    /// A zero-tax category with a non-zero rate is contradictory, and a taxed
    /// category is not an exemption.
    fn check_category(&self) -> Result<(), BillingError> {
        if !self.category.carries_tax() && !self.rate.is_zero() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "FixedRateTax category {} carries no tax, so the rate must be 0 (got {})",
                    self.category, self.rate
                ),
            });
        }
        if self.category.forbids_exemption_reason() && self.exemption_reason.is_some() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "FixedRateTax category {} must not carry an exemption reason (BT-120)",
                    self.category
                ),
            });
        }
        Ok(())
    }

    /// The tax rate as a fraction (e.g. `0.19` for 19%).
    #[must_use]
    pub fn rate(&self) -> Decimal {
        self.rate
    }

    /// The tag restricting this layer's base, if any.
    #[must_use]
    pub fn require_tag(&self) -> Option<&str> {
        self.require_tag.as_deref()
    }

    /// Restrict the tax base to positions carrying `tag`.
    ///
    /// Only positions tagged with `tag` contribute to this layer's tax base.
    /// Positions without the tag are completely excluded — they are effectively
    /// tax-exempt for this layer.
    ///
    /// # Mixed-rate documents (e.g. German prosumer billing)
    ///
    /// To apply different VAT rates to different line items in the same document,
    /// tag items by their VAT treatment and add one `FixedRateTax` per applicable rate:
    ///
    /// ```rust
    /// use billing::{FixedRateTax, LineItem, Amount, BillingDocument, DocumentMeta, Currency};
    /// use rust_decimal::dec;
    ///
    /// // Two positions: grid charges (19% VAT) + PV feed-in credit (0% / tax-exempt)
    /// let positions = vec![
    ///     LineItem::debit("Netzentgelt")
    ///         .fixed_amount(Amount::parse("100.00000").unwrap())
    ///         .tag("grid")
    ///         .build().unwrap(),
    ///     LineItem::credit_for_usage("EEG Einspeisevergütung", dec!(500), "kWh",
    ///                                dec!(0.0811), "EUR/kWh")
    ///         // No "grid" tag → excluded from the 19% VAT layer below.
    ///         .build().unwrap(),
    /// ];
    /// // Only grid-tagged items are subject to 19% VAT:
    /// let vat = FixedRateTax::new("MwSt 19%", dec!(0.19)).unwrap().with_tag("grid");
    /// let taxes: Vec<Box<dyn billing::TaxLayer>> = vec![Box::new(vat)];
    /// let doc = BillingDocument::from_positions(
    ///     DocumentMeta { currency: Currency::EUR, ..Default::default() },
    ///     positions, taxes, vec![],
    /// ).unwrap();
    /// // VAT = 100.00 × 0.19 = 19.00 (feed-in credit not in base)
    /// assert_eq!(doc.tax_total(), Amount::parse("19.00000").unwrap());
    /// ```
    #[must_use]
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
        // `with_category` and `with_exemption_reason` are infallible builders, so a
        // caller can reach a contradictory state (a zero-tax category carrying a
        // non-zero rate). `breakdown` checked this but `compute` did not, and
        // `TaxLayer` is public API — anyone driving layers directly got 19% tax
        // charged under a category that must never carry any.
        self.check_category()?;
        let base = self.taxable_base(positions)?;
        let tax = base.checked_mul_qty(self.rate)?;
        let rate_pct = rate_as_percent(self.rate)?;
        LineItem::debit(format!("{} ({}%)", self.name, rate_pct))
            .fixed_amount(tax)
            .tag("tax")
            .build()
    }

    fn breakdown(&self, positions: &[LineItem]) -> Result<Option<TaxBreakdownEntry>, BillingError> {
        self.check_category()?;
        // Same base as `compute`, so BT-116 and BT-117 are guaranteed consistent.
        let base = self.taxable_base(positions)?;
        let mut entry = TaxBreakdownEntry::new(
            self.category,
            self.rate,
            base,
            base.checked_mul_qty(self.rate)?,
        );
        entry.exemption_reason = self.exemption_reason.clone();
        entry.validate()?;
        Ok(Some(entry))
    }
}

// ── PerUnitLevy ───────────────────────────────────────────────────────────────

/// Per-unit levy (e.g. Stromsteuer 2.05 ct/kWh, CO₂ levy, excise duty).
///
/// Sums units from positions whose `unit_label` matches `unit`.
/// Optionally further restricted to positions with a specific tag.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "PerUnitLevyRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerUnitLevy {
    name: String,
    rate: Amount<5>,
    unit: String,
    currency: Currency,
    require_tag: Option<String>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct PerUnitLevyRepr {
    name: String,
    rate: Amount<5>,
    unit: String,
    #[serde(default)]
    currency: Currency,
    #[serde(default)]
    require_tag: Option<String>,
}

#[cfg(feature = "serde")]
impl TryFrom<PerUnitLevyRepr> for PerUnitLevy {
    type Error = BillingError;
    fn try_from(r: PerUnitLevyRepr) -> Result<Self, Self::Error> {
        let mut l = Self::new(r.name, r.rate, r.unit)?;
        l.currency = r.currency;
        l.require_tag = r.require_tag;
        Ok(l)
    }
}

impl PerUnitLevy {
    /// Create a `PerUnitLevy` with no tag filter and currency [`Currency::XXX`].
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if `rate` is negative (use a
    /// [`DiscountLayer`] for per-unit credits) or if `unit` is empty.
    pub fn new(
        name: impl Into<String>,
        rate: Amount<5>,
        unit: impl Into<String>,
    ) -> Result<Self, BillingError> {
        if rate.is_negative() {
            return Err(BillingError::InvalidInput {
                reason: format!("PerUnitLevy rate must be >= 0, got {rate}"),
            });
        }
        let unit = unit.into();
        if unit.trim().is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "PerUnitLevy unit must not be empty".into(),
            });
        }
        Ok(Self {
            name: name.into(),
            rate,
            unit,
            currency: Currency::XXX,
            require_tag: None,
        })
    }

    /// Set the currency used in the generated unit-price label (e.g. `"EUR/kWh"`).
    #[must_use]
    pub fn with_currency(mut self, currency: Currency) -> Self {
        self.currency = currency;
        self
    }

    /// Restrict this levy to positions carrying `tag`.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.require_tag = Some(tag.into());
        self
    }

    /// The rate per unit.
    pub fn rate(&self) -> Amount<5> {
        self.rate
    }

    /// The unit label this levy matches (e.g. `"kWh"`).
    #[must_use]
    pub fn unit(&self) -> &str {
        &self.unit
    }

    /// The currency used for the generated unit-price label.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.currency
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
        //
        // Exclude positions emitted by other tax layers.
        //
        // `BillingDocument::from_positions` feeds each layer the accumulated
        // positions, including earlier layers' output — which is required for
        // percentage taxes to compound. But a `PerUnitLevy` emits a DEBIT line
        // carrying a `Quantity` in its own unit, so a second per-unit levy on the
        // same unit would count that line as if it were consumption and double its
        // base. Stromsteuer + Konzessionsabgabe (both ct/kWh) is the standard
        // German electricity stack, where this over-billed the second levy by 100%.
        //
        // Custom `TaxLayer`s that emit a quantity should tag their output "tax"
        // to be excluded here.
        //
        // `checked_add`: `Decimal`'s `Sum`/`Add` panic on overflow.
        let mut total_units = Decimal::ZERO;
        for q in positions
            .iter()
            .filter(|p| p.is_debit())
            .filter(|p| !p.has_tag("tax"))
            .filter(|p| p.unit_label() == Some(&self.unit))
            .filter(|p| self.require_tag.as_deref().is_none_or(|t| p.has_tag(t)))
            .filter_map(|p| p.quantity_value())
        {
            total_units = total_units
                .checked_add(q)
                .ok_or(BillingError::MonetaryOverflow {
                    precision: 5,
                    input_value: None,
                })?;
        }
        let price_unit = format!("{}/{}", self.currency, self.unit);
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
///
/// # Note on totals
///
/// `PercentageCharge` implements [`TaxLayer`], so its output lands in
/// `tax_positions` and is counted in [`crate::BillingDocument::tax_total`] —
/// a commercial commission will therefore appear inside the document's tax total.
/// Filter by the `"percentage-charge"` tag to separate them for display.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "PercentageChargeRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PercentageCharge {
    name: String,
    rate: Decimal,
    apply_to_tag: Option<String>,
    min_amount: Option<Amount<5>>,
    max_amount: Option<Amount<5>>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct PercentageChargeRepr {
    name: String,
    rate: Decimal,
    #[serde(default)]
    apply_to_tag: Option<String>,
    #[serde(default)]
    min_amount: Option<Amount<5>>,
    #[serde(default)]
    max_amount: Option<Amount<5>>,
}

#[cfg(feature = "serde")]
impl TryFrom<PercentageChargeRepr> for PercentageCharge {
    type Error = BillingError;
    fn try_from(r: PercentageChargeRepr) -> Result<Self, Self::Error> {
        let mut c = Self::new(r.name, r.rate)?;
        c.apply_to_tag = r.apply_to_tag;
        if let Some(min) = r.min_amount {
            c = c.with_min(min);
        }
        if let Some(max) = r.max_amount {
            c = c.with_max(max);
        }
        c.check_bounds()?;
        Ok(c)
    }
}

impl PercentageCharge {
    /// Create a `PercentageCharge` with no tag filter and no min/max guard.
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if `rate < 0`. A negative charge is a
    /// discount; use a [`DiscountLayer`] instead.
    pub fn new(name: impl Into<String>, rate: Decimal) -> Result<Self, BillingError> {
        if rate < Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: format!("PercentageCharge rate must be >= 0, got {rate}"),
            });
        }
        Ok(Self {
            name: name.into(),
            rate,
            apply_to_tag: None,
            min_amount: None,
            max_amount: None,
        })
    }

    /// Restrict this charge to positions carrying `tag`.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.apply_to_tag = Some(tag.into());
        self
    }

    /// Set a minimum charge floor.
    #[must_use]
    pub fn with_min(mut self, min: Amount<5>) -> Self {
        self.min_amount = Some(min);
        self
    }

    /// Set a maximum charge ceiling.
    #[must_use]
    pub fn with_max(mut self, max: Amount<5>) -> Self {
        self.max_amount = Some(max);
        self
    }

    /// The charge rate as a fraction.
    #[must_use]
    pub fn rate(&self) -> Decimal {
        self.rate
    }

    /// Guard: if both min and max are set, min must not exceed max.
    /// Violating this would make the clamping logic produce a result below the
    /// declared minimum, silently breaking the contract.
    fn check_bounds(&self) -> Result<(), BillingError> {
        if let (Some(min), Some(max)) = (self.min_amount, self.max_amount) {
            if min > max {
                return Err(BillingError::InvalidInput {
                    reason: "PercentageCharge: min_amount must not exceed max_amount".into(),
                });
            }
        }
        Ok(())
    }
}

impl TaxLayer for PercentageCharge {
    fn name(&self) -> &str {
        &self.name
    }

    fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError> {
        self.check_bounds()?;
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
        let rate_pct = rate_as_percent(self.rate)?;
        LineItem::debit(format!("{} ({}%)", self.name, rate_pct))
            .fixed_amount(charge)
            .tag("percentage-charge")
            .build()
    }
}

// ── DiscountLayer trait ───────────────────────────────────────────────────────

/// Composable discount overlay — always produces a credit (negative) position.
///
/// # Discounts do not compound
///
/// Unlike [`TaxLayer`]s, every `DiscountLayer` in a document receives the
/// **original net positions only** — never the output of a preceding discount.
/// Two stacked 10% discounts therefore take 10% + 10% of the same base (20%
/// total), not 10% of 90% (19%).
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
#[cfg_attr(feature = "serde", serde(try_from = "PercentageDiscountRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PercentageDiscount {
    name: String,
    rate: Decimal,
    apply_to_tag: Option<String>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct PercentageDiscountRepr {
    name: String,
    rate: Decimal,
    #[serde(default)]
    apply_to_tag: Option<String>,
}

#[cfg(feature = "serde")]
impl TryFrom<PercentageDiscountRepr> for PercentageDiscount {
    type Error = BillingError;
    fn try_from(r: PercentageDiscountRepr) -> Result<Self, Self::Error> {
        let mut d = Self::new(r.name, r.rate)?;
        d.apply_to_tag = r.apply_to_tag;
        Ok(d)
    }
}

impl PercentageDiscount {
    /// Create a `PercentageDiscount` with no tag filter.
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if `rate` is outside `[0, 1]`. A rate
    /// above 100% would produce a net charge rather than a credit; use
    /// [`FixedDiscount`] for fixed amounts.
    pub fn new(name: impl Into<String>, rate: Decimal) -> Result<Self, BillingError> {
        if rate < Decimal::ZERO || rate > Decimal::ONE {
            return Err(BillingError::InvalidInput {
                reason: format!("PercentageDiscount rate must be in [0, 1], got {rate}"),
            });
        }
        Ok(Self {
            name: name.into(),
            rate,
            apply_to_tag: None,
        })
    }

    /// Restrict this discount to positions carrying `tag`.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.apply_to_tag = Some(tag.into());
        self
    }

    /// The discount rate as a fraction.
    #[must_use]
    pub fn rate(&self) -> Decimal {
        self.rate
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
        // A negative base (debit positions that net out negative, e.g. sustained
        // negative spot prices) would otherwise turn a "discount" into an extra
        // credit, inflating the amount owed to the customer. Clamp to zero: there
        // is nothing to discount.
        let base = if base.is_negative() {
            Amount::<5>::ZERO
        } else {
            base
        };
        let discount = base.checked_mul_qty(self.rate)?;
        let rate_pct = rate_as_percent(self.rate)?;
        LineItem::credit(format!("{} (-{}%)", self.name, rate_pct))
            .fixed_amount(discount)
            .tag("discount")
            .build()
    }
}

// ── FixedDiscount ─────────────────────────────────────────────────────────────

/// Fixed-amount discount (always a credit).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "FixedDiscountRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FixedDiscount {
    name: String,
    amount: Amount<5>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct FixedDiscountRepr {
    name: String,
    amount: Amount<5>,
}

#[cfg(feature = "serde")]
impl TryFrom<FixedDiscountRepr> for FixedDiscount {
    type Error = BillingError;
    fn try_from(r: FixedDiscountRepr) -> Result<Self, Self::Error> {
        Self::new(r.name, r.amount)
    }
}

impl FixedDiscount {
    /// Create a `FixedDiscount`.
    ///
    /// `amount` is the **magnitude** of the discount and must be non-negative;
    /// it is applied as a credit (negative `net_amount`).
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if `amount` is negative — a negative
    /// discount is a surcharge, which belongs in a [`TaxLayer`].
    pub fn new(name: impl Into<String>, amount: Amount<5>) -> Result<Self, BillingError> {
        if amount.is_negative() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "FixedDiscount amount must be >= 0 (it is applied as a credit), got {amount}"
                ),
            });
        }
        Ok(Self {
            name: name.into(),
            amount,
        })
    }

    /// The discount magnitude (non-negative).
    pub fn amount(&self) -> Amount<5> {
        self.amount
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
    use rust_decimal::dec;

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
        let tax = FixedRateTax::new("MwSt", dec!(0.19)).unwrap();
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
        let tax = FixedRateTax::new("MwSt", dec!(0.19))
            .unwrap()
            .with_tag("commodity");
        let item = tax.compute(&positions).unwrap();
        // Only 100 × 0.19 = 19, not (100+20) × 0.19
        assert_eq!(item.net_amount, Amount::parse("19.00000").unwrap());
    }

    #[test]
    fn negative_rates_are_errors_not_panics() {
        assert!(FixedRateTax::new("bad", dec!(-0.19)).is_err());
        assert!(PercentageCharge::new("bad", dec!(-0.01)).is_err());
        assert!(PercentageDiscount::new("bad", dec!(-0.01)).is_err());
        assert!(PercentageDiscount::new("bad", dec!(1.5)).is_err());
        assert!(PerUnitLevy::new("bad", Amount::parse("-0.1").unwrap(), "kWh").is_err());
        assert!(FixedDiscount::new("bad", Amount::parse("-5.0").unwrap()).is_err());
    }

    #[test]
    fn percentage_charge_with_min() {
        let positions = vec![make_item("5.00000", None)];
        let charge = PercentageCharge::new("Platform fee", dec!(0.05))
            .unwrap()
            .with_min(Amount::parse("0.50000").unwrap());
        let item = charge.compute(&positions).unwrap();
        // 5 × 0.05 = 0.25, but min = 0.50
        assert_eq!(item.net_amount, Amount::parse("0.50000").unwrap());
    }

    #[test]
    fn percentage_charge_with_max() {
        let positions = vec![make_item("10000.00000", None)];
        let charge = PercentageCharge::new("Fee", dec!(0.05))
            .unwrap()
            .with_max(Amount::parse("100.00000").unwrap());
        let item = charge.compute(&positions).unwrap();
        assert_eq!(item.net_amount, Amount::parse("100.00000").unwrap());
    }

    #[test]
    fn percentage_discount() {
        let positions = vec![make_item("200.00000", None)];
        let disc = PercentageDiscount::new("Loyalty", dec!(0.10)).unwrap();
        let item = disc.compute(&positions).unwrap();
        assert!(item.net_amount.is_negative());
        assert_eq!(item.net_amount, Amount::parse("-20.00000").unwrap());
    }

    #[test]
    fn percentage_discount_on_negative_base_is_zero_not_extra_credit() {
        // A debit position with a negative net (negative spot price) must not
        // turn a discount into an additional credit.
        let positions = vec![make_item("-200.00000", None)];
        let disc = PercentageDiscount::new("Loyalty", dec!(0.10)).unwrap();
        let item = disc.compute(&positions).unwrap();
        assert_eq!(item.net_amount, Amount::<5>::ZERO);
    }

    #[test]
    fn fixed_discount() {
        let disc = FixedDiscount::new("Voucher", Amount::parse("15.00000").unwrap()).unwrap();
        let item = disc.compute(&[]).unwrap();
        assert_eq!(item.net_amount, Amount::parse("-15.00000").unwrap());
    }

    #[test]
    fn per_unit_levy_label_uses_configured_currency() {
        let levy = PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh")
            .unwrap()
            .with_currency(Currency::EUR);
        let positions = vec![
            LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh")
                .build()
                .unwrap(),
        ];
        let item = levy.compute(&positions).unwrap();
        assert_eq!(item.unit_price.as_ref().unwrap().unit, "EUR/kWh");
        assert_eq!(item.net_amount, Amount::parse("20.50000").unwrap());
    }

    #[test]
    fn per_unit_levy_defaults_to_unset_currency_not_eur() {
        let levy = PerUnitLevy::new("Levy", Amount::parse("0.01000").unwrap(), "kWh").unwrap();
        assert_eq!(levy.currency(), Currency::XXX);
    }
}
