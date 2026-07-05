//! Error types for LLM operations.

use std::{error::Error, fmt};

use thiserror::Error;

/// Secret API key used by provider clients.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Creates a new API key wrapper.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the raw API key string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ApiKey").field(&"[redacted]").finish()
    }
}

/// Error returned by LLM operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LlmError {
    /// Request construction failed.
    #[error("failed to build request")]
    RequestBuild(#[source] Box<dyn Error + Send + Sync + 'static>),
    /// Transport operation failed.
    #[error("transport operation failed")]
    Transport(#[source] Box<dyn Error + Send + Sync + 'static>),
    /// Provider returned a non-success status.
    #[error("provider returned status {0}")]
    ProviderStatus(ProviderStatusError),
    /// Provider error payload could not be decoded.
    #[error("failed to decode provider payload")]
    ProviderPayloadDecode(#[source] serde_json::Error),
    /// Successful provider response could not be decoded.
    #[error("failed to decode response")]
    ResponseDecode(#[source] serde_json::Error),
    /// Provider payload was syntactically valid but did not match the expected shape.
    #[error("invalid provider payload: {0}")]
    InvalidProviderPayload(&'static str),
    /// Stream operation failed.
    #[error("stream operation failed")]
    Stream(#[source] Box<dyn Error + Send + Sync + 'static>),
    /// Provider does not support the requested capability.
    #[error("unsupported capability: {0}")]
    UnsupportedCapability(&'static str),
    /// Mock provider had no queued response.
    #[error("mock response queue is exhausted")]
    MockExhausted,
}

impl LlmError {
    #[expect(
        dead_code,
        reason = "provider backends use this constructor when transport calls are added"
    )]
    pub(crate) fn transport(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Transport(Box::new(source))
    }

    #[expect(
        dead_code,
        reason = "provider backends use this constructor when streaming calls are added"
    )]
    pub(crate) fn stream(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Stream(Box::new(source))
    }
}

/// Error payload returned by a provider with a non-success status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatusError {
    /// HTTP status code returned by the provider.
    pub status: u16,
    /// Provider-specific error code when available.
    pub code: Option<String>,
    /// Safe, bounded provider error message.
    pub message: String,
    /// Provider request id when available.
    pub request_id: Option<String>,
}

impl ProviderStatusError {
    /// Creates a status error and bounds the provider message.
    #[must_use]
    pub fn new(
        status: u16,
        code: Option<String>,
        message: impl Into<String>,
        request_id: Option<String>,
    ) -> Self {
        Self {
            status,
            code,
            message: bound_message(message.into()),
            request_id,
        }
    }
}

impl fmt::Display for ProviderStatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.status)
    }
}

fn bound_message(message: String) -> String {
    const MAX_LEN: usize = 160;

    if message.len() <= MAX_LEN {
        return message;
    }

    let mut bounded = String::with_capacity(MAX_LEN);
    for ch in message.chars() {
        if bounded.len() + ch.len_utf8() > MAX_LEN {
            break;
        }
        bounded.push(ch);
    }
    bounded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_debug_is_redacted() {
        let key = ApiKey::new("sk-secret");

        assert_eq!(format!("{key:?}"), "ApiKey(\"[redacted]\")");
        assert_eq!(key.as_str(), "sk-secret");
    }

    #[test]
    fn provider_message_is_bounded() {
        let error = ProviderStatusError::new(429, None, "x".repeat(300), Some("req-1".to_owned()));

        assert!(error.message.len() <= 160);
        assert_eq!(error.request_id.as_deref(), Some("req-1"));
    }
}
