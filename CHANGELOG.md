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

Pin exactly (`billing = "=0.13.0"`) if you need to schedule migrations yourself;
`"0.13"` will pick up `0.13.x` patch fixes only.

## [0.13.0]

Fourteen defects that the arithmetic could not see, and four things the engine could not
say. Several of the last group came from an [`en16931`](https://github.com/hupe1980/en16931)
adapter review; every claim in it was re-derived from the CEN, Peppol and KoSIT
artefacts before it was acted on, and one of its own citations did not survive that
(see `PartyIdentifier` on ICD `0293`).

Every release so far has been checked by asking *"do the totals add up?"* — and every
one of them did. This one came from asking a different question: **which fields does
an EN 16931 rule make mandatory that no total depends on?** There turn out to be
several, and the engine was getting them wrong on documents that balanced, validated,
serialised and rendered without a murmur.

- A shortfall produced by `minimum_charge` was an invoice line missing **BT-129,
  BT-130 and BT-146** — three fatal rules, on a position the engine generated itself.
- A document level allowance handed to `.positions(…)` was emitted as an invoice
  line, putting its amount in **BT-106** instead of **BT-107** — every total still
  correct, the decomposition a validator recomputes wrong.
- `BillingDocumentBuilder::currency` wrote into the header, so a later `.meta(…)`
  discarded it and the document carried `XXX`. Nothing failed; the currency was just
  wrong on every line.
- `Currency::new` accepted any three ASCII letters, and there was no way to ask
  whether the result was a code EN 16931 admits. `BR-CL-04` is fatal and is checked
  at an access point, not at a boundary the caller controls.
- A `PricingModel` knew which format it priced for and had nowhere to say so, so
  `bill()` and `settle()` could only ever assemble at five decimals — the shortest
  path from a model to a document produced one that fails `BR-DEC-23`.
- `from_positions` promised a document that "satisfies every invariant by
  construction" and never ran `validate()`, so two ordinary tax layers were enough to
  get back a document that failed its own `assert_valid()`.
- `UsageAggregator::aggregate` returned a bare `Decimal` at the point where raw usage
  events enter the system, and `Decimal`'s `+` and `*` panic on overflow. One corrupt
  record took down the batch run.
- **BR-B-02** was documented as "jurisdictional … stays with the caller" alongside
  BR-B-01. Only BR-B-01 is: the other is decidable from the categories on the
  document, exactly like BR-O-11, which the engine has always checked.
- **BT-107** was the only member of the totals chain with no accessor of its own —
  reachable only as `discount_total()`, which carries the opposite sign to the one
  every syntax emits.
- `PaymentTerms::Display` rendered **BT-20** without the newline `BR-DE-18` requires,
  so every German invoice offering Skonto was rejected — and the module's own
  documentation claimed the opposite.
- `ProportionalAllocation` broke **BR-S-08** for 45 % of recipients on a two-rate
  document, while every total remained exact.
- `Period::is_ordered()` existed, was correct, and was called by nothing — while
  **BR-29**, **BR-30**, `PEPPOL-EN16931-R110` and `R111` all turn on it.

As in 0.12.0, every rule cited here was re-derived from the pinned CEN and Peppol
Schematron (`ConnectingEurope/eInvoicing-EN16931` @ `validation-1.3.16`,
`OpenPEPPOL/peppol-bis-invoice-3` @ `master`) rather than from recollection. Two
findings were dropped in the process: a proposed "rate rounds to zero" branch of
`BR-CO-17` was implemented only for a rate of *exactly* zero, because the
Schematron's `round(BT-119)` — read literally — also rejects a lawful 0.4 % supply,
and this crate does not refuse arithmetically correct documents to imitate an
artefact quirk.

### Fixed — **BR-DE-18**: BT-20 was rendered in a form KoSIT rejects

`PaymentTerms: Display` omitted the newline that must follow the last `#SKONTO#…#`
segment, so **every XRechnung carrying an early-payment discount was rejected** —
`flag="fatal"`.

The rule is two assertions folded into one `satisfies`, and the second is easy to
read past because it is not a separate `<assert>`. Verbatim from
`itplr-kosit/xrechnung-schematron` @ `master`,
`src/validation/schematron/ubl/XRechnung-UBL-validation.sch`:

```xpath
every $line in cac:PaymentTerms/cbc:Note[1]/tokenize(., '(\r?\n)')
                 [starts-with(normalize-space(.), '#')]
  satisfies matches(normalize-space($line), $XR-SKONTO-REGEX)
        and matches(cac:PaymentTerms/cbc:Note[1]/tokenize(., '#.+#')[last()], '^\s*\n')
```

`#.+#` is greedy and `.` does not match a newline, so on an unterminated field
`tokenize(., '#.+#')[last()]` is the empty string and the second `matches` is false.
Confirmed three ways before the fix: the rule's own prose (*"Am Ende einer
vollständigen Skontoangabe muss ein XML-konformer Zeilenumbruch folgen"*), a KoSIT
fixture marked `schematron-invalid="xrcii:BR-DE-18"` and titled *"Every skonto entry
should end with a newline"*, and by evaluating the expression above against a real
XPath 2.0 engine for every shape this crate renders.

The terminator lives in `Display` rather than in a separate `to_bt20_xrechnung()`
because the `#SKONTO#…#` micro-syntax **is** the German CIUS convention — there is no
core EN 16931 form of it — so a rendering without the terminator is valid nowhere at
all. A prose-only BT-20 is unchanged: the `every … satisfies` is vacuously true when
no line starts with `#`, so a newline there would be noise.

`FromStr` already accepted the terminated form, so the round trip is unaffected.

Two documentation claims went with it: `PaymentTerms` said `Display` renders the
field *"exactly as it goes on the wire"*, and `EarlyPaymentDiscount::from_str` said
*"a validator enforcing BR-DE-18 will reject what this refuses"* — true of what
`from_str` refuses, and not of what `Display` emitted.

*Reported against 0.12.0 from the [`en16931`](https://github.com/hupe1980/en16931)
adapter, with the XPath already traced. Thank you.*

### Fixed — allocation now preserves **BR-S-08**

`ProportionalAllocation` scaled each position by the share and applied one penny
correction per recipient to hit a separately-computed `net_total`, while splitting
the VAT breakdown per `(category, rate)` group. The two corrections were independent,
so a group's BT-116 and the positions attributed to it landed a minor unit apart.

Measured before the change, over 13 424 recipient documents from two-rate sources:
**6 087 — 45 % — failed `verify_vat_attribution`**, every one of them on the taxable
base and none on BR-CO-14. Each document was internally exact and passed
`validate()`; each would have been rejected by a validator. After the change, 0 of
24 634.

The fix is to split **every position exactly** — `Σ(recipients) == the original
position`, line by line — and derive every total by summing a recipient's own
positions. Each group's taxable base is then read off the very positions BR-S-08
checks it against. Where the *source* document does not satisfy BR-S-08 for a group
(a breakdown with no attributed positions behind it), that group falls back to a
proportional split, so an allocated share is never worse than what it came from:

> **An allocated share satisfies `verify_vat_attribution` whenever the document it
> came from did.**

The review asked for an `is_allocated_share()` predicate so a consumer could
*suppress* the check. That is the wrong shape — a flag saying "this document is
known to fail a fatal rule" institutionalises the defect and pushes the decision onto
a caller three layers from the split, who cannot know. Fixing the split removes the
question instead, and the escape hatch it was meant to justify can be deleted rather
than automated.

`AmountScale` still does not survive allocation and cannot: `100.00` three ways is
`33.333…`.

### Added — `Note` (**BG-1**) and `PartyIdentifier` (**BT-29 / BT-46**)

Two `DocumentMeta` fields could not express the terms they stood for.

`notes` was `Option<String>`: one note, uncoded. BG-1 INVOICE NOTE is `0..n`, and each
occurrence carries **BT-21** (note subject code) beside **BT-22** (the text) —
`BR-CL-08`, *fatal*: *"Invoiced note subject code shall be coded using UNCL4451"*. A
note's purpose is not inferable from its wording, and downstream systems route on the
code. Multiplicity is real too: `PEPPOL-EN16931-R002` caps document-level notes at
one *unless both parties are German*, which only makes sense if more than one is
otherwise lawful.

`issuer_id` / `recipient_id` were `Option<String>` described as "MP-ID, GLN, BDEW
code, or free-form". EN 16931 types BT-29 and BT-46 as `Identifier`s with an optional
scheme, and `BR-CL-10` — *fatal* — restricts that scheme to the ISO 6523 ICD list. A
bare string cannot say which registry an identifier belongs to, and a consumer that
guesses produces an invoice that validates while identifying the wrong one.

```rust
notes: vec!["Lieferung ab Werk.".into(), Note::coded("AAI", "Es gelten unsere AGB.")],
issuer_id: Some(PartyIdentifier::scheme("0088", "4012345000009")),  // a GLN, said so
recipient_id: Some(PartyIdentifier::new("KND-4711")),               // no registry
```

Neither code list is embedded, for the same reason UNCL 5189, UNCL 7161, VATEX and
UN/ECE Rec 20 are not. **One trap is worth naming**, because the review fell into it:
the ICD list in `validation-1.3.16` runs `0002` … `0248`, and `0293` — commonly cited
for BDEW — **is not in it**. An identifier assigned an ICD after that revision fails
`BR-CL-10` even though it looks perfectly well formed. Check the scheme against the
artefact your target profile actually runs. The rule admits exactly one non-numeric
value, `SEPA`, and only under `cac:AccountingSupplierParty` or `cac:PayeeParty`.

### Changed — breaking — the period rules are now checked

`Period::is_ordered()` shipped in 0.12.0 and was called by **nothing but its own
tests**. Four fatal rules turn on exactly the comparison it performs:

| Rule | Requires |
|---|---|
| **BR-29** | BT-74 ≥ BT-73 — the invoicing period is ordered |
| **BR-30** | BT-135 ≥ BT-134 — a line period is ordered |
| `PEPPOL-EN16931-R110` | line period **start** ≥ invoice period start |
| `PEPPOL-EN16931-R111` | line period **end** ≤ invoice period end |

All four are properties of dates the engine already holds, so they are checked rather
than deferred — the same argument that puts BR-B-02 and BR-O-11 inside `validate()`.
BR-30 is enforced by `LineItemBuilder::build` and `LineItem::validate`; BR-29 and the
two Peppol containment rules are check 15 of `BillingDocument::validate`. `Period`
gains `within()` and `validate()`.

The crate stays date-**type**-agnostic. The comparison only runs where every endpoint
is a zero-padded ISO 8601 date — the one shape a lexicographic compare orders the way
a calendar does — and says nothing at all otherwise, exactly as `is_ordered` already
did. A confident `false` on `"1.6.2026"` would be worse than no answer.

### Changed — `RateLookup::rate_for` refuses a negative parameter

Every bound in a lookup table is positive, so a negative parameter was below all of
them and the `<=` search returned the **lowest** band — the *cheapest* rate — for
what is a data error: an installed capacity or a peak demand cannot be less than
nothing. `RateSchedule::split` and `RateSchedule::apply_peak` have always refused it;
the three rate tables now agree, and "no silent under-billing" holds across all of
them rather than two.

### Added — `DocumentKind::is_self_billed`

Joins `is_credit_note` and `is_peppol_billing_code`: the semantics as a predicate,
with the code as its rendering. Self-billing is a different Peppol *profile* with its
own `CustomizationID`, not a variant invoice — which is why `389` is absent from
`PEPPOL-EN16931-P0100` while XRechnung's `BR-DE-17` lists it. Comparing against `389`
at a call site goes stale; UNTDID 1001 has more than one self-billing code.

### Fixed — the primary constructor did not run its own validator

`BillingDocument`'s type documentation promised that a document from
`from_positions` *"satisfies every invariant by construction"*. Computing the totals
makes checks 1–4 true and nothing else. Checks 12, 13 and 14 — **BR-CO-18**,
**BR-O-11** and **BR-B-02** — are properties of the *VAT breakdown*, and an ordinary
pair of tagged tax layers reaches them:

```rust
// Both layers are individually lawful. The document was not, and was returned anyway.
.extra_tax(FixedRateTax::exempt("Nicht steuerbar", TaxCategory::OutOfScope, "…")?
    .with_tag("outside").boxed())
.extra_tax(FixedRateTax::new("MwSt", dec!(0.19))?.with_tag("taxed").boxed())
```

`from_positions` handed that back happily, and `assert_valid()` on the result
panicked. `merge_period_documents` and `AllocationRule` have re-checked their outputs
since 0.11.0 for exactly this reason; the constructor now does too, so the promise is
true and the failure arrives where the mistake is.

The extra pass is one traversal of a document that was just assembled. Nothing in the
existing suite relied on getting an invalid document back — the whole thing stayed
green when the check was added.

### Changed — breaking — an invoice line may not carry BG-20 / BG-21 detail

The bucket a position sits in decides which EN 16931 group it is emitted as:
`net_positions` becomes an invoice line (**BG-25**), `discount_positions` and
`tax_positions` become document level allowances and charges (**BG-20** / **BG-21**).
`LineItem::allowance_charge` *is* the BG-20 / BG-21 detail — its own documentation
says it is `None` on an ordinary invoice line — so a position carrying one and handed
to `.positions(…)` claims to be both, which no syntax can express.

Nothing noticed, because nothing in the totals could: the amount landed in BT-106
instead of BT-107 / BT-108, so BT-109 still came out right and only the
*decomposition* a validator recomputes — BR-CO-10, BR-CO-11, BR-CO-12 — was wrong.
This is the document level twin of the BG-20-plus-BG-27 nesting `LineItem::validate`
has always refused.

Check 11 now covers it. Supply a document level allowance as a `DiscountLayer`
(`FixedDiscount`, `PercentageDiscount`) and a charge as a `TaxLayer`
(`PercentageCharge`, `PerUnitLevy`), which is what puts them in the right bucket.

### Added — `BillingDocument::allowance_total`, EN 16931 **BT-107**

BT-107 was the only member of the totals chain with no accessor under its own name.
BT-106, BT-108, BT-109, BT-110, BT-112, BT-113, BT-114 and BT-115 all had one; the
sum of document level allowances was reachable only as `discount_total()`, which is
the same figure with the **opposite sign**.

That sign is not cosmetic. `cbc:AllowanceTotalAmount` is unsigned in UBL and in CII,
because **BR-CO-11** defines BT-107 as a magnitude and **BR-CO-13** *subtracts* it.
This engine stores allowances as negative amounts so its totals stay plain sums, so a
consumer had to remember to negate — and an invoice stating `-20.00` where BR-CO-13
expects `20.00` fails by **twice** the allowance, with nothing in the document to show
it.

`allowance_total()` is that negation, done once, under the term's own name. The chain
now reads BT-106 … BT-115 with no gap and no sign conversion left to the caller.

### Changed — breaking — `BR-B-02` is now checked

`TaxCategory::SplitPayment` described both of its rules as *"jurisdictional rather
than arithmetic, and stay with the caller"*. Only one of them is:

| Rule | Needs | Verdict |
|---|---|---|
| **BR-B-01** — shall be a domestic Italian invoice | the parties' countries | genuinely out of scope; this crate never sees them |
| **BR-B-02** — `B` shall not appear alongside `S` | the categories present | **the engine owns this entirely** |

BR-B-02 is structurally identical to **BR-O-11**, which `validate()` has always
checked — including the crate's own justification for checking it: *"because
`merge_period_documents` can otherwise manufacture the combination out of two
individually lawful documents."* That argument applies verbatim, and merging an
Italian split-payment document with a standard-rated one is exactly how you get there.

It is now check 14 (breakdown level, in `validate`) and a matching clause in
`verify_vat_attribution` (position level), mirroring how BR-O-11 and BR-O-12/13/14
divide — the rule's Schematron reads BT-151, BT-95 and BT-102 as well as BT-118, so a
document whose breakdown carries only `B` can still violate it through an attributed
line.

`validate()` therefore now documents and runs **fifteen** invariants, in the order it
lists them.

### Changed — breaking — `UsageAggregator::aggregate` returns `Result`

```rust
fn aggregate(&self, events: &[E]) -> Decimal;                      // before
fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError>; // after
```

This trait sits at the **least trusted boundary in the pipeline**: raw usage events,
straight from a meter, a log or an API. `Decimal`'s `+`, `*` and `Sum` *panic* on
overflow rather than saturating, so `SumAggregator` and `WeightedSumAggregator` over a
slice containing one corrupt magnitude aborted the process — a batch run dying on a
single malformed record, which is the failure mode section 1 of
`tests/audit_regressions.rs` exists to prevent and that every `checked_*` method in
this crate was written to remove. `Amount::checked_sum` says so in as many words:
*"prefer this over `.sum()` in any code path that could receive attacker-controlled or
unbounded values."* Usage events **are** that path, and it was the one place with no
error channel to report on.

`WeightedSumAggregator` is the likelier of the two, because it multiplies before it
accumulates: a duration handed over in seconds rather than as a fraction is a factor
of ~2.6 million.

Four of the six built-ins — `COUNT`, `UNIQUE_COUNT`, `MAX`, `LATEST` — select or count
rather than accumulate and can never fail. They return `Result` anyway: a caller
writing code generic over `UsageAggregator` needs one signature, and an implementation
that later *starts* accumulating should not become a breaking change for its callers.

Migration is `?` or `.unwrap()` at each call site; nothing else changes.

### Changed — breaking

| Old | New |
|---|---|
| `minimum_charge(&doc, minimum, description)` | `minimum_charge(&doc, minimum, description, currency)` |

`minimum_charge` built its shortfall with `LineItem::fixed`, which states BT-131 and
nothing else. A shortfall is an ordinary invoice line (BG-25), so **BR-22** (BT-129
invoiced quantity), **BR-23** (BT-130 unit code) and **BR-26** (BT-146 item net price)
all apply to it — three *fatal* rules, none of which changes an amount, which is
exactly why no total ever noticed.

It now builds through `LineItem::flat_fee`: one unit of `UNIT_CODE_ONE` (`C62`) at a
price equal to the whole shortfall. `1 × shortfall` is exactly `shortfall`, so
`PEPPOL-EN16931-R120` holds trivially and nothing is rounded that was not rounded
before. `RateSchedule::apply_peak` was moved to the same construction in 0.11.0;
`minimum_charge` was the one generated line left behind.

The currency is an argument because the generated BT-146 label is `"EUR"` / `"CHF"` /
…, and a generated label cannot invent the currency. Pass `Currency::XXX` where it
genuinely does not matter — it renders as `XXX`, which reads as the bug it is.

```rust
// before
minimum_charge(&doc, min, "Mindestentgelt")?
// after
minimum_charge(&doc, min, "Mindestentgelt", Currency::EUR)?
```

### Fixed — ⚠️ silent — `BillingDocumentBuilder::currency` is order-independent

`.currency(EUR).meta(header)` produced a document denominated in `XXX`. The setter
wrote straight into `meta`, and `.meta()` replaces the whole header, so the euro was
discarded by a method-ordering accident: no error, no failing total, `validate()`
green, and the wrong currency on every rendered line and every generated unit-price
label. The previous release *documented* the hazard and pinned it with a test; a
document that quietly claims "no currency involved" is not a hazard worth documenting.

The builder now records the currency separately and applies it when the document is
built, so both orders agree. An explicit `.currency()` still wins over
`meta.currency`; a header currency with no explicit call still applies.

**This will not fail your build.** If you relied on `.meta()` resetting an
earlier `.currency()` — which no correct code does — the currency now survives.

### Added — `Currency::is_iso4217`

`Currency::new` accepts any three ASCII letters, and deliberately keeps doing so: this
crate bills things that are not EN 16931 invoices, and a private clearing unit is a
legitimate thing to denominate an internal document in. What it must not do is let
such a code reach an *invoice* unremarked, because **BR-CL-03 / BR-CL-04 / BR-CL-05**
are fatal and are enforced downstream.

There are now two independent questions, and neither answers the other:

```rust
assert!(Currency::EUR.is_iso4217());
assert!(!Currency::new("XBT")?.is_iso4217());  // well-formed, constructible, fatal
assert!(Currency::XXX.is_iso4217());           // XXX *is* in the list …
assert!(Currency::XXX.is_unset());             // … and still means "unconfigured"
```

The 178-code list is transcribed verbatim from the `contains(' AED AFN … ZWG ')` in
BR-CL-03 of the pinned CEN artefact, not from ISO's registry in the abstract, so it
answers the question a validator will actually ask. It also makes `minor_units`
honest: the fall-back to `Some(2)` is now a documented default for codes
`is_iso4217()` rejects, rather than an unmarked guess.

This is the same split as `DocumentKind::is_peppol_billing_code`: model the value,
expose the profile question as a predicate the consumer asks before it emits.

### Added — `PricingModel::amount_scale`

A model knows which format its output is destined for; the call site often does not.
`amount_scale` is where it says so, and `bill`, `try_bill`, `settle`, `try_settle`
and `BillingDocumentBuilder::pricing` / `try_pricing` all honour it.

```rust
impl PricingModel for MeteredPlan {
    // …
    fn amount_scale(&self) -> Option<AmountScale> { Some(AmountScale::EN16931) }
}

let doc = MeteredPlan.settle(meta)?;
assert!(doc.fits_amount_scale(2));   // emittable, straight out of `settle`
```

Precision reduction has to happen while a document is assembled — rounding a finished
one breaks BR-CO-10 and BR-CO-15, which is the whole argument for `AmountScale`. So
without this hook `bill` and `settle` could only produce five-decimal documents, and
the shortest path from a model to a document was also the one that produced an
unemittable invoice. An explicit `amount_scale` on the builder still wins, so a
caller can override the model's choice.

Default `None`, so existing implementations are unaffected.

### Changed — `Amount::parse` accepts an omitted integer part

`".5"`, `"-.5"` and `"+,05"` now parse as `0.5`, `-0.5` and `0.05`. A separator with
nothing on either side (`"."`, `"-,"`) is still an error.

`Amount::parse` and `Amount::checked_from_decimal` are documented to agree on exactly
what is representable, and `Decimal::from_str` has always accepted this shape — so the
same value was refused as text and accepted as a `Decimal`, for the form a CSV export
and a hand-typed price most often take. `"5."` was already accepted; the two ends of
the number now behave alike.

### Changed — `TaxBreakdownEntry::validate` tightens BR-CO-17 at a zero rate

The CEN Schematron for BR-CO-17 is two branches, and only the second was implemented.
At a rate of zero the rule demands `round(BT-117) = 0` — i.e. `|BT-117| < 0.5`, not
the `< 1.00` slack that applies elsewhere. The branch is reachable: `L`, `M` and `B`
may carry a zero rate (BR-AF-05, BR-AG-05, and no BR-B-05 at all) while still levying
tax in general, so `TaxCategory::carries_tax` does not force BT-117 to zero for them.

Applied at a rate of **exactly** zero, not at "a percentage that rounds to zero".
Read literally the Schematron's `round(BT-119)` also selects every rate below 0.5 %
and would then demand no tax on a lawful 0.4 % supply; where the two readings differ,
this crate declines to reject a document that is arithmetically correct.

### Changed — `BillingDocument::validate` runs its checks in the documented order

The fourteen invariants were listed 1 … 14 and executed 1, 2, 3, 5, 8, 9, 11, 10, 6,
7, 4, 12, 13, 14. Same set, same result, and nothing to reconstruct when reading it.

### Performance — `fits_amount_scale` no longer allocates on the happy path

`amount_scale_violation` built a `Vec<(String, Amount<5>)>` naming *every* leaf —
one `String` per position, per line allowance and per breakdown entry — before
looking for a violation, and `fits_amount_scale` called it to answer a `bool`. The
scan is now lazy: the label is formatted only for the amount that actually fails. On
a thousand-line document that is roughly three thousand allocations saved per call,
and it is called once per emit.

Behaviour, including the order in which leaves are checked and therefore *which*
violation is reported first, is unchanged.

### Fixed — the doctests ran in nobody's CI

`src/lib.rs` ends with `#![doc = include_str!("../README.md")]` specifically so that
every Rust example in the README is compiled and run, and the README says so in a
blockquote near the top. The mechanism worked; nothing was running it.

`cargo test --all-targets` silently **excludes** doctests
([rust-lang/cargo#6669](https://github.com/rust-lang/cargo/issues/6669)), and so
does `--lib --tests`. Every one of the five test invocations in `.github/workflows/ci.yml`
and both in the `justfile` used one of those two forms, so the level of the testing
pyramid whose entire job is preventing documentation rot was enforced only by
whoever happened to type `cargo test` by hand.

Added: a `just test-doc` recipe (wired into `just ci`) and an explicit
`cargo test --doc --all-features` step in the test matrix and the MSRV job. 168
doctests, including the README as one unit, now run on stable, beta and 1.85.

Worth checking in your own project: the flag gives no warning and the test count
simply comes out lower.

### Migration

| Old | New | Why |
|---|---|---|
| BT-20 without a trailing newline | `PaymentTerms::to_string()` supplies it | ⚠️ silent — `BR-DE-18` is *fatal* and rejected every Skonto invoice |
| `is_allocated_share()` to skip BR-S-08 | nothing — allocation preserves it | the escape hatch the review asked for is no longer needed |
| `notes: Option<String>` | `notes: Vec<Note>` | BG-1 is `0..n` and each note carries BT-21 (`BR-CL-08`) |
| `issuer_id: Option<String>` | `issuer_id: Option<PartyIdentifier>` | BT-29 / BT-46 need their ISO 6523 scheme (`BR-CL-10`) |
| `kind == DocumentKind::SelfBilledInvoice` | `kind.is_self_billed()` | the code is the rendering, not the question |
| a backwards or out-of-range `Period` | now returns `Err` | BR-29 / BR-30 / `R110` / `R111` are fatal and decidable for ISO 8601 dates |
| `rate_for(negative)` returning the lowest band | now returns `Err` | ⚠️ silent — it was the *cheapest* rate, for a value that cannot exist |
| `minimum_charge(&doc, min, desc)` | `minimum_charge(&doc, min, desc, currency)` | the shortfall is now a complete invoice line (BR-22 / BR-23 / BR-26) and its BT-146 label needs a currency |
| `agg.aggregate(&events)` | `agg.aggregate(&events)?` | raw events are the least trusted input; overflow is reported instead of aborting the run |
| `-doc.discount_total()` for BT-107 | `doc.allowance_total()?` | BT-107 is unsigned in every syntax; the negation is now done once, under the term's own name |
| `.currency(c).meta(m)` losing `c` | `.currency(c).meta(m)` keeps `c` | ⚠️ silent — the setter is order-independent |
| `Currency::new(x).is_ok()` as a BT-5 check | `Currency::new(x)?.is_iso4217()` | well-formedness is not membership; BR-CL-04 is fatal |
| `builder.amount_scale(s).pricing(m, u)` | unchanged, or `PricingModel::amount_scale` on the model | the model can now declare it once instead of every call site |
| a `B` document that also carried `S` | now refused at construction | BR-B-02 is decidable from the categories present, so it is checked rather than deferred |
| `from_positions` returning an invalid document | now returns `Err` | the constructor runs `validate()`, so its documented promise is true |
| a BG-20 allowance passed in `.positions(…)` | `.extra_discount(FixedDiscount::new(…)?.boxed())` | the bucket decides the EN 16931 group; an invoice line cannot also be a document level allowance |

## [0.12.0]

The release that turned a domain-flavoured billing library into a domain-agnostic one,
and taught it three things it could not express.

**Naming.** The primary extension point was called `Tariff` — the vocabulary of
utilities and telecoms, in a crate whose own description says "no domain assumptions".
It, the rate types and two aliases are renamed or gone.

**Tax-inclusive pricing.** Consumer prices are quoted gross almost everywhere, and
EN 16931 is uncompromisingly net. The conversion had to be done by hand, and the
obvious way to do it is wrong.

**VAT in the accounting currency.** An invoice may be denominated in any currency, but
the VAT has to be reported in the one the tax is due in. That second figure — BT-111 —
had nowhere to live.

**Early payment discounts.** A Skonto changes what the buyer transfers, has a
structured wire format XRechnung makes mandatory, and must *not* touch the taxable
base. None of that was modelled.

Every rule cited below was re-derived from the CEN and Peppol Schematron rather than
from recollection. That check earned its keep: an earlier draft added a validation for
**BR-CO-25** ("if the amount due is positive, either BT-9 or BT-20 shall be present").
The rule does not exist. The CEN artefacts run BR-CO-20 … BR-CO-24 and then BR-CO-26;
it was dropped before publication, and the check was deleted rather than shipped.

### Changed — `Tariff` is now `PricingModel`, and the rate types are neutral

| Old | New |
|---|---|
| `Tariff` | `PricingModel` |
| `TariffSchedule` | `RateSchedule` |
| `TariffScheduleBuilder` | `RateScheduleBuilder` |
| `TariffBand` | `RateBand` |
| `BillingDocumentBuilder::tariff` | `BillingDocumentBuilder::pricing` |
| `BillingDocumentBuilder::try_tariff` | `BillingDocumentBuilder::try_pricing` |
| module `billing::tariff` | module `billing::pricing` |

*Tariff* is perfectly good English for a published schedule of rates. The sense is as
old as the customs-duty one — both are attested from the 1590s, from Arabic `taʿrīf`
via Italian `tariffa` — and UK primary legislation is titled the *Domestic Gas and
Electricity (Tariff Cap) Act 2018*. The word was not the problem.

Being *the neutral word* was. This crate advertises zero domain assumptions, and every
other public name keeps that promise: `LineItem`, `Quantity`, `UnitPrice`, `TaxLayer`,
`DiscountLayer`, `UsageAggregator`, `AllocationRule`, `AmountScale`, `RateLookup`.
`Tariff`, `TariffSchedule` and `TariffBand` were the only three exceptions out of
roughly forty public names — and they carry the vocabulary of utilities and telecoms
specifically. Two of the four worked examples are SaaS and cloud, where the word is
*plan*, *pricing model* or *rate card*; and in American English the dominant sense of
*tariff* is an import duty, which this crate has nothing to do with.

`PricingModel` is the phrase all of those domains already use for "how we work out what
to charge". `RateSchedule` and `RateBand` join the existing `RateLookup` in a neutral
family of rate tables — and, as a side effect, the old `Tariff` / `TariffSchedule`
near-collision is gone: nobody will now assume a `RateSchedule` implements a
`PricingModel`. (It does not, and could not usefully; the docs say so explicitly.)

`tariff` was also dropped from the crate keywords in favour of `en16931`, which is what
the crate has spent two releases becoming good at.

### Removed — `ScalarTariff`

It existed to spare a settlement-style tariff two tokens: `type Usage = ();` and an
ignored `usage` parameter. For that it charged:

- **a second vocabulary** — `positions()` versus `line_items()`, `settle()` versus
  `bill()`, for the same two operations;
- **a coherence hazard** — implementing both traits for one type was a compile error,
  which the documentation had to warn about in three places;
- **a name that said nothing true.** "Scalar" suggests a single number. The method
  returned a `Vec<LineItem>`. What the trait actually meant was *"takes no usage"*, and
  no reader could have guessed that from the name.

The ergonomics are now on `Tariff` itself. `settle` and `try_settle` are bounded on
`Usage = ()`, so they exist exactly where `&()` would otherwise have to be written at
every call site — and nowhere else.

```rust
// before
impl ScalarTariff for EegSettlement {
    type Error = Infallible;
    type NotBillable = Infallible;
    fn positions(&self) -> Result<Positions<Infallible>, Self::Error> { … }
}

// after
impl Tariff for EegSettlement {
    type Usage = ();
    type Error = Infallible;
    type NotBillable = Infallible;
    fn positions(&self, _: &()) -> Result<Positions<Infallible>, Self::Error> { … }
}
```

`settle(meta)` and `try_settle(meta)` are unchanged at the call site.

### Removed — `EuroAmount` and `InvoiceAmt`

Both were type aliases, both were **unused** anywhere in the crate, and both said
something untrue.

`EuroAmount = Amount<5>` named a currency in a crate whose own design invariant reads
*"No implicit currency — `Currency` defaults to ISO 4217 `XXX`, never `EUR`."* It is
the same defect as `Tariff`, one line further down: a domain assumption smuggled into a
name in a library that advertises none.

`InvoiceAmt = Amount<2>` abbreviated a word for no reason, and implied that two
decimals is "the invoice precision" here. It is not: documents are assembled at
`Amount<5>` and reduced with `AmountScale`, which is the whole point of that type.

Write `Amount<5>` and `Amount<2>`. They are shorter than the explanations.

### Changed — `TouBand` is now `TimeBand`

`Tou` is an abbreviation, sitting next to `TimeOfUsePricing` spelled out in full and,
since this release, `RateBand`. `TimeBand` matches both.

### Added — tax-inclusive (gross) arithmetic

`Amount::extract_tax` returns the tax **contained in** a tax-inclusive amount, and
`Amount::net_of_tax` the rest of it:

```rust
let gross = Amount::<5>::parse("119.00000")?;
assert_eq!(gross.extract_tax(dec!(0.19), MidpointAwayFromZero)?, Amount::parse("19.00000")?);
assert_eq!(gross.net_of_tax(dec!(0.19), MidpointAwayFromZero)?, Amount::parse("100.00000")?);
```

The rate applies to the *net* part, not to the gross, so the extraction is the VAT
fraction `r/(1+r)` — `19/119`, `7/107`, `1/6` at 20 %. The plausible alternative,
`gross × rate`, over-states the tax by `rate²` of the net: `22.61` instead of
`19.00`, which is 19 % too much output tax declared on every B2C line.

`net_of_tax` is the **remainder** rather than an independent rounding of
`gross ÷ (1 + rate)`, so `net + tax == gross` holds exactly for every rate and
every strategy — the number on the price tag survives into BT-112. Rounding both
halves separately breaks that for roughly half of all inputs; `tests/properties.rs`
now pins the law.

`TaxBreakdownEntry::from_gross(category, rate, gross, scale)` does the same split
straight into a BG-23 group. It takes an `AmountScale` rather than a
`RoundingStrategy` on purpose: the exact product is rounded **once**, at the
precision the document will be emitted at. BR-CO-17 defines BT-117 as that single
rounding, and a validator recomputes it that way; extracting at five decimals and
reducing afterwards rounds twice and can land a minor unit away.

### Added — VAT in the accounting currency (BT-6 / BT-111)

New `VatAccountingCurrency` in `billing::settlement`, plus
`BillingDocument::with_vat_accounting_currency` and `vat_accounting_currency()`.

The invoice currency is a commercial choice; the currency VAT is *reported* in is
not. Art. 230 of the VAT Directive lets an invoice be denominated in any currency
but requires the VAT amount in the national currency of the member state where the
tax is due — so a Polish, Czech, Swedish, Danish, Hungarian or Romanian supplier
invoicing in EUR must state the VAT again in PLN, CZK, SEK, DKK, HUF or RON. The
crate could not express that at all, and it is the one remaining EN 16931 gap that
is genuinely *arithmetic* rather than identity data.

EN 16931 models the pair and nothing more: BT-6 names the currency, BT-111 states
the amount. **The exchange rate is not a business term**, so `converted` takes it
as an argument and keeps none of it. Four fatal rules govern the pair, re-derived
from the CEN and Peppol Schematron rather than from memory:

| Rule | Source | Requires | How it holds |
|---|---|---|---|
| **BR-53** | CEN | BT-6 present ⇒ BT-111 provided | one value, so the pair cannot be half-stated |
| **BR-DEC-15** | CEN | BT-111 ≤ 2 decimals | `converted` rounds the exact product **once** at the target `AmountScale`; `fits_amount_scale` and `amount_scale_violation` now cover BT-111 |
| **`PEPPOL-EN16931-R005`** | Peppol | BT-6 ≠ BT-5 | rejected by the setter, the first place both are known |
| **`PEPPOL-EN16931-R055`** | Peppol | BT-110 and BT-111 share an operational sign | new check 14 in `validate()` |

The sign check is in `validate()` rather than only in the setter because
`reverse()`, `AllocationRule` and `merge_period_documents` all reach the field
without passing through it. Each now handles it: a credit note negates BT-111 with
BT-110, allocation splits it with the same last-recipient remainder that keeps
BT-110 exact, and merging sums it. Merging **refuses** two shapes it cannot sum — a
currency mismatch, and one half stating BT-111 while the other does not, where the
missing figure is unstated rather than zero.

`BillingDocument::validate` now checks **fourteen** invariants, not thirteen.

### Added — payment terms and Skonto (BT-20)

New module `billing::terms`, with `PaymentTerms` and `EarlyPaymentDiscount`.

EN 16931 models payment terms as one free-text field, so an early-payment discount
is not machine-readable in the core standard at all. Germany's CIUS embeds a
micro-syntax in that text, made mandatory by **BR-DE-18** and read by ZUGFeRD too:

```text
#SKONTO#TAGE=10#PROZENT=2.00#
#SKONTO#TAGE=10#PROZENT=2.50#BASISBETRAG=485.98#
```

`Display` writes exactly that form and `FromStr` reads it back, strictly — a
lowercase keyword, a missing closing `#` or a third decimal is refused rather than
normalised, because a validator enforcing BR-DE-18 refuses it too. A percentage or
base amount the syntax cannot state is rejected at construction, not silently
rounded at render time.

`PaymentTerms::discount_within(days)` returns the best qualifying window, which is
what staged terms ("3 % in 10 days, 2 % in 20, net 30") mean for a buyer paying on
day five.

`DocumentMeta::payment_terms` carries it, and attaching it moves **nothing**:
not `net_total`, not `gross_total`, not `amount_due`, not the VAT breakdown. A
Skonto is conditional on early payment, which has not happened when the invoice is
issued; deducting it up front would understate the taxable base for a supply that
may well be paid in full. When it *is* taken the consideration shrinks after the
fact and the tax follows — §17 Abs. 1 UStG, art. 90 of the VAT Directive — and that
correction is triggered by a payment event the invoice cannot know. So the crate
computes the payable figure and stops there.

### Added — `BillingDocument::unattributed_positions`

The positions carrying no VAT category: BT-151 on a line, BT-95 on an allowance,
BT-102 on a charge — mandatory under BR-CO-04, BR-32 and BR-37, all fatal.

`verify_vat_attribution` checks that the attributions which *exist* agree with the
breakdown, and passes **vacuously** on a document that has none. That is the right
behaviour — this crate bills things that are not invoices — but it meant "verify
passed" read as a licence to emit when it was only half the question. A per-unit
levy outside a tag-restricted VAT layer's base is the ordinary way to reach that
state, and it was invisible.

### Added — `examples/cross_border_retail.rs`

A Polish shop invoicing a business customer in EUR: a gross-quoted catalogue, VAT
reported in złoty, and 2 % Skonto for payment within ten days. It exercises all
three of this release's additions end to end, and is where the gross-drift
explanation above lives. `just examples` now runs four examples, not three.

Writing it also caught a misuse the crate correctly refused: BT-150 stated without
BT-149. `UnitPrice::validate` rejects that pair, because BT-150 is an attribute of
`cbc:BaseQuantity` in UBL and cannot exist alone.

### Added — a documentation site

New `site/` directory: a [Zola](https://www.getzola.org/) landing page and guide,
deployed to GitHub Pages by `.github/workflows/site.yml`. It is a **guide**, organised
by task, with search and cross-links; the README remains the complete reference and the
doctest source, and `docs.rs` remains the exhaustive API listing.

No framework, no font CDN, no analytics — one hand-written stylesheet, ~50 lines of
vanilla JavaScript for search (index fetched on first keystroke, not on load) and a
theme toggle. A crate whose whole argument is that it has three dependencies should not
need a build pipeline to say so.

Per page: a distinct `<title>` and description, a canonical URL, Open Graph and Twitter
cards, and `TechArticle` JSON-LD tied to the site's `SoftwareSourceCode` node. Zola
generates `sitemap.xml`; `robots.txt` points at it. `zola check` runs in CI on any site
change, so a renamed page that something still links to fails review rather than
becoming a 404.

**The site's Rust examples are compiled and run**, for the same reason the README's
are. It cannot use the README's `include_str!` trick — rustdoc needs a hidden
`# Ok::<(), _>(())` trailer wherever a snippet uses `?`, and Zola would print that line
on the page — so `just site-verify` extracts each fenced block into a git-ignored
integration test instead. It earned its keep immediately: the first run found a wrong
figure in the margin-scheme example (200,00 × 19/119 is 31,93277, not 31,93).

`just site-serve`, `just site-build`, `just site-check`, `just site-verify`. The last
two also run as part of `just ci`.

### Added — tests for functions whose only coverage was a doctest

Excluding doctests from the sweep made a handful of previously-caught mutants survive,
and every one of them was a function with **no test other than an example**:
`LatestAggregator::aggregate` — the only one of the six aggregators without a unit test
— and `Amount::checked_div` among them.

The fix was a unit test each, not a slower sweep. A function tested only by a doctest
is a gap whether or not a mutation run is what reveals it, and the faster configuration
is what made these visible at all.

### Fixed — a `PercentageCharge` could turn into a credit ⚠️ silent

`PercentageCharge::new` refuses a negative rate, with the reason spelled out in its
own error: *"a negative charge is a discount; use a `DiscountLayer` instead"*. The
same result was reachable through a negative **base** — debit positions that net out
below zero, which sustained negative spot prices produce — and it was not refused:

```rust
// 1000 kWh at −0.01 EUR/kWh, then a 5 % platform commission.
// before: −0.50   ← a "charge" that increases what the supplier owes the customer
// now:     0.00   ← nothing to take a percentage of
```

`PercentageDiscount` had clamped a negative base since 0.4.0, with the mirror-image
reasoning already in its source: a discount must not make a credit balance larger.
A charge must not either, and the two layers now agree.

A debit at a negative price still **contributes** to the base and reduces it — that
is what distinguishes it from a `Sign::Credit` position, which is excluded outright.
Only a base that ends up below zero is clamped. The BT-100 basis follows the clamped
base, which matters: stating the unclamped −10.00 against an amount of 0.00 puts the
pair 0.50 apart, and `PEPPOL-EN16931-R040` recomputes exactly that product with a
±0.02 tolerance.

This compiles unchanged and alters a computed amount, so it is marked silent. It
can only affect a document whose charge base is negative overall; on every other
document the result is bit-identical.

### Fixed — `RateSchedule::apply_peak` produced an incomplete invoice line

A capacity charge was built with `LineItem::debit(..).fixed_amount(..)`, which
states an amount and nothing else. BR-22 (BT-129), BR-23 (BT-130) and BR-26
(BT-146) all have `$Invoice_Line` as their Schematron context and apply to every
line without exception, so a consumer had to synthesise all three. It now uses the
`flat_fee` shape — `1 × C62` at the full amount, which satisfies `R120` trivially —
and carries the schedule's own currency in the price label.

### Fixed — documentation that described code that no longer existed

- `BillingError::ValidationFailed` listed `"prepaid_vs_advances"`, a check name
  removed when `Prepayment` was introduced in 0.9.0, and omitted
  `"vat_breakdown_presence"`, `"prepaid_vs_prepayment"`, `"vat_attribution"` and
  `"vat_total"`. The full set is now tabulated by the method that emits it.
- Three `allowance_charge()` implementations — `PercentageCharge`,
  `PercentageDiscount`, `FixedDiscount` — carried a comment copied from
  `PerUnitLevy` explaining that "a per-unit levy is quantity × rate". Each now
  says what its own layer does.
- `Prepayment::itemised`'s error message ran twenty-five spaces together from a
  string that was wrapped without a continuation.
- `minimum_charge` tagged its position with a `"minimum-charge"` string literal
  rather than `tags::MINIMUM_CHARGE`, so the two could drift apart.
- The README stated that `BillingDocument` enforces **eleven** invariants and
  listed eleven. It has enforced thirteen since 0.10.0 — BR-CO-18 and BR-O-11 were
  added without the table following — and fourteen as of this release. The table
  now lists every one, and names the two identities that are deliberately *outside*
  `validate()` because an allocated document cannot preserve them.

### Changed — breaking

| Old | New | Why |
|---|---|---|
| `Tariff` | `PricingModel` | a domain word for the primary extension point of a domain-agnostic crate |
| `TariffSchedule` / `TariffScheduleBuilder` | `RateSchedule` / `RateScheduleBuilder` | joins the existing `RateLookup` in a neutral family of rate tables |
| `TariffBand` | `RateBand` | same |
| module `billing::tariff` | module `billing::pricing` | same |
| `BillingDocumentBuilder::tariff` / `try_tariff` | `::pricing` / `::try_pricing` | same |
| `Tariff::line_items(&self, usage)` | `PricingModel::positions(&self, usage)` | the crate says *positions* everywhere else — `net_positions`, `tax_positions`, `all_positions`, `positions_by_tag`, `BillingDocumentBuilder::positions`, and the return alias `Positions<R>`. `line_items` was the one holdout, and it is also the wrong word: a discount position is not an EN 16931 invoice line |
| `ScalarTariff` | `PricingModel` with `type Usage = ()` | one trait, one vocabulary, no coherence hazard |
| `ScalarTariff::positions(&self)` | `PricingModel::positions(&self, _: &())` | one method, one signature |
| `TouBand` | `TimeBand` | `Tou` is an abbreviation; the neighbours are spelled out |
| `EuroAmount` | `Amount<5>` | removed — named a currency the crate refuses to assume |
| `InvoiceAmt` | `Amount<2>` | removed — abbreviated, and two decimals is not "the invoice precision" here |
| `LineItem::flat_fee(desc, amount)` | `LineItem::flat_fee(desc, amount, currency)` | the generated price label hardcoded `"EUR"` |
| `LineItem::credit_flat_fee(desc, amount)` | `LineItem::credit_flat_fee(desc, amount, currency)` | same |
| — | `DocumentMeta { payment_terms, .. }` | new field (BT-20); use `..Default::default()` |
| — | `BillingDocument` gained a private `vat_accounting_currency` | serde-visible as an optional field; a payload without it deserialises unchanged |

`settle` / `try_settle` moved from `ScalarTariff` to `PricingModel`; both keep their
names and signatures.

`flat_fee` builds a complete EN 16931 line — one unit at a unit price equal to the
whole amount — and generated that price label as `"EUR"` regardless of the document.
Every Swiss, sterling and dollar invoice carried a euro price on its face. It was the
last place in the crate that assumed a currency; `RateSchedule`, `TimeOfUsePricing`,
`DynamicPricing` and `PerUnitLevy` had all taken one since 0.5.0.

`DocumentMeta` is documented as extensible by struct-update syntax, so its new field
breaks only exhaustive struct literals.

### Fixed — the security audit failed on a crate that is never built

`cargo audit` failed CI on **RUSTSEC-2026-0235** (rkyv 0.7: out-of-bounds reads
validating archives containing `Rc`/`Arc`).

rkyv is not a dependency of this crate in any meaningful sense. `rust_decimal`
declares it `optional = true` behind an `rkyv` feature that nothing here enables, and
`cargo tree -i rkyv --all-features` prints *nothing*: it is not built, not linked and
not reachable. The advisory fires because `Cargo.lock` is **feature-agnostic** — it
records every optional dependency any crate in the graph could activate. There is also
nothing to upgrade to; 0.7.46 is the newest 0.7, and moving to 0.8 is `rust_decimal`'s
call.

So it is ignored in a new `.cargo/audit.toml` — **paired with the invariant that
justifies it.** `just audit` now asserts each ignored crate is absent from the resolved
build graph *before* running the audit. Enable `rust_decimal/rkyv` in a future release
and the assertion fails, rather than the ignore silently covering a real exposure. CI
runs `just audit` rather than `cargo audit`, so the check cannot exist only locally.

`deny.toml` deliberately does **not** mirror the ignore. cargo-deny resolves features,
so the optional dependency was never in its graph and it never flagged this — an entry
there would match nothing and quietly stop meaning anything. That difference between
the two tools is now written down where the empty `ignore = []` sits.

Lockfile refreshed while there: 13 patch-level updates, all within the 1.85 MSRV.

### Changed — mutation testing is no longer a CI gate

It was wired into CI on pull requests. It is not any more, and the nightly sweep that
briefly existed has been removed too.

A sweep is ~1 000 mutants. Even after the tuning below that is tens of minutes on a
laptop and considerably worse on a four-core runner, and it buys a signal that changes
rarely. Waiting for it on every pull request is the fastest way to make people stop
reading it. It is now run **by hand, when it is worth running** — `just mutants-gate`,
`just mutants-diff`, `just mutants-file`, or `just mutants-shard i n` to spread a sweep
over several sittings.

CI keeps everything fast and deterministic: fmt, clippy `-D warnings`, the full suite
on three platforms, MSRV, doctests, `cargo-deny`, `cargo-semver-checks`, benchmarks
compiling, plus the site's link check and its Rust examples.

**The tuning stays**, because it makes the ad-hoc runs bearable. Three settings roughly
halved the per-mutant cost — measured on a fixed shard, **239 s → 122 s** — and none of
them changes which mutants are caught:

- `[profile.mutants]` with `debug = false, incremental = false`. The crate is compiled
  a thousand times and debugged zero times, and incremental state never hits because
  every mutant is a different tree.
- `additional_cargo_test_args = ["--lib", "--tests"]`. Doctests were **17 of the
  baseline's 18 test seconds**.
- `just site-verify` now deletes the `tests/site_examples.rs` it generates. Leaving it
  meant every mutant rebuilt and reran a test binary that checks *documentation*.
  (Self-inflicted, and introduced earlier in this same release.)

`just mutants*` derives `--jobs` from the core count instead of running one mutant at a
time, and `just mutants-shard i n` / `just mutants-union-check dir` split a sweep into
slices that can be run separately and checked together.

### Documentation — an over-claim about gross-to-net, corrected

Writing `examples/cross_border_retail.rs` disproved a sentence this release had
already drafted into the README: that converting a gross-quoted catalogue means
"the number on the price tag survives into BT-112". It does not, and the example
prints the counterexample:

| | |
|---|---|
| 3 × 249,99 + 2 × 529,00 + 4 × 89,95 | **2 167,77** — the sum of shelf prices |
| BT-109 1 762,41 + BT-110 405,35 | **2 167,76** — what the invoice states |

Nothing is lost in the conversion. VAT on an invoice is charged **once per
category**, not once per line — BR-CO-17 defines BT-117 as a single rounding of
BT-116 × rate, and a validator recomputes it that way. The per-line taxes sum to
405,36; the tax on the summed base is 405,35. The shelf-price total is the figure
built from independently rounded numbers, and the one with no standing in EN 16931.

`extract_tax` / `net_of_tax` do guarantee `net + tax == gross` exactly — for the
amount they are applied to. The README now says that, and no more, with a section
explaining why forcing the two totals to agree would trade a checkable rule
(BR-CO-17) for an identity the standard does not have.
`tests/invoice_compliance.rs` pins the numbers so the claim cannot drift again.

A second over-claim went with it: that building net positions and calling
`TaxBreakdownEntry::from_gross` "agree, because both round once from the exact
value". They do not, and they are not alternatives. **BR-S-08** fixes BT-116 as the
sum of the line net amounts, so where lines exist the breakdown must follow them;
`from_gross` is for a gross *total* with no lines to sum. Stating one next to the
other is the BR-S-08 violation `verify_vat_attribution` reports, and the README now
tabulates which to use when.

### Documentation — mutation figures replaced with the durable claim

The README quoted "958 mutants generated, 724 caught, 98.2 %". Those were the 0.11.0
numbers, and they were already wrong: the crate has grown by roughly a thousand lines
since. A score printed in a README goes stale the next time a function is added.

What replaces it is the property that does not go stale — every surviving mutant is
**enumerated** in `.cargo/mutants-baseline.txt` with the reason it is tolerated, and
the gate fails both on a new survivor and on a recorded one that is now caught. Run
`just mutants-gate` for the counts.

## [0.11.0]

No API changes. This release comes from pointing a different kind of tool at the
crate: **mutation testing**, via [`cargo-mutants`](https://mutants.rs/).

The suite up to 0.10.0 was checked three ways — example-based tests, property
tests asserting the algebraic laws, and every README example compiled as a
doctest. All three answer "does the code do what I expect?". None answers "would
anything fail if it did something else?", and for a billing engine that is the
question that decides whether a rounding rule is verified or merely exercised.

Mutation testing answers it by rewriting the code — flipping `<` to `<=`, deleting
a guard, replacing a computed amount with its default — and reporting every
rewrite the suite still accepts.

The first sweep generated 958 rewrites, of which 737 compiled. **124 of those
survived**: code that could be changed, in ways that alter what an invoice says,
without a single test failing. One was a bug. The rest were gaps.

After this release, 13 survive — eleven provably equivalent rewrites and two in
branches no input reaches. That is 724 of 737, a **98.2 %** score.

### Fixed — `proportional_split` refused splits it had already accepted ⚠️ silent

`proportional_split` rejected inputs whose shares it had validated, with an
internal-sounding error:

```text
proportional_split: deficit of 4 units exceeds the 3 fractions available to
absorb it (fractions sum to 0.999999999)
```

`proportional_split(dec!(25000000), &[dec!(0.333333333); 3], 2)` was one such
call. The same shares at the same scale split fine at `10_000_000` — a case
0.9.0 added a regression test for — and failed at `25_000_000`.

The Hamilton step assumed the deficit could never exceed one unit per fraction,
and said so in a comment calling the branch unreachable. It has two sources, not
one: flooring each ideal share discards up to one unit per fraction, and the
share-sum tolerance separately admits a drift worth up to another unit per
fraction. Together they put the deficit just under **2n** units, so the guard was
reachable for any total large enough to make a 1e-9 drift worth about a cent.

The distribution now takes `n_units / n` units per fraction and awards the
remaining `n_units % n` in Hamilton order — the same rule, generalised. `Σ(parts)
== total` now holds for every input the share check accepts, which is what the
function's documentation always promised.

**This is a behaviour change with an unchanged signature.** Calls that returned
`Err(InvalidInput)` now return `Ok`. Nothing that previously succeeded changed its
result. Code matching on that error to fall back to a manual split can drop the
fallback; code treating the error as fatal will simply stop seeing it.

The mutation run found this by flipping `n_units > parts.len()` to `>=` and
watching nothing fail — the branch was untested because it was believed
unreachable.

### Added — mutation testing as a first-class gate

- **[`.cargo/mutants.toml`](.cargo/mutants.toml)** — configuration, with the
  reasoning for each setting recorded beside it. `all_features` so the serde
  surface is mutated too, `cap_lints` so a mutant is never scored unviable for a
  warning that describes the mutant rather than the crate.
- **[`.cargo/mutants-baseline.txt`](.cargo/mutants-baseline.txt)** — every
  tolerated survivor, each with the reason it is tolerated. Nothing is suppressed
  with `exclude_re`: an exclusion is a permanent blind spot, since the regex that
  hides `builder -> Default::default()` today also hides it once `builder()`
  starts doing real work. Every mutant is still generated, run and reported.
- **`just mutants-gate`** — the pass/fail form. Runs the sweep and diffs its
  survivors against that baseline, failing both when a mutant survives that is not
  recorded *and* when a recorded one no longer survives, because a stale entry is
  a blind spot too. Line and column numbers are stripped before comparison, so
  ordinary edits do not churn the file.
- **`just mutants`, `mutants-diff`, `mutants-file`, `mutants-iterate`,
  `mutants-list`, `mutants-baseline-update`** — the reporting form and the
  narrower loops used while closing a gap.
- **A diff-scoped CI job.** Pull requests mutate only the lines they changed, so
  new code has to arrive with tests that would notice if it were wrong. The full
  sweep takes tens of minutes and stays on the developer's machine — CI gates the
  delta, `just mutants-gate` gates the release.

### Documentation — where this crate stops

The README argued that XML serialisation and the EN 16931 rule set belong in
separate crates, and then left the reader without a pointer to one. Both now
exist and are named: [`en16931`](https://crates.io/crates/en16931) for the
semantic model and its business rules, and
[`en16931-formats`](https://crates.io/crates/en16931-formats) for UBL 2.1,
UN/CEFACT CII, the XRechnung CIUS and ZUGFeRD / Factur-X.

Neither is a dependency of this crate, and `billing` has no knowledge of either.
The split is the one the README already argued for: `billing` answers *what are
the amounts*, `en16931` answers *is this a lawful invoice*, `en16931-formats`
answers *how does it go on the wire*. What `billing` contributes is the part that
is hard to retrofit — a validator can report that BT-112 ≠ BT-109 + BT-110, but
not which rounding decision three layers earlier made them disagree.

### Added — tests for behaviour that nothing was checking

Each of these closes a surviving mutant, which is to say: each covers code that
could have been changed without a single test failing.

| Area | What was unverified |
|---|---|
| `proportional_split` | a zero share (a recipient billed nothing); that malformed shares are rejected on their own terms rather than only when the total is large enough to make the drift expensive |
| `Amount::allocate` | *which* recipient receives the odd unit — conservation held for any answer, so ranking by quotient instead of by remainder was invisible |
| `Amount::round_to_increment` | `Truncate` on a negative amount (a credit note rounds toward zero, not down); `MidpointToEven` at an exact midpoint, in both directions |
| `Amount::exact_to` | that a narrowing refusal is `InvalidInput` with the "rebuild at that scale" advice, not a generic `PrecisionLoss` |
| `BillingDocument::validate` check 8 | that a VAT breakdown may not exceed, or oppose the sign of, the tax actually charged — the check had **no test at all**, in either the invoice or the credit-note orientation |
| `BillingDocument::verify_vat_attribution` | that it ever returns `Err`; every existing caller unwrapped it on a document expected to be sound |
| `BillingDocument::assert_valid` | that it panics — the method every downstream suite leans on had only a passing case |
| `merge_breakdown` | BT-121 conflicting, matching and one-sided across merged entries (BT-120 was covered; its code twin was not) |
| Assembly of BG-21 | that a layer may declare its charge detail at the layer level, and that VAT positions never receive it |
| `AdvancePayment` | BT-26 and BT-X-292 — set, read back, and surviving a serde round trip through `try_from`, which rebuilds field by field and would have dropped them silently |
| `residual_breakdown` | over-deduction of a zero-rated group, where the tax is pinned at zero and the base is the only half that can go negative; and that a group is dropped only when *both* halves reach zero |
| `AllowanceCharge::check_amount` | that the PEPPOL-EN16931-R040 ±0.02 tolerance is inclusive at exactly 0.02, as its own error message states |
| `LineAllowanceCharge::negated` | that BT-137 is negated alongside BT-136 — R040 compares magnitudes, so a positive base beside a negated amount validated cleanly |
| `LineItemBuilder::build` | that half a price is refused: a quantity without a unit price, or a unit price without a quantity |
| `LineItem::validate` | that a zero quantity is valid — a metered line with no consumption is an ordinary invoice line |
| `Currency` | `TryFrom<&str>` and `FromStr` (a failure returning the default would have produced XXX-labelled documents that still validate); `minor_unit_increment` where the currency's minor units exactly fill the scale |
| `Prepayment::is_none` | that `Itemised(vec![])` and `Total(0)` are not `None`; and the billable side of `Billing::is_billable` / `reason` / `into_billable`, which had only ever been asserted where they answer "nothing to bill" |
| `Period::is_ordered` | that a ten-character string with the wrong separators is declined, that a nine-character one with the right pattern is too, and that one good endpoint does not carry the other — a lexicographic compare orders `YYYY-MM-DD` and nothing else |
| `prorate_amount` | that `active_days == total_days` is the whole amount rather than the error case |
| `TariffSchedule::split` | that graduated tiers are numbered from one on the generated description — the only place the tier index reaches the customer |
| `FixedRateTax` | that `require_tag` actually narrows the taxable base, and that a category forbidding an exemption reason forbids the *code* as well as the text (BR-S-10 names both) |
| `TimeOfUsePricing` / `DynamicPricing` | the plural `bands` / `intervals` setters and `currency`, which no test went through — leaving `XXX/kWh` on an invoice, which reads as a price and is not one |
| Layer accessors | `PerUnitLevy`, `PercentageCharge`, `PercentageDiscount` and `FixedDiscount` reporting their configured rate, unit, currency, BT-105 / BT-98 reason code and BT-102 / BT-95 VAT rather than a default |
| `LineVat` | that a negative rate is refused while a *signed zero* is not; that BT-119 renders as a percentage with trailing zeros stripped; that `has_exemption_reason` can say no |
| `DocumentKind` | that `Display` renders the UNTDID code and honours width, fill and alignment |
| `Amount` `Debug` and serde | that `Debug` carries `P` and the trailing zeros; that a JSON number is rejected with a message naming the decimal-string alternative |
| `BillingError::source` | that a parse failure keeps its cause in the error chain |
| Allocation | that allocated documents are numbered per recipient rather than all inheriting the parent's BT-1 |
| `TaxLayer` / `DiscountLayer` defaults | that a third-party layer overriding neither `vat` nor `allowance_charge` gets neither invented for it — an `AllowanceCharge` conjured from `Default` states a BG-20/BG-21 with no reason at all, which BR-CO-21 forbids, on a document whose every amount is right |
| `CashRounding` | that `increment` and `strategy` report what the rule was built with — a zero increment would make every amount look already-rounded |
| `Tariff` blanket impl | that `discount_layers` is forwarded alongside `tax_layers`; forwarding only the taxes drops the discount and over-bills by its amount plus the tax on it |
| `Amount::allocate` | a **zero ratio** — a recipient allocated nothing. The generator for the conservation property drew ratios from `1..1000`, so it could not produce one; largest-remainder distribution must pass such a recipient over rather than round it up, because a cent paid to someone owed zero is a cent taken from someone owed something |

### Changed — two property tests that could not have failed

Mutation testing grades the tests. Two of the property tests turned out to be
narrower than they read, in ways no mutant could reveal — a mutant changes the
code, and these were limited by their *generators*:

- `proportional_split_conserves_the_total` built fractions summing to **exactly**
  one, forcing the drift term to zero. That is the region in which the bug fixed
  above is unreachable, so the property could never have found it. Two new
  properties cover the drifting region: one asserts the general contract (refuse
  the shares, or split them exactly — never return parts that sum to something
  else), the other pins the reported shape as a law across every total the
  tolerance accepts. Reverting the fix makes the second fail and shrink to
  `total = 19585372, scale = 2` — a different counterexample from the one that
  prompted the fix, which is the point of a property.
- `allocate_conserves_the_total` drew ratios from `1..1000`, so a zero ratio never
  appeared. It now draws from `0..1000`, asserts a zero ratio receives exactly
  zero, and asserts that the *only* ratios `allocate` may refuse are all-zero ones.

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
