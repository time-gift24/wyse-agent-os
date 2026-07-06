//! DeepSeek protocol implementation.

use std::{collections::VecDeque, pin::Pin};

use futures_core::Stream;
use futures_util::{StreamExt, stream};
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use wyse_core::ModelId;

use crate::{
    ApiKey, ChatMessage, ChatRequest, ChatResponse, ChatRole, ChatStream, ChatStreamEvent,
    FinishReason, LlmError, LlmProvider, StructuredOutput,
    protocol::openai_compatible::{
        finish_reason, provider_status_error, request_id, to_chat_payload,
        tool_call_delta_from_value, tool_calls_from_message, usage_from_value,
    },
    protocol::sse::{SseEvent, SseParser, stream_eof_error},
};

/// DeepSeek chat completions provider.
#[derive(Debug, Clone)]
pub struct DeepSeekProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: ApiKey,
    model: DeepSeekModel,
    thinking: DeepSeekThinking,
}

impl DeepSeekProvider {
    /// Creates a provider using an explicit base URL.
    #[must_use]
    pub fn new(
        base_url: impl Into<String>,
        api_key: ApiKey,
        model: DeepSeekModel,
        thinking: DeepSeekThinking,
    ) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key,
            model,
            thinking,
        }
    }

    fn chat_completions_url(&self) -> Result<Url, LlmError> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        Url::parse(&url).map_err(|source| LlmError::RequestBuild(Box::new(source)))
    }

    fn headers(&self) -> Result<HeaderMap, LlmError> {
        let mut headers = HeaderMap::new();
        let value = format!("Bearer {}", self.api_key.as_str());
        let mut auth = HeaderValue::from_str(&value)
            .map_err(|source| LlmError::RequestBuild(Box::new(source)))?;
        auth.set_sensitive(true);
        headers.insert(AUTHORIZATION, auth);
        Ok(headers)
    }
}

impl LlmProvider for DeepSeekProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        if request.model != self.model.model_id() {
            return Err(LlmError::InvalidRequest(
                "request model does not match provider model",
            ));
        }

        let payload = to_deepseek_chat_payload(&request, self.thinking, false)?;
        let response = self
            .client
            .post(self.chat_completions_url()?)
            .headers(self.headers()?)
            .json(&payload)
            .send()
            .await
            .map_err(LlmError::transport)?;
        let status = response.status();
        let request_id = request_id(response.headers());
        let body = response.bytes().await.map_err(LlmError::transport)?;

        if !status.is_success() {
            let value = serde_json::from_slice(&body).map_err(LlmError::ProviderPayloadDecode)?;
            return Err(LlmError::ProviderStatus(provider_status_error(
                status.as_u16(),
                value,
                request_id,
                &self.api_key,
            )));
        }

        let value = serde_json::from_slice(&body).map_err(LlmError::ResponseDecode)?;
        deepseek_chat_response_from_value(value)
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        if request.model != self.model.model_id() {
            return Err(LlmError::InvalidRequest(
                "request model does not match provider model",
            ));
        }

        let payload = to_deepseek_chat_payload(&request, self.thinking, true)?;
        let response = self
            .client
            .post(self.chat_completions_url()?)
            .headers(self.headers()?)
            .json(&payload)
            .send()
            .await
            .map_err(LlmError::transport)?;
        let status = response.status();
        let request_id = request_id(response.headers());

        if !status.is_success() {
            let body = response.bytes().await.map_err(LlmError::transport)?;
            let value = serde_json::from_slice(&body).map_err(LlmError::ProviderPayloadDecode)?;
            return Err(LlmError::ProviderStatus(provider_status_error(
                status.as_u16(),
                value,
                request_id,
                &self.api_key,
            )));
        }

        Ok(deepseek_chat_stream(response.bytes_stream()))
    }
}

fn deepseek_chat_stream<S>(chunks: S) -> ChatStream
where
    S: Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
{
    struct State<S> {
        chunks: Pin<Box<S>>,
        parser: SseParser,
        pending: VecDeque<Result<ChatStreamEvent, LlmError>>,
        finished: bool,
        terminal_seen: bool,
    }

    let state = State {
        chunks: Box::pin(chunks),
        parser: SseParser::default(),
        pending: VecDeque::new(),
        finished: false,
        terminal_seen: false,
    };

    Box::pin(stream::unfold(state, |mut state| async move {
        loop {
            if let Some(event) = state.pending.pop_front() {
                return Some((event, state));
            }

            if state.finished {
                return None;
            }

            match state.chunks.as_mut().next().await {
                Some(Ok(chunk)) => {
                    for event in state.parser.push(&chunk) {
                        if state.finished {
                            break;
                        }

                        match event {
                            Ok(SseEvent::Data(data)) => match stream_events_from_sse_data(&data) {
                                Ok(mapped) => {
                                    for mapped_event in mapped {
                                        let is_terminal = matches!(
                                            mapped_event,
                                            ChatStreamEvent::Finished { .. }
                                        );
                                        state.pending.push_back(Ok(mapped_event));
                                        if is_terminal {
                                            state.terminal_seen = true;
                                            state.finished = true;
                                            break;
                                        }
                                    }
                                }
                                Err(error) => {
                                    state.pending.push_back(Err(error));
                                    state.finished = true;
                                }
                            },
                            Ok(SseEvent::Done) => {
                                if !state.terminal_seen {
                                    state.pending.push_back(Ok(ChatStreamEvent::Finished {
                                        finish_reason: FinishReason::Unknown,
                                        usage: None,
                                    }));
                                    state.terminal_seen = true;
                                }
                                state.finished = true;
                            }
                            Err(error) => {
                                state.pending.push_back(Err(error));
                                state.finished = true;
                            }
                        }
                    }
                }
                Some(Err(source)) => {
                    state.pending.push_back(Err(LlmError::stream(source)));
                    state.finished = true;
                }
                None => {
                    if state.parser.has_pending() {
                        state.pending.push_back(Err(stream_eof_error(
                            "stream ended with partial sse event",
                        )));
                    } else if !state.terminal_seen {
                        state
                            .pending
                            .push_back(Err(stream_eof_error("stream ended before finish event")));
                    }
                    state.finished = true;
                }
            }
        }
    }))
}

/// DeepSeek chat model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeepSeekModel {
    /// DeepSeek V4 Flash.
    V4Flash,
    /// DeepSeek V4 Pro.
    V4Pro,
}

impl DeepSeekModel {
    /// Returns the provider model id string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::V4Flash => "deepseek-v4-flash",
            Self::V4Pro => "deepseek-v4-pro",
        }
    }

    /// Returns the model id newtype for request construction.
    #[must_use]
    pub fn model_id(self) -> ModelId {
        ModelId::from(self.as_str())
    }
}

/// DeepSeek thinking mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeepSeekThinking {
    /// Enable thinking, optionally with reasoning effort.
    Enabled {
        /// Requested reasoning effort.
        effort: Option<DeepSeekReasoningEffort>,
    },
    /// Disable thinking.
    Disabled,
}

/// DeepSeek reasoning effort.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeepSeekReasoningEffort {
    /// High reasoning effort.
    High,
    /// Maximum reasoning effort.
    Max,
}

fn to_deepseek_chat_payload(
    request: &ChatRequest,
    thinking: DeepSeekThinking,
    stream: bool,
) -> Result<Value, LlmError> {
    if matches!(
        &request.structured_output,
        Some(StructuredOutput::JsonSchema { .. })
    ) {
        return Err(LlmError::UnsupportedCapability(
            "deepseek json schema structured output",
        ));
    }

    let mut payload = to_chat_payload(request, stream)?;
    add_reasoning_content(&mut payload, request);

    match thinking {
        DeepSeekThinking::Enabled { effort } => {
            payload["thinking"] = json!({"type": "enabled"});
            if let Some(effort) = effort {
                payload["reasoning_effort"] = Value::String(reasoning_effort(effort).to_owned());
            }
        }
        DeepSeekThinking::Disabled => {
            payload["thinking"] = json!({"type": "disabled"});
        }
    }

    Ok(payload)
}

fn stream_events_from_sse_data(data: &str) -> Result<Vec<ChatStreamEvent>, LlmError> {
    let value = serde_json::from_str::<Value>(data).map_err(LlmError::stream)?;
    let choice = value["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or(LlmError::InvalidProviderPayload("missing choice"))?;
    let mut events = Vec::new();

    if let Some(delta) = choice["delta"]["reasoning_content"].as_str()
        && !delta.is_empty()
    {
        events.push(ChatStreamEvent::ReasoningDelta {
            delta: delta.to_owned(),
        });
    }

    if let Some(delta) = choice["delta"]["content"].as_str()
        && !delta.is_empty()
    {
        events.push(ChatStreamEvent::TextDelta {
            delta: delta.to_owned(),
        });
    }

    if let Some(tool_calls) = choice["delta"]["tool_calls"].as_array() {
        for call in tool_calls {
            events.push(ChatStreamEvent::ToolCallDelta(tool_call_delta_from_value(
                call,
            )?));
        }
    }

    if let Some(reason) = choice["finish_reason"].as_str() {
        events.push(ChatStreamEvent::Finished {
            finish_reason: finish_reason(Some(reason)),
            usage: usage_from_value(value.get("usage")),
        });
    }

    Ok(events)
}

fn deepseek_chat_response_from_value(value: Value) -> Result<ChatResponse, LlmError> {
    let choice = value["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or(LlmError::InvalidProviderPayload("missing choice"))?;
    let message = &choice["message"];
    let content = message["content"].as_str().unwrap_or_default();
    let mut chat_message = ChatMessage::assistant(content);

    if let Some(reasoning_content) = message["reasoning_content"].as_str()
        && !reasoning_content.is_empty()
    {
        chat_message = chat_message.with_reasoning_content(reasoning_content);
    }

    chat_message.tool_calls = tool_calls_from_message(message)?;

    Ok(ChatResponse {
        message: chat_message,
        finish_reason: finish_reason(choice["finish_reason"].as_str()),
        usage: usage_from_value(value.get("usage")),
    })
}

fn add_reasoning_content(payload: &mut Value, request: &ChatRequest) {
    let Some(messages) = payload["messages"].as_array_mut() else {
        return;
    };

    for (value, message) in messages.iter_mut().zip(&request.messages) {
        if message.role == ChatRole::Assistant
            && let Some(reasoning_content) = &message.reasoning_content
        {
            value["reasoning_content"] = Value::String(reasoning_content.clone());
        }
    }
}

const fn reasoning_effort(effort: DeepSeekReasoningEffort) -> &'static str {
    match effort {
        DeepSeekReasoningEffort::High => "high",
        DeepSeekReasoningEffort::Max => "max",
    }
}
