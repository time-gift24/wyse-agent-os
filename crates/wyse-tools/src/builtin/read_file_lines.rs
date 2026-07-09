//! Builtin read-file-lines tool.

use std::{num::NonZeroUsize, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use wyse_core::ToolSpec;
use wyse_filesystem::{Filesystem, VirtualPath};

use crate::{
    Tool, ToolError, ToolInput, ToolOutput,
    builtin::filesystem::{display_path, normalize_path},
};

/// Builtin tool that reads a line range from one file.
pub struct ReadFileLinesTool {
    filesystem: Arc<dyn Filesystem>,
    spec: ToolSpec,
}

impl ReadFileLinesTool {
    /// Creates a read-file-lines tool with an explicit filesystem.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self {
            filesystem,
            spec: ToolSpec::builder()
                .name("read_file_lines")
                .description(
                    "reads a one-based line range from a file inside the virtual filesystem",
                )
                .input_schema(json!({
                    "type": "object",
                    "required": ["path", "start_line", "line_count"],
                    "properties": {
                        "path": { "type": "string" },
                        "start_line": { "type": "integer", "minimum": 1 },
                        "line_count": { "type": "integer", "minimum": 1 }
                    }
                }))
                .build(),
        }
    }
}

#[async_trait]
impl Tool for ReadFileLinesTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let raw: ReadFileLinesInput = serde_json::from_value(input.arguments)
            .map_err(|source| ToolError::InvalidInput { source })?;
        let path = normalize_path(&raw.path)?;
        let content = self.filesystem.read_file(&path).await?;
        let text = String::from_utf8(content).map_err(|source| ToolError::InvalidUtf8 {
            path: display_path(&path),
            source,
        })?;
        let result = line_range_output(&path, &text, raw.start_line.get(), raw.line_count.get());

        Ok(ToolOutput::new(result))
    }
}

#[derive(Debug, Deserialize)]
struct ReadFileLinesInput {
    path: String,
    start_line: NonZeroUsize,
    line_count: NonZeroUsize,
}

fn line_range_output(
    path: &VirtualPath,
    text: &str,
    start_line: usize,
    line_count: usize,
) -> Value {
    let end_exclusive = start_line.saturating_add(line_count);
    let mut total_lines = 0usize;
    let mut end_line = None;
    let mut lines = Vec::new();

    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        total_lines = line_number;
        if line_number >= start_line && line_number < end_exclusive {
            end_line = Some(line_number);
            lines.push(json!({
                "line_number": line_number,
                "text": line,
            }));
        }
    }

    json!({
        "path": display_path(path),
        "start_line": start_line,
        "end_line": end_line,
        "total_lines": total_lines,
        "lines": lines,
    })
}
