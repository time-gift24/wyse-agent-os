//! Virtual filesystem abstractions and local sandbox backend for Wyse agents.
//!
//! This crate keeps agent paths virtual. Public APIs accept [`VirtualPath`],
//! while backend implementations decide how to map those paths to storage.

pub mod apply_patch;
pub mod definition;
pub mod error;
pub mod local;
pub mod path;

pub use apply_patch::{ApplyPatchError, ApplyPatchOperation, ApplyPatchOperationKind, apply_patch};
pub use definition::{DirEntry, FileMetadata, FileType, Filesystem};
pub use error::FilesystemError;
pub use local::{LocalFilesystem, LocalFilesystemConfig};
pub use path::{VirtualPath, VirtualPathError};
