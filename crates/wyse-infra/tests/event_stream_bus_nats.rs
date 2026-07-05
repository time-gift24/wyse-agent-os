use std::{collections::BTreeMap, error::Error, time::Duration};

use chrono::Utc;
use futures_util::{StreamExt, future::try_join_all};
use serde_json::json;
use tokio::time::{Instant, sleep, timeout};
use wyse_core::{EventSource, RunId, RuntimeEvent, StreamEnvelope};
use wyse_infra::{
    EventStream, EventStreamBus, NatsEventStreamBusConfig, create_nats_event_stream_bus,
};

const DEFAULT_NATS_URL: &str = "nats://127.0.0.1:44227";

#[tokio::test]
#[ignore = "requires wyse-infra-test NATS container"]
async fn nats_event_stream_bus_publishes_and_subscribes_run_events() -> Result<(), Box<dyn Error>> {
    let nats_url =
        std::env::var("WYSE_INFRA_TEST_NATS_URL").unwrap_or_else(|_| DEFAULT_NATS_URL.to_owned());
    let bus = wait_for_bus(&nats_url).await?;
    let run_id = RunId::new();
    let envelopes = envelopes(run_id);
    let mut first_stream = bus.subscribe_run(run_id).await?;
    let mut second_stream = bus.subscribe_run(run_id).await?;

    eprintln!("subscribed two streams for run_id={run_id}");
    sleep(Duration::from_millis(10)).await;

    eprintln!("publishing {} envelopes concurrently", envelopes.len());
    try_join_all(envelopes.iter().cloned().map(|envelope| {
        eprintln!(
            "publish seq={} event_type={} metadata={:?}",
            envelope.seq,
            envelope.event.event_type(),
            envelope.metadata
        );
        bus.publish(envelope)
    }))
    .await?;

    let first_received = receive_envelopes("sub-1", &mut first_stream, envelopes.len()).await?;
    let second_received = receive_envelopes("sub-2", &mut second_stream, envelopes.len()).await?;

    assert_eq!(first_received, envelopes);
    assert_eq!(second_received, envelopes);

    Ok(())
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
        stream_name: "WYSE_EVENTS_TEST".to_owned(),
        subject_prefix: "wyse.test.events".to_owned(),
        replicas: 1,
    }
}

async fn receive_envelopes(
    subscriber: &str,
    stream: &mut EventStream,
    count: usize,
) -> Result<Vec<StreamEnvelope>, Box<dyn Error>> {
    let mut envelopes = Vec::with_capacity(count);

    for _ in 0..count {
        let envelope = timeout(Duration::from_secs(5), stream.next())
            .await?
            .transpose()?
            .ok_or_else(|| format!("{subscriber} ended before receiving all envelopes"))?;

        eprintln!(
            "{subscriber} received seq={} event_type={} metadata={:?}",
            envelope.seq,
            envelope.event.event_type(),
            envelope.metadata
        );
        envelopes.push(envelope);
    }

    Ok(envelopes)
}

fn envelopes(run_id: RunId) -> Vec<StreamEnvelope> {
    (1..=5)
        .map(|seq| {
            let mut metadata = BTreeMap::new();
            metadata.insert("batch".to_owned(), json!("nats-integration"));
            metadata.insert("pub_index".to_owned(), json!(seq));

            StreamEnvelope {
                run_id,
                seq,
                timestamp: Utc::now(),
                source: EventSource::Run,
                event: RuntimeEvent::NodeOutput {
                    output: json!({
                        "value": "ok",
                        "seq": seq,
                        "items": [seq, seq + 10, seq + 20],
                    }),
                },
                metadata,
            }
        })
        .collect()
}
