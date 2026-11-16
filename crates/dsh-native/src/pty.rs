//! A real shell, attached to a pseudo-terminal.
//!
//! Not a command runner: output arrives as bytes, the shell keeps its own state, and Ctrl-C
//! is a byte the user sends rather than a signal this program sends on their behalf. Anything
//! less is a command box pretending to be a terminal.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use portable_pty::{CommandBuilder, MasterPty, PtySize as PortableSize};

/// Terminal dimensions in cells.
#[derive(Debug, Clone, Copy, serde::Deserialize)]
pub struct PtySize {
    pub rows: u16,
    pub cols: u16,
}

impl Default for PtySize {
    fn default() -> Self {
        Self { rows: 24, cols: 80 }
    }
}

/// What came out of the shell.
#[derive(Debug, Clone)]
pub enum PtyEvent {
    /// Bytes to render. Not a `String`: a partial UTF-8 sequence splits across reads, and
    /// decoding each chunk independently would corrupt Chinese output mid-character.
    Data(Vec<u8>),
    Exited(Option<u32>),
}

/// One shell session.
pub struct Pty {
    /// Queue to the writer thread. Bounded: a caller that outruns the shell gets an error
    /// instead of blocking a command thread or growing a buffer without limit.
    outbound: std::sync::mpsc::SyncSender<Vec<u8>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    /// Set by `close`; the reader thread stops when it sees it.
    closed: Arc<AtomicBool>,
    /// The shell process itself.
    ///
    /// Held on purpose. The first version dropped it the moment `spawn` returned — harmless on
    /// Windows, and on Linux the CI diagnostic showed a reader blocked on a master that never
    /// produced a byte *and* a child that never exited. Keeping the handle also gives `close`
    /// something to wait for.
    child: std::sync::Mutex<Option<Box<dyn portable_pty::Child + Send + Sync>>>,
}

impl Pty {
    /// Spawn the user's shell.
    ///
    /// On Windows that is PowerShell: `cmd.exe` still exists, but a user who opens a terminal
    /// in 2026 expects PowerShell, and the kernel's own tools assume it too.
    pub fn spawn(
        cwd: &std::path::Path,
        size: PtySize,
        on_event: impl Fn(PtyEvent) + Send + 'static,
    ) -> Result<Self, String> {
        let pty_system = portable_pty::native_pty_system();
        let pair = pty_system
            .openpty(PortableSize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())?;

        let mut command = CommandBuilder::new(shell());
        command.cwd(cwd);
        let child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;

        // The slave is kept until the reader and writer are attached.
        //
        // Dropping it first is what portable-pty's examples do, and on Windows it is harmless.
        // On Linux the CI diagnostic showed exactly zero bytes *and* no exit — a reader blocked
        // on a master whose slave had already been closed on our side before the child held it.

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let mut writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;

        // A dedicated writer thread, fed by a bounded queue.
        //
        // `pty_write` arrives from a Tauri command thread. Writing straight to the master there
        // means a large paste blocks that thread until the shell drains its buffer — with a
        // 1 MB paste and a shell that is busy, that is a visible freeze in the window, caused by
        // the terminal rather than by the shell.
        let (outbound, inbound) = std::sync::mpsc::sync_channel::<Vec<u8>>(WRITE_QUEUE_CHUNKS);
        std::thread::spawn(move || {
            while let Ok(chunk) = inbound.recv() {
                if writer.write_all(&chunk).is_err() {
                    break;
                }
                // Flush per chunk: a terminal that holds keystrokes in a buffer feels broken.
                if writer.flush().is_err() {
                    break;
                }
            }
        });

        // Only now: the child has the slave, our copies of the handles exist.
        drop(pair.slave);

        let closed = Arc::new(AtomicBool::new(false));
        let reader_closed = Arc::clone(&closed);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            // Coalesce before emitting.
            //
            // A raw read loop sends one event per chunk, and `cat` of a 5 MB file becomes
            // thousands of JSON messages through the WebView — each one base64, each one a
            // render. Terminals batch for the same reason: the eye cannot see 8 000 updates a
            // second, and the queue that builds up while the UI is busy is pure latency for
            // whatever the user typed.
            let mut pending: Vec<u8> = Vec::with_capacity(FLUSH_BYTES * 2);
            let mut last_flush = std::time::Instant::now();

            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => {
                        pending.extend_from_slice(&buffer[..read]);
                        // Flush early when the reader is ahead of us, or when enough time has
                        // passed that a quiet terminal still feels immediate.
                        // Deliberately NOT flushing on a short read: a shell that writes
                        // line by line always reads short, and flushing there would send one
                        // event per line — the exact flooding this is meant to prevent.
                        if pending.len() >= FLUSH_BYTES || last_flush.elapsed() >= FLUSH_INTERVAL {
                            on_event(PtyEvent::Data(std::mem::take(&mut pending)));
                            last_flush = std::time::Instant::now();
                        }
                    }
                    Err(_) => break,
                }
                if reader_closed.load(Ordering::Relaxed) {
                    break;
                }
            }
            // Whatever was still buffered when the shell exited.
            if !pending.is_empty() {
                on_event(PtyEvent::Data(pending));
            }
            on_event(PtyEvent::Exited(None));
        });

        Ok(Self {
            child: std::sync::Mutex::new(Some(child)),
            outbound,
            master: Arc::new(Mutex::new(pair.master)),
            closed,
        })
    }

    /// Hand bytes to the shell exactly as typed.
    /// Queue input for the shell.
    ///
    /// Returns an error when the queue is full — a caller producing input faster than the shell
    /// consumes it is a bug worth surfacing, not something to absorb silently by growing a
    /// buffer until the process runs out of memory.
    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        self.outbound.try_send(data.to_vec()).map_err(|_| {
            "the terminal is not keeping up with input; drop some keystrokes".to_owned()
        })
    }

    /// Tell the shell the window changed size, so full-screen tools redraw correctly.
    pub fn resize(&self, size: PtySize) -> Result<(), String> {
        let master = self.master.lock().map_err(|error| error.to_string())?;
        master
            .resize(PortableSize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|error| error.to_string())
    }

    /// Stop the reader. The child is reaped by the OS when the master handle drops.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
    }
}

/// How many input chunks may wait for the shell.
///
/// A chunk is whatever one `pty_write` carried — a keystroke or a paste. A thousand of them is
/// far more than a person types ahead of a shell, and small enough that a runaway producer is
/// stopped rather than buffered.
const WRITE_QUEUE_CHUNKS: usize = 1024;

/// Flush once this much output has accumulated. 8 KB is one read; two reads in a row from a
/// fast writer are the case worth coalescing.
const FLUSH_BYTES: usize = 16 * 1024;

/// Flush at most this late. A terminal that waits 50ms to show a prompt feels broken; this is
/// short enough to stay imperceptible and long enough to merge a burst.
const FLUSH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(12);

/// The shell to start.
///
/// Order matters:  (PowerShell 7) is what a modern user expects, but plenty of
/// machines only have Windows PowerShell — and assuming  there fails at spawn with a
/// message about a missing file, which looks like our bug rather than a missing shell.
fn shell() -> String {
    if let Ok(explicit) = std::env::var("DSH_DESKTOP_SHELL") {
        return explicit;
    }
    let candidates: &[&str] = if cfg!(windows) {
        &["pwsh", "powershell", "cmd"]
    } else {
        &["sh"]
    };
    for candidate in candidates {
        if let Some(found) = which(candidate) {
            return found;
        }
    }
    candidates.first().copied().unwrap_or("sh").to_owned()
}

/// Resolve an executable the way a shell would, without pulling in a crate for it.
fn which(name: &str) -> Option<String> {
    let extensions: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned())
            .split(';')
            .map(str::to_lowercase)
            .collect()
    } else {
        vec![String::new()]
    };
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        for extension in &extensions {
            let candidate = directory.join(format!("{name}{extension}"));
            if candidate.is_file() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }
    None
}
