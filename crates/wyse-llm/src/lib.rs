//! LLM provider abstractions and protocol adapters for Wyse.

pub mod definition;
pub mod error;
pub mod message;
pub mod structured_output;
pub mod tool_call;

pub use definition::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmProvider,
};
pub use error::LlmError;
pub use message::{ChatContent, ChatMessage, ChatRole};
pub use structured_output::StructuredOutput;
pub use tool_call::{ToolCall, ToolCallDelta, ToolChoice, ToolSpec};
