//! [`Period`] plus the period helpers [`merge_period_documents`], [`prorate`]
//! and [`prorate_amount`].
use crate::amount::{Amount, RoundingStrategy};
use crate::document::BillingDocument;
use crate::error::BillingError;
use crate::line_item::LineItem;
use rust_decimal::Decimal;

// ── Period ───────────────────────────────────────────────────────────────────

/// An inclusive billing period: a start/end date pair stored as ISO 8601 strings.
///
/// The library is date-type-agnostic: dates are `String` values and are **not parsed
/// or validated** by the engine. Store `"YYYY-MM-DD"` dates for maximum interoperability
/// with BO4E, EDIFACT, UBL, and German energy market standards (UStG §14, MessZV §22).
///
/// # Example
/// ```rust
/// use billing::Period;
/// let p = Period::new("2026-06-01", "2026-06-30");
/// assert_eq!(p.from, "2026-06-01");
/// assert_eq!(p.to,   "2026-06-30");
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Period {
    /// Start of the period (inclusive), e.g. `"2026-06-01"`.
    pub from: String,
    /// End of the period (inclusive), e.g. `"2026-06-30"`.
    pub to: String,
}

impl Period {
    /// Create a period from any `Into<String>` date strings.
    ///
    /// # Example
    /// ```rust
    /// use billing::Period;
    /// let p = Period::new("2026-06-01", "2026-06-30");
    /// assert_eq!(p.from, "2026-06-01");
    /// assert_eq!(p.to,   "2026-06-30");
    /// ```
    #[must_use]
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
        }
    }

    /// Create a period from any values implementing [`std::fmt::Display`].
    ///
    /// More ergonomic than [`Period::new`] when working with date types that
    /// implement [`std::fmt::Display`] but not [`Into<String>`] directly
    /// (for example `time::Date`, `chrono::NaiveDate`, or any custom date type).
    ///
    /// # Example
    /// ```rust
    /// use billing::Period;
    ///
    /// // Works with anything that implements Display:
    /// let p = Period::from_display("2026-06-01", "2026-06-30");
    /// assert_eq!(p.from, "2026-06-01");
    ///
    /// // With a date type (e.g. time::Date would work here):
    /// // let start: time::Date = ...;
    /// // let end:   time::Date = ...;
    /// // let p = Period::from_display(start, end);
    /// ```
    #[must_use]
    pub fn from_display(from: impl std::fmt::Display, to: impl std::fmt::Display) -> Self {
        Self {
            from: from.to_string(),
            to: to.to_string(),
        }
    }
}

// ── merge_period_documents ────────────────────────────────────────────────────

/// Merge two billing documents for adjacent periods (e.g. after a tariff change).
///
/// Positions from `doc_a` appear first, then `doc_b`. Totals are summed.
/// Tax layers are **not** re-applied — each document was already taxed
/// independently for its half-period.
///
/// The merged document keeps **`doc_a`'s header** ([`crate::DocumentMeta`]);
/// `doc_b`'s header — including its invoice number and period — is discarded.
/// Set the merged period explicitly afterwards if it matters.
///
/// # Errors
/// - [`BillingError::CurrencyMismatch`] if the two documents are denominated in
///   different currencies — summing them would produce a meaningless total.
/// - [`BillingError::MonetaryOverflow`] if any total overflows.
///
/// # Example — Tarifwechsel on the 15th of a 31-day month
///
/// ```rust
/// # use billing::{BillingDocument, DocumentMeta, Currency};
/// # let meta = || DocumentMeta { currency: Currency::EUR, ..Default::default() };
/// # let doc_a = BillingDocument::from_positions(meta(), vec![], vec![], vec![]).unwrap();
/// # let doc_b = BillingDocument::from_positions(meta(), vec![], vec![], vec![]).unwrap();
/// let merged = billing::merge_period_documents(doc_a, doc_b).unwrap();
/// ```
pub fn merge_period_documents(
    doc_a: BillingDocument,
    doc_b: BillingDocument,
) -> Result<BillingDocument, BillingError> {
    // Itemised advances reference specific advance invoices; merging two documents
    // would produce a combined deduction table that matches neither. `from_raw`
    // cannot carry them, so refusing beats dropping them — the §14 Abs. 5 Satz 2
    // deduction data would vanish while `prepaid` stayed, and the result would
    // still pass `validate()` because check 10 is skipped when advances are empty.
    if !doc_a.advances().is_empty() || !doc_b.advances().is_empty() {
        return Err(BillingError::InvalidInput {
            reason: "cannot merge documents carrying itemised advance payments: \
                     merge the underlying positions and settle advances once"
                .into(),
        });
    }
    // Cash rounding is a property of the payable total, so a merged document needs
    // it recomputed rather than summed. `from_raw` cannot carry the rule, so the
    // stored adjustment would go unchecked by validate() check 7.
    if doc_a.cash_rounding().is_some() || doc_b.cash_rounding().is_some() {
        return Err(BillingError::InvalidInput {
            reason: "cannot merge documents with a cash-rounding rule: merge first, \
                     then apply the rounding to the combined payable amount"
                .into(),
        });
    }
    if doc_a.currency() != doc_b.currency() {
        return Err(BillingError::CurrencyMismatch {
            left: doc_a.currency(),
            right: doc_b.currency(),
        });
    }
    let net = doc_a.net_total().checked_add(doc_b.net_total())?;
    let tax = doc_a.tax_total().checked_add(doc_b.tax_total())?;
    let gross = doc_a.gross_total().checked_add(doc_b.gross_total())?;

    let mut net_positions = doc_a.net_positions().to_vec();
    let mut tax_positions = doc_a.tax_positions().to_vec();
    let mut discount_positions = doc_a.discount_positions().to_vec();

    net_positions.extend(doc_b.net_positions().iter().cloned());
    tax_positions.extend(doc_b.tax_positions().iter().cloned());
    discount_positions.extend(doc_b.discount_positions().iter().cloned());

    // Merge the two VAT breakdowns so the combined document still shows one line
    // per (category, rate) — two half-period documents at the same rate must not
    // produce two breakdown entries.
    let mut breakdown = doc_a.tax_breakdown().to_vec();
    breakdown.extend(doc_b.tax_breakdown().iter().cloned());

    // Read the settlement figures before `doc_a.meta` is moved out below.
    let prepaid = doc_a.prepaid().checked_add(doc_b.prepaid())?;
    let rounding = doc_a.rounding().checked_add(doc_b.rounding())?;

    BillingDocument::from_raw(crate::document::DocumentParts {
        meta: doc_a.meta,
        net_positions,
        tax_positions,
        discount_positions,
        net_total: net,
        tax_total: tax,
        gross_total: gross,
        tax_breakdown: crate::document::merge_breakdown(breakdown)?,
        // Both halves' already-paid amounts and rounding adjustments carry over.
        prepaid,
        rounding,
    })
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
    let fraction = Decimal::from(active_days)
        .checked_div(Decimal::from(total_days))
        .ok_or(BillingError::MonetaryOverflow {
            precision: 5,
            input_value: None,
        })?;
    // `LineItem::scaled` applies the caller's rounding strategy exactly once on the
    // raw product and scales the QUANTITY by the same fraction. Scaling net_amount
    // alone would leave the line self-contradictory — e.g. "1000 kWh × 0.30 EUR/kWh
    // = 150.00" after a half-month proration.
    let mut result = item.scaled(fraction, strategy)?;
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
    let overflow = || BillingError::MonetaryOverflow {
        precision: 5,
        input_value: None,
    };
    let fraction = Decimal::from(active_days)
        .checked_div(Decimal::from(total_days))
        .ok_or_else(overflow)?;
    // Apply strategy once on the raw product (same logic as prorate()).
    // `checked_mul`: `Decimal`'s `*` panics on overflow.
    let rounded = amount
        .into_decimal()
        .checked_mul(fraction)
        .ok_or_else(overflow)?
        .round_dp_with_strategy(5, strategy.into());
    Amount::<5>::from_decimal(rounded).ok_or_else(overflow)
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
