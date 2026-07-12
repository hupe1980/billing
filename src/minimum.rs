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
/// # Usage pattern
///
/// ```rust
/// # use billing::{BillingDocument, DocumentMeta, Amount, minimum_charge};
/// # let mut doc = BillingDocument::from_positions(
/// #     DocumentMeta::default(), vec![], vec![], vec![]).unwrap();
/// let minimum = Amount::parse("500.00000").unwrap();
/// if let Some(shortfall) = minimum_charge(&doc, minimum, "Mindestentgelt").unwrap() {
///     doc = doc.with_extra_position(shortfall).unwrap();
/// }
/// ```
pub fn minimum_charge(
    doc: &BillingDocument,
    minimum: Amount<5>,
    description: &str,
) -> Result<Option<LineItem>, BillingError> {
    if doc.net_total() >= minimum {
        return Ok(None);
    }
    let shortfall = minimum.checked_sub(doc.net_total())?;
    Ok(Some(
        LineItem::debit(description)
            .fixed_amount(shortfall)
            .tag("minimum-charge")
            .build()
            .expect("minimum_charge LineItem cannot fail: fixed_amount is always set"),
    ))
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
