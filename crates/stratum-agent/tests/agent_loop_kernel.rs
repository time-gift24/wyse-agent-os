use std::{
    collections::VecDeque,
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
    AgentLoop, AgentLoopBuildError, AgentLoopError, AllowAllToolApproval, LoopContext, LoopLimits,
    ProtocolError, ToolApproval, ToolApprovalError, ToolApprovalRequest, ToolExecutor,
};
use stratum_core::{
    AgentTelemetryEvent, ApprovalDecision, CallId, ChatContent, ChatMessage, ChatRole, DangerLevel,
    DurableAgentEvent, ModelId, TokenUsage, ToolCall, ToolCallDelta, ToolKind, ToolName, ToolSpec,
};
use stratum_infra::{DurableEventSink, DurableEventSinkError, TelemetryEventSink};
use stratum_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use stratum_tools::{
    BuiltinToolRegistry, EchoTool, Tool, ToolError, ToolInput, ToolOutput, ToolPermissionMode,
    ToolRegistry,
};
use tokio::{sync::Notify, time::timeout};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq)]
enum Operation {
    Durable(DurableAgentEvent),
    Telemetry(AgentTelemetryEvent),
    ChatStream(ChatRequest),
    ToolCall { name: ToolName, input: ToolInput },
}

#[test]
fn builder_reports_a_typed_missing_field() {
    let Err(error) = AgentLoop::builder().build() else {
        panic!("an empty builder should fail");
    };

    assert_eq!(error, AgentLoopBuildError::MissingLlmProvider);
}

struct RecordingDurableSink {
    operations: Arc<Mutex<Vec<Operation>>>,
    fail_at: Option<usize>,
    attempts: AtomicUsize,
}

#[derive(Debug, Clone, Copy)]
enum CancellationTrigger {
    FirstToolResult,
    Iteration(u64),
    ApprovalResolved,
}

struct TriggeredCancellationSink {
    operations: Arc<Mutex<Vec<Operation>>>,
    cancellation: CancellationToken,
    trigger: CancellationTrigger,
}

#[async_trait]
impl DurableEventSink for TriggeredCancellationSink {
    async fn append(&self, event: DurableAgentEvent) -> Result<(), DurableEventSinkError> {
        let should_cancel = match (&self.trigger, &event) {
            (
                CancellationTrigger::FirstToolResult,
                DurableAgentEvent::MessageAppended { message },
            ) => message.role == ChatRole::Tool,
            (
                CancellationTrigger::Iteration(expected),
                DurableAgentEvent::IterationCompleted { iteration, .. },
            ) => iteration == expected,
            (
                CancellationTrigger::ApprovalResolved,
                DurableAgentEvent::ToolApprovalResolved { .. },
            ) => true,
            _ => false,
        };
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::Durable(event));
        if should_cancel {
            self.cancellation.cancel();
        }
        Ok(())
    }
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

impl TelemetryEventSink for RecordingTelemetrySink {
    fn emit(&self, event: AgentTelemetryEvent) {
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::Telemetry(event));
    }
}

struct ScriptedProvider {
    operations: Arc<Mutex<Vec<Operation>>>,
    behaviors: Mutex<VecDeque<ProviderBehavior>>,
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
            .behaviors
            .lock()
            .expect("behavior lock should not be poisoned")
            .pop_front()
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

#[derive(Debug, Clone, Copy)]
enum RecordingToolBehavior {
    Echo,
    Fail,
    WaitForCancellation,
    CancelAndEcho,
}

struct RecordingTool {
    spec: ToolSpec,
    operations: Arc<Mutex<Vec<Operation>>>,
    behavior: RecordingToolBehavior,
}

impl RecordingTool {
    fn new(
        name: &str,
        operations: Arc<Mutex<Vec<Operation>>>,
        behavior: RecordingToolBehavior,
    ) -> Self {
        Self {
            spec: ToolSpec::builder()
                .name(name)
                .description("records calls")
                .input_schema(json!({"type": "object"}))
                .build(),
            operations,
            behavior,
        }
    }
}

#[async_trait]
impl Tool for RecordingTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    fn validate(&self, _input: &ToolInput) -> Result<(), ToolError> {
        Ok(())
    }

    async fn call(
        &self,
        input: ToolInput,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::ToolCall {
                name: self.spec.name.clone(),
                input: input.clone(),
            });
        match self.behavior {
            RecordingToolBehavior::Echo => Ok(ToolOutput::new(input.arguments)),
            RecordingToolBehavior::Fail => Err(ToolError::InvalidOperation {
                operation: "scripted failure".to_owned(),
            }),
            RecordingToolBehavior::WaitForCancellation => {
                cancellation.cancelled().await;
                Err(ToolError::Cancelled)
            }
            RecordingToolBehavior::CancelAndEcho => {
                cancellation.cancel();
                Ok(ToolOutput::new(input.arguments))
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FailingToolApproval;

#[async_trait]
impl ToolApproval for FailingToolApproval {
    async fn request(
        &self,
        _request: ToolApprovalRequest,
        _cancellation: &CancellationToken,
    ) -> Result<ApprovalDecision, ToolApprovalError> {
        Err(ToolApprovalError::interaction(std::io::Error::other(
            "scripted approval failure",
        )))
    }
}

#[derive(Debug, Clone)]
struct CancellationAwareApproval {
    entered: Arc<Notify>,
}

#[async_trait]
impl ToolApproval for CancellationAwareApproval {
    async fn request(
        &self,
        _request: ToolApprovalRequest,
        cancellation: &CancellationToken,
    ) -> Result<ApprovalDecision, ToolApprovalError> {
        self.entered.notify_one();
        cancellation.cancelled().await;
        Err(ToolApprovalError::Cancelled)
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
    test_agent_loop_with_behaviors(VecDeque::from([behavior]), limits, fail_at)
}

fn test_agent_loop_with_behaviors(
    behaviors: VecDeque<ProviderBehavior>,
    limits: LoopLimits,
    fail_at: Option<usize>,
) -> (AgentLoop, Arc<Mutex<Vec<Operation>>>) {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let mut registry = BuiltinToolRegistry::default();
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("echo tool should register");
    build_agent_loop(
        behaviors,
        limits,
        fail_at,
        Arc::new(registry),
        Arc::new(AllowAllToolApproval),
        operations,
    )
}

fn build_agent_loop(
    behaviors: VecDeque<ProviderBehavior>,
    limits: LoopLimits,
    fail_at: Option<usize>,
    registry: Arc<dyn ToolRegistry>,
    approval: Arc<dyn ToolApproval>,
    operations: Arc<Mutex<Vec<Operation>>>,
) -> (AgentLoop, Arc<Mutex<Vec<Operation>>>) {
    let durable: Arc<dyn DurableEventSink> = Arc::new(RecordingDurableSink {
        operations: Arc::clone(&operations),
        fail_at,
        attempts: AtomicUsize::new(0),
    });
    let agent_loop = assemble_agent_loop(
        behaviors,
        limits,
        registry,
        approval,
        durable,
        Arc::clone(&operations),
    );
    (agent_loop, operations)
}

fn assemble_agent_loop(
    behaviors: VecDeque<ProviderBehavior>,
    limits: LoopLimits,
    registry: Arc<dyn ToolRegistry>,
    approval: Arc<dyn ToolApproval>,
    durable: Arc<dyn DurableEventSink>,
    operations: Arc<Mutex<Vec<Operation>>>,
) -> AgentLoop {
    let telemetry: Arc<dyn TelemetryEventSink> = Arc::new(RecordingTelemetrySink {
        operations: Arc::clone(&operations),
    });
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        operations: Arc::clone(&operations),
        behaviors: Mutex::new(behaviors),
        model: "scripted:test-model"
            .parse()
            .expect("static model id should parse"),
    });
    let tool_executor = ToolExecutor::new(registry, approval, Arc::clone(&durable));
    AgentLoop::builder()
        .llm_provider(provider)
        .tool_executor(tool_executor)
        .telemetry(telemetry)
        .limits(limits)
        .build()
        .expect("all agent loop fields should be present")
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
            Operation::Durable(_) | Operation::ChatStream(_) | Operation::ToolCall { .. } => None,
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
            Operation::Telemetry(_) | Operation::ChatStream(_) | Operation::ToolCall { .. } => None,
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
        AgentLoopError::ToolCallLimitExceeded { maximum: 16 }
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
    let (agent_loop, operations) = test_agent_loop_with_behaviors(
        VecDeque::from([provider_events(events), stop_turn("done", None)]),
        LoopLimits::new(2, 2),
        None,
    );

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("use tools")],
            CancellationToken::new(),
        )
        .await
        .expect("index limit minus one should be accepted");

    assert_eq!(outcome.new_messages.len(), 5);
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
        AgentLoopError::ToolCallLimitExceeded { maximum: 0 }
    ));
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
        Operation::Durable(DurableAgentEvent::ToolExecutionStarted { .. })
    )));
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
        AgentLoopError::IterationLimitExceeded { maximum: 0 }
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
async fn streamed_text_is_rejected_at_its_byte_limit() {
    let (agent_loop, _) = test_agent_loop_with(
        provider_events(vec![
            ChatStreamEvent::TextDelta {
                delta: "hello".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
        LoopLimits::new(1, 1).with_stream_byte_limits(4, 16, 16),
        None,
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("oversized text should fail");

    assert!(matches!(
        error,
        AgentLoopError::TextByteLimitExceeded { maximum: 4 }
    ));
}

#[tokio::test]
async fn streamed_reasoning_is_rejected_at_its_byte_limit() {
    let (agent_loop, _) = test_agent_loop_with(
        provider_events(vec![
            ChatStreamEvent::ReasoningDelta {
                delta: "think".to_owned(),
            },
            ChatStreamEvent::Finished {
                finish_reason: FinishReason::Stop,
                usage: None,
            },
        ]),
        LoopLimits::new(1, 1).with_stream_byte_limits(16, 4, 16),
        None,
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("oversized reasoning should fail");

    assert!(matches!(
        error,
        AgentLoopError::ReasoningByteLimitExceeded { maximum: 4 }
    ));
}

#[tokio::test]
async fn streamed_tool_arguments_are_rejected_at_their_byte_limit() {
    let (agent_loop, _) = test_agent_loop_with(
        tool_call_turn(
            &[("call-1", "echo", json!({}))],
            FinishReason::ToolCalls,
            None,
        ),
        LoopLimits::new(1, 1).with_stream_byte_limits(16, 16, 1),
        None,
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("hello")],
            CancellationToken::new(),
        )
        .await
        .expect_err("oversized tool arguments should fail");

    assert!(matches!(
        error,
        AgentLoopError::ToolArgumentByteLimitExceeded { maximum: 1 }
    ));
}

#[tokio::test]
async fn failed_terminal_append_preserves_llm_error_and_durability_source() {
    let (agent_loop, operations) = test_agent_loop_with(
        ProviderBehavior::Items(vec![Err(LlmError::MockExhausted)]),
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
async fn durable_failures_stop_at_the_failed_boundary() {
    let cases: &[(usize, &[&str], bool)] = &[
        (1, &["loop_started", "message_appended"], false),
        (
            2,
            &["loop_started", "message_appended", "message_appended"],
            true,
        ),
        (
            3,
            &[
                "loop_started",
                "message_appended",
                "message_appended",
                "iteration_completed",
            ],
            true,
        ),
        (
            4,
            &[
                "loop_started",
                "message_appended",
                "message_appended",
                "iteration_completed",
                "loop_finished",
            ],
            true,
        ),
    ];

    for &(fail_at, expected_events, provider_called) in cases {
        let (agent_loop, operations) = test_agent_loop_with(
            provider_events(successful_no_tool_events()),
            LoopLimits::new(1, 1),
            Some(fail_at),
        );

        let error = agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("hello")],
                CancellationToken::new(),
            )
            .await
            .expect_err("configured durable append should fail");

        assert!(matches!(error, AgentLoopError::Durability { .. }));
        let operations = operations
            .lock()
            .expect("operation lock should not be poisoned");
        let durable_events = operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Durable(event) => Some(event.event_type()),
                Operation::Telemetry(_) | Operation::ChatStream(_) | Operation::ToolCall { .. } => {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert_eq!(durable_events, expected_events, "fail_at {fail_at}");
        assert_eq!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::ChatStream(_))),
            provider_called,
            "fail_at {fail_at}"
        );
    }
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
    let (agent_loop, operations) = test_agent_loop_with_behaviors(
        VecDeque::from([provider_events(events), stop_turn("done", None)]),
        LoopLimits::new(2, 1),
        None,
    );

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
            Operation::Durable(_)
            | Operation::Telemetry(_)
            | Operation::ChatStream(_)
            | Operation::ToolCall { .. } => None,
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

fn tool_call_turn(
    calls: &[(&str, &str, serde_json::Value)],
    finish_reason: FinishReason,
    usage: Option<TokenUsage>,
) -> ProviderBehavior {
    let mut events = Vec::with_capacity(calls.len() + 1);
    for (index, (call_id, name, arguments)) in calls.iter().enumerate() {
        events.push(tool_delta(
            index,
            Some(call_id),
            Some(name),
            &arguments.to_string(),
        ));
    }
    events.push(ChatStreamEvent::Finished {
        finish_reason,
        usage,
    });
    provider_events(events)
}

fn stop_turn(text: &str, usage: Option<TokenUsage>) -> ProviderBehavior {
    provider_events(vec![
        ChatStreamEvent::TextDelta {
            delta: text.to_owned(),
        },
        ChatStreamEvent::Finished {
            finish_reason: FinishReason::Stop,
            usage,
        },
    ])
}

fn recording_registry(
    operations: &Arc<Mutex<Vec<Operation>>>,
    tools: &[(&str, RecordingToolBehavior)],
    permission_mode: ToolPermissionMode,
) -> Arc<dyn ToolRegistry> {
    let mut registry = BuiltinToolRegistry::new(permission_mode);
    for (name, behavior) in tools {
        registry
            .register(
                Arc::new(RecordingTool::new(name, Arc::clone(operations), *behavior)),
                ToolKind::Read,
                DangerLevel::Low,
            )
            .expect("recording tool should register");
    }
    Arc::new(registry)
}

fn without_telemetry(operations: &[Operation]) -> Vec<Operation> {
    operations
        .iter()
        .filter(|operation| !matches!(operation, Operation::Telemetry(_)))
        .cloned()
        .collect()
}

#[tokio::test]
async fn tool_cycle_commits_each_boundary_before_the_next_model_request() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let first_usage = TokenUsage {
        input_tokens: u64::MAX - 1,
        output_tokens: 3,
        total_tokens: u64::MAX - 2,
    };
    let second_usage = TokenUsage {
        input_tokens: 5,
        output_tokens: u64::MAX,
        total_tokens: 10,
    };
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[("call-1", "echo", json!({"value": "one"}))],
                FinishReason::ToolCalls,
                Some(first_usage),
            ),
            stop_turn("done", Some(second_usage)),
        ]),
        LoopLimits::new(2, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );
    let prompt = ChatMessage::user("use echo");

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![prompt.clone()],
            CancellationToken::new(),
        )
        .await
        .expect("tool cycle should finish");

    let call = ToolCall {
        call_id: CallId::from("call-1"),
        name: "echo".to_owned(),
        arguments: json!({"value": "one"}),
    };
    let assistant = ChatMessage::assistant("").with_tool_calls(vec![call.clone()]);
    let result = ChatMessage::tool(call.call_id.clone(), call.arguments.clone());
    let final_assistant = ChatMessage::assistant("done");
    let usage = TokenUsage {
        input_tokens: u64::MAX,
        output_tokens: u64::MAX,
        total_tokens: u64::MAX,
    };
    assert_eq!(
        outcome.new_messages,
        vec![
            prompt.clone(),
            assistant.clone(),
            result.clone(),
            final_assistant.clone(),
        ]
    );
    assert_eq!(outcome.usage, usage);

    let tool_spec = ToolSpec::builder()
        .name("echo")
        .description("records calls")
        .input_schema(json!({"type": "object"}))
        .build();
    let first_request = ChatRequest {
        model: "scripted:test-model"
            .parse()
            .expect("static model id should parse"),
        messages: vec![ChatMessage::system("be precise"), prompt.clone()],
        tools: vec![tool_spec.clone()],
        structured_output: None,
    };
    let second_request = ChatRequest {
        model: first_request.model.clone(),
        messages: vec![
            ChatMessage::system("be precise"),
            prompt.clone(),
            assistant.clone(),
            result.clone(),
        ],
        tools: vec![tool_spec],
        structured_output: None,
    };
    assert_eq!(
        without_telemetry(
            &recorded
                .lock()
                .expect("operation lock should not be poisoned")
        ),
        vec![
            Operation::Durable(DurableAgentEvent::LoopStarted),
            Operation::Durable(DurableAgentEvent::MessageAppended { message: prompt }),
            Operation::ChatStream(first_request),
            Operation::Durable(DurableAgentEvent::MessageAppended { message: assistant }),
            Operation::Durable(DurableAgentEvent::ToolExecutionStarted {
                call_id: call.call_id.clone(),
                tool_name: ToolName::new("echo"),
            }),
            Operation::ToolCall {
                name: ToolName::new("echo"),
                input: ToolInput::new(call.call_id.clone(), call.arguments.clone()),
            },
            Operation::Durable(DurableAgentEvent::MessageAppended { message: result }),
            Operation::Durable(DurableAgentEvent::IterationCompleted {
                iteration: 0,
                usage: first_usage,
            }),
            Operation::ChatStream(second_request),
            Operation::Durable(DurableAgentEvent::MessageAppended {
                message: final_assistant,
            }),
            Operation::Durable(DurableAgentEvent::IterationCompleted {
                iteration: 1,
                usage,
            }),
            Operation::Durable(DurableAgentEvent::LoopFinished {
                finish_reason: "stop".to_owned(),
                usage,
            }),
        ]
    );
}

#[tokio::test]
async fn multiple_tools_execute_and_commit_strictly_in_assistant_order() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[
            ("alpha", RecordingToolBehavior::Echo),
            ("beta", RecordingToolBehavior::Echo),
        ],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[
                    ("call-a", "alpha", json!({"order": 1})),
                    ("call-b", "beta", json!({"order": 2})),
                ],
                FinishReason::ToolCalls,
                None,
            ),
            stop_turn("done", None),
        ]),
        LoopLimits::new(2, 2),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("use both")],
            CancellationToken::new(),
        )
        .await
        .expect("two tools should finish");

    let ordered = recorded
        .lock()
        .expect("operation lock should not be poisoned")
        .iter()
        .filter_map(|operation| match operation {
            Operation::Durable(DurableAgentEvent::ToolExecutionStarted { call_id, .. }) => {
                Some(("started", call_id.clone()))
            }
            Operation::ToolCall { input, .. } => Some(("called", input.call_id.clone())),
            Operation::Durable(DurableAgentEvent::MessageAppended { message })
                if message.role == ChatRole::Tool =>
            {
                Some((
                    "committed",
                    message
                        .tool_call_id
                        .clone()
                        .expect("tool message should identify its call"),
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        ordered,
        vec![
            ("started", CallId::from("call-a")),
            ("called", CallId::from("call-a")),
            ("committed", CallId::from("call-a")),
            ("started", CallId::from("call-b")),
            ("called", CallId::from("call-b")),
            ("committed", CallId::from("call-b")),
        ]
    );
}

#[tokio::test]
async fn failed_and_missing_tool_results_are_committed_and_visible_to_the_model() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("broken", RecordingToolBehavior::Fail)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[
                    ("call-fail", "broken", json!({})),
                    ("call-missing", "missing", json!({})),
                ],
                FinishReason::ToolCalls,
                None,
            ),
            stop_turn("recovered", None),
        ]),
        LoopLimits::new(2, 2),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("try tools")],
            CancellationToken::new(),
        )
        .await
        .expect("tool failures should remain model-visible results");

    assert_eq!(outcome.new_messages.len(), 5);
    assert_eq!(outcome.new_messages[2].role, ChatRole::Tool);
    assert_eq!(outcome.new_messages[3].role, ChatRole::Tool);
    let requests = recorded
        .lock()
        .expect("operation lock should not be poisoned")
        .iter()
        .filter_map(|operation| match operation {
            Operation::ChatStream(request) => Some(request.clone()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert_eq!(&requests[1].messages[3..], &outcome.new_messages[2..4]);
}

#[tokio::test]
async fn invalid_builtin_tool_input_is_committed_without_approval_or_execution_start() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let mut registry = BuiltinToolRegistry::new(ToolPermissionMode::RequireApproval);
    registry
        .register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)
        .expect("echo tool should register");
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[("call-invalid", "echo", json!(42))],
                FinishReason::ToolCalls,
                None,
            ),
            stop_turn("recovered", None),
        ]),
        LoopLimits::new(2, 1),
        None,
        Arc::new(registry),
        Arc::new(FailingToolApproval),
        Arc::clone(&operations),
    );

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("use echo")],
            CancellationToken::new(),
        )
        .await
        .expect("invalid input should be returned to the model without approval");

    let expected_result = ChatMessage::tool(
        CallId::new("call-invalid"),
        json!({"error": "invalid argument arguments: must be an object"}),
    );
    assert_eq!(outcome.new_messages[2], expected_result);
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    let requests = recorded
        .iter()
        .filter_map(|operation| match operation {
            Operation::ChatStream(request) => Some(request),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[1].messages.last(), Some(&expected_result));
    assert!(recorded.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::MessageAppended { message })
            if message == &expected_result
    )));
    assert!(!recorded.iter().any(|operation| matches!(
        operation,
        Operation::Durable(
            DurableAgentEvent::ToolApprovalRequested { .. }
                | DurableAgentEvent::ToolApprovalResolved { .. }
                | DurableAgentEvent::ToolExecutionStarted { .. }
        ) | Operation::ToolCall { .. }
    )));
}

#[tokio::test]
async fn iteration_limit_fails_once_before_another_model_request() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-1", "echo", json!({}))],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(1, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("loop")],
            CancellationToken::new(),
        )
        .await
        .expect_err("iteration limit should stop before a second model call");

    assert!(matches!(
        error,
        AgentLoopError::IterationLimitExceeded { maximum: 1 }
    ));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(operation, Operation::ChatStream(_)))
            .count(),
        1
    );
    assert_eq!(
        recorded
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
async fn cancellation_during_last_started_tool_wins_after_result_and_iteration_commit() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("cancel_after_effect", RecordingToolBehavior::CancelAndEcho)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[(
                "call-cancel",
                "cancel_after_effect",
                json!({"effect": "completed"}),
            )],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(1, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("run once")],
            CancellationToken::new(),
        )
        .await
        .expect_err("cancellation should win after the started tool boundary completes");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(operation, Operation::ChatStream(_)))
            .count(),
        1
    );
    let durable = recorded
        .iter()
        .filter_map(|operation| match operation {
            Operation::Durable(event) => Some(event),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(durable.iter().any(|event| matches!(
        event,
        DurableAgentEvent::MessageAppended { message }
            if message.role == ChatRole::Tool
                && message.content == ChatContent::Json(json!({"effect": "completed"}))
    )));
    assert!(durable.iter().any(|event| matches!(
        event,
        DurableAgentEvent::IterationCompleted { iteration: 0, .. }
    )));
    assert_eq!(
        durable
            .iter()
            .filter(|event| matches!(event, DurableAgentEvent::LoopCancelled { .. }))
            .count(),
        1
    );
    assert!(
        !durable
            .iter()
            .any(|event| matches!(event, DurableAgentEvent::LoopFailed { .. }))
    );
}

#[tokio::test]
async fn length_with_calls_commits_typed_errors_without_executing_tools() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[
                    ("call-a", "echo", json!({"partial": 1})),
                    ("call-b", "echo", json!({"partial": 2})),
                ],
                FinishReason::Length,
                None,
            ),
            stop_turn("recovered", None),
        ]),
        LoopLimits::new(2, 2),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let outcome = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("try")],
            CancellationToken::new(),
        )
        .await
        .expect("truncated calls should be reported back to the model");

    let expected_payload = json!({
        "error": {
            "code": "tool_call_truncated",
            "message": "tool call was not executed because the model response reached its length limit"
        }
    });
    assert_eq!(
        &outcome.new_messages[2..4],
        &[
            ChatMessage::tool("call-a", expected_payload.clone()),
            ChatMessage::tool("call-b", expected_payload),
        ]
    );
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(
        !recorded
            .iter()
            .any(|operation| matches!(operation, Operation::ToolCall { .. }))
    );
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(operation, Operation::ChatStream(_)))
            .count(),
        2
    );
    let second_request = recorded
        .iter()
        .filter_map(|operation| match operation {
            Operation::ChatStream(request) => Some(request),
            _ => None,
        })
        .nth(1)
        .expect("truncation results should be sent to the next model request");
    assert_eq!(&second_request.messages[3..5], &outcome.new_messages[2..4]);
}

#[tokio::test]
async fn unexpected_finish_reason_commits_errors_without_executing_tools() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[("call-1", "echo", json!({"value": 1}))],
                FinishReason::ContentFilter,
                None,
            ),
            stop_turn("done", None),
        ]),
        LoopLimits::new(2, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("tool")],
            CancellationToken::new(),
        )
        .await
        .expect("blocked calls should be reported back to the model");

    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(operation, Operation::ToolCall { .. }))
            .count(),
        0
    );
    let second_request = recorded
        .iter()
        .filter_map(|operation| match operation {
            Operation::ChatStream(request) => Some(request),
            _ => None,
        })
        .nth(1)
        .expect("blocked result should be sent to the next model request");
    assert_eq!(
        second_request.messages.last(),
        Some(&ChatMessage::tool(
            "call-1",
            json!({
                "error": {
                    "code": "tool_call_not_authorized",
                    "message": "tool call was not executed because the model did not finish with tool_calls"
                }
            }),
        ))
    );
}

#[tokio::test]
async fn tool_executor_durability_failure_is_fail_closed_without_terminal_retry() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-1", "echo", json!({}))],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(2, 1),
        Some(3),
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("tool")],
            CancellationToken::new(),
        )
        .await
        .expect_err("tool start acknowledgement should fail closed");

    assert!(matches!(error, AgentLoopError::Durability { .. }));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(recorded.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::ToolExecutionStarted { call_id, .. })
            if call_id == &CallId::from("call-1")
    )));
    assert!(
        !recorded
            .iter()
            .any(|operation| matches!(operation, Operation::ToolCall { .. }))
    );
    assert!(!recorded.iter().any(|operation| matches!(
        operation,
        Operation::Durable(
            DurableAgentEvent::LoopFailed { .. } | DurableAgentEvent::LoopCancelled { .. }
        )
    )));
}

#[tokio::test]
async fn tool_executor_approval_failure_preserves_source_and_commits_loop_failed() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::RequireApproval,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-1", "echo", json!({}))],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(2, 1),
        None,
        registry,
        Arc::new(FailingToolApproval),
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("tool")],
            CancellationToken::new(),
        )
        .await
        .expect_err("approval backend failure should stop the loop");

    assert!(matches!(&error, AgentLoopError::ToolExecution { .. }));
    assert!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<stratum_agent::ToolExecutorError>())
            .is_some()
    );
    assert_eq!(
        recorded
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
async fn repeated_call_id_in_a_later_iteration_fails_before_committing_or_dispatching_it() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[("call-reused", "echo", json!({"iteration": 1}))],
                FinishReason::ToolCalls,
                None,
            ),
            tool_call_turn(
                &[("call-reused", "echo", json!({"iteration": 2}))],
                FinishReason::ToolCalls,
                None,
            ),
        ]),
        LoopLimits::new(3, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("reuse")],
            CancellationToken::new(),
        )
        .await
        .expect_err("a later iteration must not reuse a committed call id");

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::DuplicateToolCallId { call_id },
        } if call_id == CallId::from("call-reused")
    ));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::MessageAppended { message })
                    if message.role == ChatRole::Assistant
            ))
            .count(),
        1
    );
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(operation, Operation::ToolCall { .. }))
            .count(),
        1
    );
    assert_eq!(
        recorded
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
async fn call_id_from_initial_context_cannot_be_reused_by_a_new_assistant() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-existing", "echo", json!({"new": true}))],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(2, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );
    let historical_call = ToolCall {
        call_id: CallId::from("call-existing"),
        name: "echo".to_owned(),
        arguments: json!({"old": true}),
    };
    let context = LoopContext::new("be precise").with_messages(vec![
        ChatMessage::assistant("").with_tool_calls(vec![historical_call.clone()]),
        ChatMessage::tool(historical_call.call_id, json!({"old": true})),
    ]);

    let error = agent_loop
        .run(
            context,
            vec![ChatMessage::user("reuse history")],
            CancellationToken::new(),
        )
        .await
        .expect_err("new calls must not collide with committed history");

    assert!(matches!(
        error,
        AgentLoopError::InvalidProtocol {
            reason: ProtocolError::DuplicateToolCallId { call_id },
        } if call_id == CallId::from("call-existing")
    ));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert!(!recorded.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::MessageAppended { message })
            if message.role == ChatRole::Assistant
    )));
    assert!(
        !recorded
            .iter()
            .any(|operation| matches!(operation, Operation::ToolCall { .. }))
    );
    assert_eq!(
        recorded
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
async fn fail_closed_durable_ordinals_stop_all_later_tool_actions() {
    let boundaries = [
        "loop_started",
        "message_appended",
        "message_appended",
        "tool_approval_requested",
        "tool_approval_resolved",
        "tool_execution_started",
        "message_appended",
        "iteration_completed",
    ];

    for fail_at in 0..boundaries.len() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let registry = recording_registry(
            &operations,
            &[("echo", RecordingToolBehavior::Echo)],
            ToolPermissionMode::RequireApproval,
        );
        let (agent_loop, recorded) = build_agent_loop(
            VecDeque::from([tool_call_turn(
                &[("call-ordinal", "echo", json!({"value": 1}))],
                FinishReason::ToolCalls,
                None,
            )]),
            LoopLimits::new(2, 1),
            Some(fail_at),
            registry,
            Arc::new(AllowAllToolApproval),
            Arc::clone(&operations),
        );

        let error = agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("exercise durable boundaries")],
                CancellationToken::new(),
            )
            .await
            .expect_err(boundaries[fail_at]);

        assert!(
            matches!(error, AgentLoopError::Durability { .. }),
            "unexpected error at {}",
            boundaries[fail_at]
        );
        let recorded = recorded
            .lock()
            .expect("operation lock should not be poisoned");
        assert_eq!(
            recorded
                .iter()
                .filter_map(|operation| match operation {
                    Operation::Durable(event) => Some(event.event_type()),
                    Operation::Telemetry(_)
                    | Operation::ChatStream(_)
                    | Operation::ToolCall { .. } => None,
                })
                .collect::<Vec<_>>(),
            boundaries[..=fail_at],
            "durable attempts continued after {} failed",
            boundaries[fail_at]
        );
        assert_eq!(
            recorded
                .iter()
                .filter(|operation| matches!(operation, Operation::ChatStream(_)))
                .count(),
            usize::from(fail_at >= 2),
            "model activity crossed {}",
            boundaries[fail_at]
        );
        assert_eq!(
            recorded
                .iter()
                .filter(|operation| matches!(operation, Operation::ToolCall { .. }))
                .count(),
            usize::from(fail_at >= 6),
            "tool activity crossed {}",
            boundaries[fail_at]
        );
        assert!(!recorded.iter().any(|operation| matches!(
            operation,
            Operation::Durable(
                DurableAgentEvent::LoopFailed { .. }
                    | DurableAgentEvent::LoopCancelled { .. }
                    | DurableAgentEvent::LoopFinished { .. }
            )
        )));
    }
}

#[tokio::test]
async fn cancellation_before_first_model_call_records_one_cancelled_terminal() {
    let (agent_loop, operations) = test_agent_loop_with(
        provider_events(successful_no_tool_events()),
        LoopLimits::new(1, 1),
        None,
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("do not call the model")],
            cancellation,
        )
        .await
        .expect_err("pre-cancelled loop should stop");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter_map(|operation| match operation {
                Operation::Durable(event) => Some(event.event_type()),
                Operation::Telemetry(_) | Operation::ChatStream(_) | Operation::ToolCall { .. } =>
                    None,
            })
            .collect::<Vec<_>>(),
        vec!["loop_started", "loop_cancelled"]
    );
    assert!(
        !operations
            .iter()
            .any(|operation| matches!(operation, Operation::ChatStream(_)))
    );
}

#[tokio::test]
async fn cancelled_approval_maps_to_loop_cancellation_without_execution() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let approval_entered = Arc::new(Notify::new());
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::RequireApproval,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-approval", "echo", json!({}))],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(2, 1),
        None,
        registry,
        Arc::new(CancellationAwareApproval {
            entered: Arc::clone(&approval_entered),
        }),
        Arc::clone(&operations),
    );
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("request approval")],
                task_cancellation,
            )
            .await
    });
    timeout(Duration::from_secs(1), approval_entered.notified())
        .await
        .expect("approval implementation should begin waiting");

    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), task)
        .await
        .expect("approval cancellation should stop")
        .expect("loop task should not panic")
        .expect_err("approval cancellation should return an error");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopCancelled { .. })
            ))
            .count(),
        1
    );
    assert!(!recorded.iter().any(|operation| matches!(
        operation,
        Operation::Durable(
            DurableAgentEvent::ToolApprovalResolved { .. }
                | DurableAgentEvent::ToolExecutionStarted { .. }
                | DurableAgentEvent::LoopFailed { .. }
        ) | Operation::ToolCall { .. }
    )));
}

#[tokio::test]
async fn cancellation_after_approval_resolution_stops_before_execution_start() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let cancellation = CancellationToken::new();
    let durable: Arc<dyn DurableEventSink> = Arc::new(TriggeredCancellationSink {
        operations: Arc::clone(&operations),
        cancellation: cancellation.clone(),
        trigger: CancellationTrigger::ApprovalResolved,
    });
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::RequireApproval,
    );
    let agent_loop = assemble_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-approval", "echo", json!({}))],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(2, 1),
        registry,
        Arc::new(AllowAllToolApproval),
        durable,
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("request approval")],
            cancellation,
        )
        .await
        .expect_err("cancellation after approval resolution should stop execution");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    let durable_types = operations
        .iter()
        .filter_map(|operation| match operation {
            Operation::Durable(event) => Some(event.event_type()),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        durable_types,
        vec![
            "loop_started",
            "message_appended",
            "message_appended",
            "tool_approval_requested",
            "tool_approval_resolved",
            "loop_cancelled",
        ]
    );
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
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(
            DurableAgentEvent::ToolExecutionStarted { .. } | DurableAgentEvent::LoopFailed { .. }
        ) | Operation::ToolCall { .. }
    )));
}

#[tokio::test]
async fn cancellation_after_tool_start_awaits_result_and_completes_iteration() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let usage = TokenUsage {
        input_tokens: 7,
        output_tokens: 3,
        total_tokens: 10,
    };
    let registry = recording_registry(
        &operations,
        &[("cooperative", RecordingToolBehavior::WaitForCancellation)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([tool_call_turn(
            &[("call-cooperative", "cooperative", json!({}))],
            FinishReason::ToolCalls,
            Some(usage),
        )]),
        LoopLimits::new(2, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        agent_loop
            .run(
                LoopContext::new("be precise"),
                vec![ChatMessage::user("run cooperative tool")],
                task_cancellation,
            )
            .await
    });
    wait_for_tool_calls(&recorded, 1).await;

    cancellation.cancel();
    let error = timeout(Duration::from_secs(1), task)
        .await
        .expect("cooperative tool should finish after cancellation")
        .expect("loop task should not panic")
        .expect_err("cancelled loop should return an error");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let recorded = recorded
        .lock()
        .expect("operation lock should not be poisoned");
    let durable = recorded
        .iter()
        .filter_map(|operation| match operation {
            Operation::Durable(event) => Some(event),
            Operation::Telemetry(_) | Operation::ChatStream(_) | Operation::ToolCall { .. } => None,
        })
        .collect::<Vec<_>>();
    let result_position = durable
        .iter()
        .position(|event| {
            matches!(event, DurableAgentEvent::MessageAppended { message }
            if message.role == ChatRole::Tool)
        })
        .expect("tool result must be committed");
    let iteration_position = durable
        .iter()
        .position(|event| matches!(event, DurableAgentEvent::IterationCompleted { .. }))
        .expect("completed tool set must close its iteration");
    let cancelled_position = durable
        .iter()
        .position(|event| {
            matches!(event, DurableAgentEvent::LoopCancelled { usage: event_usage }
            if *event_usage == usage)
        })
        .expect("cancellation terminal must retain completed stream usage");
    assert!(result_position < iteration_position && iteration_position < cancelled_position);
    assert_eq!(
        recorded
            .iter()
            .filter(|operation| matches!(operation, Operation::ChatStream(_)))
            .count(),
        1
    );
}

#[tokio::test]
async fn cancellation_committed_after_first_result_prevents_second_tool_start() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let cancellation = CancellationToken::new();
    let durable: Arc<dyn DurableEventSink> = Arc::new(TriggeredCancellationSink {
        operations: Arc::clone(&operations),
        cancellation: cancellation.clone(),
        trigger: CancellationTrigger::FirstToolResult,
    });
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let agent_loop = assemble_agent_loop(
        VecDeque::from([tool_call_turn(
            &[
                ("call-first", "echo", json!({"order": 1})),
                ("call-second", "echo", json!({"order": 2})),
            ],
            FinishReason::ToolCalls,
            None,
        )]),
        LoopLimits::new(2, 2),
        registry,
        Arc::new(AllowAllToolApproval),
        durable,
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("cancel after first result")],
            cancellation,
        )
        .await
        .expect_err("durably triggered cancellation should stop the tool sequence");

    assert!(matches!(error, AgentLoopError::Cancelled));
    let operations = operations
        .lock()
        .expect("operation lock should not be poisoned");
    assert_eq!(
        operations
            .iter()
            .filter(|operation| matches!(operation, Operation::ToolCall { .. }))
            .count(),
        1
    );
    assert!(operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::MessageAppended { message })
            if message.role == ChatRole::Tool
                && message.tool_call_id.as_ref() == Some(&CallId::from("call-first"))
    )));
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::ToolExecutionStarted { call_id, .. })
            if call_id == &CallId::from("call-second")
    )));
    assert!(!operations.iter().any(|operation| matches!(
        operation,
        Operation::Durable(DurableAgentEvent::IterationCompleted { .. })
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

async fn wait_for_tool_calls(operations: &Arc<Mutex<Vec<Operation>>>, expected: usize) {
    timeout(Duration::from_secs(1), async {
        loop {
            if operations
                .lock()
                .expect("operation lock should not be poisoned")
                .iter()
                .filter(|operation| matches!(operation, Operation::ToolCall { .. }))
                .count()
                >= expected
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("tool call should start");
}

#[tokio::test]
async fn failed_terminal_usage_saturates_completed_frames_and_excludes_failed_stream() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let (agent_loop, recorded) = build_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[("call-usage-a", "echo", json!({}))],
                FinishReason::ToolCalls,
                Some(TokenUsage {
                    input_tokens: u64::MAX - 1,
                    output_tokens: 2,
                    total_tokens: u64::MAX - 2,
                }),
            ),
            tool_call_turn(
                &[("call-usage-b", "echo", json!({}))],
                FinishReason::ToolCalls,
                Some(TokenUsage {
                    input_tokens: 4,
                    output_tokens: u64::MAX,
                    total_tokens: 8,
                }),
            ),
            ProviderBehavior::Items(vec![Err(LlmError::MockExhausted)]),
        ]),
        LoopLimits::new(3, 1),
        None,
        registry,
        Arc::new(AllowAllToolApproval),
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("accumulate usage")],
            CancellationToken::new(),
        )
        .await
        .expect_err("third stream should fail");

    assert!(matches!(error, AgentLoopError::Llm { .. }));
    assert!(
        recorded
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .any(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopFailed { usage, .. })
                    if *usage == TokenUsage {
                        input_tokens: u64::MAX,
                        output_tokens: u64::MAX,
                        total_tokens: u64::MAX,
                    }
            ))
    );
}

#[tokio::test]
async fn cancelled_terminal_usage_saturates_only_completed_finished_frames() {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let cancellation = CancellationToken::new();
    let durable: Arc<dyn DurableEventSink> = Arc::new(TriggeredCancellationSink {
        operations: Arc::clone(&operations),
        cancellation: cancellation.clone(),
        trigger: CancellationTrigger::Iteration(1),
    });
    let registry = recording_registry(
        &operations,
        &[("echo", RecordingToolBehavior::Echo)],
        ToolPermissionMode::Allow,
    );
    let agent_loop = assemble_agent_loop(
        VecDeque::from([
            tool_call_turn(
                &[("call-cancel-usage-a", "echo", json!({}))],
                FinishReason::ToolCalls,
                Some(TokenUsage {
                    input_tokens: u64::MAX - 1,
                    output_tokens: 2,
                    total_tokens: u64::MAX - 2,
                }),
            ),
            tool_call_turn(
                &[("call-cancel-usage-b", "echo", json!({}))],
                FinishReason::ToolCalls,
                Some(TokenUsage {
                    input_tokens: 4,
                    output_tokens: u64::MAX,
                    total_tokens: 8,
                }),
            ),
        ]),
        LoopLimits::new(3, 1),
        registry,
        Arc::new(AllowAllToolApproval),
        durable,
        Arc::clone(&operations),
    );

    let error = agent_loop
        .run(
            LoopContext::new("be precise"),
            vec![ChatMessage::user("cancel with accumulated usage")],
            cancellation,
        )
        .await
        .expect_err("cancellation after the second iteration should stop the loop");

    assert!(matches!(error, AgentLoopError::Cancelled));
    assert!(
        operations
            .lock()
            .expect("operation lock should not be poisoned")
            .iter()
            .any(|operation| matches!(
                operation,
                Operation::Durable(DurableAgentEvent::LoopCancelled { usage })
                    if *usage == TokenUsage {
                        input_tokens: u64::MAX,
                        output_tokens: u64::MAX,
                        total_tokens: u64::MAX,
                    }
            ))
    );
}
