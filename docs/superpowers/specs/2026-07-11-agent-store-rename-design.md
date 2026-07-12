# Agent Store Rename Design

## Status

Approved local design. This document remains uncommitted.

## Goal

Rename the persistence crate and its public API from checkpoint terminology to
store terminology. The implementation is a durable Agent state/message store;
it no longer captures or resumes an Agent loop checkpoint.

## Selected names

- crate directory: `crates/wyse-store`
- Cargo package/dependency: `wyse-store`
- Rust crate: `wyse_store`
- trait: `AgentStore`
- filesystem implementation: `FilesystemAgentStore`
- event-bus decorator: `StoreEventStreamBus`
- domain error: `StoreError`

`AgentState`, `AgentStatus`, `AGENT_STATE_VERSION`, message layout, CAS
behavior, retained-stream behavior, and recovery semantics do not change.

## Scope

Rename manifests, lockfile entries, imports, public types, fields, variables,
logs, comments, test names, test files, and local documentation. Rename
`filesystem_checkpoint.rs` to `filesystem_store.rs`.

There is no deprecated alias, re-export, compatibility crate, migration, or
feature flag. After the change, tracked production and test code contains no
checkpoint terminology.

## Verification

- old-name searches return no tracked code/test/manifest matches;
- `wyse-agent` still depends only on `EventStreamBus`;
- workspace formatting, all-target tests, and Clippy pass;
- the six NATS integration tests pass through the existing sequential/restart
  Makefile target;
- local `TODO.md`, crate `AGENTS.md`, and superpowers documents remain unstaged.
