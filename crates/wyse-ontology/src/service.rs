//! Pure ontology schema use cases.

use std::sync::Arc;

use serde_json::{Map, Value};

use crate::{
    DraftName, FilesystemDraftStore, GraphProjection, LinkCardinalityConstraint, LinkId,
    LinkRecord, LinkType, LinkTypeId, NewLinkRecord, NewObjectRecord, ObjectId, ObjectRecord,
    ObjectType, ObjectTypeId, OntologyError, OntologyRepository, Page, PropertyType,
    PropertyTypeId, PublishedRevision, RevisionId, SchemaDocument, SchemaRef, TagName, ValueType,
    revision_id, validate_object_values,
};

/// Data required to create an object instance.
#[derive(Debug, Clone, PartialEq)]
pub struct CreateObject {
    /// Schema used to validate the new object.
    pub schema_ref: SchemaRef,
    /// Identity of the object's schema type.
    pub object_type_id: ObjectTypeId,
    /// Complete object values document.
    pub values: Map<String, Value>,
}

/// Data required to replace an object instance's complete values document.
#[derive(Debug, Clone, PartialEq)]
pub struct ReplaceObject {
    /// Schema used to validate the replacement values.
    pub schema_ref: SchemaRef,
    /// Current optimistic-lock version.
    pub version: u64,
    /// Complete replacement values document.
    pub values: Map<String, Value>,
}

/// Data required to create a link instance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateLink {
    /// Schema used to validate the new link.
    pub schema_ref: SchemaRef,
    /// Identity of the link's schema type.
    pub link_type_id: LinkTypeId,
    /// Source object identity.
    pub source_object_id: ObjectId,
    /// Target object identity.
    pub target_object_id: ObjectId,
}

/// Data required to replace a link instance's endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplaceLink {
    /// Schema used to validate the replacement endpoints.
    pub schema_ref: SchemaRef,
    /// Current optimistic-lock version.
    pub version: u64,
    /// Complete replacement source endpoint.
    pub source_object_id: ObjectId,
    /// Complete replacement target endpoint.
    pub target_object_id: ObjectId,
}

/// Coordinates schema drafts, immutable revisions, tags, and shared-instance validation.
#[derive(Clone)]
pub struct OntologyService {
    drafts: FilesystemDraftStore,
    repository: Arc<dyn OntologyRepository>,
}

impl OntologyService {
    /// Creates a service over a draft store and a repository implementation.
    #[must_use]
    pub fn new(drafts: FilesystemDraftStore, repository: Arc<dyn OntologyRepository>) -> Self {
        Self { drafts, repository }
    }

    /// Creates an editable schema draft.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the name or schema is invalid, a draft
    /// already exists, or the backing filesystem cannot persist the draft.
    pub async fn create_draft(
        &self,
        name: DraftName,
        schema: SchemaDocument,
    ) -> Result<crate::Draft, OntologyError> {
        self.drafts.create(name, schema).await
    }

    /// Lists every editable schema draft.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the backing filesystem cannot list or
    /// decode the drafts.
    pub async fn list_drafts(&self) -> Result<Vec<crate::Draft>, OntologyError> {
        self.drafts.list().await
    }

    /// Loads one editable schema draft.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::DraftMissing`] when the draft does not exist.
    pub async fn get_draft(&self, name: &DraftName) -> Result<crate::Draft, OntologyError> {
        self.drafts.load(name).await
    }

    /// Deletes a draft if its current digest matches `expected_digest`.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::DraftConflict`] when the draft digest is stale.
    pub async fn delete_draft(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
    ) -> Result<(), OntologyError> {
        self.drafts.delete(name, expected_digest).await
    }

    /// Validates one draft's static schema.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::SchemaInvalid`] when the stored schema is invalid.
    pub async fn validate_draft(&self, name: &DraftName) -> Result<crate::Draft, OntologyError> {
        let draft = self.drafts.load(name).await?;
        draft.schema.validate()?;
        Ok(draft)
    }

    /// Adds an object type to a draft and creates its immutable identity.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the draft is stale or the resulting schema is invalid.
    pub async fn add_object_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        type_name: String,
        description: String,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            schema.object_types.push(ObjectType {
                id: ObjectTypeId::new(),
                name: type_name,
                description,
                properties: Vec::new(),
            });
            Ok(())
        })
        .await
    }

    /// Replaces an object type name and description without changing its identity.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::ObjectTypeMissing`] when the type is absent.
    pub async fn replace_object_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        object_type_id: ObjectTypeId,
        type_name: String,
        description: String,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let object_type = schema
                .object_types
                .iter_mut()
                .find(|object_type| object_type.id == object_type_id)
                .ok_or(OntologyError::ObjectTypeMissing { id: object_type_id })?;
            object_type.name = type_name;
            object_type.description = description;
            Ok(())
        })
        .await
    }

    /// Removes an object type from a draft.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::ObjectTypeMissing`] when the type is absent.
    pub async fn delete_object_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        object_type_id: ObjectTypeId,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let before = schema.object_types.len();
            schema
                .object_types
                .retain(|object_type| object_type.id != object_type_id);
            if schema.object_types.len() == before {
                return Err(OntologyError::ObjectTypeMissing { id: object_type_id });
            }
            Ok(())
        })
        .await
    }

    /// Adds a property type to an object type and creates its immutable identity.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::ObjectTypeMissing`] when the parent type is absent.
    #[allow(
        clippy::too_many_arguments,
        reason = "the schema fields mirror one compact REST DTO without an extra wrapper type"
    )]
    pub async fn add_property_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        object_type_id: ObjectTypeId,
        property_name: String,
        description: String,
        value_type: ValueType,
        required: bool,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let object_type = schema
                .object_types
                .iter_mut()
                .find(|object_type| object_type.id == object_type_id)
                .ok_or(OntologyError::ObjectTypeMissing { id: object_type_id })?;
            object_type.properties.push(PropertyType {
                id: PropertyTypeId::new(),
                name: property_name,
                description,
                value_type,
                required,
            });
            Ok(())
        })
        .await
    }

    /// Replaces one property definition without changing its identity.
    ///
    /// # Errors
    ///
    /// Returns a missing type error when the parent or property is absent.
    #[allow(
        clippy::too_many_arguments,
        reason = "the schema fields mirror one compact REST DTO without an extra wrapper type"
    )]
    pub async fn replace_property_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        object_type_id: ObjectTypeId,
        property_type_id: PropertyTypeId,
        property_name: String,
        description: String,
        value_type: ValueType,
        required: bool,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let object_type = schema
                .object_types
                .iter_mut()
                .find(|object_type| object_type.id == object_type_id)
                .ok_or(OntologyError::ObjectTypeMissing { id: object_type_id })?;
            let property = object_type
                .properties
                .iter_mut()
                .find(|property| property.id == property_type_id)
                .ok_or(OntologyError::PropertyTypeMissing {
                    object_type_id,
                    id: property_type_id,
                })?;
            property.name = property_name;
            property.description = description;
            property.value_type = value_type;
            property.required = required;
            Ok(())
        })
        .await
    }

    /// Removes a property type from an object type.
    ///
    /// # Errors
    ///
    /// Returns a missing type error when the parent or property is absent.
    pub async fn delete_property_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        object_type_id: ObjectTypeId,
        property_type_id: PropertyTypeId,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let object_type = schema
                .object_types
                .iter_mut()
                .find(|object_type| object_type.id == object_type_id)
                .ok_or(OntologyError::ObjectTypeMissing { id: object_type_id })?;
            let before = object_type.properties.len();
            object_type
                .properties
                .retain(|property| property.id != property_type_id);
            if object_type.properties.len() == before {
                return Err(OntologyError::PropertyTypeMissing {
                    object_type_id,
                    id: property_type_id,
                });
            }
            Ok(())
        })
        .await
    }

    /// Adds a link type to a draft and creates its immutable identity.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the resulting schema is invalid.
    #[allow(
        clippy::too_many_arguments,
        reason = "the schema fields mirror one compact REST DTO without an extra wrapper type"
    )]
    pub async fn add_link_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        link_name: String,
        description: String,
        source_object_type_id: ObjectTypeId,
        target_object_type_id: ObjectTypeId,
        cardinality: crate::Cardinality,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            schema.link_types.push(LinkType {
                id: LinkTypeId::new(),
                name: link_name,
                description,
                source_object_type_id,
                target_object_type_id,
                cardinality,
            });
            Ok(())
        })
        .await
    }

    /// Replaces a link type definition without changing its identity.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::LinkTypeMissing`] when the type is absent.
    #[allow(clippy::too_many_arguments)]
    pub async fn replace_link_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        link_type_id: LinkTypeId,
        link_name: String,
        description: String,
        source_object_type_id: ObjectTypeId,
        target_object_type_id: ObjectTypeId,
        cardinality: crate::Cardinality,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let link_type = schema
                .link_types
                .iter_mut()
                .find(|link_type| link_type.id == link_type_id)
                .ok_or(OntologyError::LinkTypeMissing { id: link_type_id })?;
            link_type.name = link_name;
            link_type.description = description;
            link_type.source_object_type_id = source_object_type_id;
            link_type.target_object_type_id = target_object_type_id;
            link_type.cardinality = cardinality;
            Ok(())
        })
        .await
    }

    /// Removes a link type from a draft.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::LinkTypeMissing`] when the type is absent.
    pub async fn delete_link_type(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        link_type_id: LinkTypeId,
    ) -> Result<crate::Draft, OntologyError> {
        self.update_draft(name, expected_digest, move |schema| {
            let before = schema.link_types.len();
            schema
                .link_types
                .retain(|link_type| link_type.id != link_type_id);
            if schema.link_types.len() == before {
                return Err(OntologyError::LinkTypeMissing { id: link_type_id });
            }
            Ok(())
        })
        .await
    }

    /// Resolves a selected draft, revision, or tag to its schema document.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the selected resource does not exist or
    /// its backing store cannot be read.
    pub async fn resolve_schema(
        &self,
        schema_ref: &SchemaRef,
    ) -> Result<SchemaDocument, OntologyError> {
        match schema_ref {
            SchemaRef::Draft(name) => Ok(self.drafts.load(name).await?.schema),
            SchemaRef::Revision(id) => Ok(self.load_revision(id).await?.schema),
            SchemaRef::Tag(name) => {
                let revision_id = self
                    .repository
                    .get_tag(name)
                    .await?
                    .ok_or_else(|| OntologyError::TagMissing { name: name.clone() })?;
                Ok(self.load_revision(&revision_id).await?.schema)
            }
        }
    }

    /// Publishes a draft after validating it against every shared instance.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::PublishInvalid`] when shared instances do not
    /// satisfy the draft, or another ontology error for invalid schemas and
    /// unavailable storage.
    pub async fn publish(&self, name: &DraftName) -> Result<PublishedRevision, OntologyError> {
        let draft = self.drafts.load(name).await?;
        draft.schema.validate()?;
        let revision = PublishedRevision {
            id: revision_id(&draft.schema)?,
            schema: draft.schema,
        };
        self.repository.publish_revision(revision.clone()).await?;
        Ok(revision)
    }

    /// Lists immutable published revisions.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the repository cannot be read.
    pub async fn list_revisions(&self) -> Result<Vec<PublishedRevision>, OntologyError> {
        self.repository.list_revisions().await
    }

    /// Loads one immutable published revision.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::RevisionMissing`] when the revision does not exist.
    pub async fn get_revision(&self, id: &RevisionId) -> Result<PublishedRevision, OntologyError> {
        self.load_revision(id).await
    }

    /// Moves a tag to an existing published revision.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::RevisionMissing`] when the revision does not
    /// exist or an ontology error when storage is unavailable.
    pub async fn put_tag(
        &self,
        name: &TagName,
        revision_id: &RevisionId,
    ) -> Result<(), OntologyError> {
        self.load_revision(revision_id).await?;
        self.repository.put_tag(name, revision_id).await
    }

    /// Resolves a tag to its target revision.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::TagMissing`] when the tag does not exist.
    pub async fn get_tag(&self, name: &TagName) -> Result<RevisionId, OntologyError> {
        self.repository
            .get_tag(name)
            .await?
            .ok_or_else(|| OntologyError::TagMissing { name: name.clone() })
    }

    /// Deletes a non-reserved schema tag.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::ReservedTag`] for `online`,
    /// [`OntologyError::TagMissing`] when the tag does not exist, or a storage
    /// error otherwise.
    pub async fn delete_tag(&self, name: &TagName) -> Result<(), OntologyError> {
        if name == &TagName::online() {
            return Err(OntologyError::ReservedTag);
        }
        if self.repository.get_tag(name).await?.is_none() {
            return Err(OntologyError::TagMissing { name: name.clone() });
        }
        self.repository.delete_tag(name).await
    }

    /// Builds a display-neutral type graph for the selected schema.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the selected schema cannot be resolved.
    pub async fn graph(&self, schema_ref: SchemaRef) -> Result<GraphProjection, OntologyError> {
        let schema = self.resolve_schema(&schema_ref).await?;
        Ok(GraphProjection::from_schema(schema_ref, &schema))
    }

    /// Creates an object after validating it against the requested and online schemas.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when either schema rejects the object or persistence fails.
    pub async fn create_object(
        &self,
        request: CreateObject,
    ) -> Result<ObjectRecord, OntologyError> {
        let candidate = ObjectRecord {
            id: ObjectId::new(),
            object_type_id: request.object_type_id,
            values: request.values,
            version: 1,
        };
        self.validate_object_write(&request.schema_ref, &candidate)
            .await?;
        self.repository
            .create_object(NewObjectRecord {
                id: candidate.id,
                object_type_id: candidate.object_type_id,
                values: candidate.values,
            })
            .await
    }

    /// Gets an object after confirming its type exists in the selected schema.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the schema or object cannot be found.
    pub async fn get_object(
        &self,
        schema_ref: SchemaRef,
        id: ObjectId,
    ) -> Result<ObjectRecord, OntologyError> {
        let schema = self.resolve_schema(&schema_ref).await?;
        let object = self
            .repository
            .get_object(id)
            .await?
            .ok_or(OntologyError::ObjectMissing { id })?;
        object_type_in_schema(&schema, object.object_type_id)?;
        Ok(object)
    }

    /// Returns one cursor page of objects for a selected schema type.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the page request or selected schema is invalid,
    /// or persistence fails.
    pub async fn page_objects(
        &self,
        schema_ref: SchemaRef,
        object_type_id: ObjectTypeId,
        after: Option<ObjectId>,
        limit: u32,
    ) -> Result<Page<ObjectRecord>, OntologyError> {
        validate_page_limit(limit)?;
        let schema = self.resolve_schema(&schema_ref).await?;
        object_type_in_schema(&schema, object_type_id)?;
        self.repository
            .page_objects(object_type_id, after, limit)
            .await
    }

    /// Replaces an object's complete values document.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the object version is stale, either schema rejects
    /// the replacement values, or persistence fails.
    pub async fn replace_object(
        &self,
        id: ObjectId,
        request: ReplaceObject,
    ) -> Result<ObjectRecord, OntologyError> {
        let current = self
            .repository
            .get_object(id)
            .await?
            .ok_or(OntologyError::ObjectMissing { id })?;
        if current.version != request.version {
            return Err(OntologyError::ObjectVersionConflict { id });
        }
        let candidate = ObjectRecord {
            values: request.values,
            ..current
        };
        self.validate_object_write(&request.schema_ref, &candidate)
            .await?;
        self.repository.replace_object(candidate).await
    }

    /// Deletes an object, optionally deleting all incident links atomically.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when either schema is unavailable, the version is stale,
    /// the object is still referenced without `force`, or persistence fails.
    pub async fn delete_object(
        &self,
        schema_ref: SchemaRef,
        id: ObjectId,
        version: u64,
        force: bool,
    ) -> Result<(), OntologyError> {
        self.resolve_schema(&schema_ref).await?;
        self.resolve_schema(&SchemaRef::Tag(TagName::online()))
            .await?;
        self.repository.delete_object(id, version, force).await
    }

    /// Creates a link after validating endpoints and cardinality in both schemas.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when endpoints or cardinality are invalid, or persistence fails.
    pub async fn create_link(&self, request: CreateLink) -> Result<LinkRecord, OntologyError> {
        let candidate = LinkRecord {
            id: LinkId::new(),
            link_type_id: request.link_type_id,
            source_object_id: request.source_object_id,
            target_object_id: request.target_object_id,
            version: 1,
        };
        let constraints = self
            .validate_link_write(&request.schema_ref, &candidate)
            .await?;
        self.repository
            .create_link_with_cardinality(
                NewLinkRecord {
                    id: candidate.id,
                    link_type_id: candidate.link_type_id,
                    source_object_id: candidate.source_object_id,
                    target_object_id: candidate.target_object_id,
                },
                &constraints,
            )
            .await
    }

    /// Gets a link after confirming its type exists in the selected schema.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the schema or link cannot be found.
    pub async fn get_link(
        &self,
        schema_ref: SchemaRef,
        id: LinkId,
    ) -> Result<LinkRecord, OntologyError> {
        let schema = self.resolve_schema(&schema_ref).await?;
        let link = self
            .repository
            .get_link(id)
            .await?
            .ok_or(OntologyError::LinkMissing { id })?;
        link_type_in_schema(&schema, link.link_type_id)?;
        Ok(link)
    }

    /// Returns one cursor page of links for a selected schema.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the page request or selected schema is invalid,
    /// or persistence fails.
    pub async fn page_links(
        &self,
        schema_ref: SchemaRef,
        after: Option<LinkId>,
        limit: u32,
    ) -> Result<Page<LinkRecord>, OntologyError> {
        validate_page_limit(limit)?;
        self.resolve_schema(&schema_ref).await?;
        self.repository.page_links(after, limit).await
    }

    /// Replaces a link's endpoints while preserving its link type.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the link version is stale, endpoints or cardinality
    /// are invalid, or persistence fails.
    pub async fn replace_link(
        &self,
        id: LinkId,
        request: ReplaceLink,
    ) -> Result<LinkRecord, OntologyError> {
        let current = self
            .repository
            .get_link(id)
            .await?
            .ok_or(OntologyError::LinkMissing { id })?;
        if current.version != request.version {
            return Err(OntologyError::LinkVersionConflict { id });
        }
        let candidate = LinkRecord {
            source_object_id: request.source_object_id,
            target_object_id: request.target_object_id,
            ..current
        };
        let constraints = self
            .validate_link_write(&request.schema_ref, &candidate)
            .await?;
        self.repository
            .replace_link_with_cardinality(candidate, &constraints)
            .await
    }

    /// Deletes a link after resolving the requested and online schemas.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when either schema is unavailable, the version is stale,
    /// or persistence fails.
    pub async fn delete_link(
        &self,
        schema_ref: SchemaRef,
        id: LinkId,
        version: u64,
    ) -> Result<(), OntologyError> {
        self.resolve_schema(&schema_ref).await?;
        self.resolve_schema(&SchemaRef::Tag(TagName::online()))
            .await?;
        self.repository.delete_link(id, version).await
    }

    async fn validate_object_write(
        &self,
        schema_ref: &SchemaRef,
        candidate: &ObjectRecord,
    ) -> Result<(), OntologyError> {
        let requested = self.resolve_schema(schema_ref).await?;
        let online = self
            .resolve_schema(&SchemaRef::Tag(TagName::online()))
            .await?;
        validate_object_in_schema(&requested, candidate)?;
        validate_object_in_schema(&online, candidate)
    }

    async fn validate_link_write(
        &self,
        schema_ref: &SchemaRef,
        candidate: &LinkRecord,
    ) -> Result<Vec<LinkCardinalityConstraint>, OntologyError> {
        let requested = self.resolve_schema(schema_ref).await?;
        let online = self
            .resolve_schema(&SchemaRef::Tag(TagName::online()))
            .await?;
        let source = self
            .repository
            .get_object(candidate.source_object_id)
            .await?
            .ok_or(OntologyError::ObjectMissing {
                id: candidate.source_object_id,
            })?;
        let target = self
            .repository
            .get_object(candidate.target_object_id)
            .await?
            .ok_or(OntologyError::ObjectMissing {
                id: candidate.target_object_id,
            })?;
        Ok(vec![
            validate_link_in_schema(&requested, candidate, &source, &target)?,
            validate_link_in_schema(&online, candidate, &source, &target)?,
        ])
    }

    async fn load_revision(&self, id: &RevisionId) -> Result<PublishedRevision, OntologyError> {
        self.repository
            .get_revision(id)
            .await?
            .ok_or_else(|| OntologyError::RevisionMissing { id: id.clone() })
    }

    async fn update_draft<F>(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        update: F,
    ) -> Result<crate::Draft, OntologyError>
    where
        F: FnOnce(&mut SchemaDocument) -> Result<(), OntologyError>,
    {
        let mut draft = self.drafts.load(name).await?;
        update(&mut draft.schema)?;
        draft.schema.validate()?;
        self.drafts
            .replace(name, expected_digest, draft.schema)
            .await
    }
}

fn validate_page_limit(limit: u32) -> Result<(), OntologyError> {
    if (1..=100).contains(&limit) {
        Ok(())
    } else {
        Err(OntologyError::InvalidPageLimit { limit })
    }
}

fn object_type_in_schema(
    schema: &SchemaDocument,
    object_type_id: ObjectTypeId,
) -> Result<&ObjectType, OntologyError> {
    schema
        .object_types
        .iter()
        .find(|object_type| object_type.id == object_type_id)
        .ok_or(OntologyError::ObjectTypeMissing { id: object_type_id })
}

fn link_type_in_schema(
    schema: &SchemaDocument,
    link_type_id: LinkTypeId,
) -> Result<&LinkType, OntologyError> {
    schema
        .link_types
        .iter()
        .find(|link_type| link_type.id == link_type_id)
        .ok_or(OntologyError::LinkTypeMissing { id: link_type_id })
}

fn validate_object_in_schema(
    schema: &SchemaDocument,
    candidate: &ObjectRecord,
) -> Result<(), OntologyError> {
    let object_type = object_type_in_schema(schema, candidate.object_type_id)?;
    validate_object_values(&object_type.properties, &candidate.values)
}

fn validate_link_in_schema(
    schema: &SchemaDocument,
    candidate: &LinkRecord,
    source: &ObjectRecord,
    target: &ObjectRecord,
) -> Result<LinkCardinalityConstraint, OntologyError> {
    let link_type = link_type_in_schema(schema, candidate.link_type_id)?;
    let mut diagnostics = Vec::new();
    if source.object_type_id != link_type.source_object_type_id {
        diagnostics.push("source object has the wrong type".to_owned());
    }
    if target.object_type_id != link_type.target_object_type_id {
        diagnostics.push("target object has the wrong type".to_owned());
    }
    if !diagnostics.is_empty() {
        return Err(OntologyError::LinkEndpointInvalid { diagnostics });
    }

    Ok(LinkCardinalityConstraint {
        cardinality: link_type.cardinality,
    })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use async_trait::async_trait;
    use serde_json::{Map, Value, json};
    use uuid::Uuid;
    use wyse_filesystem::{
        CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, RecordVersion,
        VersionedEntry, VirtualPath,
    };

    use super::{CreateLink, CreateObject, OntologyService, ReplaceLink, ReplaceObject};
    use crate::{
        Cardinality, DraftName, FilesystemDraftStore, LinkCardinalityConstraint, LinkId,
        LinkRecord, LinkType, LinkTypeId, NewLinkRecord, NewObjectRecord, ObjectId, ObjectRecord,
        ObjectType, ObjectTypeId, OntologyError, OntologyRepository, Page, PropertyType,
        PropertyTypeId, PublishedRevision, RevisionId, SchemaDocument, SchemaValidationSnapshot,
        TagName, ValueType, revision_id,
    };

    #[derive(Default)]
    struct MemoryFilesystem {
        entries: Mutex<BTreeMap<VirtualPath, VersionedEntry>>,
    }

    #[async_trait]
    impl Filesystem for MemoryFilesystem {
        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            Ok(self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned")
                .get(path)
                .cloned())
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            let mut entries = self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned");
            let matches = match cas {
                CasExpectation::Absent => !entries.contains_key(path),
                CasExpectation::Version(expected) => entries
                    .get(path)
                    .is_some_and(|current| current.version == expected),
                CasExpectation::Any => true,
            };
            if !matches {
                return Err(FilesystemError::VersionMismatch { path: path.clone() });
            }
            let version = RecordVersion::from_backend(1);
            entries.insert(path.clone(), VersionedEntry { entry, version });
            Ok(version)
        }

        async fn read_file(&self, _path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }

        async fn write_file(
            &self,
            _path: &VirtualPath,
            _contents: Vec<u8>,
        ) -> Result<(), FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }

        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }

        async fn metadata(&self, _path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }

        async fn create_dir(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }

        async fn remove_file(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }

        async fn remove_dir(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            Err(FilesystemError::UnsupportedCas)
        }
    }

    #[derive(Default)]
    struct MemoryInstances {
        objects: BTreeMap<ObjectId, ObjectRecord>,
        links: BTreeMap<LinkId, LinkRecord>,
    }

    struct MemoryRepository {
        write_gate: Mutex<()>,
        instances: Mutex<MemoryInstances>,
        revisions: Mutex<BTreeMap<RevisionId, PublishedRevision>>,
        tags: Mutex<BTreeMap<TagName, RevisionId>>,
        snapshot_reads: AtomicUsize,
        publish_writes: AtomicUsize,
    }

    fn ensure_cardinality<'a>(
        links: impl Iterator<Item = &'a LinkRecord>,
        candidate: &LinkRecord,
        constraints: &[LinkCardinalityConstraint],
    ) -> Result<(), OntologyError> {
        let links = links.collect::<Vec<_>>();
        let source_count = links
            .iter()
            .filter(|link| link.source_object_id == candidate.source_object_id)
            .count();
        let target_count = links
            .iter()
            .filter(|link| link.target_object_id == candidate.target_object_id)
            .count();
        if constraints
            .iter()
            .all(|constraint| match constraint.cardinality {
                Cardinality::OneToOne => source_count == 0 && target_count == 0,
                Cardinality::OneToMany => target_count == 0,
                Cardinality::ManyToOne => source_count == 0,
                Cardinality::ManyToMany => true,
            })
        {
            Ok(())
        } else {
            Err(OntologyError::CardinalityConflict {
                link_type_id: candidate.link_type_id,
            })
        }
    }

    #[async_trait]
    impl OntologyRepository for MemoryRepository {
        async fn insert_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError> {
            self.revisions
                .lock()
                .expect("memory repository mutex is not poisoned")
                .insert(revision.id.clone(), revision);
            Ok(())
        }

        async fn publish_revision(&self, revision: PublishedRevision) -> Result<(), OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            crate::validate_published_revision(&revision)?;
            let instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            crate::validate_schema_instances(
                &revision.schema,
                &instances.objects.values().cloned().collect::<Vec<_>>(),
                &instances.links.values().cloned().collect::<Vec<_>>(),
            )?;
            drop(instances);
            self.revisions
                .lock()
                .expect("memory repository mutex is not poisoned")
                .insert(revision.id.clone(), revision);
            self.publish_writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn get_revision(
            &self,
            id: &RevisionId,
        ) -> Result<Option<PublishedRevision>, OntologyError> {
            Ok(self
                .revisions
                .lock()
                .expect("memory repository mutex is not poisoned")
                .get(id)
                .cloned())
        }

        async fn list_revisions(&self) -> Result<Vec<PublishedRevision>, OntologyError> {
            Ok(self
                .revisions
                .lock()
                .expect("memory repository mutex is not poisoned")
                .values()
                .cloned()
                .collect())
        }

        async fn put_tag(
            &self,
            name: &TagName,
            revision_id: &RevisionId,
        ) -> Result<(), OntologyError> {
            self.tags
                .lock()
                .expect("memory repository mutex is not poisoned")
                .insert(name.clone(), revision_id.clone());
            Ok(())
        }

        async fn get_tag(&self, name: &TagName) -> Result<Option<RevisionId>, OntologyError> {
            Ok(self
                .tags
                .lock()
                .expect("memory repository mutex is not poisoned")
                .get(name)
                .cloned())
        }

        async fn delete_tag(&self, name: &TagName) -> Result<(), OntologyError> {
            self.tags
                .lock()
                .expect("memory repository mutex is not poisoned")
                .remove(name);
            Ok(())
        }

        async fn schema_validation_snapshot(
            &self,
        ) -> Result<SchemaValidationSnapshot, OntologyError> {
            self.snapshot_reads.fetch_add(1, Ordering::SeqCst);
            let instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            Ok(SchemaValidationSnapshot {
                objects: instances.objects.values().cloned().collect(),
                links: instances.links.values().cloned().collect(),
            })
        }

        async fn create_object(
            &self,
            object: NewObjectRecord,
        ) -> Result<ObjectRecord, OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            let record = ObjectRecord {
                id: object.id,
                object_type_id: object.object_type_id,
                values: object.values,
                version: 1,
            };
            self.instances
                .lock()
                .expect("memory repository mutex is not poisoned")
                .objects
                .insert(record.id, record.clone());
            Ok(record)
        }

        async fn get_object(&self, id: ObjectId) -> Result<Option<ObjectRecord>, OntologyError> {
            Ok(self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned")
                .objects
                .get(&id)
                .cloned())
        }

        async fn page_objects(
            &self,
            type_id: ObjectTypeId,
            after: Option<ObjectId>,
            limit: u32,
        ) -> Result<Page<ObjectRecord>, OntologyError> {
            let mut items = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned")
                .objects
                .values()
                .filter(|object| object.object_type_id == type_id)
                .filter(|object| after.is_none_or(|after| object.id > after))
                .cloned()
                .collect::<Vec<_>>();
            let has_next = items.len() > limit as usize;
            items.truncate(limit as usize);
            let next_after = has_next.then(|| {
                items
                    .last()
                    .expect("non-empty page has a cursor")
                    .id
                    .as_uuid()
            });
            Ok(Page { items, next_after })
        }

        async fn replace_object(
            &self,
            object: ObjectRecord,
        ) -> Result<ObjectRecord, OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            let mut instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            let Some(current) = instances.objects.get(&object.id) else {
                return Err(OntologyError::ObjectMissing { id: object.id });
            };
            if current.version != object.version {
                return Err(OntologyError::ObjectVersionConflict { id: object.id });
            }
            let updated = ObjectRecord {
                version: object.version + 1,
                ..object
            };
            instances.objects.insert(updated.id, updated.clone());
            Ok(updated)
        }

        async fn delete_object(
            &self,
            id: ObjectId,
            version: u64,
            force: bool,
        ) -> Result<(), OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            let mut instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            let Some(current) = instances.objects.get(&id) else {
                return Err(OntologyError::ObjectMissing { id });
            };
            if current.version != version {
                return Err(OntologyError::ObjectVersionConflict { id });
            }
            let referenced = instances
                .links
                .values()
                .any(|link| link.source_object_id == id || link.target_object_id == id);
            if referenced && !force {
                return Err(OntologyError::ObjectReferenced { id });
            }
            if force {
                instances
                    .links
                    .retain(|_, link| link.source_object_id != id && link.target_object_id != id);
            }
            instances.objects.remove(&id);
            Ok(())
        }

        async fn create_link_with_cardinality(
            &self,
            link: NewLinkRecord,
            constraints: &[LinkCardinalityConstraint],
        ) -> Result<LinkRecord, OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            let record = LinkRecord {
                id: link.id,
                link_type_id: link.link_type_id,
                source_object_id: link.source_object_id,
                target_object_id: link.target_object_id,
                version: 1,
            };
            let mut instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            ensure_cardinality(
                instances
                    .links
                    .values()
                    .filter(|existing| existing.link_type_id == record.link_type_id),
                &record,
                constraints,
            )?;
            instances.links.insert(record.id, record.clone());
            Ok(record)
        }

        async fn get_link(&self, id: LinkId) -> Result<Option<LinkRecord>, OntologyError> {
            Ok(self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned")
                .links
                .get(&id)
                .cloned())
        }

        async fn page_links(
            &self,
            after: Option<LinkId>,
            limit: u32,
        ) -> Result<Page<LinkRecord>, OntologyError> {
            let mut items = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned")
                .links
                .values()
                .filter(|link| after.is_none_or(|after| link.id > after))
                .cloned()
                .collect::<Vec<_>>();
            let has_next = items.len() > limit as usize;
            items.truncate(limit as usize);
            let next_after = has_next.then(|| {
                items
                    .last()
                    .expect("non-empty page has a cursor")
                    .id
                    .as_uuid()
            });
            Ok(Page { items, next_after })
        }

        async fn replace_link_with_cardinality(
            &self,
            link: LinkRecord,
            constraints: &[LinkCardinalityConstraint],
        ) -> Result<LinkRecord, OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            let mut instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            let Some(current) = instances.links.get(&link.id) else {
                return Err(OntologyError::LinkMissing { id: link.id });
            };
            if current.version != link.version {
                return Err(OntologyError::LinkVersionConflict { id: link.id });
            }
            ensure_cardinality(
                instances
                    .links
                    .values()
                    .filter(|existing| existing.link_type_id == link.link_type_id)
                    .filter(|existing| existing.id != link.id),
                &link,
                constraints,
            )?;
            let updated = LinkRecord {
                version: link.version + 1,
                ..link
            };
            instances.links.insert(updated.id, updated.clone());
            Ok(updated)
        }

        async fn delete_link(&self, id: LinkId, version: u64) -> Result<(), OntologyError> {
            let _write_gate = self
                .write_gate
                .lock()
                .expect("memory repository mutex is not poisoned");
            let mut instances = self
                .instances
                .lock()
                .expect("memory repository mutex is not poisoned");
            let Some(current) = instances.links.get(&id) else {
                return Err(OntologyError::LinkMissing { id });
            };
            if current.version != version {
                return Err(OntologyError::LinkVersionConflict { id });
            }
            instances.links.remove(&id);
            Ok(())
        }
    }

    async fn service_with_object(value: Value) -> (OntologyService, Arc<MemoryRepository>) {
        let object_type_id = ObjectTypeId::from(Uuid::from_u128(1));
        let schema = crate::SchemaDocument {
            schema_version: 1,
            object_types: vec![ObjectType {
                id: object_type_id,
                name: "person".to_owned(),
                description: String::new(),
                properties: vec![PropertyType {
                    id: PropertyTypeId::from(Uuid::from_u128(2)),
                    name: "age".to_owned(),
                    description: String::new(),
                    value_type: ValueType::Integer,
                    required: true,
                }],
            }],
            link_types: Vec::new(),
        };
        let filesystem: Arc<dyn Filesystem> = Arc::new(MemoryFilesystem::default());
        let drafts = FilesystemDraftStore::new(filesystem);
        drafts
            .create(
                DraftName::try_from("main".to_owned()).expect("valid draft name"),
                schema,
            )
            .await
            .expect("draft can be created");
        let repository = MemoryRepository {
            write_gate: Mutex::new(()),
            instances: Mutex::new(MemoryInstances {
                objects: BTreeMap::from([(
                    ObjectId::from(Uuid::from_u128(3)),
                    ObjectRecord {
                        id: ObjectId::from(Uuid::from_u128(3)),
                        object_type_id,
                        values: value.as_object().cloned().expect("object test value"),
                        version: 1,
                    },
                )]),
                links: BTreeMap::new(),
            }),
            revisions: Mutex::new(BTreeMap::new()),
            tags: Mutex::new(BTreeMap::new()),
            snapshot_reads: AtomicUsize::new(0),
            publish_writes: AtomicUsize::new(0),
        };

        let repository = Arc::new(repository);
        (OntologyService::new(drafts, repository.clone()), repository)
    }

    #[tokio::test]
    async fn publishing_rejects_existing_values_invalid_for_the_draft() {
        let (service, _) = service_with_object(json!({"age":"old"})).await;

        assert!(matches!(
            service
                .publish(&DraftName::try_from("main".to_owned()).expect("valid draft name"))
                .await,
            Err(OntologyError::PublishInvalid { .. })
        ));
    }

    #[tokio::test]
    async fn publishing_uses_the_repository_atomic_publish_operation() {
        let (service, repository) = service_with_object(json!({"age":42})).await;

        service
            .publish(&DraftName::try_from("main".to_owned()).expect("valid draft name"))
            .await
            .expect("valid draft can be published");

        assert_eq!(repository.snapshot_reads.load(Ordering::SeqCst), 0);
        assert_eq!(repository.publish_writes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn draft_write_cannot_break_the_online_schema() {
        let service = service_with_online_integer_age_and_draft_string_age().await;
        let request = CreateObject {
            schema_ref: crate::SchemaRef::Draft(
                DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
            ),
            object_type_id: person_type_id(),
            values: Map::from_iter([("age".to_owned(), json!("old"))]),
        };

        assert!(matches!(
            service.create_object(request).await,
            Err(OntologyError::ValueInvalid { .. })
        ));
    }

    #[tokio::test]
    async fn many_to_one_rejects_a_second_link_from_the_same_source() {
        let service = service_with_many_to_one_schema().await;
        let first = CreateLink {
            schema_ref: crate::SchemaRef::Tag(TagName::online()),
            link_type_id: knows_link_type_id(),
            source_object_id: ObjectId::from(Uuid::from_u128(10)),
            target_object_id: ObjectId::from(Uuid::from_u128(11)),
        };
        let second = CreateLink {
            target_object_id: ObjectId::from(Uuid::from_u128(12)),
            ..first.clone()
        };

        service
            .create_link(first)
            .await
            .expect("first link satisfies many-to-one");
        assert!(matches!(
            service.create_link(second).await,
            Err(OntologyError::CardinalityConflict { .. })
        ));
    }

    #[tokio::test]
    async fn force_delete_removes_incident_links_but_regular_delete_rejects_them() {
        let service = service_with_many_to_one_schema().await;
        let object_id = ObjectId::from(Uuid::from_u128(10));
        let link = service
            .create_link(CreateLink {
                schema_ref: crate::SchemaRef::Tag(TagName::online()),
                link_type_id: knows_link_type_id(),
                source_object_id: object_id,
                target_object_id: ObjectId::from(Uuid::from_u128(11)),
            })
            .await
            .expect("link can be created");

        assert!(matches!(
            service
                .delete_object(
                    crate::SchemaRef::Tag(TagName::online()),
                    object_id,
                    1,
                    false,
                )
                .await,
            Err(OntologyError::ObjectReferenced { .. })
        ));

        service
            .delete_object(crate::SchemaRef::Tag(TagName::online()), object_id, 1, true)
            .await
            .expect("force delete removes object and links");
        assert!(matches!(
            service
                .get_link(crate::SchemaRef::Tag(TagName::online()), link.id)
                .await,
            Err(OntologyError::LinkMissing { .. })
        ));
    }

    #[tokio::test]
    async fn object_replacement_replaces_all_values_and_uses_the_current_version() {
        let service = service_with_online_integer_age_and_draft_string_age().await;
        let created = service
            .create_object(CreateObject {
                schema_ref: crate::SchemaRef::Tag(TagName::online()),
                object_type_id: person_type_id(),
                values: Map::from_iter([("age".to_owned(), json!(41))]),
            })
            .await
            .expect("object can be created");

        let replaced = service
            .replace_object(
                created.id,
                ReplaceObject {
                    schema_ref: crate::SchemaRef::Tag(TagName::online()),
                    version: created.version,
                    values: Map::from_iter([("age".to_owned(), json!(42))]),
                },
            )
            .await
            .expect("complete values replacement can be persisted");

        assert_eq!(
            replaced.values,
            Map::from_iter([("age".to_owned(), json!(42))])
        );
        assert_eq!(replaced.version, 2);
    }

    #[tokio::test]
    async fn link_replacement_excludes_its_own_cardinality_slot() {
        let service = service_with_many_to_one_schema().await;
        let link = service
            .create_link(CreateLink {
                schema_ref: crate::SchemaRef::Tag(TagName::online()),
                link_type_id: knows_link_type_id(),
                source_object_id: ObjectId::from(Uuid::from_u128(10)),
                target_object_id: ObjectId::from(Uuid::from_u128(11)),
            })
            .await
            .expect("link can be created");

        let replaced = service
            .replace_link(
                link.id,
                ReplaceLink {
                    schema_ref: crate::SchemaRef::Tag(TagName::online()),
                    version: link.version,
                    source_object_id: link.source_object_id,
                    target_object_id: link.target_object_id,
                },
            )
            .await;

        assert!(replaced.is_ok());
    }

    #[tokio::test]
    async fn cardinality_rules_are_enforced_for_each_variant() {
        let source = ObjectId::from(Uuid::from_u128(10));
        let first_target = ObjectId::from(Uuid::from_u128(11));
        let second_target = ObjectId::from(Uuid::from_u128(12));

        let one_to_one = service_with_cardinality(Cardinality::OneToOne).await;
        create_test_link(&one_to_one, source, first_target)
            .await
            .expect("first one-to-one link can be created");
        assert_cardinality_conflict(create_test_link(&one_to_one, source, second_target).await);
        assert_cardinality_conflict(
            create_test_link(&one_to_one, second_target, first_target).await,
        );

        let one_to_many = service_with_cardinality(Cardinality::OneToMany).await;
        create_test_link(&one_to_many, source, first_target)
            .await
            .expect("first one-to-many link can be created");
        assert_cardinality_conflict(
            create_test_link(&one_to_many, second_target, first_target).await,
        );

        let many_to_one = service_with_cardinality(Cardinality::ManyToOne).await;
        create_test_link(&many_to_one, source, first_target)
            .await
            .expect("first many-to-one link can be created");
        assert_cardinality_conflict(create_test_link(&many_to_one, source, second_target).await);

        let many_to_many = service_with_cardinality(Cardinality::ManyToMany).await;
        create_test_link(&many_to_many, source, first_target)
            .await
            .expect("first many-to-many link can be created");
        create_test_link(&many_to_many, source, second_target)
            .await
            .expect("many-to-many permits another target for the same source");
    }

    #[tokio::test]
    async fn stricter_online_cardinality_is_enforced_when_the_draft_is_permissive() {
        let service = service_with_online_and_draft_cardinality(
            Cardinality::OneToOne,
            Cardinality::ManyToMany,
        )
        .await;
        let source = ObjectId::from(Uuid::from_u128(10));
        create_test_link_with_schema(
            &service,
            crate::SchemaRef::Draft(
                DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
            ),
            source,
            ObjectId::from(Uuid::from_u128(11)),
        )
        .await
        .expect("first link can be created through draft");

        assert_cardinality_conflict(
            create_test_link_with_schema(
                &service,
                crate::SchemaRef::Draft(
                    DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                ),
                source,
                ObjectId::from(Uuid::from_u128(12)),
            )
            .await,
        );
    }

    #[tokio::test]
    async fn online_schema_rejects_endpoints_that_only_the_requested_schema_accepts() {
        let service = service_with_different_link_endpoints().await;

        assert!(matches!(
            service
                .create_link(CreateLink {
                    schema_ref: crate::SchemaRef::Draft(
                        DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                    ),
                    link_type_id: knows_link_type_id(),
                    source_object_id: ObjectId::from(Uuid::from_u128(10)),
                    target_object_id: ObjectId::from(Uuid::from_u128(11)),
                })
                .await,
            Err(OntologyError::LinkEndpointInvalid { .. })
        ));
    }

    #[tokio::test]
    async fn writes_require_the_online_tag() {
        let service = service_without_online_tag().await;

        assert!(matches!(
            service
                .create_object(CreateObject {
                    schema_ref: crate::SchemaRef::Draft(
                        DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                    ),
                    object_type_id: person_type_id(),
                    values: Map::from_iter([("age".to_owned(), json!(1))]),
                })
                .await,
            Err(OntologyError::TagMissing { .. })
        ));
    }

    #[tokio::test]
    async fn pagination_limit_must_be_between_one_and_one_hundred() {
        let service = service_with_many_to_one_schema().await;
        let schema_ref = crate::SchemaRef::Tag(TagName::online());

        assert!(matches!(
            service
                .page_objects(schema_ref.clone(), person_type_id(), None, 0)
                .await,
            Err(OntologyError::InvalidPageLimit { limit: 0 })
        ));
        assert!(matches!(
            service
                .page_objects(schema_ref, person_type_id(), None, 101)
                .await,
            Err(OntologyError::InvalidPageLimit { limit: 101 })
        ));
    }

    #[tokio::test]
    async fn schema_mutation_preserves_type_identity_and_requires_the_current_digest() {
        let (service, _) = service_with_object(json!({ "age": 42 })).await;
        let name = DraftName::try_from("main".to_owned()).expect("valid draft name");
        let draft = service.get_draft(&name).await.expect("draft exists");
        let object_type_id = draft.schema.object_types[0].id;

        let updated = service
            .replace_object_type(
                &name,
                draft.digest.clone(),
                object_type_id,
                "member".to_owned(),
                "renamed".to_owned(),
            )
            .await
            .expect("current digest updates draft");

        assert_eq!(updated.schema.object_types[0].id, object_type_id);
        assert!(matches!(
            service
                .add_object_type(&name, draft.digest, "company".to_owned(), String::new(),)
                .await,
            Err(OntologyError::DraftConflict { .. })
        ));
    }

    async fn create_test_link(
        service: &OntologyService,
        source_object_id: ObjectId,
        target_object_id: ObjectId,
    ) -> Result<LinkRecord, OntologyError> {
        create_test_link_with_schema(
            service,
            crate::SchemaRef::Tag(TagName::online()),
            source_object_id,
            target_object_id,
        )
        .await
    }

    async fn create_test_link_with_schema(
        service: &OntologyService,
        schema_ref: crate::SchemaRef,
        source_object_id: ObjectId,
        target_object_id: ObjectId,
    ) -> Result<LinkRecord, OntologyError> {
        service
            .create_link(CreateLink {
                schema_ref,
                link_type_id: knows_link_type_id(),
                source_object_id,
                target_object_id,
            })
            .await
    }

    fn assert_cardinality_conflict(result: Result<LinkRecord, OntologyError>) {
        assert!(matches!(
            result,
            Err(OntologyError::CardinalityConflict { .. })
        ));
    }

    fn person_type_id() -> ObjectTypeId {
        ObjectTypeId::from(Uuid::from_u128(1))
    }

    fn knows_link_type_id() -> LinkTypeId {
        LinkTypeId::from(Uuid::from_u128(2))
    }

    async fn service_with_online_integer_age_and_draft_string_age() -> OntologyService {
        let online = SchemaDocument {
            schema_version: 1,
            object_types: vec![ObjectType {
                id: person_type_id(),
                name: "person".to_owned(),
                description: String::new(),
                properties: vec![PropertyType {
                    id: PropertyTypeId::from(Uuid::from_u128(3)),
                    name: "age".to_owned(),
                    description: String::new(),
                    value_type: ValueType::Integer,
                    required: true,
                }],
            }],
            link_types: Vec::new(),
        };
        let mut draft = online.clone();
        draft.object_types[0].properties[0].value_type = ValueType::String;
        service_with_online_schema(
            online,
            vec![(
                DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                draft,
            )],
            Vec::new(),
        )
        .await
    }

    async fn service_with_many_to_one_schema() -> OntologyService {
        service_with_cardinality(Cardinality::ManyToOne).await
    }

    async fn service_with_cardinality(cardinality: Cardinality) -> OntologyService {
        let schema = SchemaDocument {
            schema_version: 1,
            object_types: vec![ObjectType {
                id: person_type_id(),
                name: "person".to_owned(),
                description: String::new(),
                properties: Vec::new(),
            }],
            link_types: vec![LinkType::new(
                knows_link_type_id(),
                "knows".to_owned(),
                person_type_id(),
                person_type_id(),
                cardinality,
            )],
        };
        let objects = [10_u128, 11, 12]
            .into_iter()
            .map(|id| ObjectRecord {
                id: ObjectId::from(Uuid::from_u128(id)),
                object_type_id: person_type_id(),
                values: Map::new(),
                version: 1,
            })
            .collect();
        service_with_online_schema(schema, Vec::new(), objects).await
    }

    async fn service_with_online_and_draft_cardinality(
        online_cardinality: Cardinality,
        draft_cardinality: Cardinality,
    ) -> OntologyService {
        let online = schema_with_link_cardinality(online_cardinality);
        let draft = schema_with_link_cardinality(draft_cardinality);
        let objects = test_people();
        service_with_online_schema(
            online,
            vec![(
                DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                draft,
            )],
            objects,
        )
        .await
    }

    async fn service_with_different_link_endpoints() -> OntologyService {
        let person = person_type_id();
        let company = ObjectTypeId::from(Uuid::from_u128(4));
        let online = SchemaDocument {
            schema_version: 1,
            object_types: vec![
                empty_object_type(person, "person"),
                empty_object_type(company, "company"),
            ],
            link_types: vec![LinkType::new(
                knows_link_type_id(),
                "knows".to_owned(),
                person,
                person,
                Cardinality::ManyToMany,
            )],
        };
        let draft = SchemaDocument {
            link_types: vec![LinkType::new(
                knows_link_type_id(),
                "knows".to_owned(),
                person,
                company,
                Cardinality::ManyToMany,
            )],
            ..online.clone()
        };
        service_with_online_schema(
            online,
            vec![(
                DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                draft,
            )],
            vec![
                ObjectRecord {
                    id: ObjectId::from(Uuid::from_u128(10)),
                    object_type_id: person,
                    values: Map::new(),
                    version: 1,
                },
                ObjectRecord {
                    id: ObjectId::from(Uuid::from_u128(11)),
                    object_type_id: company,
                    values: Map::new(),
                    version: 1,
                },
            ],
        )
        .await
    }

    async fn service_without_online_tag() -> OntologyService {
        let schema = SchemaDocument {
            schema_version: 1,
            object_types: vec![ObjectType {
                id: person_type_id(),
                name: "person".to_owned(),
                description: String::new(),
                properties: vec![PropertyType {
                    id: PropertyTypeId::from(Uuid::from_u128(3)),
                    name: "age".to_owned(),
                    description: String::new(),
                    value_type: ValueType::Integer,
                    required: true,
                }],
            }],
            link_types: Vec::new(),
        };
        let filesystem: Arc<dyn Filesystem> = Arc::new(MemoryFilesystem::default());
        let drafts = FilesystemDraftStore::new(filesystem);
        drafts
            .create(
                DraftName::try_from("experiment".to_owned()).expect("valid draft name"),
                schema,
            )
            .await
            .expect("draft can be created");
        OntologyService::new(
            drafts,
            Arc::new(MemoryRepository {
                write_gate: Mutex::new(()),
                instances: Mutex::new(MemoryInstances::default()),
                revisions: Mutex::new(BTreeMap::new()),
                tags: Mutex::new(BTreeMap::new()),
                snapshot_reads: AtomicUsize::new(0),
                publish_writes: AtomicUsize::new(0),
            }),
        )
    }

    fn schema_with_link_cardinality(cardinality: Cardinality) -> SchemaDocument {
        SchemaDocument {
            schema_version: 1,
            object_types: vec![empty_object_type(person_type_id(), "person")],
            link_types: vec![LinkType::new(
                knows_link_type_id(),
                "knows".to_owned(),
                person_type_id(),
                person_type_id(),
                cardinality,
            )],
        }
    }

    fn empty_object_type(id: ObjectTypeId, name: &str) -> ObjectType {
        ObjectType {
            id,
            name: name.to_owned(),
            description: String::new(),
            properties: Vec::new(),
        }
    }

    fn test_people() -> Vec<ObjectRecord> {
        [10_u128, 11, 12]
            .into_iter()
            .map(|id| ObjectRecord {
                id: ObjectId::from(Uuid::from_u128(id)),
                object_type_id: person_type_id(),
                values: Map::new(),
                version: 1,
            })
            .collect()
    }

    async fn service_with_online_schema(
        online_schema: SchemaDocument,
        drafts_to_create: Vec<(DraftName, SchemaDocument)>,
        objects: Vec<ObjectRecord>,
    ) -> OntologyService {
        let filesystem: Arc<dyn Filesystem> = Arc::new(MemoryFilesystem::default());
        let drafts = FilesystemDraftStore::new(filesystem);
        for (name, schema) in drafts_to_create {
            drafts
                .create(name, schema)
                .await
                .expect("draft can be created");
        }
        let online_revision = PublishedRevision {
            id: revision_id(&online_schema).expect("online schema is valid"),
            schema: online_schema,
        };
        let repository = Arc::new(MemoryRepository {
            write_gate: Mutex::new(()),
            instances: Mutex::new(MemoryInstances {
                objects: objects
                    .into_iter()
                    .map(|object| (object.id, object))
                    .collect(),
                links: BTreeMap::new(),
            }),
            revisions: Mutex::new(BTreeMap::from([(
                online_revision.id.clone(),
                online_revision.clone(),
            )])),
            tags: Mutex::new(BTreeMap::from([(TagName::online(), online_revision.id)])),
            snapshot_reads: AtomicUsize::new(0),
            publish_writes: AtomicUsize::new(0),
        });
        OntologyService::new(drafts, repository)
    }
}
