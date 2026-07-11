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
    LinkRecord, NewLinkRecord, NewObjectRecord, ObjectRecord, OntologyRepository, Page,
    PublishedRevision, SchemaValidationSnapshot,
};
pub use schema::{Cardinality, LinkType, ObjectType, PropertyType, SchemaDocument, ValueType};
pub use service::OntologyService;
pub use value::validate_object_values;
