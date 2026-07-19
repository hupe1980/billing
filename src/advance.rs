//! [`AdvancePayment`] and [`DocumentKind`] — prepayments and the final invoice
//! that settles them.
//!
//! # The problem this solves
//!
//! Bill a customer in instalments and you eventually issue a document that
//! settles the lot. Two things must then be true at once:
//!
//! - the **taxable base covers the whole supply**, because that is what was
//!   supplied and what output tax is owed on; and
//! - the **amount payable is only the remainder**, because the rest is already in
//!   the bank.
//!
//! [`crate::BillingDocument::with_prepaid`] already expresses the second part —
//! it is EN 16931's BT-113, a single flat figure. What BT-113 cannot express is
//! the **tax contained in each advance**, and several jurisdictions require
//! exactly that on the settling document. Germany is the sharpest case: §14
//! Abs. 5 Satz 2 UStG requires a final invoice to deduct the advance amounts
//! *"und die auf sie entfallenden Steuerbeträge"* — and their tax. Omit it and,
//! per UStAE 14.8 Abs. 10, the issuer owes the full tax shown **plus** the
//! advance-related portion again under §14c Abs. 1: the same tax, billed twice.
//!
//! [`AdvancePayment`] carries that missing structure. It mirrors the ZUGFeRD /
//! Factur-X EXTENDED group `SpecifiedAdvancePayment` (BG-X-45), the one
//! standardised place where per-advance tax data has a home:
//!
//! | This crate | ZUGFeRD EXTENDED | Meaning |
//! |------------|------------------|---------|
//! | [`AdvancePayment::gross`] | BT-X-291 | amount received |
//! | [`AdvancePayment::received_on`] | BT-X-292 | date of receipt |
//! | [`AdvancePayment::tax`] | BG-X-46 | tax contained, per category and rate |
//! | [`AdvancePayment::reference`] | BT-X-558 / BT-25 | the advance invoice's number |
//! | [`AdvancePayment::reference_date`] | BT-X-560 / BT-26 | its issue date |
//!
//! # Two lawful shapes, and the engine supports both
//!
//! **Settle by deduction.** Invoice the full supply and deduct the advances with
//! their tax. Attach them with
//! [`BillingDocument::with_advances`](crate::BillingDocument::with_advances):
//! totals and the VAT breakdown keep describing the whole supply, and only
//! [`amount_due`](crate::BillingDocument::amount_due) shrinks.
//!
//! **Settle by residual.** Invoice only what is left and do not list the advances
//! at all. Compute the remainder with [`residual_breakdown`], then bill that.
//!
//! The residual form is structurally simpler and is what the German BMF
//! recommends for e-invoices (Schreiben v. 15.10.2024, Rn. 48), because EN 16931
//! has nowhere to put the per-advance tax in its core profiles. The engine takes
//! no position on which you use — it just refuses to let you get either wrong.
//!
//! # This module is jurisdiction-neutral
//!
//! Nothing here is specific to Germany or to any industry. Progress billing in
//! construction, deposits in retail, instalment plans, and metered utilities all
//! produce the same shape. The German rules are cited because they are the
//! strictest published statement of the requirement, not because the code
//! implements them.

use crate::amount::Amount;
use crate::error::BillingError;
use crate::vat::TaxBreakdownEntry;

// ── DocumentKind ──────────────────────────────────────────────────────────────

/// UNTDID 1001 document type code — EN 16931 **BT-3**.
///
/// The subset here is the one both XRechnung (rule BR-DE-17) and Peppol BIS
/// Billing 3.0 accept, plus the prepayment code Peppol permits.
///
/// # A final invoice has no code of its own
///
/// UNTDID has no "final invoice" or "residual invoice" code outside
/// construction: both are [`DocumentKind::CommercialInvoice`]. What distinguishes
/// them is the *amounts* — whether advances are deducted — not the type code. Use
/// [`BillingDocument::advances`](crate::BillingDocument::advances) to tell them
/// apart, and the construction codes when the project is a construction project.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub enum DocumentKind {
    /// `380` — commercial invoice. Also the correct code for a final invoice that
    /// deducts advances, and for a residual invoice.
    #[default]
    CommercialInvoice,
    /// `326` — partial invoice: an incomplete billing of a delivered service.
    PartialInvoice,
    /// `386` — prepayment invoice: billed before delivery, to be deducted later.
    ///
    /// Semantically the best fit for an advance-payment request, but **outside
    /// XRechnung's recommended set** (BR-DE-17), which offers no prepayment code;
    /// German practice uses `380` or `326` instead. Peppol accepts `386`.
    PrepaymentInvoice,
    /// `384` — corrected invoice.
    ///
    /// Peppol restricts this (and `326`) to domestic German exchanges via rule
    /// `PEPPOL-EN16931-P0112`; a cross-border invoice using it is rejected.
    CorrectedInvoice,
    /// `381` — credit note. See [`BillingDocument::reverse`](crate::BillingDocument::reverse).
    CreditNote,
    /// `383` — debit note.
    DebitNote,
    /// `389` — self-billed invoice, issued by the buyer.
    SelfBilledInvoice,
    /// `875` — partial construction invoice (Abschlagsrechnung, VOB/B §16).
    PartialConstructionInvoice,
    /// `876` — partial final construction invoice (Teilschlussrechnung).
    PartialFinalConstructionInvoice,
    /// `877` — final construction invoice (Schlussrechnung).
    FinalConstructionInvoice,
}

impl DocumentKind {
    /// The numeric UNTDID 1001 code.
    ///
    /// ```rust
    /// use billing::DocumentKind;
    /// assert_eq!(DocumentKind::CommercialInvoice.code(), 380);
    /// assert_eq!(DocumentKind::CreditNote.code(), 381);
    /// assert_eq!(DocumentKind::FinalConstructionInvoice.code(), 877);
    /// ```
    #[must_use]
    pub fn code(&self) -> u16 {
        match self {
            Self::CommercialInvoice => 380,
            Self::PartialInvoice => 326,
            Self::PrepaymentInvoice => 386,
            Self::CorrectedInvoice => 384,
            Self::CreditNote => 381,
            Self::DebitNote => 383,
            Self::SelfBilledInvoice => 389,
            Self::PartialConstructionInvoice => 875,
            Self::PartialFinalConstructionInvoice => 876,
            Self::FinalConstructionInvoice => 877,
        }
    }

    /// Parse a UNTDID 1001 code. Returns `None` for codes outside the supported set.
    ///
    /// ```rust
    /// use billing::DocumentKind;
    /// assert_eq!(DocumentKind::from_code(380), Some(DocumentKind::CommercialInvoice));
    /// assert_eq!(DocumentKind::from_code(999), None);
    /// ```
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            380 => Self::CommercialInvoice,
            326 => Self::PartialInvoice,
            386 => Self::PrepaymentInvoice,
            384 => Self::CorrectedInvoice,
            381 => Self::CreditNote,
            383 => Self::DebitNote,
            389 => Self::SelfBilledInvoice,
            875 => Self::PartialConstructionInvoice,
            876 => Self::PartialFinalConstructionInvoice,
            877 => Self::FinalConstructionInvoice,
            _ => return None,
        })
    }

    /// Whether this kind represents a credit rather than a charge.
    #[must_use]
    pub fn is_credit_note(&self) -> bool {
        matches!(self, Self::CreditNote)
    }
}

impl std::fmt::Display for DocumentKind {
    /// Renders the numeric UNTDID code, honouring width, fill and alignment.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(&self.code().to_string())
    }
}

// ── AdvancePayment ────────────────────────────────────────────────────────────

/// One previously invoiced and received advance payment, with the tax it contains.
///
/// See the [module documentation](self) for why the per-rate tax matters and how
/// this maps to ZUGFeRD's BG-X-45.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "AdvancePaymentRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdvancePayment {
    reference: Option<String>,
    reference_date: Option<String>,
    received_on: Option<String>,
    tax: Vec<TaxBreakdownEntry>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct AdvancePaymentRepr {
    #[serde(default)]
    reference: Option<String>,
    #[serde(default)]
    reference_date: Option<String>,
    #[serde(default)]
    received_on: Option<String>,
    tax: Vec<TaxBreakdownEntry>,
}

#[cfg(feature = "serde")]
impl TryFrom<AdvancePaymentRepr> for AdvancePayment {
    type Error = BillingError;
    fn try_from(r: AdvancePaymentRepr) -> Result<Self, Self::Error> {
        let mut a = Self::new(r.tax)?;
        a.reference = r.reference;
        a.reference_date = r.reference_date;
        a.received_on = r.received_on;
        Ok(a)
    }
}

impl AdvancePayment {
    /// Create an advance payment from its per-`(category, rate)` tax breakdown.
    ///
    /// `tax` describes what the advance invoice charged: one entry per VAT
    /// category and rate, each carrying the net taxable base and the tax on it.
    /// The gross received is derived as `Σ(base + tax)`, so it cannot drift from
    /// the components.
    ///
    /// # Errors
    /// - [`BillingError::InvalidInput`] if `tax` is empty. ZUGFeRD makes the
    ///   equivalent group (BG-X-46) mandatory `1..n` for the same reason: an
    ///   advance with no stated tax cannot be deducted lawfully.
    /// - [`BillingError::InvalidInput`] if any base or tax amount is negative — a
    ///   payment already received cannot be less than nothing.
    /// - [`BillingError::InvalidInput`] if two entries share a `(category, rate)`
    ///   group, or if any entry fails its EN 16931 category rules.
    ///
    /// ```rust
    /// use billing::{AdvancePayment, Amount, TaxBreakdownEntry, TaxCategory};
    /// use rust_decimal::dec;
    ///
    /// // An advance of 300.00 net + 57.00 VAT at 19%.
    /// let advance = AdvancePayment::new(vec![TaxBreakdownEntry::new(
    ///     TaxCategory::Standard,
    ///     dec!(0.19),
    ///     Amount::parse("300.00000")?,
    ///     Amount::parse("57.00000")?,
    /// )])?;
    ///
    /// assert_eq!(advance.net(),       Amount::parse("300.00000")?);
    /// assert_eq!(advance.tax_total(), Amount::parse("57.00000")?);
    /// assert_eq!(advance.gross(),     Amount::parse("357.00000")?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn new(tax: Vec<TaxBreakdownEntry>) -> Result<Self, BillingError> {
        if tax.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "AdvancePayment requires at least one tax breakdown entry: \
                         an advance whose tax is unstated cannot be deducted"
                    .into(),
            });
        }
        let mut seen: Vec<_> = Vec::with_capacity(tax.len());
        for entry in &tax {
            entry.validate()?;
            // A negative advance passes every other check — BR-CO-17 holds for
            // (-100, 0.19, -19) and the category rules say nothing about sign — yet
            // it means "a payment already received of less than nothing". Left
            // unchecked it produced a negative BT-113, an `amount_due` LARGER than
            // the gross, and a document that failed its own `validate()`.
            if entry.taxable_base.is_negative() || entry.tax_amount.is_negative() {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "AdvancePayment amounts must be >= 0, got base {} and tax {}",
                        entry.taxable_base, entry.tax_amount
                    ),
                });
            }
            let key = entry.group_key();
            if seen.contains(&key) {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "AdvancePayment has two entries for group ({}, {})",
                        key.0, key.1
                    ),
                });
            }
            seen.push(key);
        }
        Ok(Self {
            reference: None,
            reference_date: None,
            received_on: None,
            tax,
        })
    }

    /// Set the advance invoice's number — EN 16931 **BT-25** / ZUGFeRD BT-X-558.
    ///
    /// Worth setting: §14 Abs. 5 Satz 2 UStG only obliges deduction where invoices
    /// *were issued* for the advances, so the reference is what evidences the
    /// obligation.
    #[must_use]
    pub fn with_reference(mut self, reference: impl Into<String>) -> Self {
        self.reference = Some(reference.into());
        self
    }

    /// Set the advance invoice's issue date — **BT-26** / BT-X-560.
    #[must_use]
    pub fn with_reference_date(mut self, date: impl Into<String>) -> Self {
        self.reference_date = Some(date.into());
        self
    }

    /// Set the date the payment was received — ZUGFeRD **BT-X-292**.
    #[must_use]
    pub fn with_received_on(mut self, date: impl Into<String>) -> Self {
        self.received_on = Some(date.into());
        self
    }

    /// The advance invoice's number, if set.
    #[must_use]
    pub fn reference(&self) -> Option<&str> {
        self.reference.as_deref()
    }

    /// The advance invoice's issue date, if set.
    #[must_use]
    pub fn reference_date(&self) -> Option<&str> {
        self.reference_date.as_deref()
    }

    /// The date the payment was received, if set.
    #[must_use]
    pub fn received_on(&self) -> Option<&str> {
        self.received_on.as_deref()
    }

    /// The per-`(category, rate)` tax breakdown of this advance.
    #[must_use]
    pub fn tax(&self) -> &[TaxBreakdownEntry] {
        &self.tax
    }

    /// Net amount of the advance — `Σ` of the taxable bases.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn checked_net(&self) -> Result<Amount<5>, BillingError> {
        Amount::checked_sum(self.tax.iter().map(|e| e.taxable_base))
    }

    /// Tax contained in the advance — `Σ` of the tax amounts. This is the figure
    /// §14 Abs. 5 Satz 2 UStG requires a final invoice to state.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn checked_tax_total(&self) -> Result<Amount<5>, BillingError> {
        Amount::checked_sum(self.tax.iter().map(|e| e.tax_amount))
    }

    /// Gross amount received — ZUGFeRD **BT-X-291**, equal to `net + tax`.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn checked_gross(&self) -> Result<Amount<5>, BillingError> {
        self.checked_net()?.checked_add(self.checked_tax_total()?)
    }

    /// Net amount of the advance.
    ///
    /// # Panics
    /// Panics on overflow. Use [`AdvancePayment::checked_net`] when the entries
    /// come from untrusted input.
    pub fn net(&self) -> Amount<5> {
        self.checked_net().expect("advance net overflowed")
    }

    /// Tax contained in the advance.
    ///
    /// # Panics
    /// Panics on overflow. See [`AdvancePayment::checked_tax_total`].
    pub fn tax_total(&self) -> Amount<5> {
        self.checked_tax_total()
            .expect("advance tax total overflowed")
    }

    /// Gross amount received.
    ///
    /// # Panics
    /// Panics on overflow. See [`AdvancePayment::checked_gross`].
    pub fn gross(&self) -> Amount<5> {
        self.checked_gross().expect("advance gross overflowed")
    }
}

// ── Prepayment ────────────────────────────────────────────────────────────────

/// What a document has already been paid — EN 16931 **BT-113**, at either of the
/// two granularities the standard and the tax rules between them require.
///
/// A flat total and a list of itemised advances are the *same fact* at different
/// resolutions, not two independent settings. Modelling them as one enum means a
/// document cannot declare a total of 900 alongside advances summing to 476: that
/// state is not rejected at runtime, it simply cannot be written down.
///
/// ```rust
/// use billing::{AdvancePayment, Amount, Prepayment, TaxBreakdownEntry, TaxCategory};
/// use rust_decimal::dec;
///
/// // Nothing paid yet.
/// assert_eq!(Prepayment::default(), Prepayment::None);
/// assert_eq!(Prepayment::None.total()?, Amount::<5>::ZERO);
///
/// // A flat figure, when the tax split is unknown or not required.
/// let flat = Prepayment::total_of(Amount::parse("900.00000")?)?;
/// assert_eq!(flat.total()?, Amount::parse("900.00000")?);
/// assert!(flat.advances().is_empty());
///
/// // Itemised, when the settling document must state the tax in each advance.
/// let itemised = Prepayment::itemised(vec![AdvancePayment::new(vec![
///     TaxBreakdownEntry::new(TaxCategory::Standard, dec!(0.19),
///         Amount::parse("375.00000")?, Amount::parse("71.25000")?),
/// ])?])?;
/// assert_eq!(itemised.total()?,     Amount::parse("446.25000")?);
/// assert_eq!(itemised.tax_total()?, Amount::parse("71.25000")?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "PrepaymentRepr"))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Prepayment {
    /// Nothing has been paid in advance.
    #[default]
    None,
    /// A flat already-paid total, with no per-rate tax split.
    ///
    /// This is BT-113 exactly as EN 16931 defines it. Sufficient wherever the
    /// settling document need not restate the tax contained in the advances.
    Total(Amount<5>),
    /// Itemised advances, each carrying the tax it contains.
    ///
    /// Required wherever a final invoice must deduct the advances *and their tax*
    /// — see the [module documentation](self).
    Itemised(Vec<AdvancePayment>),
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
// Mirrors the derived `Serialize` representation (externally tagged). Marking this
// `untagged` would silently break the round-trip: `Serialize` emits
// `{"Itemised":[…]}` while an untagged reader expects a bare array.
enum PrepaymentRepr {
    None,
    Total(Amount<5>),
    Itemised(Vec<AdvancePayment>),
}

#[cfg(feature = "serde")]
impl TryFrom<PrepaymentRepr> for Prepayment {
    type Error = BillingError;
    fn try_from(r: PrepaymentRepr) -> Result<Self, Self::Error> {
        match r {
            PrepaymentRepr::None => Ok(Self::None),
            PrepaymentRepr::Total(a) => Self::total_of(a),
            PrepaymentRepr::Itemised(v) => Self::itemised(v),
        }
    }
}

impl Prepayment {
    /// A flat already-paid total.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `total` is negative — "already paid less
    /// than nothing" is not a state.
    pub fn total_of(total: Amount<5>) -> Result<Self, BillingError> {
        if total.is_negative() {
            return Err(BillingError::InvalidInput {
                reason: format!("prepaid amount must be >= 0, got {total}"),
            });
        }
        Ok(Self::Total(total))
    }

    /// Itemised advances.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `advances` is empty — use
    /// [`Prepayment::None`] to say nothing was paid.
    pub fn itemised(advances: Vec<AdvancePayment>) -> Result<Self, BillingError> {
        if advances.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "Prepayment::itemised requires at least one advance;                          use Prepayment::None for no prepayment"
                    .into(),
            });
        }
        Ok(Self::Itemised(advances))
    }

    /// The already-paid total — BT-113 — whichever form this is.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn total(&self) -> Result<Amount<5>, BillingError> {
        match self {
            Self::None => Ok(Amount::ZERO),
            Self::Total(a) => Ok(*a),
            Self::Itemised(v) => {
                let mut sum = Amount::ZERO;
                for a in v {
                    sum = sum.checked_add(a.checked_gross()?)?;
                }
                Ok(sum)
            }
        }
    }

    /// The tax contained in the advances — zero unless itemised.
    ///
    /// This is the figure a final invoice must state alongside the deducted
    /// amounts. A [`Prepayment::Total`] cannot supply it, which is precisely why
    /// the itemised form exists.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn tax_total(&self) -> Result<Amount<5>, BillingError> {
        let mut sum = Amount::ZERO;
        for a in self.advances() {
            sum = sum.checked_add(a.checked_tax_total()?)?;
        }
        Ok(sum)
    }

    /// The itemised advances — empty for [`Prepayment::None`] and
    /// [`Prepayment::Total`].
    #[must_use]
    pub fn advances(&self) -> &[AdvancePayment] {
        match self {
            Self::Itemised(v) => v,
            _ => &[],
        }
    }

    /// Whether anything has been paid in advance.
    #[must_use]
    pub fn is_none(&self) -> bool {
        matches!(self, Self::None)
    }
}

// ── residual_breakdown ────────────────────────────────────────────────────────

/// Subtract advances from a full VAT breakdown, yielding what is left to invoice.
///
/// Use this to build a **residual invoice**: rather than billing the whole supply
/// and deducting the advances, bill only the remainder and do not list the
/// advances at all. That form needs no per-advance tax statement, which is why the
/// German BMF recommends it for structured e-invoices (Schreiben v. 15.10.2024,
/// Rn. 48) — EN 16931's core profiles have nowhere to put that data.
///
/// Groups are matched on `(category, normalised rate)`. Entries that reduce to
/// zero base **and** zero tax are dropped, since a breakdown line for nothing is
/// noise on an invoice.
///
/// # Errors
/// - [`BillingError::InvalidInput`] if an advance names a `(category, rate)` group
///   absent from `full` — the advance cannot have taxed something this supply does
///   not contain.
/// - [`BillingError::InvalidInput`] if the advances exceed `full` in any group.
///   Over-deduction would understate output tax on the supply.
/// - [`BillingError::MonetaryOverflow`] on overflow.
///
/// ```rust
/// use billing::{advance::residual_breakdown, AdvancePayment, Amount,
///               TaxBreakdownEntry, TaxCategory};
/// use rust_decimal::dec;
///
/// // Whole supply: 1000.00 net + 190.00 VAT.
/// let full = vec![TaxBreakdownEntry::new(
///     TaxCategory::Standard, dec!(0.19),
///     Amount::parse("1000.00000")?, Amount::parse("190.00000")?,
/// )];
///
/// // Already invoiced and paid: 750.00 net + 142.50 VAT.
/// let advances = vec![AdvancePayment::new(vec![TaxBreakdownEntry::new(
///     TaxCategory::Standard, dec!(0.19),
///     Amount::parse("750.00000")?, Amount::parse("142.50000")?,
/// )])?];
///
/// let residual = residual_breakdown(&full, &advances)?;
/// assert_eq!(residual[0].taxable_base, Amount::parse("250.00000")?);
/// assert_eq!(residual[0].tax_amount,   Amount::parse("47.50000")?);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub fn residual_breakdown(
    full: &[TaxBreakdownEntry],
    advances: &[AdvancePayment],
) -> Result<Vec<TaxBreakdownEntry>, BillingError> {
    let mut residual: Vec<TaxBreakdownEntry> = full.to_vec();

    for advance in advances {
        for entry in &advance.tax {
            let key = entry.group_key();
            let target = residual
                .iter_mut()
                .find(|e| e.group_key() == key)
                .ok_or_else(|| BillingError::InvalidInput {
                    reason: format!(
                        "advance covers VAT group ({}, {}), which the supply does not contain",
                        key.0, key.1
                    ),
                })?;
            let base = target.taxable_base.checked_sub(entry.taxable_base)?;
            let tax = target.tax_amount.checked_sub(entry.tax_amount)?;
            if base.is_negative() || tax.is_negative() {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "advances exceed the supply in VAT group ({}, {}): \
                         deducting would understate output tax",
                        key.0, key.1
                    ),
                });
            }
            target.taxable_base = base;
            target.tax_amount = tax;
        }
    }

    residual.retain(|e| !(e.taxable_base.is_zero() && e.tax_amount.is_zero()));
    Ok(residual)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vat::TaxCategory;
    use rust_decimal::dec;

    fn entry(rate: &str, base: &str, tax: &str) -> TaxBreakdownEntry {
        TaxBreakdownEntry::new(
            TaxCategory::Standard,
            rate.parse().unwrap(),
            Amount::parse(base).unwrap(),
            Amount::parse(tax).unwrap(),
        )
    }

    #[test]
    fn document_kind_code_roundtrip() {
        for k in [
            DocumentKind::CommercialInvoice,
            DocumentKind::PartialInvoice,
            DocumentKind::PrepaymentInvoice,
            DocumentKind::CorrectedInvoice,
            DocumentKind::CreditNote,
            DocumentKind::DebitNote,
            DocumentKind::SelfBilledInvoice,
            DocumentKind::PartialConstructionInvoice,
            DocumentKind::PartialFinalConstructionInvoice,
            DocumentKind::FinalConstructionInvoice,
        ] {
            assert_eq!(DocumentKind::from_code(k.code()), Some(k));
        }
        assert_eq!(DocumentKind::from_code(1), None);
        assert_eq!(DocumentKind::default(), DocumentKind::CommercialInvoice);
    }

    #[test]
    fn advance_derives_gross_from_components() {
        let a = AdvancePayment::new(vec![
            entry("0.19", "300.00000", "57.00000"),
            entry("0.07", "100.00000", "7.00000"),
        ])
        .unwrap();
        assert_eq!(a.net(), Amount::parse("400.00000").unwrap());
        assert_eq!(a.tax_total(), Amount::parse("64.00000").unwrap());
        assert_eq!(a.gross(), Amount::parse("464.00000").unwrap());
    }

    #[test]
    fn advance_requires_tax_data() {
        assert!(AdvancePayment::new(vec![]).is_err());
    }

    #[test]
    fn advance_rejects_duplicate_groups() {
        let r = AdvancePayment::new(vec![
            entry("0.19", "100.00000", "19.00000"),
            entry("0.19", "50.00000", "9.50000"),
        ]);
        assert!(r.is_err());
    }

    #[test]
    fn residual_subtracts_per_group() {
        let full = vec![entry("0.19", "1000.00000", "190.00000")];
        let advances =
            vec![AdvancePayment::new(vec![entry("0.19", "750.00000", "142.50000")]).unwrap()];
        let r = residual_breakdown(&full, &advances).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].taxable_base, Amount::parse("250.00000").unwrap());
        assert_eq!(r[0].tax_amount, Amount::parse("47.50000").unwrap());
    }

    #[test]
    fn residual_drops_fully_settled_groups() {
        let full = vec![entry("0.19", "100.00000", "19.00000")];
        let advances =
            vec![AdvancePayment::new(vec![entry("0.19", "100.00000", "19.00000")]).unwrap()];
        assert!(residual_breakdown(&full, &advances).unwrap().is_empty());
    }

    #[test]
    fn residual_rejects_over_deduction_and_unknown_groups() {
        let full = vec![entry("0.19", "100.00000", "19.00000")];
        let too_much =
            vec![AdvancePayment::new(vec![entry("0.19", "200.00000", "38.00000")]).unwrap()];
        assert!(residual_breakdown(&full, &too_much).is_err());

        let wrong_rate =
            vec![AdvancePayment::new(vec![entry("0.07", "10.00000", "0.70000")]).unwrap()];
        assert!(residual_breakdown(&full, &wrong_rate).is_err());
    }

    #[test]
    fn residual_normalises_rates_when_matching_groups() {
        // 0.19 and 0.1900 are one group (Peppol: trailing zeros are not significant).
        let full = vec![TaxBreakdownEntry::new(
            TaxCategory::Standard,
            dec!(0.1900),
            Amount::parse("100.00000").unwrap(),
            Amount::parse("19.00000").unwrap(),
        )];
        let advances =
            vec![AdvancePayment::new(vec![entry("0.19", "40.00000", "7.60000")]).unwrap()];
        let r = residual_breakdown(&full, &advances).unwrap();
        assert_eq!(r[0].taxable_base, Amount::parse("60.00000").unwrap());
    }
}
