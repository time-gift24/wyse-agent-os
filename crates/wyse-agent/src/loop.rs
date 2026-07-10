//! Internal streaming loop implementation.

use std::{collections::BTreeMap, sync::atomic::Ordering};

use chrono::Utc;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use wyse_checkpoint::{CheckpointKind, CheckpointRecord, CheckpointStatus};
use wyse_core::{
    AgentEvent, CallId, ChatMessage, EventSource, LlmCallId, LlmEvent, RuntimeEvent,
    StreamEnvelope, TokenUsage, ToolCall, ToolName,
};
use wyse_llm::{ChatRequest, ChatStream, ChatStreamEvent, FinishReason};
use wyse_tools::ToolInput;

use crate::{
    Agent, AgentError,
    checkpoint::{AGENT_CHECKPOINT_STATE_VERSION, encode_checkpoint_payload},
};

impl Agent {
    pub(crate) async fn run_turn_loop(self, input: Option<ChatMessage>) -> Result<(), AgentError> {
        let mut history = self.history_snapshot();
        if let Some(message) = input {
            history.push(message);
        }

        self.save_checkpoint(CheckpointStatus::Running, &history)
            .await?;
        self.publish_agent_event(AgentEvent::Started, None).await?;

        for turn_index in 0..self.config.max_turns {
            if self
                .cancel_token()
                .expect("cancel token should be set")
                .is_cancelled()
            {
                self.publish_cancelled(&history).await?;
                return Err(AgentError::Cancelled);
            }

            self.save_checkpoint(CheckpointStatus::Running, &history)
                .await?;

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
            let stream = match tokio::select! {
                () = cancel.cancelled() => {
                    self.publish_cancelled(&history).await?;
                    return Err(AgentError::Cancelled);
                }
                result = self.llm_provider.chat_stream(request) => result,
            } {
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
                    self.publish_checkpointed_agent_event(
                        CheckpointStatus::WaitingRetry,
                        AgentEvent::Failed {
                            error_text: error.to_string(),
                        },
                        &history,
                    )
                    .await?;
                    return Err(error);
                }
            };
            let assistant = match self
                .consume_assistant_stream(turn_index, llm_call_id, stream, &history)
                .await
            {
                Ok(assistant) => assistant,
                Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
                Err(error) => {
                    if matches!(error, AgentError::Llm { .. }) {
                        self.publish_checkpointed_agent_event(
                            CheckpointStatus::WaitingRetry,
                            AgentEvent::Failed {
                                error_text: error.to_string(),
                            },
                            &history,
                        )
                        .await?;
                    } else {
                        self.save_failed_checkpoint(&error, &history).await?;
                    }
                    return Err(error);
                }
            };
            let finish_reason = assistant.finish_reason;
            let message = assistant.message;
            let tool_calls = message.tool_calls.clone();
            history.push(message);

            if finish_reason == FinishReason::ToolCalls && !tool_calls.is_empty() {
                self.save_checkpoint(CheckpointStatus::Running, &history)
                    .await?;

                if tool_calls.len() > self.config.max_tool_calls_per_turn {
                    let error = AgentError::ToolCallLimitExceeded {
                        limit: self.config.max_tool_calls_per_turn,
                    };
                    self.save_failed_checkpoint(&error, &history).await?;
                    return Err(error);
                }

                for tool_call in &tool_calls {
                    if self
                        .cancel_token()
                        .expect("cancel token should be set")
                        .is_cancelled()
                    {
                        self.publish_cancelled(&history).await?;
                        return Err(AgentError::Cancelled);
                    }
                    let tool_message = self
                        .execute_tool_call(turn_index, tool_call, &history)
                        .await?;
                    history.push(tool_message);
                    self.save_checkpoint(CheckpointStatus::Running, &history)
                        .await?;
                }
                continue;
            }

            let usage = self.current_usage();
            self.publish_checkpointed_agent_event(
                CheckpointStatus::Finished,
                AgentEvent::Finished {
                    finish_reason: finish_reason_name(finish_reason).to_owned(),
                    usage,
                },
                &history,
            )
            .await?;
            self.commit_history(history);
            return Ok(());
        }

        let error = AgentError::TurnLimitExceeded {
            limit: self.config.max_turns,
        };
        self.save_failed_checkpoint(&error, &history).await?;
        Err(error)
    }

    fn cancel_token(&self) -> Option<CancellationToken> {
        self.cancel
            .lock()
            .expect("cancel mutex should not be poisoned")
            .clone()
    }

    fn reserve_event_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::SeqCst)
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

    fn history_snapshot(&self) -> Vec<ChatMessage> {
        self.history
            .lock()
            .expect("agent history mutex should not be poisoned")
            .clone()
    }

    fn commit_history(&self, history: Vec<ChatMessage>) {
        *self
            .history
            .lock()
            .expect("agent history mutex should not be poisoned") = history;
    }

    async fn publish_checkpointed_agent_event(
        &self,
        status: CheckpointStatus,
        event: AgentEvent,
        history: &[ChatMessage],
    ) -> Result<(), AgentError> {
        let seq = self.reserve_event_seq();
        self.save_checkpoint(status, history).await?;
        self.publish_agent_event_at(seq, event, None).await
    }

    async fn publish_cancelled(&self, history: &[ChatMessage]) -> Result<(), AgentError> {
        self.publish_checkpointed_agent_event(
            CheckpointStatus::Cancelled,
            AgentEvent::Cancelled,
            history,
        )
        .await
    }

    async fn save_failed_checkpoint(
        &self,
        error: &AgentError,
        history: &[ChatMessage],
    ) -> Result<(), AgentError> {
        self.publish_checkpointed_agent_event(
            CheckpointStatus::Failed,
            AgentEvent::Failed {
                error_text: error.to_string(),
            },
            history,
        )
        .await
    }

    async fn save_checkpoint(
        &self,
        status: CheckpointStatus,
        history: &[ChatMessage],
    ) -> Result<(), AgentError> {
        let Some(store) = &self.checkpoint_store else {
            return Ok(());
        };

        let state = encode_checkpoint_payload(self.id, self.current_usage(), history)?;
        let record = CheckpointRecord::new(
            self.current_run().expect("run id should be set"),
            self.current_turn().expect("turn id should be set"),
            CheckpointKind::Agent,
            status,
            AGENT_CHECKPOINT_STATE_VERSION,
            state,
            self.seq.load(Ordering::SeqCst).saturating_sub(1),
        );
        store.put_latest(record).await?;
        Ok(())
    }

    async fn publish_agent_event(
        &self,
        event: AgentEvent,
        extra_metadata: Option<BTreeMap<String, Value>>,
    ) -> Result<(), AgentError> {
        let seq = self.reserve_event_seq();
        self.publish_agent_event_at(seq, event, extra_metadata)
            .await
    }

    async fn publish_agent_event_at(
        &self,
        seq: u64,
        event: AgentEvent,
        extra_metadata: Option<BTreeMap<String, Value>>,
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
            run_id: self.current_run().expect("run id should be set"),
            seq,
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent {
                agent_id: self.id,
                event,
            },
            metadata,
        };
        if let Err(error) = self.event_bus.publish(envelope).await {
            warn!(
                run_id = %self.current_run().expect("run id should be set"),
                current_seq = seq,
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
        history: &[ChatMessage],
    ) -> Result<AssistantStreamResult, AgentError> {
        let mut text = String::new();
        let mut reasoning = String::new();
        let mut pending_tool_calls = Vec::<PendingToolCall>::new();
        let mut finish_reason = FinishReason::Unknown;

        loop {
            let cancel = self.cancel_token().expect("cancel token should be set");
            tokio::select! {
                () = cancel.cancelled() => {
                    self.publish_cancelled(history).await?;
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
                                    role: wyse_core::LlmCallRole::Assistant,
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
        history: &[ChatMessage],
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
        let cancel = self.cancel_token().expect("cancel token should be set");
        let tool_result = tokio::select! {
            () = cancel.cancelled() => {
                self.publish_cancelled(history).await?;
                return Err(AgentError::Cancelled);
            }
            result = self.tool_registry.call(
                &name,
                ToolInput::new(tool_call.call_id.clone(), tool_call.arguments.clone()),
            ) => result,
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
                let error_text = error.to_string();
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
                let mut message = ChatMessage::text(wyse_core::ChatRole::Tool, error_text);
                message.tool_call_id = Some(tool_call.call_id.clone());
                Ok(message)
            }
        }
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
