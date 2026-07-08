//! Core protocol types shared across Wyse crates.

use std::{collections::BTreeMap, fmt, str::FromStr};

use bon::Builder;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

/// Identity of one workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RunId(Uuid);

impl RunId {
    /// Creates a new UUIDv7 run id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Returns the inner UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for RunId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for RunId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for RunId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<RunId> for Uuid {
    fn from(value: RunId) -> Self {
        value.0
    }
}

impl FromStr for RunId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<Uuid>().map(Self)
    }
}

macro_rules! string_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Creates a new id from a string-like value.
            #[must_use]
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            /// Returns the id as a string slice.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self(value.to_owned())
            }
        }
    };
}

string_id!(NodeId, "Identity of a workflow node.");
string_id!(AgentId, "Identity of an agent.");
string_id!(ModelId, "Identity of a model.");
string_id!(CallId, "Identity of one tool call.");
string_id!(ToolName, "Provider-visible identity of a tool.");
string_id!(LlmCallId, "Identity of one LLM call.");
string_id!(PlanId, "Identity of an agent-visible plan.");

/// Source that owns a runtime stream event.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventSource {
    /// Event belongs to the whole run.
    Run,
    /// Event belongs to a workflow node.
    Node {
        /// Node that owns the event.
        node_id: NodeId,
    },
    /// Event belongs to an agent running inside a workflow node.
    Agent {
        /// Node that owns the agent.
        node_id: NodeId,
        /// Agent that produced the event.
        agent_id: AgentId,
    },
}

/// Token usage reported by a model provider.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    /// Input tokens consumed.
    pub input_tokens: u64,
    /// Output tokens produced.
    pub output_tokens: u64,
    /// Total tokens reported by the provider.
    pub total_tokens: u64,
}

/// Tool definition exposed to an LLM provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Builder)]
#[non_exhaustive]
pub struct ToolSpec {
    /// Provider-visible tool name.
    #[builder(into)]
    pub name: ToolName,
    /// Provider-visible tool description.
    #[builder(into)]
    pub description: String,
    /// JSON schema for tool input.
    pub input_schema: Value,
}

/// Role of one normal text delta in an LLM call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum LlmCallRole {
    /// System instruction.
    System,
    /// End-user input.
    User,
    /// Assistant-visible answer.
    Assistant,
    /// Tool result output.
    Tool,
}

/// Event emitted inside one LLM call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum LlmEvent {
    /// LLM call started.
    Started,
    /// LLM call finished.
    Finished {
        /// Why the LLM call finished.
        finish_reason: String,
        /// Token usage reported by the provider.
        usage: Option<TokenUsage>,
    },
    /// LLM call failed.
    Failed {
        /// Error text safe to expose to callers.
        error_text: String,
    },
    /// LLM call emitted normal text.
    TextDelta {
        /// Role of this text fragment.
        role: LlmCallRole,
        /// Visible text delta.
        delta: String,
    },
    /// LLM call emitted reasoning text.
    ReasoningDelta {
        /// Reasoning text delta.
        delta: String,
    },
    /// Tool call started.
    ToolCallStarted {
        /// Tool call identity.
        call_id: CallId,
        /// Provider-visible tool name.
        name: Option<String>,
    },
    /// Tool call arguments changed.
    ToolCallDelta {
        /// Tool call identity.
        call_id: CallId,
        /// Provider-visible tool name when known.
        name: Option<String>,
        /// Tool argument text fragment.
        arguments_delta: String,
    },
    /// Tool call finished.
    ToolCallFinished {
        /// Tool call identity.
        call_id: CallId,
        /// Tool result.
        result: Value,
    },
    /// Tool call failed.
    ToolCallFailed {
        /// Tool call identity.
        call_id: CallId,
        /// Error text safe to expose to callers.
        error_text: String,
    },
}

/// Runtime event payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum RuntimeEvent {
    /// Run started.
    RunStarted,
    /// Run finished successfully.
    RunFinished {
        /// Why the run finished.
        finish_reason: String,
        /// Token usage accumulated by the run.
        usage: TokenUsage,
    },
    /// Run failed.
    RunFailed {
        /// Error text safe to expose to callers.
        error_text: String,
    },
    /// Run was cancelled.
    RunCancelled,
    /// Node execution started.
    NodeStarted,
    /// Normal node produced output.
    NodeOutput {
        /// Node output payload.
        output: Value,
    },
    /// Node execution finished.
    NodeFinished,
    /// Node execution failed.
    NodeFailed {
        /// Error text safe to expose to callers.
        error_text: String,
    },
    /// Event emitted by one LLM call.
    Llm {
        /// LLM call identity.
        llm_call_id: LlmCallId,
        /// LLM event payload.
        event: LlmEvent,
    },
    /// Agent-visible plan changed.
    PlanUpdated {
        /// Plan identity.
        plan_id: PlanId,
        /// Plan payload.
        plan: Value,
    },
}

impl RuntimeEvent {
    /// Returns the serialized event type name.
    #[must_use]
    pub const fn event_type(&self) -> &'static str {
        match self {
            Self::RunStarted => "run_started",
            Self::RunFinished { .. } => "run_finished",
            Self::RunFailed { .. } => "run_failed",
            Self::RunCancelled => "run_cancelled",
            Self::NodeStarted => "node_started",
            Self::NodeOutput { .. } => "node_output",
            Self::NodeFinished => "node_finished",
            Self::NodeFailed { .. } => "node_failed",
            Self::Llm { .. } => "llm",
            Self::PlanUpdated { .. } => "plan_updated",
        }
    }
}

/// One event in a workflow run stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StreamEnvelope {
    /// Workflow run identity.
    pub run_id: RunId,
    /// Monotonic event sequence in one run.
    pub seq: u64,
    /// Event creation time.
    pub timestamp: DateTime<Utc>,
    /// Event ownership.
    pub source: EventSource,
    /// Typed runtime event payload.
    pub event: RuntimeEvent,
    /// Runtime-only metadata; not for business payloads.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_id_uses_uuid_v7() {
        let version = RunId::new().as_uuid().get_version_num();

        assert_eq!(version, 7);
    }

    #[test]
    fn model_id_round_trips_string() {
        let model_id = ModelId::from("gpt-4.1-mini");

        assert_eq!(model_id.as_str(), "gpt-4.1-mini");
        assert_eq!(model_id.to_string(), "gpt-4.1-mini");
    }

    #[test]
    fn tool_name_round_trips_string() {
        let tool_name = ToolName::from("echo");

        assert_eq!(tool_name.as_str(), "echo");
        assert_eq!(tool_name.to_string(), "echo");
    }

    #[test]
    fn tool_spec_serializes_provider_visible_shape() {
        let spec = ToolSpec::builder()
            .name("echo")
            .description("returns input arguments")
            .input_schema(serde_json::json!({"type": "object"}))
            .build();

        assert_eq!(
            serde_json::to_value(spec).expect("tool spec should serialize"),
            serde_json::json!({
                "name": "echo",
                "description": "returns input arguments",
                "input_schema": {"type": "object"}
            })
        );
    }

    #[test]
    fn llm_call_id_round_trips_string() {
        let llm_call_id = LlmCallId::from("llm-call-1");

        assert_eq!(llm_call_id.as_str(), "llm-call-1");
        assert_eq!(llm_call_id.to_string(), "llm-call-1");
    }

    #[test]
    fn event_type_matches_protocol_name() {
        let event = RuntimeEvent::Llm {
            llm_call_id: LlmCallId::from("llm-call-1"),
            event: LlmEvent::TextDelta {
                role: LlmCallRole::Assistant,
                delta: "hello".to_owned(),
            },
        };

        assert_eq!(event.event_type(), "llm");
    }

    #[test]
    fn llm_started_has_nested_event_type() {
        let event = RuntimeEvent::Llm {
            llm_call_id: LlmCallId::from("llm-call-1"),
            event: LlmEvent::Started,
        };

        assert_eq!(event.event_type(), "llm");
        let value = serde_json::to_value(event).expect("event should serialize");
        assert_eq!(value["data"]["event"]["type"], "started");
    }

    #[test]
    fn user_input_uses_text_delta_role() {
        let event = LlmEvent::TextDelta {
            role: LlmCallRole::User,
            delta: "hello".to_owned(),
        };
        let value = serde_json::to_value(event).expect("event should serialize");

        assert_eq!(value["type"], "text_delta");
        assert_eq!(value["data"]["role"], "user");
    }

    #[test]
    fn llm_runtime_event_wraps_text_delta() {
        let event = RuntimeEvent::Llm {
            llm_call_id: LlmCallId::from("llm-call-1"),
            event: LlmEvent::TextDelta {
                role: LlmCallRole::User,
                delta: "hello".to_owned(),
            },
        };

        assert_eq!(event.event_type(), "llm");
    }

    #[test]
    fn assistant_output_uses_text_delta_role() {
        let event = LlmEvent::TextDelta {
            role: LlmCallRole::Assistant,
            delta: "hello".to_owned(),
        };
        let value = serde_json::to_value(event).expect("event should serialize");

        assert_eq!(value["type"], "text_delta");
        assert_eq!(value["data"]["role"], "assistant");
    }

    #[test]
    fn reasoning_delta_uses_llm_call_id() {
        let event = RuntimeEvent::Llm {
            llm_call_id: LlmCallId::from("llm-call-1"),
            event: LlmEvent::ReasoningDelta {
                delta: "thinking".to_owned(),
            },
        };

        let value = serde_json::to_value(event).expect("event should serialize");
        assert_eq!(value["data"]["event"]["type"], "reasoning_delta");
    }

    #[test]
    fn reasoning_delta_is_not_a_text_role() {
        let event = LlmEvent::ReasoningDelta {
            delta: "thinking".to_owned(),
        };
        let value = serde_json::to_value(event).expect("event should serialize");

        assert_eq!(value["type"], "reasoning_delta");
    }

    #[test]
    fn tool_call_delta_supports_partial_arguments() {
        let event = LlmEvent::ToolCallDelta {
            call_id: CallId::from("call-1"),
            name: Some("get_weather".to_owned()),
            arguments_delta: "{\"city".to_owned(),
        };
        let value = serde_json::to_value(event).expect("event should serialize");

        assert_eq!(value["type"], "tool_call_delta");
        assert_eq!(value["data"]["call_id"], "call-1");
    }
}
