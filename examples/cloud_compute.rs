//! A colocation invoice: power passed through at hourly spot prices (some of
//! them negative), and 95th-percentile burstable bandwidth on volume tiers.
//!
//! Run with `cargo run --example cloud_compute`.

use billing::{
    Amount, AmountScale, Currency, Document, Percent, Percentile, Price, Tariff, Tier, Tiers, Vat,
    nearest_rank, rate_series,
};
use rust_decimal::{Decimal, dec};

/// Hourly spot prices in EUR/kWh for one day: negative around a sunny noon.
const SPOT: [Decimal; 24] = [
    dec!(0.091),
    dec!(0.085),
    dec!(0.082),
    dec!(0.080),
    dec!(0.083),
    dec!(0.095),
    dec!(0.118),
    dec!(0.132),
    dec!(0.104),
    dec!(0.061),
    dec!(0.012),
    dec!(-0.004),
    dec!(-0.021),
    dec!(-0.035),
    dec!(-0.018),
    dec!(0.009),
    dec!(0.047),
    dec!(0.098),
    dec!(0.141),
    dec!(0.156),
    dec!(0.137),
    dec!(0.112),
    dec!(0.101),
    dec!(0.094),
];

fn main() -> Result<(), billing::Error> {
    // Power: the rack draws 4.2 kWh per hour, 6.0 kWh while batch jobs run 10–16 h.
    let draw = |h: usize| {
        if (10..16).contains(&h) {
            dec!(6.0)
        } else {
            dec!(4.2)
        }
    };
    let power = rate_series((0..24).map(|h| (draw(h), Price::new(SPOT[h]))))?;
    println!(
        "power: {} kWh, exact net {}, average {} EUR/kWh",
        power.quantity, power.net, power.average_price
    );
    // Σ qᵢ·pᵢ is exact: no interval was rounded.
    assert_eq!(power.quantity, dec!(111.6));
    assert_eq!(power.net, dec!(7.2894));
    assert!(!power.is_negative_average());

    // A sunny Sunday where the battery charged at negative prices all day: the
    // average is negative, so the line carries a negative quantity at a
    // non-negative price (BR-27) and the net is a credit.
    let sunday = rate_series([
        (dec!(30), Price::new(dec!(-0.012))),
        (dec!(12), Price::new(dec!(0.004))),
    ])?;
    assert_eq!(sunday.net, dec!(-0.312));
    assert!(sunday.is_negative_average());
    let sunday_line = sunday.into_line("Power, Sunday (spot)", "KWH");
    assert_eq!(sunday_line.quantity().value(), dec!(-42));
    assert!(sunday_line.price().price().value() >= Decimal::ZERO);

    // Bandwidth: 5-minute samples for 30 days, billed at the 95th percentile.
    let mut samples: Vec<Decimal> = (0..8640u32)
        .map(|i| Decimal::from(40 + (i * 37) % 200)) // 40 … 239 Mbit/s
        .collect();
    let billed = nearest_rank(&mut samples, Percentile::P95).unwrap_or_default();
    println!(
        "bandwidth: p95 = {billed} Mbit/s over {} samples",
        samples.len()
    );
    assert_eq!(billed, dec!(229)); // rank ⌈0.95 × 8640⌉ = 8208 of the sorted samples

    // Volume tiers: every Mbit/s at the price of the tier the p95 falls in.
    let bandwidth = Tariff::Volume(Tiers::new([
        Tier::up_to(dec!(100), Price::new(dec!(1.20))),
        Tier::up_to(dec!(500), Price::new(dec!(0.90))),
        Tier::above(Price::new(dec!(0.70))),
    ])?);
    let pieces = bandwidth.rate(billed)?;
    assert_eq!((pieces[0].tier, pieces[0].exact_net()?), (1, dec!(206.10)));

    let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
        .default_vat(Vat::standard(Percent::new(dec!(19)))?)
        .line(power.into_line("Power, weekday (spot)", "KWH"))
        .line(sunday_line)
        .lines(
            pieces
                .into_iter()
                .flat_map(|p| p.into_lines("Bandwidth, p95", "MBS")),
        )
        .build()?;
    for (i, line) in doc.lines().iter().enumerate() {
        println!("  {:<24} {:>8.2}", line.description(), doc.line_net(i));
    }
    println!(
        "net {:.2} + VAT {:.2} = {:.2}",
        doc.tax_exclusive_total(),
        doc.vat_total(),
        doc.payable()
    );
    // 7.2894 → 7.29, −0.312 → −0.31, 206.10: each series is rounded once.
    assert_eq!(doc.line_net(0), Amount::parse("7.29")?);
    assert_eq!(doc.line_net(1), Amount::parse("-0.31")?);
    assert_eq!(doc.tax_exclusive_total(), Amount::parse("213.08")?);
    assert_eq!(doc.vat_total(), Amount::parse("40.49")?); // 213.08 × 19 % = 40.4852
    assert!(
        doc.price_inconsistent().is_empty(),
        "PEPPOL-EN16931-R120 holds"
    );
    Ok(())
}
