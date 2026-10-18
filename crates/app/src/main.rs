//! Server: gRPC with the conversions and HTTP with a single liveness probe.
//!
//! The sync is not part of it; it is a separate scheduled binary. The server must answer in
//! microseconds, while the sync calls vendors with timeouts measured in seconds and would share
//! the connection pool with it at the worst possible moment.

mod metrics;

use std::sync::Arc;
use std::time::Duration;

use currency_api::{CurrencyApi, Registry};

/// The registry lives in memory and a conversion request never reaches the database. Connections
/// are needed by startup and the background refresh: two is plenty, and any more would be taken
/// away from other tenants of the same database instance.
const DB_CONNECTIONS: u32 = 2;

/// A client connection lives for hours and sits idle between calls. Pings in the gaps keep it alive
/// through the load balancer and detect a silently dead client before the next call.
const GRPC_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const GRPC_KEEPALIVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Ceiling for a single call. It is generous by three orders of magnitude: the handler is a HashMap
/// lookup and one multiplication. Anything slower is a stuck task, not slow work.
const GRPC_CALL_TIMEOUT: Duration = Duration::from_secs(5);

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    currency_core::logging::init("currency-manager");

    let prometheus = metrics::install()?;

    let database_url = std::env::var("DATABASE_URL")?;

    // A bare port number rather than a full address: the same number can become both the container
    // port and the Kubernetes Service port, derived from a single deployment value. ADDR variables
    // do not fit here: a diverging port is a Service aimed at a closed door, and no probe will say
    // why.
    let grpc_port: u16 = std::env::var("GRPC_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9090);
    let http_port: u16 = std::env::var("HTTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8080);
    let metrics_port: u16 = std::env::var("METRICS_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(9464);

    let grpc_addr = format!("0.0.0.0:{grpc_port}");
    let http_addr = format!("0.0.0.0:{http_port}");
    let metrics_addr = format!("0.0.0.0:{metrics_port}");

    let db = currency_store::connect(&database_url, DB_CONNECTIONS).await?;
    metrics::watch_pool(db.clone());

    // The registry is populated BEFORE the port opens. An empty registry would answer
    // UNKNOWN_CURRENCY to every call while passing the probe: the instance would look healthy and
    // refuse everyone.
    let registry = Arc::new(Registry::load(&db).await?);
    currency_api::rates::spawn_refresh(registry.clone(), db);

    tracing::info!(
        event = "startup",
        grpc = grpc_addr.as_str(),
        http = http_addr.as_str(),
        metrics = metrics_addr.as_str(),
        currencies = registry.len() as i64,
        "server starting",
    );

    let grpc = tokio::spawn(
        tonic::transport::Server::builder()
            .http2_keepalive_interval(Some(GRPC_KEEPALIVE_INTERVAL))
            .http2_keepalive_timeout(Some(GRPC_KEEPALIVE_TIMEOUT))
            .timeout(GRPC_CALL_TIMEOUT)
            .layer(metrics::GrpcMetricsLayer)
            .add_service(CurrencyApi::new(registry).into_server())
            .serve_with_shutdown(grpc_addr.parse()?, shutdown()),
    );

    let listener = tokio::net::TcpListener::bind(&http_addr).await?;

    let http = tokio::spawn(async move {
        axum::serve(listener, currency_api::health::router())
            .with_graceful_shutdown(shutdown())
            .await
    });

    // A port of its own that no public ingress needs to route to. Left out of the select below: a
    // dead exporter shows up as a missing target, and taking the conversions down with it would
    // trade a blind spot for an outage. Without a graceful shutdown of its own it keeps answering
    // scrapes while the calls drain.
    tokio::spawn(metrics::serve(metrics_addr, prometheus));

    // Either of the two failing stops the process. A surviving HTTP server would answer "ok" to the
    // probe while gRPC is dead: the instance would stay in rotation, failing every caller.
    tokio::select! {
        result = grpc => result??,
        result = http => result??,
    }

    tracing::info!(event = "shutdown.complete", "server stopped");

    Ok(())
}

/// Orchestrators send SIGTERM first and only kill later. Wait for in-flight calls.
async fn shutdown() {
    let mut term =
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()).expect("SIGTERM");

    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }

    tracing::info!(
        event = "shutdown.signal",
        "shutdown signal received, draining in-flight calls"
    );
}
