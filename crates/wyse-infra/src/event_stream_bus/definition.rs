//! Event stream bus public definitions.

use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;
use wyse_core::{RunId, StreamEnvelope};

use super::EventStreamBusError;

/// Stream of runtime event envelopes.
pub type EventStream =
    Pin<Box<dyn Stream<Item = Result<StreamEnvelope, EventStreamBusError>> + Send + 'static>>;

/// Publishes and subscribes to runtime event streams.
#[async_trait]
pub trait EventStreamBus: Send + Sync {
    /// Publishes one complete stream envelope.
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError>;

    /// Subscribes to live events for one run.
    async fn subscribe_run(&self, run_id: RunId) -> Result<EventStream, EventStreamBusError>;
}

/// Configuration for the NATS event stream bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsEventStreamBusConfig {
    /// NATS server URL.
    pub url: String,
    /// JetStream stream name.
    pub stream_name: String,
    /// Subject prefix before `<run_id>.<event_type>`.
    pub subject_prefix: String,
    /// Number of stream replicas.
    pub replicas: usize,
}

impl Default for NatsEventStreamBusConfig {
    fn default() -> Self {
        Self {
            url: "nats://localhost:4222".to_owned(),
            stream_name: "WYSE_EVENTS".to_owned(),
            subject_prefix: "wyse.events".to_owned(),
            replicas: 1,
        }
    }
}
