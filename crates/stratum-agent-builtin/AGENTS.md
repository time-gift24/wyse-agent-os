# stratum-agent-builtin

- This crate is library-only; composition roots own configuration and invocation.
- Callers inject the `AgentId`, `Arc<dyn AgentStore>`,
  `Arc<dyn EventStreamBus>`, and `Arc<dyn LlmProvider>` into
  `build_default_agent`.
- The helper performs no filesystem-backend or retained-log composition; Web
  supplies the same logical Agent store used by its `StoreEventStreamBus`.
- `ModelId` is always `provider:model`.
- Keep provider dispatch concrete in `default_agent`; add a registry only when the direct match no longer suffices.
- `stratum_repl` is an explicitly approved local-validation composition root. It accepts
  configuration only from `config.toml`, persists turn events through
  `StoreEventStreamBus`, and accepts `--resume` only for an exact existing agent ID.
- `stratum_repl` registers only `EchoTool` in `RequireApproval` mode to validate the approval
  flow, prompting for `approve` or `reject` for every tool call.
