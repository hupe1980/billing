//! Tariffs: Stripe-parity boundaries, flat fees, capacity, packages, series
//! exactness, keyed rates, percentiles, serde, and properties.

use std::collections::BTreeMap;

use billing::{
    ErrorKind, Location, PackageRounding, Percentile, Piece, Price, Tariff, Tier, TierError, Tiers,
    nearest_rank, rate_by_key, rate_series,
};
use proptest::prelude::*;
use rust_decimal::{Decimal, dec};

fn p(x: Decimal) -> Price {
    Price::new(x)
}

fn total(pieces: &[Piece]) -> Decimal {
    pieces.iter().map(|x| x.exact_net().unwrap()).sum()
}

/// 0–100 @ 0.30 + 10 flat, –500 @ 0.25 + 5 flat, above @ 0.20.
fn ladder() -> Tiers {
    Tiers::new([
        Tier::up_to(dec!(100), p(dec!(0.30))).with_flat(dec!(10)),
        Tier::up_to(dec!(500), p(dec!(0.25))).with_flat(dec!(5)),
        Tier::above(p(dec!(0.20))),
    ])
    .unwrap()
}

#[test]
fn stripe_parity_boundaries() {
    // (quantity, graduated total, volume total): 0, exactly at up_to, just above.
    let table = [
        (dec!(0), dec!(10), dec!(10)),
        (dec!(100), dec!(40), dec!(40)),
        (dec!(100.01), dec!(45.0025), dec!(30.0025)),
        (dec!(500), dec!(145), dec!(130)),
        (dec!(500.5), dec!(145.1), dec!(100.1)),
    ];
    for (q, graduated, volume) in table {
        assert_eq!(
            total(&Tariff::Graduated(ladder()).rate(q).unwrap()),
            graduated,
            "graduated {q}"
        );
        assert_eq!(
            total(&Tariff::Volume(ladder()).rate(q).unwrap()),
            volume,
            "volume {q}"
        );
    }
}

#[test]
fn graduated_pieces_at_a_bound_do_not_reach_the_next_tier() {
    let at = Tariff::Graduated(ladder()).rate(dec!(100)).unwrap();
    assert_eq!(at.len(), 1);
    assert_eq!(
        (at[0].tier, at[0].quantity.to_decimal(), at[0].flat),
        (0, dec!(100), Some(dec!(10)))
    );

    let above = Tariff::Graduated(ladder()).rate(dec!(100.01)).unwrap();
    assert_eq!(above.len(), 2);
    assert_eq!(
        (above[1].tier, above[1].quantity.to_decimal(), above[1].flat),
        (1, dec!(0.01), Some(dec!(5)))
    );
    assert_eq!(above[1].unit_price, p(dec!(0.25)));
}

#[test]
fn zero_usage_reaches_only_the_first_tier_flat() {
    let pieces = Tariff::Graduated(ladder()).rate(dec!(0)).unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(
        (
            pieces[0].tier,
            pieces[0].quantity.to_decimal(),
            pieces[0].flat
        ),
        (0, dec!(0), Some(dec!(10)))
    );

    // Without a first-tier flat fee, zero usage rates to nothing.
    let plain = Tiers::new([Tier::up_to(dec!(10), p(dec!(1))), Tier::above(p(dec!(0.5)))]).unwrap();
    assert!(
        Tariff::Graduated(plain.clone())
            .rate(dec!(0))
            .unwrap()
            .is_empty()
    );
    assert!(Tariff::Volume(plain).rate(dec!(0)).unwrap().is_empty());
    assert!(
        Tariff::PerUnit(p(dec!(1)))
            .rate(dec!(0))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn no_zero_quantity_slivers_in_graduated() {
    let pieces = Tariff::Graduated(ladder()).rate(dec!(500)).unwrap();
    assert_eq!(pieces.iter().map(|x| x.tier).collect::<Vec<_>>(), [0, 1]);
    assert!(pieces.iter().all(|x| !x.quantity.to_decimal().is_zero()));
}

#[test]
fn volume_cliff_is_documented_behaviour() {
    let v = |q| total(&Tariff::Volume(ladder()).rate(q).unwrap());
    assert!(v(dec!(100)) > v(dec!(100.01)));
    // Graduated never decreases across the same bound.
    let g = |q| total(&Tariff::Graduated(ladder()).rate(q).unwrap());
    assert!(g(dec!(100)) < g(dec!(100.01)));
}

#[test]
fn capacity_is_volume_with_flat_only_tiers() {
    let capacity = Tariff::Volume(
        Tiers::new([
            Tier::up_to(dec!(10), p(dec!(0))).with_flat(dec!(50)),
            Tier::up_to(dec!(100), p(dec!(0))).with_flat(dec!(200)),
            Tier::above(p(dec!(0))).with_flat(dec!(1000)),
        ])
        .unwrap(),
    );
    let c = |q| total(&capacity.rate(q).unwrap());
    assert_eq!(c(dec!(0)), dec!(50));
    assert_eq!(c(dec!(10)), dec!(50));
    assert_eq!(c(dec!(10.5)), dec!(200));
    assert_eq!(c(dec!(100)), dec!(200));
    assert_eq!(c(dec!(1e6)), dec!(1000));
    let at_peak = capacity.rate(dec!(42)).unwrap();
    assert_eq!(
        (at_peak[0].tier, at_peak[0].quantity.to_decimal()),
        (1, dec!(42))
    );
}

#[test]
fn packages_round_up_or_down_and_keep_the_measured_quantity() {
    let pkg = |rounding| Tariff::Package {
        size: dec!(1000),
        price: p(dec!(2.50)),
        rounding,
    };
    let up = pkg(PackageRounding::Up).rate(dec!(2001)).unwrap();
    assert_eq!(
        (up[0].quantity.to_decimal(), up[0].measured),
        (dec!(3), Some(dec!(2001)))
    );
    assert_eq!(total(&up), dec!(7.50));
    let down = pkg(PackageRounding::Down).rate(dec!(2001)).unwrap();
    assert_eq!(
        (down[0].quantity.to_decimal(), down[0].measured),
        (dec!(2), Some(dec!(2001)))
    );
    assert_eq!(
        pkg(PackageRounding::Up).rate(dec!(2000)).unwrap()[0]
            .quantity
            .to_decimal(),
        dec!(2)
    );
    assert!(
        pkg(PackageRounding::Down)
            .rate(dec!(999))
            .unwrap()
            .is_empty()
    );
    assert!(pkg(PackageRounding::Up).rate(dec!(0)).unwrap().is_empty());
    assert_eq!(
        pkg(PackageRounding::Up).rate(dec!(0.001)).unwrap()[0]
            .quantity
            .to_decimal(),
        dec!(1)
    );

    let bad = Tariff::Package {
        size: dec!(0),
        price: p(dec!(1)),
        rounding: PackageRounding::Up,
    };
    assert_eq!(
        bad.rate(dec!(1)).unwrap_err().kind(),
        &ErrorKind::InvalidTiers(TierError::PackageSize)
    );
}

#[test]
fn negative_quantity_is_the_exact_negation() {
    for tariff in [
        Tariff::Graduated(ladder()),
        Tariff::Volume(ladder()),
        Tariff::PerUnit(p(dec!(0.3))),
        Tariff::Package {
            size: dec!(7),
            price: p(dec!(1)),
            rounding: PackageRounding::Up,
        },
    ] {
        let pos = tariff.rate(dec!(150.5)).unwrap();
        let neg = tariff.rate(dec!(-150.5)).unwrap();
        assert_eq!(pos.len(), neg.len());
        for (a, b) in pos.iter().zip(&neg) {
            assert_eq!(b.quantity.to_decimal(), -a.quantity.to_decimal());
            assert_eq!(b.flat, a.flat.map(|f| -f));
            assert_eq!(b.measured, a.measured.map(|m| -m));
            assert_eq!((b.tier, b.unit_price), (a.tier, a.unit_price));
        }
        assert_eq!(total(&neg), -total(&pos));
    }
}

#[test]
fn tiers_validation_names_the_tier() {
    let one = p(dec!(1));
    let err = |t: Vec<Tier>| Tiers::new(t).unwrap_err();

    let e = err(vec![]);
    assert_eq!(
        (e.kind(), e.location()),
        (&ErrorKind::InvalidTiers(TierError::Empty), None)
    );

    let e = err(vec![Tier::up_to(dec!(10), one), Tier::up_to(dec!(10), one)]);
    assert_eq!(e.kind(), &ErrorKind::InvalidTiers(TierError::NotIncreasing));
    assert_eq!(e.location(), Some(&Location::Tier(1)));

    let e = err(vec![Tier::above(one), Tier::up_to(dec!(10), one)]);
    assert_eq!(e.kind(), &ErrorKind::InvalidTiers(TierError::OpenNotLast));
    assert_eq!(e.location(), Some(&Location::Tier(0)));

    let e = err(vec![Tier::up_to(dec!(0), one)]);
    assert_eq!(
        e.kind(),
        &ErrorKind::InvalidTiers(TierError::NonPositiveBound)
    );

    let e = err(vec![
        Tier::up_to(dec!(1), one),
        Tier::above(one).with_flat(dec!(-1)),
    ]);
    assert_eq!(e.kind(), &ErrorKind::InvalidTiers(TierError::NegativeFlat));
    assert_eq!(e.location(), Some(&Location::Tier(1)));

    let e = err(vec![Tier::up_to(dec!(1), one), Tier::above(p(dec!(-0.01)))]);
    assert_eq!(e.kind(), &ErrorKind::NegativePrice { price: dec!(-0.01) });
    assert_eq!(e.rule(), Some("BR-27"));
    assert_eq!(e.to_string(), "tier 1: price -0.01 is negative (BR-27)");

    let e = err(vec![Tier::up_to(dec!(5), one), Tier::up_to(dec!(4), one)]);
    assert_eq!(
        e.to_string(),
        "tier 1: invalid tiers: up_to must be strictly increasing"
    );
}

#[test]
fn closed_tiers_refuse_uncovered_quantities() {
    let closed = Tiers::new([Tier::up_to(dec!(10), p(dec!(1)))]).unwrap();
    assert_eq!(closed.at(dec!(10)).unwrap().limit(), Some(dec!(10)));
    for q in [dec!(10.000001), dec!(-1)] {
        assert_eq!(
            closed.at(q).unwrap_err().kind(),
            &ErrorKind::Uncovered { value: q }
        );
    }
    let uncovered = ErrorKind::Uncovered { value: dec!(11) };
    assert_eq!(
        Tariff::Graduated(closed.clone())
            .rate(dec!(11))
            .unwrap_err()
            .kind(),
        &uncovered
    );
    assert_eq!(
        Tariff::Volume(closed.clone())
            .rate(dec!(11))
            .unwrap_err()
            .kind(),
        &uncovered
    );
    assert_eq!(
        Tariff::Graduated(closed)
            .rate(dec!(-11))
            .unwrap_err()
            .kind(),
        &uncovered
    );
}

#[test]
fn negative_prices_are_refused_by_every_tariff() {
    let neg = p(dec!(-1));
    assert!(matches!(
        Tariff::PerUnit(neg).rate(dec!(1)).unwrap_err().kind(),
        ErrorKind::NegativePrice { .. }
    ));
    let pkg = Tariff::Package {
        size: dec!(1),
        price: neg,
        rounding: PackageRounding::Down,
    };
    assert!(matches!(
        pkg.rate(dec!(1)).unwrap_err().kind(),
        ErrorKind::NegativePrice { .. }
    ));
}

/// (0.5 @ 0.00999) + (2.28571 @ 0) is exactly 0.004995. The series keeps
/// the exact net so the document can reduce it once (0.00 at EN 16931, not the
/// double-rounded 0.01).
#[test]
fn series_net_is_exact() {
    let s = rate_series([(dec!(0.5), p(dec!(0.00999))), (dec!(2.28571), p(dec!(0)))]).unwrap();
    assert_eq!(s.net, dec!(0.004995));
    assert_eq!(s.quantity, dec!(2.78571));
    assert_eq!(s.average_price.value(), dec!(0.004995) / dec!(2.78571));
}

#[test]
fn series_edge_cases() {
    let empty = rate_series([]).unwrap();
    assert_eq!(
        (empty.quantity, empty.net, empty.average_price),
        (dec!(0), dec!(0), p(dec!(0)))
    );

    // Zero-quantity intervals are skipped whatever their price.
    let s = rate_series([(dec!(0), p(dec!(-99))), (dec!(2), p(dec!(0.5)))]).unwrap();
    assert_eq!(
        (s.quantity, s.net, s.average_price),
        (dec!(2), dec!(1), p(dec!(0.5)))
    );

    // Negative spot prices are allowed while the average stays ≥ 0.
    let s = rate_series([(dec!(1), p(dec!(-0.1))), (dec!(1), p(dec!(0.3)))]).unwrap();
    assert_eq!((s.net, s.average_price), (dec!(0.2), p(dec!(0.1))));

    // A negative average stays a non-negative BT-146 (BR-27); the direction moves
    // to the line's quantity and the exact net keeps its sign.
    let s = rate_series([(dec!(1), p(dec!(-0.1)))]).unwrap();
    assert_eq!((s.net, s.average_price), (dec!(-0.1), p(dec!(0.1))));
    assert!(s.is_negative_average());
    assert!(
        !rate_series([(dec!(1), p(dec!(0.1)))])
            .unwrap()
            .is_negative_average()
    );
    assert!(
        !rate_series([(dec!(-2), p(dec!(0.25)))])
            .unwrap()
            .is_negative_average()
    );

    // Σq = 0 with a non-zero net has no average price.
    let e = rate_series([(dec!(1), p(dec!(1))), (dec!(-1), p(dec!(0)))]).unwrap_err();
    assert_eq!(e.kind(), &ErrorKind::DivisionByZero);

    // A credit series: negative quantities, non-negative average.
    let s = rate_series([(dec!(-2), p(dec!(0.25)))]).unwrap();
    assert_eq!(
        (s.quantity, s.net, s.average_price),
        (dec!(-2), dec!(-0.5), p(dec!(0.25)))
    );
}

/// A zero price is not negative (BR-27 is `PriceAmount >= 0`): every tariff
/// that checks its price accepts it and rates to zero.
#[test]
fn zero_price_is_accepted_by_every_tariff() {
    let zero = p(dec!(0));
    let per_unit = Tariff::PerUnit(zero).rate(dec!(7)).unwrap();
    assert_eq!(total(&per_unit), dec!(0));
    let pkg = Tariff::Package {
        size: dec!(10),
        price: zero,
        rounding: PackageRounding::Up,
    };
    let pieces = pkg.rate(dec!(25)).unwrap();
    assert_eq!(pieces[0].quantity.to_decimal(), dec!(3));
    assert_eq!(total(&pieces), dec!(0));
    let tariffs = BTreeMap::from([(1u8, Tariff::PerUnit(zero))]);
    let rated = rate_by_key(&tariffs, [(1, dec!(4))]).unwrap();
    assert_eq!(rated.len(), 1);
    assert_eq!(rated[0].1[0].quantity.to_decimal(), dec!(4));
    assert_eq!(total(&rated[0].1), dec!(0));
}

#[test]
fn rate_by_key_aggregates_in_first_appearance_order() {
    let tariffs = BTreeMap::from([
        ("HT".to_string(), Tariff::PerUnit(p(dec!(0.32)))),
        ("NT".to_string(), Tariff::PerUnit(p(dec!(0.24)))),
        ("XT".to_string(), Tariff::PerUnit(p(dec!(0.1)))),
    ]);
    let usage = [
        ("NT", dec!(40)),
        ("HT", dec!(100)),
        ("NT", dec!(2)),
        ("XT", dec!(1)),
        ("XT", dec!(-1)),
    ]
    .map(|(k, q)| (k.to_string(), q));
    let rated = rate_by_key(&tariffs, usage).unwrap();
    let summary: Vec<_> = rated
        .iter()
        .map(|(k, pieces)| (k.as_str(), pieces[0].quantity.to_decimal(), total(pieces)))
        .collect();
    assert_eq!(
        summary,
        [("NT", dec!(42), dec!(10.08)), ("HT", dec!(100), dec!(32))]
    );
    // A negative sum is priced with the direction in the quantity.
    let rated = rate_by_key(&tariffs, [("HT".to_string(), dec!(-5))]).unwrap();
    assert_eq!(total(&rated[0].1), dec!(-1.60));
}

#[test]
fn rate_by_key_names_the_unknown_key() {
    let tariffs = BTreeMap::from([(1u8, Tariff::PerUnit(p(dec!(1))))]);
    let e = rate_by_key(&tariffs, [(1, dec!(1)), (7, dec!(2))]).unwrap_err();
    assert_eq!(e.kind(), &ErrorKind::UnknownKey { key: "7".into() });
    assert_eq!(e.to_string(), "no tariff for key \"7\"");

    let negative = BTreeMap::from([(1u8, Tariff::PerUnit(p(dec!(-1))))]);
    assert!(matches!(
        rate_by_key(&negative, [(1, dec!(1))]).unwrap_err().kind(),
        ErrorKind::NegativePrice { .. }
    ));
}

#[test]
fn percentile_is_whole_percent_in_range() {
    // `Percentile::new(0.95)` does not compile (u8); see the compile_fail doctest.
    assert_eq!(
        Percentile::new(0).unwrap_err().kind(),
        &ErrorKind::InvalidPercentile
    );
    assert_eq!(
        Percentile::new(101).unwrap_err().kind(),
        &ErrorKind::InvalidPercentile
    );
    assert_eq!(Percentile::new(95).unwrap(), Percentile::P95);
    assert_eq!(Percentile::P95.value(), 95);
}

#[test]
fn nearest_rank_returns_a_sample() {
    let mut v: Vec<Decimal> = (1..=100).rev().map(Decimal::from).collect();
    let rank = |v: &mut [Decimal], p| nearest_rank(v, Percentile::new(p).unwrap());
    assert_eq!(rank(&mut v, 95), Some(dec!(95)));
    assert_eq!(rank(&mut v, 1), Some(dec!(1)));
    assert_eq!(rank(&mut v, 100), Some(dec!(100)));
    assert_eq!(v[0], dec!(1), "sorted ascending");
    // 8640 five-minute samples (30 days): rank ⌈0.95 × 8640⌉ = 8208.
    let mut month: Vec<Decimal> = (1..=8640).map(Decimal::from).collect();
    assert_eq!(rank(&mut month, 95), Some(dec!(8208)));
    assert_eq!(rank(&mut [dec!(7)], 1), Some(dec!(7)));
    assert_eq!(rank(&mut [], 50), None);
}

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;

    #[test]
    fn tariff_round_trips_as_strings() {
        let t = Tariff::Graduated(ladder());
        let json = serde_json::to_string(&t).unwrap();
        assert_eq!(
            json,
            r#"{"Graduated":[{"up_to":"100","unit_price":"0.30","flat":"10"},{"up_to":"500","unit_price":"0.25","flat":"5"},{"up_to":null,"unit_price":"0.20"}]}"#
        );
        let back: Tariff = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);

        let pkg = Tariff::Package {
            size: dec!(1000),
            price: p(dec!(2.5)),
            rounding: PackageRounding::Down,
        };
        let json = serde_json::to_string(&pkg).unwrap();
        assert_eq!(
            json,
            r#"{"Package":{"size":"1000","price":"2.5","rounding":"Down"}}"#
        );
        assert_eq!(serde_json::from_str::<Tariff>(&json).unwrap(), pkg);
    }

    #[test]
    fn tiers_deserialise_through_validation() {
        let unsorted = r#"[{"up_to":"10","unit_price":"1"},{"up_to":"5","unit_price":"1"}]"#;
        let e = serde_json::from_str::<Tiers>(unsorted)
            .unwrap_err()
            .to_string();
        // serde carries the build error only as text: it must be exactly the
        // error `Tiers::new` returns for the same tiers (kind and location).
        let direct = Tiers::new([
            Tier::up_to(dec!(10), Price::new(dec!(1))),
            Tier::up_to(dec!(5), Price::new(dec!(1))),
        ])
        .unwrap_err();
        assert_eq!(
            direct.kind(),
            &ErrorKind::InvalidTiers(TierError::NotIncreasing)
        );
        assert!(e.starts_with(&direct.to_string()), "{e}");
        assert!(serde_json::from_str::<Tiers>("[]").is_err());
        // Numbers are refused; up_to is required (null = open).
        assert!(serde_json::from_str::<Tiers>(r#"[{"up_to":null,"unit_price":1}]"#).is_err());
        assert!(serde_json::from_str::<Tiers>(r#"[{"unit_price":"1"}]"#).is_err());
        assert!(serde_json::from_str::<Tiers>(r#"[{"up_to":null,"unit_price":"1"}]"#).is_ok());
    }

    #[test]
    fn percentile_deserialises_through_validation() {
        assert_eq!(
            serde_json::from_str::<Percentile>("95").unwrap(),
            Percentile::P95
        );
        assert!(serde_json::from_str::<Percentile>("0").is_err());
        assert_eq!(serde_json::to_string(&Percentile::P95).unwrap(), "95");
    }
}

// ── properties ────────────────────────────────────────────────────────────────

/// Valid tiers from (bound step, price in thousandths, flat) triples.
fn tiers_strategy() -> impl Strategy<Value = (Vec<(Decimal, Decimal, Option<Decimal>)>, bool)> {
    (
        prop::collection::vec((1u32..500, 0u32..2000, prop::option::of(0u32..5000)), 1..6),
        any::<bool>(),
    )
        .prop_map(|(raw, open)| {
            let mut bound = Decimal::ZERO;
            let spec = raw
                .into_iter()
                .map(|(step, price, flat)| {
                    bound += Decimal::from(step);
                    (
                        bound,
                        Decimal::new(price.into(), 3),
                        flat.map(|f| Decimal::new(f.into(), 2)),
                    )
                })
                .collect();
            (spec, open)
        })
}

fn build(spec: &[(Decimal, Decimal, Option<Decimal>)], open: bool) -> Tiers {
    let last = spec.len() - 1;
    Tiers::new(spec.iter().enumerate().map(|(i, &(b, price, flat))| {
        let t = if open && i == last {
            Tier::above(p(price))
        } else {
            Tier::up_to(b, p(price))
        };
        match flat {
            Some(f) => t.with_flat(f),
            None => t,
        }
    }))
    .unwrap()
}

/// Upper bound on quantities to try: the last bound (or 2× it when open).
fn max_q(spec: &[(Decimal, Decimal, Option<Decimal>)], open: bool) -> Decimal {
    let b = spec.last().unwrap().0;
    if open { b * dec!(2) } else { b }
}

/// Independent reference: Σ price·clamp(q − lower, 0, width) + flats of tiers reached.
fn graduated_reference(tiers: &Tiers, q: Decimal) -> Decimal {
    let mut lower = Decimal::ZERO;
    let mut sum = Decimal::ZERO;
    for (i, t) in tiers.tiers().iter().enumerate() {
        let reached = i == 0 || q > lower;
        if !reached {
            break;
        }
        let upper = t.limit().unwrap_or(Decimal::MAX);
        sum += (q.min(upper) - lower).max(Decimal::ZERO) * t.unit_price().value();
        sum += t.flat().unwrap_or_default();
        lower = upper;
    }
    sum
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(500))]

    #[test]
    fn graduated_matches_reference_and_is_monotone(
        (spec, open) in tiers_strategy(),
        a in 0u32..10_000, b in 0u32..10_000,
    ) {
        let tiers = build(&spec, open);
        let m = max_q(&spec, open);
        let q1 = m * Decimal::from(a.min(b)) / dec!(10000);
        let q2 = m * Decimal::from(a.max(b)) / dec!(10000);
        let tariff = Tariff::Graduated(tiers.clone());
        let p1 = tariff.rate(q1).unwrap();
        let p2 = tariff.rate(q2).unwrap();
        prop_assert_eq!(total(&p1), graduated_reference(&tiers, q1));
        prop_assert_eq!(p1.iter().map(|x| x.quantity.to_decimal()).sum::<Decimal>(), q1);
        prop_assert!(p1.iter().all(|x| !x.quantity.to_decimal().is_zero() || x.flat.is_some()));
        prop_assert!(total(&p1) <= total(&p2));
    }

    #[test]
    fn volume_is_the_reached_tier_for_all_units(
        (spec, open) in tiers_strategy(), a in 0u32..10_000,
    ) {
        let tiers = build(&spec, open);
        let q = max_q(&spec, open) * Decimal::from(a) / dec!(10000);
        let pieces = Tariff::Volume(tiers.clone()).rate(q).unwrap();
        // Reference tier: first with q ≤ bound, else the open last.
        let i = spec.iter().position(|&(b, _, _)| q <= b).unwrap_or(spec.len() - 1);
        let (_, price, flat) = spec[i];
        let expected = q * price + flat.unwrap_or_default();
        prop_assert_eq!(total(&pieces), expected);
        prop_assert!(pieces.len() <= 1);
        if let Some(piece) = pieces.first() {
            prop_assert_eq!(piece.tier, i);
        }
    }

    #[test]
    fn package_count_is_ceil_or_floor(q in 0u64..1_000_000, size in 1u64..5000, scale in 0u32..4) {
        let q = Decimal::new(q.try_into().unwrap(), scale);
        let size = Decimal::new(size.try_into().unwrap(), 2);
        for rounding in [PackageRounding::Up, PackageRounding::Down] {
            let pieces = Tariff::Package { size, price: p(dec!(1)), rounding }.rate(q).unwrap();
            let n = pieces.first().map_or(Decimal::ZERO, |x| x.quantity.to_decimal());
            prop_assert_eq!(n, n.trunc());
            match rounding {
                PackageRounding::Up => {
                    let ok = n * size >= q && ((n - Decimal::ONE) * size < q || q.is_zero());
                    prop_assert!(ok, "up: {} packages for {}", n, q);
                }
                PackageRounding::Down => {
                    let ok = n * size <= q && (n + Decimal::ONE) * size > q;
                    prop_assert!(ok, "down: {} packages for {}", n, q);
                }
            }
            if let Some(x) = pieces.first() {
                prop_assert_eq!(x.measured, Some(q));
            }
        }
    }

    #[test]
    fn negation_mirrors(
        (spec, open) in tiers_strategy(), a in 0u32..10_000,
    ) {
        let tiers = build(&spec, open);
        let q = max_q(&spec, open) * Decimal::from(a) / dec!(10000);
        for tariff in [Tariff::Graduated(tiers.clone()), Tariff::Volume(tiers.clone())] {
            let (neg, pos) = (total(&tariff.rate(-q).unwrap()), total(&tariff.rate(q).unwrap()));
            if q.is_zero() {
                // −0 is 0: the first tier's flat is charged once, unmirrored.
                prop_assert_eq!(neg, pos);
            } else {
                prop_assert_eq!(neg, -pos);
            }
        }
    }

    #[test]
    fn series_net_is_the_exact_sum(xs in prop::collection::vec((-100_000i64..100_000, 0i64..100_000), 0..40)) {
        let intervals: Vec<(Decimal, Price)> =
            xs.iter().map(|&(q, pr)| (Decimal::new(q, 3), p(Decimal::new(pr, 5)))).collect();
        let net: Decimal = intervals.iter().map(|(q, pr)| q * pr.value()).sum();
        let qty: Decimal = intervals.iter().map(|(q, _)| *q).sum();
        match rate_series(intervals) {
            Ok(s) => {
                prop_assert_eq!(s.net, net);
                prop_assert_eq!(s.quantity, qty);
                prop_assert!(s.average_price.value() >= Decimal::ZERO);
            }
            Err(e) => {
                let expected = matches!(e.kind(), ErrorKind::DivisionByZero | ErrorKind::NegativePrice { .. });
                prop_assert!(expected, "unexpected error {}", e);
            }
        }
    }
}
