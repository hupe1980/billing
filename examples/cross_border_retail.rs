//! Cross-border retail invoice — the three things a euro price list does not tell
//! you, and how EN 16931 wants each of them stated.
//!
//! A Polish shop sells to a business customer. Its catalogue is quoted **including
//! VAT**, because consumer price marking requires it. The invoice is issued in EUR,
//! but Polish VAT is reported in złoty. And the terms offer 2 % for payment within
//! ten days.
//!
//! Three separate problems, none of which is "add up the lines":
//!
//! 1. **Gross → net.** EN 16931 is uncompromisingly net — BT-131 per line, BT-116
//!    per category, BT-109 for the document. The catalogue is not. The conversion is
//!    the VAT fraction `r/(1+r)`, not `× r` — and the invoice still will not total
//!    the sum of the shelf prices, for a reason worth understanding rather than
//!    patching over. See the end of `main`.
//! 2. **VAT in two currencies.** Art. 230 of the VAT Directive: any invoice
//!    currency, but the VAT amount also in the currency the tax is due in. That is
//!    BT-6 + BT-111, and `BR-53` makes the pair inseparable.
//! 3. **A discount that is not a discount yet.** A Skonto is earned by paying early,
//!    which has not happened. It must not touch the taxable base — it goes in BT-20,
//!    in the structured form `BR-DE-18` requires.

use billing::FixedRateTax;
use billing::prelude::*;
use rust_decimal::{Decimal, dec};

/// Polish standard rate.
const VAT_RATE: Decimal = dec!(0.23);
/// EUR → PLN on the invoice date. A real one comes from the tax authority's table;
/// the engine deliberately holds no rates and no clock.
const PLN_PER_EUR: Decimal = dec!(4.2650);

fn main() {
    let scale = AmountScale::EN16931;

    // ── 1. A gross-quoted catalogue ─────────────────────────────────────────────
    //
    // Shelf prices, VAT included, exactly as the customer saw them.
    let catalogue = [
        (
            "Bürostuhl",
            dec!(3),
            Amount::<5>::parse("249.99000").unwrap(),
        ),
        ("Schreibtisch", dec!(2), Amount::parse("529.00000").unwrap()),
        ("Monitorarm", dec!(4), Amount::parse("89.95000").unwrap()),
    ];

    println!("=== Cross-border retail invoice ===");
    println!();
    println!(
        "  Catalogue (prices include {} % VAT):",
        VAT_RATE * dec!(100)
    );

    let mut positions = Vec::new();
    let mut gross_quoted = Amount::<5>::ZERO;
    for (name, qty, gross_unit) in catalogue {
        // Convert the *unit* price once, then let the line arithmetic run on net
        // figures. `net_of_tax` is the remainder of `extract_tax`, so this unit price
        // and the VAT in it re-sum to the shelf price exactly. Whether that survives
        // aggregation is a separate question — see the end of `main`.
        let net_unit = gross_unit.net_of_tax(VAT_RATE, scale.strategy()).unwrap();
        let line_gross = gross_unit.checked_mul_qty(qty).unwrap();
        gross_quoted = gross_quoted.checked_add(line_gross).unwrap();

        println!("    {name:16} {qty:>3} × {gross_unit} gross  →  {net_unit} net per unit",);

        positions.push(
            LineItem::for_usage(
                name,
                // BT-130 = H87 (piece). BT-150 is deliberately *not* set: it is an
                // attribute of the price base quantity (BT-149), which this price
                // does not have — one unit at a time — and stating it alone is
                // unrepresentable in UBL. `UnitPrice::validate` refuses it.
                Quantity::new(qty, "Stk").with_code("H87"),
                UnitPrice::new(net_unit.into_decimal(), "EUR/Stk"),
            )
            .build()
            .unwrap(),
        );
    }

    // ── 2. Assemble at EN 16931 precision ───────────────────────────────────────
    //
    // `amount_scale` rounds every *leaf* and recomputes the aggregates, so the
    // two-decimal cap and the totals identities hold at the same time.
    let doc = BillingDocument::builder()
        .meta(DocumentMeta {
            invoice_number: "PL-2026-08-0042".into(),
            period_label: "August 2026".into(),
            currency: Currency::EUR,
            issue_date: Some("2026-08-09".into()),
            due_date: Some("2026-09-08".into()),
            // BT-20, with the Skonto in the structured form BR-DE-18 requires.
            payment_terms: Some(
                PaymentTerms::text("Zahlbar innerhalb 30 Tagen ohne Abzug.")
                    .with_discount(EarlyPaymentDiscount::new(10, dec!(2)).unwrap()),
            ),
            ..Default::default()
        })
        .amount_scale(scale)
        .positions(positions)
        .extra_tax(FixedRateTax::new("VAT", VAT_RATE).unwrap().boxed())
        .build()
        .unwrap();

    // ── 3. VAT restated in the accounting currency ──────────────────────────────
    let in_pln = VatAccountingCurrency::converted(
        Currency::new("PLN").unwrap(),
        doc.vat_total().unwrap(),
        PLN_PER_EUR,
        scale,
    )
    .unwrap();
    let doc = doc.with_vat_accounting_currency(in_pln).unwrap();

    // ── Render ──────────────────────────────────────────────────────────────────
    println!();
    for pos in doc.all_positions() {
        println!("  {:44} {:>12}", pos.description, pos.net_amount);
    }
    println!();
    println!(
        "  {:44} {:>12}   BT-109",
        "Total without VAT",
        doc.taxable_total().unwrap()
    );
    println!("  {:44} {:>12}   BT-110", "VAT", doc.vat_total().unwrap());
    println!(
        "  {:44} {:>12}   BT-112",
        "Total with VAT",
        doc.gross_total()
    );
    println!(
        "  {:44} {:>12}   BT-115",
        "Amount due",
        doc.amount_due().unwrap()
    );

    let acct = doc.vat_accounting_currency().unwrap();
    println!();
    println!(
        "  VAT in the accounting currency: {} {}   BT-6 / BT-111 (at {PLN_PER_EUR} PLN/EUR)",
        acct.vat_total(),
        acct.currency(),
    );

    let terms = doc.meta.payment_terms.as_ref().unwrap();
    println!();
    println!("  Payment terms (BT-20):");
    for line in terms.to_string().lines() {
        println!("    {line}");
    }

    // ── The three properties worth asserting ────────────────────────────────────

    // (1) BR-CO-15 holds: BT-112 = BT-109 + BT-110, exactly, at two decimals.
    assert_eq!(
        doc.taxable_total()
            .unwrap()
            .checked_add(doc.vat_total().unwrap())
            .unwrap(),
        doc.gross_total(),
    );

    //     …and yet the invoice does *not* total the sum of the shelf prices. This
    //     looks like a bug and is not, so it is worth printing rather than hiding.
    //
    //     VAT on an invoice is charged once per category, not once per line:
    //     BR-CO-17 defines BT-117 as a single rounding of BT-116 × rate, and a
    //     validator recomputes it that way. Here the per-line taxes sum to 405,36
    //     while the tax on the summed base is 405,35. The catalogue total is the
    //     figure built from independently rounded numbers — and the one with no
    //     standing in EN 16931.
    //
    //     Forcing them to agree would mean adjusting a line to break BR-CO-17,
    //     which is checkable, in order to preserve an identity that is not.
    let drift = doc
        .gross_total()
        .checked_sub(gross_quoted)
        .unwrap()
        .checked_abs()
        .unwrap();
    println!();
    println!("  Gross the customer was quoted: {gross_quoted}  (sum of shelf prices)");
    println!(
        "  Gross the invoice states:      {}  BT-112",
        doc.gross_total()
    );
    println!("  Difference:                    {drift}  ← BR-CO-17, not a rounding error");

    // The gap is bounded by one minor unit per line: each line's tax is folded into
    // one per-category rounding instead of being rounded on its own.
    let max_drift = Amount::<5>::parse("0.01000")
        .unwrap()
        .checked_mul_qty(Decimal::from(catalogue.len()))
        .unwrap();
    assert!(
        drift <= max_drift,
        "drift {drift} exceeds one minor unit per line"
    );

    // (2) Every amount — including BT-111, capped by BR-DEC-15 — fits two decimals,
    //     and every identity still holds exactly at that precision.
    assert!(doc.fits_amount_scale(2));
    doc.assert_valid();
    doc.verify_vat_attribution().unwrap();
    assert_eq!(doc.unattributed_positions().count(), 0);
    println!("✓ EN 16931: two decimals throughout, totals and BG-23 both exact");

    // (3) The Skonto changed nothing. BT-115 is the full amount; what the buyer
    //     transfers if they pay within ten days is a separate figure, and taking it
    //     later triggers a VAT correction rather than a smaller invoice.
    let due = doc.amount_due().unwrap();
    let skonto = terms
        .discount_within(3)
        .expect("day 3 is inside the window");
    println!(
        "✓ Skonto {} %: pay {} by day {} instead of {} — BT-115 is unchanged",
        skonto.percent(),
        skonto.payable_on(due).unwrap(),
        skonto.days(),
        due,
    );
}
