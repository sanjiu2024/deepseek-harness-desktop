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
    /// The kernel-side live bridge source, copied into the kernel home at start.
    pub bridge_dir: PathBuf,
    /// The running kernel, if any. `None` means nothing has been started yet.
    pub kernel: Mutex<Option<Arc<KernelClient>>>,
}

impl AppState {
    /// Resolve paths from the app's own data directory.
    ///
    /// `DSH_DESKTOP_HOME`, `DSH_DESKTOP_KERNEL` and `DSH_DESKTOP_BRIDGE` override
    /// all three so a development run never mixes with an installed build.
    pub fn resolve(data_dir: PathBuf) -> Self {
        let home = std::env::var_os("DSH_DESKTOP_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| data_dir.join("dsh-home"));
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..");
        let kernel_dir = std::env::var_os("DSH_DESKTOP_KERNEL")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo.join("kernel"));
        let bridge_dir = std::env::var_os("DSH_DESKTOP_BRIDGE")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo.join("bridge"));

        Self {
            dsh_home: home,
            kernel_dir,
            bridge_dir,
            kernel: Mutex::new(None),
        }
    }

    /// Copy the live bridge where the kernel's module resolution can find it.
    ///
    /// Node resolves from `$DSH_HOME/profiles/sdk/` upward, so the kernel home's
    /// own `node_modules` is the one place that works without touching the
    /// fetched kernel or hard-coding a machine path into the profile patch.
    pub fn install_bridge(&self) -> Result<(), String> {
        let source = self.bridge_dir.join("index.js");
        if !source.exists() {
            return Err(format!(
                "live bridge not found — expected {} (set DSH_DESKTOP_BRIDGE)",
                source.display()
            ));
        }
        let name = bridge_package_name(&self.bridge_dir)?;
        let target = self.dsh_home.join("node_modules").join(&name);
        std::fs::create_dir_all(&target).map_err(|error| error.to_string())?;
        for entry in ["index.js", "package.json"] {
            std::fs::copy(self.bridge_dir.join(entry), target.join(entry))
                .map_err(|error| format!("copy bridge {entry}: {error}"))?;
        }
        Ok(())
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

fn bridge_package_name(dir: &std::path::Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(dir.join("package.json"))
        .map_err(|error| format!("read bridge manifest: {error}"))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&raw).map_err(|error| format!("parse bridge manifest: {error}"))?;
    manifest["name"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| "bridge/package.json has no \"name\"".to_owned())
}
