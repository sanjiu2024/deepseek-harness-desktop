//! Line-level differences, for showing what an edit actually did.
//!
//! A model's `edit` tool reports "replaced 1 occurrence" and nothing else; a `write` reports "wrote
//! 412 bytes". Neither says *which lines moved*, which is exactly what a person watching the AI
//! work wants to see. The file is the truth, so the answer comes from comparing the file before and
//! after.
//!
//! Deliberately not a general-purpose diff: no move detection, no word-level granularity, no
//! patience algorithm. Two bounded line sequences, one cheap LCS, and a fallback for the case where
//! the sequences are too large to compare honestly.

/// Lines a hunk can carry before it stops being useful to look at.
///
/// A hunk that replaced a whole file is reported as counts with the first few lines, because
/// rendering four thousand removed lines helps nobody.
pub const MAX_HUNK_LINES: usize = 400;

/// Past this many lines on either side, the pair is not compared line by line.
///
/// The dynamic program below is O(n·m) in time and memory; at 4000 lines that is 16 M cells, which
/// is already more than a preview should cost. Beyond it the honest answer is "the whole region
/// changed", not a diff that takes a second to compute.
pub const MAX_DIFF_LINES: usize = 4000;

/// One contiguous change: `removed` lines replaced by `added`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hunk {
    /// 1-based line number in the old text where the change starts.
    pub old_start: usize,
    /// 1-based line number in the new text where the change starts.
    pub new_start: usize,
    /// Lines that are gone, in order. Bounded by [`MAX_HUNK_LINES`].
    pub removed: Vec<String>,
    /// Lines that appeared, in order. Bounded by [`MAX_HUNK_LINES`].
    pub added: Vec<String>,
    /// True when `removed`/`added` were cut short; the counts are still exact.
    pub truncated: bool,
}

/// A whole comparison.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LineDiff {
    pub hunks: Vec<Hunk>,
    /// Total lines removed — exact, even when a hunk's text was truncated.
    pub deleted: usize,
    /// Total lines added — exact.
    pub added: usize,
    /// True when the inputs were too large to compare line by line.
    pub coarse: bool,
}

impl LineDiff {
    pub fn is_empty(&self) -> bool {
        self.hunks.is_empty()
    }
}

/// Compare two texts line by line.
pub fn diff(before: &str, after: &str) -> LineDiff {
    let old: Vec<&str> = before.lines().collect();
    let new: Vec<&str> = after.lines().collect();

    // Common prefix and suffix are free: stripping them keeps the expensive part small, which is
    // what makes an edit in a long file cost the size of the edit rather than the size of the file.
    let mut prefix = 0;
    while prefix < old.len() && prefix < new.len() && old[prefix] == new[prefix] {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old.len() - prefix
        && suffix < new.len() - prefix
        && old[old.len() - 1 - suffix] == new[new.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let old_mid = &old[prefix..old.len() - suffix];
    let new_mid = &new[prefix..new.len() - suffix];

    if old_mid.is_empty() && new_mid.is_empty() {
        return LineDiff::default();
    }

    if old_mid.len() > MAX_DIFF_LINES || new_mid.len() > MAX_DIFF_LINES {
        return LineDiff {
            hunks: vec![bounded_hunk(prefix + 1, prefix + 1, old_mid, new_mid)],
            deleted: old_mid.len(),
            added: new_mid.len(),
            coarse: true,
        };
    }

    let mut hunks = Vec::new();
    let mut deleted = 0;
    let mut added = 0;
    for (old_index, new_index, removed, inserted) in lcs_segments(old_mid, new_mid) {
        deleted += removed.len();
        added += inserted.len();
        hunks.push(bounded_hunk(
            prefix + old_index + 1,
            prefix + new_index + 1,
            &removed,
            &inserted,
        ));
    }

    LineDiff {
        hunks,
        deleted,
        added,
        coarse: false,
    }
}

/// Cut a hunk's text down to the renderable maximum, keeping the counts intact.
fn bounded_hunk(old_start: usize, new_start: usize, removed: &[&str], added: &[&str]) -> Hunk {
    let truncated = removed.len() > MAX_HUNK_LINES || added.len() > MAX_HUNK_LINES;
    Hunk {
        old_start,
        new_start,
        removed: removed
            .iter()
            .take(MAX_HUNK_LINES)
            .map(|line| (*line).to_owned())
            .collect(),
        added: added
            .iter()
            .take(MAX_HUNK_LINES)
            .map(|line| (*line).to_owned())
            .collect(),
        truncated,
    }
}

/// The changed regions of two line slices, left to right.
///
/// One LCS table, walked once. Returns `(old_index, new_index, removed, added)` where the indices
/// are 0-based positions in the slices and mark where each region starts.
fn lcs_segments<'a>(
    old: &[&'a str],
    new: &[&'a str],
) -> Vec<(usize, usize, Vec<&'a str>, Vec<&'a str>)> {
    let rows = old.len() + 1;
    let cols = new.len() + 1;
    // Length of the LCS of the suffixes, so the walk goes forward and regions come out in order.
    let mut table = vec![0u32; rows * cols];
    for i in (0..old.len()).rev() {
        for j in (0..new.len()).rev() {
            table[i * cols + j] = if old[i] == new[j] {
                table[(i + 1) * cols + j + 1] + 1
            } else {
                table[(i + 1) * cols + j].max(table[i * cols + j + 1])
            };
        }
    }

    let mut segments = Vec::new();
    let (mut i, mut j) = (0usize, 0usize);
    let mut removed: Vec<&str> = Vec::new();
    let mut added: Vec<&str> = Vec::new();
    let mut start = (0usize, 0usize);

    fn flush<'a>(
        segments: &mut Vec<(usize, usize, Vec<&'a str>, Vec<&'a str>)>,
        start: (usize, usize),
        removed: &mut Vec<&'a str>,
        added: &mut Vec<&'a str>,
    ) {
        if removed.is_empty() && added.is_empty() {
            return;
        }
        segments.push((
            start.0,
            start.1,
            std::mem::take(removed),
            std::mem::take(added),
        ));
    }

    while i < old.len() && j < new.len() {
        if old[i] == new[j] {
            flush(&mut segments, start, &mut removed, &mut added);
            i += 1;
            j += 1;
            start = (i, j);
        } else if table[(i + 1) * cols + j] >= table[i * cols + j + 1] {
            if removed.is_empty() && added.is_empty() {
                start = (i, j);
            }
            removed.push(old[i]);
            i += 1;
        } else {
            if removed.is_empty() && added.is_empty() {
                start = (i, j);
            }
            added.push(new[j]);
            j += 1;
        }
    }
    if i < old.len() || j < new.len() {
        if removed.is_empty() && added.is_empty() {
            start = (i, j);
        }
        removed.extend_from_slice(&old[i..]);
        added.extend_from_slice(&new[j..]);
    }
    flush(&mut segments, start, &mut removed, &mut added);
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_text_has_no_hunks() {
        let diff = diff("a\nb\nc\n", "a\nb\nc\n");
        assert!(diff.is_empty());
        assert_eq!((diff.added, diff.deleted), (0, 0));
    }

    #[test]
    fn one_changed_line_is_one_hunk_in_place() {
        // The case that matters: a person wants to see *which* line moved, not that the file is
        // different.
        let diff = diff("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        assert_eq!(hunk.old_start, 2);
        assert_eq!(hunk.new_start, 2);
        assert_eq!(hunk.removed, vec!["b"]);
        assert_eq!(hunk.added, vec!["B"]);
        assert_eq!((diff.deleted, diff.added), (1, 1));
    }

    #[test]
    fn an_appended_block_is_reported_at_the_end() {
        let diff = diff("a\nb\n", "a\nb\nc\nd\n");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].removed.len(), 0);
        assert_eq!(diff.hunks[0].added, vec!["c", "d"]);
        assert_eq!(diff.hunks[0].new_start, 3);
    }

    #[test]
    fn a_deletion_keeps_the_surrounding_lines_out_of_the_hunk() {
        let diff = diff("a\nb\nc\nd\n", "a\nd\n");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.hunks[0].removed, vec!["b", "c"]);
        assert_eq!(diff.hunks[0].added.len(), 0);
        assert_eq!(diff.deleted, 2);
    }

    #[test]
    fn two_separate_edits_are_two_hunks() {
        // Prevents the "whole file changed" report that makes an IDE view useless.
        let diff = diff("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n");
        assert_eq!(diff.hunks.len(), 2, "{:?}", diff.hunks);
        assert_eq!(diff.hunks[0].old_start, 1);
        assert_eq!(diff.hunks[1].old_start, 5);
    }

    #[test]
    fn a_huge_change_falls_back_to_one_coarse_hunk() {
        let before: String = (0..MAX_DIFF_LINES + 10)
            .map(|index| format!("old {index}\n"))
            .collect();
        let after: String = (0..MAX_DIFF_LINES + 10)
            .map(|index| format!("new {index}\n"))
            .collect();

        let diff = diff(&before, &after);
        assert!(diff.coarse, "too large to compare honestly");
        assert_eq!(diff.hunks.len(), 1);
        assert_eq!(diff.deleted, MAX_DIFF_LINES + 10, "counts stay exact");
        assert!(diff.hunks[0].truncated, "the text is cut, the count is not");
    }

    #[test]
    fn a_hunk_never_carries_more_than_the_limit() {
        let before = String::new();
        let after: String = (0..MAX_HUNK_LINES + 50)
            .map(|index| format!("line {index}\n"))
            .collect();
        let diff = diff(&before, &after);

        assert_eq!(diff.added, MAX_HUNK_LINES + 50);
        assert_eq!(diff.hunks[0].added.len(), MAX_HUNK_LINES);
        assert!(diff.hunks[0].truncated);
    }

    #[test]
    fn a_file_that_only_gained_a_trailing_newline_is_not_a_content_change() {
        // `lines()` drops the terminator, so this must not produce a spurious hunk.
        let diff = diff("a\nb", "a\nb\n");
        assert!(diff.is_empty(), "{:?}", diff.hunks);
    }
}
