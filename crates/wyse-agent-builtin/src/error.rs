//! Error types for built-in agent wiring.

use thiserror::Error;
use wyse_agent::AgentError;
use wyse_core::ModelId;

/// Error returned while wiring a default agent.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DefaultAgentError {
    /// The model reference selects a provider unavailable in this crate.
    #[error("unsupported model provider: {provider}")]
    UnsupportedProvider { provider: String },
    /// The model reference selects a DeepSeek model unsupported by `wyse-llm`.
    #[error("unsupported deepseek model: {model}")]
    UnsupportedDeepSeekModel { model: ModelId },
    /// The agent builder rejected the supplied wiring.
    #[error("failed to build default agent")]
    Agent(#[from] AgentError),
}
