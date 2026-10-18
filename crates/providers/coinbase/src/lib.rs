//! Coinbase — crypto rates without a key.
//!
//! Exists because CoinGecko answers keyless calls from many cloud/datacenter IP ranges with an
//! HTTP 403, and its key needs an account. Coinbase's public `exchange-rates` endpoint needs neither.
//!
//! Its quote is already in the registry's direction — units of each currency per ONE base — so,
//! unlike CoinGecko, nothing is inverted. Numbers arrive as JSON strings and are parsed straight
//! into `Decimal`: going through `f64` would cost digits on a rate like BTC's `1.3e-5`.

use std::collections::HashMap;
use std::str::FromStr;

use async_trait::async_trait;
use currency_provider_sdk::{kit, ProviderConfig, ProviderError, RateProvider};
use rust_decimal::Decimal;
use serde::Deserialize;

pub const PROVIDER: &str = "coinbase";

const DEFAULT_BASE_URL: &str = "https://api.coinbase.com/v2";

/// `numeric(40,18)` in the schema.
const MAX_SCALE: u32 = 18;

pub struct CoinbaseProvider {
    http: reqwest::Client,
}

impl Default for CoinbaseProvider {
    fn default() -> Self {
        Self {
            http: kit::client(),
        }
    }
}

/// `{"data":{"currency":"EUR","rates":{"BTC":"0.0000135882581144",...}}}`
#[derive(Debug, Deserialize)]
struct ExchangeRatesResponse {
    data: ExchangeRates,
}

#[derive(Debug, Deserialize)]
struct ExchangeRates {
    currency: String,
    rates: HashMap<String, String>,
}

#[async_trait]
impl RateProvider for CoinbaseProvider {
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
        let base = base.to_uppercase();

        let request = self
            .http
            .get(format!("{base_url}/exchange-rates"))
            .query(&[("currency", base.as_str())]);

        let response: ExchangeRatesResponse =
            kit::decode(PROVIDER, kit::send(PROVIDER, request).await?).await?;

        quotes(&response.data, &base, codes)
    }
}

fn quotes(
    data: &ExchangeRates,
    base: &str,
    codes: &[String],
) -> Result<HashMap<String, Decimal>, ProviderError> {
    // Numbers quoted against another base would shift every rate by that cross rate, and nothing
    // downstream could tell.
    if !data.currency.eq_ignore_ascii_case(base) {
        return Err(ProviderError::Vendor {
            provider: PROVIDER,
            message: format!(
                "response is quoted against {} instead of the engine base {base}",
                data.currency
            ),
        });
    }

    Ok(codes
        .iter()
        .map(|code| code.to_uppercase())
        .filter(|code| code != base)
        .filter_map(|code| {
            let rate = data
                .rates
                .get(&code)
                .and_then(|raw| Decimal::from_str(raw.trim()).ok())
                .filter(|rate| *rate > Decimal::ZERO)?;
            Some((code, rate.round_dp(MAX_SCALE)))
        })
        .collect())
}

pub fn provider() -> Box<dyn RateProvider> {
    Box::new(CoinbaseProvider::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const BODY: &str = r#"{"data":{"currency":"EUR","rates":{"BTC":"0.0000135882581144","USDT":"1.134095445472691","USDC":"1.1336583153837433","EUR":"1.0","ZERO":"0","JUNK":"n/a"}}}"#;

    fn codes(list: &[&str]) -> Vec<String> {
        list.iter().map(|code| code.to_string()).collect()
    }

    fn parse(body: &str) -> ExchangeRates {
        serde_json::from_str::<ExchangeRatesResponse>(body)
            .unwrap()
            .data
    }

    fn dec(raw: &str) -> Decimal {
        Decimal::from_str(raw).unwrap()
    }

    /// Serves one canned HTTP response on a local port and returns the base URL pointing at it, so
    /// the whole adapter path — request, status check, decoding — runs as in production.
    async fn vendor(status: &'static str, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();

            let mut request = Vec::new();
            let mut chunk = [0u8; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let read = socket.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
            }

            let response = format!(
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });

        format!("http://{addr}")
    }

    fn cfg(base_url: String) -> ProviderConfig {
        ProviderConfig::new(
            PROVIDER,
            HashMap::from([("base_url".to_string(), base_url)]),
        )
    }

    #[test]
    fn quotes_keep_every_digit_of_the_vendor_string() {
        let rates = quotes(&parse(BODY), "EUR", &codes(&["BTC", "USDT", "USDC"])).unwrap();

        assert_eq!(rates["BTC"], dec("0.0000135882581144"));
        assert_eq!(rates["USDT"], dec("1.134095445472691"));
        assert_eq!(rates["USDC"], dec("1.1336583153837433"));
        assert_eq!(rates.len(), 3);
    }

    #[test]
    fn a_lowercase_code_is_matched_and_returned_uppercase() {
        let rates = quotes(&parse(BODY), "EUR", &codes(&["btc"])).unwrap();

        assert_eq!(rates.get("BTC"), Some(&dec("0.0000135882581144")));
    }

    #[test]
    fn an_unknown_code_is_omitted_rather_than_guessed() {
        let rates = quotes(&parse(BODY), "EUR", &codes(&["BTC", "NOPE"])).unwrap();

        assert!(!rates.contains_key("NOPE"));
        assert_eq!(rates.len(), 1);
    }

    #[test]
    fn a_zero_or_unreadable_quote_is_omitted() {
        let rates = quotes(&parse(BODY), "EUR", &codes(&["ZERO", "JUNK"])).unwrap();

        assert!(rates.is_empty());
    }

    /// The vendor quotes the base itself as `1.0`; returning it would let the vendor move the
    /// reference point the caller pins last.
    #[test]
    fn the_base_is_not_returned_even_though_the_vendor_quotes_it() {
        let rates = quotes(&parse(BODY), "EUR", &codes(&["EUR", "BTC"])).unwrap();

        assert!(!rates.contains_key("EUR"));
        assert_eq!(rates.len(), 1);
    }

    #[test]
    fn quotes_against_another_base_are_refused() {
        let error = quotes(&parse(BODY), "USD", &codes(&["BTC"])).unwrap_err();

        assert!(matches!(
            error,
            ProviderError::Vendor {
                provider: PROVIDER,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn the_adapter_reads_a_live_shaped_response() {
        let base_url = vendor("200 OK", BODY).await;

        let rates = CoinbaseProvider::default()
            .rates(&cfg(base_url), "eur", &codes(&["USDT", "BTC", "EUR"]))
            .await
            .unwrap();

        assert_eq!(rates.get("USDT"), Some(&dec("1.134095445472691")));
        assert_eq!(rates.get("BTC"), Some(&dec("0.0000135882581144")));
        assert_eq!(rates.len(), 2);
    }

    #[tokio::test]
    async fn a_non_success_status_is_an_error() {
        let base_url = vendor(
            "400 Bad Request",
            r#"{"error":"base currency not recognized","code":3}"#,
        )
        .await;

        let error = CoinbaseProvider::default()
            .rates(&cfg(base_url), "EUR", &codes(&["BTC"]))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ProviderError::Vendor {
                provider: PROVIDER,
                ..
            }
        ));
        assert!(error.to_string().contains("HTTP 400"));
    }

    #[tokio::test]
    async fn a_malformed_body_is_an_error() {
        let base_url = vendor("200 OK", "<html>The request could not be satisfied</html>").await;

        let error = CoinbaseProvider::default()
            .rates(&cfg(base_url), "EUR", &codes(&["BTC"]))
            .await
            .unwrap_err();

        assert!(matches!(
            error,
            ProviderError::Vendor {
                provider: PROVIDER,
                ..
            }
        ));
        assert!(error.to_string().contains("unreadable response"));
    }
}
