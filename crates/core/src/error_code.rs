//! Machine-readable failure reason. Travels in gRPC metadata under the `x-error-code` key.
//!
//! The status code alone is not enough: "no such currency" and "the rate never synced" are both
//! `FAILED_PRECONDITION`, yet they call for different actions. The first is fixed by the seed, the
//! second by the rate provider. Telling them apart by the message text is not an option: messages
//! are written by people and change without notice.
//!
//! The vocabulary is shared and closed. The metadata key follows a common convention, so clients
//! talking to several gRPC services only need to read one key.

/// Metadata key carrying the error code.
pub const ERROR_CODE_KEY: &str = "x-error-code";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    /// The currency code is not in the registry. Fixed by the seed or a new migration, not by a retry.
    UnknownCurrency,
    /// The currency exists but has never had a rate. Differs from a stale rate: a stale rate is
    /// still SERVED, here there is nothing to serve.
    RateMissing,
    /// The amount is not a number, is infinite, or the result does not fit in int64.
    InvalidAmount,
    /// Everything else.
    Internal,
}

impl ErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UnknownCurrency => "UNKNOWN_CURRENCY",
            Self::RateMissing => "RATE_MISSING",
            Self::InvalidAmount => "INVALID_AMOUNT",
            Self::Internal => "INTERNAL",
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
