//! Builtin text-search tool.

use std::{num::NonZeroUsize, sync::Arc};

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};
use stratum_core::ToolSpec;
use stratum_filesystem::{FileType, Filesystem, VirtualPath};
use tokio_util::sync::CancellationToken;

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

    fn validate(&self, input: &ToolInput) -> Result<(), ToolError> {
        parse_input(input.arguments.clone()).map(|_| ())
    }

    async fn call(
        &self,
        input: ToolInput,
        cancellation: &CancellationToken,
    ) -> Result<ToolOutput, ToolError> {
        let (raw, path, max_results) = parse_input(input.arguments)?;
        if cancellation.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        let matches = search_text(
            self.filesystem.as_ref(),
            path.clone(),
            &raw.query,
            max_results,
            cancellation,
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

fn parse_input(arguments: Value) -> Result<(SearchTextInput, VirtualPath, usize), ToolError> {
    let raw: SearchTextInput =
        serde_json::from_value(arguments).map_err(|source| ToolError::InvalidInput { source })?;
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
    Ok((raw, path, max_results))
}

async fn search_text(
    filesystem: &dyn Filesystem,
    root: VirtualPath,
    query: &str,
    max_results: usize,
    cancellation: &CancellationToken,
) -> Result<Vec<serde_json::Value>, ToolError> {
    let mut pending = vec![root];
    let mut matches = Vec::new();

    while let Some(path) = pending.pop() {
        ensure_not_cancelled(cancellation)?;
        if matches.len() >= max_results {
            break;
        }

        let metadata = cancellation
            .run_until_cancelled(filesystem.metadata(&path))
            .await
            .ok_or(ToolError::Cancelled)??;
        ensure_not_cancelled(cancellation)?;
        match metadata.file_type {
            FileType::File => {
                search_file(
                    filesystem,
                    &path,
                    query,
                    max_results,
                    &mut matches,
                    cancellation,
                )
                .await?
            }
            FileType::Directory => {
                let entries = cancellation
                    .run_until_cancelled(filesystem.list_dir(&path))
                    .await
                    .ok_or(ToolError::Cancelled)??;
                ensure_not_cancelled(cancellation)?;
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
    cancellation: &CancellationToken,
) -> Result<(), ToolError> {
    let content = cancellation
        .run_until_cancelled(filesystem.read_file(path))
        .await
        .ok_or(ToolError::Cancelled)??;
    ensure_not_cancelled(cancellation)?;
    let Ok(text) = String::from_utf8(content) else {
        return Ok(());
    };

    for (index, line) in text.lines().enumerate() {
        ensure_not_cancelled(cancellation)?;
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

fn ensure_not_cancelled(cancellation: &CancellationToken) -> Result<(), ToolError> {
    if cancellation.is_cancelled() {
        Err(ToolError::Cancelled)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{future::pending, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use stratum_core::CallId;
    use stratum_filesystem::{
        CasExpectation, DirEntry, Entry, FileMetadata, FilesystemError, LocalFilesystem,
        LocalFilesystemConfig, RecordVersion, VersionedEntry,
    };

    use super::*;

    struct CancellingFilesystem {
        inner: Arc<LocalFilesystem>,
        cancellation: CancellationToken,
        block_metadata: bool,
    }

    #[async_trait]
    impl Filesystem for CancellingFilesystem {
        async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
            self.inner.get(path).await
        }

        async fn put(
            &self,
            path: &VirtualPath,
            entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            self.inner.put(path, entry, cas).await
        }

        async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            self.inner.read_file(path).await
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            contents: Vec<u8>,
        ) -> Result<(), FilesystemError> {
            self.inner.write_file(path, contents).await
        }

        async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            self.inner.list_dir(path).await
        }

        async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
            if self.block_metadata {
                return pending().await;
            }
            let metadata = self.inner.metadata(path).await?;
            self.cancellation.cancel();
            Ok(metadata)
        }

        async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.create_dir(path).await
        }

        async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.remove_file(path).await
        }

        async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
            self.inner.remove_dir(path).await
        }
    }

    #[tokio::test]
    async fn cancellation_after_started_filesystem_work_stops_the_search() {
        let root =
            std::env::temp_dir().join(format!("stratum-search-text-cancel-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root)
            .await
            .expect("create test root");
        tokio::fs::create_dir(root.join("src"))
            .await
            .expect("create search directory");
        let inner = Arc::new(
            LocalFilesystem::new(LocalFilesystemConfig {
                root: root.clone(),
                max_file_bytes: Some(4096),
            })
            .expect("filesystem should be valid"),
        );
        let cancellation = CancellationToken::new();
        let filesystem: Arc<dyn Filesystem> = Arc::new(CancellingFilesystem {
            inner,
            cancellation: cancellation.clone(),
            block_metadata: false,
        });
        let tool = SearchTextTool::new(filesystem);

        let error = tool
            .call(
                ToolInput::new(
                    CallId::from("call-1"),
                    json!({"path": "src", "query": "needle"}),
                ),
                &cancellation,
            )
            .await
            .expect_err("mid-search cancellation should stop traversal");

        assert!(matches!(error, ToolError::Cancelled));
        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn cancellation_interrupts_pending_filesystem_work() {
        let root = std::env::temp_dir().join(format!(
            "stratum-search-text-pending-cancel-{}",
            std::process::id()
        ));
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root)
            .await
            .expect("create test root");
        let inner = Arc::new(
            LocalFilesystem::new(LocalFilesystemConfig {
                root: root.clone(),
                max_file_bytes: Some(4096),
            })
            .expect("filesystem should be valid"),
        );
        let cancellation = CancellationToken::new();
        let filesystem: Arc<dyn Filesystem> = Arc::new(CancellingFilesystem {
            inner,
            cancellation: cancellation.clone(),
            block_metadata: true,
        });
        let tool = SearchTextTool::new(filesystem);
        let cancellation_task = cancellation.clone();
        tokio::spawn(async move {
            tokio::task::yield_now().await;
            cancellation_task.cancel();
        });

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            tool.call(
                ToolInput::new(
                    CallId::from("call-1"),
                    json!({"path": "src", "query": "needle"}),
                ),
                &cancellation,
            ),
        )
        .await
        .expect("cancellation should interrupt pending filesystem work")
        .expect_err("cancelled search should fail");

        assert!(matches!(error, ToolError::Cancelled));
        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
