//! [`TaxCategory`] and [`TaxBreakdownEntry`] — the per-rate VAT breakdown.
//!
//! # Why a breakdown is not optional
//!
//! A single `tax_total` is not a lawful invoice. EU VAT Directive 2006/112/EC
//! art. 226 and German §14 UStG both require the taxable amount **per rate** and
//! the tax amount **per rate**, and any invoice carrying more than one rate — a
//! reduced-rate line beside a standard-rate line, a reverse-charge position, an
//! exempt feed-in credit — must show them separately.
//!
//! This module models the EN 16931 **VAT BREAKDOWN** group (BG-23), the semantic
//! structure that XRechnung, ZUGFeRD/Factur-X and Peppol BIS all serialise:
//!
//! | Field | EN 16931 | Meaning |
//! |-------|----------|---------|
//! | [`TaxBreakdownEntry::taxable_base`] | BT-116 | VAT category taxable amount |
//! | [`TaxBreakdownEntry::tax_amount`] | BT-117 | VAT category tax amount |
//! | [`TaxBreakdownEntry::category`] | BT-118 | VAT category code |
//! | [`TaxBreakdownEntry::rate`] | BT-119 | VAT category rate |
//! | [`TaxBreakdownEntry::exemption_reason`] | BT-120 | VAT exemption reason text |
//!
//! The engine produces the breakdown; it does **not** decide which category
//! applies. That is a jurisdictional question and stays with the caller.

use rust_decimal::Decimal;

use crate::amount::Amount;

/// EN 16931 BT-118 / UNTDID 5305 VAT category code.
///
/// The code tells a tax authority *why* a given base carries the rate it does —
/// a 0% line is not self-explanatory, and "zero-rated", "exempt", "reverse
/// charge" and "outside scope" have materially different legal meanings even
/// though all four produce no tax.
///
/// This enum is deliberately **not** `#[non_exhaustive]`: it mirrors a closed,
/// externally-governed code list, and callers legitimately need exhaustive
/// matching when mapping to an output format.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaxCategory {
    /// `S` — standard rate.
    Standard,
    /// `Z` — zero-rated goods. Taxable at 0%; input tax remains deductible.
    ZeroRated,
    /// `E` — exempt from VAT. Unlike zero-rating, input tax is generally not
    /// deductible. Requires an exemption reason.
    Exempt,
    /// `AE` — VAT reverse charge: the recipient accounts for the tax
    /// (§13b UStG, art. 194–199 of the VAT Directive).
    ReverseCharge,
    /// `K` — VAT-exempt intra-Community supply of goods.
    IntraCommunity,
    /// `G` — free export item, VAT not charged.
    Export,
    /// `O` — services outside the scope of VAT.
    OutOfScope,
    /// `L` — Canary Islands general indirect tax (IGIC).
    CanaryIslands,
    /// `M` — tax for production, services and importation in Ceuta and Melilla (IPSI).
    CeutaMelilla,
}

impl TaxCategory {
    /// The UNTDID 5305 code as written in EN 16931 / UBL / CII documents.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert_eq!(TaxCategory::Standard.code(), "S");
    /// assert_eq!(TaxCategory::ReverseCharge.code(), "AE");
    /// ```
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::Standard => "S",
            Self::ZeroRated => "Z",
            Self::Exempt => "E",
            Self::ReverseCharge => "AE",
            Self::IntraCommunity => "K",
            Self::Export => "G",
            Self::OutOfScope => "O",
            Self::CanaryIslands => "L",
            Self::CeutaMelilla => "M",
        }
    }

    /// Parse a UNTDID 5305 code (case-insensitive).
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert_eq!(TaxCategory::from_code("ae"), Some(TaxCategory::ReverseCharge));
    /// assert_eq!(TaxCategory::from_code("Q"), None);
    /// ```
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.to_ascii_uppercase().as_str() {
            "S" => Some(Self::Standard),
            "Z" => Some(Self::ZeroRated),
            "E" => Some(Self::Exempt),
            "AE" => Some(Self::ReverseCharge),
            "K" => Some(Self::IntraCommunity),
            "G" => Some(Self::Export),
            "O" => Some(Self::OutOfScope),
            "L" => Some(Self::CanaryIslands),
            "M" => Some(Self::CeutaMelilla),
            _ => None,
        }
    }

    /// Whether this category actually levies tax.
    ///
    /// Only `S`, `L` and `M` do. For every other category EN 16931 requires the
    /// category tax amount (BT-117) to be exactly zero — rules BR-Z-09, BR-E-09,
    /// BR-AE-09, BR-IC-09 and BR-O-09.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert!(TaxCategory::Standard.carries_tax());
    /// assert!(!TaxCategory::ZeroRated.carries_tax());
    /// assert!(!TaxCategory::ReverseCharge.carries_tax());
    /// ```
    #[must_use]
    pub fn carries_tax(&self) -> bool {
        matches!(
            self,
            Self::Standard | Self::CanaryIslands | Self::CeutaMelilla
        )
    }

    /// Whether EN 16931 **requires** an exemption reason (BT-120/BT-121).
    ///
    /// Required for `E`, `AE`, `K`, `G` and `O` (rules BR-E-10, BR-AE-10,
    /// BR-IC-10, BR-G-10, BR-O-10).
    ///
    /// Note the asymmetry that implementers most often get wrong: **`Z` and `E`
    /// both carry zero tax, but `Z` must *not* have an exemption reason and `E`
    /// must.** Zero-rating and exemption are legally distinct — input tax stays
    /// deductible under `Z` but generally not under `E`.
    #[must_use]
    pub fn requires_exemption_reason(&self) -> bool {
        matches!(
            self,
            Self::Exempt
                | Self::ReverseCharge
                | Self::IntraCommunity
                | Self::Export
                | Self::OutOfScope
        )
    }

    /// Whether EN 16931 **forbids** an exemption reason for this category.
    ///
    /// Forbidden for `S` (BR-S-10), `Z` (BR-Z-10), `L` and `M`: a taxed or
    /// zero-rated supply is not an exemption and needs no justification.
    #[must_use]
    pub fn forbids_exemption_reason(&self) -> bool {
        matches!(
            self,
            Self::Standard | Self::ZeroRated | Self::CanaryIslands | Self::CeutaMelilla
        )
    }
}

impl std::fmt::Display for TaxCategory {
    /// Honours width, fill and alignment.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.code())
    }
}

/// One line of the EN 16931 VAT BREAKDOWN (BG-23): the taxable base and tax
/// amount for a single (category, rate) pair.
///
/// Entries sharing a category and rate are **merged** by
/// [`crate::BillingDocument`], because BR-CO-18 requires exactly one breakdown
/// line per distinct pair.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "TaxBreakdownEntryRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxBreakdownEntry {
    /// BT-118 — VAT category code.
    pub category: TaxCategory,
    /// BT-119 — the rate as a fraction (`0.19`, not `19`). Zero for the
    /// zero-tax categories.
    pub rate: Decimal,
    /// BT-116 — the sum of line net amounts subject to this category and rate.
    pub taxable_base: Amount<5>,
    /// BT-117 — the tax charged on `taxable_base`.
    pub tax_amount: Amount<5>,
    /// BT-120 — why no tax applies. Required for the categories where
    /// [`TaxCategory::requires_exemption_reason`] is `true`.
    pub exemption_reason: Option<String>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct TaxBreakdownEntryRepr {
    category: TaxCategory,
    rate: Decimal,
    taxable_base: Amount<5>,
    tax_amount: Amount<5>,
    #[serde(default)]
    exemption_reason: Option<String>,
}

#[cfg(feature = "serde")]
impl TryFrom<TaxBreakdownEntryRepr> for TaxBreakdownEntry {
    type Error = crate::error::BillingError;
    fn try_from(r: TaxBreakdownEntryRepr) -> Result<Self, Self::Error> {
        let entry = Self {
            category: r.category,
            rate: r.rate,
            taxable_base: r.taxable_base,
            tax_amount: r.tax_amount,
            exemption_reason: r.exemption_reason,
        };
        // Fields are public, so serde reconstructs them directly. Re-run the
        // category and BR-CO-17 checks so an entry can never enter the process
        // in a state the type says is impossible.
        entry.validate()?;
        Ok(entry)
    }
}

impl TaxBreakdownEntry {
    /// The group key required by EN 16931 BR-CO-18: one breakdown line per
    /// distinct `(category, rate)` pair.
    ///
    /// The rate is **normalised** before comparison, because Peppol specifies
    /// that "for the VAT rate, only significant decimals should be considered,
    /// i.e. any difference in trailing zeros should not result in different VAT
    /// breakdowns". Without normalising, `0.19` and `0.190` would produce two
    /// breakdown lines for one rate — an invalid invoice.
    #[must_use]
    pub fn group_key(&self) -> (TaxCategory, Decimal) {
        (self.category, self.rate.normalize())
    }

    /// Validate this entry against the EN 16931 per-category rules.
    ///
    /// # Errors
    /// [`crate::BillingError::InvalidInput`] if a zero-tax category carries a
    /// non-zero tax amount, if a category requiring an exemption reason lacks
    /// one, or if a category forbidding one has it.
    pub fn validate(&self) -> Result<(), crate::error::BillingError> {
        use crate::error::BillingError;
        if !self.category.carries_tax() && !self.tax_amount.is_zero() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} carries no tax, but the breakdown reports {}",
                    self.category, self.tax_amount
                ),
            });
        }
        if self.category.requires_exemption_reason() && self.exemption_reason.is_none() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} requires an exemption reason (BT-120)",
                    self.category
                ),
            });
        }
        if self.category.forbids_exemption_reason() && self.exemption_reason.is_some() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} must not carry an exemption reason (BT-120)",
                    self.category
                ),
            });
        }
        // BR-CO-17: the tax amount must follow from the base and the rate.
        //
        // Checked with EN 16931's own tolerance rather than exact equality. The
        // CEN reference Schematron asserts `|BT-117 − base × rate| < 1.00`, and
        // the slack is necessary here too: merging two entries sums amounts that
        // were each rounded to 5 dp, so `Σ(base_i × rate)` and `(Σbase_i) × rate`
        // can legitimately differ in the last place.
        let expected = self
            .taxable_base
            .into_decimal()
            .checked_mul(self.rate)
            .ok_or(BillingError::MonetaryOverflow {
                precision: 5,
                input_value: None,
            })?;
        // `checked_sub`: `Decimal`'s `-` PANICS on overflow. `expected` is only
        // bounded by `Decimal::MAX`, so an opposing-signed `tax_amount` can push the
        // difference out of range — and both operands are attacker-controlled when
        // this entry arrives from JSON. A difference too large to represent is, of
        // course, also far outside the tolerance.
        let diff = match self.tax_amount.into_decimal().checked_sub(expected) {
            Some(d) => d.abs(),
            None => Decimal::MAX,
        };
        if diff >= Decimal::ONE {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT breakdown inconsistent (BR-CO-17): base {} × rate {} is {}, \
                     but the reported tax is {}",
                    self.taxable_base, self.rate, expected, self.tax_amount
                ),
            });
        }
        Ok(())
    }

    /// Create a breakdown entry.
    #[must_use]
    pub fn new(
        category: TaxCategory,
        rate: Decimal,
        taxable_base: Amount<5>,
        tax_amount: Amount<5>,
    ) -> Self {
        Self {
            category,
            rate,
            taxable_base,
            tax_amount,
            exemption_reason: None,
        }
    }

    /// Attach the BT-120 exemption reason text.
    #[must_use]
    pub fn with_exemption_reason(mut self, reason: impl Into<String>) -> Self {
        self.exemption_reason = Some(reason.into());
        self
    }

    /// The rate formatted as a percentage with trailing zeros stripped
    /// (`0.19` → `"19"`, `0.075` → `"7.5"`).
    #[must_use]
    pub fn rate_percent(&self) -> Decimal {
        self.rate
            .checked_mul(Decimal::ONE_HUNDRED)
            .map(|d| d.normalize())
            .unwrap_or(self.rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_roundtrip() {
        for c in [
            TaxCategory::Standard,
            TaxCategory::ZeroRated,
            TaxCategory::Exempt,
            TaxCategory::ReverseCharge,
            TaxCategory::IntraCommunity,
            TaxCategory::Export,
            TaxCategory::OutOfScope,
            TaxCategory::CanaryIslands,
            TaxCategory::CeutaMelilla,
        ] {
            assert_eq!(TaxCategory::from_code(c.code()), Some(c));
        }
        assert_eq!(TaxCategory::from_code("nonsense"), None);
    }

    #[test]
    fn exemption_reason_required_only_for_zero_tax_categories() {
        assert!(!TaxCategory::Standard.requires_exemption_reason());
        assert!(!TaxCategory::CanaryIslands.requires_exemption_reason());
        assert!(TaxCategory::Exempt.requires_exemption_reason());
        assert!(TaxCategory::ReverseCharge.requires_exemption_reason());
        assert!(TaxCategory::Export.requires_exemption_reason());
    }

    #[test]
    fn zero_rated_and_exempt_differ_on_the_reason_requirement() {
        // Both carry zero tax, but Z forbids a reason and E requires one.
        assert!(!TaxCategory::ZeroRated.carries_tax());
        assert!(!TaxCategory::Exempt.carries_tax());
        assert!(TaxCategory::ZeroRated.forbids_exemption_reason());
        assert!(!TaxCategory::ZeroRated.requires_exemption_reason());
        assert!(TaxCategory::Exempt.requires_exemption_reason());
        assert!(!TaxCategory::Exempt.forbids_exemption_reason());
    }

    #[test]
    fn group_key_normalises_trailing_zeros() {
        let a = TaxBreakdownEntry::new(
            TaxCategory::Standard,
            Decimal::from_str_exact("0.19").unwrap(),
            Amount::ZERO,
            Amount::ZERO,
        );
        let b = TaxBreakdownEntry::new(
            TaxCategory::Standard,
            Decimal::from_str_exact("0.1900").unwrap(),
            Amount::ZERO,
            Amount::ZERO,
        );
        assert_eq!(
            a.group_key(),
            b.group_key(),
            "0.19 and 0.1900 are one group"
        );
    }

    #[test]
    fn validate_enforces_category_rules() {
        let taxed = Amount::<5>::parse("19.00000").unwrap();
        // Zero-tax category with a non-zero amount.
        let bad = TaxBreakdownEntry::new(
            TaxCategory::ReverseCharge,
            Decimal::ZERO,
            Amount::ZERO,
            taxed,
        );
        assert!(bad.validate().is_err());
        // Missing required reason.
        let bad = TaxBreakdownEntry::new(
            TaxCategory::Exempt,
            Decimal::ZERO,
            Amount::ZERO,
            Amount::ZERO,
        );
        assert!(bad.validate().is_err());
        assert!(bad.with_exemption_reason("Art. 132").validate().is_ok());
        // Forbidden reason present.
        let bad = TaxBreakdownEntry::new(
            TaxCategory::Standard,
            Decimal::from_str_exact("0.19").unwrap(),
            Amount::ZERO,
            Amount::ZERO,
        )
        .with_exemption_reason("nope");
        assert!(bad.validate().is_err());
    }
}
