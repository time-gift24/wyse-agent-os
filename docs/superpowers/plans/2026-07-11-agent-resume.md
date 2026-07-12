# Agent Resume Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `Agent::resume()` that restores an unfinished persisted turn and continues it without a new user message.

**Architecture:** `AgentBuilder` injects `AgentStore` beside `EventStreamBus`. `agent.json.next_iteration` is advanced only after complete messages for one iteration are durably committed; resume reconstructs history from the store and reconciles at that stable boundary before continuing the existing run and turn.

**Tech Stack:** Rust, Tokio, serde, `wyse-agent`, `wyse-store`, mounted `wyse-filesystem`, and the existing event bus.

## Global Constraints

- `Agent::resume()` takes no `Message` and returns the persisted `RunId`.
- Resume reuses the `run_id` and `turn_id` stored in `agent.json`.
- `AgentBuilder` injects `Arc<dyn AgentStore>`; JetStream is not a resume source.
- Web injects the same logical per-Agent store into `Agent` and its
  `StoreEventStreamBus`.
- `agent.json.next_iteration: u64` is the durable next LLM iteration.
- Complete message files are committed before the `agent.json` iteration CAS.
- Realtime text, reasoning, and tool-call deltas remain unsequenced and are never resume state.
- Web or scheduler composition guarantees one writer; do not add a runtime writer lease.
- Tool execution is at-least-once, and each tool implementation must be idempotent for the same tool call.
- Add no migration, compatibility alias, phase field, transaction log, or exactly-once tool protocol.
- Keep `TODO.md`, crate `AGENTS.md` files, and `docs/superpowers/` local and uncommitted.

---

### Task 1: Persist the next iteration with CAS

**Files:**
- Modify: `crates/wyse-store/src/state.rs`
- Modify: `crates/wyse-store/src/definition.rs`
- Modify: `crates/wyse-store/src/filesystem.rs`
- Modify: `crates/wyse-store/src/error.rs`
- Modify: `crates/wyse-store/src/decorator.rs`
- Test: `crates/wyse-store/tests/filesystem_store.rs`
- Test: `crates/wyse-store/tests/decorator.rs`

**Interfaces:**
- Produces: `AgentState::next_iteration: u64`
- Produces:

```rust
async fn complete_iteration(
    &self,
    run_id: RunId,
    turn_id: TurnId,
    iteration: u64,
    usage: TokenUsage,
) -> Result<AgentState, StoreError>;
```

- [ ] **Step 1: Add failing state and filesystem tests**

Add tests proving:

```rust
assert_eq!(AgentState::new(agent_id, "writer".to_owned()).next_iteration, 0);
```

For a running state at iteration zero, `complete_iteration` must return and
persist iteration one plus the supplied usage. Add separate tests proving it
fails without changing the file when status is not running, run/turn identity
does not match, or the expected iteration differs. Add a test proving a new
Started lifecycle event resets an earlier run's iteration to zero and terminal
lifecycle updates preserve the current iteration.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test -p wyse-store --test filesystem_store complete_iteration -- --nocapture
cargo test -p wyse-store --test decorator state_events_commit_matching_status_run_turn_and_usage -- --nocapture
```

Expected: compilation or assertions fail because `next_iteration` and
`complete_iteration` do not exist.

- [ ] **Step 3: Implement the state field and CAS operation**

Add `next_iteration: u64` to `AgentState` and initialize it to zero. Extend the
strict serialized-field test. Add typed store errors for non-running state,
iteration mismatch, and iteration overflow.

Implement `complete_iteration` with `cas_update`. Its apply closure must:

```rust
validate_state(current)?;
// require Running and matching run_id / turn_id
// require current.next_iteration == iteration
next.next_iteration = iteration.checked_add(1).ok_or(StoreError::IterationOverflow)?;
next.usage = usage;
next.updated_at = updated_at;
```

When `update_state` changes to a different running `run_id`, reset
`next_iteration` to zero. Other lifecycle changes preserve it. Do not change
`last_seq` behavior.

- [ ] **Step 4: Verify Task 1**

Run:

```bash
cargo fmt --all -- --check
cargo test -p wyse-store
cargo clippy -p wyse-store --all-targets -- -D warnings
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 5: Commit Task 1**

Commit only store production and test files:

```bash
git commit -m "feat(store): persist agent iteration frontier"
```

### Task 2: Inject the store and resume from a stable boundary

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Modify: `crates/wyse-agent/Cargo.toml`
- Modify: `crates/wyse-agent/src/definition.rs`
- Modify: `crates/wyse-agent/src/loop.rs`
- Modify: `crates/wyse-agent/src/error.rs`
- Modify: `crates/wyse-agent/tests/streaming_loop.rs`
- Modify: `crates/wyse-agent-builtin/Cargo.toml`
- Modify: `crates/wyse-agent-builtin/src/default_agent.rs`

**Interfaces:**
- Consumes: `AgentStore::load_agent`, `AgentStore::history_page`, and `AgentStore::complete_iteration` from Task 1.
- Produces:

```rust
pub fn store(mut self, store: Arc<dyn AgentStore>) -> Self;
pub async fn resume(&self) -> Result<RunId, AgentError>;
```

- Changes `build_default_agent` to accept `Arc<dyn AgentStore>` and inject it.

- [ ] **Step 1: Add failing builder and resume tests**

Use a test `AgentStore` implementation rather than adding test-only production
APIs. Add tests proving:

- `AgentBuilder::build` reports missing `store`.
- `resume()` rejects a second active operation with `RunAlreadyActive`.
- `resume()` rejects non-running state and missing run/turn identity with typed
  `AgentError` variants.
- Given persisted active-turn user history, `resume()` returns the persisted
  run ID, does not publish another Started or user Message event, and starts the
  LLM with the restored conversation history at `next_iteration`.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test -p wyse-agent resume -- --nocapture
cargo test -p wyse-agent-builtin --all-targets
```

Expected: compilation fails because store injection and `resume()` are absent.

- [ ] **Step 3: Add store injection and resume initialization**

Add `store: Arc<dyn AgentStore>` to `Agent` and a required `store` field to
`AgentBuilder`. Add `AgentError::Store` preserving `StoreError` as its source,
plus typed errors for non-running resume state, missing run/turn identity,
agent mismatch, and invalid resume history.

`resume()` must load the fixed history range using pages of
`MAX_HISTORY_PAGE_SIZE` with `through_seq: Some(state.last_seq)`, validate every
envelope as an Agent Message for the built agent, rebuild `Vec<ChatMessage>`,
restore usage and IDs, create cancellation and the bounded command channel,
spawn the continuation task, and return the stored run ID. If initialization
fails after setting `active`, restore `active = false` before returning.

Refactor the new-run path so initial event publication remains in
`run_turn_loop`, while both new and resumed runs enter a shared continuation
starting at an explicit `u64` iteration. Convert to `usize` only when required
by configured limits or metadata, using checked conversion.

After each assistant message and all tool result messages for that iteration
are published successfully, call `complete_iteration` before entering the next
iteration or publishing Finished.

- [ ] **Step 4: Update composition call sites**

Add `wyse-store` dependencies through workspace inheritance. Change
`build_default_agent` to accept the store beside the bus and provider. Update
all existing test builders with their test store. Do not construct a filesystem
backend inside `wyse-agent` or `wyse-agent-builtin`.

- [ ] **Step 5: Verify Task 2**

Run:

```bash
cargo fmt --all -- --check
cargo test -p wyse-agent
cargo test -p wyse-agent-builtin --all-targets
cargo test -p wyse-store
cargo clippy -p wyse-agent -p wyse-agent-builtin --all-targets -- -D warnings
git diff --check
```

Expected: all commands exit zero.

- [ ] **Step 6: Commit Task 2**

Commit only Cargo and production/test files:

```bash
git commit -m "feat(agent): resume persisted running turns"
```

### Task 3: Reconcile committed assistant and tool boundaries

**Files:**
- Modify: `crates/wyse-agent/src/loop.rs`
- Modify: `crates/wyse-agent/src/error.rs`
- Modify: `crates/wyse-agent/tests/streaming_loop.rs`

**Interfaces:**
- Consumes: `Agent::resume()` and `AgentStore::complete_iteration` from Tasks 1-2.
- Produces: no new public API.

- [ ] **Step 1: Add failing crash-window tests**

Add focused tests for these durable histories within the persisted active
`turn_id`:

1. A terminal assistant message is committed while `next_iteration` still
   names that iteration: resume advances the iteration and finishes without an
   LLM request.
2. The terminal assistant iteration is already advanced but Finished state is
   still missing: resume finishes without another advance or LLM request.
3. An assistant tool-call message plus one of multiple tool result messages is
   committed: resume skips the answered call, executes only missing calls,
   commits their result messages, advances exactly once, and continues the LLM
   at the next iteration.
4. The active turn's assistant-message count differs from `next_iteration` by
   more than one: resume fails closed as invalid history.

- [ ] **Step 2: Run focused tests and verify RED**

Run:

```bash
cargo test -p wyse-agent resume -- --nocapture
```

Expected: new crash-window assertions fail because reconciliation is absent.

- [ ] **Step 3: Implement stable-boundary reconciliation**

Before the shared LLM continuation, isolate active-turn messages by persisted
`turn_id` and count assistant messages with checked `u64` conversion.

- If `assistant_count == next_iteration`, either continue the LLM or, when the
  last active-turn message is a terminal assistant, publish Finished with the
  existing unknown finish-reason protocol value and stop.
- If `assistant_count == next_iteration + 1`, find the last assistant message.
  For tool calls, build the set of committed tool `call_id` values after it and
  execute only missing calls in original order. Then call
  `complete_iteration`. Continue the next LLM iteration for tool calls; finish
  immediately for a terminal assistant.
- Otherwise return the typed invalid-resume-history error.

Do not read JetStream, replay deltas, introduce a phase field, or attempt
exactly-once tool execution.

- [ ] **Step 4: Verify Task 3**

Run:

```bash
cargo fmt --all -- --check
cargo test -p wyse-agent
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets -- -D warnings
git diff --check
```

Expected: all commands exit zero; external container tests remain ignored in
the ordinary workspace run.

- [ ] **Step 5: Commit Task 3**

```bash
git commit -m "fix(agent): reconcile resume crash boundaries"
```

### Task 4: Archive the final resume contract locally

**Files:**
- Modify: `crates/wyse-agent/AGENTS.md`
- Modify: `crates/wyse-core/AGENTS.md`
- Modify: `TODO.md` only if a genuinely deferred capability changed

**Interfaces:**
- Consumes: final reviewed implementation from Tasks 1-3.
- Produces: local documentation only; no commit.

- [ ] **Step 1: Update documentation after code is final**

Make the dependency Mermaid show Agent receiving both `AgentStore` and
`EventStreamBus`. Add a resume sequence diagram covering fixed history loading,
same run/turn restoration, pending tool reconciliation, iteration CAS, and
continued event publication. Keep the at-least-once/idempotent-tool trade-off in
`wyse-agent/AGENTS.md`.

Do not turn `TODO.md` into an execution checklist. Keep only capabilities that
remain intentionally unimplemented.

- [ ] **Step 2: Verify documentation and working-tree separation**

Run:

```bash
git diff --check
git diff --cached --name-only
git status --short
```

Expected: no whitespace errors, an empty index after code commits, and local
TODO/AGENTS/docs changes remaining unstaged.

- [ ] **Step 3: Do not commit Task 4**

Leave all documentation changes local for user inspection.
