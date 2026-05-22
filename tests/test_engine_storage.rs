mod common;

use std::path::PathBuf;
use stratum::core::delta::{Delta, LineDiff};
use stratum::core::file_node::FileNode;
use stratum::core::snapshot::Snapshot;
use stratum::core::types::{DiffOp, Hunk, SourceType};
use stratum::engine::diff::diff_to_line_diff;
use stratum::engine::inverse::inverse_delta;
use stratum::engine::merge::apply_deltas;
use stratum::storage::repository::{DeltaStore, FileNodeStore, SnapshotStore};

// ES-01: diff → store → apply roundtrip
#[test]
fn test_diff_store_apply_roundtrip() {
    let storage = common::create_storage();
    let old = "hello\nworld\n";
    let new = "hello\nstratum\nworld\n";

    let line_diff = diff_to_line_diff(old, new);
    let file = FileNode::new(PathBuf::from("f.txt"), old.as_bytes());
    let delta = Delta::new(file, line_diff, SourceType::Manual);
    storage.store_delta(&delta).unwrap();

    let result = apply_deltas(old, &[delta]).unwrap();
    assert_eq!(result, new.trim_end_matches('\n'));
}

// ES-02: Multiple Delta chain reconstruction
#[test]
fn test_multi_delta_chain_reconstruction() {
    let storage = common::create_storage();
    let contents = vec!["v1\n", "v1\nv2\n", "v1\nv2\nv3\n", "v1\nmodified\nv3\n"];
    let file_path = PathBuf::from("chain.txt");
    let mut file = FileNode::new(file_path.clone(), contents[0].as_bytes());
    storage.store_file_node(&file, contents[0].as_bytes()).unwrap();

    let mut deltas = Vec::new();
    for i in 1..contents.len() {
        let line_diff = diff_to_line_diff(contents[i - 1], contents[i]);
        let delta = Delta::new(file.clone(), line_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        file = FileNode::new(file_path.clone(), contents[i].as_bytes());
        storage.store_file_node(&file, contents[i].as_bytes()).unwrap();
        deltas.push(delta);
    }

    let result = apply_deltas(contents[0], &deltas).unwrap();
    assert_eq!(result, contents[contents.len() - 1].trim_end_matches('\n'));
}

// ES-03: Merge result stored as new Snapshot
#[test]
fn test_merge_result_stored_as_snapshot() {
    let storage = common::create_storage();
    let base = "line1\nline2\nline3\n";
    let sid = common::create_initial_snapshot(&storage, "merge.txt", base);
    let base_snap = storage.get_snapshot(&sid).unwrap();

    // Create merge result
    let (merged, _conflicts) = stratum::engine::merge::merge_texts(base, base, base);
    let merged_diff = diff_to_line_diff(base, &merged);
    let merge_delta = Delta::new(
        base_snap.file.clone(),
        merged_diff,
        SourceType::Manual,
    );
    storage.store_delta(&merge_delta).unwrap();

    // Store merge snapshot with dual parents
    let sid2 = common::create_initial_snapshot(&storage, "merge.txt", "other\n");
    let other_snap = storage.get_snapshot(&sid2).unwrap();
    let merge_snap = Snapshot::merge(
        vec![&base_snap, &other_snap],
        merge_delta.id,
        "staged".to_string(),
    );
    storage.store_snapshot(&merge_snap, b"").unwrap();

    let retrieved = storage.get_snapshot(&merge_snap.id).unwrap();
    assert_eq!(retrieved.parents.len(), 2);
    assert!(retrieved.parents.contains(&sid));
    assert!(retrieved.parents.contains(&sid2));
}

// ES-04: Empty diff — no change produces empty diff
#[test]
fn test_empty_diff() {
    let content = "same\ncontent\n";
    let line_diff = diff_to_line_diff(content, content);
    assert!(line_diff.is_empty());
}

// ES-05: Inverse Delta applied after original restores original content
#[test]
fn test_inverse_delta_restores_content() {
    let old_content = "hello\nworld\n";
    let new_content = "hello\nstratum\nworld\n";

    let line_diff = diff_to_line_diff(old_content, new_content);
    let file_node = FileNode::new(PathBuf::from("inverse.txt"), old_content.as_bytes());
    let delta = Delta::new(file_node, line_diff, SourceType::Manual);

    // Apply original delta
    let after_apply = apply_deltas(old_content, &[delta.clone()]).unwrap();
    assert_eq!(after_apply, new_content.trim_end_matches('\n'));

    // Create and apply inverse delta
    let inv = inverse_delta(&delta, Some(old_content)).unwrap();
    let restored = apply_deltas(&after_apply, &[inv]).unwrap();
    assert_eq!(restored, old_content.trim_end_matches('\n'));
}

// ES-06: Inverse Delta from Delete requires old_content
#[test]
fn test_inverse_delete_requires_old_content() {
    let old_content = "deleted_line\n";
    let new_content = "";

    let line_diff = diff_to_line_diff(old_content, new_content);
    let file_node = FileNode::new(PathBuf::from("del.txt"), old_content.as_bytes());
    let delta = Delta::new(file_node, line_diff, SourceType::Manual);

    // Without old_content, empty strings are inserted
    let inv_no_content = inverse_delta(&delta, None).unwrap();
    let result_no_content = apply_deltas(new_content, &[inv_no_content]).unwrap();
    // With no old content, the inverse produces empty lines
    assert_eq!(result_no_content, "");

    // With old_content, the deleted line is properly restored
    let inv_with_content = inverse_delta(&delta, Some(old_content)).unwrap();
    let result_with_content = apply_deltas(new_content, &[inv_with_content]).unwrap();
    assert_eq!(result_with_content, old_content.trim_end_matches('\n'));
}

// ES-07: Large file diff (5000 lines, insert in middle)
#[test]
fn test_large_file_diff() {
    let mut lines: Vec<String> = (0..5000).map(|i| format!("line_{}", i)).collect();
    let old = lines.join("\n");

    lines.insert(2500, "inserted_line".to_string());
    let new = lines.join("\n");

    let line_diff = diff_to_line_diff(&old, &new);
    let file_node = FileNode::new(PathBuf::from("large.txt"), old.as_bytes());
    let delta = Delta::new(file_node, line_diff, SourceType::Manual);

    let result = apply_deltas(&old, &[delta]).unwrap();
    assert_eq!(result, new);
}

// ES-08: Binary content handling (content with null bytes)
#[test]
fn test_binary_content_diff() {
    let old = "hello\x00world\n";
    let new = "hello\x00stratum\x00world\n";

    let line_diff = diff_to_line_diff(old, new);
    let file_node = FileNode::new(PathBuf::from("binary.bin"), old.as_bytes());
    let delta = Delta::new(file_node, line_diff, SourceType::Manual);

    let result = apply_deltas(old, &[delta]).unwrap();
    assert_eq!(result, new.trim_end_matches('\n'));
}

// ES-02b: 10 Delta chain
#[test]
fn test_ten_delta_chain() {
    let storage = common::create_storage();
    let base = "start\n";
    let file_path = PathBuf::from("ten_chain.txt");
    let mut file = FileNode::new(file_path.clone(), base.as_bytes());
    storage.store_file_node(&file, base.as_bytes()).unwrap();

    let mut current = base.to_string();
    let mut deltas = Vec::new();
    for i in 0..10 {
        let next = format!("{}line_{}\n", current, i);
        let line_diff = diff_to_line_diff(&current, &next);
        let delta = Delta::new(file.clone(), line_diff, SourceType::Manual);
        storage.store_delta(&delta).unwrap();
        file = FileNode::new(file_path.clone(), next.as_bytes());
        storage.store_file_node(&file, next.as_bytes()).unwrap();
        deltas.push(delta);
        current = next;
    }

    let result = apply_deltas(base, &deltas).unwrap();
    assert_eq!(result, current.trim_end_matches('\n'));
}

// Delta summary statistics
#[test]
fn test_delta_summary() {
    let hunk = Hunk {
        old_start: 1,
        old_len: 1,
        new_start: 1,
        new_len: 2,
        ops: vec![
            DiffOp::Delete {
                old_start: 1,
                count: 1,
            },
            DiffOp::Insert {
                new_start: 1,
                lines: vec!["a".to_string(), "b".to_string()],
            },
        ],
    };
    let diff = LineDiff::new(vec![hunk]);
    let file_node = FileNode::new(PathBuf::from("s.txt"), b"old");
    let delta = Delta::new(file_node, diff, SourceType::Manual);
    let summary = delta.summary();
    assert_eq!(summary.deletes, 1);
    assert_eq!(summary.inserts, 2);
    assert_eq!(summary.total_hunks, 1);
}