//! Local sandbox filesystem backend.

use std::{
    collections::BTreeMap,
    ffi::OsString,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use tokio::fs;

use crate::{
    CasExpectation, DirEntry, Entry, FileMetadata, FileType, Filesystem, FilesystemError,
    RecordVersion, VersionedEntry, VirtualPath, VirtualPathError,
};

/// Configuration for [`LocalFilesystem`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFilesystemConfig {
    /// Host root directory for this sandbox.
    pub root: PathBuf,
    /// Maximum bytes allowed for one whole-file read or write.
    pub max_file_bytes: Option<u64>,
}

/// Local filesystem backend rooted at one sandbox directory.
#[derive(Debug, Clone)]
pub struct LocalFilesystem {
    root: PathBuf,
    max_file_bytes: Option<u64>,
    records: Arc<Mutex<BTreeMap<VirtualPath, RecordVersion>>>,
    next_record_version: Arc<AtomicU64>,
}

impl LocalFilesystem {
    /// Creates a local sandbox filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized.
    pub fn new(config: LocalFilesystemConfig) -> Result<Self, FilesystemError> {
        let root = config.root.canonicalize().map_err(|source| {
            FilesystemError::local_io(
                "canonicalize_root",
                VirtualPath::try_from("/").expect("root virtual path is valid"),
                source,
            )
        })?;
        if !root.is_dir() {
            return Err(FilesystemError::NotADirectory {
                path: VirtualPath::try_from("/").expect("root virtual path is valid"),
            });
        }

        Ok(Self {
            root,
            max_file_bytes: config.max_file_bytes,
            records: Arc::new(Mutex::new(BTreeMap::new())),
            next_record_version: Arc::new(AtomicU64::new(1)),
        })
    }

    fn host_path(&self, path: &VirtualPath) -> PathBuf {
        let mut host = self.root.clone();
        for segment in path.segments() {
            host.push(segment);
        }
        host
    }

    async fn ensure_parent_inside_root(
        &self,
        path: &VirtualPath,
    ) -> Result<PathBuf, FilesystemError> {
        if path.as_str() == "/" {
            return Ok(self.root.clone());
        }

        let host = self.host_path(path);
        let parent = host.parent().unwrap_or(&self.root);
        let canonical_parent = fs::canonicalize(parent).await.map_err(|source| {
            FilesystemError::local_io("canonicalize_parent", path.clone(), source)
        })?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(FilesystemError::PathEscapesSandbox { path: path.clone() });
        }
        Ok(host)
    }

    async fn ensure_existing_inside_root(
        &self,
        path: &VirtualPath,
    ) -> Result<PathBuf, FilesystemError> {
        let host = self.host_path(path);
        let canonical = fs::canonicalize(&host)
            .await
            .map_err(|source| FilesystemError::local_io("canonicalize", path.clone(), source))?;
        if !canonical.starts_with(&self.root) {
            return Err(FilesystemError::PathEscapesSandbox { path: path.clone() });
        }
        Ok(canonical)
    }

    fn check_len(&self, path: &VirtualPath, len: u64) -> Result<(), FilesystemError> {
        if self.max_file_bytes.is_some_and(|max| len > max) {
            return Err(FilesystemError::ContentTooLarge { path: path.clone() });
        }
        Ok(())
    }
}

#[async_trait]
impl Filesystem for LocalFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        let root = self.root.clone();
        let path = path.clone();
        let error_path = path.clone();
        let records = Arc::clone(&self.records);
        let next_record_version = Arc::clone(&self.next_record_version);
        let max_file_bytes = self.max_file_bytes;

        tokio::task::spawn_blocking(move || {
            let host = host_path(&root, &path);
            let metadata = match std::fs::symlink_metadata(&host) {
                Ok(metadata) => metadata,
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(None),
                Err(source) => return Err(FilesystemError::local_io("metadata", path, source)),
            };
            if metadata.file_type().is_symlink() {
                return Err(FilesystemError::PathEscapesSandbox { path });
            }
            let canonical = std::fs::canonicalize(&host).map_err(|source| {
                FilesystemError::local_io("canonicalize", path.clone(), source)
            })?;
            if !canonical.starts_with(&root) {
                return Err(FilesystemError::PathEscapesSandbox { path });
            }
            if !metadata.is_file() {
                return Err(FilesystemError::NotAFile { path });
            }
            if max_file_bytes.is_some_and(|max| metadata.len() > max) {
                return Err(FilesystemError::ContentTooLarge { path });
            }

            let mut records = records
                .lock()
                .map_err(|_| FilesystemError::RecordStatePoisoned)?;
            let contents = std::fs::read(&canonical)
                .map_err(|source| FilesystemError::local_io("read", path.clone(), source))?;
            let version = match records.get(&path) {
                Some(version) => *version,
                None => {
                    let value = next_record_version
                        .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                            value.checked_add(1)
                        })
                        .map_err(|_| FilesystemError::VersionOverflow { path: path.clone() })?;
                    let version = RecordVersion::from_backend(value);
                    records.insert(path.clone(), version);
                    version
                }
            };
            Ok(Some(VersionedEntry {
                entry: Entry::new(contents),
                version,
            }))
        })
        .await
        .map_err(|error| {
            FilesystemError::local_io("cas_get", error_path, std::io::Error::other(error))
        })?
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        self.check_len(
            path,
            u64::try_from(entry.contents().len())
                .map_err(|_| FilesystemError::ContentTooLarge { path: path.clone() })?,
        )?;
        let root = self.root.clone();
        let path = path.clone();
        let error_path = path.clone();
        let records = Arc::clone(&self.records);
        let next_record_version = Arc::clone(&self.next_record_version);

        tokio::task::spawn_blocking(move || {
            let host = host_path(&root, &path);
            let mut records = records
                .lock()
                .map_err(|_| FilesystemError::RecordStatePoisoned)?;
            let metadata = match std::fs::symlink_metadata(&host) {
                Ok(metadata) => Some(metadata),
                Err(source) if source.kind() == std::io::ErrorKind::NotFound => None,
                Err(source) => {
                    return Err(FilesystemError::local_io("metadata", path.clone(), source));
                }
            };
            let target = if let Some(metadata) = metadata.as_ref() {
                if metadata.file_type().is_symlink() {
                    return Err(FilesystemError::PathEscapesSandbox { path });
                }
                if !metadata.is_file() {
                    return Err(FilesystemError::NotAFile { path });
                }
                let canonical = std::fs::canonicalize(&host).map_err(|source| {
                    FilesystemError::local_io("canonicalize", path.clone(), source)
                })?;
                if !canonical.starts_with(&root) {
                    return Err(FilesystemError::PathEscapesSandbox { path });
                }
                canonical
            } else {
                let parent = host.parent().unwrap_or(&root);
                let canonical_parent = std::fs::canonicalize(parent).map_err(|source| {
                    FilesystemError::local_io("canonicalize_parent", path.clone(), source)
                })?;
                if !canonical_parent.starts_with(&root) {
                    return Err(FilesystemError::PathEscapesSandbox { path });
                }
                host
            };

            let current = if metadata.is_some() {
                Some(match records.get(&path) {
                    Some(version) => *version,
                    None => {
                        let value = next_record_version
                            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                                value.checked_add(1)
                            })
                            .map_err(|_| FilesystemError::VersionOverflow { path: path.clone() })?;
                        let version = RecordVersion::from_backend(value);
                        records.insert(path.clone(), version);
                        version
                    }
                })
            } else {
                None
            };
            match cas {
                CasExpectation::Absent if current.is_some() => {
                    return Err(FilesystemError::VersionMismatch { path });
                }
                CasExpectation::Version(expected) if current != Some(expected) => {
                    return Err(FilesystemError::VersionMismatch { path });
                }
                _ => {}
            }
            let value = next_record_version
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| FilesystemError::VersionOverflow { path: path.clone() })?;
            let version = RecordVersion::from_backend(value);
            let temporary = target.with_extension(format!("wyse-{value}.tmp"));
            std::fs::write(&temporary, entry.into_contents())
                .map_err(|source| FilesystemError::local_io("write", path.clone(), source))?;
            if let Err(source) = std::fs::rename(&temporary, &target) {
                let _ = std::fs::remove_file(&temporary);
                return Err(FilesystemError::local_io("rename", path, source));
            }
            records.insert(path, version);
            Ok(version)
        })
        .await
        .map_err(|error| {
            FilesystemError::local_io("cas_put", error_path, std::io::Error::other(error))
        })?
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        let metadata = fs::metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotAFile { path: path.clone() });
        }
        self.check_len(path, metadata.len())?;
        let content = fs::read(&host)
            .await
            .map_err(|source| FilesystemError::local_io("read", path.clone(), source))?;
        Ok(content)
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.put(path, Entry::new(contents), CasExpectation::Any)
            .await
            .map(|_| ())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        let metadata = fs::metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        if !metadata.is_dir() {
            return Err(FilesystemError::NotADirectory { path: path.clone() });
        }

        let mut read_dir = fs::read_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("read_dir", path.clone(), source))?;
        let mut entries = Vec::new();
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|source| FilesystemError::local_io("read_dir_entry", path.clone(), source))?
        {
            let file_name = file_name_to_string(entry.file_name())?;
            let child_path = child_virtual_path(path, &file_name)?;
            let entry_file_type = entry.file_type().await.map_err(|source| {
                FilesystemError::local_io("entry_file_type", child_path.clone(), source)
            })?;
            let file_type = file_type_from_file_type(&entry_file_type);
            entries.push(DirEntry {
                path: child_path,
                file_name,
                file_type,
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        let host = self.ensure_parent_inside_root(path).await?;
        let metadata = fs::symlink_metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        let file_type = file_type_from_file_type(&metadata.file_type());
        Ok(FileMetadata {
            file_type,
            len: metadata.is_file().then_some(metadata.len()),
        })
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let host = self.ensure_parent_inside_root(path).await?;
        fs::create_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("create_dir", path.clone(), source))
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let root = self.root.clone();
        let path = path.clone();
        let error_path = path.clone();
        let records = Arc::clone(&self.records);
        let next_record_version = Arc::clone(&self.next_record_version);

        tokio::task::spawn_blocking(move || {
            let host = host_path(&root, &path);
            let parent = host.parent().unwrap_or(&root);
            let canonical_parent = std::fs::canonicalize(parent).map_err(|source| {
                FilesystemError::local_io("canonicalize_parent", path.clone(), source)
            })?;
            if !canonical_parent.starts_with(&root) {
                return Err(FilesystemError::PathEscapesSandbox { path });
            }
            let metadata = std::fs::symlink_metadata(&host)
                .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
            if !metadata.is_file() && !metadata.file_type().is_symlink() {
                return Err(FilesystemError::NotAFile { path });
            }

            let mut records = records
                .lock()
                .map_err(|_| FilesystemError::RecordStatePoisoned)?;
            std::fs::remove_file(&host)
                .map_err(|source| FilesystemError::local_io("remove_file", path.clone(), source))?;
            let value = next_record_version
                .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                    value.checked_add(1)
                })
                .map_err(|_| FilesystemError::VersionOverflow { path: path.clone() })?;
            records.insert(path, RecordVersion::from_backend(value));
            Ok(())
        })
        .await
        .map_err(|error| {
            FilesystemError::local_io("remove_file", error_path, std::io::Error::other(error))
        })?
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if path.as_str() == "/" {
            return Err(FilesystemError::DirectoryNotEmpty { path: path.clone() });
        }
        let host = self.ensure_parent_inside_root(path).await?;
        let metadata = fs::symlink_metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        if !metadata.is_dir() {
            return Err(FilesystemError::NotADirectory { path: path.clone() });
        }
        fs::remove_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("remove_dir", path.clone(), source))
    }
}

fn host_path(root: &std::path::Path, path: &VirtualPath) -> PathBuf {
    let mut host = root.to_path_buf();
    for segment in path.segments() {
        host.push(segment);
    }
    host
}

fn child_virtual_path(
    parent: &VirtualPath,
    file_name: &str,
) -> Result<VirtualPath, FilesystemError> {
    let path = if parent.as_str() == "/" {
        format!("/{file_name}")
    } else {
        format!("{}/{}", parent.as_str(), file_name)
    };
    VirtualPath::try_from(path.as_str())
        .map_err(|source| FilesystemError::invalid_virtual_path(path, source))
}

fn file_name_to_string(file_name: OsString) -> Result<String, FilesystemError> {
    file_name.into_string().map_err(|name| {
        FilesystemError::invalid_virtual_path(name.to_string_lossy(), VirtualPathError)
    })
}

fn file_type_from_file_type(file_type: &std::fs::FileType) -> FileType {
    if file_type.is_symlink() {
        FileType::Symlink
    } else if file_type.is_file() {
        FileType::File
    } else if file_type.is_dir() {
        FileType::Directory
    } else {
        FileType::Other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CasExpectation, Entry, Filesystem, VirtualPath};
    use tokio::sync::Barrier;

    #[tokio::test]
    async fn reads_writes_lists_and_removes_inside_sandbox() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-test-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");

        let dir = VirtualPath::try_from("/src").expect("path is valid");
        let file = VirtualPath::try_from("/src/lib.rs").expect("path is valid");

        fs.create_dir(&dir).await.expect("create dir");
        fs.write_file(&file, b"pub fn ok() {}\n".to_vec())
            .await
            .expect("write file");

        let content = fs.read_file(&file).await.expect("read file");
        assert_eq!(content, b"pub fn ok() {}\n");

        let entries = fs.list_dir(&dir).await.expect("list dir");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, file);

        fs.remove_file(&file).await.expect("remove file");
        fs.remove_dir(&dir).await.expect("remove empty dir");

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn local_filesystem_compares_records_and_rejects_stale_versions() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-cas-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        let filesystem = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let path = VirtualPath::try_from("/agent.json").expect("valid path");

        assert!(filesystem.get(&path).await.expect("read record").is_none());
        let first = filesystem
            .put(&path, Entry::new(b"one".to_vec()), CasExpectation::Absent)
            .await
            .expect("create record");
        let stored = filesystem
            .get(&path)
            .await
            .expect("read record")
            .expect("record exists");
        assert_eq!(stored.version, first);
        assert_eq!(stored.entry.contents(), b"one");

        filesystem
            .put(
                &path,
                Entry::new(b"two".to_vec()),
                CasExpectation::Version(stored.version),
            )
            .await
            .expect("update matching record");
        let error = filesystem
            .put(
                &path,
                Entry::new(b"three".to_vec()),
                CasExpectation::Version(stored.version),
            )
            .await
            .expect_err("stale version is rejected");

        assert!(matches!(error, FilesystemError::VersionMismatch { .. }));
        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn concurrent_absent_puts_allow_only_one_creator() {
        let temp =
            std::env::temp_dir().join(format!("wyse-fs-concurrent-cas-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        let filesystem = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let path = VirtualPath::try_from("/agent.json").expect("path is valid");
        let barrier = Arc::new(Barrier::new(2));

        let first = {
            let filesystem = filesystem.clone();
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                filesystem
                    .put(&path, Entry::new(b"first".to_vec()), CasExpectation::Absent)
                    .await
            })
        };
        let second = {
            let filesystem = filesystem.clone();
            let path = path.clone();
            let barrier = Arc::clone(&barrier);
            tokio::spawn(async move {
                barrier.wait().await;
                filesystem
                    .put(
                        &path,
                        Entry::new(b"second".to_vec()),
                        CasExpectation::Absent,
                    )
                    .await
            })
        };

        let results = [
            first.await.expect("first task completes"),
            second.await.expect("second task completes"),
        ];
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert!(
            results
                .iter()
                .any(|result| matches!(result, Err(FilesystemError::VersionMismatch { .. })))
        );

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn ordinary_mutations_invalidate_observed_cas_versions() {
        let temp =
            std::env::temp_dir().join(format!("wyse-fs-cas-mutations-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        let filesystem = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let path = VirtualPath::try_from("/agent.json").expect("valid path");

        filesystem
            .put(&path, Entry::new(b"one".to_vec()), CasExpectation::Absent)
            .await
            .expect("create record");
        let before_write = filesystem
            .get(&path)
            .await
            .expect("read record")
            .expect("record exists");

        filesystem
            .write_file(&path, b"two".to_vec())
            .await
            .expect("ordinary write");
        let write_error = filesystem
            .put(
                &path,
                Entry::new(b"stale write".to_vec()),
                CasExpectation::Version(before_write.version),
            )
            .await
            .expect_err("write invalidates observed version");
        assert!(matches!(
            write_error,
            FilesystemError::VersionMismatch { .. }
        ));

        let before_remove = filesystem
            .get(&path)
            .await
            .expect("read record")
            .expect("record exists");
        filesystem
            .remove_file(&path)
            .await
            .expect("ordinary remove");
        let remove_error = filesystem
            .put(
                &path,
                Entry::new(b"stale remove".to_vec()),
                CasExpectation::Version(before_remove.version),
            )
            .await
            .expect_err("remove invalidates observed version");
        assert!(matches!(
            remove_error,
            FilesystemError::VersionMismatch { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn remove_dir_rejects_non_empty_directory() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-non-empty-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(temp.join("dir"))
            .await
            .expect("create dir");
        tokio::fs::write(temp.join("dir/file.txt"), b"x")
            .await
            .expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let dir = VirtualPath::try_from("/dir").expect("path is valid");

        let error = fs
            .remove_dir(&dir)
            .await
            .expect_err("directory is not empty");
        assert!(matches!(
            error,
            crate::FilesystemError::DirectoryNotEmpty { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn remove_operations_return_typed_file_kind_errors() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-remove-kind-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(temp.join("dir"))
            .await
            .expect("create dir");
        tokio::fs::write(temp.join("file.txt"), b"x")
            .await
            .expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let dir = VirtualPath::try_from("/dir").expect("path is valid");
        let file = VirtualPath::try_from("/file.txt").expect("path is valid");

        let file_error = fs.remove_file(&dir).await.expect_err("dir is not a file");
        assert!(matches!(
            file_error,
            crate::FilesystemError::NotAFile { .. }
        ));
        let dir_error = fs.remove_dir(&file).await.expect_err("file is not a dir");
        assert!(matches!(
            dir_error,
            crate::FilesystemError::NotADirectory { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_file_removes_symlink_without_removing_target() {
        use std::os::unix::fs::symlink;

        let temp =
            std::env::temp_dir().join(format!("wyse-fs-remove-symlink-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        tokio::fs::write(temp.join("target.txt"), b"target")
            .await
            .expect("write target");
        symlink(temp.join("target.txt"), temp.join("link.txt")).expect("create symlink");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let link = VirtualPath::try_from("/link.txt").expect("path is valid");

        fs.remove_file(&link).await.expect("remove symlink");

        assert!(
            !tokio::fs::try_exists(temp.join("link.txt"))
                .await
                .expect("check link")
        );
        assert_eq!(
            tokio::fs::read(temp.join("target.txt"))
                .await
                .expect("read target"),
            b"target"
        );

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_dir_rejects_directory_symlink() {
        use std::os::unix::fs::symlink;

        let temp =
            std::env::temp_dir().join(format!("wyse-fs-remove-dir-symlink-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(temp.join("target"))
            .await
            .expect("create target dir");
        symlink(temp.join("target"), temp.join("link")).expect("create symlink");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let link = VirtualPath::try_from("/link").expect("path is valid");

        let error = fs
            .remove_dir(&link)
            .await
            .expect_err("symlink is not a dir");
        assert!(matches!(
            error,
            crate::FilesystemError::NotADirectory { .. }
        ));
        assert!(
            tokio::fs::try_exists(temp.join("target"))
                .await
                .expect("check target dir")
        );
        assert!(
            tokio::fs::try_exists(temp.join("link"))
                .await
                .expect("check link")
        );

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn rejects_reads_larger_than_limit() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-limit-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        tokio::fs::write(temp.join("big.txt"), b"12345")
            .await
            .expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(4),
        })
        .expect("filesystem is valid");
        let file = VirtualPath::try_from("/big.txt").expect("path is valid");

        let error = fs.read_file(&file).await.expect_err("content is too large");
        assert!(matches!(
            error,
            crate::FilesystemError::ContentTooLarge { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn metadata_and_create_dir_handle_root_path() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-root-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let root = VirtualPath::try_from("/").expect("root path is valid");

        let metadata = fs.metadata(&root).await.expect("root metadata");
        assert_eq!(metadata.file_type, FileType::Directory);
        assert_eq!(metadata.len, None);

        let error = fs.create_dir(&root).await.expect_err("root already exists");
        assert!(matches!(
            error,
            crate::FilesystemError::AlreadyExists { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn rejects_file_as_sandbox_root() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-file-root-{}", std::process::id()));
        let _ = tokio::fs::remove_file(&temp).await;
        tokio::fs::write(&temp, b"not a dir")
            .await
            .expect("write root file");

        let error = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect_err("file root is rejected");
        assert!(matches!(
            error,
            crate::FilesystemError::NotADirectory { .. }
        ));

        let _ = tokio::fs::remove_file(&temp).await;
    }

    #[tokio::test]
    async fn remove_dir_rejects_root_path() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-root-remove-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let root = VirtualPath::try_from("/").expect("root path is valid");

        let error = fs.remove_dir(&root).await.expect_err("root is protected");
        assert!(matches!(
            error,
            crate::FilesystemError::DirectoryNotEmpty { .. }
        ));
        assert!(
            tokio::fs::try_exists(&temp)
                .await
                .expect("check root still exists")
        );

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reports_symlink_metadata_without_following_target() {
        use std::os::unix::fs::symlink;

        let temp =
            std::env::temp_dir().join(format!("wyse-fs-symlink-metadata-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        tokio::fs::write(temp.join("target.txt"), b"hello")
            .await
            .expect("write target");
        symlink(temp.join("target.txt"), temp.join("link.txt")).expect("create symlink");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let link = VirtualPath::try_from("/link.txt").expect("path is valid");

        let metadata = fs.metadata(&link).await.expect("read metadata");
        assert_eq!(metadata.file_type, FileType::Symlink);
        assert_eq!(metadata.len, None);

        let entries = fs
            .list_dir(&VirtualPath::try_from("/").expect("root path is valid"))
            .await
            .expect("list root");
        let link_entry = entries
            .into_iter()
            .find(|entry| entry.path == link)
            .expect("symlink entry exists");
        assert_eq!(link_entry.file_type, FileType::Symlink);

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_file_rejects_existing_symlink_escape() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("wyse-fs-write-{}", std::process::id()));
        let root = base.join("root");
        let outside = base.join("outside");
        let _ = tokio::fs::remove_dir_all(&base).await;
        tokio::fs::create_dir_all(&root).await.expect("create root");
        tokio::fs::create_dir_all(&outside)
            .await
            .expect("create outside");
        tokio::fs::write(outside.join("secret.txt"), b"secret")
            .await
            .expect("write outside target");
        symlink(outside.join("secret.txt"), root.join("link.txt")).expect("create symlink");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let link = VirtualPath::try_from("/link.txt").expect("path is valid");

        let error = fs
            .write_file(&link, b"owned".to_vec())
            .await
            .expect_err("symlink escape is rejected");
        assert!(matches!(
            error,
            crate::FilesystemError::PathEscapesSandbox { .. }
        ));
        assert_eq!(
            tokio::fs::read(outside.join("secret.txt"))
                .await
                .expect("read outside target"),
            b"secret"
        );

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn write_file_rejects_dangling_final_symlink_escape() {
        use std::os::unix::fs::symlink;

        let base = std::env::temp_dir().join(format!("wyse-fs-dangling-{}", std::process::id()));
        let root = base.join("root");
        let outside = base.join("outside");
        let _ = tokio::fs::remove_dir_all(&base).await;
        tokio::fs::create_dir_all(&root).await.expect("create root");
        tokio::fs::create_dir_all(&outside)
            .await
            .expect("create outside");
        symlink(outside.join("missing.txt"), root.join("link.txt")).expect("create symlink");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let link = VirtualPath::try_from("/link.txt").expect("path is valid");

        let error = fs
            .write_file(&link, b"owned".to_vec())
            .await
            .expect_err("dangling symlink escape is rejected");
        assert!(matches!(
            error,
            crate::FilesystemError::PathEscapesSandbox { .. }
        ));
        assert!(
            !tokio::fs::try_exists(outside.join("missing.txt"))
                .await
                .expect("check outside target")
        );

        let _ = tokio::fs::remove_dir_all(&base).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn list_dir_returns_invalid_virtual_path_for_unrepresentable_entry_names() {
        let temp =
            std::env::temp_dir().join(format!("wyse-fs-invalid-name-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp)
            .await
            .expect("create temp root");
        tokio::fs::write(temp.join("bad\\name.txt"), b"hello")
            .await
            .expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let root = VirtualPath::try_from("/").expect("root path is valid");

        let error = fs.list_dir(&root).await.expect_err("invalid entry path");
        assert!(matches!(
            error,
            crate::FilesystemError::InvalidVirtualPath { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[cfg(unix)]
    #[test]
    fn file_name_to_string_rejects_non_utf8_names() {
        use std::os::unix::ffi::OsStringExt;

        let error =
            file_name_to_string(OsString::from_vec(vec![0xff])).expect_err("non-utf8 entry path");
        assert!(matches!(
            error,
            crate::FilesystemError::InvalidVirtualPath { .. }
        ));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_symlink_escape() {
        use std::os::unix::fs::symlink;

        let temp = std::env::temp_dir().join(format!("wyse-fs-symlink-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("wyse-fs-outside-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        let _ = tokio::fs::remove_dir_all(&outside).await;
        tokio::fs::create_dir_all(&temp).await.expect("create root");
        tokio::fs::create_dir_all(&outside)
            .await
            .expect("create outside");
        tokio::fs::write(outside.join("secret.txt"), b"secret")
            .await
            .expect("write outside file");
        symlink(outside.join("secret.txt"), temp.join("link.txt")).expect("create symlink");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let link = VirtualPath::try_from("/link.txt").expect("path is valid");

        let error = fs
            .read_file(&link)
            .await
            .expect_err("symlink escape is rejected");
        assert!(matches!(
            error,
            crate::FilesystemError::PathEscapesSandbox { .. }
        ));

        let _ = tokio::fs::remove_dir_all(&temp).await;
        let _ = tokio::fs::remove_dir_all(&outside).await;
    }
}
