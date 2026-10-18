//! Prometheus exposition, pulled by a scraper from `/metrics` on a port of its own.
//!
//! Names, labels and buckets follow the conventions of Micrometer-instrumented JVM services, which
//! render the same `grpc_server_processing_duration_seconds`, so one dashboard can read services
//! written in either stack.
//!
//! Nothing here is on the conversion path. A sample that cannot be recorded is lost, and a metrics
//! listener that dies costs scrapes, never calls.

mod grpc_metrics;
mod grpc_metrics_layer;
mod observed_call;

use std::time::Duration;

use axum::routing::get;
use axum::Router;
use currency_store::Db;
use metrics_exporter_prometheus::{BuildError, PrometheusBuilder, PrometheusHandle};
use tonic::codegen::http::header::CONTENT_TYPE;

pub use grpc_metrics_layer::GrpcMetricsLayer;

/// A fixed bucket set: one `le` set everywhere, so one query aggregates across services.
/// Set for every histogram, because one left without buckets is rendered as a summary, and
/// summaries cannot be summed across pods.
const BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];

/// The exporter keeps raw histogram samples until something drains them, and without its own
/// listener the draining is ours. The process and pool gauges are sampled on the same beat.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);

const PATH: &str = "/metrics";

const TEXT_FORMAT: &str = "text/plain; version=0.0.4; charset=utf-8";

const POOL_ACTIVE: &str = "db_pool_connections_active";

const POOL_IDLE: &str = "db_pool_connections_idle";

const POOL_MAX: &str = "db_pool_connections_max";

/// The global recorder, with its upkeep and the process gauges on their own task. Once, before
/// anything records.
pub fn install() -> anyhow::Result<PrometheusHandle> {
    let handle = builder()?.install_recorder()?;

    let process = metrics_process::Collector::default();
    process.describe();

    let upkeep = handle.clone();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            tick.tick().await;
            process.collect();
            upkeep.run_upkeep();
        }
    });

    Ok(handle)
}

/// Conversions never touch the database: the pool serves the startup load and the registry
/// refresh. All of it taken is the first sign that the refresh is stuck on the database, long
/// before a snapshot goes stale.
pub fn watch_pool(db: Db) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(SAMPLE_INTERVAL);
        loop {
            tick.tick().await;
            record_pool(&db);
        }
    });
}

/// Spawned apart from both doors and awaited by neither: a port that cannot bind costs scrapes, not
/// conversions.
pub async fn serve(addr: String, handle: PrometheusHandle) {
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(
                event = "metrics.failed",
                addr = addr.as_str(),
                error = %error,
                "metrics listener not started"
            );
            return;
        }
    };

    if let Err(error) = axum::serve(listener, router(handle)).await {
        tracing::error!(
            event = "metrics.failed",
            addr = addr.as_str(),
            error = %error,
            "metrics listener stopped"
        );
    }
}

fn router(handle: PrometheusHandle) -> Router {
    Router::new().route(
        PATH,
        get(move || {
            let exposition = handle.render();
            async move { ([(CONTENT_TYPE, TEXT_FORMAT)], exposition) }
        }),
    )
}

fn builder() -> Result<PrometheusBuilder, BuildError> {
    PrometheusBuilder::new().set_buckets(BUCKETS)
}

fn record_pool(db: &Db) {
    let open = f64::from(db.size());
    let idle = db.num_idle() as f64;

    // `size` and `num_idle` are two separate reads, and a connection released in between would
    // otherwise show up as a negative number in use.
    metrics::gauge!(POOL_ACTIVE).set((open - idle).max(0.0));
    metrics::gauge!(POOL_IDLE).set(idle);
    metrics::gauge!(POOL_MAX).set(db.options().get_max_connections());
}

#[cfg(test)]
mod tests {
    use super::*;

    use metrics_exporter_prometheus::PrometheusRecorder;

    fn recorder() -> PrometheusRecorder {
        builder().expect("fleet buckets").build_recorder()
    }

    #[test]
    fn a_histogram_renders_with_the_fleet_buckets_not_as_a_summary() {
        let recorder = recorder();

        metrics::with_local_recorder(&recorder, || {
            metrics::histogram!("grpc_server_processing_duration_seconds").record(0.2);
        });

        let exposition = recorder.handle().render();

        for le in [
            "0.005", "0.01", "0.025", "0.05", "0.1", "0.25", "0.5", "1", "2.5", "5", "10", "30",
        ] {
            assert!(
                exposition.contains(&format!(
                    "grpc_server_processing_duration_seconds_bucket{{le=\"{le}\"}}"
                )),
                "no le={le} bucket in:\n{exposition}"
            );
        }
        assert!(!exposition.contains("quantile"), "{exposition}");
    }

    #[tokio::test]
    async fn the_pool_is_reported_as_active_idle_and_max() {
        // Lazy: nothing connects, which is exactly an idle pool of zero with its ceiling set.
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(2)
            .connect_lazy("postgres://currency@localhost/currency")
            .expect("lazy pool");
        let recorder = recorder();

        metrics::with_local_recorder(&recorder, || record_pool(&db));

        let exposition = recorder.handle().render();

        assert!(
            exposition.contains("db_pool_connections_active 0\n"),
            "{exposition}"
        );
        assert!(
            exposition.contains("db_pool_connections_idle 0\n"),
            "{exposition}"
        );
        assert!(
            exposition.contains("db_pool_connections_max 2\n"),
            "{exposition}"
        );
    }
}
