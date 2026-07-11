//! Error types for ontology domain operations.

use thiserror::Error;

use crate::{
    DraftName, LinkId, LinkTypeId, ObjectId, ObjectTypeId, PropertyTypeId, RevisionId, TagName,
};
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
    /// A revision identity does not match its canonical schema content.
    #[error("revision id does not match its schema content")]
    RevisionIdentityMismatch {
        /// The digest computed from the schema.
        expected: RevisionId,
        /// The identity supplied with the revision.
        actual: RevisionId,
    },
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
    /// The online tag changed after a write was validated against it.
    #[error("the online schema changed concurrently")]
    OnlineRevisionChanged,
    /// An object instance does not exist.
    #[error("object {id} does not exist")]
    ObjectMissing {
        /// Identity of the missing object.
        id: ObjectId,
    },
    /// An object changed since the caller's expected version.
    #[error("object {id} changed concurrently")]
    ObjectVersionConflict {
        /// Identity of the concurrently changed object.
        id: ObjectId,
    },
    /// An object cannot be deleted while links still reference it.
    #[error("object {id} is still referenced by links")]
    ObjectReferenced {
        /// Identity of the referenced object.
        id: ObjectId,
    },
    /// A selected schema does not define the requested object type.
    #[error("object type {id} does not exist in the selected schema")]
    ObjectTypeMissing {
        /// Identity of the missing object type.
        id: ObjectTypeId,
    },
    /// A selected object type does not define the requested property type.
    #[error("property type {id} does not exist in object type {object_type_id}")]
    PropertyTypeMissing {
        /// Identity of the object type.
        object_type_id: ObjectTypeId,
        /// Identity of the missing property type.
        id: PropertyTypeId,
    },
    /// A link instance does not exist.
    #[error("link {id} does not exist")]
    LinkMissing {
        /// Identity of the missing link.
        id: LinkId,
    },
    /// A link changed since the caller's expected version.
    #[error("link {id} changed concurrently")]
    LinkVersionConflict {
        /// Identity of the concurrently changed link.
        id: LinkId,
    },
    /// A selected schema does not define the requested link type.
    #[error("link type {id} does not exist in the selected schema")]
    LinkTypeMissing {
        /// Identity of the missing link type.
        id: LinkTypeId,
    },
    /// Link endpoints do not satisfy the selected link type.
    #[error("link endpoints are invalid")]
    LinkEndpointInvalid {
        /// Every discovered endpoint failure.
        diagnostics: Vec<String>,
    },
    /// A new or replaced link would violate its cardinality.
    #[error("link type {link_type_id} cardinality would be violated")]
    CardinalityConflict {
        /// Identity of the constrained link type.
        link_type_id: LinkTypeId,
    },
    /// A requested page size is outside the supported range.
    #[error("page limit must be between 1 and 100")]
    InvalidPageLimit {
        /// Unsupported requested page size.
        limit: u32,
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
