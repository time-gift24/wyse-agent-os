//! Hosted agent registry and startup recovery.

use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::Notify,
    time::{sleep, timeout},
};
use tokio_util::sync::CancellationToken;

use wyse_agent::Agent;
use wyse_config::{AgentName, Config, ConfigError, ResolvedAgentDefinition};
use wyse_core::{AgentId, ChatMessage, DangerLevel, RunId, ToolKind};
use wyse_filesystem::{CasExpectation, Entry, FileType, Filesystem, FilesystemError, VirtualPath};
use wyse_infra::EventStreamBus;
use wyse_llm::LlmProviderManager;
use wyse_store::{AgentStatus, AgentStore, FilesystemAgentStore, StoreEventStreamBus};
use wyse_tools::{BuiltinToolRegistry, EchoTool, ToolPermissionMode, ToolRegistry};

use crate::{AgentCleanupError, HostError};

const HISTORY_ROOT: &str = "/history";
const TEMPLATE_ROOT: &str = "/templates";
const DEFINITION_FILE: &str = "definition.toml";
const SHUTDOWN_GRACE: Duration = Duration::from_secs(5);
const SHUTDOWN_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Shared runtime state for all recovered agents.
pub struct HostState {
    agents: RwLock<HashMap<AgentId, Arc<HostedAgent>>>,
    #[allow(
        dead_code,
        reason = "retained for the next API endpoint assembly tasks"
    )]
    filesystem: Arc<dyn Filesystem>,
    event_bus: Arc<dyn EventStreamBus>,
    #[allow(
        dead_code,
        reason = "retained for the next API endpoint assembly tasks"
    )]
    providers: Arc<LlmProviderManager>,
    #[allow(
        dead_code,
        reason = "retained for the next API endpoint assembly tasks"
    )]
    config: Arc<Config>,
    shutdown: CancellationToken,
    admission: Arc<AdmissionState>,
}

struct AdmissionState {
    closed: AtomicBool,
    active: AtomicUsize,
    drained: Notify,
}

pub(crate) struct AdmissionGuard {
    state: Arc<AdmissionState>,
}

/// One composed agent and its durable store.
pub struct HostedAgent {
    /// Agent runtime.
    pub agent: Agent,
    /// Durable state and complete message history.
    pub store: Arc<dyn AgentStore>,
    needs_resume: AtomicBool,
}

/// Result of creating and starting a hosted agent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct CreatedAgent {
    /// New agent identity.
    pub agent_id: AgentId,
    /// Resolved template name.
    pub agent_name: AgentName,
    /// Initial run identity.
    pub run_id: RunId,
}

impl HostedAgent {
    /// Returns whether startup found an interrupted running turn.
    #[must_use]
    pub fn needs_resume(&self) -> bool {
        self.needs_resume.load(Ordering::Acquire)
    }

    pub(crate) fn mark_needs_resume(&self) {
        self.needs_resume.store(true, Ordering::Release);
    }

    pub(crate) fn clear_needs_resume(&self) {
        self.needs_resume.store(false, Ordering::Release);
    }
}

impl AdmissionState {
    fn new() -> Self {
        Self {
            closed: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            drained: Notify::new(),
        }
    }

    fn acquire(self: &Arc<Self>) -> Result<AdmissionGuard, HostError> {
        if self.closed.load(Ordering::Acquire) {
            return Err(HostError::HostShuttingDown);
        }
        self.active.fetch_add(1, Ordering::AcqRel);
        if self.closed.load(Ordering::Acquire) {
            self.release();
            return Err(HostError::HostShuttingDown);
        }
        Ok(AdmissionGuard {
            state: Arc::clone(self),
        })
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    async fn wait_until_drained(&self) {
        loop {
            let notified = self.drained.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.active.load(Ordering::Acquire) == 0 {
                return;
            }
            notified.await;
        }
    }

    fn release(&self) {
        if self.active.fetch_sub(1, Ordering::AcqRel) == 1 {
            self.drained.notify_waiters();
        }
    }
}

impl Drop for AdmissionGuard {
    fn drop(&mut self) {
        self.state.release();
    }
}

impl HostState {
    /// Restores every persisted agent under `history/`.
    ///
    /// # Errors
    ///
    /// Returns [`HostError`] when any directory, definition, provider, tool, store, or
    /// complete history is invalid. No partial registry is returned.
    pub async fn restore(
        config: Config,
        filesystem: Arc<dyn Filesystem>,
        event_bus: Arc<dyn EventStreamBus>,
        providers: LlmProviderManager,
    ) -> Result<Arc<Self>, HostError> {
        let history_root = VirtualPath::try_from(HISTORY_ROOT).map_err(|source| {
            wyse_filesystem::FilesystemError::InvalidVirtualPath {
                path: HISTORY_ROOT.to_owned(),
                source,
            }
        })?;
        ensure_directory(filesystem.as_ref(), &history_root).await?;
        let template_root = VirtualPath::try_from(TEMPLATE_ROOT).map_err(|source| {
            wyse_filesystem::FilesystemError::InvalidVirtualPath {
                path: TEMPLATE_ROOT.to_owned(),
                source,
            }
        })?;
        ensure_directory(filesystem.as_ref(), &template_root).await?;
        let entries = filesystem.list_dir(&history_root).await?;
        let mut agents = HashMap::with_capacity(entries.len());

        for entry in entries {
            let agent_id = parse_history_entry(&entry)?;
            let root = agent_root(agent_id)?;
            let definition_path = child_path(&root, DEFINITION_FILE)?;
            let bytes = filesystem.read_file(&definition_path).await?;
            let input = std::str::from_utf8(&bytes)
                .map_err(|source| HostError::InvalidDefinitionEncoding { source })?;
            let definition = ResolvedAgentDefinition::parse(input)?;
            validate_definition_model(&config, &definition)?;
            let store: Arc<dyn AgentStore> =
                Arc::new(FilesystemAgentStore::new(Arc::clone(&filesystem), root));
            let state = store.load_agent().await?;
            let expected_name = definition.agent_name.as_str();
            if state.agent_id != agent_id || state.name != expected_name {
                return Err(HostError::IdentityMismatch {
                    expected_id: agent_id,
                    actual_id: state.agent_id,
                    expected_name: expected_name.to_owned(),
                    actual_name: state.name,
                });
            }

            let provider = providers.get(&definition.model)?;
            let registry = tool_registry(&definition)?;
            let agent_bus: Arc<dyn EventStreamBus> = Arc::new(StoreEventStreamBus::new(
                Arc::clone(&store),
                Arc::clone(&event_bus),
            ));
            let agent = Agent::builder()
                .id(agent_id)
                .name(expected_name)
                .system_prompt(definition.prompt)
                .llm_provider(provider)
                .tool_registry(registry)
                .event_bus(agent_bus)
                .store(Arc::clone(&store))
                .build()?;
            let needs_resume = state.status == AgentStatus::Running;
            if !needs_resume {
                agent.load_history().await?;
            }
            agents.insert(
                agent_id,
                Arc::new(HostedAgent {
                    agent,
                    store,
                    needs_resume: AtomicBool::new(needs_resume),
                }),
            );
        }

        Ok(Arc::new(Self {
            agents: RwLock::new(agents),
            filesystem,
            event_bus,
            providers: Arc::new(providers),
            config: Arc::new(config),
            shutdown: CancellationToken::new(),
            admission: Arc::new(AdmissionState::new()),
        }))
    }

    /// Returns a hosted agent without performing I/O.
    #[must_use]
    pub fn agent(&self, agent_id: AgentId) -> Option<Arc<HostedAgent>> {
        self.agents
            .read()
            .expect("host registry lock should not be poisoned")
            .get(&agent_id)
            .map(Arc::clone)
    }

    pub(crate) fn event_bus(&self) -> Arc<dyn EventStreamBus> {
        Arc::clone(&self.event_bus)
    }

    pub(crate) fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }

    pub(crate) fn admit(&self) -> Result<AdmissionGuard, HostError> {
        self.admission.acquire()
    }

    /// Cancels HTTP streams and active turns, then waits up to the bounded grace period for
    /// terminal state persistence. A timed-out running state remains durable for explicit resume.
    pub async fn shutdown(&self) {
        self.admission.close();
        self.shutdown.cancel();
        self.admission.wait_until_drained().await;
        let agents = self
            .agents
            .read()
            .expect("host registry lock should not be poisoned")
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for hosted in &agents {
            hosted.agent.stop();
        }

        let wait_for_terminal = async {
            loop {
                let mut running = false;
                for hosted in &agents {
                    match hosted.store.load_agent().await {
                        Ok(state) if state.status == AgentStatus::Running => running = true,
                        Ok(_) => {}
                        Err(_) => running = true,
                    }
                }
                if !running {
                    return;
                }
                sleep(SHUTDOWN_POLL_INTERVAL).await;
            }
        };
        if timeout(SHUTDOWN_GRACE, wait_for_terminal).await.is_err() {
            tracing::warn!(
                agent_count = agents.len(),
                grace_millis = SHUTDOWN_GRACE.as_millis(),
                "agent shutdown grace elapsed; running turns require durable resume"
            );
        }
    }

    pub(crate) fn allowed_origins(&self) -> &[String] {
        self.config
            .api
            .as_ref()
            .map_or(&[], |api| api.allowed_origins.as_slice())
    }

    /// Creates an agent and durably commits its initial user message before returning.
    ///
    /// # Errors
    ///
    /// Returns [`HostError`] when the text is blank, the template cannot be resolved,
    /// persistence or agent composition fails, or the required turn preamble cannot be
    /// committed.
    pub async fn create_agent(
        &self,
        agent_name: AgentName,
        text: String,
    ) -> Result<CreatedAgent, HostError> {
        let _admission = self.admit()?;
        if text.trim().is_empty() {
            return Err(HostError::EmptyText);
        }

        let template_path = template_path(&agent_name)?;
        let template = match self.filesystem.read_file(&template_path).await {
            Ok(template) => template,
            Err(FilesystemError::NotFound { .. }) => {
                return Err(HostError::TemplateNotFound { agent_name });
            }
            Err(error) => return Err(error.into()),
        };
        let template = std::str::from_utf8(&template)
            .map_err(|source| HostError::InvalidDefinitionEncoding { source })?;
        let definition = self.config.resolve_template(agent_name.clone(), template)?;
        let encoded_definition = definition.encode()?.into_bytes();
        let provider = self.providers.get(&definition.model)?;
        let registry = tool_registry(&definition)?;
        let agent_id = AgentId::new();
        let root = agent_root(agent_id)?;
        let definition_path = child_path(&root, DEFINITION_FILE)?;
        self.filesystem.create_dir(&root).await?;

        let result = async {
            self.filesystem
                .put(
                    &definition_path,
                    Entry::new(encoded_definition),
                    CasExpectation::Absent,
                )
                .await?;
            let store = Arc::new(FilesystemAgentStore::new(
                Arc::clone(&self.filesystem),
                root.clone(),
            ));
            store
                .initialize(agent_id, agent_name.as_str().to_owned())
                .await?;
            let store: Arc<dyn AgentStore> = store;
            let agent_bus: Arc<dyn EventStreamBus> = Arc::new(StoreEventStreamBus::new(
                Arc::clone(&store),
                Arc::clone(&self.event_bus),
            ));
            let agent = Agent::builder()
                .id(agent_id)
                .name(agent_name.as_str())
                .system_prompt(definition.prompt)
                .llm_provider(provider)
                .tool_registry(registry)
                .event_bus(agent_bus)
                .store(Arc::clone(&store))
                .build()?;
            let run_id = agent.run_turn(ChatMessage::user(text)).await?;
            let hosted = Arc::new(HostedAgent {
                agent,
                store,
                needs_resume: AtomicBool::new(false),
            });
            self.agents
                .write()
                .expect("host registry lock should not be poisoned")
                .insert(agent_id, hosted);
            Ok(CreatedAgent {
                agent_id,
                agent_name,
                run_id,
            })
        }
        .await;

        match result {
            Ok(created) => Ok(created),
            Err(creation) => {
                match cleanup_agent_files(self.filesystem.as_ref(), &root, &definition_path).await {
                    Ok(()) => Err(creation),
                    Err(cleanup) => Err(HostError::CreationCleanup {
                        creation: Box::new(creation),
                        cleanup,
                    }),
                }
            }
        }
    }
}

async fn ensure_directory(
    filesystem: &dyn Filesystem,
    path: &VirtualPath,
) -> Result<(), FilesystemError> {
    match filesystem.create_dir(path).await {
        Ok(()) | Err(FilesystemError::AlreadyExists { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn template_path(agent_name: &AgentName) -> Result<VirtualPath, HostError> {
    let path = format!("{TEMPLATE_ROOT}/{}.toml", agent_name.as_str());
    VirtualPath::try_from(path.as_str())
        .map_err(|source| FilesystemError::InvalidVirtualPath { path, source }.into())
}

async fn cleanup_agent_files(
    filesystem: &dyn Filesystem,
    root: &VirtualPath,
    definition_path: &VirtualPath,
) -> Result<(), AgentCleanupError> {
    let messages_path = child_path(root, "messages")
        .expect("agent message path should be valid after root validation");
    let entries = match filesystem.list_dir(&messages_path).await {
        Ok(entries) => entries,
        Err(FilesystemError::NotFound { .. }) => Vec::new(),
        Err(source) => return Err(AgentCleanupError::ListMessages(source)),
    };
    for entry in entries {
        ignore_not_found(filesystem.remove_file(&entry.path).await)
            .map_err(AgentCleanupError::RemoveMessage)?;
    }
    ignore_not_found(filesystem.remove_dir(&messages_path).await)
        .map_err(AgentCleanupError::RemoveMessagesDirectory)?;
    let agent_path = child_path(root, "agent.json")
        .expect("agent state path should be valid after root validation");
    ignore_not_found(filesystem.remove_file(&agent_path).await)
        .map_err(AgentCleanupError::RemoveAgentState)?;
    ignore_not_found(filesystem.remove_file(definition_path).await)
        .map_err(AgentCleanupError::RemoveDefinition)?;
    ignore_not_found(filesystem.remove_dir(root).await)
        .map_err(AgentCleanupError::RemoveAgentDirectory)?;
    Ok(())
}

fn ignore_not_found(result: Result<(), FilesystemError>) -> Result<(), FilesystemError> {
    match result {
        Ok(()) | Err(FilesystemError::NotFound { .. }) => Ok(()),
        Err(error) => Err(error),
    }
}

fn parse_history_entry(entry: &wyse_filesystem::DirEntry) -> Result<AgentId, HostError> {
    let agent_id = entry
        .file_name
        .parse::<AgentId>()
        .ok()
        .filter(|id| id.as_uuid().get_version_num() == 7)
        .filter(|id| id.to_string() == entry.file_name)
        .ok_or_else(|| HostError::InvalidHistoryDirectory {
            name: entry.file_name.clone(),
        })?;
    if entry.file_type != FileType::Directory || entry.path != agent_root(agent_id)? {
        return Err(HostError::InvalidHistoryDirectory {
            name: entry.file_name.clone(),
        });
    }
    Ok(agent_id)
}

fn agent_root(agent_id: AgentId) -> Result<VirtualPath, HostError> {
    let path = format!("{HISTORY_ROOT}/{agent_id}");
    VirtualPath::try_from(path.as_str()).map_err(|source| {
        wyse_filesystem::FilesystemError::InvalidVirtualPath { path, source }.into()
    })
}

fn child_path(root: &VirtualPath, child: &str) -> Result<VirtualPath, HostError> {
    let path = format!("{}/{child}", root.as_str());
    VirtualPath::try_from(path.as_str()).map_err(|source| {
        wyse_filesystem::FilesystemError::InvalidVirtualPath { path, source }.into()
    })
}

fn tool_registry(definition: &ResolvedAgentDefinition) -> Result<Arc<dyn ToolRegistry>, HostError> {
    let mut registry = BuiltinToolRegistry::new(ToolPermissionMode::RequireApproval);
    for name in &definition.tools {
        if name.as_str() != "echo" {
            return Err(HostError::ToolNotAvailable { name: name.clone() });
        }
        registry.register(Arc::new(EchoTool::new()), ToolKind::Read, DangerLevel::Low)?;
    }
    Ok(Arc::new(registry))
}

fn validate_definition_model(
    config: &Config,
    definition: &ResolvedAgentDefinition,
) -> Result<(), HostError> {
    let provider = match definition.model.provider_name() {
        "deepseek" => config.llm.deepseek.as_ref(),
        "openai" => config.llm.openai.as_ref(),
        _ => None,
    };
    if provider.is_some_and(|provider| {
        provider
            .models
            .iter()
            .any(|model| model == definition.model.model_name())
    }) {
        return Ok(());
    }
    Err(ConfigError::ModelNotConfigured {
        model: definition.model.clone(),
    }
    .into())
}
