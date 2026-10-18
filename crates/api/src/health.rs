//! HTTP exists only for the probe.
//!
//! A separate port rather than a gRPC probe: a kubelet probe with `kind: grpc` treats
//! `UNIMPLEMENTED` as a failure, so a pod without a gRPC health service never becomes ready. One
//! path, one answer.
//!
//! `/__health` rather than `/health`: a namespaced path that is unlikely to collide with routes of
//! other services behind the same load balancer, so one health-check rule can cover them all.

use axum::routing::get;
use axum::Router;

pub fn router() -> Router {
    Router::new().route("/__health", get(|| async { "ok" }))
}
