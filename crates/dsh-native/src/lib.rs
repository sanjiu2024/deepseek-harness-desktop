//! Everything the application touches on the operating system that has nothing
//! to do with the kernel: interactive shells, the workspace tree, and file
//! watching.
//!
//! This crate deliberately knows nothing about DeepSeek Harness. It can be
//! lifted into another project unchanged, which is why it does not depend on
//! [`dsh_runtime`] or [`dsh_store`] and never will.
//!
//! Implementation lands in milestone M5.

/// One row of the workspace tree.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileNode {
    /// Path relative to the workspace root, always using `/` separators.
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    /// Bytes; `None` for directories.
    pub size: Option<u64>,
    /// Modified time, milliseconds since the Unix epoch.
    pub modified_at: Option<i64>,
}

/// How a spawned shell reports its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShellState {
    Starting,
    Running,
    Exited,
}

#[derive(Debug, thiserror::Error)]
pub enum NativeError {
    #[error("path escapes the workspace root: {0}")]
    OutsideWorkspace(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
