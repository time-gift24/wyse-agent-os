//! Persisted agent state.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use wyse_core::{AgentId, RunId, TokenUsage, TurnId};

/// Current serialized agent-state schema version.
pub const AGENT_STATE_VERSION: u32 = 1;

/// Maximum number of messages returned by one history page.
pub const MAX_HISTORY_PAGE_SIZE: usize = 256;

/// Persisted runtime status of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// The agent is ready for work.
    Idle,
    /// The agent is actively processing a turn.
    Running,
    /// The agent finished its work.
    Finished,
    /// The agent failed and cannot retry automatically.
    Failed,
    /// The agent was cancelled.
    Cancelled,
}

/// Strict persisted state for one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentState {
    /// Serialized state schema version.
    pub state_version: u32,
    /// Agent identity.
    pub agent_id: AgentId,
    /// Human-readable agent name.
    pub name: String,
    /// Current runtime status.
    pub status: AgentStatus,
    /// Active workflow run, when any.
    pub run_id: Option<RunId>,
    /// Active resumable turn, when any.
    pub turn_id: Option<TurnId>,
    /// Next LLM loop iteration that has not reached a durable boundary.
    pub next_iteration: u64,
    /// Cumulative model token usage.
    pub usage: TokenUsage,
    /// Last committed message sequence.
    pub last_seq: u64,
    /// Last state update time.
    pub updated_at: DateTime<Utc>,
}

impl AgentState {
    /// Creates idle state for a new agent.
    #[must_use]
    pub fn new(agent_id: AgentId, name: String) -> Self {
        Self {
            state_version: AGENT_STATE_VERSION,
            agent_id,
            name,
            status: AgentStatus::Idle,
            run_id: None,
            turn_id: None,
            next_iteration: 0,
            usage: TokenUsage::default(),
            last_seq: 0,
            updated_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::json;
    use wyse_core::AgentId;

    use super::*;

    #[test]
    fn agent_state_serializes_only_approved_fields() {
        let state = AgentState::new(AgentId::new(), "writer".to_owned());
        assert_eq!(state.next_iteration, 0);
        let value = serde_json::to_value(state).expect("serialize state");
        let keys = value
            .as_object()
            .expect("state object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();

        assert_eq!(
            keys,
            BTreeSet::from([
                "agent_id".to_owned(),
                "last_seq".to_owned(),
                "name".to_owned(),
                "next_iteration".to_owned(),
                "run_id".to_owned(),
                "state_version".to_owned(),
                "status".to_owned(),
                "turn_id".to_owned(),
                "updated_at".to_owned(),
                "usage".to_owned(),
            ])
        );
    }

    #[test]
    fn agent_state_rejects_unknown_fields() {
        let mut value = serde_json::to_value(AgentState::new(AgentId::new(), "writer".to_owned()))
            .expect("serialize state");
        value
            .as_object_mut()
            .expect("state object")
            .insert("owner_id".to_owned(), json!("x"));

        assert!(serde_json::from_value::<AgentState>(value).is_err());
    }
}
