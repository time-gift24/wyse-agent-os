use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    time::Duration,
};

use futures_util::StreamExt;
use serde_json::{Value, json};
use wyse_core::{CallId, ModelId};
use wyse_llm::{
    ApiKey, ChatMessage, ChatRequest, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
    OpenAICompatibleProvider, ToolCallDelta,
};

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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    )
    .with_client(reqwest::Client::new());

    let response = provider
        .chat(
            ChatRequest::new(ModelId::from("gpt-configured"))
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
async fn chat_rejects_request_model_that_differs_from_provider_model() {
    let provider = OpenAICompatibleProvider::new(
        "http://127.0.0.1:9/v1",
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let error = provider
        .chat(ChatRequest::new(ModelId::from("other-model")))
        .await
        .expect_err("model mismatch should fail before transport");

    assert!(matches!(
        error,
        LlmError::InvalidRequest("request model does not match provider model")
    ));
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let error = provider
        .chat(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(
            ChatRequest::new(ModelId::from("gpt-configured"))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
        .await
        .expect("stream should open");
    let error = stream
        .next()
        .await
        .expect("error event")
        .expect_err("invalid json should fail");

    assert!(matches!(error, LlmError::Stream(_)));
}

#[tokio::test]
async fn chat_stream_keeps_events_before_later_stream_error() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n\
         data: {not-json}\n\n",
    ));
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
}

#[tokio::test]
async fn chat_stream_errors_when_eof_leaves_partial_sse_event() {
    let server = TestServer::spawn(TestResponse::stream(
        "data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}",
    ));
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let mut stream = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
        .await
        .expect("stream should open");
    let error = stream
        .next()
        .await
        .expect("eof error")
        .expect_err("partial sse event should fail");

    assert!(matches!(error, LlmError::Stream(_)));
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let result = provider
        .chat_stream(ChatRequest::new(ModelId::from("gpt-configured")))
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
    let provider = OpenAICompatibleProvider::new(
        "http://127.0.0.1:9/v1",
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let result = provider
        .chat_stream(ChatRequest::new(ModelId::from("other-model")))
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
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let error = provider
        .chat(ChatRequest::new(ModelId::from("gpt-configured")))
        .await
        .expect_err("invalid success body should fail");

    assert!(matches!(error, LlmError::ResponseDecode(_)));
}

#[tokio::test]
async fn chat_maps_status_body_decode_errors_to_provider_payload_decode() {
    let server = TestServer::spawn(TestResponse::raw(500, "{not-json"));
    let provider = OpenAICompatibleProvider::new(
        server.base_url("v1"),
        ApiKey::new("sk-test"),
        ModelId::from("gpt-configured"),
    );

    let error = provider
        .chat(ChatRequest::new(ModelId::from("gpt-configured")))
        .await
        .expect_err("invalid status body should fail");

    assert!(matches!(error, LlmError::ProviderPayloadDecode(_)));
}

struct TestServer {
    base_url: String,
    request_rx: mpsc::Receiver<RecordedRequest>,
}

impl TestServer {
    fn spawn(response: TestResponse) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("test server should bind");
        let base_url = format!("http://{}", listener.local_addr().expect("local addr"));
        let (request_tx, request_rx) = mpsc::channel();

        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("test server should accept");
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .expect("read timeout should be set");
            let request = read_request(&mut stream);
            request_tx
                .send(request)
                .expect("request should be recorded");
            write_response(&mut stream, response);
        });

        Self {
            base_url,
            request_rx,
        }
    }

    fn base_url(&self, prefix: &str) -> String {
        format!("{}/{}", self.base_url, prefix)
    }

    fn request(self) -> RecordedRequest {
        self.request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("request should be recorded")
    }
}

struct TestResponse {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body_parts: Vec<String>,
}

impl TestResponse {
    fn ok(body: Value) -> Self {
        Self::status(200, Vec::new(), body)
    }

    fn raw(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body_parts: vec![body.into()],
        }
    }

    fn status(status: u16, headers: Vec<(&'static str, &'static str)>, body: Value) -> Self {
        Self {
            status,
            headers,
            body_parts: vec![body.to_string()],
        }
    }

    fn stream(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "text/event-stream")],
            body_parts: vec![body.into()],
        }
    }

    fn stream_parts(parts: Vec<&'static str>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "text/event-stream")],
            body_parts: parts.into_iter().map(str::to_owned).collect(),
        }
    }
}

struct RecordedRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn read_request(stream: &mut impl Read) -> RecordedRequest {
    let mut buffer = Vec::new();
    let mut chunk = [0; 1024];
    let header_len = loop {
        let read = stream.read(&mut chunk).expect("request should be readable");
        assert_ne!(read, 0, "request ended before headers");
        buffer.extend_from_slice(&chunk[..read]);

        if let Some(position) = find_header_end(&buffer) {
            break position;
        }
    };

    let header_bytes = &buffer[..header_len];
    let header_text = std::str::from_utf8(header_bytes).expect("headers should be utf8");
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().expect("request line");
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().expect("method").to_owned();
    let path = request_parts.next().expect("path").to_owned();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_owned()))
        .collect::<HashMap<_, _>>();

    let content_length = headers
        .get("content-length")
        .and_then(|value| value.parse::<usize>().ok())
        .expect("content-length header");
    let body_start = header_len + 4;
    while buffer.len() < body_start + content_length {
        let read = stream.read(&mut chunk).expect("body should be readable");
        assert_ne!(read, 0, "request ended before body");
        buffer.extend_from_slice(&chunk[..read]);
    }

    RecordedRequest {
        method,
        path,
        headers,
        body: buffer[body_start..body_start + content_length].to_vec(),
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(stream: &mut impl Write, response: TestResponse) {
    let reason = match response.status {
        200 => "OK",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let content_length = response.body_parts.iter().map(String::len).sum::<usize>();
    write!(
        stream,
        "HTTP/1.1 {} {}\r\ncontent-type: application/json\r\ncontent-length: {}\r\n",
        response.status, reason, content_length
    )
    .expect("response headers should write");
    for (name, value) in response.headers {
        write!(stream, "{name}: {value}\r\n").expect("response header should write");
    }
    write!(stream, "\r\n").expect("response header terminator should write");
    for part in response.body_parts {
        write!(stream, "{part}").expect("response body should write");
        stream.flush().expect("response body should flush");
        std::thread::sleep(Duration::from_millis(5));
    }
}
