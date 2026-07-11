//! Public LLM provider definitions.

use std::pin::Pin;

use async_trait::async_trait;
use futures_core::Stream;
use serde::{Deserialize, Serialize};
use wyse_core::{ModelId, TokenUsage};

use crate::{ChatMessage, LlmError, StructuredOutput, ToolCallDelta, ToolSpec};

/// Stream of chat events produced by a provider.
pub type ChatStream =
    Pin<Box<dyn Stream<Item = Result<ChatStreamEvent, LlmError>> + Send + 'static>>;

/// Provider capable of chat completion requests.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Returns the model bound to this provider.
    fn model_id(&self) -> ModelId;

    /// Sends a non-streaming chat request.
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError>;

    /// Sends a streaming chat request.
    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError>;
}

/// Request for a chat completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatRequest {
    /// Model that should answer the request.
    pub model: ModelId,
    /// Conversation messages sent to the model.
    pub messages: Vec<ChatMessage>,
    /// Tools available to the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<ToolSpec>,
    /// Structured output constraint for the response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_output: Option<StructuredOutput>,
}

impl ChatRequest {
    /// Creates an empty request for a model.
    #[must_use]
    pub fn new(model: ModelId) -> Self {
        Self {
            model,
            messages: Vec::new(),
            tools: Vec::new(),
            structured_output: None,
        }
    }

    /// Appends a message to the request.
    #[must_use]
    pub fn with_message(mut self, message: ChatMessage) -> Self {
        self.messages.push(message);
        self
    }

    /// Sets the structured output constraint.
    #[must_use]
    pub fn with_structured_output(mut self, structured_output: StructuredOutput) -> Self {
        self.structured_output = Some(structured_output);
        self
    }
}

/// Non-streaming chat completion response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChatResponse {
    /// Assistant message returned by the provider.
    pub message: ChatMessage,
    /// Reason the provider stopped generating.
    pub finish_reason: FinishReason,
    /// Token usage reported by the provider.
    pub usage: Option<TokenUsage>,
}

/// Incremental event from a streaming chat response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
#[non_exhaustive]
pub enum ChatStreamEvent {
    /// Text emitted by the model.
    TextDelta {
        /// Text fragment.
        delta: String,
    },
    /// Reasoning text emitted by the model.
    ReasoningDelta {
        /// Reasoning text fragment.
        delta: String,
    },
    /// Tool-call fragment emitted by the model.
    ToolCallDelta(ToolCallDelta),
    /// Terminal stream event.
    Finished {
        /// Reason the stream finished.
        finish_reason: FinishReason,
        /// Token usage reported at stream end.
        usage: Option<TokenUsage>,
    },
}

/// Reason a chat response finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FinishReason {
    /// Model reached a natural stop.
    Stop,
    /// Model reached a length limit.
    Length,
    /// Model requested tool calls.
    ToolCalls,
    /// Provider content filter stopped output.
    ContentFilter,
    /// Provider returned an unmapped reason.
    Unknown,
}

#[cfg(test)]
mod tests {
    use crate::{ChatMessage, ChatRequest, StructuredOutput};
    use serde_json::json;

    #[test]
    fn chat_request_uses_model_id_and_messages() {
        let request = ChatRequest::new("openai:gpt-4.1-mini".parse().expect("model id parses"))
            .with_message(ChatMessage::user("hello"))
            .with_structured_output(StructuredOutput::JsonSchema {
                name: "answer".to_owned(),
                schema: json!({"type": "object"}),
                strict: true,
            });

        assert_eq!(request.model.as_str(), "openai:gpt-4.1-mini");
        assert_eq!(request.messages.len(), 1);
        assert!(request.structured_output.is_some());
    }

    #[test]
    fn stream_event_can_represent_reasoning_delta() {
        let event = crate::ChatStreamEvent::ReasoningDelta {
            delta: "thinking".to_owned(),
        };

        assert_eq!(
            serde_json::to_value(event).expect("event should serialize"),
            serde_json::json!({
                "type": "reasoning_delta",
                "data": { "delta": "thinking" }
            })
        );
    }
}
