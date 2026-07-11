use std::{
    collections::VecDeque,
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    task::Poll,
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde_json::json;
use tokio::time::{sleep, timeout};
use wyse_agent::{Agent, AgentConfig, AgentError};
use wyse_core::{
    AgentEvent, AgentId, ApprovalDecision, ApprovalId, CallId, ChatContent, ChatMessage, ChatRole,
    DangerLevel, LlmCallRole, LlmEvent, ModelId, ReplayStart, RunId, RuntimeEvent, StreamEnvelope,
    TokenUsage, ToolCallDelta, ToolKind, ToolName, ToolSpec,
};
use wyse_infra::event_stream_bus::{
    EventStream, EventStreamBus, EventStreamBusError, InMemoryEventStreamBus,
};
use wyse_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use wyse_tools::{
    BuiltinToolRegistry, EchoTool, Tool, ToolError, ToolInput, ToolOutput, ToolPermissionMode,
    ToolRegistry,
};

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
    fn model_id(&self) -> ModelId {
        "recording:mock-model".parse().expect("model id parses")
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
    fn register(
        &mut self,
        tool: Arc<dyn wyse_tools::Tool>,
        _tool_kind: ToolKind,
        _danger_level: DangerLevel,
    ) -> Result<(), ToolError> {
        Err(ToolError::DuplicateTool {
            name: tool.spec().name.clone(),
        })
    }

    fn authorization(
        &self,
        _name: &ToolName,
    ) -> Result<Option<(ToolKind, DangerLevel)>, ToolError> {
        Ok(None)
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

struct CountingTool {
    spec: ToolSpec,
    calls: Arc<AtomicUsize>,
}

impl CountingTool {
    fn new(calls: Arc<AtomicUsize>) -> Self {
        Self {
            spec: ToolSpec::builder()
                .name("counting")
                .description("counts executions")
                .input_schema(json!({"type": "object"}))
                .build(),
            calls,
        }
    }
}

#[async_trait]
impl Tool for CountingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ToolOutput::new(input.arguments))
    }
}

#[derive(Clone, Default)]
struct FailingApprovalBus {
    inner: InMemoryEventStreamBus,
}

#[async_trait]
impl EventStreamBus for FailingApprovalBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        if matches!(
            &envelope.event,
            RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalRequested { .. },
                ..
            }
        ) {
            let source = serde_json::from_str::<serde_json::Value>("{")
                .expect_err("invalid json produces a serde error");
            return Err(EventStreamBusError::Serialize(source));
        }
        self.inner.publish(envelope).await
    }

    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        self.inner.subscribe_agent(agent_id, replay_start).await
    }
}

fn test_agent_id() -> AgentId {
    "0197fcb8-7500-7000-8000-000000000001"
        .parse()
        .expect("agent id parses")
}

async fn wait_for_request_count(provider: &RecordingProvider, count: usize) -> Vec<ChatRequest> {
    timeout(Duration::from_secs(1), async {
        loop {
            let requests = provider.requests();
            if requests.len() >= count {
                return requests;
            }
            sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("timed out waiting for provider requests")
}

async fn run_turn_and_subscribe(agent: &Agent, message: ChatMessage) -> (RunId, EventStream) {
    let run_id = agent.run_turn(message).await.expect("run should start");
    let events = agent
        .event_bus()
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe should succeed");
    (run_id, events)
}

async fn wait_for_approval_request(events: &mut EventStream) -> ApprovalId {
    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("approval event")
                .expect("event is valid")
                .envelope;
            if let RuntimeEvent::Agent {
                event:
                    AgentEvent::ToolApprovalRequested {
                        approval_id,
                        agent_name,
                        call_id,
                        tool_name,
                        arguments,
                        tool_kind,
                        danger_level,
                    },
                ..
            } = envelope.event
            {
                assert_eq!(agent_name, "test-agent");
                assert_eq!(call_id, CallId::from("call-1"));
                assert_eq!(tool_name, ToolName::from("counting"));
                assert_eq!(arguments, json!({"message": "hello"}));
                assert_eq!(tool_kind, ToolKind::Write);
                assert_eq!(danger_level, DangerLevel::High);
                return approval_id;
            }
        }
    })
    .await
    .expect("timed out waiting for approval request")
}

async fn wait_for_agent_finish(events: &mut EventStream) {
    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("finished event")
                .expect("event is valid")
                .envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Finished { .. },
                    ..
                }
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for agent finish");
}

fn approval_provider() -> Arc<RecordingProvider> {
    Arc::new(RecordingProvider::new(vec![
        ProviderResponse::Events(vec![
            ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                index: 0,
                call_id: Some(CallId::from("call-1")),
                name: Some("counting".to_owned()),
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
    ]))
}

fn approval_agent(
    calls: &Arc<AtomicUsize>,
    provider: Arc<RecordingProvider>,
    event_bus: Arc<dyn EventStreamBus>,
) -> Agent {
    let mut registry = BuiltinToolRegistry::new(ToolPermissionMode::RequireApproval);
    registry
        .register(
            Arc::new(CountingTool::new(Arc::clone(calls))),
            ToolKind::Write,
            DangerLevel::High,
        )
        .expect("tool registers");

    Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(registry))
        .event_bus(event_bus)
        .build()
        .expect("agent builds")
}

#[tokio::test]
async fn approval_allows_exactly_one_tool_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Approve)
        .await
        .expect("approval is accepted");
    wait_for_agent_finish(&mut events).await;

    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(agent.current_run(), Some(run_id));
}

#[tokio::test]
async fn approval_rejection_skips_tool_and_returns_structured_result() {
    let calls = Arc::new(AtomicUsize::new(0));
    let provider = approval_provider();
    let agent = approval_agent(
        &calls,
        Arc::clone(&provider),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Reject)
        .await
        .expect("rejection is accepted");
    wait_for_agent_finish(&mut events).await;

    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let requests = provider.requests();
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool
            && message.tool_call_id == Some(CallId::from("call-1"))
            && message.content
                == ChatContent::Json(json!({
                    "error": {
                        "type": "approval_rejected",
                        "message": "user rejected tool call"
                    }
                }))
    }));
}

#[tokio::test]
async fn approval_without_active_turn_returns_error() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    assert!(matches!(
        agent
            .resolve_tool_approval(ApprovalId::new(), ApprovalDecision::Approve)
            .await,
        Err(AgentError::NoActiveTurn)
    ));
}

#[tokio::test]
async fn approval_wrong_id_does_not_interrupt_active_request() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let different_id = ApprovalId::new();
    assert!(matches!(
        agent
            .resolve_tool_approval(different_id, ApprovalDecision::Approve)
            .await,
        Err(AgentError::ApprovalNotFound { approval_id }) if approval_id == different_id
    ));

    agent
        .resolve_tool_approval(approval_id, ApprovalDecision::Approve)
        .await
        .expect("real approval is accepted");
    wait_for_agent_finish(&mut events).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn approval_cancellation_wins_before_tool_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let resolution = agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve);
    tokio::pin!(resolution);
    assert!(matches!(
        futures_util::poll!(&mut resolution),
        Poll::Pending
    ));
    agent.stop();

    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("cancelled event")
                .expect("event is valid")
                .envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Cancelled { .. },
                    ..
                }
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for cancellation");
    assert!(matches!(resolution.await, Err(AgentError::NoActiveTurn)));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn approval_request_publish_failure_prevents_tool_execution() {
    let calls = Arc::new(AtomicUsize::new(0));
    let bus = Arc::new(FailingApprovalBus::default());
    let agent = approval_agent(&calls, approval_provider(), bus.clone());

    let _run_id = agent
        .run_turn(ChatMessage::user("change it"))
        .await
        .expect("run starts");
    let mut events = bus
        .subscribe_agent(test_agent_id(), ReplayStart::All)
        .await
        .expect("subscribe succeeds");

    timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("failed event")
                .expect("event is valid")
                .envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Failed { .. },
                    ..
                }
            ) {
                return;
            }
        }
    })
    .await
    .expect("timed out waiting for failed event");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn duplicate_approval_decisions_execute_tool_once() {
    let calls = Arc::new(AtomicUsize::new(0));
    let agent = approval_agent(
        &calls,
        approval_provider(),
        Arc::new(InMemoryEventStreamBus::default()),
    );

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("change it")).await;
    let approval_id = wait_for_approval_request(&mut events).await;
    let (first, second) = tokio::join!(
        agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve),
        agent.resolve_tool_approval(approval_id, ApprovalDecision::Approve),
    );
    wait_for_agent_finish(&mut events).await;

    assert!(first.is_ok() ^ second.is_ok());
    let duplicate = if first.is_err() { first } else { second };
    assert!(matches!(
        duplicate,
        Err(AgentError::ApprovalNotFound { .. } | AgentError::NoActiveTurn)
    ));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("echo should register");
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(registry))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    let mut saw_text_delta = false;
    let mut saw_tool_finished = false;
    let mut saw_tool_message = false;
    let mut saw_second_response_after_tool_message = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Message { message, .. }
                    if message.role == ChatRole::Tool
                        && message.tool_call_id == Some(CallId::from("call-1")) =>
                {
                    assert_eq!(
                        message.content,
                        ChatContent::Json(json!({"message": "hello"}))
                    );
                    saw_tool_message = true;
                }
                AgentEvent::Llm {
                    event:
                        LlmEvent::TextDelta {
                            role: LlmCallRole::Assistant,
                            delta,
                        },
                    ..
                } if delta == "done" => {
                    saw_text_delta = true;
                    saw_second_response_after_tool_message = saw_tool_message;
                }
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
    assert!(saw_tool_message);
    assert!(saw_second_response_after_tool_message);
    let requests = provider.requests();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].model,
        "recording:mock-model"
            .parse::<ModelId>()
            .expect("model id parses")
    );
    assert_eq!(
        requests[1].model,
        "recording:mock-model"
            .parse::<ModelId>()
            .expect("model id parses")
    );
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::Tool && message.tool_call_id == Some(CallId::from("call-1"))
    }));
}

#[tokio::test]
async fn stream_publishes_complete_turn_messages_in_order_without_business_sequences() {
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
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(InMemoryEventStreamBus::default()))
        .build()
        .expect("agent should build");
    let input = ChatMessage::user("hello");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, input.clone()).await;
    let turn_id = agent.current_turn().expect("turn id should be set");
    let mut state_events = Vec::new();

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            assert_eq!(envelope.business_seq, None);
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };
            if matches!(
                event,
                AgentEvent::Started { .. }
                    | AgentEvent::Message { .. }
                    | AgentEvent::Finished { .. }
            ) {
                let finished = matches!(event, AgentEvent::Finished { .. });
                state_events.push(event);
                if finished {
                    return;
                }
            }
        }
        panic!("expected finished event");
    })
    .await
    .expect("timed out waiting for complete turn events");

    assert_eq!(
        state_events,
        vec![
            AgentEvent::Started { turn_id },
            AgentEvent::Message {
                turn_id,
                message: input,
            },
            AgentEvent::Message {
                turn_id,
                message: ChatMessage::assistant("done"),
            },
            AgentEvent::Finished {
                finish_reason: "stop".to_owned(),
                usage: TokenUsage::default(),
            },
        ]
    );
}

#[tokio::test]
async fn failed_turn_does_not_commit_history_for_next_run() {
    let provider = Arc::new(RecordingProvider::new(vec![
        ProviderResponse::StreamResults(vec![
            Ok(ChatStreamEvent::TextDelta {
                delta: "partial".to_owned(),
            }),
            Err(LlmError::UnsupportedCapability("stream failed")),
        ]),
        ProviderResponse::Events(vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }]),
    ]));
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(Arc::new(InMemoryEventStreamBus::default()))
        .build()
        .expect("agent should build");

    let (_run_id, mut events) =
        run_turn_and_subscribe(&agent, ChatMessage::user("failed input")).await;
    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            if matches!(
                envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Failed { .. },
                    ..
                }
            ) {
                return;
            }
        }
        panic!("expected failed event");
    })
    .await
    .expect("timed out waiting for failed event");

    timeout(Duration::from_secs(1), async {
        loop {
            match agent.run_turn(ChatMessage::user("fresh input")).await {
                Ok(run_id) => return run_id,
                Err(AgentError::RunAlreadyActive) => sleep(Duration::from_millis(10)).await,
                Err(error) => panic!("unexpected run error: {error}"),
            }
        }
    })
    .await
    .expect("timed out waiting for second run");

    let requests = wait_for_request_count(&provider, 2).await;
    assert!(requests[0].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("failed input".to_owned())
    }));
    assert!(requests[1].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("fresh input".to_owned())
    }));
    assert!(!requests[1].messages.iter().any(|message| {
        message.role == ChatRole::User
            && message.content == ChatContent::Text("failed input".to_owned())
    }));
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
        .id(test_agent_id())
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

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            if let RuntimeEvent::Agent {
                event: AgentEvent::Failed { error_text, usage },
                ..
            } = envelope.event
            {
                assert!(error_text.contains("turn limit exceeded"));
                assert_eq!(usage, TokenUsage::default());
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
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    entered.notified().await;
    agent.stop();

    let mut saw_cancelled = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Cancelled { usage } => {
                    assert_eq!(usage, TokenUsage::default());
                    saw_cancelled = true;
                    break;
                }
                AgentEvent::Failed { error_text, .. } => {
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
                usage: Some(TokenUsage {
                    input_tokens: 3,
                    output_tokens: 5,
                    total_tokens: 8,
                }),
            },
        ],
    )]));
    let entered = Arc::new(tokio::sync::Notify::new());
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let agent = Agent::builder()
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider)
        .tool_registry(Arc::new(BlockingToolRegistry::new(Arc::clone(&entered))))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    entered.notified().await;
    agent.stop();

    let mut saw_cancelled = false;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
            let RuntimeEvent::Agent { event, .. } = envelope.event else {
                continue;
            };

            match event {
                AgentEvent::Cancelled { usage } => {
                    assert_eq!(
                        usage,
                        TokenUsage {
                            input_tokens: 3,
                            output_tokens: 5,
                            total_tokens: 8,
                        }
                    );
                    saw_cancelled = true;
                    break;
                }
                AgentEvent::Failed { error_text, .. } => {
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
        .id(test_agent_id())
        .name("test-agent")
        .system_prompt("be helpful")
        .llm_provider(provider.clone())
        .tool_registry(Arc::new(BuiltinToolRegistry::default()))
        .event_bus(bus)
        .build()
        .expect("agent should build");

    let (_run_id, mut events) = run_turn_and_subscribe(&agent, ChatMessage::user("hello")).await;
    let mut failure_text = None;

    timeout(Duration::from_secs(1), async {
        while let Some(envelope) = events.next().await {
            let envelope = envelope.expect("event should be delivered").envelope;
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
