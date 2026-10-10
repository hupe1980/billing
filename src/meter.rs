//! Metering as data: [`Sample`] readings, the [`Aggregation`] that reduces them
//! to a billable quantity, an optional [`UnitConversion`], the [`Meter`]
//! combining both, and the exact [`Usage`] they produce.
//!
//! Pure arithmetic: no ingestion, no storage, no clock. The caller supplies the
//! readings of one period **in chronological order** (so [`Aggregation::Latest`]
//! is the last one given) and, for time-weighted gauges, each reading's weight
//! (its share of the period). Every result is exact: a decimal, or for the two
//! averages the exact fraction, which [`Line::from_usage`](crate::Line::from_usage)
//! and [`Tariff::rate`](crate::Tariff::rate) price without rounding it first.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use rust_decimal::Decimal;

use crate::decimal::{exact_add, exact_mul, round_ratio};
use crate::{Error, ErrorKind, Location, Percentile, Result, Rounding, nearest_rank};

/// One metered reading: a value, a weight (default 1) and an optional key.
///
/// A plain input record: constructors never fail; the weight is checked (≥ 0)
/// by the weighted kinds and the key required by
/// [`UniqueCount`](Aggregation::UniqueCount), each refusal located at the
/// reading's 0-based position ([`Location::Sample`]).
///
/// ```
/// use billing::Sample;
/// use rust_decimal::dec;
///
/// let s = Sample::new(dec!(10)).with_weight(dec!(0.25)).with_key("vm-1");
/// assert_eq!((s.value(), s.weight(), s.key()), (dec!(10), dec!(0.25), Some("vm-1")));
/// assert_eq!(Sample::new(dec!(3)).weight(), dec!(1));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Sample {
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    value: Decimal,
    #[cfg_attr(
        feature = "serde",
        serde(
            default = "one",
            with = "crate::decimal::serde_str",
            skip_serializing_if = "is_one"
        )
    )]
    weight: Decimal,
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    key: Option<String>,
}

#[cfg(feature = "serde")]
fn one() -> Decimal {
    Decimal::ONE
}

#[cfg(feature = "serde")]
#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_one(d: &Decimal) -> bool {
    *d == Decimal::ONE && d.scale() == 0
}

impl Sample {
    /// A reading of `value`, weight 1, no key.
    #[must_use]
    pub fn new(value: Decimal) -> Self {
        Self {
            value,
            weight: Decimal::ONE,
            key: None,
        }
    }

    /// Set the weight (the reading's share of the period); used only by
    /// [`WeightedSum`](Aggregation::WeightedSum) and
    /// [`WeightedMean`](Aggregation::WeightedMean), which refuse a negative one.
    #[must_use]
    pub fn with_weight(mut self, weight: Decimal) -> Self {
        self.weight = weight;
        self
    }

    /// Set the key (an entity id); used only by
    /// [`UniqueCount`](Aggregation::UniqueCount).
    #[must_use]
    pub fn with_key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }

    /// The reading.
    #[must_use]
    pub fn value(&self) -> Decimal {
        self.value
    }

    /// The weight (1 unless set).
    #[must_use]
    pub fn weight(&self) -> Decimal {
        self.weight
    }

    /// The key, if set.
    #[must_use]
    pub fn key(&self) -> Option<&str> {
        self.key.as_deref()
    }
}

/// How readings reduce to a billable quantity: the kinds OpenMeter, Lago and
/// Stripe meters configure as data.
///
/// | kind | result | no readings |
/// |---|---|---|
/// | `Sum` | Σ value | 0 |
/// | `Count` | number of readings | 0 |
/// | `UniqueCount` | number of distinct keys | 0 |
/// | `Min` / `Max` | least / greatest value | none |
/// | `Latest` | the last reading's value | none |
/// | `Mean` | Σ value ÷ count, an exact fraction | none |
/// | `WeightedSum` | Σ value × weight | 0 |
/// | `WeightedMean` | Σ value × weight ÷ Σ weight, an exact fraction | none (also when Σ weight = 0) |
/// | `Percentile(p)` | nearest rank, as [`nearest_rank`] | none |
///
/// Keys matter only to `UniqueCount`, weights only to the two weighted kinds.
///
/// `#[non_exhaustive]`: later releases add kinds without a breaking change.
///
/// ```
/// use billing::{Aggregation, Percentile, Sample};
/// use rust_decimal::{Decimal, dec};
///
/// let readings = || [3, 9, 4, 7, 5].map(|v| Sample::new(Decimal::from(v)));
/// let value = |a: Aggregation| a.apply(readings()).map(|u| u.unwrap().to_decimal());
/// assert_eq!(value(Aggregation::Sum)?, dec!(28));
/// assert_eq!(value(Aggregation::Count)?, dec!(5));
/// assert_eq!(value(Aggregation::Min)?, dec!(3));
/// assert_eq!(value(Aggregation::Max)?, dec!(9));
/// assert_eq!(value(Aggregation::Latest)?, dec!(5));
/// assert_eq!(value(Aggregation::Percentile(Percentile::P95))?, dec!(9));
///
/// let mean = Aggregation::Mean.apply(readings())?.unwrap();
/// assert_eq!((mean.numerator(), mean.denominator()), (dec!(5.6), dec!(1))); // 28/5, canonical
/// assert_eq!(mean.to_decimal(), dec!(5.6));
///
/// // No readings: zero for the additive kinds, no value for the others.
/// assert_eq!(Aggregation::Sum.apply([])?.unwrap().to_decimal(), dec!(0));
/// assert_eq!(Aggregation::Max.apply([])?, None);
/// # Ok::<(), billing::Error>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Aggregation {
    /// Σ value.
    Sum,
    /// The number of readings.
    Count,
    /// The number of distinct keys; a reading without a key is refused.
    ///
    /// ```
    /// use billing::{Aggregation, ErrorKind, Location, Sample};
    /// use rust_decimal::dec;
    ///
    /// let keyed = ["a", "b", "a", "c"].map(|k| Sample::new(dec!(1)).with_key(k));
    /// assert_eq!(Aggregation::UniqueCount.apply(keyed)?.unwrap().to_decimal(), dec!(3));
    ///
    /// let err = Aggregation::UniqueCount
    ///     .apply([Sample::new(dec!(1)).with_key("a"), Sample::new(dec!(1)).with_key("b"), Sample::new(dec!(1))])
    ///     .unwrap_err();
    /// assert_eq!((err.kind(), err.location()), (&ErrorKind::MissingKey, Some(&Location::Sample(2))));
    /// # Ok::<(), billing::Error>(())
    /// ```
    UniqueCount,
    /// The least value.
    Min,
    /// The greatest value.
    Max,
    /// The last reading's value.
    Latest,
    /// Σ value ÷ count, as the exact fraction.
    Mean,
    /// Σ value × weight; a negative weight is refused.
    ///
    /// ```
    /// use billing::{Aggregation, ErrorKind, Location, Sample};
    /// use rust_decimal::dec;
    ///
    /// let gauge = [Sample::new(dec!(10)).with_weight(dec!(0.25)), Sample::new(dec!(20)).with_weight(dec!(0.75))];
    /// assert_eq!(Aggregation::WeightedSum.apply(gauge)?.unwrap().to_decimal(), dec!(17.5));
    ///
    /// let err = Aggregation::WeightedSum.apply([Sample::new(dec!(1)).with_weight(dec!(-1))]).unwrap_err();
    /// assert_eq!(err.kind(), &ErrorKind::NegativeWeight { weight: dec!(-1) });
    /// assert_eq!(err.location(), Some(&Location::Sample(0)));
    /// # Ok::<(), billing::Error>(())
    /// ```
    WeightedSum,
    /// Σ value × weight ÷ Σ weight, as the exact fraction; a negative weight is
    /// refused.
    ///
    /// ```
    /// use billing::{Aggregation, Sample};
    /// use rust_decimal::dec;
    ///
    /// let u = Aggregation::WeightedMean
    ///     .apply([Sample::new(dec!(10)), Sample::new(dec!(20)).with_weight(dec!(2))])?
    ///     .unwrap();
    /// assert_eq!((u.numerator(), u.denominator()), (dec!(50), dec!(3)));
    /// assert_eq!(u.to_decimal(), dec!(16.666666666666666666666666667));
    /// # Ok::<(), billing::Error>(())
    /// ```
    WeightedMean,
    /// The nearest-rank percentile ([`nearest_rank`]): always a measured value.
    Percentile(Percentile),
}

impl Aggregation {
    /// Reduce `samples` (in chronological order) to a usage; `None` where the
    /// kind has no value for them (see the table on [`Aggregation`]).
    ///
    /// # Errors
    /// [`ErrorKind::MissingKey`] (`UniqueCount`) and
    /// [`ErrorKind::NegativeWeight`] (weighted kinds) at the reading's
    /// [`Location::Sample`]; [`ErrorKind::Overflow`] there too when a running
    /// sum or product does not fit `Decimal` exactly.
    pub fn apply(&self, samples: impl IntoIterator<Item = Sample>) -> Result<Option<Usage>> {
        let overflow = |i: usize| Error::from(ErrorKind::Overflow).at(Location::Sample(i));
        let add = |acc: Decimal, x: Decimal, i: usize| exact_add(acc, x).ok_or_else(|| overflow(i));
        let weight = |s: &Sample, i: usize| -> Result<Decimal> {
            if s.weight.is_sign_negative() && !s.weight.is_zero() {
                return Err(Error::from(ErrorKind::NegativeWeight { weight: s.weight })
                    .at(Location::Sample(i)));
            }
            Ok(s.weight)
        };
        let mut n: usize = 0;
        let mut sum = Decimal::ZERO;
        let mut weights = Decimal::ZERO;
        let mut pick: Option<Decimal> = None;
        let mut keys: BTreeSet<String> = BTreeSet::new();
        let mut values: Vec<Decimal> = Vec::new();
        for (i, s) in samples.into_iter().enumerate() {
            n += 1;
            match self {
                Self::Sum | Self::Mean => sum = add(sum, s.value, i)?,
                Self::Count => {}
                Self::UniqueCount => {
                    let key = s.key.ok_or_else(|| {
                        Error::from(ErrorKind::MissingKey).at(Location::Sample(i))
                    })?;
                    keys.insert(key);
                }
                Self::Min => pick = Some(pick.map_or(s.value, |m| m.min(s.value))),
                Self::Max => pick = Some(pick.map_or(s.value, |m| m.max(s.value))),
                Self::Latest => pick = Some(s.value),
                Self::WeightedSum | Self::WeightedMean => {
                    let w = weight(&s, i)?;
                    let vw = exact_mul(s.value, w).ok_or_else(|| overflow(i))?;
                    sum = add(sum, vw, i)?;
                    weights = add(weights, w, i)?;
                }
                Self::Percentile(_) => values.push(s.value),
            }
        }
        let count = Decimal::from(n);
        Ok(match self {
            Self::Sum | Self::WeightedSum => Some(Usage::from(sum)),
            Self::Count => Some(Usage::from(count)),
            Self::UniqueCount => Some(Usage::from(Decimal::from(keys.len()))),
            Self::Min | Self::Max | Self::Latest => pick.map(Usage::from),
            Self::Mean => (n > 0).then(|| Usage::ratio(sum, count)),
            Self::WeightedMean => (weights > Decimal::ZERO).then(|| Usage::ratio(sum, weights)),
            Self::Percentile(p) => nearest_rank(&mut values, *p).map(Usage::from),
        })
    }
}

/// Converts an exact aggregate into billing units: divide by `divisor`, then
/// round **once** to `decimals` by `rounding` — seconds into started hours
/// (`3600`, 0, [`Rounding::AwayFromZero`]), megabytes into whole gigabytes
/// (`1024`, 0, [`Rounding::TowardZero`]).
///
/// ```
/// use billing::{Aggregation, ErrorKind, Meter, Rounding, Sample, UnitConversion};
/// use rust_decimal::dec;
///
/// let hours = UnitConversion::new(dec!(3600), 0, Rounding::AwayFromZero)?;
/// let meter = Meter::new(Aggregation::Sum).with_conversion(hours);
/// assert_eq!(meter.apply([Sample::new(dec!(7201))])?.unwrap().to_decimal(), dec!(3));
/// assert_eq!(meter.apply([Sample::new(dec!(7200))])?.unwrap().to_decimal(), dec!(2));
///
/// let gb = UnitConversion::new(dec!(1024), 0, Rounding::TowardZero)?;
/// let meter = Meter::new(Aggregation::Sum).with_conversion(gb);
/// assert_eq!(meter.apply([Sample::new(dec!(1536))])?.unwrap().to_decimal(), dec!(1));
///
/// let err = UnitConversion::new(dec!(0), 0, Rounding::TowardZero).unwrap_err();
/// assert_eq!(err.kind(), &ErrorKind::InvalidConversion);
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawConversion"))]
pub struct UnitConversion {
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    divisor: Decimal,
    decimals: u8,
    rounding: Rounding,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawConversion {
    #[serde(with = "crate::decimal::serde_str")]
    divisor: Decimal,
    decimals: u8,
    rounding: Rounding,
}

#[cfg(feature = "serde")]
impl TryFrom<RawConversion> for UnitConversion {
    type Error = Error;
    fn try_from(raw: RawConversion) -> Result<Self> {
        Self::new(raw.divisor, raw.decimals, raw.rounding)
    }
}

impl UnitConversion {
    /// Divide by `divisor` (> 0), round once to `decimals` (≤ 28) by `rounding`.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidConversion`] for a divisor ≤ 0 or more than 28
    /// decimals.
    pub fn new(divisor: Decimal, decimals: u8, rounding: Rounding) -> Result<Self> {
        if divisor <= Decimal::ZERO || decimals > crate::decimal::MAX_DECIMALS {
            return Err(ErrorKind::InvalidConversion.into());
        }
        Ok(Self {
            divisor,
            decimals,
            rounding,
        })
    }

    /// The divisor, > 0.
    #[must_use]
    pub fn divisor(&self) -> Decimal {
        self.divisor
    }

    /// The decimals the converted usage is rounded to, ≤ 28.
    #[must_use]
    pub fn decimals(&self) -> u8 {
        self.decimals
    }

    /// The rounding mode.
    #[must_use]
    pub fn rounding(&self) -> Rounding {
        self.rounding
    }

    /// `round(usage ÷ divisor)`, once, from the exact fraction.
    fn convert(self, u: Usage) -> Result<Usage> {
        let den = exact_mul(u.denominator, self.divisor).ok_or(ErrorKind::Overflow)?;
        round_ratio(u.numerator, den, self.decimals, self.rounding).map(Usage::from)
    }
}

/// A meter as data: an [`Aggregation`] and an optional [`UnitConversion`]
/// applied once to the exact aggregate (for an average, to the exact fraction).
///
/// ```
/// use billing::{Aggregation, Meter, Rounding, Sample, UnitConversion};
/// use rust_decimal::dec;
///
/// // Mean 1/3 converted to two decimals: rounded once from the fraction.
/// let m = Meter::new(Aggregation::Mean)
///     .with_conversion(UnitConversion::new(dec!(1), 2, Rounding::AwayFromZero)?);
/// let u = m.apply([dec!(1), dec!(0), dec!(0)].map(Sample::new))?.unwrap();
/// assert_eq!((u.to_decimal(), u.denominator()), (dec!(0.34), dec!(1)));
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Meter {
    /// How readings reduce.
    pub aggregation: Aggregation,
    /// The conversion into billing units, if any.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub conversion: Option<UnitConversion>,
}

impl Meter {
    /// A meter of `aggregation`, without conversion.
    #[must_use]
    pub fn new(aggregation: Aggregation) -> Self {
        Self {
            aggregation,
            conversion: None,
        }
    }

    /// Convert the aggregate with `conversion`.
    #[must_use]
    pub fn with_conversion(mut self, conversion: UnitConversion) -> Self {
        self.conversion = Some(conversion);
        self
    }

    /// Aggregate `samples`, then convert. "No value" stays no value.
    ///
    /// # Errors
    /// As [`Aggregation::apply`]; [`ErrorKind::Overflow`] if the conversion
    /// does not fit.
    pub fn apply(&self, samples: impl IntoIterator<Item = Sample>) -> Result<Option<Usage>> {
        let usage = self.aggregation.apply(samples)?;
        match (usage, self.conversion) {
            (Some(u), Some(c)) => c.convert(u).map(Some),
            (u, _) => Ok(u),
        }
    }
}

/// An exact billable quantity: `numerator ÷ denominator`, denominator > 0.
///
/// The denominator is 1 for every kind except an unconverted
/// [`Mean`](Aggregation::Mean) or [`WeightedMean`](Aggregation::WeightedMean),
/// which keep their exact fraction so that a line built from them
/// ([`Line::from_usage`](crate::Line::from_usage)) or a tariff rating them
/// ([`Tariff::rate`](crate::Tariff::rate)) rounds once from the exact value. A
/// plain `Decimal` converts with `From`.
///
/// A usage is stored in canonical form, so equality and hashing compare
/// **values**: the average of 3 and 9 equals `Usage::from(dec!(6))`, and two
/// averages that are both 1/3 compare equal however they were reached.
///
/// ```
/// use billing::{Aggregation, Sample, Usage};
/// use rust_decimal::dec;
///
/// let third = Aggregation::Mean.apply([dec!(1), dec!(0), dec!(0)].map(Sample::new))?.unwrap();
/// assert_eq!((third.numerator(), third.denominator()), (dec!(1), dec!(3)));
/// assert_eq!(third.to_decimal(), dec!(0.3333333333333333333333333333)); // display only
/// assert!(!third.is_exact_decimal());
/// assert!(Usage::from(dec!(2.5)).is_exact_decimal());
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawUsage"))]
pub struct Usage {
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    numerator: Decimal,
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    denominator: Decimal,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawUsage {
    #[serde(with = "crate::decimal::serde_str")]
    numerator: Decimal,
    #[serde(with = "crate::decimal::serde_str")]
    denominator: Decimal,
}

#[cfg(feature = "serde")]
impl TryFrom<RawUsage> for Usage {
    type Error = Error;
    fn try_from(raw: RawUsage) -> Result<Self> {
        Self::checked(raw.numerator, raw.denominator)
    }
}

impl Usage {
    /// `n / d` for `d > 0` whose decimal reading fits (every aggregate does: an
    /// average lies between its readings), in **canonical form**, so that equal
    /// values compare (and hash) equal: a value whose decimal reading is exact is
    /// stored as that decimal over 1 (28/5 and 56/10 are both 5.6/1); any other
    /// value as a fraction of integers in lowest terms (2/6 is 1/3). A fraction
    /// whose integer form would not fit `i128` keeps its normalised form.
    fn ratio(n: Decimal, d: Decimal) -> Self {
        let (n, d) = (n.normalize(), d.normalize());
        if d == Decimal::ONE {
            return Self {
                numerator: n,
                denominator: d,
            };
        }
        if let Some(q) = n.checked_div(d) {
            if exact_mul(q, d) == Some(n) {
                return Self {
                    numerator: q.normalize(),
                    denominator: Decimal::ONE,
                };
            }
        }
        match lowest_terms(n, d) {
            Some((num, den)) => Self {
                numerator: num,
                denominator: den,
            },
            None => Self {
                numerator: n,
                denominator: d,
            },
        }
    }

    /// `n / d`, refused unless `d > 0` and the decimal reading fits.
    pub(crate) fn checked(n: Decimal, d: Decimal) -> Result<Self> {
        if d <= Decimal::ZERO {
            return Err(ErrorKind::DivisionByZero.into());
        }
        n.checked_div(d).ok_or(ErrorKind::Overflow)?;
        Ok(Self::ratio(n, d))
    }

    /// The numerator (carries the sign).
    #[must_use]
    pub fn numerator(&self) -> Decimal {
        self.numerator
    }

    /// The denominator, > 0.
    #[must_use]
    pub fn denominator(&self) -> Decimal {
        self.denominator
    }

    /// The decimal reading: exact when the denominator is 1, otherwise the
    /// quotient to `Decimal`'s 28 significant digits — for display (BT-129),
    /// never for arithmetic.
    #[must_use]
    pub fn to_decimal(&self) -> Decimal {
        if self.denominator == Decimal::ONE {
            return self.numerator;
        }
        // Construction guarantees the quotient fits.
        self.numerator
            .checked_div(self.denominator)
            .unwrap_or(self.numerator)
    }

    /// Whether [`to_decimal`](Self::to_decimal) is the exact value (28/5 is
    /// 5.6 exactly; 1/3 is not).
    #[must_use]
    pub fn is_exact_decimal(&self) -> bool {
        exact_mul(self.to_decimal(), self.denominator) == Some(self.numerator)
    }

    /// The usage with the opposite sign.
    #[must_use]
    pub(crate) fn negated(self) -> Self {
        Self::ratio(crate::line::neg(self.numerator), self.denominator)
    }

    /// `|usage|`.
    #[must_use]
    pub(crate) fn abs(self) -> Self {
        Self::ratio(self.numerator.abs(), self.denominator)
    }

    /// `self + other`, exactly: `(a·d + c·b) / (b·d)` in canonical form.
    /// [`ErrorKind::Overflow`] where it does not fit.
    pub(crate) fn checked_add(self, other: Self) -> Result<Self> {
        if self.denominator == other.denominator {
            let n = exact_add(self.numerator, other.numerator).ok_or(ErrorKind::Overflow)?;
            return Self::checked(n, self.denominator);
        }
        let over = || Error::from(ErrorKind::Overflow);
        let ad = exact_mul(self.numerator, other.denominator).ok_or_else(over)?;
        let cb = exact_mul(other.numerator, self.denominator).ok_or_else(over)?;
        let n = exact_add(ad, cb).ok_or_else(over)?;
        let d = exact_mul(self.denominator, other.denominator).ok_or_else(over)?;
        Self::checked(n, d)
    }

    /// `self − other`, exactly (see [`checked_add`](Self::checked_add)).
    pub(crate) fn checked_sub(self, other: Self) -> Result<Self> {
        self.checked_add(other.negated())
    }

    /// Whether the usage is zero.
    pub(crate) fn is_zero(&self) -> bool {
        self.numerator.is_zero()
    }

    /// Whether the usage is below zero.
    pub(crate) fn is_negative(&self) -> bool {
        self.numerator.is_sign_negative() && !self.numerator.is_zero()
    }
}

/// `n / d` as a fraction of integers in lowest terms (`d > 0`), or `None` when
/// the integer form does not fit.
fn lowest_terms(n: Decimal, d: Decimal) -> Option<(Decimal, Decimal)> {
    let scale = n.scale().max(d.scale());
    let lift = |x: Decimal| -> Option<i128> {
        x.mantissa()
            .checked_mul(crate::decimal::pow10(scale - x.scale())?)
    };
    let (mut ni, mut di) = (lift(n)?, lift(d)?);
    let (mut a, mut b) = (ni.unsigned_abs(), di.unsigned_abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    if a > 1 {
        let g = i128::try_from(a).ok()?;
        ni /= g;
        di /= g;
    }
    Some((
        Decimal::try_from_i128_with_scale(ni, 0).ok()?,
        Decimal::try_from_i128_with_scale(di, 0).ok()?,
    ))
}

impl From<Decimal> for Usage {
    fn from(d: Decimal) -> Self {
        Self::ratio(d, Decimal::ONE)
    }
}
