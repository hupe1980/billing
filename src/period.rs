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
/// # Ordering is the caller's responsibility
///
/// Because the engine does not parse the dates, it cannot tell an end-before-start
/// period from a valid one — `Period::new("2026-06-30", "2026-06-01")` is accepted.
/// EN 16931 *does* check it: **BR-29** requires BT-74 ≥ BT-73 on the invoicing
/// period and **BR-30** requires BT-135 ≥ BT-134 on a line period, both fatal. A
/// consumer that parses dates should verify ordering at its own boundary;
/// [`Period::is_ordered`] does it here for the ISO 8601 case.
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

    /// Whether `to` is on or after `from` — EN 16931 **BR-29** / **BR-30**.
    ///
    /// Compares the two strings **lexicographically**, which is a correct date
    /// comparison for zero-padded ISO 8601 (`"YYYY-MM-DD"`) and for nothing else.
    /// Returns `None` when either endpoint is not of that shape, rather than
    /// answering a question it cannot actually decide — the engine stays
    /// date-agnostic, and a confident `false` on `"1.6.2026"` would be worse than
    /// no answer.
    ///
    /// ```rust
    /// use billing::Period;
    ///
    /// assert_eq!(Period::new("2026-06-01", "2026-06-30").is_ordered(), Some(true));
    /// assert_eq!(Period::new("2026-06-30", "2026-06-01").is_ordered(), Some(false));
    /// // A single-day period is ordered: BR-29 and BR-30 both say "later or equal".
    /// assert_eq!(Period::new("2026-06-01", "2026-06-01").is_ordered(), Some(true));
    /// // Not ISO 8601 — the engine declines to guess.
    /// assert_eq!(Period::new("1.6.2026", "30.6.2026").is_ordered(), None);
    /// ```
    #[must_use]
    pub fn is_ordered(&self) -> Option<bool> {
        (is_iso_date(&self.from) && is_iso_date(&self.to)).then(|| self.to >= self.from)
    }

    /// Whether this period lies inside `outer` — `PEPPOL-EN16931-R110` and
    /// `PEPPOL-EN16931-R111`.
    ///
    /// > `[PEPPOL-EN16931-R110]` Start date of line period MUST be within invoice
    /// > period.
    /// >
    /// > `[PEPPOL-EN16931-R111]` End date of line period MUST be within invoice
    /// > period.
    ///
    /// Both **fatal**, and both stated exactly as the two comparisons below:
    /// `line.from >= invoice.from` and `line.to <= invoice.to`.
    ///
    /// Returns `None` when any of the four endpoints is not a zero-padded ISO 8601
    /// date, for the same reason [`is_ordered`](Self::is_ordered) does — a confident
    /// answer on `"1.6.2026"` would be worse than none.
    ///
    /// ```rust
    /// use billing::Period;
    ///
    /// let invoice = Period::new("2026-06-01", "2026-06-30");
    /// assert_eq!(Period::new("2026-06-01", "2026-06-14").within(&invoice), Some(true));
    /// assert_eq!(Period::new("2026-06-01", "2026-06-30").within(&invoice), Some(true));
    /// // A line that starts before the invoice period, or ends after it.
    /// assert_eq!(Period::new("2026-05-28", "2026-06-14").within(&invoice), Some(false));
    /// assert_eq!(Period::new("2026-06-15", "2026-07-02").within(&invoice), Some(false));
    /// // Not ISO 8601 — the engine declines to guess.
    /// assert_eq!(Period::new("1.6.2026", "14.6.2026").within(&invoice), None);
    /// ```
    #[must_use]
    pub fn within(&self, outer: &Period) -> Option<bool> {
        let all_iso = [&self.from, &self.to, &outer.from, &outer.to]
            .into_iter()
            .all(|s| is_iso_date(s));
        all_iso.then(|| self.from >= outer.from && self.to <= outer.to)
    }

    /// Check the period rules that are decidable from the dates themselves —
    /// **BR-29** (document) and **BR-30** (line), both *fatal*.
    ///
    /// > If both … period start date and … period end date are given then the end
    /// > date shall be later or equal to the start date.
    ///
    /// Silent when either endpoint is not a zero-padded ISO 8601 date: the engine
    /// stays date-type-agnostic and does not guess at a format it cannot order.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if both endpoints are ISO 8601 dates and the
    /// end precedes the start.
    pub fn validate(&self) -> Result<(), BillingError> {
        if self.is_ordered() == Some(false) {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "period end {:?} precedes its start {:?}; BR-29 / BR-30 require the \
                     end date to be later than or equal to the start date",
                    self.to, self.from
                ),
            });
        }
        Ok(())
    }
}

/// `YYYY-MM-DD`, zero-padded — the only shape a lexicographic compare orders the
/// same way a calendar does.
fn is_iso_date(s: &str) -> bool {
    s.len() == 10
        && s.as_bytes().iter().enumerate().all(|(i, b)| match i {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        })
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

    // BT-111 sums like every other VAT total, but only when both halves state one
    // in the same currency. The asymmetric case — one half reporting VAT in PLN and
    // the other not reporting it at all — cannot be summed: the missing figure is
    // *unstated*, not zero, and treating it as zero would understate BT-111 on a
    // document where BR-53 makes it mandatory.
    let vat_accounting_currency = match (
        doc_a.vat_accounting_currency(),
        doc_b.vat_accounting_currency(),
    ) {
        (None, None) => None,
        (Some(a), Some(b)) if a.currency() == b.currency() => {
            Some(a.with_vat_total(a.vat_total().checked_add(b.vat_total())?))
        }
        (Some(a), Some(b)) => {
            return Err(BillingError::CurrencyMismatch {
                left: a.currency(),
                right: b.currency(),
            });
        }
        _ => {
            return Err(BillingError::InvalidInput {
                reason: "cannot merge documents where only one states a VAT accounting \
                         currency (BT-6 / BT-111): the other's VAT in that currency is \
                         unstated, not zero"
                    .into(),
            });
        }
    };

    let merged = BillingDocument::from_raw(crate::document::DocumentParts {
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
        vat_accounting_currency,
    })?;
    // `from_raw` performs no validation, so without this the function could hand
    // back a document that fails `validate()`. Two individually lawful halves do
    // not always compose into a lawful whole: merging an "outside the scope of
    // VAT" document with a taxed one produces a breakdown that BR-O-11 forbids,
    // and only the combined document can show it. `AllocationRule` re-checks its
    // outputs for the same reason.
    merged.validate().map_err(|e| BillingError::InvalidInput {
        reason: format!("merging produced an inconsistent document: {e}"),
    })?;
    Ok(merged)
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
    let product = amount
        .into_decimal()
        .checked_mul(fraction)
        .ok_or_else(overflow)?;
    Amount::<5>::from_decimal_rounded(product, strategy)
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

    #[test]
    fn is_ordered_declines_anything_that_is_not_a_zero_padded_iso_date() {
        // A lexicographic compare orders `YYYY-MM-DD` the way a calendar does and
        // orders nothing else correctly — `01.06.2026` sorts after `30.06.2025`.
        // So the shape check has to be exact in both directions, and it has to
        // hold for *both* endpoints. Returning `Some` for a string it merely
        // guessed at is worse than returning `None`: BR-29 would then be checked
        // against an ordering that means nothing.
        assert_eq!(
            Period::new("2026-06-01", "2026-06-30").is_ordered(),
            Some(true)
        );

        // Right length, wrong separators — the length alone must not be enough.
        assert_eq!(Period::new("2026/06/01", "2026/06/30").is_ordered(), None);
        assert_eq!(Period::new("2026-06-01", "2026/06/30").is_ordered(), None);
        // Right shape at every position it has, but not ten characters — the
        // pattern alone must not be enough either.
        assert_eq!(Period::new("2026-06-0", "2026-06-3").is_ordered(), None);
        assert_eq!(Period::new("2026-06-011", "2026-06-030").is_ordered(), None);
        // Digits where the separators belong.
        assert_eq!(Period::new("2026060111", "2026063011").is_ordered(), None);

        // One good endpoint does not carry the other.
        assert_eq!(Period::new("2026-06-01", "30.6.2026").is_ordered(), None);
        assert_eq!(Period::new("1.6.2026", "2026-06-30").is_ordered(), None);
        assert_eq!(Period::new("2026-06-01", "").is_ordered(), None);
        assert_eq!(Period::new("", "2026-06-30").is_ordered(), None);

        // And when both are ISO, the comparison itself is inclusive at equality.
        assert_eq!(
            Period::new("2026-06-30", "2026-06-01").is_ordered(),
            Some(false)
        );
        assert_eq!(
            Period::new("2026-06-01", "2026-06-01").is_ordered(),
            Some(true)
        );
    }
}
