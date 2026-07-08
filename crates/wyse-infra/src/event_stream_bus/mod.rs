//! Runtime event stream bus.

pub mod definition;
pub mod error;

pub(crate) mod memory;
pub(crate) mod nats;

pub use definition::{EventStream, EventStreamBus, NatsEventStreamBusConfig};
pub use error::EventStreamBusError;
pub use memory::InMemoryEventStreamBus;

/// Creates a NATS-backed event stream bus.
///
/// # Errors
///
/// Returns an error if the NATS connection or JetStream setup fails.
pub async fn create_nats_event_stream_bus(
    config: NatsEventStreamBusConfig,
) -> Result<impl EventStreamBus, EventStreamBusError> {
    nats::NatsEventStreamBus::new(config).await
}
