#![doc = include_str!("../README.md")]
#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::float_arithmetic)]
#![warn(missing_docs, unreachable_pub, rust_2018_idioms)]

extern crate alloc;

mod amount;
mod currency;
mod decimal;
mod document;
mod error;
mod line;
mod meter;
mod price;
mod settlement;
mod split;
mod tariff;
mod terms;
mod vat;

pub use amount::{Amount, AmountScale, Rounding};
pub use currency::Currency;
pub use document::{Basis, Bound, BoundKind, Document, DocumentBuilder};
pub use error::{Error, ErrorKind, Location, Result};
pub use line::{
    Adjust, AdjustmentKind, DocumentAdjustment, Line, LineAdjustment, apportion, per_unit_levy,
};
pub use meter::{Aggregation, Meter, Sample, UnitConversion, Usage};
pub use price::{Percent, Price, Quantity, UnitPrice};
pub use rust_decimal::Decimal;
pub use settlement::{
    Advance, CashRounding, Credit, CreditApplication, CreditPlacement, Payment, VatAccounting,
};
pub use split::{split, split_matrix};
pub use tariff::{
    Billed, FeeTotal, PackageRounding, Percentile, Piece, SeriesTotal, Tariff, Tier, TierError,
    Tiers, TransactionFee, nearest_rank, rate_by_key, rate_series,
};
pub use terms::{EarlyPaymentDiscount, InterestSegment, late_payment_interest};
pub use vat::{Exemption, TaxCategory, Vat, VatError, VatGroup, split_gross};
