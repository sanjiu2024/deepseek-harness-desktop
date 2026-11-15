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
        let titles = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<BTreeMap<String, String>>(raw.trim_start_matches('\u{feff}')).ok())
            .unwrap_or_default();
        Self { path, titles }
    }

    pub fn get(&self, id: &str) -> Option<&str> {
        self.titles.get(id).map(String::as_str)
    }

    /// Set or clear a rename. An empty title removes the override, restoring the derived one.
    pub fn set(&mut self, id: &str, title: &str) -> Result<(), String> {
        let title = title.trim();
        if title.is_empty() {
            self.titles.remove(id);
        } else {
            self.titles.insert(id.to_owned(), title.to_owned());
        }
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        let body = serde_json::to_string_pretty(&self.titles).map_err(|error| error.to_string())?;
        std::fs::write(&self.path, body).map_err(|error| format!("{}: {error}", self.path.display()))
    }
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
