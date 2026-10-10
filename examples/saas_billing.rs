//! A SaaS invoice: graduated API calls, packaged storage, per-seat licences, a
//! percentage discount across the whole invoice, and a gross minimum charge.
//!
//! Run with `cargo run --example saas_billing`.

use billing::{
    Adjust, AdjustmentKind, Amount, AmountScale, Basis, Bound, Currency, Document, Line,
    PackageRounding, Percent, Price, Quantity, Tariff, Tier, Tiers, UnitPrice, Vat,
};
use rust_decimal::{Decimal, dec};

/// One month's usage of one customer.
struct Usage {
    api_calls: Decimal,
    storage_gb: Decimal,
    seats: Decimal,
}

fn invoice(usage: &Usage) -> Result<Document, billing::Error> {
    // API calls: the first 10 000 free, then 0.002 up to 100 000, then 0.001.
    let api = Tariff::Graduated(Tiers::new([
        Tier::up_to(dec!(10000), Price::new(dec!(0))),
        Tier::up_to(dec!(100000), Price::new(dec!(0.002))),
        Tier::above(Price::new(dec!(0.001))),
    ])?);
    // Storage: started 10 GB packages at 5.00 each.
    let storage = Tariff::Package {
        size: dec!(10),
        price: Price::new(dec!(5)),
        rounding: PackageRounding::Up,
    };

    let mut lines: Vec<Line> = Vec::new();
    for piece in api.rate(usage.api_calls)? {
        let tier = piece.tier;
        lines.extend(piece.into_lines(&format!("API calls, tier {tier}"), "C62"));
    }
    for piece in storage.rate(usage.storage_gb)? {
        let measured = piece.measured.unwrap_or_default();
        lines.extend(piece.into_lines(&format!("Storage ({measured} GB used)"), "C62"));
    }
    lines.push(Line::new(
        "Seats",
        Quantity::new(usage.seats, "C62"),
        UnitPrice::new(Price::new(dec!(9))),
    ));

    Document::builder(Currency::EUR, AmountScale::EN16931)
        .default_vat(Vat::standard(Percent::new(dec!(19)))?)
        .lines(lines)
        .apportion(
            AdjustmentKind::Allowance,
            "Startup programme, 10 %",
            Adjust::Percent(Percent::new(dec!(10))),
        )
        .bound(Bound::minimum(
            Basis::Gross,
            dec!(250),
            "Minimum monthly charge",
        ))
        .build()
}

fn print(name: &str, doc: &Document) {
    println!("== {name}");
    for (i, line) in doc.lines().iter().enumerate() {
        println!(
            "  {:<32} {:>10} × {:>6}  {:>9.2}",
            line.description(),
            line.quantity().value(),
            line.price().price(),
            doc.line_net(i)
        );
    }
    for (i, a) in doc.adjustments().iter().enumerate() {
        println!(
            "  {:<32} {:>29.2}",
            a.description(),
            -doc.adjustment_amount(i)
        );
    }
    println!(
        "  net {:.2} + VAT {:.2} = gross {:.2}",
        doc.tax_exclusive_total(),
        doc.vat_total(),
        doc.tax_inclusive_total()
    );
}

fn main() -> Result<(), billing::Error> {
    let amount = |s: &str| Amount::parse(s);

    // A busy customer: well above the minimum.
    let busy = invoice(&Usage {
        api_calls: dec!(142000),
        storage_gb: dec!(23.5),
        seats: dec!(12),
    })?;
    print("busy", &busy);
    // API: 90 000 × 0.002 + 42 000 × 0.001 = 222.00; storage: 3 packages = 15.00;
    // seats: 108.00. Σ 345.00, less 10 % = 310.50, VAT 58.995 → 59.00.
    assert_eq!(busy.line_total(), amount("345.00")?);
    assert_eq!(busy.allowance_total(), amount("34.50")?);
    assert_eq!(busy.vat_total(), amount("59.00")?);
    assert_eq!(busy.tax_inclusive_total(), amount("369.50")?);
    assert_eq!(busy.bound_lines(), 0, "the minimum already holds");

    // A quiet customer: the minimum tops the invoice up to exactly 250.00 gross.
    let quiet = invoice(&Usage {
        api_calls: dec!(42000),
        storage_gb: dec!(23.5),
        seats: dec!(12),
    })?;
    print("quiet", &quiet);
    // 64.00 + 15.00 + 108.00 = 187.00, less 18.70 = 168.30, gross 200.28.
    // The top-up is the smallest whole-cent line that reaches 250.00 gross.
    assert_eq!(quiet.bound_lines(), 1);
    assert_eq!(quiet.line_net(quiet.lines().len() - 1), amount("41.78")?);
    assert_eq!(quiet.tax_exclusive_total(), amount("210.08")?);
    assert_eq!(quiet.tax_inclusive_total(), amount("250.00")?);
    // The BR-CO identities hold by construction.
    assert_eq!(
        quiet.tax_exclusive_total(),
        quiet.line_total() - quiet.allowance_total() + quiet.charge_total()
    );
    Ok(())
}
