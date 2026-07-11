//! Event stream bus store persistence.

use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use tokio::time::timeout;
use wyse_core::{AgentEvent, AgentId, ReplayStart, RuntimeEvent, StreamEnvelope, TokenUsage};
use wyse_infra::{EventStream, EventStreamBus, EventStreamBusError};

use crate::{AgentStatus, AgentStore};

const COMMITTED_FORWARD_GRACE: Duration = Duration::from_secs(1);

/// Persists complete agent messages and state before forwarding them to an event stream bus.
pub struct StoreEventStreamBus {
    store: Arc<dyn AgentStore>,
    inner: Arc<dyn EventStreamBus>,
}

impl StoreEventStreamBus {
    /// Creates a store-backed event stream bus decorator.
    #[must_use]
    pub fn new(store: Arc<dyn AgentStore>, inner: Arc<dyn EventStreamBus>) -> Self {
        Self { store, inner }
    }

    async fn forward_committed(&self, envelope: StreamEnvelope) {
        match timeout(COMMITTED_FORWARD_GRACE, self.inner.publish(envelope)).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                tracing::warn!(source = %error, "committed agent event was not retained");
            }
            Err(_) => {
                tracing::warn!(
                    grace_millis = COMMITTED_FORWARD_GRACE.as_millis(),
                    "committed agent event forwarding timed out"
                );
            }
        }
    }
}

#[async_trait]
impl EventStreamBus for StoreEventStreamBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        match &envelope.event {
            RuntimeEvent::Agent {
                event: AgentEvent::Message { .. },
                ..
            } => {
                let committed = self
                    .store
                    .append_message(envelope)
                    .await
                    .map_err(EventStreamBusError::persistence)?;
                self.forward_committed(committed).await;
                Ok(())
            }
            RuntimeEvent::Agent {
                event: AgentEvent::Started { turn_id },
                ..
            } => {
                self.store
                    .update_state(
                        AgentStatus::Running,
                        Some(envelope.run_id),
                        Some(*turn_id),
                        TokenUsage::default(),
                    )
                    .await
                    .map_err(EventStreamBusError::persistence)?;
                self.forward_committed(envelope).await;
                Ok(())
            }
            RuntimeEvent::Agent { event, .. } => {
                let (status, usage) = match event {
                    AgentEvent::Finished { usage, .. } => (AgentStatus::Finished, *usage),
                    AgentEvent::Failed { usage, .. } => (AgentStatus::Failed, *usage),
                    AgentEvent::Cancelled { usage } => (AgentStatus::Cancelled, *usage),
                    _ => return self.inner.publish(envelope).await,
                };
                let state = self
                    .store
                    .load_agent()
                    .await
                    .map_err(EventStreamBusError::persistence)?;
                self.store
                    .update_state(status, Some(envelope.run_id), state.turn_id, usage)
                    .await
                    .map_err(EventStreamBusError::persistence)?;
                self.forward_committed(envelope).await;
                Ok(())
            }
            _ => self.inner.publish(envelope).await,
        }
    }

    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        self.inner.subscribe_agent(agent_id, replay_start).await
    }
}
