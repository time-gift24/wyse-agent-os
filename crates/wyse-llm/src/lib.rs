//! LLM provider abstractions and protocol adapters for Wyse.

pub mod definition;
pub mod error;
pub mod manager;
pub mod message;
pub mod mock;
pub mod protocol;
pub mod structured_output;
pub mod tool_call;

pub use definition::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmProvider,
};
pub use error::{ApiKey, LlmError, ProviderStatusError};
pub use manager::LlmProviderManager;
pub use message::{ChatContent, ChatMessage, ChatRole};
pub use mock::MockLlmProvider;
pub use protocol::deepseek::{
    DeepSeekModel, DeepSeekProvider, DeepSeekReasoningEffort, DeepSeekThinking,
};
pub use protocol::openai_compatible::OpenAICompatibleProvider;
pub use structured_output::StructuredOutput;
pub use tool_call::{ToolCall, ToolCallDelta};
pub use wyse_core::ToolSpec;
