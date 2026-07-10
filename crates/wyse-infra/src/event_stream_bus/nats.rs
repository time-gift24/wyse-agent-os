//! NATS JetStream event stream bus implementation.

use async_nats::jetstream::{
    self,
    consumer::{DeliverPolicy, push::OrderedConfig},
    stream::{DiscardPolicy, RetentionPolicy, StorageType},
};
use async_trait::async_trait;
use bytes::Bytes;
use futures_util::{StreamExt, future, stream};
use wyse_core::{AgentId, EventCursor, EventRecord, ReplayStart, RuntimeEvent, StreamEnvelope};

use super::{EventStream, EventStreamBus, EventStreamBusError, NatsEventStreamBusConfig};

#[derive(Clone)]
pub(crate) struct NatsEventStreamBus {
    jetstream: jetstream::Context,
    config: NatsEventStreamBusConfig,
}

impl NatsEventStreamBus {
    pub(crate) async fn new(config: NatsEventStreamBusConfig) -> Result<Self, EventStreamBusError> {
        validate_config(&config)?;
        let client = async_nats::ConnectOptions::new()
            .custom_inbox_prefix("_INBOX.agent_events")
            .connect(&config.url)
            .await
            .map_err(EventStreamBusError::nats)?;
        let jetstream = jetstream::new(client);

        jetstream
            .create_or_update_stream(stream_config(&config))
            .await
            .map_err(EventStreamBusError::nats)?;

        Ok(Self { jetstream, config })
    }

    fn subject_for(&self, envelope: &StreamEnvelope) -> Result<String, EventStreamBusError> {
        subject_for(&self.config.subject_prefix, envelope)
    }

    fn subscribe_subject(&self, agent_id: AgentId) -> String {
        subscribe_subject(&self.config.subject_prefix, agent_id)
    }

    async fn validate_cursor(&self, cursor: EventCursor) -> Result<(), EventStreamBusError> {
        let stream = self
            .jetstream
            .get_stream(&self.config.stream_name)
            .await
            .map_err(EventStreamBusError::nats)?;
        let earliest_valid_cursor = stream.cached_info().state.first_sequence.saturating_sub(1);

        if cursor.transport_sequence() < earliest_valid_cursor {
            Err(EventStreamBusError::CursorExpired { cursor })
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl EventStreamBus for NatsEventStreamBus {
    async fn publish(&self, envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        let run_id = envelope.run_id;
        let event_type = envelope.event.event_type();
        let agent_id = match &envelope.event {
            RuntimeEvent::Agent { agent_id, .. } => Some(*agent_id),
            _ => None,
        };
        let result = async {
            let subject = self.subject_for(&envelope)?;
            let payload = serde_json::to_vec(&envelope).map_err(EventStreamBusError::Serialize)?;

            self.jetstream
                .publish(subject, Bytes::from(payload))
                .await
                .map_err(EventStreamBusError::nats)?
                .await
                .map_err(EventStreamBusError::nats)?;

            Ok(())
        }
        .await;

        if result.is_err() {
            if let Some(agent_id) = agent_id {
                tracing::warn!(
                    agent_id = %agent_id,
                    run_id = %run_id,
                    event_type,
                    "agent event publish failed"
                );
            } else {
                tracing::warn!(
                    run_id = %run_id,
                    event_type,
                    "event publish failed before agent routing"
                );
            }
        }

        result
    }

    async fn subscribe_agent(
        &self,
        agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        let result = async {
            if let ReplayStart::After(cursor) = replay_start {
                self.validate_cursor(cursor).await?;
            }
            let deliver_subject = self.jetstream.client().new_inbox();
            let consumer = self
                .jetstream
                .create_consumer_on_stream(
                    OrderedConfig {
                        deliver_subject,
                        filter_subject: self.subscribe_subject(agent_id),
                        deliver_policy: deliver_policy(replay_start)?,
                        ..Default::default()
                    },
                    &self.config.stream_name,
                )
                .await
                .map_err(EventStreamBusError::nats)?;
            let messages = consumer
                .messages()
                .await
                .map_err(EventStreamBusError::nats)?;

            let deliveries = messages.flat_map(|message| stream::iter([Some(message), None]));
            let events = deliveries
                .scan(false, move |terminated, message| {
                    let Some(message) = message else {
                        return future::ready(if *terminated { None } else { Some(None) });
                    };

                    if *terminated {
                        return future::ready(None);
                    }

                    let mut transport_cursor = None;
                    let result = message
                        .map_err(EventStreamBusError::nats)
                        .and_then(|message| {
                            let cursor = EventCursor::from_transport_sequence(
                                message
                                    .info()
                                    .map_err(|source| EventStreamBusError::Nats { source })?
                                    .stream_sequence,
                            );
                            transport_cursor = Some(cursor.transport_sequence());
                            let envelope =
                                serde_json::from_slice::<StreamEnvelope>(&message.message.payload)
                                    .map_err(EventStreamBusError::Deserialize)?;
                            Ok(EventRecord { cursor, envelope })
                        });

                    if result.is_err() {
                        *terminated = true;
                        if let Some(transport_cursor) = transport_cursor {
                            tracing::warn!(
                                agent_id = %agent_id,
                                transport_cursor,
                                "agent event delivery or decode failed"
                            );
                        } else {
                            tracing::warn!(
                                agent_id = %agent_id,
                                "agent event delivery failed before cursor extraction"
                            );
                        }
                    }

                    future::ready(Some(Some(result)))
                })
                .filter_map(future::ready);

            Ok(Box::pin(events) as EventStream)
        }
        .await;

        if let Err(error) = &result {
            if let EventStreamBusError::CursorExpired { cursor } = error {
                tracing::warn!(
                    agent_id = %agent_id,
                    transport_cursor = cursor.transport_sequence(),
                    "agent event cursor reset required"
                );
            } else if let ReplayStart::After(cursor) = replay_start {
                tracing::warn!(
                    agent_id = %agent_id,
                    transport_cursor = cursor.transport_sequence(),
                    "agent event subscription failed"
                );
            } else {
                tracing::warn!(agent_id = %agent_id, "agent event subscription failed");
            }
        }

        result
    }
}

fn subject_for(prefix: &str, envelope: &StreamEnvelope) -> Result<String, EventStreamBusError> {
    let RuntimeEvent::Agent { agent_id, event } = &envelope.event else {
        return Err(EventStreamBusError::MissingAgentScope);
    };
    Ok(format!("{prefix}.{agent_id}.{}", event.event_type()))
}

fn subscribe_subject(prefix: &str, agent_id: AgentId) -> String {
    format!("{prefix}.{agent_id}.>")
}

fn validate_config(config: &NatsEventStreamBusConfig) -> Result<(), EventStreamBusError> {
    let reason = if config.max_age.is_zero() {
        Some("max_age must be greater than zero")
    } else if config.max_bytes <= 0 {
        Some("max_bytes must be greater than zero")
    } else if config.max_messages <= 0 {
        Some("max_messages must be greater than zero")
    } else if !(1..=5).contains(&config.replicas) {
        Some("replicas must be between 1 and 5")
    } else {
        None
    };

    reason.map_or(Ok(()), |reason| {
        Err(EventStreamBusError::InvalidConfig { reason })
    })
}

fn stream_config(config: &NatsEventStreamBusConfig) -> jetstream::stream::Config {
    jetstream::stream::Config {
        name: config.stream_name.clone(),
        subjects: vec![format!("{}.>", config.subject_prefix)],
        storage: StorageType::File,
        retention: RetentionPolicy::Limits,
        discard: DiscardPolicy::Old,
        max_age: config.max_age,
        max_bytes: config.max_bytes,
        max_messages: config.max_messages,
        num_replicas: config.replicas,
        ..Default::default()
    }
}

fn deliver_policy(replay_start: ReplayStart) -> Result<DeliverPolicy, EventStreamBusError> {
    match replay_start {
        ReplayStart::All => Ok(DeliverPolicy::All),
        ReplayStart::New => Ok(DeliverPolicy::New),
        ReplayStart::After(cursor) => cursor
            .transport_sequence()
            .checked_add(1)
            .map(|start_sequence| DeliverPolicy::ByStartSequence { start_sequence })
            .ok_or(EventStreamBusError::CursorOverflow),
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, time::Duration};

    use async_nats::jetstream::stream::{DiscardPolicy, RetentionPolicy, StorageType};
    use chrono::Utc;
    use wyse_core::{
        AgentEvent, AgentId, EventCursor, EventSource, ReplayStart, RunId, RuntimeEvent,
    };

    use super::*;

    fn agent_envelope(agent_id: AgentId, event: AgentEvent) -> StreamEnvelope {
        StreamEnvelope {
            run_id: RunId::new(),
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent { agent_id, event },
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn agent_subject_has_no_product_prefix() {
        let agent_id = AgentId::new();
        let envelope = agent_envelope(agent_id, AgentEvent::Started);

        assert_eq!(
            subject_for("events.agent", &envelope).expect("agent subject"),
            format!("events.agent.{agent_id}.started")
        );
    }

    #[test]
    fn subscribe_subject_uses_agent_wildcard() {
        let agent_id = AgentId::new();
        let subject = subscribe_subject("events.agent", agent_id);

        assert_eq!(subject, format!("events.agent.{agent_id}.>"));
    }

    #[test]
    fn replay_all_and_new_map_to_jetstream_delivery_policy() {
        assert_eq!(
            deliver_policy(ReplayStart::All).expect("all"),
            DeliverPolicy::All
        );
        assert_eq!(
            deliver_policy(ReplayStart::New).expect("new"),
            DeliverPolicy::New
        );
    }

    #[test]
    fn replay_after_starts_at_next_transport_sequence() {
        let policy = deliver_policy(ReplayStart::After(EventCursor::from_transport_sequence(41)))
            .expect("valid cursor");

        assert_eq!(
            policy,
            DeliverPolicy::ByStartSequence { start_sequence: 42 }
        );
    }

    #[test]
    fn replay_after_rejects_transport_sequence_overflow() {
        let result = deliver_policy(ReplayStart::After(EventCursor::from_transport_sequence(
            u64::MAX,
        )));

        assert!(matches!(result, Err(EventStreamBusError::CursorOverflow)));
    }

    #[test]
    fn default_config_has_explicit_file_retention_limits() {
        let config = NatsEventStreamBusConfig::default();

        assert_eq!(config.url, "nats://localhost:4222");
        assert_eq!(config.stream_name, "AGENT_EVENTS");
        assert_eq!(config.subject_prefix, "events.agent");
        assert_eq!(config.replicas, 1);
        assert_eq!(config.max_age, Duration::from_secs(7 * 24 * 60 * 60));
        assert_eq!(config.max_bytes, 1_073_741_824);
        assert_eq!(config.max_messages, 1_000_000);

        let stream_config = stream_config(&config);
        assert_eq!(stream_config.storage, StorageType::File);
        assert_eq!(stream_config.retention, RetentionPolicy::Limits);
        assert_eq!(stream_config.discard, DiscardPolicy::Old);
        assert_eq!(stream_config.max_age, config.max_age);
        assert_eq!(stream_config.max_bytes, config.max_bytes);
        assert_eq!(stream_config.max_messages, config.max_messages);
        assert_eq!(stream_config.num_replicas, config.replicas);
    }

    #[test]
    fn config_rejects_invalid_retention_limits_and_replicas() {
        let invalid_configs = [
            NatsEventStreamBusConfig {
                max_age: Duration::ZERO,
                ..Default::default()
            },
            NatsEventStreamBusConfig {
                max_bytes: 0,
                ..Default::default()
            },
            NatsEventStreamBusConfig {
                max_messages: -1,
                ..Default::default()
            },
            NatsEventStreamBusConfig {
                replicas: 0,
                ..Default::default()
            },
            NatsEventStreamBusConfig {
                replicas: 6,
                ..Default::default()
            },
        ];

        for config in invalid_configs {
            assert!(matches!(
                validate_config(&config),
                Err(EventStreamBusError::InvalidConfig { .. })
            ));
        }
    }
}
