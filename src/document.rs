//! [`BillingDocument`] — self-validating invoice with ordered positions + totals.
use crate::advance::{AdvancePayment, DocumentKind, Prepayment};
use crate::amount::Amount;
use crate::currency::Currency;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::period::Period;
use crate::settlement::CashRounding;
use crate::tax::{DiscountLayer, TaxLayer};
use crate::vat::TaxBreakdownEntry;

// ── DocumentMeta ──────────────────────────────────────────────────────────────

/// Non-computed header fields for a billing document.
///
/// All date/identifier fields are `Option<String>` to remain date-type-agnostic.
/// Store ISO 8601 date strings (e.g. `"2026-07-01"`) for interoperability.
///
/// Fields may be extended in future versions — use struct-update syntax
/// (`DocumentMeta { invoice_number: ..., ..Default::default() }`) to be forward-compatible.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DocumentMeta {
    /// Unique identifier for this document (e.g. `"INV-2026-001"`).
    pub invoice_number: String,
    /// EN 16931 **BT-3** — the UNTDID 1001 document type code.
    ///
    /// Defaults to [`DocumentKind::CommercialInvoice`]. Note that a final invoice
    /// deducting advances and a residual invoice are *both* `380`; what tells them
    /// apart is [`BillingDocument::advances`], not this field.
    pub kind: DocumentKind,
    /// The currency all amounts in this document are denominated in.
    ///
    /// Defaults to [`Currency::XXX`] ("no currency involved") rather than to any
    /// real currency — an invoice still showing `XXX` was never configured, which
    /// is a visible bug rather than a silent mislabelling.
    ///
    /// [`crate::merge_period_documents`] refuses to merge documents whose
    /// currencies differ.
    pub currency: Currency,
    /// Human-readable period label (e.g. `"2026-06"`, `"July 2026"`). Not parsed.
    pub period_label: String,
    /// The overall billing period covered by this document.
    ///
    /// Use [`Period::new`] to set both `from` and `to` together, ensuring they are
    /// always set as a pair.  Stored as ISO 8601 date strings.
    pub period: Option<Period>,
    /// Document issue date as ISO 8601 date string, e.g. `"2026-07-01"`.
    /// Required by §14 UStG and §22 MessZV for German invoices.
    pub issue_date: Option<String>,
    /// Payment due date as ISO 8601 date string, e.g. `"2026-07-31"`.
    pub due_date: Option<String>,
    /// Sender / issuer identifier (MP-ID, GLN, BDEW code, or free-form).
    pub issuer_id: Option<String>,
    /// Recipient identifier (MP-ID, GLN, BDEW code, or free-form).
    pub recipient_id: Option<String>,
    /// Optional free-text remarks printed on the document.
    pub notes: Option<String>,
    /// Arbitrary domain-specific key-value labels.
    ///
    /// Use this bag to attach domain identifiers without encoding them into
    /// other fields (e.g. `"malo_id"` → `"52435677816"`, `"billing_year"` → `"2026"`).
    /// Keys and values are free-form strings; the billing engine does not interpret them.
    pub labels: std::collections::BTreeMap<String, String>,
}

// ── BillingDocument ───────────────────────────────────────────────────────────

/// A complete, self-validating billing document.
///
/// Holds ordered positions (net → discounts → taxes) and pre-computed totals.
///
/// [`BillingDocument::from_positions`] **computes** the totals from the positions,
/// so a document it returns satisfies every invariant by construction — exactly,
/// with no tolerance. [`BillingDocument::validate`] re-checks them for documents
/// that were assembled another way (deserialised, allocated, merged) or mutated
/// after the fact.
///
/// # Construction
///
/// - [`BillingDocument::from_positions`] — supply positions and layer vecs directly.
/// - [`BillingDocument::builder`] — fluent builder; use `.tariff(t, u)?` to load
///   positions from a [`crate::Tariff`] implementation.
/// # Validation on deserialisation
///
/// `BillingDocument` re-runs [`BillingDocument::validate`] when deserialised, so a
/// document whose stored totals disagree with its positions — truncated write,
/// hand-edited JSON, a bug in a producing system — is rejected at the boundary
/// rather than silently trusted. This is the one place the engine cannot rely on
/// construction-time invariants, because serde reconstructs private fields directly.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "BillingDocumentRepr"))]
#[derive(Debug, Clone, PartialEq)]
pub struct BillingDocument {
    /// Document header (invoice number, period label, notes).
    pub meta: DocumentMeta,
    net_positions: Vec<LineItem>,
    tax_positions: Vec<LineItem>,
    discount_positions: Vec<LineItem>,
    net_total: Amount<5>,
    tax_total: Amount<5>,
    gross_total: Amount<5>,
    discount_total: Amount<5>,
    tax_breakdown: Vec<TaxBreakdownEntry>,
    /// Already-paid amounts (BT-113), flat or itemised. One field, so a total and
    /// a set of advances can never disagree.
    prepayment: Prepayment,
    prepaid: Amount<5>,
    rounding: Amount<5>,
    /// The rule that produced `rounding`, retained so that a later change to
    /// `prepaid` recomputes it instead of leaving a stale, non-tenderable figure.
    cash_rounding: Option<CashRounding>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct BillingDocumentRepr {
    meta: DocumentMeta,
    net_positions: Vec<LineItem>,
    tax_positions: Vec<LineItem>,
    discount_positions: Vec<LineItem>,
    net_total: Amount<5>,
    tax_total: Amount<5>,
    gross_total: Amount<5>,
    discount_total: Amount<5>,
    #[serde(default)]
    tax_breakdown: Vec<TaxBreakdownEntry>,
    #[serde(default)]
    prepayment: Prepayment,
    #[serde(default)]
    prepaid: Amount<5>,
    #[serde(default)]
    rounding: Amount<5>,
    #[serde(default)]
    cash_rounding: Option<CashRounding>,
}

#[cfg(feature = "serde")]
impl TryFrom<BillingDocumentRepr> for BillingDocument {
    type Error = BillingError;
    fn try_from(r: BillingDocumentRepr) -> Result<Self, Self::Error> {
        let doc = Self {
            meta: r.meta,
            net_positions: r.net_positions,
            tax_positions: r.tax_positions,
            discount_positions: r.discount_positions,
            net_total: r.net_total,
            tax_total: r.tax_total,
            gross_total: r.gross_total,
            discount_total: r.discount_total,
            tax_breakdown: r.tax_breakdown,
            prepaid: r.prepaid,
            rounding: r.rounding,
            cash_rounding: r.cash_rounding,
            prepayment: r.prepayment,
        };
        doc.validate()?;
        Ok(doc)
    }
}

impl BillingDocument {
    // ── Accessors ─────────────────────────────────────────────────────────────

    /// Core billing positions (debit and credit).
    #[must_use]
    pub fn net_positions(&self) -> &[LineItem] {
        &self.net_positions
    }
    /// Tax / surcharge / percentage-charge positions.
    #[must_use]
    pub fn tax_positions(&self) -> &[LineItem] {
        &self.tax_positions
    }
    /// Discount positions (always negative net amounts).
    #[must_use]
    pub fn discount_positions(&self) -> &[LineItem] {
        &self.discount_positions
    }

    /// Sum of all net (non-tax, non-discount) positions.
    pub fn net_total(&self) -> Amount<5> {
        self.net_total
    }
    /// Sum of all tax positions.
    pub fn tax_total(&self) -> Amount<5> {
        self.tax_total
    }
    /// `gross = net + tax`. Discounts are included in `net_total` as negatives.
    pub fn gross_total(&self) -> Amount<5> {
        self.gross_total
    }

    /// The currency this document is denominated in. Shorthand for `meta.currency`.
    #[must_use]
    pub fn currency(&self) -> Currency {
        self.meta.currency
    }

    /// Sum of all discount positions (always ≤ 0).
    ///
    /// Discounts are already embedded in `net_total` as negative amounts.
    /// This accessor surfaces them separately for display or BO4E output.
    ///
    /// Computed once at construction (where overflow is reported as an error)
    /// rather than re-summed per call with a panicking `.sum()`.
    pub fn discount_total(&self) -> Amount<5> {
        self.discount_total
    }

    /// The EN 16931 VAT breakdown (BG-23) — taxable base and tax per
    /// `(category, rate)` pair.
    ///
    /// A per-rate breakdown is a legal requirement, not a convenience: EU VAT
    /// Directive art. 226(8)–(10) demands "the taxable amount per rate or
    /// exemption", and §14 Abs. 4 Nr. 7–8 UStG says the same. A lump
    /// [`BillingDocument::tax_total`] cannot satisfy either on a mixed-rate invoice.
    ///
    /// Entries are contributed by [`crate::TaxLayer::breakdown`] and merged by
    /// `(category, normalised rate)`. Layers that are not VAT — a platform
    /// commission, a per-unit excise — contribute nothing here, so this may be
    /// empty even when `tax_total` is not.
    #[must_use]
    pub fn tax_breakdown(&self) -> &[TaxBreakdownEntry] {
        &self.tax_breakdown
    }

    /// EN 16931 **BT-113** — the sum of amounts already paid (advance payments,
    /// deposits, instalments).
    pub fn prepaid(&self) -> Amount<5> {
        self.prepaid
    }

    /// What has already been paid, flat or itemised — EN 16931 **BT-113**.
    #[must_use]
    pub fn prepayment(&self) -> &Prepayment {
        &self.prepayment
    }

    /// The itemised advance payments this document settles, if any.
    ///
    /// Empty for an ordinary invoice, and empty for a *residual* invoice (which
    /// bills only the remainder and deliberately does not list the advances).
    /// Non-empty makes this a **final invoice**: totals and the VAT breakdown still
    /// describe the whole supply, and the advances plus their tax are deducted to
    /// reach [`BillingDocument::amount_due`].
    ///
    /// See [`crate::advance`] for why the per-advance tax matters.
    #[must_use]
    pub fn advances(&self) -> &[AdvancePayment] {
        self.prepayment.advances()
    }

    /// Total tax contained in the advance payments.
    ///
    /// This is the figure a final invoice must state alongside the deducted
    /// amounts — §14 Abs. 5 Satz 2 UStG's *"und die auf sie entfallenden
    /// Steuerbeträge"*. Zero when there are no itemised advances.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn advance_tax_total(&self) -> Result<Amount<5>, BillingError> {
        self.prepayment.tax_total()
    }

    /// The advances merged into one breakdown line per `(category, rate)`.
    ///
    /// This is the deduction table a final invoice presents: how much net and how
    /// much tax is subtracted, per VAT rate. Render it next to the VAT breakdown,
    /// which continues to describe the full supply.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn advance_deductions(&self) -> Result<Vec<TaxBreakdownEntry>, BillingError> {
        merge_breakdown(
            self.advances()
                .iter()
                .flat_map(|a| a.tax().iter().cloned())
                .collect(),
        )
    }

    /// EN 16931 **BT-114** — the cash-rounding adjustment applied to reach a
    /// tenderable figure. Zero unless [`BillingDocument::with_cash_rounding`]
    /// was used.
    pub fn rounding(&self) -> Amount<5> {
        self.rounding
    }

    /// EN 16931 **BT-115** — the amount actually due for payment.
    ///
    /// Implements rule BR-CO-16:
    ///
    /// ```text
    /// amount_due = gross_total − prepaid + rounding
    /// ```
    ///
    /// **May legitimately be negative** when prepayments exceed the gross total —
    /// the ordinary utility credit-balance case, where the supplier owes the
    /// customer a refund. It is deliberately not clamped to zero.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn amount_due(&self) -> Result<Amount<5>, BillingError> {
        self.gross_total
            .checked_sub(self.prepaid)?
            .checked_add(self.rounding)
    }

    /// Iterate over every position (net, discount, and tax) that carries `tag`.
    ///
    /// Searches all three position buckets in order: net → discounts → taxes.
    /// Use this when building domain-specific output (e.g. BO4E `rechnungspositionen`
    /// filtered by commodity tag).
    ///
    /// ```rust
    /// use billing::{BillingDocument, DocumentMeta, LineItem, Amount, Currency};
    ///
    /// let doc = BillingDocument::from_positions(
    ///     DocumentMeta { currency: Currency::EUR, ..Default::default() },
    ///     vec![
    ///         LineItem::fixed("Arbeit", Amount::parse("100.00000").unwrap())
    ///             .tag("commodity").build().unwrap(),
    ///         LineItem::fixed("Grundpreis", Amount::parse("8.50000").unwrap())
    ///             .tag("fixed").build().unwrap(),
    ///     ],
    ///     vec![], vec![],
    /// ).unwrap();
    ///
    /// let commodity: Vec<_> = doc.positions_by_tag("commodity").collect();
    /// assert_eq!(commodity.len(), 1);
    /// assert_eq!(commodity[0].description, "Arbeit");
    /// ```
    pub fn positions_by_tag<'a>(&'a self, tag: &'a str) -> impl Iterator<Item = &'a LineItem> + 'a {
        self.net_positions
            .iter()
            .chain(self.discount_positions.iter())
            .chain(self.tax_positions.iter())
            .filter(move |p| p.has_tag(tag))
    }

    /// All positions in order: net → discounts → taxes.
    ///
    /// Returns a zero-allocation iterator. Call `.collect()` if you need a `Vec`.
    pub fn all_positions(&self) -> impl Iterator<Item = &LineItem> + '_ {
        self.net_positions
            .iter()
            .chain(self.discount_positions.iter())
            .chain(self.tax_positions.iter())
    }

    // ── Construction ─────────────────────────────────────────────────────────

    /// Start a fluent builder. See [`BillingDocumentBuilder`].
    #[must_use]
    pub fn builder() -> BillingDocumentBuilder {
        BillingDocumentBuilder::default()
    }

    /// Build from already-computed positions and tax/discount layers.
    ///
    /// Discount layers are applied first (reduce the taxable base),
    /// then tax layers are applied to the combined net + discount positions.
    pub fn from_positions(
        meta: DocumentMeta,
        positions: Vec<LineItem>,
        tax_layers: Vec<Box<dyn TaxLayer>>,
        discounts: Vec<Box<dyn DiscountLayer>>,
    ) -> Result<Self, BillingError> {
        // `LineItem` has public fields, so a caller can hand us a position with an
        // empty description or a negative quantity. Check it here: the type doc
        // promises a document from this constructor satisfies every invariant, and
        // check 11 of `validate()` would otherwise reject what we just returned.
        for item in &positions {
            item.validate()?;
        }

        let discount_positions: Vec<LineItem> = discounts
            .iter()
            .map(|d| {
                let item = d.compute(&positions)?;
                // The `DiscountLayer` contract says "always returns a credit".
                // Check it here so a misbehaving layer is named at the point of
                // failure rather than surfacing later as a validation error.
                if item.net_amount.is_positive() {
                    return Err(BillingError::LayerError {
                        reason: format!(
                            "discount layer {:?} returned a positive amount ({}); \
                             a discount must be a credit",
                            d.name(),
                            item.net_amount
                        ),
                    });
                }
                Ok(item)
            })
            .collect::<Result<_, _>>()?;

        let net_total = Amount::checked_sum(
            positions
                .iter()
                .chain(&discount_positions)
                .map(|p| p.net_amount),
        )?;

        // Accumulate tax layers: each layer receives ALL positions accumulated
        // so far (net + discounts + prior tax layers).  This is required for
        // compound taxes where later layers include earlier ones in their base
        // (e.g. a levy before VAT means VAT is computed on net + levy).
        let mut accumulated: Vec<LineItem> =
            Vec::with_capacity(positions.len() + discount_positions.len() + tax_layers.len());
        accumulated.extend(positions.iter().cloned());
        accumulated.extend(discount_positions.iter().cloned());
        let mut tax_positions: Vec<LineItem> = Vec::with_capacity(tax_layers.len());
        let mut breakdown_entries: Vec<TaxBreakdownEntry> = Vec::new();
        for t in &tax_layers {
            // `breakdown` sees the SAME slice `compute` does, so the reported
            // taxable base is exactly the base the tax was charged on.
            if let Some(entry) = t.breakdown(&accumulated)? {
                entry.validate()?;
                breakdown_entries.push(entry);
            }
            let item = t.compute(&accumulated)?;
            accumulated.push(item.clone());
            tax_positions.push(item);
        }
        let tax_breakdown = merge_breakdown(breakdown_entries)?;

        let tax_total = Amount::checked_sum(tax_positions.iter().map(|p| p.net_amount))?;
        let gross_total = net_total.checked_add(tax_total)?;
        let discount_total = Amount::checked_sum(discount_positions.iter().map(|p| p.net_amount))?;

        Ok(Self {
            meta,
            net_positions: positions,
            tax_positions,
            discount_positions,
            net_total,
            tax_total,
            gross_total,
            discount_total,
            tax_breakdown,
            prepayment: Prepayment::None,
            prepaid: Amount::ZERO,
            rounding: Amount::ZERO,
            cash_rounding: None,
        })
    }

    /// Record already-paid amounts — EN 16931 **BT-113**.
    ///
    /// Use for advance payments (Abschlagszahlungen), deposits and part payments.
    /// This reduces [`BillingDocument::amount_due`] but deliberately leaves
    /// `net_total`, `tax_total`, `gross_total` and the VAT breakdown untouched:
    /// the supply was made in full and output VAT is owed on the full base.
    ///
    /// **Do not model prepayments as negative line items or as discounts.** That
    /// would shrink the taxable base and under-declare output tax — in Germany,
    /// failing to deduct advances correctly on an Endrechnung makes the entire VAT
    /// amount payable a second time under §14c Abs. 1 UStG.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `prepaid` is negative.
    pub fn with_prepaid(self, prepaid: Amount<5>) -> Result<Self, BillingError> {
        self.with_prepayment(Prepayment::total_of(prepaid)?)
    }

    /// Set what has already been paid — EN 16931 **BT-113** — in either form.
    ///
    /// Replaces any previous prepayment wholesale. Because [`Prepayment`] is one
    /// value rather than two fields, a flat total and a set of itemised advances
    /// cannot both be in force, and cannot disagree.
    ///
    /// Totals and the VAT breakdown are untouched: the supply happened in full and
    /// output tax is owed on the whole base. Only
    /// [`amount_due`](BillingDocument::amount_due) moves.
    ///
    /// # Errors
    /// - [`BillingError::InvalidInput`] if an itemised advance covers a
    ///   `(category, rate)` group this document's VAT breakdown lacks, or if the
    ///   advances exceed the supply in any group.
    /// - [`BillingError::MonetaryOverflow`] on overflow.
    pub fn with_prepayment(mut self, prepayment: Prepayment) -> Result<Self, BillingError> {
        // Reuse the residual computation purely as a validity check: it rejects
        // advances naming a VAT group the supply lacks, and advances exceeding the
        // supply in any group.
        crate::advance::residual_breakdown(&self.tax_breakdown, prepayment.advances())?;
        let prepaid = prepayment.total()?;
        if prepaid.is_negative() {
            return Err(BillingError::InvalidInput {
                reason: format!("prepaid amount must be >= 0, got {prepaid}"),
            });
        }
        self.prepayment = prepayment;
        self.prepaid = prepaid;
        // Cash rounding is a function of `gross − prepaid`, so a rule applied
        // earlier would otherwise leave a stale adjustment and an `amount_due` that
        // is not a tenderable multiple.
        self.recompute_rounding()?;
        Ok(self)
    }

    /// Recompute [`BillingDocument::rounding`] from the stored rule, if any.
    fn recompute_rounding(&mut self) -> Result<(), BillingError> {
        if let Some(rule) = self.cash_rounding {
            let payable = self.gross_total.checked_sub(self.prepaid)?;
            self.rounding = rule.difference(payable)?;
        }
        Ok(())
    }

    /// Attach the advance payments this document settles, making it a **final
    /// invoice**.
    ///
    /// Sets [`BillingDocument::prepaid`] (BT-113) to the advances' combined gross,
    /// so [`amount_due`](BillingDocument::amount_due) becomes the remainder. Totals
    /// and the VAT breakdown are **not** touched: the supply happened in full and
    /// output tax is owed on the whole base.
    ///
    /// > Advances are a **gross** deduction. Subtracting them from the net base
    /// > understates output tax and breaks EN 16931 rules BR-S-08 and BR-CO-14.
    ///
    /// # Prefer a residual invoice where the process allows
    ///
    /// EN 16931's core profiles have nowhere to put per-advance tax, so a final
    /// invoice needs that stated out of band. Billing only the remainder avoids the
    /// problem entirely — compute it with
    /// [`residual_breakdown`](crate::advance::residual_breakdown) and attach no
    /// advances.
    ///
    /// # Errors
    /// - [`BillingError::InvalidInput`] if an advance covers a `(category, rate)`
    ///   group this document's VAT breakdown lacks, or if the advances exceed the
    ///   supply in any group — the deduction would not correspond to anything
    ///   invoiced.
    /// - [`BillingError::MonetaryOverflow`] on overflow.
    ///
    /// ```rust
    /// use billing::prelude::*;
    /// use billing::{AdvancePayment, FixedRateTax, TaxBreakdownEntry, TaxCategory};
    /// use rust_decimal::dec;
    ///
    /// // Whole supply: 1000.00 net + 19% VAT.
    /// let doc = BillingDocument::from_positions(
    ///     DocumentMeta { currency: Currency::EUR, ..Default::default() },
    ///     vec![LineItem::fixed("Jahresverbrauch", Amount::parse("1000.00000")?).build()?],
    ///     vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)],
    ///     vec![],
    /// )?;
    ///
    /// // Two advances already invoiced and paid: 375.00 net + 71.25 VAT each.
    /// let advance = |n: &str| AdvancePayment::new(vec![TaxBreakdownEntry::new(
    ///     TaxCategory::Standard, dec!(0.19),
    ///     Amount::parse("375.00000").unwrap(), Amount::parse("71.25000").unwrap(),
    /// )]).unwrap().with_reference(n);
    ///
    /// let doc = doc.with_advances(vec![advance("AB-1"), advance("AB-2")])?;
    ///
    /// // The base still describes the whole supply …
    /// assert_eq!(doc.tax_breakdown()[0].taxable_base, Amount::parse("1000.00000")?);
    /// assert_eq!(doc.gross_total(), Amount::parse("1190.00000")?);
    /// // … while only the remainder is payable.
    /// assert_eq!(doc.prepaid(),            Amount::parse("892.50000")?);
    /// assert_eq!(doc.advance_tax_total()?, Amount::parse("142.50000")?);
    /// assert_eq!(doc.amount_due()?,        Amount::parse("297.50000")?);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    pub fn with_advances(self, advances: Vec<AdvancePayment>) -> Result<Self, BillingError> {
        self.with_prepayment(Prepayment::itemised(advances)?)
    }

    /// Apply a cash-rounding rule to the payable amount — EN 16931 **BT-114**.
    ///
    /// Rounds `gross_total − prepaid` to the nearest tenderable multiple and
    /// stores the difference. Totals and the VAT breakdown are **not** modified:
    /// in every jurisdiction surveyed except Switzerland the rounding difference
    /// lies outside the taxable base, and VAT stays computed on the exact
    /// pre-rounding consideration.
    ///
    /// Apply this only to a cash tender. Rounding a card or transfer payment is
    /// affirmatively unlawful in Denmark and contrary to guidance in Norway.
    ///
    /// ```rust
    /// use billing::{Amount, BillingDocument, CashRounding, Currency, DocumentMeta,
    ///               LineItem, RoundingStrategy};
    ///
    /// let doc = BillingDocument::from_positions(
    ///     DocumentMeta { currency: Currency::CHF, ..Default::default() },
    ///     vec![LineItem::fixed("Service", Amount::parse("12.34000").unwrap()).build().unwrap()],
    ///     vec![], vec![],
    /// ).unwrap();
    ///
    /// let rappen = CashRounding::new(
    ///     Amount::parse("0.05000").unwrap(),
    ///     RoundingStrategy::MidpointAwayFromZero,
    /// ).unwrap();
    /// let doc = doc.with_cash_rounding(rappen).unwrap();
    ///
    /// assert_eq!(doc.gross_total(),          Amount::parse("12.34000").unwrap()); // unchanged
    /// assert_eq!(doc.rounding(),             Amount::parse("0.01000").unwrap());  // BT-114
    /// assert_eq!(doc.amount_due().unwrap(),  Amount::parse("12.35000").unwrap());
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn with_cash_rounding(mut self, rule: CashRounding) -> Result<Self, BillingError> {
        self.cash_rounding = Some(rule);
        self.recompute_rounding()?;
        Ok(self)
    }

    /// The cash-rounding rule in force, if one was set.
    #[must_use]
    pub fn cash_rounding(&self) -> Option<CashRounding> {
        self.cash_rounding
    }

    /// Produce the reversing document for this one — a credit note (Storno).
    ///
    /// Every monetary value is negated: each position's `net_amount`, all totals,
    /// the VAT breakdown (both base and tax), the prepaid amount and the rounding
    /// adjustment. Position `sign`s are flipped so debits become credits, keeping
    /// sign-based tax and discount filtering meaningful.
    ///
    /// Quantities are **not** negated — the reversal of "1000 kWh × 0.30" is
    /// "1000 kWh × −0.30", and [`LineItem::validate`] rejects negative quantities.
    ///
    /// `meta` is the new document's header: a credit note needs its own number and
    /// should reference the original (e.g. through `DocumentMeta::labels`).
    ///
    /// ```rust
    /// use billing::{Amount, BillingDocument, Currency, DocumentMeta, LineItem};
    ///
    /// let inv = BillingDocument::from_positions(
    ///     DocumentMeta { invoice_number: "INV-1".into(), currency: Currency::EUR,
    ///                    ..Default::default() },
    ///     vec![LineItem::fixed("Service", Amount::parse("100.00000").unwrap()).build().unwrap()],
    ///     vec![], vec![],
    /// ).unwrap();
    ///
    /// let credit = inv.reverse(DocumentMeta {
    ///     invoice_number: "CN-1".into(), currency: Currency::EUR, ..Default::default()
    /// }).unwrap();
    ///
    /// assert_eq!(credit.net_total(), Amount::parse("-100.00000").unwrap());
    /// credit.assert_valid();
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if any amount is `Amount::MIN`, which has
    /// no positive counterpart.
    pub fn reverse(&self, meta: DocumentMeta) -> Result<Self, BillingError> {
        fn flip(items: &[LineItem]) -> Result<Vec<LineItem>, BillingError> {
            items
                .iter()
                .map(|p| {
                    let mut out = p.clone();
                    out.net_amount = p.net_amount.checked_neg()?;
                    // Derive the sign from the negated amount rather than blindly
                    // swapping Debit↔Credit. A `Debit` with a NEGATIVE net (a
                    // negative-spot-price line, or VAT on a negative base) would
                    // otherwise flip to a `Credit` with a POSITIVE net — a state
                    // `LineItem::validate` rejects, producing a document that
                    // passes `assert_valid()` but cannot be serialised and
                    // reloaded.
                    out.normalize_sign();
                    Ok(out)
                })
                .collect()
        }
        let tax_breakdown = self
            .tax_breakdown
            .iter()
            .map(|e| {
                Ok(TaxBreakdownEntry {
                    category: e.category,
                    rate: e.rate,
                    taxable_base: e.taxable_base.checked_neg()?,
                    tax_amount: e.tax_amount.checked_neg()?,
                    exemption_reason: e.exemption_reason.clone(),
                })
            })
            .collect::<Result<Vec<_>, BillingError>>()?;

        Ok(Self {
            meta,
            net_positions: flip(&self.net_positions)?,
            tax_positions: flip(&self.tax_positions)?,
            discount_positions: flip(&self.discount_positions)?,
            net_total: self.net_total.checked_neg()?,
            tax_total: self.tax_total.checked_neg()?,
            gross_total: self.gross_total.checked_neg()?,
            discount_total: self.discount_total.checked_neg()?,
            tax_breakdown,
            // Settlement figures do NOT carry over to the reversal.
            //
            // Negating them would produce a negative BT-113, which is meaningless
            // ("a payment already made of less than nothing") and which check 6 of
            // `validate()` rejects outright — so an invoice carrying an advance
            // payment could never be reversed into a valid credit note.
            //
            // Zero is also the arithmetically correct answer. If an invoice of
            // gross 1190 had 900 prepaid, the customer paid 900 + 290 = 1190 in
            // total, so the credit note's amount due is the full −1190 refund.
            prepayment: Prepayment::None,
            prepaid: Amount::ZERO,
            rounding: Amount::ZERO,
            cash_rounding: None,
        })
    }

    /// Construct directly from pre-computed fields. Used by `allocation.rs` and `period.rs`.
    ///
    /// No recomputation or validation of the *totals* is performed — callers must
    /// ensure consistency between positions and totals.  `discount_total` is
    /// derived here (it is a pure function of `discount_positions`) so callers
    /// cannot desynchronise it.
    pub(crate) fn from_raw(parts: DocumentParts) -> Result<Self, BillingError> {
        let discount_total =
            Amount::checked_sum(parts.discount_positions.iter().map(|p| p.net_amount))?;
        Ok(Self {
            meta: parts.meta,
            net_positions: parts.net_positions,
            tax_positions: parts.tax_positions,
            discount_positions: parts.discount_positions,
            net_total: parts.net_total,
            tax_total: parts.tax_total,
            gross_total: parts.gross_total,
            discount_total,
            tax_breakdown: parts.tax_breakdown,
            // `from_raw` carries only the aggregate: allocation and merge refuse
            // documents with itemised advances, so nothing is lost here.
            prepayment: if parts.prepaid.is_zero() {
                Prepayment::None
            } else {
                Prepayment::Total(parts.prepaid)
            },
            prepaid: parts.prepaid,
            rounding: parts.rounding,
            cash_rounding: None,
        })
    }

    // ── Validation ────────────────────────────────────────────────────────────

    /// Assert full arithmetic correctness of the document. Returns `Result`.
    ///
    /// Eleven invariants are checked (all exact — no tolerance):
    /// 1. `Σ(net_positions + discount_positions) == net_total`
    /// 2. `Σ(tax_positions) == tax_total`
    /// 3. `net_total + tax_total == gross_total`
    /// 4. `Σ(discount_positions) == discount_total`
    /// 5. every VAT breakdown entry is category-consistent, its tax matches
    ///    `base × rate` within EN 16931's tolerance (BR-CO-17), and no
    ///    `(category, rate)` group appears twice (BR-CO-18)
    /// 6. `prepaid >= 0`
    /// 7. `rounding` matches the recorded cash-rounding rule, if any
    /// 8. `Σ(tax_breakdown)` is a component of `tax_total` (same sign, no larger)
    /// 9. no discount position is positive
    /// 10. `prepaid` equals `prepayment.total()`
    /// 11. every position satisfies [`LineItem::validate`]
    ///
    /// All documents built by this library satisfy these invariants at
    /// construction time. Call this after any external mutation to verify
    /// the document has not been corrupted.
    ///
    /// # See also
    /// [`BillingDocument::assert_valid`] — panicking convenience form for use in tests.
    pub fn validate(&self) -> Result<(), BillingError> {
        // Check 1: net positions + discount positions sum exactly to net_total.
        let computed_net = Amount::checked_sum(
            self.net_positions
                .iter()
                .chain(&self.discount_positions)
                .map(|p| p.net_amount),
        )?;
        if computed_net != self.net_total {
            return Err(BillingError::ValidationFailed {
                check: "net_total".into(),
                actual: computed_net.to_string(),
                expected: self.net_total.to_string(),
            });
        }

        // Check 2: tax positions sum exactly to tax_total.
        let computed_tax = Amount::checked_sum(self.tax_positions.iter().map(|p| p.net_amount))?;
        if computed_tax != self.tax_total {
            return Err(BillingError::ValidationFailed {
                check: "tax_total".into(),
                actual: computed_tax.to_string(),
                expected: self.tax_total.to_string(),
            });
        }

        // Check 3: net_total + tax_total == gross_total.
        let expected_gross = self.net_total.checked_add(self.tax_total)?;
        if expected_gross != self.gross_total {
            return Err(BillingError::ValidationFailed {
                check: "gross_total".into(),
                actual: expected_gross.to_string(),
                expected: self.gross_total.to_string(),
            });
        }

        // Check 5: every VAT breakdown entry satisfies its EN 16931 category rules,
        // and no two entries share a (category, rate) group — BR-CO-18 requires
        // exactly one breakdown line per distinct pair.
        let mut seen = Vec::with_capacity(self.tax_breakdown.len());
        for entry in &self.tax_breakdown {
            entry.validate()?;
            let key = entry.group_key();
            if seen.contains(&key) {
                return Err(BillingError::ValidationFailed {
                    check: "tax_breakdown".into(),
                    actual: format!("duplicate group ({}, {})", key.0, key.1),
                    expected: "one breakdown entry per (category, rate)".into(),
                });
            }
            seen.push(key);
        }

        // Check 8: the VAT breakdown must be a COMPONENT of the tax actually
        // charged. Exact equality is wrong — non-VAT layers (a commission, a
        // per-unit excise) add to `tax_total` without contributing a breakdown
        // entry — but the breakdown can never exceed the total or oppose its sign.
        // Without this, a document declaring 19.00 of output VAT while charging no
        // tax at all deserialised and validated cleanly.
        let breakdown_tax = Amount::checked_sum(self.tax_breakdown.iter().map(|e| e.tax_amount))?;
        let within = if self.tax_total.is_negative() {
            breakdown_tax <= Amount::ZERO && breakdown_tax >= self.tax_total
        } else {
            breakdown_tax >= Amount::ZERO && breakdown_tax <= self.tax_total
        };
        if !within {
            return Err(BillingError::ValidationFailed {
                check: "tax_breakdown_total".into(),
                actual: breakdown_tax.to_string(),
                expected: format!("a component of tax_total {}", self.tax_total),
            });
        }

        // Check 9: a discount position that ADDS to the invoice is a surcharge, not
        // a discount. The `DiscountLayer` docs promise a credit; a third-party
        // implementation returning a debit would otherwise pass unnoticed.
        if let Some(bad) = self
            .discount_positions
            .iter()
            .find(|p| p.net_amount.is_positive())
        {
            return Err(BillingError::ValidationFailed {
                check: "discount_positions".into(),
                actual: format!("{:?} = {}", bad.description, bad.net_amount),
                expected: "every discount position <= 0".into(),
            });
        }

        // Check 11: every position must satisfy its own invariants.
        //
        // `LineItem` has public fields by design, so a document can be assembled or
        // deserialised holding a position with an empty description or a negative
        // quantity. The totals would still reconcile, and the document would pass
        // every other check here, while being unrenderable as a lawful invoice.
        for (bucket, items) in [
            ("net_positions", &self.net_positions),
            ("discount_positions", &self.discount_positions),
            ("tax_positions", &self.tax_positions),
        ] {
            for item in items.iter() {
                item.validate()
                    .map_err(|e| BillingError::ValidationFailed {
                        check: bucket.into(),
                        actual: format!("{:?}: {e}", item.description),
                        expected: "every position satisfies LineItem::validate".into(),
                    })?;
            }
        }

        // Check 10: `prepaid` caches `prepayment.total()` so the accessor can stay
        // infallible. Nothing in EN 16931 ties an itemised deduction table to
        // BT-113 — ZUGFeRD leaves "Σ BT-X-291 vs BT-113" to the implementer — so
        // the cache is verified rather than trusted.
        let derived = self.prepayment.total()?;
        if derived != self.prepaid {
            return Err(BillingError::ValidationFailed {
                check: "prepaid_vs_prepayment".into(),
                actual: self.prepaid.to_string(),
                expected: derived.to_string(),
            });
        }

        // Check 6: a negative BT-113 is meaningless — a "payment already made" of
        // less than nothing. `with_prepaid` rejects it, but serde reconstructs the
        // field directly, so the boundary needs its own check.
        if self.prepaid.is_negative() {
            return Err(BillingError::ValidationFailed {
                check: "prepaid".into(),
                actual: self.prepaid.to_string(),
                expected: ">= 0".into(),
            });
        }

        // Check 7: if a cash-rounding rule is recorded, the stored adjustment must
        // be the one that rule produces for the current payable amount.
        if let Some(rule) = self.cash_rounding {
            let payable = self.gross_total.checked_sub(self.prepaid)?;
            let expected = rule.difference(payable)?;
            if expected != self.rounding {
                return Err(BillingError::ValidationFailed {
                    check: "rounding".into(),
                    actual: self.rounding.to_string(),
                    expected: expected.to_string(),
                });
            }
        }

        // Check 4: discount positions sum exactly to discount_total.
        let computed_discount =
            Amount::checked_sum(self.discount_positions.iter().map(|p| p.net_amount))?;
        if computed_discount != self.discount_total {
            return Err(BillingError::ValidationFailed {
                check: "discount_total".into(),
                actual: computed_discount.to_string(),
                expected: self.discount_total.to_string(),
            });
        }

        Ok(())
    }

    /// Assert full arithmetic correctness — panics on failure.
    ///
    /// Convenience wrapper around [`BillingDocument::validate`] suitable for use
    /// in tests and debug assertions. Follows the Rust convention that `assert_*`
    /// methods panic rather than returning `Result`.
    ///
    /// # Panics
    /// Panics if any of the eleven invariants is violated.
    pub fn assert_valid(&self) {
        self.validate()
            .expect("BillingDocument arithmetic invariants violated");
    }

    // ── Mutation helpers ──────────────────────────────────────────────────────

    /// Append an extra position and recompute net and gross totals.
    ///
    /// Tax positions are NOT recalculated — use this only for fixed surcharges
    /// like [`crate::minimum_charge`] that are added after initial tax calculation.
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if the document carries a VAT
    /// breakdown. Adding to the net total without re-running the tax layers would
    /// leave the breakdown's taxable base describing a smaller net than the
    /// document reports — a silently unlawful invoice. Rebuild the document with
    /// the extra position included instead.
    pub fn with_extra_position(mut self, item: LineItem) -> Result<Self, BillingError> {
        if !self.tax_breakdown.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "with_extra_position cannot be used on a document with a VAT \
                         breakdown: the breakdown's taxable base would no longer match \
                         the net total. Rebuild the document with the position included."
                    .into(),
            });
        }
        self.net_total = self.net_total.checked_add(item.net_amount)?;
        self.gross_total = self.net_total.checked_add(self.tax_total)?;
        self.net_positions.push(item);
        // The gross moved, so any cash rounding derived from it is now stale — and
        // check 7 of `validate()` would reject the document we just returned.
        self.recompute_rounding()?;
        Ok(self)
    }
}

/// The pre-computed pieces of a document, for the internal [`BillingDocument::from_raw`]
/// constructor used by allocation and period merging.
///
/// A named struct rather than eight positional parameters: the four `Vec<LineItem>`
/// and three `Amount<5>` fields are trivially transposable at a call site, and a
/// silent swap of `tax_total` and `gross_total` would produce a document that
/// still validates but bills the wrong figure.
pub(crate) struct DocumentParts {
    pub meta: DocumentMeta,
    pub net_positions: Vec<LineItem>,
    pub tax_positions: Vec<LineItem>,
    pub discount_positions: Vec<LineItem>,
    pub net_total: Amount<5>,
    pub tax_total: Amount<5>,
    pub gross_total: Amount<5>,
    pub tax_breakdown: Vec<TaxBreakdownEntry>,
    pub prepaid: Amount<5>,
    pub rounding: Amount<5>,
}

/// Merge breakdown entries that share a `(category, normalised rate)` group.
///
/// EN 16931 BR-CO-18 permits exactly one breakdown line per distinct pair, so two
/// tax layers at the same rate and category must be presented as one line with
/// summed base and tax. Order of first appearance is preserved for stable output.
pub(crate) fn merge_breakdown(
    entries: Vec<TaxBreakdownEntry>,
) -> Result<Vec<TaxBreakdownEntry>, BillingError> {
    let mut merged: Vec<TaxBreakdownEntry> = Vec::with_capacity(entries.len());
    for entry in entries {
        let key = entry.group_key();
        if let Some(existing) = merged.iter_mut().find(|e| e.group_key() == key) {
            // Conflicting BT-120 texts cannot be merged: EN 16931 allows one
            // exemption reason per breakdown line, so silently keeping the first
            // would drop a legally required justification (e.g. merging an
            // "Art. 132 education" line with an "Art. 135 financial services"
            // line). Two genuinely different reasons need two different
            // categories, or one combined text supplied by the caller.
            match (&existing.exemption_reason, &entry.exemption_reason) {
                (Some(a), Some(b)) if a != b => {
                    return Err(BillingError::InvalidInput {
                        reason: format!(
                            "VAT breakdown group ({}, {}) has conflicting exemption reasons: \
                             {a:?} and {b:?}",
                            key.0, key.1
                        ),
                    });
                }
                (None, Some(_)) => existing.exemption_reason = entry.exemption_reason,
                _ => {}
            }
            existing.taxable_base = existing.taxable_base.checked_add(entry.taxable_base)?;
            existing.tax_amount = existing.tax_amount.checked_add(entry.tax_amount)?;
        } else {
            merged.push(entry);
        }
    }
    Ok(merged)
}

// ── BillingDocumentBuilder ────────────────────────────────────────────────────

/// Fluent builder for [`BillingDocument`].
///
/// # Example — from a `Tariff` implementation
///
/// ```rust,ignore
/// let doc = BillingDocument::builder()
///     .meta(meta)
///     .tariff(&my_tariff, &usage)?
///     .build()?;
/// ```
///
/// # Example — from pre-computed positions
///
/// ```rust,ignore
/// let doc = BillingDocument::builder()
///     .meta(meta)
///     .positions(vec![item1, item2])
///     .extra_tax(Box::new(FixedRateTax::new("VAT", dec!(0.20)).unwrap()))
///     .build()?;
/// ```
#[derive(Default)]
pub struct BillingDocumentBuilder {
    meta: DocumentMeta,
    positions: Vec<LineItem>,
    tax_layers: Vec<Box<dyn TaxLayer>>,
    discount_layers: Vec<Box<dyn DiscountLayer>>,
}

impl BillingDocumentBuilder {
    /// Set document metadata.
    #[must_use]
    pub fn meta(mut self, meta: DocumentMeta) -> Self {
        self.meta = meta;
        self
    }

    /// Set the document currency (shorthand for setting `meta.currency`).
    ///
    /// Call this *after* [`meta`](Self::meta), which replaces the whole header.
    #[must_use]
    pub fn currency(mut self, currency: Currency) -> Self {
        self.meta.currency = currency;
        self
    }

    /// Load positions and layers from a [`crate::Tariff`] implementation.
    ///
    /// Replaces any previously set positions and layers.
    ///
    /// # Errors
    /// Returns `Err` if `tariff.line_items(usage)` fails, converted to `BillingError`
    /// via `T::Error: Into<BillingError>`.
    pub fn tariff<T: crate::tariff::Tariff>(
        mut self,
        tariff: &T,
        usage: &T::Usage,
    ) -> Result<Self, BillingError>
    where
        T::Error: Into<BillingError>,
    {
        self.positions = tariff.line_items(usage).map_err(Into::into)?;
        self.tax_layers = tariff.tax_layers();
        self.discount_layers = tariff.discount_layers();
        Ok(self)
    }

    /// Extend positions with pre-computed `LineItem`s.
    #[must_use]
    pub fn positions(mut self, positions: Vec<LineItem>) -> Self {
        self.positions.extend(positions);
        self
    }

    /// Append an extra tax layer.
    #[must_use]
    pub fn extra_tax(mut self, layer: Box<dyn TaxLayer>) -> Self {
        self.tax_layers.push(layer);
        self
    }

    /// Append an extra discount layer.
    #[must_use]
    pub fn extra_discount(mut self, layer: Box<dyn DiscountLayer>) -> Self {
        self.discount_layers.push(layer);
        self
    }

    /// Build the [`BillingDocument`].
    pub fn build(self) -> Result<BillingDocument, BillingError> {
        BillingDocument::from_positions(
            self.meta,
            self.positions,
            self.tax_layers,
            self.discount_layers,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tax::FixedRateTax;
    use rust_decimal::dec;

    fn simple_doc(amount: &str) -> BillingDocument {
        let pos = vec![
            LineItem::fixed("Test", Amount::parse(amount).unwrap())
                .build()
                .unwrap(),
        ];
        BillingDocument::from_positions(
            DocumentMeta {
                invoice_number: "INV-001".into(),
                ..Default::default()
            },
            pos,
            vec![],
            vec![],
        )
        .unwrap()
    }

    #[test]
    fn totals_with_tax() {
        let pos = vec![
            LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ];
        let taxes: Vec<Box<dyn TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)).unwrap())];
        let doc =
            BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
        assert_eq!(doc.net_total(), Amount::parse("100.00000").unwrap());
        assert_eq!(doc.tax_total(), Amount::parse("20.00000").unwrap());
        assert_eq!(doc.gross_total(), Amount::parse("120.00000").unwrap());
        doc.assert_valid();
    }

    /// Compound-tax correctness: the second tax layer must see the first
    /// layer's output in its base.
    ///
    /// Setup: net = 100.00, levy = 5% of net = 5.00, VAT = 19% of (net + levy).
    /// Correct:  VAT base = 105.00, VAT = 19.95, gross = 124.95
    /// Wrong:    VAT base = 100.00, VAT = 19.00, gross = 124.00  ← old bug
    #[test]
    fn compound_tax_accumulates_base() {
        use crate::tax::PercentageCharge;
        let pos = vec![
            LineItem::fixed("Net charge", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ];
        let taxes: Vec<Box<dyn TaxLayer>> = vec![
            // Layer 1: 5% levy on the net
            Box::new(PercentageCharge::new("Levy", dec!(0.05)).unwrap()),
            // Layer 2: 19% VAT — should see Net (100) + Levy (5) = 105
            Box::new(FixedRateTax::new("VAT", dec!(0.19)).unwrap()),
        ];
        let doc =
            BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();

        assert_eq!(doc.net_total(), Amount::parse("100.00000").unwrap());
        // Levy = 5.00000
        // VAT base = 100.00 + 5.00 = 105.00; VAT = 105.00 × 0.19 = 19.95000
        assert_eq!(doc.tax_total(), Amount::parse("24.95000").unwrap());
        assert_eq!(doc.gross_total(), Amount::parse("124.95000").unwrap());
        doc.assert_valid();
    }

    #[test]
    fn assert_valid_full_three_checks() {
        let doc = simple_doc("42.00000");
        doc.assert_valid();

        // Manually corrupt net_total — check 1 should fire via validate().
        let mut bad = doc.clone();
        bad.net_total = Amount::parse("99.00000").unwrap();
        let err = bad.validate().unwrap_err();
        assert!(matches!(
            err,
            crate::error::BillingError::ValidationFailed { .. }
        ));
        if let crate::error::BillingError::ValidationFailed { ref check, .. } = err {
            assert_eq!(check, "net_total");
        }
    }

    #[test]
    fn builder_from_positions() {
        let pos = vec![
            LineItem::fixed("Fee", Amount::parse("50.00000").unwrap())
                .build()
                .unwrap(),
        ];
        let doc = BillingDocument::builder()
            .meta(DocumentMeta {
                invoice_number: "B-001".into(),
                ..Default::default()
            })
            .positions(pos)
            .extra_tax(Box::new(FixedRateTax::new("VAT", dec!(0.20)).unwrap()))
            .build()
            .unwrap();
        assert_eq!(doc.gross_total(), Amount::parse("60.00000").unwrap());
    }

    #[test]
    fn with_extra_position_updates_totals() {
        let doc = simple_doc("100.00000");
        let extra = LineItem::fixed(
            "Minimum charge shortfall",
            Amount::parse("50.00000").unwrap(),
        )
        .build()
        .unwrap();
        let doc2 = doc.with_extra_position(extra).unwrap();
        assert_eq!(doc2.net_total(), Amount::parse("150.00000").unwrap());
    }

    #[test]
    fn assert_valid_passes() {
        simple_doc("42.00000").assert_valid();
    }
}
