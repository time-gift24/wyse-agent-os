//! Public agent runtime definitions.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use tokio_util::sync::CancellationToken;
use wyse_checkpoint::CheckpointStore;
use wyse_core::{AgentId, ChatMessage, ChatRole, RunId, TurnId};
use wyse_infra::event_stream_bus::{EventStream, EventStreamBus};
use wyse_llm::LlmProvider;
use wyse_tools::ToolRegistry;

use crate::AgentError;

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

/// Stream handle returned by [`Agent::stream`].
#[non_exhaustive]
pub struct AgentStream {
    /// Run identity for this stream.
    pub run_id: RunId,
    /// Turn identity for this stream.
    pub turn_id: TurnId,
    /// Live event stream for the run.
    pub events: EventStream,
    /// Cancellation handle for this run.
    pub cancel: CancellationToken,
}

/// Stateful agent that owns conversation history.
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
    active: Arc<AtomicBool>,
}

impl Agent {
    /// Creates an agent builder.
    #[must_use]
    pub fn builder() -> AgentBuilder {
        AgentBuilder::default()
    }

    /// Starts streaming one user message through the agent.
    ///
    /// # Errors
    ///
    /// Returns an error if the input message role is not `User`, another run is
    /// active, or subscribing to the event bus fails.
    pub async fn stream(&self, message: ChatMessage) -> Result<AgentStream, AgentError> {
        if message.role != ChatRole::User {
            return Err(AgentError::InvalidInputMessageRole { role: message.role });
        }

        if self.active.swap(true, Ordering::SeqCst) {
            return Err(AgentError::RunAlreadyActive);
        }

        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let events = match self.event_bus.subscribe_run(run_id).await {
            Ok(events) => events,
            Err(source) => {
                self.active.store(false, Ordering::SeqCst);
                return Err(AgentError::from(source));
            }
        };
        let cancel = CancellationToken::new();
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

        Ok(AgentStream {
            run_id,
            turn_id,
            events,
            cancel,
        })
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
            active: Arc::new(AtomicBool::new(false)),
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
    use wyse_core::{ChatMessage, ModelId, RunId};
    use wyse_infra::event_stream_bus::{
        EventStream, EventStreamBus, EventStreamBusError, InMemoryEventStreamBus,
    };
    use wyse_llm::{
        ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError,
        LlmProvider, MockLlmProvider,
    };
    use wyse_tools::BuiltinToolRegistry;

    use serde_json;

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
    async fn stream_returns_turn_id() {
        let agent = test_agent();

        let agent_stream = agent
            .stream(ChatMessage::user("hello"))
            .await
            .expect("stream should start");

        assert_eq!(agent_stream.turn_id.as_uuid().get_version_num(), 7);
        agent_stream.cancel.cancel();
    }

    #[tokio::test]
    async fn stream_rejects_non_user_message() {
        let agent = test_agent();

        let error = match agent.stream(ChatMessage::assistant("nope")).await {
            Ok(_) => panic!("assistant input should be rejected"),
            Err(error) => error,
        };

        assert!(matches!(error, AgentError::InvalidInputMessageRole { .. }));
    }

    struct FailingEventBus;

    #[async_trait]
    impl EventStreamBus for FailingEventBus {
        async fn publish(
            &self,
            _envelope: wyse_core::StreamEnvelope,
        ) -> Result<(), EventStreamBusError> {
            Ok(())
        }

        async fn subscribe_run(&self, _run_id: RunId) -> Result<EventStream, EventStreamBusError> {
            Err(EventStreamBusError::Deserialize(
                serde_json::from_str::<serde_json::Value>("}")
                    .expect_err("invalid json should fail"),
            ))
        }
    }

    #[tokio::test]
    async fn stream_resets_active_on_subscribe_failure() {
        let agent = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(Arc::new(MockLlmProvider::new()))
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(FailingEventBus))
            .build()
            .expect("agent should build");

        let error = match agent.stream(ChatMessage::user("fail me")).await {
            Ok(_) => panic!("subscription failure should return error"),
            Err(error) => error,
        };

        assert!(matches!(error, AgentError::EventBus { .. }));
        assert!(!agent.active.load(Ordering::SeqCst));
    }

    struct BlockingStreamProvider {
        release: watch::Receiver<bool>,
    }

    #[async_trait]
    impl LlmProvider for BlockingStreamProvider {
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
        let (release, release_rx) = watch::channel(false);
        let agent = Agent::builder()
            .name("test-agent")
            .system_prompt("be helpful")
            .llm_provider(Arc::new(BlockingStreamProvider {
                release: release_rx,
            }))
            .tool_registry(Arc::new(BuiltinToolRegistry::default()))
            .event_bus(Arc::new(InMemoryEventStreamBus::default()))
            .build()
            .expect("agent should build");

        let mut stream = agent
            .stream(ChatMessage::user("hello"))
            .await
            .expect("stream should start");

        assert!(agent.active.load(Ordering::SeqCst));
        let error = match agent.stream(ChatMessage::user("again")).await {
            Ok(_) => panic!("second run should be rejected while loop is active"),
            Err(error) => error,
        };
        assert!(matches!(error, AgentError::RunAlreadyActive));

        release.send(true).expect("release signal should send");
        timeout(Duration::from_secs(1), async {
            while let Some(envelope) = stream.events.next().await {
                let envelope = envelope.expect("event should be delivered");
                if matches!(
                    envelope.event,
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
