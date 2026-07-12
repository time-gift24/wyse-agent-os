# Direct Chat Stream Creation Design

## Goal

Remove the pre-stream cancellation and command polling wrapper around
`LlmProvider::chat_stream`.

## Behavior

`run_turn_loop` awaits `chat_stream(request)` directly and preserves the
existing provider-error event publication. Cancellation and turn commands are
handled only after the provider returns a stream, by the existing stream and
tool-execution loops.

The test asserting cancellation while stream creation hangs is removed because
that behavior is intentionally no longer supported.

## Scope

- Modify `crates/wyse-agent/src/loop.rs`.
- Remove the obsolete integration test from
  `crates/wyse-agent/tests/streaming_loop.rs`.
- Add no new APIs, helpers, dependencies, or compatibility behavior.
