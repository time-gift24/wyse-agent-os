//! Pure ontology schema use cases.

use std::{collections::HashMap, sync::Arc};

use crate::{
    DraftName, FilesystemDraftStore, GraphProjection, LinkRecord, ObjectId, ObjectRecord,
    OntologyError, OntologyRepository, PublishedRevision, RevisionId, SchemaDocument, SchemaRef,
    TagName, revision_id, validate_object_values,
};

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
        let snapshot = self.repository.schema_validation_snapshot().await?;
        validate_schema_instances(&draft.schema, &snapshot.objects, &snapshot.links)?;

        let revision = PublishedRevision {
            id: revision_id(&draft.schema)?,
            schema: draft.schema,
        };
        self.repository.insert_revision(revision.clone()).await?;
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

    async fn load_revision(&self, id: &RevisionId) -> Result<PublishedRevision, OntologyError> {
        self.repository
            .get_revision(id)
            .await?
            .ok_or_else(|| OntologyError::RevisionMissing { id: id.clone() })
    }
}

fn validate_schema_instances(
    schema: &SchemaDocument,
    objects: &[ObjectRecord],
    links: &[LinkRecord],
) -> Result<(), OntologyError> {
    let mut diagnostics = Vec::new();
    let objects_by_id: HashMap<ObjectId, &ObjectRecord> =
        objects.iter().map(|object| (object.id, object)).collect();

    for object in objects {
        let Some(object_type) = schema
            .object_types
            .iter()
            .find(|object_type| object_type.id == object.object_type_id)
        else {
            diagnostics.push(format!("object {} has an unknown object type", object.id));
            continue;
        };
        if let Err(OntologyError::ValueInvalid {
            diagnostics: errors,
        }) = validate_object_values(&object_type.properties, &object.values)
        {
            diagnostics.extend(
                errors
                    .into_iter()
                    .map(|error| format!("object {}: {error}", object.id)),
            );
        }
    }

    for link in links {
        let Some(link_type) = schema
            .link_types
            .iter()
            .find(|link_type| link_type.id == link.link_type_id)
        else {
            diagnostics.push(format!("link {} has an unknown link type", link.id));
            continue;
        };
        match objects_by_id.get(&link.source_object_id) {
            Some(source) if source.object_type_id == link_type.source_object_type_id => {}
            Some(_) => diagnostics.push(format!("link {} has a source of the wrong type", link.id)),
            None => diagnostics.push(format!("link {} has a missing source object", link.id)),
        }
        match objects_by_id.get(&link.target_object_id) {
            Some(target) if target.object_type_id == link_type.target_object_type_id => {}
            Some(_) => diagnostics.push(format!("link {} has a target of the wrong type", link.id)),
            None => diagnostics.push(format!("link {} has a missing target object", link.id)),
        }
    }

    if diagnostics.is_empty() {
        Ok(())
    } else {
        Err(OntologyError::PublishInvalid { diagnostics })
    }
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
    use serde_json::{Value, json};
    use uuid::Uuid;
    use wyse_filesystem::{
        CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, RecordVersion,
        VersionedEntry, VirtualPath,
    };

    use super::OntologyService;
    use crate::{
        DraftName, FilesystemDraftStore, LinkId, LinkRecord, LinkTypeId, NewLinkRecord,
        NewObjectRecord, ObjectId, ObjectRecord, ObjectType, ObjectTypeId, OntologyError,
        OntologyRepository, Page, PropertyType, PropertyTypeId, PublishedRevision, RevisionId,
        SchemaValidationSnapshot, TagName, ValueType,
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
            if !matches!(
                (cas, entries.contains_key(path)),
                (CasExpectation::Absent, false)
            ) {
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

    struct MemoryRepository {
        objects: Vec<ObjectRecord>,
        revisions: Mutex<BTreeMap<RevisionId, PublishedRevision>>,
        snapshot_reads: AtomicUsize,
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
            _name: &TagName,
            _revision_id: &RevisionId,
        ) -> Result<(), OntologyError> {
            Ok(())
        }

        async fn get_tag(&self, _name: &TagName) -> Result<Option<RevisionId>, OntologyError> {
            Ok(None)
        }

        async fn delete_tag(&self, _name: &TagName) -> Result<(), OntologyError> {
            Ok(())
        }

        async fn schema_validation_snapshot(
            &self,
        ) -> Result<SchemaValidationSnapshot, OntologyError> {
            self.snapshot_reads.fetch_add(1, Ordering::SeqCst);
            Ok(SchemaValidationSnapshot {
                objects: self.objects.clone(),
                links: Vec::new(),
            })
        }

        async fn create_object(
            &self,
            _object: NewObjectRecord,
        ) -> Result<ObjectRecord, OntologyError> {
            Err(OntologyError::ObjectMissing {
                id: ObjectId::new(),
            })
        }

        async fn get_object(&self, _id: ObjectId) -> Result<Option<ObjectRecord>, OntologyError> {
            Ok(None)
        }

        async fn page_objects(
            &self,
            _type_id: ObjectTypeId,
            _after: Option<ObjectId>,
            _limit: u32,
        ) -> Result<Page<ObjectRecord>, OntologyError> {
            Ok(Page {
                items: Vec::new(),
                next_after: None,
            })
        }

        async fn replace_object(
            &self,
            object: ObjectRecord,
        ) -> Result<ObjectRecord, OntologyError> {
            Ok(object)
        }

        async fn delete_object(
            &self,
            _id: ObjectId,
            _version: u64,
            _force: bool,
        ) -> Result<(), OntologyError> {
            Ok(())
        }

        async fn create_link(&self, _link: NewLinkRecord) -> Result<LinkRecord, OntologyError> {
            Err(OntologyError::LinkMissing { id: LinkId::new() })
        }

        async fn get_link(&self, _id: LinkId) -> Result<Option<LinkRecord>, OntologyError> {
            Ok(None)
        }

        async fn page_links(
            &self,
            _after: Option<LinkId>,
            _limit: u32,
        ) -> Result<Page<LinkRecord>, OntologyError> {
            Ok(Page {
                items: Vec::new(),
                next_after: None,
            })
        }

        async fn replace_link(&self, link: LinkRecord) -> Result<LinkRecord, OntologyError> {
            Ok(link)
        }

        async fn delete_link(&self, _id: LinkId, _version: u64) -> Result<(), OntologyError> {
            Ok(())
        }

        async fn links_for_cardinality(
            &self,
            _type_id: LinkTypeId,
            _source: ObjectId,
            _target: ObjectId,
            _excluding: Option<LinkId>,
        ) -> Result<Vec<LinkRecord>, OntologyError> {
            Ok(Vec::new())
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
            objects: vec![ObjectRecord {
                id: ObjectId::from(Uuid::from_u128(3)),
                object_type_id,
                values: value.as_object().cloned().expect("object test value"),
                version: 1,
            }],
            revisions: Mutex::new(BTreeMap::new()),
            snapshot_reads: AtomicUsize::new(0),
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
    async fn publishing_reads_a_single_validation_snapshot() {
        let (service, repository) = service_with_object(json!({"age":42})).await;

        service
            .publish(&DraftName::try_from("main".to_owned()).expect("valid draft name"))
            .await
            .expect("valid draft can be published");

        assert_eq!(repository.snapshot_reads.load(Ordering::SeqCst), 1);
    }
}
