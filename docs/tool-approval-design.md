# Tool Approval Design

## Goal

Add runtime tool approval without coupling `wyse-tools` to the event bus or
checkpoint storage. The tool registry owns tool metadata and decides whether a
call can run immediately. The active agent turn owns user interaction and
executes or rejects the original tool call after receiving a command.

## Scope

The first version provides:

- registration-time tool kind and danger metadata
- registry-wide `Allow`, `PartialAllow`, and `RequireApproval` modes
- typed approval requested and resolved events
- an agent turn command channel for approval decisions
- approval, rejection, cancellation, and duplicate-decision handling

It does not provide custom policy evaluators, rule DSLs, tool-spec filtering,
approval timeouts, durable approval recovery, or user-message injection.

## Crate Boundaries

### `wyse-core`

Add protocol types shared by the registry and agent runtime:

```text
ApprovalId          UUIDv7 newtype
ToolKind            read | write
DangerLevel         low | medium | high
ApprovalDecision    approve | reject
```

Add nested agent events:

```text
tool_approval_requested
tool_approval_resolved
```

The resolved event contains only the stable `approval_id` and the accepted
`ApprovalDecision`; clients correlate the remaining display fields with the
requested event.

### `wyse-tools`

The registry stores each tool with its `ToolKind` and `DangerLevel`. It owns the
permission mode and reports whether a call is allowed or requires approval. It
does not publish events, wait for user input, or retain pending calls.

### `wyse-agent`

The active turn publishes approval events, receives approval commands, and
decides whether the original call is executed or rejected. It remains the sole
owner of run event sequencing and cancellation.

### `wyse-infra` and `wyse-checkpoint`

No approval-specific component is added. The existing `EventStreamBus` carries
the new typed events. Approval state remains process-local and is not added to
the checkpoint payload.

## Tool Registration and Permission Modes

Tool registration accepts the implementation and its security metadata:

```text
register(tool, tool_kind, danger_level)
```

`ToolSpec` remains provider-visible schema only. Runtime security metadata is
not added to the provider schema.

The registry has one `ToolPermissionMode`:

```text
allow
partial_allow
require_approval
```

The mode is selected when the registry is constructed and remains immutable
after the registry is shared with an agent. Runtime mode mutation is deferred
until a caller needs it.

`Allow` authorizes every registered tool immediately. `RequireApproval` marks
every call as requiring approval. `PartialAllow` authorizes a call immediately
only when both conditions hold:

```text
tool_kind == read && danger_level == low
```

Every other `PartialAllow` call requires approval. The default mode is `Allow`
to preserve current behavior. All registered tool specs remain visible to the
LLM in every mode.

The three concrete modes are represented by an enum. A generic policy trait is
deferred until a caller needs custom rules.

## Registry Authorization

The registry exposes an authorization check alongside its existing `call`:

```text
authorization(tool_name) -> ToolAuthorization

ToolAuthorization
- Allowed
- RequireApproval { tool_kind, danger_level }
```

The mode and registration metadata are immutable after the registry is shared,
so the agent can authorize once and call the existing execution method after an
approval without a policy race. The current agent loop already retains the
original `ToolCall`; no prepared-call continuation is added.

Every agent tool path must call `authorization` before `call`. The existing
`call` remains a lower-level in-process execution primitive and is not exposed
directly to the model or an external API.

The requested agent event contains:

```text
approval_id
agent_name
call_id
tool_name
arguments
tool_kind
danger_level
```

The request uses `agent_name` for user-facing display. Approval routing uses the
UUIDv7 `approval_id`; the frontend does not depend on `AgentId`.

## Turn Command Channel

Each `run_turn` creates a bounded Tokio MPSC channel with a fixed capacity of
1. The sender remains available through `Agent`; the receiver moves into the
single active turn task. Capacity is a constant rather than a configuration
option until a real caller needs tuning.

The first version has one internal command:

```text
TurnCommand::ResolveToolApproval {
  approval_id,
  decision,
  response: oneshot sender of Result<(), AgentError>,
}
```

`Agent::resolve_tool_approval` sends this command and waits for the response. It
returns `NoActiveTurn` when no turn command sender exists or the active turn has
already ended. `NoActiveTurn` and `ApprovalNotFound` are `AgentError` variants,
because they describe active-turn control rather than tool execution.

The receiver is included in every long-lived turn `tokio::select!`: starting an
LLM call, consuming its stream, waiting for approval, and executing an approved
tool. An approval command is accepted only while the matching approval is
active. Otherwise its response is `ApprovalNotFound`. This keeps commands from
remaining queued without a consumer and gives concurrent callers a definite
result.

Cancellation remains a `CancellationToken`, not a queued command. Each select
uses `biased;` with `cancel.cancelled()` first, so cancellation wins when it and
another branch are ready together. This also prevents cancellation from being
delayed behind a full command channel.

A future user-message feature may add a separate `TurnCommand` variant and its
own message queue. This design does not add a generic `Pending` enum or an empty
user-message variant now.

## Approval Data Flow

Direct execution keeps the current flow:

```text
tool_call_started
registry permission check
tool execution
tool_call_finished | tool_call_failed
```

An approval-required call uses:

```text
tool_call_started
registry authorization requires approval
agent publishes tool_approval_requested
agent waits for cancellation or a matching TurnCommand
agent publishes tool_approval_resolved
approve: execute the original call, then tool_call_finished | tool_call_failed
reject: do not execute, emit tool_call_failed, continue the LLM loop
```

The rejection tool message is stable JSON:

```json
{
  "error": {
    "type": "approval_rejected",
    "message": "user rejected tool call"
  }
}
```

This message is appended as the result for the original provider `call_id`, so
the next LLM request can respond without retrying the denied side effect.

## Event Delivery

`tool_approval_requested` is a control-plane notification. Its publish must
return the underlying event bus result instead of using the existing
warning-only helper. If publication fails, the turn fails without calling the
tool rather than waiting for an approval no user can see.

`tool_approval_resolved` follows the existing best-effort live event behavior.
If it cannot be published, the runtime logs one warning and continues with the
already accepted decision. The following tool finished or failed event allows
subscribers to converge.

The event order for a handled approval is:

```text
tool_call_started
tool_approval_requested
tool_approval_resolved
tool_call_finished | tool_call_failed
```

## Error and Concurrency Semantics

- Reject is a normal tool-call outcome, not a registry error.
- A wrong, duplicate, expired, or cancelled approval ID returns
  `ApprovalNotFound` through the command response.
- The first matching approval command handled by the turn wins.
- Later commands are still consumed and receive `ApprovalNotFound` while the
  turn remains active; if the turn ends first, their callers receive
  `NoActiveTurn`.
- Cancellation wins over approval and over starting an approved tool call.
- After cancellation, no new tool execution begins.
- Cancellation publishes the existing agent cancellation event and does not
  synthesize an approval rejection.
- Errors from an approved tool keep the existing `ToolError` and
  `ToolCallFailed` behavior.
- No mutex or lock guard is held across `.await`.
- Turn completion, failure, or cancellation drops the receiver and clears the
  sender stored by `Agent`.
- A process restart loses an outstanding approval. Its running checkpoint is
  not approval-resumable; the caller starts a new turn.

## Testing

### `wyse-core`

- `ApprovalId` is UUIDv7.
- New enums and approval events serialize to their documented snake-case names.

### `wyse-tools`

- `Allow` executes every tool directly.
- `PartialAllow` directly executes only `Read + Low`.
- Every other `PartialAllow` combination requires approval.
- `RequireApproval` requires approval for every tool call.
- Registration retains the correct tool kind and danger level.
- Authorization has no tool side effect.

### `wyse-agent`

- A complete requested event is published before the tool can execute.
- Approve publishes resolved, executes the tool, and continues the LLM loop.
- Reject does not execute the tool and sends the structured rejection result to
  the next LLM request.
- A wrong approval ID receives `ApprovalNotFound` without disturbing the active
  approval.
- Concurrent duplicate decisions produce one success and one
  `ApprovalNotFound`.
- Cancellation wins when cancellation and approval are ready together.
- Requested-event publication failure fails the turn without executing the
  tool.
- Resolving after turn completion returns `NoActiveTurn`.

Existing mock providers, an in-memory event bus, and a counting test tool are
sufficient. No external service or container integration test is added.

## Deferred Work

- user messages queued and explicitly injected into an active turn
- durable approval recovery
- approval timeout policy
- tool-spec filtering
- custom policy evaluator or rule DSL
- turn-wide or persistent approval grants

Before the implementation is merged, the final conventions must be archived in
the affected crate `AGENTS.md` files.
