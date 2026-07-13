//! Error types for virtual filesystem operations.

use std::io;

use thiserror::Error;

use crate::{VirtualPath, VirtualPathError};

/// Error returned by virtual filesystem operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FilesystemError {
    /// The backend does not provide compare-and-swap operations.
    #[error("filesystem backend does not support compare-and-swap")]
    UnsupportedCas,
    /// A compare-and-swap expectation did not match the current record version.
    #[error("record version mismatch {path}")]
    VersionMismatch {
        /// Virtual path whose version did not match.
        path: VirtualPath,
    },
    /// A backend cannot assign another record version.
    #[error("record version overflow {path}")]
    VersionOverflow {
        /// Virtual path whose version overflowed.
        path: VirtualPath,
    },
    /// The local compare-and-swap record state lock was poisoned.
    #[error("local filesystem record state lock poisoned")]
    RecordStatePoisoned,
    /// A virtual path failed validation.
    #[error("invalid virtual path {path}")]
    InvalidVirtualPath {
        /// Rejected path text.
        path: String,
        /// Validation failure source.
        #[source]
        source: VirtualPathError,
    },
    /// Path resolution would escape the sandbox.
    #[error("path escapes sandbox {path}")]
    PathEscapesSandbox {
        /// Virtual path that escaped.
        path: VirtualPath,
    },
    /// The requested path does not exist.
    #[error("path not found {path}")]
    NotFound {
        /// Missing virtual path.
        path: VirtualPath,
    },
    /// The requested path already exists.
    #[error("path already exists {path}")]
    AlreadyExists {
        /// Existing virtual path.
        path: VirtualPath,
    },
    /// The path is not a file.
    #[error("path is not a file {path}")]
    NotAFile {
        /// Virtual path.
        path: VirtualPath,
    },
    /// The path is not a directory.
    #[error("path is not a directory {path}")]
    NotADirectory {
        /// Virtual path.
        path: VirtualPath,
    },
    /// The directory is not empty.
    #[error("directory is not empty {path}")]
    DirectoryNotEmpty {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Operation was denied by the operating system.
    #[error("permission denied {path}")]
    PermissionDenied {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Content exceeds the configured size limit.
    #[error("content too large {path}")]
    ContentTooLarge {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Local filesystem operation failed.
    #[error("local filesystem operation failed {operation} {path}")]
    LocalIo {
        /// Safe operation name.
        operation: &'static str,
        /// Virtual path.
        path: VirtualPath,
        /// Source IO error.
        #[source]
        source: io::Error,
    },
}

impl FilesystemError {
    pub(crate) fn invalid_virtual_path(path: impl Into<String>, source: VirtualPathError) -> Self {
        Self::InvalidVirtualPath {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn local_io(operation: &'static str, path: VirtualPath, source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound { path },
            io::ErrorKind::AlreadyExists => Self::AlreadyExists { path },
            io::ErrorKind::PermissionDenied => Self::PermissionDenied { path },
            io::ErrorKind::DirectoryNotEmpty => Self::DirectoryNotEmpty { path },
            _ => Self::LocalIo {
                operation,
                path,
                source,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VirtualPath;

    #[test]
    fn invalid_path_error_does_not_include_host_paths() {
        let source = crate::VirtualPathError;
        let error = FilesystemError::invalid_virtual_path("/tmp/secret", source);
        let text = error.to_string();

        assert_eq!(text, "invalid virtual path /tmp/secret");
        assert!(!format!("{error:?}").contains("/Users/"));
    }

    #[test]
    fn not_found_mentions_only_virtual_path() {
        let path = VirtualPath::try_from("/missing.txt").expect("path is valid");
        let error = FilesystemError::NotFound { path };

        assert_eq!(error.to_string(), "path not found /missing.txt");
    }
}
