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
/// Takes a Snapshot and its corresponding Deltas (ordered oldest → newest) plus the old content
/// for each Delta when it was applied. Returns inverse Deltas in reverse order (newest → oldest),
/// so that applying them sequentially undoes all changes back to the initial state.
///
/// Each `old_contents[i]` must be the file content BEFORE `deltas[i]` was applied.
pub fn inverse_snapshot(
    snapshot: &crate::core::snapshot::Snapshot,
    deltas: &[Delta],
    old_contents: &[&str],
) -> Result<Vec<Delta>> {
    if deltas.len() != snapshot.deltas.len() {
        return Err(StratumError::Engine(format!(
            "deltas length ({}) does not match snapshot delta chain length ({})",
            deltas.len(),
            snapshot.deltas.len()
        )));
    }
    if old_contents.len() != deltas.len() {
        return Err(StratumError::Engine(format!(
            "old_contents length ({}) does not match deltas length ({})",
            old_contents.len(),
            deltas.len()
        )));
    }

    deltas
        .iter()
        .zip(old_contents.iter())
        .rev()
        .map(|(delta, content)| inverse_delta(delta, Some(content)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::delta::LineDiff;
    use crate::core::file_node::FileNode;
    use crate::core::types::{DiffOp, Hunk, SourceType};
    use crate::engine::diff::diff_to_line_diff;
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

    #[test]
    fn test_inverse_replace() {
        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 1,
            ops: vec![DiffOp::Replace {
                old_start: 1,
                old_count: 1,
                new_start: 1,
                lines: vec!["new".to_string()],
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"content"),
            diff,
            SourceType::Manual,
        );
        let inv = inverse_delta(&delta, Some("old\n")).unwrap();
        let has_replace = inv.diff.hunks.iter().any(|h| {
            h.ops.iter().any(|op| matches!(op, DiffOp::Replace { .. }))
        });
        assert!(has_replace, "inverse of Replace should be Replace");

        // Check that the old_start/new_start are swapped
        for hunk in &inv.diff.hunks {
            for op in &hunk.ops {
                if let DiffOp::Replace { old_start, new_start, lines, .. } = op {
                    assert_eq!(*old_start, 1, "old_start should be original new_start");
                    assert_eq!(*new_start, 1, "new_start should be original old_start");
                    assert_eq!(lines, &vec!["old".to_string()], "should contain original content");
                }
            }
        }
    }

    #[test]
    fn test_inverse_replace_roundtrip() {
        let content = "old\n";
        let file = FileNode::new(PathBuf::from("test.txt"), b"");

        let hunk = Hunk {
            old_start: 1,
            old_len: 1,
            new_start: 1,
            new_len: 1,
            ops: vec![DiffOp::Replace {
                old_start: 1,
                old_count: 1,
                new_start: 1,
                lines: vec!["new".to_string()],
            }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(file, diff, SourceType::Manual);

        let new_content = crate::engine::merge::apply_deltas(content, &[delta.clone()]).unwrap();
        assert_eq!(new_content, "new");

        let inv = inverse_delta(&delta, Some(content)).unwrap();
        let restored = crate::engine::merge::apply_deltas(&new_content, &[inv]).unwrap();
        assert_eq!(restored, "old");
    }

    #[test]
    fn test_inverse_preserves_equal() {
        let hunk = Hunk {
            old_start: 1,
            old_len: 2,
            new_start: 1,
            new_len: 2,
            ops: vec![DiffOp::Equal { count: 2 }],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"content"),
            diff,
            SourceType::Manual,
        );
        let inv = inverse_delta(&delta, None).unwrap();
        let has_equal = inv.diff.hunks.iter().any(|h| {
            h.ops.iter().any(|op| matches!(op, DiffOp::Equal { .. }))
        });
        assert!(has_equal, "inverse should preserve Equal ops");
    }

    #[test]
    fn test_inverse_mixed_ops() {
        let hunk = Hunk {
            old_start: 1,
            old_len: 3,
            new_start: 1,
            new_len: 4,
            ops: vec![
                DiffOp::Equal { count: 1 },
                DiffOp::Delete { old_start: 2, count: 1 },
                DiffOp::Insert { new_start: 3, lines: vec!["inserted".to_string()] },
            ],
        };
        let diff = LineDiff::new(vec![hunk]);
        let delta = Delta::new(
            FileNode::new(PathBuf::from("test.txt"), b"keep\ndeleted\n"),
            diff,
            SourceType::Manual,
        );
        let inv = inverse_delta(&delta, Some("keep\ndeleted\n")).unwrap();

        // Inverse: Delete becomes Insert, Insert becomes Delete, Equal stays
        let has_delete = inv.diff.hunks.iter().any(|h| {
            h.ops.iter().any(|op| matches!(op, DiffOp::Delete { .. }))
        });
        let has_insert = inv.diff.hunks.iter().any(|h| {
            h.ops.iter().any(|op| matches!(op, DiffOp::Insert { .. }))
        });
        let has_equal = inv.diff.hunks.iter().any(|h| {
            h.ops.iter().any(|op| matches!(op, DiffOp::Equal { .. }))
        });

        assert!(has_delete, "original Insert should become Delete");
        assert!(has_insert, "original Delete should become Insert");
        assert!(has_equal, "Equal should stay Equal");
    }

    #[test]
    fn test_inverse_snapshot_empty_chain() {
        let file_node = FileNode::new(PathBuf::from("test.txt"), b"content");
        let empty_diff = LineDiff::new(vec![]);
        let delta = Delta::new(file_node, empty_diff, SourceType::Manual);
        let snapshot = crate::core::snapshot::Snapshot::new_initial(
            delta.file.clone(),
            delta.id,
        );
        let result = inverse_snapshot(&snapshot, &[delta], &[""]);
        assert!(result.is_ok(), "inverse_snapshot should succeed");
        let inverses = result.unwrap();
        assert_eq!(inverses.len(), 1, "a single empty delta produces one trivial inverse");
        assert!(inverses[0].diff.is_empty(), "inverse of empty delta should also be empty");
    }

    #[test]
    fn test_inverse_snapshot_mismatched_length() {
        let file_node = FileNode::new(PathBuf::from("test.txt"), b"content");
        let empty_diff = LineDiff::new(vec![]);
        let delta = Delta::new(file_node, empty_diff, SourceType::Manual);
        let snapshot = crate::core::snapshot::Snapshot::new_initial(
            delta.file.clone(),
            delta.id,
        );
        let result = inverse_snapshot(&snapshot, &[], &[]);
        assert!(result.is_err(), "mismatched lengths should error");
    }

    #[test]
    fn test_inverse_snapshot_chain() {
        let content0 = "a\nb\nc\n";
        let content1 = "a\nX\nc\n";
        let file = FileNode::new(PathBuf::from("test.txt"), b"");

        let diff1 = diff_to_line_diff(content0, content1);
        let delta1 = Delta::new(file.clone(), diff1, SourceType::Manual);

        let snapshot = crate::core::snapshot::Snapshot::new_initial(
            file.clone(),
            delta1.id,
        );

        // snapshot has one delta, inverse should undo it
        let result = inverse_snapshot(&snapshot, &[delta1], &[content0]).unwrap();
        assert_eq!(result.len(), 1, "should produce one inverse delta");
    }

    #[test]
    fn test_inverse_delta_empty() {
        let diff = LineDiff::new(vec![]);
        let file_node = FileNode::new(PathBuf::from("empty.txt"), b"content");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        let old_content = "content";
        let inverse = inverse_delta(&delta, Some(old_content)).unwrap();
        assert!(inverse.diff.is_empty(), "empty delta inverse should also be empty");
    }

    #[test]
    fn test_inverse_insert_with_content() {
        let diff = diff_to_line_diff("", "a\nb\nc\n");
        let file_node = FileNode::new(PathBuf::from("new.txt"), b"a\nb\nc\n");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        let inverse = inverse_delta(&delta, Some("")).unwrap();
        // Inverse of insert should be delete
        let has_delete = inverse.diff.hunks.iter().any(|h| h.ops.iter().any(|op| matches!(op, DiffOp::Delete { .. })));
        assert!(has_delete, "inverse of insert should contain Delete ops");
    }

    #[test]
    fn test_inverse_delete_with_content() {
        let diff = diff_to_line_diff("a\nb\nc\n", "a\nc\n");
        let file_node = FileNode::new(PathBuf::from("del.txt"), b"a\nc\n");
        let delta = Delta::new(file_node, diff, SourceType::Manual);
        let inverse = inverse_delta(&delta, Some("a\nb\nc\n")).unwrap();
        // Inverse of delete should be insert
        let has_insert = inverse.diff.hunks.iter().any(|h| h.ops.iter().any(|op| matches!(op, DiffOp::Insert { .. })));
        assert!(has_insert, "inverse of delete should contain Insert ops");
    }
}
