//! The client's own metadata about sessions.
//!
//! The kernel owns the session logs: it names them, writes them, and would not thank a
//! second writer. But a user needs to rename a conversation and delete one, and neither is
//! an SDK call. So this file holds *only* what belongs to the client — a title overlay — and
//! deleting a session removes the kernel's directory while leaving its format untouched.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Filename inside the kernel home. Not inside `sessions/`: that tree belongs to the kernel
/// and anything we drop there could be mistaken for a session.
const OVERLAY: &str = "session-titles.json";

/// Renames the user has applied, keyed by session id.
#[derive(Debug, Default, Clone)]
pub struct SessionTitles {
    path: PathBuf,
    titles: BTreeMap<String, String>,
}

impl SessionTitles {
    /// Load the overlay from a kernel home. A missing or corrupt file is an empty overlay:
    /// losing a rename is annoying, failing to start is worse.
    pub fn load(home: &Path) -> Self {
        let path = home.join(OVERLAY);
        let titles = read_overlay(&path);
        Self { path, titles }
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.titles.get(id).map(String::as_str)
    }

    /// Set or clear a rename. An empty title removes the override, restoring the derived one.
    ///
    /// Serialised process-wide: `set` is a read-modify-write of a shared file, and two Tauri
    /// commands (rename in one window, delete in another) would otherwise each write a map that
    /// is missing the other's change.
    pub fn set(&mut self, id: &str, title: &str) -> Result<(), String> {
        let _guard = overlay_lock();
        // Re-read under the lock: this instance's map may predate the other writer's change.
        self.titles = read_overlay(&self.path);
        let title = title.trim();
        if title.is_empty() {
            self.titles.remove(id);
        } else {
            self.titles.insert(id.to_owned(), title.to_owned());
        }
        self.save()
    }

    /// Persist the overlay atomically.
    ///
    /// A whole-file replace of a file that is read back as one JSON document: a half-written
    /// overlay is a file that fails to parse and silently loses **every** rename.
    fn save(&self) -> Result<(), String> {
        let body = serde_json::to_string_pretty(&self.titles).map_err(|error| error.to_string())?;
        crate::fsutil::write_atomic(&self.path, body.as_bytes())
            .map_err(|error| format!("{}: {error}", self.path.display()))
    }
}

fn read_overlay(path: &Path) -> BTreeMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| {
            serde_json::from_str::<BTreeMap<String, String>>(raw.trim_start_matches('\u{feff}')).ok()
        })
        .unwrap_or_default()
}

/// One writer at a time for the overlay file, for the life of the process.
fn overlay_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dsh-titles-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn renames_survive_a_reload() {
        let home = scratch("persist");
        let mut titles = SessionTitles::load(&home);
        titles.set("a", "我的会话").expect("set");

        let reloaded = SessionTitles::load(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(reloaded.get("a"), Some("我的会话"));
    }

    #[test]
    fn an_empty_title_restores_the_derived_one() {
        let home = scratch("clear");
        let mut titles = SessionTitles::load(&home);
        titles.set("a", "临时").expect("set");
        titles.set("a", "  ").expect("clear");

        let reloaded = SessionTitles::load(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(reloaded.get("a"), None);
    }

    #[test]
    fn a_corrupt_overlay_is_empty_rather_than_fatal() {
        let home = scratch("corrupt");
        std::fs::write(home.join(OVERLAY), "{ not json").expect("write");
        let titles = SessionTitles::load(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(titles.get("a"), None);
    }
}

#[cfg(test)]
mod concurrency_tests {
    use super::*;

    #[test]
    fn two_writers_do_not_lose_each_others_rename() {
        // `set` is a read-modify-write of a shared file. Without the lock and the re-read, the
        // second writer's map — built before the first writer saved — would erase the first one.
        let home = std::env::temp_dir().join(format!("dsh-titles-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).expect("scratch");

        let mut first = SessionTitles::load(&home);
        let mut second = SessionTitles::load(&home);
        first.set("a", "甲").expect("first");
        second.set("b", "乙").expect("second");

        let merged = SessionTitles::load(&home);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(merged.get("a"), Some("甲"), "the first rename survived");
        assert_eq!(merged.get("b"), Some("乙"));
    }
}
