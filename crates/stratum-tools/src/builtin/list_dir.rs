//! Builtin list-directory tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use stratum_core::ToolSpec;
use stratum_filesystem::Filesystem;

use crate::{
    Tool, ToolError, ToolInput, ToolOutput,
    builtin::filesystem::{display_path, file_type_label, normalize_path},
};

/// Builtin tool that lists one directory.
pub struct ListDirTool {
    filesystem: Arc<dyn Filesystem>,
    spec: ToolSpec,
}

impl ListDirTool {
    /// Creates a list-directory tool with an explicit filesystem.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self {
            filesystem,
            spec: ToolSpec::builder()
                .name("list_dir")
                .description("lists entries in one directory inside the virtual filesystem")
                .input_schema(json!({
                    "type": "object",
                    "required": ["path"],
                    "properties": {
                        "path": { "type": "string" }
                    }
                }))
                .build(),
        }
    }
}

#[async_trait]
impl Tool for ListDirTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let raw: PathInput = serde_json::from_value(input.arguments)
            .map_err(|source| ToolError::InvalidInput { source })?;
        let path = normalize_path(&raw.path)?;
        let entries = self.filesystem.list_dir(&path).await?;
        let entries = entries
            .into_iter()
            .map(|entry| {
                json!({
                    "path": display_path(&entry.path),
                    "file_name": entry.file_name,
                    "file_type": file_type_label(entry.file_type),
                })
            })
            .collect::<Vec<_>>();

        Ok(ToolOutput::new(json!({
            "path": display_path(&path),
            "entries": entries,
        })))
    }
}

#[derive(Debug, Deserialize)]
struct PathInput {
    path: String,
}
