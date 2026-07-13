//! Agent store interface.

use async_trait::async_trait;
use stratum_core::{
    HistoryPage, HistoryQuery, ModelConfig, RunId, StreamEnvelope, TokenUsage, TurnId,
};

use crate::{AgentState, AgentStatus, StoreError};

/// Persists the state and complete message history of one injected agent.
#[async_trait]
pub trait AgentStore: Send + Sync {
    /// Loads the current persisted agent state.
    ///
    /// # Errors
    ///
    /// Returns an error when state is missing, malformed, unsupported, or cannot be read.
    async fn load_agent(&self) -> Result<AgentState, StoreError>;

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
    ) -> Result<AgentState, StoreError>;

    /// Starts a turn with its stable model configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the state update cannot be committed.
    async fn start_turn(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        model_config: ModelConfig,
    ) -> Result<AgentState, StoreError> {
        let _ = model_config;
        self.update_state(
            AgentStatus::Running,
            Some(run_id),
            Some(turn_id),
            TokenUsage::default(),
        )
        .await
    }

    /// Atomically advances the durable iteration frontier for the active turn.
    ///
    /// # Errors
    ///
    /// Returns an error when the state is not the expected running iteration or the update cannot
    /// be committed.
    async fn complete_iteration(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        iteration: u64,
        usage: TokenUsage,
    ) -> Result<AgentState, StoreError>;

    /// Commits an unsequenced complete agent message and returns its sequenced envelope.
    ///
    /// # Errors
    ///
    /// Returns an error when the message is invalid or cannot be committed atomically.
    async fn append_message(&self, envelope: StreamEnvelope) -> Result<StreamEnvelope, StoreError>;

    /// Loads one fixed-range page of committed complete messages.
    ///
    /// # Errors
    ///
    /// Returns an error when the query or persisted message history is invalid.
    async fn history_page(&self, query: HistoryQuery) -> Result<HistoryPage, StoreError>;
}
