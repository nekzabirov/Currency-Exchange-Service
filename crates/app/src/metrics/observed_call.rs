//! One gRPC call on its way through the server, recorded exactly once: when it is dropped.
//!
//! On drop, not on completion, because the metrics layer sits INSIDE tonic's own timeout. A call
//! that the timeout or a departed client cuts off never comes back through the layer; its future
//! is simply dropped, and it still has to be counted.

use std::mem;
use std::time::Instant;

use tonic::codegen::http::HeaderMap;
use tonic::Code;

const DURATION: &str = "grpc_server_processing_duration_seconds";

const SERVICE: &str = "service";

const METHOD: &str = "method";

const STATUS_CODE: &str = "statusCode";

const GRPC_STATUS: &str = "grpc-status";

/// The label for a path this server does not serve: naming it would let any client mint a series
/// per path. Load-balancer health checks commonly probe one, `grpc.health.v1.Health`, which is
/// answered UNIMPLEMENTED.
const UNSERVED: &str = "unknown";

pub(super) struct ObservedCall {
    service: String,

    method: String,

    started: Instant,

    /// What the caller sees when the call is dropped before its response exists: tonic answers its
    /// own timeout with CANCELLED, and a client that went away cancelled it itself.
    code: Code,
}

impl ObservedCall {
    pub(super) fn start(path: &str) -> Self {
        let (service, method) = path
            .strip_prefix('/')
            .and_then(|path| path.split_once('/'))
            .unwrap_or((UNSERVED, UNSERVED));

        Self {
            service: service.to_owned(),
            method: method.to_owned(),
            started: Instant::now(),
            code: Code::Cancelled,
        }
    }

    /// Every rpc here is unary. A refusal is trailers-only, so its `grpc-status` is in the
    /// headers; a success carries it in the trailers, so a header without it is OK.
    pub(super) fn respond(&mut self, headers: &HeaderMap) {
        self.code = headers
            .get(GRPC_STATUS)
            .map(|status| Code::from_bytes(status.as_bytes()))
            .unwrap_or(Code::Ok);
    }

    /// The router itself failed, which tonic reports to the caller as UNKNOWN.
    pub(super) fn fail(&mut self) {
        self.code = Code::Unknown;
    }
}

impl Drop for ObservedCall {
    fn drop(&mut self) {
        let (service, method) = if self.code == Code::Unimplemented {
            (UNSERVED.to_owned(), UNSERVED.to_owned())
        } else {
            (mem::take(&mut self.service), mem::take(&mut self.method))
        };

        metrics::histogram!(
            DURATION,
            SERVICE => service,
            METHOD => method,
            STATUS_CODE => name(self.code),
        )
        .record(self.started.elapsed());
    }
}

/// The name grpc-java gives the code, which is what Micrometer puts on JVM services' series.
fn name(code: Code) -> &'static str {
    match code {
        Code::Ok => "OK",
        Code::Cancelled => "CANCELLED",
        Code::Unknown => "UNKNOWN",
        Code::InvalidArgument => "INVALID_ARGUMENT",
        Code::DeadlineExceeded => "DEADLINE_EXCEEDED",
        Code::NotFound => "NOT_FOUND",
        Code::AlreadyExists => "ALREADY_EXISTS",
        Code::PermissionDenied => "PERMISSION_DENIED",
        Code::ResourceExhausted => "RESOURCE_EXHAUSTED",
        Code::FailedPrecondition => "FAILED_PRECONDITION",
        Code::Aborted => "ABORTED",
        Code::OutOfRange => "OUT_OF_RANGE",
        Code::Unimplemented => "UNIMPLEMENTED",
        Code::Internal => "INTERNAL",
        Code::Unavailable => "UNAVAILABLE",
        Code::DataLoss => "DATA_LOSS",
        Code::Unauthenticated => "UNAUTHENTICATED",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusRecorder};
    use tonic::codegen::http::HeaderValue;

    const CONVERT: &str = "/currency.v1.CurrencyService/ConvertToUnits";

    fn recorder() -> PrometheusRecorder {
        PrometheusBuilder::new()
            .set_buckets(&[1.0])
            .expect("buckets")
            .build_recorder()
    }

    fn observe(call: impl FnOnce() -> ObservedCall) -> String {
        let recorder = recorder();
        metrics::with_local_recorder(&recorder, || drop(call()));
        recorder.handle().render()
    }

    fn refused(code: &'static str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(GRPC_STATUS, HeaderValue::from_static(code));
        headers
    }

    #[test]
    fn a_success_is_labelled_by_service_method_and_ok() {
        let exposition = observe(|| {
            let mut call = ObservedCall::start(CONVERT);
            call.respond(&HeaderMap::new());
            call
        });

        assert!(
            exposition.contains(
                "grpc_server_processing_duration_seconds_count{service=\"currency.v1.CurrencyService\",method=\"ConvertToUnits\",statusCode=\"OK\"} 1"
            ),
            "{exposition}"
        );
    }

    #[test]
    fn a_refusal_carries_the_name_the_jvm_engines_use() {
        let exposition = observe(|| {
            let mut call = ObservedCall::start(CONVERT);
            call.respond(&refused("9"));
            call
        });

        assert!(
            exposition.contains("statusCode=\"FAILED_PRECONDITION\""),
            "{exposition}"
        );
    }

    #[test]
    fn a_call_dropped_before_its_response_is_cancelled() {
        let exposition = observe(|| ObservedCall::start(CONVERT));

        assert!(
            exposition.contains("method=\"ConvertToUnits\",statusCode=\"CANCELLED\""),
            "{exposition}"
        );
    }

    #[test]
    fn a_failed_router_is_unknown() {
        let exposition = observe(|| {
            let mut call = ObservedCall::start(CONVERT);
            call.fail();
            call
        });

        assert!(
            exposition.contains("statusCode=\"UNKNOWN\""),
            "{exposition}"
        );
    }

    #[test]
    fn an_unserved_path_does_not_become_a_label() {
        let exposition = observe(|| {
            let mut call = ObservedCall::start("/grpc.health.v1.Health/Check");
            call.respond(&refused("12"));
            call
        });

        assert!(!exposition.contains("grpc.health"), "{exposition}");
        assert!(
            exposition
                .contains("{service=\"unknown\",method=\"unknown\",statusCode=\"UNIMPLEMENTED\"}"),
            "{exposition}"
        );
    }

    #[test]
    fn a_path_without_a_method_is_unknown() {
        assert_eq!(ObservedCall::start("/").service, UNSERVED);
        assert_eq!(ObservedCall::start("").method, UNSERVED);
    }
}
