//! Usage aggregators — reduce a slice of events to a billing scalar.
//!
//! Provides 6 built-in aggregation types — `SUM`, `COUNT`, `UNIQUE_COUNT`,
//! `MAX`, `LATEST`, `WEIGHTED_SUM` — plus custom aggregators via the
//! [`UsageAggregator`] trait.
use rust_decimal::Decimal;

use crate::error::BillingError;

/// The overflow every accumulating aggregator reports instead of panicking.
fn overflow() -> BillingError {
    BillingError::MonetaryOverflow {
        precision: 5,
        input_value: None,
    }
}

// ── UsageAggregator trait ─────────────────────────────────────────────────────

/// Aggregate a slice of raw usage events to a single scalar quantity.
///
/// This is a pure function — no I/O, no state.  The result is passed to a
/// `RateSchedule` or `DynamicPricing` for billing.
///
/// 6 built-in types: `SUM`, `COUNT`, `UNIQUE_COUNT`, `MAX`, `LATEST`, `WEIGHTED_SUM`.
///
/// # Why this returns `Result`
///
/// This trait sits at the **least trusted boundary in the whole pipeline**: raw
/// usage events, straight from a meter, a log or an API. `Decimal`'s `+`, `*` and
/// `Sum` *panic* on overflow rather than saturating, so an accumulating aggregator
/// over a slice containing one corrupt magnitude aborts the process — and a batch
/// run dies on a single malformed record, which is the failure mode this crate has
/// spent three releases removing everywhere else.
///
/// Until 0.13.0 `aggregate` returned a bare `Decimal` and had nowhere to report it.
/// Every `checked_*` method in this crate is total for the same reason
/// ([`crate::Amount::checked_sum`] says so explicitly: *"prefer this over `.sum()`
/// in any code path that could receive attacker-controlled or unbounded values"*),
/// and usage events are that path.
///
/// Four of the six built-ins — `COUNT`, `UNIQUE_COUNT`, `MAX`, `LATEST` — cannot
/// fail: they select or count rather than accumulate. They still return `Result`,
/// because a caller writing generic code over `UsageAggregator` needs one
/// signature, and because an implementation that *starts* accumulating should not
/// become a breaking change for its callers.
pub trait UsageAggregator<E> {
    /// Aggregate a slice of events into a single [`Decimal`] quantity.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if the accumulation leaves `Decimal`'s
    /// representable range. Implementations that only select or count never
    /// return `Err`.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError>;
}

// ── SumAggregator ─────────────────────────────────────────────────────────────

/// Sum of a numeric field across all events.
///
/// Use case: total API calls, total kWh, total bytes transferred.
///
/// # Example
/// ```rust
/// use billing::aggregation::{SumAggregator, UsageAggregator};
/// use rust_decimal::Decimal;
///
/// struct ApiEvent { bytes: u64 }
/// let agg = SumAggregator::new(|e: &ApiEvent| Decimal::from(e.bytes));
/// let total = agg.aggregate(&[ApiEvent { bytes: 100 }, ApiEvent { bytes: 200 }])?;
/// assert_eq!(total, Decimal::from(300u32));
///
/// // A corrupt magnitude is reported, not a process abort.
/// let huge = |_: &ApiEvent| Decimal::MAX;
/// assert!(SumAggregator::new(huge)
///     .aggregate(&[ApiEvent { bytes: 1 }, ApiEvent { bytes: 1 }])
///     .is_err());
/// # Ok::<(), billing::BillingError>(())
/// ```
#[derive(Clone, Copy)]
pub struct SumAggregator<E, F: Fn(&E) -> Decimal> {
    value_fn: F,
    _marker: std::marker::PhantomData<E>,
}

impl<E, F: Fn(&E) -> Decimal> SumAggregator<E, F> {
    /// Create a `SumAggregator` from a value extractor closure.
    pub fn new(value_fn: F) -> Self {
        Self {
            value_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E, F: Fn(&E) -> Decimal> UsageAggregator<E> for SumAggregator<E, F> {
    /// `checked_add` throughout: `Decimal`'s `Sum` panics on overflow, which would
    /// take a batch run down on one corrupt event.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        events.iter().try_fold(Decimal::ZERO, |acc, e| {
            acc.checked_add((self.value_fn)(e)).ok_or_else(overflow)
        })
    }
}

// ── CountAggregator ───────────────────────────────────────────────────────────

/// Count of all events in the period.
///
/// Use case: number of transactions, number of API requests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CountAggregator;

impl<E> UsageAggregator<E> for CountAggregator {
    /// Infallible: `usize::MAX` is ~1.8 × 10¹⁹, far inside `Decimal`'s range.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        Ok(Decimal::from(events.len()))
    }
}

// ── UniqueCountAggregator ─────────────────────────────────────────────────────

/// Count of distinct key values across all events.
///
/// The key function can return any `Hash + Eq` type — `&str`, `u64`, an enum,
/// a struct, etc.  This avoids the allocation overhead of always converting to
/// `String`.
///
/// Use case: unique users/tenants with at least one request.
///
/// # Examples
///
/// ```rust
/// use billing::aggregation::{UniqueCountAggregator, UsageAggregator};
///
/// struct ApiCall<'a> { user_id: &'a str }
///
/// // Zero-allocation: key is a &str borrow, no String allocation.
/// let agg = UniqueCountAggregator::new(|e: &ApiCall<'_>| e.user_id);
/// let count = agg.aggregate(&[
///     ApiCall { user_id: "alice" },
///     ApiCall { user_id: "bob" },
///     ApiCall { user_id: "alice" }, // duplicate
/// ])?;
/// assert_eq!(count, rust_decimal::Decimal::from(2u32));
/// # Ok::<(), billing::BillingError>(())
/// ```
#[derive(Clone, Copy)]
pub struct UniqueCountAggregator<E, K, F>
where
    K: std::hash::Hash + Eq,
    F: Fn(&E) -> K,
{
    key_fn: F,
    _marker: std::marker::PhantomData<(E, K)>,
}

impl<E, K, F> UniqueCountAggregator<E, K, F>
where
    K: std::hash::Hash + Eq,
    F: Fn(&E) -> K,
{
    /// Create a `UniqueCountAggregator` from a key extractor closure.
    pub fn new(key_fn: F) -> Self {
        Self {
            key_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E, K, F> UsageAggregator<E> for UniqueCountAggregator<E, K, F>
where
    K: std::hash::Hash + Eq,
    F: Fn(&E) -> K,
{
    /// Infallible: a count of distinct keys is bounded by `events.len()`.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        let unique: std::collections::HashSet<K> =
            events.iter().map(|e| (self.key_fn)(e)).collect();
        Ok(Decimal::from(unique.len()))
    }
}

// ── MaxAggregator ─────────────────────────────────────────────────────────────

/// Maximum value of a numeric field across all events in the period.
///
/// Use case: peak active seats, peak bandwidth (Mbps).
/// Pairs well with `RateSchedule::capacity()`.
#[derive(Clone, Copy)]
pub struct MaxAggregator<E, F: Fn(&E) -> Decimal> {
    value_fn: F,
    _marker: std::marker::PhantomData<E>,
}

impl<E, F: Fn(&E) -> Decimal> MaxAggregator<E, F> {
    /// Create a `MaxAggregator` from a value extractor closure.
    pub fn new(value_fn: F) -> Self {
        Self {
            value_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E, F: Fn(&E) -> Decimal> UsageAggregator<E> for MaxAggregator<E, F> {
    /// Infallible: selecting an extremum computes nothing.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        Ok(events
            .iter()
            .map(|e| (self.value_fn)(e))
            .max_by(|a, b| a.cmp(b))
            .unwrap_or(Decimal::ZERO))
    }
}

// ── LatestAggregator ──────────────────────────────────────────────────────────

/// Most recent value of a numeric field (last event in slice order).
///
/// Use case: current storage GB at end of period (snapshot billing).
#[derive(Clone, Copy)]
pub struct LatestAggregator<E, F: Fn(&E) -> Decimal> {
    value_fn: F,
    _marker: std::marker::PhantomData<E>,
}

impl<E, F: Fn(&E) -> Decimal> LatestAggregator<E, F> {
    /// Create a `LatestAggregator` from a value extractor closure.
    pub fn new(value_fn: F) -> Self {
        Self {
            value_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E, F: Fn(&E) -> Decimal> UsageAggregator<E> for LatestAggregator<E, F> {
    /// Infallible: reading the last element computes nothing. An empty period is
    /// zero — there is no "current storage GB" to read — rather than an error.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        Ok(events
            .last()
            .map(|e| (self.value_fn)(e))
            .unwrap_or(Decimal::ZERO))
    }
}

// ── WeightedSumAggregator ─────────────────────────────────────────────────────

/// Time-weighted sum: `Σ(value_i × duration_fraction_i)`.
///
/// Use case: VM CPU-hours (VMs that started/stopped mid-period),
/// cloud resource GB-hours, license seats billed proportionally.
///
/// `duration_fn` should return a fraction between 0.0 and 1.0
/// (e.g. `uptime_seconds / total_period_seconds`).
/// # Panics
/// `Decimal`'s `Mul` and `Sum` panic on overflow — see [`SumAggregator`].
#[derive(Clone, Copy)]
pub struct WeightedSumAggregator<E, F: Fn(&E) -> Decimal, G: Fn(&E) -> Decimal> {
    /// Closure that extracts the numeric value from an event.
    pub value_fn: F,
    /// Closure that returns the fractional duration (0.0–1.0) the event was active.
    pub duration_fn: G,
    _marker: std::marker::PhantomData<E>,
}

impl<E, F: Fn(&E) -> Decimal, G: Fn(&E) -> Decimal> WeightedSumAggregator<E, F, G> {
    /// Create a `WeightedSumAggregator` from value and duration extractor closures.
    pub fn new(value_fn: F, duration_fn: G) -> Self {
        Self {
            value_fn,
            duration_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E, F: Fn(&E) -> Decimal, G: Fn(&E) -> Decimal> UsageAggregator<E>
    for WeightedSumAggregator<E, F, G>
{
    /// `checked_mul` **and** `checked_add`: this is the aggregator most likely to
    /// overflow, because it multiplies before it accumulates — a duration handed
    /// over in seconds rather than as a fraction is a factor of ~2.6 million.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        events.iter().try_fold(Decimal::ZERO, |acc, e| {
            let weighted = (self.value_fn)(e)
                .checked_mul((self.duration_fn)(e))
                .ok_or_else(overflow)?;
            acc.checked_add(weighted).ok_or_else(overflow)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::dec;

    struct Event {
        value: u64,
        user: String,
    }

    #[test]
    fn sum_agg() {
        let events = vec![
            Event {
                value: 100,
                user: "a".into(),
            },
            Event {
                value: 200,
                user: "b".into(),
            },
        ];
        let agg = SumAggregator::new(|e: &Event| Decimal::from(e.value));
        assert_eq!(agg.aggregate(&events).unwrap(), dec!(300));
    }

    #[test]
    fn count_agg() {
        let events = vec![
            Event {
                value: 1,
                user: "a".into(),
            },
            Event {
                value: 2,
                user: "b".into(),
            },
        ];
        assert_eq!(CountAggregator.aggregate(&events).unwrap(), dec!(2));
    }

    #[test]
    fn unique_count() {
        let events = vec![
            Event {
                value: 1,
                user: "alice".into(),
            },
            Event {
                value: 2,
                user: "bob".into(),
            },
            Event {
                value: 3,
                user: "alice".into(),
            }, // duplicate
        ];
        let agg = UniqueCountAggregator::new(|e: &Event| e.user.clone());
        assert_eq!(agg.aggregate(&events).unwrap(), dec!(2));
    }

    #[test]
    fn max_agg() {
        let events = vec![
            Event {
                value: 10,
                user: "a".into(),
            },
            Event {
                value: 50,
                user: "b".into(),
            },
            Event {
                value: 30,
                user: "c".into(),
            },
        ];
        let agg = MaxAggregator::new(|e: &Event| Decimal::from(e.value));
        assert_eq!(agg.aggregate(&events).unwrap(), dec!(50));
    }

    #[test]
    fn latest_agg() {
        // The only aggregator with no unit test until 0.12.0 — a README doctest was
        // the sole thing standing between `aggregate` and returning `Default`, which
        // a mutation sweep noticed the moment doctests left the sweep. "Latest" is
        // slice order, not the largest or the last-nonzero, so the fixture has to be
        // one where those three answers differ.
        let events = vec![
            Event {
                value: 90,
                user: "a".into(),
            },
            Event {
                value: 10,
                user: "b".into(),
            },
        ];
        let agg = LatestAggregator::new(|e: &Event| Decimal::from(e.value));
        assert_eq!(agg.aggregate(&events).unwrap(), dec!(10));

        // Snapshot billing over an empty period is zero, not a panic: there is no
        // "current storage GB" to read, and an error channel does not exist here.
        assert_eq!(agg.aggregate(&[]).unwrap(), Decimal::ZERO);
    }

    /// The trait sits at the least trusted boundary in the pipeline, so the two
    /// aggregators that *accumulate* must report an out-of-range total rather than
    /// abort the process. `Decimal`'s `Sum` and `Mul` both panic on overflow, so
    /// before 0.13.0 a single corrupt event took the whole batch run with it — and
    /// no test could observe that, because there was no error to observe.
    #[test]
    fn an_accumulating_aggregator_reports_overflow_instead_of_panicking() {
        struct Raw(Decimal);

        // Two events each carrying Decimal::MAX overflow on the first add.
        let sum = SumAggregator::new(|e: &Raw| e.0);
        assert!(
            sum.aggregate(&[Raw(Decimal::MAX), Raw(Decimal::MAX)])
                .is_err()
        );
        // …and the ordinary case still answers.
        assert_eq!(
            sum.aggregate(&[Raw(dec!(1.5)), Raw(dec!(2.5))]).unwrap(),
            dec!(4)
        );

        // WeightedSum overflows in the *multiplication*, which is the easier of the
        // two to reach: a duration passed in seconds rather than as a fraction is a
        // factor of ~2.6 million.
        let weighted = WeightedSumAggregator::new(|e: &Raw| e.0, |_: &Raw| Decimal::MAX);
        assert!(weighted.aggregate(&[Raw(Decimal::MAX)]).is_err());
        assert_eq!(
            WeightedSumAggregator::new(|e: &Raw| e.0, |_: &Raw| dec!(0.5))
                .aggregate(&[Raw(dec!(4))])
                .unwrap(),
            dec!(2)
        );

        // The four selecting aggregators cannot fail, even on the same input.
        assert!(
            MaxAggregator::new(|e: &Raw| e.0)
                .aggregate(&[Raw(Decimal::MAX), Raw(Decimal::MAX)])
                .is_ok()
        );
        assert!(
            LatestAggregator::new(|e: &Raw| e.0)
                .aggregate(&[Raw(Decimal::MAX)])
                .is_ok()
        );
        assert!(CountAggregator.aggregate(&[Raw(Decimal::MAX)]).is_ok());
    }

    #[test]
    fn weighted_sum() {
        // VM with 4 vCPUs, active for half the period (0.5)
        struct Vm {
            vcpus: u32,
            fraction: Decimal,
        }
        let vms = vec![
            Vm {
                vcpus: 4,
                fraction: dec!(0.5),
            },
            Vm {
                vcpus: 2,
                fraction: dec!(1.0),
            },
        ];
        let agg = WeightedSumAggregator::new(|v: &Vm| Decimal::from(v.vcpus), |v: &Vm| v.fraction);
        // 4 × 0.5 + 2 × 1.0 = 4.0 CPU-hours
        assert_eq!(agg.aggregate(&vms).unwrap(), dec!(4));
    }
}
