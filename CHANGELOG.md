# Changelog

All notable changes to this crate are documented here.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Versioning policy

**Pre-1.0, a minor bump (`0.x`) may break the API.** That is what the leading zero
means under semver, and this crate uses it. In exchange:

1. **Every breaking change is listed** under a `### Changed — breaking` or
   `### Removed` heading, naming the old and new spelling. A signature change is
   never merely implied by a feature entry.
2. **Semantics changes are called out even when the signature is unchanged.** A
   method that keeps its type but changes what it returns for the same input is
   the most dangerous kind of change, because it compiles. These are marked
   **⚠️ silent** — they will not fail your build.
3. **Every release carries a migration table** (old → new) for mechanical updates.
4. **No deprecation shims.** A removed name is gone rather than left as a
   deprecated alias, so the compiler shows you every affected site in one pass
   instead of a warning you can accumulate. Migrations are hard cuts by design.
5. `BillingError` is `#[non_exhaustive]`: **new variants may appear in a minor
   release** and are not treated as breaking. Always include a `_ =>` arm.
6. `cargo-semver-checks` runs in CI, so an unintended API break fails the build
   rather than shipping.

Pin exactly (`billing = "=0.10.0"`) if you need to schedule migrations yourself;
`"0.10"` will pick up `0.10.x` patch fixes only.

## [0.10.0]

Driven by a second review pass over the same primary artefacts as 0.9.0 — the CEN
artefacts (`ConnectingEurope/eInvoicing-EN16931` at `validation-1.3.16`) and the
Peppol BIS Billing 3.0 Schematron — this time working through **Annex A** of
EN 16931-1, whose eight worked examples 0.9.0's review had not covered.

Two of those eight exist specifically to demonstrate business terms this crate had
no field for. Auditing the rest of BG-25 against the standard's own subgroup list
then turned up a second, larger gap: of BG-25's four subgroups, **two** had no
representation at all. BG-26 (line period) and BG-30 (line VAT) were modelled;
BG-27 / BG-28 (line allowances and charges) and BG-29 (price details) were not.
Both are closed here, and every part of both is fatal-rule territory in Peppol.

The immediate consequence is that `PEPPOL-EN16931-R120` —

> BT-131 = BT-129 × (BT-146 ÷ BT-149) + Σ BG-28 − Σ BG-27

— is now expressible in full. Three of its five terms did not exist in 0.9.0, and
`LineItemBuilder::build` computed only `BT-129 × BT-146`.

### Migration

| 0.9.0 | 0.10.0 |
|---|---|
| `UnitPrice { value, unit }` struct literal | use `UnitPrice::new`, or add `base_quantity: None, base_quantity_code: None, gross_price: None, price_discount: None` |
| `LineItem { .. }` struct literal | add `line_allowances: vec![]` |
| pre-dividing to state "EUR 12,00 per 100" as `0.12` | `UnitPrice::new(dec!(12.00), …).per(dec!(100))` |
| a line-level discount modelled as a document level `AllowanceCharge` position | `.line_allowance(LineAllowanceCharge::allowance(amount, reason))` |
| `LineItem::fixed` for a standing charge on an emitted invoice line | `LineItem::flat_fee` — see BR-22 / BR-23 / BR-26 below |
| a line discount modelled as an allowance when it moved the *price* | `UnitPrice::discounted(gross, discount, unit)` |

### Added — EN 16931 BG-27 / BG-28 line allowances and charges

`LineAllowanceCharge` (with `AllowanceKind`) models the group the crate was missing
entirely — BT-136 … BT-140 for an allowance, BT-141 … BT-145 for a charge —
attached to a position through `LineItem::line_allowances` and
`LineItemBuilder::line_allowance`.

**This is a different group from the existing `AllowanceCharge`, which was already
correct but is easy to mistake for it.** The three now-modelled allowance concepts
differ in what they move, which is the only distinction that matters
arithmetically:

| Group | Type | Terms | Moves |
|---|---|---|---|
| BG-27 / BG-28 | `LineAllowanceCharge` *(new)* | BT-136 … BT-145 | **BT-131**, one line's net amount |
| BG-20 / BG-21 | `AllowanceCharge` | BT-92 … BT-105 | **BT-107 / BT-108** → BT-109 |
| BG-29 | `UnitPrice::discounted` *(new)* | BT-147 / BT-148 | **BT-146**, the price |

- `build` folds them into `net_amount`, completing `R120`. Because BT-106 is the
  sum of the BT-131s, the totals chain and the VAT breakdown need no special case
  — and VAT falls on the reduced base, which is the point of a line allowance.
- **BR-42 / BR-44**, restated by **BR-CO-23 / BR-CO-24**, are enforced as a type
  invariant: a line allowance needs a reason (BT-139 / BT-144) or a reason code
  (BT-140 / BT-145). A *document* level allowance can lean on the position's
  `description` for BT-97 / BT-104; a line allowance has no description of its own,
  so the constructors take the reason.
- **`PEPPOL-EN16931-R040` / `R041` / `R042` apply here identically.** Their
  Schematron contexts list `ubl-invoice:Invoice/cac:InvoiceLine/cac:AllowanceCharge`
  alongside the document level element, so the base-and-percentage pairing and the
  ±0.02 recomputation are the same rules — the checks are shared with
  `AllowanceCharge` rather than reimplemented.
- `scaled` and `BillingDocument::reverse` move line allowances with the line, so
  the stated parts never contradict BT-131 after proration, allocation or a Storno.

### Added — EN 16931 BG-29 PRICE DETAILS

`UnitPrice` now carries all five business terms of the subgroup rather than only
the mandatory one:

| BT | Name | Field | Rules |
|---|---|---|---|
| BT-146 | Item net price | `value` (unchanged) | BR-26, BR-27 |
| BT-147 | Item price discount | `price_discount` | `PEPPOL-EN16931-R046` |
| BT-148 | Item gross price | `gross_price` | BR-28, `R046` |
| BT-149 | Item price base quantity | `base_quantity` | `R120`, `R121` |
| BT-150 | …its unit of measure code | `base_quantity_code` | BR-CL-23, `R130` |

- **`UnitPrice::per(base_quantity)` — BT-149 / BT-150.** EN 16931-1 Annex A.1.3
  (*Example 2 — Item price base quantity*) is an entire worked example about the
  ordinary "EUR 12,00 per 100 pieces" quote. Pre-dividing to EUR 0,12 loses the
  term, states a BT-146 the seller never quoted — which is what a human reads off
  the rendered invoice — and, for a price that does not divide evenly, injects a
  rounding error into BT-146 that `R120`'s ±0.02 then has to absorb. The base
  quantity is also load-bearing arithmetic rather than decoration:
  `PEPPOL-EN16931-R120` computes `BT-131 = BT-129 × (BT-146 ÷ BT-149) + BG-28 −
  BG-27`, so `LineItemBuilder::build` now divides by it. `None` means 1, exactly
  as `R120`'s own `$baseQuantity` variable is defined (including its treatment of
  zero). `UnitPrice::per_unit_value()` exposes the quotient for display.
- **`UnitPrice::discounted(gross, discount, unit)` — BT-147 / BT-148.** Annex A.1.6
  (*Example 5 — Negative Invoice line*) uses this on every line: gross `9,50` less
  discount `1,00` gives net `8,50`. This is **not** `AllowanceCharge`, which is
  BG-27 / BG-28 — those move BT-131 and the VAT base, while BT-147 / BT-148 sit
  inside BG-29 and move the *price*, leaving BT-131 alone. Peppol keeps them apart
  too: `PEPPOL-EN16931-R044` forbids a *charge* at price level outright while
  allowing the discount. BT-146 is **derived** rather than accepted, because
  `PEPPOL-EN16931-R046` is an *exact* equality — unlike `R040` it carries no
  `u:slack` — so a caller cannot compute the net price and be a cent out.
- **`UnitPrice::validate`**, run by `LineItemBuilder::build`, `LineItem::validate`
  and deserialisation: `R046`, `R121` (base quantity strictly above zero — it is a
  divisor), and the two half-stated pairs (BT-147 without BT-148, which the
  standard defines as a subtraction *from* BT-148; BT-150 without BT-149, which is
  unrepresentable in UBL where it is an attribute of `cbc:BaseQuantity`).
- **`PEPPOL-EN16931-R130` is enforced** — **fatal**, and a rule the prose-only
  reading of BT-150 misses:

  > `[PEPPOL-EN16931-R130]` Unit code of price base quantity MUST be same as
  > invoiced quantity.

  A cross-field rule: `Quantity` holds BT-130 and `UnitPrice` holds BT-150, so
  neither type can check it and `LineItem` does. Only when both codes are stated —
  supplying one is not a contradiction, and this crate does not invent BT-130 from
  a display label.

BR-27 and BR-28 (BT-146 / BT-148 shall not be negative) are deliberately **not**
enforced, consistent with this crate's existing acceptance of negative unit prices
for spot markets (EPEX negative-price hours, §27 EEG 2023). This is now stated
explicitly in the type docs rather than left implicit.

### Added — BR-22 / BR-23 / BR-26 on a flat charge

- **`LineItem::flat_fee` / `credit_flat_fee`, and `UNIT_CODE_ONE`.** Every invoice
  line needs an invoiced quantity (BR-22, BT-129), its unit code (BR-23, BT-130)
  and an item net price (BR-26, BT-146) — all three fatal, all three with
  `$Invoice_Line` as their Schematron context, so there is no exception for a line
  that has no natural quantity. `LineItem::fixed` states an amount and nothing
  else, leaving a consumer to synthesise all three from a position that does not
  say how.

  `flat_fee` states the same money as the line the standard asks for: quantity `1`,
  unit code `C62` (UN/ECE Rec 20 "one"), item net price the full amount. `1 × amount`
  is exactly `amount`, so `R120` holds trivially and nothing is rounded that was
  not rounded before.

  `fixed` is unchanged and still correct where the position is *not* an invoice
  line — a document level allowance or charge has no BG-25 terms at all — and now
  documents what it does not supply.

### Changed — one derivation of BT-131 instead of two

Not a behaviour change; a structural one, and the reason the two silent bugs below
were possible.

`LineItemBuilder::build` and `BillingDocument::reduce_position` both evaluate
`R120`, at different precisions — the builder at the engine's five decimals, the
reducer at the interchange scale. They were separate implementations of the same
formula, so adding a term to one and not the other compiled, passed every test, and
silently degraded rounding. They now share `line_item::compose_bt131`, parameterised
by how each leaf is reduced, so a new term is written once.

`tests/properties.rs` additionally asserts over 2 048 generated lines that reducing
a line equals rounding the exact `R120` expression once — re-derived in the test,
independently of both call sites. Its generators favour non-terminating quotients
(base quantities of 3, 7, 100, 1000 against six-decimal prices), which is where
single and double rounding actually diverge. The check was verified to fail when
the drift is deliberately reintroduced, and the shrunk counterexample from that run
is committed as a proptest seed.

### Added — profile layering above `BR-CL-01`

- **`DocumentKind::is_peppol_billing_code()`.** Passing `BR-CL-01` says nothing
  about whether a Peppol Access Point will take the document: the CEN lists hold
  50 and 13 codes, `PEPPOL-EN16931-P0100` and `P0101` hold 26 and five. Exactly one
  kind modelled here falls in the gap — `SelfBilledInvoice` (`389`) is in
  `BR-CL-01`'s invoice list but **not** in `P0100`, because self-billing is a
  separate Peppol profile with its own `CustomizationID`. Emitting it under the
  Billing customization is fatal. This was not previously documented anywhere.
- **`DocumentKind::requires_german_parties()`.** `PEPPOL-EN16931-P0112` restricts
  `326` and `384` to invoices where **both** parties are German organisations. So
  the admissible BT-3 set is not merely profile-dependent but *party*-dependent —
  which is why `DocumentKind` stays a plain code list here and the narrowing lives
  in the layer above, which knows both.

### Fixed — precision and structure

These three were introduced by the BG-29 and BG-27 / BG-28 work above and found by
re-auditing it, so they never shipped. They are listed because two of them are
**silent** — the kind that produces a plausible wrong number rather than an error.

- **⚠️ Interchange-scale reduction ignored BT-149 and the line allowances.**
  `reduce_position` reconstructs a line as `BT-129 × BT-146` to decide whether it
  may round once from the exact product — the crate's headline precision
  guarantee. Adding a price base quantity and line allowances changed what a line
  *is* without changing that reconstruction, so any line using them no longer
  matched, silently fell back to reducing the already-rounded five-decimal amount,
  and rounded twice. It now reconstructs `R120` in full.

  Not academic: `1 × (0,014997 ÷ 3)` is exactly `0,004999`. Rounded once to two
  decimals that is `0,00`; rounded to five first it becomes `0,00500` and then
  `0,01` — a whole minor unit, from an amount that looks perfectly ordinary either
  way. This is the same defect class the crate's precision section was written
  about, reintroduced through a new field.

- **⚠️ `fits_amount_scale` could not see BT-136 / BT-137 / BT-141 / BT-142.**
  BR-DEC-24 and BR-DEC-27 cap the line allowance and charge amounts at two
  decimals, and BR-DEC-25 / BR-DEC-28 their base amounts — each in its own right,
  not merely as components of BT-131. A document could therefore report
  `fits_amount_scale(2) == true` while carrying a BT-137 with three decimals that
  a validator rejects. `amount_scale_violation` now names them, and scale
  reduction reduces them alongside the line total so the emitted parts still sum
  to the emitted BT-131.

- **A position could be a document level allowance *and* carry line allowances.**
  BG-27 / BG-28 are children of an invoice line (BG-25); BG-20 / BG-21 are children
  of the document. UBL says the same structurally — only `cac:InvoiceLine` nests
  `cac:AllowanceCharge`. Combining them described a document no syntax can express,
  and is now rejected by `LineItem::validate` and the builder.

### Fixed — documentation

- **`BR-CL-01`'s two lists were described as "disjoint".** They are not: they share
  exactly one code, `81` (credit note related to goods or services). They *are*
  disjoint on `380` / `381`, which is the part that matters and the part the 0.9.0
  fix turned on, but "disjoint" as written is false. The crate now says
  "two lists selected by the syntax element", gives both sizes, quotes the
  Schematron, and names the single overlap. Corrected in `DocumentKind::is_credit_note`,
  `BillingDocument::reverse`, the README and the 0.9.0 changelog entry.

  For the avoidance of doubt, since this is easy to misread: `BR-CL-01`'s *context*
  matches both elements, but its *test* is a disjunction over `self::`, so it does
  select per element. `381` on a `cbc:InvoiceTypeCode` fails `BR-CL-01` itself — no
  profile required.

- **BG-27 / BG-28 were described as covered by `AllowanceCharge`.** They are not,
  and never were: `AllowanceCharge` carries BT-92 … BT-105, which is BG-20 / BG-21
  at *document* level. BG-27 / BG-28 are BT-136 … BT-145 at *line* level, bound to
  a different abstract Schematron context (`$Invoice_line_allowances` versus
  `$Document_level_allowances`) and mandated by different rules (BR-41 … BR-44
  versus BR-31 … BR-38). The types are now both present and each says plainly what
  it moves.

- **Undocumented panic in `UnitPrice::discounted` and `UnitPrice::rounded`.**
  `gross - discount` uses `Decimal`'s `Sub`, which panics on overflow rather than
  returning. That is consistent with this crate's "overflow is visible, never
  silent" rule for infallible operators, and matches `Amount`'s own `Sub`, but it
  was not stated. Both now carry a `# Panics` section. Only reachable with operands
  within one of each other of `Decimal::MAX` (~7.9 × 10²⁸).

### Changed — breaking

- `UnitPrice` gained four fields and `LineItem` gained `line_allowances`.
  Struct-literal construction of either breaks; `UnitPrice::new` is unaffected and
  still means "this price, per one unit", and `line_allowances` defaults to empty
  on every builder path.
- `UnitPrice` now validates on deserialisation (`#[serde(try_from)]`), so JSON that
  was accepted before and would have produced a fatal `R046` / `R121` violation is
  now rejected at the boundary.
- `LineItemBuilder::build` and `LineItem::validate` can return `InvalidInput` for
  the BG-29 and BG-27 / BG-28 rules above. A line built without any of them is
  unaffected — `base_quantity: None` divides by nothing and an empty
  `line_allowances` sums to nothing.
- `LineItem::scaled` now scales `line_allowances` too. Documents built before this
  release carry none, so nothing changes for them.

## [0.9.0]

Driven by a review of every EN 16931 claim in this crate's documentation against
the primary validation artefacts — the CEN artefacts
(`ConnectingEurope/eInvoicing-EN16931` at `validation-1.3.16`), the Peppol BIS
Billing 3.0 Schematron, and the KoSIT XRechnung rules. Several citations here were
wrong, one VAT category was missing, and the per-position data an EN 16931 mapper
needs was not observable after assembly. All of it is fixed, and the fixes are
verified against those same artefacts rather than against secondary sources.

Two changes are **breaking without being visible at the call site** — see ⚠️.

### Migration

| 0.8.0 | 0.9.0 |
|---|---|
| `LineItem { .. }` struct literal | add `vat: None, allowance_charge: None` |
| `Quantity { value, unit }` struct literal | add `code: None`, or use `Quantity::new` |
| `FixedRateTax::new(name, dec!(0))` | `FixedRateTax::zero_rated(name)` — see ⚠️ below |
| two untagged `FixedRateTax` layers | tag them apart, or they now error — see ⚠️ below |
| `doc.tax_total()` mapped to BT-110 | `doc.vat_total()?` (BT-110) and `doc.charge_total()?` (BT-108) |
| `doc.net_total()` mapped to BT-109 | `doc.taxable_total()?` — see **Fixed — modelling** |
| `doc.net_total()` mapped to BT-106 | `doc.line_total()?` |
| `TaxBreakdownEntry { .. }` struct literal | add `exemption_reason_code: None` |
| `item.reason_code` | `item.allowance_charge.as_ref().and_then(\|a\| a.reason_code.as_deref())` |
| `LineItemBuilder::reason_code(c)` | `.allowance_charge(AllowanceCharge::coded(c))` |
| `TaxLayer::reason_code` / `DiscountLayer::reason_code` impls | `allowance_charge() -> Option<AllowanceCharge>` |
| `reverse(meta)` relying on `meta.kind` | BT-3 is forced to a credit-note code |
| manual `fits_amount_scale` + `checked_round_to` | `Amount::exact_to::<2>()` |

### Fixed — modelling

- **A document with any discount could not be reversed.** `reverse()` negates
  every amount, so a credit note's allowances are positive — but `validate()`
  check 9 tested "discount positions <= 0" against zero rather than against the
  document's own direction. `inv.reverse(..)` therefore returned a document that
  failed its own `assert_valid()` whenever the original carried a discount, which
  no test had ever combined. The check is now relative to the sign of BT-106, and
  still rejects a genuine surcharge in either direction.
- **`reverse()` produced a credit note carrying BT-3 = `380`.** It never touched
  `DocumentMeta::kind`, and the idiomatic `..Default::default()` supplies
  `CommercialInvoice` — so the documented way to build a Storno yielded negative
  totals under an *invoice* type code. `BR-CL-01` is not one code list but **two,
  selected by the syntax element** (`cbc:InvoiceTypeCode` vs
  `cbc:CreditNoteTypeCode`) — its Schematron context matches both while its test
  branches on which one it found — and `380` appears only in the first while `381`
  appears only in the second, so the result was valid as neither document.
  <sup>Corrected in 0.10.0: the two lists (50 and 13 codes) are not *disjoint* —
  they share `81`. They are disjoint on `380` / `381`, which is what the fix turns
  on.</sup>
  `reverse()` now forces a credit-note code, keeping an explicit one if given.
  `DocumentKind::is_credit_note` is documented as *the* signal for which element
  to emit, and `DocumentKind::ALL` was added.
- **Percentage allowances and charges discarded their base and their rate.**
  `PercentageDiscount` and `PercentageCharge` compute `base × rate` and kept only
  the product, so a consumer could emit BT-92 / BT-99 but never BT-93 / BT-94 or
  BT-100 / BT-101. Peppol makes those a matched pair in both directions, both
  **fatal** — `PEPPOL-EN16931-R041` ("base amount MUST be provided when percentage
  is provided") and `R042` (the converse). `LineItem::reason_code` is replaced by
  `LineItem::allowance_charge: Option<AllowanceCharge>`, which carries the reason
  code together with the base and percentage, populates them automatically, and
  enforces the pairing as a type invariant. A charge whose min/max guard clamped
  the result states neither, because the pair would no longer reproduce the
  amount.
- **`LineItemBuilder::build` did not validate the allowance/charge detail** — only
  `LineItem::validate` did, so a half-stated percentage basis could reach a
  document through the builder. It is checked in both places now, after the net
  amount is final so the R040 arithmetic can be checked too.
- **The allowance/charge basis did not survive any amount-changing transform.**
  `PEPPOL-EN16931-R040` is not a presence rule — it recomputes `amount = base ×
  percentage / 100` with a ±0.02 tolerance, **fatal**. Three paths broke it:
  `LineItem::scaled` (allocation, proration) left BT-93 / BT-100 at the unscaled
  value; `reverse()` negated the amount but not the base; and the penny correction
  in allocation adjusts one line's amount alone. The first two now transform the
  base with the amount; the third drops the basis and keeps the reason code, since
  no basis is always valid. `AllowanceCharge::check_amount` enforces R040, and
  `LineItem::validate` runs it — so any future transform that forgets fails loudly
  in every existing test rather than silently producing an invoice Peppol rejects.
- **BT-93 / BT-100 escaped the decimal caps.** `BR-DEC-02` and `BR-DEC-06` cap the
  allowance and charge *base* amounts at two decimals in their own right.
  `AmountScale` did not reduce them and `fits_amount_scale` did not check them, so
  a document could pass the scale check while carrying a five-decimal BT-93.
- **`net_total` was labelled BT-106 / BT-109; it is neither.** BR-CO-13 defines
  `BT-109 = BT-106 − BT-107 + BT-108`, and a document level *charge* (a per-unit
  levy, a commission) is produced by a `TaxLayer`, so it lands in `tax_positions`
  even though EN 16931 counts it inside the taxable base. `net_total` is therefore
  `BT-106 − BT-107`, and equals BT-109 only on a document with no charges — i.e.
  never, on the levy-bearing utility invoices this crate exists for. Added
  **`line_total()` (BT-106)** and **`taxable_total()` (BT-109)**, documented
  `discount_total()` as `−BT-107`, and corrected the labels in
  `amount_scale_violation`, which previously named the wrong BT in its diagnostic.
- **BT-121 (exemption reason **code**) could not be expressed.** BR-E-10,
  BR-AE-10, BR-IC-10, BR-G-10 and BR-O-10 each accept "a VAT exemption reason code
  (BT-121) **or** a VAT exemption reason text (BT-120)" — the crate modelled only
  the text and *required* it, forcing a caller who held a VATEX code to invent
  prose. Added `TaxBreakdownEntry::exemption_reason_code`,
  `with_exemption_reason_code`, `has_exemption_reason`,
  `FixedRateTax::exempt_coded` and `FixedRateTax::with_exemption_reason_code`.
  Symmetrically, BR-S-10 / BR-Z-10 / BR-AF-10 / BR-AG-10 forbid *both* forms, so
  the prohibition now covers the code as well — checking only the text let a VATEX
  code through on a standard-rated group.
- **`merge_period_documents` could return a document that fails `validate()`.**
  It builds via `from_raw`, which performs no checks, and two individually lawful
  halves do not always compose into a lawful whole. It now re-validates its
  result, as `AllocationRule` always has.

### Added

- **`TaxCategory::states_rate()`** — `O` is the only category that must not state
  a VAT rate at all (BR-O-05 / BR-O-06 / BR-O-07 say the element "shall not
  contain" it, where every other zero-tax category says it "shall be 0"). Because
  `rate` is a plain `Decimal`, an `O` position stores `0`; this predicate tells a
  serialiser to suppress the element rather than emit `0`, which is fatal.
- **`validate()` check 13 — BR-O-11**: an `O` breakdown group must be the only
  group. `merge_period_documents` can otherwise manufacture the forbidden
  combination from two lawful halves.
- **`verify_vat_attribution` also checks BR-CO-14 and BR-O-12/13/14** — `BT-110 =
  Σ BT-117` exactly, and no non-`O` position on an `O` document. `validate()` can
  only assert the weaker "component of `tax_total`", because it must also hold for
  an allocated document.
- **VAT charged on VAT is rejected at assembly.** A VAT breakdown group is not an
  invoice line, a charge or an allowance, so it cannot appear in another group's
  BT-116 under BR-S-08 — the construction has no EN 16931 representation. A levy
  or commission compounding into a VAT base is unaffected: those are BG-21
  charges, which is the legitimate version of the same shape.
- **`TaxCategory::SplitPayment` (`B`)** — Italian *scissione dei pagamenti*.
  `TaxCategory` had nine of the ten codes `BR-CL-17` / `BR-CL-18` permit, so
  `from_code("B")` returned `None` and a lawful Italian invoice could not be
  represented at all. Unlike `AE`, `B` **carries tax**: the artefacts contain no
  `BR-B-09` forcing BT-117 to zero, no `BR-B-05` constraining its rate, and no
  `BR-B-10` requiring an exemption reason — making it the one category where
  `requires_exemption_reason()` and `forbids_exemption_reason()` are *both* false.
  `TaxCategory::ALL` enumerates the full code list.
- **`tags::VAT`** — applied by the engine to any tax position whose layer returned
  a VAT breakdown entry. This makes the value-added-tax / document-level-charge
  split **decidable** instead of a guess over `LEVY` / `PERCENTAGE_CHARGE` tags,
  and it is total: a third-party `TaxLayer` is classified from its own `breakdown`
  return value, exactly like a built-in one.
- **`BillingDocument::vat_total` (BT-110), `charge_total` (BT-108),
  `vat_positions`, `charge_positions`.** Mapping the whole of `tax_total` to
  BT-110 — the obvious thing to do — breaks **BR-CO-14** (`BT-110 = Σ BT-117`) on
  every document carrying a levy.
- **Per-position VAT attribution.** `LineItem::vat: Option<LineVat>` carries
  BT-151/BT-152 on a line, BT-95/BT-96 on an allowance, BT-102/BT-103 on a charge —
  all mandatory under BR-CO-04, BR-32 and BR-37. The engine derives it during
  assembly from the new `TaxLayer::covers`, which `FixedRateTax` implements with
  the *same* predicate its taxable base uses, so the two cannot drift.
  `LineItemBuilder::vat` sets it explicitly.
- **`BillingDocument::verify_vat_attribution`** — checks the breakdown against that
  attribution (**BR-S-08** and siblings). Deliberately outside `validate()`:
  `AllocationRule` cannot preserve it, because it splits positions and breakdown
  with independent penny corrections.
- **`LineItem::reason_code`** — BT-98 (UNCL 5189) / BT-105 (UNCL 7161), with
  `with_reason_code` on all four built-in allowance and charge layers, plus
  `DiscountLayer::vat` / `reason_code` and `TaxLayer::vat` / `reason_code` as
  defaulted trait methods.
- **`Quantity::code`** — EN 16931 **BT-130**, the UN/ECE Rec 20/21 unit code
  (`"KWH"`, `"H87"`). `Quantity::unit` stays display text, because the mapping is
  not mechanical: `"Stk"`, `"Stück"`, `"pcs"` and `"pieces"` are all `H87`.
  `PerUnitLevy::with_unit_code` stamps it on the generated position.
- **`Amount::exact_to::<Q>()`** — narrow precision *without rounding*, or fail.
  Completes the pair `checked_from_decimal` / `from_decimal_rounded` already had,
  so no conversion path in the crate can silently lose money. Rounding at an
  interchange boundary is exactly the mistake `AmountScale` exists to prevent.
- **`Period::is_ordered`** — BR-29 / BR-30, for ISO 8601 endpoints. Returns `None`
  rather than guessing when the strings are not `YYYY-MM-DD`.
- **`validate()` check 12** — BR-CO-18 as actually written: a document that charges
  VAT must have a BG-23, and a declared breakdown must have a VAT position behind
  it.

### Changed — breaking

- `LineItem` gained the public fields `vat` and `allowance_charge`; `Quantity`
  gained `code`; `TaxBreakdownEntry` gained `exemption_reason_code`. Struct
  literals need updating; the constructors and builders do not.
- `TaxLayer::reason_code` and `DiscountLayer::reason_code` are replaced by
  `allowance_charge() -> Option<AllowanceCharge>`, which can carry the base and
  percentage the reason code alone could not.
- `TaxCategory` gained a variant. Exhaustive `match`es need a `B` arm — which is
  the point of the enum not being `#[non_exhaustive]`.

### Changed — ⚠️ silent

- **A standard-rated 0 % layer is now refused.** `FixedRateTax::new(name, dec!(0))`
  defaults to category `S`, and `BR-S-05` requires a standard-rated line's rate to
  exceed zero — which makes an `(S, 0 %)` breakdown group unsatisfiable under
  BR-S-08. The category for a supply taxed at zero is `Z`, and
  `FixedRateTax::zero_rated` has always been its constructor. `L` and `M` are
  deliberately unaffected: BR-AF-05 and BR-AG-05 explicitly permit a zero rate, and
  `B` has no rate rule at all. Reported by `compute` / `breakdown`, not by `new`,
  so the `with_category` builder chain can still reach a valid state.
- **`tags::TAX` is now applied to *every* `TaxLayer` output**, by the engine
  rather than by each layer. `PercentageCharge` previously produced positions
  tagged only `percentage-charge`, contradicting the documented meaning of `TAX`
  and leaving them out of `PerUnitLevy`'s "exclude other layers' output" filter.
  `positions_by_tag("tax")` therefore returns more positions than before.
- **`"vat"` joined the reserved tag namespace**, so a caller-supplied label of that
  name — a time-of-use band named `"vat"`, say — is now rejected rather than
  silently changing how a document is classified.
- **`merge_period_documents` now validates its result**, so a merge that produces
  an inconsistent or unlawful document returns `Err` where it previously returned
  the document.
- **Two VAT layers covering one position are now rejected** with
  `BillingError::LayerError` naming both. That position is taxed twice and BR-S-08
  cannot hold for either group; previously the document assembled silently. The
  same check catches a caller-declared `LineItem::vat` that contradicts the layer
  actually taxing the position.

### Fixed — documentation

Every correction below was verified against the primary artefact, not restated
from a secondary source.

- **BT-119 is *not* suppressed under category `O`** — a claim introduced earlier in
  this same release cycle and wrong. BR-O-05 / BR-O-06 / BR-O-07 forbid the
  *line*, *allowance* and *charge* rates (BT-152 / BT-96 / BT-103); the VAT
  breakdown rate BT-119 is a different term, and XRechnung's **BR-DE-14** requires
  it unconditionally (*"Das Element 'VAT category rate' (BT-119) muss übermittelt
  werden"*, fatal, no category exception). Suppressing BT-119 for `O` on the
  strength of BR-O-05 fails the KoSIT validator. `TaxCategory::states_rate` now
  says explicitly that it does not apply to BG-23.
- **`BR-CO-18` was cited for the wrong rule at five sites.** BR-CO-18 says *"An
  Invoice shall at least have one VAT breakdown group (BG-23)"* — it is not the
  rule that forces one breakdown line per `(category, rate)`. That comes from two
  different places, and the standard's asymmetry between them is deliberate:
  BR-S-08 / BR-AF-08 / BR-AG-08 for the taxed categories (which may appear at
  several rates, hence *"at least one"*), and BR-Z-01 / BR-E-01 / BR-AE-01 /
  BR-IC-01 / BR-G-01 / BR-O-01 for the zero-tax ones (*"exactly one"*). The
  behaviour was always right; only the citation was wrong — and it appeared in a
  `ValidationFailed` message users would look up in a validator's rule index.
- **`BT-131 = BT-129 × BT-146` is not an EN 16931 warning.** EN 16931 has no such
  rule. It is `PEPPOL-EN16931-R120`, flagged **fatal**, and the real expression
  divides by the price base quantity (BT-149) and adds line-level charges and
  allowances: `BT-131 = BT-129 × (BT-146 / BT-149) + BG-28 − BG-27`, with a ±0.02
  tolerance. That is a *stronger* argument for what `reduce_position` does than the
  one previously written — rounding once keeps the residual under half a minor
  unit, rounding twice can exceed 0.02 and every Peppol access point then rejects
  the invoice.
- **The BR-CO-17 rounding-mode claim was stronger than the evidence.** The
  Schematron uses XPath `round()` — half-up toward positive infinity, not half away
  from zero; the two differ on negative midpoints. Half-away-from-zero is still the
  right default, but for a narrower reason: BR-CO-17 and BR-S-09 apply `abs()` to
  both operands before rounding, so the modes coincide there, and those rules then
  compare with a **±1.00 tolerance** anyway. In the totals chain `round()` operates
  on already-2-decimal sums and is a no-op.
- **`Currency::XXX` passes BR-CL-04.** It is a real ISO 4217 code and the
  Schematron's 178-code list contains it, so a document that was never configured
  validates as an EN 16931 invoice claiming no currency is involved. Documented
  with a pointer at `is_unset()`.
- **`with_advances` now says which formats can carry the per-advance tax.** ZUGFeRD
  EXTENDED has `BG-X-46`; XRechnung and Peppol BIS have *nowhere* to put it and
  will silently drop it — which is precisely the §14c Abs. 1 UStG double-taxation
  scenario the module exists to prevent. Against those formats a residual invoice
  is not a preference but the only correct construction.
- **`Period` ordering is the caller's responsibility**, and now says so, with
  BR-29 / BR-30 named and `is_ordered` offered.

## [0.8.0]

A hard cut on API shape, driven by feedback from a downstream migration. Nothing
here changes what a correct invoice looks like; it changes which mistakes the API
lets you make. One item is a **silent** semantics change — see ⚠️ below.

### Migration

| 0.7.0 | 0.8.0 |
|-------|-------|
| `Amount::from_decimal(d) -> Option` | `Amount::checked_from_decimal(d) -> Result` (now **exact**) |
| `Amount::try_from_decimal(d)` | `Amount::checked_from_decimal(d)` |
| implicit rounding in `from_decimal` | `Amount::from_decimal_rounded(d, strategy)` |
| `a.within_tolerance_ppm(b, ppm)?` / `.unwrap_or(false)` | `a.within_tolerance_ppm(b, ppm)` — returns `bool` |
| `LineItem::for_usage(d, q, "kWh", p, "EUR/kWh")` | `LineItem::for_usage(d, Quantity::new(q, "kWh"), UnitPrice::new(p, "EUR/kWh"))` |
| `LineItem::for_usage_rounded(d, q, qu, p, pu, s, strat)` | `LineItem::for_usage(d, Quantity::new(q, qu), UnitPrice::new(p, pu).rounded(s, strat))` |
| `LineItem::credit_for_usage_rounded(..)` | `LineItem::credit_for_usage(..)` + `UnitPrice::rounded` |
| `fn line_items(&self, u) -> Result<Vec<LineItem>, E>` | `fn line_items(&self, u) -> Result<Positions<Self::NotBillable>, E>`; return `Ok(items.into())` |
| `type Usage = ();` + ignored `usage` param | `impl ScalarTariff` with `fn positions(&self)` |
| `Box::new(FixedRateTax::new(..)?)` | `FixedRateTax::new(..)?.boxed()` |
| `FixedRateTax::new(n, dec!(0)).with_category(c).with_exemption_reason(r)` | `FixedRateTax::exempt(n, c, r)?` / `FixedRateTax::zero_rated(n)` |
| `TryFrom<Decimal>`/`TryFrom<i64>` → `ParseAmountError` | → `BillingError` |

Every `Tariff` impl needs one new line — `type NotBillable = Infallible;` when the
tariff always bills — and then `.bill()` and `.tariff()` work exactly as before.

### Added

- **`AmountScale` — assemble a document at an interchange format's decimal limit.**
  `BillingDocumentBuilder::amount_scale(AmountScale::EN16931)` reduces every *leaf*
  amount — each position, each discount- and tax-layer output, each VAT breakdown
  entry — to two decimals **before** any total is computed, so every total is a sum
  of already-reduced values and the totals identities still hold exactly.

  This is what makes a document emittable as EN 16931 / XRechnung / Peppol BIS /
  ZUGFeRD, and it cannot be done in a serialiser. Those formats cap every monetary
  amount at two decimals (BR-DEC-09/12/13/14/16/17/18/19/20 and BR-DEC-23 for the
  line amount) *and* require the totals identities to hold at that precision
  (BR-CO-10/13/14/15/16/17). Rounding amounts independently on the way out breaks
  the identities — three positions of `0.005` round to `0.03` while their exact
  total rounds to `0.02` (BR-CO-10), and a net of `0.0042` at 19 % VAT gives
  `0.00 + 0.00 ≠ 0.01` (BR-CO-15). Both arise from ordinary inputs.

  **Every amount is rounded exactly once, from the exact value.** Rounding twice is
  a different operation: `0.004999` rounded to five decimals is `0.005`, which then
  rounds to `0.01`, while rounding it straight to two decimals gives `0.00`. So a
  VAT category tax is derived from the *reduced* base in a single rounding (what
  BR-CO-17 specifies and what a validator recomputes); the charged VAT position
  carries that same number rather than being reduced independently, making
  `BT-110 = Σ BT-117` (BR-CO-14) hold by construction; and a line derived from
  `quantity × unit_price` is reduced from the exact product rather than from the
  engine's five-decimal intermediate, so `BT-131 = BT-129 × BT-146` holds. An
  explicit `fixed_amount` is authoritative and is reduced verbatim.

  `AmountScale::apply_decimal` exposes the single-rounding reduction for callers
  doing their own mapping. `AmountScale` re-runs its constructor check on
  deserialisation via `#[serde(try_from)]`, like every other validated type here.

  Verified across ~24 000 boundary cases and 6 000 randomised multi-line documents,
  covering all five rounding strategies, VAT rates with two to seven decimals, and
  full document stacks with discounts, a per-unit levy, compound VAT, a mixed-rate
  breakdown and a prepayment.

  Companions: `BillingDocument::fits_amount_scale(n)` and
  `amount_scale_violation(n)` (the precondition to assert before emitting),
  `Amount::round_to_scale(n, strategy)` and `Amount::fits_scale(n)`.
  `AmountScale::new(0, ..)` covers zero-decimal currencies (JPY, KRW).

  Note that `reverse()` preserves the scale while `AllocationRule` cannot — a
  three-way split of `100.00` is `33.333…`, so allocation trades precision to keep
  the split exact. Re-check an allocated document before emitting.
- **A third billing outcome.** `Billing<T, R>` (`Billable` / `NotBillable`) with the
  aliases `Positions<R>` and `Billed<R>`, plus the `Tariff::NotBillable` associated
  type. A settlement that is *not billable yet for a specific reason* — no meter
  reading, unpublished reference price, ended entitlement — is neither an error nor
  an empty position list, and flattening it to `Ok(vec![])` destroyed the reason.
  `R` is the domain's own type, so reasons stay exhaustively matchable.
  Tariffs that always bill set `type NotBillable = Infallible`, which makes the
  `NotBillable` variant uninhabited and `Billing::into_inner` a total function.
- **`ScalarTariff`** for settlements whose positions are already computed: one
  method, `positions(&self)`, with no `Usage` and no ignored argument. A blanket
  impl supplies `Tariff<Usage = ()>`, so a scalar settlement still composes with
  `BillingDocumentBuilder` and with any code generic over `Tariff`. Convenience:
  `settle(meta)` / `try_settle(meta)`.
- **`Tariff::try_bill` and `BillingDocumentBuilder::try_tariff`** propagate a
  not-billable reason. Their two-outcome counterparts `bill` / `tariff` are now
  bounded on `NotBillable = Infallible`, so a tariff that can decline to bill will
  not compile against them — the reason cannot be silently dropped.
- **`Amount::from_decimal_rounded(d, strategy)`** — the explicit-rounding
  conversion, honouring the crate's "rounding is always explicit" invariant.
- **`UnitPrice::rounded(scale, strategy)`** replaces the seven-argument
  `for_usage_rounded` constructors and composes with any builder path.
- **`FixedRateTax::exempt(name, category, reason)`** and
  **`FixedRateTax::zero_rated(name)`** — the EN 16931 zero-tax families as
  constructors that validate the category/reason pairing up front rather than at
  breakdown time. `reason` is a required argument, not an `Option`, because every
  category `exempt` accepts requires one and the one that forbids one (`Z`) has its
  own constructor.
- **`TaxLayer::boxed()` / `DiscountLayer::boxed()`** — `layer.boxed()` in place of
  `Box::new(layer) as Box<dyn TaxLayer>`.
- **`BillingError::PrecisionLoss { precision, input_value }`** distinguishes "does
  not fit the precision" from "does not fit the range". Previously both were
  `MonetaryOverflow`, or silently rounded.
- **A documented representable range** for `Amount<P>`: a per-`P` table of the
  maximum magnitude and smallest step, the backing integer width, and a table of
  which operations panic versus return `Err`.

### Fixed

- **The examples double-reported tax and bundled a commission into it.**
  `cloud_compute` iterated `all_positions()` — which yields the tax positions too —
  and then printed the VAT again as a total, so it appeared twice. `saas_billing`
  printed a `TAX TOTAL` that silently included a 3 % platform commission alongside
  20 % VAT, so the VAT line and the tax total disagreed and the invoice read as
  wrong. A `PercentageCharge` is a document-level charge (EN 16931 BT-108), not the
  VAT total (BT-110); the example now separates them via the `percentage-charge` tag
  and reports VAT as `Σ BT-117` from the breakdown. The arithmetic was correct
  throughout — the presentation was not.
- The examples now demonstrate invoice precision: `saas_billing` is assembled at
  `AmountScale::EN16931` and renders at two decimals, `cloud_compute` shows a
  full-precision document, what `amount_scale_violation` says about it, and the same
  document rebuilt as emittable, and `water_utility` shows allocation keeping a split
  exact while leaving invoice precision behind.

### Changed — breaking

- **⚠️ `Amount::checked_from_decimal` is now exact and no longer rounds.** A
  `Decimal` carrying more non-zero digits than `P` returns
  `BillingError::PrecisionLoss` instead of a silently rounded value. **This is a
  silent change: your code still compiles.**
  It closes a real disagreement between the two conversion paths —
  `Amount::<5>::parse("0.123456")` was rejected while
  `checked_from_decimal(dec!(0.123456))` quietly became `0.12346`, so the same
  unrepresentable unit price was refused as text and altered as a `Decimal`. Both
  now refuse it, and `from_decimal_rounded(d, strategy)` rounds when that is what
  you meant. Trailing zeros beyond `P` remain accepted by both.
- **`Amount::within_tolerance_ppm` returns `bool`, not `Result<bool>`.** The
  difference is now taken in `i128`, where no pair of `i64` operands can overflow,
  so the failure mode is removed rather than reported. The `Result` was actively
  harmful: every real call site wrote `.unwrap_or(false)`, converting an internal
  arithmetic error into *"outside tolerance"* — a spurious discrepancy finding in
  the tolerance checks written to catch discrepancies.
- **`Amount::from_decimal` and `Amount::try_from_decimal` are gone.** Three
  conversions with two error types and two failure conventions were one
  conversion too many, and the `Option`-returning one hid the failure. Use
  `checked_from_decimal` (exact) or `from_decimal_rounded` (explicit).
- **`TryFrom<Decimal>` and `TryFrom<i64>` for `Amount<P>` now yield
  `BillingError`**, not `ParseAmountError`. `ParseAmountError` is for parsing text,
  which is what its name says; these are conversions, and they now agree with
  `checked_from_decimal` / `checked_from_int`.
- **`LineItem::for_usage` and `credit_for_usage` take `Quantity` and `UnitPrice`.**
  They used to take four loose arguments, of which two were adjacent free-form
  `&str` unit labels: swapping `"kWh"` and `"EUR/kWh"` compiled and produced a
  wrong invoice. Pre-assembled pairs make that a type error.
- **`LineItem::for_usage_rounded` and `credit_for_usage_rounded` are removed.**
  Seven positional arguments for "round the price first"; use
  `UnitPrice::rounded(scale, strategy)`.
- **`Tariff::line_items` returns `Result<Positions<Self::NotBillable>, Self::Error>`**
  and `Tariff` gains the `NotBillable` associated type. `Ok(items.into())` is the
  always-billable form.

### Fixed

- **Empty unit labels were accepted on `LineItem`.** `TariffSchedule` and
  `TimeOfUsePricing` had always rejected them, but `Quantity` / `UnitPrice` did
  not, so a hand-built or deserialised position could carry one. That is not
  cosmetic: `PerUnitLevy` selects its base by matching `unit_label`, so a position
  with an empty unit was billed while every per-unit levy on it silently was not.
  `LineItemBuilder::build`, `LineItem::validate` and therefore
  `BillingDocument::from_positions` and serde now all reject it.
- **`Amount::checked_from_decimal`'s documentation described the wrong order of
  operations**, claiming it rounded to `P` decimals before scaling when it scaled
  first. Moot now that the method is exact, but the rounding conversion documents
  its actual behaviour.

## [0.7.0]

The crate becomes invoice-grade: a per-rate VAT breakdown, advance-payment
settlement, cash rounding and credit notes, on top of three rounds of correctness
fixes. There is no migration shim — this is a hard cut.

### Added

- **VAT breakdown (EN 16931 BG-23).** `TaxBreakdownEntry` and `TaxCategory`
  (UNTDID 5305: S/Z/E/AE/K/G/O/L/M) with the per-category rules enforced, not
  merely documented. `BillingDocument::tax_breakdown()` reports the taxable base
  and tax per `(category, rate)` — legally required by EU VAT Directive art. 226
  and §14 Abs. 4 UStG, and impossible to express with a single tax total.
  `TaxLayer::breakdown()` is a defaulted trait method, so non-VAT layers
  (commissions, per-unit excise) correctly contribute nothing.
- **Advance payments.** `AdvancePayment` carries an advance's per-rate tax —
  the data EN 16931's flat BT-113 cannot express and that §14 Abs. 5 Satz 2 UStG
  requires on a final invoice. `BillingDocument::with_advances`,
  `advance_deductions()`, `advance_tax_total()`, and
  `advance::residual_breakdown` for the residual-invoice form.
- **`DocumentKind`** — UNTDID 1001 document type codes (BT-3), on `DocumentMeta`.
- **Cash rounding.** `CashRounding` implements tender-level rounding
  (Rappenrundung, öresavrundning) as EN 16931 BT-114, leaving the taxable base
  untouched. No per-currency default: the increment is a payment-law fact, not a
  currency property.
- **Prepayments and amount due.** `with_prepaid` (BT-113) and `amount_due()`
  implementing BR-CO-16 exactly. May be negative — the credit-balance case.
- **Credit notes.** `BillingDocument::reverse()` negates a document including its
  VAT breakdown, flipping signs consistently.
- **`Currency`** — ISO 4217 code with `minor_units()` (`Option<u8>`: 13 codes have
  none, 17 have zero, 7 have three, 2 have four) and `minor_unit_increment()`.
- **Money splitting.** `Amount::distribute` (N equal-as-possible parts) and
  `Amount::allocate` (integer ratios, largest-remainder). Both exact.
- **`Amount`** gains `checked_div` and `round_to_increment`.
- **`LineItem::scaled`** and `LineItem::validate`.
- **Property-based tests** (`proptest`) for the algebraic laws, **criterion
  benchmarks**, and README examples compiled as doctests.

### Changed — breaking

- **All four pricing types share one shape.** `TimeOfUsePricing` and
  `DynamicPricing` are now built through `::builder()` with infallible chainable
  setters and a single fallible `build()`, matching `TariffSchedule`.
  `TimeOfUsePricing::new`, `DynamicPricing::from_intervals`, `with_unit` and
  `with_currency` are gone; the builders use `unit()`, `currency()`, `band()` /
  `interval()`. This removes fallible mid-chain setters and puts every check in
  one place.
- **`Prepayment` replaces the `prepaid` / `advances` pair.** A flat total and
  itemised advances are the same fact at different resolutions, so they are one
  enum (`None` / `Total` / `Itemised`) rather than two fields that could disagree.
  The contradictory state is now unrepresentable rather than rejected at runtime.
  `with_prepaid` and `with_advances` remain as wrappers over `with_prepayment`,
  each replacing the whole prepayment.
- Unit labels must be non-empty; the engine's tag namespace (`tags::RESERVED`) is
  protected against caller collisions.
- Constructors that validated by panicking now return `Result`: `FixedRateTax`,
  `PercentageCharge`, `PercentageDiscount`, `PerUnitLevy`, `FixedDiscount`,
  `EqualAllocation`, `TimeOfUsePricing`. Rates and shares routinely come from
  configuration, where invalid input must be recoverable.
- **No implicit currency.** Generated unit-price labels used a hardcoded `"EUR/"`.
  `Currency` is now explicit and defaults to ISO 4217 `XXX` ("no currency
  involved") — a visible placeholder rather than a silently wrong symbol.
- `TimeOfUsePricing::calculate` returns `Err` for an unknown band name instead of
  skipping it, and is generic over `AsRef<str>`.
- `Amount<P>` serialises as a decimal string (`"0.03456"`), not a raw scaled
  integer. Deserialisation accepts strings only.
- `Period` moved from `line_item` to `period`.
- `Amount::to_decimal` removed — it was identical to `into_decimal`, and its doc
  described a distinction that did not exist.
- `BillingDocument::with_extra_position` is refused on a document carrying a VAT
  breakdown; adding to the net without re-running the tax layers under-billed VAT.
- `TariffSchedule` in `volume`/`capacity` mode now errors when a quantity exceeds
  a bounded top band, matching `graduated` mode.
- `Amount<P>` is `#[must_use]`; `BillingDocument` is `PartialEq`.
- `dec!` now comes from `rust_decimal`'s `macros` feature; the `rust_decimal_macros`
  dependency is gone.

### Fixed

- **Remote panic from untrusted JSON.** VAT-breakdown validation compared
  attacker-controlled values with a bare `Decimal` subtraction, aborting the
  process instead of returning `Err`.
- **`checked_*` methods could panic.** `rust_decimal`'s operators panic rather
  than saturate, so `checked_mul_qty`, `from_decimal`, `LineItem::build`,
  `prorate`, `proportional_split` and block-mode `split` all aborted on overflow
  despite documented `Result` contracts.
- **Allocating a taxed document produced invalid invoices.** `net`, `tax` and
  `gross` were rounded independently, so `net + tax == gross` failed whenever the
  share did not divide evenly. `gross` is now derived.
- **Stacked per-unit levies double-counted.** A second levy on the same unit
  counted the first levy's own line as consumption, over-billing it by 100%.
- **Validation was bypassable via serde** on every type with invariants; all now
  re-validate on deserialisation.
- **Scaled positions contradicted themselves** — proration and allocation scaled
  the amount but not the quantity, yielding lines like "1000 kWh × 0.30 = 150.00".
- **`Display` ignored width, fill and alignment**, so `{:>12}` was a no-op and
  invoice columns never aligned — including in this crate's own examples.
- **A negative `AdvancePayment` built a document that failed its own
  `validate()`**, with an `amount_due` larger than the gross.
- **A time-of-use band named `tax` removed its own consumption from a per-unit
  levy base**, silently under-billing; band names are config-driven.
- `from_positions` did not validate the `LineItem`s handed to it, so a document
  with an empty position description could not round-trip through serde.
- `FixedRateTax::compute` skipped the VAT-category checks that `breakdown`
  performed, letting a zero-tax category charge tax.
- `merge_period_documents` silently dropped itemised advances and cash-rounding
  rules; both are now refused.
- **A credit note of a prepaid invoice was impossible** — `reverse()` produced a
  negative BT-113 that validation rejected.
- Single-band schedules skipped contiguity validation, silently billing from zero.
- `proportional_split`'s share tolerance was absolute while its error scaled with
  the total, breaking the documented sum guarantee for large amounts.
- `Amount<19>` failed with a raw const-eval overflow; it now reports why `P ≤ 18`.

### Security

- `#![forbid(unsafe_code)]` retained. `cargo-deny` and `cargo-semver-checks` run
  in CI.
