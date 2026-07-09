//! In-memory event stream bus for tests and local embedding.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use futures_util::stream;
use tokio::sync::Notify;
use wyse_core::{RunId, StreamEnvelope};

use super::{EventStream, EventStreamBus, EventStreamBusError};

const DEFAULT_CAPACITY: usize = 1024;

/// In-memory event stream bus backed by retained per-run event history.
#[derive(Debug, Clone)]
pub struct InMemoryEventStreamBus {
    runs: Arc<Mutex<BTreeMap<RunId, Arc<RunEvents>>>>,
}

#[derive(Debug)]
struct RunEvents {
    history: Mutex<Vec<StreamEnvelope>>,
    notify: Notify,
}

impl RunEvents {
    fn new() -> Self {
        Self {
            history: Mutex::new(Vec::new()),
            notify: Notify::new(),
        }
    }

    fn event_at(&self, index: usize) -> Option<StreamEnvelope> {
        self.history
            .lock()
            .expect("in-memory event history mutex should not be poisoned")
            .get(index)
            .cloned()
    }
}

impl InMemoryEventStreamBus {
    /// Creates an in-memory bus.
    #[must_use]
    pub fn new(_capacity: usize) -> Self {
        Self {
            runs: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    fn run_events(&self, run_id: RunId) -> Arc<RunEvents> {
        let mut runs = self
            .runs
            .lock()
            .expect("in-memory event bus mutex should not be poisoned");
        Arc::clone(
            runs.entry(run_id)
                .or_insert_with(|| Arc::new(RunEvents::new())),
        )
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
        let run_events = self.run_events(envelope.run_id);
        run_events
            .history
            .lock()
            .expect("in-memory event history mutex should not be poisoned")
            .push(envelope);
        run_events.notify.notify_waiters();
        Ok(())
    }

    /// Subscribes to events for one run, including already-published events.
    ///
    /// This in-memory implementation is infallible and currently always returns `Ok`.
    async fn subscribe_run(&self, run_id: RunId) -> Result<EventStream, EventStreamBusError> {
        let run_events = self.run_events(run_id);
        Ok(Box::pin(stream::unfold(
            (run_events, 0usize),
            |(run_events, mut next_index)| async move {
                loop {
                    if let Some(envelope) = run_events.event_at(next_index) {
                        next_index += 1;
                        return Some((Ok(envelope), (run_events, next_index)));
                    }
                    let notified = run_events.notify.notified();
                    if let Some(envelope) = run_events.event_at(next_index) {
                        drop(notified);
                        next_index += 1;
                        return Some((Ok(envelope), (run_events, next_index)));
                    }
                    notified.await;
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
    async fn subscriber_receives_events_published_before_subscription() {
        let bus = InMemoryEventStreamBus::default();
        let run_id = RunId::new();

        bus.publish(envelope(run_id, 1)).await.expect("publish");
        let mut stream = bus.subscribe_run(run_id).await.expect("subscribe");

        let received = stream
            .next()
            .await
            .expect("one event")
            .expect("event should deserialize");

        assert_eq!(received.run_id, run_id);
        assert_eq!(received.seq, 1);
    }

    #[tokio::test]
    async fn subscriber_receives_each_retained_and_live_event_once() {
        let bus = InMemoryEventStreamBus::default();
        let run_id = RunId::new();

        bus.publish(envelope(run_id, 1)).await.expect("publish");
        let mut stream = bus.subscribe_run(run_id).await.expect("subscribe");
        bus.publish(envelope(run_id, 2)).await.expect("publish");

        let first = stream
            .next()
            .await
            .expect("first event")
            .expect("event should deserialize");
        let second = stream
            .next()
            .await
            .expect("second event")
            .expect("event should deserialize");

        assert_eq!(first.seq, 1);
        assert_eq!(second.seq, 2);
    }

    #[tokio::test]
    async fn slow_subscriber_receives_more_events_than_channel_capacity() {
        let bus = InMemoryEventStreamBus::new(1);
        let run_id = RunId::new();
        let mut stream = bus.subscribe_run(run_id).await.expect("subscribe");

        bus.publish(envelope(run_id, 1)).await.expect("publish 1");
        bus.publish(envelope(run_id, 2)).await.expect("publish 2");
        bus.publish(envelope(run_id, 3)).await.expect("publish 3");

        for expected_seq in 1..=3 {
            let received = stream
                .next()
                .await
                .expect("event")
                .expect("event should deserialize");
            assert_eq!(received.seq, expected_seq);
        }
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
