# Task 3 Report

Status: DONE

Files changed:
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/Cargo.toml`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/Cargo.lock`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/crates/wyse-infra/Cargo.toml`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/crates/wyse-infra/src/event_stream_bus/definition.rs`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/crates/wyse-infra/src/event_stream_bus/mod.rs`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/crates/wyse-infra/src/event_stream_bus/nats.rs`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/crates/wyse-infra/src/event_stream_bus/memory.rs`

Commits:
- `c815287` `feat: add in-memory event stream bus`

Tests run:
- `cargo fmt` - passed
- `cargo test -p wyse-infra --all-targets` - passed, 5 unit tests passed and 1 integration test was ignored as expected
- `cargo clippy -p wyse-infra --all-targets -- -D warnings` - passed

Self-review notes:
- `EventStreamBus` is now `async_trait`-based and object-safe, which keeps it consistent with the earlier `LlmProvider` change.
- `InMemoryEventStreamBus` uses a short-lived `Mutex` lock only to fetch or create the per-run broadcast sender, so no guard is held across `.await`.
- The implementation stays narrow: one bounded broadcast channel per run, no extra abstraction layer.

Concerns:
- `InMemoryEventStreamBus::new(0)` currently falls back to a minimum broadcast capacity of `1` rather than returning an error; that keeps the API simple but is an implicit behavior.
- The task brief’s step list did not include `Cargo.lock`, but the dependency changes required it, so it was updated and committed with the source changes.

## Task 3 Review Fixes

Status: DONE

Findings fixed:
- In `crates/wyse-infra/src/event_stream_bus/memory.rs`, removed silent capacity coercion in `sender()` so `broadcast::channel` is created with `self.capacity` exactly as passed to `InMemoryEventStreamBus::new(capacity)`.
- Corrected `subscribe_run` docs to remove the misleading error-path claim; implementation now documents that the in-memory backend is infallible and returns `Ok`.

Files changed:
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/crates/wyse-infra/src/event_stream_bus/memory.rs`
- `/Users/wanyaozhong/projects/wyse-agent-os/.worktrees/wyse-agent-design/.superpowers/sdd/task-3-report.md`

Tests run:
- `cargo test -p wyse-infra --all-targets`  
  - 5 unit tests passed
  - 1 integration test ignored as expected

Test output summary:
- `event_stream_bus::memory::tests::subscriber_receives_published_run_event` passed.
- `event_stream_bus::memory::tests::subscriber_ignores_other_runs` passed.
- `event_stream_bus::nats::tests::subject_for_run_id_and_event_type` passed.
- `event_stream_bus::nats::tests::message_id_uses_run_id_and_seq` passed.
- `event_stream_bus::nats::tests::subscribe_subject_uses_run_wildcard` passed.
- `tests/event_stream_bus_nats.rs::nats_event_stream_bus_publishes_and_subscribes_run_events` ignored (requires wyse-infra-test NATS container).

Concerns:
- `new(0)` now forwards zero capacity to Tokio broadcast. This follows the reviewer request to keep caller-provided capacity unchanged; Tokio panics for zero capacity, so callers must pass a non-zero value if runtime safety is required.
