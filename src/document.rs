//! [`BillingDocument`] — self-validating invoice with ordered positions + totals.
use crate::advance::{AdvancePayment, DocumentKind, Prepayment};
use crate::amount::{Amount, AmountScale};
use crate::currency::Currency;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::period::Period;
use crate::settlement::CashRounding;
use crate::tariff::Billing;
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
    ///
    /// This bucket mixes two things EN 16931 keeps apart — see
    /// [`vat_positions`](Self::vat_positions) and
    /// [`charge_positions`](Self::charge_positions) for the split.
    #[must_use]
    pub fn tax_positions(&self) -> &[LineItem] {
        &self.tax_positions
    }

    /// The tax positions that are **value added tax** — the ones whose layer
    /// contributed a VAT breakdown entry, marked with [`crate::tags::VAT`].
    ///
    /// These are the positions EN 16931 accounts for in **BT-110**, the VAT total,
    /// which BR-CO-14 requires to equal `Σ BT-117` — the sum of the breakdown's tax
    /// amounts, and nothing else.
    pub fn vat_positions(&self) -> impl Iterator<Item = &LineItem> + '_ {
        self.tax_positions
            .iter()
            .filter(|p| p.has_tag(crate::tags::VAT))
    }

    /// The tax positions that are **not** VAT: per-unit levies, commissions,
    /// surcharges. In EN 16931 these are document level charges (**BG-21**).
    ///
    /// A charge is not tax — it is part of what the customer is charged *for*, and
    /// therefore part of the taxable base: it contributes to BT-108, which feeds
    /// BT-109 (total without VAT). Putting it in BT-110 instead breaks BR-CO-14 on
    /// every document that has one.
    ///
    /// Each carries its own BT-102 / BT-103 in [`LineItem::vat`] (mandatory under
    /// BR-37) and its BT-100 / BT-101 / BT-105 in
    /// [`LineItem::allowance_charge`].
    pub fn charge_positions(&self) -> impl Iterator<Item = &LineItem> + '_ {
        self.tax_positions
            .iter()
            .filter(|p| !p.has_tag(crate::tags::VAT))
    }

    /// EN 16931 **BT-110** — the sum of the value-added-tax positions alone.
    ///
    /// Distinct from [`tax_total`](Self::tax_total), which also includes levies and
    /// commissions. On a document with no non-VAT layer the two are equal; where
    /// they differ, this is the one that belongs in BT-110, and
    /// [`charge_total`](Self::charge_total) accounts for the rest.
    ///
    /// ```rust
    /// use billing::prelude::*;
    /// use rust_decimal::dec;
    ///
    /// // Stromsteuer 2.05 ct/kWh (a charge), then 19 % MwSt on net + levy.
    /// let doc = BillingDocument::builder()
    ///     .currency(Currency::EUR)
    ///     .positions(vec![LineItem::for_usage(
    ///         "Arbeit",
    ///         Quantity::new(dec!(1000), "kWh"),
    ///         UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ///     ).build()?])
    ///     .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    ///     .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    ///     .build()?;
    ///
    /// // tax_total mixes both; only the VAT part is BT-110.
    /// // Levy = 1000 × 0.02050 = 20.50; VAT = (300.00 + 20.50) × 0.19 = 60.895.
    /// assert_eq!(doc.tax_total(),     Amount::parse("81.39500")?);
    /// assert_eq!(doc.charge_total()?, Amount::parse("20.50000")?);
    /// assert_eq!(doc.vat_total()?,    Amount::parse("60.89500")?);
    /// // BR-CO-14: BT-110 = Σ BT-117.
    /// assert_eq!(doc.vat_total()?, doc.tax_breakdown()[0].tax_amount);
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn vat_total(&self) -> Result<Amount<5>, BillingError> {
        Amount::checked_sum(self.vat_positions().map(|p| p.net_amount))
    }

    /// EN 16931 **BT-108** — the sum of the document level charges, i.e. every tax
    /// position that is not VAT. Part of the taxable base, not of BT-110.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn charge_total(&self) -> Result<Amount<5>, BillingError> {
        Amount::checked_sum(self.charge_positions().map(|p| p.net_amount))
    }
    /// Discount positions (always negative net amounts).
    #[must_use]
    pub fn discount_positions(&self) -> &[LineItem] {
        &self.discount_positions
    }

    /// Sum of the net positions **and** the discounts — i.e. `BT-106 − BT-107`.
    ///
    /// # This is not BT-109
    ///
    /// EN 16931 builds the total without VAT in three steps (**BR-CO-13**):
    ///
    /// ```text
    /// BT-109 = BT-106 (Σ line net amounts) − BT-107 (allowances) + BT-108 (charges)
    /// ```
    ///
    /// `net_total` covers only the first two. A document level **charge** — a
    /// per-unit levy, a commission — is produced by a [`crate::TaxLayer`] and
    /// therefore lands in `tax_positions`, not here, even though EN 16931 counts it
    /// inside the taxable base. The two are equal only on a document with no
    /// charges; [`taxable_total`](Self::taxable_total) is BT-109 in general, and
    /// [`line_total`](Self::line_total) is BT-106.
    pub fn net_total(&self) -> Amount<5> {
        self.net_total
    }

    /// EN 16931 **BT-106** — the sum of the invoice line net amounts (BT-131),
    /// before allowances and charges.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn line_total(&self) -> Result<Amount<5>, BillingError> {
        Amount::checked_sum(self.net_positions.iter().map(|p| p.net_amount))
    }

    /// EN 16931 **BT-109** — the invoice total amount without VAT.
    ///
    /// Implements **BR-CO-13** exactly:
    ///
    /// ```text
    /// BT-109 = BT-106 − BT-107 + BT-108
    ///        = line_total + discount_total + charge_total
    /// ```
    ///
    /// (`discount_total` is already negative here, where EN 16931's BT-107 is a
    /// positive magnitude subtracted.)
    ///
    /// This is the figure that pairs with [`vat_total`](Self::vat_total) under
    /// **BR-CO-15**: `BT-112 = BT-109 + BT-110`. Using
    /// [`net_total`](Self::net_total) in its place understates the taxable base by
    /// the charges, on exactly the documents where it matters — a levy-bearing
    /// utility invoice.
    ///
    /// ```rust
    /// use billing::prelude::*;
    /// use rust_decimal::dec;
    ///
    /// let doc = BillingDocument::builder()
    ///     .currency(Currency::EUR)
    ///     .positions(vec![LineItem::for_usage(
    ///         "Arbeit",
    ///         Quantity::new(dec!(1000), "kWh"),
    ///         UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ///     ).build()?])
    ///     .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    ///     .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    ///     .build()?;
    ///
    /// assert_eq!(doc.line_total()?,    Amount::parse("300.00000")?);  // BT-106
    /// assert_eq!(doc.net_total(),      Amount::parse("300.00000")?);  // BT-106 − BT-107
    /// assert_eq!(doc.charge_total()?,  Amount::parse("20.50000")?);   // BT-108
    /// assert_eq!(doc.taxable_total()?, Amount::parse("320.50000")?);  // BT-109 — the levy counts
    ///
    /// // BR-CO-15 pairs BT-109 with BT-110, not `net_total` with `tax_total`.
    /// assert_eq!(
    ///     doc.taxable_total()?.checked_add(doc.vat_total()?)?,
    ///     doc.gross_total(),
    /// );
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] on overflow.
    pub fn taxable_total(&self) -> Result<Amount<5>, BillingError> {
        self.net_total.checked_add(self.charge_total()?)
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

    /// Sum of all discount positions (always ≤ 0) — the negation of EN 16931
    /// **BT-107**, which states the same figure as a positive magnitude.
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

    /// Whether **every** monetary amount in this document fits `scale` decimals.
    ///
    /// The precondition for emitting the document into an interchange format that
    /// caps decimals on money — EN 16931 and its national CIUSes (XRechnung,
    /// Peppol BIS, ZUGFeRD/Factur-X) all cap at two. Covers every amount a format
    /// carries: each position (BT-131), the totals (BT-106 / BT-109 / BT-110 /
    /// BT-112), the paid amount and rounding amount (BT-113 / BT-114), the amount
    /// due (BT-115), each VAT breakdown entry's base and tax (BT-116 / BT-117), and
    /// each itemised advance payment.
    ///
    /// A document built with [`BillingDocumentBuilder::amount_scale`] satisfies this
    /// by construction. For any other document this is a genuine question, because
    /// `quantity × unit_price` routinely produces more decimals than two:
    /// `1234.567 kWh × 0.28901 EUR/kWh = 356.80221`.
    ///
    /// Do **not** respond to a `false` here by rounding the amounts on the way out —
    /// that breaks the totals identities the same format also checks. Rebuild with
    /// `amount_scale` instead; [`AmountScale`] explains why.
    ///
    /// ```rust
    /// # use billing::{BillingDocument, DocumentMeta, LineItem, Amount, Currency,
    /// #               AmountScale, Quantity, UnitPrice};
    /// # use rust_decimal::dec;
    /// let positions = || vec![LineItem::for_usage(
    ///     "Arbeit",
    ///     Quantity::new(dec!(1234.567), "kWh"),
    ///     UnitPrice::new(dec!(0.28901), "EUR/kWh"),
    /// ).build().unwrap()];
    ///
    /// let raw = BillingDocument::builder().currency(Currency::EUR)
    ///     .positions(positions()).build()?;
    /// assert!(!raw.fits_amount_scale(2)); // 356.80221 — not emittable as EN 16931
    ///
    /// let scaled = BillingDocument::builder().currency(Currency::EUR)
    ///     .amount_scale(AmountScale::EN16931)
    ///     .positions(positions()).build()?;
    /// assert!(scaled.fits_amount_scale(2));
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn fits_amount_scale(&self, scale: u8) -> bool {
        self.amount_scale_violation(scale).is_none()
    }

    /// The first amount that does not fit `scale` decimals, as
    /// `(what, value)` — for reporting which field to look at.
    ///
    /// Returns `None` exactly when [`BillingDocument::fits_amount_scale`] is `true`.
    #[must_use]
    pub fn amount_scale_violation(&self, scale: u8) -> Option<(String, Amount<5>)> {
        let mut checks: Vec<(String, Amount<5>)> = Vec::new();
        for (i, p) in self.all_positions().enumerate() {
            checks.push((format!("position[{i}] {:?}", p.description), p.net_amount));
            // BR-DEC-02 / BR-DEC-06 cap the allowance and charge *base* amounts at
            // two decimals in their own right, independently of the amount they
            // produce.
            if let Some(base) = p.allowance_charge.as_ref().and_then(|a| a.base_amount) {
                checks.push((
                    format!(
                        "position[{i}] {:?} allowance/charge base (BT-93/BT-100)",
                        p.description
                    ),
                    base,
                ));
            }
        }
        checks.extend([
            // `net_total` is BT-106 − BT-107, which is BT-109 only when the
            // document carries no charges — see `BillingDocument::net_total`.
            ("net_total".to_owned(), self.net_total),
            ("tax_total".to_owned(), self.tax_total),
            ("gross_total (BT-112)".to_owned(), self.gross_total),
            // BT-107 is a positive magnitude in EN 16931; this is its negation.
            ("discount_total (-BT-107)".to_owned(), self.discount_total),
            ("prepaid (BT-113)".to_owned(), self.prepaid),
            ("rounding (BT-114)".to_owned(), self.rounding),
        ]);
        // BT-106, BT-108, BT-109 and BT-110 are derived rather than stored, and
        // each is capped in its own right (BR-DEC-09, BR-DEC-10, BR-DEC-12,
        // BR-DEC-13). They are sums of already-checked leaves, so they cannot
        // introduce a new violation — but naming them keeps the diagnostic honest
        // about which field a consumer would actually emit.
        for (label, derived) in [
            ("line_total (BT-106)", self.line_total()),
            ("charge_total (BT-108)", self.charge_total()),
            ("taxable_total (BT-109)", self.taxable_total()),
            ("vat_total (BT-110)", self.vat_total()),
        ] {
            if let Ok(amount) = derived {
                checks.push((label.to_owned(), amount));
            }
        }
        for (i, e) in self.tax_breakdown.iter().enumerate() {
            checks.push((
                format!("tax_breakdown[{i}].taxable_base (BT-116)"),
                e.taxable_base,
            ));
            checks.push((
                format!("tax_breakdown[{i}].tax_amount (BT-117)"),
                e.tax_amount,
            ));
        }
        for (i, a) in self.prepayment.advances().iter().enumerate() {
            checks.push((format!("advance[{i}].gross"), a.gross()));
        }
        checks
            .into_iter()
            .find(|(_, amount)| !amount.fits_scale(scale))
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
    ///
    /// Amounts keep their full 5-decimal precision. To assemble a document whose
    /// every amount fits an interchange format's decimal limit, use
    /// [`BillingDocumentBuilder::amount_scale`].
    pub fn from_positions(
        meta: DocumentMeta,
        positions: Vec<LineItem>,
        tax_layers: Vec<Box<dyn TaxLayer>>,
        discounts: Vec<Box<dyn DiscountLayer>>,
    ) -> Result<Self, BillingError> {
        Self::assemble(meta, positions, tax_layers, discounts, None)
    }

    /// Reduce one position's amount to `scale`, rounding the **exact** product once
    /// where the position was derived from `quantity × unit_price`.
    ///
    /// [`crate::LineItemBuilder::build`] rounds that product to the engine's five
    /// decimals. Reducing the stored result again would round twice and can land a
    /// whole minor unit away from rounding the true product directly — the same
    /// defect that made the declared VAT disagree with the VAT breakdown.
    ///
    /// It also pushes the line off `BT-131 = BT-129 × (BT-146 / BT-149) + BG-28 −
    /// BG-27`. EN 16931 itself does not check that — there is no such rule in the
    /// CEN abstract model — but **Peppol** does: `PEPPOL-EN16931-R120`, flagged
    /// **fatal**, with a ±0.02 tolerance from Peppol's `u:slack` helper. Every
    /// Peppol access point therefore rejects a line that fails it. Rounding the
    /// exact product once leaves a residual of at most half a minor unit,
    /// comfortably inside 0.02; rounding twice can exceed it.
    ///
    /// The stored amount is only bypassed when it still **agrees** with the product
    /// at the engine's precision. A position carrying an explicit `fixed_amount`
    /// alongside a quantity is deliberately not `quantity × unit_price`, and that
    /// stated amount is authoritative — it is reduced as-is.
    fn reduce_position(item: &LineItem, scale: AmountScale) -> Result<Amount<5>, BillingError> {
        let (Some(qty), Some(price)) = (item.quantity.as_ref(), item.unit_price.as_ref()) else {
            return scale.apply(item.net_amount);
        };
        let Some(product) = qty.value.checked_mul(price.value) else {
            return scale.apply(item.net_amount);
        };
        // Reconstruct what `build()` would have stored, sign convention included.
        let engine_value = {
            let rounded = Amount::<5>::from_decimal_rounded(
                product,
                crate::amount::RoundingStrategy::MidpointAwayFromZero,
            );
            match rounded {
                Ok(v) if item.is_credit() && v.is_positive() => v.checked_neg()?,
                Ok(v) => v,
                Err(_) => return scale.apply(item.net_amount),
            }
        };
        if engine_value != item.net_amount {
            // A stated amount that is not the product — honour it verbatim.
            return scale.apply(item.net_amount);
        }
        let reduced = scale.apply_decimal(product)?;
        if item.is_credit() && reduced.is_positive() {
            reduced.checked_neg()
        } else {
            Ok(reduced)
        }
    }

    /// Reduce an allowance/charge base amount (BT-93 / BT-100) to `scale`.
    ///
    /// Capped in its own right by BR-DEC-02 and BR-DEC-06, so it cannot be left at
    /// the engine's five decimals when the amount it explains has been reduced to
    /// two. The percentage is a rate and is not a monetary amount, so no BR-DEC
    /// rule applies to it.
    fn reduce_basis(
        ac: Option<crate::line_item::AllowanceCharge>,
        scale: AmountScale,
    ) -> Result<Option<crate::line_item::AllowanceCharge>, BillingError> {
        match ac {
            None => Ok(None),
            Some(mut ac) => {
                if let Some(base) = ac.base_amount {
                    ac.base_amount = Some(scale.apply(base)?);
                }
                Ok(Some(ac))
            }
        }
    }

    /// Shared assembly path for [`BillingDocument::from_positions`] and the
    /// builder's scaled variant.
    ///
    /// When `scale` is set, every **leaf** amount — each incoming position, each
    /// discount-layer output, each tax-layer output and each VAT breakdown entry —
    /// is reduced to the requested precision *before* any aggregate is computed.
    /// Every total is then a sum of already-reduced values, so it lands on the same
    /// precision exactly, and the totals identities hold at that precision rather
    /// than only at 5 decimals. Rounding the finished totals instead would break
    /// them; see [`AmountScale`] for the worked counterexamples.
    fn assemble(
        meta: DocumentMeta,
        positions: Vec<LineItem>,
        tax_layers: Vec<Box<dyn TaxLayer>>,
        discounts: Vec<Box<dyn DiscountLayer>>,
        scale: Option<AmountScale>,
    ) -> Result<Self, BillingError> {
        // `LineItem` has public fields, so a caller can hand us a position with an
        // empty description or a negative quantity. Check it here: the type doc
        // promises a document from this constructor satisfies every invariant, and
        // check 11 of `validate()` would otherwise reject what we just returned.
        for item in &positions {
            item.validate()?;
        }

        // Reduce the leaves first. Every layer below then computes over amounts that
        // are already at the target precision, which keeps a layer's base equal to
        // the base that will be reported in the VAT breakdown.
        let positions = match scale {
            None => positions,
            Some(s) => positions
                .into_iter()
                .map(|mut p| {
                    p.net_amount = Self::reduce_position(&p, s)?;
                    Ok(p)
                })
                .collect::<Result<Vec<_>, BillingError>>()?,
        };

        let discount_positions: Vec<LineItem> = discounts
            .iter()
            .map(|d| {
                let mut item = d.compute(&positions)?;
                if let Some(s) = scale {
                    item.net_amount = s.apply(item.net_amount)?;
                    // BR-DEC-02 caps BT-93 at the same precision as BT-92. A layer
                    // that computed its base over already-reduced positions is
                    // usually there already; a third-party one need not be.
                    item.allowance_charge = Self::reduce_basis(item.allowance_charge, s)?;
                }
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
                // BT-95/BT-96 and BT-98: the layer's declaration wins here, and a
                // VAT layer covering this allowance is reconciled against it below.
                if item.vat.is_none() {
                    item.vat = d.vat();
                }
                if item.allowance_charge.is_none() {
                    item.allowance_charge = d.allowance_charge();
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
            let mut scaled_tax = None;
            let breakdown = t.breakdown(&accumulated)?;
            let is_vat = breakdown.is_some();

            // Attribute this layer's (category, rate) to every position in its base
            // — EN 16931 wants that per line (BT-151/BT-152), per allowance
            // (BT-95/BT-96) and per charge (BT-102/BT-103), and BR-S-08 checks the
            // breakdown against it.
            //
            // Derived from the same `accumulated` slice `breakdown` and `compute`
            // both see, so the attribution names exactly the base that was taxed.
            if let Some(entry) = &breakdown {
                let vat = crate::vat::LineVat::new(entry.category, entry.rate)?;
                for item in accumulated.iter_mut().filter(|i| t.covers(i)) {
                    // VAT charged on VAT has no EN 16931 representation at all: the
                    // earlier layer's output is BG-23, which is not an invoice line
                    // (BT-131), a charge (BT-99) or an allowance (BT-92), so it
                    // cannot appear in any group's BT-116 under BR-S-08. A levy or
                    // commission compounding into a VAT base is the legitimate
                    // version of this, and is unaffected — those are BG-21 charges.
                    if item.has_tag(crate::tags::VAT) {
                        return Err(BillingError::LayerError {
                            reason: format!(
                                "tax layer {:?} would charge VAT on the VAT position {:?}; \
                                 EN 16931 has no way to express that (a VAT breakdown group \
                                 is not part of another group's taxable base). Restrict the \
                                 layer's base with a tag.",
                                t.name(),
                                item.description
                            ),
                        });
                    }
                    match item.vat {
                        // Two VAT layers claiming one position means it is taxed
                        // twice: BR-S-08 cannot hold for either group, and the
                        // document used to assemble silently. The same check catches
                        // a caller-declared BT-151, or a charge layer's own `vat()`,
                        // that contradicts the layer actually taxing the position.
                        Some(prior) if prior != vat => {
                            return Err(BillingError::LayerError {
                                reason: format!(
                                    "tax layer {:?} attributes VAT {vat} to position {:?}, \
                                     which already carries {prior}; a position belongs to \
                                     exactly one VAT group (BR-S-08)",
                                    t.name(),
                                    item.description
                                ),
                            });
                        }
                        Some(_) => {}
                        None => item.vat = Some(vat),
                    }
                }
            }

            if let Some(mut entry) = breakdown {
                if let Some(s) = scale {
                    // Reduce the base first, then derive the tax from the *reduced*
                    // base in a SINGLE rounding of the exact product. EN 16931
                    // BR-CO-17 defines `BT-117 = BT-116 × rate` rounded to the
                    // reported precision, and a validator recomputes it that way —
                    // so rounding an already-rounded tax would answer a different
                    // question and can differ by a whole minor unit.
                    entry.taxable_base = s.apply(entry.taxable_base)?;
                    entry.tax_amount = s.apply_decimal(
                        entry
                            .taxable_base
                            .into_decimal()
                            .checked_mul(entry.rate)
                            .ok_or(BillingError::MonetaryOverflow {
                                precision: 5,
                                input_value: None,
                            })?,
                    )?;
                    scaled_tax = Some(entry.tax_amount);
                }
                entry.validate()?;
                breakdown_entries.push(entry);
            }
            let mut item = t.compute(&accumulated)?;
            if let Some(s) = scale {
                // A layer that reports a VAT breakdown has just had its tax
                // recomputed above, from the reduced base, in one rounding. Its
                // charged position must carry that same number: EN 16931 BR-CO-14
                // requires `BT-110 = Σ BT-117`, and a `TaxLayer` is contractually
                // computing `breakdown` and `compute` over the same base.
                //
                // Reducing the layer's own output independently would round twice
                // by a different route — `TaxLayer::compute` rounds its product to
                // the engine's 5 decimals with commercial rounding before this code
                // ever sees it — and the two results diverge whenever the scale uses
                // a different strategy, or the product carries more than 5 decimals.
                // That produced a document whose declared VAT and whose VAT
                // breakdown disagreed.
                item.net_amount = match scaled_tax {
                    Some(tax) => tax,
                    // A non-VAT layer (a per-unit excise, a commission) reports no
                    // breakdown. Reduce its output the same way a position is
                    // reduced, so a levy carrying `quantity × unit_price` is also
                    // rounded once from its exact product.
                    None => Self::reduce_position(&item, s)?,
                };
                // BR-DEC-06 caps BT-100 exactly as BR-DEC-02 caps BT-93.
                item.allowance_charge = Self::reduce_basis(item.allowance_charge, s)?;
            }
            // Every `TaxLayer` output carries `tags::TAX`, and one that contributed
            // a VAT breakdown entry also carries `tags::VAT`. Stamped here rather
            // than left to each layer so the classification is **total**: a
            // third-party layer is labelled as accurately as a built-in one, and a
            // consumer can split BT-110 (value added tax) from BG-21 (document
            // level charges) without guessing from layer-specific tags.
            for tag in [Some(crate::tags::TAX), is_vat.then_some(crate::tags::VAT)]
                .into_iter()
                .flatten()
            {
                if !item.has_tag(tag) {
                    item.tags.push(tag.to_owned());
                }
            }
            // A charge (BG-21) sits inside the taxable base and needs its own
            // BT-102 / BT-105. VAT itself is BG-23 and carries neither.
            if !is_vat {
                if item.vat.is_none() {
                    item.vat = t.vat();
                }
                if item.allowance_charge.is_none() {
                    item.allowance_charge = t.allowance_charge();
                }
            }
            accumulated.push(item.clone());
            tax_positions.push(item);
        }
        let tax_breakdown = merge_breakdown(breakdown_entries)?;

        let tax_total = Amount::checked_sum(tax_positions.iter().map(|p| p.net_amount))?;
        let gross_total = net_total.checked_add(tax_total)?;
        let discount_total = Amount::checked_sum(discount_positions.iter().map(|p| p.net_amount))?;

        // Copy the attributions back out of `accumulated`, which is where the tax
        // layers stamped them. Only `vat` moves: for the net and discount positions
        // `accumulated` holds clones taken before the layers ran, so nothing else
        // there is newer, and each tax position was cloned into it fully formed.
        let mut attributed = accumulated.into_iter().map(|i| i.vat);
        let mut positions = positions;
        let mut discount_positions = discount_positions;
        for item in positions
            .iter_mut()
            .chain(discount_positions.iter_mut())
            .chain(tax_positions.iter_mut())
        {
            // The zip is index-aligned by construction: `accumulated` was built as
            // positions ++ discounts, then extended with each tax position in order.
            if let Some(vat) = attributed.next() {
                item.vat = vat;
            }
        }

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
    /// How urgent that advice is depends on the target format:
    ///
    /// | Target | Where the per-advance tax goes |
    /// |---|---|
    /// | ZUGFeRD / Factur-X **EXTENDED** | `BG-X-46`, the only place it fits |
    /// | ZUGFeRD **BASIC** / **EN 16931**, XRechnung, Peppol BIS | **nowhere** |
    ///
    /// A consumer targeting XRechnung or Peppol BIS will emit
    /// [`prepaid`](Self::prepaid) as BT-113 and **silently drop the tax attached to
    /// each advance** — which is exactly the §14c Abs. 1 UStG double-taxation
    /// scenario [`crate::advance`] exists to prevent. Against those formats, a
    /// residual invoice is not a preference but the only correct construction.
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
    /// # The type code is set for you
    ///
    /// [`DocumentMeta::kind`] (BT-3) is forced to a credit-note code, because
    /// getting it wrong is fatal rather than cosmetic. `BR-CL-01` polices **two
    /// disjoint** UNTDID 1001 lists — one for `cbc:InvoiceTypeCode`, one for
    /// `cbc:CreditNoteTypeCode` — and `380` appears only in the first. A reversal
    /// built from `DocumentMeta { .. ..Default::default() }` would otherwise carry
    /// `380` on a document with negative totals, which no validator accepts as
    /// either kind.
    ///
    /// If `meta.kind` already names a credit-note code it is kept; anything else
    /// becomes [`DocumentKind::CreditNote`] (`381`). See
    /// [`DocumentKind::is_credit_note`].
    ///
    /// ```rust
    /// use billing::{Amount, BillingDocument, Currency, DocumentKind, DocumentMeta, LineItem};
    ///
    /// let inv = BillingDocument::from_positions(
    ///     DocumentMeta { invoice_number: "INV-1".into(), currency: Currency::EUR,
    ///                    ..Default::default() },
    ///     vec![LineItem::fixed("Service", Amount::parse("100.00000").unwrap()).build().unwrap()],
    ///     vec![], vec![],
    /// ).unwrap();
    /// assert_eq!(inv.meta.kind, DocumentKind::CommercialInvoice); // 380
    ///
    /// let credit = inv.reverse(DocumentMeta {
    ///     invoice_number: "CN-1".into(), currency: Currency::EUR, ..Default::default()
    /// }).unwrap();
    ///
    /// assert_eq!(credit.net_total(), Amount::parse("-100.00000").unwrap());
    /// // …and BT-3 is 381, not the 380 that was passed in.
    /// assert_eq!(credit.meta.kind, DocumentKind::CreditNote);
    /// assert!(credit.meta.kind.is_credit_note());
    /// credit.assert_valid();
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if any amount is `Amount::MIN`, which has
    /// no positive counterpart.
    pub fn reverse(&self, meta: DocumentMeta) -> Result<Self, BillingError> {
        // BT-3 must come from the credit-note code list — see the doc above.
        let meta = DocumentMeta {
            kind: if meta.kind.is_credit_note() {
                meta.kind
            } else {
                DocumentKind::CreditNote
            },
            ..meta
        };
        fn flip(items: &[LineItem]) -> Result<Vec<LineItem>, BillingError> {
            items
                .iter()
                .map(|p| {
                    let mut out = p.clone();
                    out.net_amount = p.net_amount.checked_neg()?;
                    // BT-93 / BT-100 must follow the amount, or PEPPOL-EN16931-R040
                    // fails on the credit note by twice the allowance.
                    if let Some(ac) = out.allowance_charge.as_ref() {
                        out.allowance_charge = Some(ac.negated()?);
                    }
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
                    exemption_reason_code: e.exemption_reason_code.clone(),
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
    /// Thirteen invariants are checked (all exact — no tolerance):
    /// 1. `Σ(net_positions + discount_positions) == net_total`
    /// 2. `Σ(tax_positions) == tax_total`
    /// 3. `net_total + tax_total == gross_total`
    /// 4. `Σ(discount_positions) == discount_total`
    /// 5. every VAT breakdown entry is category-consistent, its tax matches
    ///    `base × rate` within EN 16931's tolerance (BR-CO-17), and no
    ///    `(category, rate)` group appears twice — see
    ///    [`TaxBreakdownEntry::group_key`] for the rules that force the merge
    ///    (BR-S-08 and siblings for the taxed categories, BR-Z-01 and siblings for
    ///    the zero-tax ones)
    /// 6. `prepaid >= 0`
    /// 7. `rounding` matches the recorded cash-rounding rule, if any
    /// 8. `Σ(tax_breakdown)` is a component of `tax_total` (same sign, no larger)
    /// 9. no discount position is positive
    /// 10. `prepaid` equals `prepayment.total()`
    /// 11. every position satisfies [`LineItem::validate`]
    /// 12. a document that charges VAT has a VAT breakdown, and a declared
    ///     breakdown has a VAT position behind it (BR-CO-18)
    /// 13. a "not subject to VAT" (`O`) breakdown group is the only group (BR-O-11)
    ///
    /// EN 16931's **BR-S-08** — the VAT breakdown agrees with the per-position
    /// attribution — is deliberately *not* among them, because
    /// [`crate::AllocationRule`] cannot preserve it exactly: splitting positions
    /// and the breakdown each with their own penny correction can leave the two a
    /// minor unit apart. Check it explicitly with
    /// [`verify_vat_attribution`](Self::verify_vat_attribution) on a document you
    /// are about to emit.
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
        // and no two entries share a (category, rate) group.
        //
        // BR-S-08 / BR-AF-08 / BR-AG-08 require the taxable amount to hold per
        // distinct rate, and BR-Z-01 / BR-E-01 / BR-AE-01 / BR-IC-01 / BR-G-01 /
        // BR-O-01 require *exactly one* breakdown line for each zero-tax category.
        // Both are only satisfiable if entries sharing a (category, rate) group are
        // merged — see `TaxBreakdownEntry::group_key`.
        //
        // (Not BR-CO-18, which says an invoice shall have at least one BG-23 — that
        // is check 12 below.)
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

        // Check 9: a discount position that moves the total the SAME way the lines
        // do is a surcharge, not a discount. The `DiscountLayer` docs promise a
        // credit; a third-party implementation returning a debit would otherwise
        // pass unnoticed.
        //
        // The test is relative to the document's own direction, not to zero. On a
        // credit note every amount is negated, so its allowances are positive — and
        // an absolute "discounts <= 0" rule made `reverse()` produce a document
        // that failed its own `assert_valid()` whenever the original had any
        // discount at all.
        let lines = self.line_total()?;
        let reversed = lines.is_negative();
        if let Some(bad) = self
            .discount_positions
            .iter()
            .find(|p| p.net_amount.is_positive() != reversed && !p.net_amount.is_zero())
        {
            return Err(BillingError::ValidationFailed {
                check: "discount_positions".into(),
                actual: format!("{:?} = {}", bad.description, bad.net_amount),
                expected: if reversed {
                    "every discount position >= 0 on a reversed document".into()
                } else {
                    "every discount position <= 0".into()
                },
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

        // Check 12 — BR-CO-18, as actually written: "An Invoice shall at least have
        // one VAT breakdown group (BG-23)."
        //
        // The engine cannot demand a BG-23 unconditionally: `billing` bills things
        // that are not invoices at all, and a document with no VAT layer is a
        // legitimate state. What it *can* refuse is the incoherent one — VAT
        // charged with nothing to account for it, which is the shape BR-CO-18
        // exists to catch. The converse is checked too: a breakdown with no VAT
        // position behind it means the tax was declared but never charged.
        let charges_vat = self.vat_positions().next().is_some();
        if charges_vat != !self.tax_breakdown.is_empty() {
            let (actual, expected) = if charges_vat {
                (
                    "VAT positions but no VAT breakdown".to_owned(),
                    "at least one VAT breakdown group (BG-23), per BR-CO-18".to_owned(),
                )
            } else {
                (
                    format!(
                        "{} VAT breakdown entries but no VAT position",
                        self.tax_breakdown.len()
                    ),
                    "a VAT position for the declared breakdown".to_owned(),
                )
            };
            return Err(BillingError::ValidationFailed {
                check: "vat_breakdown_presence".into(),
                actual,
                expected,
            });
        }

        // Check 13 — BR-O-11: "An Invoice that contains a VAT breakdown group
        // (BG-23) with a VAT category code (BT-118) 'Not subject to VAT' shall not
        // contain other VAT breakdown groups (BG-23)."
        //
        // `O` means the whole transaction is outside the scope of VAT, so it cannot
        // coexist with a group that is inside it. Checked here rather than at
        // emission because it is a property of the breakdown alone, which the
        // engine owns — and because `merge_period_documents` can otherwise
        // manufacture the combination out of two individually lawful documents.
        if self.tax_breakdown.len() > 1 {
            if let Some(o) = self
                .tax_breakdown
                .iter()
                .find(|e| e.category == crate::TaxCategory::OutOfScope)
            {
                return Err(BillingError::ValidationFailed {
                    check: "tax_breakdown".into(),
                    actual: format!(
                        "category {} alongside {} other breakdown group(s)",
                        o.category,
                        self.tax_breakdown.len() - 1
                    ),
                    expected: "a 'not subject to VAT' (O) breakdown to be the only group (BR-O-11)"
                        .into(),
                });
            }
        }

        Ok(())
    }

    /// Check the VAT breakdown against the per-position attribution — EN 16931
    /// **BR-S-08** and its per-category siblings.
    ///
    /// For each `(category, rate)` group, the breakdown's taxable amount (BT-116)
    /// must equal the sum of the invoice line net amounts (BT-131) **plus** the
    /// document level charges (BT-99) **minus** the document level allowances
    /// (BT-92) carrying that same pair. In this crate's terms: the net positions,
    /// the non-VAT tax positions and the discount positions whose
    /// [`LineItem::vat`] names the group — discounts already being negative, so the
    /// sum is over all three buckets directly.
    ///
    /// Positions with no attribution at all are ignored, so a document assembled
    /// without VAT layers passes vacuously. Positions tagged [`crate::tags::VAT`]
    /// are excluded: they *are* the BG-23 the identity is being checked against.
    ///
    /// This is separate from [`validate`](Self::validate) because
    /// [`crate::AllocationRule`] cannot preserve it — it splits the positions and
    /// the breakdown with independent penny corrections, which can leave the two a
    /// minor unit apart while both remain internally exact. Run it on the document
    /// you are about to emit, not on every intermediate one.
    ///
    /// ```rust
    /// use billing::prelude::*;
    /// use rust_decimal::dec;
    ///
    /// let doc = BillingDocument::builder()
    ///     .currency(Currency::EUR)
    ///     .amount_scale(AmountScale::EN16931)
    ///     .positions(vec![
    ///         LineItem::fixed("Beratung", Amount::parse("400.00000")?).tag("full").build()?,
    ///         LineItem::fixed("Fachbuch", Amount::parse("100.00000")?).tag("reduced").build()?,
    ///     ])
    ///     .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("full").boxed())
    ///     .extra_tax(FixedRateTax::new("MwSt", dec!(0.07))?.with_tag("reduced").boxed())
    ///     .build()?;
    ///
    /// // Each line was attributed to the layer that taxed it …
    /// assert_eq!(doc.net_positions()[0].vat.unwrap().rate, dec!(0.19));
    /// assert_eq!(doc.net_positions()[1].vat.unwrap().rate, dec!(0.07));
    /// // … and the breakdown adds up to those lines.
    /// doc.verify_vat_attribution()?;
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    ///
    /// It additionally checks the two identities that only become decidable once
    /// the attribution exists:
    ///
    /// - **BR-CO-14** — `BT-110 = Σ BT-117`, i.e. [`vat_total`](Self::vat_total)
    ///   equals the breakdown's tax. [`validate`](Self::validate) can only assert
    ///   the weaker "component of `tax_total`", because it must also hold for an
    ///   allocated document.
    /// - **BR-O-12 / BR-O-13 / BR-O-14** — where the breakdown is "not subject to
    ///   VAT" (`O`), no line, allowance or charge may carry any other category.
    ///
    /// # Errors
    /// - [`BillingError::ValidationFailed`] if a group's attributed positions do
    ///   not sum to its taxable base, if a position names a group the breakdown
    ///   does not contain, if BT-110 disagrees with `Σ BT-117`, or if an `O`
    ///   document carries a position in another category.
    /// - [`BillingError::MonetaryOverflow`] on overflow.
    pub fn verify_vat_attribution(&self) -> Result<(), BillingError> {
        let attributed = self
            .all_positions()
            .filter(|p| !p.has_tag(crate::tags::VAT))
            .filter_map(|p| p.vat.map(|v| (v.group_key(), p.net_amount)));

        let mut sums: Vec<((crate::TaxCategory, rust_decimal::Decimal), Amount<5>)> = Vec::new();
        for (key, amount) in attributed {
            match sums.iter_mut().find(|(k, _)| *k == key) {
                Some((_, total)) => *total = total.checked_add(amount)?,
                None => sums.push((key, amount)),
            }
        }

        for (key, total) in &sums {
            let entry = self
                .tax_breakdown
                .iter()
                .find(|e| e.group_key() == *key)
                .ok_or_else(|| BillingError::ValidationFailed {
                    check: "vat_attribution".into(),
                    actual: format!(
                        "positions totalling {total} attributed to ({}, {})",
                        key.0, key.1
                    ),
                    expected: "a VAT breakdown entry for that group".into(),
                })?;
            if entry.taxable_base != *total {
                return Err(BillingError::ValidationFailed {
                    check: "vat_attribution".into(),
                    actual: format!(
                        "positions attributed to ({}, {}) sum to {total}",
                        key.0, key.1
                    ),
                    expected: format!("the group's taxable base {} (BR-S-08)", entry.taxable_base),
                });
            }
        }

        // BR-O-12 / BR-O-13 / BR-O-14: an "outside the scope of VAT" breakdown
        // admits no line, allowance or charge in any other category. BR-O-11
        // (check 13 of `validate`) has already established that `O` is then the
        // only group, so any other attributed group is a violation.
        if self
            .tax_breakdown
            .iter()
            .any(|e| e.category == crate::TaxCategory::OutOfScope)
        {
            if let Some((key, _)) = sums
                .iter()
                .find(|((c, _), _)| *c != crate::TaxCategory::OutOfScope)
            {
                return Err(BillingError::ValidationFailed {
                    check: "vat_attribution".into(),
                    actual: format!("a position in category {} on an 'O' document", key.0),
                    expected: "every line, allowance and charge to be 'not subject to VAT' \
                               (BR-O-12 / BR-O-13 / BR-O-14)"
                        .into(),
                });
            }
        }

        // BR-CO-14: BT-110 = Σ BT-117, exactly. `validate` cannot demand this — an
        // allocated document splits the VAT positions and the breakdown with
        // independent penny corrections — but a document about to be emitted must
        // satisfy it, because a validator recomputes the sum.
        let breakdown_tax = Amount::checked_sum(self.tax_breakdown.iter().map(|e| e.tax_amount))?;
        let vat_total = self.vat_total()?;
        if vat_total != breakdown_tax {
            return Err(BillingError::ValidationFailed {
                check: "vat_total".into(),
                actual: format!("VAT positions sum to {vat_total}"),
                expected: format!("the breakdown's tax {breakdown_tax} (BR-CO-14)"),
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
    /// Panics if any of the thirteen invariants is violated.
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
/// EN 16931 permits exactly one breakdown line per distinct pair — BR-S-08,
/// BR-AF-08 and BR-AG-08 for the taxed categories, BR-Z-01 and its siblings for
/// the zero-tax ones ([`TaxBreakdownEntry::group_key`] has the wording) — so two
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
            // BT-121 is subject to the same argument as BT-120: one code per
            // breakdown line, so two different VATEX codes cannot be collapsed.
            match (
                &existing.exemption_reason_code,
                &entry.exemption_reason_code,
            ) {
                (Some(a), Some(b)) if a != b => {
                    return Err(BillingError::InvalidInput {
                        reason: format!(
                            "VAT breakdown group ({}, {}) has conflicting exemption reason \
                             codes: {a:?} and {b:?}",
                            key.0, key.1
                        ),
                    });
                }
                (None, Some(_)) => {
                    existing.exemption_reason_code = entry.exemption_reason_code;
                }
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
    amount_scale: Option<AmountScale>,
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

    /// Load positions and layers from a [`crate::Tariff`] that always bills.
    ///
    /// Replaces any previously set positions and layers.
    ///
    /// Bounded on `T::NotBillable = Infallible`. For a tariff that can decline to
    /// bill, use [`try_tariff`](Self::try_tariff) — there is no way to represent a
    /// "nothing to bill, because X" outcome as a builder, so the bound forces the
    /// caller to handle it rather than have the reason silently dropped.
    ///
    /// # Errors
    /// Returns `Err` if `tariff.line_items(usage)` fails, converted to `BillingError`
    /// via `T::Error: Into<BillingError>`.
    pub fn tariff<T>(self, tariff: &T, usage: &T::Usage) -> Result<Self, BillingError>
    where
        T: crate::tariff::Tariff<NotBillable = std::convert::Infallible>,
        T::Error: Into<BillingError>,
    {
        Ok(self.try_tariff(tariff, usage)?.into_inner())
    }

    /// Load positions and layers from a [`crate::Tariff`], propagating a
    /// "nothing to bill" outcome.
    ///
    /// Replaces any previously set positions and layers. Returns
    /// [`Billing::NotBillable`] — carrying the tariff's own reason — when the tariff
    /// declines to bill; the builder is then not returned, because there is no
    /// document to assemble.
    ///
    /// ```rust
    /// # use billing::{BillingDocument, Billing, DocumentMeta, Tariff, Positions, LineItem, Amount};
    /// # use std::convert::Infallible;
    /// # #[derive(Debug)] struct NoData;
    /// # impl std::fmt::Display for NoData {
    /// #     fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str("no data") }
    /// # }
    /// # struct T2;
    /// # impl Tariff for T2 {
    /// #     type Usage = (); type Error = Infallible; type NotBillable = NoData;
    /// #     fn line_items(&self, _: &()) -> Result<Positions<NoData>, Infallible> {
    /// #         Ok(Billing::NotBillable(NoData))
    /// #     }
    /// # }
    /// let outcome = BillingDocument::builder()
    ///     .meta(DocumentMeta::default())
    ///     .try_tariff(&T2, &())?;
    ///
    /// let doc = match outcome {
    ///     Billing::Billable(builder) => Some(builder.build()?),
    ///     // The reason is available here, instead of appearing as an empty document.
    ///     Billing::NotBillable(reason) => { eprintln!("skipped: {reason}"); None }
    /// };
    /// assert!(doc.is_none());
    /// # Ok::<(), billing::BillingError>(())
    /// ```
    ///
    /// # Errors
    /// Returns `Err` if `tariff.line_items(usage)` fails, converted to `BillingError`
    /// via `T::Error: Into<BillingError>`.
    pub fn try_tariff<T: crate::tariff::Tariff>(
        mut self,
        tariff: &T,
        usage: &T::Usage,
    ) -> Result<Billing<Self, T::NotBillable>, BillingError>
    where
        T::Error: Into<BillingError>,
    {
        let positions = match tariff.line_items(usage).map_err(Into::into)? {
            Billing::Billable(items) => items,
            Billing::NotBillable(reason) => return Ok(Billing::NotBillable(reason)),
        };
        self.positions = positions;
        self.tax_layers = tariff.tax_layers();
        self.discount_layers = tariff.discount_layers();
        Ok(Billing::Billable(self))
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

    /// Assemble every amount at a fixed number of decimal places.
    ///
    /// Each **leaf** amount — every position, every discount- and tax-layer output,
    /// and every VAT breakdown entry — is reduced to `scale` before any total is
    /// computed, so each total is a sum of already-reduced values and lands on the
    /// same precision exactly.
    ///
    /// # Why this cannot be done afterwards
    ///
    /// Reducing the precision of a finished document breaks its own arithmetic.
    /// Rounding each total independently is not a rounding of the document, it is
    /// four unrelated roundings that no longer add up:
    ///
    /// - three positions of `0.005` each become `0.01`, summing to `0.03`, while the
    ///   exact total `0.015` becomes `0.02` — the positions no longer sum to the
    ///   total (EN 16931 **BR-CO-10**);
    /// - a net of `0.0042` with 19 % VAT gives `0.00 + 0.00 ≠ 0.01` — net plus tax
    ///   no longer equals gross (**BR-CO-15**).
    ///
    /// Both are produced by real inputs, and both make an EN 16931 validator reject
    /// the invoice. Rounding the leaves and recomputing the aggregates is the only
    /// construction that satisfies the decimal limits and the totals identities at
    /// the same time — and it has to happen here, during assembly.
    ///
    /// Use [`AmountScale::EN16931`] for the two decimals that EN 16931, XRechnung,
    /// Peppol BIS and ZUGFeRD all require.
    ///
    /// # What preserves the scale, and what does not
    ///
    /// [`BillingDocument::reverse`] preserves it — a credit note of a two-decimal
    /// invoice is two decimals. [`crate::AllocationRule`] **does not, and cannot**:
    /// splitting `100.00` three ways is `33.333…`, and there is no two-decimal
    /// answer. Allocation keeps the split *exact* (the parts still sum to the
    /// original) at the cost of precision, which is the right trade for money but
    /// means an allocated document must be re-assembled — or at least re-checked
    /// with [`BillingDocument::fits_amount_scale`] — before it is emitted.
    ///
    /// ```rust
    /// use billing::{BillingDocument, DocumentMeta, LineItem, Amount, Currency,
    ///               AmountScale, FixedRateTax, TaxLayer, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// // 1234.567 kWh × 0.28901 EUR/kWh = 356.80221 — five decimals.
    /// let doc = BillingDocument::builder()
    ///     .currency(Currency::EUR)
    ///     .amount_scale(AmountScale::EN16931)
    ///     .positions(vec![LineItem::for_usage(
    ///         "Arbeit",
    ///         Quantity::new(dec!(1234.567), "kWh"),
    ///         UnitPrice::new(dec!(0.28901), "EUR/kWh"),
    ///     ).build()?])
    ///     .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    ///     .build()?;
    ///
    /// // Every amount now fits two decimals …
    /// assert!(doc.fits_amount_scale(2));
    /// assert_eq!(doc.net_total(),   Amount::parse("356.80000")?);
    /// assert_eq!(doc.tax_total(),   Amount::parse("67.79000")?);
    /// assert_eq!(doc.gross_total(), Amount::parse("424.59000")?);
    /// // … and the identities still hold exactly, so the document validates.
    /// doc.assert_valid();
    /// # Ok::<(), Box<dyn std::error::Error>>(())
    /// ```
    #[must_use]
    pub fn amount_scale(mut self, scale: AmountScale) -> Self {
        self.amount_scale = Some(scale);
        self
    }

    /// Build the [`BillingDocument`].
    pub fn build(self) -> Result<BillingDocument, BillingError> {
        BillingDocument::assemble(
            self.meta,
            self.positions,
            self.tax_layers,
            self.discount_layers,
            self.amount_scale,
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
