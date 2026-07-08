use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde_json::json;
use tokio::time::timeout;
use wyse_agent::{Agent, AgentConfig};
use wyse_core::{
    AgentEvent, CallId, ChatMessage, ChatRole, LlmCallRole, LlmEvent, ModelId, RuntimeEvent,
    ToolCallDelta,
};
use wyse_infra::event_stream_bus::InMemoryEventStreamBus;
use wyse_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use wyse_tools::{BuiltinToolRegistry, EchoTool, ToolRegistry};

#[derive(Debug)]
struct RecordingProvider {
    requests: Mutex<Vec<ChatRequest>>,
    streams: Mutex<VecDeque<Vec<ChatStreamEvent>>>,
}

impl RecordingProvider {
    fn new(streams: Vec<Vec<ChatStreamEvent>>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            streams: Mutex::new(VecDeque::from(streams)),
        }
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .expect("requests mutex should not be poisoned")
            .clone()
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    fn provider_name(&self) -> &str {
        "recording"
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::UnsupportedCapability("chat"))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        self.requests
            .lock()
            .expect("requests mutex should not be poisoned")
            .push(request);
        let events = self
            .streams
            .lock()
            .expect("streams mutex should not be poisoned")
            .pop_front()
            .ok_or(LlmError::MockExhausted)?;

        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

#[tokio::test]
async fn stream_runs_tool_and_continues_with_tool_result() {
    let provider = Arc::new(RecordingProvider::new(vec![
        vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("echo".to_owned()),
                arguments_delta: r#"{"message":"hello"}"#.to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ],
        vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ],
    ]));
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()))
        .expect("echo should register");
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .model(ModelId::from("mock-model"))
        .tool_registry(Arc::new(registry))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");
    let mut saw_text_delta = false;
    let mut saw_tool_finished = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Llm {
                    event:
                        LlmEvent::TextDelta {
                            role: LlmCallRole::Assistant,
                            delta,
                        },
                    ..
                } if delta == "done" => saw_text_delta = true,
                AgentEvent::Llm {
                    event: LlmEvent::ToolCallFinished { call_id, result },
                    ..
                } if call_id == CallId::from("call-1") && result == json!({"message": "hello"}) => {
                    saw_tool_finished = true;
                }
                AgentEvent::Finished { .. } => break,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for streamed agent events");

    assert!(saw_text_delta);
    assert!(saw_tool_finished);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool && message.tool_call_id == Some(CallId::from("call-1"))
    }));
}

#[tokio::test]
async fn stream_publishes_failure_when_turn_limit_is_reached() {
    let provider = Arc::new(RecordingProvider::new(vec![vec![
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::ToolCalls,
            usage: None,
        },
    ]]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .model(ModelId::from("mock-model"))
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .config(AgentConfig {
            max_turns: 0,
            max_tool_calls_per_turn: 16,
        })
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            if let RuntimeEvent::Agent {
                event: AgentEvent::Failed { error_text },
                ..
            } = envelope.event
            {
                assert!(error_text.contains("turn limit exceeded"));
                return;
            }
        }

        panic!("expected failed event");
    })
    .await
    .expect("timed out waiting for failed event");
}
