//! Removal of collider-cache data left behind by older cache versions.
//!
//! The cache lives in `cache/physics/v<N>/`. Bumping the version orphans the
//! old directory (and, before versioned directories existed, loose blobs
//! directly in `cache/physics/`); nothing reads them again, so they are deleted
//! on a background thread, never on the spawn path. A newer build therefore
//! purges the directories of older builds: alternating between builds of
//! different cache versions re-bakes every time.

use std::fs::DirEntry;
use std::path::Path;
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime};

/// Temp files younger than this may belong to a live writer (this or another
/// process sharing the directory) and are left alone.
const TEMP_FILE_GRACE: Duration = Duration::from_mins(1);
/// Deletions between voluntary yields, on top of the lowered thread priority.
const REMOVALS_PER_YIELD: usize = 64;
const YIELD_PAUSE: Duration = Duration::from_millis(1);

pub fn versioned_dir_name(version: u32) -> String {
    format!("v{version}")
}

fn version_of_dir_name(name: &str) -> Option<u32> {
    name.strip_prefix('v')?.parse().ok()
}

/// Starts the purge of everything stale under `physics_root` for
/// `current_version`, plus the leftover temp files of `current_dir`, on one
/// named, low-priority thread. The caller may drop the handle to detach it.
pub fn spawn_purge(
    physics_root: &Path,
    current_version: u32,
    current_dir: &Path,
) -> Option<JoinHandle<()>> {
    let physics_root = physics_root.to_owned();
    let current_dir = current_dir.to_owned();
    let spawned = std::thread::Builder::new()
        .name("collider_cache_purge".into())
        .spawn(move || {
            crate::utils::lower_thread_priority();
            purge_stale_versions(&physics_root, current_version);
            sweep_stale_temps(&current_dir);
        });
    match spawned {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("collision cache: can't start the purge thread: {e}");
            None
        }
    }
}

/// Removes `v<M>` directories with `M < current_version` and loose
/// `*.colbin` / stale `*.tmp` files directly in `physics_root`. Directories of
/// the current or a newer version are never touched.
pub fn purge_stale_versions(physics_root: &Path, current_version: u32) {
    let mut removals = Throttle::default();
    for entry in listed_entries(physics_root) {
        let path = entry.path();
        let Some(is_dir) = is_directory(&entry) else {
            continue;
        };
        if is_dir {
            let stale = entry
                .file_name()
                .to_str()
                .and_then(version_of_dir_name)
                .is_some_and(|version| version < current_version);
            if stale {
                remove_dir_throttled(&path, &mut removals);
            }
        } else if has_extension(&path, "colbin")
            || (has_extension(&path, "tmp") && is_older_than_grace(&path))
        {
            remove_file_logged(&path, &mut removals);
        }
    }
}

/// Removes `*.tmp` files a crashed run left in `dir`, skipping recent ones.
pub fn sweep_stale_temps(dir: &Path) {
    let mut removals = Throttle::default();
    for entry in listed_entries(dir) {
        let path = entry.path();
        if has_extension(&path, "tmp") && is_older_than_grace(&path) {
            remove_file_logged(&path, &mut removals);
        }
    }
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension().is_some_and(|ext| ext == extension)
}

/// Entries of `dir`, logging (and skipping) what can't be listed. A directory
/// that does not exist is simply empty.
fn listed_entries(dir: &Path) -> impl Iterator<Item = DirEntry> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => Some(entries),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            tracing::warn!("collision cache: can't list {}: {e}", dir.display());
            None
        }
    };
    let dir = dir.to_owned();
    entries
        .into_iter()
        .flatten()
        .filter_map(move |entry| match entry {
            Ok(entry) => Some(entry),
            Err(e) => {
                tracing::warn!(
                    "collision cache: can't read an entry of {}: {e}",
                    dir.display()
                );
                None
            }
        })
}

fn is_directory(entry: &DirEntry) -> Option<bool> {
    match entry.file_type() {
        Ok(kind) => Some(kind.is_dir()),
        Err(e) => {
            tracing::warn!(
                "collision cache: can't determine the type of {}: {e}",
                entry.path().display()
            );
            None
        }
    }
}

/// Whether `path` is old enough that no live writer can still own it. Anything
/// that can't be determined counts as young: keeping a file is the safe error.
fn is_older_than_grace(path: &Path) -> bool {
    let modified = match std::fs::metadata(path).and_then(|meta| meta.modified()) {
        Ok(modified) => modified,
        Err(e) => {
            tracing::warn!(
                "collision cache: can't read the age of {}: {e}; keeping it",
                path.display()
            );
            return false;
        }
    };
    match SystemTime::now().duration_since(modified) {
        Ok(age) => age >= TEMP_FILE_GRACE,
        Err(e) => {
            tracing::warn!(
                "collision cache: {} is dated in the future ({e}); keeping it",
                path.display()
            );
            false
        }
    }
}

#[derive(Default)]
struct Throttle(usize);

impl Throttle {
    fn tick(&mut self) {
        self.0 += 1;
        if self.0.is_multiple_of(REMOVALS_PER_YIELD) {
            std::thread::sleep(YIELD_PAUSE);
        }
    }
}

fn remove_file_logged(path: &Path, throttle: &mut Throttle) {
    if let Err(e) = std::fs::remove_file(path) {
        tracing::warn!("collision cache: can't remove {}: {e}", path.display());
    }
    throttle.tick();
}

fn remove_dir_throttled(dir: &Path, throttle: &mut Throttle) {
    for entry in listed_entries(dir) {
        let path = entry.path();
        match is_directory(&entry) {
            Some(true) => remove_dir_throttled(&path, throttle),
            Some(false) => remove_file_logged(&path, throttle),
            None => {}
        }
    }
    if let Err(e) = std::fs::remove_dir(dir) {
        tracing::warn!(
            "collision cache: can't remove directory {}: {e}",
            dir.display()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sim::physics::test_support::write_file;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    fn age(path: &Path) {
        std::fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(SystemTime::now() - 2 * TEMP_FILE_GRACE)
            .unwrap();
    }

    #[test]
    fn purge_removes_older_version_dirs_and_legacy_files_only() {
        let root = tempfile::tempdir().unwrap();
        touch(&root.path().join("v1/a.colbin"));
        touch(&root.path().join("v2/b.colbin"));
        touch(&root.path().join("v3/c.colbin"));
        touch(&root.path().join("legacy.colbin"));
        touch(&root.path().join("legacy.1.2.tmp"));
        age(&root.path().join("legacy.1.2.tmp"));
        touch(&root.path().join("notes.txt"));
        touch(&root.path().join("vault/d.colbin"));
        purge_stale_versions(root.path(), 2);
        assert!(!root.path().join("v1").exists());
        assert!(root.path().join("v2/b.colbin").exists(), "current kept");
        assert!(root.path().join("v3/c.colbin").exists(), "newer kept");
        assert!(!root.path().join("legacy.colbin").exists());
        assert!(!root.path().join("legacy.1.2.tmp").exists());
        assert!(root.path().join("notes.txt").exists());
        assert!(
            root.path().join("vault/d.colbin").exists(),
            "non-version dir kept"
        );
    }

    #[test]
    fn purge_keeps_a_recent_loose_temp_file_that_a_live_writer_may_own() {
        let root = tempfile::tempdir().unwrap();
        touch(&root.path().join("live.9.0.tmp"));
        purge_stale_versions(root.path(), 2);
        assert!(root.path().join("live.9.0.tmp").exists());
    }

    #[test]
    fn purge_of_a_missing_root_is_a_no_op() {
        let root = tempfile::tempdir().unwrap();
        purge_stale_versions(&root.path().join("absent"), 2);
    }

    #[test]
    fn temp_sweep_removes_old_temp_files_but_not_recent_ones_or_blobs() {
        let dir = tempfile::tempdir().unwrap();
        let old = write_file(dir.path(), "old.1.0.tmp", b"x");
        let recent = write_file(dir.path(), "recent.2.0.tmp", b"x");
        let blob = write_file(dir.path(), "0000000000000001.colbin", b"x");
        age(&old);
        sweep_stale_temps(dir.path());
        assert!(!old.exists());
        assert!(recent.exists());
        assert!(blob.exists());
    }

    #[test]
    fn the_spawned_purge_is_joinable_and_purges_the_given_root() {
        let root = tempfile::tempdir().unwrap();
        touch(&root.path().join("v1/a.colbin"));
        touch(&root.path().join("v2/b.colbin"));
        spawn_purge(root.path(), 2, &root.path().join("v2"))
            .unwrap()
            .join()
            .unwrap();
        assert!(!root.path().join("v1").exists());
        assert!(root.path().join("v2/b.colbin").exists());
    }

    #[test]
    fn version_names_round_trip() {
        assert_eq!(version_of_dir_name(&versioned_dir_name(7)), Some(7));
        assert_eq!(version_of_dir_name("vault"), None);
        assert_eq!(version_of_dir_name("7"), None);
    }
}
