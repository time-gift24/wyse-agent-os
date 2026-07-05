//! LLM provider abstractions and protocol adapters for Wyse.

pub mod definition;
pub mod error;
pub mod message;
pub mod mock;
pub mod structured_output;
pub mod tool_call;
pub mod usage;

pub use definition::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmProvider,
};
pub use error::{ApiKey, LlmError, ProviderStatusError};
pub use message::{ChatContent, ChatMessage, ChatRole};
pub use mock::MockLlmProvider;
pub use structured_output::StructuredOutput;
pub use tool_call::{ToolCall, ToolCallDelta, ToolChoice, ToolSpec};
pub use usage::{CostEstimate, TokenPrices};
