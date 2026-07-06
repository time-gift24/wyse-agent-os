use futures_util::StreamExt;
use serde_json::{Value, json};
use wyse_llm::{
    ApiKey, ChatMessage, ChatRequest, ChatStreamEvent, DeepSeekModel, DeepSeekProvider,
    DeepSeekReasoningEffort, DeepSeekThinking, FinishReason, LlmError, LlmProvider,
    StructuredOutput,
};

mod support;

use support::{TestResponse, TestServer};

#[tokio::test]
async fn chat_posts_thinking_and_reasoning_content() {
    let server = TestServer::spawn(TestResponse::ok(json!({
        "choices": [{
            "message": {"role": "assistant", "content": "done"},
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}
    })));
    let provider = DeepSeekProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        DeepSeekModel::V4Pro,
        DeepSeekThinking::Enabled {
            effort: Some(DeepSeekReasoningEffort::Max),
        },
    );

    let model = DeepSeekModel::V4Pro.model_id();
    provider
        .chat(
            ChatRequest::new(model)
                .with_message(ChatMessage::user("solve"))
                .with_message(ChatMessage::assistant("tool answer").with_reasoning_content("why")),
        )
        .await
        .expect("chat should succeed");

    let request = server.request();
    let body: Value = serde_json::from_slice(&request.body).expect("request body should be json");

    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(body["model"], "deepseek-v4-pro");
    assert_eq!(body["thinking"], json!({"type": "enabled"}));
    assert_eq!(body["reasoning_effort"], "max");
    assert_eq!(body["messages"][1]["reasoning_content"], "why");
}

#[tokio::test]
async fn chat_maps_reasoning_content_to_assistant_message() {
    let server = TestServer::spawn(TestResponse::ok(json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": "final answer",
                "reasoning_content": "first think"
            },
            "finish_reason": "stop"
        }],
        "usage": {"prompt_tokens": 2, "completion_tokens": 3, "total_tokens": 5}
    })));
    let provider = test_provider(server.base_url("v1"));

    let response = provider
        .chat(ChatRequest::new(DeepSeekModel::V4Pro.model_id()))
        .await
        .expect("chat should succeed");

    assert_eq!(
        response.message,
        ChatMessage::assistant("final answer").with_reasoning_content("first think")
    );
    assert_eq!(response.usage.expect("usage").total_tokens, 5);
}

#[tokio::test]
async fn chat_stream_maps_reasoning_and_text_delta() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"answer\"}}]}\n\n\
         data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
         data: {\"choices\":[],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3,\"total_tokens\":5}}\n\n",
    ));
    let provider = DeepSeekProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        DeepSeekModel::V4Pro,
        DeepSeekThinking::Enabled {
            effort: Some(DeepSeekReasoningEffort::High),
        },
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(DeepSeekModel::V4Pro.model_id()))
        .await
        .expect("stream should open");
    let request = server.request();
    let body: Value = serde_json::from_slice(&request.body).expect("request body should be json");

    assert_eq!(
        stream.next().await.expect("event").expect("reasoning maps"),
        ChatStreamEvent::ReasoningDelta {
            delta: "think".to_owned()
        }
    );
    assert_eq!(
        stream.next().await.expect("event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "answer".to_owned()
        }
    );
    assert_eq!(
        stream.next().await.expect("event").expect("finish maps"),
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: Some(wyse_core::TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
                total_tokens: 5,
            })
        }
    );
    assert_eq!(body["stream"], true);
    assert_eq!(body["stream_options"], json!({"include_usage": true}));
}

#[tokio::test]
async fn chat_rejects_json_schema_structured_output() {
    let provider = test_provider("http://127.0.0.1:9/v1");

    let error = provider
        .chat(
            ChatRequest::new(DeepSeekModel::V4Pro.model_id()).with_structured_output(
                StructuredOutput::JsonSchema {
                    name: "answer".to_owned(),
                    schema: json!({"type": "object"}),
                    strict: true,
                },
            ),
        )
        .await
        .expect_err("json schema should be rejected before transport");

    assert!(matches!(
        error,
        LlmError::UnsupportedCapability("deepseek json schema structured output")
    ));
}

fn test_provider(base_url: impl Into<String>) -> DeepSeekProvider {
    DeepSeekProvider::new(
        base_url,
        ApiKey::new("sk-test"),
        DeepSeekModel::V4Pro,
        DeepSeekThinking::Disabled,
    )
}
