//! Agent-loop event sink contracts.

use async_trait::async_trait;
use stratum_core::{AgentTelemetryEvent, DurableAgentEvent};

use super::DurableEventSinkError;

/// Persists ordered agent-loop events before the loop may advance.
#[async_trait]
pub trait DurableEventSink: Send + Sync {
    /// Appends one event and waits for its persistence acknowledgement.
    ///
    /// # Errors
    ///
    /// Returns an error when the durable consumer cannot accept the event.
    async fn append(&self, event: DurableAgentEvent) -> Result<(), DurableEventSinkError>;
}

/// Publishes best-effort agent-loop telemetry.
pub trait TelemetryEventSink: Send + Sync {
    /// Emits one telemetry event without affecting agent-loop control flow.
    fn emit(&self, event: AgentTelemetryEvent);
}
