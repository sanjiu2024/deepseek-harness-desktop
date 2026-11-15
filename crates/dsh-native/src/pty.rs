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
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    /// Set by `close`; the reader thread stops when it sees it.
    closed: Arc<AtomicBool>,
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
        let _child = pair
            .slave
            .spawn_command(command)
            .map_err(|error| error.to_string())?;
        drop(pair.slave);

        let mut reader = pair
            .master
            .try_clone_reader()
            .map_err(|error| error.to_string())?;
        let writer = pair
            .master
            .take_writer()
            .map_err(|error| error.to_string())?;

        let closed = Arc::new(AtomicBool::new(false));
        let reader_closed = Arc::clone(&closed);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(read) => on_event(PtyEvent::Data(buffer[..read].to_vec())),
                    Err(_) => break,
                }
                if reader_closed.load(Ordering::Relaxed) {
                    break;
                }
            }
            on_event(PtyEvent::Exited(None));
        });

        Ok(Self {
            writer: Arc::new(Mutex::new(writer)),
            master: Arc::new(Mutex::new(pair.master)),
            closed,
        })
    }

    /// Hand bytes to the shell exactly as typed.
    pub fn write(&self, data: &[u8]) -> Result<(), String> {
        let mut writer = self.writer.lock().map_err(|error| error.to_string())?;
        writer.write_all(data).map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())
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

