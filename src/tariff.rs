//! [`Tariff`] trait — the primary extension point for domain-specific billing logic.
use crate::document::{BillingDocument, BillingDocumentBuilder, DocumentMeta};
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::tax::{DiscountLayer, TaxLayer};

/// Implement this trait to adapt any domain to the billing engine.
///
/// # Design
///
/// - `line_items` is a **pure function**: no I/O, no clock access, no mutation.
/// - Tax and discount layers declared here are applied by the document builder
///   in order. Tax ordering is significant (e.g. Stromsteuer before MwSt).
/// - The separation between pricing (`line_items`) and taxes (`tax_layers`)
///   mirrors real-world invoicing: net amount and tax calculation are
///   independently auditable.
///
/// # Example — SaaS platform
///
/// ```rust
/// use billing::{Tariff, LineItem, Amount, TaxLayer, DiscountLayer};
/// use billing::tax::FixedRateTax;
/// use billing::document::DocumentMeta;
/// use rust_decimal::dec;
///
/// struct PlatformTariff { monthly_fee_eur: u32 }
///
/// impl Tariff for PlatformTariff {
///     type Usage = ();
///     type Error = std::convert::Infallible;
///
///     fn line_items(&self, _: &()) -> Result<Vec<LineItem>, Self::Error> {
///         Ok(vec![
///             LineItem::fixed("Monthly platform fee",
///                 Amount::<5>::from_int(self.monthly_fee_eur as i64)
///             ).build().unwrap(),
///         ])
///     }
///
///     fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
///         vec![Box::new(FixedRateTax::new("VAT", dec!(0.20)).unwrap())]
///     }
/// }
/// ```
pub trait Tariff {
    /// Domain-specific usage input.
    type Usage;
    /// Domain-specific error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Generate billing positions from usage data.  Must be a **pure function**.
    fn line_items(&self, usage: &Self::Usage) -> Result<Vec<LineItem>, Self::Error>;

    /// Tax / surcharge / percentage-charge layers applied after positions.
    ///
    /// Return an ordered `Vec` — sequence determines compound-tax bases
    /// (e.g. Stromsteuer BEFORE MwSt so Stromsteuer is in the MwSt base).
    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![]
    }

    /// Discount layers applied before tax (reduce the taxable base).
    fn discount_layers(&self) -> Vec<Box<dyn DiscountLayer>> {
        vec![]
    }

    /// Convenience: compute a [`BillingDocument`] from usage data.
    ///
    /// Equivalent to:
    /// ```rust,ignore
    /// BillingDocument::builder()
    ///     .meta(meta)
    ///     .tariff(self, usage)?
    ///     .build()?
    /// ```
    fn bill(&self, meta: DocumentMeta, usage: &Self::Usage) -> Result<BillingDocument, BillingError>
    where
        Self::Error: Into<BillingError>,
        Self: Sized,
    {
        BillingDocumentBuilder::default()
            .meta(meta)
            .tariff(self, usage)?
            .build()
    }
}
