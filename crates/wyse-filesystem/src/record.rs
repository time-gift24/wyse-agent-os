//! Versioned filesystem record types.

/// Complete contents of one filesystem record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    contents: Vec<u8>,
}

impl Entry {
    /// Creates a record entry from its complete contents.
    #[must_use]
    pub fn new(contents: Vec<u8>) -> Self {
        Self { contents }
    }

    /// Returns the complete record contents.
    #[must_use]
    pub fn contents(&self) -> &[u8] {
        &self.contents
    }

    /// Consumes the entry and returns its complete contents.
    #[must_use]
    pub fn into_contents(self) -> Vec<u8> {
        self.contents
    }
}

/// Opaque version assigned by a filesystem backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordVersion(u64);

impl RecordVersion {
    /// Creates a version from a backend-specific monotonic value.
    #[doc(hidden)]
    #[must_use]
    pub const fn from_backend(value: u64) -> Self {
        Self(value)
    }
}

/// Condition required for a record write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CasExpectation {
    /// The record must not exist.
    Absent,
    /// The record must have the given version.
    Version(RecordVersion),
    /// No version condition is required.
    Any,
}

/// Record contents paired with their backend-assigned version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionedEntry {
    /// Record contents.
    pub entry: Entry,
    /// Backend-assigned record version.
    pub version: RecordVersion,
}
