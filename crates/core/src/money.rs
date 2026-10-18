//! Units and rates. All of the service's arithmetic lives here: no database, no network, fully
//! tested.
//!
//! TWO DIFFERENT THINGS that are easy to mix up, and the price of mixing them up is a balance off
//! by seven orders of magnitude:
//!
//! - `units_scale`: how many storage units make one human-readable unit. Today it is 1e9 ("nano")
//!   for every currency, the same for UAH and for BTC: the scale does NOT depend on the currency.
//!   The column is per currency not because the values differ, but because one day they may need
//!   to change individually, and then it is one row in the database rather than a release of
//!   every consuming service.
//! - `rate`: how many units of this currency one base unit (EUR) buys. This is the exchange rate;
//!   it lives for hours and the sync moves it.
//!
//! Arithmetic is done in `Decimal`, not `f64`. The contract exposes `double` (that is how it was
//! specified), but a binary fraction is unfit internally: 0.1 cannot be represented exactly, and
//! 1e8 satoshi is already at the edge of the mantissa. `from_f64_retain` takes exactly the decimal
//! digits the double held, and everything after that is exact.
//!
//! Rounding is HALF_UP (away from zero), the common convention for money (it matches Java's
//! `RoundingMode.HALF_UP`). Rounding that differs between services means a balance drifting by one
//! unit on every other operation.

use rust_decimal::prelude::*;
use rust_decimal::{Decimal, RoundingStrategy};

/// A registry row in the shape the arithmetic needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rate {
    /// 1 human-readable unit = this many storage units.
    pub units_scale: i64,
    /// Units of this currency per one base unit.
    pub rate: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RateError {
    #[error("amount is not a finite number")]
    NotFinite,
    #[error("amount does not fit in int64")]
    OutOfRange,
    #[error("rate is not usable")]
    BadRate,
}

impl Rate {
    fn scale(&self) -> Result<Decimal, RateError> {
        if self.units_scale <= 0 {
            return Err(RateError::BadRate);
        }
        Ok(Decimal::from(self.units_scale))
    }
}

/// Human-readable amount to storage units.
///
/// The currency takes part ONLY through its scale: the rate is irrelevant here. This is not an
/// exchange but a change of representation of one and the same amount.
pub fn to_units(amount: f64, rate: &Rate) -> Result<i64, RateError> {
    let amount = Decimal::from_f64_retain(amount).ok_or(RateError::NotFinite)?;

    amount
        .checked_mul(rate.scale()?)
        .ok_or(RateError::OutOfRange)?
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .ok_or(RateError::OutOfRange)
}

/// Storage units back to a human-readable amount.
pub fn from_units(units: i64, rate: &Rate) -> Result<f64, RateError> {
    Decimal::from(units)
        .checked_div(rate.scale()?)
        .ok_or(RateError::OutOfRange)?
        .to_f64()
        .ok_or(RateError::OutOfRange)
}

/// Exchange: units of one currency into units of another.
///
/// A classic cross-rate through the base currency, plus a correction for the scales. Today the
/// scales coincide, but the formula must not silently break on the day they diverge:
///
/// ```text
///                units * rate_to * scale_to
/// units_to = round( ────────────────────────── )
///                    rate_from * scale_from
/// ```
///
/// The same currency is an identity and skips the arithmetic: rounding an exchange "into itself"
/// must never take a unit away.
pub fn convert_units(units: i64, from: &Rate, to: &Rate) -> Result<i64, RateError> {
    if from == to {
        return Ok(units);
    }

    if from.rate <= Decimal::ZERO || to.rate <= Decimal::ZERO {
        return Err(RateError::BadRate);
    }

    // The ratio is computed FIRST, before multiplying by the amount, and that is not a matter of
    // style. `Decimal` holds 28 significant digits: a balance near 1e18 units multiplied by the
    // rate and the scale first would overflow them and report "does not fit" on a perfectly legal
    // amount. The ratio of rates and scales is a number close to one, which leaves enough room.
    let ratio = to
        .rate
        .checked_mul(to.scale()?)
        .ok_or(RateError::OutOfRange)?
        .checked_div(
            from.rate
                .checked_mul(from.scale()?)
                .ok_or(RateError::OutOfRange)?,
        )
        .ok_or(RateError::OutOfRange)?;

    Decimal::from(units)
        .checked_mul(ratio)
        .ok_or(RateError::OutOfRange)?
        .round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero)
        .to_i64()
        .ok_or(RateError::OutOfRange)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;

    /// The scale every currency uses today. Stored balances are kept exactly like this.
    const NANO: i64 = 1_000_000_000;

    fn nano(rate: &str) -> Rate {
        Rate {
            units_scale: NANO,
            rate: Decimal::from_str(rate).unwrap(),
        }
    }

    #[test]
    fn a_major_amount_becomes_nano() {
        assert_eq!(to_units(10.5, &nano("1")).unwrap(), 10_500_000_000);
        assert_eq!(from_units(10_500_000_000, &nano("1")).unwrap(), 10.5);
    }

    /// Exactly the case Decimal (rather than f64) is used for: 0.1 cannot be represented as a
    /// binary fraction, and a naive `0.1 * 1e9` yields 100000000.00000001.
    #[test]
    fn a_tenth_does_not_drift() {
        assert_eq!(to_units(0.1, &nano("1")).unwrap(), 100_000_000);
        assert_eq!(to_units(0.3, &nano("1")).unwrap(), 300_000_000);
    }

    /// Bitcoin's eight decimal places fit into nano with room to spare.
    #[test]
    fn a_satoshi_survives() {
        assert_eq!(to_units(0.00000001, &nano("1")).unwrap(), 10);
    }

    /// An exact half rounds AWAY FROM ZERO (HALF_UP), the usual convention for money. The same
    /// amount must yield the same number in every service, otherwise balances drift by one unit on
    /// every other operation.
    ///
    /// The midpoint is taken at a value that is exact as a binary fraction (0.25 with scale 2) so
    /// the test checks the rounding STRATEGY rather than what the double did to the number on the
    /// way; see the test below.
    #[test]
    fn an_exact_half_rounds_away_from_zero() {
        let halves = Rate {
            units_scale: 2,
            rate: Decimal::ONE,
        };

        assert_eq!(to_units(0.25, &halves).unwrap(), 1);
        assert_eq!(to_units(-0.25, &halves).unwrap(), -1);
    }

    /// THE COST OF `double` IN THE CONTRACT, pinned down by a test rather than discovered one day
    /// in production.
    ///
    /// The amount arrives over the wire as a `double`, and a midpoint below the ninth decimal place
    /// cannot be represented in it: the closest double to 1.5e-9 lies SLIGHTLY BELOW the midpoint
    /// (1.49999999999999999e-9) and therefore yields 1, while the one for 2.5e-9 lies slightly
    /// above and yields 3. The binary fraction decides this, not our rounding.
    ///
    /// It does not affect money: this is the tenth decimal place, a fraction of a nano that no
    /// currency has. But it is worth knowing, or one day it turns into an "investigation". If
    /// exactness down to the last unit on any amount is ever needed, the fix is not rounding but a
    /// string instead of a double in the proto.
    #[test]
    fn a_sub_nano_half_is_decided_by_the_double_not_by_us() {
        assert_eq!(to_units(0.0000000015, &nano("1")).unwrap(), 1);
        assert_eq!(to_units(0.0000000025, &nano("1")).unwrap(), 3);
    }

    #[test]
    fn a_negative_amount_is_allowed() {
        // The sign carries the direction of the money movement and must not be forbidden here:
        // a reversal arrives as a negative amount.
        assert_eq!(to_units(-2.5, &nano("1")).unwrap(), -2_500_000_000);
    }

    #[test]
    fn nan_and_infinity_are_refused() {
        assert_eq!(to_units(f64::NAN, &nano("1")), Err(RateError::NotFinite));
        assert_eq!(
            to_units(f64::INFINITY, &nano("1")),
            Err(RateError::NotFinite)
        );
    }

    /// A trillion euro in nano is 1e21, while int64 ends at 9.2e18. A refusal, not a silent
    /// overflow that flips the sign.
    #[test]
    fn an_amount_beyond_int64_is_refused() {
        assert_eq!(to_units(1e12, &nano("1")), Err(RateError::OutOfRange));
    }

    /// A legitimately large balance converts instead of hitting the width of the intermediate
    /// product: 1e9 EUR is 1e18 units, and the naive "amount x rate x scale" formula would exceed
    /// Decimal's 28 significant digits before the division.
    #[test]
    fn a_large_balance_still_converts() {
        let eur = nano("1");
        let uah = nano("47.5");

        assert_eq!(
            convert_units(1_000_000_000_000_000_000, &eur, &uah),
            Err(RateError::OutOfRange)
        );
        // 100 million EUR in UAH still fits: 4.75e18 < 9.2e18.
        assert_eq!(
            convert_units(100_000_000_000_000_000, &eur, &uah).unwrap(),
            4_750_000_000_000_000_000
        );
    }

    /// 1 EUR at a rate of 47.5 UAH per euro.
    #[test]
    fn an_exchange_uses_both_rates() {
        let eur = nano("1");
        let uah = nano("47.5");

        assert_eq!(convert_units(NANO, &eur, &uah).unwrap(), 47_500_000_000);
        assert_eq!(convert_units(47_500_000_000, &uah, &eur).unwrap(), NANO);
    }

    /// An exchange between two non-base currencies goes through the base and does not lose it on
    /// the way.
    #[test]
    fn an_exchange_between_two_non_base_currencies_goes_through_the_base() {
        let uah = nano("47.5");
        let usdt = nano("1.16");

        // 47.5 UAH = 1 EUR = 1.16 USDT
        assert_eq!(
            convert_units(47_500_000_000, &uah, &usdt).unwrap(),
            1_160_000_000
        );
    }

    #[test]
    fn the_same_currency_is_an_identity() {
        let uah = nano("47.5");
        assert_eq!(convert_units(7, &uah, &uah).unwrap(), 7);
    }

    /// Scales are identical today, but the formula must survive the day they diverge.
    #[test]
    fn a_different_scale_is_honoured() {
        let eur = nano("1");
        let cents = Rate {
            units_scale: 100,
            rate: Decimal::from_str("1").unwrap(),
        };

        // 1 EUR in nano -> 1 EUR in hundredths
        assert_eq!(convert_units(NANO, &eur, &cents).unwrap(), 100);
        assert_eq!(convert_units(100, &cents, &eur).unwrap(), NANO);
    }

    #[test]
    fn a_zero_rate_is_refused_rather_than_converting_to_nothing() {
        let eur = nano("1");
        let dead = nano("0");

        assert_eq!(convert_units(NANO, &eur, &dead), Err(RateError::BadRate));
        assert_eq!(convert_units(NANO, &dead, &eur), Err(RateError::BadRate));
    }

    #[test]
    fn a_zero_scale_is_refused() {
        let broken = Rate {
            units_scale: 0,
            rate: Decimal::ONE,
        };
        assert_eq!(to_units(1.0, &broken), Err(RateError::BadRate));
    }
}
