//! The one place that knows how to write a file without leaving half of one.
//!
//! Every file this client owns — settings, the rename overlay, `provider.env` — is read back on
//! the next launch and parsed as a whole. A plain `fs::write` truncates first and writes after,
//! so a crash (or a power cut, or a scanner holding the target) leaves valid-looking JSON that
//! is cut in half, and the next start either loses the setting or refuses to parse it.
//!
//! The fix is the classic one: write a sibling temp file, flush it, then `rename` over the
//! target. `rename` replaces atomically on every platform we ship, so a reader either sees the
//! old file or the new one, never a mixture.

use std::io::Write;
use std::path::Path;

/// Write `body` to `path`, replacing it atomically.
///
/// The temp file is a sibling, not a system temp file: `rename` is only atomic within one
/// filesystem, and a temp directory on another drive would make this a copy.
pub fn write_atomic(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    // Same directory, unique name: two writers racing must not share a temp file either.
    let unique = format!(
        "{}.{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "file".to_owned()),
        std::process::id()
    );
    let temp = parent.join(unique);

    {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(body)?;
        // Durability before the rename: a rename that lands before the bytes would leave an
        // empty file where a complete one used to be.
        file.sync_all()?;
    }

    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            // Leave nothing behind on failure; a stray `.tmp` next to settings.json is the kind
            // of litter that gets mistaken for a real file later.
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("dsh-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn writes_the_content_and_leaves_no_temp_file() {
        let root = scratch("basic");
        let path = root.join("settings.json");
        write_atomic(&path, br#"{"a":1}"#).expect("write");

        let body = std::fs::read_to_string(&path).expect("read");
        let leftovers = std::fs::read_dir(&root)
            .expect("list")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(body, r#"{"a":1}"#);
        assert_eq!(
            leftovers, 0,
            "a stray .tmp is litter that gets mistaken for a file"
        );
    }

    #[test]
    fn replaces_an_existing_file_whole() {
        let root = scratch("replace");
        let path = root.join("settings.json");
        write_atomic(&path, b"old contents that are longer").expect("first");
        write_atomic(&path, b"new").expect("second");

        let body = std::fs::read_to_string(&path).expect("read");
        let _ = std::fs::remove_dir_all(&root);
        assert_eq!(body, "new", "no tail of the previous file survives");
    }

    #[test]
    fn creates_missing_parent_directories() {
        let root = scratch("nested");
        let path = root.join("logs").join("nested").join("file.txt");
        write_atomic(&path, b"x").expect("write");
        let exists = path.exists();
        let _ = std::fs::remove_dir_all(&root);
        assert!(exists);
    }
}
