//! OpenExchangeRates: fiat rates.
//!
//! THE VENDOR QUOTES AGAINST THE DOLLAR, AND THERE IS NO WAY AROUND IT: on the free plan the `base`
//! parameter is ignored and the answer is always in USD. So the adapter renormalises itself:
//!
//! ```text
//! rate(x per 1 EUR) = usd_rate(x) / usd_rate(EUR)
//! ```
//!
//! If the response does not quote the base itself, there is nothing to compute, and that is a
//! refusal rather than "take it as is". Silently passing dollar numbers off as euro numbers would
//! shift EVERY rate by 8-10 percent, and the only way to notice would be a user complaint.

use std::collections::HashMap;

use async_trait::async_trait;
use currency_provider_sdk::{kit, ProviderConfig, ProviderError, RateProvider};
use rust_decimal::Decimal;
use serde::Deserialize;

pub const PROVIDER: &str = "openexchange";

const DEFAULT_BASE_URL: &str = "https://openexchangerates.org/api";

/// `numeric(40,18)` in the schema. The database would not accept a longer fraction anyway, and an
/// unbounded `checked_div` inflates the representation to Decimal's ceiling on every operation.
const MAX_SCALE: u32 = 18;

pub struct OpenExchangeProvider {
    http: reqwest::Client,
}

impl Default for OpenExchangeProvider {
    fn default() -> Self {
        Self {
            http: kit::client(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct LatestResponse {
    rates: HashMap<String, serde_json::Number>,
}

#[async_trait]
impl RateProvider for OpenExchangeProvider {
    fn key(&self) -> &'static str {
        PROVIDER
    }

    async fn rates(
        &self,
        cfg: &ProviderConfig,
        base: &str,
        codes: &[String],
    ) -> Result<HashMap<String, Decimal>, ProviderError> {
        let app_id = cfg.require("app_id")?;
        let base_url = cfg.get_or("base_url", DEFAULT_BASE_URL);

        let response = kit::send(
            PROVIDER,
            self.http
                .get(format!("{base_url}/latest.json"))
                .query(&[("app_id", app_id)]),
        )
        .await?;

        let latest: LatestResponse = kit::decode(PROVIDER, response).await?;

        let base_per_usd = decimal(latest.rates.get(&base.to_uppercase())).ok_or_else(|| {
            ProviderError::Vendor {
                provider: PROVIDER,
                message: format!("response does not quote the engine base {base}"),
            }
        })?;

        if base_per_usd <= Decimal::ZERO {
            return Err(ProviderError::Vendor {
                provider: PROVIDER,
                message: format!("base {base} quoted as {base_per_usd}"),
            });
        }

        let mut out = HashMap::new();

        for code in codes {
            let code = code.to_uppercase();

            // The base is not returned: the caller pins it to one as the last write.
            if code == base.to_uppercase() {
                continue;
            }

            // An unknown code is OMITTED. Substituting one would equate the currency with the base.
            let Some(per_usd) = decimal(latest.rates.get(&code)) else {
                continue;
            };

            if per_usd <= Decimal::ZERO {
                continue;
            }

            if let Some(rate) = per_usd.checked_div(base_per_usd) {
                out.insert(code, rate.round_dp(MAX_SCALE));
            }
        }

        Ok(out)
    }
}

fn decimal(value: Option<&serde_json::Number>) -> Option<Decimal> {
    // Through a string rather than f64: the vendor's numbers carry eight decimal places, and a
    // binary fraction halfway through would cost them precision before they reach the database.
    value.and_then(|n| n.to_string().parse::<Decimal>().ok())
}

pub fn provider() -> Box<dyn RateProvider> {
    Box::new(OpenExchangeProvider::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;

    fn body() -> LatestResponse {
        serde_json::from_str(r#"{"base":"USD","rates":{"USD":1,"EUR":0.92,"UAH":41.4,"XYZ":0}}"#)
            .unwrap()
    }

    /// Renormalisation: 41.4 UAH per dollar and 0.92 EUR per dollar make 45 UAH per euro.
    #[test]
    fn dollar_quotes_are_renormalised_to_the_engine_base() {
        let latest = body();
        let base_per_usd = decimal(latest.rates.get("EUR")).unwrap();
        let uah_per_usd = decimal(latest.rates.get("UAH")).unwrap();

        assert_eq!(
            uah_per_usd.checked_div(base_per_usd).unwrap().round_dp(4),
            Decimal::from_str("45.0000").unwrap()
        );
    }

    #[test]
    fn a_number_keeps_its_decimal_digits() {
        let latest = body();
        assert_eq!(
            decimal(latest.rates.get("EUR")),
            Some(Decimal::from_str("0.92").unwrap())
        );
    }

    #[test]
    fn an_unknown_code_is_absent_rather_than_one() {
        let latest = body();
        assert_eq!(decimal(latest.rates.get("NOPE")), None);
    }
}
