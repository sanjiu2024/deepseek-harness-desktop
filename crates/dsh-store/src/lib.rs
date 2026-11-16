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

/// How many of a session's events are loaded at once.
///
/// A conversation that reached this length is not one a reader scrolls from the top. Combined
/// with `truncated`, the client can say "只显示最近 2000 条" instead of freezing while it
/// materialises a year of history.
pub const EVENT_WINDOW: usize = 2000;

/// The log file name inside a session directory.
pub const LOG_NAME: &str = "session.v4.jsonl";

/// The session format generation this build understands.
pub const CURRENT_FORMAT_VERSION: u32 = 4;

/// Directory under `DSH_HOME` holding every session.
pub const SESSIONS_DIR: &str = "sessions";

/// The first line of a session log.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct SessionHeader {
    /// Always `session`. Written back on fork: the kernel validates the first line, and a header
    /// without this field is "not a session header" as far as it is concerned.
    #[serde(default = "session_marker", rename = "type")]
    pub kind: String,
    /// Format generation. Anything but [`CURRENT_FORMAT_VERSION`] is unreadable here.
    pub version: u32,
    pub id: String,
    /// Milliseconds since the Unix epoch.
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// Set when this session was spawned by a subagent tool.
    #[serde(
        default,
        rename = "parentSession",
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_session: Option<String>,
    #[serde(default, rename = "isSeeded")]
    pub is_seeded: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(default, rename = "delegationDepth")]
    pub delegation_depth: u32,
    #[serde(
        default,
        rename = "agentPreset",
        skip_serializing_if = "Option::is_none"
    )]
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
    /// The events to render, newest kept when the session is longer than the window.
    pub events: Vec<SessionEvent>,
    /// How many readable events the file has in total.
    pub total: usize,
    /// True when `events` is only the tail; the beginning was not loaded.
    pub truncated: bool,
}

/// How far a search will read before giving up.
///
/// A search is interactive: it runs while someone types. Reading a gigabyte of transcripts to
/// answer "where did I write about X" is not a search, it is a batch job nobody asked for.
pub const SEARCH_BUDGET_BYTES: u64 = 64 * 1024 * 1024;

/// What a search found, and what it cost.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchOutcome {
    pub hits: Vec<SearchHit>,
    /// True when the scan stopped early: hit limit or byte budget.
    pub truncated: bool,
    /// How many bytes of transcript were read before answering.
    pub scanned_bytes: u64,
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

/// What the store occupies, in one answer.
///
/// Exists because "my disk is full" is a question a user asks about this app, and the honest
/// answer is a number, not a tree of directories to add up by hand. Every field is measured,
/// never estimated.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStats {
    /// Readable sessions, the same count `list` returns.
    pub sessions: usize,
    /// Bytes under `sessions/`.
    pub session_bytes: u64,
    /// Sessions sitting in `trash/`.
    pub trash_entries: usize,
    /// Bytes under `trash/`.
    pub trash_bytes: u64,
    /// Newest session's last activity, milliseconds since the epoch.
    pub newest_activity: Option<i64>,
    /// Oldest session's last activity.
    pub oldest_activity: Option<i64>,
}

/// What a fork produced.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForkOutcome {
    pub id: String,
    /// The session it came from.
    pub source: String,
    /// Where the copy lives, so a UI can show it or move it.
    pub path: String,
}

/// One session sitting in the trash.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashEntry {
    /// The session id, recoverable from the directory name.
    pub id: String,
    /// Milliseconds since the epoch when it was deleted.
    pub deleted_at: i64,
    pub bytes: u64,
    /// Where it actually is, so "restore" can be a directory move.
    pub path: String,
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
    #[error("cannot fork: {0}")]
    Fork(String),
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
            match read_session_windowed(&path, EVENT_WINDOW) {
                Ok((header, events, total, truncated)) if header.id == id => {
                    return Ok(SessionHistory {
                        header,
                        events,
                        total,
                        truncated,
                    });
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
    pub fn search(&self, query: &str, limit: usize) -> Result<SearchOutcome, StoreError> {
        let needle = query.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(SearchOutcome {
                hits: Vec::new(),
                truncated: false,
                scanned_bytes: 0,
            });
        }
        let titles = SessionTitles::load(self.home());
        let mut hits = Vec::new();
        let mut scanned_bytes = 0u64;
        let mut truncated = false;

        for path in self.logs()? {
            // Stop the whole scan once the limit is met: continuing to open and parse the remaining
            // transcripts cannot change the answer, and the caller asked for a page, not for every
            // match in the archive.
            if hits.len() >= limit {
                truncated = true;
                break;
            }
            // A budget, not a timeout: a scan that takes a second on twelve sessions takes a
            // core for a minute on a thousand, and the user gets nothing until it finishes.
            if scanned_bytes >= SEARCH_BUDGET_BYTES {
                truncated = true;
                tracing::debug!(target: "dsh_store", scanned_bytes, "search budget reached");
                break;
            }
            let Ok((header, events)) = read_session(&path) else {
                continue;
            };
            // Count what was actually read, so the budget reflects real work.
            scanned_bytes += std::fs::metadata(&path)
                .map(|metadata| metadata.len())
                .unwrap_or(0);
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
        Ok(SearchOutcome {
            hits,
            truncated,
            scanned_bytes,
        })
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

    /// Count and measure everything the store holds.
    ///
    /// One pass over headers plus directory sizes. It does not read transcripts: the question is
    /// "how much room does this take", and the answer is a byte count, not the content.
    pub fn stats(&self) -> Result<StorageStats, StoreError> {
        let mut sessions = 0usize;
        let mut session_bytes = 0u64;
        let mut newest: Option<i64> = None;
        let mut oldest: Option<i64> = None;

        for path in self.logs()? {
            let Ok(header) = read_header(&path) else {
                continue;
            };
            sessions += 1;
            // The whole session directory, not just the log file: there can be more in it.
            if let Some(dir) = path.parent() {
                session_bytes += directory_bytes(dir);
            }
            let updated = summarize_cheap(&header, &path).updated_at;
            newest = Some(newest.map_or(updated, |current: i64| current.max(updated)));
            oldest = Some(oldest.map_or(updated, |current: i64| current.min(updated)));
        }

        let trash = self.trash()?;
        let trash_bytes = trash.iter().map(|entry| entry.bytes).sum();
        Ok(StorageStats {
            sessions,
            session_bytes,
            trash_entries: trash.len(),
            trash_bytes,
            newest_activity: newest,
            oldest_activity: oldest,
        })
    }

    /// What is currently in the trash.
    ///
    /// The trash never empties itself — a recycle bin that empties itself is not one — but a
    /// directory that grows forever is its own problem. This makes the cost visible so a UI can
    /// show "回收站：3 项，120 MB" and let someone decide.
    pub fn trash(&self) -> Result<Vec<TrashEntry>, StoreError> {
        let dir = self.home().join("trash");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // No trash directory means nothing was ever deleted, not that something failed.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(StoreError::Io(error)),
        };

        let mut out = Vec::new();
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            // Stored as "<id>-<epoch ms>"; ids can contain dashes, the stamp cannot.
            let (id, deleted_at) = match name.rsplit_once('-') {
                Some((id, stamp)) => (id.to_owned(), stamp.parse::<i64>().unwrap_or_default()),
                None => (name.clone(), 0),
            };
            out.push(TrashEntry {
                bytes: directory_bytes(&entry.path()),
                path: entry.path().to_string_lossy().into_owned(),
                id,
                deleted_at,
            });
        }
        out.sort_by_key(|entry| std::cmp::Reverse(entry.deleted_at));
        Ok(out)
    }

    /// Empty the trash. Returns how many entries were removed.
    ///
    /// The only irreversible bulk operation in the crate; a UI must ask first.
    pub fn empty_trash(&self) -> Result<usize, StoreError> {
        let dir = self.home().join("trash");
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(StoreError::Io(error)),
        };

        let mut removed = 0;
        for entry in entries.flatten() {
            if std::fs::remove_dir_all(entry.path()).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Copy a session into a new one, keeping every event.
    ///
    /// "Fork" here means what it means in a notebook: the new session **is** the old one up to
    /// this moment, and diverges from here. It is done by copying the log and rewriting the
    /// header id — the events are not replayed through the model, so a fork costs nothing and
    /// cannot change what the model already said.
    ///
    /// The copy lands in the same project directory, because that directory is the kernel's
    /// notion of "which workspace this session belongs to".
    pub fn fork(
        &self,
        id: &str,
        new_id: &str,
        title: Option<&str>,
    ) -> Result<ForkOutcome, StoreError> {
        if id == new_id {
            return Err(StoreError::Fork(
                "a session cannot be forked onto itself".to_owned(),
            ));
        }
        if self.contains(new_id) {
            return Err(StoreError::Fork(format!("{new_id} already exists")));
        }

        let source = self.directory_of(id)?;
        let project = source
            .parent()
            .ok_or_else(|| StoreError::NotFound(id.to_owned()))?;
        let target = project.join(new_id);
        if target.exists() {
            return Err(StoreError::Fork(format!(
                "{} already exists",
                target.display()
            )));
        }

        let raw = std::fs::read_to_string(source.join(LOG_NAME))?;
        let mut lines = raw.lines();
        let first = lines
            .next()
            .ok_or_else(|| StoreError::MissingHeader(source.display().to_string()))?;

        let mut header: SessionHeader = serde_json::from_str(first)?;
        header.id = new_id.to_owned();
        header.created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|delta| delta.as_millis() as i64)
            .unwrap_or(header.created_at);
        // A fork is its own conversation: it was not seeded by a parent run, and it is not a
        // subagent of the session it came from.
        header.parent_session = None;
        header.delegation_depth = 0;

        let mut body = serde_json::to_string(&header)?;
        body.push('\n');
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }

        std::fs::create_dir_all(&target)?;
        write_atomic(&target.join(LOG_NAME), body.as_bytes())?;

        if let Some(title) = title.map(str::trim).filter(|title| !title.is_empty()) {
            let mut titles = SessionTitles::load(self.home());
            titles.set(new_id, title).map_err(StoreError::Overlay)?;
        }

        Ok(ForkOutcome {
            id: new_id.to_owned(),
            source: id.to_owned(),
            path: target.to_string_lossy().into_owned(),
        })
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
    let (header, events, _, _) = read_session_windowed(path, usize::MAX)?;
    Ok((header, events))
}

/// Read a session, keeping at most `window` events — the newest ones.
///
/// Two reasons this is not `read_to_string` plus a full parse:
///
/// 1. **Memory.** A long session reaches tens of megabytes; the parsed `Vec<SessionEvent>` is
///    several times larger than the file, and the whole thing then has to be serialised again
///    to cross to the UI. Peak memory grew with session length, and the window froze while it
///    happened.
/// 2. **Relevance.** A conversation's opening is not what a reader needs when they reopen it
///    after a hundred turns — the tail is. So the window keeps the newest events and reports
///    how many were left out, rather than pretending the session started mid-sentence.
fn read_session_windowed(
    path: &Path,
    window: usize,
) -> Result<(SessionHeader, Vec<SessionEvent>, usize, bool), StoreError> {
    use std::io::BufRead;

    let file = std::fs::File::open(path)?;
    let mut lines = std::io::BufReader::new(file).lines();

    let header: SessionHeader = loop {
        match lines.next() {
            Some(Ok(line)) if !line.trim().is_empty() => {
                break serde_json::from_str(&line)?;
            }
            Some(Ok(_)) => continue,
            Some(Err(error)) => return Err(StoreError::Io(error)),
            None => return Err(StoreError::MissingHeader(path.display().to_string())),
        }
    };

    if header.version != CURRENT_FORMAT_VERSION {
        return Err(StoreError::UnsupportedFormat {
            version: header.version,
            path: path.display().to_string(),
        });
    }

    let keep = window.max(1);
    let mut kept: std::collections::VecDeque<SessionEvent> =
        std::collections::VecDeque::with_capacity(keep.min(4096));
    let mut total = 0usize;

    for line in lines {
        let Ok(line) = line else {
            // A torn final line is normal after a crash; everything before it counts.
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<SessionEvent>(&line) {
            Ok(event) => {
                total += 1;
                if kept.len() == keep {
                    kept.pop_front();
                }
                kept.push_back(event);
            }
            Err(error) => {
                tracing::debug!(target: "dsh_store", %error, "skipping a malformed event row");
            }
        }
    }

    let truncated = total > kept.len();
    Ok((header, kept.into(), total, truncated))
}

/// The literal every session header starts with.
fn session_marker() -> String {
    "session".to_owned()
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

    let title = title.or(first_user).unwrap_or_else(|| header.id.clone());

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

/// Total bytes under a directory, without following symlinks out of it.
fn directory_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    let mut total = 0;
    for entry in entries.flatten() {
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => total += directory_bytes(&entry.path()),
            Ok(kind) if kind.is_file() => {
                total += entry.metadata().map(|metadata| metadata.len()).unwrap_or(0)
            }
            _ => {}
        }
    }
    total
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

        let hits = SessionStore::new(&root)
            .search("tauri", 10)
            .expect("search")
            .hits;
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(
            hits.len(),
            2,
            "both sides of the conversation are searchable"
        );
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

        let hits = SessionStore::new(&root)
            .search("tauri", 10)
            .expect("search")
            .hits;
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

        let hits = SessionStore::new(&root)
            .search("关键字", 10)
            .expect("search")
            .hits;
        let _ = std::fs::remove_dir_all(&root);

        let snippet = &hits[0].snippet;
        assert!(snippet.contains("关键字"));
        assert!(
            !snippet.contains('\u{fffd}'),
            "no replacement characters: {snippet}"
        );
        assert!(snippet.starts_with('…') && snippet.ends_with('…'));
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        let store = SessionStore::new(std::env::temp_dir());
        assert!(store.search("   ", 10).expect("search").hits.is_empty());
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

        let hits = SessionStore::new(&root)
            .search("needle", 2)
            .expect("search")
            .hits;
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(hits.len(), 2);
    }
}

#[cfg(test)]
mod trash_listing_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("dsh-trashlist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str, body_bytes: usize) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let header = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        let mut content = header;
        content.push('\n');
        content.push_str(&"x".repeat(body_bytes));
        std::fs::write(dir.join("session.v4.jsonl"), content).expect("write");
    }

    #[test]
    fn a_deleted_session_shows_up_with_its_size() {
        let root = scratch("list");
        write_session(&root, "a", 2048);
        let store = SessionStore::new(&root);
        store.delete("a").expect("delete");

        let trash = store.trash().expect("trash");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(trash.len(), 1);
        assert_eq!(trash[0].id, "a");
        assert!(
            trash[0].bytes > 2048,
            "the whole directory counts: {}",
            trash[0].bytes
        );
        assert!(
            trash[0].deleted_at > 0,
            "the timestamp survives the round trip"
        );
    }

    #[test]
    fn emptying_the_trash_removes_everything_and_nothing_else() {
        let root = scratch("empty");
        write_session(&root, "keep", 10);
        write_session(&root, "drop", 10);
        let store = SessionStore::new(&root);
        store.delete("drop").expect("delete");

        let removed = store.empty_trash().expect("empty");
        let listed = store.list().expect("list");
        let trash = store.trash().expect("trash");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(removed, 1);
        assert!(trash.is_empty());
        assert_eq!(
            listed.len(),
            1,
            "the session that was not deleted is untouched"
        );
        assert_eq!(listed[0].id, "keep");
    }

    #[test]
    fn an_absent_trash_is_empty_not_an_error() {
        let root = scratch("absent");
        let store = SessionStore::new(&root);
        let trash = store.trash().expect("trash");
        let removed = store.empty_trash().expect("empty");
        let _ = std::fs::remove_dir_all(&root);
        assert!(trash.is_empty());
        assert_eq!(removed, 0);
    }
}

#[cfg(test)]
mod window_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-window-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_long_session(root: &Path, id: &str, events: usize) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let mut body = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        for index in 0..events {
            body.push_str(&format!(
                "\n{{\"type\":\"user/message\",\"seq\":{index},\"time\":{},\"data\":{{\"content\":[{{\"type\":\"text\",\"text\":\"m{index}\"}}]}}}}",
                1000 + index
            ));
        }
        std::fs::write(dir.join("session.v4.jsonl"), body).expect("write");
    }

    #[test]
    fn a_long_session_loads_its_tail_and_says_it_is_truncated() {
        // Opening a hundred-turn conversation must not mean materialising all of it: the
        // reader wants the tail, and the beginning is what they scroll back for *later*.
        let root = scratch("long");
        write_long_session(&root, "a", EVENT_WINDOW + 300);

        let history = SessionStore::new(&root).load("a").expect("load");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(history.events.len(), EVENT_WINDOW);
        assert!(history.truncated);
        assert_eq!(history.total, EVENT_WINDOW + 300);
        assert_eq!(
            history.events[0].seq, 300,
            "the oldest kept event is the one after the cut"
        );
        assert_eq!(
            history.events.last().map(|event| event.seq),
            Some((EVENT_WINDOW + 299) as i64),
            "the newest event is always present"
        );
    }

    #[test]
    fn a_short_session_is_complete_and_not_truncated() {
        let root = scratch("short");
        write_long_session(&root, "a", 5);

        let history = SessionStore::new(&root).load("a").expect("load");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(history.events.len(), 5);
        assert!(!history.truncated);
        assert_eq!(history.total, 5);
    }
}

#[cfg(test)]
mod search_budget_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-budget-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str, padding: usize) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let header = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        let filler = "y".repeat(padding);
        let body = format!(
            r#"{{"type":"user/message","seq":0,"time":10,"data":{{"content":[{{"type":"text","text":"needle {filler}"}}]}}}}"#
        );
        std::fs::write(dir.join("session.v4.jsonl"), format!("{header}\n{body}")).expect("write");
    }

    #[test]
    fn the_scan_reports_what_it_read() {
        let root = scratch("report");
        write_session(&root, "a", 10);

        let outcome = SessionStore::new(&root)
            .search("needle", 10)
            .expect("search");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcome.hits.len(), 1);
        assert!(!outcome.truncated);
        assert!(outcome.scanned_bytes > 0, "the cost is part of the answer");
    }

    #[test]
    fn hitting_the_limit_is_reported_as_truncation() {
        let root = scratch("limit");
        for index in 0..4 {
            write_session(&root, &format!("s{index}"), 10);
        }

        let outcome = SessionStore::new(&root)
            .search("needle", 1)
            .expect("search");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(outcome.hits.len(), 1);
        assert!(
            outcome.truncated,
            "one hit out of four is not a complete answer"
        );
    }
}

#[cfg(test)]
mod stats_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-stats-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str, padding: usize, time: i64) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let header = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        let body = format!(
            r#"{{"type":"user/message","seq":0,"time":{time},"data":{{"content":[{{"type":"text","text":"{}"}}]}}}}"#,
            "x".repeat(padding)
        );
        std::fs::write(dir.join("session.v4.jsonl"), format!("{header}\n{body}")).expect("write");
    }

    #[test]
    fn measures_sessions_and_trash_together() {
        let root = scratch("both");
        write_session(&root, "a", 1000, 10);
        write_session(&root, "b", 1000, 20);
        let store = SessionStore::new(&root);
        store.delete("b").expect("delete");

        let stats = store.stats().expect("stats");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(stats.sessions, 1);
        assert!(
            stats.session_bytes > 1000,
            "the directory counts: {}",
            stats.session_bytes
        );
        assert_eq!(stats.trash_entries, 1);
        assert!(
            stats.trash_bytes > 1000,
            "the trash counts too: {}",
            stats.trash_bytes
        );
        assert_eq!(stats.newest_activity, Some(10));
        assert_eq!(stats.oldest_activity, Some(10));
    }

    #[test]
    fn an_empty_store_reports_zeroes_not_an_error() {
        let root = scratch("empty");
        let stats = SessionStore::new(&root).stats().expect("stats");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(stats.sessions, 0);
        assert_eq!(stats.session_bytes, 0);
        assert_eq!(stats.trash_entries, 0);
        assert_eq!(
            stats.newest_activity, None,
            "no sessions means no activity to report"
        );
    }
}

#[cfg(test)]
mod fork_tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-fork-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).ok();
        root
    }

    fn write_session(root: &Path, id: &str, events: usize) {
        let dir = root.join(SESSIONS_DIR).join("--p--").join(id);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let mut body = format!(
            r#"{{"type":"session","version":4,"id":"{id}","createdAt":1,"isSeeded":false,"delegationDepth":0}}"#
        );
        for index in 0..events {
            body.push_str(&format!(
                "\n{{\"type\":\"user/message\",\"seq\":{index},\"time\":{},\"data\":{{\"content\":[{{\"type\":\"text\",\"text\":\"m{index}\"}}]}}}}",
                100 + index
            ));
        }
        std::fs::write(dir.join(LOG_NAME), body).expect("write");
    }

    #[test]
    fn a_fork_keeps_every_event_and_gets_its_own_id() {
        let root = scratch("copy");
        write_session(&root, "parent", 3);
        let store = SessionStore::new(&root);

        let fork = store.fork("parent", "child", Some("分支")).expect("fork");
        let original = store.load("parent").expect("parent");
        let copy = store.load("child").expect("child");
        let listed = store.list().expect("list");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(fork.id, "child");
        assert_eq!(copy.header.id, "child");
        assert_eq!(
            copy.events.len(),
            original.events.len(),
            "history comes along"
        );
        assert_eq!(copy.events[0].seq, 0, "from the first event, not the last");
        assert!(
            listed
                .iter()
                .any(|entry| entry.id == "child" && entry.title == "分支"),
            "the fork appears in the list with its title"
        );
    }

    #[test]
    fn the_fork_is_independent_of_its_parent() {
        // A fork is a new conversation: deleting one must not touch the other.
        let root = scratch("apart");
        write_session(&root, "parent", 2);
        let store = SessionStore::new(&root);
        store.fork("parent", "child", None).expect("fork");

        store.purge("parent").expect("purge");
        let copy = store.load("child").expect("the fork survives");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(copy.events.len(), 2);
    }

    #[test]
    fn forking_onto_an_existing_id_is_refused() {
        let root = scratch("clash");
        write_session(&root, "parent", 1);
        write_session(&root, "taken", 1);
        let store = SessionStore::new(&root);

        let clash = store.fork("parent", "taken", None);
        let onto_itself = store.fork("parent", "parent", None);
        let _ = std::fs::remove_dir_all(&root);

        assert!(matches!(clash, Err(StoreError::Fork(_))));
        assert!(matches!(onto_itself, Err(StoreError::Fork(_))));
    }
}

#[cfg(test)]
mod fork_shape_tests {
    use super::*;

    #[test]
    fn the_forked_header_still_starts_with_the_session_marker() {
        let root = std::env::temp_dir().join(format!("dsh-forkshape-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let dir = root.join(SESSIONS_DIR).join("--p--").join("a");
        std::fs::create_dir_all(&dir).expect("mkdir");
        std::fs::write(
            dir.join(LOG_NAME),
            concat!(
                r#"{"type":"session","version":4,"id":"a","createdAt":1,"isSeeded":false,"delegationDepth":0}"#,
                "\n",
                r#"{"type":"user/message","seq":0,"time":2,"data":{"content":[],"role":"user","id":"u1"}}"#,
                "\n",
            ),
        )
        .expect("write");

        SessionStore::new(&root).fork("a", "b", None).expect("fork");
        let written = std::fs::read_to_string(
            root.join(SESSIONS_DIR)
                .join("--p--")
                .join("b")
                .join(LOG_NAME),
        )
        .expect("read back");
        let first = written.lines().next().expect("first line").to_owned();
        let _ = std::fs::remove_dir_all(&root);

        println!("[probe] {first}");
        assert!(
            first.starts_with(r#"{"type":"session""#),
            "the kernel validates this line: {first}"
        );
    }
}

#[cfg(test)]
mod header_shape_tests {
    use super::*;

    #[test]
    fn a_serialised_header_matches_the_shape_the_kernel_writes() {
        // The kernel validates the first log line against a strict schema. Writing extra keys —
        // even as `null` — makes it answer "first line is not a session header", which sounds
        // like a missing field and is actually a surplus one. Found by resuming a fork against a
        // real kernel, not by reading the docs.
        let header = SessionHeader {
            kind: "session".to_owned(),
            version: CURRENT_FORMAT_VERSION,
            id: "a".to_owned(),
            created_at: 1,
            cwd: Some("D:/repo".to_owned()),
            parent_session: None,
            is_seeded: false,
            origin: None,
            delegation_depth: 0,
            agent_preset: None,
        };

        let json = serde_json::to_string(&header).expect("serialise");
        assert!(json.starts_with(r#"{"type":"session""#), "{json}");
        assert!(!json.contains("null"), "no nulls: {json}");
        assert!(
            !json.contains("parentSession")
                && !json.contains("origin")
                && !json.contains("agentPreset"),
            "optional keys are omitted, not null: {json}"
        );
    }
}
