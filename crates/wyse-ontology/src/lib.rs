//! Domain types and validation for the Wyse ontology service.

pub mod draft;
pub mod error;
pub mod graph;
pub mod id;
pub mod repository;
pub mod schema;
pub mod service;
pub mod value;

pub use draft::{Draft, FilesystemDraftStore, canonical_schema_bytes, revision_id};
pub use error::OntologyError;
pub use graph::{GraphEdge, GraphNode, GraphProjection};
pub use id::{
    DraftName, LinkId, LinkTypeId, ObjectId, ObjectTypeId, PropertyTypeId, RevisionId, SchemaRef,
    TagName,
};
pub use repository::{
    LinkCardinalityConstraint, LinkRecord, NewLinkRecord, NewObjectRecord, ObjectRecord,
    OntologyRepository, Page, PublishedRevision, SchemaValidationSnapshot,
    validate_published_revision, validate_schema_instances,
};
pub use schema::{Cardinality, LinkType, ObjectType, PropertyType, SchemaDocument, ValueType};
pub use service::{CreateLink, CreateObject, OntologyService, ReplaceLink, ReplaceObject};
pub use value::validate_object_values;
