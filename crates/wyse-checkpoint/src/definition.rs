//! Agent checkpoint interface.

use async_trait::async_trait;
use wyse_core::{HistoryPage, HistoryQuery, RunId, StreamEnvelope, TokenUsage, TurnId};

use crate::{AgentState, AgentStatus, CheckpointError};

/// Persists the state and complete message history of one injected agent.
#[async_trait]
pub trait AgentCheckpoint: Send + Sync {
    /// Loads the current persisted agent state.
    ///
    /// # Errors
    ///
    /// Returns an error when state is missing, malformed, unsupported, or cannot be read.
    async fn load_agent(&self) -> Result<AgentState, CheckpointError>;

    /// Replaces the agent's mutable runtime state.
    ///
    /// # Errors
    ///
    /// Returns an error when the state update cannot be committed.
    async fn update_state(
        &self,
        status: AgentStatus,
        run_id: Option<RunId>,
        turn_id: Option<TurnId>,
        usage: TokenUsage,
    ) -> Result<AgentState, CheckpointError>;

    /// Commits an unsequenced complete agent message and returns its sequenced envelope.
    ///
    /// # Errors
    ///
    /// Returns an error when the message is invalid or cannot be committed atomically.
    async fn append_message(
        &self,
        envelope: StreamEnvelope,
    ) -> Result<StreamEnvelope, CheckpointError>;

    /// Loads one fixed-range page of committed complete messages.
    ///
    /// # Errors
    ///
    /// Returns an error when the query or persisted message history is invalid.
    async fn history_page(&self, query: HistoryQuery) -> Result<HistoryPage, CheckpointError>;
}
