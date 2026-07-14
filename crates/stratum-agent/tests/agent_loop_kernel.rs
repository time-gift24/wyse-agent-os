use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_util::stream;
use serde_json::json;
use stratum_agent::{
    AgentLoop, AgentLoopBuildError, AgentLoopError, AllowAllToolApproval, LoopContext, LoopLimits,
    ProtocolError, ToolExecutor,
};
use stratum_core::{
    AgentTelemetryEvent, ChatMessage, ChatRole, DangerLevel, DurableAgentEvent, ModelId,
    TokenUsage, ToolKind,
};
use stratum_infra::{DurableEventSink, DurableEventSinkError, TelemetryEventSink};
use stratum_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, FinishReason, LlmError, LlmProvider,
};
use stratum_tools::{BuiltinToolRegistry, EchoTool, ToolRegistry};
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
            field: "llm_provider",
        }
    );
}

struct RecordingDurableSink {
    operations: Arc<Mutex<Vec<Operation>>>,
}

#[async_trait]
impl DurableEventSink for RecordingDurableSink {
    async fn append(&self, event: DurableAgentEvent) -> Result<(), DurableEventSinkError> {
        self.operations
            .lock()
            .expect("operation lock should not be poisoned")
            .push(Operation::Durable(event));
        Ok(())
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
    events: Mutex<Option<Vec<ChatStreamEvent>>>,
    model: ModelId,
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
        let events = self
            .events
            .lock()
            .expect("event lock should not be poisoned")
            .take()
            .ok_or(LlmError::MockExhausted)?;
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

fn test_agent_loop(events: Vec<ChatStreamEvent>) -> (AgentLoop, Arc<Mutex<Vec<Operation>>>) {
    let operations = Arc::new(Mutex::new(Vec::new()));
    let durable: Arc<dyn DurableEventSink> = Arc::new(RecordingDurableSink {
        operations: Arc::clone(&operations),
    });
    let telemetry: Arc<dyn TelemetryEventSink> = Arc::new(RecordingTelemetrySink {
        operations: Arc::clone(&operations),
    });
    let provider: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider {
        operations: Arc::clone(&operations),
        events: Mutex::new(Some(events)),
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
        .limits(LoopLimits::default())
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
