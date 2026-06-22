//! Integration tests for the engine module.
//!
//! These tests exercise the core diff/merge/inverse functionality end-to-end.
//! They verify:
//! - Text diff calculation using similar crate
//! - Delta application to reconstruct content
//! - Three-way text merge with conflict detection
//! - Inverse delta generation for rollback
//! - Round-trip operations (apply + inverse should return to original)

use std::path::PathBuf;

use layertwine::core::delta::Delta;
use layertwine::core::file_node::FileNode;
use layertwine::core::snapshot::Snapshot;
use layertwine::core::types::{LineDiff, SourceType};
use layertwine::engine::diff::{diff_to_line_diff, format_unified_diff};
use layertwine::engine::inverse::{inverse_delta, inverse_snapshot};
use layertwine::engine::merge::{apply_deltas, merge_texts, MergeConflict};

// ---------------------------------------------------------------------------
// Test: Diff calculation with various scenarios
// ---------------------------------------------------------------------------

#[test]
fn test_diff_simple_replacement() {
    let old = "hello\nworld\n";
    let new = "hello\nrust\n";
    let line_diff = diff_to_line_diff(old, new);

    assert_eq!(line_diff.hunks.len(), 1, "should have 1 hunk");
    let hunk = &line_diff.hunks[0];

    let has_replace = hunk
        .ops
        .iter()
        .any(|op| matches!(op, layertwine::core::types::DiffOp::Replace { .. }));
    assert!(has_replace, "should contain Replace operation");
}

#[test]
fn test_diff_insert_operation() {
    let old = "a\n";
    let new = "a\nb\n";
    let line_diff = diff_to_line_diff(old, new);

    assert!(!line_diff.hunks.is_empty(), "should have hunks");

    let has_insert = line_diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Insert { .. }))
    });
    assert!(has_insert, "should contain Insert operation");
}

#[test]
fn test_diff_delete_operation() {
    let old = "a\nb\n";
    let new = "a\n";
    let line_diff = diff_to_line_diff(old, new);

    assert!(!line_diff.hunks.is_empty(), "should have hunks");

    let has_delete = line_diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Delete { .. }))
    });
    assert!(has_delete, "should contain Delete operation");
}

#[test]
fn test_diff_no_change() {
    let text = "line1\nline2\nline3\n";
    let line_diff = diff_to_line_diff(text, text);

    assert_eq!(line_diff.hunks.len(), 0, "no changes = no hunks");
}

#[test]
fn test_diff_multiple_hunks() {
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
fn test_diff_empty_to_content() {
    let old = "";
    let new = "a\nb\nc\n";
    let line_diff = diff_to_line_diff(old, new);

    assert!(!line_diff.hunks.is_empty(), "should have hunks");
    assert_eq!(line_diff.hunks[0].old_start, 1);
}

#[test]
fn test_diff_content_to_empty() {
    let old = "a\nb\nc\n";
    let new = "";
    let line_diff = diff_to_line_diff(old, new);

    assert!(!line_diff.hunks.is_empty(), "should have hunks");

    let has_delete = line_diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Delete { .. }))
    });
    assert!(has_delete, "should contain Delete operations");
}

#[test]
fn test_format_unified_diff() {
    let old = "a\nb\nc\n";
    let new = "a\nd\nc\n";
    let output = format_unified_diff(old, new, 1);

    assert!(output.contains("-b"), "should show deleted line");
    assert!(output.contains("+d"), "should show inserted line");
}

// ---------------------------------------------------------------------------
// Test: Delta application (apply_deltas)
// ---------------------------------------------------------------------------

#[test]
fn test_apply_deltas_empty() {
    let content = "hello\nworld\n";
    let result = apply_deltas(content, &[]).unwrap();

    assert_eq!(result, "hello\nworld");
}

#[test]
fn test_apply_deltas_single_insert() {
    let content = "line1\nline3\n";

    let hunk = layertwine::core::types::Hunk {
        old_start: 2,
        old_len: 1,
        new_start: 2,
        new_len: 2,
        ops: vec![
            layertwine::core::types::DiffOp::Insert {
                new_start: 2,
                lines: vec!["line2".to_string()],
            },
            layertwine::core::types::DiffOp::Equal { count: 1 },
        ],
    };

    let diff = LineDiff::new(vec![hunk]);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let result = apply_deltas(content, &[delta]).unwrap();
    assert_eq!(result, "line1\nline2\nline3");
}

#[test]
fn test_apply_deltas_single_delete() {
    let content = "line1\nline2\nline3\n";

    let hunk = layertwine::core::types::Hunk {
        old_start: 2,
        old_len: 1,
        new_start: 2,
        new_len: 0,
        ops: vec![layertwine::core::types::DiffOp::Delete {
            old_start: 2,
            count: 1,
        }],
    };

    let diff = LineDiff::new(vec![hunk]);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let result = apply_deltas(content, &[delta]).unwrap();
    assert_eq!(result, "line1\nline3");
}

#[test]
fn test_apply_deltas_single_replace() {
    let content = "aaa\nbbb\nccc\n";

    let hunk = layertwine::core::types::Hunk {
        old_start: 2,
        old_len: 1,
        new_start: 2,
        new_len: 1,
        ops: vec![layertwine::core::types::DiffOp::Replace {
            old_start: 2,
            old_count: 1,
            new_start: 2,
            lines: vec!["xxx".to_string()],
        }],
    };

    let diff = LineDiff::new(vec![hunk]);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let result = apply_deltas(content, &[delta]).unwrap();
    assert_eq!(result, "aaa\nxxx\nccc");
}

#[test]
fn test_apply_deltas_chain() {
    let content = "a\nb\nc\n";

    // First delta: insert 'x' after 'a'
    let hunk1 = layertwine::core::types::Hunk {
        old_start: 2,
        old_len: 1,
        new_start: 2,
        new_len: 2,
        ops: vec![
            layertwine::core::types::DiffOp::Insert {
                new_start: 2,
                lines: vec!["x".to_string()],
            },
            layertwine::core::types::DiffOp::Equal { count: 1 },
        ],
    };

    // Second delta: replace line 3 (now 'b') with 'y'
    let hunk2 = layertwine::core::types::Hunk {
        old_start: 3,
        old_len: 1,
        new_start: 3,
        new_len: 1,
        ops: vec![layertwine::core::types::DiffOp::Replace {
            old_start: 3,
            old_count: 1,
            new_start: 3,
            lines: vec!["y".to_string()],
        }],
    };

    let diff1 = LineDiff::new(vec![hunk1]);
    let diff2 = LineDiff::new(vec![hunk2]);

    let delta1 = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff1,
        SourceType::Manual,
    );
    let delta2 = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff2,
        SourceType::Manual,
    );

    let result = apply_deltas(content, &[delta1, delta2]).unwrap();
    assert_eq!(result, "a\nx\ny\nc");
}

#[test]
fn test_apply_deltas_empty_content() {
    let content = "";
    let result = apply_deltas(content, &[]).unwrap();
    assert_eq!(result, "");
}

#[test]
fn test_apply_deltas_overlapping_hunks_error() {
    let content = "a\nb\nc\n";

    let hunk1 = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        ops: vec![layertwine::core::types::DiffOp::Equal { count: 1 }],
    };

    let hunk2 = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        ops: vec![layertwine::core::types::DiffOp::Equal { count: 1 }],
    };

    let diff = LineDiff::new(vec![hunk1, hunk2]);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let result = apply_deltas(content, &[delta]);
    assert!(result.is_err(), "overlapping hunks should error");
}

#[test]
fn test_apply_deltas_out_of_range_error() {
    let content = "a\nb\n";

    let hunk = layertwine::core::types::Hunk {
        old_start: 10,
        old_len: 1,
        new_start: 10,
        new_len: 1,
        ops: vec![layertwine::core::types::DiffOp::Equal { count: 1 }],
    };

    let diff = LineDiff::new(vec![hunk]);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let result = apply_deltas(content, &[delta]);
    assert!(result.is_err(), "out-of-range hunk should error");
}

// ---------------------------------------------------------------------------
// Test: Three-way merge (merge_texts)
// ---------------------------------------------------------------------------

#[test]
fn test_merge_identical_texts() {
    let base = "a\nb\nc\n";
    let (merged, conflicts) = merge_texts(base, base, base);

    assert!(conflicts.is_empty());
    assert_eq!(merged, "a\nb\nc");
}

#[test]
fn test_merge_non_conflicting_changes() {
    let base = "a\nb\nc\n";
    let ours = "x\nb\nc\n";
    let theirs = "a\nb\ny\n";

    let (merged, conflicts) = merge_texts(base, ours, theirs);

    assert!(conflicts.is_empty());
    assert!(merged.contains('x'));
    assert!(merged.contains('y'));
}

#[test]
fn test_merge_with_conflict() {
    let base = "a\nb\nc\n";
    let ours = "a\nX\nc\n";
    let theirs = "a\nY\nc\n";

    let (_, conflicts) = merge_texts(base, ours, theirs);

    assert!(!conflicts.is_empty());
    assert_eq!(conflicts[0].ours, vec!["X"]);
    assert_eq!(conflicts[0].theirs, vec!["Y"]);
}

#[test]
fn test_merge_same_change_both_sides() {
    let base = "a\nb\nc\n";
    let ours = "a\nX\nc\n";
    let theirs = "a\nX\nc\n";

    let (merged, conflicts) = merge_texts(base, ours, theirs);

    assert!(conflicts.is_empty());
    assert_eq!(merged, "a\nX\nc");
}

#[test]
fn test_merge_multiline_conflict() {
    let base = "a\nb\nc\nd\n";
    let ours = "a\nX\nY\nc\nd\n";
    let theirs = "a\nP\nQ\nc\nd\n";

    let (_, conflicts) = merge_texts(base, ours, theirs);

    assert!(!conflicts.is_empty());
    assert_eq!(conflicts[0].ours, vec!["X", "Y"]);
    assert_eq!(conflicts[0].theirs, vec!["P", "Q"]);
}

#[test]
fn test_merge_multiple_conflicts() {
    let base = "a\nb\nc\nd\ne\nf\n";
    let ours = "a\nX\nc\nd\nY\nf\n";
    let theirs = "a\nP\nc\nd\nQ\nf\n";

    let (_, conflicts) = merge_texts(base, ours, theirs);

    assert_eq!(conflicts.len(), 2);
}

#[test]
fn test_merge_empty_base() {
    let base = "";
    let ours = "a\nb\n";
    let theirs = "";

    let (merged, conflicts) = merge_texts(base, ours, theirs);

    assert!(conflicts.is_empty());
    assert_eq!(merged, "a\nb");
}

#[test]
fn test_merge_one_side_no_changes() {
    let base = "a\nb\nc\n";
    let ours = "a\nb\nc\n";
    let theirs = "a\nX\nc\n";

    let (merged, conflicts) = merge_texts(base, ours, theirs);

    assert!(conflicts.is_empty());
    assert_eq!(merged, "a\nX\nc");
}

#[test]
fn test_merge_insert_vs_delete() {
    let base = "a\nb\nc\n";
    let ours = "a\nc\n";
    let theirs = "a\nX\nb\nc\n";

    let (_, conflicts) = merge_texts(base, ours, theirs);

    assert!(!conflicts.is_empty(), "insert vs delete should conflict");
}

#[test]
fn test_conflict_marker_format() {
    let conflict = MergeConflict {
        start_line: 1,
        base: vec!["b".to_string()],
        ours: vec!["X".to_string()],
        theirs: vec!["Y".to_string()],
    };

    let markers = conflict.to_conflict_marker();

    assert!(markers.contains("<<<<<<< ours"));
    assert!(markers.contains("======="));
    assert!(markers.contains(">>>>>>> theirs"));
    assert!(markers.contains("X"));
    assert!(markers.contains("Y"));
}

// ---------------------------------------------------------------------------
// Test: Inverse delta generation (inverse_delta)
// ---------------------------------------------------------------------------

#[test]
fn test_inverse_insert_becomes_delete() {
    let hunk = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 2,
        ops: vec![
            layertwine::core::types::DiffOp::Equal { count: 1 },
            layertwine::core::types::DiffOp::Insert {
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

    let has_delete = inv.diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Delete { .. }))
    });
    assert!(has_delete);
}

#[test]
fn test_inverse_delete_requires_old_content() {
    let hunk = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 0,
        ops: vec![layertwine::core::types::DiffOp::Delete {
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

    // Without old_content, blank lines are generated
    let inv = inverse_delta(&delta, None).unwrap();
    let has_insert = inv.diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Insert { .. }))
    });
    assert!(has_insert);

    // With old_content, deleted lines are recovered
    let inv2 = inverse_delta(&delta, Some("deleted_line\n")).unwrap();
    let insert_lines: Vec<&str> = inv2
        .diff
        .hunks
        .iter()
        .flat_map(|h| &h.ops)
        .filter_map(|op| {
            if let layertwine::core::types::DiffOp::Insert { lines, .. } = op {
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
fn test_inverse_replace() {
    let hunk = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 1,
        ops: vec![layertwine::core::types::DiffOp::Replace {
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
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Replace { .. }))
    });
    assert!(has_replace);
}

#[test]
fn test_inverse_preserves_equal() {
    let hunk = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 2,
        new_start: 1,
        new_len: 2,
        ops: vec![layertwine::core::types::DiffOp::Equal { count: 2 }],
    };

    let diff = LineDiff::new(vec![hunk]);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b"content"),
        diff,
        SourceType::Manual,
    );

    let inv = inverse_delta(&delta, None).unwrap();

    let has_equal = inv.diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Equal { .. }))
    });
    assert!(has_equal);
}

#[test]
fn test_inverse_snapshot_empty_chain() {
    let file_node = FileNode::new(PathBuf::from("test.txt"), b"content");
    let empty_diff = LineDiff::new(vec![]);
    let delta = Delta::new(file_node.clone(), empty_diff, SourceType::Manual);
    let snapshot = Snapshot::new_initial(file_node, delta.id);

    let result = inverse_snapshot(&snapshot, &[delta], &[""]);
    assert!(result.is_ok());

    let inverses = result.unwrap();
    assert_eq!(inverses.len(), 1);
    assert!(inverses[0].diff.is_empty());
}

#[test]
fn test_inverse_snapshot_mismatched_length() {
    let file_node = FileNode::new(PathBuf::from("test.txt"), b"content");
    let empty_diff = LineDiff::new(vec![]);
    let delta = Delta::new(file_node, empty_diff, SourceType::Manual);
    let snapshot = Snapshot::new_initial(delta.file.clone(), delta.id);

    let result = inverse_snapshot(&snapshot, &[], &[]);
    assert!(result.is_err());
}

#[test]
fn test_inverse_snapshot_chain() {
    let content0 = "a\nb\nc\n";
    let content1 = "a\nX\nc\n";
    let file = FileNode::new(PathBuf::from("test.txt"), b"");

    let diff1 = diff_to_line_diff(content0, content1);
    let delta1 = Delta::new(file.clone(), diff1, SourceType::Manual);

    let snapshot = Snapshot::new_initial(file.clone(), delta1.id);

    let result = inverse_snapshot(&snapshot, &[delta1], &[content0]);
    assert!(result.is_ok());

    let inverses = result.unwrap();
    assert_eq!(inverses.len(), 1);
}

// ---------------------------------------------------------------------------
// Test: Round-trip operations (apply + inverse = identity)
// ---------------------------------------------------------------------------

#[test]
fn test_roundtrip_insert() {
    let old = "line1\nline3\n";
    let new = "line1\nline2\nline3\n";

    let diff = diff_to_line_diff(old, new);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    // Apply delta
    let applied = apply_deltas(old, std::slice::from_ref(&delta)).unwrap();
    // apply_deltas strips trailing newlines, so we compare without them
    let applied_expected = new.trim_end_matches('\n');
    assert_eq!(applied, applied_expected);

    // Inverse and apply to get back
    let inv = inverse_delta(&delta, Some(old)).unwrap();
    let restored = apply_deltas(&applied, &[inv]).unwrap();

    // apply_deltas strips trailing newlines, so we compare without them
    let old_expected = old.trim_end_matches('\n');
    assert_eq!(restored, old_expected);
}

#[test]
fn test_roundtrip_delete() {
    let old = "line1\nline2\nline3\n";
    let new = "line1\nline3\n";

    let diff = diff_to_line_diff(old, new);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let applied = apply_deltas(old, std::slice::from_ref(&delta)).unwrap();
    let applied_expected = new.trim_end_matches('\n');
    assert_eq!(applied, applied_expected);

    let inv = inverse_delta(&delta, Some(old)).unwrap();
    let restored = apply_deltas(&applied, &[inv]).unwrap();

    let old_expected = old.trim_end_matches('\n');
    assert_eq!(restored, old_expected);
}

#[test]
fn test_roundtrip_replace() {
    let old = "line1\nold\nline3\n";
    let new = "line1\nnew\nline3\n";

    let diff = diff_to_line_diff(old, new);
    let delta = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff,
        SourceType::Manual,
    );

    let applied = apply_deltas(old, std::slice::from_ref(&delta)).unwrap();
    let applied_expected = new.trim_end_matches('\n');
    assert_eq!(applied, applied_expected);

    let inv = inverse_delta(&delta, Some(old)).unwrap();
    let restored = apply_deltas(&applied, &[inv]).unwrap();

    let old_expected = old.trim_end_matches('\n');
    assert_eq!(restored, old_expected);
}

#[test]
fn test_roundtrip_chain() {
    let content0 = "a\nb\nc\n";
    let content1 = "a\nX\nc\n";
    let content2 = "a\nX\nY\nc\n";

    let diff1 = diff_to_line_diff(content0, content1);
    let diff2 = diff_to_line_diff(content1, content2);

    let delta1 = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff1,
        SourceType::Manual,
    );
    let delta2 = Delta::new(
        FileNode::new(PathBuf::from("test.txt"), b""),
        diff2,
        SourceType::Manual,
    );

    // Apply both deltas
    let applied = apply_deltas(content0, &[delta1.clone(), delta2.clone()]).unwrap();
    let applied_expected = content2.trim_end_matches('\n');
    assert_eq!(applied, applied_expected);

    // Inverse in reverse order
    let inv2 = inverse_delta(&delta2, Some(content1)).unwrap();
    let inv1 = inverse_delta(&delta1, Some(content0)).unwrap();

    let restored = apply_deltas(&applied, &[inv2, inv1]).unwrap();

    let content0_expected = content0.trim_end_matches('\n');
    assert_eq!(restored, content0_expected);
}

// ---------------------------------------------------------------------------
// Test: Edge cases and error handling
// ---------------------------------------------------------------------------

#[test]
fn test_diff_trailing_newlines() {
    let old = "a\nb\nc";
    let new = "a\nb\nc\n";
    let line_diff = diff_to_line_diff(old, new);

    // Should detect the newline addition
    assert!(!line_diff.hunks.is_empty());
}

#[test]
fn test_diff_single_character() {
    let old = "x";
    let new = "y";
    let line_diff = diff_to_line_diff(old, new);

    assert!(!line_diff.hunks.is_empty());
}

#[test]
fn test_diff_empty_both() {
    let line_diff = diff_to_line_diff("", "");

    assert!(line_diff.hunks.is_empty());
}

#[test]
fn test_apply_deltas_with_trailing_newline() {
    let content = "a\nb\nc\n";
    let result = apply_deltas(content, &[]).unwrap();

    assert_eq!(result, "a\nb\nc");
}

#[test]
fn test_merge_empty_ours() {
    let base = "a\nb\nc\n";
    let ours = "";
    let theirs = "a\nb\nc\n";

    let (merged, conflicts) = merge_texts(base, ours, theirs);

    assert!(conflicts.is_empty());
    // When one side deletes all content and the other makes no changes,
    // the merge result should be empty (deletion wins)
    assert_eq!(merged, "");
}

#[test]
fn test_merge_empty_theirs() {
    let base = "a\nb\nc\n";
    let ours = "a\nb\nc\n";
    let theirs = "";

    let (merged, conflicts) = merge_texts(base, ours, theirs);

    assert!(conflicts.is_empty());
    // When one side deletes all content and the other makes no changes,
    // the merge result should be empty (deletion wins)
    assert_eq!(merged, "");
}

#[test]
fn test_inverse_delta_empty() {
    let diff = LineDiff::new(vec![]);
    let file_node = FileNode::new(PathBuf::from("empty.txt"), b"content");
    let delta = Delta::new(file_node, diff, SourceType::Manual);

    let inverse = inverse_delta(&delta, Some("content")).unwrap();

    assert!(inverse.diff.is_empty());
}

#[test]
fn test_inverse_mixed_ops() {
    let hunk = layertwine::core::types::Hunk {
        old_start: 1,
        old_len: 3,
        new_start: 1,
        new_len: 4,
        ops: vec![
            layertwine::core::types::DiffOp::Equal { count: 1 },
            layertwine::core::types::DiffOp::Delete {
                old_start: 2,
                count: 1,
            },
            layertwine::core::types::DiffOp::Insert {
                new_start: 3,
                lines: vec!["inserted".to_string()],
            },
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
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Delete { .. }))
    });
    let has_insert = inv.diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Insert { .. }))
    });
    let has_equal = inv.diff.hunks.iter().any(|h| {
        h.ops
            .iter()
            .any(|op| matches!(op, layertwine::core::types::DiffOp::Equal { .. }))
    });

    assert!(has_delete);
    assert!(has_insert);
    assert!(has_equal);
}
