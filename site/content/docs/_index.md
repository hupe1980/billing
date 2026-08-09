+++
title = "Documentation"
description = "How to price usage, compose tax and discount layers, and assemble invoice documents whose totals and decimals satisfy EN 16931 — with the rules cited so you can check them."
sort_by = "weight"
template = "section.html"
page_template = "page.html"
insert_anchor_links = "left"
+++

`billing` is a calculation library. You describe what was supplied and how it is
priced; it produces amounts, a VAT breakdown and a self-validating document. It never
touches a network, a clock, a database or a file.

These pages are the narrative guide. The
[API reference on docs.rs](https://docs.rs/billing) is the exhaustive one, and every
Rust example in the crate's README is compiled and run as a doctest, so anything you
see there works against the released version.

## Where to start

If you are **evaluating the crate**, read [Getting started](getting-started/) and then
[E-invoicing and precision](e-invoicing/) — the second is the argument for why this
exists rather than a rounding call at the serialiser.

If you are **implementing a pricing model**, read [Pricing models](pricing/) and
[Tax and discount layers](tax-and-discounts/).

If you are **emitting EN 16931** — XRechnung, Peppol BIS, ZUGFeRD, Factur-X — read
[The totals chain](totals/), [VAT breakdown](vat-breakdown/) and
[E-invoicing and precision](e-invoicing/), in that order.

## A note on rule citations

Business rules are cited by their identifier (`BR-CO-15`, `PEPPOL-EN16931-R040`) and
by what they actually assert, because summaries of these rules are frequently wrong —
including, occasionally, summaries published by tooling vendors. Every rule named here
was read from the CEN abstract Schematron or the Peppol BIS Schematron, not from a
secondary source. Where a rule people commonly cite does **not** exist, that is said
plainly too.

This is documentation for a calculation library. It is not legal or tax advice.
