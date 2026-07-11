//! Error types for agent store persistence.

use thiserror::Error;
use wyse_core::{AgentId, ChatRole, RunId, TurnId};
use wyse_filesystem::{CasUpdateError, FilesystemError};

use crate::AgentStatus;

/// Error returned by agent store operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StoreError {
    /// A store filesystem operation failed.
    #[error("store filesystem operation failed")]
    Filesystem(#[from] FilesystemError),
    /// The agent state file is missing.
    #[error("agent store is missing")]
    AgentMissing,
    /// The persisted state schema version is not supported.
    #[error("unsupported agent state version: {version}")]
    UnsupportedStateVersion {
        /// Unsupported schema version.
        version: u32,
    },
    /// An iteration can only complete while the agent is running.
    #[error("agent is not running: {actual:?}")]
    AgentNotRunning {
        /// Current persisted agent status.
        actual: AgentStatus,
    },
    /// The requested iteration differs from the durable frontier.
    #[error("iteration mismatch: expected {expected}, actual {actual}")]
    IterationMismatch {
        /// Current durable iteration frontier.
        expected: u64,
        /// Requested iteration.
        actual: u64,
    },
    /// The next iteration cannot be represented.
    #[error("iteration overflow")]
    IterationOverflow,
    /// The next message sequence cannot be represented.
    #[error("message sequence overflow")]
    SequenceOverflow,
    /// A store append input already has a business sequence.
    #[error("store append requires an unsequenced message")]
    MessageAlreadySequenced,
    /// A message role cannot be committed to store history.
    #[error("invalid store message role: {role:?}")]
    InvalidMessageRole {
        /// Rejected message role.
        role: ChatRole,
    },
    /// The requested history page size is outside the supported range.
    #[error("history limit must be between 1 and {maximum}: {actual}")]
    InvalidHistoryLimit {
        /// Requested page size.
        actual: usize,
        /// Maximum supported page size.
        maximum: usize,
    },
    /// The exclusive history front exceeds its inclusive barrier.
    #[error("history front {after_seq} exceeds barrier {through_seq}")]
    InvalidHistoryRange {
        /// Exclusive lower sequence bound.
        after_seq: u64,
        /// Inclusive upper sequence bound.
        through_seq: u64,
    },
    /// The requested history barrier exceeds the committed sequence.
    #[error("history barrier {through_seq} exceeds committed sequence {last_seq}")]
    HistoryBarrierBeyondLast {
        /// Requested inclusive upper sequence bound.
        through_seq: u64,
        /// Last committed message sequence.
        last_seq: u64,
    },
    /// Persisted state or an event belongs to a different agent.
    #[error("store agent mismatch: expected {expected}, actual {actual}")]
    AgentMismatch {
        /// Agent identity required by the store.
        expected: AgentId,
        /// Agent identity found in persisted data.
        actual: AgentId,
    },
    /// A persisted event belongs to a different run.
    #[error("store run mismatch: expected {expected}, actual {actual}")]
    RunMismatch {
        /// Run identity required by the store.
        expected: RunId,
        /// Run identity found in persisted data.
        actual: RunId,
    },
    /// A persisted event belongs to a different turn.
    #[error("store turn mismatch: expected {expected}, actual {actual}")]
    TurnMismatch {
        /// Turn identity required by the store.
        expected: TurnId,
        /// Turn identity found in persisted data.
        actual: TurnId,
    },
    /// A message path sequence differs from its event sequence.
    #[error("message sequence mismatch: path {path_seq}, event {event_seq}")]
    MessageSequenceMismatch {
        /// Sequence encoded in the file path.
        path_seq: u64,
        /// Sequence encoded in the event.
        event_seq: u64,
    },
    /// A message within the committed range is absent.
    #[error("committed message is missing: {seq}")]
    MissingCommittedMessage {
        /// Missing committed sequence.
        seq: u64,
    },
    /// A store message file contains another event type.
    #[error("store file does not contain an agent message")]
    UnexpectedMessageEvent,
    /// A store message filename does not encode a valid sequence.
    #[error("invalid message filename: {file_name}")]
    InvalidMessageFilename {
        /// Invalid filename.
        file_name: String,
    },
    /// A message exists beyond the allowed committed frontier.
    #[error("message {seq} exists beyond allowed frontier {frontier}")]
    MessageBeyondFrontier {
        /// Unexpected message sequence.
        seq: u64,
        /// Maximum allowed message sequence.
        frontier: u64,
    },
    /// The store backend does not provide compare-and-swap.
    #[error("store backend does not support compare-and-swap")]
    CasUnsupported,
    /// The complete store compare-and-swap update timed out.
    #[error("store compare-and-swap timed out")]
    CasTimeout,
    /// Every permitted store compare-and-swap write conflicted.
    #[error("store compare-and-swap retries exhausted")]
    CasRetriesExhausted,
    /// Persisted agent state is malformed JSON.
    #[error("invalid agent state json")]
    DecodeState(#[source] serde_json::Error),
    /// A persisted message envelope is malformed JSON.
    #[error("invalid message envelope json")]
    DecodeMessage(#[source] serde_json::Error),
    /// Store state or a message could not be encoded as JSON.
    #[error("failed to encode store json")]
    Encode(#[source] serde_json::Error),
}

impl From<CasUpdateError<StoreError>> for StoreError {
    fn from(error: CasUpdateError<StoreError>) -> Self {
        match error {
            CasUpdateError::CasUnsupported => Self::CasUnsupported,
            CasUpdateError::Timeout => Self::CasTimeout,
            CasUpdateError::RetriesExhausted => Self::CasRetriesExhausted,
            CasUpdateError::Filesystem(source) => Self::Filesystem(source),
            CasUpdateError::Apply(error) => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use wyse_filesystem::{CasUpdateError, FilesystemError};

    use super::*;

    #[test]
    fn cas_update_errors_map_to_store_domain_errors() {
        let unsupported = StoreError::from(CasUpdateError::<StoreError>::CasUnsupported);
        let timeout = StoreError::from(CasUpdateError::<StoreError>::Timeout);
        let exhausted = StoreError::from(CasUpdateError::<StoreError>::RetriesExhausted);
        let filesystem = StoreError::from(CasUpdateError::<StoreError>::Filesystem(
            FilesystemError::UnsupportedCas,
        ));
        let apply = StoreError::from(CasUpdateError::Apply(StoreError::SequenceOverflow));

        assert!(matches!(unsupported, StoreError::CasUnsupported));
        assert!(matches!(timeout, StoreError::CasTimeout));
        assert!(matches!(exhausted, StoreError::CasRetriesExhausted));
        assert!(matches!(
            filesystem,
            StoreError::Filesystem(FilesystemError::UnsupportedCas)
        ));
        assert!(matches!(apply, StoreError::SequenceOverflow));
    }
}
