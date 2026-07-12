# Direct Chat Stream Creation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Await provider stream creation directly without polling cancellation or commands first.

**Architecture:** Keep error handling and downstream stream consumption unchanged. Delete only the redundant pre-stream `pin!` and `select!` loop plus its obsolete behavior test.

**Tech Stack:** Rust, Tokio, Cargo.

## Global Constraints

- Do not add APIs, helpers, dependencies, or compatibility behavior.
- Do not commit this plan or its design document.

---

### Task 1: Directly await stream creation

**Files:**
- Modify: `crates/wyse-agent/src/loop.rs:69-84`
- Test: `crates/wyse-agent/tests/streaming_loop.rs:868-917`

**Interfaces:**
- Consumes: `LlmProvider::chat_stream(ChatRequest) -> Result<ChatStream, LlmError>`
- Produces: unchanged `Agent::run_turn_loop` behavior after stream creation

- [ ] **Step 1: Remove the obsolete test**

Delete `stream_publishes_cancelled_when_provider_stream_creation_hangs`; the
new behavior intentionally waits for provider stream creation.

- [ ] **Step 2: Simplify production code**

Replace the pinned future and pre-stream `tokio::select!` loop with:

```rust
let stream = self.llm_provider.chat_stream(request).await;
```

Keep the following `match stream` error publication unchanged.

- [ ] **Step 3: Verify the crate**

Run:

```bash
cargo fmt --all -- --check
cargo test -p wyse-agent
cargo clippy -p wyse-agent --all-targets -- -D warnings
```

Expected: all commands exit successfully.

- [ ] **Step 4: Commit and push**

```bash
git add crates/wyse-agent/src/loop.rs crates/wyse-agent/tests/streaming_loop.rs
git commit -m "refactor(agent): await chat stream creation directly"
git push
```
