//! Builtin apply-patch tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use wyse_core::ToolSpec;
use wyse_filesystem::{
    ApplyPatchError, ApplyPatchOperation, ApplyPatchOperationKind, ApplyPatchStatus, Filesystem,
    FilesystemError, VirtualPath, apply_patch,
};

use crate::{Tool, ToolError, ToolInput, ToolOutput};

/// Builtin tool that applies file patches through a virtual filesystem.
pub struct ApplyPatchTool {
    filesystem: Arc<dyn Filesystem>,
    spec: ToolSpec,
}

impl ApplyPatchTool {
    /// Creates an apply-patch tool with an explicit filesystem.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self {
            filesystem,
            spec: ToolSpec::builder()
                .name("apply_patch")
                .description(
                    "applies a create, update, or delete patch inside the virtual filesystem",
                )
                .input_schema(json!({
                    "type": "object",
                    "required": ["operation"],
                    "properties": {
                        "operation": {
                            "type": "object",
                            "required": ["type", "path"],
                            "properties": {
                                "type": {
                                    "type": "string",
                                    "enum": ["create_file", "update_file", "delete_file"]
                                },
                                "path": { "type": "string" },
                                "diff": { "type": "string" }
                            }
                        }
                    }
                }))
                .build(),
        }
    }
}

#[async_trait]
impl Tool for ApplyPatchTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let raw: ApplyPatchInput = serde_json::from_value(input.arguments)
            .map_err(|source| ToolError::InvalidInput { source })?;
        let operation = operation_from_raw(raw.operation)?;
        let display_path = display_path(&operation.path);
        let result = match apply_patch(self.filesystem.as_ref(), &operation).await {
            Ok(output) => ApplyPatchOutputForTool {
                status: output.status,
                output: success_output(operation.kind, &display_path),
            },
            Err(error) => ApplyPatchOutputForTool {
                status: ApplyPatchStatus::Failed,
                output: failed_output(error, &display_path),
            },
        };

        Ok(ToolOutput::new(json!({
            "status": status_text(result.status),
            "output": result.output,
        })))
    }
}

#[derive(Debug, Deserialize)]
struct ApplyPatchInput {
    operation: RawOperation,
}

#[derive(Debug, Deserialize)]
struct RawOperation {
    #[serde(rename = "type")]
    kind: String,
    path: String,
    diff: Option<String>,
}

struct ApplyPatchOutputForTool {
    status: ApplyPatchStatus,
    output: String,
}

fn operation_from_raw(raw: RawOperation) -> Result<ApplyPatchOperation, ToolError> {
    let kind = match raw.kind.as_str() {
        "create_file" => ApplyPatchOperationKind::CreateFile,
        "update_file" => ApplyPatchOperationKind::UpdateFile,
        "delete_file" => ApplyPatchOperationKind::DeleteFile,
        _ => {
            return Err(ToolError::InvalidOperation {
                operation: raw.kind,
            });
        }
    };
    let path = normalize_path(&raw.path)?;
    Ok(ApplyPatchOperation::new(kind, path, raw.diff))
}

fn normalize_path(path: &str) -> Result<VirtualPath, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidPath {
            path: path.to_owned(),
            source: wyse_filesystem::VirtualPathError,
        });
    }

    let normalized = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    VirtualPath::try_from(normalized.as_str()).map_err(|source| ToolError::InvalidPath {
        path: path.to_owned(),
        source,
    })
}

fn display_path(path: &VirtualPath) -> String {
    path.as_str().trim_start_matches('/').to_owned()
}

fn status_text(status: ApplyPatchStatus) -> &'static str {
    match status {
        ApplyPatchStatus::Completed => "completed",
        ApplyPatchStatus::Failed => "failed",
        _ => "failed",
    }
}

fn success_output(kind: ApplyPatchOperationKind, path: &str) -> String {
    match kind {
        ApplyPatchOperationKind::CreateFile => format!("created {path}"),
        ApplyPatchOperationKind::UpdateFile => format!("updated {path}"),
        ApplyPatchOperationKind::DeleteFile => format!("deleted {path}"),
        _ => format!("patched {path}"),
    }
}

fn failed_output(error: ApplyPatchError, path: &str) -> String {
    match error {
        ApplyPatchError::MissingDiff { operation } => {
            format!("patch diff is required for {operation}")
        }
        ApplyPatchError::ContextMismatch { .. } => {
            format!("patch context did not match {path}")
        }
        ApplyPatchError::Filesystem { source } => filesystem_error_output(source, path),
        _ => format!("patch failed {path}"),
    }
}

fn filesystem_error_output(error: FilesystemError, path: &str) -> String {
    match error {
        FilesystemError::NotFound { .. } => format!("file not found at path '{path}'"),
        FilesystemError::AlreadyExists { .. } => format!("file already exists at path '{path}'"),
        FilesystemError::NotAFile { .. } => format!("path is not a file '{path}'"),
        FilesystemError::NotADirectory { .. } => format!("path is not a directory '{path}'"),
        FilesystemError::DirectoryNotEmpty { .. } => format!("directory is not empty '{path}'"),
        FilesystemError::PermissionDenied { .. } => format!("permission denied '{path}'"),
        FilesystemError::ContentTooLarge { .. } => format!("content too large '{path}'"),
        FilesystemError::PathEscapesSandbox { .. } => format!("path escapes sandbox '{path}'"),
        FilesystemError::InvalidVirtualPath { .. } => format!("invalid path '{path}'"),
        FilesystemError::LocalIo { .. } => format!("local filesystem operation failed '{path}'"),
        _ => format!("filesystem operation failed '{path}'"),
    }
}
