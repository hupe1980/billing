//! [`RateLookup`] — capacity-based rate table (EEG/KWKG style).
//!
//! Unlike a [`crate::TariffSchedule`] (which applies different prices to consumption
//! *bands*), a `RateLookup` returns a **single uniform rate** determined by a
//! *parameter value* (e.g. installed capacity in kWp). All consumption is then
//! multiplied by that one rate.
//!
//! # EEG example
//!
//! EEG §21 Vergütungssätze depend on plant capacity, not on kWh consumed:
//!
//! | Installed kWp | Rate (ct/kWh) |
//! |---------------|--------------|
//! | ≤ 10          | 8.11         |
//! | ≤ 40          | 6.79         |
//! | > 40          | 5.56         |
//!
//! ```rust
//! use billing::{RateLookup, Amount};
//! use rust_decimal::dec;
//!
//! let lookup = RateLookup::builder()
//!     .at_most(dec!(10),  Amount::parse("0.00811").unwrap())
//!     .at_most(dec!(40),  Amount::parse("0.00679").unwrap())
//!     .fallback(          Amount::parse("0.00556").unwrap())
//!     .build()
//!     .unwrap();
//!
//! assert_eq!(lookup.rate_for(dec!(8)).unwrap(),  Amount::parse("0.00811").unwrap());
//! assert_eq!(lookup.rate_for(dec!(10)).unwrap(), Amount::parse("0.00811").unwrap());
//! assert_eq!(lookup.rate_for(dec!(25)).unwrap(), Amount::parse("0.00679").unwrap());
//! assert_eq!(lookup.rate_for(dec!(999)).unwrap(),Amount::parse("0.00556").unwrap());
//! ```

use rust_decimal::Decimal;

use crate::amount::Amount;
use crate::error::BillingError;

// ── RateLookup ────────────────────────────────────────────────────────────────

/// An entry in a [`RateLookup`] table.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct RateLookupEntry {
    /// Inclusive upper bound for this entry. `Decimal::MAX` means "fallback".
    upper_bound: Decimal,
    /// Rate returned when `parameter <= upper_bound`.
    rate: Amount<5>,
}

/// A capacity-based rate table: returns the first rate whose `upper_bound ≥ parameter`.
///
/// Entries are stored sorted ascending by `upper_bound`. The last entry (added via
/// [`RateLookupBuilder::fallback`]) acts as a catch-all.
///
/// # See also
/// [`RateLookupBuilder`] — construct via [`RateLookup::builder`].
///
/// # Validation on deserialisation
///
/// `RateLookup` deserialises through [`RateLookupBuilder::build`], so a table
/// loaded from config is sorted and validated identically to one built in code.
/// Without this, unsorted deserialised entries would make `rate_for` return the
/// wrong rate — a silent mispricing.
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "RateLookupRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLookup {
    entries: Vec<RateLookupEntry>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct RateLookupRepr {
    entries: Vec<RateLookupEntry>,
}

#[cfg(feature = "serde")]
impl TryFrom<RateLookupRepr> for RateLookup {
    type Error = BillingError;
    fn try_from(r: RateLookupRepr) -> Result<Self, Self::Error> {
        RateLookupBuilder {
            entries: r.entries,
            has_fallback: false,
            duplicate_fallback: false,
        }
        .build()
    }
}

impl RateLookup {
    /// Start building a `RateLookup`.
    #[must_use]
    pub fn builder() -> RateLookupBuilder {
        RateLookupBuilder::default()
    }

    /// Return the rate for the given `parameter` value.
    ///
    /// Finds the first entry with `upper_bound >= parameter` (entries are sorted
    /// ascending). Returns `Err` if no entry matches (i.e. no fallback was added
    /// and `parameter` exceeds all `upper_bound` values).
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] when no matching entry is found.
    pub fn rate_for(&self, parameter: Decimal) -> Result<Amount<5>, BillingError> {
        self.entries
            .iter()
            .find(|e| parameter <= e.upper_bound)
            .map(|e| e.rate)
            .ok_or(BillingError::InvalidInput {
                reason: "no matching rate for parameter: add a .fallback() entry".into(),
            })
    }
}

// ── RateLookupBuilder ─────────────────────────────────────────────────────────

/// Builder for [`RateLookup`].
///
/// Add entries with [`at_most`](RateLookupBuilder::at_most) in ascending
/// `upper_bound` order, then finish with
/// [`fallback`](RateLookupBuilder::fallback) (optional but recommended).
#[derive(Default)]
pub struct RateLookupBuilder {
    entries: Vec<RateLookupEntry>,
    has_fallback: bool,
    duplicate_fallback: bool,
}

impl RateLookupBuilder {
    /// Add an entry: applies when `parameter <= upper_bound`.
    ///
    /// Entries should be added in **ascending** `upper_bound` order for
    /// clarity, but [`build`](RateLookupBuilder::build) sorts them automatically.
    ///
    /// Invalid bounds are reported by [`build`](RateLookupBuilder::build) as an
    /// `Err`, not by panicking here — a rate table is frequently assembled from
    /// external configuration, where a bad value must be a recoverable error.
    #[must_use]
    pub fn at_most(mut self, upper_bound: Decimal, rate: Amount<5>) -> Self {
        self.entries.push(RateLookupEntry { upper_bound, rate });
        self
    }

    /// Add a catch-all entry that matches any parameter not covered by
    /// previous `at_most` entries. Equivalent to `at_most(Decimal::MAX, rate)`.
    ///
    /// Only one fallback is allowed; a second call is reported by
    /// [`build`](RateLookupBuilder::build) as an `Err`.
    #[must_use]
    pub fn fallback(mut self, rate: Amount<5>) -> Self {
        if self.has_fallback {
            // Mark the duplicate with a sentinel the build() check detects.
            self.duplicate_fallback = true;
        }
        self.has_fallback = true;
        self.entries.push(RateLookupEntry {
            upper_bound: Decimal::MAX,
            rate,
        });
        self
    }

    /// Build the [`RateLookup`].
    ///
    /// Sorts entries by `upper_bound` ascending so `rate_for` always finds
    /// the most specific (lowest) matching bound.
    ///
    /// # Errors
    /// [`BillingError::InvalidSchedule`] if:
    /// - no entries have been added,
    /// - any `upper_bound` is non-positive,
    /// - two entries share an `upper_bound` (the later one is unreachable), or
    /// - [`fallback`](RateLookupBuilder::fallback) was called more than once.
    pub fn build(mut self) -> Result<RateLookup, BillingError> {
        if self.entries.is_empty() {
            return Err(BillingError::InvalidSchedule {
                reason: "RateLookup must have at least one entry".into(),
            });
        }
        if self.duplicate_fallback {
            return Err(BillingError::InvalidSchedule {
                reason: "RateLookup: fallback() may only be called once".into(),
            });
        }
        for e in &self.entries {
            if e.upper_bound <= Decimal::ZERO {
                return Err(BillingError::InvalidSchedule {
                    reason: format!(
                        "RateLookup upper_bound must be positive, got {}",
                        e.upper_bound
                    ),
                });
            }
        }
        // Sort ascending so rate_for finds the lowest matching upper_bound first.
        // `Decimal: Ord`, so a total ordering is available — the previous
        // `partial_cmp(..).unwrap_or(Equal)` silently treated incomparable pairs
        // as equal, which for a total order was dead code hiding intent.
        // `sort_by_key` is valid here because `Decimal` is `Copy`.
        self.entries.sort_by_key(|e| e.upper_bound);
        // A duplicate bound makes the second entry unreachable — almost always a
        // config error, and silently ignoring it would misprice that band.
        if let Some(w) = self
            .entries
            .windows(2)
            .find(|w| w[0].upper_bound == w[1].upper_bound)
        {
            return Err(BillingError::InvalidSchedule {
                reason: format!(
                    "RateLookup has duplicate upper_bound {} — the second entry is unreachable",
                    w[0].upper_bound
                ),
            });
        }
        Ok(RateLookup {
            entries: self.entries,
        })
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    fn eeg_lookup() -> RateLookup {
        RateLookup::builder()
            .at_most(dec!(10), Amount::parse("0.00811").unwrap())
            .at_most(dec!(40), Amount::parse("0.00679").unwrap())
            .fallback(Amount::parse("0.00556").unwrap())
            .build()
            .unwrap()
    }

    #[test]
    fn rate_at_exact_boundary() {
        let l = eeg_lookup();
        assert_eq!(
            l.rate_for(dec!(10)).unwrap(),
            Amount::parse("0.00811").unwrap()
        );
        assert_eq!(
            l.rate_for(dec!(40)).unwrap(),
            Amount::parse("0.00679").unwrap()
        );
    }

    #[test]
    fn rate_within_band() {
        let l = eeg_lookup();
        assert_eq!(
            l.rate_for(dec!(8)).unwrap(),
            Amount::parse("0.00811").unwrap()
        );
        assert_eq!(
            l.rate_for(dec!(25)).unwrap(),
            Amount::parse("0.00679").unwrap()
        );
        assert_eq!(
            l.rate_for(dec!(100)).unwrap(),
            Amount::parse("0.00556").unwrap()
        );
    }

    #[test]
    fn rate_fallback_matches_large_value() {
        let l = eeg_lookup();
        assert_eq!(
            l.rate_for(dec!(9999)).unwrap(),
            Amount::parse("0.00556").unwrap()
        );
    }

    #[test]
    fn rate_no_fallback_returns_err_on_overshoot() {
        let l = RateLookup::builder()
            .at_most(dec!(10), Amount::parse("0.00811").unwrap())
            .build()
            .unwrap();
        assert!(l.rate_for(dec!(11)).is_err());
    }

    #[test]
    fn build_empty_returns_err() {
        assert!(RateLookup::builder().build().is_err());
    }

    #[test]
    fn entries_inserted_out_of_order_still_work() {
        // Insert bands in reverse order — build() should sort them.
        let l = RateLookup::builder()
            .at_most(dec!(40), Amount::parse("0.00679").unwrap())
            .at_most(dec!(10), Amount::parse("0.00811").unwrap())
            .fallback(Amount::parse("0.00556").unwrap())
            .build()
            .unwrap();
        assert_eq!(
            l.rate_for(dec!(5)).unwrap(),
            Amount::parse("0.00811").unwrap()
        );
        assert_eq!(
            l.rate_for(dec!(20)).unwrap(),
            Amount::parse("0.00679").unwrap()
        );
        assert_eq!(
            l.rate_for(dec!(100)).unwrap(),
            Amount::parse("0.00556").unwrap()
        );
    }
}
