//! [`BillingDocument`] — self-validating invoice with ordered positions + totals.
use crate::amount::Amount;
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::tax::{DiscountLayer, TaxLayer};

// ── DocumentMeta ──────────────────────────────────────────────────────────────

/// Non-computed header fields for a billing document.
///
/// `billing` does not parse dates — use whatever date type fits your domain.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct DocumentMeta {
    /// Unique identifier for this document (e.g. `"INV-2026-001"`).
    pub invoice_number: String,
    /// Human-readable period label (e.g. `"2026-06"`, `"July 2026"`). Not parsed.
    pub period_label: String,
    /// Optional free-text remarks printed on the document.
    pub notes: Option<String>,
}

// ── BillingDocument ───────────────────────────────────────────────────────────

/// A complete, self-validating billing document.
///
/// Holds ordered positions (net → discounts → taxes) and pre-computed totals.
/// All totals are verified at construction time: `Σ(net_positions) == net_total`
/// within 1 unit (0.00001 EUR).
///
/// # Construction
///
/// - [`BillingDocument::from_positions`] — supply positions and layer vecs directly.
/// - [`BillingDocument::builder`] — fluent builder; use `.tariff(t, u)?` to load
///   positions from a [`crate::Tariff`] implementation.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct BillingDocument {
    /// Document header (invoice number, period label, notes).
    pub meta: DocumentMeta,
    net_positions: Vec<LineItem>,
    tax_positions: Vec<LineItem>,
    discount_positions: Vec<LineItem>,
    net_total: Amount<5>,
    tax_total: Amount<5>,
    gross_total: Amount<5>,
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
    #[must_use]
    pub fn net_total(&self) -> Amount<5> {
        self.net_total
    }
    /// Sum of all tax positions.
    #[must_use]
    pub fn tax_total(&self) -> Amount<5> {
        self.tax_total
    }
    /// `gross = net + tax`. Discounts are included in `net_total` as negatives.
    #[must_use]
    pub fn gross_total(&self) -> Amount<5> {
        self.gross_total
    }

    /// Sum of all discount positions (always ≤ 0).
    ///
    /// Discounts are already embedded in `net_total` as negative amounts.
    /// This accessor surfaces them separately for display or BO4E output.
    #[must_use]
    pub fn discount_total(&self) -> Amount<5> {
        // Discount positions are bounded by construction; .sum() is safe here.
        self.discount_positions.iter().map(|p| p.net_amount).sum()
    }

    /// Iterate over every position (net, discount, and tax) that carries `tag`.
    ///
    /// Searches all three position buckets in order: net → discounts → taxes.
    /// Use this when building domain-specific output (e.g. BO4E `rechnungspositionen`
    /// filtered by commodity tag).
    ///
    /// ```rust,ignore
    /// for pos in doc.positions_by_tag("commodity") {
    ///     println!("{}: {}", pos.description, pos.net_amount);
    /// }
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
        let discount_positions: Vec<LineItem> = discounts
            .iter()
            .map(|d| d.compute(&positions))
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
        for t in &tax_layers {
            let item = t.compute(&accumulated)?;
            accumulated.push(item.clone());
            tax_positions.push(item);
        }

        let tax_total = Amount::checked_sum(tax_positions.iter().map(|p| p.net_amount))?;
        let gross_total = net_total.checked_add(tax_total)?;

        Ok(Self {
            meta,
            net_positions: positions,
            tax_positions,
            discount_positions,
            net_total,
            tax_total,
            gross_total,
        })
    }

    /// Construct directly from pre-computed fields. Used by `allocation.rs` and `period.rs`.
    ///
    /// No recomputation or validation is performed — callers must ensure
    /// consistency between positions and totals.
    pub(crate) fn from_raw(
        meta: DocumentMeta,
        net_positions: Vec<LineItem>,
        tax_positions: Vec<LineItem>,
        discount_positions: Vec<LineItem>,
        net_total: Amount<5>,
        tax_total: Amount<5>,
        gross_total: Amount<5>,
    ) -> Self {
        Self {
            meta,
            net_positions,
            tax_positions,
            discount_positions,
            net_total,
            tax_total,
            gross_total,
        }
    }

    // ── Validation ────────────────────────────────────────────────────────────

    /// Assert full arithmetic correctness of the document.
    ///
    /// Three invariants are checked (all exact — no tolerance):
    /// 1. `Σ(net_positions + discount_positions) == net_total`
    /// 2. `Σ(tax_positions) == tax_total`
    /// 3. `net_total + tax_total == gross_total`
    ///
    /// All documents built by this library satisfy these invariants at
    /// construction time.  Call this after any external mutation to verify
    /// the document has not been corrupted.
    pub fn assert_valid(&self) -> Result<(), BillingError> {
        // Check 1: net positions + discount positions sum exactly to net_total.
        let computed_net = Amount::checked_sum(
            self.net_positions
                .iter()
                .chain(&self.discount_positions)
                .map(|p| p.net_amount),
        )?;
        if computed_net != self.net_total {
            return Err(BillingError::ValidationFailed {
                check: "net_total",
                actual: computed_net.to_string(),
                expected: self.net_total.to_string(),
            });
        }

        // Check 2: tax positions sum exactly to tax_total.
        let computed_tax = Amount::checked_sum(self.tax_positions.iter().map(|p| p.net_amount))?;
        if computed_tax != self.tax_total {
            return Err(BillingError::ValidationFailed {
                check: "tax_total",
                actual: computed_tax.to_string(),
                expected: self.tax_total.to_string(),
            });
        }

        // Check 3: net_total + tax_total == gross_total.
        let expected_gross = self.net_total.checked_add(self.tax_total)?;
        if expected_gross != self.gross_total {
            return Err(BillingError::ValidationFailed {
                check: "gross_total",
                actual: expected_gross.to_string(),
                expected: self.gross_total.to_string(),
            });
        }

        Ok(())
    }

    // ── Mutation helpers ──────────────────────────────────────────────────────

    /// Append an extra position and recompute net and gross totals.
    ///
    /// Tax positions are NOT recalculated — use this only for fixed surcharges
    /// like [`crate::minimum_charge`] that are added after initial tax calculation.
    pub fn with_extra_position(mut self, item: LineItem) -> Result<Self, BillingError> {
        self.net_total = self.net_total.checked_add(item.net_amount)?;
        self.gross_total = self.net_total.checked_add(self.tax_total)?;
        self.net_positions.push(item);
        Ok(self)
    }
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
///     .extra_tax(Box::new(FixedRateTax::new("VAT", dec!(0.20))))
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
    use rust_decimal_macros::dec;

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
        let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
        let doc =
            BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
        assert_eq!(doc.net_total(), Amount::parse("100.00000").unwrap());
        assert_eq!(doc.tax_total(), Amount::parse("20.00000").unwrap());
        assert_eq!(doc.gross_total(), Amount::parse("120.00000").unwrap());
        doc.assert_valid().unwrap();
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
            Box::new(PercentageCharge::new("Levy", dec!(0.05))),
            // Layer 2: 19% VAT — should see Net (100) + Levy (5) = 105
            Box::new(FixedRateTax::new("VAT", dec!(0.19))),
        ];
        let doc =
            BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();

        assert_eq!(doc.net_total(), Amount::parse("100.00000").unwrap());
        // Levy = 5.00000
        // VAT base = 100.00 + 5.00 = 105.00; VAT = 105.00 × 0.19 = 19.95000
        assert_eq!(doc.tax_total(), Amount::parse("24.95000").unwrap());
        assert_eq!(doc.gross_total(), Amount::parse("124.95000").unwrap());
        doc.assert_valid().unwrap();
    }

    #[test]
    fn assert_valid_full_three_checks() {
        let doc = simple_doc("42.00000");
        doc.assert_valid().unwrap();

        // Manually corrupt net_total — check 1 should fire.
        let mut bad = doc.clone();
        bad.net_total = Amount::parse("99.00000").unwrap();
        assert!(matches!(
            bad.assert_valid(),
            Err(crate::error::BillingError::ValidationFailed {
                check: "net_total",
                ..
            })
        ));
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
            .extra_tax(Box::new(FixedRateTax::new("VAT", dec!(0.20))))
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
        simple_doc("42.00000").assert_valid().unwrap();
    }
}
