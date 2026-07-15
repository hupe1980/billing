//! SaaS subscription + metered usage billing example.
//!
//! Demonstrates:
//! - `Tariff` trait for domain-specific billing logic
//! - `BillingDocument::builder().tariff(t, u)?` for ergonomic document creation
//! - Graduated pricing for API calls with a free tier (zero-price band)
//! - `PercentageCharge` for platform commission with a minimum floor
//! - `FixedRateTax` for VAT

use billing::prelude::*;
use rust_decimal::Decimal;
use rust_decimal_macros::dec;

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

    fn line_items(&self, usage: &SaasUsage) -> Result<Vec<LineItem>, BillingError> {
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
            .band(TariffBand::free_up_to(free).with_description(format!(
                "API calls (free tier, first {} incl.)",
                self.free_api_calls
            )))
            .band(
                TariffBand::over(
                    free,
                    Amount::<5>::try_from_decimal(self.overage_per_call.into_decimal()).map_err(
                        |_| BillingError::InvalidInput {
                            reason: "invalid overage price".into(),
                        },
                    )?,
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

        Ok([base, seats].into_iter().chain(api_items).collect())
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![
            // Commission on all positions, min EUR 2.00.
            // Applied before VAT so it's included in the VAT base.
            Box::new(
                PercentageCharge::new("Platform commission", dec!(0.03))
                    .with_min(Amount::parse("2.00000").unwrap()),
            ),
            Box::new(FixedRateTax::new("VAT", dec!(0.20))),
        ]
    }
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
        period_label: "July 2026".into(),
        notes: Some("Includes 350k API overage calls".into()),
        ..Default::default()
    };

    let doc = BillingDocument::builder()
        .meta(meta)
        .tariff(&tariff, &usage)
        .unwrap()
        .build()
        .unwrap();

    println!("=== SaaS Invoice ===");
    println!();
    for pos in doc.all_positions() {
        println!("  {:50} {:>12}", pos.description, pos.net_amount);
    }
    println!();
    println!("  {:50} {:>12}", "NET TOTAL", doc.net_total());
    println!("  {:50} {:>12}", "TAX TOTAL", doc.tax_total());
    println!("  {:50} {:>12}", "GROSS TOTAL", doc.gross_total());

    doc.assert_valid();
    println!();
    println!("✓ Document validation passed");
}
