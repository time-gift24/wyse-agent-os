use std::{
    error::Error as _,
    future::pending,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use futures_util::{StreamExt, stream};
use serde_json::json;
use stratum_agent::{
    AgentLoop, AgentLoopBuildError, AgentLoopError, AllowAllToolApproval, LoopContext, LoopLimit,
    LoopLimits, ProtocolError, RequiredAgentLoopField, ToolExecutor,
};
use stratum_core::{
    AgentTelemetryEvent, CallId, ChatMessage, ChatRole, DangerLevel, DurableAgentEvent, ModelId,
    TokenUsage, ToolCallDelta, ToolKind,
};
use stratum_infra::{DurableEventSink, DurableEventSinkError, TelemetryEventSink};
use stratum_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use stratum_tools::{BuiltinToolRegistry, EchoTool, ToolRegistry};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
enum Operation {
    Durable(DurableAgentEvent),
    Telemetry(AgentTelemetryEvent),
    ChatStream(ChatRequest),
}

#[test]
fn builder_reports_a_typed_missing_field() {
    let Err(error) = AgentLoop::builder().build() else {
        panic!("an empty builder should fail");
    };

    assert_eq!(
        error,
        AgentLoopBuildError::MissingField {
            field: RequiredAgentLoopField::LlmProvider,
        }
    );
}

struct RecordingDurableSink {
    operations: Arc<Mutex<Vec<Operation>>>,
    fail_at: Option<usize>,
    attempts: AtomicUsize,
}

#[async_trait]
impl DurableEventSink for RecordingDurableSink {
    async fn append(&self, event: DurableAgentEvent) -> Result<(), DurableEventSinkError> {
        let event_type = event.event_type();
        let attempt = self.attempts.fetch_add(1, Ordering::Relaxed);
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::Durable(event));
        if self.fail_at == Some(attempt) {
            Err(DurableEventSinkError::UnsupportedEvent { event_type })
        } else {
            Ok(())
        }
    }
}

struct RecordingTelemetrySink {
    operations: Arc<Mutex<Vec<Operation>>>,
}

#[async_trait]
impl TelemetryEventSink for RecordingTelemetrySink {
    async fn emit(&self, event: AgentTelemetryEvent) {
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::Telemetry(event));
    }
}

struct ScriptedProvider {
    operations: Arc<Mutex<Vec<Operation>>>,
    behavior: Mutex<Option<ProviderBehavior>>,
    model: ModelId,
}

enum ProviderBehavior {
    Items(Vec<Result<ChatStreamEvent, LlmError>>),
    PartialThenPending(Vec<Result<ChatStreamEvent, LlmError>>),
    SetupError,
    Pending,
}

#[async_trait]
impl LlmProvider for ScriptedProvider {
    fn model_id(&self) -> ModelId {
        self.model.clone()
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::UnsupportedCapability("chat"))
    }

    async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream, LlmError> {
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::ChatStream(request));
        let behavior = self
            .behavior
            .lock()
            .expect("behavior lock should not be poisoned")
            .take()
            .ok_or(LlmError::MockExhausted)?;
        match behavior {
            ProviderBehavior::Items(items) => Ok(Box::pin(stream::iter(items))),
            ProviderBehavior::PartialThenPending(items) => {
                Ok(Box::pin(stream::iter(items).chain(stream::pending())))
            }
            ProviderBehavior::SetupError => Err(LlmError::MockExhausted),
            ProviderBehavior::Pending => pending().await,
        }
    }
}

fn test_agent_loop(events: Vec<ChatStreamEvent>) -> (AgentLoop, Arc<Mutex<Vec<Operation>>>) {
    test_agent_loop_with(provider_events(events), LoopLimits::default(), None)
}

fn provider_events(events: Vec<ChatStreamEvent>) -> ProviderBehavior {
    ProviderBehavior::Items(events.into_iter().map(Ok).collect())
}

fn test_agent_loop_with(
    behavior: ProviderBehavior,
    limits: LoopLimits,
    fail_at: Option<usize>,
) -> (AgentLoop, Arc<Mutex<Vec<Operation>>>) {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let durable: Arc<dyn DurableEventSink> = Arc::new(RecordingDurableSink {
        operations: Arc::clone(&operations),
        fail_at,
        attempts: AtomicUsize::new(0),
    });
    let telemetry: Arc<dyn TelemetryEventSink> = Arc::new(RecordingTelemetrySink {
        operations: Arc::clone(&operations),
    });
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        operations: Arc::clone(&operations),
        behavior: Mutex::new(Some(behavior)),
        model: "scripted:test-model"
            .parse()
            .expect("static model id should parse"),
    });
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("echo tool should register");
    let tool_executor = ToolExecutor::new(
        Arc::new(registry),
        Arc::new(AllowAllToolApproval),
        Arc::clone(&durable),
    );
    let agent_loop = AgentLoop::builder()
        .llm_provider(provider)
        .tool_executor(tool_executor)
        .durable_events(durable)
        .telemetry(telemetry)
        .limits(limits)
        .build()
        .expect("all agent loop fields should be present");
    (agent_loop, operations)
}

#[tokio::test]
async fn no_tool_stream_commits_complete_messages_and_preserves_event_order() {
    let usage = TokenUsage {
        input_tokens: 11,
        output_tokens: 5,
        total_tokens: 16,
    };
    let (agent_loop, operations) = test_agent_loop(vec![
        ChatStreamEvent::ReasoningDelta {
            delta: "considering".to_owned(),
        },
        ChatStreamEvent::TextDelta {
            delta: "hel".to_owned(),
        },
        ChatStreamEvent::TextDelta {
            delta: "lo".to_owned(),
        },
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: Some(usage),
        },
    ]);
    let history = vec![
        ChatMessage::user("earlier question"),
        ChatMessage::assistant("earlier answer"),
    ];
    let prompts = vec![
        ChatMessage::user("first new question"),
        ChatMessage::user("second new question"),
    ];

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise").with_messages(history.clone()),
            prompts.clone(),
            CancellationToken::new(),
        )
        .await
        .expect("scripted loop should finish");

    let assistant = ChatMessage::assistant("hello").with_reasoning_content("considering");
    assert_eq!(
        outcome.new_messages,
        vec![prompts[0].clone(), prompts[1].clone(), assistant.clone()]
    );
    assert_eq!(outcome.finish_reason, FinishReason::Stop);
    assert_eq!(outcome.usage, usage);

    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(matches!(
        operations.first(),
        Some(Operation::Durable(DurableAgentEvent::LoopStarted))
    ));
    assert_eq!(
        operations.get(1),
        Some(&Operation::Durable(DurableAgentEvent::MessageAppended {
            message: prompts[0].clone(),
        }))
    );
    assert!(matches!(
        operations.get(2),
        Some(Operation::Durable(DurableAgentEvent::MessageAppended { message }))
            if message == &prompts[1]
    ));
    assert!(matches!(
        operations.get(3),
        Some(Operation::Telemetry(AgentTelemetryEvent::LlmStarted { .. }))
    ));
    let Operation::ChatStream(request) = operations
        .get(4)
        .expect("chat stream should follow durable prompts")
    else {
        panic!("expected chat stream operation");
    };
    assert_eq!(request.model.as_str(), "scripted:test-model");
    assert_eq!(
        request.messages,
        vec![
            ChatMessage::system("be precise"),
            history[0].clone(),
            history[1].clone(),
            ChatMessage::user("first new question"),
            ChatMessage::user("second new question"),
        ]
    );
    assert_eq!(
        request.tools,
        vec![
            stratum_core::ToolSpec::builder()
                .name("echo")
                .description("returns input arguments")
                .input_schema(json!({"type": "object"}))
                .build()
        ]
    );
    let telemetry = operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::Telemetry(event) => Some(event),
            Operation::Durable(_) | Operation::ChatStream(_) => None,
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        telemetry.first(),
        Some(AgentTelemetryEvent::LlmStarted { .. })
    ));
    assert!(matches!(
        telemetry.get(1),
        Some(AgentTelemetryEvent::ReasoningDelta { delta, .. }) if delta == "considering"
    ));
    assert!(matches!(
        telemetry.get(2),
        Some(AgentTelemetryEvent::TextDelta { delta, .. }) if delta == "hel"
    ));
    assert!(matches!(
        telemetry.get(3),
        Some(AgentTelemetryEvent::TextDelta { delta, .. }) if delta == "lo"
    ));
    assert!(matches!(
        telemetry.get(4),
        Some(AgentTelemetryEvent::LlmFinished {
            finish_reason,
            usage: Some(event_usage),
            ..
        }) if finish_reason == "stop" && *event_usage == usage
    ));
    let durable = operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::Durable(event) => Some(event),
            Operation::Telemetry(_) | Operation::ChatStream(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        durable,
        vec![
            &DurableAgentEvent::LoopStarted,
            &DurableAgentEvent::MessageAppended {
                message: ChatMessage::user("first new question"),
            },
            &DurableAgentEvent::MessageAppended {
                message: ChatMessage::user("second new question"),
            },
            &DurableAgentEvent::MessageAppended { message: assistant },
            &DurableAgentEvent::IterationCompleted {
                iteration: 0,
                usage,
            },
            &DurableAgentEvent::LoopFinished {
                finish_reason: "stop".to_owned(),
                usage,
            },
        ]
    );
}

#[tokio::test]
async fn empty_prompts_are_rejected_before_external_actions() {
    let (agent_loop, operations) = test_agent_loop(vec![ChatStreamEvent::Finished {
        finish_reason: FinishReason::Stop,
        usage: None,
    }]);

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            Vec::new(),
            CancellationToken::new(),
        )
        .await
        .expect_err("empty prompts should be rejected");

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::EmptyPrompts,
        }
    ));
    assert!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn non_user_prompts_are_rejected_before_external_actions() {
    let (agent_loop, operations) = test_agent_loop(vec![ChatStreamEvent::Finished {
        finish_reason: FinishReason::Stop,
        usage: None,
    }]);

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::assistant("not a user prompt")],
            CancellationToken::new(),
        )
        .await
        .expect_err("non-user prompts should be rejected");

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::InvalidPromptRole {
                role: ChatRole::Assistant,
            },
        }
    ));
    assert!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .is_empty()
    );
}

fn tool_delta(
    index: usize,
    call_id: Option<&str>,
    name: Option<&str>,
    arguments_delta: &str,
) -> ChatStreamEvent {
    ChatStreamEvent::ToolCallDelta(ToolCallDelta {
        index,
        call_id: call_id.map(CallId::from),
        name: name.map(str::to_owned),
        arguments_delta: arguments_delta.to_owned(),
    })
}

fn finished_tool_call_stream(mut deltas: Vec<ChatStreamEvent>) -> Vec<ChatStreamEvent> {
    deltas.push(ChatStreamEvent::Finished {
        finish_reason: FinishReason::ToolCalls,
        usage: None,
    });
    deltas
}

async fn run_error(
    events: Vec<ChatStreamEvent>,
    limits: LoopLimits,
) -> (AgentLoopError, Arc<Mutex<Vec<Operation>>>) {
    let (agent_loop, operations) = test_agent_loop_with(provider_events(events), limits, None);
    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("use a tool")],
            CancellationToken::new(),
        )
        .await
        .expect_err("scripted stream should fail");
    (error, operations)
}

#[tokio::test]
async fn usize_max_tool_index_is_rejected_before_allocation() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![tool_delta(
            usize::MAX,
            Some("call-max"),
            Some("echo"),
            "{}",
        )]),
        LoopLimits::new(1, 16),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::LimitExceeded {
            limit: LoopLimit::ToolCallsPerIteration { maximum: 16 },
        }
    ));
}

#[tokio::test]
async fn very_large_sparse_tool_index_uses_received_call_allocation_only() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![tool_delta(
            1_000_000,
            Some("call-sparse"),
            Some("echo"),
            "{}",
        )]),
        LoopLimits::new(1, usize::MAX),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::SparseToolCallIndex {
                expected: 0,
                actual: 1_000_000,
            },
        }
    ));
}

#[tokio::test]
async fn tool_index_at_exact_allowed_boundary_is_accepted() {
    let events = finished_tool_call_stream(vec![
        tool_delta(0, Some("call-0"), Some("echo"), "{"),
        tool_delta(0, Some("call-0"), Some("echo"), "}"),
        tool_delta(1, Some("call-1"), Some("echo"), "{}"),
    ]);
    let (agent_loop, operations) =
        test_agent_loop_with(provider_events(events), LoopLimits::new(1, 2), None);

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("use tools")],
            CancellationToken::new(),
        )
        .await
        .expect("index limit minus one should be accepted");

    assert_eq!(outcome.new_messages.len(), 2);
    assert_eq!(outcome.new_messages[1].tool_calls.len(), 2);
    assert!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .any(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::MessageAppended { message })
                    if message.role == ChatRole::Assistant && message.tool_calls.len() == 2
            ))
    );
}

#[tokio::test]
async fn max_zero_rejects_the_first_tool_delta() {
    let (error, operations) = run_error(
        finished_tool_call_stream(vec![tool_delta(0, Some("call-0"), Some("echo"), "{}")]),
        LoopLimits::new(1, 0),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::LimitExceeded {
            limit: LoopLimit::ToolCallsPerIteration { maximum: 0 },
        }
    ));
    assert_eq!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopFailed { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn sparse_tool_indices_are_rejected() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![tool_delta(1, Some("call-1"), Some("echo"), "{}")]),
        LoopLimits::new(1, 2),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::SparseToolCallIndex {
                expected: 0,
                actual: 1,
            },
        }
    ));
}

#[tokio::test]
async fn conflicting_repeated_tool_call_ids_are_rejected() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![
            tool_delta(0, Some("call-a"), Some("echo"), "{"),
            tool_delta(0, Some("call-b"), Some("echo"), "}"),
        ]),
        LoopLimits::new(1, 2),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::ConflictingToolCallId { index: 0, .. },
        }
    ));
}

#[tokio::test]
async fn conflicting_repeated_tool_call_names_are_rejected() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![
            tool_delta(0, Some("call-a"), Some("echo"), "{"),
            tool_delta(0, Some("call-a"), Some("other"), "}"),
        ]),
        LoopLimits::new(1, 2),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::ConflictingToolCallName { index: 0, .. },
        }
    ));
}

#[tokio::test]
async fn duplicate_finalized_tool_call_ids_are_rejected() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![
            tool_delta(0, Some("call-shared"), Some("echo"), "{}"),
            tool_delta(1, Some("call-shared"), Some("echo"), "{}"),
        ]),
        LoopLimits::new(1, 2),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::DuplicateToolCallId { call_id },
        } if call_id == CallId::from("call-shared")
    ));
}

#[tokio::test]
async fn a_tool_call_without_provider_id_is_incomplete() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![tool_delta(0, None, Some("echo"), "{}")]),
        LoopLimits::new(1, 2),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::IncompleteToolCall {
                index: 0,
                call_id: None,
            },
        }
    ));
}

#[tokio::test]
async fn a_tool_call_without_name_is_incomplete() {
    let (error, _) = run_error(
        finished_tool_call_stream(vec![tool_delta(0, Some("call-no-name"), None, "{}")]),
        LoopLimits::new(1, 2),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::IncompleteToolCall {
                index: 0,
                call_id: Some(call_id),
            },
        } if call_id == CallId::from("call-no-name")
    ));
}

#[tokio::test]
async fn malformed_tool_json_preserves_source_and_terminal_telemetry() {
    let (error, operations) = run_error(
        finished_tool_call_stream(vec![tool_delta(0, Some("call-json"), Some("echo"), "{bad")]),
        LoopLimits::new(1, 2),
    )
    .await;

    let protocol = error
        .source()
        .and_then(|source| source.downcast_ref::<ProtocolError>())
        .expect("protocol error should remain the loop error source");
    assert!(matches!(
        protocol,
        ProtocolError::MalformedToolCallArguments { call_id, .. }
            if call_id == &CallId::from("call-json")
    ));
    assert!(
        protocol
            .source()
            .and_then(|source| source.downcast_ref::<serde_json::Error>())
            .is_some()
    );
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Telemetry(AgentTelemetryEvent::LlmFinished { .. })
            ))
            .count(),
        1
    );
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::MessageAppended { message })
            if message.role == ChatRole::Assistant
    )));
}

#[tokio::test]
async fn zero_iteration_limit_is_rejected_before_external_actions() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }]),
        LoopLimits::new(0, 1),
        None,
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("zero iterations should reject the run");

    assert!(matches!(
        error,
        AgentLoopError::LimitExceeded {
            limit: LoopLimit::Iterations { maximum: 0 },
        }
    ));
    assert!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .is_empty()
    );
}

#[tokio::test]
async fn pending_chat_setup_cancellation_commits_one_cancelled_terminal() {
    let (agent_loop, operations) =
        test_agent_loop_with(ProviderBehavior::Pending, LoopLimits::new(1, 1), None);
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("hello")],
                task_cancellation,
            )
            .await
    });
    timeout(Duration::from_secs(1), async {
        loop {
            if operations
                .lock()
                .expect("operation lock should not be poisoned")
                .iter()
                .any(|operation| matches!(operation, Operation::ChatStream(_)))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider should enter chat setup");

    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), task)
        .await
        .expect("cancelled loop should stop")
        .expect("loop task should not panic")
        .expect_err("cancelled loop should return an error");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopCancelled { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn llm_setup_error_commits_one_failed_terminal() {
    let (agent_loop, operations) =
        test_agent_loop_with(ProviderBehavior::SetupError, LoopLimits::new(1, 1), None);

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("setup failure should stop the loop");

    assert!(matches!(error, AgentLoopError::Llm { .. }));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopFailed { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn stream_protocol_error_commits_one_failed_terminal() {
    let (error, operations) = run_error(
        vec![ChatStreamEvent::TextDelta {
            delta: "partial".to_owned(),
        }],
        LoopLimits::new(1, 1),
    )
    .await;

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::StreamEndedWithoutFinish,
        }
    ));
    assert_eq!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopFailed { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn failed_terminal_append_preserves_llm_error_and_durability_source() {
    let (agent_loop, operations) =
        test_agent_loop_with(ProviderBehavior::SetupError, LoopLimits::new(1, 1), Some(2));

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("failed terminal acknowledgement should stop the loop");

    assert!(matches!(
        &error,
        AgentLoopError::TerminalDurability { operation, .. }
            if matches!(
                operation.as_ref(),
                AgentLoopError::Llm {
                    source: LlmError::MockExhausted,
                }
            )
    ));
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<DurableEventSinkError>())
            .is_some()
    );
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopFailed { .. })
            ))
            .count(),
        1
    );
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::LoopCancelled { .. })
    )));
}

#[tokio::test]
async fn non_terminal_durability_failure_does_not_attempt_a_terminal_event() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(vec![ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        }]),
        LoopLimits::new(1, 1),
        Some(1),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("prompt durability failure should stop the loop");

    assert!(matches!(error, AgentLoopError::Durability { .. }));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(
            DurableAgentEvent::LoopFailed { .. } | DurableAgentEvent::LoopCancelled { .. }
        )
    )));
}

fn successful_no_tool_events() -> Vec<ChatStreamEvent> {
    vec![
        ChatStreamEvent::TextDelta {
            delta: "complete".to_owned(),
        },
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage: None,
        },
    ]
}

#[tokio::test]
async fn prompt_append_failure_stops_before_provider_and_terminal_actions() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(successful_no_tool_events()),
        LoopLimits::new(1, 1),
        Some(1),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("prompt append should fail");

    assert!(matches!(error, AgentLoopError::Durability { .. }));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Durable(event) => Some(event.event_type()),
                Operation::Telemetry(_) | Operation::ChatStream(_) => None,
            })
            .collect::<Vec<_>>(),
        vec!["loop_started", "message_appended"]
    );
    assert!(
        !operations
            .iter()
            .any(|operation| matches!(operation, Operation::ChatStream(_)))
    );
}

#[tokio::test]
async fn assistant_append_failure_stops_before_iteration_and_finish() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(successful_no_tool_events()),
        LoopLimits::new(1, 1),
        Some(2),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("assistant append should fail");

    assert!(matches!(error, AgentLoopError::Durability { .. }));
    assert_eq!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .filter_map(|operation| match operation {
                Operation::Durable(event) => Some(event.event_type()),
                Operation::Telemetry(_) | Operation::ChatStream(_) => None,
            })
            .collect::<Vec<_>>(),
        vec!["loop_started", "message_appended", "message_appended"]
    );
}

#[tokio::test]
async fn iteration_append_failure_stops_before_loop_finish() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(successful_no_tool_events()),
        LoopLimits::new(1, 1),
        Some(3),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("iteration append should fail");

    assert!(matches!(error, AgentLoopError::Durability { .. }));
    assert_eq!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .filter_map(|operation| match operation {
                Operation::Durable(event) => Some(event.event_type()),
                Operation::Telemetry(_) | Operation::ChatStream(_) => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "loop_started",
            "message_appended",
            "message_appended",
            "iteration_completed",
        ]
    );
}

#[tokio::test]
async fn loop_finished_append_failure_does_not_attempt_another_terminal() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(successful_no_tool_events()),
        LoopLimits::new(1, 1),
        Some(4),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("loop finish append should fail");

    assert!(matches!(error, AgentLoopError::Durability { .. }));
    assert_eq!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .filter_map(|operation| match operation {
                Operation::Durable(event) => Some(event.event_type()),
                Operation::Telemetry(_) | Operation::ChatStream(_) => None,
            })
            .collect::<Vec<_>>(),
        vec![
            "loop_started",
            "message_appended",
            "message_appended",
            "iteration_completed",
            "loop_finished",
        ]
    );
}

#[tokio::test]
async fn mid_stream_llm_error_fails_without_durable_partial_assistant() {
    let (agent_loop, operations) = test_agent_loop_with(
        ProviderBehavior::Items(vec![
            Ok(ChatStreamEvent::TextDelta {
                delta: "partial".to_owned(),
            }),
            Err(LlmError::MockExhausted),
        ]),
        LoopLimits::new(1, 1),
        None,
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("mid-stream provider error should stop the loop");

    assert!(matches!(error, AgentLoopError::Llm { .. }));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(operations.iter().any(|operation| matches!(
        operation,
        Operation::Telemetry(AgentTelemetryEvent::TextDelta { delta, .. })
            if delta == "partial"
    )));
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::MessageAppended { message })
            if message.role == ChatRole::Assistant
    )));
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopFailed { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn cancellation_during_stream_discards_partial_assistant_and_commits_cancelled() {
    let (agent_loop, operations) = test_agent_loop_with(
        ProviderBehavior::PartialThenPending(vec![Ok(ChatStreamEvent::TextDelta {
            delta: "partial".to_owned(),
        })]),
        LoopLimits::new(1, 1),
        None,
    );
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("hello")],
                task_cancellation,
            )
            .await
    });
    timeout(Duration::from_secs(1), async {
        loop {
            if operations
                .lock()
                .expect("operation lock should not be poisoned")
                .iter()
                .any(|operation| {
                    matches!(
                        operation,
                        Operation::Telemetry(AgentTelemetryEvent::TextDelta { delta, .. })
                            if delta == "partial"
                    )
                })
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("partial stream delta should be observed");

    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), task)
        .await
        .expect("cancelled stream should stop")
        .expect("loop task should not panic")
        .expect_err("cancelled stream should return an error");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::MessageAppended { message })
            if message.role == ChatRole::Assistant
    )));
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopCancelled { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn cancelled_terminal_append_failure_preserves_both_errors_once() {
    let (agent_loop, operations) =
        test_agent_loop_with(ProviderBehavior::Pending, LoopLimits::new(1, 1), Some(2));
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("hello")],
                task_cancellation,
            )
            .await
    });
    timeout(Duration::from_secs(1), async {
        loop {
            if operations
                .lock()
                .expect("operation lock should not be poisoned")
                .iter()
                .any(|operation| matches!(operation, Operation::ChatStream(_)))
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider should enter chat setup");

    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), task)
        .await
        .expect("cancelled setup should stop")
        .expect("loop task should not panic")
        .expect_err("failed cancelled terminal should return an error");

    assert!(matches!(
        &error,
        AgentLoopError::TerminalDurability { operation, .. }
            if matches!(operation.as_ref(), AgentLoopError::Cancelled)
    ));
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<DurableEventSinkError>())
            .is_some()
    );
    assert_eq!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopCancelled { .. })
            ))
            .count(),
        1
    );
}

#[tokio::test]
async fn late_tool_call_id_flushes_all_unemitted_arguments_with_name() {
    let events = finished_tool_call_stream(vec![
        tool_delta(0, None, Some("echo"), "{\"value\":"),
        tool_delta(0, None, None, "\""),
        tool_delta(0, Some("call-late"), None, "late"),
        tool_delta(0, None, None, "\"}"),
    ]);
    let (agent_loop, operations) =
        test_agent_loop_with(provider_events(events), LoopLimits::new(1, 1), None);

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("use a tool")],
            CancellationToken::new(),
        )
        .await
        .expect("late provider call id should be accepted");

    assert_eq!(outcome.new_messages[1].tool_calls.len(), 1);
    assert_eq!(
        outcome.new_messages[1].tool_calls[0].arguments,
        json!({"value": "late"})
    );
    let tool_deltas = operations
        .lock()
        .expect("operation lock should not be poisoned")
        .iter()
        .filter_map(|operation| match operation {
            Operation::Telemetry(AgentTelemetryEvent::ToolCallDelta {
                call_id,
                name,
                arguments_delta,
                ..
            }) => Some((call_id.clone(), name.clone(), arguments_delta.clone())),
            Operation::Durable(_) | Operation::Telemetry(_) | Operation::ChatStream(_) => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        tool_deltas,
        vec![
            (
                CallId::from("call-late"),
                Some("echo".to_owned()),
                "{\"value\":\"late".to_owned(),
            ),
            (
                CallId::from("call-late"),
                Some("echo".to_owned()),
                "\"}".to_owned(),
            ),
        ]
    );
}
