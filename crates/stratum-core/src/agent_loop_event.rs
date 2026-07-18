//! Typed events emitted by the foundational agent loop.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    ApprovalDecision, ApprovalId, CallId, ChatMessage, DangerLevel, LlmCallId, TokenUsage,
    ToolKind, ToolName,
};

/// Durable agent-loop events that require persistence acknowledgement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum DurableAgentEvent {
    /// Agent loop started.
    LoopStarted,
    /// A complete message was appended to committed loop context.
    MessageAppended {
        /// Complete message payload.
        message: ChatMessage,
    },
    /// A tool call requires user approval.
    ToolApprovalRequested {
        /// Approval request identity.
        approval_id: ApprovalId,
        /// Tool call identity.
        call_id: CallId,
        /// Provider-visible tool name.
        tool_name: ToolName,
        /// Tool call arguments.
        arguments: Value,
        /// Whether the tool observes or mutates state.
        tool_kind: ToolKind,
        /// Declared danger of the tool.
        danger_level: DangerLevel,
    },
    /// A tool approval request was resolved.
    ToolApprovalResolved {
        /// Approval request identity.
        approval_id: ApprovalId,
        /// User decision.
        decision: ApprovalDecision,
    },
    /// A tool began executing after validation and approval.
    ToolExecutionStarted {
        /// Tool call identity.
        call_id: CallId,
        /// Provider-visible tool name.
        tool_name: ToolName,
    },
    /// One loop iteration reached its durable boundary.
    IterationCompleted {
        /// Iteration number.
        iteration: u64,
        /// Token usage accumulated through this iteration.
        usage: TokenUsage,
    },
    /// Agent loop finished successfully.
    LoopFinished {
        /// Why the loop finished.
        finish_reason: String,
        /// Token usage accumulated by the loop.
        usage: TokenUsage,
    },
    /// Agent loop failed.
    LoopFailed {
        /// Error text safe to expose to callers.
        error_text: String,
        /// Token usage accumulated by the loop.
        usage: TokenUsage,
    },
    /// Agent loop was cancelled.
    LoopCancelled {
        /// Token usage accumulated by the loop.
        usage: TokenUsage,
    },
}

impl DurableAgentEvent {
    /// Returns the stable serialized event type name.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::LoopStarted => "loop_started",
            Self::MessageAppended { .. } => "message_appended",
            Self::ToolApprovalRequested { .. } => "tool_approval_requested",
            Self::ToolApprovalResolved { .. } => "tool_approval_resolved",
            Self::ToolExecutionStarted { .. } => "tool_execution_started",
            Self::IterationCompleted { .. } => "iteration_completed",
            Self::LoopFinished { .. } => "loop_finished",
            Self::LoopFailed { .. } => "loop_failed",
            Self::LoopCancelled { .. } => "loop_cancelled",
        }
    }
}

/// Best-effort agent-loop telemetry that does not control loop progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentTelemetryEvent {
    /// An LLM call started.
    LlmStarted {
        /// LLM call identity.
        llm_call_id: LlmCallId,
    },
    /// An LLM call emitted visible text.
    TextDelta {
        /// LLM call identity.
        llm_call_id: LlmCallId,
        /// Visible text fragment.
        delta: String,
    },
    /// An LLM call emitted reasoning text.
    ReasoningDelta {
        /// LLM call identity.
        llm_call_id: LlmCallId,
        /// Reasoning text fragment.
        delta: String,
    },
    /// An LLM call emitted a tool-call update.
    ToolCallDelta {
        /// LLM call identity.
        llm_call_id: LlmCallId,
        /// Tool call identity.
        call_id: CallId,
        /// Provider-visible tool name when known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Raw argument text fragment.
        arguments_delta: String,
    },
    /// An LLM call finished.
    LlmFinished {
        /// LLM call identity.
        llm_call_id: LlmCallId,
        /// Why the LLM call finished.
        finish_reason: String,
        /// Token usage reported by the provider, when available.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        usage: Option<TokenUsage>,
    },
}

impl AgentTelemetryEvent {
    /// Returns the stable serialized event type name.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::LlmStarted { .. } => "llm_started",
            Self::TextDelta { .. } => "text_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::ToolCallDelta { .. } => "tool_call_delta",
            Self::LlmFinished { .. } => "llm_finished",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentTelemetryEvent, DurableAgentEvent};
    use crate::{
        ApprovalDecision, ApprovalId, CallId, ChatMessage, DangerLevel, LlmCallId, TokenUsage,
        ToolKind, ToolName,
    };
    use serde_json::json;

    #[test]
    fn durable_message_event_serializes_with_stable_snake_case_type() -> serde_json::Result<()> {
        let event = DurableAgentEvent::MessageAppended {
            message: ChatMessage::user("hello"),
        };

        assert_eq!(event.event_type(), "message_appended");
        let serialized = serde_json::to_value(&event)?;
        assert_eq!(
            serialized,
            json!({
                "type": "message_appended",
                "data": {
                    "message": {
                        "role": "user",
                        "content": {
                            "type": "text",
                            "data": "hello"
                        }
                    }
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<DurableAgentEvent>(serialized)?,
            event
        );

        Ok(())
    }

    #[test]
    fn telemetry_delta_event_serializes_with_stable_snake_case_type() -> serde_json::Result<()> {
        let event = AgentTelemetryEvent::TextDelta {
            llm_call_id: LlmCallId::from("llm-call-1"),
            delta: "hel".to_owned(),
        };

        assert_eq!(event.event_type(), "text_delta");
        let serialized = serde_json::to_value(&event)?;
        assert_eq!(
            serialized,
            json!({
                "type": "text_delta",
                "data": {
                    "llm_call_id": "llm-call-1",
                    "delta": "hel"
                }
            })
        );
        assert_eq!(
            serde_json::from_value::<AgentTelemetryEvent>(serialized)?,
            event
        );

        Ok(())
    }

    #[test]
    fn telemetry_none_fields_are_omitted() -> serde_json::Result<()> {
        let tool_delta = serde_json::to_value(AgentTelemetryEvent::ToolCallDelta {
            llm_call_id: LlmCallId::from("llm-call-1"),
            call_id: CallId::from("tool-call-1"),
            name: None,
            arguments_delta: "{}".to_owned(),
        })?;
        let llm_finished = serde_json::to_value(AgentTelemetryEvent::LlmFinished {
            llm_call_id: LlmCallId::from("llm-call-1"),
            finish_reason: "stop".to_owned(),
            usage: None,
        })?;

        assert!(tool_delta["data"].get("name").is_none());
        assert!(llm_finished["data"].get("usage").is_none());

        Ok(())
    }

    #[test]
    fn every_durable_event_type_matches_its_serialized_type() -> serde_json::Result<()> {
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
        };
        let events = vec![
            DurableAgentEvent::LoopStarted,
            DurableAgentEvent::MessageAppended {
                message: ChatMessage::user("hello"),
            },
            DurableAgentEvent::ToolApprovalRequested {
                approval_id: ApprovalId::new(),
                call_id: CallId::from("tool-call-1"),
                tool_name: ToolName::from("echo"),
                arguments: json!({ "text": "hello" }),
                tool_kind: ToolKind::Read,
                danger_level: DangerLevel::Low,
            },
            DurableAgentEvent::ToolApprovalResolved {
                approval_id: ApprovalId::new(),
                decision: ApprovalDecision::Approve,
            },
            DurableAgentEvent::ToolExecutionStarted {
                call_id: CallId::from("tool-call-1"),
                tool_name: ToolName::from("echo"),
            },
            DurableAgentEvent::IterationCompleted {
                iteration: 1,
                usage,
            },
            DurableAgentEvent::LoopFinished {
                finish_reason: "stop".to_owned(),
                usage,
            },
            DurableAgentEvent::LoopFailed {
                error_text: "provider unavailable".to_owned(),
                usage,
            },
            DurableAgentEvent::LoopCancelled { usage },
        ];

        for event in events {
            assert_eq!(
                serde_json::to_value(&event)?["type"],
                json!(event.event_type())
            );
        }

        Ok(())
    }

    #[test]
    fn every_telemetry_event_type_matches_its_serialized_type() -> serde_json::Result<()> {
        let llm_call_id = LlmCallId::from("llm-call-1");
        let events = vec![
            AgentTelemetryEvent::LlmStarted {
                llm_call_id: llm_call_id.clone(),
            },
            AgentTelemetryEvent::TextDelta {
                llm_call_id: llm_call_id.clone(),
                delta: "hello".to_owned(),
            },
            AgentTelemetryEvent::ReasoningDelta {
                llm_call_id: llm_call_id.clone(),
                delta: "thinking".to_owned(),
            },
            AgentTelemetryEvent::ToolCallDelta {
                llm_call_id: llm_call_id.clone(),
                call_id: CallId::from("tool-call-1"),
                name: Some("echo".to_owned()),
                arguments_delta: "{}".to_owned(),
            },
            AgentTelemetryEvent::LlmFinished {
                llm_call_id,
                finish_reason: "stop".to_owned(),
                usage: Some(TokenUsage {
                    input_tokens: 1,
                    output_tokens: 2,
                    total_tokens: 3,
                }),
            },
        ];

        for event in events {
            assert_eq!(
                serde_json::to_value(&event)?["type"],
                json!(event.event_type())
            );
        }

        Ok(())
    }
}
