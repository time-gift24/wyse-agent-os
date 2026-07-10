# wyse-agent-builtin

- `ModelId` is always `provider:model`.
- `simple_agent` reads provider keys and raw model names from `./config.toml`.
  Commit only `config.example.toml`; `/config.toml` remains ignored. Never log keys.
- Binary construction creates concrete configured providers and injects them into
  `LlmProviderManager`; the manager only registers and looks up providers.
- Binaries subscribe through `EventStreamBus` and write complete NDJSON
  `StreamEnvelope` values; do not hide reasoning or metadata.
- Keep provider dispatch concrete in `default_agent`; add a registry only when
  more than the current direct match requires one.
- `simple_agent` is intentionally no-tool and one-shot. Add tools or REPL
  behavior only in a separately approved executable.
