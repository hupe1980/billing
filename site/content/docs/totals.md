+++
title = "The totals chain"
description = "BT-106 to BT-115 mapped onto the crate's accessors — including the two places where the obvious accessor is the wrong one, and the fourteen invariants a document checks."
weight = 60
+++

## The chain

EN 16931 builds the totals in a fixed order, and each step has its own business rule.
This is the mapping:

| Term | Meaning | Accessor | Rule |
|---|---|---|---|
| **BT-106** | sum of invoice line net amounts | `line_total()` | `BR-CO-10` |
| **BT-107** | sum of document level allowances | `−discount_total()` | `BR-CO-11` |
| **BT-108** | sum of document level charges | `charge_total()` | `BR-CO-12` |
| **BT-109** | total without VAT | `taxable_total()` | `BR-CO-13` |
| **BT-110** | total VAT amount | `vat_total()` | `BR-CO-14` |
| **BT-112** | total with VAT | `gross_total()` | `BR-CO-15` |
| **BT-113** | paid amount | `prepaid()` | — |
| **BT-114** | rounding amount | `rounding()` | — |
| **BT-115** | amount due for payment | `amount_due()` | `BR-CO-16` |

`BR-CO-13` is the one to internalise:

```text
BT-109 = BT-106 − BT-107 + BT-108
```

## Two accessors that are not what they look like

### `net_total()` is not BT-109

`net_total()` is `BT-106 − BT-107` — the lines plus the discounts, discounts already
being negative. It stops one term short.

A document level **charge** — a per-unit levy, a commission — is produced by a
`TaxLayer` and therefore lands in `tax_positions`, even though EN 16931 counts it
inside the taxable base. The two are equal only on a document with no charges.
`taxable_total()` is BT-109 in general.

### `tax_total()` is not BT-110

`tax_total()` is the sum of everything a `TaxLayer` produced, which mixes two things
EN 16931 keeps completely apart:

| Position | EN 16931 | Contributes to |
|---|---|---|
| value added tax | BG-23 | **BT-110**, the VAT total |
| levy, commission, surcharge | BG-21 | **BT-108** → BT-109, the *taxable base* |

Mapping the whole of `tax_total()` to BT-110 makes `BR-CO-14` (`BT-110 = Σ BT-117`)
fail on every document carrying a levy or a commission.

The split is **decidable**, not heuristic: the engine marks every tax position whose
layer contributed a VAT breakdown entry, taking that from the layer's own
`breakdown()` return value. So it is as accurate for a third-party `TaxLayer` as for a
built-in one.

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ).build()?])
    .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.boxed())
    .build()?;

assert_eq!(doc.line_total()?,    Amount::parse("300.00000")?);  // BT-106
assert_eq!(doc.net_total(),      Amount::parse("300.00000")?);  // BT-106 − BT-107
assert_eq!(doc.charge_total()?,  Amount::parse("20.50000")?);   // BT-108 — the levy
assert_eq!(doc.taxable_total()?, Amount::parse("320.50000")?);  // BT-109
assert_eq!(doc.vat_total()?,     Amount::parse("60.89500")?);   // BT-110 — VAT alone
assert_eq!(doc.tax_total(),      Amount::parse("81.39500")?);   // both together

// BR-CO-15 pairs BT-109 with BT-110 — not net_total with tax_total.
assert_eq!(doc.taxable_total()?.checked_add(doc.vat_total()?)?, doc.gross_total());
```

`vat_positions()` and `charge_positions()` give the same split as iterators, for
rendering.

## Settlement: BT-113, BT-114, BT-115

```text
BT-115 = BT-112 − BT-113 + BT-114
```

**BT-113, the paid amount.** Record advance payments, deposits and part payments with
`with_prepaid()` or `with_prepayment()`. This reduces `amount_due()` and deliberately
leaves the totals and the VAT breakdown untouched: the supply was made in full and
output VAT is owed on the full base.

> **Never model prepayments as negative line items or as discounts.** That shrinks the
> taxable base and under-declares output tax. In Germany, failing to deduct advances
> correctly on an Endrechnung makes the entire VAT amount payable a second time under
> §14c Abs. 1 UStG. See [Advance payments](../advances/).

**BT-114, the rounding amount.** `with_cash_rounding()` rounds `gross − prepaid` to a
tenderable multiple and stores the difference. Totals and the breakdown are not
modified: in every jurisdiction surveyed except Switzerland the rounding difference
lies outside the taxable base. Apply it only to a cash tender — rounding a card
payment is affirmatively unlawful in Denmark.

The rule itself is retained, not just its output, so that a later change to `prepaid`
recomputes the adjustment instead of leaving a stale, non-tenderable figure.

**BT-115 may legitimately be negative** when prepayments exceed the gross total — the
ordinary utility credit-balance case, where the supplier owes the customer a refund.
It is deliberately not clamped to zero.

## `BR-CO-25` does not exist

It is widely cited — *"if the amount due is positive, either the payment due date
(BT-9) or the payment terms (BT-20) shall be present"* — and it is not in the
published CEN artefacts. They run `BR-CO-20` … `BR-CO-24` and then `BR-CO-26`; the
rule was dropped before publication.

The engine therefore does not check it, and this page mentions it only because an
earlier draft of the crate nearly shipped a validation for a rule that no validator
enforces. If your target profile reinstates it, check it in your own mapping layer.

## The fourteen invariants

`validate()` re-derives all of these from the positions, exactly, with no tolerance.
`assert_valid()` is the panicking form for tests. Deserialisation runs `validate()`
too, so untrusted JSON cannot introduce a document whose stored totals disagree with
its own lines.

| # | Invariant |
|---|---|
| 1 | `Σ(net_positions + discount_positions) == net_total` |
| 2 | `Σ(tax_positions) == tax_total` |
| 3 | `net_total + tax_total == gross_total` |
| 4 | `Σ(discount_positions) == discount_total` |
| 5 | every breakdown entry is category-consistent, satisfies `BR-CO-17`, and no `(category, rate)` group repeats |
| 6 | `prepaid >= 0` |
| 7 | `rounding` matches the recorded cash-rounding rule |
| 8 | `Σ(tax_breakdown)` is a component of `tax_total` — same sign, no larger |
| 9 | no discount position opposes the document's own direction |
| 10 | `prepaid` equals `prepayment.total()` |
| 11 | every position satisfies `LineItem::validate` |
| 12 | VAT charged ⇔ a VAT breakdown exists (`BR-CO-18`) |
| 13 | an `O` breakdown group is the only group (`BR-O-11`) |
| 14 | BT-6 differs from BT-5, and BT-111 shares BT-110's sign (`PEPPOL-EN16931-R005` / `R055`) |

Check 8 is a *component* test rather than equality on purpose: a non-VAT layer adds to
`tax_total` without contributing a breakdown entry, so exact equality would be wrong —
but the breakdown can never exceed the total or oppose its sign. Without it, a document
declaring 19.00 of output VAT while charging no tax at all deserialised cleanly.

Check 9 is relative to the document's own direction, not to zero. On a credit note
every amount is negated, so its allowances are positive; an absolute "discounts ≤ 0"
rule made `reverse()` produce a document that failed its own `assert_valid()` whenever
the original had any discount.

Two identities are deliberately **outside** this list because
[allocation](../allocation/) cannot preserve them:
`verify_vat_attribution()` and `unattributed_positions()`. Run both on the document
you are about to emit.

## Credit notes

`reverse()` negates every monetary value — positions, totals, the VAT breakdown (base
and tax), and BT-111 — and flips each position's sign so debits become credits.
Quantities are **not** negated: the reversal of "1000 kWh × 0.30" is "1000 kWh ×
−0.30", and a negative quantity is rejected.

The document type code is set for you, because getting it wrong is fatal rather than
cosmetic. `BR-CL-01` reads as one rule but is really **two code lists selected by the
syntax element**: 50 codes for `cbc:InvoiceTypeCode`, 13 for `cbc:CreditNoteTypeCode`.
They share exactly one code (`81`). `380` is only in the first and `381` only in the
second, so putting `381` on a UBL `<Invoice>` — or `380` on a `<CreditNote>` — is a
fatal violation before any profile gets a say.

`DocumentKind::is_credit_note()` tells you which element to emit.
`is_peppol_billing_code()` narrows further: Peppol BIS Billing cuts the invoice list
from 50 codes to 26, and `389` (self-billed) falls in the gap — valid EN 16931, fatal
under the Billing customization, because self-billing is a *different profile*.

Settlement figures do not carry over to a reversal. Negating them would produce a
negative BT-113, which is meaningless and which check 6 rejects — and zero is
arithmetically correct anyway: if an invoice of gross 1190 had 900 prepaid, the
customer paid 1190 in total, so the credit note's amount due is the full −1190 refund.
