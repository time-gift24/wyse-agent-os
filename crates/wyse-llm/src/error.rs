//! Error types for LLM operations.

use thiserror::Error;

/// Error returned by LLM provider operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LlmError {
    /// Requested capability is not supported.
    #[error("unsupported capability: {0}")]
    UnsupportedCapability(&'static str),
}
