//! Watching what the agent does to the workspace, for an IDE-style view.
//!
//! The kernel tells the model "replaced 1 occurrence" and tells us nothing at all. What an IDE view
//! needs is different: **which file**, **which lines**, and **live**, while the turn is running.
//!
//! Everything here is derived from two durable events the kernel already sends:
//!
//! | event | what it gives us |
//! | --- | --- |
//! | `tool/call` | tool name and arguments — the files being touched, before anything changes |
//! | `tool/result` | the call finished — now each file can be compared with its snapshot |
//!
//! The snapshot is taken at call time and compared at result time, so the diff is **what actually
//! landed on disk**, not what the model said it did. A model that reports success and writes
//! nothing produces no hunk, which is the honest outcome.
//!
//! Nothing here parses tool *output*: the file is the truth.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;

/// Files larger than this are not snapshotted.
///
/// An IDE view of a 50 MB generated file helps nobody, and holding two copies of it in memory to
/// diff helps less.
const MAX_SNAPSHOT_BYTES: u64 = 2 * 1024 * 1024;

/// Tools that change files. `apply_patch` names its files inside the patch text, not in a field.
const EDITING_TOOLS: &[&str] = &["edit", "write", "apply_patch"];

/// One file the agent touched.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChange {
    /// Absolute path, as the kernel saw it.
    pub file: String,
    /// Path relative to the workspace, for display.
    pub display: String,
    pub added: usize,
    pub deleted: usize,
    /// True when the diff was too large to compare line by line.
    pub coarse: bool,
}

/// What the agent is doing right now.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveEdit {
    pub session_id: String,
    pub call_id: String,
    /// `edit`, `write` or `apply_patch`.
    pub tool: String,
    /// The first file the call names — what a viewer should scroll to.
    pub file: String,
    pub display: String,
    /// False when the tool is about to create the file.
    pub existed: bool,
    /// Every file the call names. `apply_patch` can name several.
    pub files: Vec<String>,
}

/// A snapshot taken when a call started.
///
/// One call can touch several files: `apply_patch` names them inside its body. Each is snapshotted,
/// so a patch that rewrites two files reports both instead of silently showing only the first.
struct Snapshot {
    files: Vec<(PathBuf, Option<String>)>,
    session_id: String,
    tool: String,
}

/// What the IDE layer remembers between events.
#[derive(Default)]
pub struct IdeState {
    /// Snapshots by call id, waiting for their result.
    pending: Mutex<HashMap<String, Snapshot>>,
    /// The file the agent is working on, for a panel that opens mid-turn.
    active: Mutex<Option<ActiveEdit>>,
    /// Per-session totals since the app started, so a UI can show a running list.
    changed: Mutex<HashMap<String, Vec<FileChange>>>,
}

impl IdeState {
    /// A panel that opens while a turn is running asks here instead of waiting for the next event.
    pub fn active(&self) -> Option<ActiveEdit> {
        self.active.lock().ok().and_then(|active| active.clone())
    }

    /// Everything this session changed since the app started.
    pub fn changed(&self, session_id: &str) -> Vec<FileChange> {
        self.changed
            .lock()
            .ok()
            .and_then(|changed| changed.get(session_id).cloned())
            .unwrap_or_default()
    }
}

/// Paths a tool call will touch, resolved against the workspace.
///
/// `edit` and `write` carry `file_path`; `apply_patch` names files inside the patch body. A call
/// that names no file (a patch with only context, or a tool we do not know) returns nothing and is
/// simply not shown — guessing a path would put the wrong file on screen.
pub fn touched_paths(tool: &str, arguments: &str, workspace: &Path) -> Vec<PathBuf> {
    if !EDITING_TOOLS.contains(&tool) {
        return Vec::new();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(arguments) else {
        return Vec::new();
    };

    let mut raw: Vec<String> = Vec::new();
    if let Some(path) = value.get("file_path").and_then(serde_json::Value::as_str) {
        raw.push(path.to_owned());
    }
    if let Some(patch) = value.get("patch").and_then(serde_json::Value::as_str) {
        for line in patch.lines() {
            for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
                if let Some(path) = line.strip_prefix(marker) {
                    raw.push(path.trim().to_owned());
                }
            }
        }
    }

    raw.into_iter()
        .map(|path| {
            let candidate = Path::new(&path);
            if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                workspace.join(candidate)
            }
        })
        .collect()
}

/// Read a file for snapshotting, or `None` when it is too large, binary, or absent.
fn read_text(path: &Path) -> Option<String> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_SNAPSHOT_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    String::from_utf8(bytes).ok()
}

/// Display path: relative to the workspace when possible, because an absolute Windows path eats the
/// whole tab title.
fn display(path: &Path, workspace: &Path) -> String {
    path.strip_prefix(workspace)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Start watching a call, and describe the edit as active.
pub fn begin(
    state: &IdeState,
    workspace: &Path,
    session_id: &str,
    call_id: &str,
    tool: &str,
    arguments: &str,
) -> Option<ActiveEdit> {
    let paths = touched_paths(tool, arguments, workspace);
    let file = paths.first()?.clone();

    let files: Vec<(PathBuf, Option<String>)> = paths
        .iter()
        .map(|path| (path.clone(), read_text(path)))
        .collect();
    let existed = files.first().is_some_and(|(_, before)| before.is_some());

    if let Ok(mut pending) = state.pending.lock() {
        pending.insert(
            call_id.to_owned(),
            Snapshot {
                files,
                session_id: session_id.to_owned(),
                tool: tool.to_owned(),
            },
        );
    }

    let active = ActiveEdit {
        session_id: session_id.to_owned(),
        call_id: call_id.to_owned(),
        tool: tool.to_owned(),
        file: file.to_string_lossy().into_owned(),
        display: display(&file, workspace),
        existed,
        files: paths
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .collect(),
    };
    if let Ok(mut current) = state.active.lock() {
        *current = Some(active.clone());
    }
    Some(active)
}

/// Finish watching a call: compare every file it named with its snapshot.
///
/// An empty result means the call wrote nothing we can see — a failed tool, a rejected approval, or
/// a file that is not text. Silence is correct there: reporting a change that did not happen is
/// worse than reporting none.
pub fn finish(
    state: &IdeState,
    workspace: &Path,
    call_id: &str,
) -> Vec<(String, dsh_native::diff::LineDiff)> {
    let Some(snapshot) = state
        .pending
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(call_id))
    else {
        return Vec::new();
    };

    let mut results = Vec::new();
    for (file, before) in &snapshot.files {
        let before = before.clone().unwrap_or_default();
        let after = read_text(file).unwrap_or_default();
        if before == after {
            continue;
        }
        let diff = dsh_native::diff::diff(&before, &after);
        if diff.is_empty() {
            continue;
        }

        let change = FileChange {
            file: file.to_string_lossy().into_owned(),
            display: display(file, workspace),
            added: diff.added,
            deleted: diff.deleted,
            coarse: diff.coarse,
        };
        if let Ok(mut changed) = state.changed.lock() {
            let list = changed.entry(snapshot.session_id.clone()).or_default();
            // One row per file: a file edited three times in a turn is still one file.
            match list.iter_mut().find(|entry| entry.file == change.file) {
                Some(existing) => {
                    existing.added += change.added;
                    existing.deleted += change.deleted;
                    existing.coarse |= change.coarse;
                }
                None => list.push(change),
            }
        }
        results.push((display(file, workspace), diff));
    }

    if !results.is_empty()
        && let Ok(mut current) = state.active.lock()
        && current
            .as_ref()
            .is_some_and(|active| active.call_id == call_id)
    {
        *current = None;
    }
    let _ = &snapshot.tool; // kept for a future "which tool touched this" column
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-ide-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        root
    }

    #[test]
    fn an_edit_call_names_its_file() {
        let workspace = Path::new("D:/repo");
        let paths = touched_paths(
            "edit",
            r#"{"file_path":"src/main.rs","old_string":"a","new_string":"b"}"#,
            workspace,
        );
        assert_eq!(paths, vec![workspace.join("src/main.rs")]);
    }

    #[test]
    fn a_patch_names_several_files() {
        let workspace = Path::new("D:/repo");
        let patch =
            "*** Begin Patch\n*** Update File: src/a.rs\n*** Add File: src/b.rs\n*** End Patch";
        let arguments = serde_json::json!({ "patch": patch }).to_string();
        let paths = touched_paths("apply_patch", &arguments, workspace);
        assert_eq!(
            paths,
            vec![workspace.join("src/a.rs"), workspace.join("src/b.rs")]
        );
    }

    #[test]
    fn a_tool_that_does_not_edit_names_nothing() {
        assert!(touched_paths("read", r#"{"file_path":"a.txt"}"#, Path::new("D:/r")).is_empty());
        assert!(touched_paths("bash", r#"{"command":"ls"}"#, Path::new("D:/r")).is_empty());
    }

    #[test]
    fn begin_then_finish_reports_the_lines_that_moved() {
        let workspace = scratch("flow");
        let file = workspace.join("a.txt");
        std::fs::write(&file, "one\ntwo\nthree\n").expect("write");
        let state = IdeState::default();

        let active = begin(
            &state,
            &workspace,
            "session-1",
            "call-1",
            "edit",
            &serde_json::json!({ "file_path": "a.txt" }).to_string(),
        )
        .expect("the call names a file");
        assert_eq!(active.display, "a.txt");
        assert!(active.existed);

        std::fs::write(&file, "one\nTWO\nthree\n").expect("rewrite");
        let finished = finish(&state, &workspace, "call-1");
        assert_eq!(finished.len(), 1);

        let (display_path, diff) = &finished[0];
        assert_eq!(display_path, "a.txt");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].removed, vec!["two"]);
        assert_eq!(diff.hunks[0].added, vec!["TWO"]);
        let totals = state.changed("session-1");
        assert_eq!(totals.len(), 1);
        assert_eq!((totals[0].added, totals[0].deleted), (1, 1));

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn a_write_that_changed_nothing_reports_nothing() {
        // A model that "succeeds" without touching the file must not produce a fake hunk.
        let workspace = scratch("noop");
        let file = workspace.join("a.txt");
        std::fs::write(&file, "same\n").expect("write");
        let state = IdeState::default();

        begin(
            &state,
            &workspace,
            "s",
            "c",
            "write",
            &serde_json::json!({ "file_path": "a.txt" }).to_string(),
        );
        assert!(finish(&state, &workspace, "c").is_empty());

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn creating_a_file_is_a_change_from_nothing() {
        let workspace = scratch("create");
        let state = IdeState::default();
        let active = begin(
            &state,
            &workspace,
            "s",
            "c",
            "write",
            &serde_json::json!({ "file_path": "new.txt" }).to_string(),
        )
        .expect("named");
        assert!(!active.existed, "the file did not exist yet");

        std::fs::write(workspace.join("new.txt"), "hello\n").expect("write");
        let finished = finish(&state, &workspace, "c");
        assert_eq!(finished.len(), 1);
        assert_eq!(finished[0].1.added, 1);
        assert_eq!(finished[0].1.deleted, 0);

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn the_active_edit_is_cleared_once_its_result_arrives() {
        let workspace = scratch("clear");
        let file = workspace.join("a.txt");
        std::fs::write(&file, "a\n").expect("write");
        let state = IdeState::default();

        begin(
            &state,
            &workspace,
            "s",
            "c",
            "edit",
            &serde_json::json!({ "file_path": "a.txt" }).to_string(),
        );
        assert!(state.active().is_some());
        std::fs::write(&file, "b\n").expect("rewrite");
        finish(&state, &workspace, "c");
        assert!(state.active().is_none(), "the turn moved on");

        let _ = std::fs::remove_dir_all(&workspace);
    }

    #[test]
    fn a_patch_that_touches_two_files_reports_both() {
        let workspace = scratch("multi");
        std::fs::write(workspace.join("a.txt"), "a1\n").expect("write");
        std::fs::write(workspace.join("b.txt"), "b1\n").expect("write");

        let state = IdeState::default();
        let patch =
            "*** Begin Patch\n*** Update File: a.txt\n*** Update File: b.txt\n*** End Patch";
        let arguments = serde_json::json!({ "patch": patch }).to_string();
        let active = begin(&state, &workspace, "s", "c", "apply_patch", &arguments).expect("named");
        assert_eq!(active.files.len(), 2);

        std::fs::write(workspace.join("a.txt"), "a2\n").expect("rewrite");
        std::fs::write(workspace.join("b.txt"), "b2\n").expect("rewrite");

        let finished = finish(&state, &workspace, "c");
        let names: Vec<&str> = finished.iter().map(|(file, _)| file.as_str()).collect();
        assert_eq!(
            names,
            vec!["a.txt", "b.txt"],
            "both files, not just the first"
        );

        let _ = std::fs::remove_dir_all(&workspace);
    }
}

/// One thing the IDE layer wants the window to know about.
///
/// Decoding and state live here rather than in the command layer so that a **real captured
/// session event** can drive the whole path in a test — no model, no window, no Tauri runtime.
#[derive(Debug, Clone)]
pub enum IdeEvent {
    Active(ActiveEdit),
    Edit {
        session_id: String,
        call_id: String,
        file: String,
        diff: dsh_native::diff::LineDiff,
    },
    Changes {
        session_id: String,
        files: Vec<FileChange>,
    },
}

/// Decode one `session.event` payload and update the IDE state.
///
/// Returns nothing for every other event type: the transcript path already handles the rest, and
/// this runs on every event the kernel sends, so it has to stay cheap.
pub fn observe(state: &IdeState, workspace: &Path, params: &serde_json::Value) -> Vec<IdeEvent> {
    let Some(event) = params.get("event") else {
        return Vec::new();
    };
    let Some(kind) = event.get("type").and_then(serde_json::Value::as_str) else {
        return Vec::new();
    };
    let Some(session_id) = params.get("sessionId").and_then(serde_json::Value::as_str) else {
        return Vec::new();
    };
    let data = event.get("data").unwrap_or(&serde_json::Value::Null);

    match kind {
        "tool/call" => {
            let tool = data
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let arguments = data
                .get("arguments")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            let call_id = data
                .get("callId")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return Vec::new();
            }
            match begin(state, workspace, session_id, call_id, tool, arguments) {
                Some(active) => vec![IdeEvent::Active(active)],
                None => Vec::new(),
            }
        }
        "tool/result" => {
            let call_id = data
                .get("message")
                .and_then(|message| message.get("toolCallId"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            if call_id.is_empty() {
                return Vec::new();
            }
            finish(state, workspace, call_id)
                .into_iter()
                .map(|(file, diff)| IdeEvent::Edit {
                    session_id: session_id.to_owned(),
                    call_id: call_id.to_owned(),
                    file,
                    diff,
                })
                .collect()
        }
        "turn/end" => vec![IdeEvent::Changes {
            session_id: session_id.to_owned(),
            files: state.changed(session_id),
        }],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod wire_tests {
    use super::*;

    /// Walk a real session log and return the first `write` tool call it contains.
    ///
    /// Reading the machine's own logs beats a hand-written fixture: a fixture only proves that we
    /// can copy a string into a test, while this fails the day the kernel renames a field.
    fn first_write_call(home: &Path) -> Option<serde_json::Value> {
        fn logs(dir: &Path, out: &mut Vec<PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    logs(&path, out);
                } else if path
                    .file_name()
                    .is_some_and(|name| name == "session.v4.jsonl")
                {
                    out.push(path);
                }
            }
        }

        let mut paths = Vec::new();
        logs(&home.join("sessions"), &mut paths);
        for path in paths {
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            for line in raw.lines() {
                // Both markers: a tool catalog in a request header also contains the tool name.
                if !line.contains("\"type\":\"tool/call\"") || !line.contains("\"name\":\"write\"")
                {
                    continue;
                }
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(line) {
                    return Some(event);
                }
            }
        }
        None
    }

    #[test]
    fn a_real_session_log_has_the_fields_we_parse() {
        let Ok(home) = std::env::var("DSH_DESKTOP_HOME") else {
            eprintln!("skipping: DSH_DESKTOP_HOME is not set");
            return;
        };
        let Some(call) = first_write_call(Path::new(&home)) else {
            eprintln!("skipping: no write call in this home");
            return;
        };

        // The exact fields `observe` reads. If the kernel renames one, this test says so.
        assert_eq!(call["type"], "tool/call");
        assert!(call["data"]["callId"].as_str().is_some());
        let arguments = call["data"]["arguments"]
            .as_str()
            .expect("arguments is a string");
        let parsed: serde_json::Value = serde_json::from_str(arguments).expect("arguments is JSON");
        assert!(
            parsed["file_path"].as_str().is_some(),
            "a write call names its file: {arguments}"
        );
    }

    #[test]
    fn a_captured_call_drives_the_whole_path() {
        // Same shapes as a real log, with the path pointed at a scratch directory so nothing real
        // is touched. The fixture above proves the shapes; this proves the state machine.
        let root = std::env::temp_dir().join(format!("dsh-ide-wire-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("scratch");
        let target = root.join("dsh-test.txt");
        let call_id = "call_a145b7bf485d4c77bbfbd8ef";

        let arguments = serde_json::json!({
            "file_path": target.to_string_lossy(),
            "content": "hello from pwsh\n",
        });
        let call = serde_json::json!({
            "type": "tool/call",
            "data": { "callId": call_id, "name": "write", "arguments": arguments.to_string() },
        });
        let state = IdeState::default();

        let events = observe(
            &state,
            &root,
            &serde_json::json!({ "sessionId": "session-1", "event": call }),
        );
        assert_eq!(events.len(), 1, "one active edit");
        match &events[0] {
            IdeEvent::Active(active) => {
                assert_eq!(active.tool, "write");
                assert_eq!(active.display, "dsh-test.txt");
                assert!(!active.existed, "the file did not exist yet");
            }
            other => panic!("expected an active edit, got {other:?}"),
        }

        std::fs::write(&target, "hello from pwsh\nmore\n").expect("write");
        let result = serde_json::json!({
            "type": "tool/result",
            "data": { "message": { "role": "tool", "toolCallId": call_id, "content": [] } },
        });
        let events = observe(
            &state,
            &root,
            &serde_json::json!({ "sessionId": "session-1", "event": result }),
        );
        assert_eq!(events.len(), 1, "one edit event");
        match &events[0] {
            IdeEvent::Edit { file, diff, .. } => {
                assert_eq!(file, "dsh-test.txt");
                assert_eq!(diff.added, 2);
                assert_eq!(diff.deleted, 0);
            }
            other => panic!("expected an edit, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn an_unrelated_event_costs_nothing() {
        let state = IdeState::default();
        let events = observe(
            &state,
            Path::new("."),
            &serde_json::json!({ "sessionId": "s", "event": { "type": "step/start", "data": {} } }),
        );
        assert!(events.is_empty());
    }
}
