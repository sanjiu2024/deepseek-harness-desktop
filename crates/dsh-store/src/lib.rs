//! Reads the kernel's session logs.
//!
//! The SDK wire protocol has no "list sessions" call and `session/resume` does
//! not replay history — it only reports events committed after resume. History
//! therefore comes from disk, and this crate is the only place in the project
//! that knows the layout.
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

/// The session format generation this build understands.
pub const CURRENT_FORMAT_VERSION: u32 = 4;

/// Directory under `DSH_HOME` holding every session.
pub const SESSIONS_DIR: &str = "sessions";

/// The first line of a session log.
#[derive(Debug, Clone, serde::Deserialize)]
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
#[derive(Debug, Clone, serde::Deserialize)]
pub struct SessionEvent {
    /// Event kind, e.g. `user/message`, `assistant/chunk`, `tool/call`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Monotonic sequence within the session.
    pub seq: u64,
    /// Milliseconds since the Unix epoch.
    pub time: i64,
    pub data: serde_json::Value,
}

/// What a session list row needs, without loading the body.
#[derive(Debug, Clone)]
pub struct SessionSummary {
    pub header: SessionHeader,
    /// Absolute path of the selected log file.
    pub path: std::path::PathBuf,
    /// Title from the `session/title` event, when one exists.
    pub title: Option<String>,
    pub size_bytes: u64,
    /// Modified time, milliseconds since the Unix epoch.
    pub updated_at: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("unreadable session format v{version} at {path}")]
    UnsupportedFormat { version: u32, path: String },
    #[error("session log has no header: {0}")]
    MissingHeader(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}
