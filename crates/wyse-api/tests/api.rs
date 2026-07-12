use std::{
    collections::BTreeMap,
    fs, io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{
        Request, StatusCode,
        header::{
            ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_ORIGIN,
            ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, LOCATION, ORIGIN,
        },
    },
    response::IntoResponse,
};
use chrono::Utc;
use futures_util::{StreamExt, stream};
use serde_json::{Map, Value, json};
use tokio::time::timeout;
use tower::ServiceExt;
use wyse_agent::AgentError;
use wyse_api::{AgentCleanupError, AgentCreated, HostError, HostState, router, run_from_path};
use wyse_config::{AgentName, Config, ResolvedAgentDefinition};
use wyse_core::{
    AgentEvent, AgentId, ApprovalId, CallId, ChatMessage, EventCursor, EventRecord, EventSource,
    HistoryPage, ModelConfig, ModelId, ReplayStart, RunId, RuntimeEvent, StreamEnvelope,
    ToolCallDelta, TurnId,
};
use wyse_filesystem::{
    CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, LocalFilesystem,
    LocalFilesystemConfig, RecordVersion, VersionedEntry, VirtualPath,
};
use wyse_infra::{
    EventStream, EventStreamBus, EventStreamBusError, event_stream_bus::InMemoryEventStreamBus,
};
use wyse_llm::{
    ChatRequest, ChatResponse, ChatStream, ChatStreamEvent, ConfigurableLlmProvider, FinishReason,
    LlmError, LlmProvider, LlmProviderManager,
};
use wyse_store::{AgentStatus, AgentStore, FilesystemAgentStore, StoreError};

struct Fixture {
    root: PathBuf,
    filesystem: Arc<dyn Filesystem>,
    config: Config,
    model: ModelId,
}

impl Fixture {
    async fn new() -> Self {
        let unique = AgentId::new();
        let root = std::env::temp_dir().join(format!("wyse-api-{unique}"));
        fs::create_dir_all(root.join("history")).expect("history directory is created");
        fs::create_dir(root.join("templates")).expect("template directory is created");
        let filesystem: Arc<dyn Filesystem> = Arc::new(
            LocalFilesystem::new(LocalFilesystemConfig {
                root: root.clone(),
                max_file_bytes: None,
            })
            .expect("local filesystem is created"),
        );
        let model = ModelId::new("openai", "test-model").expect("model id is valid");
        let config = Config::parse(&format!(
            r#"
[agent]
storage_root = {root:?}

[llm]
default = "openai:test-model"

[llm.openai]
api_key = "test-key"
models = ["test-model"]

[llm.deepseek]
api_key = "test-key"
models = ["test-model"]

[api]
bind = "127.0.0.1:0"
allowed_origins = ["http://localhost:5173"]
"#,
            root = root.to_string_lossy()
        ))
        .expect("config parses");
        Self {
            root,
            filesystem,
            config,
            model,
        }
    }

    async fn persist_agent(&self, name: &str, status: AgentStatus) -> AgentId {
        let agent_id = AgentId::new();
        let root = self.root.join("history").join(agent_id.to_string());
        fs::create_dir_all(&root).expect("agent directory is created");
        let definition = ResolvedAgentDefinition::parse(&format!(
            r#"
agent_name = "{name}"
model = "{}"
tools = ["echo"]
prompt = "be helpful"
"#,
            self.model
        ))
        .expect("definition parses");
        fs::write(
            root.join("definition.toml"),
            definition.encode().expect("definition encodes"),
        )
        .expect("definition is written");
        let store = FilesystemAgentStore::new(
            Arc::clone(&self.filesystem),
            format!("/history/{agent_id}")
                .parse()
                .expect("agent root is valid"),
        );
        store
            .initialize_with_model_config(agent_id, name.to_owned(), self.default_model_config())
            .await
            .expect("store initializes");
        let run_id = RunId::new();
        let turn_id = TurnId::new();
        if status != AgentStatus::Idle {
            store
                .append_message(StreamEnvelope {
                    business_seq: None,
                    run_id,
                    timestamp: Utc::now(),
                    source: EventSource::Run,
                    event: RuntimeEvent::Agent {
                        agent_id,
                        event: AgentEvent::Message {
                            turn_id,
                            message: ChatMessage::user("persisted message"),
                        },
                    },
                    metadata: BTreeMap::new(),
                })
                .await
                .expect("message is persisted");
            store
                .update_state(status, Some(run_id), Some(turn_id), Default::default())
                .await
                .expect("state updates");
        }
        agent_id
    }

    async fn persist_legacy_agent(&self, name: &str, status: AgentStatus) -> AgentId {
        let agent_id = self.persist_agent(name, status).await;
        let state_path = self
            .root
            .join("history")
            .join(agent_id.to_string())
            .join("agent.json");
        let mut state: Value =
            serde_json::from_slice(&fs::read(&state_path).expect("agent state is readable"))
                .expect("agent state is json");
        state["state_version"] = json!(1);
        state
            .as_object_mut()
            .expect("agent state is an object")
            .remove("model_config");
        fs::write(
            state_path,
            serde_json::to_vec(&state).expect("legacy state encodes"),
        )
        .expect("legacy state is written");
        agent_id
    }

    fn default_model_config(&self) -> ModelConfig {
        ModelConfig::new(self.model.clone(), Map::new())
    }

    fn deepseek_model_config(&self) -> ModelConfig {
        ModelConfig::new(
            ModelId::new("deepseek", "test-model").expect("model id is valid"),
            Map::new(),
        )
    }

    async fn restore_host(&self) -> Result<Arc<HostState>, HostError> {
        self.restore_host_with_bus(Arc::new(InMemoryEventStreamBus::default()))
            .await
    }

    async fn restore_host_with_bus(
        &self,
        event_bus: Arc<dyn EventStreamBus>,
    ) -> Result<Arc<HostState>, HostError> {
        let mut providers = LlmProviderManager::new();
        providers
            .register(Arc::new(TestProvider(self.model.clone())))
            .expect("provider registers");
        providers
            .register(Arc::new(TestProvider(self.deepseek_model_config().model)))
            .expect("provider registers");
        HostState::restore(
            self.config.clone(),
            Arc::clone(&self.filesystem),
            event_bus,
            providers,
        )
        .await
    }

    fn persist_template(&self, name: &str, tools: &str) {
        fs::write(
            self.root.join("templates").join(format!("{name}.toml")),
            format!(
                r#"
model = "{}"
tools = [{tools}]
prompt = "be helpful"
"#,
                self.model
            ),
        )
        .expect("template is written");
    }

    async fn write_template(&self, name: &str, source: &str) {
        fs::write(
            self.root.join("templates").join(format!("{name}.toml")),
            source,
        )
        .expect("template is written");
    }

    async fn post_agent(&self, body: Value) -> axum::response::Response {
        let host = self.restore_host().await.expect("host restores");
        router(host)
            .oneshot(
                Request::post("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("request completes")
    }

    async fn request(&self, request: Request<Body>) -> (Arc<HostState>, axum::response::Response) {
        let host = self.restore_host().await.expect("host restores");
        let response = router(Arc::clone(&host))
            .oneshot(request)
            .await
            .expect("request completes");
        (host, response)
    }
}

#[tokio::test]
async fn router_rejects_request_bodies_larger_than_64_kib() {
    let fixture = Fixture::new().await;
    let response = fixture
        .post_agent(json!({
            "agent_name": "coding-agent",
            "text": "x".repeat(64 * 1024),
        }))
        .await;

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn cors_allows_only_an_exact_configured_origin() {
    let fixture = Fixture::new().await;
    let host = fixture.restore_host().await.expect("host restores");
    let allowed = router(Arc::clone(&host))
        .oneshot(
            Request::options("/v1/agents")
                .header(ORIGIN, "http://localhost:5173")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .header(ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let denied = router(host)
        .oneshot(
            Request::options("/v1/agents")
                .header(ORIGIN, "http://localhost:5174")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(
        allowed.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
        Some(&"http://localhost:5173".parse().expect("origin is valid"))
    );
    assert_eq!(
        allowed.headers().get(ACCESS_CONTROL_ALLOW_HEADERS),
        Some(&"content-type".parse().expect("header is valid"))
    );
    assert!(denied.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).is_none());
}

#[tokio::test]
async fn empty_allowed_origins_does_not_enable_cors() {
    let mut fixture = Fixture::new().await;
    fixture
        .config
        .api
        .as_mut()
        .expect("api is configured")
        .allowed_origins
        .clear();
    let host = fixture.restore_host().await.expect("host restores");
    let response = router(host)
        .oneshot(
            Request::options("/v1/agents")
                .header(ORIGIN, "http://localhost:5173")
                .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert!(
        response
            .headers()
            .get(ACCESS_CONTROL_ALLOW_ORIGIN)
            .is_none()
    );
}

#[tokio::test]
async fn run_from_path_rejects_missing_api_before_startup() {
    let root = std::env::temp_dir().join(format!("wyse-api-config-{}", AgentId::new()));
    fs::create_dir(&root).expect("temporary directory is created");
    let path = root.join("config.toml");
    fs::write(
        &path,
        format!(
            r#"
[agent]
storage_root = {root:?}

[llm]
default = "openai:test-model"

[llm.openai]
api_key = "test-key"
models = ["test-model"]
"#,
            root = root.to_string_lossy()
        ),
    )
    .expect("config is written");

    let error = run_from_path(&path)
        .await
        .expect_err("missing api must fail");

    assert!(matches!(
        error,
        HostError::Config(wyse_config::ConfigError::MissingSection { section: "api" })
    ));
    fs::remove_dir_all(root).expect("temporary directory is removed");
}

#[tokio::test]
async fn run_from_path_rejects_missing_nats_before_startup() {
    let root = std::env::temp_dir().join(format!("wyse-api-config-{}", AgentId::new()));
    fs::create_dir(&root).expect("temporary directory is created");
    let path = root.join("config.toml");
    fs::write(
        &path,
        format!(
            r#"
[agent]
storage_root = {root:?}

[llm]
default = "openai:test-model"

[llm.openai]
api_key = "test-key"
models = ["test-model"]

[api]
bind = "127.0.0.1:0"
"#,
            root = root.to_string_lossy()
        ),
    )
    .expect("config is written");

    let error = run_from_path(&path)
        .await
        .expect_err("missing nats must fail");

    assert!(matches!(
        error,
        HostError::Config(wyse_config::ConfigError::MissingSection { section: "nats" })
    ));
    fs::remove_dir_all(root).expect("temporary directory is removed");
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Clone)]
struct TestProvider(ModelId);

#[async_trait]
impl LlmProvider for TestProvider {
    fn model_id(&self) -> ModelId {
        self.0.clone()
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::MockExhausted)
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        Err(LlmError::MockExhausted)
    }
}

impl ConfigurableLlmProvider for TestProvider {
    fn parameter_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false, "default": {}})
    }

    fn default_model_config(&self) -> ModelConfig {
        let mut parameters = Map::new();
        if self.0.provider_name() == "deepseek" {
            parameters.insert("provider_default".to_owned(), json!("deepseek"));
        }
        ModelConfig::new(self.model_id(), parameters)
    }

    fn configure(&self, parameters: &Map<String, Value>) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if parameters.is_empty() {
            Ok(Arc::new(self.clone()))
        } else {
            Err(LlmError::InvalidModelParameters {
                model: self.model_id(),
            })
        }
    }
}

#[derive(Clone)]
struct PendingProvider(ModelId);

struct TestEventStreamBus {
    replay_starts: Mutex<Vec<ReplayStart>>,
    subscription: Mutex<Option<Result<EventStream, EventStreamBusError>>>,
}

struct PendingPublishBus {
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
}

struct PendingSecondPublishBus {
    publish_count: AtomicUsize,
    second_entered: tokio::sync::Notify,
}

impl PendingPublishBus {
    fn new() -> Self {
        Self {
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
        }
    }
}

impl PendingSecondPublishBus {
    fn new() -> Self {
        Self {
            publish_count: AtomicUsize::new(0),
            second_entered: tokio::sync::Notify::new(),
        }
    }
}

#[async_trait]
impl EventStreamBus for PendingPublishBus {
    async fn publish(&self, _envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        self.entered.notify_one();
        self.release.notified().await;
        Ok(())
    }

    async fn subscribe_agent(
        &self,
        _agent_id: AgentId,
        _replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        Ok(Box::pin(stream::pending()))
    }
}

#[async_trait]
impl EventStreamBus for PendingSecondPublishBus {
    async fn publish(&self, _envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        if self.publish_count.fetch_add(1, Ordering::SeqCst) == 1 {
            self.second_entered.notify_one();
            futures_util::future::pending().await
        }
        Ok(())
    }

    async fn subscribe_agent(
        &self,
        _agent_id: AgentId,
        _replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        Ok(Box::pin(stream::pending()))
    }
}

impl TestEventStreamBus {
    fn with_events(events: Vec<Result<EventRecord, EventStreamBusError>>) -> Self {
        Self {
            replay_starts: Mutex::new(Vec::new()),
            subscription: Mutex::new(Some(Ok(Box::pin(stream::iter(events))))),
        }
    }

    fn with_error(error: EventStreamBusError) -> Self {
        Self {
            replay_starts: Mutex::new(Vec::new()),
            subscription: Mutex::new(Some(Err(error))),
        }
    }

    fn pending() -> Self {
        Self {
            replay_starts: Mutex::new(Vec::new()),
            subscription: Mutex::new(Some(Ok(Box::pin(stream::pending())))),
        }
    }

    fn replay_starts(&self) -> Vec<ReplayStart> {
        self.replay_starts
            .lock()
            .expect("replay starts lock is not poisoned")
            .clone()
    }
}

#[async_trait]
impl EventStreamBus for TestEventStreamBus {
    async fn publish(&self, _envelope: StreamEnvelope) -> Result<(), EventStreamBusError> {
        Ok(())
    }

    async fn subscribe_agent(
        &self,
        _agent_id: AgentId,
        replay_start: ReplayStart,
    ) -> Result<EventStream, EventStreamBusError> {
        self.replay_starts
            .lock()
            .expect("replay starts lock is not poisoned")
            .push(replay_start);
        self.subscription
            .lock()
            .expect("subscription lock is not poisoned")
            .take()
            .expect("test subscription is requested once")
    }
}

#[async_trait]
impl LlmProvider for PendingProvider {
    fn model_id(&self) -> ModelId {
        self.0.clone()
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        std::future::pending().await
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        std::future::pending().await
    }
}

impl ConfigurableLlmProvider for PendingProvider {
    fn parameter_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false, "default": {}})
    }

    fn default_model_config(&self) -> ModelConfig {
        ModelConfig::new(self.model_id(), Map::new())
    }

    fn configure(&self, parameters: &Map<String, Value>) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if parameters.is_empty() {
            Ok(Arc::new(self.clone()))
        } else {
            Err(LlmError::InvalidModelParameters {
                model: self.model_id(),
            })
        }
    }
}

struct ApprovalProvider {
    model: ModelId,
    requests: AtomicUsize,
}

impl ApprovalProvider {
    fn new(model: ModelId) -> Self {
        Self {
            model,
            requests: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl LlmProvider for ApprovalProvider {
    fn model_id(&self) -> ModelId {
        self.model.clone()
    }

    async fn chat(&self, _request: ChatRequest) -> Result<ChatResponse, LlmError> {
        Err(LlmError::UnsupportedCapability("chat"))
    }

    async fn chat_stream(&self, _request: ChatRequest) -> Result<ChatStream, LlmError> {
        let events = if self.requests.fetch_add(1, Ordering::SeqCst) == 0 {
            vec![
                ChatStreamEvent::ToolCallDelta(ToolCallDelta {
                    index: 0,
                    call_id: Some(CallId::from("call-1")),
                    name: Some("echo".to_owned()),
                    arguments_delta: r#"{"message":"hello"}"#.to_owned(),
                }),
                ChatStreamEvent::Finished {
                    finish_reason: FinishReason::ToolCalls,
                    usage: None,
                },
            ]
        } else {
            vec![
                ChatStreamEvent::TextDelta {
                    delta: "done".to_owned(),
                },
                ChatStreamEvent::Finished {
                    finish_reason: FinishReason::Stop,
                    usage: None,
                },
            ]
        };
        Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
    }
}

impl ConfigurableLlmProvider for ApprovalProvider {
    fn parameter_schema(&self) -> Value {
        json!({"type": "object", "additionalProperties": false, "default": {}})
    }

    fn default_model_config(&self) -> ModelConfig {
        ModelConfig::new(self.model_id(), Map::new())
    }

    fn configure(&self, parameters: &Map<String, Value>) -> Result<Arc<dyn LlmProvider>, LlmError> {
        if parameters.is_empty() {
            Ok(Arc::new(Self::new(self.model.clone())))
        } else {
            Err(LlmError::InvalidModelParameters {
                model: self.model_id(),
            })
        }
    }
}

struct FailFirstMessageFilesystem {
    inner: Arc<dyn Filesystem>,
    created_root: Mutex<Option<VirtualPath>>,
    cleanup_failure: Option<CleanupFailure>,
    fail_start_turn: AtomicBool,
    block_recovery_after_failed_start: bool,
    block_recovery_load: AtomicBool,
    recovery_entered: tokio::sync::Notify,
    recovery_release: tokio::sync::Notify,
}

#[derive(Clone, Copy)]
enum CleanupFailure {
    ListMessages,
    RemoveMessagesDirectory,
}

impl FailFirstMessageFilesystem {
    fn new(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            created_root: Mutex::new(None),
            cleanup_failure: None,
            fail_start_turn: AtomicBool::new(false),
            block_recovery_after_failed_start: false,
            block_recovery_load: AtomicBool::new(false),
            recovery_entered: tokio::sync::Notify::new(),
            recovery_release: tokio::sync::Notify::new(),
        }
    }

    fn failing_cleanup(inner: Arc<dyn Filesystem>, cleanup_failure: CleanupFailure) -> Self {
        Self {
            inner,
            created_root: Mutex::new(None),
            cleanup_failure: Some(cleanup_failure),
            fail_start_turn: AtomicBool::new(false),
            block_recovery_after_failed_start: false,
            block_recovery_load: AtomicBool::new(false),
            recovery_entered: tokio::sync::Notify::new(),
            recovery_release: tokio::sync::Notify::new(),
        }
    }

    fn failing_start_turn(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            created_root: Mutex::new(None),
            cleanup_failure: None,
            fail_start_turn: AtomicBool::new(true),
            block_recovery_after_failed_start: false,
            block_recovery_load: AtomicBool::new(false),
            recovery_entered: tokio::sync::Notify::new(),
            recovery_release: tokio::sync::Notify::new(),
        }
    }

    fn failing_start_turn_with_blocked_recovery(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            created_root: Mutex::new(None),
            cleanup_failure: None,
            fail_start_turn: AtomicBool::new(true),
            block_recovery_after_failed_start: true,
            block_recovery_load: AtomicBool::new(false),
            recovery_entered: tokio::sync::Notify::new(),
            recovery_release: tokio::sync::Notify::new(),
        }
    }

    fn release_recovery_load(&self) {
        self.recovery_release.notify_one();
    }

    fn created_agent_id(&self) -> AgentId {
        self.created_root
            .lock()
            .expect("created root lock is not poisoned")
            .as_ref()
            .expect("agent root was created")
            .as_str()
            .rsplit_once('/')
            .expect("agent root has a final segment")
            .1
            .parse()
            .expect("agent id parses")
    }

    fn created_agent_root(&self) -> Option<VirtualPath> {
        self.created_root
            .lock()
            .expect("created root lock is not poisoned")
            .clone()
    }
}

#[async_trait]
impl Filesystem for FailFirstMessageFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        if path.as_str().ends_with("/agent.json")
            && self.block_recovery_load.swap(false, Ordering::SeqCst)
        {
            self.recovery_entered.notify_one();
            self.recovery_release.notified().await;
        }
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if path.as_str().ends_with("/agent.json")
            && self.fail_start_turn.swap(false, Ordering::SeqCst)
        {
            if self.block_recovery_after_failed_start {
                self.block_recovery_load.store(true, Ordering::SeqCst);
            }
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        if path.as_str().ends_with("/messages/1.json") {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        if matches!(self.cleanup_failure, Some(CleanupFailure::ListMessages))
            && path.as_str().ends_with("/messages")
        {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if path.as_str().starts_with("/history/") && !path.as_str()[9..].contains('/') {
            *self
                .created_root
                .lock()
                .expect("created root lock is not poisoned") = Some(path.clone());
        }
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if matches!(
            self.cleanup_failure,
            Some(CleanupFailure::RemoveMessagesDirectory)
        ) && path.as_str().ends_with("/messages")
        {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.remove_dir(path).await
    }
}

struct ResumeRaceFilesystem {
    inner: Arc<dyn Filesystem>,
    enabled: AtomicBool,
    state_reads: AtomicUsize,
}

impl ResumeRaceFilesystem {
    fn new(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            enabled: AtomicBool::new(false),
            state_reads: AtomicUsize::new(0),
        }
    }

    fn enable(&self) {
        self.state_reads.store(0, Ordering::SeqCst);
        self.enabled.store(true, Ordering::SeqCst);
    }
}

#[async_trait]
impl Filesystem for ResumeRaceFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let record = self.inner.get(path).await?;
        if !self.enabled.load(Ordering::SeqCst) || !path.as_str().ends_with("/agent.json") {
            return Ok(record);
        }
        let Some(record) = record else {
            return Ok(None);
        };
        if self.state_reads.fetch_add(1, Ordering::SeqCst) < 2 {
            return Ok(Some(record));
        }
        let mut state: Value =
            serde_json::from_slice(record.entry.contents()).expect("test state decodes");
        state["status"] = json!("failed");
        let contents = serde_json::to_vec(&state).expect("test state encodes");
        Ok(Some(VersionedEntry {
            entry: Entry::new(contents),
            version: record.version,
        }))
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.inner.put(path, entry, cas).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_dir(path).await
    }
}

struct FailTerminalStateFilesystem {
    inner: Arc<dyn Filesystem>,
    failing: AtomicBool,
    failed_writes: AtomicUsize,
}

struct FailNextMessageFilesystem {
    inner: Arc<dyn Filesystem>,
    fail_next_message: AtomicBool,
}

#[async_trait]
impl Filesystem for FailNextMessageFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if path.as_str().contains("/messages/")
            && self.fail_next_message.swap(false, Ordering::SeqCst)
        {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_dir(path).await
    }
}

#[derive(Clone, Copy)]
enum CreationOperationFailure {
    TemplateRead,
    HistoryCreate,
    DefinitionWrite,
}

struct CreationOperationFilesystem {
    inner: Arc<dyn Filesystem>,
    failure: CreationOperationFailure,
}

struct AdmissionFilesystem {
    inner: Arc<dyn Filesystem>,
    block_next_agent_get: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    operations: AtomicUsize,
}

#[derive(Clone, Copy)]
enum PendingCreationStage {
    RootCreate,
    DefinitionPut,
}

struct PendingCreationFilesystem {
    inner: Arc<dyn Filesystem>,
    stage: PendingCreationStage,
    block_once: AtomicBool,
    entered: tokio::sync::Notify,
    release: tokio::sync::Notify,
    remove_operations: AtomicUsize,
}

impl PendingCreationFilesystem {
    fn new(inner: Arc<dyn Filesystem>, stage: PendingCreationStage) -> Self {
        Self {
            inner,
            stage,
            block_once: AtomicBool::new(true),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            remove_operations: AtomicUsize::new(0),
        }
    }
}

impl AdmissionFilesystem {
    fn new(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            block_next_agent_get: AtomicBool::new(false),
            entered: tokio::sync::Notify::new(),
            release: tokio::sync::Notify::new(),
            operations: AtomicUsize::new(0),
        }
    }

    fn block_next_agent_get(&self) {
        self.block_next_agent_get.store(true, Ordering::SeqCst);
    }

    fn release(&self) {
        self.release.notify_one();
    }

    fn reset_operations(&self) {
        self.operations.store(0, Ordering::SeqCst);
    }

    fn operations(&self) -> usize {
        self.operations.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Filesystem for AdmissionFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        if path.as_str().ends_with("/agent.json")
            && self.block_next_agent_get.swap(false, Ordering::SeqCst)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.put(path, entry, cas).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.operations.fetch_add(1, Ordering::SeqCst);
        self.inner.remove_dir(path).await
    }
}

#[async_trait]
impl Filesystem for PendingCreationFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        let result = self.inner.put(path, entry, cas).await;
        if matches!(self.stage, PendingCreationStage::DefinitionPut)
            && path.as_str().ends_with("/definition.toml")
            && self.block_once.swap(false, Ordering::SeqCst)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        result
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let result = self.inner.create_dir(path).await;
        if matches!(self.stage, PendingCreationStage::RootCreate)
            && path.as_str().starts_with("/history/")
            && self.block_once.swap(false, Ordering::SeqCst)
        {
            self.entered.notify_one();
            self.release.notified().await;
        }
        result
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.remove_operations.fetch_add(1, Ordering::SeqCst);
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.remove_operations.fetch_add(1, Ordering::SeqCst);
        self.inner.remove_dir(path).await
    }
}

#[async_trait]
impl Filesystem for CreationOperationFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if matches!(self.failure, CreationOperationFailure::DefinitionWrite)
            && path.as_str().ends_with("/definition.toml")
        {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.put(path, entry, cas).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        if matches!(self.failure, CreationOperationFailure::TemplateRead)
            && path.as_str().starts_with("/templates/")
        {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if matches!(self.failure, CreationOperationFailure::HistoryCreate)
            && path.as_str().starts_with("/history/")
        {
            return Err(FilesystemError::PermissionDenied { path: path.clone() });
        }
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_dir(path).await
    }
}

impl FailTerminalStateFilesystem {
    fn new(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            failing: AtomicBool::new(true),
            failed_writes: AtomicUsize::new(0),
        }
    }

    fn recover(&self) {
        self.failing.store(false, Ordering::SeqCst);
    }

    fn failed_writes(&self) -> usize {
        self.failed_writes.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Filesystem for FailTerminalStateFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if self.failing.load(Ordering::SeqCst) && path.as_str().ends_with("/agent.json") {
            let value: Value =
                serde_json::from_slice(entry.contents()).expect("agent state update contains json");
            if value["status"] != "running" {
                self.failed_writes.fetch_add(1, Ordering::SeqCst);
                return Err(FilesystemError::PermissionDenied { path: path.clone() });
            }
        }
        self.inner.put(path, entry, cas).await
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.inner.read_file(path).await
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.inner.write_file(path, contents).await
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.inner.list_dir(path).await
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        self.inner.metadata(path).await
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.create_dir(path).await
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_file(path).await
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.inner.remove_dir(path).await
    }
}

#[tokio::test]
async fn restore_loads_complete_history_directories() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;

    let host = fixture.restore_host().await.expect("host restores");

    assert!(host.agent(agent_id).is_some());
}

#[tokio::test]
async fn restore_migrates_missing_model_config_from_definition_default() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_legacy_agent("coding-agent", AgentStatus::Finished)
        .await;

    let host = fixture.restore_host().await.expect("host restores");
    let state = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");

    assert_eq!(state.model_config, Some(fixture.default_model_config()));
}

#[tokio::test]
async fn configured_start_replaces_agent_only_after_turn_is_accepted() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let host = fixture.restore_host().await.expect("host restores");

    let result = host
        .start_message(
            agent_id,
            "hello".to_owned(),
            Some(fixture.deepseek_model_config()),
        )
        .await;

    assert!(result.is_ok());
    let state = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(state.model_config, Some(fixture.deepseek_model_config()));
}

#[tokio::test]
async fn configured_start_failure_keeps_the_active_agent_and_model_config() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let filesystem = Arc::new(FailFirstMessageFilesystem::failing_start_turn(Arc::clone(
        &fixture.filesystem,
    )));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    providers
        .register(Arc::new(TestProvider(
            fixture.deepseek_model_config().model,
        )))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");

    let result = host
        .start_message(
            agent_id,
            "hello".to_owned(),
            Some(fixture.deepseek_model_config()),
        )
        .await;

    assert!(result.is_err());
    let hosted = host.agent(agent_id).expect("agent exists");
    assert!(!hosted.needs_resume());
    let state = hosted.store.load_agent().await.expect("state loads");
    assert_eq!(state.status, AgentStatus::Finished);
    assert_eq!(state.model_config, Some(fixture.default_model_config()));
    assert!(
        host.start_message(agent_id, "retry".to_owned(), None)
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn configured_start_while_current_agent_is_active_returns_busy_and_remains_cancellable() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(PendingProvider(fixture.model.clone())))
        .expect("provider registers");
    providers
        .register(Arc::new(TestProvider(
            fixture.deepseek_model_config().model,
        )))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");

    host.start_message(agent_id, "wait".to_owned(), None)
        .await
        .expect("original turn starts");
    let error = host
        .start_message(
            agent_id,
            "switch".to_owned(),
            Some(fixture.deepseek_model_config()),
        )
        .await
        .expect_err("configured start conflicts with the active runtime");

    assert!(matches!(
        error,
        HostError::Agent(AgentError::RunAlreadyActive)
    ));
    assert!(!host.agent(agent_id).expect("agent exists").needs_resume());
    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/cancel"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    host.shutdown().await;
    assert_eq!(
        host.agent(agent_id)
            .expect("agent exists")
            .store
            .load_agent()
            .await
            .expect("state loads")
            .status,
        AgentStatus::Cancelled
    );
}

#[tokio::test]
async fn failed_candidate_recovery_read_is_cancelled_by_shutdown() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let filesystem = Arc::new(
        FailFirstMessageFilesystem::failing_start_turn_with_blocked_recovery(Arc::clone(
            &fixture.filesystem,
        )),
    );
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    providers
        .register(Arc::new(TestProvider(
            fixture.deepseek_model_config().model,
        )))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");

    let message_host = Arc::clone(&host);
    let message = tokio::spawn(async move {
        message_host
            .start_message(
                agent_id,
                "switch".to_owned(),
                Some(ModelConfig::new(
                    ModelId::new("deepseek", "test-model").expect("model id is valid"),
                    Map::new(),
                )),
            )
            .await
    });
    timeout(
        Duration::from_secs(1),
        filesystem.recovery_entered.notified(),
    )
    .await
    .expect("candidate recovery reads Store");

    let shutdown_host = Arc::clone(&host);
    let mut shutdown = tokio::spawn(async move { shutdown_host.shutdown().await });
    timeout(Duration::from_secs(2), &mut shutdown)
        .await
        .expect("shutdown drains admission")
        .expect("shutdown task completes");
    let result = timeout(Duration::from_secs(1), message)
        .await
        .expect("message returns after shutdown")
        .expect("message task completes");
    assert!(matches!(result, Err(HostError::HostShuttingDown)));
    filesystem.release_recovery_load();
}

#[tokio::test]
async fn restore_marks_running_agents_as_needing_resume() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Running)
        .await;

    let host = fixture.restore_host().await.expect("host restores");

    assert!(host.agent(agent_id).expect("agent exists").needs_resume());
}

#[tokio::test]
async fn restore_creates_templates_and_history_for_a_new_storage_root() {
    let root = std::env::temp_dir().join(format!("wyse-api-empty-{}", AgentId::new()));
    fs::create_dir(&root).expect("storage root is created");
    let filesystem: Arc<dyn Filesystem> = Arc::new(
        LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: None,
        })
        .expect("filesystem is created"),
    );
    let model = ModelId::new("openai", "test-model").expect("model id is valid");
    let config = Config::parse(&format!(
        r#"
[agent]
storage_root = {root:?}

[llm]
default = "openai:test-model"

[llm.openai]
api_key = "test-key"
models = ["test-model"]
"#,
        root = root.to_string_lossy()
    ))
    .expect("config parses");
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(model)))
        .expect("provider registers");

    HostState::restore(
        config,
        filesystem,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("empty root restores");

    assert!(root.join("templates").is_dir());
    assert!(root.join("history").is_dir());
    fs::remove_dir_all(root).expect("storage root is removed");
}

#[tokio::test]
async fn restore_rejects_invalid_history_directory_id() {
    let fixture = Fixture::new().await;
    fs::create_dir(fixture.root.join("history/not-an-agent-id"))
        .expect("invalid directory is created");

    let error = match fixture.restore_host().await {
        Ok(_) => panic!("restore should fail"),
        Err(error) => error,
    };

    assert!(matches!(error, HostError::InvalidHistoryDirectory { .. }));
}

#[tokio::test]
async fn restore_rejects_corrupt_definition() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    fs::write(
        fixture
            .root
            .join("history")
            .join(agent_id.to_string())
            .join("definition.toml"),
        "not = [valid",
    )
    .expect("definition is corrupted");

    let error = match fixture.restore_host().await {
        Ok(_) => panic!("restore should fail"),
        Err(error) => error,
    };

    assert!(matches!(error, HostError::Config(_)));
}

#[tokio::test]
async fn restore_uses_persisted_model_config_when_definition_model_was_removed() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let definition_path = fixture
        .root
        .join("history")
        .join(agent_id.to_string())
        .join("definition.toml");
    let definition = fs::read_to_string(&definition_path).expect("definition is readable");
    fs::write(
        definition_path,
        definition.replace("openai:test-model", "deepseek:test-model"),
    )
    .expect("definition model is rewritten");
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");

    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("persisted configuration restores");

    assert!(host.agent(agent_id).is_some());
}

#[tokio::test]
async fn restore_rejects_store_id_and_name_mismatches() {
    for field in ["agent_id", "name"] {
        let fixture = Fixture::new().await;
        let agent_id = fixture
            .persist_agent("coding-agent", AgentStatus::Idle)
            .await;
        let state_path = fixture
            .root
            .join("history")
            .join(agent_id.to_string())
            .join("agent.json");
        let mut state: Value =
            serde_json::from_slice(&fs::read(&state_path).expect("agent state is readable"))
                .expect("agent state is json");
        state[field] = if field == "agent_id" {
            json!(AgentId::new())
        } else {
            json!("different-name")
        };
        fs::write(
            &state_path,
            serde_json::to_vec(&state).expect("state encodes"),
        )
        .expect("state is rewritten");

        let error = match fixture.restore_host().await {
            Ok(_) => panic!("identity mismatch is rejected"),
            Err(error) => error,
        };

        assert!(matches!(error, HostError::IdentityMismatch { .. }));
    }
}

#[tokio::test]
async fn restore_rejects_unknown_tools_and_missing_provider_manager_entries() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let definition_path = fixture
        .root
        .join("history")
        .join(agent_id.to_string())
        .join("definition.toml");
    let definition = fs::read_to_string(&definition_path).expect("definition is readable");
    fs::write(
        &definition_path,
        definition.replace("tools = [\"echo\"]", "tools = [\"unknown\"]"),
    )
    .expect("definition is rewritten");
    let unknown_tool = match fixture.restore_host().await {
        Ok(_) => panic!("unknown tool is rejected"),
        Err(error) => error,
    };
    assert!(matches!(unknown_tool, HostError::ToolNotAvailable { .. }));

    fs::write(&definition_path, definition).expect("definition is restored");
    let missing_provider = match HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        LlmProviderManager::new(),
    )
    .await
    {
        Ok(_) => panic!("missing provider entry is rejected"),
        Err(error) => error,
    };
    assert!(matches!(
        missing_provider,
        HostError::Llm(LlmError::ProviderNotFound { .. })
    ));
}

#[tokio::test]
async fn restore_accepts_the_definition_and_directory_format_written_by_the_repl() {
    let fixture = Fixture::new().await;
    let agent_id = AgentId::new();
    let root = fixture.root.join("history").join(agent_id.to_string());
    fs::create_dir(&root).expect("REPL history directory is created");
    let definition = fixture
        .config
        .resolve_template(
            "default-agent".parse().expect("name parses"),
            "tools = [\"echo\"]\nprompt = \"You are a helpful assistant.\"",
        )
        .expect("REPL definition resolves");
    fs::write(
        root.join("definition.toml"),
        definition.encode().expect("definition encodes"),
    )
    .expect("REPL definition is written");
    FilesystemAgentStore::new(
        Arc::clone(&fixture.filesystem),
        format!("/history/{agent_id}").parse().expect("root parses"),
    )
    .initialize(agent_id, "default-agent".to_owned())
    .await
    .expect("REPL store initializes");

    let host = fixture
        .restore_host()
        .await
        .expect("API restores REPL output");

    assert!(host.agent(agent_id).is_some());
}

#[tokio::test]
async fn create_agent_rejects_blank_text_without_creating_history() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");

    let response = fixture
        .post_agent(json!({"agent_name": "coding-agent", "text": " \n\t"}))
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        fs::read_dir(fixture.root.join("history"))
            .expect("history is readable")
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn create_agent_returns_not_found_for_missing_template() {
    let fixture = Fixture::new().await;

    let response = fixture
        .post_agent(json!({"agent_name": "missing", "text": "hello"}))
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(!fixture.root.join("templates/missing.toml").exists());
}

#[tokio::test]
async fn create_agent_returns_unprocessable_entity_for_unknown_tool() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"unknown\"");

    let response = fixture
        .post_agent(json!({"agent_name": "coding-agent", "text": "hello"}))
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert!(
        fs::read_dir(fixture.root.join("history"))
            .expect("history is readable")
            .next()
            .is_none()
    );
}

#[tokio::test]
async fn create_agent_preflights_tools_before_touching_history() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"unknown\"");
    let filesystem = Arc::new(FailFirstMessageFilesystem::new(Arc::clone(
        &fixture.filesystem,
    )));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");

    let error = host
        .create_agent(
            "coding-agent".parse().expect("name parses"),
            "hello".to_owned(),
        )
        .await
        .expect_err("unknown tool is rejected");

    assert!(matches!(error, HostError::ToolNotAvailable { .. }));
    assert_eq!(filesystem.created_agent_root(), None);
}

#[tokio::test]
async fn create_agent_preflights_provider_before_touching_history() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let filesystem = Arc::new(FailFirstMessageFilesystem::new(Arc::clone(
        &fixture.filesystem,
    )));
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        LlmProviderManager::new(),
    )
    .await
    .expect("empty history does not require a provider");

    let error = host
        .create_agent(
            "coding-agent".parse().expect("name parses"),
            "hello".to_owned(),
        )
        .await
        .expect_err("missing provider is rejected");

    assert!(matches!(
        error,
        HostError::Llm(LlmError::ProviderNotFound { .. })
    ));
    assert_eq!(filesystem.created_agent_root(), None);
}

#[tokio::test]
async fn create_agent_uses_distinct_uuid_v7_ids_for_the_same_template() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let host = fixture.restore_host().await.expect("host restores");
    let name: AgentName = "coding-agent".parse().expect("name parses");

    let first = host
        .create_agent(name.clone(), "first".to_owned())
        .await
        .expect("first agent is created");
    let second = host
        .create_agent(name, "second".to_owned())
        .await
        .expect("second agent is created");

    assert_ne!(first.agent_id, second.agent_id);
    assert_eq!(first.agent_id.as_uuid().get_version_num(), 7);
    assert_eq!(second.agent_id.as_uuid().get_version_num(), 7);
}

#[tokio::test]
async fn create_agent_commits_first_message_before_returning() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let host = fixture.restore_host().await.expect("host restores");
    let app = router(Arc::clone(&host));

    let response = app
        .oneshot(
            Request::post("/v1/agents")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"agent_name": "coding-agent", "text": "inspect the stream"}).to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::CREATED);
    let location = response
        .headers()
        .get(LOCATION)
        .expect("location is present")
        .to_str()
        .expect("location is text")
        .to_owned();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let created: AgentCreated = serde_json::from_slice(&body).expect("response decodes");
    assert_eq!(created.agent_name, "coding-agent");
    assert_eq!(location, format!("/v1/agents/{}", created.agent_id));
    assert_eq!(created.run_id.as_uuid().get_version_num(), 7);
    let hosted = host.agent(created.agent_id).expect("agent is registered");
    assert_eq!(
        hosted
            .store
            .load_agent()
            .await
            .expect("state loads")
            .last_seq,
        1
    );
}

#[tokio::test]
async fn create_agent_cleans_preamble_failure_without_registering_agent() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let filesystem = Arc::new(FailFirstMessageFilesystem::new(Arc::clone(
        &fixture.filesystem,
    )));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    let name: AgentName = "coding-agent".parse().expect("name parses");

    let result = host.create_agent(name, "hello".to_owned()).await;

    assert!(matches!(result, Err(HostError::Agent(_))));
    let agent_id = filesystem.created_agent_id();
    assert!(host.agent(agent_id).is_none());
    assert!(
        !fixture
            .root
            .join("history")
            .join(agent_id.to_string())
            .exists()
    );
}

#[tokio::test]
async fn create_agent_preserves_on_uncertain_inspection_and_exposes_remove_failures() {
    for failure in [
        CleanupFailure::ListMessages,
        CleanupFailure::RemoveMessagesDirectory,
    ] {
        let fixture = Fixture::new().await;
        fixture.persist_template("coding-agent", "\"echo\"");
        let filesystem = Arc::new(FailFirstMessageFilesystem::failing_cleanup(
            Arc::clone(&fixture.filesystem),
            failure,
        ));
        let mut providers = LlmProviderManager::new();
        providers
            .register(Arc::new(TestProvider(fixture.model.clone())))
            .expect("provider registers");
        let host = HostState::restore(
            fixture.config.clone(),
            Arc::clone(&filesystem) as Arc<dyn Filesystem>,
            Arc::new(InMemoryEventStreamBus::default()),
            providers,
        )
        .await
        .expect("host restores");
        let name: AgentName = "coding-agent".parse().expect("name parses");

        let error = host
            .create_agent(name, "hello".to_owned())
            .await
            .expect_err("creation and cleanup should fail");

        match failure {
            CleanupFailure::ListMessages => {
                assert!(matches!(error, HostError::Agent(_)));
            }
            CleanupFailure::RemoveMessagesDirectory => {
                let HostError::CreationCleanup { creation, cleanup } = &error else {
                    panic!("cleanup failure should be explicit");
                };
                assert!(matches!(creation.as_ref(), HostError::Agent(_)));
                assert!(matches!(cleanup, AgentCleanupError::Filesystem(_)));
                assert_eq!(
                    std::error::Error::source(&error)
                        .expect("creation failure is retained")
                        .to_string(),
                    "agent operation failed"
                );
            }
        }
        let agent_id = filesystem.created_agent_id();
        assert!(host.agent(agent_id).is_none());
        assert!(
            fixture
                .root
                .join("history")
                .join(agent_id.to_string())
                .exists()
        );
    }
}

#[tokio::test]
async fn creation_cleanup_http_response_does_not_expose_error_details() {
    let secret_path: VirtualPath = "/history/secret/messages"
        .parse()
        .expect("virtual path parses");
    let error = HostError::CreationCleanup {
        creation: Box::new(HostError::EmptyText),
        cleanup: AgentCleanupError::Filesystem(FilesystemError::PermissionDenied {
            path: secret_path,
        }),
    };

    let response = error.into_response();

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "internal_error");
    assert!(!body.to_string().contains("secret"));
}

#[tokio::test]
async fn get_agent_returns_not_found_json_for_unknown_agent() {
    let fixture = Fixture::new().await;
    let agent_id = AgentId::new();

    let (_, response) = fixture
        .request(
            Request::get(format!("/v1/agents/{agent_id}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "agent_not_found");
    assert!(!body.to_string().contains(&agent_id.to_string()));
}

#[tokio::test]
async fn get_agent_projects_only_public_view_fields() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;

    let (_, response) = fixture
        .request(
            Request::get(format!("/v1/agents/{agent_id}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("view body is json");
    let mut keys = body
        .as_object()
        .expect("view is an object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "agent_id",
            "agent_name",
            "last_seq",
            "model_config",
            "run_id",
            "status",
            "turn_id",
            "updated_at",
            "usage",
        ]
    );
    assert_eq!(body["agent_name"], "coding-agent");
}

#[tokio::test]
async fn models_lists_configured_models_with_provider_schema() {
    let fixture = Fixture::new().await;

    let (_, response) = fixture
        .request(
            Request::get("/v1/models")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("models body is json");
    assert_eq!(body["models"][0]["parameters_schema"]["type"], "object");
    assert!(body["models"][0].get("default_parameters").is_none());
}

#[tokio::test]
async fn agent_templates_list_resolved_default_configuration() {
    let fixture = Fixture::new().await;
    fixture
        .write_template("coding-agent", "prompt = \"be helpful\"")
        .await;
    fixture
        .write_template("zebra-agent", "prompt = \"be helpful\"")
        .await;

    let (_, response) = fixture
        .request(
            Request::get("/v1/agent/templates")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("template body is json");
    assert_eq!(body["agents"][0]["agent_name"], "coding-agent");
    assert_eq!(body["agents"][1]["agent_name"], "zebra-agent");
    assert_eq!(
        body["agents"][0]["model_config"]["model"],
        "openai:test-model"
    );
    let mut fields = body["agents"][0]
        .as_object()
        .expect("template view is an object")
        .keys()
        .map(String::as_str)
        .collect::<Vec<_>>();
    fields.sort_unstable();
    assert_eq!(fields, ["agent_name", "model_config"]);
}

#[tokio::test]
async fn agent_templates_list_provider_default_configuration_for_template_model() {
    let fixture = Fixture::new().await;
    fixture
        .write_template(
            "deepseek-agent",
            "model = \"deepseek:test-model\"\nprompt = \"be helpful\"",
        )
        .await;

    let (_, response) = fixture
        .request(
            Request::get("/v1/agent/templates")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("template body is json");
    assert_eq!(body["agents"][0]["agent_name"], "deepseek-agent");
    assert_eq!(
        body["agents"][0]["model_config"]["model"],
        "deepseek:test-model"
    );
    assert_eq!(
        body["agents"][0]["model_config"]["parameters"]["provider_default"],
        "deepseek"
    );
}

#[tokio::test]
async fn agent_templates_return_unprocessable_entity_for_invalid_template() {
    let fixture = Fixture::new().await;
    fixture.write_template("broken-agent", "model = [").await;

    let (_, response) = fixture
        .request(
            Request::get("/v1/agent/templates")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_agent_template");
}

#[tokio::test]
async fn message_model_config_is_persisted_and_returned_by_agent_view() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let host = fixture.restore_host().await.expect("host restores");
    let model_config = fixture.deepseek_model_config();

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"text": "next", "model_config": model_config}).to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let response = router(host)
        .oneshot(
            Request::get(format!("/v1/agents/{agent_id}"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("view body is json");
    assert_eq!(
        body["model_config"],
        serde_json::to_value(fixture.deepseek_model_config()).expect("config serializes")
    );
}

#[tokio::test]
async fn invalid_model_parameters_return_422_without_mutating_state() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let host = fixture.restore_host().await.expect("host restores");
    let before = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "text": "next",
                        "model_config": {
                            "model": "openai:test-model",
                            "parameters": {"x": true}
                        }
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_model_parameters");
    assert!(!body.to_string().contains("\"x\""));
    let after = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(after.model_config, before.model_config);
}

#[tokio::test]
async fn unavailable_message_model_returns_422_without_mutating_state() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let host = fixture.restore_host().await.expect("host restores");
    let before = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "text": "next",
                        "model_config": {
                            "model": "anthropic:unregistered-model",
                            "parameters": {}
                        }
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("error body is json");
    assert_eq!(body["error"]["code"], "model_not_configured");
    let after = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(after.model_config, before.model_config);
}

#[tokio::test]
async fn message_model_config_rejects_unknown_fields() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;

    let (_, response) = fixture
        .request(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "text": "next",
                        "model_config": {
                            "model": "openai:test-model",
                            "parameters": {},
                            "paramters": {}
                        }
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: Value = serde_json::from_slice(
        &to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn history_uses_a_fixed_barrier_across_pages() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;

    let (_, first) = fixture
        .request(
            Request::get(format!(
                "/v1/agents/{agent_id}/messages?after_seq=0&limit=1"
            ))
            .body(Body::empty())
            .expect("request builds"),
        )
        .await;
    assert_eq!(first.status(), StatusCode::OK);
    let first: HistoryPage = serde_json::from_slice(
        &to_bytes(first.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("history decodes");

    let (_, second) = fixture
        .request(
            Request::get(format!(
                "/v1/agents/{agent_id}/messages?after_seq={}&through_seq={}&limit=1",
                first.next_front_seq, first.through_seq
            ))
            .body(Body::empty())
            .expect("request builds"),
        )
        .await;
    assert_eq!(second.status(), StatusCode::OK);
    let second: HistoryPage = serde_json::from_slice(
        &to_bytes(second.into_body(), usize::MAX)
            .await
            .expect("body is readable"),
    )
    .expect("history decodes");
    assert_eq!(second.through_seq, first.through_seq);
}

#[tokio::test]
async fn post_message_rejects_blank_text_as_bad_request() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;

    let (_, response) = fixture
        .request(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": " \n\t"}).to_string()))
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_message");
    assert!(!body.to_string().contains("\\n"));
}

#[tokio::test]
async fn post_message_returns_accepted_after_durable_append() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;

    let (host, response) = fixture
        .request(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "hello"}).to_string()))
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("accepted body is json");
    assert!(body["run_id"].is_string());
    assert_eq!(
        host.agent(agent_id)
            .expect("agent exists")
            .store
            .load_agent()
            .await
            .expect("state loads")
            .last_seq,
        1
    );
}

#[tokio::test]
async fn needs_resume_blocks_messages_and_cancel() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Running)
        .await;
    let host = fixture.restore_host().await.expect("host restores");
    let app = router(host);

    for request in [
        Request::post(format!("/v1/agents/{agent_id}/messages"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"text": "hello"}).to_string()))
            .expect("request builds"),
        Request::post(format!("/v1/agents/{agent_id}/cancel"))
            .body(Body::empty())
            .expect("request builds"),
    ] {
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("request completes");
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable");
        let body: Value = serde_json::from_slice(&body).expect("error body is json");
        assert_eq!(body["error"]["code"], "resume_required");
    }
}

#[tokio::test]
async fn resume_rechecks_store_and_clears_stale_marker() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Running)
        .await;
    let host = fixture.restore_host().await.expect("host restores");
    let hosted = host.agent(agent_id).expect("agent exists");
    hosted
        .store
        .update_state(AgentStatus::Failed, None, None, Default::default())
        .await
        .expect("state updates");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "resume_not_running");
    assert!(!host.agent(agent_id).expect("agent exists").needs_resume());
}

#[tokio::test]
async fn cancel_is_accepted_without_an_active_turn() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;

    let (_, response) = fixture
        .request(
            Request::post(format!("/v1/agents/{agent_id}/cancel"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn approval_without_active_turn_is_a_conflict() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let approval_id = ApprovalId::new();

    let (_, response) = fixture
        .request(
            Request::post(format!("/v1/agents/{agent_id}/approvals/{approval_id}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"decision": "approve"}).to_string()))
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "approval_not_active");
}

#[tokio::test]
async fn post_message_maps_an_active_run_to_agent_busy() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(PendingProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    let app = router(host);
    let request = || {
        Request::post(format!("/v1/agents/{agent_id}/messages"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"text": "hello"}).to_string()))
            .expect("request builds")
    };

    let accepted = app
        .clone()
        .oneshot(request())
        .await
        .expect("request completes");
    let busy = app.oneshot(request()).await.expect("request completes");

    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    let body = to_bytes(busy.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "agent_busy");
}

#[tokio::test]
async fn resume_accepts_a_persisted_running_turn_and_clears_marker() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Running)
        .await;
    let host = fixture.restore_host().await.expect("host restores");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::ACCEPTED);
    assert!(!host.agent(agent_id).expect("agent exists").needs_resume());
}

#[tokio::test]
async fn resume_preserves_switched_model_after_message_write_failure() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let filesystem = Arc::new(FailNextMessageFilesystem {
        inner: Arc::clone(&fixture.filesystem),
        fail_next_message: AtomicBool::new(true),
    });
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    providers
        .register(Arc::new(TestProvider(
            fixture.deepseek_model_config().model,
        )))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        filesystem as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    let app = router(Arc::clone(&host));
    let before = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .history_page(wyse_core::HistoryQuery {
            after_seq: 0,
            through_seq: None,
            limit: 100,
        })
        .await
        .expect("old history loads");

    let failed = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "text": "lost preamble",
                        "model_config": fixture.deepseek_model_config(),
                    })
                    .to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(failed.status(), StatusCode::SERVICE_UNAVAILABLE);
    let started_only = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(started_only.status, AgentStatus::Running);
    assert_eq!(
        started_only.model_config,
        Some(fixture.deepseek_model_config())
    );
    assert_eq!(started_only.last_seq, before.through_seq);

    let reconciled = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(reconciled.status(), StatusCode::CONFLICT);
    let body = to_bytes(reconciled.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("body is json");
    assert_eq!(body["error"]["code"], "resume_not_running");
    let terminal = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(terminal.status, AgentStatus::Failed);
    assert_eq!(terminal.run_id, started_only.run_id);
    assert_eq!(terminal.turn_id, started_only.turn_id);
    assert_eq!(terminal.last_seq, started_only.last_seq);
    assert!(!host.agent(agent_id).expect("agent exists").needs_resume());

    let accepted = app
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "new message"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let current = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(current.model_config, Some(fixture.deepseek_model_config()));
    let after = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .history_page(wyse_core::HistoryQuery {
            after_seq: 0,
            through_seq: None,
            limit: 100,
        })
        .await
        .expect("new history loads");
    assert_eq!(after.events[0], before.events[0]);
    assert_eq!(after.events.len(), before.events.len() + 1);
}

#[tokio::test]
async fn resume_does_not_reconcile_a_current_turn_with_any_durable_invalid_message() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let store = FilesystemAgentStore::new(
        Arc::clone(&fixture.filesystem),
        format!("/history/{agent_id}").parse().expect("root parses"),
    );
    let run_id = RunId::new();
    let turn_id = TurnId::new();
    store
        .append_message(StreamEnvelope {
            business_seq: None,
            run_id,
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent {
                agent_id,
                event: AgentEvent::Message {
                    turn_id,
                    message: ChatMessage::assistant("invalid without user"),
                },
            },
            metadata: BTreeMap::new(),
        })
        .await
        .expect("invalid resume fixture message persists");
    store
        .update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            Default::default(),
        )
        .await
        .expect("running state persists");
    let host = fixture.restore_host().await.expect("host restores");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(
        host.agent(agent_id)
            .expect("agent exists")
            .store
            .load_agent()
            .await
            .expect("state loads")
            .status,
        AgentStatus::Running
    );
}

#[tokio::test]
async fn resume_does_not_reconcile_started_only_state_without_a_run_id() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let store = FilesystemAgentStore::new(
        Arc::clone(&fixture.filesystem),
        format!("/history/{agent_id}").parse().expect("root parses"),
    );
    let turn_id = TurnId::new();
    store
        .update_state(
            AgentStatus::Running,
            None,
            Some(turn_id),
            Default::default(),
        )
        .await
        .expect("incomplete running state persists");
    let host = fixture.restore_host().await.expect("host restores");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let persisted = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(persisted.status, AgentStatus::Running);
    assert_eq!(persisted.run_id, None);
    assert_eq!(persisted.turn_id, Some(turn_id));
}

#[tokio::test]
async fn message_preamble_failure_marks_existing_agent_for_resume() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let filesystem = Arc::new(FailFirstMessageFilesystem::new(Arc::clone(
        &fixture.filesystem,
    )));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        filesystem as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "hello"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(host.agent(agent_id).expect("agent exists").needs_resume());
}

#[tokio::test]
async fn terminal_persistence_failure_cannot_be_overwritten_by_a_new_message() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let filesystem = Arc::new(FailTerminalStateFilesystem::new(Arc::clone(
        &fixture.filesystem,
    )));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    let app = router(Arc::clone(&host));

    let accepted = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "first"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let persisted = timeout(Duration::from_secs(1), async {
        loop {
            let state = host
                .agent(agent_id)
                .expect("agent exists")
                .store
                .load_agent()
                .await
                .expect("state loads");
            if state.status == AgentStatus::Running && state.last_seq == 1 {
                return state;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("running state is retained after terminal write failure");
    timeout(Duration::from_secs(1), async {
        while filesystem.failed_writes() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("terminal state write is attempted and fails");
    filesystem.recover();
    tokio::time::sleep(Duration::from_millis(25)).await;

    let response = app
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "second"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let response = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let response: Value = serde_json::from_slice(&response).expect("body is json");

    assert_eq!(response["error"]["code"], "resume_required");
    let after = host
        .agent(agent_id)
        .expect("agent exists")
        .store
        .load_agent()
        .await
        .expect("state loads");
    assert_eq!(after.run_id, persisted.run_id);
    assert_eq!(after.turn_id, persisted.turn_id);
    assert_eq!(after.last_seq, persisted.last_seq);
}

#[tokio::test]
async fn malformed_json_uses_the_unified_error_body() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;

    let (_, response) = fixture
        .request(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"text":"#))
                .expect("request builds"),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_request");
}

async fn rendered_error(error: HostError) -> (StatusCode, Value) {
    let response = error.into_response();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("error body is readable");
    let body = serde_json::from_slice(&body).expect("error body is json");
    (status, body)
}

#[tokio::test]
async fn resume_race_clears_marker_when_agent_observes_non_running_state() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Running)
        .await;
    let filesystem = Arc::new(ResumeRaceFilesystem::new(Arc::clone(&fixture.filesystem)));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    filesystem.enable();

    let response = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "resume_not_running");
    assert!(!host.agent(agent_id).expect("agent exists").needs_resume());
}

#[tokio::test]
async fn initialization_invariant_errors_use_stable_code() {
    let model = ModelId::new("openai", "missing").expect("model is valid");
    for error in [
        HostError::Agent(AgentError::MissingBuilderField { field: "store" }),
        HostError::Llm(LlmError::ProviderNotFound { model }),
    ] {
        let (status, body) = rendered_error(error).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "agent_initialization_failed");
    }
}

#[tokio::test]
async fn store_filesystem_errors_distinguish_unavailability_from_corruption() {
    let path: VirtualPath = "/history/agent/agent.json".parse().expect("path is valid");
    let unavailable = HostError::Store(StoreError::Filesystem(FilesystemError::LocalIo {
        operation: "read",
        path: path.clone(),
        source: io::Error::other("disk unavailable"),
    }));
    let corrupt_layout = HostError::Store(StoreError::Filesystem(FilesystemError::NotAFile {
        path: path.clone(),
    }));
    let invalid_path = HostError::Store(StoreError::Filesystem(
        FilesystemError::InvalidVirtualPath {
            path: "/history/../secret".to_owned(),
            source: wyse_filesystem::VirtualPathError,
        },
    ));

    let (unavailable_status, unavailable_body) = rendered_error(unavailable).await;
    assert_eq!(unavailable_status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(unavailable_body["error"]["code"], "store_unavailable");
    for error in [corrupt_layout, invalid_path] {
        let (status, body) = rendered_error(error).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "internal_error");
        assert!(!body.to_string().contains("secret"));
    }
}

#[tokio::test]
async fn direct_filesystem_errors_distinguish_operations_from_invariants() {
    let path: VirtualPath = "/history/agent/definition.toml"
        .parse()
        .expect("path is valid");
    for error in [
        FilesystemError::PermissionDenied { path: path.clone() },
        FilesystemError::LocalIo {
            operation: "write",
            path: path.clone(),
            source: io::Error::other("disk unavailable"),
        },
    ] {
        let (status, body) = rendered_error(HostError::Filesystem(error)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"]["code"], "store_unavailable");
    }

    for error in [
        FilesystemError::NotAFile { path: path.clone() },
        FilesystemError::InvalidVirtualPath {
            path: "/history/../secret".to_owned(),
            source: wyse_filesystem::VirtualPathError,
        },
    ] {
        let (status, body) = rendered_error(HostError::Filesystem(error)).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"]["code"], "internal_error");
        assert!(!body.to_string().contains("secret"));
    }
}

#[tokio::test]
async fn creation_filesystem_operations_map_to_store_unavailable() {
    for failure in [
        CreationOperationFailure::TemplateRead,
        CreationOperationFailure::HistoryCreate,
        CreationOperationFailure::DefinitionWrite,
    ] {
        let fixture = Fixture::new().await;
        fixture.persist_template("coding-agent", "\"echo\"");
        let filesystem: Arc<dyn Filesystem> = Arc::new(CreationOperationFilesystem {
            inner: Arc::clone(&fixture.filesystem),
            failure,
        });
        let mut providers = LlmProviderManager::new();
        providers
            .register(Arc::new(TestProvider(fixture.model.clone())))
            .expect("provider registers");
        let host = HostState::restore(
            fixture.config.clone(),
            filesystem,
            Arc::new(InMemoryEventStreamBus::default()),
            providers,
        )
        .await
        .expect("host restores");

        let response = router(host)
            .oneshot(
                Request::post("/v1/agents")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"agent_name": "coding-agent", "text": "hello"}).to_string(),
                    ))
                    .expect("request builds"),
            )
            .await
            .expect("request completes");

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable");
        let body: Value = serde_json::from_slice(&body).expect("body is json");
        assert_eq!(body["error"]["code"], "store_unavailable");
    }
}

#[tokio::test]
async fn approval_before_any_request_conflicts_while_provider_is_pending() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(PendingProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    let app = router(host);
    let accepted = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "hello"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    let approval_id = ApprovalId::new();
    let response = timeout(
        Duration::from_secs(1),
        app.oneshot(
            Request::post(format!("/v1/agents/{agent_id}/approvals/{approval_id}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"decision": "approve"}).to_string()))
                .expect("request builds"),
        ),
    )
    .await
    .expect("inactive approval should return immediately")
    .expect("request completes");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "approval_not_active");
}

#[tokio::test]
async fn concurrent_same_approval_id_accepts_once_and_conflicts_once() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(ApprovalProvider::new(fixture.model.clone())))
        .expect("provider registers");
    let bus = Arc::new(InMemoryEventStreamBus::default());
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        bus.clone() as Arc<dyn EventStreamBus>,
        providers,
    )
    .await
    .expect("host restores");
    let app = router(host);
    let accepted = app
        .clone()
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "use the tool"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);
    let mut events = bus
        .subscribe_agent(agent_id, ReplayStart::All)
        .await
        .expect("subscription opens");
    let approval_id = timeout(Duration::from_secs(1), async {
        loop {
            let envelope = events
                .next()
                .await
                .expect("approval event")
                .expect("event is valid")
                .envelope;
            if let RuntimeEvent::Agent {
                event: AgentEvent::ToolApprovalRequested { approval_id, .. },
                ..
            } = envelope.event
            {
                return approval_id;
            }
        }
    })
    .await
    .expect("approval request is published");
    let request = || {
        Request::post(format!("/v1/agents/{agent_id}/approvals/{approval_id}"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"decision": "approve"}).to_string()))
            .expect("request builds")
    };

    let (first, second) = tokio::join!(
        app.clone().oneshot(request()),
        app.clone().oneshot(request())
    );
    let first = first.expect("first request completes");
    let second = second.expect("second request completes");
    let (accepted, conflict) = if first.status() == StatusCode::NO_CONTENT {
        (first, second)
    } else {
        (second, first)
    };

    assert_eq!(accepted.status(), StatusCode::NO_CONTENT);
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    let body = to_bytes(conflict.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "approval_not_active");
}

fn event_record(agent_id: AgentId, cursor: u64, event: AgentEvent) -> EventRecord {
    EventRecord {
        cursor: EventCursor::from_transport_sequence(cursor),
        envelope: StreamEnvelope {
            business_seq: None,
            run_id: RunId::new(),
            timestamp: Utc::now(),
            source: EventSource::Run,
            event: RuntimeEvent::Agent { agent_id, event },
            metadata: BTreeMap::new(),
        },
    }
}

async fn get_events(
    fixture: &Fixture,
    bus: Arc<TestEventStreamBus>,
    uri: String,
    last_event_id: Option<&str>,
) -> axum::response::Response {
    let host = fixture
        .restore_host_with_bus(bus)
        .await
        .expect("host restores");
    let mut request = Request::get(uri);
    if let Some(last_event_id) = last_event_id {
        request = request.header("last-event-id", last_event_id);
    }
    router(host)
        .oneshot(request.body(Body::empty()).expect("request builds"))
        .await
        .expect("request completes")
}

#[tokio::test]
async fn event_stream_defaults_to_all_and_uses_sse_wire_fields() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let record = event_record(
        agent_id,
        7,
        AgentEvent::Started {
            turn_id: TurnId::new(),
        },
    );
    let expected_envelope = record.envelope.clone();
    let bus = Arc::new(TestEventStreamBus::with_events(vec![Ok(record)]));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body = std::str::from_utf8(&body).expect("SSE body is utf-8");
    assert!(body.contains("id: 7\n"));
    assert!(body.contains("event: started\n"));
    let data = body
        .lines()
        .find_map(|line| line.strip_prefix("data: "))
        .expect("SSE data field exists");
    let envelope: StreamEnvelope = serde_json::from_str(data).expect("SSE data is an envelope");
    assert_eq!(envelope, expected_envelope);
    assert_eq!(bus.replay_starts(), vec![ReplayStart::All]);
}

#[tokio::test]
async fn event_stream_replay_new_skips_retained_events() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events?replay=new"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(bus.replay_starts(), vec![ReplayStart::New]);
}

#[tokio::test]
async fn event_stream_replay_all_replays_retained_events() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events?replay=all"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(bus.replay_starts(), vec![ReplayStart::All]);
}

#[tokio::test]
async fn event_stream_after_cursor_resumes_after_query_cursor() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events?after_cursor=41&replay=new&replay=all"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        bus.replay_starts(),
        vec![ReplayStart::After(EventCursor::from_transport_sequence(41))]
    );
}

#[tokio::test]
async fn event_stream_invalid_after_cursor_takes_priority_over_replay() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events?after_cursor=invalid&replay=new"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_cursor");
    assert!(bus.replay_starts().is_empty());
}

#[tokio::test]
async fn event_stream_rejects_repeated_after_cursor_as_invalid_cursor() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events?after_cursor=40&after_cursor=41"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_cursor");
    assert!(bus.replay_starts().is_empty());
}

#[tokio::test]
async fn last_event_id_takes_priority_over_query_replay_options() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!(
            "/v1/agents/{agent_id}/events?after_cursor=invalid&after_cursor=duplicate&replay=unknown"
        ),
        Some("9"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        bus.replay_starts(),
        vec![ReplayStart::After(EventCursor::from_transport_sequence(9))]
    );
}

#[tokio::test]
async fn event_stream_rejects_an_invalid_cursor_without_subscribing() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let bus = Arc::new(TestEventStreamBus::with_events(Vec::new()));

    let response = get_events(
        &fixture,
        Arc::clone(&bus),
        format!("/v1/agents/{agent_id}/events?after_cursor=41"),
        Some("not-a-cursor"),
    )
    .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "invalid_cursor");
    assert!(bus.replay_starts().is_empty());
}

#[tokio::test]
async fn event_stream_returns_gone_before_body_for_an_expired_cursor() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let cursor = EventCursor::from_transport_sequence(3);
    let bus = Arc::new(TestEventStreamBus::with_error(
        EventStreamBusError::CursorExpired { cursor },
    ));

    let response = get_events(
        &fixture,
        bus,
        format!("/v1/agents/{agent_id}/events?after_cursor=3"),
        None,
    )
    .await;

    assert_eq!(response.status(), StatusCode::GONE);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body is readable");
    let body: Value = serde_json::from_slice(&body).expect("error body is json");
    assert_eq!(body["error"]["code"], "cursor_expired");
    assert!(!body.to_string().contains('3'));
}

#[tokio::test]
async fn event_stream_emits_one_safe_stream_error_then_closes() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let events = vec![
        Ok(event_record(
            agent_id,
            1,
            AgentEvent::Started {
                turn_id: TurnId::new(),
            },
        )),
        Err(EventStreamBusError::MissingAgentScope),
        Ok(event_record(
            agent_id,
            2,
            AgentEvent::Cancelled {
                usage: Default::default(),
            },
        )),
    ];
    let bus = Arc::new(TestEventStreamBus::with_events(events));

    let response = get_events(&fixture, bus, format!("/v1/agents/{agent_id}/events"), None).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body closes after the stream error");
    let body = std::str::from_utf8(&body).expect("SSE body is utf-8");
    assert_eq!(body.matches("event: stream_error\n").count(), 1);
    assert!(body.contains("\"code\":\"event_stream_unavailable\""));
    assert!(!body.contains("missing agent scope"));
    assert!(!body.contains("event: cancelled\n"));
}

#[tokio::test]
async fn shutdown_closes_a_pending_sse_stream() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let host = fixture
        .restore_host_with_bus(Arc::new(TestEventStreamBus::pending()))
        .await
        .expect("host restores");
    let response = router(Arc::clone(&host))
        .oneshot(
            Request::get(format!("/v1/agents/{agent_id}/events"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    let mut body = response.into_body().into_data_stream();

    host.shutdown().await;

    assert!(
        timeout(Duration::from_secs(1), body.next())
            .await
            .expect("SSE observes shutdown")
            .is_none()
    );
}

#[tokio::test]
async fn shutdown_stops_an_active_turn_and_waits_for_its_terminal_state() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(PendingProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    let accepted = router(Arc::clone(&host))
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "wait"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes");
    assert_eq!(accepted.status(), StatusCode::ACCEPTED);

    host.shutdown().await;

    assert_eq!(
        host.agent(agent_id)
            .expect("agent exists")
            .store
            .load_agent()
            .await
            .expect("state loads")
            .status,
        AgentStatus::Cancelled
    );
}

#[tokio::test]
async fn shutdown_cancels_a_pending_admitted_store_operation_within_the_drain_bound() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let filesystem = Arc::new(AdmissionFilesystem::new(Arc::clone(&fixture.filesystem)));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(PendingProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    filesystem.block_next_agent_get();
    let app = router(Arc::clone(&host));
    let request = tokio::spawn(async move {
        app.oneshot(
            Request::post(format!("/v1/agents/{agent_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(json!({"text": "wait"}).to_string()))
                .expect("request builds"),
        )
        .await
        .expect("request completes")
    });
    timeout(Duration::from_secs(1), filesystem.entered.notified())
        .await
        .expect("request enters Store preamble");

    let shutdown_host = Arc::clone(&host);
    let mut shutdown = tokio::spawn(async move { shutdown_host.shutdown().await });
    timeout(Duration::from_secs(2), &mut shutdown)
        .await
        .expect("shutdown is bounded while Store is pending")
        .expect("shutdown task succeeds");
    let response = request.await.expect("request task completes");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    filesystem.release();
    tokio::task::yield_now().await;
    assert_eq!(
        host.agent(agent_id)
            .expect("agent exists")
            .store
            .load_agent()
            .await
            .expect("state loads")
            .status,
        AgentStatus::Idle
    );
}

#[tokio::test]
async fn shutdown_rejects_new_create_message_and_resume_without_store_io() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Idle)
        .await;
    let filesystem = Arc::new(AdmissionFilesystem::new(Arc::clone(&fixture.filesystem)));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");
    host.shutdown().await;
    filesystem.reset_operations();
    let app = router(host);

    for request in [
        Request::post("/v1/agents")
            .header("content-type", "application/json")
            .body(Body::from(
                json!({"agent_name": "coding-agent", "text": "new"}).to_string(),
            ))
            .expect("create request builds"),
        Request::post(format!("/v1/agents/{agent_id}/messages"))
            .header("content-type", "application/json")
            .body(Body::from(json!({"text": "new"}).to_string()))
            .expect("message request builds"),
        Request::post(format!("/v1/agents/{agent_id}/resume"))
            .body(Body::empty())
            .expect("resume request builds"),
    ] {
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("request completes");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body is readable");
        let body: Value = serde_json::from_slice(&body).expect("body is json");
        assert_eq!(body["error"]["code"], "service_unavailable");
    }
    assert_eq!(filesystem.operations(), 0);
}

#[tokio::test]
async fn shutdown_preserves_uncertain_create_when_started_forwarding_is_pending() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let bus = Arc::new(PendingPublishBus::new());
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::clone(&bus) as Arc<dyn EventStreamBus>,
        providers,
    )
    .await
    .expect("host restores");
    let app = router(Arc::clone(&host));
    let request = tokio::spawn(async move {
        app.oneshot(
            Request::post("/v1/agents")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"agent_name": "coding-agent", "text": "new"}).to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes")
    });
    timeout(Duration::from_secs(1), bus.entered.notified())
        .await
        .expect("create reaches pending event publish");

    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .expect("shutdown is bounded while event publish is pending");
    let response = request.await.expect("request task completes");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    bus.release.notify_one();
    tokio::task::yield_now().await;

    let root = fs::read_dir(fixture.root.join("history"))
        .expect("history is readable")
        .next()
        .expect("uncertain create is preserved")
        .expect("agent directory is readable")
        .path();
    assert!(root.join("definition.toml").exists());
    assert!(root.join("agent.json").exists());
}

#[tokio::test]
async fn shutdown_preserves_a_created_agent_when_message_forwarding_is_pending() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let bus = Arc::new(PendingSecondPublishBus::new());
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::clone(&bus) as Arc<dyn EventStreamBus>,
        providers,
    )
    .await
    .expect("host restores");
    let app = router(Arc::clone(&host));
    let request = tokio::spawn(async move {
        app.oneshot(
            Request::post("/v1/agents")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({"agent_name": "coding-agent", "text": "durable"}).to_string(),
                ))
                .expect("request builds"),
        )
        .await
        .expect("request completes")
    });
    timeout(Duration::from_secs(1), bus.second_entered.notified())
        .await
        .expect("message is committed before forwarding blocks");
    let agent_id: AgentId = fs::read_dir(fixture.root.join("history"))
        .expect("history is readable")
        .next()
        .expect("agent directory exists")
        .expect("agent directory is readable")
        .file_name()
        .to_str()
        .expect("agent directory is utf-8")
        .parse()
        .expect("agent id parses");
    let root = fixture.root.join("history").join(agent_id.to_string());
    assert!(root.join("definition.toml").exists());
    assert!(root.join("agent.json").exists());
    assert!(root.join("messages/1.json").exists());

    timeout(Duration::from_secs(2), host.shutdown())
        .await
        .expect("shutdown remains bounded");
    let response = request.await.expect("request task completes");
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(root.join("definition.toml").exists());
    assert!(root.join("agent.json").exists());
    assert!(root.join("messages/1.json").exists());

    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let restored = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("preserved agent restores");
    assert!(
        restored
            .agent(agent_id)
            .expect("agent restores")
            .needs_resume()
    );
    let response = router(restored)
        .oneshot(
            Request::post(format!("/v1/agents/{agent_id}/resume"))
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("resume completes");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
}

#[tokio::test]
async fn shutdown_never_cleans_a_cancelled_in_flight_creation_mutation() {
    for stage in [
        PendingCreationStage::RootCreate,
        PendingCreationStage::DefinitionPut,
    ] {
        let fixture = Fixture::new().await;
        fixture.persist_template("coding-agent", "\"echo\"");
        let filesystem = Arc::new(PendingCreationFilesystem::new(
            Arc::clone(&fixture.filesystem) as Arc<dyn Filesystem>,
            stage,
        ));
        let mut providers = LlmProviderManager::new();
        providers
            .register(Arc::new(TestProvider(fixture.model.clone())))
            .expect("provider registers");
        let host = HostState::restore(
            fixture.config.clone(),
            Arc::clone(&filesystem) as Arc<dyn Filesystem>,
            Arc::new(InMemoryEventStreamBus::default()),
            providers,
        )
        .await
        .expect("host restores");
        let request = tokio::spawn({
            let host = Arc::clone(&host);
            async move {
                host.create_agent(
                    "coding-agent".parse().expect("name parses"),
                    "new".to_owned(),
                )
                .await
            }
        });
        timeout(Duration::from_secs(1), filesystem.entered.notified())
            .await
            .expect("creation mutation becomes pending after its side effect");
        let root = fs::read_dir(fixture.root.join("history"))
            .expect("history is readable")
            .next()
            .expect("agent directory exists")
            .expect("agent directory is readable")
            .path();

        timeout(Duration::from_secs(2), host.shutdown())
            .await
            .expect("shutdown remains bounded");
        assert!(matches!(
            request.await.expect("request task completes"),
            Err(HostError::HostShuttingDown)
        ));
        assert_eq!(filesystem.remove_operations.load(Ordering::SeqCst), 0);
        assert!(root.exists());
        if matches!(stage, PendingCreationStage::DefinitionPut) {
            assert!(root.join("definition.toml").exists());
        }
        filesystem.release.notify_one();
        tokio::task::yield_now().await;
        assert!(root.exists());
    }
}

#[tokio::test]
async fn creation_mutation_timeout_preserves_the_uncertain_side_effect() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");
    let filesystem = Arc::new(PendingCreationFilesystem::new(
        Arc::clone(&fixture.filesystem) as Arc<dyn Filesystem>,
        PendingCreationStage::DefinitionPut,
    ));
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");
    let host = HostState::restore(
        fixture.config.clone(),
        Arc::clone(&filesystem) as Arc<dyn Filesystem>,
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await
    .expect("host restores");

    let error = timeout(
        Duration::from_secs(4),
        host.create_agent(
            "coding-agent".parse().expect("name parses"),
            "new".to_owned(),
        ),
    )
    .await
    .expect("creation stage is internally bounded")
    .expect_err("pending mutation times out");

    assert!(matches!(error, HostError::CreationStageTimeout));
    assert_eq!(filesystem.remove_operations.load(Ordering::SeqCst), 0);
    let root = fs::read_dir(fixture.root.join("history"))
        .expect("history is readable")
        .next()
        .expect("uncertain agent directory is preserved")
        .expect("agent directory is readable")
        .path();
    assert!(root.join("definition.toml").exists());
    filesystem.release.notify_one();
    tokio::task::yield_now().await;
    assert!(root.join("definition.toml").exists());
}

#[test]
fn request_spans_and_final_error_log_use_only_safe_structured_fields() {
    let source = include_str!("../src/api.rs");
    for field in [
        "agent_id = field::Empty",
        "run_id = field::Empty",
        "cursor = field::Empty",
    ] {
        assert!(
            source.contains(field),
            "missing request span field: {field}"
        );
    }
    let error_log = source
        .split("tracing::error!(")
        .nth(1)
        .expect("HTTP boundary has one error log")
        .split(");")
        .next()
        .expect("error log closes");
    assert!(error_log.contains("http.status"));
    assert!(error_log.contains("error.code"));
    for sensitive in [
        "message",
        "prompt",
        "arguments",
        "api_key",
        "path",
        "source",
    ] {
        assert!(
            !error_log.contains(sensitive),
            "HTTP error log must not contain {sensitive}"
        );
    }
}
