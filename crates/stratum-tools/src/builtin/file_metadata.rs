//! Builtin file-metadata tool.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use stratum_core::ToolSpec;
use stratum_filesystem::Filesystem;
use tokio_util::sync::CancellationToken;

use crate::{
    Tool, ToolError, ToolInput, ToolOutput,
    builtin::filesystem::{display_path, file_type_label, parse_path},
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

    fn validate(&self, input: &ToolInput) -> Result<(), ToolError> {
        parse_path(input.arguments.clone()).map(|_| ())
    }

    async fn call(
        &self,
        input: ToolInput,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let path = parse_path(input.arguments)?;
        if cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let metadata = self.filesystem.metadata(&path).await?;

        Ok(ToolOutput::new(json!({
            "path": display_path(&path),
            "file_type": file_type_label(metadata.file_type),
            "len": metadata.len,
        })))
    }
}
