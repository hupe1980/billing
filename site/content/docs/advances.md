+++
title = "Advance payments and final invoices"
description = "Settling instalments two lawful ways: deduct the advances with their tax, or bill only the remainder — and why getting it wrong makes the same VAT payable twice."
weight = 100
+++

## The problem

Bill a customer in instalments and you eventually issue a document that settles the
lot. Two things must be true at once:

- the **taxable base covers the whole supply**, because that is what was supplied and
  what output tax is owed on; and
- the **amount payable is only the remainder**, because the rest is already in the bank.

[`with_prepaid`](../totals/#settlement-bt-113-bt-114-bt-115) expresses the second half —
it is BT-113, a single flat figure. What BT-113 cannot express is the **tax contained
in each advance**, and several jurisdictions require exactly that on the settling
document.

Germany is the sharpest case. §14 Abs. 5 Satz 2 UStG requires a final invoice to deduct
the advance amounts *"und die auf sie entfallenden Steuerbeträge"* — and their tax.
Omit it and, per UStAE 14.8 Abs. 10, the issuer owes the full tax shown **plus** the
advance-related portion again under §14c Abs. 1: the same tax, billed twice.

## `AdvancePayment`

`AdvancePayment` carries that missing structure. It mirrors the ZUGFeRD / Factur-X
EXTENDED group `SpecifiedAdvancePayment` (BG-X-45), the one standardised place where
per-advance tax data has a home:

| This crate | ZUGFeRD EXTENDED | Meaning |
|---|---|---|
| `gross()` | BT-X-291 | amount received |
| `received_on()` | BT-X-292 | date of receipt |
| `tax()` | BG-X-46 | tax contained, per category and rate |
| `reference()` | BT-X-558 / BT-25 | the advance invoice's number |
| `reference_date()` | BT-X-560 / BT-26 | its issue date |

The gross is **derived** as `Σ(base + tax)` rather than stored, so it cannot drift from
its components. An advance with no stated tax is rejected: ZUGFeRD makes the equivalent
group mandatory `1..n` for the same reason, and an advance whose tax is unknown cannot
be deducted lawfully.

## Two lawful shapes

### Settle by deduction

Invoice the full supply and deduct the advances with their tax. Totals and the VAT
breakdown keep describing the whole supply; only `amount_due()` shrinks.

```rust
use billing::prelude::*;
use billing::{AdvancePayment, FixedRateTax};
use rust_decimal::dec;

// Whole supply: 1000.00 net + 19 % VAT.
let doc = BillingDocument::from_positions(
    DocumentMeta { currency: Currency::EUR, ..Default::default() },
    vec![LineItem::fixed("Jahresverbrauch", Amount::parse("1000.00000")?).build()?],
    vec![FixedRateTax::new("MwSt", dec!(0.19))?.boxed()],
    vec![],
)?;

// Two advances already invoiced and paid: 375.00 net + 71.25 VAT each.
let advance = |n: &str| AdvancePayment::new(vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("375.00000").unwrap(), Amount::parse("71.25000").unwrap(),
)]).unwrap().with_reference(n);

let doc = doc.with_advances(vec![advance("AB-1"), advance("AB-2")])?;

// The base still describes the whole supply …
assert_eq!(doc.tax_breakdown()[0].taxable_base, Amount::parse("1000.00000")?);
assert_eq!(doc.gross_total(), Amount::parse("1190.00000")?);
// … while only the remainder is payable.
assert_eq!(doc.prepaid(),            Amount::parse("892.50000")?);
assert_eq!(doc.advance_tax_total()?, Amount::parse("142.50000")?);
assert_eq!(doc.amount_due()?,        Amount::parse("297.50000")?);
```

`advance_deductions()` merges the advances into one line per `(category, rate)` — the
deduction table a final invoice presents, next to the VAT breakdown that still
describes the full supply.

### Settle by residual

Invoice only what is left, and do not list the advances at all.

```rust
use billing::prelude::*;
use billing::{advance::residual_breakdown, AdvancePayment};
use rust_decimal::dec;

let full = vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("1000.00000")?, Amount::parse("190.00000")?,
)];
let advances = vec![AdvancePayment::new(vec![TaxBreakdownEntry::new(
    TaxCategory::Standard, dec!(0.19),
    Amount::parse("750.00000")?, Amount::parse("142.50000")?,
)])?];

let residual = residual_breakdown(&full, &advances)?;
assert_eq!(residual[0].taxable_base, Amount::parse("250.00000")?);
assert_eq!(residual[0].tax_amount,   Amount::parse("47.50000")?);
```

Groups reduced to zero base *and* zero tax are dropped — a breakdown line for nothing
is noise on an invoice.

## Prefer the residual form for e-invoices

How urgent that advice is depends entirely on the target format:

| Target | Where the per-advance tax goes |
|---|---|
| ZUGFeRD / Factur-X **EXTENDED** | `BG-X-46`, the only place it fits |
| ZUGFeRD **BASIC** / **EN 16931**, XRechnung, Peppol BIS | **nowhere** |

A consumer targeting XRechnung or Peppol BIS will emit `prepaid()` as BT-113 and
**silently drop the tax attached to each advance** — which is precisely the §14c
Abs. 1 double-taxation scenario above. Against those formats a residual invoice is not
a preference but the only correct construction, and it is what the German BMF
recommends for structured e-invoices (Schreiben v. 15.10.2024, Rn. 48).

## One value, not two fields

`prepaid` and itemised advances are the *same fact* at different resolutions, so they
are one enum rather than two fields:

```rust
use billing::prelude::*;

assert_eq!(Prepayment::default(), Prepayment::None);
assert_eq!(Prepayment::None.total()?, Amount::<5>::ZERO);

let flat = Prepayment::total_of(Amount::parse("900.00000")?)?;
assert_eq!(flat.total()?, Amount::parse("900.00000")?);
assert!(flat.advances().is_empty());
```

"A total of 900 alongside advances summing to 476" is therefore not a state that can be
written down — no runtime check is needed to reject it. `with_prepaid` and
`with_advances` are thin wrappers over `with_prepayment`, and each replaces the whole
prepayment rather than merging into it.

## What the engine refuses

- an advance covering a `(category, rate)` group the supply does not contain — it
  cannot have taxed something that was not invoiced;
- advances exceeding the supply in any group — over-deduction understates output tax;
- a negative base or tax on an advance: it passes every other check (`BR-CO-17` holds
  for `(−100, 0.19, −19)`) yet means "a payment already received of less than nothing",
  and left unchecked it produced a negative BT-113 and an `amount_due` *larger* than the
  gross;
- two entries for the same group within one advance;
- **allocating or merging** a document that carries itemised advances. Each advance
  references a specific invoice issued to a specific recipient; slicing that across N
  recipients produces deduction tables matching no document anyone was sent. Refusing
  beats the silent alternative of dropping them and re-billing money already collected.

## Document type codes

There is **no** "final invoice" or "residual invoice" code in UNTDID 1001 outside
construction: both are `380`. What distinguishes them is the *amounts* — whether
advances are deducted — not the type code. Use `advances()` to tell them apart, and the
construction codes (`875`, `876`, `877`) when the project is a construction project.

`386` (prepayment invoice) is semantically the best fit for an advance-payment request
but sits outside XRechnung's recommended set (`BR-DE-17`), which offers no prepayment
code; German practice uses `380` or `326`. Peppol accepts `386`.
