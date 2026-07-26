//! Cloud compute billing example.
//!
//! Demonstrates:
//! - `WeightedSumAggregator` for CPU-hours (VMs active for fractions of period)
//! - `MaxAggregator` + `TariffSchedule::capacity()` for peak-bandwidth billing
//! - `DynamicPricing` with `.with_unit("GB")?` for spot-priced storage
//! - `BillingDocument::builder()` with extra tax layer
//! - Domain-specific unit labels (`"CPU-h"`, `"Mbps"`, `"GB"`)

use billing::aggregation::{MaxAggregator, UsageAggregator, WeightedSumAggregator};
use billing::prelude::*;
use rust_decimal::Decimal;
use rust_decimal::dec;

// ── Domain types ──────────────────────────────────────────────────────────────

struct VmSnapshot {
    vcpus: u32,
    /// Fraction of the billing period this VM was active (0.0 – 1.0).
    active_fraction: Decimal,
}

struct BandwidthSample {
    mbps: Decimal,
}

fn main() {
    // ── 1. CPU billing: WEIGHTED_SUM aggregation → graduated pricing ─────────
    let vms = vec![
        VmSnapshot {
            vcpus: 8,
            active_fraction: dec!(1.000),
        },
        VmSnapshot {
            vcpus: 4,
            active_fraction: dec!(0.500),
        },
        VmSnapshot {
            vcpus: 16,
            active_fraction: dec!(10.0) / dec!(31.0),
        },
    ];

    // vCPU-hours = vCPUs × (active_fraction × 744 h/month)
    let cpu_agg = WeightedSumAggregator::new(
        |v: &VmSnapshot| Decimal::from(v.vcpus) * dec!(744),
        |v: &VmSnapshot| v.active_fraction,
    );
    let total_cpu_hours = cpu_agg.aggregate(&vms);

    let cpu_schedule = TariffSchedule::graduated()
        .unit("CPU-h")
        .currency(Currency::EUR)
        .band(TariffBand::free_up_to(dec!(1000)).with_description("Free tier (first 1 000 CPU-h)"))
        .band(
            TariffBand::between(dec!(1000), dec!(5000), Amount::parse("0.04800").unwrap())
                .with_description("Standard (1k–5k CPU-h)"),
        )
        .band(
            TariffBand::over(dec!(5000), Amount::parse("0.03200").unwrap())
                .with_description("Volume discount (>5k CPU-h)"),
        )
        .build()
        .unwrap();

    let cpu_items = cpu_schedule.split(total_cpu_hours).unwrap();

    // ── 2. Network billing: MAX aggregation → capacity pricing ───────────────
    let bandwidth_samples = vec![
        BandwidthSample { mbps: dec!(45.2) },
        BandwidthSample { mbps: dec!(112.8) }, // peak
        BandwidthSample { mbps: dec!(89.3) },
        BandwidthSample { mbps: dec!(23.1) },
    ];

    let peak_agg = MaxAggregator::new(|b: &BandwidthSample| b.mbps);
    let peak_mbps = peak_agg.aggregate(&bandwidth_samples);

    let bandwidth_schedule = TariffSchedule::capacity()
        .unit("Mbps")
        .currency(Currency::EUR)
        .band(
            TariffBand::up_to(dec!(100), Amount::parse("50.00000").unwrap())
                .with_description("Up to 100 Mbps"),
        )
        .band(
            TariffBand::between(dec!(100), dec!(500), Amount::parse("100.00000").unwrap())
                .with_description("101–500 Mbps"),
        )
        .band(
            TariffBand::over(dec!(500), Amount::parse("200.00000").unwrap())
                .with_description("Over 500 Mbps"),
        )
        .build()
        .unwrap();

    let bandwidth_item = bandwidth_schedule.apply_peak(peak_mbps).unwrap();

    // ── 3. Storage billing: dynamic spot pricing ─────────────────────────────
    let storage_intervals: Vec<(Decimal, Amount<5>)> = vec![
        (dec!(100.0), Amount::parse("0.02300").unwrap()), // day 1–10: $0.023/GB
        (dec!(120.0), Amount::parse("0.02100").unwrap()), // day 11–20: spot down
        (dec!(115.0), Amount::parse("0.02500").unwrap()), // day 21–31: spot up
    ];
    let storage_item = DynamicPricing::builder()
        .unit("GB")
        .currency(Currency::EUR)
        .intervals(storage_intervals)
        .build()
        .unwrap()
        .calculate()
        .unwrap();

    // ── 4. Assemble billing document ─────────────────────────────────────────
    let mut all_positions: Vec<LineItem> = cpu_items;
    all_positions.push(bandwidth_item);
    all_positions.push(storage_item);

    let doc = BillingDocument::builder()
        .meta(DocumentMeta {
            invoice_number: "CLOUD-2026-07".into(),
            currency: Currency::EUR,
            period_label: "July 2026".into(),
            ..Default::default()
        })
        .positions(all_positions.clone())
        .extra_tax(FixedRateTax::new("VAT", dec!(0.20)).unwrap().boxed())
        .build()
        .unwrap();

    println!("=== Cloud Compute Invoice ===");
    println!();
    println!("  CPU-hours billed: {total_cpu_hours:.1}");
    println!("  Peak bandwidth:   {peak_mbps:.1} Mbps");
    println!();
    // Iterate the *net* positions only. `all_positions()` also yields the tax
    // positions, so printing those here and the VAT again below reports it twice.
    for pos in doc.net_positions() {
        println!("  {:48} {:>12}", pos.description, pos.net_amount);
    }
    println!();
    println!("  {:48} {:>12}", "NET TOTAL", doc.net_total());
    println!("  {:48} {:>12}", "VAT (20%)", doc.tax_total());
    println!("  {:48} {:>12}", "GROSS TOTAL", doc.gross_total());

    doc.assert_valid();
    println!();
    println!("✓ Document validation passed");

    // ── Engine precision vs. invoice precision ───────────────────────────────
    //
    // This document is kept at the engine's full five decimals: metered usage
    // priced per unit lands on more than two decimals routinely (net 500.655),
    // which is arithmetically exact but not a number an invoice can carry.
    // EN 16931 caps every monetary amount at two decimals, so the document above
    // is correct and *not* emittable:
    println!();
    match doc.amount_scale_violation(2) {
        None => println!("  (already fits two decimals)"),
        Some((what, value)) => println!("  Not emittable as EN 16931: {what} = {value}"),
    }

    // Rebuilding with a scale reduces every leaf *before* the totals are summed, so
    // the decimal limit and the totals identities hold at the same time. Rounding
    // the finished totals instead would break them.
    let invoiceable = BillingDocument::builder()
        .meta(DocumentMeta {
            invoice_number: "CLOUD-2026-07".into(),
            currency: Currency::EUR,
            period_label: "July 2026".into(),
            ..Default::default()
        })
        .amount_scale(AmountScale::EN16931)
        .positions(all_positions)
        .extra_tax(FixedRateTax::new("VAT", dec!(0.20)).unwrap().boxed())
        .build()
        .unwrap();

    println!();
    println!("  --- rebuilt at EN 16931 precision ---");
    println!(
        "  {:48} {:>12}",
        "NET TOTAL",
        invoiceable
            .net_total()
            .checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero)
            .unwrap()
    );
    println!(
        "  {:48} {:>12}",
        "VAT (20%)",
        invoiceable
            .tax_total()
            .checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero)
            .unwrap()
    );
    println!(
        "  {:48} {:>12}",
        "GROSS TOTAL",
        invoiceable
            .gross_total()
            .checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero)
            .unwrap()
    );
    invoiceable.assert_valid();
    assert!(invoiceable.fits_amount_scale(2));
    // net + VAT == gross still holds exactly at two decimals (EN 16931 BR-CO-15).
    assert_eq!(
        invoiceable.net_total() + invoiceable.tax_total(),
        invoiceable.gross_total()
    );
    println!();
    println!("✓ Emittable as EN 16931: every amount fits two decimals, totals intact");
}
