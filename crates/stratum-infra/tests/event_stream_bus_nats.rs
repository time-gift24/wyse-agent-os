use std::{collections::BTreeMap, error::Error, time::Duration};

use async_nats::jetstream::stream::{DiscardPolicy, RetentionPolicy, StorageType};
use bytes::Bytes;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use stratum_core::{
    AgentEvent, AgentId, EventRecord, EventSource, ReplayStart, RunId, RuntimeEvent,
    StreamEnvelope, TokenUsage, TurnId,
};
use stratum_infra::{
    EventStream, EventStreamBus, EventStreamBusError, NatsEventStreamBusConfig,
    create_nats_event_stream_bus,
};
use tokio::time::{Instant, sleep, timeout};

const DEFAULT_NATS_URL: &str = "nats://127.0.0.1:44227";
const TEST_STREAM: &str = "AGENT_EVENTS_TEST";
const TEST_SUBJECT_PREFIX: &str = "events.agent";
const INCOMPATIBLE_STREAM: &str = "INCOMPATIBLE_AGENT_EVENTS_TEST";
const INCOMPATIBLE_SUBJECT_PREFIX: &str = "events.incompatible.agent";

#[tokio::test]
#[ignore = "requires NATS JetStream"]
async fn nats_rejects_incompatible_existing_stream() -> Result<(), Box<dyn Error>> {
    let nats_url = nats_url();
    let client = async_nats::connect(&nats_url).await?;
    let jetstream = async_nats::jetstream::new(client);
    let _ = jetstream.delete_stream(INCOMPATIBLE_STREAM).await;
    jetstream
        .create_stream(async_nats::jetstream::stream::Config {
            name: INCOMPATIBLE_STREAM.to_owned(),
            subjects: vec![format!("{INCOMPATIBLE_SUBJECT_PREFIX}.>")],
            storage: StorageType::Memory,
            ..Default::default()
        })
        .await?;

    let result = create_nats_event_stream_bus(NatsEventStreamBusConfig {
        url: nats_url,
        stream_name: INCOMPATIBLE_STREAM.to_owned(),
        subject_prefix: INCOMPATIBLE_SUBJECT_PREFIX.to_owned(),
        ..Default::default()
    })
    .await;

    assert!(
        matches!(result, Err(EventStreamBusError::Nats { .. })),
        "incompatible existing stream was silently accepted"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "requires NATS JetStream"]
async fn nats_agent_replay_modes_and_isolation() -> Result<(), Box<dyn Error>> {
    let nats_url = nats_url();
    let bus = wait_for_bus(&nats_url).await?;
    assert_server_retention_config(&nats_url).await?;

    let agent_id = AgentId::new();
    let other_agent_id = AgentId::new();
    let first = agent_envelope(agent_id, started_event());
    let other = agent_envelope(other_agent_id, started_event());
    let second = agent_envelope(agent_id, cancelled_event());
    let mut live = bus.subscribe_agent(agent_id, ReplayStart::New).await?;

    bus.publish(first.clone()).await?;
    bus.publish(other.clone()).await?;
    bus.publish(second.clone()).await?;

    let live_records = receive_records(&mut live, 2).await?;
    assert_eq!(
        live_records
            .iter()
            .map(|record| &record.envelope)
            .collect::<Vec<_>>(),
        vec![&first, &second]
    );
    assert!(
        live_records[0].cursor.transport_sequence() < live_records[1].cursor.transport_sequence()
    );

    let mut all = bus.subscribe_agent(agent_id, ReplayStart::All).await?;
    let all_records = receive_records(&mut all, 2).await?;
    assert_eq!(all_records, live_records);

    let mut after = bus
        .subscribe_agent(agent_id, ReplayStart::After(all_records[0].cursor))
        .await?;
    assert_eq!(receive_record(&mut after).await?, all_records[1]);
    assert_no_record(&mut after).await?;

    let mut isolated = bus
        .subscribe_agent(other_agent_id, ReplayStart::All)
        .await?;
    assert_eq!(receive_record(&mut isolated).await?.envelope, other);
    assert_no_record(&mut isolated).await?;

    let mut new = bus.subscribe_agent(agent_id, ReplayStart::New).await?;
    let third = agent_envelope(agent_id, started_event());
    bus.publish(third.clone()).await?;
    assert_eq!(receive_record(&mut new).await?.envelope, third);

    Ok(())
}

#[tokio::test]
#[ignore = "requires NATS JetStream"]
async fn nats_reports_expired_cursor() -> Result<(), Box<dyn Error>> {
    let nats_url = nats_url();
    let bus = wait_for_bus(&nats_url).await?;
    let agent_id = AgentId::new();

    bus.publish(agent_envelope(agent_id, started_event()))
        .await?;
    bus.publish(agent_envelope(agent_id, cancelled_event()))
        .await?;
    let mut all = bus.subscribe_agent(agent_id, ReplayStart::All).await?;
    let retained = receive_records(&mut all, 2).await?;

    let client = async_nats::connect(&nats_url).await?;
    let jetstream = async_nats::jetstream::new(client);
    let stream = jetstream.get_stream(TEST_STREAM).await?;
    stream.purge().await?;

    let after_purge = agent_envelope(agent_id, started_event());
    bus.publish(after_purge.clone()).await?;

    let error = match bus
        .subscribe_agent(agent_id, ReplayStart::After(retained[0].cursor))
        .await
    {
        Ok(_) => return Err(std::io::Error::other("expired cursor was accepted").into()),
        Err(error) => error,
    };
    assert!(matches!(
        error,
        EventStreamBusError::CursorExpired { cursor } if cursor == retained[0].cursor
    ));

    let mut boundary = bus
        .subscribe_agent(agent_id, ReplayStart::After(retained[1].cursor))
        .await?;
    assert_eq!(receive_record(&mut boundary).await?.envelope, after_purge);

    Ok(())
}

#[tokio::test]
#[ignore = "requires NATS JetStream"]
async fn nats_malformed_payload_terminates_subscription() -> Result<(), Box<dyn Error>> {
    let nats_url = nats_url();
    let bus = wait_for_bus(&nats_url).await?;
    let agent_id = AgentId::new();
    let subject = format!("{TEST_SUBJECT_PREFIX}.{agent_id}.started");
    let client = async_nats::connect(&nats_url).await?;
    let jetstream = async_nats::jetstream::new(client);

    jetstream
        .publish(subject, Bytes::from_static(b"not valid json"))
        .await?
        .await?;

    let mut events = bus.subscribe_agent(agent_id, ReplayStart::All).await?;
    let error = timeout(Duration::from_secs(5), events.next())
        .await?
        .ok_or_else(|| std::io::Error::other("missing malformed retained event"))?
        .expect_err("malformed event must be reported");
    assert!(matches!(error, EventStreamBusError::Deserialize(_)));
    assert!(
        timeout(Duration::from_secs(5), events.next())
            .await?
            .is_none(),
        "subscription continued after malformed retained event"
    );

    Ok(())
}

#[tokio::test]
#[ignore = "requires NATS JetStream"]
async fn seed_file_backed_event_for_restart() -> Result<(), Box<dyn Error>> {
    let nats_url = nats_url();
    let bus = wait_for_bus(&nats_url).await?;
    bus.publish(restart_fixture_envelope()).await?;
    Ok(())
}

#[tokio::test]
#[ignore = "requires NATS JetStream"]
async fn replay_file_backed_event_after_restart() -> Result<(), Box<dyn Error>> {
    let nats_url = nats_url();
    let bus = wait_for_bus(&nats_url).await?;
    let mut events = bus
        .subscribe_agent(restart_fixture_agent_id(), ReplayStart::All)
        .await?;
    let next = timeout(Duration::from_secs(5), events.next())
        .await?
        .ok_or_else(|| std::io::Error::other("missing retained restart fixture"))?;
    let record = next?;
    assert_eq!(record.envelope, restart_fixture_envelope());
    Ok(())
}

fn nats_url() -> String {
    std::env::var("STRATUM_INFRA_TEST_NATS_URL").unwrap_or_else(|_| DEFAULT_NATS_URL.to_owned())
}

async fn wait_for_bus(nats_url: &str) -> Result<impl EventStreamBus, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(10);

    loop {
        match create_nats_event_stream_bus(config(nats_url)).await {
            Ok(bus) => return Ok(bus),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(Box::new(error));
                }
                sleep(Duration::from_millis(200)).await;
            }
        }
    }
}

fn config(nats_url: &str) -> NatsEventStreamBusConfig {
    NatsEventStreamBusConfig {
        url: nats_url.to_owned(),
        stream_name: TEST_STREAM.to_owned(),
        ..Default::default()
    }
}

async fn assert_server_retention_config(nats_url: &str) -> Result<(), Box<dyn Error>> {
    let client = async_nats::connect(nats_url).await?;
    let jetstream = async_nats::jetstream::new(client);
    let stream = jetstream.get_stream(TEST_STREAM).await?;
    let config = &stream.cached_info().config;

    assert_eq!(config.subjects, vec![format!("{TEST_SUBJECT_PREFIX}.>")]);
    assert_eq!(config.storage, StorageType::File);
    assert_eq!(config.retention, RetentionPolicy::Limits);
    assert_eq!(config.discard, DiscardPolicy::Old);
    assert_eq!(config.max_age, Duration::from_secs(7 * 24 * 60 * 60));
    assert_eq!(config.max_bytes, 1_073_741_824);
    assert_eq!(config.max_messages, 1_000_000);
    assert_eq!(config.num_replicas, 1);

    Ok(())
}

async fn receive_records(
    stream: &mut EventStream,
    count: usize,
) -> Result<Vec<EventRecord>, Box<dyn Error>> {
    let mut records = Vec::with_capacity(count);
    for _ in 0..count {
        records.push(receive_record(stream).await?);
    }
    Ok(records)
}

async fn receive_record(stream: &mut EventStream) -> Result<EventRecord, Box<dyn Error>> {
    timeout(Duration::from_secs(5), stream.next())
        .await?
        .transpose()?
        .ok_or_else(|| std::io::Error::other("event stream ended unexpectedly").into())
}

async fn assert_no_record(stream: &mut EventStream) -> Result<(), Box<dyn Error>> {
    match timeout(Duration::from_millis(200), stream.next()).await {
        Err(_) => Ok(()),
        Ok(Some(Ok(record))) => Err(std::io::Error::other(format!(
            "unexpected event at cursor {:?}",
            record.cursor
        ))
        .into()),
        Ok(Some(Err(error))) => Err(error.into()),
        Ok(None) => Err(std::io::Error::other("event stream ended unexpectedly").into()),
    }
}

fn agent_envelope(agent_id: AgentId, event: AgentEvent) -> StreamEnvelope {
    StreamEnvelope {
        business_seq: None,
        run_id: RunId::new(),
        timestamp: Utc::now(),
        source: EventSource::Run,
        event: RuntimeEvent::Agent { agent_id, event },
        metadata: BTreeMap::new(),
    }
}

fn started_event() -> AgentEvent {
    AgentEvent::Started {
        turn_id: TurnId::new(),
    }
}

fn cancelled_event() -> AgentEvent {
    AgentEvent::Cancelled {
        usage: TokenUsage::default(),
    }
}

fn restart_fixture_agent_id() -> AgentId {
    "0197f4d0-0000-7000-8000-000000000001"
        .parse()
        .expect("fixed AgentId is valid")
}

fn restart_fixture_envelope() -> StreamEnvelope {
    StreamEnvelope {
        business_seq: None,
        run_id: "0197f4d0-0000-7000-8000-000000000002"
            .parse()
            .expect("fixed RunId is valid"),
        timestamp: DateTime::parse_from_rfc3339("2026-07-11T00:00:00Z")
            .expect("fixed timestamp is valid")
            .with_timezone(&Utc),
        source: EventSource::Run,
        event: RuntimeEvent::Agent {
            agent_id: restart_fixture_agent_id(),
            event: AgentEvent::Started {
                turn_id: "0197f4d0-0000-7000-8000-000000000003"
                    .parse()
                    .expect("fixed TurnId is valid"),
            },
        },
        metadata: BTreeMap::new(),
    }
}
