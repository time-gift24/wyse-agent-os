# Agent Store Rename Implementation Plan

> **For agentic workers:** Execute each task in order. Preserve local documentation as unstaged work.

**Goal:** Replace checkpoint naming with store naming across the persistence crate without changing behavior.

**Architecture:** `Agent -> EventStreamBus`; Web injects `StoreEventStreamBus`, which wraps `AgentStore` and an inner retained bus. `FilesystemAgentStore` persists `agent.json` and `messages/{seq}.json` through the mounted filesystem CAS API.

**Tech Stack:** Rust 2024, Cargo workspace, Tokio, async-trait, mounted filesystem CAS, async-nats JetStream.

## Global Constraints

- No compatibility aliases, deprecated re-exports, migration, or duplicate crate.
- Do not change storage layout, event protocol, CAS behavior, or recovery ordering.
- Do not add dependencies.
- Keep `TODO.md`, all `AGENTS.md`, and `docs/superpowers/` unstaged and uncommitted.
- Commit only tracked code/manifests/tests belonging to the rename.

### Task 1: Rename the crate and public API

- [ ] Move `crates/wyse-checkpoint` to `crates/wyse-store`.
- [ ] Rename the package/dependency/import forms to `wyse-store` and `wyse_store`.
- [ ] Rename `AgentCheckpoint`, `FilesystemAgentCheckpoint`, `CheckpointEventStreamBus`, and `CheckpointError` to the approved store names.
- [ ] Rename internal fields, variables, logs, comments, and `filesystem_checkpoint.rs`.
- [ ] Run `cargo check --workspace --all-targets` and fix only rename fallout.

### Task 2: Remove stale terminology

- [ ] Update local `TODO.md`, crate `AGENTS.md`, and superpowers documents to store terminology without staging them.
- [ ] Run `rg -n "wyse-checkpoint|wyse_checkpoint|AgentCheckpoint|FilesystemAgentCheckpoint|CheckpointEventStreamBus|CheckpointError" Cargo.toml Cargo.lock crates .github` and require no matches.
- [ ] Inspect remaining lowercase `checkpoint` matches; remove those that describe this store while retaining unrelated third-party or historical Git metadata only if outside tracked project content.

### Task 3: Verify behavior

- [ ] Run `cargo fmt --all -- --check`.
- [ ] Run `cargo test --workspace --all-targets`.
- [ ] Run `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `make -C crates/wyse-infra test-integration COMPOSE="podman compose"` locally; CI uses the same target with Docker.
- [ ] Run `git diff --check` and confirm the index contains no local documentation.

### Task 4: Update the pull request

- [ ] Stage only the crate rename, manifests, lockfile, and tracked Rust tests/code.
- [ ] Commit with `refactor(store): rename checkpoint persistence crate`.
- [ ] Push `codex/agent-checkpoint-retained-log`.
- [ ] Monitor PR #17 until fmt/Clippy/test, image, and integration checks complete.
