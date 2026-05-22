mod common;

use std::sync::Arc;
use std::thread;

use stratum::core::delta::Delta;
use stratum::core::file_node::FileNode;
use stratum::core::types::SourceType;
use stratum::storage::repository::{DeltaStore, FileNodeStore};

// CC-01: Concurrent reads from multiple threads
#[test]
fn test_concurrent_reads() {
    let storage = Arc::new(common::create_storage());
    let file_node = FileNode::new(std::path::PathBuf::from("read_test.txt"), b"concurrent");
    storage
        .store_file_node(&file_node, b"concurrent")
        .unwrap();

    let mut handles = Vec::new();
    for _ in 0..10 {
        let storage = Arc::clone(&storage);
        let file_node = file_node.clone();
        handles.push(thread::spawn(move || {
            let result = storage.get_file_content(&file_node);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), b"concurrent");
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

// CC-02: Concurrent writes to different file nodes
#[test]
fn test_concurrent_writes_different_files() {
    let storage = Arc::new(common::create_storage());
    let mut handles = Vec::new();

    for i in 0..10 {
        let storage = Arc::clone(&storage);
        handles.push(thread::spawn(move || {
            let content = format!("content_{}", i);
            let file_node = FileNode::new(
                std::path::PathBuf::from(format!("concurrent_{}.txt", i)),
                content.as_bytes(),
            );
            storage
                .store_file_node(&file_node, content.as_bytes())
                .unwrap();
            let retrieved = storage.get_file_content(&file_node).unwrap();
            assert_eq!(String::from_utf8_lossy(&retrieved), content);
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}

// CC-03: Concurrent write and read of same file node
#[test]
fn test_concurrent_write_and_read_same_file() {
    let storage = Arc::new(common::create_storage());
    let file_node = FileNode::new(std::path::PathBuf::from("shared.txt"), b"shared");
    storage
        .store_file_node(&file_node, b"shared")
        .unwrap();

    let storage_clone = Arc::clone(&storage);
    let writer = thread::spawn(move || {
        let content = b"updated";
        let new_fn = FileNode::new(
            std::path::PathBuf::from("shared.txt"),
            content,
        );
        storage_clone.store_file_node(&new_fn, content).unwrap();
    });

    let reader = thread::spawn(move || {
        let result = storage.get_file_content(&file_node);
        assert!(result.is_ok());
    });

    writer.join().unwrap();
    reader.join().unwrap();
}

// CC-04: Concurrent delta storage
#[test]
fn test_concurrent_delta_storage() {
    let storage = Arc::new(common::create_storage());
    let mut handles = Vec::new();

    for i in 0..10 {
        let storage = Arc::clone(&storage);
        handles.push(thread::spawn(move || {
            let content = format!("delta_content_{}", i);
            let file_node = FileNode::new(
                std::path::PathBuf::from("delta_test.txt"),
                content.as_bytes(),
            );
            let hunk = stratum::core::types::Hunk {
                old_start: 1,
                old_len: 0,
                new_start: 1,
                new_len: 1,
                ops: vec![stratum::core::types::DiffOp::Insert {
                    new_start: 1,
                    lines: vec![format!("line_{}", i)],
                }],
            };
            let diff = stratum::core::delta::LineDiff::new(vec![hunk]);
            let delta = Delta::new(file_node, diff, SourceType::Manual);
            storage.store_delta(&delta).unwrap();
            assert!(storage.delta_exists(&delta.id).unwrap());
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
}