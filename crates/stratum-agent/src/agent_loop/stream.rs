//! Streaming assistant-response assembly.

use std::collections::{BTreeMap, HashSet};

use futures_util::StreamExt;
use serde_json::{Value, json};
use stratum_core::{AgentTelemetryEvent, CallId, ChatMessage, LlmCallId, TokenUsage, ToolCall};
use stratum_infra::TelemetryEventSink;
use stratum_llm::{ChatStream, ChatStreamEvent};
use tokio_util::sync::CancellationToken;

use super::{AgentLoopError, LlmCallOutput, LoopLimits, ProtocolError};

#[derive(Debug, Default)]
struct PendingToolCall {
    call_id: Option<CallId>,
    name: Option<String>,
    arguments: String,
    unemitted_arguments: String,
}

pub(super) async fn consume_assistant_stream(
    mut stream: ChatStream,
    llm_call_id: &LlmCallId,
    telemetry: &dyn TelemetryEventSink,
    cancellation: &CancellationToken,
    limits: LoopLimits,
    total_usage: &mut TokenUsage,
) -> Result<LlmCallOutput, AgentLoopError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut pending_tool_calls = BTreeMap::<usize, PendingToolCall>::new();
    let (finish_reason, call_usage) = loop {
        let event = tokio::select! {
            biased;
            () = cancellation.cancelled() => return Err(AgentLoopError::Cancelled),
            event = stream.next() => event,
        };
        let Some(event) = event else {
            return Err(ProtocolError::StreamEndedWithoutFinish.into());
        };
        match event? {
            ChatStreamEvent::TextDelta { delta } => {
                ensure_fits(text.len(), delta.len(), limits.max_text_bytes).map_err(|()| {
                    AgentLoopError::TextByteLimitExceeded {
                        maximum: limits.max_text_bytes,
                    }
                })?;
                text.push_str(&delta);
                telemetry.emit(AgentTelemetryEvent::TextDelta {
                    llm_call_id: llm_call_id.clone(),
                    delta,
                });
            }
            ChatStreamEvent::ReasoningDelta { delta } => {
                ensure_fits(reasoning.len(), delta.len(), limits.max_reasoning_bytes).map_err(
                    |()| AgentLoopError::ReasoningByteLimitExceeded {
                        maximum: limits.max_reasoning_bytes,
                    },
                )?;
                reasoning.push_str(&delta);
                telemetry.emit(AgentTelemetryEvent::ReasoningDelta {
                    llm_call_id: llm_call_id.clone(),
                    delta,
                });
            }
            ChatStreamEvent::ToolCallDelta(delta) => {
                if delta.index >= limits.max_tool_calls_per_iteration {
                    return Err(AgentLoopError::ToolCallLimitExceeded {
                        maximum: limits.max_tool_calls_per_iteration,
                    });
                }
                let pending = pending_tool_calls.entry(delta.index).or_default();
                if let Some(call_id) = delta.call_id {
                    if let Some(existing) = &pending.call_id
                        && existing != &call_id
                    {
                        return Err(ProtocolError::ConflictingToolCallId {
                            index: delta.index,
                            existing: existing.clone(),
                            received: call_id,
                        }
                        .into());
                    }
                    pending.call_id = Some(call_id);
                }
                if let Some(name) = delta.name {
                    if let Some(existing) = &pending.name
                        && existing != &name
                    {
                        return Err(ProtocolError::ConflictingToolCallName {
                            index: delta.index,
                            existing: existing.clone(),
                            received: name,
                        }
                        .into());
                    }
                    pending.name = Some(name);
                }
                ensure_fits(
                    pending.arguments.len(),
                    delta.arguments_delta.len(),
                    limits.max_tool_argument_bytes,
                )
                .map_err(|()| AgentLoopError::ToolArgumentByteLimitExceeded {
                    maximum: limits.max_tool_argument_bytes,
                })?;
                pending.arguments.push_str(&delta.arguments_delta);
                if let Some(call_id) = &pending.call_id {
                    let arguments_delta = if pending.unemitted_arguments.is_empty() {
                        delta.arguments_delta
                    } else {
                        pending.unemitted_arguments.push_str(&delta.arguments_delta);
                        std::mem::take(&mut pending.unemitted_arguments)
                    };
                    telemetry.emit(AgentTelemetryEvent::ToolCallDelta {
                        llm_call_id: llm_call_id.clone(),
                        call_id: call_id.clone(),
                        name: pending.name.clone(),
                        arguments_delta,
                    });
                } else {
                    pending.unemitted_arguments.push_str(&delta.arguments_delta);
                }
            }
            ChatStreamEvent::Finished {
                finish_reason,
                usage,
            } => {
                if let Some(event_usage) = usage {
                    add_usage(total_usage, event_usage);
                }
                telemetry.emit(AgentTelemetryEvent::LlmFinished {
                    llm_call_id: llm_call_id.clone(),
                    finish_reason: finish_reason.as_str().to_owned(),
                    usage,
                });
                break (finish_reason, usage);
            }
            _ => {}
        }
    };

    let mut message = ChatMessage::assistant(text);
    if !reasoning.is_empty() {
        message = message.with_reasoning_content(reasoning);
    }
    let tool_calls = finalize_tool_calls(pending_tool_calls)?;
    if !tool_calls.is_empty() {
        message = message.with_tool_calls(tool_calls);
    }
    Ok(LlmCallOutput::new(message, finish_reason, call_usage))
}

fn ensure_fits(current: usize, additional: usize, maximum: usize) -> Result<(), ()> {
    if additional > maximum.saturating_sub(current) {
        Err(())
    } else {
        Ok(())
    }
}

fn finalize_tool_calls(
    pending_tool_calls: BTreeMap<usize, PendingToolCall>,
) -> Result<Vec<ToolCall>, ProtocolError> {
    let mut tool_calls = Vec::with_capacity(pending_tool_calls.len());
    let mut call_ids = HashSet::with_capacity(pending_tool_calls.len());
    let mut expected_index = 0;
    for (index, pending) in pending_tool_calls {
        if index != expected_index {
            return Err(ProtocolError::SparseToolCallIndex {
                expected: expected_index,
                actual: index,
            });
        }
        expected_index = index.saturating_add(1);
        let Some(call_id) = pending.call_id else {
            return Err(ProtocolError::IncompleteToolCall {
                index,
                call_id: None,
            });
        };
        let Some(name) = pending.name else {
            return Err(ProtocolError::IncompleteToolCall {
                index,
                call_id: Some(call_id),
            });
        };
        if !call_ids.insert(call_id.clone()) {
            return Err(ProtocolError::DuplicateToolCallId { call_id });
        }
        let arguments = if pending.arguments.is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Value>(&pending.arguments).map_err(|source| {
                ProtocolError::MalformedToolCallArguments {
                    call_id: call_id.clone(),
                    source,
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

fn add_usage(total: &mut TokenUsage, usage: TokenUsage) {
    total.input_tokens = total.input_tokens.saturating_add(usage.input_tokens);
    total.output_tokens = total.output_tokens.saturating_add(usage.output_tokens);
    total.total_tokens = total.total_tokens.saturating_add(usage.total_tokens);
}
