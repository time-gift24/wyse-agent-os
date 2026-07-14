//! Concrete agent-loop runner.

use std::sync::Arc;

use stratum_core::{
    AgentTelemetryEvent, ChatMessage, ChatRole, DurableAgentEvent, LlmCallId, TokenUsage,
};
use stratum_infra::{DurableEventSink, TelemetryEventSink};
use stratum_llm::{ChatRequest, LlmProvider};
use tokio_util::sync::CancellationToken;

use crate::ToolExecutor;

use super::{
    AgentLoopBuildError, AgentLoopError, LoopContext, LoopLimit, LoopLimits, LoopOutcome,
    RequiredAgentLoopField,
    stream::{consume_assistant_stream, finish_reason_name},
};

/// Executes the foundational LLM and tool control flow without owning session state.
pub struct AgentLoop {
    llm_provider: Arc<dyn LlmProvider>,
    tool_executor: ToolExecutor,
    durable_events: Arc<dyn DurableEventSink>,
    telemetry: Arc<dyn TelemetryEventSink>,
    limits: LoopLimits,
}

impl AgentLoop {
    /// Starts construction of an agent loop.
    #[must_use]
    pub fn builder() -> AgentLoopBuilder {
        AgentLoopBuilder::default()
    }

    /// Runs one streamed assistant turn against committed context.
    ///
    /// # Errors
    ///
    /// Returns an error when cancellation, model streaming, protocol validation, or a required
    /// durable acknowledgement prevents the loop from reaching a terminal boundary.
    pub async fn run(
        &self,
        context: LoopContext,
        prompts: Vec<ChatMessage>,
        cancellation: CancellationToken,
    ) -> Result<LoopOutcome, AgentLoopError> {
        if prompts.is_empty() {
            return Err(super::ProtocolError::EmptyPrompts.into());
        }
        if let Some(role) = prompts
            .iter()
            .map(|prompt| prompt.role)
            .find(|role| *role != ChatRole::User)
        {
            return Err(super::ProtocolError::InvalidPromptRole { role }.into());
        }
        if self.limits.max_iterations == 0 {
            return Err(AgentLoopError::LimitExceeded {
                limit: LoopLimit::Iterations { maximum: 0 },
            });
        }

        self.durable_events
            .append(DurableAgentEvent::LoopStarted)
            .await?;
        let mut usage = TokenUsage::default();
        let result = self
            .run_started(context, prompts, &cancellation, &mut usage)
            .await;
        match result {
            Ok(outcome) => Ok(outcome),
            Err(
                error @ (AgentLoopError::Durability { .. }
                | AgentLoopError::TerminalDurability { .. }),
            ) => Err(error),
            Err(error @ AgentLoopError::Cancelled) => Err(self
                .append_terminal(DurableAgentEvent::LoopCancelled { usage }, error)
                .await),
            Err(error) => Err(self
                .append_terminal(
                    DurableAgentEvent::LoopFailed {
                        error_text: error.to_string(),
                        usage,
                    },
                    error,
                )
                .await),
        }
    }

    async fn append_terminal(
        &self,
        event: DurableAgentEvent,
        operation: AgentLoopError,
    ) -> AgentLoopError {
        match self.durable_events.append(event).await {
            Ok(()) => operation,
            Err(source) => AgentLoopError::TerminalDurability {
                operation: Box::new(operation),
                source,
            },
        }
    }

    async fn run_started(
        &self,
        mut context: LoopContext,
        prompts: Vec<ChatMessage>,
        cancellation: &CancellationToken,
        usage: &mut TokenUsage,
    ) -> Result<LoopOutcome, AgentLoopError> {
        if cancellation.is_cancelled() {
            return Err(AgentLoopError::Cancelled);
        }
        let mut new_messages = Vec::with_capacity(prompts.len() + 1);
        for prompt in prompts {
            self.durable_events
                .append(DurableAgentEvent::MessageAppended {
                    message: prompt.clone(),
                })
                .await?;
            context.messages.push(prompt.clone());
            new_messages.push(prompt);
        }

        if cancellation.is_cancelled() {
            return Err(AgentLoopError::Cancelled);
        }
        let llm_call_id = LlmCallId::from(uuid::Uuid::now_v7().to_string());
        self.telemetry
            .emit(AgentTelemetryEvent::LlmStarted {
                llm_call_id: llm_call_id.clone(),
            })
            .await;
        let request = ChatRequest {
            model: self.llm_provider.model_id(),
            messages: request_messages(&context.system_prompt, &context.messages),
            tools: self.tool_executor.specs(),
            structured_output: None,
        };
        let stream = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(AgentLoopError::Cancelled),
            stream = self.llm_provider.chat_stream(request) => stream?,
        };
        let assistant = consume_assistant_stream(
            stream,
            &llm_call_id,
            self.telemetry.as_ref(),
            cancellation,
            self.limits.max_tool_calls_per_iteration,
            usage,
        )
        .await?;

        self.durable_events
            .append(DurableAgentEvent::MessageAppended {
                message: assistant.message.clone(),
            })
            .await?;
        context.messages.push(assistant.message.clone());
        new_messages.push(assistant.message);
        self.durable_events
            .append(DurableAgentEvent::IterationCompleted {
                iteration: 0,
                usage: *usage,
            })
            .await?;
        self.durable_events
            .append(DurableAgentEvent::LoopFinished {
                finish_reason: finish_reason_name(assistant.finish_reason).to_owned(),
                usage: *usage,
            })
            .await?;

        Ok(LoopOutcome {
            new_messages,
            finish_reason: assistant.finish_reason,
            usage: *usage,
        })
    }
}

/// Builder for [`AgentLoop`].
#[derive(Default)]
pub struct AgentLoopBuilder {
    llm_provider: Option<Arc<dyn LlmProvider>>,
    tool_executor: Option<ToolExecutor>,
    durable_events: Option<Arc<dyn DurableEventSink>>,
    telemetry: Option<Arc<dyn TelemetryEventSink>>,
    limits: Option<LoopLimits>,
}

impl AgentLoopBuilder {
    /// Sets the bound model provider.
    #[must_use]
    pub fn llm_provider(mut self, llm_provider: Arc<dyn LlmProvider>) -> Self {
        self.llm_provider = Some(llm_provider);
        self
    }

    /// Sets the approval-aware tool executor.
    #[must_use]
    pub fn tool_executor(mut self, tool_executor: ToolExecutor) -> Self {
        self.tool_executor = Some(tool_executor);
        self
    }

    /// Sets the sink used for required durable events.
    #[must_use]
    pub fn durable_events(mut self, durable_events: Arc<dyn DurableEventSink>) -> Self {
        self.durable_events = Some(durable_events);
        self
    }

    /// Sets the best-effort telemetry sink.
    #[must_use]
    pub fn telemetry(mut self, telemetry: Arc<dyn TelemetryEventSink>) -> Self {
        self.telemetry = Some(telemetry);
        self
    }

    /// Sets safety limits for one run.
    #[must_use]
    pub const fn limits(mut self, limits: LoopLimits) -> Self {
        self.limits = Some(limits);
        self
    }

    /// Builds the agent loop.
    ///
    /// # Errors
    ///
    /// Returns [`AgentLoopBuildError::MissingField`] for the first required field not supplied.
    pub fn build(self) -> Result<AgentLoop, AgentLoopBuildError> {
        Ok(AgentLoop {
            llm_provider: self.llm_provider.ok_or(AgentLoopBuildError::MissingField {
                field: RequiredAgentLoopField::LlmProvider,
            })?,
            tool_executor: self
                .tool_executor
                .ok_or(AgentLoopBuildError::MissingField {
                    field: RequiredAgentLoopField::ToolExecutor,
                })?,
            durable_events: self
                .durable_events
                .ok_or(AgentLoopBuildError::MissingField {
                    field: RequiredAgentLoopField::DurableEvents,
                })?,
            telemetry: self.telemetry.ok_or(AgentLoopBuildError::MissingField {
                field: RequiredAgentLoopField::Telemetry,
            })?,
            limits: self.limits.ok_or(AgentLoopBuildError::MissingField {
                field: RequiredAgentLoopField::Limits,
            })?,
        })
    }
}

fn request_messages(system_prompt: &str, history: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.push(ChatMessage::system(system_prompt));
    messages.extend_from_slice(history);
    messages
}
