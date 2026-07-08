//! Error types for tool operations.

use thiserror::Error;
use wyse_core::ToolName;
use wyse_filesystem::VirtualPathError;

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
}
