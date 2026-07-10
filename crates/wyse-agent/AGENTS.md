# wyse-agent AGENTS.md

## Scope

`wyse-agent` owns the active turn loop, turn commands, tool approval events,
cancellation, and conversation history.

## Turn Control

- Use a bounded MPSC channel for interactive commands sent to an active turn.
- Keep cancellation on `CancellationToken` and prioritize it in `tokio::select!`.
- The agent owns approval interaction; `wyse-tools` owns authorization metadata.
- Publish `tool_approval_requested` successfully before waiting.
- Do not checkpoint pending approval state.
- Keep user-message queuing separate until its behavior is implemented.
