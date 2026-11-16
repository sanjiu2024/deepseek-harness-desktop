//! Spawning the kernel and pumping its three stdio pipes.
//!
//! One reader task owns stdout and does nothing but classify frames: replies go
//! to the caller waiting on that id, everything else is broadcast. That split is
//! what lets a caller block on `initialize` while the kernel is simultaneously
//! asking for approval — answering requests from inside a waiting call would
//! deadlock the way upstream's README warns about.
//!
//! stderr is never merged into stdout. stdout is the protocol channel; a stray
//! log line there is a protocol violation we surface rather than swallow.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::protocol::{self, Incoming, ResponsePayload, RpcError};

/// Prefix the kernel-side live bridge writes to stderr; see `bridge/index.js`.
pub const LIVE_PREFIX: &str = "\u{1}DSH-LIVE ";

/// Notification method the live bridge frames are republished under.
pub const LIVE_CHUNK_METHOD: &str = "dsh-desktop/live-chunk";

/// How many inbound frames a slow consumer may fall behind before it starts
/// dropping them. A dropped batch is reported by the receiver, not silently
/// ignored.
const EVENT_BUFFER: usize = 1024;

/// stderr lines kept for the diagnostics panel.
const STDERR_HISTORY: usize = 200;

/// Default bound on the `initialize` handshake.
pub const DEFAULT_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);

/// How to launch one kernel process.
#[derive(Debug, Clone)]
pub struct KernelConfig {
    /// Node executable. Packaged builds point at the bundled runtime.
    pub node: PathBuf,
    /// The kernel's `lib/bin.js`.
    pub entry: PathBuf,
    /// Profile name, normally `sdk`.
    pub profile: String,
    /// Extra patch overlays applied after the profile.
    pub patches: Vec<PathBuf>,
    /// Private kernel home; never the user's `~/.dsh`.
    pub home: PathBuf,
    /// Directory the kernel process itself runs in.
    pub cwd: PathBuf,
    /// Extra environment entries, e.g. the provider API key.
    pub env: Vec<(String, String)>,
}

impl KernelConfig {
    /// Build the argv for this configuration.
    pub fn args(&self) -> Vec<String> {
        let mut args = vec![self.entry.to_string_lossy().into_owned()];
        args.push("--profile".to_owned());
        args.push(self.profile.clone());
        for patch in &self.patches {
            args.push("--patch".to_owned());
            args.push(patch.to_string_lossy().into_owned());
        }
        args
    }
}

/// Why a kernel interaction failed.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error("failed to spawn the kernel at {path}: {source}")]
    Spawn {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("the kernel process is not running")]
    NotRunning,
    #[error("kernel rejected {method}: [{code}] {message}")]
    Rpc {
        method: String,
        code: i64,
        message: String,
    },
    #[error("{method} did not answer within {timeout:?}")]
    Timeout { method: String, timeout: Duration },
    #[error("protocol: {0}")]
    Protocol(#[from] protocol::ProtocolError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// A live kernel process plus the plumbing to talk to it.
pub struct KernelClient {
    /// Lines queued for stdin. Every write goes through this one channel so two
    /// callers can never interleave half a frame.
    outbound: mpsc::UnboundedSender<String>,
    pending: Arc<StdMutex<HashMap<u64, PendingCall>>>,
    next_id: AtomicU64,
    events: broadcast::Sender<Incoming>,
    stderr_tail: Arc<StdMutex<std::collections::VecDeque<String>>>,
    child: Arc<StdMutex<Option<Child>>>,
    exited: broadcast::Sender<ExitStatus>,
    /// Set by `shutdown`/`terminate` so the exit watcher can tell a requested stop from a
    /// crash. Without it every death looks the same, and a crash is the one worth reporting.
    stop_requested: Arc<std::sync::atomic::AtomicBool>,
}

struct PendingCall {
    sender: oneshot::Sender<ResponsePayload>,
    method: String,
}

/// How the kernel process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExitStatus {
    pub code: Option<i32>,
    /// True when the client tore the process down rather than the kernel dying.
    pub requested: bool,
}

impl KernelClient {
    /// Start a kernel and wait until its stdio pipes are up.
    ///
    /// This does not perform the protocol handshake; call [`Self::initialize`]
    /// for that. Splitting the two is what lets a caller pick provider and
    /// model after the process exists but before any turn is created.
    pub async fn spawn(config: KernelConfig) -> Result<Self, RuntimeError> {
        let mut command = Command::new(&config.node);
        command
            .args(config.args())
            .current_dir(&config.cwd)
            .env("DSH_HOME", &config.home)
            .env("DSH_TELEMETRY_DISABLED", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            // A client that panics must not orphan a Node process holding the
            // session lock.
            .kill_on_drop(true);

        for (key, value) in &config.env {
            command.env(key, value);
        }

        let child = command.spawn().map_err(|source| RuntimeError::Spawn {
            path: config.entry.to_string_lossy().into_owned(),
            source,
        })?;

        // Shared with the exit watcher: the struct keeps it for `terminate`, the watcher
        // polls the same handle. Two owners, one process.
        let child_slot = Arc::new(StdMutex::new(Some(child)));
        let (stdin, stdout, stderr) = {
            let mut guard = child_slot.lock().expect("child lock");
            let child = guard.as_mut().expect("child is present");
            (
                child.stdin.take().expect("stdin was piped"),
                child.stdout.take().expect("stdout was piped"),
                child.stderr.take().expect("stderr was piped"),
            )
        };

        let (outbound, mut outbound_rx) = mpsc::unbounded_channel::<String>();
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let (exited, _) = broadcast::channel(4);
        let stop_requested = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let pending: Arc<StdMutex<HashMap<u64, PendingCall>>> = Arc::default();
        let stderr_tail = Arc::new(StdMutex::new(std::collections::VecDeque::with_capacity(
            STDERR_HISTORY,
        )));

        // Writer: sole owner of stdin.
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(line) = outbound_rx.recv().await {
                if stdin.write_all(line.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() || stdin.flush().await.is_err() {
                    break;
                }
            }
            let _ = stdin.shutdown().await;
        });

        // Exit watcher.
        //
        // Nothing else waits on the child, so without this a **crash** is invisible: `exited`
        // would only ever fire from `shutdown`, meaning the pool keeps a dead route, the UI
        // banner never appears, and a session silently stops working with no error anywhere.
        //
        // It polls instead of `wait()`-ing on the mutex because the same mutex is what
        // `terminate` needs to kill the child — awaiting inside it would deadlock the kill.
        {
            let child = Arc::clone(&child_slot);
            let exited = exited.clone();
            let stop_requested = Arc::clone(&stop_requested);
            tokio::spawn(async move {
                loop {
                    {
                        let Ok(mut guard) = child.lock() else { return };
                        let Some(process) = guard.as_mut() else {
                            return;
                        };
                        match process.try_wait() {
                            Ok(Some(status)) => {
                                let _ = exited.send(ExitStatus {
                                    code: status.code(),
                                    requested: stop_requested
                                        .load(std::sync::atomic::Ordering::Relaxed),
                                });
                                return;
                            }
                            Ok(None) => {}
                            Err(_) => {
                                // Nothing more can be learned about this process.
                                let _ = exited.send(ExitStatus {
                                    code: None,
                                    requested: false,
                                });
                                return;
                            }
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(200)).await;
                }
            });
        }

        // Reader: sole owner of stdout.
        {
            let pending = Arc::clone(&pending);
            let events = events.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stdout).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.trim().is_empty() {
                        continue;
                    }
                    match protocol::parse_frame(&line) {
                        Ok(Incoming::Response { id, payload }) => {
                            let call = id.as_number().and_then(|number| {
                                pending.lock().expect("pending lock").remove(&number)
                            });
                            if let Some(call) = call {
                                let _ = call.sender.send(payload);
                            }
                        }
                        Ok(message) => {
                            // A missing receiver only means no UI is attached yet.
                            let _ = events.send(message);
                        }
                        Err(error) => {
                            tracing::warn!(target: "dsh_runtime", %error, "unparseable kernel frame");
                        }
                    }
                }
                // EOF: fail every waiter instead of hanging them.
                let drained: Vec<PendingCall> = {
                    let mut guard = pending.lock().expect("pending lock");
                    guard.drain().map(|(_, call)| call).collect()
                };
                for call in drained {
                    tracing::warn!(
                        target: "dsh_runtime",
                        method = %call.method,
                        "kernel closed stdout before answering"
                    );
                    let _ = call.sender.send(ResponsePayload::Err(RpcError {
                        code: 0,
                        message: "kernel closed its stdout before answering".to_owned(),
                        data: None,
                    }));
                }
            });
        }

        // stderr: diagnostics, plus the live stream the bridge republishes.
        {
            let stderr_tail = Arc::clone(&stderr_tail);
            let events = events.clone();
            tokio::spawn(async move {
                let mut lines = BufReader::new(stderr).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    // Tagged frames are protocol, not diagnostics: they join the
                    // same broadcast as stdout frames so the UI has one path.
                    if let Some(json) = line.strip_prefix(LIVE_PREFIX) {
                        match serde_json::from_str::<serde_json::Value>(json) {
                            Ok(params) => {
                                let _ = events.send(Incoming::Notification {
                                    method: LIVE_CHUNK_METHOD.to_owned(),
                                    params,
                                });
                            }
                            Err(error) => {
                                tracing::warn!(target: "dsh_runtime", %error, "unparseable live frame");
                            }
                        }
                        continue;
                    }
                    tracing::debug!(target: "dsh_kernel", "{line}");
                    let mut tail = stderr_tail.lock().expect("stderr lock");
                    if tail.len() == STDERR_HISTORY {
                        tail.pop_front();
                    }
                    tail.push_back(line);
                }
            });
        }

        let client = Self {
            outbound,
            pending,
            next_id: AtomicU64::new(1),
            events,
            stderr_tail,
            child: child_slot,
            exited,
            stop_requested,
        };

        Ok(client)
    }

    /// Subscribe to inbound notifications and kernel-initiated requests.
    ///
    /// A slow subscriber receives `Lagged(n)` rather than blocking the reader;
    /// re-read the session log to recover what it missed.
    pub fn subscribe(&self) -> broadcast::Receiver<Incoming> {
        self.events.subscribe()
    }

    /// Be told when the process ends.
    pub fn subscribe_exit(&self) -> broadcast::Receiver<ExitStatus> {
        self.exited.subscribe()
    }

    /// The last stderr lines, newest last.
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .expect("stderr lock")
            .iter()
            .cloned()
            .collect()
    }

    /// Perform the protocol handshake.
    pub async fn initialize(
        &self,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value, RuntimeError> {
        self.call(protocol::method::INITIALIZE, params, Some(timeout))
            .await
    }

    /// Send a request and wait for its reply.
    pub async fn call(
        &self,
        method: &str,
        params: serde_json::Value,
        timeout: Option<Duration>,
    ) -> Result<serde_json::Value, RuntimeError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (sender, receiver) = oneshot::channel();
        self.pending.lock().expect("pending lock").insert(
            id,
            PendingCall {
                sender,
                method: method.to_owned(),
            },
        );

        self.send_line(protocol::request_frame(id, method, &params))?;

        let payload = match timeout {
            Some(limit) => match tokio::time::timeout(limit, receiver).await {
                Ok(Ok(payload)) => payload,
                Ok(Err(_)) => {
                    self.pending.lock().expect("pending lock").remove(&id);
                    return Err(RuntimeError::NotRunning);
                }
                Err(_) => {
                    self.pending.lock().expect("pending lock").remove(&id);
                    return Err(RuntimeError::Timeout {
                        method: method.to_owned(),
                        timeout: limit,
                    });
                }
            },
            None => receiver.await.map_err(|_| RuntimeError::NotRunning)?,
        };

        match payload {
            ResponsePayload::Ok(value) => Ok(value),
            ResponsePayload::Err(error) => Err(RuntimeError::Rpc {
                method: method.to_owned(),
                code: error.code,
                message: error.message,
            }),
        }
    }

    /// Answer a kernel-initiated request. Forgetting this stalls the agent.
    pub fn answer(
        &self,
        id: &protocol::FrameId,
        payload: ResponsePayload,
    ) -> Result<(), RuntimeError> {
        self.send_line(protocol::response_frame(id, &payload))
    }

    /// Convenience wrapper for the approval flow.
    pub fn answer_approval(
        &self,
        id: &protocol::FrameId,
        outcome: protocol::ApprovalOutcome,
    ) -> Result<(), RuntimeError> {
        let value = serde_json::to_value(outcome).map_err(|error| {
            RuntimeError::Protocol(protocol::ProtocolError::Malformed(error.to_string()))
        })?;
        self.answer(
            id,
            ResponsePayload::Ok(serde_json::json!({ "outcome": value })),
        )
    }

    /// Ask the kernel to shut down, then stop waiting for politeness.
    pub async fn shutdown(&self, grace: Duration) {
        let _ = self
            .call(
                protocol::method::SHUTDOWN,
                serde_json::Value::Null,
                Some(grace),
            )
            .await;
        self.stop_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.exited.send(ExitStatus {
            code: None,
            requested: true,
        });
        self.terminate();
    }

    fn send_line(&self, line: String) -> Result<(), RuntimeError> {
        self.outbound
            .send(line)
            .map_err(|_| RuntimeError::NotRunning)
    }

    /// Kill the process if it is still alive. Safe to call twice.
    fn terminate(&self) {
        self.stop_requested
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let Ok(mut guard) = self.child.lock() else {
            return;
        };
        let Some(mut child) = guard.take() else {
            return;
        };
        // `kill_on_drop` also covers the panic path; this makes the normal path
        // deterministic instead of waiting for the drop.
        let _ = child.start_kill();
        tokio::spawn(async move {
            let _ = child.wait().await;
        });
    }
}

impl Drop for KernelClient {
    fn drop(&mut self) {
        self.terminate();
    }
}

/// Resolve the kernel entry inside a fetched install.
pub fn kernel_entry(kernel_dir: &Path) -> PathBuf {
    kernel_dir
        .join("node_modules")
        .join("@deepseek-ai")
        .join("dsh")
        .join("lib")
        .join("bin.js")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> KernelConfig {
        KernelConfig {
            node: PathBuf::from("node"),
            entry: PathBuf::from("/k/lib/bin.js"),
            profile: "sdk".to_owned(),
            patches: vec![PathBuf::from("/p/one.yml"), PathBuf::from("/p/two.yml")],
            home: PathBuf::from("/home"),
            cwd: PathBuf::from("/work"),
            env: Vec::new(),
        }
    }

    #[test]
    fn argv_places_patches_after_the_profile() {
        let args = config().args();
        assert_eq!(
            args,
            vec![
                "/k/lib/bin.js",
                "--profile",
                "sdk",
                "--patch",
                "/p/one.yml",
                "--patch",
                "/p/two.yml",
            ]
        );
    }

    #[test]
    fn kernel_entry_matches_the_fetched_layout() {
        assert_eq!(
            kernel_entry(Path::new("/repo/kernel")),
            PathBuf::from("/repo/kernel/node_modules/@deepseek-ai/dsh/lib/bin.js")
        );
    }
}
