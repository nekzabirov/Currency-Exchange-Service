//! Schema migration. A separate job, not part of server startup.
//!
//! It must finish BEFORE the rollout: an application that migrates itself cannot be rolled back
//! (the old image would already face the new schema), and replicas would apply it out of step,
//! each at its own moment.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    currency_core::logging::init("currency-migrate");

    let database_url = std::env::var("DATABASE_URL")?;

    // One connection is enough: migrations run sequentially under a shared advisory lock.
    let db = currency_store::connect(&database_url, 1).await?;

    tracing::info!(event = "migrate.started", "schema migration started");

    currency_store::migrate(&db).await?;

    tracing::info!(event = "migrate.finished", "schema migration finished");

    Ok(())
}
