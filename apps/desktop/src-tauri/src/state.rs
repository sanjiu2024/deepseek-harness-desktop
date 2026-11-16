//! Shared application state.
//!
//! The kernel pool lands here in M4. Until then this holds at most one client
//! plus the resolved paths that the pool and the session store both need and
//! that must agree.

use std::path::PathBuf;
use std::sync::Mutex;

use dsh_runtime::{KernelPool, Route};

/// The app data directory for this build: `%APPDATA%\<identifier>`.
///
/// Computed without an `AppHandle` so it can be created before the window is
/// built. This is hygiene, not a workaround: the directory is ours, and a window
/// that depends on a directory appearing later is a window with a hidden order
/// dependency.
pub fn app_data_dir() -> PathBuf {
    let roaming = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    roaming.join(identifier())
}

/// The bundle identifier, parsed from the configuration this binary was built with.
/// A second copy would silently stop matching the moment the app is renamed.
fn identifier() -> String {
    const CONFIG: &str = include_str!("../tauri.conf.json");
    serde_json::from_str::<serde_json::Value>(CONFIG)
        .ok()
        .and_then(|config| {
            config
                .get("identifier")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "com.dsh.desktop".to_owned())
}

/// The kernel home for this run.
///
/// `DSH_DESKTOP_HOME` wins so a development run keeps everything inside the
/// checkout; otherwise it sits under the app data directory.
pub fn dsh_home(data_dir: &std::path::Path) -> PathBuf {
    std::env::var_os("DSH_DESKTOP_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| data_dir.join("dsh-home"))
}

/// What the user can change without editing a file.
///
/// Deliberately tiny. The kernel's own configuration surface is enormous and
/// belongs to the kernel; this is only what the *client* must know before it can
/// start one: which route to ask for, and where the credential comes from.
///
/// Keys are never stored here — they stay in the process environment (see
/// `DSH_DESKTOP_ENV`), because a JSON file next to a session log is a worse place
/// for a secret than a file with no read path through the UI.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub provider: String,
    pub model: String,
    /// Session reopened on launch. Cleared when it no longer exists.
    #[serde(default, rename = "lastSessionId")]
    pub last_session_id: Option<String>,
    /// Root of the tree the side panel browses; absent means the process cwd.
    #[serde(default)]
    pub workspace: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: "opencode-go".to_owned(),
            model: "deepseek-v4.1-flash".to_owned(),
            last_session_id: None,
            workspace: None,
        }
    }
}

impl Settings {
    /// Read `settings.json` from the kernel home, falling back to the defaults.
    ///
    /// A missing or malformed file is not an error: a first launch has none, and a
    /// hand-edited one that broke JSON should not make the app unstartable.
    pub fn load(home: &std::path::Path) -> Self {
        let path = home.join("settings.json");
        let Ok(raw) = std::fs::read_to_string(&path) else {
            return Self::default();
        };
        // Notepad and PowerShell both write a UTF-8 BOM. serde_json treats it as a
        // syntax error, which would silently fall back to the defaults — a user who
        // edited this file would see nothing change and no reason why.
        let raw = raw.trim_start_matches('﻿');
        match serde_json::from_str(raw) {
            Ok(settings) => settings,
            Err(error) => {
                tracing::warn!(target: "dsh_desktop_app", %error, path = %path.display(), "ignoring unreadable settings");
                Self::default()
            }
        }
    }

    /// Write `settings.json` back to the kernel home.
    pub fn save(&self, home: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(home).map_err(|error| error.to_string())?;
        let path = home.join("settings.json");
        let body = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        // Atomic: settings.json is read back as one JSON document on every launch, and a
        // half-written one means the app starts on the defaults and nobody knows why.
        dsh_store::write_atomic(&path, body.as_bytes())
            .map_err(|error| format!("{}: {error}", path.display()))
    }
}

/// A path next to the executable, if this is a bundled build.
///
/// A distributed copy has `kernel/` and `node` beside the binary (see
/// `scripts/bundle-kernel.mjs`). Resolving them relative to the exe is what makes the bundle
/// self-contained: no PATH, no environment variables, no "install Node first".
pub fn alongside_exe(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.join(name);
    candidate.exists().then_some(candidate)
}

pub struct AppState {
    /// Private kernel home. Never `~/.dsh` — that belongs to the user's CLI.
    pub dsh_home: PathBuf,
    /// Directory the pinned kernel was fetched into.
    pub kernel_dir: PathBuf,
    /// The kernel-side live bridge source, copied into the kernel home at start.
    pub bridge_dir: PathBuf,
    /// Root of the tree the side panel browses. The app never reads outside it.
    pub workspace: PathBuf,
    /// One kernel per route. See `dsh_runtime::pool` for why a route needs its own process.
    pub pool: KernelPool,
    /// The initialize params that last worked, per route.
    ///
    /// A restarted kernel is a blank process: it does not know the provider, the model or the
    /// cwd. Replaying the params that worked is what makes the kernel coming back true rather
    /// than a new process merely running.
    pub initialized: std::sync::Mutex<std::collections::HashMap<Route, serde_json::Value>>,
    /// Routes whose frame readers are already attached, so a reused kernel does not get a
    /// second set of forwarders and emit every event twice.
    pub forwarded: Mutex<std::collections::HashSet<Route>>,
    /// The route new sessions start on: the last one initialized, else the settings file.
    pub route: Mutex<Route>,
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
            // A bundled copy carries its kernel; a development checkout does not.
            .or_else(|| alongside_exe("kernel"))
            .unwrap_or_else(|| repo.join("kernel"));
        let bridge_dir = std::env::var_os("DSH_DESKTOP_BRIDGE")
            .map(PathBuf::from)
            .unwrap_or_else(|| repo.join("bridge"));

        // Three kernels is a working set, not a limit on models: a route whose sessions are
        // still open is never evicted, so the pool grows rather than ending a conversation.
        let capacity = std::env::var("DSH_DESKTOP_POOL_SIZE")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(3);

        // Workspace: the setting wins, then the environment, then the process cwd. A
        // workspace that has been deleted must not leave the app unable to start.
        let workspace = Settings::load(&home)
            .workspace
            .map(PathBuf::from)
            .filter(|path| path.is_dir())
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| home.clone());

        Self {
            workspace,
            dsh_home: home.clone(),
            kernel_dir,
            bridge_dir,
            pool: KernelPool::new(capacity),
            initialized: std::sync::Mutex::new(std::collections::HashMap::new()),
            forwarded: Mutex::new(std::collections::HashSet::new()),
            route: Mutex::new(Route::new(
                Settings::load(&home).provider,
                Settings::load(&home).model,
            )),
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
    /// The route new sessions should use.
    pub fn current_route(&self) -> Route {
        self.route.lock().expect("route lock").clone()
    }

    /// Point new sessions at a different route. Existing sessions keep their own kernel, so
    /// this changes what happens *next*, never what is already running.
    pub fn set_current_route(&self, route: Route) {
        *self.route.lock().expect("route lock") = route;
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

impl AppState {
    /// Write one key into `provider.env`, replacing any existing line for it.
    ///
    /// Two reasons this is a file and not the environment: the kernel reads this file itself,
    /// so a key written here survives a restart without this process re-exporting it; and
    /// `set_var` would hand the secret to every child process spawned afterwards.
    ///
    /// The file is created with owner-only permissions where the platform has them. On Windows
    /// the parent directory already sits under the user profile; tightening the ACL further
    /// would need an `icacls` call per write, which is a decision for whoever owns the policy.
    pub fn write_secret(&self, name: &str, value: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.dsh_home).map_err(|error| error.to_string())?;
        let path = self.dsh_home.join("provider.env");

        let mut lines: Vec<String> = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter(|line| !line.trim_start().starts_with(&format!("{name}=")))
            .map(str::to_owned)
            .collect();
        lines.push(format!("{name}={value}"));

        std::fs::write(&path, format!("{}\n", lines.join("\n")))
            .map_err(|error| format!("{}: {error}", path.display()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }

        // The running kernels already have their environment; the next start picks this up.
        Ok(())
    }
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

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dsh-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn a_missing_settings_file_is_the_default_route() {
        let home = scratch("missing");
        let settings = Settings::load(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(settings.provider, "opencode-go");
        assert!(settings.model.contains("flash"));
    }

    #[test]
    fn settings_round_trip() {
        let home = scratch("roundtrip");
        Settings {
            provider: "deepseek-official".to_owned(),
            model: "deepseek-v4.1-pro".to_owned(),
            last_session_id: Some("abc".to_owned()),
            workspace: Some("D:/repo".to_owned()),
        }
        .save(&home)
        .expect("save");

        let loaded = Settings::load(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(loaded.provider, "deepseek-official");
        assert_eq!(loaded.model, "deepseek-v4.1-pro");
    }

    #[test]
    fn a_utf8_bom_is_not_a_syntax_error() {
        // PowerShell's `Set-Content -Encoding utf8` and Notepad both emit a BOM.
        // Without stripping it serde_json reports a syntax error and the app
        // silently runs on the defaults — the user sees their edit do nothing.
        let home = scratch("bom");
        let mut body = String::new();
        body.push(char::from_u32(0xFEFF).expect("BOM"));
        body.push_str(r#"{"provider":"p","model":"m"}"#);
        std::fs::write(home.join("settings.json"), body).expect("write");

        let settings = Settings::load(&home);
        let _ = std::fs::remove_dir_all(&home);

        assert_eq!(settings.provider, "p");
        assert_eq!(settings.model, "m");
    }

    #[test]
    fn a_broken_settings_file_falls_back_instead_of_refusing_to_start() {
        let home = scratch("broken");
        std::fs::write(home.join("settings.json"), "{ not json").expect("write");
        let settings = Settings::load(&home);
        let _ = std::fs::remove_dir_all(&home);
        assert_eq!(settings.provider, "opencode-go");
    }
}
