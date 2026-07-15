//! [`AllocationRule`] — proportional and equal split across N recipients.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::document::{BillingDocument, DocumentMeta};
use crate::error::BillingError;
use crate::line_item::LineItem;

// ── AllocationRule trait ──────────────────────────────────────────────────────

/// Split a [`BillingDocument`] proportionally across N recipients.
pub trait AllocationRule {
    /// Split `doc` into N recipient documents according to this rule.
    fn allocate(&self, doc: &BillingDocument) -> Result<Vec<BillingDocument>, BillingError>;
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Scale `positions` by `share`, then apply a **penny correction** to the last
/// item so that `Σ(result) == target` exactly.
///
/// The correction is at most a few units of the last decimal place.
fn scale_with_target(
    positions: &[LineItem],
    share: Decimal,
    target: Amount<5>,
) -> Result<Vec<LineItem>, BillingError> {
    if positions.is_empty() {
        return Ok(vec![]);
    }
    let mut items: Vec<LineItem> = positions
        .iter()
        .map(|p| {
            let scaled = p.net_amount.checked_mul_qty(share)?;
            let mut item = p.clone();
            item.net_amount = scaled;
            Ok(item)
        })
        .collect::<Result<Vec<_>, BillingError>>()?;
    let sum = Amount::checked_sum(items.iter().map(|p| p.net_amount))?;
    if sum != target {
        let correction = target.checked_sub(sum)?;
        if let Some(last) = items.last_mut() {
            last.net_amount = last.net_amount.checked_add(correction)?;
        }
    }
    Ok(items)
}

/// Scale `net_positions` and `discount_positions` together, applying the penny
/// correction to the combined `net_total` (since both contribute to `net_total`).
///
/// The correction is applied to the last discount item if present, otherwise to
/// the last net position.
fn scale_combined_net(
    net_positions: &[LineItem],
    discount_positions: &[LineItem],
    share: Decimal,
    target_net_total: Amount<5>,
) -> Result<(Vec<LineItem>, Vec<LineItem>), BillingError> {
    let mut scaled_net: Vec<LineItem> = net_positions
        .iter()
        .map(|p| {
            let scaled = p.net_amount.checked_mul_qty(share)?;
            let mut item = p.clone();
            item.net_amount = scaled;
            Ok(item)
        })
        .collect::<Result<Vec<_>, BillingError>>()?;
    let mut scaled_disc: Vec<LineItem> = discount_positions
        .iter()
        .map(|p| {
            let scaled = p.net_amount.checked_mul_qty(share)?;
            let mut item = p.clone();
            item.net_amount = scaled;
            Ok(item)
        })
        .collect::<Result<Vec<_>, BillingError>>()?;

    // Penny correction: ensure Σ(net) + Σ(discounts) == target_net_total.
    let combined_sum =
        Amount::checked_sum(scaled_net.iter().chain(&scaled_disc).map(|p| p.net_amount))?;
    if combined_sum != target_net_total {
        let correction = target_net_total.checked_sub(combined_sum)?;
        // Prefer correcting the last discount item (credits are typically smaller).
        if let Some(last) = scaled_disc.last_mut() {
            last.net_amount = last.net_amount.checked_add(correction)?;
        } else if let Some(last) = scaled_net.last_mut() {
            last.net_amount = last.net_amount.checked_add(correction)?;
        }
    }
    Ok((scaled_net, scaled_disc))
}

// ── ProportionalAllocation ────────────────────────────────────────────────────

/// Proportional allocation: each recipient's share is `shares[i]`.
///
/// `shares` must sum to `1.0 ± 1e-9`.
///
/// ## Arithmetic correctness guarantees
///
/// 1. `Σ(net_total)   == original.net_total`   — exact, no drift.
/// 2. `Σ(tax_total)   == original.tax_total`   — exact.
/// 3. `Σ(gross_total) == original.gross_total` — exact.
/// 4. Each recipient's [`BillingDocument::assert_valid`] passes.
///
/// (1)–(3) hold because the **last recipient receives the arithmetic remainder**
/// (`total − Σ(others)`).  (4) holds because a **penny correction** is applied
/// to the last item of each section so that `Σ(positions) == section_total`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct ProportionalAllocation {
    /// Fractional shares that must sum to `1.0 ± 1e-9`.
    shares: Vec<Decimal>,
}

impl ProportionalAllocation {
    /// Validate that `shares` sum to `1.0 ± 1e-9`.
    pub fn new(shares: Vec<Decimal>) -> Result<Self, BillingError> {
        let sum: Decimal = shares.iter().sum();
        if (sum - Decimal::ONE).abs() > Decimal::new(1, 9) {
            return Err(BillingError::InvalidAllocationShares {
                sum: sum.to_string(),
            });
        }
        Ok(Self { shares })
    }

    /// The fractional shares.
    #[must_use]
    pub fn shares(&self) -> &[Decimal] {
        &self.shares
    }
}

impl AllocationRule for ProportionalAllocation {
    fn allocate(&self, doc: &BillingDocument) -> Result<Vec<BillingDocument>, BillingError> {
        let n = self.shares.len();
        let mut docs = Vec::with_capacity(n);

        let mut net_remaining = doc.net_total();
        let mut tax_remaining = doc.tax_total();
        let mut gross_remaining = doc.gross_total();

        for (i, share) in self.shares.iter().enumerate() {
            let is_last = i == n - 1;

            let (net_total, tax_total, gross_total) = if is_last {
                (net_remaining, tax_remaining, gross_remaining)
            } else {
                let net = doc.net_total().checked_mul_qty(*share)?;
                let tax = doc.tax_total().checked_mul_qty(*share)?;
                let gross = doc.gross_total().checked_mul_qty(*share)?;
                net_remaining = net_remaining.checked_sub(net)?;
                tax_remaining = tax_remaining.checked_sub(tax)?;
                gross_remaining = gross_remaining.checked_sub(gross)?;
                (net, tax, gross)
            };

            // Scale all three position sections with penny correction so each
            // document passes assert_valid().
            let (net_positions, discount_positions) = scale_combined_net(
                doc.net_positions(),
                doc.discount_positions(),
                *share,
                net_total,
            )?;
            let tax_positions = scale_with_target(doc.tax_positions(), *share, tax_total)?;

            docs.push(BillingDocument::from_raw(
                DocumentMeta {
                    invoice_number: format!("{}/{}", doc.meta.invoice_number, i + 1),
                    period_label: doc.meta.period_label.clone(),
                    period: doc.meta.period.clone(),
                    issue_date: doc.meta.issue_date.clone(),
                    due_date: doc.meta.due_date.clone(),
                    issuer_id: doc.meta.issuer_id.clone(),
                    recipient_id: doc.meta.recipient_id.clone(),
                    notes: doc.meta.notes.clone(),
                    labels: doc.meta.labels.clone(),
                },
                net_positions,
                tax_positions,
                discount_positions,
                net_total,
                tax_total,
                gross_total,
            ));
        }
        Ok(docs)
    }
}

// ── EqualAllocation ───────────────────────────────────────────────────────────

/// Equal allocation: split N ways (each recipient gets `1/N` of the total).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone)]
pub struct EqualAllocation {
    n: usize,
}

impl EqualAllocation {
    /// Create an `EqualAllocation` that splits into `n` equal parts.
    ///
    /// # Panics
    /// Panics if `n == 0`. Allocating to zero recipients makes no sense.
    #[must_use]
    pub fn new(n: usize) -> Self {
        assert!(n > 0, "EqualAllocation requires n > 0, got n = 0");
        Self { n }
    }

    /// The number of equal recipients.
    #[must_use]
    pub fn n(&self) -> usize {
        self.n
    }
}

impl AllocationRule for EqualAllocation {
    fn allocate(&self, doc: &BillingDocument) -> Result<Vec<BillingDocument>, BillingError> {
        let share = Decimal::ONE / Decimal::from(self.n);
        let shares = vec![share; self.n];
        // Bypass the sum=1.0 check (1/n × n ≈ 1 but may not be exactly 1.0
        // in Decimal).  The last-recipient remainder in `allocate` corrects any
        // residual drift, so the check is unnecessary here.
        ProportionalAllocation { shares }.allocate(doc)
    }
}

// ── proportional_split ────────────────────────────────────────────────────────

/// Split `total` into N parts proportional to `fractions`, with penny-correct
/// rounding at `scale` decimal places.
///
/// Uses the **Largest-Remainder (Hamilton) method**:
/// 1. Each part is floored to `scale` dp.
/// 2. The under-allocation (deficit) is distributed one unit (`10⁻ˢᶜᵃˡᵉ`) at a
///    time to the fractions with the largest fractional remainders.
///
/// # Guarantees
///
/// - `Σ(parts) == total.round_dp(scale)` — exact, no drift.
/// - Each part is rounded to exactly `scale` dp.
/// - No single part absorbs a disproportionate correction: at most one smallest
///   unit of adjustment is applied per fraction.
///
/// # Use cases
///
/// Quantity splits that precede document creation — for example:
/// - §42b GGV/EEG 2023: PV output split by building-occupant consumption fractions.
/// - §24 CapacityBlock: proportional kWh per capacity tier.
/// - Any domain where a raw `Decimal` quantity must be split proportionally without drift.
///
/// For monetary document splits, use [`ProportionalAllocation`] instead.
///
/// # Example
///
/// ```rust
/// use billing::proportional_split;
/// use rust_decimal_macros::dec;
///
/// // Split 100 kWh among three tenants (fractions sum to 1.0).
/// let parts = proportional_split(
///     dec!(100),
///     &[dec!(0.333), dec!(0.333), dec!(0.334)],
///     3,
/// ).unwrap();
///
/// // Each part is rounded to 3 dp and the sum is exactly 100.000.
/// let total: rust_decimal::Decimal = parts.iter().sum();
/// assert_eq!(total, dec!(100));
/// ```
///
/// # Errors
///
/// - [`BillingError::InvalidInput`] if `fractions` is empty, `total` is negative,
///   or any fraction is negative.
/// - [`BillingError::InvalidAllocationShares`] if `Σ(fractions) ≠ 1.0 ± 1e-9`.
pub fn proportional_split(
    total: Decimal,
    fractions: &[Decimal],
    scale: u32,
) -> Result<Vec<Decimal>, BillingError> {
    use rust_decimal::prelude::ToPrimitive as _;

    if fractions.is_empty() {
        return Err(BillingError::InvalidInput {
            reason: "fractions must not be empty".into(),
        });
    }
    if total < Decimal::ZERO {
        return Err(BillingError::InvalidInput {
            reason: "proportional_split: total must be non-negative".into(),
        });
    }
    for &f in fractions {
        if f < Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: "proportional_split: each fraction must be non-negative".into(),
            });
        }
    }
    let sum: Decimal = fractions.iter().sum();
    if (sum - Decimal::ONE).abs() > Decimal::new(1, 9) {
        return Err(BillingError::InvalidAllocationShares {
            sum: sum.to_string(),
        });
    }

    // Normalise total to `scale` dp so parts can exactly sum to it.
    let total =
        total.round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointAwayFromZero);

    // Step 1 — floor each ideal share to `scale` dp.
    let unit = Decimal::new(1, scale); // 10^{-scale}
    let mut parts: Vec<Decimal> = fractions
        .iter()
        .map(|&f| {
            (total * f)
                .round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::ToNegativeInfinity)
        })
        .collect();

    // Step 2 — remainders (fractional parts discarded by the floor).
    let remainders: Vec<Decimal> = fractions
        .iter()
        .enumerate()
        .map(|(i, &f)| total * f - parts[i])
        .collect();

    // Step 3 — compute deficit: how many extra units must be distributed.
    //
    // deficit = total − Σ(floored parts).
    // Because floor(x) ≤ x and Σ(f_i) ≈ 1, deficit ≥ 0 always.
    // In the rare case where floating-point precision produces a tiny negative
    // deficit, it is safe to skip distribution (no over-allocation).
    let floored_sum: Decimal = parts.iter().sum();
    let deficit_raw = total - floored_sum;
    if deficit_raw <= Decimal::ZERO {
        return Ok(parts);
    }

    // Round deficit to `scale` dp (it should already be exact, but guard against
    // any residual Decimal precision artefact).
    let deficit = deficit_raw
        .round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointAwayFromZero);

    // Convert deficit to an integer count of `unit`s.
    // deficit is always < fractions.len() × unit (sum of fractional remainders < n).
    let n_units = (deficit / unit)
        .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        .to_usize()
        .ok_or_else(|| BillingError::InvalidInput {
            reason: "proportional_split: deficit unit count overflows usize".into(),
        })?;

    // Step 4 — Hamilton distribution: give one unit to each of the n_units
    // fractions with the largest remainder.  Ties broken by original order
    // (deterministic, no hidden randomness).
    let mut order: Vec<usize> = (0..fractions.len()).collect();
    order.sort_by(|&a, &b| {
        remainders[b].cmp(&remainders[a]).then_with(|| a.cmp(&b)) // stable tie-break: earlier index first
    });

    for &idx in order.iter().take(n_units) {
        parts[idx] += unit;
    }

    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::document::DocumentMeta;
    use crate::line_item::LineItem;
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

    /// Three-position document to exercise multi-item penny correction.
    fn multi_doc() -> BillingDocument {
        let pos = vec![
            LineItem::fixed("Item A", Amount::parse("33.33333").unwrap())
                .build()
                .unwrap(),
            LineItem::fixed("Item B", Amount::parse("33.33333").unwrap())
                .build()
                .unwrap(),
            LineItem::fixed("Item C", Amount::parse("33.33334").unwrap())
                .build()
                .unwrap(),
        ];
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap()
    }

    #[test]
    fn proportional_totals_exact_sum() {
        let doc = simple_doc("100.00000");
        let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)]).unwrap();
        let docs = alloc.allocate(&doc).unwrap();
        assert_eq!(docs.len(), 3);
        let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
        assert_eq!(total, doc.net_total(), "net_total must not drift");
    }

    #[test]
    fn each_doc_passes_assert_valid() {
        let doc = multi_doc();
        let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)]).unwrap();
        for d in alloc.allocate(&doc).unwrap().iter() {
            d.assert_valid();
        }
    }

    #[test]
    fn equal_two_way() {
        let doc = simple_doc("50.00000");
        let docs = EqualAllocation::new(2).allocate(&doc).unwrap();
        assert_eq!(docs[0].net_total(), Amount::parse("25.00000").unwrap());
        assert_eq!(docs[1].net_total(), Amount::parse("25.00000").unwrap());
    }

    #[test]
    fn equal_three_way_exact_sum_and_valid() {
        // Classic penny test: 100 / 3 = 33.33333...
        let doc = simple_doc("100.00000");
        let docs = EqualAllocation::new(3).allocate(&doc).unwrap();
        let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
        assert_eq!(total, doc.net_total(), "exact sum must hold");
        for d in &docs {
            d.assert_valid();
        }
    }

    #[test]
    fn invalid_shares_rejected() {
        assert!(ProportionalAllocation::new(vec![dec!(0.5), dec!(0.3)]).is_err());
    }

    #[test]
    #[should_panic(expected = "EqualAllocation requires n > 0")]
    fn equal_zero_panics_at_construction() {
        // Fail fast: n=0 panics at new(), not silently at allocate().
        let _ = EqualAllocation::new(0);
    }

    #[test]
    fn allocation_with_tax_preserves_gross_and_valid() {
        let pos = vec![
            LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ];
        let taxes: Vec<Box<dyn crate::tax::TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
        let doc =
            BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
        let docs = EqualAllocation::new(2).allocate(&doc).unwrap();

        let gross_sum: Amount<5> = docs.iter().map(|d| d.gross_total()).sum();
        assert_eq!(gross_sum, doc.gross_total(), "gross_total must not drift");
        for d in &docs {
            d.assert_valid();
        }
    }
}
