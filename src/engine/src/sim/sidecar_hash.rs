//! Content hashes of `.col.json` sidecars for the disk-cache key.
//!
//! A sidecar is keyed by what it contains, not by when it was written, so an
//! installer run that rewrites byte-identical sidecars keeps every baked blob
//! valid. Reading and hashing every sidecar on each launch would cost far more
//! than the `stat` the old revision token needed, so a persistent index maps
//! (path, length, mtime) to the hash: a matching stat reuses the stored hash
//! without opening the file, anything else is re-read and re-hashed.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use xxhash_rust::xxh3::xxh3_128;

use crate::shared_cache::SharedCache;

const INDEX_FILE_NAME: &str = "sidecar-hashes.idx";
/// Bump when `IndexFile` or `IndexEntry` change shape; an index of another
/// format is ignored and rebuilt.
const INDEX_FORMAT: u32 = 1;

static INDEX_TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// xxh3-128 of a sidecar's bytes, rendered as the key component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SidecarHash(u128);

impl SidecarHash {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        Self(xxh3_128(bytes))
    }

    pub fn key_component(self) -> String {
        format!("{:032x}", self.0)
    }
}

/// What a sidecar lookup found. `Unreadable` (the file exists but a stat or read
/// failed) is never memoized, so the next lookup retries; the other outcomes are
/// facts about the file and stay for the run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidecarLookup {
    Absent,
    Hashed(SidecarHash),
    Unreadable,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct FileStamp {
    len: u64,
    mtime_ns: u128,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct IndexEntry {
    stamp: FileStamp,
    hash: u128,
}

#[derive(Serialize, Deserialize)]
struct IndexFile {
    format: u32,
    entries: Vec<(String, IndexEntry)>,
}

#[derive(Debug, thiserror::Error)]
enum IndexLoadError {
    #[error("unreadable: {0}")]
    Read(std::io::Error),
    #[error("corrupt: {0}")]
    Decode(bincode::Error),
    #[error("of format {found} (expected {INDEX_FORMAT})")]
    WrongFormat { found: u32 },
}

#[derive(Debug, thiserror::Error)]
enum IndexSaveError {
    #[error("serialize failed: {0}")]
    Serialize(bincode::Error),
    #[error("writing {path} failed: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("renaming {path} into place failed: {source}")]
    Rename {
        path: PathBuf,
        source: std::io::Error,
    },
}

enum SidecarStat {
    Missing,
    Failed,
    Stamped(FileStamp),
    /// The file exists but its modification time is unavailable, so it cannot be
    /// told apart from a rewrite: it is always re-hashed and never indexed.
    Unstamped,
}

/// Per-process memo of sidecar hashes backed by the persistent index. Safe to
/// share between streaming workers: a path is hashed once per run.
pub struct SidecarHasher {
    index_path: Option<PathBuf>,
    /// Index paths are stored relative to this directory when they lie under
    /// it, so moving the install keeps the index valid.
    path_base: Option<PathBuf>,
    index: Mutex<HashMap<String, IndexEntry>>,
    dirty: AtomicBool,
    memo: SharedCache<String, SidecarLookup>,
}

impl SidecarHasher {
    /// `index_dir` is where the index file lives; `None` disables persistence
    /// (hashes are still memoized for the run).
    pub fn new(index_dir: Option<&Path>, path_base: Option<&Path>) -> Self {
        let index_path = index_dir.map(|dir| dir.join(INDEX_FILE_NAME));
        let index = index_path.as_deref().map(load_index).unwrap_or_default();
        Self {
            index_path,
            path_base: path_base.map(Path::to_owned),
            index: Mutex::new(index),
            dirty: AtomicBool::new(false),
            memo: SharedCache::new(0),
        }
    }

    pub fn lookup(&self, path: &str) -> SidecarLookup {
        self.memo
            .resolve(
                &[path.to_owned()],
                |p| self.resolve_uncached(p),
                // Weight 0 pins an entry; weight 1 against the zero cap drops it
                // again right away, which is how `Unreadable` stays unmemoized.
                |lookup| u64::from(*lookup == SidecarLookup::Unreadable),
            )
            .remove(path)
            .unwrap_or(SidecarLookup::Unreadable)
    }

    fn index_key(&self, path: &str) -> String {
        let relative = self
            .path_base
            .as_deref()
            .and_then(|base| Path::new(path).strip_prefix(base).ok());
        relative
            .map_or_else(|| path.to_owned(), |rel| rel.to_string_lossy().into_owned())
            .replace('\\', "/")
    }

    fn resolve_uncached(&self, path: &str) -> SidecarLookup {
        match stat_sidecar(path) {
            SidecarStat::Missing => SidecarLookup::Absent,
            SidecarStat::Failed => SidecarLookup::Unreadable,
            SidecarStat::Unstamped => read_and_hash(path),
            SidecarStat::Stamped(stamp) => {
                let key = self.index_key(path);
                if let Some(entry) = self.index.lock().unwrap().get(&key)
                    && entry.stamp == stamp
                {
                    return SidecarLookup::Hashed(SidecarHash(entry.hash));
                }
                // Stat precedes the read: a file rewritten in between is stored
                // with its old stamp and simply re-hashed on the next run.
                let lookup = read_and_hash(path);
                if let SidecarLookup::Hashed(hash) = lookup {
                    self.index.lock().unwrap().insert(
                        key,
                        IndexEntry {
                            stamp,
                            hash: hash.0,
                        },
                    );
                    self.dirty.store(true, Ordering::Release);
                }
                lookup
            }
        }
    }

    /// Persists the index if hashes were added since the last flush, dropping
    /// entries whose sidecar no longer exists. Does one stat per entry, so call
    /// it off the hot path. A failure is logged and the flag stays set, so the
    /// next flush retries; meanwhile the next run just re-hashes.
    pub fn flush(&self) {
        let Some(path) = &self.index_path else {
            return;
        };
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        let snapshot: Vec<(String, IndexEntry)> = self
            .index
            .lock()
            .unwrap()
            .iter()
            .map(|(key, entry)| (key.clone(), *entry))
            .collect();
        let mut entries: Vec<_> = snapshot
            .into_iter()
            .filter(|(key, _)| self.sidecar_still_exists(key))
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let file = IndexFile {
            format: INDEX_FORMAT,
            entries,
        };
        if let Err(e) = write_index(path, &file) {
            self.dirty.store(true, Ordering::Release);
            tracing::warn!("collision cache: sidecar hash index not saved: {e}");
        }
    }

    fn sidecar_still_exists(&self, index_key: &str) -> bool {
        let path = self
            .path_base
            .as_deref()
            .map_or_else(|| PathBuf::from(index_key), |base| base.join(index_key));
        match std::fs::metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => {
                tracing::warn!(
                    "collision cache: can't stat {} while pruning the index: {e}",
                    path.display()
                );
                true
            }
            Ok(_) => true,
        }
    }
}

impl Drop for SidecarHasher {
    fn drop(&mut self) {
        self.flush();
    }
}

fn read_and_hash(path: &str) -> SidecarLookup {
    match std::fs::read(path) {
        Ok(bytes) => SidecarLookup::Hashed(SidecarHash::of_bytes(&bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => SidecarLookup::Absent,
        Err(e) => {
            tracing::warn!("collision cache: can't read sidecar {path}: {e}");
            SidecarLookup::Unreadable
        }
    }
}

fn stat_sidecar(path: &str) -> SidecarStat {
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return SidecarStat::Missing,
        Err(e) => {
            tracing::warn!("collision cache: can't stat sidecar {path}: {e}");
            return SidecarStat::Failed;
        }
    };
    if let Some(stamp) = stamp_of(meta.len(), meta.modified().ok()) {
        return SidecarStat::Stamped(stamp);
    }
    crate::utils::warn_once(&format!("sidecar-no-mtime:{path}"), || {
        format!(
            "collision cache: sidecar {path} has no usable modification time;              it is re-hashed every run"
        )
    });
    SidecarStat::Unstamped
}

fn stamp_of(len: u64, modified: Option<SystemTime>) -> Option<FileStamp> {
    let mtime_ns = modified?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some(FileStamp { len, mtime_ns })
}

fn load_index(path: &Path) -> HashMap<String, IndexEntry> {
    match try_load_index(path) {
        Ok(entries) => entries,
        Err(IndexLoadError::Read(e)) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
        Err(e) => {
            tracing::warn!(
                "collision cache: sidecar hash index {} is {e}; re-hashing",
                path.display()
            );
            HashMap::new()
        }
    }
}

fn try_load_index(path: &Path) -> Result<HashMap<String, IndexEntry>, IndexLoadError> {
    let bytes = std::fs::read(path).map_err(IndexLoadError::Read)?;
    let file: IndexFile = bincode::deserialize(&bytes).map_err(IndexLoadError::Decode)?;
    if file.format != INDEX_FORMAT {
        return Err(IndexLoadError::WrongFormat { found: file.format });
    }
    Ok(file.entries.into_iter().collect())
}

fn write_index(path: &Path, file: &IndexFile) -> Result<(), IndexSaveError> {
    let bytes = bincode::serialize(file).map_err(IndexSaveError::Serialize)?;
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        std::process::id(),
        INDEX_TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(source) = std::fs::write(&tmp, &bytes) {
        remove_temp(&tmp);
        return Err(IndexSaveError::Write { path: tmp, source });
    }
    std::fs::rename(&tmp, path).map_err(|source| {
        remove_temp(&tmp);
        IndexSaveError::Rename {
            path: path.to_owned(),
            source,
        }
    })
}

fn remove_temp(tmp: &Path) {
    if let Err(e) = std::fs::remove_file(tmp)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            "collision cache: can't remove temp file {}: {e}",
            tmp.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::sim::physics::test_support::{set_mtime, write_file};

    fn sidecar(dir: &Path, name: &str, contents: &[u8]) -> String {
        write_file(dir, name, contents).to_str().unwrap().to_owned()
    }

    fn hash(hasher: &SidecarHasher, path: &str) -> SidecarHash {
        match hasher.lookup(path) {
            SidecarLookup::Hashed(hash) => hash,
            other => panic!("expected a hash, got {other:?}"),
        }
    }

    fn standalone(index_dir: &Path) -> SidecarHasher {
        SidecarHasher::new(Some(index_dir), None)
    }

    fn index_file(dir: &Path) -> PathBuf {
        dir.join(INDEX_FILE_NAME)
    }

    fn stored_keys(dir: &Path) -> Vec<String> {
        try_load_index(&index_file(dir))
            .unwrap_or_default()
            .into_keys()
            .collect()
    }

    #[test]
    fn identical_content_hashes_identically_and_changed_content_differs() {
        let dir = tempfile::tempdir().unwrap();
        let a = sidecar(dir.path(), "a.col.json", br#"{"shapes":[]}"#);
        let b = sidecar(dir.path(), "b.col.json", br#"{"shapes":[]}"#);
        let c = sidecar(dir.path(), "c.col.json", br#"{"shapes":[1]}"#);
        let hasher = SidecarHasher::new(None, None);
        assert_eq!(hash(&hasher, &a), hash(&hasher, &b));
        assert_ne!(hash(&hasher, &a), hash(&hasher, &c));
    }

    #[test]
    fn the_key_component_is_32_hex_digits() {
        let component = SidecarHash::of_bytes(b"{}").key_component();
        assert_eq!(component.len(), 32);
        assert!(component.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_rewritten_file_with_the_same_bytes_keeps_its_hash_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"{\"shapes\":[]}");
        set_mtime(Path::new(&path), 1_000);
        let first = standalone(dir.path());
        let before = hash(&first, &path);
        first.flush();
        sidecar(dir.path(), "a.col.json", b"{\"shapes\":[]}");
        set_mtime(Path::new(&path), 2_000);
        assert_eq!(hash(&standalone(dir.path()), &path), before);
    }

    #[test]
    fn a_stamp_match_reuses_the_stored_hash_without_reading_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        set_mtime(Path::new(&path), 1_000);
        let first = standalone(dir.path());
        let original = hash(&first, &path);
        first.flush();
        sidecar(dir.path(), "a.col.json", b"BBBB");
        set_mtime(Path::new(&path), 1_000);
        assert_eq!(
            hash(&standalone(dir.path()), &path),
            original,
            "a file read would have returned the hash of the swapped bytes"
        );
    }

    #[test]
    fn a_changed_stamp_rehashes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        set_mtime(Path::new(&path), 1_000);
        let first = standalone(dir.path());
        let original = hash(&first, &path);
        first.flush();
        sidecar(dir.path(), "a.col.json", b"BBBB");
        set_mtime(Path::new(&path), 2_000);
        let second = standalone(dir.path());
        assert_ne!(hash(&second, &path), original);
        assert_eq!(hash(&second, &path), SidecarHash::of_bytes(b"BBBB"));
    }

    #[test]
    fn a_corrupt_index_falls_back_to_hashing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(index_file(dir.path()), b"not an index").unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        assert_eq!(
            hash(&standalone(dir.path()), &path),
            SidecarHash::of_bytes(b"AAAA")
        );
    }

    #[test]
    fn an_index_of_another_format_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        let SidecarStat::Stamped(stamp) = stat_sidecar(&path) else {
            panic!("the fixture has a stamp");
        };
        let stale = IndexFile {
            format: INDEX_FORMAT + 1,
            entries: vec![(path.clone(), IndexEntry { stamp, hash: 7 })],
        };
        write_index(&index_file(dir.path()), &stale).unwrap();
        assert_eq!(
            hash(&standalone(dir.path()), &path),
            SidecarHash::of_bytes(b"AAAA")
        );
    }

    #[test]
    fn a_missing_sidecar_is_absent_and_stays_so() {
        let dir = tempfile::tempdir().unwrap();
        let hasher = standalone(dir.path());
        let path = dir.path().join("late.col.json");
        let path = path.to_str().unwrap();
        assert_eq!(hasher.lookup(path), SidecarLookup::Absent);
        sidecar(dir.path(), "late.col.json", b"AAAA");
        assert_eq!(
            hasher.lookup(path),
            SidecarLookup::Absent,
            "absence is memoized for the run"
        );
    }

    #[test]
    fn an_unreadable_sidecar_is_not_memoized_so_the_next_lookup_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.col.json");
        std::fs::create_dir(&path).unwrap();
        let path = path.to_str().unwrap();
        let hasher = SidecarHasher::new(None, None);
        assert_eq!(hasher.lookup(path), SidecarLookup::Unreadable);
        std::fs::remove_dir(path).unwrap();
        sidecar(dir.path(), "a.col.json", b"AAAA");
        assert_eq!(
            hasher.lookup(path),
            SidecarLookup::Hashed(SidecarHash::of_bytes(b"AAAA"))
        );
    }

    #[test]
    fn without_a_usable_modification_time_there_is_no_stamp() {
        assert!(stamp_of(4, None).is_none());
        let before_epoch = SystemTime::UNIX_EPOCH - Duration::from_secs(1);
        assert!(stamp_of(4, Some(before_epoch)).is_none());
        let stamp = stamp_of(4, Some(SystemTime::UNIX_EPOCH + Duration::from_secs(2))).unwrap();
        assert_eq!((stamp.len, stamp.mtime_ns), (4, 2_000_000_000));
    }

    #[test]
    fn a_flush_leaves_only_the_index_file_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        let hasher = standalone(dir.path());
        hash(&hasher, &path);
        hasher.flush();
        assert!(index_file(dir.path()).exists());
        let temps = std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "tmp"))
            .count();
        assert_eq!(temps, 0);
    }

    #[test]
    fn dropping_the_hasher_persists_the_index() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        let hasher = standalone(dir.path());
        hash(&hasher, &path);
        drop(hasher);
        assert_eq!(stored_keys(dir.path()).len(), 1);
    }

    #[test]
    fn a_failed_flush_keeps_the_index_dirty_so_a_later_flush_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        let index_dir = dir.path().join("not-yet");
        let hasher = standalone(&index_dir);
        hash(&hasher, &path);
        hasher.flush();
        assert!(!index_file(&index_dir).exists());
        std::fs::create_dir(&index_dir).unwrap();
        hasher.flush();
        assert_eq!(stored_keys(&index_dir).len(), 1);
    }

    #[test]
    fn flushing_drops_entries_of_deleted_sidecars_but_keeps_unseen_live_ones() {
        let dir = tempfile::tempdir().unwrap();
        let kept = sidecar(dir.path(), "kept.col.json", b"AAAA");
        let gone = sidecar(dir.path(), "gone.col.json", b"BBBB");
        let other_run = sidecar(dir.path(), "other.col.json", b"CCCC");
        let first = standalone(dir.path());
        for path in [&kept, &gone, &other_run] {
            hash(&first, path);
        }
        first.flush();
        std::fs::remove_file(&gone).unwrap();
        let second = standalone(dir.path());
        hash(&second, &kept);
        let added = sidecar(dir.path(), "new.col.json", b"DDDD");
        hash(&second, &added);
        second.flush();
        let keys = stored_keys(dir.path());
        assert_eq!(keys.len(), 3, "{keys:?}");
        assert!(keys.iter().all(|key| !key.ends_with("gone.col.json")));
        assert!(keys.iter().any(|key| key.ends_with("other.col.json")));
    }

    #[test]
    fn the_index_stores_paths_relative_to_the_base_and_survives_moving_it() {
        let install = tempfile::tempdir().unwrap();
        let index_dir = tempfile::tempdir().unwrap();
        let path = sidecar(install.path(), "a.col.json", b"AAAA");
        set_mtime(Path::new(&path), 1_000);
        let first = SidecarHasher::new(Some(index_dir.path()), Some(install.path()));
        let original = hash(&first, &path);
        first.flush();
        assert_eq!(stored_keys(index_dir.path()), vec!["a.col.json"]);

        let moved = tempfile::tempdir().unwrap();
        let moved_path = sidecar(moved.path(), "a.col.json", b"BBBB");
        set_mtime(Path::new(&moved_path), 1_000);
        let second = SidecarHasher::new(Some(index_dir.path()), Some(moved.path()));
        assert_eq!(
            hash(&second, &moved_path),
            original,
            "same relative path and stamp under the new base is an index hit"
        );
    }

    #[test]
    fn the_index_file_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<String> = ["c", "a", "b"]
            .iter()
            .map(|name| sidecar(dir.path(), &format!("{name}.col.json"), name.as_bytes()))
            .collect();
        let index_dir = tempfile::tempdir().unwrap();
        let hasher = standalone(index_dir.path());
        for path in &paths {
            hash(&hasher, path);
        }
        hasher.flush();
        let first = std::fs::read(index_file(index_dir.path())).unwrap();
        hasher.dirty.store(true, Ordering::Release);
        hasher.flush();
        assert_eq!(first, std::fs::read(index_file(index_dir.path())).unwrap());
    }

    #[test]
    fn the_memo_hashes_a_path_once_per_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = sidecar(dir.path(), "a.col.json", b"AAAA");
        let hasher = SidecarHasher::new(None, None);
        let first = hash(&hasher, &path);
        sidecar(dir.path(), "a.col.json", b"BBBB");
        assert_eq!(hash(&hasher, &path), first);
    }
}
