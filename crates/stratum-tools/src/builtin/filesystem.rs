//! Shared helpers for filesystem-backed builtin tools.

use stratum_filesystem::{FileType, VirtualPath};

use crate::ToolError;

pub(super) fn normalize_path(path: &str) -> Result<VirtualPath, ToolError> {
    if path.is_empty() {
        return Err(ToolError::InvalidPath {
            path: path.to_owned(),
            source: stratum_filesystem::VirtualPathError,
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

pub(super) fn display_path(path: &VirtualPath) -> String {
    let trimmed = path.as_str().trim_start_matches('/');
    if trimmed.is_empty() {
        ".".to_owned()
    } else {
        trimmed.to_owned()
    }
}

pub(super) const fn file_type_label(file_type: FileType) -> &'static str {
    match file_type {
        FileType::File => "file",
        FileType::Directory => "directory",
        FileType::Symlink => "symlink",
        FileType::Other => "other",
        _ => "other",
    }
}
