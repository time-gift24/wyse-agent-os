//! Public filesystem trait and metadata types.

use crate::{FilesystemError, VirtualPath};

/// Agent-visible filesystem operations.
/// Native async trait methods are intentional for this crate's IO boundary.
#[allow(async_fn_in_trait)]
pub trait Filesystem: Send + Sync {
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
