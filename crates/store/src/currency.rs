//! The `currency` table: the service's whole registry.
//!
//! A handful of rows today, dozens some day. It is read in full: nobody needs a lookup by a single
//! code, because both the sync and the server work with the whole registry at once.

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::Db;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Currency {
    pub code: String,
    /// `FIAT` | `CRYPTO`. A descriptive field: no decisions are made on it; it explains to a human
    /// why this particular adapter manages the currency.
    pub kind: String,
    /// Key of the adapter that manages it. Matches `RateProvider::key`.
    pub provider: String,
    /// 1 human-readable unit = this many storage units.
    pub units_scale: i64,
    /// Units of this currency per one base unit.
    pub rate: Decimal,
    /// When the rate last arrived. `NULL` means it never has.
    pub rate_updated_at: Option<DateTime<Utc>>,
}

/// The whole registry, ordered by code.
pub async fn all(db: &Db) -> Result<Vec<Currency>, sqlx::Error> {
    // Columns are listed by name rather than `SELECT *`. `*` in a prepared statement binds the plan
    // to the set of columns at preparation time: a new column added by a migration breaks running
    // instances with "cached plan must not change result type" until they are restarted.
    sqlx::query_as::<_, Currency>(
        "SELECT code, kind, provider, units_scale, rate, rate_updated_at \
         FROM currency ORDER BY code",
    )
    .fetch_all(db)
    .await
}

/// Creates the currency if it does not exist yet.
///
/// INSERT ONLY. The seed must never overwrite a synced rate: the seed job runs on every deploy,
/// and an updating upsert would reset the registry to ones on every release.
pub async fn seed(
    db: &Db,
    code: &str,
    kind: &str,
    provider: &str,
    units_scale: i64,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "INSERT INTO currency (code, kind, provider, units_scale) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (code) DO NOTHING",
    )
    .bind(code)
    .bind(kind)
    .bind(provider)
    .bind(units_scale)
    .execute(db)
    .await?;

    Ok(result.rows_affected() > 0)
}

/// Stores a fresh rate.
///
/// The base currency never comes through here: [`pin_base`] pins it as the run's last write.
pub async fn save_rate(db: &Db, code: &str, rate: Decimal) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE currency SET rate = $2, rate_updated_at = now(), updated_at = now() \
         WHERE code = $1",
    )
    .bind(code)
    .bind(rate)
    .execute(db)
    .await?;

    Ok(())
}

/// Resets the base currency to one, as the run's last action.
///
/// Not paranoia: a provider that returns the base in its response (some do) would shift the
/// reference point itself, and then ALL rates become wrong at once, including those already stored.
pub async fn pin_base(db: &Db, base: &str) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE currency SET rate = 1, rate_updated_at = now(), updated_at = now() \
         WHERE code = $1",
    )
    .bind(base)
    .execute(db)
    .await?;

    Ok(())
}
