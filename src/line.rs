//! Lines and adjustments: [`Line`] (BG-25), [`LineAdjustment`] (BG-27/28),
//! [`DocumentAdjustment`] (BG-20/21), and the eager helpers [`apportion`] and
//! [`per_unit_levy`] that return ordinary lines and adjustments.

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use rust_decimal::Decimal;

use crate::vat::reduce_exact;
use crate::{
    Amount, AmountScale, Error, ErrorKind, Location, Percent, Price, Quantity, Result, UnitPrice,
    Usage, Vat, split, split_matrix,
};

/// Peppol's absolute tolerance in R040 and R120 (`u:slack(…, 0.02)`).
pub(crate) const SLACK: Decimal = Decimal::from_parts(2, 0, 0, false, 2);

/// Negate, keeping a zero a (positive) zero.
pub(crate) fn neg(d: Decimal) -> Decimal {
    if d.is_zero() { Decimal::ZERO } else { -d }
}

/// Whether an allowance subtracts or a charge adds (UBL `ChargeIndicator`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum AdjustmentKind {
    /// Subtracts its amount (BG-20 / BG-27).
    Allowance,
    /// Adds its amount (BG-21 / BG-28).
    Charge,
}

impl AdjustmentKind {
    /// `amount` with the kind's direction: a charge adds, an allowance subtracts.
    pub(crate) fn signed(self, amount: Amount) -> Result<Amount> {
        match self {
            Self::Charge => Ok(amount),
            Self::Allowance => amount.checked_neg(),
        }
    }
}

/// Validate an optional `(base, percent)` basis against the reduced amount:
/// the base must fit the scale (BR-DEC-02, BR-DEC-06, BR-DEC-25, BR-DEC-28)
/// and `amount` must be within ±0.02 of `base × percent / 100`
/// (PEPPOL-EN16931-R040; R041/R042 hold by the pair's construction).
fn check_basis(
    basis: Option<(Decimal, Decimal)>,
    amount: Amount,
    scale: AmountScale,
) -> Result<()> {
    let Some((base, pct)) = basis else {
        return Ok(());
    };
    let b = Amount::<5>::from_decimal(base)?;
    if !b.fits(scale.decimals) {
        return Err(ErrorKind::ExcessPrecision {
            value: base,
            decimals: scale.decimals,
        }
        .into());
    }
    let expected = base
        .checked_mul(pct)
        .and_then(|p| p.checked_div(Decimal::ONE_HUNDRED))
        .ok_or(ErrorKind::Overflow)?;
    let diff = amount
        .to_decimal()
        .checked_sub(expected)
        .map_or(Decimal::MAX, |d| d.abs());
    if diff > SLACK {
        return Err(ErrorKind::AdjustmentBasis.into());
    }
    Ok(())
}

/// A line allowance or charge (BG-27 / BG-28).
///
/// `amount` is signed like a [`DocumentAdjustment`]'s: an allowance subtracts it,
/// a charge adds it; the usual input is ≥ 0. It is an exact `Decimal`, reduced
/// once at the document's scale.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LineAdjustment {
    kind: AdjustmentKind,
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    amount: Decimal,
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "codec::basis",
            skip_serializing_if = "Option::is_none"
        )
    )]
    basis: Option<(Decimal, Decimal)>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    reason: Option<String>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    reason_code: Option<String>,
}

impl LineAdjustment {
    /// An adjustment of `kind`.
    #[must_use]
    pub fn new(kind: AdjustmentKind, amount: Decimal) -> Self {
        Self {
            kind,
            amount,
            basis: None,
            reason: None,
            reason_code: None,
        }
    }

    /// A line allowance (BG-27) of `amount`.
    #[must_use]
    pub fn allowance(amount: Decimal) -> Self {
        Self::new(AdjustmentKind::Allowance, amount)
    }

    /// A line charge (BG-28) of `amount`.
    #[must_use]
    pub fn charge(amount: Decimal) -> Self {
        Self::new(AdjustmentKind::Charge, amount)
    }

    /// State the base (BT-137/142) and percentage (BT-138/143). Checked at
    /// build: the reduced amount must be within ±0.02 of `base × percent / 100`
    /// (PEPPOL-EN16931-R040) and `base` must fit the scale.
    #[must_use]
    pub fn with_basis(mut self, base: Decimal, percent: Percent) -> Self {
        self.basis = Some((base, percent.value()));
        self
    }

    /// The reason text (BT-139/144).
    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// The reason code (BT-140/145).
    #[must_use]
    pub fn with_reason_code(mut self, code: impl Into<String>) -> Self {
        self.reason_code = Some(code.into());
        self
    }

    /// Allowance or charge.
    #[must_use]
    pub fn kind(&self) -> AdjustmentKind {
        self.kind
    }

    /// The exact, signed amount as given.
    #[must_use]
    pub fn amount(&self) -> Decimal {
        self.amount
    }

    /// `(base, percent)`, if stated.
    #[must_use]
    pub fn basis(&self) -> Option<(Decimal, Percent)> {
        self.basis.map(|(b, p)| (b, Percent::new(p)))
    }

    /// The reason text.
    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// The reason code.
    #[must_use]
    pub fn reason_code(&self) -> Option<&str> {
        self.reason_code.as_deref()
    }

    pub(crate) fn negated(&self) -> Self {
        Self {
            amount: neg(self.amount),
            basis: self.basis.map(|(b, p)| (neg(b), p)),
            ..self.clone()
        }
    }

    /// The share with exact `amount`; the basis is dropped (a split amount no
    /// longer reproduces `base × percent`).
    fn share(&self, amount: Decimal) -> Self {
        Self {
            amount,
            basis: None,
            ..self.clone()
        }
    }

    /// Reduced and validated (R040).
    fn reduce(&self, scale: AmountScale) -> Result<Amount> {
        let a: Amount = scale.reduce(self.amount)?;
        check_basis(self.basis, a, scale)?;
        Ok(a)
    }
}

/// A document-level allowance or charge (BG-20 / BG-21).
///
/// `amount` is signed: an allowance subtracts it, a charge adds it; the usual
/// input is ≥ 0. It carries its own [`Vat`] (BT-95/96, BT-102/103) or takes the
/// document's default.
///
/// ```
/// use billing::{DocumentAdjustment, Percent, Vat};
/// use rust_decimal::dec;
///
/// let d = DocumentAdjustment::allowance("loyalty", dec!(10)).with_vat(Vat::standard(Percent::new(dec!(19)))?);
/// assert_eq!(d.amount(), dec!(10));
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct DocumentAdjustment {
    kind: AdjustmentKind,
    description: String,
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    amount: Decimal,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    vat: Option<Vat>,
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "codec::basis",
            skip_serializing_if = "Option::is_none"
        )
    )]
    basis: Option<(Decimal, Decimal)>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    reason_code: Option<String>,
}

impl DocumentAdjustment {
    /// An adjustment of `kind`.
    #[must_use]
    pub fn new(kind: AdjustmentKind, description: impl Into<String>, amount: Decimal) -> Self {
        Self {
            kind,
            description: description.into(),
            amount,
            vat: None,
            basis: None,
            reason_code: None,
        }
    }

    /// A document allowance (BG-20); `description` is the reason (BT-97).
    #[must_use]
    pub fn allowance(description: impl Into<String>, amount: Decimal) -> Self {
        Self::new(AdjustmentKind::Allowance, description, amount)
    }

    /// A document charge (BG-21); `description` is the reason (BT-104).
    #[must_use]
    pub fn charge(description: impl Into<String>, amount: Decimal) -> Self {
        Self::new(AdjustmentKind::Charge, description, amount)
    }

    /// Set the VAT treatment.
    #[must_use]
    pub fn with_vat(mut self, vat: Vat) -> Self {
        self.vat = Some(vat);
        self
    }

    /// State the base (BT-93/100) and percentage (BT-94/101); checked at build
    /// as for [`LineAdjustment::with_basis`].
    #[must_use]
    pub fn with_basis(mut self, base: Decimal, percent: Percent) -> Self {
        self.basis = Some((base, percent.value()));
        self
    }

    /// The reason code (BT-98/105).
    #[must_use]
    pub fn with_reason_code(mut self, code: impl Into<String>) -> Self {
        self.reason_code = Some(code.into());
        self
    }

    /// Allowance or charge.
    #[must_use]
    pub fn kind(&self) -> AdjustmentKind {
        self.kind
    }

    /// The reason text.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The exact, signed amount as given.
    #[must_use]
    pub fn amount(&self) -> Decimal {
        self.amount
    }

    /// The VAT treatment, if set (the document's default applies otherwise).
    #[must_use]
    pub fn vat(&self) -> Option<&Vat> {
        self.vat.as_ref()
    }

    /// `(base, percent)`, if stated.
    #[must_use]
    pub fn basis(&self) -> Option<(Decimal, Percent)> {
        self.basis.map(|(b, p)| (b, Percent::new(p)))
    }

    /// The reason code.
    #[must_use]
    pub fn reason_code(&self) -> Option<&str> {
        self.reason_code.as_deref()
    }

    pub(crate) fn set_vat(&mut self, vat: Option<Vat>) {
        self.vat = vat;
    }

    pub(crate) fn negated(&self) -> Self {
        Self {
            amount: neg(self.amount),
            basis: self.basis.map(|(b, p)| (neg(b), p)),
            ..self.clone()
        }
    }

    pub(crate) fn share(&self, amount: Decimal) -> Self {
        Self {
            amount,
            basis: None,
            ..self.clone()
        }
    }

    /// Reduced and validated (R040), located at document adjustment `i`.
    pub(crate) fn reduce(&self, i: usize, scale: AmountScale) -> Result<Amount> {
        let at = |e: Error| e.at(Location::Adjustment(i));
        let a: Amount = scale.reduce(self.amount).map_err(at)?;
        check_basis(self.basis, a, scale).map_err(at)?;
        Ok(a)
    }
}

/// An invoice line (BG-25): a signed [`Quantity`] (BT-129/130) at a
/// non-negative [`UnitPrice`] (BT-146/149/150), an optional stated exact net,
/// an optional [`Vat`], line adjustments (BG-27/28) and opaque labels.
///
/// Direction lives in the quantity's sign; the price must be ≥ 0 (BR-27,
/// checked at build). BT-131 is derived at build:
/// `reduce(quantity × price / base)` (or `reduce(stated net)`) `+ Σ reduce(charges)
/// − Σ reduce(allowances)` — each leaf rounded once in its own right.
///
/// ```
/// use billing::{Line, LineAdjustment, Price, Quantity, UnitPrice};
/// use rust_decimal::dec;
///
/// let line = Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30))))
///     .with_adjustment(LineAdjustment::allowance(dec!(5)).with_reason("bonus"))
///     .with_label("meter", "4711");
/// assert_eq!(line.exact_net()?, dec!(295));
/// assert_eq!(Line::fixed("refund", dec!(-50)).quantity().value(), dec!(-1));
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "codec::RawLine"))]
pub struct Line {
    description: String,
    quantity: Quantity,
    /// The exact quantity `(numerator, denominator)` of a fractional usage
    /// ([`Line::from_usage`]); `quantity` shows its decimal reading.
    #[cfg_attr(
        feature = "serde",
        serde(
            rename = "quantity_exact",
            default,
            with = "codec::ratio",
            skip_serializing_if = "Option::is_none"
        )
    )]
    exact: Option<(Decimal, Decimal)>,
    price: UnitPrice,
    #[cfg_attr(
        feature = "serde",
        serde(
            default,
            with = "codec::opt_dec",
            skip_serializing_if = "Option::is_none"
        )
    )]
    net: Option<Decimal>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    vat: Option<Vat>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    adjustments: Vec<LineAdjustment>,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "BTreeMap::is_empty")
    )]
    labels: BTreeMap<String, String>,
}

impl Line {
    /// `quantity` at `price`.
    #[must_use]
    pub fn new(description: impl Into<String>, quantity: Quantity, price: UnitPrice) -> Self {
        Self {
            description: description.into(),
            quantity,
            exact: None,
            price,
            net: None,
            vat: None,
            adjustments: Vec::new(),
            labels: BTreeMap::new(),
        }
    }

    /// A fixed amount: `±1 C62 × |amount|`, the sign in the quantity. Still a
    /// complete invoice line: quantity (BR-22), unit code (BR-23) and net price
    /// (BR-26) are all stated.
    #[must_use]
    pub fn fixed(description: impl Into<String>, amount: Decimal) -> Self {
        let q = if amount.is_sign_negative() && !amount.is_zero() {
            Decimal::NEGATIVE_ONE
        } else {
            Decimal::ONE
        };
        Self::new(
            description,
            Quantity::new(q, "C62"),
            UnitPrice::new(Price::new(amount.abs())),
        )
    }

    /// A line for an exact [`Usage`] at `price`: BT-129 is the usage's decimal
    /// reading ([`Usage::to_decimal`]) in `unit`, and the price part of BT-131 is
    /// derived from the **exact fraction** × price ÷ base, rounded once at the
    /// document's scale. A usage with denominator 1 gives exactly
    /// [`Line::new`].
    ///
    /// ```
    /// use billing::{Aggregation, AmountScale, Currency, Document, Line, Price, Sample, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// // The average of 1, 0, 0 is 1/3; at 0.015 per unit that is 0.005 → 0.01.
    /// let mean = Aggregation::Mean.apply([dec!(1), dec!(0), dec!(0)].map(Sample::new))?.unwrap();
    /// let line = Line::from_usage("average seats", mean, "C62", UnitPrice::new(Price::new(dec!(0.015))));
    /// assert_eq!(line.exact_quantity(), Some((dec!(1), dec!(3))));
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931).line(line).build()?;
    /// assert_eq!(doc.line_total().to_string(), "0.01000"); // the 28-digit reading would bill 0.00
    /// # Ok::<(), billing::Error>(())
    /// ```
    #[must_use]
    pub fn from_usage(
        description: impl Into<String>,
        usage: Usage,
        unit: impl Into<String>,
        price: UnitPrice,
    ) -> Self {
        let mut line = Self::new(description, Quantity::new(usage.to_decimal(), unit), price);
        if usage.denominator() != Decimal::ONE {
            line.exact = Some((usage.numerator(), usage.denominator()));
        }
        line
    }

    /// Set the VAT treatment.
    #[must_use]
    pub fn with_vat(mut self, vat: Vat) -> Self {
        self.vat = Some(vat);
        self
    }

    /// Append a line allowance or charge.
    #[must_use]
    pub fn with_adjustment(mut self, adjustment: LineAdjustment) -> Self {
        self.adjustments.push(adjustment);
        self
    }

    /// State the exact price part of BT-131 (replacing `quantity × price /
    /// base`); line adjustments still apply. Used for interval series and split
    /// shares; [`Document::price_inconsistent`](crate::Document::price_inconsistent)
    /// reports a stated net that misses R120.
    #[must_use]
    pub fn with_net(mut self, exact: Decimal) -> Self {
        self.net = Some(exact);
        self
    }

    /// Set an opaque label (the engine never reads labels).
    #[must_use]
    pub fn with_label(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.labels.insert(key.into(), value.into());
        self
    }

    /// BT-153.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// BT-129/130, signed.
    #[must_use]
    pub fn quantity(&self) -> &Quantity {
        &self.quantity
    }

    /// The exact quantity `(numerator, denominator)` of a line built from a
    /// fractional [`Usage`] ([`Line::from_usage`]); `None` when BT-129 is exact.
    #[must_use]
    pub fn exact_quantity(&self) -> Option<(Decimal, Decimal)> {
        self.exact
    }

    /// BT-146/149/150.
    #[must_use]
    pub fn price(&self) -> &UnitPrice {
        &self.price
    }

    /// The stated exact price part, if any.
    #[must_use]
    pub fn stated_net(&self) -> Option<Decimal> {
        self.net
    }

    /// The VAT treatment, if set (the document's default applies otherwise).
    #[must_use]
    pub fn vat(&self) -> Option<&Vat> {
        self.vat.as_ref()
    }

    /// BG-27/28.
    #[must_use]
    pub fn adjustments(&self) -> &[LineAdjustment] {
        &self.adjustments
    }

    /// All labels.
    #[must_use]
    pub fn labels(&self) -> &BTreeMap<String, String> {
        &self.labels
    }

    /// One label.
    #[must_use]
    pub fn label(&self, key: &str) -> Option<&str> {
        self.labels.get(key).map(String::as_str)
    }

    /// The exact net before any rounding: the stated net or `quantity × price /
    /// base` (the exact quantity's fraction for [`Line::from_usage`]), plus
    /// charges, minus allowances. A non-terminating quotient (base 3)
    /// is carried to `Decimal`'s 28 digits; the document itself reduces the exact
    /// rational.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidBaseQuantity`] for a zero base, [`ErrorKind::Overflow`].
    pub fn exact_net(&self) -> Result<Decimal> {
        let overflow = || Error::from(ErrorKind::Overflow);
        let mut net = match self.net {
            Some(n) => n,
            None => {
                if self.price.base_quantity().is_zero() {
                    return Err(ErrorKind::InvalidBaseQuantity.into());
                }
                let (q, d) = self.exact.unwrap_or((self.quantity.value(), Decimal::ONE));
                d.checked_mul(self.price.base_quantity())
                    .and_then(|den| q.checked_mul(self.price.price().value())?.checked_div(den))
                    .ok_or_else(overflow)?
            }
        };
        for a in &self.adjustments {
            net = match a.kind {
                AdjustmentKind::Charge => net.checked_add(a.amount),
                AdjustmentKind::Allowance => net.checked_sub(a.amount),
            }
            .ok_or_else(overflow)?;
        }
        Ok(net)
    }

    pub(crate) fn set_vat(&mut self, vat: Option<Vat>) {
        self.vat = vat;
    }

    /// Check BR-27, R121 and R130 (no location attached).
    fn validate(&self) -> Result<()> {
        let p = self.price.price().value();
        if p.is_sign_negative() && !p.is_zero() {
            return Err(ErrorKind::NegativePrice { price: p }.into());
        }
        if self.price.base_quantity() <= Decimal::ZERO {
            return Err(ErrorKind::InvalidBaseQuantity.into());
        }
        if self.price.unit().is_some_and(|u| u != self.quantity.unit()) {
            return Err(ErrorKind::PriceBaseUnit.into());
        }
        Ok(())
    }

    /// The price part reduced once: `reduce(stated)` or `reduce(q × p / base)`
    /// as an exact rational (a leaf in its own right).
    fn price_part(&self, scale: AmountScale) -> Result<Amount> {
        match self.net {
            Some(n) => scale.reduce(n),
            None => match self.exact {
                Some((n, d)) => reduce_exact(
                    &[n, self.price.price().value()],
                    &[d, self.price.base_quantity()],
                    scale,
                ),
                None => reduce_exact(
                    &[self.quantity.value(), self.price.price().value()],
                    &[self.price.base_quantity()],
                    scale,
                ),
            },
        }
    }

    /// Validate and derive: `(BT-131, reduced adjustment amounts, price part)`.
    /// Errors carry the line's location.
    pub(crate) fn derive(
        &self,
        i: usize,
        scale: AmountScale,
    ) -> Result<(Amount, Vec<Amount>, Amount)> {
        self.validate().map_err(|e| e.at(Location::Line(i)))?;
        let pp = self
            .price_part(scale)
            .map_err(|e| e.at(Location::Line(i)))?;
        let mut net = pp;
        let mut amounts = Vec::with_capacity(self.adjustments.len());
        for (k, a) in self.adjustments.iter().enumerate() {
            let at = |e: Error| e.at(Location::LineAdjustment { line: i, index: k });
            let r = a.reduce(scale).map_err(at)?;
            net = net.checked_add(a.kind.signed(r).map_err(at)?).map_err(at)?;
            amounts.push(r);
        }
        Ok((net, amounts, pp))
    }

    /// The leaves at `scale`: the reduced price part, then each adjustment.
    pub(crate) fn leaves(&self, scale: AmountScale) -> Result<Vec<Decimal>> {
        let (_, amounts, pp) = self.derive(0, scale)?;
        let mut v = Vec::with_capacity(1 + amounts.len());
        v.push(pp.to_decimal());
        v.extend(amounts.iter().map(|a| a.to_decimal()));
        Ok(v)
    }

    /// The quantity split by `weights`, exactly, at max(12, its own) decimals.
    pub(crate) fn split_quantity(&self, weights: &[Decimal]) -> Result<Vec<Decimal>> {
        let q = self.quantity.value().normalize();
        let decimals = u8::try_from(q.scale().clamp(12, 28)).unwrap_or(28);
        Ok(split(q, weights, decimals)?
            .into_iter()
            .map(|d| d.normalize())
            .collect())
    }

    /// A share: `quantity`, the stated net `parts[0]`, adjustments `parts[1..]`.
    pub(crate) fn share(&self, quantity: Decimal, parts: &[Decimal]) -> Self {
        Self {
            quantity: Quantity::new(quantity, self.quantity.unit()),
            exact: None,
            net: Some(parts[0]),
            adjustments: self
                .adjustments
                .iter()
                .zip(&parts[1..])
                .map(|(a, &p)| a.share(p))
                .collect(),
            ..self.clone()
        }
    }

    pub(crate) fn negated(&self) -> Self {
        Self {
            quantity: Quantity::new(neg(self.quantity.value()), self.quantity.unit()),
            exact: self.exact.map(|(n, d)| (neg(n), d)),
            net: self.net.map(neg),
            adjustments: self
                .adjustments
                .iter()
                .map(LineAdjustment::negated)
                .collect(),
            ..self.clone()
        }
    }

    /// Prorate the line into segments by `weights` (e.g. days of a tariff
    /// change): the reduced price part and every reduced adjustment are split
    /// exactly at `scale` ([`split_matrix`]), the quantity is split exactly at
    /// max(12, its own) decimals, and each segment states its net. The segments'
    /// BT-131 sum to this line's BT-131 at `scale` exactly; adjustment bases are
    /// dropped (a share no longer reproduces `base × percent`).
    ///
    /// ```
    /// use billing::{AmountScale, Line, Price, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// let fee = Line::new("base fee", Quantity::new(dec!(1), "MON"), UnitPrice::new(Price::new(dec!(10))));
    /// let parts = fee.segments(&[dec!(14), dec!(17)], AmountScale::EN16931)?;
    /// assert_eq!(parts[0].stated_net(), Some(dec!(4.52)));
    /// assert_eq!(parts[1].stated_net(), Some(dec!(5.48)));
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// As the line's build checks, and as [`split_matrix`].
    pub fn segments(&self, weights: &[Decimal], scale: AmountScale) -> Result<Vec<Line>> {
        let leaves = self.leaves(scale)?;
        let rows = split_matrix(&leaves, weights, scale.decimals)?;
        let qs = self.split_quantity(weights)?;
        Ok(qs
            .iter()
            .enumerate()
            .map(|(j, &q)| {
                let parts: Vec<Decimal> = rows.iter().map(|r| r[j]).collect();
                self.share(q, &parts)
            })
            .collect())
    }
}

/// How [`apportion`] sizes the adjustment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Adjust {
    /// A percentage of each VAT group's base, rounded once per group.
    Percent(Percent),
    /// A fixed amount (fitting the scale), split across the groups by their
    /// bases, exactly (largest remainder).
    Fixed(#[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))] Decimal),
}

/// One [`DocumentAdjustment`] per VAT group among the lines `applies` selects,
/// so a document-level discount lowers each group's base it belongs to.
///
/// Lines are evaluated at `scale` (their BT-131 as the document derives it).
/// Groups are keyed by (category, rate); a line without VAT forms its own group
/// whose adjustment takes the document's default VAT. `Percent(p)`: each group's
/// amount is `round(base × p / 100)` once, with the basis `(base, p)` stated
/// (not below two decimals, where R040's ±0.02 cannot hold).
/// `Fixed(f)`: `f` is split by the group bases (all ≥ 0, or all ≤ 0). Groups
/// whose amount is zero yield no adjustment. Order: first appearance.
///
/// ```
/// use billing::{AdjustmentKind, Adjust, AmountScale, Document, Currency, Line, Percent, Vat, apportion};
/// use rust_decimal::dec;
///
/// let lines = [
///     Line::fixed("a", dec!(400)).with_vat(Vat::standard(Percent::new(dec!(19)))?),
///     Line::fixed("b", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?),
/// ];
/// let s = AmountScale::EN16931;
/// let discount = apportion(AdjustmentKind::Allowance, "10 %", Adjust::Percent(Percent::new(dec!(10))), &lines, |_| true, s)?;
/// let doc = Document::builder(Currency::EUR, s).lines(lines).adjustments(discount).build()?;
/// assert_eq!(doc.vat_total().to_string(), "74.70000"); // 360 × 19 % + 90 × 7 %
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Errors
/// As the lines' build checks (located); [`ErrorKind::InvalidWeights`] for a
/// `Fixed` amount over bases of mixed sign or all zero; as [`split`].
pub fn apportion(
    kind: AdjustmentKind,
    description: &str,
    adjust: Adjust,
    lines: &[Line],
    applies: impl Fn(&Line) -> bool,
    scale: AmountScale,
) -> Result<Vec<DocumentAdjustment>> {
    // (key, vat, base) in first-appearance order.
    type Group = (Option<(crate::TaxCategory, Decimal)>, Option<Vat>, Amount);
    let mut groups: Vec<Group> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if !applies(line) {
            continue;
        }
        let (net, _, _) = line.derive(i, scale)?;
        let key = line.vat.as_ref().map(Vat::key);
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => g.2 = g.2.checked_add(net).map_err(|e| e.at(Location::Line(i)))?,
            None => groups.push((key, line.vat.clone(), net)),
        }
    }
    let amounts: Vec<(Amount, Option<(Decimal, Decimal)>)> = match adjust {
        Adjust::Percent(p) => {
            let p = p.value();
            groups
                .iter()
                .map(|g| {
                    let a = reduce_exact(&[g.2.to_decimal(), p], &[Decimal::ONE_HUNDRED], scale)?;
                    // Below two decimals the rounded amount cannot meet R040's
                    // ±0.02, so no basis is stated there.
                    let basis = (scale.decimals >= 2).then(|| (g.2.to_decimal().normalize(), p));
                    Ok((a, basis))
                })
                .collect::<Result<_>>()?
        }
        Adjust::Fixed(f) => {
            if groups.is_empty() {
                return Ok(Vec::new());
            }
            let bases: Vec<Decimal> = groups.iter().map(|g| g.2.to_decimal()).collect();
            let weights: Vec<Decimal> =
                if bases.iter().all(|b| !b.is_sign_negative() || b.is_zero()) {
                    bases
                } else if bases.iter().all(|b| b.is_sign_negative() || b.is_zero()) {
                    bases.iter().map(|b| -*b).collect()
                } else {
                    return Err(ErrorKind::InvalidWeights.into());
                };
            split(f, &weights, scale.decimals)?
                .into_iter()
                .map(|a| Ok((Amount::from_decimal(a)?, None)))
                .collect::<Result<_>>()?
        }
    };
    Ok(groups
        .into_iter()
        .zip(amounts)
        .filter(|(_, (a, _))| !a.is_zero())
        .map(|((_, vat, _), (a, basis))| {
            let d = a.to_decimal().normalize();
            DocumentAdjustment {
                kind,
                description: description.into(),
                amount: d,
                vat,
                basis,
                reason_code: None,
            }
        })
        .collect())
}

/// A per-unit levy (Stromsteuer, Konzessionsabgabe …) as **one line**:
/// Σ quantity of the lines `applies` selects × `rate`, in `unit`. The levy is an
/// ordinary line, taxed like any other (give it a [`Vat`] or let the document's
/// default apply).
///
/// ```
/// use billing::{Price, UnitPrice, Line, Quantity, per_unit_levy};
/// use rust_decimal::dec;
///
/// let lines = [Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30))))];
/// let tax = per_unit_levy("electricity tax", &lines, |_| true, UnitPrice::new(Price::new(dec!(0.0205))), "KWH")?;
/// assert_eq!(tax.exact_net()?, dec!(20.5));
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Errors
/// [`ErrorKind::PriceBaseUnit`] (located) if a selected line's quantity is not
/// in `unit`; [`ErrorKind::Overflow`].
pub fn per_unit_levy(
    description: &str,
    lines: &[Line],
    applies: impl Fn(&Line) -> bool,
    rate: UnitPrice,
    unit: &str,
) -> Result<Line> {
    let mut total = Decimal::ZERO;
    for (i, line) in lines.iter().enumerate() {
        if !applies(line) {
            continue;
        }
        if line.quantity.unit() != unit {
            return Err(Error::from(ErrorKind::PriceBaseUnit).at(Location::Line(i)));
        }
        total = total
            .checked_add(line.quantity.value())
            .ok_or_else(|| Error::from(ErrorKind::Overflow).at(Location::Line(i)))?;
    }
    Ok(Line::new(description, Quantity::new(total, unit), rate))
}

impl crate::tariff::Piece {
    /// The piece as lines: the usage line ([`Line::from_usage`] of `quantity`
    /// in `unit` at `unit_price`, if the quantity is non-zero) and, for a tier
    /// flat fee, a [`Line::fixed`] of the fee. Both carry `description`.
    #[must_use]
    pub fn into_lines(self, description: &str, unit: &str) -> Vec<Line> {
        let mut v = Vec::with_capacity(2);
        if !self.quantity.numerator().is_zero() {
            v.push(Line::from_usage(
                description,
                self.quantity,
                unit,
                UnitPrice::new(self.unit_price),
            ));
        }
        if let Some(flat) = self.flat.filter(|f| !f.is_zero()) {
            v.push(Line::fixed(description, flat));
        }
        v
    }
}

impl crate::tariff::SeriesTotal {
    /// The series as one line at the average price with the **exact** net
    /// stated, so the document rounds Σ qᵢ·pᵢ once. A negative average puts
    /// the direction in the quantity (BR-27: the price stays ≥ 0).
    #[must_use]
    pub fn into_line(self, description: &str, unit: &str) -> Line {
        let q = if self.is_negative_average() {
            neg(self.quantity)
        } else {
            self.quantity
        };
        Line::new(
            description,
            Quantity::new(q, unit),
            UnitPrice::new(self.average_price),
        )
        .with_net(self.net)
    }
}

impl crate::tariff::FeeTotal {
    /// The fee total as one line with the **exact** fee stated as its net, so
    /// the document rounds Σ per-transaction fees once: quantity = the charged
    /// transactions in `unit`, price = |fee| ÷ charged (BT-146, for display, to
    /// `Decimal`'s 28 digits). A net refund of fees puts the direction in the
    /// quantity (BR-27: the price stays ≥ 0); no charged transaction gives a
    /// zero line.
    ///
    /// ```
    /// use billing::{AmountScale, Currency, Document, Percent, TransactionFee};
    /// use rust_decimal::dec;
    ///
    /// // Three fees of 0.005: 0.015 exactly, rounded once to 0.02 (not 3 × 0.01).
    /// let total = TransactionFee::new(Percent::new(dec!(0.5))).rate([dec!(1), dec!(1), dec!(1)])?;
    /// let line = total.into_line("card fees", "C62");
    /// assert_eq!((line.quantity().value(), line.stated_net()), (dec!(3), Some(dec!(0.015))));
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931).line(line).build()?;
    /// assert_eq!(doc.line_net(0).to_decimal(), dec!(0.02));
    /// # Ok::<(), billing::Error>(())
    /// ```
    #[must_use]
    pub fn into_line(self, description: impl Into<String>, unit: impl Into<String>) -> Line {
        let charged = Decimal::from(self.charged);
        let price = if self.charged == 0 {
            Decimal::ZERO
        } else {
            // |fee| ≤ Decimal::MAX and charged ≥ 1: the quotient always fits.
            self.fee
                .abs()
                .checked_div(charged)
                .unwrap_or_default()
                .normalize()
        };
        let q = if self.fee < Decimal::ZERO {
            neg(charged)
        } else {
            charged
        };
        Line::new(
            description,
            Quantity::new(q, unit),
            UnitPrice::new(Price::new(price)),
        )
        .with_net(self.fee)
    }
}

#[cfg(feature = "serde")]
pub(crate) mod codec {
    //! Serde codecs for optional decimals and `(base, percent)` pairs.

    use alloc::collections::BTreeMap;
    use alloc::string::String;
    use alloc::vec::Vec;

    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use crate::{LineAdjustment, Quantity, UnitPrice, Usage, Vat};

    #[derive(Serialize, Deserialize)]
    struct Dec(#[serde(with = "crate::decimal::serde_str")] Decimal);

    pub(crate) mod opt_dec {
        use super::*;
        #[allow(clippy::ref_option)]
        pub(crate) fn serialize<S: Serializer>(
            d: &Option<Decimal>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            d.map(Dec).serialize(s)
        }
        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<Decimal>, D::Error> {
            Ok(Option::<Dec>::deserialize(d)?.map(|x| x.0))
        }
    }

    #[derive(Serialize, Deserialize)]
    struct Basis {
        #[serde(with = "crate::decimal::serde_str")]
        base: Decimal,
        #[serde(with = "crate::decimal::serde_str")]
        percent: Decimal,
    }

    #[derive(Serialize, Deserialize)]
    struct Ratio {
        #[serde(with = "crate::decimal::serde_str")]
        numerator: Decimal,
        #[serde(with = "crate::decimal::serde_str")]
        denominator: Decimal,
    }

    pub(crate) mod ratio {
        use super::*;
        #[allow(clippy::ref_option)]
        pub(crate) fn serialize<S: Serializer>(
            d: &Option<(Decimal, Decimal)>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            d.map(|(numerator, denominator)| Ratio {
                numerator,
                denominator,
            })
            .serialize(s)
        }
        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<(Decimal, Decimal)>, D::Error> {
            Ok(Option::<Ratio>::deserialize(d)?.map(|r| (r.numerator, r.denominator)))
        }
    }

    /// A [`Line`](super::Line) as read, before its exact quantity is checked
    /// against BT-129.
    #[derive(Deserialize)]
    pub(crate) struct RawLine {
        description: String,
        quantity: Quantity,
        #[serde(rename = "quantity_exact", default, with = "ratio")]
        exact: Option<(Decimal, Decimal)>,
        price: UnitPrice,
        #[serde(default, with = "opt_dec")]
        net: Option<Decimal>,
        #[serde(default)]
        vat: Option<Vat>,
        #[serde(default)]
        adjustments: Vec<LineAdjustment>,
        #[serde(default)]
        labels: BTreeMap<String, String>,
    }

    impl TryFrom<RawLine> for super::Line {
        type Error = crate::Error;
        /// The exact quantity must have a denominator > 0 and BT-129 must be its
        /// decimal reading.
        fn try_from(raw: RawLine) -> crate::Result<Self> {
            if let Some((n, d)) = raw.exact {
                let shown = Usage::checked(n, d)?.to_decimal();
                if shown != raw.quantity.value() {
                    return Err(crate::ErrorKind::Malformed {
                        input: alloc::format!("{}", raw.quantity.value()),
                    }
                    .into());
                }
            }
            Ok(Self {
                description: raw.description,
                quantity: raw.quantity,
                exact: raw.exact,
                price: raw.price,
                net: raw.net,
                vat: raw.vat,
                adjustments: raw.adjustments,
                labels: raw.labels,
            })
        }
    }

    pub(crate) mod basis {
        use super::*;
        #[allow(clippy::ref_option)]
        pub(crate) fn serialize<S: Serializer>(
            d: &Option<(Decimal, Decimal)>,
            s: S,
        ) -> Result<S::Ok, S::Error> {
            d.map(|(base, percent)| Basis { base, percent })
                .serialize(s)
        }
        pub(crate) fn deserialize<'de, D: Deserializer<'de>>(
            d: D,
        ) -> Result<Option<(Decimal, Decimal)>, D::Error> {
            Ok(Option::<Basis>::deserialize(d)?.map(|b| (b.base, b.percent)))
        }
    }
}
