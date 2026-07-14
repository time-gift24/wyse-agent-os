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
}

impl LoopLimits {
    /// Creates loop safety bounds.
    #[must_use]
    pub const fn new(max_iterations: usize, max_tool_calls_per_iteration: usize) -> Self {
        Self {
            max_iterations,
            max_tool_calls_per_iteration,
        }
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
    fn new_context_starts_with_an_empty_transcript() {
        let context = LoopContext::new("answer concisely");

        assert_eq!(context.system_prompt, "answer concisely");
        assert!(context.messages.is_empty());
    }

    #[test]
    fn with_messages_preserves_the_complete_transcript() {
        let transcript = vec![ChatMessage::user("hello"), ChatMessage::assistant("hi")];

        let context = LoopContext::new("be helpful").with_messages(transcript);

        assert_eq!(
            context.messages,
            vec![ChatMessage::user("hello"), ChatMessage::assistant("hi"),]
        );
    }

    #[test]
    fn default_limits_bound_iterations_and_tool_calls() {
        let limits = LoopLimits::default();

        assert_eq!(limits.max_iterations, 16);
        assert_eq!(limits.max_tool_calls_per_iteration, 16);
    }

    #[test]
    fn custom_limits_preserve_both_bounds() {
        let limits = LoopLimits::new(3, 5);

        assert_eq!(limits.max_iterations, 3);
        assert_eq!(limits.max_tool_calls_per_iteration, 5);
    }
}
