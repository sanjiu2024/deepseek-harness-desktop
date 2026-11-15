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

/// How many shells may exist at once.
///
/// Each one is a real process with a console and a reader thread. A UI bug that opens one per
/// render would otherwise end with a hundred PowerShell processes and no obvious cause.
const MAX_TERMINALS: usize = 8;

/// Open shells, by the id the front end minted.
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
    let pty = Pty::spawn(
        &cwd,
        PtySize { rows, cols },
        move |event| match event {
            PtyEvent::Data(bytes) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                let _ = handle.emit(PTY_DATA_EVENT, DataPayload {
                    id: label.clone(),
                    data: encoded,
                });
            }
            PtyEvent::Exited(code) => {
                let _ = handle.emit(PTY_EXIT_EVENT, ExitPayload {
                    id: label.clone(),
                    code,
                });
            }
        },
    )?;

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
    let pty = terminals
        .shells
        .lock()
        .expect("terminals lock")
        .remove(&id);
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
) -> Result<tree::Listing, String> {
    let requested = path.unwrap_or_else(|| state.workspace.to_string_lossy().into_owned());
    tree::list(
        &state.workspace,
        std::path::Path::new(&requested),
        show_noisy.unwrap_or(false),
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn fs_read(
    state: State<'_, AppState>,
    path: String,
    max_bytes: Option<u64>,
) -> Result<dsh_native::Decoded, String> {
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
