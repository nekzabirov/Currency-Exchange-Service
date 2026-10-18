//! CoinGecko: cryptocurrency rates.
//!
//! TWO VENDOR QUIRKS, both able to silently corrupt the numbers:
//!
//! 1. **It does not name a coin by its code.** `BTC` is `bitcoin` there, `USDT` is `tether`. The
//!    mapping is built in as the table below and extended by the `COINGECKO_MAPPING` variable, so
//!    a new coin can be enabled without a release.
//! 2. **It answers with the INVERSE number.** `vs_currencies=eur` gives "how many EUR one coin is
//!    worth", while the registry needs "how many coins per one EUR". Forgetting the inversion means
//!    being off by bitcoin's rate squared, i.e. five orders of magnitude.
//!
//! The free plan needs no key; `COINGECKO_API_KEY` is sent as a header when set. Note that keyless
//! calls from many cloud/datacenter IP ranges are answered with HTTP 403.

use std::collections::HashMap;

use async_trait::async_trait;
use currency_provider_sdk::{kit, ProviderConfig, ProviderError, RateProvider};
use rust_decimal::Decimal;

pub const PROVIDER: &str = "coingecko";

const DEFAULT_BASE_URL: &str = "https://api.coingecko.com/api/v3";
const API_KEY_HEADER: &str = "x-cg-demo-api-key";

/// `numeric(40,18)` in the schema.
const MAX_SCALE: u32 = 18;

/// Built-in mapping "code -> the vendor's coin id".
///
/// Extended by the `COINGECKO_MAPPING` variable in the form `TON:the-open-network,XYZ:xyz-coin`;
/// an override takes precedence over the built-in entry.
const COINS: &[(&str, &str)] = &[
    ("BTC", "bitcoin"),
    ("ETH", "ethereum"),
    ("USDT", "tether"),
    ("USDC", "usd-coin"),
    ("LTC", "litecoin"),
    ("XRP", "ripple"),
    ("DOGE", "dogecoin"),
    ("TRX", "tron"),
    ("BNB", "binancecoin"),
    ("SOL", "solana"),
    ("TON", "the-open-network"),
    ("VENOM", "venom"),
];

pub struct CoinGeckoProvider {
    http: reqwest::Client,
}

impl Default for CoinGeckoProvider {
    fn default() -> Self {
        Self {
            http: kit::client(),
        }
    }
}

/// `{"bitcoin":{"eur":95000.12}}`
type PriceResponse = HashMap<String, HashMap<String, serde_json::Number>>;

#[async_trait]
impl RateProvider for CoinGeckoProvider {
    fn key(&self) -> &'static str {
        PROVIDER
    }

    async fn rates(
        &self,
        cfg: &ProviderConfig,
        base: &str,
        codes: &[String],
    ) -> Result<HashMap<String, Decimal>, ProviderError> {
        let base_url = cfg.get_or("base_url", DEFAULT_BASE_URL);
        let vs = base.to_lowercase();

        let overrides = cfg.mapping("mapping");

        // Ids only for the codes we can name at all. An unknown code is not guessed and never makes
        // it into the request: the vendor would answer it with silence, and we would mistake the
        // silence for "no such coin".
        let wanted: Vec<(String, String)> = codes
            .iter()
            .map(|code| code.to_uppercase())
            .filter(|code| *code != base.to_uppercase())
            .filter_map(|code| coin_id(&code, &overrides).map(|id| (code, id)))
            .collect();

        if wanted.is_empty() {
            return Ok(HashMap::new());
        }

        let ids = wanted
            .iter()
            .map(|(_, id)| id.as_str())
            .collect::<Vec<_>>()
            .join(",");

        let mut request = self
            .http
            .get(format!("{base_url}/simple/price"))
            .query(&[("ids", ids.as_str()), ("vs_currencies", vs.as_str())]);

        if let Some(key) = cfg.get("api_key") {
            request = request.header(API_KEY_HEADER, key);
        }

        let prices: PriceResponse =
            kit::decode(PROVIDER, kit::send(PROVIDER, request).await?).await?;

        let mut out = HashMap::new();

        for (code, id) in wanted {
            let Some(base_per_coin) = prices
                .get(&id)
                .and_then(|quotes| quotes.get(&vs))
                .and_then(|n| n.to_string().parse::<Decimal>().ok())
            else {
                continue;
            };

            if base_per_coin <= Decimal::ZERO {
                continue;
            }

            // INVERSION. The vendor said "one coin costs N euro", the registry needs "coins per euro".
            if let Some(rate) = Decimal::ONE.checked_div(base_per_coin) {
                out.insert(code, rate.round_dp(MAX_SCALE));
            }
        }

        Ok(out)
    }
}

fn coin_id(code: &str, overrides: &HashMap<String, String>) -> Option<String> {
    if let Some(id) = overrides.get(code) {
        return Some(id.clone());
    }

    COINS
        .iter()
        .find(|(known, _)| *known == code)
        .map(|(_, id)| (*id).to_string())
}

pub fn provider() -> Box<dyn RateProvider> {
    Box::new(CoinGeckoProvider::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::str::FromStr;

    #[test]
    fn a_code_resolves_to_the_vendors_own_name() {
        assert_eq!(coin_id("BTC", &HashMap::new()).as_deref(), Some("bitcoin"));
        assert_eq!(
            coin_id("USDC", &HashMap::new()).as_deref(),
            Some("usd-coin")
        );
    }

    #[test]
    fn an_unknown_code_has_no_id_and_is_never_guessed() {
        assert_eq!(coin_id("NOPE", &HashMap::new()), None);
    }

    #[test]
    fn an_override_beats_the_built_in_table() {
        let overrides = HashMap::from([("BTC".to_string(), "wrapped-bitcoin".to_string())]);

        assert_eq!(
            coin_id("BTC", &overrides).as_deref(),
            Some("wrapped-bitcoin")
        );
    }

    /// A coin costs 0.8 EUR, so one euro buys 1.25 coins. Without the inversion the registry would
    /// store 0.8, and every exchange would be off by the rate squared.
    #[test]
    fn the_vendor_quote_is_inverted() {
        let base_per_coin = Decimal::from_str("0.8").unwrap();

        assert_eq!(
            Decimal::ONE.checked_div(base_per_coin).unwrap(),
            Decimal::from_str("1.25").unwrap()
        );
    }

    #[test]
    fn a_quote_survives_json_without_losing_digits() {
        let prices: PriceResponse =
            serde_json::from_str(r#"{"bitcoin":{"eur":95123.45678901}}"#).unwrap();

        let quote = prices["bitcoin"]["eur"]
            .to_string()
            .parse::<Decimal>()
            .unwrap();

        assert_eq!(quote, Decimal::from_str("95123.45678901").unwrap());
    }
}
