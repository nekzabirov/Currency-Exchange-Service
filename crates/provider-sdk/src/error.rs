//! Adapter failure. Three cases, because each calls for a different reaction.

/// The text is in plain English because it does not only go to the log: the sync reports with the
/// same string, and whoever investigates a failed scheduled run reads it.
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    /// The vendor answered and refused: wrong key, wrong plan, unknown base.
    /// A retry gives the same result.
    #[error("{provider} refused the call: {message}")]
    Vendor {
        provider: &'static str,
        message: String,
    },

    /// The vendor was never reached: timeout, DNS, broken TLS. A retry makes sense.
    #[error("{provider} unreachable: {message}")]
    Transport {
        provider: &'static str,
        message: String,
    },

    /// The adapter is enabled but not fully configured. Not a run failure but an unfilled secret.
    #[error("{provider} config is missing `{key}`")]
    Config {
        provider: &'static str,
        key: &'static str,
    },
}

impl ProviderError {
    pub fn provider(&self) -> &'static str {
        match self {
            Self::Vendor { provider, .. }
            | Self::Transport { provider, .. }
            | Self::Config { provider, .. } => provider,
        }
    }
}
