+++
title = "VAT breakdown (BG-23)"
description = "A single tax total is not a lawful invoice. The per-rate breakdown, the ten UNCL 5305 categories and their exemption rules, and per-position attribution."
weight = 50
+++

## Why a breakdown is not optional

A lump `tax_total` cannot satisfy the law on any invoice carrying more than one rate.
EU VAT Directive 2006/112/EC art. 226(8)–(10) requires the taxable amount **per rate
or exemption** and the tax **per rate**; §14 Abs. 4 Nr. 7–8 UStG says the same. A
reduced-rate line beside a standard-rate line, a reverse-charge position, an exempt
feed-in credit — each needs its own line.

That structure is EN 16931's **BG-23**, and it is what XRechnung, ZUGFeRD/Factur-X and
Peppol BIS all serialise:

| Field | Term | Meaning |
|---|---|---|
| `category` | BT-118 | VAT category code |
| `rate` | BT-119 | VAT category rate |
| `taxable_base` | BT-116 | taxable amount for this category and rate |
| `tax_amount` | BT-117 | tax charged on it |
| `exemption_reason` | BT-120 | exemption reason, free text |
| `exemption_reason_code` | BT-121 | exemption reason, CEF VATEX code |

The engine produces the breakdown. It does **not** decide which category applies —
that is a jurisdictional question and stays with you.

## One line per `(category, rate)`

Entries sharing a category and rate are merged. The requirement comes from two
different places depending on the category, and the asymmetry is deliberate:

| Categories | Rule | Wording |
|---|---|---|
| `S`, `L`, `M` | BR-S-08, BR-AF-08, BR-AG-08 | *"For each different value of VAT category rate … the VAT category taxable amount shall equal the sum of …"* — the equality only holds if entries at that rate are merged |
| `Z`, `E`, `AE`, `K`, `G`, `O` | BR-Z-01 and siblings | *"… shall contain in the VAT breakdown **exactly one** VAT category code equal with …"* |

The taxed categories say *"at least one"* because they can legitimately appear at
several rates; the zero-tax categories say *"exactly one"* because they cannot.

The rate is **normalised** before comparison. Peppol specifies that only significant
decimals count for the VAT rate, so `0.19` and `0.190` are one group. Without
normalising they would produce two breakdown lines for one rate — an invalid invoice.

## The ten categories

`BR-CL-17` and `BR-CL-18` restrict BT-118 and BT-151 to exactly ten UNCL 5305 codes,
and `TaxCategory` is that closed set. It is deliberately **not** `#[non_exhaustive]`:
it mirrors an externally-governed code list, and consumers legitimately need
exhaustive matching when mapping to an output format.

| Code | Meaning | Levies tax | Exemption reason |
|---|---|---|---|
| `S` | standard rate | yes | forbidden |
| `Z` | zero-rated | no | **forbidden** |
| `E` | exempt | no | **required** |
| `AE` | reverse charge | no | required |
| `K` | intra-Community supply | no | required |
| `G` | free export item | no | required |
| `O` | outside the scope of VAT | no | required |
| `L` | Canary Islands IGIC | yes | forbidden |
| `M` | Ceuta and Melilla IPSI | yes | forbidden |
| `B` | split payment (Italy) | **yes** | neither |

Three of those rows are the ones implementers get wrong.

**`Z` and `E` both carry zero tax, but `Z` must *not* have an exemption reason and `E`
must.** Zero-rating and exemption are legally distinct — input tax stays deductible
under `Z` and generally not under `E`. The crate gives them separate constructors
(`FixedRateTax::zero_rated` and `FixedRateTax::exempt`) precisely so the difference is
not an `Option` you can leave `None`.

**`B` is not `AE`.** Both mean "someone else pays", but under split payment the supply
is taxed at the normal rate and the tax **is** stated; only the settlement route
differs. The artefacts make this observable by omission: `B` is the one category with
no `BR-B-05`, no `BR-B-09` and no `BR-B-10`, so nothing constrains its rate, forces
BT-117 to zero, or requires a reason. It is therefore also the only category where
both `requires_exemption_reason()` and `forbids_exemption_reason()` are `false` —
code that assumes exactly one of the two holds is wrong on `B`.

**`S` requires a rate strictly above zero** (`BR-S-05`), while `L` and `M` explicitly
permit zero (`BR-AF-05`, `BR-AG-05`). Treating all taxed categories alike wrongly
rejects a lawful 0 % IGIC line — and a `(S, 0 %)` breakdown group is *unsatisfiable*
rather than merely unusual, because `BR-S-08` defines its base as the sum of lines
that `BR-S-05` forbids to exist.

### `O` and the rate that must be absent

`O` is the only category where a line, allowance or charge must **not state its rate
at all**. The others say the rate *"shall be 0 (zero)"* — present, and zero. `BR-O-05`
says the opposite: an invoice line with category `O` *shall not contain* an invoiced
item VAT rate. `BR-O-06` and `BR-O-07` say the same for BT-96 and BT-103.

`LineVat::rate` is a plain `Decimal`, so an `O` position stores `0`. A consumer
emitting UBL or CII must **suppress the element** rather than write
`<cbc:Percent>0</cbc:Percent>`, which is a fatal violation. `TaxCategory::states_rate()`
is that instruction in code.

It does **not** apply to BT-119. The breakdown rate is a different term governed by
different rules, no `BR-O` rule suppresses it, and XRechnung's `BR-DE-14` requires it
unconditionally — fatal, with no category exception. Suppressing BT-119 for `O` on the
strength of `BR-O-05` is a validation failure, not a safe simplification.

## Exemption reasons: text *or* code

`BR-E-10`, `BR-AE-10`, `BR-IC-10`, `BR-G-10` and `BR-O-10` each require *"a VAT
exemption reason code (BT-121) **or** a VAT exemption reason text (BT-120)"*. They are
alternatives. A caller holding only the machine-readable code — the one a validator
can actually check against `BR-CL-22` — should not have to invent prose:

```rust
use billing::{FixedRateTax, TaxCategory, TaxLayer};

let ic = FixedRateTax::exempt_coded(
    "Innergemeinschaftliche Lieferung",
    TaxCategory::IntraCommunity,
    "VATEX-EU-IC",
)?;
let entry = ic.breakdown(&[])?.unwrap();
assert_eq!(entry.exemption_reason_code.as_deref(), Some("VATEX-EU-IC"));
assert_eq!(entry.exemption_reason, None);
```

Conversely `BR-S-10`, `BR-Z-10`, `BR-AF-10` and `BR-AG-10` forbid **both** forms, so
checking only the text would let a VATEX code through. The engine checks both.

The crate does not validate VATEX membership: it carries no copy of the list, and a
stale embedded copy would be worse than none. The same reasoning applies to UNCL 5189,
UNCL 7161, UN/ECE Rec 20/21 and UNTDID 1001.

## Per-position attribution

EN 16931 wants the category and rate on every position too — BT-151/152 on a line
(`BR-CO-04`), BT-95/96 on an allowance (`BR-32`), BT-102/103 on a charge (`BR-37`) —
and `BR-S-08` checks the breakdown against them. The engine fills this in during
assembly from the layer whose base the position falls in:

```rust
use billing::prelude::*;
use rust_decimal::dec;

let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .amount_scale(AmountScale::EN16931)
    .positions(vec![
        LineItem::fixed("Beratung", Amount::parse("400.00000")?).tag("full").build()?,
        LineItem::fixed("Fachbuch", Amount::parse("100.00000")?).tag("reduced").build()?,
    ])
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("full").boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.07))?.with_tag("reduced").boxed())
    .build()?;

assert_eq!(doc.net_positions()[0].vat.unwrap().rate, dec!(0.19));
assert_eq!(doc.net_positions()[1].vat.unwrap().rate, dec!(0.07));
doc.verify_vat_attribution()?;
```

### Two checks, not one

`verify_vat_attribution()` checks that the attributions which **exist** agree with the
breakdown — `BR-S-08`, plus `BR-CO-14` (`BT-110 = Σ BT-117`) and the `BR-O-12/13/14`
family. It is deliberately not part of `validate()`, because
[allocation](../allocation/) cannot preserve it: splitting the positions and the
breakdown with independent penny corrections can leave the two a minor unit apart
while both remain internally exact.

It also passes **vacuously** on a document with no attributions at all. That is the
right behaviour — this crate bills things that are not invoices — but it means passing
it is not on its own a licence to emit. `unattributed_positions()` answers the other
half:

```rust
use billing::prelude::*;
use rust_decimal::dec;

// The VAT layer is restricted to "energy", so nothing attributes a category to
// the levy — which is a BG-21 charge, and BR-37 makes one mandatory.
let doc = BillingDocument::builder()
    .currency(Currency::EUR)
    .positions(vec![LineItem::for_usage(
        "Arbeit",
        Quantity::new(dec!(1000), "kWh"),
        UnitPrice::new(dec!(0.30), "EUR/kWh"),
    ).tag("energy").build()?])
    .extra_tax(PerUnitLevy::new("Stromsteuer", Amount::parse("0.02050")?, "kWh")?.boxed())
    .extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("energy").boxed())
    .build()?;

doc.verify_vat_attribution()?;                       // consistent …
assert_eq!(doc.unattributed_positions().count(), 1); // … but not yet emittable
```

Fix it by declaring the charge's own treatment — `PerUnitLevy::with_vat`,
`PercentageCharge::with_vat`, `PercentageDiscount::with_vat`, `FixedDiscount::with_vat`
— or by widening the VAT layer's base so it covers the charge.

## `BR-CO-17`, and why it has a tolerance

`BT-117 = BT-116 × (BT-119 / 100)`, rounded to two decimals. The engine checks it with
EN 16931's own tolerance rather than exact equality, because the CEN reference
Schematron asserts `|BT-117 − base × rate| < 1.00` and the slack is genuinely needed:
merging two entries sums amounts that were each rounded, so `Σ(base_i × rate)` and
`(Σ base_i) × rate` can legitimately differ in the last place.

When assembling at an `AmountScale`, the engine reduces the base **first** and then
derives the tax from the *reduced* base in a single rounding of the exact product —
which is what a validator recomputes. Rounding an already-rounded tax answers a
different question and can differ by a whole minor unit.
