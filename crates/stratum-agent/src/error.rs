//! Error types for agent runtime operations.

use stratum_core::{AgentId, CallId, ChatRole, RunId, TurnId};
use stratum_infra::event_stream_bus::EventStreamBusError;
use stratum_llm::LlmError;
use stratum_store::{AgentStatus, StoreError};
use thiserror::Error;

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
    /// Durable state contains an unfinished run that must be resumed.
    #[error("persisted agent run requires resume")]
    PersistedRunRequiresResume {
        /// Persisted run identity.
        run_id: RunId,
        /// Persisted turn identity.
        turn_id: TurnId,
    },
    /// No agent turn is currently active.
    #[error("no agent turn is active")]
    NoActiveTurn,
    /// The requested tool approval is not active.
    #[error("tool approval is not active: {approval_id}")]
    ApprovalNotFound {
        /// Approval request identity.
        approval_id: stratum_core::ApprovalId,
    },
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
    /// Agent store operation failed.
    #[error("agent store operation failed")]
    Store {
        /// Underlying store error.
        #[source]
        source: StoreError,
    },
    /// Persisted state cannot be resumed because it is not running.
    #[error("persisted agent is not running: {actual:?}")]
    ResumeNotRunning {
        /// Persisted status.
        actual: AgentStatus,
    },
    /// Persisted state has a resumable turn that must use `Agent::resume`.
    #[error("cannot load history from a persisted running agent")]
    LoadHistoryRunning,
    /// Persisted running state has no run identity.
    #[error("persisted running agent has no run id")]
    ResumeRunMissing,
    /// Persisted running state has no turn identity.
    #[error("persisted running agent has no turn id")]
    ResumeTurnMissing,
    /// Persisted state belongs to another agent.
    #[error("resume agent mismatch: expected {expected}, actual {actual}")]
    ResumeAgentMismatch {
        /// Built agent identity.
        expected: AgentId,
        /// Persisted agent identity.
        actual: AgentId,
    },
    /// Persisted message history cannot form a resumable conversation.
    #[error("invalid resume history")]
    InvalidResumeHistory,
    /// A persisted iteration cannot be represented by the loop implementation.
    #[error("iteration cannot be represented: {iteration}")]
    IterationOutOfRange {
        /// Persisted iteration.
        iteration: u64,
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

impl From<StoreError> for AgentError {
    fn from(source: StoreError) -> Self {
        Self::Store { source }
    }
}
