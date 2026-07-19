//! [`Amount<P>`] — fixed-point monetary arithmetic with compile-time precision.
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive as _;
use std::fmt::{self, Write as _};
use std::str::FromStr;

use crate::error::{BillingError, ParseAmountError};

// ── RoundingStrategy ─────────────────────────────────────────────────────────

/// Explicit rounding strategy. Always required — no hidden defaults.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RoundingStrategy {
    /// Rounds midpoint 0.5 away from zero (also known as commercial or
    /// half-up rounding). The most common choice for invoicing.
    MidpointAwayFromZero,
    /// Rounds midpoint 0.5 to the nearest even digit (banker's rounding).
    /// Minimises cumulative rounding bias over many operations.
    MidpointToEven,
    /// Always round toward positive infinity.
    Ceiling,
    /// Always round toward negative infinity.
    Floor,
    /// Truncate toward zero (discard fractional digits).
    Truncate,
}

impl From<RoundingStrategy> for rust_decimal::RoundingStrategy {
    fn from(s: RoundingStrategy) -> Self {
        match s {
            RoundingStrategy::MidpointAwayFromZero => {
                rust_decimal::RoundingStrategy::MidpointAwayFromZero
            }
            RoundingStrategy::MidpointToEven => rust_decimal::RoundingStrategy::MidpointNearestEven,
            RoundingStrategy::Ceiling => rust_decimal::RoundingStrategy::ToPositiveInfinity,
            RoundingStrategy::Floor => rust_decimal::RoundingStrategy::ToNegativeInfinity,
            RoundingStrategy::Truncate => rust_decimal::RoundingStrategy::ToZero,
        }
    }
}

// ── Amount<P> ─────────────────────────────────────────────────────────────────

/// Fixed-point monetary amount with `P` decimal places.
///
/// Stored internally as an `i64` scaled by `10^P`.  All arithmetic is exact —
/// no `f64` intermediate.  Overflow always panics (infallible ops) or returns
/// `Err` (fallible `checked_*` ops).
///
/// # Internal representation
///
/// | Value    | P | Raw `i64` |
/// |----------|---|-----------|
/// | 0.03456  | 5 | 3 456     |
/// | 49.99    | 2 | 4 999     |
/// | -100.00  | 5 | -10 000 000 |
///
/// # Parsing
///
/// [`Amount::parse`] accepts `"."` and `","` as decimal separators.
/// It rejects strings that carry **more non-zero digits than P**:
/// `Amount::<5>::parse("1.000011")` → `Err` (the 6th digit `1` cannot be
/// represented without loss).  Trailing zeros beyond P are accepted.
///
/// # Common type aliases
///
/// ```rust
/// use billing::{EuroAmount, InvoiceAmt};
/// let _: EuroAmount  = billing::Amount::parse("0.03456").unwrap(); // 5 dp
/// let _: InvoiceAmt  = billing::Amount::parse("49.99").unwrap();   // 2 dp
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[must_use = "Amount is an immutable value; every operation returns a new one"]
pub struct Amount<const P: u8>(i64);

impl<const P: u8> Amount<P> {
    /// Compile-time guard: `10^P` must fit in `i64`, so `P ≤ 18`.
    ///
    /// Referenced from [`Amount::SCALE`] so that any use of an out-of-range
    /// precision fails during const evaluation with this message rather than
    /// with a bare "attempt to compute `1000000000000000000_i64 * 10_i64`,
    /// which would overflow".
    const PRECISION_SUPPORTED: () = assert!(
        P <= 18,
        "Amount<P>: P must be <= 18 — 10^19 exceeds i64::MAX and cannot be represented"
    );

    /// Zero amount.
    pub const ZERO: Self = Self(0);

    /// The maximum representable value: `i64::MAX × 10⁻ᴾ`.
    ///
    /// For `Amount<5>` this is `92_233_720_368_547.75807`.
    pub const MAX: Self = Self(i64::MAX);

    /// The minimum representable value: `i64::MIN × 10⁻ᴾ`.
    ///
    /// Note: `Amount::MIN.abs()` panics — `i64::MIN` has no positive counterpart.
    /// Use `Amount::MAX` for bound checks where sign doesn't matter.
    pub const MIN: Self = Self(i64::MIN);

    pub(crate) const SCALE: i64 = {
        // Force the precision assertion to evaluate before the multiplication
        // below, so an out-of-range P reports the explanatory message.
        let () = Self::PRECISION_SUPPORTED;
        let mut s = 1i64;
        let mut i = 0u8;
        while i < P {
            s *= 10;
            i += 1;
        }
        s
    };

    /// Parse a decimal string into `Amount<P>`.
    ///
    /// Accepts `.` and `,` as decimal separators.  Returns `Err` when:
    /// - the string is empty or non-numeric,
    /// - the value would overflow `i64`, or
    /// - the string carries **non-zero digits beyond `P`** decimal places
    ///   (excess trailing zeros are accepted).
    ///
    /// # Examples
    /// ```rust
    /// use billing::Amount;
    /// assert_eq!(Amount::<5>::parse("0.03456").unwrap().to_raw(), 3_456);
    /// assert_eq!(Amount::<2>::parse("49.99").unwrap().to_raw(),   4_999);
    /// assert!(Amount::<5>::parse("").is_err());
    /// // Non-zero digit beyond precision → Err
    /// assert!(Amount::<5>::parse("0.123456").is_err());
    /// // Trailing zeros beyond precision → Ok
    /// assert!(Amount::<5>::parse("0.100000").is_ok());
    /// ```
    pub fn parse(s: &str) -> Result<Self, ParseAmountError> {
        let err = || ParseAmountError {
            input: s.to_owned(),
        };
        let s = s.trim();
        if s.is_empty() {
            return Err(err());
        }
        let s_norm;
        let s: &str = if s.contains(',') {
            s_norm = s.replace(',', ".");
            &s_norm
        } else {
            s
        };

        let negative = s.starts_with('-');
        let s = s
            .strip_prefix('-')
            .or_else(|| s.strip_prefix('+'))
            .unwrap_or(s);

        // Reject a second sign character (e.g. "--5.0" or "+-3.0").
        if s.starts_with('-') || s.starts_with('+') {
            return Err(err());
        }

        let (whole_str, frac_str) = if let Some((w, f)) = s.split_once('.') {
            (w, f)
        } else {
            (s, "")
        };

        // Fractional part must contain only ASCII digits — no signs, no letters.
        if !frac_str.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err());
        }

        // Parse `whole` as i128 so that the edge case where the whole part equals
        // |i64::MIN| / SCALE is handled correctly.  For P=0 and Amount::<0>::MIN,
        // the whole string is "9223372036854775808" which overflows i64 but fits
        // in i128 and is valid after negation (= i64::MIN).
        let whole: i128 = whole_str.parse().map_err(|_| err())?;

        // Reject non-zero digits beyond P decimal places.
        if frac_str.len() > P as usize {
            let extra = &frac_str[P as usize..];
            if extra.bytes().any(|b| b != b'0') {
                return Err(err());
            }
        }

        // Pad fractional part to exactly P digits.
        let trunc_len = frac_str.len().min(P as usize);
        let frac_padded = format!("{:0<width$}", &frac_str[..trunc_len], width = P as usize);
        // When P=0 (integer-only amounts) the padded frac string is empty;
        // treat it as 0 rather than failing the parse.
        let frac: i64 = if frac_padded.is_empty() {
            0
        } else {
            frac_padded.parse().map_err(|_| err())?
        };

        // Use i128 for the intermediate product so that Amount::MIN can be parsed.
        //
        // The magnitude of i64::MIN is 9_223_372_036_854_775_808, which exceeds i64::MAX
        // (9_223_372_036_854_775_807) by 1.  If we computed `whole * SCALE + frac` as i64
        // and then negated, the intermediate would overflow before the negation step,
        // causing `parse(Amount::MIN.to_string())` to return Err — a round-trip violation.
        //
        // With i128 the full magnitude fits, and we convert back to i64 only after
        // applying the sign and confirming the result is in [i64::MIN, i64::MAX].
        let unsigned_mag: i128 = whole
            .checked_mul(Self::SCALE as i128)
            .and_then(|w| w.checked_add(frac as i128))
            .ok_or_else(err)?;

        let raw: i64 = if negative {
            let negated = -(unsigned_mag);
            if negated < i64::MIN as i128 {
                return Err(err()); // magnitude too large even for i64::MIN
            }
            negated as i64 // safe: negated ∈ [i64::MIN, 0]
        } else {
            if unsigned_mag > i64::MAX as i128 {
                return Err(err());
            }
            unsigned_mag as i64 // safe: unsigned_mag ∈ [0, i64::MAX]
        };
        Ok(Self(raw))
    }

    /// Construct from a `rust_decimal::Decimal`. Returns `None` on overflow.
    ///
    /// The value is scaled by `10^P` and then rounded to an integer with
    /// `MidpointAwayFromZero` — equivalent to rounding to `P` decimal places, but
    /// note the scale-first order slightly narrows the accepted range near
    /// `Decimal::MAX`. Returns `None` when the scaled integer overflows `i64`, or
    /// when the intermediate `Decimal` multiplication overflows its 96-bit mantissa.
    ///
    /// For a `Result`-returning version that works with `?`, use
    /// [`Amount::checked_from_decimal`].
    ///
    /// ```rust
    /// use billing::Amount;
    /// use rust_decimal::Decimal;
    /// // Never panics, even at the extremes of Decimal's range.
    /// assert_eq!(Amount::<5>::from_decimal(Decimal::MAX), None);
    /// assert_eq!(Amount::<5>::from_decimal(Decimal::MIN), None);
    /// ```
    #[must_use]
    pub fn from_decimal(d: Decimal) -> Option<Self> {
        // `Decimal`'s `Mul` impl PANICS on overflow rather than saturating, so the
        // checked form is mandatory here: this constructor is documented as
        // returning `None`, and `checked_from_decimal` / `checked_mul_qty` /
        // `LineItem::build` all funnel their fallible paths through it.
        d.checked_mul(Decimal::from(Self::SCALE))?
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
            .map(Self)
    }

    /// Construct from a `Decimal`, returning `Err` on overflow.
    ///
    /// Rounds `d` to `P` decimal places (`MidpointAwayFromZero`) before scaling.
    /// This is the `?`-compatible counterpart of [`Amount::from_decimal`].
    ///
    /// ```rust
    /// use billing::Amount;
    /// let a = Amount::<5>::checked_from_decimal(
    ///     rust_decimal::Decimal::from_str_exact("1.23456").unwrap()
    /// ).unwrap();
    /// assert_eq!(a, Amount::parse("1.23456").unwrap());
    /// ```
    pub fn checked_from_decimal(d: Decimal) -> Result<Self, BillingError> {
        Self::from_decimal(d).ok_or(BillingError::MonetaryOverflow {
            precision: P,
            input_value: Some(d),
        })
    }

    /// Construct from a `rust_decimal::Decimal`. Returns `Err` on overflow.
    pub fn try_from_decimal(d: Decimal) -> Result<Self, ParseAmountError> {
        Self::from_decimal(d).ok_or_else(|| ParseAmountError {
            input: d.to_string(),
        })
    }

    /// Convert to `rust_decimal::Decimal` (lossless, exact).
    ///
    /// `Amount` is `Copy`, so this borrows nothing and consumes nothing despite the
    /// `into_` prefix. `Decimal::from(amount)` is equivalent.
    ///
    /// ```rust
    /// use billing::Amount;
    /// let a = Amount::<5>::parse("1.23456").unwrap();
    /// assert_eq!(a.into_decimal(), rust_decimal::Decimal::from_str_exact("1.23456").unwrap());
    /// assert_eq!(a.into_decimal(), a.into_decimal()); // Copy: usable repeatedly
    /// ```
    #[must_use]
    pub fn into_decimal(self) -> Decimal {
        Decimal::new(self.0, P as u32)
    }

    /// Checked addition. Returns `Err` on overflow.
    pub fn checked_add(self, rhs: Self) -> Result<Self, BillingError> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: None,
            })
    }

    /// Checked subtraction. Returns `Err` on overflow.
    pub fn checked_sub(self, rhs: Self) -> Result<Self, BillingError> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: None,
            })
    }

    /// Checked negation.
    pub fn checked_neg(self) -> Result<Self, BillingError> {
        self.0
            .checked_neg()
            .map(Self)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: Some(self.into_decimal()),
            })
    }

    /// Multiply a per-unit price by a quantity (`Decimal`).
    ///
    /// Uses `rust_decimal` arithmetic — no `f64` intermediate.
    /// The product is rounded to `P` decimal places using
    /// `MidpointAwayFromZero` (commercial rounding).
    /// Result precision = `P` (LHS precision).
    ///
    /// # Panics
    /// Panics if the result exceeds the representable `i64` range.
    /// Use [`Amount::checked_mul_qty`] for a fallible alternative.
    pub fn mul_qty(self, qty: Decimal) -> Self {
        self.checked_mul_qty(qty)
            .expect("monetary overflow in mul_qty")
    }

    /// Multiply by a quantity, returning `Err` on overflow.
    ///
    /// Never panics — including when `qty` is at the extremes of `Decimal`'s range.
    ///
    /// ```rust
    /// use billing::Amount;
    /// use rust_decimal::Decimal;
    /// let price = Amount::<5>::parse("1000000.00000").unwrap();
    /// assert!(price.checked_mul_qty(Decimal::MAX).is_err());
    /// ```
    pub fn checked_mul_qty(self, qty: Decimal) -> Result<Self, BillingError> {
        // `Decimal * Decimal` panics on overflow, which would break this method's
        // documented `Result` contract — use the checked form.
        let product = self
            .into_decimal()
            .checked_mul(qty)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: None,
            })?
            .round_dp_with_strategy(
                P as u32,
                rust_decimal::RoundingStrategy::MidpointAwayFromZero,
            );
        Self::from_decimal(product).ok_or(BillingError::MonetaryOverflow {
            precision: P,
            input_value: None,
        })
    }

    /// Divide by a `Decimal` divisor, returning `Err` on overflow or division by zero.
    ///
    /// The quotient is rounded to `P` decimal places with `MidpointAwayFromZero`.
    /// Useful for deriving a unit price from a total (`total / quantity`).
    ///
    /// ```rust
    /// use billing::Amount;
    /// use rust_decimal::dec;
    /// let total = Amount::<5>::parse("100.00000").unwrap();
    /// assert_eq!(total.checked_div(dec!(4)).unwrap(), Amount::parse("25.00000").unwrap());
    /// assert!(total.checked_div(dec!(0)).is_err());
    /// ```
    pub fn checked_div(self, divisor: Decimal) -> Result<Self, BillingError> {
        // `Decimal`'s `/` panics on both overflow and division by zero.
        let q = self
            .into_decimal()
            .checked_div(divisor)
            .ok_or(BillingError::InvalidInput {
                reason: format!("division by zero or overflow: {self} / {divisor}"),
            })?
            .round_dp_with_strategy(
                P as u32,
                rust_decimal::RoundingStrategy::MidpointAwayFromZero,
            );
        Self::from_decimal(q).ok_or(BillingError::MonetaryOverflow {
            precision: P,
            input_value: None,
        })
    }

    /// Split into `n` parts that sum **exactly** back to `self`.
    ///
    /// The indivisible remainder is spread one smallest-unit at a time across the
    /// leading parts, so parts differ by at most `10⁻ᴾ` and no part absorbs the
    /// whole remainder. This is the monetary-allocation problem from Fowler's
    /// *Patterns of Enterprise Application Architecture*; naive `total / n`
    /// silently loses or invents money.
    ///
    /// For a negative `self` the remainder is distributed in the same direction,
    /// so the sum still reconstructs the original exactly.
    ///
    /// ```rust
    /// use billing::Amount;
    /// // 0.10 split three ways: 0.04 + 0.03 + 0.03 — not 0.033... three times.
    /// let parts = Amount::<2>::parse("0.10").unwrap().distribute(3).unwrap();
    /// assert_eq!(parts.len(), 3);
    /// assert_eq!(parts[0], Amount::<2>::parse("0.04").unwrap());
    /// assert_eq!(parts[1], Amount::<2>::parse("0.03").unwrap());
    /// let sum: Amount<2> = parts.into_iter().sum();
    /// assert_eq!(sum, Amount::<2>::parse("0.10").unwrap());
    /// ```
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `n == 0`.
    pub fn distribute(self, n: usize) -> Result<Vec<Self>, BillingError> {
        if n == 0 {
            return Err(BillingError::InvalidInput {
                reason: "distribute requires n > 0".into(),
            });
        }
        // `as` would truncate (and flip the sign of the division) for n >= 2^63.
        // Unreachable in practice — the Vec allocation dies first — but an explicit
        // bound is cheaper than the reasoning needed to prove that.
        let n_i = i64::try_from(n).map_err(|_| BillingError::InvalidInput {
            reason: format!("distribute: n = {n} is too large"),
        })?;
        // Truncating division plus an explicitly distributed remainder keeps the
        // sum exact for both signs (Rust's `/` and `%` truncate toward zero, so
        // `base * n + rem == self.0` always holds).
        let base = self.0 / n_i;
        let rem = self.0 % n_i;
        let step = if rem >= 0 { 1 } else { -1 };
        let extra = rem.unsigned_abs() as usize;
        Ok((0..n)
            .map(|i| Self(if i < extra { base + step } else { base }))
            .collect())
    }

    /// Split proportionally to integer `ratios`, summing **exactly** back to `self`.
    ///
    /// Uses the largest-remainder method: each part gets `floor(self × ratio / Σratios)`
    /// and the remaining smallest-units go to the parts with the largest fractional
    /// remainders, ties broken by position.
    ///
    /// Prefer this over [`crate::proportional_split`] when the thing being split is
    /// money rather than a physical quantity, and over `checked_mul_qty` with
    /// fractional shares when the shares are naturally integral (seats, days, units).
    ///
    /// ```rust
    /// use billing::Amount;
    /// // A 100.00 bill split 1:1:1 — someone has to take the extra cent.
    /// let parts = Amount::<2>::parse("100.00").unwrap().allocate(&[1, 1, 1]).unwrap();
    /// assert_eq!(parts[0], Amount::<2>::parse("33.34").unwrap());
    /// assert_eq!(parts[1], Amount::<2>::parse("33.33").unwrap());
    /// let sum: Amount<2> = parts.into_iter().sum();
    /// assert_eq!(sum, Amount::<2>::parse("100.00").unwrap());
    /// ```
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `ratios` is empty or sums to zero.
    pub fn allocate(self, ratios: &[u64]) -> Result<Vec<Self>, BillingError> {
        if ratios.is_empty() {
            return Err(BillingError::InvalidInput {
                reason: "allocate requires at least one ratio".into(),
            });
        }
        let total_ratio: u128 = ratios.iter().map(|r| *r as u128).sum();
        if total_ratio == 0 {
            return Err(BillingError::InvalidInput {
                reason: "allocate requires the ratios to sum to more than zero".into(),
            });
        }
        // Work on the magnitude in i128 so the sign is handled uniformly and the
        // intermediate `raw × ratio` cannot overflow.
        let neg = self.0 < 0;
        let magnitude = (self.0 as i128).unsigned_abs();

        let mut parts = Vec::with_capacity(ratios.len());
        let mut remainders = Vec::with_capacity(ratios.len());
        let mut allocated: u128 = 0;
        for &r in ratios {
            let numer = magnitude * r as u128;
            let q = numer / total_ratio;
            remainders.push(numer % total_ratio);
            allocated += q;
            parts.push(q);
        }
        // Hand out the shortfall one unit at a time, largest remainder first.
        let mut order: Vec<usize> = (0..ratios.len()).collect();
        order.sort_by(|&a, &b| remainders[b].cmp(&remainders[a]).then_with(|| a.cmp(&b)));
        let mut shortfall = magnitude - allocated;
        for &idx in order.iter() {
            if shortfall == 0 {
                break;
            }
            parts[idx] += 1;
            shortfall -= 1;
        }

        parts
            .into_iter()
            .map(|p| {
                let signed = if neg { -(p as i128) } else { p as i128 };
                i64::try_from(signed)
                    .map(Self)
                    .map_err(|_| BillingError::MonetaryOverflow {
                        precision: P,
                        input_value: None,
                    })
            })
            .collect()
    }

    /// Round to the nearest multiple of `increment` — cash rounding.
    ///
    /// Several jurisdictions require the *payable* total to be rounded to a coarser
    /// step than the currency's minor unit, because the smallest coins were
    /// withdrawn: Switzerland rounds to 0.05 CHF (*Rappenrundung*), Sweden and
    /// Canada to their own 0.05 steps. Only the amount actually tendered is
    /// rounded — line items and the VAT breakdown keep full precision.
    ///
    /// See [`crate::CashRounding`] for the document-level helper that also records
    /// the rounding difference as its own line.
    ///
    /// ```rust
    /// use billing::{Amount, RoundingStrategy};
    /// let increment = Amount::<5>::parse("0.05000").unwrap();
    /// let total     = Amount::<5>::parse("12.34000").unwrap();
    /// assert_eq!(
    ///     total.round_to_increment(increment, RoundingStrategy::MidpointAwayFromZero).unwrap(),
    ///     Amount::<5>::parse("12.35000").unwrap()
    /// );
    /// ```
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `increment` is not strictly positive.
    pub fn round_to_increment(
        self,
        increment: Self,
        strategy: RoundingStrategy,
    ) -> Result<Self, BillingError> {
        if !increment.is_positive() {
            return Err(BillingError::InvalidInput {
                reason: format!("cash-rounding increment must be > 0, got {increment}"),
            });
        }
        // Exact integer arithmetic in i128: no Decimal, no float, no overflow.
        let value = self.0 as i128;
        let step = increment.0 as i128;
        let q = value.div_euclid(step);
        let r = value.rem_euclid(step); // always in [0, step)

        // `q` is the floor multiple and `r` the non-negative distance above it, so
        // every strategy below is expressed as "do we take the next step up?".
        let twice = r * 2;
        let round_up = match strategy {
            RoundingStrategy::Floor => false,
            RoundingStrategy::Ceiling => r != 0,
            RoundingStrategy::Truncate => {
                // Toward zero: for negatives the floor multiple is further from
                // zero, so truncation moves up; for positives it stays.
                value < 0 && r != 0
            }
            RoundingStrategy::MidpointAwayFromZero => {
                if value >= 0 {
                    twice >= step
                } else {
                    twice > step
                }
            }
            RoundingStrategy::MidpointToEven => match twice.cmp(&step) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Less => false,
                std::cmp::Ordering::Equal => q.rem_euclid(2) != 0,
            },
        };
        let multiple = if round_up { q + 1 } else { q };
        let scaled = multiple
            .checked_mul(step)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: None,
            })?;
        i64::try_from(scaled)
            .map(Self)
            .map_err(|_| BillingError::MonetaryOverflow {
                precision: P,
                input_value: None,
            })
    }

    /// Round to a different precision.
    ///
    /// # Panics
    /// Panics on overflow — see [`Amount::checked_round_to`] for a non-panicking version.
    /// Overflow can occur when converting to a **higher** precision (`Q > P`) for values
    /// near `Amount::<P>::MAX`.
    pub fn round_to<const Q: u8>(self, strategy: RoundingStrategy) -> Amount<Q> {
        self.checked_round_to(strategy)
            .expect("monetary overflow in round_to: use checked_round_to for large values")
    }

    /// Round to a different precision, returning `Err` on overflow.
    ///
    /// Overflow is only possible when converting to a **higher** precision (`Q > P`)
    /// for values near `Amount::<P>::MAX` / `Amount::<P>::MIN`.
    ///
    /// ```rust
    /// use billing::{Amount, RoundingStrategy};
    /// let a = Amount::<5>::parse("3.45678").unwrap();
    /// let r = a.checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero).unwrap();
    /// assert_eq!(r, Amount::<2>::parse("3.46").unwrap());
    /// ```
    pub fn checked_round_to<const Q: u8>(
        self,
        strategy: RoundingStrategy,
    ) -> Result<Amount<Q>, BillingError> {
        let d = self
            .into_decimal()
            .round_dp_with_strategy(Q as u32, strategy.into());
        Amount::<Q>::from_decimal(d).ok_or(BillingError::MonetaryOverflow {
            precision: Q,
            input_value: None,
        })
    }

    /// Construct from an integer (exact, no rounding).
    ///
    /// # Panics
    /// Panics if `n × 10^P` overflows `i64`. Use [`Amount::checked_from_int`] for a
    /// non-panicking version.
    ///
    /// # Example
    /// ```rust
    /// use billing::Amount;
    /// assert_eq!(Amount::<5>::from_int(49), Amount::parse("49.00000").unwrap());
    /// ```
    pub fn from_int(n: i64) -> Self {
        Self(
            n.checked_mul(Self::SCALE)
                .expect("monetary overflow in from_int: value × scale exceeds i64"),
        )
    }

    /// Fallible integer constructor — returns `Err` on overflow.
    ///
    /// `n` is treated as a whole-number monetary amount (e.g. `49` = 49.00000 at P=5).
    /// Returns `Err` if `n × 10^P` overflows `i64`.
    ///
    /// ```rust
    /// use billing::Amount;
    /// assert_eq!(Amount::<5>::checked_from_int(49).unwrap(), Amount::parse("49.00000").unwrap());
    /// assert!(Amount::<5>::checked_from_int(i64::MAX).is_err());
    /// ```
    pub fn checked_from_int(n: i64) -> Result<Self, crate::error::BillingError> {
        n.checked_mul(Self::SCALE)
            .map(Self)
            .ok_or(crate::error::BillingError::MonetaryOverflow {
                precision: P,
                input_value: None,
            })
    }

    /// Access the raw scaled `i64` representation.
    ///
    /// The raw value equals `display_value × 10^P`.
    /// Prefer the named accessors ([`Amount::is_positive`] etc.) over raw arithmetic.
    #[must_use]
    pub fn to_raw(self) -> i64 {
        self.0
    }

    /// Construct from a raw scaled `i64` — the value is `n × 10⁻ᴾ`.
    ///
    /// Use when you already have an internal representation (e.g. deserialising
    /// a previously stored [`to_raw`](Amount::to_raw) value, or constructing
    /// test fixtures that need exact raw values).
    ///
    /// # Example
    /// ```rust
    /// use billing::Amount;
    /// // 3_456 raw units = 0.03456 EUR at P=5
    /// let price = Amount::<5>::from_raw_units(3_456);
    /// assert_eq!(price, Amount::parse("0.03456").unwrap());
    /// ```
    pub fn from_raw_units(n: i64) -> Self {
        Self(n)
    }

    /// Returns `true` if the amount is strictly positive.
    #[must_use]
    pub fn is_positive(self) -> bool {
        self.0 > 0
    }

    /// Returns `true` if the amount is negative.
    #[must_use]
    pub fn is_negative(self) -> bool {
        self.0 < 0
    }

    /// Returns `true` if the amount is zero.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == 0
    }

    /// Returns the sign of the amount as `-1`, `0`, or `1`.
    ///
    /// Useful for conditional logic and multiplying by direction:
    /// ```rust
    /// use billing::Amount;
    /// let a = Amount::<5>::parse("-3.50000").unwrap();
    /// assert_eq!(a.signum(), -1);
    /// assert_eq!(Amount::<5>::ZERO.signum(), 0);
    /// assert_eq!(Amount::<5>::parse("1.00000").unwrap().signum(), 1);
    /// ```
    #[must_use]
    pub fn signum(self) -> i8 {
        self.0.signum() as i8
    }

    /// Absolute value.
    ///
    /// # Panics
    /// Panics if `self` equals `Amount(i64::MIN)` (the minimum value has no
    /// positive counterpart in `i64`). Use [`Amount::checked_abs`] for a
    /// non-panicking version.
    pub fn abs(self) -> Self {
        Self(
            self.0
                .checked_abs()
                .expect("monetary overflow in abs: i64::MIN has no positive counterpart"),
        )
    }

    /// Fallible absolute value. Returns `Err` if `self == Amount(i64::MIN)`.
    ///
    /// Use this instead of [`Amount::abs`] when the input is externally bounded
    /// and cannot be guaranteed to be above `Amount::MIN`.
    pub fn checked_abs(self) -> Result<Self, BillingError> {
        self.0
            .checked_abs()
            .map(Self)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: Some(self.into_decimal()),
            })
    }

    /// Returns `true` when `|self − expected| × 1_000_000 ≤ |expected| × ppm`.
    ///
    /// All arithmetic is exact integer (`u128`) — **no `f64`, no `Decimal`, no `.abs()` panic**.
    /// This also avoids the `i64::MIN` edge-case that would otherwise cause `.abs()` to panic
    /// when `self.0 - expected.0 == i64::MIN`.
    /// `ppm = 0` means exact equality; `ppm = 10_000` means within 1 %.
    ///
    /// When `expected` is zero the comparison degrades to an exact equality test
    /// (returns `true` only when `self` is also zero).
    ///
    /// | `ppm`       | Meaning |
    /// |-------------|---------|
    /// | `1_000`     | 0.1 %   |
    /// | `10_000`    | 1 %     |
    /// | `20_000`    | 2 %     |
    /// | `1_000_000` | 100 % (always true unless expected is zero) |
    ///
    /// # Example
    /// ```rust
    /// use billing::Amount;
    /// let stated   = Amount::<5>::parse("100.00000").unwrap();
    /// let computed = Amount::<5>::parse("100.50000").unwrap();
    /// // |100.0 − 100.5| / 100.5 ≈ 0.4975 % ≈ 4_975 ppm — within 10_000 ppm (1 %)
    /// assert!(stated.within_tolerance_ppm(computed, 10_000).unwrap());
    /// // 0.5 % exceeds a 4_000 ppm (0.4 %) window
    /// assert!(!stated.within_tolerance_ppm(computed, 4_000).unwrap());
    /// // Exact equality
    /// assert!(stated.within_tolerance_ppm(stated, 0).unwrap());
    /// ```
    ///
    /// # Errors
    /// Returns `Err` only when `self.checked_sub(expected)` overflows — requires
    /// values near the extremes of `Amount::MAX` / `Amount::MIN`.
    pub fn within_tolerance_ppm(self, expected: Self, ppm: u32) -> Result<bool, BillingError> {
        if expected.is_zero() {
            return Ok(self.is_zero());
        }
        let diff = self.checked_sub(expected)?;
        // Compare |diff| × 1_000_000 ≤ |expected| × ppm using u128 to:
        //   • avoid any multiplication overflow (u128 is wide enough for all i64 values)
        //   • avoid the i64::MIN-abs() panic (unsigned_abs() is infallible for all i64)
        //   • eliminate Decimal / f64 entirely
        let lhs = (diff.0.unsigned_abs() as u128) * 1_000_000_u128;
        let rhs = (expected.0.unsigned_abs() as u128) * (ppm as u128);
        Ok(lhs <= rhs)
    }
}

// ── serde ─────────────────────────────────────────────────────────────────────
//
// `Amount<P>` is serialised as a **decimal string** with exactly `P` fractional
// digits (`"0.03456"`), not as its raw scaled `i64`.
//
// The derived tuple-struct impl would emit the raw integer (`3456`), which is
// wrong in three ways for a monetary type:
//   • it is meaningless without knowing P out-of-band — `3456` is 0.03456 at
//     P=5 and 34.56 at P=2, so a change of precision silently rescales every
//     stored value by 10^ΔP;
//   • it does not interoperate with any invoice interchange format (BO4E, UBL,
//     EDIFACT, JSON APIs) — all of which carry money as decimal text;
//   • JSON numbers invite float round-tripping, which is exactly what a
//     fixed-point monetary type exists to prevent.
//
// Deserialisation accepts strings only, and goes through `Amount::parse`, so
// excess non-zero precision is rejected rather than silently truncated.

#[cfg(feature = "serde")]
impl<const P: u8> serde::Serialize for Amount<P> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[cfg(feature = "serde")]
impl<'de, const P: u8> serde::Deserialize<'de> for Amount<P> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V<const P: u8>;
        impl<const P: u8> serde::de::Visitor<'_> for V<P> {
            type Value = Amount<P>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "a decimal string with at most {P} fractional digits")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Amount::<P>::parse(v).map_err(serde::de::Error::custom)
            }
        }
        d.deserialize_str(V::<P>)
    }
}

impl<const P: u8> fmt::Debug for Amount<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Amount<{P}>({:.prec$})",
            self.into_decimal(),
            prec = P as usize
        )
    }
}

impl<const P: u8> fmt::Display for Amount<P> {
    /// Renders with exactly `P` decimal places, honouring the formatter's
    /// **width, fill and alignment** — so `{:>12}` and `{:*^14}` align invoice
    /// columns as expected.
    ///
    /// Like the primitive integer types, and unlike `str`, an `Amount` defaults to
    /// **right** alignment: numbers line up on their decimal point in a column.
    ///
    /// The formatter's *precision* (`{:.2}`) is deliberately ignored. The number of
    /// decimals is part of the type, and honouring `{:.2}` would round without an
    /// explicit [`RoundingStrategy`] — the one thing this crate never does
    /// implicitly. Use [`Amount::round_to`] to change precision.
    ///
    /// ```rust
    /// use billing::Amount;
    /// let a = Amount::<5>::parse("4.00000").unwrap();
    /// assert_eq!(format!("[{a:>12}]"), "[     4.00000]");
    /// assert_eq!(format!("[{a:<12}]"), "[4.00000     ]");
    /// assert_eq!(format!("[{a:*^13}]"), "[***4.00000***]");
    /// assert_eq!(format!("[{a}]"), "[4.00000]");
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let body = format!("{:.prec$}", self.into_decimal(), prec = P as usize);
        pad_numeric(f, &body)
    }
}

/// Pad `body` per the formatter's width, fill and alignment, defaulting to right
/// alignment as the numeric primitives do.
///
/// `Formatter::pad` is not usable here: it defaults to *left* alignment and it
/// truncates to the formatter's precision, which would mangle a number into
/// something like `"4."`. Writing through `write!` with an inline precision — what
/// this impl used to do — silently discards width, fill and alignment altogether,
/// so `{:>12}` was a no-op and invoice columns never lined up.
fn pad_numeric(f: &mut fmt::Formatter<'_>, body: &str) -> fmt::Result {
    let width = match f.width() {
        Some(w) => w,
        None => return f.write_str(body),
    };
    let len = body.chars().count();
    if len >= width {
        return f.write_str(body);
    }
    let padding = width - len;
    let fill = f.fill();
    let (before, after) = match f.align() {
        Some(fmt::Alignment::Left) => (0, padding),
        Some(fmt::Alignment::Center) => (padding / 2, padding - padding / 2),
        // Numbers default to right alignment, matching i64/f64.
        Some(fmt::Alignment::Right) | None => (padding, 0),
    };
    for _ in 0..before {
        f.write_char(fill)?;
    }
    f.write_str(body)?;
    for _ in 0..after {
        f.write_char(fill)?;
    }
    Ok(())
}

impl<const P: u8> std::ops::Neg for Amount<P> {
    type Output = Self;
    /// # Panics
    /// Panics if `self == Amount(i64::MIN)` (no positive counterpart).
    fn neg(self) -> Self {
        Self(self.0.checked_neg().expect("monetary overflow in negation"))
    }
}

impl<const P: u8> std::ops::Add for Amount<P> {
    type Output = Self;
    /// # Panics
    /// Panics on overflow. Use [`Amount::checked_add`] for fallible addition.
    fn add(self, rhs: Self) -> Self {
        Self(
            self.0
                .checked_add(rhs.0)
                .expect("monetary overflow in addition"),
        )
    }
}

impl<const P: u8> std::ops::Sub for Amount<P> {
    type Output = Self;
    /// # Panics
    /// Panics on overflow. Use [`Amount::checked_sub`] for fallible subtraction.
    fn sub(self, rhs: Self) -> Self {
        Self(
            self.0
                .checked_sub(rhs.0)
                .expect("monetary overflow in subtraction"),
        )
    }
}

impl<const P: u8> std::ops::AddAssign for Amount<P> {
    /// # Panics
    /// Panics on overflow. Use [`Amount::checked_add`] for fallible addition.
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs;
    }
}

impl<const P: u8> std::ops::SubAssign for Amount<P> {
    /// # Panics
    /// Panics on overflow. Use [`Amount::checked_sub`] for fallible subtraction.
    fn sub_assign(&mut self, rhs: Self) {
        *self = *self - rhs;
    }
}

impl<const P: u8> std::iter::Sum for Amount<P> {
    /// # Panics
    /// Panics if the running total overflows `i64`. Use [`Amount::checked_sum`]
    /// for fallible accumulation in production code paths.
    fn sum<I: Iterator<Item = Self>>(iter: I) -> Self {
        iter.fold(Self::ZERO, |a, b| a + b)
    }
}

impl<const P: u8> Amount<P> {
    /// Fallible sum of an iterator — returns `Err` on overflow instead of
    /// panicking. Prefer this over `.sum()` in any code path that could receive
    /// attacker-controlled or unbounded values.
    ///
    /// # Example
    /// ```rust
    /// use billing::{Amount, BillingError};
    /// let amounts = vec![
    ///     Amount::<5>::parse("1.00000").unwrap(),
    ///     Amount::<5>::parse("2.00000").unwrap(),
    /// ];
    /// let total = Amount::checked_sum(amounts.into_iter()).unwrap();
    /// assert_eq!(total, Amount::<5>::parse("3.00000").unwrap());
    /// ```
    pub fn checked_sum<I: Iterator<Item = Self>>(mut iter: I) -> Result<Self, BillingError> {
        iter.try_fold(Self::ZERO, |acc, x| acc.checked_add(x))
    }
}

impl<const P: u8> FromStr for Amount<P> {
    type Err = ParseAmountError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl<const P: u8> TryFrom<Decimal> for Amount<P> {
    type Error = ParseAmountError;
    fn try_from(d: Decimal) -> Result<Self, Self::Error> {
        Self::try_from_decimal(d)
    }
}

/// Lossless conversion from `Amount<P>` to `Decimal`.
///
/// This is the exact inverse of [`Amount::from_decimal`] for values in range.
/// Prefer `Decimal::from(amount)` over `amount.into_decimal()` in generic code.
impl<const P: u8> From<Amount<P>> for Decimal {
    fn from(a: Amount<P>) -> Self {
        a.into_decimal()
    }
}

/// Convert a raw `i64` integer into `Amount<P>` (fallible).
///
/// Treats `n` as a **whole-number monetary amount** and multiplies by `10^P`.
/// Returns `Err` if `n × 10^P` overflows `i64`.
///
/// # ⚠️ Not the inverse of `to_raw()`
///
/// [`Amount::to_raw`] returns the *scaled* internal integer.
/// `TryFrom<i64>` goes the other way — it treats `n` as whole units:
///
/// ```rust
/// use billing::Amount;
/// let a = Amount::<5>::parse("0.03456").unwrap();
/// let raw = a.to_raw();                          // 3_456  (scaled)
/// let wrong = Amount::<5>::try_from(raw);        // = 3456.00000  ← WRONG
/// let right = Amount::<5>::from_raw_units(raw);  // = 0.03456     ← correct
/// ```
///
/// Use [`Amount::from_raw_units`] to reconstruct from a `to_raw()` value.
///
/// # Example
/// ```rust
/// use billing::Amount;
/// // 49 whole units (e.g. 49 EUR stored as integer in a database)
/// let a = Amount::<5>::try_from(49i64).unwrap();
/// assert_eq!(a, Amount::parse("49.00000").unwrap());
/// ```
impl<const P: u8> TryFrom<i64> for Amount<P> {
    type Error = crate::error::ParseAmountError;
    fn try_from(n: i64) -> Result<Self, Self::Error> {
        n.checked_mul(Self::SCALE)
            .map(Self)
            .ok_or_else(|| crate::error::ParseAmountError {
                input: n.to_string(),
            })
    }
}

impl<const P: u8> Default for Amount<P> {
    fn default() -> Self {
        Self::ZERO
    }
}

/// 5 decimal places — high-precision monetary amounts.
pub type EuroAmount = Amount<5>;
/// Standard invoice precision: 2 decimal places (e.g. `49.99`).
pub type InvoiceAmt = Amount<2>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_display() {
        let a = Amount::<5>::parse("0.03456").unwrap();
        assert_eq!(a.to_raw(), 3_456);
        assert_eq!(a.to_string(), "0.03456");
    }

    #[test]
    fn parse_error_empty() {
        assert!(Amount::<5>::parse("").is_err());
        assert!(Amount::<5>::parse("not-a-number").is_err());
    }

    #[test]
    fn parse_rejects_excess_non_zero_digits() {
        // 6th digit is non-zero → rejected
        assert!(Amount::<5>::parse("0.123456").is_err());
        assert!(Amount::<5>::parse("1.000011").is_err());
    }

    #[test]
    fn parse_accepts_trailing_zeros_beyond_p() {
        // Trailing zeros beyond P are OK (no information loss)
        assert!(Amount::<5>::parse("1.100000").is_ok());
        assert!(Amount::<5>::parse("49.990000").is_ok());
        assert_eq!(
            Amount::<2>::parse("49.990").unwrap(),
            Amount::<2>::parse("49.99").unwrap()
        );
    }

    #[test]
    fn from_str_trait() {
        let a: Amount<5> = "0.03456".parse().unwrap();
        assert_eq!(a.to_raw(), 3_456);
    }

    #[test]
    fn try_from_decimal() {
        let d = Decimal::from_str_exact("0.03456").unwrap();
        let a = Amount::<5>::try_from(d).unwrap();
        assert_eq!(a.to_raw(), 3_456);
    }

    #[test]
    fn mul_qty_precision() {
        let price = Amount::<5>::parse("0.03456").unwrap();
        let qty = Decimal::from(100u32);
        let net = price.mul_qty(qty);
        assert_eq!(net, Amount::<5>::parse("3.45600").unwrap());
    }

    #[test]
    fn checked_mul_qty_overflow() {
        // i64::MAX / SCALE gives a price that would overflow when multiplied
        let max_price = Amount::<5>(i64::MAX / 2);
        assert!(max_price.checked_mul_qty(Decimal::from(3u32)).is_err());
    }

    #[test]
    fn checked_overflow() {
        let max = Amount::<5>(i64::MAX);
        assert!(max.checked_add(Amount::<5>::from_raw_units(1)).is_err());
    }

    #[test]
    fn round_to() {
        let a = Amount::<5>::parse("3.45678").unwrap();
        let r = a.round_to::<2>(RoundingStrategy::MidpointAwayFromZero);
        assert_eq!(r, Amount::<2>::parse("3.46").unwrap());
    }

    #[test]
    fn sum_iterator() {
        let items = vec![
            Amount::<5>::parse("1.00000").unwrap(),
            Amount::<5>::parse("2.00000").unwrap(),
            Amount::<5>::parse("3.00000").unwrap(),
        ];
        let total: Amount<5> = items.into_iter().sum();
        assert_eq!(total, Amount::<5>::parse("6.00000").unwrap());
    }

    #[test]
    fn from_int_correct() {
        assert_eq!(
            Amount::<5>::from_int(49),
            Amount::<5>::parse("49.00000").unwrap()
        );
    }

    #[test]
    #[should_panic(expected = "monetary overflow in from_int")]
    fn from_int_overflow_panics() {
        // For P=5, SCALE=100_000. i64::MAX / 100_000 = 92_233_720_368_547.
        // One more than that overflows.
        let _ = Amount::<5>::from_int(92_233_720_368_548);
    }

    #[test]
    #[should_panic(expected = "monetary overflow in abs")]
    fn abs_min_panics() {
        let _ = Amount::<5>(i64::MIN).abs();
    }

    #[test]
    fn abs_works() {
        assert_eq!(
            Amount::<5>::parse("-3.50000").unwrap().abs(),
            Amount::<5>::parse("3.50000").unwrap()
        );
        assert_eq!(Amount::<5>::ZERO.abs(), Amount::<5>::ZERO);
    }

    #[test]
    fn neg_panics_on_min() {
        let result = std::panic::catch_unwind(|| -Amount::<5>(i64::MIN));
        assert!(result.is_err());
    }
}
