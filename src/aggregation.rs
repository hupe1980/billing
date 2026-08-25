//! Usage aggregators — reduce a slice of events to a billing scalar.
//!
//! Provides 8 built-in aggregation types — `SUM`, `COUNT`, `UNIQUE_COUNT`,
//! `MAX`, `MEAN`, `PERCENTILE`, `LATEST`, `WEIGHTED_SUM` — plus custom
//! aggregators via the [`UsageAggregator`] trait.
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
/// 8 built-in types: `SUM`, `COUNT`, `UNIQUE_COUNT`, `MAX`, `MEAN`, `PERCENTILE`,
/// `LATEST`, `WEIGHTED_SUM`.
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
/// A bare `Decimal` return has nowhere to report that. Every `checked_*` method in
/// this crate is total for the same reason ([`crate::Amount::checked_sum`]: *"prefer
/// this over `.sum()` in any code path that could receive attacker-controlled or
/// unbounded values"*), and usage events are that path.
///
/// Five of the eight built-ins — `COUNT`, `UNIQUE_COUNT`, `MAX`, `PERCENTILE` and
/// `LATEST` — cannot fail: they select or count rather than accumulate. They still
/// return `Result`,
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
            .max()
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

// ── MeanAggregator ────────────────────────────────────────────────────────────

/// Arithmetic mean of a numeric field across all events.
///
/// Use case: average concurrent seats over the month, average daily storage,
/// average connected devices — the "average N over the period" billing model,
/// where the customer is charged for what they held on average rather than for
/// their peak ([`MaxAggregator`]) or their total ([`SumAggregator`]).
///
/// # It is not a time-weighted average
///
/// Every sample counts once, so this is only the right answer when the samples
/// are **evenly spaced**: hourly meter reads, daily snapshots, five-minute
/// polls. When each observation covers a different slice of the period — a VM
/// that ran for three hours beside one that ran for a month — the samples are
/// not interchangeable and [`WeightedSumAggregator`] is the aggregator that
/// says so.
///
/// # An empty period is zero
///
/// The mean of nothing is undefined, and this returns [`Decimal::ZERO`] rather
/// than an error, matching [`MaxAggregator`] and [`LatestAggregator`]: a period
/// with no events is a period with nothing to bill, not a failure. Guard on
/// `events.is_empty()` first if the distinction matters to you.
///
/// ```rust
/// use billing::aggregation::{MeanAggregator, UsageAggregator};
/// use rust_decimal::{Decimal, dec};
///
/// struct Sample { seats: u32 }
/// let agg = MeanAggregator::new(|s: &Sample| Decimal::from(s.seats));
///
/// // 10, 20, 30 seats observed on three days → 20 on average.
/// let mean = agg.aggregate(&[Sample { seats: 10 }, Sample { seats: 20 }, Sample { seats: 30 }])?;
/// assert_eq!(mean, dec!(20));
///
/// // No samples at all is zero, not an error.
/// assert_eq!(agg.aggregate(&[])?, Decimal::ZERO);
/// # Ok::<(), billing::BillingError>(())
/// ```
#[derive(Clone, Copy)]
pub struct MeanAggregator<E, F: Fn(&E) -> Decimal> {
    value_fn: F,
    _marker: std::marker::PhantomData<E>,
}

impl<E, F: Fn(&E) -> Decimal> MeanAggregator<E, F> {
    /// Create a `MeanAggregator` from a value extractor closure.
    pub fn new(value_fn: F) -> Self {
        Self {
            value_fn,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<E, F: Fn(&E) -> Decimal> UsageAggregator<E> for MeanAggregator<E, F> {
    /// `checked_add` and `checked_div`: `Decimal`'s `Sum` and `Div` both panic
    /// rather than saturating, and this accumulates before it divides.
    ///
    /// The quotient is **not** rounded here. It is a quantity, not money, and the
    /// single rounding belongs where it is multiplied by a price — see
    /// [`crate::Amount::checked_mul_qty`].
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        if events.is_empty() {
            return Ok(Decimal::ZERO);
        }
        let sum = events.iter().try_fold(Decimal::ZERO, |acc, e| {
            acc.checked_add((self.value_fn)(e)).ok_or_else(overflow)
        })?;
        sum.checked_div(Decimal::from(events.len()))
            .ok_or_else(overflow)
    }
}

// ── PercentileAggregator ──────────────────────────────────────────────────────

/// The `p`-th percentile of a numeric field — **burstable** (95th-percentile)
/// billing.
///
/// This is the standard settlement model for IP transit, CDN egress and DDoS
/// scrubbing, and it cannot be expressed with any of the other aggregators.
/// The provider samples throughput every five minutes, discards the top 5 % of
/// the samples, and bills the highest one that is left — so a customer may burst
/// above their committed rate for roughly 36 hours in a 30-day month without
/// paying for the peak. Billing [`MaxAggregator`] instead charges for the single
/// worst five-minute window in the month; billing [`MeanAggregator`] charges for
/// a rate the customer never actually held.
///
/// # Nearest-rank, deliberately
///
/// The samples are sorted ascending and the value at rank
/// `ceil(p ÷ 100 × n)` is returned — the *nearest-rank* method, clamped so that
/// `p = 0` still selects the smallest sample rather than nothing.
///
/// That is exactly the industry construction. With the conventional 8 640
/// five-minute samples in a 30-day month, "discard the top 5 %" removes 432 of
/// them and the next highest is sample 8 208 — which is `ceil(0.95 × 8640)`.
///
/// Interpolating between the two neighbouring samples (the other common
/// percentile definition, and the one most statistics libraries default to)
/// would invent a throughput figure that **never occurred on the link**, and bill
/// for it. Every sample this returns is a rate the customer really reached.
///
/// # An empty period is zero
///
/// As for [`MaxAggregator`] and [`MeanAggregator`]: nothing measured is nothing
/// to bill.
///
/// ```rust
/// use billing::aggregation::{PercentileAggregator, UsageAggregator};
/// use rust_decimal::{Decimal, dec};
///
/// struct Sample { mbps: u32 }
/// let value = |s: &Sample| Decimal::from(s.mbps);
///
/// // Twenty five-minute samples: nineteen at 100 Mbps and one 900 Mbps burst.
/// let mut samples: Vec<Sample> = (0..19).map(|_| Sample { mbps: 100 }).collect();
/// samples.push(Sample { mbps: 900 });
///
/// // P95 discards the single highest sample — the burst is free.
/// assert_eq!(PercentileAggregator::p95(value).aggregate(&samples)?, dec!(100));
/// // The peak would have billed the burst instead.
/// assert_eq!(PercentileAggregator::new(dec!(100), value)?.aggregate(&samples)?, dec!(900));
///
/// // Nothing measured is nothing to bill.
/// assert_eq!(PercentileAggregator::p95(value).aggregate(&[])?, Decimal::ZERO);
/// # Ok::<(), billing::BillingError>(())
/// ```
#[derive(Clone, Copy)]
pub struct PercentileAggregator<E, F: Fn(&E) -> Decimal> {
    percentile: Decimal,
    value_fn: F,
    _marker: std::marker::PhantomData<E>,
}

impl<E, F: Fn(&E) -> Decimal> PercentileAggregator<E, F> {
    /// The 95th percentile — burstable billing as every transit provider settles it.
    ///
    /// Infallible, because `95` is in range by construction. Use
    /// [`new`](Self::new) for any other percentile.
    pub fn p95(value_fn: F) -> Self {
        Self {
            percentile: Decimal::from(95u8),
            value_fn,
            _marker: std::marker::PhantomData,
        }
    }

    /// A percentile of your own, stated in **percent**: `95`, not `0.95`.
    ///
    /// `100` is the maximum and `50` the median (the lower of the two middle
    /// samples for an even count, since nearest-rank never interpolates).
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] if `percentile` is outside `[0, 100]`. Note
    /// that `0.95` is *not* rejected — it is a lawful, if unusual, request for very
    /// nearly the smallest sample — so passing a fraction by mistake produces a
    /// suspiciously small bill rather than an error. State it in percent.
    pub fn new(percentile: Decimal, value_fn: F) -> Result<Self, BillingError> {
        if percentile < Decimal::ZERO || percentile > Decimal::ONE_HUNDRED {
            return Err(BillingError::InvalidInput {
                reason: format!(
                    "percentile must be in [0, 100] and stated in percent (95, not 0.95), \
                     got {percentile}"
                ),
            });
        }
        Ok(Self {
            percentile,
            value_fn,
            _marker: std::marker::PhantomData,
        })
    }

    /// The configured percentile, in percent.
    #[must_use]
    pub fn percentile(&self) -> Decimal {
        self.percentile
    }
}

impl<E, F: Fn(&E) -> Decimal> UsageAggregator<E> for PercentileAggregator<E, F> {
    /// Infallible: sorting and indexing compute nothing that can leave range.
    ///
    /// The rank arithmetic is done on `usize` after scaling by the percentile,
    /// which is bounded by `100`, so it cannot overflow for any slice that fits in
    /// memory.
    fn aggregate(&self, events: &[E]) -> Result<Decimal, BillingError> {
        if events.is_empty() {
            return Ok(Decimal::ZERO);
        }
        let mut values: Vec<Decimal> = events.iter().map(|e| (self.value_fn)(e)).collect();
        values.sort_unstable();
        let n = values.len();
        // rank = ceil(p/100 × n), clamped to [1, n]. `Decimal` division is exact
        // enough here that `ceil` is the whole of the nearest-rank rule; the
        // fallback to `n` covers only a `Decimal` overflow that `new` forbids.
        let rank = self
            .percentile
            .checked_div(Decimal::ONE_HUNDRED)
            .and_then(|f| f.checked_mul(Decimal::from(n)))
            .map(|r| r.ceil())
            .and_then(|r| usize::try_from(r).ok())
            .unwrap_or(n)
            .clamp(1, n);
        Ok(values[rank - 1])
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
///
/// # Never panics
///
/// It multiplies before it accumulates, which makes it the aggregator most likely
/// to leave `Decimal`'s range — a duration handed over in seconds rather than as a
/// fraction is a factor of ~2.6 million. Both steps are checked, so an out-of-range
/// event is reported as [`BillingError::MonetaryOverflow`] rather than aborting the
/// batch; see [`UsageAggregator`] for why that is the contract.
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
        // "Latest" is slice order — not the largest, and not the last non-zero — so
        // the fixture has to be one where those three answers differ. A doctest alone
        // does not distinguish them from `Default`.
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
    /// abort the process. `Decimal`'s `Sum` and `Mul` both panic on overflow, so a
    /// single corrupt event would otherwise take the whole batch run with it.
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

    /// The industry construction, spelled out: 8 640 five-minute samples in a
    /// 30-day month, discard the top 5 % (432 of them), bill the next highest.
    /// `ceil(0.95 × 8640) = 8208`, and sample 8 208 of 8 640 sorted ascending is
    /// exactly the one left standing after 432 are removed from the top.
    #[test]
    fn p95_matches_discarding_the_top_five_percent() {
        struct S(u32);
        let samples: Vec<S> = (1..=8_640u32).map(S).collect();
        let agg = PercentileAggregator::p95(|s: &S| Decimal::from(s.0));
        assert_eq!(agg.aggregate(&samples).unwrap(), dec!(8208));

        // Same answer the long way round: sort, drop 432, take the max.
        let mut values: Vec<u32> = samples.iter().map(|s| s.0).collect();
        values.sort_unstable();
        values.truncate(8_640 - 432);
        assert_eq!(Decimal::from(*values.last().unwrap()), dec!(8208));
    }

    #[test]
    fn percentile_is_nearest_rank_and_never_interpolates() {
        struct S(u32);
        let v = |s: &S| Decimal::from(s.0);
        let samples = [S(10), S(20), S(30), S(40)];

        // Every answer is a sample that actually occurred — never 25.
        assert_eq!(
            PercentileAggregator::new(dec!(50), v)
                .unwrap()
                .aggregate(&samples)
                .unwrap(),
            dec!(20)
        );
        assert_eq!(
            PercentileAggregator::new(dec!(100), v)
                .unwrap()
                .aggregate(&samples)
                .unwrap(),
            dec!(40)
        );
        // `p = 0` clamps to the first rank rather than selecting nothing.
        assert_eq!(
            PercentileAggregator::new(dec!(0), v)
                .unwrap()
                .aggregate(&samples)
                .unwrap(),
            dec!(10)
        );
    }

    #[test]
    fn percentile_is_order_independent_and_empty_is_zero() {
        struct S(u32);
        let v = |s: &S| Decimal::from(s.0);
        let ascending = [S(1), S(2), S(3), S(100)];
        let shuffled = [S(100), S(2), S(1), S(3)];
        let agg = PercentileAggregator::p95(v);
        assert_eq!(
            agg.aggregate(&ascending).unwrap(),
            agg.aggregate(&shuffled).unwrap()
        );
        assert_eq!(agg.aggregate(&[]).unwrap(), Decimal::ZERO);
    }

    #[test]
    fn the_percentile_accessor_reports_what_was_configured() {
        struct S(u32);
        let v = |s: &S| Decimal::from(s.0);
        assert_eq!(PercentileAggregator::p95(v).percentile(), dec!(95));
        assert_eq!(
            PercentileAggregator::new(dec!(99.9), v)
                .unwrap()
                .percentile(),
            dec!(99.9)
        );
    }

    #[test]
    fn percentile_rejects_a_value_outside_zero_to_one_hundred() {
        struct S(u32);
        let v = |s: &S| Decimal::from(s.0);
        assert!(PercentileAggregator::new(dec!(101), v).is_err());
        assert!(PercentileAggregator::new(dec!(-1), v).is_err());
        // A fraction is in range and therefore accepted — see the doc comment.
        assert!(PercentileAggregator::new(dec!(0.95), v).is_ok());
    }

    #[test]
    fn mean_averages_and_reports_overflow() {
        struct S(Decimal);
        let agg = MeanAggregator::new(|s: &S| s.0);
        assert_eq!(
            agg.aggregate(&[S(dec!(10)), S(dec!(20)), S(dec!(30))])
                .unwrap(),
            dec!(20)
        );
        // A non-terminating mean keeps `Decimal`'s precision; the single rounding
        // happens where it is multiplied by a price.
        assert_eq!(
            agg.aggregate(&[S(dec!(1)), S(dec!(1)), S(dec!(2))])
                .unwrap(),
            dec!(1.3333333333333333333333333333)
        );
        assert_eq!(agg.aggregate(&[]).unwrap(), Decimal::ZERO);
        assert!(agg.aggregate(&[S(Decimal::MAX), S(Decimal::MAX)]).is_err());
    }
}
