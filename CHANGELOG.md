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

Pin exactly (`billing = "=0.8.0"`) if you need to schedule migrations yourself;
`"0.8"` will pick up `0.8.x` patch fixes only.

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
