//! Typed failures that stop the agent loop kernel.

use std::fmt;

use stratum_core::{CallId, ChatRole};
use stratum_infra::DurableEventSinkError;
use stratum_llm::LlmError;
use thiserror::Error;

/// Required dependency accepted by [`AgentLoopBuilder`](super::AgentLoopBuilder).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RequiredAgentLoopField {
    /// Bound model provider.
    LlmProvider,
    /// Approval-aware tool executor.
    ToolExecutor,
    /// Required durable event sink.
    DurableEvents,
    /// Best-effort telemetry sink.
    Telemetry,
    /// Safety limits for one run.
    Limits,
}

impl fmt::Display for RequiredAgentLoopField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::LlmProvider => "llm_provider",
            Self::ToolExecutor => "tool_executor",
            Self::DurableEvents => "durable_events",
            Self::Telemetry => "telemetry",
            Self::Limits => "limits",
        })
    }
}

/// Failure to construct an [`AgentLoop`](super::AgentLoop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AgentLoopBuildError {
    /// A required dependency was not supplied.
    #[error("missing agent loop field {field}")]
    MissingField {
        /// Builder field that must be supplied.
        field: RequiredAgentLoopField,
    },
}

/// Agent-loop protocol invariant that a provider response violated.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// A loop run did not receive any new user prompt.
    #[error("agent loop prompts are empty")]
    EmptyPrompts,
    /// A new prompt had a role other than user.
    #[error("agent loop prompt has invalid role {role:?}")]
    InvalidPromptRole {
        /// Role rejected at the prompt boundary.
        role: ChatRole,
    },
    /// The provider stream ended without a terminal finish event.
    #[error("stream ended without a finish event")]
    StreamEndedWithoutFinish,
    /// Tool-call indices skipped an earlier position.
    #[error("tool call index {actual} is sparse; expected {expected}")]
    SparseToolCallIndex {
        /// Next contiguous index required by the protocol.
        expected: usize,
        /// Provider index that skipped the expected position.
        actual: usize,
    },
    /// One streamed index changed its provider call identity.
    #[error("tool call index {index} changed call id from {existing} to {received}")]
    ConflictingToolCallId {
        /// Provider position of the conflicting call.
        index: usize,
        /// First identity received for the position.
        existing: CallId,
        /// Later conflicting identity.
        received: CallId,
    },
    /// One streamed index changed its provider-visible tool name.
    #[error("tool call index {index} changed name from {existing} to {received}")]
    ConflictingToolCallName {
        /// Provider position of the conflicting call.
        index: usize,
        /// First name received for the position.
        existing: String,
        /// Later conflicting name.
        received: String,
    },
    /// Two finalized tool calls shared one provider identity.
    #[error("duplicate tool call id {call_id}")]
    DuplicateToolCallId {
        /// Duplicated provider call identity.
        call_id: CallId,
    },
    /// A streamed tool call did not contain every required field.
    #[error("tool call at index {index} is incomplete")]
    IncompleteToolCall {
        /// Provider position of the incomplete call.
        index: usize,
        /// Provider identity when it was received.
        call_id: Option<CallId>,
    },
    /// Tool-call argument fragments did not form valid JSON.
    #[error("tool call {call_id} arguments are invalid")]
    MalformedToolCallArguments {
        /// Provider identity of the malformed tool call.
        call_id: CallId,
        /// JSON parsing failure.
        #[source]
        source: serde_json::Error,
    },
}

/// Configured safety bound reached by an agent loop run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum LoopLimit {
    /// The run reached its model-iteration bound.
    #[error("maximum of {maximum} iterations reached")]
    Iterations {
        /// Configured maximum number of iterations.
        maximum: usize,
    },
    /// One model response exceeded its tool-call bound.
    #[error("maximum of {maximum} tool calls per iteration exceeded")]
    ToolCallsPerIteration {
        /// Configured maximum number of tool calls per iteration.
        maximum: usize,
    },
}

/// Failure that prevents the agent loop from preserving its invariants.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentLoopError {
    /// A required durable event was not acknowledged.
    #[error("durable agent event was not acknowledged")]
    Durability {
        /// Durable event sink failure.
        #[source]
        source: DurableEventSinkError,
    },
    /// Recording a terminal event failed after another loop operation had already failed.
    #[error("durable terminal agent event was not acknowledged")]
    TerminalDurability {
        /// Operation failure that initiated terminal recording.
        operation: Box<AgentLoopError>,
        /// Durable terminal event sink failure, which is the primary error source.
        #[source]
        source: DurableEventSinkError,
    },
    /// The model provider failed before producing a recoverable response.
    #[error("llm operation failed")]
    Llm {
        /// Model provider failure.
        #[source]
        source: LlmError,
    },
    /// A provider response violated the loop protocol.
    #[error("invalid agent loop protocol: {reason}")]
    InvalidProtocol {
        /// Typed protocol violation.
        #[source]
        reason: ProtocolError,
    },
    /// The caller cancelled the loop before a terminal outcome was committed.
    #[error("agent loop cancelled")]
    Cancelled,
    /// The loop reached a configured safety bound.
    #[error("agent loop limit exceeded: {limit}")]
    LimitExceeded {
        /// Typed safety bound that stopped the loop.
        #[source]
        limit: LoopLimit,
    },
}

impl From<DurableEventSinkError> for AgentLoopError {
    fn from(source: DurableEventSinkError) -> Self {
        Self::Durability { source }
    }
}

impl From<LlmError> for AgentLoopError {
    fn from(source: LlmError) -> Self {
        Self::Llm { source }
    }
}

impl From<ProtocolError> for AgentLoopError {
    fn from(reason: ProtocolError) -> Self {
        Self::InvalidProtocol { reason }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error as _;

    use super::*;

    #[test]
    fn durability_conversion_preserves_the_source_chain() {
        let error = AgentLoopError::from(DurableEventSinkError::UnsupportedEvent {
            event_type: "future_event",
        });

        assert!(matches!(&error, AgentLoopError::Durability { .. }));
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<DurableEventSinkError>()),
            Some(DurableEventSinkError::UnsupportedEvent {
                event_type: "future_event"
            })
        ));
    }

    #[test]
    fn llm_conversion_preserves_the_source_chain() {
        let error = AgentLoopError::from(LlmError::MockExhausted);

        assert!(matches!(&error, AgentLoopError::Llm { .. }));
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<LlmError>()),
            Some(LlmError::MockExhausted)
        ));
    }

    #[test]
    fn protocol_conversion_is_typed() {
        let error = AgentLoopError::from(ProtocolError::StreamEndedWithoutFinish);

        assert!(matches!(
            &error,
            AgentLoopError::InvalidProtocol {
                reason: ProtocolError::StreamEndedWithoutFinish
            }
        ));
        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<ProtocolError>()),
            Some(ProtocolError::StreamEndedWithoutFinish)
        ));
    }

    #[test]
    fn limit_error_exposes_the_typed_limit_as_its_source() {
        let error = AgentLoopError::LimitExceeded {
            limit: LoopLimit::Iterations { maximum: 2 },
        };

        assert!(matches!(
            error
                .source()
                .and_then(|source| source.downcast_ref::<LoopLimit>()),
            Some(LoopLimit::Iterations { maximum: 2 })
        ));
    }
}
