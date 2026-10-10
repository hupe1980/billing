//! [`Currency`]: a validated three-letter code (BT-5), nothing more.

use core::fmt;

use crate::{ErrorKind, Result};

/// A currency code: three ASCII letters, stored uppercase.
///
/// The engine never interprets the code: ISO 4217 membership is the format
/// layer's check, and the decimals come from the format's
/// [`AmountScale`](crate::AmountScale), not the currency. No `Default`: a
/// document's currency is always stated.
///
/// ```
/// use billing::Currency;
///
/// let pln = Currency::new("pln")?;
/// assert_eq!(pln.code(), "PLN");
/// assert_eq!(Currency::EUR.to_string(), "EUR");
/// assert!(Currency::new("EURO").is_err());
///
/// const NOK: Currency = Currency::of(*b"NOK").unwrap();
/// assert_eq!(NOK.code(), "NOK");
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    /// Euro.
    pub const EUR: Self = Self(*b"EUR");
    /// US dollar.
    pub const USD: Self = Self(*b"USD");
    /// Pound sterling.
    pub const GBP: Self = Self(*b"GBP");
    /// Swiss franc.
    pub const CHF: Self = Self(*b"CHF");
    /// Yen.
    pub const JPY: Self = Self(*b"JPY");
    /// ISO 4217's "no currency involved" code.
    pub const XXX: Self = Self(*b"XXX");

    /// A code from three ASCII letters, uppercased.
    ///
    /// # Errors
    /// [`ErrorKind::InvalidCurrency`] unless `code` is exactly three ASCII
    /// letters.
    pub fn new(code: &str) -> Result<Self> {
        match code.as_bytes() {
            &[a, b, c] if [a, b, c].iter().all(u8::is_ascii_alphabetic) => Ok(Self([
                a.to_ascii_uppercase(),
                b.to_ascii_uppercase(),
                c.to_ascii_uppercase(),
            ])),
            _ => Err(ErrorKind::InvalidCurrency { input: code.into() }.into()),
        }
    }

    /// A code usable in a `const` item, uppercased; `None` unless all three
    /// bytes are ASCII letters. Never panics: in a `const` item,
    /// `Currency::of(*b"NOK").unwrap()` turns an invalid code into a compile
    /// error.
    #[must_use]
    pub const fn of(code: [u8; 3]) -> Option<Self> {
        if code[0].is_ascii_alphabetic()
            && code[1].is_ascii_alphabetic()
            && code[2].is_ascii_alphabetic()
        {
            Some(Self([
                code[0].to_ascii_uppercase(),
                code[1].to_ascii_uppercase(),
                code[2].to_ascii_uppercase(),
            ]))
        } else {
            None
        }
    }

    /// The three-letter code.
    #[must_use]
    pub fn code(&self) -> &str {
        // Three ASCII letters by construction.
        core::str::from_utf8(&self.0).unwrap_or("???")
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.code())
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Currency({})", self.code())
    }
}

impl core::str::FromStr for Currency {
    type Err = crate::Error;
    fn from_str(s: &str) -> Result<Self> {
        Self::new(s)
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for Currency {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.code())
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for Currency {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        crate::decimal::serde_str::from_str(d, Self::new, "a three-letter currency code")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::format;

    #[test]
    fn new_validates_and_uppercases() {
        assert_eq!(Currency::new("eur").unwrap(), Currency::EUR);
        assert_eq!(Currency::new("Chf").unwrap(), Currency::CHF);
        for bad in ["", "EU", "EURO", "E1R", "€UR", " EU", "ÄBC"] {
            assert_eq!(
                Currency::new(bad).unwrap_err().kind(),
                &ErrorKind::InvalidCurrency { input: bad.into() }
            );
        }
    }

    #[test]
    fn const_constructor_and_formatting() {
        const SEK: Currency = Currency::of(*b"sek").unwrap();
        assert_eq!(SEK.code(), "SEK");
        assert_eq!(format!("{SEK}"), "SEK");
        assert_eq!(format!("{SEK:?}"), "Currency(SEK)");
        assert_eq!("gbp".parse::<Currency>().unwrap(), Currency::GBP);
    }

    #[test]
    fn const_constructor_refuses_bad_input_without_panicking() {
        assert_eq!(Currency::of(*b"E1R"), None);
        assert_eq!(Currency::of(*b"   "), None);
    }
}
