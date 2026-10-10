+++
title = "Money and rounding"
description = "Amount<P>, the four sign-symmetric rounding modes, AmountScale and reducing once, and why a Price is not money until it is multiplied by a quantity."
weight = 20
+++

## `Amount<P>`

`Amount<P>` stores money as an `i64` count of `10⁻ᴾ`. The precision is a const
generic, so `Amount<5>` and `Amount<2>` are different types and cannot be added by
accident. `P` defaults to 5, the engine's precision: every amount a `Document`
returns is an `Amount<5>` (written `Amount`) that already fits the document's scale.

| Value | `P` | Raw `i64` |
|---|---|---|
| 0.03456 | 5 | 3 456 |
| 49.99 | 2 | 4 999 |
| −100.00 | 5 | −10 000 000 |

There is no `f64` anywhere. `P ≤ 18` is checked at compile time: every way to obtain
an `Amount<P>` evaluates the bound, so `Amount::<19>::ZERO` does not compile.

### Range

| `P` | Smallest step | `Amount::MAX` |
|---|---|---|
| 2 | 0.01 | 92 233 720 368 547 758.07 |
| 5 | 0.00001 | 92 233 720 368 547.75807 |
| 18 | 10⁻¹⁸ | 9.223372036854775807 |

At `P = 5` that is about 92 trillion units — enough for EUR, not necessarily for IRR,
VND or LBP. Past the range every operation fails with `ErrorKind::Overflow`.

## Parsing and printing

`parse` accepts exactly `[+-]digits[.digits]`: `.` is the only separator, with at
least one digit on each side, no exponent, no spaces, no grouping. Acceptance is by
**value** — trailing zeros past `P` are fine, a non-zero digit past `P` is refused.

```rust
use billing::{Amount, ErrorKind};

assert_eq!(Amount::<2>::parse("1.500")?.to_string(), "1.50");
assert!(matches!(Amount::<2>::parse("1.505").unwrap_err().kind(), ErrorKind::ExcessPrecision { .. }));
// A thousands separator is refused, not misread as a decimal point.
assert!(matches!(Amount::<2>::parse("1,000").unwrap_err().kind(), ErrorKind::Malformed { .. }));
assert!(Amount::<2>::parse(".5").is_err());

// Display: P decimals by default; a precision rounds half away from zero.
let a = Amount::<5>::parse("-1234.56789")?;
assert_eq!(a.to_string(), "-1234.56789");
assert_eq!(format!("{a:.2}"), "-1234.57");
assert_eq!(format!("{:.2}", Amount::<5>::parse("-0.004")?), "0.00"); // no "-0.00"
```

Display rounding is presentation only; reduce emitted or summed amounts with
`AmountScale` or `round_to`.

## No multiplication on `Amount`

There is no `Amount × Decimal` or `Amount ÷ Decimal`: both would hide a rounding.
Multiply `Decimal`s, then reduce **once**:

```rust
use billing::{Amount, AmountScale};
use rust_decimal::dec;

// 10.02629 × 0.19 = 1.9049951 → 1.90.
let tax: Amount = AmountScale::EN16931.reduce(dec!(10.02629) * dec!(0.19))?;
assert_eq!(tax, Amount::parse("1.90")?);

// Rounding to five decimals first gives 1.90500, which then rounds to 1.91.
let twice = AmountScale::EN16931.reduce::<5>(dec!(1.90500))?;
assert_eq!(twice, Amount::parse("1.91")?);
```

Inside a document every leaf — line net, allowance, charge, group tax — is reduced
this way, from its exact value.

## Overflow is visible, never silent

| Operation | On overflow |
|---|---|
| `+`, `-`, `+=`, `-=`, unary `-`, `Sum` | **panics** |
| `checked_add`, `checked_sub`, `checked_neg`, `checked_abs`, `checked_sum` | `Err(Overflow)` |
| `parse`, `from_decimal`, `from_decimal_rounded`, `round_to`, `round_to_increment`, `rescale`, `exact`, `split` | `Err`, never a panic |

Use the `checked_*` family for anything fed by untrusted input.

## `Rounding`: four modes, all sign-symmetric {#rounding}

```rust
use billing::{Amount, Rounding};
use rust_decimal::dec;

let r = |x, m| Amount::<2>::from_decimal_rounded(x, m).map(|a| a.to_string());
assert_eq!(r(dec!(2.345), Rounding::HalfAwayFromZero)?, "2.35"); // commercial; EN 16931
assert_eq!(r(dec!(2.345), Rounding::HalfEven)?, "2.34");         // banker's
assert_eq!(r(dec!(2.341), Rounding::AwayFromZero)?, "2.35");
assert_eq!(r(dec!(2.349), Rounding::TowardZero)?, "2.34");

// round(−x) == −round(x) for every mode.
assert_eq!(r(dec!(-2.345), Rounding::HalfAwayFromZero)?, "-2.35");
assert_eq!(r(dec!(-2.341), Rounding::AwayFromZero)?, "-2.35");
```

There is no floor or ceiling: with a sign-symmetric mode a credit rounds exactly like
the mirrored debit, which is what makes [`Document::negated`](../corrections/) exact.

## `AmountScale`

An `AmountScale` is a document's interchange scale: how many decimals derived amounts
carry (0 to 5) and how they are rounded. It belongs to the *format*, not the currency.

```rust
use billing::{Amount, AmountScale, ErrorKind, Rounding};
use rust_decimal::dec;

assert_eq!(AmountScale::EN16931, AmountScale::new(2, Rounding::HalfAwayFromZero)?);

let whole_yen = AmountScale::new(0, Rounding::HalfAwayFromZero)?;
assert_eq!(whole_yen.reduce::<5>(dec!(1234.5))?, Amount::parse("1235")?);

// More decimals than the engine's precision is refused.
assert_eq!(AmountScale::new(6, Rounding::HalfEven).unwrap_err().kind(), &ErrorKind::ScaleMismatch);
```

## Narrowing an `Amount`

| You want | Use | If it does not fit |
|---|---|---|
| fewer decimals, same type | `round_to(decimals, rounding)` | rounds |
| another precision, rounding | `rescale::<Q>(rounding)` | rounds |
| another precision, exactly | `exact::<Q>()` | `Err(ExcessPrecision)` |
| to a cash increment | `round_to_increment(increment, rounding)` | rounds |
| to know | `fits(decimals)` | `false` |

```rust
use billing::{Amount, Rounding};

let a = Amount::<5>::parse("10.03456")?;
assert_eq!(a.round_to(2, Rounding::HalfEven)?.to_string(), "10.03000");
assert_eq!(a.rescale::<2>(Rounding::HalfAwayFromZero)?.to_string(), "10.03");
assert!(a.exact::<2>().is_err());
assert!(!a.fits(2));

let five = Amount::<5>::parse("0.05")?;
assert_eq!(a.round_to_increment(five, Rounding::HalfAwayFromZero)?.to_string(), "10.05000");
assert_eq!((-a).round_to_increment(five, Rounding::HalfAwayFromZero)?.to_string(), "-10.05000");
```

Splitting an amount exactly — `Amount::split` and the free `split` — is on
[Splitting](../allocation/).

## `Price`, `UnitPrice`, `Quantity`, `Percent`

A **price is not money.** 0.123456 EUR/kWh is a lawful item net price (BT-146); it
becomes money only when multiplied by a quantity and reduced once. So `Price` wraps an
exact `Decimal` of any precision.

| Type | Business terms | Notes |
|---|---|---|
| `Price` | BT-146 | exact `Decimal`; must be ≥ 0 where it is used (`BR-27`) |
| `UnitPrice` | BT-146, BT-149, BT-150 | a `Price` per base quantity (default 1) in an optional unit |
| `Quantity` | BT-129, BT-130 | signed value and a unit code; **the sign is the direction** |
| `Percent` | BT-96, BT-119, BT-138 … | `Percent::new(dec!(19))` is 19 %; every percentage input takes one, a bare `Decimal` does not compile |

```rust
use billing::{Line, Price, Quantity, UnitPrice};
use rust_decimal::dec;

// 12.00 per 100 kg, 250 kg delivered: 250 × 12 / 100 = 30.
let per_100kg = UnitPrice::new(Price::new(dec!(12))).with_base_quantity(dec!(100)).with_unit("KGM");
let line = Line::new("gravel", Quantity::new(dec!(250), "KGM"), per_100kg);
assert_eq!(line.exact_net()?, dec!(30));

// A refund is a negative quantity at a non-negative price.
let refund = Line::fixed("refund", dec!(-50));
assert_eq!(refund.quantity().value(), dec!(-1));
assert_eq!(refund.price().price().value(), dec!(50));
```

Constructors are infallible; the EN 16931 checks on them — price ≥ 0 (`BR-27`), base
quantity > 0 (`PEPPOL-EN16931-R121`), price unit = quantity unit
(`PEPPOL-EN16931-R130`) — run inside `build()` and come back with the line's
[location](../errors/).

## `Currency`

A `Currency` is three ASCII letters, stored uppercase. The ISO 4217 code list
(`BR-CL-04`) is the format layer's check. There is no `Default`.

```rust
use billing::Currency;

assert_eq!(Currency::new("pln")?.code(), "PLN");
assert!(Currency::new("EURO").is_err());
const NOK: Currency = Currency::of(*b"NOK").unwrap(); // a bad code fails to compile
assert_eq!(NOK.to_string(), "NOK");
```

## Serialisation

With the `serde` feature, `Amount`, `Price` and every `Decimal` field serialise as
decimal **strings** in the `parse` grammar; JSON numbers are refused.

```rust
use billing::Amount;

let a: Amount = serde_json::from_str("\"12.50\"")?;
assert_eq!(serde_json::to_string(&a)?, "\"12.50000\"");
assert!(serde_json::from_str::<Amount>("12.5").is_err());
```
