//! Usage aggregators — reduce a slice of events to a billing scalar.
//!
//! Provides 6 built-in aggregation types — `SUM`, `COUNT`, `UNIQUE_COUNT`,
//! `MAX`, `LATEST`, `WEIGHTED_SUM` — plus custom aggregators via the
//! [`UsageAggregator`] trait.
use rust_decimal::Decimal;

// ── UsageAggregator trait ─────────────────────────────────────────────────────

/// Aggregate a slice of raw usage events to a single scalar quantity.
///
/// This is a pure function — no I/O, no state.  The result is passed to a
/// `TariffSchedule` or `DynamicPricing` for billing.
///
/// 6 built-in types: `SUM`, `COUNT`, `UNIQUE_COUNT`, `MAX`, `LATEST`, `WEIGHTED_SUM`.
pub trait UsageAggregator<E> {
    /// Aggregate a slice of events into a single [`Decimal`] quantity.
    fn aggregate(&self, events: &[E]) -> Decimal;
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
/// let total = agg.aggregate(&[ApiEvent { bytes: 100 }, ApiEvent { bytes: 200 }]);
/// assert_eq!(total, Decimal::from(300u32));
/// ```
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
    fn aggregate(&self, events: &[E]) -> Decimal {
        events.iter().map(|e| (self.value_fn)(e)).sum()
    }
}

// ── CountAggregator ───────────────────────────────────────────────────────────

/// Count of all events in the period.
///
/// Use case: number of transactions, number of API requests.
pub struct CountAggregator;

impl<E> UsageAggregator<E> for CountAggregator {
    fn aggregate(&self, events: &[E]) -> Decimal {
        Decimal::from(events.len())
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
/// ]);
/// assert_eq!(count, rust_decimal::Decimal::from(2u32));
/// ```
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
    fn aggregate(&self, events: &[E]) -> Decimal {
        let unique: std::collections::HashSet<K> =
            events.iter().map(|e| (self.key_fn)(e)).collect();
        Decimal::from(unique.len())
    }
}

// ── MaxAggregator ─────────────────────────────────────────────────────────────

/// Maximum value of a numeric field across all events in the period.
///
/// Use case: peak active seats, peak bandwidth (Mbps).
/// Pairs well with `TariffSchedule::capacity()`.
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
    fn aggregate(&self, events: &[E]) -> Decimal {
        events
            .iter()
            .map(|e| (self.value_fn)(e))
            .max_by(|a, b| a.cmp(b))
            .unwrap_or(Decimal::ZERO)
    }
}

// ── LatestAggregator ──────────────────────────────────────────────────────────

/// Most recent value of a numeric field (last event in slice order).
///
/// Use case: current storage GB at end of period (snapshot billing).
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
    fn aggregate(&self, events: &[E]) -> Decimal {
        events
            .last()
            .map(|e| (self.value_fn)(e))
            .unwrap_or(Decimal::ZERO)
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
    fn aggregate(&self, events: &[E]) -> Decimal {
        events
            .iter()
            .map(|e| (self.value_fn)(e) * (self.duration_fn)(e))
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal_macros::dec;

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
        assert_eq!(agg.aggregate(&events), dec!(300));
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
        assert_eq!(CountAggregator.aggregate(&events), dec!(2));
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
        assert_eq!(agg.aggregate(&events), dec!(2));
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
        assert_eq!(agg.aggregate(&events), dec!(50));
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
        assert_eq!(agg.aggregate(&vms), dec!(4));
    }
}
