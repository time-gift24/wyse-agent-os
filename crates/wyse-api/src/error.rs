//! API host errors.

use std::str::Utf8Error;

use thiserror::Error;
use wyse_agent::AgentError;
use wyse_config::{AgentName, ConfigError};
use wyse_core::{AgentId, ToolName};
use wyse_filesystem::FilesystemError;
use wyse_infra::EventStreamBusError;
use wyse_llm::LlmError;
use wyse_store::StoreError;
use wyse_tools::ToolError;

/// Failure encountered while removing a partially created agent.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentCleanupError {
    /// The messages directory could not be listed.
    #[error("could not list agent messages during cleanup")]
    ListMessages(#[source] FilesystemError),
    /// A message file could not be removed.
    #[error("could not remove agent message during cleanup")]
    RemoveMessage(#[source] FilesystemError),
    /// The messages directory could not be removed.
    #[error("could not remove agent messages directory during cleanup")]
    RemoveMessagesDirectory(#[source] FilesystemError),
    /// The agent state file could not be removed.
    #[error("could not remove agent state during cleanup")]
    RemoveAgentState(#[source] FilesystemError),
    /// The resolved definition could not be removed.
    #[error("could not remove agent definition during cleanup")]
    RemoveDefinition(#[source] FilesystemError),
    /// The agent history directory could not be removed.
    #[error("could not remove agent directory during cleanup")]
    RemoveAgentDirectory(#[source] FilesystemError),
}

/// Error returned while composing or accessing the API host.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HostError {
    /// Shared configuration is invalid.
    #[error("configuration error")]
    Config(#[from] ConfigError),
    /// Filesystem access failed.
    #[error("filesystem operation failed")]
    Filesystem(#[from] FilesystemError),
    /// Agent store access failed.
    #[error("agent store operation failed")]
    Store(#[from] StoreError),
    /// Agent construction or history recovery failed.
    #[error("agent operation failed")]
    Agent(#[from] AgentError),
    /// LLM provider lookup failed.
    #[error("llm operation failed")]
    Llm(#[from] LlmError),
    /// Tool registry construction failed.
    #[error("tool operation failed")]
    Tool(#[from] ToolError),
    /// Event stream bus access failed.
    #[error("event stream bus operation failed")]
    EventStreamBus(#[from] EventStreamBusError),
    /// An agent is not hosted by this process.
    #[error("agent not found: {agent_id}")]
    AgentNotFound { agent_id: AgentId },
    /// An agent template does not exist.
    #[error("agent template not found: {agent_name:?}")]
    TemplateNotFound { agent_name: AgentName },
    /// Initial user text is empty after trimming.
    #[error("initial agent text must not be empty")]
    EmptyText,
    /// Agent creation failed and the partial state could not be fully removed.
    #[error("agent creation failed and cleanup failed: {cleanup}")]
    CreationCleanup {
        /// Original agent creation failure.
        #[source]
        creation: Box<HostError>,
        /// Cleanup operation that failed.
        cleanup: AgentCleanupError,
    },
    /// A definition requests a tool outside the builtin catalog.
    #[error("tool is not available: {name}")]
    ToolNotAvailable { name: ToolName },
    /// A history directory is not a canonical UUIDv7 agent id.
    #[error("invalid history directory: {name}")]
    InvalidHistoryDirectory { name: String },
    /// A persisted definition is not UTF-8.
    #[error("agent definition is not utf-8")]
    InvalidDefinitionEncoding {
        #[source]
        source: Utf8Error,
    },
    /// Directory, definition, and store identities do not agree.
    #[error("persisted agent identity mismatch for {expected_id}")]
    IdentityMismatch {
        expected_id: AgentId,
        actual_id: AgentId,
        expected_name: String,
        actual_name: String,
    },
}
