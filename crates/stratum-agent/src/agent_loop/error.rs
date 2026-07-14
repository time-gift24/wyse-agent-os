//! Typed failures that stop the agent loop kernel.

use stratum_core::CallId;
use stratum_infra::DurableEventSinkError;
use stratum_llm::LlmError;
use thiserror::Error;

/// Agent-loop protocol invariant that a provider response violated.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum ProtocolError {
    /// The provider stream ended without a terminal finish event.
    #[error("stream ended without a finish event")]
    StreamEndedWithoutFinish,
    /// A streamed tool call did not contain every required field.
    #[error("tool call {call_id} is incomplete")]
    IncompleteToolCall {
        /// Provider identity of the incomplete tool call.
        call_id: CallId,
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
