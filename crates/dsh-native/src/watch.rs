//! Knowing that a directory changed, without asking the OS for events.
//!
//! The side panel shows **one directory level at a time**, so the thing being watched is small.
//! For that, polling a snapshot every second and a half is the better trade than a native event
//! backend: no per-platform quirks (inotify, FSEvents, ReadDirectoryChangesW all behave
//! differently), no dependency, and — the part that matters for a project that verifies its
//! claims — a diff function that is a pure function and therefore testable without racing a
//! filesystem.
//!
//! The cost is bounded and known: `n` entries of one directory, read every interval. A directory
//! with a hundred thousand files would be a problem, and that is exactly the case `fs_tree` refuses
//! anyway.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// What one entry looked like at one moment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// Modification time, milliseconds since the epoch. `None` when the platform cannot say.
    pub modified: Option<i64>,
    pub size: u64,
    pub is_dir: bool,
}

/// A directory level, as seen at one moment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    entries: BTreeMap<PathBuf, Stamp>,
}

impl Snapshot {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Paths that differ between two snapshots: added, removed, or modified.
    ///
    /// Renames show up as one removal plus one addition, which is what a tree needs to redraw
    /// anyway. The order is path order, so a caller's output is stable.
    pub fn changed_since(&self, previous: &Snapshot) -> Vec<PathBuf> {
        let mut changed = BTreeSet::new();
        for (path, stamp) in &self.entries {
            match previous.entries.get(path) {
                Some(before) if before == stamp => {}
                _ => {
                    changed.insert(path.clone());
                }
            }
        }
        for path in previous.entries.keys() {
            if !self.entries.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        changed.into_iter().collect()
    }
}

/// Read one directory level.
///
/// Unreadable entries are skipped rather than reported: a file that vanished mid-scan is not an
/// error, it is a change — and the next snapshot will show it as a removal.
pub fn scan(directory: &Path) -> Snapshot {
    let mut entries = BTreeMap::new();
    let Ok(listing) = std::fs::read_dir(directory) else {
        return Snapshot { entries };
    };
    for entry in listing.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        entries.insert(
            path,
            Stamp {
                modified: metadata.modified().ok().and_then(millis),
                size: metadata.len(),
                is_dir: metadata.is_dir(),
            },
        );
    }
    Snapshot { entries }
}

fn millis(time: SystemTime) -> Option<i64> {
    time.duration_since(SystemTime::UNIX_EPOCH)
        .ok()
        .map(|delta| delta.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-watch-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn an_unchanged_directory_reports_nothing() {
        let root = scratch("same");
        std::fs::write(root.join("a.txt"), b"one").expect("write");
        let before = scan(&root);
        let after = scan(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert!(after.changed_since(&before).is_empty());
    }

    #[test]
    fn a_new_file_shows_up() {
        let root = scratch("added");
        std::fs::write(root.join("a.txt"), b"one").expect("write");
        let before = scan(&root);
        std::fs::write(root.join("b.txt"), b"two").expect("write");
        let after = scan(&root);
        let changed = after.changed_since(&before);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(changed, vec![root.join("b.txt")]);
    }

    #[test]
    fn a_deleted_file_shows_up_even_though_it_is_absent() {
        // The interesting half: absence cannot be seen by looking at the new listing alone.
        let root = scratch("removed");
        std::fs::write(root.join("a.txt"), b"one").expect("write");
        std::fs::write(root.join("b.txt"), b"two").expect("write");
        let before = scan(&root);
        std::fs::remove_file(root.join("a.txt")).expect("remove");
        let after = scan(&root);
        let changed = after.changed_since(&before);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(changed, vec![root.join("a.txt")]);
    }

    #[test]
    fn a_rewritten_file_shows_up() {
        let root = scratch("modified");
        let path = root.join("a.txt");
        std::fs::write(&path, b"one").expect("write");
        let before = scan(&root);
        std::fs::write(&path, b"a longer body").expect("rewrite");
        let after = scan(&root);
        let changed = after.changed_since(&before);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(changed, vec![path.clone()]);
    }

    #[test]
    fn a_missing_directory_is_an_empty_snapshot_not_a_panic() {
        let snapshot = scan(Path::new("this/does/not/exist"));
        assert!(snapshot.is_empty());
        // And two empty snapshots agree, so a watcher on a removed directory is quiet.
        assert!(snapshot.changed_since(&Snapshot::default()).is_empty());
    }
}
