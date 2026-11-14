//! Shared application state.
//!
//! The kernel pool lands here in M4. Until then this holds at most one client
//! plus the resolved paths that the pool and the session store both need and
//! that must agree.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use dsh_runtime::KernelClient;

pub struct AppState {
    /// Private kernel home. Never `~/.dsh` — that belongs to the user's CLI.
    pub dsh_home: PathBuf,
    /// Directory the pinned kernel was fetched into.
    pub kernel_dir: PathBuf,
    /// The running kernel, if any. `None` means nothing has been started yet.
    pub kernel: Mutex<Option<Arc<KernelClient>>>,
}

impl AppState {
    /// Resolve paths from the app's own data directory.
    ///
    /// `DSH_DESKTOP_HOME` and `DSH_DESKTOP_KERNEL` override both so a
    /// development run never mixes with an installed build.
    pub fn resolve(data_dir: PathBuf) -> Self {
        let home = std::env::var_os("DSH_DESKTOP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("dsh-home"));
        let kernel_dir = std::env::var_os("DSH_DESKTOP_KERNEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../kernel"));

        Self {
            dsh_home: home,
            kernel_dir,
            kernel: Mutex::new(None),
        }
    }

    /// The running kernel, or an error the front end can show.
    pub fn running(&self) -> Result<Arc<KernelClient>, String> {
        self.kernel
            .lock()
            .expect("kernel lock")
            .clone()
            .ok_or_else(|| "the kernel is not running".to_owned())
    }
}
