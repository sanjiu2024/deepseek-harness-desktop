//! Drives one DeepSeek Harness kernel process over its SDK wire protocol.
//!
//! The kernel speaks JSON-RPC 2.0, one message per `\n`-terminated line, over
//! stdio. Method and notification names below are copied from the upstream
//! protocol package; keep them in sync when the pinned kernel version changes.
//!
//! Nothing here spawns a process yet — this crate opens with the wire
//! vocabulary, which both the Rust side and the generated front end bindings
//! describe. Process management lands in milestone M0.

/// Client-to-server request methods.
pub mod method {
    pub const INITIALIZE: &str = "initialize";
    pub const SESSION_PROMPT: &str = "session/prompt";
    pub const SESSION_QUERY: &str = "session/query";
    pub const SESSION_RESUME: &str = "session/resume";
    pub const SESSION_WAIT: &str = "session/wait";
    pub const WORKING_DIRECTORY_GET: &str = "session/working-directory/get";
    pub const WORKING_DIRECTORY_SET: &str = "session/working-directory/set";
    pub const SHUTDOWN: &str = "shutdown";
}

/// Server-to-client methods the kernel may call: the client must answer these
/// even while a turn is running, or the agent blocks forever.
pub mod request {
    pub const APPROVAL: &str = "approval/request";
    pub const USER_QUESTION: &str = "user-question/ask";
}

/// Server-to-client notifications.
pub mod notification {
    pub const SESSION_EVENT: &str = "session.event";
    pub const SESSION_STATUS: &str = "session.status";
    pub const SESSION_QUERY_COMPLETED: &str = "session.query.completed";
    pub const SUBAGENT_STARTED: &str = "subagent.started";
    pub const SUBAGENT_FINISHED: &str = "subagent.finished";
    pub const USER_QUESTION_CANCELLED: &str = "user-question.cancelled";
}

/// One decision the user makes on a tool that requires approval.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalOutcome {
    /// Grant this one operation only. Never a standing permission.
    AllowedOnce,
    Rejected,
    Cancelled,
}

/// Why a kernel interaction failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("kernel process is not running")]
    NotRunning,
    #[error("kernel rejected {method}: [{code}] {message}")]
    Rpc { method: String, code: i64, message: String },
    #[error("malformed frame: {0}")]
    Malformed(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_outcome_uses_upstream_wire_spelling() {
        let json = serde_json::to_string(&ApprovalOutcome::AllowedOnce).expect("serializes");
        assert_eq!(json, "\"allowed-once\"");
    }
}
