//! The rate-source contract. Everything an adapter needs and nothing that knows about the database.
//!
//! The crate DELIBERATELY does not depend on `currency-store`: an adapter must have no physical
//! way to touch the table. It does one thing: ask the vendor for numbers and return them.

pub mod config;
pub mod error;
pub mod kit;

pub use config::ProviderConfig;
pub use error::ProviderError;

use std::collections::HashMap;

use async_trait::async_trait;
use rust_decimal::Decimal;

/// A source of exchange rates.
///
/// Exactly one method: ask for rates. Everything else (which adapter this is and how it is
/// configured) comes from outside, so one instance serves any number of calls and is built once
/// at process start.
#[async_trait]
pub trait RateProvider: Send + Sync + 'static {
    /// The adapter key. Matches the `currency.provider` column: the sync uses it to decide whom to
    /// ask about a currency.
    ///
    /// `&'static str` rather than a `String` from a database row: the error type carries it
    /// around, and an error has no reason to own text from the database.
    fn key(&self) -> &'static str;

    /// How many units of each currency one `base` buys.
    ///
    /// THREE RULES, learned the hard way and mandatory here:
    ///
    /// 1. **An unknown code is omitted, not guessed.** Returning `1` for a code the vendor does
    ///    not know silently equates the currency with the base; the error would surface in a
    ///    user's balance.
    /// 2. **The base currency is not returned.** The caller pins it to one as the last write;
    ///    otherwise a vendor echo would one day shift the reference point itself.
    /// 3. **Errors propagate.** The sync runs on a schedule: a failed run is retried an hour later,
    ///    while a swallowed error leaves yesterday's numbers looking fresh.
    async fn rates(
        &self,
        cfg: &ProviderConfig,
        base: &str,
        codes: &[String],
    ) -> Result<HashMap<String, Decimal>, ProviderError>;
}
