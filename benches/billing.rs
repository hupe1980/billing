//! Benchmarks for the hot paths of the billing engine.
//!
//! The engine is pure arithmetic with no I/O, so throughput is entirely a
//! function of how much `Decimal` work each operation does and how many
//! `LineItem`s get cloned. These benchmarks exist to make a regression in either
//! visible: run `cargo bench -- --save-baseline main` before a change and
//! `cargo bench -- --baseline main` after it.

use std::hint::black_box;

use billing::FixedRateTax;
use billing::prelude::*;
use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use rust_decimal::{Decimal, dec};

fn amount_ops(c: &mut Criterion) {
    let mut group = c.benchmark_group("amount");
    let a = Amount::<5>::parse("1234.56789").unwrap();
    let b = Amount::<5>::parse("9876.54321").unwrap();

    group.bench_function("checked_add", |bn| {
        bn.iter(|| black_box(a).checked_add(black_box(b)))
    });
    group.bench_function("checked_mul_qty", |bn| {
        bn.iter(|| black_box(a).checked_mul_qty(black_box(dec!(1234.5))))
    });
    group.bench_function("parse", |bn| {
        bn.iter(|| Amount::<5>::parse(black_box("1234.56789")))
    });
    group.bench_function("to_string", |bn| bn.iter(|| black_box(a).to_string()));
    group.bench_function("round_to_increment", |bn| {
        bn.iter(|| {
            black_box(a).round_to_increment(
                Amount::<5>::parse("0.05000").unwrap(),
                RoundingStrategy::MidpointAwayFromZero,
            )
        })
    });
    group.bench_function("distribute_100", |bn| {
        bn.iter(|| black_box(a).distribute(black_box(100)))
    });
    group.finish();
}

fn schedule(c: &mut Criterion) {
    let sched = TariffSchedule::graduated()
        .unit("kWh")
        .currency(Currency::EUR)
        .band(TariffBand::up_to(
            dec!(500),
            Amount::parse("0.32000").unwrap(),
        ))
        .band(TariffBand::between(
            dec!(500),
            dec!(2000),
            Amount::parse("0.28000").unwrap(),
        ))
        .band(TariffBand::over(
            dec!(2000),
            Amount::parse("0.24000").unwrap(),
        ))
        .build()
        .unwrap();

    c.bench_function("schedule/graduated_split_3_bands", |bn| {
        bn.iter(|| sched.split(black_box(dec!(3456.789))))
    });
}

/// Build a document with `n` positions and a two-layer tax stack — the shape of a
/// realistic utility or SaaS invoice.
fn build_document(n: usize) -> BillingDocument {
    let positions: Vec<LineItem> = (0..n)
        .map(|i| {
            LineItem::for_usage(
                format!("Position {i}"),
                Decimal::from(i as u64 + 1) * dec!(10),
                "kWh",
                dec!(0.2891),
                "EUR/kWh",
            )
            .tag("commodity")
            .build()
            .unwrap()
        })
        .collect();
    let taxes: Vec<Box<dyn TaxLayer>> =
        vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19)).unwrap())];
    BillingDocument::from_positions(
        DocumentMeta {
            invoice_number: "BENCH-1".into(),
            currency: Currency::EUR,
            ..Default::default()
        },
        positions,
        taxes,
        vec![],
    )
    .unwrap()
}

fn document(c: &mut Criterion) {
    let mut group = c.benchmark_group("document");
    for n in [10usize, 100, 1000] {
        let positions: Vec<LineItem> = (0..n)
            .map(|i| {
                LineItem::fixed(
                    format!("Item {i}"),
                    Amount::<5>::from_raw_units(i as i64 * 997),
                )
                .build()
                .unwrap()
            })
            .collect();

        group.bench_function(format!("from_positions_{n}"), |bn| {
            bn.iter_batched(
                || positions.clone(),
                |p| {
                    let taxes: Vec<Box<dyn TaxLayer>> =
                        vec![Box::new(FixedRateTax::new("VAT", dec!(0.19)).unwrap())];
                    BillingDocument::from_positions(DocumentMeta::default(), p, taxes, vec![])
                },
                BatchSize::SmallInput,
            )
        });

        let doc = build_document(n);
        group.bench_function(format!("validate_{n}"), |bn| {
            bn.iter(|| black_box(&doc).validate())
        });
    }
    group.finish();
}

fn allocation(c: &mut Criterion) {
    let doc = build_document(50);
    let mut group = c.benchmark_group("allocation");
    for n in [3usize, 10, 50] {
        let rule = EqualAllocation::new(n).unwrap();
        group.bench_function(format!("equal_{n}_way"), |bn| {
            bn.iter(|| rule.allocate(black_box(&doc)))
        });
    }
    group.bench_function("proportional_split_1000_parts", |bn| {
        let fractions: Vec<Decimal> = (0..1000).map(|_| dec!(0.001)).collect();
        bn.iter(|| proportional_split(black_box(dec!(987654.321)), &fractions, 3))
    });
    group.finish();
}

criterion_group!(benches, amount_ops, schedule, document, allocation);
criterion_main!(benches);
