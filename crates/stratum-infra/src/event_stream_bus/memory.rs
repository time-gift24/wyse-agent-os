//! In-memory event stream bus for tests and local embedding.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures_util::stream;
use stratum_core::{AgentId, EventCursor, EventRecord, ReplayStart, RuntimeEvent, StreamEnvelope};
use tokio::sync::Notify;

use super::{EventStream, EventStreamBus, EventStreamBusError};

/// In-memory event stream bus backed by retained per-agent event history.
#[derive(Debug, Clone, Default)]
pub struct InMemoryEventStreamBus {
    agents: Arc<Mutex<BTreeMap<AgentId, Arc<AgentEvents>>>>,
}

#[derive(Debug)]
struct AgentEvents {
    history: Mutex<AgentHistory>,
    notify: Notify,
}

#[derive(Debug)]
struct AgentHistory {
    records: Vec<EventRecord>,
    next_cursor: u64,
}

impl AgentEvents {
    fn new() -> Self {
        Self {
            history: Mutex::new(AgentHistory {
                records: Vec::new(),
                next_cursor: 1,
            }),
            notify: Notify::new(),
        }
    }

    fn event_at(&self, index: usize) -> Option<EventRecord> {
        self.history
            .lock()
            .expect("in-memory event history mutex should not be poisoned")
            .records
            .get(index)
            .cloned()
    }
}

impl InMemoryEventStreamBus {
    fn agent_events(&self, agent_id: AgentId) -> Arc<AgentEvents> {
        let mut agents = self
            .agents
            .lock()
            .expect("in-memory event bus mutex should not be poisoned");
        Arc::clone(
            agents
                .entry(agent_id)
                .or_insert_with(|| Arc::new(AgentEvents::new())),
        )
    }
}

#[async_trait]
impl EventStreamBus for InMemoryEventStreamBus {
    /// Publishes one complete stream envelope.
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        let RuntimeEvent::Agent { agent_id, .. } = &envelope.event else {
            return Err(EventStreamBusError::MissingAgentScope);
        };
        let agent_events = self.agent_events(*agent_id);
        let mut history = agent_events
            .history
            .lock()
            .expect("in-memory event history mutex should not be poisoned");
        let cursor = EventCursor::from_transport_sequence(history.next_cursor);
        history.next_cursor = history
            .next_cursor
            .checked_add(1)
            .expect("retained event cursor should not overflow");
        history.records.push(EventRecord { cursor, envelope });
        drop(history);
        agent_events.notify.notify_waiters();
        Ok(())
    }

    /// Subscribes to retained and live events for one agent.
    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        let agent_events = self.agent_events(agent_id);
        let history = agent_events
            .history
            .lock()
            .expect("in-memory event history mutex should not be poisoned");
        let next_index = match replay_start {
            ReplayStart::All => 0,
            ReplayStart::New => history.records.len(),
            ReplayStart::After(cursor) => history
                .records
                .iter()
                .position(|record| record.cursor == cursor)
                .map(|index| index + 1)
                .ok_or(EventStreamBusError::CursorExpired { cursor })?,
        };
        drop(history);

        Ok(Box::pin(stream::unfold(
            (agent_events, next_index),
            |(agent_events, mut next_index)| async move {
                loop {
                    if let Some(record) = agent_events.event_at(next_index) {
                        next_index += 1;
                        return Some((Ok(record), (agent_events, next_index)));
                    }
                    let notified = agent_events.notify.notified();
                    if let Some(record) = agent_events.event_at(next_index) {
                        drop(notified);
                        next_index += 1;
                        return Some((Ok(record), (agent_events, next_index)));
                    }
                    notified.await;
                }
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use futures_util::StreamExt;
    use stratum_core::{
        AgentEvent, AgentId, ChatMessage, EventCursor, EventSource, ReplayStart, RunId,
        RuntimeEvent, TokenUsage, TurnId,
    };

    use super::*;
    use crate::event_stream_bus::EventStreamBus;

    fn agent_envelope(agent_id: AgentId, event: AgentEvent) -> StreamEnvelope {
        StreamEnvelope {
            business_seq: None,
            run_id: RunId::new(),
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent { agent_id, event },
            metadata: BTreeMap::new(),
        }
    }

    fn message_envelope(agent_id: AgentId, seq: u64) -> StreamEnvelope {
        let mut envelope = agent_envelope(
            agent_id,
            AgentEvent::Message {
                turn_id: TurnId::new(),
                message: ChatMessage::user("hello"),
            },
        );
        envelope.business_seq = Some(seq);
        envelope
    }

    fn started_event() -> AgentEvent {
        AgentEvent::Started {
            turn_id: TurnId::new(),
        }
    }

    fn cancelled_event() -> AgentEvent {
        AgentEvent::Cancelled {
            usage: TokenUsage::default(),
        }
    }

    #[tokio::test]
    async fn replay_modes_use_transport_cursor_not_message_sequence() {
        let bus = InMemoryEventStreamBus::default();
        let agent_id = AgentId::new();
        bus.publish(agent_envelope(agent_id, started_event()))
            .await
            .expect("publish 1");
        bus.publish(message_envelope(agent_id, 99))
            .await
            .expect("publish 2");

        let mut all = bus
            .subscribe_agent(agent_id, ReplayStart::All)
            .await
            .expect("all");
        let first = all.next().await.expect("first").expect("record");
        let second = all.next().await.expect("second").expect("record");
        assert_eq!(first.cursor.transport_sequence(), 1);
        assert_eq!(second.cursor.transport_sequence(), 2);

        let mut after = bus
            .subscribe_agent(agent_id, ReplayStart::After(first.cursor))
            .await
            .expect("after");
        assert_eq!(after.next().await.expect("second").expect("record"), second);
    }

    #[tokio::test]
    async fn all_replay_continues_with_live_events() {
        let bus = InMemoryEventStreamBus::default();
        let agent_id = AgentId::new();
        bus.publish(agent_envelope(agent_id, started_event()))
            .await
            .expect("publish retained");
        let mut stream = bus
            .subscribe_agent(agent_id, ReplayStart::All)
            .await
            .expect("subscribe");
        bus.publish(agent_envelope(agent_id, cancelled_event()))
            .await
            .expect("publish live");

        let retained = stream.next().await.expect("retained").expect("record");
        let live = stream.next().await.expect("live").expect("record");

        assert_eq!(retained.cursor.transport_sequence(), 1);
        assert_eq!(live.cursor.transport_sequence(), 2);
    }

    #[tokio::test]
    async fn new_subscription_starts_at_creation() {
        let bus = InMemoryEventStreamBus::default();
        let agent_id = AgentId::new();
        bus.publish(agent_envelope(agent_id, started_event()))
            .await
            .expect("publish retained");
        let mut stream = bus
            .subscribe_agent(agent_id, ReplayStart::New)
            .await
            .expect("subscribe");
        bus.publish(agent_envelope(agent_id, cancelled_event()))
            .await
            .expect("publish live");

        let received = stream.next().await.expect("live").expect("record");

        assert_eq!(received.cursor.transport_sequence(), 2);
        assert!(matches!(
            received.envelope.event,
            RuntimeEvent::Agent {
                event: AgentEvent::Cancelled { .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn subscription_is_isolated_by_agent() {
        let bus = InMemoryEventStreamBus::default();
        let agent_id = AgentId::new();
        let other_agent_id = AgentId::new();
        let mut stream = bus
            .subscribe_agent(agent_id, ReplayStart::New)
            .await
            .expect("subscribe");

        bus.publish(agent_envelope(other_agent_id, started_event()))
            .await
            .expect("publish other");
        bus.publish(agent_envelope(agent_id, cancelled_event()))
            .await
            .expect("publish target");

        let received = stream.next().await.expect("target").expect("record");

        assert_eq!(received.cursor.transport_sequence(), 1);
        assert!(matches!(
            received.envelope.event,
            RuntimeEvent::Agent {
                agent_id: received_agent_id,
                ..
            } if received_agent_id == agent_id
        ));
    }

    #[tokio::test]
    async fn after_rejects_cursor_not_retained_for_agent() {
        let bus = InMemoryEventStreamBus::default();
        let agent_id = AgentId::new();
        let cursor = EventCursor::from_transport_sequence(1);

        let result = bus
            .subscribe_agent(agent_id, ReplayStart::After(cursor))
            .await;

        assert!(matches!(
            result,
            Err(EventStreamBusError::CursorExpired {
                cursor: expired_cursor
            }) if expired_cursor == cursor
        ));
    }

    #[tokio::test]
    async fn subscriber_reads_three_retained_events_without_capacity_setting() {
        let bus = InMemoryEventStreamBus::default();
        let agent_id = AgentId::new();
        for event in [started_event(), cancelled_event(), started_event()] {
            bus.publish(agent_envelope(agent_id, event))
                .await
                .expect("publish");
        }
        let mut stream = bus
            .subscribe_agent(agent_id, ReplayStart::All)
            .await
            .expect("subscribe");

        for expected_cursor in 1..=3 {
            let received = stream.next().await.expect("event").expect("record");
            assert_eq!(received.cursor.transport_sequence(), expected_cursor);
        }
    }

    #[tokio::test]
    async fn publish_rejects_envelope_without_agent_scope() {
        let bus = InMemoryEventStreamBus::default();
        let envelope = StreamEnvelope {
            business_seq: None,
            run_id: RunId::new(),
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::RunStarted,
            metadata: BTreeMap::new(),
        };

        let result = bus.publish(envelope).await;

        assert!(matches!(
            result,
            Err(EventStreamBusError::MissingAgentScope)
        ));
    }
}
