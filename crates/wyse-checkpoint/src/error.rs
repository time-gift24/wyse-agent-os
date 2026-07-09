//! Error types for checkpoint persistence.

use thiserror::Error;

/// Error returned by checkpoint operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CheckpointError {
    /// SQLite operation failed.
    #[error("sqlite checkpoint operation failed")]
    Sqlite(#[source] rusqlite::Error),
    /// Database contained an unknown checkpoint kind.
    #[error("unknown checkpoint kind: {value}")]
    UnknownKind {
        /// Stored kind value.
        value: String,
    },
    /// Database contained an unknown checkpoint status.
    #[error("unknown checkpoint status: {value}")]
    UnknownStatus {
        /// Stored status value.
        value: String,
    },
    /// ID field in storage was invalid.
    #[error("invalid checkpoint id field: {field}")]
    InvalidId {
        /// Invalid field name.
        field: &'static str,
        /// Underlying UUID parser error.
        #[source]
        source: uuid::Error,
    },
    /// Sequence number cannot be stored in SQLite integer form.
    #[error("checkpoint sequence is too large: {value}")]
    InvalidSequence {
        /// Sequence value that did not fit.
        value: u64,
    },
    /// Blocking checkpoint task failed.
    #[error("blocking checkpoint task failed")]
    BlockingTask {
        /// Underlying join error.
        #[source]
        source: tokio::task::JoinError,
    },
}

impl From<rusqlite::Error> for CheckpointError {
    fn from(source: rusqlite::Error) -> Self {
        Self::Sqlite(source)
    }
}
