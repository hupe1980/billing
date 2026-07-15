//! Period helpers: [`merge_period_documents`], [`prorate`], [`prorate_amount`].
use crate::amount::{Amount, RoundingStrategy};
use crate::document::BillingDocument;
use crate::error::BillingError;
use crate::line_item::LineItem;
use rust_decimal::Decimal;

// ── merge_period_documents ────────────────────────────────────────────────────

/// Merge two billing documents for adjacent periods (e.g. after a tariff change).
///
/// Positions from `doc_a` appear first, then `doc_b`. Totals are summed.
/// Tax layers are **not** re-applied — each document was already taxed
/// independently for its half-period.
///
/// # Example — Tarifwechsel on the 15th of a 31-day month
///
/// ```rust
/// # use billing::{BillingDocument, DocumentMeta};
/// # let doc_a = BillingDocument::from_positions(DocumentMeta::default(), vec![], vec![], vec![]).unwrap();
/// # let doc_b = BillingDocument::from_positions(DocumentMeta::default(), vec![], vec![], vec![]).unwrap();
/// let merged = billing::merge_period_documents(doc_a, doc_b).unwrap();
/// ```
pub fn merge_period_documents(
    doc_a: BillingDocument,
    doc_b: BillingDocument,
) -> Result<BillingDocument, BillingError> {
    let net = doc_a.net_total().checked_add(doc_b.net_total())?;
    let tax = doc_a.tax_total().checked_add(doc_b.tax_total())?;
    let gross = doc_a.gross_total().checked_add(doc_b.gross_total())?;

    let mut net_positions = doc_a.net_positions().to_vec();
    let mut tax_positions = doc_a.tax_positions().to_vec();
    let mut discount_positions = doc_a.discount_positions().to_vec();

    net_positions.extend(doc_b.net_positions().iter().cloned());
    tax_positions.extend(doc_b.tax_positions().iter().cloned());
    discount_positions.extend(doc_b.discount_positions().iter().cloned());

    Ok(BillingDocument::from_raw(
        doc_a.meta,
        net_positions,
        tax_positions,
        discount_positions,
        net,
        tax,
        gross,
    ))
}

// ── prorate ───────────────────────────────────────────────────────────────────

/// Scale a `LineItem` to the fraction of a period actually used.
///
/// Formula: `amount × active_days / total_days`, rounded once with `strategy`.
///
/// Use for fixed charges when a customer joins or leaves mid-period.
///
/// # Examples
/// - Customer joins on the 15th of a 30-day month:
///   `prorate(&grundpreis, 15, 30, MidpointAwayFromZero)` → half the monthly fee
/// - Annual subscription cancelled after 100 days:
///   `prorate(&annual_fee, 100, 365, MidpointAwayFromZero)` → partial refund
///
/// # Errors
/// - `Err(BillingError::ZeroPeriod)` when `total_days == 0`.
/// - `Err(BillingError::InvalidInput)` when `active_days > total_days`.
pub fn prorate(
    item: &LineItem,
    active_days: u32,
    total_days: u32,
    strategy: RoundingStrategy,
) -> Result<LineItem, BillingError> {
    if total_days == 0 {
        return Err(BillingError::ZeroPeriod);
    }
    if active_days > total_days {
        return Err(BillingError::InvalidInput {
            reason: "active_days must not exceed total_days".into(),
        });
    }
    let fraction = Decimal::from(active_days) / Decimal::from(total_days);
    // Apply the caller's rounding strategy exactly once on the raw product.
    // Do NOT use mul_qty here — that always rounds with MidpointAwayFromZero,
    // which would silently override the strategy the caller specified.
    let raw = item.net_amount.into_decimal() * fraction;
    let rounded = raw.round_dp_with_strategy(5, strategy.into());
    let prorated = Amount::<5>::from_decimal(rounded).ok_or(BillingError::MonetaryOverflow {
        precision: 5,
        input_value: None,
    })?;
    let mut result = item.clone();
    result.net_amount = prorated;
    // Clear the period: the prorated item covers a SUB-period of the source,
    // and this function only knows day counts, not the actual date range.
    // Callers who need the exact period must set it explicitly:
    //   result.period = Some(Period::new("2026-06-15", "2026-06-30"));
    result.period = None;
    result.description = format!(
        "{} (prorated {active_days}/{total_days}d)",
        item.description
    );
    Ok(result)
}

// ── prorate_amount ────────────────────────────────────────────────────────────

/// Scale a bare `Amount<5>` to the fraction of a period.
///
/// Convenience wrapper around the `LineItem` form for cases where you only
/// need the prorated monetary value.
pub fn prorate_amount(
    amount: Amount<5>,
    active_days: u32,
    total_days: u32,
    strategy: RoundingStrategy,
) -> Result<Amount<5>, BillingError> {
    if total_days == 0 {
        return Err(BillingError::ZeroPeriod);
    }
    if active_days > total_days {
        return Err(BillingError::InvalidInput {
            reason: "active_days must not exceed total_days".into(),
        });
    }
    let fraction = Decimal::from(active_days) / Decimal::from(total_days);
    // Apply strategy once on the raw product (same logic as prorate()).
    let raw = amount.into_decimal() * fraction;
    let rounded = raw.round_dp_with_strategy(5, strategy.into());
    Amount::<5>::from_decimal(rounded).ok_or(BillingError::MonetaryOverflow {
        precision: 5,
        input_value: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::{Amount, RoundingStrategy};
    use crate::document::DocumentMeta;
    use crate::line_item::LineItem;

    #[test]
    fn prorate_half_month() {
        let item = LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
            .build()
            .unwrap();
        let prorated = prorate(&item, 15, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
        assert_eq!(prorated.net_amount, Amount::parse("15.00000").unwrap());
        assert!(prorated.description.contains("prorated 15/30d"));
    }

    #[test]
    fn prorate_zero_period_errors() {
        let item = LineItem::fixed("Fee", Amount::parse("10.00000").unwrap())
            .build()
            .unwrap();
        assert!(matches!(
            prorate(&item, 1, 0, RoundingStrategy::Truncate),
            Err(BillingError::ZeroPeriod)
        ));
    }

    #[test]
    fn merge_totals_sum() {
        let make = |amount: &str| {
            BillingDocument::from_positions(
                DocumentMeta::default(),
                vec![
                    LineItem::fixed("x", Amount::parse(amount).unwrap())
                        .build()
                        .unwrap(),
                ],
                vec![],
                vec![],
            )
            .unwrap()
        };
        let a = make("100.00000");
        let b = make("50.00000");
        let merged = merge_period_documents(a, b).unwrap();
        assert_eq!(merged.net_total(), Amount::parse("150.00000").unwrap());
        assert_eq!(merged.net_positions().len(), 2);
    }
}
