use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use wyse_core::{
    AgentEvent, AgentId, ChatMessage, EventCursor, EventSource, HistoryPage, HistoryQuery,
    LlmCallId, LlmCallRole, LlmEvent, NodeId, ReplayStart, RunId, RuntimeEvent, StreamEnvelope,
    TokenUsage, TurnId,
};
use wyse_infra::{EventStream, EventStreamBus, EventStreamBusError};
use wyse_store::{AgentState, AgentStatus, AgentStore, StoreError, StoreEventStreamBus};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateUpdate {
    status: AgentStatus,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    usage: TokenUsage,
}

struct RecordingStore {
    state: Mutex<AgentState>,
    loads: Mutex<usize>,
    appended: Mutex<Vec<StreamEnvelope>>,
    updates: Mutex<Vec<StateUpdate>>,
    fail_append: bool,
}

impl RecordingStore {
    fn new(agent_id: AgentId) -> Self {
        Self {
            state: Mutex::new(AgentState::new(agent_id, "recording".to_owned())),
            loads: Mutex::new(0),
            appended: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            fail_append: false,
        }
    }

    fn failing(agent_id: AgentId) -> Self {
        Self {
            fail_append: true,
            ..Self::new(agent_id)
        }
    }
}

#[async_trait]
impl AgentStore for RecordingStore {
    async fn load_agent(&self) -> Result<AgentState, StoreError> {
        *self.loads.lock().expect("loads lock") += 1;
        Ok(self.state.lock().expect("state lock").clone())
    }

    async fn update_state(
        &self,
        status: AgentStatus,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        usage: TokenUsage,
    ) -> Result<AgentState, StoreError> {
        self.updates
            .lock()
            .expect("updates lock")
            .push(StateUpdate {
                status,
                run_id,
                turn_id,
                usage,
            });
        let mut state = self.state.lock().expect("state lock");
        state.status = status;
        state.run_id = run_id;
        state.turn_id = turn_id;
        state.usage = usage;
        Ok(state.clone())
    }

    async fn append_message(
        &self,
        mut envelope: StreamEnvelope,
    ) -> Result<StreamEnvelope, StoreError> {
        self.appended
            .lock()
            .expect("appended lock")
            .push(envelope.clone());
        if self.fail_append {
            return Err(StoreError::AgentMissing);
        }
        envelope.business_seq = Some(1);
        Ok(envelope)
    }

    async fn history_page(&self, _query: HistoryQuery) -> Result<HistoryPage, StoreError> {
        Err(StoreError::AgentMissing)
    }
}

struct RecordingBus {
    published: Mutex<Vec<StreamEnvelope>>,
    subscriptions: Mutex<Vec<(AgentId, ReplayStart)>>,
    fail_publish: bool,
}

impl RecordingBus {
    fn new() -> Self {
        Self {
            published: Mutex::new(Vec::new()),
            subscriptions: Mutex::new(Vec::new()),
            fail_publish: false,
        }
    }

    fn failing() -> Self {
        Self {
            fail_publish: true,
            ..Self::new()
        }
    }
}

#[async_trait]
impl EventStreamBus for RecordingBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        self.published
            .lock()
            .expect("published lock")
            .push(envelope);
        if self.fail_publish {
            Err(EventStreamBusError::MissingAgentScope)
        } else {
            Ok(())
        }
    }

    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        self.subscriptions
            .lock()
            .expect("subscriptions lock")
            .push((agent_id, replay_start));
        Ok(Box::pin(futures_util::stream::empty()))
    }
}

fn envelope(agent_id: AgentId, run_id: RunId, event: AgentEvent) -> StreamEnvelope {
    StreamEnvelope {
        business_seq: None,
        run_id,
        timestamp: Utc::now(),
        source: EventSource::Agent {
            node_id: NodeId::new("node"),
            agent_id,
        },
        event: RuntimeEvent::Agent { agent_id, event },
        metadata: BTreeMap::new(),
    }
}

#[tokio::test]
async fn message_is_stored_unsequenced_then_forwarded_with_committed_sequence() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::new(store.clone(), inner.clone());
    let requested = envelope(
        agent_id,
        RunId::new(),
        AgentEvent::Message {
            turn_id: TurnId::new(),
            message: ChatMessage::user("hello"),
        },
    );

    bus.publish(requested).await.expect("publish message");

    assert_eq!(
        store.appended.lock().expect("appended lock")[0].business_seq,
        None
    );
    assert_eq!(
        inner.published.lock().expect("published lock")[0].business_seq,
        Some(1)
    );
}

#[tokio::test]
async fn state_events_commit_matching_status_run_turn_and_usage() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::new(store.clone(), inner);
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let finished_usage = TokenUsage {
        input_tokens: 1,
        output_tokens: 2,
        total_tokens: 3,
    };
    let failed_usage = TokenUsage {
        input_tokens: 4,
        output_tokens: 5,
        total_tokens: 9,
    };
    let cancelled_usage = TokenUsage {
        input_tokens: 6,
        output_tokens: 7,
        total_tokens: 13,
    };

    bus.publish(envelope(agent_id, run_id, AgentEvent::Started { turn_id }))
        .await
        .expect("publish started");
    bus.publish(envelope(
        agent_id,
        run_id,
        AgentEvent::Finished {
            finish_reason: "stop".to_owned(),
            usage: finished_usage,
        },
    ))
    .await
    .expect("publish finished");
    bus.publish(envelope(
        agent_id,
        run_id,
        AgentEvent::Failed {
            error_text: "failed".to_owned(),
            usage: failed_usage,
        },
    ))
    .await
    .expect("publish failed");
    bus.publish(envelope(
        agent_id,
        run_id,
        AgentEvent::Cancelled {
            usage: cancelled_usage,
        },
    ))
    .await
    .expect("publish cancelled");

    assert_eq!(
        *store.updates.lock().expect("updates lock"),
        vec![
            StateUpdate {
                status: AgentStatus::Running,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                usage: TokenUsage::default(),
            },
            StateUpdate {
                status: AgentStatus::Finished,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                usage: finished_usage,
            },
            StateUpdate {
                status: AgentStatus::Failed,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                usage: failed_usage,
            },
            StateUpdate {
                status: AgentStatus::Cancelled,
                run_id: Some(run_id),
                turn_id: Some(turn_id),
                usage: cancelled_usage,
            },
        ]
    );
}

#[tokio::test]
async fn llm_delta_bypasses_store_and_is_forwarded_unchanged() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::new(store.clone(), inner.clone());
    let delta = envelope(
        agent_id,
        RunId::new(),
        AgentEvent::Llm {
            llm_call_id: LlmCallId::from("call-1"),
            event: LlmEvent::TextDelta {
                role: LlmCallRole::Assistant,
                delta: "partial".to_owned(),
            },
        },
    );

    bus.publish(delta.clone()).await.expect("publish delta");

    assert!(store.appended.lock().expect("appended lock").is_empty());
    assert!(store.updates.lock().expect("updates lock").is_empty());
    assert_eq!(*store.loads.lock().expect("loads lock"), 0);
    assert_eq!(
        *inner.published.lock().expect("published lock"),
        vec![delta]
    );
}

#[tokio::test]
async fn store_failure_is_persistence_error_and_prevents_inner_publish() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::failing(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::new(store, inner.clone());
    let message = envelope(
        agent_id,
        RunId::new(),
        AgentEvent::Message {
            turn_id: TurnId::new(),
            message: ChatMessage::user("hello"),
        },
    );

    let error = bus.publish(message).await.expect_err("store failure");

    assert!(matches!(&error, EventStreamBusError::Persistence { .. }));
    assert_eq!(error.to_string(), "event persistence failed");
    assert!(inner.published.lock().expect("published lock").is_empty());
}

#[tokio::test]
async fn inner_failure_after_committed_message_is_warn_only() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::failing());
    let bus = StoreEventStreamBus::new(store.clone(), inner.clone());
    let message = envelope(
        agent_id,
        RunId::new(),
        AgentEvent::Message {
            turn_id: TurnId::new(),
            message: ChatMessage::user("hello"),
        },
    );

    bus.publish(message)
        .await
        .expect("durable store is authoritative");

    assert_eq!(store.appended.lock().expect("appended lock").len(), 1);
    assert_eq!(inner.published.lock().expect("published lock").len(), 1);
}

#[tokio::test]
async fn subscription_delegates_agent_and_replay_start_unchanged() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::new(store, inner.clone());
    let replay_start = ReplayStart::After(EventCursor::from_transport_sequence(41));

    let _stream = bus
        .subscribe_agent(agent_id, replay_start)
        .await
        .expect("subscribe");

    assert_eq!(
        *inner.subscriptions.lock().expect("subscriptions lock"),
        vec![(agent_id, replay_start)]
    );
}
