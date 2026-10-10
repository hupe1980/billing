//! `split` and `split_matrix` on arbitrary totals, weights and scales.
//!
//! Asserts: no panic; on `Ok`, one part per weight, every part fits `decimals`,
//! and the parts sum to the total exactly (conservation). `split_matrix` must
//! agree with `split` row by row.
#![no_main]

use arbitrary::Arbitrary;
use billing::{split, split_matrix};
use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

#[derive(Debug, Arbitrary)]
struct Input {
    totals: Vec<(i64, u8)>,
    weights: Vec<(i64, u8)>,
    decimals: u8,
}

fn dec((mantissa, scale): (i64, u8)) -> Decimal {
    Decimal::new(mantissa, u32::from(scale % 29))
}

fn check(total: Decimal, weights: &[Decimal], decimals: u8, parts: &[Decimal]) {
    assert_eq!(parts.len(), weights.len());
    let mut sum = Decimal::ZERO;
    for p in parts {
        assert!(
            p.round_dp(u32::from(decimals)) == *p,
            "part {p} has more than {decimals} decimals"
        );
        sum = sum
            .checked_add(*p)
            .expect("parts of a representable total cannot overflow");
    }
    assert_eq!(
        sum, total,
        "Σ parts != total for weights {weights:?} at {decimals}"
    );
}

fuzz_target!(|input: Input| {
    let weights: Vec<Decimal> = input.weights.into_iter().take(64).map(dec).collect();
    let totals: Vec<Decimal> = input.totals.into_iter().take(8).map(dec).collect();
    let decimals = input.decimals % 32;
    let mut rows = Vec::new();
    for &total in &totals {
        let row = split(total, &weights, decimals);
        if let Ok(parts) = &row {
            check(total, &weights, decimals, parts);
        }
        rows.push(row);
    }
    if let Ok(matrix) = split_matrix(&totals, &weights, decimals) {
        assert_eq!(matrix.len(), totals.len());
        for ((row, &total), single) in matrix.iter().zip(&totals).zip(&rows) {
            check(total, &weights, decimals, row);
            assert!(single.is_ok(), "split_matrix accepted a row split refused");
        }
    }
});
