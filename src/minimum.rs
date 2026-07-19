//! [`minimum_charge`] — guaranteed minimum spend helper.
use crate::amount::Amount;
use crate::document::BillingDocument;
use crate::error::BillingError;
use crate::line_item::LineItem;

/// Apply a minimum charge if `doc.net_total() < minimum`.
///
/// Returns `Ok(None)` when the minimum is already met.
/// Returns `Ok(Some(shortfall_item))` when the net total falls below the minimum.
/// Returns `Err` on monetary overflow (only possible if `minimum` and `net_total`
/// are both near the `i64` representable limits).
///
/// The returned `LineItem` is tagged `"minimum-charge"`.
///
/// # Usage pattern — settle the minimum *before* taxing
///
/// A shortfall is part of the consideration and is therefore taxable. Compute it
/// against the net positions, add it to them, and let the tax layers see the
/// final base:
///
/// ```rust
/// use billing::{Amount, BillingDocument, Currency, DocumentMeta, FixedRateTax,
///               LineItem, TaxLayer, minimum_charge};
/// use rust_decimal::dec;
///
/// let mut positions = vec![
///     LineItem::fixed("Verbrauch", Amount::parse("100.00000")?).build()?,
/// ];
///
/// // 1. Settle the minimum against the untaxed net.
/// let net_only = BillingDocument::from_positions(
///     DocumentMeta::default(), positions.clone(), vec![], vec![])?;
/// if let Some(shortfall) =
///     minimum_charge(&net_only, Amount::parse("110.00000")?, "Mindestentgelt")?
/// {
///     positions.push(shortfall);
/// }
///
/// // 2. Now build the real document, so VAT applies to the shortfall too.
/// let taxes: Vec<Box<dyn TaxLayer>> = vec![Box::new(FixedRateTax::new("MwSt", dec!(0.19))?)];
/// let doc = BillingDocument::from_positions(
///     DocumentMeta { currency: Currency::EUR, ..Default::default() },
///     positions, taxes, vec![])?;
///
/// assert_eq!(doc.net_total(), Amount::parse("110.00000")?);
/// assert_eq!(doc.tax_total(), Amount::parse("20.90000")?);   // 110 × 19%, not 100 × 19%
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// Appending the shortfall afterwards with
/// [`BillingDocument::with_extra_position`] would leave it **untaxed**, and is
/// refused outright on a document that carries a VAT breakdown.
pub fn minimum_charge(
    doc: &BillingDocument,
    minimum: Amount<5>,
    description: &str,
) -> Result<Option<LineItem>, BillingError> {
    if doc.net_total() >= minimum {
        return Ok(None);
    }
    let shortfall = minimum.checked_sub(doc.net_total())?;
    let item = LineItem::debit(description)
        .fixed_amount(shortfall)
        .tag("minimum-charge")
        .build()?;
    Ok(Some(item))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::document::{BillingDocument, DocumentMeta};
    use crate::line_item::LineItem;

    fn doc(amount: &str) -> BillingDocument {
        let pos = vec![
            LineItem::fixed("Test", Amount::parse(amount).unwrap())
                .build()
                .unwrap(),
        ];
        BillingDocument::from_positions(DocumentMeta::default(), pos, vec![], vec![]).unwrap()
    }

    #[test]
    fn minimum_not_triggered() {
        let d = doc("600.00000");
        assert!(
            minimum_charge(&d, Amount::parse("500.00000").unwrap(), "Min")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn minimum_triggered() {
        let d = doc("200.00000");
        let item = minimum_charge(&d, Amount::parse("500.00000").unwrap(), "Mindestentgelt")
            .unwrap()
            .unwrap();
        assert_eq!(item.net_amount, Amount::parse("300.00000").unwrap());
        assert!(item.has_tag("minimum-charge"));
    }

    #[test]
    fn minimum_at_boundary_not_triggered() {
        let d = doc("500.00000");
        assert!(
            minimum_charge(&d, Amount::parse("500.00000").unwrap(), "Min")
                .unwrap()
                .is_none()
        );
    }
}
