//! Public filesystem trait and metadata types.

use async_trait::async_trait;

use crate::{CasExpectation, Entry, FilesystemError, RecordVersion, VersionedEntry, VirtualPath};

/// Agent-visible filesystem operations.
///
/// This trait is object-safe so runtime tools can receive explicit filesystem
/// dependencies without knowing the backend type.
#[async_trait]
pub trait Filesystem: Send + Sync {
    /// Reads one versioned record.
    ///
    /// # Errors
    ///
    /// Returns [`FilesystemError::UnsupportedCas`] when the backend does not support
    /// compare-and-swap operations, or another backend error when the read fails.
    async fn get(&self, _path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }

    /// Writes one record when its compare-and-swap expectation holds.
    ///
    /// # Errors
    ///
    /// Returns [`FilesystemError::UnsupportedCas`] when the backend does not support
    /// compare-and-swap operations, [`FilesystemError::VersionMismatch`] when the
    /// expectation fails, or another backend error when the write fails.
    async fn put(
        &self,
        _path: &VirtualPath,
        _entry: Entry,
        _cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        Err(FilesystemError::UnsupportedCas)
    }

    /// Reads a complete file into memory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a file, too large, or the backend fails.
    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError>;

    /// Writes complete file contents.
    ///
    /// # Errors
    ///
    /// Returns an error when the path escapes the backend, cannot be written, or exceeds limits.
    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError>;

    /// Lists one directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a directory, or the backend fails.
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;

    /// Returns metadata for a path.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing or the backend fails.
    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError>;

    /// Creates one directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created or its parent is missing.
    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError>;

    /// Removes one file.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a file, or cannot be removed.
    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError>;

    /// Removes one empty directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a directory, not empty, or cannot be removed.
    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError>;
}

/// Metadata for one filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileMetadata {
    /// Path type.
    pub file_type: FileType,
    /// File length in bytes when known.
    pub len: Option<u64>,
}

/// Directory entry returned by [`Filesystem::list_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DirEntry {
    /// Entry path.
    pub path: VirtualPath,
    /// Final path segment.
    pub file_name: String,
    /// Entry type.
    pub file_type: FileType,
}

impl DirEntry {
    /// Creates a directory entry from backend-returned values.
    #[doc(hidden)]
    #[must_use]
    pub fn from_backend(path: VirtualPath, file_name: String, file_type: FileType) -> Self {
        Self {
            path,
            file_name,
            file_type,
        }
    }
}

/// Type of one filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileType {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
    /// Other platform-specific file type.
    Other,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backend_can_construct_a_directory_entry() {
        let path = VirtualPath::try_from("/messages/1.json").expect("valid path");

        let entry = DirEntry::from_backend(path.clone(), "1.json".to_owned(), FileType::File);

        assert_eq!(entry.path, path);
        assert_eq!(entry.file_name, "1.json");
        assert_eq!(entry.file_type, FileType::File);
    }
}
