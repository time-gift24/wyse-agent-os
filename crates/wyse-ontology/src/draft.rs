//! Filesystem-backed editable ontology schema drafts.

use std::sync::Arc;

use sha2::{Digest, Sha256};
use wyse_filesystem::{CasExpectation, Entry, FileType, Filesystem, FilesystemError, VirtualPath};

use crate::{DraftName, OntologyError, RevisionId, SchemaDocument};

const DRAFT_DIRECTORY: &str = "/ontology/drafts";
const ONTOLOGY_DIRECTORY: &str = "/ontology";

/// An editable schema draft and its current content digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// Logical draft name.
    pub name: DraftName,
    /// Validated schema body.
    pub schema: SchemaDocument,
    /// SHA-256 digest of the canonical schema bytes.
    pub digest: RevisionId,
}

/// Filesystem-backed storage for editable schema drafts.
#[derive(Clone)]
pub struct FilesystemDraftStore {
    filesystem: Arc<dyn Filesystem>,
}

impl FilesystemDraftStore {
    /// Creates a draft store backed by `filesystem`.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self { filesystem }
    }

    /// Creates a draft when no draft with the same name exists.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::DraftConflict`] when the draft already exists,
    /// [`OntologyError::DraftCasUnsupported`] when the backend lacks CAS support,
    /// or another ontology error when validation, encoding, or storage fails.
    pub async fn create(
        &self,
        name: DraftName,
        schema: SchemaDocument,
    ) -> Result<Draft, OntologyError> {
        let bytes = canonical_schema_bytes(&schema)?;
        let digest = digest_bytes(&bytes)?;
        let path = draft_path(&name)?;

        let write = self
            .filesystem
            .put(&path, Entry::new(bytes), CasExpectation::Absent)
            .await;
        if matches!(write, Err(FilesystemError::NotFound { .. })) {
            self.ensure_draft_directory().await?;
            self.filesystem
                .put(
                    &path,
                    Entry::new(canonical_schema_bytes(&schema)?),
                    CasExpectation::Absent,
                )
                .await
                .map_err(|error| map_write_error(name.clone(), error))?;
        } else {
            write.map_err(|error| map_write_error(name.clone(), error))?;
        }

        Ok(Draft {
            name,
            schema,
            digest,
        })
    }

    /// Loads one draft by name.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::DraftMissing`] when the draft does not exist or
    /// another ontology error when decoding, validation, or storage fails.
    pub async fn load(&self, name: &DraftName) -> Result<Draft, OntologyError> {
        let path = draft_path(name)?;
        let record = self
            .filesystem
            .get(&path)
            .await
            .map_err(|error| map_read_error(name.clone(), error))?
            .ok_or_else(|| OntologyError::DraftMissing { name: name.clone() })?;
        let schema = decode_schema(record.entry.contents())?;
        let digest = digest_bytes(record.entry.contents())?;

        Ok(Draft {
            name: name.clone(),
            schema,
            digest,
        })
    }

    /// Lists all schema drafts in logical-name order.
    ///
    /// # Errors
    ///
    /// Returns an ontology error when the draft directory cannot be listed or a
    /// listed draft cannot be loaded.
    pub async fn list(&self) -> Result<Vec<Draft>, OntologyError> {
        let directory = draft_directory()?;
        let entries = match self.filesystem.list_dir(&directory).await {
            Ok(entries) => entries,
            Err(FilesystemError::NotFound { .. }) => Vec::new(),
            Err(error) => return Err(OntologyError::Filesystem(error)),
        };
        let mut drafts = Vec::with_capacity(entries.len());

        for entry in entries {
            if !matches!(entry.file_type, FileType::File) {
                continue;
            }
            let Some(name) = entry.file_name.strip_suffix(".json") else {
                continue;
            };
            let Ok(name) = DraftName::try_from(name.to_owned()) else {
                continue;
            };
            drafts.push(self.load(&name).await?);
        }

        drafts.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(drafts)
    }

    async fn ensure_draft_directory(&self) -> Result<(), OntologyError> {
        for path in [ONTOLOGY_DIRECTORY, DRAFT_DIRECTORY] {
            let path = virtual_path(path.to_owned())?;
            match self.filesystem.create_dir(&path).await {
                Ok(()) | Err(FilesystemError::AlreadyExists { .. }) => {}
                Err(error) => return Err(OntologyError::Filesystem(error)),
            }
        }
        Ok(())
    }

    /// Replaces a draft when `expected_digest` matches its current contents.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::DraftConflict`] when the draft changed since the
    /// caller observed it, [`OntologyError::DraftMissing`] when it no longer
    /// exists, or another ontology error when validation, encoding, or storage fails.
    pub async fn replace(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
        schema: SchemaDocument,
    ) -> Result<Draft, OntologyError> {
        let path = draft_path(name)?;
        let current = self
            .filesystem
            .get(&path)
            .await
            .map_err(|error| map_read_error(name.clone(), error))?
            .ok_or_else(|| OntologyError::DraftMissing { name: name.clone() })?;

        if digest_bytes(current.entry.contents())? != expected_digest {
            return Err(OntologyError::DraftConflict { name: name.clone() });
        }

        let bytes = canonical_schema_bytes(&schema)?;
        let digest = digest_bytes(&bytes)?;
        self.filesystem
            .put(
                &path,
                Entry::new(bytes),
                CasExpectation::Version(current.version),
            )
            .await
            .map_err(|error| map_write_error(name.clone(), error))?;

        Ok(Draft {
            name: name.clone(),
            schema,
            digest,
        })
    }

    /// Deletes a draft when `expected_digest` matches its current contents.
    ///
    /// # Errors
    ///
    /// Returns [`OntologyError::DraftConflict`] when the draft changed since the
    /// caller observed it, [`OntologyError::DraftMissing`] when it no longer
    /// exists, or another ontology error when storage fails.
    pub async fn delete(
        &self,
        name: &DraftName,
        expected_digest: RevisionId,
    ) -> Result<(), OntologyError> {
        let path = draft_path(name)?;
        let current = self
            .filesystem
            .get(&path)
            .await
            .map_err(|error| map_read_error(name.clone(), error))?
            .ok_or_else(|| OntologyError::DraftMissing { name: name.clone() })?;

        if digest_bytes(current.entry.contents())? != expected_digest {
            return Err(OntologyError::DraftConflict { name: name.clone() });
        }

        self.filesystem
            .delete(&path, CasExpectation::Version(current.version))
            .await
            .map_err(|error| map_write_error(name.clone(), error))
    }
}

/// Serializes a validated schema using its declared struct and vector order.
///
/// # Errors
///
/// Returns [`OntologyError::SchemaInvalid`] when the schema violates its
/// invariants or [`OntologyError::EncodeSchema`] when serialization fails.
pub fn canonical_schema_bytes(schema: &SchemaDocument) -> Result<Vec<u8>, OntologyError> {
    schema.validate()?;
    serde_json::to_vec(schema).map_err(OntologyError::EncodeSchema)
}

/// Computes the canonical content-addressed revision identity for a schema.
///
/// # Errors
///
/// Returns the errors from [`canonical_schema_bytes`].
pub fn revision_id(schema: &SchemaDocument) -> Result<RevisionId, OntologyError> {
    digest_bytes(&canonical_schema_bytes(schema)?)
}

fn decode_schema(bytes: &[u8]) -> Result<SchemaDocument, OntologyError> {
    let schema: SchemaDocument =
        serde_json::from_slice(bytes).map_err(OntologyError::DecodeSchema)?;
    schema.validate()?;
    Ok(schema)
}

fn digest_bytes(bytes: &[u8]) -> Result<RevisionId, OntologyError> {
    RevisionId::try_from(format!("{:x}", Sha256::digest(bytes)))
}

fn draft_directory() -> Result<VirtualPath, OntologyError> {
    virtual_path(DRAFT_DIRECTORY.to_owned())
}

fn draft_path(name: &DraftName) -> Result<VirtualPath, OntologyError> {
    virtual_path(format!("{DRAFT_DIRECTORY}/{}.json", name.as_str()))
}

fn virtual_path(path: String) -> Result<VirtualPath, OntologyError> {
    VirtualPath::try_from(path.as_str()).map_err(|source| {
        OntologyError::Filesystem(FilesystemError::InvalidVirtualPath { path, source })
    })
}

fn map_read_error(_name: DraftName, error: FilesystemError) -> OntologyError {
    match error {
        FilesystemError::UnsupportedCas => OntologyError::DraftCasUnsupported,
        error => OntologyError::Filesystem(error),
    }
}

fn map_write_error(name: DraftName, error: FilesystemError) -> OntologyError {
    match error {
        FilesystemError::UnsupportedCas => OntologyError::DraftCasUnsupported,
        FilesystemError::VersionMismatch { .. } => OntologyError::DraftConflict { name },
        error => OntologyError::Filesystem(error),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use uuid::Uuid;
    use wyse_filesystem::{
        CasExpectation, DirEntry, Entry, FileMetadata, FileType, Filesystem, FilesystemError,
        RecordVersion, VersionedEntry, VirtualPath,
    };

    use super::*;
    use crate::{
        Cardinality, LinkType, LinkTypeId, ObjectType, ObjectTypeId, PropertyType, PropertyTypeId,
        ValueType,
    };

    #[derive(Default)]
    struct MemoryCasFilesystem {
        entries: Mutex<BTreeMap<VirtualPath, VersionedEntry>>,
        next_version: Mutex<u64>,
        advance_version_after_next_get: Mutex<bool>,
        unsupported_cas_on_next_put: Mutex<bool>,
    }

    impl MemoryCasFilesystem {
        fn advance_version_after_next_get(&self) {
            *self
                .advance_version_after_next_get
                .lock()
                .expect("memory filesystem script mutex is not poisoned") = true;
        }

        fn reject_next_put_with_unsupported_cas(&self) {
            *self
                .unsupported_cas_on_next_put
                .lock()
                .expect("memory filesystem script mutex is not poisoned") = true;
        }

        fn advance_version(&self, path: &VirtualPath) {
            let mut entries = self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned");
            let mut next_version = self
                .next_version
                .lock()
                .expect("memory filesystem version mutex is not poisoned");
            if let Some(current) = entries.get_mut(path) {
                *next_version += 1;
                current.version = RecordVersion::from_backend(*next_version);
            }
        }
    }

    #[async_trait]
    impl Filesystem for MemoryCasFilesystem {
        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            let entries = self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned");
            let current = entries.get(path).cloned();
            drop(entries);

            if current.is_some()
                && std::mem::take(
                    &mut *self
                        .advance_version_after_next_get
                        .lock()
                        .expect("memory filesystem script mutex is not poisoned"),
                )
            {
                self.advance_version(path);
            }

            Ok(current)
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            if std::mem::take(
                &mut *self
                    .unsupported_cas_on_next_put
                    .lock()
                    .expect("memory filesystem script mutex is not poisoned"),
            ) {
                return Err(FilesystemError::UnsupportedCas);
            }

            let mut entries = self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned");
            let current = entries.get(path);
            let matches = match (cas, current) {
                (CasExpectation::Absent, None) | (CasExpectation::Any, _) => true,
                (CasExpectation::Version(expected), Some(current)) => expected == current.version,
                _ => false,
            };
            if !matches {
                return Err(FilesystemError::VersionMismatch { path: path.clone() });
            }

            let mut next_version = self
                .next_version
                .lock()
                .expect("memory filesystem version mutex is not poisoned");
            *next_version += 1;
            let version = RecordVersion::from_backend(*next_version);
            entries.insert(path.clone(), VersionedEntry { entry, version });
            Ok(version)
        }

        async fn delete(
            &self,
            path: &VirtualPath,
            cas: CasExpectation,
        ) -> Result<(), FilesystemError> {
            let mut entries = self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned");
            let Some(current) = entries.get(path) else {
                return Err(FilesystemError::VersionMismatch { path: path.clone() });
            };
            if !matches!(cas, CasExpectation::Version(expected) if expected == current.version) {
                return Err(FilesystemError::VersionMismatch { path: path.clone() });
            }
            entries.remove(path);
            Ok(())
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

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            let prefix = format!("{}/", path.as_str());
            let entries = self
                .entries
                .lock()
                .expect("memory filesystem mutex is not poisoned");
            Ok(entries
                .keys()
                .filter_map(|entry_path| {
                    let file_name = entry_path.as_str().strip_prefix(&prefix)?;
                    (!file_name.contains('/')).then(|| {
                        DirEntry::from_backend(
                            entry_path.clone(),
                            file_name.to_owned(),
                            FileType::File,
                        )
                    })
                })
                .collect())
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

    fn test_store() -> FilesystemDraftStore {
        test_store_with_filesystem().0
    }

    fn test_store_with_filesystem() -> (FilesystemDraftStore, Arc<MemoryCasFilesystem>) {
        let filesystem = Arc::new(MemoryCasFilesystem::default());
        let store_filesystem: Arc<dyn Filesystem> = filesystem.clone();
        (FilesystemDraftStore::new(store_filesystem), filesystem)
    }

    fn valid_schema() -> SchemaDocument {
        let object_type_id = ObjectTypeId::from(Uuid::from_u128(1));
        SchemaDocument {
            schema_version: 1,
            object_types: vec![ObjectType {
                id: object_type_id,
                name: "person".to_owned(),
                description: "a person".to_owned(),
                properties: vec![PropertyType {
                    id: PropertyTypeId::from(Uuid::from_u128(2)),
                    name: "name".to_owned(),
                    description: "display name".to_owned(),
                    value_type: ValueType::String,
                    required: true,
                }],
            }],
            link_types: vec![LinkType::new(
                LinkTypeId::from(Uuid::from_u128(3)),
                "knows".to_owned(),
                object_type_id,
                object_type_id,
                Cardinality::ManyToMany,
            )],
        }
    }

    fn changed_schema() -> SchemaDocument {
        let mut schema = valid_schema();
        schema.object_types[0].description = "a changed person".to_owned();
        schema
    }

    fn another_schema() -> SchemaDocument {
        let mut schema = valid_schema();
        schema.object_types[0].properties[0].required = false;
        schema
    }

    #[test]
    fn canonical_schema_bytes_are_stable_and_follow_declared_field_order()
    -> Result<(), OntologyError> {
        let bytes = canonical_schema_bytes(&valid_schema())?;

        assert_eq!(bytes, canonical_schema_bytes(&valid_schema())?);
        assert_eq!(
            bytes,
            br#"{"schema_version":1,"object_types":[{"id":"00000000-0000-0000-0000-000000000001","name":"person","description":"a person","properties":[{"id":"00000000-0000-0000-0000-000000000002","name":"name","description":"display name","value_type":"string","required":true}]}],"link_types":[{"id":"00000000-0000-0000-0000-000000000003","name":"knows","description":"","source_object_type_id":"00000000-0000-0000-0000-000000000001","target_object_type_id":"00000000-0000-0000-0000-000000000001","cardinality":"many_to_many"}]}"#
        );
        Ok(())
    }

    #[tokio::test]
    async fn create_rejects_an_existing_draft() -> Result<(), OntologyError> {
        let store = test_store();
        let name = DraftName::try_from("main".to_owned())?;
        store.create(name.clone(), valid_schema()).await?;

        let duplicate = store.create(name, changed_schema()).await;

        assert!(matches!(
            duplicate,
            Err(OntologyError::DraftConflict { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn replace_rejects_a_stale_digest() -> Result<(), OntologyError> {
        let store = test_store();
        let created = store
            .create(DraftName::try_from("main".to_owned())?, valid_schema())
            .await?;
        let changed = store
            .replace(&created.name, created.digest.clone(), changed_schema())
            .await?;
        let stale = store
            .replace(&created.name, created.digest, another_schema())
            .await;

        assert!(matches!(stale, Err(OntologyError::DraftConflict { .. })));
        assert_ne!(changed.digest, revision_id(&another_schema())?);
        Ok(())
    }

    #[tokio::test]
    async fn load_and_list_return_created_drafts() -> Result<(), OntologyError> {
        let store = test_store();
        let main = store
            .create(DraftName::try_from("main".to_owned())?, valid_schema())
            .await?;
        let archived = store
            .create(
                DraftName::try_from("archived".to_owned())?,
                changed_schema(),
            )
            .await?;

        assert_eq!(store.load(&main.name).await?, main);
        assert_eq!(store.list().await?, vec![archived, main]);
        Ok(())
    }

    #[tokio::test]
    async fn delete_requires_the_current_digest_and_removes_the_draft() -> Result<(), OntologyError>
    {
        let store = test_store();
        let created = store
            .create(DraftName::try_from("main".to_owned())?, valid_schema())
            .await?;
        let changed = store
            .replace(&created.name, created.digest.clone(), changed_schema())
            .await?;

        let stale = store.delete(&created.name, created.digest).await;
        assert!(matches!(stale, Err(OntologyError::DraftConflict { .. })));

        store.delete(&changed.name, changed.digest).await?;
        assert!(matches!(
            store.load(&created.name).await,
            Err(OntologyError::DraftMissing { .. })
        ));
        Ok(())
    }

    #[tokio::test]
    async fn replace_maps_a_post_read_version_mismatch_to_a_draft_conflict()
    -> Result<(), OntologyError> {
        let (store, filesystem) = test_store_with_filesystem();
        let created = store
            .create(DraftName::try_from("main".to_owned())?, valid_schema())
            .await?;
        filesystem.advance_version_after_next_get();

        let replaced = store
            .replace(&created.name, created.digest, changed_schema())
            .await;

        assert!(matches!(replaced, Err(OntologyError::DraftConflict { .. })));
        Ok(())
    }

    #[tokio::test]
    async fn delete_maps_a_post_read_version_mismatch_to_a_draft_conflict()
    -> Result<(), OntologyError> {
        let (store, filesystem) = test_store_with_filesystem();
        let created = store
            .create(DraftName::try_from("main".to_owned())?, valid_schema())
            .await?;
        filesystem.advance_version_after_next_get();

        let deleted = store.delete(&created.name, created.digest).await;

        assert!(matches!(deleted, Err(OntologyError::DraftConflict { .. })));
        Ok(())
    }

    #[tokio::test]
    async fn create_maps_unsupported_cas_to_an_explicit_draft_error() -> Result<(), OntologyError> {
        let (store, filesystem) = test_store_with_filesystem();
        filesystem.reject_next_put_with_unsupported_cas();

        let created = store
            .create(DraftName::try_from("main".to_owned())?, valid_schema())
            .await;

        assert!(matches!(created, Err(OntologyError::DraftCasUnsupported)));
        Ok(())
    }
}
