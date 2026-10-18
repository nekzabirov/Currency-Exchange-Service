//! The only place where rate adapters are named.
//!
//! Nowhere else: neither the sync, nor the server, nor the database knows how many providers
//! exist. The sync asks the registry by the key from the `currency.provider` column, so a new
//! source arrives as one crate plus one line here, with zero changes to anyone else's code.

use std::collections::HashMap;
use std::sync::Arc;

use currency_provider_sdk::RateProvider;

pub struct ProviderRegistry {
    by_key: HashMap<&'static str, Arc<dyn RateProvider>>,
}

impl ProviderRegistry {
    #[allow(clippy::vec_init_then_push)]
    pub fn new() -> Self {
        let mut all: Vec<Box<dyn RateProvider>> = Vec::new();

        #[cfg(feature = "openexchange")]
        all.push(currency_provider_openexchange::provider());

        #[cfg(feature = "coingecko")]
        all.push(currency_provider_coingecko::provider());

        #[cfg(feature = "coinbase")]
        all.push(currency_provider_coinbase::provider());

        Self {
            by_key: all
                .into_iter()
                .map(|p| {
                    let key = p.key();
                    (key, Arc::from(p))
                })
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<Arc<dyn RateProvider>> {
        self.by_key.get(key).cloned()
    }

    /// Keys of all enabled adapters, sorted. Logged when the sync starts: otherwise "the rate was
    /// not updated" and "the adapter was built without its feature" look the same.
    pub fn keys(&self) -> Vec<&'static str> {
        let mut keys: Vec<_> = self.by_key.keys().copied().collect();
        keys.sort_unstable();
        keys
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two adapters with the same key would silently overwrite each other in the HashMap: the built
    /// image would run, while half of the currencies stopped updating.
    #[test]
    fn keys_are_unique() {
        assert_eq!(ProviderRegistry::new().keys().len(), 3);
    }

    #[test]
    fn every_provider_is_reachable_by_its_key() {
        let registry = ProviderRegistry::new();

        assert!(registry.get("openexchange").is_some());
        assert!(registry.get("coingecko").is_some());
        assert!(registry.get("coinbase").is_some());
        assert!(registry.get("nope").is_none());
    }
}
