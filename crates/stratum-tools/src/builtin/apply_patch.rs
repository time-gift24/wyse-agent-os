//! Builtin apply-patch tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use stratum_core::ToolSpec;
use stratum_filesystem::{
    ApplyPatchError, ApplyPatchOperation, ApplyPatchOperationKind, Filesystem, FilesystemError,
    apply_patch,
};

use crate::{
    Tool, ToolError, ToolInput, ToolOutput,
    builtin::filesystem::{display_path, normalize_path},
};

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
        let (status, output) = match apply_patch(self.filesystem.as_ref(), &operation).await {
            Ok(()) => ("completed", success_output(operation.kind, &display_path)),
            Err(error) => ("failed", failed_output(&error, &display_path)),
        };

        Ok(ToolOutput::new(json!({
            "status": status,
            "output": output,
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

fn success_output(kind: ApplyPatchOperationKind, path: &str) -> String {
    match kind {
        ApplyPatchOperationKind::CreateFile => format!("created {path}"),
        ApplyPatchOperationKind::UpdateFile => format!("updated {path}"),
        ApplyPatchOperationKind::DeleteFile => format!("deleted {path}"),
        _ => format!("patched {path}"),
    }
}

fn failed_output(error: &ApplyPatchError, path: &str) -> String {
    match error {
        ApplyPatchError::MissingDiff { operation } => {
            format!("patch diff is required for {operation}")
        }
        ApplyPatchError::ContextMismatch { .. } => {
            format!("patch context did not match {path}")
        }
        ApplyPatchError::Filesystem { source } => filesystem_error_output(source, path),
        _ => error.to_string(),
    }
}

fn filesystem_error_output(error: &FilesystemError, path: &str) -> String {
    match error {
        FilesystemError::NotFound { .. } => format!("file not found at path '{path}'"),
        FilesystemError::AlreadyExists { .. } => format!("file already exists at path '{path}'"),
        _ => error.to_string(),
    }
}
