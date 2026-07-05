//! Distributed runtime event stream bus.

use std::{future::Future, pin::Pin};

use futures_core::Stream;
use wyse_core::{RunId, StreamEnvelope};

use crate::EventStreamBusError;

/// Stream of runtime event envelopes.
pub type EventStream =
    Pin<Box<dyn Stream<Item = Result<StreamEnvelope, EventStreamBusError>> + Send + 'static>>;

/// Publishes and subscribes to runtime event streams.
pub trait EventStreamBus: Send + Sync {
    /// Publishes one complete stream envelope.
    fn publish(
        &self,
        envelope: StreamEnvelope,
    ) -> impl Future<Output = Result<(), EventStreamBusError>> + Send;

    /// Subscribes to live events for one run.
    fn subscribe_run(
        &self,
        run_id: RunId,
    ) -> impl Future<Output = Result<EventStream, EventStreamBusError>> + Send;
}
