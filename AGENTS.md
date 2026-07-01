# AGENTS.md

## Project Intent

Wyse Agent OS is a Rust-first agent runtime and workflow orchestration system. Keep the implementation modular, strongly typed, observable, and safe by default.

## Rust Development Rules

Follow the local `rust-skills` guidance in `.agents/skills/rust-skills/SKILL.md` when writing, reviewing, or refactoring Rust code. Prioritize the rules in this order:

1. Ownership and borrowing
2. Error handling
3. Memory optimization
4. Unsafe code
5. API design
6. Async/await
7. Concurrency
8. Type safety
9. Testing
10. Observability

## Workspace Structure

- Use a Cargo workspace with small crates organized by capability.
- Keep crate responsibilities narrow and explicit.
- Prefer module organization by feature, not by generic type buckets.
- Keep `main.rs` thin; put reusable logic in `lib.rs`.
- Use workspace dependency inheritance for shared dependency versions.
- Cargo features must be additive.

## API Design

- Use newtypes for domain IDs such as `RunId`, `AgentId`, `ToolId`, and `ModelId`.
- Avoid stringly typed APIs when enums or validated newtypes communicate intent better.
- Public types should implement the common useful traits where appropriate: `Debug`, `Clone`, `PartialEq`, `Eq`, `Hash`, `Serialize`, and `Deserialize`.
- Prefer `From<T>` over `Into<T>` implementations.
- Use `TryFrom` and `FromStr` for fallible parsing and conversions.
- Mark public enums and structs `#[non_exhaustive]` when future variants or fields are likely.
- Use builders for complex construction and mark builder methods `#[must_use]`.

## Error Handling

- Library crates should use typed errors with `thiserror`.
- Application binaries may use `anyhow` for top-level error handling.
- Return `Result<T, E>` for recoverable failures.
- Do not use `unwrap()` in production code.
- Use `expect()` only for invariants that indicate programmer bugs.
- Preserve error sources with `#[source]` or `From` conversions.
- Error messages should be lowercase and omit trailing punctuation.
- Document fallible public functions with a `# Errors` section.

## Ownership And Memory

- Prefer borrowing over cloning.
- Accept `&str` over `&String` and `&[T]` over `&Vec<T>`.
- Use `Arc<T>` for shared ownership across threads.
- Avoid holding large enum variants inline when boxing materially reduces enum size.
- Preallocate with `with_capacity` when size is known.
- Reuse collections in hot paths instead of repeatedly allocating.
- Avoid `format!` in hot paths when direct writes or literals work.

## Async And Concurrency

- Use Tokio for async runtime code.
- Do not hold `Mutex` or `RwLock` guards across `.await`.
- Use bounded channels for queues and backpressure.
- Use `CancellationToken` for shutdown and run cancellation.
- Use `JoinSet` for managing dynamic spawned tasks.
- Use `spawn_blocking` for CPU-heavy or blocking work.
- Ensure `tokio::select!` branches are cancellation-safe.
- Prefer native `async fn` in traits where it fits the public API.

## Unsafe Code

- Avoid `unsafe` unless there is a clear, measured need.
- Every `unsafe` block must have a `// SAFETY:` comment explaining the invariant.
- Every `unsafe fn` must document a `# Safety` section.
- Keep `unsafe` scopes as small as possible.
- Do not use `mem::uninitialized()` or invalid `mem::zeroed()` patterns.

## Serialization

- Use serde naming conventions that match external payloads, typically `#[serde(rename_all = "snake_case")]` or the protocol-required casing.
- Use `#[serde(default)]` for backward-compatible optional fields.
- Use `skip_serializing_if` for empty optional fields.
- Validate boundary data while deserializing when practical.
- Reject unknown fields for strict config formats where silent typos are dangerous.

## Observability

- Use `tracing` for structured logs and spans.
- Libraries must emit through tracing/log facades and must not install global subscribers.
- Do not log secrets, tokens, raw credentials, or sensitive user data.
- Attach structured fields to spans instead of interpolating context into strings.
- Log an error once at the boundary where it is handled.

## Testing

- Put unit tests in `#[cfg(test)] mod tests`.
- Put cross-crate integration tests in `tests/`.
- Use descriptive test names.
- Structure tests as arrange, act, assert.
- Use `#[tokio::test]` for async tests.
- Use mock providers and trait-based dependencies for agent, LLM, tool, and MCP tests.
- Prefer property tests for parsers, validators, graph scheduling, and schema conversion.

## Linting And Formatting

- Run `cargo fmt` before committing Rust changes.
- Run `cargo clippy --workspace --all-targets` for meaningful Rust changes.
- Configure workspace lints once the workspace skeleton exists.
- Start with correctness, suspicious, style, complexity, and perf lints.
- Do not silence lints without a short reason.

## Documentation

- Document public APIs with `///`.
- Use module-level `//!` docs for crate and module intent.
- Include runnable examples for important public APIs when practical.
- Link related types with intra-doc links.
- Keep examples free of `unwrap()`; use `?` where possible.

## Implementation Style

- Prefer clear, boring Rust over clever abstractions.
- Add abstractions only when they remove real duplication or encode important invariants.
- Keep dependencies explicit and minimal.
- Keep public APIs stable-looking even while internals are evolving.
- Preserve user changes in the working tree; do not revert unrelated files.
