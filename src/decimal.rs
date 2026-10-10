//! The crate's one decimal grammar, `[+-]digits[.digits]`, and the serde string
//! codec built on it. Private: the public entry points are `Amount::parse` and the
//! serde impls.

use rust_decimal::Decimal;

use crate::{Error, ErrorKind, Result, Rounding};

/// A lexically valid decimal: sign, integer digits, fraction digits (possibly
/// empty). Leading and trailing zeros are kept as written.
pub(crate) struct Lexed<'a> {
    pub(crate) negative: bool,
    pub(crate) int: &'a str,
    pub(crate) frac: &'a str,
}

/// Lex `s` against `[+-]digits[.digits]`: ASCII digits only, `.` as the only
/// separator, at least one digit on each side of a `.`, no spaces, no exponent.
pub(crate) fn lex(s: &str) -> Result<Lexed<'_>> {
    let malformed = || ErrorKind::Malformed { input: s.into() };
    let (negative, body) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let (int, frac) = match body.split_once('.') {
        Some((i, f)) => {
            if f.is_empty() {
                return Err(malformed().into());
            }
            (i, f)
        }
        None => (body, ""),
    };
    let digits = |p: &str| p.bytes().all(|b| b.is_ascii_digit());
    if int.is_empty() || !digits(int) || !digits(frac) {
        return Err(malformed().into());
    }
    Ok(Lexed {
        negative,
        int,
        frac,
    })
}

/// Parse a `Decimal` in the crate grammar, keeping the written scale
/// (`"1.50"` has scale 2). Refuses what `Decimal` cannot hold exactly.
pub(crate) fn parse(s: &str) -> Result<Decimal> {
    let lexed = lex(s)?;
    let unsigned = s.trim_start_matches(['+', '-']);
    match Decimal::from_str_exact(unsigned) {
        Ok(d) => Ok(if lexed.negative { -d } else { d }),
        Err(_) => {
            let int = lexed.int.trim_start_matches('0');
            // 29 integer digits can exceed 2^96 − 1; more always do.
            if int.len() > 28 {
                return Err(ErrorKind::Overflow.into());
            }
            // Too many fraction digits: report the value rounded to Decimal's
            // 28 decimals, the most it can say.
            let rounded = unsigned
                .parse::<Decimal>()
                .map_err(|_| ErrorKind::Overflow)?;
            let value = if lexed.negative { -rounded } else { rounded };
            Err(ErrorKind::ExcessPrecision {
                value,
                decimals: 28,
            }
            .into())
        }
    }
}

/// `10^n` as `i128`, `None` past `10^38`.
pub(crate) const fn pow10(n: u32) -> Option<i128> {
    10i128.checked_pow(n)
}

/// `round((n / 10^ns) / (q / 10^qs))` in units of `10^-decimals`, exactly: one
/// integer division, one rounding by `rounding`. [`ErrorKind::DivisionByZero`]
/// for `q = 0`, [`ErrorKind::Overflow`] past `i128`.
pub(crate) fn ratio_units(
    (mut n, ns): (i128, u32),
    (mut q, qs): (i128, u32),
    decimals: u32,
    rounding: Rounding,
) -> Result<i128> {
    let overflow = || Error::from(ErrorKind::Overflow);
    if q == 0 {
        return Err(ErrorKind::DivisionByZero.into());
    }
    if n == 0 {
        return Ok(0);
    }
    let e = i64::from(qs) + i64::from(decimals) - i64::from(ns);
    let p = |e: i64| u32::try_from(e).ok().and_then(pow10).ok_or_else(overflow);
    if e >= 0 {
        n = n.checked_mul(p(e)?).ok_or_else(overflow)?;
    } else {
        q = q.checked_mul(p(-e)?).ok_or_else(overflow)?;
    }
    if q < 0 {
        q = q.checked_neg().ok_or_else(overflow)?;
        n = n.checked_neg().ok_or_else(overflow)?;
    }
    Ok(rounding.div(n, q))
}

/// `num / den` rounded once to `decimals` (`0..=28`) by `rounding`, exactly
/// (integer arithmetic on the mantissas). [`ErrorKind::DivisionByZero`] for a
/// zero `den`, [`ErrorKind::Overflow`] past `i128` or `Decimal`,
/// [`ErrorKind::ScaleMismatch`] for `decimals > 28`.
pub(crate) fn round_ratio(
    num: Decimal,
    den: Decimal,
    decimals: u8,
    rounding: Rounding,
) -> Result<Decimal> {
    if decimals > MAX_DECIMALS {
        return Err(ErrorKind::ScaleMismatch.into());
    }
    let (num, den) = (num.normalize(), den.normalize());
    let units = ratio_units(
        (num.mantissa(), num.scale()),
        (den.mantissa(), den.scale()),
        u32::from(decimals),
        rounding,
    )?;
    Ok(
        Decimal::try_from_i128_with_scale(units, u32::from(decimals))
            .map_err(|_| ErrorKind::Overflow)?,
    )
}

/// The most decimals a `Decimal` carries.
pub(crate) const MAX_DECIMALS: u8 = 28;

/// `m · 10^-s` as a `Decimal`, dropping trailing zeros past 28 decimals;
/// `None` if it does not fit exactly.
fn from_parts_exact(mut m: i128, mut s: u32) -> Option<Decimal> {
    while s > u32::from(MAX_DECIMALS) && m % 10 == 0 {
        m /= 10;
        s -= 1;
    }
    Decimal::try_from_i128_with_scale(m, s).ok()
}

/// `a + b`, exactly: `None` where `Decimal`'s own addition would round or
/// overflow.
pub(crate) fn exact_add(a: Decimal, b: Decimal) -> Option<Decimal> {
    let s = a.scale().max(b.scale());
    let up = |d: Decimal| d.mantissa().checked_mul(pow10(s - d.scale())?);
    from_parts_exact(up(a)?.checked_add(up(b)?)?, s)
}

/// `a × b`, exactly: `None` where `Decimal`'s own multiplication would round
/// or overflow.
pub(crate) fn exact_mul(a: Decimal, b: Decimal) -> Option<Decimal> {
    let (a, b) = (a.normalize(), b.normalize());
    from_parts_exact(
        a.mantissa().checked_mul(b.mantissa())?,
        a.scale() + b.scale(),
    )
}

#[cfg(feature = "serde")]
pub(crate) mod serde_str {
    //! `#[serde(with = "crate::decimal::serde_str")]` for `Decimal` fields:
    //! serialise via `Display`, deserialise strings only, in the crate grammar.

    use core::fmt;
    use core::marker::PhantomData;

    use rust_decimal::Decimal;
    use serde::de::{self, Visitor};
    use serde::{Deserializer, Serializer};

    use crate::Result;

    pub(crate) fn serialize<S: Serializer>(d: &Decimal, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(d)
    }

    pub(crate) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Decimal, D::Error> {
        from_str(d, super::parse, "a decimal string such as \"12.50\"")
    }

    /// Deserialise a string (never a number) through `parse`.
    pub(crate) fn from_str<'de, D, T>(
        d: D,
        parse: fn(&str) -> Result<T>,
        expecting: &'static str,
    ) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct StrVisitor<T> {
            parse: fn(&str) -> Result<T>,
            expecting: &'static str,
            marker: PhantomData<T>,
        }
        impl<T> Visitor<'_> for StrVisitor<T> {
            type Value = T;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.expecting)
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<T, E> {
                (self.parse)(v).map_err(E::custom)
            }
        }
        d.deserialize_str(StrVisitor {
            parse,
            expecting,
            marker: PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;
    use rust_decimal::dec;

    #[test]
    fn round_ratio_rounds_once_in_every_mode() {
        use Rounding::{AwayFromZero, HalfAwayFromZero, HalfEven, TowardZero};
        let r = |n: Decimal, d: Decimal, k: u8, m: Rounding| round_ratio(n, d, k, m).unwrap();
        // 7201 s → 3 started hours; 7200 s → exactly 2.
        assert_eq!(r(dec!(7201), dec!(3600), 0, AwayFromZero), dec!(3));
        assert_eq!(r(dec!(7200), dec!(3600), 0, AwayFromZero), dec!(2));
        assert_eq!(r(dec!(1536), dec!(1024), 0, TowardZero), dec!(1));
        // Ties: 5/2 = 2.5 and −2.5, 7/2 = 3.5.
        for (n, half_away, half_even, away, toward) in [
            (dec!(5), 3, 2, 3, 2),
            (dec!(-5), -3, -2, -3, -2),
            (dec!(7), 4, 4, 4, 3),
            (dec!(-7), -4, -4, -4, -3),
        ] {
            let two = dec!(2);
            assert_eq!(
                r(n, two, 0, HalfAwayFromZero),
                Decimal::from(half_away),
                "{n}"
            );
            assert_eq!(r(n, two, 0, HalfEven), Decimal::from(half_even), "{n}");
            assert_eq!(r(n, two, 0, AwayFromZero), Decimal::from(away), "{n}");
            assert_eq!(r(n, two, 0, TowardZero), Decimal::from(toward), "{n}");
        }
        // Not a tie: 1/3 at 2 dp; 2/3 at 28 dp.
        assert_eq!(r(dec!(1), dec!(3), 2, HalfEven), dec!(0.33));
        assert_eq!(r(dec!(1), dec!(3), 2, AwayFromZero), dec!(0.34));
        assert_eq!(r(dec!(2), dec!(3), 2, HalfAwayFromZero), dec!(0.67));
        assert_eq!(r(dec!(-2), dec!(3), 2, TowardZero), dec!(-0.66));
        // A negative denominator moves the sign to the numerator.
        assert_eq!(r(dec!(1), dec!(-4), 1, HalfAwayFromZero), dec!(-0.3));
        assert_eq!(r(dec!(0), dec!(7), 3, AwayFromZero), dec!(0));
        assert_eq!(
            round_ratio(dec!(1), dec!(0), 0, TowardZero)
                .unwrap_err()
                .kind(),
            &ErrorKind::DivisionByZero
        );
        assert_eq!(
            round_ratio(dec!(1), dec!(3), 29, TowardZero)
                .unwrap_err()
                .kind(),
            &ErrorKind::ScaleMismatch
        );
        assert_eq!(
            round_ratio(
                Decimal::MAX,
                dec!(0.0000000000000000000000000001),
                28,
                TowardZero
            )
            .unwrap_err()
            .kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(r(dec!(1), dec!(3), 28, TowardZero).scale(), 28);
    }

    #[test]
    fn exact_add_and_mul_refuse_what_would_round() {
        assert_eq!(exact_add(dec!(1.5), dec!(-0.25)), Some(dec!(1.25)));
        assert_eq!(exact_mul(dec!(1.5), dec!(-0.25)), Some(dec!(-0.375)));
        assert_eq!(exact_mul(dec!(2.50), dec!(4)), Some(dec!(10)));
        // 10^28 + 0.1 needs 30 digits; Decimal would round it away.
        let big = Decimal::from_i128_with_scale(10i128.pow(28), 0);
        assert_eq!(exact_add(big, dec!(0.1)), None);
        assert_eq!(exact_add(Decimal::MAX, dec!(1)), None);
        assert_eq!(exact_mul(Decimal::MAX, dec!(2)), None);
        // 0.1^15 needs 30 decimals.
        let tiny = dec!(0.000000000000001);
        assert_eq!(exact_mul(tiny, tiny), None);
        // A product at exactly 28 decimals keeps its scale.
        let at_28 = exact_mul(dec!(0.0000000000000000000000000002), dec!(5)).unwrap();
        assert_eq!(
            (at_28, at_28.scale()),
            (dec!(0.000000000000000000000000001), 28)
        );
        // Trailing zeros past 28 decimals are dropped, not refused.
        assert_eq!(
            exact_mul(dec!(0.00000000000002), dec!(0.000000000000005)),
            Some(dec!(0.0000000000000000000000000001))
        );
    }

    #[test]
    fn grammar_accepts_only_dot_decimals() {
        for ok in ["0", "1", "-1", "+1", "1.5", "-0.001", "007.100"] {
            assert!(parse(ok).is_ok(), "{ok}");
        }
        for bad in [
            "", "-", "+", ".5", "5.", "1,5", "1e3", "1E3", " 1", "1 ", "1_000", "--1", "+-1",
            "1.2.3", "0x10", "١", "NaN", "inf",
        ] {
            assert!(
                matches!(parse(bad).unwrap_err().kind(), ErrorKind::Malformed { .. }),
                "{bad}"
            );
        }
    }

    #[test]
    fn parse_keeps_scale_and_sign() {
        assert_eq!(parse("1.50").unwrap().to_string(), "1.50");
        assert_eq!(parse("-1.50").unwrap().to_string(), "-1.50");
        assert_eq!(parse("+2").unwrap().to_string(), "2");
    }

    #[test]
    fn parse_refuses_what_decimal_cannot_hold() {
        assert_eq!(
            parse("123456789012345678901234567890").unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        assert!(matches!(
            parse("0.12345678901234567890123456789").unwrap_err().kind(),
            ErrorKind::ExcessPrecision { decimals: 28, .. }
        ));
        // 28 integer digits always fit: too many digits overall is excess
        // precision, not overflow.
        assert!(matches!(
            parse("1234567890123456789012345678.12").unwrap_err().kind(),
            ErrorKind::ExcessPrecision { decimals: 28, .. }
        ));
        // 29 integer digits are refused as overflow even when they would fit
        // 2^96 once the fraction is rounded away.
        assert_eq!(
            parse("70000000000000000000000000000.5").unwrap_err().kind(),
            &ErrorKind::Overflow
        );
        // The reported value keeps the sign.
        let ErrorKind::ExcessPrecision { value, .. } = *parse("-0.12345678901234567890123456789")
            .unwrap_err()
            .kind()
        else {
            panic!("excess precision expected");
        };
        assert!(value.is_sign_negative());
    }
}
