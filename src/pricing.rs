//! [`PricingModel`] — the primary extension point for domain-specific billing logic.
//!
//! One trait and one enum:
//!
//! | Item | Use it when |
//! |------|-------------|
//! | [`PricingModel`] | you are adapting a domain to the engine — usage in, priced positions and layers out |
//! | [`Billing`] | the outcome is "positions", "a document", **or** "nothing to bill, because …" |
//!
//! # Why "PricingModel" and not "Tariff"
//!
//! Until 0.12.0 this trait was `Tariff`, and the rate table was `TariffSchedule`.
//! *Tariff* is perfectly good English for a published schedule of rates — the sense
//! is as old as the customs-duty one, both from the 1590s, and UK primary legislation
//! is titled the *Domestic Gas and Electricity (Tariff Cap) Act 2018*.
//!
//! It was still the wrong name **here**. This crate advertises zero domain
//! assumptions, and every other public name keeps that promise:
//! [`crate::LineItem`], [`crate::Quantity`], [`crate::UnitPrice`],
//! [`crate::TaxLayer`], [`crate::UsageAggregator`], [`crate::AllocationRule`].
//! *Tariff* is the vocabulary of utilities and telecoms specifically — a SaaS or
//! marketplace developer says *plan*, *pricing model* or *rate card*, and in American
//! English the word's dominant sense is an import duty, which this crate has nothing
//! to do with. Naming the primary extension point after one domain cluster is exactly
//! the assumption the crate says it does not make.
//!
//! `PricingModel` is the phrase every one of those domains already uses for "how we
//! work out what to charge", and [`crate::RateSchedule`] / [`crate::RateBand`] join
//! [`crate::RateLookup`] in a neutral family of rate tables.
//!
//! # `PricingModel` is not [`crate::RateSchedule`]
//!
//! They sit at different levels, and the similar names invite the wrong guess.
//!
//! A [`crate::RateSchedule`] is a **rate table** — the banded prices themselves —
//! alongside [`crate::RateLookup`], [`crate::TimeOfUsePricing`] and
//! [`crate::DynamicPricing`]. A `PricingModel` is the **adapter** that decides which of
//! those to consult, in what order, with what quantities, and which tax and discount
//! layers apply. A schedule is a thing a model *uses*; it does not implement this
//! trait, and would have nothing to say about tax layers if it did.
use std::convert::Infallible;
use std::fmt;

use crate::document::{BillingDocument, BillingDocumentBuilder, DocumentMeta};
use crate::error::BillingError;
use crate::line_item::LineItem;
use crate::tax::{DiscountLayer, TaxLayer};

// ── Billing ───────────────────────────────────────────────────────────────────

/// The outcome of pricing: something billable, or a stated reason there is not.
///
/// # Why a third outcome
///
/// `Result<Vec<LineItem>, Error>` offers two answers, and real settlements have
/// three. A settlement can be **not billable yet for a specific, expected
/// reason** — no meter reading has arrived, the reference price for the period is
/// not published, the subsidy entitlement has ended — and that is neither a set of
/// positions nor a failure. Nothing went wrong; there is simply nothing to invoice.
///
/// Collapsed into `Ok(vec![])` the reason is gone, and "we billed nothing" becomes
/// indistinguishable from "there was nothing to bill, because X" — a distinction
/// every audit trail needs and no caller can reconstruct afterwards. Collapsed into
/// `Err` it pollutes the error path with an ordinary business state, so callers
/// cannot tell a missing price from a genuine arithmetic failure.
///
/// `R` is the model's own reason type ([`PricingModel::NotBillable`]), so domains keep
/// their own enum and match it exhaustively.
///
/// # Models that always bill
///
/// Set `type NotBillable = `[`Infallible`] and the [`NotBillable`](Billing::NotBillable)
/// variant becomes uninhabited — the compiler knows the outcome is always
/// [`Billable`](Billing::Billable), and [`Billing::into_inner`] unwraps it with no
/// `Result`, no `unwrap`, and no runtime check.
///
/// ```rust
/// use billing::{Billing, LineItem, Amount};
/// use std::convert::Infallible;
///
/// let items = vec![LineItem::fixed("Fee", Amount::<5>::from_int(10)).build().unwrap()];
/// // `Infallible` reason ⇒ statically known to be billable.
/// let outcome: Billing<Vec<LineItem>, Infallible> = items.into();
/// assert_eq!(outcome.into_inner().len(), 1);
/// ```
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Billing<T, R> {
    /// There is something to bill.
    Billable(T),
    /// There is nothing to bill, and this is why. Not an error.
    NotBillable(R),
}

impl<T, R> Billing<T, R> {
    /// Whether there is something to bill.
    #[must_use]
    pub fn is_billable(&self) -> bool {
        matches!(self, Self::Billable(_))
    }

    /// The billable value, if any.
    #[must_use]
    pub fn billable(&self) -> Option<&T> {
        match self {
            Self::Billable(t) => Some(t),
            Self::NotBillable(_) => None,
        }
    }

    /// The reason nothing is billable, if that is the outcome.
    #[must_use]
    pub fn reason(&self) -> Option<&R> {
        match self {
            Self::Billable(_) => None,
            Self::NotBillable(r) => Some(r),
        }
    }

    /// Take the billable value, discarding the reason.
    ///
    /// Named to make the discard visible: prefer matching, or
    /// [`billable_or_else`](Self::billable_or_else), when the reason must survive.
    #[must_use]
    pub fn into_billable(self) -> Option<T> {
        match self {
            Self::Billable(t) => Some(t),
            Self::NotBillable(_) => None,
        }
    }

    /// Transform the billable value, leaving a reason untouched.
    #[must_use]
    pub fn map<U, F: FnOnce(T) -> U>(self, f: F) -> Billing<U, R> {
        match self {
            Self::Billable(t) => Billing::Billable(f(t)),
            Self::NotBillable(r) => Billing::NotBillable(r),
        }
    }

    /// Transform the reason, leaving a billable value untouched.
    ///
    /// Use when composing a domain model into a wider one whose reason type is
    /// broader.
    #[must_use]
    pub fn map_reason<S, F: FnOnce(R) -> S>(self, f: F) -> Billing<T, S> {
        match self {
            Self::Billable(t) => Billing::Billable(t),
            Self::NotBillable(r) => Billing::NotBillable(f(r)),
        }
    }

    /// Convert a reason into an error of your choosing.
    ///
    /// For the callers — a scheduled job, say — that genuinely do want
    /// "not billable" to be terminal.
    pub fn billable_or_else<E, F: FnOnce(R) -> E>(self, f: F) -> Result<T, E> {
        match self {
            Self::Billable(t) => Ok(t),
            Self::NotBillable(r) => Err(f(r)),
        }
    }
}

impl<T> Billing<T, Infallible> {
    /// Unwrap a statically-always-billable outcome.
    ///
    /// Available only for `R = `[`Infallible`], where the
    /// [`NotBillable`](Billing::NotBillable) variant cannot be constructed. This is
    /// a total function — not a panicking `unwrap` — so a model that always bills
    /// pays nothing for the third outcome existing.
    #[must_use]
    pub fn into_inner(self) -> T {
        match self {
            Self::Billable(t) => t,
            // Uninhabited: `Infallible` has no values, so this arm is unreachable
            // by construction rather than by assertion.
            Self::NotBillable(never) => match never {},
        }
    }
}

/// `Ok(items.into())` — the ergonomic path for the always-billable case.
impl<R> From<Vec<LineItem>> for Billing<Vec<LineItem>, R> {
    fn from(items: Vec<LineItem>) -> Self {
        Self::Billable(items)
    }
}

impl<R> From<BillingDocument> for Billing<BillingDocument, R> {
    fn from(doc: BillingDocument) -> Self {
        Self::Billable(doc)
    }
}

impl<T, R: fmt::Display> fmt::Display for Billing<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Billable(_) => f.write_str("billable"),
            Self::NotBillable(r) => write!(f, "not billable: {r}"),
        }
    }
}

/// The positions-or-reason outcome returned by [`PricingModel::positions`].
pub type Positions<R> = Billing<Vec<LineItem>, R>;

/// The document-or-reason outcome returned by [`PricingModel::try_bill`].
pub type Billed<R> = Billing<BillingDocument, R>;

// ── PricingModel ────────────────────────────────────────────────────────────────────

/// Implement this trait to adapt any domain to the billing engine.
///
/// # Design
///
/// - `positions` is a **pure function**: no I/O, no clock access, no mutation.
/// - Tax and discount layers declared here are applied by the document builder
///   in order. Tax ordering is significant (e.g. Stromsteuer before MwSt).
/// - The separation between pricing (`positions`) and taxes (`tax_layers`)
///   mirrors real-world invoicing: net amount and tax calculation are
///   independently auditable.
///
/// # Settlements with no usage input
///
/// Plenty of settlements are not usage-driven: a subsidy payout, a redispatch
/// compensation, a KWKG or EEG settlement whose figures were determined upstream.
/// Set `type Usage = ()` and call [`settle`](PricingModel::settle), which is
/// [`bill`](PricingModel::bill) without the `&()` at the call site.
///
/// ```rust
/// use billing::{PricingModel, Positions, LineItem, Amount, DocumentMeta, Currency};
/// use std::convert::Infallible;
///
/// struct EegSettlement { payout_eur: i64 }
///
/// impl PricingModel for EegSettlement {
///     type Usage = ();
///     type Error = Infallible;
///     type NotBillable = Infallible;
///
///     fn positions(&self, _: &()) -> Result<Positions<Infallible>, Self::Error> {
///         Ok(vec![
///             LineItem::credit_fixed("EEG Vergütung", Amount::<5>::from_int(self.payout_eur))
///                 .build().unwrap(),
///         ].into())
///     }
/// }
///
/// let meta = DocumentMeta { currency: Currency::EUR, ..Default::default() };
/// let doc = EegSettlement { payout_eur: 400 }.settle(meta).unwrap();
/// assert_eq!(doc.net_total(), Amount::<5>::parse("-400.00000").unwrap());
/// ```
///
/// An earlier release had a second trait for this case, with its own method names
/// and a blanket impl. It removed one `type Usage = ();` line and one ignored
/// parameter, and cost a second vocabulary (`positions` vs `line_items`,
/// `settle` vs `bill`) plus a coherence hazard: implementing both traits for one
/// type was a compile error the documentation had to warn about. One trait is the
/// better trade.
///
/// # Example — SaaS platform
///
/// ```rust
/// use billing::{PricingModel, Billing, Positions, LineItem, Amount, TaxLayer};
/// use billing::tax::FixedRateTax;
/// use rust_decimal::dec;
/// use std::convert::Infallible;
///
/// struct PlatformPlan { monthly_fee_eur: u32 }
/// struct Seats { count: u32 }
///
/// impl PricingModel for PlatformPlan {
///     type Usage = Seats;
///     type Error = Infallible;
///     // This model always produces an invoice.
///     type NotBillable = Infallible;
///
///     fn positions(&self, usage: &Seats) -> Result<Positions<Infallible>, Self::Error> {
///         Ok(vec![
///             LineItem::fixed("Monthly platform fee",
///                 Amount::<5>::from_int(i64::from(self.monthly_fee_eur) * i64::from(usage.count))
///             ).build().unwrap(),
///         ].into())
///     }
///
///     fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
///         vec![FixedRateTax::new("VAT", dec!(0.20)).unwrap().boxed()]
///     }
/// }
/// ```
///
/// # Example — a settlement that is not always billable
///
/// ```rust
/// use billing::{PricingModel, Billing, Positions, LineItem, Amount};
/// use std::convert::Infallible;
/// use std::fmt;
///
/// #[derive(Debug, PartialEq)]
/// enum NotYet { NoMeterReading, PriceUnpublished }
///
/// impl fmt::Display for NotYet {
///     fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
///         match self {
///             Self::NoMeterReading    => f.write_str("no meter reading for the period"),
///             Self::PriceUnpublished  => f.write_str("reference price not yet published"),
///         }
///     }
/// }
///
/// struct Settlement;
/// struct Readings { kwh: Option<u32> }
///
/// impl PricingModel for Settlement {
///     type Usage = Readings;
///     type Error = Infallible;
///     type NotBillable = NotYet;
///
///     fn positions(&self, usage: &Readings) -> Result<Positions<NotYet>, Self::Error> {
///         let Some(kwh) = usage.kwh else {
///             // The reason survives — it is not flattened into an empty Vec.
///             return Ok(Billing::NotBillable(NotYet::NoMeterReading));
///         };
///         Ok(vec![
///             LineItem::fixed("Arbeit", Amount::<5>::from_int(i64::from(kwh))).build().unwrap(),
///         ].into())
///     }
/// }
///
/// let outcome = Settlement.positions(&Readings { kwh: None }).unwrap();
/// assert_eq!(outcome.reason(), Some(&NotYet::NoMeterReading));
/// ```
pub trait PricingModel {
    /// Domain-specific usage input.
    type Usage;
    /// Domain-specific error type — for things that actually went **wrong**.
    type Error: std::error::Error + Send + Sync + 'static;
    /// Domain-specific reason there is nothing to bill — for things that are
    /// merely **not billable**, which is a business state and not a failure.
    ///
    /// Use [`Infallible`] for a model that always produces positions; the
    /// [`Billing::NotBillable`] variant is then uninhabited and
    /// [`Billing::into_inner`] unwraps the outcome for free.
    type NotBillable: fmt::Debug + fmt::Display;

    /// Generate billing positions from usage data.  Must be a **pure function**.
    ///
    /// Return `Ok(items.into())` to bill, or
    /// `Ok(Billing::NotBillable(reason))` to record that there is nothing to bill
    /// and why. Reserve `Err` for genuine failures.
    fn positions(&self, usage: &Self::Usage) -> Result<Positions<Self::NotBillable>, Self::Error>;

    /// Tax / surcharge / percentage-charge layers applied after positions.
    ///
    /// Return an ordered `Vec` — sequence determines compound-tax bases
    /// (e.g. Stromsteuer BEFORE MwSt so Stromsteuer is in the MwSt base).
    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![]
    }

    /// Discount layers applied before tax (reduce the taxable base).
    fn discount_layers(&self) -> Vec<Box<dyn DiscountLayer>> {
        vec![]
    }

    /// Compute a [`BillingDocument`] from usage data.
    ///
    /// Available only when `NotBillable = `[`Infallible`] — a model that can
    /// decline to bill has nowhere to put the reason in this signature, and must use
    /// [`PricingModel::try_bill`] instead. The bound is what makes that a compile-time
    /// distinction rather than a silently-dropped reason.
    ///
    /// # Errors
    /// Propagates `positions` failures and any document-assembly error.
    fn bill(&self, meta: DocumentMeta, usage: &Self::Usage) -> Result<BillingDocument, BillingError>
    where
        Self: Sized + PricingModel<NotBillable = Infallible>,
        Self::Error: Into<BillingError>,
    {
        Ok(self.try_bill(meta, usage)?.into_inner())
    }

    /// Compute a [`BillingDocument`], or the reason there is nothing to bill.
    ///
    /// Equivalent to:
    /// ```rust,ignore
    /// BillingDocument::builder()
    ///     .meta(meta)
    ///     .try_pricing(self, usage)?
    ///     .map(|b| b.build())
    /// ```
    ///
    /// # Errors
    /// Propagates `positions` failures and any document-assembly error.
    fn try_bill(
        &self,
        meta: DocumentMeta,
        usage: &Self::Usage,
    ) -> Result<Billed<Self::NotBillable>, BillingError>
    where
        Self::Error: Into<BillingError>,
        Self: Sized,
    {
        match BillingDocumentBuilder::default()
            .meta(meta)
            .try_pricing(self, usage)?
        {
            Billing::Billable(b) => Ok(Billing::Billable(b.build()?)),
            Billing::NotBillable(r) => Ok(Billing::NotBillable(r)),
        }
    }

    /// [`bill`](PricingModel::bill) for a model that takes no usage — the settlement case.
    ///
    /// Bounded on `Usage = ()`, so it exists exactly where `&()` would otherwise
    /// have to be written at every call site and nowhere else.
    ///
    /// # Errors
    /// Propagates `positions` failures and any document-assembly error.
    fn settle(&self, meta: DocumentMeta) -> Result<BillingDocument, BillingError>
    where
        Self: Sized + PricingModel<Usage = (), NotBillable = Infallible>,
        Self::Error: Into<BillingError>,
    {
        self.bill(meta, &())
    }

    /// [`try_bill`](PricingModel::try_bill) for a model that takes no usage.
    ///
    /// # Errors
    /// Propagates `positions` failures and any document-assembly error.
    fn try_settle(&self, meta: DocumentMeta) -> Result<Billed<Self::NotBillable>, BillingError>
    where
        Self: Sized + PricingModel<Usage = ()>,
        Self::Error: Into<BillingError>,
    {
        self.try_bill(meta, &())
    }
}
