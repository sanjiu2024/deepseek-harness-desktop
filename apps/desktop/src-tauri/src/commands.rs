//! Tauri command surface. Each command is a thin wrapper: parse, delegate to a
//! crate, map the error. If a body grows past a dozen lines, the logic belongs
//! in `crates/`.

use std::sync::Arc;
use std::time::Duration;

use dsh_runtime::{Incoming, KernelClient, KernelConfig, ResponsePayload};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast::error::RecvError;

use crate::state::AppState;

/// Event name the front end listens on for every kernel frame.
pub const KERNEL_EVENT: &str = "kernel://message";
/// Emitted when a subscriber fell behind and frames were dropped.
pub const KERNEL_LAGGED_EVENT: &str = "kernel://lagged";
/// Emitted when the kernel process is gone.
pub const KERNEL_EXIT_EVENT: &str = "kernel://exit";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

/// Static facts about the running application.
#[tauri::command]
pub fn app_info(app: AppHandle) -> AppInfo {
    let package = app.package_info();
    AppInfo {
        name: package.name.clone(),
        version: package.version.to_string(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelPaths {
    pub home: String,
    pub kernel_dir: String,
    pub entry: String,
    /// False when `pnpm kernel:fetch` has not run.
    pub fetched: bool,
}

/// Where the kernel lives and whether it is present, for the setup screen.
#[tauri::command]
pub fn kernel_paths(state: State<'_, AppState>) -> KernelPaths {
    let entry = dsh_runtime::kernel_entry(&state.kernel_dir);
    KernelPaths {
        home: state.dsh_home.to_string_lossy().into_owned(),
        kernel_dir: state.kernel_dir.to_string_lossy().into_owned(),
        fetched: entry.exists(),
        entry: entry.to_string_lossy().into_owned(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelStarted {
    pub pid: Option<u32>,
}

/// Start the kernel and begin forwarding its frames as Tauri events.
///
/// Idempotent: a second call returns the running process instead of spawning a
/// second one, because two kernels on one `DSH_HOME` fight over session locks.
#[tauri::command]
pub async fn kernel_start(app: AppHandle, state: State<'_, AppState>) -> Result<KernelStarted, String> {
    {
        let guard = state.kernel.lock().expect("kernel lock");
        if guard.is_some() {
            return Ok(KernelStarted { pid: None });
        }
    }

    let entry = dsh_runtime::kernel_entry(&state.kernel_dir);
    if !entry.exists() {
        return Err(format!(
            "kernel is not fetched — run `pnpm kernel:fetch` (expected {})",
            entry.display()
        ));
    }
    std::fs::create_dir_all(&state.dsh_home).map_err(|error| error.to_string())?;
    // The live bridge must exist before the kernel resolves the profile patch.
    state.install_bridge()?;

    let client = KernelClient::spawn(KernelConfig {
        node: std::env::var_os("DSH_NODE").map_or_else(|| "node".into(), Into::into),
        entry,
        profile: "sdk".to_owned(),
        patches: vec![patch_path()],
        home: state.dsh_home.clone(),
        cwd: std::env::current_dir().map_err(|error| error.to_string())?,
        env: provider_env(),
    })
    .await
    .map_err(|error| error.to_string())?;

    forward_frames(&app, &client);
    forward_exit(&app, &client);

    *state.kernel.lock().expect("kernel lock") = Some(Arc::new(client));
    Ok(KernelStarted { pid: None })
}

/// Perform the protocol handshake. Must succeed before any session is created.
#[tauri::command]
pub async fn kernel_initialize(
    state: State<'_, AppState>,
    cwd: String,
    provider: String,
    model: String,
    reasoning_effort: Option<String>,
) -> Result<serde_json::Value, String> {
    let client = state.running()?;
    let mut params = serde_json::json!({
        "cwd": cwd,
        "provider": provider,
        "model": model,
        "approvalRequests": true,
        "capabilities": { "userQuestions": true },
    });
    if let Some(effort) = reasoning_effort {
        params["reasoningEffort"] = serde_json::Value::String(effort);
    }

    client
        .initialize(params, Duration::from_secs(30))
        .await
        .map_err(|error| error.to_string())
}

/// Call any SDK method.
///
/// A generic door beats one command per method: the protocol is small, stable,
/// and versioned with the kernel pin, so eight typed wrappers would be eight
/// places to drift. Errors keep the kernel's own code and message.
#[tauri::command]
pub async fn kernel_call(
    state: State<'_, AppState>,
    method: String,
    params: serde_json::Value,
    timeout_ms: Option<u64>,
) -> Result<serde_json::Value, String> {
    let client = state.running()?;
    let timeout = timeout_ms.map(Duration::from_millis);
    client
        .call(&method, params, timeout)
        .await
        .map_err(|error| error.to_string())
}

/// Answer a kernel-initiated request, e.g. an approval prompt.
#[tauri::command]
pub fn kernel_answer(
    state: State<'_, AppState>,
    id: u64,
    result: serde_json::Value,
) -> Result<(), String> {
    state
        .running()?
        .answer(id, ResponsePayload::Ok(result))
        .map_err(|error| error.to_string())
}

/// The kernel's recent stderr, for the diagnostics panel.
#[tauri::command]
pub fn kernel_diagnostics(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    Ok(state.running()?.stderr_tail())
}

/// Stop the kernel and clear it from state.
#[tauri::command]
pub async fn kernel_stop(state: State<'_, AppState>) -> Result<(), String> {
    let client = state.kernel.lock().expect("kernel lock").take();
    if let Some(client) = client {
        client.shutdown(Duration::from_secs(5)).await;
    }
    Ok(())
}

fn patch_path() -> std::path::PathBuf {
    if let Some(custom) = std::env::var_os("DSH_DESKTOP_PATCH") {
        return custom.into();
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../profiles/sdk/cordis.patch.yml")
}

/// Keys the kernel may need, passed through only when the app itself has them.
///
/// Credentials reach the kernel through its environment, never through a file
/// in this repository — the profile patch names the variable, not the secret.
const PROVIDER_KEYS: &[&str] = &["DEEPSEEK_API_KEY", "OPENCODE_API_KEY"];

fn provider_env() -> Vec<(String, String)> {
    PROVIDER_KEYS
        .iter()
        .filter_map(|name| match std::env::var(name) {
            Ok(value) if !value.is_empty() => Some(((*name).to_owned(), value)),
            _ => None,
        })
        .collect()
}

fn forward_frames(app: &AppHandle, client: &KernelClient) {
    let mut frames = client.subscribe();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            match frames.recv().await {
                Ok(frame) => emit(&app, frame),
                Err(RecvError::Lagged(skipped)) => {
                    let _ = app.emit(KERNEL_LAGGED_EVENT, skipped);
                }
                Err(RecvError::Closed) => break,
            }
        }
    });
}

fn forward_exit(app: &AppHandle, client: &KernelClient) {
    let mut exited = client.subscribe_exit();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(status) = exited.recv().await {
            let _ = app.emit(KERNEL_EXIT_EVENT, status);
        }
    });
}

fn emit(app: &AppHandle, frame: Incoming) {
    // A serialisation failure here means the wire held something we cannot
    // describe; losing one frame beats crashing the forwarder.
    match serde_json::to_value(frame) {
        Ok(value) => {
            let _ = app.emit(KERNEL_EVENT, value);
        }
        Err(error) => tracing::warn!(target: "dsh_desktop_app", %error, "dropped an unserialisable frame"),
    }
}

/// Re-exported so `main.rs` can register everything in one list.
pub fn handlers() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        app_info,
        kernel_paths,
        kernel_start,
        kernel_initialize,
        kernel_call,
        kernel_answer,
        kernel_diagnostics,
        kernel_stop,
    ]
}

#[allow(dead_code)]
fn _assert_state_is_managed(app: &AppHandle) -> bool {
    app.try_state::<AppState>().is_some()
}
