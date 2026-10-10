//! Benchmarks for the hot paths: amount parsing and arithmetic, splits,
//! tariffs, interval series, transaction fees, progressive increments,
//! metering aggregation, document builds and document splits.
//!
//! Run with `cargo bench`; CI only compiles them (`cargo bench --no-run`).

use std::hint::black_box;

use billing::{
    Aggregation, Amount, AmountScale, Billed, Currency, Document, Line, Percent, Percentile, Price,
    Quantity, Sample, Tariff, Tier, Tiers, TransactionFee, UnitPrice, Vat, rate_series, split,
    split_matrix,
};
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use rust_decimal::{Decimal, dec};

/// `n` lines at alternating 19 % / 7 %, with fractional quantities and prices.
fn lines(n: usize) -> Vec<Line> {
    let full = Vat::standard(Percent::new(dec!(19))).expect("valid rate");
    let reduced = Vat::standard(Percent::new(dec!(7))).expect("valid rate");
    (0..n)
        .map(|i| {
            let q = Decimal::from(i as u64 % 97 + 1) * dec!(1.25);
            let p = Decimal::new(i as i64 % 1000 + 1, 4);
            let vat = if i % 3 == 0 {
                reduced.clone()
            } else {
                full.clone()
            };
            Line::new(
                "item",
                Quantity::new(q, "C62"),
                UnitPrice::new(Price::new(p)),
            )
            .with_vat(vat)
        })
        .collect()
}

fn document(n: usize) -> Document {
    Document::builder(Currency::EUR, AmountScale::EN16931)
        .lines(lines(n))
        .build()
        .expect("valid document")
}

fn amount(c: &mut Criterion) {
    let mut g = c.benchmark_group("amount");
    g.bench_function("parse", |b| {
        b.iter(|| Amount::<5>::parse(black_box("-12345.67891")))
    });
    let (x, y) = (
        Amount::<5>::from_raw(1_234_567),
        Amount::<5>::from_raw(-7_654_321),
    );
    g.bench_function("checked_add", |b| {
        b.iter(|| black_box(x).checked_add(black_box(y)))
    });
    g.bench_function("reduce", |b| {
        b.iter(|| AmountScale::EN16931.reduce::<5>(black_box(dec!(10.02629)) * dec!(0.19)))
    });
    g.bench_function("display", |b| b.iter(|| black_box(y).to_string()));
    g.finish();
}

fn splits(c: &mut Criterion) {
    let mut g = c.benchmark_group("split");
    let weights: Vec<Decimal> = (1..=10).map(Decimal::from).collect();
    g.bench_function("split 10 ways", |b| {
        b.iter(|| split(black_box(dec!(1234.56)), black_box(&weights), 2))
    });
    for (rows, cols) in [(100usize, 10usize), (1000, 50)] {
        let totals: Vec<Decimal> = (0..rows)
            .map(|i| Decimal::new(i as i64 * 37 + 1, 2))
            .collect();
        let w: Vec<Decimal> = (0..cols).map(|j| Decimal::from(j as u64 % 7 + 1)).collect();
        g.bench_with_input(
            BenchmarkId::new("split_matrix", format!("{rows}x{cols}")),
            &(totals, w),
            |b, (t, w)| b.iter(|| split_matrix(black_box(t), black_box(w), 2)),
        );
    }
    g.finish();
}

fn pricing(c: &mut Criterion) {
    let mut g = c.benchmark_group("pricing");
    let tiers = Tiers::new([
        Tier::up_to(dec!(100), Price::new(dec!(0.30))).with_flat(dec!(5)),
        Tier::up_to(dec!(1000), Price::new(dec!(0.25))),
        Tier::up_to(dec!(10000), Price::new(dec!(0.20))),
        Tier::above(Price::new(dec!(0.15))),
    ])
    .expect("valid tiers");
    let graduated = Tariff::Graduated(tiers);
    g.bench_function("tariff graduated", |b| {
        b.iter(|| graduated.rate(black_box(dec!(12345.678))))
    });
    // One year of hourly spot prices, some negative.
    let year: Vec<(Decimal, Price)> = (0..8760i64)
        .map(|h| {
            (
                Decimal::new(h % 50 + 100, 2),
                Price::new(Decimal::new(h % 300 - 40, 4)),
            )
        })
        .collect();
    g.bench_function("rate_series 8760", |b| {
        b.iter(|| rate_series(black_box(year.iter().copied())))
    });
    // One million card transactions, a tenth of them refunds.
    let fee = TransactionFee::new(Percent::new(dec!(2.9)))
        .with_fixed(dec!(0.30))
        .with_min(dec!(0.50))
        .with_max(dec!(5.00))
        .with_free_transactions(100);
    let amounts: Vec<Decimal> = (0..1_000_000i64)
        .map(|i| {
            Decimal::new(
                if i % 10 == 0 {
                    -(i % 9_973)
                } else {
                    i % 99_991
                },
                2,
            )
        })
        .collect();
    g.sample_size(10);
    g.bench_function("transaction fee 1 000 000", |b| {
        b.iter(|| fee.rate(black_box(amounts.iter().copied())))
    });
    // An annual schedule billed monthly: twelve increments, rounded per tier.
    let months: Vec<Decimal> = (0..=12i64)
        .map(|m| Decimal::new(m * 1_234_567, 3))
        .collect();
    g.bench_function("increment_lines 12 months", |b| {
        b.iter(|| {
            let mut billed = Billed::Nothing;
            months
                .iter()
                .map(|to| {
                    let (lines, next) = graduated.increment_lines(
                        billed,
                        *to,
                        AmountScale::EN16931,
                        "energy",
                        "KWH",
                    )?;
                    billed = next;
                    Ok::<_, billing::Error>(lines.len())
                })
                .sum::<Result<usize, _>>()
        })
    });
    g.finish();
}

/// One million readings: a sum, a unique count over 10 000 keys and a p95.
fn meter(c: &mut Criterion) {
    let mut g = c.benchmark_group("meter");
    g.sample_size(10);
    let readings: Vec<Sample> = (0..1_000_000i64)
        .map(|i| Sample::new(Decimal::new(i % 10_007, 3)).with_key(format!("k{}", i % 10_000)))
        .collect();
    for (name, kind) in [
        ("sum 1M", Aggregation::Sum),
        ("unique_count 1M", Aggregation::UniqueCount),
        ("percentile 1M", Aggregation::Percentile(Percentile::P95)),
    ] {
        g.bench_function(name, |b| {
            b.iter(|| kind.apply(black_box(readings.iter().cloned())))
        });
    }
    g.finish();
}

fn documents(c: &mut Criterion) {
    let mut g = c.benchmark_group("document");
    for n in [10usize, 100, 1000] {
        let input = lines(n);
        g.bench_with_input(BenchmarkId::new("build", n), &input, |b, input| {
            b.iter(|| {
                Document::builder(Currency::EUR, AmountScale::EN16931)
                    .lines(black_box(input.clone()))
                    .build()
            })
        });
    }
    let doc = document(100);
    let weights: Vec<Decimal> = (1..=10).map(Decimal::from).collect();
    g.bench_function("split 100 lines into 10", |b| {
        b.iter(|| doc.split(black_box(&weights)))
    });
    g.finish();
}

criterion_group!(benches, amount, splits, pricing, meter, documents);
criterion_main!(benches);
