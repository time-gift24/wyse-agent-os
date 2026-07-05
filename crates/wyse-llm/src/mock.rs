//! Mock LLM provider for tests.

use std::{collections::VecDeque, sync::Mutex};

use futures_util::stream;

use crate::{ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, LlmError, LlmProvider};

/// Queue-backed mock provider for deterministic tests.
#[derive(Debug, Default)]
pub struct MockLlmProvider {
    chat_responses: Mutex<VecDeque<ChatResponse>>,
    stream_responses: Mutex<VecDeque<Vec<ChatStreamEvent>>>,
}

impl MockLlmProvider {
    /// Creates an empty mock provider.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a queued non-streaming chat response.
    #[must_use]
    pub fn with_chat_response(self, response: ChatResponse) -> Self {
        self.chat_responses
            .lock()
            .expect("mock mutex should not be poisoned")
            .push_back(response);
        self
    }

    /// Adds a queued streaming response.
    #[must_use]
    pub fn with_stream_events(self, events: Vec<ChatStreamEvent>) -> Self {
        self.stream_responses
            .lock()
            .expect("mock mutex should not be poisoned")
            .push_back(events);
        self
    }
}

impl LlmProvider for MockLlmProvider {
    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        self.chat_responses
            .lock()
            .expect("mock mutex should not be poisoned")
            .pop_front()
            .ok_or(LlmError::MockExhausted)
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        let events = self
            .stream_responses
            .lock()
            .expect("mock mutex should not be poisoned")
            .pop_front()
            .ok_or(LlmError::MockExhausted)?;

        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;
    use wyse_core::ModelId;

    use crate::{
        ChatMessage, ChatRequest, ChatResponse, ChatStreamEvent, FinishReason, LlmError,
        LlmProvider, MockLlmProvider,
    };

    #[tokio::test]
    async fn mock_returns_queued_chat_response() {
        let provider = MockLlmProvider::new().with_chat_response(ChatResponse {
            message: ChatMessage::assistant("hello"),
            finish_reason: FinishReason::Stop,
            usage: None,
        });

        let response = provider
            .chat(ChatRequest::new(ModelId::from("mock")))
            .await
            .expect("mock response should exist");

        assert_eq!(response.message, ChatMessage::assistant("hello"));
    }

    #[tokio::test]
    async fn mock_returns_stream_events_in_order() {
        let provider = MockLlmProvider::new().with_stream_events(vec![
            ChatStreamEvent::TextDelta {
                delta: "he".to_owned(),
            },
            ChatStreamEvent::TextDelta {
                delta: "llo".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]);

        let mut stream = provider
            .chat_stream(ChatRequest::new(ModelId::from("mock")))
            .await
            .expect("mock stream should exist");

        let first = stream.next().await.transpose().expect("first event");
        let second = stream.next().await.transpose().expect("second event");

        assert_eq!(
            first,
            Some(ChatStreamEvent::TextDelta {
                delta: "he".to_owned()
            })
        );
        assert_eq!(
            second,
            Some(ChatStreamEvent::TextDelta {
                delta: "llo".to_owned()
            })
        );
    }

    #[tokio::test]
    async fn mock_reports_exhausted_queue() {
        let provider = MockLlmProvider::new();

        let error = provider
            .chat(ChatRequest::new(ModelId::from("mock")))
            .await
            .expect_err("queue should be empty");

        assert!(matches!(error, LlmError::MockExhausted));
    }
}
