//! Credits drawn down against one document (before and after tax, in priority
//! order), the commitment true-up and the residual VAT statement of a final
//! invoice — every figure of spec 029's user stories as a named test.

use billing::{
    Advance, Amount, AmountScale, Basis, Bound, Credit, CreditApplication, Currency, Decimal,
    Document, DocumentBuilder, ErrorKind, Line, Location, Payment, Percent, Vat,
};
use rust_decimal::dec;

fn a(s: &str) -> Amount {
    Amount::parse(s).unwrap()
}

fn vat(rate: Decimal) -> Vat {
    Vat::standard(Percent::new(rate)).unwrap()
}

fn eur() -> DocumentBuilder {
    Document::builder(Currency::EUR, AmountScale::EN16931)
}

/// 400.00 at 19 % and 100.00 at 7 %.
fn two_rates() -> DocumentBuilder {
    eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("full", dec!(400)))
        .line(Line::fixed("reduced", dec!(100)).with_vat(vat(dec!(7))))
}

/// 100.00 net at 19 % (119.00 gross).
fn hundred() -> DocumentBuilder {
    eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("subscription", dec!(100)))
}

fn app(credit: usize, applied: &str, remaining: &str) -> CreditApplication {
    CreditApplication {
        credit,
        applied: a(applied),
        remaining: a(remaining),
    }
}

/// The credit allowance amounts by VAT rate (the trailing credit adjustments).
fn credit_allowances(doc: &Document) -> Vec<(Option<Decimal>, Amount)> {
    let n = doc.adjustments().len();
    (n - doc.credit_adjustments()..n)
        .map(|i| {
            let rate = doc.adjustments()[i].vat().map(|v| v.rate_percent().value());
            (rate, doc.adjustment_amount(i))
        })
        .collect()
}

// ── US1: before-tax credits ───────────────────────────────────────────────────

#[test]
fn us1_before_tax_credit_reduces_each_base_exactly() {
    let doc = two_rates()
        .credit(Credit::before_tax("contract credit", dec!(50)))
        .build()
        .unwrap();
    let mut parts = credit_allowances(&doc);
    parts.sort();
    assert_eq!(
        parts,
        [(Some(dec!(7)), a("10.00")), (Some(dec!(19)), a("40.00"))]
    );
    assert_eq!(doc.tax_exclusive_total(), a("450.00"));
    assert_eq!(doc.vat_total(), a("74.70")); // 360 × 19 % + 90 × 7 %
    assert_eq!(doc.tax_inclusive_total(), a("524.70"));
    assert_eq!(doc.credit_applications(), [app(0, "50.00", "0.00")]);
}

#[test]
fn us1_before_tax_credit_stops_at_zero() {
    let doc = two_rates()
        .credit(Credit::before_tax("big", dec!(600)))
        .build()
        .unwrap();
    assert_eq!(doc.credit_applications(), [app(0, "500.00", "100.00")]);
    assert_eq!(doc.tax_exclusive_total(), a("0.00"));
    assert_eq!(doc.vat_total(), a("0.00"));
    assert_eq!(doc.payable(), a("0.00"));
}

#[test]
fn us1_invalid_balances_are_refused_at_the_credit() {
    for bad in [dec!(0), dec!(-5), dec!(0.001)] {
        let err = two_rates()
            .credit(Credit::before_tax("ok", dec!(1)))
            .credit(Credit::before_tax("bad", bad))
            .build()
            .unwrap_err();
        assert_eq!(err.kind(), &ErrorKind::InvalidCredit, "{bad}");
        assert_eq!(err.location(), Some(&Location::Credit(1)));
        assert_eq!(err.rule(), None);
    }
    let err = hundred()
        .credit(Credit::after_tax("bad", dec!(0)))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::InvalidCredit);
    assert_eq!(err.location(), Some(&Location::Credit(0)));
}

#[test]
fn us1_vat_free_document_gets_one_allowance_without_vat() {
    let doc = eur()
        .line(Line::fixed("export", dec!(80)))
        .credit(Credit::before_tax("credit", dec!(30)))
        .build()
        .unwrap();
    assert_eq!(credit_allowances(&doc), [(None, a("30.00"))]);
    assert_eq!(doc.tax_exclusive_total(), a("50.00"));
    assert!(doc.vat_groups().is_empty());
}

#[test]
fn us1_uneven_split_sums_to_the_applied_amount() {
    // 1/3 : 2/3 of 0.10 → 0.03 / 0.07 (largest remainder).
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(100)))
        .line(Line::fixed("b", dec!(200)).with_vat(vat(dec!(7))))
        .credit(Credit::before_tax("dime", dec!(0.10)))
        .build()
        .unwrap();
    let parts = credit_allowances(&doc);
    let sum = parts.iter().map(|p| p.1).sum::<Amount>();
    assert_eq!(sum, a("0.10"));
    assert_eq!(doc.tax_exclusive_total(), a("299.90"));
}

// ── US2: after-tax credits ────────────────────────────────────────────────────

#[test]
fn us2_after_tax_credit_is_a_payment() {
    let doc = hundred()
        .credit(Credit::after_tax("wallet", dec!(50)))
        .build()
        .unwrap();
    assert_eq!(doc.vat_total(), a("19.00"));
    assert_eq!(doc.tax_inclusive_total(), a("119.00"));
    assert_eq!(doc.prepaid(), a("50.00"));
    assert_eq!(doc.payable(), a("69.00"));
    assert_eq!(doc.credit_applications(), [app(0, "50.00", "0.00")]);
    assert_eq!(doc.credit_adjustments(), 0);
}

#[test]
fn us2_after_tax_credit_stops_at_payable_zero() {
    let doc = hundred()
        .credit(Credit::after_tax("wallet", dec!(200)))
        .build()
        .unwrap();
    assert_eq!(doc.credit_applications(), [app(0, "119.00", "81.00")]);
    assert_eq!(doc.prepaid(), a("119.00"));
    assert_eq!(doc.payable(), a("0.00"));
}

#[test]
fn us2_prepayment_is_deducted_before_the_credit() {
    let full = hundred()
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(119),
        })
        .credit(Credit::after_tax("wallet", dec!(50)))
        .build()
        .unwrap();
    assert_eq!(full.credit_applications(), [app(0, "0.00", "50.00")]);
    assert_eq!(full.prepaid(), a("119.00"));
    assert_eq!(full.payable(), a("0.00"));
    let part = hundred()
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(100),
        })
        .credit(Credit::after_tax("wallet", dec!(50)))
        .build()
        .unwrap();
    assert_eq!(part.credit_applications(), [app(0, "19.00", "31.00")]);
    assert_eq!(part.prepaid(), a("119.00"));
    assert_eq!(part.payable(), a("0.00"));
}

#[test]
fn us2_cash_rounding_applies_after_credits() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("x", dec!(10.01))) // gross 11.91
        .cash_rounding(billing::CashRounding {
            increment: dec!(0.05),
            rounding: billing::Rounding::HalfAwayFromZero,
        })
        .credit(Credit::after_tax("wallet", dec!(5)))
        .build()
        .unwrap();
    // due 6.91 → 6.90
    assert_eq!(doc.prepaid(), a("5.00"));
    assert_eq!(doc.rounding(), a("-0.01"));
    assert_eq!(doc.payable(), a("6.90"));
}

// ── US3: priority, credit notes, rebuild ──────────────────────────────────────

#[test]
fn us3_credits_draw_in_priority_order() {
    let doc = hundred()
        .credit(Credit::before_tax("A", dec!(30)))
        .credit(Credit::after_tax("B", dec!(100)))
        .build()
        .unwrap();
    assert_eq!(
        doc.credit_applications(),
        [app(0, "30.00", "0.00"), app(1, "83.30", "16.70")]
    );
    assert_eq!(doc.tax_exclusive_total(), a("70.00"));
    assert_eq!(doc.vat_total(), a("13.30"));
    assert_eq!(doc.payable(), a("0.00"));
}

#[test]
fn us3_two_before_tax_credits_share_the_base() {
    let doc = hundred()
        .credit(Credit::before_tax("A", dec!(70)))
        .credit(Credit::before_tax("B", dec!(50)))
        .build()
        .unwrap();
    assert_eq!(
        doc.credit_applications(),
        [app(0, "70.00", "0.00"), app(1, "30.00", "20.00")]
    );
    assert_eq!(doc.tax_exclusive_total(), a("0.00"));
}

#[test]
fn us3_credit_note_draws_nothing() {
    let note = hundred().build().unwrap().negated();
    let doc = note
        .into_builder()
        .credit(Credit::before_tax("A", dec!(30)))
        .credit(Credit::after_tax("B", dec!(100)))
        .build()
        .unwrap();
    assert_eq!(
        doc.credit_applications(),
        [app(0, "0.00", "30.00"), app(1, "0.00", "100.00")]
    );
    assert_eq!(doc.payable(), a("-119.00"));
    assert_eq!(doc.credit_adjustments(), 0);
}

#[test]
fn us3_rebuild_reapplies_credits() {
    let doc = hundred()
        .credit(Credit::before_tax("A", dec!(30)))
        .credit(Credit::after_tax("B", dec!(100)))
        .build()
        .unwrap();
    assert_eq!(doc.clone().into_builder().build().unwrap(), doc);
    let edited = doc
        .into_builder()
        .line(Line::fixed("more", dec!(100)).with_vat(vat(dec!(19))))
        .build()
        .unwrap();
    assert_eq!(edited.adjustments().len(), 1); // no stale credit allowance
    assert_eq!(edited.credits().len(), 2);
    assert_eq!(
        edited.credit_applications(),
        [app(0, "30.00", "0.00"), app(1, "100.00", "0.00")]
    );
    assert_eq!(edited.tax_exclusive_total(), a("170.00"));
    assert_eq!(edited.payable(), a("102.30")); // 202.30 − 100
}

#[test]
fn us3_negated_split_and_concat_materialise_drawn_credits() {
    let doc = hundred()
        .credit(Credit::before_tax("A", dec!(30)))
        .credit(Credit::after_tax("B", dec!(50)))
        .build()
        .unwrap();
    let neg = doc.negated();
    assert!(neg.credits().is_empty());
    assert_eq!(neg.credit_adjustments(), 0);
    assert_eq!(neg.adjustments().len(), 1); // the drawn before-tax credit
    assert_eq!(neg.prepaid(), a("-50.00"));
    assert!(neg.credits().is_empty());
    assert_eq!(
        neg.payments(),
        [Payment::Gross {
            description: "B".into(),
            amount: dec!(-50)
        }]
    );
    assert_eq!(neg.payable(), -doc.payable());
    // The negated document is consistent with its inputs.
    assert_eq!(neg.clone().into_builder().build().unwrap(), neg);
    assert_eq!(neg.negated().payable(), doc.payable());

    let shares = doc.split(&[dec!(1), dec!(1)]).unwrap();
    let prepaid = shares.iter().map(Document::prepaid).sum::<Amount>();
    assert_eq!(prepaid, doc.prepaid());
    let net = shares
        .iter()
        .map(Document::tax_exclusive_total)
        .sum::<Amount>();
    assert_eq!(net, doc.tax_exclusive_total());
    assert!(shares.iter().all(|s| s.credits().is_empty()));

    let joined = Document::concat([doc.clone()]).unwrap();
    assert!(joined.credits().is_empty());
    assert_eq!(joined.payable(), doc.payable());
    assert_eq!(joined.prepaid(), doc.prepaid());
    assert_eq!(joined.vat_total(), doc.vat_total());
}

#[test]
fn us3_advance_with_after_tax_credit_keeps_both_entries() {
    let v = vat(dec!(19));
    let doc = hundred()
        .payments(
            vec![Advance {
                vat: v,
                net: dec!(10),
                tax: dec!(1.90),
            }]
            .into_iter()
            .map(Payment::Advance),
        )
        .credit(Credit::after_tax("B", dec!(5)))
        .build()
        .unwrap();
    assert_eq!(doc.prepaid(), a("16.90"));
    let neg = doc.negated();
    // 030: the advance is kept beside the drawn credit, never folded.
    assert_eq!(
        neg.payments(),
        [
            Payment::Advance(Advance {
                vat: vat(dec!(19)),
                net: dec!(-10),
                tax: dec!(-1.90),
            }),
            Payment::Gross {
                description: "B".into(),
                amount: dec!(-5)
            },
        ]
    );
    assert_eq!(neg.prepaid(), a("-16.90"));
}

// ── US4: commitment true-up ───────────────────────────────────────────────────

#[test]
fn us4_commitment_is_a_net_minimum() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("usage", dec!(600)))
        .bound(Bound::minimum(Basis::Net, dec!(1000), "commitment true-up"))
        .build()
        .unwrap();
    assert_eq!(doc.bound_lines(), 1);
    assert_eq!(doc.line_net(1), a("400.00"));
    assert_eq!(doc.tax_exclusive_total(), a("1000.00"));
}

#[test]
fn us4_bounds_settle_before_credits() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("usage", dec!(600)))
        .bound(Bound::minimum(Basis::Net, dec!(1000), "commitment true-up"))
        .credit(Credit::before_tax("credit", dec!(100)))
        .build()
        .unwrap();
    assert_eq!(doc.line_net(1), a("400.00"));
    assert_eq!(doc.tax_exclusive_total(), a("900.00"));
    assert_eq!(doc.credit_applications(), [app(0, "100.00", "0.00")]);
}

// ── US5: residual VAT statement ───────────────────────────────────────────────

fn final_invoice(advances: Vec<Advance>) -> Document {
    eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("works", dec!(1000)))
        .line(Line::fixed("books", dec!(200)).with_vat(vat(dec!(7))))
        .payments(advances.into_iter().map(Payment::Advance))
        .build()
        .unwrap()
}

fn advance(rate: Decimal, net: Decimal, tax: Decimal) -> Advance {
    Advance {
        vat: vat(rate),
        net,
        tax,
    }
}

#[test]
fn us5_residual_vat_per_rate() {
    let doc = final_invoice(vec![
        advance(dec!(19), dec!(300), dec!(57)),
        advance(dec!(7), dec!(100), dec!(7)),
    ]);
    let before = doc.vat_groups().to_vec();
    let r = doc.residual_vat().unwrap();
    let rows: Vec<_> = r
        .iter()
        .map(|g| (g.rate_percent().value(), g.taxable_base(), g.tax()))
        .collect();
    // Document VAT group order: S 7 %, then S 19 %.
    assert_eq!(
        rows,
        [
            (dec!(7), a("100.00"), a("7.00")),
            (dec!(19), a("700.00"), a("133.00"))
        ]
    );
    assert_eq!(doc.vat_groups(), before.as_slice());
    assert_eq!(doc.vat_total(), a("204.00"));
}

#[test]
fn us5_unmatched_advance_is_refused_at_its_index() {
    let doc = final_invoice(vec![
        advance(dec!(19), dec!(300), dec!(57)),
        advance(dec!(16), dec!(100), dec!(16)),
    ]);
    let err = doc.residual_vat().unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::UnmatchedAdvance);
    assert_eq!(err.location(), Some(&Location::Payment(1)));
    assert_eq!(err.rule(), None);
}

#[test]
fn us5_without_itemised_advances_the_statement_is_the_groups() {
    let doc = final_invoice(Vec::new());
    assert_eq!(doc.residual_vat().unwrap(), doc.vat_groups());
    let total = two_rates()
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(100),
        })
        .build()
        .unwrap();
    assert_eq!(total.residual_vat().unwrap(), total.vat_groups());
    let none = two_rates().build().unwrap();
    assert_eq!(none.residual_vat().unwrap(), none.vat_groups());
}

// ── serde ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
#[test]
fn serde_credits_are_inputs_and_reapplied() {
    use billing::CreditPlacement;
    let doc = two_rates()
        .credit(Credit::before_tax("A", dec!(50)))
        .credit(Credit::after_tax("B", dec!(20)))
        .build()
        .unwrap();
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("\"credits\""));
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, doc);
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    // The materialised allowances are not serialised.
    let plain = two_rates().build().unwrap();
    let plain_json = serde_json::to_string(&plain).unwrap();
    assert!(!plain_json.contains("credits"));
    for t in [CreditPlacement::BeforeTax, CreditPlacement::AfterTax] {
        let j = serde_json::to_string(&t).unwrap();
        assert_eq!(serde_json::from_str::<CreditPlacement>(&j).unwrap(), t);
    }
    let app = doc.credit_applications()[0];
    let j = serde_json::to_string(&app).unwrap();
    assert_eq!(serde_json::from_str::<CreditApplication>(&j).unwrap(), app);
}

#[cfg(feature = "serde")]
#[test]
fn serde_invalid_credit_is_refused() {
    let doc = hundred()
        .credit(Credit::after_tax("B", dec!(20)))
        .build()
        .unwrap();
    let json = serde_json::to_string(&doc).unwrap();
    let bad = json.replace("\"20\"", "\"-20\"");
    assert_ne!(bad, json);
    let err = serde_json::from_str::<Document>(&bad).unwrap_err();
    assert!(err.to_string().contains("credit 0"), "{err}");
}

// ── 030 US2: lossless settlement ──────────────────────────────────────────────

/// 1 000.00 @ 19 % and 200.00 @ 7 %, advances 300/57 and 100/7, and an
/// after-tax credit of 20.00 drawn.
fn final_with_credit() -> Document {
    eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("works", dec!(1000)))
        .line(Line::fixed("books", dec!(200)).with_vat(vat(dec!(7))))
        .payment(Payment::Advance(advance(dec!(19), dec!(300), dec!(57))))
        .payment(Payment::Advance(advance(dec!(7), dec!(100), dec!(7))))
        .credit(Credit::after_tax("wallet", dec!(20)))
        .build()
        .unwrap()
}

/// (rate, base, tax) per residual row, in VAT group order (7 %, then 19 %).
fn residual(doc: &Document) -> Vec<(Decimal, Amount, Amount)> {
    doc.residual_vat()
        .unwrap()
        .iter()
        .map(|g| (g.rate_percent().value(), g.taxable_base(), g.tax()))
        .collect()
}

fn gross(description: &str, amount: Decimal) -> Payment {
    Payment::Gross {
        description: description.into(),
        amount,
    }
}

#[test]
fn us030_payments_entries_and_residual() {
    let doc = final_with_credit();
    assert_eq!(doc.payments().len(), 2, "inputs: the two advances");
    assert_eq!(
        doc.paid_entries(),
        [
            Payment::Advance(advance(dec!(19), dec!(300), dec!(57))),
            Payment::Advance(advance(dec!(7), dec!(100), dec!(7))),
            gross("wallet", dec!(20)),
        ]
    );
    assert_eq!(doc.prepaid(), a("484.00")); // 357 + 107 + 20
    assert_eq!(
        residual(&doc),
        [
            (dec!(7), a("100.00"), a("7.00")),
            (dec!(19), a("700.00"), a("133.00"))
        ]
    );
}

#[test]
fn us030_payments_split_partitions_every_entry() {
    let doc = final_with_credit();
    let halves = doc.split(&[dec!(1), dec!(1)]).unwrap();
    for h in &halves {
        assert_eq!(
            h.paid_entries(),
            [
                Payment::Advance(advance(dec!(19), dec!(150), dec!(28.50))),
                Payment::Advance(advance(dec!(7), dec!(50), dec!(3.50))),
                gross("wallet", dec!(10)),
            ]
        );
        assert_eq!(
            residual(h),
            [
                (dec!(7), a("50.00"), a("3.50")),
                (dec!(19), a("350.00"), a("66.50"))
            ]
        );
    }
    let prepaid: Amount = halves.iter().map(Document::prepaid).sum();
    assert_eq!(prepaid, doc.prepaid());
}

#[test]
fn us030_payments_negated_residual_and_identity() {
    let doc = final_with_credit();
    let neg = doc.negated();
    assert_eq!(
        neg.paid_entries(),
        [
            Payment::Advance(advance(dec!(19), dec!(-300), dec!(-57))),
            Payment::Advance(advance(dec!(7), dec!(-100), dec!(-7))),
            gross("wallet", dec!(-20)),
        ]
    );
    assert_eq!(
        residual(&neg),
        [
            (dec!(7), a("-100.00"), a("-7.00")),
            (dec!(19), a("-700.00"), a("-133.00"))
        ]
    );
    assert_eq!(neg.prepaid(), -doc.prepaid());
    assert_eq!(neg.payable(), -doc.payable());
    let twice = neg.negated();
    assert_eq!(twice.paid_entries(), doc.paid_entries());
    assert_eq!(residual(&twice), residual(&doc));
    assert_eq!(twice.payable(), doc.payable());
    assert_eq!(twice.negated(), neg);
}

#[test]
fn us030_payments_concat_keeps_every_entry() {
    let doc = final_with_credit();
    let joined = Document::concat([doc.clone(), doc.clone()]).unwrap();
    let mut want = doc.paid_entries();
    want.extend(doc.paid_entries());
    assert_eq!(joined.paid_entries(), want);
    assert_eq!(joined.payments(), want.as_slice());
    assert_eq!(
        residual(&joined),
        [
            (dec!(7), a("200.00"), a("14.00")),
            (dec!(19), a("1400.00"), a("266.00"))
        ]
    );
    assert_eq!(joined.prepaid(), a("968.00"));
}

#[test]
fn us030_payments_gross_and_advances_mix_and_are_located() {
    // A gross payment carries no VAT: it never touches the residual.
    let doc = final_invoice(Vec::new())
        .into_builder()
        .payment(gross("transfer", dec!(100)))
        .payment(Payment::Advance(advance(dec!(19), dec!(300), dec!(57))))
        .build()
        .unwrap();
    assert_eq!(doc.prepaid(), a("457.00"));
    assert_eq!(residual(&doc)[1], (dec!(19), a("700.00"), a("133.00")));
    // Excess precision is refused at the entry's index in the payment list.
    let err = two_rates()
        .payment(gross("ok", dec!(1)))
        .payment(gross("cents", dec!(0.001)))
        .build()
        .unwrap_err();
    assert!(matches!(err.kind(), ErrorKind::ExcessPrecision { .. }));
    assert_eq!(err.location(), Some(&Location::Payment(1)));
    let err = two_rates()
        .payment(Payment::Advance(advance(dec!(19), dec!(1.001), dec!(0))))
        .build()
        .unwrap_err();
    assert_eq!(err.location(), Some(&Location::Payment(0)));
    // An unmatched advance is located likewise; gross entries count.
    let doc = final_invoice(Vec::new())
        .into_builder()
        .payment(gross("transfer", dec!(1)))
        .payment(Payment::Advance(advance(dec!(16), dec!(100), dec!(16))))
        .build()
        .unwrap();
    let err = doc.residual_vat().unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::UnmatchedAdvance);
    assert_eq!(err.location(), Some(&Location::Payment(1)));
    assert_eq!(Location::Payment(1).to_string(), "payment 1");
}

#[cfg(feature = "serde")]
#[test]
fn us030_payments_serialise_as_inputs() {
    let doc = final_with_credit();
    let json = serde_json::to_string(&doc).unwrap();
    assert!(json.contains("\"payments\""), "{json}");
    let back: Document = serde_json::from_str(&json).unwrap();
    assert_eq!(back, doc);
    for p in doc.paid_entries() {
        let j = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Payment>(&j).unwrap(), p);
    }
    let neg = doc.negated();
    let back: Document = serde_json::from_str(&serde_json::to_string(&neg).unwrap()).unwrap();
    assert_eq!(back, neg);
}
