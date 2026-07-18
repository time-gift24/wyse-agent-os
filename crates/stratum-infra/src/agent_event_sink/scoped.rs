//! Adapter from local agent-loop events to externally scoped stream envelopes.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
use serde_json::Value;
use stratum_core::{
    AgentEvent, AgentId, AgentTelemetryEvent, DurableAgentEvent, EventSource, LlmCallRole,
    LlmEvent, RunId, RuntimeEvent, StreamEnvelope, TurnId,
};
use tokio::sync::{mpsc, oneshot};
use tracing::warn;

use super::{DurableEventSink, DurableEventSinkError, TelemetryEventSink};
use crate::EventStreamBus;

const TELEMETRY_PUBLISH_TIMEOUT: Duration = Duration::from_millis(100);
const TELEMETRY_QUEUE_CAPACITY: usize = 256;
const DURABLE_QUEUE_CAPACITY: usize = 16;

struct QueuedTelemetry {
    sequence: u64,
    event_type: &'static str,
    envelope: StreamEnvelope,
}

struct QueuedDurable {
    sequence: u64,
    envelope: StreamEnvelope,
    acknowledgment: oneshot::Sender<Result<(), crate::EventStreamBusError>>,
}

struct WorkerReceivers {
    durable: mpsc::Receiver<QueuedDurable>,
    telemetry: mpsc::Receiver<QueuedTelemetry>,
}

#[derive(Default)]
struct EnqueueState {
    next_sequence: u64,
}

impl EnqueueState {
    fn take_sequence(&mut self) -> u64 {
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .expect("agent event sequence should not overflow");
        sequence
    }
}

/// Adds run, agent, and turn scope before publishing agent-loop events.
pub struct ScopedAgentEventSink {
    agent_id: AgentId,
    agent_name: String,
    run_id: RunId,
    turn_id: TurnId,
    event_bus: Arc<dyn EventStreamBus>,
    durable: mpsc::Sender<QueuedDurable>,
    telemetry: mpsc::Sender<QueuedTelemetry>,
    receivers: Mutex<Option<WorkerReceivers>>,
    enqueue: Mutex<EnqueueState>,
    telemetry_drop_reported: Arc<AtomicBool>,
}

impl ScopedAgentEventSink {
    /// Creates a sink bound to one agent turn.
    #[must_use]
    pub fn new(
        agent_id: AgentId,
        agent_name: impl Into<String>,
        run_id: RunId,
        turn_id: TurnId,
        event_bus: Arc<dyn EventStreamBus>,
    ) -> Self {
        let (durable, durable_receiver) = mpsc::channel(DURABLE_QUEUE_CAPACITY);
        let (telemetry, telemetry_receiver) = mpsc::channel(TELEMETRY_QUEUE_CAPACITY);
        let sink = Self {
            agent_id,
            agent_name: agent_name.into(),
            run_id,
            turn_id,
            event_bus,
            durable,
            telemetry,
            receivers: Mutex::new(Some(WorkerReceivers {
                durable: durable_receiver,
                telemetry: telemetry_receiver,
            })),
            enqueue: Mutex::new(EnqueueState::default()),
            telemetry_drop_reported: Arc::new(AtomicBool::new(false)),
        };
        sink.start_worker();
        sink
    }

    fn start_worker(&self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let Some(receivers) = self
            .receivers
            .lock()
            .expect("agent event receiver lock should not be poisoned")
            .take()
        else {
            return;
        };
        let event_bus = Arc::clone(&self.event_bus);
        let telemetry_drop_reported = Arc::clone(&self.telemetry_drop_reported);
        let scope = EventScope {
            agent_id: self.agent_id,
            run_id: self.run_id,
            turn_id: self.turn_id,
        };
        runtime.spawn(run_worker(
            receivers,
            event_bus,
            telemetry_drop_reported,
            scope,
        ));
    }

    fn durable_agent_event(
        &self,
        event: DurableAgentEvent,
    ) -> Result<AgentEvent, DurableEventSinkError> {
        let event_type = event.event_type();
        let event = match event {
            DurableAgentEvent::LoopStarted => AgentEvent::Started {
                turn_id: self.turn_id,
            },
            DurableAgentEvent::MessageAppended { message } => AgentEvent::Message {
                turn_id: self.turn_id,
                message,
            },
            DurableAgentEvent::ToolApprovalRequested {
                approval_id,
                call_id,
                tool_name,
                arguments,
                tool_kind,
                danger_level,
            } => AgentEvent::ToolApprovalRequested {
                approval_id,
                agent_name: self.agent_name.clone(),
                call_id,
                tool_name,
                arguments,
                tool_kind,
                danger_level,
            },
            DurableAgentEvent::ToolApprovalResolved {
                approval_id,
                decision,
            } => AgentEvent::ToolApprovalResolved {
                approval_id,
                decision,
            },
            DurableAgentEvent::ToolExecutionStarted { call_id, tool_name } => {
                AgentEvent::ToolExecutionStarted {
                    turn_id: self.turn_id,
                    call_id,
                    tool_name,
                }
            }
            DurableAgentEvent::IterationCompleted { iteration, usage } => {
                AgentEvent::IterationCompleted {
                    turn_id: self.turn_id,
                    iteration,
                    usage,
                }
            }
            DurableAgentEvent::LoopFinished {
                finish_reason,
                usage,
            } => AgentEvent::Finished {
                finish_reason,
                usage,
            },
            DurableAgentEvent::LoopFailed { error_text, usage } => {
                AgentEvent::Failed { error_text, usage }
            }
            DurableAgentEvent::LoopCancelled { usage } => AgentEvent::Cancelled { usage },
            _ => return Err(DurableEventSinkError::UnsupportedEvent { event_type }),
        };
        Ok(event)
    }

    fn telemetry_agent_event(&self, event: AgentTelemetryEvent) -> Option<AgentEvent> {
        let event_type = event.event_type();
        let event = match event {
            AgentTelemetryEvent::LlmStarted { llm_call_id } => AgentEvent::Llm {
                llm_call_id,
                event: LlmEvent::Started,
            },
            AgentTelemetryEvent::TextDelta { llm_call_id, delta } => AgentEvent::Llm {
                llm_call_id,
                event: LlmEvent::TextDelta {
                    role: LlmCallRole::Assistant,
                    delta,
                },
            },
            AgentTelemetryEvent::ReasoningDelta { llm_call_id, delta } => AgentEvent::Llm {
                llm_call_id,
                event: LlmEvent::ReasoningDelta { delta },
            },
            AgentTelemetryEvent::ToolCallDelta {
                llm_call_id,
                call_id,
                name,
                arguments_delta,
            } => AgentEvent::Llm {
                llm_call_id,
                event: LlmEvent::ToolCallDelta {
                    call_id,
                    name,
                    arguments_delta,
                },
            },
            AgentTelemetryEvent::LlmFinished {
                llm_call_id,
                finish_reason,
                usage,
            } => AgentEvent::Llm {
                llm_call_id,
                event: LlmEvent::Finished {
                    finish_reason,
                    usage,
                },
            },
            _ => {
                warn!(
                    agent_id = %self.agent_id,
                    run_id = %self.run_id,
                    turn_id = %self.turn_id,
                    event_type,
                    "ignored unsupported agent telemetry event"
                );
                return None;
            }
        };
        Some(event)
    }

    fn envelope(&self, event: AgentEvent) -> StreamEnvelope {
        let mut metadata = BTreeMap::new();
        metadata.insert(
            "agent_name".to_owned(),
            Value::String(self.agent_name.clone()),
        );
        metadata.insert(
            "turn_id".to_owned(),
            Value::String(self.turn_id.to_string()),
        );

        StreamEnvelope {
            business_seq: None,
            run_id: self.run_id,
            timestamp: SystemTime::now().into(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent {
                agent_id: self.agent_id,
                event,
            },
            metadata,
        }
    }
}

#[async_trait]
impl DurableEventSink for ScopedAgentEventSink {
    async fn append(&self, event: DurableAgentEvent) -> Result<(), DurableEventSinkError> {
        let event = self.durable_agent_event(event)?;
        self.start_worker();
        let (acknowledgment, result) = oneshot::channel();
        let permit = self
            .durable
            .reserve()
            .await
            .map_err(|_| DurableEventSinkError::PublisherUnavailable)?;
        {
            let mut enqueue = self
                .enqueue
                .lock()
                .expect("agent event enqueue lock should not be poisoned");
            permit.send(QueuedDurable {
                sequence: enqueue.take_sequence(),
                envelope: self.envelope(event),
                acknowledgment,
            });
        }
        result
            .await
            .map_err(|_| DurableEventSinkError::PublisherUnavailable)??;
        Ok(())
    }
}

impl TelemetryEventSink for ScopedAgentEventSink {
    fn emit(&self, event: AgentTelemetryEvent) {
        self.start_worker();
        let event_type = event.event_type();
        let Some(event) = self.telemetry_agent_event(event) else {
            return;
        };
        let result = {
            let mut enqueue = self
                .enqueue
                .lock()
                .expect("agent event enqueue lock should not be poisoned");
            self.telemetry.try_send(QueuedTelemetry {
                sequence: enqueue.take_sequence(),
                event_type,
                envelope: self.envelope(event),
            })
        };
        if result.is_err() {
            report_telemetry_drop(
                self.telemetry_drop_reported.as_ref(),
                EventScope {
                    agent_id: self.agent_id,
                    run_id: self.run_id,
                    turn_id: self.turn_id,
                },
                event_type,
            );
        }
    }
}

#[derive(Clone, Copy)]
struct EventScope {
    agent_id: AgentId,
    run_id: RunId,
    turn_id: TurnId,
}

async fn run_worker(
    mut receivers: WorkerReceivers,
    event_bus: Arc<dyn EventStreamBus>,
    telemetry_drop_reported: Arc<AtomicBool>,
    scope: EventScope,
) {
    let mut held_telemetry = None;
    let mut durable_fence = None;
    loop {
        if let Ok(durable) = receivers.durable.try_recv() {
            publish_durable(
                durable,
                &mut receivers.telemetry,
                &mut held_telemetry,
                &mut durable_fence,
                event_bus.as_ref(),
                telemetry_drop_reported.as_ref(),
                scope,
            )
            .await;
            continue;
        }
        if let Some(telemetry) = held_telemetry.take() {
            publish_telemetry(
                telemetry,
                durable_fence,
                event_bus.as_ref(),
                telemetry_drop_reported.as_ref(),
                scope,
            )
            .await;
            continue;
        }
        tokio::select! {
            biased;
            durable = receivers.durable.recv() => {
                let Some(durable) = durable else { break };
                publish_durable(
                    durable,
                    &mut receivers.telemetry,
                    &mut held_telemetry,
                    &mut durable_fence,
                    event_bus.as_ref(),
                    telemetry_drop_reported.as_ref(),
                    scope,
                ).await;
            }
            telemetry = receivers.telemetry.recv() => {
                let Some(telemetry) = telemetry else { break };
                publish_telemetry(
                    telemetry,
                    durable_fence,
                    event_bus.as_ref(),
                    telemetry_drop_reported.as_ref(),
                    scope,
                ).await;
            }
        }
    }
}

async fn publish_durable(
    durable: QueuedDurable,
    telemetry_receiver: &mut mpsc::Receiver<QueuedTelemetry>,
    held_telemetry: &mut Option<QueuedTelemetry>,
    durable_fence: &mut Option<u64>,
    event_bus: &dyn EventStreamBus,
    telemetry_drop_reported: &AtomicBool,
    scope: EventScope,
) {
    discard_telemetry_before(
        durable.sequence,
        telemetry_receiver,
        held_telemetry,
        telemetry_drop_reported,
        scope,
    );
    *durable_fence = Some(durable.sequence);
    let result = event_bus.publish(durable.envelope).await;
    let _ = durable.acknowledgment.send(result);
}

fn discard_telemetry_before(
    sequence: u64,
    telemetry_receiver: &mut mpsc::Receiver<QueuedTelemetry>,
    held_telemetry: &mut Option<QueuedTelemetry>,
    telemetry_drop_reported: &AtomicBool,
    scope: EventScope,
) {
    if held_telemetry
        .as_ref()
        .is_some_and(|telemetry| telemetry.sequence < sequence)
    {
        let dropped = held_telemetry
            .take()
            .expect("held telemetry should be present");
        report_telemetry_drop(telemetry_drop_reported, scope, dropped.event_type);
    }
    if held_telemetry.is_some() {
        return;
    }
    while let Ok(telemetry) = telemetry_receiver.try_recv() {
        if telemetry.sequence >= sequence {
            *held_telemetry = Some(telemetry);
            break;
        }
        report_telemetry_drop(telemetry_drop_reported, scope, telemetry.event_type);
    }
}

async fn publish_telemetry(
    telemetry: QueuedTelemetry,
    durable_fence: Option<u64>,
    event_bus: &dyn EventStreamBus,
    telemetry_drop_reported: &AtomicBool,
    scope: EventScope,
) {
    if durable_fence.is_some_and(|sequence| telemetry.sequence < sequence) {
        report_telemetry_drop(telemetry_drop_reported, scope, telemetry.event_type);
        return;
    }
    match tokio::time::timeout(
        TELEMETRY_PUBLISH_TIMEOUT,
        event_bus.publish(telemetry.envelope),
    )
    .await
    {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            if !telemetry_drop_reported.swap(true, Ordering::Relaxed) {
                warn!(
                    agent_id = %scope.agent_id,
                    run_id = %scope.run_id,
                    turn_id = %scope.turn_id,
                    event_type = telemetry.event_type,
                    error = %error,
                    "failed to publish agent telemetry event; further telemetry issues for this turn will be silent"
                );
            }
        }
        Err(error) => {
            if !telemetry_drop_reported.swap(true, Ordering::Relaxed) {
                warn!(
                    agent_id = %scope.agent_id,
                    run_id = %scope.run_id,
                    turn_id = %scope.turn_id,
                    event_type = telemetry.event_type,
                    error = %error,
                    "agent telemetry publish timed out; further telemetry issues for this turn will be silent"
                );
            }
        }
    }
}

fn report_telemetry_drop(
    telemetry_drop_reported: &AtomicBool,
    scope: EventScope,
    event_type: &'static str,
) {
    if !telemetry_drop_reported.swap(true, Ordering::Relaxed) {
        warn!(
            agent_id = %scope.agent_id,
            run_id = %scope.run_id,
            turn_id = %scope.turn_id,
            event_type,
            "dropped agent telemetry event; further drops for this turn will be silent"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{QueuedTelemetry, TELEMETRY_PUBLISH_TIMEOUT, TELEMETRY_QUEUE_CAPACITY};

    use std::{
        future::pending,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;
    use futures_util::stream;
    use serde_json::json;
    use stratum_core::{
        AgentEvent, AgentId, AgentTelemetryEvent, ApprovalDecision, ApprovalId, CallId,
        ChatMessage, DangerLevel, DurableAgentEvent, EventSource, LlmCallId, LlmCallRole, LlmEvent,
        ReplayStart, RunId, RuntimeEvent, StreamEnvelope, TokenUsage, ToolKind, ToolName, TurnId,
    };

    use crate::{
        DurableEventSink, DurableEventSinkError, EventStream, EventStreamBus, EventStreamBusError,
        ScopedAgentEventSink, TelemetryEventSink,
    };

    #[derive(Default)]
    struct RecordingEventStreamBus {
        published: Mutex<Vec<StreamEnvelope>>,
    }

    async fn wait_for_published(published: &Mutex<Vec<StreamEnvelope>>, expected: usize) {
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if published
                    .lock()
                    .expect("event stream bus lock should not be poisoned")
                    .len()
                    >= expected
                {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("telemetry worker should publish queued events");
    }

    impl RecordingEventStreamBus {
        fn take_published(&self) -> Vec<StreamEnvelope> {
            std::mem::take(
                &mut *self
                    .published
                    .lock()
                    .expect("recording event stream bus lock should not be poisoned"),
            )
        }
    }

    #[async_trait]
    impl EventStreamBus for RecordingEventStreamBus {
        async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
            self.published
                .lock()
                .expect("recording event stream bus lock should not be poisoned")
                .push(envelope);
            Ok(())
        }

        async fn subscribe_agent(
            &self,
            _agent_id: AgentId,
            _replay_start: ReplayStart,
        ) -> Result<EventStream, EventStreamBusError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[derive(Default)]
    struct FailingEventStreamBus {
        published: Mutex<Vec<StreamEnvelope>>,
    }

    struct NeverCompletingEventStreamBus;

    #[derive(Default)]
    struct BlockingOldTelemetryBus {
        published: Mutex<Vec<StreamEnvelope>>,
        telemetry_started: tokio::sync::Notify,
    }

    #[derive(Default)]
    struct DelayedFirstEventStreamBus {
        publish_count: AtomicUsize,
        published: Mutex<Vec<StreamEnvelope>>,
        first_publish_started: tokio::sync::Notify,
        release_first_publish: tokio::sync::Notify,
    }

    #[async_trait]
    impl EventStreamBus for NeverCompletingEventStreamBus {
        async fn publish(&self, _envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
            pending().await
        }

        async fn subscribe_agent(
            &self,
            _agent_id: AgentId,
            _replay_start: ReplayStart,
        ) -> Result<EventStream, EventStreamBusError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[async_trait]
    impl EventStreamBus for BlockingOldTelemetryBus {
        async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
            if matches!(
                &envelope.event,
                RuntimeEvent::Agent {
                    event: AgentEvent::Llm { llm_call_id, .. },
                    ..
                } if llm_call_id != &LlmCallId::from("new")
            ) {
                self.telemetry_started.notify_one();
                pending().await
            }
            self.published
                .lock()
                .expect("blocking telemetry bus lock should not be poisoned")
                .push(envelope);
            Ok(())
        }

        async fn subscribe_agent(
            &self,
            _agent_id: AgentId,
            _replay_start: ReplayStart,
        ) -> Result<EventStream, EventStreamBusError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[async_trait]
    impl EventStreamBus for DelayedFirstEventStreamBus {
        async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
            if self.publish_count.fetch_add(1, Ordering::SeqCst) == 0 {
                self.first_publish_started.notify_one();
                self.release_first_publish.notified().await;
            }
            self.published
                .lock()
                .expect("delayed event stream bus lock should not be poisoned")
                .push(envelope);
            Ok(())
        }

        async fn subscribe_agent(
            &self,
            _agent_id: AgentId,
            _replay_start: ReplayStart,
        ) -> Result<EventStream, EventStreamBusError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    impl FailingEventStreamBus {
        fn take_published(&self) -> Vec<StreamEnvelope> {
            std::mem::take(
                &mut *self
                    .published
                    .lock()
                    .expect("failing event stream bus lock should not be poisoned"),
            )
        }
    }

    #[async_trait]
    impl EventStreamBus for FailingEventStreamBus {
        async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
            self.published
                .lock()
                .expect("failing event stream bus lock should not be poisoned")
                .push(envelope);
            Err(EventStreamBusError::MissingAgentScope)
        }

        async fn subscribe_agent(
            &self,
            _agent_id: AgentId,
            _replay_start: ReplayStart,
        ) -> Result<EventStream, EventStreamBusError> {
            Ok(Box::pin(stream::empty()))
        }
    }

    #[tokio::test]
    async fn durable_event_is_scoped_and_returns_bus_error() {
        let agent_id = AgentId::new();
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let recorder = Arc::new(FailingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            agent_id,
            "review-agent",
            run_id,
            turn_id,
            Arc::clone(&event_bus),
        );

        let error = sink
            .append(DurableAgentEvent::LoopStarted)
            .await
            .expect_err("durable publish failure must reach the caller");

        assert!(matches!(
            error,
            DurableEventSinkError::EventStreamBus(EventStreamBusError::MissingAgentScope)
        ));
        let [envelope] = recorder
            .take_published()
            .try_into()
            .expect("exactly one envelope should be published");
        assert_eq!(envelope.run_id, run_id);
        assert_eq!(envelope.source, EventSource::Run);
        assert_eq!(
            envelope.metadata.get("agent_name"),
            Some(&json!("review-agent"))
        );
        assert_eq!(
            envelope.event,
            RuntimeEvent::Agent {
                agent_id,
                event: AgentEvent::Started { turn_id },
            }
        );
    }

    #[tokio::test]
    async fn telemetry_event_is_published_without_exposing_bus_error() {
        let agent_id = AgentId::new();
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let recorder = Arc::new(FailingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            agent_id,
            "review-agent",
            run_id,
            turn_id,
            Arc::clone(&event_bus),
        );
        let llm_call_id = LlmCallId::from("llm-call-1");

        sink.emit(AgentTelemetryEvent::TextDelta {
            llm_call_id: llm_call_id.clone(),
            delta: "hello".to_owned(),
        });
        wait_for_published(&recorder.published, 1).await;

        let [envelope] = recorder
            .take_published()
            .try_into()
            .expect("exactly one envelope should be published");
        assert_eq!(envelope.run_id, run_id);
        assert_eq!(
            envelope.event,
            RuntimeEvent::Agent {
                agent_id,
                event: AgentEvent::Llm {
                    llm_call_id,
                    event: LlmEvent::TextDelta {
                        role: LlmCallRole::Assistant,
                        delta: "hello".to_owned(),
                    },
                },
            }
        );
    }

    #[tokio::test]
    async fn every_turn_scoped_envelope_includes_stable_turn_metadata() {
        let agent_id = AgentId::new();
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let recorder = Arc::new(RecordingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            agent_id,
            "review-agent",
            run_id,
            turn_id,
            Arc::clone(&event_bus),
        );
        let approval_id = ApprovalId::new();

        sink.append(DurableAgentEvent::LoopFinished {
            finish_reason: "stop".to_owned(),
            usage: TokenUsage::default(),
        })
        .await
        .expect("terminal event should publish");
        sink.append(DurableAgentEvent::ToolApprovalRequested {
            approval_id,
            call_id: CallId::from("tool-call-1"),
            tool_name: ToolName::from("write_file"),
            arguments: json!({"path": "notes.txt"}),
            tool_kind: ToolKind::Write,
            danger_level: DangerLevel::High,
        })
        .await
        .expect("approval request should publish");
        sink.append(DurableAgentEvent::ToolApprovalResolved {
            approval_id,
            decision: ApprovalDecision::Approve,
        })
        .await
        .expect("approval resolution should publish");
        sink.emit(AgentTelemetryEvent::LlmStarted {
            llm_call_id: LlmCallId::from("llm-call-1"),
        });
        wait_for_published(&recorder.published, 4).await;

        let envelopes = recorder.take_published();
        assert_eq!(envelopes.len(), 4);
        for envelope in &envelopes {
            assert_eq!(
                envelope.metadata.get("agent_name"),
                Some(&json!("review-agent"))
            );
            assert_eq!(
                envelope.metadata.get("turn_id"),
                Some(&json!(turn_id.to_string()))
            );
        }
        assert!(matches!(
            envelopes[0].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Finished { .. },
                ..
            }
        ));
        assert!(matches!(
            envelopes[1].event,
            RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalRequested { .. },
                ..
            }
        ));
        assert!(matches!(
            envelopes[2].event,
            RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalResolved { .. },
                ..
            }
        ));
        assert!(matches!(
            envelopes[3].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Llm { .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn telemetry_emit_is_non_blocking_when_bus_publish_never_completes() {
        let event_bus: Arc<dyn EventStreamBus> = Arc::new(NeverCompletingEventStreamBus);
        let sink = ScopedAgentEventSink::new(
            AgentId::new(),
            "review-agent",
            RunId::new(),
            TurnId::new(),
            event_bus,
        );

        for index in 0..=TELEMETRY_QUEUE_CAPACITY {
            sink.emit(AgentTelemetryEvent::LlmStarted {
                llm_call_id: LlmCallId::from(format!("llm-call-{index}")),
            });
        }
    }

    #[tokio::test]
    async fn durable_events_wait_for_preceding_telemetry_publish() {
        let recorder = Arc::new(DelayedFirstEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = Arc::new(ScopedAgentEventSink::new(
            AgentId::new(),
            "review-agent",
            RunId::new(),
            TurnId::new(),
            event_bus,
        ));

        sink.emit(AgentTelemetryEvent::LlmStarted {
            llm_call_id: LlmCallId::from("llm-call-1"),
        });
        recorder.first_publish_started.notified().await;
        let durable_sink = Arc::clone(&sink);
        let append = tokio::spawn(async move {
            durable_sink
                .append(DurableAgentEvent::MessageAppended {
                    message: ChatMessage::assistant("done"),
                })
                .await
        });
        tokio::task::yield_now().await;
        assert!(!append.is_finished());

        recorder.release_first_publish.notify_one();
        append
            .await
            .expect("append task should finish")
            .expect("durable event should publish");

        let events = recorder
            .published
            .lock()
            .expect("delayed event stream bus lock should not be poisoned");
        assert!(matches!(
            events[0].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Llm { .. },
                ..
            }
        ));
        assert!(matches!(
            events[1].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Message { .. },
                ..
            }
        ));
    }

    #[tokio::test]
    async fn durable_lane_discards_telemetry_backlog_after_current_publish_timeout() {
        let recorder = Arc::new(BlockingOldTelemetryBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            AgentId::new(),
            "review-agent",
            RunId::new(),
            TurnId::new(),
            event_bus,
        );

        sink.emit(AgentTelemetryEvent::LlmStarted {
            llm_call_id: LlmCallId::from("old-in-flight"),
        });
        recorder.telemetry_started.notified().await;
        for index in 0..TELEMETRY_QUEUE_CAPACITY {
            sink.emit(AgentTelemetryEvent::TextDelta {
                llm_call_id: LlmCallId::from(format!("old-{index}")),
                delta: "x".to_owned(),
            });
        }

        tokio::time::timeout(
            TELEMETRY_PUBLISH_TIMEOUT * 3,
            sink.append(DurableAgentEvent::MessageAppended {
                message: ChatMessage::assistant("done"),
            }),
        )
        .await
        .expect("durable publish should wait for at most the in-flight telemetry timeout")
        .expect("durable event should publish");
        sink.emit(AgentTelemetryEvent::LlmStarted {
            llm_call_id: LlmCallId::from("new"),
        });
        wait_for_published(&recorder.published, 2).await;

        let published = recorder
            .published
            .lock()
            .expect("blocking telemetry bus lock should not be poisoned");
        assert!(matches!(
            published[0].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Message { .. },
                ..
            }
        ));
        assert!(matches!(
            &published[1].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Llm { llm_call_id, .. },
                ..
            } if llm_call_id == &LlmCallId::from("new")
        ));
    }

    #[tokio::test]
    async fn telemetry_enqueued_late_with_an_old_sequence_is_dropped_after_durable() {
        let recorder = Arc::new(RecordingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            AgentId::new(),
            "review-agent",
            RunId::new(),
            TurnId::new(),
            event_bus,
        );
        let old_sequence = sink
            .enqueue
            .lock()
            .expect("agent event enqueue lock should not be poisoned")
            .take_sequence();

        sink.append(DurableAgentEvent::LoopStarted)
            .await
            .expect("durable event should publish");
        let old_event = AgentTelemetryEvent::LlmStarted {
            llm_call_id: LlmCallId::from("old"),
        };
        let old_event_type = old_event.event_type();
        let old_event = sink
            .telemetry_agent_event(old_event)
            .expect("supported telemetry should map");
        sink.telemetry
            .try_send(QueuedTelemetry {
                sequence: old_sequence,
                event_type: old_event_type,
                envelope: sink.envelope(old_event),
            })
            .expect("test queue should have capacity");
        sink.emit(AgentTelemetryEvent::LlmStarted {
            llm_call_id: LlmCallId::from("new"),
        });
        wait_for_published(&recorder.published, 2).await;

        let published = recorder.take_published();
        assert_eq!(published.len(), 2);
        assert!(matches!(
            published[0].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Started { .. },
                ..
            }
        ));
        assert!(matches!(
            &published[1].event,
            RuntimeEvent::Agent {
                event: AgentEvent::Llm { llm_call_id, .. },
                ..
            } if llm_call_id == &LlmCallId::from("new")
        ));
        assert!(sink.telemetry_drop_reported.load(Ordering::Relaxed));
    }

    #[test]
    fn constructing_outside_runtime_starts_worker_on_first_emit() {
        let recorder = Arc::new(RecordingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            AgentId::new(),
            "review-agent",
            RunId::new(),
            TurnId::new(),
            event_bus,
        );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime should build");

        runtime.block_on(async {
            sink.emit(AgentTelemetryEvent::LlmStarted {
                llm_call_id: LlmCallId::from("llm-call-1"),
            });
            wait_for_published(&recorder.published, 1).await;
        });
    }

    #[tokio::test]
    async fn durable_message_maps_to_external_message_event() {
        let agent_id = AgentId::new();
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let recorder = Arc::new(RecordingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            agent_id,
            "review-agent",
            run_id,
            turn_id,
            Arc::clone(&event_bus),
        );
        let message = ChatMessage::assistant("done");

        sink.append(DurableAgentEvent::MessageAppended {
            message: message.clone(),
        })
        .await
        .expect("recording event stream bus should accept the message");

        let [envelope] = recorder
            .take_published()
            .try_into()
            .expect("exactly one envelope should be published");
        assert_eq!(
            envelope.event,
            RuntimeEvent::Agent {
                agent_id,
                event: AgentEvent::Message { turn_id, message },
            }
        );
    }

    #[tokio::test]
    async fn durable_tool_start_and_iteration_map_to_external_events() {
        let agent_id = AgentId::new();
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        let recorder = Arc::new(RecordingEventStreamBus::default());
        let event_bus: Arc<dyn EventStreamBus> = recorder.clone();
        let sink = ScopedAgentEventSink::new(
            agent_id,
            "review-agent",
            run_id,
            turn_id,
            Arc::clone(&event_bus),
        );
        let usage = TokenUsage {
            input_tokens: 1,
            output_tokens: 2,
            total_tokens: 3,
        };

        sink.append(DurableAgentEvent::ToolExecutionStarted {
            call_id: CallId::from("tool-call-1"),
            tool_name: ToolName::from("echo"),
        })
        .await
        .expect("tool start should publish");
        sink.append(DurableAgentEvent::IterationCompleted {
            iteration: 4,
            usage,
        })
        .await
        .expect("iteration completion should publish");

        let [started, completed] = recorder
            .take_published()
            .try_into()
            .expect("exactly two envelopes should be published");
        let RuntimeEvent::Agent {
            event: started_event,
            ..
        } = &started.event
        else {
            panic!("tool execution start should be an agent event");
        };
        let RuntimeEvent::Agent {
            event: completed_event,
            ..
        } = &completed.event
        else {
            panic!("iteration completion should be an agent event");
        };
        assert_eq!(started_event.event_type(), "tool_execution_started");
        assert_eq!(completed_event.event_type(), "iteration_completed");
        assert_eq!(
            started.event,
            RuntimeEvent::Agent {
                agent_id,
                event: AgentEvent::ToolExecutionStarted {
                    turn_id,
                    call_id: CallId::from("tool-call-1"),
                    tool_name: ToolName::from("echo"),
                },
            }
        );
        assert_eq!(
            completed.event,
            RuntimeEvent::Agent {
                agent_id,
                event: AgentEvent::IterationCompleted {
                    turn_id,
                    iteration: 4,
                    usage,
                },
            }
        );
    }
}
