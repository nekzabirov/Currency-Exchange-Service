//! In-process registry.
//!
//! A CONVERSION REQUEST NEVER REACHES THE DATABASE. This is not premature optimisation: the service
//! sits on the hot path of money movements (every transaction may pass through a scale
//! conversion), and a Postgres query on each such call would add both latency and a new point of
//! failure on top of the money.
//!
//! The data is a handful of rows that change once an hour. A background task re-reads the whole
//! table and swaps the snapshot with a single pointer; readers never block, not even for a moment.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use arc_swap::ArcSwap;
use chrono::{DateTime, Utc};
use currency_core::Rate;
use currency_store::{currency, Db};

/// How often the table is re-read. Rates are moved by an hourly sync, so a minute is "practically
/// immediately" and at the same time only 60 queries per hour for the whole service.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// The age after which a rate counts as stale.
///
/// A stale rate is STILL SERVED (refusing would fail a payment because somebody else's API is
/// down), but every such answer leaves a WARN with the age. Silently computing with yesterday's
/// rate means a day-long investigation: everything "works" but the numbers are wrong.
pub const STALE_AFTER: chrono::Duration = chrono::Duration::hours(24);

const REFRESH_METRIC: &str = "currency_rates_refresh_total";

const OUTCOME: &str = "outcome";

#[derive(Debug, Clone)]
pub struct Entry {
    pub rate: Rate,
    pub updated_at: Option<DateTime<Utc>>,
}

/// Registry snapshot. Replaced as a whole, never edited in place.
#[derive(Default)]
pub struct Registry {
    snapshot: ArcSwap<HashMap<String, Entry>>,
}

impl Registry {
    /// Reads the table and returns an already populated registry.
    ///
    /// Deliberately so, rather than "empty, filled in later": an empty registry answers
    /// `UNKNOWN_CURRENCY` to every call, yet an instance holding it would pass the probe and start
    /// taking traffic. An error at startup is an instance that did not come up, which is visible.
    pub async fn load(db: &Db) -> Result<Self, sqlx::Error> {
        let registry = Self::default();
        let loaded = registry.refresh(db).await?;

        if loaded == 0 {
            tracing::warn!(
                event = "registry.empty",
                "currency table is empty — did the seed job run?"
            );
        }

        Ok(registry)
    }

    /// Re-reads the table and swaps the snapshot. Returns the number of rows.
    pub async fn refresh(&self, db: &Db) -> Result<usize, sqlx::Error> {
        let rows = currency::all(db).await?;

        let snapshot: HashMap<String, Entry> = rows
            .into_iter()
            .map(|row| {
                (
                    row.code.clone(),
                    Entry {
                        rate: Rate {
                            units_scale: row.units_scale,
                            rate: row.rate,
                        },
                        updated_at: row.rate_updated_at,
                    },
                )
            })
            .collect();

        let count = snapshot.len();
        self.snapshot.store(Arc::new(snapshot));

        Ok(count)
    }

    /// The code is normalised here and only here: registry codes are upper case, while callers
    /// send them in whatever form they are used to.
    pub fn get(&self, code: &str) -> Option<Entry> {
        self.snapshot
            .load()
            .get(&code.trim().to_uppercase())
            .cloned()
    }

    pub fn len(&self) -> usize {
        self.snapshot.load().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Entry {
    /// Age of the rate, if it ever arrived.
    pub fn age(&self, now: DateTime<Utc>) -> Option<chrono::Duration> {
        self.updated_at.map(|at| now.signed_duration_since(at))
    }

    pub fn is_stale(&self, now: DateTime<Utc>) -> bool {
        self.age(now).map(|age| age > STALE_AFTER).unwrap_or(false)
    }
}

/// Background refresh. A separate task rather than a timer inside the handler: the handler must
/// answer in microseconds and may not wait for the database.
///
/// A read error does NOT crash the process or clear the snapshot: an unreachable database is no
/// reason to stop converting with the numbers already in memory.
pub fn spawn_refresh(registry: Arc<Registry>, db: Db) {
    // Both outcomes exist from the first scrape at zero: a series born on its first failure would
    // hide that very failure from `increase()`, which needs a sample before the jump.
    let refreshed = metrics::counter!(REFRESH_METRIC, OUTCOME => "ok");
    let failed = metrics::counter!(REFRESH_METRIC, OUTCOME => "failed");

    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        ticker.tick().await; // fires at once; the snapshot was loaded at startup

        loop {
            ticker.tick().await;

            match registry.refresh(&db).await {
                Ok(count) => {
                    refreshed.increment(1);
                    tracing::debug!(
                        event = "registry.refreshed",
                        currencies = count as i64,
                        "registry refreshed"
                    )
                }
                Err(error) => {
                    failed.increment(1);
                    tracing::warn!(
                        event = "registry.refresh_failed",
                        error = %error,
                        "registry refresh failed, serving the previous snapshot"
                    )
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    use rust_decimal::Decimal;

    fn entry(updated_at: Option<DateTime<Utc>>) -> Entry {
        Entry {
            rate: Rate {
                units_scale: 1_000_000_000,
                rate: Decimal::ONE,
            },
            updated_at,
        }
    }

    #[test]
    fn a_fresh_rate_is_not_stale() {
        let now = Utc::now();
        assert!(!entry(Some(now - chrono::Duration::hours(1))).is_stale(now));
    }

    #[test]
    fn a_rate_older_than_a_day_is_stale() {
        let now = Utc::now();
        assert!(entry(Some(now - chrono::Duration::hours(25))).is_stale(now));
    }

    /// A rate that NEVER arrived is not stale but missing: it has a different refusal code and a
    /// different person fixes it.
    #[test]
    fn a_rate_that_never_arrived_is_not_stale_it_is_missing() {
        let now = Utc::now();
        let never = entry(None);

        assert!(!never.is_stale(now));
        assert_eq!(never.age(now), None);
    }

    #[test]
    fn a_code_is_matched_case_insensitively() {
        let registry = Registry::default();
        registry
            .snapshot
            .store(Arc::new(HashMap::from([("UAH".to_string(), entry(None))])));

        assert!(registry.get("uah").is_some());
        assert!(registry.get(" UAH ").is_some());
        assert!(registry.get("EUR").is_none());
    }
}
