//! [`Document`]: lines, adjustments and settlement in, every EN 16931 total
//! derived once at [`build`](DocumentBuilder::build).

use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use rust_decimal::Decimal;

use crate::line::SLACK;
use crate::vat::units_exact;
use crate::{
    Adjust, AdjustmentKind, Amount, AmountScale, CashRounding, Credit, CreditApplication,
    CreditPlacement, Currency, DocumentAdjustment, Error, ErrorKind, Line, Location, Payment,
    Result, TaxCategory, Vat, VatAccounting, VatError, VatGroup, split_matrix,
};

/// A whole-document allowance or charge given to
/// [`DocumentBuilder::apportion`]: a builder input, materialised at build into
/// one [`DocumentAdjustment`] per VAT group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct Apportion {
    kind: AdjustmentKind,
    description: String,
    adjust: Adjust,
}

/// Whether a [`Bound`] is a floor or a cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BoundKind {
    /// The basis must reach the amount (a minimum charge).
    Minimum,
    /// The basis must not exceed the amount (a cap).
    Maximum,
}

/// Which total a [`Bound`] constrains.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Basis {
    /// BT-109, the total without VAT.
    Net,
    /// BT-112, the total with VAT.
    Gross,
}

/// A minimum or maximum on a document total, settled at build into one
/// ordinary line. The document keeps both: the bound as an input
/// ([`Document::bounds`]) and its line ([`Document::bound_lines`]).
///
/// A minimum adds the **smallest** amount, in whole units of the scale, that
/// makes the basis reach the bound; a maximum adds the **largest** (negative)
/// amount that keeps it at or below. For [`Basis::Gross`] the line joins its
/// VAT group before the group's tax is rounded, so the bound holds exactly
/// under every [`Rounding`](crate::Rounding). The line is
/// `Line::fixed(description, x)` with the bound's VAT (or the default VAT).
///
/// Refusals are located at [`Location::Bound`] (0-based, in call order):
/// [`ErrorKind::InvalidBound`] for an amount not above zero or not fitting the
/// scale, a second minimum or maximum, a minimum and maximum on different bases
/// or with minimum > maximum, a minimum on a credit, or a bound that does not
/// hold after settlement; [`ErrorKind::MissingVat`] for a bound without VAT in
/// a VAT document.
///
/// ```
/// use billing::{AmountScale, Basis, Bound, Currency, Document, Line, Percent, Vat};
/// use rust_decimal::dec;
///
/// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
///     .line(Line::fixed("usage", dec!(3.10)))
///     .bound(Bound::minimum(Basis::Gross, dec!(5.18), "minimum charge"))
///     .build()?;
/// assert_eq!(doc.tax_inclusive_total().to_string(), "5.18000");
/// # Ok::<(), billing::Error>(())
/// ```
///
/// **A commitment true-up is a minimum on [`Basis::Net`]**: a contract
/// committing to 1 000.00 net with 600.00 of usage bills a 400.00 top-up line.
/// Bounds settle before [`Credit`](crate::Credit)s are drawn, so a before-tax
/// credit cannot hide a shortfall: it is drawn from the trued-up total.
///
/// ```
/// use billing::{AmountScale, Basis, Bound, Credit, Currency, Document, Line, Percent, Vat};
/// use rust_decimal::dec;
///
/// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
///     .line(Line::fixed("usage", dec!(600)))
///     .bound(Bound::minimum(Basis::Net, dec!(1000), "commitment true-up"))
///     .credit(Credit::before_tax("prepaid credit", dec!(100)))
///     .build()?;
/// assert_eq!(doc.line_net(1).to_string(), "400.00000"); // the top-up
/// assert_eq!(doc.tax_exclusive_total().to_string(), "900.00000");
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Bound {
    kind: BoundKind,
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    amount: Decimal,
    basis: Basis,
    description: String,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    vat: Option<Vat>,
}

impl Bound {
    /// A bound of `kind` on `basis`. `amount` must be > 0 and fit the
    /// document's scale; [`DocumentBuilder::build`] refuses it otherwise
    /// ([`ErrorKind::InvalidBound`] at [`Location::Bound`]).
    #[must_use]
    pub fn new(
        kind: BoundKind,
        basis: Basis,
        amount: Decimal,
        description: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            amount,
            basis,
            description: description.into(),
            vat: None,
        }
    }

    /// A minimum on `basis`.
    #[must_use]
    pub fn minimum(basis: Basis, amount: Decimal, description: impl Into<String>) -> Self {
        Self::new(BoundKind::Minimum, basis, amount, description)
    }

    /// A maximum on `basis`.
    #[must_use]
    pub fn maximum(basis: Basis, amount: Decimal, description: impl Into<String>) -> Self {
        Self::new(BoundKind::Maximum, basis, amount, description)
    }

    /// The VAT of the bound line (default: the document's default VAT).
    #[must_use]
    pub fn with_vat(mut self, vat: Vat) -> Self {
        self.vat = Some(vat);
        self
    }

    /// Minimum or maximum.
    #[must_use]
    pub fn kind(&self) -> BoundKind {
        self.kind
    }

    /// The bound.
    #[must_use]
    pub fn amount(&self) -> Decimal {
        self.amount
    }

    /// Net or gross.
    #[must_use]
    pub fn basis(&self) -> Basis {
        self.basis
    }

    /// The bound line's description.
    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    /// The bound line's VAT, if set.
    #[must_use]
    pub fn vat(&self) -> Option<&Vat> {
        self.vat.as_ref()
    }
}

/// Builds a [`Document`]. Every method appends (lines, adjustments,
/// apportioned adjustments, bounds, credits, payments) or
/// sets a distinct single-valued setting (default VAT, cash
/// rounding, VAT accounting); **call order never matters**, except among
/// [`credit`](Self::credit)s, whose order is their draw-down priority, and
/// [`payment`](Self::payment)s, whose order is their index. Setting a
/// single-valued setting twice is refused at build with
/// [`ErrorKind::ConflictingSetting`], whatever the order.
#[derive(Debug, Clone, PartialEq)]
#[must_use]
pub struct DocumentBuilder {
    currency: Currency,
    scale: AmountScale,
    default_vat: Vec<Vat>,
    lines: Vec<Line>,
    adjustments: Vec<DocumentAdjustment>,
    apportions: Vec<Apportion>,
    bounds: Vec<Bound>,
    credits: Vec<Credit>,
    payments: Vec<Payment>,
    cash_rounding: Vec<CashRounding>,
    vat_accounting: Vec<VatAccounting>,
    tax_partition: Option<Vec<Amount>>,
}

/// The value of a setting given at most once.
fn single<T: Clone>(values: &[T], setting: &'static str) -> Result<Option<T>> {
    match values {
        [] => Ok(None),
        [one] => Ok(Some(one.clone())),
        _ => Err(ErrorKind::ConflictingSetting { setting }.into()),
    }
}

impl DocumentBuilder {
    /// The VAT for every line, adjustment and bound that has none of its own.
    pub fn default_vat(mut self, vat: Vat) -> Self {
        self.default_vat.push(vat);
        self
    }

    /// Append a line.
    pub fn line(mut self, line: Line) -> Self {
        self.lines.push(line);
        self
    }

    /// Append lines.
    pub fn lines(mut self, lines: impl IntoIterator<Item = Line>) -> Self {
        self.lines.extend(lines);
        self
    }

    /// Append a document allowance or charge.
    pub fn adjustment(mut self, adjustment: DocumentAdjustment) -> Self {
        self.adjustments.push(adjustment);
        self
    }

    /// Append document allowances or charges.
    pub fn adjustments(
        mut self,
        adjustments: impl IntoIterator<Item = DocumentAdjustment>,
    ) -> Self {
        self.adjustments.extend(adjustments);
        self
    }

    /// Append an allowance or charge on the **whole** document, split by VAT
    /// group: at build, after the default VAT is filled in, it becomes one
    /// [`DocumentAdjustment`] per (category, rate) group of the caller's lines,
    /// exactly as [`apportion`](crate::apportion) over every line does
    /// (`Percent`: each group's base × p / 100 rounded once, basis stated;
    /// `Fixed`: split by the group bases). Bound lines are settled afterwards
    /// and are not adjusted. The materialised adjustments follow the caller's
    /// in [`Document::adjustments`]; the document keeps this input, so
    /// [`Document::into_builder`] and serde re-apply it to the edited lines.
    /// For a discount on some lines only, use the free
    /// [`apportion`](crate::apportion) with a filter.
    ///
    /// ```
    /// use billing::{Adjust, AdjustmentKind, AmountScale, Currency, Document, Line, Percent, Vat};
    /// use rust_decimal::dec;
    ///
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    ///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    ///     .line(Line::fixed("full", dec!(400)))
    ///     .line(Line::fixed("reduced", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
    ///     .apportion(AdjustmentKind::Allowance, "10 % discount", Adjust::Percent(Percent::new(dec!(10))))
    ///     .build()?;
    /// assert_eq!(doc.adjustments().len(), 2); // one per VAT group
    /// assert_eq!(doc.vat_total().to_string(), "74.70000"); // 360 × 19 % + 90 × 7 %
    /// # Ok::<(), billing::Error>(())
    /// ```
    pub fn apportion(
        mut self,
        kind: AdjustmentKind,
        description: impl Into<String>,
        adjust: Adjust,
    ) -> Self {
        self.apportions.push(Apportion {
            kind,
            description: description.into(),
            adjust,
        });
        self
    }

    /// Add a bound (settled last, into a line): at most one minimum and one
    /// maximum, on the same basis.
    pub fn bound(mut self, bound: Bound) -> Self {
        self.bounds.push(bound);
        self
    }

    /// Append a [`Credit`], drawn down at build after the credits added
    /// before it (call order is the priority). Build order: lines →
    /// apportioned adjustments → bounds → **before-tax credits** → VAT →
    /// payments → **after-tax credits** → cash rounding.
    ///
    /// A before-tax credit draws `min(balance, remaining BT-109)` (never below
    /// zero) and becomes one document allowance per VAT group, split by the
    /// groups' bases exactly at the document's scale (largest remainder, as
    /// [`Adjust::Fixed`] in [`apportion`](Self::apportion)) and carrying the
    /// group's VAT; on a VAT-free document it is one allowance without VAT. An
    /// after-tax credit draws `min(balance, amount still due after the
    /// payments and earlier after-tax credits)` (never below zero) and adds
    /// it to BT-113 as a [`Payment::Gross`] entry of
    /// [`Document::paid_entries`]. A document whose BT-109 before credits is
    /// not above zero (a credit note) draws nothing.
    ///
    /// ```
    /// use billing::{AmountScale, Credit, Currency, Document, Line, Percent, Vat};
    /// use rust_decimal::dec;
    ///
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    ///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    ///     .line(Line::fixed("full", dec!(400)))
    ///     .line(Line::fixed("reduced", dec!(100)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
    ///     .credit(Credit::before_tax("contract credit", dec!(50)))
    ///     .build()?;
    /// assert_eq!(doc.vat_total().to_string(), "74.70000"); // 360 × 19 % + 90 × 7 %
    /// assert_eq!(doc.credit_applications()[0].applied.to_string(), "50.00000");
    /// # Ok::<(), billing::Error>(())
    /// ```
    pub fn credit(mut self, credit: Credit) -> Self {
        self.credits.push(credit);
        self
    }

    /// Append a [`Payment`] (BT-113 = Σ gross of all payments): an itemised
    /// advance or a gross amount. Any number, kept in call order (their
    /// [`Location::Payment`] index).
    ///
    /// ```
    /// use billing::{Advance, AmountScale, Currency, Document, Line, Payment, Percent, Vat};
    /// use rust_decimal::dec;
    ///
    /// let v19 = Vat::standard(Percent::new(dec!(19)))?;
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    ///     .default_vat(v19.clone())
    ///     .line(Line::fixed("works", dec!(1000)))
    ///     .payment(Payment::Advance(Advance { vat: v19, net: dec!(300), tax: dec!(57) }))
    ///     .payment(Payment::Gross { description: "transfer".into(), amount: dec!(100) })
    ///     .build()?;
    /// assert_eq!(doc.prepaid().to_string(), "457.00000");
    /// assert_eq!(doc.payable().to_string(), "733.00000");
    /// # Ok::<(), billing::Error>(())
    /// ```
    pub fn payment(mut self, payment: Payment) -> Self {
        self.payments.push(payment);
        self
    }

    /// Append payments, in order.
    pub fn payments(mut self, payments: impl IntoIterator<Item = Payment>) -> Self {
        self.payments.extend(payments);
        self
    }

    /// Set the cash rounding (BT-114).
    pub fn cash_rounding(mut self, rounding: CashRounding) -> Self {
        self.cash_rounding.push(rounding);
        self
    }

    /// Set the VAT accounting currency (BT-6 / BT-111).
    pub fn vat_accounting(mut self, accounting: VatAccounting) -> Self {
        self.vat_accounting.push(accounting);
        self
    }

    pub(crate) fn tax_partition(mut self, partition: Vec<Amount>) -> Self {
        self.tax_partition = Some(partition);
        self
    }

    /// Validate the inputs and derive every total.
    ///
    /// 1. Fill the default VAT; a document is all-VAT or VAT-free
    ///    ([`ErrorKind::MissingVat`]).
    /// 2. Validate each leaf: price ≥ 0 (BR-27), base > 0 (R121), price unit =
    ///    quantity unit (R130), adjustment basis (R040) — each error located.
    /// 3. Line net (BT-131) = `reduce(price part) + Σ reduce(charges) −
    ///    Σ reduce(allowances)`; document adjustments reduced once.
    ///    [`apportion`](Self::apportion) inputs become adjustments here, over
    ///    the caller's lines.
    /// 4. Bounds, as lines (a commitment minimum is a [`Bound`]).
    /// 5. Before-tax [`Credit`](crate::Credit)s, in call order: each draws at
    ///    most the remaining tax-exclusive total and becomes allowances split
    ///    exactly across the VAT groups by base.
    /// 6. VAT groups by (category, rate): base = Σ members; tax =
    ///    `reduce(base × rate / 100)` once. Different exemption reasons in one
    ///    group: [`ErrorKind::ConflictingExemption`]; `O` beside another group
    ///    (BR-O-11) or `B` beside `S` (BR-B-02): [`ErrorKind::InvalidVat`].
    /// 7. Totals and the payments (each located at [`Location::Payment`]);
    ///    then after-tax credits, in call order, each
    ///    drawing at most what is still due (added to BT-113); then cash
    ///    rounding and VAT accounting.
    ///
    /// # Errors
    /// As above; [`ErrorKind::ScaleMismatch`] for a scale past 5 decimals;
    /// [`ErrorKind::InvalidCredit`] at [`Location::Credit`] for a balance that
    /// is not positive or does not fit the scale;
    /// as [`apportion`](crate::apportion) for an apportioned adjustment;
    /// [`ErrorKind::Overflow`] (located where possible), also for any
    /// derived amount equal to `Amount::MIN` (it could not be negated).
    pub fn build(self) -> Result<Document> {
        self.scale.check()?;
        let default_vat = single(&self.default_vat, "default_vat")?;
        let cash_rounding = single(&self.cash_rounding, "cash_rounding")?;
        let vat_accounting = single(&self.vat_accounting, "vat_accounting")?;
        if let Some(c) = &cash_rounding {
            c.increment_at(self.scale)?;
        }
        if let Some(v) = &vat_accounting {
            v.validate(self.currency)?;
        }
        let balances = self
            .credits
            .iter()
            .enumerate()
            .map(|(i, c)| {
                c.balance_at(self.scale)
                    .map_err(|e| e.at(Location::Credit(i)))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut lines = self.lines;
        let mut adjustments = self.adjustments;
        let mut bounds = self.bounds;
        for l in &mut lines {
            if l.vat().is_none() {
                l.set_vat(default_vat.clone());
            }
        }
        for a in &mut adjustments {
            if a.vat().is_none() {
                a.set_vat(default_vat.clone());
            }
        }
        for b in &mut bounds {
            if b.vat.is_none() {
                b.vat.clone_from(&default_vat);
            }
        }
        // All-VAT or VAT-free (bounds included).
        let has = |v: Option<&Vat>| v.is_some();
        let any_vat = lines.iter().any(|l| has(l.vat()))
            || adjustments.iter().any(|a| has(a.vat()))
            || bounds.iter().any(|b| b.vat.is_some());
        if any_vat {
            if let Some(i) = lines.iter().position(|l| l.vat().is_none()) {
                return Err(Error::from(ErrorKind::MissingVat).at(Location::Line(i)));
            }
            if let Some(i) = adjustments.iter().position(|a| a.vat().is_none()) {
                let rule = match adjustments[i].kind() {
                    AdjustmentKind::Allowance => crate::error::rules::BR_32,
                    AdjustmentKind::Charge => crate::error::rules::BR_37,
                };
                return Err(Error::from(ErrorKind::MissingVat)
                    .at(Location::Adjustment(i))
                    .with_rule(rule));
            }
            if let Some(i) = bounds.iter().position(|b| b.vat.is_none()) {
                return Err(Error::from(ErrorKind::MissingVat).at(Location::Bound(i)));
            }
        }
        // Whole-document adjustments, one per VAT group of the caller's lines.
        let caller_adjustments = adjustments.len();
        for p in &self.apportions {
            let made = crate::apportion(
                p.kind,
                &p.description,
                p.adjust,
                &lines,
                |_| true,
                self.scale,
            )?;
            adjustments.extend(made.into_iter().map(|mut a| {
                if a.vat().is_none() {
                    a.set_vat(default_vat.clone());
                }
                a
            }));
        }
        let apportioned = adjustments.len() - caller_adjustments;
        let mut inputs = Inputs {
            currency: self.currency,
            scale: self.scale,
            payments: &self.payments,
            after_tax: Amount::ZERO,
            cash_rounding,
            vat_accounting,
        };
        // Bounds: at most one minimum and one maximum, on one basis, each
        // > 0 and fitting the scale, minimum ≤ maximum; minimum first.
        check_bounds(&bounds, self.scale)?;
        // Minimum first; errors keep each bound's builder index.
        let mut order: Vec<usize> = (0..bounds.len()).collect();
        order.sort_by_key(|&i| bounds[i].kind);
        let caller_lines = lines.len();
        for &i in &order {
            let b = &bounds[i];
            let d = derive(&inputs, &lines, &adjustments, None)?;
            if let Some(x) = settle(&inputs, &d, b).map_err(|e| e.at(Location::Bound(i)))? {
                let mut line = Line::fixed(b.description.clone(), x);
                line.set_vat(b.vat.clone());
                lines.push(line);
            }
        }
        let bound_lines = lines.len() - caller_lines;
        let partition = self.tax_partition.as_deref();
        let mut d = derive(&inputs, &lines, &adjustments, partition)?;
        // Bounds hold after settlement, before any credit is drawn.
        for &i in &order {
            let b = &bounds[i];
            let v = basis_value(&d.totals, b.basis);
            let ok = match b.kind {
                BoundKind::Minimum => v.to_decimal() >= b.amount,
                BoundKind::Maximum => v.to_decimal() <= b.amount,
            };
            if !ok {
                return Err(Error::from(ErrorKind::InvalidBound).at(Location::Bound(i)));
            }
        }
        bounds.sort_by_key(|b| b.kind);
        // Credits, in priority order: before tax against BT-109, then after
        // tax against the amount due. A credit note draws nothing.
        let credit_note = !d.totals.tax_exclusive.is_positive();
        let mut applied = alloc::vec![Amount::ZERO; balances.len()];
        let uncredited = adjustments.len();
        if !credit_note {
            for (i, c) in self.credits.iter().enumerate() {
                if c.placement != CreditPlacement::BeforeTax {
                    continue;
                }
                let draw = balances[i].min(d.totals.tax_exclusive.max(Amount::ZERO));
                if draw.is_positive() {
                    let at = |e: Error| e.at(Location::Credit(i));
                    let made = credit_allowances(&c.description, draw, &d.groups, self.scale)
                        .map_err(at)?;
                    adjustments.extend(made);
                    d = derive(&inputs, &lines, &adjustments, partition).map_err(at)?;
                }
                applied[i] = draw;
            }
            let mut due = d.totals.tax_inclusive.checked_sub(d.totals.prepaid)?;
            for (i, c) in self.credits.iter().enumerate() {
                if c.placement != CreditPlacement::AfterTax {
                    continue;
                }
                let draw = balances[i].min(due.max(Amount::ZERO));
                due = due.checked_sub(draw)?;
                inputs.after_tax = inputs.after_tax.checked_add(draw)?;
                applied[i] = draw;
            }
            if inputs.after_tax.is_positive() {
                d = derive(&inputs, &lines, &adjustments, partition)?;
            }
        }
        let credited = adjustments.len() - uncredited;
        let applications = balances
            .iter()
            .zip(&applied)
            .enumerate()
            .map(|(credit, (b, a))| {
                Ok(CreditApplication {
                    credit,
                    applied: *a,
                    remaining: b.checked_sub(*a)?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Document {
            currency: self.currency,
            scale: self.scale,
            lines,
            bound_lines,
            bounds,
            adjustments,
            apportioned,
            apportions: self.apportions,
            credited,
            applications,
            credits: self.credits,
            after_tax: inputs.after_tax,
            payments: self.payments,
            cash_rounding,
            vat_accounting,
            tax_partition: self.tax_partition,
            nets: d.nets,
            line_adjustments: d.line_adjustments,
            adjustment_amounts: d.adjustment_amounts,
            groups: d.groups,
            totals: d.totals,
        })
    }
}

/// Each bound > 0 and fitting the scale; at most one of each kind; a minimum
/// and a maximum share a basis and minimum ≤ maximum. Every refusal is
/// `InvalidBound` at the offending bound (for a pair, the later one).
fn check_bounds(bounds: &[Bound], scale: AmountScale) -> Result<()> {
    let invalid = |i: usize| Error::from(ErrorKind::InvalidBound).at(Location::Bound(i));
    for (i, b) in bounds.iter().enumerate() {
        let a = Amount::<5>::from_decimal(b.amount).map_err(|_| invalid(i))?;
        if !a.is_positive() || !a.fits(scale.decimals) {
            return Err(invalid(i));
        }
    }
    let of = |k: BoundKind| bounds.iter().enumerate().filter(move |(_, b)| b.kind == k);
    for k in [BoundKind::Minimum, BoundKind::Maximum] {
        if let Some((i, _)) = of(k).nth(1) {
            return Err(invalid(i));
        }
    }
    if let (Some((i, min)), Some((j, max))) =
        (of(BoundKind::Minimum).next(), of(BoundKind::Maximum).next())
    {
        if min.basis != max.basis || min.amount > max.amount {
            return Err(invalid(i.max(j)));
        }
    }
    Ok(())
}

struct Inputs<'a> {
    currency: Currency,
    scale: AmountScale,
    payments: &'a [Payment],
    /// Σ drawn after-tax credits, added to BT-113 after the payments.
    after_tax: Amount,
    cash_rounding: Option<CashRounding>,
    vat_accounting: Option<VatAccounting>,
}

/// A before-tax credit's draw as document allowances: one per VAT group, split
/// by the groups' bases with the exact largest-remainder [`crate::split`] that
/// [`Adjust::Fixed`] uses in [`crate::apportion`] (a group with a negative base
/// weighs zero, so no group's base is driven below zero); one allowance without
/// VAT on a VAT-free document. Zero parts are dropped.
fn credit_allowances(
    description: &str,
    draw: Amount,
    groups: &[VatGroup],
    scale: AmountScale,
) -> Result<Vec<DocumentAdjustment>> {
    let total = draw.to_decimal();
    if groups.is_empty() {
        return Ok(alloc::vec![DocumentAdjustment::allowance(
            description,
            total.normalize()
        )]);
    }
    let weights: Vec<Decimal> = groups
        .iter()
        .map(|g| g.taxable_base().max(Amount::ZERO).to_decimal())
        .collect();
    Ok(crate::split(total, &weights, scale.decimals)?
        .into_iter()
        .zip(groups)
        .filter(|(p, _)| !p.is_zero())
        .map(|(p, g)| {
            DocumentAdjustment::allowance(description, p.normalize()).with_vat(g.vat().clone())
        })
        .collect())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Totals {
    line: Amount,
    allowance: Amount,
    charge: Amount,
    tax_exclusive: Amount,
    vat: Amount,
    tax_inclusive: Amount,
    prepaid: Amount,
    rounding: Amount,
    payable: Amount,
    vat_accounting: Option<Amount>,
}

impl Totals {
    fn negated(&self) -> Self {
        Self {
            line: -self.line,
            allowance: -self.allowance,
            charge: -self.charge,
            tax_exclusive: -self.tax_exclusive,
            vat: -self.vat,
            tax_inclusive: -self.tax_inclusive,
            prepaid: -self.prepaid,
            rounding: -self.rounding,
            payable: -self.payable,
            vat_accounting: self.vat_accounting.map(|a| -a),
        }
    }

    fn amounts(&self) -> impl Iterator<Item = Amount> + '_ {
        [
            self.line,
            self.allowance,
            self.charge,
            self.tax_exclusive,
            self.vat,
            self.tax_inclusive,
            self.prepaid,
            self.rounding,
            self.payable,
        ]
        .into_iter()
        .chain(self.vat_accounting)
    }
}

struct Derived {
    nets: Vec<Amount>,
    line_adjustments: Vec<Vec<Amount>>,
    adjustment_amounts: Vec<Amount>,
    groups: Vec<VatGroup>,
    totals: Totals,
}

/// BG-23 order: category code, then rate, ascending.
type Key = (&'static str, Decimal);

fn derive(
    inp: &Inputs<'_>,
    lines: &[Line],
    adjustments: &[DocumentAdjustment],
    partition: Option<&[Amount]>,
) -> Result<Derived> {
    let scale = inp.scale;
    let mut nets = Vec::with_capacity(lines.len());
    let mut line_adjustments = Vec::with_capacity(lines.len());
    // key → (vat, base)
    let mut groups: BTreeMap<Key, (Vat, Amount)> = BTreeMap::new();
    let mut join = |vat: Option<&Vat>, amount: Amount, at: Location| -> Result<()> {
        let Some(vat) = vat else { return Ok(()) };
        let err = |e: Error| e.at(at);
        let key = (vat.category().code(), vat.rate_percent().value());
        match groups.get_mut(&key) {
            Some((v, base)) => {
                if v.exemption() != vat.exemption() {
                    return Err(err(ErrorKind::ConflictingExemption.into()));
                }
                *base = base.checked_add(amount).map_err(err)?;
            }
            None => {
                groups.insert(key, (vat.clone(), amount));
            }
        }
        Ok(())
    };
    let mut line_total = Amount::ZERO;
    for (i, l) in lines.iter().enumerate() {
        let (net, amounts, _) = l.derive(i, scale)?;
        let at = |e: Error| e.at(Location::Line(i));
        line_total = line_total.checked_add(net).map_err(at)?;
        join(l.vat(), net, Location::Line(i))?;
        nets.push(net);
        line_adjustments.push(amounts);
    }
    let mut adjustment_amounts = Vec::with_capacity(adjustments.len());
    let (mut allowance, mut charge) = (Amount::ZERO, Amount::ZERO);
    for (i, a) in adjustments.iter().enumerate() {
        let r = a.reduce(i, scale)?;
        let at = |e: Error| e.at(Location::Adjustment(i));
        match a.kind() {
            crate::AdjustmentKind::Allowance => allowance = allowance.checked_add(r).map_err(at)?,
            crate::AdjustmentKind::Charge => charge = charge.checked_add(r).map_err(at)?,
        }
        join(
            a.vat(),
            a.kind().signed(r).map_err(at)?,
            Location::Adjustment(i),
        )?;
        adjustment_amounts.push(r);
    }
    let has = |c: TaxCategory| groups.keys().any(|k| k.0 == c.code());
    if has(TaxCategory::OutOfScope) && groups.len() > 1 {
        return Err(ErrorKind::InvalidVat(VatError::OutOfScopeMixed).into());
    }
    if has(TaxCategory::SplitPayment) && has(TaxCategory::Standard) {
        return Err(ErrorKind::InvalidVat(VatError::SplitPaymentWithStandard).into());
    }
    if partition.is_some_and(|p| p.len() != groups.len()) {
        return Err(ErrorKind::InvalidTaxPartition.into());
    }
    let mut out_groups = Vec::with_capacity(groups.len());
    let mut vat_total = Amount::ZERO;
    for (k, (vat, base)) in groups.into_values().enumerate() {
        let computed = vat.tax_on(base, scale)?;
        let tax = match partition {
            None => computed,
            Some(p) => {
                // A split share's tax: ⌊t⌋ or ⌈t⌉ of its exact tax t, and the
                // computed tax is one of the two as well, so the two differ by at
                // most one unit and never in sign. Anything else is not a
                // partition this crate produced (BR-CO-17's ±1.00 would accept a
                // tampered 190.99 on 1000.00 at 19 %).
                let t = p[k];
                let unit = Amount::from_decimal(Decimal::new(1, u32::from(scale.decimals)))?;
                let diff = t.checked_sub(computed)?.checked_abs()?;
                let ok = t.fits(scale.decimals)
                    && if vat.rate_percent().value().is_zero() {
                        t.is_zero()
                    } else {
                        diff <= unit
                            && (t.is_zero()
                                || t.signum() == computed.signum()
                                || computed.is_zero())
                    };
                if !ok {
                    return Err(ErrorKind::InvalidTaxPartition.into());
                }
                t
            }
        };
        vat_total = vat_total.checked_add(tax)?;
        out_groups.push(VatGroup::new(vat, base, tax));
    }
    let tax_exclusive = line_total.checked_sub(allowance)?.checked_add(charge)?;
    let tax_inclusive = tax_exclusive.checked_add(vat_total)?;
    let prepaid = crate::settlement::paid_total(inp.payments, scale)?.checked_add(inp.after_tax)?;
    let due = tax_inclusive.checked_sub(prepaid)?;
    let rounding = match &inp.cash_rounding {
        Some(c) => due
            .round_to_increment(c.increment_at(scale)?, c.rounding)?
            .checked_sub(due)?,
        None => Amount::ZERO,
    };
    let payable = due.checked_add(rounding)?;
    let vat_accounting = inp
        .vat_accounting
        .map(|v| v.total(inp.currency, vat_total, scale))
        .transpose()?;
    let totals = Totals {
        line: line_total,
        allowance,
        charge,
        tax_exclusive,
        vat: vat_total,
        tax_inclusive,
        prepaid,
        rounding,
        payable,
        vat_accounting,
    };
    // Nothing may be Amount::MIN: negated() must be total.
    let min = Amount::MIN;
    if totals.amounts().any(|a| a == min)
        || nets.contains(&min)
        || adjustment_amounts.contains(&min)
        || line_adjustments.iter().flatten().any(|a| *a == min)
        || out_groups
            .iter()
            .any(|g| g.taxable_base() == min || g.tax() == min)
    {
        return Err(ErrorKind::Overflow.into());
    }
    Ok(Derived {
        nets,
        line_adjustments,
        adjustment_amounts,
        groups: out_groups,
        totals,
    })
}

fn basis_value(t: &Totals, basis: Basis) -> Amount {
    match basis {
        Basis::Net => t.tax_exclusive,
        Basis::Gross => t.tax_inclusive,
    }
}

/// The bound line's amount `x` (a multiple of the scale's unit), or `None`
/// when the bound already holds. `f(x)` — the basis after adding a fixed line
/// of `x` — is strictly increasing in `x` (each unit adds ≥ one unit: the net
/// grows by it and the group's rounded tax never falls, under every rounding
/// mode), so the extreme `x` is found by stepping from an estimate.
fn settle(inp: &Inputs<'_>, d: &Derived, b: &Bound) -> Result<Option<Decimal>> {
    let scale = inp.scale;
    let target = Amount::from_decimal(b.amount)?;
    let base_value = basis_value(&d.totals, b.basis);
    match b.kind {
        BoundKind::Minimum if base_value >= target => return Ok(None),
        BoundKind::Maximum if base_value <= target => return Ok(None),
        _ => {}
    }
    if b.kind == BoundKind::Minimum && base_value.is_negative() {
        // A floor on a credit would turn it into a debit.
        return Err(ErrorKind::InvalidBound.into());
    }
    let unit = Amount::from_decimal(Decimal::new(1, u32::from(scale.decimals)))?;
    // The group the bound line joins: its current base and tax.
    let (group_base, group_tax) = match (&b.vat, b.basis) {
        (Some(v), Basis::Gross) => d
            .groups
            .iter()
            .find(|g| g.vat().key() == v.key())
            .map_or((Amount::ZERO, Amount::ZERO), |g| {
                (g.taxable_base(), g.tax())
            }),
        _ => (Amount::ZERO, Amount::ZERO),
    };
    let f = |x: Amount| -> Result<Amount> {
        let net = base_value.checked_add(x)?;
        match (&b.vat, b.basis) {
            (Some(v), Basis::Gross) => {
                let tax = v.tax_on(group_base.checked_add(x)?, scale)?;
                net.checked_add(tax)?.checked_sub(group_tax)
            }
            _ => Ok(net),
        }
    };
    // Estimate x = (target − f(0)) / (1 + r/100), to whole units.
    let gap = target.checked_sub(base_value)?.to_decimal();
    let rate = match (&b.vat, b.basis) {
        (Some(v), Basis::Gross) => v.rate_percent().value(),
        _ => Decimal::ZERO,
    };
    let toward_zero = scale.with_rounding(crate::Rounding::TowardZero);
    let est_units = units_exact(
        &[gap, Decimal::ONE_HUNDRED],
        &[Decimal::ONE_HUNDRED
            .checked_add(rate)
            .ok_or(ErrorKind::Overflow)?],
        toward_zero,
    )?;
    let mut x: Amount = toward_zero.reduce(
        Decimal::try_from_i128_with_scale(est_units, u32::from(scale.decimals))
            .map_err(|_| ErrorKind::Overflow)?,
    )?;
    match b.kind {
        BoundKind::Minimum => {
            // Smallest x with f(x) ≥ target.
            while f(x)? < target {
                x = x.checked_add(unit)?;
            }
            while f(x.checked_sub(unit)?)? >= target {
                x = x.checked_sub(unit)?;
            }
        }
        BoundKind::Maximum => {
            // Largest x with f(x) ≤ target.
            while f(x)? > target {
                x = x.checked_sub(unit)?;
            }
            while f(x.checked_add(unit)?)? <= target {
                x = x.checked_add(unit)?;
            }
        }
    }
    Ok((!x.is_zero()).then(|| x.to_decimal().normalize()))
}

/// An EN 16931 document's arithmetic: currency (BT-5), scale, lines (BG-25),
/// document allowances and charges (BG-20/21) and settlement, with every total
/// and the VAT breakdown (BG-23) **derived** once at build.
///
/// Fields are private and there is no stored derivative a caller could edit:
/// a `Document` exists only through [`DocumentBuilder::build`] (serde
/// deserialises the inputs and builds). Every accessor is infallible.
///
/// ```
/// use billing::{AmountScale, Currency, Document, DocumentAdjustment, Line, Percent, Price, Quantity, UnitPrice, Vat};
/// use rust_decimal::dec;
///
/// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
///     .line(Line::new("energy", Quantity::new(dec!(1000), "KWH"), UnitPrice::new(Price::new(dec!(0.30)))))
///     .adjustment(DocumentAdjustment::allowance("loyalty", dec!(10)))
///     .build()?;
/// assert_eq!(doc.tax_exclusive_total().to_string(), "290.00000");
/// assert_eq!(doc.vat_total().to_string(), "55.10000");
/// assert_eq!(doc.payable().to_string(), "345.10000");
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    currency: Currency,
    scale: AmountScale,
    /// Caller lines, then `bound_lines` lines settled from `bounds`.
    lines: Vec<Line>,
    bound_lines: usize,
    bounds: Vec<Bound>,
    /// Caller adjustments, then `apportioned` adjustments materialised from
    /// `apportions`.
    adjustments: Vec<DocumentAdjustment>,
    apportioned: usize,
    apportions: Vec<Apportion>,
    /// Trailing adjustments materialised from before-tax `credits` (after the
    /// apportioned ones).
    credited: usize,
    credits: Vec<Credit>,
    applications: Vec<CreditApplication>,
    /// Σ drawn after-tax credits, part of `totals.prepaid`.
    after_tax: Amount,
    payments: Vec<Payment>,
    cash_rounding: Option<CashRounding>,
    vat_accounting: Option<VatAccounting>,
    tax_partition: Option<Vec<Amount>>,
    // derived
    nets: Vec<Amount>,
    line_adjustments: Vec<Vec<Amount>>,
    adjustment_amounts: Vec<Amount>,
    groups: Vec<VatGroup>,
    totals: Totals,
}

impl Document {
    /// A builder for a document in `currency` at `scale`.
    pub fn builder(currency: Currency, scale: AmountScale) -> DocumentBuilder {
        DocumentBuilder {
            currency,
            scale,
            default_vat: Vec::new(),
            lines: Vec::new(),
            adjustments: Vec::new(),
            apportions: Vec::new(),
            bounds: Vec::new(),
            credits: Vec::new(),
            payments: Vec::new(),
            cash_rounding: Vec::new(),
            vat_accounting: Vec::new(),
            tax_partition: None,
        }
    }

    /// BT-5.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.currency
    }

    /// The scale every derived amount is reduced to.
    #[must_use]
    pub fn scale(&self) -> AmountScale {
        self.scale
    }

    /// Every line (BG-25) as emitted: the caller's lines with the default VAT
    /// filled in, then the lines settled from the bounds.
    #[must_use]
    pub fn lines(&self) -> &[Line] {
        &self.lines
    }

    /// The bounds (default VAT filled in). Their settled lines are the last
    /// [`bound_lines`](Self::bound_lines) of [`lines`](Self::lines).
    #[must_use]
    pub fn bounds(&self) -> &[Bound] {
        &self.bounds
    }

    /// How many trailing lines were settled from the bounds.
    #[must_use]
    pub fn bound_lines(&self) -> usize {
        self.bound_lines
    }

    /// Every document allowance and charge (BG-20/21) as emitted: the caller's,
    /// with the default VAT filled in, then those materialised from
    /// [`DocumentBuilder::apportion`].
    #[must_use]
    pub fn adjustments(&self) -> &[DocumentAdjustment] {
        &self.adjustments
    }

    /// How many trailing [`adjustments`](Self::adjustments) were materialised
    /// from [`DocumentBuilder::apportion`] inputs.
    #[must_use]
    pub fn apportioned_adjustments(&self) -> usize {
        self.apportioned
    }

    /// The credits, in priority (builder call) order.
    #[must_use]
    pub fn credits(&self) -> &[Credit] {
        &self.credits
    }

    /// What each credit drew from this document, in priority order: applied
    /// and remaining, at the document's scale.
    #[must_use]
    pub fn credit_applications(&self) -> &[CreditApplication] {
        &self.applications
    }

    /// How many trailing [`adjustments`](Self::adjustments) were materialised
    /// from before-tax credits (they follow the apportioned ones).
    #[must_use]
    pub fn credit_adjustments(&self) -> usize {
        self.credited
    }

    /// The residual VAT statement of a final invoice (§14 Abs. 5 S. 2 UStG):
    /// per VAT group of [`vat_groups`](Self::vat_groups), in that order, the
    /// base minus the nets and the tax minus the taxes of the
    /// [itemised advances](Payment::Advance) with the same (category,
    /// rate); gross payments carry no VAT and do not enter it. Without
    /// itemised advances it equals the VAT groups. BG-23 itself still covers
    /// the whole supply.
    ///
    /// ```
    /// use billing::{Advance, AmountScale, Currency, Document, Line, Payment, Percent, Vat};
    /// use rust_decimal::dec;
    ///
    /// let v19 = Vat::standard(Percent::new(dec!(19)))?;
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    ///     .default_vat(v19.clone())
    ///     .line(Line::fixed("works", dec!(1000)))
    ///     .payment(Payment::Advance(Advance { vat: v19, net: dec!(300), tax: dec!(57) }))
    ///     .build()?;
    /// let residual = doc.residual_vat()?;
    /// assert_eq!(residual[0].taxable_base().to_string(), "700.00000");
    /// assert_eq!(residual[0].tax().to_string(), "133.00000");
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`ErrorKind::UnmatchedAdvance`] at [`Location::Payment`] for an advance
    /// whose (category, rate) is no group of the document;
    /// [`ErrorKind::Overflow`].
    pub fn residual_vat(&self) -> Result<Vec<VatGroup>> {
        crate::settlement::residual_vat(&self.groups, &self.payments)
    }

    /// The payments given to the builder, in order (inputs; drawn after-tax
    /// credits are not among them, see [`paid_entries`](Self::paid_entries)).
    #[must_use]
    pub fn payments(&self) -> &[Payment] {
        &self.payments
    }

    /// Everything BT-113 sums: the [`payments`](Self::payments), then each
    /// drawn after-tax credit as a [`Payment::Gross`] of its description and
    /// applied amount, in priority order. Σ gross = [`prepaid`](Self::prepaid).
    /// These are the entries [`negated`](Self::negated), [`split`](Self::split)
    /// and [`concat`](Self::concat) carry over.
    #[must_use]
    pub fn paid_entries(&self) -> Vec<Payment> {
        let drawn = self
            .credits
            .iter()
            .zip(&self.applications)
            .filter(|(c, a)| c.placement == CreditPlacement::AfterTax && !a.applied.is_zero())
            .map(|(c, a)| Payment::Gross {
                description: c.description.clone(),
                amount: a.applied.to_decimal().normalize(),
            });
        self.payments.iter().cloned().chain(drawn).collect()
    }

    /// The cash rounding rule, if any.
    #[must_use]
    pub fn cash_rounding(&self) -> Option<&CashRounding> {
        self.cash_rounding.as_ref()
    }

    /// The VAT accounting currency and rate, if any.
    #[must_use]
    pub fn vat_accounting(&self) -> Option<&VatAccounting> {
        self.vat_accounting.as_ref()
    }

    /// The partitioned group taxes of a [`split`](Self::split) share, if this
    /// is one.
    #[must_use]
    pub fn tax_partition(&self) -> Option<&[Amount]> {
        self.tax_partition.as_deref()
    }

    /// BT-131 of line `i`.
    ///
    /// # Panics
    /// If `i` is out of range.
    #[must_use]
    pub fn line_net(&self, i: usize) -> Amount {
        self.nets[i]
    }

    /// The reduced amounts (BT-136/141) of line `i`'s adjustments.
    ///
    /// # Panics
    /// If `i` is out of range.
    #[must_use]
    pub fn line_adjustment_amounts(&self, i: usize) -> &[Amount] {
        &self.line_adjustments[i]
    }

    /// The reduced amount (BT-92/99) of document adjustment `i`.
    ///
    /// # Panics
    /// If `i` is out of range.
    #[must_use]
    pub fn adjustment_amount(&self, i: usize) -> Amount {
        self.adjustment_amounts[i]
    }

    /// BT-106 = Σ BT-131.
    #[must_use]
    pub fn line_total(&self) -> Amount {
        self.totals.line
    }

    /// BT-107 = Σ BT-92.
    #[must_use]
    pub fn allowance_total(&self) -> Amount {
        self.totals.allowance
    }

    /// BT-108 = Σ BT-99.
    #[must_use]
    pub fn charge_total(&self) -> Amount {
        self.totals.charge
    }

    /// BT-109 = BT-106 − BT-107 + BT-108.
    #[must_use]
    pub fn tax_exclusive_total(&self) -> Amount {
        self.totals.tax_exclusive
    }

    /// BG-23, ordered by category code, then rate. Empty for a VAT-free
    /// document.
    #[must_use]
    pub fn vat_groups(&self) -> &[VatGroup] {
        &self.groups
    }

    /// BT-110 = Σ BT-117 (0 for a VAT-free document).
    #[must_use]
    pub fn vat_total(&self) -> Amount {
        self.totals.vat
    }

    /// BT-112 = BT-109 + BT-110.
    #[must_use]
    pub fn tax_inclusive_total(&self) -> Amount {
        self.totals.tax_inclusive
    }

    /// BT-113.
    #[must_use]
    pub fn prepaid(&self) -> Amount {
        self.totals.prepaid
    }

    /// BT-114.
    #[must_use]
    pub fn rounding(&self) -> Amount {
        self.totals.rounding
    }

    /// BT-115 = BT-112 − BT-113 + BT-114; negative is a credit balance.
    #[must_use]
    pub fn payable(&self) -> Amount {
        self.totals.payable
    }

    /// BT-6 and BT-111, if VAT accounting is set.
    #[must_use]
    pub fn vat_accounting_total(&self) -> Option<(Currency, Amount)> {
        self.vat_accounting
            .zip(self.totals.vat_accounting)
            .map(|(v, a)| (v.currency, a))
    }

    /// Indices of lines with a stated net whose BT-131 misses
    /// PEPPOL-EN16931-R120: `|BT-131 − (BT-129 × BT-146 / BT-149 + Σ charges −
    /// Σ allowances)| > 0.02`.
    ///
    /// Lines without a stated net are not inspected. Their BT-131 is the price
    /// part reduced once plus the reduced adjustments, so it is off by less than
    /// one unit of the scale (at most half a unit under the half modes). That
    /// holds R120's 0.02 at scale 2 and finer only: at scale 0 or 1 an
    /// engine-derived line can miss R120 and is **not** reported here.
    #[must_use]
    pub fn price_inconsistent(&self) -> Vec<usize> {
        self.lines
            .iter()
            .enumerate()
            .filter(|(i, l)| {
                if l.stated_net().is_none() {
                    return false;
                }
                let p = l.price();
                let expected = l
                    .quantity()
                    .value()
                    .checked_mul(p.price().value())
                    .and_then(|x| x.checked_div(p.base_quantity()));
                let Some(mut expected) = expected else {
                    return true;
                };
                for (a, r) in l.adjustments().iter().zip(&self.line_adjustments[*i]) {
                    let r = r.to_decimal();
                    expected = match a.kind() {
                        crate::AdjustmentKind::Charge => expected + r,
                        crate::AdjustmentKind::Allowance => expected - r,
                    };
                }
                (self.nets[*i].to_decimal() - expected).abs() > SLACK
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// The exact negative document: every quantity, stated net, adjustment
    /// amount and basis base, paid entry and partitioned tax negated. Every
    /// derived amount negates with it (all roundings are sign-symmetric).
    /// Bound lines, apportioned and credit adjustments become ordinary ones and
    /// drawn after-tax credits gross payments, so `negated().negated() == self`
    /// only for a document without bounds, apportion inputs or credits. Whether
    /// the result is a negative invoice (380) or a credit note (381) is the
    /// format's decision.
    #[must_use]
    pub fn negated(&self) -> Document {
        Document {
            currency: self.currency,
            scale: self.scale,
            lines: self.lines.iter().map(Line::negated).collect(),
            bound_lines: 0,
            bounds: Vec::new(),
            adjustments: self
                .adjustments
                .iter()
                .map(DocumentAdjustment::negated)
                .collect(),
            apportioned: 0,
            apportions: Vec::new(),
            credited: 0,
            credits: Vec::new(),
            applications: Vec::new(),
            after_tax: Amount::ZERO,
            payments: self.paid_entries().iter().map(Payment::negated).collect(),
            cash_rounding: self.cash_rounding,
            vat_accounting: self.vat_accounting,
            tax_partition: self
                .tax_partition
                .as_ref()
                .map(|p| p.iter().map(|a| -*a).collect()),
            nets: self.nets.iter().map(|a| -*a).collect(),
            line_adjustments: self
                .line_adjustments
                .iter()
                .map(|v| v.iter().map(|a| -*a).collect())
                .collect(),
            adjustment_amounts: self.adjustment_amounts.iter().map(|a| -*a).collect(),
            groups: self.groups.iter().map(VatGroup::negated).collect(),
            totals: self.totals.negated(),
        }
    }

    /// One document of all the inputs of `docs`, re-derived: VAT is rounded
    /// once per group over the union, exactly as building the concatenated
    /// inputs. Lines, adjustments and [paid entries](Self::paid_entries) are
    /// concatenated in order, none merged or dropped; bound lines, apportioned
    /// and credit adjustments become ordinary ones (bounds, apportion inputs
    /// and credits are not combined).
    ///
    /// # Errors
    /// [`ErrorKind::Empty`] for no documents; [`ErrorKind::CurrencyMismatch`],
    /// [`ErrorKind::ScaleMismatch`] or [`ErrorKind::SettingMismatch`] (cash
    /// rounding, VAT accounting) if the documents differ; as
    /// [`DocumentBuilder::build`].
    pub fn concat(docs: impl IntoIterator<Item = Document>) -> Result<Document> {
        let mut iter = docs.into_iter();
        let first = iter.next().ok_or(ErrorKind::Empty)?;
        let mut b = first.clone().materialised();
        for d in iter {
            if d.currency != first.currency {
                return Err(ErrorKind::CurrencyMismatch {
                    expected: first.currency,
                    found: d.currency,
                }
                .into());
            }
            if d.scale != first.scale {
                return Err(ErrorKind::ScaleMismatch.into());
            }
            if d.cash_rounding != first.cash_rounding {
                return Err(ErrorKind::SettingMismatch {
                    setting: "cash_rounding",
                }
                .into());
            }
            if d.vat_accounting != first.vat_accounting {
                return Err(ErrorKind::SettingMismatch {
                    setting: "vat_accounting",
                }
                .into());
            }
            b.payments.extend(d.paid_entries());
            b.lines.extend(d.lines);
            b.adjustments.extend(d.adjustments);
        }
        b.build()
    }

    /// Split the document into shares by non-negative `weights`.
    ///
    /// Every leaf (each line's reduced price part and adjustment amounts, each
    /// document adjustment, each paid entry's amounts) is split at the
    /// document's scale in one [`split_matrix`], so the shares' leaves sum to
    /// the parent's exactly. Share lines state their net; quantities are split
    /// exactly at max(12, their own) decimals; adjustment bases are dropped (a
    /// split amount no longer meets R040); bound lines, apportioned and credit
    /// adjustments become ordinary ones. Each VAT group's tax is
    /// **partitioned** across the shares (largest remainder over each share's
    /// exact `base × rate`), so every share's tax is the floor or ceiling of
    /// its own exact tax and the shares sum to the parent.
    ///
    /// Conserved exactly: every leaf, BT-106 … BT-110, BT-112, BT-113 (per
    /// paid entry) and every group's BT-116/BT-117 — so the shares' residual
    /// VAT statements sum to the parent's. Re-derived per share, so their sum may
    /// differ from the parent's: BT-114 and BT-115 under cash rounding, and
    /// BT-111.
    ///
    /// ```
    /// use billing::{AmountScale, Currency, Document, Line, Percent, Vat};
    /// use rust_decimal::dec;
    ///
    /// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
    ///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
    ///     .line(Line::fixed("water", dec!(47.10)))
    ///     .build()?;
    /// let shares = doc.split(&[dec!(40), dec!(35), dec!(25)])?;
    /// let net: Vec<String> = shares.iter().map(|s| s.tax_exclusive_total().to_string()).collect();
    /// assert_eq!(net, ["18.84000", "16.49000", "11.77000"]);
    /// let gross = shares.iter().map(|s| s.tax_inclusive_total()).sum::<billing::Amount>();
    /// assert_eq!(gross, doc.tax_inclusive_total()); // 56.05
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// As [`split_matrix`] (invalid weights, overflow).
    pub fn split(&self, weights: &[Decimal]) -> Result<Vec<Document>> {
        let scale = self.scale;
        let mut rows: Vec<Decimal> = Vec::new();
        let mut line_rows = Vec::with_capacity(self.lines.len());
        for (i, l) in self.lines.iter().enumerate() {
            let leaves = l.leaves(scale).map_err(|e| e.at(Location::Line(i)))?;
            line_rows.push((rows.len(), leaves.len()));
            rows.extend(leaves);
        }
        let adj_start = rows.len();
        rows.extend(self.adjustment_amounts.iter().map(|a| a.to_decimal()));
        let paid = self.paid_entries();
        let pay_start = rows.len();
        let (pay_rows, pay_spans) = crate::settlement::payment_leaves(&paid);
        rows.extend(pay_rows);
        let n = weights.len();
        let parts = split_matrix(&rows, weights, scale.decimals)?;
        let quantities = self
            .lines
            .iter()
            .enumerate()
            .map(|(i, l)| {
                l.split_quantity(weights)
                    .map_err(|e| e.at(Location::Line(i)))
            })
            .collect::<Result<Vec<_>>>()?;
        let column = |j: usize, (start, len): (usize, usize)| -> Vec<Decimal> {
            parts[start..start + len].iter().map(|r| r[j]).collect()
        };
        let mut builders = Vec::with_capacity(n);
        for j in 0..n {
            let mut b = Document::builder(self.currency, scale);
            b.lines = self
                .lines
                .iter()
                .zip(&line_rows)
                .zip(&quantities)
                .map(|((l, &r), q)| l.share(q[j], &column(j, r)))
                .collect();
            b.adjustments = self
                .adjustments
                .iter()
                .enumerate()
                .map(|(k, a)| a.share(parts[adj_start + k][j]))
                .collect();
            b.payments =
                crate::settlement::payments_share(&paid, &pay_spans, |k| parts[pay_start + k][j]);
            b.cash_rounding = self.cash_rounding.into_iter().collect();
            b.vat_accounting = self.vat_accounting.into_iter().collect();
            builders.push(b);
        }
        let plain = builders
            .iter()
            .map(|b| b.clone().build())
            .collect::<Result<Vec<_>>>()?;
        if self.groups.is_empty() {
            return Ok(plain);
        }
        // Partition each group's tax over the shares' exact taxes.
        let mut partitions = alloc::vec![Vec::with_capacity(self.groups.len()); n];
        for (k, g) in self.groups.iter().enumerate() {
            let bases: Vec<Amount> = plain
                .iter()
                .map(|s| {
                    s.groups
                        .get(k)
                        .filter(|sg| sg.vat().key() == g.vat().key())
                        .map(VatGroup::taxable_base)
                        .ok_or_else(|| Error::from(ErrorKind::InvalidTaxPartition))
                })
                .collect::<Result<_>>()?;
            for (j, t) in partition_tax(g, &bases, scale)?.into_iter().enumerate() {
                partitions[j].push(t);
            }
        }
        builders
            .into_iter()
            .zip(partitions)
            .map(|(b, p)| b.tax_partition(p).build())
            .collect()
    }

    /// Back to a builder with the inputs: the caller's lines and adjustments,
    /// the bounds, [`apportion`](DocumentBuilder::apportion) inputs and credits
    /// (all re-applied on the next build), and settlement. The default VAT is
    /// already written onto every leaf; a split share's tax partition is
    /// dropped, so the next build re-derives VAT.
    pub fn into_builder(self) -> DocumentBuilder {
        let mut b = Document::builder(self.currency, self.scale);
        let mut lines = self.lines;
        lines.truncate(lines.len() - self.bound_lines);
        b.lines = lines;
        b.bounds = self.bounds;
        let mut adjustments = self.adjustments;
        adjustments.truncate(adjustments.len() - self.apportioned - self.credited);
        b.adjustments = adjustments;
        b.apportions = self.apportions;
        b.credits = self.credits;
        b.payments = self.payments;
        b.cash_rounding = self.cash_rounding.into_iter().collect();
        b.vat_accounting = self.vat_accounting.into_iter().collect();
        b
    }

    /// A builder with every line and adjustment (bound lines, apportioned and
    /// credit adjustments as ordinary ones), no bounds, no apportion inputs
    /// and no credits (drawn after-tax credits as gross payments).
    fn materialised(self) -> DocumentBuilder {
        let mut b = Document::builder(self.currency, self.scale);
        b.payments = self.paid_entries();
        b.lines = self.lines;
        b.adjustments = self.adjustments;
        b.cash_rounding = self.cash_rounding.into_iter().collect();
        b.vat_accounting = self.vat_accounting.into_iter().collect();
        b
    }
}

/// Partition group `g`'s tax `T` over shares with bases `bases` (Σ = g's
/// base): targets `tⱼ = baseⱼ × rate / 100` in units; each share gets ⌊tⱼ⌋ and
/// the `T − Σ⌊tⱼ⌋` extra units go to the largest fractional parts (ties: lowest
/// index). Since |T − Σtⱼ| < 1 unit, every share's tax is ⌊tⱼ⌋ or ⌈tⱼ⌉.
fn partition_tax(g: &VatGroup, bases: &[Amount], scale: AmountScale) -> Result<Vec<Amount>> {
    let rate = g.rate_percent().value().normalize();
    let den = 100i128
        .checked_mul(crate::decimal::pow10(rate.scale()).ok_or(ErrorKind::Overflow)?)
        .ok_or(ErrorKind::Overflow)?;
    let unit = crate::decimal::pow10(u32::from(5 - scale.decimals)).ok_or(ErrorKind::Overflow)?;
    let mut floors = Vec::with_capacity(bases.len());
    let mut rems = Vec::with_capacity(bases.len());
    for b in bases {
        let units = i128::from(b.to_raw()) / unit;
        let num = units
            .checked_mul(rate.mantissa())
            .ok_or(ErrorKind::Overflow)?;
        floors.push(num.div_euclid(den));
        rems.push(num.rem_euclid(den));
    }
    let total = i128::from(g.tax().to_raw()) / unit;
    let extra = total - floors.iter().sum::<i128>();
    let extra = usize::try_from(extra).map_err(|_| ErrorKind::InvalidTaxPartition)?;
    if extra > bases.len() {
        return Err(ErrorKind::InvalidTaxPartition.into());
    }
    let mut order: Vec<usize> = (0..bases.len()).collect();
    order.sort_by(|&a, &b| rems[b].cmp(&rems[a]).then(a.cmp(&b)));
    for &i in order.iter().take(extra) {
        floors[i] += 1;
    }
    floors
        .into_iter()
        .map(|u| {
            let raw = u.checked_mul(unit).ok_or(ErrorKind::Overflow)?;
            Ok(Amount::from_raw(
                i64::try_from(raw).map_err(|_| ErrorKind::Overflow)?,
            ))
        })
        .collect()
}

#[cfg(feature = "serde")]
mod serde_impl {
    use super::*;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Settlement {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        payments: Vec<Payment>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cash_rounding: Option<CashRounding>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        vat_accounting: Option<VatAccounting>,
    }

    #[derive(Serialize, Deserialize)]
    struct Repr {
        currency: Currency,
        scale: AmountScale,
        lines: Vec<Line>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        bounds: Vec<Bound>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        adjustments: Vec<DocumentAdjustment>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        apportion: Vec<Apportion>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        credits: Vec<Credit>,
        settlement: Settlement,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tax_partition: Option<Vec<Amount>>,
    }

    /// The inputs only (bounds as bounds, not their lines; apportion inputs,
    /// not their adjustments); derived values are never serialised.
    impl Serialize for Document {
        fn serialize<S: Serializer>(&self, s: S) -> core::result::Result<S::Ok, S::Error> {
            Repr {
                currency: self.currency,
                scale: self.scale,
                lines: self.lines[..self.lines.len() - self.bound_lines].to_vec(),
                bounds: self.bounds.clone(),
                adjustments: self.adjustments
                    [..self.adjustments.len() - self.apportioned - self.credited]
                    .to_vec(),
                apportion: self.apportions.clone(),
                credits: self.credits.clone(),
                settlement: Settlement {
                    payments: self.payments.clone(),
                    cash_rounding: self.cash_rounding,
                    vat_accounting: self.vat_accounting,
                },
                tax_partition: self.tax_partition.clone(),
            }
            .serialize(s)
        }
    }

    /// Deserialises the inputs and builds (re-deriving and re-validating
    /// everything, the bounds, the apportion inputs and the tax partition
    /// included).
    impl<'de> Deserialize<'de> for Document {
        fn deserialize<D: Deserializer<'de>>(d: D) -> core::result::Result<Self, D::Error> {
            let r = Repr::deserialize(d)?;
            let mut b = Document::builder(r.currency, r.scale)
                .lines(r.lines)
                .adjustments(r.adjustments);
            b.bounds = r.bounds;
            b.apportions = r.apportion;
            b.credits = r.credits;
            b.payments = r.settlement.payments;
            b.cash_rounding = r.settlement.cash_rounding.into_iter().collect();
            b.vat_accounting = r.settlement.vat_accounting.into_iter().collect();
            b.tax_partition = r.tax_partition;
            b.build().map_err(serde::de::Error::custom)
        }
    }
}
