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
    http::{Request, StatusCode, header::LOCATION},
    response::IntoResponse,
};
use chrono::Utc;
use serde_json::{Value, json};
use tokio::time::timeout;
use tower::ServiceExt;
use wyse_agent::AgentError;
use wyse_api::{AgentCleanupError, AgentCreated, HostError, HostState, router};
use wyse_config::{AgentName, Config, ResolvedAgentDefinition};
use wyse_core::{
    AgentEvent, AgentId, ApprovalId, ChatMessage, EventSource, HistoryPage, ModelId, RunId,
    RuntimeEvent, StreamEnvelope, TurnId,
};
use wyse_filesystem::{
    CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, LocalFilesystem,
    LocalFilesystemConfig, RecordVersion, VersionedEntry, VirtualPath,
};
use wyse_infra::{EventStreamBus, event_stream_bus::InMemoryEventStreamBus};
use wyse_llm::{ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, LlmProviderManager};
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
            .initialize(agent_id, name.to_owned())
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

    async fn restore_host(&self) -> Result<Arc<HostState>, HostError> {
        let mut providers = LlmProviderManager::new();
        providers
            .register(Arc::new(TestProvider(self.model.clone())))
            .expect("provider registers");
        HostState::restore(
            self.config.clone(),
            Arc::clone(&self.filesystem),
            Arc::new(InMemoryEventStreamBus::default()) as Arc<dyn EventStreamBus>,
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

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

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

struct PendingProvider(ModelId);

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

struct FailFirstMessageFilesystem {
    inner: Arc<dyn Filesystem>,
    created_root: Mutex<Option<VirtualPath>>,
    cleanup_failure: Option<CleanupFailure>,
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
        }
    }

    fn failing_cleanup(inner: Arc<dyn Filesystem>, cleanup_failure: CleanupFailure) -> Self {
        Self {
            inner,
            created_root: Mutex::new(None),
            cleanup_failure: Some(cleanup_failure),
        }
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
}

#[async_trait]
impl Filesystem for FailFirstMessageFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        self.inner.get(path).await
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
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
async fn restore_marks_running_agents_as_needing_resume() {
    let fixture = Fixture::new().await;
    let agent_id = fixture
        .persist_agent("coding-agent", AgentStatus::Running)
        .await;

    let host = fixture.restore_host().await.expect("host restores");

    assert!(host.agent(agent_id).expect("agent exists").needs_resume());
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
async fn restore_rejects_definition_whose_model_was_removed() {
    let fixture = Fixture::new().await;
    fixture
        .persist_agent("coding-agent", AgentStatus::Finished)
        .await;
    let mut config = fixture.config.clone();
    config
        .llm
        .openai
        .as_mut()
        .expect("openai is configured")
        .models
        .clear();
    let mut providers = LlmProviderManager::new();
    providers
        .register(Arc::new(TestProvider(fixture.model.clone())))
        .expect("provider registers");

    let result = HostState::restore(
        config,
        Arc::clone(&fixture.filesystem),
        Arc::new(InMemoryEventStreamBus::default()),
        providers,
    )
    .await;
    let error = match result {
        Ok(_) => panic!("restore should fail"),
        Err(error) => error,
    };

    assert!(matches!(error, HostError::Config(_)));
}

#[tokio::test]
async fn create_agent_rejects_blank_text_without_creating_history() {
    let fixture = Fixture::new().await;
    fixture.persist_template("coding-agent", "\"echo\"");

    let response = fixture
        .post_agent(json!({"agent_name": "coding-agent", "text": " \n\t"}))
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
async fn create_agent_exposes_cleanup_list_and_remove_failures() {
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

        let HostError::CreationCleanup { creation, cleanup } = &error else {
            panic!("cleanup failure should be explicit");
        };
        assert!(matches!(creation.as_ref(), HostError::Agent(_)));
        assert!(matches!(
            (failure, cleanup),
            (
                CleanupFailure::ListMessages,
                AgentCleanupError::ListMessages(_)
            ) | (
                CleanupFailure::RemoveMessagesDirectory,
                AgentCleanupError::RemoveMessagesDirectory(_),
            )
        ));
        assert_eq!(
            std::error::Error::source(&error)
                .expect("creation failure is retained")
                .to_string(),
            "agent operation failed"
        );
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
        cleanup: AgentCleanupError::ListMessages(FilesystemError::PermissionDenied {
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
