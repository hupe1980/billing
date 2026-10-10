//! One supplier invoice split across the tenants of a building (Nebenkosten).
//!
//! The water utility's annual invoice is split by living area. Every leaf and
//! every VAT group is conserved exactly, so the tenants' shares add up to the
//! supplier's invoice to the cent. One flat changed tenant on 30 June, and its
//! share is prorated by days with `Line::segments`.
//!
//! Run with `cargo run --example nebenkosten_split`.

use billing::{
    Amount, AmountScale, Currency, Document, Line, Percent, Price, Quantity, UnitPrice, Vat,
};
use rust_decimal::{Decimal, dec};

fn main() -> Result<(), billing::Error> {
    let scale = AmountScale::EN16931;
    let reduced = Vat::standard(Percent::new(dec!(7)))?;

    // The supplier's invoice, as received.
    let supplier = Document::builder(Currency::EUR, scale)
        .default_vat(reduced)
        .line(Line::new(
            "Water supply",
            Quantity::new(dec!(182.5), "MTQ"),
            UnitPrice::new(Price::new(dec!(2.115))),
        ))
        .line(Line::new(
            "Standing charge",
            Quantity::new(dec!(12), "MON"),
            UnitPrice::new(Price::new(dec!(9.50))),
        ))
        .line(
            Line::new(
                "Meter rental",
                Quantity::new(dec!(1), "ANN"),
                UnitPrice::new(Price::new(dec!(23.80))),
            )
            .with_vat(Vat::standard(Percent::new(dec!(19)))?),
        )
        .build()?;
    println!(
        "supplier: net {:.2}, VAT {:.2}, gross {:.2}",
        supplier.tax_exclusive_total(),
        supplier.vat_total(),
        supplier.tax_inclusive_total()
    );
    // 182.5 × 2.115 = 385.9875 → 385.99; + 114.00 at 7 %; + 23.80 at 19 %.
    assert_eq!(supplier.tax_exclusive_total(), Amount::parse("523.79")?);
    assert_eq!(supplier.vat_total(), Amount::parse("39.52")?); // 34.9993 → 35.00, 4.522 → 4.52
    assert_eq!(supplier.tax_inclusive_total(), Amount::parse("563.31")?);

    // Split by living area (m²).
    let flats = ["EG links", "EG rechts", "OG links", "OG rechts"];
    let area = [dec!(62.5), dec!(48), dec!(81.25), dec!(35)];
    let shares = supplier.split(&area)?;
    for (flat, share) in flats.iter().zip(&shares) {
        println!(
            "  {flat:<10} net {:>7.2}  VAT {:>6.2}  gross {:>7.2}",
            share.tax_exclusive_total(),
            share.vat_total(),
            share.tax_inclusive_total()
        );
    }

    assert_eq!(shares[0].tax_exclusive_total(), Amount::parse("144.37")?);
    assert_eq!(shares[1].tax_inclusive_total(), Amount::parse("119.25")?);

    // Exact conservation: totals, every line, every VAT group.
    let sum = |f: fn(&Document) -> Amount| shares.iter().map(f).sum::<Amount>();
    assert_eq!(
        sum(Document::tax_exclusive_total),
        supplier.tax_exclusive_total()
    );
    assert_eq!(sum(Document::vat_total), supplier.vat_total());
    assert_eq!(
        sum(Document::tax_inclusive_total),
        supplier.tax_inclusive_total()
    );
    for i in 0..supplier.lines().len() {
        let lines: Amount = shares.iter().map(|s| s.line_net(i)).sum();
        assert_eq!(lines, supplier.line_net(i));
    }
    for (k, group) in supplier.vat_groups().iter().enumerate() {
        let base: Amount = shares
            .iter()
            .map(|s| s.vat_groups()[k].taxable_base())
            .sum();
        let tax: Amount = shares.iter().map(|s| s.vat_groups()[k].tax()).sum();
        assert_eq!((base, tax), (group.taxable_base(), group.tax()));
    }

    // "EG rechts" changed tenant on 30 June: 181 days old tenant, 184 days new.
    let share = &shares[1];
    let days = [dec!(181), dec!(184)];
    let mut old_tenant = Vec::new();
    let mut new_tenant = Vec::new();
    for line in share.lines() {
        let mut parts = line.segments(&days, scale)?.into_iter();
        old_tenant.extend(parts.next());
        new_tenant.extend(parts.next());
    }
    let net = |lines: &[Line]| -> Decimal { lines.iter().filter_map(Line::stated_net).sum() };
    println!(
        "  EG rechts: old tenant net {:.2}, new tenant net {:.2}",
        net(&old_tenant),
        net(&new_tenant)
    );
    assert_eq!(
        (net(&old_tenant), net(&new_tenant)),
        (dec!(54.99), dec!(55.89))
    );
    // Each line's segments add up to the share's line exactly.
    for (i, (a, b)) in old_tenant.iter().zip(&new_tenant).enumerate() {
        let whole = a.stated_net().unwrap_or_default() + b.stated_net().unwrap_or_default();
        assert_eq!(whole, share.line_net(i).to_decimal());
    }
    assert_eq!(
        net(&old_tenant) + net(&new_tenant),
        share.line_total().to_decimal()
    );

    // Each tenant's statement is an ordinary document again. Its VAT is derived
    // again from its own base, so the two statements' gross (59.14 + 60.10) may
    // differ from the share's 119.25 by a cent; the nets above are exact.
    let mut gross = Amount::ZERO;
    for (who, lines) in [("old", old_tenant), ("new", new_tenant)] {
        let doc = Document::builder(Currency::EUR, scale)
            .lines(lines)
            .build()?;
        println!(
            "  {who} tenant statement: gross {:.2}",
            doc.tax_inclusive_total()
        );
        gross += doc.tax_inclusive_total();
    }
    assert_eq!(gross, Amount::parse("119.24")?);
    assert!((gross - share.tax_inclusive_total()).checked_abs()? <= Amount::parse("0.01")?);

    // Where the VAT must add up too, split the share document itself: its VAT
    // groups are partitioned, so the two statements sum to 119.25 exactly.
    let by_days = share.split(&days)?;
    let gross: Amount = by_days.iter().map(Document::tax_inclusive_total).sum();
    println!(
        "  by Document::split: gross {:.2} + {:.2}",
        by_days[0].tax_inclusive_total(),
        by_days[1].tax_inclusive_total()
    );
    assert_eq!(gross, share.tax_inclusive_total());
    Ok(())
}
