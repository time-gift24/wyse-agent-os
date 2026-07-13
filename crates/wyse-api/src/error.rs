//! API host errors.

use std::str::Utf8Error;

use thiserror::Error;
use wyse_agent::AgentError;
use wyse_config::{AgentName, ConfigError};
use wyse_core::{AgentId, ModelId, ModelIdParseError, ToolName};
use wyse_filesystem::FilesystemError;
use wyse_infra::EventStreamBusError;
use wyse_llm::LlmError;
use wyse_store::StoreError;
use wyse_tools::ToolError;

/// Failure encountered while removing a partially created agent.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AgentCleanupError {
    /// Cleanup did not finish within the shutdown bound.
    #[error("agent cleanup timed out")]
    Timeout,
    /// A partial agent file could not be inspected or removed.
    #[error("could not clean up partial agent files")]
    Filesystem(#[from] FilesystemError),
}

/// Error returned while composing or accessing the API host.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HostError {
    /// Host configuration or listener I/O failed.
    #[error("host io operation failed")]
    Io(#[from] std::io::Error),
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
    /// A follow-up user message is empty after trimming.
    #[error("agent message must not be empty")]
    InvalidMessage,
    /// Provider-specific model parameters do not match the configured model.
    #[error("model parameters are invalid")]
    InvalidModelParameters,
    /// An HTTP request body or path parameter is invalid.
    #[error("request is invalid")]
    InvalidRequest,
    /// An HTTP request body exceeded the configured limit.
    #[error("request body is too large")]
    MessageTooLarge,
    /// A history query could not be decoded.
    #[error("history query is invalid")]
    InvalidHistoryQuery,
    /// An event replay cursor could not be decoded.
    #[error("event cursor is invalid")]
    InvalidCursor,
    /// A persisted running turn must be resumed before other run control.
    #[error("agent has an unfinished persisted turn")]
    ResumeRequired,
    /// The host has started shutdown and no longer accepts durable work.
    #[error("host is shutting down")]
    HostShuttingDown,
    /// An agent creation persistence stage did not finish within its fixed safety bound.
    #[error("agent creation persistence stage timed out")]
    CreationStageTimeout,
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
    /// A configured provider model name could not form a model id.
    #[error("invalid configured model for provider {provider}")]
    InvalidConfiguredModel {
        provider: &'static str,
        model: String,
        #[source]
        source: ModelIdParseError,
    },
    /// A DeepSeek model is configured but unsupported by the existing adapter.
    #[error("unsupported deepseek model: {model}")]
    UnsupportedDeepSeekModel { model: ModelId },
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
