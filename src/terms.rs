//! Payment-terms arithmetic: [`EarlyPaymentDiscount`] (Skonto) and
//! [`late_payment_interest`]. Rendering terms as text is the format's job.

use alloc::vec::Vec;

use rust_decimal::Decimal;

use crate::vat::{reduce_exact, units_ratio, units_to_amount};
use crate::{
    Amount, AmountScale, Document, ErrorKind, Percent, Result, VatGroup, split, split_gross,
};

/// A discount for early payment, in percent of the amount due (BT-115).
///
/// ```
/// use billing::{AmountScale, Currency, Document, EarlyPaymentDiscount, Line, Percent, Vat};
/// use rust_decimal::dec;
///
/// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
///     .line(Line::fixed("a", dec!(1000)))
///     .build()?;
/// let skonto = EarlyPaymentDiscount { percent: Percent::new(dec!(2)) };
/// assert_eq!(skonto.amount(&doc)?.to_string(), "23.80000");
/// let correction = skonto.vat_correction(&doc)?;
/// assert_eq!(correction[0].taxable_base().to_string(), "20.00000");
/// assert_eq!(correction[0].tax().to_string(), "3.80000");
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct EarlyPaymentDiscount {
    /// Percent of the amount due, `0..=100`.
    pub percent: Percent,
}

impl EarlyPaymentDiscount {
    /// `round(BT-115 × percent / 100)` once, at the document's scale.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidRate`] if the percent is outside `0..=100`;
    /// [`ErrorKind::Overflow`].
    pub fn amount(&self, doc: &Document) -> Result<Amount> {
        let percent = self.percent.value();
        if percent < Decimal::ZERO || percent > Decimal::ONE_HUNDRED {
            return Err(ErrorKind::InvalidRate { rate: percent }.into());
        }
        reduce_exact(
            &[doc.payable().to_decimal(), percent],
            &[Decimal::ONE_HUNDRED],
            doc.scale(),
        )
    }

    /// The per-group reduction of base and tax the discount implies (§17 UStG,
    /// art. 90 VAT Directive): the discount is split across the VAT groups by
    /// their gross (largest remainder; by |gross| when every group is a
    /// credit, so the correction of a negated document is the negated
    /// correction), and each group's share is split into net and tax with
    /// [`split_gross`]. Σ (base + tax) over the result equals
    /// [`amount`](Self::amount) exactly.
    ///
    /// A VAT-free document has no groups and gets an empty correction. If every
    /// group's gross is zero, a zero discount gives zero rows and a non-zero
    /// one (possible only through payments) is refused.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidRate`] if the percent is outside `0..=100` (checked
    /// even for a VAT-free document);
    /// [`ErrorKind::InvalidWeights`] if the group grosses have mixed signs (or
    /// are all zero under a non-zero discount); [`ErrorKind::Overflow`].
    pub fn vat_correction(&self, doc: &Document) -> Result<Vec<VatGroup>> {
        let discount = self.amount(doc)?;
        let groups = doc.vat_groups();
        if groups.is_empty() {
            return Ok(Vec::new());
        }
        let gross: Vec<Decimal> = groups
            .iter()
            .map(|g| Ok(g.taxable_base().checked_add(g.tax())?.to_decimal()))
            .collect::<Result<_>>()?;
        let weights: Vec<Decimal> = if gross.iter().all(|g| *g >= Decimal::ZERO) {
            gross
        } else if gross.iter().all(|g| *g <= Decimal::ZERO) {
            gross.iter().map(|g| -*g).collect()
        } else {
            return Err(ErrorKind::InvalidWeights.into());
        };
        let scale = doc.scale();
        let shares = split(discount.to_decimal(), &weights, scale.decimals)?;
        groups
            .iter()
            .zip(shares)
            .map(|(g, share)| {
                let (net, tax) = split_gross(share, g.vat(), scale)?;
                Ok(VatGroup::new(g.vat().clone(), net, tax))
            })
            .collect()
    }
}

/// A stretch of days at one annual rate (statutory rates change every half
/// year, §247 BGB), with its own day-count denominator.
///
/// `annual_percent` is not range-checked: a negative rate (the German base
/// rate was negative 2013–2022) yields negative interest. A zero
/// `year_days` is refused by [`late_payment_interest`] with
/// [`ErrorKind::DivisionByZero`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InterestSegment {
    /// Annual rate, e.g. 9.27 %.
    pub annual_percent: Percent,
    /// Days in this segment.
    pub days: u32,
    /// The year's day count (365, 360, 366 …).
    pub year_days: u32,
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Simple interest on `principal` over `segments`:
/// `Σ principal × annual_percent / 100 × days / year_days`, summed **exactly**
/// (over the common denominator of the year lengths) and rounded once at `scale`.
///
/// ```
/// use billing::{Amount, AmountScale, InterestSegment, Percent, late_payment_interest};
/// use rust_decimal::dec;
///
/// let p = Amount::parse("1000")?;
/// let i = late_payment_interest(p, &[
///     InterestSegment { annual_percent: Percent::new(dec!(9.27)), days: 10, year_days: 365 },
///     InterestSegment { annual_percent: Percent::new(dec!(9.12)), days: 20, year_days: 365 },
/// ], AmountScale::EN16931)?;
/// assert_eq!(i.to_string(), "7.54000"); // 2.539726… + 4.997260… = 7.536986… → 7.54
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Errors
/// [`ErrorKind::DivisionByZero`] for a zero `year_days`; [`ErrorKind::Overflow`].
pub fn late_payment_interest(
    principal: Amount,
    segments: &[InterestSegment],
    scale: AmountScale,
) -> Result<Amount> {
    let mut lcm: u128 = 1;
    for s in segments {
        if s.year_days == 0 {
            return Err(ErrorKind::DivisionByZero.into());
        }
        let y = u128::from(s.year_days);
        lcm = (lcm / gcd(lcm, y))
            .checked_mul(y)
            .ok_or(ErrorKind::Overflow)?;
    }
    // Σ annual_percent × days × (lcm / year_days) on mantissas at a common
    // scale, in i128: no Decimal product can lose digits.
    let overflow = || ErrorKind::Overflow;
    let terms: Vec<(i128, u32)> = segments
        .iter()
        .map(|s| {
            let a = s.annual_percent.value().normalize();
            let f = i128::try_from(lcm / u128::from(s.year_days)).map_err(|_| overflow())?;
            let m = a
                .mantissa()
                .checked_mul(i128::from(s.days))
                .and_then(|m| m.checked_mul(f))
                .ok_or_else(overflow)?;
            Ok((m, a.scale()))
        })
        .collect::<Result<_>>()?;
    let common = terms.iter().map(|t| t.1).max().unwrap_or(0);
    let mut sum: i128 = 0;
    for (m, sc) in terms {
        let f = crate::decimal::pow10(common - sc).ok_or_else(overflow)?;
        sum = m
            .checked_mul(f)
            .and_then(|m| sum.checked_add(m))
            .ok_or_else(overflow)?;
    }
    let p = principal.to_decimal().normalize();
    let n = p.mantissa().checked_mul(sum).ok_or_else(overflow)?;
    let q = 100i128
        .checked_mul(i128::try_from(lcm).map_err(|_| overflow())?)
        .ok_or_else(overflow)?;
    units_to_amount(units_ratio((n, p.scale() + common), (q, 0), scale)?, scale)
}
