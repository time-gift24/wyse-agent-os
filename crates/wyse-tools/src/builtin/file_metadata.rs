//! Builtin file-metadata tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use wyse_core::ToolSpec;
use wyse_filesystem::Filesystem;

use crate::{
    Tool, ToolError, ToolInput, ToolOutput,
    builtin::filesystem::{display_path, file_type_label, normalize_path},
};

/// Builtin tool that returns metadata for one filesystem path.
pub struct FileMetadataTool {
    filesystem: Arc<dyn Filesystem>,
    spec: ToolSpec,
}

impl FileMetadataTool {
    /// Creates a file-metadata tool with an explicit filesystem.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self {
            filesystem,
            spec: ToolSpec::builder()
                .name("file_metadata")
                .description("returns metadata for one path inside the virtual filesystem")
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
impl Tool for FileMetadataTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let raw: PathInput = serde_json::from_value(input.arguments)
            .map_err(|source| ToolError::InvalidInput { source })?;
        let path = normalize_path(&raw.path)?;
        let metadata = self.filesystem.metadata(&path).await?;

        Ok(ToolOutput::new(json!({
            "path": display_path(&path),
            "file_type": file_type_label(metadata.file_type),
            "len": metadata.len,
        })))
    }
}

#[derive(Debug, Deserialize)]
struct PathInput {
    path: String,
}
