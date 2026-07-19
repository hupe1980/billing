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
fn from_decimal_returns_none_instead_of_panicking() {
    assert_eq!(Amount::<5>::from_decimal(Decimal::MAX), None);
    assert_eq!(Amount::<5>::from_decimal(Decimal::MIN), None);
    assert!(Amount::<5>::checked_from_decimal(Decimal::MAX).is_err());
    assert!(Amount::<5>::try_from_decimal(Decimal::MAX).is_err());
}

#[test]
fn line_item_build_returns_err_instead_of_panicking_on_extreme_inputs() {
    let r = LineItem::for_usage("x", Decimal::MAX, "u", Decimal::MAX, "c/u").build();
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
    let expected = Amount::<5>::from_decimal(
        (q.value * p.value)
            .round_dp_with_strategy(5, rust_decimal::RoundingStrategy::MidpointAwayFromZero),
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
    let full = LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh")
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
        LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh")
            .build()
            .unwrap(),
        LineItem::for_usage("Gas", dec!(500), "m³", dec!(0.12), "EUR/m³")
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
}

#[test]
fn schedule_rejects_non_monotonic_bands() {
    // `up_to` leaves `lower` as None, so the contiguity check never fired and this
    // built successfully, then failed later inside split() with a misleading
    // "quantity exceeds coverage" message.
    let r = TariffSchedule::graduated()
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("1.00000").unwrap(),
        ))
        .band(TariffBand::up_to(
            dec!(50),
            Amount::parse("2.00000").unwrap(),
        ))
        .build();
    assert!(
        r.is_err(),
        "descending upper bounds must be rejected at build time"
    );

    // Equal consecutive bounds are a zero-width band — also rejected.
    assert!(
        TariffSchedule::graduated()
            .band(TariffBand::up_to(
                dec!(100),
                Amount::parse("1.00000").unwrap()
            ))
            .band(TariffBand::up_to(
                dec!(100),
                Amount::parse("2.00000").unwrap()
            ))
            .build()
            .is_err()
    );

    // The ascending equivalent still builds and prices correctly.
    let ok = TariffSchedule::graduated()
        .band(TariffBand::up_to(
            dec!(50),
            Amount::parse("1.00000").unwrap(),
        ))
        .band(TariffBand::up_to(
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
            .band(TouBand::new("HT", Amount::parse("0.32").unwrap()))
            .band(TouBand::new("HT", Amount::parse("0.18").unwrap()))
            .build()
            .is_err(),
        "a duplicate band name is unreachable and almost always a config error"
    );
    assert!(
        TimeOfUsePricing::builder()
            .band(TouBand::new("HT", Amount::parse("-0.1").unwrap()))
            .build()
            .is_err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Currency is explicit, never assumed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn generated_labels_use_the_configured_currency_not_hardcoded_eur() {
    let sched = TariffSchedule::graduated()
        .unit("GB")
        .currency(Currency::USD)
        .band(TariffBand::over(dec!(0), Amount::parse("0.10000").unwrap()))
        .build()
        .unwrap();
    let items = sched.split(dec!(100)).unwrap();
    assert_eq!(items[0].unit_price.as_ref().unwrap().unit, "USD/GB");
}

#[test]
fn unconfigured_currency_is_visible_as_xxx_not_silently_eur() {
    let sched = TariffSchedule::graduated()
        .unit("GB")
        .band(TariffBand::over(dec!(0), Amount::parse("0.10000").unwrap()))
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
        LineItem::for_usage("Spot", dec!(1000), "kWh", dec!(-0.2), "EUR/kWh")
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
        let sched = TariffSchedule::graduated()
            .unit("m³")
            .currency(Currency::EUR)
            .band(TariffBand::up_to(
                dec!(5),
                Amount::parse("0.80000").unwrap(),
            ))
            .band(TariffBand::over(dec!(5), Amount::parse("1.40000").unwrap()))
            .build()
            .unwrap();
        let json = serde_json::to_string(&sched).unwrap();
        let back: TariffSchedule = serde_json::from_str(&json).unwrap();
        assert_eq!(back.unit(), "m³");
        assert_eq!(back.currency(), Currency::EUR);
        assert_eq!(
            back.split(dec!(10)).unwrap().len(),
            sched.split(dec!(10)).unwrap().len()
        );

        // A schedule with a gap between bands is refused at the boundary.
        let bad = json.replace(r#""upper":"5""#, r#""upper":"3""#);
        if bad != json {
            assert!(serde_json::from_str::<TariffSchedule>(&bad).is_err());
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
        LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh")
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
    let sched = TariffSchedule::block()
        .unit("GB")
        .band(TariffBand::block(
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
    let item = LineItem::for_usage("Arbeit", dec!(300), "kWh", dec!(0.30), "EUR/kWh")
        .build()
        .unwrap();
    let third = prorate(&item, 1, 3, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(third.quantity_value(), Some(dec!(100)));

    let item = LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh")
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
    let r = TariffSchedule::graduated()
        .band(TariffBand::between(
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
        TariffSchedule::graduated()
            .band(TariffBand::up_to(
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
    let volume = TariffSchedule::volume()
        .unit("kWh")
        .currency(Currency::EUR)
        .band(TariffBand::up_to(
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

    let capacity = TariffSchedule::capacity()
        .unit("Mbps")
        .currency(Currency::EUR)
        .band(TariffBand::up_to(
            dec!(100),
            Amount::parse("50.00000").unwrap(),
        ))
        .build()
        .unwrap();
    assert!(capacity.apply_peak(dec!(150)).is_err());
    assert!(capacity.apply_peak(dec!(50)).is_ok());

    // An open-ended top band covers everything, as before.
    let open = TariffSchedule::volume()
        .unit("kWh")
        .currency(Currency::EUR)
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
    let item = LineItem::for_usage_rounded(
        "Arbeit",
        dec!(1),
        "kWh",
        dec!(0.1234567890123),
        "EUR/kWh",
        99,
        RoundingStrategy::MidpointAwayFromZero,
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
            .band(TouBand::new(reserved, Amount::parse("0.30000").unwrap()))
            .build();
        assert!(r.is_err(), "band name {reserved:?} must be rejected");
    }
    assert!(
        TimeOfUsePricing::builder()
            .band(TouBand::new("HT", Amount::parse("0.30000").unwrap()))
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

    let mut neg_qty = LineItem::for_usage("x", dec!(5), "kWh", dec!(1), "EUR/kWh")
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
fn empty_unit_labels_are_rejected() {
    // An empty unit renders as "EUR/" in a price label and a bare space in the
    // description — visible nonsense on an invoice.
    assert!(
        TariffSchedule::graduated()
            .unit("")
            .band(TariffBand::over(dec!(0), Amount::parse("1.00000").unwrap()))
            .build()
            .is_err()
    );
    assert!(
        TimeOfUsePricing::builder()
            .unit("  ")
            .band(TouBand::new("HT", Amount::parse("0.3").unwrap()))
            .build()
            .is_err()
    );
}

#[cfg(feature = "serde")]
#[test]
fn schedule_mode_deserialises_from_the_documented_lowercase_names() {
    // The module docs advertise JSON/YAML config loading with lowercase mode names,
    // but the derived names were TitleCase, so `"graduated"` failed.
    let sched = TariffSchedule::graduated()
        .unit("kWh")
        .currency(Currency::EUR)
        .band(TariffBand::over(dec!(0), Amount::parse("0.30000").unwrap()))
        .build()
        .unwrap();
    let json = serde_json::to_string(&sched).unwrap();
    assert!(json.contains(r#""mode":"graduated""#), "{json}");
    let back: TariffSchedule = serde_json::from_str(&json).unwrap();
    assert_eq!(back.unit(), "kWh");
}
