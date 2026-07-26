//! [`Tariff`] trait — the primary extension point for domain-specific billing logic.
//!
//! Three traits and one enum:
//!
//! | Item | Use it when |
//! |------|-------------|
//! | [`Tariff`] | pricing is driven by usage data (metered consumption, seats, calls) |
//! | [`ScalarTariff`] | the positions are already computed and there is no usage input |
//! | [`Billing`] | the outcome is "positions", "a document", **or** "nothing to bill, because …" |
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
/// `R` is the tariff's own reason type ([`Tariff::NotBillable`]), so domains keep
/// their own enum and match it exhaustively.
///
/// # Tariffs that always bill
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
    /// Use when composing a domain tariff into a wider one whose reason type is
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
    /// a total function — not a panicking `unwrap` — so a tariff that always bills
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

/// The positions-or-reason outcome returned by [`Tariff::line_items`].
pub type Positions<R> = Billing<Vec<LineItem>, R>;

/// The document-or-reason outcome returned by [`Tariff::try_bill`].
pub type Billed<R> = Billing<BillingDocument, R>;

// ── Tariff ────────────────────────────────────────────────────────────────────

/// Implement this trait to adapt any domain to the billing engine.
///
/// # Design
///
/// - `line_items` is a **pure function**: no I/O, no clock access, no mutation.
/// - Tax and discount layers declared here are applied by the document builder
///   in order. Tax ordering is significant (e.g. Stromsteuer before MwSt).
/// - The separation between pricing (`line_items`) and taxes (`tax_layers`)
///   mirrors real-world invoicing: net amount and tax calculation are
///   independently auditable.
///
/// # Choosing between `Tariff` and [`ScalarTariff`]
///
/// If pricing consumes usage data, implement `Tariff`. If the positions are
/// already computed and there is no usage input, implement [`ScalarTariff`]
/// instead — a blanket impl supplies `Tariff` for you, so you neither write
/// `type Usage = ()` nor take an argument you ignore.
///
/// # Example — SaaS platform
///
/// ```rust
/// use billing::{Tariff, Billing, Positions, LineItem, Amount, TaxLayer};
/// use billing::tax::FixedRateTax;
/// use rust_decimal::dec;
/// use std::convert::Infallible;
///
/// struct PlatformTariff { monthly_fee_eur: u32 }
/// struct Seats { count: u32 }
///
/// impl Tariff for PlatformTariff {
///     type Usage = Seats;
///     type Error = Infallible;
///     // This tariff always produces an invoice.
///     type NotBillable = Infallible;
///
///     fn line_items(&self, usage: &Seats) -> Result<Positions<Infallible>, Self::Error> {
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
/// use billing::{Tariff, Billing, Positions, LineItem, Amount};
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
/// impl Tariff for Settlement {
///     type Usage = Readings;
///     type Error = Infallible;
///     type NotBillable = NotYet;
///
///     fn line_items(&self, usage: &Readings) -> Result<Positions<NotYet>, Self::Error> {
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
/// let outcome = Settlement.line_items(&Readings { kwh: None }).unwrap();
/// assert_eq!(outcome.reason(), Some(&NotYet::NoMeterReading));
/// ```
pub trait Tariff {
    /// Domain-specific usage input.
    type Usage;
    /// Domain-specific error type — for things that actually went **wrong**.
    type Error: std::error::Error + Send + Sync + 'static;
    /// Domain-specific reason there is nothing to bill — for things that are
    /// merely **not billable**, which is a business state and not a failure.
    ///
    /// Use [`Infallible`] for a tariff that always produces positions; the
    /// [`Billing::NotBillable`] variant is then uninhabited and
    /// [`Billing::into_inner`] unwraps the outcome for free.
    type NotBillable: fmt::Debug + fmt::Display;

    /// Generate billing positions from usage data.  Must be a **pure function**.
    ///
    /// Return `Ok(items.into())` to bill, or
    /// `Ok(Billing::NotBillable(reason))` to record that there is nothing to bill
    /// and why. Reserve `Err` for genuine failures.
    fn line_items(&self, usage: &Self::Usage) -> Result<Positions<Self::NotBillable>, Self::Error>;

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
    /// Available only when `NotBillable = `[`Infallible`] — a tariff that can
    /// decline to bill has nowhere to put the reason in this signature, and must use
    /// [`Tariff::try_bill`] instead. The bound is what makes that a compile-time
    /// distinction rather than a silently-dropped reason.
    ///
    /// # Errors
    /// Propagates `line_items` failures and any document-assembly error.
    fn bill(&self, meta: DocumentMeta, usage: &Self::Usage) -> Result<BillingDocument, BillingError>
    where
        Self: Sized + Tariff<NotBillable = Infallible>,
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
    ///     .try_tariff(self, usage)?
    ///     .map(|b| b.build())
    /// ```
    ///
    /// # Errors
    /// Propagates `line_items` failures and any document-assembly error.
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
            .try_tariff(self, usage)?
        {
            Billing::Billable(b) => Ok(Billing::Billable(b.build()?)),
            Billing::NotBillable(r) => Ok(Billing::NotBillable(r)),
        }
    }
}

// ── ScalarTariff ──────────────────────────────────────────────────────────────

/// A tariff whose positions are **already computed** — no usage input.
///
/// # Why this exists
///
/// [`Tariff`] is shaped around usage-driven pricing, and plenty of settlements are
/// not: a subsidy payout, a redispatch compensation, a KWKG or EEG settlement whose
/// figures were determined upstream. Forced through `Tariff` those impls carried
/// `type Usage = ()` and a `fn line_items(&self, _usage: &())` parameter that every
/// implementation ignored — boilerplate that documented nothing and that callers
/// had to satisfy with `&()` at every call site.
///
/// Implement `ScalarTariff` instead. A blanket impl supplies
/// [`Tariff`]`<Usage = ()>`, so a scalar tariff still composes with
/// [`BillingDocumentBuilder`] and everything else that takes a `Tariff` — it just
/// never mentions `()`.
///
/// Implement **either** `ScalarTariff` or `Tariff` for a given type, never both;
/// the blanket impl makes that a coherence error.
///
/// ```rust
/// use billing::{ScalarTariff, Positions, LineItem, Amount, DocumentMeta, Currency};
/// use std::convert::Infallible;
///
/// struct EegSettlement { payout_eur: i64 }
///
/// impl ScalarTariff for EegSettlement {
///     type Error = Infallible;
///     type NotBillable = Infallible;
///
///     // No `usage` parameter, and no `type Usage = ()`.
///     fn positions(&self) -> Result<Positions<Infallible>, Self::Error> {
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
pub trait ScalarTariff {
    /// Domain-specific error type — for things that actually went wrong.
    type Error: std::error::Error + Send + Sync + 'static;
    /// Domain-specific reason there is nothing to bill. [`Infallible`] if the
    /// settlement always bills.
    type NotBillable: fmt::Debug + fmt::Display;

    /// The already-computed positions, or the reason there are none.
    ///
    /// Named `positions` rather than `line_items` so that it does not collide with
    /// [`Tariff::line_items`], which the blanket impl also brings into scope for
    /// every implementor.
    fn positions(&self) -> Result<Positions<Self::NotBillable>, Self::Error>;

    /// Tax / surcharge layers, in application order. See [`Tariff::tax_layers`].
    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        vec![]
    }

    /// Discount layers applied before tax. See [`Tariff::discount_layers`].
    fn discount_layers(&self) -> Vec<Box<dyn DiscountLayer>> {
        vec![]
    }

    /// Compute a [`BillingDocument`] — the scalar counterpart of [`Tariff::bill`].
    ///
    /// Available only when `NotBillable = `[`Infallible`]; otherwise use
    /// [`ScalarTariff::try_settle`].
    ///
    /// # Errors
    /// Propagates `positions` failures and any document-assembly error.
    fn settle(&self, meta: DocumentMeta) -> Result<BillingDocument, BillingError>
    where
        Self: Sized + ScalarTariff<NotBillable = Infallible>,
        Self::Error: Into<BillingError>,
    {
        Ok(self.try_settle(meta)?.into_inner())
    }

    /// Compute a [`BillingDocument`], or the reason there is nothing to bill.
    ///
    /// # Errors
    /// Propagates `positions` failures and any document-assembly error.
    fn try_settle(&self, meta: DocumentMeta) -> Result<Billed<Self::NotBillable>, BillingError>
    where
        Self: Sized,
        Self::Error: Into<BillingError>,
    {
        Tariff::try_bill(self, meta, &())
    }
}

/// Every [`ScalarTariff`] is a [`Tariff`] whose usage is `()`.
///
/// This is what lets a scalar settlement compose with [`BillingDocumentBuilder`]
/// and with generic code over `Tariff` without the implementor ever writing
/// `type Usage = ()`.
impl<T: ScalarTariff> Tariff for T {
    type Usage = ();
    type Error = <T as ScalarTariff>::Error;
    type NotBillable = <T as ScalarTariff>::NotBillable;

    fn line_items(&self, _usage: &()) -> Result<Positions<Self::NotBillable>, Self::Error> {
        self.positions()
    }

    fn tax_layers(&self) -> Vec<Box<dyn TaxLayer>> {
        ScalarTariff::tax_layers(self)
    }

    fn discount_layers(&self) -> Vec<Box<dyn DiscountLayer>> {
        ScalarTariff::discount_layers(self)
    }
}
