use tower_layer::Layer;

use super::grpc_metrics::GrpcMetrics;

/// Times every call on the gRPC port. Goes on the `Server` before the services, so it sees all of
/// them, the paths no service answers included.
#[derive(Debug, Clone, Copy, Default)]
pub struct GrpcMetricsLayer;

impl<S> Layer<S> for GrpcMetricsLayer {
    type Service = GrpcMetrics<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GrpcMetrics::new(inner)
    }
}
