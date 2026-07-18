//! Errors returned by durable agent event sinks.

use thiserror::Error;

use crate::EventStreamBusError;

/// Error returned when a durable agent-loop event cannot be acknowledged.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DurableEventSinkError {
    /// The configured event stream bus rejected the durable event.
    #[error("durable agent event publish failed")]
    EventStreamBus(
        #[from]
        #[source]
        EventStreamBusError,
    ),
    /// The sink does not know how to project a newer durable event variant.
    #[error("unsupported durable agent event type {event_type}")]
    UnsupportedEvent {
        /// Stable type name of the unsupported event.
        event_type: &'static str,
    },
    /// The sink's ordered publisher is no longer available.
    #[error("durable agent event publisher is unavailable")]
    PublisherUnavailable,
}
