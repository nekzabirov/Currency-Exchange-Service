# Rate Providers

A rate provider (adapter) answers one question: *how many units of each of these currencies
does one unit of the base buy?* Each currency row names its provider in `currency.provider`.
The `sync` job groups the registry by that key and asks each provider once.

| Key            | Crate                              | Kind   | Key required | Serves (seed) |
|----------------|------------------------------------|--------|--------------|---------------|
| `openexchange` | `crates/providers/openexchange`    | fiat   | yes (`OPENEXCHANGE_APP_ID`) | `EUR`, `UAH` |
| `coinbase`     | `crates/providers/coinbase`        | crypto | no           | `USDT`, `BTC`, `USDC` |
| `coingecko`    | `crates/providers/coingecko`       | crypto | optional (`COINGECKO_API_KEY`) | none by default |

All adapters share `provider-sdk::kit`. It provides one reused HTTP client per adapter with a
15 s request timeout and a 5 s connect timeout. It also logs every vendor call as
`vendor.call`, with the query string stripped, and reads error bodies as text so that an
HTML error page stays visible in the log.

## OpenExchangeRates (`openexchange`)

- Endpoint: `GET {OPENEXCHANGE_BASE_URL}/latest.json?app_id=...`
- The free plan always quotes against **USD** and ignores `base`, so the adapter
  renormalises: `rate(x per 1 base) = usd_rate(x) / usd_rate(base)`.
- If the response does not quote the base itself, the whole call fails. Passing dollar
  numbers off as euro numbers would shift every rate by several percent.
- Numbers are parsed through their decimal string, never through `f64`.
- Settings: `OPENEXCHANGE_APP_ID` (required), `OPENEXCHANGE_BASE_URL`.

## Coinbase (`coinbase`)

- Endpoint: `GET {COINBASE_BASE_URL}/exchange-rates?currency=<BASE>`, public, no key.
- The quote is already in the registry's direction (units per one base), so nothing is
  inverted. Rates arrive as JSON strings and are parsed straight into `Decimal`.
- A response quoted against a different base than requested is refused.
- Zero or unreadable quotes are omitted.
- Setting: `COINBASE_BASE_URL`.

## CoinGecko (`coingecko`)

- Endpoint: `GET {COINGECKO_BASE_URL}/simple/price?ids=...&vs_currencies=<base>`.
- CoinGecko names coins by id rather than by code. A built-in table maps `BTC`, `ETH`, `USDT`,
  `USDC`, `LTC`, `XRP`, `DOGE`, `TRX`, `BNB`, `SOL`, `TON` and `VENOM`. Extend or override it
  with `COINGECKO_MAPPING=CODE:coin-id,...`. Codes without an id are never requested.
- CoinGecko answers "how many base units one coin costs", so the adapter **inverts** the
  quote (`1 / price`).
- Keyless calls from many cloud/datacenter IP ranges get HTTP 403. Set `COINGECKO_API_KEY`
  (sent as `x-cg-demo-api-key`) if you move currencies onto this provider.
- Settings: `COINGECKO_API_KEY`, `COINGECKO_BASE_URL`, `COINGECKO_MAPPING`.

The adapter is compiled in, but the shipped migration `0002_crypto_on_coinbase.sql` moves
every `coingecko` row to `coinbase`. To use CoinGecko, add a migration that sets
`provider = 'coingecko'` on the rows you want. The seed never rewrites existing rows.

## Rules every adapter must follow

1. **An unknown code is omitted, not guessed.** Returning `1` for a code the vendor does not
   know silently equates the currency with the base.
2. **The base currency is not returned.** The sync pins it to `1` as its last write.
   Otherwise a vendor echo could shift the reference point itself.
3. **Errors propagate.** Return a `ProviderError` and do not swallow it. The run then exits
   non-zero and the scheduler retries it.

Errors come in three kinds (`provider-sdk/src/error.rs`): `Vendor` (the vendor answered and
refused), `Transport` (the vendor was not reached) and `Config` (a required setting is missing).

## Adding a provider

A new provider is one new crate plus one line in the registry, with zero changes in `api`,
`store` or `app`.

1. **Create the crate** `crates/providers/<name>/`. It depends only on
   `currency-provider-sdk` plus whatever it needs for HTTP and JSON. Copy `Cargo.toml` from
   `crates/providers/coinbase/`.

   ```rust
   use std::collections::HashMap;

   use async_trait::async_trait;
   use currency_provider_sdk::{kit, ProviderConfig, ProviderError, RateProvider};
   use rust_decimal::Decimal;
   use serde::Deserialize;

   pub const PROVIDER: &str = "myvendor"; // becomes the MYVENDOR_* env prefix

   const DEFAULT_BASE_URL: &str = "https://api.example.com/v1";

   pub struct MyVendorProvider {
       http: reqwest::Client,
   }

   /// `{"rates":{"USD":"1.08","GBP":"0.86"}}`, units per one base.
   #[derive(Deserialize)]
   struct RatesResponse {
       rates: HashMap<String, String>,
   }

   #[async_trait]
   impl RateProvider for MyVendorProvider {
       fn key(&self) -> &'static str {
           PROVIDER
       }

       async fn rates(
           &self,
           cfg: &ProviderConfig,
           base: &str,
           codes: &[String],
       ) -> Result<HashMap<String, Decimal>, ProviderError> {
           let api_key = cfg.require("api_key")?; // MYVENDOR_API_KEY
           let base_url = cfg.get_or("base_url", DEFAULT_BASE_URL); // MYVENDOR_BASE_URL
           let base = base.to_uppercase();

           let request = self
               .http
               .get(format!("{base_url}/rates"))
               .bearer_auth(api_key)
               .query(&[("base", base.as_str())]);

           let response: RatesResponse =
               kit::decode(PROVIDER, kit::send(PROVIDER, request).await?).await?;

           Ok(codes
               .iter()
               .map(|code| code.to_uppercase())
               .filter(|code| *code != base) // rule 2: never return the base
               .filter_map(|code| {
                   // rule 1: an unknown or unusable quote is omitted, not guessed
                   let rate = response.rates.get(&code)?.parse::<Decimal>().ok()?;
                   (rate > Decimal::ZERO).then(|| (code, rate.round_dp(18)))
               })
               .collect())
       }
   }

   pub fn provider() -> Box<dyn RateProvider> {
       Box::new(MyVendorProvider { http: kit::client() })
   }
   ```

2. **Register the crate in the workspace.** In the root `Cargo.toml`, add it to `members`
   and to `[workspace.dependencies]`:
   `currency-provider-myvendor = { path = "crates/providers/myvendor" }`.
3. **Add a feature in `crates/providers/registry/Cargo.toml`.** Add an optional dependency
   and a feature, and include the feature in `default`.
4. **Add one line in `crates/providers/registry/src/lib.rs`:**
   `#[cfg(feature = "myvendor")] all.push(currency_provider_myvendor::provider());`.
   Then update the expected count in the `keys_are_unique` test.
5. **Point currencies at it.** For new currencies, add them to `CURRENCIES` in
   `crates/app/src/bin/seed.rs`. For existing rows, add a migration (never edit an applied
   one). The seed refuses to run if a row names a provider that is not registered.
6. **Write tests.** Cover parsing, unknown codes, the base being skipped and error statuses.
   `crates/providers/coinbase/src/lib.rs` shows how to serve a canned HTTP response from a
   local port.
7. **Update the docs:** this page, [Configuration](Configuration) and `README.md`.
