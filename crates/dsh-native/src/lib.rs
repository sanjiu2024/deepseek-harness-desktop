//! Host-side facilities the kernel cannot provide: the file system the user sees, and a
//! shell on the user's machine.
//!
//! Both are deliberately narrow. The tree reader lists one directory at a time and refuses
//! to leave the workspace; the PTY is a real process, not a command runner — an app that
//! offers a terminal has to be honest that it is a terminal.

pub mod pty;
pub mod text;
pub mod tree;

pub use pty::{Pty, PtyEvent, PtySize};
pub use text::{decode, Decoded};
pub use tree::{Entry, EntryKind, TreeError};
