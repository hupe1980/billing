//! [`Currency`] — an ISO 4217 alphabetic currency code.
//!
//! The engine performs **no** conversion, no rate lookup, and no formatting
//! beyond the three-letter code itself.  `Currency` exists for two reasons:
//!
//! 1. **Label generation** — [`crate::TariffSchedule`] and friends build unit-price
//!    labels such as `"EUR/kWh"`.  Before `Currency` existed these were hardcoded
//!    to `EUR`, which silently mislabelled every non-euro invoice.
//! 2. **Mixing protection** — [`crate::merge_period_documents`] refuses to combine
//!    documents whose currencies differ, turning a silent arithmetic error into
//!    [`crate::BillingError::CurrencyMismatch`]. (Allocation splits a single
//!    document, so it cannot mix currencies in the first place.)
//!
//! # No implicit default
//!
//! [`Currency::XXX`] is the ISO 4217 code for "no currency involved" and is what
//! [`DocumentMeta::default`](crate::DocumentMeta) and every builder start with.
//! It is deliberately *not* `EUR`: a document that reaches production still
//! labelled `XXX` is a visible bug, whereas a silent `EUR` default is not.
//!
//! ```rust
//! use billing::Currency;
//!
//! let eur = Currency::new("EUR").unwrap();
//! assert_eq!(eur.code(), "EUR");
//! assert_eq!(eur.to_string(), "EUR");
//!
//! // Lowercase input is normalised to uppercase.
//! assert_eq!(Currency::new("usd").unwrap(), Currency::USD);
//!
//! // Anything that is not exactly three ASCII letters is rejected.
//! assert!(Currency::new("EURO").is_err());
//! assert!(Currency::new("E1R").is_err());
//! assert!(Currency::new("").is_err());
//! ```

use std::fmt;
use std::str::FromStr;

use crate::error::BillingError;

/// An ISO 4217 alphabetic currency code (exactly three ASCII letters).
///
/// Stored as three uppercase bytes — `Copy`, no heap allocation.
///
/// Construct with [`Currency::new`] (validating) or use one of the associated
/// constants ([`Currency::EUR`], [`Currency::USD`], …).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Currency([u8; 3]);

impl Currency {
    /// ISO 4217 `XXX` — "no currency involved". The default for every builder.
    ///
    /// Seeing `XXX` on a rendered invoice means a currency was never configured.
    ///
    /// # A downstream consumer must reject it explicitly
    ///
    /// EN 16931 will not do it for you. `XXX` is a real ISO 4217 code, and rule
    /// **BR-CL-04** ("Invoice currency code MUST be coded using ISO code list
    /// 4217 alpha-3") accepts it along with the other 177 — so a document that was
    /// never configured validates cleanly as an EN 16931 invoice claiming no
    /// currency is involved. [`Currency::is_unset`] is the check to run at the
    /// boundary before emitting.
    pub const XXX: Self = Self(*b"XXX");
    /// Euro.
    pub const EUR: Self = Self(*b"EUR");
    /// US dollar.
    pub const USD: Self = Self(*b"USD");
    /// Pound sterling.
    pub const GBP: Self = Self(*b"GBP");
    /// Swiss franc.
    pub const CHF: Self = Self(*b"CHF");
    /// Japanese yen.
    pub const JPY: Self = Self(*b"JPY");

    /// Parse an ISO 4217 alphabetic code, normalising ASCII lowercase to uppercase.
    ///
    /// # Errors
    /// Returns [`BillingError::InvalidInput`] unless `code` is exactly three
    /// ASCII alphabetic characters.
    ///
    /// ```rust
    /// use billing::Currency;
    /// assert_eq!(Currency::new("chf").unwrap(), Currency::CHF);
    /// assert!(Currency::new("€").is_err());
    /// ```
    pub fn new(code: &str) -> Result<Self, BillingError> {
        let bytes = code.as_bytes();
        if bytes.len() != 3 || !bytes.iter().all(u8::is_ascii_alphabetic) {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "currency must be a 3-letter ISO 4217 alphabetic code, got {code:?}"
                ),
            });
        }
        Ok(Self([
            bytes[0].to_ascii_uppercase(),
            bytes[1].to_ascii_uppercase(),
            bytes[2].to_ascii_uppercase(),
        ]))
    }

    /// The three-letter code as a string slice.
    #[must_use]
    pub fn code(&self) -> &str {
        // SAFETY-free: the bytes are validated ASCII on every construction path,
        // and the only const constructors use ASCII literals.
        std::str::from_utf8(&self.0).unwrap_or("XXX")
    }

    /// Returns `true` for [`Currency::XXX`] — i.e. no currency was configured.
    #[must_use]
    pub fn is_unset(&self) -> bool {
        *self == Self::XXX
    }

    /// The ISO 4217 **minor unit** exponent: the number of decimal places the
    /// currency is conventionally written with.
    ///
    /// Returns `None` for the codes ISO lists as `N.A.` — the precious metals
    /// (`XAU`, `XAG`, `XPT`, `XPD`), the fund/bond-market units, `XTS`, and `XXX`
    /// itself. These have no fractional subdivision, which is **not** the same as
    /// having zero decimals, so the distinction is preserved in the type.
    ///
    /// Codes outside ISO's registry fall back to `Some(2)`, the overwhelmingly
    /// common case, since [`Currency::new`] accepts any well-formed code.
    ///
    /// # This is not the smallest transactable amount
    ///
    /// Minor units describe *notation*, not the smallest coin. `CHF` has two
    /// minor units but its smallest coin is 5 Rappen, so Swiss cash totals round
    /// to `0.05`. That is a jurisdictional payment rule, not a property of the
    /// currency — configure it explicitly with [`crate::CashRounding`].
    ///
    /// ```rust
    /// use billing::Currency;
    /// assert_eq!(Currency::EUR.minor_units(), Some(2));
    /// assert_eq!(Currency::JPY.minor_units(), Some(0));   // yen has no sen in practice
    /// assert_eq!(Currency::new("KWD").unwrap().minor_units(), Some(3));
    /// assert_eq!(Currency::new("CLF").unwrap().minor_units(), Some(4));
    /// assert_eq!(Currency::XXX.minor_units(), None);      // "no currency involved"
    /// ```
    #[must_use]
    pub fn minor_units(&self) -> Option<u8> {
        // ISO 4217 List One, published 2026-01-01 (Amendment 180).
        // Only the non-2 cases are enumerated; 2 is the default for everything else.
        const ZERO: [&[u8; 3]; 17] = [
            b"BIF", b"CLP", b"DJF", b"GNF", b"ISK", b"JPY", b"KMF", b"KRW", b"PYG", b"RWF", b"UGX",
            b"UYI", b"VND", b"VUV", b"XAF", b"XOF", b"XPF",
        ];
        const THREE: [&[u8; 3]; 7] = [b"BHD", b"IQD", b"JOD", b"KWD", b"LYD", b"OMR", b"TND"];
        const FOUR: [&[u8; 3]; 2] = [b"CLF", b"UYW"];
        // ISO lists these with minor unit "N.A." — no fractional subdivision.
        const NONE: [&[u8; 3]; 13] = [
            b"XAU", b"XAG", b"XPT", b"XPD", b"XDR", b"XUA", b"XSU", b"XBA", b"XBB", b"XBC", b"XBD",
            b"XTS", b"XXX",
        ];

        if NONE.iter().any(|c| **c == self.0) {
            None
        } else if ZERO.iter().any(|c| **c == self.0) {
            Some(0)
        } else if THREE.iter().any(|c| **c == self.0) {
            Some(3)
        } else if FOUR.iter().any(|c| **c == self.0) {
            Some(4)
        } else {
            Some(2)
        }
    }

    /// The smallest representable amount in this currency, as an [`crate::Amount<P>`].
    ///
    /// This is `10^-minor_units` — `0.01` for EUR, `1` for JPY, `0.001` for KWD.
    /// Use it as the default increment when rounding a document to payable
    /// precision.
    ///
    /// Returns `None` when the currency has no minor unit ([`Currency::minor_units`]
    /// returned `None`), or when the currency is finer-grained than `P` can
    /// represent (e.g. a 4-decimal currency at `P = 2`).
    ///
    /// ```rust
    /// use billing::{Amount, Currency};
    /// assert_eq!(Currency::EUR.minor_unit_increment::<5>(), Some(Amount::parse("0.01000").unwrap()));
    /// assert_eq!(Currency::JPY.minor_unit_increment::<5>(), Some(Amount::parse("1.00000").unwrap()));
    /// assert_eq!(Currency::XXX.minor_unit_increment::<5>(), None);
    /// // CLF has 4 decimals, which Amount<2> cannot express:
    /// assert_eq!(Currency::new("CLF").unwrap().minor_unit_increment::<2>(), None);
    /// ```
    #[must_use]
    pub fn minor_unit_increment<const P: u8>(&self) -> Option<crate::Amount<P>> {
        let units = self.minor_units()?;
        if units > P {
            return None;
        }
        // 10^(P - units) raw units == 10^-units in display terms.
        let mut raw: i64 = 1;
        for _ in 0..(P - units) {
            raw = raw.checked_mul(10)?;
        }
        Some(crate::Amount::from_raw_units(raw))
    }
}

impl Default for Currency {
    /// [`Currency::XXX`] — deliberately not a real currency. See the module docs.
    fn default() -> Self {
        Self::XXX
    }
}

impl fmt::Display for Currency {
    /// Honours width, fill and alignment (`{:>8}`), left-aligned by default like
    /// any other string-like value.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad(self.code())
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Currency({})", self.code())
    }
}

impl FromStr for Currency {
    type Err = BillingError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl TryFrom<&str> for Currency {
    type Error = BillingError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
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
    /// Validates on the way in — a malformed code is a deserialization error,
    /// never a silently-accepted value.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <std::borrow::Cow<'_, str> as serde::Deserialize>::deserialize(d)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_codes() {
        assert_eq!(Currency::new("EUR").unwrap(), Currency::EUR);
        assert_eq!(Currency::new("eur").unwrap(), Currency::EUR);
        assert_eq!(Currency::new("Usd").unwrap(), Currency::USD);
        assert_eq!(Currency::new("EUR").unwrap().code(), "EUR");
    }

    #[test]
    fn invalid_codes_rejected() {
        for bad in ["", "E", "EU", "EURO", "E1R", "€", "  E", "eu r"] {
            assert!(Currency::new(bad).is_err(), "{bad:?} must be rejected");
        }
    }

    #[test]
    fn default_is_xxx_not_eur() {
        assert_eq!(Currency::default(), Currency::XXX);
        assert!(Currency::default().is_unset());
        assert!(!Currency::EUR.is_unset());
    }

    #[test]
    fn display_and_parse_roundtrip() {
        let c: Currency = "gbp".parse().unwrap();
        assert_eq!(c.to_string(), "GBP");
    }

    #[test]
    fn every_conversion_entry_point_validates() {
        // `FromStr`, `TryFrom<&str>` and `new` are three doors into the same
        // check. A door that returns the default instead of the parsed code is
        // the worst possible failure here, because `Currency::default()` is XXX
        // — "no currency" — and a document labelled XXX still validates.
        assert_eq!(Currency::try_from("EUR").unwrap(), Currency::EUR);
        assert_eq!(Currency::try_from("usd").unwrap(), Currency::USD);
        assert_ne!(Currency::try_from("EUR").unwrap(), Currency::default());
        assert!(Currency::try_from("EURO").is_err());
        assert!(Currency::try_from("").is_err());

        assert_eq!("JPY".parse::<Currency>().unwrap(), Currency::JPY);
        assert!("JP".parse::<Currency>().is_err());
    }

    #[test]
    fn debug_names_the_type_and_the_code() {
        // `Currency(EUR)` rather than the derived `Currency { code: [69,85,82] }`
        // — the byte array is what a reader sees in a failing assertion message,
        // and it is unreadable.
        assert_eq!(format!("{:?}", Currency::EUR), "Currency(EUR)");
        assert_eq!(format!("{:?}", Currency::default()), "Currency(XXX)");
        assert_eq!(
            format!("{:?}", Currency::new("chf").unwrap()),
            "Currency(CHF)"
        );
    }

    #[test]
    fn minor_unit_increment_spans_the_scale_it_exactly_fits() {
        // The increment exists whenever the currency's minor units fit within
        // `P`, including when they use it up exactly. Requiring headroom would
        // silently drop cash rounding for every 2-decimal currency assembled at
        // `Amount<2>` — the natural scale for an EN 16931 document.
        assert_eq!(
            Currency::EUR.minor_unit_increment::<2>(),
            Some(crate::Amount::<2>::parse("0.01").unwrap())
        );
        assert_eq!(
            Currency::EUR.minor_unit_increment::<5>(),
            Some(crate::Amount::<5>::parse("0.01000").unwrap())
        );

        // JPY has zero minor units, so it fits every scale down to `Amount<0>`.
        assert_eq!(
            Currency::JPY.minor_unit_increment::<0>(),
            Some(crate::Amount::<0>::parse("1").unwrap())
        );

        // CLF has four; it fits at 4 and above, and not below.
        let clf = Currency::new("CLF").unwrap();
        assert_eq!(
            clf.minor_unit_increment::<4>(),
            Some(crate::Amount::<4>::parse("0.0001").unwrap())
        );
        assert_eq!(clf.minor_unit_increment::<3>(), None);

        // XXX has no minor units at all, at any scale.
        assert_eq!(Currency::XXX.minor_unit_increment::<5>(), None);
    }
}
