use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use async_trait::async_trait;
use tokio::sync::Notify;
use wyse_filesystem::{
    CasExpectation, DirEntry, Entry, FileMetadata, FileType, Filesystem, FilesystemError,
    RecordVersion, VersionedEntry, VirtualPath,
};

#[derive(Default)]
pub(super) struct MemoryCasFilesystem {
    records: Mutex<BTreeMap<VirtualPath, VersionedEntry>>,
    directories: Mutex<BTreeSet<VirtualPath>>,
    read_counts: Mutex<BTreeMap<VirtualPath, u64>>,
    list_count: AtomicU64,
    next_version: AtomicU64,
    fail_next_version_write: AtomicBool,
    pause_next_version_write: AtomicBool,
    version_write_paused: Notify,
    resume_version_write: Notify,
}

impl MemoryCasFilesystem {
    pub(super) fn exists(&self, path: &str) -> bool {
        let path = VirtualPath::try_from(path).expect("valid fixture path");
        self.records
            .lock()
            .expect("records mutex")
            .contains_key(&path)
    }

    pub(super) fn insert_entry(&self, path: &str, entry: Entry) {
        let path = VirtualPath::try_from(path).expect("valid fixture path");
        let version = self.next_record_version();
        self.records
            .lock()
            .expect("records mutex")
            .insert(path, VersionedEntry { entry, version });
    }

    pub(super) fn remove_entry(&self, path: &str) {
        let path = VirtualPath::try_from(path).expect("valid fixture path");
        self.records.lock().expect("records mutex").remove(&path);
    }

    pub(super) fn entry_version(&self, path: &str) -> Option<RecordVersion> {
        let path = VirtualPath::try_from(path).expect("valid fixture path");
        self.records
            .lock()
            .expect("records mutex")
            .get(&path)
            .map(|record| record.version)
    }

    pub(super) fn entry(&self, path: &str) -> Option<Entry> {
        let path = VirtualPath::try_from(path).expect("valid fixture path");
        self.records
            .lock()
            .expect("records mutex")
            .get(&path)
            .map(|record| record.entry.clone())
    }

    pub(super) fn fail_next_version_write(&self) {
        self.fail_next_version_write.store(true, Ordering::SeqCst);
    }

    pub(super) fn version_write_failure_pending(&self) -> bool {
        self.fail_next_version_write.load(Ordering::SeqCst)
    }

    pub(super) fn pause_next_version_write(&self) {
        self.pause_next_version_write.store(true, Ordering::SeqCst);
    }

    pub(super) async fn wait_for_version_write_pause(&self) {
        self.version_write_paused.notified().await;
    }

    pub(super) fn resume_version_write(&self) {
        self.resume_version_write.notify_one();
    }

    pub(super) fn reset_read_counts(&self) {
        self.read_counts.lock().expect("read counts mutex").clear();
        self.list_count.store(0, Ordering::SeqCst);
    }

    pub(super) fn read_count(&self, path: &str) -> u64 {
        let path = VirtualPath::try_from(path).expect("valid fixture path");
        self.read_counts
            .lock()
            .expect("read counts mutex")
            .get(&path)
            .copied()
            .unwrap_or_default()
    }

    pub(super) fn list_count(&self) -> u64 {
        self.list_count.load(Ordering::SeqCst)
    }

    fn next_record_version(&self) -> RecordVersion {
        RecordVersion::from_backend(self.next_version.fetch_add(1, Ordering::SeqCst))
    }
}

#[async_trait]
impl Filesystem for MemoryCasFilesystem {
    async fn get(&self, path: &VirtualPath) -> Result<Option<VersionedEntry>, FilesystemError> {
        *self
            .read_counts
            .lock()
            .expect("read counts mutex")
            .entry(path.clone())
            .or_default() += 1;
        Ok(self
            .records
            .lock()
            .expect("records mutex")
            .get(path)
            .cloned())
    }

    async fn put(
        &self,
        path: &VirtualPath,
        entry: Entry,
        cas: CasExpectation,
    ) -> Result<RecordVersion, FilesystemError> {
        if matches!(cas, CasExpectation::Version(_))
            && self.pause_next_version_write.swap(false, Ordering::SeqCst)
        {
            self.version_write_paused.notify_one();
            self.resume_version_write.notified().await;
        }
        if matches!(cas, CasExpectation::Version(_))
            && self.fail_next_version_write.swap(false, Ordering::SeqCst)
        {
            return Err(FilesystemError::VersionMismatch { path: path.clone() });
        }
        let mut records = self.records.lock().expect("records mutex");

        match cas {
            CasExpectation::Absent if records.contains_key(path) => {
                Err(FilesystemError::VersionMismatch { path: path.clone() })
            }
            CasExpectation::Version(expected)
                if records.get(path).map(|record| record.version) != Some(expected) =>
            {
                Err(FilesystemError::VersionMismatch { path: path.clone() })
            }
            CasExpectation::Absent | CasExpectation::Version(_) | CasExpectation::Any => {
                let version = self.next_record_version();
                records.insert(path.clone(), VersionedEntry { entry, version });
                Ok(version)
            }
        }
    }

    async fn read_file(&self, path: &VirtualPath) -> Result<Vec<u8>, FilesystemError> {
        self.get(path)
            .await?
            .map(|record| record.entry.into_contents())
            .ok_or_else(|| FilesystemError::NotFound { path: path.clone() })
    }

    async fn write_file(
        &self,
        path: &VirtualPath,
        contents: Vec<u8>,
    ) -> Result<(), FilesystemError> {
        self.put(path, Entry::new(contents), CasExpectation::Any)
            .await
            .map(|_| ())
    }

    async fn list_dir(&self, path: &VirtualPath) -> Result<Vec<DirEntry>, FilesystemError> {
        self.list_count.fetch_add(1, Ordering::SeqCst);
        if !self
            .directories
            .lock()
            .expect("directories mutex")
            .contains(path)
        {
            return Err(FilesystemError::NotFound { path: path.clone() });
        }
        let prefix = if path.as_str() == "/" {
            "/".to_owned()
        } else {
            format!("{}/", path.as_str())
        };
        let records = self.records.lock().expect("records mutex");
        Ok(records
            .keys()
            .filter_map(|record_path| {
                let file_name = record_path.as_str().strip_prefix(&prefix)?;
                (!file_name.contains('/')).then(|| {
                    DirEntry::from_backend(
                        record_path.clone(),
                        file_name.to_owned(),
                        FileType::File,
                    )
                })
            })
            .collect())
    }

    async fn metadata(&self, path: &VirtualPath) -> Result<FileMetadata, FilesystemError> {
        Err(FilesystemError::NotFound { path: path.clone() })
    }

    async fn create_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.directories
            .lock()
            .expect("directories mutex")
            .insert(path.clone());
        Ok(())
    }

    async fn remove_file(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.records
            .lock()
            .expect("records mutex")
            .remove(path)
            .map(|_| ())
            .ok_or_else(|| FilesystemError::NotFound { path: path.clone() })
    }

    async fn remove_dir(&self, path: &VirtualPath) -> Result<(), FilesystemError> {
        self.directories
            .lock()
            .expect("directories mutex")
            .remove(path)
            .then_some(())
            .ok_or_else(|| FilesystemError::NotFound { path: path.clone() })
    }
}
