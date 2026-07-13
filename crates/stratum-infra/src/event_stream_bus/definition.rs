//! Event stream bus public definitions.

use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use futures_core::Stream;
use stratum_core::{AgentId, EventRecord, ReplayStart, StreamEnvelope};

use super::EventStreamBusError;

/// Stream of runtime event records.
pub type EventStream =
    Pin<Box<dyn Stream<Item = Result<EventRecord, EventStreamBusError>> + Send + 'static>>;

/// Publishes and subscribes to runtime event streams.
#[async_trait]
pub trait EventStreamBus: Send + Sync {
    /// Publishes one complete stream envelope.
    ///
    /// # Errors
    ///
    /// Returns an error if the envelope has no agent scope, cannot be serialized, or the backend
    /// rejects the publish.
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError>;

    /// Subscribes to one agent's retained and live events from the requested position.
    ///
    /// # Errors
    ///
    /// Returns [`EventStreamBusError::CursorExpired`] if the requested cursor is no longer
    /// retained, [`EventStreamBusError::CursorOverflow`] if the transport cannot advance past
    /// the cursor, or a backend error if the subscription cannot be created.
    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError>;
}

/// Configuration for the NATS event stream bus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NatsEventStreamBusConfig {
    /// NATS server URL.
    pub url: String,
    /// JetStream stream name.
    pub stream_name: String,
    /// Subject prefix before `<agent_id>.<agent_event_type>`.
    pub subject_prefix: String,
    /// Number of stream replicas.
    pub replicas: usize,
    /// Maximum retained event age.
    pub max_age: Duration,
    /// Maximum retained stream size in bytes.
    pub max_bytes: i64,
    /// Maximum retained event count.
    pub max_messages: i64,
}

impl Default for NatsEventStreamBusConfig {
    fn default() -> Self {
        Self {
            url: "nats://localhost:4222".to_owned(),
            stream_name: "AGENT_EVENTS".to_owned(),
            subject_prefix: "events.agent".to_owned(),
            replicas: 1,
            max_age: Duration::from_secs(7 * 24 * 60 * 60),
            max_bytes: 1_073_741_824,
            max_messages: 1_000_000,
        }
    }
}
