# Stratum REPL Echo approval-flow design

## Goal

Extend `stratum-repl` so a local operator can manually approve or reject one
safe demonstration tool call. This validates the agent runtime's complete tool
approval lifecycle without granting filesystem access to the model.

## Tool registration

`build_default_agent` stops constructing its own empty tool registry and
instead accepts an injected `Arc<dyn ToolRegistry>`. It remains a pure agent
constructor: callers continue to own provider, store, event bus, and tool
selection.

`stratum-repl` constructs a `BuiltinToolRegistry` in `RequireApproval` mode.
It registers exactly one `EchoTool`, marked `ToolKind::Read` and
`DangerLevel::Low`. The registry's permission mode means every Echo invocation
requires an explicit operator decision. No filesystem tool is registered, so
this milestone cannot let a model read or mutate local files through tools.

## Terminal interaction

The REPL continues to subscribe to the event bus before starting or resuming a
turn. While consuming that turn's events, it handles
`AgentEvent::ToolApprovalRequested` by printing the approval ID, tool name,
arguments, declared kind, and danger level, then prompting for exactly
`approve` or `reject`.

An invalid response repeats the prompt. A valid response invokes
`Agent::resolve_tool_approval` for the pending ID, after which the same event
consumer continues until a terminal agent event. EOF at the approval prompt is
treated as `reject`, so a user disconnect cannot leave the agent waiting for a
decision.

Default terminal output remains assistant text plus operational prompts and
diagnostics. With `--debug`, every received `StreamEnvelope`, including
approval requested and resolved events, is emitted as NDJSON from the event
bus.

## Approval outcomes and errors

Approval delegates to the existing runtime behavior. `approve` executes Echo
and returns its result to the model. `reject` publishes the resolved event and
returns the runtime's structured `approval_rejected` tool result without
executing Echo; the model may then finish its response.

Input, stream, and approval-resolution failures remain terminal REPL errors.
The existing closed-stream protection still applies. The REPL introduces no
approval queue, approval configuration, or slash-command protocol because one
agent turn can wait for one approval at a time.

## Verification

Mock-provider tests drive approval-requiring Echo calls through the complete
REPL event consumer. They verify both approve and reject paths: the prompt,
the resolved event, the final assistant response, and persisted message order.
An EOF-at-prompt test verifies automatic rejection. Tests use the existing
temporary `LocalFilesystem` store and no provider credentials.

`crates/wyse-agent-builtin/AGENTS.md` records that this REPL registers Echo
solely for approval-flow validation and that every tool call needs an operator
decision.
