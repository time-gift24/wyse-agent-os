# Final review fixes

- Cursor-based recovery preserves same-Agent streamed drafts, tool progress, and pending approvals. A full replay after an expired cursor still resets those transient projections before rebuilding them.
- Sending is disabled and rejected while recovery or an active turn is in progress. Command methods return `false` on failure, so the composer clears only after a successful create or send. `agent_busy` is treated as an expected unsuccessful command rather than a connection error.
- Recovery and command-side 404 responses enter the `missing` state, enabling removal of stale local history entries. Reopening a stored Agent refreshes its timestamp and moves it to the front of the recent list.
- All terminal Agent events (`finished`, `failed`, and `cancelled`) clear pending tool approvals with transient drafts. This removes a cancellation's approval card even when no `tool_approval_resolved` event is emitted.

## Verification

- `npm test -- app/features/agent-conversation/recovery.test.ts app/hooks/use-agent-conversation.test.ts app/components/chat-workspace.test.tsx app/features/agent-conversation/reducer.test.ts app/lib/recent-agents.test.ts` — 32 passed
- `npm run typecheck` — passed
- `npm run build` — passed
- `pnpm --dir wyse-web test -- reducer.test.ts` — 41 passed
- `pnpm --dir wyse-web typecheck` — passed
- `pnpm --dir wyse-web build` — passed

## Host model configuration final-review fixes

- P1: an unavailable message `model_config.model` now returns `422` with
  `model_not_configured`, without changing the persisted model configuration.
- P2: the HTTP-only nested `model_config` request schema denies unknown fields,
  returning `400 invalid_request` for misspellings such as `paramters`.

### TDD evidence

- Before the implementation, `unavailable_message_model_returns_422_without_mutating_state`
  failed with `500` instead of `422`; `message_model_config_rejects_unknown_fields` failed with
  `202` instead of `400`.
- After the implementation, both exact regression tests passed.

### Verification

- `cargo fmt --all -- --check` — passed
- `cargo test -p wyse-api --test api` — 77 passed
- `cargo clippy --workspace --all-targets -- -D warnings` — passed

## Selector pending-state and documentation cleanup

### Changes

- Passed `ChatWorkspace` command-pending state (`isSubmitting`) to `ModelConfigMenu`.
- Added the command-pending condition to the selector disabled predicate, covering the interval after message acceptance and before the SSE `started` event updates `turnRunning`.
- Added a pure regression test for the command-pending disabled predicate.
- Removed the two prohibited committed process documents:
  - `docs/superpowers/plans/2026-07-12-host-model-config.md`
  - `docs/superpowers/specs/2026-07-12-host-model-config-design.md`

### Verification

- `npm test` — 1 file, 9 tests passed.
- `npm run typecheck` — passed.
- `npm exec prettier -- --check app/lib/model-config.ts app/lib/model-config.test.ts app/components/stratum/model-config-menu.tsx app/components/stratum/chat-workspace.tsx` — passed.
- `git diff --check` — passed.
- Confirmed both prohibited process-document paths are absent.

### Concerns

- `config.docker.toml` was already modified before this fix and was deliberately left unstaged and uncommitted. It contains a credential-looking value and should be reviewed and rotated outside this change.
- No Rust check was run: this change only affects TypeScript and documentation deletion.
