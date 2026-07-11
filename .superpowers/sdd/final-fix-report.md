# Final review fixes

- Cursor-based recovery preserves same-Agent streamed drafts, tool progress, and pending approvals. A full replay after an expired cursor still resets those transient projections before rebuilding them.
- Sending is disabled and rejected while recovery or an active turn is in progress. Command methods return `false` on failure, so the composer clears only after a successful create or send. `agent_busy` is treated as an expected unsuccessful command rather than a connection error.
- Recovery and command-side 404 responses enter the `missing` state, enabling removal of stale local history entries. Reopening a stored Agent refreshes its timestamp and moves it to the front of the recent list.

## Verification

- `npm test -- app/features/agent-conversation/recovery.test.ts app/hooks/use-agent-conversation.test.ts app/components/chat-workspace.test.tsx app/features/agent-conversation/reducer.test.ts app/lib/recent-agents.test.ts` — 32 passed
- `npm run typecheck` — passed
- `npm run build` — passed
