//! Regression tests for the v0.7.0 correctness audit.
//!
//! Every test here pins a defect that was present in v0.6.0 and is now fixed.
//! Each names the concrete failure mode, so a future refactor that reintroduces
//! it fails with an explanation rather than a bare assertion.

use billing::prelude::*;
use billing::{FixedDiscount, FixedRateTax, PerUnitLevy, PercentageCharge, PercentageDiscount};
use rust_decimal::Decimal;
use rust_decimal::dec;

// ─────────────────────────────────────────────────────────────────────────────
// 1. `checked_*` APIs must never panic
//
// `rust_decimal`'s `Mul`/`Add`/`Div` operators PANIC on overflow rather than
// saturating. Several APIs documented as fallible used them internally, so they
// aborted the process instead of returning `Err` — the worst possible outcome in
// a billing pipeline, where a single malformed record could take down a batch run.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn checked_mul_qty_returns_err_instead_of_panicking_on_decimal_overflow() {
    let price = Amount::<5>::parse("1000000.00000").unwrap();
    assert!(price.checked_mul_qty(Decimal::MAX).is_err());
    assert!(price.checked_mul_qty(Decimal::MIN).is_err());
}

#[test]
fn decimal_conversion_returns_err_instead_of_panicking() {
    // `Decimal`'s own `Mul` panics on overflow; every conversion here must not.
    assert!(Amount::<5>::checked_from_decimal(Decimal::MAX).is_err());
    assert!(Amount::<5>::checked_from_decimal(Decimal::MIN).is_err());
    assert!(Amount::<5>::try_from(Decimal::MAX).is_err());
    assert!(
        Amount::<5>::from_decimal_rounded(Decimal::MAX, RoundingStrategy::MidpointAwayFromZero)
            .is_err()
    );
    assert!(Amount::<5>::from_decimal_rounded(Decimal::MIN, RoundingStrategy::Floor).is_err());
}

/// The two conversion paths into `Amount` must agree on what is representable.
///
/// `parse` has always rejected non-zero digits beyond `P`; `checked_from_decimal`
/// used to round them away, so the same unrepresentable price was refused as text
/// and silently altered as a `Decimal`.
#[test]
fn string_and_decimal_conversions_agree_on_representability() {
    for s in ["0.123456", "1.000011", "-0.000001"] {
        let d = Decimal::from_str_exact(s).unwrap();
        assert!(Amount::<5>::parse(s).is_err(), "parse must reject {s}");
        assert!(
            matches!(
                Amount::<5>::checked_from_decimal(d),
                Err(BillingError::PrecisionLoss { .. })
            ),
            "checked_from_decimal must reject {s}"
        );
    }
    // Trailing zeros carry no information — both paths accept them.
    for s in ["1.100000", "49.990000"] {
        let d = Decimal::from_str_exact(s).unwrap();
        assert!(Amount::<5>::parse(s).is_ok(), "parse must accept {s}");
        assert!(
            Amount::<5>::checked_from_decimal(d).is_ok(),
            "checked_from_decimal must accept {s}"
        );
    }
}

#[test]
fn line_item_build_returns_err_instead_of_panicking_on_extreme_inputs() {
    let r = LineItem::for_usage(
        "x",
        Quantity::new(Decimal::MAX, "u"),
        UnitPrice::new(Decimal::MAX, "c/u"),
    )
    .build();
    assert!(r.is_err(), "quantity × price overflow must be an Err");
}

#[test]
fn prorate_and_scaled_never_panic_on_extreme_amounts() {
    let item = LineItem::fixed("Huge", Amount::<5>::MAX).build().unwrap();
    // Scaling down is fine; the point is that no path panics.
    assert!(prorate(&item, 1, 365, RoundingStrategy::MidpointAwayFromZero).is_ok());
    assert!(item.scaled(dec!(1), RoundingStrategy::Truncate).is_ok());
}

#[test]
fn dynamic_pricing_calculate_is_fallible_not_panicking() {
    let dp = DynamicPricing::builder()
        .interval(Decimal::MAX, Amount::parse("1.00000").unwrap())
        .interval(Decimal::MAX, Amount::parse("1.00000").unwrap())
        .build()
        .unwrap();
    assert!(dp.calculate().is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Scaled positions stay internally consistent
//
// `prorate` and the allocation rules used to scale `net_amount` while leaving
// `quantity` untouched, producing invoice lines that contradict themselves —
// "1000 kWh × 0.30 EUR/kWh = 150.00". An auditor recomputing the line from its
// own quantity and price gets a different number than the line states.
// ─────────────────────────────────────────────────────────────────────────────

/// `quantity × unit_price` must reproduce `net_amount` (within one 1e-5 unit,
/// which is the unavoidable residue of penny correction).
fn assert_line_self_consistent(item: &LineItem) {
    let (Some(q), Some(p)) = (item.quantity.as_ref(), item.unit_price.as_ref()) else {
        return; // fixed-amount lines carry no quantity to cross-check
    };
    let expected = Amount::<5>::from_decimal_rounded(
        q.value * p.value,
        RoundingStrategy::MidpointAwayFromZero,
    )
    .unwrap();
    let diff = (expected.to_raw() - item.net_amount.to_raw()).abs();
    assert!(
        diff <= 1,
        "line {:?} is self-contradictory: {} {} × {} = {} but net_amount is {}",
        item.description,
        q.value,
        q.unit,
        p.value,
        expected,
        item.net_amount
    );
}

#[test]
fn prorate_scales_quantity_alongside_amount() {
    let full = LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    )
    .build()
    .unwrap();
    let half = prorate(&full, 15, 30, RoundingStrategy::MidpointAwayFromZero).unwrap();

    assert_eq!(half.net_amount, Amount::<5>::parse("150.00000").unwrap());
    assert_eq!(
        half.quantity_value(),
        Some(dec!(500)),
        "quantity must be prorated too, not left at the full-period value"
    );
    assert_line_self_consistent(&half);
}

#[test]
fn allocated_positions_are_self_consistent() {
    let positions = vec![
        LineItem::for_usage(
            "Arbeit",
            Quantity::new(dec!(1000), "kWh"),
            UnitPrice::new(dec!(0.30), "EUR/kWh"),
        )
        .build()
        .unwrap(),
        LineItem::for_usage(
            "Gas",
            Quantity::new(dec!(500), "m³"),
            UnitPrice::new(dec!(0.12), "EUR/m³"),
        )
        .build()
        .unwrap(),
    ];
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        positions,
        vec![],
        vec![],
    )
    .unwrap();

    let alloc = ProportionalAllocation::new(vec![dec!(0.4), dec!(0.35), dec!(0.25)]).unwrap();
    let docs = alloc.allocate(&doc).unwrap();

    for d in &docs {
        d.assert_valid();
        for pos in d.all_positions() {
            assert_line_self_consistent(pos);
        }
    }
    // Exactness is preserved on top of the consistency fix.
    let total: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
    assert_eq!(total, doc.net_total());
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Validation cannot be bypassed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn proportional_allocation_rejects_negative_shares_that_sum_to_one() {
    // [1.5, -0.5] sums to exactly 1.0, so the sum check alone accepted it and one
    // "recipient" was handed a credit funded by the other.
    assert!(ProportionalAllocation::new(vec![dec!(1.5), dec!(-0.5)]).is_err());
    assert!(ProportionalAllocation::new(vec![]).is_err());
}

#[test]
fn equal_allocation_rejects_zero_recipients() {
    assert!(EqualAllocation::new(0).is_err());
    assert!(EqualAllocation::new(1).is_ok());

    // `n()` reports what was asked for, not a constant that happens to match the
    // common case.
    for n in [1usize, 2, 3, 7, 64] {
        assert_eq!(EqualAllocation::new(n).unwrap().n(), n);
    }
}

#[test]
fn proportional_allocation_reports_the_shares_it_was_built_from() {
    let shares = vec![dec!(0.4), dec!(0.35), dec!(0.25)];
    let alloc = ProportionalAllocation::new(shares.clone()).unwrap();
    assert_eq!(alloc.shares(), shares.as_slice());
    assert_eq!(alloc.shares().len(), 3);
    assert_eq!(alloc.shares().iter().sum::<Decimal>(), Decimal::ONE);
}

#[test]
fn allocated_documents_are_numbered_per_recipient() {
    // Every recipient gets a document, and every document needs its own BT-1.
    // Handing all three the parent's number produces invoices that are
    // indistinguishable in a ledger — and, under §14 UStG, not lawfully numbered.
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "R-2026-0007".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Grundpreis", Amount::parse("900.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();

    let docs = ProportionalAllocation::new(vec![dec!(0.4), dec!(0.35), dec!(0.25)])
        .unwrap()
        .allocate(&doc)
        .unwrap();

    let numbers: Vec<&str> = docs
        .iter()
        .map(|d| d.meta.invoice_number.as_str())
        .collect();
    assert_eq!(numbers, ["R-2026-0007/1", "R-2026-0007/2", "R-2026-0007/3"]);
    // Distinct, and none of them silently reuses the parent's number.
    assert!(!numbers.contains(&"R-2026-0007"));

    // The rest of the header is inherited rather than reset.
    for d in &docs {
        assert_eq!(d.meta.currency, doc.meta.currency);
        assert_eq!(d.meta.kind, doc.meta.kind);
    }
}

#[test]
fn schedule_rejects_non_monotonic_bands() {
    // `up_to` leaves `lower` as None, so the contiguity check never fired and this
    // built successfully, then failed later inside split() with a misleading
    // "quantity exceeds coverage" message.
    let r = RateSchedule::graduated()
        .band(RateBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .band(RateBand::up_to(dec!(50), Amount::parse("2.00000").unwrap()))
        .build();
    assert!(
        r.is_err(),
        "descending upper bounds must be rejected at build time"
    );

    // Equal consecutive bounds are a zero-width band — also rejected.
    assert!(
        RateSchedule::graduated()
            .band(RateBand::up_to(
                dec!(100),
                Amount::parse("1.00000").unwrap()
            ))
            .band(RateBand::up_to(
                dec!(100),
                Amount::parse("2.00000").unwrap()
            ))
            .build()
            .is_err()
    );

    // The ascending equivalent still builds and prices correctly.
    let ok = RateSchedule::graduated()
        .band(RateBand::up_to(dec!(50), Amount::parse("1.00000").unwrap()))
        .band(RateBand::up_to(
            dec!(100),
            Amount::parse("2.00000").unwrap(),
        ))
        .build()
        .unwrap();
    let items = ok.split(dec!(80)).unwrap();
    // 50 × 1.00 + 30 × 2.00 = 110.00
    let total: Amount<5> = items.iter().map(|i| i.net_amount).sum();
    assert_eq!(total, Amount::<5>::parse("110.00000").unwrap());
}

#[test]
fn rate_lookup_rejects_duplicate_and_non_positive_bounds() {
    assert!(
        RateLookup::builder()
            .at_most(dec!(10), Amount::parse("0.00811").unwrap())
            .at_most(dec!(10), Amount::parse("0.00679").unwrap())
            .build()
            .is_err(),
        "a duplicate upper_bound makes the second entry unreachable"
    );
    assert!(
        RateLookup::builder()
            .at_most(dec!(0), Amount::parse("0.00811").unwrap())
            .build()
            .is_err()
    );
    assert!(
        RateLookup::builder()
            .fallback(Amount::parse("0.001").unwrap())
            .fallback(Amount::parse("0.002").unwrap())
            .build()
            .is_err(),
        "two fallbacks is a config error, not a silent first-wins"
    );
}

#[test]
fn tou_rejects_empty_duplicate_and_negative_bands() {
    assert!(TimeOfUsePricing::builder().build().is_err());
    assert!(
        TimeOfUsePricing::builder()
            .band(TimeBand::new("HT", Amount::parse("0.32").unwrap()))
            .band(TimeBand::new("HT", Amount::parse("0.18").unwrap()))
            .build()
            .is_err(),
        "a duplicate band name is unreachable and almost always a config error"
    );
    assert!(
        TimeOfUsePricing::builder()
            .band(TimeBand::new("HT", Amount::parse("-0.1").unwrap()))
            .build()
            .is_err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Currency is explicit, never assumed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn generated_labels_use_the_configured_currency_not_hardcoded_eur() {
    let sched = RateSchedule::graduated()
        .unit("GB")
        .currency(Currency::USD)
        .band(RateBand::over(dec!(0), Amount::parse("0.10000").unwrap()))
        .build()
        .unwrap();
    let items = sched.split(dec!(100)).unwrap();
    assert_eq!(items[0].unit_price.as_ref().unwrap().unit, "USD/GB");
}

#[test]
fn unconfigured_currency_is_visible_as_xxx_not_silently_eur() {
    let sched = RateSchedule::graduated()
        .unit("GB")
        .band(RateBand::over(dec!(0), Amount::parse("0.10000").unwrap()))
        .build()
        .unwrap();
    let items = sched.split(dec!(100)).unwrap();
    // ISO 4217 "no currency involved" — a loud placeholder, not a wrong answer.
    assert_eq!(items[0].unit_price.as_ref().unwrap().unit, "XXX/GB");
}

#[test]
fn merging_documents_across_currencies_is_rejected() {
    let mk = |c: Currency| {
        BillingDocument::from_positions(
            DocumentMeta {
                currency: c,
                ..Default::default()
            },
            vec![
                LineItem::fixed("x", Amount::parse("10.00000").unwrap())
                    .build()
                    .unwrap(),
            ],
            vec![],
            vec![],
        )
        .unwrap()
    };
    let err = merge_period_documents(mk(Currency::EUR), mk(Currency::USD)).unwrap_err();
    assert!(matches!(err, BillingError::CurrencyMismatch { .. }));
    assert!(merge_period_documents(mk(Currency::EUR), mk(Currency::EUR)).is_ok());
}

#[test]
fn currency_validation() {
    assert!(Currency::new("EUR").is_ok());
    assert_eq!(Currency::new("chf").unwrap(), Currency::CHF);
    for bad in ["", "EU", "EURO", "E1R", "€"] {
        assert!(Currency::new(bad).is_err(), "{bad:?} must be rejected");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Tax / discount layer semantics
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn invalid_layer_configuration_is_err_not_panic() {
    assert!(FixedRateTax::new("t", dec!(-0.01)).is_err());
    assert!(PercentageCharge::new("c", dec!(-0.01)).is_err());
    assert!(PercentageDiscount::new("d", dec!(1.01)).is_err());
    assert!(PercentageDiscount::new("d", dec!(-0.01)).is_err());
    assert!(FixedDiscount::new("f", Amount::parse("-1.00000").unwrap()).is_err());
    assert!(PerUnitLevy::new("l", Amount::parse("-0.01").unwrap(), "kWh").is_err());
    assert!(PerUnitLevy::new("l", Amount::parse("0.01").unwrap(), "").is_err());
}

#[test]
fn percentage_discount_on_negative_base_does_not_become_an_extra_credit() {
    // Sustained negative spot prices can drive the debit base below zero. A
    // "10% discount" on a -200 base used to yield another -20 credit, increasing
    // the amount owed to the customer.
    let positions = vec![
        LineItem::for_usage(
            "Spot",
            Quantity::new(dec!(1000), "kWh"),
            UnitPrice::new(dec!(-0.2), "EUR/kWh"),
        )
        .build()
        .unwrap(),
    ];
    let disc = PercentageDiscount::new("Loyalty", dec!(0.10)).unwrap();
    let item = disc.compute(&positions).unwrap();
    assert_eq!(item.net_amount, Amount::<5>::ZERO);
}

#[test]
fn discount_total_is_consistent_and_validated() {
    let positions = vec![
        LineItem::fixed("Item", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    let discounts: Vec<Box<dyn DiscountLayer>> = vec![
        Box::new(FixedDiscount::new("Voucher", Amount::parse("15.00000").unwrap()).unwrap()),
        Box::new(PercentageDiscount::new("Loyalty", dec!(0.10)).unwrap()),
    ];
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        positions,
        vec![],
        discounts,
    )
    .unwrap();

    // -15.00 + -10.00
    assert_eq!(
        doc.discount_total(),
        Amount::<5>::parse("-25.00000").unwrap()
    );
    assert_eq!(doc.net_total(), Amount::<5>::parse("75.00000").unwrap());
    doc.assert_valid();
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. serde: representation and validation-on-deserialise
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;

    #[test]
    fn amount_serialises_as_a_decimal_string_not_a_raw_integer() {
        let a = Amount::<5>::parse("0.03456").unwrap();
        // Was `3456` — meaningless without knowing P out of band, and silently
        // rescaled by 10^ΔP if the precision ever changed.
        assert_eq!(serde_json::to_string(&a).unwrap(), "\"0.03456\"");

        let two = Amount::<2>::parse("49.99").unwrap();
        assert_eq!(serde_json::to_string(&two).unwrap(), "\"49.99\"");
    }

    #[test]
    fn amount_roundtrips_exactly_including_extremes() {
        for s in ["0.00000", "0.03456", "-41.60000", "92233720368547.75807"] {
            let a = Amount::<5>::parse(s).unwrap();
            let json = serde_json::to_string(&a).unwrap();
            let back: Amount<5> = serde_json::from_str(&json).unwrap();
            assert_eq!(a, back, "roundtrip failed for {s}");
        }
        let min = Amount::<5>::MIN;
        let back: Amount<5> = serde_json::from_str(&serde_json::to_string(&min).unwrap()).unwrap();
        assert_eq!(min, back);
    }

    #[test]
    fn amount_rejects_floats_and_excess_precision() {
        // A bare JSON number is refused: floats are exactly what a fixed-point
        // monetary type exists to keep out.
        assert!(serde_json::from_str::<Amount<5>>("0.03456").is_err());
        // Excess non-zero precision is rejected rather than silently truncated.
        assert!(serde_json::from_str::<Amount<5>>("\"0.123456\"").is_err());
        assert!(serde_json::from_str::<Amount<5>>("\"0.100000\"").is_ok());
    }

    #[test]
    fn equal_allocation_with_zero_no_longer_deserialises_into_a_divide_by_zero() {
        // `{"n":0}` used to deserialise fine and then panic with "Division by zero"
        // inside allocate().
        assert!(serde_json::from_str::<EqualAllocation>(r#"{"n":0}"#).is_err());
        assert!(serde_json::from_str::<EqualAllocation>(r#"{"n":3}"#).is_ok());
    }

    #[test]
    fn proportional_allocation_validates_on_deserialise() {
        assert!(
            serde_json::from_str::<ProportionalAllocation>(r#"{"shares":["1","1","3"]}"#).is_err()
        );
        assert!(
            serde_json::from_str::<ProportionalAllocation>(r#"{"shares":["1.5","-0.5"]}"#).is_err()
        );
        assert!(
            serde_json::from_str::<ProportionalAllocation>(r#"{"shares":["0.5","0.5"]}"#).is_ok()
        );
    }

    #[test]
    fn tax_layers_validate_on_deserialise() {
        assert!(serde_json::from_str::<FixedRateTax>(r#"{"name":"t","rate":"-0.19"}"#).is_err());
        assert!(serde_json::from_str::<FixedRateTax>(r#"{"name":"t","rate":"0.19"}"#).is_ok());
        assert!(
            serde_json::from_str::<PercentageDiscount>(r#"{"name":"d","rate":"1.5"}"#).is_err()
        );
    }

    #[test]
    fn currency_validates_on_deserialise() {
        assert!(serde_json::from_str::<Currency>(r#""EUR""#).is_ok());
        assert!(serde_json::from_str::<Currency>(r#""EURO""#).is_err());
    }

    #[test]
    fn schedule_roundtrips_and_validates_on_deserialise() {
        let sched = RateSchedule::graduated()
            .unit("m³")
            .currency(Currency::EUR)
            .band(RateBand::up_to(dec!(5), Amount::parse("0.80000").unwrap()))
            .band(RateBand::over(dec!(5), Amount::parse("1.40000").unwrap()))
            .build()
            .unwrap();
        let json = serde_json::to_string(&sched).unwrap();
        let back: RateSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.unit(), "m³");
        assert_eq!(back.currency(), Currency::EUR);
        assert_eq!(
            back.split(dec!(10)).unwrap().len(),
            sched.split(dec!(10)).unwrap().len()
        );

        // A schedule with a gap between bands is refused at the boundary.
        let bad = json.replace(r#""upper":"5""#, r#""upper":"3""#);
        if bad != json {
            assert!(serde_json::from_str::<RateSchedule>(&bad).is_err());
        }
    }

    #[test]
    fn document_revalidates_totals_on_deserialise() {
        let doc = BillingDocument::from_positions(
            DocumentMeta {
                invoice_number: "INV-1".into(),
                currency: Currency::EUR,
                ..Default::default()
            },
            vec![
                LineItem::fixed("Item", Amount::parse("100.00000").unwrap())
                    .build()
                    .unwrap(),
            ],
            vec![Box::new(FixedRateTax::new("VAT", dec!(0.19)).unwrap())],
            vec![],
        )
        .unwrap();

        let json = serde_json::to_string(&doc).unwrap();
        let back: BillingDocument = serde_json::from_str(&json).unwrap();
        assert_eq!(back.gross_total(), doc.gross_total());
        assert_eq!(back.currency(), Currency::EUR);
        back.assert_valid();

        // Tamper with the stored total: it must be rejected, not silently trusted.
        let tampered = json.replace(r#""net_total":"100.00000""#, r#""net_total":"999.00000""#);
        assert_ne!(
            tampered, json,
            "test fixture did not match the serialised form"
        );
        assert!(
            serde_json::from_str::<BillingDocument>(&tampered).is_err(),
            "a document whose totals disagree with its positions must not deserialise"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Defects found by the adversarial verification pass
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn allocation_of_a_taxed_document_produces_valid_documents() {
    // Rounding net, tax and gross INDEPENDENTLY breaks `net + tax == gross`
    // whenever the share does not divide evenly. Splitting 100.00 + 19% three
    // ways produced three documents that each failed validate() by one unit —
    // and `assert_valid()` panicked on all of them.
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Net", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();

    for n in [2usize, 3, 6, 7, 11] {
        let docs = EqualAllocation::new(n).unwrap().allocate(&doc).unwrap();
        assert_eq!(docs.len(), n);
        for (i, d) in docs.iter().enumerate() {
            d.validate()
                .unwrap_or_else(|e| panic!("n={n} recipient {i} is inconsistent: {e}"));
        }
        // Cross-document sums stay exact for all three totals.
        let net: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
        let tax: Amount<5> = docs.iter().map(|d| d.tax_total()).sum();
        let gross: Amount<5> = docs.iter().map(|d| d.gross_total()).sum();
        assert_eq!(net, doc.net_total(), "n={n} net drift");
        assert_eq!(tax, doc.tax_total(), "n={n} tax drift");
        assert_eq!(gross, doc.gross_total(), "n={n} gross drift");
    }

    // Same for uneven proportional shares.
    let docs = ProportionalAllocation::new(vec![dec!(0.37), dec!(0.33), dec!(0.30)])
        .unwrap()
        .allocate(&doc)
        .unwrap();
    for d in &docs {
        d.assert_valid();
    }
    let gross: Amount<5> = docs.iter().map(|d| d.gross_total()).sum();
    assert_eq!(gross, doc.gross_total());
}

#[test]
fn stacked_per_unit_levies_do_not_double_count_quantities() {
    // Each tax layer receives prior layers' output so percentage taxes can
    // compound. But a PerUnitLevy emits a DEBIT line carrying a Quantity in its
    // own unit, so a second levy on the same unit counted that line as
    // consumption and doubled its base — over-billing by 100% on the standard
    // German Stromsteuer + Konzessionsabgabe stack.
    let positions = vec![
        LineItem::for_usage(
            "Arbeit",
            Quantity::new(dec!(1000), "kWh"),
            UnitPrice::new(dec!(0.30), "EUR/kWh"),
        )
        .build()
        .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(
            PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh")
                .unwrap()
                .with_currency(Currency::EUR),
        ),
        Box::new(
            PerUnitLevy::new(
                "Konzessionsabgabe",
                Amount::parse("0.01590").unwrap(),
                "kWh",
            )
            .unwrap()
            .with_currency(Currency::EUR),
        ),
    ];
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        positions,
        taxes,
        vec![],
    )
    .unwrap();

    let levies = doc.tax_positions();
    assert_eq!(levies[0].quantity_value(), Some(dec!(1000)));
    assert_eq!(levies[0].net_amount, Amount::parse("20.50000").unwrap());
    // Was 2000 kWh / 31.80000 before the fix.
    assert_eq!(
        levies[1].quantity_value(),
        Some(dec!(1000)),
        "the second levy must not count the first levy's own line as consumption"
    );
    assert_eq!(levies[1].net_amount, Amount::parse("15.90000").unwrap());
    doc.assert_valid();
}

#[test]
fn proportional_split_is_fallible_not_panicking() {
    // `Decimal::new(1, scale)` panics for scale > 28.
    assert!(proportional_split(dec!(100), &[dec!(1.0)], 29).is_err());
    assert!(proportional_split(dec!(100), &[dec!(1.0)], 28).is_ok());
    // Summing caller-supplied fractions used to panic before any validation ran.
    assert!(proportional_split(dec!(1), &[Decimal::MAX, Decimal::MAX], 2).is_err());
    // Intermediate products used to panic for a large total.
    assert!(proportional_split(Decimal::MAX, &[dec!(0.5), dec!(0.5)], 5).is_err());
}

#[test]
fn block_schedule_with_tiny_block_size_is_fallible_not_panicking() {
    // block_size is only validated as > 0, so a tiny value overflowed Decimal's
    // division inside a Result-returning method.
    let sched = RateSchedule::block()
        .unit("GB")
        .band(RateBand::block(
            Decimal::new(1, 28),
            Amount::parse("1.00000").unwrap(),
        ))
        .build()
        .unwrap();
    assert!(sched.split(dec!(100000000000)).is_err());
}

#[test]
fn scaled_quantity_does_not_explode_in_decimal_scale() {
    // Exact 1/3 scaling produced "99.99999999999999999999999999 kWh" on the
    // invoice line and walked toward Decimal's 28-digit ceiling under repeated
    // scaling.
    let item = LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(300), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    )
    .build()
    .unwrap();
    let third = prorate(&item, 1, 3, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(third.quantity_value(), Some(dec!(100)));

    let item = LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    )
    .build()
    .unwrap();
    let mut cur = item;
    for _ in 0..6 {
        cur = cur
            .scaled(dec!(1) / dec!(3), RoundingStrategy::MidpointAwayFromZero)
            .unwrap();
    }
    assert!(
        cur.quantity_value().unwrap().scale() <= 12,
        "quantity scale grew unbounded: {:?}",
        cur.quantity_value()
    );
}

#[cfg(feature = "serde")]
#[test]
fn line_item_validates_on_deserialise() {
    // LineItem's fields are public, so `build()`'s invariants were bypassable
    // from untrusted JSON: empty descriptions, negative quantities, and
    // Sign::Credit lines with a positive net_amount (which corrupts the
    // sign-based filtering that tax and discount layers rely on).
    let empty_desc = r#"{"description":"   ","quantity":null,"unit_price":null,
        "net_amount":"1.00000","sign":"Debit","period":null,"tags":[],"metadata":{}}"#;
    assert!(serde_json::from_str::<LineItem>(empty_desc).is_err());

    let neg_qty = r#"{"description":"x","quantity":{"value":"-500","unit":"kWh"},
        "unit_price":null,"net_amount":"1.00000","sign":"Debit","period":null,
        "tags":[],"metadata":{}}"#;
    assert!(serde_json::from_str::<LineItem>(neg_qty).is_err());

    let bad_credit = r#"{"description":"x","quantity":null,"unit_price":null,
        "net_amount":"999.00000","sign":"Credit","period":null,"tags":[],"metadata":{}}"#;
    assert!(serde_json::from_str::<LineItem>(bad_credit).is_err());

    // A well-formed item still round-trips.
    let good = LineItem::fixed("Grundpreis", Amount::parse("8.50000").unwrap())
        .tag("fixed")
        .build()
        .unwrap();
    let back: LineItem = serde_json::from_str(&serde_json::to_string(&good).unwrap()).unwrap();
    assert_eq!(good, back);
}

#[test]
fn line_item_validate_catches_post_construction_mutation() {
    let item = LineItem::fixed("Grundpreis", Amount::parse("8.50000").unwrap())
        .build()
        .unwrap();
    assert!(item.validate().is_ok());

    let mut broken = item.clone();
    broken.description = "  ".into();
    assert!(broken.validate().is_err());

    let mut broken = item.clone();
    broken.sign = Sign::Credit; // positive net_amount + Credit is inconsistent
    assert!(broken.validate().is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. Third-round audit findings
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
#[test]
fn a_hostile_vat_breakdown_cannot_panic_the_deserialiser() {
    // `TaxBreakdownEntry::validate` compared `tax_amount - base × rate` with a bare
    // `Decimal` subtraction. Both operands are attacker-controlled and the product
    // is only bounded by Decimal::MAX, so opposing signs aborted the process
    // instead of returning Err — a remote panic from untrusted JSON.
    let hostile = r#"{"category":"Standard",
        "rate":"858993459200000.0000931321575",
        "taxable_base":"92233720368547.75807",
        "tax_amount":"-92233720368547.75808",
        "exemption_reason":null}"#;
    let r = serde_json::from_str::<billing::TaxBreakdownEntry>(hostile);
    assert!(r.is_err(), "must be an Err, not a panic");

    // A well-formed entry still round-trips.
    let good = billing::TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("100.00000").unwrap(),
        Amount::parse("19.00000").unwrap(),
    );
    let json = serde_json::to_string(&good).unwrap();
    assert_eq!(
        serde_json::from_str::<billing::TaxBreakdownEntry>(&json).unwrap(),
        good
    );
}

#[test]
fn with_extra_position_keeps_cash_rounding_consistent() {
    // The gross moved but `rounding` was not recomputed, so the returned document
    // failed its own validate() and reported a non-tenderable amount due.
    let rule = CashRounding::new(
        Amount::parse("0.05000").unwrap(),
        RoundingStrategy::MidpointAwayFromZero,
    )
    .unwrap();
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::CHF,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Leistung", Amount::parse("12.34000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_cash_rounding(rule)
    .unwrap();
    assert_eq!(doc.rounding(), Amount::parse("0.01000").unwrap());

    let doc2 = doc
        .with_extra_position(
            LineItem::fixed("Extra", Amount::parse("1.01000").unwrap())
                .build()
                .unwrap(),
        )
        .unwrap();

    // 13.35 is already a multiple of 0.05, so the adjustment must now be zero.
    doc2.assert_valid();
    assert_eq!(doc2.rounding(), Amount::<5>::ZERO);
    assert_eq!(
        doc2.amount_due().unwrap(),
        Amount::parse("13.35000").unwrap()
    );
    assert_eq!(doc2.amount_due().unwrap().to_raw() % 5_000, 0);
}

#[test]
fn a_prepaid_invoice_can_be_reversed_into_a_valid_credit_note() {
    // reverse() negated prepaid, but validate() rejects a negative BT-113 — so any
    // invoice carrying an advance payment could never be Storno'd.
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Verbrauch", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_prepaid(Amount::parse("900.00000").unwrap())
    .unwrap();

    let credit = doc.reverse(DocumentMeta::default()).unwrap();
    credit.validate().expect("credit note must be valid");
    // The customer paid 900 + 100 = 1000 in total and gets all of it back.
    assert_eq!(
        credit.amount_due().unwrap(),
        Amount::parse("-1000.00000").unwrap()
    );
}

#[test]
fn a_single_band_schedule_is_still_checked_for_contiguity() {
    // The contiguity loop was gated on `bands.len() > 1`, so a lone band declaring
    // a non-zero lower bound built fine and then billed from zero.
    let r = RateSchedule::graduated()
        .band(RateBand::between(
            dec!(500),
            dec!(1000),
            Amount::parse("0.30000").unwrap(),
        ))
        .build();
    assert!(
        r.is_err(),
        "a single band starting at 500 must not silently bill from 0"
    );

    // A band that genuinely starts at zero is still fine.
    assert!(
        RateSchedule::graduated()
            .band(RateBand::up_to(
                dec!(1000),
                Amount::parse("0.30000").unwrap()
            ))
            .build()
            .is_ok()
    );
}

#[test]
fn volume_and_capacity_modes_error_on_uncovered_quantities() {
    // `find_tier_price` fell back to the last band, so a bounded top band silently
    // priced anything above it — while `graduated` correctly errored.
    let volume = RateSchedule::volume()
        .unit("kWh")
        .currency(Currency::EUR)
        .band(RateBand::up_to(
            dec!(1000),
            Amount::parse("0.32000").unwrap(),
        ))
        .build()
        .unwrap();
    assert!(
        volume.split(dec!(5000)).is_err(),
        "5000 exceeds the top bound of 1000 and must not be priced silently"
    );
    assert!(volume.split(dec!(900)).is_ok());

    let capacity = RateSchedule::capacity()
        .unit("Mbps")
        .currency(Currency::EUR)
        .band(RateBand::up_to(
            dec!(100),
            Amount::parse("50.00000").unwrap(),
        ))
        .build()
        .unwrap();
    assert!(capacity.apply_peak(dec!(150)).is_err());
    assert!(capacity.apply_peak(dec!(50)).is_ok());

    // An open-ended top band covers everything, as before.
    let open = RateSchedule::volume()
        .unit("kWh")
        .currency(Currency::EUR)
        .band(RateBand::up_to(
            dec!(1000),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(RateBand::over(
            dec!(1000),
            Amount::parse("0.28000").unwrap(),
        ))
        .build()
        .unwrap();
    assert!(open.split(dec!(5000)).is_ok());
}

#[test]
fn proportional_split_upholds_its_sum_guarantee_or_errors() {
    // The share tolerance was absolute (1e-9) while the resulting error scaled with
    // `total`, so a large total silently lost or invented units.
    let shares = [dec!(0.4999999995), dec!(0.4999999995)];
    // Either outcome is acceptable — upholding the guarantee, or rejecting the
    // shares. What is not acceptable is returning parts that do not sum to `total`.
    if let Ok(parts) = proportional_split(dec!(1000000000000000000), &shares, 0) {
        let sum: Decimal = parts.iter().sum();
        assert_eq!(sum, dec!(1000000000000000000), "guarantee must hold");
    }

    // Shares summing to MORE than one used to return over-allocated parts.
    if let Ok(parts) = proportional_split(dec!(10000000000), &[dec!(0.5), dec!(0.5000000005)], 0) {
        let sum: Decimal = parts.iter().sum();
        assert_eq!(sum, dec!(10000000000));
    }

    // Well-formed input still works exactly.
    let parts =
        proportional_split(dec!(987.654), &[dec!(0.45), dec!(0.35), dec!(0.20)], 3).unwrap();
    let sum: Decimal = parts.iter().sum();
    assert_eq!(sum, dec!(987.654));
}

#[cfg(feature = "serde")]
#[test]
fn validate_rejects_a_fabricated_vat_breakdown_and_a_positive_discount() {
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Item", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();
    let json = serde_json::to_string(&doc).unwrap();

    // A breakdown claiming 19.00 of VAT on a document that charges no tax at all.
    let fabricated = json.replace(
        r#""tax_breakdown":[]"#,
        r#""tax_breakdown":[{"category":"Standard","rate":"0.19","taxable_base":"100.00000","tax_amount":"19.00000","exemption_reason":null}]"#,
    );
    assert_ne!(fabricated, json);
    assert!(serde_json::from_str::<BillingDocument>(&fabricated).is_err());

    // A "discount" that increases the invoice.
    let surcharge = json.replace(
        r#""discount_positions":[]"#,
        r#""discount_positions":[{"description":"x","quantity":null,"unit_price":null,"net_amount":"25.00000","sign":"Debit","period":null,"tags":[],"metadata":{}}]"#,
    );
    assert_ne!(surcharge, json);
    assert!(serde_json::from_str::<BillingDocument>(&surcharge).is_err());
}

#[test]
fn for_usage_rounded_clamps_an_out_of_range_price_scale() {
    // `round_dp_with_strategy` silently no-ops above scale 28, leaving the price
    // unrounded while the caller was promised `price_scale` decimals.
    let item = LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1), "kWh"),
        UnitPrice::new(dec!(0.1234567890123), "EUR/kWh")
            .rounded(99, RoundingStrategy::MidpointAwayFromZero),
    )
    .build()
    .unwrap();
    assert!(item.unit_price.as_ref().unwrap().value.scale() <= 28);
}

// ─────────────────────────────────────────────────────────────────────────────
// 9. Fourth-round audit findings
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn display_honours_width_fill_and_alignment() {
    // `write!` with an inline precision silently discards the formatter's width,
    // so `{:>12}` was a no-op and invoice columns never lined up — including in
    // this crate's own examples.
    let a = Amount::<5>::parse("4.00000").unwrap();
    let b = Amount::<5>::parse("21.00000").unwrap();
    assert_eq!(format!("[{a:>12}]"), "[     4.00000]");
    assert_eq!(format!("[{b:>12}]"), "[    21.00000]");
    assert_eq!(format!("[{a:<12}]"), "[4.00000     ]");
    assert_eq!(format!("[{a:^13}]"), "[   4.00000   ]");
    assert_eq!(format!("[{a:*>12}]"), "[*****4.00000]");
    // Unpadded output is unchanged.
    assert_eq!(format!("{a}"), "4.00000");
    // Numbers right-align by default, like the integer primitives.
    assert_eq!(format!("[{a:12}]"), "[     4.00000]");
    // Padding matches what std produces for the equivalent string, including the
    // odd-padding case where the extra fill goes on the right.
    for w in 0..20usize {
        let plain = a.to_string();
        assert_eq!(format!("{a:>w$}"), format!("{plain:>w$}"), "right w={w}");
        assert_eq!(format!("{a:<w$}"), format!("{plain:<w$}"), "left w={w}");
        assert_eq!(format!("{a:^w$}"), format!("{plain:^w$}"), "center w={w}");
        assert_eq!(format!("{a:*^w$}"), format!("{plain:*^w$}"), "fill w={w}");
    }
    // String-like types honour it too, left-aligned as strings do.
    assert_eq!(format!("[{:<6}]", Currency::EUR), "[EUR   ]");
    assert_eq!(format!("[{:>6}]", TaxCategory::ReverseCharge), "[    AE]");
}

#[test]
fn a_tou_band_cannot_hijack_an_engine_reserved_tag() {
    // `calculate` tags each position with its band name, and `PerUnitLevy` excludes
    // anything tagged "tax" from its base. A band literally named "tax" therefore
    // removed its own consumption from the levy base — 2000 kWh consumed, levy
    // charged on 1000, no error.
    for reserved in [
        "tax",
        "levy",
        "discount",
        "percentage-charge",
        "minimum-charge",
    ] {
        let r = TimeOfUsePricing::builder()
            .band(TimeBand::new(reserved, Amount::parse("0.30000").unwrap()))
            .build();
        assert!(r.is_err(), "band name {reserved:?} must be rejected");
    }
    assert!(
        TimeOfUsePricing::builder()
            .band(TimeBand::new("HT", Amount::parse("0.30000").unwrap()))
            .build()
            .is_ok()
    );
}

#[test]
fn from_positions_rejects_a_position_that_fails_its_own_validate() {
    // LineItem has public fields, so a caller can hand in a broken position. The
    // document then failed its own validate() despite the constructor's promise
    // that it satisfies every invariant.
    let mut broken = LineItem::fixed("ok", Amount::parse("100.00000").unwrap())
        .build()
        .unwrap();
    broken.description = String::new();
    assert!(
        BillingDocument::from_positions(DocumentMeta::default(), vec![broken], vec![], vec![])
            .is_err()
    );

    let mut neg_qty = LineItem::for_usage(
        "x",
        Quantity::new(dec!(5), "kWh"),
        UnitPrice::new(dec!(1), "EUR/kWh"),
    )
    .build()
    .unwrap();
    neg_qty.quantity = Some(billing::Quantity::new(dec!(-5), "kWh"));
    assert!(
        BillingDocument::from_positions(DocumentMeta::default(), vec![neg_qty], vec![], vec![])
            .is_err()
    );
}

#[test]
fn merging_documents_with_advances_or_cash_rounding_is_refused() {
    let mk = |n: &str| {
        BillingDocument::from_positions(
            DocumentMeta {
                invoice_number: n.into(),
                currency: Currency::EUR,
                ..Default::default()
            },
            vec![
                LineItem::fixed("x", Amount::parse("100.00000").unwrap())
                    .build()
                    .unwrap(),
            ],
            vec![Box::new(FixedRateTax::new("VAT", dec!(0.19)).unwrap())],
            vec![],
        )
        .unwrap()
    };

    // Advances would vanish through from_raw while `prepaid` survived — and the
    // result would still pass validate(), because check 10 skips empty advances.
    let with_adv = mk("A")
        .with_advances(vec![
            billing::AdvancePayment::new(vec![billing::TaxBreakdownEntry::new(
                TaxCategory::Standard,
                dec!(0.19),
                Amount::parse("50.00000").unwrap(),
                Amount::parse("9.50000").unwrap(),
            )])
            .unwrap(),
        ])
        .unwrap();
    assert!(merge_period_documents(with_adv, mk("B")).is_err());

    // A summed rounding amount need not be a multiple of the increment, and the
    // rule that would let validate() check it cannot be carried.
    let rounded = mk("C")
        .with_cash_rounding(
            CashRounding::new(
                Amount::parse("0.05000").unwrap(),
                RoundingStrategy::MidpointAwayFromZero,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(merge_period_documents(rounded, mk("D")).is_err());

    // Plain documents still merge.
    assert!(merge_period_documents(mk("E"), mk("F")).is_ok());
}

#[test]
fn a_zero_tax_category_cannot_charge_tax_through_compute() {
    // `with_category` is an infallible builder, so a contradictory state was
    // reachable; `breakdown` checked it but `compute` did not — and `TaxLayer` is
    // public API, so anyone driving layers directly got 19% under a 0%-only code.
    let bad = FixedRateTax::new("RC", dec!(0.19))
        .unwrap()
        .with_category(TaxCategory::ReverseCharge);
    let positions = vec![
        LineItem::fixed("x", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
    ];
    assert!(bad.compute(&positions).is_err());
    assert!(bad.breakdown(&positions).is_err());
}

#[test]
fn proportional_split_still_accepts_shares_the_hamilton_step_can_absorb() {
    // Regression guard on the tolerance fix: bounding the drift at ONE unit
    // rejected inputs that split exactly, because the Hamilton step can distribute
    // up to one unit PER FRACTION.
    let parts = proportional_split(dec!(10000000), &[dec!(0.333333333); 3], 2).unwrap();
    let sum: Decimal = parts.iter().sum();
    assert_eq!(sum, dec!(10000000.00));
    assert_eq!(parts[0], dec!(3333333.34));

    for (total, scale) in [
        (dec!(1000000000), 0u32),
        (dec!(1000000), 3),
        (dec!(10000), 5),
    ] {
        let parts = proportional_split(total, &[dec!(0.333333333); 3], scale).unwrap();
        let sum: Decimal = parts.iter().sum();
        assert_eq!(sum, total, "total={total} scale={scale}");
    }
}

#[test]
fn proportional_split_absorbs_a_deficit_larger_than_one_unit_per_fraction() {
    // Found by mutation testing: the guard rejecting `n_units > parts.len()` was
    // commented "unreachable", and it was not. Two sources feed the deficit — the
    // step-1 floor discards up to one unit per fraction, and the share-sum
    // tolerance admits a drift worth up to another unit per fraction — so it can
    // reach almost 2n units.
    //
    // The previous release therefore refused a split it had already accepted the
    // shares for. `10_000_000` (the case above) worked; `25_000_000` with the same
    // shares and scale returned "deficit of 4 units exceeds the 3 fractions
    // available to absorb it".
    let parts = proportional_split(dec!(25000000), &[dec!(0.333333333); 3], 2).unwrap();
    let sum: Decimal = parts.iter().sum();
    assert_eq!(sum, dec!(25000000.00));
    // Four units over three fractions: one each, then the largest remainder — which
    // ties three ways and so goes to the earliest index — takes the fourth.
    assert_eq!(
        parts,
        vec![dec!(8333333.34), dec!(8333333.33), dec!(8333333.33)]
    );

    // The whole band that used to fail. Every one of these must conserve the total
    // exactly rather than error; `29_999_999` sits just under the point where the
    // tolerance check itself takes over and rejects the shares outright.
    for total in [
        dec!(20000001),
        dec!(25000000),
        dec!(29000000),
        dec!(29999998),
        dec!(29999999),
        dec!(29999999.99),
    ] {
        let parts = proportional_split(total, &[dec!(0.333333333); 3], 2)
            .unwrap_or_else(|e| panic!("total={total} rejected: {e}"));
        let sum: Decimal = parts.iter().sum();
        assert_eq!(sum, total.round_dp(2), "total={total}");
        // No recipient may drift more than the units the deficit actually forced.
        for part in &parts {
            let ideal = total * dec!(0.333333333);
            assert!(
                (part - ideal).abs() <= dec!(0.02),
                "total={total} part={part}"
            );
        }
    }

    // Shares that drift further than the Hamilton step could ever absorb are still
    // rejected — widening the distribution must not widen what is accepted.
    assert!(proportional_split(dec!(30000000), &[dec!(0.333333333); 3], 2).is_err());
    assert!(proportional_split(dec!(100), &[dec!(0.5), dec!(0.4)], 2).is_err());
}

#[test]
fn proportional_split_judges_shares_on_their_own_terms_not_on_the_total() {
    // The share-sum check has two arms: an absolute drift bound and a
    // value-scaled one. Only the absolute arm fires here — shares summing to 0.9
    // are wrong regardless of what they are splitting, and the value-scaled arm
    // goes quiet as the total shrinks, because 10% of nearly nothing is worth
    // less than a cent.
    //
    // Splitting a zero or near-zero total is a real case (a fully discounted
    // document, a period with no consumption), so it is exactly where a
    // malformed share set would otherwise slip through unnoticed.
    assert!(proportional_split(Decimal::ZERO, &[dec!(0.5), dec!(0.4)], 2).is_err());
    assert!(proportional_split(dec!(0.01), &[dec!(0.5), dec!(0.4)], 2).is_err());
    assert!(proportional_split(dec!(0.01), &[dec!(0.6), dec!(0.6)], 2).is_err());

    // A zero total with correct shares still splits, into zeros.
    let parts = proportional_split(Decimal::ZERO, &[dec!(0.5), dec!(0.5)], 2).unwrap();
    assert_eq!(parts, vec![Decimal::ZERO, Decimal::ZERO]);
}

#[test]
fn proportional_split_accepts_a_zero_share() {
    // A recipient allocated nothing is a legitimate input — a tenant who moved in
    // mid-period, a cost centre switched off for the month. Only a *negative*
    // fraction is an error, and treating zero as one would turn a routine case
    // into a hard failure at the call site.
    let parts = proportional_split(dec!(100), &[dec!(0.5), Decimal::ZERO, dec!(0.5)], 2).unwrap();
    assert_eq!(parts, vec![dec!(50.00), Decimal::ZERO, dec!(50.00)]);
    assert_eq!(parts.iter().sum::<Decimal>(), dec!(100.00));

    // All but one at zero degenerates to "give it all to the remaining one".
    let parts = proportional_split(dec!(100), &[Decimal::ZERO, Decimal::ONE], 2).unwrap();
    assert_eq!(parts, vec![Decimal::ZERO, dec!(100.00)]);

    // Negative is still rejected, so the boundary sits between them and not
    // somewhere else.
    assert!(proportional_split(dec!(100), &[dec!(1.5), dec!(-0.5)], 2).is_err());
}

#[test]
fn empty_unit_labels_are_rejected() {
    // An empty unit renders as "EUR/" in a price label and a bare space in the
    // description — visible nonsense on an invoice.
    assert!(
        RateSchedule::graduated()
            .unit("")
            .band(RateBand::over(dec!(0), Amount::parse("1.00000").unwrap()))
            .build()
            .is_err()
    );
    assert!(
        TimeOfUsePricing::builder()
            .unit("  ")
            .band(TimeBand::new("HT", Amount::parse("0.3").unwrap()))
            .build()
            .is_err()
    );
}

#[cfg(feature = "serde")]
#[test]
fn schedule_mode_deserialises_from_the_documented_lowercase_names() {
    // The module docs advertise JSON/YAML config loading with lowercase mode names,
    // but the derived names were TitleCase, so `"graduated"` failed.
    let sched = RateSchedule::graduated()
        .unit("kWh")
        .currency(Currency::EUR)
        .band(RateBand::over(dec!(0), Amount::parse("0.30000").unwrap()))
        .build()
        .unwrap();
    let json = serde_json::to_string(&sched).unwrap();
    assert!(json.contains(r#""mode":"graduated""#), "{json}");
    let back: RateSchedule = serde_json::from_str(&json).unwrap();
    assert_eq!(back.unit(), "kWh");
}

// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
// 0.8.0 — API-shape corrections reported from downstream use
// ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

/// An empty unit label must not survive into a document.
///
/// `RateSchedule` and `TimeOfUsePricing` had always rejected empty units, but
/// `Quantity` / `UnitPrice` did not, so a hand-built position could carry one. That
/// is not merely cosmetic: `PerUnitLevy` selects its base by matching `unit_label`,
/// so an empty unit silently drops the position out of every per-unit levy — the
/// position is billed, the levy on it is not.
#[test]
fn empty_unit_labels_are_rejected_at_every_entry_point() {
    use billing::{Quantity, UnitPrice};

    // Builder path.
    assert!(
        LineItem::debit("Arbeit")
            .quantity(Quantity::new(dec!(100), ""))
            .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
            .build()
            .is_err(),
        "empty quantity unit must be rejected"
    );
    assert!(
        LineItem::debit("Arbeit")
            .quantity(Quantity::new(dec!(100), "kWh"))
            .unit_price(UnitPrice::new(dec!(0.30), "   "))
            .build()
            .is_err(),
        "whitespace-only price unit must be rejected"
    );

    // Struct-literal / post-mutation path, which `validate()` backstops.
    let mut item = LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(100), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    )
    .build()
    .unwrap();
    assert!(item.validate().is_ok());
    item.quantity = Some(Quantity::new(dec!(100), ""));
    assert!(item.validate().is_err(), "validate() must catch it too");
}

/// A document refuses a position with an empty unit rather than emitting it.
#[test]
fn from_positions_rejects_an_empty_unit_label() {
    use billing::{Quantity, UnitPrice};
    let bad = LineItem {
        description: "Arbeit".into(),
        quantity: Some(Quantity::new(dec!(100), "")),
        unit_price: Some(UnitPrice::new(dec!(0.30), "EUR/kWh")),
        net_amount: Amount::parse("30.00000").unwrap(),
        sign: billing::Sign::Debit,
        period: None,
        tags: vec![],
        vat: None,
        allowance_charge: None,
        line_allowances: vec![],
        metadata: Default::default(),
    };
    assert!(
        BillingDocument::from_positions(DocumentMeta::default(), vec![bad], vec![], vec![])
            .is_err()
    );
}

/// `UnitPrice::rounded` replaces the seven-positional-argument `for_usage_rounded`,
/// and must round identically.
#[test]
fn unit_price_rounded_pins_a_derived_price() {
    use billing::{Quantity, UnitPrice};
    // 1/300 EUR/kWh is non-terminating; pin it to 6 decimals.
    let price = UnitPrice::new(dec!(1) / dec!(300), "EUR/kWh")
        .rounded(6, RoundingStrategy::MidpointAwayFromZero);
    assert_eq!(price.value, dec!(0.003333));

    let item = LineItem::for_usage("Arbeit", Quantity::new(dec!(500), "kWh"), price)
        .build()
        .unwrap();
    assert_eq!(item.net_amount, Amount::parse("1.66650").unwrap());

    // Truncation is a different, explicitly chosen answer.
    let trunc =
        UnitPrice::new(dec!(1) / dec!(300), "EUR/kWh").rounded(6, RoundingStrategy::Truncate);
    assert_eq!(trunc.value, dec!(0.003333));
    let up = UnitPrice::new(dec!(2) / dec!(300), "EUR/kWh").rounded(6, RoundingStrategy::Truncate);
    assert_eq!(up.value, dec!(0.006666)); // not 0.006667
}

/// A scale above `Decimal`'s maximum must clamp, not silently no-op.
#[test]
fn unit_price_rounded_clamps_scale_to_decimal_max() {
    use billing::UnitPrice;
    let p =
        UnitPrice::new(dec!(0.1), "EUR/kWh").rounded(99, RoundingStrategy::MidpointAwayFromZero);
    assert_eq!(p.value, dec!(0.1));
}

/// The "nothing to bill, because X" outcome must survive to the caller.
#[test]
fn not_billable_reason_survives_instead_of_becoming_an_empty_document() {
    use billing::{Billing, Positions, PricingModel};
    use std::fmt;

    #[derive(Debug, PartialEq, Eq)]
    enum Why {
        NoData,
        PriceMissing,
    }
    impl fmt::Display for Why {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::NoData => f.write_str("no data"),
                Self::PriceMissing => f.write_str("price missing"),
            }
        }
    }

    struct Settle(Option<Why>);
    impl PricingModel for Settle {
        type Usage = ();
        type Error = BillingError;
        type NotBillable = Why;
        fn positions(&self, _: &()) -> Result<Positions<Why>, BillingError> {
            match self.0 {
                Some(Why::NoData) => Ok(Billing::NotBillable(Why::NoData)),
                Some(Why::PriceMissing) => Ok(Billing::NotBillable(Why::PriceMissing)),
                None => Ok(vec![
                    LineItem::fixed("Vergütung", Amount::parse("42.00000").unwrap()).build()?,
                ]
                .into()),
            }
        }
    }

    // Each reason arrives distinct — not flattened into an empty Vec.
    let out = Settle(Some(Why::PriceMissing))
        .try_settle(DocumentMeta::default())
        .unwrap();
    assert_eq!(out.reason(), Some(&Why::PriceMissing));
    assert!(!out.is_billable());
    assert!(out.into_billable().is_none());

    let out = Settle(Some(Why::NoData))
        .try_settle(DocumentMeta::default())
        .unwrap();
    assert_eq!(out.reason(), Some(&Why::NoData));
    // The Display impl carries the reason for logs.
    assert_eq!(out.to_string(), "not billable: no data");

    // And the billable case still produces a document.
    let out = Settle(None).try_settle(DocumentMeta::default()).unwrap();
    assert_eq!(
        out.billable().unwrap().net_total(),
        Amount::parse("42.00000").unwrap()
    );
    // The billable side of every accessor, so each one is pinned in both
    // directions rather than only where it answers "no". A predicate that always
    // answers "nothing to bill" suppresses invoices in perfect silence.
    assert!(out.is_billable());
    assert_eq!(out.reason(), None);
    assert_eq!(
        out.into_billable().unwrap().net_total(),
        Amount::parse("42.00000").unwrap()
    );
}

/// A tariff that takes no usage still declares its tax and discount layers, and
/// `settle` reaches them without an `&()` at the call site.
#[test]
fn a_usageless_tariff_still_carries_its_layers() {
    use billing::{Positions, PricingModel};
    use std::convert::Infallible;

    struct Payout(i64);
    impl PricingModel for Payout {
        type Usage = ();
        type Error = BillingError;
        type NotBillable = Infallible;
        fn positions(&self, _: &()) -> Result<Positions<Infallible>, BillingError> {
            Ok(vec![LineItem::fixed("Payout", Amount::<5>::from_int(self.0)).build()?].into())
        }
        fn tax_layers(&self) -> Vec<Box<dyn billing::TaxLayer>> {
            vec![FixedRateTax::new("VAT", dec!(0.19)).unwrap().boxed()]
        }
        fn discount_layers(&self) -> Vec<Box<dyn billing::DiscountLayer>> {
            vec![
                billing::tax::FixedDiscount::new("Rabatt", Amount::parse("10.00000").unwrap())
                    .unwrap()
                    .boxed(),
            ]
        }
    }

    // Via the no-usage convenience…
    let doc = Payout(100).settle(DocumentMeta::default()).unwrap();
    // 100 billed, less a 10.00 discount, taxed at 19 % of the reduced base.
    assert_eq!(doc.net_total(), Amount::parse("90.00000").unwrap());
    assert_eq!(doc.discount_total(), Amount::parse("-10.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("17.10000").unwrap());

    // …and via `bill`, which is the same call with the unit usage written out.
    // Both layer kinds must reach the document: dropping the discounts would
    // over-bill by their amount plus the tax on it.
    let doc2 = PricingModel::bill(&Payout(100), DocumentMeta::default(), &()).unwrap();
    assert_eq!(doc2.gross_total(), doc.gross_total());
    assert_eq!(doc2.discount_total(), doc.discount_total());
    assert_eq!(doc2.discount_positions().len(), 1);

    // …and through the document builder.
    let doc3 = BillingDocument::builder()
        .meta(DocumentMeta::default())
        .pricing(&Payout(100), &())
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(doc3.gross_total(), doc.gross_total());
    assert_eq!(doc3.discount_total(), doc.discount_total());
}

/// `FixedRateTax::exempt` enforces the EN 16931 category/reason pairing that the
/// chainable setters can only detect after the fact.
#[test]
fn exempt_constructor_enforces_the_category_reason_pairing() {
    use billing::{TaxCategory, TaxLayer};

    // Every category that requires a reason is constructible with one.
    for c in [
        TaxCategory::Exempt,
        TaxCategory::ReverseCharge,
        TaxCategory::IntraCommunity,
        TaxCategory::Export,
        TaxCategory::OutOfScope,
    ] {
        let layer = FixedRateTax::exempt("Steuerfrei", c, "legal basis").unwrap();
        assert_eq!(layer.category(), c);
        assert_eq!(layer.rate(), dec!(0));
        assert!(layer.compute(&[]).unwrap().net_amount.is_zero());
        // The breakdown reports the category and a zero tax amount.
        let entry = layer.breakdown(&[]).unwrap().unwrap();
        assert_eq!(entry.category, c);
        assert!(entry.tax_amount.is_zero());
    }

    // Z forbids a reason; S/L/M levy tax. Neither belongs in `exempt`.
    assert!(FixedRateTax::exempt("Z", TaxCategory::ZeroRated, "why").is_err());
    for c in [
        TaxCategory::Standard,
        TaxCategory::CanaryIslands,
        TaxCategory::CeutaMelilla,
    ] {
        assert!(FixedRateTax::exempt("taxed", c, "why").is_err());
    }

    // `zero_rated` is the Z path, and carries no reason.
    let z = FixedRateTax::zero_rated("Nullsatz");
    assert_eq!(z.category(), TaxCategory::ZeroRated);
    assert_eq!(z.exemption_reason(), None);
    assert!(z.breakdown(&[]).unwrap().unwrap().tax_amount.is_zero());
}

/// The three German VAT regimes are three layer sets over the same positions — no
/// wrapper type per regime.
#[test]
fn vat_regimes_are_data_not_three_tariff_impls() {
    use billing::{TaxCategory, TaxLayer};

    let positions = || {
        vec![
            LineItem::fixed("Direktvermarktung", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ]
    };
    let regime = |layers: Vec<Box<dyn TaxLayer>>| {
        BillingDocument::from_positions(
            DocumentMeta {
                currency: Currency::EUR,
                ..Default::default()
            },
            positions(),
            layers,
            vec![],
        )
        .unwrap()
    };

    // Regelbesteuerung — 19 %.
    let standard = regime(vec![
        FixedRateTax::new("USt 19%", dec!(0.19)).unwrap().boxed(),
    ]);
    assert_eq!(standard.tax_total(), Amount::parse("190.00000").unwrap());

    // §12 Abs. 3 UStG — zero-rated, input tax still deductible, no reason text.
    let zero = regime(vec![FixedRateTax::zero_rated("§12 Abs. 3 UStG").boxed()]);
    assert_eq!(zero.tax_total(), Amount::<5>::ZERO);
    assert_eq!(zero.tax_breakdown().len(), 1);
    assert_eq!(zero.tax_breakdown()[0].category, TaxCategory::ZeroRated);

    // §19 UStG Kleinunternehmer — exempt, reason required.
    let small = regime(vec![
        FixedRateTax::exempt(
            "§19 UStG",
            TaxCategory::Exempt,
            "Kleinunternehmer gemäß §19 UStG",
        )
        .unwrap()
        .boxed(),
    ]);
    assert_eq!(small.tax_total(), Amount::<5>::ZERO);
    assert_eq!(small.tax_breakdown()[0].category, TaxCategory::Exempt);
    assert!(small.tax_breakdown()[0].exemption_reason.is_some());

    // All three agree on the net; only the tax treatment differs.
    for d in [&standard, &zero, &small] {
        assert_eq!(d.net_total(), Amount::parse("1000.00000").unwrap());
        d.assert_valid();
    }
}

/// `AmountScale` must re-run its constructor check on deserialisation.
///
/// The crate's stated invariant is that validated types route deserialisation
/// through the same validation via `#[serde(try_from = ...)]`; a derived impl on
/// private fields would let stored configuration reintroduce a precision that
/// `AmountScale::new` refuses.
#[cfg(feature = "serde")]
#[test]
fn amount_scale_revalidates_on_deserialisation() {
    // Round-trip of a valid value.
    let scale = AmountScale::new(2, RoundingStrategy::Floor).unwrap();
    let json = serde_json::to_string(&scale).unwrap();
    let back: AmountScale = serde_json::from_str(&json).unwrap();
    assert_eq!(back, scale);

    // A precision the constructor refuses must not slip in through JSON.
    let err = serde_json::from_str::<AmountScale>(r#"{"decimals":99,"strategy":"Floor"}"#)
        .expect_err("decimals > 5 must be rejected on deserialisation");
    assert!(
        err.to_string().contains("must be <= 5"),
        "error should name the violated bound, got: {err}"
    );
}

/// `Amount::round_to_scale` agrees with `round_to_increment` for every strategy,
/// and is a no-op at or above the type's own precision.
#[test]
fn round_to_scale_matches_increment_rounding_for_every_strategy() {
    let strategies = [
        RoundingStrategy::MidpointAwayFromZero,
        RoundingStrategy::MidpointToEven,
        RoundingStrategy::Ceiling,
        RoundingStrategy::Floor,
        RoundingStrategy::Truncate,
    ];
    for strategy in strategies {
        for raw in -2_000i64..2_000 {
            let a = Amount::<5>::from_raw_units(raw);
            // scale 2 == increment 0.001_00 (10^(5-2) = 1000 raw units)
            assert_eq!(
                a.round_to_scale(2, strategy).unwrap(),
                a.round_to_increment(Amount::<5>::from_raw_units(1_000), strategy)
                    .unwrap(),
                "scale 2 vs increment 1000 for raw {raw}, {strategy:?}"
            );
            assert!(a.round_to_scale(2, strategy).unwrap().fits_scale(2));
            // At or above P, nothing changes.
            assert_eq!(a.round_to_scale(5, strategy).unwrap(), a);
            assert_eq!(a.round_to_scale(9, strategy).unwrap(), a);
            assert!(a.fits_scale(5));
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn a_vat_breakdown_must_be_a_component_of_the_tax_actually_charged() {
    // Validation check 8. Exact equality would be wrong — a commission or a
    // per-unit excise adds to `tax_total` without contributing a breakdown entry
    // — so the rule is a containment one: the breakdown may not exceed the tax
    // charged, and may not oppose its sign.
    //
    // Both halves matter and they fail differently. A breakdown *larger* than the
    // tax charged is output VAT declared on an invoice that never charged it. A
    // breakdown of the *opposite* sign is an invoice claiming input VAT. Checking
    // only one of the two lets the other through, and both deserialise cleanly
    // from a hand-built payload if nothing looks.
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "COMP-1".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();
    let json = serde_json::to_string(&doc).unwrap();
    assert!(serde_json::from_str::<BillingDocument>(&json).is_ok());

    // Only the breakdown entry is moved; `tax_total` stays at 19.00.
    let tamper = |tax_amount: &str| {
        let bad = json.replace(
            r#""taxable_base":"100.00000","tax_amount":"19.00000""#,
            &format!(r#""taxable_base":"100.00000","tax_amount":"{tax_amount}""#),
        );
        assert_ne!(
            bad, json,
            "replacement did not match — payload shape changed"
        );
        serde_json::from_str::<BillingDocument>(&bad)
    };

    // Exceeds the tax charged.
    assert!(tamper("19.00001").is_err());
    assert!(tamper("38.00000").is_err());
    // Opposes its sign: an invoice cannot declare negative output VAT.
    assert!(tamper("-19.00000").is_err());
    assert!(tamper("-0.00001").is_err());

    // The other side of the rule: a breakdown strictly *below* `tax_total` is
    // lawful, and is the whole reason this is a containment check rather than an
    // equality one. A platform commission is taxed but is not itself VAT, so it
    // raises the tax total without appearing in BG-23.
    let with_charge = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "COMP-1B".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![
            Box::new(PercentageCharge::new("Platform fee", dec!(0.05)).unwrap()),
            Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap()),
        ],
        vec![],
    )
    .unwrap();
    let breakdown_tax: Amount<5> = with_charge
        .tax_breakdown()
        .iter()
        .map(|e| e.tax_amount)
        .sum();
    assert!(breakdown_tax < with_charge.tax_total());
    assert!(breakdown_tax > Amount::ZERO);
    with_charge.assert_valid();
    let json = serde_json::to_string(&with_charge).unwrap();
    assert!(serde_json::from_str::<BillingDocument>(&json).is_ok());
}

#[cfg(feature = "serde")]
#[test]
fn the_breakdown_containment_rule_mirrors_for_a_credit_note() {
    // A credit note's tax_total is negative, so "does not exceed" and "does not
    // oppose the sign" both flip. Handling only the invoice orientation leaves
    // credit notes unchecked — and a credit note is the document where a wrong
    // sign is worth twice the money, because it reverses one.
    let invoice = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "COMP-2".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();
    let credit = invoice
        .reverse(DocumentMeta {
            invoice_number: "COMP-2-STORNO".into(),
            kind: DocumentKind::CreditNote,
            currency: Currency::EUR,
            ..Default::default()
        })
        .unwrap();
    assert!(credit.tax_total().is_negative());

    let json = serde_json::to_string(&credit).unwrap();
    assert!(serde_json::from_str::<BillingDocument>(&json).is_ok());

    let tamper = |tax_amount: &str| {
        let bad = json.replace(
            r#""tax_amount":"-19.00000""#,
            &format!(r#""tax_amount":"{tax_amount}""#),
        );
        assert_ne!(
            bad, json,
            "replacement did not match — payload shape changed"
        );
        serde_json::from_str::<BillingDocument>(&bad)
    };

    // More negative than the tax reversed.
    assert!(tamper("-19.00001").is_err());
    assert!(tamper("-38.00000").is_err());
    // Positive breakdown on a negative tax total — the sign opposition.
    assert!(tamper("19.00000").is_err());
    assert!(tamper("0.00001").is_err());

    // And the untampered credit note, whose breakdown sits exactly at the bound,
    // still validates — the rule is inclusive at both ends.
    credit.assert_valid();
    assert_eq!(
        credit
            .tax_breakdown()
            .iter()
            .map(|e| e.tax_amount)
            .sum::<Amount<5>>(),
        credit.tax_total()
    );
}

#[cfg(feature = "serde")]
#[test]
fn verify_vat_attribution_actually_rejects_a_mis_attributed_position() {
    // BR-CO-04 / BR-S-08: every position carries a VAT category and rate, and the
    // positions carrying each pair must sum to that group's taxable base. The
    // whole point of `verify_vat_attribution` is to say no — but every other test
    // in this suite calls it on a document it expects to be sound and unwraps.
    // A version of it that returned `Ok(())` unconditionally would have passed
    // all of them.
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "ATTR-1".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();
    doc.verify_vat_attribution().unwrap();

    let json = serde_json::to_string(&doc).unwrap();

    // Re-label the line as 7 % — a group the breakdown does not contain.
    let orphaned = json.replacen(
        r#""vat":{"category":"Standard","rate":"0.19"}"#,
        r#""vat":{"category":"Standard","rate":"0.07"}"#,
        1,
    );
    assert_ne!(
        orphaned, json,
        "replacement did not match — payload shape changed"
    );
    let doc = serde_json::from_str::<BillingDocument>(&orphaned).unwrap();
    assert!(
        doc.verify_vat_attribution().is_err(),
        "a position naming a group outside BG-23 must be rejected"
    );

    // Attribution is read from the *taxable* positions. Counting the VAT position
    // itself instead would double the base of its own group and make a sound
    // document look broken — and make this broken one look sound.
    let base: Amount<5> = doc
        .all_positions()
        .filter(|p| !p.has_tag(billing::tags::VAT))
        .filter_map(|p| p.vat.map(|_| p.net_amount))
        .sum();
    assert_eq!(base, Amount::parse("100.00000").unwrap());
}

#[cfg(feature = "serde")]
#[test]
fn no_public_path_produces_a_document_that_fails_its_own_validation() {
    // The counterpart to the in-crate `assert_valid_panics_on_an_inconsistent_document`
    // test: from outside the crate an invalid `BillingDocument` cannot be reached
    // at all. Every constructor computes the totals it stores, `with_extra_position`
    // refuses where it would strand the VAT breakdown, and a tampered payload is
    // rejected on the way in rather than validated after the fact.
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "PANIC-1".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();
    doc.assert_valid();

    // A tampered gross does not survive deserialisation.
    let json = serde_json::to_string(&doc).unwrap();
    let broken = json.replace(
        r#""gross_total":"119.00000""#,
        r#""gross_total":"219.00000""#,
    );
    assert_ne!(
        broken, json,
        "replacement did not match — payload shape changed"
    );
    assert!(serde_json::from_str::<BillingDocument>(&broken).is_err());

    // Appending to a document carrying a VAT breakdown is refused outright, rather
    // than silently producing one whose breakdown no longer covers its base.
    assert!(
        doc.with_extra_position(
            LineItem::fixed("Nachtrag", Amount::parse("50.00000").unwrap())
                .build()
                .unwrap(),
        )
        .is_err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 16. The 0.13.0 audit — three silent defects the totals could not see
//
// Every one of these produced a document that balanced, validated and rendered.
// What each got wrong was a field no arithmetic check looks at, which is why
// none of them surfaced as a failing total.
// ─────────────────────────────────────────────────────────────────────────────

/// `.currency()` used to write straight into `meta`, so a later `.meta()` — which
/// replaces the whole header — discarded it. The document then carried
/// `Currency::XXX`: ISO 4217 for "no currency involved", accepted by BR-CL-04, and
/// printed on every line of the rendered invoice.
///
/// Nothing failed. The totals were right, `validate()` passed, and the only symptom
/// was a wrong currency.
#[test]
fn the_builder_currency_is_order_independent() {
    let positions = || {
        vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ]
    };
    let header = || DocumentMeta {
        invoice_number: "R-1".into(),
        ..Default::default()
    };

    let after = BillingDocument::builder()
        .meta(header())
        .currency(Currency::EUR)
        .positions(positions())
        .build()
        .unwrap();
    let before = BillingDocument::builder()
        .currency(Currency::EUR)
        .meta(header())
        .positions(positions())
        .build()
        .unwrap();

    assert_eq!(after.currency(), Currency::EUR);
    assert_eq!(
        before.currency(),
        Currency::EUR,
        "`.currency()` before `.meta()` used to be silently discarded"
    );
    assert!(!before.currency().is_unset());
}

/// `minimum_charge` used to return `LineItem::fixed`, which states BT-131 and
/// nothing else. A shortfall is an ordinary invoice line (BG-25), so BR-22, BR-23
/// and BR-26 all apply to it — three *fatal* rules, on a position the engine itself
/// produced, none of which changes an amount.
#[test]
fn a_minimum_charge_shortfall_is_a_complete_invoice_line() {
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Verbrauch", Amount::parse("40.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap();

    let shortfall = minimum_charge(
        &doc,
        Amount::parse("100.00000").unwrap(),
        "Mindestentgelt",
        Currency::EUR,
    )
    .unwrap()
    .expect("40 is below the 100 minimum");

    assert_eq!(shortfall.net_amount, Amount::parse("60.00000").unwrap());
    // BR-22 / BR-23 / BR-26 — the three terms `LineItem::fixed` leaves unstated.
    let qty = shortfall.quantity.as_ref().expect("BT-129");
    assert_eq!(qty.value, dec!(1));
    assert_eq!(qty.code.as_deref(), Some(UNIT_CODE_ONE)); // BT-130
    assert_eq!(
        shortfall.unit_price.as_ref().expect("BT-146").value,
        dec!(60)
    );
    // The label follows the currency it was given, not a hardcoded "EUR".
    assert_eq!(shortfall.unit_price.as_ref().unwrap().unit, "EUR");

    // …and 1 × 60.00 is exactly 60.00, so PEPPOL-EN16931-R120 holds with nothing
    // rounded and the document still assembles at the interchange scale.
    let full = BillingDocument::builder()
        .currency(Currency::EUR)
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("Verbrauch", Amount::parse("40.00000").unwrap())
                .build()
                .unwrap(),
            shortfall,
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    assert_eq!(full.net_total(), Amount::parse("100.00000").unwrap());
    assert!(full.fits_amount_scale(2));
    full.assert_valid();
    full.verify_vat_attribution().unwrap();
}

/// A `PricingModel` knows which format it prices for, but `bill()` and `settle()`
/// had nowhere to say so — they always assembled at the engine's five decimals. The
/// shortest path from a model to a document therefore produced one that fails
/// BR-DEC-23, with no error anywhere.
#[test]
fn a_pricing_model_can_declare_the_interchange_scale_it_prices_for() {
    struct Metered;

    impl PricingModel for Metered {
        type Usage = ();
        type Error = std::convert::Infallible;
        type NotBillable = std::convert::Infallible;

        fn positions(&self, _: &()) -> Result<Positions<Self::NotBillable>, Self::Error> {
            Ok(vec![
                LineItem::for_usage(
                    "Arbeit",
                    Quantity::new(dec!(1234.567), "kWh"),
                    UnitPrice::new(dec!(0.28901), "EUR/kWh"),
                )
                .build()
                .unwrap(),
            ]
            .into())
        }

        fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
            vec![FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed()]
        }

        fn amount_scale(&self) -> Option<AmountScale> {
            Some(AmountScale::EN16931)
        }
    }

    let meta = DocumentMeta {
        currency: Currency::EUR,
        ..Default::default()
    };
    let doc = Metered.settle(meta.clone()).unwrap();

    // 1234.567 × 0.28901 = 356.80221 — reduced once, at the leaf.
    assert_eq!(doc.net_total(), Amount::parse("356.80000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("67.79000").unwrap());
    assert!(doc.fits_amount_scale(2), "settle() must honour the model");
    doc.assert_valid();
    doc.verify_vat_attribution().unwrap();

    // An explicit scale on the builder still wins over the model's.
    let finer = BillingDocument::builder()
        .meta(meta)
        .amount_scale(AmountScale::new(3, RoundingStrategy::MidpointAwayFromZero).unwrap())
        .pricing(&Metered, &())
        .unwrap()
        .build()
        .unwrap();
    assert_eq!(finer.net_total(), Amount::parse("356.80200").unwrap());
}

/// `Currency::new` accepts any three ASCII letters — deliberately, since this crate
/// bills things that are not EN 16931 invoices. What it must not do is let such a
/// code reach an invoice unremarked: BR-CL-04 is fatal on BT-5, and the rejection
/// happens at the access point rather than at a boundary the caller controls.
#[test]
fn a_well_formed_currency_is_not_necessarily_one_en16931_admits() {
    let doc = BillingDocument::builder()
        .currency(Currency::new("XBT").unwrap())
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap();

    // The document is arithmetically perfect …
    doc.assert_valid();
    // … and its BT-5 is fatal under BR-CL-04, which only this predicate can say.
    assert!(!doc.currency().is_iso4217());
    assert!(
        !doc.currency().is_unset(),
        "it is set — just not to a real code"
    );

    // The two boundary questions are independent: `XXX` is *in* the ISO list.
    assert!(Currency::XXX.is_iso4217());
    assert!(Currency::XXX.is_unset());
    assert!(Currency::EUR.is_iso4217() && !Currency::EUR.is_unset());
}

// ─────────────────────────────────────────────────────────────────────────────
// 17. The 0.13.0 audit, second pass — a rule the engine declined to check, a
//     term with no name, and the last unguarded panic on the untrusted path
// ─────────────────────────────────────────────────────────────────────────────

/// BR-B-02 forbids "split payment" (`B`) and "standard rated" (`S`) in one
/// document. The crate described both `B` rules as "jurisdictional … and stay with
/// the caller", but only BR-B-01 ("shall be a domestic Italian invoice") actually
/// needs anything the engine cannot see. BR-B-02 is a property of the categories
/// present — decidable here, and structurally identical to BR-O-11, which
/// `validate()` has always checked.
///
/// It matters because `merge_period_documents` can manufacture the forbidden
/// combination out of two individually lawful documents, which is the same argument
/// the crate already makes for checking BR-O-11.
#[test]
fn split_payment_and_standard_rated_cannot_share_a_document() {
    let italian = |tag: &str, category: TaxCategory, rate: Decimal| {
        let layer = if category == TaxCategory::SplitPayment {
            FixedRateTax::new("Scissione dei pagamenti", rate)
                .unwrap()
                .with_category(TaxCategory::SplitPayment)
        } else {
            FixedRateTax::new("IVA", rate).unwrap()
        };
        BillingDocument::builder()
            .currency(Currency::EUR)
            .positions(vec![
                LineItem::fixed("Fornitura", Amount::parse("1000.00000").unwrap())
                    .tag(tag)
                    .build()
                    .unwrap(),
            ])
            .extra_tax(layer.with_tag(tag).boxed())
            .build()
            .unwrap()
    };

    // Each half is lawful on its own.
    let split = italian("b", TaxCategory::SplitPayment, dec!(0.22));
    let standard = italian("s", TaxCategory::Standard, dec!(0.22));
    split.assert_valid();
    standard.assert_valid();

    // Merging them produces the combination BR-B-02 forbids, and the merge refuses.
    let err = merge_period_documents(split, standard).unwrap_err();
    assert!(
        err.to_string().contains("BR-B-02"),
        "merge must name the rule it refused on, got: {err}"
    );

    // The same document built directly is refused too — by `validate`, not only by
    // the merge path.
    let mixed = BillingDocument::builder()
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Fornitura", Amount::parse("1000.00000").unwrap())
                .tag("b")
                .build()
                .unwrap(),
            LineItem::fixed("Servizio", Amount::parse("500.00000").unwrap())
                .tag("s")
                .build()
                .unwrap(),
        ])
        .extra_tax(
            FixedRateTax::new("Scissione", dec!(0.22))
                .unwrap()
                .with_category(TaxCategory::SplitPayment)
                .with_tag("b")
                .boxed(),
        )
        .extra_tax(
            FixedRateTax::new("IVA", dec!(0.22))
                .unwrap()
                .with_tag("s")
                .boxed(),
        )
        .build();
    let err = mixed.unwrap_err();
    assert!(err.to_string().contains("BR-B-02"), "got: {err}");

    // A `B`-only document is fine, and so is an `S`-only one.
    italian("b", TaxCategory::SplitPayment, dec!(0.22)).assert_valid();
    italian("s", TaxCategory::Standard, dec!(0.22)).assert_valid();

    // The hand-attributed shape — categories set with `.vat(..)` and no VAT layer
    // at all, so there is no breakdown for check 14 to look at — is caught too, by
    // `verify_vat_attribution`: an attributed group with no breakdown entry behind
    // it. This is why BR-B-02 needs no position-level clause of its own; the
    // attribution can never be a superset of the breakdown.
    let hand_attributed = BillingDocument::builder()
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Fornitura", Amount::parse("1000.00000").unwrap())
                .vat(LineVat::new(TaxCategory::SplitPayment, dec!(0.22)).unwrap())
                .build()
                .unwrap(),
            LineItem::fixed("Servizio", Amount::parse("500.00000").unwrap())
                .vat(LineVat::new(TaxCategory::Standard, dec!(0.22)).unwrap())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap();
    // It balances and validates — the breakdown is empty, so check 14 sees nothing.
    hand_attributed.assert_valid();
    // …and the attribution check refuses it before it can be emitted.
    let err = hand_attributed.verify_vat_attribution().unwrap_err();
    assert!(
        err.to_string()
            .contains("VAT breakdown entry for that group"),
        "got: {err}"
    );
}

/// BT-107 was the only member of the EN 16931 totals chain with no accessor under
/// its own name: BT-106, BT-108, BT-109, BT-110, BT-112, BT-113, BT-114 and BT-115
/// all had one, while the sum of document level allowances was reachable only as
/// `discount_total()` — the same figure with the *opposite sign*.
///
/// That is the sign a consumer must flip to emit `cbc:AllowanceTotalAmount`, which
/// is unsigned in both UBL and CII. Forgetting to flip it fails BR-CO-13 by twice
/// the allowance, and nothing in the document itself can show it.
#[test]
fn bt107_is_available_as_the_positive_magnitude_the_standard_states() {
    let doc = BillingDocument::builder()
        .currency(Currency::EUR)
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("Leistung", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            PercentageDiscount::new("Treuerabatt", dec!(0.02))
                .unwrap()
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    let bt106 = doc.line_total().unwrap();
    let bt107 = doc.allowance_total().unwrap();
    let bt108 = doc.charge_total().unwrap();
    let bt109 = doc.taxable_total().unwrap();

    assert_eq!(bt106, Amount::parse("1000.00000").unwrap());
    assert_eq!(bt107, Amount::parse("20.00000").unwrap());
    assert!(bt107.is_positive(), "BT-107 is stated as a magnitude");
    assert_eq!(bt107, doc.discount_total().checked_neg().unwrap());

    // BR-CO-13, spelled the way the standard spells it: BT-109 = BT-106 − BT-107 + BT-108.
    assert_eq!(
        bt109,
        bt106
            .checked_sub(bt107)
            .unwrap()
            .checked_add(bt108)
            .unwrap()
    );

    // …and BR-CO-15 closes the chain.
    assert_eq!(
        doc.gross_total(),
        bt109.checked_add(doc.vat_total().unwrap()).unwrap()
    );
    doc.verify_vat_attribution().unwrap();

    // With no allowances at all it is zero, not absent.
    let plain = BillingDocument::builder()
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Leistung", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap();
    assert_eq!(plain.allowance_total().unwrap(), Amount::ZERO);
}

/// `UsageAggregator::aggregate` returned a bare `Decimal`, and `Decimal`'s `+`,
/// `*` and `Sum` panic on overflow. The trait consumes raw usage events — the least
/// trusted input in the pipeline — so one corrupt magnitude aborted the process,
/// which is precisely the failure mode section 1 of this file exists to prevent.
#[test]
fn an_aggregator_reports_a_corrupt_event_instead_of_aborting_the_run() {
    use billing::aggregation::{
        CountAggregator, LatestAggregator, MaxAggregator, SumAggregator, UniqueCountAggregator,
        UsageAggregator, WeightedSumAggregator,
    };

    struct Reading(Decimal);

    let corrupt = [Reading(Decimal::MAX), Reading(Decimal::MAX)];

    // The two accumulating aggregators report; they do not panic.
    assert!(
        SumAggregator::new(|r: &Reading| r.0)
            .aggregate(&corrupt)
            .is_err()
    );
    assert!(
        WeightedSumAggregator::new(|r: &Reading| r.0, |_: &Reading| Decimal::MAX)
            .aggregate(&corrupt)
            .is_err()
    );

    // The four selecting ones cannot fail, on the same input.
    assert!(
        MaxAggregator::new(|r: &Reading| r.0)
            .aggregate(&corrupt)
            .is_ok()
    );
    assert!(
        LatestAggregator::new(|r: &Reading| r.0)
            .aggregate(&corrupt)
            .is_ok()
    );
    assert!(CountAggregator.aggregate(&corrupt).is_ok());
    assert!(
        UniqueCountAggregator::new(|r: &Reading| r.0.to_string())
            .aggregate(&corrupt)
            .is_ok()
    );

    // Ordinary input still answers, so the guard is not a blanket refusal.
    assert_eq!(
        SumAggregator::new(|r: &Reading| r.0)
            .aggregate(&[Reading(dec!(1.25)), Reading(dec!(2.75))])
            .unwrap(),
        dec!(4)
    );
}

/// `BillingDocument`'s type documentation promises that a document from
/// `from_positions` "satisfies every invariant by construction". Computing the
/// totals makes checks 1–4 true and nothing else: BR-CO-18, BR-O-11 and BR-B-02
/// are properties of the *breakdown*, reachable with an ordinary combination of
/// layers, and `assemble` never ran `validate()`.
///
/// So the primary constructor handed back documents that failed their own
/// `assert_valid()`. `merge_period_documents` and `AllocationRule` have always
/// re-checked their outputs; the constructor now does too.
#[test]
fn the_primary_constructor_returns_only_documents_that_pass_their_own_validate() {
    // BR-O-11: "not subject to VAT" cannot coexist with another breakdown group.
    // Two tagged layers is all it takes, and both are individually lawful.
    let out_of_scope = FixedRateTax::exempt(
        "Nicht steuerbar",
        TaxCategory::OutOfScope,
        "Nicht steuerbarer Auslagenersatz",
    )
    .unwrap()
    .with_tag("outside");

    let result = BillingDocument::builder()
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Auslagen", Amount::parse("100.00000").unwrap())
                .tag("outside")
                .build()
                .unwrap(),
            LineItem::fixed("Beratung", Amount::parse("900.00000").unwrap())
                .tag("taxed")
                .build()
                .unwrap(),
        ])
        .extra_tax(out_of_scope.boxed())
        .extra_tax(
            FixedRateTax::new("MwSt", dec!(0.19))
                .unwrap()
                .with_tag("taxed")
                .boxed(),
        )
        .build();

    let err = result.unwrap_err();
    assert!(
        err.to_string().contains("BR-O-11"),
        "the constructor must refuse, naming the rule; got: {err}"
    );

    // Each half on its own still builds and validates, so this is not a blanket
    // refusal of `O` or of tagged layers.
    BillingDocument::builder()
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Auslagen", Amount::parse("100.00000").unwrap())
                .tag("outside")
                .build()
                .unwrap(),
        ])
        .extra_tax(
            FixedRateTax::exempt("Nicht steuerbar", TaxCategory::OutOfScope, "Auslagenersatz")
                .unwrap()
                .boxed(),
        )
        .build()
        .unwrap()
        .assert_valid();
}

// ─────────────────────────────────────────────────────────────────────────────
// 18. The `en16931` adapter review — verified against KoSIT and CEN artefacts
//
// Every claim below was re-derived from the primary artefacts before it was
// acted on:
//   itplr-kosit/xrechnung-schematron @ master  (BR-DE-18, and its test fixtures)
//   ConnectingEurope/eInvoicing-EN16931 @ validation-1.3.16  (BR-CL-08, BR-CL-10)
//   OpenPEPPOL/peppol-bis-invoice-3 @ master  (PEPPOL-EN16931-R002)
// ─────────────────────────────────────────────────────────────────────────────

/// `PaymentTerms: Display` omitted the newline `BR-DE-18` requires after the last
/// `#SKONTO#…#` segment, so every XRechnung carrying an early-payment discount was
/// rejected — **fatally**.
///
/// The rule is two assertions folded into one `satisfies`, and the second is the
/// one that bites:
///
/// ```xpath
/// matches(cac:PaymentTerms/cbc:Note[1]/tokenize(., '#.+#')[last()], '^\s*\n')
/// ```
///
/// Without a terminator `tokenize(., '#.+#')[last()]` is the empty string. Verified
/// against a real XPath 2.0 engine and against KoSIT's own fixture, which is marked
/// `schematron-invalid="xrcii:BR-DE-18"` and titled "Every skonto entry should end
/// with a newline".
#[test]
fn bt20_terminates_every_skonto_entry_with_a_newline() {
    use billing::{EarlyPaymentDiscount, PaymentTerms};

    let one = PaymentTerms::text("Zahlbar innerhalb 30 Tagen ohne Abzug.")
        .with_discount(EarlyPaymentDiscount::new(10, dec!(2.00)).unwrap());
    assert_eq!(
        one.to_string(),
        "Zahlbar innerhalb 30 Tagen ohne Abzug.\n#SKONTO#TAGE=10#PROZENT=2.00#\n"
    );

    // Staged terms: one terminator, after the last segment.
    let staged = PaymentTerms::default()
        .with_discount(EarlyPaymentDiscount::new(10, dec!(3)).unwrap())
        .with_discount(EarlyPaymentDiscount::new(20, dec!(2)).unwrap());
    assert_eq!(
        staged.to_string(),
        "#SKONTO#TAGE=10#PROZENT=3.00#\n#SKONTO#TAGE=20#PROZENT=2.00#\n"
    );

    // With BASISBETRAG too.
    let based = PaymentTerms::default().with_discount(
        EarlyPaymentDiscount::new(10, dec!(2.50))
            .unwrap()
            .with_base_amount(Amount::parse("485.98000").unwrap())
            .unwrap(),
    );
    assert_eq!(
        based.to_string(),
        "#SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#\n"
    );

    // A prose-only BT-20 gets NO terminator: BR-DE-18's `every … satisfies` is
    // vacuously true when no line starts with `#`, so one would be pure noise.
    let prose = PaymentTerms::text("Zahlbar innerhalb 30 Tagen ohne Abzug.");
    assert_eq!(prose.to_string(), "Zahlbar innerhalb 30 Tagen ohne Abzug.");
    assert_eq!(PaymentTerms::default().to_string(), "");

    // The terminator must not break the round trip.
    for terms in [one.clone(), staged.clone(), based.clone(), prose.clone()] {
        assert_eq!(
            terms.to_string().parse::<PaymentTerms>().unwrap(),
            terms,
            "Display -> FromStr must be an identity for {terms:?}"
        );
    }

    // Every rendered field either ends in a newline or carries no segment at all —
    // which is the shape of BR-DE-18's second assertion, stated as an invariant.
    for terms in [&one, &staged, &based, &prose] {
        let rendered = terms.to_string();
        assert_eq!(
            rendered.ends_with('\n'),
            !terms.discounts().is_empty(),
            "wrong terminator for {rendered:?}"
        );
    }
}

/// BG-1 is `0..n` and each note has a UNCL 4451 subject code (BT-21, `BR-CL-08`).
/// `DocumentMeta::notes` was one uncoded `Option<String>` and could express neither.
#[test]
fn document_notes_model_bg_1_with_its_subject_code() {
    use billing::Note;

    let meta = DocumentMeta {
        notes: vec![
            "Lieferung ab Werk.".into(),
            Note::coded("AAI", "Es gelten unsere AGB."),
            Note::coded("REG", "Amtsgericht Musterstadt HRB 12345"),
        ],
        ..Default::default()
    };

    assert_eq!(meta.notes.len(), 3);
    assert_eq!(meta.notes[0].subject_code, None); // BT-21 absent is lawful
    assert_eq!(meta.notes[0].text, "Lieferung ab Werk.");
    assert_eq!(meta.notes[1].subject_code.as_deref(), Some("AAI"));
    // `Display` renders BT-22 alone — BT-21 is metadata about the note.
    assert_eq!(
        meta.notes[2].to_string(),
        "Amtsgericht Musterstadt HRB 12345"
    );

    // The document carries them through assembly untouched.
    let doc = BillingDocument::builder()
        .meta(meta)
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap();
    assert_eq!(doc.meta.notes.len(), 3);
}

/// BT-29 / BT-46 are `Identifier`s whose scheme `BR-CL-10` restricts to the ISO 6523
/// ICD list — *fatal*. A bare `String` could not say which registry an identifier
/// came from, so the adapter could not map these fields at all.
#[test]
fn party_identifiers_carry_the_registry_they_came_from() {
    use billing::PartyIdentifier;

    let gln = PartyIdentifier::scheme("0088", "4012345000009");
    assert_eq!(gln.scheme.as_deref(), Some("0088"));
    assert_eq!(gln.value, "4012345000009");
    assert_eq!(gln.to_string(), "4012345000009"); // the scheme is not part of the text

    // No scheme is lawful — BR-CL-10 only constrains one that is present. The
    // *value* has to be asserted as well as the absent scheme: `Default::default()`
    // also has no scheme, so checking only that cannot tell a working `From` impl
    // from one that discards its input.
    let internal: PartyIdentifier = "KND-4711".into();
    assert_eq!(internal.value, "KND-4711");
    assert_eq!(internal.scheme, None);
    assert_ne!(internal, PartyIdentifier::default());

    let doc = BillingDocument::builder()
        .meta(DocumentMeta {
            issuer_id: Some(gln),
            recipient_id: Some(internal),
            ..Default::default()
        })
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap();
    assert_eq!(
        doc.meta.issuer_id.as_ref().unwrap().scheme.as_deref(),
        Some("0088")
    );
    assert_eq!(doc.meta.recipient_id.as_ref().unwrap().scheme, None);
}

/// `is_self_billed()` joins `is_credit_note()` and `is_peppol_billing_code()`:
/// the semantics as a predicate, with the code as its rendering.
#[test]
fn self_billing_is_a_predicate_not_a_comparison_against_389() {
    assert!(DocumentKind::SelfBilledInvoice.is_self_billed());
    assert_eq!(DocumentKind::SelfBilledInvoice.code(), 389);
    for k in DocumentKind::ALL {
        assert_eq!(k.is_self_billed(), k == DocumentKind::SelfBilledInvoice);
    }
    // The one kind outside the Peppol *Billing* profile is exactly the self-billed
    // one — self-billing is a different customization, not a forbidden invoice.
    let outside: Vec<_> = DocumentKind::ALL
        .into_iter()
        .filter(|k| !k.is_peppol_billing_code())
        .collect();
    assert_eq!(outside, vec![DocumentKind::SelfBilledInvoice]);
    assert!(outside.iter().all(DocumentKind::is_self_billed));
}

// ─────────────────────────────────────────────────────────────────────────────
// 19. Period rules the engine could decide and did not
//
// `Period::is_ordered()` shipped in 0.12.0 and was called by nothing but its own
// tests. BR-29, BR-30, PEPPOL-EN16931-R110 and R111 are all fatal and all
// decidable from dates the engine already holds — the same argument that puts
// BR-B-02 and BR-O-11 inside `validate()` rather than in a consumer's lap.
// ─────────────────────────────────────────────────────────────────────────────

/// BR-30: a line period whose end precedes its start is refused by the position
/// itself, so it cannot enter a document at all.
#[test]
fn a_line_period_must_not_end_before_it_starts() {
    let backwards = LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
        .period("2026-06-30", "2026-06-01")
        .build();
    let err = backwards.unwrap_err();
    assert!(err.to_string().contains("BR-29 / BR-30"), "got: {err}");

    // A single-day period is ordered — both rules say "later *or equal*".
    LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
        .period("2026-06-01", "2026-06-01")
        .build()
        .unwrap();

    // Not ISO 8601: the engine stays date-type-agnostic and says nothing rather
    // than ordering a format it cannot order.
    LineItem::fixed("Grundpreis", Amount::parse("30.00000").unwrap())
        .period("30.6.2026", "1.6.2026")
        .build()
        .unwrap();
}

/// BR-29 on the document, and `PEPPOL-EN16931-R110` / `R111` across the two levels:
/// a line period must lie inside the invoicing period.
#[test]
fn a_line_period_must_lie_inside_the_invoicing_period() {
    let build = |line_from: &str, line_to: &str| {
        BillingDocument::builder()
            .meta(DocumentMeta {
                period: Some(Period::new("2026-06-01", "2026-06-30")),
                ..Default::default()
            })
            .currency(Currency::EUR)
            .positions(vec![
                LineItem::fixed("Arbeit", Amount::parse("100.00000").unwrap())
                    .period(line_from, line_to)
                    .build()
                    .unwrap(),
            ])
            .build()
    };

    // Inside, and coterminous, are both fine.
    build("2026-06-01", "2026-06-14").unwrap().assert_valid();
    build("2026-06-01", "2026-06-30").unwrap().assert_valid();

    // R110 — the line starts before the invoicing period.
    let err = build("2026-05-28", "2026-06-14").unwrap_err();
    assert!(err.to_string().contains("R110"), "got: {err}");
    // R111 — the line ends after it.
    let err = build("2026-06-15", "2026-07-02").unwrap_err();
    assert!(err.to_string().contains("R111"), "got: {err}");

    // BR-29 on the document's own period.
    let err = BillingDocument::builder()
        .meta(DocumentMeta {
            period: Some(Period::new("2026-06-30", "2026-06-01")),
            ..Default::default()
        })
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Arbeit", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("BR-29"), "got: {err}");

    // A non-ISO invoicing period disables the comparison rather than failing it —
    // the crate never guesses at a date format it cannot order.
    BillingDocument::builder()
        .meta(DocumentMeta {
            period: Some(Period::new("1.6.2026", "30.6.2026")),
            ..Default::default()
        })
        .currency(Currency::EUR)
        .positions(vec![
            LineItem::fixed("Arbeit", Amount::parse("100.00000").unwrap())
                .period("2026-05-01", "2026-05-31")
                .build()
                .unwrap(),
        ])
        .build()
        .unwrap()
        .assert_valid();
}

/// `RateSchedule::split` and `apply_peak` both refuse a negative quantity;
/// `RateLookup::rate_for` did not. Every bound in a lookup table is positive, so a
/// negative parameter is below all of them and the `<=` search returned the
/// **lowest** band — the cheapest rate — for what is a data error. The three rate
/// tables now agree.
#[test]
fn a_negative_lookup_parameter_is_an_error_not_the_cheapest_band() {
    let lookup = billing::RateLookup::builder()
        .at_most(dec!(10), Amount::parse("0.00811").unwrap())
        .at_most(dec!(40), Amount::parse("0.00679").unwrap())
        .fallback(Amount::parse("0.00556").unwrap())
        .build()
        .unwrap();

    // Zero is a real installed capacity; negative is not.
    assert_eq!(
        lookup.rate_for(dec!(0)).unwrap(),
        Amount::parse("0.00811").unwrap()
    );
    let err = lookup.rate_for(dec!(-1)).unwrap_err();
    assert!(err.to_string().contains("non-negative"), "got: {err}");

    // The sibling rate tables have always refused it, which is the point.
    let schedule = RateSchedule::capacity()
        .unit("kW")
        .band(RateBand::over(dec!(0), Amount::parse("10.00000").unwrap()))
        .build()
        .unwrap();
    assert!(schedule.apply_peak(dec!(-1)).is_err());
    assert!(
        RateSchedule::graduated()
            .unit("kWh")
            .band(RateBand::over(dec!(0), Amount::parse("0.30000").unwrap()))
            .build()
            .unwrap()
            .split(dec!(-1))
            .is_err()
    );
}
