//! [`split`] and [`split_matrix`]: the crate's only apportionment.
//!
//! Both work on integers: the total becomes a count of `10^-decimals` units and
//! the weights become integers over their common scale, so every quota
//! `total·wᵢ/Σw` is an exact fraction of `i128`s. Nothing is approximated; an
//! input whose products leave `i128` is refused with [`ErrorKind::Overflow`].

use alloc::vec;
use alloc::vec::Vec;

use rust_decimal::Decimal;

use crate::amount::to_units;
use crate::decimal::pow10;
use crate::{Error, ErrorKind, Location, Result};

/// Weights as integers over their common scale.
struct Weights {
    ints: Vec<i128>,
    sum: i128,
}

fn weights(weights: &[Decimal]) -> Result<Weights> {
    if weights.is_empty() {
        return Err(ErrorKind::InvalidWeights.into());
    }
    let normal: Vec<Decimal> = weights.iter().map(Decimal::normalize).collect();
    for (i, w) in normal.iter().enumerate() {
        if w.is_sign_negative() && !w.is_zero() {
            return Err(Error::from(ErrorKind::InvalidWeights).at(Location::Weight(i)));
        }
    }
    let scale = normal.iter().map(Decimal::scale).max().unwrap_or(0);
    let mut ints = Vec::with_capacity(normal.len());
    let mut sum: i128 = 0;
    for (i, w) in normal.iter().enumerate() {
        let overflow = || Error::from(ErrorKind::Overflow).at(Location::Weight(i));
        let f = pow10(scale - w.scale()).ok_or_else(overflow)?;
        let int = w.mantissa().abs().checked_mul(f).ok_or_else(overflow)?;
        sum = sum.checked_add(int).ok_or_else(overflow)?;
        ints.push(int);
    }
    Ok(Weights { ints, sum })
}

/// One row: floors, remainders and the number of extra units, all for |total|.
struct Row {
    negative: bool,
    /// `|total|·wᵢ` (the quota numerator over `Σw`).
    products: Vec<i128>,
    floors: Vec<i128>,
    rems: Vec<i128>,
    extra: usize,
}

fn row(total: i128, w: &Weights) -> Result<Row> {
    let mag = total.checked_abs().ok_or(ErrorKind::Overflow)?;
    let products = w
        .ints
        .iter()
        .map(|&wi| mag.checked_mul(wi).ok_or(ErrorKind::Overflow))
        .collect::<core::result::Result<Vec<_>, _>>()?;
    let floors: Vec<i128> = products.iter().map(|p| p / w.sum).collect();
    let rems: Vec<i128> = products.iter().map(|p| p % w.sum).collect();
    // Σ floors ≤ mag, and mag − Σ floors = Σ rems / Σw < number of non-zero rems.
    let extra = mag - floors.iter().sum::<i128>();
    let extra = usize::try_from(extra).map_err(|_| ErrorKind::Overflow)?;
    Ok(Row {
        negative: total < 0,
        products,
        floors,
        rems,
        extra,
    })
}

/// Give one extra unit to the first `extra` of the candidates (non-zero
/// remainder) ordered by `key` descending, then index ascending; apply the sign.
fn finish(r: &Row, key: impl Fn(usize) -> i128) -> Vec<i128> {
    let mut order: Vec<usize> = (0..r.rems.len()).filter(|&i| r.rems[i] != 0).collect();
    order.sort_by(|&a, &b| key(b).cmp(&key(a)).then(a.cmp(&b)));
    let mut parts = r.floors.clone();
    for &i in order.iter().take(r.extra) {
        parts[i] += 1;
    }
    if r.negative {
        for p in &mut parts {
            *p = -*p;
        }
    }
    parts
}

/// Largest remainder over integer units. `total` is a count of units.
pub(crate) fn split_units(total: i128, weights_in: &[Decimal]) -> Result<Vec<i128>> {
    let w = weights(weights_in)?;
    if w.sum == 0 {
        return if total == 0 {
            Ok(vec![0; w.ints.len()])
        } else {
            Err(ErrorKind::InvalidWeights.into())
        };
    }
    let r = row(total, &w)?;
    Ok(finish(&r, |i| r.rems[i]))
}

fn units_to_decimals(units: Vec<i128>, decimals: u8) -> Result<Vec<Decimal>> {
    units
        .into_iter()
        .map(|u| {
            Decimal::try_from_i128_with_scale(u, u32::from(decimals))
                .map_err(|_| ErrorKind::Overflow.into())
        })
        .collect()
}

fn check_decimals(decimals: u8) -> Result<()> {
    if u32::from(decimals) > Decimal::MAX_SCALE {
        return Err(ErrorKind::ScaleMismatch.into());
    }
    Ok(())
}

/// Largest remainder (Hamilton) of a signed `total` by non-negative `weights` at
/// `decimals`.
///
/// Exact: integer arithmetic (`i128`) on units of `10^-decimals`; anything past
/// that bound is refused, never approximated.
///
/// Guarantees, for `q = total·wᵢ/Σw` in units:
///
/// * **Conservation**: `Σ parts == total` exactly.
/// * **Quota**: each `|partᵢ|` is `⌊|qᵢ|⌋` or `⌈|qᵢ|⌉` units.
/// * **Ties** (equal remainders) go to the **lowest index**; a zero weight gets
///   exactly zero.
/// * **Sign** mirrors the total: `split(−t) == −split(t)` element-wise.
/// * **Permutation**: permuting weights whose remainders are distinct permutes
///   the parts the same way.
///
/// **Not** house- or population-monotone, and cannot be: by Balinski–Young no
/// method has quota and monotonicity together. A larger total can give a
/// recipient less (the Alabama paradox):
///
/// ```
/// use billing::split;
/// use rust_decimal::dec;
///
/// let w = [dec!(6), dec!(6), dec!(2)];
/// assert_eq!(split(dec!(0.10), &w, 2)?, [dec!(0.04), dec!(0.04), dec!(0.02)]);
/// assert_eq!(split(dec!(0.11), &w, 2)?, [dec!(0.05), dec!(0.05), dec!(0.01)]);
/// # Ok::<(), billing::Error>(())
/// ```
///
/// So never split cumulative totals and bill the differences; split each
/// increment instead.
///
/// # Errors
/// * [`ErrorKind::ExcessPrecision`] if `total` does not fit `decimals` (it is
///   never rounded silently).
/// * [`ErrorKind::InvalidWeights`] for empty weights, a negative weight (with
///   [`Location::Weight`]), or all-zero weights with a non-zero total (all-zero
///   weights with a zero total give all zeros).
/// * [`ErrorKind::Overflow`] if `|total|·wᵢ` or `Σw` leaves `i128` (weights are
///   scaled to integers over their common number of decimals first).
/// * [`ErrorKind::ScaleMismatch`] if `decimals > 28`.
pub fn split(total: Decimal, weights: &[Decimal], decimals: u8) -> Result<Vec<Decimal>> {
    check_decimals(decimals)?;
    let units = to_units(total, decimals, None)?;
    units_to_decimals(split_units(units, weights)?, decimals)
}

/// Several `totals` split by the same `weights` ("matrix rounding"), one row per
/// total, at `decimals`.
///
/// Every row keeps [`split`]'s per-row guarantees: its parts sum to its total
/// exactly, each cell is the floor or ceiling of its own quota (so a zero
/// weight's column is all zero and a debit row never yields a credit cell), and
/// the sign mirrors the row's total. What differs is *who* gets a row's extra
/// units: the candidates (cells with a non-zero remainder) are ranked by their
/// **running deficit** — the recipient's exact cumulative share including this
/// row, minus what it has been assigned so far plus this row's floor — with ties
/// to the lowest index. For a negative row the ranking is mirrored: the extra
/// −1 goes to the recipient whose assigned total is furthest *above* its exact
/// share.
///
/// Per-row largest remainder lets the same recipient win every tie: ten rows of
/// 0.01 split three ways give it 0.10 and the others nothing. Here the column
/// totals stay close to their exact shares:
///
/// ```
/// use billing::split_matrix;
/// use rust_decimal::dec;
///
/// let rows = split_matrix(&[dec!(0.01); 10], &[dec!(1); 3], 2)?;
/// let column = |j: usize| rows.iter().map(|r| r[j]).sum::<rust_decimal::Decimal>();
/// assert_eq!([column(0), column(1), column(2)], [dec!(0.04), dec!(0.03), dec!(0.03)]);
/// # Ok::<(), billing::Error>(())
/// ```
///
/// **Column bound.** A column's deviation from its exact share
/// `Σtotals·wᵢ/Σw`, after every prefix of the rows, is at most ½ unit for two
/// recipients. For three or more there is no proven bound and it can exceed
/// one unit (measured worst case: 1.28 units; `tests/split.rs` asserts < 1.5
/// for up to 8 recipients). A strict one-unit column bound needs controlled
/// rounding over the whole matrix, which this function does not do.
///
/// # Errors
/// As [`split`], for any row; additionally [`ErrorKind::Overflow`] if a
/// recipient's running `Σ totalᵣ·wᵢ` leaves `i128`.
pub fn split_matrix(
    totals: &[Decimal],
    weights_in: &[Decimal],
    decimals: u8,
) -> Result<Vec<Vec<Decimal>>> {
    check_decimals(decimals)?;
    let w = weights(weights_in)?;
    let n = w.ints.len();
    let units = totals
        .iter()
        .map(|&t| to_units(t, decimals, None))
        .collect::<Result<Vec<_>>>()?;
    if w.sum == 0 {
        if units.iter().any(|&u| u != 0) {
            return Err(ErrorKind::InvalidWeights.into());
        }
        return units
            .iter()
            .map(|_| units_to_decimals(vec![0; n], decimals))
            .collect();
    }
    let overflow = || Error::from(ErrorKind::Overflow);
    // exact[i] = Σ totalᵣ·wᵢ (units × Σw); assigned[i] = Σ parts (units).
    let mut exact = vec![0i128; n];
    let mut assigned = vec![0i128; n];
    let mut out = Vec::with_capacity(units.len());
    for &total in &units {
        let r = row(total, &w)?;
        let sign: i128 = if r.negative { -1 } else { 1 };
        // Deficit (× Σw) in the row's direction if i gets only its floor here.
        let mut deficit = vec![0i128; n];
        for i in 0..n {
            let owed = exact[i]
                .checked_add(sign * r.products[i])
                .ok_or_else(overflow)?;
            let held = assigned[i]
                .checked_add(sign * r.floors[i])
                .and_then(|h| h.checked_mul(w.sum))
                .ok_or_else(overflow)?;
            deficit[i] = owed.checked_sub(held).ok_or_else(overflow)? * sign;
        }
        let parts = finish(&r, |i| deficit[i]);
        for i in 0..n {
            exact[i] += sign * r.products[i];
            assigned[i] += parts[i];
        }
        out.push(units_to_decimals(parts, decimals)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    #[test]
    fn hamilton_basics() {
        assert_eq!(
            split(dec!(100), &[dec!(1), dec!(1), dec!(1)], 2).unwrap(),
            [dec!(33.34), dec!(33.33), dec!(33.33)]
        );
        assert_eq!(
            split(dec!(-100), &[dec!(1), dec!(1), dec!(1)], 2).unwrap(),
            [dec!(-33.34), dec!(-33.33), dec!(-33.33)]
        );
        assert_eq!(
            split(dec!(47.10), &[dec!(40), dec!(35), dec!(25)], 2).unwrap(),
            [dec!(18.84), dec!(16.49), dec!(11.77)]
        );
        assert_eq!(
            split(dec!(1), &[dec!(0), dec!(0.5), dec!(0.25)], 0).unwrap(),
            [dec!(0), dec!(1), dec!(0)]
        );
    }

    #[test]
    fn refusals() {
        let w = [dec!(1), dec!(2)];
        assert_eq!(
            split(dec!(1.005), &w, 2).unwrap_err().kind(),
            &ErrorKind::ExcessPrecision {
                value: dec!(1.005),
                decimals: 2
            }
        );
        assert_eq!(
            split(dec!(1), &[], 2).unwrap_err().kind(),
            &ErrorKind::InvalidWeights
        );
        let neg = split(dec!(1), &[dec!(1), dec!(-1)], 2).unwrap_err();
        assert_eq!(neg.kind(), &ErrorKind::InvalidWeights);
        assert_eq!(neg.location(), Some(&Location::Weight(1)));
        assert_eq!(
            split(dec!(1), &[dec!(0), dec!(0)], 2).unwrap_err().kind(),
            &ErrorKind::InvalidWeights
        );
        assert_eq!(
            split(dec!(0), &[dec!(0), dec!(0)], 2).unwrap(),
            [dec!(0), dec!(0)]
        );
        assert_eq!(
            split(dec!(1), &w, 29).unwrap_err().kind(),
            &ErrorKind::ScaleMismatch
        );
        // 7.9e28 × 10^28-scaled weights leaves i128.
        assert_eq!(
            split(Decimal::MAX, &[Decimal::MAX, Decimal::MAX], 0)
                .unwrap_err()
                .kind(),
            &ErrorKind::Overflow
        );
        assert_eq!(
            split(
                dec!(1),
                &[dec!(1), dec!(0.0000000000000000000000000001)],
                28
            )
            .unwrap_err()
            .kind(),
            &ErrorKind::Overflow
        );
    }

    #[test]
    fn negative_zero_weight_is_zero() {
        assert_eq!(
            split(dec!(1), &[-dec!(0), dec!(1)], 0).unwrap(),
            [dec!(0), dec!(1)]
        );
    }

    #[test]
    fn matrix_zero_weights() {
        assert_eq!(
            split_matrix(&[dec!(0), dec!(0)], &[dec!(0)], 2).unwrap(),
            [[dec!(0)], [dec!(0)]]
        );
        assert_eq!(
            split_matrix(&[dec!(0), dec!(1)], &[dec!(0)], 2)
                .unwrap_err()
                .kind(),
            &ErrorKind::InvalidWeights
        );
        assert!(split_matrix(&[], &[dec!(1)], 2).unwrap().is_empty());
    }
}
