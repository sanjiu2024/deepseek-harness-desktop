//! Host-side facilities the kernel cannot provide: the file system the user sees, and a
//! shell on the user's machine.
//!
//! Both are deliberately narrow. The tree reader lists one directory at a time and refuses
//! to leave the workspace; the PTY is a real process, not a command runner — an app that
//! offers a terminal has to be honest that it is a terminal.

pub mod gate;
pub mod pty;
pub mod text;
pub mod tree;
pub mod watch;

pub use gate::{DEFAULT_PREVIEW_LIMIT, Gate};
pub use pty::{Pty, PtyEvent, PtySize};
pub use text::{Decoded, decode};
pub use tree::{Entry, EntryKind, Order, TreeError};
