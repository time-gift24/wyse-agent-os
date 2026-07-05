//! OpenAI-compatible protocol implementation.
use reqwest::{
    Url,
    header::{AUTHORIZATION, HeaderMap, HeaderValue},
};
use serde_json::{Value, json};
use wyse_core::{CallId, ModelId, TokenUsage, ToolId};

use crate::{
    ApiKey, ChatContent, ChatMessage, ChatRequest, ChatResponse, ChatRole, ChatStream,
    FinishReason, LlmError, LlmProvider, ProviderStatusError, StructuredOutput, ToolCall,
    ToolChoice, ToolSpec,
};

/// OpenAI-compatible chat completions provider.
#[derive(Debug, Clone)]
pub struct OpenAICompatibleProvider {
    client: reqwest::Client,
    base_url: String,
    api_key: ApiKey,
    model: ModelId,
}

impl OpenAICompatibleProvider {
    /// Creates a provider using a default reqwest client.
    #[must_use]
    pub fn new(base_url: impl Into<String>, api_key: ApiKey, model: ModelId) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            api_key,
            model,
        }
    }

    /// Replaces the HTTP client used by the provider.
    #[must_use]
    pub fn with_client(mut self, client: reqwest::Client) -> Self {
        self.client = client;
        self
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

impl LlmProvider for OpenAICompatibleProvider {
    async fn chat(&self, request: ChatRequest) -> Result<ChatResponse, LlmError> {
        if request.model != self.model {
            return Err(LlmError::InvalidRequest(
                "request model does not match provider model",
            ));
        }

        let payload = to_chat_payload(&request, false)?;
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
        chat_response_from_value(value, &request.tools)
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        Err(LlmError::UnsupportedCapability("streaming"))
    }
}

pub(crate) fn to_chat_payload(request: &ChatRequest, stream: bool) -> Result<Value, LlmError> {
    let messages = request
        .messages
        .iter()
        .map(|message| message_to_value(message, &request.tools))
        .collect::<Result<Vec<_>, _>>()?;
    let mut payload = json!({
        "model": request.model.as_str(),
        "messages": messages,
        "stream": stream,
    });

    if !request.tools.is_empty() {
        payload["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": tool.name,
                            "description": tool.description,
                            "parameters": tool.input_schema,
                        }
                    })
                })
                .collect(),
        );
    }

    if let Some(choice) = &request.tool_choice {
        payload["tool_choice"] = tool_choice_to_value(choice, &request.tools)?;
    }

    if let Some(output) = &request.structured_output {
        payload["response_format"] = structured_output_to_value(output);
    }

    Ok(payload)
}

pub(crate) fn chat_response_from_value(
    value: Value,
    tools: &[ToolSpec],
) -> Result<ChatResponse, LlmError> {
    let choice = value["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or(LlmError::InvalidProviderPayload("missing choice"))?;
    let message = &choice["message"];
    let content = message["content"].as_str().unwrap_or_default();
    let finish_reason = finish_reason(choice["finish_reason"].as_str());
    let usage = usage_from_value(value.get("usage"));
    let mut chat_message = ChatMessage::assistant(content);
    chat_message.tool_calls = tool_calls_from_message(message, tools)?;

    Ok(ChatResponse {
        message: chat_message,
        finish_reason,
        usage,
    })
}

fn request_id(headers: &HeaderMap) -> Option<String> {
    headers
        .get("x-request-id")
        .or_else(|| headers.get("request-id"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn provider_status_error(
    status: u16,
    value: Value,
    request_id: Option<String>,
    api_key: &ApiKey,
) -> ProviderStatusError {
    let error = value.get("error").unwrap_or(&value);
    let code = error
        .get("code")
        .and_then(error_field)
        .or_else(|| error.get("type").and_then(error_field))
        .map(|value| redact_secret(&value, api_key));
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| value.get("message").and_then(Value::as_str))
        .unwrap_or("provider request failed");

    ProviderStatusError::new(status, code, redact_secret(message, api_key), request_id)
}

fn error_field(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(value) if value.is_empty() => None,
        Value::String(value) => Some(value.clone()),
        value => Some(value.to_string()),
    }
}

fn redact_secret(value: &str, api_key: &ApiKey) -> String {
    value.replace(api_key.as_str(), "[redacted]")
}

fn message_to_value(message: &ChatMessage, tools: &[ToolSpec]) -> Result<Value, LlmError> {
    let role = match message.role {
        ChatRole::System => "system",
        ChatRole::User => "user",
        ChatRole::Assistant => "assistant",
        ChatRole::Tool => "tool",
    };
    let content = match &message.content {
        ChatContent::Text(text) => Value::String(text.clone()),
        ChatContent::Json(value) => value.clone(),
    };

    let mut value = json!({"role": role, "content": content});

    if message.role == ChatRole::Assistant && !message.tool_calls.is_empty() {
        value["tool_calls"] = Value::Array(
            message
                .tool_calls
                .iter()
                .map(tool_call_to_value)
                .collect::<Result<Vec<_>, _>>()?,
        );
    }

    if message.role == ChatRole::Tool {
        if let Some(call_id) = &message.tool_call_id {
            value["tool_call_id"] = Value::String(call_id.as_str().to_owned());
        }

        if let Some(tool_id) = &message.tool_id
            && let Some(name) = provider_tool_name(tool_id, tools)
        {
            value["name"] = Value::String(name.to_owned());
        }
    }

    Ok(value)
}

fn tool_call_to_value(tool_call: &ToolCall) -> Result<Value, LlmError> {
    let arguments = serde_json::to_string(&tool_call.arguments)
        .map_err(|source| LlmError::RequestBuild(Box::new(source)))?;

    Ok(json!({
        "id": tool_call.call_id.as_str(),
        "type": "function",
        "function": {
            "name": tool_call.name,
            "arguments": arguments
        }
    }))
}

fn tool_choice_to_value(choice: &ToolChoice, tools: &[ToolSpec]) -> Result<Value, LlmError> {
    match choice {
        ToolChoice::Auto => Ok(Value::String("auto".to_owned())),
        ToolChoice::None => Ok(Value::String("none".to_owned())),
        ToolChoice::Required => Ok(Value::String("required".to_owned())),
        ToolChoice::Tool(tool_id) => {
            let name = provider_tool_name(tool_id, tools)
                .ok_or(LlmError::InvalidProviderPayload("unknown tool choice"))?;
            Ok(json!({
                "type": "function",
                "function": {"name": name}
            }))
        }
    }
}

fn provider_tool_name<'a>(tool_id: &ToolId, tools: &'a [ToolSpec]) -> Option<&'a str> {
    tools
        .iter()
        .find(|tool| &tool.tool_id == tool_id)
        .map(|tool| tool.name.as_str())
}

fn tool_id_for_provider_name(name: &str, tools: &[ToolSpec]) -> Option<ToolId> {
    tools
        .iter()
        .find(|tool| tool.name == name)
        .map(|tool| tool.tool_id.clone())
}

fn structured_output_to_value(output: &StructuredOutput) -> Value {
    match output {
        StructuredOutput::JsonObject => json!({"type": "json_object"}),
        StructuredOutput::JsonSchema {
            name,
            schema,
            strict,
        } => json!({
            "type": "json_schema",
            "json_schema": {
                "name": name,
                "schema": schema,
                "strict": strict
            }
        }),
    }
}

fn finish_reason(value: Option<&str>) -> FinishReason {
    match value {
        Some("stop") => FinishReason::Stop,
        Some("length") => FinishReason::Length,
        Some("tool_calls") => FinishReason::ToolCalls,
        Some("content_filter") => FinishReason::ContentFilter,
        _ => FinishReason::Unknown,
    }
}

fn usage_from_value(value: Option<&Value>) -> Option<TokenUsage> {
    let value = value?;
    Some(TokenUsage {
        input_tokens: value["prompt_tokens"].as_u64().unwrap_or_default(),
        output_tokens: value["completion_tokens"].as_u64().unwrap_or_default(),
        total_tokens: value["total_tokens"].as_u64().unwrap_or_default(),
    })
}

fn tool_calls_from_message(message: &Value, tools: &[ToolSpec]) -> Result<Vec<ToolCall>, LlmError> {
    let Some(value) = message.get("tool_calls") else {
        return Ok(Vec::new());
    };

    let calls = value
        .as_array()
        .ok_or(LlmError::InvalidProviderPayload("invalid tool calls"))?;

    calls
        .iter()
        .map(|call| {
            let call_id = required_str(&call["id"], "missing tool call id")?;
            let name = required_str(&call["function"]["name"], "missing tool name")?;
            let arguments = required_str(&call["function"]["arguments"], "missing tool arguments")?;
            let tool_id = tool_id_for_provider_name(name, tools)
                .ok_or(LlmError::InvalidProviderPayload("unknown tool call"))?;

            Ok(ToolCall {
                call_id: CallId::from(call_id),
                tool_id,
                name: name.to_owned(),
                arguments: serde_json::from_str(arguments).map_err(LlmError::ResponseDecode)?,
            })
        })
        .collect()
}

fn required_str<'a>(value: &'a Value, message: &'static str) -> Result<&'a str, LlmError> {
    value
        .as_str()
        .filter(|value| !value.is_empty())
        .ok_or(LlmError::InvalidProviderPayload(message))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wyse_core::{CallId, ModelId, ToolId};

    use super::*;
    use crate::{
        ChatMessage, ChatRequest, ChatRole, FinishReason, LlmError, StructuredOutput, ToolCall,
        ToolChoice, ToolSpec,
    };

    #[test]
    fn request_maps_messages_tools_and_json_schema() {
        let request = ChatRequest::new(ModelId::from("gpt-4.1-mini"))
            .with_message(ChatMessage::system("be brief"))
            .with_message(ChatMessage::user("answer"));
        let request = ChatRequest {
            tools: vec![ToolSpec {
                tool_id: ToolId::from("weather"),
                name: "weather".to_owned(),
                description: "get weather".to_owned(),
                input_schema: json!({"type": "object"}),
            }],
            tool_choice: Some(ToolChoice::Required),
            structured_output: Some(StructuredOutput::JsonSchema {
                name: "answer".to_owned(),
                schema: json!({"type": "object"}),
                strict: true,
            }),
            ..request
        };

        let payload = to_chat_payload(&request, false).expect("payload maps");

        assert_eq!(payload["model"], "gpt-4.1-mini");
        assert_eq!(payload["messages"][0]["role"], "system");
        assert_eq!(payload["tools"][0]["function"]["name"], "weather");
        assert_eq!(payload["tool_choice"], "required");
        assert_eq!(payload["response_format"]["type"], "json_schema");
    }

    #[test]
    fn response_maps_text_usage_and_finish_reason() {
        let payload = json!({
            "choices": [{
                "message": {"role": "assistant", "content": "hello"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 2,
                "completion_tokens": 3,
                "total_tokens": 5
            }
        });

        let response = chat_response_from_value(payload, &[]).expect("response maps");

        assert_eq!(response.message, ChatMessage::assistant("hello"));
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert_eq!(response.usage.expect("usage").total_tokens, 5);
    }

    #[test]
    fn response_maps_tool_calls_to_message() {
        let payload = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"city\":\"Shanghai\"}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let response = chat_response_from_value(payload, &[weather_tool()]).expect("response maps");

        assert_eq!(response.finish_reason, FinishReason::ToolCalls);
        assert_eq!(response.message.tool_calls.len(), 1);
        assert_eq!(
            response.message.tool_calls[0].call_id,
            CallId::from("call-1")
        );
        assert_eq!(
            response.message.tool_calls[0].tool_id,
            ToolId::from("internal-weather")
        );
        assert_eq!(response.message.tool_calls[0].name, "get_weather");
        assert_eq!(
            response.message.tool_calls[0].arguments,
            json!({"city": "Shanghai"})
        );
    }

    #[test]
    fn unknown_response_tool_name_returns_mapping_error() {
        let payload = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "unknown_tool",
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let error =
            chat_response_from_value(payload, &[weather_tool()]).expect_err("tool should fail");

        assert!(matches!(
            error,
            LlmError::InvalidProviderPayload("unknown tool call")
        ));
    }

    #[test]
    fn invalid_response_tool_arguments_return_decode_error() {
        let payload = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{not json"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let error =
            chat_response_from_value(payload, &[weather_tool()]).expect_err("tool should fail");

        assert!(matches!(error, LlmError::ResponseDecode(_)));
    }

    #[test]
    fn response_tool_call_requires_id_and_name() {
        let missing_id = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });
        let missing_name = json!({
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call-1",
                        "type": "function",
                        "function": {
                            "arguments": "{}"
                        }
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let missing_id_error = chat_response_from_value(missing_id, &[weather_tool()])
            .expect_err("missing id should fail");
        let missing_name_error = chat_response_from_value(missing_name, &[weather_tool()])
            .expect_err("missing name should fail");

        assert!(matches!(
            missing_id_error,
            LlmError::InvalidProviderPayload("missing tool call id")
        ));
        assert!(matches!(
            missing_name_error,
            LlmError::InvalidProviderPayload("missing tool name")
        ));
    }

    #[test]
    fn response_tool_calls_must_be_an_array() {
        for tool_calls in [json!({}), Value::Null] {
            let payload = json!({
                "choices": [{
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": tool_calls
                    },
                    "finish_reason": "tool_calls"
                }]
            });

            let error = chat_response_from_value(payload, &[weather_tool()])
                .expect_err("tool calls should fail");

            assert!(matches!(
                error,
                LlmError::InvalidProviderPayload("invalid tool calls")
            ));
        }
    }

    #[test]
    fn assistant_message_maps_tool_calls() {
        let mut message = ChatMessage::assistant("checking");
        message.tool_calls = vec![ToolCall {
            call_id: CallId::from("call-1"),
            tool_id: ToolId::from("internal-weather"),
            name: "get_weather".to_owned(),
            arguments: json!({"city": "Paris"}),
        }];
        let request = ChatRequest::new(ModelId::from("gpt-4.1-mini")).with_message(message);

        let payload = to_chat_payload(&request, false).expect("payload maps");
        let tool_call = &payload["messages"][0]["tool_calls"][0];
        let arguments: serde_json::Value = serde_json::from_str(
            tool_call["function"]["arguments"]
                .as_str()
                .expect("arguments"),
        )
        .expect("arguments are json");

        assert_eq!(tool_call["id"], "call-1");
        assert_eq!(tool_call["type"], "function");
        assert_eq!(tool_call["function"]["name"], "get_weather");
        assert_eq!(arguments, json!({"city": "Paris"}));
    }

    #[test]
    fn tool_message_maps_call_id_and_provider_name() {
        let mut message = ChatMessage::text(ChatRole::Tool, "sunny");
        message.tool_call_id = Some(CallId::from("call-1"));
        message.tool_id = Some(ToolId::from("internal-weather"));
        let request = ChatRequest {
            tools: vec![weather_tool()],
            ..ChatRequest::new(ModelId::from("gpt-4.1-mini")).with_message(message)
        };

        let payload = to_chat_payload(&request, false).expect("payload maps");
        let message = &payload["messages"][0];

        assert_eq!(message["role"], "tool");
        assert_eq!(message["content"], "sunny");
        assert_eq!(message["tool_call_id"], "call-1");
        assert_eq!(message["name"], "get_weather");
    }

    #[test]
    fn tool_choice_tool_uses_provider_tool_name() {
        let request = ChatRequest {
            tools: vec![weather_tool()],
            tool_choice: Some(ToolChoice::Tool(ToolId::from("internal-weather"))),
            ..ChatRequest::new(ModelId::from("gpt-4.1-mini"))
        };

        let payload = to_chat_payload(&request, false).expect("payload maps");

        assert_eq!(payload["tool_choice"]["function"]["name"], "get_weather");
    }

    #[test]
    fn unknown_tool_choice_returns_mapping_error() {
        let request = ChatRequest {
            tool_choice: Some(ToolChoice::Tool(ToolId::from("missing-tool"))),
            ..ChatRequest::new(ModelId::from("gpt-4.1-mini"))
        };

        let error = to_chat_payload(&request, false).expect_err("tool choice should fail");

        assert!(matches!(
            error,
            LlmError::InvalidProviderPayload("unknown tool choice")
        ));
    }

    fn weather_tool() -> ToolSpec {
        ToolSpec {
            tool_id: ToolId::from("internal-weather"),
            name: "get_weather".to_owned(),
            description: "get weather".to_owned(),
            input_schema: json!({"type": "object"}),
        }
    }
}
