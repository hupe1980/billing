//! `split`, `split_matrix` and `Amount::split`: guarantees as properties, plus
//! concrete regressions.

use billing::{Amount, ErrorKind, Location, split, split_matrix};
use proptest::prelude::*;
use rust_decimal::{Decimal, dec};

/// A decimal at `decimals` as an integer count of units.
fn units(d: Decimal, decimals: u32) -> i128 {
    let mut d = d;
    d.rescale(decimals);
    d.mantissa()
}

fn dec_weights(w: &[u32]) -> Vec<Decimal> {
    w.iter().map(|&x| Decimal::from(x)).collect()
}

/// `|part|·Σw` is within `Σw` of `|total|·wᵢ`: the part is floor or ceil of its quota.
fn within_quota(part: i128, total: i128, wi: i128, sum: i128) -> bool {
    let lhs = part.abs() * sum;
    let q = total.abs() * wi;
    (lhs - q).abs() < sum || (sum == 0 && part == 0)
}

fn weights() -> impl Strategy<Value = Vec<u32>> {
    prop::collection::vec(0u32..50, 1..9).prop_filter("some weight", |w| w.iter().any(|&x| x > 0))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn conserves_and_respects_quota(t in -1_000_000i64..1_000_000, w in weights()) {
        let total = Decimal::new(t, 2);
        let parts = split(total, &dec_weights(&w), 2).unwrap();
        prop_assert_eq!(parts.iter().sum::<Decimal>(), total);
        let sum: i128 = w.iter().map(|&x| i128::from(x)).sum();
        for (p, &wi) in parts.iter().zip(&w) {
            prop_assert_eq!(p.scale(), 2);
            prop_assert!(within_quota(units(*p, 2), i128::from(t), i128::from(wi), sum));
            if wi == 0 {
                prop_assert!(p.is_zero());
            }
            // Sign mirrors the total: no part has the opposite sign.
            prop_assert!(p.is_zero() || p.is_sign_negative() == (t < 0));
        }
    }

    #[test]
    fn sign_mirrors(t in -1_000_000i64..1_000_000, w in weights()) {
        let ws = dec_weights(&w);
        let pos = split(Decimal::new(t, 2), &ws, 2).unwrap();
        let neg = split(Decimal::new(-t, 2), &ws, 2).unwrap();
        for (a, b) in pos.iter().zip(&neg) {
            prop_assert_eq!(*a, -*b);
        }
    }

    #[test]
    fn permutation_equivariant_for_distinct_remainders(
        t in 1i64..1_000_000,
        w in weights(),
        rot in 0usize..8,
    ) {
        let sum: i128 = w.iter().map(|&x| i128::from(x)).sum();
        let mut rems: Vec<i128> = w.iter().map(|&x| i128::from(t) * i128::from(x) % sum).collect();
        rems.sort_unstable();
        prop_assume!(rems.windows(2).all(|p| p[0] != p[1]));
        let ws = dec_weights(&w);
        let parts = split(Decimal::new(t, 2), &ws, 2).unwrap();
        let k = rot % w.len();
        let mut rotated = ws.clone();
        rotated.rotate_left(k);
        let mut expected = parts.clone();
        expected.rotate_left(k);
        prop_assert_eq!(split(Decimal::new(t, 2), &rotated, 2).unwrap(), expected.clone());
        let mut reversed = ws;
        reversed.reverse();
        let mut expected = parts;
        expected.reverse();
        prop_assert_eq!(split(Decimal::new(t, 2), &reversed, 2).unwrap(), expected);
    }

    #[test]
    fn ties_go_to_the_lowest_index(n in 2usize..10, k in 0usize..10) {
        let k = k % n;
        let total = Decimal::new(i64::try_from(n * 7 + k).unwrap(), 2);
        let parts = split(total, &vec![Decimal::ONE; n], 2).unwrap();
        for (i, p) in parts.iter().enumerate() {
            prop_assert_eq!(*p, Decimal::new(if i < k { 8 } else { 7 }, 2));
        }
    }

    #[test]
    fn decimal_weights_equal_scaled_integer_weights(t in -100_000i64..100_000, w in weights()) {
        let as_dec: Vec<Decimal> = w.iter().map(|&x| Decimal::new(i64::from(x), 3)).collect();
        prop_assert_eq!(
            split(Decimal::new(t, 2), &as_dec, 2).unwrap(),
            split(Decimal::new(t, 2), &dec_weights(&w), 2).unwrap()
        );
    }

    #[test]
    fn amount_split_is_split(raw in -1_000_000_000i64..1_000_000_000, w in weights()) {
        let a = Amount::<5>::from_raw(raw);
        let ws = dec_weights(&w);
        let via_amount: Vec<Decimal> = a.split(&ws).unwrap().into_iter().map(Amount::to_decimal).collect();
        prop_assert_eq!(via_amount, split(a.to_decimal(), &ws, 5).unwrap());
    }

    #[test]
    fn excess_precision_is_refused(t in 1i64..1_000_000, w in weights()) {
        let total = Decimal::new(t * 10 + 1, 3);
        let err = split(total, &dec_weights(&w), 2).unwrap_err();
        prop_assert_eq!(err.kind(), &ErrorKind::ExcessPrecision { value: total, decimals: 2 });
    }

    #[test]
    fn matrix_rows_and_cells_keep_split_guarantees(
        totals in prop::collection::vec(-5_000i64..5_000, 0..40),
        w in weights(),
    ) {
        let ts: Vec<Decimal> = totals.iter().map(|&t| Decimal::new(t, 2)).collect();
        let rows = split_matrix(&ts, &dec_weights(&w), 2).unwrap();
        let sum: i128 = w.iter().map(|&x| i128::from(x)).sum();
        prop_assert_eq!(rows.len(), ts.len());
        for (row, (&total, &t)) in rows.iter().zip(ts.iter().zip(&totals)) {
            prop_assert_eq!(row.iter().sum::<Decimal>(), total);
            for (p, &wi) in row.iter().zip(&w) {
                prop_assert!(within_quota(units(*p, 2), i128::from(t), i128::from(wi), sum));
                prop_assert!(p.is_zero() || p.is_sign_negative() == (t < 0));
            }
        }
    }

    /// The measured column bound: see `split_matrix`'s rustdoc. Two recipients:
    /// ≤ ½ unit (provable — the two deficits mirror each other). Up to eight:
    /// < 1.5 units (empirical; the worst seen in 2·10⁶ random and hill-climbed
    /// cases was 1.28 units).
    #[test]
    fn matrix_columns_stay_near_their_exact_share(
        totals in prop::collection::vec(-50i64..50, 1..100),
        w in prop::collection::vec(0u32..20, 2..9).prop_filter("some weight", |w| w.iter().any(|&x| x > 0)),
    ) {
        let ts: Vec<Decimal> = totals.iter().map(|&t| Decimal::from(t)).collect();
        let rows = split_matrix(&ts, &dec_weights(&w), 0).unwrap();
        let sum: i128 = w.iter().map(|&x| i128::from(x)).sum();
        let grand: i128 = totals.iter().map(|&t| i128::from(t)).sum();
        for (j, &wj) in w.iter().enumerate() {
            let column: i128 = rows.iter().map(|r| units(r[j], 0)).sum();
            // deviation × Σw, doubled to stay in integers.
            let dev2 = 2 * (column * sum - grand * i128::from(wj)).abs();
            if w.len() == 2 {
                prop_assert!(dev2 <= sum, "two recipients: > ½ unit");
            } else {
                prop_assert!(dev2 < 3 * sum, "> 1.5 units: w={w:?} totals={totals:?}");
            }
        }
    }
}

#[test]
fn alabama_paradox_is_documented_not_fixed() {
    // The total grows, recipient 2 loses a cent. Hamilton keeps quota,
    // and Balinski–Young say quota and house-monotonicity cannot both hold.
    let w = [dec!(6), dec!(6), dec!(2)];
    assert_eq!(
        split(dec!(0.10), &w, 2).unwrap(),
        [dec!(0.04), dec!(0.04), dec!(0.02)]
    );
    assert_eq!(
        split(dec!(0.11), &w, 2).unwrap(),
        [dec!(0.05), dec!(0.05), dec!(0.01)]
    );
}

#[test]
fn ten_cent_positions_do_not_all_go_to_one_recipient() {
    // Per-position Hamilton gives recipient 0 all ten cents.
    let w = [dec!(1); 3];
    let per_row: Vec<Vec<Decimal>> = (0..10).map(|_| split(dec!(0.01), &w, 2).unwrap()).collect();
    let column = |rows: &[Vec<Decimal>], j: usize| rows.iter().map(|r| r[j]).sum::<Decimal>();
    assert_eq!(column(&per_row, 0), dec!(0.10));
    let matrix = split_matrix(&[dec!(0.01); 10], &w, 2).unwrap();
    let cols = [column(&matrix, 0), column(&matrix, 1), column(&matrix, 2)];
    assert_eq!(cols, [dec!(0.04), dec!(0.03), dec!(0.03)]);
}

#[test]
fn equal_three_way_split_of_a_thousand_lines() {
    // An equal three-way allocation of 1000 × 1.00 must not drift: per-row
    // Hamilton at two decimals gives recipient 0 340.00.
    let w = [dec!(1); 3];
    let column = |rows: &[Vec<Decimal>], j: usize| rows.iter().map(|r| r[j]).sum::<Decimal>();
    for decimals in [2u8, 5] {
        let rows = split_matrix(&[dec!(1.00); 1000], &w, decimals).unwrap();
        let unit = Decimal::new(1, u32::from(decimals));
        let exact = dec!(1000) / dec!(3);
        let mut total = Decimal::ZERO;
        for j in 0..3 {
            let c = column(&rows, j);
            assert!(
                (c - exact).abs() < unit,
                "decimals {decimals}: column {j} = {c}"
            );
            total += c;
        }
        assert_eq!(total, dec!(1000));
    }
    let per_row: Vec<Vec<Decimal>> = (0..1000)
        .map(|_| split(dec!(1.00), &w, 2).unwrap())
        .collect();
    assert_eq!(column(&per_row, 0), dec!(340.00));
}

#[test]
fn tiny_positions_reach_every_recipient() {
    // 40/35/25 of 1000 × 0.00001 split per row gives [0.01, 0, 0].
    let w = [dec!(40), dec!(35), dec!(25)];
    let rows = split_matrix(&[dec!(0.00001); 1000], &w, 5).unwrap();
    let cols: Vec<Decimal> = (0..3).map(|j| rows.iter().map(|r| r[j]).sum()).collect();
    assert_eq!(cols, [dec!(0.00400), dec!(0.00350), dec!(0.00250)]);
}

#[test]
fn column_deviation_can_exceed_one_unit() {
    // Found by random search: eight recipients, 58 positive rows, worst column
    // 1.22 units from its exact share. The greedy running-deficit rule does not
    // guarantee < 1 unit for more than three recipients; see split_matrix docs.
    let w = [3u32, 3, 1, 1, 3, 3, 3, 1];
    let totals: [i64; 58] = [
        7, 6, 19, 15, 46, 25, 40, 48, 15, 50, 3, 24, 3, 3, 1, 28, 13, 2, 45, 47, 6, 32, 29, 2, 6,
        35, 30, 32, 29, 28, 19, 19, 39, 4, 36, 14, 48, 13, 24, 35, 1, 25, 1, 46, 48, 22, 20, 11,
        25, 30, 44, 26, 45, 50, 45, 27, 48, 50,
    ];
    let ts: Vec<Decimal> = totals.iter().map(|&t| Decimal::from(t)).collect();
    let rows = split_matrix(&ts, &dec_weights(&w), 0).unwrap();
    let sum: i128 = w.iter().map(|&x| i128::from(x)).sum();
    let mut worst = 0i128;
    let mut running = vec![0i128; w.len()];
    let mut grand = 0i128;
    for (row, &t) in rows.iter().zip(&totals) {
        grand += i128::from(t);
        for (j, &wj) in w.iter().enumerate() {
            running[j] += units(row[j], 0);
            worst = worst.max((running[j] * sum - grand * i128::from(wj)).abs());
        }
    }
    // worst / Σw is in (1, 1.5) units.
    assert!(worst > sum && 2 * worst < 3 * sum, "worst = {worst}/{sum}");
}

/// Named counterexample to "every column stays within one unit": eight
/// weights (two of them zero), twelve mixed-sign totals at 0 decimals. Column 6
/// ends 1.0746 units from its exact share, so for three or more recipients the
/// running-deficit rule keeps columns below 1.5 units, not below 1.
#[test]
fn split_matrix_column_can_deviate_more_than_one_unit() {
    let w = [2u32, 0, 8, 11, 11, 0, 849, 876];
    let totals: [i64; 12] = [-3, 1, 3, -1, -3, 21, 1, 2, -3, 3, 44, -1];
    let ts: Vec<Decimal> = totals.iter().map(|&t| Decimal::from(t)).collect();
    let rows = split_matrix(&ts, &dec_weights(&w), 0).unwrap();
    let sum: i64 = w.iter().map(|&x| i64::from(x)).sum();
    let grand: i64 = totals.iter().sum();
    let column = |j: usize| rows.iter().map(|r| r[j]).sum::<Decimal>();
    // deviation_j = |Σ parts_j − grand·w_j/Σw|, exactly.
    let deviation =
        |j: usize| (column(j) - Decimal::from(grand * i64::from(w[j])) / Decimal::from(sum)).abs();
    let d6 = deviation(6);
    assert_eq!(d6.round_dp(4), dec!(1.0746), "{d6}");
    assert!(d6 > Decimal::ONE && d6 < dec!(1.5));
    for j in 0..w.len() {
        assert!(deviation(j) < dec!(1.5), "column {j}: {}", deviation(j));
    }
    // Rows still conserve exactly and zero weights get nothing.
    for (row, t) in rows.iter().zip(&ts) {
        assert_eq!(row.iter().sum::<Decimal>(), *t);
        assert!(row[1].is_zero() && row[5].is_zero());
    }
}

#[test]
fn refusals() {
    let err = split(dec!(1), &[dec!(1), dec!(-0.5)], 2).unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::InvalidWeights);
    assert_eq!(err.location(), Some(&Location::Weight(1)));
    assert_eq!(
        split(dec!(1), &[], 2).unwrap_err().kind(),
        &ErrorKind::InvalidWeights
    );
    assert_eq!(
        split(dec!(1), &[Decimal::ZERO; 3], 2).unwrap_err().kind(),
        &ErrorKind::InvalidWeights
    );
    assert_eq!(
        split_matrix(&[dec!(1), dec!(0.001)], &[dec!(1)], 2)
            .unwrap_err()
            .kind(),
        &ErrorKind::ExcessPrecision {
            value: dec!(0.001),
            decimals: 2
        }
    );
    // |total|·w leaves i128: refused, not approximated.
    assert_eq!(
        split(Decimal::MAX, &[Decimal::MAX, dec!(1)], 0)
            .unwrap_err()
            .kind(),
        &ErrorKind::Overflow
    );
    // A running column sum leaves i128.
    let big = Decimal::from(1i64 << 62);
    assert_eq!(split(big, &[dec!(1), big], 0).unwrap().len(), 2);
    assert_eq!(
        split_matrix(&[big; 10], &[dec!(1), big], 0)
            .unwrap_err()
            .kind(),
        &ErrorKind::Overflow
    );
}

/// OP-5: weights that a tolerance check once accepted and then failed on a
/// larger total split exactly on both totals (no tolerance is involved).
#[test]
fn op5_near_half_weights_split_on_every_total() {
    let w = [dec!(0.4999999995), dec!(0.5)];
    assert_eq!(
        split(dec!(1000.00), &w, 2).unwrap(),
        [dec!(500.00), dec!(500.00)]
    );
    assert_eq!(
        split(dec!(1000000.00), &w, 2).unwrap(),
        [dec!(500000.00), dec!(500000.00)]
    );
}

/// S3: a 28-digit quotient on a rounding boundary. Multiply-first proration
/// gave …74999.99977 one unit off the exact share; the split must land each
/// part on the floor or ceiling of its exact quota and conserve.
#[test]
fn s3_boundary_quotient_is_exact() {
    let total = Decimal::from_i128_with_scale(8_999_999_999_749_999_973, 5);
    let w = [Decimal::from(4_000_000_006u64), Decimal::ONE];
    let parts = split(total, &w, 5).unwrap();
    assert_eq!(parts, [dec!(89999999974999.99977), dec!(22499.99996)]);
    assert_eq!(parts.iter().sum::<Decimal>(), total);
    let sum = 4_000_000_007i128;
    for (p, wi) in parts.iter().zip([4_000_000_006i128, 1]) {
        assert!(within_quota(
            units(*p, 5),
            8_999_999_999_749_999_973,
            wi,
            sum
        ));
    }
}
