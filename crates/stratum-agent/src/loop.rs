//! Internal streaming loop implementation.

use std::{
    cmp::Ordering,
    collections::{BTreeMap, HashSet},
};

use chrono::Utc;
use futures_util::StreamExt;
use serde_json::{Value, json};
use stratum_core::{
    AgentEvent, ApprovalDecision, ApprovalId, CallId, ChatMessage, ChatRole, EventSource,
    LlmCallId, LlmEvent, RuntimeEvent, StreamEnvelope, TokenUsage, ToolCall, ToolName,
};
use stratum_llm::{ChatRequest, ChatStream, ChatStreamEvent, FinishReason};
use stratum_tools::ToolInput;
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::{
    Agent, AgentError,
    definition::{ActiveApprovalGuard, ApprovalResolution},
};

pub(crate) struct ResumeContinuation {
    history: Vec<ChatMessage>,
    iteration: u64,
    boundary: ResumeBoundary,
}

impl ResumeContinuation {
    pub(crate) fn history(&self) -> &[ChatMessage] {
        &self.history
    }
}

enum ResumeBoundary {
    Continue,
    Finish {
        advance_iteration: bool,
    },
    ReconcileTools {
        assistant_index: usize,
        next_tool_index: usize,
    },
}

struct ActiveTurnSummary {
    assistant_count: u64,
    end: ActiveTurnEnd,
}

enum ActiveTurnEnd {
    UserOnly,
    Terminal,
    ToolCalls {
        assistant_index: usize,
        answered_count: usize,
        call_count: usize,
    },
}

fn validate_active_turn(active_turn: &[ChatMessage]) -> Result<ActiveTurnSummary, AgentError> {
    if active_turn.first().map(|message| message.role) != Some(ChatRole::User) {
        return Err(AgentError::InvalidResumeHistory);
    }

    let mut cursor = 1;
    let mut assistant_count = 0_usize;
    let mut end = ActiveTurnEnd::UserOnly;
    while cursor < active_turn.len() {
        let assistant_index = cursor;
        let assistant = &active_turn[cursor];
        if assistant.role != ChatRole::Assistant {
            return Err(AgentError::InvalidResumeHistory);
        }
        assistant_count = assistant_count
            .checked_add(1)
            .ok_or(AgentError::InvalidResumeHistory)?;
        cursor = cursor
            .checked_add(1)
            .ok_or(AgentError::InvalidResumeHistory)?;

        let mut call_ids = HashSet::with_capacity(assistant.tool_calls.len());
        if !assistant
            .tool_calls
            .iter()
            .all(|tool_call| call_ids.insert(&tool_call.call_id))
        {
            return Err(AgentError::InvalidResumeHistory);
        }

        if assistant.tool_calls.is_empty() {
            if cursor != active_turn.len() {
                return Err(AgentError::InvalidResumeHistory);
            }
            end = ActiveTurnEnd::Terminal;
            break;
        }

        let mut answered_count = 0_usize;
        while active_turn
            .get(cursor)
            .is_some_and(|message| message.role == ChatRole::Tool)
        {
            let expected_call = assistant
                .tool_calls
                .get(answered_count)
                .ok_or(AgentError::InvalidResumeHistory)?;
            let result = &active_turn[cursor];
            if result.tool_call_id.as_ref() != Some(&expected_call.call_id) {
                return Err(AgentError::InvalidResumeHistory);
            }
            answered_count = answered_count
                .checked_add(1)
                .ok_or(AgentError::InvalidResumeHistory)?;
            cursor = cursor
                .checked_add(1)
                .ok_or(AgentError::InvalidResumeHistory)?;
        }

        end = ActiveTurnEnd::ToolCalls {
            assistant_index,
            answered_count,
            call_count: assistant.tool_calls.len(),
        };
        if answered_count < assistant.tool_calls.len() && cursor != active_turn.len() {
            return Err(AgentError::InvalidResumeHistory);
        }
    }

    Ok(ActiveTurnSummary {
        assistant_count: u64::try_from(assistant_count)
            .map_err(|_| AgentError::InvalidResumeHistory)?,
        end,
    })
}

impl Agent {
    pub(crate) async fn continue_turn_loop(
        self,
        mut history: Vec<ChatMessage>,
        mut iteration: u64,
    ) -> Result<(), AgentError> {
        let turn_id = self.current_turn().expect("turn id should be set");
        loop {
            let turn_index = usize::try_from(iteration)
                .map_err(|_| AgentError::IterationOutOfRange { iteration })?;
            if turn_index >= self.config.max_turns {
                break;
            }

            if self
                .cancel_token()
                .expect("cancel token should be set")
                .is_cancelled()
            {
                self.publish_cancelled().await?;
                return Err(AgentError::Cancelled);
            }

            let llm_call_id = LlmCallId::from(uuid::Uuid::now_v7().to_string());
            self.publish_llm_event(
                turn_index,
                llm_call_id.clone(),
                LlmEvent::Started,
                None,
                None,
            )
            .await?;

            let request = ChatRequest {
                model: self.llm_provider.model_id(),
                messages: request_messages(&self.system_prompt, &history),
                tools: self.tool_registry.specs(),
                structured_output: None,
            };
            let cancel = self.cancel_token().expect("cancel token should be set");
            let stream = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    self.publish_cancelled().await?;
                    return Err(AgentError::Cancelled);
                }
                stream = self.llm_provider.chat_stream(request) => stream,
            };
            let stream = match stream {
                Ok(stream) => stream,
                Err(source) => {
                    self.publish_llm_event(
                        turn_index,
                        llm_call_id,
                        LlmEvent::Failed {
                            error_text: source.to_string(),
                        },
                        None,
                        None,
                    )
                    .await?;
                    let error = AgentError::from(source);
                    self.publish_failed(&error).await?;
                    return Err(error);
                }
            };
            let assistant = match self
                .consume_assistant_stream(turn_index, llm_call_id, stream)
                .await
            {
                Ok(assistant) => assistant,
                Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                Err(error) => {
                    self.publish_failed(&error).await?;
                    return Err(error);
                }
            };
            let finish_reason = assistant.finish_reason;
            let message = assistant.message;
            let tool_calls = message.tool_calls.clone();
            self.publish_required_agent_event(
                AgentEvent::Message {
                    turn_id,
                    message: message.clone(),
                },
                None,
            )
            .await?;
            history.push(message);

            if finish_reason == FinishReason::ToolCalls && !tool_calls.is_empty() {
                if tool_calls.len() > self.config.max_tool_calls_per_turn {
                    let error = AgentError::ToolCallLimitExceeded {
                        limit: self.config.max_tool_calls_per_turn,
                    };
                    self.publish_failed(&error).await?;
                    return Err(error);
                }

                for tool_call in &tool_calls {
                    if self
                        .cancel_token()
                        .expect("cancel token should be set")
                        .is_cancelled()
                    {
                        self.publish_cancelled().await?;
                        return Err(AgentError::Cancelled);
                    }
                    let tool_message = match self.execute_tool_call(turn_index, tool_call).await {
                        Ok(message) => message,
                        Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                        Err(error) => {
                            self.publish_failed(&error).await?;
                            return Err(error);
                        }
                    };
                    self.publish_required_agent_event(
                        AgentEvent::Message {
                            turn_id,
                            message: tool_message.clone(),
                        },
                        None,
                    )
                    .await?;
                    history.push(tool_message);
                }
                self.complete_iteration(iteration).await?;
                iteration = iteration
                    .checked_add(1)
                    .ok_or(AgentError::IterationOutOfRange { iteration })?;
                continue;
            }

            self.complete_iteration(iteration).await?;

            self.publish_required_agent_event(
                AgentEvent::Finished {
                    finish_reason: finish_reason_name(finish_reason).to_owned(),
                    usage: self.current_usage(),
                },
                None,
            )
            .await?;
            self.commit_history(history);
            return Ok(());
        }

        let error = AgentError::TurnLimitExceeded {
            limit: self.config.max_turns,
        };
        self.publish_failed(&error).await?;
        Err(error)
    }

    pub(crate) fn prepare_resume_continuation(
        &self,
        history: Vec<ChatMessage>,
        active_turn_start: usize,
        next_iteration: u64,
    ) -> Result<ResumeContinuation, AgentError> {
        let active_turn = history
            .get(active_turn_start..)
            .ok_or(AgentError::InvalidResumeHistory)?;
        let summary = validate_active_turn(active_turn)?;

        let boundary = match (summary.assistant_count.cmp(&next_iteration), summary.end) {
            (Ordering::Equal, ActiveTurnEnd::UserOnly) => ResumeBoundary::Continue,
            (Ordering::Equal, ActiveTurnEnd::Terminal) => ResumeBoundary::Finish {
                advance_iteration: false,
            },
            (
                Ordering::Equal,
                ActiveTurnEnd::ToolCalls {
                    answered_count,
                    call_count,
                    ..
                },
            ) if answered_count == call_count => ResumeBoundary::Continue,
            (
                Ordering::Greater,
                ActiveTurnEnd::ToolCalls {
                    assistant_index: relative_index,
                    answered_count,
                    ..
                },
            ) if next_iteration.checked_add(1) == Some(summary.assistant_count) => {
                let assistant_index = active_turn_start
                    .checked_add(relative_index)
                    .ok_or(AgentError::InvalidResumeHistory)?;
                ResumeBoundary::ReconcileTools {
                    assistant_index,
                    next_tool_index: answered_count,
                }
            }
            (Ordering::Greater, ActiveTurnEnd::Terminal)
                if next_iteration.checked_add(1) == Some(summary.assistant_count) =>
            {
                ResumeBoundary::Finish {
                    advance_iteration: true,
                }
            }
            _ => return Err(AgentError::InvalidResumeHistory),
        };

        Ok(ResumeContinuation {
            history,
            iteration: next_iteration,
            boundary,
        })
    }

    pub(crate) async fn continue_resumed_turn_loop(
        self,
        mut continuation: ResumeContinuation,
    ) -> Result<(), AgentError> {
        match continuation.boundary {
            ResumeBoundary::Continue => {
                self.continue_turn_loop(continuation.history, continuation.iteration)
                    .await
            }
            ResumeBoundary::Finish { advance_iteration } => {
                if advance_iteration {
                    self.complete_iteration(continuation.iteration).await?;
                }
                self.publish_required_agent_event(
                    AgentEvent::Finished {
                        finish_reason: finish_reason_name(FinishReason::Unknown).to_owned(),
                        usage: self.current_usage(),
                    },
                    None,
                )
                .await?;
                self.commit_history(continuation.history);
                Ok(())
            }
            ResumeBoundary::ReconcileTools {
                assistant_index,
                next_tool_index,
            } => {
                let tool_calls = continuation
                    .history
                    .get(assistant_index)
                    .ok_or(AgentError::InvalidResumeHistory)?
                    .tool_calls
                    .clone();
                if tool_calls.len() > self.config.max_tool_calls_per_turn {
                    let error = AgentError::ToolCallLimitExceeded {
                        limit: self.config.max_tool_calls_per_turn,
                    };
                    self.publish_failed(&error).await?;
                    return Err(error);
                }
                let turn_index = usize::try_from(continuation.iteration).map_err(|_| {
                    AgentError::IterationOutOfRange {
                        iteration: continuation.iteration,
                    }
                })?;
                let turn_id = self.current_turn().expect("turn id should be set");
                let missing_tool_calls = tool_calls
                    .get(next_tool_index..)
                    .ok_or(AgentError::InvalidResumeHistory)?;
                for tool_call in missing_tool_calls {
                    if self
                        .cancel_token()
                        .expect("cancel token should be set")
                        .is_cancelled()
                    {
                        self.publish_cancelled().await?;
                        return Err(AgentError::Cancelled);
                    }
                    let tool_message = match self.execute_tool_call(turn_index, tool_call).await {
                        Ok(message) => message,
                        Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                        Err(error) => {
                            self.publish_failed(&error).await?;
                            return Err(error);
                        }
                    };
                    self.publish_required_agent_event(
                        AgentEvent::Message {
                            turn_id,
                            message: tool_message.clone(),
                        },
                        None,
                    )
                    .await?;
                    continuation.history.push(tool_message);
                }
                self.complete_iteration(continuation.iteration).await?;
                continuation.iteration = continuation.iteration.checked_add(1).ok_or(
                    AgentError::IterationOutOfRange {
                        iteration: continuation.iteration,
                    },
                )?;
                self.continue_turn_loop(continuation.history, continuation.iteration)
                    .await
            }
        }
    }

    async fn complete_iteration(&self, iteration: u64) -> Result<(), AgentError> {
        self.store
            .complete_iteration(
                self.current_run().expect("run id should be set"),
                self.current_turn().expect("turn id should be set"),
                iteration,
                self.current_usage(),
            )
            .await?;
        Ok(())
    }

    fn cancel_token(&self) -> Option<CancellationToken> {
        self.cancel
            .lock()
            .expect("cancel mutex should not be poisoned")
            .clone()
    }

    fn current_usage(&self) -> TokenUsage {
        *self
            .usage
            .lock()
            .expect("usage mutex should not be poisoned")
    }

    fn add_usage(&self, usage: TokenUsage) {
        let mut total = self
            .usage
            .lock()
            .expect("usage mutex should not be poisoned");
        total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
        total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
        total.total_tokens = total.total_tokens.saturating_add(usage.total_tokens);
    }

    async fn publish_cancelled(&self) -> Result<(), AgentError> {
        self.publish_required_agent_event(
            AgentEvent::Cancelled {
                usage: self.current_usage(),
            },
            None,
        )
        .await
    }

    async fn publish_failed(&self, error: &AgentError) -> Result<(), AgentError> {
        self.publish_required_agent_event(
            AgentEvent::Failed {
                error_text: error.to_string(),
                usage: self.current_usage(),
            },
            None,
        )
        .await
    }

    async fn publish_agent_event(
        &self,
        event: AgentEvent,
        extra_metadata: Option<BTreeMap<String, Value>>,
    ) -> Result<(), AgentError> {
        self.publish_agent_event_inner(event, extra_metadata, false)
            .await
    }

    pub(crate) async fn publish_required_agent_event(
        &self,
        event: AgentEvent,
        extra_metadata: Option<BTreeMap<String, Value>>,
    ) -> Result<(), AgentError> {
        self.publish_agent_event_inner(event, extra_metadata, true)
            .await
    }

    async fn publish_agent_event_inner(
        &self,
        event: AgentEvent,
        extra_metadata: Option<BTreeMap<String, Value>>,
        fail_on_publish_error: bool,
    ) -> Result<(), AgentError> {
        let mut metadata = BTreeMap::new();
        metadata.insert("agent_name".to_owned(), Value::String(self.name.clone()));
        let model = self.llm_provider.model_id();
        metadata.insert(
            "llm_provider".to_owned(),
            Value::String(model.provider_name().to_owned()),
        );
        metadata.insert(
            "model".to_owned(),
            Value::String(model.model_name().to_owned()),
        );
        metadata.insert("llm".to_owned(), Value::String(model.as_str().to_owned()));
        if let Some(extra_metadata) = extra_metadata {
            metadata.extend(extra_metadata);
        }

        let envelope = StreamEnvelope {
            business_seq: None,
            run_id: self.current_run().expect("run id should be set"),
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent {
                agent_id: self.id,
                event,
            },
            metadata,
        };
        if let Err(error) = self.event_bus.publish(envelope).await {
            if fail_on_publish_error {
                return Err(AgentError::from(error));
            }
            warn!(
                run_id = %self.current_run().expect("run id should be set"),
                source = %error,
                "failed to publish live agent event"
            );
        }
        Ok(())
    }

    async fn consume_assistant_stream(
        &self,
        turn_index: usize,
        llm_call_id: LlmCallId,
        mut stream: ChatStream,
    ) -> Result<AssistantStreamResult, AgentError> {
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut pending_tool_calls = Vec::<PendingToolCall>::new();
        let mut finish_reason = FinishReason::Unknown;

        loop {
            let cancel = self.cancel_token().expect("cancel token should be set");
            tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    self.publish_cancelled().await?;
                    return Err(AgentError::Cancelled);
                }
                event = stream.next() => {
                    let Some(event) = event else {
                        break;
                    };
                    match event {
                        Ok(ChatStreamEvent::TextDelta { delta }) => {
                            text.push_str(&delta);
                            self.publish_llm_event(
                                turn_index,
                                llm_call_id.clone(),
                                LlmEvent::TextDelta {
                                    role: stratum_core::LlmCallRole::Assistant,
                                    delta,
                                },
                                None,
                                None,
                            )
                            .await?;
                        }
                        Ok(ChatStreamEvent::ReasoningDelta { delta }) => {
                            reasoning.push_str(&delta);
                            self.publish_llm_event(
                                turn_index,
                                llm_call_id.clone(),
                                LlmEvent::ReasoningDelta { delta },
                                None,
                                None,
                            )
                            .await?;
                        }
                        Ok(ChatStreamEvent::ToolCallDelta(delta)) => {
                            if pending_tool_calls.len() <= delta.index {
                                pending_tool_calls.resize_with(delta.index + 1, PendingToolCall::default);
                            }
                            let pending = &mut pending_tool_calls[delta.index];
                            if let Some(call_id) = delta.call_id.clone() {
                                pending.call_id = Some(call_id.clone());
                            }
                            if let Some(name) = delta.name.clone() {
                                pending.name = Some(name.clone());
                            }
                            pending.arguments.push_str(&delta.arguments_delta);
                            let call_id = pending
                                .call_id
                                .clone()
                                .unwrap_or_else(|| CallId::from(format!("tool-call-{}", delta.index)));
                            self.publish_llm_event(
                                turn_index,
                                llm_call_id.clone(),
                                LlmEvent::ToolCallDelta {
                                    call_id: call_id.clone(),
                                    name: pending.name.clone(),
                                    arguments_delta: delta.arguments_delta,
                                },
                                pending.name.as_deref(),
                                Some(&call_id),
                            )
                            .await?;
                        }
                        Ok(ChatStreamEvent::Finished {
                            finish_reason: reason,
                            usage: event_usage,
                        }) => {
                            finish_reason = reason;
                            if let Some(event_usage) = event_usage {
                                self.add_usage(event_usage);
                            }
                            self.publish_llm_event(
                                turn_index,
                                llm_call_id.clone(),
                                LlmEvent::Finished {
                                    finish_reason: finish_reason_name(finish_reason).to_owned(),
                                    usage: event_usage,
                                },
                                None,
                                None,
                            )
                            .await?;
                            break;
                        }
                        Err(source) => {
                            self.publish_llm_event(
                                turn_index,
                                llm_call_id.clone(),
                                LlmEvent::Failed {
                                    error_text: source.to_string(),
                                },
                                None,
                                None,
                            )
                            .await?;
                            return Err(AgentError::from(source));
                        }
                        Ok(_) => {}
                    }
                }
            }
        }

        let mut message = ChatMessage::assistant(text);
        if !reasoning.is_empty() {
            message = message.with_reasoning_content(reasoning);
        }
        let tool_calls = match finalize_tool_calls(pending_tool_calls) {
            Ok(tool_calls) => tool_calls,
            Err(error) => {
                self.publish_llm_event(
                    turn_index,
                    llm_call_id,
                    LlmEvent::Failed {
                        error_text: error.to_string(),
                    },
                    None,
                    None,
                )
                .await?;
                return Err(error);
            }
        };
        if !tool_calls.is_empty() {
            message = message.with_tool_calls(tool_calls);
        }

        Ok(AssistantStreamResult {
            message,
            finish_reason,
        })
    }

    async fn publish_llm_event(
        &self,
        turn_index: usize,
        llm_call_id: LlmCallId,
        event: LlmEvent,
        tool_name: Option<&str>,
        tool_call_id: Option<&CallId>,
    ) -> Result<(), AgentError> {
        let mut metadata = BTreeMap::new();
        metadata.insert("turn_index".to_owned(), json!(turn_index));
        if let Some(tool_name) = tool_name {
            metadata.insert("tool_name".to_owned(), Value::String(tool_name.to_owned()));
        }
        if let Some(tool_call_id) = tool_call_id {
            metadata.insert(
                "tool_call_id".to_owned(),
                Value::String(tool_call_id.as_str().to_owned()),
            );
        }

        self.publish_agent_event(AgentEvent::Llm { llm_call_id, event }, Some(metadata))
            .await
    }

    async fn execute_tool_call(
        &self,
        turn_index: usize,
        tool_call: &ToolCall,
    ) -> Result<ChatMessage, AgentError> {
        let llm_call_id = LlmCallId::from(uuid::Uuid::now_v7().to_string());
        self.publish_llm_event(
            turn_index,
            llm_call_id.clone(),
            LlmEvent::ToolCallStarted {
                call_id: tool_call.call_id.clone(),
                name: Some(tool_call.name.clone()),
            },
            Some(&tool_call.name),
            Some(&tool_call.call_id),
        )
        .await?;

        let name = ToolName::from(tool_call.name.as_str());
        let approval_metadata = match self.tool_registry.authorization(&name) {
            Ok(approval_metadata) => approval_metadata,
            Err(error) => {
                return self
                    .tool_failure_message(turn_index, llm_call_id, tool_call, error.to_string())
                    .await;
            }
        };

        if let Some((tool_kind, danger_level)) = approval_metadata {
            let approval_id = ApprovalId::new();
            let (decision, mut decision_receiver) = oneshot::channel();
            let mut active_approval =
                ActiveApprovalGuard::new(self.active_approval.as_ref(), approval_id, decision);
            self.publish_required_agent_event(
                AgentEvent::ToolApprovalRequested {
                    approval_id,
                    agent_name: self.name.clone(),
                    call_id: tool_call.call_id.clone(),
                    tool_name: name.clone(),
                    arguments: tool_call.arguments.clone(),
                    tool_kind,
                    danger_level,
                },
                None,
            )
            .await?;

            let cancel = self.cancel_token().expect("cancel token should be set");
            let decision = tokio::select! {
                biased;
                () = cancel.cancelled() => {
                    active_approval.clear();
                    self.publish_cancelled().await?;
                    return Err(AgentError::Cancelled);
                }
                resolution = &mut decision_receiver => {
                    let ApprovalResolution { decision, response } =
                        resolution.map_err(|_| AgentError::NoActiveTurn)?;
                    active_approval.clear();
                    let _ = response.send(Ok(()));
                    decision
                }
            };
            drop(active_approval);

            self.publish_agent_event(
                AgentEvent::ToolApprovalResolved {
                    approval_id,
                    decision,
                },
                None,
            )
            .await?;

            match decision {
                ApprovalDecision::Approve => {}
                ApprovalDecision::Reject => {
                    let result = json!({
                        "error": {
                            "type": "approval_rejected",
                            "message": "user rejected tool call"
                        }
                    });
                    self.publish_llm_event(
                        turn_index,
                        llm_call_id,
                        LlmEvent::ToolCallFailed {
                            call_id: tool_call.call_id.clone(),
                            error_text: "user rejected tool call".to_owned(),
                        },
                        Some(&tool_call.name),
                        Some(&tool_call.call_id),
                    )
                    .await?;
                    return Ok(ChatMessage::tool(tool_call.call_id.clone(), result));
                }
                _ => return Err(AgentError::UnsupportedApprovalDecision),
            }
        }

        let future = self.tool_registry.call(
            &name,
            ToolInput::new(tool_call.call_id.clone(), tool_call.arguments.clone()),
        );
        tokio::pin!(future);
        let cancel = self.cancel_token().expect("cancel token should be set");
        let tool_result = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                self.publish_cancelled().await?;
                return Err(AgentError::Cancelled);
            }
            result = &mut future => result,
        };

        match tool_result {
            Ok(output) => {
                self.publish_llm_event(
                    turn_index,
                    llm_call_id,
                    LlmEvent::ToolCallFinished {
                        call_id: tool_call.call_id.clone(),
                        result: output.result.clone(),
                    },
                    Some(&tool_call.name),
                    Some(&tool_call.call_id),
                )
                .await?;
                Ok(ChatMessage::tool(tool_call.call_id.clone(), output.result))
            }
            Err(error) => {
                self.tool_failure_message(turn_index, llm_call_id, tool_call, error.to_string())
                    .await
            }
        }
    }

    async fn tool_failure_message(
        &self,
        turn_index: usize,
        llm_call_id: LlmCallId,
        tool_call: &ToolCall,
        error_text: String,
    ) -> Result<ChatMessage, AgentError> {
        self.publish_llm_event(
            turn_index,
            llm_call_id,
            LlmEvent::ToolCallFailed {
                call_id: tool_call.call_id.clone(),
                error_text: error_text.clone(),
            },
            Some(&tool_call.name),
            Some(&tool_call.call_id),
        )
        .await?;
        let mut message = ChatMessage::text(stratum_core::ChatRole::Tool, error_text);
        message.tool_call_id = Some(tool_call.call_id.clone());
        Ok(message)
    }
}

fn request_messages(system_prompt: &str, history: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.push(ChatMessage::system(system_prompt));
    messages.extend_from_slice(history);
    messages
}

struct AssistantStreamResult {
    message: ChatMessage,
    finish_reason: FinishReason,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    call_id: Option<CallId>,
    name: Option<String>,
    arguments: String,
}

fn finalize_tool_calls(
    pending_tool_calls: Vec<PendingToolCall>,
) -> Result<Vec<ToolCall>, AgentError> {
    let mut tool_calls = Vec::with_capacity(pending_tool_calls.len());
    for (index, pending) in pending_tool_calls.into_iter().enumerate() {
        let call_id = pending
            .call_id
            .unwrap_or_else(|| CallId::from(format!("tool-call-{index}")));
        let Some(name) = pending.name else {
            return Err(AgentError::IncompleteToolCall { call_id });
        };
        let arguments = if pending.arguments.is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Value>(&pending.arguments).map_err(|_| {
                AgentError::IncompleteToolCall {
                    call_id: call_id.clone(),
                }
            })?
        };
        tool_calls.push(ToolCall {
            call_id,
            name,
            arguments,
        });
    }
    Ok(tool_calls)
}

const fn finish_reason_name(finish_reason: FinishReason) -> &'static str {
    match finish_reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Unknown => "unknown",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_reason_names_match_protocol_values() {
        assert_eq!(finish_reason_name(FinishReason::Stop), "stop");
        assert_eq!(finish_reason_name(FinishReason::Length), "length");
        assert_eq!(finish_reason_name(FinishReason::ToolCalls), "tool_calls");
        assert_eq!(
            finish_reason_name(FinishReason::ContentFilter),
            "content_filter"
        );
        assert_eq!(finish_reason_name(FinishReason::Unknown), "unknown");
    }
}
