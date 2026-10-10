+++
title = "Gross prices"
description = "Consumer prices are quoted gross and EN 16931 is net. split_gross applies the VAT fraction once — and the invoice still will not always total the shelf prices."
weight = 45
+++

## The conversion people get wrong

Consumer prices are quoted **including tax** (§1 PAngV and its equivalents); EN 16931
is **net**. The tax inside a gross price is the fraction `r / (100 + r)` — `19/119` at
19 % — not the gross times the rate.

`split_gross` applies the fraction **once** at the document's scale and takes the net
as the remainder, so the halves re-add to the gross exactly:

```rust
use billing::{Amount, AmountScale, Percent, Vat, split_gross};
use rust_decimal::dec;

let vat = Vat::standard(Percent::new(dec!(19)))?;
let (net, tax) = split_gross(dec!(119.00), &vat, AmountScale::EN16931)?;
assert_eq!((net, tax), (Amount::parse("100.00")?, Amount::parse("19.00")?));

// The plausible, wrong answer: 119.00 × 19 % = 22.61.
assert_eq!(AmountScale::EN16931.reduce::<5>(dec!(119.00) * dec!(0.19))?, Amount::parse("22.61")?);

// A shelf price of 9.99: 9.99 × 19/119 = 1.595… → 1.60, net 8.39.
let (net, tax) = split_gross(dec!(9.99), &vat, AmountScale::EN16931)?;
assert_eq!((net, tax), (Amount::parse("8.39")?, Amount::parse("1.60")?));
assert_eq!(net + tax, Amount::parse("9.99")?);
```

The gross must fit the scale (`ErrorKind::ExcessPrecision` otherwise).

## Invoicing a gross-quoted basket

Split each line's gross, invoice the nets, and let the document derive the VAT per
group:

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Vat, split_gross};
use rust_decimal::dec;

let vat = Vat::standard(Percent::new(dec!(19)))?;
let scale = AmountScale::EN16931;
let shelf = [("TV", dec!(749.97)), ("fridge", dec!(1058.00)), ("cables", dec!(359.80))];

let mut builder = Document::builder(Currency::EUR, scale).default_vat(vat.clone());
let mut line_taxes = Amount::ZERO;
for (item, gross) in shelf {
    let (net, tax) = split_gross(gross, &vat, scale)?;
    line_taxes += tax;
    builder = builder.line(Line::fixed(item, net.to_decimal()));
}
let doc = builder.build()?;

assert_eq!(doc.tax_exclusive_total(), Amount::parse("1821.66")?);
assert_eq!(line_taxes, Amount::parse("346.11")?);       // Σ of the per-line taxes
assert_eq!(doc.vat_total(), Amount::parse("346.12")?);  // the group's tax, rounded once
assert_eq!(doc.tax_inclusive_total(), Amount::parse("2167.78")?); // the shelf prices sum to 2167.77
```

## The invoice will not always total the shelf prices {#shelf-prices}

That cent is not a bug: `BR-CO-17` derives VAT from the *group's* base, and the sum of
shelf prices has no standing in EN 16931. The gap is at most about one unit per line.

And **not every gross total is reachable**: at 19 % the gross steps by one or two
cents per cent of net:

```rust
use billing::{Amount, AmountScale, Currency, Document, Line, Percent, Vat};
use rust_decimal::dec;

let gross = |net| -> Result<Amount, billing::Error> {
    Ok(Document::builder(Currency::EUR, AmountScale::EN16931)
        .default_vat(Vat::standard(Percent::new(dec!(19)))?)
        .line(Line::fixed("basket", net))
        .build()?
        .tax_inclusive_total())
};
assert_eq!(gross(dec!(25.18))?, Amount::parse("29.96")?);
assert_eq!(gross(dec!(25.19))?, Amount::parse("29.98")?); // 29.97 has no net
```

> **Do not force the two to agree.** For a contractual gross minimum or maximum, use
> a [gross `Bound`](../corrections/#bounds).

## Margin schemes

Under a margin scheme (§25a UStG, arts. 306–325 of the VAT Directive) the tax is
contained in the **margin**, not the selling price:

```rust
use billing::{Amount, AmountScale, Percent, Vat, split_gross};
use rust_decimal::dec;

let (purchase, selling) = (dec!(800.00), dec!(1000.00));
let (_, tax) = split_gross(selling - purchase, &Vat::standard(Percent::new(dec!(19)))?, AmountScale::EN16931)?;
assert_eq!(tax, Amount::parse("31.93")?); // 200.00 × 19/119, not 200.00 × 19 % = 38.00
```

What the invoice shows under a margin scheme belongs to the format layer.
