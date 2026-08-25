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
use billing::{CashRounding, FixedRateTax, LineAllowanceCharge, TaxCategory};
use proptest::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::dec;

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
/// A VAT rate in (0, 1], with four decimals.
///
/// Zero is excluded on purpose: these generators feed `FixedRateTax::new`, which
/// defaults to category `S`, and BR-S-05 requires a standard-rated rate above
/// zero. A supply taxed at zero is zero-*rated* (`Z`) and has its own constructor,
/// `FixedRateTax::zero_rated`.
fn arb_rate() -> impl Strategy<Value = Decimal> {
    (1u32..=10_000u32).prop_map(|n| Decimal::new(n as i64, 4))
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
        // `into_decimal` never produces more than P digits, so the *exact*
        // constructor must accept every value — no rounding involved.
        prop_assert_eq!(Amount::<5>::checked_from_decimal(a.into_decimal()), Ok(a));
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
        // Zero is included deliberately. A recipient allocated nothing — a tenant
        // who moved in mid-period, a cost centre switched off for the month — is a
        // routine input, and it is the one the old `1..1000` generator could never
        // produce. Only an all-zero set is a genuine error.
        ratios in prop::collection::vec(0u64..1000, 1..16),
    ) {
        match a.allocate(&ratios) {
            Ok(parts) => {
                prop_assert_eq!(parts.len(), ratios.len());
                let sum = Amount::checked_sum(parts.iter().copied()).unwrap();
                prop_assert_eq!(sum, a, "allocate must neither create nor destroy money");
                // A zero ratio is a zero share — never a rounding crumb. The
                // largest-remainder step hands out spare units by remainder, and a
                // zero ratio has none, so it must never be served.
                for (ratio, part) in ratios.iter().zip(&parts) {
                    if *ratio == 0 {
                        prop_assert_eq!(
                            *part,
                            Amount::<5>::ZERO,
                            "a zero ratio must receive nothing"
                        );
                    }
                }
            }
            Err(_) => prop_assert!(
                ratios.iter().all(|r| *r == 0),
                "the only ratios that may be refused are all-zero ones, got {:?}",
                ratios
            ),
        }
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

    #[test]
    fn proportional_split_conserves_the_total_when_the_shares_do_not_sum_to_one(
        total in 0u64..1_000_000_000u64,
        n in 1usize..12,
        scale in 0u32..6,
    ) {
        // The test above builds fractions that sum to *exactly* one, which is the
        // region where the interesting bug cannot occur: with no drift the deficit
        // never exceeds one unit per fraction, so the branch that has to handle a
        // larger one is unreachable from that generator.
        //
        // Real callers do not get exact fractions. `1/n` truncated to nine
        // decimals is what a config file or a spreadsheet hands you, and it sums
        // to slightly under one — which is the case the share-sum tolerance
        // deliberately admits, and therefore the case the split has to honour.
        let share = (Decimal::ONE / Decimal::from(n as u64))
            .round_dp_with_strategy(9, rust_decimal::RoundingStrategy::ToZero);
        let fractions = vec![share; n];
        let total = Decimal::from(total);

        // Whatever the tolerance decides, the guarantee is the same: this
        // function either refuses the shares or splits them exactly. Returning a
        // set of parts that sums to anything other than the total is the one
        // outcome it must never produce — and an `Err` for shares it accepted is
        // what the bug actually looked like.
        if let Ok(parts) = proportional_split(total, &fractions, scale) {
            prop_assert_eq!(parts.len(), n);
            let got: Decimal = parts.iter().sum();
            prop_assert_eq!(
                got,
                total.round_dp_with_strategy(
                    scale,
                    rust_decimal::RoundingStrategy::MidpointAwayFromZero,
                ),
                "accepted shares must split exactly"
            );
            // The correction is spread, not concentrated: the deficit is under two
            // units per fraction, so no part may run further than that from its
            // ideal share.
            let unit = Decimal::new(1, scale);
            for part in &parts {
                let ideal = total * share;
                prop_assert!(
                    (part - ideal).abs() <= unit * Decimal::from(2u64),
                    "part {} strays more than two units from its ideal {}",
                    part,
                    ideal
                );
            }
        }
    }

    #[test]
    fn proportional_split_never_rejects_evenly_truncated_thirds(
        total in 0u64..29_999_999u64,
        scale in 0u32..3,
    ) {
        // Held as a law rather than as a single example: three `0.333333333` shares
        // are inside the documented tolerance, so they must split rather than fail —
        // at every total the
        // tolerance accepts, not merely at the one that was tested.
        let fractions = [Decimal::new(333_333_333, 9); 3];
        let total = Decimal::from(total);
        let parts = proportional_split(total, &fractions, scale)
            .map_err(|e| TestCaseError::fail(format!("rejected total {total}: {e}")))?;
        let got: Decimal = parts.iter().sum();
        prop_assert_eq!(
            got,
            total.round_dp_with_strategy(scale, rust_decimal::RoundingStrategy::MidpointAwayFromZero)
        );
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

    /// The whole EN 16931 totals chain, spelled the way the standard spells it —
    /// with BT-107 as a positive magnitude that BR-CO-13 *subtracts*.
    ///
    /// `documents_are_always_internally_consistent` above checks BR-CO-15 on a
    /// document with neither allowances nor charges, where BT-107 and BT-108 are
    /// both zero and the identity degenerates. This one puts a discount layer and a
    /// levy on every document, so BR-CO-11, BR-CO-12 and BR-CO-13 are each doing
    /// work — and it reads the allowance total through `allowance_total()`, which
    /// is the accessor a consumer emits as the unsigned `cbc:AllowanceTotalAmount`.
    #[test]
    fn the_totals_chain_holds_with_allowances_and_charges_present(
        amounts in prop::collection::vec(1i64..100_000_000, 1..8),
        vat_rate in arb_rate(),
        discount_rate in (0u32..=100).prop_map(|p| Decimal::new(p as i64, 2)),
        levy_raw in 0i64..500_000,
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                LineItem::for_usage(
                    format!("Item {i}"),
                    Quantity::new(Decimal::new(*raw, 5), "kWh"),
                    UnitPrice::new(Decimal::ONE, "EUR/kWh"),
                )
                .build()
                .unwrap()
            })
            .collect();

        let doc = BillingDocument::builder()
            .currency(Currency::EUR)
            .positions(positions)
            .extra_discount(PercentageDiscount::new("Rabatt", discount_rate).unwrap().boxed())
            // A per-unit levy is a BG-21 charge: it feeds BT-108, not BT-110.
            .extra_tax(
                PerUnitLevy::new("Abgabe", Amount::<5>::from_raw_units(levy_raw), "kWh")
                    .unwrap()
                    .boxed(),
            )
            .extra_tax(FixedRateTax::new("MwSt", vat_rate).unwrap().boxed())
            .build()
            .unwrap();

        let bt106 = doc.line_total().unwrap();
        let bt107 = doc.allowance_total().unwrap();
        let bt108 = doc.charge_total().unwrap();
        let bt109 = doc.taxable_total().unwrap();
        let bt110 = doc.vat_total().unwrap();

        // BT-107 is a magnitude, never a negative number.
        prop_assert!(!bt107.is_negative(), "BT-107 must be stated as a magnitude");
        // …and it is exactly the negation of the engine's signed view.
        prop_assert_eq!(bt107, doc.discount_total().checked_neg().unwrap());

        // BR-CO-13: BT-109 = BT-106 − BT-107 + BT-108.
        prop_assert_eq!(
            bt109,
            bt106.checked_sub(bt107).unwrap().checked_add(bt108).unwrap()
        );
        // BR-CO-15: BT-112 = BT-109 + BT-110. This is the identity that fails if a
        // charge is mistaken for VAT, which is why BT-108 and BT-110 are separate.
        prop_assert_eq!(bt109.checked_add(bt110).unwrap(), doc.gross_total());
        // BR-CO-14: BT-110 = Σ BT-117.
        doc.verify_vat_attribution().unwrap();
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
        let sched = RateSchedule::graduated()
            .unit("kWh")
            .currency(Currency::EUR)
            .band(RateBand::up_to(Decimal::from(500), Amount::<5>::from_raw_units(p1)))
            .band(RateBand::over(Decimal::from(500), Amount::<5>::from_raw_units(p2)))
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

    /// The two entry points must agree on **exactly** what is representable.
    ///
    /// `Amount::parse` and `Amount::checked_from_decimal` are documented as the
    /// text and `Decimal` halves of one conversion — *"a value refused as text is
    /// refused as a `Decimal`"*. Nothing enforced that across arbitrary inputs, and
    /// they can disagree in ways no example catches: `Decimal::from_str` accepts an
    /// omitted integer part, so a `parse` that refused `".5"` would reject as text
    /// what it accepts as a `Decimal` — the exact shape a CSV export produces.
    ///
    /// Comparing acceptance rather than the value is what makes this a real check:
    /// it fails whichever of the two is the stricter one.
    #[test]
    fn parse_and_checked_from_decimal_accept_the_same_strings(
        sign in "[+-]?",
        whole in "[0-9]{0,12}",
        sep in "[.,]?",
        frac in "[0-9]{0,8}",
    ) {
        let text = format!("{sign}{whole}{sep}{frac}");
        let from_text = Amount::<5>::parse(&text);

        // `Decimal` only knows the dot, so normalise the separator the same way
        // `parse` does before handing it over.
        let normalised = text.replace(',', ".");
        let Ok(d) = Decimal::from_str_exact(&normalised) else {
            // Not a number to `Decimal` either — `parse` must refuse it too.
            prop_assert!(
                from_text.is_err(),
                "parse accepted {text:?}, which Decimal rejects"
            );
            return Ok(());
        };
        let from_decimal = Amount::<5>::checked_from_decimal(d);

        prop_assert_eq!(
            from_text.is_ok(),
            from_decimal.is_ok(),
            "parse and checked_from_decimal disagree on {:?}: {:?} vs {:?}",
            text, from_text, from_decimal
        );
        if let (Ok(a), Ok(b)) = (from_text, from_decimal) {
            prop_assert_eq!(a, b, "same input, different value: {:?}", text);
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

// ─────────────────────────────────────────────────────────────────────────────
// PEPPOL-EN16931-R120 — one derivation, two precisions
// ─────────────────────────────────────────────────────────────────────────────
//
// `LineItemBuilder::build` and `BillingDocument::reduce_position` both evaluate
//
//     BT-131 = BT-129 × (BT-146 ÷ BT-149) + Σ BG-28 − Σ BG-27
//
// at different precisions. They are meant to share one implementation
// (`line_item::compose_bt131`) precisely because they once drifted: BT-149 and
// BG-27 / BG-28 were added to `build` and not to `reduce_position`, which then
// stopped recognising such a line, fell back to reducing the already-rounded
// five-decimal amount, and rounded **twice** — silently, and by up to a whole
// minor unit.
//
// The law below re-derives the expression independently of both, so any future
// drift fails here rather than in a customer's invoice.

/// Divisors chosen so the quotient is usually non-terminating: that is exactly
/// where rounding once and rounding twice disagree.
fn arb_base_quantity() -> impl Strategy<Value = Option<Decimal>> {
    prop_oneof![
        Just(None),
        Just(Some(Decimal::from(3))),
        Just(Some(Decimal::from(7))),
        Just(Some(Decimal::from(100))),
        Just(Some(Decimal::from(1000))),
    ]
}

/// Six-decimal prices, so `quantity × price` carries a long tail into the
/// division and the two-decimal boundary is approached from every side.
fn arb_long_tail_price() -> impl Strategy<Value = Decimal> {
    (1i64..2_000_000i64).prop_map(|n| Decimal::new(n, 6))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(2048))]

    /// Reducing a line to the interchange scale must round the whole R120
    /// expression **once**, from its exact value — never the stored five-decimal
    /// amount, and never each half by a different route.
    #[test]
    fn line_reduction_rounds_the_r120_expression_exactly_once(
        qty_raw in 1i64..1_000_000i64,
        price in arb_long_tail_price(),
        base in arb_base_quantity(),
        allowance_raw in 0i64..500_000i64,
        charge_raw in 0i64..500_000i64,
        credit in any::<bool>(),
    ) {
        let quantity = Quantity::new(Decimal::new(qty_raw, 3), "pcs");
        let mut unit_price = UnitPrice::new(price, "EUR/pcs");
        if let Some(b) = base {
            unit_price = unit_price.per(b);
        }
        // BT-136 and BT-141, at the engine's precision.
        let allowance = Amount::<5>::from_raw_units(allowance_raw);
        let charge = Amount::<5>::from_raw_units(charge_raw);

        let mut builder = if credit {
            LineItem::credit_for_usage("Ware", quantity.clone(), unit_price.clone())
        } else {
            LineItem::for_usage("Ware", quantity.clone(), unit_price.clone())
        };
        builder = builder
            .line_allowance(LineAllowanceCharge::allowance(allowance, "Rabatt"))
            .line_allowance(LineAllowanceCharge::charge(charge, "Verpackung"));
        let line = builder.build().unwrap();

        let doc = BillingDocument::builder()
            .currency(Currency::EUR)
            .amount_scale(AmountScale::EN16931)
            .positions(vec![line])
            .build()
            .unwrap();
        let reduced = &doc.net_positions()[0];

        // Re-derive R120 here, from the inputs, independently of the engine.
        let exact = quantity.value * price / base.unwrap_or(Decimal::ONE);
        let round2 = |d: Decimal| d.round_dp_with_strategy(
            2, rust_decimal::RoundingStrategy::MidpointAwayFromZero,
        );
        let expected_unsigned =
            round2(exact) - round2(allowance.into_decimal()) + round2(charge.into_decimal());
        let expected = if credit && expected_unsigned > Decimal::ZERO {
            -expected_unsigned
        } else {
            expected_unsigned
        };

        prop_assert_eq!(
            reduced.net_amount.into_decimal(),
            expected,
            "R120 was not rounded once from the exact value \
             (qty={}, price={}, base={:?})",
            quantity.value, price, base,
        );

        // The emitted parts reproduce the emitted total — what a validator checks.
        prop_assert!(reduced.net_amount.fits_scale(2));
        for lac in &reduced.line_allowances {
            prop_assert!(lac.amount.fits_scale(2));
        }
        prop_assert!(doc.fits_amount_scale(2));
    }

    // ── Tax-inclusive amounts ────────────────────────────────────────────────

    /// The gross split is *lossless*: the net and the tax always re-sum to the
    /// price that was quoted, under every rate and every rounding strategy.
    ///
    /// This is only true because the net is defined as the remainder. Rounding
    /// `gross / (1 + rate)` and `gross * rate / (1 + rate)` independently fails
    /// here for roughly half of all inputs, and the symptom on a real invoice is
    /// a cent that appears or vanishes between the shelf price and BT-112.
    #[test]
    fn a_gross_amount_splits_into_net_and_tax_without_loss(
        gross in arb_amount(),
        rate in arb_rate(),
        strategy in arb_strategy(),
    ) {
        let tax = gross.extract_tax(rate, strategy).unwrap();
        let net = gross.net_of_tax(rate, strategy).unwrap();
        prop_assert_eq!(net.checked_add(tax).unwrap(), gross);

        // The extracted tax never exceeds the amount it came out of, and shares
        // its sign — a gross of 119 cannot contain 130 of tax, nor -19.
        prop_assert_eq!(tax.is_negative(), gross.is_negative() && !tax.is_zero());
        prop_assert!(tax.checked_abs().unwrap() <= gross.checked_abs().unwrap());
    }

    /// `TaxBreakdownEntry::from_gross` produces a group a validator accepts:
    /// BT-116 + BT-117 is the quoted gross, and BR-CO-17 holds on the result.
    #[test]
    fn a_breakdown_built_from_a_gross_price_satisfies_br_co_17(
        gross in arb_positive_amount(),
        rate in arb_rate(),
    ) {
        let entry = TaxBreakdownEntry::from_gross(
            TaxCategory::Standard, rate, gross, AmountScale::EN16931,
        ).unwrap();
        prop_assert_eq!(
            entry.taxable_base.checked_add(entry.tax_amount).unwrap(),
            gross
        );
        prop_assert!(entry.validate().is_ok());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Allocation preserves the per-position VAT attribution (BR-S-08)
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// An allocated share satisfies `verify_vat_attribution` whenever the document
    /// it came from did — **BR-S-08** and **BR-CO-14**, per recipient.
    ///
    /// It needs a *multi-group* document to be interesting. Splitting the breakdown
    /// per group while correcting the positions in aggregate happens to agree on a
    /// single VAT group, and lands a minor unit apart for roughly 45 % of recipients
    /// on two — so a single-group generator would pass against a broken split, which
    /// is exactly the way a property test can be strong and still prove nothing.
    #[test]
    fn allocation_preserves_br_s_08_on_a_multi_rate_document(
        full in prop::collection::vec(1i64..50_000_000, 1..5),
        reduced in prop::collection::vec(1i64..50_000_000, 1..5),
        n in 2usize..8,
        prepaid_raw in 0i64..1_000_000,
    ) {
        let position = |i: usize, raw: i64, tag: &str| {
            LineItem::fixed(format!("{tag}-{i}"), Amount::<5>::from_raw_units(raw))
                .tag(tag.to_string())
                .build()
                .unwrap()
        };
        let positions: Vec<LineItem> = full
            .iter()
            .enumerate()
            .map(|(i, r)| position(i, *r, "full"))
            .chain(reduced.iter().enumerate().map(|(i, r)| position(i, *r, "reduced")))
            .collect();

        let doc = BillingDocument::builder()
            .currency(Currency::EUR)
            .positions(positions)
            .extra_tax(FixedRateTax::new("MwSt", dec!(0.19)).unwrap().with_tag("full").boxed())
            .extra_tax(FixedRateTax::new("MwSt", dec!(0.07)).unwrap().with_tag("reduced").boxed())
            .build()
            .unwrap()
            .with_prepaid(Amount::<5>::from_raw_units(prepaid_raw))
            .unwrap();

        // The source satisfies it, so every share must too.
        doc.verify_vat_attribution().unwrap();

        let parts = EqualAllocation::new(n).unwrap().allocate(&doc).unwrap();
        prop_assert_eq!(parts.len(), n);

        for (i, part) in parts.iter().enumerate() {
            prop_assert!(part.validate().is_ok(), "recipient {} invalid: {:?}", i, part.validate());
            prop_assert!(
                part.verify_vat_attribution().is_ok(),
                "recipient {} broke BR-S-08: {:?}",
                i,
                part.verify_vat_attribution()
            );
        }

        // …and nothing leaks or is invented across the split.
        let sum = |f: fn(&BillingDocument) -> Amount<5>| -> Amount<5> {
            parts.iter().map(f).sum()
        };
        prop_assert_eq!(sum(|d| d.net_total()), doc.net_total());
        prop_assert_eq!(sum(|d| d.tax_total()), doc.tax_total());
        prop_assert_eq!(sum(|d| d.gross_total()), doc.gross_total());
        prop_assert_eq!(sum(|d| d.prepaid()), doc.prepaid());
        for (g, src) in doc.tax_breakdown().iter().enumerate() {
            let base: Amount<5> = parts.iter().map(|d| d.tax_breakdown()[g].taxable_base).sum();
            let tax: Amount<5> = parts.iter().map(|d| d.tax_breakdown()[g].tax_amount).sum();
            prop_assert_eq!(base, src.taxable_base);
            prop_assert_eq!(tax, src.tax_amount);
        }
    }

    /// Every position is split exactly, not merely every total — which is what the
    /// group-level guarantee above is built on.
    #[test]
    fn allocation_splits_each_position_exactly(
        amounts in prop::collection::vec(-50_000_000i64..50_000_000, 1..8),
        n in 2usize..8,
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                let amount = Amount::<5>::from_raw_units(*raw);
                if *raw < 0 {
                    LineItem::credit_fixed(format!("C{i}"), amount).build().unwrap()
                } else {
                    LineItem::fixed(format!("D{i}"), amount).build().unwrap()
                }
            })
            .collect();
        let doc = BillingDocument::builder()
            .currency(Currency::EUR)
            .positions(positions)
            .build()
            .unwrap();

        let parts = EqualAllocation::new(n).unwrap().allocate(&doc).unwrap();
        for (p, source) in doc.net_positions().iter().enumerate() {
            let rebuilt: Amount<5> = parts.iter().map(|d| d.net_positions()[p].net_amount).sum();
            prop_assert_eq!(
                rebuilt, source.net_amount,
                "position {} does not reassemble from its shares", p
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Percentile billing — every figure billed is a rate that actually occurred
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Nearest-rank never interpolates, so the answer is always one of the samples.
    /// The other common percentile definition averages the two neighbours, which
    /// would bill a throughput figure the link never carried.
    #[test]
    fn a_percentile_is_always_a_sample_that_occurred(
        values in prop::collection::vec(0i64..1_000_000, 1..64),
        percent in 0u32..=100,
    ) {
        use billing::aggregation::{PercentileAggregator, UsageAggregator};
        let agg = PercentileAggregator::new(
            Decimal::from(percent),
            |v: &i64| Decimal::from(*v),
        ).unwrap();
        let got = agg.aggregate(&values).unwrap();
        prop_assert!(values.iter().any(|v| Decimal::from(*v) == got));
    }

    /// A higher percentile never bills less, and 100 % is the peak.
    #[test]
    fn percentiles_are_monotonic_and_top_out_at_the_maximum(
        values in prop::collection::vec(0i64..1_000_000, 1..64),
        a in 0u32..=100,
        b in 0u32..=100,
    ) {
        use billing::aggregation::{MaxAggregator, PercentileAggregator, UsageAggregator};
        let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
        let at = |p: u32| PercentileAggregator::new(Decimal::from(p), |v: &i64| Decimal::from(*v))
            .unwrap()
            .aggregate(&values)
            .unwrap();
        prop_assert!(at(lo) <= at(hi));
        prop_assert_eq!(
            at(100),
            MaxAggregator::new(|v: &i64| Decimal::from(*v)).aggregate(&values).unwrap()
        );
    }

    /// "Discard the top 5 %, bill the highest one left" is the settlement rule every
    /// transit contract states, and `ceil(0.95 × n)` is that rule as an index.
    #[test]
    fn p95_is_the_highest_sample_left_after_the_top_five_percent_is_discarded(
        values in prop::collection::vec(0i64..1_000_000, 1..200),
    ) {
        use billing::aggregation::{PercentileAggregator, UsageAggregator};
        let got = PercentileAggregator::p95(|v: &i64| Decimal::from(*v))
            .aggregate(&values)
            .unwrap();

        let mut sorted = values.clone();
        sorted.sort_unstable();
        // `n - ceil(0.95n)` is what "the top 5 %" rounds down to.
        let keep = (values.len() * 95).div_ceil(100).max(1);
        sorted.truncate(keep);
        prop_assert_eq!(got, Decimal::from(*sorted.last().unwrap()));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dynamic pricing rounds once over the whole sequence
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// An interval is not a leaf — it never becomes its own invoice line — so the
    /// products are accumulated exactly and reduced **once**. Rounding each one
    /// first drifts in a single direction, because commercial rounding is not
    /// unbiased on a repeated fractional part.
    #[test]
    fn dynamic_pricing_is_the_single_rounding_of_the_exact_sum(
        intervals in prop::collection::vec((1i64..1_000_000i64, 1i64..100_000i64), 1..40),
    ) {
        let mut builder = DynamicPricing::builder().unit("kWh").currency(Currency::EUR);
        let mut exact = Decimal::ZERO;
        for (qty_raw, price_raw) in &intervals {
            // Quantities carry three decimals, prices five — the shape a spot
            // sequence actually has, and enough for the product to exceed five.
            let qty = Decimal::new(*qty_raw, 3);
            let price = Amount::<5>::from_raw_units(*price_raw);
            exact += qty * price.into_decimal();
            builder = builder.interval(qty, price);
        }
        let net = builder.build().unwrap().calculate().unwrap().net_amount;
        prop_assert_eq!(
            net,
            Amount::<5>::from_decimal_rounded(exact, RoundingStrategy::MidpointAwayFromZero).unwrap()
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Late payment interest
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// Simple interest, never compounded — art. 2(6) of Directive 2011/7/EU says
    /// so and §289 BGB forbids the alternative outright. Doubling the days doubles
    /// the interest, up to the one rounding at the end.
    #[test]
    fn late_payment_interest_is_linear_in_the_days(
        overdue in arb_positive_amount(),
        percent in 1u32..=100_000,
        days in 1u32..1_000,
    ) {
        let penalty = LatePaymentPenalty::new(Decimal::new(percent as i64, 4)).unwrap();
        let once = penalty.interest_on(overdue, days).unwrap();
        let twice = penalty.interest_on(overdue, days * 2).unwrap();
        let doubled = once.checked_add(once).unwrap();
        // Both sides round once at five decimals, so they can differ by one unit.
        let gap = twice.checked_sub(doubled).unwrap().abs();
        prop_assert!(gap <= Amount::<5>::from_raw_units(1), "gap {gap}");
    }

    /// The whole point of the type: an invoice's own totals never move, and paying
    /// late costs strictly more than paying on the due date.
    #[test]
    fn a_penalty_only_ever_adds_to_what_is_payable(
        overdue in arb_positive_amount(),
        percent in 1u32..=100_000,
        days in 0u32..1_000,
        fee_raw in 0i64..10_000_000,
    ) {
        let penalty = LatePaymentPenalty::new(Decimal::new(percent as i64, 4))
            .unwrap()
            .with_recovery_fee(Amount::<5>::from_raw_units(fee_raw))
            .unwrap();
        let payable = penalty.payable_on(overdue, days).unwrap();
        prop_assert!(payable >= overdue);
        if days == 0 {
            prop_assert_eq!(payable, overdue);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Every position the engine generates is emittable as an invoice line
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// The crate states this as a design invariant. The arithmetic half of it is
    /// `PEPPOL-EN16931-R120` — **fatal** — which requires BT-131 to equal
    /// `BT-129 × (BT-146 ÷ BT-149)` plus the BG-27 / BG-28 leaves, within ±0.02.
    ///
    /// Every generator in the crate is exercised here, because the one that fails it
    /// is the one whose price is *derived* rather than quoted.
    #[test]
    fn every_generated_position_satisfies_r120(
        qty_raw in 1i64..100_000_000i64,
        price_raw in 1i64..10_000_000i64,
        block_raw in 1i64..1_000_000i64,
        intervals in prop::collection::vec((1i64..1_000_000i64, 1i64..100_000i64), 1..40),
    ) {
        let qty = Decimal::new(qty_raw, 3);
        let price = Amount::<5>::from_raw_units(price_raw);
        let check = |item: &LineItem| item.check_price_consistency();

        // Graduated, volume and block schedules.
        for schedule in [
            RateSchedule::graduated()
                .unit("kWh").currency(Currency::EUR)
                .band(RateBand::up_to(Decimal::new(qty_raw / 2 + 1, 3), price))
                .band(RateBand::over(Decimal::new(qty_raw / 2 + 1, 3), price))
                .build().unwrap(),
            RateSchedule::volume()
                .unit("kWh").currency(Currency::EUR)
                .band(RateBand::over(Decimal::ZERO, price))
                .build().unwrap(),
            RateSchedule::block()
                .unit("GB").currency(Currency::EUR)
                .band(RateBand::block(Decimal::new(block_raw, 3), price))
                .build().unwrap(),
        ] {
            for item in schedule.split(qty).unwrap() {
                prop_assert!(check(&item).is_ok(), "{:?}: {:?}", item.description, check(&item));
            }
        }

        // Capacity — a flat fee, so R120 holds as `1 × amount`.
        let capacity = RateSchedule::capacity()
            .unit("Mbps").currency(Currency::EUR)
            .band(RateBand::over(Decimal::ZERO, price))
            .build().unwrap();
        prop_assert!(check(&capacity.apply_peak(qty).unwrap()).is_ok());

        // Time-of-use bands.
        let tou = TimeOfUsePricing::builder()
            .unit("kWh").currency(Currency::EUR)
            .band(TimeBand::new("peak", price))
            .build().unwrap();
        for item in tou.calculate(&[("peak", qty)]).unwrap() {
            prop_assert!(check(&item).is_ok());
        }

        // Dynamic pricing — the aggregate line, whose BT-146 is a weighted average
        // with a long non-terminating expansion. This is the one that can fail.
        let mut b = DynamicPricing::builder().unit("kWh").currency(Currency::EUR);
        for (q, p) in &intervals {
            b = b.interval(Decimal::new(*q, 3), Amount::<5>::from_raw_units(*p));
        }
        let dynamic = b.build().unwrap().calculate().unwrap();
        prop_assert!(check(&dynamic).is_ok(), "dynamic: {:?}", check(&dynamic));

        // …and it still holds once the document is reduced to EN 16931's two
        // decimals, which is where the amount moves and the price does not.
        let doc = BillingDocument::builder()
            .currency(Currency::EUR)
            .amount_scale(AmountScale::EN16931)
            .positions(vec![dynamic])
            .build().unwrap();
        prop_assert_eq!(doc.price_inconsistent_positions().count(), 0);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// A gross minimum charge reaches its target
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// `minimum_charge_gross` tops the document up to `minimum_gross` at the rate
    /// the contract names. The net top-up is one rounding of the exact quotient, so
    /// the gross that comes back lands within a rounding unit of the target — never
    /// the 5 % short that deriving it by hand at the wrong rate produced.
    #[test]
    fn a_gross_minimum_lands_on_its_target(
        base_raw in 1i64..1_000_000_000i64,
        extra_raw in 1i64..1_000_000_000i64,
        rate in arb_rate(),
    ) {
        use billing::{TaxCategory, minimum_charge_gross};

        let base = Amount::<5>::from_raw_units(base_raw);
        let vat = LineVat::new(TaxCategory::Standard, rate).unwrap();
        let layers = || -> Vec<Box<dyn billing::TaxLayer>> {
            vec![FixedRateTax::new("VAT", rate).unwrap().boxed()]
        };
        let positions = vec![LineItem::fixed("Verbrauch", base).build().unwrap()];
        let meta = || DocumentMeta { currency: Currency::EUR, ..Default::default() };

        let provisional =
            BillingDocument::from_positions(meta(), positions.clone(), layers(), vec![]).unwrap();
        // A minimum strictly above the current gross, so a top-up is always due.
        let minimum = provisional
            .gross_total()
            .checked_add(Amount::<5>::from_raw_units(extra_raw))
            .unwrap();

        let top_up = minimum_charge_gross(&provisional, minimum, vat, "Mindestbetrag",
                                          Currency::EUR).unwrap().unwrap();
        prop_assert!(!top_up.net_amount.is_negative());

        let mut positions = positions;
        positions.push(top_up);
        let doc = BillingDocument::from_positions(meta(), positions, layers(), vec![]).unwrap();

        // Two roundings stand between the exact quotient and BT-112 — the net top-up
        // at five decimals and the tax on it — so the target is met to within them.
        let gap = doc.gross_total().checked_sub(minimum).unwrap().abs();
        prop_assert!(gap <= Amount::<5>::from_raw_units(2), "gap {gap}");
        doc.verify_vat_attribution().unwrap();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// serde round trip
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "serde")]
proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `from_str(to_string(doc)) == doc`, for a document carrying every field the
    /// header and the settlement chain have.
    ///
    /// Worth stating as a law rather than as a handful of examples, because almost
    /// every type in this crate deserialises through `#[serde(try_from = …)]` and
    /// re-runs its constructor. That is what keeps an invalid document out of the
    /// process — and it is also what makes a field that serialises but is never read
    /// back look exactly like a field that works. A `Vec` without `#[serde(default)]`
    /// fails loudly; one *with* it silently comes back empty if the writer spelled
    /// the key differently.
    #[test]
    fn a_document_survives_a_json_round_trip(
        amounts in prop::collection::vec(1i64..100_000_000, 1..6),
        rate in arb_rate(),
        prepaid_raw in 0i64..1_000_000,
        note in "[a-zA-Z ]{0,24}",
        preceding in prop::collection::vec("[A-Z]{2}-[0-9]{4}", 0..3),
    ) {
        let positions: Vec<LineItem> = amounts
            .iter()
            .enumerate()
            .map(|(i, raw)| {
                LineItem::fixed(format!("Item {i}"), Amount::<5>::from_raw_units(*raw))
                    .period("2026-06-01", "2026-06-30")
                    .meta("malo_id", "52435677816")
                    .build()
                    .unwrap()
            })
            .collect();

        let meta = DocumentMeta {
            invoice_number: "INV-2026-001".into(),
            currency: Currency::EUR,
            period: Some(Period::new("2026-06-01", "2026-06-30")),
            issue_date: Some("2026-07-01".into()),
            due_date: Some("2026-07-31".into()),
            payment_terms: Some(
                PaymentTerms::text("Zahlbar in 30 Tagen.")
                    .with_discount(EarlyPaymentDiscount::new(10, dec!(2)).unwrap()),
            ),
            issuer_id: Some(PartyIdentifier::scheme("0088", "4004404000018")),
            recipient_id: Some(PartyIdentifier::new("KND-4711")),
            preceding: preceding
                .iter()
                .map(|n| PrecedingInvoice::new(n).unwrap().with_issue_date("2026-05-31"))
                .collect(),
            notes: vec![Note::coded("AAI", note)],
            ..Default::default()
        };

        let taxes: Vec<Box<dyn TaxLayer>> =
            vec![FixedRateTax::new("MwSt", rate).unwrap().boxed()];
        let doc = BillingDocument::from_positions(meta, positions, taxes, vec![]).unwrap();
        // BT-111 is BT-110 converted, so `PEPPOL-EN16931-R055` (shared sign) holds.
        let bt111 = VatAccountingCurrency::converted(
            Currency::new("PLN").unwrap(),
            doc.vat_total().unwrap(),
            dec!(4.30),
            AmountScale::EN16931,
        ).unwrap();
        let doc = doc
            .with_prepaid(Amount::<5>::from_raw_units(prepaid_raw))
            .unwrap()
            .with_vat_accounting_currency(bt111)
            .unwrap();

        let json = serde_json::to_string(&doc).unwrap();
        // Non-vacuity: the field has to be *on the wire* for the round trip to mean
        // anything. Comparing two values that both lost it would still pass.
        for reference in &preceding {
            prop_assert!(json.contains(reference.as_str()), "BT-25 {reference} missing from {json}");
        }
        // `PaymentTerms` persists as its two structured halves, not as the BT-20
        // rendering — `Display`/`FromStr` are the wire syntax, serde is storage.
        prop_assert!(json.contains("Zahlbar in 30 Tagen."));
        prop_assert!(json.contains("\"days\":10"));
        prop_assert!(json.contains("4004404000018"));

        let back: BillingDocument = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(&back, &doc);

        // The header fields specifically, because `PartialEq` on the whole document
        // would still pass if *both* sides lost the same field to a `default`.
        prop_assert_eq!(back.meta.preceding.len(), doc.meta.preceding.len());
        prop_assert_eq!(back.meta.payment_terms.as_ref(), doc.meta.payment_terms.as_ref());
        prop_assert_eq!(back.meta.issuer_id.as_ref(), doc.meta.issuer_id.as_ref());
        prop_assert_eq!(back.meta.notes.len(), 1);
        prop_assert_eq!(back.prepaid(), doc.prepaid());

        // And it is validated on the way in, not merely reconstructed.
        back.validate().unwrap();
    }
}
