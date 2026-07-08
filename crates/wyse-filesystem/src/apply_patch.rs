//! Apply-patch operations for virtual filesystems.

use thiserror::Error;

use crate::{Filesystem, FilesystemError, VirtualPath};

/// Kind of file operation requested by apply-patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplyPatchOperationKind {
    /// Creates a new file from a diff.
    CreateFile,
    /// Updates an existing file from a diff.
    UpdateFile,
    /// Deletes an existing file.
    DeleteFile,
}

/// One apply-patch operation against a virtual path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApplyPatchOperation {
    /// Operation kind.
    pub kind: ApplyPatchOperationKind,
    /// Target virtual path.
    pub path: VirtualPath,
    /// V4A diff for create and update operations.
    pub diff: Option<String>,
}

impl ApplyPatchOperation {
    /// Creates an apply-patch operation.
    #[must_use]
    pub fn new(kind: ApplyPatchOperationKind, path: VirtualPath, diff: Option<String>) -> Self {
        Self { kind, path, diff }
    }
}

/// Result status for an apply-patch operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ApplyPatchStatus {
    /// Operation completed.
    Completed,
    /// Operation failed in a recoverable way.
    Failed,
}

/// Output for an apply-patch operation.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApplyPatchOutput {
    /// Operation status.
    pub status: ApplyPatchStatus,
    /// Short human-readable output.
    pub output: String,
}

/// Error returned while applying a patch.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ApplyPatchError {
    /// A create or update operation did not include a diff.
    #[error("patch diff is required for {operation}")]
    MissingDiff {
        /// Operation name.
        operation: &'static str,
    },
    /// Diff context did not match the current file content.
    #[error("patch context did not match {path}")]
    ContextMismatch {
        /// Target virtual path.
        path: VirtualPath,
    },
    /// Filesystem operation failed.
    #[error("filesystem operation failed")]
    Filesystem {
        /// Source filesystem error.
        #[source]
        source: FilesystemError,
    },
}

/// Applies one patch operation through a virtual filesystem.
///
/// # Errors
///
/// Returns an error when the patch is malformed, does not match file context,
/// or a filesystem operation fails.
pub async fn apply_patch(
    filesystem: &dyn Filesystem,
    operation: &ApplyPatchOperation,
) -> Result<ApplyPatchOutput, ApplyPatchError> {
    let output = match operation.kind {
        ApplyPatchOperationKind::CreateFile => {
            let diff = required_diff(operation)?;
            ensure_missing(filesystem, &operation.path).await?;
            let contents = apply_create_diff(diff);
            filesystem
                .write_file(&operation.path, contents.into_bytes())
                .await
                .map_err(|source| ApplyPatchError::Filesystem { source })?;
            format!("created {}", operation.path)
        }
        ApplyPatchOperationKind::UpdateFile => {
            let diff = required_diff(operation)?;
            let current = filesystem
                .read_file(&operation.path)
                .await
                .map_err(|source| ApplyPatchError::Filesystem { source })?;
            let current = String::from_utf8_lossy(&current);
            let updated = apply_update_diff(&current, diff, &operation.path)?;
            filesystem
                .write_file(&operation.path, updated.into_bytes())
                .await
                .map_err(|source| ApplyPatchError::Filesystem { source })?;
            format!("updated {}", operation.path)
        }
        ApplyPatchOperationKind::DeleteFile => {
            filesystem
                .remove_file(&operation.path)
                .await
                .map_err(|source| ApplyPatchError::Filesystem { source })?;
            format!("deleted {}", operation.path)
        }
    };

    Ok(ApplyPatchOutput {
        status: ApplyPatchStatus::Completed,
        output,
    })
}

fn required_diff(operation: &ApplyPatchOperation) -> Result<&str, ApplyPatchError> {
    operation
        .diff
        .as_deref()
        .ok_or_else(|| ApplyPatchError::MissingDiff {
            operation: operation_name(operation.kind),
        })
}

async fn ensure_missing(
    filesystem: &dyn Filesystem,
    path: &VirtualPath,
) -> Result<(), ApplyPatchError> {
    match filesystem.metadata(path).await {
        Ok(_) => Err(ApplyPatchError::Filesystem {
            source: FilesystemError::AlreadyExists { path: path.clone() },
        }),
        Err(FilesystemError::NotFound { .. }) => Ok(()),
        Err(source) => Err(ApplyPatchError::Filesystem { source }),
    }
}

fn operation_name(kind: ApplyPatchOperationKind) -> &'static str {
    match kind {
        ApplyPatchOperationKind::CreateFile => "create_file",
        ApplyPatchOperationKind::UpdateFile => "update_file",
        ApplyPatchOperationKind::DeleteFile => "delete_file",
    }
}

fn apply_create_diff(diff: &str) -> String {
    diff_lines(diff)
        .filter_map(|line| line.strip_prefix('+'))
        .map(line_with_newline)
        .collect()
}

fn apply_update_diff(
    current: &str,
    diff: &str,
    path: &VirtualPath,
) -> Result<String, ApplyPatchError> {
    let mut expected = Vec::new();
    let mut replacement = Vec::new();

    for line in diff_lines(diff) {
        if let Some(context) = line.strip_prefix(' ') {
            let context = line_with_newline(context);
            expected.push(context.clone());
            replacement.push(context);
        } else if let Some(removed) = line.strip_prefix('-') {
            expected.push(line_with_newline(removed));
        } else if let Some(added) = line.strip_prefix('+') {
            replacement.push(line_with_newline(added));
        }
    }

    let current_lines = split_lines(current);
    let Some(start) = find_subsequence(&current_lines, &expected) else {
        return Err(ApplyPatchError::ContextMismatch { path: path.clone() });
    };

    let mut updated = Vec::with_capacity(current_lines.len() - expected.len() + replacement.len());
    updated.extend_from_slice(&current_lines[..start]);
    updated.extend(replacement);
    updated.extend_from_slice(&current_lines[start + expected.len()..]);
    Ok(updated.concat())
}

fn diff_lines(diff: &str) -> impl Iterator<Item = &str> {
    diff.lines().filter(|line| !line.starts_with("@@"))
}

fn line_with_newline(line: &str) -> String {
    let mut owned = String::with_capacity(line.len() + 1);
    owned.push_str(line);
    owned.push('\n');
    owned
}

fn split_lines(contents: &str) -> Vec<String> {
    contents
        .split_inclusive('\n')
        .map(ToOwned::to_owned)
        .collect()
}

fn find_subsequence(haystack: &[String], needle: &[String]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Filesystem, LocalFilesystem, LocalFilesystemConfig, VirtualPath};

    fn temp_root(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("wyse-apply-patch-{name}-{}", std::process::id()))
    }

    async fn local_filesystem(name: &str) -> (LocalFilesystem, std::path::PathBuf) {
        let root = temp_root(name);
        let _ = tokio::fs::remove_dir_all(&root).await;
        tokio::fs::create_dir_all(&root).await.expect("create root");
        let filesystem = LocalFilesystem::new(LocalFilesystemConfig {
            root: root.clone(),
            max_file_bytes: Some(4096),
        })
        .expect("filesystem is valid");
        (filesystem, root)
    }

    #[tokio::test]
    async fn create_file_applies_v4a_diff_to_empty_file() {
        let (filesystem, root) = local_filesystem("create").await;
        let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::CreateFile,
            path: path.clone(),
            diff: Some("@@\n+hello\n+world\n".to_owned()),
        };

        let output = apply_patch(&filesystem, &operation)
            .await
            .expect("patch should apply");

        assert_eq!(output.status, ApplyPatchStatus::Completed);
        assert_eq!(
            filesystem.read_file(&path).await.expect("read file"),
            b"hello\nworld\n"
        );

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn update_file_applies_contextual_v4a_diff() {
        let (filesystem, root) = local_filesystem("update").await;
        let path = VirtualPath::try_from("/src.txt").expect("path is valid");
        filesystem
            .write_file(&path, b"one\ntwo\nthree\n".to_vec())
            .await
            .expect("seed file");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::UpdateFile,
            path: path.clone(),
            diff: Some("@@\n one\n-two\n+deux\n three\n".to_owned()),
        };

        let output = apply_patch(&filesystem, &operation)
            .await
            .expect("patch should apply");

        assert_eq!(output.status, ApplyPatchStatus::Completed);
        assert_eq!(
            filesystem.read_file(&path).await.expect("read file"),
            b"one\ndeux\nthree\n"
        );

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn delete_file_removes_target() {
        let (filesystem, root) = local_filesystem("delete").await;
        let path = VirtualPath::try_from("/old.txt").expect("path is valid");
        filesystem
            .write_file(&path, b"old".to_vec())
            .await
            .expect("seed file");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::DeleteFile,
            path: path.clone(),
            diff: None,
        };

        let output = apply_patch(&filesystem, &operation)
            .await
            .expect("delete should apply");

        assert_eq!(output.status, ApplyPatchStatus::Completed);
        assert!(matches!(
            filesystem
                .read_file(&path)
                .await
                .expect_err("file should be gone"),
            crate::FilesystemError::NotFound { .. }
        ));

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn update_file_rejects_context_mismatch_without_writing_content() {
        let (filesystem, root) = local_filesystem("conflict").await;
        let path = VirtualPath::try_from("/src.txt").expect("path is valid");
        filesystem
            .write_file(&path, b"one\ntwo\n".to_vec())
            .await
            .expect("seed file");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::UpdateFile,
            path: path.clone(),
            diff: Some("@@\n missing\n-two\n+deux\n".to_owned()),
        };

        let error = apply_patch(&filesystem, &operation)
            .await
            .expect_err("context should fail");

        assert!(matches!(error, ApplyPatchError::ContextMismatch { .. }));
        assert_eq!(
            filesystem.read_file(&path).await.expect("read original"),
            b"one\ntwo\n"
        );

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn create_file_requires_diff() {
        let (filesystem, root) = local_filesystem("missing-diff").await;
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::CreateFile,
            path: VirtualPath::try_from("/new.txt").expect("path is valid"),
            diff: None,
        };

        let error = apply_patch(&filesystem, &operation)
            .await
            .expect_err("diff is required");

        assert!(matches!(error, ApplyPatchError::MissingDiff { .. }));

        let _ = tokio::fs::remove_dir_all(root).await;
    }

    #[tokio::test]
    async fn create_file_rejects_existing_file_without_overwriting() {
        let (filesystem, root) = local_filesystem("create-existing").await;
        let path = VirtualPath::try_from("/notes.txt").expect("path is valid");
        filesystem
            .write_file(&path, b"original\n".to_vec())
            .await
            .expect("seed file");
        let operation = ApplyPatchOperation {
            kind: ApplyPatchOperationKind::CreateFile,
            path: path.clone(),
            diff: Some("@@\n+replacement\n".to_owned()),
        };

        let error = apply_patch(&filesystem, &operation)
            .await
            .expect_err("create should reject existing file");

        assert!(matches!(
            error,
            ApplyPatchError::Filesystem {
                source: crate::FilesystemError::AlreadyExists { .. }
            }
        ));
        assert_eq!(
            filesystem.read_file(&path).await.expect("read original"),
            b"original\n"
        );

        let _ = tokio::fs::remove_dir_all(root).await;
    }
}
