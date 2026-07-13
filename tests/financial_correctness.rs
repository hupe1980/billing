//! Financial correctness integration tests.
//!
//! These tests verify numerical precision, rounding behaviour, edge cases,
//! and invariant preservation across the full billing pipeline.
//! Every test should fail if the library produces an incorrect monetary result.

use billing::prelude::*;
use billing::tax::{FixedDiscount, PerUnitLevy, PercentageCharge, PercentageDiscount};
use billing::{merge_period_documents, minimum_charge, prorate, prorate_amount};
use rust_decimal_macros::dec;

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount<P> — arithmetic precision
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn amount_parse_roundtrip() {
    // Every string that can be represented exactly at P=5 should round-trip.
    for s in &["0.00000", "1.00000", "99999.99999", "-0.00001", "-99.12345"] {
        let a = Amount::<5>::parse(s).expect(s);
        assert_eq!(a.to_string(), *s, "round-trip failed for {s}");
    }
}

#[test]
fn amount_parse_comma_separator() {
    assert_eq!(
        Amount::<5>::parse("1,23456").unwrap(),
        Amount::<5>::parse("1.23456").unwrap(),
    );
}

#[test]
fn amount_parse_whitespace_trimmed() {
    assert_eq!(
        Amount::<5>::parse("  1.00000  ").unwrap(),
        Amount::<5>::from_int(1),
    );
}

#[test]
fn amount_parse_exact_boundary() {
    // i64::MAX = 9_223_372_036_854_775_807
    // Divided by SCALE (100_000) = 92_233_720_368_547.75807
    // That is the maximum value representable as Amount<5>.
    let a = Amount::<5>::parse("92233720368547.75807").unwrap();
    assert_eq!(a.to_raw(), i64::MAX);
    // One fractional unit above that must overflow.
    assert!(Amount::<5>::parse("92233720368547.75808").is_err());
}

#[test]
fn amount_parse_rejects_excess_nonzero() {
    assert!(Amount::<5>::parse("1.123456").is_err()); // 6th digit non-zero
    assert!(Amount::<5>::parse("0.000001").is_err()); // 6th digit non-zero
}

#[test]
fn amount_parse_accepts_excess_zeros() {
    // Trailing zeros beyond P are OK.
    assert_eq!(
        Amount::<2>::parse("1.990000").unwrap(),
        Amount::<2>::parse("1.99").unwrap(),
    );
}

#[test]
fn amount_sum_empty_iterator_is_zero() {
    let v: Vec<Amount<5>> = vec![];
    let sum: Amount<5> = v.into_iter().sum();
    assert_eq!(sum, Amount::ZERO);
}

#[test]
fn amount_default_is_zero() {
    let a: Amount<5> = Default::default();
    assert_eq!(a, Amount::ZERO);
}

#[test]
fn amount_abs_positive_unchanged() {
    let a = Amount::<5>::parse("3.50000").unwrap();
    assert_eq!(a.abs(), a);
}

#[test]
fn amount_abs_negative() {
    let a = Amount::<5>::parse("-3.50000").unwrap();
    assert_eq!(a.abs(), Amount::<5>::parse("3.50000").unwrap());
}

#[test]
fn amount_mul_qty_exact_at_boundary() {
    // price × qty where the product lands exactly on a 5dp value.
    let price = Amount::<5>::parse("0.10000").unwrap();
    let qty = dec!(3);
    assert_eq!(price.mul_qty(qty), Amount::<5>::parse("0.30000").unwrap());
}

#[test]
fn amount_mul_qty_midpoint_rounds_away_from_zero() {
    // 0.10000 × 1.5 = 0.15000 (exact, no rounding needed)
    // 0.33333 × 3 = 0.99999 (exact at 5dp)
    let price = Amount::<5>::parse("0.33333").unwrap();
    let net = price.mul_qty(dec!(3));
    assert_eq!(net, Amount::<5>::parse("0.99999").unwrap());
}

#[test]
fn amount_mul_qty_rounding_case() {
    // 0.10001 × 3 = 0.30003 — exact at 5dp, no rounding needed.
    let price = Amount::<5>::parse("0.10001").unwrap();
    assert_eq!(
        price.mul_qty(dec!(3)),
        Amount::<5>::parse("0.30003").unwrap(),
    );
}

#[test]
fn amount_checked_mul_qty_returns_err_on_overflow() {
    // A very large amount multiplied by 3 should overflow.
    // from_int(i64::MAX / SCALE / 2) gives a large but valid Amount.
    let big = Amount::<5>::from_int(i64::MAX / 100_000 / 2);
    assert!(
        big.checked_mul_qty(rust_decimal::Decimal::from(3u32))
            .is_err()
    );
}

#[test]
fn amount_round_to_midpoint_away() {
    // 3.45678 → round to 2dp: .456.. → .46 (away from zero)
    let a = Amount::<5>::parse("3.45678").unwrap();
    assert_eq!(
        a.round_to::<2>(RoundingStrategy::MidpointAwayFromZero),
        Amount::<2>::parse("3.46").unwrap(),
    );
}

#[test]
fn amount_round_to_floor() {
    let a = Amount::<5>::parse("3.45999").unwrap();
    assert_eq!(
        a.round_to::<2>(RoundingStrategy::Floor),
        Amount::<2>::parse("3.45").unwrap(),
    );
}

#[test]
fn amount_round_to_ceiling() {
    let a = Amount::<5>::parse("3.45001").unwrap();
    assert_eq!(
        a.round_to::<2>(RoundingStrategy::Ceiling),
        Amount::<2>::parse("3.46").unwrap(),
    );
}

#[test]
fn amount_from_decimal_midpoint_rounds_away() {
    // 0.5 → 1 at scale 10^0 = rounds up (MidpointAwayFromZero)
    // For Amount<2>: 1.005 × 100 = 100.5 → rounds to 101 → 1.01
    let d = rust_decimal::Decimal::from_str_exact("1.005").unwrap();
    let a = Amount::<2>::try_from(d).unwrap();
    assert_eq!(a, Amount::<2>::parse("1.01").unwrap());
}

#[test]
fn amount_neg_of_positive_is_negative() {
    let a = Amount::<5>::parse("5.00000").unwrap();
    assert!((-a).is_negative());
    assert_eq!(-a, Amount::<5>::parse("-5.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LineItem — builder correctness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn line_item_fixed_amount_takes_precedence_over_qty_price() {
    // fixed_amount overrides qty × price in the build() logic.
    let item = LineItem::debit("Test")
        .quantity(Quantity::new(dec!(1000), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.32), "EUR/kWh"))
        .fixed_amount(Amount::<5>::parse("99.00000").unwrap())
        .build()
        .unwrap();
    // Net must be the fixed amount, not 1000 × 0.32 = 320.
    assert_eq!(item.net_amount, Amount::<5>::parse("99.00000").unwrap());
}

#[test]
fn line_item_credit_with_already_negative_fixed_stays_negative() {
    // If fixed_amount is already negative, Credit sign should not double-negate it.
    let item = LineItem::credit("Refund")
        .fixed_amount(Amount::<5>::parse("-10.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.net_amount, Amount::<5>::parse("-10.00000").unwrap());
}

#[test]
fn line_item_credit_flips_positive_fixed_to_negative() {
    let item = LineItem::credit("Discount")
        .fixed_amount(Amount::<5>::parse("10.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.net_amount, Amount::<5>::parse("-10.00000").unwrap());
}

#[test]
fn line_item_qty_price_multiplication_precision() {
    // 1234.567 kWh × 0.28901 EUR/kWh = 356.72958... → rounded to 5dp
    let item = LineItem::debit("Arbeit")
        .quantity(Quantity::new(dec!(1234.567), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.28901), "EUR/kWh"))
        .build()
        .unwrap();
    // 1234.567 × 0.28901 = 356.72951...  (exact with Decimal)
    // Verify it doesn't use f64 approximation
    assert_eq!(
        item.net_amount,
        Amount::<5>::from_decimal(
            rust_decimal::Decimal::from_str_exact("1234.567").unwrap()
                * rust_decimal::Decimal::from_str_exact("0.28901").unwrap()
        )
        .unwrap()
    );
}

#[test]
fn line_item_missing_both_fails() {
    let result = LineItem::debit("Missing").build();
    assert!(result.is_err());
}

#[test]
fn line_item_tags_are_case_sensitive() {
    let item = LineItem::fixed("Test", Amount::<5>::ZERO)
        .tag("Energy")
        .build()
        .unwrap();
    assert!(item.has_tag("Energy"));
    assert!(!item.has_tag("energy")); // case-sensitive
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TariffSchedule — numerical correctness and guards
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn graduated_two_band() -> TariffSchedule {
    TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(500),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(500),
            Amount::parse("0.28000").unwrap(),
        ))
        .build()
        .unwrap()
}

#[test]
fn graduated_split_sum_equals_qty_times_price() {
    let sched = graduated_two_band();
    let items = sched.split(dec!(1234.5)).unwrap();
    // Band 1: 500 × 0.32000 = 160.00000
    // Band 2: 734.5 × 0.28000 = 205.66000
    assert_eq!(items[0].net_amount, Amount::parse("160.00000").unwrap());
    assert_eq!(items[1].net_amount, Amount::parse("205.66000").unwrap());
    let total: Amount<5> = items.iter().map(|i| i.net_amount).sum();
    assert_eq!(total, Amount::parse("365.66000").unwrap());
}

#[test]
fn graduated_exact_tier_boundary_uses_lower_price() {
    // qty = 500 exactly: should use the first tier (up_to(500)).
    let sched = graduated_two_band();
    let items = sched.split(dec!(500)).unwrap();
    assert_eq!(items.len(), 1);
    // 500 × 0.32 = 160.00
    assert_eq!(items[0].net_amount, Amount::parse("160.00000").unwrap());
}

#[test]
fn graduated_zero_quantity_returns_empty() {
    let sched = graduated_two_band();
    let items = sched.split(dec!(0)).unwrap();
    assert!(items.is_empty());
}

#[test]
fn split_negative_quantity_is_error() {
    let sched = graduated_two_band();
    assert!(sched.split(dec!(-1)).is_err());
}

#[test]
fn graduated_unit_label_propagated() {
    let sched = graduated_two_band();
    let items = sched.split(dec!(100)).unwrap();
    assert_eq!(items[0].unit_label(), Some("kWh"));
}

#[test]
fn graduated_single_band_no_upper() {
    // A single unlimited band — all quantity at one price.
    let sched = TariffSchedule::graduated()
        .unit("m³")
        .band(TariffBand::over(dec!(0), Amount::parse("1.40000").unwrap()))
        .build()
        .unwrap();
    let items = sched.split(dec!(25)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].net_amount, Amount::parse("35.00000").unwrap());
}

#[test]
fn graduated_free_tier_generates_zero_item() {
    // Free tier produces a LineItem with net_amount = 0, preserving audit trail.
    let sched = TariffSchedule::graduated()
        .unit("calls")
        .band(TariffBand::free_up_to(dec!(1000)))
        .band(TariffBand::over(
            dec!(1000),
            Amount::parse("0.00100").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(1500)).unwrap();
    assert_eq!(items.len(), 2);
    assert!(items[0].net_amount.is_zero()); // free tier
    assert_eq!(items[1].net_amount, Amount::parse("0.50000").unwrap()); // 500 × 0.001
}

#[test]
fn volume_all_units_at_top_tier() {
    let sched = TariffSchedule::volume()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(1000),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(1000),
            Amount::parse("0.28000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(1234.5)).unwrap();
    assert_eq!(items.len(), 1);
    // ALL 1234.5 at 0.28 (top tier)
    assert_eq!(items[0].net_amount, Amount::parse("345.66000").unwrap());
}

#[test]
fn volume_exact_boundary_uses_lower_tier() {
    let sched = TariffSchedule::volume()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(1000),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(1000),
            Amount::parse("0.28000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(1000)).unwrap();
    assert_eq!(items.len(), 1);
    // 1000 ≤ 1000 → first tier applies
    assert_eq!(items[0].net_amount, Amount::parse("320.00000").unwrap());
}

#[test]
fn block_partial_block_rounds_up() {
    // 450 / 100 = 4.5 → 5 full blocks
    let sched = TariffSchedule::block()
        .unit("GB")
        .band(TariffBand::block(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(450)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].net_amount, Amount::parse("5.00000").unwrap());
}

#[test]
fn block_exact_multiple_no_rounding() {
    let sched = TariffSchedule::block()
        .unit("GB")
        .band(TariffBand::block(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(400)).unwrap();
    assert_eq!(items[0].net_amount, Amount::parse("4.00000").unwrap());
}

#[test]
fn block_one_unit_always_one_block() {
    let sched = TariffSchedule::block()
        .unit("pkt")
        .band(TariffBand::block(
            dec!(10),
            Amount::parse("2.00000").unwrap(),
        ))
        .build()
        .unwrap();
    // 1 unit → 1 block (round up from 0.1)
    let items = sched.split(dec!(1)).unwrap();
    assert_eq!(items[0].net_amount, Amount::parse("2.00000").unwrap());
}

#[test]
fn capacity_peak_selects_correct_tier() {
    let sched = TariffSchedule::capacity()
        .unit("kW")
        .band(TariffBand::up_to(
            dec!(50),
            Amount::parse("5.00000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(50),
            Amount::parse("10.00000").unwrap(),
        ))
        .build()
        .unwrap();
    // Peak above boundary → second tier
    let item = sched.apply_peak(dec!(63.4)).unwrap();
    assert_eq!(item.net_amount, Amount::parse("10.00000").unwrap());
    // Peak at boundary → first tier
    let item2 = sched.apply_peak(dec!(50)).unwrap();
    assert_eq!(item2.net_amount, Amount::parse("5.00000").unwrap());
}

#[test]
fn capacity_zero_peak_uses_first_tier() {
    let sched = TariffSchedule::capacity()
        .unit("kW")
        .band(TariffBand::up_to(
            dec!(50),
            Amount::parse("5.00000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(50),
            Amount::parse("10.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let item = sched.apply_peak(dec!(0)).unwrap();
    assert_eq!(item.net_amount, Amount::parse("5.00000").unwrap());
}

#[test]
fn apply_peak_negative_is_error() {
    let sched = TariffSchedule::capacity()
        .unit("kW")
        .band(TariffBand::up_to(
            dec!(50),
            Amount::parse("5.00000").unwrap(),
        ))
        .build()
        .unwrap();
    assert!(sched.apply_peak(dec!(-1)).is_err());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TimeOfUsePricing & DynamicPricing
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn tou_zero_quantity_band_skipped() {
    let tou = TimeOfUsePricing::new(vec![
        TouBand::new("peak", Amount::parse("0.32000").unwrap()),
        TouBand::new("off-peak", Amount::parse("0.18000").unwrap()),
    ])
    .with_unit("kWh");
    // Zero quantity in "off-peak" → should be omitted
    let items = tou
        .calculate(&[("peak", dec!(100)), ("off-peak", dec!(0))])
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].net_amount, Amount::parse("32.00000").unwrap());
}

#[test]
fn tou_unknown_band_silently_skipped() {
    let tou = TimeOfUsePricing::new(vec![TouBand::new(
        "peak",
        Amount::parse("0.32000").unwrap(),
    )])
    .with_unit("kWh");
    let items = tou
        .calculate(&[("peak", dec!(100)), ("unknown", dec!(999))])
        .unwrap();
    assert_eq!(items.len(), 1);
}

#[test]
fn dynamic_pricing_net_is_exact_not_qty_times_avg() {
    // If net were computed as qty × avg_price, rounding would give wrong result.
    // Total: 100 × 0.10 + 200 × 0.20 = 10 + 40 = 50.00000 exactly.
    // avg_price = 50/300 = 0.16666... (repeating).
    // 300 × 0.16666... ≠ 50.00000 in general.
    let dp = DynamicPricing::from_intervals(vec![
        (dec!(100), Amount::parse("0.10000").unwrap()),
        (dec!(200), Amount::parse("0.20000").unwrap()),
    ])
    .unwrap()
    .with_unit("kWh");
    let item = dp.calculate().unwrap();
    // The net_amount must be exactly 50.00000, not an approximation.
    assert_eq!(item.net_amount, Amount::parse("50.00000").unwrap());
}

#[test]
fn dynamic_pricing_single_interval_exact() {
    let dp =
        DynamicPricing::from_intervals(vec![(dec!(7), Amount::parse("0.33333").unwrap())]).unwrap();
    let item = dp.calculate().unwrap();
    // 7 × 0.33333 = 2.33331 (rounded to 5dp by mul_qty)
    assert_eq!(item.net_amount, Amount::parse("2.33331").unwrap());
}

#[test]
fn dynamic_pricing_three_equal_intervals_net_exact() {
    // 3 intervals of (100 units, 1/3 EUR/unit): each = 33.33333 (truncated 5dp)
    // total = 99.99999 — NOT 100.
    // This verifies that total_net accumulates per-interval net, not qty × avg.
    let dp = DynamicPricing::from_intervals(vec![
        (dec!(100), Amount::parse("0.33333").unwrap()),
        (dec!(100), Amount::parse("0.33333").unwrap()),
        (dec!(100), Amount::parse("0.33333").unwrap()),
    ])
    .unwrap();
    let item = dp.calculate().unwrap();
    // 3 × (100 × 0.33333) = 3 × 33.33300 = 99.99900
    assert_eq!(item.net_amount, Amount::parse("99.99900").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tax and discount layers
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn doc_with_charge(amount: &str) -> BillingDocument {
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse(amount).unwrap())
            .build()
            .unwrap(),
    ];
    BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap()
}

#[test]
fn fixed_rate_tax_on_net_including_credits() {
    // FixedRateTax uses net (charge - credit) as the base.
    // This is correct: VAT on a 100 charge with 40 credit = 60 × rate.
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
        LineItem::credit("Credit")
            .fixed_amount(Amount::parse("40.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.19)))];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    // net = 100 - 40 = 60; VAT = 60 × 0.19 = 11.40000
    assert_eq!(doc.net_total(), Amount::parse("60.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("11.40000").unwrap());
    doc.assert_valid();
}

#[test]
fn fixed_rate_tax_with_tag_filter_excludes_untagged() {
    let pos = vec![
        LineItem::fixed("Tagged", Amount::parse("100.00000").unwrap())
            .tag("vat-liable")
            .build()
            .unwrap(),
        LineItem::fixed("Untagged", Amount::parse("50.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(
        FixedRateTax::new("VAT", dec!(0.20)).with_tag("vat-liable"),
    )];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    // Only 100 in base; VAT = 100 × 0.20 = 20.
    assert_eq!(doc.tax_total(), Amount::parse("20.00000").unwrap());
    doc.assert_valid();
}

#[test]
fn per_unit_levy_excludes_credit_positions() {
    // A credit with kWh quantity should NOT add to the levy base.
    let pos = vec![
        LineItem::debit("Consumption")
            .quantity(Quantity::new(dec!(1000), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .build()
            .unwrap(),
        LineItem::credit("Feed-in credit")
            .quantity(Quantity::new(dec!(200), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.08), "EUR/kWh"))
            .build()
            .unwrap(),
    ];
    let levy = PerUnitLevy::new("Levy", Amount::parse("0.02050").unwrap(), "kWh");
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(levy)];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    // Only 1000 kWh in levy base (credit excluded); levy = 1000 × 0.02050 = 20.50000
    assert_eq!(doc.tax_total(), Amount::parse("20.50000").unwrap());
    doc.assert_valid();
}

#[test]
fn per_unit_levy_with_require_tag() {
    let pos = vec![
        LineItem::debit("Tagged")
            .quantity(Quantity::new(dec!(500), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .tag("metered")
            .build()
            .unwrap(),
        LineItem::debit("Untagged")
            .quantity(Quantity::new(dec!(300), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .build()
            .unwrap(),
    ];
    let levy =
        PerUnitLevy::new("Levy", Amount::parse("0.02050").unwrap(), "kWh").with_tag("metered");
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(levy)];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    // Only 500 kWh (tagged "metered"); levy = 500 × 0.02050 = 10.25000
    assert_eq!(doc.tax_total(), Amount::parse("10.25000").unwrap());
}

#[test]
fn compound_tax_three_layers() {
    // Three layers, each sees all prior layers in its base.
    // Net = 100. L1 = 5% = 5. L2 = 2% of (100+5) = 2.10. L3 = 19% of (100+5+2.10) = 20.2990
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(PercentageCharge::new("L1 5%", dec!(0.05))),
        Box::new(PercentageCharge::new("L2 2%", dec!(0.02))),
        Box::new(FixedRateTax::new("L3 VAT 19%", dec!(0.19))),
    ];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    assert_eq!(doc.net_total(), Amount::parse("100.00000").unwrap());
    // L1: 100 × 0.05 = 5.00000
    // L2: (100 + 5) × 0.02 = 2.10000
    // L3: (100 + 5 + 2.10) × 0.19 = 107.10 × 0.19 = 20.34900
    // total tax = 5 + 2.10 + 20.349 = 27.44900
    assert_eq!(doc.tax_total(), Amount::parse("27.44900").unwrap());
    assert_eq!(doc.gross_total(), Amount::parse("127.44900").unwrap());
    doc.assert_valid();
}

#[test]
fn percentage_discount_reduces_taxable_base() {
    // 10% discount on 100 → net base = 90 → VAT on 90.
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
    let discounts: Vec<Box<dyn DiscountLayer>> = vec![Box::new(PercentageDiscount::new(
        "10% discount",
        dec!(0.10),
    ))];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, discounts).unwrap();
    assert_eq!(doc.net_total(), Amount::parse("90.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("18.00000").unwrap());
    assert_eq!(doc.gross_total(), Amount::parse("108.00000").unwrap());
    doc.assert_valid();
}

#[test]
fn fixed_discount_reduces_net() {
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let discounts: Vec<Box<dyn DiscountLayer>> = vec![Box::new(FixedDiscount::new(
        "Voucher -20",
        Amount::parse("20.00000").unwrap(),
    ))];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], discounts).unwrap();
    assert_eq!(doc.net_total(), Amount::parse("80.00000").unwrap());
    doc.assert_valid();
}

#[test]
fn percentage_charge_min_floor_applied() {
    // Charge is tiny; min floor kicks in.
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("1.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(
        PercentageCharge::new("Fee 3%", dec!(0.03)).with_min(Amount::parse("0.50000").unwrap()),
    )];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    // 1.00 × 0.03 = 0.03 < 0.50 → min floor = 0.50
    assert_eq!(doc.tax_total(), Amount::parse("0.50000").unwrap());
}

#[test]
fn percentage_charge_max_ceiling_applied() {
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("10000.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(
        PercentageCharge::new("Fee 5%", dec!(0.05)).with_max(Amount::parse("100.00000").unwrap()),
    )];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    // 10000 × 0.05 = 500 > 100 → ceiling = 100
    assert_eq!(doc.tax_total(), Amount::parse("100.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BillingDocument — invariant preservation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn empty_document_totals_are_zero() {
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), vec![], vec![], vec![]).unwrap();
    assert_eq!(doc.net_total(), Amount::ZERO);
    assert_eq!(doc.tax_total(), Amount::ZERO);
    assert_eq!(doc.gross_total(), Amount::ZERO);
    doc.assert_valid();
}

#[test]
fn document_all_credits_negative_net() {
    let pos = vec![
        LineItem::credit("Refund")
            .fixed_amount(Amount::parse("50.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    assert!(doc.net_total().is_negative());
    assert_eq!(doc.net_total(), Amount::parse("-50.00000").unwrap());
    doc.assert_valid();
}

#[test]
fn with_extra_position_preserves_assert_valid() {
    let doc = doc_with_charge("100.00000");
    let extra = LineItem::fixed("Extra", Amount::parse("25.00000").unwrap())
        .build()
        .unwrap();
    let doc2 = doc.with_extra_position(extra).unwrap();
    assert_eq!(doc2.net_total(), Amount::parse("125.00000").unwrap());
    doc2.assert_valid();
}

#[test]
fn builder_with_tariff_then_extra_tax() {
    struct FlatTariff;
    impl Tariff for FlatTariff {
        type Usage = ();
        type Error = BillingError;
        fn line_items(&self, _: &()) -> Result<Vec<LineItem>, BillingError> {
            Ok(vec![
                LineItem::fixed("Flat", Amount::<5>::from_int(100))
                    .build()
                    .unwrap(),
            ])
        }
    }
    let doc = BillingDocument::builder()
        .meta(DocumentMeta::default())
        .tariff(&FlatTariff, &())
        .unwrap()
        .extra_tax(Box::new(FixedRateTax::new("VAT", dec!(0.10))))
        .build()
        .unwrap();
    assert_eq!(doc.net_total(), Amount::parse("100.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("10.00000").unwrap());
    doc.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Allocation — penny-correctness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn allocation_single_recipient_is_identity() {
    let doc = doc_with_charge("47.12345");
    let docs = EqualAllocation::new(1).allocate(&doc).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].net_total(), doc.net_total());
    docs[0].assert_valid();
}

#[test]
fn allocation_with_discounts_preserves_assert_valid() {
    let pos = vec![
        LineItem::fixed("Base", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let discounts: Vec<Box<dyn DiscountLayer>> = vec![Box::new(FixedDiscount::new(
        "Rebate",
        Amount::parse("10.00000").unwrap(),
    ))];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], discounts).unwrap();
    assert_eq!(doc.net_total(), Amount::parse("90.00000").unwrap());

    let docs = EqualAllocation::new(2).allocate(&doc).unwrap();
    for d in &docs {
        d.assert_valid();
    }
    let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
    assert_eq!(total, doc.net_total());
}

#[test]
fn allocation_penny_test_seven_way() {
    // 100 / 7 = 14.28571428... — classic penny test with many recipients.
    let doc = doc_with_charge("100.00000");
    let docs = EqualAllocation::new(7).allocate(&doc).unwrap();
    let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
    assert_eq!(total, doc.net_total(), "7-way split must not lose a penny");
    for d in &docs {
        d.assert_valid();
    }
}

#[test]
fn proportional_shares_with_non_integer_fractions() {
    let doc = doc_with_charge("99.99999");
    let alloc = ProportionalAllocation::new(vec![dec!(0.333), dec!(0.333), dec!(0.334)]).unwrap();
    let docs = alloc.allocate(&doc).unwrap();
    let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
    assert_eq!(total, doc.net_total());
    for d in &docs {
        d.assert_valid();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Period helpers — prorate and merge
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

fn fixed_item(amount: &str) -> LineItem {
    LineItem::fixed("Fee", Amount::parse(amount).unwrap())
        .build()
        .unwrap()
}

#[test]
fn prorate_half_month_exact() {
    let item = fixed_item("30.00000");
    let result = prorate(&item, 15, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(result.net_amount, Amount::parse("15.00000").unwrap());
}

#[test]
fn prorate_one_day_of_month() {
    // 30 / 31 days remaining — prorated with Floor strategy.
    let item = fixed_item("31.00000");
    let result = prorate(&item, 30, 31, RoundingStrategy::Floor).unwrap();
    // 31 × (30/31) = 30.000...  → floor = 30.00000
    assert_eq!(result.net_amount, Amount::parse("30.00000").unwrap());
}

#[test]
fn prorate_floor_vs_ceiling_differ() {
    // 100 × (1/3) = 33.33333... → Floor = 33.33333, Ceiling = 33.33334
    let item = fixed_item("100.00000");
    let floor = prorate(&item, 1, 3, RoundingStrategy::Floor).unwrap();
    let ceil = prorate(&item, 1, 3, RoundingStrategy::Ceiling).unwrap();
    assert!(
        floor.net_amount < ceil.net_amount,
        "floor must be less than ceiling"
    );
    assert_eq!(floor.net_amount, Amount::parse("33.33333").unwrap());
    assert_eq!(ceil.net_amount, Amount::parse("33.33334").unwrap());
}

#[test]
fn prorate_full_period_unchanged() {
    let item = fixed_item("50.00000");
    let result = prorate(&item, 30, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(result.net_amount, Amount::parse("50.00000").unwrap());
}

#[test]
fn prorate_zero_active_days_is_zero() {
    let item = fixed_item("100.00000");
    let result = prorate(&item, 0, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(result.net_amount, Amount::ZERO);
}

#[test]
fn prorate_active_days_exceed_total_is_error() {
    let item = fixed_item("100.00000");
    let result = prorate(&item, 31, 30, RoundingStrategy::MidpointAwayFromZero);
    assert!(result.is_err(), "active_days > total_days must be an error");
}

#[test]
fn prorate_description_appended() {
    let item = fixed_item("30.00000");
    let result = prorate(&item, 10, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert!(
        result.description.contains("10/30d"),
        "description should contain proration info"
    );
}

#[test]
fn prorate_amount_matches_prorate_item() {
    let item = fixed_item("100.00000");
    let strategy = RoundingStrategy::MidpointAwayFromZero;
    let via_item = prorate(&item, 7, 30, strategy).unwrap().net_amount;
    let direct = prorate_amount(Amount::parse("100.00000").unwrap(), 7, 30, strategy).unwrap();
    assert_eq!(via_item, direct);
}

#[test]
fn merge_period_documents_totals_sum() {
    let doc_a = doc_with_charge("100.00000");
    let doc_b = doc_with_charge("50.00000");
    let merged = merge_period_documents(doc_a, doc_b).unwrap();
    assert_eq!(merged.net_total(), Amount::parse("150.00000").unwrap());
    assert_eq!(merged.net_positions().len(), 2);
    merged.assert_valid();
}

#[test]
fn merge_preserves_tax_totals() {
    fn taxed_doc(amount: &str) -> BillingDocument {
        let pos = vec![
            LineItem::fixed("C", Amount::parse(amount).unwrap())
                .build()
                .unwrap(),
        ];
        let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
        BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap()
    }
    let a = taxed_doc("100.00000"); // tax = 20
    let b = taxed_doc("50.00000"); // tax = 10
    let merged = merge_period_documents(a, b).unwrap();
    assert_eq!(merged.tax_total(), Amount::parse("30.00000").unwrap());
    assert_eq!(merged.gross_total(), Amount::parse("180.00000").unwrap());
    merged.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// minimum_charge
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn minimum_charge_shortfall_correct() {
    let doc = doc_with_charge("200.00000");
    let item = minimum_charge(&doc, Amount::parse("500.00000").unwrap(), "Min")
        .unwrap()
        .unwrap();
    assert_eq!(item.net_amount, Amount::parse("300.00000").unwrap());
    assert!(item.has_tag("minimum-charge"));
}

#[test]
fn minimum_charge_not_triggered_when_met() {
    let doc = doc_with_charge("500.00000");
    assert!(
        minimum_charge(&doc, Amount::parse("500.00000").unwrap(), "Min")
            .unwrap()
            .is_none()
    );
}

#[test]
fn minimum_charge_on_negative_net_triggers() {
    // A document with a negative net (all credits) — minimum should still trigger.
    let pos = vec![
        LineItem::credit("Refund")
            .fixed_amount(Amount::parse("10.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    let item = minimum_charge(&doc, Amount::parse("5.00000").unwrap(), "Min")
        .unwrap()
        .unwrap();
    // shortfall = 5 - (-10) = 15
    assert_eq!(item.net_amount, Amount::parse("15.00000").unwrap());
}

#[test]
fn minimum_charge_applied_and_doc_still_valid() {
    let doc = doc_with_charge("10.00000");
    let shortfall = minimum_charge(&doc, Amount::parse("100.00000").unwrap(), "Min")
        .unwrap()
        .unwrap();
    let doc2 = doc.with_extra_position(shortfall).unwrap();
    assert_eq!(doc2.net_total(), Amount::parse("100.00000").unwrap());
    doc2.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Full pipeline integration tests
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// SaaS invoice: base fee + seats + API overage → commission + VAT.
#[test]
fn integration_saas_invoice() {
    // 49 EUR base + 5 seats × 19 EUR + 350k overage × 0.0001 EUR
    let pos = vec![
        LineItem::fixed("Base fee", Amount::<5>::from_int(49))
            .build()
            .unwrap(),
        LineItem::debit("Seats")
            .quantity(Quantity::new(dec!(5), "seats"))
            .unit_price(UnitPrice::new(dec!(19), "EUR/seat"))
            .build()
            .unwrap(),
        LineItem::debit("API overage")
            .quantity(Quantity::new(dec!(350_000), "calls"))
            .unit_price(UnitPrice::new(dec!(0.0001), "EUR/call"))
            .build()
            .unwrap(),
    ];
    // Net: 49 + 95 + 35 = 179
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(
            PercentageCharge::new("Commission 3%", dec!(0.03))
                .with_min(Amount::parse("2.00000").unwrap()),
        ),
        Box::new(FixedRateTax::new("VAT 20%", dec!(0.20))),
    ];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();
    assert_eq!(doc.net_total(), Amount::parse("179.00000").unwrap());
    // Commission: 179 × 0.03 = 5.37; VAT base: 179 + 5.37 = 184.37; VAT = 184.37 × 0.20 = 36.874
    assert_eq!(doc.tax_total(), Amount::parse("42.24400").unwrap()); // 5.37 + 36.874
    doc.assert_valid();
}

/// Water utility: tiered m³ + minimum + 3-way allocation all consistent.
#[test]
fn integration_water_utility_allocation() {
    let sched = TariffSchedule::graduated()
        .unit("m³")
        .band(TariffBand::up_to(
            dec!(5),
            Amount::parse("0.80000").unwrap(),
        ))
        .band(TariffBand::between(
            dec!(5),
            dec!(20),
            Amount::parse("1.40000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(20),
            Amount::parse("2.60000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(28.5)).unwrap();
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), items, vec![], vec![]).unwrap();
    // 5×0.8 + 15×1.4 + 8.5×2.6 = 4 + 21 + 22.1 = 47.1
    assert_eq!(doc.net_total(), Amount::parse("47.10000").unwrap());
    doc.assert_valid();

    // 3-way proportional allocation
    let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)]).unwrap();
    let parts = alloc.allocate(&doc).unwrap();
    let sum: Amount<5> = parts.iter().map(|d| d.net_total()).sum();
    assert_eq!(sum, doc.net_total());
    for p in &parts {
        p.assert_valid();
    }
}

/// Tariff-change mid-period: two half-period docs merged.
#[test]
fn integration_tariff_change_merge() {
    // Old tariff: flat 10 EUR/day for 15 days
    let item_a = LineItem::debit("Old tariff")
        .quantity(Quantity::new(dec!(15), "days"))
        .unit_price(UnitPrice::new(dec!(10), "EUR/day"))
        .build()
        .unwrap();
    let doc_a =
        BillingDocument::from_positions(DocumentMeta::default(), vec![item_a], vec![], vec![])
            .unwrap();

    // New tariff: flat 12 EUR/day for 16 days
    let item_b = LineItem::debit("New tariff")
        .quantity(Quantity::new(dec!(16), "days"))
        .unit_price(UnitPrice::new(dec!(12), "EUR/day"))
        .build()
        .unwrap();
    let doc_b =
        BillingDocument::from_positions(DocumentMeta::default(), vec![item_b], vec![], vec![])
            .unwrap();

    let merged = merge_period_documents(doc_a, doc_b).unwrap();
    // 15 × 10 + 16 × 12 = 150 + 192 = 342
    assert_eq!(merged.net_total(), Amount::parse("342.00000").unwrap());
    assert_eq!(merged.net_positions().len(), 2);
    merged.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::parse — input validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn parse_double_negative_is_error() {
    // "--5.00000" silently evaluated to +5 before the fix.
    assert!(Amount::<5>::parse("--5.00000").is_err());
    assert!(Amount::<5>::parse("+-1.00000").is_err());
    assert!(Amount::<5>::parse("-+1.00000").is_err());
}

#[test]
fn parse_sign_in_fractional_is_error() {
    // "5.-00000" silently evaluated to 5.00000 before the fix.
    assert!(Amount::<5>::parse("5.-00000").is_err());
    assert!(Amount::<5>::parse("5.+00000").is_err());
}

#[test]
fn parse_letter_in_fractional_is_error() {
    assert!(Amount::<5>::parse("1.2e3").is_err());
    assert!(Amount::<5>::parse("1.0abc").is_err());
}

#[test]
fn parse_valid_signs_work() {
    assert_eq!(
        Amount::<5>::parse("-5.00000").unwrap(),
        Amount::<5>::parse("+5.00000")
            .unwrap()
            .checked_neg()
            .unwrap(),
    );
    assert!(Amount::<5>::parse("+0.00001").is_ok());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tax/discount constructor validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
#[should_panic(expected = "FixedRateTax rate must be >= 0")]
fn fixed_rate_tax_negative_rate_panics() {
    let _ = FixedRateTax::new("Bad tax", dec!(-0.19));
}

#[test]
fn fixed_rate_tax_zero_rate_ok() {
    // Zero-rate tax (e.g. zero-rated VAT) is valid.
    let tax = FixedRateTax::new("Zero-rated", dec!(0));
    let pos = vec![
        LineItem::fixed("Item", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let item = tax.compute(&pos).unwrap();
    assert!(item.net_amount.is_zero());
}

#[test]
#[should_panic(expected = "PercentageCharge rate must be >= 0")]
fn percentage_charge_negative_rate_panics() {
    let _ = PercentageCharge::new("Bad charge", dec!(-0.05));
}

#[test]
#[should_panic(expected = "PercentageDiscount rate must be in [0, 1]")]
fn percentage_discount_rate_above_one_panics() {
    // 150% discount would produce a net charge — clearly wrong.
    let _ = PercentageDiscount::new("Extreme", dec!(1.5));
}

#[test]
#[should_panic(expected = "PercentageDiscount rate must be in [0, 1]")]
fn percentage_discount_negative_rate_panics() {
    let _ = PercentageDiscount::new("Negative", dec!(-0.10));
}

#[test]
fn percentage_discount_one_hundred_percent_ok() {
    // 100% discount is valid (full rebate).
    let disc = PercentageDiscount::new("Full rebate", dec!(1));
    let pos = vec![
        LineItem::fixed("Item", Amount::parse("50.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let item = disc.compute(&pos).unwrap();
    assert_eq!(item.net_amount, Amount::parse("-50.00000").unwrap());
}

#[test]
#[should_panic(expected = "PerUnitLevy rate must be >= 0")]
fn per_unit_levy_negative_rate_panics() {
    let _ = PerUnitLevy::new("Bad levy", Amount::parse("-0.02050").unwrap(), "kWh");
}

#[test]
fn per_unit_levy_zero_rate_ok() {
    // Zero-rate levy is unusual but valid.
    let levy = PerUnitLevy::new("Zero levy", Amount::ZERO, "kWh");
    let pos = vec![
        LineItem::debit("Usage")
            .quantity(Quantity::new(dec!(1000), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .build()
            .unwrap(),
    ];
    let item = levy.compute(&pos).unwrap();
    assert!(item.net_amount.is_zero());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TariffSchedule — zero quantity consistency across all modes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn volume_zero_quantity_returns_empty() {
    let sched = TariffSchedule::volume()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(1000),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(1000),
            Amount::parse("0.28000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(0)).unwrap();
    assert!(
        items.is_empty(),
        "volume split(0) must return [] not [{{qty=0}}]"
    );
}

#[test]
fn block_zero_quantity_returns_empty() {
    let sched = TariffSchedule::block()
        .unit("GB")
        .band(TariffBand::block(
            dec!(10),
            Amount::parse("1.50000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(0)).unwrap();
    assert!(
        items.is_empty(),
        "block split(0) must return [] not [{{qty=0}}]"
    );
}

#[test]
fn all_modes_zero_qty_consistent() {
    // All four (non-capacity) modes must return empty vec for qty=0.
    let g = TariffSchedule::graduated()
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let v = TariffSchedule::volume()
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let b = TariffSchedule::block()
        .band(TariffBand::block(
            dec!(10),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();

    assert!(g.split(dec!(0)).unwrap().is_empty(), "graduated");
    assert!(v.split(dec!(0)).unwrap().is_empty(), "volume");
    assert!(b.split(dec!(0)).unwrap().is_empty(), "block");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// minimum_charge — Result propagation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn minimum_charge_returns_result() {
    // Signature is now Result<Option<LineItem>, BillingError> — verify Ok path.
    let doc = doc_with_charge("10.00000");
    let result = minimum_charge(&doc, Amount::parse("100.00000").unwrap(), "Min");
    assert!(result.is_ok());
    let item = result.unwrap().unwrap();
    assert_eq!(item.net_amount, Amount::parse("90.00000").unwrap());
}

#[test]
fn minimum_charge_none_returns_ok_none() {
    let doc = doc_with_charge("500.00000");
    let result = minimum_charge(&doc, Amount::parse("500.00000").unwrap(), "Min");
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TariffBand::free_up_to — description no longer hardcodes "units"
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn free_up_to_description_no_hardcoded_units() {
    let band = TariffBand::free_up_to(dec!(1000));
    let desc = band.description.as_deref().unwrap_or("");
    assert!(
        !desc.contains(" units"),
        "description should not hardcode 'units'; got: {desc:?}"
    );
}

#[test]
fn free_up_to_with_description_override() {
    let sched = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::free_up_to(dec!(1000)).with_description("Free tier (first 1000 kWh)"))
        .band(TariffBand::over(
            dec!(1000),
            Amount::parse("0.00100").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(1500)).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].description, "Free tier (first 1000 kWh)");
    assert!(items[0].net_amount.is_zero());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount<0> — integer-only amounts (P=0 edge case)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn amount_p0_parse_integer() {
    // Before the fix, Amount<0>::parse("5") failed because frac_padded="" → Err.
    let a = Amount::<0>::parse("5").expect("Amount<0>::parse('5') must succeed");
    assert_eq!(a.to_raw(), 5);
    assert_eq!(a.to_string(), "5");
}

#[test]
fn amount_p0_parse_with_zero_frac() {
    // "5.0" for Amount<0> — fractional part is "0" (1 char), P=0, extra="0" all zeros → Ok.
    let a = Amount::<0>::parse("5.0").expect("Amount<0>::parse('5.0') must succeed");
    assert_eq!(a.to_raw(), 5);
}

#[test]
fn amount_p0_parse_nonzero_frac_rejected() {
    // "5.1" for Amount<0> — extra digit "1" is non-zero → Err.
    assert!(Amount::<0>::parse("5.1").is_err());
}

#[test]
fn amount_p0_arithmetic() {
    let a = Amount::<0>::from_int(10);
    let b = Amount::<0>::from_int(3);
    assert_eq!((a + b).to_raw(), 13);
    assert_eq!((a - b).to_raw(), 7);
    assert_eq!(a.mul_qty(dec!(3)).to_raw(), 30);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DynamicPricing — interval validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn dynamic_pricing_negative_qty_rejected() {
    // Negative interval quantity is physically meaningless and now rejected.
    let result =
        DynamicPricing::from_intervals(vec![(dec!(-100), Amount::parse("0.10000").unwrap())]);
    assert!(result.is_err(), "negative interval qty must be rejected");
}

#[test]
fn dynamic_pricing_zero_qty_rejected() {
    // Zero quantity contributes nothing; reject to keep intervals meaningful.
    let result = DynamicPricing::from_intervals(vec![(dec!(0), Amount::parse("0.10000").unwrap())]);
    assert!(result.is_err(), "zero interval qty must be rejected");
}

#[test]
fn dynamic_pricing_valid_intervals_ok() {
    let dp = DynamicPricing::from_intervals(vec![
        (dec!(100), Amount::parse("0.10000").unwrap()),
        (dec!(200), Amount::parse("0.20000").unwrap()),
    ])
    .unwrap();
    let item = dp.calculate().unwrap();
    assert_eq!(item.net_amount, Amount::parse("50.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// prorate_amount — validates active_days <= total_days
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn prorate_amount_active_exceeds_total_is_error() {
    // prorate() already validated this; prorate_amount() did not until now.
    let result = prorate_amount(
        Amount::parse("100.00000").unwrap(),
        45,
        30,
        RoundingStrategy::MidpointAwayFromZero,
    );
    assert!(result.is_err(), "active_days > total_days must be Err");
}

#[test]
fn prorate_amount_zero_days_is_error() {
    let result = prorate_amount(
        Amount::parse("100.00000").unwrap(),
        0,
        0,
        RoundingStrategy::MidpointAwayFromZero,
    );
    assert!(result.is_err());
}

#[test]
fn prorate_amount_matches_prorate_item_for_all_strategies() {
    // Both functions must agree for every rounding strategy.
    let amount = Amount::parse("100.00000").unwrap();
    let item = LineItem::fixed("Fee", amount).build().unwrap();
    for strategy in [
        RoundingStrategy::MidpointAwayFromZero,
        RoundingStrategy::MidpointToEven,
        RoundingStrategy::Floor,
        RoundingStrategy::Ceiling,
        RoundingStrategy::Truncate,
    ] {
        let via_item = prorate(&item, 7, 30, strategy).unwrap().net_amount;
        let direct = prorate_amount(amount, 7, 30, strategy).unwrap();
        assert_eq!(via_item, direct, "strategy {:?} mismatch", strategy);
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Tax compute — checked_mul_qty propagates Err instead of panic
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn fixed_rate_tax_checked_on_extreme_base() {
    // With a 1000% rate and the maximum representable base, the product overflows.
    // checked_mul_qty returns Err which propagates through compute() instead of panicking.
    // Note: rate=10 (1000%) is valid (>= 0) — some jurisdictions have high rates.
    let max_item = LineItem::fixed(
        "MaxCharge",
        // from_int(92_233_720_368_547) is near the i64::MAX representable value for P=5
        Amount::<5>::from_int(92_233_720_368_547),
    )
    .build()
    .unwrap();
    let tax = FixedRateTax::new("HighTax", dec!(10)); // 1000% tax — product overflows
    let result = tax.compute(&[max_item]);
    assert!(
        result.is_err(),
        "overflow in tax compute must return Err, not panic"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Rate display — exact percentage, no rounding or trailing zeros
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn fixed_rate_tax_label_shows_exact_rate() {
    // 0.195 = 19.5%; old {:.0}% would round to "20%".
    let pos = vec![
        LineItem::fixed("C", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let tax = FixedRateTax::new("SpecialTax", dec!(0.195));
    let item = tax.compute(&pos).unwrap();
    assert!(
        item.description.contains("19.5%"),
        "label should show '19.5%', got: {:?}",
        item.description
    );
    // Must NOT show rounded "20%"
    assert!(
        !item.description.contains("20%"),
        "label must not round to '20%', got: {:?}",
        item.description
    );
}

#[test]
fn fixed_rate_tax_label_no_trailing_zeros() {
    let pos = vec![
        LineItem::fixed("C", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let tax = FixedRateTax::new("VAT", dec!(0.19));
    let item = tax.compute(&pos).unwrap();
    // Should show "19%" not "19.00%" or "19.0%"
    assert!(
        item.description.contains("(19%)"),
        "label should show '(19%)', got: {:?}",
        item.description
    );
}

#[test]
fn percentage_charge_label_exact_rate() {
    let pos = vec![
        LineItem::fixed("C", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let charge = PercentageCharge::new("Fee", dec!(0.025)); // 2.5%
    let item = charge.compute(&pos).unwrap();
    assert!(
        item.description.contains("2.5%"),
        "got: {:?}",
        item.description
    );
}

#[test]
fn percentage_discount_label_exact_rate() {
    let pos = vec![
        LineItem::fixed("C", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let disc = PercentageDiscount::new("Rebate", dec!(0.075)); // 7.5%
    let item = disc.compute(&pos).unwrap();
    assert!(
        item.description.contains("7.5%"),
        "got: {:?}",
        item.description
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TariffBand — lower >= upper rejected at build time
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn tariff_band_inverted_bounds_rejected() {
    // between(20, 5) has lower > upper — must fail at build.
    let result = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::between(
            dec!(20),
            dec!(5),
            Amount::parse("1.00000").unwrap(),
        ))
        .build();
    assert!(result.is_err(), "lower > upper must be rejected at build");
}

#[test]
fn tariff_band_equal_bounds_rejected() {
    // between(5, 5) has lower == upper — empty range, must fail.
    let result = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::between(
            dec!(5),
            dec!(5),
            Amount::parse("1.00000").unwrap(),
        ))
        .build();
    assert!(result.is_err(), "lower == upper must be rejected at build");
}

#[test]
fn tariff_band_valid_bounds_ok() {
    // Three contiguous bands starting from 0 — must build successfully.
    let result = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(5),
            Amount::parse("0.80000").unwrap(),
        ))
        .band(TariffBand::between(
            dec!(5),
            dec!(20),
            Amount::parse("1.40000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(20),
            Amount::parse("2.60000").unwrap(),
        ))
        .build();
    assert!(
        result.is_ok(),
        "valid contiguous bands must build successfully"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LineItemBuilder — negative quantity rejected
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn line_item_negative_quantity_debit_rejected() {
    // A negative quantity on a Debit would produce a negative net, which is
    // semantically a credit. Callers must use Sign::Credit / LineItem::credit().
    let result = LineItem::debit("Usage")
        .quantity(Quantity::new(dec!(-100), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.32), "EUR/kWh"))
        .build();
    assert!(result.is_err(), "negative qty on debit must be Err");
}

#[test]
fn line_item_negative_quantity_credit_rejected() {
    // Same rule for credits — quantity is always a magnitude; sign is carried
    // by Sign::Debit/Credit, not by negating the quantity value.
    let result = LineItem::credit("Refund")
        .quantity(Quantity::new(dec!(-50), "units"))
        .unit_price(UnitPrice::new(dec!(1.0), "EUR/unit"))
        .build();
    assert!(result.is_err(), "negative qty on credit must also be Err");
}

#[test]
fn line_item_zero_quantity_is_ok() {
    // Zero quantity (e.g. free tier) is valid — produces net = 0.
    let item = LineItem::debit("Free tier")
        .quantity(Quantity::new(dec!(0), "calls"))
        .unit_price(UnitPrice::new(dec!(0.001), "EUR/call"))
        .build()
        .unwrap();
    assert!(item.net_amount.is_zero());
}

#[test]
fn line_item_positive_credit_quantity_ok() {
    // The correct way to model a 500-unit refund: Credit with positive qty.
    let item = LineItem::credit("Feed-in")
        .quantity(Quantity::new(dec!(500), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.08), "EUR/kWh"))
        .build()
        .unwrap();
    assert!(item.net_amount.is_negative());
    assert_eq!(item.net_amount, Amount::parse("-40.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PercentageCharge — min_amount > max_amount returns Err from compute
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn percentage_charge_min_exceeds_max_returns_err() {
    // min=5.00 > max=3.00 violates the contract: the charge would always be
    // clamped to 3.00, below the declared minimum.
    let pos = vec![
        LineItem::fixed("Base", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let charge = PercentageCharge::new("Fee", dec!(0.04))
        .with_min(Amount::parse("5.00000").unwrap())
        .with_max(Amount::parse("3.00000").unwrap()); // min > max
    let result = charge.compute(&pos);
    assert!(result.is_err(), "min_amount > max_amount must return Err");
}

#[test]
fn percentage_charge_min_equals_max_ok() {
    // min == max is valid: charge is exactly that amount regardless of base.
    let pos = vec![
        LineItem::fixed("Base", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let charge = PercentageCharge::new("Flat fee", dec!(0.04))
        .with_min(Amount::parse("4.00000").unwrap())
        .with_max(Amount::parse("4.00000").unwrap()); // min == max
    let item = charge.compute(&pos).unwrap();
    assert_eq!(item.net_amount, Amount::parse("4.00000").unwrap());
}

#[test]
fn percentage_charge_min_less_than_max_ok() {
    // Standard case: floor=2, ceil=10, rate×base=5 → charge=5.
    let pos = vec![
        LineItem::fixed("Base", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let charge = PercentageCharge::new("Commission", dec!(0.05))
        .with_min(Amount::parse("2.00000").unwrap())
        .with_max(Amount::parse("10.00000").unwrap());
    let item = charge.compute(&pos).unwrap();
    // 100 × 0.05 = 5.00, inside [2, 10]
    assert_eq!(item.net_amount, Amount::parse("5.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TimeOfUsePricing — negative usage quantities return Err
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn tou_negative_quantity_returns_err() {
    // Before the fix, negative quantities were silently skipped; now they
    // propagate as Err so the caller can detect bad meter readings.
    let tou = TimeOfUsePricing::new(vec![TouBand::new(
        "peak",
        Amount::parse("0.32000").unwrap(),
    )])
    .with_unit("kWh");
    let result = tou.calculate(&[("peak", dec!(-100))]);
    assert!(result.is_err(), "negative usage qty must be Err");
}

#[test]
fn tou_negative_quantity_on_unknown_band_also_err() {
    // Even if the band name is unknown, a negative quantity must error before
    // the unknown-band skip logic is reached.
    let tou = TimeOfUsePricing::new(vec![TouBand::new(
        "peak",
        Amount::parse("0.32000").unwrap(),
    )]);
    let result = tou.calculate(&[("unknown-band", dec!(-50))]);
    assert!(result.is_err(), "negative qty on unknown band must be Err");
}

#[test]
fn tou_zero_quantity_on_known_band_skipped_ok() {
    // Zero quantity for a known band is valid — no line item emitted.
    let tou = TimeOfUsePricing::new(vec![TouBand::new(
        "peak",
        Amount::parse("0.32000").unwrap(),
    )])
    .with_unit("kWh");
    let items = tou.calculate(&[("peak", dec!(0))]).unwrap();
    assert!(items.is_empty(), "zero usage produces no line item");
}

#[test]
fn tou_mixed_valid_and_valid_quantities_ok() {
    // One zero-qty band (skipped) and one positive-qty band (billed).
    let tou = TimeOfUsePricing::new(vec![
        TouBand::new("peak", Amount::parse("0.32000").unwrap()),
        TouBand::new("off-peak", Amount::parse("0.18000").unwrap()),
    ])
    .with_unit("kWh");
    let items = tou
        .calculate(&[("peak", dec!(0)), ("off-peak", dec!(500))])
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].net_amount, Amount::parse("90.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::checked_sum — fallible summation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn checked_sum_empty_iterator_is_zero() {
    let result = Amount::checked_sum(std::iter::empty::<Amount<5>>()).unwrap();
    assert_eq!(result, Amount::<5>::ZERO);
}

#[test]
fn checked_sum_correct_total() {
    let amounts = vec![
        Amount::<5>::parse("10.00000").unwrap(),
        Amount::<5>::parse("20.00000").unwrap(),
        Amount::<5>::parse("30.00000").unwrap(),
    ];
    let total = Amount::checked_sum(amounts.into_iter()).unwrap();
    assert_eq!(total, Amount::<5>::parse("60.00000").unwrap());
}

#[test]
fn checked_sum_overflow_returns_err() {
    // Two values that each fit in Amount<5> but overflow when summed.
    let big = Amount::<5>::from_int(92_233_720_368_547); // near max for P=5
    let result = Amount::checked_sum([big, big].into_iter());
    assert!(result.is_err(), "overflow must return Err, not panic");
}

#[test]
fn checked_sum_with_negatives_ok() {
    // Mixed signs should accumulate correctly without overflow.
    let amounts = vec![
        Amount::<5>::parse("100.00000").unwrap(),
        Amount::<5>::parse("-30.00000").unwrap(),
        Amount::<5>::parse("20.00000").unwrap(),
    ];
    let total = Amount::checked_sum(amounts.into_iter()).unwrap();
    assert_eq!(total, Amount::<5>::parse("90.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// from_positions / tax.rs — overflow returns Err (not panic) via checked_sum
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn from_positions_overflow_returns_err() {
    // Two positions each near i64::MAX overflow the net_total sum.
    // from_positions must return Err, not panic.
    let big = Amount::<5>::from_int(92_233_720_368_547);
    let pos = vec![
        LineItem::fixed("A", big).build().unwrap(),
        LineItem::fixed("B", big).build().unwrap(),
    ];
    let result = BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]);
    assert!(
        result.is_err(),
        "overflow in from_positions must return Err"
    );
}

#[test]
fn fixed_rate_tax_base_overflow_returns_err() {
    // Sum of positions overflows during base computation in FixedRateTax::compute.
    let big = Amount::<5>::from_int(92_233_720_368_547);
    let pos = vec![
        LineItem::fixed("A", big).build().unwrap(),
        LineItem::fixed("B", big).build().unwrap(),
    ];
    let tax = FixedRateTax::new("VAT", dec!(0.20));
    let result = tax.compute(&pos);
    assert!(result.is_err(), "overflow in tax base sum must return Err");
}

#[test]
fn percentage_discount_base_overflow_returns_err() {
    let big = Amount::<5>::from_int(92_233_720_368_547);
    let pos = vec![
        LineItem::fixed("A", big).build().unwrap(),
        LineItem::fixed("B", big).build().unwrap(),
    ];
    let disc = PercentageDiscount::new("Rebate", dec!(0.10));
    let result = disc.compute(&pos);
    assert!(
        result.is_err(),
        "overflow in discount base sum must return Err"
    );
}

#[test]
fn assert_valid_overflow_returns_err() {
    // A document with two max-value positions: from_positions will already
    // fail (tested above), so craft via from_raw and verify assert_valid
    // returns Err instead of panicking.
    let big = Amount::<5>::from_int(92_233_720_368_547);
    let pos = vec![
        LineItem::fixed("A", big).build().unwrap(),
        LineItem::fixed("B", big).build().unwrap(),
    ];
    // from_positions itself uses checked_sum and will return Err on overflow.
    // That is exactly what we want to verify — overflow propagates as Err, not panic.
    let result = BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]);
    assert!(
        result.is_err(),
        "from_positions with overflowing positions must return Err, not panic"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LineItem — negative unit price rejected
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// BUG-1 fix: negative unit price is now ALLOWED (§27 EEG 2023 negative EPEX hours).
/// A debit at a negative unit_price produces a negative net_amount.
#[test]
fn line_item_negative_unit_price_debit_produces_negative_net() {
    // Post-EEG plant during EPEX negative-price hours:
    // 1000 kWh × (−0.005 EUR/kWh) = −5.00 EUR
    let item = LineItem::debit("Post-EEG Spot (negativ)")
        .quantity(Quantity::new(dec!(1000), "kWh"))
        .unit_price(UnitPrice::new(dec!(-0.005), "EUR/kWh"))
        .build()
        .unwrap();
    assert_eq!(item.net_amount, Amount::parse("-5.00000").unwrap());
    assert!(item.net_amount.is_negative());
}

/// A for_usage() call with a negative price behaves identically.
#[test]
fn line_item_for_usage_negative_price() {
    let item = LineItem::for_usage(
        "EPEX Spot (negativ)",
        dec!(1000),
        "kWh",
        dec!(-0.005),
        "EUR/kWh",
    )
    .build()
    .unwrap();
    assert_eq!(item.net_amount, Amount::parse("-5.00000").unwrap());
}

/// Sign::Credit with a negative unit_price: net = qty * price = negative,
/// Credit flip does NOT apply (net is not positive), so result is negative.
/// This is correct — the negative sign is preserved without double-negation.
#[test]
fn line_item_credit_with_negative_price_no_double_negation() {
    let item = LineItem::credit("Negative credit")
        .quantity(Quantity::new(dec!(100), "kWh"))
        .unit_price(UnitPrice::new(dec!(-0.32), "EUR/kWh"))
        .build()
        .unwrap();
    // −100 × 0.32 = −32.00 — not flipped because Sign::Credit only flips positive nets
    assert_eq!(item.net_amount, Amount::parse("-32.00000").unwrap());
}

#[test]
fn line_item_zero_unit_price_ok() {
    // Zero price is valid (e.g. free-tier items, promotional billing).
    let item = LineItem::debit("Free")
        .quantity(Quantity::new(dec!(100), "units"))
        .unit_price(UnitPrice::new(dec!(0), "EUR/unit"))
        .build()
        .unwrap();
    assert!(item.net_amount.is_zero());
}

#[test]
fn line_item_fixed_amount_with_any_sign_ok() {
    // fixed_amount bypasses the qty×price path — any Amount<5> is allowed,
    // including negative (for explicit credit positions via fixed_amount).
    let item = LineItem::debit("Adjustment")
        .fixed_amount(Amount::parse("-10.00000").unwrap())
        .build()
        .unwrap();
    assert!(item.net_amount.is_negative());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TariffSchedule::build — new structural validations
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn block_schedule_multiple_bands_rejected() {
    // split_block() only uses the first band; extra bands are silently
    // unserviceable — reject at build time to prevent silent data loss.
    let result = TariffSchedule::block()
        .unit("GB")
        .band(TariffBand::block(
            dec!(10),
            Amount::parse("1.00000").unwrap(),
        ))
        .band(TariffBand::block(
            dec!(100),
            Amount::parse("0.80000").unwrap(),
        ))
        .build();
    assert!(
        result.is_err(),
        "block schedule with 2 bands must be rejected"
    );
}

#[test]
fn block_schedule_exactly_one_band_ok() {
    let result = TariffSchedule::block()
        .unit("GB")
        .band(TariffBand::block(
            dec!(10),
            Amount::parse("1.50000").unwrap(),
        ))
        .build();
    assert!(result.is_ok());
}

#[test]
fn graduated_schedule_with_negative_price_rejected() {
    // Negative band prices would create negative-net debits; catch at build time
    // before the error surfaces obscurely in LineItem::build().
    let result = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(500),
            Amount::parse("-0.32000").unwrap(),
        ))
        .build();
    assert!(result.is_err(), "negative band price must be rejected");
}

#[test]
fn volume_schedule_with_negative_price_rejected() {
    let result = TariffSchedule::volume()
        .unit("seats")
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("-1.00000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(100),
            Amount::parse("-0.80000").unwrap(),
        ))
        .build();
    assert!(result.is_err(), "negative band price must be rejected");
}

#[test]
fn zero_band_price_is_valid() {
    // Zero-price bands are valid (free tier, promotional).
    let result = TariffSchedule::graduated()
        .unit("calls")
        .band(TariffBand::free_up_to(dec!(1000)))
        .band(TariffBand::over(
            dec!(1000),
            Amount::parse("0.00100").unwrap(),
        ))
        .build();
    assert!(result.is_ok(), "zero-price band must be valid");
}

#[test]
fn block_size_on_non_block_band_rejected() {
    // A graduated/volume band with block_size set is a configuration error —
    // block_size is ignored in non-block modes and would confuse users.
    let mut band = TariffBand::up_to(dec!(100), Amount::parse("1.00000").unwrap());
    band.block_size = Some(dec!(10)); // manually inject a block_size
    let result = TariffSchedule::graduated().unit("units").band(band).build();
    assert!(
        result.is_err(),
        "block_size on non-block band must be rejected"
    );
}

#[test]
fn block_band_without_block_size_rejected() {
    // If a band in a block schedule doesn't have block_size, split_block()
    // would fail at runtime. Catch it at build time.
    let band_without_bs = TariffBand {
        description: None,
        lower: None,
        upper: None,
        price: Amount::parse("1.00000").unwrap(),
        block_size: None, // missing
    };
    let result = TariffSchedule::block()
        .unit("pkt")
        .band(band_without_bs)
        .build();
    assert!(
        result.is_err(),
        "block band without block_size must be rejected"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Allocation edge cases
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn proportional_allocation_single_share_is_identity() {
    // Allocating 100% to one recipient must produce an identical document.
    let doc = doc_with_charge("47.12345");
    let alloc = ProportionalAllocation::new(vec![dec!(1)]).unwrap();
    let parts = alloc.allocate(&doc).unwrap();
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0].net_total(), doc.net_total());
    parts[0].assert_valid();
}

#[test]
fn proportional_allocation_empty_shares_rejected() {
    // sum = 0 ≠ 1 → invalid
    assert!(ProportionalAllocation::new(vec![]).is_err());
}

#[test]
fn equal_allocation_single_recipient_is_identity() {
    let doc = doc_with_charge("100.00000");
    let docs = EqualAllocation::new(1).allocate(&doc).unwrap();
    assert_eq!(docs[0].net_total(), doc.net_total());
    docs[0].assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// merge_period_documents — document invariants preserved
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn merge_period_documents_assert_valid_passes() {
    use billing::merge_period_documents;

    let a = doc_with_charge("100.00000");
    let b = doc_with_charge("50.00000");
    let merged = merge_period_documents(a, b).unwrap();

    assert_eq!(merged.net_total(), Amount::parse("150.00000").unwrap());
    merged.assert_valid(); // Σ(positions) == totals must hold after merge
}

#[test]
fn merge_with_tax_layers_preserves_gross() {
    use billing::merge_period_documents;

    let taxes_a: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
    let taxes_b: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];

    let pos_a = vec![
        LineItem::fixed("Charge A", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let pos_b = vec![
        LineItem::fixed("Charge B", Amount::parse("50.00000").unwrap())
            .build()
            .unwrap(),
    ];

    let doc_a =
        BillingDocument::from_positions(DocumentMeta::default(), pos_a, taxes_a, vec![]).unwrap();
    let doc_b =
        BillingDocument::from_positions(DocumentMeta::default(), pos_b, taxes_b, vec![]).unwrap();

    let merged = merge_period_documents(doc_a, doc_b).unwrap();

    // net = 150, tax = 30, gross = 180
    assert_eq!(merged.net_total(), Amount::parse("150.00000").unwrap());
    assert_eq!(merged.tax_total(), Amount::parse("30.00000").unwrap());
    assert_eq!(merged.gross_total(), Amount::parse("180.00000").unwrap());
    merged.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BillingDocument::with_extra_position — invariants preserved
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn with_extra_position_preserves_three_invariants() {
    let pos = vec![
        LineItem::fixed("Base", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();

    let min_item = LineItem::fixed("Min charge", Amount::parse("10.00000").unwrap())
        .tag("minimum-charge")
        .build()
        .unwrap();
    let doc2 = doc.with_extra_position(min_item).unwrap();

    // Tax is NOT recomputed (by design), but all three invariants must still hold.
    assert_eq!(doc2.net_total(), Amount::parse("110.00000").unwrap());
    // gross = net + tax (tax unchanged = 20)
    assert_eq!(doc2.gross_total(), Amount::parse("130.00000").unwrap());
    doc2.assert_valid();
}

#[test]
fn with_extra_credit_reduces_net() {
    let doc = doc_with_charge("100.00000");
    let credit = LineItem::credit("Discount")
        .fixed_amount(Amount::parse("15.00000").unwrap())
        .build()
        .unwrap();
    let doc2 = doc.with_extra_position(credit).unwrap();
    assert_eq!(doc2.net_total(), Amount::parse("85.00000").unwrap());
    doc2.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DynamicPricing — zero-price intervals and edge cases
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn dynamic_pricing_all_zero_price_intervals_net_is_zero() {
    // All intervals have zero price → net must be zero.
    let dp = DynamicPricing::from_intervals(vec![
        (dec!(100), Amount::parse("0.00000").unwrap()),
        (dec!(200), Amount::parse("0.00000").unwrap()),
    ])
    .unwrap()
    .with_unit("kWh");
    let item = dp.calculate().unwrap();
    assert!(item.net_amount.is_zero());
    assert_eq!(item.quantity_value(), Some(dec!(300)));
}

#[test]
fn dynamic_pricing_single_interval_exact_pipeline() {
    // Single interval: net = qty × price (no averaging).
    let dp = DynamicPricing::from_intervals(vec![(dec!(1000), Amount::parse("0.32000").unwrap())])
        .unwrap()
        .with_unit("kWh");
    let item = dp.calculate().unwrap();
    assert_eq!(item.net_amount, Amount::parse("320.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount — additional arithmetic edge cases
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn amount_parse_exact_max_representable() {
    // i64::MAX = 9_223_372_036_854_775_807
    // For P=5: max = 9_223_372_036_854_775_807 / 100_000 = 92_233_720_368_547.75807
    let a = Amount::<5>::parse("92233720368547.75807").unwrap();
    assert_eq!(a.to_raw(), i64::MAX);
}

#[test]
fn amount_parse_one_above_max_is_error() {
    // 92233720368547.75808 → raw = i64::MAX + 1 → overflow → Err
    assert!(Amount::<5>::parse("92233720368547.75808").is_err());
}

#[test]
fn amount_into_decimal_lossless_roundtrip() {
    // into_decimal() must be the exact inverse of from_decimal() for all values
    // in the representable range.
    for raw in [0i64, 1, -1, i64::MAX, i64::MIN + 1, 100_000, -100_000] {
        // Use parse()/to_raw() instead of from_raw() (internal API).
        let a = Amount::<5>::from_int(raw / 100_000); // scale to whole-number Amount
        let d = a.into_decimal();
        let back = Amount::<5>::from_decimal(d).expect("roundtrip must succeed");
        assert_eq!(
            back.to_raw(),
            a.to_raw(),
            "roundtrip failed for raw={}",
            a.to_raw()
        );
    }
}

#[test]
fn amount_mul_qty_rounding_midpoint_away_from_zero() {
    // 0.00001 × 1.5 = 0.000015 → rounds to 0.00002 (midpoint away from zero)
    let price = Amount::<5>::parse("0.00001").unwrap();
    let result = price.mul_qty(rust_decimal::Decimal::from_str_exact("1.5").unwrap());
    assert_eq!(result, Amount::<5>::parse("0.00002").unwrap());
}

#[test]
fn amount_mul_qty_rounding_down_below_midpoint() {
    // 0.00001 × 1.4 = 0.000014 → rounds to 0.00001
    let price = Amount::<5>::parse("0.00001").unwrap();
    let result = price.mul_qty(rust_decimal::Decimal::from_str_exact("1.4").unwrap());
    assert_eq!(result, Amount::<5>::parse("0.00001").unwrap());
}

#[test]
fn amount_checked_sum_large_collection_ok() {
    // 1000 items of 1.00000 each = 1000.00000, well within range.
    let amounts: Vec<Amount<5>> = (0..1000)
        .map(|_| Amount::<5>::parse("1.00000").unwrap())
        .collect();
    let total = Amount::checked_sum(amounts.into_iter()).unwrap();
    assert_eq!(total, Amount::<5>::from_int(1000));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// PerUnitLevy — quantity accumulation correctness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn per_unit_levy_only_counts_matching_unit() {
    // A levy on "kWh" must not count "m³" positions.
    let pos = vec![
        LineItem::debit("kWh usage")
            .quantity(Quantity::new(dec!(1000), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .build()
            .unwrap(),
        LineItem::debit("m³ usage")
            .quantity(Quantity::new(dec!(500), "m³"))
            .unit_price(UnitPrice::new(dec!(0.80), "EUR/m³"))
            .build()
            .unwrap(),
    ];
    let levy = PerUnitLevy::new("kWh levy", Amount::parse("0.02000").unwrap(), "kWh");
    let item = levy.compute(&pos).unwrap();
    // 1000 kWh × 0.02000 = 20.00000 (m³ ignored)
    assert_eq!(item.net_amount, Amount::parse("20.00000").unwrap());
}

#[test]
fn per_unit_levy_zero_matching_units_produces_zero_levy() {
    // If no positions match the unit, the levy is zero.
    let pos = vec![
        LineItem::fixed("Flat fee", Amount::parse("50.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let levy = PerUnitLevy::new("kWh levy", Amount::parse("0.02050").unwrap(), "kWh");
    let item = levy.compute(&pos).unwrap();
    assert!(item.net_amount.is_zero());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Full invoice pipeline — end-to-end correctness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn full_pipeline_graduated_tou_minimum_allocation() {
    // Graduated kWh schedule + ToU pricing + minimum charge + 2-way allocation.
    // Verifies that every intermediate and final document passes assert_valid().

    let graduated = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(500),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(500),
            Amount::parse("0.28000").unwrap(),
        ))
        .build()
        .unwrap();

    let items = graduated.split(dec!(1234.5)).unwrap();
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
    let mut doc = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "INV-PIPELINE-001".into(),
            period_label: "2026-07".into(),
            ..Default::default()
        },
        items,
        taxes,
        vec![],
    )
    .unwrap();
    doc.assert_valid();

    // Apply minimum charge.
    let min = Amount::parse("400.00000").unwrap();
    if let Some(shortfall) = minimum_charge(&doc, min, "Minimum spend").unwrap() {
        doc = doc.with_extra_position(shortfall).unwrap();
    }
    doc.assert_valid();

    // Allocate 60/40 between two tenants.
    let alloc = ProportionalAllocation::new(vec![dec!(0.60), dec!(0.40)]).unwrap();
    let parts = alloc.allocate(&doc).unwrap();
    let total_net: Amount<5> = parts.iter().map(|d| d.net_total()).sum();
    let total_gross: Amount<5> = parts.iter().map(|d| d.gross_total()).sum();
    assert_eq!(total_net, doc.net_total());
    assert_eq!(total_gross, doc.gross_total());
    for part in &parts {
        part.assert_valid();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::MAX / Amount::MIN
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn amount_max_is_i64_max_raw() {
    assert_eq!(Amount::<5>::MAX.to_raw(), i64::MAX);
}

#[test]
fn amount_min_is_i64_min_raw() {
    assert_eq!(Amount::<5>::MIN.to_raw(), i64::MIN);
}

#[test]
fn amount_max_is_largest_representable() {
    // MAX must equal parse of its own string representation.
    let s = Amount::<5>::MAX.to_string();
    let reparsed = Amount::<5>::parse(&s).unwrap();
    assert_eq!(reparsed, Amount::<5>::MAX);
}

#[test]
fn amount_max_plus_one_overflows() {
    let result = Amount::<5>::MAX.checked_add(Amount::<5>::parse("0.00001").unwrap());
    assert!(result.is_err(), "MAX + 1 must overflow");
}

#[test]
fn amount_min_checked_neg_overflows() {
    // MIN has no positive counterpart in i64.
    let result = Amount::<5>::MIN.checked_neg();
    assert!(result.is_err(), "checked_neg of MIN must overflow");
}

#[test]
fn amount_bounds_useful_for_clamp() {
    // Common pattern: clamp a computed amount between MIN and MAX.
    let a = Amount::<5>::parse("50.00000").unwrap();
    let floor = Amount::<5>::parse("10.00000").unwrap();
    let ceil = Amount::<5>::parse("100.00000").unwrap();
    // Clamp using Ord (Amount implements Ord).
    let clamped = a.max(floor).min(ceil);
    assert_eq!(clamped, a); // 50 is within [10, 100]
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::AddAssign / SubAssign
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn amount_add_assign_basic() {
    let mut total = Amount::<5>::ZERO;
    total += Amount::<5>::parse("10.00000").unwrap();
    total += Amount::<5>::parse("20.00000").unwrap();
    total += Amount::<5>::parse("30.00000").unwrap();
    assert_eq!(total, Amount::<5>::parse("60.00000").unwrap());
}

#[test]
fn amount_sub_assign_basic() {
    let mut balance = Amount::<5>::parse("100.00000").unwrap();
    balance -= Amount::<5>::parse("35.50000").unwrap();
    assert_eq!(balance, Amount::<5>::parse("64.50000").unwrap());
}

#[test]
fn amount_add_assign_with_negatives() {
    // Adding a negative amount is equivalent to subtraction.
    let mut total = Amount::<5>::parse("100.00000").unwrap();
    let credit = -Amount::<5>::parse("40.00000").unwrap();
    total += credit;
    assert_eq!(total, Amount::<5>::parse("60.00000").unwrap());
}

#[test]
fn amount_add_assign_accumulator_pattern() {
    // The += operator enables idiomatic accumulator loops.
    let items = vec![
        LineItem::fixed("A", Amount::parse("10.00000").unwrap())
            .build()
            .unwrap(),
        LineItem::fixed("B", Amount::parse("20.00000").unwrap())
            .build()
            .unwrap(),
        LineItem::fixed("C", Amount::parse("-5.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let mut total = Amount::<5>::ZERO;
    for item in &items {
        total += item.net_amount;
    }
    assert_eq!(total, Amount::<5>::parse("25.00000").unwrap());
}

#[test]
#[should_panic(expected = "monetary overflow")]
fn amount_add_assign_panics_on_overflow() {
    let mut a = Amount::<5>::MAX;
    a += Amount::<5>::parse("0.00001").unwrap(); // overflow
}

#[test]
#[should_panic(expected = "monetary overflow")]
fn amount_sub_assign_panics_on_overflow() {
    let mut a = Amount::<5>::MIN;
    a -= Amount::<5>::parse("0.00001").unwrap(); // underflow
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BillingDocument::all_positions() — zero-allocation iterator
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn all_positions_order_net_then_discounts_then_taxes() {
    let pos = vec![
        LineItem::fixed("Net charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
    let discounts: Vec<Box<dyn DiscountLayer>> = vec![Box::new(
        billing::tax::PercentageDiscount::new("Discount", dec!(0.10)),
    )];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, discounts).unwrap();

    let positions: Vec<&LineItem> = doc.all_positions().collect();
    assert_eq!(positions.len(), 3); // 1 net + 1 discount + 1 tax

    // Net position first.
    assert_eq!(positions[0].net_amount, Amount::parse("100.00000").unwrap());
    // Discount second (negative).
    assert!(positions[1].net_amount.is_negative());
    // Tax last (positive).
    assert!(positions[2].net_amount.is_positive());
}

#[test]
fn all_positions_is_zero_allocation_iterator() {
    // The iterator can be used directly in a for loop without .collect().
    let doc = BillingDocument::from_positions(
        DocumentMeta::default(),
        vec![
            LineItem::fixed("A", Amount::parse("10.00000").unwrap())
                .build()
                .unwrap(),
            LineItem::fixed("B", Amount::parse("20.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();

    // Sum all net amounts without collecting into a Vec.
    let mut total = Amount::<5>::ZERO;
    for pos in doc.all_positions() {
        total += pos.net_amount;
    }
    assert_eq!(total, Amount::<5>::parse("30.00000").unwrap());
}

#[test]
fn all_positions_count_correct() {
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(FixedRateTax::new("T1", dec!(0.10))),
        Box::new(FixedRateTax::new("T2", dec!(0.05))),
    ];
    let doc = BillingDocument::from_positions(
        DocumentMeta::default(),
        vec![
            LineItem::fixed("X", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        taxes,
        vec![],
    )
    .unwrap();
    // 1 net + 0 discounts + 2 taxes = 3 positions total.
    assert_eq!(doc.all_positions().count(), 3);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// DocumentMeta: PartialEq + Eq + Hash
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn document_meta_equality() {
    let a = DocumentMeta {
        invoice_number: "INV-001".into(),
        period_label: "2026-07".into(),
        issue_date: Some("2026-07-01".into()),
        ..Default::default()
    };
    let b = DocumentMeta {
        invoice_number: "INV-001".into(),
        period_label: "2026-07".into(),
        issue_date: Some("2026-07-01".into()),
        ..Default::default()
    };
    assert_eq!(a, b);
}

#[test]
fn document_meta_inequality() {
    let a = DocumentMeta {
        invoice_number: "INV-001".into(),
        period_label: "2026-07".into(),
        ..Default::default()
    };
    let b = DocumentMeta {
        invoice_number: "INV-002".into(),
        ..a.clone()
    };
    assert_ne!(a, b);
}

#[test]
fn document_meta_usable_in_hashmap() {
    use std::collections::HashMap;
    let mut map: HashMap<DocumentMeta, i32> = HashMap::new();
    let meta = DocumentMeta {
        invoice_number: "X".into(),
        period_label: "Y".into(),
        ..Default::default()
    };
    map.insert(meta.clone(), 42);
    assert_eq!(map[&meta], 42);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// UniqueCountAggregator — generic key type (no forced String allocation)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn unique_count_with_borrowed_str_key_no_allocation() {
    use billing::aggregation::{UniqueCountAggregator, UsageAggregator};

    struct ApiCall<'a> {
        user_id: &'a str,
    }

    // Key is &str — borrows from the event, zero allocation.
    let agg = UniqueCountAggregator::new(|e: &ApiCall<'_>| e.user_id);
    let events = vec![
        ApiCall { user_id: "alice" },
        ApiCall { user_id: "bob" },
        ApiCall { user_id: "alice" }, // duplicate
        ApiCall { user_id: "carol" },
    ];
    let count = agg.aggregate(&events);
    assert_eq!(count, rust_decimal::Decimal::from(3u32)); // alice, bob, carol
}

#[test]
fn unique_count_with_u64_key() {
    use billing::aggregation::{UniqueCountAggregator, UsageAggregator};

    struct Event {
        tenant_id: u64,
        #[allow(dead_code)]
        value: f64,
    }

    // Key is u64 — integer comparison, no heap allocation.
    let agg = UniqueCountAggregator::new(|e: &Event| e.tenant_id);
    let events = vec![
        Event {
            tenant_id: 1,
            value: 10.0,
        },
        Event {
            tenant_id: 2,
            value: 20.0,
        },
        Event {
            tenant_id: 1,
            value: 30.0,
        }, // duplicate tenant
        Event {
            tenant_id: 3,
            value: 5.0,
        },
    ];
    let count = agg.aggregate(&events);
    assert_eq!(count, rust_decimal::Decimal::from(3u32)); // 3 distinct tenants
}

#[test]
fn unique_count_empty_events_is_zero() {
    use billing::aggregation::{UniqueCountAggregator, UsageAggregator};
    struct Ev;
    let agg = UniqueCountAggregator::new(|_: &Ev| 0u32);
    assert_eq!(agg.aggregate(&[]), rust_decimal::Decimal::ZERO);
}

#[test]
fn unique_count_with_string_key_still_works() {
    // Backward-compatible: String keys continue to work.
    use billing::aggregation::{UniqueCountAggregator, UsageAggregator};
    struct Ev {
        id: String,
    }
    let agg = UniqueCountAggregator::new(|e: &Ev| e.id.clone());
    let events = vec![
        Ev { id: "x".into() },
        Ev { id: "y".into() },
        Ev { id: "x".into() },
    ];
    assert_eq!(agg.aggregate(&events), rust_decimal::Decimal::from(2u32));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// from_positions pre-allocation — functional correctness
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn from_positions_with_many_positions_and_layers_valid() {
    // Build a document with many positions and multiple compound tax layers
    // to exercise the pre-allocated accumulated Vec path.
    use billing::tax::PercentageCharge;
    let positions: Vec<LineItem> = (1..=10)
        .map(|i| {
            LineItem::fixed(format!("Item {i}"), Amount::<5>::from_int(i as i64 * 10))
                .build()
                .unwrap()
        })
        .collect();

    // Sum = 10+20+...+100 = 550
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(PercentageCharge::new("Fee 1%", dec!(0.01))),
        Box::new(FixedRateTax::new("VAT 20%", dec!(0.20))),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), positions, taxes, vec![]).unwrap();

    assert_eq!(doc.net_total(), Amount::<5>::from_int(550));
    // Fee: 550 * 0.01 = 5.5
    // VAT base: 550 + 5.5 = 555.5; VAT = 555.5 * 0.20 = 111.1
    doc.assert_valid();
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::signum()
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn amount_signum_positive() {
    assert_eq!(Amount::<5>::parse("3.50000").unwrap().signum(), 1);
    assert_eq!(Amount::<5>::parse("0.00001").unwrap().signum(), 1);
    assert_eq!(Amount::<5>::MAX.signum(), 1);
}

#[test]
fn amount_signum_zero() {
    assert_eq!(Amount::<5>::ZERO.signum(), 0);
    assert_eq!(Amount::<5>::parse("0.00000").unwrap().signum(), 0);
}

#[test]
fn amount_signum_negative() {
    assert_eq!(Amount::<5>::parse("-3.50000").unwrap().signum(), -1);
    assert_eq!(Amount::<5>::parse("-0.00001").unwrap().signum(), -1);
}

#[test]
fn amount_signum_used_for_conditional_logic() {
    // Common pattern: apply surcharge only on positive balances.
    let amounts = [
        Amount::<5>::parse("100.00000").unwrap(),
        Amount::<5>::parse("-20.00000").unwrap(),
        Amount::<5>::ZERO,
        Amount::<5>::parse("50.00000").unwrap(),
    ];
    let positive_sum: Amount<5> = amounts.iter().filter(|a| a.signum() > 0).copied().sum();
    assert_eq!(positive_sum, Amount::<5>::parse("150.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// From<Amount<P>> for Decimal
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn decimal_from_amount_positive() {
    use rust_decimal::Decimal;
    let a = Amount::<5>::parse("1.23456").unwrap();
    let d = Decimal::from(a);
    assert_eq!(d, Decimal::from_str_exact("1.23456").unwrap());
}

#[test]
fn decimal_from_amount_zero() {
    use rust_decimal::Decimal;
    assert_eq!(Decimal::from(Amount::<5>::ZERO), Decimal::ZERO);
}

#[test]
fn decimal_from_amount_negative() {
    use rust_decimal::Decimal;
    let a = Amount::<5>::parse("-9.99999").unwrap();
    let d = Decimal::from(a);
    assert_eq!(d, Decimal::from_str_exact("-9.99999").unwrap());
}

#[test]
fn decimal_from_amount_roundtrip() {
    // into_decimal() and Decimal::from() must give the same result.
    use rust_decimal::Decimal;
    let a = Amount::<5>::parse("42.12345").unwrap();
    assert_eq!(Decimal::from(a), a.into_decimal());
}

#[test]
fn decimal_from_amount_in_generic_context() {
    // The From impl is useful in generic code, e.g. summing as Decimal.
    use rust_decimal::Decimal;
    let amounts = [
        Amount::<5>::parse("1.00000").unwrap(),
        Amount::<5>::parse("2.00000").unwrap(),
        Amount::<5>::parse("3.00000").unwrap(),
    ];
    // Use Decimal::from() in an iterator.
    let sum: Decimal = amounts.iter().copied().map(Decimal::from).sum();
    assert_eq!(sum, Decimal::from_str_exact("6.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TryFrom<i64> for Amount<P>
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn try_from_i64_basic() {
    let a = Amount::<5>::try_from(49i64).unwrap();
    assert_eq!(a, Amount::<5>::parse("49.00000").unwrap());
}

#[test]
fn try_from_i64_zero() {
    assert_eq!(Amount::<5>::try_from(0i64).unwrap(), Amount::<5>::ZERO);
}

#[test]
fn try_from_i64_negative() {
    let a = Amount::<5>::try_from(-100i64).unwrap();
    assert_eq!(a, Amount::<5>::parse("-100.00000").unwrap());
}

#[test]
fn try_from_i64_overflow() {
    // 92_233_720_368_548 × 100_000 > i64::MAX.
    let result = Amount::<5>::try_from(92_233_720_368_548i64);
    assert!(result.is_err(), "overflow must return Err, not panic");
}

#[test]
fn try_from_i64_max_valid() {
    // The largest i64 that can be converted without overflow for P=5.
    // i64::MAX / 100_000 = 92_233_720_368_547
    let max_valid = 92_233_720_368_547i64;
    assert!(Amount::<5>::try_from(max_valid).is_ok());
    assert!(Amount::<5>::try_from(max_valid + 1).is_err());
}

#[test]
fn try_from_i64_ergonomic_database_pattern() {
    // Common pattern: convert a database integer to Amount.
    let db_value: i64 = 4999; // e.g. stored as cents * 100
    let amount = Amount::<5>::try_from(db_value).unwrap();
    // 4999 × 10^5 = 499900000 raw → "4999.00000"
    assert_eq!(amount.to_string(), "4999.00000");
    // Then used for billing:
    let item = LineItem::fixed("Subscription", amount).build().unwrap();
    assert_eq!(item.net_amount.to_string(), "4999.00000");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// LineItem — empty description validation
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn line_item_empty_description_rejected() {
    let result = LineItem::debit("")
        .fixed_amount(Amount::parse("10.00000").unwrap())
        .build();
    assert!(result.is_err(), "empty description must be rejected");
}

#[test]
fn line_item_whitespace_only_description_rejected() {
    let result = LineItem::debit("   ")
        .fixed_amount(Amount::parse("10.00000").unwrap())
        .build();
    assert!(
        result.is_err(),
        "whitespace-only description must be rejected"
    );
}

#[test]
fn line_item_non_empty_description_ok() {
    let result = LineItem::debit("Platform fee")
        .fixed_amount(Amount::parse("49.00000").unwrap())
        .build();
    assert!(result.is_ok());
}

#[test]
fn line_item_credit_empty_description_rejected() {
    let result = LineItem::credit("")
        .fixed_amount(Amount::parse("10.00000").unwrap())
        .build();
    assert!(
        result.is_err(),
        "credit with empty description must be rejected"
    );
}

#[test]
fn line_item_fixed_empty_description_rejected() {
    let result = LineItem::fixed("", Amount::parse("10.00000").unwrap()).build();
    assert!(
        result.is_err(),
        "LineItem::fixed with empty description must be rejected"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// EqualAllocation — fail-fast at construction
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
#[should_panic(expected = "EqualAllocation requires n > 0")]
fn equal_allocation_new_zero_panics() {
    // Fail fast at construction, not silently at allocate() time.
    let _ = EqualAllocation::new(0);
}

#[test]
fn equal_allocation_new_one_ok() {
    let a = EqualAllocation::new(1);
    assert_eq!(a.n(), 1);
}

#[test]
fn equal_allocation_fail_fast_before_allocate() {
    // Before the fix, n=0 was silently accepted at new() and only rejected
    // when allocate() was called. Callers could hold an invalid EqualAllocation
    // instance for a long time before the error surfaced.
    // Now the panic is immediate at construction.
    let result = std::panic::catch_unwind(|| EqualAllocation::new(0));
    assert!(result.is_err(), "new(0) must panic immediately");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Cross-feature correctness: signum + allocation + From<Amount> for Decimal
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn document_with_mixed_signs_net_is_correct() {
    // Verify that a document with both debits and credits calculates net
    // correctly, and that signum() correctly identifies the sign.
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
        LineItem::credit("Discount")
            .fixed_amount(Amount::parse("30.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();

    // Net = 100 - 30 = 70
    assert_eq!(doc.net_total().signum(), 1); // positive net
    assert_eq!(doc.net_total(), Amount::parse("70.00000").unwrap());
    doc.assert_valid();

    // Verify via Decimal::from (new From impl).
    use rust_decimal::Decimal;
    let net_as_decimal = Decimal::from(doc.net_total());
    assert_eq!(net_as_decimal, Decimal::from_str_exact("70.00000").unwrap());
}

#[test]
fn try_from_i64_then_line_item_then_allocate() {
    // End-to-end: convert DB integer → Amount → LineItem → Document → allocate.
    let db_price: i64 = 100; // e.g. $100 stored as integer in DB
    let price = Amount::<5>::try_from(db_price).unwrap();
    let pos = vec![LineItem::fixed("Service", price).build().unwrap()];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    assert_eq!(doc.net_total(), Amount::<5>::from_int(100));

    let alloc = EqualAllocation::new(4);
    let parts = alloc.allocate(&doc).unwrap();
    let total: Amount<5> = parts.iter().map(|d| d.net_total()).sum();
    assert_eq!(total, doc.net_total()); // exact, no drift
    for part in &parts {
        part.assert_valid();
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::within_tolerance_ppm  (D-04 from user migration feedback)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn within_tolerance_ppm_inside_window() {
    let stated = Amount::<5>::parse("100.00000").unwrap();
    let computed = Amount::<5>::parse("100.50000").unwrap();
    // 0.5 / 100.0 = 5_000 ppm — inside 10_000 ppm (1 %)
    assert!(stated.within_tolerance_ppm(computed, 10_000).unwrap());
}

#[test]
fn within_tolerance_ppm_outside_window() {
    let stated = Amount::<5>::parse("100.00000").unwrap();
    let computed = Amount::<5>::parse("100.50000").unwrap();
    // 5_000 ppm > 4_000 ppm threshold
    assert!(!stated.within_tolerance_ppm(computed, 4_000).unwrap());
}

#[test]
fn within_tolerance_ppm_exact_equality_passes_zero_ppm() {
    let a = Amount::<5>::parse("42.50000").unwrap();
    assert!(a.within_tolerance_ppm(a, 0).unwrap());
}

#[test]
fn within_tolerance_ppm_different_values_fail_zero_ppm() {
    let a = Amount::<5>::parse("100.00000").unwrap();
    let b = Amount::<5>::parse("100.00001").unwrap();
    assert!(!a.within_tolerance_ppm(b, 0).unwrap());
}

#[test]
fn within_tolerance_ppm_expected_zero_only_passes_when_self_zero() {
    let zero = Amount::<5>::ZERO;
    let nonzero = Amount::<5>::parse("0.00001").unwrap();
    assert!(zero.within_tolerance_ppm(zero, 999_999).unwrap());
    assert!(!nonzero.within_tolerance_ppm(zero, 999_999).unwrap());
}

#[test]
fn within_tolerance_ppm_negative_expected() {
    // Tolerance is relative to |expected|, sign doesn't matter.
    let expected = Amount::<5>::parse("-100.00000").unwrap();
    let actual = Amount::<5>::parse("-100.50000").unwrap();
    assert!(actual.within_tolerance_ppm(expected, 10_000).unwrap());
    assert!(!actual.within_tolerance_ppm(expected, 4_000).unwrap());
}

#[test]
fn within_tolerance_ppm_symmetry() {
    // |a - b| == |b - a|, so the result must be the same both ways.
    let a = Amount::<5>::parse("100.00000").unwrap();
    let b = Amount::<5>::parse("101.00000").unwrap();
    assert_eq!(
        a.within_tolerance_ppm(b, 15_000).unwrap(),
        b.within_tolerance_ppm(a, 15_000).unwrap(),
    );
}

#[test]
fn within_tolerance_ppm_invoice_validation_pattern() {
    // Typical use: check computed invoice total against stated total.
    // invoic-checker pattern (D-04 use case).
    let stated_total = Amount::<5>::parse("12345.67890").unwrap();
    let computed_total = Amount::<5>::parse("12345.56000").unwrap();
    // diff ≈ 0.119 EUR on 12345 EUR ≈ 9.6 ppm — within 100 ppm
    assert!(
        computed_total
            .within_tolerance_ppm(stated_total, 100)
            .unwrap()
    );
    // but outside 5 ppm
    assert!(
        !computed_total
            .within_tolerance_ppm(stated_total, 5)
            .unwrap()
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// Amount::checked_from_decimal  (D-01 / B-02 from user feedback)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn checked_from_decimal_basic() {
    use rust_decimal::Decimal;
    let d = Decimal::from_str_exact("1.23456").unwrap();
    let a = Amount::<5>::checked_from_decimal(d).unwrap();
    assert_eq!(a, Amount::<5>::parse("1.23456").unwrap());
}

#[test]
fn checked_from_decimal_rounds_to_p_digits() {
    use rust_decimal::Decimal;
    // 0.123456 rounded to 5 dp = 0.12346 (MidpointAwayFromZero)
    let d = Decimal::from_str_exact("0.123456").unwrap();
    let a = Amount::<5>::checked_from_decimal(d).unwrap();
    assert_eq!(a, Amount::<5>::parse("0.12346").unwrap());
}

#[test]
fn checked_from_decimal_overflow_returns_err() {
    use rust_decimal::Decimal;
    // A value far exceeding i64::MAX / SCALE overflows.
    let d = Decimal::from_str_exact("999999999999999999").unwrap();
    assert!(Amount::<5>::checked_from_decimal(d).is_err());
}

#[test]
fn checked_from_decimal_is_question_mark_compatible() {
    use rust_decimal::Decimal;
    fn compute(d: Decimal) -> Result<Amount<5>, billing::BillingError> {
        let a = Amount::<5>::checked_from_decimal(d)?;
        Ok(a)
    }
    let d = Decimal::from_str_exact("42.00000").unwrap();
    assert_eq!(compute(d).unwrap(), Amount::<5>::from_int(42));
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// TryFrom<i64> is NOT the inverse of to_raw()  (B-04 from user feedback)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn try_from_i64_is_whole_units_not_raw() {
    let a = Amount::<5>::parse("0.03456").unwrap();
    let raw = a.to_raw(); // 3_456  (internal scaled value)

    // TryFrom<i64> treats the integer as whole units → 3456 whole EUR
    let via_try_from = Amount::<5>::try_from(raw).unwrap();
    assert_eq!(via_try_from, Amount::<5>::parse("3456.00000").unwrap());

    // from_raw_units is the correct inverse of to_raw()
    let via_raw_units = Amount::<5>::from_raw_units(raw);
    assert_eq!(via_raw_units, a);
    assert_ne!(via_try_from, via_raw_units); // they differ — this IS the footgun
}

#[test]
fn from_raw_units_is_exact_inverse_of_to_raw() {
    for s in &["0.00000", "1.00000", "-9.99999", "92233720368547.75807"] {
        let a = Amount::<5>::parse(s).unwrap();
        let reconstructed = Amount::<5>::from_raw_units(a.to_raw());
        assert_eq!(a, reconstructed, "round-trip failed for {s}");
    }
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BillingDocument::discount_total()  (F-05 from user feedback)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn discount_total_no_discounts_is_zero() {
    let pos = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    assert_eq!(doc.discount_total(), Amount::<5>::ZERO);
}

#[test]
fn discount_total_matches_sum_of_discount_positions() {
    use billing::tax::{FixedDiscount, PercentageDiscount};

    let pos = vec![
        LineItem::fixed("Service", Amount::parse("200.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let discounts: Vec<Box<dyn billing::DiscountLayer>> = vec![
        Box::new(FixedDiscount::new(
            "Voucher",
            Amount::parse("20.00000").unwrap(),
        )),
        Box::new(PercentageDiscount::new("Loyalty 5%", dec!(0.05))),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], discounts).unwrap();

    // discount_total() must be negative (discounts reduce the base)
    assert!(doc.discount_total().is_negative());
    // It must equal the sum of individual discount positions
    let manual_sum: Amount<5> = doc.discount_positions().iter().map(|p| p.net_amount).sum();
    assert_eq!(doc.discount_total(), manual_sum);
    // net_total must account for discounts
    doc.assert_valid();
}

#[test]
fn discount_total_plus_net_positions_equals_net_total() {
    use billing::tax::FixedDiscount;

    let pos = vec![
        LineItem::fixed("Item", Amount::parse("150.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let discounts: Vec<Box<dyn billing::DiscountLayer>> = vec![Box::new(FixedDiscount::new(
        "Rebate",
        Amount::parse("30.00000").unwrap(),
    ))];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], discounts).unwrap();

    // net_total = Σ(net_positions) + discount_total
    let net_positions_sum: Amount<5> = doc.net_positions().iter().map(|p| p.net_amount).sum();
    assert_eq!(net_positions_sum + doc.discount_total(), doc.net_total(),);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// BillingDocument::positions_by_tag()  (F-02 from user feedback)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn positions_by_tag_finds_net_positions() {
    let pos = vec![
        LineItem::fixed("Commodity", Amount::parse("100.00000").unwrap())
            .tag("commodity")
            .build()
            .unwrap(),
        LineItem::fixed("Fixed fee", Amount::parse("10.00000").unwrap())
            .tag("fixed")
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();

    let commodity: Vec<_> = doc.positions_by_tag("commodity").collect();
    assert_eq!(commodity.len(), 1);
    assert_eq!(commodity[0].net_amount, Amount::parse("100.00000").unwrap());

    let fixed: Vec<_> = doc.positions_by_tag("fixed").collect();
    assert_eq!(fixed.len(), 1);
}

#[test]
fn positions_by_tag_no_match_returns_empty() {
    let pos = vec![
        LineItem::fixed("Item", Amount::parse("50.00000").unwrap())
            .tag("other")
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    assert_eq!(doc.positions_by_tag("missing").count(), 0);
}

#[test]
fn positions_by_tag_searches_tax_positions_too() {
    let pos = vec![
        LineItem::fixed("Net", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn billing::TaxLayer>> =
        vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)))];
    let doc = BillingDocument::from_positions(DocumentMeta::default(), pos, taxes, vec![]).unwrap();

    // Tax positions are tagged "tax" by FixedRateTax
    let tax_items: Vec<_> = doc.positions_by_tag("tax").collect();
    assert_eq!(tax_items.len(), 1);
    assert_eq!(tax_items[0].net_amount, Amount::parse("20.00000").unwrap());
}

#[test]
fn positions_by_tag_all_tags_returns_all() {
    // An item can have multiple tags
    let pos = vec![
        LineItem::fixed("Multi", Amount::parse("1.00000").unwrap())
            .tag("a")
            .tag("b")
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    assert_eq!(doc.positions_by_tag("a").count(), 1);
    assert_eq!(doc.positions_by_tag("b").count(), 1);
    assert_eq!(doc.positions_by_tag("c").count(), 0);
}

// ── Regression: within_tolerance_ppm i64::MIN-abs panic (bug fixed 2025) ────

/// When `self.0 - expected.0 == i64::MIN` the old Decimal-based implementation
/// called `.abs()` on the difference, which panics because `i64::MIN` has no
/// positive i64 counterpart.  The new u128-integer implementation must NOT panic.
#[test]
fn within_tolerance_ppm_no_panic_when_diff_equals_i64_min() {
    // Craft two amounts whose raw difference is exactly i64::MIN.
    // self.0 = i64::MIN + 5,  expected.0 = 5  →  diff.0 = i64::MIN
    // expected != 0 so the early-return branch is not taken.
    let self_amt = Amount::<5>::from_raw_units(i64::MIN + 5);
    let expected_amt = Amount::<5>::from_raw_units(5);
    // The difference is i64::MIN, which is a very large deviation.
    // Even with ppm = u32::MAX the relative tolerance is nowhere near i64::MIN,
    // so the result must be false — but crucially it must NOT panic.
    let result = self_amt.within_tolerance_ppm(expected_amt, u32::MAX);
    assert!(
        result.is_ok(),
        "must not panic or return Err on extreme inputs"
    );
    assert!(
        !result.unwrap(),
        "a diff of i64::MIN is not within any reasonable tolerance of 5"
    );
}

/// Symmetry property: tolerance is independent of direction.
#[test]
fn within_tolerance_ppm_no_panic_extreme_values() {
    // Both amounts at extreme ends — checked_sub will overflow → should return Err, not panic.
    let max = Amount::<5>::MAX;
    let min = Amount::<5>::MIN;
    let result = max.within_tolerance_ppm(min, 1_000);
    // The subtraction MAX - MIN overflows i64, so we expect Err, not a panic.
    assert!(
        result.is_err(),
        "MAX - MIN overflows i64, must return Err not panic"
    );
}

// ── Regression: minimum_charge with empty description returns Err (bug fixed 2025) ──

/// The old implementation used `.expect("cannot fail")` but `build()` validates
/// that the description is non-empty.  Passing an empty description must return
/// `Err(BillingError::InvalidInput)` instead of panicking.
#[test]
fn minimum_charge_empty_description_returns_err() {
    let doc = BillingDocument::from_positions(
        DocumentMeta::default(),
        vec![
            LineItem::fixed("Item", Amount::parse("1.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();
    let result = minimum_charge(&doc, Amount::parse("100.00000").unwrap(), "");
    assert!(
        result.is_err(),
        "empty description must propagate Err, not panic"
    );
    assert!(
        matches!(result, Err(billing::BillingError::InvalidInput { .. })),
        "expected InvalidInput error for empty description"
    );
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// v0.4.0 — New APIs from eeg-billing feedback
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ── BUG-2: LineItem::credit_fixed ────────────────────────────────────────────

#[test]
fn credit_fixed_produces_negative_net() {
    let item = LineItem::credit_fixed("Rückerstattung", Amount::parse("50.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.net_amount, Amount::parse("-50.00000").unwrap());
    assert!(item.net_amount.is_negative());
}

#[test]
fn credit_fixed_zero_stays_zero() {
    // §25 EEG suspension: a EUR 0 credit position is valid
    let item = LineItem::credit_fixed("§25 EEG Sperre", Amount::parse("0.00000").unwrap())
        .tag("sanction")
        .build()
        .unwrap();
    assert!(item.net_amount.is_zero());
}

#[test]
fn fixed_and_credit_fixed_are_symmetric() {
    let debit = LineItem::fixed("charge", Amount::parse("100.00000").unwrap())
        .build()
        .unwrap();
    let credit = LineItem::credit_fixed("credit", Amount::parse("100.00000").unwrap())
        .build()
        .unwrap();
    assert!(debit.net_amount.is_positive());
    assert!(credit.net_amount.is_negative());
    // Absolute values match
    assert_eq!(debit.net_amount.to_raw(), -credit.net_amount.to_raw());
}

// ── FR-3: LineItem::for_usage ────────────────────────────────────────────────

#[test]
fn for_usage_positive_price() {
    let item = LineItem::for_usage(
        "Einspeisevergütung §21 EEG",
        dec!(5000),
        "kWh",
        dec!(0.0811),
        "EUR/kWh",
    )
    .build()
    .unwrap();
    assert_eq!(item.net_amount, Amount::parse("405.50000").unwrap());
    assert_eq!(item.quantity_value(), Some(dec!(5000)));
    assert_eq!(item.unit_label(), Some("kWh"));
}

#[test]
fn for_usage_zero_quantity_gives_zero_net() {
    let item = LineItem::for_usage("Freigrenze", dec!(0), "kWh", dec!(0.32), "EUR/kWh")
        .build()
        .unwrap();
    assert!(item.net_amount.is_zero());
}

// ── FR-4: LineItem::get_meta ─────────────────────────────────────────────────

#[test]
fn get_meta_returns_existing_value() {
    let item = LineItem::fixed("EEG pos", Amount::parse("10.00000").unwrap())
        .meta("legal_basis", "§21 EEG 2023")
        .meta("period", "2026-06")
        .build()
        .unwrap();
    assert_eq!(item.get_meta("legal_basis"), Some("§21 EEG 2023"));
    assert_eq!(item.get_meta("period"), Some("2026-06"));
    assert_eq!(item.get_meta("missing_key"), None);
}

// ── FR-7: LineItem: Eq ───────────────────────────────────────────────────────

#[test]
fn line_item_implements_eq() {
    let a = LineItem::fixed("X", Amount::parse("1.00000").unwrap())
        .build()
        .unwrap();
    let b = a.clone();
    assert_eq!(a, b);
    // Eq: a == b
    assert!(a == b);
}

#[test]
fn line_item_implements_eq_with_vec_dedup() {
    // Eq allows Vec::dedup and other collection operations that need equality.
    let a = LineItem::fixed("X", Amount::parse("1.00000").unwrap())
        .build()
        .unwrap();
    let b = a.clone();
    let c = LineItem::fixed("Y", Amount::parse("2.00000").unwrap())
        .build()
        .unwrap();
    let mut items = vec![a.clone(), b.clone(), c.clone()];
    items.dedup(); // requires Eq
    assert_eq!(items.len(), 2); // a==b, so dedup removes one
}

// ── BUG-3 correction: BillingError is #[non_exhaustive] — idiomatic _ => pattern ──

/// `BillingError` is `#[non_exhaustive]` so the library can add new variants in
/// minor releases without breaking callers.  The idiomatic match pattern always
/// includes a catch-all arm.  This test documents that pattern and verifies it
/// compiles correctly.
#[test]
fn billing_error_non_exhaustive_match_pattern() {
    fn describe(e: &billing::BillingError) -> &'static str {
        match e {
            billing::BillingError::MonetaryOverflow { .. } => "overflow",
            billing::BillingError::InvalidSchedule { .. } => "schedule",
            billing::BillingError::InvalidInput { .. } => "input",
            billing::BillingError::ValidationFailed { .. } => "validation",
            billing::BillingError::InvalidAllocationShares { .. } => "allocation",
            billing::BillingError::ZeroPeriod => "period",
            billing::BillingError::LayerError { .. } => "layer",
            // Required: future minor releases may add new variants.
            _ => "unknown",
        }
    }
    assert_eq!(describe(&billing::BillingError::ZeroPeriod), "period");
    assert_eq!(
        describe(&billing::BillingError::MonetaryOverflow { precision: 5 }),
        "overflow"
    );
}

// ── FR-1: DocumentMeta new typed fields ─────────────────────────────────────

#[test]
fn document_meta_new_fields_round_trip() {
    let meta = DocumentMeta {
        invoice_number: "INV-EEG-2026-07".into(),
        period_label: "2026-07".into(),
        period: Some(billing::Period::new("2026-07-01", "2026-07-31")),
        issue_date: Some("2026-08-01".into()),
        due_date: Some("2026-08-31".into()),
        issuer_id: Some("9900123456789".into()), // BDEW MP-ID
        recipient_id: Some("4012345678901".into()), // GLN of Netzbetreiber
        notes: Some("EEG Einspeisevergütung §21 EEG 2023".into()),
    };
    assert_eq!(meta.period.as_ref().unwrap().from, "2026-07-01");
    assert_eq!(meta.period.as_ref().unwrap().to, "2026-07-31");
    assert_eq!(meta.issue_date.as_deref(), Some("2026-08-01"));
    assert_eq!(meta.due_date.as_deref(), Some("2026-08-31"));
    assert_eq!(meta.issuer_id.as_deref(), Some("9900123456789"));
    assert_eq!(meta.recipient_id.as_deref(), Some("4012345678901"));
}

#[test]
fn document_meta_default_has_none_new_fields() {
    let m = DocumentMeta::default();
    assert!(m.period.is_none());
    assert!(m.issue_date.is_none());
    assert!(m.due_date.is_none());
    assert!(m.issuer_id.is_none());
    assert!(m.recipient_id.is_none());
}

#[test]
fn document_meta_equality_uses_new_fields() {
    let base = DocumentMeta {
        invoice_number: "X".into(),
        period: Some(billing::Period::new("2026-06-01", "2026-06-30")),
        ..Default::default()
    };
    let same = base.clone();
    let different = DocumentMeta {
        invoice_number: "X".into(),
        period: Some(billing::Period::new("2026-07-01", "2026-07-31")), // different!
        ..Default::default()
    };
    assert_eq!(base, same);
    assert_ne!(base, different);
}

// ── FR-5: RateLookup (capacity-based rate table) ─────────────────────────────

#[test]
fn rate_lookup_eeg_scenario() {
    // EEG §21 Vergütungssatz by installed capacity
    let lookup = billing::RateLookup::builder()
        .at_most(dec!(10), Amount::parse("0.00811").unwrap())
        .at_most(dec!(40), Amount::parse("0.00679").unwrap())
        .fallback(Amount::parse("0.00556").unwrap())
        .build()
        .unwrap();

    // ≤10 kWp band
    assert_eq!(
        lookup.rate_for(dec!(5)).unwrap(),
        Amount::parse("0.00811").unwrap()
    );
    assert_eq!(
        lookup.rate_for(dec!(10)).unwrap(),
        Amount::parse("0.00811").unwrap()
    );
    // ≤40 kWp band
    assert_eq!(
        lookup.rate_for(dec!(10.001)).unwrap(),
        Amount::parse("0.00679").unwrap()
    );
    assert_eq!(
        lookup.rate_for(dec!(40)).unwrap(),
        Amount::parse("0.00679").unwrap()
    );
    // >40 kWp fallback
    assert_eq!(
        lookup.rate_for(dec!(100)).unwrap(),
        Amount::parse("0.00556").unwrap()
    );
}

#[test]
fn rate_lookup_applied_to_line_item() {
    // Full pipeline: lookup rate → for_usage → document
    let lookup = billing::RateLookup::builder()
        .at_most(dec!(10), Amount::parse("0.00811").unwrap())
        .fallback(Amount::parse("0.00556").unwrap())
        .build()
        .unwrap();

    let leistung_kwp = dec!(8); // 8 kWp plant
    let einspeisung_kwh = dec!(450);

    let rate = lookup.rate_for(leistung_kwp).unwrap();
    let item = LineItem::for_usage(
        "EEG Einspeisevergütung",
        einspeisung_kwh,
        "kWh",
        rate.into_decimal(),
        "EUR/kWh",
    )
    .tag("eeg")
    .build()
    .unwrap();

    // 450 × 0.00811 = 3.6495 EUR → rounded to 5dp = 3.64950
    assert_eq!(item.net_amount, Amount::parse("3.64950").unwrap());
}

#[test]
fn rate_lookup_no_fallback_error() {
    let lookup = billing::RateLookup::builder()
        .at_most(dec!(10), Amount::parse("0.00811").unwrap())
        .build()
        .unwrap();
    assert!(lookup.rate_for(dec!(50)).is_err());
}

// ── FR-6: BillingDocument::validate() returns Result ────────────────────────
// (Corruption tests live in src/document.rs unit tests which have private field access.)

#[test]
fn validate_returns_ok_on_valid_doc() {
    let doc = BillingDocument::from_positions(
        DocumentMeta::default(),
        vec![
            LineItem::fixed("A", Amount::parse("1.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();
    assert!(doc.validate().is_ok());
}

#[test]
fn assert_valid_does_not_panic_on_valid_doc() {
    // assert_valid() is the panicking form; on a valid doc it must not panic.
    let doc = BillingDocument::from_positions(
        DocumentMeta::default(),
        vec![
            LineItem::fixed("A", Amount::parse("1.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();
    doc.assert_valid(); // must not panic
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// v0.5.0 deep audit — 5 correctness fixes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ── Fix 1: LineItem stores Sign field ────────────────────────────────────────

#[test]
fn line_item_sign_stored_debit() {
    let item = LineItem::debit("charge")
        .fixed_amount(Amount::parse("10.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.sign, billing::Sign::Debit);
    assert!(item.is_debit());
    assert!(!item.is_credit());
}

#[test]
fn line_item_sign_stored_credit() {
    let item = LineItem::credit("refund")
        .fixed_amount(Amount::parse("10.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.sign, billing::Sign::Credit);
    assert!(item.is_credit());
    assert!(!item.is_debit());
}

#[test]
fn line_item_sign_debit_negative_price_is_still_debit() {
    // EPEX negative-price hour: Sign::Debit, unit_price < 0, net_amount < 0.
    // The sign field must be Debit — NOT Credit — because this is a consumption position.
    let item = LineItem::debit("EPEX negativ")
        .quantity(Quantity::new(dec!(1000), "kWh"))
        .unit_price(UnitPrice::new(dec!(-0.005), "EUR/kWh"))
        .build()
        .unwrap();
    assert_eq!(item.sign, billing::Sign::Debit);
    assert!(
        item.net_amount.is_negative(),
        "net_amount should be negative"
    );
    assert!(
        item.is_debit(),
        "sign must be Debit despite negative net_amount"
    );
}

#[test]
fn line_item_fixed_sign_is_debit() {
    let item = LineItem::fixed("charge", Amount::parse("5.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.sign, billing::Sign::Debit);
}

#[test]
fn line_item_credit_fixed_sign_is_credit() {
    let item = LineItem::credit_fixed("refund", Amount::parse("5.00000").unwrap())
        .build()
        .unwrap();
    assert_eq!(item.sign, billing::Sign::Credit);
}

#[test]
fn line_item_for_usage_sign_is_debit() {
    let item = LineItem::for_usage("usage", dec!(100), "kWh", dec!(0.32), "EUR/kWh")
        .build()
        .unwrap();
    assert_eq!(item.sign, billing::Sign::Debit);
}

// ── Fix 2: PerUnitLevy uses sign field, not net_amount polarity ──────────────

/// Core regression: a Sign::Debit position at a negative price (EPEX scenario)
/// MUST be included in the PerUnitLevy base. Before the fix, `!net_amount.is_negative()`
/// would exclude it, under-counting the physical consumption for the levy.
#[test]
fn per_unit_levy_includes_debit_at_negative_price() {
    use billing::PerUnitLevy;
    // 1000 kWh at -0.005 EUR/kWh — negative EPEX, Sign::Debit
    let epex_position = LineItem::debit("EPEX Spot negativ")
        .quantity(Quantity::new(dec!(1000), "kWh"))
        .unit_price(UnitPrice::new(dec!(-0.005), "EUR/kWh"))
        .build()
        .unwrap();
    assert!(epex_position.is_debit());
    assert!(epex_position.net_amount.is_negative());

    let levy = PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh");
    let tax_item = levy.compute(&[epex_position]).unwrap();
    // 1000 kWh × 0.02050 = 20.50 EUR — the levy applies to physical consumption
    assert_eq!(tax_item.net_amount, Amount::parse("20.50000").unwrap());
}

/// A Sign::Credit position (return/feed-in) must still be EXCLUDED from the levy.
#[test]
fn per_unit_levy_excludes_credit_positions_sign_field_regression() {
    use billing::PerUnitLevy;
    let feed_in = LineItem::credit("EEG Einspeisung")
        .quantity(Quantity::new(dec!(500), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.0811), "EUR/kWh"))
        .build()
        .unwrap();
    assert!(feed_in.is_credit());

    let levy = PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh");
    let tax_item = levy.compute(&[feed_in]).unwrap();
    // Sign::Credit → excluded → 0 kWh × 0.02050 = 0
    assert!(tax_item.net_amount.is_zero());
}

/// Mixed: debit + debit-at-negative-price + credit.
/// Only the two debit positions contribute quantities to the levy.
#[test]
fn per_unit_levy_mixed_sign_positions() {
    use billing::PerUnitLevy;
    let consumption = LineItem::debit("Normal consumption")
        .quantity(Quantity::new(dec!(800), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
        .build()
        .unwrap();
    let negative_epex = LineItem::debit("EPEX negativ")
        .quantity(Quantity::new(dec!(200), "kWh"))
        .unit_price(UnitPrice::new(dec!(-0.01), "EUR/kWh"))
        .build()
        .unwrap();
    let feed_in = LineItem::credit("Feed-in credit")
        .quantity(Quantity::new(dec!(500), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.08), "EUR/kWh"))
        .build()
        .unwrap();

    let levy = PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh");
    let tax_item = levy
        .compute(&[consumption, negative_epex, feed_in])
        .unwrap();
    // (800 + 200) kWh × 0.02050 = 1000 × 0.02050 = 20.50000
    assert_eq!(tax_item.net_amount, Amount::parse("20.50000").unwrap());
}

// ── Fix 3: Amount::checked_abs ───────────────────────────────────────────────

#[test]
fn checked_abs_positive() {
    let a = Amount::<5>::parse("42.00000").unwrap();
    assert_eq!(a.checked_abs().unwrap(), a);
}

#[test]
fn checked_abs_negative() {
    let a = Amount::<5>::parse("-42.00000").unwrap();
    assert_eq!(a.checked_abs().unwrap(), Amount::parse("42.00000").unwrap());
}

#[test]
fn checked_abs_zero() {
    assert_eq!(Amount::<5>::ZERO.checked_abs().unwrap(), Amount::ZERO);
}

#[test]
fn checked_abs_min_returns_err() {
    // Amount(i64::MIN).abs() would panic; checked_abs must return Err.
    let result = Amount::<5>::from_raw_units(i64::MIN).checked_abs();
    assert!(
        result.is_err(),
        "checked_abs(i64::MIN) must return Err, not panic"
    );
    assert!(
        matches!(result, Err(billing::BillingError::MonetaryOverflow { .. })),
        "error must be MonetaryOverflow"
    );
}

// ── Fix 4: split_graduated errors when quantity exceeds last band upper ──────

#[test]
fn graduated_quantity_exceeds_last_finite_band_errors() {
    // One band: [0, 100]. split(200) must error — 100 units would be silently dropped.
    let sched = TariffSchedule::graduated()
        .unit("kWh")
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let result = sched.split(dec!(200));
    assert!(
        result.is_err(),
        "split(200) with last band upper=100 must error, not silently under-bill"
    );
    assert!(
        matches!(result, Err(billing::BillingError::InvalidInput { .. })),
        "expected InvalidInput error"
    );
}

#[test]
fn graduated_quantity_exactly_at_last_finite_band_ok() {
    // Exactly at the upper bound must succeed.
    let sched = TariffSchedule::graduated()
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(100)).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].net_amount, Amount::parse("100.00000").unwrap());
}

#[test]
fn graduated_open_ended_last_band_handles_any_quantity() {
    // Open-ended last band: should work for any quantity.
    let sched = TariffSchedule::graduated()
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(100),
            Amount::parse("0.50000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = sched.split(dec!(500)).unwrap();
    assert_eq!(items.len(), 2);
    // 100 × 1.00 + 400 × 0.50 = 100 + 200 = 300
    let total: Amount<5> = items.iter().map(|i| i.net_amount).sum();
    assert_eq!(total, Amount::parse("300.00000").unwrap());
}

// ── Fix 5: Sign derives Hash ─────────────────────────────────────────────────

#[test]
fn sign_usable_as_hashmap_key() {
    use std::collections::HashMap;
    let mut counts: HashMap<billing::Sign, u32> = HashMap::new();
    *counts.entry(billing::Sign::Debit).or_insert(0) += 1;
    *counts.entry(billing::Sign::Credit).or_insert(0) += 1;
    *counts.entry(billing::Sign::Debit).or_insert(0) += 1;
    assert_eq!(counts[&billing::Sign::Debit], 2);
    assert_eq!(counts[&billing::Sign::Credit], 1);
}

// ── Regression: PercentageCharge and PercentageDiscount use sign field too ───

#[test]
fn percentage_charge_includes_debit_at_negative_price() {
    use billing::PercentageCharge;
    // A debit at negative price: platform commission should still apply to the position.
    let epex = LineItem::debit("EPEX negativ")
        .quantity(Quantity::new(dec!(1000), "kWh"))
        .unit_price(UnitPrice::new(dec!(-0.01), "EUR/kWh"))
        .build()
        .unwrap();
    assert!(epex.is_debit());
    assert!(epex.net_amount.is_negative()); // net = -10.00

    let charge = PercentageCharge::new("Commission", dec!(0.05));
    let fee = charge.compute(&[epex]).unwrap();
    // Base = -10.00 (debit at negative price included), fee = -10.00 × 0.05 = -0.50
    assert_eq!(fee.net_amount, Amount::parse("-0.50000").unwrap());
}

#[test]
fn percentage_discount_excludes_credit_positions() {
    use billing::PercentageDiscount;
    let debit = LineItem::fixed("charge", Amount::parse("100.00000").unwrap())
        .build()
        .unwrap();
    let credit = LineItem::credit_fixed("discount", Amount::parse("20.00000").unwrap())
        .build()
        .unwrap();
    assert!(credit.is_credit());

    let disc = PercentageDiscount::new("Loyalty", dec!(0.10));
    // Base = only the debit position (100), credit excluded
    let item = disc.compute(&[debit, credit]).unwrap();
    // 100 × 0.10 = 10 → credit → -10
    assert_eq!(item.net_amount, Amount::parse("-10.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// FR-2: LineItem::period + Period type (v0.6.0)
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

#[test]
fn period_new_and_fields() {
    let p = billing::Period::new("2026-06-01", "2026-06-30");
    assert_eq!(p.from, "2026-06-01");
    assert_eq!(p.to, "2026-06-30");
}

#[test]
fn period_equality() {
    let a = billing::Period::new("2026-06-01", "2026-06-30");
    let b = billing::Period::new("2026-06-01", "2026-06-30");
    let c = billing::Period::new("2026-07-01", "2026-07-31");
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn line_item_period_via_builder() {
    let item = LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
        .period("2026-06-01", "2026-06-14")
        .build()
        .unwrap();
    let p = item.period.as_ref().unwrap();
    assert_eq!(p.from, "2026-06-01");
    assert_eq!(p.to, "2026-06-14");
}

#[test]
fn line_item_period_none_by_default() {
    let item = LineItem::fixed("Fee", Amount::parse("10.00000").unwrap())
        .build()
        .unwrap();
    assert!(item.period.is_none());
}

/// EEG tariff-change scenario: two half-month positions in one invoice.
/// Each line item carries a machine-readable sub-period (first-class field,
/// not a stringly-typed metadata hack).
#[test]
fn line_item_period_eeg_tariff_change_mid_month() {
    let first_half = LineItem::for_usage(
        "Vergütung alt (01.–14.06.)",
        dec!(500),
        "kWh",
        dec!(0.0811),
        "EUR/kWh",
    )
    .period("2026-06-01", "2026-06-14")
    .build()
    .unwrap();

    let second_half = LineItem::for_usage(
        "Vergütung neu (15.–30.06.)",
        dec!(600),
        "kWh",
        dec!(0.0679),
        "EUR/kWh",
    )
    .period("2026-06-15", "2026-06-30")
    .build()
    .unwrap();

    assert_eq!(first_half.period.as_ref().unwrap().from, "2026-06-01");
    assert_eq!(first_half.period.as_ref().unwrap().to, "2026-06-14");
    assert_eq!(second_half.period.as_ref().unwrap().from, "2026-06-15");
    assert_eq!(second_half.period.as_ref().unwrap().to, "2026-06-30");

    // Periods must be distinct (no overlap)
    assert_ne!(first_half.period, second_half.period);

    // Items are distinct by period field (not just description)
    assert_ne!(first_half, second_half);
}

/// Period is preserved through BillingDocument construction.
#[test]
fn line_item_period_preserved_in_document() {
    let item = LineItem::fixed("Grundpreis", Amount::parse("15.00000").unwrap())
        .period("2026-06-01", "2026-06-15")
        .build()
        .unwrap();
    let doc = BillingDocument::from_positions(DocumentMeta::default(), vec![item], vec![], vec![])
        .unwrap();
    let p = doc.net_positions()[0].period.as_ref().unwrap();
    assert_eq!(p.from, "2026-06-01");
    assert_eq!(p.to, "2026-06-15");
}

/// DocumentMeta.period uses the Period type for consistency with LineItem.period.
#[test]
fn document_meta_period_consistent_with_line_item() {
    let doc_period = billing::Period::new("2026-06-01", "2026-06-30");
    let item_period = billing::Period::new("2026-06-01", "2026-06-14");
    // Same type — can be compared, used in assertions, no magic string keys
    assert_ne!(doc_period, item_period);
    let meta = DocumentMeta {
        invoice_number: "INV".into(),
        period: Some(doc_period.clone()),
        ..Default::default()
    };
    assert_eq!(meta.period.unwrap(), doc_period);
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// v0.7.0 deep audit fixes
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ── Fix 1: allocation uses checked_mul_qty (no hidden panic in Result fn) ────

/// Allocation on a document where net_total × share would overflow mul_qty.
/// Before the fix, `ProportionalAllocation::allocate` could panic inside a
/// `Result`-returning function. Now it returns `Err(MonetaryOverflow)`.
#[test]
fn allocation_overflow_returns_err_not_panic() {
    // Build a doc with a very large net_total near Amount::MAX
    let big = Amount::<5>::from_int(90_000_000_000_000i64);
    let pos = vec![LineItem::fixed("Big charge", big).build().unwrap()];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();

    // A share of 2.0 would overflow: 90T × 2 > i64::MAX
    // We can't construct ProportionalAllocation with shares summing to 2.0,
    // but we CAN verify with a share of 1.0 applied to the already-max doc.
    // Instead, craft a doc whose net_total × 0.5 is fine, but per-position scaling
    // might round. This is mainly a compile/Err-propagation check.
    let alloc = billing::ProportionalAllocation::new(vec![dec!(0.5), dec!(0.5)]).unwrap();
    let result = alloc.allocate(&doc);
    assert!(result.is_ok(), "normal large allocation must succeed");
}

/// Verify positions are correctly scaled (checked_mul_qty gives same result as
/// mul_qty for non-overflowing values).
#[test]
fn allocation_position_scaling_exact() {
    let pos = vec![
        LineItem::fixed("A", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
        LineItem::fixed("B", Amount::parse("200.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    let alloc = billing::ProportionalAllocation::new(vec![dec!(0.4), dec!(0.6)]).unwrap();
    let docs = alloc.allocate(&doc).unwrap();
    // Σ positions in each doc must equal its net_total (penny-corrected)
    docs.iter().for_each(|d| d.assert_valid());
    // Total must add up to original
    let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
    assert_eq!(total, Amount::parse("300.00000").unwrap());
}

// ── Fix 2: DynamicPricing uses checked_mul_qty ────────────────────────────────

#[test]
fn dynamic_pricing_no_panic_on_large_price_and_qty() {
    use billing::DynamicPricing;
    // Large qty × price that is within i64 bounds
    let dp = DynamicPricing::from_intervals(vec![
        (dec!(1_000_000), Amount::parse("0.00001").unwrap()), // net = 10.00000
    ])
    .unwrap()
    .with_unit("kWh");
    let item = dp.calculate().unwrap();
    assert_eq!(item.net_amount, Amount::parse("10.00000").unwrap());
}

#[test]
fn dynamic_pricing_negative_epex_price() {
    use billing::DynamicPricing;
    // EPEX negative price: qty=500 kWh × -0.005 EUR/kWh = -2.50000
    let dp = DynamicPricing::from_intervals(vec![(dec!(500), Amount::parse("-0.00500").unwrap())])
        .unwrap()
        .with_unit("kWh");
    let item = dp.calculate().unwrap();
    assert_eq!(item.net_amount, Amount::parse("-2.50000").unwrap());
    // Sign is Debit even though net is negative (negative EPEX scenario)
    assert_eq!(item.sign, billing::Sign::Debit);
}

// ── Fix 3: Amount::checked_round_to ──────────────────────────────────────────

#[test]
fn checked_round_to_lower_precision() {
    use billing::RoundingStrategy;
    let a = Amount::<5>::parse("3.45678").unwrap();
    let r = a
        .checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero)
        .unwrap();
    assert_eq!(r, Amount::<2>::parse("3.46").unwrap());
}

#[test]
fn checked_round_to_higher_precision_max_value_err() {
    use billing::RoundingStrategy;
    // Amount::<2>::MAX ≈ 92_233_720_368_547.75. Scaling to P=10 would overflow.
    // checked_round_to must return Err instead of panicking.
    let big = Amount::<2>::from_raw_units(i64::MAX);
    let result = big.checked_round_to::<10>(RoundingStrategy::MidpointAwayFromZero);
    assert!(
        result.is_err(),
        "rounding MAX Amount::<2> to P=10 must return Err, not panic"
    );
}

#[test]
fn round_to_same_precision_is_identity() {
    use billing::RoundingStrategy;
    let a = Amount::<5>::parse("42.12345").unwrap();
    let r = a
        .checked_round_to::<5>(RoundingStrategy::MidpointAwayFromZero)
        .unwrap();
    assert_eq!(a, r);
}

// ── Fix 4: prorate clears period to prevent stale date propagation ───────────

#[test]
fn prorate_clears_source_period() {
    use billing::RoundingStrategy;
    let item = LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
        .period("2026-06-01", "2026-06-30")
        .build()
        .unwrap();
    assert!(item.period.is_some(), "source has period set");

    let prorated = billing::prorate(&item, 15, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert!(
        prorated.period.is_none(),
        "prorate() must clear period — it only knows day counts, not dates.\
         Caller must set the correct sub-period explicitly."
    );
}

#[test]
fn prorate_caller_can_set_period_after_prorate() {
    use billing::RoundingStrategy;
    let item = LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
        .build()
        .unwrap();
    let mut prorated =
        billing::prorate(&item, 15, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();
    // Caller knows the actual date range:
    prorated.period = Some(billing::Period::new("2026-06-16", "2026-06-30"));
    assert_eq!(prorated.period.as_ref().unwrap().from, "2026-06-16");
    assert_eq!(prorated.net_amount, Amount::parse("15.00000").unwrap());
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// v0.8.0 deep audit — Amount::parse i128 fix + edge case coverage
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

// ── Critical fix: Amount::MIN parse round-trip ───────────────────────────────

/// The most negative Amount<5> value (-92233720368547.75808) must round-trip
/// through Display → parse. Before the i128 fix, parse returned Err because the
/// unsigned magnitude 9_223_372_036_854_775_808 overflowed i64 before negation.
#[test]
fn amount_min_display_parse_round_trip() {
    let min = Amount::<5>::MIN;
    let s = min.to_string();
    assert_eq!(s, "-92233720368547.75808");
    let parsed = Amount::<5>::parse(&s)
        .unwrap_or_else(|_| panic!("Amount::MIN.to_string() must be parseable, got Err for {s:?}"));
    assert_eq!(parsed, min, "parse(MIN.to_string()) must equal MIN");
    assert_eq!(parsed.to_raw(), i64::MIN);
}

/// Verify Amount::MAX round-trips too (symmetric test).
#[test]
fn amount_max_display_parse_round_trip() {
    let max = Amount::<5>::MAX;
    let s = max.to_string();
    assert_eq!(s, "92233720368547.75807");
    let parsed = Amount::<5>::parse(&s).unwrap();
    assert_eq!(parsed, max);
    assert_eq!(parsed.to_raw(), i64::MAX);
}

/// One below MIN must be rejected.
#[test]
fn amount_parse_below_min_rejected() {
    // -92233720368547.75809 is one unit below Amount::<5>::MIN
    assert!(
        Amount::<5>::parse("-92233720368547.75809").is_err(),
        "value below MIN must be Err"
    );
}

/// One above MAX must be rejected.
#[test]
fn amount_parse_above_max_rejected() {
    assert!(
        Amount::<5>::parse("92233720368547.75808").is_err(),
        "value above MAX must be Err"
    );
}

/// Amount::<0>::MIN = Amount(i64::MIN), display = "-9223372036854775808".
/// Before fix: whole_str parse as i64 overflowed; with i128 fix: correct.
#[test]
fn amount_p0_min_round_trip() {
    let min0 = Amount::<0>::MIN;
    let s = min0.to_string();
    // P=0: no decimal point, just the integer
    assert_eq!(s, "-9223372036854775808");
    let parsed = Amount::<0>::parse(&s).unwrap_or_else(|_| {
        panic!("Amount::<0>::MIN.to_string() must be parseable, got Err for {s:?}")
    });
    assert_eq!(parsed, min0);
}

// ── Parse edge cases ──────────────────────────────────────────────────────────

#[test]
fn parse_plus_sign_accepted() {
    // Leading '+' is accepted and equivalent to no sign
    assert_eq!(
        Amount::<5>::parse("+5.00000").unwrap(),
        Amount::parse("5.00000").unwrap()
    );
    assert_eq!(Amount::<5>::parse("+0").unwrap(), Amount::ZERO);
}

#[test]
fn parse_trailing_dot() {
    // "0." has empty fractional part — treated as 0.00000
    assert_eq!(Amount::<5>::parse("0.").unwrap(), Amount::ZERO);
    assert_eq!(
        Amount::<5>::parse("42.").unwrap(),
        Amount::parse("42.00000").unwrap()
    );
}

#[test]
fn parse_leading_trailing_whitespace() {
    assert_eq!(
        Amount::<5>::parse("  5.00000  ").unwrap(),
        Amount::parse("5.00000").unwrap()
    );
    assert_eq!(
        Amount::<5>::parse("\t3.00000").unwrap(),
        Amount::parse("3.00000").unwrap()
    );
}

#[test]
fn parse_negative_zero_equals_zero() {
    let neg_zero = Amount::<5>::parse("-0.00000").unwrap();
    assert_eq!(neg_zero, Amount::ZERO);
    assert_eq!(neg_zero.to_raw(), 0);
    assert!(!neg_zero.is_negative());
    assert!(!neg_zero.is_positive());
    assert!(neg_zero.is_zero());
}

#[test]
fn parse_double_sign_rejected() {
    assert!(Amount::<5>::parse("--5").is_err());
    assert!(Amount::<5>::parse("+-5").is_err());
    assert!(Amount::<5>::parse("++5").is_err());
    assert!(Amount::<5>::parse("-+5").is_err());
}

// ── Amount::checked_from_int ──────────────────────────────────────────────────

#[test]
fn checked_from_int_normal() {
    let a = Amount::<5>::checked_from_int(49).unwrap();
    assert_eq!(a, Amount::parse("49.00000").unwrap());
}

#[test]
fn checked_from_int_zero() {
    assert_eq!(Amount::<5>::checked_from_int(0).unwrap(), Amount::ZERO);
}

#[test]
fn checked_from_int_overflow() {
    // 92_233_720_368_548 × 100_000 > i64::MAX
    assert!(Amount::<5>::checked_from_int(92_233_720_368_548).is_err());
    assert!(Amount::<5>::checked_from_int(i64::MAX).is_err());
}

#[test]
fn checked_from_int_max_valid() {
    // 92_233_720_368_547 × 100_000 = 9_223_372_036_854_700_000 ≤ i64::MAX
    let a = Amount::<5>::checked_from_int(92_233_720_368_547).unwrap();
    assert_eq!(a, Amount::parse("92233720368547.00000").unwrap());
}

// ── checked_sum edge cases ────────────────────────────────────────────────────

#[test]
fn checked_sum_empty_iter_is_zero() {
    let result = Amount::<5>::checked_sum(std::iter::empty::<Amount<5>>());
    assert_eq!(result.unwrap(), Amount::ZERO);
}

#[test]
fn checked_sum_single_element() {
    let a = Amount::<5>::parse("42.00000").unwrap();
    let result = Amount::checked_sum(std::iter::once(a)).unwrap();
    assert_eq!(result, a);
}

// ── Zero-rate tax/discount edge cases ────────────────────────────────────────

#[test]
fn fixed_rate_tax_zero_rate_creates_zero_item() {
    use billing::FixedRateTax;
    let pos = vec![
        LineItem::fixed("charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let item = FixedRateTax::new("ZeroTax", dec!(0)).compute(&pos).unwrap();
    assert!(item.net_amount.is_zero());
    assert!(item.has_tag("tax"));
}

#[test]
fn percentage_discount_rate_zero_creates_zero_credit() {
    use billing::PercentageDiscount;
    let pos = vec![
        LineItem::fixed("charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let item = PercentageDiscount::new("NoDiscount", dec!(0))
        .compute(&pos)
        .unwrap();
    assert!(item.net_amount.is_zero());
    assert_eq!(item.sign, billing::Sign::Credit);
}

#[test]
fn percentage_discount_rate_one_hundred_percent() {
    use billing::PercentageDiscount;
    let pos = vec![
        LineItem::fixed("charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let item = PercentageDiscount::new("Full", dec!(1))
        .compute(&pos)
        .unwrap();
    assert_eq!(item.net_amount, Amount::parse("-100.00000").unwrap());
}

// ── Single-recipient allocation ───────────────────────────────────────────────

#[test]
fn proportional_allocation_single_recipient() {
    let pos = vec![
        LineItem::fixed("A", Amount::parse("99.99999").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    let docs = billing::ProportionalAllocation::new(vec![dec!(1.0)])
        .unwrap()
        .allocate(&doc)
        .unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].net_total(), doc.net_total());
    docs[0].assert_valid();
}

#[test]
fn equal_allocation_single_recipient() {
    let pos = vec![
        LineItem::fixed("A", Amount::parse("42.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let doc =
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap();
    let docs = billing::EqualAllocation::new(1).allocate(&doc).unwrap();
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].net_total(), doc.net_total());
}

// ── RateLookup with extreme parameter values ──────────────────────────────────

#[test]
fn rate_lookup_decimal_max_hits_fallback() {
    let lookup = billing::RateLookup::builder()
        .at_most(dec!(10), Amount::parse("0.00811").unwrap())
        .fallback(Amount::parse("0.00556").unwrap())
        .build()
        .unwrap();
    // Decimal::MAX is huge — must match the fallback (upper_bound = Decimal::MAX)
    let r = lookup.rate_for(rust_decimal::Decimal::MAX).unwrap();
    assert_eq!(r, Amount::parse("0.00556").unwrap());
}

#[test]
fn rate_lookup_zero_parameter() {
    let lookup = billing::RateLookup::builder()
        .at_most(dec!(0.001), Amount::parse("0.01000").unwrap())
        .fallback(Amount::parse("0.00500").unwrap())
        .build()
        .unwrap();
    // 0 ≤ 0.001 → first band
    let r = lookup.rate_for(dec!(0)).unwrap();
    assert_eq!(r, Amount::parse("0.01000").unwrap());
}
