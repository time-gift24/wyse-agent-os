# Stratum REPL file-store milestone design

## Goal

Provide a local `stratum-repl` executable in `wyse-agent-builtin` for manually
validating a multi-turn conversation's filesystem persistence. The executable
prints the conversation through `EventStreamBus`; the operator then inspects
the generated agent directory and JSON records.

The repository and existing Rust packages keep their `wyse-*` names in this
milestone. `stratum` is used for the new binary, user-facing messages, and
configuration namespace only.

## Configuration and command line

`config.toml` is the sole source for the model, provider credentials, and
storage location. It is not committed. `config.example.toml` documents the
minimal secret-free shape:

```toml
[stratum]
storage_root = "./.stratum/repl"
model = "deepseek:deepseek-v4-flash"

[deepseek]
api_key = "..."
```

The configuration rejects unknown fields. Provider configuration supports the
existing OpenAI-compatible and DeepSeek paths, while `stratum.model` selects
the one provider/model to use for this invocation.

The executable uses `clap`. Its only application option is
`--resume <agent-id>`; standard `--help` is supplied by clap. There is no model
or storage-path command-line option.

## Composition and persisted layout

The existing `build_default_agent` library API remains an injection-only
helper. The new binary is the composition root and constructs:

1. `LocalFilesystem`, rooted at the configured host directory. The binary
   creates this root if necessary.
2. One `FilesystemAgentStore` per agent ID, rooted at that ID's virtual
   subdirectory.
3. `StoreEventStreamBus`, decorating an `InMemoryEventStreamBus`.
4. The default agent using the selected LLM provider, store, and decorated
   event bus.

For a new session, the binary generates an `AgentId`, initializes the store,
and prints both that ID and the host storage path. The resulting layout is
`<storage_root>/<agent-id>/agent.json` plus
`<storage_root>/<agent-id>/messages/<sequence>.json`.

For `--resume <agent-id>`, the binary opens that exact existing store. It never
creates a missing store or overwrites its data.

## Conversation restoration

`Agent::resume()` retains its current, narrow responsibility: continuing an
interrupted persisted turn whose store status is `running`.

`wyse-agent` receives one small asynchronous public API that loads and
validates complete persisted conversation history into an inactive agent. It
checks agent identity and obtains history through the store's paginated API;
the store remains the authority for sequence and record validation. It does
not expose agent internals or duplicate crash-resume logic.

When `--resume` finds a `running` agent, the REPL subscribes and calls the
existing crash-resume path before accepting input. For `idle`, `finished`,
`failed`, or `cancelled` agents, it loads history through the new API and then
allows the next ordinary user turn. Thus any existing conversation can be
continued, while an interrupted run keeps its recovery semantics.

## REPL and event output

The REPL accepts one user line at a time. Empty lines are ignored; `/quit` and
EOF exit normally. No additional control commands belong to this milestone.

For each normal or crash-resumed turn, the REPL subscribes to new events before
starting the work. `StoreEventStreamBus` commits relevant state and complete
messages before forwarding events to the in-memory bus. The terminal consumes
that subscription until `Finished`, `Failed`, or `Cancelled`.

Default output displays the assistant's complete final text. `--debug` also
writes every received `StreamEnvelope` as one NDJSON line, preserving metadata,
event type, run ID, timestamp, and message sequence for manual correlation
with filesystem files. No output path reads result data directly from the
agent or store.

Unrecoverable configuration, CLI parsing, provider, identity, filesystem, or
store errors terminate with a typed error chain. A turn-level failure or
cancellation is displayed and returns to the prompt after its terminal event.

## Verification

Tests use mock LLM providers and temporary filesystem roots; they do not use
live provider credentials.

- `wyse-agent` tests prove completed histories load into a new agent and are
  included in the next model request; they also cover missing, wrong-agent, and
  corrupt persisted history.
- REPL tests cover strict configuration parsing, clap's `--resume` handling,
  session path selection, and default versus debug event rendering.
- A multi-turn local filesystem test verifies `agent.json` and
  `messages/*.json` sequences match the complete events received by the REPL
  subscription.

Before delivery, run formatting, the relevant workspace tests, and Clippy.
The operator's manual acceptance check is a multi-turn `stratum-repl` session
followed by inspection of its printed NDJSON and the matching persisted files.
