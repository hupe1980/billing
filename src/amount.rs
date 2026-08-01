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

// ── AmountScale ───────────────────────────────────────────────────────────────

/// How many decimal places every monetary amount in a document may carry, and how
/// to get there.
///
/// # Why a document-wide policy rather than a rounding call per amount
///
/// Interchange formats cap the number of decimals on money. EN 16931 — and with it
/// XRechnung, Peppol BIS and ZUGFeRD — caps **every** monetary amount at two:
/// each invoice line net amount (BT-131, rule BR-DEC-23), the line sum (BT-106,
/// BR-DEC-09), the total without VAT (BT-109, BR-DEC-12), the VAT total (BT-110,
/// BR-DEC-13), the total with VAT (BT-112, BR-DEC-14), the paid amount (BT-113),
/// the rounding amount (BT-114), the amount due (BT-115), and each VAT category's
/// taxable base and tax (BT-116 / BT-117, BR-DEC-19 / BR-DEC-20).
///
/// At the same time the totals identities must still hold **exactly** at that
/// precision: BR-CO-10 (`BT-106 = Σ BT-131`), BR-CO-13, BR-CO-14
/// (`BT-110 = Σ BT-117`), BR-CO-15 (`BT-112 = BT-109 + BT-110`) and BR-CO-16.
///
/// Those two demands together are why rounding at the serialiser does not work.
/// Rounding each amount independently breaks the identities:
///
/// - three lines of `0.005` round to `0.01` each, summing to `0.03`, while the
///   exact sum `0.015` rounds to `0.02` — **BR-CO-10 violated**;
/// - a net of `0.0042` with 19 % VAT gives `0.00 + 0.00 ≠ 0.01` — **BR-CO-15
///   violated**.
///
/// The only construction that satisfies both is to round the **leaves** and then
/// recompute every aggregate from the rounded leaves — never to round an aggregate.
/// That is what [`crate::BillingDocumentBuilder::amount_scale`] does, and it has to
/// happen while the document is assembled, because by the time you hold a
/// [`crate::BillingDocument`] the aggregates are already computed.
///
/// ```rust
/// use billing::{AmountScale, RoundingStrategy};
/// // EN 16931's two decimals with German commercial rounding.
/// assert_eq!(AmountScale::EN16931.decimals(), 2);
/// // Or choose your own.
/// let s = AmountScale::new(3, RoundingStrategy::MidpointToEven).unwrap();
/// assert_eq!(s.decimals(), 3);
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "AmountScaleRepr"))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AmountScale {
    decimals: u8,
    strategy: RoundingStrategy,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct AmountScaleRepr {
    decimals: u8,
    strategy: RoundingStrategy,
}

/// Deserialisation goes through [`AmountScale::new`], so a stored configuration
/// cannot reintroduce a precision the constructor refuses.
#[cfg(feature = "serde")]
impl TryFrom<AmountScaleRepr> for AmountScale {
    type Error = BillingError;
    fn try_from(r: AmountScaleRepr) -> Result<Self, Self::Error> {
        Self::new(r.decimals, r.strategy)
    }
}

impl AmountScale {
    /// The precision EN 16931 mandates: **two decimals**, rounded half away from
    /// zero — German commercial rounding.
    ///
    /// # Why the rounding mode is safe rather than mandated
    ///
    /// The CEN Schematron rounds with XPath `round()`, which is half-up *toward
    /// positive infinity* — not half away from zero. The two disagree on negative
    /// midpoints (`round(-200.5)` is `-200`; half-away gives `-201`), so "the mode
    /// every validator assumes" would be an overstatement. What makes the choice
    /// safe is narrower and checkable:
    ///
    /// - **BR-CO-17** and **BR-S-09** apply `abs()` to *both* sides before
    ///   rounding, so the rounded argument is never negative and the two modes
    ///   coincide there. Those same rules then compare with a **±1.00 tolerance**
    ///   (`abs(BT-117) - 1 < … and abs(BT-117) + 1 > …`), which swallows any
    ///   last-place disagreement anyway.
    /// - In the totals chain (**BR-CO-10** … **BR-CO-16**) `round()` is applied to
    ///   sums of values that already carry two decimals, where it is a no-op.
    ///
    /// So half away from zero is safe everywhere the standard actually rounds, and
    /// it is what German commercial practice expects. Choose a different mode with
    /// [`AmountScale::new`] if a jurisdiction requires one — the identities hold
    /// under any mode, because this crate rounds the leaves and recomputes the
    /// aggregates rather than rounding both.
    pub const EN16931: Self = Self {
        decimals: 2,
        strategy: RoundingStrategy::MidpointAwayFromZero,
    };

    /// A custom scale.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `decimals` exceeds the 5 places a
    /// [`crate::LineItem`] amount carries — a "scale" that cannot lose information
    /// is a no-op dressed up as a policy, and asking for it is a caller mistake
    /// worth reporting.
    pub fn new(decimals: u8, strategy: RoundingStrategy) -> Result<Self, BillingError> {
        if decimals > 5 {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "AmountScale decimals must be <= 5 (the precision of a LineItem amount), got {decimals}"
                ),
            });
        }
        Ok(Self { decimals, strategy })
    }

    /// The number of decimal places amounts are reduced to.
    #[must_use]
    pub fn decimals(self) -> u8 {
        self.decimals
    }

    /// The rounding strategy applied when reducing.
    #[must_use]
    pub fn strategy(self) -> RoundingStrategy {
        self.strategy
    }

    /// Apply this scale to one amount.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if rounding up leaves the representable range.
    pub fn apply(self, amount: Amount<5>) -> Result<Amount<5>, BillingError> {
        amount.round_to_scale(self.decimals, self.strategy)
    }

    /// Reduce an exact `Decimal` to this scale in a **single** rounding.
    ///
    /// Use this wherever the exact value is still available, rather than rounding a
    /// value that has already been rounded once. Rounding twice is not the same
    /// operation and can land a whole minor unit away: `0.004999` rounded to five
    /// decimals half-away-from-zero is `0.005`, which then rounds to `0.01`, while
    /// rounding `0.004999` straight to two decimals gives `0.00`.
    ///
    /// EN 16931 makes this the required behaviour rather than a preference —
    /// **BR-CO-17** defines the VAT category tax amount as
    /// `BT-117 = BT-116 × (BT-119 / 100)` *rounded to two decimals*, a single
    /// rounding of the exact product. A validator recomputes it exactly that way.
    ///
    /// ```rust
    /// use billing::{AmountScale, Amount, RoundingStrategy};
    /// use rust_decimal::dec;
    ///
    /// let scale = AmountScale::EN16931;
    /// // One rounding of the exact value:
    /// assert_eq!(scale.apply_decimal(dec!(0.004999)).unwrap(), Amount::<5>::ZERO);
    /// // Two roundings would have produced 0.01 — the same input, a different answer.
    /// assert_eq!(
    ///     scale.apply(Amount::<5>::from_decimal_rounded(
    ///         dec!(0.004999), RoundingStrategy::MidpointAwayFromZero).unwrap()).unwrap(),
    ///     Amount::<5>::parse("0.01000").unwrap(),
    /// );
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if the rounded value leaves the
    /// representable range of [`Amount<5>`].
    pub fn apply_decimal(self, value: Decimal) -> Result<Amount<5>, BillingError> {
        // Round once, at the target scale, from the exact input …
        let rounded = value.round_dp_with_strategy(self.decimals as u32, self.strategy.into());
        // … then convert exactly: the value now has at most `decimals` places, so
        // this cannot round again and cannot lose precision.
        Amount::<5>::checked_from_decimal(rounded)
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
/// # Representable range
///
/// The backing integer is **`i64`** — 8 bytes, `Copy`, no allocation — so the
/// representable magnitude is `i64::MAX × 10⁻ᴾ`, available as [`Amount::MAX`] and
/// [`Amount::MIN`] for any `P`. Precision is bought with range, one decimal digit
/// at a time:
///
/// | `P` | Smallest step | [`Amount::MAX`] |
/// |-----|---------------|-----------------|
/// | 2   | 0.01          | 92 233 720 368 547 758.07 (≈ 9.2 × 10¹⁶) |
/// | 4   | 0.0001        | 922 337 203 685 477.5807 (≈ 9.2 × 10¹⁴) |
/// | 5   | 0.00001       | 92 233 720 368 547.75807 (≈ 9.2 × 10¹³) |
/// | 6   | 0.000001      | 9 223 372 036 854.775807 (≈ 9.2 × 10¹²) |
/// | 9   | 0.000000001   | 9 223 372 036.854775807 (≈ 9.2 × 10⁹) |
/// | 18  | 10⁻¹⁸         | 9.223372036854775807 |
///
/// `P > 18` is a **compile-time** error: `10¹⁹` exceeds `i64::MAX`.
///
/// `EuroAmount` (`P = 5`) therefore tops out around **92 trillion** currency
/// units. That is far above any single invoice and above any realistic portfolio
/// aggregate, which is why the backing type is `i64` rather than `i128`: doubling
/// the width of every amount in every document buys headroom nobody needs. If you
/// do aggregate past it, sum at a coarser precision — `Amount<2>` reaches 9.2 × 10¹⁶
/// — rather than reaching for a wider integer.
///
/// # Overflow semantics
///
/// Overflow is never silent, and never wraps or saturates. Each operation is
/// either infallible-and-panicking or fallible-and-total:
///
/// | Operation | On overflow |
/// |-----------|-------------|
/// | `+`, `-`, `+=`, `-=`, `-` (neg), [`sum`](std::iter::Sum) | **panics** |
/// | [`from_int`](Amount::from_int), [`abs`](Amount::abs), [`mul_qty`](Amount::mul_qty), [`round_to`](Amount::round_to) | **panics** |
/// | every `checked_*` method, [`from_decimal_rounded`](Amount::from_decimal_rounded), [`distribute`](Amount::distribute), [`allocate`](Amount::allocate), [`round_to_increment`](Amount::round_to_increment) | [`BillingError::MonetaryOverflow`] |
///
/// The `checked_*` family is **total**: it returns `Err` and never panics, even
/// where the underlying `rust_decimal` operators would (`Decimal`'s `Mul`, `Div`
/// and `Sum` panic on overflow rather than saturating). To refuse an
/// out-of-range total at a domain boundary, convert through
/// [`Amount::checked_from_decimal`] and map the error:
///
/// ```rust
/// use billing::{Amount, BillingError};
/// use rust_decimal::Decimal;
///
/// fn ensure_representable(total: Decimal) -> Result<Amount<5>, BillingError> {
///     Amount::<5>::checked_from_decimal(total) // Err rather than a truncated total
/// }
/// assert!(ensure_representable(Decimal::MAX).is_err());
/// ```
///
/// # Parsing
///
/// [`Amount::parse`] accepts `"."` and `","` as decimal separators.
/// It rejects strings that carry **more non-zero digits than P**:
/// `Amount::<5>::parse("1.000011")` → `Err` (the 6th digit `1` cannot be
/// represented without loss).  Trailing zeros beyond P are accepted.
///
/// [`Amount::checked_from_decimal`] applies the same rule to a `Decimal`, so the
/// two conversion paths agree: a value refused as text is refused as a `Decimal`.
/// [`Amount::from_decimal_rounded`] is the opt-in that rounds instead, and it
/// requires you to name the [`RoundingStrategy`].
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

    /// Scale by `10^P` and round the product to an integer, returning `None` if it
    /// does not fit `i64`.
    ///
    /// The rounding here is a no-op for any caller that has already reduced `d` to
    /// at most `P` fractional digits — which is what both public constructors
    /// guarantee before calling in. It exists only so a scaled product carrying
    /// residual `Decimal` representation noise lands on an integer rather than
    /// truncating toward zero.
    ///
    /// `Decimal`'s `Mul` impl **panics** on overflow rather than saturating, so the
    /// checked form is mandatory: every public path through here is documented as
    /// returning `Err`, never as panicking.
    fn scaled_to_raw(d: Decimal) -> Option<i64> {
        d.checked_mul(Decimal::from(Self::SCALE))?
            .round_dp_with_strategy(0, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
            .to_i64()
    }

    /// Whether `d` has more non-zero fractional digits than `P` can represent.
    ///
    /// `normalize()` strips trailing zeros first, so `1.230000` at `P = 2` is
    /// representable while `0.123456` at `P = 5` is not — the same rule
    /// [`Amount::parse`] applies to strings.
    fn exceeds_precision(d: Decimal) -> bool {
        d.normalize().scale() > P as u32
    }

    /// Construct from a `rust_decimal::Decimal` — **exact**, never rounding.
    ///
    /// Returns `Err` when the value cannot be represented at precision `P`:
    ///
    /// - [`BillingError::PrecisionLoss`] if `d` carries **non-zero digits beyond
    ///   `P`** decimal places (trailing zeros are fine), and
    /// - [`BillingError::MonetaryOverflow`] if the scaled value exceeds `i64`.
    ///
    /// # Why exact, and not silently rounded
    ///
    /// This is the `Decimal` counterpart of [`Amount::parse`], and the two now
    /// agree: `parse("0.123456")` and `checked_from_decimal(dec!(0.123456))` both
    /// fail at `P = 5`. Earlier releases rounded this conversion while `parse`
    /// rejected it, so the same unrepresentable price was refused when it arrived
    /// as text and quietly altered when it arrived as a `Decimal`.
    ///
    /// Rounding is never implicit in this crate. To round, name the strategy with
    /// [`Amount::from_decimal_rounded`].
    ///
    /// ```rust
    /// use billing::{Amount, BillingError};
    /// use rust_decimal::{Decimal, dec};
    ///
    /// // Exactly representable at P = 5.
    /// assert_eq!(
    ///     Amount::<5>::checked_from_decimal(dec!(1.23456)).unwrap(),
    ///     Amount::parse("1.23456").unwrap()
    /// );
    /// // Trailing zeros carry no information — accepted.
    /// assert!(Amount::<5>::checked_from_decimal(dec!(1.2340000)).is_ok());
    /// // A sixth non-zero digit is a refusal, not a rounding.
    /// assert!(matches!(
    ///     Amount::<5>::checked_from_decimal(dec!(0.123456)),
    ///     Err(BillingError::PrecisionLoss { .. })
    /// ));
    /// // Never panics, even at the extremes of Decimal's range.
    /// assert!(Amount::<5>::checked_from_decimal(Decimal::MAX).is_err());
    /// assert!(Amount::<5>::checked_from_decimal(Decimal::MIN).is_err());
    /// ```
    pub fn checked_from_decimal(d: Decimal) -> Result<Self, BillingError> {
        if Self::exceeds_precision(d) {
            return Err(BillingError::PrecisionLoss {
                precision: P,
                input_value: d,
            });
        }
        Self::scaled_to_raw(d)
            .map(Self)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: Some(d),
            })
    }

    /// Construct from a `rust_decimal::Decimal`, rounding to `P` decimal places
    /// with an **explicitly named** strategy.
    ///
    /// The rounding counterpart of [`Amount::checked_from_decimal`]: use this when
    /// the input legitimately carries more precision than the target and you have
    /// decided how to discard it.
    ///
    /// Returns [`BillingError::MonetaryOverflow`] if the rounded value exceeds the
    /// representable range — see [Representable range](#representable-range).
    /// Never panics, including at the extremes of `Decimal`'s range.
    ///
    /// ```rust
    /// use billing::{Amount, RoundingStrategy};
    /// use rust_decimal::dec;
    ///
    /// let d = dec!(0.123456);
    /// assert_eq!(
    ///     Amount::<5>::from_decimal_rounded(d, RoundingStrategy::MidpointAwayFromZero).unwrap(),
    ///     Amount::parse("0.12346").unwrap()
    /// );
    /// assert_eq!(
    ///     Amount::<5>::from_decimal_rounded(d, RoundingStrategy::Truncate).unwrap(),
    ///     Amount::parse("0.12345").unwrap()
    /// );
    /// ```
    pub fn from_decimal_rounded(
        d: Decimal,
        strategy: RoundingStrategy,
    ) -> Result<Self, BillingError> {
        let rounded = d.round_dp_with_strategy(P as u32, strategy.into());
        Self::scaled_to_raw(rounded)
            .map(Self)
            .ok_or(BillingError::MonetaryOverflow {
                precision: P,
                input_value: Some(d),
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
        let product =
            self.into_decimal()
                .checked_mul(qty)
                .ok_or(BillingError::MonetaryOverflow {
                    precision: P,
                    input_value: None,
                })?;
        Self::from_decimal_rounded(product, RoundingStrategy::MidpointAwayFromZero)
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
            })?;
        Self::from_decimal_rounded(q, RoundingStrategy::MidpointAwayFromZero)
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

    /// Round to at most `scale` decimal places, **keeping the type**.
    ///
    /// Unlike [`Amount::checked_round_to`], which changes `P`, this leaves the value
    /// an `Amount<P>` whose trailing `P − scale` digits are zero. That is what an
    /// interchange format usually wants: EN 16931 caps every monetary amount at two
    /// decimals (rules BR-DEC-01 … BR-DEC-28) while the surrounding arithmetic still
    /// happens at the engine's own precision, so the amount must *become* a
    /// two-decimal value without the rest of the document changing type.
    ///
    /// Exact integer arithmetic — this is [`Amount::round_to_increment`] with an
    /// increment of `10^(P − scale)`, so all five strategies behave identically to
    /// cash rounding, including for negative values.
    ///
    /// `scale >= P` is a no-op: the value already fits.
    ///
    /// ```rust
    /// use billing::{Amount, RoundingStrategy};
    /// let a = Amount::<5>::parse("356.80221").unwrap();
    /// assert_eq!(
    ///     a.round_to_scale(2, RoundingStrategy::MidpointAwayFromZero).unwrap(),
    ///     Amount::<5>::parse("356.80000").unwrap()
    /// );
    /// // Already within scale — unchanged.
    /// let b = Amount::<5>::parse("10.50000").unwrap();
    /// assert_eq!(b.round_to_scale(2, RoundingStrategy::Floor).unwrap(), b);
    /// ```
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if rounding up would leave the
    /// representable range.
    pub fn round_to_scale(
        self,
        scale: u8,
        strategy: RoundingStrategy,
    ) -> Result<Self, BillingError> {
        match Self::scale_increment(scale) {
            None => Ok(self),
            Some(increment) => self.round_to_increment(Self(increment), strategy),
        }
    }

    /// `10^(P − scale)` as a raw increment, or `None` when `scale` already covers
    /// every digit `P` carries and the operation is a no-op.
    ///
    /// Cannot overflow: `P ≤ 18` is a compile-time guarantee, so the widest result
    /// is `10^18`, which fits `i64`.
    fn scale_increment(scale: u8) -> Option<i64> {
        if scale >= P {
            return None;
        }
        let mut increment: i64 = 1;
        for _ in 0..(P - scale) {
            increment *= 10;
        }
        Some(increment)
    }

    /// Whether this value fits in `scale` decimal places without rounding.
    ///
    /// The predicate behind an interchange-format precision check: EN 16931's
    /// BR-DEC rules cap monetary amounts at two decimals, and an amount that fails
    /// this must be rounded ([`Amount::round_to_scale`]) before it is emitted, not
    /// truncated by the serialiser.
    ///
    /// ```rust
    /// use billing::Amount;
    /// assert!(!Amount::<5>::parse("356.80221").unwrap().fits_scale(2));
    /// assert!(Amount::<5>::parse("356.80000").unwrap().fits_scale(2));
    /// assert!(Amount::<5>::ZERO.fits_scale(0));
    /// ```
    #[must_use]
    pub fn fits_scale(self, scale: u8) -> bool {
        match Self::scale_increment(scale) {
            None => true,
            Some(increment) => self.0 % increment == 0,
        }
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
        Amount::<Q>::from_decimal_rounded(self.into_decimal(), strategy)
    }

    /// Convert to `Q` decimals **without rounding**, or fail.
    ///
    /// The counterpart to [`Amount::checked_round_to`] for the case where losing
    /// precision is a bug rather than a policy — narrowing a document's amounts on
    /// the way into an interchange format, for instance. Rounding *there* is
    /// precisely the mistake [`AmountScale`] exists to prevent: it rounds the
    /// leaves and the aggregates independently, which breaks the totals identities
    /// (BR-CO-10, BR-CO-15) that the same format also checks. The right response to
    /// an `Err` here is to rebuild with
    /// [`amount_scale`](crate::BillingDocumentBuilder::amount_scale), not to round.
    ///
    /// This completes the pair the `Decimal` conversions already have —
    /// [`checked_from_decimal`](Amount::checked_from_decimal) refuses excess
    /// precision, [`from_decimal_rounded`](Amount::from_decimal_rounded) opts into
    /// rounding — and extends it to precision-to-precision conversion, so no
    /// conversion path in this crate can silently lose money.
    ///
    /// Widening (`Q > P`) is always exact and can only fail on overflow.
    ///
    /// ```rust
    /// use billing::Amount;
    ///
    /// let exact = Amount::<5>::parse("356.80000").unwrap();
    /// assert_eq!(exact.exact_to::<2>().unwrap(), Amount::<2>::parse("356.80").unwrap());
    ///
    /// // 356.80221 does not fit two decimals — say so rather than round it away.
    /// let inexact = Amount::<5>::parse("356.80221").unwrap();
    /// assert!(inexact.exact_to::<2>().is_err());
    /// // `fits_scale` asks the same question without producing a value.
    /// assert!(!inexact.fits_scale(2));
    /// ```
    ///
    /// # Errors
    /// - [`BillingError::InvalidInput`] if the value does not fit `Q` decimals
    ///   exactly; ask first with [`Amount::fits_scale`].
    /// - [`BillingError::MonetaryOverflow`] if the value is outside `Amount<Q>`'s
    ///   representable range.
    pub fn exact_to<const Q: u8>(self) -> Result<Amount<Q>, BillingError> {
        if Q < P && !self.fits_scale(Q) {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "{self} does not fit {Q} decimal places exactly; \
                     rebuild the document at that scale rather than rounding here"
                ),
            });
        }
        Amount::<Q>::checked_from_decimal(self.into_decimal())
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
    /// # Total, and deliberately not fallible
    ///
    /// This comparison **cannot fail** — there is no error arm to handle and no
    /// `unwrap` to write. Earlier releases returned `Result`, because the
    /// difference was taken with `checked_sub` in `i64` and could overflow for
    /// operands at opposite extremes of the range. Every real call site collapsed
    /// that with `.unwrap_or(false)`, which silently reported *"outside
    /// tolerance"* for what was actually an internal arithmetic failure — turning
    /// a computation error into a spurious finding, in exactly the tolerance
    /// checks written to catch billing discrepancies.
    ///
    /// Widening the subtraction to `i128` removes the failure mode rather than
    /// reporting it: every `i64` difference fits, so the answer is always the
    /// mathematically correct one.
    ///
    /// # Example
    /// ```rust
    /// use billing::Amount;
    /// let stated   = Amount::<5>::parse("100.00000").unwrap();
    /// let computed = Amount::<5>::parse("100.50000").unwrap();
    /// // |100.0 − 100.5| / 100.5 ≈ 0.4975 % ≈ 4_975 ppm — within 10_000 ppm (1 %)
    /// assert!(stated.within_tolerance_ppm(computed, 10_000));
    /// // 0.5 % exceeds a 4_000 ppm (0.4 %) window
    /// assert!(!stated.within_tolerance_ppm(computed, 4_000));
    /// // Exact equality
    /// assert!(stated.within_tolerance_ppm(stated, 0));
    /// // Opposite extremes: a real answer, not an error to swallow.
    /// assert!(!Amount::<5>::MAX.within_tolerance_ppm(Amount::<5>::MIN, 1_000));
    /// ```
    #[must_use]
    pub fn within_tolerance_ppm(self, expected: Self, ppm: u32) -> bool {
        if expected.is_zero() {
            return self.is_zero();
        }
        // Compare |diff| × 1_000_000 ≤ |expected| × ppm in 128-bit integers:
        //   • the difference is taken in i128, so no pair of i64 operands can
        //     overflow it — this is what makes the method total;
        //   • unsigned_abs() is infallible for every i128, so there is no
        //     i64::MIN-style .abs() panic;
        //   • both products are bounded by ~2^64 × 2^32, far inside u128;
        //   • no Decimal, no f64 — exact integer arithmetic throughout.
        let diff = (self.0 as i128) - (expected.0 as i128);
        let lhs = diff.unsigned_abs() * 1_000_000_u128;
        let rhs = (expected.0 as i128).unsigned_abs() * (ppm as u128);
        lhs <= rhs
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

/// Exact conversion — equivalent to [`Amount::checked_from_decimal`].
///
/// Fails with [`BillingError::PrecisionLoss`] rather than rounding; use
/// [`Amount::from_decimal_rounded`] when rounding is intended.
impl<const P: u8> TryFrom<Decimal> for Amount<P> {
    type Error = BillingError;
    fn try_from(d: Decimal) -> Result<Self, Self::Error> {
        Self::checked_from_decimal(d)
    }
}

/// Lossless conversion from `Amount<P>` to `Decimal`.
///
/// This is the exact inverse of [`Amount::checked_from_decimal`] for values in range.
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
    type Error = BillingError;
    fn try_from(n: i64) -> Result<Self, Self::Error> {
        Self::checked_from_int(n)
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
    fn try_from_decimal_is_the_exact_conversion() {
        let d = Decimal::from_str_exact("0.03456").unwrap();
        let a = Amount::<5>::try_from(d).unwrap();
        assert_eq!(a.to_raw(), 3_456);
        // Exact, so excess precision is refused rather than rounded away.
        let too_precise = Decimal::from_str_exact("0.034561").unwrap();
        assert!(matches!(
            Amount::<5>::try_from(too_precise),
            Err(BillingError::PrecisionLoss { .. })
        ));
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

    #[test]
    fn debug_shows_the_precision_and_the_full_value() {
        // `Debug` is what every failing `assert_eq!` in this crate and in every
        // downstream test suite prints. It has to carry `P`, because two amounts
        // that compare unequal often differ only in the scale they were built at,
        // and it has to keep the trailing zeros, because that is where the scale
        // is visible at all.
        assert_eq!(
            format!("{:?}", Amount::<5>::parse("1.50000").unwrap()),
            "Amount<5>(1.50000)"
        );
        assert_eq!(
            format!("{:?}", Amount::<2>::parse("1.50").unwrap()),
            "Amount<2>(1.50)"
        );
        assert_eq!(format!("{:?}", Amount::<5>::ZERO), "Amount<5>(0.00000)");
        assert_eq!(
            format!("{:?}", Amount::<5>::parse("-0.00001").unwrap()),
            "Amount<5>(-0.00001)"
        );
        // `Amount<0>` has no fractional part and no trailing dot.
        assert_eq!(
            format!("{:?}", Amount::<0>::parse("42").unwrap()),
            "Amount<0>(42)"
        );
    }

    #[cfg(feature = "serde")]
    #[test]
    fn deserialising_a_non_string_says_what_it_wanted_instead() {
        // `Amount` is serialised as a decimal string precisely so a JSON number
        // can never round-trip through an f64. When a payload carries one anyway,
        // the error has to name the alternative — otherwise "invalid type:
        // integer" leaves the author of the payload with no idea what to write.
        let err = serde_json::from_str::<Amount<5>>("42")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("a decimal string with at most 5 fractional digits"),
            "{err}"
        );

        let err = serde_json::from_str::<Amount<2>>("1.5")
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("a decimal string with at most 2 fractional digits"),
            "{err}"
        );

        // A string with too many digits is rejected by `parse`, with its own
        // message rather than the visitor's.
        let err = serde_json::from_str::<Amount<2>>(r#""1.005""#)
            .unwrap_err()
            .to_string();
        assert!(err.contains("1.005"), "{err}");
    }
}
