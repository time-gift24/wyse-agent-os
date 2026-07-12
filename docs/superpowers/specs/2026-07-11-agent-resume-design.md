# Agent Resume Design

## Goal

Allow a fully built `Agent` to resume an unfinished persisted turn without a
new user message:

```rust
let run_id = agent.resume().await?;
```

Resume continues the `run_id` and `turn_id` stored in `agent.json`. Web or the
scheduler guarantees that only the owning writer calls `resume`; the runtime
does not add another lease.

## Dependencies

`AgentBuilder` receives both `Arc<dyn AgentStore>` and
`Arc<dyn EventStreamBus>`. The store is the durable resume source. The event bus
continues to publish and retain runtime events; JetStream metadata and realtime
deltas are never used to reconstruct execution state.

Web composition must inject the same logical per-Agent store into the Agent and
the `StoreEventStreamBus` that commits its messages. The crates do not construct
or discover that store themselves.

## Persisted iteration frontier

`agent.json` adds `next_iteration: u64`. It is the next LLM loop iteration that
has not reached a stable durable boundary. New runs start at zero.

`AgentStore::complete_iteration` atomically verifies the running run, turn, and
expected iteration, then advances `next_iteration`, persists cumulative usage,
and updates `updated_at` through the existing filesystem CAS loop. Message files
are committed before this state update.

The state schema is changed in place. No migration or compatibility aliases are
added because the project is still in active development.

## Resume reconstruction

`Agent::resume()`:

1. Rejects a second active operation.
2. Loads `agent.json` and requires `status = running`, matching `agent_id`, and
   present `run_id` and `turn_id`.
3. Pages committed messages through the fixed `last_seq` barrier and rebuilds
   the complete conversation history.
4. Restores run identity, turn identity, usage, cancellation, and the bounded
   command channel.
5. Spawns the turn continuation and returns the persisted `run_id`.

It does not publish another Started event or duplicate the persisted user
message.

## Stable-boundary reconciliation

For the active turn, the number of committed assistant messages must equal
`next_iteration` or exceed it by exactly one:

- Equal: no completed assistant response is waiting for iteration commit; start
  the LLM at `next_iteration`, unless the last assistant is already terminal.
- One greater: the assistant response was committed before the process stopped.
  Resume skips already committed tool results, executes only missing tool calls,
  then calls `complete_iteration`.
- Any other relationship is corrupted resume history and fails closed.

An assistant message without tool calls is terminal. If its Finished lifecycle
event was not persisted before the process stopped, resume finishes the turn
without another LLM request and reports an unknown finish reason.

## Tool execution trade-off

Tool execution is at-least-once. A process can stop after an external side
effect but before the tool result message is committed. Resume then executes the
same tool call again. Every tool implementation must guarantee idempotent
execution for the same tool call identity. The runtime does not add a tool
transaction log or exactly-once protocol.

## Testing

Tests cover CAS iteration advancement, new-run reset, direct resume from a
stable history boundary, terminal assistant reconciliation, partial tool-result
reconciliation, duplicate active resume rejection, and malformed persisted
resume state. Workspace tests, Clippy, formatting, and the existing NATS
integration suite remain green.
