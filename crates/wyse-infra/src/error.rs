//! Error types for infrastructure primitives.

use std::error::Error;

use thiserror::Error;

/// Error returned by event stream bus operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EventStreamBusError {
    /// Event envelope serialization failed.
    #[error("failed to serialize stream envelope")]
    Serialize(#[source] serde_json::Error),
    /// Event envelope deserialization failed.
    #[error("failed to deserialize stream envelope")]
    Deserialize(#[source] serde_json::Error),
    /// NATS operation failed.
    #[error("nats operation failed")]
    Nats {
        /// Underlying NATS error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
}

impl EventStreamBusError {
    pub(crate) fn nats(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Nats {
            source: Box::new(source),
        }
    }
}
