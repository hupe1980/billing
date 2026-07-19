//! [`AllocationRule`] — proportional and equal split across N recipients.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::amount::RoundingStrategy;
use crate::document::{BillingDocument, DocumentMeta};
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::vat::TaxBreakdownEntry;

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
/// Quantities are scaled alongside amounts (see [`LineItem::scaled`]) so each
/// allocated line stays internally consistent.  The single corrected line may
/// differ from `quantity × unit_price` by at most a few units of the last decimal
/// place — that residue is inherent to exact-sum allocation and is why the
/// correction is concentrated on one line rather than smeared across all of them.
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
        .map(|p| p.scaled(share, RoundingStrategy::MidpointAwayFromZero))
        .collect::<Result<Vec<_>, BillingError>>()?;
    let sum = Amount::checked_sum(items.iter().map(|p| p.net_amount))?;
    if sum != target {
        let correction = target.checked_sub(sum)?;
        if let Some(last) = items.last_mut() {
            last.net_amount = last.net_amount.checked_add(correction)?;
            // The correction can push a line across zero (a tiny credit becoming a
            // small debit), which would leave `Sign::Credit` on a positive amount —
            // a state `LineItem::validate` rejects.
            last.normalize_sign();
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
        .map(|p| p.scaled(share, RoundingStrategy::MidpointAwayFromZero))
        .collect::<Result<Vec<_>, BillingError>>()?;
    let mut scaled_disc: Vec<LineItem> = discount_positions
        .iter()
        .map(|p| p.scaled(share, RoundingStrategy::MidpointAwayFromZero))
        .collect::<Result<Vec<_>, BillingError>>()?;

    // Penny correction: ensure Σ(net) + Σ(discounts) == target_net_total.
    let combined_sum =
        Amount::checked_sum(scaled_net.iter().chain(&scaled_disc).map(|p| p.net_amount))?;
    if combined_sum != target_net_total {
        let correction = target_net_total.checked_sub(combined_sum)?;
        // Prefer correcting the last discount item (credits are typically smaller).
        // `normalize_sign` guards the case where the correction pushes the line
        // across zero — see `scale_with_target`.
        if let Some(last) = scaled_disc.last_mut() {
            last.net_amount = last.net_amount.checked_add(correction)?;
            last.normalize_sign();
        } else if let Some(last) = scaled_net.last_mut() {
            last.net_amount = last.net_amount.checked_add(correction)?;
            last.normalize_sign();
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
/// (1) and (2) hold because the **last recipient receives the arithmetic
/// remainder** (`total − Σ(others)`).  (3) follows because each document's
/// `gross_total` is *derived* as `net + tax` rather than rounded independently —
/// rounding all three separately breaks `net + tax == gross` for shares that do
/// not divide evenly.  (4) holds because a **penny correction** is applied to the
/// last item of each section so that `Σ(positions) == section_total`.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "ProportionalAllocationRepr"))]
#[derive(Debug, Clone)]
pub struct ProportionalAllocation {
    /// Fractional shares that must sum to `1.0 ± 1e-9`.
    shares: Vec<Decimal>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct ProportionalAllocationRepr {
    shares: Vec<Decimal>,
}

#[cfg(feature = "serde")]
impl TryFrom<ProportionalAllocationRepr> for ProportionalAllocation {
    type Error = BillingError;
    fn try_from(r: ProportionalAllocationRepr) -> Result<Self, Self::Error> {
        // Route deserialisation through `new` so shares loaded from config are
        // subject to the same checks as shares constructed in code.
        Self::new(r.shares)
    }
}

impl ProportionalAllocation {
    /// Validate that `shares` is non-empty, has no negative entry, and sums to
    /// `1.0 ± 1e-9`.
    ///
    /// # Errors
    /// - [`BillingError::InvalidInput`] if `shares` is empty or any share is negative.
    /// - [`BillingError::InvalidAllocationShares`] if the sum is not `1.0 ± 1e-9`.
    ///
    /// ```rust
    /// use billing::ProportionalAllocation;
    /// use rust_decimal::dec;
    ///
    /// assert!(ProportionalAllocation::new(vec![dec!(0.4), dec!(0.6)]).is_ok());
    /// assert!(ProportionalAllocation::new(vec![]).is_err());
    /// // Sums to 1.0, but a negative share is never a meaningful allocation:
    /// assert!(ProportionalAllocation::new(vec![dec!(1.5), dec!(-0.5)]).is_err());
    /// ```
    pub fn new(shares: Vec<Decimal>) -> Result<Self, BillingError> {
        if shares.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "ProportionalAllocation requires at least one share".into(),
            });
        }
        // A negative share would hand one recipient a credit funded by the others
        // while still summing to 1.0 — checked explicitly because the sum test
        // below cannot detect it.
        if let Some(neg) = shares.iter().find(|s| **s < Decimal::ZERO) {
            return Err(BillingError::InvalidInput {
                reason: format!("ProportionalAllocation shares must be non-negative, got {neg}"),
            });
        }
        let mut sum = Decimal::ZERO;
        for s in &shares {
            sum = sum
                .checked_add(*s)
                .ok_or(BillingError::InvalidAllocationShares {
                    sum: "overflow".into(),
                })?;
        }
        if (sum - Decimal::ONE).abs() > Decimal::new(1, 9) {
            return Err(BillingError::InvalidAllocationShares {
                sum: sum.to_string(),
            });
        }
        Ok(Self { shares })
    }

    /// The fractional shares.
    ///
    /// These may not sum to exactly `1.0`: an [`EqualAllocation`] of `n` parts uses
    /// `n` copies of `1/n`, which is inexact in decimal for most `n`. The split is
    /// still exact, because the last recipient receives the arithmetic remainder
    /// rather than its own scaled share.
    #[must_use]
    pub fn shares(&self) -> &[Decimal] {
        &self.shares
    }

    /// Construct without the sum-to-one check.
    ///
    /// Used only by [`EqualAllocation`], where the shares are `n` copies of `1/n`
    /// and cannot sum to exactly one for most `n` (`3 × 0.333…` falls one unit
    /// short). The last-recipient remainder in `allocate` absorbs that drift.
    ///
    /// Deliberately crate-private: it is the one path that may break the type's
    /// stated invariant, so external callers must go through
    /// [`ProportionalAllocation::new`].
    pub(crate) fn from_shares_unchecked(shares: Vec<Decimal>) -> Self {
        Self { shares }
    }
}

impl AllocationRule for ProportionalAllocation {
    fn allocate(&self, doc: &BillingDocument) -> Result<Vec<BillingDocument>, BillingError> {
        // Itemised advances cannot be split meaningfully: each one references a
        // specific advance invoice issued to a specific recipient, and slicing that
        // reference across N recipients would produce deduction tables that match
        // no document anyone was ever sent. Refusing is better than the silent
        // alternative of dropping them and re-billing money already collected.
        // A rounding adjustment is a property of one payable total; scaling it by a
        // share generally lands off the increment, and `from_raw` cannot carry the
        // rule that would let validate() check it.
        if doc.cash_rounding().is_some() {
            return Err(BillingError::InvalidInput {
                reason: "cannot allocate a document with a cash-rounding rule: \
                         allocate first, then apply rounding per recipient"
                    .into(),
            });
        }
        if !doc.advances().is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "cannot allocate a document carrying itemised advance payments: \
                         allocate the underlying positions and settle advances per recipient"
                    .into(),
            });
        }
        let n = self.shares.len();
        let mut docs = Vec::with_capacity(n);

        let mut net_remaining = doc.net_total();
        let mut tax_remaining = doc.tax_total();
        // Prepaid (BT-113) and rounding (BT-114) must be split too. Dropping them
        // would re-bill money the customer has already handed over: the recipients'
        // amounts due would sum to the gross rather than to the original amount due.
        let mut prepaid_remaining = doc.prepaid();
        let mut rounding_remaining = doc.rounding();
        // The VAT breakdown must be split alongside the totals: an allocated
        // document without its per-rate breakdown is not a lawful invoice. Each
        // entry is tracked separately so the last recipient absorbs the remainder
        // of each group, keeping Σ(recipient bases) and Σ(recipient tax) exact.
        let mut breakdown_remaining: Vec<(Amount<5>, Amount<5>)> = doc
            .tax_breakdown()
            .iter()
            .map(|e| (e.taxable_base, e.tax_amount))
            .collect();

        for (i, share) in self.shares.iter().enumerate() {
            let is_last = i == n - 1;

            let (net_total, tax_total) = if is_last {
                (net_remaining, tax_remaining)
            } else {
                let net = doc.net_total().checked_mul_qty(*share)?;
                let tax = doc.tax_total().checked_mul_qty(*share)?;
                net_remaining = net_remaining.checked_sub(net)?;
                tax_remaining = tax_remaining.checked_sub(tax)?;
                (net, tax)
            };

            let (prepaid, rounding) = if is_last {
                (prepaid_remaining, rounding_remaining)
            } else {
                let p = doc.prepaid().checked_mul_qty(*share)?;
                let r = doc.rounding().checked_mul_qty(*share)?;
                prepaid_remaining = prepaid_remaining.checked_sub(p)?;
                rounding_remaining = rounding_remaining.checked_sub(r)?;
                (p, r)
            };

            // Split each VAT breakdown group the same way as the totals.
            let mut breakdown = Vec::with_capacity(doc.tax_breakdown().len());
            for (i_entry, src) in doc.tax_breakdown().iter().enumerate() {
                let (base, tax) = if is_last {
                    breakdown_remaining[i_entry]
                } else {
                    let base = src.taxable_base.checked_mul_qty(*share)?;
                    let tax = src.tax_amount.checked_mul_qty(*share)?;
                    breakdown_remaining[i_entry].0 =
                        breakdown_remaining[i_entry].0.checked_sub(base)?;
                    breakdown_remaining[i_entry].1 =
                        breakdown_remaining[i_entry].1.checked_sub(tax)?;
                    (base, tax)
                };
                breakdown.push(TaxBreakdownEntry {
                    category: src.category,
                    rate: src.rate,
                    taxable_base: base,
                    tax_amount: tax,
                    exemption_reason: src.exemption_reason.clone(),
                });
            }

            // `gross` is DERIVED, never rounded independently.
            //
            // Rounding net, tax and gross separately breaks invariant 3
            // (`net + tax == gross`) whenever the share does not divide evenly,
            // because `round(n·s) + round(t·s) != round((n+t)·s)`. Splitting
            // 100.00 + 19% MwSt three ways used to produce three documents that
            // all failed `validate()` by one unit of the last decimal place.
            //
            // Deriving it keeps each document internally consistent, and the
            // cross-document sum still holds exactly:
            //   Σgross = Σnet + Σtax = doc.net_total + doc.tax_total = doc.gross_total
            // (the last recipient absorbs the net and tax remainders).
            let gross_total = net_total.checked_add(tax_total)?;

            // Scale all three position sections with penny correction so each
            // document passes assert_valid().
            let (net_positions, discount_positions) = scale_combined_net(
                doc.net_positions(),
                doc.discount_positions(),
                *share,
                net_total,
            )?;
            let tax_positions = scale_with_target(doc.tax_positions(), *share, tax_total)?;

            docs.push(BillingDocument::from_raw(crate::document::DocumentParts {
                // Clone the whole header and override only the invoice number, so
                // new DocumentMeta fields propagate automatically instead of being
                // silently dropped from every allocated document.
                meta: DocumentMeta {
                    invoice_number: format!("{}/{}", doc.meta.invoice_number, i + 1),
                    ..doc.meta.clone()
                },
                net_positions,
                tax_positions,
                discount_positions,
                net_total,
                tax_total,
                gross_total,
                tax_breakdown: breakdown,
                prepaid,
                rounding,
            })?);
            // The rustdoc promises "(4) each recipient's assert_valid() passes".
            // `from_raw` performs no total validation, so without this the promise
            // rested on the scaling arithmetic being right rather than on a check.
            docs.last().expect("just pushed").validate().map_err(|e| {
                BillingError::InvalidInput {
                    reason: format!("allocation produced an inconsistent document: {e}"),
                }
            })?;
        }
        Ok(docs)
    }
}

// ── EqualAllocation ───────────────────────────────────────────────────────────

/// Equal allocation: split N ways (each recipient gets `1/N` of the total).
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "EqualAllocationRepr"))]
#[derive(Debug, Clone)]
pub struct EqualAllocation {
    n: usize,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct EqualAllocationRepr {
    n: usize,
}

#[cfg(feature = "serde")]
impl TryFrom<EqualAllocationRepr> for EqualAllocation {
    type Error = BillingError;
    fn try_from(r: EqualAllocationRepr) -> Result<Self, Self::Error> {
        // Without this, `{"n":0}` deserialised into an `EqualAllocation` whose
        // `allocate` divided by zero and panicked.
        Self::new(r.n)
    }
}

impl EqualAllocation {
    /// Create an `EqualAllocation` that splits into `n` equal parts.
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] if `n == 0` — allocating to zero
    /// recipients has no meaning, and `1/n` would divide by zero.
    ///
    /// ```rust
    /// use billing::EqualAllocation;
    /// assert!(EqualAllocation::new(3).is_ok());
    /// assert!(EqualAllocation::new(0).is_err());
    /// ```
    pub fn new(n: usize) -> Result<Self, BillingError> {
        if n == 0 {
            return Err(BillingError::InvalidInput {
                reason: "EqualAllocation requires n > 0".into(),
            });
        }
        Ok(Self { n })
    }

    /// The number of equal recipients.
    #[must_use]
    pub fn n(&self) -> usize {
        self.n
    }
}

impl AllocationRule for EqualAllocation {
    fn allocate(&self, doc: &BillingDocument) -> Result<Vec<BillingDocument>, BillingError> {
        // `self.n > 0` is guaranteed by `new` and by the serde `try_from` shim,
        // so this division is safe; `checked_div` documents that rather than
        // relying on the reader to reconstruct the invariant.
        let share =
            Decimal::ONE
                .checked_div(Decimal::from(self.n))
                .ok_or(BillingError::InvalidInput {
                    reason: "EqualAllocation: n must be > 0".into(),
                })?;
        let shares = vec![share; self.n];
        // Bypass the sum=1.0 check (1/n × n ≈ 1 but may not be exactly 1.0
        // in Decimal).  The last-recipient remainder in `allocate` corrects any
        // residual drift, so the check is unnecessary here.
        ProportionalAllocation::from_shares_unchecked(shares).allocate(doc)
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
/// use rust_decimal::dec;
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
///   any fraction is negative, `scale > 28` (`Decimal`'s maximum), or the
///   intermediate arithmetic overflows.
/// - [`BillingError::InvalidAllocationShares`] if `Σ(fractions) ≠ 1.0 ± 1e-9`.
pub fn proportional_split(
    total: Decimal,
    fractions: &[Decimal],
    scale: u32,
) -> Result<Vec<Decimal>, BillingError> {
    use rust_decimal::prelude::ToPrimitive as _;

    // `Decimal`'s maximum scale. `Decimal::new(1, scale)` below PANICS above it,
    // and `round_dp_with_strategy` silently no-ops, so an unchecked `scale` turned
    // a caller typo into an abort inside a function that returns `Result`.
    const MAX_DECIMAL_SCALE: u32 = 28;

    if fractions.is_empty() {
        return Err(BillingError::InvalidInput {
            reason: "fractions must not be empty".into(),
        });
    }
    if scale > MAX_DECIMAL_SCALE {
        return Err(BillingError::InvalidInput {
            reason: format!(
                "proportional_split: scale must be <= {MAX_DECIMAL_SCALE}, got {scale}"
            ),
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
    // `checked_add`: `Decimal`'s `Sum` panics on overflow, and this runs on
    // caller-supplied values before any other bound has been established.
    let mut sum = Decimal::ZERO;
    for f in fractions {
        sum = sum
            .checked_add(*f)
            .ok_or_else(|| BillingError::InvalidAllocationShares {
                sum: "overflow".into(),
            })?;
    }
    // The share-sum tolerance must scale with `total`, not be a fixed 1e-9.
    //
    // A fixed absolute tolerance lets the resulting error grow without bound: with
    // `total = 1e18`, shares summing to 1 − 5e-10 pass the check and leave a
    // deficit of ~1e9 units, which the Hamilton step below can only ever
    // distribute `fractions.len()` of — silently returning parts that sum to less
    // than `total` and breaking this function's documented guarantee. Requiring the
    // drift to be worth less than the Hamilton step can absorb ties the tolerance
    // to its actual monetary impact.
    let drift = (sum - Decimal::ONE).abs();
    let unit = Decimal::new(1, scale); // 10^{-scale}
    let drift_value =
        drift
            .checked_mul(total.abs())
            .ok_or_else(|| BillingError::InvalidAllocationShares {
                sum: sum.to_string(),
            })?;
    // The Hamilton step distributes at most one unit per fraction, so a drift worth
    // up to `n` units is still fully absorbable. Bounding it at a single unit
    // rejected inputs that split exactly — 1e7 across three 0.333333333 shares, for
    // instance, which yields [3333333.34, 3333333.33, 3333333.33].
    let absorbable = unit
        .checked_mul(Decimal::from(fractions.len()))
        .ok_or_else(|| BillingError::InvalidAllocationShares {
            sum: sum.to_string(),
        })?;
    if drift > Decimal::new(1, 9) || drift_value >= absorbable {
        return Err(BillingError::InvalidAllocationShares {
            sum: sum.to_string(),
        });
    }

    // Normalise total to `scale` dp so parts can exactly sum to it.
    let total =
        total.round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointAwayFromZero);

    // Step 1 — floor each ideal share to `scale` dp.
    // Every product below uses `checked_mul`/`checked_sub`: `Decimal`'s operators
    // panic on overflow, which would break this function's `Result` contract for
    // a large `total`.
    let overflow = || BillingError::InvalidInput {
        reason: "proportional_split: intermediate arithmetic overflowed".into(),
    };
    let mut parts: Vec<Decimal> = fractions
        .iter()
        .map(|&f| {
            total.checked_mul(f).ok_or_else(overflow).map(|p| {
                p.round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::ToNegativeInfinity)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Step 2 — remainders (fractional parts discarded by the floor).
    let remainders: Vec<Decimal> = fractions
        .iter()
        .enumerate()
        .map(|(i, &f)| {
            total
                .checked_mul(f)
                .and_then(|p| p.checked_sub(parts[i]))
                .ok_or_else(overflow)
        })
        .collect::<Result<Vec<_>, _>>()?;

    // Step 3 — compute deficit: how many extra units must be distributed.
    //
    // deficit = total − Σ(floored parts).
    // Because floor(x) ≤ x and Σ(f_i) ≈ 1, deficit ≥ 0 always.
    // In the rare case where floating-point precision produces a tiny negative
    // deficit, it is safe to skip distribution (no over-allocation).
    let mut floored_sum = Decimal::ZERO;
    for p in &parts {
        floored_sum = floored_sum.checked_add(*p).ok_or_else(overflow)?;
    }
    let deficit_raw = total.checked_sub(floored_sum).ok_or_else(overflow)?;
    if deficit_raw < Decimal::ZERO {
        // Flooring can never over-allocate when the shares sum to 1, so a negative
        // deficit means they summed to more than 1. Returning the parts unchecked
        // would hand back a set summing to MORE than `total`.
        //
        // Reported as `InvalidInput` rather than `InvalidAllocationShares`: that
        // variant's message asserts the shares are outside 1.0 ± 1e-9, which is not
        // what happened here and would send a reader chasing the wrong thing.
        return Err(BillingError::InvalidInput {
            reason: format!(
                "proportional_split: fractions sum to {sum}, over-allocating {total} \
                 at scale {scale}"
            ),
        });
    }
    if deficit_raw.is_zero() {
        return Ok(parts);
    }

    // Round deficit to `scale` dp (it should already be exact, but guard against
    // any residual Decimal precision artefact).
    let deficit = deficit_raw
        .round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointAwayFromZero);

    // Convert deficit to an integer count of `unit`s.
    // deficit is always < fractions.len() × unit (sum of fractional remainders < n).
    let n_units = (deficit.checked_div(unit).ok_or_else(overflow)?)
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

    if n_units > parts.len() {
        // Unreachable given the tolerance check above; asserted rather than silently
        // truncated by `.take()`, which is how the old code lost units.
        return Err(BillingError::InvalidInput {
            reason: format!(
                "proportional_split: deficit of {n_units} units exceeds the {} \
                 fractions available to absorb it (fractions sum to {sum})",
                parts.len()
            ),
        });
    }
    for &idx in order.iter().take(n_units) {
        parts[idx] = parts[idx].checked_add(unit).ok_or_else(overflow)?;
    }

    // Assert the documented guarantee rather than trusting the derivation.
    let mut check = Decimal::ZERO;
    for p in &parts {
        check = check.checked_add(*p).ok_or_else(overflow)?;
    }
    if check != total {
        return Err(BillingError::InvalidInput {
            reason: format!(
                "proportional_split post-condition failed: parts sum to {check}, expected {total}"
            ),
        });
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
        let docs = EqualAllocation::new(2).unwrap().allocate(&doc).unwrap();
        assert_eq!(docs[0].net_total(), Amount::parse("25.00000").unwrap());
        assert_eq!(docs[1].net_total(), Amount::parse("25.00000").unwrap());
    }

    #[test]
    fn equal_three_way_exact_sum_and_valid() {
        // Classic penny test: 100 / 3 = 33.33333...
        let doc = simple_doc("100.00000");
        let docs = EqualAllocation::new(3).unwrap().allocate(&doc).unwrap();
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
        let _ = EqualAllocation::new(0).unwrap();
    }

    #[test]
    fn allocation_with_tax_preserves_gross_and_valid() {
        let pos = vec![
            LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ];
        let taxes: Vec<Box<dyn crate::tax::TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)).unwrap())];
        let doc =
            BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
        let docs = EqualAllocation::new(2).unwrap().allocate(&doc).unwrap();

        let gross_sum: Amount<5> = docs.iter().map(|d| d.gross_total()).sum();
        assert_eq!(gross_sum, doc.gross_total(), "gross_total must not drift");
        for d in &docs {
            d.assert_valid();
        }
    }
}
