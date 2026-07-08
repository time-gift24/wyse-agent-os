//! Local sandbox filesystem backend.

use std::path::PathBuf;

use bytes::Bytes;
use tokio::fs;

use crate::{DirEntry, FileMetadata, FileType, Filesystem, FilesystemError, VirtualPath};

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

        Ok(Self {
            root,
            max_file_bytes: config.max_file_bytes,
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

    async fn ensure_write_target_inside_root(
        &self,
        path: &VirtualPath,
    ) -> Result<PathBuf, FilesystemError> {
        let host = self.host_path(path);
        if let Ok(metadata) = fs::symlink_metadata(&host).await {
            if metadata.file_type().is_symlink() {
                return Err(FilesystemError::PathEscapesSandbox { path: path.clone() });
            }
        }
        if fs::try_exists(&host)
            .await
            .map_err(|source| FilesystemError::local_io("try_exists", path.clone(), source))?
        {
            let canonical = fs::canonicalize(&host).await.map_err(|source| {
                FilesystemError::local_io("canonicalize", path.clone(), source)
            })?;
            if !canonical.starts_with(&self.root) {
                return Err(FilesystemError::PathEscapesSandbox { path: path.clone() });
            }
            Ok(canonical)
        } else {
            self.ensure_parent_inside_root(path).await
        }
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

impl Filesystem for LocalFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Bytes, FilesystemError> {
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
        Ok(Bytes::from(content))
    }

    async fn write_file(&self, path: &VirtualPath, contents: Bytes) -> Result<(), FilesystemError> {
        let len = u64::try_from(contents.len())
            .map_err(|_| FilesystemError::ContentTooLarge { path: path.clone() })?;
        self.check_len(path, len)?;
        let host = self.ensure_write_target_inside_root(path).await?;
        fs::write(&host, contents)
            .await
            .map_err(|source| FilesystemError::local_io("write", path.clone(), source))
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
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let child_path = child_virtual_path(path, &file_name)?;
            let entry_file_type = entry.file_type().await.map_err(|source| {
                FilesystemError::local_io("entry_file_type", child_path.clone(), source)
            })?;
            let file_type = file_type_from_file_type(&entry_file_type);
            let entry_metadata = if file_type.is_file() {
                Some(entry.metadata().await.map_err(|source| {
                    FilesystemError::local_io("entry_metadata", child_path.clone(), source)
                })?)
            } else {
                None
            };
            entries.push(DirEntry {
                path: child_path,
                file_name,
                file_type,
                metadata: Some(FileMetadata {
                    file_type,
                    len: entry_metadata.map(|metadata| metadata.len()),
                }),
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
        let file_type = file_type_from_metadata(&metadata);
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
        let host = self.ensure_existing_inside_root(path).await?;
        fs::remove_file(&host)
            .await
            .map_err(|source| FilesystemError::local_io("remove_file", path.clone(), source))
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        if path.as_str() == "/" {
            return Err(FilesystemError::DirectoryNotEmpty { path: path.clone() });
        }
        let host = self.ensure_existing_inside_root(path).await?;
        fs::remove_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("remove_dir", path.clone(), source))
    }
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

fn file_type_from_metadata(metadata: &std::fs::Metadata) -> FileType {
    file_type_from_file_type(&metadata.file_type())
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::{Filesystem, VirtualPath};

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
        fs.write_file(&file, Bytes::from_static(b"pub fn ok() {}\n"))
            .await
            .expect("write file");

        let content = fs.read_file(&file).await.expect("read file");
        assert_eq!(content, Bytes::from_static(b"pub fn ok() {}\n"));

        let entries = fs.list_dir(&dir).await.expect("list dir");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, file);

        fs.remove_file(&file).await.expect("remove file");
        fs.remove_dir(&dir).await.expect("remove empty dir");

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

        let temp = std::env::temp_dir().join(format!("wyse-fs-symlink-{}", std::process::id()));
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
        assert_eq!(
            link_entry
                .metadata
                .expect("symlink metadata is present")
                .file_type,
            FileType::Symlink
        );

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
            .write_file(&link, Bytes::from_static(b"owned"))
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
            .write_file(&link, Bytes::from_static(b"owned"))
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
}
