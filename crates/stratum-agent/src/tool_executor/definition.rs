use std::sync::Arc;

use serde_json::json;
use stratum_core::{
    ApprovalDecision, ApprovalId, ChatMessage, DurableAgentEvent, ToolCall, ToolName, ToolSpec,
};
use stratum_infra::DurableEventSink;
use stratum_tools::{ToolInput, ToolRegistry};
use tokio_util::sync::CancellationToken;

use super::{ToolApproval, ToolApprovalRequest, ToolExecutorError};

/// Result of processing one provider tool call.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ToolExecutionOutcome {
    message: ChatMessage,
    reached_tool: bool,
}

impl ToolExecutionOutcome {
    /// Borrows the model-visible tool result message.
    #[must_use]
    pub const fn message(&self) -> &ChatMessage {
        &self.message
    }

    /// Returns whether the external tool implementation was invoked.
    #[must_use]
    pub const fn reached_tool(&self) -> bool {
        self.reached_tool
    }

    /// Consumes the outcome and returns its model-visible message.
    #[must_use]
    pub fn into_message(self) -> ChatMessage {
        self.message
    }
}

/// Sequential executor that durably gates external tool calls.
pub struct ToolExecutor {
    registry: Arc<dyn ToolRegistry>,
    approval: Arc<dyn ToolApproval>,
    durable_events: Arc<dyn DurableEventSink>,
}

impl ToolExecutor {
    /// Creates an executor from its registry, approval policy, and durable sink.
    #[must_use]
    pub fn new(
        registry: Arc<dyn ToolRegistry>,
        approval: Arc<dyn ToolApproval>,
        durable_events: Arc<dyn DurableEventSink>,
    ) -> Self {
        Self {
            registry,
            approval,
            durable_events,
        }
    }

    /// Returns provider-visible specifications from the registry.
    #[must_use]
    pub fn specs(&self) -> Vec<ToolSpec> {
        self.registry.specs()
    }

    /// Processes one provider tool call.
    ///
    /// # Errors
    ///
    /// Returns an error when a required durable event or approval interaction fails.
    pub async fn execute(
        &self,
        tool_call: &ToolCall,
        cancellation: &CancellationToken,
    ) -> Result<ToolExecutionOutcome, ToolExecutorError> {
        let tool_name = ToolName::new(tool_call.name.clone());
        let authorization = match self.registry.authorization(&tool_name) {
            Ok(authorization) => authorization,
            Err(error) => {
                return Ok(ToolExecutionOutcome {
                    message: ChatMessage::tool(
                        tool_call.call_id.clone(),
                        json!({"error": error.to_string()}),
                    ),
                    reached_tool: false,
                });
            }
        };

        if let Some((tool_kind, danger_level)) = authorization {
            let request = ToolApprovalRequest {
                approval_id: ApprovalId::new(),
                call_id: tool_call.call_id.clone(),
                tool_name: tool_name.clone(),
                arguments: tool_call.arguments.clone(),
                tool_kind,
                danger_level,
            };
            self.durable_events
                .append(DurableAgentEvent::ToolApprovalRequested {
                    approval_id: request.approval_id,
                    call_id: request.call_id.clone(),
                    tool_name: request.tool_name.clone(),
                    arguments: request.arguments.clone(),
                    tool_kind: request.tool_kind,
                    danger_level: request.danger_level,
                })
                .await?;
            let decision = self.approval.request(request.clone(), cancellation).await?;
            self.durable_events
                .append(DurableAgentEvent::ToolApprovalResolved {
                    approval_id: request.approval_id,
                    decision,
                })
                .await?;
            match decision {
                ApprovalDecision::Reject => {
                    return Ok(ToolExecutionOutcome {
                        message: ChatMessage::tool(
                            tool_call.call_id.clone(),
                            json!({"error": "tool approval rejected"}),
                        ),
                        reached_tool: false,
                    });
                }
                ApprovalDecision::Approve => {}
                _ => return Err(ToolExecutorError::UnsupportedApprovalDecision),
            }
        }

        self.durable_events
            .append(DurableAgentEvent::ToolExecutionStarted {
                call_id: tool_call.call_id.clone(),
                tool_name: tool_name.clone(),
            })
            .await?;
        let result = self
            .registry
            .call(
                &tool_name,
                ToolInput::new(tool_call.call_id.clone(), tool_call.arguments.clone()),
                cancellation,
            )
            .await;
        let payload = match result {
            Ok(output) => output.result,
            Err(error) => json!({"error": error.to_string()}),
        };
        Ok(ToolExecutionOutcome {
            message: ChatMessage::tool(tool_call.call_id.clone(), payload),
            reached_tool: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use serde_json::json;
    use stratum_core::{
        ApprovalDecision, CallId, ChatMessage, DangerLevel, DurableAgentEvent, ToolCall, ToolKind,
        ToolName, ToolSpec,
    };
    use stratum_infra::{DurableEventSink, DurableEventSinkError};
    use stratum_tools::{Tool, ToolError, ToolInput, ToolOutput, ToolRegistry};
    use tokio_util::sync::CancellationToken;

    use crate::tool_executor::{
        ToolApproval, ToolApprovalError, ToolApprovalRequest, ToolExecutor,
    };

    #[derive(Debug, Clone, PartialEq)]
    enum Operation {
        Authorization(ToolName),
        Durable(DurableAgentEvent),
        Approval(ToolApprovalRequest),
        ToolCall {
            name: ToolName,
            input: ToolInput,
            cancelled: bool,
        },
    }

    #[derive(Debug, Clone)]
    enum RegistryCallResult {
        Success(serde_json::Value),
        Failure,
        Cancelled,
    }

    struct RecordingRegistry {
        operations: Arc<Mutex<Vec<Operation>>>,
        missing: bool,
        approval: Option<(ToolKind, DangerLevel)>,
        specs: Vec<ToolSpec>,
        call_result: RegistryCallResult,
    }

    #[async_trait]
    impl ToolRegistry for RecordingRegistry {
        fn register(
            &mut self,
            _tool: Arc<dyn Tool>,
            _tool_kind: ToolKind,
            _danger_level: DangerLevel,
        ) -> Result<(), ToolError> {
            unreachable!("the executor never registers tools")
        }

        fn authorization(
            &self,
            name: &ToolName,
        ) -> Result<Option<(ToolKind, DangerLevel)>, ToolError> {
            self.operations
                .lock()
                .expect("operation lock should not be poisoned")
                .push(Operation::Authorization(name.clone()));
            if self.missing {
                Err(ToolError::ToolNotFound { name: name.clone() })
            } else {
                Ok(self.approval)
            }
        }

        fn get(&self, _name: &ToolName) -> Option<Arc<dyn Tool>> {
            None
        }

        fn specs(&self) -> Vec<ToolSpec> {
            self.specs.clone()
        }

        async fn call(
            &self,
            name: &ToolName,
            input: ToolInput,
            cancellation: &CancellationToken,
        ) -> Result<ToolOutput, ToolError> {
            self.operations
                .lock()
                .expect("operation lock should not be poisoned")
                .push(Operation::ToolCall {
                    name: name.clone(),
                    input,
                    cancelled: cancellation.is_cancelled(),
                });
            match &self.call_result {
                RegistryCallResult::Success(result) => Ok(ToolOutput::new(result.clone())),
                RegistryCallResult::Failure => Err(ToolError::InvalidArgument {
                    name: "value",
                    reason: "test failure",
                }),
                RegistryCallResult::Cancelled => Err(ToolError::Cancelled),
            }
        }
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

    struct FailingDurableSink {
        operations: Arc<Mutex<Vec<Operation>>>,
        fail_at: usize,
        attempts: AtomicUsize,
    }

    #[async_trait]
    impl DurableEventSink for FailingDurableSink {
        async fn append(&self, event: DurableAgentEvent) -> Result<(), DurableEventSinkError> {
            self.operations
                .lock()
                .expect("operation lock should not be poisoned")
                .push(Operation::Durable(event));
            let attempt = self.attempts.fetch_add(1, Ordering::Relaxed);
            if attempt == self.fail_at {
                Err(DurableEventSinkError::UnsupportedEvent {
                    event_type: "test_failure",
                })
            } else {
                Ok(())
            }
        }
    }

    struct StaticApproval {
        operations: Arc<Mutex<Vec<Operation>>>,
        decision: ApprovalDecision,
    }

    #[async_trait]
    impl ToolApproval for StaticApproval {
        async fn request(
            &self,
            request: ToolApprovalRequest,
            _cancellation: &CancellationToken,
        ) -> Result<ApprovalDecision, ToolApprovalError> {
            self.operations
                .lock()
                .expect("operation lock should not be poisoned")
                .push(Operation::Approval(request));
            Ok(self.decision)
        }
    }

    fn tool_call(name: &str) -> ToolCall {
        ToolCall {
            call_id: CallId::new("call-1"),
            name: name.to_owned(),
            arguments: json!({"value": 1}),
        }
    }

    #[tokio::test]
    async fn missing_tool_returns_error_message_without_execution_start() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = ToolExecutor::new(
            Arc::new(RecordingRegistry {
                operations: Arc::clone(&operations),
                missing: true,
                approval: None,
                specs: Vec::new(),
                call_result: RegistryCallResult::Success(json!(null)),
            }),
            Arc::new(StaticApproval {
                operations: Arc::clone(&operations),
                decision: ApprovalDecision::Approve,
            }),
            Arc::new(RecordingDurableSink {
                operations: Arc::clone(&operations),
            }),
        );
        let call = tool_call("missing");

        let outcome = executor
            .execute(&call, &CancellationToken::new())
            .await
            .expect("missing tools are recoverable tool results");

        assert_eq!(
            outcome.message(),
            &ChatMessage::tool(
                call.call_id.clone(),
                json!({"error": "tool not found: missing"}),
            )
        );
        assert!(!outcome.reached_tool());
        assert_eq!(
            *operations
                .lock()
                .expect("operation lock should not be poisoned"),
            vec![Operation::Authorization(ToolName::new("missing"))]
        );
    }

    #[tokio::test]
    async fn rejected_approval_is_durable_and_does_not_call_tool() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = ToolExecutor::new(
            Arc::new(RecordingRegistry {
                operations: Arc::clone(&operations),
                missing: false,
                approval: Some((ToolKind::Write, DangerLevel::High)),
                specs: Vec::new(),
                call_result: RegistryCallResult::Success(json!({"ok": true})),
            }),
            Arc::new(StaticApproval {
                operations: Arc::clone(&operations),
                decision: ApprovalDecision::Reject,
            }),
            Arc::new(RecordingDurableSink {
                operations: Arc::clone(&operations),
            }),
        );
        let call = tool_call("dangerous");

        let outcome = executor
            .execute(&call, &CancellationToken::new())
            .await
            .expect("rejection is a recoverable tool result");

        assert_eq!(
            outcome.message(),
            &ChatMessage::tool(
                call.call_id.clone(),
                json!({"error": "tool approval rejected"}),
            )
        );
        assert!(!outcome.reached_tool());

        let operations = operations
            .lock()
            .expect("operation lock should not be poisoned");
        assert_eq!(operations.len(), 4);
        assert_eq!(
            operations[0],
            Operation::Authorization(ToolName::new("dangerous"))
        );
        let Operation::Durable(DurableAgentEvent::ToolApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            tool_kind,
            danger_level,
        }) = &operations[1]
        else {
            panic!("second operation should persist the approval request");
        };
        assert_eq!(call_id, &call.call_id);
        assert_eq!(tool_name, &ToolName::new("dangerous"));
        assert_eq!(arguments, &call.arguments);
        assert_eq!(*tool_kind, ToolKind::Write);
        assert_eq!(*danger_level, DangerLevel::High);
        assert_eq!(
            operations[2],
            Operation::Approval(ToolApprovalRequest {
                approval_id: *approval_id,
                call_id: call.call_id.clone(),
                tool_name: ToolName::new("dangerous"),
                arguments: call.arguments.clone(),
                tool_kind: ToolKind::Write,
                danger_level: DangerLevel::High,
            })
        );
        assert_eq!(
            operations[3],
            Operation::Durable(DurableAgentEvent::ToolApprovalResolved {
                approval_id: *approval_id,
                decision: ApprovalDecision::Reject,
            })
        );
    }

    #[tokio::test]
    async fn approved_call_is_started_durably_before_tool_invocation() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = ToolExecutor::new(
            Arc::new(RecordingRegistry {
                operations: Arc::clone(&operations),
                missing: false,
                approval: Some((ToolKind::Write, DangerLevel::Medium)),
                specs: Vec::new(),
                call_result: RegistryCallResult::Success(json!({"ok": true})),
            }),
            Arc::new(StaticApproval {
                operations: Arc::clone(&operations),
                decision: ApprovalDecision::Approve,
            }),
            Arc::new(RecordingDurableSink {
                operations: Arc::clone(&operations),
            }),
        );
        let call = tool_call("writer");

        let outcome = executor
            .execute(&call, &CancellationToken::new())
            .await
            .expect("approved tool should execute");

        assert_eq!(
            outcome.message(),
            &ChatMessage::tool(call.call_id.clone(), json!({"ok": true}))
        );
        assert!(outcome.reached_tool());

        let operations = operations
            .lock()
            .expect("operation lock should not be poisoned");
        assert_eq!(operations.len(), 6);
        assert_eq!(
            operations[0],
            Operation::Authorization(ToolName::new("writer"))
        );
        let Operation::Durable(DurableAgentEvent::ToolApprovalRequested {
            approval_id,
            call_id,
            tool_name,
            arguments,
            tool_kind,
            danger_level,
        }) = &operations[1]
        else {
            panic!("second operation should persist the approval request");
        };
        assert_eq!(call_id, &call.call_id);
        assert_eq!(tool_name, &ToolName::new("writer"));
        assert_eq!(arguments, &call.arguments);
        assert_eq!(*tool_kind, ToolKind::Write);
        assert_eq!(*danger_level, DangerLevel::Medium);
        assert_eq!(
            operations[2],
            Operation::Approval(ToolApprovalRequest {
                approval_id: *approval_id,
                call_id: call.call_id.clone(),
                tool_name: ToolName::new("writer"),
                arguments: call.arguments.clone(),
                tool_kind: ToolKind::Write,
                danger_level: DangerLevel::Medium,
            })
        );
        assert_eq!(
            operations[3],
            Operation::Durable(DurableAgentEvent::ToolApprovalResolved {
                approval_id: *approval_id,
                decision: ApprovalDecision::Approve,
            })
        );
        assert_eq!(
            operations[4],
            Operation::Durable(DurableAgentEvent::ToolExecutionStarted {
                call_id: call.call_id.clone(),
                tool_name: ToolName::new("writer"),
            })
        );
        assert_eq!(
            operations[5],
            Operation::ToolCall {
                name: ToolName::new("writer"),
                input: ToolInput::new(call.call_id.clone(), call.arguments.clone()),
                cancelled: false,
            }
        );
    }

    #[tokio::test]
    async fn tool_failure_becomes_a_model_visible_error_result() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = ToolExecutor::new(
            Arc::new(RecordingRegistry {
                operations: Arc::clone(&operations),
                missing: false,
                approval: None,
                specs: Vec::new(),
                call_result: RegistryCallResult::Failure,
            }),
            Arc::new(StaticApproval {
                operations: Arc::clone(&operations),
                decision: ApprovalDecision::Approve,
            }),
            Arc::new(RecordingDurableSink {
                operations: Arc::clone(&operations),
            }),
        );
        let call = tool_call("fallible");

        let outcome = executor
            .execute(&call, &CancellationToken::new())
            .await
            .expect("tool domain failures are recoverable results");

        assert!(outcome.reached_tool());
        assert_eq!(
            outcome.into_message(),
            ChatMessage::tool(
                call.call_id.clone(),
                json!({"error": "invalid argument value: test failure"}),
            )
        );
        assert_eq!(
            *operations
                .lock()
                .expect("operation lock should not be poisoned"),
            vec![
                Operation::Authorization(ToolName::new("fallible")),
                Operation::Durable(DurableAgentEvent::ToolExecutionStarted {
                    call_id: call.call_id.clone(),
                    tool_name: ToolName::new("fallible"),
                }),
                Operation::ToolCall {
                    name: ToolName::new("fallible"),
                    input: ToolInput::new(call.call_id, call.arguments),
                    cancelled: false,
                },
            ]
        );
    }

    #[tokio::test]
    async fn every_failed_pre_execution_ack_prevents_tool_invocation() {
        for fail_at in 0..3 {
            let operations = Arc::new(Mutex::new(Vec::new()));
            let executor = ToolExecutor::new(
                Arc::new(RecordingRegistry {
                    operations: Arc::clone(&operations),
                    missing: false,
                    approval: Some((ToolKind::Write, DangerLevel::High)),
                    specs: Vec::new(),
                    call_result: RegistryCallResult::Success(json!({"ok": true})),
                }),
                Arc::new(StaticApproval {
                    operations: Arc::clone(&operations),
                    decision: ApprovalDecision::Approve,
                }),
                Arc::new(FailingDurableSink {
                    operations: Arc::clone(&operations),
                    fail_at,
                    attempts: AtomicUsize::new(0),
                }),
            );
            let call = tool_call("dangerous");

            let error = executor
                .execute(&call, &CancellationToken::new())
                .await
                .expect_err("a failed required ack must stop execution");

            assert!(matches!(
                error,
                crate::ToolExecutorError::Durability {
                    source: DurableEventSinkError::UnsupportedEvent {
                        event_type: "test_failure"
                    }
                }
            ));
            let operations = operations
                .lock()
                .expect("operation lock should not be poisoned");
            assert_eq!(
                operations[0],
                Operation::Authorization(ToolName::new("dangerous"))
            );
            assert_eq!(
                operations
                    .iter()
                    .filter(|operation| matches!(operation, Operation::ToolCall { .. }))
                    .count(),
                0
            );
            let event_types = operations
                .iter()
                .filter_map(|operation| match operation {
                    Operation::Durable(event) => Some(event.event_type()),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(
                event_types,
                &[
                    "tool_approval_requested",
                    "tool_approval_resolved",
                    "tool_execution_started",
                ][..=fail_at]
            );
            assert_eq!(
                operations
                    .iter()
                    .filter(|operation| matches!(operation, Operation::Approval(_)))
                    .count(),
                usize::from(fail_at > 0)
            );
        }
    }

    #[tokio::test]
    async fn cancellation_token_reaches_tool_and_cancelled_outcome_is_preserved() {
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = ToolExecutor::new(
            Arc::new(RecordingRegistry {
                operations: Arc::clone(&operations),
                missing: false,
                approval: None,
                specs: Vec::new(),
                call_result: RegistryCallResult::Cancelled,
            }),
            Arc::new(StaticApproval {
                operations: Arc::clone(&operations),
                decision: ApprovalDecision::Approve,
            }),
            Arc::new(RecordingDurableSink {
                operations: Arc::clone(&operations),
            }),
        );
        let call = tool_call("cancellable");
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let outcome = executor
            .execute(&call, &cancellation)
            .await
            .expect("a tool-reported cancellation remains a tool result");

        assert!(outcome.reached_tool());
        assert_eq!(
            outcome.into_message(),
            ChatMessage::tool(
                call.call_id.clone(),
                json!({"error": "tool execution cancelled"}),
            )
        );
        assert_eq!(
            *operations
                .lock()
                .expect("operation lock should not be poisoned"),
            vec![
                Operation::Authorization(ToolName::new("cancellable")),
                Operation::Durable(DurableAgentEvent::ToolExecutionStarted {
                    call_id: call.call_id.clone(),
                    tool_name: ToolName::new("cancellable"),
                }),
                Operation::ToolCall {
                    name: ToolName::new("cancellable"),
                    input: ToolInput::new(call.call_id, call.arguments),
                    cancelled: true,
                },
            ]
        );
    }

    #[test]
    fn specs_are_returned_unchanged_from_registry() {
        let specs = vec![
            ToolSpec::builder()
                .name("alpha")
                .description("first tool")
                .input_schema(json!({"type": "object"}))
                .build(),
            ToolSpec::builder()
                .name("beta")
                .description("second tool")
                .input_schema(json!({"type": "string"}))
                .build(),
        ];
        let operations = Arc::new(Mutex::new(Vec::new()));
        let executor = ToolExecutor::new(
            Arc::new(RecordingRegistry {
                operations: Arc::clone(&operations),
                missing: false,
                approval: None,
                specs: specs.clone(),
                call_result: RegistryCallResult::Success(json!(null)),
            }),
            Arc::new(StaticApproval {
                operations: Arc::clone(&operations),
                decision: ApprovalDecision::Approve,
            }),
            Arc::new(RecordingDurableSink { operations }),
        );

        assert_eq!(executor.specs(), specs);
    }
}
