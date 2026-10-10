//! Document properties over random inputs: BG-23 is a group-by, negation is an
//! involution, concat ≡ build, split and segments conserve, concat(split(d))
//! has d's totals, the builder is order-independent, bounds reach their
//! target at scale, serde round-trips byte-identically; and the exact
//! reductions (line price part, BT-111, Skonto correction, interest) against
//! references written independently of the crate's arithmetic; metering
//! (every aggregation kind, conversion, lines and tariffs priced from an exact
//! usage) against an exact rational reference.

use billing::{
    Advance, Amount, AmountScale, Basis, Bound, CashRounding, Currency, Decimal, Document,
    DocumentAdjustment, DocumentBuilder, EarlyPaymentDiscount, Exemption, InterestSegment, Line,
    LineAdjustment, Payment, Percent, Price, Quantity, Rounding, TaxCategory, UnitPrice, Vat,
    VatAccounting, late_payment_interest,
};
use billing::{
    Aggregation, Billed, Meter, PackageRounding, Percentile, Sample, Tariff, Tier, Tiers,
    UnitConversion, Usage,
};
use proptest::prelude::*;
use rust_decimal::RoundingStrategy;
use rust_decimal::dec;

const MODES: [Rounding; 4] = [
    Rounding::HalfAwayFromZero,
    Rounding::HalfEven,
    Rounding::AwayFromZero,
    Rounding::TowardZero,
];

fn strategy(r: Rounding) -> RoundingStrategy {
    match r {
        Rounding::HalfAwayFromZero => RoundingStrategy::MidpointAwayFromZero,
        Rounding::HalfEven => RoundingStrategy::MidpointNearestEven,
        Rounding::AwayFromZero => RoundingStrategy::AwayFromZero,
        Rounding::TowardZero => RoundingStrategy::ToZero,
    }
}

fn dec_in(lo: i64, hi: i64, max_scale: u32) -> impl Strategy<Value = Decimal> {
    (lo..=hi, 0..=max_scale).prop_map(|(m, s)| Decimal::new(m, s))
}

fn vats() -> Vec<Vat> {
    vec![
        Vat::standard(Percent::new(dec!(19))).unwrap(),
        Vat::standard(Percent::new(dec!(7))).unwrap(),
        Vat::standard(Percent::new(dec!(7.7))).unwrap(),
        Vat::zero_rated(),
        Vat::exempt(TaxCategory::Exempt, Exemption::Text("§4".into())).unwrap(),
        Vat::regional(TaxCategory::CanaryIslands, Percent::new(dec!(0))).unwrap(),
        Vat::exempt(
            TaxCategory::ReverseCharge,
            Exemption::Code("VATEX-EU-AE".into()),
        )
        .unwrap(),
    ]
}

fn line(vat: Option<usize>) -> impl Strategy<Value = Line> {
    (
        dec_in(-100_000, 100_000, 3),
        dec_in(0, 1_000_000, 6),
        prop::sample::select(vec![dec!(1), dec!(3), dec!(100)]),
        prop::option::weighted(0.2, dec_in(-10_000, 10_000, 6)),
        prop::collection::vec((any::<bool>(), dec_in(0, 2_000, 4)), 0..3),
    )
        .prop_map(move |(q, p, base, net, adj)| {
            let mut l = Line::new(
                "l",
                Quantity::new(q, "KWH"),
                UnitPrice::new(Price::new(p)).with_base_quantity(base),
            );
            if let Some(n) = net {
                l = l.with_net(n);
            }
            for (charge, amount) in adj {
                l = l.with_adjustment(if charge {
                    LineAdjustment::charge(amount)
                } else {
                    LineAdjustment::allowance(amount)
                });
            }
            if let Some(v) = vat {
                l = l.with_vat(vats()[v].clone());
            }
            l
        })
}

/// A random valid document builder at a random scale (2..=5 decimals).
fn document() -> impl Strategy<Value = DocumentBuilder> {
    (
        2u8..=5,
        prop::sample::select(MODES.to_vec()),
        any::<bool>(),
        prop::collection::vec(prop::option::of(0..7usize), 1..8),
        prop::collection::vec(
            (
                any::<bool>(),
                dec_in(0, 5_000, 3),
                prop::option::of(0..7usize),
            ),
            0..3,
        ),
        prop::option::of(dec_in(-50_000, 50_000, 2)),
        any::<bool>(),
        any::<bool>(),
    )
        .prop_flat_map(|(dp, mode, vat_free, line_vats, adjs, pre, cash, acct)| {
            let lines: Vec<_> = line_vats
                .iter()
                .map(|v| line(if vat_free { None } else { *v }))
                .collect();
            (Just((dp, mode, vat_free, adjs, pre, cash, acct)), lines)
        })
        .prop_map(|((dp, mode, vat_free, adjs, pre, cash, acct), lines)| {
            let mut b =
                Document::builder(Currency::EUR, AmountScale::new(dp, mode).unwrap()).lines(lines);
            if !vat_free {
                b = b.default_vat(Vat::standard(Percent::new(dec!(19))).unwrap());
            }
            for (charge, amount, v) in adjs {
                let mut a = if charge {
                    DocumentAdjustment::charge("c", amount)
                } else {
                    DocumentAdjustment::allowance("a", amount)
                };
                if let (false, Some(v)) = (vat_free, v) {
                    a = a.with_vat(vats()[v].clone());
                }
                b = b.adjustment(a);
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
            if acct {
                b = b.vat_accounting(VatAccounting {
                    currency: Currency::new("PLN").unwrap(),
                    rate: dec!(4.2817),
                });
            }
            b
        })
}

fn weights() -> impl Strategy<Value = Vec<Decimal>> {
    prop::collection::vec(dec_in(0, 1_000, 2), 1..6)
        .prop_filter("not all zero", |w| w.iter().any(|x| !x.is_zero()))
}

fn round(x: Decimal, s: AmountScale) -> Decimal {
    x.round_dp_with_strategy(u32::from(s.decimals), strategy(s.rounding))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// BG-23 is the group-by of lines and adjustments by (category, rate),
    /// and BT-117 = round(BT-116 × rate / 100) once (oracle: rust_decimal).
    #[test]
    fn vat_breakdown_is_a_group_by(b in document()) {
        let d = b.build().unwrap();
        let s = d.scale();
        for g in d.vat_groups() {
            let key = (g.category(), g.rate_percent());
            let of = |v: Option<&Vat>| v.is_some_and(|v| (v.category(), v.rate_percent()) == key);
            let mut base = Decimal::ZERO;
            for (i, l) in d.lines().iter().enumerate() {
                if of(l.vat()) { base += d.line_net(i).to_decimal(); }
            }
            for (i, a) in d.adjustments().iter().enumerate() {
                if of(a.vat()) {
                    let x = d.adjustment_amount(i).to_decimal();
                    base += if a.kind() == billing::AdjustmentKind::Charge { x } else { -x };
                }
            }
            prop_assert_eq!(g.taxable_base().to_decimal(), base);
            prop_assert_eq!(g.tax().to_decimal(), round(base * g.rate_percent().value() / dec!(100), s));
        }
        let members = d.lines().iter().filter_map(Line::vat).chain(d.adjustments().iter().filter_map(DocumentAdjustment::vat));
        for v in members {
            prop_assert!(d.vat_groups().iter().any(|g| g.vat().category() == v.category() && g.rate_percent() == v.rate_percent()));
        }
    }

    /// Each line's BT-131 is the sum of its leaves, each rounded once.
    #[test]
    fn line_net_rounds_each_leaf_once(b in document()) {
        let d = b.build().unwrap();
        let s = d.scale();
        for (i, l) in d.lines().iter().enumerate() {
            let pp = match l.stated_net() {
                Some(n) => round(n, s),
                None => round(l.quantity().value() * l.price().price().value() / l.price().base_quantity(), s),
            };
            let mut net = pp;
            for a in l.adjustments() {
                let r = round(a.amount(), s);
                net += if a.kind() == billing::AdjustmentKind::Charge { r } else { -r };
            }
            prop_assert_eq!(d.line_net(i).to_decimal(), net);
        }
    }

    /// negated() is an involution, negates every total, and equals building
    /// the negated inputs (all roundings are sign-symmetric).
    #[test]
    fn negation_is_an_exact_involution(b in document()) {
        let d = b.build().unwrap();
        let n = d.negated();
        prop_assert_eq!(&n.negated(), &d);
        prop_assert_eq!(n.payable(), -d.payable());
        prop_assert_eq!(n.vat_total(), -d.vat_total());
        prop_assert_eq!(n.rounding(), -d.rounding());
        prop_assert_eq!(n.clone().into_builder().build().unwrap(), n);
    }

    /// concat ≡ build of the concatenated inputs.
    #[test]
    fn concat_equals_build(b in document(), cut in 0usize..8) {
        let d = b.build().unwrap();
        let cut = cut.min(d.lines().len());
        let with = |lines: &[Line], adjustments: bool, settlement: bool| {
            let mut x = Document::builder(d.currency(), d.scale()).lines(lines.to_vec());
            if adjustments { x = x.adjustments(d.adjustments().to_vec()); }
            if settlement { x = x.payments(d.payments().to_vec()); }
            if let Some(c) = d.cash_rounding() { x = x.cash_rounding(*c); }
            if let Some(v) = d.vat_accounting() { x = x.vat_accounting(*v); }
            x.build()
        };
        let (Ok(first), Ok(second)) = (with(&d.lines()[..cut], true, true), with(&d.lines()[cut..], false, false)) else {
            return Ok(()); // a half that is VAT-free next to a VAT half
        };
        let c = Document::concat([first, second]).unwrap();
        prop_assert_eq!(c, with(d.lines(), true, true).unwrap());
    }

    /// split: every conserved total sums to the parent exactly; each share's
    /// line nets are within their leaves' quota; every share's partitioned tax
    /// is within one unit of its own derivation; shares hold R120.
    #[test]
    fn split_conserves_and_keeps_quota(b in document(), w in weights()) {
        let d = b.build().unwrap();
        let shares = d.split(&w).unwrap();
        prop_assert_eq!(shares.len(), w.len());
        let sum = |f: fn(&Document) -> Amount| shares.iter().map(f).sum::<Amount>();
        prop_assert_eq!(sum(Document::line_total), d.line_total());
        prop_assert_eq!(sum(Document::allowance_total), d.allowance_total());
        prop_assert_eq!(sum(Document::charge_total), d.charge_total());
        prop_assert_eq!(sum(Document::tax_exclusive_total), d.tax_exclusive_total());
        prop_assert_eq!(sum(Document::vat_total), d.vat_total());
        prop_assert_eq!(sum(Document::tax_inclusive_total), d.tax_inclusive_total());
        prop_assert_eq!(sum(Document::prepaid), d.prepaid());
        if d.cash_rounding().is_none() {
            prop_assert_eq!(sum(Document::payable), d.payable());
        }
        let total_w: Decimal = w.iter().sum();
        let unit = Decimal::new(1, u32::from(d.scale().decimals));
        for (j, s) in shares.iter().enumerate() {
            let parent = d.price_inconsistent();
            prop_assert!(s.price_inconsistent().iter().all(|i| parent.contains(i)));
            for (k, g) in s.vat_groups().iter().enumerate() {
                prop_assert_eq!(g.vat(), d.vat_groups()[k].vat());
                let own = round(g.taxable_base().to_decimal() * g.rate_percent().value() / dec!(100), d.scale());
                prop_assert!((g.tax().to_decimal() - own).abs() <= unit);
            }
            for (i, l) in d.lines().iter().enumerate() {
                let leaves = Decimal::from(1 + l.adjustments().len());
                let exact = d.line_net(i).to_decimal() * w[j] / total_w;
                prop_assert!((s.line_net(i).to_decimal() - exact).abs() < leaves * unit);
            }
        }
        for (k, g) in d.vat_groups().iter().enumerate() {
            let base: Amount = shares.iter().map(|s| s.vat_groups()[k].taxable_base()).sum();
            let tax: Amount = shares.iter().map(|s| s.vat_groups()[k].tax()).sum();
            prop_assert_eq!((base, tax), (g.taxable_base(), g.tax()));
        }
    }

    /// segments conserve the line's BT-131 and quantity exactly.
    #[test]
    fn segments_conserve(l in line(Some(0)), w in weights(), dp in 2u8..=5) {
        let s = AmountScale::new(dp, Rounding::HalfEven).unwrap();
        let whole = Document::builder(Currency::EUR, s).line(l.clone()).build().unwrap();
        let segs = l.segments(&w, s).unwrap();
        let parts = Document::builder(Currency::EUR, s).lines(segs.clone()).build().unwrap();
        prop_assert_eq!(parts.line_total(), whole.line_total());
        prop_assert_eq!(segs.iter().map(|x| x.quantity().value()).sum::<Decimal>(), l.quantity().value());
        if !whole.price_inconsistent().is_empty() { return Ok(()); }
        prop_assert!(parts.price_inconsistent().is_empty());
    }

    /// A minimum (or maximum) on the gross is reached (or kept) exactly at
    /// the scale, under every rounding, by the smallest (largest) whole-unit
    /// line; on the net the line is exact.
    #[test]
    fn bounds_reach_their_target_minimally(
        nets in prop::collection::vec((dec_in(0, 5_000, 2), 0..3usize), 1..4),
        target in dec_in(1, 50_000, 2),
        mode in prop::sample::select(MODES.to_vec()),
        gross in any::<bool>(),
        minimum in any::<bool>(),
        bound_vat in 0..3usize,
    ) {
        let s = AmountScale::new(2, mode).unwrap();
        let v = vats();
        let lines: Vec<Line> = nets.iter().map(|(n, k)| Line::fixed("l", *n).with_vat(v[*k].clone())).collect();
        let basis = if gross { Basis::Gross } else { Basis::Net };
        let bound = if minimum { Bound::minimum(basis, target, "b") } else { Bound::maximum(basis, target, "b") }
            .with_vat(v[bound_vat].clone());
        let plain = Document::builder(Currency::EUR, s).lines(lines.clone()).build().unwrap();
        let built = Document::builder(Currency::EUR, s).lines(lines.clone()).bound(bound).build();
        let Ok(d) = built else {
            prop_assert!(minimum && plain.tax_exclusive_total().is_negative());
            return Ok(());
        };
        let value = |d: &Document| if gross { d.tax_inclusive_total() } else { d.tax_exclusive_total() }.to_decimal();
        if d.bound_lines() == 0 {
            let held = if minimum { value(&d) >= target } else { value(&d) <= target };
            prop_assert!(held);
            return Ok(());
        }
        let held = if minimum { value(&d) >= target } else { value(&d) <= target };
        prop_assert!(held);
        // One unit less (more) misses.
        let x = d.lines().last().unwrap().exact_net().unwrap();
        let step = if minimum { dec!(-0.01) } else { dec!(0.01) };
        let other = Document::builder(Currency::EUR, s)
            .lines(lines)
            .line(Line::fixed("m", x + step).with_vat(v[bound_vat].clone()))
            .build()
            .unwrap();
        let missed = if minimum { value(&other) < target } else { value(&other) > target };
        prop_assert!(missed);
    }
}

#[cfg(feature = "serde")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// JSON round-trip is byte-identical (documents and split shares).
    #[test]
    fn serde_round_trip_is_byte_identical(b in document(), w in weights()) {
        let d = b.build().unwrap();
        for doc in core::iter::once(d.clone()).chain(d.split(&w).unwrap()) {
            let json = serde_json::to_string(&doc).unwrap();
            let back: Document = serde_json::from_str(&json).unwrap();
            prop_assert_eq!(&back, &doc);
            prop_assert_eq!(serde_json::to_string(&back).unwrap(), json);
        }
    }
}

// ── Independent integer references ───────────────────────────────────────────

/// `n / d` (d > 0) rounded by `mode`, from the integer quotient and remainder.
/// Written independently of the crate's `Rounding::div`.
fn rational_round(n: i128, d: i128, mode: Rounding) -> i128 {
    assert!(d > 0);
    let neg = n < 0;
    let (q, r) = (n.abs() / d, n.abs() % d);
    let twice = 2 * r;
    let bump = match mode {
        Rounding::TowardZero => false,
        Rounding::AwayFromZero => r != 0,
        Rounding::HalfAwayFromZero => twice >= d,
        Rounding::HalfEven => twice > d || (twice == d && q % 2 == 1),
    };
    let m = q + i128::from(bump);
    if neg { -m } else { m }
}

fn pow10(e: u32) -> i128 {
    10i128.pow(e)
}

/// A decimal as (mantissa, scale).
fn parts(d: Decimal) -> (i128, u32) {
    (d.mantissa(), d.scale())
}

/// The order-independent builder inputs, applied in any permutation.
#[derive(Debug, Clone)]
enum Op {
    Lines(Vec<Line>),
    Adjustments(Vec<DocumentAdjustment>),
    DefaultVat(Vat),
    Payment(Payment),
    Cash(CashRounding),
    Accounting(VatAccounting),
    Bound(Bound),
}

fn apply(b: DocumentBuilder, op: Op) -> DocumentBuilder {
    match op {
        Op::Lines(l) => b.lines(l),
        Op::Adjustments(a) => b.adjustments(a),
        Op::DefaultVat(v) => b.default_vat(v),
        Op::Payment(p) => b.payment(p),
        Op::Cash(c) => b.cash_rounding(c),
        Op::Accounting(v) => b.vat_accounting(v),
        Op::Bound(x) => b.bound(x),
    }
}

fn ops() -> impl Strategy<Value = (AmountScale, Vec<Op>)> {
    (
        2u8..=5,
        prop::sample::select(MODES.to_vec()),
        prop::collection::vec(line(None), 1..5),
        prop::collection::vec((any::<bool>(), dec_in(0, 5_000, 2)), 0..3),
        prop::option::of(dec_in(-5_000, 5_000, 2)),
        any::<bool>(),
        any::<bool>(),
        prop::option::of((any::<bool>(), any::<bool>(), dec_in(1, 500_000, 2))),
    )
        .prop_flat_map(|(dp, mode, lines, adjs, pre, cash, acct, bound)| {
            let mut ops = vec![
                Op::Lines(lines),
                Op::Adjustments(
                    adjs.into_iter()
                        .map(|(c, x)| {
                            if c {
                                DocumentAdjustment::charge("c", x)
                            } else {
                                DocumentAdjustment::allowance("a", x)
                            }
                        })
                        .collect(),
                ),
                Op::DefaultVat(Vat::standard(Percent::new(dec!(19))).unwrap()),
            ];
            if let Some(p) = pre {
                ops.push(Op::Payment(Payment::Gross {
                    description: "prepayment".into(),
                    amount: p,
                }));
            }
            if cash {
                ops.push(Op::Cash(CashRounding {
                    increment: dec!(0.05),
                    rounding: mode,
                }));
            }
            if acct {
                ops.push(Op::Accounting(VatAccounting {
                    currency: Currency::new("PLN").unwrap(),
                    rate: dec!(4.2817),
                }));
            }
            if let Some((gross, minimum, x)) = bound {
                let basis = if gross { Basis::Gross } else { Basis::Net };
                ops.push(Op::Bound(if minimum {
                    Bound::minimum(basis, x, "b")
                } else {
                    Bound::maximum(basis, x, "b")
                }));
            }
            let scale = AmountScale::new(dp, mode).unwrap();
            (Just(scale), Just(ops).prop_shuffle())
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// The builder is order-independent: every permutation of the inputs
    /// (lines, adjustments, default VAT, prepayment, cash rounding, VAT
    /// accounting, a bound) builds the same document, or the same error.
    #[test]
    fn builder_is_order_independent((scale, shuffled) in ops()) {
        let mut canonical = shuffled.clone();
        canonical.sort_by_key(|op| match op {
            Op::Lines(_) => 0,
            Op::Adjustments(_) => 1,
            Op::DefaultVat(_) => 2,
            Op::Payment(_) => 3,
            Op::Cash(_) => 4,
            Op::Accounting(_) => 5,
            Op::Bound(_) => 6,
        });
        let build = |ops: Vec<Op>| {
            ops.into_iter()
                .fold(Document::builder(Currency::EUR, scale), apply)
                .build()
        };
        prop_assert_eq!(build(shuffled), build(canonical));
    }

    /// `concat(split(d))` has every total of d, BT-117 per group included.
    #[test]
    fn concat_of_split_restores_every_total(b in document(), w in weights()) {
        let d = b.build().unwrap();
        let c = Document::concat(d.split(&w).unwrap()).unwrap();
        let totals = |x: &Document| (
            x.line_total(), x.allowance_total(), x.charge_total(), x.tax_exclusive_total(),
            x.vat_total(), x.tax_inclusive_total(), x.prepaid(), x.rounding(), x.payable(),
        );
        prop_assert_eq!(totals(&c), totals(&d));
        prop_assert_eq!(c.vat_groups(), d.vat_groups());
        prop_assert_eq!(c.vat_accounting_total(), d.vat_accounting_total());
    }

    /// BT-111 is BT-110 × rate rounded once at the document scale (oracle:
    /// the exact integer product, rounded by remainder).
    #[test]
    fn vat_accounting_rounds_once(
        lines in prop::collection::vec((dec_in(-100_000, 100_000, 2), 0..3usize), 1..5),
        dp in 0u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
        rate in dec_in(1, 10_000_000, 6),
    ) {
        let s = AmountScale::new(dp, mode).unwrap();
        let v = vats();
        let d = Document::builder(Currency::EUR, s)
            .lines(lines.iter().map(|(n, k)| Line::fixed("l", *n).with_vat(v[*k].clone())))
            .vat_accounting(VatAccounting { currency: Currency::new("PLN").unwrap(), rate })
            .build()
            .unwrap();
        let (vm, vs) = parts(d.vat_total().to_decimal());
        let (rm, rs) = parts(rate);
        // vat × rate × 10^dp / 10^(vs + rs)
        let units = rational_round(vm * rm * pow10(u32::from(dp)), pow10(vs + rs), mode);
        let (_, bt111) = d.vat_accounting_total().unwrap();
        prop_assert_eq!(bt111.to_decimal(), Decimal::from_i128_with_scale(units, u32::from(dp)));
        if d.vat_total().is_zero() {
            prop_assert!(bt111.is_zero());
        }
    }
}

/// A small VAT document for the 5 000-case properties: 1–4 lines at S 19,
/// S 7 or Z, one sign, a prepayment, any scale and mode.
fn small_document() -> impl Strategy<Value = Document> {
    (
        prop::collection::vec((dec_in(0, 1_000_000, 3), 0..3usize), 1..5),
        any::<bool>(),
        prop::option::of(dec_in(-100_000, 100_000, 2)),
        2u8..=5,
        prop::sample::select(MODES.to_vec()),
    )
        .prop_map(|(lines, credit, pre, dp, mode)| {
            let v = [
                Vat::standard(Percent::new(dec!(19))).unwrap(),
                Vat::standard(Percent::new(dec!(7))).unwrap(),
                Vat::zero_rated(),
            ];
            let mut b = Document::builder(Currency::EUR, AmountScale::new(dp, mode).unwrap())
                .lines(lines.iter().map(|(n, k)| {
                    Line::fixed("l", if credit { -*n } else { *n }).with_vat(v[*k].clone())
                }));
            if let Some(p) = pre {
                b = b.payments(
                    vec![Advance {
                        vat: v[0].clone(),
                        net: p,
                        tax: Decimal::ZERO,
                    }]
                    .into_iter()
                    .map(Payment::Advance),
                );
            }
            b.build().unwrap()
        })
}

fn segment() -> impl Strategy<Value = InterestSegment> {
    (
        dec_in(-50_000, 250_000, 4),
        0u32..=400,
        prop::sample::select(vec![360u32, 365, 366]),
    )
        .prop_map(|(annual_percent, days, year_days)| InterestSegment {
            annual_percent: Percent::new(annual_percent),
            days,
            year_days,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(5_000))]

    /// Σ (base + tax) over the Skonto VAT correction is the Skonto amount,
    /// exactly, and the amount is BT-115 × percent / 100 rounded once.
    #[test]
    fn vat_correction_sums_to_the_discount(d in small_document(), percent in (0i64..=10_000).prop_map(|m| Decimal::new(m, 2))) {
        let skonto = EarlyPaymentDiscount { percent: Percent::new(percent) };
        let amount = skonto.amount(&d).unwrap();
        let s = d.scale();
        let (pm, ps) = parts(d.payable().to_decimal());
        let (qm, qs) = parts(percent);
        let units = rational_round(pm * qm * pow10(u32::from(s.decimals)), 100 * pow10(ps + qs), s.rounding);
        prop_assert_eq!(amount.to_decimal(), Decimal::from_i128_with_scale(units, u32::from(s.decimals)));
        let all_zero = d.vat_groups().iter().all(|g| (g.taxable_base() + g.tax()).is_zero());
        if all_zero && !amount.is_zero() {
            // Only a prepayment makes BT-115 non-zero here: no weight to split by.
            prop_assert!(skonto.vat_correction(&d).is_err());
            return Ok(());
        }
        let rows = skonto.vat_correction(&d).unwrap();
        prop_assert_eq!(rows.len(), d.vat_groups().len());
        let sum: Amount = rows.iter().map(|g| g.taxable_base() + g.tax()).sum();
        prop_assert_eq!(sum, amount);
    }

    /// Late-payment interest is the exact Σ P × pᵢ/100 × dᵢ/yᵢ rounded once,
    /// at every scale and mode. Reference: one fraction over the product of the
    /// year lengths (the crate uses their lcm), rounded by remainder.
    #[test]
    fn interest_matches_an_exact_rational_reference(
        principal in dec_in(-100_000_000, 100_000_000, 2),
        segments in prop::collection::vec(segment(), 1..5),
        dp in 0u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        let s = AmountScale::new(dp, mode).unwrap();
        let got = late_payment_interest(Amount::from_decimal(principal).unwrap(), &segments, s).unwrap();
        let (pm, ps) = parts(principal);
        let a_max = segments.iter().map(|x| x.annual_percent.value().scale()).max().unwrap();
        let prod: i128 = segments.iter().map(|x| i128::from(x.year_days)).product();
        let mut n: i128 = 0;
        for x in &segments {
            let (am, as_) = parts(x.annual_percent.value());
            n += am * pow10(a_max - as_) * i128::from(x.days) * (prod / i128::from(x.year_days));
        }
        let num = pm * n * pow10(u32::from(dp));
        let den = 100 * prod * pow10(ps + a_max);
        let units = rational_round(num, den, mode);
        prop_assert_eq!(got.to_decimal(), Decimal::from_i128_with_scale(units, u32::from(dp)));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2_000))]

    /// SC-007: the price part of a line (no adjustments) is q × p / base
    /// reduced once, against an integer reference: mantissas in i128, rounded
    /// by remainder, bases {1, 3, 7, 1000}, every scale 0–5 and every mode.
    #[test]
    fn line_price_part_matches_an_integer_reference(
        q in dec_in(-1_000_000, 1_000_000, 3),
        p in dec_in(0, 1_000_000, 6),
        base in prop::sample::select(vec![1i64, 3, 7, 1000]),
        dp in 0u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        let s = AmountScale::new(dp, mode).unwrap();
        let line = Line::new(
            "l",
            Quantity::new(q, "KWH"),
            UnitPrice::new(Price::new(p)).with_base_quantity(Decimal::from(base)),
        );
        let d = Document::builder(Currency::EUR, s).line(line).build().unwrap();
        let (qm, qs) = parts(q);
        let (pm, ps) = parts(p);
        // q × p / base × 10^dp = qm × pm × 10^dp / (base × 10^(qs + ps))
        let units = rational_round(qm * pm * pow10(u32::from(dp)), i128::from(base) * pow10(qs + ps), mode);
        prop_assert_eq!(d.line_net(0).to_decimal(), Decimal::from_i128_with_scale(units, u32::from(dp)));
    }
}

// ── Metering (spec 027) against an exact rational reference ─────────────────

/// An exact rational `n / d`, `d > 0`, in `i128` — independent of the crate.
#[derive(Debug, Clone, Copy)]
struct Q {
    n: i128,
    d: i128,
}

impl Q {
    fn of(x: Decimal) -> Self {
        let (m, s) = parts(x);
        Self { n: m, d: pow10(s) }
    }
    fn int(n: i128) -> Self {
        Self { n, d: 1 }
    }
    fn add(self, o: Self) -> Self {
        Self {
            n: self.n * o.d + o.n * self.d,
            d: self.d * o.d,
        }
        .reduced()
    }
    fn mul(self, o: Self) -> Self {
        Self {
            n: self.n * o.n,
            d: self.d * o.d,
        }
        .reduced()
    }
    fn div(self, o: Self) -> Self {
        assert!(o.n != 0);
        let (n, d) = (self.n * o.d, self.d * o.n);
        let s = if d < 0 { -1 } else { 1 };
        Self { n: n * s, d: d * s }.reduced()
    }
    fn reduced(self) -> Self {
        fn gcd(a: i128, b: i128) -> i128 {
            if b == 0 { a.abs() } else { gcd(b, a % b) }
        }
        let g = gcd(self.n, self.d).max(1);
        Self {
            n: self.n / g,
            d: self.d / g,
        }
    }
    fn eq(self, o: Self) -> bool {
        self.n * o.d == o.n * self.d
    }
    fn le(self, o: Self) -> bool {
        self.n * o.d <= o.n * self.d
    }
    /// Rounded once to `dp` decimals by `mode`.
    fn round(self, dp: u8, mode: Rounding) -> Decimal {
        let units = rational_round(self.n * pow10(u32::from(dp)), self.d, mode);
        Decimal::from_i128_with_scale(units, u32::from(dp))
    }
}

fn usage_q(u: Usage) -> Q {
    Q::of(u.numerator()).div(Q::of(u.denominator()))
}

fn readings() -> impl Strategy<Value = Vec<(Decimal, Decimal)>> {
    prop::collection::vec((dec_in(-10_000, 10_000, 3), dec_in(0, 1_000, 2)), 0..25)
}

fn nearest_rank_ref(values: &[Decimal], p: u8) -> Option<Decimal> {
    let mut v = values.to_vec();
    v.sort();
    let n = v.len();
    if n == 0 {
        return None;
    }
    let rank = (usize::from(p) * n).div_ceil(100);
    Some(v[rank - 1])
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1_000))]

    /// SC-002: every kind equals the exact rational reference; Min ≤ v ≤ Max,
    /// Latest = last, unit weights give Sum and Mean, Percentile(100) = Max,
    /// Percentile(1) = Min.
    #[test]
    fn meter_every_kind_matches_an_exact_reference(r in readings(), p in 1u8..=100) {
        let values: Vec<Decimal> = r.iter().map(|x| x.0).collect();
        let weighted = || r.iter().map(|&(v, w)| Sample::new(v).with_weight(w));
        let plain = || values.iter().map(|&v| Sample::new(v));
        let get = |a: Aggregation, weighted_input: bool| {
            if weighted_input { a.apply(weighted()) } else { a.apply(plain()) }.unwrap()
        };
        let n = values.len();
        let sum = values.iter().fold(Q::int(0), |a, &v| a.add(Q::of(v)));
        let wsum = r.iter().fold(Q::int(0), |a, &(v, w)| a.add(Q::of(v).mul(Q::of(w))));
        let wtot = r.iter().fold(Q::int(0), |a, &(_, w)| a.add(Q::of(w)));
        let same = |u: Option<Usage>, q: Q| u.is_some_and(|u| usage_q(u).eq(q));

        prop_assert!(same(get(Aggregation::Sum, false), sum));
        prop_assert!(same(get(Aggregation::Count, false), Q::int(n as i128)));
        prop_assert!(same(get(Aggregation::WeightedSum, true), wsum));
        prop_assert!(same(get(Aggregation::WeightedSum, false), sum));
        if n == 0 {
            for k in [Aggregation::Min, Aggregation::Max, Aggregation::Latest, Aggregation::Mean, Aggregation::WeightedMean] {
                prop_assert_eq!(get(k, false), None);
            }
        } else {
            let mean = sum.div(Q::int(n as i128));
            prop_assert!(same(get(Aggregation::Mean, false), mean));
            prop_assert_eq!(get(Aggregation::WeightedMean, false), get(Aggregation::Mean, false));
            let min = get(Aggregation::Min, false).unwrap().to_decimal();
            let max = get(Aggregation::Max, false).unwrap().to_decimal();
            prop_assert!(values.iter().all(|&v| min <= v && v <= max));
            prop_assert!(values.contains(&min) && values.contains(&max));
            prop_assert_eq!(get(Aggregation::Latest, false).unwrap().to_decimal(), values[n - 1]);
            let pc = |p: u8| get(Aggregation::Percentile(Percentile::new(p).unwrap()), false).map(|u| u.to_decimal());
            prop_assert_eq!(pc(p), nearest_rank_ref(&values, p));
            prop_assert_eq!(pc(100), Some(max));
            prop_assert_eq!(pc(1), Some(min));
        }
        let wm = get(Aggregation::WeightedMean, true);
        if wtot.n == 0 {
            prop_assert_eq!(wm, None);
        } else {
            prop_assert!(same(wm, wsum.div(wtot)));
        }
    }

    /// SC-002: UniqueCount is the number of distinct keys.
    #[test]
    fn meter_unique_count_counts_distinct_keys(keys in prop::collection::vec("[a-e]{1,2}", 0..30)) {
        let u = Aggregation::UniqueCount
            .apply(keys.iter().map(|k| Sample::new(dec!(1)).with_key(k.as_str())))
            .unwrap()
            .unwrap();
        let distinct: std::collections::BTreeSet<&String> = keys.iter().collect();
        prop_assert_eq!(u.to_decimal(), Decimal::from(distinct.len()));
    }

    /// SC-006: a converted usage is the exact aggregate ÷ divisor rounded once,
    /// for all four modes (Mean included: rounded from the fraction).
    #[test]
    fn meter_conversion_rounds_once_from_the_exact_aggregate(
        r in readings(),
        divisor in dec_in(1, 100_000, 2),
        dp in 0u8..=4,
        mode in prop::sample::select(MODES.to_vec()),
        mean in any::<bool>(),
    ) {
        let kind = if mean { Aggregation::Mean } else { Aggregation::Sum };
        let c = UnitConversion::new(divisor, dp, mode).unwrap();
        let s = || r.iter().map(|&(v, _)| Sample::new(v));
        let exact = kind.apply(s()).unwrap();
        let converted = Meter::new(kind).with_conversion(c).apply(s()).unwrap();
        match exact {
            None => prop_assert_eq!(converted, None),
            Some(u) => {
                let expected = usage_q(u).div(Q::of(divisor)).round(dp, mode);
                let got = converted.unwrap();
                prop_assert_eq!(got.denominator(), Decimal::ONE);
                prop_assert_eq!(got.to_decimal(), expected);
            }
        }
    }

    /// SC-002: a line built from a Mean usage has the net of the exact
    /// fraction × price rounded once, at every scale 2–5 and mode.
    #[test]
    fn meter_line_from_usage_rounds_once(
        r in readings(),
        p in dec_in(0, 1_000_000, 6),
        dp in 2u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        let Some(u) = Aggregation::Mean.apply(r.iter().map(|&(v, _)| Sample::new(v))).unwrap() else {
            return Ok(());
        };
        let s = AmountScale::new(dp, mode).unwrap();
        let line = Line::from_usage("avg", u, "C62", UnitPrice::new(Price::new(p)));
        let d = Document::builder(Currency::EUR, s).line(line).build().unwrap();
        prop_assert_eq!(d.line_net(0).to_decimal(), usage_q(u).mul(Q::of(p)).round(dp, mode));
    }

    /// SC-007: pricing a fractional usage through every tariff shape equals the
    /// exact rational computation, each piece rounded once.
    #[test]
    fn meter_tariff_of_a_fraction_matches_an_exact_reference(
        r in prop::collection::vec(dec_in(-3_000, 3_000, 1), 1..8),
        b0 in 1i64..50,
        prices in (dec_in(0, 100_000, 4), dec_in(0, 100_000, 4)),
        size in dec_in(1, 5_000, 2),
        up in any::<bool>(),
        dp in 2u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        let u = Aggregation::Mean.apply(r.iter().map(|&v| Sample::new(v))).unwrap().unwrap();
        let x = usage_q(u);
        let neg = x.n < 0;
        let q = Q { n: x.n.abs(), d: x.d };
        let b0 = Decimal::from(b0);
        let tiers = Tiers::new([
            Tier::up_to(b0, Price::new(prices.0)),
            Tier::above(Price::new(prices.1)),
        ]).unwrap();
        let rounding = if up { PackageRounding::Up } else { PackageRounding::Down };
        let pr = |x: Decimal| Q::of(x);
        let piece = |qty: Q, price: Decimal| -> Decimal {
            let signed = if neg { Q { n: -qty.n, d: qty.d } } else { qty };
            signed.mul(pr(price)).round(dp, mode)
        };
        // Graduated: [0, b0] at prices.0, the rest at prices.1.
        let graduated = if q.le(pr(b0)) {
            piece(q, prices.0)
        } else {
            piece(pr(b0), prices.0) + piece(q.add(Q { n: -pr(b0).n, d: pr(b0).d }), prices.1)
        };
        let volume = piece(q, if q.le(pr(b0)) { prices.0 } else { prices.1 });
        let ratio = q.div(pr(size));
        let whole = ratio.n / ratio.d;
        let count = if up && ratio.n % ratio.d != 0 { whole + 1 } else { whole };
        let package = piece(Q::int(count), prices.0);
        let cases = [
            (Tariff::PerUnit(Price::new(prices.0)), piece(q, prices.0)),
            (Tariff::Graduated(tiers.clone()), graduated),
            (Tariff::Volume(tiers), volume),
            (Tariff::Package { size, price: Price::new(prices.0), rounding }, package),
        ];
        let s = AmountScale::new(dp, mode).unwrap();
        for (tariff, expected) in cases {
            let lines: Vec<Line> = tariff.rate(u).unwrap().into_iter().flat_map(|p| p.into_lines("u", "C62")).collect();
            let got = if lines.is_empty() {
                Decimal::ZERO
            } else {
                Document::builder(Currency::EUR, s).lines(lines).build().unwrap().line_total().to_decimal()
            };
            prop_assert_eq!(got, expected, "{:?}", tariff);
        }
    }
}

// ── Credits and the residual VAT statement (spec 029) ─────────────────────────

/// Credits at two decimals (they fit every scale `document()` draws), each with
/// a unique description so its allowances can be found.
fn credits() -> impl Strategy<Value = Vec<billing::Credit>> {
    prop::collection::vec((any::<bool>(), 1i64..=200_000), 1..5).prop_map(|cs| {
        cs.into_iter()
            .enumerate()
            .map(|(i, (before, m))| {
                let d = format!("credit {i}");
                let balance = Decimal::new(m, 2);
                if before {
                    billing::Credit::before_tax(d, balance)
                } else {
                    billing::Credit::after_tax(d, balance)
                }
            })
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn credit_draw_down_conserves_and_never_overdraws(b in document(), cs in credits()) {
        let Ok(base) = b.clone().build() else { return Ok(()) };
        let doc = cs.iter().cloned().fold(b, |b, c| b.credit(c)).build().unwrap();
        let apps = doc.credit_applications();
        prop_assert_eq!(apps.len(), cs.len());
        let (mut applied, mut balances) = (Amount::ZERO, Decimal::ZERO);
        let (mut before, mut after) = (Amount::ZERO, Amount::ZERO);
        for (i, (c, app)) in cs.iter().zip(apps).enumerate() {
            prop_assert_eq!(app.credit, i);
            prop_assert!(!app.applied.is_negative());
            prop_assert_eq!((app.applied + app.remaining).to_decimal(), c.balance);
            applied += app.applied;
            balances += c.balance;
            let n = doc.adjustments().len();
            let allowances: Amount = (n - doc.credit_adjustments()..n)
                .filter(|&k| doc.adjustments()[k].description() == c.description)
                .map(|k| doc.adjustment_amount(k))
                .sum();
            match c.placement {
                billing::CreditPlacement::BeforeTax => {
                    prop_assert_eq!(allowances, app.applied);
                    before += app.applied;
                }
                billing::CreditPlacement::AfterTax => {
                    prop_assert!(allowances.is_zero());
                    after += app.applied;
                }
            }
        }
        prop_assert!(applied.to_decimal() <= balances);
        if base.tax_exclusive_total().is_positive() {
            prop_assert!(!doc.tax_exclusive_total().is_negative());
        } else {
            prop_assert!(applied.is_zero());
        }
        prop_assert_eq!(doc.tax_exclusive_total(), base.tax_exclusive_total() - before);
        let base_prepaid = base.prepaid();
        prop_assert_eq!(doc.prepaid(), base_prepaid + after);
        if after.is_positive() {
            prop_assert!(!doc.payable().is_negative());
        }
        // VAT on the credited bases, rounded once per group: the materialised
        // document (credits as ordinary allowances and prepayment) agrees.
        let flat = Document::concat([doc.clone()]).unwrap();
        prop_assert_eq!(flat.vat_groups(), doc.vat_groups());
        prop_assert_eq!(flat.prepaid(), doc.prepaid());
        prop_assert_eq!(flat.payable(), doc.payable());
        // Rebuilding from the recovered builder is idempotent.
        prop_assert_eq!(doc.clone().into_builder().build().unwrap(), doc);
    }

    #[test]
    fn credit_residual_vat_plus_advances_is_the_breakdown(
        b in document(),
        shares in prop::collection::vec((0i64..=100_000, 0i64..=20_000), 1..4),
    ) {
        let Ok(base) = b.clone().build() else { return Ok(()) };
        let groups = base.vat_groups().to_vec();
        let advances: Vec<Advance> = groups
            .iter()
            .cycle()
            .zip(&shares)
            .map(|(g, &(net, tax))| Advance {
                vat: g.vat().clone(),
                net: Decimal::new(net, 2),
                tax: Decimal::new(tax, 2),
            })
            .collect();
        // Appended beside any gross payment: it carries no VAT.
        let doc = b.payments(advances.clone().into_iter().map(Payment::Advance)).build().unwrap();
        let r = doc.residual_vat().unwrap();
        prop_assert_eq!(doc.vat_groups(), groups.as_slice());
        prop_assert_eq!(r.len(), groups.len());
        let sum = |v: &[billing::VatGroup], f: fn(&billing::VatGroup) -> Amount| -> Amount {
            v.iter().map(f).sum()
        };
        let nets: Decimal = advances.iter().map(|a| a.net).sum();
        let taxes: Decimal = advances.iter().map(|a| a.tax).sum();
        prop_assert_eq!(
            sum(&r, billing::VatGroup::taxable_base).to_decimal() + nets,
            sum(&groups, billing::VatGroup::taxable_base).to_decimal()
        );
        prop_assert_eq!(
            sum(&r, billing::VatGroup::tax).to_decimal() + taxes,
            sum(&groups, billing::VatGroup::tax).to_decimal()
        );
    }
}

// ── Pricing models (spec 028) ───────────────────────────────────────────────

/// Every tariff shape over three tiers with flat fees (some zero), from drawn
/// bounds and prices.
fn pricing_tariffs() -> impl Strategy<Value = Vec<Tariff>> {
    (
        (1i64..40, 1i64..40),
        prop::collection::vec(dec_in(0, 5_000, 3), 3),
        prop::collection::vec(prop::option::of(dec_in(0, 500, 3)), 3),
        dec_in(1, 300, 1),
        any::<bool>(),
    )
        .prop_map(|((b0, w), prices, flats, size, up)| {
            let (b0, b1) = (Decimal::from(b0), Decimal::from(b0 + w));
            let tier = |t: Tier, i: usize| match flats[i] {
                Some(f) => t.with_flat(f),
                None => t,
            };
            let tiers = Tiers::new([
                tier(Tier::up_to(b0, Price::new(prices[0])), 0),
                tier(Tier::up_to(b1, Price::new(prices[1])), 1),
                tier(Tier::above(Price::new(prices[2])), 2),
            ])
            .unwrap();
            vec![
                Tariff::PerUnit(Price::new(prices[0])),
                Tariff::Graduated(tiers.clone()),
                Tariff::Volume(tiers),
                Tariff::Package {
                    size,
                    price: Price::new(prices[1]),
                    rounding: if up {
                        PackageRounding::Up
                    } else {
                        PackageRounding::Down
                    },
                },
            ]
        })
}

/// A non-decreasing cumulative sequence `c₁ ≤ … ≤ cₙ` of exact usages
/// `mᵢ / d` (d = 1, 2, 3 or 7: fractional averages included), billed from
/// `Billed::Nothing`; a quarter of the sequences stay at zero throughout
/// (idle periods: 030 US1).
fn cumulative() -> impl Strategy<Value = Vec<Usage>> {
    let any = (
        prop::collection::vec(0i64..=300, 1..7),
        prop::sample::select(vec![1usize, 2, 3, 7]),
    )
        .prop_map(|(mut ms, d)| {
            ms.sort_unstable();
            ms.into_iter()
                .map(|m| {
                    let mut r = vec![Sample::new(Decimal::new(m, 1))];
                    r.resize(d, Sample::new(Decimal::ZERO));
                    Aggregation::Mean.apply(r).unwrap().unwrap()
                })
                .collect::<Vec<_>>()
        });
    let idle = (1usize..6).prop_map(|n| vec![Usage::from(Decimal::ZERO); n]);
    prop_oneof![3 => any, 1 => idle]
}

fn piece_q(p: &billing::Piece) -> Q {
    usage_q(p.quantity)
        .mul(Q::of(p.unit_price.value()))
        .add(Q::of(p.flat.unwrap_or_default()))
}

fn pieces_q(ps: &[billing::Piece]) -> Q {
    ps.iter().fold(Q::int(0), |acc, p| acc.add(piece_q(p)))
}

/// Σ line nets of `lines` at `s` (0 without lines).
fn lines_total(lines: Vec<Line>, s: AmountScale) -> Decimal {
    if lines.is_empty() {
        return Decimal::ZERO;
    }
    Document::builder(Currency::EUR, s)
        .lines(lines)
        .build()
        .unwrap()
        .line_total()
        .to_decimal()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// SC-002 (030): the exact increments of every shape, billed from
    /// `Billed::Nothing`, telescope to rating the last cumulative value —
    /// without exception, all-zero sequences (the first-tier flat fee) included.
    /// Each returned state is `UpTo(c)`.
    #[test]
    fn pricing_exact_increments_telescope(tariffs in pricing_tariffs(), cs in cumulative()) {
        let last = *cs.last().unwrap();
        for t in &tariffs {
            let mut sum = Q::int(0);
            let mut billed = Billed::Nothing;
            for c in &cs {
                let (pieces, next) = t.rate_increment(billed, *c).unwrap();
                prop_assert_eq!(next, Billed::UpTo(*c));
                sum = sum.add(pieces_q(&pieces));
                billed = next;
            }
            let want = pieces_q(&t.rate(last).unwrap());
            prop_assert!(sum.eq(want), "{:?}: {:?} vs {:?}", t, sum, want);
        }
    }

    /// SC-002/SC-006 (030): the rounded increment lines of every shape, billed
    /// from `Billed::Nothing`, telescope to the lines of the whole range, at
    /// scales 0–5 and every rounding mode, all-zero sequences included.
    #[test]
    fn pricing_rounded_increments_telescope(
        tariffs in pricing_tariffs(),
        cs in cumulative(),
        dp in 0u8..=5,
        mode in prop::sample::select(MODES.to_vec()),
    ) {
        let s = AmountScale::new(dp, mode).unwrap();
        let last = *cs.last().unwrap();
        for t in &tariffs {
            let mut total = Decimal::ZERO;
            let mut billed = Billed::Nothing;
            for c in &cs {
                let (lines, next) = t.increment_lines(billed, *c, s, "u", "KWH").unwrap();
                total += lines_total(lines, s);
                billed = next;
            }
            let (lines, _) = t.increment_lines(Billed::Nothing, last, s, "u", "KWH").unwrap();
            let at_once = lines_total(lines, s);
            prop_assert_eq!(total, at_once, "{:?}", t);
            // The whole range rounds each piece of rating `last` once, at zero too.
            let leaves: Decimal = t.rate(last).unwrap().iter().map(|p| {
                let usage = usage_q(p.quantity).mul(Q::of(p.unit_price.value())).round(dp, mode);
                usage + Q::of(p.flat.unwrap_or_default()).round(dp, mode)
            }).sum();
            prop_assert_eq!(at_once, leaves, "{:?}", t);
        }
    }

    /// SC-002: keyed pricing equals pricing each key's summed usage alone, in
    /// first-appearance order.
    #[test]
    fn pricing_keyed_equals_each_key_alone(
        tariffs in pricing_tariffs(),
        usage in prop::collection::vec((0u8..4, dec_in(-500, 2_000, 1)), 0..12),
    ) {
        let map: std::collections::BTreeMap<u8, Tariff> =
            tariffs.iter().cloned().enumerate().map(|(i, t)| (i as u8, t)).collect();
        let rated = billing::rate_by_key(&map, usage.clone()).unwrap();
        let mut order: Vec<u8> = Vec::new();
        for (k, _) in &usage {
            if !order.contains(k) {
                order.push(*k);
            }
        }
        let alone: Vec<(u8, Vec<billing::Piece>)> = order
            .into_iter()
            .map(|k| {
                let sum: Decimal = usage.iter().filter(|(j, _)| *j == k).map(|(_, q)| *q).sum();
                (k, map[&k].rate(sum).unwrap())
            })
            .filter(|(_, ps)| !ps.is_empty())
            .collect();
        prop_assert_eq!(rated, alone);
    }

    /// SC-003: each charged fee's magnitude lies within the bounds, refunds
    /// mirror purchases, and the total is the exact sum after the free ones.
    #[test]
    fn pricing_fee_bounds_hold_and_refunds_mirror(
        percent in (0i64..=10_000).prop_map(|m| Decimal::new(m, 2)),
        fixed in dec_in(0, 500, 2),
        bounds in (prop::option::of(dec_in(0, 300, 2)), prop::option::of(dec_in(0, 3_000, 2))),
        free in 0u32..4,
        amounts in prop::collection::vec(dec_in(-100_000, 100_000, 2), 0..10),
    ) {
        let (min, max) = match bounds {
            (Some(a), Some(b)) => (Some(a.min(b)), Some(a.max(b))),
            other => other,
        };
        let mut fee = billing::TransactionFee::new(Percent::new(percent)).with_fixed(fixed);
        if let Some(m) = min { fee = fee.with_min(m); }
        if let Some(m) = max { fee = fee.with_max(m); }
        let mut want = Decimal::ZERO;
        let mut base = Decimal::ZERO;
        for (i, &a) in amounts.iter().enumerate() {
            let one = fee.rate([a]).unwrap().fee;
            prop_assert!(min.is_none_or(|m| one.abs() >= m), "{} below min", one);
            prop_assert!(max.is_none_or(|m| one.abs() <= m), "{} above max", one);
            // The reference: |a| × percent / 100 + fixed, clamped, signed.
            let mut mag = a.abs() * percent / dec!(100) + fixed;
            if let Some(m) = min { mag = mag.max(m); }
            if let Some(m) = max { mag = mag.min(m); }
            prop_assert_eq!(one, if a < Decimal::ZERO { -mag } else { mag });
            if !a.is_zero() {
                prop_assert_eq!(fee.rate([-a]).unwrap().fee, -one);
            }
            if i >= free as usize {
                want += one;
                base += a;
            }
        }
        let total = fee.with_free_transactions(free).rate(amounts.clone()).unwrap();
        prop_assert_eq!(total.fee, want);
        prop_assert_eq!(total.base, base);
        prop_assert_eq!(total.transactions, amounts.len() as u64);
        prop_assert_eq!(total.charged, amounts.len().saturating_sub(free as usize) as u64);
    }
}

/// A document with mixed paid entries (030): `document()`'s optional gross
/// payment, advances matched to its VAT groups, and drawn credits.
fn paid_document() -> impl Strategy<Value = Document> {
    (
        document(),
        prop::collection::vec((0i64..=100_000, 0i64..=20_000), 0..4),
        credits(),
    )
        .prop_filter_map("buildable", |(b, advances, cs)| {
            let groups = b.clone().build().ok()?.vat_groups().to_vec();
            let advances: Vec<Payment> = groups
                .iter()
                .cycle()
                .zip(&advances)
                .map(|(g, &(net, tax))| {
                    Payment::Advance(Advance {
                        vat: g.vat().clone(),
                        net: Decimal::new(net, 2),
                        tax: Decimal::new(tax, 2),
                    })
                })
                .collect();
            cs.into_iter()
                .fold(b.payments(advances), |b, c| b.credit(c))
                .build()
                .ok()
        })
}

/// An entry's amounts, in split-leaf order.
fn entry_leaves(p: &Payment) -> Vec<Decimal> {
    match p {
        Payment::Advance(a) => vec![a.net, a.tax],
        Payment::Gross { amount, .. } => vec![*amount],
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// SC-003 (030): split shares' paid entries sum exactly, entry by entry
    /// and leaf by leaf, to the parent's; each share keeps every entry's kind,
    /// VAT and description; the shares' residual VAT sums to the parent's per
    /// group.
    #[test]
    fn us030_split_partitions_every_paid_entry(doc in paid_document(), w in weights()) {
        let parent = doc.paid_entries();
        let shares = doc.split(&w).unwrap();
        for s in &shares {
            let entries = s.paid_entries();
            prop_assert_eq!(entries.len(), parent.len());
            for (e, p) in entries.iter().zip(&parent) {
                match (e, p) {
                    (Payment::Advance(x), Payment::Advance(y)) => prop_assert_eq!(&x.vat, &y.vat),
                    (Payment::Gross { description: x, .. }, Payment::Gross { description: y, .. }) => {
                        prop_assert_eq!(x, y);
                    }
                    _ => prop_assert!(false, "entry kind changed: {:?} vs {:?}", e, p),
                }
            }
            prop_assert!(s.credits().is_empty());
        }
        for (k, p) in parent.iter().enumerate() {
            let want = entry_leaves(p);
            let mut got = vec![Decimal::ZERO; want.len()];
            for s in &shares {
                for (g, l) in got.iter_mut().zip(entry_leaves(&s.paid_entries()[k])) {
                    *g += l;
                }
            }
            prop_assert_eq!(got, want, "entry {}", k);
        }
        let prepaid: Amount = shares.iter().map(Document::prepaid).sum();
        prop_assert_eq!(prepaid, doc.prepaid());
        let r = doc.residual_vat().unwrap();
        for (k, g) in r.iter().enumerate() {
            let (mut base, mut tax) = (Amount::ZERO, Amount::ZERO);
            for s in &shares {
                let sr = s.residual_vat().unwrap();
                prop_assert_eq!(sr[k].vat(), g.vat());
                base += sr[k].taxable_base();
                tax += sr[k].tax();
            }
            prop_assert_eq!((base, tax), (g.taxable_base(), g.tax()), "group {}", k);
        }
    }

    /// SC-003 (030): negating twice is the identity on the paid entries (and
    /// on the residual); concatenation keeps every entry of every document, in
    /// order.
    #[test]
    fn us030_negate_and_concat_keep_every_paid_entry(doc in paid_document()) {
        let neg = doc.negated();
        let negated: Vec<Payment> = doc.paid_entries().iter().map(|p| match p {
            Payment::Advance(a) => Payment::Advance(Advance { vat: a.vat.clone(), net: -a.net, tax: -a.tax }),
            Payment::Gross { description, amount } => Payment::Gross { description: description.clone(), amount: -*amount },
        }).collect();
        prop_assert_eq!(neg.paid_entries(), negated);
        prop_assert_eq!(neg.prepaid(), -doc.prepaid());
        prop_assert_eq!(neg.negated().paid_entries(), doc.paid_entries());
        let r = doc.residual_vat().unwrap();
        let back = neg.negated().residual_vat().unwrap();
        prop_assert_eq!(back, r.clone());
        let neg_r = neg.residual_vat().unwrap();
        for (x, y) in neg_r.iter().zip(&r) {
            prop_assert_eq!((x.taxable_base(), x.tax()), (-y.taxable_base(), -y.tax()));
        }
        let joined = Document::concat([doc.clone(), neg.clone()]).unwrap();
        let mut want = doc.paid_entries();
        want.extend(neg.paid_entries());
        prop_assert_eq!(joined.paid_entries(), want);
        prop_assert_eq!(joined.prepaid(), Amount::ZERO);
    }
}
