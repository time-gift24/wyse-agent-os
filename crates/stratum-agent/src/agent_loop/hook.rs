//! Ordered lifecycle hooks for the agent loop kernel.

use async_trait::async_trait;
use serde_json::Value;
use stratum_core::{ChatContent, ChatMessage, LlmCallId, TokenUsage, ToolCall};
use stratum_llm::{ChatRequest, FinishReason, LlmError};
use tokio_util::sync::CancellationToken;

use super::AgentLoopHookError;

/// Read-only state supplied at an iteration boundary.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct IterationHookContext<'a> {
    /// Zero-based iteration number.
    pub iteration: u64,
    /// Committed transcript visible to the next model call.
    pub messages: &'a [ChatMessage],
    /// Model usage accumulated through the current run.
    pub usage: TokenUsage,
    /// Cooperative cancellation signal for this run.
    pub cancellation: &'a CancellationToken,
}

/// Identity supplied around one model-call attempt.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct LlmCallHookContext<'a> {
    /// Zero-based iteration number.
    pub iteration: u64,
    /// Zero-based attempt number inside this iteration.
    pub attempt: usize,
    /// Identity used by telemetry for this attempt.
    pub llm_call_id: &'a LlmCallId,
    /// Cooperative cancellation signal for this run.
    pub cancellation: &'a CancellationToken,
}

/// Validated assistant output exposed to post-model hooks.
///
/// Hooks may rewrite content, reasoning, and tool calls before the assistant message is durably
/// committed. The assistant role, tool-result identity, provider finish reason, and usage remain
/// runtime-owned invariants.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LlmCallOutput {
    message: ChatMessage,
    finish_reason: FinishReason,
    usage: Option<TokenUsage>,
}

impl LlmCallOutput {
    pub(crate) const fn new(
        message: ChatMessage,
        finish_reason: FinishReason,
        usage: Option<TokenUsage>,
    ) -> Self {
        Self {
            message,
            finish_reason,
            usage,
        }
    }

    /// Returns the assistant message after hook rewrites applied so far.
    #[must_use]
    pub const fn message(&self) -> &ChatMessage {
        &self.message
    }

    /// Returns mutable assistant content.
    #[must_use]
    pub const fn content_mut(&mut self) -> &mut ChatContent {
        &mut self.message.content
    }

    /// Returns mutable assistant reasoning content.
    #[must_use]
    pub const fn reasoning_content_mut(&mut self) -> &mut Option<String> {
        &mut self.message.reasoning_content
    }

    /// Returns mutable tool calls requested by the assistant.
    #[must_use]
    pub const fn tool_calls_mut(&mut self) -> &mut Vec<ToolCall> {
        &mut self.message.tool_calls
    }

    /// Returns the provider finish reason, which hooks cannot rewrite.
    #[must_use]
    pub const fn finish_reason(&self) -> FinishReason {
        self.finish_reason
    }

    /// Returns usage reported by this model call, when available.
    #[must_use]
    pub const fn usage(&self) -> Option<TokenUsage> {
        self.usage
    }

    pub(crate) fn into_parts(self) -> (ChatMessage, FinishReason) {
        (self.message, self.finish_reason)
    }
}

/// State supplied around one tool-call orchestration attempt.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct ToolCallHookContext<'a> {
    /// Zero-based iteration number that produced the tool call.
    pub iteration: u64,
    /// Validated provider tool call.
    pub tool_call: &'a ToolCall,
    /// Cooperative cancellation signal for this run.
    pub cancellation: &'a CancellationToken,
}

/// Recovery requested after a model provider or model stream failure.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LlmErrorAction {
    /// Propagate the model failure.
    #[default]
    Fail,
    /// Retry with a fresh request and call identity when the configured retry budget permits it.
    Retry,
}

/// Ordered interception points around agent-loop external actions and durable iteration boundaries.
///
/// Hooks execute sequentially in builder registration order. A callback error stops the chain and
/// fails the loop. Default methods are no-ops, so implementations only override the boundaries they
/// need. Hook implementations must cooperatively observe the cancellation token carried by each
/// context when they perform asynchronous work.
#[async_trait]
pub trait AgentLoopHook: Send + Sync {
    /// Runs once before each model iteration.
    ///
    /// # Errors
    ///
    /// Returns a hook error to stop the loop before the model request begins.
    async fn before_iteration(
        &self,
        _context: IterationHookContext<'_>,
    ) -> Result<(), AgentLoopHookError> {
        Ok(())
    }

    /// Rewrites or inspects a model request immediately before provider dispatch.
    ///
    /// This callback runs again for every retry from a freshly constructed canonical request.
    ///
    /// # Errors
    ///
    /// Returns a hook error to stop the loop before provider dispatch.
    async fn before_llm_call(
        &self,
        _context: LlmCallHookContext<'_>,
        _request: &mut ChatRequest,
    ) -> Result<(), AgentLoopHookError> {
        Ok(())
    }

    /// Rewrites or inspects a complete assistant output before it is durably committed.
    ///
    /// # Errors
    ///
    /// Returns a hook error to stop the loop before the assistant message is committed.
    async fn after_llm_call(
        &self,
        _context: LlmCallHookContext<'_>,
        _output: &mut LlmCallOutput,
    ) -> Result<(), AgentLoopHookError> {
        Ok(())
    }

    /// Chooses whether a model failure should be retried.
    ///
    /// All registered hooks observe the failure. A retry is requested when at least one hook
    /// returns [`LlmErrorAction::Retry`], subject to
    /// [`LoopLimits::max_llm_retries_per_iteration`](super::LoopLimits::max_llm_retries_per_iteration).
    ///
    /// # Errors
    ///
    /// Returns a hook error to stop recovery and fail the loop.
    async fn on_llm_error(
        &self,
        _context: LlmCallHookContext<'_>,
        _error: &LlmError,
    ) -> Result<LlmErrorAction, AgentLoopHookError> {
        Ok(LlmErrorAction::Fail)
    }

    /// Runs immediately before tool lookup, validation, approval, and execution orchestration.
    ///
    /// Arguments can be rewritten safely by [`Self::after_llm_call`] before the assistant message
    /// is committed; this callback intentionally receives the committed call read-only.
    ///
    /// # Errors
    ///
    /// Returns a hook error to stop the loop before tool orchestration begins.
    async fn before_tool_call(
        &self,
        _context: ToolCallHookContext<'_>,
    ) -> Result<(), AgentLoopHookError> {
        Ok(())
    }

    /// Rewrites or inspects the model-visible JSON result before it is durably committed.
    ///
    /// This runs for successful tool output and model-visible lookup, validation, approval, or tool
    /// errors. It does not run when orchestration itself fails without producing a tool message.
    /// If this callback fails after a tool was dispatched, the runtime durably commits the original
    /// unmodified result before failing the loop so the external outcome is not lost.
    ///
    /// # Errors
    ///
    /// Returns a hook error to stop the loop before the tool result message is committed.
    async fn after_tool_call(
        &self,
        _context: ToolCallHookContext<'_>,
        _result: &mut Value,
    ) -> Result<(), AgentLoopHookError> {
        Ok(())
    }

    /// Runs after all iteration messages are committed but before `IterationCompleted`.
    ///
    /// # Errors
    ///
    /// Returns a hook error to prevent the durable iteration frontier from advancing.
    async fn after_iteration(
        &self,
        _context: IterationHookContext<'_>,
    ) -> Result<(), AgentLoopHookError> {
        Ok(())
    }
}
