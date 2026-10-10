//! [`Amount`], [`Rounding`] and [`AmountScale`]: the money core.

use alloc::vec::Vec;
use core::fmt;
use core::iter::Sum;
use core::ops::{Add, AddAssign, Neg, Sub, SubAssign};

use rust_decimal::Decimal;

use crate::decimal::{self, pow10};
use crate::{ErrorKind, Result};

/// How a value is reduced to fewer decimals.
///
/// Every mode is **sign-symmetric**: `round(−x) == −round(x)`, so a credit
/// rounds exactly like the mirrored debit whichever path builds it. Directed
/// floor/ceiling are deliberately not offered.
///
/// ```
/// use billing::{Amount, Rounding};
/// use rust_decimal::dec;
///
/// let r = |x, m| Amount::<2>::from_decimal_rounded(x, m).unwrap().to_string();
/// assert_eq!(r(dec!(2.345), Rounding::HalfAwayFromZero), "2.35");
/// assert_eq!(r(dec!(2.345), Rounding::HalfEven), "2.34");
/// assert_eq!(r(dec!(-2.341), Rounding::AwayFromZero), "-2.35");
/// assert_eq!(r(dec!(-2.349), Rounding::TowardZero), "-2.34");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Rounding {
    /// Ties go away from zero (commercial rounding; EN 16931's choice).
    HalfAwayFromZero,
    /// Ties go to the even neighbour (banker's rounding).
    HalfEven,
    /// Any discarded non-zero digit moves the result away from zero.
    AwayFromZero,
    /// Discarded digits are dropped (truncation).
    TowardZero,
}

impl Rounding {
    /// `n / d` rounded by this mode. `d > 0`.
    pub(crate) fn div(self, n: i128, d: i128) -> i128 {
        debug_assert!(d > 0);
        let d = d.unsigned_abs();
        let mag = n.unsigned_abs();
        let (q, r) = (mag / d, mag % d);
        let up = match self {
            Self::TowardZero => false,
            Self::AwayFromZero => r != 0,
            Self::HalfAwayFromZero => r != 0 && r >= d - r,
            Self::HalfEven => r > d - r || (r != 0 && r == d - r && q % 2 == 1),
        };
        let q = q + u128::from(up);
        // q ≤ |n| (an increment needs d ≥ 2), so it fits; −2^127 wraps to itself.
        #[allow(clippy::cast_possible_wrap)]
        let q = q as i128;
        if n < 0 { q.wrapping_neg() } else { q }
    }
}

/// The interchange scale of a document: how many decimals emitted amounts carry
/// and how they are rounded to them. A property of the *format*, not of the
/// currency: EN 16931 emits two decimals for EUR, JPY and KWD alike.
///
/// `decimals` is at most 5 (the engine's [`Amount`] precision).
/// [`new`](Self::new) refuses more, and so does every consumer of a scale built
/// from the public fields ([`ErrorKind::ScaleMismatch`]).
///
/// ```
/// use billing::{AmountScale, ErrorKind, Rounding};
///
/// let yen = AmountScale::new(0, Rounding::HalfAwayFromZero)?;
/// assert_eq!(yen.decimals, 0);
/// let err = AmountScale::new(6, Rounding::HalfEven).unwrap_err();
/// assert_eq!(err.kind(), &ErrorKind::ScaleMismatch);
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawScale"))]
pub struct AmountScale {
    /// Decimals of every emitted amount, `0..=5`.
    pub decimals: u8,
    /// How exact values are reduced to `decimals`.
    pub rounding: Rounding,
}

/// The most decimals an [`AmountScale`] may carry: the engine's precision.
pub(crate) const MAX_SCALE_DECIMALS: u8 = 5;

impl AmountScale {
    /// EN 16931 (BR-DEC): two decimals, half away from zero.
    pub const EN16931: Self = Self {
        decimals: 2,
        rounding: Rounding::HalfAwayFromZero,
    };

    /// A scale of `decimals` (≤ 5) rounded by `rounding`.
    ///
    /// # Errors
    /// [`ErrorKind::ScaleMismatch`] if `decimals > 5`.
    pub fn new(decimals: u8, rounding: Rounding) -> Result<Self> {
        let scale = Self { decimals, rounding };
        scale.check()?;
        Ok(scale)
    }

    /// `decimals` of this scale, refused past 5 (the fields are public, so
    /// every consumer re-checks).
    pub(crate) fn check(self) -> Result<u8> {
        if self.decimals > MAX_SCALE_DECIMALS {
            return Err(ErrorKind::ScaleMismatch.into());
        }
        Ok(self.decimals)
    }

    /// The same number of decimals, rounded by `rounding` instead.
    pub(crate) const fn with_rounding(self, rounding: Rounding) -> Self {
        Self {
            decimals: self.decimals,
            rounding,
        }
    }

    /// Reduce an exact value to this scale, **once**: the result is `exact`
    /// rounded straight to `decimals`, never via an intermediate precision.
    ///
    /// ```
    /// use billing::{Amount, AmountScale};
    /// use rust_decimal::dec;
    ///
    /// // 10.02629 × 0.19 = 1.9049951 → 1.90 (rounding to 5 dp first would give 1.91).
    /// let tax: Amount = AmountScale::EN16931.reduce(dec!(10.02629) * dec!(0.19))?;
    /// assert_eq!(tax.to_string(), "1.90000");
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`ErrorKind::ScaleMismatch`] if `decimals` exceeds `P` or 5;
    /// [`ErrorKind::Overflow`] if the result does not fit `Amount<P>`.
    pub fn reduce<const P: u8>(self, exact: Decimal) -> Result<Amount<P>> {
        let decimals = self.check()?;
        if decimals > P {
            return Err(ErrorKind::ScaleMismatch.into());
        }
        let units = to_units(exact, decimals, Some(self.rounding))?;
        let factor = pow10(u32::from(P - decimals)).ok_or(ErrorKind::Overflow)?;
        Amount::<P>::from_units(units.checked_mul(factor).ok_or(ErrorKind::Overflow)?)
    }
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawScale {
    decimals: u8,
    rounding: Rounding,
}

#[cfg(feature = "serde")]
impl TryFrom<RawScale> for AmountScale {
    type Error = crate::Error;
    fn try_from(raw: RawScale) -> Result<Self> {
        Self::new(raw.decimals, raw.rounding)
    }
}

/// `d` as an integer number of `10^-decimals` units. `rounding: None` means
/// exact (refuse with `ExcessPrecision`).
pub(crate) fn to_units(d: Decimal, decimals: u8, rounding: Option<Rounding>) -> Result<i128> {
    let (m, s) = (d.mantissa(), d.scale());
    let p = u32::from(decimals);
    if s <= p {
        let f = pow10(p - s).ok_or(ErrorKind::Overflow)?;
        return Ok(m.checked_mul(f).ok_or(ErrorKind::Overflow)?);
    }
    // s ≤ 28, so 10^(s−p) fits i128.
    let f = pow10(s - p).ok_or(ErrorKind::Overflow)?;
    match rounding {
        Some(r) => Ok(r.div(m, f)),
        None if m % f == 0 => Ok(m / f),
        None => Err(ErrorKind::ExcessPrecision { value: d, decimals }.into()),
    }
}

/// A fixed-point money amount: an `i64` count of `10^-P` units.
///
/// `P` defaults to 5, the engine's precision; consumers use other precisions
/// (`Amount<2>` for emitted cents, `Amount<6>` …), `P ≤ 18`. At `P = 5` the range is
/// ±92 233 720 368 547.75807, which a single invoice in a high-nominal currency
/// (IRR, VND, LBP) can exceed: every operation then fails with
/// [`ErrorKind::Overflow`] — never silently.
///
/// There is no multiplication or division on `Amount`: both need a rounding, and
/// a hidden one is how "rounded once" becomes "rounded twice". Multiply through
/// `Decimal` and round explicitly, once:
///
/// ```
/// use billing::{Amount, Rounding};
/// use rust_decimal::dec;
///
/// let price = Amount::<5>::parse("10.02629")?;
/// let tax = Amount::<2>::from_decimal_rounded(price.to_decimal() * dec!(0.19), Rounding::HalfAwayFromZero)?;
/// assert_eq!(tax.to_string(), "1.90");
///
/// // The grammar is `.` only: a thousands separator is refused, not misread.
/// assert!(Amount::<2>::parse("1,000").is_err());
/// # Ok::<(), billing::Error>(())
/// ```
///
/// Neither `Amount * Decimal` nor `Amount * Amount` compiles, so no product can
/// round behind the caller's back:
///
/// ```compile_fail
/// use billing::{Amount, Decimal};
/// let _ = Amount::<2>::ZERO * Decimal::ONE;
/// ```
///
/// ```compile_fail
/// use billing::Amount;
/// let _ = Amount::<2>::ZERO * Amount::<2>::ZERO;
/// ```
///
/// The `+`, `-`, unary `-`, `+=`, `-=` and `Sum` operators **panic on overflow**;
/// the `checked_*` methods, rounding, rescaling, parsing and splitting never
/// panic: they return [`ErrorKind::Overflow`] instead.
///
/// `P ≤ 18` is checked at compile time: every way to obtain an `Amount<P>`
/// (the constants, [`from_raw`](Self::from_raw), [`parse`](Self::parse), the
/// conversions, `Default`) evaluates the bound, so no value with `P > 18` can
/// exist.
///
/// ```compile_fail
/// let _ = billing::Amount::<19>::ZERO;
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Amount<const P: u8 = 5>(i64);

impl<const P: u8> Amount<P> {
    /// `10^P`; evaluating it for `P > 18` is a compile error.
    const FACTOR: i64 = {
        assert!(P <= 18, "Amount<P> supports P <= 18");
        10i64.pow(P as u32)
    };

    /// Zero.
    pub const ZERO: Self = Self::from_raw(0);
    /// The smallest amount, `i64::MIN` units.
    pub const MIN: Self = Self::from_raw(i64::MIN);
    /// The largest amount, `i64::MAX` units.
    pub const MAX: Self = Self::from_raw(i64::MAX);

    /// The amount of `raw` units of `10^-P`: `Amount::<2>::from_raw(150)` is 1.50.
    #[must_use]
    pub const fn from_raw(raw: i64) -> Self {
        let _ = Self::FACTOR;
        Self(raw)
    }

    /// The count of `10^-P` units.
    #[must_use]
    pub const fn to_raw(self) -> i64 {
        self.0
    }

    fn from_units(units: i128) -> Result<Self> {
        let _ = Self::FACTOR;
        i64::try_from(units)
            .map(Self)
            .map_err(|_| ErrorKind::Overflow.into())
    }

    /// Parse `[+-]digits[.digits]`: `.` is the only separator, at least one
    /// digit on each side of it (`.5` and `5.` are refused), no exponent, no
    /// spaces, no grouping, no `_`. Acceptance is by **value**: trailing zeros
    /// past `P` are accepted (`"1.500"` is 1.50 at `P = 2`); a non-zero digit
    /// past `P` is refused. `"-0"` is zero.
    ///
    /// # Errors
    /// [`ErrorKind::Malformed`] outside the grammar; [`ErrorKind::ExcessPrecision`]
    /// for a non-zero digit past `P`; [`ErrorKind::Overflow`] out of range.
    /// When a value both has excess digits and is out of range, the excess is
    /// reported (as [`from_decimal`](Self::from_decimal) does), unless the value
    /// is past `Decimal`'s range too, which only `Overflow` can describe.
    pub fn parse(s: &str) -> Result<Self> {
        let lexed = decimal::lex(s)?;
        let frac = lexed.frac.trim_end_matches('0');
        if frac.len() > usize::from(P) {
            let value = match decimal::parse(s) {
                Ok(value) => value,
                Err(e) => match e.kind() {
                    ErrorKind::ExcessPrecision { value, .. } => *value,
                    _ => return Err(e),
                },
            };
            return Err(ErrorKind::ExcessPrecision { value, decimals: P }.into());
        }
        let mut int: i128 = 0;
        for b in lexed.int.bytes() {
            int = int
                .checked_mul(10)
                .and_then(|v| v.checked_add(i128::from(b - b'0')))
                .ok_or(ErrorKind::Overflow)?;
        }
        let mut f: i128 = 0;
        for b in frac.bytes() {
            f = f * 10 + i128::from(b - b'0');
        }
        // frac.len() ≤ P ≤ 18, so these powers fit.
        let len = u32::try_from(frac.len()).map_err(|_| ErrorKind::Overflow)?;
        f *= pow10(u32::from(P) - len).ok_or(ErrorKind::Overflow)?;
        let units = int
            .checked_mul(i128::from(Self::FACTOR))
            .and_then(|v| v.checked_add(f))
            .ok_or(ErrorKind::Overflow)?;
        Self::from_units(if lexed.negative { -units } else { units })
    }

    /// The exact amount of `d`.
    ///
    /// # Errors
    /// [`ErrorKind::ExcessPrecision`] if `d` has a non-zero digit past `P`;
    /// [`ErrorKind::Overflow`] out of range. A value with both faults reports
    /// the excess precision.
    pub fn from_decimal(d: Decimal) -> Result<Self> {
        Self::from_units(to_units(d, P, None)?)
    }

    /// `d` rounded to `P` decimals by `rounding`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] out of range.
    pub fn from_decimal_rounded(d: Decimal, rounding: Rounding) -> Result<Self> {
        Self::from_units(to_units(d, P, Some(rounding))?)
    }

    /// The exact `Decimal` value, with scale `P`.
    #[must_use]
    pub fn to_decimal(self) -> Decimal {
        Decimal::from_i128_with_scale(i128::from(self.0), u32::from(P))
    }

    /// `self + rhs`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`].
    pub fn checked_add(self, rhs: Self) -> Result<Self> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or_else(|| ErrorKind::Overflow.into())
    }

    /// `self - rhs`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`].
    pub fn checked_sub(self, rhs: Self) -> Result<Self> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or_else(|| ErrorKind::Overflow.into())
    }

    /// `-self`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] for [`Amount::MIN`].
    pub fn checked_neg(self) -> Result<Self> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or_else(|| ErrorKind::Overflow.into())
    }

    /// `|self|`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] for [`Amount::MIN`].
    pub fn checked_abs(self) -> Result<Self> {
        self.0
            .checked_abs()
            .map(Self)
            .ok_or_else(|| ErrorKind::Overflow.into())
    }

    /// The sum of `amounts`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] if any partial sum overflows.
    pub fn checked_sum<I: IntoIterator<Item = Self>>(amounts: I) -> Result<Self> {
        amounts
            .into_iter()
            .try_fold(Self::ZERO, |acc, a| acc.checked_add(a))
    }

    /// Round to `decimals` (still an `Amount<P>`). `decimals ≥ P` is the
    /// identity: an `Amount<P>` has no digit past `P` to round.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] when rounding away from zero passes
    /// [`Amount::MAX`] / [`Amount::MIN`].
    pub fn round_to(self, decimals: u8, rounding: Rounding) -> Result<Self> {
        if decimals >= P {
            return Ok(self);
        }
        let inc = pow10(u32::from(P - decimals)).ok_or(ErrorKind::Overflow)?;
        Self::from_units(rounding.div(i128::from(self.0), inc) * inc)
    }

    /// Round to a multiple of `increment` (> 0), e.g. a cash rounding to 0.05.
    ///
    /// ```
    /// use billing::{Amount, Rounding};
    ///
    /// let a = Amount::<2>::parse("10.03")?;
    /// let five = Amount::<2>::parse("0.05")?;
    /// assert_eq!(a.round_to_increment(five, Rounding::HalfAwayFromZero)?.to_string(), "10.05");
    /// assert_eq!((-a).round_to_increment(five, Rounding::HalfAwayFromZero)?.to_string(), "-10.05");
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`ErrorKind::InvalidIncrement`] for an increment ≤ 0;
    /// [`ErrorKind::Overflow`] out of range.
    pub fn round_to_increment(self, increment: Self, rounding: Rounding) -> Result<Self> {
        if increment.0 <= 0 {
            return Err(ErrorKind::InvalidIncrement.into());
        }
        let inc = i128::from(increment.0);
        Self::from_units(rounding.div(i128::from(self.0), inc) * inc)
    }

    /// Whether the value has no non-zero digit past `decimals`.
    #[must_use]
    pub fn fits(self, decimals: u8) -> bool {
        if decimals >= P {
            return true;
        }
        pow10(u32::from(P - decimals)).is_some_and(|f| i128::from(self.0) % f == 0)
    }

    /// The value at precision `Q`, rounded by `rounding` when `Q < P`.
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] if it does not fit `Amount<Q>`.
    pub fn rescale<const Q: u8>(self, rounding: Rounding) -> Result<Amount<Q>> {
        let raw = i128::from(self.0);
        if Q >= P {
            let f = pow10(u32::from(Q - P)).ok_or(ErrorKind::Overflow)?;
            Amount::<Q>::from_units(raw.checked_mul(f).ok_or(ErrorKind::Overflow)?)
        } else {
            let f = pow10(u32::from(P - Q)).ok_or(ErrorKind::Overflow)?;
            Amount::<Q>::from_units(rounding.div(raw, f))
        }
    }

    /// The same value at precision `Q`, exactly.
    ///
    /// # Errors
    /// [`ErrorKind::ExcessPrecision`] if a non-zero digit lies past `Q`;
    /// [`ErrorKind::Overflow`] if it does not fit `Amount<Q>`.
    pub fn exact<const Q: u8>(self) -> Result<Amount<Q>> {
        if !self.fits(Q) {
            return Err(ErrorKind::ExcessPrecision {
                value: self.to_decimal(),
                decimals: Q,
            }
            .into());
        }
        self.rescale(Rounding::TowardZero)
    }

    /// Split into parts proportional to `weights`, at `P` decimals; see
    /// [`split`](crate::split) for the guarantees.
    ///
    /// ```
    /// use billing::Amount;
    /// use rust_decimal::dec;
    ///
    /// let parts = Amount::<2>::parse("100.00")?.split(&[dec!(1), dec!(1), dec!(1)])?;
    /// let shown: Vec<String> = parts.iter().map(|p| p.to_string()).collect();
    /// assert_eq!(shown, ["33.34", "33.33", "33.33"]);
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// As [`split`](crate::split).
    pub fn split(self, weights: &[Decimal]) -> Result<Vec<Self>> {
        crate::split::split_units(i128::from(self.0), weights)?
            .into_iter()
            .map(Self::from_units)
            .collect()
    }

    /// Whether the amount is zero.
    #[must_use]
    pub const fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Whether the amount is above zero.
    #[must_use]
    pub const fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Whether the amount is below zero.
    #[must_use]
    pub const fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// `-1`, `0` or `1`.
    #[must_use]
    pub const fn signum(self) -> i8 {
        if self.0 > 0 {
            1
        } else if self.0 < 0 {
            -1
        } else {
            0
        }
    }
}

impl<const P: u8> From<Amount<P>> for Decimal {
    fn from(a: Amount<P>) -> Self {
        a.to_decimal()
    }
}

impl<const P: u8> core::str::FromStr for Amount<P> {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::parse(s)
    }
}

/// The canonical string: `P` decimals, `-` for negatives, no `+`.
///
/// A formatter **precision** prints that many decimals instead: fewer than `P`
/// round half away from zero (EN 16931's rounding), more pad with zeros. A value
/// that rounds to zero prints without a sign. Width, fill, alignment, `+` and
/// `0` flags apply as for integers. Display rounding is presentation only;
/// amounts that are emitted or summed are reduced with
/// [`round_to`](Amount::round_to) or [`AmountScale`] instead.
///
/// ```
/// use billing::Amount;
///
/// let a = Amount::<5>::parse("-1234.56789")?;
/// assert_eq!(a.to_string(), "-1234.56789");
/// assert_eq!(format!("{a:.2}"), "-1234.57");
/// assert_eq!(format!("{a:.0}"), "-1235");
/// assert_eq!(format!("{a:.7}"), "-1234.5678900");
/// assert_eq!(format!("{:.2}", Amount::<5>::parse("-0.004")?), "0.00");
/// assert_eq!(format!("{:>10.2}", Amount::<5>::parse("2.005")?), "      2.01");
/// # Ok::<(), billing::Error>(())
/// ```
impl<const P: u8> fmt::Display for Amount<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use fmt::Write;
        let p = u32::from(P);
        let shown = f.precision().unwrap_or(usize::from(P));
        // The magnitude at min(shown, P) decimals.
        let mut mag = u128::from(self.0.unsigned_abs());
        let mut kept = p;
        if let Ok(s) = u32::try_from(shown) {
            if s < p {
                // 10^(P − s) ≤ 10^18: no overflow of u128.
                let d = 10u128.pow(p - s);
                mag = (mag + d / 2) / d;
                kept = s;
            }
        }
        let unit = 10u128.pow(kept);
        let mut digits = alloc::string::String::new();
        write!(digits, "{}", mag / unit)?;
        if shown > 0 {
            digits.push('.');
            let width = usize::try_from(kept).unwrap_or(usize::MAX);
            if width > 0 {
                write!(digits, "{:0width$}", mag % unit)?;
            }
            for _ in width..shown {
                digits.push('0');
            }
        }
        f.pad_integral(self.0 >= 0 || mag == 0, "", &digits)
    }
}

impl<const P: u8> Default for Amount<P> {
    fn default() -> Self {
        Self::ZERO
    }
}

impl<const P: u8> fmt::Debug for Amount<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Amount({self})")
    }
}

impl<const P: u8> Add for Amount<P> {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self(
            self.0
                .checked_add(rhs.0)
                .expect("Amount addition overflowed"),
        )
    }
}

impl<const P: u8> Sub for Amount<P> {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self(
            self.0
                .checked_sub(rhs.0)
                .expect("Amount subtraction overflowed"),
        )
    }
}

impl<const P: u8> Neg for Amount<P> {
    type Output = Self;
    fn neg(self) -> Self {
        Self(self.0.checked_neg().expect("Amount negation overflowed"))
    }
}

impl<const P: u8> AddAssign for Amount<P> {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl<const P: u8> SubAssign for Amount<P> {
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl<const P: u8> Sum for Amount<P> {
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, Add::add)
    }
}

impl<'a, const P: u8> Sum<&'a Amount<P>> for Amount<P> {
    fn sum<I: Iterator<Item = &'a Self>>(iter: I) -> Self {
        iter.copied().sum()
    }
}

#[cfg(feature = "serde")]
impl<const P: u8> serde::Serialize for Amount<P> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de, const P: u8> serde::Deserialize<'de> for Amount<P> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::decimal::serde_str::from_str(d, Self::parse, "an amount string such as \"12.50\"")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use rust_decimal::dec;

    type A2 = Amount<2>;

    #[test]
    fn parse_and_display() {
        assert_eq!(A2::parse("1.5").unwrap().to_raw(), 150);
        assert_eq!(A2::parse("+1.50").unwrap().to_string(), "1.50");
        assert_eq!(A2::parse("-0.01").unwrap().to_string(), "-0.01");
        assert_eq!(A2::parse("-0").unwrap(), A2::ZERO);
        assert_eq!(A2::parse("1.2300").unwrap().to_raw(), 123);
        assert_eq!(Amount::<0>::parse("42").unwrap().to_string(), "42");
        assert_eq!(Amount::<5>::default().to_string(), "0.00000");
        assert_eq!(alloc::format!("{:?}", A2::from_raw(-5)), "Amount(-0.05)");
    }

    #[test]
    fn parse_refuses() {
        for bad in ["1,000", "1e3", " 1", "", "-", "1.", ".1", "1 000"] {
            assert!(
                matches!(
                    A2::parse(bad).unwrap_err().kind(),
                    ErrorKind::Malformed { .. }
                ),
                "{bad}"
            );
        }
        assert_eq!(
            A2::parse("1.005").unwrap_err().kind(),
            &ErrorKind::ExcessPrecision {
                value: dec!(1.005),
                decimals: 2
            }
        );
        assert_eq!(
            A2::parse("92233720368547758.08").unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(A2::parse("-92233720368547758.08").unwrap(), A2::MIN);
        assert_eq!(
            A2::parse("1".repeat(60).as_str()).unwrap_err().kind(),
            &ErrorKind::Overflow
        );
    }

    #[test]
    fn extremes_round_trip_through_display() {
        for a in [
            A2::MIN,
            A2::MAX,
            Amount::<18>::MIN
                .rescale::<2>(Rounding::TowardZero)
                .unwrap(),
        ] {
            assert_eq!(A2::parse(&a.to_string()).unwrap(), a);
        }
        let m = Amount::<18>::MIN;
        assert_eq!(Amount::<18>::parse(&m.to_string()).unwrap(), m);
        assert_eq!(m.to_string(), "-9.223372036854775808");
    }

    #[test]
    fn decimal_conversions() {
        assert_eq!(A2::from_decimal(dec!(1.230)).unwrap().to_raw(), 123);
        assert_eq!(
            A2::from_decimal(dec!(1.234)).unwrap_err().kind(),
            &ErrorKind::ExcessPrecision {
                value: dec!(1.234),
                decimals: 2
            }
        );
        assert_eq!(
            A2::from_decimal_rounded(dec!(-1.005), Rounding::HalfAwayFromZero)
                .unwrap()
                .to_raw(),
            -101
        );
        assert_eq!(
            A2::from_decimal_rounded(dec!(-1.005), Rounding::HalfEven)
                .unwrap()
                .to_raw(),
            -100
        );
        assert_eq!(A2::from_raw(-150).to_decimal().to_string(), "-1.50");
        assert_eq!(Decimal::from(A2::from_raw(7)), dec!(0.07));
        assert_eq!(
            A2::from_decimal(Decimal::MAX).unwrap_err().kind(),
            &ErrorKind::Overflow
        );
    }

    #[test]
    fn rounding_modes() {
        let cases = [
            // value, HAFZ, HE, AFZ, TZ
            (dec!(2.5), 3, 2, 3, 2),
            (dec!(3.5), 4, 4, 4, 3),
            (dec!(2.4), 2, 2, 3, 2),
            (dec!(2.6), 3, 3, 3, 2),
            (dec!(2.0), 2, 2, 2, 2),
        ];
        for (v, hafz, he, afz, tz) in cases {
            for (sign, v) in [(1, v), (-1, -v)] {
                let r = |m| Amount::<0>::from_decimal_rounded(v, m).unwrap().to_raw();
                assert_eq!(r(Rounding::HalfAwayFromZero), sign * hafz, "{v}");
                assert_eq!(r(Rounding::HalfEven), sign * he, "{v}");
                assert_eq!(r(Rounding::AwayFromZero), sign * afz, "{v}");
                assert_eq!(r(Rounding::TowardZero), sign * tz, "{v}");
            }
        }
        assert_eq!(Rounding::HalfEven.div(i128::MIN, 1), i128::MIN);
        assert_eq!(Rounding::AwayFromZero.div(i128::MIN, 3), i128::MIN / 3 - 1);
    }

    #[test]
    fn checked_arithmetic() {
        let one = A2::from_raw(100);
        assert_eq!(one.checked_add(one).unwrap().to_raw(), 200);
        assert_eq!(one.checked_sub(one).unwrap(), A2::ZERO);
        assert_eq!(
            A2::MAX.checked_add(A2::from_raw(1)).unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(
            A2::MIN.checked_sub(A2::from_raw(1)).unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(
            A2::MIN.checked_neg().unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(
            A2::MIN.checked_abs().unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(A2::from_raw(-3).checked_abs().unwrap().to_raw(), 3);
        assert_eq!(A2::from_raw(-3).checked_neg().unwrap().to_raw(), 3);
        assert_eq!(A2::checked_sum([one, one, -one]).unwrap(), one);
        assert!(A2::checked_sum([A2::MAX, one]).is_err());
        let mut x = one;
        x += one;
        x -= A2::from_raw(50);
        assert_eq!(x.to_raw(), 150);
        assert_eq!([one, one].iter().sum::<A2>().to_raw(), 200);
        assert_eq!([one, one].into_iter().sum::<A2>().to_raw(), 200);
        assert_eq!((one - one - one).signum(), -1);
        assert!(one.is_positive() && !one.is_negative() && !one.is_zero());
        assert!((-one).is_negative() && A2::ZERO.is_zero() && A2::ZERO.signum() == 0);
        assert_eq!(one.signum(), 1);
    }

    #[test]
    #[should_panic(expected = "overflowed")]
    fn operator_panics_on_overflow() {
        let _ = A2::MAX + A2::from_raw(1);
    }

    #[test]
    fn rounding_to_decimals_and_increments() {
        let a = Amount::<5>::parse("1.00500").unwrap();
        assert_eq!(
            a.round_to(2, Rounding::HalfAwayFromZero)
                .unwrap()
                .to_string(),
            "1.01000"
        );
        assert_eq!(
            a.round_to(2, Rounding::HalfEven).unwrap().to_string(),
            "1.00000"
        );
        assert_eq!(a.round_to(5, Rounding::TowardZero).unwrap(), a);
        assert_eq!(a.round_to(9, Rounding::TowardZero).unwrap(), a);
        assert_eq!(
            Amount::<5>::MAX
                .round_to(0, Rounding::AwayFromZero)
                .unwrap_err()
                .kind(),
            &ErrorKind::Overflow
        );
        for bad in [
            Amount::ZERO,
            Amount::<5>::parse("-0.05").unwrap(),
            Amount::MIN,
        ] {
            assert_eq!(
                a.round_to_increment(bad, Rounding::HalfEven)
                    .unwrap_err()
                    .kind(),
                &ErrorKind::InvalidIncrement
            );
        }
        let five = Amount::<5>::parse("0.05").unwrap();
        assert_eq!(
            Amount::<5>::parse("0.025")
                .unwrap()
                .round_to_increment(five, Rounding::HalfEven)
                .unwrap(),
            Amount::ZERO
        );
        assert_eq!(
            Amount::<5>::parse("0.075")
                .unwrap()
                .round_to_increment(five, Rounding::HalfEven)
                .unwrap()
                .to_string(),
            "0.10000"
        );
    }

    #[test]
    fn fits_rescale_exact() {
        let a = Amount::<5>::parse("1.23").unwrap();
        assert!(a.fits(2) && a.fits(5) && a.fits(7) && !a.fits(1));
        assert_eq!(a.exact::<2>().unwrap().to_string(), "1.23");
        assert_eq!(
            a.exact::<1>().unwrap_err().kind(),
            &ErrorKind::ExcessPrecision {
                value: dec!(1.23000),
                decimals: 1
            }
        );
        assert_eq!(
            a.rescale::<1>(Rounding::HalfAwayFromZero)
                .unwrap()
                .to_string(),
            "1.2"
        );
        assert_eq!(
            a.rescale::<8>(Rounding::TowardZero).unwrap().to_string(),
            "1.23000000"
        );
        assert_eq!(
            Amount::<5>::MAX
                .rescale::<8>(Rounding::TowardZero)
                .unwrap_err()
                .kind(),
            &ErrorKind::Overflow
        );
    }

    #[test]
    fn scale_reduces_once() {
        let s = AmountScale::EN16931;
        assert_eq!(
            s.reduce::<5>(dec!(1.9049951)).unwrap().to_string(),
            "1.90000"
        );
        assert_eq!(s.reduce::<2>(dec!(-1.005)).unwrap().to_string(), "-1.01");
        assert_eq!(
            s.reduce::<1>(dec!(1)).unwrap_err().kind(),
            &ErrorKind::ScaleMismatch
        );
        let bad = AmountScale {
            decimals: 6,
            rounding: Rounding::HalfEven,
        };
        assert_eq!(
            bad.reduce::<8>(dec!(1)).unwrap_err().kind(),
            &ErrorKind::ScaleMismatch
        );
        assert_eq!(
            AmountScale::new(0, Rounding::TowardZero)
                .unwrap()
                .reduce::<5>(dec!(-2.9))
                .unwrap()
                .to_raw(),
            -200_000
        );
    }

    #[test]
    fn scale_new_refuses_past_five() {
        for d in 0..=5 {
            assert_eq!(AmountScale::new(d, Rounding::HalfEven).unwrap().decimals, d);
        }
        for d in [6, 18, u8::MAX] {
            assert_eq!(
                AmountScale::new(d, Rounding::HalfEven).unwrap_err().kind(),
                &ErrorKind::ScaleMismatch
            );
        }
    }

    #[test]
    fn excess_precision_is_reported_before_overflow() {
        // Both faults: the excess digit wins, for parse and from_decimal alike.
        let both = "92233720368547758.081";
        for err in [
            A2::parse(both).unwrap_err(),
            A2::from_decimal(both.parse().unwrap()).unwrap_err(),
        ] {
            assert!(matches!(
                err.kind(),
                ErrorKind::ExcessPrecision { decimals: 2, .. }
            ));
        }
        // Past Decimal's range only Overflow can be said.
        let huge = alloc::format!("{}.001", "9".repeat(40));
        assert_eq!(A2::parse(&huge).unwrap_err().kind(), &ErrorKind::Overflow);
    }

    #[test]
    fn display_honours_precision() {
        use alloc::format;
        let a = Amount::<5>::parse("2.34500").unwrap();
        assert_eq!(format!("{a:.2}"), "2.35");
        assert_eq!(format!("{:.2}", -a), "-2.35");
        assert_eq!(format!("{a:.5}"), "2.34500");
        assert_eq!(format!("{a:.6}"), "2.345000");
        assert_eq!(format!("{a:.0}"), "2");
        assert_eq!(format!("{:.0}", Amount::<5>::parse("-0.5").unwrap()), "-1");
        assert_eq!(
            format!("{:.0}", Amount::<5>::parse("-0.49999").unwrap()),
            "0"
        );
        assert_eq!(format!("{a:+.1}"), "+2.3");
        assert_eq!(format!("{a:08.2}"), "00002.35");
        assert_eq!(format!("{:<7}|", A2::from_raw(-5)), "-0.05  |");
        assert_eq!(format!("{:.1}", Amount::<0>::from_raw(7)), "7.0");
        assert_eq!(format!("{:.2}", Amount::<5>::MIN), "-92233720368547.76");
        assert_eq!(format!("{:.4}", Amount::<18>::MAX), "9.2234");
    }
}
