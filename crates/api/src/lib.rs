//! Two doors: gRPC with the conversions and HTTP with a single liveness probe.

pub mod grpc;
pub mod health;
pub mod rates;

pub mod pb {
    tonic::include_proto!("currency.v1");
}

pub use grpc::CurrencyApi;
pub use rates::Registry;
