use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    time::Duration,
};

use serde_json::{Value, json};
use wyse_llm::{
    ApiKey, ChatMessage, ChatRequest, DeepSeekModel, DeepSeekProvider, DeepSeekReasoningEffort,
    DeepSeekThinking, LlmProvider,
};

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
    body: String,
}

impl TestResponse {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
        }
    }
}

struct RecordedRequest {
    path: String,
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
    let _method = request_parts.next().expect("method");
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
        path,
        body: buffer[body_start..body_start + content_length].to_vec(),
    }
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

fn write_response(stream: &mut impl Write, response: TestResponse) {
    write!(
        stream,
        "HTTP/1.1 {} OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{}",
        response.status,
        response.body.len(),
        response.body
    )
    .expect("response should write");
}
