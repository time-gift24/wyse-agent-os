# `wyse-agent-builtin` simple agent design

## Goal

Add `wyse-agent-builtin`, an internal wiring crate with room for several
executable entry points. The first executable, `simple_agent`, runs one
prompt against an OpenAI or DeepSeek model and writes the complete runtime
event stream to stdout as NDJSON. It provides a small, real end-to-end smoke
test of the existing agent loop.

## Scope

`simple_agent` is a one-shot command:

```sh
API_KEY=... MODEL=openai:gpt-4.1-mini \
  cargo run -p wyse-agent-builtin --bin simple_agent -- "hello"
```

It accepts exactly one prompt argument. `API_KEY` and `MODEL` are required
environment variables. There are no provider-specific model variables.

The initial provider set is:

- `openai:<model>`: the official OpenAI endpoint, using the existing
  OpenAI-compatible transport.
- `deepseek:<model>`: the official DeepSeek endpoint, using the existing
  DeepSeek transport. Only models already represented by `DeepSeekModel` are
  accepted.

The default agent has no tools, no checkpoint store, no REPL, and no custom
endpoint configuration. Its system prompt is the fixed, minimal helpful-agent
prompt `You are a helpful assistant.` required by `AgentBuilder`.

## Global model identity

`ModelRef` is the global model identity. Its only textual representation is
`provider:model`, such as `openai:gpt-4.1-mini` and
`deepseek:deepseek-v4-flash`.

`wyse-core` owns the validated `ModelRef` newtype and its parse error. It
rejects empty segments, whitespace-surrounded values, and extra separators.
It exposes the provider and the provider-local `ModelId`. `ModelId` remains
the exact name sent to the upstream API; it is not replaced by `ModelRef`.

`MODEL` is parsed to `ModelRef` at the binary boundary. The default-agent
wiring dispatches on its provider and passes only the contained `ModelId` to
the selected transport.

Runtime metadata must continue to represent the canonical identity. The
existing `llm` metadata value is therefore always equal to the original
`ModelRef` string. The OpenAI wiring configures the existing compatible
transport to report provider name `openai`, rather than its generic
`openai_compatible` default; DeepSeek already reports `deepseek`.

## Components

The new crate contains only these production files:

```text
crates/wyse-agent-builtin/
  Cargo.toml
  src/lib.rs
  src/default_agent.rs
  src/error.rs
  src/bin/simple_agent.rs
```

`default_agent` provides one fallible function that receives an event bus, an
`ApiKey`, and a `ModelRef`, then returns the configured `Agent`. It matches
only `openai` and `deepseek`; unsupported providers or unsupported DeepSeek
models return typed crate errors. It creates an empty `BuiltinToolRegistry`.
No provider trait, registry, factory, or configuration layer is added.

`simple_agent` owns CLI-only work: environment lookup, prompt validation,
in-memory bus creation, run start, subscription, NDJSON output, and process
exit status. Future built-in executables live beside it under `src/bin/` and
reuse `default_agent` when appropriate.

The existing generic OpenAI-compatible provider gets the smallest required
addition: an explicit provider-name setting, defaulting to
`openai_compatible`. The built-in OpenAI path sets it to `openai`; other
existing uses keep their current identity.

## Event flow and output

1. `simple_agent` loads `API_KEY`, parses `MODEL` as `ModelRef`, and reads the
   prompt argument.
2. It creates an `InMemoryEventStreamBus`, then builds the default agent with
   the selected provider and an empty tool registry.
3. It calls `Agent::run_turn(ChatMessage::user(prompt))` and subscribes to the
   returned run id. The in-memory bus retains earlier events, so subscribing
   after starting cannot lose the first event.
4. For every received envelope, it serializes the complete `StreamEnvelope`
   to stdout as one JSON line and flushes it. This intentionally includes
   text deltas, reasoning deltas, metadata, lifecycle events, and errors.
5. On the terminal agent event, it exits: `finished` succeeds; `failed` and
   `cancelled` return a non-zero process status after their event has been
   emitted.

The binary does not reformat or filter events. Consumers can use `jq` or a
later executable to render a friendlier view while preserving this raw smoke
test as the protocol reference.

## Failures and secrets

Missing `API_KEY`, missing `MODEL`, an invalid model reference, unsupported
provider/model, or an absent prompt fail before a run starts and print a
concise stderr error. Provider failures are emitted through the normal event
stream and then cause non-zero exit.

API keys are never included in envelopes or stderr. Existing LLM error
redaction remains the only provider-error text source.

## Verification

Automated tests remain offline:

- `wyse-core` unit tests cover `ModelRef` parsing, display, and invalid input.
- `wyse-agent-builtin` unit tests cover OpenAI and DeepSeek provider wiring,
  including the reported provider and model identity before agent events are
  published.
- `simple_agent` unit tests cover prompt argument cardinality, including
  missing and extra prompt arguments.

Before handoff, run `cargo fmt`, `cargo test --workspace --all-targets`, and
`cargo clippy --workspace --all-targets`. A manual smoke test uses a real
`API_KEY` and `MODEL`; it is deliberately not part of the automated suite.

## Non-goals

- Generic provider plugins or a provider registry.
- Tools, filesystem access, checkpointing, retries, or a conversational REPL.
- Config files, dotenv support, model aliases, or endpoint overrides.
- Human-readable event rendering. NDJSON is the first binary's contract.
