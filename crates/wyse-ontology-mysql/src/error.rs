//! Errors produced while decoding persisted MySQL data.

use thiserror::Error;

/// Error source retained by [`wyse_ontology::OntologyError::Repository`].
#[derive(Debug, Error)]
pub(crate) enum MySqlOntologyRepositoryError {
    /// A persisted JSON document could not be decoded.
    #[error("failed to decode persisted {kind}")]
    DecodeJson {
        /// Kind of persisted JSON document.
        kind: &'static str,
        /// Original serialization failure.
        #[source]
        source: serde_json::Error,
    },
    /// A persisted typed value did not satisfy its domain format.
    #[error("persisted {kind} is invalid")]
    InvalidPersisted {
        /// Domain value that failed validation.
        kind: &'static str,
    },
    /// The database operation failed.
    #[error("mysql operation failed")]
    Sqlx(#[source] sqlx::Error),
}
