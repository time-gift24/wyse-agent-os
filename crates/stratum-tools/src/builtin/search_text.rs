//! Builtin text-search tool.

use std::{num::NonZeroUsize, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::json;
use stratum_core::ToolSpec;
use stratum_filesystem::{FileType, Filesystem, VirtualPath};

use crate::{
    Tool, ToolError, ToolInput, ToolOutput,
    builtin::filesystem::{display_path, normalize_path},
};

const DEFAULT_MAX_RESULTS: usize = 100;

/// Builtin tool that searches text files under a virtual filesystem path.
pub struct SearchTextTool {
    filesystem: Arc<dyn Filesystem>,
    spec: ToolSpec,
}

impl SearchTextTool {
    /// Creates a text-search tool with an explicit filesystem.
    #[must_use]
    pub fn new(filesystem: Arc<dyn Filesystem>) -> Self {
        Self {
            filesystem,
            spec: ToolSpec::builder()
                .name("search_text")
                .description("searches for literal text under a file or directory")
                .input_schema(json!({
                    "type": "object",
                    "required": ["path", "query"],
                    "properties": {
                        "path": { "type": "string" },
                        "query": { "type": "string", "minLength": 1 },
                        "max_results": { "type": "integer", "minimum": 1 }
                    }
                }))
                .build(),
        }
    }
}

#[async_trait]
impl Tool for SearchTextTool {
    fn spec(&self) -> &ToolSpec {
        &self.spec
    }

    async fn call(&self, input: ToolInput) -> Result<ToolOutput, ToolError> {
        let raw: SearchTextInput = serde_json::from_value(input.arguments)
            .map_err(|source| ToolError::InvalidInput { source })?;
        if raw.query.is_empty() {
            return Err(ToolError::InvalidArgument {
                name: "query",
                reason: "must not be empty",
            });
        }

        let path = normalize_path(&raw.path)?;
        let max_results = raw
            .max_results
            .map(NonZeroUsize::get)
            .unwrap_or(DEFAULT_MAX_RESULTS);
        let matches = search_text(
            self.filesystem.as_ref(),
            path.clone(),
            &raw.query,
            max_results,
        )
        .await?;

        Ok(ToolOutput::new(json!({
            "path": display_path(&path),
            "query": raw.query,
            "matches": matches,
        })))
    }
}

#[derive(Debug, Deserialize)]
struct SearchTextInput {
    path: String,
    query: String,
    max_results: Option<NonZeroUsize>,
}

async fn search_text(
    filesystem: &dyn Filesystem,
    root: VirtualPath,
    query: &str,
    max_results: usize,
) -> Result<Vec<serde_json::Value>, ToolError> {
    let mut pending = vec![root];
    let mut matches = Vec::new();

    while let Some(path) = pending.pop() {
        if matches.len() >= max_results {
            break;
        }

        let metadata = filesystem.metadata(&path).await?;
        match metadata.file_type {
            FileType::File => {
                search_file(filesystem, &path, query, max_results, &mut matches).await?
            }
            FileType::Directory => {
                let entries = filesystem.list_dir(&path).await?;
                for entry in entries.into_iter().rev() {
                    pending.push(entry.path);
                }
            }
            FileType::Symlink | FileType::Other => {}
            _ => {}
        }
    }

    Ok(matches)
}

async fn search_file(
    filesystem: &dyn Filesystem,
    path: &VirtualPath,
    query: &str,
    max_results: usize,
    matches: &mut Vec<serde_json::Value>,
) -> Result<(), ToolError> {
    let content = filesystem.read_file(path).await?;
    let Ok(text) = String::from_utf8(content) else {
        return Ok(());
    };

    for (index, line) in text.lines().enumerate() {
        if matches.len() >= max_results {
            break;
        }
        if let Some(match_start) = line.find(query) {
            matches.push(json!({
                "path": display_path(path),
                "line_number": index + 1,
                "line_text": line,
                "match_start": match_start,
                "match_end": match_start + query.len(),
            }));
        }
    }

    Ok(())
}
