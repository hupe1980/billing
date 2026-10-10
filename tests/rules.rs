//! Each cited EN 16931 / Peppol rule as a check against random derived
//! documents (and their split shares), and R120 on lines built from fractional
//! metered usages. Rule texts: CEN validation-1.3.16
//! (`EN16931-model.sch`), Peppol BIS 3.0.20 (`PEPPOL-EN16931-UBL.sch`).

use billing::{
    AdjustmentKind, Amount, AmountScale, CashRounding, Currency, Decimal, Document,
    DocumentAdjustment, Exemption, Line, LineAdjustment, Payment, Percent, Price, Quantity,
    Rounding, TaxCategory, UnitPrice, Vat, VatAccounting,
};
use billing::{Aggregation, Meter, Sample, UnitConversion};
use proptest::prelude::*;
use rust_decimal::dec;

fn dec_in(lo: i64, hi: i64, max_scale: u32) -> impl Strategy<Value = Decimal> {
    (lo..=hi, 0..=max_scale).prop_map(|(m, s)| Decimal::new(m, s))
}

/// `lo..=hi` units at exactly `scale` decimals.
fn fixed(lo: i64, hi: i64, scale: u32) -> impl Strategy<Value = Decimal> {
    (lo..=hi).prop_map(move |m| Decimal::new(m, scale))
}

fn vats() -> Vec<Vat> {
    vec![
        Vat::standard(Percent::new(dec!(19))).unwrap(),
        Vat::standard(Percent::new(dec!(7))).unwrap(),
        Vat::zero_rated(),
        Vat::exempt(TaxCategory::Exempt, Exemption::Text("§4".into())).unwrap(),
        Vat::exempt(
            TaxCategory::IntraCommunity,
            Exemption::Code("VATEX-EU-IC".into()),
        )
        .unwrap(),
        Vat::regional(TaxCategory::CeutaMelilla, Percent::new(dec!(4))).unwrap(),
    ]
}

/// A random valid document at EN 16931 scale (rounding varies), lines derived
/// from quantity × price (no stated nets), with VAT accounting on some.
fn document() -> impl Strategy<Value = Document> {
    let line = (
        dec_in(-100_000, 100_000, 3),
        fixed(0, 10_000_000, 6),
        prop::sample::select(vec![dec!(1), dec!(3), dec!(1000)]),
        prop::collection::vec((any::<bool>(), dec_in(0, 2_000, 4)), 0..3),
        0..6usize,
    )
        .prop_map(|(q, p, base, adj, v)| {
            let mut l = Line::new(
                "l",
                Quantity::new(q, "KWH"),
                UnitPrice::new(Price::new(p)).with_base_quantity(base),
            )
            .with_vat(vats()[v].clone());
            for (charge, amount) in adj {
                l = l.with_adjustment(if charge {
                    LineAdjustment::charge(amount)
                } else {
                    LineAdjustment::allowance(amount)
                });
            }
            l
        });
    (
        prop::sample::select(vec![
            Rounding::HalfAwayFromZero,
            Rounding::HalfEven,
            Rounding::AwayFromZero,
            Rounding::TowardZero,
        ]),
        prop::collection::vec(line, 1..8),
        prop::collection::vec((any::<bool>(), dec_in(0, 5_000, 3), 0..6usize), 0..3),
        prop::option::of(dec_in(-50_000, 50_000, 2)),
        any::<bool>(),
        prop::option::of(fixed(1, 1_000_000, 4)),
    )
        .prop_map(|(mode, lines, adjs, pre, cash, acct)| {
            let mut b =
                Document::builder(Currency::EUR, AmountScale::new(2, mode).unwrap()).lines(lines);
            for (charge, amount, v) in adjs {
                let a = if charge {
                    DocumentAdjustment::charge("c", amount)
                } else {
                    DocumentAdjustment::allowance("a", amount)
                };
                b = b.adjustment(a.with_vat(vats()[v].clone()));
            }
            if let Some(p) = pre {
                b = b.payment(Payment::Gross {
                    description: "prepayment".into(),
                    amount: p,
                });
            }
            if cash {
                b = b.cash_rounding(CashRounding {
                    increment: dec!(0.05),
                    rounding: mode,
                });
            }
            if let Some(rate) = acct {
                b = b.vat_accounting(VatAccounting {
                    currency: Currency::new("PLN").unwrap(),
                    rate,
                });
            }
            b.build().unwrap()
        })
}

fn signed(kind: AdjustmentKind, a: Amount) -> Decimal {
    match kind {
        AdjustmentKind::Charge => a.to_decimal(),
        _ => -a.to_decimal(),
    }
}

/// XPath `round(x * 100) div 100`: half towards +∞ at two decimals.
fn xround(x: Decimal) -> Decimal {
    (x * dec!(100) + dec!(0.5)).floor() / dec!(100)
}

/// Every rule below, on one document.
fn check(d: &Document) -> Result<(), TestCaseError> {
    let lines = d.lines();
    // BR-CO-10: BT-106 = Σ BT-131.
    let sum_131: Decimal = (0..lines.len()).map(|i| d.line_net(i).to_decimal()).sum();
    prop_assert_eq!(d.line_total().to_decimal(), sum_131, "BR-CO-10");
    // BR-CO-11 / BR-CO-12: BT-107 = Σ BT-92, BT-108 = Σ BT-99.
    let by_kind = |k: AdjustmentKind| -> Decimal {
        d.adjustments()
            .iter()
            .enumerate()
            .filter(|(_, a)| a.kind() == k)
            .map(|(i, _)| d.adjustment_amount(i).to_decimal())
            .sum()
    };
    prop_assert_eq!(
        d.allowance_total().to_decimal(),
        by_kind(AdjustmentKind::Allowance),
        "BR-CO-11"
    );
    prop_assert_eq!(
        d.charge_total().to_decimal(),
        by_kind(AdjustmentKind::Charge),
        "BR-CO-12"
    );
    // BR-CO-13: BT-109 = Σ BT-131 − BT-107 + BT-108.
    prop_assert_eq!(
        d.tax_exclusive_total(),
        d.line_total() - d.allowance_total() + d.charge_total(),
        "BR-CO-13"
    );
    // BR-CO-14: BT-110 = Σ BT-117.
    prop_assert_eq!(
        d.vat_total(),
        d.vat_groups().iter().map(|g| g.tax()).sum::<Amount>(),
        "BR-CO-14"
    );
    // BR-CO-15: BT-112 = BT-109 + BT-110.
    prop_assert_eq!(
        d.tax_inclusive_total(),
        d.tax_exclusive_total() + d.vat_total(),
        "BR-CO-15"
    );
    // BR-CO-16: BT-115 = BT-112 − BT-113 + BT-114.
    prop_assert_eq!(
        d.payable(),
        d.tax_inclusive_total() - d.prepaid() + d.rounding(),
        "BR-CO-16"
    );
    for g in d.vat_groups() {
        // BR-CO-17 (the Schematron's two branches, XPath rounding).
        let expected =
            xround(g.taxable_base().to_decimal().abs() * g.rate_percent().value() / dec!(100));
        let tax = g.tax().to_decimal();
        if g.rate_percent().value().round().is_zero() {
            prop_assert!(tax.round().is_zero(), "BR-CO-17 zero branch");
        } else {
            prop_assert!(
                tax.abs() - dec!(1) < expected && tax.abs() + dec!(1) > expected,
                "BR-CO-17"
            );
        }
        // BR-S-08, BR-Z-08, BR-E-08, BR-IC-08, BR-AG-08: BT-116 per (category,
        // rate) = Σ BT-131 + Σ BT-99 − Σ BT-92 of its members.
        let member = |v: Option<&Vat>| {
            v.is_some_and(|v| v.category() == g.category() && v.rate_percent() == g.rate_percent())
        };
        let mut base: Decimal = (0..lines.len())
            .filter(|&i| member(lines[i].vat()))
            .map(|i| d.line_net(i).to_decimal())
            .sum();
        for (i, a) in d.adjustments().iter().enumerate() {
            if member(a.vat()) {
                base += signed(a.kind(), d.adjustment_amount(i));
            }
        }
        prop_assert_eq!(g.taxable_base().to_decimal(), base, "BR-S-08 and siblings");
        // A group exists iff it has a member (BR-S-08's `exists(...)` clause).
        prop_assert!(
            lines.iter().any(|l| member(l.vat()))
                || d.adjustments().iter().any(|a| member(a.vat()))
        );
        // BR-Z-09/BR-E-09/BR-IC-09: zero-tax categories carry no tax.
        if !g.category().carries_tax() {
            prop_assert!(g.tax().is_zero());
        }
    }
    // BR-DEC-09 … BR-DEC-20, BR-DEC-23 … BR-DEC-25, BR-DEC-27, BR-DEC-28: every
    // emitted amount has at most two decimals.
    let mut amounts = vec![
        d.line_total(),
        d.allowance_total(),
        d.charge_total(),
        d.tax_exclusive_total(),
        d.vat_total(),
        d.tax_inclusive_total(),
        d.prepaid(),
        d.rounding(),
        d.payable(),
    ];
    amounts.extend((0..lines.len()).map(|i| d.line_net(i)));
    amounts.extend((0..lines.len()).flat_map(|i| d.line_adjustment_amounts(i).to_vec()));
    amounts.extend((0..d.adjustments().len()).map(|i| d.adjustment_amount(i)));
    amounts.extend(
        d.vat_groups()
            .iter()
            .flat_map(|g| [g.taxable_base(), g.tax()]),
    );
    amounts.extend(d.vat_accounting_total().map(|(_, a)| a));
    prop_assert!(amounts.iter().all(|a| a.fits(2)), "BR-DEC");
    // PEPPOL-EN16931-R120 for derived lines: |BT-131 − (BT-129 × BT-146 / BT-149
    // + Σ charges − Σ allowances)| ≤ 0.02.
    for (i, l) in lines.iter().enumerate() {
        if l.stated_net().is_some() {
            continue;
        }
        let mut expected =
            l.quantity().value() * l.price().price().value() / l.price().base_quantity();
        for (a, r) in l.adjustments().iter().zip(d.line_adjustment_amounts(i)) {
            expected += signed(a.kind(), *r);
        }
        prop_assert!(
            (d.line_net(i).to_decimal() - expected).abs() <= dec!(0.02),
            "PEPPOL-EN16931-R120"
        );
    }
    prop_assert!(d.price_inconsistent().is_empty(), "PEPPOL-EN16931-R120");
    if let Some(acct) = d.vat_accounting() {
        // PEPPOL-EN16931-R005: BT-6 differs from BT-5.
        prop_assert_ne!(acct.currency, d.currency(), "PEPPOL-EN16931-R005");
        // BR-53: with BT-6 present, BT-111 is provided.
        let (_, bt111) = d.vat_accounting_total().expect("BR-53");
        // PEPPOL-EN16931-R055: BT-110 and BT-111 have the same sign.
        let same = (bt111.is_negative() || bt111.is_zero()) && !d.vat_total().is_positive()
            || !bt111.is_negative() && !d.vat_total().is_negative();
        prop_assert!(same, "PEPPOL-EN16931-R055");
    } else {
        prop_assert!(d.vat_accounting_total().is_none());
    }
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(400))]

    #[test]
    fn derived_documents_satisfy_the_rules(d in document()) {
        check(&d)?;
        check(&d.negated())?;
    }

    #[test]
    fn split_shares_satisfy_the_rules(d in document(), w in prop::collection::vec(dec_in(0, 100, 1), 1..5)) {
        prop_assume!(w.iter().any(|x| !x.is_zero()));
        for share in d.split(&w).unwrap() {
            check(&share)?;
        }
    }
}

/// A line built from a fractional usage: Mean, WeightedMean or a converted
/// Sum, at a random price and base quantity.
fn usage_line() -> impl Strategy<Value = Line> {
    (
        prop::collection::vec((dec_in(-5_000, 5_000, 3), dec_in(0, 1_000, 2)), 1..12),
        0u8..3,
        dec_in(0, 1_000_000, 6),
        prop::sample::select(vec![dec!(1), dec!(3), dec!(7), dec!(100)]),
        (dec_in(1, 10_000, 2), 0u8..=6),
    )
        .prop_map(|(readings, kind, price, base, (divisor, decimals))| {
            let samples = readings
                .iter()
                .map(|&(v, w)| Sample::new(v).with_weight(w + dec!(0.01)));
            let meter = match kind {
                0 => Meter::new(Aggregation::Mean),
                1 => Meter::new(Aggregation::WeightedMean),
                _ => Meter::new(Aggregation::Sum).with_conversion(
                    UnitConversion::new(divisor, decimals, Rounding::HalfEven).unwrap(),
                ),
            };
            let usage = meter.apply(samples).unwrap().unwrap();
            Line::from_usage(
                "usage",
                usage,
                "C62",
                UnitPrice::new(Price::new(price)).with_base_quantity(base),
            )
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1_000))]

    /// Constitution III (spec 027): PEPPOL-EN16931-R120 holds for lines built
    /// from fractional usages at scales 2–5 under every mode — BT-129 is the
    /// decimal reading, BT-131 is derived from the exact fraction.
    #[test]
    fn usage_lines_satisfy_r120(
        line in usage_line(),
        dp in 2u8..=5,
        mode in prop::sample::select(vec![
            Rounding::HalfAwayFromZero,
            Rounding::HalfEven,
            Rounding::AwayFromZero,
            Rounding::TowardZero,
        ]),
    ) {
        let s = AmountScale::new(dp, mode).unwrap();
        let d = Document::builder(Currency::EUR, s).line(line).build().unwrap();
        let l = &d.lines()[0];
        let expected =
            l.quantity().value() * l.price().price().value() / l.price().base_quantity();
        prop_assert!(
            (d.line_net(0).to_decimal() - expected).abs() <= dec!(0.02),
            "PEPPOL-EN16931-R120"
        );
        prop_assert!(d.price_inconsistent().is_empty(), "PEPPOL-EN16931-R120");
        if dp == 2 {
            check(&d)?;
            check(&d.negated())?;
        }
    }
}

#[test]
fn rule_ids_on_errors() {
    let err = Document::builder(Currency::EUR, AmountScale::EN16931)
        .line(Line::fixed("x", dec!(1)))
        .vat_accounting(VatAccounting {
            currency: Currency::EUR,
            rate: dec!(1),
        })
        .build()
        .unwrap_err();
    assert_eq!(err.rule(), Some("PEPPOL-EN16931-R005"));
    assert_eq!(
        Vat::standard(Percent::new(dec!(-1))).unwrap_err().rule(),
        Some("BR-S-05")
    );
    // Rates are typed percentages: a standard rate below 1 % (Corsica's
    // 0.9 %) is a rate like any other — nothing is guessed (030, D64 superseded).
    for rate in [
        dec!(0.19),
        dec!(0.5),
        dec!(0.9),
        dec!(0.999),
        dec!(0.0001),
        dec!(1),
    ] {
        let v = Vat::standard(Percent::new(rate)).unwrap();
        assert_eq!(v.rate_percent(), Percent::new(rate));
    }
    assert_eq!(
        Vat::standard(Percent::new(dec!(0))).unwrap_err().rule(),
        Some("BR-S-05")
    );
    assert_eq!(
        Vat::exempt(TaxCategory::Export, Exemption::Code(String::new()))
            .unwrap_err()
            .rule(),
        Some("BR-G-10")
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Constitution III: the credit input is pinned — documents with credits
    /// of both placements still meet every rule above (BR-CO-13 allowances in
    /// BT-109, BR-CO-14 VAT per group, BR-CO-16 BT-115 with the credits in
    /// BT-113), and so do their split shares.
    #[test]
    fn documents_with_credits_satisfy_the_rules(
        d in document(),
        cs in prop::collection::vec((any::<bool>(), fixed(1, 10_000_000, 2)), 1..4),
        w in prop::collection::vec(dec_in(0, 100, 1), 1..4),
    ) {
        let b = cs.into_iter().fold(d.into_builder(), |b, (before, balance)| {
            b.credit(if before {
                billing::Credit::before_tax("credit", balance)
            } else {
                billing::Credit::after_tax("credit", balance)
            })
        });
        let doc = b.build().unwrap();
        check(&doc)?;
        if let Ok(shares) = doc.split(&w) {
            for s in &shares {
                check(s)?;
            }
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Constitution III (030): the `Payment` input is pinned — documents with
    /// mixed paid entries (itemised advances matched to their groups, a gross
    /// payment, after-tax credits drawn as gross entries) meet every rule
    /// above, BR-CO-16 (BT-115 = BT-112 − BT-113 + BT-114) with BT-113 = Σ
    /// entries, and so do their split shares and their negation.
    #[test]
    fn documents_with_mixed_payments_satisfy_the_rules(
        d in document(),
        advances in prop::collection::vec((fixed(0, 10_000_000, 2), fixed(0, 2_000_000, 2)), 0..4),
        gross in prop::option::of(fixed(-1_000_000, 1_000_000, 2)),
        credits in prop::collection::vec(fixed(1, 10_000_000, 2), 0..3),
        w in prop::collection::vec(dec_in(0, 100, 1), 1..4),
    ) {
        let groups = d.vat_groups().to_vec();
        let mut b = d.into_builder();
        for (g, (net, tax)) in groups.iter().cycle().zip(advances) {
            b = b.payment(Payment::Advance(billing::Advance { vat: g.vat().clone(), net, tax }));
        }
        if let Some(amount) = gross {
            b = b.payment(Payment::Gross { description: "transfer".into(), amount });
        }
        for balance in credits {
            b = b.credit(billing::Credit::after_tax("wallet", balance));
        }
        let doc = b.build().unwrap();
        let entries: Amount = doc
            .paid_entries()
            .iter()
            .map(|p| match p {
                Payment::Advance(a) => Amount::from_decimal(a.net + a.tax).unwrap(),
                Payment::Gross { amount, .. } => Amount::from_decimal(*amount).unwrap(),
            })
            .sum();
        prop_assert_eq!(doc.prepaid(), entries, "BT-113 = Σ paid entries");
        check(&doc)?;
        check(&doc.negated())?;
        if let Ok(shares) = doc.split(&w) {
            for s in &shares {
                check(s)?;
                check(&s.negated())?;
            }
        }
    }
}

// ── Pricing models (spec 028): R120 on stated-net lines ───────────────────────

const MODES: [Rounding; 4] = [
    Rounding::HalfAwayFromZero,
    Rounding::HalfEven,
    Rounding::AwayFromZero,
    Rounding::TowardZero,
];

/// Every line of `d` meets PEPPOL-EN16931-R120: BT-131 within 0.02 of
/// quantity × price ÷ base (adjustments: none here).
fn r120(d: &Document) -> Result<(), TestCaseError> {
    for (i, l) in d.lines().iter().enumerate() {
        let expected = l.quantity().value() * l.price().price().value() / l.price().base_quantity();
        prop_assert!(
            (d.line_net(i).to_decimal() - expected).abs() <= dec!(0.02),
            "PEPPOL-EN16931-R120 line {}",
            i
        );
    }
    prop_assert!(d.price_inconsistent().is_empty(), "PEPPOL-EN16931-R120");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    /// Constitution III (spec 028): R120 holds for the line of a transaction
    /// fee total at scales 2–5 under every mode.
    #[test]
    fn fee_total_lines_satisfy_r120(
        percent in (0i64..=10_000).prop_map(|m| Decimal::new(m, 2)),
        fixed in dec_in(0, 500, 3),
        bounds in (prop::option::of(dec_in(0, 300, 3)), prop::option::of(dec_in(0, 3_000, 2))),
        free in 0u32..3,
        amounts in prop::collection::vec(dec_in(-100_000, 100_000, 3), 0..12),
        dp in 2u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        let mut fee = billing::TransactionFee::new(Percent::new(percent)).with_fixed(fixed).with_free_transactions(free);
        match bounds {
            (Some(a), Some(b)) => fee = fee.with_min(a.min(b)).with_max(a.max(b)),
            (Some(a), None) => fee = fee.with_min(a),
            (None, Some(b)) => fee = fee.with_max(b),
            (None, None) => {}
        }
        let total = fee.rate(amounts).unwrap();
        let s = AmountScale::new(dp, mode).unwrap();
        let d = Document::builder(Currency::EUR, s).line(total.into_line("fees", "C62")).build().unwrap();
        prop_assert_eq!(d.line_net(0), s.reduce(total.fee).unwrap());
        r120(&d)?;
        if dp == 2 {
            check(&d)?;
        }
    }

    /// Constitution III (spec 028): R120 holds for every increment line of
    /// every tariff shape over random cumulative sequences (fractional ones
    /// included) at scales 2–5 under every mode.
    #[test]
    fn increment_lines_satisfy_r120(
        b0 in 1i64..50,
        prices in prop::collection::vec(dec_in(0, 100_000, 4), 2),
        flats in prop::collection::vec(prop::option::of(dec_in(0, 5_000, 3)), 2),
        size in dec_in(1, 500, 1),
        mut ms in prop::collection::vec(0i64..=2_000, 1..6),
        d in prop::sample::select(vec![1usize, 3, 7]),
        dp in 2u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        use billing::{Billed, PackageRounding, Tariff, Tier, Tiers, Usage};
        ms.sort_unstable();
        let mut cs = vec![Usage::from(Decimal::ZERO)];
        cs.extend(ms.into_iter().map(|m| {
            let mut r = vec![Sample::new(Decimal::new(m, 1))];
            r.resize(d, Sample::new(Decimal::ZERO));
            Aggregation::Mean.apply(r).unwrap().unwrap()
        }));
        let tier = |t: Tier, f: Option<Decimal>| match f { Some(f) => t.with_flat(f), None => t };
        let tiers = Tiers::new([
            tier(Tier::up_to(Decimal::from(b0), Price::new(prices[0])), flats[0]),
            tier(Tier::above(Price::new(prices[1])), flats[1]),
        ]).unwrap();
        let tariffs = [
            Tariff::PerUnit(Price::new(prices[0])),
            Tariff::Graduated(tiers.clone()),
            Tariff::Volume(tiers),
            Tariff::Package { size, price: Price::new(prices[1]), rounding: PackageRounding::Up },
        ];
        let s = AmountScale::new(dp, mode).unwrap();
        for t in &tariffs {
            let mut billed = Billed::Nothing;
            for c in &cs {
                let (lines, next) = t.increment_lines(billed, *c, s, "usage", "KWH").unwrap();
                billed = next;
                if lines.is_empty() {
                    continue;
                }
                let doc = Document::builder(Currency::EUR, s).lines(lines).build().unwrap();
                r120(&doc)?;
                if dp == 2 {
                    check(&doc)?;
                }
            }
        }
    }
}
