//! The kernel's attachment store, and the objects nothing will ever read again.
//!
//! Images sent in a prompt are admitted into `$DSH_HOME/attachments/v1/objects/<2 hex>/<sha256>`
//! and referenced from the session log by `sha256:<64 hex>`. Nothing in the kernel deletes them:
//! the store is content-addressed, so the same image is never written twice, but an image whose
//! session was deleted stays on disk forever. That is a slow leak the user cannot see, and the
//! only place to fix it is here — the client owns the disk.
//!
//! Two rules make the cleanup safe rather than merely bold:
//!
//! 1. **Only unreferenced objects** — every session log is scanned for `sha256:` mentions before
//!    anything is removed. A log that failed to parse still counts as a reference: unreadable is
//!    not the same as unused.
//! 2. **Only objects older than a grace period** — an image is admitted *before* the turn writes
//!    it to the log, so a just-written object legitimately has no reference yet. Deleting it would
//!    break the prompt in flight.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::{SESSIONS_DIR, StoreError};

/// How long an unreferenced object is left alone.
///
/// Long enough to cover a slow turn (the model can think for minutes) and a crashed run whose log
/// is written on the next start. Short enough that garbage does not accumulate for a month.
pub const GRACE: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// What the attachment store holds, and what of it is garbage.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentsReport {
    /// Objects on disk.
    pub objects: usize,
    pub bytes: u64,
    /// Objects some session log mentions.
    pub referenced: usize,
    /// Objects no log mentions.
    pub unreferenced: usize,
    pub unreferenced_bytes: u64,
    /// Unreferenced objects young enough to be a prompt in flight.
    pub kept_recent: usize,
    /// Removed by this run (0 for a dry run).
    pub removed: usize,
    pub removed_bytes: u64,
}

/// Where the kernel keeps objects.
fn objects_root(home: &Path) -> PathBuf {
    home.join("attachments").join("v1").join("objects")
}

/// Every `sha256:<hex>` digest mentioned anywhere under `sessions/`.
///
/// Deliberately a byte scan rather than a parse: a log written by a newer kernel must still count
/// as a reference, and a log that is half-flushed must not make its images look collectable.
fn referenced_hashes(home: &Path) -> Result<HashSet<String>, StoreError> {
    let mut seen = HashSet::new();
    let sessions = home.join(SESSIONS_DIR);
    let Ok(projects) = std::fs::read_dir(&sessions) else {
        return Ok(seen);
    };
    for project in projects.flatten() {
        let Ok(logs) = std::fs::read_dir(project.path()) else {
            continue;
        };
        for session in logs.flatten() {
            let log = session.path().join("session.v4.jsonl");
            let Ok(bytes) = std::fs::read(&log) else {
                continue;
            };
            // Lossy: a truncated UTF-8 tail must not hide the digests before it.
            let text = String::from_utf8_lossy(&bytes);
            for (index, _) in text.match_indices("sha256:") {
                let rest = &text[index + 7..];
                let digest: String = rest
                    .chars()
                    .take_while(char::is_ascii_hexdigit)
                    .take(64)
                    .collect();
                if digest.len() == 64 {
                    seen.insert(digest);
                }
            }
        }
    }
    Ok(seen)
}

/// Collect the objects no session references any more.
///
/// `dry_run` reports without removing: the default, because a cleanup that runs as a side effect
/// of opening a panel is a bug waiting to happen.
pub fn collect(home: &Path, dry_run: bool) -> Result<AttachmentsReport, StoreError> {
    let referenced = referenced_hashes(home)?;
    let root = objects_root(home);
    let mut report = AttachmentsReport::default();

    let Ok(buckets) = std::fs::read_dir(&root) else {
        return Ok(report);
    };
    let now = std::time::SystemTime::now();
    for bucket in buckets.flatten() {
        let Ok(objects) = std::fs::read_dir(bucket.path()) else {
            continue;
        };
        for object in objects.flatten() {
            let Ok(metadata) = object.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let name = object.file_name().to_string_lossy().into_owned();
            report.objects += 1;
            report.bytes += metadata.len();

            if referenced.contains(&name) {
                report.referenced += 1;
                continue;
            }
            report.unreferenced += 1;
            report.unreferenced_bytes += metadata.len();

            let age = metadata
                .modified()
                .ok()
                .and_then(|time| now.duration_since(time).ok())
                .unwrap_or(GRACE);
            if age < GRACE {
                report.kept_recent += 1;
                continue;
            }
            if !dry_run {
                match std::fs::remove_file(object.path()) {
                    Ok(()) => {
                        report.removed += 1;
                        report.removed_bytes += metadata.len();
                    }
                    // An object that vanished between listing and removal is already gone — the
                    // report should not claim a failure for the outcome it wanted.
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => {
                        tracing::warn!(target: "dsh_store", %error, path = %object.path().display(), "cannot remove attachment");
                    }
                }
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("dsh-att-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(SESSIONS_DIR).join("--p--").join("s"))
            .expect("scratch sessions");
        root
    }

    fn object(home: &Path, digest: &str, body: &[u8]) -> PathBuf {
        let dir = home
            .join("attachments")
            .join("v1")
            .join("objects")
            .join(&digest[..2]);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join(digest);
        std::fs::write(&path, body).expect("write object");
        path
    }

    fn session(home: &Path, body: &str) {
        std::fs::write(
            home.join(SESSIONS_DIR)
                .join("--p--")
                .join("s")
                .join("session.v4.jsonl"),
            body,
        )
        .expect("write log");
    }

    fn age(path: &Path, seconds: u64) {
        // Back-date by writing the mtime through the std API only — no external tooling.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(seconds);
        let file = std::fs::File::options()
            .write(true)
            .open(path)
            .expect("open");
        file.set_modified(old).expect("set mtime");
    }

    const HASH_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const HASH_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    #[test]
    fn a_referenced_object_is_never_removed() {
        let home = scratch("kept");
        let kept = object(&home, HASH_A, b"keep me");
        age(&kept, 30 * 24 * 3600);
        session(&home, &format!(r#"{{"attachmentId":"sha256:{HASH_A}"}}"#));

        let report = collect(&home, false).expect("collect");

        assert!(kept.exists(), "a log mentions it");
        assert_eq!(report.referenced, 1);
        assert_eq!(report.removed, 0);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_orphan_is_removed_once_it_is_old_enough() {
        let home = scratch("orphan");
        let orphan = object(&home, HASH_B, b"nobody needs this");
        age(&orphan, 24 * 3600);
        session(&home, r#"{"type":"user/message"}"#);

        let report = collect(&home, false).expect("collect");

        assert!(!orphan.exists());
        assert_eq!(report.removed, 1);
        assert_eq!(report.removed_bytes, 17);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_fresh_orphan_survives_because_a_turn_may_be_in_flight() {
        // The kernel admits bytes before the log records them, so "unreferenced" is not the same
        // as "unused" within the grace period.
        let home = scratch("fresh");
        let fresh = object(&home, HASH_B, b"just admitted");

        let report = collect(&home, false).expect("collect");

        assert!(fresh.exists());
        assert_eq!(report.kept_recent, 1);
        assert_eq!(report.removed, 0);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_dry_run_reports_without_deleting() {
        let home = scratch("dry");
        let orphan = object(&home, HASH_B, b"report me");
        age(&orphan, 24 * 3600);

        let report = collect(&home, true).expect("collect");

        assert!(orphan.exists(), "a dry run must not delete");
        assert_eq!(report.removed, 0);
        assert_eq!(report.unreferenced, 1);
        assert_eq!(report.unreferenced_bytes, 9);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn an_unreadable_log_still_counts_as_a_reference() {
        // Half-written or newer-format logs must not make their images look collectable.
        let home = scratch("broken");
        let kept = object(&home, HASH_A, b"keep me");
        age(&kept, 30 * 24 * 3600);
        session(
            &home,
            &format!("{{\"x\":\"sha256:{HASH_A}\"}}\nnot json at all"),
        );

        let report = collect(&home, false).expect("collect");

        assert!(kept.exists());
        assert_eq!(report.referenced, 1);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_missing_store_is_an_empty_report_not_an_error() {
        let home = std::env::temp_dir().join(format!("dsh-att-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        let report = collect(&home, false).expect("collect");
        assert_eq!(report.objects, 0);
    }
}
