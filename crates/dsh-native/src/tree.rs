//! One directory at a time.
//!
//! A file tree that reads the whole repository up front is a file tree that freezes on a
//! `node_modules`. This lists a single directory per call and lets the UI decide what to
//! expand — which also means a directory the user never opens costs nothing.

use std::path::{Component, Path, PathBuf};

/// What a row is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EntryKind {
    File,
    Directory,
    Symlink,
}

/// One row of the tree.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub name: String,
    /// Full path, so the UI never has to join paths and get it wrong on Windows.
    pub path: String,
    pub kind: EntryKind,
    /// Bytes for a file; `None` for a directory.
    pub size: Option<u64>,
    /// Milliseconds since the epoch.
    pub modified: Option<i64>,
}

/// One directory, capped.
///
/// A flat directory with a hundred thousand files is not hypothetical — a build output folder is
/// exactly that — and returning every entry turns one call into megabytes of JSON that no tree
/// view can render anyway. The cap is reported rather than applied silently: a truncated listing
/// that looks complete is worse than an error.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    pub entries: Vec<Entry>,
    /// How many entries the directory actually had, before the cap.
    pub total: usize,
    pub truncated: bool,
}

/// More than this and nobody is scrolling, they are searching.
pub const MAX_ENTRIES: usize = 2000;

#[derive(Debug, thiserror::Error)]
pub enum TreeError {
    #[error("path escapes the workspace: {0}")]
    OutsideWorkspace(String),
    #[error("not a directory: {0}")]
    NotADirectory(String),
    #[error("file is too large to preview ({0} bytes)")]
    TooLarge(u64),
    #[error("not a text file: {0}")]
    NotATextFile(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Directories worth hiding by default. Not a policy — a courtesy: nobody browses
/// `node_modules` in a 280px column, and listing it makes the tree unusable.
pub const NOISY: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    ".dsh-home",
    ".dev-home",
    ".kernel-staging",
];

/// List one directory, sorted directories first then names, with noisy entries hidden.
///
/// `root` is the workspace. Any request outside it is refused rather than clamped: silently
/// showing a different directory than the one asked for is worse than an error.
pub fn list(root: &Path, path: &Path, show_noisy: bool) -> Result<Listing, TreeError> {
    let root = normalize(root)?;
    let target = if path.as_os_str().is_empty() {
        root.clone()
    } else {
        normalize(path)?
    };
    if !target.starts_with(&root) {
        return Err(TreeError::OutsideWorkspace(target.display().to_string()));
    }
    if !target.is_dir() {
        return Err(TreeError::NotADirectory(target.display().to_string()));
    }

    let mut entries = Vec::new();
    let mut total = 0usize;
    for item in std::fs::read_dir(&target)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if !show_noisy && NOISY.contains(&name.as_str()) {
            continue;
        }
        total += 1;
        // Count everything, build only what we will return: `total` is what makes "showing
        // 2000 of 41823" possible instead of a listing that quietly lies.
        if entries.len() >= MAX_ENTRIES {
            continue;
        }
        let metadata = match item.metadata() {
            Ok(metadata) => metadata,
            // A broken symlink or a file that vanished between readdir and stat is not a
            // reason to fail the whole listing.
            Err(_) => continue,
        };
        let file_type = item.file_type()?;
        let kind = if file_type.is_symlink() {
            EntryKind::Symlink
        } else if file_type.is_dir() {
            EntryKind::Directory
        } else {
            EntryKind::File
        };
        entries.push(Entry {
            name,
            path: item.path().to_string_lossy().into_owned(),
            kind,
            size: (kind == EntryKind::File).then_some(metadata.len()),
            modified: metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|delta| delta.as_millis() as i64),
        });
    }

    let truncated = total > entries.len();
    entries.sort_by(|left, right| {
        let left_dir = left.kind == EntryKind::Directory;
        let right_dir = right.kind == EntryKind::Directory;
        right_dir
            .cmp(&left_dir)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(Listing {
        entries,
        total,
        truncated,
    })
}

/// Read a text file for preview, with the encoding it was actually in.
///
/// The cap is not about memory: a preview that streams 40 MB into a WebView locks the window,
/// and the user wanted to glance at the top of a file.
pub fn read_preview(
    root: &Path,
    path: &Path,
    max_bytes: u64,
) -> Result<crate::text::Decoded, TreeError> {
    let root = normalize(root)?;
    let target = normalize(path)?;
    if !target.starts_with(&root) {
        return Err(TreeError::OutsideWorkspace(target.display().to_string()));
    }
    let metadata = std::fs::metadata(&target)?;
    if metadata.len() > max_bytes {
        return Err(TreeError::TooLarge(metadata.len()));
    }
    let bytes = std::fs::read(&target)?;
    // Decoding is in `text`: GBK is text, UTF-16 is text, and calling either "binary" (or
    // rendering it as replacement characters) is how a preview lies to the reader.
    crate::text::decode(&bytes).ok_or_else(|| TreeError::NotATextFile(target.display().to_string()))
}

/// Resolve a path to its absolute, canonical form.
fn normalize(path: &Path) -> Result<PathBuf, TreeError> {
    // `canonicalize` also collapses `..`, which is what makes the containment check sound:
    // comparing unnormalised paths would let `a/../../etc` pass.
    let canonical = std::fs::canonicalize(path).map_err(TreeError::Io)?;
    debug_assert!(
        !canonical
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    );
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dsh-tree-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("src")).expect("scratch");
        std::fs::write(path.join("README.md"), "hello").expect("write");
        std::fs::write(path.join("src/main.ts"), "console.log(1)").expect("write");
        std::fs::create_dir_all(path.join("node_modules")).expect("scratch");
        path
    }

    #[test]
    fn lists_directories_first_and_hides_noise() {
        let root = scratch("list");
        let listing = list(&root, &root, false).expect("list");
        let _ = std::fs::remove_dir_all(&root);

        let names: Vec<&str> = listing
            .entries
            .iter()
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(names, vec!["src", "README.md"]);
        assert!(!names.contains(&"node_modules"));
    }

    #[test]
    fn noise_can_be_shown_on_request() {
        let root = scratch("noisy");
        let listing = list(&root, &root, true).expect("list");
        let _ = std::fs::remove_dir_all(&root);
        assert!(
            listing
                .entries
                .iter()
                .any(|entry| entry.name == "node_modules")
        );
    }

    #[test]
    fn a_path_outside_the_workspace_is_refused() {
        // Silently showing a different directory than the one asked for is worse than failing.
        let root = scratch("escape");
        let outside = std::env::temp_dir();
        let result = list(&root, &outside, false);
        let _ = std::fs::remove_dir_all(&root);
        assert!(matches!(result, Err(TreeError::OutsideWorkspace(_))));
    }

    #[test]
    fn a_preview_reads_text_and_refuses_binaries() {
        let root = scratch("preview");
        let preview = read_preview(&root, &root.join("README.md"), 1024).expect("text");
        std::fs::write(root.join("blob.bin"), [0u8, 1, 2, 3]).expect("binary");
        let binary = read_preview(&root, &root.join("blob.bin"), 1024);
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(preview.text, "hello");
        assert_eq!(preview.encoding, "utf-8");
        assert!(binary.is_err(), "a NUL byte means binary, not text");
    }

    #[test]
    fn a_huge_file_is_refused_rather_than_truncated() {
        // Truncation would look like a file that ends mid-sentence.
        let root = scratch("huge");
        std::fs::write(root.join("big.txt"), "x".repeat(4096)).expect("write");
        let result = read_preview(&root, &root.join("big.txt"), 1024);
        let _ = std::fs::remove_dir_all(&root);
        assert!(matches!(result, Err(TreeError::TooLarge(_))));
    }
}

#[cfg(test)]
mod cap_tests {
    use super::*;

    fn flat_dir(name: &str, count: usize) -> PathBuf {
        let path = std::env::temp_dir().join(format!("dsh-cap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        for index in 0..count {
            std::fs::write(path.join(format!("f{index:05}.txt")), b"x").expect("write");
        }
        path
    }

    #[test]
    fn a_huge_directory_is_capped_and_says_so() {
        // A build output folder is exactly this shape. Returning every entry turns one call
        // into megabytes that no tree view can render.
        let root = flat_dir("big", MAX_ENTRIES + 25);
        let listing = list(&root, &root, false).expect("list");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(listing.entries.len(), MAX_ENTRIES);
        assert!(listing.truncated);
        assert_eq!(
            listing.total,
            MAX_ENTRIES + 25,
            "the real count is still reported"
        );
    }

    #[test]
    fn a_small_directory_is_not_truncated() {
        let root = flat_dir("small", 3);
        let listing = list(&root, &root, false).expect("list");
        let _ = std::fs::remove_dir_all(&root);

        assert_eq!(listing.entries.len(), 3);
        assert!(!listing.truncated);
        assert_eq!(listing.total, 3);
    }
}
