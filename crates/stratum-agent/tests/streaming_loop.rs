use std::{
    collections::{BTreeMap, VecDeque},
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use futures_util::{StreamExt, stream};
use serde_json::json;
use stratum_agent::{Agent, AgentConfig, AgentError};
use stratum_core::{
    AgentEvent, AgentId, ApprovalDecision, ApprovalId, CallId, ChatContent, ChatMessage, ChatRole,
    DangerLevel, EventSource, HistoryPage, HistoryQuery, LlmCallRole, LlmEvent, ModelId,
    ReplayStart, RunId, RuntimeEvent, StreamEnvelope, TokenUsage, ToolCall, ToolCallDelta,
    ToolKind, ToolName, ToolSpec, TurnId,
};
use stratum_infra::event_stream_bus::{
    EventStream, EventStreamBus, EventStreamBusError, InMemoryEventStreamBus,
};
use stratum_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use stratum_store::{AgentState, AgentStatus, AgentStore, StoreError, StoreEventStreamBus};
use stratum_tools::{
    BuiltinToolRegistry, EchoTool, Tool, ToolError, ToolInput, ToolOutput, ToolPermissionMode,
    ToolRegistry,
};
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
enum ProviderResponse {
    Events(Vec<ChatStreamEvent>),
    StreamResults(Vec<Result<ChatStreamEvent, LlmError>>),
    Pending,
}

#[derive(Debug)]
struct RecordingProvider {
    requests: Mutex<Vec<ChatRequest>>,
    responses: Mutex<VecDeque<ProviderResponse>>,
    order: Option<Arc<Mutex<Vec<&'static str>>>>,
}

impl RecordingProvider {
    fn new(responses: Vec<ProviderResponse>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
            order: None,
        }
    }

    fn with_order(responses: Vec<ProviderResponse>, order: Arc<Mutex<Vec<&'static str>>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
            order: Some(order),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .expect("requests mutex should not be poisoned")
            .clone()
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    fn model_id(&self) -> ModelId {
        "recording:mock-model".parse().expect("model id parses")
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::UnsupportedCapability("chat"))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        if let Some(order) = &self.order {
            order.lock().expect("order mutex").push("request");
        }
        self.requests
            .lock()
            .expect("requests mutex should not be poisoned")
            .push(request);
        let response = self
            .responses
            .lock()
            .expect("responses mutex should not be poisoned")
            .pop_front()
            .ok_or(LlmError::MockExhausted)?;

        match response {
            ProviderResponse::Events(events) => {
                Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
            }
            ProviderResponse::StreamResults(results) => Ok(Box::pin(stream::iter(results))),
            ProviderResponse::Pending => Ok(Box::pin(stream::pending())),
        }
    }
}

#[derive(Debug)]
struct PendingChatProvider;

#[async_trait]
impl LlmProvider for PendingChatProvider {
    fn model_id(&self) -> ModelId {
        "pending:mock-model".parse().expect("model id parses")
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::UnsupportedCapability("chat"))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        pending().await
    }
}

struct TestStore {
    state: Mutex<AgentState>,
    history: Mutex<Vec<StreamEnvelope>>,
    completed: Mutex<Vec<(RunId, TurnId, u64, TokenUsage)>>,
    load_entered: Option<Arc<tokio::sync::Notify>>,
    block_load_once: AtomicBool,
    order: Option<Arc<Mutex<Vec<&'static str>>>>,
}

impl TestStore {
    fn idle(agent_id: AgentId) -> Self {
        Self {
            state: Mutex::new(AgentState::new(agent_id, "test-agent".to_owned())),
            history: Mutex::new(Vec::new()),
            completed: Mutex::new(Vec::new()),
            load_entered: None,
            block_load_once: AtomicBool::new(false),
            order: None,
        }
    }

    fn with_state(state: AgentState, history: Vec<StreamEnvelope>) -> Self {
        Self {
            state: Mutex::new(state),
            history: Mutex::new(history),
            completed: Mutex::new(Vec::new()),
            load_entered: None,
            block_load_once: AtomicBool::new(false),
            order: None,
        }
    }

    fn with_state_and_order(
        state: AgentState,
        history: Vec<StreamEnvelope>,
        order: Arc<Mutex<Vec<&'static str>>>,
    ) -> Self {
        Self {
            state: Mutex::new(state),
            history: Mutex::new(history),
            completed: Mutex::new(Vec::new()),
            load_entered: None,
            block_load_once: AtomicBool::new(false),
            order: Some(order),
        }
    }

    fn blocking_load(agent_id: AgentId, entered: Arc<tokio::sync::Notify>) -> Self {
        Self {
            load_entered: Some(entered),
            block_load_once: AtomicBool::new(true),
            ..Self::idle(agent_id)
        }
    }
}

#[async_trait]
impl AgentStore for TestStore {
    async fn load_agent(&self) -> Result<AgentState, StoreError> {
        if let Some(entered) = &self.load_entered
            && self.block_load_once.swap(false, Ordering::SeqCst)
        {
            entered.notify_one();
            pending::<()>().await;
        }
        Ok(self.state.lock().expect("state mutex").clone())
    }

    async fn update_state(
        &self,
        status: AgentStatus,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        usage: TokenUsage,
    ) -> Result<AgentState, StoreError> {
        let mut state = self.state.lock().expect("state mutex");
        state.status = status;
        state.run_id = run_id;
        state.turn_id = turn_id;
        state.usage = usage;
        Ok(state.clone())
    }

    async fn complete_iteration(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        iteration: u64,
        usage: TokenUsage,
    ) -> Result<AgentState, StoreError> {
        self.completed
            .lock()
            .expect("completed mutex")
            .push((run_id, turn_id, iteration, usage));
        let mut state = self.state.lock().expect("state mutex");
        state.next_iteration = iteration
            .checked_add(1)
            .ok_or(StoreError::IterationOverflow)?;
        state.usage = usage;
        let updated = state.clone();
        drop(state);
        if let Some(order) = &self.order {
            order.lock().expect("order mutex").push("complete");
        }
        Ok(updated)
    }

    async fn append_message(
        &self,
        mut envelope: StreamEnvelope,
    ) -> Result<StreamEnvelope, StoreError> {
        let mut history = self.history.lock().expect("history mutex");
        let seq = u64::try_from(history.len())
            .expect("test history length fits u64")
            .checked_add(1)
            .ok_or(StoreError::SequenceOverflow)?;
        envelope.business_seq = Some(seq);
        history.push(envelope.clone());
        drop(history);
        self.state.lock().expect("state mutex").last_seq = seq;
        if let Some(order) = &self.order {
            order.lock().expect("order mutex").push("append");
        }
        Ok(envelope)
    }

    async fn history_page(&self, query: HistoryQuery) -> Result<HistoryPage, StoreError> {
        let history = self.history.lock().expect("history mutex");
        let through_seq = query.through_seq.unwrap_or_else(|| {
            history
                .last()
                .and_then(StreamEnvelope::business_seq)
                .unwrap_or_default()
        });
        let events = history
            .iter()
            .filter(|event| {
                event
                    .business_seq()
                    .is_some_and(|seq| seq > query.after_seq && seq <= through_seq)
            })
            .take(query.limit)
            .cloned()
            .collect::<Vec<_>>();
        let next_front_seq = events
            .last()
            .and_then(StreamEnvelope::business_seq)
            .unwrap_or(query.after_seq);
        Ok(HistoryPage {
            through_seq,
            events,
            next_front_seq,
            has_more: next_front_seq < through_seq,
        })
    }
}

fn test_store() -> Arc<dyn AgentStore> {
    Arc::new(TestStore::idle(test_agent_id()))
}

fn persisted_message(
    seq: u64,
    run_id: RunId,
    turn_id: TurnId,
    message: ChatMessage,
) -> StreamEnvelope {
    StreamEnvelope {
        business_seq: Some(seq),
        run_id,
        timestamp: Utc::now(),
        source: EventSource::Run,
        event: RuntimeEvent::Agent {
            agent_id: test_agent_id(),
            event: AgentEvent::Message { turn_id, message },
        },
        metadata: BTreeMap::new(),
    }
}

fn assistant_tool_call(call_id: &str, arguments: serde_json::Value) -> ChatMessage {
    ChatMessage::assistant("").with_tool_calls(vec![ToolCall {
        call_id: CallId::from(call_id),
        name: "counting".to_owned(),
        arguments,
    }])
}

fn assistant_tool_calls(call_ids: &[&str]) -> ChatMessage {
    ChatMessage::assistant("").with_tool_calls(
        call_ids
            .iter()
            .map(|call_id| ToolCall {
                call_id: CallId::from(*call_id),
                name: "counting".to_owned(),
                arguments: json!({"call_id": call_id}),
            })
            .collect(),
    )
}

async fn assert_invalid_active_turn_history(
    case: &str,
    messages: Vec<ChatMessage>,
    next_iteration: u64,
) {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let last_seq = u64::try_from(messages.len()).expect("test history length fits u64");
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.next_iteration = next_iteration;
    state.last_seq = last_seq;
    let history = messages
        .into_iter()
        .enumerate()
        .map(|(index, message)| {
            let seq = u64::try_from(index)
                .expect("test history index fits u64")
                .checked_add(1)
                .expect("test history sequence does not overflow");
            persisted_message(seq, run_id, turn_id, message)
        })
        .collect();
    let store = Arc::new(TestStore::with_state(state, history));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Pending]));
    let agent = agent_with_store(
        provider.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    let result = agent.resume().await;
    assert!(
        matches!(result, Err(AgentError::InvalidResumeHistory)),
        "{case}: expected invalid resume history, got {result:?}"
    );
    assert!(provider.requests().is_empty(), "{case}: must not call LLM");
}

fn agent_with_store(
    provider: Arc<RecordingProvider>,
    event_bus: Arc<dyn EventStreamBus>,
    store: Arc<dyn AgentStore>,
) -> Agent {
    Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(event_bus)
        .store(store)
        .build()
        .expect("agent should build")
}

#[tokio::test]
async fn resume_rejects_second_active_operation() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.last_seq = 1;
    let store = Arc::new(TestStore::with_state(
        state,
        vec![persisted_message(
            1,
            run_id,
            turn_id,
            ChatMessage::user("continue"),
        )],
    ));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Pending]));
    let agent = agent_with_store(provider, Arc::new(InMemoryEventStreamBus::default()), store);

    assert_eq!(agent.resume().await.expect("resume starts"), run_id);
    assert!(matches!(
        agent.resume().await,
        Err(AgentError::RunAlreadyActive)
    ));
    agent.stop();
}

#[tokio::test]
async fn dropping_resume_while_store_load_is_pending_releases_active_guard() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let store = Arc::new(TestStore::blocking_load(
        test_agent_id(),
        Arc::clone(&entered),
    ));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Pending]));
    let agent = agent_with_store(provider, Arc::new(InMemoryEventStreamBus::default()), store);
    let resuming_agent = agent.clone();
    let resume = tokio::spawn(async move { resuming_agent.resume().await });

    timeout(Duration::from_secs(1), entered.notified())
        .await
        .expect("resume should enter store load");
    resume.abort();
    assert!(
        resume
            .await
            .expect_err("resume task should be aborted")
            .is_cancelled()
    );

    let next = agent.run_turn(ChatMessage::user("new operation")).await;
    assert!(
        !matches!(next, Err(AgentError::RunAlreadyActive)),
        "dropped resume must release the active guard"
    );
    agent.stop();
}

#[tokio::test]
async fn resume_rejects_non_running_state() {
    let state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    let store = Arc::new(TestStore::with_state(state, Vec::new()));
    let agent = agent_with_store(
        Arc::new(RecordingProvider::new(Vec::new())),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.resume().await,
        Err(AgentError::ResumeNotRunning {
            actual: AgentStatus::Idle
        })
    ));
}

#[tokio::test]
async fn resume_rejects_missing_run_identity() {
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.turn_id = Some(TurnId::new());
    let store = Arc::new(TestStore::with_state(state, Vec::new()));
    let agent = agent_with_store(
        Arc::new(RecordingProvider::new(Vec::new())),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.resume().await,
        Err(AgentError::ResumeRunMissing)
    ));
}

#[tokio::test]
async fn resume_rejects_missing_turn_identity() {
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(RunId::new());
    let store = Arc::new(TestStore::with_state(state, Vec::new()));
    let agent = agent_with_store(
        Arc::new(RecordingProvider::new(Vec::new())),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.resume().await,
        Err(AgentError::ResumeTurnMissing)
    ));
}

#[tokio::test]
async fn resume_rejects_persisted_agent_mismatch() {
    let actual = AgentId::new();
    let mut state = AgentState::new(actual, "other-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(RunId::new());
    state.turn_id = Some(TurnId::new());
    let store = Arc::new(TestStore::with_state(state, Vec::new()));
    let agent = agent_with_store(
        Arc::new(RecordingProvider::new(Vec::new())),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.resume().await,
        Err(AgentError::ResumeAgentMismatch { expected, actual: found })
            if expected == test_agent_id() && found == actual
    ));
}

#[tokio::test]
async fn resume_rejects_non_message_history_and_releases_active_guard() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.last_seq = 1;
    let history = vec![StreamEnvelope {
        business_seq: Some(1),
        run_id,
        timestamp: Utc::now(),
        source: EventSource::Run,
        event: RuntimeEvent::Agent {
            agent_id: test_agent_id(),
            event: AgentEvent::Started { turn_id },
        },
        metadata: BTreeMap::new(),
    }];
    let store = Arc::new(TestStore::with_state(state, history));
    let agent = agent_with_store(
        Arc::new(RecordingProvider::new(Vec::new())),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.resume().await,
        Err(AgentError::InvalidResumeHistory)
    ));
    assert!(matches!(
        agent.resume().await,
        Err(AgentError::InvalidResumeHistory)
    ));
}

#[tokio::test]
async fn resume_advances_committed_terminal_assistant_and_finishes_without_llm_request() {
    let run_id = RunId::new();
    let previous_turn_id = TurnId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.last_seq = 4;
    let store = Arc::new(TestStore::with_state(
        state,
        vec![
            persisted_message(
                1,
                run_id,
                previous_turn_id,
                ChatMessage::user("previous turn"),
            ),
            persisted_message(
                2,
                run_id,
                previous_turn_id,
                ChatMessage::assistant("previous answer"),
            ),
            persisted_message(3, run_id, turn_id, ChatMessage::user("active turn")),
            persisted_message(4, run_id, turn_id, ChatMessage::assistant("done")),
        ],
    ));
    let provider = Arc::new(RecordingProvider::new(Vec::new()));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = agent_with_store(provider.clone(), bus.clone(), store.clone());

    assert_eq!(agent.resume().await.expect("resume starts"), run_id);
    let mut events = bus
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe succeeds");
    timeout(Duration::from_secs(1), async {
        while let Some(record) = events.next().await {
            if let RuntimeEvent::Agent {
                event: AgentEvent::Finished { finish_reason, .. },
                ..
            } = record.expect("event is valid").envelope.event
            {
                assert_eq!(finish_reason, "unknown");
                return;
            }
        }
        panic!("expected finished event");
    })
    .await
    .expect("resume finishes");

    assert!(provider.requests().is_empty());
    assert_eq!(
        *store.completed.lock().expect("completed mutex"),
        vec![(run_id, turn_id, 0, TokenUsage::default())]
    );
}

#[tokio::test]
async fn resume_finishes_advanced_terminal_assistant_without_another_advance_or_llm_request() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.next_iteration = 1;
    state.last_seq = 2;
    let store = Arc::new(TestStore::with_state(
        state,
        vec![
            persisted_message(1, run_id, turn_id, ChatMessage::user("active turn")),
            persisted_message(2, run_id, turn_id, ChatMessage::assistant("done")),
        ],
    ));
    let provider = Arc::new(RecordingProvider::new(Vec::new()));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = agent_with_store(provider.clone(), bus.clone(), store.clone());

    assert_eq!(agent.resume().await.expect("resume starts"), run_id);
    let mut events = bus
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe succeeds");
    wait_for_agent_finish(&mut events).await;

    assert!(provider.requests().is_empty());
    assert!(store.completed.lock().expect("completed mutex").is_empty());
}

#[tokio::test]
async fn load_history_restores_finished_conversation_for_next_turn() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Finished;
    state.last_seq = 2;
    let store = Arc::new(TestStore::with_state(
        state,
        vec![
            persisted_message(1, run_id, turn_id, ChatMessage::user("first question")),
            persisted_message(2, run_id, turn_id, ChatMessage::assistant("first answer")),
        ],
    ));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::TextDelta {
                delta: "second answer".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ],
    )]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = agent_with_store(provider.clone(), bus.clone(), store);

    agent.load_history().await.expect("history should load");
    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("second question")).await;
    wait_for_agent_finish(&mut events).await;

    assert_eq!(
        provider.requests()[0].messages,
        vec![
            ChatMessage::system("be helpful"),
            ChatMessage::user("first question"),
            ChatMessage::assistant("first answer"),
            ChatMessage::user("second question"),
        ]
    );
}

#[tokio::test]
async fn load_history_rejects_running_store_before_request() {
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    let store = Arc::new(TestStore::with_state(state, Vec::new()));
    let provider = Arc::new(RecordingProvider::new(Vec::new()));
    let agent = agent_with_store(
        provider.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.load_history().await,
        Err(AgentError::LoadHistoryRunning)
    ));
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn run_turn_rejects_persisted_running_state_before_publishing() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    let store = Arc::new(TestStore::with_state(state, Vec::new()));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Pending]));
    let agent = agent_with_store(
        provider.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    let error = agent
        .run_turn(ChatMessage::user("must resume"))
        .await
        .expect_err("persisted running state must reject a new turn");

    assert!(matches!(
        error,
        AgentError::PersistedRunRequiresResume {
            run_id: actual_run_id,
            turn_id: actual_turn_id,
        } if actual_run_id == run_id && actual_turn_id == turn_id
    ));
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn load_history_rejects_persisted_agent_mismatch_before_request() {
    let actual = AgentId::new();
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Finished;
    state.last_seq = 1;
    let mut persisted =
        persisted_message(1, run_id, turn_id, ChatMessage::user("persisted question"));
    persisted.event = RuntimeEvent::Agent {
        agent_id: actual,
        event: AgentEvent::Message {
            turn_id,
            message: ChatMessage::user("persisted question"),
        },
    };
    let store = Arc::new(TestStore::with_state(state, vec![persisted]));
    let provider = Arc::new(RecordingProvider::new(Vec::new()));
    let agent = agent_with_store(
        provider.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
        store,
    );

    assert!(matches!(
        agent.load_history().await,
        Err(AgentError::ResumeAgentMismatch { expected, actual: found })
            if expected == test_agent_id() && found == actual
    ));
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn load_history_rejects_invalid_history_without_committing_partial_history() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Finished;
    state.last_seq = 1;
    let mut malformed =
        persisted_message(1, run_id, turn_id, ChatMessage::user("persisted question"));
    malformed.business_seq = Some(2);
    let store = Arc::new(TestStore::with_state(state, vec![malformed]));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }],
    )]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = agent_with_store(provider.clone(), bus.clone(), store);

    assert!(matches!(
        agent.load_history().await,
        Err(AgentError::InvalidResumeHistory)
    ));
    assert!(matches!(
        agent.run_turn(ChatMessage::user("new input")).await,
        Err(AgentError::InvalidResumeHistory)
    ));
    assert!(provider.requests().is_empty());
}

#[tokio::test]
async fn resume_rejects_duplicate_assistant_tool_call_ids() {
    assert_invalid_active_turn_history(
        "duplicate assistant call ids",
        vec![
            ChatMessage::user("use tools"),
            assistant_tool_calls(&["call-1", "call-1"]),
        ],
        0,
    )
    .await;
}

#[tokio::test]
async fn resume_rejects_tool_results_that_are_not_an_ordered_prefix() {
    for (case, results) in [
        (
            "unknown result call id",
            vec![ChatMessage::tool("call-x", json!({}))],
        ),
        (
            "duplicate result call id",
            vec![
                ChatMessage::tool("call-1", json!({})),
                ChatMessage::tool("call-1", json!({})),
            ],
        ),
        (
            "out-of-order result call id",
            vec![ChatMessage::tool("call-2", json!({}))],
        ),
    ] {
        let mut messages = vec![
            ChatMessage::user("use tools"),
            assistant_tool_calls(&["call-1", "call-2"]),
        ];
        messages.extend(results);
        assert_invalid_active_turn_history(case, messages, 0).await;
    }
}

#[tokio::test]
async fn resume_rejects_invalid_active_turn_role_grammar() {
    for (case, messages, next_iteration) in [
        (
            "user is not first",
            vec![ChatMessage::assistant("early"), ChatMessage::user("late")],
            1,
        ),
        (
            "extra user",
            vec![ChatMessage::user("first"), ChatMessage::user("second")],
            0,
        ),
        (
            "system after user",
            vec![ChatMessage::user("first"), ChatMessage::system("system")],
            0,
        ),
        (
            "orphan tool",
            vec![
                ChatMessage::user("first"),
                ChatMessage::tool("call-1", json!({})),
            ],
            0,
        ),
        (
            "terminal assistant followed by tool",
            vec![
                ChatMessage::user("first"),
                ChatMessage::assistant("done"),
                ChatMessage::tool("call-1", json!({})),
            ],
            1,
        ),
        (
            "terminal assistant followed by assistant",
            vec![
                ChatMessage::user("first"),
                ChatMessage::assistant("done"),
                ChatMessage::assistant("again"),
            ],
            2,
        ),
        (
            "incomplete tool iteration followed by assistant",
            vec![
                ChatMessage::user("first"),
                assistant_tool_calls(&["call-1"]),
                ChatMessage::assistant("too early"),
            ],
            2,
        ),
    ] {
        assert_invalid_active_turn_history(case, messages, next_iteration).await;
    }
}

#[tokio::test]
async fn resume_executes_only_missing_tool_calls_then_advances_once_and_continues() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.last_seq = 3;
    let assistant = ChatMessage::assistant("").with_tool_calls(vec![
        ToolCall {
            call_id: CallId::from("call-1"),
            name: "counting".to_owned(),
            arguments: json!({"value": 1}),
        },
        ToolCall {
            call_id: CallId::from("call-2"),
            name: "counting".to_owned(),
            arguments: json!({"value": 2}),
        },
    ]);
    let order = Arc::new(Mutex::new(Vec::new()));
    let store = Arc::new(TestStore::with_state_and_order(
        state,
        vec![
            persisted_message(1, run_id, turn_id, ChatMessage::user("use tools")),
            persisted_message(2, run_id, turn_id, assistant),
            persisted_message(
                3,
                run_id,
                turn_id,
                ChatMessage::tool(CallId::from("call-1"), json!({"value": 1})),
            ),
        ],
        Arc::clone(&order),
    ));
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = BuiltinToolRegistry::new(ToolPermissionMode::Allow);
    registry
        .register(
            Arc::new(CountingTool::new(Arc::clone(&calls))),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("tool registers");
    let provider = Arc::new(RecordingProvider::with_order(
        vec![ProviderResponse::Pending],
        Arc::clone(&order),
    ));
    let bus = Arc::new(StoreEventStreamBus::new(
        store.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
    ));
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(registry))
        .event_bus(bus)
        .store(store.clone())
        .build()
        .expect("agent should build");

    assert_eq!(agent.resume().await.expect("resume starts"), run_id);
    let requests = wait_for_request_count(&provider, 1).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(requests[0].messages.iter().any(|message| {
        message.role == ChatRole::Tool && message.tool_call_id == Some(CallId::from("call-1"))
    }));
    assert!(requests[0].messages.iter().any(|message| {
        message.role == ChatRole::Tool && message.tool_call_id == Some(CallId::from("call-2"))
    }));
    assert_eq!(
        *store.completed.lock().expect("completed mutex"),
        vec![(run_id, turn_id, 0, TokenUsage::default())]
    );
    assert_eq!(
        *order.lock().expect("order mutex"),
        vec!["append", "complete", "request"]
    );
    agent.stop();
}

#[tokio::test]
async fn resume_rejects_active_turn_assistant_count_more_than_one_past_frontier() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.last_seq = 3;
    let store = Arc::new(TestStore::with_state(
        state,
        vec![
            persisted_message(1, run_id, turn_id, ChatMessage::user("active turn")),
            persisted_message(2, run_id, turn_id, ChatMessage::assistant("first")),
            persisted_message(3, run_id, turn_id, ChatMessage::assistant("second")),
        ],
    ));
    let provider = Arc::new(RecordingProvider::new(Vec::new()));
    let agent = agent_with_store(
        provider.clone(),
        Arc::new(InMemoryEventStreamBus::default()),
        store.clone(),
    );

    assert!(matches!(
        agent.resume().await,
        Err(AgentError::InvalidResumeHistory)
    ));
    assert!(provider.requests().is_empty());
    assert!(store.completed.lock().expect("completed mutex").is_empty());
}

#[tokio::test]
async fn resume_restores_history_usage_ids_and_next_iteration_without_duplicate_input_events() {
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    let prior_usage = TokenUsage {
        input_tokens: 10,
        output_tokens: 5,
        total_tokens: 15,
    };
    let resumed_usage = TokenUsage {
        input_tokens: 3,
        output_tokens: 2,
        total_tokens: 5,
    };
    let input = ChatMessage::user("persisted input");
    let mut state = AgentState::new(test_agent_id(), "test-agent".to_owned());
    state.status = AgentStatus::Running;
    state.run_id = Some(run_id);
    state.turn_id = Some(turn_id);
    state.next_iteration = 3;
    state.usage = prior_usage;
    state.last_seq = 7;
    let store = Arc::new(TestStore::with_state(
        state,
        vec![
            persisted_message(1, run_id, turn_id, input.clone()),
            persisted_message(
                2,
                run_id,
                turn_id,
                assistant_tool_call("prior-call-1", json!({"iteration": 0})),
            ),
            persisted_message(
                3,
                run_id,
                turn_id,
                ChatMessage::tool("prior-call-1", json!({"iteration": 0})),
            ),
            persisted_message(
                4,
                run_id,
                turn_id,
                assistant_tool_call("prior-call-2", json!({"iteration": 1})),
            ),
            persisted_message(
                5,
                run_id,
                turn_id,
                ChatMessage::tool("prior-call-2", json!({"iteration": 1})),
            ),
            persisted_message(
                6,
                run_id,
                turn_id,
                assistant_tool_call("prior-call-3", json!({"iteration": 2})),
            ),
            persisted_message(
                7,
                run_id,
                turn_id,
                ChatMessage::tool("prior-call-3", json!({"iteration": 2})),
            ),
        ],
    ));
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::TextDelta {
                delta: "resumed".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: Some(resumed_usage),
            },
        ],
    )]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = agent_with_store(provider.clone(), bus.clone(), store.clone());

    assert_eq!(agent.resume().await.expect("resume starts"), run_id);
    assert_eq!(agent.current_run(), Some(run_id));
    assert_eq!(agent.current_turn(), Some(turn_id));
    let requests = wait_for_request_count(&provider, 1).await;
    assert!(requests[0].messages.iter().any(|message| message == &input));

    let mut events = bus
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe succeeds");
    let mut saw_iteration = false;
    timeout(Duration::from_secs(1), async {
        while let Some(record) = events.next().await {
            let envelope = record.expect("event is valid").envelope;
            if envelope.metadata.get("turn_index") == Some(&json!(3)) {
                saw_iteration = true;
            }
            match envelope.event {
                RuntimeEvent::Agent {
                    event: AgentEvent::Started { .. },
                    ..
                } => panic!("resume must not publish another started event"),
                RuntimeEvent::Agent {
                    event:
                        AgentEvent::Message {
                            message:
                                ChatMessage {
                                    role: ChatRole::User,
                                    ..
                                },
                            ..
                        },
                    ..
                } => panic!("resume must not publish another user message"),
                RuntimeEvent::Agent {
                    event: AgentEvent::Finished { usage, .. },
                    ..
                } => {
                    assert_eq!(
                        usage,
                        TokenUsage {
                            input_tokens: 13,
                            output_tokens: 7,
                            total_tokens: 20,
                        }
                    );
                    return;
                }
                _ => {}
            }
        }
        panic!("expected finished event");
    })
    .await
    .expect("resume finishes");

    assert!(saw_iteration);
    assert_eq!(
        *store.completed.lock().expect("completed mutex"),
        vec![(
            run_id,
            turn_id,
            3,
            TokenUsage {
                input_tokens: 13,
                output_tokens: 7,
                total_tokens: 20,
            },
        )]
    );
}

#[derive(Debug)]
struct BlockingToolRegistry {
    entered: Arc<tokio::sync::Notify>,
    spec: ToolSpec,
}

impl BlockingToolRegistry {
    fn new(entered: Arc<tokio::sync::Notify>) -> Self {
        Self {
            entered,
            spec: ToolSpec::builder()
                .name("hang")
                .description("never returns")
                .input_schema(json!({"type": "object"}))
                .build(),
        }
    }
}

#[async_trait]
impl ToolRegistry for BlockingToolRegistry {
    fn register(
        &mut self,
        tool: Arc<dyn stratum_tools::Tool>,
        _tool_kind: ToolKind,
        _danger_level: DangerLevel,
    ) -> Result<(), ToolError> {
        Err(ToolError::DuplicateTool {
            name: tool.spec().name.clone(),
        })
    }

    fn authorization(
        &self,
        _name: &ToolName,
    ) -> Result<Option<(ToolKind, DangerLevel)>, ToolError> {
        Ok(None)
    }

    fn get(&self, _name: &ToolName) -> Option<Arc<dyn stratum_tools::Tool>> {
        None
    }

    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    async fn call(
        &self,
        _name: &ToolName,
        _input: ToolInput,
        _cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        self.entered.notify_waiters();
        pending::<Result<ToolOutput, ToolError>>().await
    }
}

struct CountingTool {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
}

impl CountingTool {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            spec: ToolSpec::builder()
                .name("counting")
                .description("counts executions")
                .input_schema(json!({"type": "object"}))
                .build(),
            calls,
        }
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(
        &self,
        input: ToolInput,
        _cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::new(input.arguments))
    }
}

struct CancellationRecordingTool {
    spec: ToolSpec,
    entered: Arc<tokio::sync::Notify>,
    cancellation_observed: Arc<tokio::sync::Notify>,
    saw_cancelled: Arc<AtomicBool>,
}

impl CancellationRecordingTool {
    fn new(
        entered: Arc<tokio::sync::Notify>,
        cancellation_observed: Arc<tokio::sync::Notify>,
        saw_cancelled: Arc<AtomicBool>,
    ) -> Self {
        Self {
            spec: ToolSpec::builder()
                .name("observe_cancellation")
                .description("records cancellation")
                .input_schema(json!({"type": "object"}))
                .build(),
            entered,
            cancellation_observed,
            saw_cancelled,
        }
    }
}

#[async_trait]
impl Tool for CancellationRecordingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(
        &self,
        _input: ToolInput,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let cancellation = cancellation.clone();
        let cancellation_observed = Arc::clone(&self.cancellation_observed);
        let saw_cancelled = Arc::clone(&self.saw_cancelled);
        tokio::spawn(async move {
            cancellation.cancelled().await;
            saw_cancelled.store(cancellation.is_cancelled(), Ordering::SeqCst);
            cancellation_observed.notify_one();
        });
        self.entered.notify_one();
        pending::<Result<ToolOutput, ToolError>>().await
    }
}

#[derive(Clone, Default)]
struct FailingApprovalBus {
    inner: InMemoryEventStreamBus,
}

#[async_trait]
impl EventStreamBus for FailingApprovalBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        if matches!(
            &envelope.event,
            RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalRequested { .. },
                ..
            }
        ) {
            let source = serde_json::from_str::<serde_json::Value>("{")
                .expect_err("invalid json produces a serde error");
            return Err(EventStreamBusError::Serialize(source));
        }
        self.inner.publish(envelope).await
    }

    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        self.inner.subscribe_agent(agent_id, replay_start).await
    }
}

#[derive(Clone, Default)]
struct BlockingCancelledBus {
    inner: InMemoryEventStreamBus,
    cancelled_entered: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl EventStreamBus for BlockingCancelledBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        if matches!(
            &envelope.event,
            RuntimeEvent::Agent {
                event: AgentEvent::Cancelled { .. },
                ..
            }
        ) {
            self.cancelled_entered.notify_waiters();
            return pending().await;
        }
        self.inner.publish(envelope).await
    }

    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        self.inner.subscribe_agent(agent_id, replay_start).await
    }
}

fn test_agent_id() -> AgentId {
    "0197fcb8-7500-7000-8000-000000000001"
        .parse()
        .expect("agent id parses")
}

async fn wait_for_request_count(provider: &RecordingProvider, count: usize) -> Vec<ChatRequest> {
    timeout(Duration::from_secs(1), async {
        loop {
            let requests = provider.requests();
            if requests.len() >= count {
                return requests;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for provider requests")
}

async fn run_turn_and_subscribe(agent: &Agent, message: ChatMessage) -> (RunId, EventStream) {
    let run_id = agent.run_turn(message).await.expect("run should start");
    let events = agent
        .event_bus()
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe should succeed");
    (run_id, events)
}

async fn wait_for_approval_request(events: &mut EventStream) -> ApprovalId {
    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("approval event")
                .expect("event is valid")
                .envelope;
            if let RuntimeEvent::Agent {
                event:
                    AgentEvent::ToolApprovalRequested {
                        approval_id,
                        agent_name,
                        call_id,
                        tool_name,
                        arguments,
                        tool_kind,
                        danger_level,
                    },
                ..
            } = envelope.event
            {
                assert_eq!(agent_name, "test-agent");
                assert_eq!(call_id, CallId::from("call-1"));
                assert_eq!(tool_name, ToolName::from("counting"));
                assert_eq!(arguments, json!({"message": "hello"}));
                assert_eq!(tool_kind, ToolKind::Write);
                assert_eq!(danger_level, DangerLevel::High);
                return approval_id;
            }
        }
    })
    .await
    .expect("timed out waiting for approval request")
}

async fn wait_for_agent_finish(events: &mut EventStream) {
    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("finished event")
                .expect("event is valid")
                .envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Finished { .. },
                    ..
                }
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for agent finish");
}

fn approval_provider() -> Arc<RecordingProvider> {
    Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Events(vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("counting".to_owned()),
                arguments_delta: r#"{"message":"hello"}"#.to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ]),
        ProviderResponse::Events(vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
    ]))
}

fn approval_agent<P>(
    calls: &Arc<AtomicUsize>,
    provider: Arc<P>,
    event_bus: Arc<dyn EventStreamBus>,
) -> Agent
where
    P: LlmProvider + 'static,
{
    let mut registry = BuiltinToolRegistry::new(ToolPermissionMode::RequireApproval);
    registry
        .register(
            Arc::new(CountingTool::new(Arc::clone(calls))),
            ToolKind::Write,
            DangerLevel::High,
        )
        .expect("tool registers");

    Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(registry))
        .event_bus(event_bus)
        .store(test_store())
        .build()
        .expect("agent builds")
}

#[tokio::test]
async fn approval_allows_exactly_one_tool_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Approve)
        .await
        .expect("approval is accepted");
    wait_for_agent_finish(&mut events).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(agent.current_run(), Some(run_id));
}

#[tokio::test]
async fn approval_rejection_skips_tool_and_returns_structured_result() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = approval_provider();
    let agent = approval_agent(
        &calls,
        Arc::clone(&provider),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Reject)
        .await
        .expect("rejection is accepted");
    wait_for_agent_finish(&mut events).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let requests = provider.requests();
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool
            && message.tool_call_id == Some(CallId::from("call-1"))
            && message.content
                == ChatContent::Json(json!({
                    "error": {
                        "type": "approval_rejected",
                        "message": "user rejected tool call"
                    }
                }))
    }));
}

#[tokio::test]
async fn approval_without_active_turn_returns_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    assert!(matches!(
        agent
            .resolve_tool_approval(ApprovalId::new(), ApprovalDecision::Approve)
            .await,
        Err(AgentError::NoActiveTurn)
    ));
}

#[tokio::test]
async fn approval_before_any_request_returns_not_found_without_waiting() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        Arc::new(PendingChatProvider),
        Arc::new(InMemoryEventStreamBus::default()),
    );
    agent
        .run_turn(ChatMessage::user("wait for provider"))
        .await
        .expect("run starts");
    let approval_id = ApprovalId::new();

    let result = timeout(
        Duration::from_secs(1),
        agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve),
    )
    .await
    .expect("inactive approval should return immediately");

    assert!(matches!(
        result,
        Err(AgentError::ApprovalNotFound { approval_id: actual }) if actual == approval_id
    ));
}

#[tokio::test]
async fn approval_wrong_id_does_not_interrupt_active_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let different_id = ApprovalId::new();
    assert!(matches!(
        agent
            .resolve_tool_approval(different_id, ApprovalDecision::Approve)
            .await,
        Err(AgentError::ApprovalNotFound { approval_id }) if approval_id == different_id
    ));

    agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Approve)
        .await
        .expect("real approval is accepted");
    wait_for_agent_finish(&mut events).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn approval_cancellation_wins_before_tool_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let resolution = agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve);
    tokio::pin!(resolution);
    assert!(matches!(
        futures_util::poll!(&mut resolution),
        Poll::Pending
    ));
    agent.stop();

    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("cancelled event")
                .expect("event is valid")
                .envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Cancelled { .. },
                    ..
                }
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for cancellation");
    assert!(matches!(resolution.await, Err(AgentError::NoActiveTurn)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_clears_active_approval_before_publishing() {
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(BlockingCancelledBus::default());
    let agent = approval_agent(&calls, approval_provider(), bus.clone());
    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let cancelled_entered = bus.cancelled_entered.notified();
    tokio::pin!(cancelled_entered);

    agent.stop();
    timeout(Duration::from_secs(1), &mut cancelled_entered)
        .await
        .expect("cancel publication starts");
    let result = agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Approve)
        .await;

    assert!(matches!(
        result,
        Err(AgentError::ApprovalNotFound { approval_id: actual }) if actual == approval_id
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_request_publish_failure_prevents_tool_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(FailingApprovalBus::default());
    let agent = approval_agent(&calls, approval_provider(), bus.clone());

    let _run_id = agent
        .run_turn(ChatMessage::user("change it"))
        .await
        .expect("run starts");
    let mut events = bus
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe succeeds");

    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("failed event")
                .expect("event is valid")
                .envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Failed { .. },
                    ..
                }
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for failed event");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn duplicate_approval_decisions_execute_tool_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let (first, second) = tokio::join!(
        agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve),
        agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve),
    );
    wait_for_agent_finish(&mut events).await;

    assert!(first.is_ok() ^ second.is_ok());
    let duplicate = if first.is_err() { first } else { second };
    assert!(matches!(
        duplicate,
        Err(AgentError::ApprovalNotFound { approval_id: actual }) if actual == approval_id
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn stream_runs_tool_and_continues_with_tool_result() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Events(vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("echo".to_owned()),
                arguments_delta: r#"{"message":"hello"}"#.to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ]),
        ProviderResponse::Events(vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
    ]));
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("echo should register");
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(registry))
        .event_bus(bus)
        .store(test_store())
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    let mut saw_text_delta = false;
    let mut saw_tool_finished = false;
    let mut saw_tool_message = false;
    let mut saw_second_response_after_tool_message = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Message { message, .. }
                    if message.role == ChatRole::Tool
                        && message.tool_call_id == Some(CallId::from("call-1")) =>
                {
                    assert_eq!(
                        message.content,
                        ChatContent::Json(json!({"message": "hello"}))
                    );
                    saw_tool_message = true;
                }
                AgentEvent::Llm {
                    event:
                        LlmEvent::TextDelta {
                            role: LlmCallRole::Assistant,
                            delta,
                        },
                    ..
                } if delta == "done" => {
                    saw_text_delta = true;
                    saw_second_response_after_tool_message = saw_tool_message;
                }
                AgentEvent::Llm {
                    event: LlmEvent::ToolCallFinished { call_id, result },
                    ..
                } if call_id == CallId::from("call-1") && result == json!({"message": "hello"}) => {
                    saw_tool_finished = true;
                }
                AgentEvent::Finished { .. } => break,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for streamed agent events");

    assert!(saw_text_delta);
    assert!(saw_tool_finished);
    assert!(saw_tool_message);
    assert!(saw_second_response_after_tool_message);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].model,
        "recording:mock-model"
            .parse::<ModelId>()
            .expect("model id parses")
    );
    assert_eq!(
        requests[1].model,
        "recording:mock-model"
            .parse::<ModelId>()
            .expect("model id parses")
    );
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool && message.tool_call_id == Some(CallId::from("call-1"))
    }));
}

#[tokio::test]
async fn stream_publishes_complete_turn_messages_in_order_without_business_sequences() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ],
    )]));
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(InMemoryEventStreamBus::default()))
        .store(test_store())
        .build()
        .expect("agent should build");
    let input = ChatMessage::user("hello");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, input.clone()).await;
    let turn_id = agent.current_turn().expect("turn id should be set");
    let mut state_events = Vec::new();

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            assert_eq!(envelope.business_seq, None);
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };
            if matches!(
                event,
                AgentEvent::Started { .. }
                    | AgentEvent::Message { .. }
                    | AgentEvent::Finished { .. }
            ) {
                let finished = matches!(event, AgentEvent::Finished { .. });
                state_events.push(event);
                if finished {
                    return;
                }
            }
        }
        panic!("expected finished event");
    })
    .await
    .expect("timed out waiting for complete turn events");

    assert_eq!(
        state_events,
        vec![
            AgentEvent::Started { turn_id },
            AgentEvent::Message {
                turn_id,
                message: input,
            },
            AgentEvent::Message {
                turn_id,
                message: ChatMessage::assistant("done"),
            },
            AgentEvent::Finished {
                finish_reason: "stop".to_owned(),
                usage: TokenUsage::default(),
            },
        ]
    );
}

#[tokio::test]
async fn failed_turn_commits_complete_persisted_history_for_next_run() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::StreamResults(vec![
            Ok(ChatStreamEvent::TextDelta {
                delta: "partial".to_owned(),
            }),
            Err(LlmError::UnsupportedCapability("stream failed")),
        ]),
        ProviderResponse::Events(vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }]),
    ]));
    let store = test_store();
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(StoreEventStreamBus::new(
            Arc::clone(&store),
            Arc::new(InMemoryEventStreamBus::default()),
        )))
        .store(store)
        .build()
        .expect("agent should build");

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("failed input")).await;
    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Failed { .. },
                    ..
                }
            ) {
                return;
            }
        }
        panic!("expected failed event");
    })
    .await
    .expect("timed out waiting for failed event");

    timeout(Duration::from_secs(1), async {
        loop {
            match agent.run_turn(ChatMessage::user("fresh input")).await {
                Ok(run_id) => return run_id,
                Err(AgentError::RunAlreadyActive) => sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("unexpected run error: {error}"),
            }
        }
    })
    .await
    .expect("timed out waiting for second run");

    let requests = wait_for_request_count(&provider, 2).await;
    assert!(requests[0].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("failed input".to_owned())
    }));
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("fresh input".to_owned())
    }));
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("failed input".to_owned())
    }));
}

#[tokio::test]
async fn cancelled_turn_context_matches_same_process_and_restart_requests() {
    let same_provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Pending,
        ProviderResponse::Events(vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }]),
    ]));
    let same_store = Arc::new(TestStore::idle(test_agent_id()));
    let same_store_trait: Arc<dyn AgentStore> = same_store.clone();
    let same_agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(same_provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(StoreEventStreamBus::new(
            Arc::clone(&same_store_trait),
            Arc::new(InMemoryEventStreamBus::default()),
        )))
        .store(same_store_trait)
        .build()
        .expect("agent should build");
    same_agent
        .run_turn(ChatMessage::user("cancelled input"))
        .await
        .expect("turn starts");
    wait_for_request_count(&same_provider, 1).await;
    same_agent.stop();
    timeout(Duration::from_secs(1), async {
        loop {
            if same_store.state.lock().expect("state mutex").status == AgentStatus::Cancelled {
                return;
            }
            sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("cancelled state is persisted");
    sleep(Duration::from_millis(10)).await;

    let restart_state = same_store.state.lock().expect("state mutex").clone();
    let restart_history = same_store.history.lock().expect("history mutex").clone();
    let restart_store: Arc<dyn AgentStore> =
        Arc::new(TestStore::with_state(restart_state, restart_history));
    let restart_provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }],
    )]));
    let restart_agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(restart_provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(StoreEventStreamBus::new(
            Arc::clone(&restart_store),
            Arc::new(InMemoryEventStreamBus::default()),
        )))
        .store(restart_store)
        .build()
        .expect("agent should build");
    restart_agent
        .load_history()
        .await
        .expect("restart loads history");

    same_agent
        .run_turn(ChatMessage::user("next input"))
        .await
        .expect("same-process turn starts");
    restart_agent
        .run_turn(ChatMessage::user("next input"))
        .await
        .expect("restart turn starts");
    let same_requests = wait_for_request_count(&same_provider, 2).await;
    let restart_requests = wait_for_request_count(&restart_provider, 1).await;

    assert_eq!(same_requests[1].messages, restart_requests[0].messages);
    assert!(same_requests[1].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("cancelled input".to_owned())
    }));
}

#[tokio::test]
async fn stream_publishes_failure_when_turn_limit_is_reached() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::ToolCalls,
            usage: None,
        }],
    )]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .store(test_store())
        .config(AgentConfig {
            max_turns: 0,
            max_tool_calls_per_turn: 16,
        })
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            if let RuntimeEvent::Agent {
                event: AgentEvent::Failed { error_text, usage },
                ..
            } = envelope.event
            {
                assert!(error_text.contains("turn limit exceeded"));
                assert_eq!(usage, TokenUsage::default());
                return;
            }
        }

        panic!("expected failed event");
    })
    .await
    .expect("timed out waiting for failed event");
}

#[tokio::test]
async fn stream_publishes_cancelled_when_tool_call_hangs() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("hang".to_owned()),
                arguments_delta: "{}".to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: Some(TokenUsage {
                    input_tokens: 3,
                    output_tokens: 5,
                    total_tokens: 8,
                }),
            },
        ],
    )]));
    let entered = Arc::new(tokio::sync::Notify::new());
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BlockingToolRegistry::new(Arc::clone(&entered))))
        .event_bus(bus)
        .store(test_store())
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    entered.notified().await;
    agent.stop();

    let mut saw_cancelled = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Cancelled { usage } => {
                    assert_eq!(
                        usage,
                        TokenUsage {
                            input_tokens: 3,
                            output_tokens: 5,
                            total_tokens: 8,
                        }
                    );
                    saw_cancelled = true;
                    break;
                }
                AgentEvent::Failed { error_text, .. } => {
                    panic!("unexpected failure event: {error_text}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for cancelled event");

    assert!(saw_cancelled);
}

#[tokio::test]
async fn agent_cancellation_reaches_active_tool_token() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("observe_cancellation".to_owned()),
                arguments_delta: "{}".to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ],
    )]));
    let entered = Arc::new(tokio::sync::Notify::new());
    let cancellation_observed = Arc::new(tokio::sync::Notify::new());
    let saw_cancelled = Arc::new(AtomicBool::new(false));
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(
            Arc::new(CancellationRecordingTool::new(
                Arc::clone(&entered),
                Arc::clone(&cancellation_observed),
                Arc::clone(&saw_cancelled),
            )),
            ToolKind::Read,
            DangerLevel::Low,
        )
        .expect("recording tool should register");
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(registry))
        .event_bus(Arc::new(InMemoryEventStreamBus::default()))
        .store(test_store())
        .build()
        .expect("agent should build");

    let (_run_id, _events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    timeout(Duration::from_secs(1), entered.notified())
        .await
        .expect("timed out waiting for tool call");
    agent.stop();
    timeout(Duration::from_secs(1), cancellation_observed.notified())
        .await
        .expect("tool did not observe active cancellation");

    assert!(saw_cancelled.load(Ordering::SeqCst));
}

#[tokio::test]
async fn stream_publishes_tool_failure_and_retries_with_tool_error_message() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Events(vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("missing".to_owned()),
                arguments_delta: "{}".to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ]),
        ProviderResponse::Events(vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
    ]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .store(test_store())
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    let mut failure_text = None;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Llm {
                    event:
                        LlmEvent::ToolCallFailed {
                            call_id,
                            error_text,
                        },
                    ..
                } if call_id == CallId::from("call-1") => {
                    failure_text = Some(error_text);
                }
                AgentEvent::Finished { .. } => break,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for streamed agent events");

    let failure_text = failure_text.expect("expected tool failure event");
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool
            && message.tool_call_id == Some(CallId::from("call-1"))
            && matches!(&message.content, ChatContent::Text(text) if text == &failure_text)
    }));
}
