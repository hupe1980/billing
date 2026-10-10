//! Pricing as data: [`Tier`], [`Tiers`], [`Tariff`] and the [`Piece`]s a tariff
//! rates a quantity into; [`Tariff::rate_increment`] and
//! [`Tariff::increment_lines`] for progressive billing of cumulative usage
//! from a [`Billed`] state;
//! [`rate_series`] for interval (spot) prices, [`rate_by_key`] for a tariff per
//! key (HT/NT, region, model), [`TransactionFee`] for fees on transaction value,
//! and [`nearest_rank`] for burstable (95th-percentile) billing.
//!
//! Nothing here rounds. Every result is an exact `Decimal`; the document reduces
//! it once at its scale. [`Tariff`] documents the rating semantics.

use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

use rust_decimal::Decimal;

use crate::decimal::{exact_add, exact_mul};
use crate::vat::reduce_exact;
use crate::{
    Amount, AmountScale, Error, ErrorKind, Line, Location, Percent, Price, Result, UnitPrice, Usage,
};

/// Why a set of tiers (or a package size) is invalid; carried by
/// [`ErrorKind::InvalidTiers`], usually with a [`Location::Tier`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TierError {
    /// No tiers were given.
    Empty,
    /// An `up_to` is not above the previous tier's.
    NotIncreasing,
    /// A tier other than the last has no `up_to`.
    OpenNotLast,
    /// An `up_to` is zero or negative.
    NonPositiveBound,
    /// A flat fee is negative.
    NegativeFlat,
    /// A package size is zero or negative.
    PackageSize,
}

impl fmt::Display for TierError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "at least one tier is required",
            Self::NotIncreasing => "up_to must be strictly increasing",
            Self::OpenNotLast => "only the last tier may be open",
            Self::NonPositiveBound => "up_to must be above zero",
            Self::NegativeFlat => "flat fee must not be negative",
            Self::PackageSize => "package size must be above zero",
        })
    }
}

fn tier_error(e: TierError) -> Error {
    ErrorKind::InvalidTiers(e).into()
}

/// One step of a step function: every unit up to and **including** `up_to`
/// (`None`: and above) costs `unit_price`, plus an optional `flat` fee charged
/// once when the tier is reached.
///
/// A capacity or stairstep price is a flat-only tier (`unit_price` 0) in a
/// [`Tariff::Volume`].
///
/// ```
/// use billing::{Price, Tier};
/// use rust_decimal::dec;
///
/// let t = Tier::up_to(dec!(100), Price::new(dec!(0.30))).with_flat(dec!(5));
/// assert_eq!((t.limit(), t.flat()), (Some(dec!(100)), Some(dec!(5))));
/// assert_eq!(Tier::above(Price::new(dec!(0.25))).limit(), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tier {
    #[cfg_attr(feature = "serde", serde(with = "opt_dec"))]
    up_to: Option<Decimal>,
    unit_price: Price,
    #[cfg_attr(
        feature = "serde",
        serde(default, with = "opt_dec", skip_serializing_if = "Option::is_none")
    )]
    flat: Option<Decimal>,
}

impl Tier {
    /// Units up to and including `limit` at `unit_price`.
    #[must_use]
    pub const fn up_to(limit: Decimal, unit_price: Price) -> Self {
        Self {
            up_to: Some(limit),
            unit_price,
            flat: None,
        }
    }

    /// The open last tier: every unit above the previous bound at `unit_price`.
    #[must_use]
    pub const fn above(unit_price: Price) -> Self {
        Self {
            up_to: None,
            unit_price,
            flat: None,
        }
    }

    /// Add a flat fee charged once when this tier is reached.
    #[must_use]
    pub const fn with_flat(mut self, flat: Decimal) -> Self {
        self.flat = Some(flat);
        self
    }

    /// The inclusive upper bound, `None` for the open last tier.
    #[must_use]
    pub const fn limit(&self) -> Option<Decimal> {
        self.up_to
    }

    /// The price per unit in this tier.
    #[must_use]
    pub const fn unit_price(&self) -> Price {
        self.unit_price
    }

    /// The flat fee, if any.
    #[must_use]
    pub const fn flat(&self) -> Option<Decimal> {
        self.flat
    }

    /// The flat fee when it is non-zero: what a piece carries.
    fn charged_flat(&self) -> Option<Decimal> {
        self.flat.filter(|f| !f.is_zero())
    }
}

/// Validated tiers: at least one, `up_to` above zero and strictly increasing,
/// only the last tier open, prices and flat fees ≥ 0. Input order is the order
/// of the tiers; unsorted input is refused, never sorted.
///
/// Deserialising goes through [`Tiers::new`].
///
/// ```
/// use billing::{ErrorKind, Location, Price, Tier, TierError, Tiers};
/// use rust_decimal::dec;
///
/// let p = |x| Price::new(x);
/// let tiers = Tiers::new([Tier::up_to(dec!(100), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?;
/// assert_eq!(tiers.at(dec!(100))?.unit_price(), p(dec!(0.30))); // inclusive
/// assert_eq!(tiers.at(dec!(100.01))?.unit_price(), p(dec!(0.25)));
///
/// let err = Tiers::new([Tier::up_to(dec!(100), p(dec!(1))), Tier::up_to(dec!(50), p(dec!(1)))])
///     .unwrap_err();
/// assert_eq!(err.kind(), &ErrorKind::InvalidTiers(TierError::NotIncreasing));
/// assert_eq!(err.location(), Some(&Location::Tier(1)));
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "Vec<Tier>", into = "Vec<Tier>"))]
pub struct Tiers(Vec<Tier>);

impl Tiers {
    /// Validate `tiers` in the order given.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidTiers`] (with the [`TierError`] and, except for
    /// [`TierError::Empty`], the [`Location::Tier`]);
    /// [`ErrorKind::NegativePrice`] for a negative unit price (BR-27).
    pub fn new(tiers: impl IntoIterator<Item = Tier>) -> Result<Self> {
        let tiers: Vec<Tier> = tiers.into_iter().collect();
        if tiers.is_empty() {
            return Err(tier_error(TierError::Empty));
        }
        let last = tiers.len() - 1;
        let mut prev: Option<Decimal> = None;
        for (i, t) in tiers.iter().enumerate() {
            let at = Location::Tier(i);
            if t.unit_price.value() < Decimal::ZERO {
                return Err(Error::from(ErrorKind::NegativePrice {
                    price: t.unit_price.value(),
                })
                .at(at));
            }
            if t.flat.is_some_and(|f| f < Decimal::ZERO) {
                return Err(tier_error(TierError::NegativeFlat).at(at));
            }
            match t.up_to {
                None if i != last => return Err(tier_error(TierError::OpenNotLast).at(at)),
                None => {}
                Some(u) if u <= Decimal::ZERO => {
                    return Err(tier_error(TierError::NonPositiveBound).at(at));
                }
                Some(u) => {
                    if prev.is_some_and(|p| u <= p) {
                        return Err(tier_error(TierError::NotIncreasing).at(at));
                    }
                    prev = Some(u);
                }
            }
        }
        Ok(Self(tiers))
    }

    /// The tiers, in order.
    #[must_use]
    pub fn tiers(&self) -> &[Tier] {
        &self.0
    }

    /// The tier containing `x`: the first whose inclusive `up_to` is ≥ `x`, or
    /// the open last tier.
    ///
    /// # Errors
    /// [`ErrorKind::Uncovered`] if `x` is negative or above the last bound of
    /// tiers that end closed.
    pub fn at(&self, x: Decimal) -> Result<&Tier> {
        self.index_of(x).map(|i| &self.0[i])
    }

    fn index_of(&self, x: Decimal) -> Result<usize> {
        self.index_of_usage(Usage::from(x))
    }

    /// [`index_of`](Self::index_of) for an exact fraction `n / d`: a bound `u`
    /// is compared as `n ≤ u·d`, exactly.
    fn index_of_usage(&self, x: Usage) -> Result<usize> {
        let uncovered = || {
            Error::from(ErrorKind::Uncovered {
                value: x.to_decimal(),
            })
        };
        if x.is_negative() {
            return Err(uncovered());
        }
        let mut below = Ok(());
        let i = self.0.partition_point(|t| {
            t.up_to.is_some_and(|u| match exceeds(x, u) {
                Ok(b) => b,
                Err(e) => {
                    below = Err(e);
                    false
                }
            })
        });
        below?;
        if i == self.0.len() {
            return Err(uncovered());
        }
        Ok(i)
    }
}

impl TryFrom<Vec<Tier>> for Tiers {
    type Error = Error;
    fn try_from(tiers: Vec<Tier>) -> Result<Self> {
        Self::new(tiers)
    }
}

impl From<Tiers> for Vec<Tier> {
    fn from(tiers: Tiers) -> Self {
        tiers.0
    }
}

/// How a measured quantity becomes a whole number of packages.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PackageRounding {
    /// Every started package is billed: ⌈q / size⌉.
    Up,
    /// Only complete packages are billed: ⌊q / size⌋.
    Down,
}

/// A price model as data: how a quantity is priced.
///
/// # Semantics shared by every tariff
///
/// * **Upper bounds are inclusive** ([Stripe] `up_to`): with tiers `up to 100`
///   and `above`, 100 units are all in tier 0 and 100.5 put 0.5 in tier 1.
/// * **A flat fee applies when its tier is reached.** Quantity 0 reaches tier 0
///   (Stripe's behaviour), so a flat fee on the first tier is charged even for
///   zero usage; a graduated tier `i > 0` is reached when the quantity exceeds
///   tier `i − 1`'s bound.
/// * **No slivers.** A piece is emitted only if its quantity is non-zero or it
///   carries a non-zero flat fee. Zero usage therefore rates to nothing, except
///   the first tier's flat fee (graduated or volume).
/// * **Direction lives in the quantity.** A negative quantity rates its
///   magnitude and negates every piece's `quantity`, `flat` and `measured`:
///   `rate(−q)` is the exact negation of `rate(q)` (the credit for the same
///   usage), never a delta against earlier usage. Prices stay ≥ 0 (BR-27).
///
/// Prices and package sizes in the variants are checked when rating.
///
/// [Stripe]: https://docs.stripe.com/api/prices/create
///
/// ```
/// use billing::{Price, Tariff, Tier, Tiers};
/// use rust_decimal::dec;
///
/// let p = |x| Price::new(x);
/// let tiers = Tiers::new([Tier::up_to(dec!(100), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?;
///
/// // Graduated: each tier's units at its own price.
/// let pieces = Tariff::Graduated(tiers.clone()).rate(dec!(150))?;
/// assert_eq!(pieces.len(), 2);
/// assert_eq!((pieces[1].tier, pieces[1].quantity.to_decimal()), (1, dec!(50)));
///
/// // Volume: every unit at the reached tier's price.
/// let pieces = Tariff::Volume(tiers).rate(dec!(150))?;
/// assert_eq!(pieces[0].exact_net()?, dec!(37.50));
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Graduated percentage pricing
///
/// A percentage that steps down with volume (Lago's graduated percentage, a
/// payment processor's 2.9 % on the first 10 000 and 1.9 % above) needs no
/// model of its own: it is a graduated tariff over the transaction **volume**
/// with the rates as prices. Per-transaction fixed fees and bounds are
/// [`TransactionFee`]'s.
///
/// ```
/// use billing::{Price, Tariff, Tier, Tiers};
/// use rust_decimal::{Decimal, dec};
///
/// let rates = Tiers::new([
///     Tier::up_to(dec!(10000), Price::new(dec!(0.029))),
///     Tier::above(Price::new(dec!(0.019))),
/// ])?;
/// let pieces = Tariff::Graduated(rates).rate(dec!(15000))?; // the month's volume
/// let fee: Decimal = pieces.iter().map(|x| x.exact_net()).sum::<Result<_, _>>()?;
/// assert_eq!(fee, dec!(385.00)); // 10 000 × 2.9 % + 5 000 × 1.9 %
/// # Ok::<(), billing::Error>(())
/// ```
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Tariff {
    /// Every unit at one price.
    PerUnit(Price),
    /// Each tier's share of the quantity at that tier's price, plus the flat fee
    /// of every tier reached. Non-decreasing in `|quantity|`.
    Graduated(Tiers),
    /// The whole quantity at the price of the tier it falls in, plus that tier's
    /// flat fee. **Not monotone**: crossing a bound into a cheaper tier can lower
    /// the total (the "volume cliff", as in Stripe and Orb "bulk"). Flat-only
    /// tiers (`unit_price` 0) model capacity / stairstep prices.
    Volume(Tiers),
    /// Whole packages of `size` units at `price` each; the piece keeps the
    /// measured quantity.
    Package {
        /// Units per package, > 0.
        #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
        size: Decimal,
        /// Price per package, ≥ 0.
        price: Price,
        /// Whether a started package counts.
        rounding: PackageRounding,
    },
}

/// One priced piece of a rated quantity. The caller describes it (descriptions
/// and units are not the engine's business).
///
/// `exact_net = quantity × unit_price + flat`. The quantity is an exact
/// [`Usage`]: a fractional usage (an average) rates to fractional pieces, each
/// priced from its exact fraction by [`Piece::into_lines`].
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Piece {
    /// Index of the tier this piece belongs to (0 for per-unit and package).
    pub tier: usize,
    /// Units billed at `unit_price` (packages, for a package tariff), exactly:
    /// a tier width has denominator 1, the remainder of a fractional usage keeps
    /// its denominator. Signed: negative for a negative rated quantity. Zero
    /// only for a flat-only piece.
    pub quantity: Usage,
    /// Price per unit (per package).
    pub unit_price: Price,
    /// The tier's flat fee, if non-zero, signed like the rated quantity.
    #[cfg_attr(
        feature = "serde",
        serde(default, with = "opt_dec", skip_serializing_if = "Option::is_none")
    )]
    pub flat: Option<Decimal>,
    /// For a package tariff, the measured (unrounded) quantity, as its decimal
    /// reading ([`Usage::to_decimal`]).
    #[cfg_attr(
        feature = "serde",
        serde(default, with = "opt_dec", skip_serializing_if = "Option::is_none")
    )]
    pub measured: Option<Decimal>,
}

impl Piece {
    /// `quantity × unit_price + flat`: exact when the quantity's denominator
    /// is 1 or divides `numerator × unit_price` exactly, otherwise the quotient to
    /// `Decimal`'s 28 digits (the document reduces the exact fraction, see
    /// [`Piece::into_lines`]).
    ///
    /// # Errors
    /// [`ErrorKind::Overflow`] if it does not fit `Decimal`.
    pub fn exact_net(&self) -> Result<Decimal> {
        let usage = exact_mul(self.quantity.numerator(), self.unit_price.value())
            .and_then(|p| p.checked_div(self.quantity.denominator()))
            .ok_or(ErrorKind::Overflow)?;
        Ok(usage
            .checked_add(self.flat.unwrap_or_default())
            .ok_or(ErrorKind::Overflow)?)
    }

    fn negate(&mut self) {
        self.quantity = self.quantity.negated();
        self.flat = self.flat.map(|f| -f);
        self.measured = self.measured.map(|m| -m);
    }
}

impl Tariff {
    /// Rate `usage` into pieces (see [`Tariff`] for zero, negative and boundary
    /// semantics). A `Decimal` converts into a [`Usage`]; a fractional usage
    /// (an average) is rated exactly: tier bounds are compared with the
    /// fraction, and every piece carries an exact [`Usage`].
    ///
    /// ```
    /// use billing::{Aggregation, Price, Sample, Tariff, Tier, Tiers, Usage};
    /// use rust_decimal::dec;
    ///
    /// let tiers = Tiers::new([Tier::up_to(dec!(1), Price::new(dec!(0.015))), Tier::above(Price::new(dec!(0.01)))])?;
    /// let four_thirds = Aggregation::Mean.apply([dec!(1), dec!(1), dec!(2)].map(Sample::new))?.unwrap();
    /// let pieces = Tariff::Graduated(tiers).rate(four_thirds)?;
    /// assert_eq!(pieces[0].quantity, Usage::from(dec!(1)));
    /// assert_eq!((pieces[1].quantity.numerator(), pieces[1].quantity.denominator()), (dec!(1), dec!(3)));
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// Each piece is a separate leaf: the document reduces each piece's net on
    /// its own (Stripe also rounds per tier), so two graduated pieces of
    /// 0.005 each give 0.02 at two decimals where one line would give 0.01.
    ///
    /// # Errors
    /// [`ErrorKind::Uncovered`] if the tiers end closed below `|quantity|`;
    /// [`ErrorKind::NegativePrice`] for a negative per-unit or package price;
    /// [`ErrorKind::InvalidTiers`] with [`TierError::PackageSize`] for a package
    /// size ≤ 0; [`ErrorKind::Overflow`].
    pub fn rate(&self, usage: impl Into<Usage>) -> Result<Vec<Piece>> {
        let usage = usage.into();
        let q = usage.abs();
        let mut pieces = match self {
            Self::PerUnit(price) => {
                check_price(*price)?;
                let piece = Piece {
                    tier: 0,
                    quantity: q,
                    unit_price: *price,
                    flat: None,
                    measured: None,
                };
                if q.is_zero() {
                    Vec::new()
                } else {
                    alloc::vec![piece]
                }
            }
            Self::Graduated(tiers) => graduated(tiers, q)?,
            Self::Volume(tiers) => {
                let i = tiers.index_of_usage(q)?;
                let t = &tiers.0[i];
                let piece = Piece {
                    tier: i,
                    quantity: q,
                    unit_price: t.unit_price,
                    flat: t.charged_flat(),
                    measured: None,
                };
                if q.is_zero() && piece.flat.is_none() {
                    Vec::new()
                } else {
                    alloc::vec![piece]
                }
            }
            Self::Package {
                size,
                price,
                rounding,
            } => {
                check_price(*price)?;
                // d > 0, so `per` has the sign of `size`: `packages` checks it.
                let per = exact_mul(q.denominator(), *size).ok_or(ErrorKind::Overflow)?;
                let n = packages(q.numerator(), per, *rounding)?;
                if n.is_zero() {
                    Vec::new()
                } else {
                    alloc::vec![Piece {
                        tier: 0,
                        quantity: Usage::from(n),
                        unit_price: *price,
                        flat: None,
                        measured: Some(q.to_decimal()),
                    }]
                }
            }
        };
        if usage.is_negative() {
            pieces.iter_mut().for_each(Piece::negate);
        }
        Ok(pieces)
    }
}

fn check_price(price: Price) -> Result<()> {
    if price.value() < Decimal::ZERO {
        return Err(ErrorKind::NegativePrice {
            price: price.value(),
        }
        .into());
    }
    Ok(())
}

/// Whether the exact usage `x = n / d` exceeds the bound `u`: `n > u·d`.
fn exceeds(x: Usage, u: Decimal) -> Result<bool> {
    let ud = exact_mul(u, x.denominator()).ok_or(ErrorKind::Overflow)?;
    Ok(x.numerator() > ud)
}

/// `x − lower` for `x = n / d`, exactly: `(n − lower·d) / d`.
fn minus(x: Usage, lower: Decimal) -> Result<Usage> {
    let overflow = || Error::from(ErrorKind::Overflow);
    let ld = exact_mul(lower, x.denominator()).ok_or_else(overflow)?;
    let n = exact_add(x.numerator(), -ld).ok_or_else(overflow)?;
    Usage::checked(n, x.denominator())
}

/// Walk the tiers for `q ≥ 0`: tier 0 is always reached, tier `i` when `q`
/// exceeds tier `i − 1`'s bound.
fn graduated(tiers: &Tiers, q: Usage) -> Result<Vec<Piece>> {
    tiers.index_of_usage(q)?; // coverage
    let mut pieces = Vec::new();
    let mut lower = Decimal::ZERO;
    for (i, t) in tiers.0.iter().enumerate() {
        if i > 0 && !exceeds(q, lower)? {
            break;
        }
        // The full tier width when q passes its bound, else the rest of q.
        let quantity = match t.up_to {
            Some(u) if exceeds(q, u)? => {
                Usage::from(u.checked_sub(lower).ok_or(ErrorKind::Overflow)?)
            }
            _ => minus(q, lower)?,
        };
        let flat = t.charged_flat();
        if !quantity.is_zero() || flat.is_some() {
            pieces.push(Piece {
                tier: i,
                quantity,
                unit_price: t.unit_price,
                flat,
                measured: None,
            });
        }
        match t.up_to {
            Some(u) => lower = u,
            None => break,
        }
    }
    Ok(pieces)
}

/// Whole packages of `size` in `q ≥ 0`, exactly: the quotient is corrected so
/// that `n·size ≤ q < (n+1)·size` holds in exact arithmetic, whatever the
/// 28-digit division rounded to.
fn packages(q: Decimal, size: Decimal, rounding: PackageRounding) -> Result<Decimal> {
    if size <= Decimal::ZERO {
        return Err(tier_error(TierError::PackageSize));
    }
    let over = || Error::from(ErrorKind::Overflow);
    let mul = |n: Decimal| exact_mul(n, size).ok_or_else(over);
    let mut n = q.checked_div(size).ok_or_else(over)?.trunc();
    while n > Decimal::ZERO && mul(n)? > q {
        n -= Decimal::ONE;
    }
    while mul(n + Decimal::ONE)? <= q {
        n += Decimal::ONE;
    }
    if rounding == PackageRounding::Up && mul(n)? < q {
        n += Decimal::ONE;
    }
    Ok(n.normalize())
}

/// An interval series rated exactly: see [`rate_series`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct SeriesTotal {
    /// Σ qᵢ, exact.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub quantity: Decimal,
    /// Σ qᵢ·pᵢ, exact. This, not `quantity × average_price`, is the net: the
    /// document reduces it once.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub net: Decimal,
    /// `|net / quantity|` (to `Decimal`'s 28 digits; 0 for an empty series): the
    /// stated BT-146, never negative (BR-27). When the average itself is negative
    /// — a month of spot prices below zero — the direction belongs to the line's
    /// quantity instead: see [`SeriesTotal::is_negative_average`].
    pub average_price: Price,
}

/// Rate a series of `(quantity, price)` intervals — spot / dynamic prices —
/// exactly: Σ qᵢ·pᵢ with no intermediate rounding. Zero-quantity intervals are
/// skipped.
///
/// Interval prices may be negative (spot markets go below zero), and so may
/// their average. A line's price must not be (BR-27), so a negative average is
/// reported as its magnitude and the direction moves to the line's quantity
/// ([`SeriesTotal::is_negative_average`]); the exact `net` keeps its sign.
///
/// ```
/// use billing::{Price, rate_series};
/// use rust_decimal::dec;
///
/// let s = rate_series([(dec!(0.5), Price::new(dec!(0.00999))), (dec!(2.28571), Price::new(dec!(0)))])?;
/// assert_eq!(s.net, dec!(0.004995)); // exact; reduced once by the document (→ 0.00 at 2 dp)
/// assert_eq!(s.quantity, dec!(2.78571));
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Errors
/// [`ErrorKind::DivisionByZero`] if Σ qᵢ is 0 but the net is not;
/// [`ErrorKind::Overflow`].
pub fn rate_series(intervals: impl IntoIterator<Item = (Decimal, Price)>) -> Result<SeriesTotal> {
    let (mut quantity, mut net) = (Decimal::ZERO, Decimal::ZERO);
    for (q, p) in intervals {
        if q.is_zero() {
            continue;
        }
        let amount = q.checked_mul(p.value()).ok_or(ErrorKind::Overflow)?;
        net = net.checked_add(amount).ok_or(ErrorKind::Overflow)?;
        quantity = quantity.checked_add(q).ok_or(ErrorKind::Overflow)?;
    }
    let average = if quantity.is_zero() {
        if !net.is_zero() {
            return Err(ErrorKind::DivisionByZero.into());
        }
        Decimal::ZERO
    } else {
        net.checked_div(quantity).ok_or(ErrorKind::Overflow)?
    };
    Ok(SeriesTotal {
        quantity,
        net,
        average_price: Price::new(average.abs().normalize()),
    })
}

impl SeriesTotal {
    /// Whether the exact average price `net / quantity` is negative. A line built
    /// from this series then carries `-quantity` at `average_price`, so that its
    /// net keeps the series' sign while its price stays non-negative (BR-27).
    #[must_use]
    pub fn is_negative_average(&self) -> bool {
        !self.quantity.is_zero()
            && (self.net.is_sign_negative() != self.quantity.is_sign_negative())
            && !self.net.is_zero()
    }
}

/// Price usage per key — a time-of-use band (HT/NT), a region, a model, a
/// storage class — with **a full [`Tariff`] per key** (a one-dimensional Orb
/// matrix price or Lago charge filter): each key's summed usage rated by its own
/// tariff, tiers included. A flat price per key is [`Tariff::PerUnit`]. Nothing
/// is tagged or described; the caller does that.
///
/// Usage for the same key is **summed exactly** (fractional usages add as
/// fractions); the result lists each key once, in order of first appearance,
/// with the pieces its tariff rates the sum into. A key whose pieces are empty
/// (summed usage zero, no first-tier flat fee) is dropped (no slivers); a key
/// in `tariffs` that no usage names yields nothing.
///
/// ```
/// use std::collections::BTreeMap;
/// use billing::{ErrorKind, Price, Tariff, Tier, Tiers, rate_by_key};
/// use rust_decimal::{Decimal, dec};
///
/// let p = |x| Price::new(x);
/// let tariffs = BTreeMap::from([
///     ("HT", Tariff::Graduated(Tiers::new([Tier::up_to(dec!(100), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?)),
///     ("NT", Tariff::PerUnit(p(dec!(0.20)))),
/// ]);
/// let rated = rate_by_key(&tariffs, [("HT", dec!(80)), ("NT", dec!(50)), ("HT", dec!(40))])?;
/// let total = |k: usize| rated[k].1.iter().map(|x| x.exact_net()).sum::<Result<Decimal, _>>();
/// assert_eq!((rated[0].0, total(0)?), ("HT", dec!(35.00))); // 100 × 0.30 + 20 × 0.25
/// assert_eq!((rated[1].0, total(1)?), ("NT", dec!(10.00)));
///
/// let err = rate_by_key(&tariffs, [("XX", dec!(1))]).unwrap_err();
/// assert_eq!(err.kind(), &ErrorKind::UnknownKey { key: "XX".into() });
/// # Ok::<(), billing::Error>(())
/// ```
///
/// # Errors
/// [`ErrorKind::UnknownKey`] naming the first key without a tariff; whatever
/// [`Tariff::rate`] refuses for a key's summed usage; [`ErrorKind::Overflow`]
/// if a sum does not fit.
pub fn rate_by_key<K, U>(
    tariffs: &BTreeMap<K, Tariff>,
    usage: impl IntoIterator<Item = (K, U)>,
) -> Result<Vec<(K, Vec<Piece>)>>
where
    K: Ord + Clone + fmt::Display,
    U: Into<Usage>,
{
    let mut sums: Vec<(K, &Tariff, Usage)> = Vec::new();
    let mut index: BTreeMap<K, usize> = BTreeMap::new();
    for (key, q) in usage {
        let Some(tariff) = tariffs.get(&key) else {
            return Err(ErrorKind::UnknownKey {
                key: key.to_string(),
            }
            .into());
        };
        let q = q.into();
        if let Some(&i) = index.get(&key) {
            sums[i].2 = sums[i].2.checked_add(q)?;
        } else {
            index.insert(key.clone(), sums.len());
            sums.push((key, tariff, q));
        }
    }
    let mut out = Vec::with_capacity(sums.len());
    for (key, tariff, q) in sums {
        let pieces = tariff.rate(q)?;
        if !pieces.is_empty() {
            out.push((key, pieces));
        }
    }
    Ok(out)
}

/// A fee on transaction value — a percentage of each transaction plus a fixed
/// amount, bounded per transaction, after a number of free transactions
/// (Stripe's application fees, Lago's percentage charge model, marketplace
/// commissions).
///
/// A plain input record, checked when rating: `0 ≤ percent ≤ 100`, `fixed ≥
/// 0`, `min, max ≥ 0` and `min ≤ max` when both are set. Per charged
/// transaction the fee is, exactly,
///
/// `sign(amount) × clamp(|amount| × percent ÷ 100 + fixed, min, max)`:
///
/// the bounds apply to the magnitude and a refund (a negative amount) returns
/// the fee of the purchase it mirrors. A zero amount is a transaction charged
/// the bounded fixed fee. The first `free_transactions` amounts, in input order
/// and refunds included, are counted but not charged. No bound means
/// unbounded. A policy that keeps the fixed fee on refunds is the caller's:
/// rate the refunds under a fee without it.
///
/// ```
/// use billing::{Percent, TransactionFee};
/// use rust_decimal::dec;
///
/// let fee = TransactionFee::new(Percent::new(dec!(2.9)))
///     .with_fixed(dec!(0.30))
///     .with_min(dec!(0.50))
///     .with_max(dec!(5.00))
///     .with_free_transactions(2);
/// // 10 and 100 free; 1 000 → 29.30 capped at 5.00; 5 → 0.445 raised to 0.50;
/// // the refund of 100 → −3.20.
/// let total = fee.rate([dec!(10), dec!(100), dec!(1000), dec!(5), dec!(-100)])?;
/// assert_eq!((total.transactions, total.charged), (5, 3));
/// assert_eq!((total.base, total.fee), (dec!(905), dec!(2.30)));
/// # Ok::<(), billing::Error>(())
/// ```
///
/// Deserialising checks the same rules (`fixed` and `free_transactions`
/// default to 0, the bounds to none).
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RawFee"))]
pub struct TransactionFee {
    /// Percent of each transaction's magnitude, `0..=100`.
    pub percent: Percent,
    /// Fixed fee per charged transaction, ≥ 0.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub fixed: Decimal,
    /// Least fee magnitude per charged transaction, ≥ 0.
    #[cfg_attr(
        feature = "serde",
        serde(default, with = "opt_dec", skip_serializing_if = "Option::is_none")
    )]
    pub min: Option<Decimal>,
    /// Greatest fee magnitude per charged transaction, ≥ 0 and ≥ `min`.
    #[cfg_attr(
        feature = "serde",
        serde(default, with = "opt_dec", skip_serializing_if = "Option::is_none")
    )]
    pub max: Option<Decimal>,
    /// How many transactions, from the first, are not charged.
    pub free_transactions: u32,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RawFee {
    percent: Percent,
    #[serde(default, with = "crate::decimal::serde_str")]
    fixed: Decimal,
    #[serde(default, with = "opt_dec")]
    min: Option<Decimal>,
    #[serde(default, with = "opt_dec")]
    max: Option<Decimal>,
    #[serde(default)]
    free_transactions: u32,
}

#[cfg(feature = "serde")]
impl TryFrom<RawFee> for TransactionFee {
    type Error = Error;
    fn try_from(raw: RawFee) -> Result<Self> {
        let fee = Self {
            percent: raw.percent,
            fixed: raw.fixed,
            min: raw.min,
            max: raw.max,
            free_transactions: raw.free_transactions,
        };
        fee.check()?;
        Ok(fee)
    }
}

/// `percent ÷ 100` is applied as `× 0.01`.
const HUNDREDTH: Decimal = Decimal::from_parts(1, 0, 0, false, 2);

impl TransactionFee {
    /// `percent` of each transaction: no fixed fee, no bounds, none free.
    #[must_use]
    pub fn new(percent: Percent) -> Self {
        Self {
            percent,
            fixed: Decimal::ZERO,
            min: None,
            max: None,
            free_transactions: 0,
        }
    }

    /// Add a fixed fee per charged transaction.
    #[must_use]
    pub fn with_fixed(mut self, fixed: Decimal) -> Self {
        self.fixed = fixed;
        self
    }

    /// Set the least fee magnitude per charged transaction.
    #[must_use]
    pub fn with_min(mut self, min: Decimal) -> Self {
        self.min = Some(min);
        self
    }

    /// Set the greatest fee magnitude per charged transaction.
    #[must_use]
    pub fn with_max(mut self, max: Decimal) -> Self {
        self.max = Some(max);
        self
    }

    /// Leave the first `n` transactions uncharged.
    #[must_use]
    pub fn with_free_transactions(mut self, n: u32) -> Self {
        self.free_transactions = n;
        self
    }

    fn check(&self) -> Result<()> {
        let negative = |d: Decimal| d < Decimal::ZERO;
        let percent = self.percent.value();
        let invalid = negative(percent)
            || percent > Decimal::ONE_HUNDRED
            || negative(self.fixed)
            || self.min.is_some_and(negative)
            || self.max.is_some_and(negative)
            || matches!((self.min, self.max), (Some(lo), Some(hi)) if lo > hi);
        if invalid {
            return Err(ErrorKind::InvalidFee.into());
        }
        Ok(())
    }

    /// Rate `amounts` (in the document's currency, in input order) exactly:
    /// see [`TransactionFee`] for the per-transaction formula. Nothing is
    /// rounded; [`FeeTotal::into_line`] states the exact fee on a line, which
    /// the document rounds once.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidFee`] for a fee breaking the rules on
    /// [`TransactionFee`]; [`ErrorKind::Overflow`] at the
    /// [`Location::Transaction`] whose fee or running sum does not fit
    /// `Decimal` exactly.
    pub fn rate(&self, amounts: impl IntoIterator<Item = Decimal>) -> Result<FeeTotal> {
        self.check()?;
        let mut total = FeeTotal {
            transactions: 0,
            charged: 0,
            base: Decimal::ZERO,
            fee: Decimal::ZERO,
        };
        let free = u64::from(self.free_transactions);
        for (i, amount) in amounts.into_iter().enumerate() {
            total.transactions += 1;
            if total.transactions <= free {
                continue;
            }
            let over = || Error::from(ErrorKind::Overflow).at(Location::Transaction(i));
            let mut fee = exact_mul(amount.abs(), self.percent.value())
                .and_then(|x| exact_mul(x, HUNDREDTH))
                .and_then(|x| exact_add(x, self.fixed))
                .ok_or_else(over)?;
            if let Some(lo) = self.min {
                fee = fee.max(lo);
            }
            if let Some(hi) = self.max {
                fee = fee.min(hi);
            }
            if amount < Decimal::ZERO {
                fee = crate::line::neg(fee);
            }
            total.charged += 1;
            total.base = exact_add(total.base, amount).ok_or_else(over)?;
            total.fee = exact_add(total.fee, fee).ok_or_else(over)?;
        }
        Ok(total)
    }
}

/// The result of [`TransactionFee::rate`]: a result record, exact.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct FeeTotal {
    /// The amounts seen, free ones included.
    pub transactions: u64,
    /// The amounts charged (after the free ones).
    pub charged: u64,
    /// Σ charged amounts, exact.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub base: Decimal,
    /// Σ per-transaction fees, exact and signed: the net the document rounds
    /// once.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub fee: Decimal,
}

/// The tiers of two ratings side by side: `(before, after)` for every tier
/// index up to the highest either rates into, ascending (`None` where a rating
/// has no piece in that tier; a pair of `None`s yields no piece or line). A
/// rating has at most one piece per tier.
fn by_tier<'a>(
    before: &'a [Piece],
    after: &'a [Piece],
) -> Vec<(Option<&'a Piece>, Option<&'a Piece>)> {
    let tiers = before
        .iter()
        .chain(after)
        .map(|p| p.tier)
        .max()
        .map_or(0, |m| m + 1);
    let at = |ps: &'a [Piece], k: usize| ps.iter().find(|p| p.tier == k);
    (0..tiers).map(|k| (at(before, k), at(after, k))).collect()
}

/// What progressive billing has billed so far: the state
/// [`Tariff::rate_increment`] and [`Tariff::increment_lines`] take and return.
///
/// `Nothing` and `UpTo(0)` are different states: from `Nothing` the next
/// increment charges the full rating of its target — the first-tier flat fee
/// included, even at zero usage — while from `UpTo(0)` that rating has already
/// been billed. The crate is stateless: the caller stores the returned state
/// between invoices (it serialises as `"nothing"` or `{"up_to": usage}`).
///
/// ```
/// use billing::{Billed, Price, Tariff, Tier, Tiers};
/// use rust_decimal::{Decimal, dec};
///
/// // A base fee of 10.00 with the first tier; months of 0, 0 and 50 kWh cumulative.
/// let t = Tariff::Graduated(Tiers::new([
///     Tier::up_to(dec!(100), Price::new(dec!(0.30))).with_flat(dec!(10)),
///     Tier::above(Price::new(dec!(0.25))),
/// ])?);
/// let mut billed = Billed::Nothing;
/// let mut months = Vec::new();
/// for to in [dec!(0), dec!(0), dec!(50)] {
///     let (pieces, next) = t.rate_increment(billed, to)?;
///     months.push(pieces.iter().map(|x| x.exact_net()).sum::<Result<Decimal, _>>()?);
///     billed = next;
/// }
/// assert_eq!(months, [dec!(10), dec!(0), dec!(15)]); // Σ 25 = rating 50 at once
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Billed {
    /// Nothing billed yet: the next increment charges the full rating.
    #[default]
    Nothing,
    /// Billed up to this cumulative usage (≥ 0): the rating of it is billed.
    UpTo(Usage),
}

impl Tariff {
    /// The validated range from `billed` to `to` and both ratings (what was
    /// billed, what is due), or `None` when nothing changes.
    fn increment_ratings(
        &self,
        billed: Billed,
        to: Usage,
    ) -> Result<Option<(Vec<Piece>, Vec<Piece>)>> {
        let from = match billed {
            Billed::Nothing => Usage::from(Decimal::ZERO),
            Billed::UpTo(f) => f,
        };
        if from.is_negative() || to.checked_sub(from)?.is_negative() {
            return Err(ErrorKind::InvalidRange {
                from: from.to_decimal(),
                to: to.to_decimal(),
            }
            .into());
        }
        match billed {
            Billed::Nothing => Ok(Some((Vec::new(), self.rate(to)?))),
            Billed::UpTo(f) if f == to => Ok(None),
            Billed::UpTo(f) => Ok(Some((self.rate(f)?, self.rate(to)?))),
        }
    }

    /// The usage measured by a package increment: `to − from`, `from` being
    /// zero when nothing was billed.
    fn usage_added(billed: Billed, to: Usage) -> Result<Usage> {
        match billed {
            Billed::Nothing => Ok(to),
            Billed::UpTo(f) => to.checked_sub(f),
        }
    }

    /// Rate the **increment** from what was `billed` to cumulative usage `to`
    /// (progressive billing: tiers count usage across several invoices, each
    /// invoice bills what changed). Returns the pieces and the next state,
    /// [`Billed::UpTo`]`(to)`, for the caller to store.
    ///
    /// From [`Billed::Nothing`] the increment is the full rating of `to`, the
    /// first-tier flat fee included even at zero usage. From
    /// [`Billed::UpTo`]`(from)`, each tier's piece is the exact difference
    /// between rating `to` and rating `from` (for a package tariff `measured`
    /// is the usage added); `from == to` yields nothing. For every
    /// non-decreasing sequence `c₁ ≤ … ≤ cₙ` rated from `Nothing`, the exact
    /// nets of the increments sum to the exact net of
    /// [`rate`](Self::rate)`(cₙ)`.
    ///
    /// **Increments can be negative**: crossing into a cheaper volume tier
    /// lowers the total, the direction carried in the quantity. Use
    /// [`increment_lines`](Self::increment_lines) for rounded lines that also
    /// add up across invoices.
    ///
    /// ```
    /// use billing::{Billed, Price, Tariff, Tier, Tiers, Usage};
    /// use rust_decimal::{Decimal, dec};
    ///
    /// let p = |x| Price::new(x);
    /// let tiers = Tiers::new([Tier::up_to(dec!(1000), p(dec!(0.30))), Tier::above(p(dec!(0.25)))])?;
    /// let net = |t: &Tariff, billed, to| -> Result<Decimal, billing::Error> {
    ///     t.rate_increment(billed, to)?.0.iter().map(|x| x.exact_net()).sum()
    /// };
    /// let up_to = |x| Billed::UpTo(Usage::from(x));
    /// let graduated = Tariff::Graduated(tiers.clone());
    /// assert_eq!(net(&graduated, Billed::Nothing, dec!(400))?, dec!(120.00));
    /// assert_eq!(net(&graduated, up_to(dec!(400)), dec!(900))?, dec!(150.00));
    /// assert_eq!(net(&graduated, up_to(dec!(900)), dec!(1300))?, dec!(105.00)); // 100 × 0.30 + 300 × 0.25
    ///
    /// // The volume cliff: 1 010 × 0.25 − 990 × 0.30.
    /// assert_eq!(net(&Tariff::Volume(tiers), up_to(dec!(990)), dec!(1010))?, dec!(-44.50));
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// [`ErrorKind::InvalidRange`] unless `0 ≤ to` from `Nothing`, or
    /// `0 ≤ from ≤ to` from `UpTo(from)`; whatever [`rate`](Self::rate)
    /// refuses for `from` or `to`; [`ErrorKind::Overflow`].
    pub fn rate_increment(
        &self,
        billed: Billed,
        to: impl Into<Usage>,
    ) -> Result<(Vec<Piece>, Billed)> {
        let to = to.into();
        let next = Billed::UpTo(to);
        let Some((before, after)) = self.increment_ratings(billed, to)? else {
            return Ok((Vec::new(), next));
        };
        let measured = match self {
            // The usage added, exactly, then read as a decimal (display only).
            Self::Package { .. } => Some(Self::usage_added(billed, to)?.to_decimal()),
            _ => None,
        };
        let mut out = Vec::new();
        for (b, a) in by_tier(&before, &after) {
            let (tier, unit_price) = tier_of(b, a);
            let quantity = usage_of(a).checked_sub(usage_of(b))?;
            let flat =
                exact_add(flat_of(a), crate::line::neg(flat_of(b))).ok_or(ErrorKind::Overflow)?;
            if quantity.is_zero() && flat.is_zero() {
                continue;
            }
            out.push(Piece {
                tier,
                quantity,
                unit_price,
                flat: (!flat.is_zero()).then_some(flat),
                measured,
            });
        }
        Ok((out, next))
    }

    /// The increment from what was `billed` to `to` as invoice lines at the
    /// document's `scale`, with the next state [`Billed::UpTo`]`(to)`, rounded
    /// so that the **rounded** invoices add up too: for every non-decreasing
    /// cumulative sequence rated from [`Billed::Nothing`], zeros included, the
    /// lines of all increments sum to the lines of
    /// `increment_lines(Billed::Nothing, cₙ)`, at every scale and rounding mode.
    ///
    /// Per tier, the usage line states the net
    /// `reduce(tier net at to) − reduce(tier net at from)` (nothing at `from`
    /// from `Nothing`), with the tier's slice difference as its exact quantity
    /// (as [`Line::from_usage`]) at the tier price; an unchanged tier is
    /// omitted. A flat fee newly reached (or reversed) is its own
    /// [`Line::fixed`] of `reduce(flat at to) − reduce(flat at from)`, so every
    /// stated net stays within one unit of `quantity × price`
    /// (PEPPOL-EN16931-R120's 0.02 at two decimals and finer).
    ///
    /// ```
    /// use billing::{AmountScale, Billed, Currency, Document, Price, Tariff};
    /// use rust_decimal::{Decimal, dec};
    ///
    /// // 0.4, 0.8 and 1.2 kWh cumulative at 0.015: each exact increment is 0.006.
    /// let t = Tariff::PerUnit(Price::new(dec!(0.015)));
    /// let s = AmountScale::EN16931;
    /// let (mut state, mut invoices) = (Billed::Nothing, Vec::new());
    /// for to in [dec!(0.4), dec!(0.8), dec!(1.2)] {
    ///     let (lines, next) = t.increment_lines(state, to, s, "energy", "KWH")?;
    ///     invoices.push(Document::builder(Currency::EUR, s).lines(lines).build()?.line_total().to_decimal());
    ///     state = next;
    /// }
    /// // round(0.006) − 0, round(0.012) − round(0.006), round(0.018) − round(0.012):
    /// assert_eq!(invoices, [dec!(0.01), dec!(0.00), dec!(0.01)]);
    /// assert_eq!(invoices.iter().sum::<Decimal>(), dec!(0.02)); // = round(0.018), not 0.03
    /// # Ok::<(), billing::Error>(())
    /// ```
    ///
    /// # Errors
    /// As [`rate_increment`](Self::rate_increment);
    /// [`ErrorKind::ScaleMismatch`] for a scale beyond 5 decimals.
    pub fn increment_lines(
        &self,
        billed: Billed,
        to: impl Into<Usage>,
        scale: AmountScale,
        description: impl Into<String>,
        unit: impl Into<String>,
    ) -> Result<(Vec<Line>, Billed)> {
        scale.check()?;
        let to = to.into();
        let next = Billed::UpTo(to);
        let Some((before, after)) = self.increment_ratings(billed, to)? else {
            return Ok((Vec::new(), next));
        };
        let (description, unit) = (description.into(), unit.into());
        let usage_net = |p: Option<&Piece>| -> Result<Decimal> {
            p.map_or(Ok(Decimal::ZERO), |p| {
                let q = p.quantity;
                reduce_exact(
                    &[q.numerator(), p.unit_price.value()],
                    &[q.denominator()],
                    scale,
                )
                .map(Amount::to_decimal)
            })
        };
        let flat_net = |p: Option<&Piece>| -> Result<Decimal> {
            reduce_exact(&[flat_of(p)], &[], scale).map(Amount::to_decimal)
        };
        let diff = |a: Decimal, b: Decimal| -> Result<Decimal> {
            Ok(exact_add(a, crate::line::neg(b)).ok_or(ErrorKind::Overflow)?)
        };
        let mut lines = Vec::new();
        for (b, a) in by_tier(&before, &after) {
            let (_, unit_price) = tier_of(b, a);
            let quantity = usage_of(a).checked_sub(usage_of(b))?;
            let net = diff(usage_net(a)?, usage_net(b)?)?;
            // Equal quantities in a tier rate to equal nets: an unchanged
            // quantity is an unchanged tier.
            if !quantity.is_zero() {
                lines.push(
                    Line::from_usage(
                        description.clone(),
                        quantity,
                        unit.clone(),
                        UnitPrice::new(unit_price),
                    )
                    .with_net(net),
                );
            }
            let flat = diff(flat_net(a)?, flat_net(b)?)?;
            if !flat.is_zero() {
                lines.push(Line::fixed(description.clone(), flat));
            }
        }
        Ok((lines, next))
    }
}

/// The tier and price of a pair from [`by_tier`] (at least one side is set).
fn tier_of(b: Option<&Piece>, a: Option<&Piece>) -> (usize, Price) {
    a.or(b)
        .map_or((0, Price::new(Decimal::ZERO)), |p| (p.tier, p.unit_price))
}

fn usage_of(p: Option<&Piece>) -> Usage {
    p.map_or(Usage::from(Decimal::ZERO), |p| p.quantity)
}

fn flat_of(p: Option<&Piece>) -> Decimal {
    p.and_then(|p| p.flat).unwrap_or_default()
}

/// A percentile in whole percent, `1..=100`.
///
/// Takes a `u8`, so 95 % written as `0.95` does not compile; 0 is refused.
///
/// ```compile_fail
/// let p = billing::Percentile::new(0.95);
/// ```
///
/// ```
/// use billing::{ErrorKind, Percentile};
///
/// assert_eq!(Percentile::new(95)?, Percentile::P95);
/// assert_eq!(Percentile::new(0).unwrap_err().kind(), &ErrorKind::InvalidPercentile);
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "u8", into = "u8"))]
pub struct Percentile(u8);

impl Percentile {
    /// The 95th percentile: burstable billing as transit providers settle it.
    pub const P95: Self = Self(95);

    /// The `p`-th percentile.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidPercentile`] unless `1 <= p <= 100`.
    pub fn new(p: u8) -> Result<Self> {
        if (1..=100).contains(&p) {
            Ok(Self(p))
        } else {
            Err(ErrorKind::InvalidPercentile.into())
        }
    }

    /// The percentile in percent.
    #[must_use]
    pub const fn value(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Percentile {
    type Error = Error;
    fn try_from(p: u8) -> Result<Self> {
        Self::new(p)
    }
}

impl From<Percentile> for u8 {
    fn from(p: Percentile) -> Self {
        p.0
    }
}

/// The nearest-rank percentile of `values`: sorts them ascending and returns
/// the sample at rank ⌈p/100 · n⌉ (1-based), never interpolated, so the result
/// is always a measured sample. `None` for an empty slice.
///
/// ```
/// use billing::{Percentile, nearest_rank};
/// use rust_decimal::Decimal;
///
/// let mut samples: Vec<Decimal> = (1..=100).rev().map(Decimal::from).collect();
/// assert_eq!(nearest_rank(&mut samples, Percentile::P95), Some(Decimal::from(95)));
/// assert_eq!(nearest_rank(&mut [], Percentile::P95), None);
/// ```
pub fn nearest_rank(values: &mut [Decimal], p: Percentile) -> Option<Decimal> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let n = values.len() as u128;
    let rank = (u128::from(p.0) * n).div_ceil(100);
    // 1 ≤ rank ≤ n because 1 ≤ p ≤ 100 and n ≥ 1.
    #[allow(clippy::cast_possible_truncation)]
    values.get(rank as usize - 1).copied()
}

#[cfg(feature = "serde")]
mod opt_dec {
    //! `Option<Decimal>` as an optional decimal string.

    use rust_decimal::Decimal;
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    #[serde(transparent)]
    struct Str(#[serde(with = "crate::decimal::serde_str")] Decimal);

    #[allow(clippy::ref_option)]
    pub(super) fn serialize<S: Serializer>(v: &Option<Decimal>, s: S) -> Result<S::Ok, S::Error> {
        v.map(Str).serialize(s)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        d: D,
    ) -> Result<Option<Decimal>, D::Error> {
        Ok(Option::<Str>::deserialize(d)?.map(|s| s.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    fn p(x: Decimal) -> Price {
        Price::new(x)
    }

    #[test]
    fn packages_are_exact_whatever_the_division_rounds_to() {
        let up = PackageRounding::Up;
        let down = PackageRounding::Down;
        assert_eq!(packages(dec!(0), dec!(10), up).unwrap(), dec!(0));
        assert_eq!(packages(dec!(10), dec!(10), up).unwrap(), dec!(1));
        assert_eq!(packages(dec!(10.0001), dec!(10), up).unwrap(), dec!(2));
        assert_eq!(packages(dec!(10.0001), dec!(10), down).unwrap(), dec!(1));
        assert_eq!(packages(dec!(9.9999), dec!(10), down).unwrap(), dec!(0));
        // An exact multiple is exactly that many packages.
        assert_eq!(packages(dec!(0.9), dec!(0.3), down).unwrap(), dec!(3));
        // q just below a multiple whose quotient rounds up to an integer at 28 digits.
        let size = dec!(3);
        let q = dec!(2.9999999999999999999999999999);
        assert_eq!(packages(q, size, down).unwrap(), dec!(0));
        assert_eq!(packages(q, size, up).unwrap(), dec!(1));
        assert_eq!(
            packages(dec!(1), dec!(0), up).unwrap_err().kind(),
            &ErrorKind::InvalidTiers(TierError::PackageSize)
        );
    }

    #[test]
    fn zero_flat_is_not_a_piece() {
        let tiers = Tiers::new([Tier::above(p(dec!(1))).with_flat(dec!(0))]).unwrap();
        assert!(
            Tariff::Graduated(tiers.clone())
                .rate(dec!(0))
                .unwrap()
                .is_empty()
        );
        assert!(Tariff::Volume(tiers).rate(dec!(0)).unwrap().is_empty());
    }

    #[test]
    fn index_of_uses_inclusive_bounds() {
        let tiers = Tiers::new([
            Tier::up_to(dec!(1), p(dec!(3))),
            Tier::up_to(dec!(2), p(dec!(2))),
        ])
        .unwrap();
        assert_eq!(tiers.index_of(dec!(0)).unwrap(), 0);
        assert_eq!(tiers.index_of(dec!(1)).unwrap(), 0);
        assert_eq!(tiers.index_of(dec!(1.5)).unwrap(), 1);
        assert_eq!(tiers.index_of(dec!(2)).unwrap(), 1);
        assert!(tiers.index_of(dec!(2.1)).is_err());
        assert!(tiers.index_of(dec!(-0.1)).is_err());
    }
}
