//! [`minimum_charge`] — guaranteed minimum spend helper.
use crate::amount::Amount;
use crate::currency::Currency;
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
/// # The shortfall is a complete EN 16931 invoice line
///
/// A shortfall is an ordinary invoice line (BG-25), not a document level charge,
/// so **every** line rule applies to it — and three of them are fatal and have
/// nothing to do with the amount:
///
/// | Rule | Requires |
/// |---|---|
/// | BR-22 | Invoiced quantity (BT-129) |
/// | BR-23 | Invoiced quantity unit of measure code (BT-130) |
/// | BR-26 | Item net price (BT-146) |
///
/// This helper therefore builds the position with [`LineItem::flat_fee`] — one
/// unit of [`crate::UNIT_CODE_ONE`] at a price equal to the whole shortfall —
/// rather than with [`LineItem::fixed`], which states an amount and nothing else.
/// `1 × shortfall` is exactly `shortfall`, so `PEPPOL-EN16931-R120` holds
/// trivially and no rounding is introduced.
///
/// That is why `currency` is an argument: the generated BT-146 label is
/// `"EUR"` / `"CHF"` / … and a generated label cannot invent the currency. Pass
/// [`Currency::XXX`] where it genuinely does not matter; it renders as `XXX`,
/// which reads as the bug it is.
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
/// if let Some(shortfall) = minimum_charge(
///     &net_only, Amount::parse("110.00000")?, "Mindestentgelt", Currency::EUR)?
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
    currency: Currency,
) -> Result<Option<LineItem>, BillingError> {
    if doc.net_total() >= minimum {
        return Ok(None);
    }
    let shortfall = minimum.checked_sub(doc.net_total())?;
    let item = LineItem::flat_fee(description, shortfall, currency)
        .tag(crate::tags::MINIMUM_CHARGE)
        .build()?;
    Ok(Some(item))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::amount::Amount;
    use crate::document::{BillingDocument, DocumentMeta};
    use crate::line_item::LineItem;
    use rust_decimal::Decimal;

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
            minimum_charge(
                &d,
                Amount::parse("500.00000").unwrap(),
                "Min",
                Currency::EUR
            )
            .unwrap()
            .is_none()
        );
    }

    #[test]
    fn minimum_triggered() {
        let d = doc("200.00000");
        let item = minimum_charge(
            &d,
            Amount::parse("500.00000").unwrap(),
            "Mindestentgelt",
            Currency::EUR,
        )
        .unwrap()
        .unwrap();
        assert_eq!(item.net_amount, Amount::parse("300.00000").unwrap());
        assert!(item.has_tag("minimum-charge"));
    }

    #[test]
    fn minimum_at_boundary_not_triggered() {
        let d = doc("500.00000");
        assert!(
            minimum_charge(
                &d,
                Amount::parse("500.00000").unwrap(),
                "Min",
                Currency::EUR
            )
            .unwrap()
            .is_none()
        );
    }

    /// The shortfall is an invoice line (BG-25), so BR-22 / BR-23 / BR-26 apply
    /// to it. Building it with `LineItem::fixed` — which is what this helper did
    /// until 0.13.0 — states BT-131 and leaves BT-129, BT-130 and BT-146 unset,
    /// which is three fatal rules on a position the *engine itself* produced.
    /// Nothing in the totals notices, because a missing quantity changes no
    /// amount.
    #[test]
    fn the_shortfall_is_a_complete_en16931_invoice_line() {
        let d = doc("200.00000");
        let item = minimum_charge(
            &d,
            Amount::parse("500.00000").unwrap(),
            "Mindestentgelt",
            Currency::EUR,
        )
        .unwrap()
        .unwrap();

        let qty = item.quantity.as_ref().expect("BR-22 wants BT-129");
        assert_eq!(qty.value, Decimal::ONE);
        assert_eq!(qty.code.as_deref(), Some(crate::UNIT_CODE_ONE)); // BR-23
        let price = item.unit_price.as_ref().expect("BR-26 wants BT-146");
        assert_eq!(price.value, Decimal::from(300u32));
        assert_eq!(price.unit, "EUR"); // not a hardcoded currency
        // 1 x 300.00 is exactly 300.00, so R120 holds with nothing rounded.
        assert_eq!(item.net_amount, Amount::parse("300.00000").unwrap());
        item.validate().unwrap();
    }

    #[test]
    fn the_generated_price_label_follows_the_currency_it_was_given() {
        let d = doc("0.00000");
        let item = minimum_charge(&d, Amount::parse("10.00000").unwrap(), "Min", Currency::CHF)
            .unwrap()
            .unwrap();
        assert_eq!(item.unit_price.as_ref().unwrap().unit, "CHF");
    }
}
