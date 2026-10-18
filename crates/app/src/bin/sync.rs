//! Rate synchronisation. A scheduled job, hourly.
//!
//! One pass: group the registry's currencies by adapter, ask each adapter, store what it gave,
//! and as the LAST action reset the base to one.
//!
//! THREE DECISIONS, each learned the hard way:
//!
//! 1. **The base is pinned at the end.** A provider that returned the base in its response would
//!    shift the reference point itself, and then ALL rates become wrong at once, including those
//!    already written in the same run.
//! 2. **A partial result stays in the database.** If one adapter fails, the other's currencies
//!    are still fresh. Rolling them back would hold the whole registry hostage to the least
//!    reliable vendor.
//! 3. **The run still fails.** A non-zero exit code is a visible failed job and a retry an hour
//!    later. A swallowed error would leave yesterday's numbers looking fresh.

use std::collections::BTreeMap;

use currency_provider_sdk::ProviderConfig;
use currency_providers::ProviderRegistry;

const DEFAULT_BASE: &str = "EUR";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    currency_core::logging::init("currency-sync");

    let database_url = std::env::var("DATABASE_URL")?;
    let base = std::env::var("BASE_CURRENCY")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE.to_string())
        .to_uppercase();

    let db = currency_store::connect(&database_url, 2).await?;
    let providers = ProviderRegistry::new();

    let registry = currency_store::currency::all(&db).await?;

    // Group by adapter: one vendor request for all of its currencies rather than one per currency.
    let mut by_provider: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for row in &registry {
        by_provider
            .entry(row.provider.clone())
            .or_default()
            .push(row.code.clone());
    }

    tracing::info!(
        event = "sync.started",
        base = base.as_str(),
        currencies = registry.len() as i64,
        adapters = ?providers.keys(),
        "rate sync started"
    );

    let mut written = 0i64;
    let mut failed: Vec<String> = Vec::new();

    for (key, codes) in by_provider {
        let Some(provider) = providers.get(&key) else {
            // The currency names an adapter this build does not have. A configuration defect, not
            // a run failure: silently skipping it would leave its rate stale forever.
            tracing::warn!(
                event = "sync.provider_unknown",
                provider = key.as_str(),
                currencies = ?codes,
                "registry names a provider this build does not have"
            );
            failed.push(key);
            continue;
        };

        let cfg = ProviderConfig::from_env(provider.key());

        match provider.rates(&cfg, &base, &codes).await {
            Ok(rates) => {
                for (code, rate) in &rates {
                    currency_store::currency::save_rate(&db, code, *rate).await?;
                    written += 1;
                }

                // Both halves: what the adapter knew and what it did not. Otherwise a silently
                // skipped currency looks like a currency with no change.
                let missing: Vec<&String> = codes
                    .iter()
                    .filter(|code| code.as_str() != base && !rates.contains_key(*code))
                    .collect();

                tracing::info!(
                    event = "sync.provider_done",
                    provider = provider.key(),
                    quoted = rates.len() as i64,
                    missing = ?missing,
                    "provider answered"
                );
            }
            Err(error) => {
                tracing::warn!(
                    event = "sync.provider_failed",
                    provider = provider.key(),
                    error = %error,
                    "provider failed, its currencies keep their previous rates"
                );
                failed.push(key);
            }
        }
    }

    // The LAST action. See decision 1 in the file header.
    currency_store::currency::pin_base(&db, &base).await?;

    tracing::info!(
        event = "sync.finished",
        written,
        failed = failed.len() as i64,
        "rate sync finished"
    );

    if !failed.is_empty() {
        anyhow::bail!("rate providers failed: {}", failed.join(", "));
    }

    Ok(())
}
