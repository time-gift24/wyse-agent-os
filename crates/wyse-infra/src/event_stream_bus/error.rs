//! Error types for event stream bus operations.

use std::error::Error;

use thiserror::Error;
use wyse_core::EventCursor;

/// Error returned by event stream bus operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EventStreamBusError {
    /// Published envelope does not belong to an agent.
    #[error("stream envelope is missing agent scope")]
    MissingAgentScope,
    /// Requested cursor is no longer retained for the subscribed agent.
    #[error("event cursor {cursor:?} is no longer retained")]
    CursorExpired {
        /// Cursor requested by the subscriber.
        cursor: EventCursor,
    },
    /// Requested cursor cannot be advanced to the next transport sequence.
    #[error("event cursor cannot advance beyond the maximum transport sequence")]
    CursorOverflow,
    /// NATS event stream configuration is invalid.
    #[error("invalid nats event stream configuration: {reason}")]
    InvalidConfig {
        /// Invalid configuration condition.
        reason: &'static str,
    },
    /// Event envelope serialization failed.
    #[error("failed to serialize stream envelope")]
    Serialize(#[source] serde_json::Error),
    /// Event envelope deserialization failed.
    #[error("failed to deserialize stream envelope")]
    Deserialize(#[source] serde_json::Error),
    /// Durable event persistence failed.
    #[error("event persistence failed")]
    Persistence {
        /// Underlying persistence error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
    /// NATS operation failed.
    #[error("nats operation failed")]
    Nats {
        /// Underlying NATS error.
        #[source]
        source: Box<dyn Error + Send + Sync + 'static>,
    },
}

impl EventStreamBusError {
    /// Wraps a durable event persistence failure.
    pub fn persistence(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Persistence {
            source: Box::new(source),
        }
    }

    pub(crate) fn nats(source: impl Error + Send + Sync + 'static) -> Self {
        Self::Nats {
            source: Box::new(source),
        }
    }
}
