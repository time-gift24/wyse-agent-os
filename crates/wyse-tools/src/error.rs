//! Error types for tool operations.

use std::string::FromUtf8Error;

use thiserror::Error;
use wyse_core::ToolName;
use wyse_filesystem::{FilesystemError, VirtualPathError};

/// Error returned by tool registry or execution operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ToolError {
    /// A tool with this name is already registered.
    #[error("tool is already registered: {name}")]
    DuplicateTool {
        /// Duplicate tool name.
        name: ToolName,
    },
    /// No tool with this name is registered.
    #[error("tool not found: {name}")]
    ToolNotFound {
        /// Missing tool name.
        name: ToolName,
    },
    /// Tool input could not be decoded.
    #[error("invalid tool input")]
    InvalidInput {
        /// Decode failure source.
        #[source]
        source: serde_json::Error,
    },
    /// Tool operation type is unknown.
    #[error("invalid tool operation: {operation}")]
    InvalidOperation {
        /// Rejected operation type.
        operation: String,
    },
    /// Tool path is invalid.
    #[error("invalid path: {path}")]
    InvalidPath {
        /// Rejected path.
        path: String,
        /// Path validation source.
        #[source]
        source: VirtualPathError,
    },
    /// Tool argument is semantically invalid.
    #[error("invalid argument {name}: {reason}")]
    InvalidArgument {
        /// Argument name.
        name: &'static str,
        /// Rejection reason.
        reason: &'static str,
    },
    /// File content is not valid UTF-8.
    #[error("file is not valid utf-8: {path}")]
    InvalidUtf8 {
        /// File path.
        path: String,
        /// UTF-8 conversion source.
        #[source]
        source: FromUtf8Error,
    },
    /// Filesystem operation failed.
    #[error("filesystem operation failed")]
    Filesystem {
        /// Filesystem failure source.
        #[source]
        source: FilesystemError,
    },
}

impl From<FilesystemError> for ToolError {
    fn from(source: FilesystemError) -> Self {
        Self::Filesystem { source }
    }
}
