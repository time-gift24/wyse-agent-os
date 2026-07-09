//! Checkpoint store public definitions.

use std::{fmt, str::FromStr};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wyse_core::{RunId, TurnId};

use crate::CheckpointError;

/// Identity of one checkpoint write.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CheckpointId(Uuid);

impl CheckpointId {
    /// Creates a new UUIDv7 checkpoint id.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// Returns the inner UUID.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for CheckpointId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for CheckpointId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl From<Uuid> for CheckpointId {
    fn from(value: Uuid) -> Self {
        Self(value)
    }
}

impl From<CheckpointId> for Uuid {
    fn from(value: CheckpointId) -> Self {
        value.0
    }
}

impl FromStr for CheckpointId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        value.parse::<Uuid>().map(Self)
    }
}

/// Type of checkpoint payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckpointKind {
    /// Agent turn checkpoint.
    Agent,
}

impl CheckpointKind {
    /// Returns the database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
        }
    }
}

/// Latest checkpoint status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CheckpointStatus {
    /// Runtime work is active.
    Running,
    /// Runtime work paused at a retryable failure.
    WaitingRetry,
    /// Runtime work finished successfully.
    Finished,
    /// Runtime work was cancelled.
    Cancelled,
}

impl CheckpointStatus {
    /// Returns the database representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingRetry => "waiting_retry",
            Self::Finished => "finished",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Latest checkpoint for one `(run_id, turn_id, kind)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointRecord {
    /// Workflow run identity.
    pub run_id: RunId,
    /// Resumable turn identity.
    pub turn_id: TurnId,
    /// Identity of this checkpoint write.
    pub checkpoint_id: CheckpointId,
    /// Payload kind.
    pub kind: CheckpointKind,
    /// Runtime status.
    pub status: CheckpointStatus,
    /// Version of the serialized state payload.
    pub state_version: u32,
    /// Serialized state payload bytes.
    pub state: Vec<u8>,
    /// Last stream sequence covered by this checkpoint.
    pub last_seq: u64,
    /// Last update time.
    pub updated_at: DateTime<Utc>,
}

/// Stores latest turn checkpoints.
#[async_trait]
pub trait CheckpointStore: Send + Sync {
    /// Upserts the latest checkpoint for `(run_id, turn_id, kind)`.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence fails.
    async fn put_latest(&self, record: CheckpointRecord) -> Result<(), CheckpointError>;

    /// Loads the latest checkpoint for `(run_id, turn_id, kind)`.
    ///
    /// # Errors
    ///
    /// Returns an error when persistence fails.
    async fn latest_turn(
        &self,
        run_id: RunId,
        turn_id: TurnId,
        kind: CheckpointKind,
    ) -> Result<Option<CheckpointRecord>, CheckpointError>;
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use wyse_core::{RunId, TurnId};

    use super::*;

    #[test]
    fn checkpoint_id_uses_uuid_v7() {
        let id = CheckpointId::new();

        assert_eq!(id.as_uuid().get_version_num(), 7);
    }

    #[test]
    fn checkpoint_record_keeps_turn_kind_key_fields() {
        let record = CheckpointRecord {
            run_id: RunId::new(),
            turn_id: TurnId::new(),
            checkpoint_id: CheckpointId::new(),
            kind: CheckpointKind::Agent,
            status: CheckpointStatus::Running,
            state_version: 1,
            state: br#"{"ok":true}"#.to_vec(),
            last_seq: 7,
            updated_at: Utc::now(),
        };

        assert_eq!(record.kind, CheckpointKind::Agent);
        assert_eq!(record.status, CheckpointStatus::Running);
        assert_eq!(record.state_version, 1);
        assert_eq!(record.last_seq, 7);
    }
}
