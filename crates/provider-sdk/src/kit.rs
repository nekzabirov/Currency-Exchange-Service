//! What every adapter needs and nobody should write twice.

use std::time::Duration;

use serde::de::DeserializeOwned;

use crate::ProviderError;

/// The sync is a background job with a budget of 600 seconds for the whole run. Waiting for a
/// single vendor longer than fifteen seconds is pointless: by then it has either answered or will
/// not answer on a retry either.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// `reqwest` sends NO User-Agent at all, and some vendor WAFs block unknown agents: such a client
/// gets an HTML challenge under HTTP 403 instead of JSON. This value is a common, widely accepted
/// client string.
pub const USER_AGENT: &str = "Ktor client";

/// One client per adapter, built at startup and reused: a client per call means a new connection
/// pool and a new TLS handshake every time.
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

pub fn transport(provider: &'static str, message: impl std::fmt::Display) -> ProviderError {
    ProviderError::Transport {
        provider,
        message: redact(&message.to_string()),
    }
}

pub fn vendor(provider: &'static str, message: impl std::fmt::Display) -> ProviderError {
    ProviderError::Vendor {
        provider,
        message: redact(&message.to_string()),
    }
}

/// Strips the query string from URLs inside an error message.
///
/// OpenExchangeRates authenticates with an `app_id` query parameter, and `reqwest` puts the WHOLE
/// URL into its error, key included. The error goes to the log, the log goes to storage, and the
/// key ends up somewhere it can no longer be removed from.
///
/// The whole query is cut, not just known names: every vendor has its own parameter list, and a
/// missed name does not announce itself. The path stays, since that is how the call is identified.
pub fn redact(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;

    while let Some(scheme) = rest.find("://") {
        let after_scheme = scheme + 3;

        // The query ends where the URL itself ends inside the surrounding text.
        let tail = &rest[after_scheme..];
        let url_end = tail
            .find(|c: char| c.is_whitespace() || matches!(c, ')' | '"' | '\'' | ',' | '>'))
            .map(|i| after_scheme + i)
            .unwrap_or(rest.len());

        match rest[after_scheme..url_end].find('?') {
            Some(query) => {
                out.push_str(&rest[..after_scheme + query]);
                out.push_str("?…");
                rest = &rest[url_end..];
            }
            None => {
                out.push_str(&rest[..url_end]);
                rest = &rest[url_end..];
            }
        }
    }

    out.push_str(rest);
    out
}

/// Send a request to a vendor and leave a log line about it.
///
/// The single point through which the service reaches the outside world. The line holds: adapter,
/// method, HOST and PATH (without the query, which carries the key, see [`redact`]), HTTP status and
/// duration measured at our boundary. No body: the status answers "did we get through and what
/// did they say".
pub async fn send(
    provider: &'static str,
    request: reqwest::RequestBuilder,
) -> Result<reqwest::Response, ProviderError> {
    let started = std::time::Instant::now();

    let (method, endpoint) = match request.try_clone().and_then(|r| r.build().ok()) {
        Some(built) => (
            built.method().to_string(),
            format!(
                "{}{}",
                built.url().host_str().unwrap_or("?"),
                built.url().path()
            ),
        ),
        None => ("?".to_string(), "?".to_string()),
    };

    let outcome = request.send().await;
    let duration_ms = started.elapsed().as_millis() as u64;

    match outcome {
        Ok(response) => {
            let status = response.status().as_u16();
            if response.status().is_success() {
                tracing::debug!(
                    event = "vendor.call",
                    provider,
                    method = %method,
                    endpoint = %endpoint,
                    status,
                    durationMs = duration_ms,
                    "vendor responded"
                );
            } else {
                tracing::warn!(
                    event = "vendor.call",
                    provider,
                    method = %method,
                    endpoint = %endpoint,
                    status,
                    durationMs = duration_ms,
                    "vendor responded with an error status"
                );
            }
            Ok(response)
        }
        Err(error) => {
            // Timeouts, DNS failures and broken TLS land here: things the vendor never saw.
            tracing::warn!(
                event = "vendor.call",
                provider,
                method = %method,
                endpoint = %endpoint,
                durationMs = duration_ms,
                error = %redact(&error.to_string()),
                "request never reached the vendor"
            );
            Err(transport(provider, error))
        }
    }
}

/// Response decoding.
///
/// The body is read as text BEFORE parsing: a vendor under load answers with an HTML error page,
/// and a bare JSON decoder error would hide what it said.
pub async fn decode<T: DeserializeOwned>(
    provider: &'static str,
    response: reqwest::Response,
) -> Result<T, ProviderError> {
    let status = response.status();
    let body = response.text().await.map_err(|e| transport(provider, e))?;

    if !status.is_success() {
        return Err(vendor(
            provider,
            format!("HTTP {status}: {}", truncate(&body)),
        ));
    }

    serde_json::from_str(&body).map_err(|e| {
        vendor(
            provider,
            format!("unreadable response: {e}: {}", truncate(&body)),
        )
    })
}

pub fn truncate(body: &str) -> String {
    const LIMIT: usize = 300;

    if body.len() <= LIMIT {
        return body.to_string();
    }

    let mut end = LIMIT;
    while !body.is_char_boundary(end) {
        end -= 1;
    }

    format!("{}…", &body[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_in_the_query_never_reaches_the_log() {
        let leaked = "error sending request for url (https://openexchangerates.org/api/latest.json?app_id=SECRET)";

        let cleaned = redact(leaked);

        assert!(!cleaned.contains("SECRET"));
        assert!(cleaned.contains("openexchangerates.org/api/latest.json"));
    }

    #[test]
    fn a_url_without_a_query_is_left_alone() {
        let message = "connection refused: https://api.coingecko.com/api/v3/simple/price";

        assert_eq!(redact(message), message);
    }
}
