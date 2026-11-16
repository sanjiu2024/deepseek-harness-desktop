//! Tauri command surface. Each command is a thin wrapper: parse, delegate to a
//! crate, map the error. If a body grows past a dozen lines, the logic belongs
//! in `crates/`.

use std::sync::Arc;
use std::time::Duration;

use dsh_runtime::{Incoming, KernelClient, KernelConfig, ResponsePayload, Route};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast::error::RecvError;

use crate::state::{AppState, Settings};

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
    /// `DSH_DESKTOP_AUTOSTART=1` — the launch screen starts the kernel itself.
    /// Useful for a kiosk-style launch, and for exercising the full path without
    /// a click.
    pub auto_start: bool,
}

/// Where the kernel lives and whether it is present, for the setup screen.
#[tauri::command]
pub fn kernel_paths(state: State<'_, AppState>) -> KernelPaths {
    let entry = dsh_runtime::kernel_entry(&state.kernel_dir);
    tracing::debug!(target: "dsh_desktop_app", fetched = entry.exists(), auto = std::env::var("DSH_DESKTOP_AUTOSTART").is_ok(), "kernel_paths invoked");
    KernelPaths {
        home: state.dsh_home.to_string_lossy().into_owned(),
        kernel_dir: state.kernel_dir.to_string_lossy().into_owned(),
        fetched: entry.exists(),
        entry: entry.to_string_lossy().into_owned(),
        auto_start: std::env::var("DSH_DESKTOP_AUTOSTART").is_ok_and(|value| value == "1"),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KernelStarted {
    pub pid: Option<u32>,
    /// The route this kernel serves; the UI shows it instead of guessing from settings.
    pub provider: Option<String>,
    pub model: Option<String>,
}

/// Start the kernel for the current route and forward its frames as Tauri events.
///
/// Idempotent per route: the pool returns the running kernel for a route instead of
/// spawning a second one. Different routes get different processes — a kernel pins its
/// model at `initialize`, so "switch model" can only mean "run another kernel".
#[tauri::command]
pub async fn kernel_start(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<KernelStarted, String> {
    let route = state.current_route();
    let _client = acquire(&app, &state, &route).await?;
    Ok(KernelStarted {
        pid: None,
        provider: Some(route.provider),
        model: Some(route.model),
    })
}

/// Get this route's kernel, starting it when the pool does not have one.
async fn acquire(
    app: &AppHandle,
    state: &AppState,
    route: &Route,
) -> Result<Arc<KernelClient>, String> {
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

    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    let env = state.provider_env();
    let home = state.dsh_home.clone();
    let client = state
        .pool
        .acquire(route, move || {
            let entry = entry.clone();
            let home = home.clone();
            let env = env.clone();
            let cwd = cwd.clone();
            async move {
                KernelClient::spawn(KernelConfig {
                    node: std::env::var_os("DSH_NODE")
                        .map(Into::into)
                        .or_else(|| {
                            crate::state::alongside_exe(if cfg!(windows) {
                                "node.exe"
                            } else {
                                "node"
                            })
                        })
                        .unwrap_or_else(|| "node".into()),
                    entry,
                    profile: "sdk".to_owned(),
                    patches: vec![patch_path()],
                    home,
                    cwd,
                    env,
                })
                .await
                .map_err(|error| error.to_string())
            }
        })
        .await?;

    // Attach the readers once per kernel: on reuse a second attach would double every frame,
    // which looks exactly like the kernel sending each chunk twice.
    if state
        .forwarded
        .lock()
        .expect("forward lock")
        .insert(route.clone())
    {
        forward_frames(app, &client);
        forward_exit(app, &client);
    }
    Ok(client)
}

/// Perform the protocol handshake. Must succeed before any session is created.
#[tauri::command]
pub async fn kernel_initialize(
    app: AppHandle,
    state: State<'_, AppState>,
    cwd: String,
    provider: String,
    model: String,
    reasoning_effort: Option<String>,
) -> Result<serde_json::Value, String> {
    let route = Route::new(provider.clone(), model.clone());
    let client = acquire(&app, &state, &route).await?;
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

    let result = client.initialize(params, Duration::from_secs(30)).await;
    match &result {
        Ok(value) => tracing::debug!(target: "dsh_desktop_app", ?value, "kernel_initialize ok"),
        Err(error) => tracing::warn!(target: "dsh_desktop_app", %error, "kernel_initialize failed"),
    }
    if result.is_ok() {
        state.set_current_route(route);
    }
    result.map_err(|error| error.to_string())
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
    let route = state.current_route();
    let client = state
        .pool
        .existing(&route)
        .await
        .ok_or_else(|| "the kernel is not running".to_owned())?;
    ensure_known_session(&state, &client, &route, &method, &params).await;
    let timeout = timeout_ms.map(Duration::from_millis);
    client
        .call(&method, params, timeout)
        .await
        .map_err(|error| error.to_string())
}

/// Answer a kernel-initiated request, e.g. an approval prompt.
///
/// The id is passed through as JSON because the kernel issues **string** ids
/// (`req_<uuid>`) for the requests it sends; assuming numbers here is what once
/// made every approval look like a notification.
#[tauri::command]
pub async fn kernel_answer(
    app: AppHandle,
    state: State<'_, AppState>,
    id: serde_json::Value,
    result: serde_json::Value,
) -> Result<(), String> {
    let id = match &id {
        serde_json::Value::Number(number) => number
            .as_u64()
            .map(dsh_runtime::FrameId::Number)
            .ok_or_else(|| format!("unsupported numeric id: {id}"))?,
        serde_json::Value::String(text) => dsh_runtime::FrameId::Text(text.clone()),
        other => return Err(format!("unsupported id: {other}")),
    };
    trace_answer(&app, &id, &result);
    let client = state
        .pool
        .existing(&state.current_route())
        .await
        .ok_or_else(|| "the kernel is not running".to_owned())?;
    client
        .answer(&id, ResponsePayload::Ok(result))
        .map_err(|error| error.to_string())
}

/// The kernel's recent stderr, for the diagnostics panel.
#[tauri::command]
pub fn kernel_diagnostics(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    // Synchronous on purpose: a diagnostic read that spawns a process as a side effect is
    // not a diagnostic. The pool is async, so this reports nothing rather than misreporting.
    let _ = state;
    Ok(Vec::new())
}

/// Stop the kernel and clear it from state.
#[tauri::command]
pub async fn kernel_stop(state: State<'_, AppState>) -> Result<(), String> {
    state.pool.shutdown_all(Duration::from_secs(5)).await;
    state.forwarded.lock().expect("forward lock").clear();
    Ok(())
}

fn patch_path() -> std::path::PathBuf {
    if let Some(custom) = std::env::var_os("DSH_DESKTOP_PATCH") {
        return custom.into();
    }
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../profiles/sdk/cordis.patch.yml")
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
            trace_frame(app, &value);
            let _ = app.emit(KERNEL_EVENT, value);
        }
        Err(error) => {
            tracing::warn!(target: "dsh_desktop_app", %error, "dropped an unserialisable frame")
        }
    }
}

/// Append the answer to the same log, so "did we send it?" and "did the kernel
/// take it?" stop being a guessing game.
fn trace_answer(app: &AppHandle, id: &dsh_runtime::FrameId, result: &serde_json::Value) {
    if std::env::var_os("DSH_DESKTOP_LOG_FRAMES").is_none() {
        return;
    }
    let state = app.state::<AppState>();
    let path = state.dsh_home.join("frames.log");
    let outcome = result
        .get("outcome")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let line = format!("answer\t{id}\t{outcome}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}

/// Append one line per forwarded frame when `DSH_DESKTOP_LOG_FRAMES` is set.
///
/// "The dialog never appeared" has two very different causes — the frame never
/// reached the client, or it reached the client and the view did not draw it —
/// and they need opposite fixes. A log of what was actually forwarded separates
/// them without anyone reading a screenshot.
fn trace_frame(app: &AppHandle, value: &serde_json::Value) {
    if std::env::var_os("DSH_DESKTOP_LOG_FRAMES").is_none() {
        return;
    }
    let state = app.state::<AppState>();
    let path = state.dsh_home.join("frames.log");
    let kind = value
        .get("kind")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("?");
    let method = value
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("-");
    let line = format!("{kind}\t{method}\n");
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = file.write_all(line.as_bytes());
    }
}

/// The route the client should ask for, plus whether a credential is present.
///
/// The credential itself never crosses this boundary: the UI only needs to know
/// whether starting a kernel will work, not what the key is.
#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> SettingsSnapshot {
    let settings = Settings::load(&state.dsh_home);
    tracing::debug!(target: "dsh_desktop_app", provider = %settings.provider, model = %settings.model, has_key = credential_present(), "settings_get");
    SettingsSnapshot {
        provider: settings.provider,
        model: settings.model,
        has_credential: credential_present(),
        last_session_id: settings.last_session_id,
        workspace: settings.workspace,
    }
}

/// Persist a new route. Takes effect on the next kernel start.
#[tauri::command]
pub fn settings_set(
    state: State<'_, AppState>,
    provider: String,
    model: String,
) -> Result<SettingsSnapshot, String> {
    let provider = provider.trim().to_owned();
    let model = model.trim().to_owned();
    if provider.is_empty() || model.is_empty() {
        return Err("provider and model must not be empty".to_owned());
    }
    // Load, modify, save: constructing a fresh `Settings` here would silently erase the
    // session and workspace fields the user set somewhere else.
    let mut settings = Settings::load(&state.dsh_home);
    settings.provider = provider;
    settings.model = model;
    settings.save(&state.dsh_home)?;
    Ok(snapshot(&state.dsh_home))
}

/// Remember which session to reopen next launch.
#[tauri::command]
pub fn settings_remember_session(
    state: State<'_, AppState>,
    id: Option<String>,
) -> Result<(), String> {
    let mut settings = Settings::load(&state.dsh_home);
    settings.last_session_id = id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    settings.save(&state.dsh_home)
}

/// Point the file tree at another root. Must be an existing directory.
#[tauri::command]
pub fn settings_set_workspace(
    state: State<'_, AppState>,
    path: String,
) -> Result<SettingsSnapshot, String> {
    let trimmed = path.trim();
    let candidate = std::path::Path::new(trimmed);
    if !candidate.is_dir() {
        return Err(format!("not a directory: {trimmed}"));
    }
    let mut settings = Settings::load(&state.dsh_home);
    // Absolute, or a relative path would mean something different on the next launch.
    settings.workspace = Some(
        std::fs::canonicalize(candidate)
            .unwrap_or_else(|_| candidate.to_path_buf())
            .to_string_lossy()
            .into_owned(),
    );
    settings.save(&state.dsh_home)?;
    Ok(snapshot(&state.dsh_home))
}

/// Store a provider key.
///
/// The value goes to `<dsh_home>/provider.env`, which is the file the kernel already reads,
/// and it never enters this process's environment: exporting it here would leak it into every
/// child process for the rest of the run.
#[tauri::command]
pub fn settings_set_secret(
    state: State<'_, AppState>,
    name: String,
    value: String,
) -> Result<SettingsSnapshot, String> {
    let name = name.trim().to_owned();
    let value = value.trim().to_owned();
    if !name
        .chars()
        .all(|character| character.is_ascii_uppercase() || character == '_')
    {
        return Err(format!("not an environment variable name: {name}"));
    }
    if value.is_empty() {
        return Err("the key must not be empty".to_owned());
    }
    state.write_secret(&name, &value)?;
    Ok(snapshot(&state.dsh_home))
}

fn snapshot(home: &std::path::Path) -> SettingsSnapshot {
    let settings = Settings::load(home);
    SettingsSnapshot {
        provider: settings.provider,
        model: settings.model,
        has_credential: credential_present(),
        last_session_id: settings.last_session_id,
        workspace: settings.workspace,
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSnapshot {
    pub provider: String,
    pub model: String,
    /// True when some provider key is in the environment.
    pub has_credential: bool,
    /// Session to reopen on launch, when it still exists.
    pub last_session_id: Option<String>,
    /// Root the file tree browses.
    pub workspace: Option<String>,
}

/// Does any provider key exist in this process?
///
/// Whether the key came from the environment or from  is a detail the
/// user cannot act on; "no key at all" is the thing that stops a turn.
fn credential_present() -> bool {
    ["DEEPSEEK_API_KEY", "OPENCODE_API_KEY"]
        .iter()
        .any(|key| std::env::var_os(key).is_some())
}

/// Every session on disk, newest first.
///
/// The kernel keeps no index and exposes no "list" call, so this reads the store
/// the kernel is already writing. A session the kernel has never flushed simply is
/// not listed; nothing here can corrupt one.
#[tauri::command]
pub fn session_list(state: State<'_, AppState>) -> Result<Vec<dsh_store::SessionSummary>, String> {
    let listed = dsh_store::SessionStore::new(&state.dsh_home).list();
    match &listed {
        Ok(sessions) => {
            tracing::debug!(target: "dsh_desktop_app", count = sessions.len(), "session_list")
        }
        Err(error) => tracing::warn!(target: "dsh_desktop_app", %error, "session_list failed"),
    }
    listed.map_err(|error| error.to_string())
}

/// One session's full history, for opening it in the view.
#[tauri::command]
pub fn session_load(
    state: State<'_, AppState>,
    id: String,
) -> Result<dsh_store::SessionHistory, String> {
    dsh_store::SessionStore::new(&state.dsh_home)
        .load(&id)
        .map_err(|error| error.to_string())
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
        session_list,
        session_load,
        session_search,
        settings_get,
        settings_set,
        settings_remember_session,
        settings_set_workspace,
        settings_set_secret,
        session_rename,
        session_delete,
        session_purge,
        session_fork,
        session_stats,
        session_trash,
        session_empty_trash,
        session_export,
        session_export_to,
        kernel_stop,
        crate::workspace::pty_open,
        crate::workspace::pty_write,
        crate::workspace::pty_resize,
        crate::workspace::pty_close,
        crate::workspace::fs_tree,
        crate::workspace::fs_read,
        kernel_route_switch,
    ]
}

#[allow(dead_code)]
fn _assert_state_is_managed(app: &AppHandle) -> bool {
    app.try_state::<AppState>().is_some()
}

/// Rename a session. The kernel's log is untouched; see `dsh_store::SessionTitles`.
#[tauri::command]
pub fn session_rename(state: State<'_, AppState>, id: String, title: String) -> Result<(), String> {
    dsh_store::SessionStore::new(&state.dsh_home)
        .rename(&id, &title)
        .map_err(|error| error.to_string())
}

/// Delete a session from disk. Irreversible — the UI confirms first.
#[tauri::command]
pub fn session_delete(state: State<'_, AppState>, id: String) -> Result<(), String> {
    tracing::info!(target: "dsh_desktop_app", %id, "session_delete");
    dsh_store::SessionStore::new(&state.dsh_home)
        .delete(&id)
        .map_err(|error| error.to_string())
}

/// Remove a session for good. Nothing recovers this — `session_delete` only moves it to trash.
#[tauri::command]
pub fn session_purge(state: State<'_, AppState>, id: String) -> Result<(), String> {
    tracing::warn!(target: "dsh_desktop_app", %id, "session_purge");
    dsh_store::SessionStore::new(&state.dsh_home)
        .purge(&id)
        .map_err(|error| error.to_string())
}

/// One session rendered as Markdown, for export.
#[tauri::command]
pub fn session_export(state: State<'_, AppState>, id: String) -> Result<String, String> {
    let history = dsh_store::SessionStore::new(&state.dsh_home)
        .load(&id)
        .map_err(|error| error.to_string())?;
    Ok(dsh_store::render_markdown(&history))
}

/// Export a session straight to a path chosen by the user.
#[tauri::command]
pub fn session_export_to(
    state: State<'_, AppState>,
    id: String,
    path: String,
) -> Result<(), String> {
    let history = dsh_store::SessionStore::new(&state.dsh_home)
        .load(&id)
        .map_err(|error| error.to_string())?;
    let markdown = dsh_store::render_markdown(&history);
    std::fs::write(&path, markdown).map_err(|error| format!("{path}: {error}"))
}

/// Switch the route new sessions start on, without restarting the app.
///
/// The kernel pins its model at `initialize`, so this starts (or reuses) a kernel for the
/// new route and leaves the old one running: sessions already on it keep working, which is
/// the whole reason a pool exists.
#[tauri::command]
pub async fn kernel_route_switch(
    app: AppHandle,
    state: State<'_, AppState>,
    provider: String,
    model: String,
) -> Result<RouteSwitched, String> {
    let route = Route::new(provider, model);
    let client = acquire(&app, &state, &route).await?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    client
        .initialize(
            serde_json::json!({
                "cwd": cwd.to_string_lossy(),
                "provider": route.provider,
                "model": route.model,
                "approvalRequests": true,
                "capabilities": { "userQuestions": true },
            }),
            Duration::from_secs(30),
        )
        .await
        .map_err(|error| error.to_string())?;
    state.set_current_route(route.clone());
    tracing::info!(target: "dsh_desktop_app", route = %route, "kernel_route_switch");
    Ok(RouteSwitched {
        provider: route.provider,
        model: route.model,
    })
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteSwitched {
    pub provider: String,
    pub model: String,
}

/// Search the text of every stored session.
#[tauri::command]
pub fn session_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> Result<dsh_store::SearchOutcome, String> {
    dsh_store::SessionStore::new(&state.dsh_home)
        .search(&query, limit.unwrap_or(40))
        .map_err(|error| error.to_string())
}

/// Make the kernel aware of a session before it is used.
///
/// A session id from the sidebar belongs to a session the *store* has, but a freshly started
/// kernel has never heard of it: `session/prompt` with an unknown id either fails or creates a
/// second session under a name that already exists. `session/resume` is the SDK's answer, and
/// doing it here means the UI does not have to know the difference between "the session I just
/// created" and "the session I opened from last week".
///
/// Only runs for the two methods that take a sessionId, and only once per session per kernel.
async fn ensure_known_session(
    state: &AppState,
    client: &Arc<KernelClient>,
    route: &Route,
    method: &str,
    params: &serde_json::Value,
) {
    if method != "session/prompt" && method != "session/query" {
        return;
    }
    let Some(session_id) = params.get("sessionId").and_then(serde_json::Value::as_str) else {
        return;
    };
    if state.pool.route_of(session_id).await.is_some() {
        return;
    }
    if !dsh_store::SessionStore::new(&state.dsh_home).contains(session_id) {
        // Not on disk: a genuinely new session, which the kernel creates on first prompt.
        return;
    }

    let resumed = client
        .call(
            "session/resume",
            serde_json::json!({ "sessionId": session_id }),
            Some(Duration::from_secs(30)),
        )
        .await;
    match resumed {
        Ok(_) => {
            tracing::debug!(target: "dsh_desktop_app", %session_id, "resumed a stored session");
            state.pool.bind_session(route, session_id).await;
        }
        // Resuming is a courtesy, not a gate: if it fails the prompt may still work, and
        // refusing to send would turn a warning into a broken app.
        Err(error) => {
            tracing::warn!(target: "dsh_desktop_app", %session_id, %error, "session/resume failed; prompting anyway");
        }
    }
}

/// What is sitting in the trash, with sizes.
#[tauri::command]
pub fn session_trash(state: State<'_, AppState>) -> Result<Vec<dsh_store::TrashEntry>, String> {
    dsh_store::SessionStore::new(&state.dsh_home)
        .trash()
        .map_err(|error| error.to_string())
}

/// Empty the trash. Returns how many sessions were removed.
#[tauri::command]
pub fn session_empty_trash(state: State<'_, AppState>) -> Result<usize, String> {
    tracing::warn!(target: "dsh_desktop_app", "session_empty_trash");
    dsh_store::SessionStore::new(&state.dsh_home)
        .empty_trash()
        .map_err(|error| error.to_string())
}

/// How much room the session store takes.
#[tauri::command]
pub fn session_stats(state: State<'_, AppState>) -> Result<dsh_store::StorageStats, String> {
    dsh_store::SessionStore::new(&state.dsh_home)
        .stats()
        .map_err(|error| error.to_string())
}

/// Copy a session into a new one.
///
/// The copy is a real session the kernel will resume — verified against a live kernel, not
/// assumed: a log copied into the right project directory and given a valid header is accepted
/// exactly like one the kernel wrote itself.
#[tauri::command]
pub fn session_fork(
    state: State<'_, AppState>,
    id: String,
    title: Option<String>,
) -> Result<dsh_store::ForkOutcome, String> {
    let new_id = format!("fork-{}", uuid_like());
    tracing::info!(target: "dsh_desktop_app", source = %id, fork = %new_id, "session_fork");
    dsh_store::SessionStore::new(&state.dsh_home)
        .fork(&id, &new_id, title.as_deref())
        .map_err(|error| error.to_string())
}

/// A unique suffix without pulling in a uuid crate for one call.
fn uuid_like() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|delta| delta.as_nanos())
        .unwrap_or_default();
    format!("{now:x}-{:x}", std::process::id())
}
