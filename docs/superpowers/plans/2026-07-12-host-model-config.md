# Host Model Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let hosted agents select and persist a complete model configuration, expose configured-model schemas, and apply a selected configuration atomically when a message turn starts.

**Architecture:** `wyse-core` owns the stable `{ model, parameters }` snapshot and `wyse-store` persists it with agent state. `wyse-llm` owns provider-specific schemas, defaults, validation, and configured provider construction. `wyse-api` composes the persisted configuration into a replaceable hosted `Agent`; its store-backed event decorator commits the configuration together with the existing `Started` transition.

**Tech Stack:** Rust 2024 workspace, Tokio, Axum, Serde/serde_json, thiserror, existing filesystem store and provider adapters.

## Global Constraints

- Work only on `codex/host-model-config`, never commit directly to `main`.
- Do not add a JSON Schema validation crate, provider factory layer, or a generic parameter registry.
- Do not introduce names ending in `V2`; advance the existing serialized `state_version` numerically for compatibility.
- `parameters` is an object at the HTTP/persistence boundary; providers parse it into their own strong types.
- Do not log prompts, parameter payloads, API keys, credentials, or provider raw payloads.
- Do not hold `Mutex` or `RwLock` guards across `.await`; retain the registry lock only for map access.
- Run `cargo fmt` and `cargo clippy --workspace --all-targets` before the final handoff.

---

## File Structure

- `crates/wyse-core/src/lib.rs` — add the stable public `ModelConfig` snapshot.
- `crates/wyse-store/src/state.rs` — persist the optional legacy-or-current model configuration and advance the current state version.
- `crates/wyse-store/src/definition.rs` — add a focused `AgentStore::start_turn` transition.
- `crates/wyse-store/src/error.rs` — distinguish a current state missing its required model configuration.
- `crates/wyse-store/src/filesystem.rs` — initialize, migrate, validate, and atomically commit model configuration.
- `crates/wyse-store/src/decorator.rs` — attach model configuration to the existing `Started` persistence path.
- `crates/wyse-llm/src/definition.rs` — define the configurable-provider trait and model descriptor DTO.
- `crates/wyse-llm/src/manager.rs` — register configurators, list descriptors, build defaults, and configure providers.
- `crates/wyse-llm/src/protocol/deepseek.rs` — map JSON parameters to `DeepSeekThinking` and return its schema.
- `crates/wyse-llm/src/protocol/openai_compatible.rs` — expose an empty-object schema and reject non-empty parameters.
- `crates/wyse-api/src/host.rs` — recover/migrate persisted settings, construct replacement agents, and retain immutable definitions.
- `crates/wyse-api/src/api.rs` — add `GET /v1/models`, accept `model_config` on messages, and project it through `AgentView`.
- `crates/wyse-api/src/error.rs` — map invalid provider parameters to a stable 422 response without exposing input.
- Existing tests in `crates/wyse-store/tests/`, `crates/wyse-llm/tests/`, and `crates/wyse-api/tests/api.rs` — cover the external contract and persistence invariants.
- `crates/wyse-core/AGENTS.md`, `crates/wyse-store/AGENTS.md`, `crates/wyse-llm/AGENTS.md`, and `crates/wyse-api/AGENTS.md` — archive the resulting crate boundaries before merge.

## Task 1: Persist a stable model configuration

**Files:**
- Modify: `crates/wyse-core/src/lib.rs`
- Modify: `crates/wyse-store/src/state.rs`
- Modify: `crates/wyse-store/src/definition.rs`
- Modify: `crates/wyse-store/src/error.rs`
- Modify: `crates/wyse-store/src/filesystem.rs`
- Modify: `crates/wyse-store/tests/filesystem_store.rs`
- Modify: `crates/wyse-store/tests/recovery_composition.rs`

**Interfaces:**
- Produces `wyse_core::ModelConfig { model: ModelId, parameters: serde_json::Map<String, serde_json::Value> }`.
- Produces `AgentStore::start_turn(run_id, turn_id, model_config) -> Result<AgentState, StoreError>`.
- Produces `FilesystemAgentStore::initialize_with_model_config(agent_id, name, model_config) -> Result<AgentState, StoreError>`.
- Produces `FilesystemAgentStore::write_model_config_if_missing(model_config) -> Result<AgentState, StoreError>` for one-time host recovery.

- [ ] **Step 1: Write failing state serialization and filesystem tests**

```rust
#[test]
fn agent_state_serializes_model_config() {
    let state = AgentState::new_configured(AgentId::new(), "writer".to_owned(), test_model_config());
    assert_eq!(serde_json::to_value(state).expect("state serializes")["model_config"]["model"], "openai:test-model");
}

#[tokio::test]
async fn write_model_config_if_missing_upgrades_legacy_state_once() {
    let legacy = legacy_agent_state_without_model_config(agent_id);
    filesystem.insert_entry("/agents/a/agent.json", json_entry(&legacy));
    let updated = store.write_model_config_if_missing(test_model_config()).await.expect("migration succeeds");
    assert_eq!(updated.model_config, Some(test_model_config()));
    assert_eq!(updated.state_version, AGENT_STATE_VERSION);
}
```

- [ ] **Step 2: Run the focused store tests and verify they fail because the configuration API is absent**

Run: `cargo test -p wyse-store --test filesystem_store`

Expected: FAIL with missing `ModelConfig`, `AgentState::new` argument, or `write_model_config_if_missing`.

- [ ] **Step 3: Add the minimum durable types and CAS operations**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ModelConfig {
    pub model: ModelId,
    pub parameters: serde_json::Map<String, serde_json::Value>,
}

#[async_trait]
pub trait AgentStore: Send + Sync {
    async fn start_turn(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        model_config: ModelConfig,
    ) -> Result<AgentState, StoreError> {
        let _ = model_config;
        self.update_state(AgentStatus::Running, Some(run_id), Some(turn_id), TokenUsage::default()).await
    }
}
```

Keep `AgentState::new` and `FilesystemAgentStore::initialize` unchanged for existing non-host callers that create legacy-compatible fixtures. Add `AgentState::new_configured` and `FilesystemAgentStore::initialize_with_model_config` for host creation; both serialize `Some(model_config)` at the current numeric state version. Keep the field as `Option<ModelConfig>` with `#[serde(default)]` only so legacy state can deserialize. Accept the prior numeric state version only when `model_config` is absent; return `StoreError::MissingModelConfig` when a current-version state lacks it. `FilesystemAgentStore::start_turn` must use its existing CAS update to set `status`, IDs, zero usage, `next_iteration`, `model_config`, and the current numeric state version in one write. `write_model_config_if_missing` must CAS only a legacy state without configuration, leave an already migrated state unchanged, and never rewrite message history.


- [ ] **Step 4: Add only local helpers required by the new tests and run the store suite**

Add a local `test_model_config()` helper only to the new state/migration test modules:

```rust
ModelConfig {
    model: ModelId::new("openai", "test-model").expect("static model is valid"),
    parameters: serde_json::Map::new(),
}
```

Run: `cargo test -p wyse-store --lib --tests`

Expected: PASS.

- [ ] **Step 5: Commit the persistence boundary**

```bash
git add crates/wyse-core/src/lib.rs crates/wyse-store/src/state.rs crates/wyse-store/src/definition.rs crates/wyse-store/src/filesystem.rs crates/wyse-store/tests
git commit -m "feat: persist agent model configuration"
```

## Task 2: Make providers configurable without host-specific branches

**Files:**
- Modify: `crates/wyse-llm/src/definition.rs`
- Modify: `crates/wyse-llm/src/manager.rs`
- Modify: `crates/wyse-llm/src/error.rs`
- Modify: `crates/wyse-llm/src/lib.rs`
- Modify: `crates/wyse-llm/src/protocol/deepseek.rs`
- Modify: `crates/wyse-llm/src/protocol/openai_compatible.rs`
- Modify: `crates/wyse-llm/tests/deepseek_provider.rs`
- Modify: `crates/wyse-llm/tests/openai_compatible_provider.rs`

**Interfaces:**
- Produces `ConfigurableLlmProvider: LlmProvider` with `parameter_schema`, `default_model_config`, and `configure` methods.
- Produces `ModelDescriptor { model: ModelId, parameters_schema: serde_json::Value }`.
- Produces `LlmProviderManager::{register, configure, default_model_config, models}`.

- [ ] **Step 1: Write failing DeepSeek/OpenAI provider tests**

```rust
#[test]
fn deepseek_schema_defaults_to_disabled_thinking() {
    let provider = test_provider("https://example.test/v1");
    assert_eq!(provider.parameter_schema()["default"], json!({"thinking": {"type": "disabled"}}));
}

#[test]
fn openai_rejects_non_empty_parameters() {
    let error = openai_provider().configure(&Map::from_iter([("temperature".to_owned(), json!(1))]));
    assert!(matches!(error, Err(LlmError::InvalidModelParameters { .. })));
}
```

- [ ] **Step 2: Run provider tests and verify the configurator interface is missing**

Run: `cargo test -p wyse-llm --tests`

Expected: FAIL with missing `ConfigurableLlmProvider`, `configure`, or `InvalidModelParameters`.

- [ ] **Step 3: Implement the narrow provider contract and manager methods**

```rust
pub trait ConfigurableLlmProvider: LlmProvider {
    fn parameter_schema(&self) -> serde_json::Value;
    fn default_model_config(&self) -> ModelConfig;
    fn configure(
        &self,
        parameters: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<Arc<dyn LlmProvider>, LlmError>;
}

pub struct ModelDescriptor {
    pub model: ModelId,
    pub parameters_schema: serde_json::Value,
}
```

Store `Arc<dyn ConfigurableLlmProvider>` in the manager's existing `BTreeMap`; preserve deterministic iteration. `configure(&ModelConfig)` must reject a missing model with the existing `ProviderNotFound` error and delegate parameters to the registered configurator. `models()` returns one descriptor per registered map entry. `default_model_config(&ModelId)` delegates to the same configurator, so the schema default and runtime default originate from the same provider helper.

For DeepSeek, deserialize the exact nested `thinking` object with a private serde input enum, reject unknown fields, and construct a cloned `DeepSeekProvider` carrying the parsed `DeepSeekThinking`. For OpenAI, accept only `Map::new()` and clone the existing provider. Each schema must use `additionalProperties: false` and contain a root `default`; no JSON Schema validator dependency is added.

- [ ] **Step 4: Run LLM unit and protocol tests**

Run: `cargo test -p wyse-llm --lib --tests`

Expected: PASS.

- [ ] **Step 5: Commit provider configuration support**

```bash
git add crates/wyse-llm/src crates/wyse-llm/tests
git commit -m "feat: configure llm providers per agent"
```

## Task 3: Persist the selected configuration on the existing start event

**Files:**
- Modify: `crates/wyse-store/src/decorator.rs`
- Modify: `crates/wyse-store/tests/decorator.rs`
- Modify: `crates/wyse-agent/tests/streaming_loop.rs`
- Modify: `crates/wyse-agent/src/definition.rs`

**Interfaces:**
- Produces `StoreEventStreamBus::with_model_config(store, inner, model_config)`.
- Keeps `StoreEventStreamBus::new(store, inner)` for existing REPL and test callers that do not own a host model configuration.

- [ ] **Step 1: Write a failing decorator test for the start-event commit**

```rust
#[tokio::test]
async fn started_event_persists_config_with_running_state() {
    let bus = StoreEventStreamBus::with_model_config(store.clone(), inner, test_model_config());
    bus.publish(started_envelope(agent_id, run_id, turn_id)).await.expect("started commits");
    let state = store.load_agent().await.expect("state loads");
    assert_eq!(state.status, AgentStatus::Running);
    assert_eq!(state.model_config, Some(test_model_config()));
}
```

- [ ] **Step 2: Run the decorator test and verify it fails because the configured constructor is absent**

Run: `cargo test -p wyse-store --test decorator started_event_persists_config_with_running_state`

Expected: FAIL with missing `with_model_config`.

- [ ] **Step 3: Route only configured started events through `start_turn`**

```rust
pub struct StoreEventStreamBus {
    store: Arc<dyn AgentStore>,
    inner: Arc<dyn EventStreamBus>,
    model_config: Option<ModelConfig>,
}

pub fn with_model_config(
    store: Arc<dyn AgentStore>,
    inner: Arc<dyn EventStreamBus>,
    model_config: ModelConfig,
) -> Self {
    Self {
        store,
        inner,
        model_config: Some(model_config),
    }
}
```

On `AgentEvent::Started`, call `start_turn(run_id, turn_id, model_config.clone())` when the option is present; otherwise retain the existing `update_state` call. Do not alter complete-message persistence, terminal state updates, or bounded forwarding. Update the `AgentStore` test doubles only where their assertions inspect the started transition.

- [ ] **Step 4: Run store and agent streaming tests**

Run: `cargo test -p wyse-store --test decorator && cargo test -p wyse-agent --test streaming_loop`

Expected: PASS.

- [ ] **Step 5: Commit the atomic start transition**

```bash
git add crates/wyse-store/src/decorator.rs crates/wyse-store/tests/decorator.rs crates/wyse-agent/src/definition.rs crates/wyse-agent/tests/streaming_loop.rs
git commit -m "feat: commit model config with agent start"
```

## Task 4: Compose, recover, and switch hosted agents

**Files:**
- Modify: `crates/wyse-api/src/host.rs`
- Modify: `crates/wyse-api/src/lib.rs`
- Modify: `crates/wyse-api/tests/api.rs`

**Interfaces:**
- Produces `HostedAgent::{agent, replace_agent, begin_transition}` methods, with a clonable current `Agent` stored behind a short-lived `RwLock`.
- Produces host-private `compose_agent(agent_id, definition, store, model_config) -> Result<Agent, HostError>`.
- Produces `HostState::models() -> Vec<ModelDescriptor>` and `HostState::prepare_message_agent(hosted, requested) -> Result<Agent, HostError>`.

- [ ] **Step 1: Write failing host tests for recovery, migration, and replacement**

```rust
#[tokio::test]
async fn restore_migrates_missing_model_config_from_definition_default() {
    fixture.persist_legacy_agent("coding-agent", AgentStatus::Finished).await;
    let host = fixture.restore_host().await.expect("host restores");
    let state = host.agent(agent_id).expect("agent exists").store.load_agent().await.expect("state loads");
    assert_eq!(state.model_config, Some(fixture.default_model_config()));
}

#[tokio::test]
async fn configured_start_replaces_agent_only_after_turn_is_accepted() {
    let host = fixture.restore_host().await.expect("host restores");
    let result = host.start_message(agent_id, "hello".to_owned(), Some(fixture.deepseek_model_config())).await;
    assert!(result.is_ok());
    assert_eq!(host.agent(agent_id).expect("agent exists").store.load_agent().await.expect("state loads").model_config, Some(fixture.deepseek_model_config()));
}
```

- [ ] **Step 2: Run the focused API tests and verify the host lacks model-aware composition**

Run: `cargo test -p wyse-api --test api`

Expected: FAIL with missing fixture configuration helpers or host methods.

- [ ] **Step 3: Implement the smallest safe host composition path**

Keep the immutable `ResolvedAgentDefinition` in `HostedAgent` for rebuilding its provider, prompt, and tools. Replace the public `agent: Agent` field with a private `RwLock<Agent>` and an `agent()` method that clones while the read guard is held briefly; `replace_agent` writes without awaiting.

Use an `AtomicBool` RAII transition flag on `HostedAgent` to serialize message-start, cancellation, approval, and resume admission while a candidate agent is being composed and started. It must return the existing `AgentError::RunAlreadyActive` conflict when set, and it must clear in `Drop`; no mutex guard spans I/O.

```rust
struct HostedAgentTransition<'a> {
    transitioning: &'a AtomicBool,
}

impl Drop for HostedAgentTransition<'_> {
    fn drop(&mut self) {
        self.transitioning.store(false, Ordering::Release);
    }
}

pub(crate) fn agent(&self) -> Agent {
    self.agent
        .read()
        .expect("hosted agent lock should not be poisoned")
        .clone()
}

pub(crate) fn replace_agent(&self, agent: Agent) {
    *self
        .agent
        .write()
        .expect("hosted agent lock should not be poisoned") = agent;
}

pub(crate) fn begin_transition(&self) -> Result<HostedAgentTransition<'_>, AgentError> {
    if self.transitioning.swap(true, Ordering::AcqRel) {
        return Err(AgentError::RunAlreadyActive);
    }
    Ok(HostedAgentTransition { transitioning: &self.transitioning })
}
```

`compose_agent` must obtain `providers.configure(&model_config)`, build the existing tool registry, and construct `StoreEventStreamBus::with_model_config`. On restore, load the filesystem store before converting it to `Arc<dyn AgentStore>`; if `model_config` is missing, call `write_model_config_if_missing(providers.default_model_config(&definition.model)?)`. For a persisted configuration, configure that exact model rather than validating the template's original model. On creation, obtain the template model's default configuration before `initialize` and compose with it.

Move the message-start orchestration from the HTTP handler into `HostState::start_message`: select the persisted setting when the request is absent, or validate/configure the requested complete setting; run the candidate; replace the hosted agent only after `run_turn` succeeds. If starting fails, do not replace the current agent and rely on the decorator's atomic start write to leave persisted configuration unchanged.

- [ ] **Step 4: Run host recovery and creation tests**

Run: `cargo test -p wyse-api --test api`

Expected: PASS, including the new migration/replacement tests and existing recovery tests.

- [ ] **Step 5: Commit host composition changes**

```bash
git add crates/wyse-api/src/host.rs crates/wyse-api/src/lib.rs crates/wyse-api/tests/api.rs
git commit -m "feat: compose hosted agents from model config"
```

## Task 5: Expose the API contract, stable validation errors, and archived invariants

**Files:**
- Modify: `crates/wyse-api/src/api.rs`
- Modify: `crates/wyse-api/src/error.rs`
- Modify: `crates/wyse-api/src/lib.rs`
- Modify: `crates/wyse-api/tests/api.rs`
- Modify: `crates/wyse-core/AGENTS.md`
- Modify: `crates/wyse-store/AGENTS.md`
- Modify: `crates/wyse-llm/AGENTS.md`
- Modify: `crates/wyse-api/AGENTS.md`

**Interfaces:**
- Produces `GET /v1/models` returning `ModelsResponse { models: Vec<ModelDescriptor> }`.
- Extends `MessageRequest` with `model_config: Option<ModelConfig>`.
- Extends `AgentView` with `model_config: ModelConfig`.
- Produces `HostError::InvalidModelParameters` mapped to HTTP 422 code `invalid_model_parameters`.

- [ ] **Step 1: Write failing HTTP contract tests**

```rust
#[tokio::test]
async fn models_lists_configured_models_with_provider_schema() {
    let response = fixture.request(Method::GET, "/v1/models", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(json_body(response).await["models"][0]["parameters_schema"]["type"], "object");
}

#[tokio::test]
async fn message_model_config_is_persisted_and_returned_by_agent_view() {
    let response = fixture.post_message(agent_id, json!({"text": "next", "model_config": fixture.deepseek_model_config()})).await;
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert_eq!(fixture.get_agent(agent_id).await["model_config"], serde_json::to_value(fixture.deepseek_model_config()).expect("config serializes"));
}

#[tokio::test]
async fn invalid_model_parameters_return_422_without_mutating_state() {
    let before = fixture.agent_state(agent_id).await;
    let response = fixture.post_message(agent_id, json!({"text": "next", "model_config": {"model": "openai:test-model", "parameters": {"x": true}}})).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(fixture.agent_state(agent_id).await.model_config, before.model_config);
}
```

- [ ] **Step 2: Run the focused API tests and verify routes/DTO fields are absent**

Run: `cargo test -p wyse-api --test api`

Expected: FAIL with a 404 models route, missing DTO field, or the old request deserializer.

- [ ] **Step 3: Implement the narrow HTTP boundary**

Add `.route("/v1/models", get(get_models))`. Keep the response model-only: serialize the manager descriptors directly and do not add a second `default_parameters` field. Add `#[serde(default)] model_config: Option<ModelConfig>` to the strict message request and pass it to `HostState::start_message`.

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ModelsResponse {
    models: Vec<ModelDescriptor>,
}

async fn get_models(State(state): State<Arc<HostState>>) -> Json<ModelsResponse> {
    Json(ModelsResponse { models: state.models() })
}
```

Replace `From<AgentState> for AgentView` with `TryFrom<AgentState, Error = HostError>` so an impossible state without `model_config` becomes a safe initialization error rather than a panic. Update `get_agent` to use `AgentView::try_from(persisted)?`. Add `HostError::InvalidModelParameters` with no user-provided string and map it to:

```rust
(StatusCode::UNPROCESSABLE_ENTITY, "invalid_model_parameters", "model parameters are invalid")
```

Catch provider parameter validation in `HostState::start_message` and convert it to this host error. Preserve `ConfigError::ModelNotConfigured` as `model_not_configured`, existing busy/resume mappings, request-deserialization 400, and shutdown 503.

- [ ] **Step 4: Archive invariants and run the full verification set**

Add concise crate-local `AGENTS.md` rules: `wyse-core` owns the common snapshot; `wyse-store` commits it only with the start transition and accepts legacy state solely for migration; `wyse-llm` validates provider parameters; `wyse-api` derives recovery from persisted config and exposes only schemas for configured models.

Run:

```bash
cargo fmt --check
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets
```

Expected: all commands exit 0. If ignored external-provider tests exist, leave them ignored; ordinary tests must not require credentials, containers, or NATS.

- [ ] **Step 5: Commit the public API and documentation**

```bash
git add crates/wyse-api/src crates/wyse-api/tests/api.rs crates/wyse-core/AGENTS.md crates/wyse-store/AGENTS.md crates/wyse-llm/AGENTS.md crates/wyse-api/AGENTS.md
git commit -m "feat: expose hosted model configuration"
```
