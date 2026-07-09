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

/// Identity of one resumable turn inside a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TurnId(Uuid);

impl TurnId {
    /// Creates a new UUIDv7 turn id.
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

impl Default for TurnId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TurnId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for TurnId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<TurnId> for Uuid {
    fn from(value: TurnId) -> Self {
        value.0
    }
}

impl FromStr for TurnId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<Uuid>().map(Self)
    }
}

/// Identity of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentId(Uuid);

impl AgentId {
    /// Creates a new UUIDv7 agent id.
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

impl Default for AgentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for AgentId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<AgentId> for Uuid {
    fn from(value: AgentId) -> Self {
        value.0
    }
}

impl FromStr for AgentId {
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

/// Complete tool call emitted by a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    /// Provider call identity.
    pub call_id: CallId,
    /// Provider-visible tool name.
    pub name: String,
    /// Parsed tool arguments.
    pub arguments: Value,
}

/// Incremental tool call update from a stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCallDelta {
    /// Position of the tool call in the response.
    pub index: usize,
    /// Provider call identity when known.
    pub call_id: Option<CallId>,
    /// Provider-visible tool name when known.
    pub name: Option<String>,
    /// Raw argument text fragment.
    pub arguments_delta: String,
}

/// Role of a chat message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatRole {
    /// System instruction message.
    System,
    /// End-user message.
    User,
    /// Assistant response message.
    Assistant,
    /// Tool result message.
    Tool,
}

/// Content carried by a chat message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatContent {
    /// Plain text content.
    Text(String),
    /// JSON content.
    Json(Value),
}

/// Message exchanged with an LLM provider.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatMessage {
    /// Message role.
    pub role: ChatRole,
    /// Message content.
    pub content: ChatContent,
    /// Tool calls requested by an assistant message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
    /// Reasoning content produced by an assistant message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    /// Tool call this tool message answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<CallId>,
}

impl ChatMessage {
    /// Creates a system text message.
    #[must_use]
    pub fn system(content: impl Into<String>) -> Self {
        Self::text(ChatRole::System, content)
    }

    /// Creates a user text message.
    #[must_use]
    pub fn user(content: impl Into<String>) -> Self {
        Self::text(ChatRole::User, content)
    }

    /// Creates an assistant text message.
    #[must_use]
    pub fn assistant(content: impl Into<String>) -> Self {
        Self::text(ChatRole::Assistant, content)
    }

    /// Creates a tool result message.
    #[must_use]
    pub fn tool(call_id: impl Into<CallId>, result: Value) -> Self {
        Self {
            role: ChatRole::Tool,
            content: ChatContent::Json(result),
            tool_calls: Vec::new(),
            reasoning_content: None,
            tool_call_id: Some(call_id.into()),
        }
    }

    /// Creates a text message for a role.
    #[must_use]
    pub fn text(role: ChatRole, content: impl Into<String>) -> Self {
        Self {
            role,
            content: ChatContent::Text(content.into()),
            tool_calls: Vec::new(),
            reasoning_content: None,
            tool_call_id: None,
        }
    }

    /// Sets assistant reasoning content.
    #[must_use]
    pub fn with_reasoning_content(mut self, content: impl Into<String>) -> Self {
        self.reasoning_content = Some(content.into());
        self
    }

    /// Sets tool calls for this message.
    #[must_use]
    pub fn with_tool_calls(mut self, tool_calls: Vec<ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self
    }
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

/// Event emitted by an agent runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum AgentEvent {
    /// Agent run started.
    Started,
    /// Agent run finished.
    Finished {
        /// Why the run finished.
        finish_reason: String,
        /// Token usage accumulated by the run.
        usage: TokenUsage,
    },
    /// Agent run failed.
    Failed {
        /// Error text safe to expose to callers.
        error_text: String,
    },
    /// Agent run was cancelled.
    Cancelled,
    /// Event emitted by one LLM call inside the agent run.
    Llm {
        /// LLM call identity.
        llm_call_id: LlmCallId,
        /// LLM event payload.
        event: LlmEvent,
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
    /// Event emitted by one agent.
    Agent {
        /// Agent identity.
        agent_id: AgentId,
        /// Agent event payload.
        event: AgentEvent,
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
            Self::Agent { .. } => "agent",
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
    fn turn_id_new_uses_uuid_v7() {
        let id = TurnId::new();

        assert_eq!(id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn turn_id_round_trips_string() {
        let id = TurnId::new();
        let parsed = id.to_string().parse::<TurnId>().expect("turn id parses");

        assert_eq!(parsed, id);
    }

    #[test]
    fn agent_id_new_uses_uuid_v7() {
        let id = AgentId::new();

        assert_eq!(id.as_uuid().get_version_num(), 7);
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
    fn chat_message_user_constructor_sets_role_and_text() {
        let message = ChatMessage::user("hello");

        assert_eq!(message.role, ChatRole::User);
        assert_eq!(message.content, ChatContent::Text("hello".to_owned()));
        assert!(message.tool_calls.is_empty());
        assert!(message.tool_call_id.is_none());
    }

    #[test]
    fn chat_message_with_tool_calls_sets_tool_calls() {
        let calls = vec![
            ToolCall {
                call_id: CallId::from("call-1"),
                name: "get_weather".to_owned(),
                arguments: serde_json::json!({"city": "Tokyo"}),
            },
            ToolCall {
                call_id: CallId::from("call-2"),
                name: "get_time".to_owned(),
                arguments: serde_json::json!({"tz": "UTC"}),
            },
        ];
        let message = ChatMessage::assistant("hi").with_tool_calls(calls.clone());

        assert_eq!(message.tool_calls, calls);
    }

    #[test]
    fn tool_message_records_answered_call_id() {
        let message = ChatMessage::tool(CallId::from("call-1"), serde_json::json!({"ok": true}));

        assert_eq!(message.role, ChatRole::Tool);
        assert_eq!(message.tool_call_id, Some(CallId::from("call-1")));
        assert_eq!(
            message.content,
            ChatContent::Json(serde_json::json!({"ok": true}))
        );
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
    fn runtime_agent_event_type_is_agent() {
        let event = RuntimeEvent::Agent {
            agent_id: AgentId::new(),
            event: AgentEvent::Started,
        };

        assert_eq!(event.event_type(), "agent");
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
