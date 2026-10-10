//! Pricing models (spec 028): keyed tariffs, transaction fees, progressive
//! billing (exact and rounded telescoping) and graduated percentage pricing.
//! Every figure of the spec's user stories, asserted exactly.

use std::collections::BTreeMap;

use billing::{
    Aggregation, AmountScale, Billed, Currency, Decimal, Document, ErrorKind, Line, Location,
    PackageRounding, Percent, Piece, Price, Rounding, Sample, Tariff, Tier, Tiers, TransactionFee,
    Usage, rate_by_key,
};
use rust_decimal::dec;

fn p(x: Decimal) -> Price {
    Price::new(x)
}

fn total(pieces: &[Piece]) -> Decimal {
    pieces.iter().map(|x| x.exact_net().unwrap()).sum()
}

/// A VAT-free document at `scale` over `lines`.
fn document(scale: AmountScale, lines: Vec<Line>) -> Document {
    Document::builder(Currency::EUR, scale)
        .lines(lines)
        .build()
        .unwrap()
}

fn nets(scale: AmountScale, lines: Vec<Line>) -> Vec<Decimal> {
    let n = lines.len();
    let d = document(scale, lines);
    (0..n).map(|i| d.line_net(i).to_decimal()).collect()
}

/// [up to `bound` at `low`, above at `high`].
/// The state after billing up to `x` (028's `from`): nothing billed at zero.
fn billed_at(x: impl Into<Usage>) -> Billed {
    let x = x.into();
    if x == Usage::from(Decimal::ZERO) {
        Billed::Nothing
    } else {
        Billed::UpTo(x)
    }
}

fn two_tiers(bound: Decimal, low: Decimal, high: Decimal) -> Tiers {
    Tiers::new([Tier::up_to(bound, p(low)), Tier::above(p(high))]).unwrap()
}

// ---------------------------------------------------------------- US1 ----

fn ht_nt() -> BTreeMap<&'static str, Tariff> {
    BTreeMap::from([
        (
            "HT",
            Tariff::Graduated(two_tiers(dec!(100), dec!(0.30), dec!(0.25))),
        ),
        ("NT", Tariff::PerUnit(p(dec!(0.20)))),
    ])
}

#[test]
fn us1_keyed_tariffs_price_each_key_with_its_own_tariff() {
    let rated = rate_by_key(
        &ht_nt(),
        [("HT", dec!(80)), ("NT", dec!(50)), ("HT", dec!(40))],
    )
    .unwrap();
    let keys: Vec<_> = rated.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["HT", "NT"]);
    assert_eq!(total(&rated[0].1), dec!(35.00));
    assert_eq!(rated[0].1.len(), 2); // 100 × 0.30 + 20 × 0.25
    assert_eq!(total(&rated[1].1), dec!(10.00));
}

#[test]
fn us1_unknown_key_is_refused_naming_it() {
    let err = rate_by_key(&ht_nt(), [("HT", dec!(1)), ("XX", dec!(2))]).unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::UnknownKey { key: "XX".into() });
    assert_eq!(err.location(), None);
}

#[test]
fn us1_zero_sum_key_yields_nothing() {
    let rated = rate_by_key(
        &ht_nt(),
        [("NT", dec!(5)), ("HT", dec!(10)), ("NT", dec!(-5))],
    )
    .unwrap();
    let keys: Vec<_> = rated.iter().map(|(k, _)| *k).collect();
    assert_eq!(keys, ["HT"]);
    // A key with a first-tier flat fee still yields that fee at zero.
    let flat = BTreeMap::from([(
        1u8,
        Tariff::Volume(Tiers::new([Tier::above(p(dec!(1))).with_flat(dec!(4))]).unwrap()),
    )]);
    let rated = rate_by_key(&flat, [(1, dec!(2)), (1, dec!(-2))]).unwrap();
    assert_eq!(total(&rated[0].1), dec!(4));
}

#[test]
fn us1_fractional_usage_per_key_is_priced_exactly() {
    // Two thirds plus one third: exactly 1, never 0.9999…
    let third = |n: i64| {
        Aggregation::Mean
            .apply([Decimal::from(n), dec!(0), dec!(0)].map(Sample::new))
            .unwrap()
            .unwrap()
    };
    let tariffs = BTreeMap::from([("NT", Tariff::PerUnit(p(dec!(0.20))))]);
    let rated = rate_by_key(&tariffs, [("NT", third(2)), ("NT", third(1))]).unwrap();
    assert_eq!(rated[0].1[0].quantity, Usage::from(dec!(1)));
    assert_eq!(total(&rated[0].1), dec!(0.20));
    // A sum that stays a fraction keeps its exact denominator.
    let rated = rate_by_key(&tariffs, [("NT", third(1)), ("NT", Usage::from(dec!(1)))]).unwrap();
    let q = rated[0].1[0].quantity;
    assert_eq!((q.numerator(), q.denominator()), (dec!(4), dec!(3)));
}

// ---------------------------------------------------------------- US2 ----

fn card_fee() -> TransactionFee {
    TransactionFee::new(Percent::new(dec!(2.9)))
        .with_fixed(dec!(0.30))
        .with_min(dec!(0.50))
        .with_max(dec!(5.00))
        .with_free_transactions(2)
}

#[test]
fn us2_transaction_fee_figures() {
    let t = card_fee()
        .rate([dec!(10), dec!(100), dec!(1000), dec!(5), dec!(-100)])
        .unwrap();
    assert_eq!(t.transactions, 5);
    assert_eq!(t.charged, 3);
    assert_eq!(t.base, dec!(905));
    assert_eq!(t.fee, dec!(2.30)); // 5.00 (capped) + 0.50 (raised) − 3.20 (refund)
}

#[test]
fn us2_invalid_fees_are_refused() {
    let bad = [
        TransactionFee::new(Percent::new(dec!(-0.1))),
        TransactionFee::new(Percent::new(dec!(100.01))),
        TransactionFee::new(Percent::new(dec!(1))).with_fixed(dec!(-0.01)),
        TransactionFee::new(Percent::new(dec!(1))).with_min(dec!(-1)),
        TransactionFee::new(Percent::new(dec!(1))).with_max(dec!(-1)),
        TransactionFee::new(Percent::new(dec!(1)))
            .with_min(dec!(2))
            .with_max(dec!(1)),
    ];
    for fee in bad {
        let err = fee.rate([dec!(1)]).unwrap_err();
        assert_eq!(err.kind(), &ErrorKind::InvalidFee, "{fee:?}");
        assert_eq!(err.location(), None);
    }
    // The edges are valid; min == max fixes the fee.
    assert!(
        TransactionFee::new(Percent::new(dec!(0)))
            .rate([dec!(1)])
            .is_ok()
    );
    assert!(
        TransactionFee::new(Percent::new(dec!(100)))
            .rate([dec!(1)])
            .is_ok()
    );
    let fixed = TransactionFee::new(Percent::new(dec!(50)))
        .with_min(dec!(1))
        .with_max(dec!(1))
        .rate([dec!(1000), dec!(0.01)])
        .unwrap();
    assert_eq!(fixed.fee, dec!(2));
}

#[test]
fn us2_zero_amount_is_charged_the_bounded_fixed_fee() {
    let fee = TransactionFee::new(Percent::new(dec!(2.9))).with_fixed(dec!(0.30));
    assert_eq!(fee.rate([dec!(0)]).unwrap().fee, dec!(0.30));
    let bounded = fee.clone().with_min(dec!(0.50));
    let t = bounded.rate([dec!(0)]).unwrap();
    assert_eq!((t.fee, t.charged, t.base), (dec!(0.50), 1, dec!(0)));
    // No bound means unbounded; no fixed fee, no amount: no fee.
    assert_eq!(
        TransactionFee::new(Percent::new(dec!(1)))
            .rate([dec!(0)])
            .unwrap()
            .fee,
        dec!(0)
    );
    let empty = card_fee().rate([]).unwrap();
    assert_eq!(
        (empty.transactions, empty.charged, empty.base, empty.fee),
        (0, 0, dec!(0), dec!(0))
    );
}

#[test]
fn us2_refunds_mirror_and_free_transactions_count_refunds() {
    let fee = card_fee().with_free_transactions(0);
    for a in [dec!(0.01), dec!(5), dec!(100), dec!(1000), dec!(123.456)] {
        let buy = fee.rate([a]).unwrap().fee;
        let refund = fee.rate([-a]).unwrap().fee;
        assert_eq!(refund, -buy, "{a}");
    }
    // The first transaction is free even when it is a refund.
    let t = fee
        .with_free_transactions(1)
        .rate([dec!(-100), dec!(100)])
        .unwrap();
    assert_eq!((t.charged, t.base, t.fee), (1, dec!(100), dec!(3.20)));
}

#[test]
fn us2_fee_is_exact_per_transaction() {
    // 0.1 % of 0.01 is 0.00001: kept exactly, never rounded per transaction.
    let t = TransactionFee::new(Percent::new(dec!(0.1)))
        .rate([dec!(0.01); 3])
        .unwrap();
    assert_eq!(t.fee, dec!(0.00003));
}

#[test]
fn us2_overflow_is_located_at_the_transaction() {
    let err = TransactionFee::new(Percent::new(dec!(100)))
        .with_fixed(dec!(1))
        .rate([dec!(1), Decimal::MAX])
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::Overflow);
    assert_eq!(err.location(), Some(&Location::Transaction(1)));
}

#[test]
fn us2_fee_total_into_line_rounds_the_exact_fee_once() {
    // Three fees of 0.005 each: exactly 0.015 → 0.02 (per transaction: 0.03).
    let t = TransactionFee::new(Percent::new(dec!(0.5)))
        .rate([dec!(1), dec!(1), dec!(1)])
        .unwrap();
    assert_eq!(t.fee, dec!(0.015));
    let line = t.into_line("card fees", "C62");
    assert_eq!(line.quantity().value(), dec!(3));
    assert_eq!(line.quantity().unit(), "C62");
    assert_eq!(line.price().price().value(), dec!(0.005));
    assert_eq!(line.stated_net(), Some(dec!(0.015)));
    let d = document(AmountScale::EN16931, vec![line]);
    assert_eq!(d.line_net(0).to_decimal(), dec!(0.02));
    assert!(d.price_inconsistent().is_empty());

    // The spec figures: 2.30 over 3 charged transactions.
    let t = card_fee()
        .rate([dec!(10), dec!(100), dec!(1000), dec!(5), dec!(-100)])
        .unwrap();
    let d = document(AmountScale::EN16931, vec![t.into_line("fees", "C62")]);
    assert_eq!(d.line_net(0).to_decimal(), dec!(2.30));
    assert!(d.price_inconsistent().is_empty());
}

#[test]
fn us2_net_refund_fee_line_carries_the_direction_in_the_quantity() {
    let t = card_fee()
        .with_free_transactions(0)
        .rate([dec!(-100), dec!(-100)])
        .unwrap();
    assert_eq!(t.fee, dec!(-6.40));
    let line = t.into_line("fee refunds", "C62");
    assert_eq!(line.quantity().value(), dec!(-2));
    assert_eq!(line.price().price().value(), dec!(3.2));
    let d = document(AmountScale::EN16931, vec![line]);
    assert_eq!(d.line_net(0).to_decimal(), dec!(-6.40));
    assert!(d.price_inconsistent().is_empty());
}

#[test]
fn us2_no_charged_transaction_is_a_zero_line() {
    let t = card_fee().rate([dec!(10)]).unwrap();
    assert_eq!((t.transactions, t.charged), (1, 0));
    let line = t.into_line("fees", "C62");
    assert_eq!(line.quantity().value(), dec!(0));
    assert_eq!(line.price().price().value(), dec!(0));
    assert_eq!(line.stated_net(), Some(dec!(0)));
    let d = document(AmountScale::EN16931, vec![line]);
    assert_eq!(d.line_net(0).to_decimal(), dec!(0));
}

#[cfg(feature = "serde")]
#[test]
fn us2_serde_round_trip_and_refusal() {
    use billing::FeeTotal;
    let fee = card_fee();
    let json = serde_json::to_string(&fee).unwrap();
    assert_eq!(
        json,
        r#"{"percent":"2.9","fixed":"0.30","min":"0.50","max":"5.00","free_transactions":2}"#
    );
    let back: TransactionFee = serde_json::from_str(&json).unwrap();
    assert_eq!(back, fee);
    assert_eq!(serde_json::to_string(&back).unwrap(), json);
    // Defaults: no fixed fee, no bounds, nothing free.
    let minimal: TransactionFee = serde_json::from_str(r#"{"percent":"1"}"#).unwrap();
    assert_eq!(minimal, TransactionFee::new(Percent::new(dec!(1))));

    let total = fee.rate([dec!(10), dec!(100), dec!(1000)]).unwrap();
    let json = serde_json::to_string(&total).unwrap();
    assert_eq!(
        json,
        r#"{"transactions":3,"charged":1,"base":"1000","fee":"5.00"}"#
    );
    let back: FeeTotal = serde_json::from_str(&json).unwrap();
    assert_eq!(back, total);
    assert_eq!(serde_json::to_string(&back).unwrap(), json);

    for bad in [
        r#"{"percent":"101"}"#,
        r#"{"percent":"-1"}"#,
        r#"{"percent":"1","fixed":"-1"}"#,
        r#"{"percent":"1","min":"2","max":"1"}"#,
        r#"{"percent":1}"#,
    ] {
        assert!(
            serde_json::from_str::<TransactionFee>(bad).is_err(),
            "{bad}"
        );
    }
}

// ---------------------------------------------------------------- US3 ----

fn annual() -> Tiers {
    two_tiers(dec!(1000), dec!(0.30), dec!(0.25))
}

#[test]
fn us3_graduated_increments_telescope() {
    let t = Tariff::Graduated(annual());
    let readings = [dec!(0), dec!(400), dec!(900), dec!(1300)];
    let increments: Vec<Decimal> = readings
        .windows(2)
        .map(|w| total(&t.rate_increment(billed_at(w[0]), w[1]).unwrap().0))
        .collect();
    assert_eq!(increments, [dec!(120.00), dec!(150.00), dec!(105.00)]);
    assert_eq!(increments.iter().sum::<Decimal>(), dec!(375.00));
    assert_eq!(total(&t.rate(dec!(1300)).unwrap()), dec!(375.00));
    // 900 → 1 300 crosses the bound: 100 in tier 0, 300 in tier 1.
    let pieces = t
        .rate_increment(billed_at(dec!(900)), dec!(1300))
        .unwrap()
        .0;
    let split: Vec<_> = pieces
        .iter()
        .map(|x| (x.tier, x.quantity.to_decimal()))
        .collect();
    assert_eq!(split, [(0, dec!(100)), (1, dec!(300))]);
}

#[test]
fn us3_volume_cliff_increment_is_negative() {
    let t = Tariff::Volume(annual());
    let pieces = t
        .rate_increment(billed_at(dec!(990)), dec!(1010))
        .unwrap()
        .0;
    assert_eq!(total(&pieces), dec!(-44.50)); // 252.50 − 297.00
    let split: Vec<_> = pieces
        .iter()
        .map(|x| (x.tier, x.quantity.to_decimal()))
        .collect();
    assert_eq!(split, [(0, dec!(-990)), (1, dec!(1010))]);
    // Within one tier it is the difference.
    let same = t
        .rate_increment(billed_at(dec!(1010)), dec!(1100))
        .unwrap()
        .0;
    assert_eq!(total(&same), dec!(22.50));
    assert_eq!(same.len(), 1);
}

#[test]
fn us3_package_increment_adds_packages() {
    let t = Tariff::Package {
        size: dec!(100),
        price: p(dec!(5.00)),
        rounding: PackageRounding::Up,
    };
    let pieces = t.rate_increment(billed_at(dec!(250)), dec!(410)).unwrap().0;
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].quantity, Usage::from(dec!(2)));
    assert_eq!(pieces[0].measured, Some(dec!(160)));
    assert_eq!(total(&pieces), dec!(10.00));
    // No package added: nothing.
    assert!(
        t.rate_increment(billed_at(dec!(210)), dec!(250))
            .unwrap()
            .0
            .is_empty()
    );
}

#[test]
fn us3_per_unit_increment_is_the_difference() {
    let t = Tariff::PerUnit(p(dec!(0.30)));
    let pieces = t.rate_increment(billed_at(dec!(400)), dec!(900)).unwrap().0;
    assert_eq!(pieces[0].quantity, Usage::from(dec!(500)));
    assert_eq!(total(&pieces), dec!(150));
}

#[test]
fn us3_flat_fees_are_billed_when_reached() {
    let tiers = Tiers::new([
        Tier::up_to(dec!(100), p(dec!(1))).with_flat(dec!(10)),
        Tier::above(p(dec!(0.5))).with_flat(dec!(3)),
    ])
    .unwrap();
    let g = Tariff::Graduated(tiers);
    // The first-tier flat fee comes with the first increment from zero.
    assert_eq!(
        total(&g.rate_increment(Billed::Nothing, dec!(50)).unwrap().0),
        dec!(60)
    );
    // Tier 1's flat fee comes with the increment that reaches it, once.
    assert_eq!(
        total(&g.rate_increment(billed_at(dec!(50)), dec!(150)).unwrap().0),
        dec!(78)
    );
    assert_eq!(
        total(&g.rate_increment(billed_at(dec!(150)), dec!(160)).unwrap().0),
        dec!(5)
    );
    assert_eq!(total(&g.rate(dec!(160)).unwrap()), dec!(143));
}

#[test]
fn us3_invalid_ranges_are_refused() {
    let t = Tariff::Graduated(annual());
    let err = t.rate_increment(billed_at(dec!(10)), dec!(5)).unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::InvalidRange {
            from: dec!(10),
            to: dec!(5)
        }
    );
    assert_eq!(err.location(), None);
    let err = t.rate_increment(billed_at(dec!(-1)), dec!(5)).unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::InvalidRange {
            from: dec!(-1),
            to: dec!(5)
        }
    );
    let err = t
        .increment_lines(
            billed_at(dec!(3)),
            dec!(2),
            AmountScale::EN16931,
            "x",
            "KWH",
        )
        .unwrap_err();
    assert!(matches!(err.kind(), ErrorKind::InvalidRange { .. }));
    // from == to: nothing.
    assert!(
        t.rate_increment(billed_at(dec!(5)), dec!(5))
            .unwrap()
            .0
            .is_empty()
    );
    assert!(
        t.increment_lines(
            billed_at(dec!(5)),
            dec!(5),
            AmountScale::EN16931,
            "x",
            "KWH"
        )
        .unwrap()
        .0
        .is_empty()
    );
}

#[test]
fn us3_fractional_cumulative_usage_is_rated_exactly() {
    let mean = |vals: [i64; 3]| {
        Aggregation::Mean
            .apply(vals.map(|v| Sample::new(Decimal::from(v))))
            .unwrap()
            .unwrap()
    };
    let t = Tariff::Graduated(two_tiers(dec!(1), dec!(0.015), dec!(0.01)));
    let (a, b) = (mean([1, 0, 0]), mean([4, 0, 0])); // 1/3, 4/3
    let pieces = t.rate_increment(billed_at(a), b).unwrap().0;
    let q: Vec<_> = pieces
        .iter()
        .map(|x| (x.quantity.numerator(), x.quantity.denominator()))
        .collect();
    assert_eq!(q, [(dec!(2), dec!(3)), (dec!(1), dec!(3))]);
}

#[test]
fn us3_increment_lines_bill_the_figures() {
    let t = Tariff::Graduated(annual());
    let s = AmountScale::EN16931;
    let mut billed = Decimal::ZERO;
    for (from, to, want) in [
        (dec!(0), dec!(400), dec!(120.00)),
        (dec!(400), dec!(900), dec!(150.00)),
        (dec!(900), dec!(1300), dec!(105.00)),
    ] {
        let lines = t
            .increment_lines(billed_at(from), to, s, "energy", "KWH")
            .unwrap()
            .0;
        let got: Decimal = nets(s, lines.clone()).iter().sum();
        assert_eq!(got, want);
        assert!(document(s, lines).price_inconsistent().is_empty());
        billed += got;
    }
    assert_eq!(billed, dec!(375.00));
    let lines = Tariff::Volume(annual())
        .increment_lines(billed_at(dec!(990)), dec!(1010), s, "energy", "KWH")
        .unwrap()
        .0;
    assert_eq!(lines[0].quantity().value(), dec!(-990));
    assert_eq!(lines[1].quantity().value(), dec!(1010));
    assert_eq!(nets(s, lines).iter().sum::<Decimal>(), dec!(-44.50));
}

#[test]
fn telescoping_rounded_increments_add_up_to_the_rounded_total() {
    // 0.4, 0.8, 1.2 kWh at 0.015: each exact increment is 0.006.
    let t = Tariff::PerUnit(p(dec!(0.015)));
    let s = AmountScale::EN16931;
    let readings = [dec!(0), dec!(0.4), dec!(0.8), dec!(1.2)];
    let mut billed = Vec::new();
    for w in readings.windows(2) {
        let lines = t
            .increment_lines(billed_at(w[0]), w[1], s, "energy", "KWH")
            .unwrap()
            .0;
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].quantity().value(), dec!(0.4));
        billed.push(nets(s, lines)[0]);
    }
    assert_eq!(billed, [dec!(0.01), dec!(0.00), dec!(0.01)]);
    assert_eq!(billed.iter().sum::<Decimal>(), dec!(0.02));
    let at_once = t
        .increment_lines(Billed::Nothing, dec!(1.2), s, "energy", "KWH")
        .unwrap()
        .0;
    assert_eq!(nets(s, at_once), [dec!(0.02)]);
    // Rounding each exact increment on its own would bill 0.03.
    let naive: Decimal = readings
        .windows(2)
        .map(|w| {
            let pieces = t.rate_increment(billed_at(w[0]), w[1]).unwrap().0;
            let lines = pieces
                .into_iter()
                .flat_map(|x| x.into_lines("energy", "KWH"))
                .collect();
            nets(s, lines).iter().sum::<Decimal>()
        })
        .sum();
    assert_eq!(naive, dec!(0.03));
}

#[test]
fn telescoping_flat_fees_are_their_own_lines() {
    let tiers = Tiers::new([
        Tier::up_to(dec!(1), p(dec!(0.015))).with_flat(dec!(0.004)),
        Tier::above(p(dec!(0.01))).with_flat(dec!(2)),
    ])
    .unwrap();
    let t = Tariff::Graduated(tiers);
    let s = AmountScale::EN16931;
    let lines = t
        .increment_lines(billed_at(dec!(0.5)), dec!(3), s, "e", "KWH")
        .unwrap()
        .0;
    // Tier 0: 0.5 more units (0.015 − 0.0075 → 0.02 − 0.01); tier 1: 2 units
    // and its flat fee, newly reached, as a separate fixed line.
    let shape: Vec<_> = lines
        .iter()
        .map(|l| (l.quantity().value(), l.quantity().unit().to_string()))
        .collect();
    assert_eq!(
        shape,
        [
            (dec!(0.5), "KWH".to_string()),
            (dec!(2), "KWH".to_string()),
            (dec!(1), "C62".to_string())
        ]
    );
    assert_eq!(nets(s, lines.clone()), [dec!(0.01), dec!(0.02), dec!(2)]);
    assert!(document(s, lines).price_inconsistent().is_empty());
    // A flat fee below the scale's unit telescopes like any leaf.
    let first = t
        .increment_lines(Billed::Nothing, dec!(0.5), s, "e", "KWH")
        .unwrap()
        .0;
    assert_eq!(nets(s, first), [dec!(0.01)]); // 0.0075 → 0.01; flat 0.004 → 0.00
    let s4 = AmountScale::new(4, Rounding::HalfEven).unwrap();
    let first = t
        .increment_lines(Billed::Nothing, dec!(0.5), s4, "e", "KWH")
        .unwrap()
        .0;
    assert_eq!(nets(s4, first), [dec!(0.0075), dec!(0.004)]);
}

// ---------------------------------------------------------------- US4 ----

#[test]
fn us4_graduated_percentage() {
    // 2.9 % on the first 10 000 of volume, 1.9 % above: rates as prices.
    let t = Tariff::Graduated(two_tiers(dec!(10000), dec!(0.029), dec!(0.019)));
    let pieces = t.rate(dec!(15000)).unwrap();
    assert_eq!(
        pieces
            .iter()
            .map(|x| x.exact_net().unwrap())
            .collect::<Vec<_>>(),
        [dec!(290), dec!(95)]
    );
    assert_eq!(total(&pieces), dec!(385.00));
}

#[test]
fn us3_package_increment_of_fractions_measures_the_exact_difference() {
    // 2/15 → 49/6: decimal readings whose difference needs 29 digits.
    let mean = |v: Decimal| {
        Aggregation::Mean
            .apply([Sample::new(v), Sample::new(dec!(0)), Sample::new(dec!(0))])
            .unwrap()
            .unwrap()
    };
    let t = Tariff::Package {
        size: dec!(1),
        price: p(dec!(2)),
        rounding: PackageRounding::Down,
    };
    let pieces = t
        .rate_increment(billed_at(mean(dec!(0.4))), mean(dec!(24.5)))
        .unwrap()
        .0;
    assert_eq!(pieces[0].quantity, Usage::from(dec!(8)));
    assert_eq!(
        pieces[0].measured,
        Some(dec!(241) / dec!(30)) // 49/6 − 2/15
    );
}

#[test]
fn us3_unchanged_tiers_make_no_piece_and_no_line() {
    let t = Tariff::Graduated(annual());
    // Tier 0 is full at both 1 100 and 1 200: only tier 1 moves.
    let pieces = t
        .rate_increment(billed_at(dec!(1100)), dec!(1200))
        .unwrap()
        .0;
    let tiers: Vec<_> = pieces.iter().map(|x| x.tier).collect();
    assert_eq!(tiers, [1]);
    let lines = t
        .increment_lines(
            billed_at(dec!(1100)),
            dec!(1200),
            AmountScale::EN16931,
            "e",
            "KWH",
        )
        .unwrap()
        .0;
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].quantity().value(), dec!(100));
    assert_eq!(lines[0].stated_net(), Some(dec!(25.00)));
    // Volume: from tier 0 to tier 1 skips nothing; tier 2 of three is reached
    // from tier 0 with tier 1 left untouched.
    let three = Tiers::new([
        Tier::up_to(dec!(10), p(dec!(3))),
        Tier::up_to(dec!(20), p(dec!(2))),
        Tier::above(p(dec!(1))),
    ])
    .unwrap();
    let v = Tariff::Volume(three);
    let pieces = v.rate_increment(billed_at(dec!(5)), dec!(25)).unwrap().0;
    let split: Vec<_> = pieces
        .iter()
        .map(|x| (x.tier, x.quantity.to_decimal()))
        .collect();
    assert_eq!(split, [(0, dec!(-5)), (2, dec!(25))]);
    let lines = v
        .increment_lines(
            billed_at(dec!(5)),
            dec!(25),
            AmountScale::EN16931,
            "e",
            "KWH",
        )
        .unwrap()
        .0;
    let nets: Vec<_> = lines.iter().map(|l| l.stated_net().unwrap()).collect();
    assert_eq!(nets, [dec!(-15.00), dec!(25.00)]);
}

// ------------------------------------------------------- 030 US1 ----

fn up_to(x: Decimal) -> Billed {
    Billed::UpTo(Usage::from(x))
}

/// Tier 1 [up to 100 at 0.30, flat 10.00], then 0.25 above.
fn base_fee() -> Tariff {
    Tariff::Graduated(
        Tiers::new([
            Tier::up_to(dec!(100), p(dec!(0.30))).with_flat(dec!(10.00)),
            Tier::above(p(dec!(0.25))),
        ])
        .unwrap(),
    )
}

/// Months: nothing → 0, 0 → 0, 0 → 50: 10.00, 0.00, 15.00 = rating 50.
#[test]
fn us030_billed_idle_months_charge_the_base_fee_once() {
    let t = base_fee();
    let mut billed = Billed::Nothing;
    let mut months = Vec::new();
    for to in [dec!(0), dec!(0), dec!(50)] {
        let (pieces, next) = t.rate_increment(billed, to).unwrap();
        assert_eq!(next, up_to(to), "the returned state is UpTo(to)");
        months.push(total(&pieces));
        billed = next;
    }
    assert_eq!(months, [dec!(10.00), dec!(0.00), dec!(15.00)]);
    assert_eq!(months.iter().sum::<Decimal>(), dec!(25.00));
    assert_eq!(total(&t.rate(dec!(50)).unwrap()), dec!(25.00));
}

/// A sequence that stays at zero totals the zero-usage rating: 10.00.
#[test]
fn us030_billed_all_zero_sequence_totals_the_zero_rating() {
    let t = base_fee();
    let s = AmountScale::EN16931;
    let mut billed = Billed::Nothing;
    let (mut exact, mut rounded) = (Decimal::ZERO, Decimal::ZERO);
    for _ in 0..4 {
        let (pieces, _) = t.rate_increment(billed, dec!(0)).unwrap();
        let (lines, next) = t
            .increment_lines(billed, dec!(0), s, "base", "KWH")
            .unwrap();
        exact += total(&pieces);
        rounded += document(s, lines).line_total().to_decimal();
        billed = next;
    }
    assert_eq!(exact, dec!(10.00));
    assert_eq!(rounded, dec!(10.00));
    assert_eq!(total(&t.rate(dec!(0)).unwrap()), dec!(10.00));
}

#[test]
fn us030_billed_increment_lines_match_the_pieces() {
    let t = base_fee();
    let s = AmountScale::EN16931;
    let (lines, next) = t
        .increment_lines(Billed::Nothing, dec!(0), s, "base", "KWH")
        .unwrap();
    assert_eq!(next, up_to(dec!(0)));
    assert_eq!(nets(s, lines), [dec!(10.00)]);
    let (lines, next) = t.increment_lines(next, dec!(0), s, "base", "KWH").unwrap();
    assert!(lines.is_empty());
    let (lines, _) = t.increment_lines(next, dec!(50), s, "base", "KWH").unwrap();
    assert_eq!(nets(s, lines), [dec!(15.00)]);
}

/// A lower cumulative value than what was billed is refused.
#[test]
fn us030_billed_lower_target_is_refused() {
    let t = base_fee();
    let err = t.rate_increment(up_to(dec!(50)), dec!(40)).unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::InvalidRange {
            from: dec!(50),
            to: dec!(40)
        }
    );
    let err = t
        .increment_lines(up_to(dec!(50)), dec!(40), AmountScale::EN16931, "x", "KWH")
        .unwrap_err();
    assert!(matches!(err.kind(), ErrorKind::InvalidRange { .. }));
    let err = t.rate_increment(Billed::Nothing, dec!(-1)).unwrap_err();
    assert_eq!(
        err.kind(),
        &ErrorKind::InvalidRange {
            from: dec!(0),
            to: dec!(-1)
        }
    );
    let err = t.rate_increment(up_to(dec!(-1)), dec!(5)).unwrap_err();
    assert!(matches!(err.kind(), ErrorKind::InvalidRange { .. }));
}

#[cfg(feature = "serde")]
#[test]
fn us030_billed_state_serialises() {
    for b in [Billed::Nothing, up_to(dec!(50))] {
        let json = serde_json::to_string(&b).unwrap();
        assert_eq!(serde_json::from_str::<Billed>(&json).unwrap(), b);
    }
    assert_eq!(
        serde_json::to_string(&Billed::Nothing).unwrap(),
        r#""nothing""#
    );
    assert_eq!(
        serde_json::to_string(&up_to(dec!(50))).unwrap(),
        r#"{"up_to":{"numerator":"50","denominator":"1"}}"#
    );
}

/// From `UpTo(x)` to `x` nothing changed: nothing is rated at all, so a
/// tariff that could not rate `x` is not consulted (kills the `f == to`
/// shortcut's mutant).
#[test]
fn us030_billed_unchanged_target_rates_nothing() {
    let closed = Tariff::Graduated(Tiers::new([Tier::up_to(dec!(100), p(dec!(1)))]).unwrap());
    assert!(matches!(
        closed.rate(dec!(150)).unwrap_err().kind(),
        ErrorKind::Uncovered { .. }
    ));
    let (pieces, next) = closed.rate_increment(up_to(dec!(150)), dec!(150)).unwrap();
    assert!(pieces.is_empty());
    assert_eq!(next, up_to(dec!(150)));
    let (lines, _) = closed
        .increment_lines(
            up_to(dec!(150)),
            dec!(150),
            AmountScale::EN16931,
            "x",
            "KWH",
        )
        .unwrap();
    assert!(lines.is_empty());
}

#[test]
fn a_zero_fee_line_keeps_a_positive_quantity() {
    // Charged transactions with a 0 % fee and no fixed part: the fee is zero, and
    // the line must read +charged at a zero price — not a reversed (negative)
    // quantity, which a zero fee gives no reason for.
    let fee = TransactionFee::new(Percent::new(dec!(0)))
        .rate([dec!(10), dec!(20), dec!(30)])
        .unwrap();
    assert_eq!((fee.charged, fee.fee), (3, dec!(0)));
    let line = fee.into_line("fees", "C62");
    assert_eq!(line.quantity().value(), dec!(3));
}
