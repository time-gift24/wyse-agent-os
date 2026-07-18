use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::future;
use stratum_core::{
    AgentEvent, AgentId, CallId, ChatMessage, EventCursor, EventSource, HistoryPage, HistoryQuery,
    LlmCallId, LlmCallRole, LlmEvent, ModelConfig, ModelId, NodeId, ReplayStart, RunId,
    RuntimeEvent, StreamEnvelope, TokenUsage, ToolName, TurnId,
};
use stratum_infra::{EventStream, EventStreamBus, EventStreamBusError};
use stratum_store::{AgentState, AgentStatus, AgentStore, StoreError, StoreEventStreamBus};
use tokio::time::{Duration, timeout};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StateUpdate {
    status: AgentStatus,
    run_id: Option<RunId>,
    turn_id: Option<TurnId>,
    usage: TokenUsage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IterationCommit {
    run_id: RunId,
    turn_id: TurnId,
    iteration: u64,
    usage: TokenUsage,
}

#[derive(Debug, Clone, PartialEq)]
enum ObservedOperation {
    IterationCommitted(IterationCommit),
    InnerPublished(StreamEnvelope),
}

struct RecordingStore {
    state: Mutex<AgentState>,
    loads: Mutex<usize>,
    appended: Mutex<Vec<StreamEnvelope>>,
    updates: Mutex<Vec<StateUpdate>>,
    iterations: Mutex<Vec<IterationCommit>>,
    operation_order: Option<Arc<Mutex<Vec<ObservedOperation>>>>,
    fail_append: bool,
    fail_iteration: bool,
}

impl RecordingStore {
    fn new(agent_id: AgentId) -> Self {
        Self {
            state: Mutex::new(AgentState::new(agent_id, "recording".to_owned())),
            loads: Mutex::new(0),
            appended: Mutex::new(Vec::new()),
            updates: Mutex::new(Vec::new()),
            iterations: Mutex::new(Vec::new()),
            operation_order: None,
            fail_append: false,
            fail_iteration: false,
        }
    }

    fn failing(agent_id: AgentId) -> Self {
        Self {
            fail_append: true,
            ..Self::new(agent_id)
        }
    }

    fn with_operation_order(
        agent_id: AgentId,
        operation_order: Arc<Mutex<Vec<ObservedOperation>>>,
    ) -> Self {
        Self {
            operation_order: Some(operation_order),
            ..Self::new(agent_id)
        }
    }

    fn failing_iteration(agent_id: AgentId) -> Self {
        Self {
            fail_iteration: true,
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
        if status == AgentStatus::Running && state.run_id != run_id {
            state.next_iteration = 0;
        }
        state.status = status;
        state.run_id = run_id;
        state.turn_id = turn_id;
        state.usage = usage;
        Ok(state.clone())
    }

    async fn start_turn(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        model_config: ModelConfig,
    ) -> Result<AgentState, StoreError> {
        let mut state = self.state.lock().expect("state lock");
        state.status = AgentStatus::Running;
        state.run_id = Some(run_id);
        state.turn_id = Some(turn_id);
        state.model_config = Some(model_config);
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

    async fn complete_iteration(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        iteration: u64,
        usage: TokenUsage,
    ) -> Result<AgentState, StoreError> {
        let commit = IterationCommit {
            run_id,
            turn_id,
            iteration,
            usage,
        };
        self.iterations
            .lock()
            .expect("iterations lock")
            .push(commit);
        if self.fail_iteration {
            return Err(StoreError::AgentMissing);
        }
        if let Some(operation_order) = &self.operation_order {
            operation_order
                .lock()
                .expect("operation order lock")
                .push(ObservedOperation::IterationCommitted(commit));
        }
        Ok(self.state.lock().expect("state lock").clone())
    }

    async fn history_page(&self, _query: HistoryQuery) -> Result<HistoryPage, StoreError> {
        Err(StoreError::AgentMissing)
    }
}

struct RecordingBus {
    published: Mutex<Vec<StreamEnvelope>>,
    subscriptions: Mutex<Vec<(AgentId, ReplayStart)>>,
    operation_order: Option<Arc<Mutex<Vec<ObservedOperation>>>>,
    fail_publish: bool,
}

struct PendingBus;

#[async_trait]
impl EventStreamBus for PendingBus {
    async fn publish(&self, _envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        future::pending().await
    }

    async fn subscribe_agent(
        &self,
        _agent_id: AgentId,
        _replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        Ok(Box::pin(futures_util::stream::pending()))
    }
}

impl RecordingBus {
    fn new() -> Self {
        Self {
            published: Mutex::new(Vec::new()),
            subscriptions: Mutex::new(Vec::new()),
            operation_order: None,
            fail_publish: false,
        }
    }

    fn failing() -> Self {
        Self {
            fail_publish: true,
            ..Self::new()
        }
    }

    fn with_operation_order(operation_order: Arc<Mutex<Vec<ObservedOperation>>>) -> Self {
        Self {
            operation_order: Some(operation_order),
            ..Self::new()
        }
    }
}

#[async_trait]
impl EventStreamBus for RecordingBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        if let Some(operation_order) = &self.operation_order {
            operation_order
                .lock()
                .expect("operation order lock")
                .push(ObservedOperation::InnerPublished(envelope.clone()));
        }
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

fn test_model_config() -> ModelConfig {
    ModelConfig::new(
        ModelId::new("openai", "test-model").expect("static model is valid"),
        serde_json::Map::new(),
    )
}

#[tokio::test]
async fn started_event_persists_config_with_running_state() {
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::with_model_config(store.clone(), inner, test_model_config());

    bus.publish(envelope(agent_id, run_id, AgentEvent::Started { turn_id }))
        .await
        .expect("started commits");

    let state = store.load_agent().await.expect("state loads");
    assert_eq!(state.status, AgentStatus::Running);
    assert_eq!(state.model_config, Some(test_model_config()));
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
    {
        let mut state = store.state.lock().expect("state lock");
        state.status = AgentStatus::Running;
        state.run_id = Some(RunId::new());
        state.turn_id = Some(TurnId::new());
        state.next_iteration = 3;
    }

    bus.publish(envelope(agent_id, run_id, AgentEvent::Started { turn_id }))
        .await
        .expect("publish started");
    assert_eq!(store.state.lock().expect("state lock").next_iteration, 0);
    store.state.lock().expect("state lock").next_iteration = 4;
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
    assert_eq!(store.state.lock().expect("state lock").next_iteration, 4);
}

#[tokio::test]
async fn iteration_completed_persists_exact_frontier_before_forwarding() {
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let usage = TokenUsage {
        input_tokens: 13,
        output_tokens: 21,
        total_tokens: 34,
    };
    let iteration = 5;
    let operation_order = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(RecordingStore::with_operation_order(
        agent_id,
        operation_order.clone(),
    ));
    let inner = Arc::new(RecordingBus::with_operation_order(operation_order.clone()));
    let bus = StoreEventStreamBus::new(store.clone(), inner);
    let requested = envelope(
        agent_id,
        run_id,
        AgentEvent::IterationCompleted {
            turn_id,
            iteration,
            usage,
        },
    );

    bus.publish(requested.clone())
        .await
        .expect("iteration commits");

    let commit = IterationCommit {
        run_id,
        turn_id,
        iteration,
        usage,
    };
    assert_eq!(
        *store.iterations.lock().expect("iterations lock"),
        vec![commit]
    );
    assert_eq!(
        *operation_order.lock().expect("operation order lock"),
        vec![
            ObservedOperation::IterationCommitted(commit),
            ObservedOperation::InnerPublished(requested),
        ]
    );
}

#[tokio::test]
async fn iteration_completed_store_failure_prevents_forwarding() {
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let usage = TokenUsage {
        input_tokens: 8,
        output_tokens: 5,
        total_tokens: 13,
    };
    let iteration = 3;
    let store = Arc::new(RecordingStore::failing_iteration(agent_id));
    let inner = Arc::new(RecordingBus::new());
    let bus = StoreEventStreamBus::new(store.clone(), inner.clone());
    let requested = envelope(
        agent_id,
        run_id,
        AgentEvent::IterationCompleted {
            turn_id,
            iteration,
            usage,
        },
    );

    let error = bus
        .publish(requested)
        .await
        .expect_err("iteration store failure");

    assert!(matches!(error, EventStreamBusError::Persistence { .. }));
    assert_eq!(
        *store.iterations.lock().expect("iterations lock"),
        vec![IterationCommit {
            run_id,
            turn_id,
            iteration,
            usage,
        }]
    );
    assert!(inner.published.lock().expect("published lock").is_empty());
}

#[tokio::test]
async fn tool_execution_started_requires_inner_publish_ack() {
    let agent_id = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let inner = Arc::new(RecordingBus::failing());
    let bus = StoreEventStreamBus::new(store.clone(), inner.clone());
    let requested = envelope(
        agent_id,
        run_id,
        AgentEvent::ToolExecutionStarted {
            turn_id,
            call_id: CallId::from("call-1"),
            tool_name: ToolName::from("write_file"),
        },
    );

    let error = bus
        .publish(requested.clone())
        .await
        .expect_err("inner ack is required");

    assert!(matches!(error, EventStreamBusError::MissingAgentScope));
    assert!(store.iterations.lock().expect("iterations lock").is_empty());
    assert!(store.appended.lock().expect("appended lock").is_empty());
    assert!(store.updates.lock().expect("updates lock").is_empty());
    assert_eq!(*store.loads.lock().expect("loads lock"), 0);
    assert_eq!(
        *inner.published.lock().expect("published lock"),
        vec![requested]
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
async fn pending_inner_publish_is_bounded_after_message_commit() {
    let agent_id = AgentId::new();
    let store = Arc::new(RecordingStore::new(agent_id));
    let bus = StoreEventStreamBus::new(store.clone(), Arc::new(PendingBus));
    let message = envelope(
        agent_id,
        RunId::new(),
        AgentEvent::Message {
            turn_id: TurnId::new(),
            message: ChatMessage::user("hello"),
        },
    );

    timeout(Duration::from_secs(2), bus.publish(message))
        .await
        .expect("best-effort forwarding is bounded")
        .expect("durable commit remains authoritative");

    assert_eq!(store.appended.lock().expect("appended lock").len(), 1);
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
