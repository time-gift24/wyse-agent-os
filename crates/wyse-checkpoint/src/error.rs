//! Error types for agent checkpoint persistence.

use thiserror::Error;
use wyse_core::{AgentId, RunId, TurnId};
use wyse_filesystem::{CasUpdateError, FilesystemError};

/// Error returned by agent checkpoint operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CheckpointError {
    /// A checkpoint filesystem operation failed.
    #[error("checkpoint filesystem operation failed")]
    Filesystem(#[from] FilesystemError),
    /// The agent state file is missing.
    #[error("agent checkpoint is missing")]
    AgentMissing,
    /// The persisted state schema version is not supported.
    #[error("unsupported agent state version: {version}")]
    UnsupportedStateVersion {
        /// Unsupported schema version.
        version: u32,
    },
    /// The next message sequence cannot be represented.
    #[error("message sequence overflow")]
    SequenceOverflow,
    /// A checkpoint append input already has a business sequence.
    #[error("checkpoint append requires an unsequenced message")]
    MessageAlreadySequenced,
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
    #[error("checkpoint agent mismatch: expected {expected}, actual {actual}")]
    AgentMismatch {
        /// Agent identity required by the checkpoint.
        expected: AgentId,
        /// Agent identity found in persisted data.
        actual: AgentId,
    },
    /// A persisted event belongs to a different run.
    #[error("checkpoint run mismatch: expected {expected}, actual {actual}")]
    RunMismatch {
        /// Run identity required by the checkpoint.
        expected: RunId,
        /// Run identity found in persisted data.
        actual: RunId,
    },
    /// A persisted event belongs to a different turn.
    #[error("checkpoint turn mismatch: expected {expected}, actual {actual}")]
    TurnMismatch {
        /// Turn identity required by the checkpoint.
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
    /// A checkpoint message file contains another event type.
    #[error("checkpoint file does not contain an agent message")]
    UnexpectedMessageEvent,
    /// A checkpoint message filename does not encode a valid sequence.
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
    /// The checkpoint backend does not provide compare-and-swap.
    #[error("checkpoint backend does not support compare-and-swap")]
    CasUnsupported,
    /// The complete checkpoint compare-and-swap update timed out.
    #[error("checkpoint compare-and-swap timed out")]
    CasTimeout,
    /// Every permitted checkpoint compare-and-swap write conflicted.
    #[error("checkpoint compare-and-swap retries exhausted")]
    CasRetriesExhausted,
    /// Persisted agent state is malformed JSON.
    #[error("invalid agent state json")]
    DecodeState(#[source] serde_json::Error),
    /// A persisted message envelope is malformed JSON.
    #[error("invalid message envelope json")]
    DecodeMessage(#[source] serde_json::Error),
    /// Checkpoint state or a message could not be encoded as JSON.
    #[error("failed to encode checkpoint json")]
    Encode(#[source] serde_json::Error),
}

impl From<CasUpdateError<CheckpointError>> for CheckpointError {
    fn from(error: CasUpdateError<CheckpointError>) -> Self {
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
    fn cas_update_errors_map_to_checkpoint_domain_errors() {
        let unsupported = CheckpointError::from(CasUpdateError::<CheckpointError>::CasUnsupported);
        let timeout = CheckpointError::from(CasUpdateError::<CheckpointError>::Timeout);
        let exhausted = CheckpointError::from(CasUpdateError::<CheckpointError>::RetriesExhausted);
        let filesystem = CheckpointError::from(CasUpdateError::<CheckpointError>::Filesystem(
            FilesystemError::UnsupportedCas,
        ));
        let apply = CheckpointError::from(CasUpdateError::Apply(CheckpointError::SequenceOverflow));

        assert!(matches!(unsupported, CheckpointError::CasUnsupported));
        assert!(matches!(timeout, CheckpointError::CasTimeout));
        assert!(matches!(exhausted, CheckpointError::CasRetriesExhausted));
        assert!(matches!(
            filesystem,
            CheckpointError::Filesystem(FilesystemError::UnsupportedCas)
        ));
        assert!(matches!(apply, CheckpointError::SequenceOverflow));
    }
}
