//! OpenAI-compatible protocol implementation.
#![cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "provider transport will call this mapper in the next task"
    )
)]

use serde_json::{Value, json};
use wyse_core::{CallId, TokenUsage, ToolId};

use crate::{
    ChatContent, ChatMessage, ChatRequest, ChatResponse, ChatRole, FinishReason, LlmError,
    StructuredOutput, ToolCall, ToolChoice,
};

pub(crate) fn to_chat_payload(request: &ChatRequest, stream: bool) -> Result<Value, LlmError> {
    let mut payload = json!({
        "model": request.model.as_str(),
        "messages": request.messages.iter().map(message_to_value).collect::<Vec<_>>(),
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
        payload["tool_choice"] = tool_choice_to_value(choice);
    }

    if let Some(output) = &request.structured_output {
        payload["response_format"] = structured_output_to_value(output);
    }

    Ok(payload)
}

pub(crate) fn chat_response_from_value(value: Value) -> Result<ChatResponse, LlmError> {
    let choice = value["choices"]
        .as_array()
        .and_then(|choices| choices.first())
        .ok_or(LlmError::InvalidProviderPayload("missing choice"))?;
    let message = &choice["message"];
    let content = message["content"].as_str().unwrap_or_default();
    let finish_reason = finish_reason(choice["finish_reason"].as_str());
    let usage = usage_from_value(value.get("usage"));
    let mut chat_message = ChatMessage::assistant(content);
    chat_message.tool_calls = tool_calls_from_message(message);

    Ok(ChatResponse {
        message: chat_message,
        finish_reason,
        usage,
    })
}

fn message_to_value(message: &ChatMessage) -> Value {
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

    json!({"role": role, "content": content})
}

fn tool_choice_to_value(choice: &ToolChoice) -> Value {
    match choice {
        ToolChoice::Auto => Value::String("auto".to_owned()),
        ToolChoice::None => Value::String("none".to_owned()),
        ToolChoice::Required => Value::String("required".to_owned()),
        ToolChoice::Tool(tool_id) => json!({
            "type": "function",
            "function": {"name": tool_id.as_str()}
        }),
    }
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

fn tool_calls_from_message(message: &Value) -> Vec<ToolCall> {
    message["tool_calls"]
        .as_array()
        .map(|calls| {
            calls
                .iter()
                .map(|call| {
                    let name = call["function"]["name"].as_str().unwrap_or_default();
                    ToolCall {
                        call_id: CallId::from(call["id"].as_str().unwrap_or_default()),
                        tool_id: ToolId::from(name),
                        name: name.to_owned(),
                        arguments: serde_json::from_str(
                            call["function"]["arguments"].as_str().unwrap_or("{}"),
                        )
                        .unwrap_or_else(|_| json!({})),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use wyse_core::{ModelId, ToolId};

    use super::*;
    use crate::{ChatMessage, ChatRequest, FinishReason, StructuredOutput, ToolChoice, ToolSpec};

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

        let response = chat_response_from_value(payload).expect("response maps");

        assert_eq!(response.message, ChatMessage::assistant("hello"));
        assert_eq!(response.finish_reason, FinishReason::Stop);
        assert_eq!(response.usage.expect("usage").total_tokens, 5);
    }
}
