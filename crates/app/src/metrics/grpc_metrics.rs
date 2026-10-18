use tonic::codegen::http::{Request, Response};
use tonic::codegen::{BoxFuture, Context, Poll, Service};

use super::observed_call::ObservedCall;

#[derive(Debug, Clone)]
pub struct GrpcMetrics<S> {
    inner: S,
}

impl<S> GrpcMetrics<S> {
    pub(super) fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, B, R> Service<Request<B>> for GrpcMetrics<S>
where
    S: Service<Request<B>, Response = Response<R>>,
    S::Future: Send + 'static,
{
    type Response = S::Response;

    type Error = S::Error;

    type Future = BoxFuture<S::Response, S::Error>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<B>) -> Self::Future {
        let mut call = ObservedCall::start(request.uri().path());
        let response = self.inner.call(request);

        Box::pin(async move {
            let result = response.await;

            match &result {
                Ok(response) => call.respond(response.headers()),
                Err(_) => call.fail(),
            }

            result
        })
    }
}
