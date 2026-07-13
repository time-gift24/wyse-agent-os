//! Error types for Stratum core values.

use thiserror::Error;

/// Error returned when a model id is not canonical.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ModelIdParseError {
    /// The value is not exactly `provider:model`.
    #[error("model id must use provider:model")]
    InvalidFormat,
}
