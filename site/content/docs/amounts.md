+++
title = "Amounts and rounding"
description = "Fixed-point money with the precision in the type: no floating point, no silent overflow, and no rounding you did not ask for by name."
weight = 20
+++

## `Amount<P>`

`Amount<P>` stores money as an `i64` scaled by `10^P`. The precision is a const
generic, so `Amount<5>` and `Amount<2>` are different types and cannot be added by
accident.

| Value | `P` | Raw `i64` |
|---|---|---|
| 0.03456 | 5 | 3 456 |
| 49.99 | 2 | 4 999 |
| −100.00 | 5 | −10 000 000 |

There is **no `f64`** anywhere in the arithmetic path. `P > 18` is a compile-time
error, because `10¹⁹` exceeds `i64::MAX`.

### Range

Precision is bought with range, one digit at a time:

| `P` | Smallest step | `Amount::MAX` |
|---|---|---|
| 2 | 0.01 | 92 233 720 368 547 758.07 |
| 5 | 0.00001 | 92 233 720 368 547.75807 |
| 9 | 0.000000001 | 9 223 372 036.854775807 |
| 18 | 10⁻¹⁸ | 9.223372036854775807 |

`Amount<5>` therefore tops out around 92 trillion currency units — far
above any single invoice, which is why the backing type is `i64` rather than `i128`.
If you do aggregate past it, sum at a coarser precision rather than reaching for a
wider integer.

> There is deliberately no `EuroAmount` or `InvoiceAmt` alias. Both existed until
> 0.12.0 and both were unused: `Amount<5>` says what the first said without claiming a
> currency this crate is careful never to assume, and `Amount<2>` says what the second
> said without implying two decimals is "the invoice precision" — documents are
> assembled at `Amount<5>` and reduced with `AmountScale`.

## Overflow is visible, never silent

Every operation is either infallible-and-panicking or fallible-and-total. Nothing
wraps and nothing saturates.

| Operation | On overflow |
|---|---|
| `+`, `-`, `+=`, `-=`, unary `-`, `sum()` | **panics** |
| `from_int`, `abs`, `mul_qty`, `round_to` | **panics** |
| every `checked_*`, `from_decimal_rounded`, `distribute`, `allocate`, `round_to_increment` | returns `Err` |

The `checked_*` family is **total**: it returns `Err` and never panics, including
where the underlying `rust_decimal` operators would. That matters more than it
sounds — `Decimal`'s `Mul`, `Div` and `Sum` panic on overflow rather than saturating,
so a method that merely forwarded to them would break its own `Result` contract on
attacker-controlled input.

## Rounding is always explicit

`RoundingStrategy` is a required parameter. There is no implicit `round()` anywhere
in the crate, and the two `Decimal` conversion paths agree about it:

```rust
use billing::{Amount, BillingError, RoundingStrategy};
use rust_decimal::dec;

// Exact conversion: excess precision is refused, not rounded away.
assert!(matches!(
    Amount::<5>::checked_from_decimal(dec!(0.123456)),
    Err(BillingError::PrecisionLoss { .. })
));
// The same value is refused when it arrives as text.
assert!(Amount::<5>::parse("0.123456").is_err());

// Rounding is the opt-in, and it makes you name the strategy.
assert_eq!(
    Amount::<5>::from_decimal_rounded(dec!(0.123456), RoundingStrategy::MidpointAwayFromZero)?,
    Amount::parse("0.12346")?
);
```

Earlier releases rounded the `Decimal` conversion while `parse` rejected it, so the
same unrepresentable price was refused when it arrived as text and quietly altered
when it arrived as a `Decimal`. That asymmetry is gone.

### The five strategies

`MidpointAwayFromZero` (commercial), `MidpointToEven` (banker's), `Ceiling`, `Floor`,
`Truncate`. They differ only at the midpoint and in sign handling, and the crate
never picks one for you at a boundary where the choice is visible.

## Narrowing without losing money

Three operations, three different answers to "what if it does not fit":

| Method | If the value does not fit | Use when |
|---|---|---|
| `exact_to::<Q>()` | `Err` | losing precision would be a bug |
| `checked_round_to::<Q>(strategy)` | rounds | losing precision is a decision you are making |
| `fits_scale(n)` | — (returns `bool`) | you want to ask before acting |

```rust
use billing::Amount;

let exact = Amount::<5>::parse("356.80000")?;
assert_eq!(exact.exact_to::<2>()?, Amount::<2>::parse("356.80")?);

// 356.80221 does not fit two decimals — say so rather than round it away.
let inexact = Amount::<5>::parse("356.80221")?;
assert!(inexact.exact_to::<2>().is_err());
assert!(!inexact.fits_scale(2));
```

The right response to that `Err` on a document is **not** to round: rounding the
leaves and the aggregates independently breaks the totals identities. Rebuild with
[`amount_scale`](../e-invoicing/) instead.

## Splitting money exactly

Naive division loses or invents cents. Both splitters here are exact by construction.

```rust
use billing::Amount;

// 0.10 three ways — not 0.033… three times.
let parts = Amount::<2>::parse("0.10")?.distribute(3)?;
assert_eq!(parts, vec![
    Amount::<2>::parse("0.04")?,
    Amount::<2>::parse("0.03")?,
    Amount::<2>::parse("0.03")?,
]);

// By integer ratios, largest-remainder: someone takes the extra cent.
let seats = Amount::<2>::parse("100.00")?.allocate(&[1, 1, 1])?;
assert_eq!(seats[0], Amount::<2>::parse("33.34")?);
let sum: Amount<2> = seats.into_iter().sum();
assert_eq!(sum, Amount::<2>::parse("100.00")?);
```

`distribute` spreads the indivisible remainder one smallest-unit at a time, so no
part absorbs it all. `allocate` uses the largest-remainder (Hamilton) method with
ties broken by position — deterministic, no hidden randomness.

For a physical quantity rather than money, use
[`proportional_split`](../allocation/#splitting-a-raw-quantity); for a whole document,
[`AllocationRule`](../allocation/).

## Cash rounding

Several jurisdictions withdrew their smallest coins and round the amount actually
*tendered*: 0.05 in Switzerland, Belgium, Ireland, Italy, Canada and Australia; 0.10
in New Zealand; 0.50 in Denmark; 1.00 in Sweden and Norway.

```rust
use billing::{Amount, RoundingStrategy};

let increment = Amount::<5>::parse("0.05000")?;
assert_eq!(
    Amount::<5>::parse("12.34000")?
        .round_to_increment(increment, RoundingStrategy::MidpointAwayFromZero)?,
    Amount::parse("12.35000")?
);
```

There is deliberately **no** `for_currency` helper and no default increment: the
increment is a payment-law fact, not a currency fact. CHF has two ISO minor units but
rounds cash to 0.05; EUR rounds to 0.05 in Belgium and not at all in Germany. The
midpoint rule varies too — Norway legislates 0.50 up, Denmark leaves 0.25 and 0.75
undefined in the statute, and New Zealand leaves the 5-cent case to the retailer.

At document level this is `CashRounding`, which stores the rule so that a later change
to the paid amount recomputes the adjustment instead of leaving a stale one. See
[The totals chain](../totals/#settlement-bt-113-bt-114-bt-115).

## Comparing amounts

`within_tolerance_ppm` returns `bool`, not `Result`:

```rust
use billing::Amount;

let stated   = Amount::<5>::parse("100.00000")?;
let computed = Amount::<5>::parse("100.50000")?;
assert!(stated.within_tolerance_ppm(computed, 10_000));   // within 1 %
assert!(!stated.within_tolerance_ppm(computed, 4_000));   // not within 0.4 %
```

It used to return `Result`, and every real call site collapsed that with
`.unwrap_or(false)` — which reported *"outside tolerance"* for what was actually an
internal arithmetic failure, in exactly the checks written to catch billing
discrepancies. Widening the subtraction to `i128` removed the failure mode instead of
reporting it.

## Currency

`Currency` is an ISO 4217 alphabetic code stored as three bytes — `Copy`, no
allocation. It performs no conversion and no rate lookup. It exists for two reasons:
generated unit-price labels (`"EUR/kWh"`), and refusing to combine documents that are
denominated differently.

The default is `Currency::XXX`, ISO 4217 for *"no currency involved"* — deliberately
not `EUR`. A document that reaches production still labelled `XXX` is a visible bug; a
silent `EUR` default is not.

> **A consumer must reject `XXX` explicitly.** EN 16931 will not do it for you:
> `BR-CL-04` requires BT-5 to be a valid ISO 4217 alpha-3 code, and `XXX` is one.
> `Currency::is_unset()` is the check to run before emitting.

`minor_units()` reports the ISO exponent, and distinguishes *zero decimals* (JPY) from
*no fractional subdivision at all* (`XAU`, `XXX`), which are not the same thing.

## Serialisation

With the `serde` feature, `Amount<P>` serialises as a decimal **string**
(`"0.03456"`), never as its raw scaled integer and never as a JSON number.

The raw integer would be meaningless without knowing `P` out of band — `3456` is
`0.03456` at `P = 5` and `34.56` at `P = 2`, so a change of precision would silently
rescale every stored value. A JSON number invites float round-tripping, which is the
one thing a fixed-point monetary type exists to prevent. Deserialisation goes through
`parse`, so excess precision is rejected rather than truncated.
