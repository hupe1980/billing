//! A German web shop selling across borders: an export outside the EU, a
//! reverse-charge service to an EU business, a domestic sale invoiced in USD with
//! the VAT also stated in EUR, and a credit note for that sale.
//!
//! Run with `cargo run --example cross_border_retail`.

use billing::{
    Amount, AmountScale, Currency, Document, Exemption, Line, Percent, Price, Quantity,
    TaxCategory, UnitPrice, Vat, VatAccounting,
};
use rust_decimal::dec;

fn item(description: &str, quantity: rust_decimal::Decimal, price: rust_decimal::Decimal) -> Line {
    Line::new(
        description,
        Quantity::new(quantity, "C62"),
        UnitPrice::new(Price::new(price)),
    )
}

fn show(name: &str, doc: &Document) {
    println!("== {name} ({})", doc.currency());
    for g in doc.vat_groups() {
        let why = g.exemption().and_then(Exemption::code).unwrap_or("-");
        println!(
            "  VAT {:<2} {:>6}  base {:>9.2}  tax {:>7.2}  exemption {why}",
            g.category(),
            g.rate_percent(),
            g.taxable_base(),
            g.tax()
        );
    }
    println!("  gross {:.2}", doc.tax_inclusive_total());
    if let Some((currency, vat)) = doc.vat_accounting_total() {
        println!("  VAT in accounting currency: {vat:.2} {currency}");
    }
}

fn main() -> Result<(), billing::Error> {
    let scale = AmountScale::EN16931;

    // 1. Export to Switzerland: category G, zero rate, with its exemption code.
    let export = Vat::exempt(TaxCategory::Export, Exemption::Code("VATEX-EU-G".into()))?;
    let order1 = Document::builder(Currency::EUR, scale)
        .default_vat(export)
        .line(item("Espresso machine", dec!(1), dec!(649)))
        .line(item("Descaler", dec!(3), dec!(8.95)))
        .build()?;
    show("export to CH", &order1);
    assert_eq!(order1.tax_exclusive_total(), Amount::parse("675.85")?);
    assert!(order1.vat_total().is_zero());
    assert_eq!(order1.vat_groups()[0].category().code(), "G");

    // 2. Installation service for a Polish business: reverse charge (AE). The
    //    reason is mandatory (BR-AE-10), and the constructor refuses a blank one.
    let reverse = Vat::exempt(
        TaxCategory::ReverseCharge,
        Exemption::Both {
            text: "Reverse charge".into(),
            code: "VATEX-EU-AE".into(),
        },
    )?;
    assert!(Vat::exempt(TaxCategory::ReverseCharge, Exemption::Code(String::new())).is_err());
    let order2 = Document::builder(Currency::EUR, scale)
        .default_vat(reverse)
        .line(item("Installation", dec!(4), dec!(85)))
        .build()?;
    show("reverse charge to PL", &order2);
    assert_eq!(order2.tax_inclusive_total(), Amount::parse("340.00")?);

    // 3. A domestic sale invoiced in USD. German VAT is also stated in EUR
    //    (BT-6 / BT-111): 0.92 EUR per USD, rounded once.
    let order3 = Document::builder(Currency::USD, scale)
        .default_vat(Vat::standard(Percent::new(dec!(19)))?)
        .line(item("Grinder", dec!(1), dec!(329)))
        .line(item("Beans, 1 kg", dec!(2), dec!(24.90)))
        .line(item("Book", dec!(1), dec!(39.90)).with_vat(Vat::standard(Percent::new(dec!(7)))?))
        .vat_accounting(VatAccounting {
            currency: Currency::EUR,
            rate: dec!(0.92),
        })
        .build()?;
    show("domestic in USD", &order3);
    // 378.80 × 19 % = 71.972 → 71.97; 39.90 × 7 % = 2.793 → 2.79; Σ 74.76 USD.
    assert_eq!(order3.vat_total(), Amount::parse("74.76")?);
    assert_eq!(order3.tax_inclusive_total(), Amount::parse("493.46")?);
    // 74.76 × 0.92 = 68.7792 → 68.78 EUR.
    assert_eq!(
        order3.vat_accounting_total(),
        Some((Currency::EUR, Amount::parse("68.78")?))
    );

    // The customer returns everything: the credit is the exact negative. Whether
    // it goes out as a negative invoice (380) or as a credit note (381) with the
    // original signs is decided by the format layer, never both.
    let credit = order3.negated();
    show("credit for order 3", &credit);
    assert_eq!(credit.tax_inclusive_total(), -order3.tax_inclusive_total());
    assert_eq!(
        credit.vat_accounting_total(),
        Some((Currency::EUR, Amount::parse("-68.78")?)) // same sign as BT-110 (R055)
    );
    for (g, c) in order3.vat_groups().iter().zip(credit.vat_groups()) {
        assert_eq!((c.taxable_base(), c.tax()), (-g.taxable_base(), -g.tax()));
    }
    // The order and its credit together come to zero.
    let both = Document::concat([order3.clone(), credit])?;
    assert!(both.payable().is_zero());

    // The VAT accounting currency must differ from the invoice currency (R005).
    let err = Document::builder(Currency::EUR, scale)
        .line(item("x", dec!(1), dec!(1)))
        .vat_accounting(VatAccounting {
            currency: Currency::EUR,
            rate: dec!(1),
        })
        .build()
        .err();
    assert_eq!(err.and_then(|e| e.rule()), Some("PEPPOL-EN16931-R005"));
    Ok(())
}
