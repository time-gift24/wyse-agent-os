//! Public agent runtime definitions.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use wyse_core::{
    AgentEvent, AgentId, ApprovalDecision, ApprovalId, ChatMessage, ChatRole, HistoryQuery, RunId,
    RuntimeEvent, TokenUsage, TurnId,
};
use wyse_infra::event_stream_bus::EventStreamBus;
use wyse_llm::LlmProvider;
use wyse_store::{AgentStatus, AgentStore, MAX_HISTORY_PAGE_SIZE};
use wyse_tools::ToolRegistry;

use crate::{AgentError, command::TurnCommand};

/// Runtime tuning for an agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    /// Maximum LLM turns in one run.
    pub max_turns: usize,
    /// Maximum tool calls accepted from one assistant turn.
    pub max_tool_calls_per_turn: usize,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            max_turns: 16,
            max_tool_calls_per_turn: 16,
        }
    }
}

/// Stateful agent that owns conversation history.
#[derive(Clone)]
pub struct Agent {
    pub(crate) id: AgentId,
    pub(crate) name: String,
    pub(crate) system_prompt: String,
    pub(crate) llm_provider: Arc<dyn LlmProvider>,
    pub(crate) tool_registry: Arc<dyn ToolRegistry>,
    pub(crate) event_bus: Arc<dyn EventStreamBus>,
    pub(crate) store: Arc<dyn AgentStore>,
    pub(crate) config: AgentConfig,
    pub(crate) history: Arc<Mutex<Vec<ChatMessage>>>,
    pub(crate) usage: Arc<Mutex<TokenUsage>>,
    pub(crate) active: Arc<AtomicBool>,
    current_run_id: Arc<Mutex<Option<RunId>>>,
    current_turn_id: Arc<Mutex<Option<TurnId>>>,
    pub(crate) cancel: Arc<Mutex<Option<CancellationToken>>>,
    pub(crate) turn_commands: Arc<Mutex<Option<mpsc::Sender<TurnCommand>>>>,
}

struct ActiveGuard<'a> {
    active: &'a AtomicBool,
    armed: bool,
}

struct ResumeState {
    run_id: RunId,
    turn_id: TurnId,
    next_iteration: u64,
    usage: TokenUsage,
    history: Vec<ChatMessage>,
    active_turn_start: usize,
}

impl<'a> ActiveGuard<'a> {
    fn new(active: &'a AtomicBool) -> Self {
        Self {
            active,
            armed: true,
        }
    }

    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for ActiveGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.active.store(false, Ordering::SeqCst);
        }
    }
}

impl Agent {
    /// Creates an agent builder.
    #[must_use]
    pub fn builder() -> AgentBuilder {
        AgentBuilder::default()
    }

    /// Starts one user turn through the agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the input message role is not `User` or another run
    /// is active.
    pub async fn run_turn(&self, message: ChatMessage) -> Result<RunId, AgentError> {
        if message.role != ChatRole::User {
            return Err(AgentError::InvalidInputMessageRole { role: message.role });
        }

        if self.active.swap(true, Ordering::SeqCst) {
            return Err(AgentError::RunAlreadyActive);
        }

        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let cancel = CancellationToken::new();
        *self
            .current_run_id
            .lock()
            .expect("current run mutex should not be poisoned") = Some(run_id);
        *self
            .current_turn_id
            .lock()
            .expect("current turn mutex should not be poisoned") = Some(turn_id);
        *self
            .cancel
            .lock()
            .expect("cancel mutex should not be poisoned") = Some(cancel.clone());
        let (command_tx, command_rx) = mpsc::channel(1);
        *self
            .turn_commands
            .lock()
            .expect("turn command mutex should not be poisoned") = Some(command_tx);
        self.set_usage(TokenUsage::default());
        let agent = self.clone();
        let active = Arc::clone(&self.active);
        let turn_commands = Arc::clone(&self.turn_commands);

        tokio::spawn(async move {
            let _ = agent.run_turn_loop(message, command_rx).await;
            *turn_commands
                .lock()
                .expect("turn command mutex should not be poisoned") = None;
            active.store(false, Ordering::SeqCst);
        });

        Ok(run_id)
    }

    /// Resumes the persisted running turn at its last durable iteration boundary.
    ///
    /// # Errors
    ///
    /// Returns an error when another operation is active, persisted state is not
    /// resumable, history is invalid, or the store cannot be read.
    pub async fn resume(&self) -> Result<RunId, AgentError> {
        if self.active.swap(true, Ordering::SeqCst) {
            return Err(AgentError::RunAlreadyActive);
        }
        let active_guard = ActiveGuard::new(&self.active);

        let resumed = self.initialize_resume().await?;
        let continuation = self.prepare_resume_continuation(
            resumed.history,
            resumed.active_turn_start,
            resumed.next_iteration,
        )?;

        let cancel = CancellationToken::new();
        *self
            .current_run_id
            .lock()
            .expect("current run mutex should not be poisoned") = Some(resumed.run_id);
        *self
            .current_turn_id
            .lock()
            .expect("current turn mutex should not be poisoned") = Some(resumed.turn_id);
        *self
            .cancel
            .lock()
            .expect("cancel mutex should not be poisoned") = Some(cancel);
        let (command_tx, command_rx) = mpsc::channel(1);
        *self
            .turn_commands
            .lock()
            .expect("turn command mutex should not be poisoned") = Some(command_tx);
        self.set_usage(resumed.usage);
        self.commit_history(continuation.history().to_vec());

        let agent = self.clone();
        let active = Arc::clone(&self.active);
        let turn_commands = Arc::clone(&self.turn_commands);
        tokio::spawn(async move {
            let _ = agent
                .continue_resumed_turn_loop(continuation, command_rx)
                .await;
            *turn_commands
                .lock()
                .expect("turn command mutex should not be poisoned") = None;
            active.store(false, Ordering::SeqCst);
        });
        active_guard.disarm();

        Ok(resumed.run_id)
    }

    /// Loads the durable complete message history into this inactive agent.
    ///
    /// # Errors
    ///
    /// Returns an error when an operation is active, the persisted turn must be resumed,
    /// the store identity differs, or the persisted history cannot be read as contiguous
    /// agent messages.
    pub async fn load_history(&self) -> Result<(), AgentError> {
        if self
            .active
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AgentError::RunAlreadyActive);
        }
        let _active_guard = ActiveGuard::new(&self.active);
        let state = self.store.load_agent().await?;
        if state.status == AgentStatus::Running {
            return Err(AgentError::LoadHistoryRunning);
        }
        if state.agent_id != self.id {
            return Err(AgentError::ResumeAgentMismatch {
                expected: self.id,
                actual: state.agent_id,
            });
        }
        let history = self.load_complete_history(state.last_seq).await?;
        self.commit_history(history);
        Ok(())
    }

    async fn load_complete_history(&self, last_seq: u64) -> Result<Vec<ChatMessage>, AgentError> {
        let mut history = Vec::new();
        let mut after_seq = 0;
        while after_seq < last_seq {
            let page = self
                .store
                .history_page(HistoryQuery {
                    after_seq,
                    through_seq: Some(last_seq),
                    limit: MAX_HISTORY_PAGE_SIZE,
                })
                .await?;
            if page.through_seq != last_seq
                || page.events.is_empty()
                || page.next_front_seq <= after_seq
                || page.next_front_seq > last_seq
            {
                return Err(AgentError::InvalidResumeHistory);
            }

            let mut expected_seq = after_seq;
            for envelope in page.events {
                expected_seq = expected_seq
                    .checked_add(1)
                    .ok_or(AgentError::InvalidResumeHistory)?;
                if envelope.business_seq != Some(expected_seq) {
                    return Err(AgentError::InvalidResumeHistory);
                }
                let RuntimeEvent::Agent { agent_id, event } = envelope.event else {
                    return Err(AgentError::InvalidResumeHistory);
                };
                if agent_id != self.id {
                    return Err(AgentError::ResumeAgentMismatch {
                        expected: self.id,
                        actual: agent_id,
                    });
                }
                let AgentEvent::Message { message, .. } = event else {
                    return Err(AgentError::InvalidResumeHistory);
                };
                history.push(message);
            }
            if expected_seq != page.next_front_seq
                || page.has_more != (page.next_front_seq < last_seq)
            {
                return Err(AgentError::InvalidResumeHistory);
            }
            after_seq = page.next_front_seq;
        }

        Ok(history)
    }

    async fn initialize_resume(&self) -> Result<ResumeState, AgentError> {
        let state = self.store.load_agent().await?;
        if state.status != AgentStatus::Running {
            return Err(AgentError::ResumeNotRunning {
                actual: state.status,
            });
        }
        if state.agent_id != self.id {
            return Err(AgentError::ResumeAgentMismatch {
                expected: self.id,
                actual: state.agent_id,
            });
        }
        let run_id = state.run_id.ok_or(AgentError::ResumeRunMissing)?;
        let turn_id = state.turn_id.ok_or(AgentError::ResumeTurnMissing)?;

        let mut history = Vec::new();
        let mut after_seq = 0;
        let mut has_active_user_message = false;
        let mut active_turn_start = None;
        while after_seq < state.last_seq {
            let page = self
                .store
                .history_page(HistoryQuery {
                    after_seq,
                    through_seq: Some(state.last_seq),
                    limit: MAX_HISTORY_PAGE_SIZE,
                })
                .await?;
            if page.through_seq != state.last_seq
                || page.events.is_empty()
                || page.next_front_seq <= after_seq
                || page.next_front_seq > state.last_seq
            {
                return Err(AgentError::InvalidResumeHistory);
            }

            let mut expected_seq = after_seq;
            for envelope in page.events {
                expected_seq = expected_seq
                    .checked_add(1)
                    .ok_or(AgentError::InvalidResumeHistory)?;
                if envelope.business_seq != Some(expected_seq) {
                    return Err(AgentError::InvalidResumeHistory);
                }
                let RuntimeEvent::Agent { agent_id, event } = envelope.event else {
                    return Err(AgentError::InvalidResumeHistory);
                };
                if agent_id != self.id {
                    return Err(AgentError::ResumeAgentMismatch {
                        expected: self.id,
                        actual: agent_id,
                    });
                }
                let AgentEvent::Message {
                    turn_id: message_turn_id,
                    message,
                } = event
                else {
                    return Err(AgentError::InvalidResumeHistory);
                };
                if message_turn_id == turn_id {
                    if active_turn_start.is_none() {
                        active_turn_start = Some(history.len());
                    }
                    if message.role == ChatRole::User {
                        has_active_user_message = true;
                    }
                } else if active_turn_start.is_some() {
                    return Err(AgentError::InvalidResumeHistory);
                }
                history.push(message);
            }
            if expected_seq != page.next_front_seq
                || page.has_more != (page.next_front_seq < state.last_seq)
            {
                return Err(AgentError::InvalidResumeHistory);
            }
            after_seq = page.next_front_seq;
        }

        if !has_active_user_message {
            return Err(AgentError::InvalidResumeHistory);
        }

        Ok(ResumeState {
            run_id,
            turn_id,
            next_iteration: state.next_iteration,
            usage: state.usage,
            history,
            active_turn_start: active_turn_start.ok_or(AgentError::InvalidResumeHistory)?,
        })
    }

    /// Cancels the current run, if any.
    pub fn stop(&self) {
        if let Some(cancel) = self
            .cancel
            .lock()
            .expect("cancel mutex should not be poisoned")
            .as_ref()
        {
            cancel.cancel();
        }
    }

    /// Resolves the active tool approval request.
    ///
    /// # Errors
    ///
    /// Returns an error when no turn is active, the approval id is not active, or
    /// the turn ends before accepting the command.
    pub async fn resolve_tool_approval(
        &self,
        approval_id: ApprovalId,
        decision: ApprovalDecision,
    ) -> Result<(), AgentError> {
        let sender = self
            .turn_commands
            .lock()
            .expect("turn command mutex should not be poisoned")
            .clone()
            .ok_or(AgentError::NoActiveTurn)?;
        let (response, receiver) = oneshot::channel();
        sender
            .send(TurnCommand::ResolveToolApproval {
                approval_id,
                decision,
                response,
            })
            .await
            .map_err(|_| AgentError::NoActiveTurn)?;
        receiver.await.map_err(|_| AgentError::NoActiveTurn)?
    }

    /// Returns the current run id, if one has been started.
    pub fn current_run(&self) -> Option<RunId> {
        *self
            .current_run_id
            .lock()
            .expect("current run mutex should not be poisoned")
    }

    /// Returns the current turn id, if one has been started.
    pub fn current_turn(&self) -> Option<TurnId> {
        *self
            .current_turn_id
            .lock()
            .expect("current turn mutex should not be poisoned")
    }

    /// Returns the configured event bus.
    pub fn event_bus(&self) -> Arc<dyn EventStreamBus> {
        Arc::clone(&self.event_bus)
    }

    fn set_usage(&self, usage: TokenUsage) {
        *self
            .usage
            .lock()
            .expect("usage mutex should not be poisoned") = usage;
    }

    pub(crate) fn commit_history(&self, history: Vec<ChatMessage>) {
        *self
            .history
            .lock()
            .expect("agent history mutex should not be poisoned") = history;
    }
}

/// Builder for [`Agent`].
#[derive(Default)]
pub struct AgentBuilder {
    id: Option<AgentId>,
    name: Option<String>,
    system_prompt: Option<String>,
    llm_provider: Option<Arc<dyn LlmProvider>>,
    tool_registry: Option<Arc<dyn ToolRegistry>>,
    event_bus: Option<Arc<dyn EventStreamBus>>,
    store: Option<Arc<dyn AgentStore>>,
    config: Option<AgentConfig>,
}

impl AgentBuilder {
    /// Sets the agent id.
    #[must_use]
    pub fn id(mut self, id: AgentId) -> Self {
        self.id = Some(id);
        self
    }

    /// Sets the agent name.
    #[must_use]
    pub fn name(mut self, name: impl Into<String>) -> Self {
        self.name = Some(name.into());
        self
    }

    /// Sets the system prompt.
    #[must_use]
    pub fn system_prompt(mut self, system_prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(system_prompt.into());
        self
    }

    /// Sets the LLM provider.
    #[must_use]
    pub fn llm_provider(mut self, llm_provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(llm_provider);
        self
    }

    /// Sets the tool registry.
    #[must_use]
    pub fn tool_registry(mut self, tool_registry: Arc<dyn ToolRegistry>) -> Self {
        self.tool_registry = Some(tool_registry);
        self
    }

    /// Sets the event bus.
    #[must_use]
    pub fn event_bus(mut self, event_bus: Arc<dyn EventStreamBus>) -> Self {
        self.event_bus = Some(event_bus);
        self
    }

    /// Sets the durable agent store.
    #[must_use]
    pub fn store(mut self, store: Arc<dyn AgentStore>) -> Self {
        self.store = Some(store);
        self
    }

    /// Sets runtime config.
    #[must_use]
    pub fn config(mut self, config: AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Builds an [`Agent`].
    ///
    /// # Errors
    ///
    /// Returns an error when a required builder field is missing.
    pub fn build(self) -> Result<Agent, AgentError> {
        Ok(Agent {
            id: self.id.unwrap_or_default(),
            name: self
                .name
                .ok_or(AgentError::MissingBuilderField { field: "name" })?,
            system_prompt: self.system_prompt.ok_or(AgentError::MissingBuilderField {
                field: "system_prompt",
            })?,
            llm_provider: self.llm_provider.ok_or(AgentError::MissingBuilderField {
                field: "llm_provider",
            })?,
            tool_registry: self.tool_registry.ok_or(AgentError::MissingBuilderField {
                field: "tool_registry",
            })?,
            event_bus: self
                .event_bus
                .ok_or(AgentError::MissingBuilderField { field: "event_bus" })?,
            store: self
                .store
                .ok_or(AgentError::MissingBuilderField { field: "store" })?,
            config: self.config.unwrap_or_default(),
            history: Arc::new(Mutex::new(Vec::new())),
            usage: Arc::new(Mutex::new(TokenUsage::default())),
            active: Arc::new(AtomicBool::new(false)),
            current_run_id: Arc::new(Mutex::new(None)),
            current_turn_id: Arc::new(Mutex::new(None)),
            cancel: Arc::new(Mutex::new(None)),
            turn_commands: Arc::new(Mutex::new(None)),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, atomic::Ordering};

    use async_trait::async_trait;
    use futures_util::{StreamExt, stream};
    use tokio::{
        sync::watch,
        time::{Duration, timeout},
    };
    use wyse_core::{ChatMessage, HistoryPage, HistoryQuery, ModelId, ReplayStart, StreamEnvelope};
    use wyse_infra::event_stream_bus::InMemoryEventStreamBus;
    use wyse_llm::{
        ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError,
        LlmProvider, MockLlmProvider,
    };
    use wyse_store::{AgentState, AgentStatus, AgentStore, StoreError};
    use wyse_tools::BuiltinToolRegistry;

    use super::*;

    struct UnitTestStore;

    #[async_trait]
    impl AgentStore for UnitTestStore {
        async fn load_agent(&self) -> Result<AgentState, StoreError> {
            Err(StoreError::AgentMissing)
        }

        async fn update_state(
            &self,
            _status: AgentStatus,
            _run_id: Option<RunId>,
            _turn_id: Option<TurnId>,
            _usage: TokenUsage,
        ) -> Result<AgentState, StoreError> {
            Err(StoreError::AgentMissing)
        }

        async fn complete_iteration(
            &self,
            _run_id: RunId,
            _turn_id: TurnId,
            iteration: u64,
            usage: TokenUsage,
        ) -> Result<AgentState, StoreError> {
            let mut state = AgentState::new(AgentId::new(), "test-agent".to_owned());
            state.next_iteration = iteration
                .checked_add(1)
                .ok_or(StoreError::IterationOverflow)?;
            state.usage = usage;
            Ok(state)
        }

        async fn append_message(
            &self,
            _envelope: StreamEnvelope,
        ) -> Result<StreamEnvelope, StoreError> {
            Err(StoreError::AgentMissing)
        }

        async fn history_page(&self, _query: HistoryQuery) -> Result<HistoryPage, StoreError> {
            Err(StoreError::AgentMissing)
        }
    }

    fn test_store() -> Arc<dyn AgentStore> {
        Arc::new(UnitTestStore)
    }

    fn test_agent() -> Agent {
        Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(Arc::new(MockLlmProvider::new()))
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
            .store(test_store())
            .build()
            .expect("agent should build")
    }

    #[test]
    fn builder_uses_provider_model() {
        let agent = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(Arc::new(MockLlmProvider::new()))
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
            .store(test_store())
            .build();

        assert!(agent.is_ok());
    }

    #[test]
    fn builder_reports_missing_store() {
        let result = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(Arc::new(MockLlmProvider::new()))
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
            .build();
        let error = match result {
            Ok(_) => panic!("store should be required"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AgentError::MissingBuilderField { field: "store" }
        ));
    }

    #[tokio::test]
    async fn run_turn_returns_run_id_and_sets_current_ids() {
        let provider = Arc::new(BlockingStartProvider::new());
        let bus = Arc::new(InMemoryEventStreamBus::default());
        let agent = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(provider.clone())
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(bus)
            .store(test_store())
            .build()
            .expect("agent should build");

        let run_id = agent
            .run_turn(ChatMessage::user("hello"))
            .await
            .expect("run should start");

        assert_eq!(agent.current_run(), Some(run_id));
        assert!(agent.current_turn().is_some());
        agent.stop();
    }

    #[tokio::test]
    async fn stream_rejects_non_user_message() {
        let agent = test_agent();

        let error = match agent.run_turn(ChatMessage::assistant("nope")).await {
            Ok(_) => panic!("assistant message should be rejected"),
            Err(error) => error,
        };

        assert!(matches!(
            error,
            AgentError::InvalidInputMessageRole {
                role: ChatRole::Assistant
            }
        ));
    }

    struct BlockingStartProvider {
        started_tx: watch::Sender<bool>,
        release: watch::Receiver<bool>,
    }

    impl BlockingStartProvider {
        fn new() -> Self {
            let (started_tx, _started_rx) = watch::channel(false);
            let (_release_tx, release_rx) = watch::channel(false);
            Self {
                started_tx,
                release: release_rx,
            }
        }
    }

    #[async_trait]
    impl LlmProvider for BlockingStartProvider {
        fn model_id(&self) -> ModelId {
            "blocking:mock-model".parse().expect("model id parses")
        }

        async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
            Err(LlmError::UnsupportedCapability("chat"))
        }

        async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
            self.started_tx
                .send(true)
                .expect("started signal should send");
            let release = self.release.clone();

            Ok(Box::pin(stream::unfold(
                Some(release),
                |release| async move {
                    let mut release = release?;
                    while !*release.borrow() {
                        if release.changed().await.is_err() {
                            return None;
                        }
                    }
                    Some((
                        Ok(ChatStreamEvent::Finished {
                            finish_reason: FinishReason::Stop,
                            usage: None,
                        }),
                        None,
                    ))
                },
            )))
        }
    }

    #[tokio::test]
    async fn stream_rejects_second_run_while_background_loop_is_active() {
        let (started_tx, started_rx) = watch::channel(false);
        let (release, release_rx) = watch::channel(false);
        let provider = Arc::new(BlockingStartProvider {
            started_tx,
            release: release_rx,
        });
        let agent = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(provider.clone())
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
            .store(test_store())
            .build()
            .expect("agent should build");

        let first_run_id = agent
            .run_turn(ChatMessage::user("hello"))
            .await
            .expect("first run should start");

        timeout(Duration::from_secs(1), async {
            let mut started_rx = started_rx.clone();
            while !*started_rx.borrow() {
                started_rx
                    .changed()
                    .await
                    .expect("started signal should remain open");
            }
        })
        .await
        .expect("background loop should start");

        assert_eq!(agent.current_run(), Some(first_run_id));
        assert!(agent.active.load(Ordering::SeqCst));
        let error = match agent.run_turn(ChatMessage::user("again")).await {
            Ok(_) => panic!("second run should be rejected while loop is active"),
            Err(error) => error,
        };
        assert!(matches!(error, AgentError::RunAlreadyActive));

        release.send(true).expect("release signal should send");
        timeout(Duration::from_secs(1), async {
            let mut events = agent
                .event_bus()
                .subscribe_agent(agent.id, ReplayStart::All)
                .await
                .expect("event subscription should succeed");
            while let Some(envelope) = events.next().await {
                let StreamEnvelope { event, .. } =
                    envelope.expect("event should be delivered").envelope;
                if matches!(
                    event,
                    wyse_core::RuntimeEvent::Agent {
                        event: wyse_core::AgentEvent::Finished { .. },
                        ..
                    }
                ) {
                    break;
                }
            }
        })
        .await
        .expect("background loop should finish");
        timeout(Duration::from_secs(1), async {
            while agent.active.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("active flag should clear after loop completion");
    }
}
