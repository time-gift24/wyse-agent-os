//! Infrastructure primitives for Stratum runtimes.

use thiserror::Error;

pub mod event_stream_bus;

pub use event_stream_bus::{
    EventStream, EventStreamBus, EventStreamBusError, NatsEventStreamBusConfig,
    create_nats_event_stream_bus,
};

/// Error returned by infrastructure operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InfraError {
    /// Event stream bus operation failed.
    #[error("event stream bus operation failed")]
    EventStreamBus(#[from] EventStreamBusError),
}
