//! Internal streaming loop implementation.

use std::{collections::BTreeMap, sync::Arc};

use chrono::Utc;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use wyse_checkpoint::{CheckpointKind, CheckpointRecord, CheckpointStatus, CheckpointStore};
use wyse_core::{
    AgentEvent, AgentId, CallId, ChatMessage, EventSource, LlmCallId, LlmEvent, RuntimeEvent,
    StreamEnvelope, TokenUsage, ToolCall, ToolName, TurnId,
};
use wyse_infra::event_stream_bus::EventStreamBus;
use wyse_llm::{ChatRequest, ChatStream, ChatStreamEvent, FinishReason, LlmProvider};
use wyse_tools::{ToolInput, ToolRegistry};

use crate::{
    AgentConfig, AgentError,
    checkpoint::{AGENT_CHECKPOINT_STATE_VERSION, AgentCheckpointPhase, AgentCheckpointState},
};

pub(crate) struct AgentLoopInput {
    pub(crate) run_id: wyse_core::RunId,
    pub(crate) agent_id: AgentId,
    pub(crate) agent_name: String,
    pub(crate) system_prompt: String,
    pub(crate) turn_id: TurnId,
    pub(crate) history: Vec<ChatMessage>,
    pub(crate) llm_provider: Arc<dyn LlmProvider>,
    pub(crate) tool_registry: Arc<dyn ToolRegistry>,
    pub(crate) event_bus: Arc<dyn EventStreamBus>,
    pub(crate) checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    pub(crate) config: AgentConfig,
    pub(crate) cancel: CancellationToken,
}

pub(crate) async fn run_agent_loop(
    mut input: AgentLoopInput,
) -> Result<Vec<ChatMessage>, AgentError> {
    let mut seq = 1;
    let mut usage = TokenUsage::default();

    save_checkpoint(
        &input,
        seq,
        CheckpointStatus::Running,
        checkpoint_state(
            &input,
            AgentCheckpointPhase::ReadyForLlm { turn_index: 0 },
            0,
            None,
            usage,
            Vec::new(),
            0,
        ),
    )
    .await?;

    publish_agent_event(&input, &mut seq, AgentEvent::Started, None).await?;

    for turn_index in 0..input.config.max_turns {
        if input.cancel.is_cancelled() {
            publish_cancelled(&input, &mut seq).await?;
            return Err(AgentError::Cancelled);
        }

        save_checkpoint(
            &input,
            seq,
            CheckpointStatus::Running,
            checkpoint_state(
                &input,
                AgentCheckpointPhase::RunningLlm { turn_index },
                0,
                None,
                usage,
                Vec::new(),
                0,
            ),
        )
        .await?;

        let llm_call_id = LlmCallId::from(uuid::Uuid::now_v7().to_string());
        publish_llm_event(
            &input,
            &mut seq,
            turn_index,
            llm_call_id.clone(),
            LlmEvent::Started,
            None,
            None,
        )
        .await?;

        let request = ChatRequest {
            model: input.llm_provider.model_id(),
            messages: request_messages(&input.system_prompt, &input.history),
            tools: input.tool_registry.specs(),
            structured_output: None,
        };
        let stream = match tokio::select! {
            () = input.cancel.cancelled() => {
                publish_cancelled(&input, &mut seq).await?;
                return Err(AgentError::Cancelled);
            }
            result = input.llm_provider.chat_stream(request) => result,
        } {
            Ok(stream) => stream,
            Err(source) => {
                save_checkpoint(
                    &input,
                    seq,
                    CheckpointStatus::WaitingRetry,
                    checkpoint_state(
                        &input,
                        AgentCheckpointPhase::RunningLlm { turn_index },
                        1,
                        Some(source.to_string()),
                        usage,
                        Vec::new(),
                        0,
                    ),
                )
                .await?;
                publish_llm_event(
                    &input,
                    &mut seq,
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
                publish_failed(&input, &mut seq, &error).await?;
                return Err(error);
            }
        };
        let assistant = match consume_assistant_stream(
            &input,
            &mut seq,
            turn_index,
            llm_call_id,
            stream,
            &mut usage,
        )
        .await
        {
            Ok(assistant) => assistant,
            Err(AgentError::Cancelled) => return Err(AgentError::Cancelled),
            Err(error) => {
                if matches!(error, AgentError::Llm { .. }) {
                    save_checkpoint(
                        &input,
                        seq,
                        CheckpointStatus::WaitingRetry,
                        checkpoint_state(
                            &input,
                            AgentCheckpointPhase::RunningLlm { turn_index },
                            1,
                            Some(error.to_string()),
                            usage,
                            Vec::new(),
                            0,
                        ),
                    )
                    .await?;
                }
                publish_failed(&input, &mut seq, &error).await?;
                return Err(error);
            }
        };
        let finish_reason = assistant.finish_reason;
        let message = assistant.message;
        let tool_calls = message.tool_calls.clone();
        input.history.push(message);

        if finish_reason == FinishReason::ToolCalls && !tool_calls.is_empty() {
            save_checkpoint(
                &input,
                seq,
                CheckpointStatus::Running,
                checkpoint_state(
                    &input,
                    AgentCheckpointPhase::RunningTools {
                        turn_index,
                        tool_calls: tool_calls.clone(),
                        next_tool_call_index: 0,
                    },
                    0,
                    None,
                    usage,
                    tool_calls.clone(),
                    0,
                ),
            )
            .await?;

            if tool_calls.len() > input.config.max_tool_calls_per_turn {
                let error = AgentError::ToolCallLimitExceeded {
                    limit: input.config.max_tool_calls_per_turn,
                };
                publish_failed(&input, &mut seq, &error).await?;
                return Err(error);
            }

            for (next_tool_call_index, tool_call) in tool_calls.iter().enumerate() {
                if input.cancel.is_cancelled() {
                    publish_cancelled(&input, &mut seq).await?;
                    return Err(AgentError::Cancelled);
                }
                let tool_message =
                    execute_tool_call(&input, &mut seq, turn_index, tool_call).await?;
                input.history.push(tool_message);
                save_checkpoint(
                    &input,
                    seq,
                    CheckpointStatus::Running,
                    checkpoint_state(
                        &input,
                        AgentCheckpointPhase::RunningTools {
                            turn_index,
                            tool_calls: tool_calls.clone(),
                            next_tool_call_index: next_tool_call_index.saturating_add(1),
                        },
                        0,
                        None,
                        usage,
                        tool_calls.clone(),
                        next_tool_call_index.saturating_add(1),
                    ),
                )
                .await?;
            }
            continue;
        }

        save_checkpoint(
            &input,
            seq,
            CheckpointStatus::Finished,
            checkpoint_state(
                &input,
                AgentCheckpointPhase::Finished {
                    finish_reason: finish_reason_name(finish_reason).to_owned(),
                },
                0,
                None,
                usage,
                Vec::new(),
                0,
            ),
        )
        .await?;

        publish_agent_event(
            &input,
            &mut seq,
            AgentEvent::Finished {
                finish_reason: finish_reason_name(finish_reason).to_owned(),
                usage,
            },
            None,
        )
        .await?;
        return Ok(input.history);
    }

    let error = AgentError::TurnLimitExceeded {
        limit: input.config.max_turns,
    };
    publish_failed(&input, &mut seq, &error).await?;
    Err(error)
}

fn request_messages(system_prompt: &str, history: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 1);
    messages.push(ChatMessage::system(system_prompt));
    messages.extend_from_slice(history);
    messages
}

async fn publish_failed(
    input: &AgentLoopInput,
    seq: &mut u64,
    error: &AgentError,
) -> Result<(), AgentError> {
    publish_agent_event(
        input,
        seq,
        AgentEvent::Failed {
            error_text: error.to_string(),
        },
        None,
    )
    .await
}

async fn publish_cancelled(input: &AgentLoopInput, seq: &mut u64) -> Result<(), AgentError> {
    publish_agent_event(input, seq, AgentEvent::Cancelled, None).await
}

async fn save_checkpoint(
    input: &AgentLoopInput,
    seq: u64,
    status: CheckpointStatus,
    state: AgentCheckpointState,
) -> Result<(), AgentError> {
    let Some(store) = &input.checkpoint_store else {
        return Ok(());
    };

    let record = CheckpointRecord::new(
        input.run_id,
        input.turn_id,
        CheckpointKind::Agent,
        status,
        AGENT_CHECKPOINT_STATE_VERSION,
        state.encode()?,
        seq.saturating_sub(1),
    );
    store.put_latest(record).await?;
    Ok(())
}

fn checkpoint_state(
    input: &AgentLoopInput,
    phase: AgentCheckpointPhase,
    retry_count: u32,
    last_error_text: Option<String>,
    usage: TokenUsage,
    pending_tool_calls: Vec<ToolCall>,
    next_tool_call_index: usize,
) -> AgentCheckpointState {
    AgentCheckpointState {
        agent_id: input.agent_id,
        phase,
        retry_count,
        last_error_text,
        usage,
        history: input.history.clone(),
        pending_tool_calls,
        next_tool_call_index,
    }
}

async fn publish_agent_event(
    input: &AgentLoopInput,
    seq: &mut u64,
    event: AgentEvent,
    extra_metadata: Option<BTreeMap<String, Value>>,
) -> Result<(), AgentError> {
    let mut metadata = BTreeMap::new();
    metadata.insert(
        "agent_name".to_owned(),
        Value::String(input.agent_name.clone()),
    );
    metadata.insert(
        "llm_provider".to_owned(),
        Value::String(input.llm_provider.provider_name().to_owned()),
    );
    let model = input.llm_provider.model_id();
    metadata.insert("model".to_owned(), Value::String(model.as_str().to_owned()));
    metadata.insert(
        "llm".to_owned(),
        Value::String(format!("{}:{}", input.llm_provider.provider_name(), model)),
    );
    if let Some(extra_metadata) = extra_metadata {
        metadata.extend(extra_metadata);
    }

    let envelope = StreamEnvelope {
        run_id: input.run_id,
        seq: *seq,
        timestamp: Utc::now(),
        source: EventSource::Run,
        event: RuntimeEvent::Agent {
            agent_id: input.agent_id,
            event,
        },
        metadata,
    };
    *seq = seq.saturating_add(1);
    input.event_bus.publish(envelope).await?;
    Ok(())
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

async fn consume_assistant_stream(
    input: &AgentLoopInput,
    seq: &mut u64,
    turn_index: usize,
    llm_call_id: LlmCallId,
    mut stream: ChatStream,
    usage: &mut TokenUsage,
) -> Result<AssistantStreamResult, AgentError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut pending_tool_calls = Vec::<PendingToolCall>::new();
    let mut finish_reason = FinishReason::Unknown;

    loop {
        tokio::select! {
            () = input.cancel.cancelled() => {
                publish_cancelled(input, seq).await?;
                return Err(AgentError::Cancelled);
            }
            event = stream.next() => {
                let Some(event) = event else {
                    break;
                };
                match event {
                    Ok(ChatStreamEvent::TextDelta { delta }) => {
                        text.push_str(&delta);
                        publish_llm_event(
                            input,
                            seq,
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
                        publish_llm_event(
                            input,
                            seq,
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
                            pending_tool_calls
                                .resize_with(delta.index + 1, PendingToolCall::default);
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
                        publish_llm_event(
                            input,
                            seq,
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
                            add_usage(usage, event_usage);
                        }
                        publish_llm_event(
                            input,
                            seq,
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
                        publish_llm_event(
                            input,
                            seq,
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
            publish_llm_event(
                input,
                seq,
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
    input: &AgentLoopInput,
    seq: &mut u64,
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

    publish_agent_event(
        input,
        seq,
        AgentEvent::Llm { llm_call_id, event },
        Some(metadata),
    )
    .await
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

async fn execute_tool_call(
    input: &AgentLoopInput,
    seq: &mut u64,
    turn_index: usize,
    tool_call: &ToolCall,
) -> Result<ChatMessage, AgentError> {
    let llm_call_id = LlmCallId::from(uuid::Uuid::now_v7().to_string());
    publish_llm_event(
        input,
        seq,
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
    let tool_result = tokio::select! {
        () = input.cancel.cancelled() => {
            publish_cancelled(input, seq).await?;
            return Err(AgentError::Cancelled);
        }
        result = input.tool_registry.call(
            &name,
            ToolInput::new(tool_call.call_id.clone(), tool_call.arguments.clone()),
        ) => result,
    };

    match tool_result {
        Ok(output) => {
            publish_llm_event(
                input,
                seq,
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
            publish_llm_event(
                input,
                seq,
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

fn add_usage(total: &mut TokenUsage, usage: TokenUsage) {
    total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    total.total_tokens = total.total_tokens.saturating_add(usage.total_tokens);
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
