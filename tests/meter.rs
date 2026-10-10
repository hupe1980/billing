//! Metering aggregation (spec 027): the ten kinds on fixed readings, unique
//! counting, time-weighted gauges, unit conversion, exact pricing of fractional
//! usages, and serde.

use billing::{
    Aggregation, AmountScale, Currency, Document, ErrorKind, Line, Location, Meter,
    PackageRounding, Percentile, Price, Rounding, Sample, Tariff, Tier, Tiers, UnitConversion,
    UnitPrice, Usage,
};
use rust_decimal::Decimal;
use rust_decimal::dec;

/// Readings with these values (weight 1, no key).
fn samples(values: &[Decimal]) -> Vec<Sample> {
    values.iter().copied().map(Sample::new).collect()
}

/// US1's fixed readings, in chronological order.
fn us1() -> Vec<Sample> {
    samples(&[dec!(3), dec!(9), dec!(4), dec!(7), dec!(5)])
}

fn apply(a: Aggregation, s: Vec<Sample>) -> Option<Usage> {
    a.apply(s).unwrap()
}

fn value(a: Aggregation, s: Vec<Sample>) -> Decimal {
    let u = apply(a, s).expect("a value");
    assert!(u.is_exact_decimal());
    u.to_decimal()
}

/// The net of a one-line document at EN 16931's scale.
fn billed(line: Line) -> Decimal {
    Document::builder(Currency::EUR, AmountScale::EN16931)
        .line(line)
        .build()
        .unwrap()
        .line_total()
        .to_decimal()
}

// ---------------------------------------------------------------- US1

#[test]
fn us1_fixed_readings_per_kind() {
    assert_eq!(value(Aggregation::Sum, us1()), dec!(28));
    assert_eq!(value(Aggregation::Count, us1()), dec!(5));
    assert_eq!(value(Aggregation::Min, us1()), dec!(3));
    assert_eq!(value(Aggregation::Max, us1()), dec!(9));
    assert_eq!(value(Aggregation::Latest, us1()), dec!(5));
    assert_eq!(
        value(Aggregation::Percentile(Percentile::P95), us1()),
        dec!(9)
    );
    let mean = apply(Aggregation::Mean, us1()).unwrap();
    // 28/5 is exactly 5.6, so the canonical form is 5.6 over 1.
    assert_eq!((mean.numerator(), mean.denominator()), (dec!(5.6), dec!(1)));
    assert_eq!(mean.to_decimal(), dec!(5.6));
    assert!(mean.is_exact_decimal());
}

#[test]
fn us1_empty_input_is_zero_or_no_value() {
    for zero in [
        Aggregation::Sum,
        Aggregation::Count,
        Aggregation::UniqueCount,
        Aggregation::WeightedSum,
    ] {
        assert_eq!(
            apply(zero, Vec::new()),
            Some(Usage::from(dec!(0))),
            "{zero:?}"
        );
    }
    for none in [
        Aggregation::Min,
        Aggregation::Max,
        Aggregation::Latest,
        Aggregation::Mean,
        Aggregation::WeightedMean,
        Aggregation::Percentile(Percentile::P95),
    ] {
        assert_eq!(apply(none, Vec::new()), None, "{none:?}");
    }
}

#[test]
fn us1_single_reading_is_its_value() {
    let one = || vec![Sample::new(dec!(4.2)).with_key("a")];
    for kind in [
        Aggregation::Sum,
        Aggregation::Min,
        Aggregation::Max,
        Aggregation::Latest,
        Aggregation::Mean,
        Aggregation::WeightedSum,
        Aggregation::WeightedMean,
        Aggregation::Percentile(Percentile::P95),
    ] {
        assert_eq!(
            apply(kind, one()).unwrap().to_decimal(),
            dec!(4.2),
            "{kind:?}"
        );
    }
    assert_eq!(value(Aggregation::Count, one()), dec!(1));
    assert_eq!(value(Aggregation::UniqueCount, one()), dec!(1));
}

#[test]
fn us1_negative_readings_are_accepted() {
    let s = || samples(&[dec!(-2), dec!(5), dec!(-7)]);
    assert_eq!(value(Aggregation::Sum, s()), dec!(-4));
    assert_eq!(value(Aggregation::Count, s()), dec!(3));
    assert_eq!(value(Aggregation::Min, s()), dec!(-7));
    assert_eq!(value(Aggregation::Max, s()), dec!(5));
    assert_eq!(value(Aggregation::Latest, s()), dec!(-7));
    assert_eq!(value(Aggregation::WeightedSum, s()), dec!(-4));
    assert_eq!(
        value(Aggregation::Percentile(Percentile::new(50).unwrap()), s()),
        dec!(-2)
    );
    let mean = apply(Aggregation::Mean, s()).unwrap();
    assert_eq!((mean.numerator(), mean.denominator()), (dec!(-4), dec!(3)));
    assert!(!mean.is_exact_decimal());
    assert_eq!(mean.to_decimal(), dec!(-4) / dec!(3));
}

#[test]
fn us1_mean_of_a_non_terminating_quotient_is_the_exact_fraction() {
    let mean = apply(Aggregation::Mean, samples(&[dec!(1), dec!(1), dec!(2)])).unwrap();
    assert_eq!((mean.numerator(), mean.denominator()), (dec!(4), dec!(3)));
    assert_eq!(mean.to_decimal(), dec!(1.3333333333333333333333333333));
    assert!(!mean.is_exact_decimal());
}

#[test]
fn us1_overflow_is_located_at_the_reading() {
    let err = Aggregation::Sum
        .apply(samples(&[dec!(1), Decimal::MAX - dec!(1), dec!(1)]))
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::Overflow);
    assert_eq!(err.location(), Some(&Location::Sample(2)));
    // A sum that Decimal would silently round is refused, not approximated.
    let big = Decimal::from_i128_with_scale(10i128.pow(28), 0);
    let err = Aggregation::Mean
        .apply(samples(&[big, dec!(0.1)]))
        .unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::Overflow);
    assert_eq!(err.location(), Some(&Location::Sample(1)));
}

#[test]
fn us1_equal_values_are_deterministic() {
    let s = || samples(&[dec!(2), dec!(2.0), dec!(2.00)]);
    assert_eq!(value(Aggregation::Min, s()), dec!(2));
    assert_eq!(value(Aggregation::Max, s()), dec!(2));
    assert_eq!(
        value(Aggregation::Percentile(Percentile::P95), s()),
        dec!(2)
    );
}

#[test]
fn mean_bills_the_exact_cent() {
    let mean = Aggregation::Mean
        .apply(samples(&[dec!(1), dec!(0), dec!(0)]))
        .unwrap()
        .unwrap();
    let line = Line::from_usage("avg", mean, "C62", UnitPrice::new(Price::new(dec!(0.015))));
    assert_eq!(line.exact_quantity(), Some((dec!(1), dec!(3))));
    assert_eq!(line.quantity().value(), mean.to_decimal());
    assert_eq!(billed(line), dec!(0.01));
    // The 28-digit decimal reading bills 0.00: the defect the fraction avoids.
    let decimal = Line::new(
        "avg",
        billing::Quantity::new(mean.to_decimal(), "C62"),
        UnitPrice::new(Price::new(dec!(0.015))),
    );
    assert_eq!(billed(decimal), dec!(0.00));
}

#[test]
fn from_usage_of_a_decimal_is_an_ordinary_line() {
    let price = UnitPrice::new(Price::new(dec!(0.30)));
    let line = Line::from_usage("kwh", Usage::from(dec!(10)), "KWH", price.clone());
    assert_eq!(
        line,
        Line::new("kwh", billing::Quantity::new(dec!(10), "KWH"), price)
    );
    assert_eq!(line.exact_quantity(), None);
}

#[test]
fn exact_line_negates_and_segments() {
    let mean = Aggregation::Mean
        .apply(samples(&[dec!(1), dec!(0), dec!(0)]))
        .unwrap()
        .unwrap();
    let line = Line::from_usage("avg", mean, "C62", UnitPrice::new(Price::new(dec!(0.015))));
    assert_eq!(line.exact_net().unwrap(), dec!(0.005));
    let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
        .line(line.clone())
        .build()
        .unwrap();
    let neg = doc.negated();
    assert_eq!(neg.line_total().to_decimal(), dec!(-0.01));
    assert_eq!(neg.lines()[0].exact_quantity(), Some((dec!(-1), dec!(3))));
    let parts = line
        .segments(&[dec!(1), dec!(1)], AmountScale::EN16931)
        .unwrap();
    let total: Decimal = parts.iter().map(|p| p.stated_net().unwrap()).sum();
    assert_eq!(total, dec!(0.01));
    assert!(parts.iter().all(|p| p.exact_quantity().is_none()));
}

#[test]
fn sample_accessors_return_what_was_set() {
    let s = Sample::new(dec!(10))
        .with_weight(dec!(0.25))
        .with_key("vm-1");
    assert_eq!(
        (s.value(), s.weight(), s.key()),
        (dec!(10), dec!(0.25), Some("vm-1"))
    );
    let plain = Sample::new(dec!(-3));
    assert_eq!(
        (plain.value(), plain.weight(), plain.key()),
        (dec!(-3), dec!(1), None)
    );
}

// ---------------------------------------------------------------- US2

#[test]
fn us2_unique_count_counts_distinct_keys() {
    let s = ["a", "b", "a", "c"]
        .into_iter()
        .map(|k| Sample::new(dec!(1)).with_key(k));
    assert_eq!(
        Aggregation::UniqueCount.apply(s).unwrap(),
        Some(Usage::from(dec!(3)))
    );
}

#[test]
fn us2_missing_key_is_located() {
    let s = vec![
        Sample::new(dec!(1)).with_key("a"),
        Sample::new(dec!(1)).with_key("b"),
        Sample::new(dec!(1)),
        Sample::new(dec!(1)).with_key("c"),
    ];
    let err = Aggregation::UniqueCount.apply(s).unwrap_err();
    assert_eq!(err.kind(), &ErrorKind::MissingKey);
    assert_eq!(err.location(), Some(&Location::Sample(2)));
    assert_eq!(err.rule(), None);
}

#[test]
fn us2_keys_are_ignored_by_other_kinds() {
    let keyed = || {
        vec![
            Sample::new(dec!(3)).with_key("a"),
            Sample::new(dec!(9)),
            Sample::new(dec!(4)).with_key("a"),
        ]
    };
    assert_eq!(value(Aggregation::Sum, keyed()), dec!(16));
    assert_eq!(value(Aggregation::Count, keyed()), dec!(3));
    assert_eq!(value(Aggregation::Max, keyed()), dec!(9));
}

// ---------------------------------------------------------------- US3

#[test]
fn us3_weighted_sum_is_exact() {
    let s = vec![
        Sample::new(dec!(10)).with_weight(dec!(0.25)),
        Sample::new(dec!(20)).with_weight(dec!(0.75)),
    ];
    assert_eq!(value(Aggregation::WeightedSum, s), dec!(17.5));
}

#[test]
fn us3_weighted_mean_is_the_exact_fraction() {
    let s = vec![
        Sample::new(dec!(10)).with_weight(dec!(1)),
        Sample::new(dec!(20)).with_weight(dec!(2)),
    ];
    let u = apply(Aggregation::WeightedMean, s).unwrap();
    assert_eq!((u.numerator(), u.denominator()), (dec!(50), dec!(3)));
    assert_eq!(u.to_decimal(), dec!(16.666666666666666666666666667));
    assert!(!u.is_exact_decimal());
    let gauge = vec![
        Sample::new(dec!(10)).with_weight(dec!(0.25)),
        Sample::new(dec!(20)).with_weight(dec!(0.75)),
    ];
    assert_eq!(
        apply(Aggregation::WeightedMean, gauge)
            .unwrap()
            .to_decimal(),
        dec!(17.5)
    );
}

#[test]
fn us3_unit_weights_equal_the_plain_kinds() {
    assert_eq!(
        value(Aggregation::WeightedSum, us1()),
        value(Aggregation::Sum, us1())
    );
    assert_eq!(
        apply(Aggregation::WeightedMean, us1()),
        apply(Aggregation::Mean, us1())
    );
}

#[test]
fn us3_zero_total_weight_is_no_value() {
    let s = vec![
        Sample::new(dec!(10)).with_weight(dec!(0)),
        Sample::new(dec!(20)).with_weight(dec!(0)),
    ];
    assert_eq!(apply(Aggregation::WeightedMean, s.clone()), None);
    assert_eq!(
        apply(Aggregation::WeightedSum, s),
        Some(Usage::from(dec!(0)))
    );
}

#[test]
fn us3_negative_weight_is_located() {
    let s = || {
        vec![
            Sample::new(dec!(10)),
            Sample::new(dec!(20)).with_weight(dec!(-1)),
        ]
    };
    for kind in [Aggregation::WeightedSum, Aggregation::WeightedMean] {
        let err = kind.apply(s()).unwrap_err();
        assert_eq!(err.kind(), &ErrorKind::NegativeWeight { weight: dec!(-1) });
        assert_eq!(err.location(), Some(&Location::Sample(1)));
    }
    // Only the weighted kinds check weights.
    assert_eq!(value(Aggregation::Sum, s()), dec!(30));
}

// ---------------------------------------------------------------- conversion

fn convert(divisor: Decimal, decimals: u8, rounding: Rounding) -> Meter {
    Meter::new(Aggregation::Sum)
        .with_conversion(UnitConversion::new(divisor, decimals, rounding).unwrap())
}

#[test]
fn conversion_seconds_to_started_hours() {
    let hours = convert(dec!(3600), 0, Rounding::AwayFromZero);
    assert_eq!(
        hours.apply(samples(&[dec!(7201)])).unwrap(),
        Some(Usage::from(dec!(3)))
    );
    assert_eq!(
        hours.apply(samples(&[dec!(3600), dec!(3600)])).unwrap(),
        Some(Usage::from(dec!(2)))
    );
}

#[test]
fn conversion_megabytes_to_whole_gigabytes() {
    let gb = convert(dec!(1024), 0, Rounding::TowardZero);
    let u = gb.apply(samples(&[dec!(1536)])).unwrap().unwrap();
    assert_eq!((u.to_decimal(), u.denominator()), (dec!(1), dec!(1)));
}

#[test]
fn conversion_of_a_mean_rounds_once_from_the_fraction() {
    // Mean 1/3 ÷ 1 at 2 dp: 0.33 (half away), 0.34 (away from zero).
    let m = |rounding| {
        Meter::new(Aggregation::Mean)
            .with_conversion(UnitConversion::new(dec!(1), 2, rounding).unwrap())
            .apply(samples(&[dec!(1), dec!(0), dec!(0)]))
            .unwrap()
            .unwrap()
    };
    assert_eq!(m(Rounding::HalfAwayFromZero), Usage::from(dec!(0.33)));
    assert_eq!(m(Rounding::AwayFromZero), Usage::from(dec!(0.34)));
    // 5/2 ÷ 0.5 = 5 exactly; 5/3 ÷ 0.5 = 3.33… → 4 away from zero.
    let w = Meter::new(Aggregation::Mean)
        .with_conversion(UnitConversion::new(dec!(0.5), 0, Rounding::AwayFromZero).unwrap());
    assert_eq!(
        w.apply(samples(&[dec!(2), dec!(3)])).unwrap(),
        Some(Usage::from(dec!(5)))
    );
    assert_eq!(
        w.apply(samples(&[dec!(2), dec!(3), dec!(0)])).unwrap(),
        Some(Usage::from(dec!(4)))
    );
}

#[test]
fn conversion_refuses_invalid_settings() {
    for (divisor, decimals) in [(dec!(0), 0), (dec!(-1), 0), (dec!(1), 29)] {
        let err = UnitConversion::new(divisor, decimals, Rounding::TowardZero).unwrap_err();
        assert_eq!(
            err.kind(),
            &ErrorKind::InvalidConversion,
            "{divisor} {decimals}"
        );
        assert_eq!(err.location(), None);
    }
    let c = UnitConversion::new(dec!(3600), 28, Rounding::HalfEven).unwrap();
    assert_eq!(
        (c.divisor(), c.decimals(), c.rounding()),
        (dec!(3600), 28, Rounding::HalfEven)
    );
}

#[test]
fn conversion_keeps_no_value_and_zero() {
    let c = UnitConversion::new(dec!(3600), 0, Rounding::AwayFromZero).unwrap();
    let max = Meter::new(Aggregation::Max).with_conversion(c);
    assert_eq!(max.apply(Vec::new()).unwrap(), None);
    let sum = Meter::new(Aggregation::Sum).with_conversion(c);
    assert_eq!(sum.apply(Vec::new()).unwrap(), Some(Usage::from(dec!(0))));
    // Without a conversion the meter is its aggregation.
    let plain = Meter::new(Aggregation::Mean);
    assert_eq!(plain.conversion, None);
    assert_eq!(
        plain.apply(us1()).unwrap(),
        Aggregation::Mean.apply(us1()).unwrap()
    );
}

// ---------------------------------------------------------------- exact tariffs

fn third() -> Usage {
    Aggregation::Mean
        .apply(samples(&[dec!(1), dec!(0), dec!(0)]))
        .unwrap()
        .unwrap()
}

fn billed_pieces(t: &Tariff, u: Usage) -> Decimal {
    let lines = t
        .rate(u)
        .unwrap()
        .into_iter()
        .flat_map(|p| p.into_lines("usage", "C62"));
    Document::builder(Currency::EUR, AmountScale::EN16931)
        .lines(lines)
        .build()
        .unwrap()
        .line_total()
        .to_decimal()
}

#[test]
fn tariff_exact_graduated_bills_the_exact_cent() {
    let tiers = Tiers::new([
        Tier::up_to(dec!(1), Price::new(dec!(0.015))),
        Tier::above(Price::new(dec!(0.01))),
    ])
    .unwrap();
    let t = Tariff::Graduated(tiers);
    let pieces = t.rate(third()).unwrap();
    assert_eq!(pieces.len(), 1);
    assert_eq!(pieces[0].quantity, third());
    assert_eq!(pieces[0].exact_net().unwrap(), dec!(0.005));
    assert_eq!(billed_pieces(&t, third()), dec!(0.01));
    // 4/3 crosses the bound: 1 at 0.015, then the fraction 1/3 at 0.01.
    let four_thirds = Aggregation::Mean
        .apply(samples(&[dec!(1), dec!(1), dec!(2)]))
        .unwrap()
        .unwrap();
    let pieces = t.rate(four_thirds).unwrap();
    assert_eq!(pieces[0].quantity, Usage::from(dec!(1)));
    assert_eq!(
        (
            pieces[1].quantity.numerator(),
            pieces[1].quantity.denominator()
        ),
        (dec!(1), dec!(3))
    );
    // 0.015 + round(0.0033…) = 0.015 → 0.02 and 0.00
    assert_eq!(billed_pieces(&t, four_thirds), dec!(0.02));
}

#[test]
fn tariff_exact_bounds_compare_the_fraction() {
    // 1000001/1000000 is above 1, though close: the volume tier is 1.
    let tiers = Tiers::new([
        Tier::up_to(dec!(1), Price::new(dec!(3))),
        Tier::above(Price::new(dec!(2))),
    ])
    .unwrap();
    let just_above = Aggregation::Mean
        .apply(samples(&[dec!(3.000003), dec!(0), dec!(0)]))
        .unwrap()
        .unwrap();
    let v = Tariff::Volume(tiers.clone()).rate(just_above).unwrap();
    assert_eq!(v[0].tier, 1);
    let exactly_one = Aggregation::Mean
        .apply(samples(&[dec!(3), dec!(0), dec!(0)]))
        .unwrap()
        .unwrap();
    let v = Tariff::Volume(tiers).rate(exactly_one).unwrap();
    assert_eq!(v[0].tier, 0);
}

#[test]
fn tariff_exact_volume_and_package_on_a_fraction() {
    let tiers = Tiers::new([
        Tier::up_to(dec!(1), Price::new(dec!(0.015))),
        Tier::above(Price::new(dec!(0.01))),
    ])
    .unwrap();
    let v = Tariff::Volume(tiers);
    assert_eq!(billed_pieces(&v, third()), dec!(0.01));
    let pkg = |rounding| Tariff::Package {
        size: dec!(0.25),
        price: Price::new(dec!(2)),
        rounding,
    };
    // 1/3 ÷ 0.25 = 4/3 packages.
    let up = pkg(PackageRounding::Up).rate(third()).unwrap();
    assert_eq!(up[0].quantity, Usage::from(dec!(2)));
    assert_eq!(up[0].measured, Some(third().to_decimal()));
    let down = pkg(PackageRounding::Down).rate(third()).unwrap();
    assert_eq!(down[0].quantity, Usage::from(dec!(1)));
    // Negative: the exact negation.
    let neg = Aggregation::Mean
        .apply(samples(&[dec!(-1), dec!(0), dec!(0)]))
        .unwrap()
        .unwrap();
    let pieces = Tariff::PerUnit(Price::new(dec!(0.015))).rate(neg).unwrap();
    assert_eq!(
        (
            pieces[0].quantity.numerator(),
            pieces[0].quantity.denominator()
        ),
        (dec!(-1), dec!(3))
    );
    assert_eq!(
        billed_pieces(&Tariff::PerUnit(Price::new(dec!(0.015))), neg),
        dec!(-0.01)
    );
}

#[test]
fn tariff_exact_plain_decimal_is_unchanged() {
    let tiers = Tiers::new([
        Tier::up_to(dec!(100), Price::new(dec!(0.30))),
        Tier::above(Price::new(dec!(0.25))),
    ])
    .unwrap();
    let pieces = Tariff::Graduated(tiers).rate(dec!(150)).unwrap();
    assert_eq!(pieces[1].quantity, Usage::from(dec!(50)));
    assert_eq!(pieces[1].quantity.to_decimal(), dec!(50));
    assert_eq!(
        pieces[0].clone().into_lines("x", "C62")[0].exact_quantity(),
        None
    );
}

// ---------------------------------------------------------------- serde

#[cfg(feature = "serde")]
mod serde_tests {
    use super::*;

    fn round_trip<T>(v: &T) -> String
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + core::fmt::Debug,
    {
        let json = serde_json::to_string(v).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, v);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        json
    }

    #[test]
    fn serde_every_kind_round_trips_byte_identically() {
        for kind in [
            Aggregation::Sum,
            Aggregation::Count,
            Aggregation::UniqueCount,
            Aggregation::Min,
            Aggregation::Max,
            Aggregation::Latest,
            Aggregation::Mean,
            Aggregation::WeightedSum,
            Aggregation::WeightedMean,
            Aggregation::Percentile(Percentile::P95),
        ] {
            round_trip(&kind);
            round_trip(&Meter::new(kind));
        }
        let m = Meter::new(Aggregation::Sum)
            .with_conversion(UnitConversion::new(dec!(3600), 0, Rounding::AwayFromZero).unwrap());
        let json = round_trip(&m);
        assert!(json.contains(r#""divisor":"3600""#), "{json}");
        round_trip(
            &Sample::new(dec!(1.50))
                .with_weight(dec!(0.25))
                .with_key("a"),
        );
        let s = round_trip(&Sample::new(dec!(2)));
        assert_eq!(s, r#"{"value":"2"}"#);
        // Only the default weight 1 is omitted; 2 and 1.0 are written as given.
        let s = round_trip(&Sample::new(dec!(2)).with_weight(dec!(2)));
        assert_eq!(s, r#"{"value":"2","weight":"2"}"#);
        let s = round_trip(&Sample::new(dec!(2)).with_weight(dec!(1.0)));
        assert_eq!(s, r#"{"value":"2","weight":"1.0"}"#);
        let u = round_trip(&third());
        assert_eq!(u, r#"{"numerator":"1","denominator":"3"}"#);
        let line = Line::from_usage(
            "avg",
            third(),
            "C62",
            UnitPrice::new(Price::new(dec!(0.015))),
        );
        let json = round_trip(&line);
        assert!(json.contains(r#""quantity_exact""#), "{json}");
    }

    #[test]
    fn serde_refuses_json_numbers_and_invalid_values() {
        assert!(serde_json::from_str::<Sample>(r#"{"value":1}"#).is_err());
        assert!(serde_json::from_str::<Usage>(r#"{"numerator":1,"denominator":"3"}"#).is_err());
        assert!(serde_json::from_str::<Aggregation>(r#"{"Percentile":0}"#).is_err());
        assert!(serde_json::from_str::<Aggregation>(r#"{"Percentile":101}"#).is_err());
        assert!(serde_json::from_str::<Usage>(r#"{"numerator":"1","denominator":"0"}"#).is_err());
        assert!(serde_json::from_str::<Usage>(r#"{"numerator":"1","denominator":"-3"}"#).is_err());
        let bad = r#"{"aggregation":"Sum","conversion":{"divisor":"0","decimals":0,"rounding":"TowardZero"}}"#;
        assert!(serde_json::from_str::<Meter>(bad).is_err());
        let bad = r#"{"divisor":"1","decimals":29,"rounding":"TowardZero"}"#;
        assert!(serde_json::from_str::<UnitConversion>(bad).is_err());
    }

    #[test]
    fn serde_line_with_a_tampered_quantity_is_refused() {
        let line = Line::from_usage(
            "avg",
            third(),
            "C62",
            UnitPrice::new(Price::new(dec!(0.015))),
        );
        let json = serde_json::to_string(&line).unwrap();
        let shown = third().to_decimal().to_string();
        let tampered = json.replace(&shown, "0.5");
        assert_ne!(tampered, json);
        assert!(serde_json::from_str::<Line>(&tampered).is_err());
        let zero = json.replace(r#""denominator":"3""#, r#""denominator":"0""#);
        assert_ne!(zero, json);
        assert!(serde_json::from_str::<Line>(&zero).is_err());
    }
}

#[test]
fn usage_equality_compares_values() {
    use billing::{Aggregation, Sample, Usage};
    let mean = |xs: &[Decimal]| {
        Aggregation::Mean
            .apply(xs.iter().map(|&v| Sample::new(v)))
            .unwrap()
            .unwrap()
    };
    // 28/5 and 56/10 are the same value 5.6.
    let a = mean(&[dec!(28), dec!(0), dec!(0), dec!(0), dec!(0)]);
    let b = mean(&[
        dec!(28),
        dec!(28),
        dec!(0),
        dec!(0),
        dec!(0),
        dec!(0),
        dec!(0),
        dec!(0),
        dec!(0),
        dec!(0),
    ]);
    assert_eq!(a, b);
    assert_eq!(a, Usage::from(dec!(5.6)));
    // 1/3 reached two ways is one value.
    let c = mean(&[dec!(1), dec!(0), dec!(0)]);
    let d = mean(&[dec!(2), dec!(0), dec!(0), dec!(0), dec!(0), dec!(0)]);
    assert_eq!(c, d);
    assert_eq!((c.numerator(), c.denominator()), (dec!(1), dec!(3)));
    assert_ne!(c, Usage::from(dec!(0.3333333333333333333333333333)));
}
