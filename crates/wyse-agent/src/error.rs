//! Error types for agent runtime operations.

use thiserror::Error;
use wyse_core::{CallId, ChatRole};
use wyse_infra::event_stream_bus::EventStreamBusError;
use wyse_llm::LlmError;

/// Error returned by agent operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentError {
    /// Input message role is not accepted by `Agent::run_turn`.
    #[error("invalid input message role: {role:?}")]
    InvalidInputMessageRole {
        /// Rejected role.
        role: ChatRole,
    },
    /// Another run is already active for this stateful agent.
    #[error("agent run is already active")]
    RunAlreadyActive,
    /// No agent turn is currently active.
    #[error("no agent turn is active")]
    NoActiveTurn,
    /// The requested tool approval is not active.
    #[error("tool approval is not active: {approval_id}")]
    ApprovalNotFound {
        /// Approval request identity.
        approval_id: wyse_core::ApprovalId,
    },
    /// The active turn's command channel closed.
    #[error("agent turn command channel closed")]
    TurnCommandClosed,
    /// The approval decision is not supported by this runtime.
    #[error("unsupported tool approval decision")]
    UnsupportedApprovalDecision,
    /// LLM provider operation failed.
    #[error("llm operation failed")]
    Llm {
        /// Underlying LLM error.
        #[source]
        source: LlmError,
    },
    /// Event bus operation failed.
    #[error("event bus operation failed")]
    EventBus {
        /// Underlying event bus error.
        #[source]
        source: EventStreamBusError,
    },
    /// A required builder field was not provided.
    #[error("missing builder field: {field}")]
    MissingBuilderField {
        /// Missing field name.
        field: &'static str,
    },
    /// A turn requested more tool calls than allowed.
    #[error("tool call limit exceeded: {limit}")]
    ToolCallLimitExceeded {
        /// Configured limit.
        limit: usize,
    },
    /// The run reached the configured turn limit.
    #[error("turn limit exceeded: {limit}")]
    TurnLimitExceeded {
        /// Configured limit.
        limit: usize,
    },
    /// A streamed tool call ended without enough information to execute.
    #[error("incomplete tool call: {call_id}")]
    IncompleteToolCall {
        /// Incomplete call id.
        call_id: CallId,
    },
    /// The run was cancelled.
    #[error("agent run cancelled")]
    Cancelled,
}

impl From<LlmError> for AgentError {
    fn from(source: LlmError) -> Self {
        Self::Llm { source }
    }
}

impl From<EventStreamBusError> for AgentError {
    fn from(source: EventStreamBusError) -> Self {
        Self::EventBus { source }
    }
}
