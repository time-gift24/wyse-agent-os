//! Core protocol types shared across Wyse crates.

use std::{collections::BTreeMap, fmt, str::FromStr};

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
string_id!(ToolId, "Identity of a tool.");
string_id!(CallId, "Identity of one tool call.");
string_id!(MessageId, "Identity of one streamed message.");
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
    /// Agent message started.
    MessageStarted {
        /// Message identity.
        message_id: MessageId,
    },
    /// Agent emitted visible text.
    TextDelta {
        /// Visible text delta.
        delta: String,
    },
    /// Agent emitted reasoning text.
    ReasoningDelta {
        /// Reasoning text delta.
        delta: String,
    },
    /// Tool call started.
    ToolCallStarted {
        /// Tool call identity.
        call_id: CallId,
        /// Tool identity.
        tool_id: ToolId,
        /// Tool arguments.
        arguments: Value,
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
            Self::MessageStarted { .. } => "message_started",
            Self::TextDelta { .. } => "text_delta",
            Self::ReasoningDelta { .. } => "reasoning_delta",
            Self::ToolCallStarted { .. } => "tool_call_started",
            Self::ToolCallFinished { .. } => "tool_call_finished",
            Self::ToolCallFailed { .. } => "tool_call_failed",
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
    fn event_type_matches_protocol_name() {
        let event = RuntimeEvent::TextDelta {
            delta: "hello".to_owned(),
        };

        assert_eq!(event.event_type(), "text_delta");
    }
}
