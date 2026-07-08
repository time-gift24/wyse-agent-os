# Task 2 Report

Status: DONE

Files changed:
- `Cargo.lock`
- `crates/wyse-llm/Cargo.toml`
- `crates/wyse-llm/src/definition.rs`
- `crates/wyse-llm/src/message.rs`
- `crates/wyse-llm/src/mock.rs`
- `crates/wyse-llm/src/protocol/deepseek.rs`
- `crates/wyse-llm/src/protocol/openai_compatible.rs`
- `crates/wyse-llm/src/tool_call.rs`
- `crates/wyse-llm/tests/deepseek_provider.rs`
- `crates/wyse-llm/tests/openai_compatible_provider.rs`

Commit:
- `3bcdf7d` `feat: expose llm provider names`

Tests run:
- `cargo test -p wyse-llm provider_reports_provider_name` initially failed as expected before implementation, then passed after the trait/provider changes
- `cargo test -p wyse-llm --all-targets` passed
- `cargo fmt --all --check` initially reported formatting drift, then `cargo fmt --all` was applied and the code formatted cleanly
- `cargo clippy --workspace --all-targets` passed

Self-review notes:
- Kept the change localized to `wyse-llm` and the lockfile entry required by `async-trait`
- Re-exported `ChatContent`, `ChatMessage`, `ChatRole`, `ToolCall`, and `ToolCallDelta` from `wyse-core` so the public `wyse-llm` API stays intact
- Made `LlmProvider` object-safe with `async_trait` and added stable provider names for mock, OpenAI-compatible, and DeepSeek implementations

Concerns:
- `message_to_value` now matches the non-exhaustive core enums with `unreachable!()` fallback arms; that is fine for current core variants, but any future `wyse-core` additions will need a matching update here

## Review Fix (2026-07-09)

Status: DONE

Fix applied:
- Updated `crates/wyse-llm/src/protocol/openai_compatible.rs` in `message_to_value`.
- Replaced both `_ => unreachable!()` arms for `ChatRole` and `ChatContent` with `Err(LlmError::UnsupportedCapability(...))` returns.
- This ensures unsupported/future core enum variants fail as typed errors instead of panicking while preserving the existing function signature (`Result<Value, LlmError>`).

Tests run:
- `cargo test -p wyse-llm --all-targets`
- Result: **all tests passed** (unit tests and integration tests under `wyse-llm`, with expected smoke tests ignored due to missing env vars).

Concerns:
- No additional tests added for synthetic future enum variants because they are not constructible via current public API without unsafe/fabricated data; behavior is now covered via typed error path for fallback matching.
