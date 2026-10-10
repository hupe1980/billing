//! Text in: `Amount::<P>::parse` for P ∈ {0, 2, 5}, `Currency::new`, and `Price`'s
//! string deserialiser.
//!
//! Asserts: no panic; an accepted value round-trips through its own rendering;
//! and `parse` agrees with `from_decimal(Decimal::from_str_exact(s))` wherever
//! both are defined (the crate grammar is narrower than `rust_decimal`'s, and
//! `parse` accepts trailing zeros past 28 digits that `Decimal` cannot hold, so
//! agreement is only asserted on inputs both sides accept).
#![no_main]

use billing::{Amount, Currency, Price};
use libfuzzer_sys::fuzz_target;
use rust_decimal::Decimal;

fn amount<const P: u8>(s: &str) {
    let parsed = Amount::<P>::parse(s);
    if let Ok(a) = parsed {
        let shown = a.to_string();
        assert_eq!(
            Amount::<P>::parse(&shown).ok(),
            Some(a),
            "Display round-trip of {s:?} via {shown:?}"
        );
        assert_eq!(
            Amount::<P>::from_decimal(a.to_decimal()).ok(),
            Some(a),
            "to_decimal round-trip of {s:?}"
        );
    }
    if let (true, Ok(d)) = (in_grammar(s), Decimal::from_str_exact(s)) {
        match (parsed, Amount::<P>::from_decimal(d)) {
            (Ok(a), Ok(b)) => assert_eq!(a, b, "parse vs from_decimal on {s:?}"),
            (Ok(a), Err(e)) => panic!("parse accepted {s:?} as {a} but from_decimal refused: {e}"),
            (Err(e), Ok(b)) => panic!("from_decimal accepted {s:?} as {b} but parse refused: {e}"),
            (Err(_), Err(_)) => {}
        }
    }
}

/// `[+-]digits[.digits]`, the documented grammar, written independently of the crate.
fn in_grammar(s: &str) -> bool {
    let body = s.strip_prefix(['+', '-']).unwrap_or(s);
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    match body.split_once('.') {
        Some((int, frac)) => digits(int) && digits(frac),
        None => digits(body),
    }
}

fn currency(s: &str) {
    if let Ok(c) = Currency::new(s) {
        assert_eq!(c.code().len(), 3);
        assert!(
            c.code().bytes().all(|b| b.is_ascii_uppercase()),
            "{s:?} → {:?}",
            c.code()
        );
        assert_eq!(Currency::new(c.code()).ok(), Some(c));
    }
}

fn price(s: &str) {
    let Ok(json) = serde_json::to_string(s) else {
        return;
    };
    if let Ok(p) = serde_json::from_str::<Price>(&json) {
        let out = serde_json::to_string(&p).expect("an accepted Price serialises");
        let back: Price = serde_json::from_str(&out).expect("a serialised Price deserialises");
        assert_eq!(back, p);
        assert_eq!(
            serde_json::to_string(&back).expect("serialises"),
            out,
            "Price re-serialises identically"
        );
    }
}

fuzz_target!(|data: &[u8]| {
    let Ok(s) = core::str::from_utf8(data) else {
        return;
    };
    amount::<0>(s);
    amount::<2>(s);
    amount::<5>(s);
    currency(s);
    price(s);
});
