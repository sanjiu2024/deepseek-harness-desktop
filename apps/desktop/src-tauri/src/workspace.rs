//! The side panel's two backends: the workspace tree and a shell.
//!
//! Kept apart from `commands.rs` because these are the only two long-lived things the app
//! owns — a PTY keeps a child process and a reader thread — and mixing their lifecycle into
//! the thin command wrappers is how a process leak starts.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use base64::Engine;
use dsh_native::pty::{Pty, PtyEvent, PtySize};
use dsh_native::tree;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::state::AppState;

/// Terminal output. Base64 because a Tauri event is JSON: an array of numbers would cost
/// roughly four bytes per byte on the wire, and xterm wants a byte array anyway.
pub const PTY_DATA_EVENT: &str = "pty://data";
/// One terminal ended.
pub const PTY_EXIT_EVENT: &str = "pty://exit";
/// Emitted when the watched directory level changed. Payload: `{ paths: string[] }`.
pub const FS_CHANGED_EVENT: &str = "fs://changed";

/// How often the watched directory is re-read.
///
/// 1.5 s is invisible to a person watching a tree and cheap for one directory level. A native
/// event backend would be instant and would also differ on every platform; see `dsh_native::watch`
/// for why that trade was made the other way.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1500);

/// How many shells may exist at once.
///
/// Each one is a real process with a console and a reader thread. A UI bug that opens one per
/// render would otherwise end with a hundred PowerShell processes and no obvious cause.
const MAX_TERMINALS: usize = 8;

/// Open shells, by the id the front end minted.
/// Caps how many previews are read at once; see `dsh_native::gate`.
pub struct Previews(pub dsh_native::Gate);

impl Default for Previews {
    fn default() -> Self {
        Self(dsh_native::Gate::new(dsh_native::DEFAULT_PREVIEW_LIMIT))
    }
}

/// One directory being watched, and the switch that stops it.
///
/// The tree expands one level at a time, so there is exactly one watch at a time: watching every
/// expanded directory would multiply the polling for no benefit — a collapse re-reads anyway.
#[derive(Default)]
pub struct Watcher {
    current: std::sync::Mutex<Option<(String, std::sync::Arc<std::sync::atomic::AtomicBool>)>>,
}

#[derive(Default)]
pub struct Terminals {
    shells: Mutex<HashMap<String, Arc<Pty>>>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PtyOpened {
    pub id: String,
    /// The shell that was started, shown in the tab so a mismatch is visible.
    pub shell: String,
}

#[tauri::command]
pub fn pty_open(
    app: AppHandle,
    state: State<'_, AppState>,
    terminals: State<'_, Terminals>,
    id: String,
    rows: u16,
    cols: u16,
) -> Result<PtyOpened, String> {
    let cwd = state.workspace.clone();
    let label = id.clone();
    let handle = app.clone();
    let pty = Pty::spawn(&cwd, PtySize { rows, cols }, move |event| match event {
        PtyEvent::Data(bytes) => {
            let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
            let _ = handle.emit(
                PTY_DATA_EVENT,
                DataPayload {
                    id: label.clone(),
                    data: encoded,
                },
            );
        }
        PtyEvent::Exited(code) => {
            let _ = handle.emit(
                PTY_EXIT_EVENT,
                ExitPayload {
                    id: label.clone(),
                    code,
                },
            );
        }
    })?;

    {
        let mut shells = terminals.shells.lock().expect("terminals lock");
        if shells.len() >= MAX_TERMINALS {
            pty.close();
            return Err(format!(
                "already {MAX_TERMINALS} terminals open — close one first"
            ));
        }
        shells.insert(id.clone(), Arc::new(pty));
    }

    Ok(PtyOpened {
        id,
        shell: std::env::var("DSH_DESKTOP_SHELL").unwrap_or_else(|_| "默认 shell".to_owned()),
    })
}

#[tauri::command]
pub fn pty_write(terminals: State<'_, Terminals>, id: String, data: String) -> Result<(), String> {
    let pty = terminals
        .shells
        .lock()
        .expect("terminals lock")
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("no terminal {id}"))?;
    pty.write(data.as_bytes())
}

#[tauri::command]
pub fn pty_resize(
    terminals: State<'_, Terminals>,
    id: String,
    rows: u16,
    cols: u16,
) -> Result<(), String> {
    let pty = terminals
        .shells
        .lock()
        .expect("terminals lock")
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("no terminal {id}"))?;
    pty.resize(PtySize { rows, cols })
}

#[tauri::command]
pub fn pty_close(terminals: State<'_, Terminals>, id: String) -> Result<(), String> {
    let pty = terminals.shells.lock().expect("terminals lock").remove(&id);
    if let Some(pty) = pty {
        pty.close();
    }
    Ok(())
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct DataPayload {
    id: String,
    data: String,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExitPayload {
    id: String,
    code: Option<u32>,
}

#[tauri::command]
pub fn fs_tree(
    state: State<'_, AppState>,
    path: Option<String>,
    show_noisy: Option<bool>,
    order: Option<tree::Order>,
) -> Result<tree::Listing, String> {
    let requested = path.unwrap_or_else(|| state.workspace.to_string_lossy().into_owned());
    tree::list(
        &state.workspace,
        std::path::Path::new(&requested),
        show_noisy.unwrap_or(false),
        order.unwrap_or_default(),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn fs_read(
    state: State<'_, AppState>,
    previews: State<'_, Previews>,
    path: String,
    max_bytes: Option<u64>,
) -> Result<dsh_native::Decoded, String> {
    // A permit, not a queue: a preview from ten clicks ago is no longer wanted, so the call
    // that arrives when the gate is full is refused with a reason instead of waiting.
    let _permit = previews
        .0
        .try_enter()
        .ok_or_else(|| "已经有 3 个文件在读，等一个读完再点".to_owned())?;
    tree::read_preview(
        &state.workspace,
        std::path::Path::new(&path),
        max_bytes.unwrap_or(256 * 1024),
    )
    .map_err(|error| error.to_string())
}

impl Terminals {
    /// Stop every shell. Called on exit; a PTY left behind keeps a console alive after the
    /// window is gone, and the next launch inherits it.
    pub fn close_all(&self) {
        let drained: Vec<Arc<Pty>> = {
            let mut shells = self.shells.lock().expect("terminals lock");
            shells.drain().map(|(_, pty)| pty).collect()
        };
        for pty in drained {
            pty.close();
        }
    }
}

/// Watch one directory level and emit `fs://changed` when it moves.
///
/// Starting a watch replaces the previous one: the tree only ever shows one level at a time, and
/// two watchers would emit duplicate events for the same click.
#[tauri::command]
pub fn fs_watch_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    watcher: State<'_, Watcher>,
    path: Option<String>,
) -> Result<(), String> {
    let root = state.workspace.clone();
    let target = match path {
        Some(path) => std::path::Path::new(&path).to_path_buf(),
        None => root.clone(),
    };
    // Containment is checked here rather than inside the loop: a watch outside the workspace is a
    // mistake worth reporting once, not every second and a half.
    let canonical_root = std::fs::canonicalize(&root).map_err(|error| error.to_string())?;
    let canonical_target = std::fs::canonicalize(&target).map_err(|error| error.to_string())?;
    if !canonical_target.starts_with(&canonical_root) {
        return Err(format!(
            "拒绝监听工作区之外的目录：{}",
            canonical_target.display()
        ));
    }

    let key = canonical_target.to_string_lossy().into_owned();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut current = watcher.current.lock().map_err(|error| error.to_string())?;
        if let Some((previous, previous_stop)) = current.take() {
            previous_stop.store(true, std::sync::atomic::Ordering::Relaxed);
            tracing::debug!(target: "dsh_desktop_app", path = %previous, "fs_watch replaced");
        }
        *current = Some((key.clone(), std::sync::Arc::clone(&stop)));
    }

    std::thread::spawn(move || {
        let mut snapshot = dsh_native::watch::scan(&canonical_target);
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            std::thread::sleep(WATCH_INTERVAL);
            if stop.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let next = dsh_native::watch::scan(&canonical_target);
            let changed = next.changed_since(&snapshot);
            snapshot = next;
            if changed.is_empty() {
                continue;
            }
            let paths: Vec<String> = changed
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            let _ = app.emit(FS_CHANGED_EVENT, serde_json::json!({ "paths": paths }));
        }
        tracing::debug!(target: "dsh_desktop_app", path = %key, "fs_watch stopped");
    });
    Ok(())
}

/// Stop watching. Called when the panel closes or the tree collapses.
#[tauri::command]
pub fn fs_watch_stop(watcher: State<'_, Watcher>) -> Result<(), String> {
    let mut current = watcher.current.lock().map_err(|error| error.to_string())?;
    if let Some((_, stop)) = current.take() {
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}
