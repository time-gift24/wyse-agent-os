//! DeepSeek protocol implementation.

use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use wyse_core::ModelId;

use crate::{
    ApiKey, ChatRequest, ChatResponse, ChatRole, ChatStream, LlmError, LlmProvider,
    protocol::openai_compatible::{
        chat_response_from_value, provider_status_error, request_id, to_chat_payload,
    },
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

        let payload = to_deepseek_chat_payload(&request, self.thinking)?;
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
        chat_response_from_value(value)
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        if request.model != self.model.model_id() {
            return Err(LlmError::InvalidRequest(
                "request model does not match provider model",
            ));
        }

        Err(LlmError::UnsupportedCapability("deepseek streaming chat"))
    }
}

/// DeepSeek chat model.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
pub enum DeepSeekReasoningEffort {
    /// High reasoning effort.
    High,
    /// Maximum reasoning effort.
    Max,
}

fn to_deepseek_chat_payload(
    request: &ChatRequest,
    thinking: DeepSeekThinking,
) -> Result<Value, LlmError> {
    let mut payload = to_chat_payload(request, false)?;
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
