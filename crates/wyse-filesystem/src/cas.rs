//! Shared compare-and-swap update support.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::time::{sleep, timeout};

use crate::{CasExpectation, Entry, Filesystem, FilesystemError, VirtualPath};

/// Maximum number of compare-and-swap write attempts.
pub const FILESYSTEM_CAS_RETRIES: usize = 32;
/// Maximum duration of one complete compare-and-swap update.
pub const FILESYSTEM_APPLY_TIMEOUT: Duration = Duration::from_secs(15);
const INITIAL_BACKOFF: Duration = Duration::from_millis(2);
const MAX_BACKOFF: Duration = Duration::from_millis(50);

/// Error returned by [`cas_update`].
#[derive(Debug, thiserror::Error)]
pub enum CasUpdateError<E: std::error::Error + 'static> {
    /// The filesystem backend does not provide compare-and-swap operations.
    #[error("filesystem backend does not support compare-and-swap")]
    CasUnsupported,
    /// The complete update exceeded [`FILESYSTEM_APPLY_TIMEOUT`].
    #[error("compare-and-swap update timed out")]
    Timeout,
    /// Every permitted compare-and-swap write conflicted.
    #[error("compare-and-swap retries exhausted")]
    RetriesExhausted,
    /// A filesystem operation failed.
    #[error("filesystem operation failed")]
    Filesystem(#[source] FilesystemError),
    /// Decoding, applying, or encoding the update failed.
    #[error("compare-and-swap apply failed")]
    Apply(#[source] E),
}

/// Reads, transforms, and conditionally replaces one versioned record.
///
/// A version mismatch causes the record to be read and the transformation to be
/// applied again, up to [`FILESYSTEM_CAS_RETRIES`] attempts.
///
/// # Errors
///
/// Returns [`CasUpdateError`] when the backend does not support compare-and-swap,
/// the record is missing, the operation times out or exhausts its retries, a
/// filesystem operation fails, or one of the supplied transformations fails.
pub async fn cas_update<T, E, Decode, Encode, Apply>(
    filesystem: &dyn Filesystem,
    path: &VirtualPath,
    decode: Decode,
    encode: Encode,
    apply: Apply,
) -> Result<T, CasUpdateError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    Decode: Fn(&Entry) -> Result<T, E>,
    Encode: Fn(&T) -> Result<Entry, E>,
    Apply: Fn(&T) -> Result<T, E>,
{
    timeout(
        FILESYSTEM_APPLY_TIMEOUT,
        cas_update_inner(filesystem, path, &decode, &encode, &apply),
    )
    .await
    .map_err(|_| CasUpdateError::Timeout)?
}

async fn cas_update_inner<T, E, Decode, Encode, Apply>(
    filesystem: &dyn Filesystem,
    path: &VirtualPath,
    decode: &Decode,
    encode: &Encode,
    apply: &Apply,
) -> Result<T, CasUpdateError<E>>
where
    E: std::error::Error + Send + Sync + 'static,
    Decode: Fn(&Entry) -> Result<T, E>,
    Encode: Fn(&T) -> Result<Entry, E>,
    Apply: Fn(&T) -> Result<T, E>,
{
    for attempt in 0..FILESYSTEM_CAS_RETRIES {
        let current = match filesystem.get(path).await {
            Ok(Some(current)) => current,
            Ok(None) => {
                return Err(CasUpdateError::Filesystem(FilesystemError::NotFound {
                    path: path.clone(),
                }));
            }
            Err(FilesystemError::UnsupportedCas) => {
                return Err(CasUpdateError::CasUnsupported);
            }
            Err(error) => return Err(CasUpdateError::Filesystem(error)),
        };
        let decoded = decode(&current.entry).map_err(CasUpdateError::Apply)?;
        let next = apply(&decoded).map_err(CasUpdateError::Apply)?;
        let entry = encode(&next).map_err(CasUpdateError::Apply)?;

        match filesystem
            .put(path, entry, CasExpectation::Version(current.version))
            .await
        {
            Ok(_) => return Ok(next),
            Err(FilesystemError::VersionMismatch { .. })
                if attempt + 1 < FILESYSTEM_CAS_RETRIES =>
            {
                sleep(jittered_backoff(attempt)).await;
            }
            Err(FilesystemError::VersionMismatch { .. }) => {
                return Err(CasUpdateError::RetriesExhausted);
            }
            Err(FilesystemError::UnsupportedCas) => {
                return Err(CasUpdateError::CasUnsupported);
            }
            Err(error) => return Err(CasUpdateError::Filesystem(error)),
        }
    }
    Err(CasUpdateError::RetriesExhausted)
}

fn jittered_backoff(attempt: usize) -> Duration {
    let shift = u32::try_from(attempt.min(5)).expect("attempt fits u32");
    let base_ms = INITIAL_BACKOFF.as_millis().try_into().unwrap_or(u64::MAX);
    let max_ms = MAX_BACKOFF.as_millis().try_into().unwrap_or(u64::MAX);
    let base_ms = base_ms.saturating_mul(1_u64 << shift).min(max_ms);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    let jitter_ms = u64::from(nanos) % (base_ms / 2 + 1);
    Duration::from_millis((base_ms + jitter_ms).min(max_ms))
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;

    use crate::{
        CasExpectation, DirEntry, Entry, FileMetadata, Filesystem, FilesystemError, RecordVersion,
        VersionedEntry, VirtualPath, cas_update,
    };

    struct ScriptedFilesystem {
        version: AtomicUsize,
        puts: AtomicUsize,
    }

    impl ScriptedFilesystem {
        fn with_one_conflict() -> Self {
            Self {
                version: AtomicUsize::new(1),
                puts: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl Filesystem for ScriptedFilesystem {
        async fn read_file(&self, _path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
            unreachable!("CAS test does not read files")
        }

        async fn write_file(
            &self,
            _path: &VirtualPath,
            _contents: Vec<u8>,
        ) -> Result<(), FilesystemError> {
            unreachable!("CAS test does not write files")
        }

        async fn list_dir(&self, _path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
            unreachable!("CAS test does not list directories")
        }

        async fn metadata(&self, _path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
            unreachable!("CAS test does not read metadata")
        }

        async fn create_dir(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            unreachable!("CAS test does not create directories")
        }

        async fn remove_file(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            unreachable!("CAS test does not remove files")
        }

        async fn remove_dir(&self, _path: &VirtualPath) -> Result<(), FilesystemError> {
            unreachable!("CAS test does not remove directories")
        }

        async fn get(
            &self,
            _path: &VirtualPath,
        ) -> Result<Option<VersionedEntry>, FilesystemError> {
            Ok(Some(VersionedEntry {
                entry: Entry::new(vec![1]),
                version: RecordVersion::from_backend(
                    u64::try_from(self.version.load(Ordering::SeqCst))
                        .expect("test version fits u64"),
                ),
            }))
        }

        async fn put(
            &self,
            path: &VirtualPath,
            _entry: Entry,
            cas: CasExpectation,
        ) -> Result<RecordVersion, FilesystemError> {
            let expected = CasExpectation::Version(RecordVersion::from_backend(
                u64::try_from(self.version.load(Ordering::SeqCst)).expect("test version fits u64"),
            ));
            assert_eq!(cas, expected);

            if self.puts.fetch_add(1, Ordering::SeqCst) == 0 {
                self.version.store(2, Ordering::SeqCst);
                return Err(FilesystemError::VersionMismatch { path: path.clone() });
            }

            Ok(RecordVersion::from_backend(3))
        }
    }

    #[test]
    fn record_versions_support_equality() {
        assert_eq!(
            RecordVersion::from_backend(1),
            RecordVersion::from_backend(1)
        );
        assert_ne!(
            RecordVersion::from_backend(1),
            RecordVersion::from_backend(2)
        );
    }

    #[tokio::test]
    async fn cas_update_reloads_and_reapplies_after_version_mismatch() {
        let filesystem = ScriptedFilesystem::with_one_conflict();
        let path = VirtualPath::try_from("/agent.json").expect("valid path");
        let applies = AtomicUsize::new(0);

        let updated = cas_update(
            &filesystem,
            &path,
            |entry| Ok::<u8, Infallible>(entry.contents()[0]),
            |value| Ok::<Entry, Infallible>(Entry::new(vec![*value])),
            |current| {
                applies.fetch_add(1, Ordering::SeqCst);
                Ok::<u8, Infallible>(*current + 1)
            },
        )
        .await
        .expect("CAS eventually succeeds");

        assert_eq!(updated, 2);
        assert_eq!(applies.load(Ordering::SeqCst), 2);
    }
}
