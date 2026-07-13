#![allow(dead_code)] // Shared integration-test support is compiled per test target.

use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::mpsc,
    time::Duration,
};

use serde_json::Value;

pub(crate) struct TestServer {
    base_url: String,
    request_rx: mpsc::Receiver<RecordedRequest>,
}

impl TestServer {
    pub(crate) fn spawn(response: TestResponse) -> Self {
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

    pub(crate) fn base_url(&self, prefix: &str) -> String {
        format!("{}/{}", self.base_url, prefix)
    }

    pub(crate) fn request(self) -> RecordedRequest {
        self.request_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("request should be recorded")
    }
}

pub(crate) struct TestResponse {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    body_parts: Vec<String>,
}

impl TestResponse {
    pub(crate) fn ok(body: Value) -> Self {
        Self::status(200, Vec::new(), body)
    }

    pub(crate) fn raw(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body_parts: vec![body.into()],
        }
    }

    pub(crate) fn status(
        status: u16,
        headers: Vec<(&'static str, &'static str)>,
        body: Value,
    ) -> Self {
        Self {
            status,
            headers,
            body_parts: vec![body.to_string()],
        }
    }

    pub(crate) fn stream(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "text/event-stream")],
            body_parts: vec![body.into()],
        }
    }

    pub(crate) fn stream_parts(parts: Vec<&'static str>) -> Self {
        Self {
            status: 200,
            headers: vec![("content-type", "text/event-stream")],
            body_parts: parts.into_iter().map(str::to_owned).collect(),
        }
    }
}

pub(crate) struct RecordedRequest {
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) headers: HashMap<String, String>,
    pub(crate) body: Vec<u8>,
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
