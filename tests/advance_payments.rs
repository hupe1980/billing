//! Advance payments, final invoices and residual invoices.
//!
//! The scenario throughout: a supply is billed in instalments, and a settling
//! document has to reconcile them. The two lawful shapes are exercised side by
//! side, and the arithmetic is checked to agree between them.

use billing::advance::residual_breakdown;
use billing::prelude::*;
use billing::{AdvancePayment, FixedRateTax, Prepayment, TaxBreakdownEntry};
use rust_decimal::{Decimal, dec};

fn meta(number: &str) -> DocumentMeta {
    DocumentMeta {
        invoice_number: number.into(),
        currency: Currency::EUR,
        ..Default::default()
    }
}

/// An advance of `base` net plus `tax` VAT at 19%.
fn advance(reference: &str, base: &str, tax: &str) -> AdvancePayment {
    AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse(base).unwrap(),
        Amount::parse(tax).unwrap(),
    )])
    .unwrap()
    .with_reference(reference)
}

/// The whole supply: 1000.00 net + 19% VAT.
fn full_supply(number: &str) -> BillingDocument {
    BillingDocument::from_positions(
        meta(number),
        vec![
            LineItem::fixed("Jahresverbrauch", Amount::parse("1000.00000").unwrap())
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap()
}

// ─────────────────────────────────────────────────────────────────────────────
// Final invoice: full base, advances deducted
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn final_invoice_keeps_the_full_base_and_deducts_advances_with_their_tax() {
    let doc = full_supply("END-1")
        .with_advances(vec![
            advance("AB-1", "375.00000", "71.25000"),
            advance("AB-2", "375.00000", "71.25000"),
        ])
        .unwrap();

    // The taxable base still describes the whole supply — this is the rule that
    // must not bend. Reducing it by the advances would understate output tax.
    assert_eq!(doc.net_total(), Amount::parse("1000.00000").unwrap());
    assert_eq!(doc.tax_total(), Amount::parse("190.00000").unwrap());
    assert_eq!(doc.gross_total(), Amount::parse("1190.00000").unwrap());
    assert_eq!(
        doc.tax_breakdown()[0].taxable_base,
        Amount::parse("1000.00000").unwrap()
    );

    // The deduction, and the tax it contains — the figure §14 Abs. 5 S. 2 demands.
    assert_eq!(doc.prepaid(), Amount::parse("892.50000").unwrap());
    assert_eq!(
        doc.advance_tax_total().unwrap(),
        Amount::parse("142.50000").unwrap()
    );
    assert_eq!(
        doc.amount_due().unwrap(),
        Amount::parse("297.50000").unwrap()
    );
    doc.assert_valid();
}

#[test]
fn advance_deductions_merge_per_rate() {
    let doc = full_supply("END-2")
        .with_advances(vec![
            advance("AB-1", "375.00000", "71.25000"),
            advance("AB-2", "375.00000", "71.25000"),
        ])
        .unwrap();

    // Two advances at the same rate present as ONE deduction line.
    let deductions = doc.advance_deductions().unwrap();
    assert_eq!(deductions.len(), 1);
    assert_eq!(
        deductions[0].taxable_base,
        Amount::parse("750.00000").unwrap()
    );
    assert_eq!(
        deductions[0].tax_amount,
        Amount::parse("142.50000").unwrap()
    );
    assert_eq!(deductions[0].category, TaxCategory::Standard);
}

#[test]
fn advances_are_rejected_when_they_do_not_correspond_to_the_supply() {
    // A rate the supply does not contain.
    let wrong_rate = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.07),
        Amount::parse("100.00000").unwrap(),
        Amount::parse("7.00000").unwrap(),
    )])
    .unwrap();
    assert!(
        full_supply("END-3")
            .with_advances(vec![wrong_rate])
            .is_err()
    );

    // More than the whole supply — deducting would understate output tax.
    let too_much = advance("AB-X", "2000.00000", "380.00000");
    assert!(full_supply("END-4").with_advances(vec![too_much]).is_err());
}

#[test]
fn a_prepayment_is_one_value_so_a_total_and_advances_cannot_disagree() {
    // `Prepayment` is a single enum rather than two fields, so "a flat total of 900
    // alongside advances summing to 476" is not a state that can be written down.
    let doc = full_supply("END-5")
        .with_advances(vec![advance("AB-1", "375.00000", "71.25000")])
        .unwrap();
    doc.assert_valid();
    assert_eq!(doc.prepaid(), Amount::parse("446.25000").unwrap());
    assert_eq!(doc.advances().len(), 1);
    assert!(matches!(doc.prepayment(), Prepayment::Itemised(_)));

    // Setting a flat total REPLACES the itemisation wholesale — there is no
    // in-between state where both are half in force.
    let flat = doc
        .clone()
        .with_prepaid(Amount::parse("100.00000").unwrap())
        .unwrap();
    flat.assert_valid();
    assert_eq!(flat.prepaid(), Amount::parse("100.00000").unwrap());
    assert!(
        flat.advances().is_empty(),
        "the itemisation is gone, not merged"
    );
    assert_eq!(flat.advance_tax_total().unwrap(), Amount::<5>::ZERO);
    assert!(matches!(flat.prepayment(), Prepayment::Total(_)));

    // And the reverse direction replaces just as cleanly.
    let itemised = flat
        .with_advances(vec![advance("AB-2", "375.00000", "71.25000")])
        .unwrap();
    itemised.assert_valid();
    assert_eq!(itemised.prepaid(), Amount::parse("446.25000").unwrap());

    // `Prepayment::itemised` refuses an empty list: say `None` instead.
    assert!(Prepayment::itemised(vec![]).is_err());
    assert!(Prepayment::total_of(Amount::parse("-1.00000").unwrap()).is_err());
}

#[test]
fn a_negative_advance_is_rejected_at_construction() {
    // A negative advance satisfies BR-CO-17 (-100 × 0.19 == -19) and every category
    // rule, so nothing else would have caught it — and it produced a negative
    // BT-113, an amount_due LARGER than the gross, and a document that failed its
    // own validate().
    let r = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("-100.00000").unwrap(),
        Amount::parse("-19.00000").unwrap(),
    )]);
    assert!(r.is_err(), "a negative advance must not construct");
}

#[test]
fn an_advance_must_state_its_tax() {
    assert!(AdvancePayment::new(vec![]).is_err());
}

// ─────────────────────────────────────────────────────────────────────────────
// Residual invoice: only the remainder is billed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn residual_invoice_bills_only_the_remainder_and_reaches_the_same_amount_due() {
    let full = full_supply("END-6");
    let advances = vec![
        advance("AB-1", "375.00000", "71.25000"),
        advance("AB-2", "375.00000", "71.25000"),
    ];

    // The final-invoice route.
    let final_invoice = full.clone().with_advances(advances.clone()).unwrap();

    // The residual route: work out what is left, then bill exactly that.
    let residual = residual_breakdown(full.tax_breakdown(), &advances).unwrap();
    assert_eq!(residual.len(), 1);
    assert_eq!(
        residual[0].taxable_base,
        Amount::parse("250.00000").unwrap()
    );
    assert_eq!(residual[0].tax_amount, Amount::parse("47.50000").unwrap());

    let residual_invoice = BillingDocument::from_positions(
        meta("REST-6"),
        vec![
            LineItem::fixed("Restbetrag", residual[0].taxable_base)
                .build()
                .unwrap(),
        ],
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())],
        vec![],
    )
    .unwrap();

    // Different documents, same money owed — the two lawful shapes agree.
    assert_eq!(
        residual_invoice.gross_total(),
        final_invoice.amount_due().unwrap()
    );
    // The residual invoice does NOT list the advances.
    assert!(residual_invoice.advances().is_empty());
    assert_eq!(residual_invoice.prepaid(), Amount::<5>::ZERO);
    residual_invoice.assert_valid();
}

#[test]
fn residual_of_a_fully_settled_supply_is_empty() {
    let full = full_supply("END-7");
    let all = vec![advance("AB-1", "1000.00000", "190.00000")];
    assert!(
        residual_breakdown(full.tax_breakdown(), &all)
            .unwrap()
            .is_empty()
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Interactions
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn a_final_invoice_can_be_reversed_into_a_valid_credit_note() {
    // The case that used to be impossible: an invoice carrying advances could not
    // be Storno'd, because reverse() produced a negative BT-113 that validate()
    // rejected.
    let doc = full_supply("END-8")
        .with_advances(vec![advance("AB-1", "375.00000", "71.25000")])
        .unwrap();

    let credit = doc.reverse(meta("CN-8")).unwrap();
    credit.assert_valid();

    // The credit note refunds the FULL gross: the customer paid the advances plus
    // the remainder, so all of it comes back.
    assert_eq!(credit.gross_total(), Amount::parse("-1190.00000").unwrap());
    assert_eq!(
        credit.amount_due().unwrap(),
        Amount::parse("-1190.00000").unwrap()
    );
    // Advances belong to the original document and are not restated.
    assert!(credit.advances().is_empty());
    assert_eq!(credit.prepaid(), Amount::<5>::ZERO);
}

#[test]
fn allocating_a_document_with_itemised_advances_is_refused() {
    // Each advance references a specific advance invoice sent to a specific
    // recipient; slicing that across N recipients produces deduction tables that
    // match no real document. Silently dropping them would re-bill collected money.
    let doc = full_supply("END-9")
        .with_advances(vec![advance("AB-1", "375.00000", "71.25000")])
        .unwrap();

    let err = EqualAllocation::new(3).unwrap().allocate(&doc).unwrap_err();
    assert!(err.to_string().contains("advance"), "{err}");

    // Without advances the same document allocates fine.
    assert!(
        EqualAllocation::new(3)
            .unwrap()
            .allocate(&full_supply("END-10"))
            .is_ok()
    );
}

#[test]
fn cash_rounding_applies_to_the_remainder_after_advances() {
    let doc = full_supply("END-11")
        .with_advances(vec![advance("AB-1", "900.00000", "171.00000")])
        .unwrap()
        .with_cash_rounding(
            billing::CashRounding::new(
                Amount::parse("0.05000").unwrap(),
                RoundingStrategy::MidpointAwayFromZero,
            )
            .unwrap(),
        )
        .unwrap();

    // Payable before rounding: 1190.00 − 1071.00 = 119.00, already a multiple.
    assert_eq!(doc.rounding(), Amount::<5>::ZERO);
    assert_eq!(
        doc.amount_due().unwrap(),
        Amount::parse("119.00000").unwrap()
    );
    doc.assert_valid();
}

#[test]
fn document_kind_defaults_to_commercial_invoice_and_carries_untdid_codes() {
    let doc = full_supply("END-12");
    assert_eq!(doc.meta.kind, DocumentKind::CommercialInvoice);
    assert_eq!(doc.meta.kind.code(), 380);

    // A final invoice and a residual invoice are BOTH 380 — the type code does not
    // distinguish them; the presence of advances does.
    let final_invoice = full_supply("END-13")
        .with_advances(vec![advance("AB-1", "100.00000", "19.00000")])
        .unwrap();
    assert_eq!(final_invoice.meta.kind.code(), 380);
    assert!(!final_invoice.advances().is_empty());

    // The construction chain has purpose-built codes.
    assert_eq!(DocumentKind::PartialConstructionInvoice.code(), 875);
    assert_eq!(DocumentKind::FinalConstructionInvoice.code(), 877);
    assert_eq!(DocumentKind::PrepaymentInvoice.code(), 386);
}

#[test]
fn prepayment_is_none_distinguishes_nothing_paid_from_nothing_itemised() {
    // `Itemised(vec![])` and `Total(0)` both look empty from the outside, but
    // neither is `None`: they say "advances were considered and came to nothing",
    // which is what decides whether BT-113 appears on the document at all.
    assert!(Prepayment::None.is_none());
    assert!(Prepayment::default().is_none());

    assert!(
        !Prepayment::total_of(Amount::parse("100.00000").unwrap())
            .unwrap()
            .is_none()
    );
    assert!(
        !Prepayment::total_of(Amount::parse("0.00000").unwrap())
            .unwrap()
            .is_none()
    );

    let itemised = Prepayment::itemised(vec![advance("AB-1", "100.00000", "19.00000")]).unwrap();
    assert!(!itemised.is_none());
}

#[test]
fn residual_breakdown_rejects_over_deduction_of_either_base_or_tax() {
    // The two halves of a breakdown group are checked independently. An advance
    // that stays within the supply's net but overstates the tax it contained
    // still understates output tax on the residual — and it is the likelier
    // mistake of the two, because the net is what a human eyeballs.
    let full = vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("1000.00000").unwrap(),
        Amount::parse("190.00000").unwrap(),
    )];

    // Both halves over-deduct together, because BR-CO-17 ties the tax to the base
    // at a positive rate.
    let over = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("1500.00000").unwrap(),
        Amount::parse("285.00000").unwrap(),
    )])
    .unwrap();
    assert!(residual_breakdown(&full, &[over]).is_err());

    // A zero-rated group is where the two halves come apart: the tax is pinned at
    // zero on both sides, so it never goes negative and the base is the only thing
    // left that can. Checking the pair jointly instead of independently would let
    // this through and hand back a residual with a negative taxable base — BR-Z-08
    // reported as a negative supply.
    let zero_rated = vec![TaxBreakdownEntry::new(
        TaxCategory::ZeroRated,
        Decimal::ZERO,
        Amount::parse("100.00000").unwrap(),
        Amount::parse("0.00000").unwrap(),
    )];
    let too_much = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::ZeroRated,
        Decimal::ZERO,
        Amount::parse("200.00000").unwrap(),
        Amount::parse("0.00000").unwrap(),
    )])
    .unwrap();
    assert!(residual_breakdown(&zero_rated, &[too_much]).is_err());

    // Both within the supply — the deduction stands.
    let ok = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("500.00000").unwrap(),
        Amount::parse("95.00000").unwrap(),
    )])
    .unwrap();
    let residual = residual_breakdown(&full, &[ok]).unwrap();
    assert_eq!(residual.len(), 1);
    assert_eq!(
        residual[0].taxable_base,
        Amount::parse("500.00000").unwrap()
    );
    assert_eq!(residual[0].tax_amount, Amount::parse("95.00000").unwrap());
}

#[test]
fn residual_breakdown_drops_only_groups_that_are_zero_on_both_halves() {
    // A zero-rated group has a real taxable base and no tax by construction.
    // Dropping a group the moment *either* half reaches zero would delete it from
    // the breakdown, and BR-Z-08 requires a zero-rated supply to be reported with
    // its base — a residual invoice that omits it is not a lawful invoice.
    let full = vec![
        TaxBreakdownEntry::new(
            TaxCategory::Standard,
            dec!(0.19),
            Amount::parse("1000.00000").unwrap(),
            Amount::parse("190.00000").unwrap(),
        ),
        TaxBreakdownEntry::new(
            TaxCategory::ZeroRated,
            Decimal::ZERO,
            Amount::parse("400.00000").unwrap(),
            Amount::parse("0.00000").unwrap(),
        ),
    ];

    // Settle the standard-rated group completely; leave the zero-rated one alone.
    let paid = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::Standard,
        dec!(0.19),
        Amount::parse("1000.00000").unwrap(),
        Amount::parse("190.00000").unwrap(),
    )])
    .unwrap();

    let residual = residual_breakdown(&full, &[paid]).unwrap();

    // The fully settled group goes; the untaxed-but-non-zero one stays.
    assert_eq!(residual.len(), 1);
    assert_eq!(residual[0].category, TaxCategory::ZeroRated);
    assert_eq!(
        residual[0].taxable_base,
        Amount::parse("400.00000").unwrap()
    );
    assert!(residual[0].tax_amount.is_zero());

    // Settling that one too empties the breakdown entirely.
    let rest = AdvancePayment::new(vec![TaxBreakdownEntry::new(
        TaxCategory::ZeroRated,
        Decimal::ZERO,
        Amount::parse("400.00000").unwrap(),
        Amount::parse("0.00000").unwrap(),
    )])
    .unwrap();
    assert!(residual_breakdown(&residual, &[rest]).unwrap().is_empty());
}

#[test]
fn document_kind_displays_as_its_untdid_code_and_honours_format_specs() {
    // `Display` renders the code, not the variant name — an e-invoicing profile
    // writes BT-3 as the number, so `format!("{kind}")` has to be usable directly.
    assert_eq!(DocumentKind::CommercialInvoice.to_string(), "380");
    assert_eq!(DocumentKind::CreditNote.to_string(), "381");
    assert_eq!(DocumentKind::FinalConstructionInvoice.to_string(), "877");

    // The impl routes through `Formatter::pad` rather than writing directly, so
    // width, fill and alignment work. A hand-rolled `write!` would silently
    // ignore all three, which is exactly how fixed-width EDIFACT output breaks.
    assert_eq!(format!("{:>6}", DocumentKind::CommercialInvoice), "   380");
    assert_eq!(
        format!("{:<6}|", DocumentKind::CommercialInvoice),
        "380   |"
    );
    assert_eq!(format!("{:0>6}", DocumentKind::PrepaymentInvoice), "000386");
    assert_eq!(format!("{:^7}|", DocumentKind::CreditNote), "  381  |");

    // Every variant round-trips through its rendered code.
    for kind in DocumentKind::ALL {
        assert_eq!(kind.to_string(), kind.code().to_string());
    }
}

#[test]
fn an_advance_carries_its_invoice_reference_date_and_date_of_receipt() {
    // BT-25 / BT-26 / BT-X-292. §14 Abs. 5 UStG only obliges deduction where
    // advance invoices *were issued*, so these three fields are the evidence
    // that the deduction on the final invoice is lawful — they have to survive
    // being set, and they have to stay distinct from one another.
    let plain = advance("AB-1", "100.00000", "19.00000");
    assert_eq!(plain.reference(), Some("AB-1"));
    assert_eq!(plain.reference_date(), None);
    assert_eq!(plain.received_on(), None);

    let dated = plain
        .clone()
        .with_reference_date("2026-01-15")
        .with_received_on("2026-01-31");
    assert_eq!(dated.reference(), Some("AB-1"));
    assert_eq!(dated.reference_date(), Some("2026-01-15"));
    assert_eq!(dated.received_on(), Some("2026-01-31"));

    // Setting one must not disturb the others, and the builder must not mutate
    // the value it was called on.
    assert_eq!(plain.reference_date(), None);
    assert_eq!(plain.received_on(), None);
    assert_ne!(dated.reference_date(), dated.received_on());

    // Later calls overwrite rather than accumulate.
    let corrected = dated.with_reference_date("2026-02-01");
    assert_eq!(corrected.reference_date(), Some("2026-02-01"));
    assert_eq!(corrected.received_on(), Some("2026-01-31"));
}

#[cfg(feature = "serde")]
#[test]
fn advance_reference_metadata_survives_a_serde_roundtrip() {
    // `AdvancePayment` deserialises through `try_from`, which rebuilds the value
    // field by field. A field left out of that reconstruction is not a
    // compile error and not a validation failure — it just silently disappears,
    // taking BT-26 with it. Assert each one back individually.
    let doc = full_supply("END-15")
        .with_advances(vec![
            advance("AB-1", "375.00000", "71.25000")
                .with_reference_date("2026-01-15")
                .with_received_on("2026-01-31"),
        ])
        .unwrap();

    let json = serde_json::to_string(&doc).unwrap();
    let back: BillingDocument = serde_json::from_str(&json).unwrap();

    let restored = &back.advances()[0];
    assert_eq!(restored.reference(), Some("AB-1"));
    assert_eq!(restored.reference_date(), Some("2026-01-15"));
    assert_eq!(restored.received_on(), Some("2026-01-31"));
    assert_eq!(back, doc);
}

#[cfg(feature = "serde")]
#[test]
fn advance_reference_metadata_is_optional_on_the_way_in() {
    // The three fields are `#[serde(default)]`, so a payload that omits them
    // must deserialise to `None` rather than fail — older documents and minimal
    // hand-written payloads both rely on it.
    let json = r#"{"tax":[{"category":"Standard","rate":"0.19",
                   "taxable_base":"100.00000","tax_amount":"19.00000"}]}"#;
    let advance: AdvancePayment = serde_json::from_str(json).unwrap();
    assert_eq!(advance.reference(), None);
    assert_eq!(advance.reference_date(), None);
    assert_eq!(advance.received_on(), None);
    assert_eq!(advance.tax().len(), 1);
    assert_eq!(advance.net(), Amount::parse("100.00000").unwrap());
}

#[cfg(feature = "serde")]
#[test]
fn advances_survive_a_serde_roundtrip_and_validate_on_the_way_in() {
    let doc = full_supply("END-14")
        .with_advances(vec![advance("AB-1", "375.00000", "71.25000")])
        .unwrap();
    let json = serde_json::to_string(&doc).unwrap();
    let back: BillingDocument = serde_json::from_str(&json).unwrap();
    assert_eq!(back, doc);
    assert_eq!(back.advances().len(), 1);
    assert_eq!(back.advances()[0].reference(), Some("AB-1"));

    // A tampered BT-113 that no longer matches the advances is rejected.
    let bad = json.replace(r#""prepaid":"446.25000""#, r#""prepaid":"1.00000""#);
    assert_ne!(bad, json);
    assert!(serde_json::from_str::<BillingDocument>(&bad).is_err());
}
