//! Context, limits, and successful outcome types for the agent loop kernel.

use stratum_core::{ChatMessage, TokenUsage};
use stratum_llm::FinishReason;

/// Committed conversation state supplied to an agent loop run.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LoopContext {
    /// Instruction prepended to the model conversation.
    pub system_prompt: String,
    /// Complete committed transcript in provider order.
    pub messages: Vec<ChatMessage>,
}

impl LoopContext {
    /// Creates an empty loop context with the provided system instruction.
    #[must_use]
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            messages: Vec::new(),
        }
    }

    /// Moves a committed transcript into this context.
    #[must_use]
    pub fn with_messages(mut self, messages: Vec<ChatMessage>) -> Self {
        self.messages = messages;
        self
    }
}

/// Safety bounds applied before the loop starts additional work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoopLimits {
    /// Maximum number of model iterations in one run.
    pub max_iterations: usize,
    /// Maximum tool calls accepted from one model iteration.
    pub max_tool_calls_per_iteration: usize,
    /// Maximum streamed assistant text bytes in one model response.
    pub max_text_bytes: usize,
    /// Maximum streamed reasoning bytes in one model response.
    pub max_reasoning_bytes: usize,
    /// Maximum streamed argument bytes for one tool call.
    pub max_tool_argument_bytes: usize,
}

impl LoopLimits {
    const DEFAULT_MAX_TEXT_BYTES: usize = 1024 * 1024;
    const DEFAULT_MAX_REASONING_BYTES: usize = 1024 * 1024;
    const DEFAULT_MAX_TOOL_ARGUMENT_BYTES: usize = 256 * 1024;

    /// Creates loop safety bounds.
    #[must_use]
    pub const fn new(max_iterations: usize, max_tool_calls_per_iteration: usize) -> Self {
        Self {
            max_iterations,
            max_tool_calls_per_iteration,
            max_text_bytes: Self::DEFAULT_MAX_TEXT_BYTES,
            max_reasoning_bytes: Self::DEFAULT_MAX_REASONING_BYTES,
            max_tool_argument_bytes: Self::DEFAULT_MAX_TOOL_ARGUMENT_BYTES,
        }
    }

    /// Overrides the streamed response byte limits.
    #[must_use]
    pub const fn with_stream_byte_limits(
        mut self,
        max_text_bytes: usize,
        max_reasoning_bytes: usize,
        max_tool_argument_bytes: usize,
    ) -> Self {
        self.max_text_bytes = max_text_bytes;
        self.max_reasoning_bytes = max_reasoning_bytes;
        self.max_tool_argument_bytes = max_tool_argument_bytes;
        self
    }
}

impl Default for LoopLimits {
    fn default() -> Self {
        Self::new(16, 16)
    }
}

/// Successful terminal result returned by the agent loop kernel.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LoopOutcome {
    /// Messages committed during this loop run.
    pub new_messages: Vec<ChatMessage>,
    /// Reason the final model response completed.
    pub finish_reason: FinishReason,
    /// Aggregate model token usage for this loop run.
    pub usage: TokenUsage,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_preserve_context_and_default_limits() {
        let transcript = vec![ChatMessage::user("hello"), ChatMessage::assistant("hi")];
        let context = LoopContext::new("be helpful").with_messages(transcript);

        assert_eq!(context.system_prompt, "be helpful");
        assert_eq!(
            context.messages,
            vec![ChatMessage::user("hello"), ChatMessage::assistant("hi"),]
        );
        assert_eq!(LoopLimits::default(), LoopLimits::new(16, 16));
    }
}
