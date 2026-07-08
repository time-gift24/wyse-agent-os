//! NATS JetStream event stream bus implementation.

use async_nats::{HeaderMap, jetstream};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::StreamExt;
use wyse_core::{RunId, StreamEnvelope};

use super::{EventStream, EventStreamBus, EventStreamBusError, NatsEventStreamBusConfig};

#[derive(Clone)]
pub(crate) struct NatsEventStreamBus {
    client: async_nats::Client,
    jetstream: jetstream::Context,
    config: NatsEventStreamBusConfig,
}

impl NatsEventStreamBus {
    pub(crate) async fn new(config: NatsEventStreamBusConfig) -> Result<Self, EventStreamBusError> {
        let client = async_nats::connect(&config.url)
            .await
            .map_err(EventStreamBusError::nats)?;
        let jetstream = jetstream::new(client.clone());

        jetstream
            .get_or_create_stream(jetstream::stream::Config {
                name: config.stream_name.clone(),
                subjects: vec![format!("{}.>", config.subject_prefix)],
                storage: jetstream::stream::StorageType::File,
                num_replicas: config.replicas,
                ..Default::default()
            })
            .await
            .map_err(EventStreamBusError::nats)?;

        Ok(Self {
            client,
            jetstream,
            config,
        })
    }

    fn subject_for(&self, envelope: &StreamEnvelope) -> String {
        subject_for(&self.config.subject_prefix, envelope)
    }

    fn subscribe_subject(&self, run_id: RunId) -> String {
        subscribe_subject(&self.config.subject_prefix, run_id)
    }
}

#[async_trait]
impl EventStreamBus for NatsEventStreamBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        let subject = self.subject_for(&envelope);
        let message_id = message_id(&envelope);
        let payload = serde_json::to_vec(&envelope).map_err(EventStreamBusError::Serialize)?;
        let mut headers = HeaderMap::new();
        headers.append("Nats-Msg-Id", message_id);

        self.jetstream
            .publish_with_headers(subject, headers, Bytes::from(payload))
            .await
            .map_err(EventStreamBusError::nats)?
            .await
            .map_err(EventStreamBusError::nats)?;

        Ok(())
    }

    async fn subscribe_run(&self, run_id: RunId) -> Result<EventStream, EventStreamBusError> {
        let subject = self.subscribe_subject(run_id);
        let subscription = self
            .client
            .subscribe(subject)
            .await
            .map_err(EventStreamBusError::nats)?;

        Ok(Box::pin(subscription.map(|message| {
            serde_json::from_slice::<StreamEnvelope>(&message.payload)
                .map_err(EventStreamBusError::Deserialize)
        })))
    }
}

fn subject_for(prefix: &str, envelope: &StreamEnvelope) -> String {
    format!(
        "{}.{}.{}",
        prefix,
        envelope.run_id,
        envelope.event.event_type()
    )
}

fn subscribe_subject(prefix: &str, run_id: RunId) -> String {
    format!("{prefix}.{run_id}.>")
}

fn message_id(envelope: &StreamEnvelope) -> String {
    format!("{}:{}", envelope.run_id, envelope.seq)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::Utc;
    use wyse_core::{EventSource, RuntimeEvent};

    use super::*;

    fn envelope() -> StreamEnvelope {
        StreamEnvelope {
            run_id: RunId::new(),
            seq: 42,
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::RunStarted,
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn subject_for_uses_run_id_and_event_type() {
        let envelope = envelope();
        let subject = subject_for("wyse.events", &envelope);

        assert_eq!(
            subject,
            format!("wyse.events.{}.run_started", envelope.run_id)
        );
    }

    #[test]
    fn subscribe_subject_uses_run_wildcard() {
        let run_id = RunId::new();
        let subject = subscribe_subject("wyse.events", run_id);

        assert_eq!(subject, format!("wyse.events.{run_id}.>"));
    }

    #[test]
    fn message_id_uses_run_id_and_seq() {
        let envelope = envelope();

        assert_eq!(message_id(&envelope), format!("{}:42", envelope.run_id));
    }
}
