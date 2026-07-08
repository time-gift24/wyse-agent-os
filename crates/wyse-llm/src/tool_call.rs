//! Tool-call types normalized across LLM providers.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use wyse_core::CallId;

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
