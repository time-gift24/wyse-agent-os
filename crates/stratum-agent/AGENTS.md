# stratum-agent AGENTS.md

## Scope

`stratum-agent` contains the session-independent `AgentLoop` kernel and the
legacy stateful `Agent` compatibility path.

## AgentLoop Kernel

- `AgentLoop` consumes a caller-preloaded `LoopContext` plus user prompts. It
  does not own session creation, history loading, an `AgentStore`, or an
  `EventStreamBus`.
- Required transitions use `DurableEventSink`; partial output and progress use
  the separate best-effort `TelemetryEventSink`.
- A durable append must be acknowledged before the kernel mutates its in-memory
  transcript or starts the next external action.
- Tool calls execute sequentially through `ToolExecutor`. Approval and
  `ToolExecutionStarted` must be durable before dispatch, and each tool result
  must be durable before the next tool or model request.
- The run's supplied `CancellationToken` propagates through model and tool
  operations. Cancellation is cooperative: after `ToolExecutionStarted`, the
  caller must keep polling the loop so it can await and record the outcome.
  A durable start without a result is an unknown outcome and is never retried
  automatically by the kernel.

## Legacy Agent Compatibility

The following rules describe the existing `Agent`, session, resume, store, and
`EventStreamBus` integration. This remains temporary compatibility code and is
not the ownership model for the new `AgentLoop` kernel.

- The Agent receives an injected `EventStreamBus` for event delivery and an
  injected `AgentStore` for durable resumption.
- The loop publishes required complete-message and lifecycle events as
  unsequenced `StreamEnvelope` values.
- Complete-message commit and retained event delivery remain downstream bus
  responsibilities; the Agent uses the store to restore durable state and
  advance its resume position.

## Turn Control (Legacy Agent)

- Use a bounded MPSC channel for interactive commands sent to an active turn.
- Keep cancellation on `CancellationToken` and prioritize it in `tokio::select!`.
- The agent owns approval interaction; `stratum-tools` owns authorization metadata.
- Publish `tool_approval_requested` successfully before waiting.
- Keep user-message queuing separate until its behavior is implemented.

## Resume (Legacy Agent)

- `Agent::resume()` takes no user message. It loads the injected store and
  continues the unfinished turn with the same persisted `run_id` and `turn_id`.
- `agent.json` records `next_iteration` as the durable iteration frontier: every
  lower iteration has committed its stable boundary, while the frontier has not.
  It is not simply the next LLM request because committed history may instead
  require tool reconciliation or terminal completion without another LLM call.
  Do not recover this frontier from JetStream metadata.
- Resume rebuilds conversation history only from committed complete messages
  through the fixed `last_seq` captured from the loaded state; realtime deltas
  are never resume state.
- Resume validates the active turn against `next_iteration`. Committed tool
  result messages must be the exact ordered prefix of the immediately preceding
  assistant `tool_calls`. Unknown, duplicate, sparse, or out-of-order results
  are invalid resume history; only the missing suffix executes.
- Advance `next_iteration` with the `agent.json` CAS only after the assistant
  message and every tool result message for the iteration are durably committed.
- Resumed LLM, tool, complete-message, and lifecycle events continue through the
  injected `EventStreamBus`; resume does not publish directly to the store or
  retained transport.
- Tool execution is at-least-once. A process may stop after a tool has produced
  an external side effect but before its result message is committed, causing
  resume to execute that tool again. Every tool implementation must therefore
  guarantee idempotent execution for the same tool call.
- Web or scheduler composition guarantees that only the Agent owner resumes and
  writes the turn; `stratum-agent` does not add a second writer lease.
