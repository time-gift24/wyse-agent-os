//! Error types for LLM operations.

use std::{error::Error, fmt};

use thiserror::Error;
use wyse_core::ModelId;

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
    /// A provider was already registered for the model.
    #[error("provider already registered for model: {model}")]
    DuplicateProvider { model: ModelId },
    /// No provider was registered for the model.
    #[error("provider not found for model: {model}")]
    ProviderNotFound { model: ModelId },
    /// Request is invalid before it reaches a provider.
    #[error("invalid request: {0}")]
    InvalidRequest(&'static str),
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
    pub(crate) fn transport(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Transport(Box::new(source))
    }

    pub(crate) fn stream(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Stream(Box::new(source))
    }
}

/// Error payload returned by a provider with a non-success status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderStatusError {
    status: u16,
    code: Option<String>,
    message: String,
    request_id: Option<String>,
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

    /// Returns the HTTP status code returned by the provider.
    #[must_use]
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Returns the provider-specific error code when available.
    #[must_use]
    pub fn code(&self) -> Option<&str> {
        self.code.as_deref()
    }

    /// Returns the safe, bounded provider error message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns the provider request id when available.
    #[must_use]
    pub fn request_id(&self) -> Option<&str> {
        self.request_id.as_deref()
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
        let debug = format!("{key:?}");

        assert_eq!(debug, "ApiKey(\"[redacted]\")");
        assert!(!debug.contains("sk-secret"));
        assert_eq!(key.as_str(), "sk-secret");
    }

    #[test]
    fn provider_message_is_bounded() {
        let error = ProviderStatusError::new(
            429,
            Some("rate_limit".to_owned()),
            "x".repeat(300),
            Some("req-1".to_owned()),
        );
        let multibyte = ProviderStatusError::new(500, None, "你".repeat(100), None);

        assert_eq!(error.status(), 429);
        assert_eq!(error.code(), Some("rate_limit"));
        assert!(error.message().len() <= 160);
        assert_eq!(error.request_id(), Some("req-1"));
        assert!(multibyte.message().len() <= 160);
        assert!(multibyte.message().chars().all(|ch| ch == '你'));
    }
}
