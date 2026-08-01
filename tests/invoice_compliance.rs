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
    // One breakdown line per (category, rate) — BR-S-08 for the taxed categories.
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
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1000), "kWh"),
                UnitPrice::new(dec!(0.30), "EUR/kWh"),
            )
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

/// BT-121 merges under the same rule as BT-120, and either field may arrive from
/// whichever of the merged entries happens to carry it.
#[test]
fn exemption_reason_codes_merge_conflict_and_carry_over_like_the_texts() {
    fn two_exempt_layers(
        number: &str,
        first: FixedRateTax,
        second: FixedRateTax,
    ) -> Result<BillingDocument, BillingError> {
        BillingDocument::from_positions(
            meta(number),
            vec![
                LineItem::fixed("A", Amount::parse("100.00000").unwrap())
                    .tag("a")
                    .build()
                    .unwrap(),
                LineItem::fixed("B", Amount::parse("50.00000").unwrap())
                    .tag("b")
                    .build()
                    .unwrap(),
            ],
            vec![Box::new(first), Box::new(second)],
            vec![],
        )
    }

    let exempt = |name: &str, tag: &str| {
        FixedRateTax::new(name, dec!(0))
            .unwrap()
            .with_category(TaxCategory::Exempt)
            .with_tag(tag)
    };

    // Two different VATEX codes in one breakdown line cannot both be stated, and
    // keeping whichever arrived first would silently drop a legally required
    // justification for half the base.
    let err = two_exempt_layers(
        "VATEX-1",
        exempt("Bildung", "a").with_exemption_reason_code("VATEX-EU-132"),
        exempt("Finanz", "b").with_exemption_reason_code("VATEX-EU-135"),
    )
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("conflicting exemption reason codes"),
        "{err}"
    );

    // The same code twice is one code, and merges.
    let doc = two_exempt_layers(
        "VATEX-2",
        exempt("X", "a").with_exemption_reason_code("VATEX-EU-132"),
        exempt("Y", "b").with_exemption_reason_code("VATEX-EU-132"),
    )
    .unwrap();
    assert_eq!(doc.tax_breakdown().len(), 1);
    assert_eq!(
        doc.tax_breakdown()[0].exemption_reason_code.as_deref(),
        Some("VATEX-EU-132")
    );
    assert_eq!(
        doc.tax_breakdown()[0].taxable_base,
        Amount::parse("150.00000").unwrap()
    );

    // BR-E-10 accepts the code *or* the text, so the two layers may satisfy it
    // differently. The merged line has to end up with both — dropping whichever
    // the first entry lacked is how a lawful pair becomes an unlawful single.
    let doc = two_exempt_layers(
        "VATEX-3",
        exempt("X", "a").with_exemption_reason_code("VATEX-EU-132"),
        exempt("Y", "b")
            .with_exemption_reason("Art. 132 education")
            .with_exemption_reason_code("VATEX-EU-132"),
    )
    .unwrap();
    assert_eq!(doc.tax_breakdown().len(), 1);
    assert_eq!(
        doc.tax_breakdown()[0].exemption_reason.as_deref(),
        Some("Art. 132 education"),
        "BT-120 must carry over from the entry that had it"
    );
    assert_eq!(
        doc.tax_breakdown()[0].exemption_reason_code.as_deref(),
        Some("VATEX-EU-132")
    );

    // And the mirror image: the text arrives first, the code second.
    let doc = two_exempt_layers(
        "VATEX-4",
        exempt("X", "a").with_exemption_reason("Art. 132 education"),
        exempt("Y", "b")
            .with_exemption_reason("Art. 132 education")
            .with_exemption_reason_code("VATEX-EU-132"),
    )
    .unwrap();
    assert_eq!(doc.tax_breakdown().len(), 1);
    assert_eq!(
        doc.tax_breakdown()[0].exemption_reason_code.as_deref(),
        Some("VATEX-EU-132"),
        "BT-121 must carry over from the entry that had it"
    );
    assert_eq!(
        doc.tax_breakdown()[0].exemption_reason.as_deref(),
        Some("Art. 132 education")
    );
    doc.assert_valid();
}

#[test]
fn reversing_a_negative_debit_does_not_mint_an_invalid_credit_line() {
    // A Debit with a NEGATIVE net (negative spot price, or VAT on a negative base)
    // used to flip to a Credit with a POSITIVE net — a state LineItem::validate
    // rejects, so the document passed assert_valid() but could not be persisted.
    let doc = BillingDocument::from_positions(
        meta("INV-X3"),
        vec![
            LineItem::for_usage(
                "EPEX negativ",
                Quantity::new(dec!(1000), "kWh"),
                UnitPrice::new(dec!(-0.04), "EUR/kWh"),
            )
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

// ─────────────────────────────────────────────────────────────────────────────
// EN 16931 amount precision — BR-DEC-* decimal limits and BR-CO-* identities
//
// Every monetary amount an EN 16931 invoice carries is capped at two decimals
// (BR-DEC-09/12/13/14/16/17/18/19/20 and BR-DEC-23 for the line amount), while the
// totals identities must still hold exactly at that precision (BR-CO-10/13/14/15/16
// and BR-CO-17). A serialiser cannot satisfy both by rounding on the way out; the
// leaves have to be rounded and the aggregates recomputed from them.
// ─────────────────────────────────────────────────────────────────────────────

/// Rounding the finished totals independently — what a serialiser would do —
/// breaks the identities EN 16931 also checks. Both counterexamples come from
/// ordinary inputs, and both are why `amount_scale` exists.
#[test]
fn rounding_a_finished_document_breaks_the_en16931_identities() {
    let r2 = |a: Amount<5>| a.round_to::<2>(RoundingStrategy::MidpointAwayFromZero);

    // BR-CO-10: Σ line net amounts (BT-131) = BT-106.
    let positions: Vec<LineItem> = ["0.00500", "0.00500", "0.00500"]
        .iter()
        .map(|a| {
            LineItem::fixed("x", Amount::parse(a).unwrap())
                .build()
                .unwrap()
        })
        .collect();
    let doc = BillingDocument::from_positions(meta("BR-CO-10"), positions, vec![], vec![]).unwrap();
    let sum_of_rounded: Amount<2> = doc.net_positions().iter().map(|p| r2(p.net_amount)).sum();
    assert_ne!(
        sum_of_rounded,
        r2(doc.net_total()),
        "0.005 × 3 must expose the BR-CO-10 break that motivates leaf rounding"
    );

    // BR-CO-15: BT-112 = BT-109 + BT-110.
    let doc = BillingDocument::from_positions(
        meta("BR-CO-15"),
        vec![
            LineItem::fixed("x", Amount::parse("0.00420").unwrap())
                .build()
                .unwrap(),
        ],
        vec![FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed()],
        vec![],
    )
    .unwrap();
    assert_ne!(
        r2(doc.net_total()) + r2(doc.tax_total()),
        r2(doc.gross_total()),
        "0.0042 at 19% must expose the BR-CO-15 break"
    );
}

/// Assert every EN 16931 rule that constrains amounts, for one document.
///
/// `scale` is the policy the document was assembled under; BR-CO-17 is checked
/// against a **single** rounding of the exact product, which is how a validator
/// recomputes it.
fn assert_amounts_conform(doc: &BillingDocument, scale: AmountScale) {
    let d = scale.decimals();
    // BR-DEC-09/12/13/14/16/17/18/19/20/23 — nothing carries more decimals than the
    // format allows.
    assert!(
        doc.fits_amount_scale(d),
        "BR-DEC violation: {:?}",
        doc.amount_scale_violation(d)
    );
    // BR-CO-10: the positions sum to the net total.
    let sum: Amount<5> = doc
        .net_positions()
        .iter()
        .chain(doc.discount_positions())
        .map(|p| p.net_amount)
        .sum();
    assert_eq!(sum, doc.net_total(), "BR-CO-10/13");
    // BR-CO-14: the VAT total is the sum of the per-category tax amounts. Non-VAT
    // layers add to tax_total without a breakdown entry, so this holds as equality
    // only when every layer is a VAT layer — the caller's own check.
    // BR-CO-15: gross = net + tax.
    assert_eq!(
        doc.net_total() + doc.tax_total(),
        doc.gross_total(),
        "BR-CO-15"
    );
    // BR-CO-16: amount due = gross - prepaid + rounding.
    assert_eq!(
        doc.gross_total() - doc.prepaid() + doc.rounding(),
        doc.amount_due().unwrap(),
        "BR-CO-16"
    );
    // BR-CO-17: each category's tax = base × rate, in ONE rounding of the exact
    // product. Rounding an already-rounded tax answers a different question and can
    // differ by a whole minor unit, so this recomputes the way a validator does.
    for e in doc.tax_breakdown() {
        let expected = scale
            .apply_decimal(e.taxable_base.into_decimal() * e.rate)
            .unwrap();
        assert_eq!(e.tax_amount, expected, "BR-CO-17 for rate {}", e.rate);
    }
    // BR-CO-14: the declared VAT total is the sum of the per-category tax amounts.
    // Equality holds when every layer is a VAT layer; non-VAT layers (a commission,
    // a per-unit excise) add to `tax_total` without a breakdown entry, so the
    // breakdown can only ever be a component.
    let breakdown_tax: Amount<5> = doc.tax_breakdown().iter().map(|e| e.tax_amount).sum();
    assert!(
        breakdown_tax.abs() <= doc.tax_total().abs(),
        "BR-CO-14: breakdown {breakdown_tax} exceeds declared tax {}",
        doc.tax_total()
    );
    // BT-131 = BT-129 × BT-146 for every position derived from a quantity and a
    // unit price. EN 16931 validators check this per line (at warning severity), and
    // it only holds if the reduction rounded the exact product rather than an
    // already-rounded amount.
    for p in doc.all_positions() {
        let (Some(q), Some(up)) = (p.quantity.as_ref(), p.unit_price.as_ref()) else {
            continue;
        };
        let mut expected = scale.apply_decimal(q.value * up.value).unwrap();
        if p.is_credit() && expected.is_positive() {
            expected = expected.checked_neg().unwrap();
        }
        assert_eq!(
            p.net_amount, expected,
            "BT-131 = quantity × unit_price for {:?}",
            p.description
        );
    }
    // And the document's own eleven invariants still hold.
    doc.assert_valid();
}

/// The scaled construction satisfies every amount rule across a wide input sweep —
/// including VAT rates with four decimals, where naive double rounding would drift.
#[test]
fn amount_scale_satisfies_en16931_across_a_sweep() {
    // Rates with 2, 3 and 4 decimals. The 4-decimal ones (a real US sales-tax
    // shape) are the case where rounding a 5-decimal intermediate a second time
    // could land a minor unit away from what a validator recomputes for BR-CO-17.
    // `dec!(0)` is deliberately absent: a standard-rated 0 % layer is not a lawful
    // EN 16931 state (BR-S-05), and `FixedRateTax::zero_rated` is the constructor
    // for a zero-rated supply. The sweep is about rounding, and 0 % rounds nothing.
    let rates = [
        dec!(0.19),
        dec!(0.07),
        dec!(0.081),
        dec!(0.0825),
        dec!(0.0625),
    ];
    let mut checked = 0;
    for rate in rates {
        // Sweep raw net amounts through every 10⁻⁵ residue class, so each rounding
        // boundary (including exact midpoints) is hit.
        for raw in (1..4_000i64).chain([49_999, 50_000, 50_001, 149_999, 150_000, 150_001]) {
            let doc = BillingDocument::builder()
                .meta(meta("SWEEP"))
                .amount_scale(AmountScale::EN16931)
                .positions(vec![
                    LineItem::fixed("x", Amount::<5>::from_raw_units(raw))
                        .build()
                        .unwrap(),
                ])
                .extra_tax(FixedRateTax::new("VAT", rate).unwrap().boxed())
                .build()
                .unwrap();
            assert_amounts_conform(&doc, AmountScale::EN16931);
            checked += 1;
        }
    }
    assert!(checked > 20_000, "sweep should be broad, ran {checked}");
}

/// The realistic metered line: five decimals in, two decimals out, still valid.
#[test]
fn amount_scale_makes_a_metered_invoice_emittable() {
    let positions = || {
        vec![
            // 1234.567 kWh × 0.28901 EUR/kWh = 356.80221 — five decimals.
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1234.567), "kWh"),
                UnitPrice::new(dec!(0.28901), "EUR/kWh"),
            )
            .build()
            .unwrap(),
            LineItem::fixed("Grundpreis", Amount::parse("8.50000").unwrap())
                .build()
                .unwrap(),
        ]
    };

    // Without the policy the document is arithmetically correct but not emittable.
    let raw = BillingDocument::from_positions(
        meta("RAW"),
        positions(),
        vec![FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed()],
        vec![],
    )
    .unwrap();
    raw.assert_valid();
    assert!(!raw.fits_amount_scale(2));
    let (what, value) = raw.amount_scale_violation(2).unwrap();
    assert!(what.contains("position[0]"), "{what}");
    assert_eq!(value, Amount::parse("356.80221").unwrap());

    // With it, every amount fits and every identity still holds.
    let scaled = BillingDocument::builder()
        .meta(meta("SCALED"))
        .amount_scale(AmountScale::EN16931)
        .positions(positions())
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    assert_amounts_conform(&scaled, AmountScale::EN16931);
    assert_eq!(
        scaled.net_positions()[0].net_amount,
        Amount::parse("356.80000").unwrap()
    );
    assert_eq!(scaled.net_total(), Amount::parse("365.30000").unwrap());
    assert_eq!(scaled.tax_total(), Amount::parse("69.41000").unwrap());
    assert_eq!(scaled.gross_total(), Amount::parse("434.71000").unwrap());
}

/// A full stack — discounts, a per-unit levy, compound VAT, a mixed-rate breakdown
/// and a prepayment — all reduced consistently.
#[test]
fn amount_scale_holds_for_a_full_document_stack() {
    let doc = BillingDocument::builder()
        .meta(meta("STACK"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(3333.333), "kWh"),
                UnitPrice::new(dec!(0.28901), "EUR/kWh"),
            )
            .tag("standard")
            .build()
            .unwrap(),
            LineItem::fixed("Reduziert", Amount::parse("77.77700").unwrap())
                .tag("reduced")
                .build()
                .unwrap(),
        ])
        .extra_discount(
            PercentageDiscount::new("Treuerabatt", dec!(0.033))
                .unwrap()
                .boxed(),
        )
        .extra_tax(
            PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh")
                .unwrap()
                .with_currency(Currency::EUR)
                .boxed(),
        )
        .extra_tax(
            FixedRateTax::new("MwSt 19%", dec!(0.19))
                .unwrap()
                .with_tag("standard")
                .boxed(),
        )
        .extra_tax(
            FixedRateTax::new("MwSt 7%", dec!(0.07))
                .unwrap()
                .with_tag("reduced")
                .boxed(),
        )
        .build()
        .unwrap()
        .with_prepaid(Amount::parse("100.00000").unwrap())
        .unwrap();

    assert_amounts_conform(&doc, AmountScale::EN16931);
    assert_eq!(doc.tax_breakdown().len(), 2, "one entry per rate (BR-S-08)");
}

/// A scale of 0 (whole units — JPY, KRW) works the same way.
#[test]
fn amount_scale_supports_zero_decimal_currencies() {
    let doc = BillingDocument::builder()
        .meta(DocumentMeta {
            currency: Currency::JPY,
            ..meta("JPY")
        })
        .amount_scale(AmountScale::new(0, RoundingStrategy::MidpointAwayFromZero).unwrap())
        .positions(vec![
            LineItem::fixed("Item", Amount::parse("1234.56700").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("JCT", dec!(0.10)).unwrap().boxed())
        .build()
        .unwrap();
    assert!(doc.fits_amount_scale(0));
    assert_eq!(doc.net_total(), Amount::parse("1235.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("124.00000").unwrap());
    assert_eq!(doc.gross_total(), Amount::parse("1359.00000").unwrap());
    doc.assert_valid();
}

/// The scale policy refuses a precision it cannot deliver.
#[test]
fn amount_scale_rejects_a_scale_beyond_line_item_precision() {
    assert!(AmountScale::new(6, RoundingStrategy::MidpointAwayFromZero).is_err());
    assert!(AmountScale::new(5, RoundingStrategy::MidpointAwayFromZero).is_ok());
    assert_eq!(AmountScale::EN16931.decimals(), 2);
}

/// Regression: the declared VAT total and the VAT breakdown must never disagree.
///
/// `TaxLayer::compute` rounds its product to the engine's five decimals with
/// commercial rounding before the document ever sees it. Reducing *that* to the
/// reporting scale rounds twice, by a different route than BR-CO-17's single
/// rounding of the exact product — so with any strategy other than
/// half-away-from-zero, or any rate whose product exceeds five decimals, the
/// charged VAT and the reported VAT drifted apart. `0.10 × 0.0999999` is the
/// smallest case: `0.01` charged against `0.00` declared, violating BR-CO-14.
#[test]
fn declared_vat_and_vat_breakdown_agree_for_every_strategy() {
    let strategies = [
        RoundingStrategy::MidpointAwayFromZero,
        RoundingStrategy::MidpointToEven,
        RoundingStrategy::Ceiling,
        RoundingStrategy::Floor,
        RoundingStrategy::Truncate,
    ];
    // Rates whose product with a two-decimal base exceeds the engine's five
    // decimals — where the second rounding can move the result.
    let rates = [
        dec!(0.19),
        dec!(0.0825),
        dec!(0.06125),
        dec!(0.190625),
        dec!(0.0999999),
        dec!(0.123456),
    ];
    let mut checked = 0usize;
    for strategy in strategies {
        let scale = AmountScale::new(2, strategy).unwrap();
        for rate in rates {
            for cents in 1..600i64 {
                let doc = BillingDocument::builder()
                    .meta(meta("BR-CO-14"))
                    .amount_scale(scale)
                    .positions(vec![
                        LineItem::fixed("x", Amount::<5>::from_raw_units(cents * 1_000))
                            .build()
                            .unwrap(),
                    ])
                    .extra_tax(FixedRateTax::new("VAT", rate).unwrap().boxed())
                    .build()
                    .unwrap();

                // The whole point: one VAT layer, so BR-CO-14 is an equality.
                let breakdown_tax: Amount<5> =
                    doc.tax_breakdown().iter().map(|e| e.tax_amount).sum();
                assert_eq!(
                    breakdown_tax,
                    doc.tax_total(),
                    "BR-CO-14 with strategy {strategy:?}, rate {rate}, base {}",
                    doc.net_total()
                );
                assert_amounts_conform(&doc, scale);
                checked += 1;
            }
        }
    }
    assert!(checked >= 17_000, "ran only {checked} cases");
}

/// Multi-line documents, pseudo-random quantities and prices, every strategy.
///
/// The single-line sweeps pin the rounding boundaries; this pins the *aggregation* —
/// that many independently reduced lines still sum to totals which satisfy every
/// identity. Deterministic (xorshift from a fixed seed), so a failure reproduces.
#[test]
fn amount_scale_holds_for_random_multi_line_documents() {
    let strategies = [
        RoundingStrategy::MidpointAwayFromZero,
        RoundingStrategy::MidpointToEven,
        RoundingStrategy::Ceiling,
        RoundingStrategy::Floor,
        RoundingStrategy::Truncate,
    ];
    let rates = [dec!(0.19), dec!(0.0825), dec!(0.190625), dec!(0.0999999)];

    let mut seed: u64 = 0x243F_6A88_85A3_08D3;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    let mut cases = 0usize;
    for strategy in strategies {
        let scale = AmountScale::new(2, strategy).unwrap();
        for rate in rates {
            for _ in 0..300 {
                let lines = (next() % 5 + 1) as usize;
                let positions: Vec<LineItem> = (0..lines)
                    .map(|i| {
                        let qty = rust_decimal::Decimal::new((next() % 2_000_000) as i64, 3);
                        let price = rust_decimal::Decimal::new((next() % 1_000_000) as i64, 6);
                        LineItem::for_usage(
                            format!("line {i}"),
                            Quantity::new(qty, "kWh"),
                            UnitPrice::new(price, "EUR/kWh"),
                        )
                        .build()
                        .unwrap()
                    })
                    .collect();

                let doc = BillingDocument::builder()
                    .meta(meta("RANDOM"))
                    .amount_scale(scale)
                    .positions(positions)
                    .extra_tax(FixedRateTax::new("VAT", rate).unwrap().boxed())
                    .build()
                    .unwrap();

                assert_amounts_conform(&doc, scale);
                // One VAT layer and no other tax, so BR-CO-14 is an equality here.
                let breakdown_tax: Amount<5> =
                    doc.tax_breakdown().iter().map(|e| e.tax_amount).sum();
                assert_eq!(breakdown_tax, doc.tax_total(), "BR-CO-14");
                cases += 1;
            }
        }
    }
    assert_eq!(cases, 6_000);
}

/// The exact case that exposed the divergence, pinned on its own.
#[test]
fn br_co_14_regression_floor_strategy_with_a_seven_decimal_rate() {
    let scale = AmountScale::new(2, RoundingStrategy::Floor).unwrap();
    let doc = BillingDocument::builder()
        .meta(meta("REG"))
        .amount_scale(scale)
        .positions(vec![
            LineItem::fixed("x", Amount::parse("0.10000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("VAT", dec!(0.0999999)).unwrap().boxed())
        .build()
        .unwrap();

    // 0.10 × 0.0999999 = 0.00999999. Floored to two decimals that is 0.00 —
    // one rounding of the exact product, which is what BR-CO-17 specifies.
    // Rounding to five decimals first gave 0.01000, then floored to 0.01.
    assert_eq!(doc.tax_breakdown()[0].tax_amount, Amount::<5>::ZERO);
    assert_eq!(doc.tax_total(), Amount::<5>::ZERO);
    doc.assert_valid();
}

/// A metered line is reduced from its **exact** product, not from the already-
/// rounded stored amount.
///
/// `LineItemBuilder::build` rounds `quantity × unit_price` to the engine's five
/// decimals. Reducing that a second time rounds twice and can move the line a whole
/// cent, which also pushes it off `BT-131 = BT-129 × BT-146` — a rule EN 16931
/// validators check on every line.
#[test]
fn a_metered_line_is_reduced_from_the_exact_product() {
    // 0.0999999 EUR/kWh × 0.1 kWh = 0.00999999.
    //   Engine value (5 dp, commercial): 0.01000  -> floored to 2 dp: 0.01
    //   Exact product floored to 2 dp:   0.00     <- correct, one rounding
    let doc = BillingDocument::builder()
        .meta(meta("LINE"))
        .amount_scale(AmountScale::new(2, RoundingStrategy::Floor).unwrap())
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(0.1), "kWh"),
                UnitPrice::new(dec!(0.0999999), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .build()
        .unwrap();
    assert_eq!(doc.net_positions()[0].net_amount, Amount::<5>::ZERO);
    assert_eq!(doc.net_total(), Amount::<5>::ZERO);
    doc.assert_valid();

    // The credit counterpart keeps its sign.
    let credit = BillingDocument::builder()
        .meta(meta("LINE-C"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::credit_for_usage(
                "Einspeisung",
                Quantity::new(dec!(1234.567), "kWh"),
                UnitPrice::new(dec!(0.08110), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .build()
        .unwrap();
    // 1234.567 × 0.0811 = 100.1233837 -> 100.12, credited.
    assert_eq!(
        credit.net_positions()[0].net_amount,
        Amount::parse("-100.12000").unwrap()
    );
    assert!(credit.net_positions()[0].is_credit());
    credit.assert_valid();
}

/// A stated `fixed_amount` is authoritative even when a quantity is also present —
/// it is reduced verbatim, never recomputed from `quantity × unit_price`.
#[test]
fn a_stated_fixed_amount_is_never_recomputed_from_the_quantity() {
    let stated = LineItem::debit("Pauschale mit Mengenangabe")
        .quantity(Quantity::new(dec!(1000), "kWh"))
        .unit_price(UnitPrice::new(dec!(0.30), "EUR/kWh"))
        // Deliberately NOT 1000 × 0.30 = 300.
        .fixed_amount(Amount::parse("123.45678").unwrap())
        .build()
        .unwrap();
    assert_eq!(stated.net_amount, Amount::parse("123.45678").unwrap());

    let doc = BillingDocument::builder()
        .meta(meta("FIXED"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![stated])
        .build()
        .unwrap();
    // The stated amount, reduced — not 300.00.
    assert_eq!(
        doc.net_positions()[0].net_amount,
        Amount::parse("123.46000").unwrap()
    );
    doc.assert_valid();
}

/// Reversal preserves the scale; **allocation does not**, and cannot.
///
/// Splitting 100.00 three ways is 33.333… — there is no two-decimal answer, so an
/// allocated document has to be re-assembled (or re-checked) before it is emitted.
/// This is pinned as a test rather than left to be discovered downstream.
#[test]
fn allocation_breaks_the_amount_scale_but_reversal_keeps_it() {
    let doc = BillingDocument::builder()
        .meta(meta("SPLIT"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("x", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("V", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    assert!(doc.fits_amount_scale(2));

    // A credit note of a two-decimal invoice is still two decimals.
    let reversed = doc.reverse(meta("STORNO")).unwrap();
    assert!(reversed.fits_amount_scale(2));
    assert_amounts_conform(&reversed, AmountScale::EN16931);

    // A three-way split of 100.00 is not.
    let parts = EqualAllocation::new(3).unwrap().allocate(&doc).unwrap();
    assert!(
        parts.iter().any(|p| !p.fits_amount_scale(2)),
        "an equal three-way split cannot land on two decimals"
    );
    // The allocation is still exact — it is the *precision*, not the sum, that gives.
    let total: Amount<5> = parts.iter().map(|p| p.gross_total()).sum();
    assert_eq!(total, doc.gross_total());
    for p in &parts {
        p.assert_valid();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The VAT / charge split (EN 16931 BT-110 vs BG-21)
// ─────────────────────────────────────────────────────────────────────────────

/// `tax_total` mixes value added tax with document level charges. Mapping all of
/// it to BT-110 breaks BR-CO-14 (`BT-110 = Σ BT-117`) on every document carrying a
/// levy — which is the whole German electricity stack.
#[test]
fn vat_total_and_charge_total_split_bt_110_from_bg_21() {
    let doc = BillingDocument::builder()
        .meta(meta("SPLIT-1"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1000), "kWh").with_code("KWH"),
                UnitPrice::new(dec!(0.30), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        // Two document level charges, then VAT on net + both.
        .extra_tax(
            PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh")
                .unwrap()
                .with_unit_code("KWH")
                .with_reason_code("AAE")
                .boxed(),
        )
        .extra_tax(
            PercentageCharge::new("Abrechnungsentgelt", dec!(0.01))
                .unwrap()
                .with_reason_code("ABK")
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // Exactly one VAT position, two charges.
    assert_eq!(doc.vat_positions().count(), 1);
    assert_eq!(doc.charge_positions().count(), 2);
    assert_eq!(doc.tax_positions().len(), 3);

    // BR-CO-14: BT-110 is the VAT alone, and equals Σ BT-117.
    let bt_110 = doc.vat_total().unwrap();
    let sum_bt_117 = Amount::checked_sum(doc.tax_breakdown().iter().map(|e| e.tax_amount)).unwrap();
    assert_eq!(bt_110, sum_bt_117, "BR-CO-14");
    assert_ne!(bt_110, doc.tax_total(), "the naive mapping would differ");

    // The charges account for the rest, exactly.
    assert_eq!(
        doc.vat_total()
            .unwrap()
            .checked_add(doc.charge_total().unwrap())
            .unwrap(),
        doc.tax_total(),
    );

    // Every charge carries the BT-102/BT-103 the VAT layer attributed to it, plus
    // the BT-105 its layer declared — both mandatory-ish under BR-37 / BR-38.
    for charge in doc.charge_positions() {
        let vat = charge.vat.expect("BT-102 on every BG-21 charge");
        assert_eq!(vat.category, TaxCategory::Standard);
        assert_eq!(vat.rate, dec!(0.19));
        assert!(
            charge
                .allowance_charge
                .as_ref()
                .and_then(|a| a.reason_code.as_deref())
                .is_some(),
            "BT-105"
        );
    }
    // The VAT position itself is BG-23 and carries no BT-102.
    assert!(doc.vat_positions().all(|p| p.vat.is_none()));

    // BT-130 survives assembly on both the metered line and the levy.
    assert_eq!(
        doc.net_positions()[0]
            .quantity
            .as_ref()
            .unwrap()
            .code
            .as_deref(),
        Some("KWH")
    );
    let levy = doc.charge_positions().next().unwrap();
    assert_eq!(levy.quantity.as_ref().unwrap().code.as_deref(), Some("KWH"));

    // BR-S-08: the breakdown agrees with the per-position attribution.
    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

/// A third-party `TaxLayer` is classified by what it returns from `breakdown`,
/// not by tags it happened to write — so the split is total, not a heuristic.
#[test]
fn vat_tag_is_engine_assigned_so_custom_layers_classify_correctly() {
    struct CustomVat;
    impl TaxLayer for CustomVat {
        fn name(&self) -> &str {
            "Custom VAT"
        }
        fn compute(&self, positions: &[LineItem]) -> Result<LineItem, BillingError> {
            let base = Amount::checked_sum(positions.iter().map(|p| p.net_amount))?;
            // Deliberately writes no reserved tag of its own.
            LineItem::debit("Custom VAT")
                .fixed_amount(base.checked_mul_qty(dec!(0.19))?)
                .build()
        }
        fn breakdown(
            &self,
            positions: &[LineItem],
        ) -> Result<Option<TaxBreakdownEntry>, BillingError> {
            let base = Amount::checked_sum(positions.iter().map(|p| p.net_amount))?;
            Ok(Some(TaxBreakdownEntry::new(
                TaxCategory::Standard,
                dec!(0.19),
                base,
                base.checked_mul_qty(dec!(0.19))?,
            )))
        }
    }

    struct CustomCharge;
    impl TaxLayer for CustomCharge {
        fn name(&self) -> &str {
            "Handling"
        }
        fn compute(&self, _: &[LineItem]) -> Result<LineItem, BillingError> {
            LineItem::debit("Handling")
                .fixed_amount(Amount::parse("5.00000").unwrap())
                .build()
        }
    }

    let doc = BillingDocument::builder()
        .meta(meta("CUSTOM-1"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(Box::new(CustomCharge))
        .extra_tax(Box::new(CustomVat))
        .build()
        .unwrap();

    // The engine tagged both, from the `breakdown` return value alone.
    assert_eq!(doc.charge_positions().count(), 1);
    assert_eq!(doc.vat_positions().count(), 1);
    assert!(
        doc.tax_positions()
            .iter()
            .all(|p| p.has_tag(billing::tags::TAX))
    );
    // VAT on 100 + 5 = 105 → 19.95.
    assert_eq!(doc.vat_total().unwrap(), Amount::parse("19.95000").unwrap());
    assert_eq!(
        doc.charge_total().unwrap(),
        Amount::parse("5.00000").unwrap()
    );
    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();

    // `CustomVat` overrides neither `covers` nor `allowance_charge`, so this is
    // also the test of what those defaults do. `covers` defaults to `true` —
    // "this layer taxes everything it is given" — and that is what puts BT-151 /
    // BT-152 on each taxable position. A default of `false` would attribute
    // nothing, leaving BR-CO-04 unsatisfied on every line.
    let line = &doc.net_positions()[0];
    let attribution = line.vat.as_ref().expect("BT-151 / BT-152 on the line");
    assert_eq!(attribution.category, TaxCategory::Standard);
    assert_eq!(attribution.rate, dec!(0.19));

    // `CustomCharge` overrides neither, and its `allowance_charge` default of
    // `None` is load-bearing: an empty `AllowanceCharge` conjured in its place
    // would state a BG-21 with no reason at all, which BR-CO-21 forbids.
    let charge = doc.charge_positions().next().unwrap();
    assert!(charge.allowance_charge.is_none());
    // The charge is inside the VAT base, so it carries the covering layer's pair.
    assert_eq!(
        charge.vat.as_ref().map(|v| v.rate),
        Some(dec!(0.19)),
        "BT-102 / BT-103 derived from the layer that covers it"
    );
}

/// A layer may declare its BG-21 detail at the layer level rather than filling it
/// in `compute`, and assembly attaches it — but only to charges, never to VAT.
#[test]
fn assembly_backfills_charge_detail_onto_charges_and_not_onto_vat() {
    use billing::{AllowanceCharge, LineVat};

    // Unlike `PercentageCharge`, this layer computes a flat amount and has nothing
    // it needs to know about the base — so it states BT-102 / BT-105 once, on the
    // layer, and leaves the position it returns bare. That fallback is what the
    // `TaxLayer::allowance_charge` and `TaxLayer::vat` docs promise.
    struct DeclaredCharge;
    impl TaxLayer for DeclaredCharge {
        fn name(&self) -> &str {
            "Handling"
        }
        fn compute(&self, _: &[LineItem]) -> Result<LineItem, BillingError> {
            // Carries neither `allowance_charge` nor `vat` of its own.
            LineItem::debit("Handling")
                .fixed_amount(Amount::parse("5.00000").unwrap())
                .build()
        }
        fn allowance_charge(&self) -> Option<AllowanceCharge> {
            Some(AllowanceCharge {
                reason_code: Some("ABK".into()),
                base_amount: None,
                percentage: None,
            })
        }
        fn vat(&self) -> Option<LineVat> {
            // BR-37: a charge needs its own category and rate. It must agree with
            // the VAT layer that covers it, or assembly reports a LayerError.
            Some(LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap())
        }
    }

    let doc = BillingDocument::builder()
        .meta(meta("BG21-FALLBACK"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(Box::new(DeclaredCharge))
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // The charge picked up what the layer declared.
    let charge = doc.charge_positions().next().expect("one charge");
    assert_eq!(charge.net_amount, Amount::parse("5.00000").unwrap());
    let detail = charge.allowance_charge.as_ref().expect("BG-21 detail");
    assert_eq!(detail.reason_code.as_deref(), Some("ABK")); // BT-105
    let vat = charge.vat.as_ref().expect("BT-102 / BT-103");
    assert_eq!(vat.category, TaxCategory::Standard);
    assert_eq!(vat.rate, dec!(0.19));

    // The VAT position is BG-23 and carries neither: a charge sits *inside* the
    // taxable base, VAT is the tax *on* that base. Attaching BT-102 to it would
    // report the VAT as a second charge and double the document's BT-108.
    let vat_position = doc.vat_positions().next().expect("one VAT position");
    assert!(vat_position.allowance_charge.is_none());
    assert!(vat_position.vat.is_none());

    // VAT is on 100 + 5, and the charge total counts the charge once.
    assert_eq!(doc.vat_total().unwrap(), Amount::parse("19.95000").unwrap());
    assert_eq!(
        doc.charge_total().unwrap(),
        Amount::parse("5.00000").unwrap()
    );
    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

/// A `DiscountLayer` that overrides neither `vat` nor `allowance_charge` gets
/// neither invented for it — the defaults are `None`, and `None` is load-bearing.
#[test]
fn a_discount_layer_using_the_defaults_gets_no_bg_20_detail_invented() {
    use billing::DiscountLayer;

    // The mirror of `CustomCharge` on the allowance side. Both defaults matter for
    // the same reason: an `AllowanceCharge` conjured out of `Default` states a
    // BG-20 with no reason code and no reason text, which BR-CO-21 forbids —
    // and it would do so on a document whose every amount is right, so nothing
    // arithmetic would notice.
    struct BareDiscount;
    impl DiscountLayer for BareDiscount {
        fn name(&self) -> &str {
            "Kulanz"
        }
        fn compute(&self, _: &[LineItem]) -> Result<LineItem, BillingError> {
            LineItem::credit("Kulanz")
                .fixed_amount(Amount::parse("10.00000").unwrap())
                .build()
        }
    }

    let doc = BillingDocument::builder()
        .meta(meta("BG20-DEFAULT"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(Box::new(BareDiscount))
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    let allowance = &doc.discount_positions()[0];
    assert_eq!(allowance.net_amount, Amount::parse("-10.00000").unwrap());
    assert!(
        allowance.allowance_charge.is_none(),
        "no BG-20 detail may be invented for a layer that declared none"
    );
    // BT-95 / BT-96 still arrive, but from the VAT layer that covers the
    // allowance rather than from the discount layer's own (absent) declaration.
    assert_eq!(
        allowance.vat.as_ref().map(|v| v.rate),
        Some(dec!(0.19)),
        "BR-32 attribution derived from the covering layer"
    );

    // 100 − 10 = 90 taxable, 19 % of 90 = 17.10.
    assert_eq!(doc.net_total(), Amount::parse("90.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("17.10000").unwrap());
    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

/// BR-CO-18, as actually written: an invoice that charges VAT must have a BG-23.
#[test]
fn br_co_18_requires_a_breakdown_behind_declared_vat() {
    let doc = BillingDocument::builder()
        .meta(meta("CO18"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    doc.assert_valid();

    // A document with only charges and no VAT is fine — `billing` bills things
    // that are not EN 16931 invoices, and BR-CO-18 cannot be demanded blindly.
    let charges_only = BillingDocument::builder()
        .meta(meta("CO18-B"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(PercentageCharge::new("Fee", dec!(0.05)).unwrap().boxed())
        .build()
        .unwrap();
    assert!(charges_only.tax_breakdown().is_empty());
    charges_only.assert_valid();

    // Stripping the breakdown from a document that charges VAT is what BR-CO-18
    // catches. Round-tripping through JSON is the realistic way to reach it.
    #[cfg(feature = "serde")]
    {
        let mut json: serde_json::Value = serde_json::to_value(&doc).unwrap();
        json["tax_breakdown"] = serde_json::json!([]);
        let err = serde_json::from_value::<BillingDocument>(json).unwrap_err();
        assert!(err.to_string().contains("BR-CO-18"), "unhelpful: {err}");
    }
}

/// A `B` (split payment) invoice: taxed at the normal rate, and the tax is stated.
#[test]
fn split_payment_invoice_states_its_tax() {
    let doc = BillingDocument::builder()
        .meta(meta("IT-SPLIT-1"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("Consulenza", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(
            FixedRateTax::new("IVA", dec!(0.22))
                .unwrap()
                .with_category(TaxCategory::SplitPayment)
                .boxed(),
        )
        .build()
        .unwrap();

    let entry = &doc.tax_breakdown()[0];
    assert_eq!(entry.category, TaxCategory::SplitPayment);
    assert_eq!(entry.category.code(), "B");
    // Unlike `AE`, BT-117 is NOT zero — there is no BR-B-09.
    assert_eq!(entry.tax_amount, Amount::parse("220.00000").unwrap());
    assert_eq!(
        doc.vat_total().unwrap(),
        Amount::parse("220.00000").unwrap()
    );
    // And no exemption reason is required or forbidden.
    assert!(entry.exemption_reason.is_none());
    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

/// Allowances carry their own BT-95 / BT-96 / BT-98, and reduce the base they name.
#[test]
fn allowances_carry_vat_and_reason_codes() {
    let doc = BillingDocument::builder()
        .meta(meta("ALLOW-1"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("Beratung", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            billing::PercentageDiscount::new("Treuerabatt", dec!(0.10))
                .unwrap()
                // UNCL 5189 "95" = Discount.
                .with_reason_code("95")
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    let allowance = &doc.discount_positions()[0];
    let ac = allowance.allowance_charge.as_ref().expect("BG-20 detail");
    assert_eq!(ac.reason_code.as_deref(), Some("95"), "BT-98");
    // BT-93 / BT-94 travel together (PEPPOL-EN16931-R041 / R042).
    assert_eq!(ac.base_amount, Some(Amount::parse("1000.00000").unwrap()));
    assert_eq!(ac.percentage, Some(dec!(10)));
    // The VAT layer covered it, so it was attributed BT-95 / BT-96.
    let vat = allowance.vat.expect("BT-95 on every BG-20 allowance");
    assert_eq!(vat.category, TaxCategory::Standard);
    assert_eq!(vat.rate, dec!(0.19));

    // BR-S-08: 1000.00 − 100.00 = 900.00 is the taxable base.
    assert_eq!(
        doc.tax_breakdown()[0].taxable_base,
        Amount::parse("900.00000").unwrap()
    );
    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

/// A caller-declared BT-151 that contradicts the layer actually taxing the
/// position is a tagging bug, and is reported rather than silently overridden.
#[test]
fn contradictory_declared_vat_is_rejected() {
    let err = BillingDocument::builder()
        .meta(meta("CONTRA"))
        .positions(vec![
            LineItem::fixed("Buch", Amount::parse("100.00000").unwrap())
                // The caller says 7 % …
                .vat(LineVat::new(TaxCategory::Standard, dec!(0.07)).unwrap())
                .build()
                .unwrap(),
        ])
        // … while the layer that taxes it says 19 %.
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap_err();
    assert!(
        matches!(err, BillingError::LayerError { .. }),
        "got {err:?}"
    );
    assert!(err.to_string().contains("Buch"));
}

/// `exact_to` narrows without rounding, or says it cannot — the conversion an
/// interchange boundary needs, where rounding would break the totals identities.
#[test]
fn exact_to_refuses_to_lose_money_at_the_boundary() {
    let doc = BillingDocument::builder()
        .meta(meta("EXACT"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1234.567), "kWh"),
                UnitPrice::new(dec!(0.28901), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // Every amount narrows exactly, so the EN 16931 identities survive the move.
    let net: Amount<2> = doc.net_total().exact_to().unwrap();
    let tax: Amount<2> = doc.tax_total().exact_to().unwrap();
    let gross: Amount<2> = doc.gross_total().exact_to().unwrap();
    assert_eq!(net.checked_add(tax).unwrap(), gross, "BR-CO-15 at 2 dp");

    // An unscaled document does not narrow, and says so instead of rounding.
    let raw = BillingDocument::builder()
        .meta(meta("EXACT-RAW"))
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1234.567), "kWh"),
                UnitPrice::new(dec!(0.28901), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .build()
        .unwrap();
    assert!(!raw.fits_amount_scale(2));
    assert!(raw.net_total().exact_to::<2>().is_err());
}

/// A credit note keeps the attribution and the tags, so it maps to EN 16931 the
/// same way the invoice did — and BR-S-08 still holds with every sign flipped.
#[test]
fn reversal_preserves_vat_attribution_and_classification() {
    let doc = BillingDocument::builder()
        .meta(meta("INV-REV"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1000), "kWh").with_code("KWH"),
                UnitPrice::new(dec!(0.30), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .extra_tax(
            PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh")
                .unwrap()
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    doc.verify_vat_attribution().unwrap();

    let credit = doc.reverse(meta("CN-REV")).unwrap();
    assert_eq!(credit.vat_positions().count(), 1);
    assert_eq!(credit.charge_positions().count(), 1);
    assert_eq!(
        credit.vat_total().unwrap(),
        doc.vat_total().unwrap().checked_neg().unwrap()
    );
    assert_eq!(
        credit.net_positions()[0].vat.unwrap().rate,
        dec!(0.19),
        "BT-152 survives the reversal"
    );
    assert_eq!(
        credit.net_positions()[0]
            .quantity
            .as_ref()
            .unwrap()
            .code
            .as_deref(),
        Some("KWH"),
        "BT-130 survives the reversal"
    );
    // Base and tax are both negated, so the identity is unchanged.
    credit.verify_vat_attribution().unwrap();
    credit.assert_valid();
}

/// Everything added for the EN 16931 mapping survives a serde round trip, and the
/// re-validation on the way in still runs.
#[cfg(feature = "serde")]
#[test]
fn en16931_attribution_survives_a_serde_round_trip() {
    let doc = BillingDocument::builder()
        .meta(meta("SERDE-1"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1000), "kWh").with_code("KWH"),
                UnitPrice::new(dec!(0.30), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .extra_discount(
            billing::FixedDiscount::new("Gutschrift", Amount::parse("10.00000").unwrap())
                .unwrap()
                .with_reason_code("95")
                .boxed(),
        )
        .extra_tax(
            FixedRateTax::new("IVA", dec!(0.22))
                .unwrap()
                .with_category(TaxCategory::SplitPayment)
                .boxed(),
        )
        .build()
        .unwrap();

    let json = serde_json::to_string(&doc).unwrap();
    let back: BillingDocument = serde_json::from_str(&json).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.tax_breakdown()[0].category, TaxCategory::SplitPayment);
    assert_eq!(
        back.net_positions()[0].vat.unwrap().category,
        TaxCategory::SplitPayment
    );
    assert_eq!(
        back.net_positions()[0]
            .quantity
            .as_ref()
            .unwrap()
            .code
            .as_deref(),
        Some("KWH")
    );
    assert_eq!(
        back.discount_positions()[0]
            .allowance_charge
            .as_ref()
            .and_then(|a| a.reason_code.as_deref()),
        Some("95")
    );
    back.verify_vat_attribution().unwrap();

    // A hand-edited line rate that contradicts its category is refused on the way
    // in, rather than trusted because it came from JSON.
    let mut bad: serde_json::Value = serde_json::from_str(&json).unwrap();
    bad["net_positions"][0]["vat"]["category"] = serde_json::json!("ReverseCharge");
    assert!(serde_json::from_value::<BillingDocument>(bad).is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// The totals chain: BT-106 / BT-107 / BT-108 / BT-109 (BR-CO-13)
// ─────────────────────────────────────────────────────────────────────────────

/// `net_total` is `BT-106 − BT-107`, which is **not** BT-109 once the document
/// carries a charge — and a levy-bearing utility invoice always does.
#[test]
fn taxable_total_is_bt_109_and_net_total_is_not() {
    let doc = BillingDocument::builder()
        .meta(meta("CO13"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1000), "kWh"),
                UnitPrice::new(dec!(0.30), "EUR/kWh"),
            )
            .build()
            .unwrap(),
            LineItem::fixed("Grundpreis", Amount::parse("120.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            billing::FixedDiscount::new("Neukundenbonus", Amount::parse("30.00000").unwrap())
                .unwrap()
                .boxed(),
        )
        .extra_tax(
            PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050").unwrap(), "kWh")
                .unwrap()
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    let bt_106 = doc.line_total().unwrap(); // 300.00 + 120.00
    let bt_107 = doc.discount_total(); // −30.00
    let bt_108 = doc.charge_total().unwrap(); // 20.50
    let bt_109 = doc.taxable_total().unwrap();
    let bt_110 = doc.vat_total().unwrap();
    let bt_112 = doc.gross_total();

    assert_eq!(bt_106, Amount::parse("420.00000").unwrap());
    assert_eq!(bt_107, Amount::parse("-30.00000").unwrap());
    assert_eq!(bt_108, Amount::parse("20.50000").unwrap());

    // BR-CO-13: BT-109 = BT-106 − BT-107 + BT-108.
    assert_eq!(
        bt_109,
        bt_106
            .checked_add(bt_107)
            .unwrap()
            .checked_add(bt_108)
            .unwrap()
    );
    // BR-CO-15: BT-112 = BT-109 + BT-110.
    assert_eq!(bt_112, bt_109.checked_add(bt_110).unwrap());

    // And the trap this exists to close: net_total is BT-106 − BT-107, so it
    // differs from BT-109 by exactly the charge.
    assert_eq!(doc.net_total(), Amount::parse("390.00000").unwrap());
    assert_ne!(doc.net_total(), bt_109);
    assert_eq!(bt_109.checked_sub(doc.net_total()).unwrap(), bt_108);

    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

// ─────────────────────────────────────────────────────────────────────────────
// Exemption reasons: BT-120 text vs BT-121 code
// ─────────────────────────────────────────────────────────────────────────────

/// BR-E-10 and friends accept a reason **code** as an alternative to the text, so
/// a caller holding only the VATEX code need not invent prose.
#[test]
fn exemption_reason_code_satisfies_the_requirement_on_its_own() {
    let doc = BillingDocument::builder()
        .meta(meta("VATEX-1"))
        .positions(vec![
            LineItem::fixed("Lieferung nach FR", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(
            FixedRateTax::exempt_coded(
                "Innergemeinschaftliche Lieferung",
                TaxCategory::IntraCommunity,
                "VATEX-EU-IC",
            )
            .unwrap()
            .boxed(),
        )
        .build()
        .unwrap();

    let entry = &doc.tax_breakdown()[0];
    assert_eq!(entry.exemption_reason_code.as_deref(), Some("VATEX-EU-IC"));
    assert_eq!(entry.exemption_reason, None);
    assert!(entry.has_exemption_reason());
    doc.assert_valid();

    // Neither form present is still a violation of BR-IC-10.
    let bare = TaxBreakdownEntry::new(
        TaxCategory::IntraCommunity,
        dec!(0),
        Amount::parse("1000.00000").unwrap(),
        Amount::ZERO,
    );
    let err = bare.validate().unwrap_err();
    assert!(err.to_string().contains("BT-120") && err.to_string().contains("BT-121"));

    // BR-S-10 / BR-Z-10 forbid *both*, so the code must be refused too — checking
    // only the text would have let this through.
    let coded_standard = TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("100.00000").unwrap(),
        Amount::parse("19.00000").unwrap(),
    )
    .with_exemption_reason_code("VATEX-EU-AE");
    assert!(
        coded_standard.validate().is_err(),
        "BR-S-10 forbids BT-121 too"
    );
    assert!(FixedRateTax::exempt_coded("Z", TaxCategory::ZeroRated, "VATEX-EU-O").is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Category O — outside the scope of VAT
// ─────────────────────────────────────────────────────────────────────────────

/// `O` is the only category that must not state a rate at all (BR-O-05/06/07),
/// and it must stand alone in the breakdown (BR-O-11 … BR-O-14).
#[test]
fn out_of_scope_stands_alone_and_states_no_rate() {
    // `states_rate` is the instruction a serialiser needs: suppress the element.
    assert!(!TaxCategory::OutOfScope.states_rate());
    assert!(TaxCategory::ReverseCharge.states_rate());
    assert!(TaxCategory::ZeroRated.states_rate());

    let doc = BillingDocument::builder()
        .meta(meta("O-1"))
        .positions(vec![
            LineItem::fixed("Schadenersatz", Amount::parse("500.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(
            FixedRateTax::exempt_coded("Nicht steuerbar", TaxCategory::OutOfScope, "VATEX-EU-O")
                .unwrap()
                .boxed(),
        )
        .build()
        .unwrap();
    doc.assert_valid();
    doc.verify_vat_attribution().unwrap();

    // BR-O-11: merging an `O` document with a taxed one produces a breakdown that
    // no validator accepts, and the engine now refuses to hand it back.
    let taxed = BillingDocument::builder()
        .meta(meta("O-2"))
        .positions(vec![
            LineItem::fixed("Beratung", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    let err = billing::merge_period_documents(doc, taxed).unwrap_err();
    assert!(err.to_string().contains("BR-O-11"), "unhelpful: {err}");
}

// ─────────────────────────────────────────────────────────────────────────────
// VAT on VAT
// ─────────────────────────────────────────────────────────────────────────────

/// A levy compounding into a VAT base is legitimate (it is a BG-21 charge). VAT
/// compounding onto VAT is not representable in EN 16931 at all.
#[test]
fn vat_on_vat_is_rejected_but_a_levy_in_the_vat_base_is_not() {
    // Legitimate: charge first, VAT over net + charge.
    BillingDocument::builder()
        .meta(meta("COMPOUND-OK"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(PercentageCharge::new("Fee", dec!(0.05)).unwrap().boxed())
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // Not representable: a second VAT layer whose base includes the first's output.
    let err = BillingDocument::builder()
        .meta(meta("VAT-ON-VAT"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .tag("a")
                .build()
                .unwrap(),
        ])
        .extra_tax(
            FixedRateTax::new("MwSt", dec!(0.19))
                .unwrap()
                .with_tag("a")
                .boxed(),
        )
        // Untagged, so its base includes the first layer's VAT position.
        .extra_tax(FixedRateTax::new("Zuschlag", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap_err();
    assert!(
        matches!(err, BillingError::LayerError { .. }),
        "got {err:?}"
    );
    assert!(
        err.to_string().contains("VAT on the VAT position"),
        "unhelpful: {err}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Document type code (BT-3) and the credit-note syntax split
// ─────────────────────────────────────────────────────────────────────────────

/// `BR-CL-01` polices two UNTDID 1001 lists, chosen by the syntax element — they
/// share only `81`, and `380` / `381` sit one in each. A reversal carrying `380`
/// is fatal in either element.
#[test]
fn reverse_sets_a_credit_note_type_code() {
    use billing::DocumentKind;

    let inv = BillingDocument::builder()
        .meta(meta("INV-K"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();
    assert_eq!(inv.meta.kind, DocumentKind::CommercialInvoice); // 380

    // The obvious spelling — `..Default::default()` — used to leave BT-3 at 380
    // on a document with negative totals.
    let credit = inv.reverse(meta("CN-K")).unwrap();
    assert_eq!(credit.meta.kind, DocumentKind::CreditNote); // 381
    assert!(credit.meta.kind.is_credit_note());
    assert!(credit.net_total().is_negative());
    credit.assert_valid();

    // An explicit credit-note code is honoured rather than overwritten.
    let explicit = inv
        .reverse(DocumentMeta {
            kind: DocumentKind::CreditNote,
            ..meta("CN-K2")
        })
        .unwrap();
    assert_eq!(explicit.meta.kind, DocumentKind::CreditNote);

    // Exactly one modelled kind belongs to the credit-note list; `383` does not,
    // despite being called a debit note.
    assert_eq!(
        DocumentKind::ALL
            .iter()
            .filter(|k| k.is_credit_note())
            .count(),
        1
    );
    assert!(!DocumentKind::DebitNote.is_credit_note());
}

// ─────────────────────────────────────────────────────────────────────────────
// Allowance / charge base amount and percentage (PEPPOL-EN16931-R041 / R042)
// ─────────────────────────────────────────────────────────────────────────────

/// A percentage allowance or charge must state its base **and** its percentage,
/// or neither — both directions are fatal in Peppol.
#[test]
fn percentage_allowances_and_charges_state_base_and_percentage() {
    let doc = BillingDocument::builder()
        .meta(meta("R041"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("Beratung", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            billing::PercentageDiscount::new("Treuerabatt", dec!(0.10))
                .unwrap()
                .with_reason_code("95")
                .boxed(),
        )
        .extra_tax(
            PercentageCharge::new("Servicepauschale", dec!(0.025))
                .unwrap()
                .with_reason_code("ABK")
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // BG-20: 10 % of 1000.00.
    let allowance = doc.discount_positions()[0]
        .allowance_charge
        .as_ref()
        .expect("BG-20 detail");
    assert_eq!(allowance.reason_code.as_deref(), Some("95")); // BT-98
    assert_eq!(
        allowance.base_amount,
        Some(Amount::parse("1000.00000").unwrap())
    ); // BT-93
    assert_eq!(allowance.percentage, Some(dec!(10))); // BT-94 — percent, not fraction
    assert!(allowance.validate().is_ok());

    // BG-21: 2.5 % of the net. `PercentageCharge` excludes credit positions from
    // its base by design, so the allowance does not reduce it — and the stated
    // pair still reproduces the amount exactly, which is what R041/R042 are for.
    let charge = doc
        .charge_positions()
        .next()
        .unwrap()
        .allowance_charge
        .as_ref()
        .expect("BG-21 detail");
    assert_eq!(charge.reason_code.as_deref(), Some("ABK")); // BT-105
    assert_eq!(
        charge.base_amount,
        Some(Amount::parse("1000.00000").unwrap())
    ); // BT-100
    assert_eq!(charge.percentage, Some(dec!(2.5))); // BT-101
    // The stated pair reproduces the amount: 1000.00 × 2.5 % = 25.00.
    assert_eq!(
        doc.charge_positions().next().unwrap().net_amount,
        Amount::parse("25.00000").unwrap()
    );

    doc.verify_vat_attribution().unwrap();
    doc.assert_valid();
}

/// A clamped percentage charge no longer equals `base × percentage`, so stating
/// the pair would be a claim a validator can disprove. It is suppressed instead.
#[test]
fn a_capped_percentage_charge_states_no_percentage_basis() {
    let doc = BillingDocument::builder()
        .meta(meta("R041-CAP"))
        .positions(vec![
            LineItem::fixed("Umsatz", Amount::parse("10000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(
            PercentageCharge::new("Provision", dec!(0.05))
                .unwrap()
                .with_max(Amount::parse("100.00000").unwrap())
                .with_reason_code("ABK")
                .boxed(),
        )
        .build()
        .unwrap();

    let charge = doc.charge_positions().next().unwrap();
    assert_eq!(charge.net_amount, Amount::parse("100.00000").unwrap()); // capped
    let ac = charge
        .allowance_charge
        .as_ref()
        .expect("reason code survives");
    assert_eq!(ac.reason_code.as_deref(), Some("ABK"));
    assert_eq!(
        ac.base_amount, None,
        "would not reproduce the capped amount"
    );
    assert_eq!(ac.percentage, None);
    assert!(ac.validate().is_ok());
    doc.assert_valid();
}

/// The R041/R042 pairing is an invariant of the type, not just of the layers.
#[test]
fn a_half_stated_percentage_basis_is_rejected() {
    use billing::AllowanceCharge;

    let only_base = AllowanceCharge {
        base_amount: Some(Amount::parse("100.00000").unwrap()),
        ..Default::default()
    };
    assert!(
        only_base
            .validate()
            .unwrap_err()
            .to_string()
            .contains("R042")
    );

    let only_pct = AllowanceCharge {
        percentage: Some(dec!(10)),
        ..Default::default()
    };
    assert!(
        only_pct
            .validate()
            .unwrap_err()
            .to_string()
            .contains("R041")
    );

    // And a LineItem carrying one is rejected too, including from JSON.
    let bad = LineItem::fixed("x", Amount::parse("10.00000").unwrap())
        .allowance_charge(only_pct)
        .build();
    assert!(bad.is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// PEPPOL-EN16931-R040 — the stated basis must reproduce the amount
// ─────────────────────────────────────────────────────────────────────────────

/// Stating BT-93 / BT-94 is a claim a validator recomputes, so the base has to
/// follow the amount through every transform. Allocation used to leave it behind.
#[test]
fn allocation_keeps_the_allowance_basis_consistent() {
    use billing::{AllocationRule, EqualAllocation};

    let doc = BillingDocument::builder()
        .meta(meta("R040-ALLOC"))
        .positions(vec![
            LineItem::fixed("Beratung", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            billing::PercentageDiscount::new("Rabatt", dec!(0.10))
                .unwrap()
                .with_reason_code("95")
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // 1000.00 split three ways does not divide evenly — the penny-correction path.
    let parts = EqualAllocation::new(3).unwrap().allocate(&doc).unwrap();
    for part in &parts {
        part.assert_valid(); // check 11 now runs R040 on every position
        for p in part.all_positions() {
            if let Some(ac) = &p.allowance_charge {
                // Either the basis was rescaled and still reproduces the amount …
                ac.check_amount(p.net_amount).unwrap();
                // … or it was dropped, keeping the reason code.
                if ac.base_amount.is_none() {
                    assert_eq!(ac.reason_code.as_deref(), Some("95"));
                }
            }
        }
    }
    // The split is still exact.
    let total: Amount<5> = parts.iter().map(|d| d.gross_total()).sum();
    assert_eq!(total, doc.gross_total());
}

/// A credit note negates the amount, so the base must negate with it — otherwise
/// R040 fails by twice the allowance.
#[test]
fn reversal_keeps_the_allowance_basis_consistent() {
    let doc = BillingDocument::builder()
        .meta(meta("R040-REV"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::fixed("Beratung", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            billing::PercentageDiscount::new("Rabatt", dec!(0.10))
                .unwrap()
                .boxed(),
        )
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    let credit = doc.reverse(meta("R040-CN")).unwrap();
    credit.assert_valid();

    let ac = credit.discount_positions()[0]
        .allowance_charge
        .as_ref()
        .expect("BG-20 detail survives");
    // Both negated: −10 % of −1000.00 = +100.00, which is the credit's amount.
    assert_eq!(ac.base_amount, Some(Amount::parse("-1000.00000").unwrap()));
    assert_eq!(ac.percentage, Some(dec!(10)));
    ac.check_amount(credit.discount_positions()[0].net_amount)
        .unwrap();
}

/// `validate()` refuses a basis that does not reproduce the amount — the state
/// every transform above exists to avoid.
#[test]
fn a_basis_that_contradicts_the_amount_is_rejected() {
    use billing::AllowanceCharge;

    // 10 % of 1000.00 is 100.00, not 250.00.
    let bad = LineItem::credit("Rabatt")
        .fixed_amount(Amount::parse("250.00000").unwrap())
        .allowance_charge(AllowanceCharge::percentage_of(
            Amount::parse("1000.00000").unwrap(),
            dec!(0.10),
        ))
        .build();
    let err = bad.unwrap_err();
    assert!(err.to_string().contains("R040"), "unhelpful: {err}");

    // Inside Peppol's ±0.02 slack is accepted — rounding residuals are not errors.
    assert!(
        LineItem::credit("Rabatt")
            .fixed_amount(Amount::parse("100.01000").unwrap())
            .allowance_charge(AllowanceCharge::percentage_of(
                Amount::parse("1000.00000").unwrap(),
                dec!(0.10),
            ))
            .build()
            .is_ok()
    );
}

/// BR-DEC-02 / BR-DEC-06 cap BT-93 / BT-100 at two decimals in their own right,
/// so the scale check has to look at them too.
#[test]
fn the_allowance_base_is_covered_by_the_amount_scale() {
    let scaled = BillingDocument::builder()
        .meta(meta("DEC02"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Arbeit",
                Quantity::new(dec!(1234.567), "kWh"),
                UnitPrice::new(dec!(0.28901), "EUR/kWh"),
            )
            .build()
            .unwrap(),
        ])
        .extra_discount(
            billing::PercentageDiscount::new("Rabatt", dec!(0.10))
                .unwrap()
                .boxed(),
        )
        .build()
        .unwrap();

    // The base is reduced along with everything else …
    let ac = scaled.discount_positions()[0]
        .allowance_charge
        .as_ref()
        .unwrap();
    assert_eq!(ac.base_amount, Some(Amount::parse("356.80000").unwrap()));
    assert!(scaled.fits_amount_scale(2));
    scaled.assert_valid();

    // … and a hand-built document whose base carries five decimals is reported,
    // naming the field rather than only the amount.
    let mut item = scaled.discount_positions()[0].clone();
    item.allowance_charge = Some(billing::AllowanceCharge {
        reason_code: None,
        base_amount: Some(Amount::parse("356.80221").unwrap()),
        percentage: Some(dec!(10)),
    });
    let violation = billing::BillingDocument::from_positions(
        DocumentMeta::default(),
        vec![item],
        vec![],
        vec![],
    )
    .unwrap()
    .amount_scale_violation(2);
    assert!(
        violation
            .as_ref()
            .is_some_and(|(what, _)| what.contains("BT-93")),
        "expected the base to be named, got {violation:?}"
    );
}

/// Reversing a document that carried **any** discount used to produce a document
/// that failed its own `assert_valid()`: every amount is negated, so the
/// allowances become positive, and check 9 tested against zero rather than
/// against the document's own direction.
#[test]
fn a_document_with_discounts_can_be_reversed() {
    for scale in [None, Some(AmountScale::EN16931)] {
        let mut b = BillingDocument::builder()
            .meta(meta("INV-D"))
            .positions(vec![
                LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                    .build()
                    .unwrap(),
            ]);
        if let Some(s) = scale {
            b = b.amount_scale(s);
        }
        let doc = b
            .extra_discount(
                billing::FixedDiscount::new("Gutschein", Amount::parse("10.00000").unwrap())
                    .unwrap()
                    .boxed(),
            )
            .extra_discount(
                billing::PercentageDiscount::new("Treue", dec!(0.05))
                    .unwrap()
                    .boxed(),
            )
            .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
            .build()
            .unwrap();
        doc.assert_valid();

        let credit = doc.reverse(meta("CN-D")).unwrap();
        credit.assert_valid();

        // The allowances are positive on the credit note — that is the point.
        assert!(
            credit
                .discount_positions()
                .iter()
                .all(|p| !p.net_amount.is_negative())
        );
        assert_eq!(
            credit.discount_total(),
            doc.discount_total().checked_neg().unwrap()
        );
        // And the pair still settles to nothing.
        assert_eq!(
            doc.gross_total().checked_add(credit.gross_total()).unwrap(),
            Amount::<5>::ZERO
        );
        // Reversing twice is the identity.
        let back = credit.reverse(meta("INV-D2")).unwrap();
        back.assert_valid();
        assert_eq!(back.gross_total(), doc.gross_total());
        assert_eq!(back.discount_total(), doc.discount_total());
    }
}

/// A genuine surcharge in the discount bucket is still rejected — the relative
/// test must not become a no-op. Hand-edited JSON is the realistic route into
/// that state, so this needs the `serde` feature.
#[cfg(feature = "serde")]
#[test]
fn a_surcharge_in_the_discount_bucket_is_still_rejected() {
    let doc = BillingDocument::builder()
        .meta(meta("SUR"))
        .positions(vec![
            LineItem::fixed("Service", Amount::parse("100.00000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_discount(
            billing::FixedDiscount::new("Rabatt", Amount::parse("10.00000").unwrap())
                .unwrap()
                .boxed(),
        )
        .build()
        .unwrap();

    // A positive "discount" on a normal document is a surcharge.
    {
        let mut json: serde_json::Value = serde_json::to_value(&doc).unwrap();
        json["discount_positions"][0]["net_amount"] = serde_json::json!("5.00000");
        json["discount_positions"][0]["sign"] = serde_json::json!("Debit");
        json["net_total"] = serde_json::json!("105.00000");
        json["gross_total"] = serde_json::json!("105.00000");
        json["discount_total"] = serde_json::json!("5.00000");
        let err = serde_json::from_value::<BillingDocument>(json).unwrap_err();
        assert!(
            err.to_string().contains("discount"),
            "a surcharge must still be caught: {err}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BG-29 PRICE DETAILS — BT-146 … BT-150
// ─────────────────────────────────────────────────────────────────────────────

/// EN 16931-1 **Annex A.1.3**, *Example 2 — Item price base quantity*: the
/// ordinary "EUR 12,00 per 100 pieces" quote.
///
/// The point of BT-149 is that the seller's own price survives onto the invoice.
/// Pre-dividing to EUR 0,12 states a BT-146 nobody quoted, and
/// `PEPPOL-EN16931-R120` — which computes the line net amount as
/// `BT-129 × (BT-146 ÷ BT-149)` — has no way to reconstruct it.
#[test]
fn a_price_base_quantity_keeps_the_quoted_price() {
    let line = LineItem::for_usage(
        "Schrauben",
        Quantity::new(dec!(250), "pcs").with_code("H87"),
        UnitPrice::new(dec!(12.00), "EUR/100 pcs")
            .per(dec!(100))
            .with_base_quantity_code("H87"),
    )
    .build()
    .unwrap();

    // R120: 250 × (12.00 / 100) = 30.00
    assert_eq!(line.net_amount, Amount::<5>::parse("30.00000").unwrap());

    let price = line.unit_price.as_ref().unwrap();
    assert_eq!(price.value, dec!(12.00)); // BT-146 — as quoted, not 0.12
    assert_eq!(price.base_quantity, Some(dec!(100))); // BT-149
    assert_eq!(price.base_quantity_code.as_deref(), Some("H87")); // BT-150
    assert_eq!(price.per_unit_value().unwrap(), dec!(0.12));
    line.validate().unwrap();
}

/// A base quantity that does not divide evenly is exactly the case that pushes a
/// pre-dividing caller into a rounding error, so the engine must not introduce
/// one either: it multiplies before it divides.
#[test]
fn a_non_dividing_price_base_quantity_does_not_lose_cents() {
    // EUR 12,00 per 7 pieces. 12/7 = 1.714285714… — non-terminating.
    let price = UnitPrice::new(dec!(12.00), "EUR/7 pcs").per(dec!(7));
    let line = LineItem::for_usage("Teile", Quantity::new(dec!(7000), "pcs"), price)
        .build()
        .unwrap();

    // (7000 × 12.00) / 7 = 12000 exactly. Rounding 12/7 to any finite scale first
    // and *then* multiplying by 7000 would not land here.
    assert_eq!(line.net_amount, Amount::<5>::parse("12000.00000").unwrap());
}

/// EN 16931-1 **Annex A.1.6**, *Example 5*: item gross price 9,50 less an item
/// price discount 1,00 gives the item net price 8,50.
///
/// This moves the *price*, not BT-131 — unlike a BG-27 line allowance, which is
/// what [`AllowanceCharge`] models. Peppol keeps them apart too: `R044` forbids a
/// *charge* at price level outright.
#[test]
fn a_gross_price_and_discount_derive_the_net_price() {
    let price = UnitPrice::discounted(dec!(9.50), dec!(1.00), "EUR/pcs");
    assert_eq!(price.gross_price, Some(dec!(9.50))); // BT-148
    assert_eq!(price.price_discount, Some(dec!(1.00))); // BT-147
    assert_eq!(price.value, dec!(8.50)); // BT-146, derived — R046 by construction

    let line = LineItem::for_usage("Ware", Quantity::new(dec!(20), "pcs"), price)
        .build()
        .unwrap();
    // BT-131 follows the *net* price; the discount never enters the line total
    // twice, which is what modelling it as a BG-27 allowance would have done.
    assert_eq!(line.net_amount, Amount::<5>::parse("170.00000").unwrap());
    assert!(line.allowance_charge.is_none());
    line.validate().unwrap();
}

/// `PEPPOL-EN16931-R046` is an **exact** equality — unlike `R040`, it has no
/// `u:slack`. A hand-assembled price whose parts do not add up is rejected.
#[test]
fn a_net_price_that_contradicts_the_gross_price_is_rejected() {
    let mut price = UnitPrice::discounted(dec!(9.50), dec!(1.00), "EUR/pcs");
    price.value = dec!(8.51); // one cent out — fatal under R046, tolerance zero

    let err = price.validate().unwrap_err();
    assert!(err.to_string().contains("R046"), "{err}");

    // …and the builder refuses it rather than leaving `validate` as the only guard.
    let err = LineItem::for_usage("Ware", Quantity::new(dec!(20), "pcs"), price)
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("R046"), "{err}");
}

/// Half-stated BG-29 pairs. BT-147 is *defined* as a subtraction from BT-148, and
/// BT-150 is an attribute of BT-149 in UBL — neither can stand alone.
#[test]
fn half_stated_price_details_are_rejected() {
    let mut discount_only = UnitPrice::new(dec!(8.50), "EUR/pcs");
    discount_only.price_discount = Some(dec!(1.00));
    let err = discount_only.validate().unwrap_err();
    assert!(err.to_string().contains("BT-147"), "{err}");

    let code_only = UnitPrice::new(dec!(8.50), "EUR/pcs").with_base_quantity_code("H87");
    let err = code_only.validate().unwrap_err();
    assert!(err.to_string().contains("BT-150"), "{err}");
}

/// `PEPPOL-EN16931-R121` — the base quantity is a divisor, so zero is not merely
/// invalid, it is arithmetically fatal.
#[test]
fn a_non_positive_price_base_quantity_is_rejected() {
    for base in [dec!(0), dec!(-100)] {
        let price = UnitPrice::new(dec!(12.00), "EUR/pcs").per(base);
        let err = price.validate().unwrap_err();
        assert!(err.to_string().contains("R121"), "base {base}: {err}");

        let err = LineItem::for_usage("Teile", Quantity::new(dec!(10), "pcs"), price)
            .build()
            .unwrap_err();
        assert!(err.to_string().contains("R121"), "base {base}: {err}");
    }
}

/// `PEPPOL-EN16931-R130` — BT-150 must equal BT-130. A cross-field rule: only the
/// line sees both codes.
#[test]
fn a_price_base_unit_that_differs_from_the_quantity_unit_is_rejected() {
    let err = LineItem::for_usage(
        "Schrauben",
        Quantity::new(dec!(250), "pcs").with_code("H87"), // BT-130
        UnitPrice::new(dec!(12.00), "EUR/100 pcs")
            .per(dec!(100))
            .with_base_quantity_code("KGM"), // BT-150 — disagrees
    )
    .build()
    .unwrap_err();
    assert!(err.to_string().contains("R130"), "{err}");

    // Stating only one of the two codes is not a contradiction, and the engine
    // does not invent BT-130 from a display label.
    LineItem::for_usage(
        "Schrauben",
        Quantity::new(dec!(250), "pcs"),
        UnitPrice::new(dec!(12.00), "EUR/100 pcs")
            .per(dec!(100))
            .with_base_quantity_code("H87"),
    )
    .build()
    .unwrap();
}

/// `rounded()` re-derives BT-146 from the rounded BT-148 / BT-147 rather than
/// rounding it independently, because R046 admits no residual.
#[test]
fn rounding_a_discounted_price_keeps_r046_exact() {
    let p = UnitPrice::discounted(dec!(9.5049), dec!(1.0049), "EUR/pcs")
        .rounded(2, RoundingStrategy::MidpointAwayFromZero);

    assert_eq!(p.gross_price, Some(dec!(9.50)));
    assert_eq!(p.price_discount, Some(dec!(1.00)));
    assert_eq!(p.value, dec!(8.50)); // exactly 9.50 − 1.00
    p.validate().unwrap();

    // Without a gross price the behaviour is unchanged: BT-146 is rounded directly.
    let plain = UnitPrice::new(dec!(0.123456), "EUR/kWh")
        .rounded(4, RoundingStrategy::MidpointAwayFromZero);
    assert_eq!(plain.value, dec!(0.1235));
}

/// Scaling a line (proration, allocation) leaves BG-29 alone and moves the
/// quantity, so `R120` still reproduces the net amount.
#[test]
fn scaling_a_line_preserves_the_price_base_quantity() {
    let full = LineItem::for_usage(
        "Schrauben",
        Quantity::new(dec!(250), "pcs"),
        UnitPrice::new(dec!(12.00), "EUR/100 pcs").per(dec!(100)),
    )
    .build()
    .unwrap();
    let half = full
        .scaled(dec!(0.5), RoundingStrategy::MidpointAwayFromZero)
        .unwrap();

    assert_eq!(half.quantity_value(), Some(dec!(125)));
    assert_eq!(half.unit_price.as_ref().unwrap().value, dec!(12.00));
    assert_eq!(
        half.unit_price.as_ref().unwrap().base_quantity,
        Some(dec!(100))
    );
    // R120 recomputed: 125 × (12.00 / 100) = 15.00
    assert_eq!(half.net_amount, Amount::<5>::parse("15.00000").unwrap());
    half.validate().unwrap();
}

/// Deserialisation re-runs the BG-29 checks, so untrusted JSON cannot introduce a
/// price that violates R046 or R121.
#[cfg(feature = "serde")]
#[test]
fn deserialising_a_price_re_runs_the_bg29_checks() {
    let price = UnitPrice::discounted(dec!(9.50), dec!(1.00), "EUR/pcs")
        .per(dec!(10))
        .with_base_quantity_code("H87");
    let json = serde_json::to_string(&price).unwrap();
    assert_eq!(serde_json::from_str::<UnitPrice>(&json).unwrap(), price);

    let mut broken: serde_json::Value = serde_json::from_str(&json).unwrap();
    broken["value"] = serde_json::json!("8.51");
    let err = serde_json::from_value::<UnitPrice>(broken).unwrap_err();
    assert!(err.to_string().contains("R046"), "{err}");

    let mut broken: serde_json::Value = serde_json::from_str(&json).unwrap();
    broken["base_quantity"] = serde_json::json!("0");
    let err = serde_json::from_value::<UnitPrice>(broken).unwrap_err();
    assert!(err.to_string().contains("R121"), "{err}");
}

/// A price built with no BG-29 extras behaves exactly as before — the subgroup is
/// additive, not a new requirement.
#[test]
fn a_plain_price_is_unaffected_by_bg29() {
    let line = LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.289), "EUR/kWh"),
    )
    .build()
    .unwrap();
    assert_eq!(line.net_amount, Amount::<5>::parse("289.00000").unwrap());

    let price = line.unit_price.as_ref().unwrap();
    assert_eq!(price.base_quantity, None);
    assert_eq!(price.per_unit_value().unwrap(), dec!(0.289));
}

// ─────────────────────────────────────────────────────────────────────────────
// Profile layering above BR-CL-01 (P0100 / P0101 / P0112)
// ─────────────────────────────────────────────────────────────────────────────

/// Passing `BR-CL-01` says nothing about whether Peppol BIS Billing accepts the
/// code: the CEN lists hold 50 and 13 codes, the Peppol ones 26 and five.
#[test]
fn peppol_billing_narrows_the_document_type_code_list() {
    use billing::DocumentKind;

    // `389` is in BR-CL-01's invoice list but not in PEPPOL-EN16931-P0100.
    assert!(!DocumentKind::SelfBilledInvoice.is_peppol_billing_code());
    assert_eq!(DocumentKind::SelfBilledInvoice.code(), 389);

    // Everything else this crate models survives the Billing profile.
    for kind in DocumentKind::ALL {
        if kind == DocumentKind::SelfBilledInvoice {
            continue;
        }
        assert!(kind.is_peppol_billing_code(), "{kind:?} ({})", kind.code());
    }

    // P0112 layers a *party* condition on two of them, which is why the narrowing
    // cannot live in this crate.
    assert!(DocumentKind::PartialInvoice.requires_german_parties()); // 326
    assert!(DocumentKind::CorrectedInvoice.requires_german_parties()); // 384
    assert_eq!(
        DocumentKind::ALL
            .iter()
            .filter(|k| k.requires_german_parties())
            .count(),
        2
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// BG-27 / BG-28 — invoice line allowances and charges (BT-136 … BT-145)
// ─────────────────────────────────────────────────────────────────────────────

/// The full `PEPPOL-EN16931-R120` identity, which needs all three of BT-149,
/// BG-27 and BG-28 to be expressible at once:
///
/// > BT-131 = BT-129 × (BT-146 ÷ BT-149) + Σ BG-28 − Σ BG-27
#[test]
fn r120_holds_with_a_base_quantity_and_line_allowances() {
    use billing::LineAllowanceCharge;

    let line = LineItem::for_usage(
        "Schrauben",
        Quantity::new(dec!(250), "pcs"),
        UnitPrice::new(dec!(12.00), "EUR/100 pcs").per(dec!(100)), // BT-149
    )
    .line_allowance(
        LineAllowanceCharge::allowance(Amount::parse("3.00000").unwrap(), "Mengenrabatt")
            .of(Amount::parse("30.00000").unwrap(), dec!(0.10)), // BT-137 / BT-138
    )
    .line_allowance(LineAllowanceCharge::charge(
        Amount::parse("1.50000").unwrap(),
        "Verpackung",
    ))
    .build()
    .unwrap();

    // 250 × (12.00 / 100) = 30.00, − 3.00 + 1.50 = 28.50
    assert_eq!(line.net_amount, Amount::<5>::parse("28.50000").unwrap());
    assert_eq!(line.line_allowances.len(), 2);

    // The parts are carried, not just their effect — a consumer can emit BG-27/28.
    let a = &line.line_allowances[0];
    assert_eq!(a.kind, billing::AllowanceKind::Allowance);
    assert!(!a.kind.charge_indicator()); // cbc:ChargeIndicator = false
    assert_eq!(a.base_amount, Some(Amount::parse("30.00000").unwrap())); // BT-137
    assert_eq!(a.percentage, Some(dec!(10))); // BT-138
    assert_eq!(
        a.signed_amount().unwrap(),
        Amount::parse("-3.00000").unwrap()
    );
    assert!(line.line_allowances[1].kind.charge_indicator());
    line.validate().unwrap();
}

/// BR-42 / BR-44, restated by BR-CO-23 / BR-CO-24: a line allowance or charge
/// needs a reason **or** a reason code. Unlike a document level allowance it has
/// no `description` to fall back on for BT-97 / BT-104.
#[test]
fn a_line_allowance_without_any_reason_is_rejected() {
    use billing::LineAllowanceCharge;

    let mut naked = LineAllowanceCharge::allowance(Amount::parse("5.00000").unwrap(), "Rabatt");
    naked.reason = None;
    let err = naked.validate().unwrap_err();
    assert!(err.to_string().contains("BR-42"), "{err}");

    // Whitespace is not a reason.
    let mut blank = LineAllowanceCharge::charge(Amount::parse("5.00000").unwrap(), "   ");
    assert!(blank.validate().is_err());
    blank.reason_code = Some("ZZZ".into()); // BT-145 alone satisfies BR-44
    blank.validate().unwrap();

    // The charge variant names its own terms.
    let mut naked_charge = LineAllowanceCharge::charge(Amount::parse("5.00000").unwrap(), "x");
    naked_charge.reason = None;
    let err = naked_charge.validate().unwrap_err();
    assert!(err.to_string().contains("BR-44"), "{err}");
    assert!(err.to_string().contains("BT-145"), "{err}");

    // A code-only constructor is fine from the start.
    LineAllowanceCharge::coded_allowance(Amount::parse("5.00000").unwrap(), "95")
        .validate()
        .unwrap();
    LineAllowanceCharge::coded_charge(Amount::parse("5.00000").unwrap(), "AAA")
        .validate()
        .unwrap();

    // And the builder refuses it rather than leaving `validate` as the only guard.
    let err = LineItem::fixed("Ware", Amount::parse("100.00000").unwrap())
        .line_allowance(naked)
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("BR-42"), "{err}");
}

/// `PEPPOL-EN16931-R040` / `R041` / `R042` list `cac:InvoiceLine/cac:AllowanceCharge`
/// in their contexts alongside the document level one, so a line allowance is held
/// to the same base-and-percentage rules.
#[test]
fn a_line_allowance_basis_obeys_the_same_peppol_rules() {
    use billing::LineAllowanceCharge;

    // R042: a base without a percentage.
    let mut half = LineAllowanceCharge::allowance(Amount::parse("5.00000").unwrap(), "Rabatt");
    half.base_amount = Some(Amount::parse("100.00000").unwrap());
    let err = half.validate().unwrap_err();
    assert!(err.to_string().contains("R042"), "{err}");

    // R040: a basis that does not reproduce the amount.
    let wrong = LineAllowanceCharge::allowance(Amount::parse("5.00000").unwrap(), "Rabatt")
        .of(Amount::parse("100.00000").unwrap(), dec!(0.10)); // claims 10.00
    let err = wrong.validate().unwrap_err();
    assert!(err.to_string().contains("R040"), "{err}");

    // Dropping the basis is always valid.
    wrong.without_basis().validate().unwrap();
}

/// Line allowances are components of BT-131, so scaling and reversal must move
/// them with it — otherwise the stated parts contradict the line total.
#[test]
fn line_allowances_follow_scaling_and_reversal() {
    use billing::LineAllowanceCharge;

    let full = LineItem::for_usage(
        "Ware",
        Quantity::new(dec!(100), "pcs"),
        UnitPrice::new(dec!(10.00), "EUR/pcs"),
    )
    .line_allowance(
        LineAllowanceCharge::allowance(Amount::parse("100.00000").unwrap(), "Rabatt")
            .of(Amount::parse("1000.00000").unwrap(), dec!(0.10)),
    )
    .build()
    .unwrap();
    assert_eq!(full.net_amount, Amount::<5>::parse("900.00000").unwrap());

    // Scaling: amount and base both halve, so R040 still holds and the parts still
    // sum to BT-131.
    let half = full
        .scaled(dec!(0.5), RoundingStrategy::MidpointAwayFromZero)
        .unwrap();
    assert_eq!(half.net_amount, Amount::<5>::parse("450.00000").unwrap());
    assert_eq!(
        half.line_allowances[0].amount,
        Amount::parse("50.00000").unwrap()
    );
    assert_eq!(
        half.line_allowances[0].base_amount,
        Some(Amount::parse("500.00000").unwrap())
    );
    assert_eq!(half.line_allowances[0].percentage, Some(dec!(10))); // a rate: unscaled
    half.validate().unwrap();
    // 50 × (10.00/1) … 500 − 50 = 450 ✓ R120 still reproduces the net amount.
    assert_eq!(
        half.quantity_value().unwrap() * half.unit_price.as_ref().unwrap().value
            - half.line_allowances[0].amount.into_decimal(),
        half.net_amount.into_decimal()
    );

    // Reversal negates them alongside everything else.
    let doc = BillingDocument::builder()
        .meta(meta("INV-LA"))
        .positions(vec![full])
        .build()
        .unwrap();
    let credit = doc.reverse(meta("CN-LA")).unwrap();
    let reversed = &credit.net_positions()[0];
    assert_eq!(
        reversed.net_amount,
        Amount::<5>::parse("-900.00000").unwrap()
    );
    assert_eq!(
        reversed.line_allowances[0].amount,
        Amount::parse("-100.00000").unwrap()
    );
    // BT-137 follows BT-136. Leaving the base positive beside a negated amount
    // states "−100.00, being 10 % of 1000.00" — arithmetic that only holds
    // because R040 compares magnitudes, which is exactly why nothing else here
    // would catch it.
    assert_eq!(
        reversed.line_allowances[0].base_amount,
        Some(Amount::parse("-1000.00000").unwrap())
    );
    assert_eq!(reversed.line_allowances[0].percentage, Some(dec!(10))); // a rate: unsigned
    reversed.validate().unwrap(); // R040 compares magnitudes, so it survives
    credit.assert_valid();

    // Reversing twice is the identity, base included.
    let restored = credit.reverse(meta("INV-LA-2")).unwrap();
    let restored = &restored.net_positions()[0];
    assert_eq!(
        restored.line_allowances[0].base_amount,
        Some(Amount::parse("1000.00000").unwrap())
    );
    assert_eq!(
        restored.line_allowances[0].amount,
        Amount::parse("100.00000").unwrap()
    );
}

/// A line allowance moves BT-131 only. The document totals chain needs no special
/// case, because BT-106 is the sum of the BT-131s.
#[test]
fn line_allowances_reach_the_totals_only_through_bt_131() {
    use billing::LineAllowanceCharge;

    let doc = BillingDocument::builder()
        .meta(meta("INV-LA2"))
        .positions(vec![
            LineItem::for_usage(
                "Ware",
                Quantity::new(dec!(100), "pcs"),
                UnitPrice::new(dec!(10.00), "EUR/pcs"),
            )
            .line_allowance(LineAllowanceCharge::allowance(
                Amount::parse("100.00000").unwrap(),
                "Rabatt",
            ))
            .build()
            .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    // BT-106 = Σ BT-131 = 900,00 — the allowance is already inside the line.
    assert_eq!(
        doc.line_total().unwrap(),
        Amount::parse("900.00000").unwrap()
    );
    // It is NOT a document level allowance (BT-92), so BT-107 is untouched.
    assert_eq!(doc.discount_total(), Amount::<5>::ZERO);
    // VAT is charged on the reduced base, which is the point.
    assert_eq!(
        doc.vat_total().unwrap(),
        Amount::parse("171.00000").unwrap()
    );
    doc.assert_valid();
}

/// Deserialisation re-runs the BG-27 / BG-28 checks.
#[cfg(feature = "serde")]
#[test]
fn deserialising_a_line_allowance_re_runs_its_checks() {
    use billing::LineAllowanceCharge;

    let lac = LineAllowanceCharge::allowance(Amount::parse("10.00000").unwrap(), "Rabatt")
        .of(Amount::parse("100.00000").unwrap(), dec!(0.10));
    let json = serde_json::to_string(&lac).unwrap();
    assert_eq!(
        serde_json::from_str::<LineAllowanceCharge>(&json).unwrap(),
        lac
    );

    // Reason stripped → BR-42.
    let mut broken: serde_json::Value = serde_json::from_str(&json).unwrap();
    broken["reason"] = serde_json::Value::Null;
    let err = serde_json::from_value::<LineAllowanceCharge>(broken).unwrap_err();
    assert!(err.to_string().contains("BR-42"), "{err}");

    // Basis no longer reproduces the amount → R040.
    let mut broken: serde_json::Value = serde_json::from_str(&json).unwrap();
    broken["amount"] = serde_json::json!("50.00000");
    let err = serde_json::from_value::<LineAllowanceCharge>(broken).unwrap_err();
    assert!(err.to_string().contains("R040"), "{err}");
}

// ─────────────────────────────────────────────────────────────────────────────
// Interchange-scale reduction must see the new BG-27 / BG-28 / BT-149 leaves
// ─────────────────────────────────────────────────────────────────────────────

/// The crate's headline precision guarantee is that every leaf is rounded **once**
/// from its exact value. A price base quantity divides, so the quotient is exactly
/// where a double rounding hurts — and `reduce_position` must reconstruct
/// `BT-129 × (BT-146 ÷ BT-149)`, not `BT-129 × BT-146`, or it silently falls back
/// to reducing the already-rounded five-decimal amount.
#[test]
fn scale_reduction_rounds_once_through_a_price_base_quantity() {
    // The decisive shape from the README: 1 × (0.014997 ÷ 3) is exactly 0.004999.
    // Rounded once to 2 decimals that is 0.00. Rounded to the engine's 5 first it
    // becomes 0.00500, and *then* to 2 it becomes 0.01 — a whole minor unit away.
    let line = || {
        LineItem::for_usage(
            "Teil",
            Quantity::new(dec!(1), "pcs"),
            UnitPrice::new(dec!(0.014997), "EUR/3 pcs").per(dec!(3)),
        )
        .build()
        .unwrap()
    };
    assert_eq!(line().net_amount, Amount::<5>::parse("0.00500").unwrap());

    let doc = BillingDocument::builder()
        .meta(meta("INV-BQ"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![line()])
        .build()
        .unwrap();

    // Reduced from the exact quotient, not from the stored five decimals. Getting
    // this wrong is invisible: both answers are plausible two-decimal amounts.
    assert_eq!(
        doc.net_positions()[0].net_amount,
        Amount::<5>::parse("0.00000").unwrap(),
        "reduced twice — `reduce_position` did not divide by BT-149"
    );
    assert!(doc.fits_amount_scale(2));

    // And the ordinary non-terminating case still lands where R120 needs it.
    let doc = BillingDocument::builder()
        .meta(meta("INV-BQ2"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::for_usage(
                "Teile",
                Quantity::new(dec!(7), "pcs"),
                UnitPrice::new(dec!(10.00), "EUR/3 pcs").per(dec!(3)),
            )
            .build()
            .unwrap(),
        ])
        .build()
        .unwrap();
    assert_eq!(
        doc.net_positions()[0].net_amount,
        Amount::<5>::parse("23.33000").unwrap()
    );
    // R120 recomputed at the emitted precision: |23.33 − 7 × (10.00/3)| ≤ 0.02.
    let r120 = dec!(7) * (dec!(10.00) / dec!(3));
    assert!((doc.net_positions()[0].net_amount.into_decimal() - r120).abs() <= dec!(0.02));
}

/// BT-136 / BT-141 are capped at two decimals in their own right by BR-DEC-24 /
/// BR-DEC-27, and they are components of BT-131 — so scale reduction has to touch
/// them, and `fits_amount_scale` has to look at them.
#[test]
fn scale_reduction_covers_line_allowance_amounts() {
    use billing::LineAllowanceCharge;

    let raw = BillingDocument::builder()
        .meta(meta("INV-LA3"))
        .positions(vec![
            LineItem::for_usage(
                "Ware",
                Quantity::new(dec!(100), "pcs"),
                UnitPrice::new(dec!(10.00), "EUR/pcs"),
            )
            .line_allowance(
                // 3.333 % of 1000,00 → 33.33000, a base with more than 2 decimals.
                LineAllowanceCharge::allowance(Amount::parse("33.33300").unwrap(), "Rabatt")
                    .of(Amount::parse("1000.00000").unwrap(), dec!(0.033333)),
            )
            .build()
            .unwrap(),
        ])
        .build()
        .unwrap();

    // Un-reduced, the line allowance amount is not emittable as EN 16931. It is
    // reported through BT-131, which carries the same third decimal.
    let (label, _) = raw
        .amount_scale_violation(2)
        .expect("BT-136 has 3 decimals, and so does the BT-131 it feeds");
    assert!(label.contains("position[0]"), "{label}");

    // BT-137 is capped in its own right (BR-DEC-25), so a base with more decimals
    // than its amount is caught even when every total is clean. Nothing but the
    // new check can see this one: 1000,00 − 10,00 = 990,00 fits perfectly.
    let base_only = BillingDocument::builder()
        .meta(meta("INV-LA3b"))
        .positions(vec![
            LineItem::for_usage(
                "Ware",
                Quantity::new(dec!(100), "pcs"),
                UnitPrice::new(dec!(10.00), "EUR/pcs"),
            )
            .line_allowance(
                LineAllowanceCharge::allowance(Amount::parse("10.00000").unwrap(), "Rabatt")
                    .of(Amount::parse("1000.00500").unwrap(), dec!(0.01)),
            )
            .build()
            .unwrap(),
        ])
        .build()
        .unwrap();
    assert_eq!(
        base_only.net_positions()[0].net_amount,
        Amount::<5>::parse("990.00000").unwrap()
    );
    let (label, amount) = base_only
        .amount_scale_violation(2)
        .expect("BT-137 has 3 decimals");
    assert!(label.contains("BT-137"), "{label}");
    assert_eq!(amount, Amount::parse("1000.00500").unwrap());

    // … and with a scale everything is reduced along with the line total.
    let doc = BillingDocument::builder()
        .meta(meta("INV-LA4"))
        .amount_scale(AmountScale::EN16931)
        .positions(raw.net_positions().to_vec())
        .build()
        .unwrap();
    assert!(
        doc.fits_amount_scale(2),
        "{:?}",
        doc.amount_scale_violation(2)
    );

    let line = &doc.net_positions()[0];
    assert_eq!(
        line.line_allowances[0].amount,
        Amount::parse("33.33000").unwrap()
    );
    // BT-131 equals the sum of the very parts a consumer emits: 1000,00 − 33,33.
    assert_eq!(line.net_amount, Amount::<5>::parse("966.67000").unwrap());
    assert_eq!(
        line.net_amount,
        Amount::<5>::parse("1000.00000")
            .unwrap()
            .checked_sub(line.line_allowances[0].amount)
            .unwrap()
    );
}

/// BG-27 / BG-28 are children of an invoice line (BG-25); BG-20 / BG-21 are
/// children of the document. A position cannot be both.
#[test]
fn a_document_level_allowance_cannot_carry_line_allowances() {
    use billing::{AllowanceCharge, LineAllowanceCharge};

    let err = LineItem::credit_fixed("Rabatt", Amount::parse("50.00000").unwrap())
        .allowance_charge(AllowanceCharge::coded("95")) // declares BG-20
        .line_allowance(LineAllowanceCharge::allowance(
            Amount::parse("5.00000").unwrap(),
            "Sub-Rabatt",
        ))
        .build()
        .unwrap_err();
    assert!(err.to_string().contains("BG-27/BG-28"), "{err}");

    // Either alone is fine.
    LineItem::credit_fixed("Rabatt", Amount::parse("50.00000").unwrap())
        .allowance_charge(AllowanceCharge::coded("95"))
        .build()
        .unwrap();
    LineItem::fixed("Ware", Amount::parse("100.00000").unwrap())
        .line_allowance(LineAllowanceCharge::allowance(
            Amount::parse("5.00000").unwrap(),
            "Rabatt",
        ))
        .build()
        .unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// BR-22 / BR-23 / BR-26 — every invoice line needs BT-129, BT-130 and BT-146
// ─────────────────────────────────────────────────────────────────────────────

/// `LineItem::fixed` states an amount and nothing else, which is three fatal rules
/// short of a complete invoice line. `flat_fee` states the same amount as the
/// line EN 16931 actually requires.
#[test]
fn flat_fee_is_a_complete_invoice_line_where_fixed_is_not() {
    use billing::UNIT_CODE_ONE;

    let bare = LineItem::fixed("Grundpreis", Amount::parse("8.50000").unwrap())
        .build()
        .unwrap();
    // BR-22, BR-23 and BR-26 all unsatisfied — a consumer must synthesise them.
    assert!(bare.quantity.is_none());
    assert!(bare.unit_price.is_none());

    let complete = LineItem::flat_fee("Grundpreis", Amount::parse("8.50000").unwrap())
        .build()
        .unwrap();
    assert_eq!(complete.net_amount, bare.net_amount); // same money
    let q = complete.quantity.as_ref().unwrap();
    assert_eq!(q.value, dec!(1)); // BR-22 — BT-129
    assert_eq!(q.code.as_deref(), Some(UNIT_CODE_ONE)); // BR-23 — BT-130 = C62
    assert_eq!(complete.unit_price.as_ref().unwrap().value, dec!(8.5)); // BR-26 — BT-146
    complete.validate().unwrap();

    // R120 is trivially satisfied: 1 × 8,50 = 8,50.
    assert_eq!(
        q.value * complete.unit_price.as_ref().unwrap().value,
        complete.net_amount.into_decimal()
    );

    // The credit counterpart is symmetric.
    let refund = LineItem::credit_flat_fee("Gutschrift", Amount::parse("8.50000").unwrap())
        .build()
        .unwrap();
    assert_eq!(refund.net_amount, Amount::<5>::parse("-8.50000").unwrap());
    assert!(refund.is_credit());
    refund.validate().unwrap();
}

/// A flat fee still reduces correctly at the interchange scale, and does not lose
/// the amount to the price/quantity round trip.
#[test]
fn flat_fee_survives_scale_reduction_and_vat() {
    let doc = BillingDocument::builder()
        .meta(meta("INV-FF"))
        .amount_scale(AmountScale::EN16931)
        .positions(vec![
            LineItem::flat_fee("Grundpreis", Amount::parse("8.50000").unwrap())
                .build()
                .unwrap(),
            LineItem::flat_fee("Zählermiete", Amount::parse("2.33000").unwrap())
                .build()
                .unwrap(),
        ])
        .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().boxed())
        .build()
        .unwrap();

    assert_eq!(
        doc.line_total().unwrap(),
        Amount::parse("10.83000").unwrap()
    );
    assert_eq!(doc.vat_total().unwrap(), Amount::parse("2.06000").unwrap()); // 10.83 × 0.19
    assert!(doc.fits_amount_scale(2));
    doc.assert_valid();
}
