//! Postgres. One table, one module.

pub mod currency;

pub type Db = sqlx::PgPool;

/// Opens the pool.
///
/// Migrations are NOT run here: that is a separate job (`migrate`). An application that migrates
/// itself cannot be rolled back (the old image would already face the new schema), and replicas
/// would apply it out of step.
///
/// The pool is small on purpose: a conversion request NEVER reaches the database. The registry
/// lives in process memory and is refreshed by a single background task. Connections are needed by
/// that task and by startup, not by calls.
pub async fn connect(url: &str, max_connections: u32) -> Result<Db, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(max_connections)
        .min_connections(MIN_CONNECTIONS.min(max_connections))
        .acquire_timeout(ACQUIRE_TIMEOUT)
        .idle_timeout(IDLE_TIMEOUT)
        .max_lifetime(MAX_LIFETIME)
        // By default sqlx PINGS a connection before handing it out: two round trips per query. A
        // dead connection fails on first use anyway.
        .test_before_acquire(false)
        .connect(url)
        .await
}

const MIN_CONNECTIONS: u32 = 1;
const ACQUIRE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
const IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const MAX_LIFETIME: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// Runs the migrations. Called ONLY by the `migrate` job.
///
/// Bookkeeping is done by `sqlx::migrate`: a version per file, a checksum, a table of applied
/// migrations, an advisory lock for the duration of the run. A file edited after the fact fails
/// the run instead of being applied silently.
pub async fn migrate(db: &Db) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("../../migrations").run(db).await
}
