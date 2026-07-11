use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue};
use serde_json::{Value, json};
use wyse_core::CallId;
use wyse_llm::{
    ApiKey, ChatMessage, ChatRequest, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
    OpenAICompatibleProvider, ToolCallDelta,
};

mod support;

use support::{TestResponse, TestServer};

#[tokio::test]
async fn chat_posts_chat_completion_and_maps_response() {
    let server = TestServer::spawn(TestResponse::ok(json!({
        "choices": [{
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "stop"
        }],
        "usage": {
            "prompt_tokens": 2,
            "completion_tokens": 3,
            "total_tokens": 5
        }
    })));
    let provider = test_provider(server.base_url("v1"));

    let response = provider
        .chat(
            ChatRequest::new(
                "openai_compatible:gpt-configured"
                    .parse()
                    .expect("model id parses"),
            )
            .with_message(ChatMessage::user("say hello")),
        )
        .await
        .expect("chat should succeed");
    let request = server.request();
    let body: Value = serde_json::from_slice(&request.body).expect("request body should be json");

    assert_eq!(response.message, ChatMessage::assistant("hello"));
    assert_eq!(response.finish_reason, FinishReason::Stop);
    assert_eq!(response.usage.expect("usage").total_tokens, 5);
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(body["model"], "gpt-configured");
    assert_eq!(body["stream"], false);
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "say hello");
}

#[tokio::test]
async fn builder_uses_injected_client() {
    let server = TestServer::spawn(TestResponse::ok(json!({
        "choices": [{
            "message": {"role": "assistant", "content": "hello"},
            "finish_reason": "stop"
        }]
    })));
    let mut headers = HeaderMap::new();
    headers.insert("x-wyse-test-client", HeaderValue::from_static("injected"));
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .expect("test client should build");
    let provider = OpenAICompatibleProvider::builder()
        .base_url(server.base_url("v1"))
        .api_key(ApiKey::new("sk-test"))
        .model(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        )
        .client(client)
        .build();

    provider
        .chat(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("chat should succeed");
    let request = server.request();

    assert_eq!(
        request
            .headers
            .get("x-wyse-test-client")
            .map(String::as_str),
        Some("injected")
    );
}

#[tokio::test]
async fn chat_rejects_request_model_that_differs_from_provider_model() {
    let provider = test_provider("http://127.0.0.1:9/v1");

    let error = provider
        .chat(ChatRequest::new(
            "openai_compatible:other-model"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect_err("model mismatch should fail before transport");

    assert!(matches!(
        error,
        LlmError::InvalidRequest("request model does not match provider model")
    ));
}

#[test]
fn openai_compatible_provider_model_id_includes_provider_name() {
    let provider = test_provider("http://127.0.0.1:9/v1");

    assert_eq!(
        provider.model_id().as_str(),
        "openai_compatible:gpt-configured"
    );
}

#[tokio::test]
async fn chat_maps_provider_status_error_payload() {
    let server = TestServer::spawn(TestResponse::status(
        429,
        vec![("x-request-id", "req-123")],
        json!({
            "error": {
                "message": "rate limited for sk-test",
                "type": "rate_limit_error"
            }
        }),
    ));
    let provider = test_provider(server.base_url("v1"));

    let error = provider
        .chat(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect_err("status should fail");

    let LlmError::ProviderStatus(status) = error else {
        panic!("expected provider status error");
    };
    assert_eq!(status.status(), 429);
    assert_eq!(status.code(), Some("rate_limit_error"));
    assert_eq!(status.message(), "rate limited for [redacted]");
    assert_eq!(status.request_id(), Some("req-123"));
}

#[tokio::test]
async fn chat_stream_posts_streaming_chat_completion_request() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(
            ChatRequest::new(
                "openai_compatible:gpt-configured"
                    .parse()
                    .expect("model id parses"),
            )
            .with_message(ChatMessage::user("say hello")),
        )
        .await
        .expect("stream should open");
    let event = stream
        .next()
        .await
        .expect("stream should emit finish")
        .expect("finish should map");
    let request = server.request();
    let body: Value = serde_json::from_slice(&request.body).expect("request body should be json");

    assert_eq!(
        event,
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None
        }
    );
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/v1/chat/completions");
    assert_eq!(
        request.headers.get("authorization").map(String::as_str),
        Some("Bearer sk-test")
    );
    assert_eq!(body["model"], "gpt-configured");
    assert_eq!(body["stream"], true);
    assert_eq!(body["messages"][0]["role"], "user");
    assert_eq!(body["messages"][0]["content"], "say hello");
}

#[tokio::test]
async fn chat_stream_maps_text_delta_and_finished_event() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n\
         data: {\"choices\":[{\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":3,\"total_tokens\":5}}\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream.next().await.expect("text event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "hel".to_owned()
        }
    );
    assert_eq!(
        stream.next().await.expect("text event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "lo".to_owned()
        }
    );
    assert_eq!(
        stream
            .next()
            .await
            .expect("finish event")
            .expect("finish maps"),
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: Some(wyse_core::TokenUsage {
                input_tokens: 2,
                output_tokens: 3,
                total_tokens: 5,
            })
        }
    );
}

#[tokio::test]
async fn chat_stream_maps_tool_call_delta() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-1\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\"}}]}}]}\n\n\
         data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\":\\\"Paris\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream.next().await.expect("tool event").expect("tool maps"),
        ChatStreamEvent::ToolCallDelta(ToolCallDelta {
            index: 0,
            call_id: Some(CallId::from("call-1")),
            name: Some("get_weather".to_owned()),
            arguments_delta: "{\"city".to_owned(),
        })
    );
    assert_eq!(
        stream.next().await.expect("tool event").expect("tool maps"),
        ChatStreamEvent::ToolCallDelta(ToolCallDelta {
            index: 0,
            call_id: None,
            name: None,
            arguments_delta: "\":\"Paris\"}".to_owned(),
        })
    );
    assert_eq!(
        stream
            .next()
            .await
            .expect("finish event")
            .expect("finish maps"),
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::ToolCalls,
            usage: None
        }
    );
}

#[tokio::test]
async fn chat_stream_recovers_sse_events_split_across_tcp_chunks() {
    let server = TestServer::spawn(TestResponse::stream_parts(vec![
        ": keepalive\r\n",
        "\r\n",
        "data: {\"choices\":[{\"delta\":{\"content\":\"he",
        "llo\"}}]}\r\n\r\n",
        "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\r",
        "\n\r\n",
    ]));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream.next().await.expect("text event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "hello".to_owned()
        }
    );
    assert_eq!(
        stream
            .next()
            .await
            .expect("finish event")
            .expect("finish maps"),
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None
        }
    );
}

#[tokio::test]
async fn chat_stream_done_emits_unknown_finish_when_no_finish_reason_arrived() {
    let server = TestServer::spawn(TestResponse::stream("data: [DONE]\n\n"));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream
            .next()
            .await
            .expect("finish event")
            .expect("done should map"),
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Unknown,
            usage: None
        }
    );
}

#[tokio::test]
async fn chat_stream_joins_multiple_data_lines_in_one_sse_event() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[\n\
         data: {\"delta\":{\"content\":\"hello\"}}]}\n\n\
         data: [DONE]\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream.next().await.expect("text event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "hello".to_owned()
        }
    );
}

#[tokio::test]
async fn chat_stream_maps_invalid_json_event_to_stream_error() {
    let server = TestServer::spawn(TestResponse::stream("data: {not-json}\n\n"));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");
    let error = stream
        .next()
        .await
        .expect("error event")
        .expect_err("invalid json should fail");

    assert!(matches!(error, LlmError::Stream(_)));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_stream_keeps_events_before_later_stream_error() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
         data: {not-json}\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream.next().await.expect("text event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "hello".to_owned()
        }
    );
    let error = stream
        .next()
        .await
        .expect("error event")
        .expect_err("invalid json should fail after text");
    assert!(matches!(error, LlmError::Stream(_)));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_stream_errors_when_eof_arrives_before_finish_event() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream.next().await.expect("text event").expect("text maps"),
        ChatStreamEvent::TextDelta {
            delta: "hello".to_owned()
        }
    );
    let error = stream
        .next()
        .await
        .expect("eof error")
        .expect_err("missing finish should fail");
    assert!(matches!(error, LlmError::Stream(_)));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_stream_errors_when_eof_leaves_partial_sse_event() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");
    let error = stream
        .next()
        .await
        .expect("eof error")
        .expect_err("partial sse event should fail");

    assert!(matches!(error, LlmError::Stream(_)));
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_stream_ignores_events_after_terminal_event() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"finish_reason\":\"stop\"}]}\n\n\
         data: [DONE]\n\n\
         data: {not-json}\n\n",
    ));
    let provider = test_provider(server.base_url("v1"));

    let mut stream = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect("stream should open");

    assert_eq!(
        stream
            .next()
            .await
            .expect("finish event")
            .expect("finish maps"),
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None
        }
    );
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn chat_stream_maps_provider_status_error_payload() {
    let server = TestServer::spawn(TestResponse::status(
        429,
        vec![("request-id", "req-stream")],
        json!({
            "error": {
                "message": "rate limited for sk-test",
                "code": "rate_limit"
            }
        }),
    ));
    let provider = test_provider(server.base_url("v1"));

    let result = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await;
    let Err(error) = result else {
        panic!("status should fail");
    };

    let LlmError::ProviderStatus(status) = error else {
        panic!("expected provider status error");
    };
    assert_eq!(status.status(), 429);
    assert_eq!(status.code(), Some("rate_limit"));
    assert_eq!(status.message(), "rate limited for [redacted]");
    assert_eq!(status.request_id(), Some("req-stream"));
}

#[tokio::test]
async fn chat_stream_rejects_request_model_that_differs_from_provider_model() {
    let provider = test_provider("http://127.0.0.1:9/v1");

    let result = provider
        .chat_stream(ChatRequest::new(
            "openai_compatible:other-model"
                .parse()
                .expect("model id parses"),
        ))
        .await;
    let Err(error) = result else {
        panic!("model mismatch should fail before transport");
    };

    assert!(matches!(
        error,
        LlmError::InvalidRequest("request model does not match provider model")
    ));
}

#[tokio::test]
async fn chat_maps_success_body_decode_errors_to_response_decode() {
    let server = TestServer::spawn(TestResponse::raw(200, "{not-json"));
    let provider = test_provider(server.base_url("v1"));

    let error = provider
        .chat(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect_err("invalid success body should fail");

    assert!(matches!(error, LlmError::ResponseDecode(_)));
}

#[tokio::test]
async fn chat_maps_status_body_decode_errors_to_provider_payload_decode() {
    let server = TestServer::spawn(TestResponse::raw(500, "{not-json"));
    let provider = test_provider(server.base_url("v1"));

    let error = provider
        .chat(ChatRequest::new(
            "openai_compatible:gpt-configured"
                .parse()
                .expect("model id parses"),
        ))
        .await
        .expect_err("invalid status body should fail");

    assert!(matches!(error, LlmError::ProviderPayloadDecode(_)));
}

fn test_provider(base_url: impl Into<String>) -> OpenAICompatibleProvider {
    OpenAICompatibleProvider::new(
        base_url,
        ApiKey::new("sk-test"),
        "openai_compatible:gpt-configured"
            .parse()
            .expect("model id parses"),
    )
}
