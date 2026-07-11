# Stratum REPL File-Store Milestone Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `stratum-repl`, a configurable multi-turn local REPL that persists each conversation to the filesystem and can reopen any previous agent ID.

**Architecture:** Keep `wyse-agent-builtin` as an injection-only library and add the executable as its composition root. The executable creates a `LocalFilesystem`, one `FilesystemAgentStore` per agent ID, and a `StoreEventStreamBus` over the in-memory bus; it consumes that bus for terminal output. Add one `Agent::load_history` API for terminal sessions so a non-running store can restore durable history without changing crash-resume behavior.

**Tech Stack:** Rust 2024/MSRV 1.88, Tokio, clap derive, serde/toml, `thiserror`, `LocalFilesystem`, `FilesystemAgentStore`, `StoreEventStreamBus`.

## Global Constraints

- Keep existing `wyse-*` package names unchanged; use `stratum` only in new user-facing names and configuration.
- Use a new `codex/` worktree branch; do not commit to `main`.
- Do not add a wrapper, registry, or persistence abstraction beyond the one history-load method required here.
- `config.toml` contains credentials and must be ignored; only a secret-free `config.example.toml` is committed.
- `--resume <agent-id>` never initializes, overwrites, or substitutes a missing store.
- `Agent::resume()` remains reserved for persisted `running` turns; terminal state restoration uses `Agent::load_history()`.
- Default terminal output is assistant text; `--debug` writes full `StreamEnvelope` NDJSON from the event-bus subscription.
- Production Rust uses typed errors, no `unwrap()`, and public fallible APIs document `# Errors`.
- Before handoff run `cargo fmt`, focused tests, `cargo test --workspace --all-targets`, and `cargo clippy --workspace --all-targets`.

---

## File Structure

- `crates/wyse-agent/src/definition.rs` — add and document durable terminal-history loading while preserving the existing crash-resume path.
- `crates/wyse-agent/tests/streaming_loop.rs` — prove an agent restored from terminal history sends that history with its next LLM request.
- `Cargo.toml` — declare the shared CLI/config dependencies once at workspace level.
- `crates/wyse-agent-builtin/Cargo.toml` — add the executable's inherited dependencies and `wyse-filesystem`.
- `crates/wyse-agent-builtin/src/bin/stratum_repl.rs` — private executable composition, strict configuration, REPL driver, event renderer, typed errors, and unit tests.
- `.gitignore` — ignore root `config.toml`.
- `config.example.toml` — document the minimal secret-free Stratum configuration.
- `crates/wyse-agent-builtin/AGENTS.md` — archive the executable's narrow composition and persistence contract after implementation.

### Task 1: Restore a completed conversation into an inactive agent

**Files:**
- Modify: `crates/wyse-agent/src/definition.rs:3-19, 209-301`
- Modify: `crates/wyse-agent/tests/streaming_loop.rs:344-357, after terminal resume tests`

**Interfaces:**
- Consumes: `AgentStore::load_agent() -> Result<AgentState, StoreError>` and `AgentStore::history_page(HistoryQuery) -> Result<HistoryPage, StoreError>`.
- Produces: `pub async fn Agent::load_history(&self) -> Result<(), AgentError>`.
- Preserves: `pub async fn Agent::resume(&self) -> Result<RunId, AgentError>` and its `running`-only contract.

- [ ] **Step 1: Write the failing restoration integration test**

  In `crates/wyse-agent/tests/streaming_loop.rs`, add a `RecordingProvider` response containing one assistant text delta and `FinishReason::Stop`. Build a terminal `AgentState` with `status = AgentStatus::Finished`, `last_seq = 2`, and persisted user/assistant envelopes. Build an agent with that store, call `load_history()`, run a new user turn, wait for `Finished`, then assert its first recorded request is exactly:

  ```rust
  vec![
      ChatMessage::system("be helpful"),
      ChatMessage::user("first question"),
      ChatMessage::assistant("first answer"),
      ChatMessage::user("second question"),
  ]
  ```

  Add a second test where the persisted envelope uses a different `AgentId` and assert `load_history()` returns `AgentError::ResumeAgentMismatch` before the provider receives a request. Add a third test with `last_seq = 1` but an envelope `business_seq = Some(2)`; assert `load_history()` returns `AgentError::InvalidResumeHistory` and the subsequent `run_turn` request contains only its new input, proving no partial history was committed.

- [ ] **Step 2: Run the new tests and verify failure**

  Run:

  ```bash
  cargo test -p wyse-agent --test streaming_loop load_history -- --nocapture
  ```

  Expected: compilation failure because `Agent::load_history` does not yet exist.

- [ ] **Step 3: Implement the minimal atomic history-load path**

  In `crates/wyse-agent/src/definition.rs`, add a documented public method that reserves `self.active` with `compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)`, keeps an `ActiveGuard` armed to clear it on every exit, and only commits history after all pages validate:

  ```rust
  /// Loads the durable complete message history into this inactive agent.
  ///
  /// # Errors
  ///
  /// Returns an error when an operation is active, the store identity differs,
  /// or the persisted history cannot be read as contiguous agent messages.
  pub async fn load_history(&self) -> Result<(), AgentError> {
      if self
          .active
          .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
          .is_err()
      {
          return Err(AgentError::RunAlreadyActive);
      }
      let _active_guard = ActiveGuard::new(&self.active);
      let state = self.store.load_agent().await?;
      if state.agent_id != self.id {
          return Err(AgentError::ResumeAgentMismatch {
              expected: self.id,
              actual: state.agent_id,
          });
      }
      let history = self.load_complete_history(state.last_seq).await?;
      self.commit_history(history);
      Ok(())
  }
  ```

  Add the private `load_complete_history(&self, last_seq: u64) -> Result<Vec<ChatMessage>, AgentError>` beside `initialize_resume`. Page from `after_seq = 0` through the fixed `last_seq` barrier using `MAX_HISTORY_PAGE_SIZE`; reject an empty/non-advancing page, a non-contiguous `business_seq`, a non-agent event, a mismatched `agent_id`, or a non-`AgentEvent::Message` event with `InvalidResumeHistory`. Push only message payloads. Do not call `load_agent` again inside the helper and do not modify `resume`.

- [ ] **Step 4: Run the restoration tests and the agent suite**

  Run:

  ```bash
  cargo test -p wyse-agent --test streaming_loop load_history -- --nocapture
  cargo test -p wyse-agent
  ```

  Expected: both commands pass; the new request includes the two restored messages plus the new input, a mismatched ID performs no LLM request, and malformed history is rejected without changing in-memory history.

- [ ] **Step 5: Commit the isolated runtime change**

  ```bash
  git add crates/wyse-agent/src/definition.rs crates/wyse-agent/tests/streaming_loop.rs
  git commit -m "feat(agent): load persisted terminal history"
  ```

### Task 2: Add the strict Stratum CLI configuration and filesystem composition root

**Files:**
- Modify: `Cargo.toml:15-40`
- Modify: `crates/wyse-agent-builtin/Cargo.toml:7-14`
- Create: `crates/wyse-agent-builtin/src/bin/stratum_repl.rs`
- Modify: `.gitignore:1-4`
- Create: `config.example.toml`

**Interfaces:**
- Consumes: `build_default_agent(AgentId, Arc<dyn AgentStore>, Arc<dyn EventStreamBus>, Arc<dyn LlmProvider>)`.
- Consumes: `FilesystemAgentStore::new(Arc<dyn Filesystem>, VirtualPath)` and `initialize(AgentId, String)`.
- Produces: `stratum-repl [--resume <agent-id>] [--debug]`, `Config::read() -> Result<Config, ReplError>`, `async fn compose_session(config: &Config, agent_id: AgentId, initialize: bool) -> Result<Session, ReplError>`, and `Session { agent_id: AgentId, agent: Agent, bus: Arc<dyn EventStreamBus>, storage_root: PathBuf }`.

- [ ] **Step 1: Write failing binary unit tests for CLI/config/path behavior**

  At the end of `stratum_repl.rs`, add tests that parse:

  ```rust
  let args = Args::try_parse_from(["stratum-repl", "--resume", &agent_id.to_string(), "--debug"])?;
  assert_eq!(args.resume, Some(agent_id));
  assert!(args.debug);
  ```

  Add config tests that accept the minimal `[stratum]` plus `[openai]` document and reject an extra `[stratum] unexpected = true` key. Add a provider-selection test asserting `custom:model` returns the executable's unsupported-provider error without issuing a network call. Add an `agent_root(agent_id)` test asserting the virtual path is `format!("/{agent_id}")`.

- [ ] **Step 2: Run binary tests and verify failure**

  Run:

  ```bash
  cargo test -p wyse-agent-builtin --bin stratum_repl
  ```

  Expected: Cargo reports that binary target `stratum_repl` does not exist.

- [ ] **Step 3: Add only the required dependencies and typed composition**

  Add `clap` (derive feature) and `toml` to the root workspace dependency table. Consume those plus the already workspace-managed `futures-util`, `serde`, `serde_json`, `thiserror`, and `tokio` from `wyse-agent-builtin`, and add its missing `wyse-filesystem` workspace dependency. Do not add a dependency that the workspace already provides.

  In `stratum_repl.rs`, define the following private types and constants:

  ```rust
  const CONFIG_PATH: &str = "config.toml";
  const DEFAULT_AGENT_NAME: &str = "default-agent";
  const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
  const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";

  #[derive(clap::Parser)]
  #[command(name = "stratum-repl")]
  struct Args { #[arg(long)] resume: Option<AgentId>, #[arg(long)] debug: bool }

  #[derive(serde::Deserialize)]
  #[serde(deny_unknown_fields)]
  struct Config { stratum: StratumConfig, openai: Option<ProviderConfig>, deepseek: Option<ProviderConfig> }

  #[derive(serde::Deserialize)]
  #[serde(deny_unknown_fields)]
  struct StratumConfig { storage_root: PathBuf, model: ModelId }

  #[derive(serde::Deserialize)]
  #[serde(deny_unknown_fields)]
  struct ProviderConfig { api_key: String }
  ```

  `ReplError` wraps I/O, TOML, model parsing, `AgentError`, `StoreError`, `FilesystemError`, `EventStreamBusError`, `LlmError`, JSON encoding, and unsupported provider/model errors with lower-case messages. Select the provider directly from `stratum.model`: construct `OpenAICompatibleProvider` for `openai`, and map only `deepseek-v4-flash`/`deepseek-v4-pro` to the existing `DeepSeekModel` variants. Never log an API key.

  `compose_session` must create `storage_root` with `std::fs::create_dir_all`, construct `LocalFilesystem`, set the store root to `agent_root(agent_id)`, and use `StoreEventStreamBus::new(store.clone(), Arc::new(InMemoryEventStreamBus::default()))`. For a new ID, call `store.initialize(agent_id, DEFAULT_AGENT_NAME.to_owned()).await` before building the agent. For a resume ID, call only `store.load_agent().await`; propagate `AgentMissing` unchanged.

  Add `config.toml` to `.gitignore` and commit this exact example without secrets:

  ```toml
  [stratum]
  storage_root = "./.stratum/repl"
  model = "openai:gpt-4.1-mini"

  [openai]
  api_key = "replace-with-your-api-key"
  ```

- [ ] **Step 4: Run formatting and configuration tests**

  Run:

  ```bash
  cargo fmt --check
  cargo test -p wyse-agent-builtin --bin stratum_repl
  ```

  Expected: formatting and all CLI/config/path tests pass; no test opens a network connection.

- [ ] **Step 5: Commit the executable composition baseline**

  ```bash
  git add Cargo.toml Cargo.lock crates/wyse-agent-builtin/Cargo.toml crates/wyse-agent-builtin/src/bin/stratum_repl.rs .gitignore config.example.toml
  git commit -m "feat(builtin): add stratum repl composition"
  ```

### Task 3: Drive turns through the bus and verify on-disk event consistency

**Files:**
- Modify: `crates/wyse-agent-builtin/src/bin/stratum_repl.rs`
- Modify: `crates/wyse-agent-builtin/AGENTS.md`

**Interfaces:**
- Consumes: `EventStreamBus::subscribe_agent(agent_id, ReplayStart::New) -> EventStream`.
- Consumes: `Agent::run_turn(ChatMessage) -> Result<RunId, AgentError>`, `Agent::resume()`, and `Agent::load_history()`.
- Produces: `async fn consume_turn_events<W: Write>(events: &mut EventStream, debug: bool, output: &mut W) -> Result<(), ReplError>`, `async fn drive_turn<W: Write>(session: &Session, input: &str, debug: bool, output: &mut W) -> Result<(), ReplError>`, and `async fn restore_session<W: Write>(session: &Session, debug: bool, output: &mut W) -> Result<(), ReplError>`.

- [ ] **Step 1: Write failing turn-driver tests with a local filesystem and mock LLM**

  In the binary's test module, create a unique directory under `std::env::temp_dir()` using `AgentId::new()`, create it, and build `LocalFilesystem`, `FilesystemAgentStore`, `StoreEventStreamBus`, and an agent with `MockLlmProvider` queued with two text responses. Subscribe and drive two input turns through the same helper using a `Vec<u8>` writer.

  Assert that default output contains both assistant texts but contains no JSON line. Repeat the first turn with `debug = true`; deserialize each output line beginning with `{` as a `StreamEnvelope` and assert it contains the same assistant message event. Finally load `agent.json` and `messages/1.json` through `messages/4.json` from the temp directory and assert: `last_seq == 4`, each message's `business_seq` is `Some(1..=4)`, and the assistant `StreamEnvelope`s equal those emitted to the debug writer.

  Add a restoration test that initializes a finished store, calls `restore_session`, drives one new mock response, and verifies the existing directory advances without replacement. Keep crash-resume covered by an explicit `AgentStatus::Running` branch that calls `Agent::resume()`.

- [ ] **Step 2: Run the tests and verify failure**

  Run:

  ```bash
  cargo test -p wyse-agent-builtin --bin stratum_repl drive_turn -- --nocapture
  ```

  Expected: failure because the REPL turn driver and renderer are not implemented.

- [ ] **Step 3: Implement the REPL loop, restoration branch, and renderer**

  Add `restore_session` that loads `AgentState` once and chooses exactly:

  ```rust
  if state.status == AgentStatus::Running {
      let mut events = session.bus.subscribe_agent(session.agent_id, ReplayStart::New).await?;
      session.agent.resume().await?;
      consume_turn_events(&mut events, debug, output).await
  } else {
      session.agent.load_history().await?;
      Ok(())
  }
  ```

  For each submitted user line, subscribe with `ReplayStart::New` before calling `agent.run_turn(ChatMessage::user(line))`. Consume `EventRecord`s until a terminal `AgentEvent`. For every record, write `record.envelope` as one JSON line when debug is enabled. In default output, write only `AgentEvent::Message` values with `message.role == ChatRole::Assistant`; print `ChatContent::Text` directly and serialize `ChatContent::Json` as one line. Print `Failed`/`Cancelled` diagnostics to stderr and return to the prompt; propagate configuration, store, identity, subscription, and serialization errors.

  The outer loop uses `std::io::BufRead::read_line`, ignores `trim().is_empty()`, exits on `/quit` or zero bytes (EOF), and flushes the prompt/output. It prints the generated or resumed ID and host storage root once at startup. Do not read conversation result data from the store for display.

  Archive the stable contract in `crates/wyse-agent-builtin/AGENTS.md`: the binary is an explicitly approved local validation composition root; it receives configuration only from `config.toml`, writes through `StoreEventStreamBus`, and uses `--resume` only for an exact existing agent ID.

- [ ] **Step 4: Run focused and workspace verification**

  Run:

  ```bash
  cargo fmt --check
  cargo test -p wyse-agent-builtin --bin stratum_repl
  cargo test --workspace --all-targets
  cargo clippy --workspace --all-targets
  ```

  Expected: all commands exit 0. The binary tests verify both human output and NDJSON/file consistency without a live provider.

- [ ] **Step 5: Commit the verified REPL behavior and archived contract**

  ```bash
  git add crates/wyse-agent-builtin/src/bin/stratum_repl.rs crates/wyse-agent-builtin/AGENTS.md
  git commit -m "feat(builtin): persist stratum repl conversations"
  ```

## Final Manual Acceptance Check

- [ ] Create a local `config.toml` from `config.example.toml` with a real provider key and a chosen supported model.
- [ ] Run `cargo run -p wyse-agent-builtin --bin stratum_repl -- --debug`, complete two ordinary user turns, and record the printed agent ID.
- [ ] Inspect `<storage_root>/<agent-id>/agent.json` and every `messages/<seq>.json`; verify their contiguous sequence and message content match the emitted debug envelopes.
- [ ] Run `cargo run -p wyse-agent-builtin --bin stratum_repl -- --resume <agent-id>`, submit another user turn, and verify the provider receives prior context and the same directory advances without replacement.
