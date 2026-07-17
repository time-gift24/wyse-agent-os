# stratum-infra conventions

## Scope

- `stratum-infra` contains external infrastructure adapters. Keep interface definitions in
  capability `definition.rs` files, errors in `error.rs`, and concrete backends/adapters in named
  modules.
- Do not move AgentStore projection into this crate. `StoreEventStreamBus` belongs to
  `stratum-store`; this crate owns retained transport and the adapter from foundational loop events
  to scoped runtime envelopes.

## EventStreamBus

- `EventStreamBus` publishes complete `StreamEnvelope` values and subscribes by `AgentId` plus
  `ReplayStart`. It is a retained delivery boundary, not the durable source of complete Agent
  history.
- `EventCursor` is an opaque transport position. JetStream uses its stream sequence internally;
  callers may serialize/replay it but must not compare it with `business_seq` or loop `iteration`.
- `ReplayStart::After(cursor)` means strictly after that cursor. Cursor expiry must be reported as
  `CursorExpired`, not silently changed to `All` or `New`; overflow and backend failures remain
  typed errors.
- NATS subjects are derived from the nested `RuntimeEvent::Agent.agent_id` and agent event type.
  An envelope without agent scope is rejected rather than published to a fallback subject.
- JetStream is limits-retained and independent of AgentStore. Retention loss is expected and is
  recovered through fixed-barrier AgentStore history plus a new subscription.
- Subscription decoding/delivery failure terminates the stream after the typed error. Never skip a
  malformed retained event and continue, because that would create an undetectable projection gap.

## Durable and telemetry sinks

- `DurableEventSink::append` is a correctness boundary: return only after the configured consumer
  acknowledges the event, and preserve the source error chain. Do not downgrade durable failure to
  logging.
- `TelemetryEventSink::emit` is synchronous best-effort and cannot fail or block the loop on
  transport I/O. The scoped implementation uses `try_send` into a bounded telemetry queue of 256,
  reports only the first loss/failure/timeout for a turn, and bounds each telemetry publish to 100ms. Durable events
  use a separate bounded priority lane and wait for their publish acknowledgment; they never wait
  behind a telemetry backlog. On durable arrival, discard older queued telemetry. Sequence assignment
  and enqueue share one critical section, and the worker keeps a durable fence for defensive late
  arrivals, so older telemetry cannot publish after a later durable message or terminal event.
- `ScopedAgentEventSink` is bound to exactly one `(agent_id, run_id, turn_id)` and agent name. It
  performs scope/protocol projection only; it must not own recovery, history paging, tool policy, or
  AgentStore state transitions.
- The current host has no workflow node context. Scoped envelopes therefore use
  `EventSource::Run` and nest identity in `RuntimeEvent::Agent { agent_id, event }`. Introducing
  `EventSource::Agent` requires an actual `node_id` from orchestration, not a placeholder.
- Durable projection includes loop start, complete messages, approvals, tool execution start,
  iteration completion, and terminal events. Telemetry projection includes supported LLM start,
  text/reasoning/tool-call deltas, and finish events. Adding a core variant requires an explicit
  mapping decision and tests; do not rely on wildcard behavior accidentally.
- Metadata is diagnostic only. The current `agent_name` and `turn_id` entries must not become
  business truth; downstream projections use the typed nested fields.

## Safety and observability

- Infrastructure errors must not contain payloads, prompts, reasoning, tool arguments/results,
  credentials, NATS auth material, or secrets. Structured logs use IDs, event type, cursor, timeout,
  and backend error only.
- Keep publish and subscription work cancellation-safe. Do not hold mutex/RwLock guards across
  `.await`; use bounded behavior wherever transport latency can otherwise block runtime progress.
- Real NATS tests remain ignored integration tests under `tests/` and run through the crate
  `Makefile`/`docker-compose.test.yml`. Unit tests must not require a live broker.
