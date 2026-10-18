//! Populates the currency registry.
//!
//! INSERT ONLY. The job runs on every deploy, and an updating upsert would reset rates to one on
//! every release, silently breaking money exactly when everyone is looking at a green deploy.
//!
//! ONE SCALE FOR ALL: 1e9. Balances are stored in nano units for UAH and BTC alike; the scale does
//! not depend on the currency. The column is per currency so that this can one day be changed by a
//! database row rather than a release of every consuming service; for now the values are equal.

use currency_providers::ProviderRegistry;

/// 1 human-readable unit = this many storage units.
const NANO: i64 = 1_000_000_000;

/// `code, kind, adapter`. The base currency comes first, and it is EUR: every rate is relative to it.
const CURRENCIES: &[(&str, &str, &str)] = &[
    ("EUR", "FIAT", "openexchange"),
    ("UAH", "FIAT", "openexchange"),
    ("USDT", "CRYPTO", "coinbase"),
    ("BTC", "CRYPTO", "coinbase"),
    ("USDC", "CRYPTO", "coinbase"),
];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    currency_core::logging::init("currency-seed");

    let database_url = std::env::var("DATABASE_URL")?;
    let db = currency_store::connect(&database_url, 1).await?;

    // The adapter key in the table is whom the sync will ask for the rate. A typo here breaks
    // nothing visible: the row is stored, the seed reports success, and the currency simply never
    // updates. So the adapter registry is checked BEFORE the first insert.
    let providers = ProviderRegistry::new();
    for (code, _, provider) in CURRENCIES {
        if providers.get(provider).is_none() {
            anyhow::bail!(
                "currency {code} names provider `{provider}`, which is not registered (known: {:?})",
                providers.keys()
            );
        }
    }

    tracing::info!(
        event = "seed.started",
        currencies = CURRENCIES.len() as i64,
        "currency seed started"
    );

    let mut inserted = 0i64;

    for (code, kind, provider) in CURRENCIES {
        if currency_store::currency::seed(&db, code, kind, provider, NANO).await? {
            inserted += 1;
            tracing::info!(
                event = "seed.inserted",
                currency = *code,
                provider = *provider,
                "currency added to the registry"
            );
        }
    }

    tracing::info!(
        event = "seed.finished",
        inserted,
        skipped = CURRENCIES.len() as i64 - inserted,
        "currency seed finished"
    );

    Ok(())
}
