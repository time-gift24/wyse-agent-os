//! Virtual filesystem abstractions and local sandbox backend for Stratum agents.
//!
//! This crate keeps agent paths virtual. Public APIs accept [`VirtualPath`],
//! while backend implementations decide how to map those paths to storage.

pub mod apply_patch;
pub mod cas;
pub mod definition;
pub mod error;
pub mod local;
pub mod path;
pub mod record;

pub use apply_patch::{ApplyPatchError, ApplyPatchOperation, ApplyPatchOperationKind, apply_patch};
pub use cas::{CasUpdateError, FILESYSTEM_APPLY_TIMEOUT, FILESYSTEM_CAS_RETRIES, cas_update};
pub use definition::{DirEntry, FileMetadata, FileType, Filesystem};
pub use error::FilesystemError;
pub use local::{LocalFilesystem, LocalFilesystemConfig};
pub use path::{VirtualPath, VirtualPathError};
pub use record::{CasExpectation, Entry, RecordVersion, VersionedEntry};
