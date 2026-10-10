//! VAT as an attribute: [`TaxCategory`], [`Exemption`], [`Vat`], the derived
//! [`VatGroup`] (BG-23) and [`split_gross`].
//!
//! EN 16931 states a VAT category and rate on every line (BT-151/152),
//! allowance (BT-95/96) and charge (BT-102/103); the VAT breakdown (BG-23) is a
//! group-by over those attributes with the tax rounded **once** per group. This
//! module holds the attribute; [`Document`](crate::Document) does the grouping.

use alloc::string::String;
use core::fmt;

use rust_decimal::Decimal;

use crate::{Amount, AmountScale, Error, ErrorKind, Percent, Result};

/// A VAT category code (UNTDID 5305, the subset BR-CL-17 permits).
///
/// `#[non_exhaustive]`: a later edition of the code list adds variants without a
/// breaking change. Serialises as its code (`"S"`, `"AE"` …).
///
/// ```
/// use billing::TaxCategory;
///
/// assert_eq!(TaxCategory::ReverseCharge.code(), "AE");
/// assert_eq!(TaxCategory::from_code("ae"), Some(TaxCategory::ReverseCharge));
/// assert!(!TaxCategory::OutOfScope.states_rate()); // BR-O-05: no rate on the line
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TaxCategory {
    /// `S` — standard rate (rate > 0, BR-S-05).
    Standard,
    /// `Z` — zero rated (rate 0, no exemption reason: BR-Z-05, BR-Z-10).
    ZeroRated,
    /// `E` — exempt (rate 0, reason required: BR-E-05, BR-E-10).
    Exempt,
    /// `AE` — reverse charge (BR-AE-05, BR-AE-10).
    ReverseCharge,
    /// `K` — intra-community supply (BR-IC-05, BR-IC-10).
    IntraCommunity,
    /// `G` — export outside the EU (BR-G-05, BR-G-10).
    Export,
    /// `O` — not subject to VAT: no rate is stated (BR-O-05), reason required
    /// (BR-O-10), and no other group may coexist (BR-O-11).
    OutOfScope,
    /// `L` — Canary Islands IGIC (rate ≥ 0, BR-AF-05).
    CanaryIslands,
    /// `M` — Ceuta and Melilla IPSI (rate ≥ 0, BR-AG-05).
    CeutaMelilla,
    /// `B` — Italian split payment: taxed normally, never beside `S` (BR-B-02).
    SplitPayment,
}

impl TaxCategory {
    /// Every category, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::Standard,
        Self::ZeroRated,
        Self::Exempt,
        Self::ReverseCharge,
        Self::IntraCommunity,
        Self::Export,
        Self::OutOfScope,
        Self::CanaryIslands,
        Self::CeutaMelilla,
        Self::SplitPayment,
    ];

    /// The UNTDID 5305 code.
    #[must_use]
    pub const fn code(self) -> &'static str {
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

    /// The category of a code, case-insensitively.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|c| c.code().eq_ignore_ascii_case(code))
    }

    /// Whether a line, allowance or charge in this category states its rate
    /// (BT-152/96/103). `false` only for `O` (BR-O-05/06/07). The breakdown
    /// rate BT-119 is a different term and is always stated.
    #[must_use]
    pub const fn states_rate(self) -> bool {
        !matches!(self, Self::OutOfScope)
    }

    /// Whether the category levies tax at its rate (`S`, `L`, `M`, `B`); every
    /// other category has rate 0 and tax 0.
    #[must_use]
    pub const fn carries_tax(self) -> bool {
        matches!(
            self,
            Self::Standard | Self::CanaryIslands | Self::CeutaMelilla | Self::SplitPayment
        )
    }

    const fn needs_exemption(self) -> bool {
        matches!(
            self,
            Self::Exempt
                | Self::ReverseCharge
                | Self::IntraCommunity
                | Self::Export
                | Self::OutOfScope
        )
    }

    const fn forbids_exemption(self) -> bool {
        matches!(
            self,
            Self::Standard | Self::ZeroRated | Self::CanaryIslands | Self::CeutaMelilla
        )
    }
}

impl fmt::Display for TaxCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.code())
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for TaxCategory {
    fn serialize<S: serde::Serializer>(&self, s: S) -> core::result::Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for TaxCategory {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> core::result::Result<Self, D::Error> {
        crate::decimal::serde_str::from_str(
            d,
            |s| Self::from_code(s).ok_or_else(|| ErrorKind::Malformed { input: s.into() }.into()),
            "a VAT category code such as \"S\"",
        )
    }
}

/// The VAT exemption reason (BT-120 text, BT-121 code), required for `E`, `AE`,
/// `K`, `G` and `O`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Exemption {
    /// BT-120 only.
    Text(String),
    /// BT-121 only (e.g. `"VATEX-EU-AE"`).
    Code(String),
    /// Both.
    Both {
        /// BT-120.
        text: String,
        /// BT-121.
        code: String,
    },
}

impl Exemption {
    /// BT-120, if stated.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(t) | Self::Both { text: t, .. } => Some(t),
            Self::Code(_) => None,
        }
    }

    /// BT-121, if stated.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        match self {
            Self::Code(c) | Self::Both { code: c, .. } => Some(c),
            Self::Text(_) => None,
        }
    }

    fn is_blank(&self) -> bool {
        self.text().is_none_or(|t| t.trim().is_empty())
            && self.code().is_none_or(|c| c.trim().is_empty())
    }
}

/// Why a VAT treatment is invalid; the payload of [`ErrorKind::InvalidVat`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VatError {
    /// The rate is not allowed for the category (BR-S-05: > 0; BR-Z-05 and
    /// siblings: = 0; BR-AF-05/BR-AG-05: ≥ 0).
    Rate {
        /// The category.
        category: TaxCategory,
        /// The refused rate.
        rate_percent: Percent,
    },
    /// The category requires an exemption reason (BR-E-10 and siblings).
    ExemptionRequired {
        /// The category.
        category: TaxCategory,
    },
    /// The category forbids an exemption reason (BR-S-10 and siblings).
    ExemptionForbidden {
        /// The category.
        category: TaxCategory,
    },
    /// This constructor does not build this category (e.g.
    /// `Vat::exempt(Standard, …)`).
    Constructor {
        /// The category.
        category: TaxCategory,
    },
    /// `O` beside another VAT group (BR-O-11).
    OutOfScopeMixed,
    /// `B` beside `S` (BR-B-02).
    SplitPaymentWithStandard,
}

impl VatError {
    pub(crate) fn rule(&self) -> Option<&'static str> {
        use TaxCategory as C;
        match self {
            Self::Rate { category, .. } => match category {
                C::Standard => Some("BR-S-05"),
                C::ZeroRated => Some("BR-Z-05"),
                C::Exempt => Some("BR-E-05"),
                C::ReverseCharge => Some("BR-AE-05"),
                C::IntraCommunity => Some("BR-IC-05"),
                C::Export => Some("BR-G-05"),
                C::OutOfScope => Some("BR-O-05"),
                C::CanaryIslands => Some("BR-AF-05"),
                C::CeutaMelilla => Some("BR-AG-05"),
                C::SplitPayment => None,
            },
            Self::ExemptionRequired { category } => match category {
                C::Exempt => Some("BR-E-10"),
                C::ReverseCharge => Some("BR-AE-10"),
                C::IntraCommunity => Some("BR-IC-10"),
                C::Export => Some("BR-G-10"),
                C::OutOfScope => Some("BR-O-10"),
                _ => None,
            },
            Self::ExemptionForbidden { category } => match category {
                C::Standard => Some("BR-S-10"),
                C::ZeroRated => Some("BR-Z-10"),
                C::CanaryIslands => Some("BR-AF-10"),
                C::CeutaMelilla => Some("BR-AG-10"),
                _ => None,
            },
            Self::Constructor { .. } => None,
            Self::OutOfScopeMixed => Some("BR-O-11"),
            Self::SplitPaymentWithStandard => Some("BR-B-02"),
        }
    }
}

impl fmt::Display for VatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rate {
                category,
                rate_percent,
            } => write!(
                f,
                "rate {rate_percent} is not allowed for category {category}"
            ),
            Self::ExemptionRequired { category } => {
                write!(f, "category {category} requires an exemption reason")
            }
            Self::ExemptionForbidden { category } => {
                write!(f, "category {category} must not state an exemption reason")
            }
            Self::Constructor { category } => {
                write!(f, "category {category} is not built by this constructor")
            }
            Self::OutOfScopeMixed => {
                f.write_str("category O must be the only vat group of a document (BR-O-11)")
            }
            Self::SplitPaymentWithStandard => {
                f.write_str("category B must not appear beside category S (BR-B-02)")
            }
        }
    }
}

fn vat_err(e: VatError) -> Error {
    ErrorKind::InvalidVat(e).into()
}

/// The VAT treatment of a line, allowance or charge: category (BT-151), rate
/// as a [`Percent`] (BT-152) and exemption reason (BT-120/121).
///
/// Built only through validating constructors, so every `Vat` satisfies its
/// category's rate and reason rules. The rate is stored normalised (`19.00` →
/// `19`).
///
/// ```
/// use billing::{Exemption, Percent, TaxCategory, Vat};
/// use rust_decimal::dec;
///
/// let s = Vat::standard(Percent::new(dec!(19)))?;
/// assert_eq!(s.rate_percent(), Percent::new(dec!(19)));
/// assert!(Vat::standard(Percent::ZERO).is_err()); // BR-S-05
///
/// let ae = Vat::exempt(TaxCategory::ReverseCharge, Exemption::Code("VATEX-EU-AE".into()))?;
/// assert_eq!(ae.rate_percent(), Percent::ZERO);
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "VatRepr", into = "VatRepr"))]
pub struct Vat {
    category: TaxCategory,
    rate_percent: Decimal,
    exemption: Option<Exemption>,
}

impl Vat {
    /// The single gate: every `Vat` (constructors and serde) passes here.
    fn checked(
        category: TaxCategory,
        rate_percent: Decimal,
        exemption: Option<Exemption>,
    ) -> Result<Self> {
        let rate_percent = rate_percent.normalize();
        let rate_ok = match category {
            TaxCategory::Standard => rate_percent > Decimal::ZERO,
            TaxCategory::CanaryIslands | TaxCategory::CeutaMelilla | TaxCategory::SplitPayment => {
                !rate_percent.is_sign_negative() || rate_percent.is_zero()
            }
            _ => rate_percent.is_zero(),
        };
        if !rate_ok {
            return Err(vat_err(VatError::Rate {
                category,
                rate_percent: Percent::new(rate_percent),
            }));
        }
        // A negative zero would serialise as "-0"; there is one zero.
        let rate_percent = if rate_percent.is_zero() {
            Decimal::ZERO
        } else {
            rate_percent
        };
        let blank = exemption.as_ref().is_none_or(Exemption::is_blank);
        if category.needs_exemption() && blank {
            return Err(vat_err(VatError::ExemptionRequired { category }));
        }
        if category.forbids_exemption() && exemption.is_some() {
            return Err(vat_err(VatError::ExemptionForbidden { category }));
        }
        Ok(Self {
            category,
            rate_percent,
            exemption,
        })
    }

    /// `S` at `rate` (BT-152). The rate is a [`Percent`], so its unit is never
    /// guessed: Corsica's 0.9 % is as expressible as 19 %.
    ///
    /// ```
    /// use billing::{Percent, Vat};
    /// use rust_decimal::dec;
    ///
    /// assert_eq!(Vat::standard(Percent::new(dec!(19)))?.rate_percent(), Percent::new(dec!(19)));
    /// assert_eq!(Vat::standard(Percent::new(dec!(0.9)))?.rate_percent().to_string(), "0.9 %");
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// A bare number is not a percentage and does not compile:
    ///
    /// ```compile_fail
    /// use billing::Vat;
    /// use rust_decimal::dec;
    ///
    /// let _ = Vat::standard(dec!(19));
    /// ```
    ///
    /// # Errors
    /// [`ErrorKind::InvalidVat`] unless the rate is above zero (BR-S-05).
    pub fn standard(rate: Percent) -> Result<Self> {
        Self::checked(TaxCategory::Standard, rate.value(), None)
    }

    /// `Z`: zero rated, no exemption reason (BR-Z-10).
    #[must_use]
    pub fn zero_rated() -> Self {
        Self {
            category: TaxCategory::ZeroRated,
            rate_percent: Decimal::ZERO,
            exemption: None,
        }
    }

    /// `E`, `AE`, `K`, `G` or `O` at rate 0 with the reason `why`. For `O` the
    /// rate is not stated on the line ([`TaxCategory::states_rate`]).
    ///
    /// # Errors
    /// [`ErrorKind::InvalidVat`] for another category or a blank reason.
    pub fn exempt(category: TaxCategory, why: Exemption) -> Result<Self> {
        if !category.needs_exemption() {
            return Err(vat_err(VatError::Constructor { category }));
        }
        Self::checked(category, Decimal::ZERO, Some(why))
    }

    /// `L`, `M` or `B` at `rate` (≥ 0).
    ///
    /// # Errors
    /// [`ErrorKind::InvalidVat`] for another category or a negative rate.
    pub fn regional(category: TaxCategory, rate: Percent) -> Result<Self> {
        if !matches!(
            category,
            TaxCategory::CanaryIslands | TaxCategory::CeutaMelilla | TaxCategory::SplitPayment
        ) {
            return Err(vat_err(VatError::Constructor { category }));
        }
        Self::checked(category, rate.value(), None)
    }

    /// The category (BT-151).
    #[must_use]
    pub fn category(&self) -> TaxCategory {
        self.category
    }

    /// The rate (BT-152); 0 % for the zero-tax categories and `O`.
    #[must_use]
    pub fn rate_percent(&self) -> Percent {
        Percent::new(self.rate_percent)
    }

    /// The rate as a line states it: `None` for `O` (BR-O-05).
    #[must_use]
    pub fn stated_rate(&self) -> Option<Percent> {
        self.category
            .states_rate()
            .then_some(Percent::new(self.rate_percent))
    }

    /// The exemption reason, if any.
    #[must_use]
    pub fn exemption(&self) -> Option<&Exemption> {
        self.exemption.as_ref()
    }

    /// The BG-23 grouping key: category and rate.
    pub(crate) fn key(&self) -> (TaxCategory, Decimal) {
        (self.category, self.rate_percent)
    }

    /// `round(base × rate / 100)` at `scale`, once, exactly.
    pub(crate) fn tax_on(&self, base: Amount, scale: AmountScale) -> Result<Amount> {
        reduce_exact(
            &[base.to_decimal(), self.rate_percent],
            &[Decimal::ONE_HUNDRED],
            scale,
        )
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Serialize, serde::Deserialize)]
struct VatRepr {
    category: TaxCategory,
    #[serde(with = "crate::decimal::serde_str")]
    rate_percent: Decimal,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    exemption: Option<Exemption>,
}

#[cfg(feature = "serde")]
impl TryFrom<VatRepr> for Vat {
    type Error = Error;
    fn try_from(r: VatRepr) -> Result<Self> {
        Self::checked(r.category, r.rate_percent, r.exemption)
    }
}

#[cfg(feature = "serde")]
impl From<Vat> for VatRepr {
    fn from(v: Vat) -> Self {
        Self {
            category: v.category,
            rate_percent: v.rate_percent,
            exemption: v.exemption,
        }
    }
}

/// One row of the VAT breakdown (BG-23): the group's [`Vat`], taxable amount
/// (BT-116) and tax (BT-117). Derived by [`Document`](crate::Document) (or
/// [`EarlyPaymentDiscount::vat_correction`](crate::EarlyPaymentDiscount::vat_correction));
/// read-only.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VatGroup {
    vat: Vat,
    taxable_base: Amount,
    tax: Amount,
}

impl VatGroup {
    pub(crate) fn new(vat: Vat, taxable_base: Amount, tax: Amount) -> Self {
        Self {
            vat,
            taxable_base,
            tax,
        }
    }

    /// The group's VAT treatment.
    #[must_use]
    pub fn vat(&self) -> &Vat {
        &self.vat
    }

    /// BT-118.
    #[must_use]
    pub fn category(&self) -> TaxCategory {
        self.vat.category
    }

    /// BT-119 (stated for every category, `O` included: it is 0 %).
    #[must_use]
    pub fn rate_percent(&self) -> Percent {
        Percent::new(self.vat.rate_percent)
    }

    /// BT-120/121.
    #[must_use]
    pub fn exemption(&self) -> Option<&Exemption> {
        self.vat.exemption.as_ref()
    }

    /// BT-116.
    #[must_use]
    pub fn taxable_base(&self) -> Amount {
        self.taxable_base
    }

    /// BT-117.
    #[must_use]
    pub fn tax(&self) -> Amount {
        self.tax
    }

    pub(crate) fn negated(&self) -> Self {
        Self::new(self.vat.clone(), -self.taxable_base, -self.tax)
    }
}

/// Split a tax-inclusive `gross` into `(net, tax)` at `scale`:
/// `tax = round(gross × r / (100 + r))` once, exactly, and `net = gross − tax`, so
/// the two re-add to `gross` exactly.
///
/// ```
/// use billing::{AmountScale, Percent, Vat, split_gross};
/// use rust_decimal::dec;
///
/// let (net, tax) = split_gross(dec!(9.99), &Vat::standard(Percent::new(dec!(19)))?, AmountScale::EN16931)?;
/// assert_eq!((net.to_string().as_str(), tax.to_string().as_str()), ("8.39000", "1.60000"));
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Errors
/// [`ErrorKind::ExcessPrecision`] if `gross` does not fit `scale`;
/// [`ErrorKind::Overflow`].
pub fn split_gross(gross: Decimal, vat: &Vat, scale: AmountScale) -> Result<(Amount, Amount)> {
    let g = Amount::from_decimal(gross)?;
    if !g.fits(scale.decimals) {
        return Err(ErrorKind::ExcessPrecision {
            value: gross,
            decimals: scale.decimals,
        }
        .into());
    }
    let r = vat.rate_percent;
    let tax = reduce_exact(
        &[gross, r],
        &[Decimal::ONE_HUNDRED
            .checked_add(r)
            .ok_or(ErrorKind::Overflow)?],
        scale,
    )?;
    Ok((g.checked_sub(tax)?, tax))
}

/// `round(Π num / Π den)` at `scale`, exactly: integer arithmetic on the
/// mantissas, one rounding. [`ErrorKind::Overflow`] past `i128`,
/// [`ErrorKind::DivisionByZero`] for a zero denominator.
pub(crate) fn reduce_exact(num: &[Decimal], den: &[Decimal], scale: AmountScale) -> Result<Amount> {
    units_to_amount(units_exact(num, den, scale)?, scale)
}

/// [`reduce_exact`] as a count of `10^-decimals` units.
pub(crate) fn units_exact(num: &[Decimal], den: &[Decimal], scale: AmountScale) -> Result<i128> {
    let overflow = || Error::from(ErrorKind::Overflow);
    let product = |ds: &[Decimal]| -> Result<(i128, u32)> {
        let mut m: i128 = 1;
        let mut s: u32 = 0;
        for d in ds {
            let d = d.normalize();
            m = m.checked_mul(d.mantissa()).ok_or_else(overflow)?;
            s += d.scale();
        }
        Ok((m, s))
    };
    let (n, ns) = product(num)?;
    let (q, qs) = product(den)?;
    units_ratio((n, ns), (q, qs), scale)
}

/// `round((n / 10^ns) / (q / 10^qs))` in units of `10^-decimals`, exactly:
/// one integer division, one rounding.
pub(crate) fn units_ratio(n: (i128, u32), q: (i128, u32), scale: AmountScale) -> Result<i128> {
    let decimals = scale.check()?;
    crate::decimal::ratio_units(n, q, u32::from(decimals), scale.rounding)
}

/// Units at `scale` as an `Amount`.
pub(crate) fn units_to_amount(units: i128, scale: AmountScale) -> Result<Amount> {
    let d = Decimal::try_from_i128_with_scale(units, u32::from(scale.decimals))
        .map_err(|_| ErrorKind::Overflow)?;
    Amount::from_decimal(d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Rounding;
    use alloc::string::ToString;
    use rust_decimal::dec;

    /// Every row of the `VatError` rule table, the category predicates, the
    /// category code as `Display`, and every `VatError` message shape.
    #[test]
    fn every_vat_error_maps_to_its_rule() {
        use TaxCategory as C;
        let required = [
            (C::Exempt, Some("BR-E-10")),
            (C::ReverseCharge, Some("BR-AE-10")),
            (C::IntraCommunity, Some("BR-IC-10")),
            (C::Export, Some("BR-G-10")),
            (C::OutOfScope, Some("BR-O-10")),
            (C::Standard, None),
        ];
        for (category, rule) in required {
            assert_eq!(
                VatError::ExemptionRequired { category }.rule(),
                rule,
                "{category:?}"
            );
        }
        let forbidden = [
            (C::Standard, Some("BR-S-10")),
            (C::ZeroRated, Some("BR-Z-10")),
            (C::CanaryIslands, Some("BR-AF-10")),
            (C::CeutaMelilla, Some("BR-AG-10")),
            (C::Exempt, None),
        ];
        for (category, rule) in forbidden {
            assert_eq!(
                VatError::ExemptionForbidden { category }.rule(),
                rule,
                "{category:?}"
            );
        }
        let rate = [
            (C::Standard, Some("BR-S-05")),
            (C::ZeroRated, Some("BR-Z-05")),
            (C::Exempt, Some("BR-E-05")),
            (C::ReverseCharge, Some("BR-AE-05")),
            (C::IntraCommunity, Some("BR-IC-05")),
            (C::Export, Some("BR-G-05")),
            (C::OutOfScope, Some("BR-O-05")),
            (C::CanaryIslands, Some("BR-AF-05")),
            (C::CeutaMelilla, Some("BR-AG-05")),
            (C::SplitPayment, None),
        ];
        for (category, rule) in rate {
            let e = VatError::Rate {
                category,
                rate_percent: Percent::new(dec!(1)),
            };
            assert_eq!(e.rule(), rule, "{category:?}");
            let shown = e.to_string();
            assert!(
                shown.starts_with(|c: char| c.is_ascii_lowercase()),
                "{shown}"
            );
        }
        for e in [
            VatError::ExemptionRequired {
                category: C::Exempt,
            },
            VatError::ExemptionForbidden {
                category: C::Standard,
            },
            VatError::OutOfScopeMixed,
            VatError::SplitPaymentWithStandard,
        ] {
            let shown = e.to_string();
            assert!(
                shown.starts_with(|c: char| c.is_ascii_lowercase()),
                "{shown}"
            );
        }
        for c in TaxCategory::ALL {
            assert_eq!(c.to_string(), c.code());
            assert_eq!(c.states_rate(), c != C::OutOfScope, "{c:?}");
            let taxed = matches!(
                c,
                C::Standard | C::CanaryIslands | C::CeutaMelilla | C::SplitPayment
            );
            assert_eq!(c.carries_tax(), taxed, "{c:?}");
        }
        let s = Vat::standard(Percent::new(dec!(19))).unwrap();
        assert_eq!(s.stated_rate(), Some(Percent::new(dec!(19))));
        let why = Exemption::Text("§4".into());
        let e = Vat::exempt(C::Exempt, why.clone()).unwrap();
        let g = VatGroup::new(e, Amount::ZERO, Amount::ZERO);
        assert_eq!(g.exemption(), Some(&why));
        assert_eq!(
            VatGroup::new(s, Amount::ZERO, Amount::ZERO).exemption(),
            None
        );
    }

    #[test]
    fn codes_round_trip_and_rules_map() {
        for c in TaxCategory::ALL {
            assert_eq!(TaxCategory::from_code(c.code()), Some(c));
        }
        assert_eq!(TaxCategory::from_code("Q"), None);
        let e = Vat::standard(Percent::ZERO).unwrap_err();
        assert_eq!(e.rule(), Some("BR-S-05"));
        let e = Vat::exempt(TaxCategory::Exempt, Exemption::Text(" ".into())).unwrap_err();
        assert_eq!(e.rule(), Some("BR-E-10"));
        let e = Vat::exempt(TaxCategory::Standard, Exemption::Text("x".into())).unwrap_err();
        assert_eq!(e.rule(), None);
        assert!(Vat::regional(TaxCategory::CanaryIslands, Percent::ZERO).is_ok());
        assert!(Vat::regional(TaxCategory::CeutaMelilla, Percent::new(dec!(-1))).is_err());
        assert!(Vat::regional(TaxCategory::Standard, Percent::new(dec!(1))).is_err());
        let o = Vat::exempt(
            TaxCategory::OutOfScope,
            Exemption::Code("VATEX-EU-O".into()),
        )
        .unwrap();
        assert_eq!(o.stated_rate(), None);
        assert_eq!(
            Vat::standard(Percent::new(dec!(19.00)))
                .unwrap()
                .rate_percent()
                .to_string(),
            "19 %"
        );
        assert!(
            Vat::checked(
                TaxCategory::Standard,
                dec!(19),
                Some(Exemption::Text("x".into()))
            )
            .is_err()
        );
    }

    #[test]
    fn reduce_exact_rounds_once() {
        let s = AmountScale::EN16931;
        // 1/3 → 0.33; 2/3 → 0.67; −2/3 → −0.67
        assert_eq!(
            reduce_exact(&[dec!(1)], &[dec!(3)], s).unwrap().to_string(),
            "0.33000"
        );
        assert_eq!(
            reduce_exact(&[dec!(-2)], &[dec!(3)], s)
                .unwrap()
                .to_string(),
            "-0.67000"
        );
        assert_eq!(
            reduce_exact(&[dec!(2)], &[dec!(-3)], s)
                .unwrap()
                .to_string(),
            "-0.67000"
        );
        let t = AmountScale::new(2, Rounding::TowardZero).unwrap();
        assert_eq!(
            reduce_exact(&[dec!(0.004999)], &[], t).unwrap(),
            Amount::ZERO
        );
        assert_eq!(
            reduce_exact(&[dec!(1)], &[dec!(0)], s).unwrap_err().kind(),
            &ErrorKind::DivisionByZero
        );
    }

    #[test]
    fn split_gross_reconstructs() {
        let v = Vat::standard(Percent::new(dec!(19))).unwrap();
        let (n, t) = split_gross(dec!(119), &v, AmountScale::EN16931).unwrap();
        assert_eq!((n.to_decimal(), t.to_decimal()), (dec!(100), dec!(19)));
        assert!(split_gross(dec!(1.001), &v, AmountScale::EN16931).is_err());
        let (n, t) = split_gross(dec!(5), &Vat::zero_rated(), AmountScale::EN16931).unwrap();
        assert_eq!((n.to_decimal(), t), (dec!(5), Amount::ZERO));
    }
}
