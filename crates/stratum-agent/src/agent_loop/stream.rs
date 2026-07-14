//! Streaming assistant-response assembly.

use futures_util::StreamExt;
use serde_json::{Value, json};
use stratum_core::{AgentTelemetryEvent, CallId, ChatMessage, LlmCallId, TokenUsage, ToolCall};
use stratum_infra::TelemetryEventSink;
use stratum_llm::{ChatStream, ChatStreamEvent, FinishReason};
use tokio_util::sync::CancellationToken;

use super::{AgentLoopError, ProtocolError};

pub(super) struct AssistantStreamResult {
    pub(super) message: ChatMessage,
    pub(super) finish_reason: FinishReason,
    pub(super) usage: TokenUsage,
}

#[derive(Debug, Default)]
struct PendingToolCall {
    call_id: Option<CallId>,
    name: Option<String>,
    arguments: String,
}

pub(super) async fn consume_assistant_stream(
    mut stream: ChatStream,
    llm_call_id: &LlmCallId,
    telemetry: &dyn TelemetryEventSink,
    cancellation: &CancellationToken,
) -> Result<AssistantStreamResult, AgentLoopError> {
    let mut text = String::new();
    let mut reasoning = String::new();
    let mut pending_tool_calls = Vec::<PendingToolCall>::new();
    let (finish_reason, usage) = loop {
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
                text.push_str(&delta);
                telemetry
                    .emit(AgentTelemetryEvent::TextDelta {
                        llm_call_id: llm_call_id.clone(),
                        delta,
                    })
                    .await;
            }
            ChatStreamEvent::ReasoningDelta { delta } => {
                reasoning.push_str(&delta);
                telemetry
                    .emit(AgentTelemetryEvent::ReasoningDelta {
                        llm_call_id: llm_call_id.clone(),
                        delta,
                    })
                    .await;
            }
            ChatStreamEvent::ToolCallDelta(delta) => {
                if pending_tool_calls.len() <= delta.index {
                    pending_tool_calls.resize_with(delta.index + 1, PendingToolCall::default);
                }
                let pending = &mut pending_tool_calls[delta.index];
                if let Some(call_id) = delta.call_id {
                    pending.call_id = Some(call_id);
                }
                if let Some(name) = delta.name {
                    pending.name = Some(name);
                }
                pending.arguments.push_str(&delta.arguments_delta);
                let call_id = pending
                    .call_id
                    .clone()
                    .unwrap_or_else(|| CallId::from(format!("tool-call-{}", delta.index)));
                telemetry
                    .emit(AgentTelemetryEvent::ToolCallDelta {
                        llm_call_id: llm_call_id.clone(),
                        call_id,
                        name: pending.name.clone(),
                        arguments_delta: delta.arguments_delta,
                    })
                    .await;
            }
            ChatStreamEvent::Finished {
                finish_reason,
                usage,
            } => break (finish_reason, usage),
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
    let aggregate_usage = usage.unwrap_or_default();
    telemetry
        .emit(AgentTelemetryEvent::LlmFinished {
            llm_call_id: llm_call_id.clone(),
            finish_reason: finish_reason_name(finish_reason).to_owned(),
            usage,
        })
        .await;

    Ok(AssistantStreamResult {
        message,
        finish_reason,
        usage: aggregate_usage,
    })
}

fn finalize_tool_calls(
    pending_tool_calls: Vec<PendingToolCall>,
) -> Result<Vec<ToolCall>, ProtocolError> {
    let mut tool_calls = Vec::with_capacity(pending_tool_calls.len());
    for (index, pending) in pending_tool_calls.into_iter().enumerate() {
        let call_id = pending
            .call_id
            .unwrap_or_else(|| CallId::from(format!("tool-call-{index}")));
        let Some(name) = pending.name else {
            return Err(ProtocolError::IncompleteToolCall { call_id });
        };
        let arguments = if pending.arguments.is_empty() {
            json!({})
        } else {
            serde_json::from_str::<Value>(&pending.arguments).map_err(|_| {
                ProtocolError::IncompleteToolCall {
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

pub(super) const fn finish_reason_name(finish_reason: FinishReason) -> &'static str {
    match finish_reason {
        FinishReason::Stop => "stop",
        FinishReason::Length => "length",
        FinishReason::ToolCalls => "tool_calls",
        FinishReason::ContentFilter => "content_filter",
        FinishReason::Unknown => "unknown",
        _ => "unknown",
    }
}
