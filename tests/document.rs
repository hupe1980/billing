//! Golden EN 16931 documents and every reproduced arithmetic defect as a named
//! regression, each with the correct number.

use billing::{
    Adjust, AdjustmentKind, Advance, Amount, AmountScale, Basis, Bound, BoundKind, CashRounding,
    Currency, Decimal, Document, DocumentAdjustment, DocumentBuilder, EarlyPaymentDiscount,
    ErrorKind, Exemption, InterestSegment, Line, LineAdjustment, Location, Payment, Percent, Price,
    Quantity, Rounding, Tariff, TaxCategory, Tier, Tiers, UnitPrice, Vat, VatAccounting, VatError,
    apportion, late_payment_interest, per_unit_levy, rate_series,
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

fn usage(desc: &str, q: Decimal, unit: &str, price: Decimal) -> Line {
    Line::new(
        desc,
        Quantity::new(q, unit),
        UnitPrice::new(Price::new(price)),
    )
}

// ── VAT as an attribute ───────────────────────────────────────────────────────

/// A 10 % discount over 400 @ 19 % and 100 @ 7 % must lower both bases. Leaving
/// the discount outside the VAT bases gives VAT 83.00; the right figure is 74.70.
#[test]
fn mixed_rate_discount_via_apportion_is_taxed_per_group() {
    let s = AmountScale::EN16931;
    let lines = [
        Line::fixed("full", dec!(400)).with_vat(vat(dec!(19))),
        Line::fixed("reduced", dec!(100)).with_vat(vat(dec!(7))),
    ];
    let discount = apportion(
        AdjustmentKind::Allowance,
        "10 % discount",
        Adjust::Percent(Percent::new(dec!(10))),
        &lines,
        |_| true,
        s,
    )
    .unwrap();
    assert_eq!(discount.len(), 2);
    assert_eq!(
        discount[0].basis(),
        Some((dec!(400), Percent::new(dec!(10))))
    );
    let doc = eur().lines(lines).adjustments(discount).build().unwrap();
    assert_eq!(doc.vat_total(), a("74.70"));
    assert_eq!(doc.tax_exclusive_total(), a("450"));
    assert_eq!(doc.vat_groups()[0].taxable_base(), a("90")); // 7 % sorts by rate
    assert_eq!(doc.vat_groups()[1].taxable_base(), a("360"));
}

/// The same discount as a builder input: no pre-stamped VAT, no cloned lines.
/// One line takes the default VAT, the other states its own; the builder
/// materialises one allowance per group, exactly as the free `apportion` does.
#[test]
fn whole_document_apportion_is_taxed_per_group() {
    let build = || {
        eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("full", dec!(400)))
            .line(Line::fixed("reduced", dec!(100)).with_vat(vat(dec!(7))))
            .apportion(
                AdjustmentKind::Allowance,
                "Rabatt",
                Adjust::Percent(Percent::new(dec!(10))),
            )
    };
    let doc = build().build().unwrap();
    assert_eq!(doc.vat_total(), a("74.70"));
    assert_eq!(doc.tax_exclusive_total(), a("450"));
    assert_eq!(doc.allowance_total(), a("50"));
    assert_eq!(doc.apportioned_adjustments(), 2);
    type Made<'a> = (Decimal, Option<(Decimal, Percent)>, Option<&'a Vat>);
    let made: Vec<Made<'_>> = doc
        .adjustments()
        .iter()
        .map(|d| (d.amount(), d.basis(), d.vat()))
        .collect();
    let (v19, v7) = (vat(dec!(19)), vat(dec!(7)));
    assert_eq!(
        made,
        [
            (
                dec!(40),
                Some((dec!(400), Percent::new(dec!(10)))),
                Some(&v19)
            ),
            (
                dec!(10),
                Some((dec!(100), Percent::new(dec!(10)))),
                Some(&v7)
            ),
        ]
    );
    // Identical to the free function over every line, VAT stamped by hand.
    let lines = [
        Line::fixed("full", dec!(400)).with_vat(v19.clone()),
        Line::fixed("reduced", dec!(100)).with_vat(v7.clone()),
    ];
    let free = apportion(
        AdjustmentKind::Allowance,
        "Rabatt",
        Adjust::Percent(Percent::new(dec!(10))),
        &lines,
        |_| true,
        AmountScale::EN16931,
    )
    .unwrap();
    assert_eq!(doc.adjustments(), free.as_slice());
    // Caller adjustments come first; the builder call order does not matter.
    let with_fee = build()
        .adjustment(DocumentAdjustment::charge("fee", dec!(5)))
        .build()
        .unwrap();
    let reordered = eur()
        .apportion(
            AdjustmentKind::Allowance,
            "Rabatt",
            Adjust::Percent(Percent::new(dec!(10))),
        )
        .adjustment(DocumentAdjustment::charge("fee", dec!(5)))
        .line(Line::fixed("reduced", dec!(100)).with_vat(vat(dec!(7))))
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("full", dec!(400)))
        .build()
        .unwrap();
    assert_eq!(with_fee.adjustments()[0].description(), "fee");
    assert_eq!(with_fee.apportioned_adjustments(), 2);
    assert_eq!(with_fee.lines().len(), 2);
    assert_eq!(reordered.vat_total(), with_fee.vat_total());
    // into_builder keeps the input, not its output: rebuilding is idempotent,
    // and an edited document re-apportions over the new lines.
    let again = doc.clone().into_builder().build().unwrap();
    assert_eq!(again, doc);
    assert_eq!(again.adjustments().len(), 2);
    let edited = doc
        .clone()
        .into_builder()
        .line(Line::fixed("more", dec!(100)).with_vat(vat(dec!(7))))
        .build()
        .unwrap();
    assert_eq!(edited.allowance_total(), a("60"));
    assert_eq!(edited.adjustments()[1].amount(), dec!(20));
    // Transforms turn the materialised adjustments into ordinary ones.
    let n = doc.negated();
    assert_eq!(n.apportioned_adjustments(), 0);
    assert_eq!(n.vat_total(), a("-74.70"));
    assert_eq!(n.clone().into_builder().build().unwrap(), n);
    let c = Document::concat([doc.clone()]).unwrap();
    assert_eq!((c.apportioned_adjustments(), c.adjustments().len()), (0, 2));
    assert_eq!(c.vat_total(), doc.vat_total());
    for share in doc.split(&[dec!(1), dec!(2)]).unwrap() {
        assert_eq!(share.apportioned_adjustments(), 0);
    }
    // A fixed amount over mixed-sign group bases is refused at build.
    let err = eur()
        .line(Line::fixed("a", dec!(1)).with_vat(vat(dec!(19))))
        .line(Line::fixed("b", dec!(-1)).with_vat(vat(dec!(7))))
        .apportion(AdjustmentKind::Charge, "x", Adjust::Fixed(dec!(1)))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::InvalidWeights);
}

/// The same case at a single rate: 68.40, not 76.00.
#[test]
fn single_rate_tagged_discount_enters_the_vat_base() {
    let lines = [Line::fixed("energy", dec!(400))];
    let discount = apportion(
        AdjustmentKind::Allowance,
        "10 %",
        Adjust::Percent(Percent::new(dec!(10))),
        &lines,
        |_| true,
        AmountScale::EN16931,
    )
    .unwrap();
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .lines(lines)
        .adjustments(discount)
        .build()
        .unwrap();
    assert_eq!(doc.vat_total(), a("68.40"));
    assert_eq!(doc.payable(), a("428.40"));
}

/// A fixed amount is split by the group bases, exactly.
#[test]
fn apportion_fixed_splits_exactly() {
    let lines = [
        Line::fixed("a", dec!(400)).with_vat(vat(dec!(19))),
        Line::fixed("b", dec!(100)).with_vat(vat(dec!(7))),
        Line::fixed("c", dec!(99)).with_vat(vat(dec!(7))),
    ];
    let d = apportion(
        AdjustmentKind::Allowance,
        "voucher",
        Adjust::Fixed(dec!(10.01)),
        &lines,
        |l| l.description() != "c",
        AmountScale::EN16931,
    )
    .unwrap();
    let amounts: Vec<Decimal> = d.iter().map(DocumentAdjustment::amount).collect();
    assert_eq!(amounts, [dec!(8.01), dec!(2)]);
    let mixed = [
        Line::fixed("a", dec!(1)).with_vat(vat(dec!(19))),
        Line::fixed("b", dec!(-1)).with_vat(vat(dec!(7))),
    ];
    let err = apportion(
        AdjustmentKind::Allowance,
        "x",
        Adjust::Fixed(dec!(1)),
        &mixed,
        |_| true,
        AmountScale::EN16931,
    )
    .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::InvalidWeights);
}

/// VAT declared on each line (no default) is computed, not dropped to zero.
#[test]
fn line_level_vat_computes_vat() {
    let doc = eur()
        .line(Line::fixed("a", dec!(400)).with_vat(vat(dec!(19))))
        .line(Line::fixed("b", dec!(100)).with_vat(vat(dec!(7))))
        .build()
        .unwrap();
    assert_eq!(doc.vat_total(), a("83.00"));
    assert_eq!(doc.vat_groups().len(), 2);
    assert_eq!(doc.tax_inclusive_total(), a("583.00"));
}

/// Stromsteuer is in the VAT base (§ 10 UStG): 60.90, not 57.00.
#[test]
fn per_unit_levy_line_is_taxed() {
    let energy = [usage("energy", dec!(1000), "KWH", dec!(0.30))];
    let levy = per_unit_levy(
        "Stromsteuer",
        &energy,
        |_| true,
        UnitPrice::new(Price::new(dec!(0.0205))).with_unit("KWH"),
        "KWH",
    )
    .unwrap();
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .lines(energy)
        .line(levy)
        .build()
        .unwrap();
    assert_eq!(doc.line_net(1), a("20.50"));
    assert_eq!(doc.vat_total(), a("60.90"));
    let wrong_unit = per_unit_levy(
        "x",
        &[usage("gas", dec!(1), "MTQ", dec!(1))],
        |_| true,
        UnitPrice::new(Price::new(dec!(1))),
        "KWH",
    )
    .unwrap_err();
    assert_eq!(wrong_unit.location(), Some(&Location::Line(0)));
}

/// D-7: the free helpers are eager. After an edit via `into_builder`, the
/// caller re-runs `per_unit_levy` over the edited lines: 41.00, not a stale
/// 20.50.
#[test]
fn levy_is_re_run_after_an_edit() {
    let rate = || UnitPrice::new(Price::new(dec!(0.0205))).with_unit("KWH");
    let energy = [usage("energy", dec!(1000), "KWH", dec!(0.30))];
    let levy = per_unit_levy("Stromsteuer", &energy, |_| true, rate(), "KWH").unwrap();
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .lines(energy)
        .line(levy)
        .build()
        .unwrap();
    assert_eq!(doc.line_net(1), a("20.50"));
    let mut lines: Vec<Line> = doc
        .lines()
        .iter()
        .filter(|l| l.description() != "Stromsteuer")
        .cloned()
        .collect();
    lines.push(usage("energy 2", dec!(1000), "KWH", dec!(0.30)).with_vat(vat(dec!(19))));
    let levy = per_unit_levy("Stromsteuer", &lines, |_| true, rate(), "KWH").unwrap();
    let edited = eur()
        .default_vat(vat(dec!(19)))
        .lines(lines)
        .line(levy)
        .build()
        .unwrap();
    assert_eq!(edited.line_net(2), a("41.00"));
    assert_eq!(edited.tax_exclusive_total(), a("641.00"));
}

/// L5: a commission is a share of the selected lines' net, never of the
/// VAT-inclusive amount: 10.00 on a 100.00 sale, not 11.90.
#[test]
fn commission_is_on_the_net_not_the_vat() {
    let lines = [Line::fixed("sale", dec!(100)).with_vat(vat(dec!(19)))];
    let commission = apportion(
        AdjustmentKind::Charge,
        "commission",
        Adjust::Percent(Percent::new(dec!(10))),
        &lines,
        |_| true,
        AmountScale::EN16931,
    )
    .unwrap();
    assert_eq!(commission.len(), 1);
    assert_eq!(commission[0].amount(), dec!(10));
    let doc = eur().lines(lines).adjustments(commission).build().unwrap();
    assert_eq!(doc.charge_total(), a("10.00"));
    assert_eq!(doc.tax_exclusive_total(), a("110.00"));
}

/// L7: labels are never read. A line tagged `{"tax": "x"}` builds to the
/// same figures as the untagged line (it stays in the VAT base).
#[test]
fn labels_are_ignored_by_the_engine() {
    let plain = || {
        usage("energy", dec!(1000), "KWH", dec!(0.30))
            .with_adjustment(LineAdjustment::allowance(dec!(5)))
            .with_vat(vat(dec!(19)))
    };
    let build = |l: Line| {
        eur()
            .line(l)
            .line(Line::fixed("levy", dec!(20.50)).with_vat(vat(dec!(19))))
            .build()
            .unwrap()
    };
    let untagged = build(plain());
    let tagged = build(plain().with_label("tax", "x"));
    assert_eq!(tagged.lines()[0].label("tax"), Some("x"));
    assert_eq!(tagged.line_net(0), untagged.line_net(0));
    assert_eq!(tagged.vat_groups(), untagged.vat_groups());
    assert_eq!(tagged.vat_total(), a("59.95")); // (295.00 + 20.50) × 19 % = 59.945
    assert_eq!(tagged.payable(), untagged.payable());
    // Only the label differs.
    let mut stripped = tagged.lines().to_vec();
    stripped[0] = plain();
    assert_eq!(eur().lines(stripped).build().unwrap(), untagged);
}

/// PR-1: every one of the 6 call orders of (line 100, line 10,
/// `default_vat(S 19)`) gives net 110.00 and VAT 20.90.
#[test]
fn every_call_order_gives_one_document() {
    let ops: [fn(DocumentBuilder) -> DocumentBuilder; 3] = [
        |b| b.line(Line::fixed("a", dec!(100))),
        |b| b.line(Line::fixed("b", dec!(10))),
        |b| b.default_vat(vat(dec!(19))),
    ];
    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    for order in orders {
        let doc = order.iter().fold(eur(), |b, &i| ops[i](b)).build().unwrap();
        assert_eq!(doc.tax_exclusive_total(), a("110.00"), "{order:?}");
        assert_eq!(doc.vat_total(), a("20.90"), "{order:?}");
        assert_eq!(doc.vat_groups().len(), 1, "{order:?}");
    }
}

/// Two VAT "sources" at one rate (an explicit line VAT and the default) form
/// one group and round once: 0.01, not 0.02.
#[test]
fn two_vat_sources_at_one_rate_round_once() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(0.03)).with_vat(vat(dec!(19.00))))
        .line(Line::fixed("b", dec!(0.03)))
        .build()
        .unwrap();
    assert_eq!(doc.vat_groups().len(), 1);
    assert_eq!(doc.vat_total(), a("0.01"));
}

/// A leaf is rounded once from its exact value: 10.00 × 0.1004995 =
/// 1.004995 → 1.00 (the old engine rounded to 5 dp first: 1.01).
#[test]
fn every_leaf_rounds_once() {
    let doc = eur()
        .line(usage("x", dec!(10), "C62", dec!(0.1004995)))
        .adjustment(DocumentAdjustment::charge("c", dec!(1.004995)))
        .adjustment(DocumentAdjustment::allowance("d", dec!(1.004995)))
        .build()
        .unwrap();
    assert_eq!(doc.line_net(0), a("1.00"));
    assert_eq!(doc.charge_total(), a("1.00"));
    assert_eq!(doc.allowance_total(), a("1.00"));
}

/// BT-131 = reduce(price part) + Σ reduce(charges) − Σ reduce(allowances),
/// each a leaf; the price part is the exact rational q × p / base (base 3).
#[test]
fn line_net_reduces_each_leaf() {
    let line = Line::new(
        "x",
        Quantity::new(dec!(1), "C62"),
        UnitPrice::new(Price::new(dec!(1))).with_base_quantity(dec!(3)),
    )
    .with_adjustment(LineAdjustment::charge(dec!(0.005)))
    .with_adjustment(LineAdjustment::allowance(dec!(0.004)));
    let doc = eur().line(line).build().unwrap();
    // 0.33 + 0.01 − 0.00
    assert_eq!(doc.line_net(0), a("0.34"));
    assert_eq!(doc.line_adjustment_amounts(0), [a("0.01"), a("0.00")]);
}

#[test]
fn out_of_scope_and_exemptions() {
    let o = Vat::exempt(
        TaxCategory::OutOfScope,
        Exemption::Text("not subject".into()),
    )
    .unwrap();
    let doc = eur()
        .line(Line::fixed("x", dec!(10)).with_vat(o.clone()))
        .build()
        .unwrap();
    assert_eq!(doc.vat_groups()[0].tax(), Amount::ZERO);
    assert_eq!(doc.vat_groups()[0].vat().stated_rate(), None);
    // BR-O-11
    let err = eur()
        .line(Line::fixed("x", dec!(10)).with_vat(o))
        .line(Line::fixed("y", dec!(10)).with_vat(vat(dec!(19))))
        .build()
        .unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::InvalidVat(VatError::OutOfScopeMixed)
    );
    assert_eq!(err.rule(), Some("BR-O-11"));
    // BR-B-02
    let b = Vat::regional(TaxCategory::SplitPayment, Percent::new(dec!(22))).unwrap();
    let err = eur()
        .line(Line::fixed("x", dec!(10)).with_vat(b))
        .line(Line::fixed("y", dec!(10)).with_vat(vat(dec!(22))))
        .build()
        .unwrap_err();
    assert_eq!(err.rule(), Some("BR-B-02"));
    // Groups are keyed by (category, rate): two reasons in one E group conflict.
    let e1 = Vat::exempt(TaxCategory::Exempt, Exemption::Text("§4 Nr. 8".into())).unwrap();
    let e2 = Vat::exempt(TaxCategory::Exempt, Exemption::Text("§4 Nr. 14".into())).unwrap();
    let err = eur()
        .line(Line::fixed("x", dec!(10)).with_vat(e1))
        .line(Line::fixed("y", dec!(10)).with_vat(e2))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::ConflictingExemption);
    assert_eq!(err.location(), Some(&Location::Line(1)));
}

// ── Builder and leaf validation ───────────────────────────────────────────────

/// Every builder method appends or sets a distinct field, so the order of
/// calls never changes the document.
#[test]
fn builder_is_order_independent() {
    let s = vat(dec!(19));
    let l1 = usage("a", dec!(3), "C62", dec!(1.99));
    let l2 = Line::fixed("b", dec!(5));
    let adj = DocumentAdjustment::allowance("d", dec!(1));
    let cash = CashRounding {
        increment: dec!(0.05),
        rounding: Rounding::HalfAwayFromZero,
    };
    let acct = VatAccounting {
        currency: Currency::new("PLN").unwrap(),
        rate: dec!(4.31),
    };
    let min = Bound::minimum(Basis::Gross, dec!(20), "minimum");
    let x = eur()
        .default_vat(s.clone())
        .line(l1.clone())
        .adjustment(adj.clone())
        .line(l2.clone())
        .bound(min.clone())
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(1),
        })
        .cash_rounding(cash)
        .vat_accounting(acct)
        .build()
        .unwrap();
    let y = eur()
        .vat_accounting(acct)
        .cash_rounding(cash)
        .bound(min)
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(1),
        })
        .line(l1)
        .line(l2)
        .adjustment(adj)
        .default_vat(s)
        .build()
        .unwrap();
    assert_eq!(x, y);
    assert_eq!(x.tax_inclusive_total(), a("20.00"));
    // A single-valued setting given twice is refused, whatever the order.
    let err = eur()
        .cash_rounding(cash)
        .cash_rounding(CashRounding {
            increment: dec!(0.1),
            ..cash
        })
        .build()
        .unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::ConflictingSetting {
            setting: "cash_rounding"
        }
    );
    // Payments append (030): two are two entries, not a conflict.
    let two = eur()
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(1),
        })
        .line(Line::fixed("x", dec!(1)))
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(1),
        })
        .build()
        .unwrap();
    assert_eq!(two.payments().len(), 2);
    assert_eq!(two.prepaid(), a("2"));
    let err = eur()
        .default_vat(vat(dec!(19)))
        .default_vat(vat(dec!(19)))
        .build()
        .unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::ConflictingSetting {
            setting: "default_vat"
        }
    );
}

#[test]
fn leaves_are_validated_with_a_location() {
    let neg = eur()
        .line(Line::fixed("ok", dec!(1)))
        .line(usage("x", dec!(1), "C62", dec!(-1)))
        .build()
        .unwrap_err();
    assert_eq!(neg.location(), Some(&Location::Line(1)));
    assert_eq!(neg.rule(), Some("BR-27"));
    let r130 = eur()
        .line(Line::new(
            "x",
            Quantity::new(dec!(1), "KWH"),
            UnitPrice::new(Price::new(dec!(1))).with_unit("MWH"),
        ))
        .build()
        .unwrap_err();
    assert_eq!(
        (r130.kind(), r130.location(), r130.rule()),
        (
            &ErrorKind::PriceBaseUnit,
            Some(&Location::Line(0)),
            Some("PEPPOL-EN16931-R130")
        )
    );
    // R130 fires only on a stated base-quantity unit: none stated, no check.
    assert!(
        eur()
            .line(Line::new(
                "x",
                Quantity::new(dec!(1), "KWH"),
                UnitPrice::new(Price::new(dec!(1))),
            ))
            .build()
            .is_ok()
    );
    let base = eur()
        .line(Line::new(
            "x",
            Quantity::one(),
            UnitPrice::new(Price::new(dec!(1))).with_base_quantity(dec!(0)),
        ))
        .build()
        .unwrap_err();
    assert_eq!(base.kind(), &ErrorKind::InvalidBaseQuantity);
    let r040 = eur()
        .line(Line::fixed("x", dec!(100)).with_adjustment(
            LineAdjustment::allowance(dec!(10.03)).with_basis(dec!(100), Percent::new(dec!(10))),
        ))
        .build()
        .unwrap_err();
    assert_eq!(
        r040.location(),
        Some(&Location::LineAdjustment { line: 0, index: 0 })
    );
    assert_eq!(r040.rule(), Some("PEPPOL-EN16931-R040"));
    // ±0.02 is accepted (u:slack is inclusive).
    assert!(
        eur()
            .line(
                Line::fixed("x", dec!(100)).with_adjustment(
                    LineAdjustment::allowance(dec!(10.02))
                        .with_basis(dec!(100), Percent::new(dec!(10)))
                )
            )
            .build()
            .is_ok()
    );
    let doc_r040 = eur()
        .line(Line::fixed("x", dec!(100)))
        .adjustment(
            DocumentAdjustment::charge("c", dec!(5)).with_basis(dec!(100), Percent::new(dec!(4))),
        )
        .build()
        .unwrap_err();
    assert_eq!(doc_r040.location(), Some(&Location::Adjustment(0)));
    let missing = eur()
        .line(Line::fixed("x", dec!(1)).with_vat(vat(dec!(19))))
        .line(Line::fixed("y", dec!(1)))
        .build()
        .unwrap_err();
    assert_eq!(
        (missing.kind(), missing.location(), missing.rule()),
        (
            &ErrorKind::MissingVat,
            Some(&Location::Line(1)),
            Some("BR-CO-04")
        )
    );
    let missing_adj = eur()
        .line(Line::fixed("x", dec!(1)).with_vat(vat(dec!(19))))
        .adjustment(DocumentAdjustment::allowance("d", dec!(1)))
        .build()
        .unwrap_err();
    assert_eq!(
        (
            missing_adj.kind(),
            missing_adj.location(),
            missing_adj.rule()
        ),
        (
            &ErrorKind::MissingVat,
            Some(&Location::Adjustment(0)),
            Some("BR-32")
        )
    );
    let missing_charge = eur()
        .line(Line::fixed("x", dec!(1)).with_vat(vat(dec!(19))))
        .adjustment(DocumentAdjustment::allowance("d", dec!(1)).with_vat(vat(dec!(19))))
        .adjustment(DocumentAdjustment::charge("c", dec!(1)))
        .build()
        .unwrap_err();
    assert_eq!(
        (
            missing_charge.kind(),
            missing_charge.location(),
            missing_charge.rule()
        ),
        (
            &ErrorKind::MissingVat,
            Some(&Location::Adjustment(1)),
            Some("BR-37")
        )
    );
    let bad_scale = Document::builder(
        Currency::EUR,
        AmountScale {
            decimals: 6,
            rounding: Rounding::HalfEven,
        },
    )
    .line(Line::fixed("x", dec!(1)))
    .build()
    .unwrap_err();
    assert_eq!(bad_scale.kind(), &ErrorKind::ScaleMismatch);
}

/// One refund, two spellings, one result: direction lives in the quantity.
#[test]
fn a_credit_is_a_negative_quantity() {
    let x = eur()
        .line(Line::fixed("refund", dec!(-50)))
        .build()
        .unwrap();
    let y = eur()
        .line(usage("refund", dec!(-1), "C62", dec!(50)))
        .build()
        .unwrap();
    assert_eq!(x, y);
    assert_eq!(x.line_net(0), a("-50"));
    // A fixed allowance on a negative line total is lawful.
    let z = eur()
        .line(usage("feed-in", dec!(10), "KWH", dec!(0.5)).with_net(dec!(-5)))
        .line(Line::fixed("base", dec!(2)))
        .adjustment(DocumentAdjustment::allowance("goodwill", dec!(1)))
        .build()
        .unwrap();
    assert_eq!(z.tax_exclusive_total(), a("-4"));
}

// ── Transforms ────────────────────────────────────────────────────────────────

fn rich() -> Document {
    eur()
        .default_vat(vat(dec!(19)))
        .line(usage("energy", dec!(1234.567), "KWH", dec!(0.3123)))
        .line(
            Line::fixed("base", dec!(9.99))
                .with_vat(vat(dec!(7)))
                .with_adjustment(LineAdjustment::allowance(dec!(1.005)).with_reason("x")),
        )
        .adjustment(
            DocumentAdjustment::charge("fee", dec!(2.50))
                .with_basis(dec!(250), Percent::new(dec!(1))),
        )
        .payments(
            vec![Advance {
                vat: vat(dec!(19)),
                net: dec!(100),
                tax: dec!(19),
            }]
            .into_iter()
            .map(Payment::Advance),
        )
        .cash_rounding(CashRounding {
            increment: dec!(0.05),
            rounding: Rounding::HalfEven,
        })
        .vat_accounting(VatAccounting {
            currency: Currency::new("PLN").unwrap(),
            rate: dec!(4.2817),
        })
        .build()
        .unwrap()
}

/// negated() is the exact negative document (no double negation by a 381
/// type code) and an involution.
#[test]
fn negated_is_an_involution_and_flips_every_amount() {
    let d = rich();
    let n = d.negated();
    assert_eq!(n.negated(), d);
    assert_eq!(n.payable(), -d.payable());
    assert_eq!(n.rounding(), -d.rounding());
    assert_eq!(n.vat_total(), -d.vat_total());
    assert_eq!(
        n.vat_accounting_total().unwrap().1,
        -d.vat_accounting_total().unwrap().1
    );
    assert_eq!(n.lines()[0].quantity().value(), dec!(-1234.567));
    assert!(n.lines()[0].price().price().value() >= Decimal::ZERO);
    assert_eq!(
        n.adjustments()[0].basis(),
        Some((dec!(-250), Percent::new(dec!(1))))
    );
    // The negated inputs rebuild to the same derived document.
    assert_eq!(n.clone().into_builder().build().unwrap(), n);
}

/// concat ≡ build of the concatenated inputs (VAT rounded once
/// over the union).
#[test]
fn concat_equals_build_of_concatenated_inputs() {
    let x = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(0.03)))
        .build()
        .unwrap();
    let y = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("b", dec!(0.03)))
        .build()
        .unwrap();
    assert_eq!(x.vat_total() + y.vat_total(), a("0.02"));
    let c = Document::concat([x.clone(), y.clone()]).unwrap();
    assert_eq!(c.vat_total(), a("0.01"));
    let direct = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(0.03)))
        .line(Line::fixed("b", dec!(0.03)))
        .build()
        .unwrap();
    assert_eq!(c, direct);
    assert_eq!(Document::concat([]).unwrap_err().kind(), &ErrorKind::Empty);
    let usd = Document::builder(Currency::USD, AmountScale::EN16931)
        .line(Line::fixed("u", dec!(1)))
        .build()
        .unwrap();
    assert!(matches!(
        Document::concat([x.clone(), usd]).unwrap_err().kind(),
        ErrorKind::CurrencyMismatch { .. }
    ));
    let other_scale = Document::builder(
        Currency::EUR,
        AmountScale::new(5, Rounding::HalfEven).unwrap(),
    )
    .default_vat(vat(dec!(19)))
    .line(Line::fixed("u", dec!(1)))
    .build()
    .unwrap();
    assert_eq!(
        Document::concat([x, other_scale]).unwrap_err().kind(),
        &ErrorKind::ScaleMismatch
    );
}

/// concat keeps every payment entry in order and refuses mismatched settings.
#[test]
fn concat_keeps_every_payment() {
    let adv = |net| Advance {
        vat: vat(dec!(19)),
        net,
        tax: dec!(0),
    };
    let doc = |p: Vec<Payment>| {
        eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("a", dec!(100)))
            .payments(p)
            .build()
            .unwrap()
    };
    let none = Document::concat([doc(vec![]), doc(vec![])]).unwrap();
    assert!(none.payments().is_empty());
    let items = Document::concat([
        doc(vec![Payment::Advance(adv(dec!(1)))]),
        doc(vec![]),
        doc(vec![Payment::Advance(adv(dec!(2)))]),
    ])
    .unwrap();
    assert_eq!(
        items.payments(),
        [
            Payment::Advance(adv(dec!(1))),
            Payment::Advance(adv(dec!(2)))
        ]
    );
    // 030: a gross payment beside an advance is kept, not folded into a total.
    let five = Payment::Gross {
        description: "deposit".into(),
        amount: dec!(5),
    };
    let mixed = Document::concat([
        doc(vec![five.clone()]),
        doc(vec![Payment::Advance(adv(dec!(2)))]),
    ])
    .unwrap();
    assert_eq!(mixed.payments(), [five, Payment::Advance(adv(dec!(2)))]);
    assert_eq!(mixed.prepaid(), a("7"));
    let cash = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(1)))
        .cash_rounding(CashRounding {
            increment: dec!(0.05),
            rounding: Rounding::HalfEven,
        })
        .build()
        .unwrap();
    let err = Document::concat([doc(vec![]), cash]).unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::SettingMismatch {
            setting: "cash_rounding"
        }
    );
    assert_eq!(err.rule(), None);
}

/// At scale 0 the basis is omitted (R040's ±0.02 cannot hold).
#[test]
fn apportion_at_scale_zero_states_no_basis() {
    let s = AmountScale::new(0, Rounding::HalfAwayFromZero).unwrap();
    let lines = [Line::fixed("item", dec!(105))];
    let d = apportion(
        AdjustmentKind::Allowance,
        "10 %",
        Adjust::Percent(Percent::new(dec!(10))),
        &lines,
        |_| true,
        s,
    )
    .unwrap();
    assert_eq!((d[0].amount(), d[0].basis()), (dec!(11), None));
    let doc = Document::builder(Currency::JPY, s)
        .lines(lines)
        .adjustments(d)
        .build()
        .unwrap();
    assert_eq!(doc.payable(), a("94"));
}

/// A split share of an apportioned document drops the bases, or R040
/// would refuse the share.
#[test]
fn split_of_an_apportioned_document_builds() {
    let lines = [
        Line::fixed("a", dec!(33.33))
            .with_vat(vat(dec!(19)))
            .with_adjustment(
                LineAdjustment::allowance(dec!(3.33))
                    .with_basis(dec!(33.30), Percent::new(dec!(10))),
            ),
        Line::fixed("b", dec!(10.01)).with_vat(vat(dec!(7))),
    ];
    let d = apportion(
        AdjustmentKind::Allowance,
        "3 %",
        Adjust::Percent(Percent::new(dec!(3))),
        &lines,
        |_| true,
        AmountScale::EN16931,
    )
    .unwrap();
    let doc = eur().lines(lines).adjustments(d).build().unwrap();
    let shares = doc.split(&[dec!(1), dec!(1), dec!(1)]).unwrap();
    assert_eq!(
        shares
            .iter()
            .map(Document::tax_inclusive_total)
            .sum::<Amount>(),
        doc.tax_inclusive_total()
    );
    assert!(
        shares
            .iter()
            .all(|s| s.adjustments().iter().all(|a| a.basis().is_none()))
    );
    assert!(
        shares
            .iter()
            .all(|s| s.lines()[0].adjustments()[0].basis().is_none())
    );
}

/// A VAT-free document has no groups; groups are ordered by code.
#[test]
fn vat_free_documents_and_group_order() {
    let free = eur().line(Line::fixed("a", dec!(1))).build().unwrap();
    assert!(free.vat_groups().is_empty());
    assert_eq!(free.vat_total(), Amount::ZERO);
    let ae = Vat::exempt(
        TaxCategory::ReverseCharge,
        Exemption::Code("VATEX-EU-AE".into()),
    )
    .unwrap();
    let doc = eur()
        .line(Line::fixed("z", dec!(1)).with_vat(Vat::zero_rated()))
        .line(Line::fixed("s19", dec!(1)).with_vat(vat(dec!(19))))
        .line(Line::fixed("s7", dec!(1)).with_vat(vat(dec!(7))))
        .line(Line::fixed("ae", dec!(1)).with_vat(ae))
        .build()
        .unwrap();
    let order: Vec<(&str, Decimal)> = doc
        .vat_groups()
        .iter()
        .map(|g| (g.category().code(), g.rate_percent().value()))
        .collect();
    assert_eq!(
        order,
        [
            ("AE", dec!(0)),
            ("S", dec!(7)),
            ("S", dec!(19)),
            ("Z", dec!(0))
        ]
    );
}

/// A water bill of 47.10 net / 56.05 gross split 40/35/25 must re-assemble to
/// exactly 47.10 / 56.05, not 47.11 / 56.06.
#[test]
fn split_water_utility_is_exact() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("water", dec!(47.10)))
        .build()
        .unwrap();
    assert_eq!(doc.tax_inclusive_total(), a("56.05"));
    let shares = doc.split(&[dec!(40), dec!(35), dec!(25)]).unwrap();
    let sum = |f: fn(&Document) -> Amount| shares.iter().map(f).sum::<Amount>();
    assert_eq!(sum(Document::tax_exclusive_total), a("47.10"));
    assert_eq!(sum(Document::vat_total), a("8.95"));
    assert_eq!(sum(Document::tax_inclusive_total), a("56.05"));
    assert_eq!(sum(Document::payable), a("56.05"));
    for s in &shares {
        assert!(s.price_inconsistent().is_empty());
        // Round-trip of a share keeps its partition.
        assert!(s.tax_partition().is_some());
    }
}

/// Splitting each position on its own gives the first recipient every tie;
/// the document split keeps each recipient's total near its exact share.
#[test]
fn split_keeps_document_level_quota() {
    let lines: Vec<Line> = (0..1000)
        .map(|i| Line::fixed(format!("l{i}"), dec!(1.00)))
        .collect();
    let doc = eur().lines(lines).build().unwrap();
    let shares = doc.split(&[dec!(1); 3]).unwrap();
    let totals: Vec<Amount> = shares.iter().map(Document::line_total).collect();
    assert_eq!(totals.iter().copied().sum::<Amount>(), a("1000"));
    for t in &totals {
        let dev = (t.to_decimal() - dec!(333.333333)).abs();
        assert!(dev < dec!(0.02), "{t}");
    }
    let zero = doc.split(&[dec!(0), dec!(1)]).unwrap();
    assert_eq!(zero[0].payable(), Amount::ZERO);
    assert_eq!(zero[1].payable(), doc.payable());
    assert_eq!(
        doc.split(&[dec!(-1), dec!(2)]).unwrap_err().kind(),
        &ErrorKind::InvalidWeights
    );
}

/// Two same-group lines split n ways never fail and keep BR-CO-17.
#[test]
fn split_same_group_never_fails() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(10.01)))
        .line(Line::fixed("b", dec!(10.01)))
        .build()
        .unwrap();
    for n in 2..10 {
        let shares = doc.split(&vec![dec!(1); n]).unwrap();
        assert_eq!(
            shares.iter().map(Document::vat_total).sum::<Amount>(),
            doc.vat_total()
        );
    }
}

/// BT-114/115/111 are re-derived per share; everything else
/// is conserved.
#[test]
fn split_conserves_all_but_cash_rounding_and_accounting() {
    let d = rich();
    let shares = d.split(&[dec!(3), dec!(1), dec!(1)]).unwrap();
    let sum = |f: fn(&Document) -> Amount| shares.iter().map(f).sum::<Amount>();
    assert_eq!(sum(Document::line_total), d.line_total());
    assert_eq!(sum(Document::allowance_total), d.allowance_total());
    assert_eq!(sum(Document::charge_total), d.charge_total());
    assert_eq!(sum(Document::tax_exclusive_total), d.tax_exclusive_total());
    assert_eq!(sum(Document::vat_total), d.vat_total());
    assert_eq!(sum(Document::tax_inclusive_total), d.tax_inclusive_total());
    assert_eq!(sum(Document::prepaid), d.prepaid());
    for s in &shares {
        assert!(s.payable().fits(2) && s.rounding().fits(2));
        assert_eq!(s.adjustments()[0].basis(), None);
    }
}

#[test]
fn line_segments_conserve_the_line() {
    let line =
        usage("fee", dec!(1), "MON", dec!(10)).with_adjustment(LineAdjustment::charge(dec!(0.07)));
    let segs = line
        .segments(&[dec!(14), dec!(17)], AmountScale::EN16931)
        .unwrap();
    let doc = eur().lines(segs.clone()).build().unwrap();
    let whole = eur().line(line).build().unwrap();
    assert_eq!(doc.line_total(), whole.line_total());
    assert_eq!(
        segs[0].quantity().value() + segs[1].quantity().value(),
        dec!(1)
    );
    assert!(doc.price_inconsistent().is_empty());
}

// ── Bounds ────────────────────────────────────────────────────────────────────

/// A gross minimum of 5.18 at EN 16931 scale must reach 5.18, not under-bill
/// at 5.17. The bound is settled on the document's own VAT, under every rounding.
#[test]
fn minimum_gross_bound_is_reached_at_scale() {
    for rounding in [
        Rounding::HalfAwayFromZero,
        Rounding::HalfEven,
        Rounding::AwayFromZero,
        Rounding::TowardZero,
    ] {
        let s = AmountScale::new(2, rounding).unwrap();
        for cents in (500..2000).step_by(7) {
            let min = Decimal::new(cents, 2);
            let doc = Document::builder(Currency::EUR, s)
                .line(Line::fixed("a", dec!(1.37)).with_vat(vat(dec!(7))))
                .line(Line::fixed("b", dec!(2.11)).with_vat(vat(dec!(19))))
                .bound(Bound::minimum(Basis::Gross, min, "minimum").with_vat(vat(dec!(19))))
                .build()
                .unwrap();
            let g = doc.tax_inclusive_total().to_decimal();
            assert!(g >= min, "{rounding:?} {min} {g}");
            // Minimal: one unit less in the top-up misses the minimum.
            let mut lines = doc.lines().to_vec();
            let top = lines.pop().unwrap();
            let x = top.exact_net().unwrap() - dec!(0.01);
            let less = Document::builder(Currency::EUR, s)
                .lines(lines)
                .line(Line::fixed("m", x).with_vat(vat(dec!(19))))
                .build()
                .unwrap();
            assert!(
                less.tax_inclusive_total().to_decimal() < min,
                "{rounding:?} {min}"
            );
        }
    }
}

#[test]
fn maximum_and_invalid_bounds() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(100)))
        .bound(Bound::maximum(Basis::Net, dec!(80), "cap"))
        .build()
        .unwrap();
    assert_eq!(doc.tax_exclusive_total(), a("80"));
    assert_eq!(doc.line_net(1), a("-20"));
    let untouched = eur()
        .line(Line::fixed("a", dec!(100)))
        .bound(Bound::minimum(Basis::Net, dec!(50), "m"))
        .build()
        .unwrap();
    assert_eq!(untouched.lines().len(), 1);
    let conflict = eur()
        .line(Line::fixed("a", dec!(10)))
        .bound(Bound::minimum(Basis::Net, dec!(50), "m"))
        .bound(Bound::maximum(Basis::Net, dec!(40), "c"))
        .build()
        .unwrap_err();
    assert_eq!(conflict.kind(), &ErrorKind::InvalidBound);
    assert_eq!(conflict.location(), Some(&Location::Bound(1)));
    assert_eq!(conflict.rule(), None);
    let credit = eur()
        .line(Line::fixed("a", dec!(-10)))
        .bound(Bound::minimum(Basis::Net, dec!(5), "m"))
        .build()
        .unwrap_err();
    assert_eq!(credit.kind(), &ErrorKind::InvalidBound);
    assert_eq!(credit.location(), Some(&Location::Bound(0)));
    let negative = eur()
        .line(Line::fixed("a", dec!(1)))
        .bound(Bound::minimum(Basis::Net, dec!(-5), "m"))
        .build()
        .unwrap_err();
    assert_eq!(negative.kind(), &ErrorKind::InvalidBound);
    assert_eq!(negative.location(), Some(&Location::Bound(0)));
    let zero = eur()
        .line(Line::fixed("a", dec!(1)))
        .bound(Bound::maximum(Basis::Net, dec!(0), "m"))
        .build()
        .unwrap_err();
    assert_eq!(zero.kind(), &ErrorKind::InvalidBound);
    assert_eq!(zero.location(), Some(&Location::Bound(0)));
    let bases = eur()
        .line(Line::fixed("a", dec!(10)))
        .bound(Bound::minimum(Basis::Net, dec!(5), "m"))
        .bound(Bound::maximum(Basis::Gross, dec!(50), "c"))
        .build()
        .unwrap_err();
    assert_eq!(bases.kind(), &ErrorKind::InvalidBound);
    assert_eq!(bases.location(), Some(&Location::Bound(1)));
    let twice = eur()
        .line(Line::fixed("a", dec!(10)))
        .bound(Bound::minimum(Basis::Net, dec!(5), "m"))
        .bound(Bound::minimum(Basis::Net, dec!(6), "m"))
        .build()
        .unwrap_err();
    assert_eq!(twice.kind(), &ErrorKind::InvalidBound);
    assert_eq!(twice.location(), Some(&Location::Bound(1)));
    let both = eur()
        .line(Line::fixed("a", dec!(10)))
        .bound(Bound::maximum(Basis::Net, dec!(60), "c"))
        .bound(Bound::minimum(Basis::Net, dec!(50), "m"))
        .build()
        .unwrap();
    assert_eq!(both.tax_exclusive_total(), a("50"));
    let unvatted = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("a", dec!(1)))
        .build()
        .unwrap();
    // into_builder hands back resolved leaves, not the default VAT: a bound
    // added afterwards states its VAT (or the build refuses the mixed document).
    let bare = unvatted
        .clone()
        .into_builder()
        .bound(Bound::minimum(Basis::Net, dec!(5), "m"))
        .build();
    let bare = bare.unwrap_err();
    assert_eq!(bare.kind(), &ErrorKind::MissingVat);
    assert_eq!(bare.location(), Some(&Location::Bound(0)));
    assert_eq!(bare.rule(), None);
    let with = unvatted
        .into_builder()
        .bound(Bound::minimum(Basis::Net, dec!(5), "m").with_vat(vat(dec!(19))))
        .build()
        .unwrap();
    assert_eq!(with.lines()[1].vat(), Some(&vat(dec!(19))));
    assert_eq!(with.bound_lines(), 1);
    assert_eq!(with.clone().into_builder().build().unwrap(), with);
    // An edited document re-settles its bound instead of keeping a stale
    // top-up line.
    let edited = with
        .into_builder()
        .line(Line::fixed("more", dec!(2)).with_vat(vat(dec!(19))))
        .build()
        .unwrap();
    assert_eq!(edited.tax_exclusive_total(), a("5"));
    assert_eq!(edited.lines().len(), 3);
    assert_eq!(edited.line_net(2), a("2"));
}

/// A bound refusal names the bound by its builder position, also after the
/// build has put the minimum first.
#[test]
fn bound_refusals_are_located_in_call_order() {
    // Maximum given first, minimum second: the minimum on a credit is bound 1.
    let credit = eur()
        .line(Line::fixed("a", dec!(-10)))
        .bound(Bound::maximum(Basis::Net, dec!(60), "cap"))
        .bound(Bound::minimum(Basis::Net, dec!(5), "floor"))
        .build()
        .unwrap_err();
    assert_eq!(credit.kind(), &ErrorKind::InvalidBound);
    assert_eq!(credit.location(), Some(&Location::Bound(1)));
    // An amount with more decimals than the scale.
    let fine = eur()
        .line(Line::fixed("a", dec!(1)))
        .bound(Bound::maximum(Basis::Net, dec!(60), "cap"))
        .bound(Bound::minimum(Basis::Net, dec!(5.001), "floor"))
        .build()
        .unwrap_err();
    assert_eq!(fine.kind(), &ErrorKind::InvalidBound);
    assert_eq!(fine.location(), Some(&Location::Bound(1)));
    // Two maxima: the second is refused.
    let twice = eur()
        .line(Line::fixed("a", dec!(1)))
        .bound(Bound::maximum(Basis::Net, dec!(60), "cap"))
        .bound(Bound::minimum(Basis::Net, dec!(5), "floor"))
        .bound(Bound::maximum(Basis::Net, dec!(70), "cap 2"))
        .build()
        .unwrap_err();
    assert_eq!(twice.location(), Some(&Location::Bound(2)));
    // A bound without VAT in a VAT document: MissingVat at that bound.
    let bare = eur()
        .line(Line::fixed("a", dec!(1)).with_vat(vat(dec!(19))))
        .bound(Bound::maximum(Basis::Net, dec!(60), "cap").with_vat(vat(dec!(19))))
        .bound(Bound::minimum(Basis::Net, dec!(5), "floor"))
        .build()
        .unwrap_err();
    assert_eq!(bare.kind(), &ErrorKind::MissingVat);
    assert_eq!(bare.location(), Some(&Location::Bound(1)));
    // The stored bounds stay minimum-first whatever the call order.
    let ok = eur()
        .line(Line::fixed("a", dec!(10)))
        .bound(Bound::maximum(Basis::Net, dec!(60), "cap"))
        .bound(Bound::minimum(Basis::Net, dec!(50), "floor"))
        .build()
        .unwrap();
    assert_eq!(ok.bounds()[0].kind(), BoundKind::Minimum);
}

/// A bound's accessors return what it was built with; a minimum equal to the
/// maximum is a valid pair (only minimum > maximum is refused).
#[test]
fn bound_accessors_and_equal_pair() {
    let b = Bound::minimum(Basis::Gross, dec!(5.18), "minimum charge").with_vat(vat(dec!(19)));
    assert_eq!(b.kind(), BoundKind::Minimum);
    assert_eq!(b.basis(), Basis::Gross);
    assert_eq!(b.amount(), dec!(5.18));
    assert_eq!(b.description(), "minimum charge");
    assert_eq!(b.vat(), Some(&vat(dec!(19))));
    let pinned = eur()
        .line(Line::fixed("a", dec!(10)))
        .bound(Bound::minimum(Basis::Net, dec!(50), "m"))
        .bound(Bound::maximum(Basis::Net, dec!(50), "c"))
        .build()
        .unwrap();
    assert_eq!(pinned.tax_exclusive_total(), a("50"));
}

/// A document whose only VAT is on a line (no adjustments, no bounds) still
/// refuses a line without VAT.
#[test]
fn a_vat_line_beside_a_bare_line_is_refused() {
    let err = eur()
        .line(Line::fixed("a", dec!(1)).with_vat(vat(dec!(19))))
        .line(Line::fixed("b", dec!(1)))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::MissingVat);
    assert_eq!(err.location(), Some(&Location::Line(1)));
    assert_eq!(err.rule(), Some("BR-CO-04"));
}

/// No derived amount may be `Amount::MIN` (it could not be negated), wherever
/// it appears: a total, a line net, a line adjustment, a document adjustment or
/// a VAT group's base — each alone. At scale 5 (`Amount::MIN` is not a whole
/// number of cents).
#[test]
fn amount_min_is_refused_in_every_derived_position() {
    let min = Amount::<5>::MIN.to_decimal();
    let half = min / dec!(2);
    let five = || {
        Document::builder(
            Currency::EUR,
            AmountScale::new(5, Rounding::HalfEven).unwrap(),
        )
    };
    let overflow = |b: DocumentBuilder| {
        let err = b.build().unwrap_err();
        assert_eq!(err.kind(), &ErrorKind::Overflow);
        assert_eq!(err.location(), None);
    };
    // Only a total: two halves.
    overflow(
        five()
            .line(Line::fixed("h1", half))
            .line(Line::fixed("h2", half)),
    );
    // Only a line net: MIN beside +1.
    overflow(
        five()
            .line(Line::fixed("min", min))
            .line(Line::fixed("one", dec!(1))),
    );
    // Only a line charge: MIN and +1 on a zero line.
    overflow(
        five().line(
            Line::fixed("x", dec!(0))
                .with_adjustment(LineAdjustment::charge(min))
                .with_adjustment(LineAdjustment::charge(dec!(1))),
        ),
    );
    // Only a document charge: MIN beside a charge of +1.
    overflow(
        five()
            .line(Line::fixed("x", dec!(0)))
            .adjustment(DocumentAdjustment::charge("min", min))
            .adjustment(DocumentAdjustment::charge("one", dec!(1))),
    );
    // Only a VAT group base: two halves at Z; a +1 line at S keeps every total
    // in range.
    overflow(
        five()
            .line(Line::fixed("h1", half).with_vat(Vat::zero_rated()))
            .line(Line::fixed("h2", half).with_vat(Vat::zero_rated()))
            .line(Line::fixed("one", dec!(1)).with_vat(vat(dec!(19)))),
    );
    // One unit above MIN everywhere builds.
    five()
        .line(Line::fixed("m", min + dec!(0.00001)))
        .build()
        .unwrap();
}

/// A document whose only VAT is on an adjustment still refuses a line without
/// VAT (the VAT test looks at lines, adjustments and bounds alike).
#[test]
fn a_vat_adjustment_beside_a_bare_line_is_refused() {
    let err = eur()
        .line(Line::fixed("a", dec!(1)))
        .adjustment(DocumentAdjustment::charge("c", dec!(1)).with_vat(vat(dec!(19))))
        .build()
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::MissingVat);
    assert_eq!(err.location(), Some(&Location::Line(0)));
}

/// A split share exposes its partition; a built document has none; a
/// negated share carries the negated partition.
#[test]
fn tax_partition_is_exposed_and_negated() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("x", dec!(1000)))
        .build()
        .unwrap();
    assert_eq!(doc.tax_partition(), None);
    let shares = doc.split(&[dec!(1), dec!(1)]).unwrap();
    assert_eq!(shares[0].tax_partition(), Some(&[a("95.00")][..]));
    assert_eq!(
        shares[0].negated().tax_partition(),
        Some(&[a("-95.00")][..])
    );
}

/// Adjustment accessors return what was stated; `negated` negates a line
/// adjustment's basis base with its amount, so R040 still holds; a line's
/// exact net is quantity × price ÷ base plus charges minus allowances.
#[test]
fn adjustment_accessors_negation_and_exact_net() {
    let adj = LineAdjustment::charge(dec!(2.50))
        .with_basis(dec!(250), Percent::new(dec!(1)))
        .with_reason("freight")
        .with_reason_code("FC");
    assert_eq!(adj.kind(), AdjustmentKind::Charge);
    assert_eq!(adj.basis(), Some((dec!(250), Percent::new(dec!(1)))));
    assert_eq!(adj.reason(), Some("freight"));
    assert_eq!(adj.reason_code(), Some("FC"));
    assert_eq!(LineAdjustment::allowance(dec!(1)).reason(), None);
    assert_eq!(LineAdjustment::allowance(dec!(1)).reason_code(), None);
    let doc_adj = DocumentAdjustment::allowance("d", dec!(1)).with_reason_code("95");
    assert_eq!(doc_adj.reason_code(), Some("95"));
    assert_eq!(
        DocumentAdjustment::allowance("d", dec!(1)).reason_code(),
        None
    );

    let line = usage("x", dec!(3), "C62", dec!(1.5))
        .with_adjustment(adj)
        .with_adjustment(LineAdjustment::allowance(dec!(1)));
    assert_eq!(line.exact_net().unwrap(), dec!(6)); // 4.50 + 2.50 − 1
    let doc = eur().line(line).build().unwrap();
    let n = doc.negated();
    let negated = &n.lines()[0].adjustments()[0];
    assert_eq!(negated.amount(), dec!(-2.50));
    assert_eq!(negated.basis(), Some((dec!(-250), Percent::new(dec!(1)))));
    assert_eq!(negated.reason(), Some("freight"));
    assert_eq!(n.clone().into_builder().build().unwrap(), n);
}

/// A fixed whole-document amount over a credit (every base negative) splits
/// by |base|, like the debit: 8.01 / 2.00 of 10.01.
#[test]
fn apportion_fixed_over_a_credit_splits_by_magnitude() {
    let lines = [
        Line::fixed("a", dec!(-400)).with_vat(vat(dec!(19))),
        Line::fixed("b", dec!(-100)).with_vat(vat(dec!(7))),
    ];
    let d = apportion(
        AdjustmentKind::Allowance,
        "voucher",
        Adjust::Fixed(dec!(10.01)),
        &lines,
        |_| true,
        AmountScale::EN16931,
    )
    .unwrap();
    let amounts: Vec<Decimal> = d.iter().map(DocumentAdjustment::amount).collect();
    assert_eq!(amounts, [dec!(8.01), dec!(2)]);
}

// ── Settlement ────────────────────────────────────────────────────────────────

/// The Guthaben final invoice: advances exceed the supply, payable
/// goes negative.
#[test]
fn credit_balance_final_invoice() {
    let advances: Vec<Advance> = (0..12)
        .map(|_| Advance {
            vat: vat(dec!(19)),
            net: dec!(100),
            tax: dec!(19),
        })
        .collect();
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("supply", dec!(1000)))
        .payments(advances.into_iter().map(Payment::Advance))
        .build()
        .unwrap();
    assert_eq!(doc.prepaid(), a("1428"));
    assert_eq!(doc.payable(), a("-238.00"));
    let excess = eur()
        .line(Line::fixed("x", dec!(1)))
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(1.001),
        })
        .build()
        .unwrap_err();
    assert!(matches!(excess.kind(), ErrorKind::ExcessPrecision { .. }));
    assert_eq!(excess.location(), Some(&Location::Payment(0)));
}

/// US1-2: a flat prepayment is BT-113 as given; BT-115 = BT-112 − BT-113.
#[test]
fn flat_prepayment_is_deducted() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("supply", dec!(1000)))
        .payment(Payment::Gross {
            description: "prepayment".into(),
            amount: dec!(500),
        })
        .build()
        .unwrap();
    assert_eq!(doc.tax_inclusive_total(), a("1190.00"));
    assert_eq!(doc.prepaid(), a("500.00"));
    assert_eq!(doc.payable(), a("690.00"));
}

/// US1-3: an advance at a rate the final invoice no longer has (a rate change
/// in between) is restated, not re-derived: accepted, and it adds no group.
#[test]
fn advance_at_a_rate_the_supply_lacks_is_restated() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("supply", dec!(1000)))
        .payments(
            vec![Advance {
                vat: vat(dec!(16)),
                net: dec!(100),
                tax: dec!(16),
            }]
            .into_iter()
            .map(Payment::Advance),
        )
        .build()
        .unwrap();
    assert_eq!(doc.vat_groups().len(), 1);
    assert_eq!(doc.vat_groups()[0].rate_percent(), Percent::new(dec!(19)));
    assert_eq!(doc.prepaid(), a("116.00"));
    assert_eq!(doc.payable(), a("1074.00"));
}

/// US1-4: an advance whose net or tax has more decimals than the scale is
/// refused at `Location::Payment(i)`.
#[test]
fn advance_with_excess_digits_is_located() {
    let ok = Advance {
        vat: vat(dec!(19)),
        net: dec!(100),
        tax: dec!(19),
    };
    for (net, tax) in [(dec!(100.001), dec!(19)), (dec!(100), dec!(19.005))] {
        let bad = Advance {
            net,
            tax,
            ..ok.clone()
        };
        let err = eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("supply", dec!(1000)))
            .payments(vec![ok.clone(), bad].into_iter().map(Payment::Advance))
            .build()
            .unwrap_err();
        assert!(matches!(err.kind(), ErrorKind::ExcessPrecision { .. }));
        assert_eq!(err.location(), Some(&Location::Payment(1)));
        assert_eq!(err.rule(), None);
    }
}

#[test]
fn cash_rounding_to_five_rappen() {
    let chf = Document::builder(Currency::CHF, AmountScale::EN16931)
        .default_vat(vat(dec!(8.1)))
        .line(Line::fixed("x", dec!(9.27)))
        .cash_rounding(CashRounding {
            increment: dec!(0.05),
            rounding: Rounding::HalfAwayFromZero,
        })
        .build()
        .unwrap();
    assert_eq!(chf.tax_inclusive_total(), a("10.02"));
    assert_eq!(chf.rounding(), a("-0.02"));
    assert_eq!(chf.payable(), a("10.00"));
    // An increment finer than the scale is refused.
    let fine = eur()
        .line(Line::fixed("x", dec!(1)))
        .cash_rounding(CashRounding {
            increment: dec!(0.003),
            rounding: Rounding::HalfEven,
        })
        .build()
        .unwrap_err();
    assert_eq!(fine.kind(), &ErrorKind::InvalidCashRounding);
}

/// BT-111 is derived from BT-110 and the rate; R005 refuses the same
/// currency; a non-positive rate is a plain domain error.
#[test]
fn vat_accounting_is_derived_and_validated() {
    let pln = Currency::new("PLN").unwrap();
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("x", dec!(1000)))
        .vat_accounting(VatAccounting {
            currency: pln,
            rate: dec!(4.2817),
        })
        .build()
        .unwrap();
    assert_eq!(doc.vat_accounting_total(), Some((pln, a("813.52")))); // 190 × 4.2817 = 813.523
    let same = eur()
        .line(Line::fixed("x", dec!(1)))
        .vat_accounting(VatAccounting {
            currency: Currency::EUR,
            rate: dec!(1),
        })
        .build()
        .unwrap_err();
    assert_eq!(same.rule(), Some("PEPPOL-EN16931-R005"));
    let zero = eur()
        .line(Line::fixed("x", dec!(1)))
        .vat_accounting(VatAccounting {
            currency: pln,
            rate: dec!(0),
        })
        .build()
        .unwrap_err();
    assert_eq!(zero.kind(), &ErrorKind::InvalidRate { rate: dec!(0) });
    assert_eq!(zero.rule(), None);
}

/// US3-2 (D-10): BT-110 = 0 gives BT-111 = 0; a free-floating 817 PLN
/// cannot be expressed.
#[test]
fn vat_accounting_of_zero_vat_is_zero() {
    let pln = Currency::new("PLN").unwrap();
    let doc = eur()
        .line(Line::fixed("x", dec!(1000)).with_vat(Vat::zero_rated()))
        .vat_accounting(VatAccounting {
            currency: pln,
            rate: dec!(4.2817),
        })
        .build()
        .unwrap();
    assert_eq!(doc.vat_total(), a("0"));
    assert_eq!(doc.vat_accounting_total(), Some((pln, a("0"))));
}

/// R120: a stated net that misses quantity × price by more than 0.02 is
/// reported.
#[test]
fn price_inconsistent_reports_stated_nets_missing_r120() {
    let doc = eur()
        .line(usage("ok", dec!(3), "C62", dec!(1)).with_net(dec!(3.02)))
        .line(usage("off", dec!(3), "C62", dec!(1)).with_net(dec!(3.03)))
        .line(usage("derived", dec!(3), "C62", dec!(1.333333)))
        .build()
        .unwrap();
    assert_eq!(doc.price_inconsistent(), [1]);
}

// ── Tariff integration ────────────────────────────────────────────────────────

/// A rate series rounded twice gives 0.01; the stated exact net 0.004995 is
/// reduced once: 0.00.
#[test]
fn rate_series_line_rounds_once() {
    let s = rate_series([
        (dec!(0.5), Price::new(dec!(0.00999))),
        (dec!(2.28571), Price::new(dec!(0))),
    ])
    .unwrap();
    let doc = eur().line(s.into_line("spot", "KWH")).build().unwrap();
    assert_eq!(doc.line_net(0), a("0.00"));
    assert!(doc.price_inconsistent().is_empty());
}

#[test]
fn negative_average_series_puts_the_sign_in_the_quantity() {
    let s = rate_series([(dec!(1), Price::new(dec!(-0.1)))]).unwrap();
    let line = s.into_line("feed-in", "KWH");
    assert_eq!(line.quantity().value(), dec!(-1));
    assert_eq!(line.price().price().value(), dec!(0.1));
    let doc = eur().line(line).build().unwrap();
    assert_eq!(doc.line_net(0), a("-0.10"));
    assert!(doc.price_inconsistent().is_empty());
}

#[test]
fn tariff_pieces_become_lines() {
    let tiers = Tiers::new([
        Tier::up_to(dec!(100), Price::new(dec!(0.10))).with_flat(dec!(5)),
        Tier::above(Price::new(dec!(0.05))),
    ])
    .unwrap();
    let lines: Vec<Line> = Tariff::Graduated(tiers)
        .rate(dec!(150))
        .unwrap()
        .into_iter()
        .flat_map(|p| p.into_lines("usage", "KWH"))
        .collect();
    let doc = eur().lines(lines).build().unwrap();
    assert_eq!(doc.line_total(), a("17.50"));
}

// ── Terms ─────────────────────────────────────────────────────────────────────

/// Skonto rounded once at the document scale, its VAT correction
/// partitioned exactly and sign-symmetric.
#[test]
fn early_payment_discount_and_vat_correction() {
    let doc = eur()
        .line(Line::fixed("a", dec!(100)).with_vat(vat(dec!(19))))
        .line(Line::fixed("b", dec!(50)).with_vat(vat(dec!(7))))
        .build()
        .unwrap();
    assert_eq!(doc.payable(), a("172.50"));
    let skonto = EarlyPaymentDiscount {
        percent: Percent::new(dec!(3)),
    };
    assert_eq!(skonto.amount(&doc).unwrap(), a("5.18")); // 5.175
    let c = skonto.vat_correction(&doc).unwrap();
    // 5.18 split 119 : 53.50 → 1.61 (7 %) / 3.57 (19 %); groups sorted by rate.
    assert_eq!((c[0].taxable_base(), c[0].tax()), (a("1.50"), a("0.11")));
    assert_eq!((c[1].taxable_base(), c[1].tax()), (a("3.00"), a("0.57")));
    let total: Amount = c.iter().map(|g| g.taxable_base() + g.tax()).sum();
    assert_eq!(total, a("5.18"));
    let n = skonto.vat_correction(&doc.negated()).unwrap();
    for (x, y) in c.iter().zip(&n) {
        assert_eq!((x.taxable_base(), x.tax()), (-y.taxable_base(), -y.tax()));
    }
    let free = eur().line(Line::fixed("a", dec!(1))).build().unwrap();
    assert!(skonto.vat_correction(&free).unwrap().is_empty());
}

/// Statutory interest across a base-rate change rounds the exact sum
/// once: 16.33 (per-segment rounding gives 16.32).
#[test]
fn late_payment_interest_rounds_once() {
    let i = late_payment_interest(
        a("1000"),
        &[
            InterestSegment {
                annual_percent: Percent::new(dec!(9.12)),
                days: 37,
                year_days: 365,
            },
            InterestSegment {
                annual_percent: Percent::new(dec!(8.62)),
                days: 30,
                year_days: 365,
            },
        ],
        AmountScale::EN16931,
    )
    .unwrap();
    assert_eq!(i, a("16.33"));
    let mixed = late_payment_interest(
        a("1000"),
        &[
            InterestSegment {
                annual_percent: Percent::new(dec!(5)),
                days: 1,
                year_days: 365,
            },
            InterestSegment {
                annual_percent: Percent::new(dec!(5)),
                days: 1,
                year_days: 366,
            },
        ],
        AmountScale::new(5, Rounding::HalfEven).unwrap(),
    )
    .unwrap();
    // 50/365 + 50/366 = 0.136986… + 0.136612… = 0.273598…
    assert_eq!(mixed, a("0.27360"));
    let zero = late_payment_interest(
        a("1"),
        &[InterestSegment {
            annual_percent: Percent::new(dec!(1)),
            days: 1,
            year_days: 0,
        }],
        AmountScale::EN16931,
    );
    assert_eq!(zero.unwrap_err().kind(), &ErrorKind::DivisionByZero);
    // A negative rate (the base rate was negative 2013–2022) is accepted:
    // 1000 × −0.88 / 100 × 365 / 365 = −8.80.
    let negative = late_payment_interest(
        a("1000"),
        &[InterestSegment {
            annual_percent: Percent::new(dec!(-0.88)),
            days: 365,
            year_days: 365,
        }],
        AmountScale::EN16931,
    )
    .unwrap();
    assert_eq!(negative, a("-8.80"));
}

/// Twenty segments at one year length sum exactly as one segment of the
/// summed days (the common denominator stays 365; it must not grow per
/// segment).
#[test]
fn many_interest_segments_equal_one() {
    let seg = |days| InterestSegment {
        annual_percent: Percent::new(dec!(9.12)),
        days,
        year_days: 365,
    };
    let many: Vec<_> = (1..=20).map(seg).collect();
    let total_days: u32 = (1..=20).sum();
    for s in [
        AmountScale::EN16931,
        AmountScale::new(5, Rounding::TowardZero).unwrap(),
    ] {
        let split = late_payment_interest(a("1234.56"), &many, s).unwrap();
        let whole = late_payment_interest(a("1234.56"), &[seg(total_days)], s).unwrap();
        assert_eq!(split, whole);
    }
    // 1234.56 × 9.12 / 100 × 210 / 365 = 64.778… → 64.78.
    assert_eq!(
        late_payment_interest(a("1234.56"), &many, AmountScale::EN16931).unwrap(),
        a("64.78")
    );
    // Mixed year lengths, 20 segments: still one exact sum.
    let mixed: Vec<_> = (0..20)
        .map(|i| InterestSegment {
            annual_percent: Percent::new(dec!(5)),
            days: 1,
            year_days: [365, 366, 360][i % 3],
        })
        .collect();
    let i = late_payment_interest(
        a("1000"),
        &mixed,
        AmountScale::new(5, Rounding::HalfEven).unwrap(),
    );
    // 1000 × 5/100 × (7/365 + 7/366 + 6/360) = 0.958904… + 0.956284… + 0.833333… = 2.748521…
    assert_eq!(i.unwrap(), a("2.74852"));
}

/// A Skonto percent outside 0..=100 is refused at use, by both the amount
/// and the VAT correction (even on a VAT-free document); the bounds pass.
#[test]
fn early_payment_discount_percent_out_of_range() {
    let doc = eur()
        .line(Line::fixed("a", dec!(100)).with_vat(vat(dec!(19))))
        .build()
        .unwrap();
    let free = eur().line(Line::fixed("a", dec!(1))).build().unwrap();
    for percent in [dec!(-0.01), dec!(100.01), dec!(-100)] {
        let skonto = EarlyPaymentDiscount {
            percent: Percent::new(percent),
        };
        let invalid = ErrorKind::InvalidRate { rate: percent };
        assert_eq!(skonto.amount(&doc).unwrap_err().kind(), &invalid);
        assert_eq!(skonto.vat_correction(&doc).unwrap_err().kind(), &invalid);
        assert_eq!(skonto.vat_correction(&free).unwrap_err().kind(), &invalid);
    }
    let none = EarlyPaymentDiscount {
        percent: Percent::new(dec!(0)),
    };
    assert_eq!(none.amount(&doc).unwrap(), a("0"));
    let all = EarlyPaymentDiscount {
        percent: Percent::new(dec!(100)),
    };
    assert_eq!(all.amount(&doc).unwrap(), doc.payable());
}

// ── serde ─────────────────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
mod serde_round_trip {
    use super::*;

    #[test]
    fn json_round_trip_is_byte_identical() {
        for d in [rich(), rich().negated()] {
            let json = serde_json::to_string(&d).unwrap();
            let back: Document = serde_json::from_str(&json).unwrap();
            assert_eq!(back, d);
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
            assert!(
                !json.contains("payable"),
                "derived values are not serialised"
            );
        }
        for share in rich().split(&[dec!(1), dec!(2)]).unwrap() {
            let json = serde_json::to_string(&share).unwrap();
            assert!(json.contains("tax_partition"));
            let back: Document = serde_json::from_str(&json).unwrap();
            assert_eq!(back, share);
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
    }

    #[test]
    fn a_tampered_tax_partition_is_refused() {
        let doc = eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("x", dec!(1000)))
            .build()
            .unwrap();
        let share = &doc.split(&[dec!(1), dec!(1)]).unwrap()[0];
        let json = serde_json::to_string(share).unwrap();
        assert!(json.contains("\"tax_partition\":[\"95.00000\"]"), "{json}");
        // A share's tax is the floor or ceiling of its exact tax: one unit off
        // the derived 95.00 is a partition, anything more is tampering — even
        // inside BR-CO-17's ±1.00.
        let near: Document = serde_json::from_str(&json.replace("95.00000", "95.01")).unwrap();
        assert_eq!(near.vat_total(), a("95.01"));
        for bad in ["95.02", "95.99", "-95.00"] {
            let far = serde_json::from_str::<Document>(&json.replace("95.00000", bad));
            // serde reports the build error through its Display; compare
            // against the kind's own text rather than a hard-coded message.
            let shown = far.unwrap_err().to_string();
            let kind = ErrorKind::InvalidTaxPartition.to_string();
            assert!(shown.starts_with(&kind), "{bad}: {shown}");
        }
        let wrong_len = serde_json::from_str::<Document>(&json.replace("[\"95.00000\"]", "[]"));
        assert!(wrong_len.is_err());
    }

    /// An injected 190.99 on 1000.00 at 19 % passes BR-CO-17 (±1.00) but
    /// is not a partition.
    #[test]
    fn injected_tax_inside_br_co_17_is_refused() {
        let doc = eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("x", dec!(1000)))
            .build()
            .unwrap();
        let mut json = serde_json::to_string(&doc).unwrap();
        json.pop();
        json.push_str(r#","tax_partition":["190.99"]}"#);
        assert!(serde_json::from_str::<Document>(&json).is_err());
    }

    /// US7-2: a JSON line with price −1 is refused on the way in (BR-27,
    /// line 0); no document exists.
    #[test]
    fn a_negative_price_is_refused_on_the_way_in() {
        let doc = eur()
            .line(usage("x", dec!(2), "C62", dec!(1)))
            .build()
            .unwrap();
        let json = serde_json::to_string(&doc).unwrap();
        let price = r#""price":"1""#;
        assert!(json.contains(price), "{json}");
        let err = serde_json::from_str::<Document>(&json.replace(price, r#""price":"-1""#))
            .unwrap_err()
            .to_string();
        let want = format!(
            "{}: {}",
            Location::Line(0),
            ErrorKind::NegativePrice { price: dec!(-1) }
        );
        assert!(err.starts_with(&want), "{err}");
    }

    /// D-17: BT-114 is never read from the wire. A document carrying a
    /// stored rounding of 999 gets BT-114 re-derived from its rule (or 0
    /// without one).
    #[test]
    fn a_stored_cash_rounding_amount_is_re_derived() {
        let chf = Document::builder(Currency::CHF, AmountScale::EN16931)
            .default_vat(vat(dec!(8.1)))
            .line(Line::fixed("x", dec!(9.27)))
            .cash_rounding(CashRounding {
                increment: dec!(0.05),
                rounding: Rounding::HalfAwayFromZero,
            })
            .build()
            .unwrap();
        let plain = Document::builder(Currency::CHF, AmountScale::EN16931)
            .default_vat(vat(dec!(8.1)))
            .line(Line::fixed("x", dec!(9.27)))
            .build()
            .unwrap();
        for doc in [chf.clone(), plain] {
            let json = serde_json::to_string(&doc).unwrap();
            let mut injected = json.clone();
            injected.pop();
            injected.push_str(r#","rounding":"999","payable":"999"}"#);
            let back: Document = serde_json::from_str(&injected).unwrap();
            assert_ne!(back.rounding(), a("999"));
            assert_eq!(back.rounding(), doc.rounding());
            assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
        assert_eq!(chf.rounding(), a("-0.02"));
    }

    /// Bounds are serialised as inputs and re-settled on the way in.
    #[test]
    fn bounds_round_trip_as_inputs() {
        let doc = eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("x", dec!(3.10)))
            .bound(Bound::minimum(Basis::Gross, dec!(5.18), "minimum"))
            .build()
            .unwrap();
        let json = serde_json::to_string(&doc).unwrap();
        assert!(json.contains("\"bounds\""));
        assert_eq!(json.matches("\"description\"").count(), 2, "{json}"); // one line, one bound
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    /// Apportion inputs are serialised as inputs (not their adjustments) and
    /// re-applied on the way in.
    #[test]
    fn apportion_round_trips_as_an_input() {
        let doc = eur()
            .default_vat(vat(dec!(19)))
            .line(Line::fixed("full", dec!(400)))
            .line(Line::fixed("reduced", dec!(100)).with_vat(vat(dec!(7))))
            .adjustment(DocumentAdjustment::charge("fee", dec!(5)))
            .apportion(
                AdjustmentKind::Allowance,
                "Rabatt",
                Adjust::Percent(Percent::new(dec!(10))),
            )
            .build()
            .unwrap();
        let json = serde_json::to_string(&doc).unwrap();
        assert!(json.contains("\"apportion\""), "{json}");
        assert!(
            !json.contains("\"40\""),
            "materialised amounts are derived: {json}"
        );
        let back: Document = serde_json::from_str(&json).unwrap();
        assert_eq!(back, doc);
        assert_eq!(back.vat_total(), a("75.65"));
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
    }

    #[test]
    fn invalid_inputs_are_refused_on_the_way_in() {
        let json =
            serde_json::to_string(&eur().line(Line::fixed("x", dec!(1))).build().unwrap()).unwrap();
        assert!(
            serde_json::from_str::<Document>(&json.replace("\"1\"", "1")).is_err(),
            "numbers refused"
        );
        let v = serde_json::to_string(&vat(dec!(19))).unwrap();
        assert_eq!(v, r#"{"category":"S","rate_percent":"19"}"#);
        assert!(serde_json::from_str::<Vat>(r#"{"category":"S","rate_percent":"0"}"#).is_err());
        // A rate below 1 % is a rate (030): no fraction guard, in serde either.
        let corsica = serde_json::from_str::<Vat>(r#"{"category":"S","rate_percent":"0.9"}"#);
        assert_eq!(corsica.unwrap(), vat(dec!(0.9)));
        assert!(serde_json::from_str::<Vat>(r#"{"category":"S","rate_percent":"-1"}"#).is_err());
        assert!(serde_json::from_str::<Vat>(r#"{"category":"S","rate_percent":"1"}"#).is_ok());
        assert!(serde_json::from_str::<Vat>(r#"{"category":"E","rate_percent":"0"}"#).is_err());
    }
}

/// Every public type that implements `Deserialize` is exercised by the serde
/// fuzz target (`fuzz/fuzz_targets/serde.rs`). Scans the sources: a derive
/// followed by a `pub struct`/`pub enum`, or a hand-written
/// `Deserialize<'de> for` impl, of a type re-exported from `lib.rs`.
#[test]
fn fuzz_serde_target_lists_every_deserialize_type() {
    use std::fs;
    use std::path::Path;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let Ok(fuzz) = fs::read_to_string(root.join("fuzz/fuzz_targets/serde.rs")) else {
        // A packaged crate leaves out the nested fuzz package.
        eprintln!("fuzz/ not present; skipped");
        return;
    };
    let lib = fs::read_to_string(root.join("src/lib.rs")).unwrap();
    let exported = |name: &str| {
        lib.split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == name)
    };
    let ident = |s: &str| -> String {
        s.chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect()
    };
    let mut found = Vec::new();
    for entry in fs::read_dir(root.join("src")).unwrap() {
        let text = fs::read_to_string(entry.unwrap().path()).unwrap();
        let lines: Vec<&str> = text.lines().map(str::trim).collect();
        for (i, l) in lines.iter().enumerate() {
            if l.contains("derive(") && l.contains("Deserialize") {
                let item = lines[i + 1..]
                    .iter()
                    .find(|n| n.contains("struct ") || n.contains("enum "));
                let name = item.and_then(|n| {
                    n.strip_prefix("pub struct ")
                        .or_else(|| n.strip_prefix("pub enum "))
                });
                found.extend(name.map(ident));
            }
            if l.starts_with("impl") {
                let name = l.split_once("Deserialize<'de> for ").map(|(_, rest)| rest);
                found.extend(name.map(ident));
            }
        }
    }
    found.retain(|n| exported(n));
    found.sort();
    found.dedup();
    assert!(found.len() >= 30, "scan found too few types: {found:?}");
    let missing: Vec<_> = found
        .iter()
        .filter(|n| {
            !fuzz.contains(&format!("roundtrip::<{n}>"))
                && !fuzz.contains(&format!("roundtrip::<{n}<"))
        })
        .collect();
    assert!(
        missing.is_empty(),
        "public Deserialize types missing from fuzz/fuzz_targets/serde.rs: {missing:?}"
    );
}

// ── 030 US3: percentages carry their unit ─────────────────────────────────────

/// Corsica's 0.9 % standard rate is expressible and exact: 1 000.00 → 9.00.
#[test]
fn us030_percent_sub_one_standard_rate_is_exact() {
    let doc = eur()
        .default_vat(Vat::standard(Percent::new(dec!(0.9))).unwrap())
        .line(Line::fixed("works", dec!(1000)))
        .build()
        .unwrap();
    assert_eq!(doc.vat_total(), a("9.00"));
    assert_eq!(doc.vat_groups()[0].rate_percent(), Percent::new(dec!(0.9)));
    assert_eq!(doc.vat_groups()[0].rate_percent().to_string(), "0.9 %");
}

#[test]
fn us030_percent_nineteen_percent_bills_190() {
    let doc = eur()
        .default_vat(Vat::standard(Percent::new(dec!(19))).unwrap())
        .line(Line::fixed("works", dec!(1000)))
        .build()
        .unwrap();
    assert_eq!(doc.vat_total(), a("190.00"));
}

/// Out-of-range percentages are refused with a typed error, never guessed.
#[test]
fn us030_percent_out_of_range_is_refused() {
    let doc = eur()
        .default_vat(vat(dec!(19)))
        .line(Line::fixed("works", dec!(1000)))
        .build()
        .unwrap();
    let skonto = EarlyPaymentDiscount {
        percent: Percent::new(dec!(101)),
    };
    assert_eq!(
        skonto.amount(&doc).unwrap_err().kind(),
        &ErrorKind::InvalidRate { rate: dec!(101) }
    );
    let err = Vat::standard(Percent::new(dec!(-19))).unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::InvalidVat(VatError::Rate {
            category: TaxCategory::Standard,
            rate_percent: Percent::new(dec!(-19)),
        })
    );
    assert_eq!(err.rule(), Some("BR-S-05"));
    // Every rate strictly between 0 and 1 is a rate now: no fraction guard.
    for rate in [dec!(0.19), dec!(0.5), dec!(0.999), dec!(0.0001)] {
        let v = Vat::standard(Percent::new(rate)).unwrap();
        assert_eq!(v.rate_percent().value(), rate);
    }
}

/// Every percentage input takes the same `Percent` value.
#[test]
fn us030_percent_every_input_takes_a_percent() {
    let ten = Percent::new(dec!(10));
    let line = Line::fixed("a", dec!(100))
        .with_vat(vat(dec!(19)))
        .with_adjustment(LineAdjustment::allowance(dec!(10)).with_basis(dec!(100), ten));
    assert_eq!(line.adjustments()[0].basis(), Some((dec!(100), ten)));
    let doc_adj = DocumentAdjustment::allowance("d", dec!(10)).with_basis(dec!(100), ten);
    assert_eq!(doc_adj.basis(), Some((dec!(100), ten)));
    let _ = Adjust::Percent(ten);
    let _ = InterestSegment {
        annual_percent: Percent::new(dec!(9.12)),
        days: 1,
        year_days: 365,
    };
    let fee = billing::TransactionFee::new(Percent::new(dec!(2.9)));
    assert_eq!(fee.percent, Percent::new(dec!(2.9)));
    assert_eq!(
        Vat::regional(TaxCategory::CanaryIslands, Percent::new(dec!(7)))
            .unwrap()
            .rate_percent(),
        Percent::new(dec!(7))
    );
}
