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

    /// Provider keys for the kernel process.
    ///
    /// Two sources, in priority order: this process's environment, then
    /// `provider.env` in the kernel home (`KEY=value` per line, `#` comments).
    /// The file exists so a development run does not need the variable exported
    /// in every shell; it is git-ignored by name and never enters the repository.
    pub fn provider_env(&self) -> Vec<(String, String)> {
        let mut keys: Vec<(String, String)> = PROVIDER_KEYS
            .iter()
            .filter_map(|name| match std::env::var(name) {
                Ok(value) if !value.is_empty() => Some(((*name).to_owned(), value)),
                _ => None,
            })
            .collect();

        for (name, value) in read_env_file(&self.dsh_home.join("provider.env")) {
            if !PROVIDER_KEYS.contains(&name.as_str()) {
                continue;
            }
            if keys.iter().any(|(existing, _)| existing == &name) {
                continue;
            }
            keys.push((name, value));
        }
        keys
    }
}

/// Keys the kernel may need; anything else in `provider.env` is ignored.
const PROVIDER_KEYS: &[&str] = &["DEEPSEEK_API_KEY", "OPENCODE_API_KEY"];

/// Parse a minimal `KEY=value` file. Unreadable is normal, not an error.
fn read_env_file(path: &std::path::Path) -> Vec<(String, String)> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| {
            let (name, value) = line.split_once('=')?;
            let name = name.trim();
            let value = value.trim().trim_matches('"').trim_matches('\'').trim();
            (!name.is_empty() && !value.is_empty()).then(|| (name.to_owned(), value.to_owned()))
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_file_parsing_ignores_comments_and_blank_values() {
        let dir = std::env::temp_dir().join(format!("dsh-env-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let path = dir.join("provider.env");
        std::fs::write(
            &path,
            "# a comment\n\nOPENCODE_API_KEY=oc_sk_test\nQUOTED=\"quoted value\"\nEMPTY=\n",
        )
        .expect("write file");

        let parsed = read_env_file(&path);
        let _ = std::fs::remove_dir_all(&dir);

        assert!(parsed.contains(&("OPENCODE_API_KEY".to_owned(), "oc_sk_test".to_owned())));
        assert!(parsed.contains(&("QUOTED".to_owned(), "quoted value".to_owned())));
        assert!(!parsed.iter().any(|(name, _)| name == "EMPTY"));
    }

    #[test]
    fn missing_env_file_is_not_an_error() {
        assert!(read_env_file(std::path::Path::new("does-not-exist.env")).is_empty());
    }
}
