//! Diff Engine - Row-level diff calculation based on similar crate
//!
//! Convert the output of similar::TextDiff to a Delta representation within Layertwine.

use crate::core::delta::Delta;
use crate::core::file_node::FileNode;
use crate::core::types::{DiffOp, Hunk, LineDiff, SourceType};
use similar::{ChangeTag, TextDiff};
use std::path::PathBuf;

// Performance optimizations
use lazy_static::lazy_static;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

lazy_static! {
    static ref DIFF_CACHE: Mutex<HashMap<u64, String>> = Mutex::new(HashMap::new());
}

fn compute_hash(old: &str, new: &str, context: usize) -> u64 {
    let mut hasher = DefaultHasher::new();
    old.hash(&mut hasher);
    new.hash(&mut hasher);
    context.hash(&mut hasher);
    hasher.finish()
}

fn strip_newline(s: &str) -> String {
    if s.ends_with('\n') || s.ends_with('\r') {
        s.trim_end_matches(['\n', '\r']).to_string()
    } else {
        s.to_string()
    }
}

/// Calculates the line level difference between two texts, returns LineDiff
///
/// Generate a line-level diff using similar::TextDiff::from_lines.
/// Grouped as Hunk list by grouped_ops(3), context rows = 3.
pub fn diff_to_line_diff(old: &str, new: &str) -> LineDiff {
    let diff = TextDiff::from_lines(old, new);
    let grouped = diff.grouped_ops(3);

    let hunks: Vec<Hunk> = grouped
        .iter()
        .map(|ops| {
            let first = ops.first().expect("group should have at least one op");
            let last = ops.last().expect("group should have at least one op");

            let old_range_first = first.old_range();
            let old_range_last = last.old_range();
            let new_range_first = first.new_range();
            let new_range_last = last.new_range();

            let hunk_old_start = old_range_first.start;
            let hunk_old_end = old_range_last.end;
            let hunk_new_start = new_range_first.start;
            let hunk_new_end = new_range_last.end;

            let mut my_ops = Vec::new();
            for op in ops {
                let o_range = op.old_range();
                let n_range = op.new_range();
                match op.tag() {
                    similar::DiffTag::Equal => {
                        my_ops.push(DiffOp::Equal {
                            count: (o_range.end - o_range.start) as u32,
                        });
                    }
                    similar::DiffTag::Delete => {
                        let cnt = (o_range.end - o_range.start) as u32;
                        my_ops.push(DiffOp::Delete {
                            old_start: o_range.start as u32 + 1,
                            count: cnt,
                        });
                    }
                    similar::DiffTag::Insert => {
                        let changes: Vec<_> = diff.iter_changes(op).collect();
                        let mut lines = Vec::with_capacity(changes.len());
                        for c in changes {
                            lines.push(strip_newline(c.value()));
                        }
                        my_ops.push(DiffOp::Insert {
                            new_start: n_range.start as u32 + 1,
                            lines,
                        });
                    }
                    similar::DiffTag::Replace => {
                        let old_cnt = (o_range.end - o_range.start) as u32;
                        let changes: Vec<_> = diff.iter_changes(op).collect();
                        let mut lines = Vec::with_capacity(changes.len());
                        for c in changes {
                            if c.tag() == ChangeTag::Insert {
                                lines.push(strip_newline(c.value()));
                            }
                        }
                        my_ops.push(DiffOp::Replace {
                            old_start: o_range.start as u32 + 1,
                            old_count: old_cnt,
                            new_start: n_range.start as u32 + 1,
                            lines,
                        });
                    }
                }
            }

            Hunk {
                old_start: hunk_old_start as u32 + 1,
                old_len: (hunk_old_end - hunk_old_start) as u32,
                new_start: hunk_new_start as u32 + 1,
                new_len: (hunk_new_end - hunk_new_start) as u32,
                ops: my_ops,
            }
        })
        .collect();

    LineDiff { hunks }
}

/// Gather all changes from diff and build the complete Delta
///
/// Use iter_all_changes() to iterate over all row changes and construct a Delta containing the full content mapping.
#[allow(dead_code)]
fn collect_changes_from_diff<'a>(
    diff: &'a TextDiff<'a, 'a, str>,
    path: PathBuf,
    old_content: &[u8],
    source_type: SourceType,
) -> Delta {
    // Building LineDiff from iter_all_changes
    let mut equal_ops: Vec<DiffOp> = Vec::new();
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current_ops = Vec::new();
    let mut old_pos: usize = 0;
    let mut new_pos: usize = 0;
    let mut in_change = false;

    for change in diff.iter_all_changes() {
        match change.tag() {
            ChangeTag::Equal => {
                if in_change {
                    // End the previous change block
                    if !current_ops.is_empty() {
                        let old_start = old_pos.saturating_sub(1) as u32;
                        let new_start = new_pos.saturating_sub(1) as u32;
                        hunks.push(Hunk {
                            old_start,
                            old_len: 0, // Will be amended below
                            new_start,
                            new_len: 0,
                            ops: std::mem::take(&mut current_ops),
                        });
                    }
                    in_change = false;
                }
                equal_ops.push(DiffOp::Equal { count: 1 });
                old_pos += 1;
                new_pos += 1;
            }
            ChangeTag::Delete => {
                in_change = true;
                current_ops.push(DiffOp::Delete {
                    old_start: old_pos as u32 + 1,
                    count: 1,
                });
                old_pos += 1;
            }
            ChangeTag::Insert => {
                in_change = true;
                current_ops.push(DiffOp::Insert {
                    new_start: new_pos as u32 + 1,
                    lines: vec![change.value().to_string()],
                });
                new_pos += 1;
            }
        }
    }

    // Last paragraph change
    if !current_ops.is_empty() {
        let old_start = old_pos.saturating_sub(1) as u32;
        let new_start = new_pos.saturating_sub(1) as u32;
        hunks.push(Hunk {
            old_start,
            old_len: 0,
            new_start,
            new_len: 0,
            ops: std::mem::take(&mut current_ops),
        });
    }

    // Fix the len field in Hunk
    for hunk in &mut hunks {
        let mut old_len = 0u32;
        let mut new_len = 0u32;
        for op in &hunk.ops {
            match op {
                DiffOp::Equal { count } => {
                    old_len += count;
                    new_len += count;
                }
                DiffOp::Delete { count, .. } => {
                    old_len += count;
                }
                DiffOp::Insert { lines, .. } => {
                    new_len += lines.len() as u32;
                }
                DiffOp::Replace {
                    old_count, lines, ..
                } => {
                    old_len += old_count;
                    new_len += lines.len() as u32;
                }
            }
        }
        hunk.old_len = old_len;
        hunk.new_len = new_len;
    }

    let line_diff = LineDiff { hunks };

    let file_node = FileNode::new(path, old_content);
    Delta::new(file_node, line_diff, source_type)
}

/// Unified diff output (with context preserved) for displaying the
pub fn format_unified_diff(old: &str, new: &str, context: usize) -> String {
    // Try cache for small files
    let total_size = old.len() + new.len();
    if total_size < 100000 {
        let hash = compute_hash(old, new, context);
        if let Some(cached) = DIFF_CACHE.lock().unwrap().get(&hash) {
            return cached.clone();
        }
    }

    let diff = TextDiff::from_lines(old, new);

    // Use streaming for large files to reduce memory pressure
    let result = if old.len() > 50000 || new.len() > 50000 {
        // Stream processing for large files
        let mut output = String::new();
        for hunk in diff.unified_diff().context_radius(context).iter_hunks() {
            output.push_str(&hunk.to_string());
        }
        output
    } else {
        // Direct generation for small files
        diff.unified_diff().context_radius(context).to_string()
    };

    // Cache small file results
    let total_size = old.len() + new.len();
    if total_size < 100000 {
        let hash = compute_hash(old, new, context);
        DIFF_CACHE.lock().unwrap().insert(hash, result.clone());
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diff_to_line_diff_simple() {
        let old = "hello\nworld\nfoo\n";
        let new = "hello\nrust\nfoo\n";
        let line_diff = diff_to_line_diff(old, new);
        assert_eq!(line_diff.hunks.len(), 1, "should have 1 hunk");

        let hunk = &line_diff.hunks[0];
        assert_eq!(hunk.ops.len(), 3);
        assert!(matches!(hunk.ops[0], DiffOp::Equal { count: 1 }));
    }

    #[test]
    fn test_diff_to_line_diff_no_change() {
        let text = "line1\nline2\nline3\n";
        let line_diff = diff_to_line_diff(text, text);
        assert_eq!(line_diff.hunks.len(), 0, "no changes = no hunks");
    }

    #[test]
    fn test_diff_to_line_diff_insert() {
        let old = "a\nc\n";
        let new = "a\nb\nc\n";
        let line_diff = diff_to_line_diff(old, new);
        assert!(!line_diff.hunks.is_empty());
    }

    #[test]
    fn test_diff_to_line_diff_delete_only() {
        let old = "a\nb\nc\n";
        let new = "a\nc\n";
        let line_diff = diff_to_line_diff(old, new);
        assert!(!line_diff.hunks.is_empty());

        let has_delete = line_diff
            .hunks
            .iter()
            .any(|h| h.ops.iter().any(|op| matches!(op, DiffOp::Delete { .. })));
        assert!(has_delete);
    }

    #[test]
    fn test_diff_to_line_diff_replace() {
        let old = "a\nb\nc\n";
        let new = "a\nx\nc\n";
        let line_diff = diff_to_line_diff(old, new);
        assert!(!line_diff.hunks.is_empty());

        let has_replace = line_diff
            .hunks
            .iter()
            .any(|h| h.ops.iter().any(|op| matches!(op, DiffOp::Replace { .. })));
        assert!(has_replace);
    }

    #[test]
    fn test_diff_to_line_diff_empty_old() {
        let old = "";
        let new = "a\nb\nc\n";
        let line_diff = diff_to_line_diff(old, new);
        assert!(!line_diff.hunks.is_empty());
        assert_eq!(line_diff.hunks[0].old_start, 1);
        assert_eq!(line_diff.hunks[0].new_start, 1);
    }

    #[test]
    fn test_diff_to_line_diff_empty_new() {
        let old = "a\nb\nc\n";
        let new = "";
        let line_diff = diff_to_line_diff(old, new);
        assert!(!line_diff.hunks.is_empty());
        let has_delete = line_diff
            .hunks
            .iter()
            .any(|h| h.ops.iter().any(|op| matches!(op, DiffOp::Delete { .. })));
        assert!(has_delete);
    }

    #[test]
    fn test_diff_to_line_diff_multiple_hunks() {
        let old = "a\nb\nc\nd\ne\nf\ng\nh\ni\nj\nk\nl\nm\nn\n";
        let new = "a\nX\nc\nd\ne\nf\ng\nh\ni\nY\nk\nl\nm\nn\n";
        let line_diff = diff_to_line_diff(old, new);
        assert_eq!(
            line_diff.hunks.len(),
            2,
            "two separated changes should produce 2 hunks"
        );
    }

    #[test]
    fn test_collect_changes() {
        let old = "keep\nremove\nkeep\n";
        let new = "keep\nadded\nkeep\n";
        let diff = TextDiff::from_lines(old, new);
        let delta = collect_changes_from_diff(
            &diff,
            PathBuf::from("test.txt"),
            old.as_bytes(),
            SourceType::Manual,
        );
        assert!(delta.id.to_hex().len() == 64, "delta should have valid id");
    }

    #[test]
    fn test_collect_changes_empty_diff() {
        let old = "same\n";
        let new = "same\n";
        let diff = TextDiff::from_lines(old, new);
        let delta = collect_changes_from_diff(
            &diff,
            PathBuf::from("test.txt"),
            old.as_bytes(),
            SourceType::Manual,
        );
        assert!(delta.diff.hunks.is_empty() || delta.diff.hunks.iter().all(|h| h.ops.is_empty()));
    }

    #[test]
    fn test_collect_changes_only_insert() {
        let old = "";
        let new = "a\nb\nc\n";
        let diff = TextDiff::from_lines(old, new);
        let delta = collect_changes_from_diff(
            &diff,
            PathBuf::from("test.txt"),
            old.as_bytes(),
            SourceType::Manual,
        );
        assert!(!delta.diff.hunks.is_empty());
        let has_insert = delta
            .diff
            .hunks
            .iter()
            .any(|h| h.ops.iter().any(|op| matches!(op, DiffOp::Insert { .. })));
        assert!(has_insert);
    }

    #[test]
    fn test_format_unified_diff() {
        let old = "a\nb\nc\n";
        let new = "a\nd\nc\n";
        let output = format_unified_diff(old, new, 1);
        assert!(output.contains("-b"));
        assert!(output.contains("+d"));
    }

    #[test]
    fn test_format_unified_diff_no_changes() {
        let text = "a\nb\nc\n";
        let output = format_unified_diff(text, text, 1);
        assert!(!output.contains('-'));
        assert!(!output.contains('+'));
    }

    #[test]
    fn test_diff_to_line_diff_single_char_no_newline() {
        let diff = diff_to_line_diff("x", "y");
        assert!(
            !diff.hunks.is_empty() || diff.hunks.iter().any(|h| !h.ops.is_empty()),
            "single char change should produce diff"
        );
    }

    #[test]
    fn test_diff_to_line_diff_both_empty() {
        let diff = diff_to_line_diff("", "");
        assert!(diff.is_empty(), "both empty should produce no diff");
    }

    #[test]
    fn test_diff_to_line_diff_only_newlines() {
        let diff = diff_to_line_diff("\n\n", "\n\n\n");
        assert!(
            !diff.is_empty(),
            "different newline count should produce diff"
        );
    }
}
