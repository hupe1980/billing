//! End-to-end tests for the invoice-grade features: VAT breakdown, prepayments,
//! cash rounding and reversal.
//!
//! Each test is a realistic document shape that a lump `tax_total` cannot express
//! lawfully, exercised against the EN 16931 semantics the engine implements.

use billing::prelude::*;
use billing::{CashRounding, FixedRateTax, PerUnitLevy, PercentageCharge};
use rust_decimal::dec;

fn meta(number: &str) -> DocumentMeta {
    DocumentMeta {
        invoice_number: number.into(),
        currency: Currency::EUR,
        ..Default::default()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// VAT breakdown (EN 16931 BG-23)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn mixed_rate_invoice_produces_one_breakdown_line_per_rate() {
    // The case a single tax_total cannot express lawfully: 19% on goods, 7% on
    // the reduced-rate line. §14 Abs. 4 Nr. 7 UStG requires the net *per rate*.
    let positions = vec![
        LineItem::fixed("Elektronik", Amount::parse("100.00000").unwrap())
            .tag("standard")
            .build()
            .unwrap(),
        LineItem::fixed("Buch", Amount::parse("50.00000").unwrap())
            .tag("reduced")
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(
            FixedRateTax::new("MwSt 19%", dec!(0.19))
                .unwrap()
                .with_tag("standard"),
        ),
        Box::new(
            FixedRateTax::new("MwSt 7%", dec!(0.07))
                .unwrap()
                .with_tag("reduced"),
        ),
    ];
    let doc = BillingDocument::from_positions(meta("INV-1"), positions, taxes, vec![]).unwrap();

    let bd = doc.tax_breakdown();
    assert_eq!(bd.len(), 2, "one line per rate");

    assert_eq!(bd[0].category, TaxCategory::Standard);
    assert_eq!(bd[0].rate, dec!(0.19));
    assert_eq!(bd[0].taxable_base, Amount::parse("100.00000").unwrap());
    assert_eq!(bd[0].tax_amount, Amount::parse("19.00000").unwrap());
    assert_eq!(bd[0].rate_percent(), dec!(19));

    assert_eq!(bd[1].rate, dec!(0.07));
    assert_eq!(bd[1].taxable_base, Amount::parse("50.00000").unwrap());
    assert_eq!(bd[1].tax_amount, Amount::parse("3.50000").unwrap());

    // The breakdown reconciles with the document total (BR-CO-14).
    let sum: Amount<5> = bd.iter().map(|e| e.tax_amount).sum();
    assert_eq!(sum, doc.tax_total());
    assert_eq!(doc.gross_total(), Amount::parse("172.50000").unwrap());
    doc.assert_valid();
}

#[test]
fn same_rate_from_two_layers_merges_into_one_breakdown_line() {
    // BR-CO-18: exactly one breakdown line per (category, rate) pair.
    let positions = vec![
        LineItem::fixed("A", Amount::parse("100.00000").unwrap())
            .tag("a")
            .build()
            .unwrap(),
        LineItem::fixed("B", Amount::parse("200.00000").unwrap())
            .tag("b")
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(
            FixedRateTax::new("MwSt A", dec!(0.19))
                .unwrap()
                .with_tag("a"),
        ),
        Box::new(
            FixedRateTax::new("MwSt B", dec!(0.19))
                .unwrap()
                .with_tag("b"),
        ),
    ];
    let doc = BillingDocument::from_positions(meta("INV-2"), positions, taxes, vec![]).unwrap();

    assert_eq!(doc.tax_breakdown().len(), 1, "same rate must merge");
    let e = &doc.tax_breakdown()[0];
    assert_eq!(e.taxable_base, Amount::parse("300.00000").unwrap());
    assert_eq!(e.tax_amount, Amount::parse("57.00000").unwrap());
    doc.assert_valid();
}

#[test]
fn reverse_charge_invoice_carries_zero_tax_and_a_reason() {
    // §13b UStG: the recipient owes the tax. Category AE, 0%, reason required.
    let vat = FixedRateTax::new("Reverse charge", dec!(0))
        .unwrap()
        .with_category(TaxCategory::ReverseCharge)
        .with_exemption_reason("Steuerschuldnerschaft des Leistungsempfängers (§13b UStG)");

    let doc = BillingDocument::from_positions(
        meta("INV-3"),
        vec![
            LineItem::fixed("Bauleistung", Amount::parse("10000.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(vat)],
        vec![],
    )
    .unwrap();

    let e = &doc.tax_breakdown()[0];
    assert_eq!(e.category, TaxCategory::ReverseCharge);
    assert_eq!(e.tax_amount, Amount::<5>::ZERO);
    assert_eq!(e.taxable_base, Amount::parse("10000.00000").unwrap());
    assert!(e.exemption_reason.is_some());
    // Gross equals net: no VAT is charged by the supplier.
    assert_eq!(doc.gross_total(), doc.net_total());
    doc.assert_valid();
}

#[test]
fn zero_tax_category_with_a_nonzero_rate_is_rejected() {
    // A category that carries no tax cannot have a rate — BR-AE-09 etc.
    let bad = FixedRateTax::new("Bad", dec!(0.19))
        .unwrap()
        .with_category(TaxCategory::ReverseCharge)
        .with_exemption_reason("x");
    assert!(bad.breakdown(&[]).is_err());
}

#[test]
fn missing_or_forbidden_exemption_reason_is_rejected() {
    // E requires a reason...
    let missing = FixedRateTax::new("Exempt", dec!(0))
        .unwrap()
        .with_category(TaxCategory::Exempt);
    assert!(missing.breakdown(&[]).is_err());

    // ...and S forbids one.
    let forbidden = FixedRateTax::new("Standard", dec!(0.19))
        .unwrap()
        .with_exemption_reason("not allowed here");
    assert!(forbidden.breakdown(&[]).is_err());
}

#[test]
fn non_vat_layers_contribute_nothing_to_the_breakdown() {
    // A platform commission and a per-unit excise are not VAT: the commission is a
    // commercial charge and the excise is part of the VAT *base*, not a VAT.
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
        Box::new(PercentageCharge::new("Platform fee", dec!(0.02)).unwrap()),
        Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap()),
    ];
    let doc = BillingDocument::from_positions(meta("INV-4"), positions, taxes, vec![]).unwrap();

    // Exactly one VAT line, whose base includes the levy and the commission.
    assert_eq!(doc.tax_breakdown().len(), 1);
    let e = &doc.tax_breakdown()[0];
    // 300.00 net + 20.50 levy + 6.41 fee = 326.91 base
    assert_eq!(e.taxable_base, Amount::parse("326.91000").unwrap());
    assert_eq!(e.tax_amount, Amount::parse("62.11290").unwrap());
    doc.assert_valid();
}

// ─────────────────────────────────────────────────────────────────────────────
// Prepayments and amount due (BT-113 / BT-115)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn prepayments_reduce_the_amount_due_but_not_the_taxable_base() {
    // The utility Abschlagszahlung case. Modelling advances as negative lines
    // would shrink the VAT base and under-declare output tax — in Germany that
    // makes the whole VAT amount payable again under §14c Abs. 1 UStG.
    let doc = BillingDocument::from_positions(
        meta("INV-5"),
        vec![
            LineItem::fixed("Jahresverbrauch", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap()
    .with_prepaid(Amount::parse("900.00000").unwrap())
    .unwrap();

    // Totals and breakdown are untouched by the prepayment.
    assert_eq!(doc.net_total(), Amount::parse("1000.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("190.00000").unwrap());
    assert_eq!(doc.gross_total(), Amount::parse("1190.00000").unwrap());
    assert_eq!(
        doc.tax_breakdown()[0].taxable_base,
        Amount::parse("1000.00000").unwrap()
    );
    // Only the payable figure moves: 1190 − 900 = 290.
    assert_eq!(
        doc.amount_due().unwrap(),
        Amount::parse("290.00000").unwrap()
    );
    doc.assert_valid();
}

#[test]
fn amount_due_may_be_negative_when_prepayments_exceed_the_total() {
    // Credit balance: the supplier owes the customer. Not clamped to zero.
    let doc = BillingDocument::from_positions(
        meta("INV-6"),
        vec![
            LineItem::fixed("Verbrauch", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_prepaid(Amount::parse("250.00000").unwrap())
    .unwrap();

    assert_eq!(
        doc.amount_due().unwrap(),
        Amount::parse("-150.00000").unwrap()
    );
}

#[test]
fn negative_prepaid_is_rejected() {
    let doc = BillingDocument::from_positions(meta("INV-7"), vec![], vec![], vec![]).unwrap();
    assert!(
        doc.with_prepaid(Amount::parse("-1.00000").unwrap())
            .is_err()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Cash rounding (BT-114)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn swiss_rappenrundung_adjusts_only_the_payable_amount() {
    let doc = BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "CH-1".into(),
            currency: Currency::CHF,
            ..Default::default()
        },
        vec![
            LineItem::fixed("Leistung", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MWST", dec!(0.081)).unwrap())],
        vec![],
    )
    .unwrap();

    // 100.00 + 8.1% = 108.10 exactly — already a multiple of 0.05.
    assert_eq!(doc.gross_total(), Amount::parse("108.10000").unwrap());

    let rappen = CashRounding::new(
        Amount::parse("0.05000").unwrap(),
        RoundingStrategy::MidpointAwayFromZero,
    )
    .unwrap();
    let doc = doc.with_cash_rounding(rappen).unwrap();
    assert_eq!(doc.rounding(), Amount::<5>::ZERO);

    // A total that is not a multiple: 12.34 → 12.35, BT-114 = +0.01.
    let doc2 = BillingDocument::from_positions(
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
    .with_cash_rounding(rappen)
    .unwrap();

    assert_eq!(
        doc2.gross_total(),
        Amount::parse("12.34000").unwrap(),
        "gross untouched"
    );
    assert_eq!(doc2.rounding(), Amount::parse("0.01000").unwrap());
    assert_eq!(
        doc2.amount_due().unwrap(),
        Amount::parse("12.35000").unwrap()
    );
    doc2.assert_valid();
}

#[test]
fn cash_rounding_applies_after_prepayment_deduction() {
    // The tenderable figure is what remains to pay, not the gross.
    let doc = BillingDocument::from_positions(
        meta("INV-8"),
        vec![
            LineItem::fixed("Leistung", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_prepaid(Amount::parse("87.97000").unwrap())
    .unwrap()
    .with_cash_rounding(
        CashRounding::new(
            Amount::parse("0.05000").unwrap(),
            RoundingStrategy::MidpointAwayFromZero,
        )
        .unwrap(),
    )
    .unwrap();

    // Payable before rounding: 100.00 − 87.97 = 12.03 → 12.05
    assert_eq!(doc.rounding(), Amount::parse("0.02000").unwrap());
    assert_eq!(
        doc.amount_due().unwrap(),
        Amount::parse("12.05000").unwrap()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Reversal / credit note
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn credit_note_negates_everything_and_settles_to_zero() {
    let inv = BillingDocument::from_positions(
        meta("INV-9"),
        vec![
            LineItem::for_usage("Arbeit", dec!(1000), "kWh", dec!(0.30), "EUR/kWh")
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();

    let credit = inv
        .reverse(DocumentMeta {
            invoice_number: "CN-9".into(),
            currency: Currency::EUR,
            ..Default::default()
        })
        .unwrap();

    assert_eq!(credit.net_total(), Amount::parse("-300.00000").unwrap());
    assert_eq!(credit.tax_total(), Amount::parse("-57.00000").unwrap());
    assert_eq!(credit.gross_total(), Amount::parse("-357.00000").unwrap());

    // The VAT breakdown is negated too — a credit note must reverse the reported
    // base, not just the total.
    assert_eq!(
        credit.tax_breakdown()[0].taxable_base,
        Amount::parse("-300.00000").unwrap()
    );
    assert_eq!(
        credit.tax_breakdown()[0].tax_amount,
        Amount::parse("-57.00000").unwrap()
    );

    // Signs flip so sign-based filtering stays meaningful.
    assert!(credit.net_positions()[0].is_credit());
    // Quantities are NOT negated: the reversal is a negative price, not a
    // negative quantity (which LineItem::validate rejects outright).
    assert_eq!(credit.net_positions()[0].quantity_value(), Some(dec!(1000)));
    credit.net_positions()[0].validate().unwrap();

    // Invoice + credit note = nothing owed.
    assert_eq!(
        inv.gross_total().checked_add(credit.gross_total()).unwrap(),
        Amount::<5>::ZERO
    );
    credit.assert_valid();
}

// ─────────────────────────────────────────────────────────────────────────────
// Currency minor units
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn minor_units_follow_iso_4217_not_a_hardcoded_two() {
    assert_eq!(Currency::EUR.minor_units(), Some(2));
    assert_eq!(Currency::USD.minor_units(), Some(2));
    assert_eq!(Currency::JPY.minor_units(), Some(0));
    assert_eq!(Currency::new("ISK").unwrap().minor_units(), Some(0));
    assert_eq!(Currency::new("KWD").unwrap().minor_units(), Some(3));
    assert_eq!(Currency::new("BHD").unwrap().minor_units(), Some(3));
    assert_eq!(Currency::new("CLF").unwrap().minor_units(), Some(4));
    // "No minor unit" is distinct from "zero decimals".
    assert_eq!(Currency::XXX.minor_units(), None);
    assert_eq!(Currency::new("XAU").unwrap().minor_units(), None);
    // Unregistered but well-formed codes fall back to the common case.
    assert_eq!(Currency::new("ZZZ").unwrap().minor_units(), Some(2));
}

#[test]
fn minor_unit_increment_respects_precision() {
    assert_eq!(
        Currency::EUR.minor_unit_increment::<5>(),
        Some(Amount::parse("0.01000").unwrap())
    );
    assert_eq!(
        Currency::JPY.minor_unit_increment::<5>(),
        Some(Amount::parse("1.00000").unwrap())
    );
    assert_eq!(
        Currency::new("KWD").unwrap().minor_unit_increment::<5>(),
        Some(Amount::parse("0.00100").unwrap())
    );
    // A 4-decimal currency cannot be represented at P=2.
    assert_eq!(
        Currency::new("CLF").unwrap().minor_unit_increment::<2>(),
        None
    );
    assert_eq!(Currency::XXX.minor_unit_increment::<5>(), None);
}

#[test]
fn cash_rounding_to_the_currency_minor_unit() {
    // The common "round the payable amount to whole cents" case, expressed
    // through the currency rather than a magic constant.
    let inc = Currency::EUR.minor_unit_increment::<5>().unwrap();
    let rule = CashRounding::new(inc, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(
        rule.round(Amount::parse("12.34567").unwrap()).unwrap(),
        Amount::parse("12.35000").unwrap()
    );

    // Yen has no sub-unit at all.
    let yen = Currency::JPY.minor_unit_increment::<5>().unwrap();
    let rule = CashRounding::new(yen, RoundingStrategy::MidpointAwayFromZero).unwrap();
    assert_eq!(
        rule.round(Amount::parse("1234.60000").unwrap()).unwrap(),
        Amount::parse("1235.00000").unwrap()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Defects found by the adversarial verification pass
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn conflicting_exemption_reasons_cannot_be_silently_merged() {
    // Two Exempt layers at 0% with DIFFERENT BT-120 texts share a (category, rate)
    // group. Keeping only the first would drop a legally required justification.
    let positions = vec![
        LineItem::fixed("Kurs", Amount::parse("100.00000").unwrap())
            .tag("edu")
            .build()
            .unwrap(),
        LineItem::fixed("Zinsen", Amount::parse("50.00000").unwrap())
            .tag("fin")
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(
            FixedRateTax::new("Bildung", dec!(0))
                .unwrap()
                .with_category(TaxCategory::Exempt)
                .with_exemption_reason("Art. 132 education")
                .with_tag("edu"),
        ),
        Box::new(
            FixedRateTax::new("Finanz", dec!(0))
                .unwrap()
                .with_category(TaxCategory::Exempt)
                .with_exemption_reason("Art. 135 financial services")
                .with_tag("fin"),
        ),
    ];
    let err =
        BillingDocument::from_positions(meta("INV-X1"), positions, taxes, vec![]).unwrap_err();
    assert!(
        err.to_string().contains("conflicting exemption reasons"),
        "{err}"
    );

    // Identical reasons merge without complaint.
    let positions = vec![
        LineItem::fixed("A", Amount::parse("100.00000").unwrap())
            .tag("a")
            .build()
            .unwrap(),
        LineItem::fixed("B", Amount::parse("50.00000").unwrap())
            .tag("b")
            .build()
            .unwrap(),
    ];
    let taxes: Vec<Box<dyn TaxLayer>> = vec![
        Box::new(
            FixedRateTax::new("X", dec!(0))
                .unwrap()
                .with_category(TaxCategory::Exempt)
                .with_exemption_reason("Art. 132")
                .with_tag("a"),
        ),
        Box::new(
            FixedRateTax::new("Y", dec!(0))
                .unwrap()
                .with_category(TaxCategory::Exempt)
                .with_exemption_reason("Art. 132")
                .with_tag("b"),
        ),
    ];
    let doc = BillingDocument::from_positions(meta("INV-X2"), positions, taxes, vec![]).unwrap();
    assert_eq!(doc.tax_breakdown().len(), 1);
    assert_eq!(
        doc.tax_breakdown()[0].taxable_base,
        Amount::parse("150.00000").unwrap()
    );
}

#[test]
fn reversing_a_negative_debit_does_not_mint_an_invalid_credit_line() {
    // A Debit with a NEGATIVE net (negative spot price, or VAT on a negative base)
    // used to flip to a Credit with a POSITIVE net — a state LineItem::validate
    // rejects, so the document passed assert_valid() but could not be persisted.
    let doc = BillingDocument::from_positions(
        meta("INV-X3"),
        vec![
            LineItem::for_usage("EPEX negativ", dec!(1000), "kWh", dec!(-0.04), "EUR/kWh")
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();
    assert!(doc.net_total().is_negative());

    let credit = doc.reverse(meta("CN-X3")).unwrap();
    credit.assert_valid();
    for p in credit.all_positions() {
        p.validate()
            .unwrap_or_else(|e| panic!("reversed position {:?} is invalid: {e}", p.description));
    }
    assert!(credit.net_total().is_positive());
}

#[test]
fn changing_prepaid_after_cash_rounding_recomputes_the_adjustment() {
    // The rounding is a function of gross − prepaid. Applying the rule first and
    // then the prepayment used to leave a stale adjustment and an amount_due that
    // was not a tenderable multiple.
    let rule = CashRounding::new(
        Amount::parse("0.05000").unwrap(),
        RoundingStrategy::MidpointAwayFromZero,
    )
    .unwrap();
    let base = || {
        BillingDocument::from_positions(
            meta("INV-X4"),
            vec![
                LineItem::fixed("Leistung", Amount::parse("12.34000").unwrap())
                    .build()
                    .unwrap(),
            ],
            vec![],
            vec![],
        )
        .unwrap()
    };

    let rounding_first = base()
        .with_cash_rounding(rule)
        .unwrap()
        .with_prepaid(Amount::parse("2.03000").unwrap())
        .unwrap();
    let prepaid_first = base()
        .with_prepaid(Amount::parse("2.03000").unwrap())
        .unwrap()
        .with_cash_rounding(rule)
        .unwrap();

    // Order no longer matters, and the result is genuinely tenderable.
    assert_eq!(
        rounding_first.amount_due().unwrap(),
        prepaid_first.amount_due().unwrap()
    );
    assert_eq!(
        rounding_first.amount_due().unwrap(),
        Amount::parse("10.30000").unwrap()
    );
    assert_eq!(rounding_first.amount_due().unwrap().to_raw() % 5_000, 0);
    rounding_first.assert_valid();
}

#[test]
fn allocation_splits_prepaid_and_rounding_instead_of_dropping_them() {
    // Dropping BT-113/BT-114 re-bills money the customer already paid: the
    // recipients' amounts due summed to the gross rather than the amount due.
    let doc = BillingDocument::from_positions(
        meta("INV-X5"),
        vec![
            LineItem::fixed("Leistung", Amount::parse("12.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![],
        vec![],
    )
    .unwrap()
    .with_prepaid(Amount::parse("4.00000").unwrap())
    .unwrap();

    let docs = EqualAllocation::new(3).unwrap().allocate(&doc).unwrap();
    let prepaid: Amount<5> = docs.iter().map(|d| d.prepaid()).sum();
    let due: Amount<5> = docs
        .iter()
        .map(|d| d.amount_due().unwrap())
        .fold(Amount::<5>::ZERO, |a, b| a.checked_add(b).unwrap());

    assert_eq!(prepaid, doc.prepaid(), "prepaid must not vanish");
    assert_eq!(due, doc.amount_due().unwrap(), "amount due must not drift");
    for d in &docs {
        d.assert_valid();
    }
}

#[test]
fn allocation_penny_correction_cannot_flip_a_credit_line_positive() {
    // A correction that pushes a tiny credit across zero used to leave
    // Sign::Credit on a positive amount.
    let positions = vec![
        LineItem::fixed("Charge", Amount::parse("100.00000").unwrap())
            .build()
            .unwrap(),
        LineItem::credit_fixed("Tiny credit", Amount::parse("0.00001").unwrap())
            .build()
            .unwrap(),
    ];
    let doc = BillingDocument::from_positions(meta("INV-X6"), positions, vec![], vec![]).unwrap();

    for n in 2usize..12 {
        let docs = EqualAllocation::new(n).unwrap().allocate(&doc).unwrap();
        for d in &docs {
            d.assert_valid();
            for p in d.all_positions() {
                p.validate().unwrap_or_else(|e| {
                    panic!(
                        "n={n}: position {:?} invalid after correction: {e}",
                        p.description
                    )
                });
            }
        }
    }
}

#[cfg(feature = "serde")]
#[test]
fn deserialisation_rejects_a_negative_prepaid_and_an_inconsistent_breakdown() {
    let doc = BillingDocument::from_positions(
        meta("INV-X7"),
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
    assert!(serde_json::from_str::<BillingDocument>(&json).is_ok());

    // A negative BT-113 is meaningless — with_prepaid rejects it, and so must serde.
    let bad = json.replace(r#""prepaid":"0.00000""#, r#""prepaid":"-999.00000""#);
    assert_ne!(bad, json);
    assert!(serde_json::from_str::<BillingDocument>(&bad).is_err());

    // A breakdown whose tax does not follow from base × rate (BR-CO-17).
    let bad = json.replace(
        r#""tax_amount":"19.00000""#,
        r#""tax_amount":"12345.00000""#,
    );
    assert_ne!(bad, json);
    assert!(serde_json::from_str::<BillingDocument>(&bad).is_err());
}

#[test]
fn with_extra_position_is_refused_when_it_would_stale_the_breakdown() {
    let doc = BillingDocument::from_positions(
        meta("INV-X8"),
        vec![
            LineItem::fixed("Base", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("VAT", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();
    let extra = LineItem::fixed("Extra", Amount::parse("50.00000").unwrap())
        .build()
        .unwrap();
    assert!(doc.with_extra_position(extra).is_err());
}
