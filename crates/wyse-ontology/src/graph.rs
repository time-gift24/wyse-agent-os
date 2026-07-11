//! Presentation-neutral projection of ontology type definitions.

use crate::{Cardinality, LinkTypeId, ObjectTypeId, SchemaDocument, SchemaRef};

/// A graph projection for an explicitly selected schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphProjection {
    /// Schema selection that produced this projection.
    pub schema_ref: SchemaRef,
    /// Object type nodes.
    pub nodes: Vec<GraphNode>,
    /// Link type edges.
    pub edges: Vec<GraphEdge>,
}

/// One object type node in a schema graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    /// Object type identity.
    pub id: ObjectTypeId,
    /// Display name.
    pub label: String,
    /// Number of defined properties.
    pub property_count: usize,
}

/// One directed link type edge in a schema graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    /// Link type identity.
    pub id: LinkTypeId,
    /// Display name.
    pub label: String,
    /// Source object type identity.
    pub source: ObjectTypeId,
    /// Target object type identity.
    pub target: ObjectTypeId,
    /// Endpoint multiplicity.
    pub cardinality: Cardinality,
}

impl GraphProjection {
    /// Projects a schema's types without layout, style, or instance data.
    #[must_use]
    pub fn from_schema(schema_ref: SchemaRef, schema: &SchemaDocument) -> Self {
        Self {
            schema_ref,
            nodes: schema
                .object_types
                .iter()
                .map(|object_type| GraphNode {
                    id: object_type.id,
                    label: object_type.name.clone(),
                    property_count: object_type.properties.len(),
                })
                .collect(),
            edges: schema
                .link_types
                .iter()
                .map(|link_type| GraphEdge {
                    id: link_type.id,
                    label: link_type.name.clone(),
                    source: link_type.source_object_type_id,
                    target: link_type.target_object_type_id,
                    cardinality: link_type.cardinality,
                })
                .collect(),
        }
    }
}
