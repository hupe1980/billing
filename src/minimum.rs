//! [`minimum_charge`] and [`minimum_charge_gross`] — guaranteed minimum spend.
//!
//! # Two functions, because contracts are written both ways
//!
//! A B2B supply agreement states a *Mindestumsatz* **netto**; a consumer supply
//! contract states a *Mindestbetrag* **brutto**, because that is the figure on the
//! price sheet and the one §1 PAngV makes the seller quote. The two are not the
//! same arithmetic, and picking the wrong one is silent:
//!
//! | Contract says | Use | Compares against |
//! |---|---|---|
//! | "Mindestumsatz 100,00 € netto" | [`minimum_charge`] | [`BillingDocument::net_total`] |
//! | "Mindestbetrag 9,90 € brutto" | [`minimum_charge_gross`] | [`BillingDocument::gross_total`] (BT-112) |
//!
//! Deriving the second from the first by hand is where the money goes missing —
//! see [`minimum_charge_gross`] for the mixed-rate case that motivated it.
use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::currency::Currency;
use crate::document::BillingDocument;
use crate::error::BillingError;
use crate::line_item::{LineItem, LineItemBuilder};
use crate::vat::LineVat;

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
    Ok(Some(
        shortfall_line(description, shortfall, currency).build()?,
    ))
}

/// The shortfall position both helpers return: a complete invoice line carrying
/// the reserved [`crate::tags::MINIMUM_CHARGE`] tag.
fn shortfall_line(description: &str, net: Amount<5>, currency: Currency) -> LineItemBuilder {
    LineItem::flat_fee(description, net, currency).tag(crate::tags::MINIMUM_CHARGE)
}

/// Apply a minimum charge stated **gross** — `doc.gross_total() < minimum_gross`.
///
/// The consumer-contract counterpart of [`minimum_charge`]. A *Mindestbetrag* is
/// quoted inclusive of VAT, because that is the figure §1 PAngV makes the seller
/// show, while EN 16931's totals chain is net — so the top-up enters the document as
/// a net line:
///
/// ```text
/// net top-up = (BT-112 target − BT-112 so far) ÷ (1 + rate)
/// ```
///
/// # The rate is an argument, and it is enforced
///
/// A minimum charge is a **contractual** charge, so the contract names its rate. It
/// is not derivable from the document: a Stadtwerk bill carrying 7 % Trinkwasser
/// beside 19 % Energie has two, and "use the standard one" fails the moment the
/// top-up is taxed at the other — silently, and not by a rounding unit. Grossing a
/// 5,38 € shortfall down at 19 % while it is taxed at 7 % invoices **9,36 €** against
/// a 9,90 € minimum: 0,54 € short, 5,5 %, with every total reconciling and
/// [`BillingDocument::validate`] passing.
///
/// So the returned line carries the [`LineVat`] you name, and the claim is checked:
/// a layer covering the shortfall at a different (category, rate) fails assembly with
/// [`BillingError::LayerError`] (BR-S-08 — one VAT group per position), and no layer
/// covering it at all is what [`BillingDocument::verify_vat_attribution`] reports.
///
/// A zero rate needs no special case: under reverse charge the divisor is `1`, so
/// gross and net coincide.
///
/// # The document must already be taxed
///
/// [`minimum_charge`] settles against an *untaxed* provisional document, because a
/// net minimum does not depend on the layers. A gross minimum does.
///
/// ```rust
/// use billing::prelude::*;
/// use billing::{FixedRateTax, TaxCategory, TaxLayer};
/// use rust_decimal::dec;
///
/// let positions = vec![
///     LineItem::fixed("Trinkwasser", Amount::parse("2.00000")?).tag("reduced").build()?,
///     LineItem::fixed("Grundpreis",  Amount::parse("2.00000")?).tag("full").build()?,
/// ];
/// // Rebuilt per document: a `Box<dyn TaxLayer>` is consumed by assembly.
/// let layers = || -> Result<Vec<Box<dyn TaxLayer>>, BillingError> {
///     Ok(vec![
///         FixedRateTax::new("MwSt 7 %",  dec!(0.07))?.with_tag("reduced").boxed(),
///         FixedRateTax::new("MwSt 19 %", dec!(0.19))?.with_tag("full").boxed(),
///     ])
/// };
/// let meta = || DocumentMeta { currency: Currency::EUR, ..Default::default() };
///
/// let provisional = BillingDocument::from_positions(meta(), positions.clone(), layers()?, vec![])?;
/// assert_eq!(provisional.gross_total(), Amount::parse("4.52000")?);
///
/// let mut positions = positions;
/// if let Some(mut top_up) = minimum_charge_gross(
///     &provisional,
///     Amount::parse("9.90000")?,
///     LineVat::new(TaxCategory::Standard, dec!(0.19))?,   // the rate the contract names
///     "Mindestbetrag",
///     Currency::EUR,
/// )? {
///     top_up.tags.push("full".into());   // so the 19 % layer really covers it
///     positions.push(top_up);
/// }
///
/// let doc = BillingDocument::from_positions(meta(), positions, layers()?, vec![])?;
/// assert_eq!(doc.gross_total(), Amount::parse("9.90000")?);
/// doc.verify_vat_attribution()?;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
///
/// The net top-up is one rounding of an exact quotient, so BT-112 lands within a
/// rounding unit of the target rather than always exactly on it. Assert
/// `doc.gross_total() >= minimum_gross` if the contract reads as a hard floor.
///
/// # Errors
/// - [`BillingError::MonetaryOverflow`] if the shortfall or the quotient leaves the
///   representable range.
/// - [`BillingError::InvalidInput`] from [`LineItem::flat_fee`]'s own checks.
pub fn minimum_charge_gross(
    doc: &BillingDocument,
    minimum_gross: Amount<5>,
    vat: LineVat,
    description: &str,
    currency: Currency,
) -> Result<Option<LineItem>, BillingError> {
    if doc.gross_total() >= minimum_gross {
        return Ok(None);
    }
    let shortfall_gross = minimum_gross.checked_sub(doc.gross_total())?;
    // `1 + rate` is at least 1 — `LineVat::new` refuses a negative rate — so the
    // division is always defined, and degenerates to the identity at a zero rate.
    // `checked_add`: `Decimal`'s `+` panics on overflow rather than saturating.
    let divisor = Decimal::ONE
        .checked_add(vat.rate)
        .ok_or(BillingError::MonetaryOverflow {
            precision: 5,
            input_value: Some(vat.rate),
        })?;
    // One rounding of the exact quotient, at the engine's five decimals.
    let net = shortfall_gross.checked_div(divisor)?;
    Ok(Some(
        shortfall_line(description, net, currency)
            .vat(vat)
            .build()?,
    ))
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

    /// The shortfall is an invoice line (BG-25), so BR-22 / BR-23 / BR-26 apply to
    /// it. Building it with `LineItem::fixed` states BT-131 and leaves BT-129,
    /// BT-130 and BT-146 unset — three fatal rules on a position the *engine itself*
    /// produced, and nothing in the totals notices, because a missing quantity
    /// changes no amount.
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

    // ── minimum_charge_gross ─────────────────────────────────────────────────

    use crate::tax::{FixedRateTax, TaxLayer};
    use crate::vat::{LineVat, TaxCategory};
    use rust_decimal::dec;

    fn meta() -> DocumentMeta {
        DocumentMeta {
            currency: Currency::EUR,
            ..Default::default()
        }
    }

    /// A Stadtwerk bill: 7 % Trinkwasser beside 19 % Energie.
    fn mixed_positions() -> Vec<LineItem> {
        vec![
            LineItem::fixed("Trinkwasser", Amount::parse("2.00000").unwrap())
                .tag("reduced")
                .build()
                .unwrap(),
            LineItem::fixed("Grundpreis", Amount::parse("2.00000").unwrap())
                .tag("full")
                .build()
                .unwrap(),
        ]
    }

    fn mixed_layers() -> Vec<Box<dyn TaxLayer>> {
        vec![
            FixedRateTax::new("MwSt 7", dec!(0.07))
                .unwrap()
                .with_tag("reduced")
                .boxed(),
            FixedRateTax::new("MwSt 19", dec!(0.19))
                .unwrap()
                .with_tag("full")
                .boxed(),
        ]
    }

    #[test]
    fn a_gross_minimum_is_met_exactly_when_the_named_rate_is_the_one_applied() {
        let positions = mixed_positions();
        let provisional =
            BillingDocument::from_positions(meta(), positions.clone(), mixed_layers(), vec![])
                .unwrap();
        assert_eq!(provisional.gross_total(), Amount::parse("4.52000").unwrap());

        let mut top_up = minimum_charge_gross(
            &provisional,
            Amount::parse("9.90000").unwrap(),
            LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap(),
            "Mindestbetrag",
            Currency::EUR,
        )
        .unwrap()
        .unwrap();
        assert!(top_up.has_tag(crate::tags::MINIMUM_CHARGE));
        // 5.38 / 1.19 = 4.5210084…
        assert_eq!(top_up.net_amount, Amount::parse("4.52101").unwrap());

        top_up.tags.push("full".into());
        let mut positions = positions;
        positions.push(top_up);
        let doc =
            BillingDocument::from_positions(meta(), positions, mixed_layers(), vec![]).unwrap();
        assert_eq!(doc.gross_total(), Amount::parse("9.90000").unwrap());
        doc.verify_vat_attribution().unwrap();
    }

    /// The mistake this function exists to make impossible, as a build-time error.
    ///
    /// Grossing the shortfall down at 19 % while the top-up is actually taxed at
    /// 7 % lands 0,54 € short of a 9,90 € minimum — 5,5 % — with every total
    /// reconciling and `validate()` passing. The stated [`LineVat`] makes the two
    /// layers contradict each other instead.
    #[test]
    fn naming_one_rate_and_being_taxed_at_another_is_an_error_not_a_shortfall() {
        let positions = mixed_positions();
        let provisional =
            BillingDocument::from_positions(meta(), positions.clone(), mixed_layers(), vec![])
                .unwrap();

        let mut top_up = minimum_charge_gross(
            &provisional,
            Amount::parse("9.90000").unwrap(),
            LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap(),
            "Mindestbetrag",
            Currency::EUR,
        )
        .unwrap()
        .unwrap();
        // The mistake: tagged into the 7 % layer, priced for 19 %.
        top_up.tags.push("reduced".into());
        let mut positions = positions;
        positions.push(top_up);

        let err =
            BillingDocument::from_positions(meta(), positions, mixed_layers(), vec![]).unwrap_err();
        assert!(
            matches!(err, BillingError::LayerError { .. }),
            "expected a layer conflict, got {err}"
        );
        assert!(err.to_string().contains("exactly one VAT group"), "{err}");
    }

    /// The other half of the enforcement: a shortfall no layer covers keeps an
    /// attribution the breakdown does not contain.
    #[test]
    fn a_shortfall_no_layer_taxes_is_reported_by_verify_vat_attribution() {
        let positions = mixed_positions();
        let provisional =
            BillingDocument::from_positions(meta(), positions.clone(), mixed_layers(), vec![])
                .unwrap();
        let top_up = minimum_charge_gross(
            &provisional,
            Amount::parse("9.90000").unwrap(),
            LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap(),
            "Mindestbetrag",
            Currency::EUR,
        )
        .unwrap()
        .unwrap();
        // Neither "full" nor "reduced": no layer's base includes it.
        let mut positions = positions;
        positions.push(top_up);

        let doc =
            BillingDocument::from_positions(meta(), positions, mixed_layers(), vec![]).unwrap();
        // The totals still reconcile …
        doc.validate().unwrap();
        // … and the attribution check is what notices.
        assert!(doc.verify_vat_attribution().is_err());
        assert!(doc.gross_total() < Amount::parse("9.90000").unwrap());
    }

    /// Under reverse charge or a zero-rated supply the divisor is 1, so the gross
    /// minimum *is* the net minimum. No special case, no branch.
    #[test]
    fn a_zero_rate_makes_gross_and_net_coincide() {
        let doc = BillingDocument::from_positions(
            meta(),
            vec![
                LineItem::fixed("Leistung", Amount::parse("40.00000").unwrap())
                    .build()
                    .unwrap(),
            ],
            vec![],
            vec![],
        )
        .unwrap();
        let top_up = minimum_charge_gross(
            &doc,
            Amount::parse("100.00000").unwrap(),
            LineVat::new(TaxCategory::ReverseCharge, Decimal::ZERO).unwrap(),
            "Mindestbetrag",
            Currency::EUR,
        )
        .unwrap()
        .unwrap();
        assert_eq!(top_up.net_amount, Amount::parse("60.00000").unwrap());
        assert_eq!(
            top_up.vat.unwrap().category,
            crate::vat::TaxCategory::ReverseCharge
        );
    }

    #[test]
    fn a_gross_minimum_already_met_adds_nothing() {
        let doc =
            BillingDocument::from_positions(meta(), mixed_positions(), mixed_layers(), vec![])
                .unwrap();
        assert!(
            minimum_charge_gross(
                &doc,
                Amount::parse("4.52000").unwrap(),
                LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap(),
                "Mindestbetrag",
                Currency::EUR,
            )
            .unwrap()
            .is_none()
        );
    }

    /// The gross variant compares BT-112 and the net variant BT-106 − BT-107, so on
    /// a taxed document they disagree — which is the entire point.
    #[test]
    fn the_net_and_gross_helpers_answer_different_questions() {
        let doc =
            BillingDocument::from_positions(meta(), mixed_positions(), mixed_layers(), vec![])
                .unwrap();
        let minimum = Amount::<5>::parse("4.52000").unwrap();
        // Gross is already 4.52, so nothing is owed …
        assert!(
            minimum_charge_gross(
                &doc,
                minimum,
                LineVat::new(TaxCategory::Standard, dec!(0.19)).unwrap(),
                "m",
                Currency::EUR
            )
            .unwrap()
            .is_none()
        );
        // … while the net is 4.00 and the net helper still tops it up.
        assert_eq!(
            minimum_charge(&doc, minimum, "m", Currency::EUR)
                .unwrap()
                .unwrap()
                .net_amount,
            Amount::parse("0.52000").unwrap()
        );
    }
}
