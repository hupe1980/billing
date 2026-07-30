//! [`Quantity`] and [`UnitPrice`] — value + unit-label pairs used in [`crate::LineItem`].
use rust_decimal::Decimal;

use crate::error::BillingError;

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
/// A measured quantity: a numeric value paired with a unit label, and optionally
/// the interchange code for that unit.
pub struct Quantity {
    /// The numeric value.
    pub value: Decimal,
    /// Unit label, e.g. `"kWh"`, `"m³"`, `"seats"`, `"GB"`.
    ///
    /// **Display text**, and load-bearing for [`crate::PerUnitLevy`] base matching.
    /// It is deliberately not a code list: `"Stück"` and `"pcs"` are both perfectly
    /// good labels for the same thing. For the machine-readable code, see
    /// [`Quantity::code`].
    pub unit: String,
    /// EN 16931 **BT-130** — the UN/ECE Recommendation 20 / 21 unit code
    /// (`"KWH"`, `"H87"`, `"MTQ"`), if known.
    ///
    /// Rule **BR-CL-23** restricts BT-130 to that code list, and the mapping from a
    /// display label is *not* mechanical: `"Stk"`, `"Stück"`, `"pcs"` and
    /// `"pieces"` are all `H87`. A consumer emitting EN 16931 therefore has to
    /// either carry a resolver table or ask the caller — and the caller usually
    /// knows the answer exactly. This field is where that answer goes, so it
    /// survives into the finished document instead of being re-derived downstream.
    ///
    /// The engine never interprets it; [`Quantity::unit`] remains what drives
    /// levy matching and generated labels.
    #[cfg_attr(feature = "serde", serde(default))]
    pub code: Option<String>,
}

impl Quantity {
    #[must_use]
    /// Create a new `Quantity` with no unit code.
    pub fn new(value: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
            code: None,
        }
    }

    /// Attach the UN/ECE Rec 20 / 21 unit code — EN 16931 BT-130.
    ///
    /// ```rust
    /// use billing::Quantity;
    /// use rust_decimal::dec;
    ///
    /// let q = Quantity::new(dec!(1234.567), "kWh").with_code("KWH");
    /// assert_eq!(q.code.as_deref(), Some("KWH"));
    /// // The display label is untouched — it is what the invoice prints.
    /// assert_eq!(q.unit, "kWh");
    /// ```
    #[must_use]
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "UnitPriceRepr"))]
#[derive(Debug, Clone, PartialEq, Eq)]
/// A unit price — EN 16931 **BG-29 PRICE DETAILS** in full.
///
/// Stored as [`rust_decimal::Decimal`] (not [`crate::Amount`]) because unit
/// prices often require higher precision than invoice totals.
///
/// # The whole subgroup, not just the mandatory term
///
/// BG-29 has five business terms. All five live here:
///
/// | BT | Name | Field | Rules |
/// |---|---|---|---|
/// | BT-146 | Item net price | [`value`](Self::value) | BR-26 (mandatory), BR-27 |
/// | BT-147 | Item price discount | [`price_discount`](Self::price_discount) | `PEPPOL-EN16931-R046` |
/// | BT-148 | Item gross price | [`gross_price`](Self::gross_price) | BR-28, `R046` |
/// | BT-149 | Item price base quantity | [`base_quantity`](Self::base_quantity) | `R120`, `R121` |
/// | BT-150 | …its unit of measure code | [`base_quantity_code`](Self::base_quantity_code) | BR-CL-23, `R130` |
///
/// Both optional halves are **fatal-rule territory** in Peppol, and both are
/// ordinary commercial practice rather than exotica — EN 16931-1 Annex A spends
/// two of its eight worked examples on them (A.1.3 *Item price base quantity*,
/// A.1.6 *Negative Invoice line*).
///
/// ## BT-149 / BT-150 — "EUR 12,00 per 100 pieces"
///
/// Without a price base quantity the caller has to pre-divide to EUR 0,12 per
/// piece, which states a BT-146 the seller never quoted, and — for a price that
/// does not divide evenly, like `12,00 / 7` — bakes a rounding error into the
/// line before the invoice arithmetic even starts. The base quantity is also
/// load-bearing rather than decorative: `PEPPOL-EN16931-R120` computes the line
/// net amount as
///
/// > BT-131 = BT-129 × (BT-146 ÷ BT-149) + Σ BG-28 − Σ BG-27
///
/// so [`crate::LineItemBuilder::build`] divides by it. `None` means 1, which is
/// exactly how the rule's own `$baseQuantity` variable is defined.
///
/// ## BT-147 / BT-148 — list price less a line discount
///
/// `9,50` gross − `1,00` discount = `8,50` net. BT-147 / BT-148 move the
/// **price**, and nothing else: BT-131 is computed from the resulting BT-146 and
/// is not itself adjusted. That distinguishes them from both of the crate's
/// allowance types —
///
/// | Group | Type | Moves |
/// |---|---|---|
/// | BG-27 / BG-28 line allowance / charge | [`crate::LineAllowanceCharge`] | **BT-131**, via `R120` |
/// | BG-20 / BG-21 document allowance / charge | [`crate::AllowanceCharge`] | **BT-107 / BT-108** → BT-109 |
/// | BG-29 price discount | this type | **BT-146** only |
///
/// Peppol keeps price level apart too — `PEPPOL-EN16931-R044` makes a *charge* at
/// price level illegal outright, while allowing the discount.
///
/// Use [`UnitPrice::discounted`] to state them; it derives BT-146 so that
/// `PEPPOL-EN16931-R046` — which is an **exact** equality with no tolerance,
/// unlike `R040`'s ±0.02 — cannot be violated.
///
/// # Sign: BR-27 / BR-28 are the caller's to honour
///
/// EN 16931 says BT-146 and BT-148 shall not be negative. This crate accepts
/// negative prices anyway, because they are legally binding in spot markets
/// (EPEX negative-price hours, §27 EEG 2023) and refusing them would make the
/// engine unable to represent a real invoice. [`validate`](Self::validate)
/// therefore checks the *structural* invariants — the ones that are unconditional
/// — and leaves the sign to the consumer, which must re-model a negative-price
/// line (as a credit position, or an allowance) before emitting EN 16931.
pub struct UnitPrice {
    /// EN 16931 **BT-146** — the item net price, per
    /// [`base_quantity`](Self::base_quantity) units.
    pub value: Decimal,
    /// Price unit label, e.g. `"EUR/kWh"`, `"EUR/seat/month"`.
    ///
    /// **Display text.** For the machine-readable unit of the price base
    /// quantity, see [`base_quantity_code`](Self::base_quantity_code).
    pub unit: String, // e.g. "EUR/kWh", "EUR/seat/month"
    /// EN 16931 **BT-149** — the number of item units the price applies to.
    ///
    /// `None` means 1, matching `PEPPOL-EN16931-R120`'s own default.
    /// `PEPPOL-EN16931-R121` requires it strictly above zero when present, which
    /// [`validate`](Self::validate) enforces.
    #[cfg_attr(feature = "serde", serde(default))]
    pub base_quantity: Option<Decimal>,
    /// EN 16931 **BT-150** — the UN/ECE Rec 20 / 21 unit code of
    /// [`base_quantity`](Self::base_quantity).
    ///
    /// Constrained to that code list by BR-CL-23, and required by
    /// `PEPPOL-EN16931-R130` (**fatal**) to equal the invoiced quantity's own code
    /// ([`Quantity::code`], BT-130):
    ///
    /// > `[PEPPOL-EN16931-R130]` Unit code of price base quantity MUST be same as
    /// > invoiced quantity.
    ///
    /// [`crate::LineItem::validate`] checks that cross-field agreement, because
    /// only the line knows both. Meaningless without
    /// [`base_quantity`](Self::base_quantity) — in UBL it is an attribute of
    /// `cbc:BaseQuantity` and cannot exist alone — so stating one without the
    /// other is rejected.
    #[cfg_attr(feature = "serde", serde(default))]
    pub base_quantity_code: Option<String>,
    /// EN 16931 **BT-148** — the item gross price, before
    /// [`price_discount`](Self::price_discount). BR-28: shall not be negative.
    #[cfg_attr(feature = "serde", serde(default))]
    pub gross_price: Option<Decimal>,
    /// EN 16931 **BT-147** — the discount subtracted from BT-148 to reach BT-146.
    ///
    /// Only meaningful alongside [`gross_price`](Self::gross_price), since the
    /// standard defines it as a subtraction *from* the gross price; stating it
    /// alone is rejected.
    #[cfg_attr(feature = "serde", serde(default))]
    pub price_discount: Option<Decimal>,
}

#[cfg(feature = "serde")]
#[derive(serde::Deserialize)]
struct UnitPriceRepr {
    value: Decimal,
    unit: String,
    #[serde(default)]
    base_quantity: Option<Decimal>,
    #[serde(default)]
    base_quantity_code: Option<String>,
    #[serde(default)]
    gross_price: Option<Decimal>,
    #[serde(default)]
    price_discount: Option<Decimal>,
}

#[cfg(feature = "serde")]
impl TryFrom<UnitPriceRepr> for UnitPrice {
    type Error = BillingError;
    fn try_from(r: UnitPriceRepr) -> Result<Self, Self::Error> {
        let p = UnitPrice {
            value: r.value,
            unit: r.unit,
            base_quantity: r.base_quantity,
            base_quantity_code: r.base_quantity_code,
            gross_price: r.gross_price,
            price_discount: r.price_discount,
        };
        p.validate()?;
        Ok(p)
    }
}

impl UnitPrice {
    #[must_use]
    /// Create a new `UnitPrice` — BT-146 alone, applying to one unit.
    pub fn new(value: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value,
            unit: unit.into(),
            base_quantity: None,
            base_quantity_code: None,
            gross_price: None,
            price_discount: None,
        }
    }

    /// A price quoted as a **gross price less a discount** — BT-148 − BT-147.
    ///
    /// BT-146 is *derived*, never passed in, so `PEPPOL-EN16931-R046` holds by
    /// construction:
    ///
    /// > `[PEPPOL-EN16931-R046]` Item net price MUST equal (Gross price −
    /// > Allowance amount) when gross price is provided.
    ///
    /// — **fatal**, and unlike `R040` it is an exact equality with no tolerance,
    /// so there is no room for a caller to compute the net price themselves and be
    /// a cent out.
    ///
    /// This is EN 16931-1 Annex A.1.6 (*Example 5*), which uses the pattern on
    /// every line.
    ///
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// // Annex A.1.6: gross 9,50 − discount 1,00 = net 8,50.
    /// let price = UnitPrice::discounted(dec!(9.50), dec!(1.00), "EUR/pcs");
    /// assert_eq!(price.value,         dec!(8.50)); // BT-146, derived
    /// assert_eq!(price.gross_price,   Some(dec!(9.50))); // BT-148
    /// assert_eq!(price.price_discount, Some(dec!(1.00))); // BT-147
    ///
    /// let line = LineItem::for_usage("Item", Quantity::new(dec!(10), "pcs"), price)
    ///     .build().unwrap();
    /// assert_eq!(line.net_amount, Amount::<5>::parse("85.00000").unwrap());
    /// ```
    ///
    /// # Panics
    /// If `gross - discount` leaves [`Decimal`]'s range, matching the crate's
    /// "overflow is visible, never silent" rule for infallible operators. Only
    /// reachable with operands within one of each other of `Decimal::MAX`
    /// (~7.9 × 10²⁸), which no unit price occupies.
    #[must_use]
    pub fn discounted(gross: Decimal, discount: Decimal, unit: impl Into<String>) -> Self {
        Self {
            value: gross - discount,
            unit: unit.into(),
            base_quantity: None,
            base_quantity_code: None,
            gross_price: Some(gross),
            price_discount: Some(discount),
        }
    }

    /// Declare that the price applies to `base_quantity` units — EN 16931
    /// **BT-149**.
    ///
    /// The "EUR 12,00 per 100 pieces" case, EN 16931-1 Annex A.1.3. The line's
    /// net amount becomes `quantity × (price ÷ base_quantity)`, per
    /// `PEPPOL-EN16931-R120`.
    ///
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// // 250 pieces at EUR 12,00 per 100 → 30,00. The quoted price survives.
    /// let price = UnitPrice::new(dec!(12.00), "EUR/100 pcs").per(dec!(100));
    /// let line = LineItem::for_usage("Schrauben", Quantity::new(dec!(250), "pcs"), price)
    ///     .build().unwrap();
    ///
    /// assert_eq!(line.net_amount, Amount::<5>::parse("30.00000").unwrap());
    /// assert_eq!(line.unit_price.unwrap().value, dec!(12.00)); // not 0.12
    /// ```
    #[must_use]
    pub fn per(mut self, base_quantity: Decimal) -> Self {
        self.base_quantity = Some(base_quantity);
        self
    }

    /// Attach the UN/ECE Rec 20 / 21 unit code of the price base quantity — EN
    /// 16931 **BT-150**.
    ///
    /// Must equal the invoiced quantity's [`Quantity::code`] (BT-130) —
    /// `PEPPOL-EN16931-R130`, **fatal** — which
    /// [`crate::LineItem::validate`] checks.
    ///
    /// ```rust
    /// use billing::{LineItem, Quantity, UnitPrice};
    /// use rust_decimal::dec;
    ///
    /// let line = LineItem::for_usage(
    ///     "Schrauben",
    ///     Quantity::new(dec!(250), "pcs").with_code("H87"),          // BT-130
    ///     UnitPrice::new(dec!(12.00), "EUR/100 pcs")
    ///         .per(dec!(100))                                        // BT-149
    ///         .with_base_quantity_code("H87"),                       // BT-150 — must match
    /// ).build().unwrap();
    /// assert!(line.validate().is_ok());
    /// ```
    #[must_use]
    pub fn with_base_quantity_code(mut self, code: impl Into<String>) -> Self {
        self.base_quantity_code = Some(code.into());
        self
    }

    /// The price of a **single** unit — BT-146 ÷ BT-149.
    ///
    /// What `PEPPOL-EN16931-R120` multiplies the invoiced quantity by. Equal to
    /// [`value`](Self::value) when no base quantity is stated.
    ///
    /// Prefer [`crate::LineItemBuilder::build`], which reassociates the product to
    /// `(quantity × price) ÷ base` and so avoids rounding the quotient first; this
    /// accessor exists for display and for callers doing their own arithmetic.
    ///
    /// # Errors
    /// [`BillingError::MonetaryOverflow`] if the quotient cannot be represented.
    /// A zero base quantity is rejected by [`validate`](Self::validate) and would
    /// surface here as the same error.
    pub fn per_unit_value(&self) -> Result<Decimal, BillingError> {
        let Some(base) = self.base_quantity else {
            return Ok(self.value);
        };
        self.value
            .checked_div(base)
            .ok_or(BillingError::MonetaryOverflow {
                precision: 5,
                input_value: None,
            })
    }

    /// Check the BG-29 invariants that hold regardless of profile.
    ///
    /// 1. **`PEPPOL-EN16931-R046`** — when BT-148 is stated, BT-146 must equal
    ///    BT-148 − BT-147 *exactly*. [`discounted`](Self::discounted) guarantees
    ///    this; a hand-assembled struct might not.
    /// 2. **BT-147 without BT-148** — a discount defined as a subtraction from a
    ///    gross price that is not stated.
    /// 3. **`PEPPOL-EN16931-R121`** — BT-149 must be strictly above zero.
    /// 4. **BT-150 without BT-149** — unrepresentable in UBL, where BT-150 is an
    ///    attribute of `cbc:BaseQuantity`.
    ///
    /// The cross-field rule `PEPPOL-EN16931-R130` (BT-150 = BT-130) needs the
    /// invoiced quantity and so lives in [`crate::LineItem::validate`]. The sign
    /// rules BR-27 / BR-28 are deliberately not checked — see the type-level docs.
    ///
    /// Runs automatically in [`crate::LineItemBuilder::build`],
    /// [`crate::LineItem::validate`] and on deserialisation.
    ///
    /// # Errors
    /// [`BillingError::InvalidInput`] naming the violated rule.
    pub fn validate(&self) -> Result<(), BillingError> {
        match (self.gross_price, self.price_discount) {
            (Some(gross), discount) => {
                let expected = gross - discount.unwrap_or(Decimal::ZERO);
                if self.value != expected {
                    return Err(BillingError::InvalidInput {
                        reason: format!(
                            "item gross price {gross} (BT-148) less discount {} (BT-147) is \
                             {expected}, but the item net price (BT-146) is {}; \
                             PEPPOL-EN16931-R046 requires exact equality",
                            discount.unwrap_or(Decimal::ZERO),
                            self.value
                        ),
                    });
                }
            }
            (None, Some(discount)) => {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "item price discount {discount} (BT-147) is set without an item gross \
                         price (BT-148); BT-147 is defined as the amount subtracted from BT-148"
                    ),
                });
            }
            (None, None) => {}
        }
        match (self.base_quantity, &self.base_quantity_code) {
            (Some(base), _) if base <= Decimal::ZERO => {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "item price base quantity {base} (BT-149) must be above zero; \
                         PEPPOL-EN16931-R121"
                    ),
                });
            }
            (None, Some(code)) => {
                return Err(BillingError::InvalidInput {
                    reason: format!(
                        "item price base quantity unit code {code:?} (BT-150) is set without a \
                         base quantity (BT-149), which it is an attribute of"
                    ),
                });
            }
            _ => {}
        }
        Ok(())
    }

    /// Round the price to `scale` decimal places with an explicit strategy.
    ///
    /// Use when a price is *derived* rather than quoted — a `ct/kWh → EUR/kWh`
    /// division, or a total divided by a quantity — and the exact quotient carries
    /// more decimals than the price should display. Without this the stored
    /// `unit_price` and the invoice's own `quantity × unit_price` arithmetic can
    /// disagree in the last digits.
    ///
    /// `scale` is clamped to 28, [`Decimal`]'s maximum: above that
    /// `round_dp_with_strategy` silently no-ops, which would leave the price
    /// unrounded while the caller was promised `scale` decimals.
    ///
    /// This replaces the former `LineItem::for_usage_rounded` /
    /// `credit_for_usage_rounded` constructors, which needed seven positional
    /// arguments to say the same thing.
    ///
    /// ```rust
    /// use billing::{LineItem, Amount, Quantity, UnitPrice, RoundingStrategy};
    /// use rust_decimal::dec;
    ///
    /// // 1/3 ct/kWh as EUR/kWh is non-terminating — pin it to 6 decimals.
    /// let price = UnitPrice::new(dec!(1) / dec!(300), "EUR/kWh")
    ///     .rounded(6, RoundingStrategy::MidpointAwayFromZero);
    /// assert_eq!(price.value, dec!(0.003333));
    ///
    /// let item = LineItem::for_usage("Arbeit", Quantity::new(dec!(500), "kWh"), price)
    ///     .build().unwrap();
    /// assert_eq!(item.net_amount, Amount::<5>::parse("1.66650").unwrap());
    /// ```
    ///
    /// # With a gross price, BT-146 is re-derived rather than rounded
    ///
    /// Rounding BT-146 on its own would break `PEPPOL-EN16931-R046`, which is an
    /// *exact* equality. So when [`gross_price`](Self::gross_price) is present,
    /// BT-148 and BT-147 — the numbers the seller actually quoted — are rounded to
    /// `scale` and BT-146 is recomputed from them. Their difference then has at
    /// most `scale` decimals too, so the identity survives exactly and the net
    /// price still lands on `scale`.
    ///
    /// [`base_quantity`](Self::base_quantity) (BT-149) is a count, not a price, and
    /// is left alone.
    ///
    /// ```rust
    /// use billing::{UnitPrice, RoundingStrategy};
    /// use rust_decimal::dec;
    ///
    /// let p = UnitPrice::discounted(dec!(9.5049), dec!(1.0049), "EUR/pcs")
    ///     .rounded(2, RoundingStrategy::MidpointAwayFromZero);
    ///
    /// assert_eq!(p.gross_price,    Some(dec!(9.50)));
    /// assert_eq!(p.price_discount, Some(dec!(1.00)));
    /// assert_eq!(p.value,          dec!(8.50)); // exactly 9.50 − 1.00, so R046 holds
    /// assert!(p.validate().is_ok());
    /// ```
    ///
    /// # Panics
    /// If the re-derived `gross - discount` leaves [`Decimal`]'s range — see
    /// [`discounted`](Self::discounted), which has the same bound.
    #[must_use]
    pub fn rounded(mut self, scale: u32, strategy: crate::amount::RoundingStrategy) -> Self {
        /// `Decimal`'s maximum representable scale.
        const MAX_DECIMAL_SCALE: u32 = 28;
        let scale = scale.min(MAX_DECIMAL_SCALE);
        let strategy = strategy.into();
        let round = |d: Decimal| d.round_dp_with_strategy(scale, strategy);

        self.gross_price = self.gross_price.map(round);
        self.price_discount = self.price_discount.map(round);
        // BT-146 is derived wherever BT-148 exists, so that R046 stays exact.
        self.value = match self.gross_price {
            Some(gross) => gross - self.price_discount.unwrap_or(Decimal::ZERO),
            None => round(self.value),
        };
        self
    }
}
