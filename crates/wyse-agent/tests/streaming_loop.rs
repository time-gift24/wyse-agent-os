use std::{
    collections::VecDeque,
    future::pending,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde_json::json;
use tokio::time::timeout;
use wyse_agent::{Agent, AgentConfig};
use wyse_checkpoint::{
    CheckpointError, CheckpointKind, CheckpointRecord, CheckpointStatus, CheckpointStore,
};
use wyse_core::{
    AgentEvent, CallId, ChatContent, ChatMessage, ChatRole, LlmCallRole, LlmEvent, ModelId,
    RuntimeEvent, ToolCallDelta, ToolName, ToolSpec, TurnId,
};
use wyse_infra::event_stream_bus::InMemoryEventStreamBus;
use wyse_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use wyse_tools::{BuiltinToolRegistry, EchoTool, ToolError, ToolInput, ToolOutput, ToolRegistry};

#[derive(Debug)]
enum ProviderResponse {
    Events(Vec<ChatStreamEvent>),
    StreamResults(Vec<Result<ChatStreamEvent, LlmError>>),
    PendingStart { entered: Arc<tokio::sync::Notify> },
}

#[derive(Debug)]
struct RecordingProvider {
    requests: Mutex<Vec<ChatRequest>>,
    responses: Mutex<VecDeque<ProviderResponse>>,
}

impl RecordingProvider {
    fn new(responses: Vec<ProviderResponse>) -> Self {
        Self {
            requests: Mutex::new(Vec::new()),
            responses: Mutex::new(VecDeque::from(responses)),
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

    fn model_id(&self) -> ModelId {
        ModelId::from("mock-model")
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::UnsupportedCapability("chat"))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        self.requests
            .lock()
            .expect("requests mutex should not be poisoned")
            .push(request);
        let response = self
            .responses
            .lock()
            .expect("responses mutex should not be poisoned")
            .pop_front()
            .ok_or(LlmError::MockExhausted)?;

        match response {
            ProviderResponse::Events(events) => {
                Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
            }
            ProviderResponse::StreamResults(results) => Ok(Box::pin(stream::iter(results))),
            ProviderResponse::PendingStart { entered } => {
                entered.notify_waiters();
                pending::<Result<ChatStream, LlmError>>().await
            }
        }
    }
}

#[derive(Debug)]
struct BlockingToolRegistry {
    entered: Arc<tokio::sync::Notify>,
    spec: ToolSpec,
}

impl BlockingToolRegistry {
    fn new(entered: Arc<tokio::sync::Notify>) -> Self {
        Self {
            entered,
            spec: ToolSpec::builder()
                .name("hang")
                .description("never returns")
                .input_schema(json!({"type": "object"}))
                .build(),
        }
    }
}

#[async_trait]
impl ToolRegistry for BlockingToolRegistry {
    fn register(&mut self, tool: Arc<dyn wyse_tools::Tool>) -> Result<(), ToolError> {
        Err(ToolError::DuplicateTool {
            name: tool.spec().name.clone(),
        })
    }

    fn get(&self, _name: &ToolName) -> Option<Arc<dyn wyse_tools::Tool>> {
        None
    }

    fn specs(&self) -> Vec<ToolSpec> {
        vec![self.spec.clone()]
    }

    async fn call(&self, _name: &ToolName, _input: ToolInput) -> Result<ToolOutput, ToolError> {
        self.entered.notify_waiters();
        pending::<Result<ToolOutput, ToolError>>().await
    }
}

#[derive(Debug, Default)]
struct RecordingCheckpointStore {
    records: Mutex<Vec<CheckpointRecord>>,
}

impl RecordingCheckpointStore {
    fn records(&self) -> Vec<CheckpointRecord> {
        self.records
            .lock()
            .expect("checkpoint records mutex should not be poisoned")
            .clone()
    }
}

#[async_trait]
impl CheckpointStore for RecordingCheckpointStore {
    async fn put_latest(&self, record: CheckpointRecord) -> Result<(), CheckpointError> {
        self.records
            .lock()
            .expect("checkpoint records mutex should not be poisoned")
            .push(record);
        Ok(())
    }

    async fn latest_turn(
        &self,
        _run_id: wyse_core::RunId,
        _turn_id: TurnId,
        _kind: CheckpointKind,
    ) -> Result<Option<CheckpointRecord>, CheckpointError> {
        Ok(self
            .records
            .lock()
            .expect("checkpoint records mutex should not be poisoned")
            .last()
            .cloned())
    }
}

#[tokio::test]
async fn stream_runs_tool_and_continues_with_tool_result() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Events(vec![
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
        ]),
        ProviderResponse::Events(vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
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
    assert_eq!(requests[0].model, ModelId::from("mock-model"));
    assert_eq!(requests[1].model, ModelId::from("mock-model"));
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool && message.tool_call_id == Some(CallId::from("call-1"))
    }));
}

#[tokio::test]
async fn stream_saves_finished_checkpoint_with_stable_history() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ],
    )]));
    let checkpoints = Arc::new(RecordingCheckpointStore::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(InMemoryEventStreamBus::default()))
        .checkpoint_store(checkpoints.clone())
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Finished { .. },
                    ..
                }
            ) {
                break;
            }
        }
    })
    .await
    .expect("timed out waiting for finished event");

    let records = checkpoints.records();
    let latest = records.last().expect("finished checkpoint exists");

    assert_eq!(latest.run_id, agent_stream.run_id);
    assert_eq!(latest.turn_id, agent_stream.turn_id);
    assert_eq!(latest.kind, CheckpointKind::Agent);
    assert_eq!(latest.status, CheckpointStatus::Finished);
    assert_eq!(latest.last_seq, 4);
    assert!(
        latest
            .state
            .windows(b"done".len())
            .any(|window| window == b"done")
    );
}

#[tokio::test]
async fn stream_saves_waiting_retry_without_partial_assistant_on_llm_error() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::StreamResults(vec![
            Ok(ChatStreamEvent::TextDelta {
                delta: "partial".to_owned(),
            }),
            Err(LlmError::UnsupportedCapability("stream failed")),
        ]),
    ]));
    let checkpoints = Arc::new(RecordingCheckpointStore::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(InMemoryEventStreamBus::default()))
        .checkpoint_store(checkpoints.clone())
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Failed { .. },
                    ..
                }
            ) {
                break;
            }
        }
    })
    .await
    .expect("timed out waiting for failed event");

    let records = checkpoints.records();
    let latest = records.last().expect("waiting retry checkpoint exists");

    assert_eq!(latest.status, CheckpointStatus::WaitingRetry);
    assert!(
        latest
            .state
            .windows(b"hello".len())
            .any(|window| window == b"hello")
    );
    assert!(
        !latest
            .state
            .windows(b"partial".len())
            .any(|window| window == b"partial")
    );
    let checkpoint_state: serde_json::Value = serde_json::from_slice(&latest.state)
        .expect("waiting retry checkpoint state should deserialize");
    assert_eq!(checkpoint_state["retry_count"].as_u64(), Some(1));
}

#[tokio::test]
async fn stream_publishes_failure_when_turn_limit_is_reached() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::ToolCalls,
            usage: None,
        }],
    )]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
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

#[tokio::test]
async fn stream_publishes_cancelled_when_provider_stream_creation_hangs() {
    let entered = Arc::new(tokio::sync::Notify::new());
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::PendingStart {
            entered: Arc::clone(&entered),
        },
    ]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");
    entered.notified().await;
    agent_stream.cancel.cancel();

    let mut saw_cancelled = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Cancelled => {
                    saw_cancelled = true;
                    break;
                }
                AgentEvent::Failed { error_text } => {
                    panic!("unexpected failure event: {error_text}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for cancelled event");

    assert!(saw_cancelled);
}

#[tokio::test]
async fn stream_publishes_cancelled_when_tool_call_hangs() {
    let provider = Arc::new(RecordingProvider::new(vec![ProviderResponse::Events(
        vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("hang".to_owned()),
                arguments_delta: "{}".to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ],
    )]));
    let entered = Arc::new(tokio::sync::Notify::new());
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BlockingToolRegistry::new(Arc::clone(&entered))))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");
    entered.notified().await;
    agent_stream.cancel.cancel();

    let mut saw_cancelled = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Cancelled => {
                    saw_cancelled = true;
                    break;
                }
                AgentEvent::Failed { error_text } => {
                    panic!("unexpected failure event: {error_text}");
                }
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for cancelled event");

    assert!(saw_cancelled);
}

#[tokio::test]
async fn stream_publishes_tool_failure_and_retries_with_tool_error_message() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Events(vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("missing".to_owned()),
                arguments_delta: "{}".to_owned(),
            }),
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::ToolCalls,
                usage: None,
            },
        ]),
        ProviderResponse::Events(vec![
            ChatStreamEvent::TextDelta {
                delta: "done".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
    ]));
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let mut agent_stream = agent
        .stream(ChatMessage::user("hello"))
        .await
        .expect("stream should start");
    let mut failure_text = None;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = agent_stream.events.next().await {
            let envelope = envelope.expect("event should be delivered");
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Llm {
                    event:
                        LlmEvent::ToolCallFailed {
                            call_id,
                            error_text,
                        },
                    ..
                } if call_id == CallId::from("call-1") => {
                    failure_text = Some(error_text);
                }
                AgentEvent::Finished { .. } => break,
                _ => {}
            }
        }
    })
    .await
    .expect("timed out waiting for streamed agent events");

    let failure_text = failure_text.expect("expected tool failure event");
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool
            && message.tool_call_id == Some(CallId::from("call-1"))
            && matches!(&message.content, ChatContent::Text(text) if text == &failure_text)
    }));
}
