//! Property-based tests for the monetary invariants.
//!
//! Hand-written tests check the cases we thought of. These check the ones we did
//! not: every test below states a law that must hold for *all* inputs in range,
//! and `proptest` searches for a counterexample, shrinking any it finds to a
//! minimal case.
//!
//! The laws here are the ones a billing engine is actually judged on — money is
//! neither created nor destroyed by splitting it, rounding is idempotent and
//! bounded, and parsing round-trips exactly.
//!
//! Counterexamples are persisted to `proptest-regressions/` and replayed on every
//! subsequent run, so a failure found once becomes a permanent regression test.

use billing::prelude::*;
use billing::{CashRounding, FixedRateTax, TaxCategory};
use proptest::prelude::*;
use rust_decimal::Decimal;

// ── Strategies ───────────────────────────────────────────────────────────────

/// Raw `i64` values well inside `Amount<5>`'s range, so that sums of a handful of
/// them cannot overflow and the properties test real behaviour rather than
/// saturation.
fn arb_raw() -> impl Strategy<Value = i64> {
    -1_000_000_000_000i64..1_000_000_000_000i64
}

fn arb_amount() -> impl Strategy<Value = Amount<5>> {
    arb_raw().prop_map(Amount::<5>::from_raw_units)
}

/// Non-negative amounts — the common case for prices and bases.
fn arb_positive_amount() -> impl Strategy<Value = Amount<5>> {
    (0i64..1_000_000_000_000i64).prop_map(Amount::<5>::from_raw_units)
}

/// A VAT rate in [0, 1] with at most 4 decimal places.
fn arb_rate() -> impl Strategy<Value = Decimal> {
    (0u32..=10_000u32).prop_map(|n| Decimal::new(n as i64, 4))
}

fn arb_strategy() -> impl Strategy<Value = RoundingStrategy> {
    prop_oneof![
        Just(RoundingStrategy::MidpointAwayFromZero),
        Just(RoundingStrategy::MidpointToEven),
        Just(RoundingStrategy::Ceiling),
        Just(RoundingStrategy::Floor),
        Just(RoundingStrategy::Truncate),
    ]
}

proptest! {
    // ── Amount: algebraic laws ───────────────────────────────────────────────

    #[test]
    fn parse_display_roundtrips_exactly(raw in any::<i64>()) {
        let a = Amount::<5>::from_raw_units(raw);
        let reparsed = Amount::<5>::parse(&a.to_string()).unwrap();
        prop_assert_eq!(a, reparsed);
    }

    #[test]
    fn decimal_conversion_roundtrips_exactly(raw in any::<i64>()) {
        let a = Amount::<5>::from_raw_units(raw);
        prop_assert_eq!(Amount::<5>::from_decimal(a.into_decimal()), Some(a));
    }

    #[test]
    fn addition_is_commutative(a in arb_amount(), b in arb_amount()) {
        prop_assert_eq!(a.checked_add(b).unwrap(), b.checked_add(a).unwrap());
    }

    #[test]
    fn addition_is_associative(a in arb_amount(), b in arb_amount(), c in arb_amount()) {
        let left  = a.checked_add(b).unwrap().checked_add(c).unwrap();
        let right = a.checked_add(b.checked_add(c).unwrap()).unwrap();
        prop_assert_eq!(left, right);
    }

    #[test]
    fn sub_then_add_is_identity(a in arb_amount(), b in arb_amount()) {
        prop_assert_eq!(a.checked_sub(b).unwrap().checked_add(b).unwrap(), a);
    }

    #[test]
    fn checked_ops_never_panic(a in any::<i64>(), b in any::<i64>(), qty in -1_000_000i64..1_000_000) {
        // The whole point of the `checked_*` family: an Err is fine, a panic is not.
        // `any::<i64>()` includes i64::MIN, the historical panic source.
        let (a, b) = (Amount::<5>::from_raw_units(a), Amount::<5>::from_raw_units(b));
        let _ = a.checked_add(b);
        let _ = a.checked_sub(b);
        let _ = a.checked_neg();
        let _ = a.checked_abs();
        let _ = a.checked_mul_qty(Decimal::from(qty));
        let _ = a.checked_div(Decimal::from(qty));
        let _ = a.checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero);
    }

    // ── Amount: conservation of money ────────────────────────────────────────

    #[test]
    fn distribute_conserves_the_total(a in arb_amount(), n in 1usize..64) {
        let parts = a.distribute(n).unwrap();
        prop_assert_eq!(parts.len(), n);
        let sum = Amount::checked_sum(parts.iter().copied()).unwrap();
        prop_assert_eq!(sum, a, "distribute must neither create nor destroy money");
    }

    #[test]
    fn distribute_parts_differ_by_at_most_one_unit(a in arb_amount(), n in 1usize..64) {
        let parts = a.distribute(n).unwrap();
        let min = parts.iter().map(|p| p.to_raw()).min().unwrap();
        let max = parts.iter().map(|p| p.to_raw()).max().unwrap();
        prop_assert!(max - min <= 1, "parts spread too far: {} vs {}", min, max);
    }

    #[test]
    fn allocate_conserves_the_total(
        a in arb_amount(),
        ratios in prop::collection::vec(1u64..1000, 1..16),
    ) {
        let parts = a.allocate(&ratios).unwrap();
        prop_assert_eq!(parts.len(), ratios.len());
        let sum = Amount::checked_sum(parts.iter().copied()).unwrap();
        prop_assert_eq!(sum, a, "allocate must neither create nor destroy money");
    }

    #[test]
    fn allocate_is_monotonic_in_the_ratios(a in arb_positive_amount()) {
        // A larger ratio never yields a smaller share of a non-negative amount.
        let parts = a.allocate(&[1, 2, 3]).unwrap();
        prop_assert!(parts[0] <= parts[1]);
        prop_assert!(parts[1] <= parts[2]);
    }

    #[test]
    fn proportional_split_conserves_the_total(
        total in 0u64..1_000_000_000,
        weights in prop::collection::vec(1u32..100, 1..12),
    ) {
        // Build fractions that sum to exactly 1 by construction.
        let sum: u32 = weights.iter().sum();
        let mut fractions: Vec<Decimal> = weights
            .iter()
            .map(|w| Decimal::from(*w) / Decimal::from(sum))
            .collect();
        // Force an exact sum of 1 despite division rounding.
        let drift: Decimal = fractions.iter().sum::<Decimal>() - Decimal::ONE;
        let last = fractions.len() - 1;
        fractions[last] -= drift;

        let total = Decimal::from(total);
        let parts = proportional_split(total, &fractions, 3).unwrap();
        let got: Decimal = parts.iter().sum();
        prop_assert_eq!(got, total, "Hamilton split must sum to the rounded total");
    }

    // ── Rounding laws ────────────────────────────────────────────────────────

    #[test]
    fn round_to_increment_is_idempotent(
        a in arb_amount(),
        inc in 1i64..100_000,
        strat in arb_strategy(),
    ) {
        let inc = Amount::<5>::from_raw_units(inc);
        let once = a.round_to_increment(inc, strat).unwrap();
        let twice = once.round_to_increment(inc, strat).unwrap();
        prop_assert_eq!(once, twice, "rounding an already-rounded value must not move it");
    }

    #[test]
    fn round_to_increment_lands_on_a_multiple(
        a in arb_amount(),
        inc in 1i64..100_000,
        strat in arb_strategy(),
    ) {
        let inc_amt = Amount::<5>::from_raw_units(inc);
        let r = a.round_to_increment(inc_amt, strat).unwrap();
        prop_assert_eq!(r.to_raw() % inc, 0, "{} is not a multiple of {}", r, inc_amt);
    }

    #[test]
    fn round_to_increment_stays_within_one_increment(
        a in arb_amount(),
        inc in 1i64..100_000,
        strat in arb_strategy(),
    ) {
        let inc_amt = Amount::<5>::from_raw_units(inc);
        let r = a.round_to_increment(inc_amt, strat).unwrap();
        prop_assert!(
            (r.to_raw() - a.to_raw()).abs() < inc,
            "rounding moved {} to {}, further than one increment {}",
            a, r, inc_amt
        );
    }

    #[test]
    fn cash_rounding_difference_reconstructs_the_rounded_amount(
        a in arb_amount(),
        inc in 1i64..100_000,
        strat in arb_strategy(),
    ) {
        let rule = CashRounding::new(Amount::<5>::from_raw_units(inc), strat).unwrap();
        let rounded = rule.round(a).unwrap();
        let diff = rule.difference(a).unwrap();
        prop_assert_eq!(a.checked_add(diff).unwrap(), rounded);
    }

    // ── Document invariants ──────────────────────────────────────────────────

    #[test]
    fn documents_are_always_internally_consistent(
        amounts in prop::collection::vec(arb_raw(), 0..12),
        rate in arb_rate(),
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                LineItem::fixed(format!("Item {i}"), Amount::<5>::from_raw_units(*raw))
                    .build()
                    .unwrap()
            })
            .collect();
        let taxes: Vec<Box<dyn TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", rate).unwrap())];
        let doc = BillingDocument::from_positions(
            DocumentMeta { currency: Currency::EUR, ..Default::default() },
            positions, taxes, vec![],
        ).unwrap();

        prop_assert!(doc.validate().is_ok(), "constructed document failed validation");
        // BR-CO-15: gross = net + tax.
        prop_assert_eq!(
            doc.net_total().checked_add(doc.tax_total()).unwrap(),
            doc.gross_total()
        );
    }

    #[test]
    fn allocation_conserves_every_total_and_keeps_documents_valid(
        amounts in prop::collection::vec(1i64..100_000_000, 1..8),
        rate in arb_rate(),
        n in 1usize..9,
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                LineItem::fixed(format!("Item {i}"), Amount::<5>::from_raw_units(*raw))
                    .build()
                    .unwrap()
            })
            .collect();
        let taxes: Vec<Box<dyn TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", rate).unwrap())];
        let doc = BillingDocument::from_positions(
            DocumentMeta { currency: Currency::EUR, ..Default::default() },
            positions, taxes, vec![],
        ).unwrap();

        let docs = EqualAllocation::new(n).unwrap().allocate(&doc).unwrap();

        // Every recipient document is self-consistent...
        for (i, d) in docs.iter().enumerate() {
            prop_assert!(d.validate().is_ok(), "recipient {} invalid: {:?}", i, d.validate());
        }
        // ...and nothing leaks or is invented across the split.
        let net: Amount<5> = docs.iter().map(|d| d.net_total()).sum();
        let tax: Amount<5> = docs.iter().map(|d| d.tax_total()).sum();
        let gross: Amount<5> = docs.iter().map(|d| d.gross_total()).sum();
        prop_assert_eq!(net, doc.net_total());
        prop_assert_eq!(tax, doc.tax_total());
        prop_assert_eq!(gross, doc.gross_total());

        // The VAT breakdown splits exactly too.
        for (idx, src) in doc.tax_breakdown().iter().enumerate() {
            let base: Amount<5> = docs.iter().map(|d| d.tax_breakdown()[idx].taxable_base).sum();
            let amt: Amount<5> = docs.iter().map(|d| d.tax_breakdown()[idx].tax_amount).sum();
            prop_assert_eq!(base, src.taxable_base);
            prop_assert_eq!(amt, src.tax_amount);
        }
    }

    #[test]
    fn reversal_is_an_involution_and_nets_to_zero(
        amounts in prop::collection::vec(arb_raw(), 1..8),
        rate in arb_rate(),
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                LineItem::fixed(format!("Item {i}"), Amount::<5>::from_raw_units(*raw))
                    .build()
                    .unwrap()
            })
            .collect();
        let taxes: Vec<Box<dyn TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", rate).unwrap())];
        let doc = BillingDocument::from_positions(
            DocumentMeta { currency: Currency::EUR, ..Default::default() },
            positions, taxes, vec![],
        ).unwrap();

        let credit = doc.reverse(DocumentMeta::default()).unwrap();
        prop_assert!(credit.validate().is_ok());

        // An invoice plus its credit note settles to nothing.
        prop_assert_eq!(
            doc.gross_total().checked_add(credit.gross_total()).unwrap(),
            Amount::<5>::ZERO
        );
        // Reversing twice returns the original figures.
        let back = credit.reverse(DocumentMeta::default()).unwrap();
        prop_assert_eq!(back.net_total(), doc.net_total());
        prop_assert_eq!(back.gross_total(), doc.gross_total());
    }

    #[test]
    fn amount_due_follows_br_co_16(
        gross_raw in 0i64..1_000_000_000,
        prepaid_raw in 0i64..1_000_000_000,
        inc in 1i64..100_000,
    ) {
        let doc = BillingDocument::from_positions(
            DocumentMeta { currency: Currency::EUR, ..Default::default() },
            vec![LineItem::fixed("Item", Amount::<5>::from_raw_units(gross_raw))
                .build().unwrap()],
            vec![], vec![],
        ).unwrap();

        let prepaid = Amount::<5>::from_raw_units(prepaid_raw);
        let rule = CashRounding::new(
            Amount::<5>::from_raw_units(inc),
            RoundingStrategy::MidpointAwayFromZero,
        ).unwrap();
        let doc = doc.with_prepaid(prepaid).unwrap().with_cash_rounding(rule).unwrap();

        // BT-115 = BT-112 − BT-113 + BT-114, exactly.
        let expected = doc.gross_total()
            .checked_sub(doc.prepaid()).unwrap()
            .checked_add(doc.rounding()).unwrap();
        prop_assert_eq!(doc.amount_due().unwrap(), expected);

        // Prepayments must not disturb the taxable base.
        prop_assert_eq!(doc.gross_total(), Amount::<5>::from_raw_units(gross_raw));
    }

    // ── Schedule invariants ──────────────────────────────────────────────────

    #[test]
    fn graduated_split_bills_every_unit_exactly_once(
        qty in 0u64..10_000_000,
        p1 in 1i64..500_000,
        p2 in 1i64..500_000,
    ) {
        let sched = TariffSchedule::graduated()
            .unit("kWh")
            .currency(Currency::EUR)
            .band(TariffBand::up_to(Decimal::from(500), Amount::<5>::from_raw_units(p1)))
            .band(TariffBand::over(Decimal::from(500), Amount::<5>::from_raw_units(p2)))
            .build()
            .unwrap();

        let qty = Decimal::from(qty);
        let items = sched.split(qty).unwrap();
        // The tiers partition the quantity: no unit billed twice, none dropped.
        let billed: Decimal = items.iter().filter_map(|i| i.quantity_value()).sum();
        prop_assert_eq!(billed, qty);
    }

    // ── Tax invariants ───────────────────────────────────────────────────────

    #[test]
    fn vat_breakdown_matches_the_tax_it_reports(
        amounts in prop::collection::vec(0i64..100_000_000, 1..8),
        rate in arb_rate(),
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                LineItem::fixed(format!("Item {i}"), Amount::<5>::from_raw_units(*raw))
                    .build()
                    .unwrap()
            })
            .collect();
        let taxes: Vec<Box<dyn TaxLayer>> =
            vec![Box::new(FixedRateTax::new("VAT", rate).unwrap())];
        let doc = BillingDocument::from_positions(
            DocumentMeta { currency: Currency::EUR, ..Default::default() },
            positions, taxes, vec![],
        ).unwrap();

        // A single VAT layer produces exactly one breakdown line, whose base is the
        // net total and whose tax equals the document's tax total.
        prop_assert_eq!(doc.tax_breakdown().len(), 1);
        let entry = &doc.tax_breakdown()[0];
        prop_assert_eq!(entry.category, TaxCategory::Standard);
        prop_assert_eq!(entry.taxable_base, doc.net_total());
        prop_assert_eq!(entry.tax_amount, doc.tax_total());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parser robustness
//
// `Amount::parse` is the crate's only string parser and its only untrusted-input
// surface. These stand in for a fuzz target: `proptest` shrinks counterexamples
// and runs on stable, whereas `cargo-fuzz` needs nightly and finds the same
// class of defect here (this crate has no `unsafe`, so the reachable bugs are
// panics and logic errors, not memory unsafety).
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn parse_never_panics_on_arbitrary_input(s in ".*") {
        // Any string at all: an Err is fine, an abort is not.
        let _ = Amount::<5>::parse(&s);
        let _ = Amount::<0>::parse(&s);
        let _ = Amount::<18>::parse(&s);
    }

    #[test]
    fn parse_never_panics_on_numeric_looking_input(
        sign in "[+-]?",
        whole in "[0-9]{0,25}",
        sep in "[.,]?",
        frac in "[0-9]{0,25}",
    ) {
        let s = format!("{sign}{whole}{sep}{frac}");
        let _ = Amount::<5>::parse(&s);
    }

    #[test]
    fn parse_accepts_what_display_produces_for_every_precision(raw in any::<i64>()) {
        // Display/parse must round-trip at every supported precision, including the
        // extremes where the i128 intermediate matters.
        prop_assert_eq!(
            Amount::<0>::parse(&Amount::<0>::from_raw_units(raw).to_string()).unwrap().to_raw(),
            raw
        );
        prop_assert_eq!(
            Amount::<2>::parse(&Amount::<2>::from_raw_units(raw).to_string()).unwrap().to_raw(),
            raw
        );
        prop_assert_eq!(
            Amount::<18>::parse(&Amount::<18>::from_raw_units(raw).to_string()).unwrap().to_raw(),
            raw
        );
    }

    #[test]
    fn comma_and_dot_separators_are_equivalent(
        whole in "[0-9]{1,10}",
        frac in "[0-9]{1,5}",
    ) {
        let dot = Amount::<5>::parse(&format!("{whole}.{frac}"));
        let comma = Amount::<5>::parse(&format!("{whole},{frac}"));
        prop_assert_eq!(dot.is_ok(), comma.is_ok());
        if let (Ok(a), Ok(b)) = (dot, comma) {
            prop_assert_eq!(a, b);
        }
    }

    #[test]
    fn parse_rejects_precision_it_cannot_represent(
        whole in "[0-9]{1,8}",
        frac in "[0-9]{6,12}",
    ) {
        // A 6th+ fractional digit is representable at P=5 only if it is zero.
        let s = format!("{whole}.{frac}");
        let excess_is_all_zero = frac[5..].bytes().all(|b| b == b'0');
        prop_assert_eq!(Amount::<5>::parse(&s).is_ok(), excess_is_all_zero);
    }

    // ── Formatting ───────────────────────────────────────────────────────────

    #[test]
    fn display_honours_width_and_never_truncates(raw in arb_raw(), width in 0usize..40) {
        let a = Amount::<5>::from_raw_units(raw);
        let plain = a.to_string();
        let padded = format!("{a:>width$}");
        // Padding may extend, never shorten or mangle.
        prop_assert!(padded.chars().count() >= plain.chars().count());
        prop_assert!(padded.chars().count() >= width || plain.chars().count() >= width);
        prop_assert!(padded.trim_start().starts_with(&plain));
        // And it matches what the equivalent string formatting would produce.
        prop_assert_eq!(padded, format!("{plain:>width$}"));
    }
}
