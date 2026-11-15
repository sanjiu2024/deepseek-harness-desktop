//! Reads the kernel's session logs.
//!
//! The SDK wire protocol has no "list sessions" call and `session/resume` does not
//! replay history — it only reports events committed after resume. History
//! therefore comes from disk, and this crate is the only place in the project that
//! knows the layout.
//!
//! ```text
//! <DSH_HOME>/sessions/
//!   --<normalized-cwd>--/
//!     <encoded-session-id>/
//!       session.v4.jsonl         # current generation, plain NDJSON (see profile patch)
//!       session.v4.jsonl.zstd    # same, compressed — upstream default
//! ```
//!
//! We read; we never write. That is why this crate does not reproduce upstream's
//! v0→v4 migration chain: an unrecognised `version` is reported as unreadable
//! rather than translated.

use std::path::{Path, PathBuf};

mod export;
pub mod fsutil;
mod titles;

pub use export::render_markdown;
pub use fsutil::write_atomic;
pub use titles::SessionTitles;

/// The session format generation this build understands.
pub const CURRENT_FORMAT_VERSION: u32 = 4;

/// Directory under `DSH_HOME` holding every session.
pub const SESSIONS_DIR: &str = "sessions";

/// The first line of a session log.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SessionHeader {
    /// Format generation. Anything but [`CURRENT_FORMAT_VERSION`] is unreadable here.
    pub version: u32,
    pub id: String,
    /// Milliseconds since the Unix epoch.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(default)]
    pub cwd: Option<String>,
    /// Set when this session was spawned by a subagent tool.
    #[serde(default, rename = "parentSession")]
    pub parent_session: Option<String>,
    #[serde(default, rename = "isSeeded")]
    pub is_seeded: bool,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default, rename = "delegationDepth")]
    pub delegation_depth: u32,
    #[serde(default, rename = "agentPreset")]
    pub agent_preset: Option<String>,
}

/// One durable event row.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SessionEvent {
    /// Event kind, e.g. `user/message`, `assistant/chunk`, `tool/call`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Monotonic sequence within the session.
    pub seq: i64,
    /// Milliseconds since the Unix epoch.
    pub time: i64,
    pub data: serde_json::Value,
}

/// What a session list row needs, without the body.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    /// `session/title`, or the first line of the user's first message.
    pub title: String,
    pub cwd: Option<String>,
    pub created_at: i64,
    /// Last event time; falls back to the header's creation time.
    pub updated_at: i64,
    pub event_count: usize,
    /// True when the session was spawned by a subagent tool.
    pub is_subagent: bool,
    /// The session that spawned this one, when it was a subagent run.
    pub parent_id: Option<String>,
}

/// One session's complete history.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistory {
    pub header: SessionHeader,
    pub events: Vec<SessionEvent>,
}

/// One match inside a session.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    pub session_id: String,
    pub title: String,
    /// Sequence of the matching event, so the UI can open the session at it later.
    pub seq: i64,
    pub time: i64,
    /// A window of text around the match, trimmed at the edges.
    pub snippet: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unreadable session format v{version} at {path}")]
    UnsupportedFormat { version: u32, path: String },
    #[error("session log has no header: {0}")]
    MissingHeader(String),
    #[error("no session with id {0}")]
    NotFound(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("session titles: {0}")]
    Overlay(String),
}

/// Read-only view of a session root.
#[derive(Debug, Clone)]
pub struct SessionStore {
    root: PathBuf,
}

impl SessionStore {
    /// Point the store at a `DSH_HOME` (the `sessions` directory is derived).
    pub fn new(dsh_home: impl AsRef<Path>) -> Self {
        Self {
            root: dsh_home.as_ref().join(SESSIONS_DIR),
        }
    }

    /// Every readable session, newest first.
    ///
    /// Unreadable entries are skipped rather than failing the listing: one corrupt
    /// log must not hide the other fifty.
    pub fn list(&self) -> Result<Vec<SessionSummary>, StoreError> {
        let titles = SessionTitles::load(self.home());
        let mut out = Vec::new();
        for path in self.logs()? {
            match read_header(&path) {
                Ok(header) => {
                    let mut summary = summarize_cheap(&header, &path);
                    if let Some(renamed) = titles.get(&summary.id) {
                        summary.title = renamed.to_owned();
                    }
                    out.push(summary);
                }
                Err(error) => {
                    tracing::debug!(target: "dsh_store", %error, path = %path.display(), "skipping unreadable session");
                }
            }
        }
        out.sort_by_key(|session| std::cmp::Reverse(session.updated_at));
        Ok(out)
    }

    /// One session's header and every event, in file order.
    pub fn load(&self, id: &str) -> Result<SessionHistory, StoreError> {
        let mut fallback: Option<StoreError> = None;
        for path in self.logs()? {
            match read_session(&path) {
                Ok((header, events)) if header.id == id => {
                    return Ok(SessionHistory { header, events });
                }
                Ok(_) => {}
                Err(error) => fallback = fallback.or(Some(error)),
            }
        }
        Err(fallback.unwrap_or_else(|| StoreError::NotFound(id.to_owned())))
    }

    /// Search the text of every session for a substring.
    ///
    /// Deliberately a scan, not an index. There are tens of sessions, not millions of documents:
    /// an index would be a second thing to keep in sync with files the kernel writes behind our
    /// back, and it would go stale the moment a session is deleted. If this ever gets slow, the
    /// fix is an index **built from** these logs, not a different source of truth.
    pub fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchHit>, StoreError> {
      let needle = query.trim().to_lowercase();
      if needle.is_empty() {
        return Ok(Vec::new());
      }
      let titles = SessionTitles::load(self.home());
      let mut hits = Vec::new();

      for path in self.logs()? {
        // Stop the whole scan once the limit is met: continuing to open and parse the remaining
        // transcripts cannot change the answer, and the caller asked for a page, not for every
        // match in the archive.
        if hits.len() >= limit {
          break;
        }
        let Ok((header, events)) = read_session(&path) else {
          continue;
        };
        let title = titles
          .get(&header.id)
          .map(str::to_owned)
          .unwrap_or_else(|| summarize(&header, &events).title);

        for event in &events {
          if hits.len() >= limit {
            break;
          }
          let text = match event.kind.as_str() {
            // The user's own words and the model's answer. Tool traffic is machine text:
            // matching it would surface a session because a path happens to contain the term.
            "user/message" => content_text(&event.data),
            "assistant/message" => event
              .data
              .get("message")
              .and_then(content_text)
              .or_else(|| content_text(&event.data)),
            _ => None,
          };
          let Some(text) = text else {
            continue;
          };
          if let Some(snippet) = snippet_around(&text, &needle) {
            hits.push(SearchHit {
              session_id: header.id.clone(),
              title: title.clone(),
              seq: event.seq,
              time: event.time,
              snippet,
            });
          }
        }
      }

      hits.sort_by_key(|hit| std::cmp::Reverse(hit.time));
      // (the hit limit is enforced above; this comment marks where the scan stops)
      Ok(hits)
    }

    /// Does a session with this id exist on disk?
    ///
    /// Reads headers only. Prompting an old session needs this answer on every first message,
    /// and parsing a 30 MB transcript to learn "yes" would make reopening slow for no reason.
    pub fn contains(&self, id: &str) -> bool {
        let Ok(paths) = self.logs() else {
            return false;
        };
        paths.iter().any(|path| {
            std::fs::read_to_string(path)
                .ok()
                .and_then(|raw| raw.lines().next().map(str::to_owned))
                .and_then(|first| serde_json::from_str::<SessionHeader>(&first).ok())
                .is_some_and(|header| header.id == id)
        })
    }

    /// The kernel home this store was opened against.
    pub fn home(&self) -> &Path {
        self.root.parent().unwrap_or(&self.root)
    }

    /// Give a session a user-chosen title.
    ///
    /// The kernel's log is not rewritten: the title is stored in the client's own overlay
    /// and wins over the derived one at read time. A rename therefore survives the kernel
    /// appending more events, and reverts cleanly.
    pub fn rename(&self, id: &str, title: &str) -> Result<(), StoreError> {
        let mut titles = SessionTitles::load(self.home());
        titles.set(id, title).map_err(StoreError::Overlay)?;
        Ok(())
    }

    /// Move a session out of the store, into `trash/`.
    ///
    /// A deleted conversation is somebody's work, and "delete" in a UI is one keystroke away
    /// from "I clicked the wrong row". Moving the directory keeps the decision recoverable:
    /// the session disappears from every list, and the bytes are still there for whoever knows
    /// where to look. `purge` is the irreversible one.
    pub fn delete(&self, id: &str) -> Result<(), StoreError> {
        let dir = self.directory_of(id)?;
        let trash = self.home().join("trash");
        std::fs::create_dir_all(&trash)?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|delta| delta.as_millis())
            .unwrap_or_default();
        // The stamp keeps two deletions of the same id apart, and survives a restore.
        let target = trash.join(format!("{id}-{stamp}"));
        std::fs::rename(&dir, &target)?;

        // A rename would otherwise outlive the session it described.
        let mut titles = SessionTitles::load(self.home());
        titles.set(id, "").map_err(StoreError::Overlay)?;
        Ok(())
    }

    /// Remove a session for good. Nothing recovers this.
    pub fn purge(&self, id: &str) -> Result<(), StoreError> {
        let dir = self.directory_of(id)?;
        std::fs::remove_dir_all(dir)?;
        let mut titles = SessionTitles::load(self.home());
        titles.set(id, "").map_err(StoreError::Overlay)?;
        Ok(())
    }

    /// The directory holding a session's log.
    fn directory_of(&self, id: &str) -> Result<PathBuf, StoreError> {
        for path in self.logs()? {
            let Ok((header, _)) = read_session(&path) else {
                continue;
            };
            if header.id == id {
                return path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .ok_or_else(|| StoreError::NotFound(id.to_owned()));
            }
        }
        Err(StoreError::NotFound(id.to_owned()))
    }

    /// Canonical log files, newest generation per session directory.
    fn logs(&self) -> Result<Vec<PathBuf>, StoreError> {
        let mut out = Vec::new();
        let projects = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            // An empty installation is not an error: it is a first launch.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(error) => return Err(StoreError::Io(error)),
        };

        for project in projects.flatten() {
            let Ok(sessions) = std::fs::read_dir(project.path()) else {
                continue;
            };
            for session in sessions.flatten() {
                if let Some(path) = newest_log(&session.path()) {
                    out.push(path);
                }
            }
        }
        Ok(out)
    }
}

/// The highest-generation plain-JSONL log in one session directory.
///
/// Compressed logs are skipped: the profile patch turns compression off so the
/// client can read them directly, and decoding zstd would be a second parser to
/// keep correct. A session that is only compressed is simply absent from the list.
fn newest_log(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(u32, PathBuf)> = None;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(version) = parse_log_name(&name) else {
            continue;
        };
        if best.as_ref().is_none_or(|(current, _)| version > *current) {
            best = Some((version, entry.path()));
        }
    }
    best.map(|(_, path)| path)
}

/// `session.jsonl` is v0, `session.vN.jsonl` is vN. Compression suffixes are not
/// canonical and are rejected here.
fn parse_log_name(name: &str) -> Option<u32> {
    if name == "session.jsonl" {
        return Some(0);
    }
    let rest = name.strip_prefix("session.v")?.strip_suffix(".jsonl")?;
    rest.parse::<u32>().ok()
}

/// Read just the first line.
fn read_header(path: &Path) -> Result<SessionHeader, StoreError> {
    let raw = std::fs::read_to_string(path)?;
    let first = raw
        .lines()
        .next()
        .ok_or_else(|| StoreError::MissingHeader(path.display().to_string()))?;
    let header: SessionHeader = serde_json::from_str(first)?;
    if header.version != CURRENT_FORMAT_VERSION {
        return Err(StoreError::UnsupportedFormat {
            version: header.version,
            path: path.display().to_string(),
        });
    }
    Ok(header)
}

fn read_session(path: &Path) -> Result<(SessionHeader, Vec<SessionEvent>), StoreError> {
    let raw = std::fs::read_to_string(path)?;
    let mut lines = raw.lines().filter(|line| !line.trim().is_empty());

    let first = lines
        .next()
        .ok_or_else(|| StoreError::MissingHeader(path.display().to_string()))?;
    let header: SessionHeader = serde_json::from_str(first)?;
    if header.version != CURRENT_FORMAT_VERSION {
        return Err(StoreError::UnsupportedFormat {
            version: header.version,
            path: path.display().to_string(),
        });
    }

    let mut events = Vec::new();
    for line in lines {
        // A torn final line is normal after a crash; everything before it counts.
        match serde_json::from_str::<SessionEvent>(line) {
            Ok(event) => events.push(event),
            Err(error) => {
                tracing::debug!(target: "dsh_store", %error, "skipping a malformed event row");
            }
        }
    }
    Ok((header, events))
}

/// A summary is built without parsing the transcript.
///
/// Listing forty sessions used to mean deserialising every event in every file — the listing
/// cost grew with the *content* instead of the count. Two changes: only lines whose text
/// contains a marker are parsed at all, and "updated at" comes from the file's mtime, which is
/// what the file system already knows.
fn summarize_cheap(header: &SessionHeader, path: &Path) -> SessionSummary {
    let mut title: Option<String> = None;
    let mut updated_at = header.created_at;
    let mut last_line = String::new();
    let mut first_user: Option<String> = None;
    let mut event_count = 0;

    if let Ok(raw) = std::fs::read_to_string(path) {
        for line in raw.lines().skip(1) {
            if line.trim().is_empty() {
                continue;
            }
            event_count += 1;
            // Substring first, JSON second: most lines are neither of these two kinds.
            if title.is_none()
                && line.contains("session/title")
                && let Ok(event) = serde_json::from_str::<SessionEvent>(line)
            {
                title = event
                    .data
                    .get("title")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
            }
            last_line.clear();
            last_line.push_str(line);
            if first_user.is_none()
                && line.contains("user/message")
                && let Ok(event) = serde_json::from_str::<SessionEvent>(line)
            {
                let authored = event
                    .data
                    .get("source")
                    .and_then(|source| source.get("kind"))
                    .and_then(serde_json::Value::as_str)
                    .is_none_or(|kind| kind == "user");
                if authored {
                    first_user = content_text(&event.data)
                        .map(|text| text.trim().to_owned())
                        .filter(|text| !text.is_empty());
                }
            }
        }
    }

    // The last row's timestamp is the honest "updated at": file mtime moves when anything
    // touches the file, including a tool we have no interest in.
    if let Ok(event) = serde_json::from_str::<SessionEvent>(&last_line) {
        updated_at = updated_at.max(event.time);
    }

    let title = title
        .or(first_user)
        .unwrap_or_else(|| header.id.clone());

    SessionSummary {
        id: header.id.clone(),
        title: shorten(&title, 80),
        cwd: header.cwd.clone(),
        created_at: header.created_at,
        updated_at,
        event_count,
        is_subagent: header.origin.as_deref() == Some("subagent")
            || header.parent_session.is_some(),
        parent_id: header.parent_session.clone(),
    }
}

fn summarize(header: &SessionHeader, events: &[SessionEvent]) -> SessionSummary {
    let title = events
        .iter()
        .find(|event| event.kind == "session/title")
        .and_then(|event| event.data.get("title"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| first_user_text(events))
        .unwrap_or_else(|| header.id.clone());

    let updated_at = events
        .iter()
        .map(|event| event.time)
        .max()
        .unwrap_or(header.created_at);

    SessionSummary {
        id: header.id.clone(),
        title: shorten(&title, 80),
        cwd: header.cwd.clone(),
        created_at: header.created_at,
        updated_at,
        event_count: events.len(),
        is_subagent: header.origin.as_deref() == Some("subagent")
            || header.parent_session.is_some(),
        parent_id: header.parent_session.clone(),
    }
}

/// The user's own first message, ignoring the injected context blocks.
fn first_user_text(events: &[SessionEvent]) -> Option<String> {
    for event in events {
        if event.kind != "user/message" {
            continue;
        }
        let authored = event
            .data
            .get("source")
            .and_then(|source| source.get("kind"))
            .and_then(serde_json::Value::as_str)
            .is_none_or(|kind| kind == "user");
        if !authored {
            continue;
        }
        if let Some(text) = content_text(&event.data) {
            let text = text.trim();
            if !text.is_empty() {
                return Some(text.to_owned());
            }
        }
    }
    None
}

fn content_text(data: &serde_json::Value) -> Option<String> {
    let content = data.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.to_owned());
    }
    let blocks = content.as_array()?;
    let joined: String = blocks
        .iter()
        .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
        .collect();
    (!joined.is_empty()).then_some(joined)
}

/// A readable window around the first case-insensitive occurrence of `needle`.
///
/// The window is character-based, not byte-based: slicing bytes at a fixed offset would cut a
/// multi-byte character in half and produce a replacement glyph in the middle of Chinese text.
fn snippet_around(text: &str, needle: &str) -> Option<String> {
    let lowered = text.to_lowercase();
    let at = lowered.find(needle)?;
    // Byte offset in the lowered string is not the byte offset in the original when lowercase
    // changes length (rare, but 'İ' does). Fall back to the flattened text in that case.
    let source: Vec<char> = if lowered.len() == text.len() {
        text.chars().collect()
    } else {
        lowered.chars().collect()
    };
    let index = lowered[..at].chars().count();
    let start = index.saturating_sub(24);
    let end = (index + needle.chars().count() + 48).min(source.len());
    let mut window: String = source[start..end].iter().collect();
    if start > 0 {
        window.insert(0, '…');
    }
    if end < source.len() {
        window.push('…');
    }
    Some(window.replace('\n', " "))
}

fn shorten(text: &str, limit: usize) -> String {
    let first_line = text.lines().next().unwrap_or(text).trim();
    if first_line.chars().count() <= limit {
        return first_line.to_owned();
    }
    let cut: String = first_line.chars().take(limit).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_session(root: &Path, project: &str, dir: &str, lines: &[&str]) {
        let path = root.join(SESSIONS_DIR).join(project).join(dir);
        std::fs::create_dir_all(&path).expect("mkdir");
        std::fs::write(path.join("session.v4.jsonl"), lines.join("\n")).expect("write");
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn lists_sessions_newest_first_with_titles() {
        let root = scratch("list");
        write_session(
            &root,
            "--p--",
            "a",
            &[
                r#"{"type":"session","version":4,"id":"a","createdAt":100,"isSeeded":false,"delegationDepth":0}"#,
                r#"{"type":"user/message","seq":0,"time":110,"data":{"id":"u1","source":{"kind":"user"},"content":[{"type":"text","text":"第一个问题"}]}}"#,
            ],
        );
        write_session(
            &root,
            "--p--",
            "b",
            &[
                r#"{"type":"session","version":4,"id":"b","createdAt":200,"isSeeded":false,"delegationDepth":0}"#,
                r#"{"type":"session/title","seq":0,"time":210,"data":{"title":"已命名的会话"}}"#,
            ],
        );

        let listed = SessionStore::new(&root).list().expect("list");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, "b", "newest first");
        assert_eq!(listed[0].title, "已命名的会话");
        assert_eq!(listed[1].title, "第一个问题");
        assert_eq!(listed[1].updated_at, 110);
    }

    #[test]
    fn the_injected_context_blocks_are_not_titles() {
        let root = scratch("context");
        write_session(
            &root,
            "--p--",
            "a",
            &[
                r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
                r#"{"type":"user/message","seq":0,"time":2,"data":{"source":{"kind":"system-reminder"},"content":[{"type":"text","text":"<system-reminder>context</system-reminder>"}]}}"#,
                r#"{"type":"user/message","seq":1,"time":3,"data":{"id":"u1","source":{"kind":"user"},"content":[{"type":"text","text":"真正的问题"}]}}"#,
            ],
        );

        let listed = SessionStore::new(&root).list().expect("list");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(listed[0].title, "真正的问题");
    }

    #[test]
    fn loads_one_session_by_id() {
        let root = scratch("load");
        write_session(
            &root,
            "--p--",
            "a",
            &[
                r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
                r#"{"type":"user/message","seq":0,"time":2,"data":{"content":[{"type":"text","text":"hi"}]}}"#,
            ],
        );

        let history = SessionStore::new(&root).load("a").expect("load");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(history.header.id, "a");
        assert_eq!(history.events.len(), 1);
        assert_eq!(history.events[0].kind, "user/message");
    }

    #[test]
    fn a_newer_generation_wins_and_an_unknown_one_is_skipped() {
        let root = scratch("versions");
        let dir = root.join(SESSIONS_DIR).join("--p--").join("a");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("session.v3.jsonl"),
            r#"{"type":"session","version":3,"id":"old","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
        )
        .expect("v3");
        std::fs::write(
            dir.join("session.v4.jsonl"),
            r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
        )
        .expect("v4");

        let listed = SessionStore::new(&root).list().expect("list");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "a");
    }

    #[test]
    fn a_missing_root_is_an_empty_list_not_an_error() {
        let store = SessionStore::new(std::env::temp_dir().join("dsh-store-does-not-exist"));
        assert!(store.list().expect("list").is_empty());
    }
}

#[cfg(test)]
mod rename_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str, title: Option<&str>) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let header = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        let mut lines = vec![header];
        if let Some(title) = title {
            lines.push(format!(
                r#"{{"type":"session/title","seq":0,"time":2,"data":{{"title":"{title}"}}}}"#
            ));
        }
        std::fs::write(dir.join("session.v4.jsonl"), lines.join("\n")).expect("write");
    }

    #[test]
    fn a_rename_wins_over_the_derived_title() {
        let root = scratch("rename");
        write_session(&root, "a", Some("内核给的标题"));
        let store = SessionStore::new(&root);

        store.rename("a", "我的叫法").expect("rename");
        let listed = store.list().expect("list");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(listed[0].title, "我的叫法");
    }

    #[test]
    fn deleting_removes_the_directory_and_the_rename() {
        let root = scratch("delete");
        write_session(&root, "a", Some("要删的"));
        let store = SessionStore::new(&root);
        store.rename("a", "改名了").expect("rename");

        store.delete("a").expect("delete");
        let listed = store.list().expect("list");
        let overlay = SessionTitles::load(&root);
        let _ = std::fs::remove_dir_all(&root);

        assert!(listed.is_empty(), "the session is gone");
        assert_eq!(overlay.get("a"), None, "the rename went with it");
    }

    #[test]
    fn deleting_an_unknown_id_is_an_error_not_a_silent_no_op() {
        let root = scratch("delete-missing");
        let store = SessionStore::new(&root);
        let result = store.delete("nope");
        let _ = std::fs::remove_dir_all(&root);
        assert!(matches!(result, Err(StoreError::NotFound(_))));
    }

    #[test]
    fn a_subagent_session_reports_its_parent() {
        let root = scratch("parent");
        let dir = root.join(SESSIONS_DIR).join("--p--").join("child");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("session.v4.jsonl"),
            r#"{"type":"session","version":4,"id":"child","createdAt":1,"isSeeded":false,"delegationDepth":1,"parentSession":"parent"}"#,
        )
        .expect("write");

        let listed = SessionStore::new(&root).list().expect("list");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(listed[0].parent_id.as_deref(), Some("parent"));
        assert!(listed[0].is_subagent);
    }
}

#[cfg(test)]
mod search_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-search-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str, lines: &[&str]) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(dir.join("session.v4.jsonl"), lines.join("\n")).expect("write");
    }

    #[test]
    fn finds_text_in_the_users_words_and_the_answer() {
        let root = scratch("hit");
        write_session(
            &root,
            "a",
            &[
                r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
                r#"{"type":"user/message","seq":0,"time":10,"data":{"source":{"kind":"user"},"content":[{"type":"text","text":"怎么配置 Tauri 的窗口"}]}}"#,
                r#"{"type":"assistant/message","seq":1,"time":11,"data":{"message":{"content":[{"type":"text","text":"在 tauri.conf.json 里改"}]}}}"#,
            ],
        );

        let hits = SessionStore::new(&root).search("tauri", 10).expect("search");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(hits.len(), 2, "both sides of the conversation are searchable");
        assert!(hits.iter().all(|hit| hit.session_id == "a"));
    }

    #[test]
    fn ignores_tool_traffic() {
        // A path containing the term is not a conversation about the term.
        let root = scratch("tools");
        write_session(
            &root,
            "a",
            &[
                r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
                r#"{"type":"tool/call","seq":0,"time":10,"data":{"name":"pwsh","arguments":"{\"command\":\"ls tauri\"}"}}"#,
            ],
        );

        let hits = SessionStore::new(&root).search("tauri", 10).expect("search");
        let _ = std::fs::remove_dir_all(&root);
        assert!(hits.is_empty());
    }

    #[test]
    fn a_snippet_never_splits_a_character() {
        let root = scratch("utf8");
        let body = "前".repeat(60) + "关键字" + &"后".repeat(60);
        write_session(
            &root,
            "a",
            &[
                r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
                &format!(
                    r#"{{"type":"user/message","seq":0,"time":10,"data":{{"content":[{{"type":"text","text":"{body}"}}]}}}}"#
                ),
            ],
        );

        let hits = SessionStore::new(&root).search("关键字", 10).expect("search");
        let _ = std::fs::remove_dir_all(&root);

        let snippet = &hits[0].snippet;
        assert!(snippet.contains("关键字"));
        assert!(!snippet.contains('\u{fffd}'), "no replacement characters: {snippet}");
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        let store = SessionStore::new(std::env::temp_dir());
        assert!(store.search("   ", 10).expect("search").is_empty());
    }
}

#[cfg(test)]
mod contains_tests {
    use super::*;

    #[test]
    fn finds_a_session_by_header_without_reading_the_body() {
        let root = std::env::temp_dir().join(format!("dsh-contains-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(SESSIONS_DIR).join("--p--").join("a");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("session.v4.jsonl"),
            r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
        )
        .expect("write");

        let store = SessionStore::new(&root);
        let found = store.contains("a");
        let missing = store.contains("b");
        let _ = std::fs::remove_dir_all(&root);

        assert!(found);
        assert!(!missing);
    }
}

#[cfg(test)]
mod trash_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-trash-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join("session.v4.jsonl"),
            format!(
                r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
            ),
        )
        .expect("write");
    }

    #[test]
    fn delete_moves_the_session_to_trash_instead_of_erasing_it() {
        // "Delete" is one keystroke away from "I clicked the wrong row". The bytes stay.
        let root = scratch("delete");
        write_session(&root, "a");
        let store = SessionStore::new(&root);

        store.delete("a").expect("delete");
        let listed = store.list().expect("list");
        let trashed = std::fs::read_dir(root.join("trash"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        let _ = std::fs::remove_dir_all(&root);

        assert!(listed.is_empty(), "gone from every list");
        assert_eq!(trashed, 1, "but still on disk, under trash/");
    }

    #[test]
    fn purge_is_the_irreversible_one() {
        let root = scratch("purge");
        write_session(&root, "a");
        let store = SessionStore::new(&root);

        store.purge("a").expect("purge");
        let listed = store.list().expect("list");
        let trashed = std::fs::read_dir(root.join("trash"))
            .map(|entries| entries.count())
            .unwrap_or(0);
        let _ = std::fs::remove_dir_all(&root);

        assert!(listed.is_empty());
        assert_eq!(trashed, 0, "purge leaves nothing behind");
    }
}

#[cfg(test)]
mod search_limit_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-limit-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let header = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        let body = format!(
            r#"{{"type":"user/message","seq":0,"time":10,"data":{{"content":[{{"type":"text","text":"needle {id}"}}]}}}}"#
        );
        std::fs::write(dir.join("session.v4.jsonl"), format!("{header}\n{body}")).expect("write");
    }

    #[test]
    fn the_limit_is_a_limit_not_a_target() {
        // Ten sessions, ask for two: the answer is two, and the scan stops there instead of
        // parsing the other eight for matches it cannot use.
        let root = scratch("limit");
        for index in 0..10 {
            write_session(&root, &format!("s{index}"));
        }

        let hits = SessionStore::new(&root).search("needle", 2).expect("search");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(hits.len(), 2);
    }
}
