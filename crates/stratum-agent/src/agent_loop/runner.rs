//! Concrete agent-loop runner.

use std::{collections::HashSet, sync::Arc};

use stratum_core::{
    AgentTelemetryEvent, CallId, ChatMessage, ChatRole, DurableAgentEvent, LlmCallId, TokenUsage,
    ToolCall,
};
use stratum_infra::{DurableEventSink, TelemetryEventSink};
use stratum_llm::{ChatRequest, FinishReason, LlmProvider};
use tokio_util::sync::CancellationToken;

use crate::{ToolApprovalError, ToolExecutor, ToolExecutorError};

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

    /// Runs streamed assistant and sequential tool iterations against committed context.
    ///
    /// # Errors
    ///
    /// Returns an error when cancellation, model streaming, protocol validation, or a required
    /// durable acknowledgement prevents the loop from reaching a terminal boundary.
    ///
    /// # Cancellation safety
    ///
    /// Request cancellation through the supplied [`CancellationToken`], then continue polling
    /// this future to completion. Do not race, drop, or abort it: after
    /// [`DurableAgentEvent::ToolExecutionStarted`] is acknowledged, an external side effect may
    /// be in flight and the loop must finish recording the tool outcome. A durable start without
    /// a corresponding result has an unknown outcome and must not be retried automatically unless
    /// the tool has an explicit idempotency guarantee.
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
        let mut seen_tool_call_ids = committed_tool_call_ids(&context.messages);
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

        for iteration in 0..self.limits.max_iterations {
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
            let finish_reason = assistant.finish_reason;
            let tool_calls = assistant.message.tool_calls.clone();
            let new_tool_call_ids = validate_new_tool_call_ids(&tool_calls, &seen_tool_call_ids)?;

            self.durable_events
                .append(DurableAgentEvent::MessageAppended {
                    message: assistant.message.clone(),
                })
                .await?;
            seen_tool_call_ids.extend(new_tool_call_ids);
            context.messages.push(assistant.message.clone());
            new_messages.push(assistant.message);

            if tool_calls.len() > self.limits.max_tool_calls_per_iteration {
                return Err(AgentLoopError::LimitExceeded {
                    limit: LoopLimit::ToolCallsPerIteration {
                        maximum: self.limits.max_tool_calls_per_iteration,
                    },
                });
            }

            if !tool_calls.is_empty() {
                context.messages.reserve(tool_calls.len());
                new_messages.reserve(tool_calls.len());
                for tool_call in &tool_calls {
                    if cancellation.is_cancelled() {
                        return Err(AgentLoopError::Cancelled);
                    }
                    let message = if finish_reason == FinishReason::Length {
                        truncated_tool_result(tool_call)
                    } else {
                        match self.tool_executor.execute(tool_call, cancellation).await {
                            Ok(outcome) => outcome.into_message(),
                            Err(ToolExecutorError::Durability { source }) => {
                                return Err(AgentLoopError::Durability { source });
                            }
                            Err(ToolExecutorError::Approval {
                                source: ToolApprovalError::Cancelled,
                            }) => return Err(AgentLoopError::Cancelled),
                            Err(source) => return Err(AgentLoopError::ToolExecution { source }),
                        }
                    };
                    self.durable_events
                        .append(DurableAgentEvent::MessageAppended {
                            message: message.clone(),
                        })
                        .await?;
                    context.messages.push(message.clone());
                    new_messages.push(message);
                }
            }

            let iteration = u64::try_from(iteration).unwrap_or(u64::MAX);
            self.durable_events
                .append(DurableAgentEvent::IterationCompleted {
                    iteration,
                    usage: *usage,
                })
                .await?;

            if tool_calls.is_empty() {
                self.durable_events
                    .append(DurableAgentEvent::LoopFinished {
                        finish_reason: finish_reason_name(finish_reason).to_owned(),
                        usage: *usage,
                    })
                    .await?;
                return Ok(LoopOutcome {
                    new_messages,
                    finish_reason,
                    usage: *usage,
                });
            }
        }

        if cancellation.is_cancelled() {
            return Err(AgentLoopError::Cancelled);
        }
        Err(AgentLoopError::LimitExceeded {
            limit: LoopLimit::Iterations {
                maximum: self.limits.max_iterations,
            },
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

fn truncated_tool_result(tool_call: &ToolCall) -> ChatMessage {
    ChatMessage::tool(
        tool_call.call_id.clone(),
        serde_json::json!({
            "error": {
                "code": "tool_call_truncated",
                "message": "tool call was not executed because the model response reached its length limit",
            }
        }),
    )
}

fn committed_tool_call_ids(messages: &[ChatMessage]) -> HashSet<CallId> {
    messages
        .iter()
        .filter(|message| message.role == ChatRole::Assistant)
        .flat_map(|message| message.tool_calls.iter())
        .map(|tool_call| tool_call.call_id.clone())
        .collect()
}

fn validate_new_tool_call_ids(
    tool_calls: &[ToolCall],
    seen: &HashSet<CallId>,
) -> Result<HashSet<CallId>, super::ProtocolError> {
    let mut new_call_ids = HashSet::with_capacity(tool_calls.len());
    for tool_call in tool_calls {
        if seen.contains(&tool_call.call_id) || !new_call_ids.insert(tool_call.call_id.clone()) {
            return Err(super::ProtocolError::DuplicateToolCallId {
                call_id: tool_call.call_id.clone(),
            });
        }
    }
    Ok(new_call_ids)
}
