# Wyse Filesystem Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `wyse-filesystem`, an async virtual filesystem crate with `VirtualPath`, whole-file IO, a local sandbox backend, and Codex-style patch application.

**Architecture:** The crate exposes one `Filesystem` trait over safe virtual paths. Backends implement minimal file primitives; `apply_patch` is a default method implemented once using those primitives. `LocalFilesystem` maps `/...` paths into one sandbox root and rejects escapes.

**Tech Stack:** Rust 2024, Tokio `fs`, `bytes::Bytes`, `thiserror`, workspace dependency inheritance, native async trait methods.

## Global Constraints

- Use Cargo workspace dependency inheritance.
- Public file APIs accept `VirtualPath`, not raw strings or host paths.
- `VirtualPath` accepts only `/...` virtual absolute paths and rejects relative paths, `..`, empty segments, backslashes, Windows drive prefixes, and NUL bytes.
- No stream read/write in this implementation.
- No mount router, registry, factory, manager, read-only policy, glob/search, watch, snapshot, remote backend, or object storage backend.
- `remove_dir` removes empty directories only.
- `apply_patch` is a `Filesystem` default method using minimal file primitives.
- Errors must not expose host paths, sandbox root, file contents, or patch contents.
- Run `cargo fmt`, `cargo test --workspace --all-targets`, and `cargo clippy --workspace --all-targets` before completion.
- After implementation, remind the user to archive final filesystem conventions in crate `AGENTS.md` before PR merge.

---

## File Structure

- Create `crates/wyse-filesystem/Cargo.toml`: crate manifest using workspace package fields and dependencies.
- Create `crates/wyse-filesystem/AGENTS.md`: crate-specific rules for virtual paths, sandbox safety, and patch scope.
- Create `crates/wyse-filesystem/src/lib.rs`: crate docs and public re-exports.
- Create `crates/wyse-filesystem/src/path.rs`: `VirtualPath` newtype and validation.
- Create `crates/wyse-filesystem/src/error.rs`: `FilesystemError`.
- Create `crates/wyse-filesystem/src/definition.rs`: `Filesystem`, metadata types, default `apply_patch`.
- Create `crates/wyse-filesystem/src/local.rs`: `LocalFilesystem` and sandbox mapping.
- Create `crates/wyse-filesystem/src/patch.rs`: Codex-style patch parser and default applier.
- Modify `Cargo.toml`: add `crates/wyse-filesystem` to workspace members. Do not add a `wyse-filesystem` workspace dependency because no existing crate consumes it in this plan.

---

### Task 1: Crate Skeleton And VirtualPath

**Files:**
- Modify: `Cargo.toml`
- Create: `crates/wyse-filesystem/Cargo.toml`
- Create: `crates/wyse-filesystem/src/lib.rs`
- Create: `crates/wyse-filesystem/src/path.rs`

**Interfaces:**
- Produces: `pub struct VirtualPath(String)`
- Produces: `impl VirtualPath { pub fn as_str(&self) -> &str; pub(crate) fn segments(&self) -> impl Iterator<Item = &str>; }`
- Produces: `impl TryFrom<&str> for VirtualPath`
- Produces: `impl FromStr for VirtualPath`

- [ ] **Step 1: Add crate to workspace**

Modify root `Cargo.toml`:

```toml
[workspace]
members = [
    "crates/wyse-core",
    "crates/wyse-filesystem",
    "crates/wyse-infra",
    "crates/wyse-llm",
]
resolver = "3"
```

- [ ] **Step 2: Create crate manifest**

Create `crates/wyse-filesystem/Cargo.toml`:

```toml
[package]
name = "wyse-filesystem"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[dependencies]
bytes.workspace = true
thiserror.workspace = true
tokio = { workspace = true, features = ["fs", "io-util"] }

[lints]
workspace = true
```

- [ ] **Step 3: Write failing VirtualPath tests**

Create `crates/wyse-filesystem/src/path.rs` with tests first:

```rust
//! Virtual path handling for agent-visible filesystem paths.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_root_and_virtual_absolute_paths() {
        assert_eq!(VirtualPath::try_from("/").expect("root is valid").as_str(), "/");
        assert_eq!(
            VirtualPath::try_from("/src/lib.rs")
                .expect("absolute virtual path is valid")
                .as_str(),
            "/src/lib.rs"
        );
    }

    #[test]
    fn rejects_paths_that_are_not_safe_virtual_absolutes() {
        for value in [
            "",
            "src/lib.rs",
            "../secret",
            "/../secret",
            "/src/../secret",
            "/src//lib.rs",
            r"/src\\lib.rs",
            "C:/Users/me/file.txt",
            "/has\0nul",
        ] {
            assert!(VirtualPath::try_from(value).is_err(), "{value:?} should be rejected");
        }
    }

    #[test]
    fn exposes_validated_segments_without_root_marker() {
        let path = VirtualPath::try_from("/src/lib.rs").expect("path is valid");
        let segments = path.segments().collect::<Vec<_>>();
        assert_eq!(segments, ["src", "lib.rs"]);
    }
}
```

- [ ] **Step 4: Run test to verify it fails**

Run:

```bash
cargo test -p wyse-filesystem path::tests::accepts_root_and_virtual_absolute_paths
```

Expected: FAIL because `VirtualPath` is not defined.

- [ ] **Step 5: Implement minimal VirtualPath**

Replace `crates/wyse-filesystem/src/path.rs` with:

```rust
//! Virtual path handling for agent-visible filesystem paths.

use std::{fmt, str::FromStr};

/// Agent-visible absolute path inside a virtual filesystem.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VirtualPath(String);

impl VirtualPath {
    /// Returns the original virtual path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn segments(&self) -> impl Iterator<Item = &str> {
        self.0.trim_start_matches('/').split('/').filter(|segment| !segment.is_empty())
    }
}

impl TryFrom<&str> for VirtualPath {
    type Error = VirtualPathError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl FromStr for VirtualPath {
    type Err = VirtualPathError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl fmt::Display for VirtualPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Error returned when parsing an invalid [`VirtualPath`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VirtualPathError;

impl fmt::Display for VirtualPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("invalid virtual path")
    }
}

impl std::error::Error for VirtualPathError {}

fn validate(value: &str) -> Result<(), VirtualPathError> {
    if value.is_empty()
        || !value.starts_with('/')
        || value.contains('\\')
        || value.contains('\0')
        || looks_like_windows_drive(value)
    {
        return Err(VirtualPathError);
    }

    if value == "/" {
        return Ok(());
    }

    for segment in value.trim_start_matches('/').split('/') {
        if segment.is_empty() || segment == ".." {
            return Err(VirtualPathError);
        }
    }

    Ok(())
}

fn looks_like_windows_drive(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_root_and_virtual_absolute_paths() {
        assert_eq!(VirtualPath::try_from("/").expect("root is valid").as_str(), "/");
        assert_eq!(
            VirtualPath::try_from("/src/lib.rs")
                .expect("absolute virtual path is valid")
                .as_str(),
            "/src/lib.rs"
        );
    }

    #[test]
    fn rejects_paths_that_are_not_safe_virtual_absolutes() {
        for value in [
            "",
            "src/lib.rs",
            "../secret",
            "/../secret",
            "/src/../secret",
            "/src//lib.rs",
            r"/src\\lib.rs",
            "C:/Users/me/file.txt",
            "/has\0nul",
        ] {
            assert!(VirtualPath::try_from(value).is_err(), "{value:?} should be rejected");
        }
    }

    #[test]
    fn exposes_validated_segments_without_root_marker() {
        let path = VirtualPath::try_from("/src/lib.rs").expect("path is valid");
        let segments = path.segments().collect::<Vec<_>>();
        assert_eq!(segments, ["src", "lib.rs"]);
    }
}
```

- [ ] **Step 6: Add crate root exports**

Create `crates/wyse-filesystem/src/lib.rs`:

```rust
//! Virtual filesystem abstractions and local sandbox backend for Wyse agents.

pub mod path;

pub use path::{VirtualPath, VirtualPathError};
```

- [ ] **Step 7: Verify Task 1**

Run:

```bash
cargo test -p wyse-filesystem path::tests
cargo fmt
```

Expected: all `path::tests` pass and formatting succeeds.

- [ ] **Step 8: Commit Task 1**

```bash
git add Cargo.toml crates/wyse-filesystem
git commit -m "feat: add filesystem virtual paths"
```

---

### Task 2: Public Trait, Metadata, And Errors

**Files:**
- Create: `crates/wyse-filesystem/src/error.rs`
- Create: `crates/wyse-filesystem/src/definition.rs`
- Modify: `crates/wyse-filesystem/src/lib.rs`
- Modify: `crates/wyse-filesystem/src/path.rs`

**Interfaces:**
- Consumes: `VirtualPath`
- Produces: `pub enum FilesystemError`
- Produces: `pub trait Filesystem`
- Produces: `pub struct FileMetadata`
- Produces: `pub struct DirEntry`
- Produces: `pub enum FileType`

- [ ] **Step 1: Write failing metadata and error tests**

Create `crates/wyse-filesystem/src/error.rs`:

```rust
//! Error types for virtual filesystem operations.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VirtualPath;

    #[test]
    fn invalid_path_error_does_not_include_host_paths() {
        let source = crate::VirtualPathError;
        let error = FilesystemError::invalid_virtual_path("/tmp/secret", source);
        let text = error.to_string();

        assert_eq!(text, "invalid virtual path /tmp/secret");
        assert!(!format!("{error:?}").contains("/Users/"));
    }

    #[test]
    fn not_found_mentions_only_virtual_path() {
        let path = VirtualPath::try_from("/missing.txt").expect("path is valid");
        let error = FilesystemError::not_found(path);

        assert_eq!(error.to_string(), "path not found /missing.txt");
    }
}
```

Create `crates/wyse-filesystem/src/definition.rs`:

```rust
//! Public filesystem trait and metadata types.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_type_helpers_match_variants() {
        assert!(FileType::File.is_file());
        assert!(FileType::Directory.is_dir());
        assert!(!FileType::Symlink.is_file());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p wyse-filesystem error::tests definition::tests
```

Expected: FAIL because `FilesystemError` and metadata types are not defined.

- [ ] **Step 3: Implement errors**

Replace `crates/wyse-filesystem/src/error.rs` with:

```rust
//! Error types for virtual filesystem operations.

use std::io;

use thiserror::Error;

use crate::{VirtualPath, VirtualPathError};

/// Error returned by virtual filesystem operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FilesystemError {
    /// A virtual path failed validation.
    #[error("invalid virtual path {path}")]
    InvalidVirtualPath {
        /// Rejected path text.
        path: String,
        /// Validation failure source.
        #[source]
        source: VirtualPathError,
    },
    /// Path resolution would escape the sandbox.
    #[error("path escapes sandbox {path}")]
    PathEscapesSandbox {
        /// Virtual path that escaped.
        path: VirtualPath,
    },
    /// The requested path does not exist.
    #[error("path not found {path}")]
    NotFound {
        /// Missing virtual path.
        path: VirtualPath,
    },
    /// The requested path already exists.
    #[error("path already exists {path}")]
    AlreadyExists {
        /// Existing virtual path.
        path: VirtualPath,
    },
    /// The path is not a file.
    #[error("path is not a file {path}")]
    NotAFile {
        /// Virtual path.
        path: VirtualPath,
    },
    /// The path is not a directory.
    #[error("path is not a directory {path}")]
    NotADirectory {
        /// Virtual path.
        path: VirtualPath,
    },
    /// The directory is not empty.
    #[error("directory is not empty {path}")]
    DirectoryNotEmpty {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Operation was denied by the operating system.
    #[error("permission denied {path}")]
    PermissionDenied {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Content exceeds the configured size limit.
    #[error("content too large {path}")]
    ContentTooLarge {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Patch content is not supported as text.
    #[error("unsupported binary content for patch {path}")]
    UnsupportedBinaryContent {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Patch text failed to parse.
    #[error("patch parse error")]
    PatchParse,
    /// Patch context did not match file content.
    #[error("patch conflict {path}")]
    PatchConflict {
        /// Virtual path.
        path: VirtualPath,
    },
    /// Local filesystem operation failed.
    #[error("local filesystem operation failed {operation} {path}")]
    LocalIo {
        /// Safe operation name.
        operation: &'static str,
        /// Virtual path.
        path: VirtualPath,
        /// Source IO error.
        #[source]
        source: io::Error,
    },
}

impl FilesystemError {
    pub(crate) fn invalid_virtual_path(path: impl Into<String>, source: VirtualPathError) -> Self {
        Self::InvalidVirtualPath {
            path: path.into(),
            source,
        }
    }

    pub(crate) fn not_found(path: VirtualPath) -> Self {
        Self::NotFound { path }
    }

    pub(crate) fn local_io(operation: &'static str, path: VirtualPath, source: io::Error) -> Self {
        match source.kind() {
            io::ErrorKind::NotFound => Self::NotFound { path },
            io::ErrorKind::AlreadyExists => Self::AlreadyExists { path },
            io::ErrorKind::PermissionDenied => Self::PermissionDenied { path },
            io::ErrorKind::DirectoryNotEmpty => Self::DirectoryNotEmpty { path },
            _ => Self::LocalIo {
                operation,
                path,
                source,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VirtualPath;

    #[test]
    fn invalid_path_error_does_not_include_host_paths() {
        let source = crate::VirtualPathError;
        let error = FilesystemError::invalid_virtual_path("/tmp/secret", source);
        let text = error.to_string();

        assert_eq!(text, "invalid virtual path /tmp/secret");
        assert!(!format!("{error:?}").contains("/Users/"));
    }

    #[test]
    fn not_found_mentions_only_virtual_path() {
        let path = VirtualPath::try_from("/missing.txt").expect("path is valid");
        let error = FilesystemError::not_found(path);

        assert_eq!(error.to_string(), "path not found /missing.txt");
    }
}
```

- [ ] **Step 4: Implement trait and metadata types**

Replace `crates/wyse-filesystem/src/definition.rs` with:

```rust
//! Public filesystem trait and metadata types.

use bytes::Bytes;

use crate::{patch::apply_patch_using_filesystem, FilesystemError, Patch, PatchApplyReport, VirtualPath};

/// Agent-visible filesystem operations.
// Native async trait methods are intentional for this crate's IO boundary.
#[allow(async_fn_in_trait)]
pub trait Filesystem: Send + Sync {
    /// Reads a complete file into memory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a file, too large, or the backend fails.
    async fn read_file(&self, path: &VirtualPath) -> Result<Bytes, FilesystemError>;

    /// Writes complete file contents.
    ///
    /// # Errors
    ///
    /// Returns an error when the path escapes the backend, cannot be written, or exceeds limits.
    async fn write_file(&self, path: &VirtualPath, contents: Bytes) -> Result<(), FilesystemError>;

    /// Lists one directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a directory, or the backend fails.
    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError>;

    /// Returns metadata for a path.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing or the backend fails.
    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError>;

    /// Creates one directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be created or its parent is missing.
    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError>;

    /// Removes one file.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a file, or cannot be removed.
    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError>;

    /// Removes one empty directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is missing, not a directory, not empty, or cannot be removed.
    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError>;

    /// Applies a Codex-style patch using this filesystem's primitive operations.
    ///
    /// # Errors
    ///
    /// Returns an error when the patch conflicts, references invalid state, or IO fails.
    async fn apply_patch(&self, patch: &Patch) -> Result<PatchApplyReport, FilesystemError> {
        apply_patch_using_filesystem(self, patch).await
    }
}

/// Metadata for one filesystem path.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FileMetadata {
    /// Path type.
    pub file_type: FileType,
    /// File length in bytes when known.
    pub len: Option<u64>,
}

/// Directory entry returned by [`Filesystem::list_dir`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct DirEntry {
    /// Entry path.
    pub path: VirtualPath,
    /// Final path segment.
    pub file_name: String,
    /// Entry type.
    pub file_type: FileType,
    /// Entry metadata when available.
    pub metadata: Option<FileMetadata>,
}

/// Type of one filesystem path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FileType {
    /// Regular file.
    File,
    /// Directory.
    Directory,
    /// Symbolic link.
    Symlink,
    /// Other platform-specific file type.
    Other,
}

impl FileType {
    /// Returns whether this type is a regular file.
    #[must_use]
    pub const fn is_file(self) -> bool {
        matches!(self, Self::File)
    }

    /// Returns whether this type is a directory.
    #[must_use]
    pub const fn is_dir(self) -> bool {
        matches!(self, Self::Directory)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_type_helpers_match_variants() {
        assert!(FileType::File.is_file());
        assert!(FileType::Directory.is_dir());
        assert!(!FileType::Symlink.is_file());
    }
}
```

- [ ] **Step 5: Add patch types and exports**

Create `crates/wyse-filesystem/src/patch.rs`:

```rust
//! Codex-style patch parsing and application.

use crate::{Filesystem, FilesystemError};

/// Parsed Codex-style patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    operations: Vec<PatchOperation>,
}

/// Summary of paths changed by a patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PatchApplyReport {
    /// Added files.
    pub added: Vec<crate::VirtualPath>,
    /// Updated files.
    pub updated: Vec<crate::VirtualPath>,
    /// Deleted files.
    pub deleted: Vec<crate::VirtualPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PatchOperation {}

pub(crate) async fn apply_patch_using_filesystem<F>(
    _filesystem: &F,
    _patch: &Patch,
) -> Result<PatchApplyReport, FilesystemError>
where
    F: Filesystem + ?Sized,
{
    Ok(PatchApplyReport::default())
}
```

Modify `crates/wyse-filesystem/src/lib.rs`:

```rust
//! Virtual filesystem abstractions and local sandbox backend for Wyse agents.

pub mod definition;
pub mod error;
pub mod patch;
pub mod path;

pub use definition::{DirEntry, FileMetadata, FileType, Filesystem};
pub use error::FilesystemError;
pub use patch::{Patch, PatchApplyReport};
pub use path::{VirtualPath, VirtualPathError};
```

- [ ] **Step 6: Verify Task 2**

Run:

```bash
cargo test -p wyse-filesystem error::tests definition::tests
cargo fmt
```

Expected: selected tests pass and formatting succeeds.

- [ ] **Step 7: Commit Task 2**

```bash
git add crates/wyse-filesystem
git commit -m "feat: define filesystem trait"
```

---

### Task 3: Local Sandbox Backend

**Files:**
- Create: `crates/wyse-filesystem/src/local.rs`
- Modify: `crates/wyse-filesystem/src/lib.rs`
- Modify: `crates/wyse-filesystem/src/error.rs`
- Modify: `crates/wyse-filesystem/src/definition.rs`

**Interfaces:**
- Consumes: `Filesystem`, `VirtualPath`, `FilesystemError`
- Produces: `pub struct LocalFilesystem`
- Produces: `pub struct LocalFilesystemConfig { pub root: PathBuf, pub max_file_bytes: Option<u64> }`
- Produces: `impl LocalFilesystem { pub fn new(config: LocalFilesystemConfig) -> Result<Self, FilesystemError>; }`

- [ ] **Step 1: Write failing local backend tests**

Create `crates/wyse-filesystem/src/local.rs`:

```rust
//! Local sandbox filesystem backend.

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::{Filesystem, VirtualPath};

    #[tokio::test]
    async fn reads_writes_lists_and_removes_inside_sandbox() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-test-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp).await.expect("create temp root");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");

        let dir = VirtualPath::try_from("/src").expect("path is valid");
        let file = VirtualPath::try_from("/src/lib.rs").expect("path is valid");

        fs.create_dir(&dir).await.expect("create dir");
        fs.write_file(&file, Bytes::from_static(b"pub fn ok() {}\n"))
            .await
            .expect("write file");

        let content = fs.read_file(&file).await.expect("read file");
        assert_eq!(content, Bytes::from_static(b"pub fn ok() {}\n"));

        let entries = fs.list_dir(&dir).await.expect("list dir");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, file);

        fs.remove_file(&file).await.expect("remove file");
        fs.remove_dir(&dir).await.expect("remove empty dir");

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn remove_dir_rejects_non_empty_directory() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-non-empty-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(temp.join("dir")).await.expect("create dir");
        tokio::fs::write(temp.join("dir/file.txt"), b"x").await.expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let dir = VirtualPath::try_from("/dir").expect("path is valid");

        let error = fs.remove_dir(&dir).await.expect_err("directory is not empty");
        assert!(matches!(error, crate::FilesystemError::DirectoryNotEmpty { .. }));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn rejects_reads_larger_than_limit() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-limit-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp).await.expect("create temp root");
        tokio::fs::write(temp.join("big.txt"), b"12345").await.expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(4),
        })
        .expect("filesystem is valid");
        let file = VirtualPath::try_from("/big.txt").expect("path is valid");

        let error = fs.read_file(&file).await.expect_err("content is too large");
        assert!(matches!(error, crate::FilesystemError::ContentTooLarge { .. }));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p wyse-filesystem local::tests
```

Expected: FAIL because `LocalFilesystem` is not implemented.

- [ ] **Step 3: Implement local backend**

Replace `crates/wyse-filesystem/src/local.rs` with:

```rust
//! Local sandbox filesystem backend.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use tokio::fs;

use crate::{
    DirEntry, FileMetadata, FileType, Filesystem, FilesystemError, VirtualPath,
};

/// Configuration for [`LocalFilesystem`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalFilesystemConfig {
    /// Host root directory for this sandbox.
    pub root: PathBuf,
    /// Maximum bytes allowed for one whole-file read or write.
    pub max_file_bytes: Option<u64>,
}

/// Local filesystem backend rooted at one sandbox directory.
#[derive(Debug, Clone)]
pub struct LocalFilesystem {
    root: PathBuf,
    max_file_bytes: Option<u64>,
}

impl LocalFilesystem {
    /// Creates a local sandbox filesystem.
    ///
    /// # Errors
    ///
    /// Returns an error when the root cannot be canonicalized.
    pub fn new(config: LocalFilesystemConfig) -> Result<Self, FilesystemError> {
        let root = config
            .root
            .canonicalize()
            .map_err(|source| FilesystemError::LocalIo {
                operation: "canonicalize_root",
                path: VirtualPath::try_from("/").expect("root virtual path is valid"),
                source,
            })?;

        Ok(Self {
            root,
            max_file_bytes: config.max_file_bytes,
        })
    }

    fn host_path(&self, path: &VirtualPath) -> Result<PathBuf, FilesystemError> {
        let mut host = self.root.clone();
        for segment in path.segments() {
            host.push(segment);
        }
        Ok(host)
    }

    async fn ensure_parent_inside_root(&self, path: &VirtualPath) -> Result<PathBuf, FilesystemError> {
        let host = self.host_path(path)?;
        let parent = host.parent().unwrap_or(&self.root);
        let canonical_parent = fs::canonicalize(parent)
            .await
            .map_err(|source| FilesystemError::local_io("canonicalize_parent", path.clone(), source))?;
        if !canonical_parent.starts_with(&self.root) {
            return Err(FilesystemError::PathEscapesSandbox { path: path.clone() });
        }
        Ok(host)
    }

    async fn ensure_existing_inside_root(&self, path: &VirtualPath) -> Result<PathBuf, FilesystemError> {
        let host = self.host_path(path)?;
        let canonical = fs::canonicalize(&host)
            .await
            .map_err(|source| FilesystemError::local_io("canonicalize", path.clone(), source))?;
        if !canonical.starts_with(&self.root) {
            return Err(FilesystemError::PathEscapesSandbox { path: path.clone() });
        }
        Ok(canonical)
    }

    fn check_len(&self, path: &VirtualPath, len: u64) -> Result<(), FilesystemError> {
        if self.max_file_bytes.is_some_and(|max| len > max) {
            return Err(FilesystemError::ContentTooLarge { path: path.clone() });
        }
        Ok(())
    }
}

impl Filesystem for LocalFilesystem {
    async fn read_file(&self, path: &VirtualPath) -> Result<Bytes, FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        let metadata = fs::metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        if !metadata.is_file() {
            return Err(FilesystemError::NotAFile { path: path.clone() });
        }
        self.check_len(path, metadata.len())?;
        let content = fs::read(&host)
            .await
            .map_err(|source| FilesystemError::local_io("read", path.clone(), source))?;
        Ok(Bytes::from(content))
    }

    async fn write_file(&self, path: &VirtualPath, contents: Bytes) -> Result<(), FilesystemError> {
        self.check_len(path, u64::try_from(contents.len()).unwrap_or(u64::MAX))?;
        let host = self.ensure_parent_inside_root(path).await?;
        fs::write(&host, contents)
            .await
            .map_err(|source| FilesystemError::local_io("write", path.clone(), source))
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        let metadata = fs::metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        if !metadata.is_dir() {
            return Err(FilesystemError::NotADirectory { path: path.clone() });
        }

        let mut read_dir = fs::read_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("read_dir", path.clone(), source))?;
        let mut entries = Vec::new();
        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|source| FilesystemError::local_io("read_dir_entry", path.clone(), source))?
        {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let child_path = child_virtual_path(path, &file_name);
            let entry_metadata = entry
                .metadata()
                .await
                .map_err(|source| FilesystemError::local_io("entry_metadata", child_path.clone(), source))?;
            let file_type = file_type_from_metadata(&entry_metadata);
            entries.push(DirEntry {
                path: child_path,
                file_name,
                file_type,
                metadata: Some(FileMetadata {
                    file_type,
                    len: entry_metadata.is_file().then_some(entry_metadata.len()),
                }),
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(entries)
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        let metadata = fs::metadata(&host)
            .await
            .map_err(|source| FilesystemError::local_io("metadata", path.clone(), source))?;
        let file_type = file_type_from_metadata(&metadata);
        Ok(FileMetadata {
            file_type,
            len: metadata.is_file().then_some(metadata.len()),
        })
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let host = self.ensure_parent_inside_root(path).await?;
        fs::create_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("create_dir", path.clone(), source))
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        fs::remove_file(&host)
            .await
            .map_err(|source| FilesystemError::local_io("remove_file", path.clone(), source))
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        let host = self.ensure_existing_inside_root(path).await?;
        fs::remove_dir(&host)
            .await
            .map_err(|source| FilesystemError::local_io("remove_dir", path.clone(), source))
    }
}

fn child_virtual_path(parent: &VirtualPath, file_name: &str) -> VirtualPath {
    let path = if parent.as_str() == "/" {
        format!("/{file_name}")
    } else {
        format!("{}/{file_name}", parent.as_str())
    };
    VirtualPath::try_from(path.as_str()).expect("child from validated path and directory entry is valid")
}

fn file_type_from_metadata(metadata: &std::fs::Metadata) -> FileType {
    if metadata.is_file() {
        FileType::File
    } else if metadata.is_dir() {
        FileType::Directory
    } else if metadata.file_type().is_symlink() {
        FileType::Symlink
    } else {
        FileType::Other
    }
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;

    use super::*;
    use crate::{Filesystem, VirtualPath};

    #[tokio::test]
    async fn reads_writes_lists_and_removes_inside_sandbox() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-test-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp).await.expect("create temp root");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");

        let dir = VirtualPath::try_from("/src").expect("path is valid");
        let file = VirtualPath::try_from("/src/lib.rs").expect("path is valid");

        fs.create_dir(&dir).await.expect("create dir");
        fs.write_file(&file, Bytes::from_static(b"pub fn ok() {}\n"))
            .await
            .expect("write file");

        let content = fs.read_file(&file).await.expect("read file");
        assert_eq!(content, Bytes::from_static(b"pub fn ok() {}\n"));

        let entries = fs.list_dir(&dir).await.expect("list dir");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].path, file);

        fs.remove_file(&file).await.expect("remove file");
        fs.remove_dir(&dir).await.expect("remove empty dir");

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn remove_dir_rejects_non_empty_directory() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-non-empty-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(temp.join("dir")).await.expect("create dir");
        tokio::fs::write(temp.join("dir/file.txt"), b"x").await.expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(1024),
        })
        .expect("filesystem is valid");
        let dir = VirtualPath::try_from("/dir").expect("path is valid");

        let error = fs.remove_dir(&dir).await.expect_err("directory is not empty");
        assert!(matches!(error, crate::FilesystemError::DirectoryNotEmpty { .. }));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }

    #[tokio::test]
    async fn rejects_reads_larger_than_limit() {
        let temp = std::env::temp_dir().join(format!("wyse-fs-limit-{}", std::process::id()));
        let _ = tokio::fs::remove_dir_all(&temp).await;
        tokio::fs::create_dir_all(&temp).await.expect("create temp root");
        tokio::fs::write(temp.join("big.txt"), b"12345").await.expect("write file");

        let fs = LocalFilesystem::new(LocalFilesystemConfig {
            root: temp.clone(),
            max_file_bytes: Some(4),
        })
        .expect("filesystem is valid");
        let file = VirtualPath::try_from("/big.txt").expect("path is valid");

        let error = fs.read_file(&file).await.expect_err("content is too large");
        assert!(matches!(error, crate::FilesystemError::ContentTooLarge { .. }));

        let _ = tokio::fs::remove_dir_all(&temp).await;
    }
}
```

- [ ] **Step 4: Export local backend**

Modify `crates/wyse-filesystem/src/lib.rs`:

```rust
//! Virtual filesystem abstractions and local sandbox backend for Wyse agents.

pub mod definition;
pub mod error;
pub mod local;
pub mod patch;
pub mod path;

pub use definition::{DirEntry, FileMetadata, FileType, Filesystem};
pub use error::FilesystemError;
pub use local::{LocalFilesystem, LocalFilesystemConfig};
pub use patch::{Patch, PatchApplyReport};
pub use path::{VirtualPath, VirtualPathError};
```

- [ ] **Step 5: Verify Task 3**

Run:

```bash
cargo test -p wyse-filesystem local::tests
cargo fmt
```

Expected: local backend tests pass and formatting succeeds.

- [ ] **Step 6: Commit Task 3**

```bash
git add crates/wyse-filesystem
git commit -m "feat: add local filesystem backend"
```

---

### Task 4: Patch Parser And Default Apply

**Files:**
- Modify: `crates/wyse-filesystem/src/patch.rs`
- Modify: `crates/wyse-filesystem/src/error.rs`

**Interfaces:**
- Consumes: `Filesystem::read_file`, `Filesystem::write_file`, `Filesystem::remove_file`
- Produces: `impl Patch { pub fn parse(input: &str) -> Result<Self, FilesystemError>; }`
- Produces: `pub(crate) async fn apply_patch_using_filesystem<F: Filesystem + ?Sized>(filesystem: &F, patch: &Patch) -> Result<PatchApplyReport, FilesystemError>`

- [ ] **Step 1: Write failing patch tests**

Replace the test module in `crates/wyse-filesystem/src/patch.rs` with:

```rust
#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, sync::Mutex};

    use bytes::Bytes;

    use super::*;
    use crate::{DirEntry, FileMetadata, FileType, Filesystem, VirtualPath};

    #[test]
    fn parses_add_update_and_delete_paths() {
        let patch = Patch::parse(
            "*** Begin Patch\n\
             *** Add File: /new.txt\n\
             +hello\n\
             *** Update File: /old.txt\n\
              old\n\
             -old\n\
             +new\n\
             *** Delete File: /gone.txt\n\
             *** End Patch\n",
        )
        .expect("patch parses");

        assert_eq!(patch.operations.len(), 3);
    }

    #[test]
    fn rejects_relative_patch_paths() {
        let error = Patch::parse(
            "*** Begin Patch\n\
             *** Add File: new.txt\n\
             +hello\n\
             *** End Patch\n",
        )
        .expect_err("relative path is rejected");

        assert!(matches!(error, crate::FilesystemError::PatchParse));
    }

    #[tokio::test]
    async fn applies_add_update_and_delete_with_default_method() {
        let fs = MemoryFilesystem::new([
            ("/old.txt", "old\n"),
            ("/gone.txt", "bye\n"),
        ]);
        let patch = Patch::parse(
            "*** Begin Patch\n\
             *** Add File: /new.txt\n\
             +hello\n\
             *** Update File: /old.txt\n\
             -old\n\
             +new\n\
             *** Delete File: /gone.txt\n\
             *** End Patch\n",
        )
        .expect("patch parses");

        let report = fs.apply_patch(&patch).await.expect("patch applies");

        assert_eq!(report.added, [VirtualPath::try_from("/new.txt").expect("valid")]);
        assert_eq!(report.updated, [VirtualPath::try_from("/old.txt").expect("valid")]);
        assert_eq!(report.deleted, [VirtualPath::try_from("/gone.txt").expect("valid")]);
        assert_eq!(fs.read_text("/new.txt"), Some("hello\n".to_owned()));
        assert_eq!(fs.read_text("/old.txt"), Some("new\n".to_owned()));
        assert_eq!(fs.read_text("/gone.txt"), None);
    }

    #[tokio::test]
    async fn conflict_does_not_write_any_file() {
        let fs = MemoryFilesystem::new([
            ("/a.txt", "aaa\n"),
            ("/b.txt", "bbb\n"),
        ]);
        let patch = Patch::parse(
            "*** Begin Patch\n\
             *** Update File: /a.txt\n\
             -missing\n\
             +changed\n\
             *** Update File: /b.txt\n\
             -bbb\n\
             +changed\n\
             *** End Patch\n",
        )
        .expect("patch parses");

        let error = fs.apply_patch(&patch).await.expect_err("patch conflicts");

        assert!(matches!(error, crate::FilesystemError::PatchConflict { .. }));
        assert_eq!(fs.read_text("/a.txt"), Some("aaa\n".to_owned()));
        assert_eq!(fs.read_text("/b.txt"), Some("bbb\n".to_owned()));
    }

    #[derive(Debug, Default)]
    struct MemoryFilesystem {
        files: Mutex<BTreeMap<VirtualPath, Bytes>>,
    }

    impl MemoryFilesystem {
        fn new<const N: usize>(files: [(&str, &str); N]) -> Self {
            let mut map = BTreeMap::new();
            for (path, content) in files {
                map.insert(
                    VirtualPath::try_from(path).expect("path is valid"),
                    Bytes::from(content.to_owned()),
                );
            }
            Self {
                files: Mutex::new(map),
            }
        }

        fn read_text(&self, path: &str) -> Option<String> {
            let path = VirtualPath::try_from(path).expect("path is valid");
            let files = self.files.lock().expect("lock is not poisoned");
            files.get(&path).map(|content| String::from_utf8_lossy(content).into_owned())
        }
    }

    impl Filesystem for MemoryFilesystem {
        async fn read_file(&self, path: &VirtualPath) -> Result<Bytes, crate::FilesystemError> {
            let files = self.files.lock().expect("lock is not poisoned");
            files
                .get(path)
                .cloned()
                .ok_or_else(|| crate::FilesystemError::NotFound { path: path.clone() })
        }

        async fn write_file(
            &self,
            path: &VirtualPath,
            contents: Bytes,
        ) -> Result<(), crate::FilesystemError> {
            let mut files = self.files.lock().expect("lock is not poisoned");
            files.insert(path.clone(), contents);
            Ok(())
        }

        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, crate::FilesystemError> {
            Ok(Vec::new())
        }

        async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, crate::FilesystemError> {
            let files = self.files.lock().expect("lock is not poisoned");
            if files.contains_key(path) {
                Ok(FileMetadata {
                    file_type: FileType::File,
                    len: None,
                })
            } else {
                Err(crate::FilesystemError::NotFound { path: path.clone() })
            }
        }

        async fn create_dir(&self, _path: &VirtualPath) -> Result<(), crate::FilesystemError> {
            Ok(())
        }

        async fn remove_file(&self, path: &VirtualPath) -> Result<(), crate::FilesystemError> {
            let mut files = self.files.lock().expect("lock is not poisoned");
            files.remove(path);
            Ok(())
        }

        async fn remove_dir(&self, _path: &VirtualPath) -> Result<(), crate::FilesystemError> {
            Ok(())
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p wyse-filesystem patch::tests
```

Expected: FAIL because parser and apply logic return the default empty report.

- [ ] **Step 3: Implement parser and default applier**

Replace `crates/wyse-filesystem/src/patch.rs` with:

```rust
//! Codex-style patch parsing and application.

use bytes::Bytes;

use crate::{Filesystem, FilesystemError, VirtualPath};

/// Parsed Codex-style patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub(crate) operations: Vec<PatchOperation>,
}

impl Patch {
    /// Parses a Codex-style patch.
    ///
    /// # Errors
    ///
    /// Returns an error when the patch syntax or any path is invalid.
    pub fn parse(input: &str) -> Result<Self, FilesystemError> {
        let mut lines = input.lines();
        if lines.next() != Some("*** Begin Patch") {
            return Err(FilesystemError::PatchParse);
        }

        let mut operations = Vec::new();
        let mut current: Option<RawOperation> = None;

        for line in lines {
            if line == "*** End Patch" {
                if let Some(operation) = current.take() {
                    operations.push(operation.into_operation()?);
                }
                return Ok(Self { operations });
            }

            if let Some(path) = line.strip_prefix("*** Add File: ") {
                if let Some(operation) = current.take() {
                    operations.push(operation.into_operation()?);
                }
                current = Some(RawOperation::add(path)?);
                continue;
            }

            if let Some(path) = line.strip_prefix("*** Update File: ") {
                if let Some(operation) = current.take() {
                    operations.push(operation.into_operation()?);
                }
                current = Some(RawOperation::update(path)?);
                continue;
            }

            if let Some(path) = line.strip_prefix("*** Delete File: ") {
                if let Some(operation) = current.take() {
                    operations.push(operation.into_operation()?);
                }
                operations.push(PatchOperation::Delete {
                    path: parse_patch_path(path)?,
                });
                current = None;
                continue;
            }

            let Some(operation) = current.as_mut() else {
                return Err(FilesystemError::PatchParse);
            };
            operation.push_line(line)?;
        }

        Err(FilesystemError::PatchParse)
    }
}

/// Summary of paths changed by a patch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PatchApplyReport {
    /// Added files.
    pub added: Vec<VirtualPath>,
    /// Updated files.
    pub updated: Vec<VirtualPath>,
    /// Deleted files.
    pub deleted: Vec<VirtualPath>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PatchOperation {
    Add { path: VirtualPath, content: String },
    Update { path: VirtualPath, replacements: Vec<Replacement> },
    Delete { path: VirtualPath },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Replacement {
    old: String,
    new: String,
}

pub(crate) async fn apply_patch_using_filesystem<F>(
    filesystem: &F,
    patch: &Patch,
) -> Result<PatchApplyReport, FilesystemError>
where
    F: Filesystem + ?Sized,
{
    let mut writes = Vec::new();
    let mut deletes = Vec::new();

    for operation in &patch.operations {
        match operation {
            PatchOperation::Add { path, content } => {
                if filesystem.metadata(path).await.is_ok() {
                    return Err(FilesystemError::AlreadyExists { path: path.clone() });
                }
                writes.push((path.clone(), Bytes::from(content.clone()), ChangeKind::Add));
            }
            PatchOperation::Update { path, replacements } => {
                let old = filesystem.read_file(path).await?;
                let old_text = String::from_utf8(old.to_vec())
                    .map_err(|_| FilesystemError::UnsupportedBinaryContent { path: path.clone() })?;
                let new_text = apply_replacements(path, &old_text, replacements)?;
                writes.push((path.clone(), Bytes::from(new_text), ChangeKind::Update));
            }
            PatchOperation::Delete { path } => {
                filesystem.metadata(path).await?;
                deletes.push(path.clone());
            }
        }
    }

    let mut report = PatchApplyReport::default();
    for (path, content, kind) in writes {
        filesystem.write_file(&path, content).await?;
        match kind {
            ChangeKind::Add => report.added.push(path),
            ChangeKind::Update => report.updated.push(path),
        }
    }
    for path in deletes {
        filesystem.remove_file(&path).await?;
        report.deleted.push(path);
    }
    Ok(report)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChangeKind {
    Add,
    Update,
}

#[derive(Debug)]
enum RawOperation {
    Add { path: VirtualPath, lines: Vec<String> },
    Update { path: VirtualPath, lines: Vec<String> },
}

impl RawOperation {
    fn add(path: &str) -> Result<Self, FilesystemError> {
        Ok(Self::Add {
            path: parse_patch_path(path)?,
            lines: Vec::new(),
        })
    }

    fn update(path: &str) -> Result<Self, FilesystemError> {
        Ok(Self::Update {
            path: parse_patch_path(path)?,
            lines: Vec::new(),
        })
    }

    fn push_line(&mut self, line: &str) -> Result<(), FilesystemError> {
        match line.as_bytes().first() {
            Some(b'+') | Some(b'-') | Some(b' ') => {
                let target = match self {
                    Self::Add { lines, .. } | Self::Update { lines, .. } => lines,
                };
                target.push(line.to_owned());
                Ok(())
            }
            _ => Err(FilesystemError::PatchParse),
        }
    }

    fn into_operation(self) -> Result<PatchOperation, FilesystemError> {
        match self {
            Self::Add { path, lines } => {
                let mut content = String::new();
                for line in lines {
                    let Some(text) = line.strip_prefix('+') else {
                        return Err(FilesystemError::PatchParse);
                    };
                    content.push_str(text);
                    content.push('\n');
                }
                Ok(PatchOperation::Add { path, content })
            }
            Self::Update { path, lines } => Ok(PatchOperation::Update {
                path,
                replacements: parse_replacements(lines)?,
            }),
        }
    }
}

fn parse_replacements(lines: Vec<String>) -> Result<Vec<Replacement>, FilesystemError> {
    let mut replacements = Vec::new();
    let mut old = String::new();
    let mut new = String::new();

    for line in lines {
        if let Some(text) = line.strip_prefix('-') {
            old.push_str(text);
            old.push('\n');
        } else if let Some(text) = line.strip_prefix('+') {
            new.push_str(text);
            new.push('\n');
        } else if line.starts_with(' ') {
            if !old.is_empty() || !new.is_empty() {
                replacements.push(Replacement {
                    old: std::mem::take(&mut old),
                    new: std::mem::take(&mut new),
                });
            }
        } else {
            return Err(FilesystemError::PatchParse);
        }
    }

    if !old.is_empty() || !new.is_empty() {
        replacements.push(Replacement { old, new });
    }

    if replacements.is_empty() {
        return Err(FilesystemError::PatchParse);
    }

    Ok(replacements)
}

fn apply_replacements(
    path: &VirtualPath,
    original: &str,
    replacements: &[Replacement],
) -> Result<String, FilesystemError> {
    let mut output = original.to_owned();
    for replacement in replacements {
        if !output.contains(&replacement.old) {
            return Err(FilesystemError::PatchConflict { path: path.clone() });
        }
        output = output.replacen(&replacement.old, &replacement.new, 1);
    }
    Ok(output)
}

fn parse_patch_path(path: &str) -> Result<VirtualPath, FilesystemError> {
    VirtualPath::try_from(path).map_err(|_| FilesystemError::PatchParse)
}
```

Keep the complete test module from Step 1 at the bottom of `crates/wyse-filesystem/src/patch.rs` after the implementation.

- [ ] **Step 4: Verify Task 4**

Run:

```bash
cargo test -p wyse-filesystem patch::tests
cargo fmt
```

Expected: patch tests pass and formatting succeeds.

- [ ] **Step 5: Commit Task 4**

```bash
git add crates/wyse-filesystem
git commit -m "feat: add filesystem patch support"
```

---

### Task 5: Final Safety Tests, Docs, And Workspace Verification

**Files:**
- Create: `crates/wyse-filesystem/AGENTS.md`
- Modify: `crates/wyse-filesystem/src/local.rs`
- Modify: `crates/wyse-filesystem/src/lib.rs`

**Interfaces:**
- Consumes all previous public APIs.
- Produces crate-level implementation conventions in `crates/wyse-filesystem/AGENTS.md`.

- [ ] **Step 1: Add symlink escape test**

Add this test to `crates/wyse-filesystem/src/local.rs` tests:

```rust
#[cfg(unix)]
#[tokio::test]
async fn rejects_symlink_escape() {
    use std::os::unix::fs::symlink;

    let temp = std::env::temp_dir().join(format!("wyse-fs-symlink-{}", std::process::id()));
    let outside = std::env::temp_dir().join(format!("wyse-fs-outside-{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp).await;
    let _ = tokio::fs::remove_dir_all(&outside).await;
    tokio::fs::create_dir_all(&temp).await.expect("create root");
    tokio::fs::create_dir_all(&outside).await.expect("create outside");
    tokio::fs::write(outside.join("secret.txt"), b"secret")
        .await
        .expect("write outside file");
    symlink(outside.join("secret.txt"), temp.join("link.txt")).expect("create symlink");

    let fs = LocalFilesystem::new(LocalFilesystemConfig {
        root: temp.clone(),
        max_file_bytes: Some(1024),
    })
    .expect("filesystem is valid");
    let link = VirtualPath::try_from("/link.txt").expect("path is valid");

    let error = fs.read_file(&link).await.expect_err("symlink escape is rejected");
    assert!(matches!(error, crate::FilesystemError::PathEscapesSandbox { .. }));

    let _ = tokio::fs::remove_dir_all(&temp).await;
    let _ = tokio::fs::remove_dir_all(&outside).await;
}
```

- [ ] **Step 2: Add default apply_patch integration test with LocalFilesystem**

Add this test to `crates/wyse-filesystem/src/local.rs` tests:

```rust
#[tokio::test]
async fn local_filesystem_uses_default_apply_patch() {
    let temp = std::env::temp_dir().join(format!("wyse-fs-patch-{}", std::process::id()));
    let _ = tokio::fs::remove_dir_all(&temp).await;
    tokio::fs::create_dir_all(&temp).await.expect("create temp root");
    tokio::fs::write(temp.join("old.txt"), b"old\n").await.expect("write old file");

    let fs = LocalFilesystem::new(LocalFilesystemConfig {
        root: temp.clone(),
        max_file_bytes: Some(1024),
    })
    .expect("filesystem is valid");
    let patch = crate::Patch::parse(
        "*** Begin Patch\n\
         *** Update File: /old.txt\n\
         -old\n\
         +new\n\
         *** End Patch\n",
    )
    .expect("patch parses");

    let report = fs.apply_patch(&patch).await.expect("patch applies");

    assert_eq!(report.updated, [VirtualPath::try_from("/old.txt").expect("valid")]);
    let content = fs
        .read_file(&VirtualPath::try_from("/old.txt").expect("valid"))
        .await
        .expect("read updated file");
    assert_eq!(content, Bytes::from_static(b"new\n"));

    let _ = tokio::fs::remove_dir_all(&temp).await;
}
```

- [ ] **Step 3: Write crate AGENTS.md**

Create `crates/wyse-filesystem/AGENTS.md`:

```markdown
# wyse-filesystem AGENTS.md

## Scope

`wyse-filesystem` owns the agent-visible virtual filesystem trait, virtual path validation, Codex-style patch application, and the local sandbox backend.

## Design Rules

- Public file APIs accept `VirtualPath`, not raw strings or host paths.
- Keep paths virtual and absolute, for example `/README.md`.
- Do not expose host paths, sandbox roots, file contents, or patch contents in errors or tracing.
- Backend implementations should implement minimal file primitives; keep `apply_patch` as the default trait method until a backend has a real need to override it.
- `remove_dir` removes empty directories only.
- Do not add mount routers, registries, factories, managers, read-only policy, stream IO, glob/search, watch, snapshot, remote backends, or object storage until a concrete caller needs them.
- Local sandbox operations must reject symlink escapes by default.
```

- [ ] **Step 4: Ensure crate root docs are complete**

Replace `crates/wyse-filesystem/src/lib.rs` with:

```rust
//! Virtual filesystem abstractions and local sandbox backend for Wyse agents.
//!
//! This crate keeps agent paths virtual. Public APIs accept [`VirtualPath`],
//! while backend implementations decide how to map those paths to storage.

pub mod definition;
pub mod error;
pub mod local;
pub mod patch;
pub mod path;

pub use definition::{DirEntry, FileMetadata, FileType, Filesystem};
pub use error::FilesystemError;
pub use local::{LocalFilesystem, LocalFilesystemConfig};
pub use patch::{Patch, PatchApplyReport};
pub use path::{VirtualPath, VirtualPathError};
```

- [ ] **Step 5: Run focused tests**

Run:

```bash
cargo test -p wyse-filesystem
```

Expected: all `wyse-filesystem` tests pass.

- [ ] **Step 6: Run workspace verification**

Run:

```bash
cargo fmt
cargo test --workspace --all-targets
cargo clippy --workspace --all-targets
```

Expected: all commands pass.

- [ ] **Step 7: Commit Task 5**

```bash
git add Cargo.toml crates/wyse-filesystem
git commit -m "test: verify filesystem safety"
```

---

## Self-Review

- Spec coverage: The plan covers async trait, `VirtualPath`, whole-file `Bytes` IO, local sandbox backend, empty-dir-only removal, default `apply_patch`, error redaction constraints, and tests for path validation, patch behavior, local IO, size limits, and symlink escape rejection.
- Placeholder scan: The plan contains no `TBD`, `TODO`, "implement later", or unspecified edge-case instructions.
- Type consistency: `VirtualPath`, `FilesystemError`, `Filesystem`, `Patch`, `PatchApplyReport`, `LocalFilesystem`, and `LocalFilesystemConfig` are introduced before later tasks consume them.
- Ponytail check: No mount router, registry, factory, policy layer, streaming API, recursive delete, remote backend, or new dependency beyond already-present workspace crates.
