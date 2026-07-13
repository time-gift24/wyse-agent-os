//! Shared server-sent event parsing helpers.
use std::io;

use crate::LlmError;

#[derive(Debug, Default)]
pub(crate) struct SseParser {
    buffer: Vec<u8>,
}

impl SseParser {
    pub(crate) fn push(&mut self, chunk: &[u8]) -> Vec<Result<SseEvent, LlmError>> {
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();

        while let Some((event_end, delimiter_len)) = event_delimiter(&self.buffer) {
            let event = self.buffer[..event_end].to_vec();
            self.buffer.drain(..event_end + delimiter_len);

            match parse_sse_event(event) {
                Ok(Some(event)) => events.push(Ok(event)),
                Ok(None) => {}
                Err(error) => {
                    events.push(Err(error));
                    break;
                }
            }
        }

        events
    }

    pub(crate) fn has_pending(&self) -> bool {
        !self.buffer.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SseEvent {
    Data(String),
    Done,
}

fn event_delimiter(buffer: &[u8]) -> Option<(usize, usize)> {
    let lf = buffer
        .windows(2)
        .position(|window| window == b"\n\n")
        .map(|position| (position, 2));
    let crlf = buffer
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| (position, 4));

    match (lf, crlf) {
        (Some(lf), Some(crlf)) => Some(lf.min(crlf)),
        (Some(lf), None) => Some(lf),
        (None, Some(crlf)) => Some(crlf),
        (None, None) => None,
    }
}

fn parse_sse_event(event: Vec<u8>) -> Result<Option<SseEvent>, LlmError> {
    let text = String::from_utf8(event).map_err(LlmError::stream)?;
    let mut data_lines = Vec::new();

    for line in text.lines() {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.is_empty() || line.starts_with(':') {
            continue;
        }

        if let Some(data) = line.strip_prefix("data:") {
            data_lines.push(data.strip_prefix(' ').unwrap_or(data).to_owned());
        }
    }

    if data_lines.is_empty() {
        return Ok(None);
    }

    let data = data_lines.join("\n");
    if data == "[DONE]" {
        return Ok(Some(SseEvent::Done));
    }

    Ok(Some(SseEvent::Data(data)))
}

pub(crate) fn stream_eof_error(message: &'static str) -> LlmError {
    LlmError::stream(io::Error::new(io::ErrorKind::UnexpectedEof, message))
}
