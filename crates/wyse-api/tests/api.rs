use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode, header::LOCATION},
};
use chrono::Utc;
use serde_json::{Value, json};
use tower::ServiceExt;
use wyse_api::{AgentCreated, HostError, HostState, router};
use wyse_config::{AgentName, Config, ResolvedAgentDefinition};
use wyse_core::{
    AgentEvent, AgentId, ChatMessage, EventSource, ModelId, RunId, RuntimeEvent, StreamEnvelope,
    TurnId,
};
use wyse_filesystem::{
    CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, LocalFilesystem,
    LocalFilesystemConfig, RecordVersion, VersionedEntry, VirtualPath,
};
use wyse_infra::{EventStreamBus, event_stream_bus::InMemoryEventStreamBus};
use wyse_llm::{ChatRequest, ChatResponse, ChatStream, LlmError, LlmProvider, LlmProviderManager};
use wyse_store::{AgentStatus, AgentStore, FilesystemAgentStore};

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
        if status == AgentStatus::Running {
            store
                .update_state(status, Some(run_id), Some(turn_id), Default::default())
                .await
                .expect("state updates");
        } else if status != AgentStatus::Idle {
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

struct FailFirstMessageFilesystem {
    inner: Arc<dyn Filesystem>,
    created_root: Mutex<Option<VirtualPath>>,
}

impl FailFirstMessageFilesystem {
    fn new(inner: Arc<dyn Filesystem>) -> Self {
        Self {
            inner,
            created_root: Mutex::new(None),
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
