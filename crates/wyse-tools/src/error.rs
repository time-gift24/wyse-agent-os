//! Error types for tool operations.

use thiserror::Error;
use wyse_core::ToolName;

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
}
