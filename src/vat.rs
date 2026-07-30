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
/// matching when mapping to an output format. The list is fixed by rules
/// **BR-CL-17** and **BR-CL-18**, which restrict BT-118 / BT-151 to exactly the
/// ten codes below — [`TaxCategory::ALL`] is that set, in the artefact's order.
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
    /// `B` — split payment (Italy, *scissione dei pagamenti*): the buyer remits
    /// the VAT directly to the tax authority instead of paying it to the supplier.
    ///
    /// Unlike the other "someone else pays" category (`AE`), the tax amount is
    /// **not** zero — the supply is taxed at the normal rate and the tax is stated;
    /// only the settlement route differs. The CEN artefacts make this observable by
    /// omission: `B` is the one category with no `BR-B-05`, no `BR-B-09` and no
    /// `BR-B-10`, so nothing constrains its rate, forces BT-117 to zero, or requires
    /// an exemption reason. It is therefore also the only category for which both
    /// [`requires_exemption_reason`](Self::requires_exemption_reason) and
    /// [`forbids_exemption_reason`](Self::forbids_exemption_reason) are `false`.
    ///
    /// Its two rules are jurisdictional rather than arithmetic, and stay with the
    /// caller — the engine only refuses states that cannot add up:
    ///
    /// - **BR-B-01** — an invoice using `B` shall be a domestic Italian invoice.
    /// - **BR-B-02** — `B` shall not appear in the same document as `S`.
    SplitPayment,
}

impl TaxCategory {
    /// Every UNCL 5305 code EN 16931 permits, in the order BR-CL-17 lists them.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert_eq!(TaxCategory::ALL.len(), 10);
    /// // Every code round-trips through `from_code`.
    /// for c in TaxCategory::ALL {
    ///     assert_eq!(TaxCategory::from_code(c.code()), Some(c));
    /// }
    /// ```
    pub const ALL: [Self; 10] = [
        Self::ReverseCharge,
        Self::CanaryIslands,
        Self::CeutaMelilla,
        Self::Exempt,
        Self::Standard,
        Self::ZeroRated,
        Self::Export,
        Self::OutOfScope,
        Self::IntraCommunity,
        Self::SplitPayment,
    ];

    /// The UNTDID 5305 code as written in EN 16931 / UBL / CII documents.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert_eq!(TaxCategory::Standard.code(), "S");
    /// assert_eq!(TaxCategory::ReverseCharge.code(), "AE");
    /// assert_eq!(TaxCategory::SplitPayment.code(), "B");
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
            Self::SplitPayment => "B",
        }
    }

    /// Parse a UNTDID 5305 code (case-insensitive).
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert_eq!(TaxCategory::from_code("ae"), Some(TaxCategory::ReverseCharge));
    /// assert_eq!(TaxCategory::from_code("B"), Some(TaxCategory::SplitPayment));
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
            "B" => Some(Self::SplitPayment),
            _ => None,
        }
    }

    /// Whether this category actually levies tax.
    ///
    /// True for `S`, `L`, `M` and `B`. For every other category EN 16931 requires
    /// the category tax amount (BT-117) to be exactly zero — rules BR-Z-09,
    /// BR-E-09, BR-AE-09, BR-IC-09, BR-G-09 and BR-O-09. There is deliberately no
    /// `BR-B-09`: under split payment the supply is taxed normally and the tax is
    /// stated, the buyer merely remits it to the authority rather than to the
    /// supplier.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert!(TaxCategory::Standard.carries_tax());
    /// assert!(TaxCategory::SplitPayment.carries_tax());
    /// assert!(!TaxCategory::ZeroRated.carries_tax());
    /// assert!(!TaxCategory::ReverseCharge.carries_tax());
    /// ```
    #[must_use]
    pub fn carries_tax(&self) -> bool {
        matches!(
            self,
            Self::Standard | Self::CanaryIslands | Self::CeutaMelilla | Self::SplitPayment
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
    /// Forbidden for `S` (BR-S-10), `Z` (BR-Z-10), `L` (BR-AF-10) and `M`
    /// (BR-AG-10): a taxed or zero-rated supply is not an exemption and needs no
    /// justification.
    ///
    /// `B` is neither required nor forbidden — the artefacts contain no `BR-B-10` —
    /// so it is the single category for which this and
    /// [`requires_exemption_reason`](Self::requires_exemption_reason) are both
    /// `false`. Code that assumes exactly one of the two holds is wrong on `B`.
    #[must_use]
    pub fn forbids_exemption_reason(&self) -> bool {
        matches!(
            self,
            Self::Standard | Self::ZeroRated | Self::CanaryIslands | Self::CeutaMelilla
        )
    }

    /// Whether EN 16931 requires a **strictly positive** line/allowance/charge VAT
    /// rate (BT-152 / BT-96 / BT-103) for this category.
    ///
    /// True for `S` alone. **BR-S-05** says the rate "shall be greater than zero",
    /// whereas the corresponding rules for `L` and `M` — BR-AF-05 and BR-AG-05 —
    /// say "0 (zero) or greater than zero", and `B` has no rule at all. Treating
    /// all taxed categories alike would wrongly reject a lawful 0 % IGIC line.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    /// assert!(TaxCategory::Standard.requires_positive_rate());
    /// assert!(!TaxCategory::CanaryIslands.requires_positive_rate()); // BR-AF-05 allows 0
    /// assert!(!TaxCategory::SplitPayment.requires_positive_rate());  // no BR-B-05
    /// ```
    #[must_use]
    pub fn requires_positive_rate(&self) -> bool {
        matches!(self, Self::Standard)
    }

    /// Whether EN 16931 requires a **zero** line/allowance/charge VAT rate for this
    /// category — rules BR-Z-05, BR-E-05, BR-AE-05, BR-IC-05, BR-G-05 and BR-O-05.
    ///
    /// For `O` this crate stores zero, but a consumer must **omit** the rate rather
    /// than write `0` — see [`states_rate`](Self::states_rate).
    #[must_use]
    pub fn requires_zero_rate(&self) -> bool {
        !self.carries_tax()
    }

    /// Whether a **line, allowance or charge** in this category may state its VAT
    /// rate — BT-152, BT-96, BT-103.
    ///
    /// **`O` is the only category where the answer is no**, and the distinction is
    /// not cosmetic. The other zero-tax categories say the rate *"shall be 0
    /// (zero)"* — present, and zero (BR-Z-05, BR-E-05, BR-AE-05, BR-IC-05,
    /// BR-G-05). `O` says the opposite:
    ///
    /// > `[BR-O-05]` An Invoice line (BG-25) where the VAT category code (BT-151)
    /// > is "Not subject to VAT" shall **not contain** an Invoiced item VAT rate
    /// > (BT-152).
    ///
    /// BR-O-06 and BR-O-07 say the same for BT-96 and BT-103. Because
    /// [`LineVat::rate`] is a plain `Decimal` rather than an `Option`, an `O`
    /// position stores `0` — so a consumer emitting UBL or CII must **suppress the
    /// element** for `O` instead of writing `<cbc:Percent>0</cbc:Percent>`, which
    /// is a fatal violation. This predicate is that instruction, in code.
    ///
    /// # It does **not** apply to BT-119
    ///
    /// The VAT *breakdown* rate ([`TaxBreakdownEntry::rate`]) is a different term
    /// and is governed by different rules. No BR-O rule suppresses it, and
    /// XRechnung's **BR-DE-14** requires it unconditionally — *"Das Element 'VAT
    /// category rate' (BT-119) muss übermittelt werden"*, fatal, with no category
    /// exception. Applying this predicate to BG-23 would produce an invoice that
    /// fails the KoSIT validator.
    ///
    /// ```rust
    /// use billing::TaxCategory;
    ///
    /// // Every other zero-tax category states an explicit 0 on its lines …
    /// assert!(TaxCategory::ReverseCharge.states_rate());
    /// assert!(TaxCategory::ZeroRated.states_rate());
    /// // … while `O` must not state one at all (BR-O-05 / BR-O-06 / BR-O-07).
    /// assert!(!TaxCategory::OutOfScope.states_rate());
    /// ```
    #[must_use]
    pub fn states_rate(&self) -> bool {
        !matches!(self, Self::OutOfScope)
    }
}

impl std::fmt::Display for TaxCategory {
    /// Honours width, fill and alignment.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.code())
    }
}

/// The VAT treatment of a single position — EN 16931 **BT-151 / BT-152** on an
/// invoice line (BG-30), **BT-95 / BT-96** on a document level allowance (BG-20),
/// **BT-102 / BT-103** on a document level charge (BG-21).
///
/// EN 16931 requires this on every line: BR-CO-04 makes BT-151 mandatory, BR-32
/// and BR-37 do the same for allowances and charges. It is what BR-S-08 and its
/// siblings check the VAT breakdown against — for each `(category, rate)`, the
/// breakdown's taxable amount (BT-116) must equal the sum of the line net amounts
/// plus charges minus allowances carrying that pair.
///
/// [`crate::BillingDocument`] fills this in during assembly from the layer that
/// covers the position ([`crate::TaxLayer::covers`]), so a document built by this
/// crate carries the attribution the standard asks for. Set it explicitly with
/// [`crate::LineItemBuilder::vat`] when the caller already knows it.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "LineVatRepr"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LineVat {
    /// BT-151 / BT-95 / BT-102 — the UNCL 5305 VAT category code.
    pub category: TaxCategory,
    /// BT-152 / BT-96 / BT-103 — the rate as a fraction (`0.19`, not `19`).
    pub rate: Decimal,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct LineVatRepr {
    category: TaxCategory,
    rate: Decimal,
}

#[cfg(feature = "serde")]
impl TryFrom<LineVatRepr> for LineVat {
    type Error = crate::error::BillingError;
    fn try_from(r: LineVatRepr) -> Result<Self, Self::Error> {
        Self::new(r.category, r.rate)
    }
}

impl LineVat {
    /// Create a VAT attribution, checking the category's rate rule.
    ///
    /// # Errors
    /// [`crate::BillingError::InvalidInput`] if the rate contradicts the category:
    /// zero (or negative) under `S` (BR-S-05), or non-zero under a category that
    /// levies none (BR-Z-05, BR-E-05, BR-AE-05, BR-IC-05, BR-G-05, BR-O-05).
    ///
    /// ```rust
    /// use billing::{LineVat, TaxCategory};
    /// use rust_decimal::dec;
    ///
    /// assert!(LineVat::new(TaxCategory::Standard, dec!(0.19)).is_ok());
    /// // BR-S-05 — a standard-rated line at 0 % is not standard-rated.
    /// assert!(LineVat::new(TaxCategory::Standard, dec!(0)).is_err());
    /// // BR-AE-05 — reverse charge carries no rate.
    /// assert!(LineVat::new(TaxCategory::ReverseCharge, dec!(0.19)).is_err());
    /// // `L` may legitimately be 0 % (BR-AF-05), unlike `S`.
    /// assert!(LineVat::new(TaxCategory::CanaryIslands, dec!(0)).is_ok());
    /// ```
    pub fn new(category: TaxCategory, rate: Decimal) -> Result<Self, crate::error::BillingError> {
        let this = Self { category, rate };
        this.validate()?;
        Ok(this)
    }

    /// Re-check the category/rate consistency of an existing value.
    ///
    /// # Errors
    /// As [`LineVat::new`].
    pub fn validate(&self) -> Result<(), crate::error::BillingError> {
        use crate::error::BillingError;
        if self.category.requires_positive_rate() && self.rate <= Decimal::ZERO {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} requires a rate greater than zero (BR-S-05), got {}",
                    self.category, self.rate
                ),
            });
        }
        if self.category.requires_zero_rate() && !self.rate.is_zero() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} levies no tax, so its rate must be 0, got {}",
                    self.category, self.rate
                ),
            });
        }
        if self.rate.is_sign_negative() && !self.rate.is_zero() {
            return Err(BillingError::InvalidInput {
                reason: format!("VAT rate must not be negative, got {}", self.rate),
            });
        }
        Ok(())
    }

    /// The group this position contributes to in the VAT breakdown, with the rate
    /// normalised exactly as [`TaxBreakdownEntry::group_key`] normalises it.
    #[must_use]
    pub fn group_key(&self) -> (TaxCategory, Decimal) {
        (self.category, self.rate.normalize())
    }

    /// The rate as a percentage with trailing zeros stripped (`0.19` → `19`).
    #[must_use]
    pub fn rate_percent(&self) -> Decimal {
        self.rate
            .checked_mul(Decimal::ONE_HUNDRED)
            .map(|d| d.normalize())
            .unwrap_or(self.rate)
    }
}

impl std::fmt::Display for LineVat {
    /// `"S 19%"`, `"AE 0%"`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}%", self.category, self.rate_percent())
    }
}

/// One line of the EN 16931 VAT BREAKDOWN (BG-23): the taxable base and tax
/// amount for a single (category, rate) pair.
///
/// Entries sharing a category and rate are **merged** by
/// [`crate::BillingDocument`] — see [`TaxBreakdownEntry::group_key`] for which
/// rules make that mandatory.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "TaxBreakdownEntryRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxBreakdownEntry {
    /// BT-118 — VAT category code.
    pub category: TaxCategory,
    /// BT-119 — the rate as a fraction (`0.19`, not `19`). Zero for the zero-tax
    /// categories.
    ///
    /// **Always emit it**, including under `O`. BT-119 is *not* covered by
    /// BR-O-05/06/07 — those forbid the *line*, *allowance* and *charge* rates
    /// (BT-152 / BT-96 / BT-103), which is what [`TaxCategory::states_rate`]
    /// describes. XRechnung goes further and requires BT-119 unconditionally:
    ///
    /// > `[BR-DE-14]` Das Element "VAT category rate" (BT-119) muss übermittelt
    /// > werden.
    ///
    /// — **fatal**, with no category exception. Suppressing BT-119 for `O` on the
    /// strength of BR-O-05 is therefore a validation failure, not a safe
    /// simplification.
    pub rate: Decimal,
    /// BT-116 — the sum of line net amounts subject to this category and rate.
    pub taxable_base: Amount<5>,
    /// BT-117 — the tax charged on `taxable_base`.
    pub tax_amount: Amount<5>,
    /// BT-120 — the exemption reason as **free text**.
    ///
    /// One of this and [`exemption_reason_code`](Self::exemption_reason_code) is
    /// required for the categories where
    /// [`TaxCategory::requires_exemption_reason`] is `true`, and **both** are
    /// forbidden where [`TaxCategory::forbids_exemption_reason`] is.
    pub exemption_reason: Option<String>,
    /// BT-121 — the exemption reason as a **code** from the CEF VATEX list
    /// (`"VATEX-EU-AE"`, `"VATEX-EU-IC"`, `"VATEX-EU-O"`).
    ///
    /// A lawful *alternative* to [`exemption_reason`](Self::exemption_reason), not
    /// an addition to it: BR-E-10, BR-AE-10, BR-IC-10, BR-G-10 and BR-O-10 each
    /// require "a VAT exemption reason code (BT-121) **or** a VAT exemption reason
    /// text (BT-120)". A caller holding only the code no longer has to invent
    /// prose to satisfy the engine.
    ///
    /// **BR-CL-22** restricts the value to the VATEX code list; the engine does not
    /// check membership, for the same reason it does not check UNCL 5189 or
    /// UN/ECE Rec 20 — it carries no copy of the list, and a stale embedded copy
    /// would be worse than none.
    pub exemption_reason_code: Option<String>,
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
    #[serde(default)]
    exemption_reason_code: Option<String>,
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
            exemption_reason_code: r.exemption_reason_code,
        };
        // Fields are public, so serde reconstructs them directly. Re-run the
        // category and BR-CO-17 checks so an entry can never enter the process
        // in a state the type says is impossible.
        entry.validate()?;
        Ok(entry)
    }
}

impl TaxBreakdownEntry {
    /// The grouping key EN 16931 forces on the VAT breakdown: one line per
    /// distinct `(category, rate)` pair.
    ///
    /// The requirement comes from two different places depending on the category,
    /// and the standard's asymmetry between them is deliberate:
    ///
    /// | Categories | Rule | Wording |
    /// |---|---|---|
    /// | `S`, `L`, `M` | BR-S-08, BR-AF-08, BR-AG-08 | *"For each different value of VAT category rate (BT-119) … the VAT category taxable amount (BT-116) … shall equal the sum of …"* — the equality can only hold for each rate if entries at that rate are merged |
    /// | `Z`, `E`, `AE`, `K`, `G`, `O` | BR-Z-01, BR-E-01, BR-AE-01, BR-IC-01, BR-G-01, BR-O-01 | *"… shall contain in the VAT breakdown (BG-23) **exactly one** VAT category code (BT-118) equal with …"* |
    ///
    /// The taxed categories say *"at least one"* because they can legitimately
    /// appear at several rates; the zero-tax categories say *"exactly one"* because
    /// they cannot.
    ///
    /// This is **not** BR-CO-18, which is a different rule entirely — *"An Invoice
    /// shall at least have one VAT breakdown group (BG-23)"* — and is checked by
    /// [`crate::BillingDocument::validate`] instead.
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
        // BR-E-10, BR-AE-10, BR-IC-10, BR-G-10 and BR-O-10 each require "a VAT
        // exemption reason code (BT-121) OR a VAT exemption reason text (BT-120)" —
        // the two are alternatives, so demanding the text would reject a lawful
        // invoice from a caller who holds only the code.
        if self.category.requires_exemption_reason()
            && self.exemption_reason.is_none()
            && self.exemption_reason_code.is_none()
        {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} requires an exemption reason text (BT-120) \
                     or reason code (BT-121)",
                    self.category
                ),
            });
        }
        // BR-S-10, BR-Z-10, BR-AF-10 and BR-AG-10 forbid *both* ("shall not have a
        // VAT exemption reason code (BT-121) or VAT exemption reason text
        // (BT-120)"), so checking only the text would let the code through.
        if self.category.forbids_exemption_reason()
            && (self.exemption_reason.is_some() || self.exemption_reason_code.is_some())
        {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} must not carry an exemption reason text (BT-120) \
                     or reason code (BT-121)",
                    self.category
                ),
            });
        }
        // A (S, 0 %) group is unsatisfiable rather than merely unusual: BR-S-05
        // forbids a standard-rated line from carrying a zero rate, and BR-S-08
        // defines this group's BT-116 as the sum of exactly those lines. `Z` is the
        // category for a supply taxed at zero. `L` and `M` are excluded here —
        // BR-AF-05 and BR-AG-05 explicitly permit a zero rate — and so is `B`,
        // which has no rate rule at all.
        if self.category.requires_positive_rate() && self.rate.is_zero() {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "VAT category {} requires a rate greater than zero (BR-S-05); \
                     use {} for a supply taxed at zero",
                    self.category,
                    TaxCategory::ZeroRated
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
            exemption_reason_code: None,
        }
    }

    /// Attach the BT-120 exemption reason text.
    #[must_use]
    pub fn with_exemption_reason(mut self, reason: impl Into<String>) -> Self {
        self.exemption_reason = Some(reason.into());
        self
    }

    /// Attach the BT-121 exemption reason code (CEF VATEX).
    ///
    /// Satisfies BR-E-10 / BR-AE-10 / BR-IC-10 / BR-G-10 / BR-O-10 on its own —
    /// the text is not additionally required.
    ///
    /// ```rust
    /// use billing::{TaxBreakdownEntry, TaxCategory, Amount};
    /// use rust_decimal::Decimal;
    ///
    /// // Reverse charge stated by code alone, with no invented prose.
    /// let e = TaxBreakdownEntry::new(
    ///     TaxCategory::ReverseCharge, Decimal::ZERO, Amount::ZERO, Amount::ZERO,
    /// ).with_exemption_reason_code("VATEX-EU-AE");
    /// assert!(e.validate().is_ok());
    /// ```
    #[must_use]
    pub fn with_exemption_reason_code(mut self, code: impl Into<String>) -> Self {
        self.exemption_reason_code = Some(code.into());
        self
    }

    /// Whether this entry states an exemption reason in either form —
    /// BT-120 text or BT-121 code.
    #[must_use]
    pub fn has_exemption_reason(&self) -> bool {
        self.exemption_reason.is_some() || self.exemption_reason_code.is_some()
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
        for c in TaxCategory::ALL {
            assert_eq!(TaxCategory::from_code(c.code()), Some(c));
        }
        assert_eq!(TaxCategory::from_code("nonsense"), None);
    }

    /// BR-CL-17 / BR-CL-18 permit exactly these ten codes, and no others.
    #[test]
    fn all_matches_the_uncl_5305_subset_br_cl_17_permits() {
        let mut codes: Vec<&str> = TaxCategory::ALL.iter().map(TaxCategory::code).collect();
        codes.sort_unstable();
        let mut expected = ["AE", "L", "M", "E", "S", "Z", "G", "O", "K", "B"];
        expected.sort_unstable();
        assert_eq!(codes, expected);
    }

    /// `B` is the one category where neither exemption-reason predicate holds —
    /// the artefacts contain no BR-B-10 requiring a reason and none forbidding one.
    /// Every *other* category satisfies exactly one of the two.
    #[test]
    fn split_payment_is_the_only_category_with_neither_reason_rule() {
        for c in TaxCategory::ALL {
            let n = usize::from(c.requires_exemption_reason())
                + usize::from(c.forbids_exemption_reason());
            if c == TaxCategory::SplitPayment {
                assert_eq!(n, 0, "B must be governed by neither rule");
            } else {
                assert_eq!(n, 1, "{c} must be governed by exactly one rule");
            }
        }
    }

    /// Split payment is taxed at the normal rate — there is no BR-B-09 forcing
    /// BT-117 to zero, unlike the other "someone else pays" category, `AE`.
    #[test]
    fn split_payment_carries_tax_unlike_reverse_charge() {
        assert!(TaxCategory::SplitPayment.carries_tax());
        assert!(!TaxCategory::ReverseCharge.carries_tax());

        let entry = TaxBreakdownEntry::new(
            TaxCategory::SplitPayment,
            Decimal::from_str_exact("0.22").unwrap(),
            Amount::parse("1000.00000").unwrap(),
            Amount::parse("220.00000").unwrap(),
        );
        assert!(
            entry.validate().is_ok(),
            "B must be allowed a non-zero BT-117"
        );
    }

    /// BR-S-05 demands a rate above zero; BR-AF-05 and BR-AG-05 explicitly allow
    /// zero, and `B` has no rate rule at all.
    #[test]
    fn only_standard_requires_a_positive_rate() {
        use rust_decimal::dec;
        assert!(LineVat::new(TaxCategory::Standard, dec!(0)).is_err());
        assert!(LineVat::new(TaxCategory::CanaryIslands, dec!(0)).is_ok());
        assert!(LineVat::new(TaxCategory::CeutaMelilla, dec!(0)).is_ok());
        assert!(LineVat::new(TaxCategory::SplitPayment, dec!(0)).is_ok());
        assert!(LineVat::new(TaxCategory::SplitPayment, dec!(0.22)).is_ok());
        // Zero-tax categories must not carry a rate (BR-Z-05 and siblings).
        for c in TaxCategory::ALL.into_iter().filter(|c| !c.carries_tax()) {
            assert!(
                LineVat::new(c, dec!(0.19)).is_err(),
                "{c} must reject a rate"
            );
            assert!(LineVat::new(c, dec!(0)).is_ok());
        }
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
