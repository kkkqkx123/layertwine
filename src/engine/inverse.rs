//! Reverse Delta Engine
//!
//! Generates an inverse Delta that can undo a Delta operation.
//! Refer to Immer's inversePatches design.

use crate::core::delta::{Delta, LineDiff};
use crate::core::types::{DiffOp, Hunk};
use crate::error::{Result, StratumError};

/// Inverse operation to generate Delta
///
/// For Insert → Delete, Delete → Insert, Replace → content swap.
/// Since the `Delete` operation does not save the contents of the deleted line, `old_content` is required.
/// This is the old text at the time the Delta was generated, which is used to extract the deleted lines.
///
/// If `old_content` is None, the reverse Insert of Delete will contain the empty string.
pub fn inverse_delta(delta: &Delta, old_content: Option<&str>) -> Result<Delta> {
    let old_lines: Vec<&str> = old_content
        .map(|c| c.lines().collect())
        .unwrap_or_default();

    let mut inv_hunks = Vec::new();

    for hunk in &delta.diff.hunks {
        let mut inv_ops = Vec::new();

        for op in &hunk.ops {
            match op {
                DiffOp::Equal { count } => {
                    inv_ops.push(DiffOp::Equal { count: *count });
                }
                DiffOp::Insert {
                    new_start,
                    lines,
                } => {
                    // The inverse of Insert is Delete (which removes the lines from the new text).
                    inv_ops.push(DiffOp::Delete {
                        old_start: *new_start,
                        count: lines.len() as u32,
                    });
                }
                DiffOp::Delete {
                    old_start,
                    count,
                } => {
                    // The inverse of Delete is Insert (you need to know what was deleted)
                    let deleted_lines: Vec<String> = if !old_lines.is_empty() {
                        let start = (*old_start as usize).saturating_sub(1);
                        let end = (start + *count as usize).min(old_lines.len());
                        old_lines[start..end]
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        // No old text message, generate placeholder
                        (0..*count).map(|_| String::new()).collect()
                    };
                    inv_ops.push(DiffOp::Insert {
                        new_start: *old_start,
                        lines: deleted_lines,
                    });
                }
                DiffOp::Replace {
                    old_start,
                    old_count,
                    new_start,
                    lines,
                } => {
                    // The inverse of Replace is reverse Replace
                    // Need to know what was originally replaced
                    let original_lines: Vec<String> = if !old_lines.is_empty() {
                        let start = (*old_start as usize).saturating_sub(1);
                        let end = (start + *old_count as usize).min(old_lines.len());
                        old_lines[start..end]
                            .iter()
                            .map(|s| s.to_string())
                            .collect()
                    } else {
                        lines.iter().map(|_| String::new()).collect()
                    };
                    inv_ops.push(DiffOp::Replace {
                        old_start: *new_start,
                        old_count: lines.len() as u32,
                        new_start: *old_start,
                        lines: original_lines,
                    });
                }
            }
        }

        inv_hunks.push(Hunk {
            old_start: hunk.new_start,
            old_len: hunk.new_len,
            new_start: hunk.old_start,
            new_len: hunk.old_len,
            ops: inv_ops,
        });
    }

    let inv_diff = LineDiff::new(inv_hunks);
    Ok(Delta::new(
        delta.file.clone(),
        inv_diff,
        delta.source.clone(),
    ))
}

/// Generate a reverse Delta list for Snapshot's incremental chains
///
/// You need to provide the old content that corresponds to each phase when building the snapshot.
/// Generate a reverse Delta from the newest to the oldest, so that the sequential application returns to the initial state.
pub fn inverse_snapshot(
    _snapshot: &crate::core::snapshot::Snapshot,
    _contents: &[&str],
) -> Result<Vec<Delta>> {
    // This function needs to read from the storage layer to build the reverse chain
    // Currently returns an empty list (placeholder), actual use requires integration with the storage layer
    Err(StratumError::Engine(
        "inverse_snapshot Not yet fully implemented: requires the storage layer to provide the contents of each version".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::LineDiff;
    use crate::core::file_node::FileNode;
    use crate::core::types::{DiffOp, Hunk, SourceType};
    use std::path::PathBuf;

    #[test]
    fn test_inverse_insert_becomes_delete() {
        // Insert "new_line" → Delete "new_line"
        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 2,
            ops: vec![
                DiffOp::Equal { count: 1 },
                DiffOp::Insert {
                    new_start: 2,
                    lines: vec!["new_line".to_string()],
                },
            ],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"content"),
            diff,
            SourceType::Manual,
        );
        let inv = inverse_delta(&delta, None).unwrap();

        // The inverse delta should have a Delete operation
        let has_delete = inv.diff.hunks.iter().any(|h| {
            h.ops
                .iter()
                .any(|op| matches!(op, DiffOp::Delete { .. }))
        });
        assert!(has_delete);
    }

    #[test]
    fn test_inverse_delete_requires_content() {
        // Delete needs old_content to know what was deleted.
        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 0,
            ops: vec![DiffOp::Delete {
                old_start: 1,
                count: 1,
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"content"),
            diff,
            SourceType::Manual,
        );

        // Without old_content, a blank line is generated.
        let inv = inverse_delta(&delta, None).unwrap();
        let has_insert = inv.diff.hunks.iter().any(|h| {
            h.ops
                .iter()
                .any(|op| matches!(op, DiffOp::Insert { .. }))
        });
        assert!(has_insert);

        // With old_content, it should be possible to extract the deleted rows
        let inv2 = inverse_delta(&delta, Some("deleted_line\n")).unwrap();
        let insert_lines: Vec<&str> = inv2
            .diff
            .hunks
            .iter()
            .flat_map(|h| &h.ops)
            .filter_map(|op| {
                if let DiffOp::Insert { lines, .. } = op {
                    Some(lines.iter().map(|s| s.as_str()).collect::<Vec<_>>())
                } else {
                    None
                }
            })
            .flatten()
            .collect();
        assert_eq!(insert_lines, vec!["deleted_line"]);
    }

    #[test]
    fn test_inverse_delta_roundtrip() {
        // Insert → Delete returns to the original after application.
        let content = "line1\nline3\n";
        let file = FileNode::new(PathBuf::from("test.txt"), b"");

        // Create insert delta: insert line2 after line1
        let hunk = Hunk {
            old_start: 2,
            old_len: 0,
            new_start: 2,
            new_len: 1,
            ops: vec![DiffOp::Insert {
                new_start: 2,
                lines: vec!["line2".to_string()],
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(file.clone(), diff, SourceType::Manual);

        // Apply insert → get line1\nline2\nline3
        let new_content = crate::engine::merge::apply_deltas(content, &[delta.clone()]).unwrap();
        assert_eq!(new_content, "line1\nline2\nline3");

        // Generate an inverse delta and apply it to the new content → go back to original
        let inv = inverse_delta(&delta, Some(content)).unwrap();
        let restored = crate::engine::merge::apply_deltas(&new_content, &[inv]).unwrap();
        assert_eq!(restored, content.trim_end_matches('\n'));
    }
}
