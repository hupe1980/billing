//! Water utility tiered billing example.
//!
//! Demonstrates:
//! - Graduated pricing with a domain-specific unit (`"m³"`) and an explicit currency
//! - Minimum charge
//! - ProportionalAllocation for shared-meter billing

use billing::prelude::*;
use rust_decimal::dec;

fn main() {
    // Three-tier graduated water tariff — unit is m³, not kWh
    let schedule = TariffSchedule::graduated()
        .unit("m³")
        // Without this, generated unit-price labels read "XXX/m³" — the ISO 4217
        // "no currency" code. The engine never assumes a currency.
        .currency(Currency::EUR)
        .band(TariffBand::up_to(
            dec!(5),
            Amount::parse("0.80000").unwrap(),
        ))
        .band(TariffBand::between(
            dec!(5),
            dec!(20),
            Amount::parse("1.40000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(20),
            Amount::parse("2.60000").unwrap(),
        ))
        .build()
        .unwrap();

    let consumption_m3 = dec!(28.5);
    let items = schedule.split(consumption_m3).unwrap();

    let meta = DocumentMeta {
        invoice_number: "WATER-2026-07-001".into(),
        period_label: "July 2026".into(),
        currency: Currency::EUR,
        ..Default::default()
    };

    let mut doc = BillingDocument::from_positions(meta, items, vec![], vec![]).unwrap();

    // Minimum charge: 5 m³ × EUR 0.80 = EUR 4.00 even on low-consumption months
    let min = Amount::parse("4.00000").unwrap();
    if let Some(min_item) = minimum_charge(&doc, min, "Mindestmenge (5 m³)").unwrap() {
        doc = doc.with_extra_position(min_item).unwrap();
    }

    println!("=== Water Invoice ({consumption_m3} m³) ===");
    println!();
    for pos in doc.all_positions() {
        println!(
            "  {:40} {:>12} {}",
            pos.description,
            pos.net_amount,
            doc.currency()
        );
    }
    println!();
    println!(
        "  {:40} {:>12} {}",
        "TOTAL",
        doc.net_total(),
        doc.currency()
    );

    doc.assert_valid();
    println!("✓ Document validation passed");

    // Shared meter: allocate between 3 tenants proportionally
    println!();
    println!("--- Allocation (3 tenants: 40% / 35% / 25%) ---");
    let alloc = ProportionalAllocation::new(vec![dec!(0.40), dec!(0.35), dec!(0.25)]).unwrap();
    let tenant_docs = alloc.allocate(&doc).unwrap();
    let sum: Amount<5> = tenant_docs.iter().map(|d| d.net_total()).sum();
    for (i, tenant_doc) in tenant_docs.iter().enumerate() {
        println!("  Tenant {}: {}", i + 1, tenant_doc.net_total());
    }
    // Exact: no rounding drift
    assert_eq!(sum, doc.net_total());
    println!("✓ Allocation sum matches original (no rounding drift)");
}
