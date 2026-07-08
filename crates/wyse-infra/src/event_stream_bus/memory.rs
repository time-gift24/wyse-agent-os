//! In-memory event stream bus for tests and local embedding.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures_util::stream;
use tokio::sync::broadcast;
use wyse_core::{RunId, StreamEnvelope};

use super::{EventStream, EventStreamBus, EventStreamBusError};

const DEFAULT_CAPACITY: usize = 1024;

/// In-memory event stream bus backed by Tokio broadcast channels.
#[derive(Debug, Clone)]
pub struct InMemoryEventStreamBus {
    capacity: usize,
    runs: Arc<Mutex<BTreeMap<RunId, broadcast::Sender<StreamEnvelope>>>>,
}

impl InMemoryEventStreamBus {
    /// Creates an in-memory bus with a bounded per-run channel capacity.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            runs: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn sender(&self, run_id: RunId) -> broadcast::Sender<StreamEnvelope> {
        let mut runs = self
            .runs
            .lock()
            .expect("in-memory event bus mutex should not be poisoned");

        runs.entry(run_id)
            .or_insert_with(|| {
                let (sender, _) = broadcast::channel(self.capacity.max(1));
                sender
            })
            .clone()
    }
}

impl Default for InMemoryEventStreamBus {
    fn default() -> Self {
        Self::new(DEFAULT_CAPACITY)
    }
}

#[async_trait]
impl EventStreamBus for InMemoryEventStreamBus {
    /// Publishes one complete stream envelope.
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        let sender = self.sender(envelope.run_id);
        let _ = sender.send(envelope);
        Ok(())
    }

    /// Subscribes to live events for one run.
    ///
    /// # Errors
    ///
    /// Returns an error only if the underlying stream construction fails.
    async fn subscribe_run(&self, run_id: RunId) -> Result<EventStream, EventStreamBusError> {
        let receiver = self.sender(run_id).subscribe();

        Ok(Box::pin(stream::unfold(
            receiver,
            |mut receiver| async move {
                loop {
                    match receiver.recv().await {
                        Ok(envelope) => return Some((Ok(envelope), receiver)),
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return None,
                    }
                }
            },
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use futures_util::StreamExt;
    use wyse_core::{EventSource, RuntimeEvent};

    use super::*;
    use crate::event_stream_bus::EventStreamBus;

    fn envelope(run_id: RunId, seq: u64) -> StreamEnvelope {
        StreamEnvelope {
            run_id,
            seq,
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::RunStarted,
            metadata: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn subscriber_receives_published_run_event() {
        let bus = InMemoryEventStreamBus::default();
        let run_id = RunId::new();
        let mut stream = bus.subscribe_run(run_id).await.expect("subscribe");

        bus.publish(envelope(run_id, 1)).await.expect("publish");

        let received = stream
            .next()
            .await
            .expect("one event")
            .expect("event should deserialize");

        assert_eq!(received.run_id, run_id);
        assert_eq!(received.seq, 1);
    }

    #[tokio::test]
    async fn subscriber_ignores_other_runs() {
        let bus = InMemoryEventStreamBus::default();
        let run_id = RunId::new();
        let other_run_id = RunId::new();
        let mut stream = bus.subscribe_run(run_id).await.expect("subscribe");

        bus.publish(envelope(other_run_id, 1))
            .await
            .expect("publish other");
        bus.publish(envelope(run_id, 2))
            .await
            .expect("publish target");

        let received = stream
            .next()
            .await
            .expect("one event")
            .expect("event should deserialize");

        assert_eq!(received.run_id, run_id);
        assert_eq!(received.seq, 2);
    }
}
