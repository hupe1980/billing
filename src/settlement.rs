//! Settlement inputs: [`Payment`]s (BT-113), [`CashRounding`] (BT-114),
//! [`VatAccounting`] (BT-6 / BT-111) and [`Credit`]s drawn down at build.

use alloc::vec::Vec;

use rust_decimal::Decimal;

use crate::line::neg;
use crate::vat::reduce_exact;
use alloc::string::String;

use crate::{
    Amount, AmountScale, Currency, Error, ErrorKind, Location, Result, Rounding, Vat, VatGroup,
};

fn at_scale(d: Decimal, scale: AmountScale) -> Result<Amount> {
    let a = Amount::from_decimal(d)?;
    if !a.fits(scale.decimals) {
        return Err(ErrorKind::ExcessPrecision {
            value: d,
            decimals: scale.decimals,
        }
        .into());
    }
    Ok(a)
}

/// An advance that was invoiced earlier and is deducted here, restated (not
/// re-derived): its VAT treatment, net and tax as they were invoiced.
///
/// Net and tax must fit the document's scale:
/// [`DocumentBuilder::build`](crate::DocumentBuilder::build) refuses excess
/// digits as [`ErrorKind::ExcessPrecision`] at [`Location::Payment`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Advance {
    /// The advance invoice's VAT treatment.
    pub vat: Vat,
    /// Its net, at the document's scale.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub net: Decimal,
    /// Its VAT, at the document's scale.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub tax: Decimal,
}

/// One entry of what was already paid (BT-113 = Σ gross of all entries): an
/// itemised [`Advance`] (gross = net + tax) or a gross payment without VAT
/// information (a transfer, a credit balance). Given to
/// [`DocumentBuilder::payment`](crate::DocumentBuilder::payment), any number,
/// in order; drawn after-tax [`Credit`]s appear as `Gross` entries in
/// [`Document::paid_entries`](crate::Document::paid_entries).
///
/// Negating, splitting and concatenating a document keep every entry, so the
/// residual VAT statement is available on every derived document. Payments
/// may exceed the supply: the payable amount then goes negative.
///
/// ```
/// use billing::{Advance, Payment, Percent, Vat};
/// use rust_decimal::dec;
///
/// let transfer = Payment::Gross { description: "bank transfer".into(), amount: dec!(119.00) };
/// let advance = Payment::Advance(Advance { vat: Vat::standard(Percent::new(dec!(19)))?, net: dec!(100), tax: dec!(19) });
/// # let _ = (transfer, advance);
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Payment {
    /// An invoiced advance, restated per VAT treatment.
    Advance(Advance),
    /// A gross amount paid, with no VAT information.
    Gross {
        /// What the payment was (a transfer, a drawn credit's description).
        description: String,
        /// The gross amount, at the document's scale.
        #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
        amount: Decimal,
    },
}

impl Payment {
    /// The gross paid at `scale`: an advance's net + tax, or the amount.
    fn gross_at(&self, scale: AmountScale) -> Result<Amount> {
        match self {
            Self::Advance(a) => at_scale(a.net, scale)?.checked_add(at_scale(a.tax, scale)?),
            Self::Gross { amount, .. } => at_scale(*amount, scale),
        }
    }

    /// The leaves a document split partitions, in order.
    fn leaves(&self) -> Vec<Decimal> {
        match self {
            Self::Advance(a) => alloc::vec![a.net, a.tax],
            Self::Gross { amount, .. } => alloc::vec![*amount],
        }
    }

    /// Rebuild from split leaves (same order as `leaves`).
    fn with_leaves(&self, parts: &[Decimal]) -> Self {
        match self {
            Self::Advance(a) => Self::Advance(Advance {
                vat: a.vat.clone(),
                net: parts[0],
                tax: parts[1],
            }),
            Self::Gross { description, .. } => Self::Gross {
                description: description.clone(),
                amount: parts[0],
            },
        }
    }

    pub(crate) fn negated(&self) -> Self {
        match self {
            Self::Advance(a) => Self::Advance(Advance {
                vat: a.vat.clone(),
                net: neg(a.net),
                tax: neg(a.tax),
            }),
            Self::Gross {
                description,
                amount,
            } => Self::Gross {
                description: description.clone(),
                amount: neg(*amount),
            },
        }
    }
}

/// Σ gross of `payments` at `scale`; an entry that does not fit
/// (`ExcessPrecision`) or overflows is refused at [`Location::Payment`].
pub(crate) fn paid_total(payments: &[Payment], scale: AmountScale) -> Result<Amount> {
    payments
        .iter()
        .enumerate()
        .try_fold(Amount::ZERO, |acc, (i, p)| {
            p.gross_at(scale)
                .and_then(|g| acc.checked_add(g))
                .map_err(|e| e.at(Location::Payment(i)))
        })
}

/// Every payment's leaves, concatenated, with each payment's `(start, len)`.
pub(crate) fn payment_leaves(payments: &[Payment]) -> (Vec<Decimal>, Vec<(usize, usize)>) {
    let mut rows = Vec::new();
    let mut spans = Vec::with_capacity(payments.len());
    for p in payments {
        let leaves = p.leaves();
        spans.push((rows.len(), leaves.len()));
        rows.extend(leaves);
    }
    (rows, spans)
}

/// The payments rebuilt from one share's leaves (`parts[start..start + len]`).
pub(crate) fn payments_share(
    payments: &[Payment],
    spans: &[(usize, usize)],
    parts: impl Fn(usize) -> Decimal,
) -> Vec<Payment> {
    payments
        .iter()
        .zip(spans)
        .map(|(p, &(start, len))| {
            let leaves: Vec<Decimal> = (start..start + len).map(&parts).collect();
            p.with_leaves(&leaves)
        })
        .collect()
}

/// Cash rounding of the amount due (BT-114): `round_to_increment(BT-112 −
/// BT-113) − (BT-112 − BT-113)`. The increment must be positive and fit the
/// document's scale.
///
/// ```
/// use billing::{CashRounding, Rounding};
/// use rust_decimal::dec;
///
/// let five_rappen = CashRounding { increment: dec!(0.05), rounding: Rounding::HalfAwayFromZero };
/// # let _ = five_rappen;
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CashRounding {
    /// The smallest payable unit, e.g. 0.05.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub increment: Decimal,
    /// How the amount due is rounded to it.
    pub rounding: Rounding,
}

impl CashRounding {
    pub(crate) fn increment_at(&self, scale: AmountScale) -> Result<Amount> {
        match Amount::from_decimal(self.increment) {
            Ok(a) if a.is_positive() && a.fits(scale.decimals) => Ok(a),
            _ => Err(ErrorKind::InvalidCashRounding.into()),
        }
    }
}

/// VAT accounting currency (BT-6) and the conversion rate; BT-111 =
/// `round(BT-110 × rate)` once. The currency must differ from the document's
/// (PEPPOL-EN16931-R005, [`ErrorKind::InvalidVatAccounting`]) and an exchange
/// rate is positive ([`ErrorKind::InvalidRate`]), so BT-111 always has BT-110's
/// sign (sign-symmetric rounding).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct VatAccounting {
    /// BT-6.
    pub currency: Currency,
    /// Units of `currency` per unit of the document currency.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub rate: Decimal,
}

impl VatAccounting {
    /// R005: a different currency; an exchange rate is positive.
    pub(crate) fn validate(&self, document: Currency) -> Result<()> {
        if self.currency == document {
            return Err(ErrorKind::InvalidVatAccounting.into());
        }
        if self.rate <= Decimal::ZERO {
            return Err(ErrorKind::InvalidRate { rate: self.rate }.into());
        }
        Ok(())
    }

    pub(crate) fn total(
        &self,
        document: Currency,
        vat: Amount,
        scale: AmountScale,
    ) -> Result<Amount> {
        self.validate(document)?;
        reduce_exact(&[vat.to_decimal(), self.rate], &[], scale)
    }
}

/// Where a [`Credit`] is drawn: against the taxable base or against the amount
/// due.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum CreditPlacement {
    /// Before tax: the drawn amount becomes document allowances split across
    /// the VAT groups by their bases, so it reduces each group's taxable base
    /// (a promotional or contractual credit).
    BeforeTax,
    /// After tax: the drawn amount is added to BT-113 after the payments, as
    /// a [`Payment::Gross`] entry (a balance the customer already paid for).
    AfterTax,
}

/// A credit balance given to [`DocumentBuilder::credit`](crate::DocumentBuilder::credit)
/// and drawn down against **one** document at build, in call order (the
/// caller's priority). A plain input record: the balance must be above zero
/// and fit the document's scale, else the build refuses it as
/// [`ErrorKind::InvalidCredit`] at [`Location::Credit`].
///
/// The ledger (balances across invoices, expiry, top-ups) is the caller's; the
/// document reports what it drew in
/// [`Document::credit_applications`](crate::Document::credit_applications).
///
/// ```
/// use billing::{AmountScale, Credit, Currency, Document, Line, Percent, Vat};
/// use rust_decimal::dec;
///
/// let doc = Document::builder(Currency::EUR, AmountScale::EN16931)
///     .default_vat(Vat::standard(Percent::new(dec!(19)))?)
///     .line(Line::fixed("subscription", dec!(100)))
///     .credit(Credit::after_tax("wallet", dec!(50)))
///     .build()?;
/// assert_eq!(doc.vat_total().to_string(), "19.00000"); // the tax is untouched
/// assert_eq!(doc.payable().to_string(), "69.00000");
/// # Ok::<(), billing::Error>(())
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Credit {
    /// The description of the allowances a before-tax credit becomes.
    pub description: String,
    /// The balance available to this document: > 0, at the document's scale.
    #[cfg_attr(feature = "serde", serde(with = "crate::decimal::serde_str"))]
    pub balance: Decimal,
    /// Before or after tax.
    pub placement: CreditPlacement,
}

impl Credit {
    /// A credit drawn against the taxable base.
    #[must_use]
    pub fn before_tax(description: impl Into<String>, balance: Decimal) -> Self {
        Self {
            description: description.into(),
            balance,
            placement: CreditPlacement::BeforeTax,
        }
    }

    /// A credit drawn against the amount due, like a payment.
    #[must_use]
    pub fn after_tax(description: impl Into<String>, balance: Decimal) -> Self {
        Self {
            description: description.into(),
            balance,
            placement: CreditPlacement::AfterTax,
        }
    }

    /// The balance at `scale`, or `InvalidCredit`.
    pub(crate) fn balance_at(&self, scale: AmountScale) -> Result<Amount> {
        match Amount::from_decimal(self.balance) {
            Ok(a) if a.is_positive() && a.fits(scale.decimals) => Ok(a),
            _ => Err(ErrorKind::InvalidCredit.into()),
        }
    }
}

/// What one [`Credit`] drew from a document: a result record, at the
/// document's scale, with `applied + remaining = balance`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CreditApplication {
    /// The credit's index in priority (builder call) order.
    pub credit: usize,
    /// The amount drawn against this document.
    pub applied: Amount,
    /// The balance left for the caller's ledger.
    pub remaining: Amount,
}

/// The residual VAT statement (§14 Abs. 5 S. 2 UStG): per group, base and tax
/// minus those of the itemised advances with the same (category, rate); gross
/// payments carry no VAT and are skipped. Errors are located at the advance's
/// index in `payments`.
pub(crate) fn residual_vat(groups: &[VatGroup], payments: &[Payment]) -> Result<Vec<VatGroup>> {
    let mut out: Vec<VatGroup> = groups.to_vec();
    for (i, p) in payments.iter().enumerate() {
        let Payment::Advance(a) = p else { continue };
        let at = |e: Error| e.at(Location::Payment(i));
        let k = groups
            .iter()
            .position(|g| g.vat().key() == a.vat.key())
            .ok_or_else(|| at(ErrorKind::UnmatchedAdvance.into()))?;
        let g = &out[k];
        let base = g
            .taxable_base()
            .checked_sub(Amount::from_decimal(a.net).map_err(at)?)
            .map_err(at)?;
        let tax = g
            .tax()
            .checked_sub(Amount::from_decimal(a.tax).map_err(at)?)
            .map_err(at)?;
        out[k] = VatGroup::new(g.vat().clone(), base, tax);
    }
    Ok(out)
}
