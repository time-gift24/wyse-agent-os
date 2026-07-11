//! Error types for ontology domain operations.

use thiserror::Error;

use crate::{DraftName, LinkId, ObjectId, RevisionId, TagName};
use wyse_filesystem::FilesystemError;

/// Error returned by ontology domain operations.
#[derive(Debug, Error)]
pub enum OntologyError {
    /// A draft name does not satisfy the identifier format.
    #[error(
        "draft name must be 1-64 ASCII letters, digits, underscores, or hyphens and start with a letter or digit"
    )]
    InvalidDraftName,
    /// A tag name does not satisfy the identifier format.
    #[error(
        "tag name must be 1-64 ASCII letters, digits, underscores, or hyphens and start with a letter or digit"
    )]
    InvalidTagName,
    /// A revision id is not a lowercase SHA-256 digest.
    #[error("revision id must be a 64-character lowercase hexadecimal SHA-256 digest")]
    InvalidRevisionId,
    /// A schema violates one or more structural invariants.
    #[error("schema is invalid")]
    SchemaInvalid {
        /// Every discovered validation failure.
        diagnostics: Vec<String>,
    },
    /// Object values do not match the selected schema.
    #[error("object values are invalid")]
    ValueInvalid {
        /// Every discovered validation failure.
        diagnostics: Vec<String>,
    },
    /// Existing instances prevent a draft from being published.
    #[error("draft cannot be published because existing instances are invalid")]
    PublishInvalid {
        /// Every discovered validation failure.
        diagnostics: Vec<String>,
    },
    /// A requested draft does not exist.
    #[error("draft {name} does not exist")]
    DraftMissing {
        /// Name of the missing draft.
        name: DraftName,
    },
    /// A draft changed since the caller's expected digest.
    #[error("draft {name} changed concurrently")]
    DraftConflict {
        /// Name of the conflicted draft.
        name: DraftName,
    },
    /// The filesystem backend does not support the required CAS operations.
    #[error("draft filesystem does not support compare-and-swap")]
    DraftCasUnsupported,
    /// A published revision does not exist.
    #[error("revision {id} does not exist")]
    RevisionMissing {
        /// Identity of the missing revision.
        id: RevisionId,
    },
    /// A schema tag does not exist.
    #[error("tag {name} does not exist")]
    TagMissing {
        /// Name of the missing tag.
        name: TagName,
    },
    /// The reserved online tag cannot be deleted.
    #[error("the online tag cannot be deleted")]
    ReservedTag,
    /// An object instance does not exist.
    #[error("object {id} does not exist")]
    ObjectMissing {
        /// Identity of the missing object.
        id: ObjectId,
    },
    /// A link instance does not exist.
    #[error("link {id} does not exist")]
    LinkMissing {
        /// Identity of the missing link.
        id: LinkId,
    },
    /// A repository operation failed.
    #[error("ontology repository operation failed")]
    Repository(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// A persisted draft schema is not valid JSON.
    #[error("failed to decode draft schema")]
    DecodeSchema(#[source] serde_json::Error),
    /// A schema cannot be encoded as its canonical JSON form.
    #[error("failed to encode schema")]
    EncodeSchema(#[source] serde_json::Error),
    /// A filesystem operation failed.
    #[error("draft filesystem operation failed")]
    Filesystem(#[source] FilesystemError),
}
