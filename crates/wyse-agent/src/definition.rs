//! Public agent runtime definitions.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;
use wyse_core::{
    AgentId, ApprovalDecision, ApprovalId, ChatMessage, ChatRole, RunId, TokenUsage, TurnId,
};
use wyse_infra::event_stream_bus::EventStreamBus;
use wyse_llm::LlmProvider;
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
    pub(crate) config: AgentConfig,
    pub(crate) history: Arc<Mutex<Vec<ChatMessage>>>,
    pub(crate) usage: Arc<Mutex<TokenUsage>>,
    pub(crate) active: Arc<AtomicBool>,
    current_run_id: Arc<Mutex<Option<RunId>>>,
    current_turn_id: Arc<Mutex<Option<TurnId>>>,
    pub(crate) cancel: Arc<Mutex<Option<CancellationToken>>>,
    pub(crate) turn_commands: Arc<Mutex<Option<mpsc::Sender<TurnCommand>>>>,
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
    use wyse_core::{ChatMessage, ModelId, ReplayStart, StreamEnvelope};
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
