//! Shared application state.
//!
//! The kernel process pool lands here in M0. Until then this holds the resolved
//! paths, which the pool and the session store both need and which must agree.

use std::path::PathBuf;

pub struct AppState {
    /// Private kernel home. Never `~/.dsh` — that belongs to the user's CLI.
    pub dsh_home: PathBuf,
    /// Directory the pinned kernel was fetched into.
    pub kernel_dir: PathBuf,
}

impl AppState {
    /// Resolve paths from the app's own data directory.
    ///
    /// `DSH_DESKTOP_HOME` overrides the home directory so development runs do
    /// not mix with an installed build.
    pub fn resolve(data_dir: PathBuf) -> Self {
        let home = std::env::var_os("DSH_DESKTOP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("dsh-home"));
        let kernel_dir = std::env::var_os("DSH_DESKTOP_KERNEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../kernel"));

        Self { dsh_home: home, kernel_dir }
    }
}
