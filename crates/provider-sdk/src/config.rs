//! Adapter settings: API keys, base URLs, code mappings.
//!
//! The source is the environment, not a table: there are only a handful of these values, they
//! change when a secret is rotated, and keeping them in a database row would mean keeping a copy
//! of the secret in the database.
//!
//! Shape: `<ADAPTER KEY>_<NAME>` in upper case. The adapter asks for `cfg.require("app_id")` and
//! knows nothing about what the variable was called outside.

use std::collections::HashMap;

use crate::ProviderError;

#[derive(Debug, Clone, Default)]
pub struct ProviderConfig {
    provider: &'static str,
    values: HashMap<String, String>,
}

impl ProviderConfig {
    /// Collects the adapter's settings from the environment by the prefix of its key.
    ///
    /// An empty value counts as ABSENT. In Kubernetes an unfilled secret key arrives in the pod as
    /// an empty string rather than a missing variable, and without this check the adapter would
    /// call the vendor with `appId=` instead of plainly saying "not configured".
    pub fn from_env(provider: &'static str) -> Self {
        let prefix = format!("{}_", provider.to_uppercase().replace('-', "_"));

        let values = std::env::vars()
            .filter_map(|(key, value)| {
                let name = key.strip_prefix(&prefix)?;
                let value = value.trim();
                if value.is_empty() {
                    return None;
                }
                Some((name.to_lowercase(), value.to_string()))
            })
            .collect();

        Self { provider, values }
    }

    pub fn new(provider: &'static str, values: HashMap<String, String>) -> Self {
        Self { provider, values }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.values.get(key).map(String::as_str)
    }

    pub fn get_or<'a>(&'a self, key: &str, fallback: &'a str) -> &'a str {
        self.get(key).unwrap_or(fallback)
    }

    /// A required value. Its absence is a refusal of this adapter, not a process panic: the other
    /// adapters must still run without it.
    pub fn require(&self, key: &'static str) -> Result<&str, ProviderError> {
        self.get(key).ok_or(ProviderError::Config {
            provider: self.provider,
            key,
        })
    }

    /// Mapping "currency code -> vendor identifier", parsed from a string like `BTC:bitcoin,TON:the-open-network`.
    ///
    /// Needed where a vendor does not name a currency by its code: CoinGecko calls it `bitcoin`, not `BTC`.
    pub fn mapping(&self, key: &str) -> HashMap<String, String> {
        self.get(key)
            .map(|raw| {
                raw.split(',')
                    .filter_map(|pair| {
                        let (code, id) = pair.split_once(':')?;
                        let code = code.trim();
                        let id = id.trim();
                        if code.is_empty() || id.is_empty() {
                            return None;
                        }
                        Some((code.to_uppercase(), id.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pairs: &[(&str, &str)]) -> ProviderConfig {
        ProviderConfig::new(
            "test",
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        )
    }

    #[test]
    fn a_missing_key_is_a_refusal_not_a_panic() {
        let error = cfg(&[]).require("app_id").unwrap_err();
        assert!(matches!(error, ProviderError::Config { key: "app_id", .. }));
    }

    #[test]
    fn a_mapping_is_read_as_pairs() {
        let parsed = cfg(&[("mapping", "btc:bitcoin, USDT:tether")]).mapping("mapping");

        assert_eq!(parsed.get("BTC").map(String::as_str), Some("bitcoin"));
        assert_eq!(parsed.get("USDT").map(String::as_str), Some("tether"));
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn a_malformed_mapping_entry_is_dropped_rather_than_guessed() {
        let parsed = cfg(&[("mapping", "BTC:bitcoin,garbage,:x,Y:")]).mapping("mapping");

        assert_eq!(parsed.len(), 1);
    }
}
