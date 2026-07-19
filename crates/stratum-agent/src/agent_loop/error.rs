//! Typed failures that stop the agent loop kernel.

use std::{error::Error as StdError, fmt};

use stratum_core::{CallId, ChatRole};
use stratum_infra::DurableEventSinkError;
use stratum_llm::LlmError;
use thiserror::Error;

use crate::ToolExecutorError;

/// Failure returned by an [`AgentLoopHook`](super::AgentLoopHook) callback.
#[derive(Debug, Error)]
#[error("agent loop hook callback failed")]
pub struct AgentLoopHookError {
    #[source]
    source: Box<dyn StdError + Send + Sync + 'static>,
}

impl AgentLoopHookError {
    /// Wraps a concrete hook failure while preserving its source chain.
    #[must_use]
    pub fn new(source: impl StdError + Send + Sync + 'static) -> Self {
        Self {
            source: Box::new(source),
        }
    }
}

/// Lifecycle callback that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum AgentLoopHookStage {
    /// [`AgentLoopHook::before_iteration`](super::AgentLoopHook::before_iteration).
    BeforeIteration,
    /// [`AgentLoopHook::before_llm_call`](super::AgentLoopHook::before_llm_call).
    BeforeLlmCall,
    /// [`AgentLoopHook::after_llm_call`](super::AgentLoopHook::after_llm_call).
    AfterLlmCall,
    /// [`AgentLoopHook::on_llm_error`](super::AgentLoopHook::on_llm_error).
    OnLlmError,
    /// [`AgentLoopHook::before_tool_call`](super::AgentLoopHook::before_tool_call).
    BeforeToolCall,
    /// [`AgentLoopHook::after_tool_call`](super::AgentLoopHook::after_tool_call).
    AfterToolCall,
    /// [`AgentLoopHook::after_iteration`](super::AgentLoopHook::after_iteration).
    AfterIteration,
}

impl fmt::Display for AgentLoopHookStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::BeforeIteration => "before_iteration",
            Self::BeforeLlmCall => "before_llm_call",
            Self::AfterLlmCall => "after_llm_call",
            Self::OnLlmError => "on_llm_error",
            Self::BeforeToolCall => "before_tool_call",
            Self::AfterToolCall => "after_tool_call",
            Self::AfterIteration => "after_iteration",
        })
    }
}

/// Failure to construct an [`AgentLoop`](super::AgentLoop).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[non_exhaustive]
pub enum AgentLoopBuildError {
    /// The model provider was not supplied.
    #[error("missing agent loop field llm_provider")]
    MissingLlmProvider,
    /// The tool executor was not supplied.
    #[error("missing agent loop field tool_executor")]
    MissingToolExecutor,
    /// The telemetry sink was not supplied.
    #[error("missing agent loop field telemetry")]
    MissingTelemetry,
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
    /// A finalized tool call reused an identity from its batch or committed loop context.
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
    /// A tool executor returned a non-JSON tool result message.
    #[error("tool result {call_id} does not contain JSON")]
    InvalidToolResultContent {
        /// Tool call whose result violated the executor contract.
        call_id: CallId,
    },
}

/// Failure that prevents the agent loop from preserving its invariants.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentLoopError {
    /// A lifecycle hook rejected or failed an agent-loop boundary.
    #[error("agent loop hook {stage} failed")]
    Hook {
        /// Callback boundary that failed.
        stage: AgentLoopHookStage,
        /// Hook implementation failure.
        #[source]
        source: AgentLoopHookError,
    },
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
    /// Approval or execution orchestration failed before producing a model-visible tool result.
    #[error("tool execution orchestration failed")]
    ToolExecution {
        /// Tool executor failure.
        #[source]
        source: ToolExecutorError,
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
    /// The run reached its model-iteration bound.
    #[error("maximum of {maximum} agent loop iterations reached")]
    IterationLimitExceeded {
        /// Configured maximum number of iterations.
        maximum: usize,
    },
    /// One model response exceeded its tool-call bound.
    #[error("maximum of {maximum} tool calls per iteration exceeded")]
    ToolCallLimitExceeded {
        /// Configured maximum number of tool calls per iteration.
        maximum: usize,
    },
    /// Streamed assistant text exceeded its byte bound.
    #[error("maximum of {maximum} streamed assistant text bytes exceeded")]
    TextByteLimitExceeded {
        /// Configured maximum number of bytes.
        maximum: usize,
    },
    /// Streamed reasoning exceeded its byte bound.
    #[error("maximum of {maximum} streamed reasoning bytes exceeded")]
    ReasoningByteLimitExceeded {
        /// Configured maximum number of bytes.
        maximum: usize,
    },
    /// One streamed tool call exceeded its argument byte bound.
    #[error("maximum of {maximum} streamed tool argument bytes exceeded")]
    ToolArgumentByteLimitExceeded {
        /// Configured maximum number of bytes.
        maximum: usize,
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
    fn hook_failure_preserves_stage_and_source_chain() {
        let error = AgentLoopError::Hook {
            stage: AgentLoopHookStage::BeforeLlmCall,
            source: AgentLoopHookError::new(std::io::Error::other("hook unavailable")),
        };

        assert!(matches!(
            &error,
            AgentLoopError::Hook {
                stage: AgentLoopHookStage::BeforeLlmCall,
                ..
            }
        ));
        assert_eq!(error.to_string(), "agent loop hook before_llm_call failed");
        assert_eq!(
            error
                .source()
                .and_then(std::error::Error::source)
                .map(ToString::to_string),
            Some("hook unavailable".to_owned())
        );
    }
}
