//! Arbitrary bytes deserialised as JSON into every public serde type.
//!
//! Asserts, per type: no panic; an accepted value serialises, deserialises back
//! to an equal value, and re-serialises byte-identically (the canonical form is
//! a fixed point). Deserialisation runs the same checks as the constructors, so
//! an accepted value is one the constructors could have built — except the
//! result records `Piece`, `SeriesTotal` and `FeeTotal`, which have no constructor
//! and accept any fields (their consumers re-check them: `Document::build` refuses a
//! negative price, `price_inconsistent` reports a stated net off R120). A `Document`
//! additionally equals its own rebuild (`into_builder().build()`) when it is not
//! a split share (a share's tax partition is not a public builder input).
//!
//! The list below must name every public type that implements `Deserialize`;
//! `tests/document.rs` (`fuzz_serde_target_lists_every_deserialize_type`) fails
//! when one is missing.
#![no_main]

use core::fmt::Debug;

use billing::{
    Adjust, AdjustmentKind, Advance, Aggregation, Amount, AmountScale, Basis, Billed, Bound,
    BoundKind, CashRounding, Credit, CreditApplication, CreditPlacement, Currency, Document,
    DocumentAdjustment, EarlyPaymentDiscount, Exemption, FeeTotal, InterestSegment, Line,
    LineAdjustment, Meter, PackageRounding, Payment, Percent, Percentile, Piece, Price, Quantity,
    Rounding, Sample, SeriesTotal, Tariff, TaxCategory, Tier, Tiers, TransactionFee,
    UnitConversion, UnitPrice, Usage, Vat, VatAccounting, VatGroup,
};
use libfuzzer_sys::fuzz_target;
use serde::{Serialize, de::DeserializeOwned};

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(data: &[u8]) -> Option<T> {
    let value = serde_json::from_slice::<T>(data).ok()?;
    let once = serde_json::to_vec(&value).expect("an accepted value serialises");
    let back: T = serde_json::from_slice(&once).unwrap_or_else(|e| {
        panic!(
            "{} did not re-read its own output {:?}: {e}",
            core::any::type_name::<T>(),
            String::from_utf8_lossy(&once)
        )
    });
    assert_eq!(
        back,
        value,
        "{} round-trip changed the value",
        core::any::type_name::<T>()
    );
    let twice = serde_json::to_vec(&back).expect("serialises");
    assert_eq!(
        once,
        twice,
        "{} does not re-serialise identically",
        core::any::type_name::<T>()
    );
    Some(value)
}

fuzz_target!(|data: &[u8]| {
    // Money.
    roundtrip::<Amount<0>>(data);
    roundtrip::<Amount<2>>(data);
    roundtrip::<Amount<5>>(data);
    roundtrip::<AmountScale>(data);
    roundtrip::<Rounding>(data);
    roundtrip::<Currency>(data);
    roundtrip::<Price>(data);
    roundtrip::<Percent>(data);
    roundtrip::<UnitPrice>(data);
    roundtrip::<Quantity>(data);
    // Pricing.
    roundtrip::<Tier>(data);
    roundtrip::<Tiers>(data);
    roundtrip::<Tariff>(data);
    roundtrip::<PackageRounding>(data);
    roundtrip::<Piece>(data);
    roundtrip::<Billed>(data);
    roundtrip::<SeriesTotal>(data);
    roundtrip::<Percentile>(data);
    if let Some(fee) = roundtrip::<TransactionFee>(data) {
        // Deserialising validated it: rating refuses nothing but overflow.
        fee.rate([]).expect("an accepted fee rates");
    }
    roundtrip::<FeeTotal>(data);
    // Metering.
    roundtrip::<Sample>(data);
    roundtrip::<Aggregation>(data);
    roundtrip::<UnitConversion>(data);
    roundtrip::<Meter>(data);
    roundtrip::<Usage>(data);
    // VAT.
    roundtrip::<TaxCategory>(data);
    roundtrip::<Exemption>(data);
    roundtrip::<Vat>(data);
    roundtrip::<VatGroup>(data);
    // Lines and adjustments.
    roundtrip::<AdjustmentKind>(data);
    roundtrip::<Adjust>(data);
    roundtrip::<LineAdjustment>(data);
    roundtrip::<DocumentAdjustment>(data);
    roundtrip::<Line>(data);
    roundtrip::<BoundKind>(data);
    roundtrip::<Basis>(data);
    roundtrip::<Bound>(data);
    // Settlement and terms.
    roundtrip::<Advance>(data);
    roundtrip::<Payment>(data);
    roundtrip::<CashRounding>(data);
    roundtrip::<VatAccounting>(data);
    roundtrip::<CreditPlacement>(data);
    roundtrip::<Credit>(data);
    roundtrip::<CreditApplication>(data);
    roundtrip::<EarlyPaymentDiscount>(data);
    roundtrip::<InterestSegment>(data);
    // Documents: also equal to a rebuild from their own inputs.
    if let Some(doc) = roundtrip::<Document>(data)
        && doc.tax_partition().is_none()
    {
        let rebuilt = doc
            .clone()
            .into_builder()
            .build()
            .unwrap_or_else(|e| panic!("an accepted document does not rebuild: {e}"));
        assert_eq!(rebuilt, doc, "a document differs from its own rebuild");
    }
});
