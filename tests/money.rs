//! Properties of the money core: `Amount`, `Rounding`, the decimal grammar and
//! (with `--features serde`) the string serde codec.

use billing::{Amount, AmountScale, ErrorKind, Rounding};
use proptest::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy, dec};

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

fn decimal() -> impl Strategy<Value = Decimal> {
    (-1_000_000_000_000i64..1_000_000_000_000, 0u32..=12).prop_map(|(m, s)| Decimal::new(m, s))
}

fn mode() -> impl Strategy<Value = Rounding> {
    prop::sample::select(MODES.to_vec())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2000))]

    #[test]
    fn display_parse_round_trip(raw in any::<i64>()) {
        let a = Amount::<2>::from_raw(raw);
        prop_assert_eq!(Amount::<2>::parse(&a.to_string()).unwrap(), a);
        let b = Amount::<5>::from_raw(raw);
        prop_assert_eq!(b.to_string().parse::<Amount<5>>().unwrap(), b);
        let c = Amount::<0>::from_raw(raw);
        prop_assert_eq!(Amount::<0>::parse(&c.to_string()).unwrap(), c);
        prop_assert_eq!(Amount::<5>::from_decimal(b.to_decimal()).unwrap(), b);
    }

    #[test]
    fn parse_display_parse_is_stable(s in "[+-]?[0-9]{1,12}(\\.[0-9]{1,5})?") {
        let a = Amount::<5>::parse(&s).unwrap();
        prop_assert_eq!(Amount::<5>::parse(&a.to_string()).unwrap(), a);
        // The grammar is exactly what Decimal reads for these strings.
        prop_assert_eq!(a.to_decimal(), s.trim_start_matches('+').parse::<Decimal>().unwrap());
    }

    #[test]
    fn parse_refuses_comma(int in "[0-9]{1,6}", frac in "[0-9]{1,6}") {
        let s = format!("{int},{frac}");
        let err = Amount::<5>::parse(&s).unwrap_err();
        prop_assert!(matches!(err.kind(), ErrorKind::Malformed { .. }), "{s}");
    }

    #[test]
    fn parse_refuses_exponent_and_spaces(int in "[0-9]{1,6}", e in 0u8..9) {
        for s in [format!("{int}e{e}"), format!("{int}E-{e}"), format!(" {int}"), format!("{int} "), format!("- {int}")] {
            let err = Amount::<5>::parse(&s).unwrap_err();
            prop_assert!(matches!(err.kind(), ErrorKind::Malformed { .. }), "{s}");
        }
    }

    #[test]
    fn rounding_is_sign_symmetric(x in decimal(), r in mode()) {
        let pos = Amount::<2>::from_decimal_rounded(x, r).unwrap();
        let neg = Amount::<2>::from_decimal_rounded(-x, r).unwrap();
        prop_assert_eq!(neg, -pos);
        let a = Amount::<5>::from_decimal_rounded(x, r).unwrap();
        prop_assert_eq!(
            (-a).round_to(2, r).unwrap(),
            -(a.round_to(2, r).unwrap())
        );
        prop_assert_eq!(
            (-a).rescale::<1>(r).unwrap(),
            -(a.rescale::<1>(r).unwrap())
        );
    }

    #[test]
    fn rounding_agrees_with_decimal(x in decimal(), r in mode(), dp in 0u8..=5) {
        let ours = AmountScale::new(dp, r).unwrap().reduce::<5>(x).unwrap();
        let theirs = x.round_dp_with_strategy(u32::from(dp), strategy(r));
        prop_assert_eq!(ours.to_decimal(), theirs);
    }

    #[test]
    fn half_even_and_half_away_differ_only_on_ties(units in -1_000_000i64..1_000_000, frac in 0i64..10) {
        // x = units + frac/10 at one decimal; a tie is frac == 5.
        let x = Decimal::new(units * 10 + if units < 0 { -frac } else { frac }, 1);
        let hafz = Amount::<0>::from_decimal_rounded(x, Rounding::HalfAwayFromZero).unwrap().to_raw();
        let he = Amount::<0>::from_decimal_rounded(x, Rounding::HalfEven).unwrap().to_raw();
        if frac == 5 {
            prop_assert_eq!(hafz, units + if x.is_sign_negative() { -1 } else { 1 });
            prop_assert_eq!(he % 2, 0);
            prop_assert!((hafz - he).abs() <= 1);
        } else {
            prop_assert_eq!(hafz, he);
        }
    }

    #[test]
    fn round_to_increment_lands_on_a_multiple(raw in -1_000_000_000i64..1_000_000_000, inc in 1i64..10_000, r in mode()) {
        let a = Amount::<5>::from_raw(raw);
        let i = Amount::<5>::from_raw(inc);
        let rounded = a.round_to_increment(i, r).unwrap();
        prop_assert_eq!(rounded.to_raw() % inc, 0);
        prop_assert!((rounded.to_raw() - raw).abs() < inc);
        prop_assert_eq!((-a).round_to_increment(i, r).unwrap(), -rounded);
    }

    #[test]
    fn checked_ops_never_panic(a in any::<i64>(), b in any::<i64>()) {
        let (x, y) = (Amount::<2>::from_raw(a), Amount::<2>::from_raw(b));
        prop_assert_eq!(x.checked_add(y).ok().map(Amount::to_raw), a.checked_add(b));
        prop_assert_eq!(x.checked_sub(y).ok().map(Amount::to_raw), a.checked_sub(b));
        prop_assert_eq!(x.checked_neg().ok().map(Amount::to_raw), a.checked_neg());
        prop_assert_eq!(x.checked_abs().ok().map(Amount::to_raw), a.checked_abs());
    }
}

/// Raw values with the extremes over-represented.
fn raw() -> impl Strategy<Value = i64> {
    prop_oneof![
        any::<i64>(),
        prop::sample::select(vec![
            i64::MIN,
            i64::MIN + 1,
            i64::MIN / 2,
            -1,
            0,
            1,
            i64::MAX / 2,
            i64::MAX - 1,
            i64::MAX,
        ]),
    ]
}

/// The error kinds the money core may return; anything else (or a panic) fails.
fn allowed(e: &billing::Error) -> bool {
    matches!(
        e.kind(),
        ErrorKind::Overflow
            | ErrorKind::ExcessPrecision { .. }
            | ErrorKind::InvalidIncrement
            | ErrorKind::InvalidWeights
            | ErrorKind::ScaleMismatch
    )
}

fn ok_or_typed<T>(r: Result<T, billing::Error>) -> Result<(), TestCaseError> {
    if let Err(e) = &r {
        prop_assert!(allowed(e), "unexpected kind {:?}", e.kind());
    }
    Ok(())
}

/// Every fallible money operation at precision `P` returns `Ok` or a typed
/// error — and never panics — for any raw values, decimals and rounding mode.
fn no_panic_at<const P: u8>(
    a: i64,
    b: i64,
    decimals: u8,
    scale: u32,
    r: Rounding,
    weights: &[Decimal],
) -> Result<(), TestCaseError> {
    let (x, y) = (Amount::<P>::from_raw(a), Amount::<P>::from_raw(b));
    ok_or_typed(x.checked_add(y))?;
    ok_or_typed(x.checked_sub(y))?;
    ok_or_typed(x.checked_neg())?;
    ok_or_typed(x.checked_abs())?;
    ok_or_typed(Amount::<P>::checked_sum([x, y, x]))?;
    ok_or_typed(x.round_to(decimals, r))?;
    match x.round_to_increment(y, r) {
        Ok(v) => prop_assert_eq!(v.to_raw() % b, 0),
        Err(e) if b <= 0 => prop_assert_eq!(e.kind(), &ErrorKind::InvalidIncrement),
        Err(e) => prop_assert_eq!(e.kind(), &ErrorKind::Overflow),
    }
    let _ = x.fits(decimals);
    ok_or_typed(x.rescale::<0>(r))?;
    ok_or_typed(x.rescale::<2>(r))?;
    ok_or_typed(x.rescale::<5>(r))?;
    ok_or_typed(x.rescale::<18>(r))?;
    ok_or_typed(x.exact::<0>())?;
    ok_or_typed(x.exact::<2>())?;
    ok_or_typed(x.exact::<5>())?;
    ok_or_typed(x.exact::<18>())?;
    // from_decimal(_rounded) on the amount itself and on the raw value read at
    // any Decimal scale.
    for d in [
        x.to_decimal(),
        Decimal::from_i128_with_scale(i128::from(a), scale),
    ] {
        ok_or_typed(Amount::<P>::from_decimal(d))?;
        ok_or_typed(Amount::<P>::from_decimal_rounded(d, r))?;
        ok_or_typed(Amount::<0>::from_decimal_rounded(d, r))?;
        ok_or_typed(Amount::<18>::from_decimal(d))?;
        ok_or_typed(billing::split(d, weights, decimals))?;
        if let Ok(s) = AmountScale::new(decimals, r) {
            ok_or_typed(s.reduce::<P>(d))?;
        }
    }
    ok_or_typed(x.split(weights))?;
    let _ = format!("{x} {x:.0} {x:.2} {x:.30}");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(3000))]

    #[test]
    fn money_operations_never_panic(
        a in raw(),
        b in raw(),
        decimals in 0u8..=40,
        scale in 0u32..=28,
        weights in prop::collection::vec(prop_oneof![Just(0i64), any::<i64>(), 0i64..100], 1..5),
    ) {
        let weights: Vec<Decimal> = weights.into_iter().map(Decimal::from).collect();
        for r in MODES {
            no_panic_at::<0>(a, b, decimals, scale, r, &weights)?;
            no_panic_at::<2>(a, b, decimals, scale, r, &weights)?;
            no_panic_at::<5>(a, b, decimals, scale, r, &weights)?;
            no_panic_at::<18>(a, b, decimals, scale, r, &weights)?;
        }
    }
}

#[test]
fn rounded_once_not_twice() {
    // 10.02629 × 19 % = 1.9049951 → 1.90; rounding to 5 dp first gives 1.91.
    let base = Amount::<5>::parse("10.02629").unwrap();
    let exact = base.to_decimal() * dec!(0.19);
    assert_eq!(
        AmountScale::EN16931.reduce::<2>(exact).unwrap().to_string(),
        "1.90"
    );
}

#[test]
fn credit_rounds_like_debit_in_every_mode() {
    for r in MODES {
        let debit = Amount::<2>::from_decimal_rounded(dec!(1.005), r).unwrap();
        let credit = Amount::<2>::from_decimal_rounded(dec!(-1.005), r).unwrap();
        assert_eq!(credit, -debit, "{r:?}");
    }
}

#[test]
fn thousands_separator_is_refused() {
    for s in ["1,000", "12,345", "1.000,00"] {
        assert!(matches!(
            Amount::<2>::parse(s).unwrap_err().kind(),
            ErrorKind::Malformed { .. }
        ));
    }
}

/// Pins the `rust_decimal` floor (>= 1.42.1): the two vectors of
/// rust_decimal PR #804 (`subtract_borrow_propagation_into_upper_words`), where
/// the borrow of an unaligned subtraction into the upper words was lost or
/// spurious and the result off by about 2^128. Asserted on `Decimal` itself
/// (re-exported as `billing::Decimal`), since every amount the crate derives
/// passes through its subtraction; no `billing` operation reaches these exact
/// operands.
#[test]
fn decimal_subtraction_borrow_propagation_pr_804() {
    use billing::Decimal;
    let seven = Decimal::from_str_exact("7.0000000000000000000000000000").unwrap();
    for (minuend, expected) in [
        (dec!(34028236693), dec!(34028236686)),
        (dec!(34028236701), dec!(34028236694)),
    ] {
        let msg = "rust_decimal subtraction is wrong (PR #804): require rust_decimal >= 1.42.1";
        assert_eq!(minuend - seven, expected, "{msg}");
        assert_eq!(minuend.checked_sub(seven), Some(expected), "{msg}");
    }
}

#[cfg(feature = "serde")]
mod serde_codec {
    use billing::{Amount, AmountScale, Currency, Price, Quantity, Rounding, UnitPrice};
    use proptest::prelude::*;
    use rust_decimal::{Decimal, dec};

    fn round_trip<T>(value: &T) -> String
    where
        T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + core::fmt::Debug,
    {
        let json = serde_json::to_string(value).unwrap();
        let back: T = serde_json::from_str(&json).unwrap();
        assert_eq!(&back, value);
        assert_eq!(serde_json::to_string(&back).unwrap(), json);
        json
    }

    proptest! {
        #[test]
        fn amount_round_trips_as_a_string(raw in any::<i64>()) {
            let a = Amount::<2>::from_raw(raw);
            let json = round_trip(&a);
            prop_assert_eq!(json, format!("\"{a}\""));
        }

        #[test]
        fn decimals_round_trip_byte_identical(m in any::<i64>(), s in 0u32..=12) {
            let d = Decimal::new(m, s);
            let p = Price::new(d);
            prop_assert_eq!(round_trip(&p), format!("\"{d}\""));
            round_trip(&Quantity::new(d, "KWH"));
            round_trip(&UnitPrice::new(p).with_base_quantity(d).with_unit("KWH"));
        }
    }

    #[test]
    fn shapes() {
        assert_eq!(round_trip(&Currency::EUR), "\"EUR\"");
        assert_eq!(round_trip(&Rounding::HalfEven), "\"HalfEven\"");
        assert_eq!(
            round_trip(&AmountScale::EN16931),
            r#"{"decimals":2,"rounding":"HalfAwayFromZero"}"#
        );
        assert_eq!(
            round_trip(&Quantity::new(dec!(-1.50), "C62")),
            r#"{"value":"-1.50","unit":"C62"}"#
        );
        assert_eq!(
            round_trip(&UnitPrice::new(Price::new(dec!(0.123456)))),
            r#"{"price":"0.123456","base_quantity":"1","unit":null}"#
        );
    }

    #[test]
    fn numbers_and_bad_strings_are_refused() {
        assert!(serde_json::from_str::<Amount<2>>("1.5").is_err());
        assert!(serde_json::from_str::<Amount<2>>("1").is_err());
        assert!(serde_json::from_str::<Amount<2>>("\"1,000\"").is_err());
        assert!(serde_json::from_str::<Amount<2>>("\"1.005\"").is_err());
        assert!(serde_json::from_str::<Price>("0.5").is_err());
        assert!(serde_json::from_str::<Price>("\"1e3\"").is_err());
        assert!(serde_json::from_str::<Quantity>(r#"{"value":1,"unit":"C62"}"#).is_err());
        assert!(serde_json::from_str::<Currency>("\"EURO\"").is_err());
        assert!(
            serde_json::from_str::<AmountScale>(r#"{"decimals":6,"rounding":"HalfEven"}"#).is_err()
        );
        assert!(serde_json::from_str::<Rounding>("\"Ceiling\"").is_err());
        let a: Amount<2> = serde_json::from_str("\"-0.50\"").unwrap();
        assert_eq!(a.to_raw(), -50);
    }
}

/// Past `Decimal`'s 28 fraction digits, `Amount::parse` still reports the
/// value (rounded, with its sign) against the amount's own `P`.
#[test]
fn parse_past_decimal_precision_reports_p() {
    let err = Amount::<2>::parse("-0.12345678901234567890123456789").unwrap_err();
    let ErrorKind::ExcessPrecision { value, decimals } = *err.kind() else {
        panic!("excess precision expected, got {err:?}");
    };
    assert_eq!(decimals, 2);
    assert!(value.is_sign_negative());
}

#[test]
fn zero_is_neither_negative_nor_positive() {
    let z = Amount::<2>::ZERO;
    assert!(!z.is_negative());
    assert!(!z.is_positive());
    assert!(Amount::<2>::parse("-0.01").unwrap().is_negative());
}

#[test]
fn price_displays_its_exact_value() {
    assert_eq!(billing::Price::new(dec!(0.30120)).to_string(), "0.30120");
}

#[test]
fn value_types_honour_width_and_alignment() {
    use billing::{Currency, Percent, TaxCategory};
    use rust_decimal::dec;
    assert_eq!(format!("[{:>5}]", Currency::EUR), "[  EUR]");
    assert_eq!(format!("[{:<3}]", TaxCategory::Export), "[G  ]");
    assert_eq!(format!("[{:>6}]", Percent::new(dec!(7))), "[   7 %]");
}
