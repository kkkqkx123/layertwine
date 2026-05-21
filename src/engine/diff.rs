//! Diff Engine - Row-level diff calculation based on similar crate
//!
//! Convert the output of similar::TextDiff to a Delta representation within Stratum.

use std::path::PathBuf;
use similar::{ChangeTag, TextDiff};
use crate::core::delta::{Delta, LineDiff};
use crate::core::file_node::FileNode;
use crate::core::types::{DiffOp, Hunk, SourceType};

fn strip_newline(s: &str) -> String {
    s.trim_end_matches('\n').trim_end_matches('\r').to_string()
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
                        let lines: Vec<String> = diff.iter_changes(op)
                            .map(|c| strip_newline(c.value()))
                            .collect();
                        my_ops.push(DiffOp::Insert {
                            new_start: n_range.start as u32 + 1,
                            lines,
                        });
                    }
                    similar::DiffTag::Replace => {
                        let old_cnt = (o_range.end - o_range.start) as u32;
                        let lines: Vec<String> = diff.iter_changes(op)
                            .filter(|c| c.tag() == ChangeTag::Insert)
                            .map(|c| strip_newline(c.value()))
                            .collect();
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
pub fn collect_changes_from_diff<'a>(
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
                        let old_start = old_pos.saturating_sub(1).max(0) as u32;
                        let new_start = new_pos.saturating_sub(1).max(0) as u32;
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
        let old_start = old_pos.saturating_sub(1).max(0) as u32;
        let new_start = new_pos.saturating_sub(1).max(0) as u32;
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
                DiffOp::Replace { old_count, lines, .. } => {
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
    let diff = TextDiff::from_lines(old, new);
    diff.unified_diff()
        .context_radius(context)
        .to_string()
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
    fn test_format_unified_diff() {
        let old = "a\nb\nc\n";
        let new = "a\nd\nc\n";
        let output = format_unified_diff(old, new, 1);
        assert!(output.contains("-b"));
        assert!(output.contains("+d"));
    }
}
