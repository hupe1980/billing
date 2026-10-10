+++
title = "Documentation"
description = "How to rate usage, attach VAT, derive EN 16931 totals, split documents exactly and settle them — with the rules cited so you can check them."
sort_by = "weight"
template = "section.html"
page_template = "page.html"
insert_anchor_links = "left"
+++

`billing` is a calculation library: you state what was supplied, at what price and
under which VAT treatment; it derives the line nets, the VAT breakdown and every
EN 16931 total. No I/O.

These pages are the guide; [docs.rs](https://docs.rs/billing) is the API reference.
Every Rust example here is compiled and run against the crate.

## Where to start

- **Evaluating:** [Getting started](getting-started/), then
  [EN 16931 and scope](e-invoicing/).
- **Rating usage:** [Metering](metering/), [Pricing](pricing/),
  [Allowances, charges and levies](tax-and-discounts/).
- **Emitting EN 16931** (XRechnung, Peppol BIS, ZUGFeRD, Factur-X):
  [The totals chain](totals/), [VAT](vat-breakdown/), [Errors](errors/).
- **Upgrading:** [Migrating from 0.15](migrating/).

Every rule id cited here exists in the pinned CEN, Peppol or KoSIT Schematron and is
hash-locked in `citations.lock`. This is not legal or tax advice.
