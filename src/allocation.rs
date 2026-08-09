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

/// Split one amount across `shares` so the parts sum back to it **exactly**.
///
/// This is the primitive the whole allocation is built on, and the reason it is
/// per-*amount* rather than per-total is what makes EN 16931's **BR-S-08** survive
/// the split — see [`ProportionalAllocation`].
///
/// [`proportional_split`] is defined on non-negative totals, so a credit is split
/// by magnitude and negated afterwards. Negating an exact partition of `|v|` is an
/// exact partition of `v`, so the guarantee is unaffected by sign.
fn split_amount(value: Amount<5>, shares: &[Decimal]) -> Result<Vec<Amount<5>>, BillingError> {
    let negative = value.is_negative();
    let magnitude = value.checked_abs()?;
    proportional_split(magnitude.into_decimal(), shares, 5)?
        .into_iter()
        .map(|part| {
            // `proportional_split` at scale 5 returns at most five decimals, so this
            // conversion is exact and cannot round — see its own documentation.
            let amount = Amount::<5>::checked_from_decimal(part)?;
            if negative {
                amount.checked_neg()
            } else {
                Ok(amount)
            }
        })
        .collect()
}

/// Split every position across the recipients, returning `[recipient][position]`.
///
/// Each position is split **exactly** with [`split_amount`], so
/// `Σ(recipients) == the original position` holds line by line rather than only in
/// aggregate. Every total is then derived by summing a recipient's own positions,
/// which is what makes both the cross-recipient sums and BR-S-08 exact at once.
///
/// The structural parts — quantity, BG-27 / BG-28 line allowances, and the
/// BT-93 / BT-100 basis — are scaled by the share via [`LineItem::scaled`], and
/// only the amount comes from the exact split. They can therefore differ from
/// `share × amount` by at most one unit in the last place (10⁻⁵), which is three
/// orders of magnitude inside the ±0.02 that `PEPPOL-EN16931-R040` and
/// `PEPPOL-EN16931-R120` allow.
fn split_positions(
    positions: &[LineItem],
    shares: &[Decimal],
) -> Result<Vec<Vec<LineItem>>, BillingError> {
    let mut out: Vec<Vec<LineItem>> = vec![Vec::with_capacity(positions.len()); shares.len()];
    for position in positions {
        let parts = split_amount(position.net_amount, shares)?;
        for (recipient, amount) in parts.into_iter().enumerate() {
            let mut item =
                position.scaled(shares[recipient], RoundingStrategy::MidpointAwayFromZero)?;
            item.net_amount = amount;
            // The split can push a line across zero (a tiny credit becoming a small
            // debit), which would leave `Sign::Credit` on a positive amount — a
            // state `LineItem::validate` rejects.
            item.normalize_sign();
            out[recipient].push(item);
        }
    }
    Ok(out)
}

/// The sum of the positions attributed to `key`, over the buckets **BR-S-08**
/// names: invoice lines, document level charges and document level allowances.
///
/// Positions tagged [`crate::tags::VAT`] are excluded — they *are* the BG-23 the
/// identity is checked against.
fn attributed_sum(
    doc: &BillingDocument,
    key: &(crate::TaxCategory, Decimal),
) -> Result<Amount<5>, BillingError> {
    Amount::checked_sum(
        doc.all_positions()
            .filter(|p| !p.has_tag(crate::tags::VAT))
            .filter(|p| p.vat.is_some_and(|v| v.group_key() == *key))
            .map(|p| p.net_amount),
    )
}

// ── ProportionalAllocation ────────────────────────────────────────────────────

/// Proportional allocation: each recipient's share is `shares[i]`.
///
/// `shares` must sum to `1.0 ± 1e-9`.
///
/// ## Correctness guarantees
///
/// 1. `Σ(net_total)   == original.net_total`   — exact, no drift.
/// 2. `Σ(tax_total)   == original.tax_total`   — exact.
/// 3. `Σ(gross_total) == original.gross_total` — exact.
/// 4. Each recipient's [`BillingDocument::assert_valid`] passes.
/// 5. Each recipient's
///    [`verify_vat_attribution`](BillingDocument::verify_vat_attribution) passes —
///    EN 16931 **BR-S-08** — whenever the document being split satisfied it.
///
/// All five rest on one decision: **every position is split exactly**, so
/// `Σ(recipients) == the original position` holds line by line rather than only in
/// aggregate. Every total is then a sum of that recipient's own
/// positions, which gives (1)–(3) immediately; (3) additionally *derives* gross as
/// `net + tax` rather than rounding it, because rounding all three separately
/// breaks `net + tax == gross` for shares that do not divide evenly. Each VAT
/// group's taxable base is read off the very positions BR-S-08 checks it against,
/// which gives (5).
///
/// ## Why not split the totals and correct the positions?
///
/// That is the obvious construction — scale each position by the share, then apply
/// one penny correction per recipient so the positions sum to a separately-computed
/// total — and it was used until 0.13.0. It keeps (1)–(4) and quietly breaks (5):
/// the VAT breakdown is split per `(category, rate)` group while the positions are
/// corrected in aggregate, so a group's BT-116 and the positions attributed to it
/// land a minor unit apart. On a two-rate document that happened for roughly **45 %**
/// of recipients — each document internally exact, each one rejected by a validator.
///
/// The one case that cannot be fixed is the one that was already broken: where the
/// *source* document does not satisfy BR-S-08 for a group — because it carries a
/// breakdown with no attributed positions behind it — the group's base falls back to
/// a proportional split, and the share is no worse than what it came from.
///
/// ## What is not preserved
///
/// [`crate::AmountScale`] **cannot** survive a split: `100.00` three ways is
/// `33.333…`, and there is no two-decimal answer. Allocation keeps the sum exact at
/// the cost of precision, which is the right trade for money — but an allocated
/// document must be re-assembled at invoice precision before it is billed onward.
/// [`BillingDocument::fits_amount_scale`] is how you find out.
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
        let shares = &self.shares[..];
        let mut docs = Vec::with_capacity(n);

        // Every position is split EXACTLY, line by line, before any total exists.
        //
        // The previous construction scaled each position by the share and then
        // applied one penny correction per recipient to hit a separately-computed
        // `net_total`. That kept the cross-recipient sums exact and broke EN 16931
        // **BR-S-08**: the VAT breakdown was split per group while the positions
        // were corrected in aggregate, so a group's BT-116 and the positions
        // attributed to it could land a minor unit apart. On a two-rate document
        // that happened for roughly 45 % of recipients.
        //
        // Splitting each position exactly removes the aggregate correction
        // entirely: every total below is now a *sum of that recipient's own
        // positions*, so `Σ(recipients) == the original` follows from
        // `Σ(recipients) == the original` holding for each position, and each
        // group's taxable base can be derived from the very positions BR-S-08
        // checks it against.
        let net_parts = split_positions(doc.net_positions(), shares)?;
        let discount_parts = split_positions(doc.discount_positions(), shares)?;
        let tax_parts = split_positions(doc.tax_positions(), shares)?;

        // The settlement scalars are not positions, so they are split with the same
        // primitive directly. Dropping them would re-bill money the customer has
        // already handed over, and BT-111 is mandatory once BT-6 is present (BR-53).
        let prepaid_parts = split_amount(doc.prepaid(), shares)?;
        let rounding_parts = split_amount(doc.rounding(), shares)?;
        let accounting_parts = match doc.vat_accounting_currency() {
            None => None,
            Some(acct) => Some(split_amount(acct.vat_total(), shares)?),
        };

        // For each VAT breakdown group, decide once whether its base can be
        // *derived* from the split positions — which is what makes BR-S-08 exact —
        // or must fall back to splitting the stored figure.
        //
        // Deriving is only sound where the source document already satisfies
        // BR-S-08 for that group. Where it does not — a document assembled without
        // VAT attribution, or one whose breakdown was set by hand — deriving would
        // silently rewrite the base to zero. The fallback keeps the old behaviour,
        // which is exactly as good as the input deserved: an allocated share now
        // satisfies `verify_vat_attribution` **iff the document it came from did**.
        let derivable: Vec<bool> = doc
            .tax_breakdown()
            .iter()
            .map(|e| Ok(attributed_sum(doc, &e.group_key())? == e.taxable_base))
            .collect::<Result<Vec<_>, BillingError>>()?;
        let mut base_remaining: Vec<Amount<5>> =
            doc.tax_breakdown().iter().map(|e| e.taxable_base).collect();
        // The tax amounts keep the group-wise split: BR-CO-14 (`BT-110 = Σ BT-117`)
        // holds under it, and re-deriving them from positions is not possible —
        // a VAT position carries no link back to the group it belongs to.
        let tax_parts_by_group: Vec<Vec<Amount<5>>> = doc
            .tax_breakdown()
            .iter()
            .map(|e| split_amount(e.tax_amount, shares))
            .collect::<Result<Vec<_>, BillingError>>()?;

        for (i, share) in self.shares.iter().enumerate() {
            let is_last = i == n - 1;
            let net_positions = net_parts[i].clone();
            let discount_positions = discount_parts[i].clone();
            let tax_positions = tax_parts[i].clone();

            // Every total is a sum of this recipient's own positions.
            let net_total = Amount::checked_sum(
                net_positions
                    .iter()
                    .chain(&discount_positions)
                    .map(|p| p.net_amount),
            )?;
            let tax_total = Amount::checked_sum(tax_positions.iter().map(|p| p.net_amount))?;
            // `gross` is DERIVED, never rounded independently: rounding net, tax and
            // gross separately breaks `net + tax == gross` whenever the share does
            // not divide evenly, because `round(n·s) + round(t·s) != round((n+t)·s)`.
            let gross_total = net_total.checked_add(tax_total)?;

            let prepaid = prepaid_parts[i];
            let rounding = rounding_parts[i];
            let vat_accounting_currency = doc.vat_accounting_currency().map(|acct| {
                acct.with_vat_total(accounting_parts.as_ref().expect("set together")[i])
            });

            let mut breakdown = Vec::with_capacity(doc.tax_breakdown().len());
            for (g, src) in doc.tax_breakdown().iter().enumerate() {
                let key = src.group_key();
                let taxable_base = if derivable[g] {
                    // BR-S-08, by construction: the base *is* the sum of the
                    // positions this recipient carries in the group.
                    let derived = Amount::checked_sum(
                        net_positions
                            .iter()
                            .chain(&discount_positions)
                            .chain(&tax_positions)
                            .filter(|p| !p.has_tag(crate::tags::VAT))
                            .filter(|p| p.vat.is_some_and(|v| v.group_key() == key))
                            .map(|p| p.net_amount),
                    )?;
                    base_remaining[g] = base_remaining[g].checked_sub(derived)?;
                    derived
                } else if is_last {
                    base_remaining[g]
                } else {
                    let part = src.taxable_base.checked_mul_qty(*share)?;
                    base_remaining[g] = base_remaining[g].checked_sub(part)?;
                    part
                };
                breakdown.push(TaxBreakdownEntry {
                    category: src.category,
                    rate: src.rate,
                    taxable_base,
                    tax_amount: tax_parts_by_group[g][i],
                    exemption_reason: src.exemption_reason.clone(),
                    exemption_reason_code: src.exemption_reason_code.clone(),
                });
            }

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
                vat_accounting_currency,
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
/// - `Σ(parts) == total.round_dp(scale)` — exact, no drift, for every input the
///   share check accepts.
/// - Each part is rounded to exactly `scale` dp.
/// - The correction is spread as evenly as the deficit allows: every fraction
///   takes the same whole number of units, and only the leftover — fewer units
///   than there are fractions — is awarded by remainder. Where the fractions sum
///   to exactly one that leftover *is* the whole deficit, so no part is adjusted
///   by more than a single unit. Shares that sum slightly under one add a second
///   unit of deficit per fraction, and it is shared out rather than concentrated.
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
/// # Splitting a bare [`Amount`] by fractional shares
///
/// [`Amount::allocate`] takes **integer** ratios and [`Amount::distribute`] splits
/// evenly, so neither covers fractional shares such as `[0.4, 0.35, 0.25]`. Compose
/// this function at the amount's own precision, and the round trip is exact:
///
/// ```rust
/// use billing::{Amount, proportional_split};
/// use rust_decimal::dec;
///
/// let total = Amount::<5>::parse("100.00000")?;
/// let parts: Vec<Amount<5>> =
///     proportional_split(total.into_decimal(), &[dec!(0.4), dec!(0.35), dec!(0.25)], 5)?
///         .into_iter()
///         .map(Amount::<5>::checked_from_decimal)
///         .collect::<Result<_, _>>()?;
///
/// assert_eq!(parts, [
///     Amount::parse("40.00000")?, Amount::parse("35.00000")?, Amount::parse("25.00000")?,
/// ]);
/// assert_eq!(Amount::checked_sum(parts.into_iter())?, total);
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Passing `scale = 5` is what makes the conversion total: every part then carries at
/// most five decimals, so [`Amount::checked_from_decimal`] — which *refuses* excess
/// precision rather than rounding it — always succeeds, and the guarantee above
/// (`Σ(parts) == total`) carries straight through it. A coarser `scale` is fine too;
/// a finer one is not, and fails loudly rather than silently re-rounding.
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

    // `n_units` can exceed the number of fractions, so one unit each is not always
    // enough to close the deficit.
    //
    // Two independent sources feed it. The floor in step 1 discards up to one unit
    // per fraction, contributing almost `n` units on its own. The share-sum
    // tolerance above then admits a `drift_value` worth up to another `n` units —
    // that is precisely what it was widened to allow. Together they put `n_units`
    // just under `2n`.
    //
    // Three `0.333333333` shares of `25_000_000` at scale 2 is a concrete case: the
    // deficit is four units across three fractions. Treating that as an error
    // rejected an input the tolerance check had already accepted, and `.take(n)`
    // before it silently dropped the fourth unit, leaving Σ(parts) one cent short
    // of `total` — the exact guarantee this function exists to provide.
    //
    // Distributing `n_units / n` to every fraction and the remaining `n_units % n`
    // in Hamilton order is the same rule, generalised: Σ(parts) reaches `total` for
    // any `n_units`, and the fractions with the largest discarded remainders are
    // still the ones served first.
    // Written without a `base > 0` guard: when the deficit is smaller than the
    // number of fractions — the ordinary case — `base_units` is zero and the first
    // loop adds nothing. Guarding it would only trade a no-op pass for a branch
    // whose two sides are indistinguishable.
    let n = parts.len();
    let (base, extra) = (n_units / n, n_units % n);
    let base_units = unit.checked_mul(Decimal::from(base)).ok_or_else(overflow)?;
    for part in &mut parts {
        *part = part.checked_add(base_units).ok_or_else(overflow)?;
    }
    for &idx in order.iter().take(extra) {
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
