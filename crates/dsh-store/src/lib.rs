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
}

/// One session's complete history.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHistory {
    pub header: SessionHeader,
    pub events: Vec<SessionEvent>,
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
        let mut out = Vec::new();
        for path in self.logs()? {
            match read_session(&path) {
                Ok((header, events)) => out.push(summarize(&header, &events)),
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
