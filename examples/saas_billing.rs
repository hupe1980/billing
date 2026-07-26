//! SaaS subscription + metered usage billing example.
//!
//! Demonstrates:
//! - `Tariff` trait for domain-specific billing logic
//! - `BillingDocument::builder().tariff(t, u)?` for ergonomic document creation
//! - Graduated pricing for API calls with a free tier (zero-price band)
//! - `PercentageCharge` for platform commission with a minimum floor
//! - `FixedRateTax` for VAT

use billing::prelude::*;
use billing::tags;
use rust_decimal::Decimal;
use rust_decimal::dec;
use std::convert::Infallible;

struct SaasUsage {
    seats: u32,
    api_calls: u64,
}

struct SaasTariff {
    base_fee_eur: u32,
    seat_price_eur: u32,
    free_api_calls: u64,
    /// Price per API call above the free tier. Use `Amount<6>` for sub-cent precision.
    overage_per_call: Amount<6>,
}

impl Tariff for SaasTariff {
    type Usage = SaasUsage;
    type Error = BillingError;
    /// This tariff always produces an invoice, so the "nothing to bill" outcome is
    /// uninhabited and `.bill()` returns a document directly.
    type NotBillable = Infallible;

    fn line_items(&self, usage: &SaasUsage) -> Result<Positions<Infallible>, BillingError> {
        let base = LineItem::fixed(
            "Platform base fee",
            Amount::<5>::from_int(self.base_fee_eur.into()),
        )
        .tag("base")
        .build()?;

        let seats = LineItem::debit("Active seats")
            .quantity(Quantity::new(Decimal::from(usage.seats), "seats"))
            .unit_price(UnitPrice::new(
                Decimal::from(self.seat_price_eur),
                "EUR/seat",
            ))
            .tag("seat")
            .build()?;

        let free = Decimal::from(self.free_api_calls);
        let total_calls = Decimal::from(usage.api_calls);

        // Graduated: free tier (zero price) + overage tier.
        // Both bands are kept so the document shows what was included vs. charged.
        let api_schedule = TariffSchedule::graduated()
            .unit("calls")
            .currency(Currency::EUR)
            .band(TariffBand::free_up_to(free).with_description(format!(
                "API calls (free tier, first {} incl.)",
                self.free_api_calls
            )))
            .band(
                TariffBand::over(
                    free,
                    // Exact: Amount<6> → Amount<5> only succeeds when the price
                    // needs no more than 5 decimals. It does not here, so a silent
                    // rounding of the overage rate is impossible.
                    Amount::<5>::checked_from_decimal(self.overage_per_call.into_decimal())?,
                )
                .with_description("API calls (overage)"),
            )
            .build()?;

        let api_items: Vec<LineItem> = api_schedule
            .split(total_calls)?
            .into_iter()
            .map(|mut i| {
                i.tags.push("usage".into());
                i
            })
            .collect();

        Ok([base, seats]
            .into_iter()
            .chain(api_items)
            .collect::<Vec<_>>()
            .into())
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![
            // Commission on all positions, min EUR 2.00.
            // Applied before VAT so it's included in the VAT base.
            PercentageCharge::new("Platform commission", dec!(0.03))
                .unwrap()
                .with_min(Amount::parse("2.00000").unwrap())
                .boxed(),
            FixedRateTax::new("VAT", dec!(0.20)).unwrap().boxed(),
        ]
    }
}

/// Render at the two decimals the document was assembled with.
///
/// `Amount<5>` always displays five decimals — the precision is part of the type.
/// Because every amount here fits two, this conversion is **lossless**: it is a
/// change of type, not a rounding, which is exactly what `amount_scale` bought.
fn eur(a: Amount<5>) -> Amount<2> {
    a.checked_round_to::<2>(RoundingStrategy::MidpointAwayFromZero)
        .expect("assembled at two decimals, so this cannot lose a digit")
}

fn main() {
    let tariff = SaasTariff {
        base_fee_eur: 49,
        seat_price_eur: 19,
        free_api_calls: 100_000,
        overage_per_call: Amount::<6>::parse("0.000100").unwrap(), // EUR 0.0001/call
    };
    let usage = SaasUsage {
        seats: 5,
        api_calls: 450_000,
    };

    let meta = DocumentMeta {
        invoice_number: "SaaS-2026-07-001".into(),
        currency: Currency::EUR,
        period_label: "July 2026".into(),
        notes: Some("Includes 350k API overage calls".into()),
        ..Default::default()
    };

    // `amount_scale` assembles every amount at the two decimals an invoice actually
    // carries — and that EN 16931 mandates. Without it a 20 % VAT on 184.37 is
    // 36.874, which is arithmetically right but not a number you can put on an
    // invoice or serialise into XRechnung.
    let doc = BillingDocument::builder()
        .meta(meta)
        .amount_scale(AmountScale::EN16931)
        .tariff(&tariff, &usage)
        .unwrap()
        .build()
        .unwrap();

    println!("=== SaaS Invoice ===");
    println!();
    for pos in doc.net_positions() {
        println!("  {:50} {:>12}", pos.description, eur(pos.net_amount));
    }
    println!();
    println!("  {:50} {:>12}", "NET TOTAL", eur(doc.net_total()));

    // `PercentageCharge` implements `TaxLayer`, so a commercial commission lands in
    // `tax_total` alongside the VAT. It is not a tax — in EN 16931 terms it is a
    // document-level charge (BT-108), not the VAT total (BT-110) — so report the two
    // separately rather than printing a "TAX TOTAL" that bundles them. The
    // `percentage-charge` tag is what separates them.
    let commission: Amount<5> = doc
        .tax_positions()
        .iter()
        .filter(|p| p.has_tag(tags::PERCENTAGE_CHARGE))
        .map(|p| p.net_amount)
        .sum();
    // The VAT total is the sum of the per-category tax amounts — EN 16931 BR-CO-14.
    let vat: Amount<5> = doc.tax_breakdown().iter().map(|e| e.tax_amount).sum();

    for pos in doc
        .tax_positions()
        .iter()
        .filter(|p| p.has_tag(tags::PERCENTAGE_CHARGE))
    {
        println!("  {:50} {:>12}", pos.description, eur(pos.net_amount));
    }
    println!("  {:50} {:>12}", "VAT", eur(vat));
    println!("  {:50} {:>12}", "GROSS TOTAL", eur(doc.gross_total()));

    // The two components account for the whole of `tax_total` — nothing unclassified.
    assert_eq!(commission + vat, doc.tax_total());

    // The VAT breakdown (BG-23): the taxable base and tax per (category, rate).
    // Legally required on the invoice, and impossible to state from a single total.
    println!();
    println!("  VAT breakdown (EN 16931 BG-23):");
    for e in doc.tax_breakdown() {
        println!(
            "    category {} at {:>5}%   base {:>10}   tax {:>10}",
            e.category,
            (e.rate * rust_decimal::Decimal::ONE_HUNDRED).normalize(),
            eur(e.taxable_base),
            eur(e.tax_amount)
        );
    }

    doc.assert_valid();
    // Every amount fits two decimals, so this document can be emitted as EN 16931
    // without a rounding step that would break its own totals.
    assert!(
        doc.fits_amount_scale(2),
        "{:?}",
        doc.amount_scale_violation(2)
    );
    println!();
    println!("✓ Document validation passed");
    println!("✓ Every amount fits EN 16931's two decimals");
}
