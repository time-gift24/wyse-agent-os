//! Public agent runtime definitions.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tokio_util::sync::CancellationToken;
use wyse_checkpoint::{CheckpointKind, CheckpointStatus, CheckpointStore};
use wyse_core::{AgentId, ChatMessage, ChatRole, RunId, TokenUsage, TurnId};
use wyse_infra::event_stream_bus::EventStreamBus;
use wyse_llm::LlmProvider;
use wyse_tools::ToolRegistry;

use crate::{AgentError, checkpoint::AgentCheckpointState};

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
    id: AgentId,
    name: String,
    system_prompt: String,
    llm_provider: Arc<dyn LlmProvider>,
    tool_registry: Arc<dyn ToolRegistry>,
    event_bus: Arc<dyn EventStreamBus>,
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    config: AgentConfig,
    history: Arc<Mutex<Vec<ChatMessage>>>,
    next_seq: Arc<Mutex<u64>>,
    usage: Arc<Mutex<TokenUsage>>,
    active: Arc<AtomicBool>,
    current_run_id: Arc<Mutex<Option<RunId>>>,
    current_turn_id: Arc<Mutex<Option<TurnId>>>,
    cancel: Arc<Mutex<Option<CancellationToken>>>,
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
        let mut history = self
            .history
            .lock()
            .expect("agent history mutex should not be poisoned")
            .clone();
        history.push(message);

        let loop_input = crate::r#loop::AgentLoopInput {
            run_id,
            agent_id: self.id,
            agent_name: self.name.clone(),
            system_prompt: self.system_prompt.clone(),
            turn_id,
            history,
            llm_provider: Arc::clone(&self.llm_provider),
            tool_registry: Arc::clone(&self.tool_registry),
            event_bus: Arc::clone(&self.event_bus),
            checkpoint_store: self.checkpoint_store.clone(),
            config: self.config.clone(),
            cancel: cancel.clone(),
            start_seq: 1,
            start_usage: TokenUsage::default(),
        };
        let history = Arc::clone(&self.history);
        let active = Arc::clone(&self.active);

        tokio::spawn(async move {
            if let Ok(new_history) = crate::r#loop::run_agent_loop(loop_input).await {
                *history
                    .lock()
                    .expect("agent history mutex should not be poisoned") = new_history;
            }
            active.store(false, Ordering::SeqCst);
        });

        Ok(run_id)
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

    /// Resumes the current checkpointed turn.
    ///
    /// # Errors
    ///
    /// Returns an error if another run is active or the agent was not restored
    /// from a retryable checkpoint.
    pub async fn resume_turn(&self) -> Result<RunId, AgentError> {
        if self.active.swap(true, Ordering::SeqCst) {
            return Err(AgentError::RunAlreadyActive);
        }

        let Some(run_id) = self.current_run() else {
            self.active.store(false, Ordering::SeqCst);
            return Err(AgentError::CheckpointNotRetryable);
        };
        let Some(turn_id) = self.current_turn() else {
            self.active.store(false, Ordering::SeqCst);
            return Err(AgentError::CheckpointNotRetryable);
        };
        let cancel = CancellationToken::new();
        *self
            .cancel
            .lock()
            .expect("cancel mutex should not be poisoned") = Some(cancel.clone());
        let history = self
            .history
            .lock()
            .expect("agent history mutex should not be poisoned")
            .clone();
        let start_seq = *self
            .next_seq
            .lock()
            .expect("next seq mutex should not be poisoned");
        let start_usage = *self
            .usage
            .lock()
            .expect("usage mutex should not be poisoned");

        let loop_input = crate::r#loop::AgentLoopInput {
            run_id,
            agent_id: self.id,
            agent_name: self.name.clone(),
            system_prompt: self.system_prompt.clone(),
            turn_id,
            history,
            llm_provider: Arc::clone(&self.llm_provider),
            tool_registry: Arc::clone(&self.tool_registry),
            event_bus: Arc::clone(&self.event_bus),
            checkpoint_store: self.checkpoint_store.clone(),
            config: self.config.clone(),
            cancel,
            start_seq,
            start_usage,
        };
        let history = Arc::clone(&self.history);
        let active = Arc::clone(&self.active);

        tokio::spawn(async move {
            if let Ok(new_history) = crate::r#loop::run_agent_loop(loop_input).await {
                *history
                    .lock()
                    .expect("agent history mutex should not be poisoned") = new_history;
            }
            active.store(false, Ordering::SeqCst);
        });

        Ok(run_id)
    }

    fn set_next_seq(&mut self, seq: u64) {
        *self
            .next_seq
            .lock()
            .expect("next seq mutex should not be poisoned") = seq;
    }

    fn set_usage(&mut self, usage: TokenUsage) {
        *self
            .usage
            .lock()
            .expect("usage mutex should not be poisoned") = usage;
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
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
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

    /// Sets the checkpoint store.
    #[must_use]
    pub fn checkpoint_store(mut self, checkpoint_store: Arc<dyn CheckpointStore>) -> Self {
        self.checkpoint_store = Some(checkpoint_store);
        self
    }

    /// Sets runtime config.
    #[must_use]
    pub fn config(mut self, config: AgentConfig) -> Self {
        self.config = Some(config);
        self
    }

    /// Restores an [`Agent`] from a retryable checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when a required builder field is missing, checkpoint
    /// loading fails, or the checkpoint cannot be resumed by this agent.
    pub async fn resume(self, run_id: RunId, turn_id: TurnId) -> Result<Agent, AgentError> {
        let checkpoint_store =
            self.checkpoint_store
                .clone()
                .ok_or(AgentError::MissingBuilderField {
                    field: "checkpoint_store",
                })?;
        let record = match checkpoint_store
            .latest_turn(run_id, turn_id, CheckpointKind::Agent)
            .await?
        {
            Some(record) if record.status == CheckpointStatus::WaitingRetry => record,
            Some(_) | None => return Err(AgentError::CheckpointNotRetryable),
        };
        let checkpoint = AgentCheckpointState::decode(&record.state, record.state_version)?;
        if let Some(expected) = self.id
            && checkpoint.agent_id != expected
        {
            return Err(AgentError::CheckpointAgentMismatch {
                expected,
                actual: checkpoint.agent_id,
            });
        }

        let mut agent = Agent {
            id: checkpoint.agent_id,
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
            checkpoint_store: Some(checkpoint_store),
            config: self.config.unwrap_or_default(),
            history: Arc::new(Mutex::new(checkpoint.history)),
            next_seq: Arc::new(Mutex::new(1)),
            usage: Arc::new(Mutex::new(TokenUsage::default())),
            active: Arc::new(AtomicBool::new(false)),
            current_run_id: Arc::new(Mutex::new(Some(run_id))),
            current_turn_id: Arc::new(Mutex::new(Some(turn_id))),
            cancel: Arc::new(Mutex::new(None)),
        };
        agent.set_next_seq(record.last_seq.saturating_add(1));
        agent.set_usage(checkpoint.usage);
        Ok(agent)
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
            checkpoint_store: self.checkpoint_store,
            config: self.config.unwrap_or_default(),
            history: Arc::new(Mutex::new(Vec::new())),
            next_seq: Arc::new(Mutex::new(1)),
            usage: Arc::new(Mutex::new(TokenUsage::default())),
            active: Arc::new(AtomicBool::new(false)),
            current_run_id: Arc::new(Mutex::new(None)),
            current_turn_id: Arc::new(Mutex::new(None)),
            cancel: Arc::new(Mutex::new(None)),
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
    use wyse_core::{ChatMessage, ModelId, StreamEnvelope};
    use wyse_infra::event_stream_bus::InMemoryEventStreamBus;
    use wyse_llm::{
        ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError,
        LlmProvider, MockLlmProvider,
    };
    use wyse_tools::BuiltinToolRegistry;

    use super::*;

    fn test_agent() -> Agent {
        Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(Arc::new(MockLlmProvider::new()))
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
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
            .build();

        assert!(agent.is_ok());
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
        fn provider_name(&self) -> &str {
            "blocking"
        }

        fn model_id(&self) -> ModelId {
            ModelId::from("mock-model")
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
                .subscribe_run(first_run_id)
                .await
                .expect("event subscription should succeed");
            while let Some(envelope) = events.next().await {
                let StreamEnvelope { event, .. } = envelope.expect("event should be delivered");
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

    #[tokio::test]
    async fn resume_turn_keeps_agent_inactive_when_checkpoint_is_not_retryable() {
        let provider = Arc::new(BlockingStartProvider::new());
        let agent = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(provider)
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
            .build()
            .expect("agent should build");

        let error = agent
            .resume_turn()
            .await
            .expect_err("resume should reject non-resumed agent");
        assert!(matches!(error, AgentError::CheckpointNotRetryable));

        let run_id = agent
            .run_turn(ChatMessage::user("hello"))
            .await
            .expect("run_turn should still start");
        assert_eq!(agent.current_run(), Some(run_id));

        agent.stop();
        timeout(Duration::from_secs(1), async {
            while agent.active.load(Ordering::SeqCst) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("active flag should clear after cancellation");
    }
}
