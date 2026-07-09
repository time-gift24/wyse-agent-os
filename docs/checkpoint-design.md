# Checkpoint Design

## Goal

Wyse needs durable resume and explicit retry for agent runs, and later for workflow
graph runs. Checkpoints are the source of truth for runtime recovery. Event streams
remain a live UI channel and are not used to rebuild runtime state.

This follows the same broad split used by LangGraph: checkpoint durable state at
execution boundaries, while token streaming remains a separate live stream.

## Crate Boundary

Add a `wyse-checkpoint` crate.

The crate owns:

- common checkpoint record types
- a small checkpoint store trait
- a SQLite store implementation

It does not own agent or workflow state machines. `wyse-agent` defines its own
typed checkpoint state and serializes it into the common checkpoint record. A
future `wyse-workflow` crate can reuse the same store with a workflow-specific
state payload.

## Common Record

The shared checkpoint record should be small and generic:

```text
CheckpointRecord
- run_id
- checkpoint_id
- parent_checkpoint_id
- owner: agent | workflow
- status: running | waiting_retry | finished | cancelled
- phase: serde_json::Value
- state: serde_json::Value
- last_seq
- retry_count
- last_error_text
- created_at
```

The first store trait should stay narrow:

```text
CheckpointStore
- put(record) -> Result<()>
- latest(run_id, owner) -> Result<Option<CheckpointRecord>>
```

Skip checkpoint history browsing, time travel, branching APIs, Postgres, pending
writes, and event logs until a caller needs them.

## Agent State

`wyse-agent` should keep a typed state before serializing it into
`CheckpointRecord.state`:

```text
AgentCheckpointState
- agent_id
- history: Vec<ChatMessage>
- usage
- turn_index
- pending_tool_calls
- next_tool_call_index
```

`history` is required for resume. It must contain only stable messages that the
runtime can safely continue from. Partial assistant text from a failed LLM stream
is not stable and must not be written into `history`.

## Save Points

Agent runtime saves checkpoints at durable execution boundaries:

- after the user message enters `history`
- before starting an LLM call, with `running_llm`
- after an LLM call finishes, with the complete assistant message in `history`
- after each tool call finishes, with the tool message and advanced tool index
- after a retryable runtime failure, with `waiting_retry`
- after successful finish or user cancellation

LLM token deltas, reasoning deltas, and tool-call argument deltas are not
checkpointed.

## Retry

`waiting_retry` is a paused state, not a terminal failure. Retry is explicit:
a caller resumes the run by `run_id`.

LLM stream failure:

- checkpoint: `waiting_retry` at `running_llm`
- history: unchanged from before the LLM call
- retry: send the same request again from stable history

Tool runtime failure:

- checkpoint: `waiting_retry` at `running_tools`
- history: includes only completed tool results
- retry: continue at `next_tool_call_index`

Tool business failure is different. If a tool returns a structured error output
that the model can reason about, write it as a tool message and continue.

## Event Stream Relationship

`EventStreamBus` remains the live stream boundary. It should not persist or
derive checkpoints.

Runtime order at durable boundaries:

```text
save checkpoint
publish live event
```

Checkpoint save failure stops the runtime. Continuing would create work with no
reliable resume point.

Live stream publish failure does not invalidate the checkpoint. The runtime may
continue after logging a warning, because NATS is a frontend delivery channel,
not the recovery source of truth.

## Frontend Recovery

Frontend recovery uses two layers:

- checkpoint snapshot for stable state
- NATS JetStream short retention for best-effort recent live events

The API shape can be:

```text
GET /runs/{run_id}
GET /runs/{run_id}/events?after_seq=123
```

On reconnect, the frontend first reads the checkpoint-derived snapshot, then
subscribes to events after its last seen sequence. If NATS retention no longer
contains the requested events, the server sends a reset event and the frontend
rebuilds from the snapshot.

Partial assistant text is attempt-local. It is promoted into stable UI state only
after the matching LLM call finishes and the assistant message is present in the
checkpoint state.

## Failure Semantics

LLM disconnect:

- already streamed token deltas may remain visible as failed-attempt draft text
- checkpoint becomes `waiting_retry`
- stable history does not include partial assistant text
- retry creates a new LLM attempt from stable history

NATS publish failure:

- checkpoint remains valid
- frontend may miss token deltas or live progress
- reconnect repairs stable UI from the checkpoint snapshot

Checkpoint save failure:

- runtime stops immediately
- no further live durable event should be published for that boundary

Runtime crash:

- resume from latest checkpoint
- in-flight token deltas are lost unless still available in NATS retention

## First Version

Build the minimal version:

- `wyse-checkpoint` with common records, narrow trait, and SQLite store
- `wyse-agent` save points for history, phase, tool progress, usage, and retry
- explicit resume from `waiting_retry`
- best-effort NATS/live stream with warning-only failure handling

Do not build event log persistence in the first version.

## References

- LangGraph checkpointers: <https://docs.langchain.com/oss/python/langgraph/checkpointers>
- LangGraph streaming: <https://docs.langchain.com/oss/javascript/langgraph/streaming>
