+++
title = "VAT in the accounting currency"
description = "BT-6 and BT-111: why an invoice carries two VAT totals when it is not denominated in the currency the tax is reported in, and the four fatal rules involved."
weight = 80
+++

## Why an invoice carries two VAT totals

The invoice currency (BT-5) is a commercial choice. The currency VAT is *reported* in
is not. Art. 230 of the VAT Directive lets an invoice be denominated in any currency
but requires the VAT amount to be expressed in the national currency of the member
state where the tax is due.

So a Polish, Czech, Swedish, Danish, Hungarian or Romanian supplier invoicing in EUR
must state the VAT again in PLN, CZK, SEK, DKK, HUF or RON. It is not an exotic case —
it is the default whenever a non-euro-area supplier quotes in euro.

EN 16931 models exactly that pair and nothing more: **BT-6** names the currency,
**BT-111** states the amount.

```rust
use billing::prelude::*;
use billing::FixedRateTax;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![LineItem::fixed("Leistung", Amount::parse("1000.00000")?).build()?])
    .extra_tax(FixedRateTax::new("VAT", dec!(0.19))?.boxed())
    .build()?
    .with_vat_accounting_currency(VatAccountingCurrency::converted(
        Currency::new("PLN")?, Amount::parse("190.00000")?, dec!(4.30), AmountScale::EN16931,
    )?)?;

assert_eq!(doc.vat_total()?, Amount::parse("190.00000")?);   // BT-110, in EUR
let acct = doc.vat_accounting_currency().unwrap();
assert_eq!(acct.currency(),  Currency::new("PLN")?);         // BT-6
assert_eq!(acct.vat_total(), Amount::parse("817.00000")?);   // BT-111
assert!(doc.fits_amount_scale(2));                           // BR-DEC-15
doc.assert_valid();
```

## The exchange rate is not a business term

The standard carries the *converted figure*, not the arithmetic that produced it.
There is no BT for the rate, no BT for the rate date, and no rule relating BT-111 to
BT-110 numerically.

That is why `converted()` takes the rate as an argument and keeps none of it, and why
`new()` exists at all — when the figure comes from an ERP that already applied the
official rate, there is nothing for this crate to compute. It holds no rates, no rate
tables and no clock, for the same reason it holds no jurisdiction constants.

## Four fatal rules

| Rule | Source | Requires | How it holds here |
|---|---|---|---|
| **BR-53** | CEN | BT-6 present ⇒ BT-111 provided | they are **one value**, so the pair cannot be half-stated |
| **BR-DEC-15** | CEN | BT-111 at most two decimals | `converted` rounds the exact product once at the target scale; `fits_amount_scale` and `amount_scale_violation` cover BT-111 |
| **`PEPPOL-EN16931-R005`** | Peppol | BT-6 ≠ BT-5 | rejected by `with_vat_accounting_currency`, the first place both are known |
| **`PEPPOL-EN16931-R055`** | Peppol | BT-110 and BT-111 share an operational sign | check 14 of `validate()` |

`PEPPOL-EN16931-R051` is the reason nothing else moves: every other amount in the
document must carry BT-5, and BT-111 is the single documented exception. BT-110,
BT-112 and BT-115 stay in the invoice currency.

`BT-6` also cannot be `XXX`. It is a valid ISO 4217 code that `BR-CL-04` accepts, so
"VAT accounted for in no currency at all" would pass every downstream check; the
constructor refuses it outright.

## Rounded once, at the emission scale

`converted()` takes an `AmountScale` rather than a bare strategy, for the same reason
[`from_gross`](../gross-pricing/) does: computing the product at the engine's five
decimals and reducing afterwards rounds twice, and can land a minor unit away from what
a tax authority recomputes from the same rate.

```rust
use billing::prelude::*;
use rust_decimal::dec;

// 190.00 × 4.3059 = 818.121 exactly.
let once = VatAccountingCurrency::converted(
    Currency::new("PLN")?, Amount::parse("190.00000")?, dec!(4.3059), AmountScale::EN16931,
)?;
assert_eq!(once.vat_total(), Amount::parse("818.12000")?);
assert!(once.vat_total().fits_scale(2));   // BR-DEC-15
```

## Why the sign check lives in `validate()`

`PEPPOL-EN16931-R055` is checked by `validate()` rather than only by the setter,
because `reverse()`, `AllocationRule` and `merge_period_documents` all reach the field
without passing through it. Each does the right thing, and `validate()` is what proves
it rather than assuming it:

- **`reverse()`** negates BT-111 along with BT-110. Carrying it over unnegated would
  leave a credit note whose two VAT figures point in opposite directions.
- **Allocation** splits BT-111 with the same last-recipient remainder that keeps BT-110
  exact, so `Σ(recipient BT-111) == original BT-111`.
- **Merging** sums it, and **refuses** two shapes it cannot sum: a currency mismatch,
  and one half stating BT-111 while the other does not — where the missing figure is
  *unstated*, not zero, and treating it as zero would understate BT-111 on a document
  where `BR-53` makes it mandatory.

"Same operational sign" is a test for strict opposition, not for equal `signum`: zero
is compatible with either direction, which matters on a document whose VAT is zero.
